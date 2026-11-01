//! Importer for the previous Print Vault (Python) database.
//!
//! Each Print Vault project becomes a *linked* Project Vault project in the
//! 3D-printing category, pointing at the same folder — nothing is copied or
//! moved. Print metadata goes into the 3D-printing module fields, notes go
//! into a project note. The Print Vault database is opened read-only.

use crate::db::{Db, DbResult};
use crate::json::Json;
use crate::projects::{self, NewProject};
use crate::{files, knowledge, p, Vault};
use std::path::{Path, PathBuf};

fn clean(s: String) -> String {
    let t = s.trim();
    if t == "-" { String::new() } else { t.to_string() }
}

/// Floats stored as "0.4000000059604645 mm" become "0.4 mm".
fn tidy_mm(s: String) -> String {
    let s = clean(s);
    let num: String = s.chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
    match num.parse::<f64>() {
        Ok(f) if !num.is_empty() => {
            let r = (f * 1000.0).round() / 1000.0;
            format!("{}{}", r, &s[num.len()..])
        }
        _ => s,
    }
}

/// Print Vault stored absolute Windows paths. If the vault was moved, find the
/// folder again relative to the `Projects` directory of the given vault.
fn resolve_folder(vault_dir: &Path, stored: &str) -> Option<PathBuf> {
    let p = PathBuf::from(stored);
    if p.is_dir() {
        return Some(p);
    }
    let parts: Vec<&str> = stored.split(['\\', '/']).filter(|s| !s.is_empty()).collect();
    let idx = parts.iter().rposition(|s| *s == "Projects")?;
    let mut out = vault_dir.join("Projects");
    for s in &parts[idx + 1..] {
        if *s == ".." {
            return None;
        }
        out.push(s);
    }
    out.is_dir().then_some(out)
}

pub fn import_print_vault(v: &Vault, pv_dir: &Path) -> DbResult<Json> {
    let db_path = if pv_dir.is_file() { pv_dir.to_path_buf() } else { pv_dir.join("print_vault.sqlite") };
    let vault_dir = db_path.parent().unwrap_or(Path::new(".")).to_path_buf();
    if !db_path.is_file() {
        return Err(format!("No print_vault.sqlite found in {}", pv_dir.display()));
    }
    let src = Db::open_readonly(&db_path)?;
    let rows = src.query("SELECT * FROM projects ORDER BY id", p![])?;
    let mut created = Vec::new();
    let mut skipped = Vec::new();
    for r in rows {
        let name = r.s("name");
        let Some(folder) = resolve_folder(&vault_dir, &r.s("folder_path")) else {
            skipped.push(Json::obj().with("name", name).with("reason", format!("folder not found: {}", r.s("folder_path"))));
            continue;
        };
        let pv_status = r.s("status");
        let status = if pv_status == "ARCHIVED" { "Archived" } else if pv_status == "IDEA" { "Planned" } else { "Active" };
        let module = Json::obj()
            .with("printStatus", pv_status.clone())
            .with("pvCategory", r.s("category"))
            .with("fitsContains", clean(r.s("fits_contains")))
            .with("material", clean(r.s("material")))
            .with("printer", clean(r.s("printer")))
            .with("nozzle", tidy_mm(r.s("nozzle")))
            .with("layerHeight", tidy_mm(r.s("layer_height")))
            .with("printTime", clean(r.s("print_time")))
            .with("filamentGrams", clean(r.s("filament_grams")))
            .with("dimensions", clean(r.s("dimensions_mm")))
            .with("importedFrom", "Print Vault");
        let res = projects::create(
            v,
            NewProject {
                name: name.clone(),
                description: clean(r.s("fits_contains")),
                category: "3d-printing".into(),
                status: status.into(),
                link_path: Some(folder.clone()),
                module,
            },
        );
        let pj = match res {
            Ok(pj) => pj,
            Err(e) => {
                skipped.push(Json::obj().with("name", name).with("reason", e));
                continue;
            }
        };
        let pid = pj.str_of("id").unwrap();
        let cat = r.s("category");
        if !cat.is_empty() && cat != "Uncategorized" {
            projects::set_tags(v, "project", &pid, &[cat])?;
        }
        let notes = r.s("notes");
        if !notes.trim().is_empty() {
            knowledge::create(v, &pid, "Print notes", None, "General", &notes)?;
        }
        let rep = files::scan(v, &pid)?;
        created.push(Json::obj().with("id", pid).with("name", name).with("folder", folder.to_string_lossy().to_string()).with("files", rep.total));
    }
    Ok(Json::obj().with("created", Json::Arr(created)).with("skipped", Json::Arr(skipped)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;
    #[test]
    fn imports_print_vault_db() {
        let (t, v) = vault();
        let pv = t.path().join("3D Prints Vault");
        let proj = pv.join("Projects/Frames/150x225-frame");
        std::fs::create_dir_all(&proj).unwrap();
        std::fs::write(proj.join("frame.blend"), "blend").unwrap();
        let db = Db::open(&pv.join("print_vault.sqlite")).unwrap();
        db.batch("CREATE TABLE projects (id INTEGER PRIMARY KEY, name TEXT, slug TEXT, category TEXT, status TEXT, fits_contains TEXT, material TEXT, printer TEXT, nozzle TEXT, layer_height TEXT, print_time TEXT, filament_grams TEXT, dimensions_mm TEXT, notes TEXT, folder_path TEXT, created_at TEXT, updated_at TEXT)").unwrap();
        db.exec(
            "INSERT INTO projects VALUES (8,'150x225 Frame','150x225-frame','Frames','MODELED','150x225 Hueforges','PLA','X2D','0.4000000059604645 mm','-','-','-','164 x 243','Already used on: cat',?, '', '')",
            p!["D:\\PrintVault\\3D Prints Vault\\Projects\\Frames\\150x225-frame"],
        )
        .unwrap();
        db.exec("INSERT INTO projects VALUES (9,'Gone','gone','Frames','IDEA','','','','','','','','','',?,'','')", p!["D:\\nowhere\\x"]).unwrap();
        drop(db);
        let r = import_print_vault(&v, &pv).unwrap();
        assert_eq!(r.get("created").unwrap().as_arr().unwrap().len(), 1);
        assert_eq!(r.get("skipped").unwrap().as_arr().unwrap().len(), 1);
        let pid = r.get("created").unwrap().as_arr().unwrap()[0].str_of("id").unwrap();
        let pj = projects::get(&v, &pid).unwrap();
        assert_eq!(pj.get("module").unwrap().str_of("nozzle").unwrap(), "0.4 mm");
        assert_eq!(pj.get("module").unwrap().get("layerHeight"), Some(&Json::Str(String::new())));
        assert_eq!(pj.get("fileCount").unwrap().as_i64(), Some(1));
        // second import skips the already-linked folder
        let r2 = import_print_vault(&v, &pv).unwrap();
        assert_eq!(r2.get("created").unwrap().as_arr().unwrap().len(), 0);
    }
}

//! Projects: registration (linked or managed), metadata, statuses, tags,
//! favorites, dashboard summaries.

use crate::db::{DbResult, Row};
use crate::json::Json;
use crate::util::{new_id, now_ms, slugify};
use crate::{p, search, timeline, Vault};
use std::path::{Path, PathBuf};

pub const BUILTIN_STATUSES: &[(&str, &str)] = &[
    ("Planned", "#7c8cff"),
    ("Active", "#3ecf8e"),
    ("Paused", "#f5b94a"),
    ("Completed", "#4fb3ff"),
    ("Archived", "#8b93a7"),
];

pub const CATEGORIES: &[&str] = &["general", "software", "game", "3d-printing", "electronics", "creative", "documents"];

pub fn seed_statuses(v: &Vault) -> DbResult<()> {
    for (i, (n, c)) in BUILTIN_STATUSES.iter().enumerate() {
        v.db.exec(
            "INSERT OR IGNORE INTO statuses(id,name,color,sort,builtin) VALUES (?,?,?,?,1)",
            p![new_id(), *n, *c, i as i64],
        )?;
    }
    Ok(())
}

pub fn default_ignores(category: &str) -> &'static str {
    match category {
        // Regenerable Unity/engine output; can be removed per project.
        "game" => "Library/\nTemp/\nLogs/\nobj/\nBuild/\nBuilds/\nUserSettings/",
        "software" => "target/\ndist/\nbuild/\n.venv/",
        _ => "",
    }
}

pub struct NewProject {
    pub name: String,
    pub description: String,
    pub category: String,
    pub status: String,
    /// Some(path) => linked to an existing folder; None => managed.
    pub link_path: Option<PathBuf>,
    pub module: Json,
}

pub fn create(v: &Vault, np: NewProject) -> DbResult<Json> {
    let name = np.name.trim();
    if name.is_empty() {
        return Err("Project name is required".into());
    }
    let category = if CATEGORIES.contains(&np.category.as_str()) { np.category.clone() } else { "general".into() };
    let id = new_id();
    let (mode, root) = match &np.link_path {
        Some(p) => {
            let md = std::fs::metadata(p).map_err(|e| format!("Cannot access folder {}: {e}", p.display()))?;
            if !md.is_dir() {
                return Err(format!("{} is not a folder", p.display()));
            }
            let canon = dunce(p);
            if let Some(r) = v.db.one("SELECT name FROM projects WHERE root_path=?", p![canon.to_string_lossy().to_string()])? {
                return Err(format!("This folder is already linked to project \"{}\"", r.s("name")));
            }
            ("linked", canon)
        }
        None => {
            let dir = v.managed_root().join(format!("{}-{}", slugify(name), &id[..8]));
            std::fs::create_dir_all(&dir).map_err(|e| format!("Cannot create project storage: {e}"))?;
            ("managed", dir)
        }
    };
    let now = now_ms();
    let status = if np.status.trim().is_empty() { "Active".to_string() } else { np.status.trim().to_string() };
    v.db.tx(|db| {
        ensure_status(v, &status)?;
        db.exec(
            "INSERT INTO projects(id,name,description,category,status,mode,root_path,module_json,ignore_globs,created_at,updated_at,last_opened_at)
             VALUES (?,?,?,?,?,?,?,?,?,?,?,?)",
            p![
                id.clone(),
                name,
                np.description.clone(),
                category.clone(),
                status.clone(),
                mode,
                root.to_string_lossy().to_string(),
                np.module.to_string(),
                default_ignores(&category),
                now,
                now,
                now
            ],
        )?;
        timeline::add(
            v,
            &id,
            "project",
            &format!("Project created ({mode})"),
            &root.to_string_lossy(),
            None,
            false,
        )?;
        Ok(())
    })?;
    search::index_project(v, &id)?;
    get(v, &id)
}

/// Canonicalize without Windows `\\?\` verbatim prefixes.
pub fn dunce(p: &Path) -> PathBuf {
    let c = std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let s = c.to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{rest}"))
    } else if let Some(rest) = s.strip_prefix(r"\\?\") {
        PathBuf::from(rest)
    } else {
        c
    }
}

fn ensure_status(v: &Vault, s: &str) -> DbResult<()> {
    let n = v.db.scalar_i64("SELECT count(*) FROM statuses WHERE name=?", p![s])?;
    if n == 0 {
        let sort = v.db.scalar_i64("SELECT COALESCE(MAX(sort),0)+1 FROM statuses", p![])?;
        v.db.exec("INSERT INTO statuses(id,name,sort) VALUES (?,?,?)", p![new_id(), s, sort])?;
    }
    Ok(())
}

pub fn statuses(v: &Vault) -> DbResult<Json> {
    let rows = v.db.query("SELECT name,color,builtin FROM statuses ORDER BY sort", p![])?;
    Ok(Json::Arr(
        rows.iter().map(|r| Json::obj().with("name", r.s("name")).with("color", r.s("color")).with("builtin", r.b("builtin"))).collect(),
    ))
}

pub fn tags_for(v: &Vault, et: &str, id: &str) -> DbResult<Vec<String>> {
    Ok(v.db
        .query(
            "SELECT t.name FROM entity_tags e JOIN tags t ON t.id=e.tag_id WHERE e.entity_type=? AND e.entity_id=? ORDER BY t.name",
            p![et, id],
        )?
        .iter()
        .map(|r| r.s("name"))
        .collect())
}

pub fn set_tags(v: &Vault, et: &str, id: &str, tags: &[String]) -> DbResult<()> {
    v.db.tx(|db| {
        db.exec("DELETE FROM entity_tags WHERE entity_type=? AND entity_id=?", p![et, id])?;
        for t in tags.iter().map(|t| t.trim()).filter(|t| !t.is_empty()) {
            db.exec("INSERT OR IGNORE INTO tags(id,name) VALUES (?,?)", p![new_id(), t])?;
            db.exec(
                "INSERT OR IGNORE INTO entity_tags SELECT ?,?,id FROM tags WHERE name=?",
                p![et, id, t],
            )?;
        }
        Ok(())
    })
}

pub fn all_tags(v: &Vault) -> DbResult<Json> {
    let rows = v.db.query(
        "SELECT t.name, count(e.tag_id) AS uses FROM tags t LEFT JOIN entity_tags e ON e.tag_id=t.id GROUP BY t.id ORDER BY t.name",
        p![],
    )?;
    Ok(Json::Arr(rows.iter().map(|r| Json::obj().with("name", r.s("name")).with("uses", r.i("uses"))).collect()))
}

fn row_json(v: &Vault, r: &Row) -> DbResult<Json> {
    let id = r.s("id");
    let stats = v.db.one(
        "SELECT count(*) AS n, COALESCE(SUM(size),0) AS bytes,
                SUM(state='missing') AS missing, SUM(changed_since_review) AS changed
         FROM files WHERE project_id=?",
        p![id.clone()],
    )?;
    let st = stats.unwrap();
    let open_tasks = v.db.scalar_i64(
        "SELECT count(*) FROM tasks WHERE project_id=? AND status!='done'",
        p![id.clone()],
    )?;
    let reviews = v.db.scalar_i64("SELECT count(*) FROM reconcile_reviews WHERE project_id=?", p![id.clone()])?;
    let latest = v.db.one(
        "SELECT summary, at FROM timeline WHERE project_id=? ORDER BY at DESC LIMIT 1",
        p![id.clone()],
    )?;
    let module = Json::parse(&r.s("module_json")).unwrap_or(Json::obj());
    let root = r.s("root_path");
    let available = Path::new(&root).is_dir();
    Ok(Json::obj()
        .with("id", id.clone())
        .with("name", r.s("name"))
        .with("description", r.s("description"))
        .with("category", r.s("category"))
        .with("status", r.s("status"))
        .with("mode", r.s("mode"))
        .with("rootPath", root)
        .with("rootAvailable", available)
        .with("icon", r.s("icon"))
        .with("color", r.s("color"))
        .with("progress", r.i("progress"))
        .with("favorite", r.b("favorite"))
        .with("module", module)
        .with("ignoreGlobs", r.s("ignore_globs"))
        .with("createdAt", r.i("created_at"))
        .with("updatedAt", r.i("updated_at"))
        .with("lastOpenedAt", r.oi("last_opened_at"))
        .with("lastScanAt", r.oi("last_scan_at"))
        .with("tags", tags_for(v, "project", &id)?)
        .with("fileCount", st.i("n"))
        .with("bytes", st.i("bytes"))
        .with("missingFiles", st.i("missing"))
        .with("changedFiles", st.i("changed"))
        .with("pendingReviews", reviews)
        .with("openTasks", open_tasks)
        .with("latestActivity", latest.as_ref().map(|l| l.s("summary")))
        .with("latestActivityAt", latest.as_ref().map(|l| l.i("at"))))
}

pub fn get(v: &Vault, id: &str) -> DbResult<Json> {
    let r = v.db.one("SELECT * FROM projects WHERE id=?", p![id])?.ok_or("Project not found")?;
    row_json(v, &r)
}

pub fn list(v: &Vault) -> DbResult<Json> {
    let rows = v.db.query("SELECT * FROM projects ORDER BY favorite DESC, COALESCE(last_opened_at, updated_at) DESC", p![])?;
    let mut out = Vec::new();
    for r in &rows {
        out.push(row_json(v, r)?);
    }
    Ok(Json::Arr(out))
}

pub fn root_of(v: &Vault, id: &str) -> DbResult<PathBuf> {
    let r = v.db.one("SELECT root_path FROM projects WHERE id=?", p![id])?.ok_or("Project not found")?;
    Ok(PathBuf::from(r.s("root_path")))
}

/// Partial update: only keys present in `patch` are changed.
pub fn update(v: &Vault, id: &str, patch: &Json) -> DbResult<Json> {
    let before = v.db.one("SELECT * FROM projects WHERE id=?", p![id])?.ok_or("Project not found")?;
    v.db.tx(|db| {
        if let Some(n) = patch.str_of("name") {
            if n.trim().is_empty() {
                return Err("Project name cannot be empty".into());
            }
            db.exec("UPDATE projects SET name=? WHERE id=?", p![n.trim(), id])?;
        }
        for (k, col) in [("description", "description"), ("icon", "icon"), ("color", "color"), ("ignoreGlobs", "ignore_globs")] {
            if let Some(s) = patch.str_of(k) {
                db.exec(&format!("UPDATE projects SET {col}=? WHERE id=?"), p![s, id])?;
            }
        }
        if let Some(c) = patch.str_of("category") {
            if !CATEGORIES.contains(&c.as_str()) {
                return Err(format!("Unknown category {c}"));
            }
            db.exec("UPDATE projects SET category=? WHERE id=?", p![c, id])?;
        }
        if let Some(s) = patch.str_of("status") {
            ensure_status(v, s.trim())?;
            db.exec("UPDATE projects SET status=? WHERE id=?", p![s.trim(), id])?;
            if s.trim() != before.s("status") {
                timeline::add(v, id, "status", &format!("Status: {} → {}", before.s("status"), s.trim()), "", None, false)?;
            }
        }
        if let Some(pr) = patch.get("progress").and_then(|x| x.as_i64()) {
            db.exec("UPDATE projects SET progress=? WHERE id=?", p![pr.clamp(0, 100), id])?;
        }
        if let Some(f) = patch.get("favorite").and_then(|x| x.as_bool()) {
            db.exec("UPDATE projects SET favorite=? WHERE id=?", p![f, id])?;
        }
        if let Some(m) = patch.get("module") {
            db.exec("UPDATE projects SET module_json=? WHERE id=?", p![m.to_string(), id])?;
        }
        if let Some(Json::Arr(tags)) = patch.get("tags") {
            let t: Vec<String> = tags.iter().filter_map(|x| x.as_str().map(String::from)).collect();
            set_tags(v, "project", id, &t)?;
        }
        db.exec("UPDATE projects SET updated_at=? WHERE id=?", p![now_ms(), id])?;
        Ok(())
    })?;
    search::index_project(v, id)?;
    get(v, id)
}

pub fn touch_opened(v: &Vault, id: &str) -> DbResult<()> {
    v.db.exec("UPDATE projects SET last_opened_at=? WHERE id=?", p![now_ms(), id])?;
    Ok(())
}

/// Re-points a linked project at a folder (e.g. the drive letter changed).
/// The folder must exist. Files are re-matched on the next scan by path.
pub fn relink(v: &Vault, id: &str, new_root: &Path) -> DbResult<Json> {
    let r = v.db.one("SELECT mode, root_path FROM projects WHERE id=?", p![id])?.ok_or("Project not found")?;
    if r.s("mode") != "linked" {
        return Err("Only linked projects can be re-linked".into());
    }
    if !new_root.is_dir() {
        return Err(format!("{} is not an accessible folder", new_root.display()));
    }
    let canon = dunce(new_root);
    v.db.exec(
        "UPDATE projects SET root_path=?, updated_at=? WHERE id=?",
        p![canon.to_string_lossy().to_string(), now_ms(), id],
    )?;
    timeline::add(v, id, "project", "Project folder re-linked", &format!("{} → {}", r.s("root_path"), canon.display()), None, false)?;
    get(v, id)
}

/// Removes the project from the vault. Never touches a linked folder.
/// Managed storage is deleted only if `delete_managed_files` is true.
pub fn remove(v: &Vault, id: &str, delete_managed_files: bool) -> DbResult<Json> {
    let r = v.db.one("SELECT mode, root_path FROM projects WHERE id=?", p![id])?.ok_or("Project not found")?;
    let root = PathBuf::from(r.s("root_path"));
    v.db.tx(|db| {
        search::unindex_project(v, id)?;
        db.exec("DELETE FROM entity_tags WHERE entity_id IN (SELECT id FROM files WHERE project_id=?) OR entity_id=?", p![id, id])?;
        db.exec("DELETE FROM projects WHERE id=?", p![id])?;
        Ok(())
    })?;
    let mut deleted = false;
    if r.s("mode") == "managed" && delete_managed_files && root.starts_with(v.managed_root()) && root != v.managed_root() {
        std::fs::remove_dir_all(&root).map_err(|e| format!("Project removed, but its managed folder could not be deleted: {e}"))?;
        deleted = true;
    }
    Ok(Json::obj().with("removed", true).with("filesDeleted", deleted))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;
    #[test]
    fn create_linked_and_managed() {
        let (t, v) = vault();
        let dir = t.path().join("omen");
        std::fs::create_dir_all(&dir).unwrap();
        let p = create(
            &v,
            NewProject {
                name: "OMEN v2".into(),
                description: "".into(),
                category: "software".into(),
                status: "Active".into(),
                link_path: Some(dir.clone()),
                module: Json::obj(),
            },
        )
        .unwrap();
        assert_eq!(p.str_of("mode").unwrap(), "linked");
        // linking the same folder twice is refused
        let again = create(&v, NewProject { name: "dup".into(), description: "".into(), category: "general".into(), status: "".into(), link_path: Some(dir.clone()), module: Json::obj() });
        assert!(again.is_err());
        let m = create(&v, NewProject { name: "Print stuff".into(), description: "".into(), category: "3d-printing".into(), status: "Custom Thing".into(), link_path: None, module: Json::obj() }).unwrap();
        assert!(Path::new(&m.str_of("rootPath").unwrap()).is_dir());
        assert!(statuses(&v).unwrap().to_string().contains("Custom Thing"));
        let id = p.str_of("id").unwrap();
        let u = update(&v, &id, &Json::parse(r#"{"tags":["gsc","mw2"],"favorite":true,"progress":140}"#).unwrap()).unwrap();
        assert_eq!(u.get("progress").unwrap().as_i64(), Some(100));
        assert_eq!(u.get("tags").unwrap().as_arr().unwrap().len(), 2);
        let r = remove(&v, &id, true).unwrap();
        assert_eq!(r.get("filesDeleted").unwrap().as_bool(), Some(false));
        assert!(dir.is_dir(), "linked folder must never be deleted");
    }
}

//! Metadata backups and portable exports.
//!
//! A backup folder contains:
//!   vault.db            consistent SQLite snapshot (VACUUM INTO, safe while running)
//!   export/<project>/   human-readable Markdown + JSON for every project:
//!                       project.json, notes/*.md, files.md (every documented file),
//!                       tasks.md, timeline.md
//!   manifest.json       app/schema version, per-file SHA-256, and a statement of
//!                       whether project files are included (they are not, here:
//!                       linked projects are backed up as references + metadata)
//!
//! `verify` re-hashes every file and runs an integrity check on the database.

use crate::db::{Db, DbResult};
use crate::json::Json;
use crate::util::{atomic_write, now_ms, safe_join, sha256_file, slugify};
use crate::{p, schema, Vault};
use std::path::{Path, PathBuf};

fn md_escape_title(s: &str) -> String {
    s.replace(['\n', '\r'], " ")
}

fn export_project(v: &Vault, pid: &str, dir: &Path) -> DbResult<()> {
    let pj = crate::projects::get(v, pid)?;
    std::fs::create_dir_all(dir.join("notes")).map_err(|e| e.to_string())?;
    let w = |p: PathBuf, s: String| atomic_write(&p, s.as_bytes()).map_err(|e| format!("{}: {e}", p.display()));
    w(dir.join("project.json"), pj.pretty())?;

    for (i, n) in v.db.query("SELECT * FROM notes WHERE project_id=? AND deleted_at IS NULL ORDER BY sort", p![pid])?.iter().enumerate() {
        let fname = format!("{:03}-{}.md", i + 1, slugify(&n.s("title")));
        w(dir.join("notes").join(fname), format!("# {}\n\n_Category: {}_\n\n{}\n", md_escape_title(&n.s("title")), n.s("category"), n.s("body_md")))?;
    }

    let mut files_md = format!("# Documented files — {}\n\nRoot: `{}` ({})\n\n", pj.str_of("name").unwrap_or_default(), pj.str_of("rootPath").unwrap_or_default(), pj.str_of("mode").unwrap_or_default());
    for f in v.db.query(
        "SELECT f.rel_path, f.state, f.sha256, d.purpose, d.notes_md, d.warnings FROM files f JOIN file_docs d ON d.file_id=f.id
         WHERE f.project_id=? AND (d.purpose!='' OR d.notes_md!='' OR d.warnings!='') ORDER BY f.rel_path",
        p![pid],
    )? {
        files_md.push_str(&format!("## `{}`{}\n\n", f.s("rel_path"), if f.s("state") == "missing" { " (missing)" } else { "" }));
        if !f.s("purpose").is_empty() {
            files_md.push_str(&format!("**Purpose:** {}\n\n", f.s("purpose")));
        }
        if !f.s("warnings").is_empty() {
            files_md.push_str(&format!("> ⚠ {}\n\n", f.s("warnings").replace('\n', "\n> ")));
        }
        if !f.s("notes_md").is_empty() {
            files_md.push_str(&format!("{}\n\n", f.s("notes_md")));
        }
        files_md.push_str(&format!("<sub>sha256 {}</sub>\n\n", f.os("sha256").unwrap_or_default()));
    }
    w(dir.join("files.md"), files_md)?;

    let mut tasks_md = String::from("# Tasks & bugs\n\n");
    for t in v.db.query("SELECT * FROM tasks WHERE project_id=? ORDER BY status='done', created_at", p![pid])? {
        tasks_md.push_str(&format!("- [{}] **{}** ({} · {})\n", if t.s("status") == "done" { "x" } else { " " }, md_escape_title(&t.s("title")), t.s("kind"), t.s("status")));
        for (label, col) in [("Description", "description"), ("Steps", "repro_steps"), ("Expected", "expected"), ("Actual", "actual"), ("Resolution", "resolution")] {
            if !t.s(col).is_empty() {
                tasks_md.push_str(&format!("  - {label}: {}\n", t.s(col).replace('\n', "\n    ")));
            }
        }
    }
    w(dir.join("tasks.md"), tasks_md)?;

    let mut tl = String::from("# Timeline\n\n");
    for e in v.db.query("SELECT * FROM timeline WHERE project_id=? ORDER BY at", p![pid])? {
        tl.push_str(&format!("- {} — {}{}\n", e.i("at"), e.s("summary"), if e.s("detail").is_empty() { String::new() } else { format!(" ({})", e.s("detail")) }));
    }
    w(dir.join("timeline.md"), tl)?;
    Ok(())
}

/// Creates a backup folder inside `dest` and verifies it before returning.
pub fn create(v: &Vault, dest: &Path, project_id: Option<&str>) -> DbResult<Json> {
    if !dest.is_dir() {
        return Err(format!("Backup destination {} is not an accessible folder", dest.display()));
    }
    let stamp = now_ms();
    let name = format!("ProjectVault-backup-{stamp}");
    let tmp = dest.join(format!(".{name}.partial"));
    let out = dest.join(&name);
    std::fs::create_dir_all(&tmp).map_err(|e| format!("Cannot write to {}: {e}", dest.display()))?;
    let result = (|| -> DbResult<()> {
        v.db.exec("VACUUM INTO ?", p![tmp.join("vault.db").to_string_lossy().to_string()])?;
        let projects: Vec<(String, String)> = v
            .db
            .query("SELECT id, name FROM projects WHERE ? IS NULL OR id=?", p![project_id, project_id])?
            .iter()
            .map(|r| (r.s("id"), r.s("name")))
            .collect();
        for (id, pname) in &projects {
            export_project(v, id, &tmp.join("export").join(format!("{}-{}", slugify(pname), &id[..8])))?;
        }
        let mut entries = Vec::new();
        let w = crate::files::walk(&tmp, &crate::files::Ignore::parse(""));
        for (rel, s) in &w.files {
            let h = sha256_file(&safe_join(&tmp, rel)?).map_err(|e| e.to_string())?;
            entries.push(Json::obj().with("path", rel.as_str()).with("size", s.size).with("sha256", h));
        }
        let manifest = Json::obj()
            .with("format", "project-vault-backup")
            .with("formatVersion", 1)
            .with("appVersion", crate::APP_VERSION)
            .with("schemaVersion", schema::latest_version())
            .with("createdAt", stamp)
            .with("scope", if project_id.is_some() { "project" } else { "vault" })
            .with("projects", Json::Arr(projects.iter().map(|(i, n)| Json::obj().with("id", i.as_str()).with("name", n.as_str())).collect()))
            .with("includesProjectFiles", false)
            .with("note", "Metadata backup: database, notes, file documentation, tasks and timeline. Project files themselves are not copied; linked projects are stored as references to their folders.")
            .with("files", Json::Arr(entries));
        atomic_write(&tmp.join("manifest.json"), manifest.pretty().as_bytes()).map_err(|e| e.to_string())?;
        Ok(())
    })();
    if let Err(e) = result {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(format!("Backup failed: {e}"));
    }
    std::fs::rename(&tmp, &out).map_err(|e| format!("Backup could not be finalized: {e}"))?;
    let ver = verify(&out)?;
    if !ver.get("ok").and_then(|x| x.as_bool()).unwrap_or(false) {
        return Err(format!("Backup was written but failed verification: {}", ver.to_string()));
    }
    Ok(Json::obj().with("path", out.to_string_lossy().to_string()).with("verification", ver))
}

pub fn verify(dir: &Path) -> DbResult<Json> {
    let m = std::fs::read_to_string(dir.join("manifest.json")).map_err(|e| format!("manifest.json unreadable: {e}"))?;
    let m = Json::parse(&m)?;
    if m.str_of("format").as_deref() != Some("project-vault-backup") {
        return Err("Not a Project Vault backup".into());
    }
    let mut problems = Vec::new();
    let mut checked = 0;
    for f in m.get("files").and_then(|x| x.as_arr()).cloned().unwrap_or_default() {
        let rel = f.str_of("path").unwrap_or_default();
        let path = match safe_join(dir, &rel) {
            Ok(p) => p,
            Err(e) => {
                problems.push(Json::from(e));
                continue;
            }
        };
        match sha256_file(&path) {
            Ok(h) if Some(h.clone()) == f.str_of("sha256") => checked += 1,
            Ok(_) => problems.push(Json::from(format!("{rel}: checksum mismatch"))),
            Err(e) => problems.push(Json::from(format!("{rel}: {e}"))),
        }
    }
    match Db::open_readonly(&dir.join("vault.db")).and_then(|db| db.one("PRAGMA integrity_check", p![])) {
        Ok(Some(r)) if r.vals[0].as_str() == Some("ok") => {}
        Ok(r) => problems.push(Json::from(format!("database integrity: {:?}", r.map(|x| x.vals)))),
        Err(e) => problems.push(Json::from(format!("database: {e}"))),
    }
    Ok(Json::obj().with("ok", problems.is_empty()).with("checkedFiles", checked).with("problems", Json::Arr(problems)).with("manifest", m.get("note").cloned().unwrap_or(Json::Null)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;
    #[test]
    fn backup_and_verify_detects_tamper() {
        let (t, v) = vault();
        let pid = crate::projects::create(&v, crate::projects::NewProject { name: "OMEN v2".into(), description: "".into(), category: "software".into(), status: "".into(), link_path: None, module: Json::obj() }).unwrap().str_of("id").unwrap();
        crate::knowledge::create(&v, &pid, "Server Setup", None, "Installation", "run `start.bat`").unwrap();
        let dest = t.path().join("bk");
        std::fs::create_dir_all(&dest).unwrap();
        let r = create(&v, &dest, None).unwrap();
        let out = PathBuf::from(r.str_of("path").unwrap());
        let note = std::fs::read_dir(out.join("export")).unwrap().next().unwrap().unwrap().path().join("notes/001-server-setup.md");
        assert!(std::fs::read_to_string(&note).unwrap().contains("start.bat"));
        std::fs::write(&note, "tampered").unwrap();
        assert_eq!(verify(&out).unwrap().get("ok").unwrap().as_bool(), Some(false));
    }
}

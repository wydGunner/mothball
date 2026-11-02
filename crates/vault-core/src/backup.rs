//! Vault backups, portable exports and restores.
//!
//! A backup (folder, or the same layout inside a .zip) contains:
//!   vault.db            consistent SQLite snapshot (VACUUM INTO, safe while running)
//!   export/<project>/   human-readable Markdown + JSON for every project:
//!                       project.json, notes/*.md, files.md (every documented file),
//!                       tasks.md, timeline.md
//!   files/<project>/    optional: copies of the project files themselves
//!   manifest.json       app/schema version, per-file SHA-256, which projects and
//!                       whether (and which) project files are included
//!
//! `verify` re-hashes every file and runs an integrity check on the database.
//! Restores never delete anything: project files are recreated or (only when
//! asked) overwritten after keeping the current version; the database is
//! staged and swapped in on the next start, keeping the current one.

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
        tl.push_str(&format!("- {} — {}{}\n", crate::util::iso_utc(e.i("at")), e.s("summary"), if e.s("detail").is_empty() { String::new() } else { format!(" ({})", e.s("detail")) }));
    }
    w(dir.join("timeline.md"), tl)?;
    Ok(())
}

#[derive(Clone, Copy)]
pub struct Opts {
    /// copy the project files themselves, not just metadata
    pub include_files: bool,
    /// skip project files larger than this (MB)
    pub max_file_mb: f64,
    /// deliver as a single .zip instead of a folder
    pub zip: bool,
    /// made by the schedule (named "-auto", subject to the schedule's retention)
    pub auto: bool,
}
impl Default for Opts {
    fn default() -> Self {
        Opts { include_files: false, max_file_mb: 500.0, zip: false, auto: false }
    }
}

/// Creates a backup inside `dest` and verifies it before returning.
pub fn create(v: &Vault, dest: &Path, project_id: Option<&str>) -> DbResult<Json> {
    create_opts(v, dest, project_id, Opts::default())
}

pub fn create_opts(v: &Vault, dest: &Path, project_id: Option<&str>, o: Opts) -> DbResult<Json> {
    if !dest.is_dir() {
        return Err(format!("Backup destination {} is not an accessible folder", dest.display()));
    }
    let stamp = now_ms();
    let readable = crate::util::iso_utc(stamp).replace([' ', ':'], "-").replace("-UTC", "");
    let mut name = format!("Mothball-backup-{readable}{}", if o.auto { "-auto" } else { "" });
    while dest.join(&name).exists() || dest.join(format!("{name}.zip")).exists() {
        name.push('_');
    }
    let tmp = dest.join(format!(".{name}.partial"));
    let out = dest.join(&name);
    std::fs::create_dir_all(&tmp).map_err(|e| format!("Cannot write to {}: {e}", dest.display()))?;
    let mut skipped_files: Vec<Json> = Vec::new();
    let mut copied_files = 0i64;
    let mut copied_bytes = 0i64;
    let result = (|| -> DbResult<()> {
        v.db.exec("VACUUM INTO ?", p![tmp.join("vault.db").to_string_lossy().to_string()])?;
        let projects: Vec<(String, String, String)> = v
            .db
            .query("SELECT id, name, root_path FROM projects WHERE ? IS NULL OR id=?", p![project_id, project_id])?
            .iter()
            .map(|r| (r.s("id"), r.s("name"), r.s("root_path")))
            .collect();
        let mut proj_json = Vec::new();
        for (id, pname, root) in &projects {
            let folder = format!("{}-{}", slugify(pname), &id[..8]);
            export_project(v, id, &tmp.join("export").join(&folder))?;
            let mut pj = Json::obj().with("id", id.as_str()).with("name", pname.as_str()).with("root", root.as_str()).with("folder", folder.as_str());
            if o.include_files {
                let rootp = PathBuf::from(root);
                if !rootp.is_dir() {
                    skipped_files.push(Json::obj().with("project", pname.as_str()).with("path", "").with("reason", "project folder not reachable"));
                } else {
                    let mut n = 0i64;
                    for f in v.db.query("SELECT rel_path, size, sha256 FROM files WHERE project_id=? AND state='present' ORDER BY rel_path", p![id.clone()])? {
                        let rel = f.s("rel_path");
                        if f.i("size") as f64 > o.max_file_mb * 1_048_576.0 {
                            skipped_files.push(Json::obj().with("project", pname.as_str()).with("path", rel.as_str()).with("reason", format!("larger than {} MB", o.max_file_mb)));
                            continue;
                        }
                        let src = safe_join(&rootp, &rel)?;
                        let dst = safe_join(&tmp.join("files").join(&folder), &rel)?;
                        if let Some(d) = dst.parent() {
                            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
                        }
                        match std::fs::copy(&src, &dst) {
                            Ok(b) => {
                                n += 1;
                                copied_bytes += b as i64;
                            }
                            Err(e) => skipped_files.push(Json::obj().with("project", pname.as_str()).with("path", rel.as_str()).with("reason", e.to_string())),
                        }
                    }
                    copied_files += n;
                    pj.set("files", n);
                }
            }
            proj_json.push(pj);
        }
        // project card pictures chosen from outside the projects
        if let Ok(rd) = std::fs::read_dir(v.data_dir.join("pictures")) {
            std::fs::create_dir_all(tmp.join("pictures")).map_err(|e| e.to_string())?;
            for e in rd.flatten() {
                std::fs::copy(e.path(), tmp.join("pictures").join(e.file_name())).map_err(|e| e.to_string())?;
            }
        }
        let mut entries = Vec::new();
        let w = crate::files::walk(&tmp, &crate::files::Ignore::parse(""));
        for (rel, s) in &w.files {
            let h = sha256_file(&safe_join(&tmp, rel)?).map_err(|e| e.to_string())?;
            entries.push(Json::obj().with("path", rel.as_str()).with("size", s.size).with("sha256", h));
        }
        let manifest = Json::obj()
            .with("format", "project-vault-backup")
            .with("formatVersion", 2)
            .with("appVersion", crate::APP_VERSION)
            .with("schemaVersion", schema::latest_version())
            .with("createdAt", stamp)
            .with("scope", if project_id.is_some() { "project" } else { "vault" })
            .with("automatic", o.auto)
            .with("projects", Json::Arr(proj_json))
            .with("includesProjectFiles", o.include_files)
            .with("skippedProjectFiles", Json::Arr(skipped_files.clone()))
            .with(
                "note",
                if o.include_files {
                    "Database, notes, file documentation, tasks, timeline, and copies of the project files (under files/)."
                } else {
                    "Metadata backup: database, notes, file documentation, tasks and timeline. Project files themselves are not copied; linked projects are stored as references to their folders."
                },
            )
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
    let mut path = out.clone();
    if o.zip {
        let zp = dest.join(format!("{name}.zip"));
        crate::zip::zip_dir(&out, &zp).map_err(|e| format!("The backup folder was created and verified at {}, but the ZIP could not be written: {e}", out.display()))?;
        let zv = verify(&zp)?;
        if !zv.get("ok").and_then(|x| x.as_bool()).unwrap_or(false) {
            let _ = std::fs::remove_file(&zp);
            return Err(format!("The ZIP failed verification, so the backup folder was kept at {}", out.display()));
        }
        // the folder was ours and the verified ZIP holds the same files
        let _ = std::fs::remove_dir_all(&out);
        path = zp;
    }
    Ok(Json::obj()
        .with("path", path.to_string_lossy().to_string())
        .with("verification", ver)
        .with("projectFiles", copied_files)
        .with("projectBytes", copied_bytes)
        .with("skipped", Json::Arr(skipped_files)))
}

// ---------------------------------------------------------------- reading a backup (folder or zip)

enum Src {
    Dir(PathBuf),
    Zip(crate::zip::Archive<std::fs::File>),
}
impl Src {
    fn open(path: &Path) -> DbResult<Src> {
        if path.is_dir() {
            Ok(Src::Dir(path.to_path_buf()))
        } else if path.extension().map(|e| e.eq_ignore_ascii_case("zip")).unwrap_or(false) {
            Ok(Src::Zip(crate::zip::Archive::open(path)?))
        } else {
            Err(format!("{} is not a backup folder or .zip", path.display()))
        }
    }
    fn read(&mut self, rel: &str) -> DbResult<Vec<u8>> {
        match self {
            Src::Dir(d) => std::fs::read(safe_join(d, rel)?).map_err(|e| format!("{rel}: {e}")),
            Src::Zip(a) => {
                let e = a.find(rel).ok_or_else(|| format!("{rel}: missing from the ZIP"))?;
                a.read(&e, usize::MAX)
            }
        }
    }
    fn manifest(&mut self) -> DbResult<Json> {
        let m = String::from_utf8(self.read("manifest.json").map_err(|e| format!("manifest.json unreadable: {e}"))?).map_err(|_| "manifest.json is not text".to_string())?;
        let m = Json::parse(&m)?;
        if m.str_of("format").as_deref() != Some("project-vault-backup") {
            return Err("Not a Mothball backup".into());
        }
        Ok(m)
    }
    /// The database as a file on disk (extracted to `scratch` for ZIPs).
    fn db_file(&mut self, scratch: &Path) -> DbResult<PathBuf> {
        match self {
            Src::Dir(d) => Ok(d.join("vault.db")),
            Src::Zip(_) => {
                let data = self.read("vault.db")?;
                let p = scratch.join(format!("pv-restore-{}.db", &crate::util::new_id()[..8]));
                std::fs::write(&p, data).map_err(|e| e.to_string())?;
                Ok(p)
            }
        }
    }
}

/// Checks every file's SHA-256 against the manifest and the database's
/// integrity. Works on backup folders and .zip backups.
pub fn verify(path: &Path) -> DbResult<Json> {
    let mut src = Src::open(path)?;
    let m = src.manifest()?;
    let mut problems = Vec::new();
    let mut checked = 0;
    for f in m.get("files").and_then(|x| x.as_arr()).cloned().unwrap_or_default() {
        let rel = f.str_of("path").unwrap_or_default();
        match src.read(&rel) {
            Ok(data) if Some(crate::util::sha256_bytes(&data)) == f.str_of("sha256") => checked += 1,
            Ok(_) => problems.push(Json::from(format!("{rel}: checksum mismatch"))),
            Err(e) => problems.push(Json::from(e)),
        }
    }
    let scratch = std::env::temp_dir();
    match src.db_file(&scratch) {
        Ok(dbp) => {
            match Db::open_readonly(&dbp).and_then(|db| db.one("PRAGMA integrity_check", p![])) {
                Ok(Some(r)) if r.vals[0].as_str() == Some("ok") => {}
                Ok(r) => problems.push(Json::from(format!("database integrity: {:?}", r.map(|x| x.vals)))),
                Err(e) => problems.push(Json::from(format!("database: {e}"))),
            }
            if matches!(src, Src::Zip(_)) {
                let _ = std::fs::remove_file(&dbp);
            }
        }
        Err(e) => problems.push(Json::from(format!("database: {e}"))),
    }
    Ok(Json::obj().with("ok", problems.is_empty()).with("checkedFiles", checked).with("problems", Json::Arr(problems)).with("manifest", m.get("note").cloned().unwrap_or(Json::Null)))
}

/// What a backup holds, and how its projects map onto the vault as it is now.
pub fn inspect(v: &Vault, path: &Path) -> DbResult<Json> {
    let mut src = Src::open(path)?;
    let m = src.manifest()?;
    let ver = verify(path)?;
    let files: Vec<Json> = m.get("files").and_then(|x| x.as_arr()).cloned().unwrap_or_default();
    let mut projects = Vec::new();
    for pj in m.get("projects").and_then(|x| x.as_arr()).cloned().unwrap_or_default() {
        let id = pj.str_of("id").unwrap_or_default();
        let folder = pj.str_of("folder").unwrap_or_default();
        let prefix = format!("files/{folder}/");
        let n = if folder.is_empty() { 0 } else { files.iter().filter(|f| f.str_of("path").map(|p| p.starts_with(&prefix)).unwrap_or(false)).count() };
        let now = v.db.one("SELECT name, root_path FROM projects WHERE id=?", p![id.clone()])?;
        projects.push(
            pj.clone()
                .with("fileCopies", n as i64)
                .with("existsNow", now.is_some())
                .with("currentName", now.as_ref().map(|r| r.s("name")))
                .with("restoreTo", now.map(|r| r.s("root_path")).unwrap_or_else(|| pj.str_of("root").unwrap_or_default())),
        );
    }
    Ok(Json::obj()
        .with("path", path.to_string_lossy().to_string())
        .with("createdAt", m.get("createdAt").cloned().unwrap_or(Json::Null))
        .with("appVersion", m.get("appVersion").cloned().unwrap_or(Json::Null))
        .with("schemaVersion", m.get("schemaVersion").cloned().unwrap_or(Json::Null))
        .with("automatic", m.get("automatic").cloned().unwrap_or(Json::from(false)))
        .with("includesProjectFiles", m.get("includesProjectFiles").cloned().unwrap_or(Json::from(false)))
        .with("projects", Json::Arr(projects))
        .with("verification", ver))
}

fn project_entry(m: &Json, project_id: &str) -> DbResult<Json> {
    m.get("projects")
        .and_then(|x| x.as_arr())
        .and_then(|a| a.iter().find(|p| p.str_of("id").as_deref() == Some(project_id)).cloned())
        .ok_or_else(|| "That project is not in this backup".to_string())
}

fn restore_target(v: &Vault, pj: &Json) -> DbResult<PathBuf> {
    let id = pj.str_of("id").unwrap_or_default();
    Ok(match v.db.one("SELECT root_path FROM projects WHERE id=?", p![id])? {
        Some(r) => PathBuf::from(r.s("root_path")),
        None => PathBuf::from(pj.str_of("root").unwrap_or_default()),
    })
}

/// Per-file plan for restoring one project's files: recreate (missing now),
/// differs (would only be replaced when asked), or unchanged.
pub fn restore_preview(v: &Vault, path: &Path, project_id: &str) -> DbResult<Json> {
    let mut src = Src::open(path)?;
    let m = src.manifest()?;
    let pj = project_entry(&m, project_id)?;
    let folder = pj.str_of("folder").unwrap_or_default();
    let prefix = format!("files/{folder}/");
    let root = restore_target(v, &pj)?;
    let mut items = Vec::new();
    let (mut recreate, mut differs, mut same) = (0, 0, 0);
    for f in m.get("files").and_then(|x| x.as_arr()).cloned().unwrap_or_default() {
        let Some(rel) = f.str_of("path").and_then(|p| p.strip_prefix(&prefix).map(String::from)) else { continue };
        let target = safe_join(&root, &rel)?;
        let state = if !target.exists() {
            recreate += 1;
            "recreate"
        } else if sha256_file(&target).ok() == f.str_of("sha256") {
            same += 1;
            continue;
        } else {
            differs += 1;
            "differs"
        };
        items.push(Json::obj().with("path", rel).with("action", state).with("size", f.get("size").cloned().unwrap_or(Json::from(0))));
    }
    Ok(Json::obj()
        .with("root", root.to_string_lossy().to_string())
        .with("rootExists", root.is_dir())
        .with("recreate", recreate)
        .with("differs", differs)
        .with("unchanged", same)
        .with("items", Json::Arr(items)))
}

/// Restores one project's files from a backup into its folder. Missing files
/// are recreated; files that differ are replaced only with `overwrite`, and
/// then the current content is kept as a version first. Nothing is deleted.
pub fn restore_files(v: &Vault, path: &Path, project_id: &str, overwrite: bool) -> DbResult<Json> {
    let mut src = Src::open(path)?;
    let m = src.manifest()?;
    let pj = project_entry(&m, project_id)?;
    let folder = pj.str_of("folder").unwrap_or_default();
    let prefix = format!("files/{folder}/");
    let root = restore_target(v, &pj)?;
    if !root.is_dir() {
        std::fs::create_dir_all(&root).map_err(|e| format!("The project folder {} can't be created: {e}", root.display()))?;
    }
    let tracked = v.db.one("SELECT id FROM projects WHERE id=?", p![project_id])?.is_some();
    let (mut restored, mut kept, mut errors) = (Vec::new(), 0, Vec::new());
    for f in m.get("files").and_then(|x| x.as_arr()).cloned().unwrap_or_default() {
        let Some(rel) = f.str_of("path").and_then(|p| p.strip_prefix(&prefix).map(String::from)) else { continue };
        let want = f.str_of("sha256").unwrap_or_default();
        let res = (|| -> DbResult<bool> {
            let target = safe_join(&root, &rel)?;
            if target.exists() {
                if sha256_file(&target).ok().as_deref() == Some(want.as_str()) {
                    return Ok(false);
                }
                if !overwrite {
                    kept += 1;
                    return Ok(false);
                }
                // keep what's there now before replacing it
                if tracked {
                    if let Some(row) = v.db.one("SELECT * FROM files WHERE project_id=? AND rel_path=?", p![project_id, rel.clone()])? {
                        let cur = sha256_file(&target).map_err(|e| e.to_string())?;
                        crate::versions::ensure_stored(v, &row, &target, &cur, "before-restore")?;
                    }
                }
            }
            let data = src.read(&format!("{prefix}{rel}"))?;
            if crate::util::sha256_bytes(&data) != want {
                return Err("the copy in the backup is damaged".into());
            }
            let dir = target.parent().ok_or("bad path")?;
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
            let tmp = dir.join(format!(".pv-tmp-{}", &crate::util::new_id()[..8]));
            std::fs::write(&tmp, &data).map_err(|e| e.to_string())?;
            std::fs::rename(&tmp, &target).map_err(|e| {
                let _ = std::fs::remove_file(&tmp);
                format!("could not write (open in another program?): {e}")
            })?;
            if sha256_file(&target).ok().as_deref() != Some(want.as_str()) {
                return Err("content differs after writing".into());
            }
            Ok(true)
        })();
        match res {
            Ok(true) => restored.push(Json::from(rel)),
            Ok(false) => {}
            Err(e) => errors.push(Json::from(format!("{rel}: {e}"))),
        }
    }
    if tracked {
        crate::files::scan(v, project_id)?;
        crate::timeline::add(v, project_id, "backup", "Restored project files from a backup", &format!("{} restored, {} left as they are", restored.len(), kept), None, false)?;
    }
    Ok(Json::obj().with("restored", Json::Arr(restored)).with("keptDifferent", kept).with("errors", Json::Arr(errors)).with("root", root.to_string_lossy().to_string()))
}

/// Stages the backup's database to replace the current one on the next start.
/// The backup is verified first; the current database is kept in backups/.
pub fn stage_database_restore(v: &Vault, path: &Path) -> DbResult<Json> {
    let ver = verify(path)?;
    if !ver.get("ok").and_then(|x| x.as_bool()).unwrap_or(false) {
        return Err("This backup failed verification, so its database won't be restored.".into());
    }
    let mut src = Src::open(path)?;
    let m = src.manifest()?;
    if m.get("schemaVersion").and_then(|x| x.as_i64()).unwrap_or(0) > schema::latest_version() {
        return Err("This backup was made by a newer version of Mothball. Update the app first.".into());
    }
    let data = src.read("vault.db")?;
    let pending = v.data_dir.join(PENDING_DB);
    atomic_write(&pending, &data).map_err(|e| e.to_string())?;
    match Db::open_readonly(&pending).and_then(|db| db.one("PRAGMA integrity_check", p![])) {
        Ok(Some(r)) if r.vals[0].as_str() == Some("ok") => {}
        _ => {
            let _ = std::fs::remove_file(&pending);
            return Err("The staged database failed its integrity check; nothing will change.".into());
        }
    }
    // card pictures that belong to the restored database
    let names: Vec<String> = m.get("files").and_then(|x| x.as_arr()).cloned().unwrap_or_default().iter().filter_map(|f| f.str_of("path")).filter(|p| p.starts_with("pictures/")).collect();
    if !names.is_empty() {
        let dir = v.data_dir.join("pictures");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        for n in names {
            let data = src.read(&n)?;
            let fname = n.trim_start_matches("pictures/");
            if !fname.contains('/') && !fname.contains("..") {
                atomic_write(&dir.join(fname), &data).map_err(|e| e.to_string())?;
            }
        }
    }
    Ok(Json::obj().with("staged", true).with("restartRequired", true))
}

pub fn cancel_database_restore(v: &Vault) -> DbResult<Json> {
    let p = v.data_dir.join(PENDING_DB);
    if p.exists() {
        std::fs::remove_file(&p).map_err(|e| e.to_string())?;
    }
    Ok(Json::obj().with("cancelled", true))
}

pub const PENDING_DB: &str = "restore-pending.db";

/// Called while opening the vault: swaps in a staged database, keeping the
/// current one as backups/before-restore-<time>.db.
pub fn apply_pending_restore(data_dir: &Path) -> DbResult<Option<PathBuf>> {
    let pending = data_dir.join(PENDING_DB);
    if !pending.exists() {
        return Ok(None);
    }
    let cur = data_dir.join("vault.db");
    let kept = data_dir.join("backups").join(format!("before-restore-{}.db", now_ms()));
    if cur.exists() {
        {
            let db = Db::open(&cur)?;
            db.batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
        }
        std::fs::rename(&cur, &kept).map_err(|e| format!("Could not set the current database aside: {e}"))?;
        for ext in ["vault.db-wal", "vault.db-shm"] {
            let _ = std::fs::remove_file(data_dir.join(ext));
        }
    }
    if let Err(e) = std::fs::rename(&pending, &cur) {
        // put the original back
        if kept.exists() {
            let _ = std::fs::rename(&kept, &cur);
        }
        return Err(format!("Could not apply the restored database: {e}"));
    }
    Ok(Some(kept))
}

// ---------------------------------------------------------------- schedule

/// Scheduled backups run while the app is open. Retention only applies to
/// backups the schedule itself made ("-auto"), and only when the user set it.
pub fn schedule_get(v: &Vault) -> DbResult<Json> {
    let s = v.db.one("SELECT value FROM meta WHERE key='backup_schedule'", p![])?.map(|r| r.s("value")).unwrap_or_default();
    let j = Json::parse(&s).unwrap_or(Json::obj());
    let mut out = Json::obj()
        .with("enabled", j.get("enabled").and_then(|x| x.as_bool()).unwrap_or(false))
        .with("everyDays", j.get("everyDays").and_then(|x| x.as_f64()).unwrap_or(1.0))
        .with("dest", j.str_of("dest").unwrap_or_default())
        .with("includeFiles", j.get("includeFiles").and_then(|x| x.as_bool()).unwrap_or(false))
        .with("maxFileMb", j.get("maxFileMb").and_then(|x| x.as_f64()).unwrap_or(500.0))
        .with("zip", j.get("zip").and_then(|x| x.as_bool()).unwrap_or(true))
        .with("keep", j.get("keep").and_then(|x| x.as_i64()).unwrap_or(0))
        .with("lastRun", j.get("lastRun").cloned().unwrap_or(Json::Null))
        .with("lastResult", j.get("lastResult").cloned().unwrap_or(Json::Null));
    let next = match (out.get("enabled").and_then(|x| x.as_bool()), j.get("lastRun").and_then(|x| x.as_i64())) {
        (Some(true), Some(l)) => Json::from(l + (out.get("everyDays").and_then(|x| x.as_f64()).unwrap_or(1.0) * 86_400_000.0) as i64),
        (Some(true), None) => Json::from(now_ms()),
        _ => Json::Null,
    };
    out.set("nextRun", next);
    Ok(out)
}

pub fn schedule_set(v: &Vault, s: &Json) -> DbResult<Json> {
    let mut cur = schedule_get(v)?;
    for k in ["enabled", "everyDays", "dest", "includeFiles", "maxFileMb", "zip", "keep"] {
        if let Some(x) = s.get(k) {
            cur.set(k, x.clone());
        }
    }
    if cur.get("enabled").and_then(|x| x.as_bool()).unwrap_or(false) {
        let d = cur.str_of("dest").unwrap_or_default();
        if d.trim().is_empty() || !Path::new(&d).is_dir() {
            return Err("Choose a backup folder that exists before turning on scheduled backups".into());
        }
    }
    let days = cur.get("everyDays").and_then(|x| x.as_f64()).unwrap_or(1.0);
    if !(0.04..=90.0).contains(&days) {
        return Err("The interval must be between 1 hour and 90 days".into());
    }
    let keep = cur.get("keep").and_then(|x| x.as_i64()).unwrap_or(0);
    if keep < 0 {
        return Err("Keep must be 0 (keep all) or more".into());
    }
    let mut store = cur.clone();
    store.remove("nextRun");
    v.db.exec("INSERT INTO meta(key,value) VALUES ('backup_schedule',?) ON CONFLICT(key) DO UPDATE SET value=excluded.value", p![store.to_string()])?;
    schedule_get(v)
}

/// Runs the scheduled backup if it is due. Returns what happened.
pub fn run_due(v: &Vault, force: bool) -> DbResult<Json> {
    let s = schedule_get(v)?;
    if !s.get("enabled").and_then(|x| x.as_bool()).unwrap_or(false) && !force {
        return Ok(Json::obj().with("ran", false).with("reason", "off"));
    }
    if !force && s.get("nextRun").and_then(|x| x.as_i64()).map(|n| n > now_ms()).unwrap_or(true) {
        return Ok(Json::obj().with("ran", false).with("reason", "not due"));
    }
    let dest = PathBuf::from(s.str_of("dest").unwrap_or_default());
    let o = Opts {
        include_files: s.get("includeFiles").and_then(|x| x.as_bool()).unwrap_or(false),
        max_file_mb: s.get("maxFileMb").and_then(|x| x.as_f64()).unwrap_or(500.0),
        zip: s.get("zip").and_then(|x| x.as_bool()).unwrap_or(true),
        auto: true,
    };
    let res = create_opts(v, &dest, None, o);
    let mut removed = Vec::new();
    let result = match &res {
        Ok(r) => {
            let keep = s.get("keep").and_then(|x| x.as_i64()).unwrap_or(0);
            if keep > 0 {
                removed = prune_auto(&dest, keep as usize);
            }
            Json::obj().with("ok", true).with("path", r.get("path").cloned().unwrap_or(Json::Null)).with("removedOld", removed.len() as i64)
        }
        Err(e) => Json::obj().with("ok", false).with("error", e.as_str()),
    };
    let mut cur = s.clone();
    cur.set("lastRun", now_ms());
    cur.set("lastResult", result.clone());
    cur.remove("nextRun");
    v.db.exec("INSERT INTO meta(key,value) VALUES ('backup_schedule',?) ON CONFLICT(key) DO UPDATE SET value=excluded.value", p![cur.to_string()])?;
    Ok(Json::obj().with("ran", true).with("result", result).with("removed", Json::Arr(removed.into_iter().map(Json::from).collect())))
}

/// Deletes the oldest automatic backups beyond `keep`. Only touches entries
/// named Mothball-backup-…-auto (or ProjectVault-backup-…-auto, made before
/// the rename) whose manifest says they're automatic.
fn prune_auto(dest: &Path, keep: usize) -> Vec<String> {
    let Ok(rd) = std::fs::read_dir(dest) else { return vec![] };
    let mut autos: Vec<(String, PathBuf)> = rd
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            let stem = name.trim_end_matches(".zip");
            ((name.starts_with("Mothball-backup-") || name.starts_with("ProjectVault-backup-")) && stem.trim_end_matches('_').ends_with("-auto")).then(|| (name, e.path()))
        })
        .filter(|(_, p)| Src::open(p).and_then(|mut s| s.manifest()).map(|m| m.get("automatic").and_then(|x| x.as_bool()).unwrap_or(false)).unwrap_or(false))
        .collect();
    autos.sort();
    let n = autos.len().saturating_sub(keep);
    let mut removed = Vec::new();
    for (name, p) in autos.into_iter().take(n) {
        let r = if p.is_dir() { std::fs::remove_dir_all(&p) } else { std::fs::remove_file(&p) };
        if r.is_ok() {
            removed.push(name);
        }
    }
    removed
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

    #[test]
    fn files_zip_restore_and_schedule() {
        let (t, v) = vault();
        let d = t.path().join("proj");
        std::fs::create_dir_all(d.join("scripts")).unwrap();
        std::fs::write(d.join("scripts/a.gsc"), "init() {}").unwrap();
        std::fs::write(d.join("big.bin"), vec![1u8; 3000]).unwrap();
        let pid = crate::projects::create(&v, crate::projects::NewProject { name: "OMEN".into(), description: "".into(), category: "software".into(), status: "".into(), link_path: Some(d.clone()), module: Json::obj() }).unwrap().str_of("id").unwrap();
        crate::files::scan(&v, &pid).unwrap();
        let dest = t.path().join("bk");
        std::fs::create_dir_all(&dest).unwrap();
        let r = create_opts(&v, &dest, None, Opts { include_files: true, max_file_mb: 0.001, zip: true, auto: false }).unwrap();
        let zp = PathBuf::from(r.str_of("path").unwrap());
        assert!(zp.extension().unwrap() == "zip" && zp.is_file());
        assert_eq!(r.get("projectFiles").unwrap().as_i64(), Some(1), "big.bin skipped by size");
        assert_eq!(verify(&zp).unwrap().get("ok").unwrap().as_bool(), Some(true));
        // restore: deleted file comes back; changed file is kept unless asked
        std::fs::remove_file(d.join("scripts/a.gsc")).unwrap();
        let pv = restore_preview(&v, &zp, &pid).unwrap();
        assert_eq!(pv.get("recreate").unwrap().as_i64(), Some(1));
        restore_files(&v, &zp, &pid, false).unwrap();
        assert_eq!(std::fs::read_to_string(d.join("scripts/a.gsc")).unwrap(), "init() {}");
        std::fs::write(d.join("scripts/a.gsc"), "changed").unwrap();
        let r2 = restore_files(&v, &zp, &pid, false).unwrap();
        assert_eq!(r2.get("keptDifferent").unwrap().as_i64(), Some(1));
        assert_eq!(std::fs::read_to_string(d.join("scripts/a.gsc")).unwrap(), "changed");
        restore_files(&v, &zp, &pid, true).unwrap();
        assert_eq!(std::fs::read_to_string(d.join("scripts/a.gsc")).unwrap(), "init() {}");
        let fid = v.db.one("SELECT id FROM files WHERE rel_path='scripts/a.gsc'", p![]).unwrap().unwrap().s("id");
        let changed_sha = crate::util::sha256_bytes(b"changed");
        assert!(crate::versions::list(&v, &fid).unwrap().as_arr().unwrap().iter().any(|x| x.str_of("sha256").as_deref() == Some(changed_sha.as_str())), "changed content kept first");
        // schedule: off by default; needs a real folder; keep prunes only automatic ones
        assert_eq!(run_due(&v, false).unwrap().get("ran").unwrap().as_bool(), Some(false));
        assert!(schedule_set(&v, &Json::parse(r#"{"enabled":true,"dest":"/no/such/dir"}"#).unwrap()).is_err());
        schedule_set(&v, &Json::obj().with("enabled", true).with("dest", dest.to_string_lossy().to_string()).with("keep", 2).with("zip", false)).unwrap();
        for _ in 0..3 {
            run_due(&v, true).unwrap();
        }
        let names: Vec<String> = std::fs::read_dir(&dest).unwrap().flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect();
        assert_eq!(names.iter().filter(|n| n.contains("-auto")).count(), 2, "{names:?}");
        assert!(names.iter().any(|n| n.ends_with(".zip") && !n.contains("-auto")), "manual backup never pruned");
        assert_eq!(run_due(&v, false).unwrap().get("ran").unwrap().as_bool(), Some(false), "not due right after a run");
        // database restore is staged, then applied on the next open
        crate::knowledge::create(&v, &pid, "Added after backup", None, "General", "x").unwrap();
        stage_database_restore(&v, &zp).unwrap();
        let data_dir = v.data_dir.clone();
        drop(v);
        let v2 = Vault::open(&data_dir).unwrap();
        assert_eq!(v2.db.scalar_i64("SELECT count(*) FROM notes WHERE title='Added after backup'", p![]).unwrap(), 0);
        assert!(std::fs::read_dir(data_dir.join("backups")).unwrap().flatten().any(|e| e.file_name().to_string_lossy().starts_with("before-restore-")));
    }
}

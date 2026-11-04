//! Project-wide restore points. A snapshot records the exact content
//! (SHA-256) of every included file; contents live in the same deduplicated
//! object store as file versions, so unchanged files cost nothing extra.
//!
//! Restoring never deletes files that were added after the snapshot, and an
//! automatic snapshot of the current state is taken first so a restore can be
//! undone.

use crate::db::DbResult;
use crate::json::Json;
use crate::util::{new_id, now_ms, safe_join, sha256_file};
use crate::{files, p, projects, timeline, versions, Vault};

pub fn create(v: &Vault, project_id: &str, name: &str, description: &str, max_file_mb: f64) -> DbResult<Json> {
    create_expiring(v, project_id, name, description, max_file_mb, None)
}

/// A snapshot that deletes itself at `expires_at` (ms, UTC) unless kept.
pub fn create_expiring(v: &Vault, project_id: &str, name: &str, description: &str, max_file_mb: f64, expires_at: Option<i64>) -> DbResult<Json> {
    if let Some(t) = expires_at {
        if t <= now_ms() {
            return Err("The time a temporary snapshot deletes itself has to be in the future".into());
        }
    }
    versions::check_storage(v)?;
    let rep = files::scan(v, project_id)?;
    if !rep.errors.is_empty() && rep.total == 0 {
        return Err(format!("The project could not be read: {}", rep.errors.join("; ")));
    }
    let root = projects::root_of(v, project_id)?;
    let max = (max_file_mb.max(0.01) * 1048576.0) as i64;
    let rows = v.db.query("SELECT id, rel_path, size, sha256 FROM files WHERE project_id=? AND state='present' ORDER BY rel_path", p![project_id])?;
    let id = new_id();
    let mut included = Vec::new();
    let mut skipped = Vec::new();
    let mut total = 0i64;
    for r in &rows {
        let rel = r.s("rel_path");
        if r.i("size") > max {
            skipped.push(Json::obj().with("path", rel).with("reason", format!("larger than {max_file_mb} MB")));
            continue;
        }
        let path = safe_join(&root, &rel)?;
        // hash now: the snapshot must describe exactly what gets stored
        let sha = match sha256_file(&path) {
            Ok(h) => h,
            Err(e) => {
                skipped.push(Json::obj().with("path", rel).with("reason", e.to_string()));
                continue;
            }
        };
        if let Err(e) = versions::store(v, &path, &sha) {
            skipped.push(Json::obj().with("path", rel).with("reason", e));
            continue;
        }
        total += r.i("size");
        included.push((r.s("id"), rel, sha, r.i("size")));
    }
    let name = if name.trim().is_empty() { format!("Snapshot {}", crate::util::iso_utc(now_ms())) } else { name.trim().to_string() };
    v.db.tx(|db| {
        db.exec(
            "INSERT INTO snapshots(id,project_id,name,description,created_at,file_count,total_bytes,skipped_json,expires_at) VALUES (?,?,?,?,?,?,?,?,?)",
            p![id.clone(), project_id, name.clone(), description.trim(), now_ms(), included.len() as i64, total, Json::Arr(skipped.clone()).to_string(), expires_at],
        )?;
        for (fid, rel, sha, size) in &included {
            db.exec("INSERT INTO snapshot_files(snapshot_id,file_id,rel_path,sha256,size) VALUES (?,?,?,?,?)", p![id.clone(), fid.clone(), rel.clone(), sha.clone(), *size])?;
        }
        Ok(())
    })?;
    write_manifest(v, &id)?;
    crate::search::index(v, "snapshot", &id, project_id, &name, &format!("{} snapshot {} files", description, included.len()))?;
    let when = match expires_at {
        Some(t) => format!(", deletes itself {}", crate::util::iso_utc(t)),
        None => String::new(),
    };
    timeline::add(v, project_id, "snapshot", &format!("Snapshot created: {name}"), &format!("{} files, {} skipped{when}", included.len(), skipped.len()), None, false)?;
    get(v, &id)
}

/// Portable manifest: storage-root/snapshots/<project>/<time>_<name>_<id>.json
fn write_manifest(v: &Vault, id: &str) -> DbResult<()> {
    let s = v.db.one("SELECT s.*, p.name AS pname, p.root_path FROM snapshots s JOIN projects p ON p.id=s.project_id WHERE s.id=?", p![id])?.ok_or("Snapshot not found")?;
    let files: Vec<Json> = v
        .db
        .query("SELECT rel_path, sha256, size FROM snapshot_files WHERE snapshot_id=? ORDER BY rel_path", p![id])?
        .iter()
        .map(|r| Json::obj().with("path", r.s("rel_path")).with("sha256", r.s("sha256")).with("size", r.i("size")))
        .collect();
    let j = Json::obj()
        .with("format", "project-vault-snapshot")
        .with("formatVersion", 1)
        .with("id", id)
        .with("name", s.s("name"))
        .with("description", s.s("description"))
        .with("createdAtUtc", crate::util::iso_utc(s.i("created_at")))
        .with("project", Json::obj().with("id", s.s("project_id")).with("name", s.s("pname")).with("root", s.s("root_path")))
        .with("objectsDir", "objects/<first 2 hex>/<remaining 62 hex>")
        .with("files", Json::Arr(files))
        .with("skipped", Json::parse(&s.s("skipped_json")).unwrap_or(Json::Arr(vec![])));
    let dir = versions::storage_root(v).join("snapshots").join(format!("{}-{}", crate::util::slugify(&s.s("pname")), &s.s("project_id")[..8]));
    let ts = crate::util::iso_utc(s.i("created_at")).replace([' ', ':'], "-").replace("-UTC", "Z");
    crate::util::atomic_write(&dir.join(format!("{ts}_{}_{}.json", crate::util::slugify(&s.s("name")), &id[..8])), j.pretty().as_bytes()).map_err(|e| e.to_string())
}

fn snap_json(r: &crate::db::Row) -> Json {
    Json::obj()
        .with("id", r.s("id"))
        .with("projectId", r.s("project_id"))
        .with("name", r.s("name"))
        .with("description", r.s("description"))
        .with("at", r.i("created_at"))
        .with("fileCount", r.i("file_count"))
        .with("bytes", r.i("total_bytes"))
        .with("skipped", Json::parse(&r.s("skipped_json")).unwrap_or(Json::Arr(vec![])))
        .with("expiresAt", r.oi("expires_at"))
}

pub fn get(v: &Vault, id: &str) -> DbResult<Json> {
    let r = v.db.one("SELECT * FROM snapshots WHERE id=?", p![id])?.ok_or("Snapshot not found")?;
    Ok(snap_json(&r))
}

pub fn list(v: &Vault, project_id: &str) -> DbResult<Json> {
    Ok(Json::Arr(v.db.query("SELECT * FROM snapshots WHERE project_id=? ORDER BY created_at DESC", p![project_id])?.iter().map(snap_json).collect()))
}

/// What a restore would do, file by file — nothing is changed.
pub fn preview(v: &Vault, id: &str) -> DbResult<Json> {
    let s = v.db.one("SELECT * FROM snapshots WHERE id=?", p![id])?.ok_or("Snapshot not found")?;
    let root = projects::root_of(v, &s.s("project_id"))?;
    files::scan(v, &s.s("project_id"))?; // so files added on disk since the last scan are reported as kept
    let mut change = Vec::new();
    let mut same = 0;
    for r in v.db.query("SELECT rel_path, sha256 FROM snapshot_files WHERE snapshot_id=?", p![id])? {
        let path = safe_join(&root, &r.s("rel_path"))?;
        let state = if !path.exists() {
            "recreate"
        } else if sha256_file(&path).ok().as_deref() == Some(r.s("sha256").as_str()) {
            same += 1;
            continue;
        } else {
            "overwrite"
        };
        change.push(Json::obj().with("path", r.s("rel_path")).with("action", state));
    }
    // files that exist now but weren't in the snapshot: left alone
    let newer: Vec<Json> = v
        .db
        .query(
            "SELECT rel_path FROM files WHERE project_id=? AND state='present' AND rel_path NOT IN (SELECT rel_path FROM snapshot_files WHERE snapshot_id=?) ORDER BY rel_path",
            p![s.s("project_id"), id],
        )?
        .iter()
        .map(|r| Json::from(r.s("rel_path")))
        .collect();
    Ok(Json::obj().with("changes", Json::Arr(change)).with("unchanged", same).with("leftAlone", Json::Arr(newer)))
}

/// Restores every changed/missing file to its snapshot content. Takes an
/// automatic "before restore" snapshot first. Never deletes newer files.
pub fn restore(v: &Vault, id: &str) -> DbResult<Json> {
    let s = v.db.one("SELECT * FROM snapshots WHERE id=?", p![id])?.ok_or("Snapshot not found")?;
    let pid = s.s("project_id");
    let root = projects::root_of(v, &pid)?;
    if !root.is_dir() {
        return Err("Project folder is not reachable".into());
    }
    // all stored contents must be intact before touching anything
    let entries = v.db.query("SELECT rel_path, sha256 FROM snapshot_files WHERE snapshot_id=?", p![id])?;
    for r in &entries {
        let obj = versions::object_path(v, &r.s("sha256"))?;
        if sha256_file(&obj).ok().as_deref() != Some(r.s("sha256").as_str()) {
            return Err(format!("The stored copy of {} is missing or damaged; nothing was restored.", r.s("rel_path")));
        }
    }
    let undo = create(v, &pid, &format!("Before restoring “{}”", s.s("name")), "Automatic snapshot taken before a restore", 1_000_000.0)?;
    let mut restored = Vec::new();
    let mut errors = Vec::new();
    for r in &entries {
        let rel = r.s("rel_path");
        let res = (|| -> DbResult<bool> {
            let target = safe_join(&root, &rel)?;
            if target.exists() && sha256_file(&target).ok().as_deref() == Some(r.s("sha256").as_str()) {
                return Ok(false);
            }
            let dir = target.parent().ok_or("bad path")?;
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
            let tmp = dir.join(format!(".pv-tmp-{}", &new_id()[..8]));
            std::fs::copy(versions::object_path(v, &r.s("sha256"))?, &tmp).map_err(|e| e.to_string())?;
            if sha256_file(&tmp).ok().as_deref() != Some(r.s("sha256").as_str()) {
                let _ = std::fs::remove_file(&tmp);
                return Err("copy verification failed".into());
            }
            std::fs::rename(&tmp, &target).map_err(|e| {
                let _ = std::fs::remove_file(&tmp);
                format!("could not replace (open in another program?): {e}")
            })?;
            if sha256_file(&target).ok().as_deref() != Some(r.s("sha256").as_str()) {
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
    files::scan(v, &pid)?;
    timeline::add(v, &pid, "snapshot", &format!("Restored snapshot “{}”", s.s("name")), &format!("{} files restored, {} errors", restored.len(), errors.len()), None, false)?;
    Ok(Json::obj().with("restored", Json::Arr(restored)).with("errors", Json::Arr(errors)).with("undoSnapshot", undo))
}

/// Keep a temporary snapshot (None) or change when it deletes itself.
pub fn set_expiry(v: &Vault, id: &str, expires_at: Option<i64>) -> DbResult<Json> {
    if let Some(t) = expires_at {
        if t <= now_ms() {
            return Err("Pick a time in the future".into());
        }
    }
    let n = v.db.exec("UPDATE snapshots SET expires_at=? WHERE id=?", p![expires_at, id])?;
    if n == 0 {
        return Err("Snapshot not found".into());
    }
    get(v, id)
}

/// Deletes temporary snapshots whose time is up. Runs when the vault opens
/// and on every watcher pass. Returns how many were removed.
pub fn expire_due(v: &Vault) -> DbResult<usize> {
    let due = v.db.query("SELECT id, project_id, name FROM snapshots WHERE expires_at IS NOT NULL AND expires_at<=?", p![now_ms()])?;
    for s in &due {
        delete(v, &s.s("id"))?;
        timeline::add(v, &s.s("project_id"), "snapshot", &format!("Temporary snapshot “{}” deleted itself", s.s("name")), "Its time was up", None, false)?;
    }
    Ok(due.len())
}

pub fn delete(v: &Vault, id: &str) -> DbResult<()> {
    // its portable manifest goes too, so temporary snapshots don't leave files behind
    if let Some(s) = v.db.one("SELECT p.name AS pname, s.project_id FROM snapshots s JOIN projects p ON p.id=s.project_id WHERE s.id=?", p![id])? {
        let dir = versions::storage_root(v).join("snapshots").join(format!("{}-{}", crate::util::slugify(&s.s("pname")), &s.s("project_id")[..8]));
        let tail = format!("_{}.json", &id[..8.min(id.len())]);
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for e in rd.flatten() {
                if e.file_name().to_string_lossy().ends_with(&tail) {
                    let _ = std::fs::remove_file(e.path());
                }
            }
        }
    }
    v.db.exec("DELETE FROM snapshots WHERE id=?", p![id])?;
    crate::search::unindex(v, "snapshot", id)?;
    versions::gc(v)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projects::{create as cp, NewProject};
    use crate::testutil::*;
    use std::fs;
    /// User request: snapshot before edits you're unsure of; it deletes
    /// itself after a while unless you keep it.
    #[test]
    fn temporary_snapshots_delete_themselves_unless_kept() {
        let (t, v) = vault();
        let d = t.path().join("p");
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join("menu.gsc"), "before risky edit").unwrap();
        let pid = cp(&v, NewProject { name: "P".into(), description: "".into(), category: "software".into(), status: "".into(), link_path: Some(d.clone()), module: Json::obj() }).unwrap().str_of("id").unwrap();
        assert!(create_expiring(&v, &pid, "x", "", 100.0, Some(now_ms() - 1)).is_err(), "expiry in the past is refused");
        let day = now_ms() + 86_400_000;
        let a = create_expiring(&v, &pid, "before menu rework", "", 100.0, Some(day)).unwrap();
        let b = create_expiring(&v, &pid, "keep this one", "", 100.0, Some(day)).unwrap();
        let forever = create(&v, &pid, "release 1", "", 100.0).unwrap();
        assert_eq!(a.get("expiresAt").and_then(|x| x.as_i64()), Some(day));
        assert_eq!(forever.get("expiresAt").and_then(|x| x.as_i64()), None);
        let manifests = || fs::read_dir(versions::storage_root(&v).join("snapshots")).unwrap().flatten().flat_map(|d| fs::read_dir(d.path()).unwrap().flatten()).count();
        assert_eq!(manifests(), 3);
        // the risky edit went fine
        fs::write(d.join("menu.gsc"), "after risky edit").unwrap();
        assert_eq!(expire_due(&v).unwrap(), 0, "not due yet");
        set_expiry(&v, &b.str_of("id").unwrap(), None).unwrap(); // "Keep"
        // a day later
        v.db.exec("UPDATE snapshots SET expires_at=? WHERE expires_at IS NOT NULL", p![now_ms() - 1]).unwrap();
        assert_eq!(expire_due(&v).unwrap(), 1);
        let names: Vec<String> = list(&v, &pid).unwrap().as_arr().unwrap().iter().map(|s| s.str_of("name").unwrap()).collect();
        assert_eq!(names.len(), 2);
        assert!(!names.contains(&"before menu rework".to_string()));
        assert_eq!(manifests(), 2, "its manifest file is removed too");
        // the content the kept snapshots need is still stored and restorable
        fs::write(d.join("menu.gsc"), "broken").unwrap();
        restore(&v, &b.str_of("id").unwrap()).unwrap();
        assert_eq!(fs::read_to_string(d.join("menu.gsc")).unwrap(), "before risky edit");
    }

    #[test]
    fn snapshot_restore_roundtrip() {
        let (t, v) = vault();
        let d = t.path().join("p");
        fs::create_dir_all(d.join("a")).unwrap();
        fs::write(d.join("a/one.gsc"), "one v1").unwrap();
        fs::write(d.join("two.cfg"), "two v1").unwrap();
        fs::write(d.join("same.txt"), "unchanged").unwrap();
        let pid = cp(&v, NewProject { name: "P".into(), description: "".into(), category: "software".into(), status: "".into(), link_path: Some(d.clone()), module: Json::obj() }).unwrap().str_of("id").unwrap();
        let s1 = create(&v, &pid, "omen before animation", "known good", 100.0).unwrap();
        assert_eq!(s1.get("fileCount").unwrap().as_i64(), Some(3));
        // change, delete, add
        fs::write(d.join("a/one.gsc"), "one v2 broken").unwrap();
        fs::remove_file(d.join("two.cfg")).unwrap();
        fs::write(d.join("new.txt"), "added later").unwrap();
        let pv = preview(&v, &s1.str_of("id").unwrap()).unwrap();
        assert_eq!(pv.get("changes").unwrap().as_arr().unwrap().len(), 2);
        assert_eq!(pv.get("unchanged").unwrap().as_i64(), Some(1));
        let r = restore(&v, &s1.str_of("id").unwrap()).unwrap();
        assert_eq!(r.get("restored").unwrap().as_arr().unwrap().len(), 2);
        assert_eq!(fs::read_to_string(d.join("a/one.gsc")).unwrap(), "one v1");
        assert_eq!(fs::read_to_string(d.join("two.cfg")).unwrap(), "two v1");
        assert!(d.join("new.txt").exists(), "newer files are never deleted");
        // the automatic undo snapshot holds the broken state
        let undo = r.get("undoSnapshot").unwrap().str_of("id").unwrap();
        restore(&v, &undo).unwrap();
        assert_eq!(fs::read_to_string(d.join("a/one.gsc")).unwrap(), "one v2 broken");
        assert_eq!(list(&v, &pid).unwrap().as_arr().unwrap().len(), 3, "original + one automatic undo snapshot per restore");
    }
}

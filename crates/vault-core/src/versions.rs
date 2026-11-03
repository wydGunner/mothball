//! Stored file versions: lets you compare and roll back files without Git.
//!
//! Content goes into a content-addressed object store
//! (`<data>/objects/ab/cdef…`, named by SHA-256), so identical content is
//! stored once no matter how many files/versions share it. A version row
//! records which file had that content, where, and when.
//!
//! What gets versioned automatically after each scan (per project policy):
//!   * text/code/config files up to `textMaxMb` (default 2 MB) — on by default
//!   * the category's key binary files (3MF/STL/Blend… for 3D printing)
//!     up to `keyMaxMb` (default 150 MB) — on by default for 3D printing and
//!     electronics
//! Any file can also be saved manually. Restoring always saves the current
//! content as a version first, so a restore can itself be undone.

use crate::db::{DbResult, Row};
use crate::json::Json;
use crate::util::{new_id, now_ms, safe_join, sha256_file};
use crate::{diff, files, p, profiles, projects, timeline, Vault};
use std::path::{Path, PathBuf};

pub struct Policy {
    pub text: bool,
    pub text_max: i64,
    pub key_files: bool,
    pub key_max: i64,
}

pub fn policy(v: &Vault, project_id: &str) -> DbResult<(Policy, String)> {
    let r = v.db.one("SELECT category, version_policy FROM projects WHERE id=?", p![project_id])?.ok_or("Project not found")?;
    let cat = r.s("category");
    let j = Json::parse(&r.s("version_policy")).unwrap_or(Json::obj());
    let has_key = !profiles::key_version_exts(&cat).is_empty();
    Ok((
        Policy {
            text: j.get("text").and_then(|x| x.as_bool()).unwrap_or(true),
            text_max: (j.get("textMaxMb").and_then(|x| x.as_f64()).unwrap_or(2.0) * 1048576.0) as i64,
            key_files: j.get("keyFiles").and_then(|x| x.as_bool()).unwrap_or(has_key),
            key_max: (j.get("keyMaxMb").and_then(|x| x.as_f64()).unwrap_or(150.0) * 1048576.0) as i64,
        },
        cat,
    ))
}

pub fn policy_json(v: &Vault, project_id: &str) -> DbResult<Json> {
    let (pl, cat) = policy(v, project_id)?;
    Ok(Json::obj()
        .with("text", pl.text)
        .with("textMaxMb", pl.text_max as f64 / 1048576.0)
        .with("keyFiles", pl.key_files)
        .with("keyMaxMb", pl.key_max as f64 / 1048576.0)
        .with("keyExts", Json::Arr(profiles::key_version_exts(&cat).iter().map(|e| Json::from(*e)).collect())))
}

pub fn set_policy(v: &Vault, project_id: &str, j: &Json) -> DbResult<Json> {
    let mut o = Json::obj();
    for k in ["text", "keyFiles"] {
        if let Some(b) = j.get(k).and_then(|x| x.as_bool()) {
            o.set(k, b);
        }
    }
    for k in ["textMaxMb", "keyMaxMb"] {
        if let Some(n) = j.get(k).and_then(|x| x.as_f64()) {
            o.set(k, n.clamp(0.01, 4096.0));
        }
    }
    v.db.exec("UPDATE projects SET version_policy=? WHERE id=?", p![o.to_string(), project_id])?;
    policy_json(v, project_id)
}

/// Where backup contents (`objects/`) and portable metadata (`history/`,
/// `snapshots/`) are kept. Defaults to the vault's data folder; can be moved
/// to another drive in Settings.
pub fn storage_root(v: &Vault) -> PathBuf {
    v.db.one("SELECT value FROM meta WHERE key='storage_root'", p![])
        .ok()
        .flatten()
        .map(|r| PathBuf::from(r.s("value")))
        .unwrap_or_else(|| v.data_dir.clone())
}

/// Fails with a clear message when the backup location is unavailable.
pub fn check_storage(v: &Vault) -> DbResult<PathBuf> {
    let root = storage_root(v);
    if !root.is_dir() {
        return Err(format!(
            "The backup location {} is not available (drive disconnected or folder removed). Reconnect it, or choose another location in Settings → Backup storage. Nothing was changed.",
            root.display()
        ));
    }
    Ok(root)
}

pub(crate) fn object_path(v: &Vault, sha: &str) -> DbResult<PathBuf> {
    if sha.len() != 64 || !sha.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err("invalid object id".into());
    }
    Ok(storage_root(v).join("objects").join(&sha[..2]).join(&sha[2..]))
}

/// Copies `src` into the object store; verifies the content hash.
pub(crate) fn store(v: &Vault, src: &Path, expected_sha: &str) -> DbResult<()> {
    check_storage(v)?;
    let dest = object_path(v, expected_sha)?;
    if dest.exists() {
        return Ok(());
    }
    let dir = dest.parent().unwrap();
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let tmp = dir.join(format!(".tmp-{}", &new_id()[..8]));
    std::fs::copy(src, &tmp).map_err(|e| format!("{}: {e}", src.display()))?;
    match sha256_file(&tmp) {
        Ok(h) if h == expected_sha => {}
        _ => {
            let _ = std::fs::remove_file(&tmp);
            return Err(format!("{} changed while being saved; will retry on next scan", src.display()));
        }
    }
    std::fs::rename(&tmp, &dest).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        e.to_string()
    })
}

fn add_version(v: &Vault, f: &Row, root: &Path, reason: &str, note: &str) -> DbResult<Option<String>> {
    let Some(sha) = f.os("sha256") else { return Ok(None) };
    let src = safe_join(root, &f.s("rel_path"))?;
    store(v, &src, &sha)?;
    Ok(Some(insert_version(v, f, &sha, f.i("size"), reason, note, "", "")?))
}

/// Records a version event. Contents must already be in the object store.
/// Every event gets its own row (and metadata), even when its content is
/// identical to an earlier version — only the bytes are deduplicated.
pub(crate) fn insert_version(v: &Vault, f: &Row, sha: &str, size: i64, reason: &str, note: &str, name: &str, tags: &str) -> DbResult<String> {
    let id = new_id();
    let now = now_ms();
    v.db.exec(
        "INSERT INTO file_versions(id,project_id,file_id,sha256,size,rel_path,reason,note,name,tags,created_at) VALUES (?,?,?,?,?,?,?,?,?,?,?)",
        p![id.clone(), f.s("project_id"), f.s("id"), sha, size, f.s("rel_path"), reason, note, name, tags, now],
    )?;
    if !name.is_empty() || !note.is_empty() || matches!(reason, "manual" | "pre-save" | "post-save") {
        write_sidecar(v, &id)?;
        index_backup(v, &id)?;
    }
    Ok(id)
}

/// Portable metadata next to the backup store, so a backup's context survives
/// even if the database is lost: history/<project>/<file>/<time>_<name>_<id>.json
fn write_sidecar(v: &Vault, version_id: &str) -> DbResult<()> {
    let r = v
        .db
        .one(
            "SELECT fv.*, p.name AS project_name, p.root_path, f.name AS file_name FROM file_versions fv
             JOIN projects p ON p.id=fv.project_id JOIN files f ON f.id=fv.file_id WHERE fv.id=?",
            p![version_id],
        )?
        .ok_or("Version not found")?;
    let dir = storage_root(v)
        .join("history")
        .join(format!("{}-{}", crate::util::slugify(&r.s("project_name")), &r.s("project_id")[..8]))
        .join(format!("{}-{}", &r.s("file_id")[..8], crate::util::slugify(&r.s("file_name"))));
    let label = if r.s("name").is_empty() { r.s("reason") } else { crate::util::slugify(&r.s("name")) };
    let ts = crate::util::iso_utc(r.i("created_at")).replace([' ', ':'], "-").replace("-UTC", "Z");
    let path = dir.join(format!("{ts}_{label}_{}.json", &version_id[..8]));
    let j = Json::obj()
        .with("format", "project-vault-backup-entry")
        .with("formatVersion", 1)
        .with("id", version_id)
        .with("backupName", r.s("name"))
        .with("description", r.s("note"))
        .with("tags", Json::Arr(r.s("tags").split(',').map(str::trim).filter(|t| !t.is_empty()).map(Json::from).collect()))
        .with("reason", r.s("reason"))
        .with("createdAt", r.i("created_at"))
        .with("createdAtUtc", crate::util::iso_utc(r.i("created_at")))
        .with("project", Json::obj().with("id", r.s("project_id")).with("name", r.s("project_name")).with("root", r.s("root_path")))
        .with("file", Json::obj().with("id", r.s("file_id")).with("originalName", r.s("file_name")).with("relativePath", r.s("rel_path")))
        .with("sha256", r.s("sha256"))
        .with("size", r.i("size"))
        .with("contentPath", format!("objects/{}/{}", &r.s("sha256")[..2], &r.s("sha256")[2..]));
    crate::util::atomic_write(&path, j.pretty().as_bytes()).map_err(|e| format!("Could not write backup metadata to {}: {e}", dir.display()))
}

pub(crate) fn index_backup(v: &Vault, version_id: &str) -> DbResult<()> {
    if let Some(r) = v.db.one(
        "SELECT fv.*, f.name AS file_name FROM file_versions fv JOIN files f ON f.id=fv.file_id WHERE fv.id=?",
        p![version_id],
    )? {
        let title = if r.s("name").is_empty() { format!("{} ({})", r.s("file_name"), r.s("reason")) } else { r.s("name") };
        let body = format!("{} {} {} {} {}", r.s("note"), r.s("tags").replace(',', " "), r.s("rel_path"), r.s("file_name"), r.s("reason"));
        crate::search::index(v, "backup", version_id, &r.s("project_id"), &title, &body)?;
    }
    Ok(())
}

/// After a scan: stores versions for eligible files whose current content is
/// not stored yet. Returns (stored, errors).
pub fn capture(v: &Vault, project_id: &str) -> DbResult<(usize, Vec<String>)> {
    let (pl, cat) = policy(v, project_id)?;
    if !pl.text && !pl.key_files {
        return Ok((0, vec![]));
    }
    let root = projects::root_of(v, project_id)?;
    let key_exts = if pl.key_files { profiles::key_version_exts(&cat).join(",") } else { String::new() };
    let rows = v.db.query(
        "SELECT f.*, EXISTS(SELECT 1 FROM file_versions x WHERE x.file_id=f.id) AS has_any FROM files f
         WHERE f.project_id=? AND f.state='present' AND f.sha256 IS NOT NULL
           AND ((? AND f.kind IN ('code','config') AND f.size<=?)
             OR (? AND f.ext IN ('md','markdown','txt','csv','gcode','scad','meta','asmdef') AND f.size<=?)
             OR (instr(',' || ? || ',', ',' || f.ext || ',')>0 AND f.ext!='' AND f.size<=?))
           AND NOT EXISTS (SELECT 1 FROM file_versions x WHERE x.file_id=f.id AND x.sha256=f.sha256)",
        p![project_id, pl.text, pl.text_max, pl.text, pl.text_max, key_exts, pl.key_max],
    )?;
    let mut rows = rows;
    if pl.text {
        // Files of unknown type that turn out to be text (a mod's own formats,
        // extensionless notes) are versioned like text, so outside edits diff.
        let unknown = v.db.query(
            "SELECT f.*, EXISTS(SELECT 1 FROM file_versions x WHERE x.file_id=f.id) AS has_any, s.is_text AS sniffed FROM files f
             LEFT JOIN text_sniff s ON s.sha256=f.sha256
             WHERE f.project_id=? AND f.state='present' AND f.sha256 IS NOT NULL AND f.kind='other' AND f.size>0 AND f.size<=?
               AND (s.is_text IS NULL OR s.is_text=1)
               AND NOT EXISTS (SELECT 1 FROM file_versions x WHERE x.file_id=f.id AND x.sha256=f.sha256)",
            p![project_id, pl.text_max],
        )?;
        for f in unknown {
            let text = match f.oi("sniffed") {
                Some(t) => t == 1,
                None => {
                    let t = safe_join(&root, &f.s("rel_path")).map(|ap| files::sniff_text(&ap)).unwrap_or(false);
                    v.db.exec("INSERT OR REPLACE INTO text_sniff(sha256,is_text) VALUES (?,?)", p![f.s("sha256"), t as i64])?;
                    t
                }
            };
            if text {
                rows.push(f);
            }
        }
    }
    let mut n = 0;
    let mut errs = Vec::new();
    for f in &rows {
        let reason = if f.b("has_any") { "auto" } else { "baseline" };
        match add_version(v, f, &root, reason, "") {
            Ok(Some(_)) => n += 1,
            Ok(None) => {}
            Err(e) => errs.push(e),
        }
    }
    Ok((n, errs))
}

/// Makes sure the file's current on-disk content is stored as a version
/// (used before the built-in editor overwrites a file).
pub(crate) fn ensure_stored(v: &Vault, file: &Row, path: &Path, sha: &str, reason: &str) -> DbResult<()> {
    let exists = v.db.scalar_i64("SELECT count(*) FROM file_versions WHERE file_id=? AND sha256=?", p![file.s("id"), sha])?;
    if exists > 0 {
        return Ok(());
    }
    store(v, path, sha)?;
    let size = std::fs::metadata(path).map(|m| m.len() as i64).unwrap_or(0);
    insert_version(v, file, sha, size, reason, "", "", "")?;
    Ok(())
}

pub fn save_manual(v: &Vault, file_id: &str, note: &str) -> DbResult<Json> {
    backup_now(v, file_id, "", note, "")
}

/// Backs up the file as it is on disk right now, with optional name/description/tags.
pub fn backup_now(v: &Vault, file_id: &str, name: &str, note: &str, tags: &str) -> DbResult<Json> {
    let f = v.db.one("SELECT * FROM files WHERE id=?", p![file_id])?.ok_or("File not found")?;
    if f.s("state") != "present" {
        return Err("The file is missing on disk".into());
    }
    let root = projects::root_of(v, &f.s("project_id"))?;
    // re-hash: the index may be a scan behind
    let path = safe_join(&root, &f.s("rel_path"))?;
    let sha = sha256_file(&path).map_err(|e| e.to_string())?;
    if Some(&sha) != f.os("sha256").as_ref() {
        files::scan(v, &f.s("project_id"))?;
    }
    let f = v.db.one("SELECT * FROM files WHERE id=?", p![file_id])?.ok_or("File not found")?;
    let name = if name.trim().is_empty() { String::new() } else { unique_backup_name(v, file_id, name.trim())? };
    let Some(sha) = f.os("sha256") else { return Err("The file could not be read".into()) };
    store(v, &safe_join(&root, &f.s("rel_path"))?, &sha)?;
    insert_version(v, &f, &sha, f.i("size"), "manual", note.trim(), &name, &norm_tags(tags))?;
    let label = if name.is_empty() { f.s("rel_path") } else { name.clone() };
    timeline::add(v, &f.s("project_id"), "version", &format!("Backed up {label}"), note.trim(), Some(("file", file_id)), false)?;
    list(v, file_id)
}

pub fn list(v: &Vault, file_id: &str) -> DbResult<Json> {
    let cur = v.db.one("SELECT sha256 FROM files WHERE id=?", p![file_id])?.and_then(|r| r.os("sha256"));
    let rows = v.db.query("SELECT * FROM file_versions WHERE file_id=? ORDER BY created_at DESC", p![file_id])?;
    Ok(Json::Arr(
        rows.iter()
            .map(|r| {
                Json::obj()
                    .with("id", r.s("id"))
                    .with("sha256", r.s("sha256"))
                    .with("size", r.i("size"))
                    .with("path", r.s("rel_path"))
                    .with("reason", r.s("reason"))
                    .with("note", r.s("note"))
                    .with("name", r.s("name"))
                    .with("tags", Json::Arr(r.s("tags").split(',').map(str::trim).filter(|t| !t.is_empty()).map(Json::from).collect()))
                    .with("at", r.i("created_at"))
                    .with("isCurrent", Some(r.s("sha256")) == cur)
            })
            .collect(),
    ))
}

/// A side of a comparison: a stored version id, or "current" (the file on disk).
fn side(v: &Vault, file_id: &str, which: &str) -> DbResult<(PathBuf, String, i64, String)> {
    if which == "current" {
        let f = v.db.one("SELECT * FROM files WHERE id=?", p![file_id])?.ok_or("File not found")?;
        if f.s("state") != "present" {
            return Err("The file is missing on disk".into());
        }
        let root = projects::root_of(v, &f.s("project_id"))?;
        Ok((safe_join(&root, &f.s("rel_path"))?, f.os("sha256").unwrap_or_default(), f.i("size"), "Current file".into()))
    } else if let Some(sha) = which.strip_prefix("sha:") {
        // Content by hash: from the version store, or the file itself if it still has that content.
        if sha.len() != 64 || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("Bad content hash".into());
        }
        let obj = object_path(v, sha)?;
        if let Ok(m) = std::fs::metadata(&obj) {
            return Ok((obj, sha.to_string(), m.len() as i64, "Stored copy".into()));
        }
        let f = v.db.one("SELECT * FROM files WHERE id=?", p![file_id])?.ok_or("File not found")?;
        if f.s("state") == "present" && f.os("sha256").as_deref() == Some(sha) {
            let root = projects::root_of(v, &f.s("project_id"))?;
            return Ok((safe_join(&root, &f.s("rel_path"))?, sha.to_string(), f.i("size"), "Current file".into()));
        }
        Err("NOT_KEPT:That version of the file wasn't kept, so the change can't be shown. Your version settings decide which files get copies.".into())
    } else {
        let r = v.db.one("SELECT * FROM file_versions WHERE id=? AND file_id=?", p![which, file_id])?.ok_or("Version not found")?;
        let label = if r.s("name").is_empty() { format!("Version from {}", crate::util::iso_utc(r.i("created_at"))) } else { format!("{} ({})", r.s("name"), crate::util::iso_utc(r.i("created_at"))) };
        Ok((object_path(v, &r.s("sha256"))?, r.s("sha256"), r.i("size"), label))
    }
}

fn read_text(p: &Path) -> Option<String> {
    let b = std::fs::read(p).ok()?;
    if b.iter().take(8000).any(|c| *c == 0) {
        return None;
    }
    Some(String::from_utf8_lossy(&b).to_string())
}

pub fn compare(v: &Vault, file_id: &str, old: &str, new: &str) -> DbResult<Json> {
    let (pa, sa, za, la) = side(v, file_id, old)?;
    let (pb, sb, zb, lb) = side(v, file_id, new)?;
    let ext = v.db.one("SELECT ext FROM files WHERE id=?", p![file_id])?.map(|r| r.s("ext")).unwrap_or_default();
    let mut out = Json::obj()
        .with("old", Json::obj().with("label", la).with("sha256", sa.clone()).with("size", za))
        .with("new", Json::obj().with("label", lb).with("sha256", sb.clone()).with("size", zb));
    let text_like = files::is_text_ext(&ext) || matches!(files::classify(&ext), "code" | "config" | "document") && ext != "pdf"
        || (files::classify(&ext) == "other" && files::sniff_text(&pa) && files::sniff_text(&pb));
    let max = 8 * 1024 * 1024;
    if text_like && za <= max && zb <= max {
        if let (Some(ta), Some(tb)) = (read_text(&pa), read_text(&pb)) {
            out.set("kind", "text");
            out.set("diff", diff::diff_json(&ta, &tb, 3));
            out.set("ext", ext.clone());
            out.set("oldText", ta);
            out.set("newText", tb);
            return Ok(out);
        }
    }
    out.set("kind", "binary");
    out.set("identical", sa == sb);
    if ext == "stl" {
        let dim = |p: &Path| files::stl_dimensions(p).map(|(x, y, z)| Json::Arr(vec![x.into(), y.into(), z.into()])).unwrap_or(Json::Null);
        out.set("oldDims", dim(&pa));
        out.set("newDims", dim(&pb));
    }
    Ok(out)
}

/// Writes a stored version back to the file's location. The current content
/// is saved as a "before-restore" version first.
pub fn restore(v: &Vault, version_id: &str) -> DbResult<Json> {
    restore_opts(v, version_id, true)
}

/// Restore with an explicit choice about preserving the current content.
/// Steps: verify the stored copy, optionally back up the current file, write
/// to a temp file in the same folder, verify, atomically replace, re-verify.
pub fn restore_opts(v: &Vault, version_id: &str, keep_current: bool) -> DbResult<Json> {
    let ver = v.db.one("SELECT * FROM file_versions WHERE id=?", p![version_id])?.ok_or("Version not found")?;
    let fid = ver.s("file_id");
    let f = v.db.one("SELECT * FROM files WHERE id=?", p![fid.clone()])?.ok_or("File not found")?;
    let pid = f.s("project_id");
    let root = projects::root_of(v, &pid)?;
    if !root.is_dir() {
        return Err("Project folder is not reachable".into());
    }
    let obj = object_path(v, &ver.s("sha256"))?;
    if sha256_file(&obj).ok().as_deref() != Some(ver.s("sha256").as_str()) {
        return Err("The stored version is damaged or missing (checksum mismatch). Nothing was changed.".into());
    }
    let target = safe_join(&root, &f.s("rel_path"))?;
    if target.exists() {
        let cur = sha256_file(&target).map_err(|e| format!("Cannot read current file: {e}"))?;
        if cur == ver.s("sha256") {
            return Ok(Json::obj().with("ok", true).with("unchanged", true));
        }
        if keep_current {
            store(v, &target, &cur)?;
            let size = std::fs::metadata(&target).map(|m| m.len() as i64).unwrap_or(0);
            let what = if ver.s("name").is_empty() { crate::util::iso_utc(ver.i("created_at")) } else { ver.s("name") };
            insert_version(v, &f, &cur, size, "before-restore", &format!("Automatically kept before restoring {what}"), "", "")?;
        }
    }
    let dir = target.parent().unwrap();
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let tmp = dir.join(format!(".pv-tmp-{}", &new_id()[..8]));
    std::fs::copy(&obj, &tmp).map_err(|e| e.to_string())?;
    if sha256_file(&tmp).ok().as_deref() != Some(ver.s("sha256").as_str()) {
        let _ = std::fs::remove_file(&tmp);
        return Err("Copy verification failed. Nothing was changed.".into());
    }
    std::fs::rename(&tmp, &target).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("Could not replace the file (is it open in another program?): {e}")
    })?;
    if sha256_file(&target).ok().as_deref() != Some(ver.s("sha256").as_str()) {
        return Err("The restored file does not match the backup after writing (another program may have changed it). Check the file and its History.".into());
    }
    let what = if ver.s("name").is_empty() { format!("the version from {}", crate::util::iso_utc(ver.i("created_at"))) } else { format!("backup “{}”", ver.s("name")) };
    v.db.exec(
        "INSERT INTO file_events(id,project_id,file_id,kind,detail,sha256,at) VALUES (?,?,?,'restored-version',?,?,?)",
        p![new_id(), pid.clone(), fid.clone(), format!("Restored {what}"), ver.s("sha256"), now_ms()],
    )?;
    timeline::add(v, &pid, "version", &format!("Restored {} to {what}", f.s("rel_path")), "", Some(("file", &fid)), false)?;
    files::scan(v, &pid)?;
    Ok(Json::obj().with("ok", true).with("unchanged", false))
}

pub fn delete_version(v: &Vault, version_id: &str) -> DbResult<()> {
    v.db.exec("DELETE FROM file_versions WHERE id=?", p![version_id])?;
    crate::search::unindex(v, "backup", version_id)?;
    gc(v)?;
    Ok(())
}

/// Keeps the newest `keep` automatic versions per file. Named/described
/// backups (manual, pre-save, post-save) are never pruned unless
/// `include_named` is set explicitly by the user.
pub fn prune(v: &Vault, project_id: &str, keep: i64) -> DbResult<Json> {
    prune_opts(v, project_id, keep, false)
}

pub fn prune_opts(v: &Vault, project_id: &str, keep: i64, include_named: bool) -> DbResult<Json> {
    prune_impl(v, project_id, keep, include_named, false)
}

/// With `dry_run`, only counts what would be deleted (for the confirmation).
pub fn prune_impl(v: &Vault, project_id: &str, keep: i64, include_named: bool, dry_run: bool) -> DbResult<Json> {
    let keep = keep.max(1);
    // automatic versions only; anything you named or saved on purpose is kept
    let cond = if include_named { "1" } else { "name='' AND reason IN ('auto','baseline','before-edit','before-restore','before-delete')" };
    let doomed: Vec<String> = v
        .db
        .query(
            &format!(
                // never the copy of a file that's deleted or missing: it may be the only one left
                "SELECT id FROM (SELECT id, ROW_NUMBER() OVER (PARTITION BY file_id ORDER BY created_at DESC) AS rn
                 FROM file_versions fv WHERE project_id=? AND {cond}
                   AND NOT EXISTS (SELECT 1 FROM files f WHERE f.id=fv.file_id AND f.state IN ('deleted','missing') AND f.sha256=fv.sha256)) WHERE rn>?"
            ),
            p![project_id, keep],
        )?
        .iter()
        .map(|r| r.s("id"))
        .collect();
    if dry_run {
        return Ok(Json::obj().with("wouldDelete", doomed.len() as i64));
    }
    let mut n = 0;
    for id in &doomed {
        n += v.db.exec("DELETE FROM file_versions WHERE id=?", p![id.clone()])?;
        crate::search::unindex(v, "backup", id)?;
    }
    let freed = gc(v)?;
    Ok(Json::obj().with("deletedVersions", n).with("freedBytes", freed))
}

/// Removes objects no version refers to. Returns bytes freed.
pub fn gc(v: &Vault) -> DbResult<i64> {
    let dir = storage_root(v).join("objects");
    let Ok(rd) = std::fs::read_dir(&dir) else { return Ok(0) };
    let referenced: std::collections::HashSet<String> = v
        .db
        .query("SELECT DISTINCT sha256 FROM file_versions UNION SELECT DISTINCT sha256 FROM snapshot_files", p![])?
        .iter()
        .map(|r| r.s("sha256"))
        .collect();
    let mut freed = 0i64;
    for sub in rd.flatten() {
        let Ok(inner) = std::fs::read_dir(sub.path()) else { continue };
        let prefix = sub.file_name().to_string_lossy().to_string();
        for f in inner.flatten() {
            let name = f.file_name().to_string_lossy().to_string();
            if name.starts_with(".tmp-") || !referenced.contains(&format!("{prefix}{name}")) {
                freed += f.metadata().map(|m| m.len() as i64).unwrap_or(0);
                let _ = std::fs::remove_file(f.path());
            }
        }
    }
    Ok(freed)
}

pub fn norm_tags(t: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    for x in t.split(',').map(|x| x.trim()).filter(|x| !x.is_empty()) {
        if !out.iter().any(|o| o.eq_ignore_ascii_case(x)) {
            out.push(x.to_string());
        }
    }
    out.join(", ")
}

const STOP: &[&str] = &[
    "a", "an", "the", "and", "or", "but", "to", "of", "in", "on", "for", "with", "it", "is", "was", "this", "that", "i", "my", "we", "be", "so",
    "now", "just", "some", "fixed", "changed", "updated", "made", "also", "at", "from", "by", "as", "should", "test", "next", "then",
];

/// Offline backup-name suggestion: <stem>_<keywords from the description>.<ext>.
/// With no usable description (or task) it falls back to a neutral
/// <stem>_v<N>_<date>.<ext> — it never pretends to understand the code change.
pub fn suggest_name(v: &Vault, file_id: &str, description: &str, task_title: &str) -> DbResult<Json> {
    let f = v.db.one("SELECT name FROM files WHERE id=?", p![file_id])?.ok_or("File not found")?;
    let name = f.s("name");
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (name[..i].to_string(), name[i..].to_string()),
        _ => (name.clone(), String::new()),
    };
    let words = |s: &str| -> Vec<String> {
        s.split(|c: char| !c.is_alphanumeric())
            .map(|w| w.to_lowercase())
            .filter(|w| w.len() > 1 && !STOP.contains(&w.as_str()) && !stem.to_lowercase().contains(w.as_str()))
            .collect()
    };
    let mut kw = words(description);
    if kw.is_empty() {
        kw = words(task_title);
    }
    kw.dedup();
    let source;
    let suggestion = if kw.is_empty() {
        source = "neutral";
        let n = v.db.scalar_i64("SELECT count(*) FROM file_versions WHERE file_id=? AND name!=''", p![file_id])? + 1;
        let date = crate::util::iso_utc(crate::util::now_ms())[..10].to_string();
        format!("{stem}_v{n}_{date}{ext}")
    } else {
        source = if words(description).is_empty() { "task" } else { "description" };
        format!("{stem}_{}{ext}", kw.into_iter().take(4).collect::<Vec<_>>().join("_"))
    };
    Ok(Json::obj().with("name", unique_backup_name(v, file_id, &suggestion)?).with("source", source))
}

/// Makes a backup name unique among this file's backups: name, name_2, name_3…
pub fn unique_backup_name(v: &Vault, file_id: &str, name: &str) -> DbResult<String> {
    let clean: String = name.trim().chars().filter(|c| !matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') && !c.is_control()).collect();
    let clean = if clean.is_empty() { "backup".to_string() } else { clean };
    let taken = |n: &str| v.db.scalar_i64("SELECT count(*) FROM file_versions WHERE file_id=? AND lower(name)=lower(?)", p![file_id, n]);
    if taken(&clean)? == 0 {
        return Ok(clean);
    }
    let (stem, ext) = match clean.rfind('.') {
        Some(i) if i > 0 => (&clean[..i], &clean[i..]),
        _ => (clean.as_str(), ""),
    };
    for i in 2.. {
        let c = format!("{stem}_{i}{ext}");
        if taken(&c)? == 0 {
            return Ok(c);
        }
    }
    unreachable!()
}

pub struct SmartSave<'a> {
    pub backup_before: bool,
    pub backup_after: bool,
    pub name: &'a str,
    pub description: &'a str,
    pub tags: &'a str,
}

/// Editor save with optional named backups:
///  * backup_before — preserves the content that is on disk *before* this save
///  * backup_after  — keeps a named copy of the *new* content being saved
/// Both refuse to proceed if the file changed on disk since it was opened.
pub fn smart_save(v: &Vault, file_id: &str, text: &str, expected_sha: &str, eol: &str, bom: bool, o: &SmartSave) -> DbResult<Json> {
    let f = v.db.one("SELECT * FROM files WHERE id=?", p![file_id])?.ok_or("File not found")?;
    let root = projects::root_of(v, &f.s("project_id"))?;
    let path = safe_join(&root, &f.s("rel_path"))?;
    if o.backup_before || o.backup_after {
        check_storage(v)?;
    }
    let name = if o.name.trim().is_empty() { String::new() } else { o.name.trim().to_string() };
    let tags = norm_tags(o.tags);
    let mut before_id = None;
    let mut before_name = String::new();
    let mut after_name = String::new();
    if o.backup_before {
        let cur = sha256_file(&path).map_err(|e| format!("Cannot read the file before saving: {e}"))?;
        if cur != expected_sha {
            return Err(format!("CONFLICT:{cur}:The file was changed on disk after you opened it."));
        }
        store(v, &path, &cur)?;
        let bname = if name.is_empty() { String::new() } else {
            let (stem, ext) = match name.rfind('.') { Some(i) if i > 0 => (&name[..i], &name[i..]), _ => (name.as_str(), "") };
            unique_backup_name(v, file_id, &format!("{stem}_before{ext}"))?
        };
        let note = if o.description.trim().is_empty() { String::new() } else { format!("State before: {}", o.description.trim()) };
        let size = std::fs::metadata(&path).map(|m| m.len() as i64).unwrap_or(0);
        before_id = Some(insert_version(v, &f, &cur, size, "pre-save", &note, &bname, &tags)?);
        before_name = bname;
    }
    let w = files::write_text(v, file_id, text, expected_sha, eol, bom)?;
    let new_sha = w.str_of("sha256").unwrap_or_default();
    let mut after_id = None;
    if o.backup_after {
        let f2 = v.db.one("SELECT * FROM files WHERE id=?", p![file_id])?.ok_or("File not found")?;
        let path2 = safe_join(&root, &f2.s("rel_path"))?;
        store(v, &path2, &new_sha)?;
        let bname = if name.is_empty() { String::new() } else { unique_backup_name(v, file_id, &name)? };
        let size = std::fs::metadata(&path2).map(|m| m.len() as i64).unwrap_or(0);
        after_id = Some(insert_version(v, &f2, &new_sha, size, "post-save", o.description.trim(), &bname, &tags)?);
        after_name = bname;
    }
    if o.backup_before || o.backup_after {
        let mut kept = Vec::new();
        if o.backup_before {
            kept.push(if before_name.is_empty() { "the previous version".to_string() } else { format!("“{before_name}” (before)") });
        }
        if o.backup_after {
            kept.push(if after_name.is_empty() { "the saved version".to_string() } else { format!("“{after_name}” (saved)") });
        }
        timeline::add(v, &f.s("project_id"), "version", &format!("Saved {} — backup kept: {}", f.s("name"), kept.join(", ")), o.description.trim(), Some(("file", file_id)), false)?;
    }
    Ok(Json::obj()
        .with("sha256", new_sha)
        .with("beforeBackupId", before_id)
        .with("afterBackupId", after_id)
        .with("beforeName", before_name)
        .with("afterName", after_name))
}

pub fn update_backup(v: &Vault, version_id: &str, name: &str, note: &str, tags: &str) -> DbResult<()> {
    let r = v.db.one("SELECT file_id, name FROM file_versions WHERE id=?", p![version_id])?.ok_or("Backup not found")?;
    let name = if name.trim().is_empty() || name.trim() == r.s("name") { name.trim().to_string() } else { unique_backup_name(v, &r.s("file_id"), name)? };
    v.db.exec("UPDATE file_versions SET name=?, note=?, tags=? WHERE id=?", p![name, note.trim(), norm_tags(tags), version_id])?;
    write_sidecar(v, version_id)?;
    index_backup(v, version_id)
}

/// Writes a stored version out as a normal, readable file (e.g.
/// weapon_tuner_instashoot_fix.gsc) into `dest_dir`. Never overwrites.
pub fn export_copy(v: &Vault, version_id: &str, dest_dir: &Path) -> DbResult<Json> {
    let r = v.db.one("SELECT fv.*, f.name AS file_name FROM file_versions fv JOIN files f ON f.id=fv.file_id WHERE fv.id=?", p![version_id])?.ok_or("Backup not found")?;
    if !dest_dir.is_dir() {
        return Err(format!("{} is not an accessible folder", dest_dir.display()));
    }
    let obj = object_path(v, &r.s("sha256"))?;
    if sha256_file(&obj).ok().as_deref() != Some(r.s("sha256").as_str()) {
        return Err("The stored copy is damaged or missing".into());
    }
    let fname = if r.s("name").is_empty() {
        let n = r.s("file_name");
        let ts = crate::util::iso_utc(r.i("created_at"))[..10].to_string();
        match n.rfind('.') { Some(i) if i > 0 => format!("{}_{ts}{}", &n[..i], &n[i..]), _ => format!("{n}_{ts}") }
    } else {
        r.s("name")
    };
    let dest = crate::util::unique_dest(dest_dir, &fname);
    std::fs::copy(&obj, &dest).map_err(|e| e.to_string())?;
    Ok(Json::obj().with("path", projects::display_path(&dest)))
}

/// All version/backup records for a project (or across projects), newest first,
/// filtered by free text (name, description, tags, file name/path) and dates.
pub fn browse(v: &Vault, project_id: Option<&str>, query: &str, named_only: bool, from: Option<i64>, to: Option<i64>, limit: i64) -> DbResult<Json> {
    let like = format!("%{}%", query.trim().to_lowercase().replace('%', "\\%").replace('_', "\\_"));
    let rows = v.db.query(
        "SELECT fv.*, f.name AS file_name, f.rel_path AS current_path, f.state AS file_state, p.name AS project_name, f.sha256 AS current_sha
         FROM file_versions fv JOIN files f ON f.id=fv.file_id JOIN projects p ON p.id=fv.project_id
         WHERE (? IS NULL OR fv.project_id=?)
           AND (?=0 OR fv.name!='' OR fv.note!='' OR fv.reason IN ('manual','pre-save','post-save'))
           AND (?='%%' OR lower(fv.name || ' ' || fv.note || ' ' || fv.tags || ' ' || f.name || ' ' || fv.rel_path || ' ' || p.name) LIKE ? ESCAPE '\\')
           AND (? IS NULL OR fv.created_at>=?) AND (? IS NULL OR fv.created_at<=?)
         ORDER BY fv.created_at DESC LIMIT ?",
        p![project_id, project_id, named_only, like.clone(), like, from, from, to, to, limit.clamp(1, 2000)],
    )?;
    Ok(Json::Arr(
        rows.iter()
            .map(|r| {
                Json::obj()
                    .with("id", r.s("id"))
                    .with("fileId", r.s("file_id"))
                    .with("fileName", r.s("file_name"))
                    .with("path", r.s("rel_path"))
                    .with("currentPath", r.s("current_path"))
                    .with("fileState", r.s("file_state"))
                    .with("projectId", r.s("project_id"))
                    .with("projectName", r.s("project_name"))
                    .with("name", r.s("name"))
                    .with("note", r.s("note"))
                    .with("tags", Json::Arr(r.s("tags").split(',').map(str::trim).filter(|t| !t.is_empty()).map(Json::from).collect()))
                    .with("reason", r.s("reason"))
                    .with("size", r.i("size"))
                    .with("sha256", r.s("sha256"))
                    .with("at", r.i("created_at"))
                    .with("isCurrent", r.os("current_sha") == Some(r.s("sha256")))
            })
            .collect(),
    ))
}

/// One timeline per file, every entry labelled with its source. Only entries
/// backed by stored contents are marked restorable.
pub fn history(v: &Vault, file_id: &str) -> DbResult<Json> {
    let mut out: Vec<Json> = Vec::new();
    // events() works out before/after for each content change, so the
    // timeline can show what an outside edit changed.
    if let Json::Arr(evs) = files::events(v, file_id, i64::MAX)? {
        for e in evs {
            let kind = e.str_of("kind").unwrap_or_default();
            let source = match kind.as_str() {
                "renamed" | "deleted" | "restored-version" => "project-vault",
                _ => "filesystem",
            };
            let mut j = Json::obj().with("source", source).with("kind", kind.clone()).with("title", e.str_of("detail").unwrap_or_default())
                .with("at", e.get("at").and_then(|x| x.as_i64()).unwrap_or(0)).with("sha256", e.str_of("sha256")).with("restorable", false);
            if let (Some(a), Some(b)) = (e.str_of("from"), e.str_of("to")) {
                j.set("from", a);
                j.set("to", b);
            }
            out.push(j);
        }
    }
    for r in v.db.query("SELECT * FROM file_versions WHERE file_id=?", p![file_id])? {
        let named = !r.s("name").is_empty() || !r.s("note").is_empty() || matches!(r.s("reason").as_str(), "manual" | "pre-save" | "post-save");
        out.push(
            Json::obj()
                .with("source", if named { "backup" } else { "auto-version" })
                .with("kind", r.s("reason"))
                .with("title", if r.s("name").is_empty() { r.s("reason") } else { r.s("name") })
                .with("description", r.s("note"))
                .with("versionId", r.s("id"))
                .with("at", r.i("created_at"))
                .with("sha256", r.s("sha256"))
                .with("restorable", true),
        );
    }
    for r in v.db.query("SELECT * FROM file_doc_revisions WHERE file_id=?", p![file_id])? {
        out.push(Json::obj().with("source", "notes").with("kind", "doc-revision").with("title", "Notes edited (previous text kept)").with("at", r.i("created_at")).with("restorable", false));
    }
    out.sort_by_key(|j| -j.get("at").and_then(|x| x.as_i64()).unwrap_or(0));
    Ok(Json::Arr(out))
}

/// Moves the backup store (objects + metadata) to another folder: copies and
/// verifies everything first, switches over, and leaves the old copy in place.
pub fn set_storage_root(v: &Vault, new_root: &Path) -> DbResult<Json> {
    let old = storage_root(v);
    std::fs::create_dir_all(new_root).map_err(|e| format!("Cannot use {}: {e}", new_root.display()))?;
    let probe = new_root.join(format!(".pv-write-test-{}", &new_id()[..8]));
    std::fs::write(&probe, b"ok").map_err(|e| format!("{} is not writable: {e}", new_root.display()))?;
    let _ = std::fs::remove_file(&probe);
    let new_canon = projects::dunce(new_root);
    if projects::dunce(&old) == new_canon {
        return Ok(Json::obj().with("root", projects::display_path(&new_canon)).with("copied", 0));
    }
    let mut copied = 0;
    for sub in ["objects", "history", "snapshots"] {
        let src = old.join(sub);
        if !src.is_dir() {
            continue;
        }
        let w = files::walk(&src, &files::Ignore::parse(""));
        for (rel, _) in &w.files {
            let from = safe_join(&src, rel)?;
            let to = safe_join(&new_canon.join(sub), rel)?;
            if to.exists() {
                continue;
            }
            std::fs::create_dir_all(to.parent().unwrap()).map_err(|e| e.to_string())?;
            std::fs::copy(&from, &to).map_err(|e| format!("Copy failed for {}: {e}", from.display()))?;
            if sub == "objects" {
                let expect = format!("{}{}", rel.split('/').next().unwrap_or(""), rel.split('/').nth(1).unwrap_or(""));
                if sha256_file(&to).ok() != Some(expect.clone()) {
                    return Err(format!("Verification failed for object {expect}; the location was not changed."));
                }
            }
            copied += 1;
        }
    }
    v.db.exec("INSERT INTO meta(key,value) VALUES ('storage_root',?) ON CONFLICT(key) DO UPDATE SET value=excluded.value", p![new_canon.to_string_lossy().to_string()])?;
    Ok(Json::obj().with("root", projects::display_path(&new_canon)).with("copied", copied).with("oldRoot", projects::display_path(&old)))
}

pub fn stats(v: &Vault, project_id: Option<&str>) -> DbResult<Json> {
    let r = v.db.one(
        "SELECT count(*) AS n, count(DISTINCT file_id) AS files,
                COALESCE((SELECT SUM(size) FROM (SELECT DISTINCT sha256, size FROM file_versions WHERE ? IS NULL OR project_id=?)),0) AS bytes
         FROM file_versions WHERE ? IS NULL OR project_id=?",
        p![project_id, project_id, project_id, project_id],
    )?
    .unwrap();
    Ok(Json::obj().with("versions", r.i("n")).with("files", r.i("files")).with("bytes", r.i("bytes")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projects::{create, NewProject};
    use crate::testutil::*;
    use std::fs;

    #[test]
    fn versions_compare_restore() {
        let (t, v) = vault();
        let d = t.path().join("p");
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join("main.gsc"), "init()\n{\n  a();\n}\n").unwrap();
        fs::write(d.join("model.stl"), "solid x\nvertex 0 0 0\nvertex 1 1 1\nendsolid").unwrap();
        fs::write(d.join("photo.png"), [0u8, 1, 2, 3]).unwrap();
        let pid = create(&v, NewProject { name: "P".into(), description: "".into(), category: "3d-printing".into(), status: "".into(), link_path: Some(d.clone()), module: Json::obj() })
            .unwrap()
            .str_of("id")
            .unwrap();
        let r = files::scan(&v, &pid).unwrap();
        assert_eq!((r.versioned, r.errors.len()), (2, 0), "gsc (text) + stl (key file); png not versioned");
        let fid = v.db.one("SELECT id FROM files WHERE rel_path='main.gsc'", p![]).unwrap().unwrap().s("id");
        // edit -> new auto version after scan
        fs::write(d.join("main.gsc"), "init()\n{\n  a();\n  b();\n}\n").unwrap();
        let f = fs::File::options().append(true).open(d.join("main.gsc")).unwrap();
        f.set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(5)).unwrap();
        assert_eq!(files::scan(&v, &pid).unwrap().versioned, 1);
        assert_eq!(capture(&v, &pid).unwrap().0, 0, "idempotent");
        let vers = list(&v, &fid).unwrap();
        let vs = vers.as_arr().unwrap();
        assert_eq!(vs.len(), 2);
        let old = vs[1].str_of("id").unwrap();
        let c = compare(&v, &fid, &old, "current").unwrap();
        assert_eq!(c.str_of("kind").unwrap(), "text");
        assert_eq!(c.get("diff").unwrap().get("added").unwrap().as_i64(), Some(1));
        // restore old: current saved first, file content reverted
        restore(&v, &old).unwrap();
        assert_eq!(fs::read_to_string(d.join("main.gsc")).unwrap(), "init()\n{\n  a();\n}\n");
        assert!(list(&v, &fid).unwrap().to_string().contains("before-restore"));
        // prune keeps newest 1
        let r = prune(&v, &pid, 1).unwrap();
        assert!(r.get("deletedVersions").unwrap().as_i64().unwrap() >= 1);
        assert_eq!(list(&v, &fid).unwrap().as_arr().unwrap().len(), 1);
        // stl binary compare reports dims
        let sid = v.db.one("SELECT id FROM files WHERE rel_path='model.stl'", p![]).unwrap().unwrap().s("id");
        let sv = list(&v, &sid).unwrap().as_arr().unwrap()[0].str_of("id").unwrap();
        let c = compare(&v, &sid, &sv, "current").unwrap();
        assert_eq!(c.str_of("kind").unwrap(), "binary");
        assert_eq!(c.get("oldDims").unwrap().to_string(), "[1,1,1]");
        // damaged object refuses restore
        let sha = list(&v, &fid).unwrap().as_arr().unwrap()[0].str_of("sha256").unwrap();
        fs::write(object_path(&v, &sha).unwrap(), "corrupt").unwrap();
        let vid = list(&v, &fid).unwrap().as_arr().unwrap()[0].str_of("id").unwrap();
        // current content equals that version, so write a change first
        fs::write(d.join("main.gsc"), "changed").unwrap();
        assert!(restore(&v, &vid).is_err());
        assert_eq!(fs::read_to_string(d.join("main.gsc")).unwrap(), "changed");
    }

    fn setup(name: &str, content: &str) -> (TempDir, Vault, std::path::PathBuf, String, String) {
        let (t, v) = vault();
        let d = t.path().join("omen");
        fs::create_dir_all(d.join("scripts")).unwrap();
        fs::write(d.join("scripts").join(name), content).unwrap();
        let pid = create(&v, NewProject { name: "OMEN v2".into(), description: "".into(), category: "software".into(), status: "".into(), link_path: Some(d.clone()), module: Json::obj() }).unwrap().str_of("id").unwrap();
        files::scan(&v, &pid).unwrap();
        let fid = v.db.one("SELECT id FROM files WHERE name=?", p![name]).unwrap().unwrap().s("id");
        (t, v, d, pid, fid)
    }
    fn sha_of(p: &Path) -> String { sha256_file(p).unwrap() }

    #[test]
    fn smart_save_backups_names_search_restore() {
        let (t, v, d, _pid, fid) = setup("weapon_tuner.gsc", "v1 stable");
        let path = d.join("scripts/weapon_tuner.gsc");
        // offline name suggestion: from the description; neutral fallback when nothing usable
        let sug = suggest_name(&v, &fid, "Fixed the instashoot timing on dedicated servers", "").unwrap();
        assert_eq!(sug.str_of("name").unwrap(), "weapon_tuner_instashoot_timing_dedicated_servers.gsc");
        let neutral = suggest_name(&v, &fid, "  ", "").unwrap();
        assert!(neutral.str_of("name").unwrap().starts_with("weapon_tuner_v1_"));
        assert_eq!(neutral.str_of("source").unwrap(), "neutral");

        // save with backup before AND a named copy after
        let o = SmartSave { backup_before: true, backup_after: true, name: "weapon_tuner_instashoot_fix.gsc", description: "Instashoot fix", tags: "instashoot, working" };
        let r = smart_save(&v, &fid, "v2 instashoot fixed", &sha_of(&path), "lf", false, &o).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "v2 instashoot fixed");
        let before = r.str_of("beforeBackupId").unwrap();
        let after = r.str_of("afterBackupId").unwrap();
        let bv = v.db.one("SELECT * FROM file_versions WHERE id=?", p![before.clone()]).unwrap().unwrap();
        let av = v.db.one("SELECT * FROM file_versions WHERE id=?", p![after.clone()]).unwrap().unwrap();
        assert_eq!(fs::read_to_string(object_path(&v, &bv.s("sha256")).unwrap()).unwrap(), "v1 stable", "pre-save backup has the previous bytes");
        assert_eq!(fs::read_to_string(object_path(&v, &av.s("sha256")).unwrap()).unwrap(), "v2 instashoot fixed", "post-save backup has the new bytes");
        assert_eq!(bv.s("name"), "weapon_tuner_instashoot_fix_before.gsc");
        assert_eq!(av.s("name"), "weapon_tuner_instashoot_fix.gsc");
        // same name again -> no collision
        let o2 = SmartSave { backup_before: false, backup_after: true, name: "weapon_tuner_instashoot_fix.gsc", description: "again", tags: "" };
        let r2 = smart_save(&v, &fid, "v3", &sha_of(&path), "lf", false, &o2).unwrap();
        let n2 = v.db.one("SELECT name FROM file_versions WHERE id=?", p![r2.str_of("afterBackupId").unwrap()]).unwrap().unwrap().s("name");
        assert_eq!(n2, "weapon_tuner_instashoot_fix_2.gsc");
        // portable metadata exists next to the store
        let side = files::walk(&storage_root(&v).join("history"), &files::Ignore::parse(""));
        assert!(side.files.keys().any(|k| k.contains("weapon-tuner-instashoot-fix") && k.ends_with(".json")));
        // searchable by description and tags
        let hits = crate::search::search(&v, "instashoot fix", None, &["backup".into()], 10).unwrap();
        assert!(hits.as_arr().unwrap().iter().any(|h| h.str_of("id").as_deref() == Some(after.as_str())));
        assert!(crate::search::search(&v, "working", None, &["backup".into()], 10).unwrap().as_arr().unwrap().len() >= 1);
        // browse filters
        assert_eq!(browse(&v, None, "instashoot", true, None, None, 50).unwrap().as_arr().unwrap().len(), 3);
        // restore the pre-save backup: exact bytes, current kept first
        restore_opts(&v, &before, true).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"v1 stable");
        assert!(list(&v, &fid).unwrap().to_string().contains("before-restore"));
        // history timeline labels sources
        let h = history(&v, &fid).unwrap();
        let srcs: Vec<String> = h.as_arr().unwrap().iter().map(|x| x.str_of("source").unwrap()).collect();
        assert!(srcs.contains(&"backup".to_string()) && srcs.contains(&"project-vault".to_string()));
        // conflict: external change after opening -> nothing written, no backup made
        let opened = sha_of(&path);
        fs::write(&path, "edited elsewhere").unwrap();
        let n_before = v.db.scalar_i64("SELECT count(*) FROM file_versions", p![]).unwrap();
        let e = smart_save(&v, &fid, "mine", &opened, "lf", false, &o).unwrap_err();
        assert!(e.starts_with("CONFLICT:"));
        assert_eq!(fs::read_to_string(&path).unwrap(), "edited elsewhere");
        assert_eq!(v.db.scalar_i64("SELECT count(*) FROM file_versions", p![]).unwrap(), n_before);
        // metadata persists across restart
        let data = v.data_dir.clone();
        drop(v);
        let v2 = Vault::open(&data).unwrap();
        let b2 = browse(&v2, None, "instashoot", true, None, None, 50).unwrap();
        assert!(b2.to_string().contains("Instashoot fix"));
        drop(t);
    }

    #[test]
    fn dedup_backups_restore_independently_and_storage_errors() {
        let (_t, v, d, _pid, fid) = setup("a.cfg", "same content");
        fs::write(d.join("scripts/b.cfg"), "same content").unwrap();
        let pid = v.db.one("SELECT project_id FROM files WHERE id=?", p![fid.clone()]).unwrap().unwrap().s("project_id");
        files::scan(&v, &pid).unwrap();
        let fid_b = v.db.one("SELECT id FROM files WHERE name='b.cfg'", p![]).unwrap().unwrap().s("id");
        backup_now(&v, &fid, "a_named.cfg", "A", "").unwrap();
        backup_now(&v, &fid_b, "b_named.cfg", "B", "").unwrap();
        let va = v.db.one("SELECT id, sha256 FROM file_versions WHERE name='a_named.cfg'", p![]).unwrap().unwrap();
        let vb = v.db.one("SELECT id, sha256 FROM file_versions WHERE name='b_named.cfg'", p![]).unwrap().unwrap();
        assert_eq!(va.s("sha256"), vb.s("sha256"), "one stored object");
        // deleting one backup keeps the shared object for the other
        delete_version(&v, &va.s("id")).unwrap();
        fs::write(d.join("scripts/b.cfg"), "changed").unwrap();
        restore_opts(&v, &vb.s("id"), false).unwrap();
        assert_eq!(fs::read_to_string(d.join("scripts/b.cfg")).unwrap(), "same content");
        // prune never removes named backups unless asked; a dry run deletes nothing
        let before = v.db.one("SELECT count(*) AS n FROM file_versions", p![]).unwrap().unwrap().i("n");
        let dry = prune_impl(&v, &pid, 1, true, true).unwrap();
        assert!(dry.get("wouldDelete").and_then(|x| x.as_i64()).unwrap() > 0);
        assert_eq!(v.db.one("SELECT count(*) AS n FROM file_versions", p![]).unwrap().unwrap().i("n"), before, "dry run must not delete");
        let r = prune_opts(&v, &pid, 1, false).unwrap();
        assert!(v.db.one("SELECT id FROM file_versions WHERE name='b_named.cfg'", p![]).unwrap().is_some(), "{}", r.to_string());
        // move the store elsewhere, then make it unavailable -> clear error, nothing written
        let newroot = d.parent().unwrap().join("ext-drive/PVBackups");
        set_storage_root(&v, &newroot).unwrap();
        assert!(newroot.join("objects").is_dir());
        restore_opts(&v, &vb.s("id"), false).unwrap(); // still restorable from the new location
        fs::remove_dir_all(d.parent().unwrap().join("ext-drive")).unwrap();
        let e = backup_now(&v, &fid_b, "x", "", "").unwrap_err();
        assert!(e.contains("not available"), "{e}");
    }
}

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

fn object_path(v: &Vault, sha: &str) -> DbResult<PathBuf> {
    if sha.len() != 64 || !sha.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err("invalid object id".into());
    }
    Ok(v.data_dir.join("objects").join(&sha[..2]).join(&sha[2..]))
}

/// Copies `src` into the object store; verifies the content hash.
fn store(v: &Vault, src: &Path, expected_sha: &str) -> DbResult<()> {
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
    let id = new_id();
    v.db.exec(
        "INSERT INTO file_versions(id,project_id,file_id,sha256,size,rel_path,reason,note,created_at) VALUES (?,?,?,?,?,?,?,?,?)",
        p![id.clone(), f.s("project_id"), f.s("id"), sha, f.i("size"), f.s("rel_path"), reason, note, now_ms()],
    )?;
    Ok(Some(id))
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

pub fn save_manual(v: &Vault, file_id: &str, note: &str) -> DbResult<Json> {
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
    add_version(v, &f, &root, "manual", note)?;
    timeline::add_coalesced(v, &f.s("project_id"), "version", &format!("Saved a version of {}", f.s("rel_path")), "file", file_id)?;
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
    } else {
        let r = v.db.one("SELECT * FROM file_versions WHERE id=? AND file_id=?", p![which, file_id])?.ok_or("Version not found")?;
        Ok((object_path(v, &r.s("sha256"))?, r.s("sha256"), r.i("size"), format!("Version from {}", crate::util::iso_utc(r.i("created_at")))))
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
    let text_like = files::is_text_ext(&ext) || matches!(files::classify(&ext), "code" | "config" | "document") && ext != "pdf";
    let max = 8 * 1024 * 1024;
    if text_like && za <= max && zb <= max {
        if let (Some(ta), Some(tb)) = (read_text(&pa), read_text(&pb)) {
            out.set("kind", "text");
            out.set("diff", diff::diff_json(&ta, &tb, 3));
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
        store(v, &target, &cur)?;
        v.db.exec(
            "INSERT INTO file_versions(id,project_id,file_id,sha256,size,rel_path,reason,note,created_at) VALUES (?,?,?,?,?,?,'before-restore','',?)",
            p![new_id(), pid.clone(), fid.clone(), cur, std::fs::metadata(&target).map(|m| m.len() as i64).unwrap_or(0), f.s("rel_path"), now_ms()],
        )?;
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
    timeline::add(v, &pid, "version", &format!("Restored {} to the version from {}", f.s("rel_path"), crate::util::iso_utc(ver.i("created_at"))), "", Some(("file", &fid)), false)?;
    files::scan(v, &pid)?;
    Ok(Json::obj().with("ok", true).with("unchanged", false))
}

pub fn delete_version(v: &Vault, version_id: &str) -> DbResult<()> {
    v.db.exec("DELETE FROM file_versions WHERE id=?", p![version_id])?;
    gc(v)?;
    Ok(())
}

/// Keeps the newest `keep` versions per file (and always the newest one).
pub fn prune(v: &Vault, project_id: &str, keep: i64) -> DbResult<Json> {
    let keep = keep.max(1);
    let n = v.db.exec(
        "DELETE FROM file_versions WHERE project_id=? AND id IN (
           SELECT id FROM (SELECT id, ROW_NUMBER() OVER (PARTITION BY file_id ORDER BY created_at DESC) AS rn
                           FROM file_versions WHERE project_id=?) WHERE rn>?)",
        p![project_id, project_id, keep],
    )?;
    let freed = gc(v)?;
    Ok(Json::obj().with("deletedVersions", n).with("freedBytes", freed))
}

/// Removes objects no version refers to. Returns bytes freed.
pub fn gc(v: &Vault) -> DbResult<i64> {
    let dir = v.data_dir.join("objects");
    let Ok(rd) = std::fs::read_dir(&dir) else { return Ok(0) };
    let referenced: std::collections::HashSet<String> =
        v.db.query("SELECT DISTINCT sha256 FROM file_versions", p![])?.iter().map(|r| r.s("sha256")).collect();
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
}

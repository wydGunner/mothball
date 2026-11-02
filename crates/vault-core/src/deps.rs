//! Files a project needs that live outside it — e.g. the modded iw4x.dll an
//! IW4X mod only works with.
//!
//! Attaching a dependency keeps a verified copy in Mothball's version storage
//! (`dependencies/<sha256>`), so the exact file can't get lost, and records
//! where it has to be for the project to work ("place at"). Installing puts
//! that copy in place; Deploy and Launch do it automatically, so switching
//! between two versions of a mod also swaps the DLL each one needs.
//!
//! Nothing a user owns is overwritten blindly: if the file being replaced is
//! not a dependency Mothball knows, it is kept first and can be put back.

use crate::db::DbResult;
use crate::json::Json;
use crate::util::{new_id, now_ms, sha256_file};
use crate::{p, projects, timeline, versions, Vault};
use std::path::{Path, PathBuf};

fn key(pid: &str) -> String {
    format!("deps:{pid}")
}
const REPLACED: &str = "deps-replaced";

fn meta_get(v: &Vault, k: &str) -> DbResult<Json> {
    let s = v.db.one("SELECT value FROM meta WHERE key=?", p![k])?.map(|r| r.s("value")).unwrap_or_default();
    Ok(Json::parse(&s).unwrap_or(Json::Null))
}
fn meta_set(v: &Vault, k: &str, j: &Json) -> DbResult<()> {
    v.db.exec("INSERT OR REPLACE INTO meta(key,value) VALUES (?,?)", p![k, j.to_string()])?;
    Ok(())
}

fn store_dir(v: &Vault) -> PathBuf {
    versions::storage_root(v).join("dependencies")
}
fn stored(v: &Vault, sha: &str) -> PathBuf {
    store_dir(v).join(sha)
}

fn deps_of(v: &Vault, pid: &str) -> DbResult<Vec<Json>> {
    Ok(meta_get(v, &key(pid))?.as_arr().cloned().unwrap_or_default())
}
fn save_deps(v: &Vault, pid: &str, list: Vec<Json>) -> DbResult<()> {
    meta_set(v, &key(pid), &Json::Arr(list))
}

/// Every dependency of every project: (project id, project name, dep).
fn all_deps(v: &Vault) -> DbResult<Vec<(String, String, Json)>> {
    let mut out = Vec::new();
    for r in v.db.query("SELECT m.key, p.name FROM meta m JOIN projects p ON m.key = 'deps:' || p.id", p![])? {
        let pid = r.s("key")[5..].to_string();
        for d in deps_of(v, &pid)? {
            out.push((pid.clone(), r.s("name"), d));
        }
    }
    Ok(out)
}

/// Copies `src` into the store under its hash; returns (sha, size).
fn keep_copy(v: &Vault, src: &Path) -> Result<(String, i64), String> {
    let sha = sha256_file(src).map_err(|e| format!("Can't read {}: {e}", src.display()))?;
    let size = std::fs::metadata(src).map(|m| m.len() as i64).unwrap_or(0);
    let dest = stored(v, &sha);
    if sha256_file(&dest).ok().as_deref() != Some(sha.as_str()) {
        std::fs::create_dir_all(store_dir(v)).map_err(|e| e.to_string())?;
        let tmp = store_dir(v).join(format!(".tmp-{}", &new_id()[..8]));
        std::fs::copy(src, &tmp).map_err(|e| format!("Couldn't keep a copy of {}: {e}", src.display()))?;
        if sha256_file(&tmp).ok().as_deref() != Some(sha.as_str()) {
            let _ = std::fs::remove_file(&tmp);
            return Err(format!("{} changed while it was being copied; try again", src.display()));
        }
        let _ = std::fs::remove_file(&dest);
        std::fs::rename(&tmp, &dest).map_err(|e| e.to_string())?;
    }
    Ok((sha, size))
}

fn file_name(p: &str) -> String {
    p.rsplit(['/', '\\']).next().unwrap_or(p).to_string()
}

pub fn add(v: &Vault, pid: &str, a: &Json) -> DbResult<Json> {
    projects::root_of(v, pid)?;
    let path = a.str_of("path").unwrap_or_default();
    let src = PathBuf::from(path.trim());
    if !src.is_file() {
        return Err(format!("{} isn't a file Mothball can read", src.display()));
    }
    let place = a.str_of("placeAt").map(|s| s.trim().to_string()).unwrap_or_else(|| src.to_string_lossy().to_string());
    if !place.is_empty() && !Path::new(&place).is_absolute() {
        return Err("\"Where it has to be\" must be a full path, like D:\\Games\\MW2\\iw4x.dll".into());
    }
    let mut list = deps_of(v, pid)?;
    if !place.is_empty() && list.iter().any(|d| d.str_of("placeAt").map(|p| same_path(&p, &place)).unwrap_or(false)) {
        return Err("This project already has a dependency that goes there. Remove or edit that one first.".into());
    }
    let (sha, size) = keep_copy(v, &src)?;
    let name = a.str_of("name").map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).unwrap_or_else(|| file_name(&path));
    let d = Json::obj()
        .with("id", new_id())
        .with("name", name.as_str())
        .with("fileName", file_name(&path))
        .with("sha256", sha.as_str())
        .with("size", size)
        .with("placeAt", place.as_str())
        .with("note", a.str_of("note").unwrap_or_default())
        .with("addedAt", now_ms());
    list.push(d.clone());
    save_deps(v, pid, list)?;
    timeline::add(v, pid, "dependency", &format!("Dependency attached: {name}"), &format!("{} · a copy is kept in Mothball", if place.is_empty() { file_name(&path) } else { place.clone() }), None, false)?;
    Ok(d)
}

pub fn update(v: &Vault, pid: &str, id: &str, patch: &Json) -> DbResult<Json> {
    let mut list = deps_of(v, pid)?;
    let i = list.iter().position(|d| d.str_of("id").as_deref() == Some(id)).ok_or("Dependency not found")?;
    if let Some(p) = patch.str_of("placeAt").map(|s| s.trim().to_string()) {
        if !p.is_empty() && !Path::new(&p).is_absolute() {
            return Err("\"Where it has to be\" must be a full path".into());
        }
        if !p.is_empty() && list.iter().enumerate().any(|(j, d)| j != i && d.str_of("placeAt").map(|q| same_path(&q, &p)).unwrap_or(false)) {
            return Err("Another dependency of this project already goes there.".into());
        }
        list[i].set("placeAt", p);
    }
    for k in ["name", "note"] {
        if let Some(s) = patch.str_of(k) {
            list[i].set(k, s.trim().to_string());
        }
    }
    let d = list[i].clone();
    save_deps(v, pid, list)?;
    Ok(d)
}

/// Removes the dependency (its stored copy too, unless something else uses it).
/// The installed file is left where it is.
pub fn remove(v: &Vault, pid: &str, id: &str) -> DbResult<Json> {
    let mut list = deps_of(v, pid)?;
    let Some(i) = list.iter().position(|d| d.str_of("id").as_deref() == Some(id)) else { return Ok(Json::obj().with("ok", true)) };
    let d = list.remove(i);
    save_deps(v, pid, list)?;
    gc(v, &d.str_of("sha256").unwrap_or_default())?;
    Ok(Json::obj().with("ok", true))
}

/// Forget a removed project's dependencies.
pub fn forget(v: &Vault, pid: &str) -> DbResult<()> {
    let list = deps_of(v, pid)?;
    v.db.exec("DELETE FROM meta WHERE key=?", p![key(pid)])?;
    for d in list {
        gc(v, &d.str_of("sha256").unwrap_or_default())?;
    }
    Ok(())
}

fn gc(v: &Vault, sha: &str) -> DbResult<()> {
    if sha.is_empty() {
        return Ok(());
    }
    let used = all_deps(v)?.iter().any(|(_, _, d)| d.str_of("sha256").as_deref() == Some(sha))
        || replaced(v)?.iter().any(|r| r.str_of("sha256").as_deref() == Some(sha));
    if !used {
        let _ = std::fs::remove_file(stored(v, sha));
    }
    Ok(())
}

fn same_path(a: &str, b: &str) -> bool {
    let n = |s: &str| s.replace('\\', "/").trim_end_matches('/').to_lowercase();
    n(a) == n(b)
}

fn replaced(v: &Vault) -> DbResult<Vec<Json>> {
    Ok(meta_get(v, REPLACED)?.as_arr().cloned().unwrap_or_default())
}

fn status_of(v: &Vault, d: &Json, everyone: &[(String, String, Json)], backups: &[Json]) -> Json {
    let sha = d.str_of("sha256").unwrap_or_default();
    let place = d.str_of("placeAt").unwrap_or_default();
    let have = stored(v, &sha).metadata().map(|m| m.len() as i64 == d.get("size").and_then(|s| s.as_i64()).unwrap_or(-1)).unwrap_or(false);
    let mut out = d.clone().with("stored", have);
    let state = if place.is_empty() {
        "kept"
    } else {
        let pp = PathBuf::from(&place);
        match pp.parent() {
            Some(dir) if !dir.is_dir() => "unreachable",
            _ if !pp.exists() => "missing",
            _ => match sha256_file(&pp) {
                Ok(s) if s == sha => "installed",
                Ok(s) => {
                    if let Some((_, pname, od)) = everyone.iter().find(|(_, _, od)| od.str_of("sha256").as_deref() == Some(s.as_str())) {
                        out.set("inPlaceBelongsTo", format!("{pname}'s {}", od.str_of("name").unwrap_or_default()));
                    }
                    "different"
                }
                Err(_) => "unreadable",
            },
        }
    };
    out.set("state", state);
    if backups.iter().any(|b| b.str_of("path").map(|p| same_path(&p, &place)).unwrap_or(false)) {
        out.set("canRestoreOriginal", true);
    }
    out
}

pub fn list(v: &Vault, pid: &str) -> DbResult<Json> {
    let everyone = all_deps(v)?;
    let backups = replaced(v)?;
    Ok(Json::Arr(deps_of(v, pid)?.iter().map(|d| status_of(v, d, &everyone, &backups)).collect()))
}

/// Puts each dependency (or just `only`) in place. Returns per-dependency results.
pub fn install(v: &Vault, pid: &str, only: Option<&str>) -> DbResult<Json> {
    let everyone = all_deps(v)?;
    let mut results = Vec::new();
    let mut backups = replaced(v)?;
    for d in deps_of(v, pid)? {
        if only.is_some() && d.str_of("id").as_deref() != only {
            continue;
        }
        let name = d.str_of("name").unwrap_or_default();
        let place = d.str_of("placeAt").unwrap_or_default();
        if place.is_empty() {
            continue;
        }
        let sha = d.str_of("sha256").unwrap_or_default();
        let res = (|| -> Result<&'static str, String> {
            let src = stored(v, &sha);
            if sha256_file(&src).ok().as_deref() != Some(sha.as_str()) {
                return Err("Mothball's copy is missing or damaged; attach the file again".into());
            }
            let dest = PathBuf::from(&place);
            let dir = dest.parent().ok_or("bad location")?;
            if !dir.is_dir() {
                return Err(format!("{} doesn't exist (drive not connected?)", dir.display()));
            }
            if dest.exists() {
                let cur = sha256_file(&dest).map_err(|e| format!("can't read the file in place: {e}"))?;
                if cur == sha {
                    return Ok("already");
                }
                // not a dependency Mothball knows: keep it so it can be put back
                let known = everyone.iter().any(|(_, _, od)| od.str_of("sha256").as_deref() == Some(cur.as_str()));
                if !known && !backups.iter().any(|b| b.str_of("sha256").as_deref() == Some(cur.as_str()) && b.str_of("path").map(|p| same_path(&p, &place)).unwrap_or(false)) {
                    let (bsha, size) = keep_copy(v, &dest)?;
                    backups.push(Json::obj().with("path", place.as_str()).with("sha256", bsha).with("size", size).with("at", now_ms()));
                    meta_set(v, REPLACED, &Json::Arr(backups.clone()))?;
                }
            }
            crate::deploy::copy_verified(&src, &dest, &sha).map_err(|e| e.to_string())?;
            Ok("installed")
        })();
        results.push(match res {
            Ok(s) => Json::obj().with("name", name.as_str()).with("result", s),
            Err(e) => Json::obj().with("name", name.as_str()).with("result", "failed").with("error", e),
        });
    }
    let n = results.iter().filter(|r| r.str_of("result").as_deref() == Some("installed")).count();
    if n > 0 {
        let names: Vec<String> = results.iter().filter(|r| r.str_of("result").as_deref() == Some("installed")).filter_map(|r| r.str_of("name")).collect();
        timeline::add(v, pid, "dependency", &format!("Dependencies put in place: {}", names.join(", ")), "", None, false)?;
    }
    Ok(Json::Arr(results))
}

/// Puts back the file that was in `placeAt` before Mothball first replaced it.
pub fn restore_original(v: &Vault, pid: &str, id: &str) -> DbResult<Json> {
    let d = deps_of(v, pid)?.into_iter().find(|d| d.str_of("id").as_deref() == Some(id)).ok_or("Dependency not found")?;
    let place = d.str_of("placeAt").unwrap_or_default();
    let b = replaced(v)?.into_iter().rev().find(|b| b.str_of("path").map(|p| same_path(&p, &place)).unwrap_or(false)).ok_or("There's no earlier file kept for that location")?;
    let sha = b.str_of("sha256").unwrap_or_default();
    crate::deploy::copy_verified(&stored(v, &sha), Path::new(&place), &sha)?;
    timeline::add(v, pid, "dependency", &format!("Original file put back: {}", file_name(&place)), &place, None, false)?;
    Ok(Json::obj().with("ok", true))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_versions_of_a_mod_swap_the_dll_each_needs() {
        let (t, v) = crate::testutil::vault();
        let game = t.0.join("MW2");
        std::fs::create_dir_all(&game).unwrap();
        let dll = game.join("iw4x.dll");
        std::fs::write(&dll, b"stock dll").unwrap(); // what the user had
        let mk = |name: &str| crate::api::dispatch(&v, "projects.create", &Json::obj().with("name", name).with("category", "addon")).unwrap().str_of("id").unwrap();
        let (old, new) = (mk("Old Omen"), mk("Newest Omen"));
        let src_old = t.0.join("old.dll");
        let src_new = t.0.join("new.dll");
        std::fs::write(&src_old, b"old modded dll").unwrap();
        std::fs::write(&src_new, b"new modded dll").unwrap();
        let place = dll.to_string_lossy().to_string();
        let a = add(&v, &old, &Json::obj().with("path", src_old.to_string_lossy().to_string()).with("name", "iw4x.dll (old)").with("placeAt", place.as_str())).unwrap();
        add(&v, &new, &Json::obj().with("path", src_new.to_string_lossy().to_string()).with("name", "iw4x.dll").with("placeAt", place.as_str())).unwrap();
        // the original can go away; Mothball has its own copy
        std::fs::remove_file(&src_old).unwrap();
        assert!(add(&v, &old, &Json::obj().with("path", src_new.to_string_lossy().to_string()).with("placeAt", place.as_str())).unwrap_err().contains("already has a dependency"));

        let st = |pid: &str| list(&v, pid).unwrap().as_arr().unwrap()[0].clone();
        assert_eq!(st(&old).str_of("state").as_deref(), Some("different"));
        assert_eq!(st(&old).get("inPlaceBelongsTo"), None, "the stock dll belongs to no project");

        // installing Old Omen's: the stock dll is kept first
        let r = install(&v, &old, None).unwrap();
        assert_eq!(r.as_arr().unwrap()[0].str_of("result").as_deref(), Some("installed"));
        assert_eq!(std::fs::read(&dll).unwrap(), b"old modded dll");
        assert_eq!(st(&old).str_of("state").as_deref(), Some("installed"));
        assert_eq!(st(&new).str_of("inPlaceBelongsTo").as_deref(), Some("Old Omen's iw4x.dll (old)"));

        // switching to Newest Omen swaps it; a known dependency isn't "backed up" again
        install(&v, &new, None).unwrap();
        assert_eq!(std::fs::read(&dll).unwrap(), b"new modded dll");
        assert_eq!(replaced(&v).unwrap().len(), 1);
        assert_eq!(install(&v, &new, None).unwrap().as_arr().unwrap()[0].str_of("result").as_deref(), Some("already"));

        // the stock dll can be put back
        assert_eq!(st(&old).get("canRestoreOriginal").and_then(|b| b.as_bool()), Some(true));
        restore_original(&v, &old, &a.str_of("id").unwrap()).unwrap();
        assert_eq!(std::fs::read(&dll).unwrap(), b"stock dll");

        // removing a dependency keeps the stored copy while something else uses it
        let sha_old = a.str_of("sha256").unwrap();
        remove(&v, &old, &a.str_of("id").unwrap()).unwrap();
        assert!(!stored(&v, &sha_old).exists(), "unused copy removed");
        assert!(stored(&v, &sha256_file(&src_new).unwrap()).exists());

        // an unreachable location is reported, not an error
        let p3 = mk("Elsewhere");
        add(&v, &p3, &Json::obj().with("path", src_new.to_string_lossy().to_string()).with("placeAt", t.0.join("Z-drive/game/x.dll").to_string_lossy().to_string())).unwrap();
        assert_eq!(st(&p3).str_of("state").as_deref(), Some("unreachable"));
        assert_eq!(install(&v, &p3, None).unwrap().as_arr().unwrap()[0].str_of("result").as_deref(), Some("failed"));
    }
}

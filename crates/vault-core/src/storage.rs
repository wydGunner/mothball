//! Where Mothball keeps the folders of projects it manages ("app storage").
//!
//! By default that is `<data dir>/managed`, i.e. on the system drive under
//! AppData. It can be pointed at any folder (another drive); existing managed
//! projects can be moved there. A move copies every file, verifies each copy
//! by SHA-256 and keeps modified times, switches the project over, and only
//! then deletes the old folder, so the space on the old drive is really freed.
//! Projects are moved one at a time: if one fails, it stays where it was and
//! the others are unaffected.

use crate::db::DbResult;
use crate::files::{walk, Ignore};
use crate::json::Json;
use crate::projects::{display_path, dunce};
use crate::util::{now_ms, safe_join, sha256_file};
use crate::{p, timeline, Vault};
use std::path::{Path, PathBuf};

pub fn default_root(v: &Vault) -> PathBuf {
    v.data_dir.join("managed")
}

/// Where new managed projects are created.
pub fn root(v: &Vault) -> PathBuf {
    v.db.one("SELECT value FROM meta WHERE key='managed_root'", p![])
        .ok()
        .flatten()
        .map(|r| PathBuf::from(r.s("value")))
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| default_root(v))
}

/// True if `path` is a project folder inside one of Mothball's storage
/// locations (current or default) — and not the location itself.
pub fn is_managed_folder(v: &Vault, path: &Path) -> bool {
    [root(v), default_root(v)].iter().any(|r| path.starts_with(r) && path != r.as_path())
}

fn folder_size(dir: &Path) -> (i64, i64) {
    let w = walk(dir, &Ignore::parse(""));
    (w.files.len() as i64, w.files.values().map(|s| s.size).sum())
}

pub fn info(v: &Vault) -> DbResult<Json> {
    let rows = v.db.query("SELECT id, name, root_path FROM projects WHERE mode='managed' ORDER BY name", p![])?;
    let cur = root(v);
    let (mut elsewhere, mut bytes_elsewhere, mut bytes_here) = (0i64, 0i64, 0i64);
    for r in &rows {
        let path = PathBuf::from(r.s("root_path"));
        let (_, b) = folder_size(&path);
        if path.starts_with(&cur) {
            bytes_here += b;
        } else {
            elsewhere += 1;
            bytes_elsewhere += b;
        }
    }
    Ok(Json::obj()
        .with("root", display_path(&cur))
        .with("isDefault", dunce(&cur) == dunce(&default_root(v)))
        .with("defaultRoot", display_path(&default_root(v)))
        .with("projects", rows.len())
        .with("bytes", bytes_here + bytes_elsewhere)
        .with("bytesHere", bytes_here)
        .with("notHere", elsewhere)
        .with("bytesNotHere", bytes_elsewhere))
}

fn is_drive_root(p: &Path) -> bool {
    p.parent().is_none()
}

/// Sets the location for managed projects; with `move_existing`, moves the
/// managed projects that live elsewhere into it.
pub fn set_root(v: &Vault, folder: &Path, move_existing: bool) -> DbResult<Json> {
    if is_drive_root(folder) {
        return Err(format!("Choose or create a folder on that drive (for example {}), not the drive itself.", folder.join("Mothball Projects").display()));
    }
    std::fs::create_dir_all(folder).map_err(|e| format!("Cannot use {}: {e}", folder.display()))?;
    let new_root = dunce(folder);
    let probe = new_root.join(format!(".mothball-write-test-{}", &crate::util::new_id()[..8]));
    std::fs::write(&probe, b"ok").map_err(|e| format!("{} is not writable: {e}", new_root.display()))?;
    let _ = std::fs::remove_file(&probe);
    // must not be inside a project (it would index itself) or inside Mothball's data
    for r in v.db.query("SELECT name, root_path FROM projects", p![])? {
        let pr = PathBuf::from(r.s("root_path"));
        if new_root.starts_with(&pr) {
            return Err(format!("That folder is inside the project \"{}\". Choose a folder outside your projects.", r.s("name")));
        }
    }
    if new_root.starts_with(dunce(&v.data_dir)) && new_root != dunce(&default_root(v)) {
        return Err("Choose a folder outside Mothball's data folder.".into());
    }
    let value = if new_root == dunce(&default_root(v)) { String::new() } else { new_root.to_string_lossy().to_string() };
    v.db.exec("INSERT OR REPLACE INTO meta(key,value) VALUES ('managed_root',?)", p![value])?;

    let mut moved = Vec::new();
    let mut failed = Vec::new();
    let mut freed = 0i64;
    if move_existing {
        let rows = v.db.query("SELECT id, name, root_path FROM projects WHERE mode='managed' ORDER BY name", p![])?;
        for r in rows {
            let old = PathBuf::from(r.s("root_path"));
            if old.starts_with(&new_root) {
                continue;
            }
            match move_project(v, &r.s("id"), &old, &new_root) {
                Ok((bytes, left_behind)) => {
                    freed += if left_behind { 0 } else { bytes };
                    moved.push(Json::obj().with("name", r.s("name")).with("bytes", bytes).with("oldFolderKept", left_behind));
                }
                Err(e) => failed.push(Json::obj().with("name", r.s("name")).with("error", e)),
            }
        }
    }
    Ok(info(v)?.with("moved", Json::Arr(moved)).with("failed", Json::Arr(failed)).with("freed", freed))
}

/// Copies a folder (minus ignored paths) into `dest`, checking every copy
/// against its original and keeping modified times. On failure the partial
/// copy is removed. Returns (files, bytes).
pub fn copy_tree_verified(src: &Path, dest: &Path, ignore: &Ignore) -> Result<(i64, i64), String> {
    let w = walk(src, ignore);
    if let Some(e) = w.errors.first() {
        return Err(format!("Couldn't read the whole folder ({e})"));
    }
    std::fs::create_dir_all(dest).map_err(|e| e.to_string())?;
    let mut bytes = 0i64;
    for (rel, seen) in &w.files {
        let res = (|| -> Result<(), String> {
            let from = safe_join(src, rel)?;
            let to = safe_join(dest, rel)?;
            if let Some(d) = to.parent() {
                std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
            }
            std::fs::copy(&from, &to).map_err(|e| format!("{rel}: {e}"))?;
            if sha256_file(&from).ok() != sha256_file(&to).ok() {
                return Err(format!("{rel}: the copy didn't match the original"));
            }
            // keep the modified time so nothing looks changed afterwards
            if let Ok(mt) = std::fs::metadata(&from).and_then(|m| m.modified()) {
                if let Ok(f) = std::fs::File::options().write(true).open(&to) {
                    let _ = f.set_modified(mt);
                }
            }
            Ok(())
        })();
        if let Err(e) = res {
            let _ = std::fs::remove_dir_all(dest);
            return Err(e);
        }
        bytes += seen.size;
    }
    Ok((w.files.len() as i64, bytes))
}

/// New managed project filled with a copy of `src`; the original is never touched.
pub fn create_copy(v: &Vault, args: &Json, src: &Path) -> DbResult<Json> {
    if !src.is_dir() {
        return Err(format!("{} is not an accessible folder", src.display()));
    }
    let mut a = args.clone();
    a.remove("linkPath");
    a.remove("copyFrom");
    let pr = crate::api::dispatch(v, "projects.create", &a)?;
    let id = pr.str_of("id").unwrap_or_default();
    let root = PathBuf::from(pr.str_of("rootPath").unwrap_or_default());
    if dunce(src) == dunce(&root) || root.starts_with(dunce(src)) {
        let _ = crate::projects::remove(v, &id, true);
        return Err("Choose a folder outside Mothball's storage to copy from".into());
    }
    let ignore = v.db.one("SELECT ignore_globs FROM projects WHERE id=?", p![id.as_str()])?.map(|r| r.s("ignore_globs")).unwrap_or_default();
    match copy_tree_verified(src, &root, &Ignore::parse(&ignore)) {
        Ok((files, bytes)) => {
            timeline::add(v, &id, "project", &format!("Copied {files} {} in", if files == 1 { "file" } else { "files" }), &format!("from {} ({} bytes); the original folder was not changed", src.display(), bytes), None, false)?;
            Ok(crate::projects::get(v, &id)?.with("copied", files))
        }
        Err(e) => {
            let _ = crate::projects::remove(v, &id, false);
            let _ = std::fs::remove_dir_all(&root);
            Err(format!("Copying failed ({e}). No project was created; your folder was not changed."))
        }
    }
}

/// Copies one project folder into `new_root`, verifies it, switches the
/// project over and deletes the old folder. Returns (bytes, old folder kept).
fn move_project(v: &Vault, id: &str, old: &Path, new_root: &Path) -> Result<(i64, bool), String> {
    let name = old.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| id.to_string());
    let mut dest = new_root.join(&name);
    let mut n = 2;
    while dest.exists() {
        dest = new_root.join(format!("{name}-{n}"));
        n += 1;
    }
    let bytes = if old.is_dir() {
        copy_tree_verified(old, &dest, &Ignore::parse("")).map_err(|e| format!("{e}. Nothing was moved; the project is still in {}.", old.display()))?.1
    } else {
        std::fs::create_dir_all(&dest).map_err(|e| e.to_string())?;
        0
    };
    let cleanup = |d: &Path| {
        let _ = std::fs::remove_dir_all(d);
    };
    v.db.exec("UPDATE projects SET root_path=?, updated_at=? WHERE id=?", p![dest.to_string_lossy().to_string(), now_ms(), id])
        .inspect_err(|_| cleanup(&dest))?;
    timeline::add(v, id, "project", "Project folder moved", &format!("{} → {}", old.display(), dest.display()), None, false)?;
    // the new copy is verified and in use; now free the old space
    let kept = old.is_dir() && std::fs::remove_dir_all(old).is_err();
    Ok((bytes, kept))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moves_managed_projects_and_frees_the_old_space() {
        let (t, v) = crate::testutil::vault();
        let pr = crate::api::dispatch(&v, "projects.create", &Json::obj().with("name", "3D Prints")).unwrap();
        let pid = pr.str_of("id").unwrap();
        let old = PathBuf::from(pr.str_of("rootPath").unwrap());
        assert!(old.starts_with(default_root(&v)));
        std::fs::create_dir_all(old.join("plates")).unwrap();
        std::fs::write(old.join("plates/cat.3mf"), vec![7u8; 50_000]).unwrap();
        std::fs::write(old.join("notes.txt"), b"hello").unwrap();
        crate::files::scan(&v, &pid).unwrap();
        let mt = std::fs::metadata(old.join("notes.txt")).unwrap().modified().unwrap();

        // the drive itself is refused; a folder inside a project is refused
        assert!(set_root(&v, Path::new("/"), true).unwrap_err().contains("not the drive itself"));
        assert!(set_root(&v, &old.join("plates"), true).unwrap_err().contains("inside the project"));

        let target = t.0.join("D-drive").join("Mothball Projects");
        let r = set_root(&v, &target, true).unwrap();
        assert_eq!(r.get("moved").unwrap().as_arr().unwrap().len(), 1, "{}", r.to_string());
        assert_eq!(r.get("freed").unwrap().as_i64(), Some(50_005));
        assert!(!old.exists(), "old folder deleted after the verified copy");
        let p = crate::projects::get(&v, &pid).unwrap();
        let new = PathBuf::from(p.str_of("rootPath").unwrap());
        assert!(new.starts_with(dunce(&target)));
        assert_eq!(std::fs::read(new.join("notes.txt")).unwrap(), b"hello");
        assert_eq!(std::fs::metadata(new.join("notes.txt")).unwrap().modified().unwrap(), mt, "modified time kept");
        // nothing looks changed to a rescan
        let s = crate::files::scan(&v, &pid).unwrap().json();
        for k in ["added", "modified", "missing"] {
            assert_eq!(s.get(k).and_then(|x| x.as_i64()).unwrap_or(0), 0, "{k}: {}", s.to_string());
        }
        // new projects go to the new place; removing with "delete folder" still works there
        let p2 = crate::api::dispatch(&v, "projects.create", &Json::obj().with("name", "Next")).unwrap();
        let p2root = PathBuf::from(p2.str_of("rootPath").unwrap());
        assert!(p2root.starts_with(dunce(&target)));
        assert!(is_managed_folder(&v, &p2root));
        assert!(!is_managed_folder(&v, &dunce(&target)));
        let i = info(&v).unwrap();
        assert_eq!(i.get("notHere").unwrap().as_i64(), Some(0));
        assert_eq!(i.get("isDefault").unwrap().as_bool(), Some(false));
        // and back to the default location
        let r = set_root(&v, &default_root(&v), true).unwrap();
        assert_eq!(r.get("moved").unwrap().as_arr().unwrap().len(), 2);
        assert_eq!(info(&v).unwrap().get("isDefault").unwrap().as_bool(), Some(true));
        assert_eq!(std::fs::read(PathBuf::from(crate::projects::get(&v, &pid).unwrap().str_of("rootPath").unwrap()).join("plates/cat.3mf")).unwrap().len(), 50_000);
    }

    #[test]
    fn copying_a_folder_in_leaves_the_original_alone() {
        let (t, v) = crate::testutil::vault();
        let src = t.0.join("OMEN old");
        std::fs::create_dir_all(src.join("scripts")).unwrap();
        std::fs::create_dir_all(src.join("node_modules/x")).unwrap();
        std::fs::write(src.join("scripts/a.gsc"), b"init() {}").unwrap();
        std::fs::write(src.join("node_modules/x/big.js"), b"x").unwrap();
        let p = crate::api::dispatch(&v, "projects.create", &Json::obj().with("name", "Old Omen").with("category", "software").with("copyFrom", src.to_string_lossy().to_string())).unwrap();
        assert_eq!(p.str_of("mode").as_deref(), Some("managed"));
        let root = PathBuf::from(p.str_of("rootPath").unwrap());
        assert!(root.starts_with(root_dir_of(&v)));
        assert_eq!(std::fs::read(root.join("scripts/a.gsc")).unwrap(), b"init() {}");
        assert!(!root.join("node_modules").exists(), "ignored folders are not copied");
        assert!(src.join("node_modules/x/big.js").exists() && src.join("scripts/a.gsc").exists(), "original untouched");
        // the copy is separate: editing it doesn't touch the original
        std::fs::write(root.join("scripts/a.gsc"), b"changed").unwrap();
        assert_eq!(std::fs::read(src.join("scripts/a.gsc")).unwrap(), b"init() {}");
        // a missing source creates nothing
        let n = v.db.scalar_i64("SELECT count(*) FROM projects", p![]).unwrap();
        assert!(crate::api::dispatch(&v, "projects.create", &Json::obj().with("name", "Nope").with("copyFrom", t.0.join("missing").to_string_lossy().to_string())).is_err());
        assert_eq!(v.db.scalar_i64("SELECT count(*) FROM projects", p![]).unwrap(), n);
    }

    fn root_dir_of(v: &Vault) -> PathBuf {
        dunce(&root(v))
    }

    /// An unreadable subfolder makes the move fail; the project must stay put.
    /// (Root can read anything, so this only bites as a normal user, e.g. in CI.)
    #[cfg(unix)]
    #[test]
    fn a_failed_move_leaves_the_project_where_it_was() {
        use std::os::unix::fs::PermissionsExt;
        if unsafe_is_root() {
            return;
        }
        let (t, v) = crate::testutil::vault();
        let pr = crate::api::dispatch(&v, "projects.create", &Json::obj().with("name", "Keep")).unwrap();
        let pid = pr.str_of("id").unwrap();
        let old = PathBuf::from(pr.str_of("rootPath").unwrap());
        std::fs::write(old.join("a.txt"), b"a").unwrap();
        let sub = old.join("locked");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join("b.txt"), b"b").unwrap();
        std::fs::set_permissions(&sub, std::fs::Permissions::from_mode(0o000)).unwrap();
        let r = set_root(&v, &t.0.join("elsewhere"), true);
        std::fs::set_permissions(&sub, std::fs::Permissions::from_mode(0o755)).unwrap();
        let r = r.unwrap();
        assert_eq!(r.get("failed").unwrap().as_arr().unwrap().len(), 1, "{}", r.to_string());
        assert_eq!(PathBuf::from(crate::projects::get(&v, &pid).unwrap().str_of("rootPath").unwrap()), old, "project not switched");
        assert!(old.join("a.txt").exists() && sub.join("b.txt").exists(), "original untouched");
        assert!(!t.0.join("elsewhere").join(old.file_name().unwrap()).exists(), "partial copy cleaned up");
    }

    #[cfg(unix)]
    fn unsafe_is_root() -> bool {
        std::fs::read_to_string("/proc/self/status").map(|s| s.lines().any(|l| l.starts_with("Uid:") && l.split_whitespace().nth(1) == Some("0"))).unwrap_or(false)
    }
}

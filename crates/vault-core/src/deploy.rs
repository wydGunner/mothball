//! Deploy & launch: copy a project (or one of its folders) into a game's mod
//! or addon folder in one click, then optionally start the game.
//!
//! Safety rules:
//! * Only files that differ are copied (compared by size, then SHA-256).
//! * A file in the destination that was changed there since the last deploy
//!   (or that this app never put there) is never overwritten silently: it is
//!   reported as a conflict and only replaced when the caller passes `force`.
//! * "Remove files deleted from the project" only removes files this app
//!   deployed before and that are still exactly as deployed.
//! * The destination can't be inside the project or contain it, or be a drive root.

use crate::db::DbResult;
use crate::files::{walk, Ignore};
use crate::json::Json;
use crate::util::{new_id, now_ms, safe_join, sha256_file};
use crate::{p, projects, timeline, Vault};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

fn key(pid: &str) -> String {
    format!("deploy:{pid}")
}
fn manifest_key(pid: &str) -> String {
    format!("deploy-manifest:{pid}")
}

fn meta_get(v: &Vault, k: &str) -> DbResult<Json> {
    let s = v.db.one("SELECT value FROM meta WHERE key=?", p![k])?.map(|r| r.s("value")).unwrap_or_default();
    Ok(Json::parse(&s).unwrap_or(Json::obj()))
}
fn meta_set(v: &Vault, k: &str, j: &Json) -> DbResult<()> {
    v.db.exec("INSERT OR REPLACE INTO meta(key,value) VALUES (?,?)", p![k, j.to_string()])?;
    Ok(())
}

pub fn get(v: &Vault, pid: &str) -> DbResult<Json> {
    let j = meta_get(v, &key(pid))?;
    Ok(Json::obj()
        .with("source", j.str_of("source").unwrap_or_default())
        .with("dest", j.str_of("dest").unwrap_or_default())
        .with("exclude", j.str_of("exclude").unwrap_or_default())
        .with("clean", j.get("clean").and_then(|x| x.as_bool()).unwrap_or(true))
        .with("exe", j.str_of("exe").unwrap_or_default())
        .with("args", j.str_of("args").unwrap_or_default())
        .with("cwd", j.str_of("cwd").unwrap_or_default())
        .with("last", j.get("last").cloned().unwrap_or(Json::Null)))
}

pub fn set(v: &Vault, pid: &str, patch: &Json) -> DbResult<Json> {
    projects::root_of(v, pid)?; // project exists
    let mut cur = get(v, pid)?;
    for k in ["source", "dest", "exclude", "exe", "args", "cwd"] {
        if let Some(s) = patch.str_of(k) {
            cur.set(k, s.trim().to_string());
        }
    }
    if let Some(b) = patch.get("clean").and_then(|x| x.as_bool()) {
        cur.set("clean", b);
    }
    let src = cur.str_of("source").unwrap_or_default();
    if src.contains("..") {
        return Err("The folder to deploy must be inside the project".into());
    }
    meta_set(v, &key(pid), &cur)?;
    Ok(cur)
}

/// Forget deploy settings when a project is removed.
pub fn forget(v: &Vault, pid: &str) -> DbResult<()> {
    v.db.exec("DELETE FROM meta WHERE key IN (?,?)", p![key(pid), manifest_key(pid)])?;
    Ok(())
}

struct Plan {
    src_root: PathBuf,
    dest: PathBuf,
    /// rel -> (action, size). action: copy | update | same | conflict
    items: BTreeMap<String, (&'static str, i64, String)>,
    /// deployed earlier, gone from the project: rel -> removable?
    stale: Vec<(String, bool)>,
}

fn norm(p: &Path) -> String {
    let s = p.to_string_lossy().replace('\\', "/").to_lowercase();
    s.trim_end_matches('/').to_string()
}

fn plan(v: &Vault, pid: &str) -> DbResult<Plan> {
    let cfg = get(v, pid)?;
    let root = projects::root_of(v, pid)?;
    let dest_s = cfg.str_of("dest").unwrap_or_default();
    if dest_s.is_empty() {
        return Err("Choose where to deploy to first".into());
    }
    let dest = PathBuf::from(&dest_s);
    if !dest.is_absolute() {
        return Err("The deploy destination must be a full path, like D:\\Games\\MW2\\mods\\omen".into());
    }
    if dest.parent().is_none() || dest.parent() == Some(Path::new("")) || dest.components().count() <= 1 {
        return Err("Refusing to deploy into the root of a drive".into());
    }
    let src = cfg.str_of("source").unwrap_or_default();
    let src_root = if src.is_empty() { root.clone() } else { safe_join(&root, &src)? };
    if !src_root.is_dir() {
        return Err(format!("The folder to deploy doesn't exist: {}", src_root.display()));
    }
    let (d, r) = (norm(&dest), norm(&root));
    if d == r || d.starts_with(&format!("{r}/")) || r.starts_with(&format!("{d}/")) {
        return Err("The destination can't be the project folder, inside it, or contain it".into());
    }
    let ign = v.db.one("SELECT ignore_globs FROM projects WHERE id=?", p![pid])?.map(|r| r.s("ignore_globs")).unwrap_or_default();
    let rules = format!("{ign}\n{}", cfg.str_of("exclude").unwrap_or_default());
    let w = walk(&src_root, &Ignore::parse(&rules));
    let manifest = meta_get(v, &manifest_key(pid))?;
    let old: HashMap<String, String> = match manifest.get("files") {
        Some(Json::Obj(m)) => m.iter().filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string()))).collect(),
        _ => HashMap::new(),
    };
    let mut items = BTreeMap::new();
    for (rel, seen) in &w.files {
        let sp = safe_join(&src_root, rel)?;
        let dp = safe_join(&dest, rel)?;
        let src_sha = sha256_file(&sp).map_err(|e| format!("{rel}: {e}"))?;
        let action = match std::fs::metadata(&dp) {
            Err(_) => "copy",
            Ok(m) if m.is_dir() => "conflict",
            Ok(m) => {
                let same_size = m.len() as i64 == seen.size;
                let dest_sha = if same_size || old.contains_key(rel) { sha256_file(&dp).ok() } else { None };
                if same_size && dest_sha.as_deref() == Some(src_sha.as_str()) {
                    "same"
                } else {
                    // changed in the destination since we put it there, or not ours at all
                    match (old.get(rel), &dest_sha) {
                        (Some(prev), Some(now)) if prev == now => "update",
                        _ => "conflict",
                    }
                }
            }
        };
        items.insert(rel.clone(), (action, seen.size, src_sha));
    }
    let mut stale = Vec::new();
    if cfg.get("clean").and_then(|x| x.as_bool()).unwrap_or(true) {
        for (rel, prev) in &old {
            if items.contains_key(rel) {
                continue;
            }
            let Ok(dp) = safe_join(&dest, rel) else { continue };
            if !dp.exists() {
                continue;
            }
            let unchanged = sha256_file(&dp).ok().as_deref() == Some(prev.as_str());
            stale.push((rel.clone(), unchanged));
        }
        stale.sort();
    }
    Ok(Plan { src_root, dest, items, stale })
}

fn plan_json(pl: &Plan) -> Json {
    let mut counts: HashMap<&str, i64> = HashMap::new();
    let mut list = Vec::new();
    for (rel, (a, size, _)) in &pl.items {
        *counts.entry(a).or_default() += 1;
        if *a != "same" {
            list.push(Json::obj().with("path", rel.as_str()).with("action", *a).with("size", *size));
        }
    }
    for (rel, ok) in &pl.stale {
        list.push(Json::obj().with("path", rel.as_str()).with("action", if *ok { "remove" } else { "keep" }));
    }
    Json::obj()
        .with("dest", pl.dest.to_string_lossy().to_string())
        .with("source", pl.src_root.to_string_lossy().to_string())
        .with("copy", counts.get("copy").copied().unwrap_or(0))
        .with("update", counts.get("update").copied().unwrap_or(0))
        .with("same", counts.get("same").copied().unwrap_or(0))
        .with("conflict", counts.get("conflict").copied().unwrap_or(0))
        .with("remove", pl.stale.iter().filter(|x| x.1).count())
        .with("keep", pl.stale.iter().filter(|x| !x.1).count())
        .with("changes", Json::Arr(list))
}

/// What a deploy would do, without touching anything.
pub fn preview(v: &Vault, pid: &str) -> DbResult<Json> {
    Ok(plan_json(&plan(v, pid)?))
}

fn copy_verified(src: &Path, dest: &Path, sha: &str) -> Result<(), String> {
    let dir = dest.parent().ok_or("bad destination")?;
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let tmp = dir.join(format!(".pv-deploy-{}", &new_id()[..8]));
    std::fs::copy(src, &tmp).map_err(|e| format!("{}: {e}", src.display()))?;
    if sha256_file(&tmp).ok().as_deref() != Some(sha) {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("{} changed while it was being copied; try again", src.display()));
    }
    if dest.exists() {
        // Windows rename doesn't replace; remove first (the verified copy is ready)
        std::fs::remove_file(dest).map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            format!("{} is in use (is the game running?): {e}", dest.display())
        })?;
    }
    std::fs::rename(&tmp, dest).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("{}: {e}", dest.display())
    })
}

/// Copies changed files. Conflicts block the deploy unless `force`.
pub fn run(v: &Vault, pid: &str, force: bool) -> DbResult<Json> {
    let pl = plan(v, pid)?;
    let conflicts = pl.items.values().filter(|x| x.0 == "conflict").count();
    if conflicts > 0 && !force {
        return Ok(plan_json(&pl).with("blocked", true));
    }
    let mut manifest: BTreeMap<String, Json> = match meta_get(v, &manifest_key(pid))?.get("files") {
        Some(Json::Obj(m)) => m.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        _ => BTreeMap::new(),
    };
    let (mut copied, mut removed, mut errors) = (0, 0, Vec::new());
    for (rel, (action, _, sha)) in &pl.items {
        if *action == "same" {
            manifest.insert(rel.clone(), Json::from(sha.as_str()));
            continue;
        }
        let r = safe_join(&pl.src_root, rel).and_then(|s| safe_join(&pl.dest, rel).map(|d| (s, d))).and_then(|(s, d)| copy_verified(&s, &d, sha));
        match r {
            Ok(()) => {
                copied += 1;
                manifest.insert(rel.clone(), Json::from(sha.as_str()));
            }
            Err(e) => errors.push(Json::from(e)),
        }
    }
    for (rel, unchanged) in &pl.stale {
        if !unchanged {
            manifest.remove(rel); // changed in the game folder: leave it, and stop tracking it
            continue;
        }
        if let Ok(dp) = safe_join(&pl.dest, rel) {
            match std::fs::remove_file(&dp) {
                Ok(()) => {
                    removed += 1;
                    manifest.remove(rel);
                    // tidy folders this left empty (never the destination itself)
                    let mut d = dp.parent().map(Path::to_path_buf);
                    while let Some(dir) = d {
                        if dir == pl.dest || std::fs::remove_dir(&dir).is_err() {
                            break;
                        }
                        d = dir.parent().map(Path::to_path_buf);
                    }
                }
                Err(e) => errors.push(Json::from(format!("{rel}: {e}"))),
            }
        }
    }
    meta_set(v, &manifest_key(pid), &Json::obj().with("files", Json::Obj(manifest.into_iter().collect())))?;
    let at = now_ms();
    let unchanged = pl.items.values().filter(|x| x.0 == "same").count();
    let summary = if copied + removed == 0 && errors.is_empty() {
        format!("Deployed to {}: already up to date", pl.dest.display())
    } else {
        format!("Deployed to {}: {copied} copied{}{}", pl.dest.display(), if removed > 0 { format!(", {removed} removed") } else { String::new() }, if errors.is_empty() { String::new() } else { format!(", {} failed", errors.len()) })
    };
    timeline::add(v, pid, "deploy", &summary, &errors.iter().filter_map(|e| e.as_str()).take(5).collect::<Vec<_>>().join("\n"), None, false)?;
    let last = Json::obj().with("at", at).with("copied", copied).with("removed", removed).with("unchanged", unchanged).with("errors", errors.len());
    let mut cfg = meta_get(v, &key(pid))?;
    cfg.set("last", last);
    meta_set(v, &key(pid), &cfg)?;
    Ok(Json::obj().with("copied", copied).with("removed", removed).with("unchanged", unchanged).with("errors", Json::Arr(errors)).with("dest", pl.dest.to_string_lossy().to_string()))
}

/// Splits a command line into arguments ("quoted parts" stay together).
pub fn split_args(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut any = false;
    for c in s.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                any = true;
            }
            c if c.is_whitespace() && !quoted => {
                if any || !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
                any = false;
            }
            c => cur.push(c),
        }
    }
    if any || !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Starts the configured program (the game) without waiting for it.
pub fn launch(v: &Vault, pid: &str) -> DbResult<Json> {
    let cfg = get(v, pid)?;
    let exe = cfg.str_of("exe").unwrap_or_default();
    if exe.is_empty() {
        return Err("No program to launch is set for this project".into());
    }
    let exe = PathBuf::from(exe);
    if !exe.is_file() {
        return Err(format!("{} doesn't exist", exe.display()));
    }
    let cwd = match cfg.str_of("cwd").filter(|s| !s.is_empty()) {
        Some(c) => PathBuf::from(c),
        None => exe.parent().map(Path::to_path_buf).unwrap_or_default(),
    };
    let mut cmd = std::process::Command::new(&exe);
    cmd.args(split_args(&cfg.str_of("args").unwrap_or_default())).current_dir(&cwd);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0000_0008 | 0x0000_0200); // DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP
    }
    cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    let child = cmd.spawn().map_err(|e| format!("Couldn't start {}: {e}", exe.display()))?;
    Ok(Json::obj().with("pid", child.id() as i64))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;

    #[test]
    fn deploys_only_changes_and_never_clobbers_edits_in_the_game_folder() {
        let (t, v) = vault();
        let src = t.path().join("OMEN");
        std::fs::create_dir_all(src.join("scripts/mp")).unwrap();
        std::fs::create_dir_all(src.join("docs")).unwrap();
        std::fs::write(src.join("scripts/mp/a.gsc"), "a1").unwrap();
        std::fs::write(src.join("scripts/mp/b.gsc"), "b1").unwrap();
        std::fs::write(src.join("docs/readme.md"), "doc").unwrap();
        let pid = projects::create(&v, projects::NewProject { name: "OMEN".into(), description: "".into(), category: "game".into(), status: "".into(), link_path: Some(src.clone()), module: Json::obj() }).unwrap().str_of("id").unwrap();
        let game = t.path().join("MW2/mods/omen");
        // guards
        set(&v, &pid, &Json::obj().with("dest", src.join("out").to_string_lossy().to_string())).unwrap();
        assert!(preview(&v, &pid).unwrap_err().contains("inside it"));
        set(&v, &pid, &Json::obj().with("dest", game.to_string_lossy().to_string()).with("exclude", "docs/")).unwrap();
        // first deploy copies everything except excluded files
        let p = preview(&v, &pid).unwrap();
        assert_eq!(p.get("copy").unwrap().as_i64(), Some(2));
        let r = run(&v, &pid, false).unwrap();
        assert_eq!(r.get("copied").unwrap().as_i64(), Some(2));
        assert_eq!(std::fs::read_to_string(game.join("scripts/mp/a.gsc")).unwrap(), "a1");
        assert!(!game.join("docs").exists());
        // nothing changed: nothing copied
        assert_eq!(run(&v, &pid, false).unwrap().get("copied").unwrap().as_i64(), Some(0));
        // project change -> update; edit made in the game folder -> conflict, not overwritten
        std::fs::write(src.join("scripts/mp/a.gsc"), "a2").unwrap();
        std::fs::write(src.join("scripts/mp/b.gsc"), "b2").unwrap();
        std::fs::write(game.join("scripts/mp/b.gsc"), "edited in game folder").unwrap();
        let r = run(&v, &pid, false).unwrap();
        assert_eq!(r.get("blocked").unwrap().as_bool(), Some(true));
        assert_eq!(r.get("update").unwrap().as_i64(), Some(1));
        assert_eq!(r.get("conflict").unwrap().as_i64(), Some(1));
        assert_eq!(std::fs::read_to_string(game.join("scripts/mp/a.gsc")).unwrap(), "a1", "blocked deploy changes nothing");
        run(&v, &pid, true).unwrap();
        assert_eq!(std::fs::read_to_string(game.join("scripts/mp/b.gsc")).unwrap(), "b2");
        // a file deleted from the project is removed from the game folder (it was ours and unchanged)
        std::fs::remove_file(src.join("scripts/mp/a.gsc")).unwrap();
        std::fs::write(game.join("other_mod_file.txt"), "not ours").unwrap();
        let r = run(&v, &pid, false).unwrap();
        assert_eq!(r.get("removed").unwrap().as_i64(), Some(1));
        assert!(!game.join("scripts/mp/a.gsc").exists());
        assert!(game.join("other_mod_file.txt").exists(), "files we never deployed are left alone");
        let tl = crate::timeline::list(&v, &pid, 1, None).unwrap();
        assert!(tl.as_arr().unwrap()[0].str_of("summary").unwrap().contains("1 removed"));
    }

    #[test]
    fn splits_launch_arguments() {
        assert_eq!(split_args(r#"+set fs_game "mods/my mod" -windowed"#), vec!["+set", "fs_game", "mods/my mod", "-windowed"]);
        assert_eq!(split_args(r#""" x"#), vec!["", "x"]);
        assert!(split_args("  ").is_empty());
    }
}

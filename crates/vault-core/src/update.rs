//! "Update the project from this": drop a newer copy of the project (a
//! downloaded folder or .zip, or some of its files) onto the open project and
//! the files it changed replace the project's, files it adds are added, and
//! nothing else is touched.
//!
//! - The dropped files are matched to the project's by path. A download often
//!   wraps the project in a folder or two ("Downtime-v3/Downtime/…"), so the
//!   wrapping that matches the most project files is stripped. A loose file
//!   dropped on its own that matches no path is matched by name when exactly
//!   one project file has that name.
//! - Before a file is replaced, its current content is stored (any kind of
//!   file, images included), so the update can be undone: it lands in the
//!   Timeline as one "Updated from …" batch with "Undo these changes".
//! - Project files that aren't in the drop are left alone unless asked to
//!   remove them (to the Recycle Bin, with a copy kept), since a download is
//!   often only part of a project.
//! - Each file is written next to its target, verified, then renamed over
//!   it, so a failure never leaves a half-written file.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use crate::db::DbResult;
use crate::files::{self, Ignore};
use crate::json::Json;
use crate::p;
use crate::util::{new_id, now_ms, safe_join, sha256_bytes, sha256_file};
use crate::{projects, versions, Vault};

/// Largest single file read from a .zip (bigger entries are reported, not read).
const ZIP_LIMIT: usize = 512 * 1024 * 1024;

#[derive(Clone)]
enum Src {
    Disk(PathBuf),
    Zip(PathBuf, String),
}

impl Src {
    fn read(&self) -> Result<Vec<u8>, String> {
        match self {
            Src::Disk(p) => std::fs::read(p).map_err(|e| format!("{}: {e}", p.display())),
            Src::Zip(z, name) => {
                let mut a = crate::zip::Archive::open(z)?;
                let e = a.entries.iter().find(|e| e.name == *name).cloned().ok_or(format!("{name} is no longer in the zip"))?;
                a.read(&e, ZIP_LIMIT)
            }
        }
    }
    fn sha(&self) -> Result<String, String> {
        match self {
            Src::Disk(p) => sha256_file(p).map_err(|e| format!("{}: {e}", p.display())),
            Src::Zip(..) => Ok(sha256_bytes(&self.read()?)),
        }
    }
}

/// Names that are never project content.
fn junk(rel: &str) -> bool {
    let l = rel.to_lowercase();
    let name = l.rsplit('/').next().unwrap_or(&l);
    l.starts_with("__macosx/") || l.contains("/__macosx/") || matches!(name, ".ds_store" | "thumbs.db" | "desktop.ini") || name.starts_with("._")
}

/// Everything dropped, as relative paths in one virtual folder: a dropped
/// folder or zip keeps its own name as the first path part; a loose file is
/// just its name (and is marked loose, for matching by name).
fn gather(sources: &[PathBuf]) -> DbResult<(BTreeMap<String, Src>, Vec<String>)> {
    let mut out = BTreeMap::new();
    let mut loose = Vec::new();
    for s in sources {
        let name = s.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        if s.is_dir() {
            let w = files::walk(s, &Ignore::parse(""));
            for rel in w.files.keys() {
                out.insert(format!("{name}/{rel}"), Src::Disk(s.join(rel)));
            }
        } else if name.to_lowercase().ends_with(".zip") {
            let a = crate::zip::Archive::open(s).map_err(|e| format!("{name}: {e}"))?;
            let stem = &name[..name.len() - 4];
            for e in &a.entries {
                let n = e.name.replace('\\', "/");
                if n.ends_with('/') || n.split('/').any(|p| p == "..") {
                    continue;
                }
                out.insert(format!("{stem}/{}", n.trim_start_matches('/')), Src::Zip(s.clone(), e.name.clone()));
            }
        } else if s.is_file() {
            out.insert(name.clone(), Src::Disk(s.clone()));
            loose.push(name);
        } else {
            return Err(format!("{} wasn't found", s.display()).into());
        }
    }
    out.retain(|k, _| !junk(k));
    Ok((out, loose))
}

struct Plan {
    root: PathBuf,
    /// what to strip from the dropped paths ("Downtime-v3/" etc.)
    strip: String,
    /// project path → (source, status, current sha, new sha)
    items: Vec<(String, Src, &'static str, Option<String>, String)>,
    /// project files (present) that aren't in the drop
    not_in_drop: Vec<String>,
    /// whether the drop is a whole copy of the project (so "not in the drop" means something)
    whole: bool,
    ignored: usize,
}

fn make_plan(v: &Vault, project_id: &str, sources: &[PathBuf]) -> DbResult<Plan> {
    if sources.is_empty() {
        return Err("Nothing was dropped".into());
    }
    let root = projects::root_of(v, project_id)?;
    if !root.is_dir() {
        return Err("The project folder isn't reachable".into());
    }
    let ig = Ignore::parse(&v.db.one("SELECT ignore_globs FROM projects WHERE id=?", p![project_id])?.map(|r| r.s("ignore_globs")).unwrap_or_default());
    let rows = v.db.query("SELECT id, rel_path, sha256 FROM files WHERE project_id=? AND state='present'", p![project_id])?;
    let by_path: HashMap<String, (String, Option<String>)> = rows.iter().map(|r| (r.s("rel_path").to_lowercase(), (r.s("rel_path"), r.os("sha256")))).collect();
    let mut by_name: HashMap<String, Vec<String>> = HashMap::new();
    for r in &rows {
        let rel = r.s("rel_path");
        by_name.entry(rel.rsplit('/').next().unwrap_or(&rel).to_lowercase()).or_default().push(rel.clone());
    }
    let (dropped, loose) = gather(sources)?;
    if dropped.is_empty() {
        return Err("The drop has no files in it".into());
    }
    // which wrapping folders to strip: try each folder prefix (up to 3 deep) and keep the
    // one under which the most dropped files are already in the project
    let mut prefixes: Vec<String> = vec![String::new()];
    for k in dropped.keys() {
        let parts: Vec<&str> = k.split('/').collect();
        for d in 1..parts.len().min(4) {
            let pre = format!("{}/", parts[..d].join("/"));
            if !prefixes.contains(&pre) {
                prefixes.push(pre);
            }
        }
    }
    let score = |pre: &str| dropped.keys().filter(|k| k.starts_with(pre) && by_path.contains_key(&k[pre.len()..].to_lowercase())).count();
    let mut strip = String::new();
    let mut best = score("");
    for pre in &prefixes {
        let s = score(pre);
        if s > best || (s == best && s > 0 && pre.len() > strip.len()) {
            best = s;
            strip = pre.clone();
        }
    }
    // with no matches at all, a single dropped folder (or zip) still stands for the
    // project: strip down to a wrapper folder named like the project's own folder
    // ("Downtime-v3/Downtime/…"), or else just the dropped folder itself
    let single = sources.len() == 1 && (sources[0].is_dir() || sources[0].to_string_lossy().to_lowercase().ends_with(".zip"));
    if best == 0 && single {
        let root_name = root.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
        let named = prefixes.iter().filter(|p| !p.is_empty() && p.trim_end_matches('/').rsplit('/').next().map(|n| n.to_lowercase()) == Some(root_name.clone())).min_by_key(|p| p.len()).cloned();
        strip = named.unwrap_or_else(|| {
            let first = dropped.keys().next().cloned().unwrap_or_default();
            first.split_once('/').map(|(a, _)| format!("{a}/")).unwrap_or_default()
        });
    }
    let mut items = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut ignored = 0;
    for (k, src) in &dropped {
        let mut rel = if k.starts_with(&strip) { k[strip.len()..].to_string() } else { k.clone() };
        if rel.is_empty() {
            continue;
        }
        let mut cur = by_path.get(&rel.to_lowercase()).cloned();
        if cur.is_none() && loose.contains(k) {
            if let Some([only]) = by_name.get(&rel.to_lowercase()).map(|v| v.as_slice()) {
                rel = only.clone();
                cur = by_path.get(&rel.to_lowercase()).cloned();
            }
        }
        if let Some((actual, _)) = &cur {
            rel = actual.clone(); // the project's spelling (case)
        }
        if ig.ignores_path(&rel) {
            ignored += 1;
            continue;
        }
        if safe_join(&root, &rel).is_err() || !seen.insert(rel.to_lowercase()) {
            continue;
        }
        let new_sha = src.sha().map_err(|e| e.to_string())?;
        let (status, old) = match &cur {
            Some((_, Some(old))) if *old == new_sha => ("same", Some(old.clone())),
            Some((_, old)) => ("changed", old.clone()),
            None if safe_join(&root, &rel)?.exists() => ("changed", None), // on disk but not indexed yet
            None => ("new", None),
        };
        items.push((rel, src.clone(), status, old, new_sha));
    }
    items.sort_by(|a, b| a.0.cmp(&b.0));
    let whole = sources.len() == 1 && (sources[0].is_dir() || sources[0].to_string_lossy().to_lowercase().ends_with(".zip"));
    let not_in_drop = if whole {
        let mut v: Vec<String> = rows.iter().map(|r| r.s("rel_path")).filter(|r| !seen.contains(&r.to_lowercase())).collect();
        v.sort();
        v
    } else {
        Vec::new()
    };
    Ok(Plan { root, strip, items, not_in_drop, whole, ignored })
}

/// What dropping these would do to the project (nothing is changed).
pub fn preview(v: &Vault, project_id: &str, sources: &[PathBuf]) -> DbResult<Json> {
    let pl = make_plan(v, project_id, sources)?;
    let count = |s: &str| pl.items.iter().filter(|i| i.2 == s).count() as i64;
    Ok(Json::obj()
        .with("strip", pl.strip.trim_end_matches('/'))
        .with("changed", count("changed"))
        .with("new", count("new"))
        .with("same", count("same"))
        .with("ignored", pl.ignored as i64)
        .with("whole", pl.whole)
        .with("items", Json::Arr(pl.items.iter().map(|(rel, _, st, _, _)| Json::obj().with("path", rel.as_str()).with("status", *st)).collect()))
        .with("notInDrop", Json::Arr(pl.not_in_drop.iter().map(|s| Json::from(s.as_str())).collect())))
}

/// Writes `bytes` to `target` via a verified temp file next to it.
fn write_verified(target: &Path, bytes: &[u8], sha: &str) -> Result<(), String> {
    let dir = target.parent().ok_or("bad path")?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let tmp = dir.join(format!(".pv-tmp-{}", &new_id()[..8]));
    std::fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
    if sha256_file(&tmp).ok().as_deref() != Some(sha) {
        let _ = std::fs::remove_file(&tmp);
        return Err("the written copy didn't verify; nothing was replaced".into());
    }
    std::fs::rename(&tmp, target).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("could not replace it (is it open in another program?): {e}")
    })
}

/// Update the project: replace changed files, add new ones, and (if asked)
/// remove project files that aren't in the drop. One "Updated from …"
/// Timeline batch, undoable.
pub fn apply(v: &Vault, project_id: &str, sources: &[PathBuf], remove_missing: bool) -> DbResult<Json> {
    let pl = make_plan(v, project_id, sources)?;
    let label = match sources {
        [one] => one.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
        many => format!("{} dropped items", many.len()),
    };
    let start = now_ms();
    let (mut updated, mut added, mut errors) = (Vec::new(), Vec::new(), Vec::new());
    let mut written_ids: Vec<String> = Vec::new();
    for (rel, src, status, _old, new_sha) in &pl.items {
        if *status == "same" {
            continue;
        }
        let res = (|| -> Result<(), String> {
            let target = safe_join(&pl.root, rel)?;
            let bytes = src.read()?;
            if sha256_bytes(&bytes) != *new_sha {
                return Err("the dropped file changed while updating; skipped".into());
            }
            if target.exists() {
                // keep what's being replaced, whatever kind of file it is
                let cur = sha256_file(&target).map_err(|e| e.to_string())?;
                if let Some(f) = v.db.one("SELECT * FROM files WHERE project_id=? AND rel_path=?", p![project_id, rel.as_str()]).map_err(|e| e.to_string())? {
                    versions::ensure_stored(v, &f, &target, &cur, "before-update").map_err(|e| e.to_string())?;
                    written_ids.push(f.s("id"));
                } else {
                    return Err("it exists on disk but isn't tracked yet; rescan first".into());
                }
            }
            write_verified(&target, &bytes, new_sha)
        })();
        match res {
            Ok(()) => if *status == "new" { added.push(Json::from(rel.as_str())) } else { updated.push(Json::from(rel.as_str())) },
            Err(e) => errors.push(Json::obj().with("path", rel.as_str()).with("error", e)),
        }
    }
    let mut removed = Vec::new();
    if remove_missing && pl.whole && !pl.not_in_drop.is_empty() {
        let ids: Vec<String> = pl.not_in_drop.iter().filter_map(|r| v.db.one("SELECT id FROM files WHERE project_id=? AND rel_path=? AND state='present'", p![project_id, r.as_str()]).ok().flatten().map(|x| x.s("id"))).collect();
        let r = files::delete(v, &ids)?;
        removed = r.get("deleted").and_then(|x| x.as_arr()).cloned().unwrap_or_default();
        for e in r.get("errors").and_then(|x| x.as_arr()).cloned().unwrap_or_default() {
            errors.push(Json::obj().with("error", e));
        }
    }
    // one scan records the batch (with before/after for each changed file, so
    // "Undo these changes" works); then it's labelled as this update
    files::scan(v, project_id)?;
    let added_ids: Vec<String> = added.iter().filter_map(|r| r.as_str()).filter_map(|r| v.db.one("SELECT id FROM files WHERE project_id=? AND rel_path=?", p![project_id, r]).ok().flatten().map(|x| x.s("id"))).collect();
    for id in written_ids.iter().chain(added_ids.iter()) {
        v.db.exec("UPDATE files SET changed_since_review=0 WHERE id=?", p![id.as_str()])?;
        v.db.exec("UPDATE file_events SET detail=? WHERE file_id=? AND at>=? AND kind IN ('modified','added')", p![format!("Updated from {label}"), id.as_str(), start])?;
    }
    let mut entry_id = String::new();
    for r in v.db.query("SELECT id, summary FROM timeline WHERE project_id=? AND kind='files' AND at>=? AND summary LIKE 'File changes detected:%' ORDER BY at", p![project_id, start])? {
        let rest = r.s("summary").trim_start_matches("File changes detected:").trim().to_string();
        v.db.exec("UPDATE timeline SET summary=? WHERE id=?", p![format!("Updated from {label}: {rest}"), r.s("id")])?;
        entry_id = r.s("id");
    }
    Ok(Json::obj()
        .with("updated", Json::Arr(updated))
        .with("added", Json::Arr(added))
        .with("removed", Json::Arr(removed))
        .with("unchanged", pl.items.iter().filter(|i| i.2 == "same").count() as i64)
        .with("entryId", entry_id)
        .with("errors", Json::Arr(errors)))
}

#[cfg(test)]
mod tests {
    use crate::json::Json;
    use crate::p;
    use std::fs;

    /// User request: drop a downloaded newer copy of the project on it and
    /// have the edited files replaced, instead of copy/pasting in Explorer.
    #[test]
    fn update_from_a_wrapped_download_folder_then_undo_it() {
        let (t, v) = crate::testutil::vault();
        let d = t.0.join("Downtime");
        fs::create_dir_all(d.join("Games")).unwrap();
        fs::create_dir_all(d.join("Assets")).unwrap();
        fs::write(d.join("Downtime.toc"), "## Title: Downtime\n").unwrap();
        fs::write(d.join("Games/Slots.lua"), "slots v1\n").unwrap();
        fs::write(d.join("Assets/Slots.tga"), [0u8, 1, 2, 3]).unwrap();
        fs::write(d.join("Notes.md"), "mine, not in the download\n").unwrap();
        let pid = crate::api::dispatch(&v, "projects.create", &Json::obj().with("name", "Downtime").with("linkPath", d.to_string_lossy().to_string())).unwrap().str_of("id").unwrap();
        crate::files::scan(&v, &pid).unwrap();

        // the download: wrapped twice, one same, two changed (one binary), one new
        let dl = t.0.join("Downloads/Downtime-v3/Downtime");
        fs::create_dir_all(dl.join("Games")).unwrap();
        fs::create_dir_all(dl.join("Assets")).unwrap();
        fs::write(dl.join("Downtime.toc"), "## Title: Downtime\n").unwrap();
        fs::write(dl.join("Games/Slots.lua"), "slots v2\n").unwrap();
        fs::write(dl.join("Assets/Slots.tga"), [9u8, 9, 9]).unwrap();
        fs::write(dl.join("Games/Roulette.lua"), "roulette\n").unwrap();
        let drop = t.0.join("Downloads/Downtime-v3");
        let paths = Json::Arr(vec![Json::from(drop.to_string_lossy().to_string())]);

        let pv = crate::api::dispatch(&v, "files.updatePreview", &Json::obj().with("projectId", pid.as_str()).with("paths", paths.clone())).unwrap();
        assert_eq!(pv.str_of("strip").as_deref(), Some("Downtime-v3/Downtime"));
        assert_eq!((pv.get("changed").unwrap().as_i64(), pv.get("new").unwrap().as_i64(), pv.get("same").unwrap().as_i64()), (Some(2), Some(1), Some(1)));
        assert_eq!(pv.get("notInDrop").unwrap().to_string(), r#"["Notes.md"]"#);
        assert_eq!(fs::read_to_string(d.join("Games/Slots.lua")).unwrap(), "slots v1\n", "preview changes nothing");

        let r = crate::api::dispatch(&v, "files.update", &Json::obj().with("projectId", pid.as_str()).with("paths", paths)).unwrap();
        assert_eq!(r.get("errors").unwrap().as_arr().unwrap().len(), 0, "{}", r.to_string());
        assert_eq!(fs::read_to_string(d.join("Games/Slots.lua")).unwrap(), "slots v2\n");
        assert_eq!(fs::read(d.join("Assets/Slots.tga")).unwrap(), [9, 9, 9]);
        assert_eq!(fs::read_to_string(d.join("Games/Roulette.lua")).unwrap(), "roulette\n");
        assert!(d.join("Notes.md").exists(), "not in the download: left alone");
        // labelled as the update, not flagged as outside changes
        let tl = crate::timeline::list(&v, &pid, 5, None).unwrap();
        let e = tl.as_arr().unwrap().iter().find(|e| e.str_of("summary").unwrap_or_default().starts_with("Updated from Downtime-v3:")).cloned().expect("one Updated from entry");
        assert_eq!(v.db.scalar_i64("SELECT COUNT(*) FROM files WHERE project_id=? AND changed_since_review=1", p![pid.as_str()]).unwrap(), 0);
        // undo: both replaced files come back, the image included (its copy was kept first)
        let plan = crate::undo::plan(&v, &e.str_of("id").unwrap()).unwrap();
        let act = |p: &str| plan.get("items").unwrap().as_arr().unwrap().iter().find(|i| i.str_of("path").as_deref() == Some(p)).unwrap().str_of("action").unwrap();
        assert_eq!((act("Games/Slots.lua").as_str(), act("Assets/Slots.tga").as_str(), act("Games/Roulette.lua").as_str()), ("restore", "restore", "remove"));
        crate::undo::apply(&v, &e.str_of("id").unwrap()).unwrap();
        assert_eq!(fs::read_to_string(d.join("Games/Slots.lua")).unwrap(), "slots v1\n");
        assert_eq!(fs::read(d.join("Assets/Slots.tga")).unwrap(), [0, 1, 2, 3]);
        assert!(!d.join("Games/Roulette.lua").exists());

        // the same download as a .zip, and removing what isn't in it
        let z = t.0.join("Downloads/Downtime-v3.zip");
        crate::zip::zip_dir(&drop, &z).unwrap();
        let zp = Json::Arr(vec![Json::from(z.to_string_lossy().to_string())]);
        let pv = crate::api::dispatch(&v, "files.updatePreview", &Json::obj().with("projectId", pid.as_str()).with("paths", zp.clone())).unwrap();
        assert_eq!((pv.get("changed").unwrap().as_i64(), pv.get("new").unwrap().as_i64()), (Some(2), Some(1)), "{}", pv.to_string());
        crate::api::dispatch(&v, "files.update", &Json::obj().with("projectId", pid.as_str()).with("paths", zp).with("removeMissing", true)).unwrap();
        assert_eq!(fs::read_to_string(d.join("Games/Slots.lua")).unwrap(), "slots v2\n");
        assert!(!d.join("Notes.md").exists(), "removed when asked (to the Recycle Bin, copy kept)");
        assert_eq!(v.db.one("SELECT state FROM files WHERE rel_path='Notes.md'", p![]).unwrap().unwrap().s("state"), "deleted");

        // one loose file, matched by name
        let loose = t.0.join("Downloads/Slots.lua");
        fs::write(&loose, "slots v4\n").unwrap();
        let lp = Json::Arr(vec![Json::from(loose.to_string_lossy().to_string())]);
        let pv = crate::api::dispatch(&v, "files.updatePreview", &Json::obj().with("projectId", pid.as_str()).with("paths", lp.clone())).unwrap();
        assert_eq!(pv.get("items").unwrap().as_arr().unwrap()[0].str_of("path").as_deref(), Some("Games/Slots.lua"));
        crate::api::dispatch(&v, "files.update", &Json::obj().with("projectId", pid.as_str()).with("paths", lp)).unwrap();
        assert_eq!(fs::read_to_string(d.join("Games/Slots.lua")).unwrap(), "slots v4\n");
    }

    /// A download with only new files: the wrapper named like the project's
    /// folder is stripped, so they land at the right place.
    #[test]
    fn only_new_files_still_land_under_the_project_root() {
        let (t, v) = crate::testutil::vault();
        let d = t.0.join("Downtime");
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join("Downtime.toc"), "x").unwrap();
        let pid = crate::api::dispatch(&v, "projects.create", &Json::obj().with("name", "Downtime").with("linkPath", d.to_string_lossy().to_string())).unwrap().str_of("id").unwrap();
        crate::files::scan(&v, &pid).unwrap();
        let dl = t.0.join("dl/Downtime-v9/Downtime/Games");
        fs::create_dir_all(&dl).unwrap();
        fs::write(dl.join("Keno.lua"), "keno").unwrap();
        let paths = Json::Arr(vec![Json::from(t.0.join("dl/Downtime-v9").to_string_lossy().to_string())]);
        let pv = crate::api::dispatch(&v, "files.updatePreview", &Json::obj().with("projectId", pid.as_str()).with("paths", paths)).unwrap();
        assert_eq!(pv.get("items").unwrap().as_arr().unwrap()[0].str_of("path").as_deref(), Some("Games/Keno.lua"), "{}", pv.to_string());
    }

}

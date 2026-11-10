//! Backup drive: a complete, incremental backup on another drive.
//!
//! Unlike the export-style backups in `backup.rs`, a backup drive holds
//! everything needed to get back to where you were if this PC's drive dies:
//! the current project files, every stored version, snapshot and checkpoint
//! (the `objects/` store), and the database.
//!
//! Layout, inside `<chosen folder>/Mothball Backup/`:
//!   objects/ab/cdef…            file contents, named by SHA-256 (shared by
//!                               every run, so unchanged files are copied once)
//!   runs/<run id>/vault.db      the database at that point
//!   runs/<run id>/manifest.json every project file at that point → object
//!   runs/<run id>/objects.txt   the history objects that database refers to
//!   runs/<run id>/summary.json  small summary for listing runs
//!   store/history, store/snapshots, pictures/   portable metadata
//!
//! A run is written to `runs/.<id>.partial/` and only renamed into place once
//! every object it refers to has been checked, so a drive pulled out halfway
//! leaves an incomplete folder that is never mistaken for a backup.
//!
//! Copying happens without holding the vault: the vault is only used briefly
//! to plan a run (database copy + file list) and to record the result, so the
//! app stays usable while gigabytes are copied.

use crate::db::{Db, DbResult};
use crate::json::Json;
use crate::util::{atomic_write, iso_utc, new_id, now_ms, safe_join, sha256_file, Sha256};
use crate::{p, projects, schema, versions, Vault};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub const FOLDER: &str = "Mothball Backup";
const FORMAT: i64 = 1;
const HOUR: i64 = 3_600_000;
const DAY: i64 = 24 * HOUR;

/// Gives a closure access to the vault (under the app's lock).
pub type With<'a> = &'a dyn Fn(&mut dyn FnMut(&Vault));
/// Progress: what is happening, items done, items total, bytes copied.
pub type Progress<'a> = &'a dyn Fn(&str, i64, i64, i64);

fn with<T>(w: With, f: impl FnOnce(&Vault) -> DbResult<T>) -> DbResult<T> {
    let mut f = Some(f);
    let mut out = None;
    w(&mut |v| {
        if let Some(f) = f.take() {
            out = Some(f(v));
        }
    });
    out.unwrap_or_else(|| Err("The vault isn't open".into()))
}

// ---------------------------------------------------------------- settings

fn meta_json(v: &Vault, key: &str) -> Json {
    let s = v.db.one("SELECT value FROM meta WHERE key=?", p![key]).ok().flatten().map(|r| r.s("value")).unwrap_or_default();
    Json::parse(&s).unwrap_or(Json::obj())
}
fn meta_put(v: &Vault, key: &str, j: &Json) -> DbResult<()> {
    v.db.exec("INSERT INTO meta(key,value) VALUES (?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value", p![key, j.to_string()])?;
    Ok(())
}

fn vault_id(v: &Vault) -> DbResult<String> {
    if let Some(r) = v.db.one("SELECT value FROM meta WHERE key='vault_id'", p![])? {
        if !r.s("value").is_empty() {
            return Ok(r.s("value"));
        }
    }
    let id = new_id();
    v.db.exec("INSERT INTO meta(key,value) VALUES ('vault_id',?) ON CONFLICT(key) DO UPDATE SET value=excluded.value", p![id.clone()])?;
    Ok(id)
}

pub fn settings(v: &Vault) -> DbResult<Json> {
    let j = meta_json(v, "backup_drive");
    Ok(Json::obj()
        .with("enabled", j.get("enabled").and_then(|x| x.as_bool()).unwrap_or(false))
        .with("dest", j.str_of("dest").unwrap_or_default())
        .with("everyHours", j.get("everyHours").and_then(|x| x.as_f64()).unwrap_or(24.0))
        .with("keep", j.get("keep").and_then(|x| x.as_i64()).unwrap_or(30))
        .with("enabledAt", j.get("enabledAt").cloned().unwrap_or(Json::Null)))
}

pub fn set_settings(v: &Vault, s: &Json) -> DbResult<Json> {
    let mut cur = settings(v)?;
    let was_on = cur.get("enabled").and_then(|x| x.as_bool()).unwrap_or(false);
    for k in ["enabled", "dest", "everyHours", "keep"] {
        if let Some(x) = s.get(k) {
            cur.set(k, x.clone());
        }
    }
    let on = cur.get("enabled").and_then(|x| x.as_bool()).unwrap_or(false);
    let dest = cur.str_of("dest").unwrap_or_default();
    if on && (dest.trim().is_empty() || !Path::new(&dest).is_dir()) {
        return Err("Choose a folder on the backup drive (the drive has to be connected) before turning this on".into());
    }
    let h = cur.get("everyHours").and_then(|x| x.as_f64()).unwrap_or(24.0);
    if !(1.0..=720.0).contains(&h) {
        return Err("Back up between every hour and every 30 days".into());
    }
    let keep = cur.get("keep").and_then(|x| x.as_i64()).unwrap_or(30);
    if !(3..=1000).contains(&keep) {
        return Err("Keep between 3 and 1000 backups".into());
    }
    if on && !was_on {
        cur.set("enabledAt", now_ms());
    }
    meta_put(v, "backup_drive", &cur)?;
    settings(v)
}

/// The `Mothball Backup` folder for a chosen destination (which may itself
/// already be that folder).
pub fn root_of(dest: &Path) -> PathBuf {
    if dest.file_name().map(|n| n == FOLDER).unwrap_or(false) {
        dest.to_path_buf()
    } else {
        dest.join(FOLDER)
    }
}

// ---------------------------------------------------------------- same drive?

#[cfg(unix)]
fn disk_id(p: &Path) -> Option<String> {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(p).ok().map(|m| m.dev().to_string())
}
#[cfg(windows)]
fn disk_id(p: &Path) -> Option<String> {
    use std::path::{Component, Prefix};
    let c = std::fs::canonicalize(p).ok()?;
    match c.components().next()? {
        Component::Prefix(px) => Some(match px.kind() {
            Prefix::Disk(d) | Prefix::VerbatimDisk(d) => format!("{}:", (d as char).to_ascii_uppercase()),
            Prefix::UNC(s, h) | Prefix::VerbatimUNC(s, h) => format!(r"\\{}\{}", s.to_string_lossy(), h.to_string_lossy()).to_lowercase(),
            _ => px.as_os_str().to_string_lossy().to_uppercase(),
        }),
        _ => None,
    }
}
#[cfg(not(any(unix, windows)))]
fn disk_id(_p: &Path) -> Option<String> {
    None
}

fn drive_label(p: &Path) -> String {
    #[cfg(windows)]
    {
        if let Some(d) = disk_id(p) {
            return d;
        }
    }
    projects::display_path(p)
}

/// What would be lost together with the backup if the drive holding `dest`
/// failed: Mothball's own data, its version store, and projects on it.
pub fn check(v: &Vault, dest: &Path) -> DbResult<Json> {
    let available = dest.is_dir();
    let mut same: Vec<Json> = Vec::new();
    if let Some(d) = available.then(|| disk_id(dest)).flatten() {
        let mut add = |what: String, p: &Path| {
            if disk_id(p).as_deref() == Some(d.as_str()) {
                same.push(Json::obj().with("what", what).with("path", projects::display_path(p)));
            }
        };
        add("Mothball's data".into(), &v.data_dir);
        let store = versions::storage_root(v);
        if store != v.data_dir {
            add("File history".into(), &store);
        }
        for r in v.db.query("SELECT name, root_path FROM projects ORDER BY name", p![])? {
            add(r.s("name"), Path::new(&r.s("root_path")));
        }
    }
    let root = root_of(dest);
    let runs = if root.join("runs").is_dir() { list_runs_in(&root).len() } else { 0 };
    Ok(Json::obj()
        .with("available", available)
        .with("drive", if available { drive_label(dest) } else { String::new() })
        .with("sameDisk", Json::Arr(same))
        .with("existingRuns", runs))
}

// ---------------------------------------------------------------- status

static PROGRESS: Mutex<Option<Json>> = Mutex::new(None);
static LAST_JOB: Mutex<Option<Json>> = Mutex::new(None);
static QUEUE: Mutex<Vec<Job>> = Mutex::new(Vec::new());

fn state(v: &Vault) -> Json {
    meta_json(v, "backup_drive_state")
}

pub fn status(v: &Vault) -> DbResult<Json> {
    let s = settings(v)?;
    let st = state(v);
    let dest = s.str_of("dest").unwrap_or_default();
    let enabled = s.get("enabled").and_then(|x| x.as_bool()).unwrap_or(false);
    let available = !dest.is_empty() && Path::new(&dest).is_dir();
    let every = (s.get("everyHours").and_then(|x| x.as_f64()).unwrap_or(24.0) * HOUR as f64) as i64;
    let last_ok = st.get("lastOk").and_then(|x| x.as_i64());
    let since = last_ok.or(s.get("enabledAt").and_then(|x| x.as_i64()));
    let overdue = enabled && since.map(|t| now_ms() - t > (3 * DAY).max(3 * every)).unwrap_or(false);
    let mut out = s.clone();
    out.set("available", available);
    out.set("lastRun", st.get("lastRun").cloned().unwrap_or(Json::Null));
    out.set("lastOk", st.get("lastOk").cloned().unwrap_or(Json::Null));
    out.set("lastResult", st.get("lastResult").cloned().unwrap_or(Json::Null));
    out.set("lastVerify", st.get("lastVerify").cloned().unwrap_or(Json::Null));
    out.set("nextRun", if enabled { Json::from(last_ok.map(|t| t + every).unwrap_or_else(now_ms)) } else { Json::Null });
    out.set("overdue", overdue);
    out.set("running", PROGRESS.lock().map(|p| p.clone()).unwrap_or(None).unwrap_or(Json::Null));
    out.set("queued", QUEUE.lock().map(|q| q.len()).unwrap_or(0));
    out.set("lastJob", LAST_JOB.lock().map(|p| p.clone()).unwrap_or(None).unwrap_or(Json::Null));
    Ok(out)
}

/// Whether a scheduled backup should start now.
fn due(v: &Vault, became_available: bool) -> bool {
    let Ok(s) = settings(v) else { return false };
    if !s.get("enabled").and_then(|x| x.as_bool()).unwrap_or(false) {
        return false;
    }
    let dest = s.str_of("dest").unwrap_or_default();
    if dest.is_empty() || !Path::new(&dest).is_dir() {
        return false;
    }
    let st = state(v);
    let now = now_ms();
    let every = (s.get("everyHours").and_then(|x| x.as_f64()).unwrap_or(24.0) * HOUR as f64) as i64;
    let last_ok = st.get("lastOk").and_then(|x| x.as_i64()).unwrap_or(0);
    if now - last_ok < every {
        return false;
    }
    // after a failure, wait a bit before trying again — unless the drive was
    // just plugged back in
    let failed = st.get("lastResult").and_then(|r| r.get("ok")).and_then(|x| x.as_bool()) == Some(false);
    let last_try = st.get("lastRun").and_then(|x| x.as_i64()).unwrap_or(0);
    !(failed && !became_available && now - last_try < 15 * 60_000)
}

// ---------------------------------------------------------------- copying

enum CopyErr {
    /// reading the source failed (file gone, locked…): skip it
    Src(String),
    /// the content isn't what was expected (changed while copying)
    Changed,
    /// writing to the backup drive failed: stop the run
    Dst(std::io::Error),
}

fn hash_file(p: &Path) -> std::io::Result<String> {
    sha256_file(p)
}

/// Copies `src` to the object `dst`, via a temporary file that is synced,
/// read back and checked before being renamed into place.
fn copy_verified(src: &Path, dst: &Path, expect: &str) -> Result<i64, CopyErr> {
    let dir = dst.parent().unwrap();
    // only the last level (objects/ab) is ever created: if the drive went
    // away, this fails instead of quietly re-creating the folders elsewhere
    if !dir.is_dir() {
        if let Err(e) = std::fs::create_dir(dir) {
            if !dir.is_dir() {
                return Err(CopyErr::Dst(e));
            }
        }
    }
    let tmp = dir.join(format!(".tmp-{}", &new_id()[..12]));
    let mut input = std::fs::File::open(src).map_err(|e| CopyErr::Src(e.to_string()))?;
    let res = (|| {
        let mut out = std::fs::File::create(&tmp).map_err(CopyErr::Dst)?;
        let mut h = Sha256::new();
        let mut buf = vec![0u8; 1 << 20];
        let mut n = 0i64;
        loop {
            let k = input.read(&mut buf).map_err(|e| CopyErr::Src(e.to_string()))?;
            if k == 0 {
                break;
            }
            h.update(&buf[..k]);
            out.write_all(&buf[..k]).map_err(CopyErr::Dst)?;
            n += k as i64;
        }
        out.sync_all().map_err(CopyErr::Dst)?;
        drop(out);
        if h.hex() != expect {
            return Err(CopyErr::Changed);
        }
        if hash_file(&tmp).map_err(CopyErr::Dst)? != expect {
            return Err(CopyErr::Dst(std::io::Error::other("the copy on the backup drive doesn't match (failing drive?)")));
        }
        if dst.exists() {
            let _ = std::fs::remove_file(&tmp);
        } else {
            std::fs::rename(&tmp, dst).map_err(CopyErr::Dst)?;
        }
        Ok(n)
    })();
    if res.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    res
}

fn obj_path(root: &Path, sha: &str) -> PathBuf {
    root.join("objects").join(&sha[..2]).join(&sha[2..])
}

fn valid_sha(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|c| c.is_ascii_hexdigit())
}

/// A friendlier message for a failed write to the backup drive.
fn drive_error(dest: &Path, e: &std::io::Error) -> String {
    if !dest.is_dir() {
        return "The backup drive was disconnected during the backup. Nothing was lost — it will finish the next time the drive is connected.".into();
    }
    if matches!(e.raw_os_error(), Some(28) | Some(39) | Some(112)) || e.kind() == std::io::ErrorKind::StorageFull {
        return "The backup drive is full. Free some space on it, or keep fewer backups.".into();
    }
    format!("Writing to the backup drive failed: {e}")
}

/// Copies files that are new or a different size into `to` (never deletes).
/// `base` (an ancestor of `to`) must exist: folders are never created above it.
fn mirror_dir(from: &Path, to: &Path, base: &Path) -> Result<(i64, i64), std::io::Error> {
    let (mut n, mut bytes) = (0, 0);
    if !from.is_dir() {
        return Ok((0, 0));
    }
    let w = crate::files::walk(from, &crate::files::Ignore::parse(""));
    for (rel, seen) in &w.files {
        let (Ok(src), Ok(dst)) = (safe_join(from, rel), safe_join(to, rel)) else { continue };
        if std::fs::metadata(&dst).map(|m| m.len() as i64 == seen.size).unwrap_or(false) {
            continue;
        }
        if !base.is_dir() {
            return Err(std::io::Error::new(std::io::ErrorKind::NotFound, "the folder went away"));
        }
        std::fs::create_dir_all(dst.parent().unwrap())?;
        let tmp = dst.with_file_name(format!(".tmp-{}", &new_id()[..12]));
        match std::fs::copy(&src, &tmp) {
            Ok(b) => {
                std::fs::rename(&tmp, &dst)?;
                n += 1;
                bytes += b as i64;
            }
            Err(e) if std::fs::File::open(&src).is_err() => {
                let _ = std::fs::remove_file(&tmp);
                let _ = e; // source vanished or is locked: next run
            }
            Err(e) => {
                let _ = std::fs::remove_file(&tmp);
                return Err(e);
            }
        }
    }
    Ok((n, bytes))
}

// ---------------------------------------------------------------- runs

struct PlanFile {
    rel: String,
    sha: Option<String>,
    size: i64,
    mtime: i64,
}
struct PlanProject {
    id: String,
    name: String,
    root: PathBuf,
    files: Vec<PlanFile>,
}
struct Plan {
    dest: PathBuf,
    root: PathBuf,
    run_id: String,
    created_at: i64,
    db_tmp: PathBuf,
    store: PathBuf,
    data_dir: PathBuf,
    managed_root: PathBuf,
    vault_id: String,
    keep: usize,
    projects: Vec<PlanProject>,
}

/// Under the vault lock: copies the database (locally, fast) and lists files.
fn prepare(v: &Vault, dest: &Path) -> DbResult<Plan> {
    if !dest.is_dir() {
        return Err(format!("The backup drive isn't connected ({})", projects::display_path(dest)));
    }
    let root = root_of(dest);
    for sub in ["objects", "runs"] {
        std::fs::create_dir_all(root.join(sub)).map_err(|e| format!("Can't write to {}: {e}", projects::display_path(&root)))?;
    }
    let created_at = now_ms();
    let vid = vault_id(v)?;
    let run_id = format!("{}-{}", iso_utc(created_at).replace([' ', ':'], "-").replace("-UTC", ""), &new_id()[..8]);
    let db_tmp = v.data_dir.join("backups").join(format!(".drive-{run_id}.db"));
    let _ = std::fs::remove_file(&db_tmp);
    std::fs::create_dir_all(db_tmp.parent().unwrap()).map_err(|e| e.to_string())?;
    v.db.exec("VACUUM INTO ?", p![db_tmp.to_string_lossy().to_string()])?;
    let mut list = Vec::new();
    for pr in v.db.query("SELECT id, name, root_path FROM projects ORDER BY name", p![])? {
        let files = v
            .db
            .query("SELECT rel_path, sha256, size, mtime FROM files WHERE project_id=? AND state='present' ORDER BY rel_path", p![pr.s("id")])?
            .into_iter()
            .map(|f| PlanFile { rel: f.s("rel_path"), sha: f.os("sha256"), size: f.i("size"), mtime: f.i("mtime") })
            .collect();
        list.push(PlanProject { id: pr.s("id"), name: pr.s("name"), root: PathBuf::from(pr.s("root_path")), files });
    }
    let keep = settings(v)?.get("keep").and_then(|x| x.as_i64()).unwrap_or(30).max(1) as usize;
    Ok(Plan {
        dest: dest.to_path_buf(),
        root,
        run_id,
        created_at,
        db_tmp,
        store: versions::storage_root(v),
        data_dir: v.data_dir.clone(),
        managed_root: crate::storage::root(v),
        vault_id: vid,
        keep,
        projects: list,
    })
}

struct RunInfo {
    id: String,
    dir: PathBuf,
    summary: Json,
}

fn list_runs_in(root: &Path) -> Vec<RunInfo> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(root.join("runs")) else { return out };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if name.starts_with('.') || !e.path().join("manifest.json").is_file() {
            continue;
        }
        let summary = std::fs::read_to_string(e.path().join("summary.json")).ok().and_then(|s| Json::parse(&s).ok()).unwrap_or(Json::obj());
        out.push(RunInfo { id: name, dir: e.path(), summary });
    }
    out.sort_by_key(|r| -r.summary.get("createdAt").and_then(|x| x.as_i64()).unwrap_or(0));
    out
}

fn read_manifest(dir: &Path) -> DbResult<Json> {
    let s = std::fs::read_to_string(dir.join("manifest.json")).map_err(|e| format!("This backup can't be read: {e}"))?;
    Json::parse(&s).map_err(|_| "This backup's file list is damaged".to_string())
}

fn manifest_files(pj: &Json) -> Vec<(String, String, i64, i64)> {
    pj.get("files")
        .and_then(|x| x.as_arr())
        .map(|a| {
            a.iter()
                .filter_map(|f| {
                    let f = f.as_arr()?;
                    Some((f.first()?.as_str()?.to_string(), f.get(1)?.as_str()?.to_string(), f.get(2)?.as_i64()?, f.get(3).and_then(|x| x.as_i64()).unwrap_or(0)))
                })
                .collect()
        })
        .unwrap_or_default()
}

const README: &str = "This folder is a Mothball backup.\r\n\r\n\
It holds your projects' files, every saved version, snapshot and checkpoint, and Mothball's database, \
as they were at each backup in runs/. File contents are stored once in objects/ (named by their SHA-256), \
so later backups only add what changed.\r\n\r\n\
To get things back, open Mothball → Settings → Backups → Backup drive, choose this folder, and pick a backup. \
You can restore one project's files, or everything (for example on a new PC).\r\n\r\n\
Don't edit or delete files in here by hand: Mothball removes old backups itself, as set in Settings.\r\n";

/// Without the vault: copies everything new to the drive and writes the run.
fn execute(plan: &Plan, progress: Progress) -> Result<Json, String> {
    let started = now_ms();
    let root = &plan.root;
    let derr = |e: &std::io::Error| drive_error(&plan.dest, e);
    let partial = root.join("runs").join(format!(".{}.partial", plan.run_id));
    std::fs::create_dir(&partial).map_err(|e| derr(&e))?;
    let res = (|| -> Result<Json, String> {
        if !root.join("README.txt").exists() {
            let _ = atomic_write(&root.join("README.txt"), README.as_bytes());
        }
        // database
        progress("Copying the database", 0, 1, 0);
        let db_sha = hash_file(&plan.db_tmp).map_err(|e| e.to_string())?;
        let mut bytes = match copy_verified(&plan.db_tmp, &partial.join("vault.db"), &db_sha) {
            Ok(n) => n,
            Err(CopyErr::Dst(e)) => return Err(derr(&e)),
            Err(_) => return Err("Couldn't copy the database".into()),
        };

        // history store (versions, snapshots, checkpoints)
        let mut store: Vec<(String, i64)> = Vec::new();
        let local_objects = plan.store.join("objects");
        if local_objects.is_dir() {
            let w = crate::files::walk(&local_objects, &crate::files::Ignore::parse(""));
            for (rel, seen) in &w.files {
                let sha: String = rel.split('/').collect();
                if valid_sha(&sha) && rel.len() == 65 {
                    store.push((sha, seen.size));
                }
            }
        }
        let (mut new_hist, total_hist) = (0i64, store.len() as i64);
        let mut gone = HashSet::new();
        for (i, (sha, _)) in store.iter().enumerate() {
            if i % 64 == 0 {
                progress("Copying file history", i as i64, total_hist, bytes);
            }
            let dst = obj_path(root, sha);
            if dst.exists() {
                continue;
            }
            match copy_verified(&obj_path(&plan.store, sha), &dst, sha) {
                Ok(n) => {
                    new_hist += 1;
                    bytes += n;
                }
                Err(CopyErr::Dst(e)) => return Err(derr(&e)),
                // removed (pruned) since it was listed
                Err(_) => {
                    gone.insert(sha.clone());
                }
            }
        }
        store.retain(|(s, _)| !gone.contains(s));
        for sub in ["history", "snapshots"] {
            let (_, b) = mirror_dir(&plan.store.join(sub), &root.join("store").join(sub), root).map_err(|e| derr(&e))?;
            bytes += b;
        }
        let (_, b) = mirror_dir(&plan.data_dir.join("pictures"), &root.join("pictures"), root).map_err(|e| derr(&e))?;
        bytes += b;

        // project files
        let prev = list_runs_in(root).into_iter().find(|r| r.summary.str_of("vaultId").as_deref() == Some(plan.vault_id.as_str()));
        let prev_manifest = prev.as_ref().and_then(|r| read_manifest(&r.dir).ok());
        let total: i64 = plan.projects.iter().map(|p| p.files.len() as i64).sum();
        let (mut done, mut files_n, mut new_files) = (0i64, 0i64, 0i64);
        let mut skipped: Vec<Json> = Vec::new();
        let mut skipped_n = 0i64;
        let mut carried: Vec<Json> = Vec::new();
        let mut sizes: HashMap<String, i64> = store.iter().cloned().collect();
        let mut pjs = Vec::new();
        for pr in &plan.projects {
            let mut entry = Json::obj().with("id", pr.id.as_str()).with("name", pr.name.as_str()).with("root", pr.root.to_string_lossy().to_string());
            let mut rows: Vec<Json> = Vec::new();
            if !pr.root.is_dir() {
                // folder not reachable (drive unplugged?): keep what the
                // previous backup had for it rather than backing up nothing
                let old = prev_manifest.as_ref().and_then(|m| m.get("projects")?.as_arr()?.iter().find(|p| p.str_of("id").as_deref() == Some(pr.id.as_str())).cloned());
                if let Some(old) = old {
                    for (rel, sha, size, mtime) in manifest_files(&old) {
                        sizes.insert(sha.clone(), size);
                        rows.push(Json::Arr(vec![Json::from(rel), Json::from(sha), Json::from(size), Json::from(mtime)]));
                    }
                    entry.set("carriedFrom", old.get("at").cloned().or_else(|| prev.as_ref().and_then(|r| r.summary.get("createdAt").cloned())).unwrap_or(Json::Null));
                    entry.set("at", old.get("at").cloned().unwrap_or(Json::Null));
                    carried.push(Json::from(pr.name.as_str()));
                } else {
                    skipped_n += pr.files.len() as i64;
                    skipped.push(Json::obj().with("project", pr.name.as_str()).with("path", "").with("reason", "project folder not reachable"));
                }
                done += pr.files.len() as i64;
                entry.set("files", Json::Arr(rows));
                pjs.push(entry);
                continue;
            }
            entry.set("at", plan.created_at);
            for f in &pr.files {
                done += 1;
                if done % 64 == 0 {
                    progress("Copying project files", done, total, bytes);
                }
                let mut skip = |reason: &str| {
                    skipped_n += 1;
                    if skipped.len() < 50 {
                        skipped.push(Json::obj().with("project", pr.name.as_str()).with("path", f.rel.as_str()).with("reason", reason));
                    }
                };
                let Ok(path) = safe_join(&pr.root, &f.rel) else {
                    skip("unsafe path");
                    continue;
                };
                let Ok(meta) = std::fs::metadata(&path) else {
                    continue; // deleted since the last scan: not part of the project now
                };
                if crate::files::is_cloud_placeholder(&meta) {
                    skip("only in OneDrive, not on this PC");
                    continue;
                }
                let size = meta.len() as i64;
                let mtime = meta.modified().map(crate::util::system_time_ms).unwrap_or(0);
                let mut ok = None;
                for attempt in 0..2 {
                    let sha = match (&f.sha, attempt) {
                        (Some(s), 0) if size == f.size && mtime == f.mtime => s.clone(),
                        _ => match hash_file(&path) {
                            Ok(h) => h,
                            Err(e) => {
                                skip(&format!("couldn't read it: {e}"));
                                break;
                            }
                        },
                    };
                    let dst = obj_path(root, &sha);
                    if dst.exists() {
                        ok = Some(sha);
                        break;
                    }
                    match copy_verified(&path, &dst, &sha) {
                        Ok(n) => {
                            bytes += n;
                            new_files += 1;
                            ok = Some(sha);
                            break;
                        }
                        Err(CopyErr::Dst(e)) => return Err(derr(&e)),
                        Err(CopyErr::Src(e)) => {
                            skip(&format!("couldn't read it: {e}"));
                            break;
                        }
                        Err(CopyErr::Changed) if attempt == 0 => continue,
                        Err(CopyErr::Changed) => skip("kept changing while being copied"),
                    }
                }
                if let Some(sha) = ok {
                    files_n += 1;
                    sizes.insert(sha.clone(), size);
                    rows.push(Json::Arr(vec![Json::from(f.rel.as_str()), Json::from(sha), Json::from(size), Json::from(mtime)]));
                }
            }
            entry.set("files", Json::Arr(rows));
            pjs.push(entry);
        }

        // check every object this run refers to is on the drive, complete
        progress("Checking the backup", 0, sizes.len() as i64, bytes);
        for (sha, size) in &sizes {
            match std::fs::metadata(obj_path(root, sha)) {
                Ok(m) if m.len() as i64 == *size => {}
                Ok(_) => {
                    let _ = std::fs::remove_file(obj_path(root, sha));
                    return Err(format!("A file on the backup drive was incomplete ({}); it will be copied again next time.", &sha[..12]));
                }
                Err(e) => return Err(derr(&e)),
            }
        }

        let manifest = Json::obj()
            .with("format", FORMAT)
            .with("app", crate::APP_VERSION)
            .with("schemaVersion", schema::latest_version())
            .with("vaultId", plan.vault_id.as_str())
            .with("createdAt", plan.created_at)
            .with("dataDir", plan.data_dir.to_string_lossy().to_string())
            .with("storageRoot", plan.store.to_string_lossy().to_string())
            .with("managedRoot", plan.managed_root.to_string_lossy().to_string())
            .with("dbSha256", db_sha.as_str())
            .with("projects", Json::Arr(pjs.clone()));
        let w = |name: &str, data: &[u8]| atomic_write(&partial.join(name), data).map_err(|e| derr(&e));
        let objs: String = store.iter().map(|(s, n)| format!("{s} {n}\n")).collect();
        w("objects.txt", objs.as_bytes())?;
        w("manifest.json", manifest.to_string().as_bytes())?;
        let summary = Json::obj()
            .with("format", FORMAT)
            .with("app", crate::APP_VERSION)
            .with("vaultId", plan.vault_id.as_str())
            .with("createdAt", plan.created_at)
            .with("files", files_n)
            .with("historyItems", store.len())
            .with("bytes", sizes.values().sum::<i64>())
            .with(
                "projects",
                Json::Arr(
                    pjs.iter()
                        .map(|p| {
                            Json::obj()
                                .with("id", p.str_of("id").unwrap_or_default())
                                .with("name", p.str_of("name").unwrap_or_default())
                                .with("files", p.get("files").and_then(|x| x.as_arr()).map(|a| a.len()).unwrap_or(0))
                                .with("carriedFrom", p.get("carriedFrom").cloned().unwrap_or(Json::Null))
                        })
                        .collect(),
                ),
            );
        w("summary.json", summary.to_string().as_bytes())?;
        std::fs::rename(&partial, root.join("runs").join(&plan.run_id)).map_err(|e| derr(&e))?;

        progress("Removing old backups", 0, 1, bytes);
        let (removed, freed) = prune(root, &plan.vault_id, plan.keep);
        Ok(Json::obj()
            .with("ok", true)
            .with("runId", plan.run_id.as_str())
            .with("files", files_n)
            .with("newFiles", new_files)
            .with("historyItems", store.len())
            .with("newHistoryItems", new_hist)
            .with("bytesCopied", bytes)
            .with("carriedOver", Json::Arr(carried))
            .with("skipped", Json::Arr(skipped))
            .with("skippedCount", skipped_n)
            .with("removedRuns", removed)
            .with("freedBytes", freed)
            .with("ms", now_ms() - started))
    })();
    if res.is_err() {
        let _ = std::fs::remove_dir_all(&partial);
    }
    res
}

/// Removes this vault's oldest runs beyond `keep`, stale unfinished runs, and
/// objects no remaining run refers to. Returns (runs removed, bytes freed).
fn prune(root: &Path, vault_id: &str, keep: usize) -> (i64, i64) {
    let runs = list_runs_in(root);
    let mine: Vec<&RunInfo> = runs.iter().filter(|r| r.summary.str_of("vaultId").as_deref() == Some(vault_id)).collect();
    let mut removed = 0;
    for r in mine.iter().skip(keep.max(1)) {
        if std::fs::remove_dir_all(&r.dir).is_ok() {
            removed += 1;
        }
    }
    // unfinished runs: another Mothball could be writing one right now, so
    // only clean up old ones, and don't collect objects while one is young
    let now = now_ms();
    let mut young_partial = false;
    if let Ok(rd) = std::fs::read_dir(root.join("runs")) {
        for e in rd.flatten() {
            if !e.file_name().to_string_lossy().ends_with(".partial") {
                continue;
            }
            let age = e.metadata().and_then(|m| m.modified()).map(|t| now - crate::util::system_time_ms(t)).unwrap_or(0);
            if age > 6 * HOUR {
                let _ = std::fs::remove_dir_all(e.path());
            } else {
                young_partial = true;
            }
        }
    }
    if removed == 0 || young_partial {
        return (removed, 0);
    }
    let Some(live) = referenced(root) else { return (removed, 0) };
    let mut freed = 0;
    let w = crate::files::walk(&root.join("objects"), &crate::files::Ignore::parse(""));
    for (rel, seen) in &w.files {
        let p = root.join("objects").join(rel);
        let old = now - seen.mtime > DAY;
        let name = rel.rsplit('/').next().unwrap_or("");
        if name.starts_with(".tmp-") {
            if old {
                let _ = std::fs::remove_file(&p);
            }
            continue;
        }
        let sha: String = rel.split('/').collect();
        if old && valid_sha(&sha) && !live.contains(&sha) && std::fs::remove_file(&p).is_ok() {
            freed += seen.size;
        }
    }
    (removed, freed)
}

/// Every object any complete run refers to; None if a run can't be read (then
/// nothing is collected, to be safe).
fn referenced(root: &Path) -> Option<HashSet<String>> {
    let mut live = HashSet::new();
    for r in list_runs_in(root) {
        let m = read_manifest(&r.dir).ok()?;
        for pj in m.get("projects")?.as_arr()? {
            for (_, sha, _, _) in manifest_files(pj) {
                live.insert(sha);
            }
        }
        let objs = std::fs::read_to_string(r.dir.join("objects.txt")).ok()?;
        for l in objs.lines() {
            if let Some(s) = l.split(' ').next() {
                live.insert(s.to_string());
            }
        }
    }
    Some(live)
}

fn finish(v: &Vault, result: &Json) -> DbResult<()> {
    let mut st = state(v);
    st.set("lastRun", now_ms());
    if result.get("ok").and_then(|x| x.as_bool()) == Some(true) {
        st.set("lastOk", now_ms());
    }
    st.set("lastResult", result.clone());
    meta_put(v, "backup_drive_state", &st)
}

/// Makes one backup run now (whether or not it's due).
pub fn backup(w: With, progress: Progress) -> DbResult<Json> {
    let dest = with(w, |v| Ok(PathBuf::from(settings(v)?.str_of("dest").unwrap_or_default())))?;
    if dest.as_os_str().is_empty() {
        return Err("Choose a backup drive first".into());
    }
    let plan = match with(w, |v| prepare(v, &dest)) {
        Ok(p) => p,
        Err(e) => {
            let r = Json::obj().with("ok", false).with("error", e.as_str());
            with(w, |v| finish(v, &r))?;
            return Ok(r);
        }
    };
    let r = execute(&plan, progress).unwrap_or_else(|e| Json::obj().with("ok", false).with("error", e));
    let _ = std::fs::remove_file(&plan.db_tmp);
    with(w, |v| finish(v, &r))?;
    Ok(r)
}

// ---------------------------------------------------------------- reading backups

/// Backups on the drive (newest first). `dest` defaults to the configured one.
pub fn runs(v: &Vault, dest: Option<&Path>) -> DbResult<Json> {
    let dest = match dest {
        Some(d) => d.to_path_buf(),
        None => PathBuf::from(settings(v)?.str_of("dest").unwrap_or_default()),
    };
    let root = root_of(&dest);
    if !dest.is_dir() {
        return Err("The backup drive isn't connected".into());
    }
    let me = vault_id(v)?;
    let list: Vec<Json> = list_runs_in(&root)
        .into_iter()
        .map(|r| {
            let mine = r.summary.str_of("vaultId").as_deref() == Some(me.as_str());
            let mut j = r.summary.clone();
            j.set("id", r.id.as_str());
            j.set("mine", mine);
            j
        })
        .collect();
    Ok(Json::obj().with("root", projects::display_path(&root)).with("runs", Json::Arr(list)))
}

fn run_dir(dest: &Path, run_id: &str) -> DbResult<PathBuf> {
    if run_id.is_empty() || run_id.contains(['/', '\\']) || run_id.starts_with('.') {
        return Err("invalid backup".into());
    }
    let d = root_of(dest).join("runs").join(run_id);
    if !d.join("manifest.json").is_file() {
        return Err("That backup isn't on the drive (disconnected, or removed as an old backup)".into());
    }
    Ok(d)
}

fn dest_of(v: &Vault, a: Option<&Path>) -> DbResult<PathBuf> {
    Ok(match a {
        Some(d) => d.to_path_buf(),
        None => PathBuf::from(settings(v)?.str_of("dest").unwrap_or_default()),
    })
}

fn project_in(m: &Json, project_id: &str) -> DbResult<Json> {
    m.get("projects")
        .and_then(|x| x.as_arr())
        .and_then(|a| a.iter().find(|p| p.str_of("id").as_deref() == Some(project_id)).cloned())
        .ok_or_else(|| "That project isn't in this backup".to_string())
}

fn target_root(v: &Vault, pj: &Json) -> DbResult<PathBuf> {
    Ok(match v.db.one("SELECT root_path FROM projects WHERE id=?", p![pj.str_of("id").unwrap_or_default()])? {
        Some(r) => PathBuf::from(r.s("root_path")),
        None => PathBuf::from(pj.str_of("root").unwrap_or_default()),
    })
}

/// Whether the file at `target` already has `sha` — using the scan's record
/// when size and time still match, to avoid reading big files.
fn same_as(v: &Vault, project_id: &str, rel: &str, target: &Path, sha: &str) -> bool {
    let Ok(m) = std::fs::metadata(target) else { return false };
    if let Ok(Some(r)) = v.db.one("SELECT sha256, size, mtime FROM files WHERE project_id=? AND rel_path=? AND state='present'", p![project_id, rel]) {
        let mt = m.modified().map(crate::util::system_time_ms).unwrap_or(0);
        if r.i("size") == m.len() as i64 && r.i("mtime") == mt && r.os("sha256").is_some() {
            return r.os("sha256").as_deref() == Some(sha);
        }
    }
    hash_file(target).ok().as_deref() == Some(sha)
}

/// What restoring one project from a backup would do.
pub fn restore_preview(v: &Vault, dest: Option<&Path>, run_id: &str, project_id: &str) -> DbResult<Json> {
    let dir = run_dir(&dest_of(v, dest)?, run_id)?;
    let m = read_manifest(&dir)?;
    let pj = project_in(&m, project_id)?;
    let root = target_root(v, &pj)?;
    let (mut recreate, mut differs, mut same) = (0, 0, 0);
    let mut items = Vec::new();
    for (rel, sha, size, _) in manifest_files(&pj) {
        let Ok(t) = safe_join(&root, &rel) else { continue };
        let state = if !t.exists() {
            recreate += 1;
            "recreate"
        } else if same_as(v, project_id, &rel, &t, &sha) {
            same += 1;
            "same"
        } else {
            differs += 1;
            "differs"
        };
        if state != "same" && items.len() < 500 {
            items.push(Json::obj().with("path", rel).with("state", state).with("size", size));
        }
    }
    Ok(Json::obj()
        .with("root", projects::display_path(&root))
        .with("rootExists", root.is_dir())
        .with("recreate", recreate)
        .with("differs", differs)
        .with("same", same)
        .with("items", Json::Arr(items))
        .with("at", pj.get("at").cloned().unwrap_or(Json::Null)))
}

/// Writes object `sha` from the backup to `target` (checked), via a temp file.
fn write_from(root: &Path, sha: &str, target: &Path) -> Result<i64, String> {
    let dir = target.parent().ok_or("bad path")?;
    std::fs::create_dir_all(dir).map_err(|e| format!("can't create the folder: {e}"))?;
    let tmp = dir.join(format!(".pv-tmp-{}", &new_id()[..8]));
    let src = obj_path(root, sha);
    if !src.is_file() {
        return Err("its copy is missing from the backup drive".into());
    }
    match copy_verified(&src, &tmp, sha) {
        Ok(n) => {
            std::fs::rename(&tmp, target).map_err(|e| {
                let _ = std::fs::remove_file(&tmp);
                format!("couldn't write it (open in another program?): {e}")
            })?;
            Ok(n)
        }
        Err(CopyErr::Changed) => Err("its copy on the backup drive is damaged".into()),
        Err(CopyErr::Src(e)) => Err(format!("couldn't read it from the backup drive: {e}")),
        Err(CopyErr::Dst(e)) => Err(format!("couldn't write it: {e}")),
    }
}

/// Restores one project's files from a backup. Missing files are recreated;
/// different ones are replaced only with `overwrite`, after keeping the
/// current content as a version. Nothing is deleted.
pub fn restore_project(w: With, dest: Option<&Path>, run_id: &str, project_id: &str, overwrite: bool, progress: Progress) -> DbResult<Json> {
    let dest = with(w, |v| dest_of(v, dest))?;
    let dir = run_dir(&dest, run_id)?;
    let root = root_of(&dest);
    let m = read_manifest(&dir)?;
    let pj = project_in(&m, project_id)?;
    let (target, tracked) = with(w, |v| Ok((target_root(v, &pj)?, v.db.one("SELECT id FROM projects WHERE id=?", p![project_id])?.is_some())))?;
    std::fs::create_dir_all(&target).map_err(|e| format!("The project folder {} can't be created: {e}", projects::display_path(&target)))?;
    let files = manifest_files(&pj);
    let (mut restored, mut kept, mut errors) = (0i64, 0i64, Vec::new());
    for (i, (rel, sha, _, _)) in files.iter().enumerate() {
        if i % 32 == 0 {
            progress("Restoring files", i as i64, files.len() as i64, 0);
        }
        let res = (|| -> Result<bool, String> {
            let t = safe_join(&target, rel)?;
            if t.exists() {
                let same = with(w, |v| Ok(same_as(v, project_id, rel, &t, sha))).unwrap_or(false);
                if same {
                    return Ok(false);
                }
                if !overwrite {
                    kept += 1;
                    return Ok(false);
                }
                if tracked {
                    with(w, |v| {
                        if let Some(row) = v.db.one("SELECT * FROM files WHERE project_id=? AND rel_path=?", p![project_id, rel.as_str()])? {
                            let cur = hash_file(&t).map_err(|e| e.to_string())?;
                            versions::ensure_stored(v, &row, &t, &cur, "before-restore")?;
                        }
                        Ok(())
                    })?;
                }
            }
            write_from(&root, sha, &t)?;
            Ok(true)
        })();
        match res {
            Ok(true) => restored += 1,
            Ok(false) => {}
            Err(e) => errors.push(Json::from(format!("{rel}: {e}"))),
        }
    }
    let at = pj.get("at").and_then(|x| x.as_i64()).unwrap_or(0);
    if tracked {
        with(w, |v| {
            crate::files::scan(v, project_id)?;
            crate::timeline::add(
                v,
                project_id,
                "backup",
                "Restored project files from the backup drive",
                &format!("Backup from {}: {restored} restored, {kept} left as they are", iso_utc(at)),
                None,
                false,
            )?;
            Ok(())
        })?;
    }
    Ok(Json::obj()
        .with("ok", errors.is_empty())
        .with("kind", "restoreProject")
        .with("project", pj.str_of("name").unwrap_or_default())
        .with("restored", restored)
        .with("keptDifferent", kept)
        .with("errors", Json::Arr(errors))
        .with("root", projects::display_path(&target)))
}

fn remap(p: &str, from: &Path, to: &Path) -> Option<String> {
    let rest = Path::new(p).strip_prefix(from).ok()?;
    Some(to.join(rest).to_string_lossy().to_string())
}

/// Restores everything from a backup — for a new PC, or after losing the
/// drive Mothball was on. Copies the file history into this PC's store,
/// recreates project files that are missing (never overwrites), and stages
/// the backup's database to replace the current one at the next start (the
/// current one is kept in backups/).
pub fn restore_all(w: With, dest: Option<&Path>, run_id: &str, progress: Progress) -> DbResult<Json> {
    let dest = with(w, |v| dest_of(v, dest))?;
    let dir = run_dir(&dest, run_id)?;
    let root = root_of(&dest);
    let m = read_manifest(&dir)?;
    if m.get("schemaVersion").and_then(|x| x.as_i64()).unwrap_or(0) > schema::latest_version() {
        return Err("This backup was made by a newer version of Mothball. Update the app first.".into());
    }
    match Db::open_readonly(&dir.join("vault.db")).and_then(|db| db.one("PRAGMA integrity_check", p![])) {
        Ok(Some(r)) if r.vals[0].as_str() == Some("ok") => {}
        _ => return Err("The database in this backup failed its check, so nothing was changed.".into()),
    }
    let (data_dir, store, managed) = with(w, |v| Ok((v.data_dir.clone(), versions::storage_root(v), crate::storage::root(v))))?;
    std::fs::create_dir_all(store.join("objects")).map_err(|e| e.to_string())?;

    // file history
    let objs = std::fs::read_to_string(dir.join("objects.txt")).map_err(|e| format!("This backup can't be read: {e}"))?;
    let list: Vec<&str> = objs.lines().filter_map(|l| l.split(' ').next()).filter(|s| valid_sha(s)).collect();
    let mut errors: Vec<Json> = Vec::new();
    let mut bytes = 0;
    for (i, sha) in list.iter().enumerate() {
        if i % 64 == 0 {
            progress("Restoring file history", i as i64, list.len() as i64, bytes);
        }
        let to = obj_path(&store, sha);
        if to.exists() {
            continue;
        }
        match copy_verified(&obj_path(&root, sha), &to, sha) {
            Ok(n) => bytes += n,
            Err(CopyErr::Dst(e)) => return Err(format!("Couldn't write to {}: {e}", projects::display_path(&store))),
            Err(_) => {
                if errors.len() < 50 {
                    errors.push(Json::from(format!("history item {}: missing or damaged on the backup drive", &sha[..12])));
                }
            }
        }
    }
    for sub in ["history", "snapshots"] {
        mirror_dir(&root.join("store").join(sub), &store.join(sub), &store).map_err(|e| e.to_string())?;
    }
    mirror_dir(&root.join("pictures"), &data_dir.join("pictures"), &data_dir).map_err(|e| e.to_string())?;

    // where things go on this PC
    let old_data = PathBuf::from(m.str_of("dataDir").unwrap_or_default());
    let old_managed = PathBuf::from(m.str_of("managedRoot").unwrap_or_default());
    let place = |root: &str| -> String {
        if !old_managed.as_os_str().is_empty() && !old_managed.is_dir() {
            if let Some(p) = remap(root, &old_managed, &managed) {
                return p;
            }
        }
        if !old_data.as_os_str().is_empty() && old_data != data_dir {
            if let Some(p) = remap(root, &old_data, &data_dir) {
                return p;
            }
        }
        root.to_string()
    };

    // project files: only missing ones
    let pjs = m.get("projects").and_then(|x| x.as_arr()).cloned().unwrap_or_default();
    let total: i64 = pjs.iter().map(|p| manifest_files(p).len() as i64).sum();
    let (mut done, mut restored) = (0i64, 0i64);
    let mut moved: Vec<(String, String)> = Vec::new();
    for pj in &pjs {
        let name = pj.str_of("name").unwrap_or_default();
        let old_root = pj.str_of("root").unwrap_or_default();
        let new_root = place(&old_root);
        if new_root != old_root {
            moved.push((pj.str_of("id").unwrap_or_default(), new_root.clone()));
        }
        let target = PathBuf::from(&new_root);
        let files = manifest_files(pj);
        if let Err(e) = std::fs::create_dir_all(&target) {
            errors.push(Json::from(format!("{name}: the folder {} can't be created ({e}) — restore this project later from Settings", projects::display_path(&target))));
            done += files.len() as i64;
            continue;
        }
        for (rel, sha, _, _) in files {
            done += 1;
            if done % 32 == 0 {
                progress("Restoring project files", done, total, bytes);
            }
            let Ok(t) = safe_join(&target, &rel) else { continue };
            if t.exists() {
                continue;
            }
            match write_from(&root, &sha, &t) {
                Ok(n) => {
                    restored += 1;
                    bytes += n;
                }
                Err(e) => {
                    if errors.len() < 50 {
                        errors.push(Json::from(format!("{name}/{rel}: {e}")));
                    }
                }
            }
        }
    }

    // the database, adjusted to this PC, applied at the next start
    progress("Preparing the database", 0, 1, bytes);
    let pending = data_dir.join(crate::backup::PENDING_DB);
    let staging = data_dir.join(format!("{}.staging", crate::backup::PENDING_DB));
    let _ = std::fs::remove_file(&staging);
    std::fs::copy(dir.join("vault.db"), &staging).map_err(|e| format!("Couldn't copy the database: {e}"))?;
    let adjust = (|| -> DbResult<()> {
        let db = Db::open(&staging)?;
        let set = |k: &str, val: String| db.exec("INSERT INTO meta(key,value) VALUES (?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value", p![k, val]);
        set("storage_root", store.to_string_lossy().to_string())?;
        if let Some(r) = db.one("SELECT value FROM meta WHERE key='managed_root'", p![])? {
            set("managed_root", place(&r.s("value")))?;
        }
        for (id, root) in &moved {
            db.exec("UPDATE projects SET root_path=? WHERE id=?", p![root.as_str(), id.as_str()])?;
        }
        // keep backing up to the drive this came from (its letter may differ here)
        if let Some(r) = db.one("SELECT value FROM meta WHERE key='backup_drive'", p![])? {
            let mut s = Json::parse(&r.s("value")).unwrap_or(Json::obj());
            s.set("dest", dest.to_string_lossy().to_string());
            set("backup_drive", s.to_string())?;
        }
        db.batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
        Ok(())
    })();
    if let Err(e) = adjust {
        let _ = std::fs::remove_file(&staging);
        return Err(format!("Couldn't prepare the database: {e}"));
    }
    for ext in ["-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{ext}", staging.display()));
    }
    std::fs::rename(&staging, &pending).map_err(|e| e.to_string())?;
    Ok(Json::obj()
        .with("ok", errors.is_empty())
        .with("kind", "restoreAll")
        .with("restoredFiles", restored)
        .with("historyItems", list.len())
        .with("errors", Json::Arr(errors))
        .with("restartRequired", true))
}

/// Re-reads every file the newest backup of this vault refers to and checks
/// its content. Slow (reads the whole backup) — run on request.
pub fn verify(w: With, progress: Progress) -> DbResult<Json> {
    let (dest, me) = with(w, |v| Ok((PathBuf::from(settings(v)?.str_of("dest").unwrap_or_default()), vault_id(v)?)))?;
    if !dest.is_dir() {
        return Err("The backup drive isn't connected".into());
    }
    let root = root_of(&dest);
    let run = list_runs_in(&root).into_iter().find(|r| r.summary.str_of("vaultId").as_deref() == Some(me.as_str())).ok_or("There's no backup on this drive yet")?;
    let m = read_manifest(&run.dir)?;
    let mut want: BTreeMap<String, ()> = BTreeMap::new();
    for pj in m.get("projects").and_then(|x| x.as_arr()).cloned().unwrap_or_default() {
        for (_, sha, _, _) in manifest_files(&pj) {
            want.insert(sha, ());
        }
    }
    for l in std::fs::read_to_string(run.dir.join("objects.txt")).unwrap_or_default().lines() {
        if let Some(s) = l.split(' ').next().filter(|s| valid_sha(s)) {
            want.insert(s.to_string(), ());
        }
    }
    let mut bad = Vec::new();
    let mut bytes = 0i64;
    for (i, sha) in want.keys().enumerate() {
        if i % 32 == 0 {
            progress("Checking every file on the drive", i as i64, want.len() as i64, bytes);
        }
        let p = obj_path(&root, sha);
        bytes += std::fs::metadata(&p).map(|m| m.len() as i64).unwrap_or(0);
        if hash_file(&p).ok().as_deref() != Some(sha.as_str()) {
            // remove it so the next backup copies it again
            let _ = std::fs::remove_file(&p);
            bad.push(sha.clone());
        }
    }
    let db_ok = matches!(Db::open_readonly(&run.dir.join("vault.db")).and_then(|db| db.one("PRAGMA integrity_check", p![])), Ok(Some(r)) if r.vals[0].as_str() == Some("ok"));
    let ok = bad.is_empty() && db_ok;
    let r = Json::obj()
        .with("ok", ok)
        .with("kind", "verify")
        .with("at", now_ms())
        .with("checked", want.len())
        .with("bytes", bytes)
        .with("damaged", bad.len())
        .with("databaseOk", db_ok);
    with(w, |v| {
        let mut st = state(v);
        st.set("lastVerify", r.clone());
        if !bad.is_empty() {
            // damaged copies were removed; make sure the next run replaces them
            st.remove("lastOk");
        }
        meta_put(v, "backup_drive_state", &st)
    })?;
    Ok(r)
}

// ---------------------------------------------------------------- background

#[derive(Clone)]
pub enum Job {
    Backup,
    Verify,
    RestoreProject { dest: Option<PathBuf>, run: String, project: String, overwrite: bool },
    RestoreAll { dest: Option<PathBuf>, run: String },
}

/// Queues a job for the background worker (see [`start`]).
pub fn enqueue(job: Job) -> DbResult<Json> {
    let busy = PROGRESS.lock().map(|p| p.is_some()).unwrap_or(false);
    let mut q = QUEUE.lock().map_err(|_| "busy")?;
    if busy || !q.is_empty() {
        return Err("The backup drive is busy — wait for the current job to finish".into());
    }
    q.push(job);
    Ok(Json::obj().with("queued", true))
}

pub fn run_job(w: With, job: &Job, progress: Progress) -> Json {
    let r = match job {
        Job::Backup => backup(w, progress),
        Job::Verify => verify(w, progress),
        Job::RestoreProject { dest, run, project, overwrite } => restore_project(w, dest.as_deref(), run, project, *overwrite, progress),
        Job::RestoreAll { dest, run } => restore_all(w, dest.as_deref(), run, progress),
    };
    let kind = match job {
        Job::Backup => "backup",
        Job::Verify => "verify",
        Job::RestoreProject { .. } => "restoreProject",
        Job::RestoreAll { .. } => "restoreAll",
    };
    let mut j = r.unwrap_or_else(|e| Json::obj().with("ok", false).with("error", e));
    if j.get("kind").is_none() {
        j.set("kind", kind);
    }
    j.set("finishedAt", now_ms());
    j
}

/// Background worker: makes scheduled backups (including as soon as the drive
/// is plugged back in) and runs queued jobs, copying without holding the vault.
pub fn start(locker: crate::printers::Locker) {
    std::thread::spawn(move || {
        let w = |f: &mut dyn FnMut(&Vault)| locker(f);
        let mut was_available = true;
        let mut tick = 0u64;
        loop {
            std::thread::sleep(std::time::Duration::from_secs(1));
            tick += 1;
            let mut job = QUEUE.lock().ok().and_then(|mut q| if q.is_empty() { None } else { Some(q.remove(0)) });
            if job.is_none() && tick % 30 == 1 {
                let available = with(&w, |v| {
                    let d = settings(v)?.str_of("dest").unwrap_or_default();
                    Ok(!d.is_empty() && Path::new(&d).is_dir())
                })
                .unwrap_or(false);
                let became = available && !was_available;
                was_available = available;
                if with(&w, |v| Ok(due(v, became))).unwrap_or(false) {
                    job = Some(Job::Backup);
                }
            }
            let Some(job) = job else { continue };
            if let Ok(mut p) = PROGRESS.lock() {
                *p = Some(Json::obj().with("phase", "Starting").with("done", 0).with("total", 0).with("bytes", 0));
            }
            let started = now_ms();
            let progress = |phase: &str, done: i64, total: i64, bytes: i64| {
                if let Ok(mut p) = PROGRESS.lock() {
                    *p = Some(Json::obj().with("phase", phase).with("done", done).with("total", total).with("bytes", bytes).with("startedAt", started));
                }
            };
            let r = run_job(&w, &job, &progress);
            if let Ok(mut l) = LAST_JOB.lock() {
                *l = Some(r);
            }
            if let Ok(mut p) = PROGRESS.lock() {
                *p = None;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projects::{create, NewProject};
    use crate::testutil::*;
    use std::fs;

    fn sync<'a>(v: &'a Vault) -> impl Fn(&mut dyn FnMut(&Vault)) + 'a {
        move |f: &mut dyn FnMut(&Vault)| f(v)
    }
    fn quiet(_: &str, _: i64, _: i64, _: i64) {}

    fn setup() -> (TempDir, Vault, PathBuf, String, PathBuf) {
        let (t, v) = vault();
        let d = t.path().join("proj");
        fs::create_dir_all(d.join("maps")).unwrap();
        fs::write(d.join("main.gsc"), "init()\n{\n  a();\n}\n").unwrap();
        fs::write(d.join("maps/big.bin"), vec![7u8; 300_000]).unwrap();
        let pid = create(&v, NewProject { name: "Golf".into(), description: "".into(), category: "iw4x".into(), status: "".into(), link_path: Some(d.clone()), module: Json::obj() })
            .unwrap()
            .str_of("id")
            .unwrap();
        crate::files::scan(&v, &pid).unwrap();
        let drive = t.path().join("E");
        fs::create_dir_all(&drive).unwrap();
        set_settings(&v, &Json::obj().with("enabled", true).with("dest", drive.to_string_lossy().to_string()).with("keep", 3)).unwrap();
        (t, v, d, pid, drive)
    }

    fn bump(p: &Path, s: &str) {
        fs::write(p, s).unwrap();
        let f = fs::File::options().append(true).open(p).unwrap();
        f.set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(5)).unwrap();
    }

    #[test]
    fn complete_incremental_backup_and_restore() {
        let (_t, v, d, pid, drive) = setup();
        let w = sync(&v);
        // a stored version exists in the history store
        assert!(versions::storage_root(&v).join("objects").is_dir());
        let r1 = backup(&w, &quiet).unwrap();
        assert_eq!(r1.get("ok").and_then(|x| x.as_bool()), Some(true), "{}", r1.pretty());
        assert_eq!(r1.get("files").and_then(|x| x.as_i64()), Some(2));
        assert!(r1.get("historyItems").and_then(|x| x.as_i64()).unwrap() >= 1, "file history included");
        let root = root_of(&drive);
        assert!(root.join("README.txt").exists());
        assert!(status(&v).unwrap().get("lastOk").and_then(|x| x.as_i64()).is_some());
        assert!(!due(&v, false), "not due right after a backup");

        // nothing changed: nothing new copied
        let r2 = backup(&w, &quiet).unwrap();
        assert_eq!(r2.get("newFiles").and_then(|x| x.as_i64()), Some(0));
        assert_eq!(r2.get("newHistoryItems").and_then(|x| x.as_i64()), Some(0));

        // edit, back up, then lose the file and restore it from the first run
        bump(&d.join("main.gsc"), "init()\n{\n  b();\n}\n");
        crate::files::scan(&v, &pid).unwrap();
        let r3 = backup(&w, &quiet).unwrap();
        let n = |k: &str| r3.get(k).and_then(|x| x.as_i64()).unwrap();
        assert_eq!(n("newFiles") + n("newHistoryItems"), 1, "only the edited file is copied, once");
        let runs_j = runs(&v, None).unwrap();
        let list = runs_j.get("runs").unwrap().as_arr().unwrap();
        assert_eq!(list.len(), 3);
        let first = list.last().unwrap().str_of("id").unwrap();

        fs::remove_file(d.join("maps/big.bin")).unwrap();
        let pv = restore_preview(&v, None, &first, &pid).unwrap();
        assert_eq!((pv.get("recreate").and_then(|x| x.as_i64()), pv.get("differs").and_then(|x| x.as_i64())), (Some(1), Some(1)));
        // without overwrite: recreate the missing one, leave the edited one
        let r = restore_project(&w, None, &first, &pid, false, &quiet).unwrap();
        assert_eq!((r.get("restored").and_then(|x| x.as_i64()), r.get("keptDifferent").and_then(|x| x.as_i64())), (Some(1), Some(1)));
        assert_eq!(fs::read(d.join("maps/big.bin")).unwrap().len(), 300_000);
        assert!(fs::read_to_string(d.join("main.gsc")).unwrap().contains("b();"));
        // with overwrite: the edited one goes back, and is kept as a version first
        restore_project(&w, None, &first, &pid, true, &quiet).unwrap();
        assert!(fs::read_to_string(d.join("main.gsc")).unwrap().contains("a();"));
        let fid = v.db.one("SELECT id FROM files WHERE rel_path='main.gsc'", p![]).unwrap().unwrap().s("id");
        let edited = crate::util::sha256_bytes(b"init()\n{\n  b();\n}\n");
        let hist = versions::list(&v, &fid).unwrap();
        assert!(hist.as_arr().unwrap().iter().any(|x| x.str_of("sha256").as_deref() == Some(edited.as_str())), "the replaced content is still in history");
    }

    #[test]
    fn keeps_newest_runs_and_collects_unused_objects() {
        let (_t, v, d, pid, drive) = setup();
        let w = sync(&v);
        // big.bin isn't versioned, so its old contents only live in old backups
        for i in 0..5 {
            bump(&d.join("maps/big.bin"), &format!("blob {i}"));
            crate::files::scan(&v, &pid).unwrap();
            assert_eq!(backup(&w, &quiet).unwrap().get("ok").and_then(|x| x.as_bool()), Some(true));
        }
        let root = root_of(&drive);
        assert_eq!(list_runs_in(&root).len(), 3, "keep = 3");
        // objects are only collected when a day old; age them and prune again
        let w2 = crate::files::walk(&root.join("objects"), &crate::files::Ignore::parse(""));
        for rel in w2.files.keys() {
            let f = fs::File::options().append(true).open(root.join("objects").join(rel)).unwrap();
            f.set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(3 * 86400)).unwrap();
        }
        // another run removes the oldest and collects what nothing refers to
        bump(&d.join("maps/big.bin"), "final");
        crate::files::scan(&v, &pid).unwrap();
        let r = backup(&w, &quiet).unwrap();
        assert!(r.get("freedBytes").and_then(|x| x.as_i64()).unwrap() > 0, "{}", r.pretty());
        // every remaining run is still complete
        let live = referenced(&root).unwrap();
        for sha in &live {
            assert!(obj_path(&root, sha).exists(), "object {sha} referenced but missing");
        }
        assert_eq!(verify(&w, &quiet).unwrap().get("ok").and_then(|x| x.as_bool()), Some(true));
    }

    #[test]
    fn drive_missing_or_pulled_out() {
        let (_t, v, d, _pid, drive) = setup();
        let w = sync(&v);
        fs::rename(&drive, drive.with_file_name("E-unplugged")).unwrap();
        assert!(!due(&v, false), "not due while the drive is missing");
        let r = backup(&w, &quiet).unwrap();
        assert_eq!(r.get("ok").and_then(|x| x.as_bool()), Some(false));
        assert!(r.str_of("error").unwrap().contains("isn't connected"));
        fs::rename(drive.with_file_name("E-unplugged"), &drive).unwrap();
        assert!(due(&v, true), "plugged back in: due at once, despite the failure");
        assert!(!due(&v, false), "otherwise wait before retrying");

        // pulled out halfway: no run appears, and the next one completes
        let plan = prepare(&v, &drive).unwrap();
        let pulled = std::cell::Cell::new(false);
        let r = execute(&plan, &|phase, _, _, _| {
            if phase == "Copying project files" || phase == "Checking the backup" {
                if !pulled.get() {
                    pulled.set(true);
                    fs::rename(&drive, drive.with_file_name("E-unplugged")).unwrap();
                }
            }
        });
        assert!(pulled.get());
        assert!(r.is_err(), "the run fails");
        assert!(r.unwrap_err().contains("disconnected"));
        fs::rename(drive.with_file_name("E-unplugged"), &drive).unwrap();
        assert_eq!(list_runs_in(&root_of(&drive)).len(), 0, "no half-written backup is listed");
        let _ = fs::remove_file(&plan.db_tmp);
        assert_eq!(backup(&w, &quiet).unwrap().get("ok").and_then(|x| x.as_bool()), Some(true));
        let _ = d;
    }

    #[test]
    fn unreachable_project_keeps_previous_backup() {
        let (_t, v, d, pid, drive) = setup();
        let w = sync(&v);
        backup(&w, &quiet).unwrap();
        fs::rename(&d, d.with_file_name("proj-away")).unwrap();
        let r = backup(&w, &quiet).unwrap();
        assert_eq!(r.get("carriedOver").unwrap().as_arr().unwrap().len(), 1);
        let latest = list_runs_in(&root_of(&drive)).remove(0);
        let m = read_manifest(&latest.dir).unwrap();
        assert_eq!(manifest_files(&project_in(&m, &pid).unwrap()).len(), 2, "files still in the newest backup");
    }

    #[test]
    fn same_drive_warning() {
        let (_t, v, _d, _pid, drive) = setup();
        let c = check(&v, &drive).unwrap();
        // in tests everything is on one disk
        assert!(c.get("sameDisk").unwrap().as_arr().unwrap().len() >= 2, "{}", c.pretty());
        assert_eq!(c.get("available").and_then(|x| x.as_bool()), Some(true));
    }

    #[test]
    fn restore_everything_on_a_new_pc() {
        let (t, v, d, pid, drive) = setup();
        let w = sync(&v);
        bump(&d.join("main.gsc"), "init() { two(); }\n");
        crate::files::scan(&v, &pid).unwrap();
        backup(&w, &quiet).unwrap();
        let fid = v.db.one("SELECT id FROM files WHERE rel_path='main.gsc'", p![]).unwrap().unwrap().s("id");
        let n_versions = versions::list(&v, &fid).unwrap().as_arr().unwrap().len();
        assert!(n_versions >= 2);
        let run = list_runs_in(&root_of(&drive)).remove(0).id;
        drop(w);
        drop(v);
        // the old PC is gone, project folder included
        fs::remove_dir_all(&d).unwrap();
        fs::remove_dir_all(t.path().join("data")).unwrap();

        let v2 = Vault::open(&t.path().join("newpc")).unwrap();
        let w2 = sync(&v2);
        let r = restore_all(&w2, Some(&drive), &run, &quiet).unwrap();
        assert_eq!(r.get("ok").and_then(|x| x.as_bool()), Some(true), "{}", r.pretty());
        assert!(d.join("maps/big.bin").exists() && fs::read_to_string(d.join("main.gsc")).unwrap().contains("two"));
        drop(w2);
        drop(v2);
        let v3 = Vault::open(&t.path().join("newpc")).unwrap();
        assert_eq!(v3.db.scalar_i64("SELECT count(*) FROM projects", p![]).unwrap(), 1);
        assert_eq!(versions::storage_root(&v3), t.path().join("newpc"));
        // old versions can be read back on the new PC
        let vers = versions::list(&v3, &fid).unwrap();
        assert_eq!(vers.as_arr().unwrap().len(), n_versions);
        for x in vers.as_arr().unwrap() {
            let sha = x.str_of("sha256").unwrap();
            assert!(versions::object_path(&v3, &sha).unwrap().exists(), "version content restored");
        }
        assert_eq!(settings(&v3).unwrap().str_of("dest").unwrap(), drive.to_string_lossy());
    }
}

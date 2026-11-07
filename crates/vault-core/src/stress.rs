//! Torture test for saving, versions, snapshots, backups and restores.
//!
//! A seeded random sequence of operations runs against a real project folder
//! through the same command interface the app uses (`api::dispatch`), while an
//! independent model keeps the exact bytes every file and every saved version
//! should have. After each step the invariants are checked:
//!
//! * the database's view of the folder matches the disk (after a scan)
//! * every saved version still reproduces its exact bytes
//! * a restore writes exactly the version's bytes and touches nothing else;
//!   with "keep current", the replaced content is itself restorable
//! * snapshot and backup restores bring back exactly what was captured, and
//!   never delete newer files
//! * a damaged backup is detected by verification
//! * the database passes an integrity check after reopening
//!
//! Crash mode (`--crash N`) runs the same work in a child process and kills it
//! at random moments N times; after each kill the vault must open, pass its
//! integrity check, and every version it lists must still be intact.
//!
//! Run: `vault-stress --rounds 2000 [--seed 7] [--dir D:\StressTest] [--crash 20]`
//! A failure prints the seed and round so it can be replayed exactly.

use crate::json::Json;
use crate::util::sha256_bytes;
use crate::{api, Vault};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        // xorshift64*
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
    fn chance(&mut self, pct: u64) -> bool {
        self.next() % 100 < pct
    }
    fn pick<'a, T>(&mut self, v: &'a [T]) -> Option<&'a T> {
        if v.is_empty() { None } else { Some(&v[self.below(v.len())]) }
    }
}

pub struct Opts {
    pub rounds: usize,
    pub seed: u64,
    pub dir: Option<PathBuf>,
    pub crash: usize,
    pub long_paths: bool,
    pub verbose: bool,
}

struct Tracked {
    id: String,
    file_id: String,
    bytes: Vec<u8>,
    named: bool,
}

struct World {
    v: Option<Vault>,
    data: PathBuf,
    root: PathBuf,
    scratch: PathBuf,
    pid: String,
    rng: Rng,
    /// exact bytes every file in the folder should have
    truth: BTreeMap<String, Vec<u8>>,
    /// rel path → file id (refreshed after scans)
    ids: HashMap<String, String>,
    versions: Vec<Tracked>,
    snapshots: Vec<(String, BTreeMap<String, Vec<u8>>)>,
    /// app-deleted files that can be restored: file id → (rel, bytes)
    deleted: HashMap<String, (String, Vec<u8>)>,
    stats: BTreeMap<&'static str, usize>,
    checks: usize,
    long_paths: bool,
    /// whether the game-output folder is ignored yet
    game_ignored: bool,
    /// snapshots created as temporary (they may delete themselves)
    temp_snaps: std::collections::HashSet<String>,
}

/// A folder a game writes into and deletes from on its own (like IW4X's
/// demos/). It isn't part of `truth`: what's on disk there is the truth.
const GAME: &str = "game output";

type R<T> = Result<T, String>;

fn call(v: &Vault, cmd: &str, a: Json) -> R<Json> {
    api::dispatch(v, cmd, &a).map_err(|e| format!("{cmd} failed: {e}"))
}

const NAMES: &[&str] = &[
    "notes", "Spaces in name", "ünïcödé", "日本語", "emoji 🎯", "dots.in.name", "UPPER", "a", "weapon_tuner", "file (2)",
    "-dash", "under_score", "Ελληνικά", "#hash", "100%", "plate_1", "release v1.2",
];
const EXTS: &[&str] = &["txt", "gsc", "lua", "json", "md", "stl", "3mf", "png", "bin", "blend", "cfg", ""];
const TEXT_EXTS: &[&str] = &["txt", "gsc", "lua", "json", "md", "cfg"];

fn in_game(rel: &str) -> bool {
    rel.to_lowercase().starts_with(&format!("{GAME}/"))
}

impl World {
    fn v(&self) -> &Vault {
        self.v.as_ref().expect("vault open")
    }
    fn bump(&mut self, k: &'static str) {
        *self.stats.entry(k).or_default() += 1;
    }
    fn abs(&self, rel: &str) -> PathBuf {
        self.root.join(rel)
    }

    fn new_name(&mut self) -> String {
        loop {
            let depth = self.rng.below(4);
            let mut parts: Vec<String> = (0..depth).map(|_| self.rng.pick(&["scripts", "mp", "Models", "deep dir", "ünï", "a b", "configs", "x"]).unwrap().to_string()).collect();
            if self.long_paths && self.rng.chance(5) {
                // a path well past 260 characters, from ordinary folder names (each ≤ 255)
                for i in 0..6 {
                    parts.push(format!("long folder name number {i} with some more words"));
                }
            }
            let base = self.rng.pick(NAMES).unwrap().to_string();
            let ext = self.rng.pick(EXTS).unwrap();
            let n = self.rng.below(1000);
            let file = if ext.is_empty() { format!("{base} {n}") } else { format!("{base} {n}.{ext}") };
            parts.push(file);
            let rel = parts.join("/");
            // the folder may be on a case-insensitive drive (Windows): no two paths that differ only by case,
            // and no file where a folder of that name exists (or the other way round)
            let lower = rel.to_lowercase();
            let clash = self.truth.keys().any(|k| {
                let k = k.to_lowercase();
                k == lower || lower.starts_with(&format!("{k}/")) || k.starts_with(&format!("{lower}/"))
            });
            if !clash {
                return rel;
            }
        }
    }

    fn content(&mut self, ext: &str) -> Vec<u8> {
        let size = match self.rng.below(100) {
            0..=4 => 0,
            5..=69 => 1 + self.rng.below(4096),
            70..=93 => 4096 + self.rng.below(1 << 20),
            _ => (1 << 20) + self.rng.below(7 << 20),
        };
        if TEXT_EXTS.contains(&ext) {
            let words = ["init()", "{", "}", "level.instashoot = 1;", "-- lua", "\"key\": true,", "ünïcödé", "日本語", "🎯", "\t", "\n"];
            let mut s = String::new();
            while s.len() < size {
                s.push_str(words[self.rng.below(words.len())]);
                s.push(if self.rng.chance(15) { '\n' } else { ' ' });
            }
            s.into_bytes()
        } else {
            let mut b = vec![0u8; size];
            let mut i = 0;
            while i < size {
                let r = self.rng.next().to_le_bytes();
                let n = (size - i).min(8);
                b[i..i + n].copy_from_slice(&r[..n]);
                i += n;
            }
            b
        }
    }

    fn ext_of(rel: &str) -> String {
        let name = rel.rsplit('/').next().unwrap_or(rel);
        name.rsplit_once('.').map(|x| x.1.to_lowercase()).unwrap_or_default()
    }

    fn write(&mut self, rel: &str, bytes: Vec<u8>) -> R<()> {
        let p = self.abs(rel);
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d).map_err(|e| format!("mkdir {}: {e}", d.display()))?;
        }
        std::fs::write(&p, &bytes).map_err(|e| format!("write {rel}: {e}"))?;
        self.truth.insert(rel.to_string(), bytes);
        Ok(())
    }

    fn scan(&mut self) -> R<()> {
        call(self.v(), "files.scan", Json::obj().with("projectId", self.pid.as_str()))?;
        self.refresh_ids()
    }

    fn refresh_ids(&mut self) -> R<()> {
        let list = call(self.v(), "files.list", Json::obj().with("projectId", self.pid.as_str()).with("limit", 50000))?;
        self.ids.clear();
        for f in list.as_arr().cloned().unwrap_or_default() {
            if f.str_of("state").as_deref() == Some("present") {
                self.ids.insert(f.str_of("path").unwrap_or_default(), f.str_of("id").unwrap_or_default());
            }
        }
        Ok(())
    }

    /// The database and the disk agree, and the disk matches the model.
    fn check_folder(&mut self) -> R<()> {
        self.checks += 1;
        let list = call(self.v(), "files.list", Json::obj().with("projectId", self.pid.as_str()).with("limit", 50000))?;
        let mut db: BTreeMap<String, String> = BTreeMap::new();
        for f in list.as_arr().cloned().unwrap_or_default() {
            if f.str_of("state").as_deref() == Some("present") {
                db.insert(f.str_of("path").unwrap_or_default(), f.str_of("sha256").unwrap_or_default());
            }
        }
        for (rel, bytes) in &self.truth {
            let disk = std::fs::read(self.abs(rel)).map_err(|e| format!("{rel} should exist on disk: {e}"))?;
            if &disk != bytes {
                return Err(format!("{rel}: disk content differs from what it should be ({} vs {} bytes)", disk.len(), bytes.len()));
            }
            match db.get(rel) {
                None => return Err(format!("{rel} is on disk but the database doesn't list it as present")),
                Some(sha) if !sha.is_empty() && *sha != sha256_bytes(bytes) => return Err(format!("{rel}: database checksum is stale")),
                _ => {}
            }
        }
        for rel in db.keys() {
            if !self.truth.contains_key(rel) && !in_game(rel) {
                return Err(format!("database lists {rel} as present but it shouldn't exist"));
            }
        }
        self.check_game()
    }

    /// The game-output folder: tracked like anything else until it's ignored;
    /// after that, no record of it at all (not even "missing").
    fn check_game(&mut self) -> R<()> {
        self.checks += 1;
        let rows = self.v().db.query("SELECT rel_path, state, sha256 FROM files WHERE project_id=? AND lower(rel_path) LIKE ?", crate::p![self.pid.as_str(), format!("{GAME}/%")]).map_err(|e| e.to_string())?;
        if self.game_ignored {
            if let Some(r) = rows.first() {
                return Err(format!("{GAME}/ is ignored, but {} is still recorded ({})", r.s("rel_path"), r.s("state")));
            }
            return Ok(());
        }
        let dir = self.root.join(GAME);
        let mut disk: BTreeMap<String, Vec<u8>> = BTreeMap::new();
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for e in rd.flatten() {
                if let Ok(b) = std::fs::read(e.path()) {
                    disk.insert(format!("{GAME}/{}", e.file_name().to_string_lossy()), b);
                }
            }
        }
        for (rel, b) in &disk {
            match rows.iter().find(|r| r.s("rel_path") == *rel && r.s("state") == "present") {
                None => return Err(format!("game output {rel} is on disk but not tracked as present")),
                Some(r) if r.s("sha256") != sha256_bytes(b) => return Err(format!("game output {rel}: database checksum is stale")),
                _ => {}
            }
        }
        for r in rows.iter().filter(|r| r.s("state") == "present") {
            if !disk.contains_key(&r.s("rel_path")) {
                return Err(format!("database lists game output {} as present but it's gone", r.s("rel_path")));
            }
        }
        Ok(())
    }

    /// Every tracked version still reproduces its bytes (checked without touching the folder).
    fn check_versions(&mut self, sample: usize) -> R<()> {
        let out = self.scratch.join(format!("export-{}", self.rng.next() % 1_000_000));
        std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
        let n = self.versions.len();
        let idx: Vec<usize> = if n <= sample { (0..n).collect() } else { (0..sample).map(|_| self.rng.below(n)).collect() };
        for i in idx {
            self.checks += 1;
            let t = &self.versions[i];
            let r = call(self.v(), "versions.export", Json::obj().with("id", t.id.as_str()).with("dest", out.to_string_lossy().to_string()))?;
            let p = r.str_of("path").map(PathBuf::from).ok_or("export returned no path")?;
            let got = std::fs::read(&p).map_err(|e| e.to_string())?;
            if got != t.bytes {
                return Err(format!("version {} no longer reproduces its content ({} vs {} bytes)", t.id, got.len(), t.bytes.len()));
            }
            let _ = std::fs::remove_file(&p);
        }
        let _ = std::fs::remove_dir_all(&out);
        Ok(())
    }

    fn present(&self) -> Vec<String> {
        self.truth.keys().cloned().collect()
    }

    // ------------------------------------------------------------------ operations

    fn op_create(&mut self) -> R<()> {
        let rel = self.new_name();
        let ext = Self::ext_of(&rel);
        let c = self.content(&ext);
        self.write(&rel, c)?;
        self.bump("create");
        Ok(())
    }

    fn op_modify(&mut self) -> R<()> {
        let Some(rel) = self.rng.pick(&self.present()).cloned() else { return self.op_create() };
        let ext = Self::ext_of(&rel);
        let old = self.truth[&rel].clone();
        let new = match self.rng.below(5) {
            0 => Vec::new(),                                   // truncated to nothing
            1 => old.clone(),                                  // rewritten unchanged
            2 => { let mut o = old.clone(); o.extend(self.content(&ext).into_iter().take(5000)); o } // appended
            3 => { let mut o = old.clone(); if !o.is_empty() { let i = self.rng.below(o.len()); o[i] ^= 0x5A; } o } // one byte flipped
            _ => self.content(&ext),                           // replaced
        };
        self.write(&rel, new)?;
        self.bump("modify");
        Ok(())
    }

    fn op_rename(&mut self) -> R<()> {
        let Some(rel) = self.rng.pick(&self.present()).cloned() else { return Ok(()) };
        let to = if self.rng.chance(20) {
            // case-only rename (a real edge case on Windows)
            let (dir, name) = rel.rsplit_once('/').map(|(d, n)| (format!("{d}/"), n.to_string())).unwrap_or((String::new(), rel.clone()));
            let flipped: String = name.chars().map(|c| if c.is_lowercase() { c.to_uppercase().next().unwrap_or(c) } else { c.to_lowercase().next().unwrap_or(c) }).collect();
            if flipped == name { return Ok(()); }
            format!("{dir}{flipped}")
        } else {
            let mut n = self.new_name();
            // keep the extension so file types (and versioning rules) stay the same
            let ext = Self::ext_of(&rel);
            if !ext.is_empty() && Self::ext_of(&n) != ext { n = format!("{n}.{ext}"); }
            n
        };
        let bytes = self.truth.remove(&rel).unwrap();
        let to_lower = to.to_lowercase();
        if self.truth.keys().any(|k| k.to_lowercase() == to_lower || to_lower.starts_with(&format!("{}/", k.to_lowercase())) || k.to_lowercase().starts_with(&format!("{to_lower}/"))) {
            self.truth.insert(rel, bytes);
            return Ok(());
        }
        let dest = self.abs(&to);
        if let Some(d) = dest.parent() {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        std::fs::rename(self.abs(&rel), &dest).map_err(|e| format!("rename {rel} → {to}: {e}"))?;
        self.truth.insert(to, bytes);
        self.bump("rename");
        Ok(())
    }

    fn op_external_delete(&mut self) -> R<()> {
        let Some(rel) = self.rng.pick(&self.present()).cloned() else { return Ok(()) };
        std::fs::remove_file(self.abs(&rel)).map_err(|e| format!("delete {rel}: {e}"))?;
        self.truth.remove(&rel);
        self.bump("delete outside");
        Ok(())
    }

    fn op_app_delete(&mut self) -> R<()> {
        self.scan()?;
        let Some(rel) = self.rng.pick(&self.present()).cloned() else { return Ok(()) };
        let id = self.ids.get(&rel).cloned().ok_or(format!("{rel} has no id after scan"))?;
        let r = call(self.v(), "files.delete", Json::obj().with("ids", Json::Arr(vec![Json::from(id.as_str())])))?;
        if r.get("errors").and_then(|e| e.as_arr()).map(|a| !a.is_empty()).unwrap_or(false) {
            return Err(format!("delete reported errors: {}", r.to_string()));
        }
        if self.abs(&rel).exists() {
            return Err(format!("{rel} still on disk after Mothball deleted it"));
        }
        let bytes = self.truth.remove(&rel).unwrap();
        self.deleted.insert(id, (rel, bytes));
        self.bump("delete in app");
        Ok(())
    }

    fn op_restore_deleted(&mut self) -> R<()> {
        let keys: Vec<String> = self.deleted.keys().cloned().collect();
        let Some(id) = self.rng.pick(&keys).cloned() else { return Ok(()) };
        let (rel, bytes) = self.deleted.remove(&id).unwrap();
        // a snapshot or backup restore may have brought it back already, or a new
        // file at its old path took over the record: then it isn't deleted any more
        let st = call(self.v(), "files.get", Json::obj().with("id", id.as_str()))?.str_of("state").unwrap_or_default();
        if st != "deleted" {
            return Ok(());
        }
        // something else may have taken the place meanwhile; then the restore must refuse or pick another name
        let taken = self.truth.keys().any(|k| k.to_lowercase() == rel.to_lowercase());
        match call(self.v(), "files.restoreDeleted", Json::obj().with("id", id.as_str())) {
            Ok(_) => {
                let f = call(self.v(), "files.get", Json::obj().with("id", id.as_str()))?;
                let now = f.str_of("path").unwrap_or_default();
                if taken && now.to_lowercase() == rel.to_lowercase() && self.truth.get(&rel).map(|b| b != &bytes).unwrap_or(false) {
                    return Err(format!("restoring a deleted file overwrote a newer {rel}"));
                }
                let got = std::fs::read(self.abs(&now)).map_err(|e| format!("restored {now} not on disk: {e}"))?;
                if got != bytes {
                    return Err(format!("restored deleted file {now} has the wrong content"));
                }
                self.truth.insert(now, bytes);
            }
            Err(e) if taken => { let _ = e; } // refusing is fine
            Err(e) => return Err(e),
        }
        self.bump("restore deleted");
        self.scan()
    }

    fn op_editor_save(&mut self) -> R<()> {
        self.scan()?;
        let texts: Vec<String> = self.present().into_iter().filter(|r| TEXT_EXTS.contains(&Self::ext_of(r).as_str()) && self.truth[r].len() < 2_000_000 && std::str::from_utf8(&self.truth[r]).is_ok()).collect();
        let Some(rel) = self.rng.pick(&texts).cloned() else { return Ok(()) };
        let id = self.ids.get(&rel).cloned().ok_or(format!("{rel} has no id"))?;
        let mut text = String::from_utf8(self.truth[&rel].clone()).unwrap();
        text.push_str("\n// saved in the editor ");
        text.push_str(&self.rng.next().to_string());
        let sha = sha256_bytes(&self.truth[&rel]);
        call(self.v(), "files.writeText", Json::obj().with("id", id.as_str()).with("text", text.as_str()).with("expectedSha", sha.as_str()))?;
        self.truth.insert(rel.clone(), text.into_bytes());
        // a stale save (file changed underneath) must be refused, not overwrite
        if self.rng.chance(30) {
            let r = call(self.v(), "files.writeText", Json::obj().with("id", id.as_str()).with("text", "stale").with("expectedSha", sha.as_str()));
            if r.is_ok() {
                return Err(format!("a save based on old content of {rel} was accepted and overwrote newer content"));
            }
            self.bump("stale save refused");
        }
        self.bump("editor save");
        Ok(())
    }

    fn op_backup_now(&mut self) -> R<()> {
        self.scan()?;
        let Some(rel) = self.rng.pick(&self.present()).cloned() else { return Ok(()) };
        let id = self.ids.get(&rel).cloned().ok_or(format!("{rel} has no id"))?;
        let named = self.rng.chance(60);
        let name = if named { format!("backup {}", self.rng.below(10000)) } else { String::new() };
        let r = call(self.v(), "versions.backupNow", Json::obj().with("fileId", id.as_str()).with("name", name.as_str()))?;
        // returns the file's versions, newest first
        let sha = sha256_bytes(&self.truth[&rel]);
        let vid = r.as_arr().and_then(|a| a.iter().find(|x| x.str_of("reason").as_deref() == Some("manual") && x.str_of("sha256").as_deref() == Some(sha.as_str()))).and_then(|x| x.str_of("id"))
            .ok_or(format!("backing up {rel} didn't produce a version with its current content"))?;
        // manual backups are never pruned automatically, named or not
        self.versions.push(Tracked { id: vid, file_id: id, bytes: self.truth[&rel].clone(), named: true });
        self.bump("backup a file");
        Ok(())
    }

    fn op_restore_version(&mut self) -> R<()> {
        if self.versions.is_empty() { return Ok(()); }
        self.scan()?;
        let i = self.rng.below(self.versions.len());
        let (vid, fid, bytes) = (self.versions[i].id.clone(), self.versions[i].file_id.clone(), self.versions[i].bytes.clone());
        let f = call(self.v(), "files.get", Json::obj().with("id", fid.as_str()))?;
        let rel = f.str_of("path").unwrap_or_default();
        let state = f.str_of("state").unwrap_or_default();
        if state == "deleted" { return Ok(()); } // restore from the bin first
        // only restore where the model knows the path is this file's (or free)
        let lower = rel.to_lowercase();
        if self.truth.keys().any(|k| k.to_lowercase() == lower && *k != rel) || self.truth.keys().any(|k| lower.starts_with(&format!("{}/", k.to_lowercase())) || k.to_lowercase().starts_with(&format!("{lower}/"))) {
            return Ok(());
        }
        let keep = self.rng.chance(70);
        let before = self.truth.get(&rel).cloned();
        let others: BTreeMap<String, Vec<u8>> = self.truth.iter().filter(|(k, _)| **k != rel).map(|(k, v)| (k.clone(), v.clone())).collect();
        call(self.v(), "versions.restoreOpts", Json::obj().with("id", vid.as_str()).with("keepCurrent", keep))?;
        let got = std::fs::read(self.abs(&rel)).map_err(|e| format!("restored {rel} missing: {e}"))?;
        if got != bytes {
            return Err(format!("restore of {vid} wrote the wrong content to {rel}"));
        }
        for (k, b) in &others {
            if std::fs::read(self.abs(k)).ok().as_ref() != Some(b) {
                return Err(format!("restoring {rel} changed another file, {k}"));
            }
        }
        self.truth.insert(rel.clone(), bytes.clone());
        // with "keep current", what was replaced must itself be restorable
        if keep {
            if let Some(prev) = before.filter(|p| *p != bytes) {
                let list = call(self.v(), "versions.list", Json::obj().with("fileId", fid.as_str()))?;
                let kept = list.as_arr().cloned().unwrap_or_default().into_iter()
                    .find(|x| x.str_of("reason").as_deref() == Some("before-restore") && x.str_of("sha256").as_deref() == Some(sha256_bytes(&prev).as_str()))
                    .ok_or(format!("restoring {rel} with 'keep current' didn't keep the replaced content"))?;
                self.versions.push(Tracked { id: kept.str_of("id").unwrap_or_default(), file_id: fid, bytes: prev, named: false });
                self.checks += 1;
            }
        }
        self.bump("restore a version");
        Ok(())
    }

    fn op_snapshot(&mut self) -> R<()> {
        self.scan()?;
        let name = format!("snap {}", self.rng.below(1000));
        let temp = self.rng.chance(35);
        let mut a = Json::obj().with("projectId", self.pid.as_str()).with("name", name).with("maxFileMb", 64.0);
        if temp {
            a.set("expiresAt", crate::util::now_ms() + 3_600_000);
        }
        let r = call(self.v(), "snapshots.create", a)?;
        let id = r.str_of("id").ok_or(format!("snapshot returned no id: {}", r.to_string()))?;
        if temp {
            self.temp_snaps.insert(id.clone());
        }
        self.snapshots.push((id, self.truth.clone()));
        self.bump("snapshot");
        Ok(())
    }

    fn op_restore_snapshot(&mut self) -> R<()> {
        if self.snapshots.is_empty() { return Ok(()); }
        let i = self.rng.below(self.snapshots.len());
        let (id, snap) = self.snapshots[i].clone();
        // a path the snapshot needs may now be a folder (or inside a file); the model skips those cases
        for k in snap.keys() {
            let kl = k.to_lowercase();
            if self.truth.keys().any(|t| { let tl = t.to_lowercase(); (tl != kl && (kl.starts_with(&format!("{tl}/")) || tl.starts_with(&format!("{kl}/")))) || (tl == kl && t != k) }) {
                return Ok(());
            }
        }
        let newer: BTreeMap<String, Vec<u8>> = self.truth.iter().filter(|(k, _)| !snap.contains_key(*k)).map(|(k, v)| (k.clone(), v.clone())).collect();
        call(self.v(), "snapshots.restore", Json::obj().with("id", id.as_str()))?;
        for (k, b) in &snap {
            let got = std::fs::read(self.abs(k)).map_err(|e| format!("snapshot restore didn't bring back {k}: {e}"))?;
            if &got != b {
                return Err(format!("snapshot restore: {k} has the wrong content"));
            }
            self.truth.insert(k.clone(), b.clone());
        }
        for (k, b) in &newer {
            if std::fs::read(self.abs(k)).ok().as_ref() != Some(b) {
                return Err(format!("snapshot restore changed or deleted a newer file, {k}"));
            }
        }
        self.bump("restore a snapshot");
        self.scan()
    }

    fn op_backup_roundtrip(&mut self) -> R<()> {
        self.scan()?;
        let dest = self.scratch.join("backups");
        std::fs::create_dir_all(&dest).map_err(|e| e.to_string())?;
        let zip = self.rng.chance(50);
        let r = call(self.v(), "backup.create", Json::obj().with("dest", dest.to_string_lossy().to_string()).with("projectId", self.pid.as_str()).with("includeFiles", true).with("zip", zip).with("maxFileMb", 64.0))?;
        let path = r.str_of("path").ok_or(format!("backup returned no path: {}", r.to_string()))?;
        let ver = call(self.v(), "backup.verify", Json::obj().with("path", path.as_str()))?;
        if ver.get("ok").and_then(|x| x.as_bool()) != Some(true) {
            return Err(format!("a fresh backup failed verification: {}", ver.to_string()));
        }
        // a damaged copy must be caught
        if !self.truth.is_empty() {
            let bad = self.scratch.join(format!("damaged-{}", self.rng.below(1_000_000)));
            let damaged = damage_copy(Path::new(&path), &bad, &mut self.rng)?;
            if damaged {
                let v2 = call(self.v(), "backup.verify", Json::obj().with("path", bad.to_string_lossy().to_string()));
                if v2.map(|j| j.get("ok").and_then(|x| x.as_bool()) == Some(true)).unwrap_or(false) {
                    return Err("a damaged backup passed verification".into());
                }
                self.checks += 1;
                self.bump("damaged backup caught");
            }
            let _ = std::fs::remove_dir_all(&bad);
            let _ = std::fs::remove_file(&bad);
        }
        // lose some files, change others, then restore from the backup (overwrite on)
        let snap = self.truth.clone();
        for rel in self.present() {
            if self.rng.chance(30) {
                let _ = std::fs::remove_file(self.abs(&rel));
                self.truth.remove(&rel);
            } else if self.rng.chance(20) {
                let ext = Self::ext_of(&rel);
                let c = self.content(&ext);
                self.write(&rel, c)?;
            }
        }
        call(self.v(), "backup.restoreFiles", Json::obj().with("path", path.as_str()).with("projectId", self.pid.as_str()).with("overwrite", true))?;
        for (k, b) in &snap {
            let got = std::fs::read(self.abs(k)).map_err(|e| format!("backup restore didn't bring back {k}: {e}"))?;
            if &got != b {
                return Err(format!("backup restore: {k} has the wrong content"));
            }
            self.truth.insert(k.clone(), b.clone());
        }
        let _ = std::fs::remove_dir_all(&path);
        let _ = std::fs::remove_file(&path);
        self.bump("backup + restore");
        self.scan()
    }

    fn op_prune(&mut self) -> R<()> {
        let keep = 1 + self.rng.below(3) as i64;
        call(self.v(), "versions.prune", Json::obj().with("projectId", self.pid.as_str()).with("keep", keep))?;
        // anything named must survive; drop only versions the database no longer lists
        let mut alive = std::collections::HashSet::new();
        let fids: Vec<String> = self.versions.iter().map(|t| t.file_id.clone()).collect::<std::collections::BTreeSet<_>>().into_iter().collect();
        for fid in fids {
            for x in call(self.v(), "versions.list", Json::obj().with("fileId", fid.as_str()))?.as_arr().cloned().unwrap_or_default() {
                alive.insert(x.str_of("id").unwrap_or_default());
            }
        }
        for t in &self.versions {
            if t.named && !alive.contains(&t.id) {
                return Err(format!("pruning removed a named backup ({})", t.id));
            }
        }
        self.versions.retain(|t| alive.contains(&t.id));
        self.bump("prune");
        Ok(())
    }

    fn op_reopen(&mut self) -> R<()> {
        self.v = None;
        let v = Vault::open(&self.data).map_err(|e| format!("the vault didn't reopen: {e}"))?;
        let chk = v.db.one("PRAGMA integrity_check", crate::p![]).map_err(|e| e.to_string())?.map(|r| r.vals[0].as_str().unwrap_or("").to_string()).unwrap_or_default();
        if chk != "ok" {
            return Err(format!("database integrity check failed: {chk}"));
        }
        self.v = Some(v);
        self.checks += 1;
        self.bump("reopen");
        Ok(())
    }

    /// The game writes a few new uniquely named files and deletes old ones.
    fn op_game_output(&mut self) -> R<()> {
        let dir = self.root.join(GAME);
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        for _ in 0..1 + self.rng.below(3) {
            let n = self.rng.next() % 1_000_000_000;
            let mut b = vec![0u8, 0x44, 0x4d]; // binary, like a demo
            b.extend((0..16 + self.rng.below(200)).map(|_| (self.rng.next() & 0xff) as u8));
            std::fs::write(dir.join(format!("auto_{n}.dm_13")), &b).map_err(|e| e.to_string())?;
        }
        let mut old: Vec<PathBuf> = std::fs::read_dir(&dir).map_err(|e| e.to_string())?.flatten().map(|e| e.path()).collect();
        old.sort();
        while old.len() > 6 {
            let p = old.remove(0);
            std::fs::remove_file(&p).map_err(|e| format!("game deleting {}: {e}", p.display()))?;
        }
        self.bump("game output");
        Ok(())
    }

    /// "Ignore folder" on the game output: its records go, nothing is reported missing.
    fn op_ignore_game(&mut self) -> R<()> {
        if self.game_ignored || !self.root.join(GAME).exists() {
            return Ok(());
        }
        self.scan()?;
        call(self.v(), "files.ignoreFolder", Json::obj().with("projectId", self.pid.as_str()).with("dir", GAME).with("anywhere", false))?;
        self.game_ignored = true;
        self.check_game()?;
        self.bump("ignore game folder");
        Ok(())
    }

    /// Drop in `x (1).ext`, delete `x.ext`, rename the copy onto `x.ext`: the
    /// file keeps its record (and history), and nothing is left "missing".
    fn op_rename_onto_old(&mut self) -> R<()> {
        self.scan()?;
        let Some(rel) = self.rng.pick(&self.present()).cloned() else { return Ok(()) };
        let Some(id_before) = self.ids.get(&rel).cloned() else { return Ok(()) };
        let ext = Self::ext_of(&rel);
        let copy = match rel.rsplit_once('.') {
            Some((stem, e)) if !stem.ends_with('/') && !e.contains('/') => format!("{stem} (1).{e}"),
            _ => format!("{rel} (1)"),
        };
        let cl = copy.to_lowercase();
        if self.truth.keys().any(|k| k.to_lowercase() == cl || k.to_lowercase().starts_with(&format!("{cl}/"))) {
            return Ok(());
        }
        let bytes = self.content(&ext);
        if Some(&bytes) == self.truth.get(&rel) {
            return Ok(());
        }
        self.write(&copy, bytes.clone())?;
        self.scan()?;
        std::fs::remove_file(self.abs(&rel)).map_err(|e| format!("delete {rel}: {e}"))?;
        self.truth.remove(&rel);
        self.scan()?;
        std::fs::rename(self.abs(&copy), self.abs(&rel)).map_err(|e| format!("rename {copy} → {rel}: {e}"))?;
        self.truth.remove(&copy);
        self.truth.insert(rel.clone(), bytes);
        self.scan()?;
        self.checks += 1;
        match self.ids.get(&rel) {
            Some(id) if *id == id_before => {}
            other => return Err(format!("{rel} lost its record (and history) when a copy was renamed onto it: {other:?} vs {id_before}")),
        }
        let left = self.v().db.query("SELECT state FROM files WHERE project_id=? AND rel_path=?", crate::p![self.pid.as_str(), copy.as_str()]).map_err(|e| e.to_string())?;
        if let Some(r) = left.first() {
            // only kept if it holds something of the user's (the stress test gives it nothing)
            return Err(format!("{copy} was renamed onto {rel} but is still recorded ({})", r.s("state")));
        }
        self.bump("rename onto old name");
        Ok(())
    }

    /// Temporary snapshots' time runs out: exactly those are deleted, the
    /// others stay and still restore exactly (op_restore_snapshot, later).
    fn op_expire_snapshots(&mut self) -> R<()> {
        if self.temp_snaps.is_empty() {
            return Ok(());
        }
        let ids: Vec<String> = self.temp_snaps.drain().collect();
        for id in &ids {
            self.v().db.exec("UPDATE snapshots SET expires_at=? WHERE id=?", crate::p![crate::util::now_ms() - 1, id.as_str()]).map_err(|e| e.to_string())?;
        }
        let n = crate::snapshots::expire_due(self.v()).map_err(|e| e.to_string())?;
        if n != ids.len() {
            return Err(format!("{} temporary snapshots were due, {n} deleted themselves", ids.len()));
        }
        self.snapshots.retain(|(id, _)| !ids.contains(id));
        let listed = call(self.v(), "snapshots.list", Json::obj().with("projectId", self.pid.as_str()))?;
        let have: Vec<String> = listed.as_arr().cloned().unwrap_or_default().iter().filter_map(|x| x.str_of("id")).collect();
        self.checks += 1;
        for (id, _) in &self.snapshots {
            if !have.contains(id) {
                return Err(format!("snapshot {id} was kept, but it's gone"));
            }
        }
        for id in &ids {
            if have.contains(id) {
                return Err(format!("expired snapshot {id} is still listed"));
            }
        }
        self.bump("expire snapshots");
        Ok(())
    }

    fn step(&mut self) -> R<&'static str> {
        let ops: &[(&'static str, u64)] = &[
            ("create", 16), ("modify", 18), ("rename", 7), ("delete outside", 4), ("delete in app", 3), ("restore deleted", 3),
            ("editor save", 8), ("backup a file", 10), ("restore a version", 10), ("snapshot", 3), ("restore a snapshot", 3),
            ("backup + restore", 1), ("prune", 2), ("reopen", 2), ("scan", 10),
            ("game output", 6), ("ignore game folder", 1), ("rename onto old name", 3), ("expire snapshots", 2),
        ];
        let total: u64 = ops.iter().map(|x| x.1).sum();
        let mut r = self.rng.next() % total;
        let mut which = ops[0].0;
        for (n, w) in ops {
            if r < *w { which = n; break; }
            r -= w;
        }
        match which {
            "create" => self.op_create(),
            "modify" => self.op_modify(),
            "rename" => self.op_rename(),
            "delete outside" => self.op_external_delete(),
            "delete in app" => self.op_app_delete(),
            "restore deleted" => self.op_restore_deleted(),
            "editor save" => self.op_editor_save(),
            "backup a file" => self.op_backup_now(),
            "restore a version" => self.op_restore_version(),
            "snapshot" => self.op_snapshot(),
            "restore a snapshot" => self.op_restore_snapshot(),
            "backup + restore" => self.op_backup_roundtrip(),
            "prune" => self.op_prune(),
            "reopen" => self.op_reopen(),
            "game output" => self.op_game_output(),
            "ignore game folder" => {
                // not too early, so the folder is tracked (and churns) for a while first
                if self.stats.get("game output").copied().unwrap_or(0) >= 8 { self.op_ignore_game() } else { Ok(()) }
            }
            "rename onto old name" => self.op_rename_onto_old(),
            "expire snapshots" => self.op_expire_snapshots(),
            _ => { self.bump("scan"); self.scan() }
        }?;
        Ok(which)
    }
}

/// Copies a backup (folder or .zip) to `to` and damages one project file in it.
fn damage_copy(from: &Path, to: &Path, rng: &mut Rng) -> R<bool> {
    if from.is_dir() {
        let w = crate::files::walk(from, &crate::files::Ignore::parse(""));
        let files: Vec<&String> = w.files.iter().filter(|(k, s)| k.starts_with("files/") && s.size > 0).map(|(k, _)| k).collect();
        let Some(victim) = rng.pick(&files).map(|s| s.to_string()) else { return Ok(false) };
        for rel in w.files.keys() {
            let dst = to.join(rel);
            std::fs::create_dir_all(dst.parent().unwrap()).map_err(|e| e.to_string())?;
            std::fs::copy(from.join(rel), &dst).map_err(|e| e.to_string())?;
        }
        let p = to.join(&victim);
        let mut b = std::fs::read(&p).map_err(|e| e.to_string())?;
        let i = rng.below(b.len());
        b[i] ^= 0xFF;
        std::fs::write(&p, b).map_err(|e| e.to_string())?;
        Ok(true)
    } else {
        let mut b = std::fs::read(from).map_err(|e| e.to_string())?;
        if b.len() < 200 { return Ok(false); }
        // somewhere in the stored data, past the first header
        let i = 100 + rng.below(b.len() - 200);
        b[i] ^= 0xFF;
        std::fs::write(to, b).map_err(|e| e.to_string())?;
        Ok(true)
    }
}

fn setup(dir: &Path, seed: u64, long_paths: bool) -> R<World> {
    let data = dir.join("data");
    let root = dir.join("Project ünï 🎯");
    let scratch = dir.join("scratch");
    for d in [&data, &root, &scratch] {
        std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    let v = Vault::open(&data)?;
    let existing = call(&v, "projects.list", Json::obj())?.as_arr().cloned().unwrap_or_default().into_iter().find(|p| p.str_of("name").as_deref() == Some("Stress"));
    let pid = match existing {
        Some(p) => p.str_of("id").unwrap_or_default(),
        None => call(&v, "projects.create", Json::obj().with("name", "Stress").with("category", "software").with("linkPath", root.to_string_lossy().to_string()))?.str_of("id").unwrap_or_default(),
    };
    // keep a version of every file type, so restores are exercised everywhere
    let _ = call(&v, "versions.setPolicy", Json::obj().with("projectId", pid.as_str()).with("policy", Json::obj().with("text", true).with("keyFiles", true).with("textMaxMb", 64.0).with("keyMaxMb", 64.0)));
    let game_ignored = v.db.one("SELECT ignore_globs FROM projects WHERE id=?", crate::p![pid.as_str()]).ok().flatten().map(|r| r.s("ignore_globs").contains(&format!("/{GAME}/"))).unwrap_or(false);
    let mut w = World { v: Some(v), data, root, scratch, pid, rng: Rng(seed.max(1)), truth: BTreeMap::new(), ids: HashMap::new(), versions: Vec::new(), snapshots: Vec::new(), deleted: HashMap::new(), stats: BTreeMap::new(), checks: 0, long_paths, game_ignored, temp_snaps: Default::default() };
    // pick up whatever is already in the folder (crash mode reuses it); the game's folder isn't modelled
    let walk = crate::files::walk(&w.root, &crate::files::Ignore::parse(""));
    for rel in walk.files.keys().filter(|r| !in_game(r)) {
        if let Ok(b) = std::fs::read(w.root.join(rel)) {
            w.truth.insert(rel.clone(), b);
        }
    }
    w.scan()?;
    Ok(w)
}

fn temp_dir(seed: u64) -> PathBuf {
    std::env::temp_dir().join(format!("mothball-stress-{seed}-{}", crate::util::now_ms()))
}

pub fn run(o: &Opts) -> R<()> {
    let dir = o.dir.clone().unwrap_or_else(|| temp_dir(o.seed));
    println!("Mothball stress test · seed {} · {} rounds · {}", o.seed, o.rounds, dir.display());
    let mut w = setup(&dir, o.seed, o.long_paths)?;
    let t0 = Instant::now();
    for round in 1..=o.rounds {
        let what = w.step().map_err(|e| format!("FAILED at round {round} (seed {}): {e}", o.seed))?;
        if o.verbose { println!("  {round:>5} {what}"); }
        let check_every = 7;
        if round % check_every == 0 || round == o.rounds {
            w.scan().and_then(|_| w.check_folder()).map_err(|e| format!("FAILED at round {round} (seed {}), after {what}: {e}", o.seed))?;
        }
        if round % 50 == 0 || round == o.rounds {
            w.check_versions(60).map_err(|e| format!("FAILED at round {round} (seed {}): {e}", o.seed))?;
            let size: usize = w.truth.values().map(|b| b.len()).sum();
            println!("  round {round}: {} files ({:.1} MB), {} versions tracked, {} checks passed, {:.0}s", w.truth.len(), size as f64 / 1e6, w.versions.len(), w.checks, t0.elapsed().as_secs_f64());
        }
    }
    w.check_versions(usize::MAX)?;
    w.op_reopen()?;
    println!("\nAll checks passed: {} checks over {} rounds in {:.0}s.", w.checks, o.rounds, t0.elapsed().as_secs_f64());
    for (k, n) in &w.stats {
        println!("  {k:<22} {n}");
    }
    if o.dir.is_none() {
        let _ = std::fs::remove_dir_all(&dir);
    }
    Ok(())
}

/// Child for crash mode: does the same work, forever, until it's killed.
/// Its own errors are reported but don't stop it: in crash mode the parent's
/// checks after each kill are what's being tested, and a killed worker's
/// leftovers (on Windows, a Recycle Bin helper finishing a delete) can change
/// files under the next one.
pub fn run_child(dir: &Path, seed: u64) -> R<()> {
    let mut w = setup(dir, seed, false)?;
    loop {
        if let Err(e) = w.step() {
            eprintln!("worker: {e} (re-reading the folder)");
            w = setup(dir, w.rng.next(), false)?;
        }
    }
}

/// Kills a worker at random moments and checks the vault survives each time.
pub fn crash(o: &Opts, exe: &Path) -> R<()> {
    let dir = o.dir.clone().unwrap_or_else(|| temp_dir(o.seed));
    println!("Mothball crash test · {} kills · seed {} · {}", o.crash, o.seed, dir.display());
    let mut rng = Rng(o.seed.max(1));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    for k in 1..=o.crash {
        let mut child = std::process::Command::new(exe)
            .args(["--child", "--dir", &dir.to_string_lossy(), "--seed", &(o.seed + k as u64).to_string()])
            .stdout(std::process::Stdio::null())
            .spawn()
            .map_err(|e| format!("couldn't start the worker: {e}"))?;
        let wait = Duration::from_millis(300 + rng.below(2500) as u64);
        std::thread::sleep(wait);
        if let Ok(Some(status)) = child.try_wait() {
            return Err(format!("kill {k}: the worker stopped by itself ({status}); run it with the same seed without --crash to see why"));
        }
        child.kill().map_err(|e| e.to_string())?;
        let _ = child.wait();
        // helpers the worker started (on Windows, the Recycle Bin's PowerShell) outlive it briefly
        std::thread::sleep(Duration::from_millis(if cfg!(windows) { 3000 } else { 200 }));
        // the vault must open, pass integrity checks, and every listed version must be intact
        let v = Vault::open(&dir.join("data")).map_err(|e| format!("kill {k}: the vault didn't open after the kill: {e}"))?;
        let chk = v.db.one("PRAGMA integrity_check", crate::p![]).map_err(|e| e.to_string())?.map(|r| r.vals[0].as_str().unwrap_or("").to_string()).unwrap_or_default();
        if chk != "ok" {
            return Err(format!("kill {k}: database integrity check failed: {chk}"));
        }
        let rows = v.db.query("SELECT id, sha256 FROM file_versions", crate::p![]).map_err(|e| e.to_string())?;
        for r in &rows {
            let obj = crate::versions::object_path(&v, &r.s("sha256")).map_err(|e| e.to_string())?;
            if crate::util::sha256_file(&obj).ok().as_deref() != Some(r.s("sha256").as_str()) {
                return Err(format!("kill {k}: version {} is listed but its stored copy is missing or damaged", r.s("id")));
            }
        }
        let leftovers = v.db.scalar_i64("SELECT count(*) FROM files WHERE name LIKE '.pv-tmp-%' OR name LIKE '.pv-deploy-%'", crate::p![]).unwrap_or(0);
        if leftovers > 0 {
            return Err(format!("kill {k}: a temporary file from an interrupted save was indexed as a project file"));
        }
        let pid = call(&v, "projects.list", Json::obj())?.as_arr().cloned().unwrap_or_default().into_iter().find(|p| p.str_of("name").as_deref() == Some("Stress")).and_then(|p| p.str_of("id"));
        if let Some(pid) = pid {
            call(&v, "files.scan", Json::obj().with("projectId", pid.as_str())).map_err(|e| format!("kill {k}: scanning after the kill failed: {e}"))?;
        }
        println!("  kill {k:>3} after {:>4} ms: vault opens, integrity ok, {} versions intact", wait.as_millis(), rows.len());
    }
    println!("\nThe vault survived {} kills.", o.crash);
    if o.dir.is_none() {
        let _ = std::fs::remove_dir_all(&dir);
    }
    Ok(())
}

/// Command-line entry shared by the two stress binaries.
pub fn main_with_args() -> i32 {
    let args: Vec<String> = std::env::args().collect();
    let get = |k: &str| args.iter().position(|a| a == k).and_then(|i| args.get(i + 1)).cloned();
    let o = Opts {
        rounds: get("--rounds").and_then(|x| x.parse().ok()).unwrap_or(1000),
        seed: get("--seed").and_then(|x| x.parse().ok()).unwrap_or_else(|| crate::util::now_ms() as u64 % 100_000),
        dir: get("--dir").map(PathBuf::from),
        crash: get("--crash").and_then(|x| x.parse().ok()).unwrap_or(0),
        long_paths: args.iter().any(|a| a == "--long-paths"),
        verbose: args.iter().any(|a| a == "--verbose"),
    };
    let res = if args.iter().any(|a| a == "--child") {
        run_child(o.dir.as_deref().unwrap_or(Path::new(".")), o.seed)
    } else if o.crash > 0 {
        crash(&o, &std::env::current_exe().unwrap_or_default())
    } else {
        run(&o)
    };
    match res {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("\n{e}");
            1
        }
    }
}

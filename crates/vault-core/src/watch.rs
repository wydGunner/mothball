//! Background change watching for every project, not just the open one.
//!
//! A cheap "fingerprint" of each project folder (paths, sizes and modified
//! times; no file contents are read) is taken outside the database lock. Only
//! when it differs from the last one is a real scan run, which is what writes
//! the timeline entry, so changes are logged within seconds of being made.
//!
//! The loop itself lives in the host (desktop shell / dev server) because it
//! owns the lock around the vault; this module provides the pieces.

use crate::db::DbResult;
use crate::files::{walk, Ignore};
use crate::json::Json;
use crate::util::now_ms;
use crate::{files, p, Vault};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

pub const DEFAULT_SECONDS: u64 = 15;

pub struct Target {
    pub project_id: String,
    pub root: PathBuf,
    pub ignore: String,
}

#[derive(Default)]
pub struct Watcher {
    prints: HashMap<String, u64>,
    /// Projects whose walk is slow are checked less often.
    next_due: HashMap<String, Instant>,
}

pub fn settings(v: &Vault) -> DbResult<Json> {
    let s = v.db.one("SELECT value FROM meta WHERE key='watch'", p![])?.map(|r| r.s("value")).unwrap_or_default();
    let j = Json::parse(&s).unwrap_or(Json::obj());
    Ok(Json::obj()
        .with("enabled", j.get("enabled").and_then(|x| x.as_bool()).unwrap_or(true))
        .with("seconds", j.get("seconds").and_then(|x| x.as_i64()).unwrap_or(DEFAULT_SECONDS as i64).clamp(5, 600)))
}

pub fn set_settings(v: &Vault, patch: &Json) -> DbResult<Json> {
    let mut cur = settings(v)?;
    if let Some(b) = patch.get("enabled").and_then(|x| x.as_bool()) {
        cur.set("enabled", b);
    }
    if let Some(n) = patch.get("seconds").and_then(|x| x.as_i64()) {
        cur.set("seconds", n.clamp(5, 600));
    }
    v.db.exec("INSERT OR REPLACE INTO meta(key,value) VALUES ('watch',?)", p![cur.to_string()])?;
    Ok(cur)
}

/// Time between checks.
pub fn interval(v: &Vault) -> Duration {
    let s = settings(v).ok();
    let secs = s.as_ref().and_then(|j| j.get("seconds")).and_then(|x| x.as_i64()).unwrap_or(DEFAULT_SECONDS as i64);
    Duration::from_secs(secs.max(5) as u64)
}

pub fn enabled(v: &Vault) -> bool {
    settings(v).ok().and_then(|j| j.get("enabled").and_then(|x| x.as_bool())).unwrap_or(true)
}

/// Projects to watch (read under the lock; quick).
pub fn targets(v: &Vault) -> DbResult<Vec<Target>> {
    let rows = v.db.query("SELECT id, root_path, ignore_globs FROM projects", p![])?;
    Ok(rows.iter().map(|r| Target { project_id: r.s("id"), root: PathBuf::from(r.s("root_path")), ignore: r.s("ignore_globs") }).collect())
}

/// Hash of every indexed path with its size and modified time. None when the
/// folder isn't reachable (the scan reports that when the project is opened).
pub fn fingerprint(t: &Target) -> Option<u64> {
    if !t.root.is_dir() {
        return None;
    }
    let w = walk(&t.root, &Ignore::parse(&t.ignore));
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |b: &[u8]| {
        for x in b {
            h ^= *x as u64;
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
    };
    for (rel, s) in &w.files {
        eat(rel.as_bytes());
        eat(&s.size.to_le_bytes());
        eat(&s.mtime.to_le_bytes());
        eat(b"\n");
    }
    Some(h)
}

impl Watcher {
    /// Fingerprints the due projects (call WITHOUT holding the vault lock) and
    /// returns those that changed since the last check. The first check of a
    /// project only records its fingerprint: opening a project scans it anyway.
    pub fn changed(&mut self, targets: &[Target]) -> Vec<String> {
        let mut out = Vec::new();
        let now = Instant::now();
        self.prints.retain(|id, _| targets.iter().any(|t| &t.project_id == id));
        for t in targets {
            if self.next_due.get(&t.project_id).is_some_and(|d| *d > now) {
                continue;
            }
            let started = Instant::now();
            let Some(fp) = fingerprint(t) else { continue };
            let took = started.elapsed();
            // a project that takes long to walk is checked at most every 20x its walk time
            if took > Duration::from_millis(500) {
                self.next_due.insert(t.project_id.clone(), now + took * 20);
            } else {
                self.next_due.remove(&t.project_id);
            }
            match self.prints.insert(t.project_id.clone(), fp) {
                Some(prev) if prev != fp => out.push(t.project_id.clone()),
                _ => {}
            }
        }
        out
    }

    /// Forget a project's fingerprint so the next check re-baselines it.
    pub fn forget(&mut self, project_id: &str) {
        self.prints.remove(project_id);
    }
}

/// Scans one project after the watcher saw a change (call WITH the lock).
/// Returns whether the scan recorded any change.
pub fn scan_changed(v: &Vault, project_id: &str) -> bool {
    match files::scan(v, project_id) {
        Ok(r) => {
            let changed = r.changes() > 0;
            if changed {
                let _ = bump(v);
            }
            changed
        }
        Err(_) => false,
    }
}

/// A counter the UI polls to learn that something changed in the background.
pub fn bump(v: &Vault) -> DbResult<()> {
    v.db.exec("INSERT OR REPLACE INTO meta(key,value) VALUES ('change_stamp',?)", p![now_ms().to_string()])?;
    Ok(())
}

pub fn stamp(v: &Vault) -> DbResult<Json> {
    let s = v.db.one("SELECT value FROM meta WHERE key='change_stamp'", p![])?.map(|r| r.s("value")).unwrap_or_default();
    Ok(Json::obj().with("stamp", s.parse::<i64>().unwrap_or(0)))
}

/// One full watcher pass, for hosts that keep the vault behind a mutex.
/// `with` runs a closure with the vault locked (None when the vault isn't open).
pub fn pass<F>(w: &mut Watcher, with: F) -> Duration
where
    F: Fn(&mut dyn FnMut(&Vault)),
{
    let mut targets = Vec::new();
    let mut on = true;
    let mut wait = Duration::from_secs(DEFAULT_SECONDS);
    with(&mut |v: &Vault| {
        on = enabled(v);
        wait = interval(v);
        if on {
            targets = targets_or_empty(v);
        }
    });
    if !on {
        return wait;
    }
    let changed = w.changed(&targets);
    for id in changed {
        with(&mut |v: &Vault| {
            scan_changed(v, &id);
        });
    }
    wait
}

fn targets_or_empty(v: &Vault) -> Vec<Target> {
    targets(v).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;

    #[test]
    fn watcher_scans_only_changed_projects() {
        let (t, v) = vault();
        let a = t.path().join("a");
        let b = t.path().join("b");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        std::fs::write(a.join("x.txt"), "1").unwrap();
        std::fs::write(b.join("y.txt"), "1").unwrap();
        let pa = crate::projects::create(&v, crate::projects::NewProject { name: "A".into(), description: "".into(), category: "general".into(), status: "".into(), link_path: Some(a.clone()), module: Json::obj() }).unwrap().str_of("id").unwrap();
        let pb = crate::projects::create(&v, crate::projects::NewProject { name: "B".into(), description: "".into(), category: "general".into(), status: "".into(), link_path: Some(b.clone()), module: Json::obj() }).unwrap().str_of("id").unwrap();
        files::scan(&v, &pa).unwrap();
        files::scan(&v, &pb).unwrap();
        let mut w = Watcher::default();
        let ts = targets(&v).unwrap();
        assert!(w.changed(&ts).is_empty(), "first pass only records");
        assert!(w.changed(&ts).is_empty(), "nothing changed");
        std::fs::write(b.join("new.txt"), "n").unwrap();
        assert_eq!(w.changed(&ts), vec![pb.clone()]);
        let before = stamp(&v).unwrap().get("stamp").unwrap().as_i64().unwrap();
        assert!(scan_changed(&v, &pb));
        assert!(stamp(&v).unwrap().get("stamp").unwrap().as_i64().unwrap() > before);
        let tl = crate::timeline::list(&v, &pb, 5, None).unwrap();
        assert!(tl.as_arr().unwrap()[0].str_of("summary").unwrap().contains("1 added"));
        // the full pass works through a lock-style callback
        let m = std::sync::Mutex::new(v);
        std::fs::write(a.join("x.txt"), "22").unwrap();
        pass(&mut w, |f| f(&m.lock().unwrap()));
        let v = m.into_inner().unwrap();
        let tl = crate::timeline::list(&v, &pa, 5, None).unwrap();
        assert!(tl.as_arr().unwrap()[0].str_of("summary").unwrap().contains("modified"));
        // switched off: nothing is scanned
        set_settings(&v, &Json::obj().with("enabled", false)).unwrap();
        assert!(!enabled(&v));
    }
}

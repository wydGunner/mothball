//! "Undo these changes": puts the files from one batch of detected changes
//! (a "File changes detected…" activity entry) back the way they were.
//!
//! Only what can be done exactly is done: a changed file goes back to its
//! stored earlier copy, an added file is removed (a copy is kept and it goes
//! to the Recycle Bin, as when deleting in Mothball), a file that went missing
//! is brought back. A file that has changed again since, or whose earlier
//! content wasn't kept, is left alone and listed with the reason. Before
//! anything is touched, a temporary snapshot of the whole project is taken,
//! so the undo itself can be undone.

use crate::db::DbResult;
use crate::json::Json;
use crate::p;
use crate::util::{iso_utc, now_ms, safe_join, sha256_file};
use crate::{files, projects, snapshots, timeline, versions, Vault};

/// How long the "before undo" snapshot is kept unless you keep it.
const SNAPSHOT_DAYS: i64 = 7;

struct Item {
    file_id: String,
    path: String,
    change: String,
    /// restore | remove | bring-back | skip
    action: &'static str,
    version_id: Option<String>,
    reason: String,
}

fn plan_items(v: &Vault, entry_id: &str) -> DbResult<(Json, Vec<Item>)> {
    let e = v.db.one("SELECT * FROM timeline WHERE id=?", p![entry_id])?.ok_or("Activity entry not found")?;
    let data = Json::parse(&e.s("data")).unwrap_or(Json::Null);
    let changes = data.get("changes").and_then(|c| c.as_arr()).cloned().unwrap_or_default();
    if changes.is_empty() {
        return Err("This entry doesn't list any file changes".into());
    }
    let root = projects::root_of(v, &e.s("project_id"))?;
    let mut items = Vec::new();
    for c in &changes {
        let fid = c.str_of("fileId").unwrap_or_default();
        let path = c.str_of("path").unwrap_or_default();
        let kind = c.str_of("change").unwrap_or_default();
        let mut it = Item { file_id: fid.clone(), path: path.clone(), change: kind.clone(), action: "skip", version_id: None, reason: String::new() };
        let Some(f) = v.db.one("SELECT * FROM files WHERE id=?", p![fid.as_str()])? else {
            it.reason = "No longer tracked".into();
            items.push(it);
            continue;
        };
        let disk = safe_join(&root, &f.s("rel_path")).ok();
        let cur = disk.as_ref().filter(|p| p.is_file()).and_then(|p| sha256_file(p).ok());
        let version_with = |sha: &str| -> DbResult<Option<String>> {
            Ok(v.db.one("SELECT id FROM file_versions WHERE file_id=? AND sha256=? ORDER BY created_at DESC LIMIT 1", p![fid.as_str(), sha])?.map(|r| r.s("id")))
        };
        match kind.as_str() {
            "modified" => {
                let (from, to) = (c.str_of("from").unwrap_or_default(), c.str_of("to").unwrap_or_default());
                if cur.is_none() {
                    it.reason = "Not on disk now".into();
                } else if cur.as_deref() != Some(to.as_str()) {
                    it.reason = "Changed again since; check its History".into();
                } else if from.is_empty() {
                    it.reason = "What it looked like before wasn't recorded".into();
                } else if let Some(vid) = version_with(&from)? {
                    it.action = "restore";
                    it.version_id = Some(vid);
                } else {
                    it.reason = "No earlier copy was kept (only text files are copied automatically)".into();
                }
            }
            "added" => {
                let added = v.db.one("SELECT sha256 FROM file_events WHERE file_id=? AND kind='added' ORDER BY at DESC, rowid DESC LIMIT 1", p![fid.as_str()])?.and_then(|r| r.os("sha256"));
                if cur.is_none() {
                    it.reason = "Already gone".into();
                } else if added.is_some() && cur != added {
                    it.reason = "Changed since it was added; check its History".into();
                } else {
                    it.action = "remove";
                }
            }
            "missing" => {
                if f.s("state") == "present" || cur.is_some() {
                    it.reason = "Already back".into();
                } else if let Some(vid) = version_with(&f.s("sha256"))? {
                    it.action = "bring-back";
                    it.version_id = Some(vid);
                } else {
                    it.reason = "No copy of it was kept; check the Recycle Bin".into();
                }
            }
            "moved" => it.reason = "Renames and moves aren't undone here".into(),
            "restored" => it.reason = "It came back on its own: nothing to undo".into(),
            other => it.reason = format!("Can't undo “{other}”"),
        }
        items.push(it);
    }
    let total = data.get("total").and_then(|x| x.as_i64()).unwrap_or(changes.len() as i64);
    let head = Json::obj().with("summary", e.s("summary")).with("at", e.i("at")).with("listed", changes.len() as i64).with("total", total).with("projectId", e.s("project_id"));
    Ok((head, items))
}

fn item_json(it: &Item) -> Json {
    Json::obj().with("fileId", it.file_id.as_str()).with("path", it.path.as_str()).with("change", it.change.as_str()).with("action", it.action).with("reason", it.reason.as_str())
}

/// What undoing this entry would do, file by file (nothing is changed).
pub fn plan(v: &Vault, entry_id: &str) -> DbResult<Json> {
    let (head, items) = plan_items(v, entry_id)?;
    Ok(head.with("items", Json::Arr(items.iter().map(item_json).collect())))
}

/// Undo: snapshot the project, then do every step in the plan that can be
/// done exactly. Returns what was done, what was skipped (with reasons) and
/// any errors, plus the snapshot to go back to.
pub fn apply(v: &Vault, entry_id: &str) -> DbResult<Json> {
    let (head, items) = plan_items(v, entry_id)?;
    let todo: Vec<&Item> = items.iter().filter(|i| i.action != "skip").collect();
    if todo.is_empty() {
        return Err("Nothing in this entry can be undone".into());
    }
    let pid = head.str_of("projectId").unwrap_or_default();
    let at = head.get("at").and_then(|x| x.as_i64()).unwrap_or(0);
    let snap = snapshots::create_expiring(
        v,
        &pid,
        &format!("Before undoing changes from {}", iso_utc(at)),
        "Taken automatically before “Undo these changes”. Restore it to redo them.",
        200.0,
        Some(now_ms() + SNAPSHOT_DAYS * 86_400_000),
    )?;
    let (mut done, mut errors) = (Vec::new(), Vec::new());
    let mut removes = Vec::new();
    for it in &todo {
        match it.action {
            "restore" | "bring-back" => match versions::restore_quiet(v, it.version_id.as_deref().unwrap_or_default()) {
                Ok(_) => {
                    // the change it was flagged for is undone: nothing left to review
                    v.db.exec("UPDATE files SET changed_since_review=0 WHERE id=?", p![it.file_id.as_str()])?;
                    done.push(item_json(it));
                }
                Err(e) => errors.push(Json::obj().with("path", it.path.as_str()).with("error", e)),
            },
            "remove" => removes.push(it.file_id.clone()),
            _ => {}
        }
    }
    if !removes.is_empty() {
        let r = files::delete(v, &removes)?;
        for e in r.get("errors").and_then(|x| x.as_arr()).cloned().unwrap_or_default() {
            errors.push(Json::obj().with("error", e));
        }
        let gone: Vec<String> = r.get("deleted").and_then(|x| x.as_arr()).cloned().unwrap_or_default().iter().filter_map(|x| x.as_str().map(String::from)).collect();
        for it in todo.iter().filter(|i| i.action == "remove" && gone.contains(&i.path)) {
            done.push(item_json(it));
        }
    }
    files::scan(v, &pid)?;
    let skipped: Vec<Json> = items.iter().filter(|i| i.action == "skip").map(item_json).collect();
    let n = done.len();
    timeline::add(
        v,
        &pid,
        "version",
        &format!("Undid {n} change{} from “{}”", if n == 1 { "" } else { "s" }, head.str_of("summary").unwrap_or_default()),
        &if skipped.is_empty() { String::new() } else { format!("{} left as they are", skipped.len()) },
        None,
        false,
    )?;
    Ok(Json::obj()
        .with("done", Json::Arr(done))
        .with("skipped", Json::Arr(skipped))
        .with("errors", Json::Arr(errors))
        .with("snapshotId", snap.str_of("id").unwrap_or_default())
        .with("snapshotName", snap.str_of("name").unwrap_or_default()))
}

#[cfg(test)]
mod tests {
    use crate::json::Json;
    use crate::p;

    /// User request: "restore these files" on a File changes detected entry.
    #[test]
    fn undo_a_batch_of_changes_exactly_and_redo_it_from_the_snapshot() {
        let (t, v) = crate::testutil::vault();
        let d = t.0.join("Downtime");
        std::fs::create_dir_all(d.join("Games")).unwrap();
        let w = |rel: &str, b: &[u8]| std::fs::write(d.join(rel), b).unwrap();
        w("Games/Slots.lua", b"slots v1\n");
        w("Games/Holdem.lua", b"holdem v1\n");
        w("Core/Init.lua".replace("Core/", "").as_str(), b"init v1\n");
        w("thumb.tga", &[0, 1, 2, 3]);
        let pid = crate::api::dispatch(&v, "projects.create", &Json::obj().with("name", "Downtime").with("linkPath", d.to_string_lossy().to_string())).unwrap().str_of("id").unwrap();
        crate::files::scan(&v, &pid).unwrap();
        // the batch: 2 changed, a binary changed (no copy kept), 2 added, 1 deleted
        w("Games/Slots.lua", b"slots v2 (LLM)\n");
        w("Games/Holdem.lua", b"holdem v2 (LLM)\n");
        w("thumb.tga", &[9, 9, 9, 9, 9]);
        w("Games/Roulette.lua", b"roulette\n");
        w("Blackjack.tga", &[0, 7, 7]);
        std::fs::remove_file(d.join("Init.lua")).unwrap();
        crate::files::scan(&v, &pid).unwrap();
        let entry = crate::timeline::list(&v, &pid, 5, None).unwrap().as_arr().unwrap().iter().find(|e| e.str_of("summary").unwrap_or_default().starts_with("File changes detected")).cloned().unwrap();
        let eid = entry.str_of("id").unwrap();
        // Holdem changed again afterwards: left alone
        w("Games/Holdem.lua", b"holdem v3 (by hand)\n");
        crate::files::scan(&v, &pid).unwrap();

        let plan = crate::api::dispatch(&v, "timeline.undoPlan", &Json::obj().with("id", eid.as_str())).unwrap();
        let act = |path: &str| plan.get("items").unwrap().as_arr().unwrap().iter().find(|i| i.str_of("path").as_deref() == Some(path)).map(|i| i.str_of("action").unwrap()).unwrap();
        assert_eq!(act("Games/Slots.lua"), "restore");
        assert_eq!(act("Games/Holdem.lua"), "skip");
        assert_eq!(act("thumb.tga"), "skip");
        assert_eq!(act("Games/Roulette.lua"), "remove");
        assert_eq!(act("Blackjack.tga"), "remove");
        assert_eq!(act("Init.lua"), "bring-back");
        assert!(d.join("Games/Roulette.lua").exists(), "planning changes nothing");

        let r = crate::api::dispatch(&v, "timeline.undo", &Json::obj().with("id", eid.as_str())).unwrap();
        assert_eq!(r.get("errors").unwrap().as_arr().unwrap().len(), 0, "{}", r.to_string());
        assert_eq!(r.get("done").unwrap().as_arr().unwrap().len(), 4);
        assert_eq!(std::fs::read(d.join("Games/Slots.lua")).unwrap(), b"slots v1\n");
        assert_eq!(std::fs::read(d.join("Games/Holdem.lua")).unwrap(), b"holdem v3 (by hand)\n", "changed since: untouched");
        assert_eq!(std::fs::read(d.join("thumb.tga")).unwrap(), [9, 9, 9, 9, 9], "no copy: untouched");
        assert!(!d.join("Games/Roulette.lua").exists() && !d.join("Blackjack.tga").exists());
        assert_eq!(std::fs::read(d.join("Init.lua")).unwrap(), b"init v1\n");
        // the added files' copies are kept, like any delete in Mothball
        let st: String = v.db.one("SELECT state FROM files WHERE rel_path='Games/Roulette.lua'", p![]).unwrap().unwrap().s("state");
        assert_eq!(st, "deleted");
        // the undo isn't reported as changes made outside Mothball
        assert_eq!(v.db.scalar_i64("SELECT COUNT(*) FROM files WHERE project_id=? AND changed_since_review=1 AND rel_path IN ('Games/Slots.lua','Init.lua')", p![pid.as_str()]).unwrap(), 0);
        // one activity entry for the whole undo
        let tl = crate::timeline::list(&v, &pid, 3, None).unwrap();
        assert!(tl.as_arr().unwrap()[0].str_of("summary").unwrap().starts_with("Undid 4 changes"), "{}", tl.to_string());

        // changed your mind: the snapshot taken before the undo redoes it
        let snap = r.str_of("snapshotId").unwrap();
        let s = crate::snapshots::get(&v, &snap).unwrap();
        assert!(s.get("expiresAt").and_then(|x| x.as_i64()).is_some(), "temporary");
        crate::snapshots::restore(&v, &snap).unwrap();
        assert_eq!(std::fs::read(d.join("Games/Slots.lua")).unwrap(), b"slots v2 (LLM)\n");
        assert_eq!(std::fs::read(d.join("Games/Roulette.lua")).unwrap(), b"roulette\n");
    }
}

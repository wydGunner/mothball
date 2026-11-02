//! Time spent per project.
//!
//! Two sources, never double-counted:
//! * Tracked time: manual entries and the start/stop timer.
//! * Estimated time from file activity: saves close together (within
//!   `GAP_MIN` minutes) form one work session, plus `LEAD_MIN` minutes before
//!   its first save. This is what covers work in other programs (Blender,
//!   Vegas, a slicer, an IDE), which the app can't observe directly.
//!
//! Where the two overlap, the overlap counts once.

use crate::db::DbResult;
use crate::json::Json;
use crate::util::{new_id, now_ms};
use crate::{p, Vault};

pub const GAP_MIN: i64 = 30;
pub const LEAD_MIN: i64 = 10;
const MIN: i64 = 60_000;

fn settings(v: &Vault) -> DbResult<bool> {
    let s = v.db.one("SELECT value FROM meta WHERE key='time_estimate'", p![])?.map(|r| r.s("value")).unwrap_or_default();
    Ok(s != "off")
}

pub fn set_estimate(v: &Vault, on: bool) -> DbResult<()> {
    v.db.exec("INSERT OR REPLACE INTO meta(key,value) VALUES ('time_estimate',?)", p![if on { "on" } else { "off" }])?;
    Ok(())
}

/// The running timer, if any: {projectId, start, note}.
pub fn timer(v: &Vault) -> DbResult<Json> {
    let s = v.db.one("SELECT value FROM meta WHERE key='timer'", p![])?.map(|r| r.s("value")).unwrap_or_default();
    Ok(Json::parse(&s).unwrap_or(Json::Null))
}

/// Starts the timer for a project (stopping one that runs for another project).
pub fn start(v: &Vault, project_id: &str, note: &str) -> DbResult<Json> {
    v.db.one("SELECT id FROM projects WHERE id=?", p![project_id])?.ok_or("Project not found")?;
    if let Some(cur) = timer(v)?.str_of("projectId") {
        if cur == project_id {
            return timer(v);
        }
        stop(v)?;
    }
    let t = Json::obj().with("projectId", project_id).with("start", now_ms()).with("note", note);
    v.db.exec("INSERT OR REPLACE INTO meta(key,value) VALUES ('timer',?)", p![t.to_string()])?;
    Ok(t)
}

/// Stops the timer and records the entry (under a minute is discarded).
pub fn stop(v: &Vault) -> DbResult<Json> {
    let t = timer(v)?;
    v.db.exec("DELETE FROM meta WHERE key='timer'", p![])?;
    let (Some(pid), Some(start)) = (t.str_of("projectId"), t.get("start").and_then(|x| x.as_i64())) else { return Ok(Json::Null) };
    let end = now_ms();
    if end - start < MIN || v.db.one("SELECT id FROM projects WHERE id=?", p![pid.clone()])?.is_none() {
        return Ok(Json::Null);
    }
    add(v, &pid, start, end, &t.str_of("note").unwrap_or_default(), "timer")
}

pub fn add(v: &Vault, project_id: &str, start: i64, end: i64, note: &str, source: &str) -> DbResult<Json> {
    if end <= start {
        return Err("The end must be after the start".into());
    }
    if end - start > 24 * 60 * MIN {
        return Err("One entry can be at most 24 hours".into());
    }
    let id = new_id();
    v.db.exec(
        "INSERT INTO time_entries(id,project_id,start_at,end_at,note,source,created_at) VALUES (?,?,?,?,?,?,?)",
        p![id.clone(), project_id, start, end, note.trim(), source, now_ms()],
    )?;
    Ok(Json::obj().with("id", id).with("start", start).with("end", end))
}

pub fn delete(v: &Vault, id: &str) -> DbResult<()> {
    v.db.exec("DELETE FROM time_entries WHERE id=?", p![id])?;
    Ok(())
}

/// File-activity sessions for a project: (start, end) pairs.
fn activity_sessions(v: &Vault, project_id: &str, since: i64) -> DbResult<Vec<(i64, i64)>> {
    let rows = v.db.query(
        "SELECT COALESCE(CASE WHEN mtime BETWEEN at - 2592000000 AND at THEN mtime END, at) AS t FROM file_events
         WHERE project_id=? AND kind IN ('added','modified','moved','renamed','restored-version','imported') ORDER BY t",
        p![project_id],
    )?;
    let mut ts: Vec<i64> = rows.iter().map(|r| r.i("t")).filter(|t| *t >= since - GAP_MIN * MIN).collect();
    ts.sort_unstable();
    let mut out: Vec<(i64, i64)> = Vec::new();
    for t in ts {
        match out.last_mut() {
            Some(s) if t - s.1 <= GAP_MIN * MIN => s.1 = s.1.max(t),
            _ => out.push((t - LEAD_MIN * MIN, t)),
        }
    }
    Ok(out)
}

fn merge(mut iv: Vec<(i64, i64, &'static str)>) -> Vec<(i64, i64, Vec<&'static str>)> {
    iv.sort_by_key(|x| x.0);
    let mut out: Vec<(i64, i64, Vec<&'static str>)> = Vec::new();
    for (a, b, src) in iv {
        match out.last_mut() {
            Some(m) if a <= m.1 => {
                m.1 = m.1.max(b);
                if !m.2.contains(&src) {
                    m.2.push(src);
                }
            }
            _ => out.push((a, b, vec![src])),
        }
    }
    out
}

/// Totals and sessions for a project (or every project when None).
pub fn summary(v: &Vault, project_id: Option<&str>, since: i64) -> DbResult<Json> {
    let estimate = settings(v)?;
    let pids: Vec<String> = match project_id {
        Some(p) => vec![p.to_string()],
        None => v.db.query("SELECT id FROM projects", p![])?.iter().map(|r| r.s("id")).collect(),
    };
    let mut all = Vec::new();
    let mut total_all = 0;
    let mut sessions = Vec::new();
    let mut entries = Vec::new();
    for pid in &pids {
        let mut iv: Vec<(i64, i64, &'static str)> = Vec::new();
        for r in v.db.query("SELECT * FROM time_entries WHERE project_id=? ORDER BY start_at", p![pid.clone()])? {
            iv.push((r.i("start_at"), r.i("end_at"), if r.s("source") == "timer" { "timer" } else { "manual" }));
            if r.i("end_at") >= since {
                entries.push(Json::obj().with("id", r.s("id")).with("projectId", pid.as_str()).with("start", r.i("start_at")).with("end", r.i("end_at")).with("note", r.s("note")).with("source", r.s("source")));
            }
        }
        if estimate {
            for (a, b) in activity_sessions(v, pid, 0)? {
                iv.push((a, b, "activity"));
            }
        }
        let merged = merge(iv);
        let total: i64 = merged.iter().map(|m| m.1 - m.0).sum();
        total_all += total;
        for m in merged.iter().filter(|m| m.1 >= since) {
            sessions.push(
                Json::obj()
                    .with("projectId", pid.as_str())
                    .with("start", m.0)
                    .with("end", m.1)
                    .with("sources", Json::Arr(m.2.iter().map(|s| Json::from(*s)).collect())),
            );
        }
        all.push(Json::obj().with("projectId", pid.as_str()).with("totalMs", total));
    }
    sessions.sort_by_key(|s| std::cmp::Reverse(s.get("start").and_then(|x| x.as_i64()).unwrap_or(0)));
    Ok(Json::obj()
        .with("estimate", estimate)
        .with("totalMs", total_all)
        .with("projects", Json::Arr(all))
        .with("sessions", Json::Arr(sessions))
        .with("entries", Json::Arr(entries))
        .with("timer", timer(v)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;

    #[test]
    fn sessions_from_activity_and_entries_count_once() {
        let (_t, v) = vault();
        let pid = crate::projects::create(&v, crate::projects::NewProject { name: "P".into(), description: "".into(), category: "general".into(), status: "".into(), link_path: None, module: Json::obj() }).unwrap().str_of("id").unwrap();
        v.db.exec("INSERT INTO files(id,project_id,rel_path,name,ext,kind,size,mtime,state,first_seen,last_seen,last_changed) VALUES ('f',?,'a.blend','a.blend','blend','model',1,0,'present',0,0,0)", p![pid.clone()]).unwrap();
        let h = 3_600_000;
        let base = 1_000 * h;
        // saves at 0, 20 and 40 min -> one session (10 min lead + 40 min)
        // a save 3 hours later (detected 2 days later, but its file time is used) -> a second session
        for (at, mtime) in [(base, None), (base + 20 * MIN, None), (base + 40 * MIN, None), (base + 50 * h, Some(base + 3 * h))] {
            v.db.exec("INSERT INTO file_events(id,project_id,file_id,kind,detail,at,mtime) VALUES (?,?,'f','modified','',?,?)", p![new_id(), pid.clone(), at, mtime]).unwrap();
        }
        let s = summary(&v, Some(&pid), 0).unwrap();
        assert_eq!(s.get("totalMs").unwrap().as_i64(), Some(50 * MIN + 10 * MIN));
        // a manual hour overlapping the first session adds only the part outside it
        add(&v, &pid, base, base + h, "modelling", "manual").unwrap();
        let s = summary(&v, Some(&pid), 0).unwrap();
        assert_eq!(s.get("totalMs").unwrap().as_i64(), Some(70 * MIN + 10 * MIN));
        let first = s.get("sessions").unwrap().as_arr().unwrap().last().unwrap().clone();
        assert_eq!(first.get("sources").unwrap().as_arr().unwrap().len(), 2);
        // estimates can be switched off
        set_estimate(&v, false).unwrap();
        assert_eq!(summary(&v, Some(&pid), 0).unwrap().get("totalMs").unwrap().as_i64(), Some(h));
        // timer
        start(&v, &pid, "").unwrap();
        assert_eq!(timer(&v).unwrap().str_of("projectId").unwrap(), pid);
        assert!(matches!(stop(&v).unwrap(), Json::Null), "under a minute is dropped");
        assert!(add(&v, &pid, 5, 1, "", "manual").is_err());
    }
}

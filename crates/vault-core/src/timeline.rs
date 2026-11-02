//! Project activity timeline. Filesystem activity is written as one grouped
//! entry per scan, never one per file, so the timeline stays readable.

use crate::db::DbResult;
use crate::json::Json;
use crate::util::{new_id, now_ms};
use crate::{p, search, Vault};

pub fn add(
    v: &Vault,
    project_id: &str,
    kind: &str,
    summary: &str,
    detail: &str,
    reference: Option<(&str, &str)>,
    manual: bool,
) -> DbResult<String> {
    add_with_data(v, project_id, kind, summary, detail, reference, manual, None)
}

#[allow(clippy::too_many_arguments)]
pub fn add_with_data(
    v: &Vault,
    project_id: &str,
    kind: &str,
    summary: &str,
    detail: &str,
    reference: Option<(&str, &str)>,
    manual: bool,
    data: Option<&Json>,
) -> DbResult<String> {
    let id = new_id();
    let (rt, ri) = reference.map(|(a, b)| (Some(a.to_string()), Some(b.to_string()))).unwrap_or((None, None));
    let data = data.map(|d| d.to_string()).unwrap_or_default();
    v.db.exec(
        "INSERT INTO timeline(id,project_id,kind,summary,detail,ref_type,ref_id,manual,at,data) VALUES (?,?,?,?,?,?,?,?,?,?)",
        p![id.clone(), project_id, kind, summary, detail, rt, ri, manual, now_ms(), data],
    )?;
    if manual {
        search::index(v, "journal", &id, project_id, summary, detail)?;
    }
    Ok(id)
}

/// Note/file-doc edits within 30 minutes of a previous entry for the same
/// thing are folded into that entry instead of creating a new one.
pub fn add_coalesced(v: &Vault, project_id: &str, kind: &str, summary: &str, rt: &str, ri: &str) -> DbResult<()> {
    let recent = v.db.one(
        "SELECT id FROM timeline WHERE project_id=? AND kind=? AND ref_type=? AND ref_id=? AND at>? ORDER BY at DESC LIMIT 1",
        p![project_id, kind, rt, ri, now_ms() - 30 * 60 * 1000],
    )?;
    match recent {
        Some(r) => {
            v.db.exec("UPDATE timeline SET at=?, summary=? WHERE id=?", p![now_ms(), summary, r.s("id")])?;
        }
        None => {
            add(v, project_id, kind, summary, "", Some((rt, ri)), false)?;
        }
    }
    Ok(())
}

/// An edit made in Mothball's editor. Saves of the same file within 30
/// minutes are folded into one entry that spans from the content before the
/// first save to the content after the last one — but only while nothing
/// else changed the file in between; otherwise a new entry starts.
pub fn add_file_edit(v: &Vault, project_id: &str, file_id: &str, rel_path: &str, before_sha: &str, after_sha: &str) -> DbResult<()> {
    let summary = format!("Edited {rel_path} in Mothball");
    let recent = v.db.one(
        "SELECT id, data FROM timeline WHERE project_id=? AND kind='edit' AND ref_type='file' AND ref_id=? AND at>? ORDER BY at DESC LIMIT 1",
        p![project_id, file_id, now_ms() - 30 * 60 * 1000],
    )?;
    if let Some(r) = recent {
        let d = Json::parse(&r.s("data")).unwrap_or(Json::Null);
        if d.str_of("to").as_deref() == Some(before_sha) {
            let saves = d.get("saves").and_then(|x| x.as_i64()).unwrap_or(1) + 1;
            let nd = d.clone().with("to", after_sha).with("path", rel_path).with("saves", saves);
            v.db.exec("UPDATE timeline SET at=?, summary=?, data=? WHERE id=?", p![now_ms(), summary, nd.to_string(), r.s("id")])?;
            return Ok(());
        }
    }
    let d = Json::obj().with("fileId", file_id).with("path", rel_path).with("from", before_sha).with("to", after_sha).with("saves", 1);
    add_with_data(v, project_id, "edit", &summary, "", Some(("file", file_id)), false, Some(&d))?;
    Ok(())
}

pub fn list(v: &Vault, project_id: &str, limit: i64, before: Option<i64>) -> DbResult<Json> {
    let rows = v.db.query(
        "SELECT * FROM timeline WHERE project_id=? AND at<? ORDER BY at DESC LIMIT ?",
        p![project_id, before.unwrap_or(i64::MAX), limit.clamp(1, 500)],
    )?;
    Ok(Json::Arr(
        rows.iter()
            .map(|r| {
                Json::obj()
                    .with("id", r.s("id"))
                    .with("kind", r.s("kind"))
                    .with("summary", r.s("summary"))
                    .with("detail", r.s("detail"))
                    .with("refType", r.os("ref_type"))
                    .with("refId", r.os("ref_id"))
                    .with("manual", r.b("manual"))
                    .with("at", r.i("at"))
                    .with("data", Json::parse(&r.s("data")).unwrap_or(Json::Null))
            })
            .collect(),
    ))
}

pub fn delete_manual(v: &Vault, id: &str) -> DbResult<()> {
    let n = v.db.exec("DELETE FROM timeline WHERE id=? AND manual=1", p![id])?;
    if n == 0 {
        return Err("Only manual journal entries can be deleted".into());
    }
    search::unindex(v, "journal", id)
}

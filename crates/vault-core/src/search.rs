//! Universal search over an FTS5 index. Each searchable entity has exactly
//! one row (entity_type, entity_id) that is replaced on every change.

use crate::db::DbResult;
use crate::json::Json;
use crate::{p, Vault};

pub fn index(v: &Vault, et: &str, id: &str, project_id: &str, title: &str, body: &str) -> DbResult<()> {
    unindex(v, et, id)?;
    v.db.exec(
        "INSERT INTO search_index(entity_type,entity_id,project_id,title,body) VALUES (?,?,?,?,?)",
        p![et, id, project_id, title, body],
    )?;
    let rowid = v.db.last_insert_rowid();
    v.db.exec("INSERT INTO search_keys(entity_type,entity_id,fts_rowid) VALUES (?,?,?)", p![et, id, rowid])?;
    Ok(())
}

pub fn unindex(v: &Vault, et: &str, id: &str) -> DbResult<()> {
    if let Some(r) = v.db.one("SELECT fts_rowid FROM search_keys WHERE entity_type=? AND entity_id=?", p![et, id])? {
        v.db.exec("DELETE FROM search_index WHERE rowid=?", p![r.i("fts_rowid")])?;
        v.db.exec("DELETE FROM search_keys WHERE entity_type=? AND entity_id=?", p![et, id])?;
    }
    Ok(())
}

/// Removes every index row belonging to a project.
pub fn unindex_project(v: &Vault, project_id: &str) -> DbResult<()> {
    v.db.exec(
        "DELETE FROM search_keys WHERE fts_rowid IN (SELECT rowid FROM search_index WHERE project_id=?)",
        p![project_id],
    )?;
    v.db.exec("DELETE FROM search_index WHERE project_id=?", p![project_id])?;
    Ok(())
}

fn tag_text(v: &Vault, et: &str, id: &str) -> DbResult<String> {
    Ok(crate::projects::tags_for(v, et, id)?.join(" "))
}

pub fn index_project(v: &Vault, id: &str) -> DbResult<()> {
    if let Some(r) = v.db.one("SELECT name, description, category, status FROM projects WHERE id=?", p![id])? {
        let body = format!("{} {} {} {}", r.s("description"), r.s("category"), r.s("status"), tag_text(v, "project", id)?);
        index(v, "project", id, id, &r.s("name"), &body)?;
    }
    Ok(())
}

pub fn index_file(v: &Vault, file_id: &str) -> DbResult<()> {
    let r = v.db.one(
        "SELECT f.project_id, f.name, f.rel_path, d.purpose, d.notes_md, d.warnings
         FROM files f LEFT JOIN file_docs d ON d.file_id=f.id WHERE f.id=?",
        p![file_id],
    )?;
    if let Some(r) = r {
        let body = format!(
            "{} {} {} {} {}",
            r.s("rel_path"),
            r.s("purpose"),
            r.s("notes_md"),
            r.s("warnings"),
            tag_text(v, "file", file_id)?
        );
        index(v, "file", file_id, &r.s("project_id"), &r.s("name"), &body)?;
    }
    Ok(())
}

/// Turns free text into a safe FTS5 query: every word becomes a quoted
/// prefix term, so user input can never be parsed as FTS syntax.
pub fn fts_query(q: &str) -> Option<String> {
    let terms: Vec<String> = q
        .split(|c: char| c.is_whitespace())
        .map(|t| t.chars().filter(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '.')).collect::<String>())
        .filter(|t| !t.is_empty())
        .map(|t| format!("\"{}\"*", t.replace('"', "")))
        .collect();
    if terms.is_empty() { None } else { Some(terms.join(" ")) }
}

/// File name/path substring matches ("server" finds "createserver.menu"),
/// every term must appear. Name matches rank before path-only matches.
fn file_substring_ids(v: &Vault, q: &str, project_id: Option<&str>, limit: i64) -> DbResult<Vec<String>> {
    let terms: Vec<String> = q.split_whitespace().map(|t| t.to_lowercase()).filter(|t| t.len() >= 2).take(6).collect();
    if terms.is_empty() {
        return Ok(vec![]);
    }
    let esc = |t: &str| format!("%{}%", t.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_"));
    let mut sql = String::from("SELECT id, (");
    sql.push_str(&terms.iter().map(|_| "(lower(name) LIKE ? ESCAPE '\\')").collect::<Vec<_>>().join(" + "));
    sql.push_str(") AS name_hits FROM files WHERE state!='deleted' AND (? IS NULL OR project_id=?)");
    for _ in &terms {
        sql.push_str(" AND lower(rel_path) LIKE ? ESCAPE '\\'");
    }
    sql.push_str(" ORDER BY name_hits DESC, length(rel_path) LIMIT ?");
    let mut params: Vec<crate::db::Value> = terms.iter().map(|t| esc(t).into()).collect();
    params.push(project_id.map(String::from).into());
    params.push(project_id.map(String::from).into());
    params.extend(terms.iter().map(|t| esc(t).into()));
    params.push(limit.into());
    Ok(v.db.query(&sql, &params)?.iter().map(|r| r.s("id")).collect())
}

fn file_hit(v: &Vault, file_id: &str) -> DbResult<Option<Json>> {
    let Some(f) = v.db.one(
        "SELECT f.id, f.project_id, f.rel_path, f.name, f.state, f.ext, f.kind, p.root_path, p.name AS project_name, d.purpose
         FROM files f JOIN projects p ON p.id=f.project_id LEFT JOIN file_docs d ON d.file_id=f.id WHERE f.id=?",
        p![file_id],
    )?
    else {
        return Ok(None);
    };
    if f.s("state") == "deleted" {
        return Ok(None);
    }
    let mut j = Json::obj()
        .with("type", "file")
        .with("id", f.s("id"))
        .with("projectId", f.s("project_id"))
        .with("projectName", f.s("project_name"))
        .with("title", f.s("name"))
        .with("snippet", f.s("purpose"))
        .with("path", f.s("rel_path"))
        .with("name", f.s("name"))
        .with("ext", f.s("ext"))
        .with("kind", f.s("kind"))
        .with("state", f.s("state"));
    if let Ok(abs) = crate::util::safe_join(std::path::Path::new(&f.s("root_path")), &f.s("rel_path")) {
        j.set("absPath", crate::projects::display_path(&abs));
    }
    Ok(Some(j))
}

pub fn search(v: &Vault, q: &str, project_id: Option<&str>, types: &[String], limit: i64) -> DbResult<Json> {
    let Some(fq) = fts_query(q) else { return Ok(Json::Arr(vec![])) };
    let want_files = types.is_empty() || types.iter().any(|t| t == "file");
    let sub_ids = if want_files { file_substring_ids(v, q, project_id, limit.min(40))? } else { vec![] };
    let rows = v.db.query(
        "SELECT s.entity_type, s.entity_id, s.project_id, s.title,
                snippet(search_index, 4, '\u{1}', '\u{2}', '…', 12) AS snip,
                bm25(search_index, 0, 0, 0, 8.0, 1.0) AS score, pr.name AS project_name
         FROM search_index s LEFT JOIN projects pr ON pr.id = s.project_id
         WHERE search_index MATCH ? AND (? IS NULL OR s.project_id = ?)
         ORDER BY score LIMIT ?",
        p![fq, project_id, project_id, limit.clamp(1, 500) * 3],
    )?;
    let mut out = Vec::new();
    for r in rows {
        let et = r.s("entity_type");
        if !types.is_empty() && !types.contains(&et) {
            continue;
        }
        let mut item = Json::obj()
            .with("type", et.clone())
            .with("id", r.s("entity_id"))
            .with("projectId", r.s("project_id"))
            .with("projectName", r.os("project_name"))
            .with("title", r.s("title"))
            .with("snippet", r.s("snip"))
            .with("score", r.get("score").as_i64().map(|x| x as f64).unwrap_or(0.0));
        if et == "backup" {
            match v.db.one(
                "SELECT fv.*, f.name AS file_name, f.state AS file_state FROM file_versions fv JOIN files f ON f.id=fv.file_id WHERE fv.id=?",
                p![r.s("entity_id")],
            )? {
                Some(b) => {
                    item.set("fileId", b.s("file_id"));
                    item.set("fileName", b.s("file_name"));
                    item.set("path", b.s("rel_path"));
                    item.set("backupName", b.s("name"));
                    item.set("description", b.s("note"));
                    item.set("reason", b.s("reason"));
                    item.set("at", b.i("created_at"));
                    item.set("fileState", b.s("file_state"));
                }
                None => continue,
            }
        }
        if et == "snapshot" {
            if let Some(sn) = v.db.one("SELECT created_at, file_count FROM snapshots WHERE id=?", p![r.s("entity_id")])? {
                item.set("at", sn.i("created_at"));
                item.set("fileCount", sn.i("file_count"));
            } else {
                continue;
            }
        }
        if et == "file" {
            if let Some(f) = v.db.one("SELECT f.rel_path, f.name, f.state, f.ext, f.kind, p.root_path FROM files f JOIN projects p ON p.id=f.project_id WHERE f.id=?", p![r.s("entity_id")])? {
                if f.s("state") == "deleted" {
                    continue;
                }
                let root = std::path::PathBuf::from(f.s("root_path"));
                item.set("path", f.s("rel_path"));
                item.set("name", f.s("name"));
                item.set("ext", f.s("ext"));
                item.set("kind", f.s("kind"));
                item.set("state", f.s("state"));
                // real indexed location: project root + stored relative path (no guessing)
                if let Ok(abs) = crate::util::safe_join(&root, &f.s("rel_path")) {
                    item.set("absPath", crate::projects::display_path(&abs));
                }
            }
        }
        out.push(item);
        if out.len() as i64 >= limit {
            break;
        }
    }
    // substring matches on file names/paths that the word index didn't find
    let seen: std::collections::HashSet<String> = out.iter().filter_map(|h| h.str_of("id")).collect();
    let mut extra = Vec::new();
    for id in sub_ids.iter().filter(|id| !seen.contains(*id)) {
        if let Some(h) = file_hit(v, id)? {
            extra.push(h);
        }
    }
    // file-name matches go before body-only matches from the index
    let ql = q.to_lowercase();
    let name_match = |h: &Json| h.str_of("type").as_deref() == Some("file") && ql.split_whitespace().all(|t| h.str_of("name").unwrap_or_default().to_lowercase().contains(t));
    let (mut first, rest): (Vec<Json>, Vec<Json>) = out.into_iter().partition(|h| name_match(h));
    let (extra_names, extra_paths): (Vec<Json>, Vec<Json>) = extra.into_iter().partition(|h| name_match(h));
    first.extend(extra_names);
    first.extend(rest);
    first.extend(extra_paths);
    first.truncate(limit.max(1) as usize);
    Ok(Json::Arr(first))
}

/// Rebuilds the whole index from source tables (recovery tool).
pub fn rebuild(v: &Vault) -> DbResult<i64> {
    v.db.exec("DELETE FROM search_index", p![])?;
    v.db.exec("DELETE FROM search_keys", p![])?;
    for r in v.db.query("SELECT id FROM projects", p![])? {
        index_project(v, &r.s("id"))?;
    }
    for r in v.db.query("SELECT id FROM files", p![])? {
        index_file(v, &r.s("id"))?;
    }
    for r in v.db.query("SELECT id FROM notes WHERE deleted_at IS NULL", p![])? {
        crate::knowledge::index_note(v, &r.s("id"))?;
    }
    for r in v.db.query("SELECT id FROM tasks", p![])? {
        crate::tasks::index_task(v, &r.s("id"))?;
    }
    for r in v.db.query("SELECT id FROM file_versions WHERE name!='' OR note!='' OR reason IN ('manual','pre-save','post-save')", p![])? {
        crate::versions::index_backup(v, &r.s("id"))?;
    }
    for r in v.db.query("SELECT id, project_id, name, description, file_count FROM snapshots", p![])? {
        index(v, "snapshot", &r.s("id"), &r.s("project_id"), &r.s("name"), &format!("{} snapshot {} files", r.s("description"), r.i("file_count")))?;
    }
    for r in v.db.query("SELECT id, project_id, summary, detail FROM timeline WHERE manual=1", p![])? {
        index(v, "journal", &r.s("id"), &r.s("project_id"), &r.s("summary"), &r.s("detail"))?;
    }
    v.db.scalar_i64("SELECT count(*) FROM search_index", p![])
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fts_query_is_safe() {
        assert_eq!(fts_query("weapon tuner").unwrap(), "\"weapon\"* \"tuner\"*");
        assert_eq!(fts_query("a\" OR NEAR(x").unwrap(), "\"a\"* \"OR\"* \"NEARx\"*");
        assert!(fts_query("  \"() ").is_none());
    }
}

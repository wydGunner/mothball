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

pub fn search(v: &Vault, q: &str, project_id: Option<&str>, types: &[String], limit: i64) -> DbResult<Json> {
    let Some(fq) = fts_query(q) else { return Ok(Json::Arr(vec![])) };
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
        if et == "file" {
            if let Some(f) = v.db.one("SELECT rel_path, state FROM files WHERE id=?", p![r.s("entity_id")])? {
                item.set("path", f.s("rel_path"));
                item.set("state", f.s("state"));
            }
        }
        out.push(item);
        if out.len() as i64 >= limit {
            break;
        }
    }
    Ok(Json::Arr(out))
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

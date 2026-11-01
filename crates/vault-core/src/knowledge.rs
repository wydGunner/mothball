//! Project knowledge base (hierarchical Markdown notes with revisions) and
//! the generic link graph between files, notes and tasks.

use crate::db::{DbResult, Row};
use crate::json::Json;
use crate::util::{new_id, now_ms};
use crate::{p, search, timeline, Vault};

pub const NOTE_CATEGORIES: &[&str] = &[
    "General", "Journal", "Research", "Installation", "Configuration", "Troubleshooting", "Ideas", "Architecture", "Release Notes", "Known Bugs",
];

fn note_json(r: &Row, with_body: bool) -> Json {
    let mut j = Json::obj()
        .with("id", r.s("id"))
        .with("projectId", r.s("project_id"))
        .with("parentId", r.os("parent_id"))
        .with("title", r.s("title"))
        .with("category", r.s("category"))
        .with("sort", r.i("sort"))
        .with("createdAt", r.i("created_at"))
        .with("updatedAt", r.i("updated_at"));
    if with_body {
        j.set("body", r.s("body_md"));
    }
    j
}

pub fn index_note(v: &Vault, id: &str) -> DbResult<()> {
    if let Some(r) = v.db.one("SELECT * FROM notes WHERE id=? AND deleted_at IS NULL", p![id])? {
        search::index(v, "note", id, &r.s("project_id"), &r.s("title"), &format!("{} {}", r.s("category"), r.s("body_md")))?;
    }
    Ok(())
}

pub fn tree(v: &Vault, project_id: &str) -> DbResult<Json> {
    let rows = v.db.query(
        "SELECT * FROM notes WHERE project_id=? AND deleted_at IS NULL ORDER BY sort, title COLLATE NOCASE",
        p![project_id],
    )?;
    Ok(Json::Arr(rows.iter().map(|r| note_json(r, false)).collect()))
}

pub fn get(v: &Vault, id: &str) -> DbResult<Json> {
    let r = v.db.one("SELECT * FROM notes WHERE id=?", p![id])?.ok_or("Note not found")?;
    let mut j = note_json(&r, true);
    j.set("links", links_for(v, "note", id)?);
    j.set("deleted", r.oi("deleted_at").is_some());
    Ok(j)
}

pub fn create(v: &Vault, project_id: &str, title: &str, parent_id: Option<String>, category: &str, body: &str) -> DbResult<Json> {
    let title = if title.trim().is_empty() { "Untitled" } else { title.trim() };
    if let Some(pid) = &parent_id {
        let ok = v.db.scalar_i64("SELECT count(*) FROM notes WHERE id=? AND project_id=?", p![pid.clone(), project_id])?;
        if ok == 0 {
            return Err("Parent note not found in this project".into());
        }
    }
    let id = new_id();
    let now = now_ms();
    let sort = v.db.scalar_i64("SELECT COALESCE(MAX(sort),0)+1 FROM notes WHERE project_id=? AND parent_id IS ?", p![project_id, parent_id.clone()])?;
    v.db.exec(
        "INSERT INTO notes(id,project_id,parent_id,title,body_md,category,sort,created_at,updated_at) VALUES (?,?,?,?,?,?,?,?,?)",
        p![id.clone(), project_id, parent_id, title, body, if category.is_empty() { "General" } else { category }, sort, now, now],
    )?;
    timeline::add(v, project_id, "note", &format!("Note created: {title}"), "", Some(("note", &id)), false)?;
    index_note(v, &id)?;
    get(v, &id)
}

/// Saves a note. Keeps a revision of the previous content when it changed and
/// the last revision is older than 5 minutes, so autosave does not create a
/// revision per keystroke but history is never lost for long.
pub fn save(v: &Vault, id: &str, patch: &Json) -> DbResult<Json> {
    let old = v.db.one("SELECT * FROM notes WHERE id=? AND deleted_at IS NULL", p![id])?.ok_or("Note not found")?;
    let title = patch.str_of("title").unwrap_or(old.s("title"));
    let body = patch.str_of("body").unwrap_or(old.s("body_md"));
    let category = patch.str_of("category").unwrap_or(old.s("category"));
    let now = now_ms();
    if let Some(parent) = patch.get("parentId") {
        let new_parent = parent.as_str().map(String::from);
        if let Some(np) = &new_parent {
            // refuse cycles: walk up from the new parent
            let mut cur = Some(np.clone());
            while let Some(c) = cur {
                if c == id {
                    return Err("A note cannot be moved inside itself".into());
                }
                cur = v.db.one("SELECT parent_id FROM notes WHERE id=?", p![c])?.and_then(|r| r.os("parent_id"));
            }
        }
        v.db.exec("UPDATE notes SET parent_id=? WHERE id=?", p![new_parent, id])?;
    }
    if let Some(s) = patch.get("sort").and_then(|x| x.as_i64()) {
        v.db.exec("UPDATE notes SET sort=? WHERE id=?", p![s, id])?;
    }
    let changed = title != old.s("title") || body != old.s("body_md") || category != old.s("category");
    if changed {
        v.db.tx(|db| {
            let last = db.scalar_i64("SELECT COALESCE(MAX(created_at),0) FROM note_revisions WHERE note_id=?", p![id])?;
            if (old.s("body_md") != body || old.s("title") != title) && now - last > 5 * 60 * 1000 && !old.s("body_md").is_empty() {
                db.exec(
                    "INSERT INTO note_revisions(id,note_id,title,body_md,created_at) VALUES (?,?,?,?,?)",
                    p![new_id(), id, old.s("title"), old.s("body_md"), now],
                )?;
            }
            db.exec("UPDATE notes SET title=?, body_md=?, category=?, updated_at=? WHERE id=?", p![title.clone(), body, category, now, id])?;
            timeline::add_coalesced(v, &old.s("project_id"), "note", &format!("Note edited: {title}"), "note", id)?;
            Ok(())
        })?;
        index_note(v, id)?;
    }
    get(v, id)
}

pub fn revisions(v: &Vault, id: &str) -> DbResult<Json> {
    let rows = v.db.query("SELECT * FROM note_revisions WHERE note_id=? ORDER BY created_at DESC LIMIT 200", p![id])?;
    Ok(Json::Arr(rows.iter().map(|r| Json::obj().with("id", r.s("id")).with("title", r.s("title")).with("body", r.s("body_md")).with("at", r.i("created_at"))).collect()))
}

/// Soft delete (children are moved up one level so nothing disappears).
pub fn delete(v: &Vault, id: &str) -> DbResult<()> {
    let r = v.db.one("SELECT project_id, parent_id, title FROM notes WHERE id=?", p![id])?.ok_or("Note not found")?;
    v.db.tx(|db| {
        db.exec("UPDATE notes SET parent_id=? WHERE parent_id=?", p![r.os("parent_id"), id])?;
        db.exec("UPDATE notes SET deleted_at=? WHERE id=?", p![now_ms(), id])?;
        timeline::add(v, &r.s("project_id"), "note", &format!("Note moved to trash: {}", r.s("title")), "", Some(("note", id)), false)?;
        Ok(())
    })?;
    search::unindex(v, "note", id)
}

pub fn trash(v: &Vault, project_id: &str) -> DbResult<Json> {
    let rows = v.db.query("SELECT * FROM notes WHERE project_id=? AND deleted_at IS NOT NULL ORDER BY deleted_at DESC", p![project_id])?;
    Ok(Json::Arr(rows.iter().map(|r| note_json(r, false)).collect()))
}

pub fn restore(v: &Vault, id: &str) -> DbResult<Json> {
    v.db.exec("UPDATE notes SET deleted_at=NULL, parent_id=CASE WHEN parent_id IN (SELECT id FROM notes WHERE deleted_at IS NULL) THEN parent_id ELSE NULL END WHERE id=?", p![id])?;
    index_note(v, id)?;
    get(v, id)
}

// ------------------------------------------------------------------ links

const ENTITY_TABLES: &[(&str, &str)] = &[("file", "files"), ("note", "notes"), ("task", "tasks")];

fn entity_project(v: &Vault, et: &str, id: &str) -> DbResult<String> {
    let table = ENTITY_TABLES.iter().find(|(t, _)| *t == et).map(|(_, tb)| *tb).ok_or(format!("Unknown entity type {et}"))?;
    let r = v.db.one(&format!("SELECT project_id FROM {table} WHERE id=?"), p![id])?.ok_or(format!("{et} not found"))?;
    Ok(r.s("project_id"))
}

pub fn link(v: &Vault, a_type: &str, a_id: &str, b_type: &str, b_id: &str, label: &str) -> DbResult<Json> {
    if a_type == b_type && a_id == b_id {
        return Err("Cannot link an item to itself".into());
    }
    let pa = entity_project(v, a_type, a_id)?;
    let pb = entity_project(v, b_type, b_id)?;
    if pa != pb {
        return Err("Links must stay within one project".into());
    }
    // store in a canonical order so A-B and B-A are the same link
    let (x, y) = if (a_type, a_id) <= (b_type, b_id) { ((a_type, a_id), (b_type, b_id)) } else { ((b_type, b_id), (a_type, a_id)) };
    v.db.exec(
        "INSERT INTO links(id,project_id,a_type,a_id,b_type,b_id,label,created_at) VALUES (?,?,?,?,?,?,?,?)
         ON CONFLICT(a_type,a_id,b_type,b_id) DO UPDATE SET label=excluded.label",
        p![new_id(), pa, x.0, x.1, y.0, y.1, label, now_ms()],
    )?;
    Ok(Json::obj().with("ok", true))
}

pub fn unlink(v: &Vault, link_id: &str) -> DbResult<()> {
    v.db.exec("DELETE FROM links WHERE id=?", p![link_id])?;
    Ok(())
}

fn entity_label(v: &Vault, et: &str, id: &str) -> DbResult<(String, String)> {
    Ok(match et {
        "file" => v.db.one("SELECT rel_path, state FROM files WHERE id=?", p![id])?.map(|r| (r.s("rel_path"), r.s("state"))).unwrap_or(("(deleted file)".into(), "gone".into())),
        "note" => v.db.one("SELECT title FROM notes WHERE id=?", p![id])?.map(|r| (r.s("title"), String::new())).unwrap_or(("(deleted note)".into(), "gone".into())),
        "task" => v.db.one("SELECT title, status, kind FROM tasks WHERE id=?", p![id])?.map(|r| (r.s("title"), format!("{}:{}", r.s("kind"), r.s("status")))).unwrap_or(("(deleted task)".into(), "gone".into())),
        _ => ("?".into(), String::new()),
    })
}

pub fn links_for(v: &Vault, et: &str, id: &str) -> DbResult<Json> {
    let rows = v.db.query(
        "SELECT * FROM links WHERE (a_type=? AND a_id=?) OR (b_type=? AND b_id=?) ORDER BY created_at",
        p![et, id, et, id],
    )?;
    let mut out = Vec::new();
    for r in rows {
        let (ot, oid) = if r.s("a_type") == et && r.s("a_id") == id { (r.s("b_type"), r.s("b_id")) } else { (r.s("a_type"), r.s("a_id")) };
        let (label, state) = entity_label(v, &ot, &oid)?;
        out.push(Json::obj().with("linkId", r.s("id")).with("type", ot).with("id", oid).with("title", label).with("state", state).with("label", r.s("label")));
    }
    Ok(Json::Arr(out))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projects::{create as cp, NewProject};
    use crate::testutil::*;
    #[test]
    fn notes_hierarchy_revisions_links() {
        let (_t, v) = vault();
        let pid = cp(&v, NewProject { name: "OMEN v2".into(), description: "".into(), category: "software".into(), status: "".into(), link_path: None, module: Json::obj() }).unwrap().str_of("id").unwrap();
        let arch = create(&v, &pid, "Architecture", None, "Architecture", "v1 body").unwrap().str_of("id").unwrap();
        let cmds = create(&v, &pid, "Commands", Some(arch.clone()), "General", "").unwrap().str_of("id").unwrap();
        assert!(save(&v, &arch, &Json::obj().with("parentId", cmds.clone())).is_err(), "cycle refused");
        save(&v, &arch, &Json::obj().with("body", "v2 body")).unwrap();
        assert_eq!(revisions(&v, &arch).unwrap().as_arr().unwrap()[0].str_of("body").unwrap(), "v1 body");
        // rapid autosave doesn't create another revision
        save(&v, &arch, &Json::obj().with("body", "v3 body")).unwrap();
        assert_eq!(revisions(&v, &arch).unwrap().as_arr().unwrap().len(), 1);
        link(&v, "note", &arch, "note", &cmds, "see also").unwrap();
        link(&v, "note", &cmds, "note", &arch, "dup").unwrap();
        assert_eq!(links_for(&v, "note", &arch).unwrap().as_arr().unwrap().len(), 1);
        delete(&v, &arch).unwrap();
        let t = tree(&v, &pid).unwrap();
        assert_eq!(t.as_arr().unwrap().len(), 1);
        assert!(t.as_arr().unwrap()[0].get("parentId").is_none(), "child moved up");
        restore(&v, &arch).unwrap();
        assert_eq!(tree(&v, &pid).unwrap().as_arr().unwrap().len(), 2);
    }
}

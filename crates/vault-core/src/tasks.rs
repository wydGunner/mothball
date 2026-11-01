//! Tasks, bug reports and milestones. Completed tasks stay in history.

use crate::db::{DbResult, Row};
use crate::json::Json;
use crate::util::{new_id, now_ms};
use crate::{p, projects, search, timeline, Vault};

pub const STATUSES: &[&str] = &["todo", "in_progress", "blocked", "done"];

fn task_json(v: &Vault, r: &Row) -> DbResult<Json> {
    let id = r.s("id");
    Ok(Json::obj()
        .with("id", id.clone())
        .with("projectId", r.s("project_id"))
        .with("kind", r.s("kind"))
        .with("title", r.s("title"))
        .with("description", r.s("description"))
        .with("status", r.s("status"))
        .with("priority", r.i("priority"))
        .with("dueAt", r.oi("due_at"))
        .with("milestoneId", r.os("milestone_id"))
        .with("checklist", Json::parse(&r.s("checklist_json")).unwrap_or(Json::Arr(vec![])))
        .with("reproSteps", r.s("repro_steps"))
        .with("expected", r.s("expected"))
        .with("actual", r.s("actual"))
        .with("resolution", r.s("resolution"))
        .with("sort", r.i("sort"))
        .with("createdAt", r.i("created_at"))
        .with("updatedAt", r.i("updated_at"))
        .with("completedAt", r.oi("completed_at"))
        .with("tags", projects::tags_for(v, "task", &id)?))
}

pub fn index_task(v: &Vault, id: &str) -> DbResult<()> {
    if let Some(r) = v.db.one("SELECT * FROM tasks WHERE id=?", p![id])? {
        let body = format!(
            "{} {} {} {} {} {} {}",
            r.s("kind"),
            r.s("description"),
            r.s("repro_steps"),
            r.s("expected"),
            r.s("actual"),
            r.s("resolution"),
            projects::tags_for(v, "task", id)?.join(" ")
        );
        search::index(v, if r.s("kind") == "bug" { "bug" } else { "task" }, id, &r.s("project_id"), &r.s("title"), &body)?;
    }
    Ok(())
}

pub fn list(v: &Vault, project_id: &str) -> DbResult<Json> {
    let rows = v.db.query("SELECT * FROM tasks WHERE project_id=? ORDER BY status='done', priority DESC, sort, created_at", p![project_id])?;
    let mut out = Vec::new();
    for r in &rows {
        out.push(task_json(v, r)?);
    }
    Ok(Json::Arr(out))
}

pub fn get(v: &Vault, id: &str) -> DbResult<Json> {
    let r = v.db.one("SELECT * FROM tasks WHERE id=?", p![id])?.ok_or("Task not found")?;
    let mut j = task_json(v, &r)?;
    j.set("links", crate::knowledge::links_for(v, "task", id)?);
    Ok(j)
}

pub fn create(v: &Vault, project_id: &str, d: &Json) -> DbResult<Json> {
    let title = d.str_of("title").unwrap_or_default();
    if title.trim().is_empty() {
        return Err("Title is required".into());
    }
    let kind = if d.str_of("kind").as_deref() == Some("bug") { "bug" } else { "task" };
    let id = new_id();
    let now = now_ms();
    v.db.exec(
        "INSERT INTO tasks(id,project_id,kind,title,description,status,priority,due_at,created_at,updated_at) VALUES (?,?,?,?,?,?,?,?,?,?)",
        p![
            id.clone(),
            project_id,
            kind,
            title.trim(),
            d.str_of("description").unwrap_or_default(),
            "todo",
            d.get("priority").and_then(|x| x.as_i64()).unwrap_or(1).clamp(0, 3),
            d.get("dueAt").and_then(|x| x.as_i64()),
            now,
            now
        ],
    )?;
    if d.get("status").is_some() || d.get("reproSteps").is_some() || d.get("tags").is_some() {
        update(v, &id, d)?;
    }
    timeline::add(v, project_id, kind, &format!("{} created: {}", if kind == "bug" { "Bug" } else { "Task" }, title.trim()), "", Some(("task", &id)), false)?;
    index_task(v, &id)?;
    get(v, &id)
}

pub fn update(v: &Vault, id: &str, d: &Json) -> DbResult<Json> {
    let old = v.db.one("SELECT * FROM tasks WHERE id=?", p![id])?.ok_or("Task not found")?;
    let now = now_ms();
    v.db.tx(|db| {
        for (k, col) in [("title", "title"), ("description", "description"), ("reproSteps", "repro_steps"), ("expected", "expected"), ("actual", "actual"), ("resolution", "resolution")] {
            if let Some(s) = d.str_of(k) {
                if k == "title" && s.trim().is_empty() {
                    return Err("Title is required".into());
                }
                db.exec(&format!("UPDATE tasks SET {col}=? WHERE id=?"), p![s, id])?;
            }
        }
        if let Some(k) = d.str_of("kind") {
            if k == "bug" || k == "task" {
                db.exec("UPDATE tasks SET kind=? WHERE id=?", p![k, id])?;
            }
        }
        if let Some(pr) = d.get("priority").and_then(|x| x.as_i64()) {
            db.exec("UPDATE tasks SET priority=? WHERE id=?", p![pr.clamp(0, 3), id])?;
        }
        if let Some(s) = d.get("sort").and_then(|x| x.as_i64()) {
            db.exec("UPDATE tasks SET sort=? WHERE id=?", p![s, id])?;
        }
        if let Some(x) = d.get("dueAt") {
            db.exec("UPDATE tasks SET due_at=? WHERE id=?", p![x.as_i64(), id])?;
        } else if matches!(d, Json::Obj(m) if m.contains_key("dueAt")) {
            db.exec("UPDATE tasks SET due_at=NULL WHERE id=?", p![id])?;
        }
        if let Json::Obj(m) = d {
            if m.contains_key("milestoneId") {
                db.exec("UPDATE tasks SET milestone_id=? WHERE id=?", p![d.str_of("milestoneId"), id])?;
            }
        }
        if let Some(c) = d.get("checklist") {
            if !matches!(c, Json::Arr(_)) {
                return Err("checklist must be an array".into());
            }
            db.exec("UPDATE tasks SET checklist_json=? WHERE id=?", p![c.to_string(), id])?;
        }
        if let Some(s) = d.str_of("status") {
            if !STATUSES.contains(&s.as_str()) {
                return Err(format!("Invalid status {s}"));
            }
            if s != old.s("status") {
                db.exec(
                    "UPDATE tasks SET status=?, completed_at=CASE WHEN ?='done' THEN ? ELSE NULL END WHERE id=?",
                    p![s.clone(), s.clone(), now, id],
                )?;
                if s == "done" {
                    timeline::add(v, &old.s("project_id"), "task-done", &format!("Completed: {}", old.s("title")), "", Some(("task", id)), false)?;
                }
            }
        }
        if let Some(Json::Arr(tags)) = d.get("tags") {
            let t: Vec<String> = tags.iter().filter_map(|x| x.as_str().map(String::from)).collect();
            projects::set_tags(v, "task", id, &t)?;
        }
        db.exec("UPDATE tasks SET updated_at=? WHERE id=?", p![now, id])?;
        Ok(())
    })?;
    index_task(v, id)?;
    get(v, id)
}

/// Hard delete is allowed only for tasks that were never completed;
/// completed work stays in project history.
pub fn delete(v: &Vault, id: &str) -> DbResult<()> {
    let r = v.db.one("SELECT status FROM tasks WHERE id=?", p![id])?.ok_or("Task not found")?;
    if r.s("status") == "done" {
        return Err("Completed tasks are kept in project history and cannot be deleted.".into());
    }
    v.db.exec("DELETE FROM links WHERE (a_type='task' AND a_id=?) OR (b_type='task' AND b_id=?)", p![id, id])?;
    v.db.exec("DELETE FROM tasks WHERE id=?", p![id])?;
    search::unindex(v, "task", id)?;
    search::unindex(v, "bug", id)
}

pub fn milestones(v: &Vault, project_id: &str) -> DbResult<Json> {
    let rows = v.db.query(
        "SELECT m.*, (SELECT count(*) FROM tasks t WHERE t.milestone_id=m.id) AS total,
                (SELECT count(*) FROM tasks t WHERE t.milestone_id=m.id AND t.status='done') AS done_count
         FROM milestones m WHERE project_id=? ORDER BY due_at IS NULL, due_at, created_at",
        p![project_id],
    )?;
    Ok(Json::Arr(
        rows.iter()
            .map(|r| {
                Json::obj()
                    .with("id", r.s("id"))
                    .with("title", r.s("title"))
                    .with("description", r.s("description"))
                    .with("dueAt", r.oi("due_at"))
                    .with("done", r.b("done"))
                    .with("total", r.i("total"))
                    .with("completed", r.i("done_count"))
            })
            .collect(),
    ))
}

pub fn milestone_save(v: &Vault, project_id: &str, d: &Json) -> DbResult<Json> {
    let title = d.str_of("title").unwrap_or_default();
    match d.str_of("id") {
        Some(id) => {
            let was_done = v.db.one("SELECT done, title FROM milestones WHERE id=?", p![id.clone()])?.ok_or("Milestone not found")?;
            let done = d.get("done").and_then(|x| x.as_bool()).unwrap_or(was_done.b("done"));
            v.db.exec(
                "UPDATE milestones SET title=COALESCE(NULLIF(?,''),title), description=COALESCE(?,description), due_at=?, done=? WHERE id=?",
                p![title, d.str_of("description"), d.get("dueAt").and_then(|x| x.as_i64()), done, id.clone()],
            )?;
            if done && !was_done.b("done") {
                timeline::add(v, project_id, "milestone", &format!("Milestone reached: {}", was_done.s("title")), "", None, false)?;
            }
        }
        None => {
            if title.trim().is_empty() {
                return Err("Title is required".into());
            }
            v.db.exec(
                "INSERT INTO milestones(id,project_id,title,description,due_at,created_at) VALUES (?,?,?,?,?,?)",
                p![new_id(), project_id, title.trim(), d.str_of("description").unwrap_or_default(), d.get("dueAt").and_then(|x| x.as_i64()), now_ms()],
            )?;
        }
    }
    milestones(v, project_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projects::{create as cp, NewProject};
    use crate::testutil::*;
    #[test]
    fn task_lifecycle() {
        let (_t, v) = vault();
        let pid = cp(&v, NewProject { name: "X".into(), description: "".into(), category: "software".into(), status: "".into(), link_path: None, module: Json::obj() }).unwrap().str_of("id").unwrap();
        let t = create(&v, &pid, &Json::parse(r#"{"title":"Fix instashoot desync","kind":"bug","reproSteps":"1. join server","tags":["netcode"]}"#).unwrap()).unwrap();
        let id = t.str_of("id").unwrap();
        assert_eq!(t.str_of("reproSteps").unwrap(), "1. join server");
        assert!(update(&v, &id, &Json::obj().with("status", "nope")).is_err());
        update(&v, &id, &Json::obj().with("status", "done")).unwrap();
        assert!(get(&v, &id).unwrap().get("completedAt").is_some());
        assert!(delete(&v, &id).is_err());
        assert!(search::search(&v, "instashoot", None, &["bug".into()], 5).unwrap().as_arr().unwrap().len() == 1);
    }
}

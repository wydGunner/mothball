//! Projects: registration (linked or managed), metadata, statuses, tags,
//! favorites, dashboard summaries.

use crate::db::{DbResult, Row};
use crate::json::Json;
use crate::util::{new_id, now_ms, slugify};
use crate::{p, search, timeline, Vault};
use std::path::{Path, PathBuf};

pub const BUILTIN_STATUSES: &[(&str, &str)] = &[
    ("Planned", "#7c8cff"),
    ("Active", "#3ecf8e"),
    ("Paused", "#f5b94a"),
    ("Completed", "#4fb3ff"),
    ("Archived", "#8b93a7"),
];

pub const CATEGORIES: &[&str] = &["general", "software", "game", "addon", "3d-printing", "3d-art", "cad", "video", "motion", "design", "electronics", "creative", "documents"];

pub fn seed_statuses(v: &Vault) -> DbResult<()> {
    for (i, (n, c)) in BUILTIN_STATUSES.iter().enumerate() {
        v.db.exec(
            "INSERT OR IGNORE INTO statuses(id,name,color,sort,builtin) VALUES (?,?,?,?,1)",
            p![new_id(), *n, *c, i as i64],
        )?;
    }
    Ok(())
}

pub fn default_ignores(category: &str) -> &'static str {
    match category {
        // Regenerable Unity/engine output; can be removed per project.
        "game" => "Library/\nTemp/\nLogs/\nobj/\nBuild/\nBuilds/\nUserSettings/",
        "software" => "target/\ndist/\nbuild/\n.venv/",
        // Caches, previews and autosaves the editing apps rebuild themselves.
        "video" => "Adobe Premiere Pro Auto-Save/\nAdobe Premiere Pro Video Previews/\nAdobe Premiere Pro Audio Previews/\nMedia Cache Files/\nMedia Cache/\nCacheClip/\nProxyMedia/\n*.pek\n*.cfa\n*.sfk\n*.sfl",
        "motion" => "Adobe After Effects Auto-Save/\nMedia Cache Files/\nMedia Cache/\n*.pek\n*.cfa\n*.blend1\n*.blend2",
        "3d-art" => "*.blend1\n*.blend2\n*.blend3\n*.blend@",
        "cad" => "*.bak\n*.dwl\n*.dwl2\n*.f3d.bak",
        "addon" => ".release/",
        _ => "",
    }
}

pub struct NewProject {
    pub name: String,
    pub description: String,
    pub category: String,
    pub status: String,
    /// Some(path) => linked to an existing folder; None => managed.
    pub link_path: Option<PathBuf>,
    pub module: Json,
}

pub fn create(v: &Vault, np: NewProject) -> DbResult<Json> {
    let name = np.name.trim();
    if name.is_empty() {
        return Err("Project name is required".into());
    }
    let category = if CATEGORIES.contains(&np.category.as_str()) { np.category.clone() } else { "general".into() };
    let id = new_id();
    let (mode, root) = match &np.link_path {
        Some(p) => {
            let md = std::fs::metadata(p).map_err(|e| format!("Cannot access folder {}: {e}", p.display()))?;
            if !md.is_dir() {
                return Err(format!("{} is not a folder", p.display()));
            }
            let canon = dunce(p);
            if let Some(r) = v.db.one("SELECT name FROM projects WHERE root_path=?", p![canon.to_string_lossy().to_string()])? {
                return Err(format!("This folder is already linked to project \"{}\"", r.s("name")));
            }
            ("linked", canon)
        }
        None => {
            let dir = v.managed_root().join(format!("{}-{}", slugify(name), &id[..8]));
            std::fs::create_dir_all(&dir).map_err(|e| format!("Cannot create project storage: {e}"))?;
            ("managed", dir)
        }
    };
    let now = now_ms();
    let status = if np.status.trim().is_empty() { "Active".to_string() } else { np.status.trim().to_string() };
    v.db.tx(|db| {
        ensure_status(v, &status)?;
        db.exec(
            "INSERT INTO projects(id,name,description,category,status,mode,root_path,module_json,ignore_globs,created_at,updated_at,last_opened_at)
             VALUES (?,?,?,?,?,?,?,?,?,?,?,?)",
            p![
                id.clone(),
                name,
                np.description.clone(),
                category.clone(),
                status.clone(),
                mode,
                root.to_string_lossy().to_string(),
                np.module.to_string(),
                default_ignores(&category),
                now,
                now,
                now
            ],
        )?;
        timeline::add(
            v,
            &id,
            "project",
            &format!("Project created ({mode})"),
            &root.to_string_lossy(),
            None,
            false,
        )?;
        Ok(())
    })?;
    search::index_project(v, &id)?;
    get(v, &id)
}

/// Canonicalize without Windows `\\?\` verbatim prefixes.
pub fn dunce(p: &Path) -> PathBuf {
    let c = std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let s = c.to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{rest}"))
    } else if let Some(rest) = s.strip_prefix(r"\\?\") {
        PathBuf::from(rest)
    } else {
        c
    }
}

/// Path as the OS shows it (backslashes on Windows).
pub fn display_path(p: &Path) -> String {
    let s = p.to_string_lossy().to_string();
    if cfg!(windows) { s.replace('/', "\\") } else { s }
}

fn ensure_status(v: &Vault, s: &str) -> DbResult<()> {
    let n = v.db.scalar_i64("SELECT count(*) FROM statuses WHERE name=?", p![s])?;
    if n == 0 {
        let sort = v.db.scalar_i64("SELECT COALESCE(MAX(sort),0)+1 FROM statuses", p![])?;
        v.db.exec("INSERT INTO statuses(id,name,sort) VALUES (?,?,?)", p![new_id(), s, sort])?;
    }
    Ok(())
}

pub fn statuses(v: &Vault) -> DbResult<Json> {
    let rows = v.db.query("SELECT name,color,builtin FROM statuses ORDER BY sort", p![])?;
    Ok(Json::Arr(
        rows.iter().map(|r| Json::obj().with("name", r.s("name")).with("color", r.s("color")).with("builtin", r.b("builtin"))).collect(),
    ))
}

pub fn tags_for(v: &Vault, et: &str, id: &str) -> DbResult<Vec<String>> {
    Ok(v.db
        .query(
            "SELECT t.name FROM entity_tags e JOIN tags t ON t.id=e.tag_id WHERE e.entity_type=? AND e.entity_id=? ORDER BY t.name",
            p![et, id],
        )?
        .iter()
        .map(|r| r.s("name"))
        .collect())
}

pub fn set_tags(v: &Vault, et: &str, id: &str, tags: &[String]) -> DbResult<()> {
    v.db.tx(|db| {
        db.exec("DELETE FROM entity_tags WHERE entity_type=? AND entity_id=?", p![et, id])?;
        for t in tags.iter().map(|t| t.trim()).filter(|t| !t.is_empty()) {
            db.exec("INSERT OR IGNORE INTO tags(id,name) VALUES (?,?)", p![new_id(), t])?;
            db.exec(
                "INSERT OR IGNORE INTO entity_tags SELECT ?,?,id FROM tags WHERE name=?",
                p![et, id, t],
            )?;
        }
        Ok(())
    })
}

pub fn all_tags(v: &Vault) -> DbResult<Json> {
    let rows = v.db.query(
        "SELECT t.name, count(e.tag_id) AS uses FROM tags t LEFT JOIN entity_tags e ON e.tag_id=t.id GROUP BY t.id ORDER BY t.name",
        p![],
    )?;
    Ok(Json::Arr(rows.iter().map(|r| Json::obj().with("name", r.s("name")).with("uses", r.i("uses"))).collect()))
}

fn row_json(v: &Vault, r: &Row) -> DbResult<Json> {
    let id = r.s("id");
    let stats = v.db.one(
        "SELECT count(*) AS n, COALESCE(SUM(size),0) AS bytes,
                SUM(state='missing') AS missing, SUM(changed_since_review) AS changed
         FROM files WHERE project_id=? AND state!='deleted'",
        p![id.clone()],
    )?;
    let st = stats.unwrap();
    let open_tasks = v.db.scalar_i64(
        "SELECT count(*) FROM tasks WHERE project_id=? AND status!='done'",
        p![id.clone()],
    )?;
    let reviews = v.db.scalar_i64("SELECT count(*) FROM reconcile_reviews WHERE project_id=?", p![id.clone()])?;
    // documented files whose content changed after the notes were written
    let stale_docs = v.db.scalar_i64(
        "SELECT count(*) FROM file_docs d JOIN files f ON f.id=d.file_id
         WHERE f.project_id=? AND f.state='present' AND d.doc_sha256 IS NOT NULL AND d.doc_sha256 != COALESCE(f.sha256,'')
           AND (d.purpose!='' OR d.notes_md!='' OR d.warnings!='')",
        p![id.clone()],
    )?;
    let latest = v.db.one(
        "SELECT summary, at FROM timeline WHERE project_id=? ORDER BY at DESC LIMIT 1",
        p![id.clone()],
    )?;
    let recent_files: Vec<Json> = v
        .db
        .query("SELECT rel_path, last_changed FROM files WHERE project_id=? AND state='present' ORDER BY last_changed DESC, rel_path LIMIT 3", p![id.clone()])?
        .iter()
        .map(|f| Json::obj().with("path", f.s("rel_path")).with("at", f.i("last_changed")))
        .collect();
    let last_file = recent_files.first().cloned();
    let module = Json::parse(&r.s("module_json")).unwrap_or(Json::obj());
    let root = r.s("root_path");
    let available = Path::new(&root).is_dir();
    Ok(Json::obj()
        .with("id", id.clone())
        .with("name", r.s("name"))
        .with("description", r.s("description"))
        .with("category", r.s("category"))
        .with("status", r.s("status"))
        .with("mode", r.s("mode"))
        .with("rootPath", root)
        .with("rootAvailable", available)
        .with("icon", r.s("icon"))
        .with("color", r.s("color"))
        .with("progress", r.i("progress"))
        .with("favorite", r.b("favorite"))
        .with("group", r.s("group_name"))
        .with("module", module)
        .with("ignoreGlobs", r.s("ignore_globs"))
        .with("createdAt", r.i("created_at"))
        .with("updatedAt", r.i("updated_at"))
        .with("lastOpenedAt", r.oi("last_opened_at"))
        .with("lastScanAt", r.oi("last_scan_at"))
        .with("tags", tags_for(v, "project", &id)?)
        .with("fileCount", st.i("n"))
        .with("bytes", st.i("bytes"))
        .with("missingFiles", st.i("missing"))
        .with("changedFiles", st.i("changed"))
        .with("pendingReviews", reviews)
        .with("staleDocs", stale_docs)
        .with("openTasks", open_tasks)
        .with("latestActivity", latest.as_ref().map(|l| l.s("summary")))
        .with("latestActivityAt", latest.as_ref().map(|l| l.i("at")))
        .with("lastFile", last_file.as_ref().and_then(|f| f.str_of("path")))
        .with("lastFileAt", last_file.as_ref().and_then(|f| f.get("at").and_then(|x| x.as_i64())))
        .with("recentFiles", Json::Arr(recent_files)))
}

/// The picture shown on a project's card. Settings live in the project's
/// module JSON under "thumb": {mode: auto|file|custom|none, fileId, fit:
/// cover|contain, x, y (0–100 focus)}. `auto` = newest .3mf plate image,
/// else an image whose name suggests a cover, else the newest image.
pub fn thumbnail(v: &Vault, id: &str) -> DbResult<Json> {
    let pr = v.db.one("SELECT module_json FROM projects WHERE id=?", p![id])?.ok_or("Project not found")?;
    let module = Json::parse(&pr.s("module_json")).unwrap_or(Json::obj());
    let t = thumb_settings(&module);
    let mode = t.str_of("mode").unwrap_or_else(|| "auto".into());
    let style = |j: Json| j.with("fit", t.str_of("fit").unwrap_or_else(|| "cover".into())).with("x", t.get("x").cloned().unwrap_or(Json::from(50))).with("y", t.get("y").cloned().unwrap_or(Json::from(50))).with("mode", mode.as_str());
    match mode.as_str() {
        "none" => return Ok(Json::Null),
        "custom" => {
            if let Some(src) = custom_image(v, id) {
                return Ok(style(Json::obj().with("src", src)));
            }
        }
        "file" => {
            if let Some(fid) = t.str_of("fileId") {
                if let Some(src) = file_picture(v, &fid)? {
                    return Ok(style(Json::obj().with("src", src).with("fileId", fid)));
                }
            }
        }
        _ => {}
    }
    for r in v.db.query("SELECT id FROM files WHERE project_id=? AND state='present' AND ext='3mf' ORDER BY last_changed DESC LIMIT 3", p![id])? {
        if let Some(src) = file_picture(v, &r.s("id"))? {
            return Ok(style(Json::obj().with("src", src).with("fileId", r.s("id"))));
        }
    }
    let imgs = v.db.query(
        "SELECT id FROM files WHERE project_id=? AND state='present' AND ext IN ('png','jpg','jpeg','webp','gif') AND size<=4194304
         ORDER BY (instr(lower(name),'cover')>0 OR instr(lower(name),'preview')>0 OR instr(lower(name),'thumb')>0 OR instr(lower(name),'screenshot')>0 OR instr(lower(name),'photo')>0) DESC,
                  (instr(lower(rel_path),'icon')>0 OR instr(lower(rel_path),'texture')>0 OR instr(lower(rel_path),'node_modules')>0) ASC, last_changed DESC LIMIT 1",
        p![id],
    )?;
    if let Some(r) = imgs.first() {
        if let Some(src) = file_picture(v, &r.s("id"))? {
            return Ok(style(Json::obj().with("src", src).with("fileId", r.s("id"))));
        }
    }
    Ok(Json::Null)
}

fn thumb_settings(module: &Json) -> Json {
    match module.get("thumb") {
        Some(t @ Json::Obj(_)) => t.clone(),
        // older builds stored only the chosen file id
        _ => match module.str_of("thumbFileId") {
            Some(f) => Json::obj().with("mode", "file").with("fileId", f),
            None => Json::obj(),
        },
    }
}

/// An image file or a .3mf's plate picture as a data URL.
pub fn file_picture(v: &Vault, fid: &str) -> DbResult<Option<String>> {
    let Some(f) = v.db.one("SELECT project_id, rel_path, ext, size, state FROM files WHERE id=?", p![fid])? else { return Ok(None) };
    if f.s("state") != "present" {
        return Ok(None);
    }
    let path = crate::util::safe_join(&root_of(v, &f.s("project_id"))?, &f.s("rel_path"))?;
    if f.s("ext") == "3mf" {
        return Ok(crate::models::threemf_thumbnail(&path));
    }
    if f.i("size") > 8 * 1024 * 1024 {
        return Ok(None);
    }
    let img = crate::files::read_base64(v, fid, 8 * 1024 * 1024)?;
    Ok(Some(format!("data:{};base64,{}", img.str_of("mime").unwrap_or_default(), img.str_of("base64").unwrap_or_default())))
}

fn custom_dir(v: &Vault) -> PathBuf {
    v.data_dir.join("pictures")
}
fn custom_image(v: &Vault, id: &str) -> Option<String> {
    for ext in ["png", "jpg", "webp", "gif"] {
        let p = custom_dir(v).join(format!("{id}.{ext}"));
        if let Ok(d) = std::fs::read(&p) {
            let mime = match ext {
                "jpg" => "image/jpeg",
                e => return Some(format!("data:image/{e};base64,{}", crate::files::base64(&d))),
            };
            return Some(format!("data:{mime};base64,{}", crate::files::base64(&d)));
        }
    }
    None
}

/// Changes the card picture. `patch` may hold mode, fileId, path (an image
/// anywhere on disk: copied into the vault's pictures folder), fit, x, y.
pub fn set_thumbnail(v: &Vault, id: &str, patch: &Json) -> DbResult<Json> {
    let pr = v.db.one("SELECT module_json FROM projects WHERE id=?", p![id])?.ok_or("Project not found")?;
    let mut module = Json::parse(&pr.s("module_json")).unwrap_or(Json::obj());
    let mut t = thumb_settings(&module);
    if let Some(path) = patch.str_of("path") {
        let src = PathBuf::from(&path);
        let ext = src.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        let ext = match ext.as_str() {
            "png" | "webp" | "gif" => ext,
            "jpg" | "jpeg" => "jpg".into(),
            _ => return Err("Choose a PNG, JPG, WebP or GIF image".into()),
        };
        let size = std::fs::metadata(&src).map_err(|e| format!("{}: {e}", src.display()))?.len();
        if size > 8 * 1024 * 1024 {
            return Err("That image is larger than 8 MB; pick a smaller one".into());
        }
        std::fs::create_dir_all(custom_dir(v)).map_err(|e| e.to_string())?;
        for old in ["png", "jpg", "webp", "gif"] {
            let _ = std::fs::remove_file(custom_dir(v).join(format!("{id}.{old}")));
        }
        std::fs::copy(&src, custom_dir(v).join(format!("{id}.{ext}"))).map_err(|e| e.to_string())?;
        t.set("mode", "custom");
        t.remove("fileId");
    }
    for k in ["mode", "fileId", "fit", "x", "y"] {
        if let Some(x) = patch.get(k) {
            t.set(k, x.clone());
        }
    }
    if let Some(m) = t.str_of("mode") {
        if !["auto", "file", "custom", "none"].contains(&m.as_str()) {
            return Err(format!("Unknown picture mode {m}"));
        }
        if m != "custom" {
            for old in ["png", "jpg", "webp", "gif"] {
                let _ = std::fs::remove_file(custom_dir(v).join(format!("{id}.{old}")));
            }
        }
    }
    module.remove("thumbFileId");
    module.set("thumb", t);
    v.db.exec("UPDATE projects SET module_json=? WHERE id=?", p![module.to_string(), id])?;
    thumbnail(v, id)
}

/// Images and .3mf files in the project that can be used as its picture.
pub fn thumbnail_candidates(v: &Vault, id: &str) -> DbResult<Json> {
    let rows = v.db.query(
        "SELECT id, rel_path, ext, size FROM files WHERE project_id=? AND state='present' AND (ext='3mf' OR (ext IN ('png','jpg','jpeg','webp','gif') AND size<=8388608))
         ORDER BY ext='3mf' DESC, last_changed DESC LIMIT 60",
        p![id],
    )?;
    Ok(Json::Arr(rows.iter().map(|r| Json::obj().with("fileId", r.s("id")).with("path", r.s("rel_path")).with("ext", r.s("ext"))).collect()))
}

pub fn get(v: &Vault, id: &str) -> DbResult<Json> {
    let r = v.db.one("SELECT * FROM projects WHERE id=?", p![id])?.ok_or("Project not found")?;
    row_json(v, &r)
}

pub fn list(v: &Vault) -> DbResult<Json> {
    let rows = v.db.query("SELECT * FROM projects ORDER BY favorite DESC, COALESCE(last_opened_at, updated_at) DESC", p![])?;
    let mut out = Vec::new();
    for r in &rows {
        out.push(row_json(v, r)?);
    }
    Ok(Json::Arr(out))
}

pub fn root_of(v: &Vault, id: &str) -> DbResult<PathBuf> {
    let r = v.db.one("SELECT root_path FROM projects WHERE id=?", p![id])?.ok_or("Project not found")?;
    Ok(PathBuf::from(r.s("root_path")))
}

/// Partial update: only keys present in `patch` are changed.
pub fn update(v: &Vault, id: &str, patch: &Json) -> DbResult<Json> {
    let before = v.db.one("SELECT * FROM projects WHERE id=?", p![id])?.ok_or("Project not found")?;
    v.db.tx(|db| {
        if let Some(n) = patch.str_of("name") {
            if n.trim().is_empty() {
                return Err("Project name cannot be empty".into());
            }
            db.exec("UPDATE projects SET name=? WHERE id=?", p![n.trim(), id])?;
        }
        for (k, col) in [("description", "description"), ("icon", "icon"), ("color", "color"), ("ignoreGlobs", "ignore_globs")] {
            if let Some(s) = patch.str_of(k) {
                db.exec(&format!("UPDATE projects SET {col}=? WHERE id=?"), p![s, id])?;
            }
        }
        if let Some(c) = patch.str_of("category") {
            if !CATEGORIES.contains(&c.as_str()) {
                return Err(format!("Unknown category {c}"));
            }
            db.exec("UPDATE projects SET category=? WHERE id=?", p![c, id])?;
        }
        if let Some(s) = patch.str_of("status") {
            ensure_status(v, s.trim())?;
            db.exec("UPDATE projects SET status=? WHERE id=?", p![s.trim(), id])?;
            if s.trim() != before.s("status") {
                timeline::add(v, id, "status", &format!("Status: {} → {}", before.s("status"), s.trim()), "", None, false)?;
            }
        }
        if let Some(pr) = patch.get("progress").and_then(|x| x.as_i64()) {
            db.exec("UPDATE projects SET progress=? WHERE id=?", p![pr.clamp(0, 100), id])?;
        }
        if let Some(f) = patch.get("favorite").and_then(|x| x.as_bool()) {
            db.exec("UPDATE projects SET favorite=? WHERE id=?", p![f, id])?;
        }
        if let Some(g) = patch.str_of("group") {
            let g: String = g.split_whitespace().collect::<Vec<_>>().join(" ");
            if g.chars().count() > 60 {
                return Err("Group names can be at most 60 characters".into());
            }
            db.exec("UPDATE projects SET group_name=? WHERE id=?", p![g, id])?;
        }
        if let Some(m) = patch.get("module") {
            db.exec("UPDATE projects SET module_json=? WHERE id=?", p![m.to_string(), id])?;
        }
        if let Some(Json::Arr(tags)) = patch.get("tags") {
            let t: Vec<String> = tags.iter().filter_map(|x| x.as_str().map(String::from)).collect();
            set_tags(v, "project", id, &t)?;
        }
        db.exec("UPDATE projects SET updated_at=? WHERE id=?", p![now_ms(), id])?;
        Ok(())
    })?;
    search::index_project(v, id)?;
    get(v, id)
}

pub fn touch_opened(v: &Vault, id: &str) -> DbResult<()> {
    v.db.exec("UPDATE projects SET last_opened_at=? WHERE id=?", p![now_ms(), id])?;
    Ok(())
}

/// Re-points a linked project at a folder (e.g. the drive letter changed).
/// The folder must exist. Files are re-matched on the next scan by path.
pub fn relink(v: &Vault, id: &str, new_root: &Path) -> DbResult<Json> {
    let r = v.db.one("SELECT mode, root_path FROM projects WHERE id=?", p![id])?.ok_or("Project not found")?;
    if r.s("mode") != "linked" {
        return Err("Only linked projects can be re-linked".into());
    }
    if !new_root.is_dir() {
        return Err(format!("{} is not an accessible folder", new_root.display()));
    }
    let canon = dunce(new_root);
    v.db.exec(
        "UPDATE projects SET root_path=?, updated_at=? WHERE id=?",
        p![canon.to_string_lossy().to_string(), now_ms(), id],
    )?;
    timeline::add(v, id, "project", "Project folder re-linked", &format!("{} → {}", r.s("root_path"), canon.display()), None, false)?;
    get(v, id)
}

/// Turns a managed project into a linked one that works on `folder` in place,
/// so files there are edited directly instead of copied into app storage.
/// Anything already in the managed folder is copied over first (verified,
/// never overwriting; a differing file with the same name is kept under a
/// new name). The old managed folder is left untouched.
pub fn use_folder(v: &Vault, id: &str, folder: &Path) -> DbResult<Json> {
    let r = v.db.one("SELECT mode, root_path FROM projects WHERE id=?", p![id])?.ok_or("Project not found")?;
    if r.s("mode") != "managed" {
        return Err("This project already works on a folder of yours. Use “Re-link” to point it somewhere else.".into());
    }
    if !folder.is_dir() {
        return Err(format!("{} is not an accessible folder", folder.display()));
    }
    let old = PathBuf::from(r.s("root_path"));
    let new_root = dunce(folder);
    if new_root.starts_with(v.managed_root()) {
        return Err("Choose a folder outside Project Vault's own storage".into());
    }
    let mut copied = 0i64;
    let mut renamed = Vec::new();
    if old.is_dir() {
        let rows = v.db.query("SELECT id, rel_path, sha256 FROM files WHERE project_id=? AND state='present'", p![id])?;
        for f in &rows {
            let src = crate::util::safe_join(&old, &f.s("rel_path"))?;
            let mut dest = crate::util::safe_join(&new_root, &f.s("rel_path"))?;
            if dest.exists() {
                if crate::util::sha256_file(&dest).ok() == f.os("sha256") {
                    continue; // already there
                }
                let name = dest.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                let (stem, ext) = match name.rfind('.') { Some(i) if i > 0 => (name[..i].to_string(), name[i..].to_string()), _ => (name.clone(), String::new()) };
                let mut n = 2;
                while dest.exists() {
                    dest = dest.with_file_name(format!("{stem} (from vault {n}){ext}"));
                    n += 1;
                }
                renamed.push(Json::from(crate::util::rel_path(&new_root, &dest).unwrap_or_default()));
                // the record follows the renamed copy so its notes stay attached
                v.db.exec("UPDATE files SET rel_path=?, name=? WHERE id=?", p![crate::util::rel_path(&new_root, &dest).unwrap_or_default(), dest.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(), f.s("id")])?;
            }
            if let Some(dir) = dest.parent() {
                std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
            }
            std::fs::copy(&src, &dest).map_err(|e| format!("{}: {e}", src.display()))?;
            if crate::util::sha256_file(&dest).ok() != f.os("sha256") {
                return Err(format!("Copy of {} could not be verified; the project was not changed.", f.s("rel_path")));
            }
            copied += 1;
        }
    }
    v.db.exec("UPDATE projects SET mode='linked', root_path=?, updated_at=? WHERE id=?", p![new_root.to_string_lossy().to_string(), now_ms(), id])?;
    timeline::add(v, id, "project", "Project now works on your folder", &format!("{} ({} file(s) copied over from app storage)", new_root.display(), copied), None, false)?;
    let scan = crate::files::scan(v, id)?;
    Ok(get(v, id)?.with("copied", copied).with("renamed", Json::Arr(renamed)).with("oldFolder", old.to_string_lossy().to_string()).with("scan", scan.json()))
}

/// Removes the project from the vault. Never touches a linked folder.
/// With `delete_managed_files`, a managed project's folder is sent to the
/// Recycle Bin first; if that fails nothing is removed, so the project and
/// its folder never get out of step.
pub fn remove(v: &Vault, id: &str, delete_managed_files: bool) -> DbResult<Json> {
    let r = v.db.one("SELECT mode, root_path FROM projects WHERE id=?", p![id])?.ok_or("Project not found")?;
    let root = PathBuf::from(r.s("root_path"));
    let mut deleted = false;
    if delete_managed_files && r.s("mode") == "managed" {
        if !root.starts_with(v.managed_root()) || root == v.managed_root() {
            return Err(format!("Refusing to delete {}: it is not a project folder inside the vault's managed storage", root.display()));
        }
        if root.exists() {
            crate::files::to_recycle_bin(&root)
                .map_err(|e| format!("The folder couldn't be moved to the Recycle Bin ({e}). Nothing was removed."))?;
            deleted = true;
        }
    }
    v.db.tx(|db| {
        search::unindex_project(v, id)?;
        db.exec("DELETE FROM entity_tags WHERE entity_id IN (SELECT id FROM files WHERE project_id=?) OR entity_id=?", p![id, id])?;
        db.exec("DELETE FROM projects WHERE id=?", p![id])?;
        Ok(())
    })?;
    let _ = crate::deploy::forget(v, id);
    Ok(Json::obj().with("removed", true).with("filesDeleted", deleted))
}

/// What the dashboard shows above the project grid: projects that need
/// attention (with the reasons) and the latest file changes across projects.
pub fn dashboard(v: &Vault, limit: i64) -> DbResult<Json> {
    let mut attention = Vec::new();
    for r in v.db.query("SELECT * FROM projects ORDER BY lower(name)", p![])? {
        let j = row_json(v, &r)?;
        let mut reasons = Vec::new();
        if !j.get("rootAvailable").and_then(|x| x.as_bool()).unwrap_or(true) {
            reasons.push(Json::obj().with("kind", "offline").with("text", "Folder not reachable"));
        }
        let n = |k: &str| j.get(k).and_then(|x| x.as_i64()).unwrap_or(0);
        if n("missingFiles") > 0 {
            reasons.push(Json::obj().with("kind", "missing").with("count", n("missingFiles")).with("text", format!("{} missing file{}", n("missingFiles"), if n("missingFiles") == 1 { "" } else { "s" })));
        }
        if n("pendingReviews") > 0 {
            reasons.push(Json::obj().with("kind", "reviews").with("count", n("pendingReviews")).with("text", format!("{} file match{} to confirm", n("pendingReviews"), if n("pendingReviews") == 1 { "" } else { "es" })));
        }
        if n("staleDocs") > 0 {
            reasons.push(Json::obj().with("kind", "staleDocs").with("count", n("staleDocs")).with("text", format!("{} file note{} may be out of date", n("staleDocs"), if n("staleDocs") == 1 { "" } else { "s" })));
        }
        if !reasons.is_empty() {
            attention.push(Json::obj().with("projectId", r.s("id")).with("name", r.s("name")).with("category", r.s("category")).with("icon", r.s("icon")).with("color", r.s("color")).with("reasons", Json::Arr(reasons)));
        }
    }
    // latest change per file (first indexing isn't a change)
    let recent = v.db.query(
        "SELECT e.file_id, e.kind, MAX(e.at) AS at, f.rel_path, f.name, f.ext, f.state, p.id AS project_id, p.name AS project_name
         FROM file_events e JOIN files f ON f.id=e.file_id JOIN projects p ON p.id=e.project_id
         WHERE e.kind IN ('added','modified','moved','restored','restored-version','renamed','imported')
         GROUP BY e.file_id ORDER BY at DESC LIMIT ?",
        p![limit.clamp(1, 50)],
    )?;
    Ok(Json::obj().with("attention", Json::Arr(attention)).with(
        "recent",
        Json::Arr(
            recent
                .iter()
                .map(|r| {
                    Json::obj()
                        .with("fileId", r.s("file_id"))
                        .with("path", r.s("rel_path"))
                        .with("name", r.s("name"))
                        .with("ext", r.s("ext"))
                        .with("present", r.s("state") == "present")
                        .with("change", r.s("kind"))
                        .with("at", r.i("at"))
                        .with("projectId", r.s("project_id"))
                        .with("projectName", r.s("project_name"))
                })
                .collect(),
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;
    #[test]
    fn card_picture_choice() {
        let (t, v) = vault();
        let d = t.path().join("p");
        std::fs::create_dir_all(d.join("textures")).unwrap();
        std::fs::write(d.join("textures/grass.png"), "png1").unwrap();
        std::fs::write(d.join("Cover shot.jpg"), "jpg").unwrap();
        std::fs::write(d.join("main.gsc"), "init(){}").unwrap();
        let id = create(&v, NewProject { name: "P".into(), description: "".into(), category: "game".into(), status: "".into(), link_path: Some(d.clone()), module: Json::obj() }).unwrap().str_of("id").unwrap();
        crate::files::scan(&v, &id).unwrap();
        let th = thumbnail(&v, &id).unwrap();
        assert!(th.str_of("src").unwrap().starts_with("data:image/jpeg;base64,"), "cover-like name wins");
        let grass = v.db.one("SELECT id FROM files WHERE name='grass.png'", p![]).unwrap().unwrap().s("id");
        set_thumbnail(&v, &id, &Json::obj().with("mode", "file").with("fileId", grass.as_str()).with("y", 20)).unwrap();
        let th = thumbnail(&v, &id).unwrap();
        assert_eq!(th.str_of("fileId").unwrap(), grass);
        assert_eq!(th.get("y").unwrap().as_i64(), Some(20));
        // a picture from anywhere on disk is copied into the vault
        let ext_img = t.path().join("Desktop/me.png");
        std::fs::create_dir_all(ext_img.parent().unwrap()).unwrap();
        std::fs::write(&ext_img, "PNGDATA").unwrap();
        set_thumbnail(&v, &id, &Json::obj().with("path", ext_img.to_string_lossy().to_string())).unwrap();
        std::fs::remove_file(&ext_img).unwrap();
        assert!(thumbnail(&v, &id).unwrap().str_of("src").unwrap().starts_with("data:image/png"));
        set_thumbnail(&v, &id, &Json::obj().with("mode", "none")).unwrap();
        assert!(matches!(thumbnail(&v, &id).unwrap(), Json::Null));
        assert!(!v.data_dir.join("pictures").join(format!("{id}.png")).exists(), "custom copy removed when no longer used");
        assert!(thumbnail_candidates(&v, &id).unwrap().as_arr().unwrap().len() == 2);
        let pj = get(&v, &id).unwrap();
        assert!(pj.str_of("lastFile").is_some());
    }

    #[test]
    fn remove_can_delete_managed_folder_only() {
        let (t, v) = vault();
        let keep = create(&v, NewProject { name: "Keep".into(), description: "".into(), category: "game".into(), status: "".into(), link_path: None, module: Json::obj() }).unwrap();
        let gone = create(&v, NewProject { name: "Gone".into(), description: "".into(), category: "game".into(), status: "".into(), link_path: None, module: Json::obj() }).unwrap();
        let keep_root = PathBuf::from(keep.str_of("rootPath").unwrap());
        let gone_root = PathBuf::from(gone.str_of("rootPath").unwrap());
        std::fs::write(gone_root.join("a.txt"), "x").unwrap();
        let r = remove(&v, &keep.str_of("id").unwrap(), false).unwrap();
        assert_eq!(r.get("filesDeleted").unwrap().as_bool(), Some(false));
        assert!(keep_root.exists(), "folder kept unless asked");
        let r = remove(&v, &gone.str_of("id").unwrap(), true).unwrap();
        assert_eq!(r.get("filesDeleted").unwrap().as_bool(), Some(true));
        assert!(!gone_root.exists());
        // a linked folder is never deleted, even when asked
        let d = t.path().join("mine");
        std::fs::create_dir_all(&d).unwrap();
        let l = create(&v, NewProject { name: "L".into(), description: "".into(), category: "game".into(), status: "".into(), link_path: Some(d.clone()), module: Json::obj() }).unwrap();
        remove(&v, &l.str_of("id").unwrap(), true).unwrap();
        assert!(d.exists());
    }

    #[test]
    fn dashboard_lists_attention_and_recent_changes() {
        let (t, v) = vault();
        let d = t.path().join("p");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("a.gsc"), "1").unwrap();
        std::fs::write(d.join("b.gsc"), "1").unwrap();
        let id = create(&v, NewProject { name: "P".into(), description: "".into(), category: "game".into(), status: "".into(), link_path: Some(d.clone()), module: Json::obj() }).unwrap().str_of("id").unwrap();
        crate::files::scan(&v, &id).unwrap();
        let db = dashboard(&v, 10).unwrap();
        assert!(db.get("attention").unwrap().as_arr().unwrap().is_empty());
        assert!(db.get("recent").unwrap().as_arr().unwrap().is_empty(), "first indexing isn't a change");
        let a = v.db.one("SELECT id FROM files WHERE name='a.gsc'", p![]).unwrap().unwrap().s("id");
        crate::files::docs_save(&v, &a, &Json::obj().with("purpose", "Entry point")).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(15));
        std::fs::write(d.join("a.gsc"), "22").unwrap();
        std::fs::remove_file(d.join("b.gsc")).unwrap();
        crate::files::scan(&v, &id).unwrap();
        let db = dashboard(&v, 10).unwrap();
        let reasons: Vec<String> = db.get("attention").unwrap().as_arr().unwrap()[0].get("reasons").unwrap().as_arr().unwrap().iter().map(|r| r.str_of("kind").unwrap()).collect();
        assert_eq!(reasons, vec!["missing", "staleDocs"]);
        let recent = db.get("recent").unwrap().as_arr().unwrap();
        assert_eq!(recent[0].str_of("path").unwrap(), "a.gsc");
        assert_eq!(recent[0].str_of("change").unwrap(), "modified");
    }

    #[test]
    fn create_linked_and_managed() {
        let (t, v) = vault();
        let dir = t.path().join("omen");
        std::fs::create_dir_all(&dir).unwrap();
        let p = create(
            &v,
            NewProject {
                name: "OMEN v2".into(),
                description: "".into(),
                category: "software".into(),
                status: "Active".into(),
                link_path: Some(dir.clone()),
                module: Json::obj(),
            },
        )
        .unwrap();
        assert_eq!(p.str_of("mode").unwrap(), "linked");
        // linking the same folder twice is refused
        let again = create(&v, NewProject { name: "dup".into(), description: "".into(), category: "general".into(), status: "".into(), link_path: Some(dir.clone()), module: Json::obj() });
        assert!(again.is_err());
        let m = create(&v, NewProject { name: "Print stuff".into(), description: "".into(), category: "3d-printing".into(), status: "Custom Thing".into(), link_path: None, module: Json::obj() }).unwrap();
        assert!(Path::new(&m.str_of("rootPath").unwrap()).is_dir());
        assert!(statuses(&v).unwrap().to_string().contains("Custom Thing"));
        let id = p.str_of("id").unwrap();
        let u = update(&v, &id, &Json::parse(r#"{"tags":["gsc","mw2"],"favorite":true,"progress":140,"group":"  MW2   mods "}"#).unwrap()).unwrap();
        assert_eq!(u.str_of("group").unwrap(), "MW2 mods");
        assert_eq!(u.get("progress").unwrap().as_i64(), Some(100));
        assert_eq!(u.get("tags").unwrap().as_arr().unwrap().len(), 2);
        let r = remove(&v, &id, true).unwrap();
        assert_eq!(r.get("filesDeleted").unwrap().as_bool(), Some(false));
        assert!(dir.is_dir(), "linked folder must never be deleted");
    }
}

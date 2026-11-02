//! File indexing, reconciliation, imports, and file-level documentation.
//!
//! Identity model
//! --------------
//! Every file gets a random, stable `id` the first time it is seen. Notes,
//! tags, links and history hang off that id, never off the path. A scan
//! compares the folder with the index and classifies each difference:
//!
//! * same path, same size+mtime        -> unchanged (not re-hashed)
//! * same path, content changed        -> modified (doc kept, flagged)
//! * path gone + new path, same SHA-256,
//!   and that hash is unique on both sides -> moved/renamed (doc follows)
//! * anything ambiguous (duplicate hashes, same name elsewhere with different
//!   content)                            -> both kept separate, and a review
//!                                          item is created for the user
//! * path gone, no match                -> marked missing (doc retained)
//!
//! Documentation is therefore never silently attached to a different file.
//! If the project folder itself is unreachable (unplugged drive), the scan
//! aborts without marking anything missing.

use crate::db::{DbResult, Row};
use crate::json::Json;
use crate::util::{extension_of, new_id, now_ms, rel_path, safe_join, sha256_file, system_time_ms, unique_dest};
use crate::{p, projects, search, timeline, Vault};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

const ALWAYS_IGNORE_DIRS: &[&str] = &[".git", ".hg", ".svn", "node_modules", "__pycache__", ".vs", "$RECYCLE.BIN", "System Volume Information"];
const ALWAYS_IGNORE_FILES: &[&str] = &["Thumbs.db", "desktop.ini", ".DS_Store"];
const MAX_FILES_PER_PROJECT: usize = 500_000;

pub fn classify(ext: &str) -> &'static str {
    match ext {
        "rs" | "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" | "py" | "cs" | "c" | "h" | "cpp" | "hpp" | "cc" | "go"
        | "java" | "kt" | "lua" | "gsc" | "csc" | "gsh" | "gd" | "shader" | "hlsl" | "glsl" | "sh" | "bat" | "ps1" | "cmd"
        | "rb" | "php" | "swift" | "sql" | "css" | "scss" | "html" | "vue" | "svelte" | "ino" => "code",
        "md" | "markdown" | "txt" | "rtf" | "pdf" | "doc" | "docx" | "odt" | "xls" | "xlsx" | "csv" | "ppt" | "pptx" | "srt" | "vtt" => "document",
        "json" | "yaml" | "yml" | "toml" | "xml" | "ini" | "cfg" | "conf" | "env" | "lock" | "toc" => "config",
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "svg" | "ico" | "tga" | "psd" | "psb" | "tif" | "tiff" | "exr" | "hdr" | "ai" | "afdesign"
        | "afphoto" | "afpub" | "kra" | "xcf" | "clip" | "indd" | "sketch" | "fig" | "xd" | "procreate" | "blp" | "dds" => "image",
        "stl" | "obj" | "3mf" | "ply" | "amf" => "mesh",
        "step" | "stp" | "f3d" | "f3z" | "fcstd" | "iges" | "igs" | "scad" | "blend" | "fbx" | "dae" | "gltf" | "glb" | "max" | "ma" | "mb" | "c4d"
        | "hip" | "hipnc" | "ztl" | "zpr" | "usd" | "usda" | "usdz" | "abc" | "spp" | "sbs" | "sbsar" | "sldprt" | "sldasm" | "slddrw" | "ipt" | "iam"
        | "idw" | "dwg" | "dxf" | "3dm" | "catpart" | "catproduct" | "x_t" | "x_b" => "model",
        "gcode" | "bgcode" | "gco" | "nc" | "tap" | "ngc" => "gcode",
        "zip" | "7z" | "rar" | "tar" | "gz" | "xz" | "bz2" | "iwd" | "ff" => "archive",
        "unity" | "prefab" | "asset" | "mat" | "meta" | "controller" | "anim" | "asmdef" | "uasset" | "umap" | "tscn" | "tres" => "engine",
        "wav" | "mp3" | "ogg" | "flac" | "aiff" | "m4a" | "als" | "flp" | "rpp" => "audio",
        "mp4" | "mov" | "mkv" | "webm" | "avi" | "mxf" | "m4v" | "veg" | "vf" | "prproj" | "drp" | "fcpxml" | "kdenlive" | "mlt" | "wfp" | "aep"
        | "aepx" | "mogrt" | "comp" => "video",
        "kicad_pcb" | "kicad_sch" | "kicad_pro" | "sch" | "brd" | "gbr" | "drl" | "fzz" => "electronics",
        "exe" | "dll" | "so" | "dylib" | "bin" | "elf" | "hex" | "uf2" => "binary",
        "otf" | "ttf" | "woff" | "woff2" => "font",
        _ => "other",
    }
}

pub fn is_text_ext(ext: &str) -> bool {
    matches!(classify(ext), "code" | "config")
        || matches!(ext, "md" | "markdown" | "txt" | "csv" | "log" | "gitignore" | "gcode" | "scad" | "meta" | "asmdef" | "kicad_sch" | "kicad_pro" | "tres" | "tscn" | "srt" | "vtt" | "fcpxml" | "kdenlive" | "mlt" | "aepx" | "jsx" | "nc" | "ngc" | "tap")
}

// ------------------------------------------------------------------ ignore

pub struct Ignore {
    paths: HashSet<String>,
    dirs: HashSet<String>,
    dir_paths: Vec<String>,
    exts: HashSet<String>,
    names: HashSet<String>,
}

impl Ignore {
    /// One rule per line: `Name/` (directory name anywhere), `a/b/` (directory
    /// at a project-relative path), `*.ext`, or an exact file name.
    pub fn parse(rules: &str) -> Ignore {
        let mut ig = Ignore { paths: HashSet::new(), dirs: HashSet::new(), dir_paths: Vec::new(), exts: HashSet::new(), names: HashSet::new() };
        for d in ALWAYS_IGNORE_DIRS {
            ig.dirs.insert(d.to_lowercase());
        }
        for f in ALWAYS_IGNORE_FILES {
            ig.names.insert(f.to_lowercase());
        }
        for line in rules.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
            let l = line.replace('\\', "/").to_lowercase();
            if let Some(e) = l.strip_prefix("*.") {
                ig.exts.insert(e.to_string());
            } else if let Some(d) = l.strip_suffix('/') {
                let d = d.trim_start_matches('/');
                if d.contains('/') { ig.dir_paths.push(d.to_string()) } else { ig.dirs.insert(d.to_string()); }
            } else if l.contains('/') {
                ig.paths.insert(l.trim_start_matches('/').to_string());
            } else {
                ig.names.insert(l);
            }
        }
        ig
    }
    pub(crate) fn skip_dir(&self, name: &str, rel: &str) -> bool {
        let n = name.to_lowercase();
        let r = rel.to_lowercase();
        self.dirs.contains(&n) || self.dir_paths.iter().any(|d| *d == r)
    }
    pub(crate) fn skip_file(&self, name: &str, rel: &str) -> bool {
        let n = name.to_lowercase();
        self.paths.contains(&rel.to_lowercase()) || self.names.contains(&n) || self.exts.contains(&extension_of(&n)) || n.starts_with(".pv-tmp-")
    }
}

// ------------------------------------------------------------------ walk

pub struct Seen {
    pub size: i64,
    pub mtime: i64,
    /// OneDrive/cloud placeholder whose content is not on this PC. Reading it
    /// would force a download, so it is indexed without hashing.
    pub cloud_only: bool,
}

#[cfg(windows)]
fn is_cloud_placeholder(m: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_OFFLINE: u32 = 0x1000;
    const FILE_ATTRIBUTE_RECALL_ON_OPEN: u32 = 0x40000;
    const FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS: u32 = 0x400000;
    m.file_attributes() & (FILE_ATTRIBUTE_OFFLINE | FILE_ATTRIBUTE_RECALL_ON_OPEN | FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS) != 0
}
#[cfg(not(windows))]
fn is_cloud_placeholder(_m: &std::fs::Metadata) -> bool {
    false
}

pub struct Walk {
    pub files: BTreeMap<String, Seen>,
    pub errors: Vec<String>,
}

/// Read-only walk. Never locks or opens files for writing, never follows
/// symlinks/junctions (avoids loops and escaping the project root).
pub fn walk(root: &Path, ig: &Ignore) -> Walk {
    let mut w = Walk { files: BTreeMap::new(), errors: Vec::new() };
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let rd = match std::fs::read_dir(&dir) {
            Ok(r) => r,
            Err(e) => {
                w.errors.push(format!("{}: {e}", dir.display()));
                continue;
            }
        };
        for ent in rd {
            let ent = match ent {
                Ok(e) => e,
                Err(e) => {
                    w.errors.push(format!("{}: {e}", dir.display()));
                    continue;
                }
            };
            let path = ent.path();
            let name = ent.file_name().to_string_lossy().to_string();
            let ft = match ent.file_type() {
                Ok(t) => t,
                Err(e) => {
                    w.errors.push(format!("{}: {e}", path.display()));
                    continue;
                }
            };
            if ft.is_symlink() {
                continue;
            }
            let Some(rel) = rel_path(root, &path) else { continue };
            if ft.is_dir() {
                if !ig.skip_dir(&name, &rel) {
                    stack.push(path);
                }
            } else if ft.is_file() && !ig.skip_file(&name, &rel) {
                match ent.metadata() {
                    Ok(m) => {
                        let mtime = m.modified().map(system_time_ms).unwrap_or(0);
                        w.files.insert(rel, Seen { size: m.len() as i64, mtime, cloud_only: is_cloud_placeholder(&m) });
                    }
                    Err(e) => w.errors.push(format!("{}: {e}", path.display())),
                }
                if w.files.len() > MAX_FILES_PER_PROJECT {
                    w.errors.push(format!("Stopped after {MAX_FILES_PER_PROJECT} files; add ignore rules for generated folders."));
                    return w;
                }
            }
        }
    }
    w
}

// ------------------------------------------------------------------ scan

#[derive(Default, Debug)]
pub struct ScanReport {
    pub total: usize,
    pub added: usize,
    pub modified: usize,
    pub moved: usize,
    pub missing: usize,
    pub restored: usize,
    pub needs_review: usize,
    pub errors: Vec<String>,
    pub hashed: usize,
    pub versioned: usize,
}

impl ScanReport {
    pub fn json(&self) -> Json {
        Json::obj()
            .with("total", self.total)
            .with("added", self.added)
            .with("modified", self.modified)
            .with("moved", self.moved)
            .with("missing", self.missing)
            .with("restored", self.restored)
            .with("needsReview", self.needs_review)
            .with("hashed", self.hashed)
            .with("versioned", self.versioned)
            .with("errors", Json::Arr(self.errors.iter().take(50).map(|e| Json::from(e.as_str())).collect()))
            .with("errorCount", self.errors.len())
    }
    fn changes(&self) -> usize {
        self.added + self.modified + self.moved + self.missing + self.restored
    }
}

fn event(v: &Vault, pid: &str, fid: &str, kind: &str, detail: &str, sha: Option<&str>) -> DbResult<()> {
    v.db.exec(
        "INSERT INTO file_events(id,project_id,file_id,kind,detail,sha256,at) VALUES (?,?,?,?,?,?,?)",
        p![new_id(), pid, fid, kind, detail, sha.map(String::from), now_ms()],
    )?;
    Ok(())
}

fn file_name_of(rel: &str) -> String {
    rel.rsplit('/').next().unwrap_or(rel).to_string()
}

pub fn scan(v: &Vault, project_id: &str) -> DbResult<ScanReport> {
    let pr = v.db.one("SELECT root_path, ignore_globs FROM projects WHERE id=?", p![project_id])?.ok_or("Project not found")?;
    let root = PathBuf::from(pr.s("root_path"));
    if !root.is_dir() {
        v.db.exec("UPDATE projects SET root_available=0 WHERE id=?", p![project_id])?;
        return Err(format!(
            "The project folder {} is not reachable (drive disconnected, renamed or deleted?). Nothing was changed. \
             Reconnect the drive or use \"Re-link folder\".",
            root.display()
        ));
    }
    let ig = Ignore::parse(&pr.s("ignore_globs"));
    let w = walk(&root, &ig);
    let mut rep = ScanReport { errors: w.errors, ..Default::default() };
    rep.total = w.files.len();
    let now = now_ms();
    let first_scan = v.db.scalar_i64("SELECT count(*) FROM files WHERE project_id=?", p![project_id])? == 0;

    let existing = v.db.query("SELECT * FROM files WHERE project_id=?", p![project_id])?;
    let by_path: HashMap<String, &Row> = existing.iter().map(|r| (r.s("rel_path"), r)).collect();

    let hash = |rel: &str, rep: &mut ScanReport| -> Option<String> {
        if w.files.get(rel).map(|s| s.cloud_only).unwrap_or(false) {
            return None;
        }
        let ap = safe_join(&root, rel).ok()?;
        match sha256_file(&ap) {
            Ok(h) => {
                rep.hashed += 1;
                Some(h)
            }
            Err(e) => {
                rep.errors.push(format!("{rel}: {e}"));
                None
            }
        }
    };

    // What this scan changed, for the timeline entry's "what changed" list.
    let mut changes: Vec<Json> = Vec::new();
    let change = |kind: &str, fid: &str, path: &str| Json::obj().with("change", kind).with("fileId", fid).with("path", path);
    v.db.tx(|db| {
        // 1. paths that exist both on disk and in the index
        for (rel, seen) in &w.files {
            let Some(r) = by_path.get(rel) else { continue };
            let fid = r.s("id");
            let was_missing = r.s("state") != "present"; // missing, or deleted earlier and now back
            if !was_missing && r.i("size") == seen.size && r.i("mtime") == seen.mtime && r.os("sha256").is_some() {
                continue; // unchanged: no write, keeps rescans of big trees cheap
            }
            let h = hash(rel, &mut rep);
            let content_changed = h.is_some() && h != r.os("sha256");
            db.exec(
                "UPDATE files SET size=?, mtime=?, sha256=COALESCE(?, sha256), state='present', last_seen=?,
                   last_changed=CASE WHEN ? THEN ? ELSE last_changed END,
                   changed_since_review=CASE WHEN ? THEN 1 ELSE changed_since_review END
                 WHERE id=?",
                p![seen.size, seen.mtime, h.clone(), now, content_changed, now, content_changed, fid.clone()],
            )?;
            if was_missing {
                rep.restored += 1;
                event(v, project_id, &fid, "restored", "File is present again", h.as_deref())?;
                changes.push(change("restored", &fid, rel));
            }
            if content_changed {
                rep.modified += 1;
                event(v, project_id, &fid, "modified", "Content changed outside Project Vault", h.as_deref())?;
                let mut c = change("modified", &fid, rel).with("to", h.clone().unwrap_or_default());
                if let Some(old) = r.os("sha256") { c.set("from", old); }
                changes.push(c);
            }
        }

        // 2. vanished (present in index, absent on disk) + previously missing
        let disk: HashSet<&String> = w.files.keys().collect();
        // files deleted through Project Vault stay deleted; they don't take part in matching
        let gone: Vec<&Row> = existing.iter().filter(|r| !disk.contains(&r.s("rel_path")) && r.s("state") != "deleted").collect();
        // 3. brand-new paths
        let mut new_files: Vec<(String, i64, i64, Option<String>)> = Vec::new();
        for (rel, seen) in &w.files {
            if !by_path.contains_key(rel) {
                let h = hash(rel, &mut rep);
                new_files.push((rel.clone(), seen.size, seen.mtime, h));
            }
        }

        // 4. hash matching between gone and new
        let mut gone_by_hash: HashMap<String, Vec<&Row>> = HashMap::new();
        for g in &gone {
            if let Some(h) = g.os("sha256") {
                gone_by_hash.entry(h).or_default().push(g);
            }
        }
        let mut new_by_hash: HashMap<String, Vec<usize>> = HashMap::new();
        for (i, n) in new_files.iter().enumerate() {
            if let Some(h) = &n.3 {
                new_by_hash.entry(h.clone()).or_default().push(i);
            }
        }
        let mut consumed_new: HashSet<usize> = HashSet::new();
        let mut consumed_gone: HashSet<String> = HashSet::new();
        for (h, gs) in &gone_by_hash {
            let Some(ns) = new_by_hash.get(h) else { continue };
            if gs.len() == 1 && ns.len() == 1 {
                // also require the hash to be unique among files still present,
                // otherwise a copy could be mistaken for the moved original
                let present_dupes = db.scalar_i64(
                    "SELECT count(*) FROM files WHERE project_id=? AND sha256=? AND state='present' AND id!=?",
                    p![project_id, h.clone(), gs[0].s("id")],
                )?;
                let gone_present = gs[0].s("state") == "present";
                if present_dupes == 0 {
                    let g = gs[0];
                    let n = &new_files[ns[0]];
                    let fid = g.s("id");
                    db.exec(
                        "UPDATE files SET rel_path=?, name=?, ext=?, kind=?, size=?, mtime=?, state='present', last_seen=? WHERE id=?",
                        p![n.0.clone(), file_name_of(&n.0), extension_of(&n.0), classify(&extension_of(&n.0)), n.1, n.2, now, fid.clone()],
                    )?;
                    event(v, project_id, &fid, "moved", &format!("{} → {}", g.s("rel_path"), n.0), Some(h))?;
                    changes.push(change(if gone_present { "moved" } else { "restored" }, &fid, &n.0).with("fromPath", g.s("rel_path")));
                    if gone_present { rep.moved += 1 } else { rep.restored += 1 }
                    consumed_new.insert(ns[0]);
                    consumed_gone.insert(fid);
                }
            }
        }

        // 5. remaining new files are inserted as new identities
        let mut inserted: Vec<(String, usize)> = Vec::new();
        for (i, n) in new_files.iter().enumerate() {
            if consumed_new.contains(&i) {
                continue;
            }
            let fid = new_id();
            let ext = extension_of(&n.0);
            db.exec(
                "INSERT INTO files(id,project_id,rel_path,name,ext,kind,size,mtime,sha256,state,first_seen,last_seen,last_changed)
                 VALUES (?,?,?,?,?,?,?,?,?,'present',?,?,?)",
                p![fid.clone(), project_id, n.0.clone(), file_name_of(&n.0), ext.clone(), classify(&ext), n.1, n.2, n.3.clone(), now, now, now],
            )?;
            if !first_scan {
                event(v, project_id, &fid, "added", "", n.3.as_deref())?;
                changes.push(change("added", &fid, &n.0).with("to", n.3.clone().unwrap_or_default()));
            }
            rep.added += 1;
            inserted.push((fid, i));
        }

        // 6. remaining gone files become missing; queue reviews for ambiguity
        for g in &gone {
            let gid = g.s("id");
            if consumed_gone.contains(&gid) {
                continue;
            }
            if g.s("state") == "present" {
                db.exec("UPDATE files SET state='missing' WHERE id=?", p![gid.clone()])?;
                event(v, project_id, &gid, "missing", &format!("No longer at {}", g.s("rel_path")), None)?;
                changes.push(change("missing", &gid, &g.s("rel_path")));
                rep.missing += 1;
            }
            let gname = g.s("name").to_lowercase();
            let gh = g.os("sha256");
            for (nid, i) in &inserted {
                let n = &new_files[*i];
                let reason = if gh.is_some() && gh == n.3 {
                    Some("identical content")
                } else if file_name_of(&n.0).to_lowercase() == gname {
                    Some("same file name, different content")
                } else {
                    None
                };
                if let Some(reason) = reason {
                    let c = db.exec(
                        "INSERT OR IGNORE INTO reconcile_reviews(id,project_id,missing_file_id,candidate_file_id,reason,created_at) VALUES (?,?,?,?,?,?)",
                        p![new_id(), project_id, gid.clone(), nid.clone(), reason, now],
                    )?;
                    rep.needs_review += c;
                }
            }
        }

        db.exec("UPDATE projects SET last_scan_at=?, root_available=1 WHERE id=?", p![now, project_id])?;
        if first_scan && rep.added > 0 {
            timeline::add(v, project_id, "files", &format!("Indexed {} files", rep.added), "", None, false)?;
        } else if rep.changes() > 0 {
            let mut parts = Vec::new();
            for (n, label) in [(rep.added, "added"), (rep.modified, "modified"), (rep.moved, "moved/renamed"), (rep.missing, "missing"), (rep.restored, "restored")] {
                if n > 0 {
                    parts.push(format!("{n} {label}"));
                }
            }
            changes.sort_by(|a, b| a.str_of("path").cmp(&b.str_of("path")));
            let total = changes.len();
            changes.truncate(500);
            let data = Json::obj().with("changes", Json::Arr(changes.clone())).with("total", total);
            timeline::add_with_data(v, project_id, "files", &format!("File changes detected: {}", parts.join(", ")), "", None, false, Some(&data))?;
        }
        Ok(())
    })?;

    // index text for new/changed rows (outside the scan tx keeps it short)
    let touched = v.db.query(
        "SELECT id FROM files WHERE project_id=? AND first_seen=?
         UNION SELECT file_id FROM file_events WHERE project_id=? AND at>=?",
        p![project_id, now, project_id, now],
    )?;
    v.db.tx(|_| {
        for r in &touched {
            search::index_file(v, &r.s("id"))?;
        }
        Ok(())
    })?;
    // keep versions of eligible files (see versions.rs); failures are reported, not fatal
    match crate::versions::capture(v, project_id) {
        Ok((n, errs)) => {
            rep.versioned = n;
            rep.errors.extend(errs);
        }
        Err(e) => rep.errors.push(format!("Saving versions failed: {e}")),
    }
    Ok(rep)
}

// ------------------------------------------------------------------ reviews

pub fn reviews(v: &Vault, project_id: &str) -> DbResult<Json> {
    let rows = v.db.query(
        "SELECT r.id, r.reason, r.created_at,
                m.id AS mid, m.rel_path AS mpath, m.size AS msize,
                c.id AS cid, c.rel_path AS cpath, c.size AS csize,
                (SELECT count(*) FROM file_docs d WHERE d.file_id=m.id AND (d.purpose!='' OR d.notes_md!='')) AS mdoc
         FROM reconcile_reviews r JOIN files m ON m.id=r.missing_file_id JOIN files c ON c.id=r.candidate_file_id
         WHERE r.project_id=? ORDER BY r.created_at",
        p![project_id],
    )?;
    Ok(Json::Arr(
        rows.iter()
            .map(|r| {
                Json::obj()
                    .with("id", r.s("id"))
                    .with("reason", r.s("reason"))
                    .with("missing", Json::obj().with("id", r.s("mid")).with("path", r.s("mpath")).with("size", r.i("msize")).with("hasDocs", r.i("mdoc") > 0))
                    .with("candidate", Json::obj().with("id", r.s("cid")).with("path", r.s("cpath")).with("size", r.i("csize")))
            })
            .collect(),
    ))
}

/// Resolves a review. `accept` = the candidate IS the missing file: the
/// candidate's location is merged into the original identity (keeping all
/// documentation/history) and the candidate row is removed. `reject` = they
/// are different files; the review is dismissed.
pub fn resolve_review(v: &Vault, review_id: &str, accept: bool) -> DbResult<Json> {
    let r = v.db.one("SELECT * FROM reconcile_reviews WHERE id=?", p![review_id])?.ok_or("Review not found")?;
    let (pid, mid, cid) = (r.s("project_id"), r.s("missing_file_id"), r.s("candidate_file_id"));
    v.db.tx(|db| {
        if accept {
            let c = db.one("SELECT * FROM files WHERE id=?", p![cid.clone()])?.ok_or("Candidate no longer exists")?;
            let m = db.one("SELECT * FROM files WHERE id=?", p![mid.clone()])?.ok_or("File no longer exists")?;
            if m.s("state") != "missing" {
                return Err("The original file is present again; this review is obsolete.".into());
            }
            // move candidate-owned data to the original identity, then drop candidate
            db.exec("UPDATE OR IGNORE links SET a_id=? WHERE a_type='file' AND a_id=?", p![mid.clone(), cid.clone()])?;
            db.exec("UPDATE OR IGNORE links SET b_id=? WHERE b_type='file' AND b_id=?", p![mid.clone(), cid.clone()])?;
            db.exec("UPDATE OR IGNORE entity_tags SET entity_id=? WHERE entity_type='file' AND entity_id=?", p![mid.clone(), cid.clone()])?;
            let cand_doc = db.one("SELECT 1 FROM file_docs WHERE file_id=? AND (purpose!='' OR notes_md!='')", p![cid.clone()])?;
            let orig_doc = db.one("SELECT 1 FROM file_docs WHERE file_id=?", p![mid.clone()])?;
            if cand_doc.is_some() && orig_doc.is_some() {
                return Err("Both files have documentation. Copy what you need from the newer one, clear it, then accept.".into());
            }
            if cand_doc.is_some() {
                db.exec("UPDATE file_docs SET file_id=? WHERE file_id=?", p![mid.clone(), cid.clone()])?;
            }
            search::unindex(v, "file", &cid)?;
            db.exec("DELETE FROM files WHERE id=?", p![cid.clone()])?;
            let changed = c.os("sha256") != m.os("sha256");
            db.exec(
                "UPDATE files SET rel_path=?, name=?, ext=?, kind=?, size=?, mtime=?, sha256=?, state='present', last_seen=?,
                   changed_since_review = CASE WHEN ? THEN 1 ELSE changed_since_review END,
                   last_changed = CASE WHEN ? THEN ? ELSE last_changed END WHERE id=?",
                p![c.s("rel_path"), c.s("name"), c.s("ext"), c.s("kind"), c.i("size"), c.i("mtime"), c.os("sha256"), now_ms(), changed, changed, now_ms(), mid.clone()],
            )?;
            event(v, &pid, &mid, "relinked", &format!("{} → {} (confirmed by user)", m.s("rel_path"), c.s("rel_path")), c.os("sha256").as_deref())?;
            db.exec("DELETE FROM reconcile_reviews WHERE missing_file_id=? OR candidate_file_id=? OR candidate_file_id=?", p![mid.clone(), cid.clone(), mid.clone()])?;
        } else {
            db.exec("DELETE FROM reconcile_reviews WHERE id=?", p![review_id])?;
        }
        Ok(())
    })?;
    if accept {
        search::index_file(v, &mid)?;
    }
    Ok(Json::obj().with("ok", true))
}

/// Permanently forgets a missing file's record (and its documentation).
pub fn forget_missing(v: &Vault, file_id: &str) -> DbResult<()> {
    let r = v.db.one("SELECT state FROM files WHERE id=?", p![file_id])?.ok_or("File not found")?;
    if r.s("state") != "missing" {
        return Err("Only missing files can be forgotten".into());
    }
    v.db.tx(|db| {
        db.exec("DELETE FROM links WHERE (a_type='file' AND a_id=?) OR (b_type='file' AND b_id=?)", p![file_id, file_id])?;
        db.exec("DELETE FROM entity_tags WHERE entity_type='file' AND entity_id=?", p![file_id])?;
        search::unindex(v, "file", file_id)?;
        db.exec("DELETE FROM files WHERE id=?", p![file_id])?;
        Ok(())
    })
}

// ------------------------------------------------------------------ listing

fn file_json(r: &Row) -> Json {
    Json::obj()
        .with("id", r.s("id"))
        .with("path", r.s("rel_path"))
        .with("name", r.s("name"))
        .with("ext", r.s("ext"))
        .with("kind", r.s("kind"))
        .with("size", r.i("size"))
        .with("mtime", r.i("mtime"))
        .with("sha256", r.os("sha256"))
        .with("state", r.s("state"))
        .with("firstSeen", r.i("first_seen"))
        .with("lastChanged", r.i("last_changed"))
        .with("changedSinceReview", r.b("changed_since_review"))
        .with("hasDocs", r.i("has_docs") > 0)
        .with("purpose", r.s("purpose"))
        .with("versions", r.i("version_count"))
}

const FILE_SELECT: &str = "SELECT f.*, COALESCE(d.purpose,'') AS purpose,
    (d.file_id IS NOT NULL AND (d.purpose!='' OR d.notes_md!='' OR d.warnings!='')) AS has_docs,
    (SELECT count(*) FROM file_versions fv WHERE fv.file_id=f.id) AS version_count
    FROM files f LEFT JOIN file_docs d ON d.file_id=f.id";

pub fn list(v: &Vault, project_id: &str, filter: &Json) -> DbResult<Json> {
    let q = filter.str_of("query").unwrap_or_default().to_lowercase();
    let kind = filter.str_of("kind");
    let state = filter.str_of("state");
    let dir = filter.str_of("dir");
    let only_changed = filter.get("changedOnly").and_then(|x| x.as_bool()).unwrap_or(false);
    let undocumented = filter.get("undocumented").and_then(|x| x.as_bool()).unwrap_or(false);
    let limit = filter.get("limit").and_then(|x| x.as_i64()).unwrap_or(5000).clamp(1, 50000);
    let ext_list: Option<Vec<String>> = filter.get("exts").and_then(|x| x.as_arr()).map(|a| a.iter().filter_map(|e| e.as_str()).map(|e| e.to_lowercase()).collect());
    let has_exts = ext_list.is_some();
    let exts = ext_list.unwrap_or_default().join(",");
    let like = format!("%{}%", q.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_"));
    let dir_like = dir.as_ref().map(|d| format!("{}/%", d.trim_end_matches('/').replace('%', "\\%").replace('_', "\\_")));
    let sort = match filter.str_of("sort").as_deref() {
        Some("size") => "f.size DESC",
        Some("changed") => "f.last_changed DESC",
        Some("name") => "f.name COLLATE NOCASE",
        _ => "f.rel_path COLLATE NOCASE",
    };
    let rows = v.db.query(
        &format!(
            "{FILE_SELECT} WHERE f.project_id=? AND (?='' OR lower(f.rel_path) LIKE ? ESCAPE '\\')
             AND (? IS NULL OR f.kind=?) AND (CASE WHEN ? IS NULL THEN f.state!='deleted' ELSE f.state=? END) AND (? IS NULL OR f.rel_path LIKE ? ESCAPE '\\')
             AND (?=0 OR f.changed_since_review=1) AND (?=0 OR d.file_id IS NULL OR (d.purpose='' AND d.notes_md=''))
             AND (?=0 OR instr(',' || ? || ',', ',' || f.ext || ',')>0)
             ORDER BY {sort} LIMIT ?"
        ),
        p![project_id, q.clone(), like, kind.clone(), kind, state.clone(), state, dir_like.clone(), dir_like, only_changed, undocumented, has_exts, exts, limit],
    )?;
    Ok(Json::Arr(rows.iter().map(file_json).collect()))
}

pub fn get(v: &Vault, file_id: &str) -> DbResult<Json> {
    let r = v.db.one(&format!("{FILE_SELECT} WHERE f.id=?"), p![file_id])?.ok_or("File not found")?;
    let mut j = file_json(&r);
    let root = projects::root_of(v, &r.s("project_id"))?;
    j.set("projectId", r.s("project_id"));
    j.set("absPath", safe_join(&root, &r.s("rel_path"))?.to_string_lossy().to_string());
    j.set("tags", projects::tags_for(v, "file", file_id)?);
    j.set("docs", docs_get(v, file_id)?);
    j.set("events", events(v, file_id, 100)?);
    j.set("links", crate::knowledge::links_for(v, "file", file_id)?);
    j.set("duplicates", Json::Arr(match r.os("sha256") {
        Some(h) => v
            .db
            .query("SELECT id, rel_path FROM files WHERE project_id=? AND sha256=? AND id!=? AND state='present'", p![r.s("project_id"), h, file_id])?
            .iter()
            .map(|d| Json::obj().with("id", d.s("id")).with("path", d.s("rel_path")))
            .collect(),
        None => vec![],
    }));
    if r.s("ext") == "stl" && r.s("state") == "present" {
        if let Ok(ap) = safe_join(&root, &r.s("rel_path")) {
            if let Some((x, y, z)) = stl_dimensions(&ap) {
                j.set("meshDimensionsMm", Json::Arr(vec![x.into(), y.into(), z.into()]));
            }
        }
    }
    Ok(j)
}

/// Folder tree of present+missing files: [{path, files, bytes}] per directory.
pub fn dirs(v: &Vault, project_id: &str) -> DbResult<Json> {
    let rows = v.db.query("SELECT rel_path, size FROM files WHERE project_id=? AND state!='deleted'", p![project_id])?;
    let mut agg: BTreeMap<String, (i64, i64)> = BTreeMap::new();
    for r in &rows {
        let rp = r.s("rel_path");
        let mut cur = String::new();
        let parts: Vec<&str> = rp.split('/').collect();
        agg.entry(String::new()).or_default();
        for seg in &parts[..parts.len().saturating_sub(1)] {
            cur = if cur.is_empty() { seg.to_string() } else { format!("{cur}/{seg}") };
            let e = agg.entry(cur.clone()).or_default();
            e.0 += 1;
            e.1 += r.i("size");
        }
        let e = agg.entry(String::new()).or_default();
        e.0 += 1;
        e.1 += r.i("size");
    }
    Ok(Json::Arr(agg.into_iter().map(|(k, (n, b))| Json::obj().with("path", k).with("files", n).with("bytes", b)).collect()))
}

/// The project's file-type groups (from its category) with file counts, plus
/// an "Other files" group for every extension no group covers, so nothing in
/// the project is ever hidden by the type chips. `found` lists the extensions
/// actually present in each group, most common first.
pub fn groups(v: &Vault, project_id: &str) -> DbResult<Json> {
    let pr = v.db.one("SELECT category FROM projects WHERE id=?", p![project_id])?.ok_or("Project not found")?;
    let mut counts: Vec<(String, i64)> = v
        .db
        .query("SELECT ext, count(*) AS n FROM files WHERE project_id=? AND state='present' GROUP BY ext ORDER BY n DESC, ext", p![project_id])?
        .iter()
        .map(|r| (r.s("ext"), r.i("n")))
        .collect();
    let found = |exts: &[String], counts: &[(String, i64)]| -> (i64, Json) {
        let hits: Vec<&(String, i64)> = counts.iter().filter(|(e, _)| exts.contains(e)).collect();
        (hits.iter().map(|(_, n)| n).sum(), Json::Arr(hits.iter().map(|(e, n)| Json::obj().with("ext", e.as_str()).with("count", *n)).collect()))
    };
    let mut out = Vec::new();
    let mut covered: Vec<String> = Vec::new();
    let defs = crate::profiles::groups(&pr.s("category"));
    for g in &defs {
        let exts: Vec<String> = g.exts.iter().map(|e| e.to_string()).collect();
        let (n, f) = found(&exts, &counts);
        covered.extend(exts.iter().cloned());
        out.push(
            Json::obj()
                .with("id", g.id)
                .with("label", g.label)
                .with("hint", g.hint)
                .with("count", n)
                .with("found", f)
                .with("exts", Json::Arr(exts.into_iter().map(Json::from).collect())),
        );
    }
    if !defs.is_empty() {
        counts.retain(|(e, _)| !covered.contains(e));
        let other: Vec<String> = counts.iter().map(|(e, _)| e.clone()).collect();
        let (n, f) = found(&other, &counts);
        if n > 0 {
            out.push(
                Json::obj()
                    .with("id", "other")
                    .with("label", "Other files")
                    .with("hint", "File types this project type doesn't group")
                    .with("count", n)
                    .with("found", f)
                    .with("exts", Json::Arr(other.into_iter().map(Json::from).collect())),
            );
        }
    }
    Ok(Json::Arr(out))
}

pub fn events(v: &Vault, file_id: &str, limit: i64) -> DbResult<Json> {
    let rows = v.db.query("SELECT * FROM file_events WHERE file_id=? ORDER BY at DESC LIMIT ?", p![file_id, limit])?;
    Ok(Json::Arr(
        rows.iter()
            .map(|r| Json::obj().with("kind", r.s("kind")).with("detail", r.s("detail")).with("sha256", r.os("sha256")).with("at", r.i("at")))
            .collect(),
    ))
}

pub fn recent_activity(v: &Vault, project_id: &str, limit: i64) -> DbResult<Json> {
    let rows = v.db.query(
        "SELECT e.kind, e.detail, e.at, f.id, f.rel_path FROM file_events e JOIN files f ON f.id=e.file_id
         WHERE e.project_id=? ORDER BY e.at DESC LIMIT ?",
        p![project_id, limit],
    )?;
    Ok(Json::Arr(
        rows.iter()
            .map(|r| Json::obj().with("kind", r.s("kind")).with("detail", r.s("detail")).with("at", r.i("at")).with("fileId", r.s("id")).with("path", r.s("rel_path")))
            .collect(),
    ))
}

pub fn duplicates(v: &Vault, project_id: &str) -> DbResult<Json> {
    let rows = v.db.query(
        "SELECT sha256, size, group_concat(id, '|') AS ids, group_concat(rel_path, '|') AS paths, count(*) AS n
         FROM files WHERE project_id=? AND state='present' AND sha256 IS NOT NULL AND size>0
         GROUP BY sha256 HAVING n>1 ORDER BY size*n DESC LIMIT 500",
        p![project_id],
    )?;
    Ok(Json::Arr(
        rows.iter()
            .map(|r| {
                let ids: Vec<Json> = r.s("ids").split('|').map(Json::from).collect();
                let paths: Vec<Json> = r.s("paths").split('|').map(Json::from).collect();
                Json::obj().with("sha256", r.s("sha256")).with("size", r.i("size")).with("ids", Json::Arr(ids)).with("paths", Json::Arr(paths))
            })
            .collect(),
    ))
}

pub fn mark_reviewed(v: &Vault, file_ids: &[String]) -> DbResult<()> {
    v.db.tx(|db| {
        for id in file_ids {
            db.exec("UPDATE files SET changed_since_review=0 WHERE id=?", p![id.clone()])?;
        }
        Ok(())
    })
}

/// Text preview (first `max` bytes) for supported text formats.
pub fn preview_text(v: &Vault, file_id: &str, max: usize) -> DbResult<Json> {
    let r = v.db.one("SELECT project_id, rel_path, ext, size, state FROM files WHERE id=?", p![file_id])?.ok_or("File not found")?;
    if r.s("state") != "present" {
        return Err("File is missing on disk".into());
    }
    let root = projects::root_of(v, &r.s("project_id"))?;
    let ap = safe_join(&root, &r.s("rel_path"))?;
    use std::io::Read;
    let mut f = std::fs::File::open(&ap).map_err(|e| format!("Cannot read {}: {e}", ap.display()))?;
    let mut buf = vec![0u8; max.min(2 * 1024 * 1024)];
    let n = f.read(&mut buf).map_err(|e| e.to_string())?;
    buf.truncate(n);
    let binary = buf.iter().take(8000).any(|b| *b == 0);
    if binary && !is_text_ext(&r.s("ext")) {
        return Ok(Json::obj().with("binary", true));
    }
    Ok(Json::obj()
        .with("binary", false)
        .with("text", String::from_utf8_lossy(&buf).to_string())
        .with("truncated", (n as i64) < r.i("size")))
}

/// Reads an image as base64 (for previews) — bounded size.
pub fn read_base64(v: &Vault, file_id: &str, max_bytes: u64) -> DbResult<Json> {
    let r = v.db.one("SELECT project_id, rel_path, size, ext FROM files WHERE id=?", p![file_id])?.ok_or("File not found")?;
    if r.i("size") as u64 > max_bytes {
        return Err("File too large to preview".into());
    }
    let root = projects::root_of(v, &r.s("project_id"))?;
    let data = std::fs::read(safe_join(&root, &r.s("rel_path"))?).map_err(|e| e.to_string())?;
    let mime = match r.s("ext").as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        _ => "application/octet-stream",
    };
    Ok(Json::obj().with("mime", mime).with("base64", base64(&data)))
}

fn model_path(v: &Vault, file_id: &str) -> DbResult<(std::path::PathBuf, String)> {
    let r = v.db.one("SELECT project_id, rel_path, ext, state FROM files WHERE id=?", p![file_id])?.ok_or("File not found")?;
    if r.s("state") != "present" {
        return Err("The file is not on disk".into());
    }
    let root = projects::root_of(v, &r.s("project_id"))?;
    Ok((safe_join(&root, &r.s("rel_path"))?, r.s("ext")))
}

/// Triangles for the 3D preview (STL, OBJ, 3MF).
pub fn mesh(v: &Vault, file_id: &str) -> DbResult<Json> {
    let (path, ext) = model_path(v, file_id)?;
    crate::models::preview_mesh(&path, &ext)
}

/// Print information stored in a .3mf (printer, filaments, time, thumbnail…).
pub fn threemf_info(v: &Vault, file_id: &str) -> DbResult<Json> {
    let (path, ext) = model_path(v, file_id)?;
    if ext != "3mf" {
        return Err("Not a .3mf file".into());
    }
    crate::models::threemf_info(&path)
}

pub fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        s.push(T[(n >> 18) as usize & 63] as char);
        s.push(T[(n >> 12) as usize & 63] as char);
        s.push(if c.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        s.push(if c.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    s
}

/// Bounding box of an STL (binary or ASCII), in model units (normally mm).
/// Ported from Print Vault's extractor.
pub fn stl_dimensions(p: &Path) -> Option<(f64, f64, f64)> {
    let md = std::fs::metadata(p).ok()?;
    if md.len() > 512 * 1024 * 1024 {
        return None;
    }
    let data = std::fs::read(p).ok()?;
    let mut mn = [f64::MAX; 3];
    let mut mx = [f64::MIN; 3];
    let mut push = |v: [f64; 3]| {
        for i in 0..3 {
            mn[i] = mn[i].min(v[i]);
            mx[i] = mx[i].max(v[i]);
        }
    };
    let binary_ok = data.len() >= 84 && {
        let n = u32::from_le_bytes([data[80], data[81], data[82], data[83]]) as usize;
        84 + n * 50 == data.len()
    };
    let mut any = false;
    if binary_ok {
        let n = (data.len() - 84) / 50;
        for t in 0..n {
            let base = 84 + t * 50 + 12;
            for k in 0..3 {
                let o = base + k * 12;
                let f = |i: usize| f32::from_le_bytes([data[o + i], data[o + i + 1], data[o + i + 2], data[o + i + 3]]) as f64;
                push([f(0), f(4), f(8)]);
                any = true;
            }
        }
    } else {
        let text = String::from_utf8_lossy(&data);
        for line in text.lines() {
            let mut it = line.split_whitespace();
            if it.next().map(|w| w.eq_ignore_ascii_case("vertex")) == Some(true) {
                let v: Vec<f64> = it.filter_map(|x| x.parse().ok()).collect();
                if v.len() == 3 {
                    push([v[0], v[1], v[2]]);
                    any = true;
                }
            }
        }
    }
    if !any {
        return None;
    }
    let r = |x: f64| (x * 1000.0).round() / 1000.0;
    Some((r(mx[0] - mn[0]), r(mx[1] - mn[1]), r(mx[2] - mn[2])))
}

// ------------------------------------------------------------------ import

/// Copies external files into the project (managed storage or a folder of a
/// linked project). Never overwrites: name collisions get " (2)" suffixes.
/// Files whose content already exists in the project are skipped unless
/// `allow_duplicates`. Each copy is verified by hash before it is kept.
pub fn import(v: &Vault, project_id: &str, sources: &[PathBuf], dest_rel_dir: &str, allow_duplicates: bool) -> DbResult<Json> {
    import_op(v, project_id, sources, dest_rel_dir, allow_duplicates, false)
}

/// Copies (or, with `move_files`, moves) files into the project. A move is a
/// rename when source and project are on the same drive; otherwise the copy
/// is verified first and only then is the original sent to the Recycle Bin.
pub fn import_op(v: &Vault, project_id: &str, sources: &[PathBuf], dest_rel_dir: &str, allow_duplicates: bool, move_files: bool) -> DbResult<Json> {
    let root = projects::root_of(v, project_id)?;
    if !root.is_dir() {
        return Err(format!("Project folder {} is not reachable", root.display()));
    }
    let dest_dir = safe_join(&root, dest_rel_dir)?;
    std::fs::create_dir_all(&dest_dir).map_err(|e| format!("Cannot create {}: {e}", dest_dir.display()))?;
    let mut imported = Vec::new();
    let mut skipped = Vec::new();
    let mut errors = Vec::new();
    // expand folders
    let mut queue: Vec<(PathBuf, PathBuf)> = Vec::new();
    for s in sources {
        if s.is_dir() {
            let base = s.parent().unwrap_or(s).to_path_buf();
            let w = walk(s, &Ignore::parse(""));
            for (rel, _) in w.files {
                let src = s.join(&rel);
                let sub = rel_path(&base, &src).unwrap_or(rel.clone());
                queue.push((src, PathBuf::from(sub)));
            }
            errors.extend(w.errors);
        } else {
            queue.push((s.clone(), PathBuf::from(s.file_name().map(|x| x.to_os_string()).unwrap_or_default())));
        }
    }
    for (src, sub) in queue {
        if src.starts_with(&root) {
            skipped.push(Json::obj().with("source", src.to_string_lossy().to_string()).with("reason", "already inside the project folder"));
            continue;
        }
        let h = match sha256_file(&src) {
            Ok(h) => h,
            Err(e) => {
                errors.push(format!("{}: {e}", src.display()));
                continue;
            }
        };
        if !allow_duplicates {
            if let Some(d) = v.db.one("SELECT rel_path FROM files WHERE project_id=? AND sha256=? AND state='present'", p![project_id, h.clone()])? {
                skipped.push(Json::obj().with("source", src.to_string_lossy().to_string()).with("reason", format!("duplicate of {}", d.s("rel_path"))));
                continue;
            }
        }
        let sub_s = sub.to_string_lossy().replace('\\', "/");
        let target_dir = match sub_s.rsplit_once('/') {
            Some((d, _)) => match safe_join(&dest_dir, d) {
                Ok(p) => p,
                Err(e) => {
                    errors.push(e);
                    continue;
                }
            },
            None => dest_dir.clone(),
        };
        if let Err(e) = std::fs::create_dir_all(&target_dir) {
            errors.push(format!("{}: {e}", target_dir.display()));
            continue;
        }
        let name = src.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "file".into());
        let dest = unique_dest(&target_dir, &name);
        if move_files && std::fs::rename(&src, &dest).is_ok() {
            imported.push(
                Json::obj()
                    .with("source", src.to_string_lossy().to_string())
                    .with("path", rel_path(&root, &dest).unwrap_or_default())
                    .with("sha256", h)
                    .with("moved", true),
            );
            continue;
        }
        let tmp = target_dir.join(format!(".pv-tmp-{}", &new_id()[..8]));
        let res = std::fs::copy(&src, &tmp).map_err(|e| e.to_string()).and_then(|_| {
            let h2 = sha256_file(&tmp).map_err(|e| e.to_string())?;
            if h2 != h {
                return Err("verification failed (source changed during copy?)".into());
            }
            if dest.exists() {
                return Err("destination appeared during copy".into());
            }
            std::fs::rename(&tmp, &dest).map_err(|e| e.to_string())
        });
        if let Err(e) = res {
            let _ = std::fs::remove_file(&tmp);
            errors.push(format!("{}: {e}", src.display()));
            continue;
        }
        let mut item = Json::obj().with("source", src.to_string_lossy().to_string()).with("path", rel_path(&root, &dest).unwrap_or_default()).with("sha256", h);
        if move_files {
            // the verified copy is in place; now retire the original
            match to_recycle_bin(&src) {
                Ok(()) => item.set("moved", true),
                Err(e) => errors.push(format!("{}: copied, but the original could not be removed ({e})", src.display())),
            }
        }
        imported.push(item);
    }
    let n = imported.len();
    let scan_rep = scan(v, project_id)?;
    let mut batch_id = Json::Null;
    if n > 0 {
        let id = new_id();
        v.db.exec(
            "INSERT INTO import_batches(id,project_id,dest_dir,files_json,created_at) VALUES (?,?,?,?,?)",
            p![id.clone(), project_id, dest_rel_dir, Json::Arr(imported.clone()).to_string(), now_ms()],
        )?;
        let verb = if move_files { "Moved" } else { "Imported" };
        timeline::add(v, project_id, "import", &format!("{verb} {n} file{} in", if n == 1 { "" } else { "s" }), &format!("into /{}", dest_rel_dir.trim_matches('/')), Some(("import", &id)), false)?;
        batch_id = Json::from(id);
    }
    Ok(Json::obj()
        .with("batchId", batch_id)
        .with("imported", Json::Arr(imported))
        .with("skipped", Json::Arr(skipped))
        .with("errors", Json::Arr(errors.into_iter().map(Json::from).collect()))
        .with("scan", scan_rep.json()))
}

/// Dry run of an import: what would be copied where, with conflicts.
/// Nothing on disk changes.
pub fn import_preview(v: &Vault, project_id: &str, sources: &[PathBuf], dest_rel_dir: &str) -> DbResult<Json> {
    let pr = v.db.one("SELECT name, root_path, mode FROM projects WHERE id=?", p![project_id])?.ok_or("Project not found")?;
    let root = PathBuf::from(pr.s("root_path"));
    if !root.is_dir() {
        return Err(format!("Project folder {} is not reachable", root.display()));
    }
    let dest_dir = safe_join(&root, dest_rel_dir)?;
    let mut items = Vec::new();
    let mut planned: HashSet<PathBuf> = HashSet::new();
    let mut add = |src: PathBuf, sub: String, items: &mut Vec<Json>| -> DbResult<()> {
        let mut status = "copy";
        let mut detail = String::new();
        if src.starts_with(&root) {
            status = "skip";
            detail = "already inside this project's folder".into();
        }
        let h = if status == "copy" { sha256_file(&src).ok() } else { None };
        if status == "copy" && h.is_none() {
            status = "error";
            detail = "can't be read".into();
        }
        if let (Some(h), "copy") = (&h, status) {
            if let Some(d) = v.db.one("SELECT rel_path FROM files WHERE project_id=? AND sha256=? AND state='present'", p![project_id, h.clone()])? {
                status = "skip";
                detail = format!("identical to {}", d.s("rel_path"));
            }
        }
        let mut dest_rel = String::new();
        if status == "copy" {
            let target_dir = match sub.rsplit_once('/') { Some((d, _)) => safe_join(&dest_dir, d)?, None => dest_dir.clone() };
            let name = src.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            let mut dest = target_dir.join(&name);
            if dest.exists() || planned.contains(&dest) {
                let mut n = 2;
                let (stem, ext) = match name.rfind('.') { Some(i) if i > 0 => (name[..i].to_string(), name[i..].to_string()), _ => (name.clone(), String::new()) };
                while dest.exists() || planned.contains(&dest) {
                    dest = target_dir.join(format!("{stem} ({n}){ext}"));
                    n += 1;
                }
                status = "rename";
                detail = format!("“{name}” already exists there; the copy will be named “{}”", dest.file_name().unwrap().to_string_lossy());
            }
            planned.insert(dest.clone());
            dest_rel = rel_path(&root, &dest).unwrap_or_default();
        }
        items.push(Json::obj().with("source", src.to_string_lossy().to_string()).with("dest", dest_rel).with("status", status).with("detail", detail));
        Ok(())
    };
    for s in sources {
        if s.is_dir() {
            let base = s.parent().unwrap_or(s).to_path_buf();
            for (rel, _) in walk(s, &Ignore::parse("")).files {
                let src = s.join(&rel);
                let sub = rel_path(&base, &src).unwrap_or(rel.clone());
                add(src, sub, &mut items)?;
            }
        } else if s.is_file() {
            let name = s.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            add(s.clone(), name, &mut items)?;
        } else {
            items.push(Json::obj().with("source", s.to_string_lossy().to_string()).with("dest", "").with("status", "error").with("detail", "not found"));
        }
    }
    Ok(Json::obj()
        .with("projectName", pr.s("name"))
        .with("mode", pr.s("mode"))
        .with("destDir", dest_rel_dir.trim_matches('/'))
        .with("destAbs", projects::display_path(&dest_dir))
        .with("root", projects::display_path(&root))
        .with("fileCount", v.db.scalar_i64("SELECT count(*) FROM files WHERE project_id=? AND state='present'", p![project_id])?)
        .with("operation", "copy")
        .with("items", Json::Arr(items)))
}

pub fn import_batches(v: &Vault, project_id: &str) -> DbResult<Json> {
    Ok(Json::Arr(
        v.db.query("SELECT * FROM import_batches WHERE project_id=? ORDER BY created_at DESC LIMIT 50", p![project_id])?
            .iter()
            .map(|r| {
                Json::obj()
                    .with("id", r.s("id"))
                    .with("destDir", r.s("dest_dir"))
                    .with("files", Json::parse(&r.s("files_json")).unwrap_or(Json::Arr(vec![])))
                    .with("at", r.i("created_at"))
                    .with("undoneAt", r.oi("undone_at"))
            })
            .collect(),
    ))
}

/// Undo an import: removes only the copies that import created, and only if
/// they are byte-for-byte unchanged since (hash check). Anything edited since,
/// or not created by the import, is left alone and reported.
pub fn undo_import(v: &Vault, batch_id: &str) -> DbResult<Json> {
    let b = v.db.one("SELECT * FROM import_batches WHERE id=?", p![batch_id])?.ok_or("Import not found")?;
    if b.oi("undone_at").is_some() {
        return Err("This import was already undone".into());
    }
    let pid = b.s("project_id");
    let root = projects::root_of(v, &pid)?;
    let list = Json::parse(&b.s("files_json")).unwrap_or(Json::Arr(vec![]));
    let mut removed = Vec::new();
    let mut kept = Vec::new();
    for it in list.as_arr().cloned().unwrap_or_default() {
        let rel = it.str_of("path").unwrap_or_default();
        let sha = it.str_of("sha256").unwrap_or_default();
        let path = safe_join(&root, &rel)?;
        if !path.exists() {
            kept.push(Json::obj().with("path", rel).with("reason", "already gone"));
            continue;
        }
        if sha256_file(&path).ok().as_deref() != Some(sha.as_str()) {
            kept.push(Json::obj().with("path", rel).with("reason", "changed since the import — kept"));
            continue;
        }
        if it.get("moved").and_then(|x| x.as_bool()).unwrap_or(false) {
            // put it back where it was; never overwrite something that's there now
            let src = PathBuf::from(it.str_of("source").unwrap_or_default());
            if src.exists() {
                kept.push(Json::obj().with("path", rel).with("reason", format!("{} exists again — kept both", src.display())));
                continue;
            }
            let back = (|| -> Result<(), String> {
                if let Some(parent) = src.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                }
                if std::fs::rename(&path, &src).is_ok() {
                    return Ok(());
                }
                std::fs::copy(&path, &src).map_err(|e| e.to_string())?;
                if sha256_file(&src).ok().as_deref() != Some(sha.as_str()) {
                    let _ = std::fs::remove_file(&src);
                    return Err("verification failed".into());
                }
                std::fs::remove_file(&path).map_err(|e| e.to_string())
            })();
            match back {
                Ok(()) => {
                    if let Some(r) = v.db.one("SELECT id FROM files WHERE project_id=? AND rel_path=?", p![pid.clone(), rel.clone()])? {
                        search::unindex(v, "file", &r.s("id"))?;
                        v.db.exec("DELETE FROM files WHERE id=?", p![r.s("id")])?;
                    }
                    removed.push(Json::from(rel));
                }
                Err(e) => kept.push(Json::obj().with("path", rel).with("reason", format!("could not move back: {e}"))),
            }
            continue;
        }
        match to_recycle_bin(&path) {
            Ok(()) => {
                if let Some(r) = v.db.one("SELECT id FROM files WHERE project_id=? AND rel_path=?", p![pid.clone(), rel.clone()])? {
                    search::unindex(v, "file", &r.s("id"))?;
                    v.db.exec("DELETE FROM files WHERE id=?", p![r.s("id")])?;
                }
                removed.push(Json::from(rel));
            }
            Err(e) => kept.push(Json::obj().with("path", rel).with("reason", format!("could not remove: {e}"))),
        }
    }
    v.db.exec("UPDATE import_batches SET undone_at=? WHERE id=?", p![now_ms(), batch_id])?;
    timeline::add(v, &pid, "import", &format!("Undid import ({} removed, {} kept)", removed.len(), kept.len()), "", None, false)?;
    scan(v, &pid)?;
    Ok(Json::obj().with("removed", Json::Arr(removed)).with("kept", Json::Arr(kept)))
}

/// Moves files to another project: copies (verified) into the target folder,
/// carries over notes, tags and stored versions, then sends the original to
/// the Recycle Bin. Links to other items stay behind (links are per project).
pub fn move_to_project(v: &Vault, file_ids: &[String], target_pid: &str, dest_rel_dir: &str) -> DbResult<Json> {
    let troot = projects::root_of(v, target_pid)?;
    if !troot.is_dir() {
        return Err("The target project's folder is not reachable".into());
    }
    let tdir = safe_join(&troot, dest_rel_dir)?;
    std::fs::create_dir_all(&tdir).map_err(|e| e.to_string())?;
    let mut moved = Vec::new();
    let mut errors = Vec::new();
    let mut sources = HashSet::new();
    for fid in file_ids {
        let res = (|| -> DbResult<String> {
            let f = v.db.one("SELECT * FROM files WHERE id=?", p![fid.clone()])?.ok_or("File not found")?;
            if f.s("project_id") == target_pid {
                return Err(format!("{} is already in that project", f.s("rel_path")));
            }
            if f.s("state") != "present" {
                return Err(format!("{} is not on disk", f.s("rel_path")));
            }
            let sroot = projects::root_of(v, &f.s("project_id"))?;
            let src = safe_join(&sroot, &f.s("rel_path"))?;
            let sha = sha256_file(&src).map_err(|e| e.to_string())?;
            let dest = unique_dest(&tdir, &f.s("name"));
            let tmp = tdir.join(format!(".pv-tmp-{}", &new_id()[..8]));
            std::fs::copy(&src, &tmp).map_err(|e| e.to_string())?;
            if sha256_file(&tmp).ok().as_deref() != Some(sha.as_str()) {
                let _ = std::fs::remove_file(&tmp);
                return Err("copy verification failed".into());
            }
            std::fs::rename(&tmp, &dest).map_err(|e| e.to_string())?;
            let new_rel = rel_path(&troot, &dest).unwrap_or_default();
            // move the identity: same file id, new project + path (keeps notes, tags, versions)
            to_recycle_bin(&src).map_err(|e| {
                let _ = std::fs::remove_file(&dest);
                format!("the original couldn't be moved to the Recycle Bin ({e}); the move was cancelled")
            })?;
            v.db.tx(|db| {
                db.exec("DELETE FROM links WHERE (a_type='file' AND a_id=?) OR (b_type='file' AND b_id=?)", p![fid.clone(), fid.clone()])?;
                db.exec("DELETE FROM reconcile_reviews WHERE missing_file_id=? OR candidate_file_id=?", p![fid.clone(), fid.clone()])?;
                let ext = extension_of(&new_rel);
                db.exec(
                    "UPDATE files SET project_id=?, rel_path=?, name=?, ext=?, kind=?, sha256=?, state='present' WHERE id=?",
                    p![target_pid, new_rel.clone(), file_name_of(&new_rel), ext.clone(), classify(&ext), sha.clone(), fid.clone()],
                )?;
                db.exec("UPDATE file_versions SET project_id=? WHERE file_id=?", p![target_pid, fid.clone()])?;
                db.exec("UPDATE file_events SET project_id=? WHERE file_id=?", p![target_pid, fid.clone()])?;
                Ok(())
            })?;
            event(v, target_pid, fid, "moved", &format!("Moved here from another project ({})", f.s("rel_path")), Some(&sha))?;
            timeline::add(v, &f.s("project_id"), "files", &format!("Moved {} to another project", f.s("rel_path")), "", None, false)?;
            sources.insert(f.s("project_id"));
            search::index_file(v, fid)?;
            for r in v.db.query("SELECT id FROM file_versions WHERE file_id=?", p![fid.clone()])? {
                crate::versions::index_backup(v, &r.s("id"))?;
            }
            Ok(new_rel)
        })();
        match res {
            Ok(p) => moved.push(Json::from(p)),
            Err(e) => errors.push(Json::from(e)),
        }
    }
    if !moved.is_empty() {
        timeline::add(v, target_pid, "files", &format!("Received {} file(s) from another project", moved.len()), "", None, false)?;
    }
    for s in &sources {
        scan(v, s)?;
    }
    scan(v, target_pid)?;
    Ok(Json::obj().with("moved", Json::Arr(moved)).with("errors", Json::Arr(errors)))
}

// ------------------------------------------------------------------ rename / delete

/// Validates a new file name for Windows and other platforms.
pub fn validate_file_name(name: &str) -> Result<String, String> {
    let n = name.trim().trim_end_matches(['.', ' ']).to_string();
    if n.is_empty() || n == "." || n == ".." {
        return Err("Enter a file name".into());
    }
    if let Some(c) = n.chars().find(|c| matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || (*c as u32) < 32) {
        return Err(format!("File names can't contain {}", if c.is_control() { "control characters".to_string() } else { format!("“{c}”") }));
    }
    let stem = n.split('.').next().unwrap_or("").to_ascii_uppercase();
    const RESERVED: &[&str] = &["CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9"];
    if RESERVED.contains(&stem.as_str()) {
        return Err(format!("“{n}” is a reserved name on Windows"));
    }
    if n.len() > 240 {
        return Err("That name is too long".into());
    }
    Ok(n)
}

/// Renames a file on disk, keeping its identity (notes, links, history).
pub fn rename(v: &Vault, file_id: &str, new_name: &str) -> DbResult<Json> {
    let f = v.db.one("SELECT * FROM files WHERE id=?", p![file_id])?.ok_or("File not found")?;
    if f.s("state") != "present" {
        return Err("Only files that exist on disk can be renamed".into());
    }
    let name = validate_file_name(new_name)?;
    if name == f.s("name") {
        return get(v, file_id);
    }
    let pid = f.s("project_id");
    let root = projects::root_of(v, &pid)?;
    let old_rel = f.s("rel_path");
    let new_rel = match old_rel.rsplit_once('/') {
        Some((dir, _)) => format!("{dir}/{name}"),
        None => name.clone(),
    };
    let from = safe_join(&root, &old_rel)?;
    let to = safe_join(&root, &new_rel)?;
    let case_only = old_rel.to_lowercase() == new_rel.to_lowercase();
    if !case_only && to.exists() {
        return Err(format!("“{name}” already exists in that folder"));
    }
    // a remembered missing/deleted record may still hold that path
    if let Some(o) = v.db.one("SELECT id, state FROM files WHERE project_id=? AND rel_path=? AND id!=?", p![pid.clone(), new_rel.clone(), file_id])? {
        let has_docs = v.db.scalar_i64("SELECT count(*) FROM file_docs WHERE file_id=? AND (purpose!='' OR notes_md!='')", p![o.s("id")])? > 0;
        if o.s("state") == "present" || has_docs {
            return Err(format!("Project Vault still has notes for an earlier “{name}” in this folder. Forget that record first (it's listed under Filter → Missing/Deleted)."));
        }
        v.db.exec("DELETE FROM files WHERE id=?", p![o.s("id")])?;
        search::unindex(v, "file", &o.s("id"))?;
    }
    std::fs::rename(&from, &to).map_err(|e| format!("Could not rename (is the file open in another program?): {e}"))?;
    let ext = extension_of(&name);
    v.db.exec(
        "UPDATE files SET rel_path=?, name=?, ext=?, kind=? WHERE id=?",
        p![new_rel.clone(), name.clone(), ext.clone(), classify(&ext), file_id],
    )?;
    event(v, &pid, file_id, "renamed", &format!("{old_rel} → {new_rel} (in Project Vault)"), f.os("sha256").as_deref())?;
    timeline::add(v, &pid, "files", &format!("Renamed {old_rel} → {name}"), "", Some(("file", file_id)), false)?;
    search::index_file(v, file_id)?;
    get(v, file_id)
}

#[cfg(windows)]
pub(crate) fn to_recycle_bin(path: &Path) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    let method = if path.is_dir() { "DeleteDirectory" } else { "DeleteFile" };
    // Path goes through an environment variable, never into the script text.
    let script = format!("Add-Type -AssemblyName Microsoft.VisualBasic; [Microsoft.VisualBasic.FileIO.FileSystem]::{method}($env:PV_TARGET, 'OnlyErrorDialogs', 'SendToRecycleBin')");
    let out = std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", script.as_str()])
        .env("PV_TARGET", path)
        .creation_flags(0x0800_0000)
        .output()
        .map_err(|e| format!("could not start PowerShell: {e}"))?;
    if !out.status.success() || path.exists() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(())
}

#[cfg(not(windows))]
pub(crate) fn to_recycle_bin(path: &Path) -> Result<(), String> {
    // No system trash here (tests and the dev server only).
    if path.is_dir() { std::fs::remove_dir_all(path) } else { std::fs::remove_file(path) }.map_err(|e| e.to_string())
}

/// Deletes files: keeps a verified copy in the version store, then moves the
/// file to the Recycle Bin. The record stays (state `deleted`) so notes and
/// history survive and the file can be restored from Project Vault.
pub fn delete(v: &Vault, file_ids: &[String]) -> DbResult<Json> {
    let mut deleted = Vec::new();
    let mut errors = Vec::new();
    let mut projects_touched = HashSet::new();
    for fid in file_ids {
        let res = (|| -> DbResult<String> {
            let f = v.db.one("SELECT * FROM files WHERE id=?", p![fid.clone()])?.ok_or("File not found")?;
            if f.s("state") != "present" {
                return Err(format!("{} is not on disk", f.s("rel_path")));
            }
            let pid = f.s("project_id");
            let root = projects::root_of(v, &pid)?;
            let path = safe_join(&root, &f.s("rel_path"))?;
            let sha = sha256_file(&path).map_err(|e| format!("{}: {e}", f.s("rel_path")))?;
            crate::versions::ensure_stored(v, &f, &path, &sha, "before-delete")?;
            to_recycle_bin(&path).map_err(|e| format!("{}: could not move to the Recycle Bin ({e}). Nothing was deleted.", f.s("rel_path")))?;
            v.db.exec("UPDATE files SET state='deleted', changed_since_review=0, sha256=? WHERE id=?", p![sha.clone(), fid.clone()])?;
            v.db.exec("DELETE FROM reconcile_reviews WHERE missing_file_id=? OR candidate_file_id=?", p![fid.clone(), fid.clone()])?;
            event(v, &pid, fid, "deleted", "Deleted in Project Vault (moved to the Recycle Bin; a copy is kept in History)", Some(&sha))?;
            search::unindex(v, "file", fid)?;
            projects_touched.insert(pid);
            Ok(f.s("rel_path"))
        })();
        match res {
            Ok(p) => deleted.push(Json::from(p)),
            Err(e) => errors.push(Json::from(e)),
        }
    }
    for pid in &projects_touched {
        let n = deleted.len();
        timeline::add(v, pid, "files", &format!("Deleted {n} file{}", if n == 1 { "" } else { "s" }), "", None, false)?;
    }
    Ok(Json::obj().with("deleted", Json::Arr(deleted)).with("errors", Json::Arr(errors)))
}

/// "Remove from Project Vault": stops tracking a file without touching it on
/// disk. Adds an exact-path ignore rule (so the next scan doesn't re-add it)
/// and removes its record, notes, links and tags. Stored versions are removed
/// too unless `keep_versions`.
pub fn stop_tracking(v: &Vault, file_ids: &[String], keep_versions: bool) -> DbResult<Json> {
    let mut removed = Vec::new();
    for fid in file_ids {
        let Some(f) = v.db.one("SELECT * FROM files WHERE id=?", p![fid.clone()])? else { continue };
        let pid = f.s("project_id");
        let rel = f.s("rel_path");
        let rules = v.db.one("SELECT ignore_globs FROM projects WHERE id=?", p![pid.clone()])?.map(|r| r.s("ignore_globs")).unwrap_or_default();
        let rule = if rel.contains('/') { rel.clone() } else { format!("./{rel}").replace("./", "/") };
        if !rules.lines().any(|l| l.trim().eq_ignore_ascii_case(&rule)) {
            let new_rules = if rules.trim().is_empty() { rule.clone() } else { format!("{}\n{rule}", rules.trim_end()) };
            v.db.exec("UPDATE projects SET ignore_globs=? WHERE id=?", p![new_rules, pid.clone()])?;
        }
        if keep_versions {
            // versions reference the file row; keep it hidden as 'untracked' instead of deleting
            v.db.exec("UPDATE files SET state='deleted' WHERE id=?", p![fid.clone()])?;
        } else {
            v.db.exec("DELETE FROM links WHERE (a_type='file' AND a_id=?) OR (b_type='file' AND b_id=?)", p![fid.clone(), fid.clone()])?;
            v.db.exec("DELETE FROM entity_tags WHERE entity_type='file' AND entity_id=?", p![fid.clone()])?;
            for r in v.db.query("SELECT id FROM file_versions WHERE file_id=?", p![fid.clone()])? {
                search::unindex(v, "backup", &r.s("id"))?;
            }
            v.db.exec("DELETE FROM files WHERE id=?", p![fid.clone()])?;
        }
        search::unindex(v, "file", fid)?;
        timeline::add(v, &pid, "files", &format!("Stopped tracking {rel} (file left on disk)"), "", None, false)?;
        removed.push(Json::from(rel));
    }
    if !keep_versions {
        crate::versions::gc(v)?;
    }
    Ok(Json::obj().with("removed", Json::Arr(removed)))
}

/// Brings back a file deleted through Project Vault from its stored copy.
pub fn restore_deleted(v: &Vault, file_id: &str) -> DbResult<Json> {
    let f = v.db.one("SELECT * FROM files WHERE id=?", p![file_id])?.ok_or("File not found")?;
    if f.s("state") == "present" {
        return Err("The file is already on disk".into());
    }
    let root = projects::root_of(v, &f.s("project_id"))?;
    if safe_join(&root, &f.s("rel_path"))?.exists() {
        return Err("A file already exists at that location; rescan the project".into());
    }
    let ver = v
        .db
        .one("SELECT id FROM file_versions WHERE file_id=? ORDER BY created_at DESC LIMIT 1", p![file_id])?
        .ok_or("No stored copy of this file exists, so it can't be restored here. Check the Recycle Bin.")?;
    crate::versions::restore(v, &ver.s("id"))?;
    get(v, file_id)
}

// ------------------------------------------------------------------ editing

pub const MAX_EDIT_BYTES: u64 = 8 * 1024 * 1024;

/// Loads a file for the built-in editor. Only UTF-8 text is editable, so a
/// save can never mangle a binary or differently-encoded file.
pub fn read_text(v: &Vault, file_id: &str) -> DbResult<Json> {
    let f = v.db.one("SELECT * FROM files WHERE id=?", p![file_id])?.ok_or("File not found")?;
    if f.s("state") != "present" {
        return Err("The file is missing on disk".into());
    }
    let root = projects::root_of(v, &f.s("project_id"))?;
    let path = safe_join(&root, &f.s("rel_path"))?;
    let md = std::fs::metadata(&path).map_err(|e| e.to_string())?;
    if md.len() > MAX_EDIT_BYTES {
        return Err(format!("This file is {} MB — too large for the built-in editor. Open it in its own program.", md.len() / 1048576));
    }
    let bytes = std::fs::read(&path).map_err(|e| format!("Cannot read the file: {e}"))?;
    if bytes.iter().take(8000).any(|b| *b == 0) {
        return Err("This is a binary file; it can't be edited as text.".into());
    }
    let bom = bytes.starts_with(&[0xEF, 0xBB, 0xBF]);
    let body = if bom { &bytes[3..] } else { &bytes[..] };
    let text = String::from_utf8(body.to_vec()).map_err(|_| "This file isn't UTF-8 text, so editing it here could damage it. Open it in its own program.".to_string())?;
    let crlf = text.contains("\r\n");
    let readonly = md.permissions().readonly();
    Ok(Json::obj()
        .with("text", text)
        .with("sha256", crate::util::sha256_bytes(&bytes))
        .with("eol", if crlf { "crlf" } else { "lf" })
        .with("bom", bom)
        .with("readonly", readonly)
        .with("path", f.s("rel_path")))
}

/// Saves editor content. Refuses if the file changed on disk since it was
/// opened (`expected_sha`), unless the caller passes the new on-disk hash.
/// The previous content is stored as a version first; the write is atomic.
pub fn write_text(v: &Vault, file_id: &str, text: &str, expected_sha: &str, eol: &str, bom: bool) -> DbResult<Json> {
    let f = v.db.one("SELECT * FROM files WHERE id=?", p![file_id])?.ok_or("File not found")?;
    let pid = f.s("project_id");
    let root = projects::root_of(v, &pid)?;
    let path = safe_join(&root, &f.s("rel_path"))?;
    let current = sha256_file(&path).map_err(|e| format!("Cannot read the file before saving: {e}"))?;
    if current != expected_sha {
        return Err(format!("CONFLICT:{current}:The file was changed on disk after you opened it."));
    }
    crate::versions::ensure_stored(v, &f, &path, &current, "before-edit")?;
    let normalized = text.replace("\r\n", "\n");
    let body = if eol == "crlf" { normalized.replace('\n', "\r\n") } else { normalized };
    let mut bytes = Vec::with_capacity(body.len() + 3);
    if bom {
        bytes.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
    }
    bytes.extend_from_slice(body.as_bytes());
    let dir = path.parent().ok_or("bad path")?;
    let tmp = dir.join(format!(".pv-tmp-{}", &new_id()[..8]));
    std::fs::write(&tmp, &bytes).map_err(|e| format!("Cannot write: {e}"))?;
    if let Err(e) = std::fs::rename(&tmp, &path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("Could not save (is the file open and locked in another program?): {e}"));
    }
    let new_sha = crate::util::sha256_bytes(&bytes);
    // Record the save on the file row, so the scan below sees it as already
    // known instead of reporting it as a change made outside the app.
    if let Ok(m) = std::fs::metadata(&path) {
        let mtime = m.modified().map(system_time_ms).unwrap_or(0);
        let now = now_ms();
        v.db.exec(
            "UPDATE files SET size=?, mtime=?, sha256=?, last_seen=?, last_changed=? WHERE id=?",
            p![m.len() as i64, mtime, new_sha.clone(), now, now, file_id],
        )?;
        if current != new_sha {
            event(v, &pid, file_id, "modified", "Edited in Project Vault", Some(&new_sha))?;
        }
    }
    if current != new_sha {
        timeline::add_file_edit(v, &pid, file_id, &f.s("rel_path"), &current, &new_sha)?;
    }
    scan(v, &pid)?;
    v.db.exec("UPDATE files SET changed_since_review=0 WHERE id=?", p![file_id])?;
    Ok(Json::obj().with("sha256", new_sha))
}

// ------------------------------------------------------------------ docs

pub fn docs_get(v: &Vault, file_id: &str) -> DbResult<Json> {
    let d = v.db.one("SELECT * FROM file_docs WHERE file_id=?", p![file_id])?;
    let f = v.db.one("SELECT sha256 FROM files WHERE id=?", p![file_id])?.ok_or("File not found")?;
    Ok(match d {
        Some(d) => Json::obj()
            .with("purpose", d.s("purpose"))
            .with("notes", d.s("notes_md"))
            .with("warnings", d.s("warnings"))
            .with("custom", Json::parse(&d.s("custom_json")).unwrap_or(Json::obj()))
            .with("updatedAt", d.i("updated_at"))
            .with("docSha256", d.os("doc_sha256"))
            .with("fileChangedSinceDoc", d.os("doc_sha256").is_some() && d.os("doc_sha256") != f.os("sha256")),
        None => Json::obj().with("purpose", "").with("notes", "").with("warnings", "").with("custom", Json::obj()).with("updatedAt", Json::Null).with("fileChangedSinceDoc", false),
    })
}

/// Saves documentation. A revision of the *previous* content is kept when it
/// differs and the last revision is older than 10 minutes (autosave-friendly).
pub fn docs_save(v: &Vault, file_id: &str, d: &Json) -> DbResult<Json> {
    let f = v.db.one("SELECT project_id, sha256, rel_path FROM files WHERE id=?", p![file_id])?.ok_or("File not found")?;
    let purpose = d.str_of("purpose").unwrap_or_default();
    let notes = d.str_of("notes").unwrap_or_default();
    let warnings = d.str_of("warnings").unwrap_or_default();
    let custom = d.get("custom").cloned().unwrap_or(Json::obj());
    if !matches!(custom, Json::Obj(_)) {
        return Err("custom fields must be an object".into());
    }
    let now = now_ms();
    v.db.tx(|db| {
        if let Some(old) = db.one("SELECT * FROM file_docs WHERE file_id=?", p![file_id])? {
            let changed = old.s("purpose") != purpose || old.s("notes_md") != notes || old.s("warnings") != warnings;
            let last_rev = db.scalar_i64("SELECT COALESCE(MAX(created_at),0) FROM file_doc_revisions WHERE file_id=?", p![file_id])?;
            if changed && now - last_rev > 10 * 60 * 1000 && (old.s("purpose") != "" || old.s("notes_md") != "") {
                db.exec(
                    "INSERT INTO file_doc_revisions(id,file_id,purpose,notes_md,warnings,file_sha256,created_at) VALUES (?,?,?,?,?,?,?)",
                    p![new_id(), file_id, old.s("purpose"), old.s("notes_md"), old.s("warnings"), old.os("doc_sha256"), now],
                )?;
            }
        }
        db.exec(
            "INSERT INTO file_docs(file_id,purpose,notes_md,warnings,custom_json,doc_sha256,updated_at) VALUES (?,?,?,?,?,?,?)
             ON CONFLICT(file_id) DO UPDATE SET purpose=excluded.purpose, notes_md=excluded.notes_md, warnings=excluded.warnings,
               custom_json=excluded.custom_json, doc_sha256=excluded.doc_sha256, updated_at=excluded.updated_at",
            p![file_id, purpose.clone(), notes.clone(), warnings.clone(), custom.to_string(), f.os("sha256"), now],
        )?;
        db.exec("UPDATE files SET changed_since_review=0 WHERE id=?", p![file_id])?;
        if let Some(Json::Arr(tags)) = d.get("tags") {
            let t: Vec<String> = tags.iter().filter_map(|x| x.as_str().map(String::from)).collect();
            projects::set_tags(v, "file", file_id, &t)?;
        }
        timeline::add_coalesced(v, &f.s("project_id"), "file-doc", &format!("Documented {}", f.s("rel_path")), "file", file_id)?;
        Ok(())
    })?;
    search::index_file(v, file_id)?;
    docs_get(v, file_id)
}

pub fn doc_revisions(v: &Vault, file_id: &str) -> DbResult<Json> {
    let rows = v.db.query("SELECT * FROM file_doc_revisions WHERE file_id=? ORDER BY created_at DESC LIMIT 200", p![file_id])?;
    Ok(Json::Arr(
        rows.iter()
            .map(|r| {
                Json::obj()
                    .with("id", r.s("id"))
                    .with("purpose", r.s("purpose"))
                    .with("notes", r.s("notes_md"))
                    .with("warnings", r.s("warnings"))
                    .with("fileSha256", r.os("file_sha256"))
                    .with("at", r.i("created_at"))
            })
            .collect(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projects::{create, NewProject};
    use crate::testutil::*;
    use std::fs;

    fn linked(v: &Vault, dir: &Path) -> String {
        create(v, NewProject { name: "OMEN v2".into(), description: "".into(), category: "software".into(), status: "".into(), link_path: Some(dir.to_path_buf()), module: Json::obj() })
            .unwrap()
            .str_of("id")
            .unwrap()
    }
    fn id_at(v: &Vault, pid: &str, rel: &str) -> String {
        v.db.one("SELECT id FROM files WHERE project_id=? AND rel_path=?", p![pid, rel]).unwrap().unwrap().s("id")
    }
    fn bump_mtime(p: &Path) {
        // ensure mtime differs even on coarse-grained filesystems
        let f = fs::File::options().append(true).open(p).unwrap();
        f.set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(5)).unwrap();
    }

    #[test]
    fn modify_move_missing_and_docs_follow() {
        let (t, v) = vault();
        let d = t.path().join("omen");
        fs::create_dir_all(d.join("scripts")).unwrap();
        fs::create_dir_all(d.join(".git")).unwrap();
        fs::write(d.join(".git/HEAD"), "x").unwrap();
        fs::write(d.join("scripts/weapon_tuner.gsc"), "init() {}").unwrap();
        fs::write(d.join("scripts/util.gsc"), "util() {}").unwrap();
        let pid = linked(&v, &d);
        let r = scan(&v, &pid).unwrap();
        assert_eq!(r.added, 2, "ignored .git");
        let wid = id_at(&v, &pid, "scripts/weapon_tuner.gsc");
        docs_save(&v, &wid, &Json::parse(r#"{"purpose":"Handles weapon tuning","notes":"- instashoot config","tags":["weapons"]}"#).unwrap()).unwrap();

        // unchanged rescan: nothing hashed
        let r = scan(&v, &pid).unwrap();
        assert_eq!((r.hashed, r.added, r.modified), (0, 0, 0));

        // external modification
        fs::write(d.join("scripts/weapon_tuner.gsc"), "init() { tune(); }").unwrap();
        bump_mtime(&d.join("scripts/weapon_tuner.gsc"));
        let r = scan(&v, &pid).unwrap();
        assert_eq!(r.modified, 1);
        let g = get(&v, &wid).unwrap();
        assert_eq!(g.get("changedSinceReview").unwrap().as_bool(), Some(true));
        assert_eq!(g.get("docs").unwrap().get("fileChangedSinceDoc").unwrap().as_bool(), Some(true));

        // rename + move: identity and docs follow
        fs::create_dir_all(d.join("weapons")).unwrap();
        fs::rename(d.join("scripts/weapon_tuner.gsc"), d.join("weapons/tuner.gsc")).unwrap();
        let r = scan(&v, &pid).unwrap();
        assert_eq!((r.moved, r.added, r.missing), (1, 0, 0));
        assert_eq!(id_at(&v, &pid, "weapons/tuner.gsc"), wid);
        assert_eq!(get(&v, &wid).unwrap().get("docs").unwrap().str_of("purpose").unwrap(), "Handles weapon tuning");

        // delete -> missing, docs retained
        fs::remove_file(d.join("weapons/tuner.gsc")).unwrap();
        let r = scan(&v, &pid).unwrap();
        assert_eq!(r.missing, 1);
        assert_eq!(get(&v, &wid).unwrap().str_of("state").unwrap(), "missing");
        assert!(search::search(&v, "weapon tuning", None, &[], 10).unwrap().to_string().contains(&wid));
    }

    #[test]
    fn ambiguous_matches_go_to_review_not_auto_attach() {
        let (t, v) = vault();
        let d = t.path().join("p");
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join("a.cfg"), "same").unwrap();
        let pid = linked(&v, &d);
        scan(&v, &pid).unwrap();
        let aid = id_at(&v, &pid, "a.cfg");
        docs_save(&v, &aid, &Json::parse(r#"{"purpose":"A's purpose"}"#).unwrap()).unwrap();
        // a.cfg disappears; two identical copies appear -> ambiguous
        fs::remove_file(d.join("a.cfg")).unwrap();
        fs::write(d.join("b.cfg"), "same").unwrap();
        fs::write(d.join("c.cfg"), "same").unwrap();
        let r = scan(&v, &pid).unwrap();
        assert_eq!((r.moved, r.added, r.missing, r.needs_review), (0, 2, 1, 2));
        let bid = id_at(&v, &pid, "b.cfg");
        assert_eq!(get(&v, &bid).unwrap().get("docs").unwrap().str_of("purpose").unwrap(), "", "docs must not jump to a guess");
        // user confirms b.cfg is the original
        let revs = reviews(&v, &pid).unwrap();
        let rv = revs.as_arr().unwrap().iter().find(|x| x.get("candidate").unwrap().str_of("path").unwrap() == "b.cfg").unwrap();
        resolve_review(&v, &rv.str_of("id").unwrap(), true).unwrap();
        assert_eq!(id_at(&v, &pid, "b.cfg"), aid);
        assert_eq!(get(&v, &aid).unwrap().get("docs").unwrap().str_of("purpose").unwrap(), "A's purpose");
        assert_eq!(reviews(&v, &pid).unwrap().as_arr().unwrap().len(), 0);
    }

    #[test]
    fn rename_with_edit_is_flagged_for_review() {
        let (t, v) = vault();
        let d = t.path().join("p");
        fs::create_dir_all(d.join("x")).unwrap();
        fs::write(d.join("x/main.lua"), "v1").unwrap();
        let pid = linked(&v, &d);
        scan(&v, &pid).unwrap();
        fs::create_dir_all(d.join("y")).unwrap();
        fs::remove_file(d.join("x/main.lua")).unwrap();
        fs::write(d.join("y/main.lua"), "v2").unwrap();
        let r = scan(&v, &pid).unwrap();
        assert_eq!((r.moved, r.needs_review), (0, 1));
    }

    #[test]
    fn unreachable_root_changes_nothing() {
        let (t, v) = vault();
        let d = t.path().join("ext");
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join("f.txt"), "x").unwrap();
        let pid = linked(&v, &d);
        scan(&v, &pid).unwrap();
        fs::rename(&d, t.path().join("ext-unplugged")).unwrap();
        assert!(scan(&v, &pid).is_err());
        assert_eq!(v.db.scalar_i64("SELECT count(*) FROM files WHERE state='missing'", p![]).unwrap(), 0);
        crate::projects::relink(&v, &pid, &t.path().join("ext-unplugged")).unwrap();
        let r = scan(&v, &pid).unwrap();
        assert_eq!((r.added, r.missing), (0, 0));
    }

    #[test]
    fn import_never_overwrites_and_dedupes() {
        let (t, v) = vault();
        let pid = create(&v, NewProject { name: "Prints".into(), description: "".into(), category: "3d-printing".into(), status: "".into(), link_path: None, module: Json::obj() }).unwrap().str_of("id").unwrap();
        let src = t.path().join("src");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("cat.stl"), "solid c\nvertex 0 0 0\nvertex 10 5 2\nendsolid").unwrap();
        fs::write(src.join("other.stl"), "solid d\nvertex 0 0 0\nvertex 1 1 1\nendsolid").unwrap();
        let r = import(&v, &pid, &[src.join("cat.stl")], "models", false).unwrap();
        assert_eq!(r.get("imported").unwrap().as_arr().unwrap().len(), 1);
        let r = import(&v, &pid, &[src.join("cat.stl")], "models", false).unwrap();
        assert_eq!(r.get("skipped").unwrap().as_arr().unwrap().len(), 1, "duplicate content skipped");
        // same name, different content -> suffixed, original intact
        fs::write(src.join("cat.stl"), "solid c2\nvertex 0 0 0\nvertex 3 3 3\nendsolid").unwrap();
        let r = import(&v, &pid, &[src.join("cat.stl")], "models", false).unwrap();
        assert_eq!(r.get("imported").unwrap().as_arr().unwrap()[0].str_of("path").unwrap(), "models/cat (2).stl");
        let fid = id_at(&v, &pid, "models/cat.stl");
        let dims = get(&v, &fid).unwrap();
        assert_eq!(dims.get("meshDimensionsMm").unwrap().to_string(), "[10,5,2]");
        assert!(import(&v, &pid, &[src.join("other.stl")], "../escape", false).is_err());
    }

    #[test]
    fn edit_text_with_conflict_detection() {
        let (t, v) = vault();
        let d = t.path().join("p");
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join("cfg.ini"), "\u{feff}a=1\r\nb=2\r\n").unwrap();
        fs::write(d.join("blob.bin"), [0u8, 1, 2]).unwrap();
        let pid = linked(&v, &d);
        scan(&v, &pid).unwrap();
        let fid = id_at(&v, &pid, "cfg.ini");
        let r = read_text(&v, &fid).unwrap();
        assert_eq!(r.str_of("eol").unwrap(), "crlf");
        assert_eq!(r.get("bom").unwrap().as_bool(), Some(true));
        assert_eq!(r.str_of("text").unwrap(), "a=1\r\nb=2\r\n");
        assert!(read_text(&v, &id_at(&v, &pid, "blob.bin")).is_err());
        let sha = r.str_of("sha256").unwrap();
        // editor (textarea) sends LF; CRLF + BOM are preserved on disk
        let w = write_text(&v, &fid, "a=1\nb=3\n", &sha, "crlf", true).unwrap();
        assert_eq!(fs::read(d.join("cfg.ini")).unwrap(), b"\xEF\xBB\xBFa=1\r\nb=3\r\n");
        // previous content kept as a version (baseline from the scan), new content captured too
        let vs = crate::versions::list(&v, &fid).unwrap();
        assert_eq!(vs.as_arr().unwrap().len(), 2);
        // external change -> conflict, nothing written
        fs::write(d.join("cfg.ini"), "external").unwrap();
        let e = write_text(&v, &fid, "mine", &w.str_of("sha256").unwrap(), "lf", false).unwrap_err();
        assert!(e.starts_with("CONFLICT:"));
        assert_eq!(fs::read_to_string(d.join("cfg.ini")).unwrap(), "external");
    }

    #[test]
    fn timeline_records_what_changed() {
        let (t, v) = vault();
        let d = t.path().join("p");
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join("notes.txt"), "one\ntwo\n").unwrap();
        let pid = linked(&v, &d);
        scan(&v, &pid).unwrap();
        let fid = id_at(&v, &pid, "notes.txt");
        let tl = |kind: &str| -> Vec<Json> {
            crate::timeline::list(&v, &pid, 100, None).unwrap().as_arr().unwrap().iter().filter(|e| e.str_of("kind").as_deref() == Some(kind)).cloned().collect()
        };
        // two saves in the editor: one entry spanning both, and no "changes detected" entry
        let s0 = read_text(&v, &fid).unwrap().str_of("sha256").unwrap();
        let s1 = write_text(&v, &fid, "one\nTWO\n", &s0, "lf", false).unwrap().str_of("sha256").unwrap();
        let s2 = write_text(&v, &fid, "one\nTWO\nthree\n", &s1, "lf", false).unwrap().str_of("sha256").unwrap();
        let edits = tl("edit");
        assert_eq!(edits.len(), 1);
        let dd = edits[0].get("data").unwrap();
        assert_eq!((dd.str_of("from").unwrap(), dd.str_of("to").unwrap()), (s0.clone(), s2.clone()));
        assert!(tl("files").iter().all(|e| !e.str_of("summary").unwrap().contains("modified")), "in-app edit isn't reported as an outside change");
        let c = crate::versions::compare(&v, &fid, &format!("sha:{s0}"), &format!("sha:{s2}")).unwrap();
        assert_eq!(c.get("diff").unwrap().get("added").unwrap().as_i64(), Some(2));
        // an outside change, a new file and a removed file show up in one scan entry
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(d.join("notes.txt"), "outside\n").unwrap();
        fs::write(d.join("new.cfg"), "x=1").unwrap();
        let f = fs::File::options().write(true).open(d.join("notes.txt")).unwrap();
        f.set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(5)).unwrap();
        scan(&v, &pid).unwrap();
        let files = tl("files");
        let ch = files[0].get("data").unwrap().get("changes").unwrap().as_arr().unwrap().clone();
        let kinds: Vec<String> = ch.iter().map(|c| format!("{}:{}", c.str_of("change").unwrap(), c.str_of("path").unwrap())).collect();
        assert_eq!(kinds, vec!["added:new.cfg", "modified:notes.txt"]);
        assert_eq!(ch[1].str_of("from").unwrap(), s2);
        // a further editor save after an outside change starts a new entry
        let cur = read_text(&v, &fid).unwrap().str_of("sha256").unwrap();
        write_text(&v, &fid, "outside\nmore\n", &cur, "lf", false).unwrap();
        assert_eq!(tl("edit").len(), 2);
        // content that was never kept gives a clear error
        let e = crate::versions::compare(&v, &fid, &format!("sha:{}", "0".repeat(64)), "current").unwrap_err();
        assert!(e.starts_with("NOT_KEPT:"));
    }

    #[test]
    fn import_move_and_undo_moves_back() {
        let (t, v) = vault();
        let d = t.path().join("p");
        fs::create_dir_all(&d).unwrap();
        let pid = linked(&v, &d);
        let src = t.path().join("Downloads");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("glock.3mf"), "model").unwrap();
        fs::write(src.join("mag.stl"), "mag").unwrap();
        let r = import_op(&v, &pid, &[src.join("glock.3mf"), src.join("mag.stl")], "prints", false, true).unwrap();
        assert!(d.join("prints/glock.3mf").exists() && !src.join("glock.3mf").exists(), "moved, not copied");
        // undo: unchanged file goes back where it came from; edited one stays
        fs::write(d.join("prints/mag.stl"), "edited").unwrap();
        let u = undo_import(&v, &r.str_of("batchId").unwrap()).unwrap();
        assert_eq!(u.get("removed").unwrap().as_arr().unwrap().len(), 1);
        assert_eq!(fs::read_to_string(src.join("glock.3mf")).unwrap(), "model");
        assert!(!d.join("prints/glock.3mf").exists());
        assert!(d.join("prints/mag.stl").exists() && !src.join("mag.stl").exists());
    }

    #[test]
    fn managed_project_can_switch_to_working_in_place() {
        let (t, v) = vault();
        let pid = create(&v, NewProject { name: "3D Prints".into(), description: "".into(), category: "3d-printing".into(), status: "".into(), link_path: None, module: Json::obj() }).unwrap().str_of("id").unwrap();
        let src = t.path().join("dl");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("old.stl"), "in vault").unwrap();
        import(&v, &pid, &[src.join("old.stl")], "", false).unwrap();
        let fid = id_at(&v, &pid, "old.stl");
        docs_save(&v, &fid, &Json::parse(r#"{"purpose":"keep my notes"}"#).unwrap()).unwrap();
        // the user's own folder, with a different file of the same name already in it
        let mine = t.path().join("My Prints");
        fs::create_dir_all(&mine).unwrap();
        fs::write(mine.join("glock.3mf"), "edit me in place").unwrap();
        fs::write(mine.join("old.stl"), "something else").unwrap();
        let r = projects::use_folder(&v, &pid, &mine).unwrap();
        assert_eq!(r.str_of("mode").unwrap(), "linked");
        assert_eq!(fs::read_to_string(mine.join("old.stl")).unwrap(), "something else", "never overwritten");
        let moved_rel = r.get("renamed").unwrap().as_arr().unwrap()[0].as_str().unwrap().to_string();
        assert_eq!(fs::read_to_string(mine.join(&moved_rel)).unwrap(), "in vault");
        assert_eq!(id_at(&v, &pid, &moved_rel), fid, "notes stay with the vault's copy");
        assert!(id_at(&v, &pid, "glock.3mf").len() > 0, "files in the folder are now part of the project");
        assert!(projects::use_folder(&v, &pid, &mine).is_err(), "only managed projects");
    }

    #[test]
    fn type_groups_never_hide_files() {
        let (t, v) = vault();
        let d = t.path().join("g");
        fs::create_dir_all(d.join("maps")).unwrap();
        fs::write(d.join("a.gsc"), "x").unwrap();
        fs::write(d.join("b.iwi"), "x").unwrap();
        fs::write(d.join("maps/c.d3dbsp"), "x").unwrap();
        fs::write(d.join("README"), "no extension").unwrap();
        let pid = create(&v, NewProject { name: "Old Omen".into(), description: "".into(), category: "game".into(), status: "".into(), link_path: Some(d.clone()), module: Json::obj() }).unwrap().str_of("id").unwrap();
        scan(&v, &pid).unwrap();
        let gs = groups(&v, &pid).unwrap();
        let g = |id: &str| gs.as_arr().unwrap().iter().find(|x| x.str_of("id").as_deref() == Some(id)).cloned().unwrap();
        assert_eq!(g("scripts").get("count").and_then(|x| x.as_i64()), Some(1), "gsc is a script in game projects");
        assert_eq!(g("art").get("count").and_then(|x| x.as_i64()), Some(1), "iwi is a texture");
        let other = g("other");
        assert_eq!(other.get("count").and_then(|x| x.as_i64()), Some(2));
        // every group's exts filter finds exactly its files, including files without an extension
        let total: usize = gs.as_arr().unwrap().iter().map(|x| list(&v, &pid, &Json::obj().with("exts", x.get("exts").cloned().unwrap())).unwrap().as_arr().unwrap().len()).sum();
        assert_eq!(total, 4);
    }

    #[test]
    fn rename_and_safe_delete() {
        let (t, v) = vault();
        let d = t.path().join("p");
        fs::create_dir_all(d.join("scripts")).unwrap();
        fs::create_dir_all(d.join("menus")).unwrap();
        fs::write(d.join("scripts/createserver.menu"), "menu a").unwrap();
        fs::write(d.join("menus/createserver.menu"), "menu b").unwrap();
        fs::write(d.join("scripts/other.gsc"), "x").unwrap();
        let pid = linked(&v, &d);
        scan(&v, &pid).unwrap();
        // identically named files are distinguishable in search by path + absolute path
        let hits = search::search(&v, "createserver", None, &["file".into()], 10).unwrap();
        let paths: Vec<String> = hits.as_arr().unwrap().iter().map(|h| h.str_of("path").unwrap()).collect();
        assert!(paths.contains(&"scripts/createserver.menu".to_string()) && paths.contains(&"menus/createserver.menu".to_string()));
        let abs = hits.as_arr().unwrap()[0].str_of("absPath").unwrap();
        // substring match inside a word: "server" finds "createserver.menu"
        let sub = search::search(&v, "server", None, &[], 10).unwrap();
        assert_eq!(sub.as_arr().unwrap().iter().filter(|h| h.str_of("name").as_deref() == Some("createserver.menu")).count(), 2);
        assert!(std::path::Path::new(&abs).is_file(), "absPath must be the real file: {abs}");

        let fid = id_at(&v, &pid, "scripts/createserver.menu");
        docs_save(&v, &fid, &Json::parse(r#"{"purpose":"Server creation menu"}"#).unwrap()).unwrap();
        // validation
        for bad in ["", "a/b", "x:y", "CON.txt", "..", "a?b"] {
            assert!(rename(&v, &fid, bad).is_err(), "{bad}");
        }
        assert!(rename(&v, &fid, "other.gsc").is_err(), "existing target refused");
        let r = rename(&v, &fid, "create_server.menu").unwrap();
        assert_eq!(r.str_of("path").unwrap(), "scripts/create_server.menu");
        assert!(d.join("scripts/create_server.menu").is_file() && !d.join("scripts/createserver.menu").exists());
        assert_eq!(r.get("docs").unwrap().str_of("purpose").unwrap(), "Server creation menu", "identity kept");
        let rs = scan(&v, &pid).unwrap();
        assert_eq!((rs.added, rs.missing, rs.moved), (0, 0, 0), "index already consistent");

        // delete: copy kept, file gone, record hidden from lists/search/counts, rescan doesn't resurrect
        let res = delete(&v, &[fid.clone()]).unwrap();
        assert_eq!(res.get("deleted").unwrap().as_arr().unwrap().len(), 1);
        assert!(!d.join("scripts/create_server.menu").exists());
        assert!(!list(&v, &pid, &Json::obj()).unwrap().to_string().contains(&fid));
        assert!(search::search(&v, "create_server", None, &[], 10).unwrap().as_arr().unwrap().is_empty());
        assert_eq!(crate::projects::get(&v, &pid).unwrap().get("fileCount").unwrap().as_i64(), Some(2));
        let rs = scan(&v, &pid).unwrap();
        assert_eq!((rs.missing, rs.added), (0, 0));
        assert!(list(&v, &pid, &Json::obj().with("state", "deleted")).unwrap().to_string().contains(&fid));
        // restore from the stored copy
        restore_deleted(&v, &fid).unwrap();
        assert_eq!(fs::read_to_string(d.join("scripts/create_server.menu")).unwrap(), "menu a");
        let g = get(&v, &fid).unwrap();
        assert_eq!(g.str_of("state").unwrap(), "present");
        assert_eq!(g.get("docs").unwrap().str_of("purpose").unwrap(), "Server creation menu");
    }

    #[test]
    fn stop_tracking_import_undo_and_move() {
        let (t, v) = vault();
        let d = t.path().join("p");
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join("keep.txt"), "pre-existing").unwrap();
        fs::write(d.join("noise.log"), "x").unwrap();
        let pid = linked(&v, &d);
        scan(&v, &pid).unwrap();
        // stop tracking: file stays on disk, doesn't come back on rescan
        let nid = id_at(&v, &pid, "noise.log");
        stop_tracking(&v, &[nid], false).unwrap();
        assert!(d.join("noise.log").exists());
        scan(&v, &pid).unwrap();
        assert_eq!(v.db.scalar_i64("SELECT count(*) FROM files WHERE rel_path='noise.log'", p![]).unwrap(), 0);

        // import preview shows destination + conflicts, changes nothing
        let src = t.path().join("downloads");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("keep.txt"), "different content").unwrap();
        fs::write(src.join("model.stl"), "solid m").unwrap();
        let pv = import_preview(&v, &pid, &[src.join("keep.txt"), src.join("model.stl")], "").unwrap();
        let items = pv.get("items").unwrap().as_arr().unwrap().clone();
        assert_eq!(items[0].str_of("status").unwrap(), "rename");
        assert_eq!(items[0].str_of("dest").unwrap(), "keep (2).txt");
        assert!(!d.join("keep (2).txt").exists());
        // import, edit one copy, undo: only the untouched copy is removed; pre-existing file intact
        let r = import(&v, &pid, &[src.join("keep.txt"), src.join("model.stl")], "", false).unwrap();
        let batch = r.str_of("batchId").unwrap();
        fs::write(d.join("model.stl"), "edited after import").unwrap();
        let u = undo_import(&v, &batch).unwrap();
        assert_eq!(u.get("removed").unwrap().as_arr().unwrap().len(), 1);
        assert!(!d.join("keep (2).txt").exists());
        assert!(d.join("model.stl").exists(), "changed since import -> kept");
        assert_eq!(fs::read_to_string(d.join("keep.txt")).unwrap(), "pre-existing");
        assert!(undo_import(&v, &batch).is_err());

        // move to another project keeps identity, notes and versions
        let d2 = t.path().join("q");
        fs::create_dir_all(&d2).unwrap();
        let pid2 = create(&v, NewProject { name: "Other".into(), description: "".into(), category: "general".into(), status: "".into(), link_path: Some(d2.clone()), module: Json::obj() }).unwrap().str_of("id").unwrap();
        let kid = id_at(&v, &pid, "keep.txt");
        docs_save(&v, &kid, &Json::parse(r#"{"purpose":"wrong project"}"#).unwrap()).unwrap();
        let versions_before = crate::versions::list(&v, &kid).unwrap().as_arr().unwrap().len();
        let m = move_to_project(&v, &[kid.clone()], &pid2, "imported").unwrap();
        assert_eq!(m.get("moved").unwrap().as_arr().unwrap()[0].str_of("").unwrap_or_default(), "");
        assert!(d2.join("imported/keep.txt").exists() && !d.join("keep.txt").exists());
        let g = get(&v, &kid).unwrap();
        assert_eq!(g.str_of("projectId").unwrap(), pid2);
        assert_eq!(g.get("docs").unwrap().str_of("purpose").unwrap(), "wrong project");
        assert_eq!(crate::versions::list(&v, &kid).unwrap().as_arr().unwrap().len(), versions_before);
        let r1 = scan(&v, &pid).unwrap();
        assert_eq!(r1.missing, 0, "source project doesn't report it missing");
    }

    #[test]
    fn stl_binary_dims() {
        let t = TempDir::new("stl");
        let mut b = vec![0u8; 80];
        b.extend_from_slice(&1u32.to_le_bytes());
        b.extend_from_slice(&[0u8; 12]);
        for v in [[0f32, 0., 0.], [20., 0., 0.], [0., 7.5, 3.]] {
            for x in v {
                b.extend_from_slice(&x.to_le_bytes());
            }
        }
        b.extend_from_slice(&[0, 0]);
        let p = t.path().join("b.stl");
        fs::write(&p, b).unwrap();
        assert_eq!(stl_dimensions(&p), Some((20.0, 7.5, 3.0)));
    }

    #[test]
    fn ignore_rules() {
        let ig = Ignore::parse("Library/\nAssets/Gen/\n*.log\nsecret.txt");
        assert!(ig.skip_dir("Library", "Library"));
        assert!(ig.skip_dir("node_modules", "a/node_modules"));
        assert!(ig.skip_dir("Gen", "Assets/Gen"));
        assert!(!ig.skip_dir("Gen", "Other/Gen"));
        assert!(ig.skip_file("x.LOG", "x.LOG"));
        assert!(ig.skip_file("secret.txt", "a/secret.txt"));
        assert!(!ig.skip_file("main.rs", "main.rs"));
        let ig2 = Ignore::parse("scripts/old/test.gsc");
        assert!(ig2.skip_file("test.gsc", "scripts/old/test.gsc"));
        assert!(!ig2.skip_file("test.gsc", "scripts/test.gsc"));
    }
}

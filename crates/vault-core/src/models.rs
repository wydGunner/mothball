//! 3D model files: triangle meshes for the preview (STL, OBJ, 3MF) and the
//! print information stored inside .3mf files by Bambu Studio, OrcaSlicer,
//! PrusaSlicer and others (printer, filaments, layer height, print time,
//! weight, plate thumbnails). Nothing here writes to the file.

use crate::db::DbResult;
use crate::json::Json;
use crate::zip::Archive;
use std::collections::HashMap;
use std::path::Path;

/// Most triangles sent to the preview; larger models are thinned evenly and
/// the result says so.
pub const MAX_PREVIEW_TRIANGLES: usize = 400_000;
const MAX_MODEL_XML: usize = 400 * 1024 * 1024;

pub struct Mesh {
    /// 9 floats per triangle (x,y,z × 3)
    pub tris: Vec<f32>,
}
impl Mesh {
    fn count(&self) -> usize {
        self.tris.len() / 9
    }
}

// ---------------------------------------------------------------- STL / OBJ

pub fn read_stl(data: &[u8]) -> Result<Mesh, String> {
    let binary = data.len() >= 84 && {
        let n = u32::from_le_bytes([data[80], data[81], data[82], data[83]]) as usize;
        84 + n * 50 == data.len()
    };
    let mut tris = Vec::new();
    if binary {
        let n = (data.len() - 84) / 50;
        tris.reserve(n * 9);
        for t in 0..n {
            let base = 84 + t * 50 + 12;
            for k in 0..9 {
                let o = base + k * 4;
                tris.push(f32::from_le_bytes([data[o], data[o + 1], data[o + 2], data[o + 3]]));
            }
        }
    } else {
        let text = String::from_utf8_lossy(data);
        for line in text.lines() {
            let mut it = line.split_whitespace();
            if it.next().map(|w| w.eq_ignore_ascii_case("vertex")) == Some(true) {
                let v: Vec<f32> = it.filter_map(|x| x.parse().ok()).collect();
                if v.len() == 3 {
                    tris.extend_from_slice(&v);
                }
            }
        }
        tris.truncate(tris.len() / 9 * 9);
    }
    if tris.is_empty() {
        return Err("No triangles found in this STL file".into());
    }
    Ok(Mesh { tris })
}

pub fn read_obj(data: &[u8]) -> Result<Mesh, String> {
    let text = String::from_utf8_lossy(data);
    let mut verts: Vec<[f32; 3]> = Vec::new();
    let mut tris = Vec::new();
    for line in text.lines() {
        let mut it = line.split_whitespace();
        match it.next() {
            Some("v") => {
                let v: Vec<f32> = it.take(3).filter_map(|x| x.parse().ok()).collect();
                if v.len() == 3 {
                    verts.push([v[0], v[1], v[2]]);
                }
            }
            Some("f") => {
                let idx: Vec<usize> = it
                    .filter_map(|w| w.split('/').next()?.parse::<i64>().ok())
                    .filter_map(|i| {
                        let n = verts.len() as i64;
                        let k = if i < 0 { n + i } else { i - 1 };
                        (0..n).contains(&k).then_some(k as usize)
                    })
                    .collect();
                for k in 1..idx.len().saturating_sub(1) {
                    for &i in &[idx[0], idx[k], idx[k + 1]] {
                        tris.extend_from_slice(&verts[i]);
                    }
                }
            }
            _ => {}
        }
    }
    if tris.is_empty() {
        return Err("No faces found in this OBJ file".into());
    }
    Ok(Mesh { tris })
}

// ---------------------------------------------------------------- tiny XML scanning

/// Calls `f(tag_name, attrs_text)` for every start/empty tag, in order.
fn each_tag<'a>(xml: &'a str, mut f: impl FnMut(&'a str, &'a str)) {
    let b = xml.as_bytes();
    let mut i = 0;
    while let Some(off) = xml[i..].find('<') {
        let s = i + off + 1;
        if s >= b.len() {
            break;
        }
        if matches!(b[s], b'/' | b'?' | b'!') {
            i = s;
            continue;
        }
        let end = match xml[s..].find('>') {
            Some(e) => s + e,
            None => break,
        };
        let inner = &xml[s..end];
        let name_end = inner.find(|c: char| c.is_whitespace() || c == '/').unwrap_or(inner.len());
        f(&inner[..name_end], &inner[name_end..]);
        i = end + 1;
    }
}

fn attr<'a>(attrs: &'a str, name: &str) -> Option<&'a str> {
    let mut rest = attrs;
    loop {
        let p = rest.find(name)?;
        let before_ok = p == 0 || rest.as_bytes()[p - 1].is_ascii_whitespace();
        let after = rest[p + name.len()..].trim_start();
        if before_ok && after.starts_with('=') {
            let v = after[1..].trim_start();
            let q = v.chars().next()?;
            if q == '"' || q == '\'' {
                let v = &v[1..];
                return v.find(q).map(|e| &v[..e]);
            }
        }
        rest = &rest[p + name.len()..];
    }
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&#34;", "\"").replace("&#39;", "'").replace("&nbsp;", " ").replace("&amp;", "&")
}

/// Model descriptions are often HTML, sometimes escaped twice: turn them into plain text.
fn plain_text(s: &str) -> String {
    let mut t = s.to_string();
    for _ in 0..3 {
        let u = unescape(&t);
        if u == t {
            break;
        }
        t = u;
    }
    let t = t.replace("</p>", "\n").replace("<br>", "\n").replace("<br/>", "\n").replace("</li>", "\n").replace("<li>", "• ").replace("</h2>", "\n");
    let mut out = String::new();
    let mut in_tag = false;
    for c in t.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    let lines: Vec<String> = out.lines().map(|l| l.split_whitespace().collect::<Vec<_>>().join(" ")).filter(|l| !l.is_empty()).collect();
    let joined = lines.join("\n");
    if joined.chars().count() > 1500 {
        format!("{}…", joined.chars().take(1500).collect::<String>())
    } else {
        joined
    }
}

// ---------------------------------------------------------------- 3MF meshes

type Xf = [f32; 12]; // 3MF matrix: m00 m01 m02 m10 m11 m12 m20 m21 m22 m30 m31 m32
const IDENT: Xf = [1., 0., 0., 0., 1., 0., 0., 0., 1., 0., 0., 0.];

fn parse_xf(s: Option<&str>) -> Xf {
    let v: Vec<f32> = s.unwrap_or("").split_whitespace().filter_map(|x| x.parse().ok()).collect();
    if v.len() == 12 {
        let mut m = [0f32; 12];
        m.copy_from_slice(&v);
        m
    } else {
        IDENT
    }
}
/// a then b (row-vector convention used by 3MF: p' = p·M)
fn mul(a: &Xf, b: &Xf) -> Xf {
    let r = |i: usize, j: usize, m: &Xf| m[i * 3 + j];
    let mut o = [0f32; 12];
    for i in 0..4 {
        for j in 0..3 {
            let mut s = 0.0;
            for k in 0..3 {
                s += r(i, k, a) * r(k, j, b);
            }
            if i == 3 {
                s += r(3, j, b);
            }
            o[i * 3 + j] = s;
        }
    }
    o
}
fn apply(m: &Xf, p: [f32; 3]) -> [f32; 3] {
    [
        p[0] * m[0] + p[1] * m[3] + p[2] * m[6] + m[9],
        p[0] * m[1] + p[1] * m[4] + p[2] * m[7] + m[10],
        p[0] * m[2] + p[1] * m[5] + p[2] * m[8] + m[11],
    ]
}

enum Obj {
    Mesh(Vec<[f32; 3]>, Vec<[u32; 3]>),
    Comps(Vec<(String, String, Xf)>), // (path, objectid, transform)
}

fn parse_model(xml: &str, objects: &mut HashMap<(String, String), Obj>, path: &str) -> Vec<(String, Xf)> {
    let mut build = Vec::new();
    let mut cur: Option<String> = None;
    let mut verts = Vec::new();
    let mut tris = Vec::new();
    let mut comps = Vec::new();
    let mut flush = |cur: &mut Option<String>, verts: &mut Vec<[f32; 3]>, tris: &mut Vec<[u32; 3]>, comps: &mut Vec<(String, String, Xf)>| {
        if let Some(id) = cur.take() {
            let o = if !comps.is_empty() { Obj::Comps(std::mem::take(comps)) } else { Obj::Mesh(std::mem::take(verts), std::mem::take(tris)) };
            objects.insert((path.to_string(), id), o);
        }
    };
    each_tag(xml, |name, a| {
        let local = name.rsplit(':').next().unwrap_or(name);
        match local {
            "object" => {
                flush(&mut cur, &mut verts, &mut tris, &mut comps);
                cur = attr(a, "id").map(String::from);
            }
            "vertex" => {
                let g = |k| attr(a, k).and_then(|x| x.parse::<f32>().ok()).unwrap_or(0.0);
                verts.push([g("x"), g("y"), g("z")]);
            }
            "triangle" => {
                let g = |k| attr(a, k).and_then(|x| x.parse::<u32>().ok());
                if let (Some(a1), Some(a2), Some(a3)) = (g("v1"), g("v2"), g("v3")) {
                    tris.push([a1, a2, a3]);
                }
            }
            "component" => {
                let p = attr(a, "p:path").or_else(|| attr(a, "path")).unwrap_or(path).trim_start_matches('/').to_string();
                comps.push((p, attr(a, "objectid").unwrap_or("").to_string(), parse_xf(attr(a, "transform"))));
            }
            "item" => {
                flush(&mut cur, &mut verts, &mut tris, &mut comps);
                build.push((attr(a, "objectid").unwrap_or("").to_string(), parse_xf(attr(a, "transform"))));
            }
            _ => {}
        }
    });
    flush(&mut cur, &mut verts, &mut tris, &mut comps);
    build
}

fn emit(objects: &HashMap<(String, String), Obj>, key: &(String, String), xf: &Xf, out: &mut Vec<f32>, depth: usize) {
    if depth > 8 {
        return;
    }
    match objects.get(key) {
        Some(Obj::Mesh(v, t)) => {
            for tri in t {
                for &i in tri {
                    if let Some(p) = v.get(i as usize) {
                        out.extend_from_slice(&apply(xf, *p));
                    }
                }
            }
        }
        Some(Obj::Comps(c)) => {
            for (p, id, cx) in c {
                emit(objects, &(p.clone(), id.clone()), &mul(cx, xf), out, depth + 1);
            }
        }
        None => {}
    }
}

pub fn read_3mf(path: &Path) -> Result<Mesh, String> {
    let mut a = Archive::open(path)?;
    let mut objects = HashMap::new();
    let model_names: Vec<String> = a.entries.iter().filter(|e| e.name.to_lowercase().ends_with(".model")).map(|e| e.name.clone()).collect();
    let root = model_names.iter().find(|n| n.eq_ignore_ascii_case("3D/3dmodel.model")).cloned().ok_or("This .3mf has no 3D/3dmodel.model")?;
    let mut build = Vec::new();
    for n in &model_names {
        let e = a.find(n).unwrap();
        let xml = String::from_utf8_lossy(&a.read(&e, MAX_MODEL_XML)?).to_string();
        let b = parse_model(&xml, &mut objects, n);
        if *n == root {
            build = b;
        }
    }
    let mut tris = Vec::new();
    for (id, xf) in &build {
        emit(&objects, &(root.clone(), id.clone()), xf, &mut tris, 0);
    }
    if tris.is_empty() {
        // no build section: show every mesh as stored
        for (k, o) in &objects {
            if matches!(o, Obj::Mesh(..)) {
                emit(&objects, k, &IDENT, &mut tris, 0);
            }
        }
    }
    if tris.is_empty() {
        return Err("No triangles found in this .3mf".into());
    }
    Ok(Mesh { tris })
}

/// Mesh for the preview as base64 little-endian float32 triangles.
pub fn preview_mesh(path: &Path, ext: &str) -> DbResult<Json> {
    let md = std::fs::metadata(path).map_err(|e| e.to_string())?;
    if md.len() > 1024 * 1024 * 1024 {
        return Err("This model is larger than 1 GB; open it in your slicer instead.".into());
    }
    let mesh = match ext {
        "stl" => read_stl(&std::fs::read(path).map_err(|e| e.to_string())?)?,
        "obj" => read_obj(&std::fs::read(path).map_err(|e| e.to_string())?)?,
        "3mf" => read_3mf(path)?,
        _ => return Err("No 3D preview for this file type".into()),
    };
    let total = mesh.count();
    let (mut mn, mut mx) = ([f32::MAX; 3], [f32::MIN; 3]);
    for c in mesh.tris.chunks(3) {
        for i in 0..3 {
            mn[i] = mn[i].min(c[i]);
            mx[i] = mx[i].max(c[i]);
        }
    }
    let shown = if total <= MAX_PREVIEW_TRIANGLES { mesh.tris } else { simplify(&mesh.tris, mn, mx, MAX_PREVIEW_TRIANGLES) };
    let mut bytes = Vec::with_capacity(shown.len() * 4);
    for v in &shown {
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    let step = if shown.len() / 9 < total { 2 } else { 1 };
    Ok(Json::obj()
        .with("triangles", total as i64)
        .with("shown", (bytes.len() / 36) as i64)
        .with("reduced", step > 1)
        .with("min", Json::Arr(mn.iter().map(|x| Json::from(*x as f64)).collect()))
        .with("max", Json::Arr(mx.iter().map(|x| Json::from(*x as f64)).collect()))
        .with("data", crate::files::base64(&bytes)))
}

/// Vertex-clustering simplification: snaps vertices to a grid and drops
/// triangles that collapse, so big models keep their shape (no holes) at a
/// lower resolution. The grid is made coarser until the result fits `cap`.
fn simplify(tris: &[f32], mn: [f32; 3], mx: [f32; 3], cap: usize) -> Vec<f32> {
    let size = (0..3).map(|i| mx[i] - mn[i]).fold(0f32, f32::max).max(1e-6);
    let mut res = 2048f32;
    loop {
        let cell = size / res;
        let q = |v: f32, i: usize| ((v - mn[i]) / cell).round() as i32;
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        for t in tris.chunks_exact(9) {
            let k: [(i32, i32, i32); 3] = [
                (q(t[0], 0), q(t[1], 1), q(t[2], 2)),
                (q(t[3], 0), q(t[4], 1), q(t[5], 2)),
                (q(t[6], 0), q(t[7], 1), q(t[8], 2)),
            ];
            if k[0] == k[1] || k[1] == k[2] || k[0] == k[2] {
                continue;
            }
            let mut key = k;
            key.sort();
            if !seen.insert(key) {
                continue;
            }
            for (x, y, z) in k {
                out.extend_from_slice(&[mn[0] + x as f32 * cell, mn[1] + y as f32 * cell, mn[2] + z as f32 * cell]);
            }
            if out.len() / 9 > cap {
                break;
            }
        }
        if out.len() / 9 <= cap || res < 16.0 {
            return out;
        }
        res *= 0.7;
    }
}

// ---------------------------------------------------------------- 3MF print info

fn first_str(j: &Json, k: &str) -> Option<String> {
    match j.get(k)? {
        Json::Str(s) => Some(s.clone()),
        Json::Arr(a) => a.first().and_then(|x| x.as_str()).map(String::from),
        x => Some(x.to_string()),
    }
}
fn str_list(j: &Json, k: &str) -> Vec<String> {
    match j.get(k) {
        Some(Json::Arr(a)) => a.iter().filter_map(|x| x.as_str().map(String::from)).collect(),
        Some(Json::Str(s)) => s.split(';').map(String::from).collect(),
        _ => vec![],
    }
}

/// The slicer's plate picture as a data URL (plate 1 / thumbnail / any PNG under Metadata).
fn thumbnail_of(a: &mut Archive<std::fs::File>) -> Option<String> {
    let mut thumbs: Vec<String> = a.entries.iter().filter(|e| e.name.starts_with("Metadata/") && e.name.to_lowercase().ends_with(".png")).map(|e| e.name.clone()).collect();
    thumbs.sort_by_key(|n| {
        let l = n.to_lowercase();
        if l == "metadata/plate_1.png" {
            0
        } else if l.contains("thumbnail") && !l.contains("small") {
            1
        } else if l.contains("plate_") && !l.contains("no_light") && !l.contains("top") && !l.contains("pick") && !l.contains("small") {
            2
        } else {
            9
        }
    });
    let e = a.find(thumbs.first()?)?;
    if e.usize > 8 * 1024 * 1024 {
        return None;
    }
    a.read(&e, 8 * 1024 * 1024).ok().map(|d| format!("data:image/png;base64,{}", crate::files::base64(&d)))
}

/// Only the plate picture — cheap, for project cards.
pub fn threemf_thumbnail(path: &Path) -> Option<String> {
    thumbnail_of(&mut Archive::open(path).ok()?)
}

/// Print-relevant information found in a .3mf. Only reports what the file
/// actually contains; fields that aren't there are left out.
pub fn threemf_info(path: &Path) -> DbResult<Json> {
    let mut a = Archive::open(path)?;
    let mut out = Json::obj();
    let read_text = |a: &mut Archive<std::fs::File>, name: &str, limit: usize| -> Option<String> {
        let e = a.find(name)?;
        a.read(&e, limit).ok().map(|d| String::from_utf8_lossy(&d).to_string())
    };

    // model metadata (title, designer, application) — read only the head of big models
    if let Some(e) = a.find("3D/3dmodel.model") {
        if let Ok(d) = a.read(&e, MAX_MODEL_XML) {
            let head = String::from_utf8_lossy(&d[..d.len().min(256 * 1024)]).to_string();
            let mut meta = Json::obj();
            let mut rest = head.as_str();
            while let Some(p) = rest.find("<metadata") {
                let r = &rest[p..];
                let Some(close) = r.find('>') else { break };
                let open = &r[..close];
                let name = attr(open, "name").unwrap_or("").to_string();
                let val = if open.ends_with('/') { String::new() } else { r[close + 1..].find("</").map(|e| unescape(r[close + 1..close + 1 + e].trim())).unwrap_or_default() };
                if !name.is_empty() && !val.is_empty() {
                    meta.set(&name, val);
                }
                rest = &r[close..];
            }
            for (k, label) in [("Title", "title"), ("Designer", "designer"), ("Application", "application"), ("CreationDate", "created"), ("Description", "description"), ("License", "license")] {
                if let Some(v) = meta.str_of(k) {
                    let v = plain_text(&v);
                    if !v.is_empty() {
                        out.set(label, v);
                    }
                }
            }
        }
    }

    // Bambu Studio / OrcaSlicer project settings (JSON)
    if let Some(t) = read_text(&mut a, "Metadata/project_settings.config", 32 * 1024 * 1024) {
        if let Ok(j) = Json::parse(&t) {
            let mut s = Json::obj();
            for (k, label) in [
                ("printer_settings_id", "printer"),
                ("printer_model", "printerModel"),
                ("print_settings_id", "process"),
                ("layer_height", "layerHeight"),
                ("initial_layer_print_height", "firstLayerHeight"),
                ("nozzle_diameter", "nozzle"),
                ("sparse_infill_density", "infill"),
                ("sparse_infill_pattern", "infillPattern"),
                ("wall_loops", "walls"),
                ("top_shell_layers", "topLayers"),
                ("bottom_shell_layers", "bottomLayers"),
                ("enable_support", "supports"),
                ("support_type", "supportType"),
                ("brim_type", "brim"),
                ("curr_bed_type", "bed"),
            ] {
                if let Some(v) = first_str(&j, k) {
                    if !v.is_empty() {
                        s.set(label, v);
                    }
                }
            }
            let types = str_list(&j, "filament_type");
            let names = str_list(&j, "filament_settings_id");
            let colors = str_list(&j, "filament_colour");
            let fil: Vec<Json> = (0..types.len().max(names.len()))
                .map(|i| Json::obj().with("type", types.get(i).cloned().unwrap_or_default()).with("name", names.get(i).cloned().unwrap_or_default()).with("color", colors.get(i).cloned().unwrap_or_default()))
                .collect();
            if !fil.is_empty() {
                s.set("filaments", Json::Arr(fil));
            }
            out.set("settings", s);
            out.set("source", "Bambu Studio / OrcaSlicer project");
        }
    }
    // PrusaSlicer / SuperSlicer (ini)
    for cfg in ["Metadata/Slic3r_PE.config", "Metadata/PrusaSlicer.config"] {
        if out.get("settings").is_some() {
            break;
        }
        if let Some(t) = read_text(&mut a, cfg, 16 * 1024 * 1024) {
            let mut kv = HashMap::new();
            for line in t.lines() {
                if let Some((k, v)) = line.trim_start_matches(';').split_once('=') {
                    kv.insert(k.trim().to_string(), v.trim().to_string());
                }
            }
            let mut s = Json::obj();
            for (k, label) in [
                ("printer_settings_id", "printer"),
                ("printer_model", "printerModel"),
                ("print_settings_id", "process"),
                ("layer_height", "layerHeight"),
                ("first_layer_height", "firstLayerHeight"),
                ("nozzle_diameter", "nozzle"),
                ("fill_density", "infill"),
                ("fill_pattern", "infillPattern"),
                ("perimeters", "walls"),
                ("support_material", "supports"),
            ] {
                if let Some(v) = kv.get(k).filter(|v| !v.is_empty()) {
                    s.set(label, v.split(',').next().unwrap_or("").to_string());
                }
            }
            let types: Vec<&str> = kv.get("filament_type").map(|v| v.split(';').collect()).unwrap_or_default();
            let names: Vec<&str> = kv.get("filament_settings_id").map(|v| v.split(';').collect()).unwrap_or_default();
            let colors: Vec<&str> = kv.get("filament_colour").or_else(|| kv.get("extruder_colour")).map(|v| v.split(';').collect()).unwrap_or_default();
            let fil: Vec<Json> = (0..types.len().max(names.len()))
                .map(|i| Json::obj().with("type", types.get(i).map(|x| x.trim_matches('"')).unwrap_or("")).with("name", names.get(i).map(|x| x.trim_matches('"')).unwrap_or("")).with("color", colors.get(i).unwrap_or(&"").to_string()))
                .collect();
            if !fil.is_empty() {
                s.set("filaments", Json::Arr(fil));
            }
            out.set("settings", s);
            out.set("source", "PrusaSlicer project");
        }
    }

    // sliced plates (Bambu/Orca slice_info.config)
    if let Some(t) = read_text(&mut a, "Metadata/slice_info.config", 8 * 1024 * 1024) {
        let mut plates: Vec<Json> = Vec::new();
        let mut cur: Option<Json> = None;
        let mut fils: Vec<Json> = Vec::new();
        let mut in_plate = false;
        let push = |cur: &mut Option<Json>, fils: &mut Vec<Json>, plates: &mut Vec<Json>| {
            if let Some(mut p) = cur.take() {
                p.set("filaments", Json::Arr(std::mem::take(fils)));
                plates.push(p);
            }
        };
        let mut i = 0;
        while let Some(off) = t[i..].find('<') {
            let s = i + off;
            let Some(e) = t[s..].find('>') else { break };
            let tag = &t[s + 1..s + e];
            i = s + e + 1;
            if tag.starts_with("/plate") {
                push(&mut cur, &mut fils, &mut plates);
                in_plate = false;
            } else if tag.starts_with("plate") {
                in_plate = true;
                cur = Some(Json::obj());
            } else if in_plate && tag.starts_with("metadata") {
                let (k, v) = (attr(tag, "key").unwrap_or(""), attr(tag, "value").unwrap_or(""));
                if let Some(p) = cur.as_mut() {
                    match k {
                        "index" => p.set("index", v.parse::<i64>().unwrap_or(0)),
                        "prediction" => p.set("printSeconds", v.parse::<i64>().unwrap_or(0)),
                        "weight" => p.set("weightG", v.parse::<f64>().unwrap_or(0.0)),
                        "printer_model_id" => p.set("printerModelId", v),
                        "nozzle_diameters" => p.set("nozzle", v),
                        _ => {}
                    }
                }
            } else if in_plate && tag.starts_with("filament") {
                fils.push(
                    Json::obj()
                        .with("type", attr(tag, "type").unwrap_or(""))
                        .with("color", attr(tag, "color").unwrap_or(""))
                        .with("usedG", attr(tag, "used_g").and_then(|x| x.parse::<f64>().ok()).unwrap_or(0.0))
                        .with("usedM", attr(tag, "used_m").and_then(|x| x.parse::<f64>().ok()).unwrap_or(0.0)),
                );
            }
        }
        push(&mut cur, &mut fils, &mut plates);
        if !plates.is_empty() {
            out.set("plates", Json::Arr(plates));
        }
    }

    // gcode inside the project?
    let gcode_plates = a.entries.iter().filter(|e| e.name.to_lowercase().ends_with(".gcode")).count();
    out.set("sliced", gcode_plates > 0 || out.get("plates").is_some());

    if let Some(t) = thumbnail_of(&mut a) {
        out.set("thumbnail", t);
    }
    out.set("objectFiles", a.entries.iter().filter(|e| e.name.to_lowercase().ends_with(".model")).count() as i64);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Builds a small STORE zip by hand (same writer as backups).
    fn make_3mf(dir: &Path, files: &[(&str, &str)]) -> std::path::PathBuf {
        let src = dir.join("src");
        for (n, c) in files {
            let p = src.join(n);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::File::create(&p).unwrap().write_all(c.as_bytes()).unwrap();
        }
        let out = dir.join("plate.3mf");
        crate::zip::zip_dir(&src, &out).unwrap();
        out
    }

    #[test]
    fn bambu_style_3mf_mesh_components_and_info() {
        let t = std::env::temp_dir().join(format!("pv-3mf-{}", &crate::util::new_id()[..8]));
        let root = r#"<?xml version="1.0"?><model unit="millimeter" xmlns:p="http://schemas.microsoft.com/3dmanufacturing/production/2015/06">
 <metadata name="Title">Skeleton Glock</metadata><metadata name="Application">BambuStudio-01.09</metadata>
 <resources><object id="2" type="model"><components><component p:path="/3D/Objects/object_1.model" objectid="1" transform="1 0 0 0 1 0 0 0 1 0 0 0"/></components></object></resources>
 <build><item objectid="2" transform="1 0 0 0 1 0 0 0 1 100 100 5" printable="1"/></build></model>"#;
        let obj = r#"<model><resources><object id="1" type="model"><mesh><vertices>
 <vertex x="0" y="0" z="0"/><vertex x="10" y="0" z="0"/><vertex x="0" y="10" z="0"/><vertex x="0" y="0" z="10"/>
 </vertices><triangles><triangle v1="0" v2="1" v3="2"/><triangle v1="0" v2="1" v3="3"/></triangles></mesh></object></resources></model>"#;
        let settings = r##"{"printer_settings_id":"Bambu Lab X1 Carbon 0.4 nozzle","layer_height":"0.2","nozzle_diameter":["0.4"],"sparse_infill_density":"15%","wall_loops":"2","enable_support":"0","filament_type":["PLA","PETG"],"filament_settings_id":["Bambu PLA Basic","Generic PETG"],"filament_colour":["#FFFFFF","#000000"]}"##;
        let slice = r##"<?xml version="1.0"?><config><plate><metadata key="index" value="1"/><metadata key="prediction" value="5025"/><metadata key="weight" value="21.37"/><filament id="1" type="PLA" color="#FFFFFF" used_m="7.1" used_g="21.37"/></plate></config>"##;
        let p = make_3mf(&t, &[("3D/3dmodel.model", root), ("3D/Objects/object_1.model", obj), ("Metadata/project_settings.config", settings), ("Metadata/slice_info.config", slice), ("Metadata/plate_1.png", "\u{89}PNG fake")]);
        let m = read_3mf(&p).unwrap();
        assert_eq!(m.count(), 2);
        assert_eq!(&m.tris[0..3], &[100.0, 100.0, 5.0], "build transform applied through the component");
        let info = threemf_info(&p).unwrap();
        assert_eq!(info.str_of("title").unwrap(), "Skeleton Glock");
        let s = info.get("settings").unwrap();
        assert_eq!(s.str_of("printer").unwrap(), "Bambu Lab X1 Carbon 0.4 nozzle");
        assert_eq!(s.str_of("nozzle").unwrap(), "0.4");
        assert_eq!(s.get("filaments").unwrap().as_arr().unwrap()[1].str_of("type").unwrap(), "PETG");
        let plate = &info.get("plates").unwrap().as_arr().unwrap()[0];
        assert_eq!(plate.get("printSeconds").unwrap().as_i64(), Some(5025));
        assert!(info.str_of("thumbnail").unwrap().starts_with("data:image/png;base64,"));
        assert_eq!(info.get("sliced").unwrap().as_bool(), Some(true));
        let pm = preview_mesh(&p, "3mf").unwrap();
        assert_eq!(pm.get("triangles").unwrap().as_i64(), Some(2));
        let _ = std::fs::remove_dir_all(&t);
    }

    #[test]
    fn stl_and_obj() {
        let mut bin = vec![0u8; 80];
        bin.extend_from_slice(&1u32.to_le_bytes());
        bin.extend_from_slice(&[0u8; 12]);
        for v in [0f32, 0., 0., 1., 0., 0., 0., 1., 0.] {
            bin.extend_from_slice(&v.to_le_bytes());
        }
        bin.extend_from_slice(&[0, 0]);
        assert_eq!(read_stl(&bin).unwrap().count(), 1);
        let ascii = b"solid x\nfacet normal 0 0 1\nouter loop\nvertex 0 0 0\nvertex 1 0 0\nvertex 0 1 0\nendloop\nendfacet\nendsolid";
        assert_eq!(read_stl(ascii).unwrap().count(), 1);
        let obj = b"v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nf 1 2 3 4\nf -4/1 -3/2 -2/3\n";
        assert_eq!(read_obj(obj).unwrap().count(), 3);
    }
}

#[cfg(test)]
mod real_file {
    /// Run with PV_3MF=<path to a real .3mf> cargo test real_3mf -- --ignored --nocapture
    #[test]
    #[ignore]
    fn real_3mf() {
        let p = std::path::PathBuf::from(std::env::var("PV_3MF").unwrap());
        let m = super::read_3mf(&p).unwrap();
        println!("triangles {}", m.tris.len() / 9);
        let mut info = super::threemf_info(&p).unwrap();
        if let Some(t) = info.str_of("thumbnail") {
            info.set("thumbnail", format!("{} bytes", t.len()));
        }
        println!("{}", info.pretty());
    }
}

//! Per-category file profiles: which file types matter for a kind of project,
//! grouped by the role they play. Drives the Files view's default filter, the
//! import picker's type filter, and which binary files get versioned.

use crate::json::Json;

pub struct Group {
    pub id: &'static str,
    pub label: &'static str,
    pub hint: &'static str,
    pub exts: &'static [&'static str],
}

const IMAGES: &[&str] = &["png", "jpg", "jpeg", "webp", "gif", "bmp", "svg"];

pub fn groups(category: &str) -> Vec<Group> {
    match category {
        "3d-printing" => vec![
            Group { id: "print", label: "Print projects", hint: "Sliced / plate files ready to print", exts: &["3mf", "gcode", "bgcode", "gco"] },
            Group { id: "mesh", label: "Meshes", hint: "Exported models", exts: &["stl", "obj", "ply", "amf"] },
            Group { id: "source", label: "Source models", hint: "Editable CAD / Blender files", exts: &["blend", "step", "stp", "f3d", "fcstd", "scad", "iges", "igs", "skp"] },
            Group { id: "images", label: "Photos & previews", hint: "Reference images and print photos", exts: IMAGES },
        ],
        "software" => vec![
            Group {
                id: "code",
                label: "Source code",
                hint: "Scripts and program code",
                exts: &[
                    "rs", "ts", "tsx", "js", "jsx", "mjs", "py", "cs", "c", "h", "cpp", "hpp", "cc", "go", "java", "kt", "lua", "gsc", "csc", "sh",
                    "bat", "ps1", "cmd", "rb", "php", "swift", "sql", "css", "scss", "html", "vue", "svelte",
                ],
            },
            Group { id: "config", label: "Config & data", hint: "Settings, manifests, data files", exts: &["json", "yaml", "yml", "toml", "xml", "ini", "cfg", "conf", "env", "csv"] },
            Group { id: "docs", label: "Docs", hint: "Readmes and documentation", exts: &["md", "markdown", "txt", "pdf", "rst"] },
            Group { id: "assets", label: "Assets & archives", hint: "Packaged mod files and resources", exts: &["iwd", "ff", "zip", "7z", "rar", "png", "jpg", "dds", "tga"] },
        ],
        "game" => vec![
            Group {
                id: "scripts",
                label: "Scripts & code",
                hint: "Gameplay and mod scripts, shaders",
                exts: &[
                    "cs", "gsc", "csc", "gsh", "menu", "inc", "lua", "py", "js", "ts", "gd", "cpp", "c", "h", "hpp", "shader", "hlsl", "glsl", "compute", "asmdef", "nut", "as",
                    "sp", "qc", "uc",
                ],
            },
            Group { id: "scenes", label: "Scenes & prefabs", hint: "Level and object setup", exts: &["unity", "prefab", "asset", "controller", "anim", "overridecontroller", "tscn", "umap"] },
            Group {
                id: "art",
                label: "Models & textures",
                hint: "3D models, textures, materials",
                exts: &["fbx", "obj", "blend", "gltf", "glb", "png", "tga", "psd", "jpg", "jpeg", "dds", "iwi", "exr", "mat", "xmodel_export", "xanim_export"],
            },
            Group { id: "audio", label: "Audio", hint: "Sounds and music", exts: &["wav", "mp3", "ogg", "flac", "aiff"] },
            Group { id: "config", label: "Config & data", hint: "Settings, string tables, data", exts: &["json", "xml", "yaml", "yml", "csv", "cfg", "ini", "toml", "str", "arena", "txt", "md"] },
            Group { id: "packages", label: "Packages & archives", hint: "Packed mod files and archives", exts: &["iwd", "ff", "pak", "pk3", "vpk", "zip", "7z", "rar", "unitypackage"] },
        ],
        "electronics" => vec![
            Group { id: "design", label: "Schematics & PCB", hint: "KiCad / Eagle / Fritzing files", exts: &["kicad_pcb", "kicad_sch", "kicad_pro", "sch", "brd", "fzz"] },
            Group { id: "fab", label: "Fabrication", hint: "Gerbers, drill files, BOM", exts: &["gbr", "drl", "zip", "csv", "pos"] },
            Group { id: "firmware", label: "Firmware", hint: "Microcontroller code", exts: &["ino", "c", "h", "cpp", "hpp", "py", "hex", "uf2", "bin"] },
            Group { id: "docs", label: "Datasheets & docs", hint: "PDFs and notes", exts: &["pdf", "md", "txt"] },
            Group { id: "enclosure", label: "Enclosure models", hint: "3D-printable parts", exts: &["stl", "3mf", "step", "stp", "f3d"] },
        ],
        "creative" => vec![
            Group { id: "images", label: "Images", hint: "", exts: &["png", "jpg", "jpeg", "webp", "psd", "kra", "xcf", "svg", "tif", "tiff"] },
            Group { id: "video", label: "Video", hint: "", exts: &["mp4", "mov", "mkv", "prproj", "aep"] },
            Group { id: "audio", label: "Audio", hint: "", exts: &["wav", "mp3", "flac", "als", "flp"] },
            Group { id: "docs", label: "Writing", hint: "", exts: &["md", "txt", "docx", "pdf"] },
        ],
        "video" => vec![
            Group { id: "project", label: "Edit projects", hint: "Vegas, Premiere, Resolve, Final Cut, Kdenlive projects", exts: &["veg", "vf", "prproj", "drp", "fcpxml", "kdenlive", "mlt", "wfp"] },
            Group { id: "exports", label: "Exports & footage", hint: "Rendered videos and source clips", exts: &["mp4", "mov", "mkv", "webm", "avi", "mxf", "m4v"] },
            Group { id: "audio", label: "Audio & music", hint: "Music, voice-over, sound effects", exts: &["wav", "mp3", "flac", "aiff", "m4a", "ogg"] },
            Group { id: "graphics", label: "Graphics & thumbnails", hint: "Overlays, titles, thumbnails", exts: &["png", "jpg", "jpeg", "psd", "webp", "svg"] },
            Group { id: "docs", label: "Scripts & notes", hint: "Scripts, shot lists, subtitles", exts: &["txt", "md", "docx", "pdf", "srt", "vtt"] },
        ],
        "motion" => vec![
            Group { id: "project", label: "Project files", hint: "After Effects, Cinema 4D, Blender, Fusion", exts: &["aep", "aepx", "c4d", "blend", "comp", "mogrt", "hip", "hipnc"] },
            Group { id: "renders", label: "Renders", hint: "Rendered videos and image sequences", exts: &["mp4", "mov", "webm", "gif", "exr", "png"] },
            Group { id: "assets", label: "Assets", hint: "Footage, images, models, fonts", exts: &["psd", "ai", "svg", "jpg", "jpeg", "png", "fbx", "obj", "abc", "otf", "ttf"] },
            Group { id: "audio", label: "Audio", hint: "Music and sound", exts: &["wav", "mp3", "flac", "aiff"] },
            Group { id: "scripts", label: "Expressions & scripts", hint: "Expression and automation scripts", exts: &["jsx", "jsxbin", "js", "py", "txt"] },
        ],
        "design" => vec![
            Group { id: "source", label: "Design files", hint: "Photoshop, Illustrator, Affinity, Krita, GIMP, Figma exports", exts: &["psd", "psb", "ai", "afdesign", "afphoto", "afpub", "kra", "xcf", "clip", "indd", "sketch", "fig", "xd", "procreate"] },
            Group { id: "exports", label: "Exports", hint: "Final images and PDFs", exts: &["png", "jpg", "jpeg", "webp", "svg", "pdf", "gif", "tif", "tiff"] },
            Group { id: "fonts", label: "Fonts", hint: "Fonts the design uses", exts: &["otf", "ttf", "woff", "woff2"] },
            Group { id: "docs", label: "Briefs & notes", hint: "", exts: &["md", "txt", "docx", "pdf"] },
        ],
        "3d-art" => vec![
            Group { id: "scenes", label: "Scene files", hint: "Blender, Cinema 4D, Maya, 3ds Max, Houdini, ZBrush", exts: &["blend", "c4d", "ma", "mb", "max", "hip", "hipnc", "ztl", "zpr", "usd", "usda"] },
            Group { id: "textures", label: "Textures & materials", hint: "Substance projects, texture maps", exts: &["spp", "sbs", "sbsar", "png", "tga", "exr", "hdr", "jpg", "jpeg", "tif", "dds"] },
            Group { id: "exports", label: "Exported models", hint: "For games, printing or sharing", exts: &["fbx", "obj", "gltf", "glb", "abc", "stl", "usdz", "dae"] },
            Group { id: "renders", label: "Renders", hint: "Stills and animations", exts: &["png", "jpg", "jpeg", "exr", "mp4", "mov"] },
        ],
        "cad" => vec![
            Group { id: "parts", label: "Parts & assemblies", hint: "SolidWorks, Fusion, Inventor, FreeCAD, Onshape exports", exts: &["sldprt", "sldasm", "f3d", "f3z", "ipt", "iam", "fcstd", "prt", "asm", "catpart", "catproduct", "3dm", "scad"] },
            Group { id: "drawings", label: "Drawings", hint: "2D drawings and DXF/DWG", exts: &["slddrw", "idw", "dwg", "dxf", "pdf"] },
            Group { id: "exchange", label: "Exchange formats", hint: "STEP, IGES, STL, Parasolid", exts: &["step", "stp", "iges", "igs", "x_t", "x_b", "stl", "3mf", "obj"] },
            Group { id: "cam", label: "CAM & manufacturing", hint: "Toolpaths, G-code, laser files", exts: &["nc", "tap", "gcode", "ngc", "lbrn", "lbrn2", "svg"] },
            Group { id: "docs", label: "Specs & BOM", hint: "", exts: &["pdf", "xlsx", "csv", "md", "txt"] },
        ],
        "addon" => vec![
            Group { id: "code", label: "Code", hint: "Lua and XML", exts: &["lua", "xml", "toc"] },
            Group { id: "media", label: "Textures & sounds", hint: "", exts: &["blp", "tga", "png", "ogg", "mp3"] },
            Group { id: "docs", label: "Docs & packaging", hint: "Readme, changelog, packager config", exts: &["md", "txt", "yml", "yaml", "json"] },
        ],
        "documents" => vec![
            Group { id: "docs", label: "Documents", hint: "", exts: &["pdf", "docx", "doc", "odt", "md", "txt", "rtf"] },
            Group { id: "sheets", label: "Spreadsheets", hint: "", exts: &["xlsx", "xls", "csv", "ods"] },
            Group { id: "images", label: "Scans & images", hint: "", exts: IMAGES },
        ],
        _ => vec![],
    }
}

/// Extensions whose versions are kept by default (binary "key" files).
pub fn key_version_exts(category: &str) -> &'static [&'static str] {
    match category {
        "3d-printing" => &["3mf", "stl", "obj", "blend", "step", "stp", "f3d", "fcstd", "scad"],
        "electronics" => &["kicad_pcb", "kicad_sch", "kicad_pro", "sch", "brd"],
        "video" => &["veg", "vf", "prproj", "drp", "fcpxml", "kdenlive", "wfp"],
        "motion" => &["aep", "aepx", "c4d", "comp", "mogrt"],
        "design" => &["psd", "psb", "ai", "afdesign", "afphoto", "afpub", "kra", "xcf", "clip", "indd", "sketch", "fig", "xd"],
        "3d-art" => &["blend", "c4d", "ma", "mb", "max", "hip", "hipnc", "ztl", "zpr", "spp", "sbs"],
        "cad" => &["sldprt", "sldasm", "slddrw", "f3d", "f3z", "ipt", "iam", "idw", "fcstd", "3dm", "scad", "dwg", "dxf", "step", "stp"],
        _ => &[],
    }
}

/// Best guess at a folder's project type from what's in it (first ~5000
/// files). Returns the category and a short reason, or None when unclear.
/// (category, reason, subtype) — the subtype is set where it's unmistakable
/// ("wow" for a .toc addon, "iw4x" for Call of Duty GSC scripts).
pub fn guess(root: &std::path::Path) -> Option<(String, String, Option<String>)> {
    let w = crate::files::walk(root, &crate::files::Ignore::parse("Library/\nnode_modules/\ntarget/\nTemp/"));
    let mut score: std::collections::HashMap<&'static str, f64> = std::collections::HashMap::new();
    let mut tops: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut exts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for rel in w.files.keys().take(5000) {
        let lower = rel.to_lowercase();
        tops.insert(lower.split('/').next().unwrap_or("").to_string());
        *exts.entry(crate::util::extension_of(&lower)).or_default() += 1;
    }
    let has = |e: &str| exts.get(e).copied().unwrap_or(0);
    // unmistakable markers first
    if has("toc") > 0 && has("lua") > 0 {
        return Some(("addon".into(), "has a .toc file and Lua code".into(), Some("wow".into())));
    }
    if tops.contains("assets") && tops.contains("projectsettings") {
        return Some(("game".into(), "looks like a Unity project".into(), None));
    }
    if has("gsc") + has("csc") > 0 {
        // Call of Duty script mods (IW4X uses .gsc; .csc is client-side in later games)
        let sub = if has("csc") == 0 { Some("iw4x".to_string()) } else { Some("other".to_string()) };
        return Some(("addon".into(), "has GSC scripts".into(), sub));
    }
    for c in crate::projects::CATEGORIES {
        let keys = key_version_exts(c);
        for g in groups(c) {
            for e in g.exts {
                let n = has(e) as f64;
                if n > 0.0 {
                    // a category's own project files count far more than shared formats like .png
                    *score.entry(c).or_default() += if keys.contains(e) { 20.0 + n } else { n.sqrt() * 0.5 };
                }
            }
        }
    }
    if ["cargo.toml", "package.json", "pyproject.toml", "go.mod"].iter().any(|f| w.files.keys().any(|k| k.eq_ignore_ascii_case(f))) {
        *score.entry("software").or_default() += 25.0;
    }
    if has("3mf") + has("gcode") + has("bgcode") > 0 {
        *score.entry("3d-printing").or_default() += 25.0;
    }
    // highest score wins; ties go to the category listed first (deterministic)
    let mut best: Option<(&str, f64)> = None;
    for c in crate::projects::CATEGORIES.iter().filter(|c| !matches!(**c, "creative" | "general")) {
        let sc = score.get(c).copied().unwrap_or(0.0);
        if best.is_none_or(|b| sc > b.1) {
            best = Some((c, sc));
        }
    }
    let (best, top) = best?;
    if top < 3.0 {
        return None;
    }
    let reason = match best {
        "video" => "has video editing projects",
        "motion" => "has After Effects / Cinema 4D files",
        "design" => "has design files",
        "3d-art" => "has 3D scene files",
        "cad" => "has CAD parts or drawings",
        "3d-printing" => "has print files",
        "electronics" => "has schematics or PCB files",
        "software" => "has source code",
        "game" => "has game engine files",
        "documents" => "is mostly documents",
        _ => "matches its file types",
    };
    Some((best.to_string(), reason.to_string(), None))
}

/// Everything the new-project dialog shows about a project type.
pub fn info_json(category: &str) -> Json {
    Json::obj()
        .with("groups", groups_json(category))
        .with("ignores", crate::projects::default_ignores(category))
        .with("keyExts", Json::Arr(key_version_exts(category).iter().map(|e| Json::from(*e)).collect()))
}

pub fn groups_json(category: &str) -> Json {
    Json::Arr(
        groups(category)
            .iter()
            .map(|g| {
                Json::obj()
                    .with("id", g.id)
                    .with("label", g.label)
                    .with("hint", g.hint)
                    .with("exts", Json::Arr(g.exts.iter().map(|e| Json::from(*e)).collect()))
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::files::{classify, Ignore};

    #[test]
    fn every_category_is_described() {
        for c in crate::projects::CATEGORIES {
            let g = groups(c);
            if !matches!(*c, "general") {
                assert!(!g.is_empty(), "{c} has file groups");
            }
            for e in key_version_exts(c) {
                assert!(g.iter().any(|g| g.exts.contains(e)), "{c}: key ext .{e} is in a group");
            }
        }
        // project files of the new types are recognised, and their caches skipped
        assert_eq!(classify("veg"), "video");
        assert_eq!(classify("c4d"), "model");
        assert_eq!(classify("afdesign"), "image");
        assert_eq!(classify("sldprt"), "model");
        let ig = Ignore::parse(crate::projects::default_ignores("3d-art"));
        assert!(ig.skip_file("robot.blend1", "scenes/robot.blend1"));
        assert!(!ig.skip_file("robot.blend", "scenes/robot.blend"));
        let ig = Ignore::parse(crate::projects::default_ignores("video"));
        assert!(ig.skip_dir("Adobe Premiere Pro Auto-Save", "Adobe Premiere Pro Auto-Save"));
        assert!(ig.skip_file("clip.mp4.sfk", "clip.mp4.sfk"));
    }

    #[test]
    fn guesses_project_type() {
        let t = crate::testutil::TempDir::new("guess");
        let mk = |dir: &str, files: &[&str]| {
            let d = t.path().join(dir);
            for f in files {
                let p = d.join(f);
                std::fs::create_dir_all(p.parent().unwrap()).unwrap();
                std::fs::write(p, "x").unwrap();
            }
            d
        };
        let g = |d: std::path::PathBuf| guess(&d).map(|x| x.0);
        assert_eq!(g(mk("wow", &["WhatsLeft.toc", "Core.lua", "Media/icon.tga"])), Some("addon".into()));
        assert_eq!(guess(&mk("wow2", &["A.toc", "a.lua"])).unwrap().2.as_deref(), Some("wow"));
        assert_eq!(guess(&mk("omen", &["scripts/mp/menu.gsc", "mod.ff"])).map(|x| (x.0, x.2)), Some(("addon".into(), Some("iw4x".into()))));
        assert_eq!(g(mk("vid", &["montage.veg", "clips/a.mp4", "clips/b.mp4", "thumb.png"])), Some("video".into()));
        assert_eq!(g(mk("ae", &["intro.aep", "render/intro.mp4"])), Some("motion".into()));
        assert_eq!(g(mk("print", &["frame.3mf", "frame.stl", "photo.jpg"])), Some("3d-printing".into()));
        assert_eq!(g(mk("cad", &["bracket.sldprt", "bracket.step", "bracket.slddrw"])), Some("cad".into()));
        assert_eq!(g(mk("art", &["robot.blend", "tex/a.png", "tex/b.png"])), Some("3d-art".into()));
        assert_eq!(g(mk("code", &["Cargo.toml", "src/main.rs"])), Some("software".into()));
        assert_eq!(g(mk("misc", &["a.png"])), None);
    }
}

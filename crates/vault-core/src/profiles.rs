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
            Group { id: "project", label: "Edit projects", hint: "Vegas, Premiere, Resolve, Final Cut, Kdenlive projects", exts: &["veg", "vf", "prproj", "drp", "fcpxml", "kdenlive", "mlt", "wfp", "aep", "mogrt"] },
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
        "video" => &["veg", "vf", "prproj", "drp", "fcpxml", "kdenlive", "wfp", "aep"],
        "motion" => &["aep", "aepx", "c4d", "blend", "comp", "mogrt", "hip", "hipnc"],
        "design" => &["psd", "psb", "ai", "afdesign", "afphoto", "afpub", "kra", "xcf", "clip", "indd", "sketch", "fig", "xd"],
        "3d-art" => &["blend", "c4d", "ma", "mb", "max", "hip", "hipnc", "ztl", "zpr", "spp", "sbs"],
        "cad" => &["sldprt", "sldasm", "slddrw", "f3d", "f3z", "ipt", "iam", "idw", "fcstd", "3dm", "scad", "dwg", "dxf", "step", "stp"],
        _ => &[],
    }
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
}

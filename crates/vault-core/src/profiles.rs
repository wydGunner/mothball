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
            Group { id: "scripts", label: "Scripts & shaders", hint: "Gameplay code", exts: &["cs", "shader", "hlsl", "glsl", "compute", "gd", "lua", "cpp", "h", "asmdef"] },
            Group { id: "scenes", label: "Scenes & prefabs", hint: "Level and object setup", exts: &["unity", "prefab", "asset", "controller", "anim", "overridecontroller", "tscn", "umap"] },
            Group { id: "art", label: "Models & textures", hint: "3D models, textures, materials", exts: &["fbx", "obj", "blend", "gltf", "glb", "png", "tga", "psd", "jpg", "exr", "mat"] },
            Group { id: "audio", label: "Audio", hint: "Sounds and music", exts: &["wav", "mp3", "ogg", "flac", "aiff"] },
            Group { id: "config", label: "Config & data", hint: "Settings and data tables", exts: &["json", "xml", "yaml", "yml", "csv", "txt", "md"] },
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
        _ => &[],
    }
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

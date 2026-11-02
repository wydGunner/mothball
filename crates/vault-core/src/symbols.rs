//! Function index for GSC / CSC scripts (Call of Duty modding: IW4X, Plutonium,
//! BO3…). Finds top-level function definitions and #include / #using lines,
//! kept per file and refreshed only when a file's content hash changes.
//! Used for go-to-definition, the per-file function list and symbol search.

use crate::db::DbResult;
use crate::json::Json;
use crate::util::safe_join;
use crate::{p, projects, Vault};

pub const SCRIPT_EXTS: &[&str] = &["gsc", "csc", "gsh"];
const MAX_SCRIPT: u64 = 8 * 1024 * 1024;

#[derive(Debug, PartialEq)]
pub struct Def {
    pub name: String,
    pub kind: &'static str, // function | include
    pub line: i64,
    pub signature: String,
}

/// Blanks out comments and string contents (keeping newlines so line
/// numbers stay right).
fn strip(src: &str) -> String {
    let b: Vec<char> = src.chars().collect();
    let mut out = String::with_capacity(src.len());
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c == '/' && b.get(i + 1) == Some(&'/') {
            while i < b.len() && b[i] != '\n' {
                out.push(' ');
                i += 1;
            }
        } else if c == '/' && b.get(i + 1) == Some(&'*') {
            out.push_str("  ");
            i += 2;
            while i < b.len() && !(b[i] == '*' && b.get(i + 1) == Some(&'/')) {
                out.push(if b[i] == '\n' { '\n' } else { ' ' });
                i += 1;
            }
            out.push_str("  ");
            i += 2;
        } else if c == '"' {
            out.push('"');
            i += 1;
            while i < b.len() && b[i] != '"' && b[i] != '\n' {
                if b[i] == '\\' {
                    out.push(' ');
                    i += 1;
                }
                if i < b.len() {
                    out.push(if b[i] == '\n' { '\n' } else { ' ' });
                    i += 1;
                }
            }
            if i < b.len() && b[i] == '"' {
                out.push('"');
                i += 1;
            }
        } else {
            out.push(c);
            i += 1;
        }
    }
    out
}

fn is_ident(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

const NOT_FUNCS: &[&str] = &["if", "while", "for", "foreach", "switch", "return", "wait", "thread", "else"];

/// Top-level function definitions and include lines in a script.
pub fn parse(src: &str) -> Vec<Def> {
    let s = strip(src);
    let mut out = Vec::new();
    // includes
    for (n, line) in s.lines().enumerate() {
        let t = line.trim();
        for kw in ["#include", "#using", "#insert"] {
            if let Some(rest) = t.strip_prefix(kw) {
                let path = rest.trim().trim_end_matches(';').trim().to_string();
                if !path.is_empty() && !(kw == "#using" && path.starts_with("_animtree")) {
                    out.push(Def { name: path, kind: "include", line: n as i64 + 1, signature: t.trim_end_matches(';').to_string() });
                }
            }
        }
    }
    // functions: at brace depth 0, IDENT ( ... ) followed by {
    let c: Vec<char> = s.chars().collect();
    let mut depth = 0i32;
    let mut line = 1i64;
    let mut i = 0;
    while i < c.len() {
        let ch = c[i];
        if ch == '\n' {
            line += 1;
        }
        if ch == '{' {
            depth += 1;
        } else if ch == '}' {
            depth = (depth - 1).max(0);
        } else if depth == 0 && is_ident(ch) && (i == 0 || !is_ident(c[i - 1])) {
            let start = i;
            while i < c.len() && is_ident(c[i]) {
                i += 1;
            }
            let name: String = c[start..i].iter().collect();
            let def_line = line;
            let mut j = i;
            while j < c.len() && c[j].is_whitespace() {
                j += 1;
            }
            if j < c.len() && c[j] == '(' && !NOT_FUNCS.contains(&name.as_str()) && !name.chars().next().unwrap().is_ascii_digit() {
                // find the matching ')'
                let mut k = j;
                let mut d = 0;
                while k < c.len() {
                    if c[k] == '(' {
                        d += 1;
                    } else if c[k] == ')' {
                        d -= 1;
                        if d == 0 {
                            break;
                        }
                    }
                    k += 1;
                }
                let mut m = k + 1;
                while m < c.len() && c[m].is_whitespace() {
                    m += 1;
                }
                if k < c.len() && m < c.len() && c[m] == '{' {
                    let args: String = c[j + 1..k].iter().collect::<String>().split_whitespace().collect::<Vec<_>>().join(" ");
                    out.push(Def { name: name.clone(), kind: "function", line: def_line, signature: format!("{name}({args})") });
                    // count newlines we skipped so `line` stays right
                    for &x in &c[i..m] {
                        if x == '\n' {
                            line += 1;
                        }
                    }
                    i = m;
                    continue;
                }
            }
            continue;
        }
        i += 1;
    }
    out
}

/// Re-parses scripts whose content changed since they were last indexed.
pub fn refresh(v: &Vault, project_id: &str) -> DbResult<i64> {
    let root = projects::root_of(v, project_id)?;
    let rows = v.db.query(
        &format!(
            "SELECT f.id, f.rel_path, f.sha256, f.size, s.sha256 AS done FROM files f LEFT JOIN symbol_files s ON s.file_id=f.id
             WHERE f.project_id=? AND f.state='present' AND f.ext IN ({})",
            SCRIPT_EXTS.iter().map(|e| format!("'{e}'")).collect::<Vec<_>>().join(",")
        ),
        p![project_id],
    )?;
    let mut n = 0;
    for r in rows {
        if r.os("done") == r.os("sha256") && r.os("done").is_some() {
            continue;
        }
        let defs = if r.i("size") as u64 <= MAX_SCRIPT {
            match safe_join(&root, &r.s("rel_path")).and_then(|p| std::fs::read(p).map_err(|e| e.to_string())) {
                Ok(b) => parse(&String::from_utf8_lossy(&b)),
                Err(_) => continue,
            }
        } else {
            vec![]
        };
        let fid = r.s("id");
        v.db.tx(|db| {
            db.exec("DELETE FROM symbols WHERE file_id=?", p![fid.clone()])?;
            for d in &defs {
                db.exec(
                    "INSERT INTO symbols(file_id, project_id, name, kind, line, signature) VALUES (?,?,?,?,?,?)",
                    p![fid.clone(), project_id, d.name.clone(), d.kind, d.line, d.signature.clone()],
                )?;
            }
            db.exec("INSERT INTO symbol_files(file_id, sha256) VALUES (?,?) ON CONFLICT(file_id) DO UPDATE SET sha256=excluded.sha256", p![fid.clone(), r.os("sha256")])?;
            Ok(())
        })?;
        n += 1;
    }
    // drop entries for files that are gone or no longer scripts
    v.db.exec(
        "DELETE FROM symbols WHERE project_id=? AND file_id NOT IN (SELECT id FROM files WHERE project_id=? AND state='present')",
        p![project_id, project_id],
    )?;
    Ok(n)
}

fn def_json(r: &crate::db::Row) -> Json {
    Json::obj()
        .with("name", r.s("name"))
        .with("kind", r.s("kind"))
        .with("line", r.i("line"))
        .with("signature", r.s("signature"))
        .with("fileId", r.s("file_id"))
        .with("path", r.s("rel_path"))
}

/// Where `name` is defined. `qualifier` is the script path before `::`
/// (e.g. `maps\mp\_utility`); matches in that file come first.
pub fn find(v: &Vault, project_id: &str, name: &str, qualifier: &str) -> DbResult<Json> {
    refresh(v, project_id)?;
    let rows = v.db.query(
        "SELECT s.*, f.rel_path FROM symbols s JOIN files f ON f.id=s.file_id WHERE s.project_id=? AND s.kind='function' AND lower(s.name)=lower(?) ORDER BY f.rel_path",
        p![project_id, name],
    )?;
    let q = qualifier.replace('\\', "/").to_lowercase();
    let mut defs: Vec<Json> = rows.iter().map(def_json).collect();
    if !q.is_empty() {
        defs.sort_by_key(|d| {
            let path = d.str_of("path").unwrap_or_default().to_lowercase();
            let stem = path.rsplit_once('.').map(|x| x.0).unwrap_or(&path).to_string();
            if stem == q || stem.ends_with(&format!("/{q}")) {
                0
            } else {
                1
            }
        });
    }
    Ok(Json::Arr(defs))
}

/// Functions and includes in one file, in order.
pub fn in_file(v: &Vault, file_id: &str) -> DbResult<Json> {
    let pid = v.db.one("SELECT project_id FROM files WHERE id=?", p![file_id])?.ok_or("File not found")?.s("project_id");
    refresh(v, &pid)?;
    let rows = v.db.query("SELECT s.*, f.rel_path FROM symbols s JOIN files f ON f.id=s.file_id WHERE s.file_id=? ORDER BY s.line", p![file_id])?;
    let mut out = Vec::new();
    for r in &rows {
        let mut j = def_json(r);
        if r.s("kind") == "include" {
            // resolve the include to a file in the project when possible
            let target = r.s("name").replace('\\', "/").to_lowercase();
            if let Some(f) = v.db.one(
                "SELECT id, rel_path FROM files WHERE project_id=? AND state='present' AND (lower(rel_path) IN (?,?,?) OR lower(rel_path) LIKE ? OR lower(rel_path) LIKE ? OR lower(rel_path) LIKE ?) ORDER BY length(rel_path) LIMIT 1",
                p![pid.clone(), format!("{target}.gsc"), format!("{target}.csc"), format!("{target}.gsh"), format!("%/{target}.gsc"), format!("%/{target}.csc"), format!("%/{target}.gsh")],
            )? {
                j.set("targetId", f.s("id"));
                j.set("targetPath", f.s("rel_path"));
            }
        }
        out.push(j);
    }
    Ok(Json::Arr(out))
}

/// Function names starting with (or containing) `query`, for search.
pub fn search(v: &Vault, project_id: Option<&str>, query: &str, limit: i64) -> DbResult<Json> {
    if let Some(pid) = project_id {
        refresh(v, pid)?;
    } else {
        for r in v.db.query("SELECT DISTINCT project_id FROM files WHERE ext IN ('gsc','csc','gsh')", p![])? {
            let _ = refresh(v, &r.s("project_id"));
        }
    }
    let q = query.trim().trim_start_matches('@').to_lowercase();
    if q.len() < 2 {
        return Ok(Json::Arr(vec![]));
    }
    let rows = v.db.query(
        "SELECT s.*, f.rel_path, p.name AS project_name FROM symbols s JOIN files f ON f.id=s.file_id JOIN projects p ON p.id=s.project_id
         WHERE s.kind='function' AND (? IS NULL OR s.project_id=?) AND instr(lower(s.name), ?)>0
         ORDER BY instr(lower(s.name), ?) , length(s.name), s.name LIMIT ?",
        p![project_id, project_id, q.clone(), q, limit],
    )?;
    Ok(Json::Arr(rows.iter().map(|r| def_json(r).with("projectId", r.s("project_id")).with("projectName", r.s("project_name"))).collect()))
}

/// Lines in the project's scripts that call or reference `name`.
pub fn references(v: &Vault, project_id: &str, name: &str, limit: usize) -> DbResult<Json> {
    let root = projects::root_of(v, project_id)?;
    let rows = v.db.query(
        &format!("SELECT id, rel_path, size FROM files WHERE project_id=? AND state='present' AND ext IN ({}) ORDER BY rel_path", SCRIPT_EXTS.iter().map(|e| format!("'{e}'")).collect::<Vec<_>>().join(",")),
        p![project_id],
    )?;
    let lname = name.to_lowercase();
    refresh(v, project_id)?;
    let defs: std::collections::HashSet<(String, i64)> = v
        .db
        .query("SELECT file_id, line FROM symbols WHERE project_id=? AND kind='function' AND lower(name)=?", p![project_id, lname.clone()])?
        .iter()
        .map(|r| (r.s("file_id"), r.i("line")))
        .collect();
    let mut out = Vec::new();
    'files: for r in rows {
        if r.i("size") as u64 > MAX_SCRIPT {
            continue;
        }
        let Ok(text) = safe_join(&root, &r.s("rel_path")).and_then(|p| std::fs::read(p).map_err(|e| e.to_string())) else { continue };
        let raw = String::from_utf8_lossy(&text).to_string();
        let code = strip(&raw);
        for (n, (line, orig)) in code.lines().zip(raw.lines()).enumerate() {
            if defs.contains(&(r.s("id"), n as i64 + 1)) {
                continue; // the definition itself
            }
            let low = line.to_lowercase();
            let mut from = 0;
            while let Some(pos) = low[from..].find(&lname) {
                let at = from + pos;
                let before = low[..at].chars().last();
                let after = low[at + lname.len()..].trim_start().chars().next();
                let ident_ok = before.map(|c| !is_ident(c)).unwrap_or(true) && low[at + lname.len()..].chars().next().map(|c| !is_ident(c)).unwrap_or(true);
                let is_call = after == Some('(') || before == Some(':');
                if ident_ok && is_call {
                    out.push(Json::obj().with("fileId", r.s("id")).with("path", r.s("rel_path")).with("line", n as i64 + 1).with("text", orig.trim().chars().take(160).collect::<String>()));
                    if out.len() >= limit {
                        break 'files;
                    }
                    break;
                }
                from = at + lname.len();
            }
        }
    }
    Ok(Json::Arr(out))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_gsc_functions_and_includes() {
        let src = r#"#include maps\mp\_utility;
#include common_scripts\utility;

// init() { fake }
init()
{
    level thread onPlayerConnect();
    if ( isDefined( level.x ) ) { wait 0.05; }
    s = "not_a_func() {";
}

/* block
   comment(x) { } */
onPlayerConnect( player, extra )
{
    for(;;)
    {
        level waittill( "connected", player );
        player thread maps\mp\_utility::giveWeapon( "ak47_mp" );
    }
}
function autoexec __init__sytem__() { }
"#;
        let d = parse(src);
        let f: Vec<(&str, i64, &str)> = d.iter().filter(|x| x.kind == "function").map(|x| (x.name.as_str(), x.line, x.signature.as_str())).collect();
        assert_eq!(f, vec![("init", 5, "init()"), ("onPlayerConnect", 14, "onPlayerConnect(player, extra)"), ("__init__sytem__", 22, "__init__sytem__()")]);
        let inc: Vec<&str> = d.iter().filter(|x| x.kind == "include").map(|x| x.name.as_str()).collect();
        assert_eq!(inc, vec!["maps\\mp\\_utility", "common_scripts\\utility"]);
    }

    #[test]
    fn find_definitions_and_references_in_a_project() {
        let (t, v) = crate::testutil::vault();
        let d = t.path().join("mod");
        std::fs::create_dir_all(d.join("maps/mp")).unwrap();
        std::fs::write(d.join("maps/mp/_utility.gsc"), "giveWeapon(w)\n{\n  self giveweapon(w);\n}\n").unwrap();
        std::fs::create_dir_all(d.join("maps/mp/gametypes")).unwrap();
        std::fs::write(d.join("maps/mp/gametypes/_rank.gsc"), "#include maps\\mp\\_utility;\ninit()\n{\n  self maps\\mp\\_utility::giveWeapon(\"ak47_mp\");\n}\n").unwrap();
        let pid = crate::projects::create(&v, crate::projects::NewProject { name: "OMEN".into(), description: "".into(), category: "game".into(), status: "".into(), link_path: Some(d.clone()), module: Json::obj() }).unwrap().str_of("id").unwrap();
        crate::files::scan(&v, &pid).unwrap();
        let f = find(&v, &pid, "giveweapon", "maps\\mp\\_utility").unwrap();
        let first = &f.as_arr().unwrap()[0];
        assert_eq!(first.str_of("path").unwrap(), "maps/mp/_utility.gsc");
        assert_eq!(first.get("line").unwrap().as_i64(), Some(1));
        let rank = v.db.one("SELECT id FROM files WHERE rel_path='maps/mp/gametypes/_rank.gsc'", p![]).unwrap().unwrap().s("id");
        let syms = in_file(&v, &rank).unwrap();
        let inc = syms.as_arr().unwrap().iter().find(|x| x.str_of("kind").as_deref() == Some("include")).unwrap();
        assert_eq!(inc.str_of("targetPath").unwrap(), "maps/mp/_utility.gsc");
        assert_eq!(search(&v, Some(&pid), "givew", 10).unwrap().as_arr().unwrap().len(), 1);
        let refs = references(&v, &pid, "giveWeapon", 50).unwrap();
        // GSC names are case-insensitive: the call in _rank.gsc and `self giveweapon(w)`; never the definition line
        assert_eq!(refs.as_arr().unwrap().len(), 2, "{}", refs.pretty());
        assert!(refs.as_arr().unwrap().iter().all(|r| r.get("line").unwrap().as_i64() != Some(1) || r.str_of("path").unwrap().contains("_rank")));
        // editing a file updates the index
        std::fs::write(d.join("maps/mp/_utility.gsc"), "\n\ngiveWeapon(w, x)\n{\n}\n").unwrap();
        crate::files::scan(&v, &pid).unwrap();
        let f2 = find(&v, &pid, "giveWeapon", "").unwrap();
        assert_eq!(f2.as_arr().unwrap()[0].get("line").unwrap().as_i64(), Some(3));
    }
}

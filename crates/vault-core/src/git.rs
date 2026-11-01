//! Read-only Git integration via the `git` CLI (whatever the user has
//! installed). Arguments are passed as an argv array — never through a shell —
//! and paths always follow `--`.

use crate::db::DbResult;
use crate::json::Json;
use std::path::Path;
use std::process::Command;

fn git(root: &Path, args: &[&str]) -> Result<String, String> {
    let mut c = Command::new("git");
    c.arg("-C").arg(root).args(["-c", "core.quotepath=off", "-c", "color.ui=never"]).args(args);
    c.env("GIT_TERMINAL_PROMPT", "0").env("GIT_OPTIONAL_LOCKS", "0"); // never take index locks: don't disturb the user's own git use
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let out = c.output().map_err(|e| format!("git is not available: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

pub fn info(root: &Path) -> DbResult<Json> {
    let top = match git(root, &["rev-parse", "--show-toplevel"]) {
        Ok(t) => t.trim().to_string(),
        Err(e) => {
            let available = !e.starts_with("git is not available");
            return Ok(Json::obj().with("isRepo", false).with("gitAvailable", available).with("message", e));
        }
    };
    let branch = git(root, &["rev-parse", "--abbrev-ref", "HEAD"]).unwrap_or_default().trim().to_string();
    let head = git(root, &["log", "-1", "--format=%H%x1f%s%x1f%an%x1f%at"]).unwrap_or_default();
    let h: Vec<&str> = head.trim().split('\u{1f}').collect();
    let upstream = git(root, &["rev-list", "--left-right", "--count", "@{upstream}...HEAD"]).ok().map(|s| {
        let n: Vec<i64> = s.split_whitespace().filter_map(|x| x.parse().ok()).collect();
        Json::obj().with("behind", *n.first().unwrap_or(&0)).with("ahead", *n.get(1).unwrap_or(&0))
    });
    let remotes = git(root, &["remote", "-v"]).unwrap_or_default();
    let remote = remotes.lines().find(|l| l.ends_with("(fetch)")).map(|l| l.split_whitespace().nth(1).unwrap_or("").to_string());
    Ok(Json::obj()
        .with("isRepo", true)
        .with("gitAvailable", true)
        .with("topLevel", top)
        .with("branch", branch)
        .with("head", if h.len() == 4 { Json::obj().with("hash", h[0]).with("subject", h[1]).with("author", h[2]).with("time", h[3].parse::<i64>().unwrap_or(0) * 1000) } else { Json::Null })
        .with("upstream", upstream)
        .with("remote", remote))
}

pub fn status(root: &Path) -> DbResult<Json> {
    let out = git(root, &["status", "--porcelain=v1", "-z", "--untracked-files=all"])?;
    let mut items = Vec::new();
    let mut parts = out.split('\0').filter(|s| !s.is_empty());
    while let Some(entry) = parts.next() {
        if entry.len() < 4 {
            continue;
        }
        let (code, path) = entry.split_at(3);
        let code = code.trim_end().to_string();
        let mut j = Json::obj().with("code", code.clone()).with("path", path);
        if code.starts_with('R') || code.starts_with('C') {
            if let Some(from) = parts.next() {
                j.set("from", from);
            }
        }
        items.push(j);
    }
    Ok(Json::Arr(items))
}

pub fn log(root: &Path, path: Option<&str>, limit: i64) -> DbResult<Json> {
    let n = format!("-n{}", limit.clamp(1, 1000));
    let mut args = vec!["log", &n, "--format=%x1e%H%x1f%h%x1f%an%x1f%at%x1f%s%x1f%b", "--name-status"];
    if let Some(p) = path {
        args.extend(["--follow", "--", p]);
    }
    let out = match git(root, &args) {
        Ok(o) => o,
        Err(e) if e.contains("does not have any commits") => return Ok(Json::Arr(vec![])),
        Err(e) => return Err(e),
    };
    let mut commits = Vec::new();
    for rec in out.split('\u{1e}').filter(|r| !r.trim().is_empty()) {
        let f: Vec<&str> = rec.splitn(6, '\u{1f}').collect();
        if f.len() < 6 {
            continue;
        }
        // body ends where the name-status block begins (after a blank line)
        let (body, files_block) = match f[5].find("\n\n") {
            Some(i) => (&f[5][..i], &f[5][i + 2..]),
            None => (f[5], ""),
        };
        let files: Vec<Json> = files_block
            .lines()
            .filter_map(|l| {
                let mut it = l.split('\t');
                let st = it.next()?.trim();
                if st.is_empty() {
                    return None;
                }
                let p1 = it.next()?;
                let p2 = it.next();
                Some(Json::obj().with("status", st).with("path", p2.unwrap_or(p1)).with("from", p2.map(|_| p1)))
            })
            .collect();
        commits.push(
            Json::obj()
                .with("hash", f[0].trim())
                .with("short", f[1])
                .with("author", f[2])
                .with("time", f[3].parse::<i64>().unwrap_or(0) * 1000)
                .with("subject", f[4])
                .with("body", body.trim())
                .with("files", Json::Arr(files)),
        );
    }
    Ok(Json::Arr(commits))
}

fn valid_rev(r: &str) -> bool {
    !r.is_empty() && r.len() <= 64 && r.chars().all(|c| c.is_ascii_hexdigit()) || r == "HEAD"
}

/// Unified diff. `rev` = None: working tree vs HEAD; Some(hash): that commit.
pub fn diff(root: &Path, rev: Option<&str>, path: Option<&str>) -> DbResult<Json> {
    let mut args: Vec<String> = match rev {
        Some(r) => {
            if !valid_rev(r) {
                return Err("Invalid revision".into());
            }
            vec!["show".into(), "--format=".into(), "--no-ext-diff".into(), r.into()]
        }
        None => vec!["diff".into(), "--no-ext-diff".into(), "HEAD".into()],
    };
    if let Some(p) = path {
        args.push("--".into());
        args.push(p.into());
    }
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    let mut out = git(root, &a)?;
    let truncated = out.len() > 1_000_000;
    if truncated {
        let mut cut = 1_000_000;
        while !out.is_char_boundary(cut) {
            cut -= 1;
        }
        out.truncate(cut);
    }
    Ok(Json::obj().with("diff", out).with("truncated", truncated))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;
    #[test]
    fn git_roundtrip() {
        let t = TempDir::new("git");
        let r = t.path();
        if git(r, &["init", "-q"]).is_err() {
            return; // git not installed in this environment
        }
        assert!(info(r).unwrap().get("isRepo").unwrap().as_bool().unwrap());
        std::fs::write(r.join("a.gsc"), "one\n").unwrap();
        git(r, &["add", "."]).unwrap();
        git(r, &["-c", "user.name=T", "-c", "user.email=t@t", "commit", "-qm", "first", "-m", "body text"]).unwrap();
        std::fs::rename(r.join("a.gsc"), r.join("b.gsc")).unwrap();
        std::fs::write(r.join("b.gsc"), "one\n").unwrap();
        git(r, &["add", "-A"]).unwrap();
        git(r, &["-c", "user.name=T", "-c", "user.email=t@t", "commit", "-qm", "rename"]).unwrap();
        std::fs::write(r.join("b.gsc"), "one\ntwo\n").unwrap();
        let l = log(r, Some("b.gsc"), 10).unwrap();
        assert_eq!(l.as_arr().unwrap().len(), 2, "--follow tracks rename");
        assert_eq!(l.as_arr().unwrap()[1].str_of("body").unwrap(), "body text");
        let st = status(r).unwrap();
        assert_eq!(st.as_arr().unwrap()[0].str_of("path").unwrap(), "b.gsc");
        assert!(diff(r, None, Some("b.gsc")).unwrap().str_of("diff").unwrap().contains("+two"));
        assert!(diff(r, Some("; rm -rf"), None).is_err());
    }
}

//! Git integration via the `git` CLI (whatever the user has installed).
//! Arguments are passed as an argv array — never through a shell — and paths
//! always follow `--`. Reads never take repository locks. Writes (init,
//! commit, push, pull, remote) happen only on an explicit user action; pushes
//! use the user's own Git credentials (Git Credential Manager on Windows).

use crate::db::DbResult;
use crate::json::Json;
use std::path::Path;
use std::process::Command;

fn git(root: &Path, args: &[&str]) -> Result<String, String> {
    run(root, args, true)
}

fn git_write(root: &Path, args: &[&str]) -> Result<String, String> {
    run(root, args, false)
}

fn run(root: &Path, args: &[&str], read_only: bool) -> Result<String, String> {
    let mut c = Command::new("git");
    c.arg("-C").arg(root).args(["-c", "core.quotepath=off", "-c", "color.ui=never"]).args(args);
    c.env("GIT_TERMINAL_PROMPT", "0");
    if read_only {
        c.env("GIT_OPTIONAL_LOCKS", "0"); // never take index locks: don't disturb the user's own git use
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let out = c.output().map_err(|e| format!("git is not available: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        let o = String::from_utf8_lossy(&out.stdout).trim().to_string();
        return Err(if err.is_empty() { o } else { err });
    }
    // git writes progress for push/pull to stderr; keep it for display
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    if !read_only {
        text.push_str(&String::from_utf8_lossy(&out.stderr));
    }
    Ok(text)
}

fn gh(root: &Path, args: &[&str]) -> Result<String, String> {
    let mut c = Command::new("gh");
    c.current_dir(root).args(args).env("GH_PROMPT_DISABLED", "1");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000);
    }
    let out = c.output().map_err(|e| format!("GitHub CLI (gh) is not available: {e}"))?;
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    if out.status.success() { Ok(text) } else { Err(text.trim().to_string()) }
}

// ------------------------------------------------------------------ writes

/// `git init` (+ a sensible .gitignore if none exists; never overwrites).
pub fn init(root: &Path) -> DbResult<Json> {
    if info(root)?.get("isRepo").and_then(|x| x.as_bool()) == Some(true) {
        return Err("This folder is already inside a Git repository".into());
    }
    git_write(root, &["init", "-b", "main"]).or_else(|_| git_write(root, &["init"]))?;
    let gi = root.join(".gitignore");
    if !gi.exists() {
        let _ = crate::util::atomic_write(&gi, b"# Added by Project Vault\nnode_modules/\ntarget/\ndist/\nbuild/\n.vs/\n*.tmp\nThumbs.db\ndesktop.ini\n");
    }
    info(root)
}

pub fn identity(root: &Path) -> DbResult<Json> {
    let name = git(root, &["config", "user.name"]).unwrap_or_default().trim().to_string();
    let email = git(root, &["config", "user.email"]).unwrap_or_default().trim().to_string();
    Ok(Json::obj().with("name", name).with("email", email))
}

/// Sets the commit author for this repository only (not globally).
pub fn set_identity(root: &Path, name: &str, email: &str) -> DbResult<Json> {
    if name.trim().is_empty() || !email.contains('@') {
        return Err("Enter a name and an email address".into());
    }
    git_write(root, &["config", "user.name", name.trim()])?;
    git_write(root, &["config", "user.email", email.trim()])?;
    identity(root)
}

/// Stages the given paths (or everything when `paths` is empty) and commits.
pub fn commit(root: &Path, message: &str, paths: &[String]) -> DbResult<Json> {
    if message.trim().is_empty() {
        return Err("Write a commit message describing the change".into());
    }
    let id = identity(root)?;
    if id.str_of("name").unwrap_or_default().is_empty() || id.str_of("email").unwrap_or_default().is_empty() {
        return Err("Git doesn't know who you are yet. Set your name and email in the GitHub section first.".into());
    }
    if paths.is_empty() {
        git_write(root, &["add", "-A"])?;
    } else {
        let mut a: Vec<&str> = vec!["add", "-A", "--"];
        a.extend(paths.iter().map(|s| s.as_str()));
        git_write(root, &a)?;
    }
    let mut a: Vec<&str> = vec!["commit", "-m", message.trim()];
    if !paths.is_empty() {
        a.push("--");
        a.extend(paths.iter().map(|s| s.as_str()));
    }
    let out = git_write(root, &a)?;
    Ok(Json::obj().with("output", out.trim()).with("info", info(root)?))
}

fn current_branch(root: &Path) -> DbResult<String> {
    let b = git(root, &["rev-parse", "--abbrev-ref", "HEAD"])?.trim().to_string();
    if b == "HEAD" || b.is_empty() {
        return Err("Not on a branch (detached HEAD); check out a branch first".into());
    }
    Ok(b)
}

pub fn push(root: &Path) -> DbResult<Json> {
    let branch = current_branch(root)?;
    let remotes = git(root, &["remote"])?;
    if remotes.trim().is_empty() {
        return Err("This repository has no remote yet. Connect it to GitHub first.".into());
    }
    let has_upstream = git(root, &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{upstream}"]).is_ok();
    let out = if has_upstream {
        git_write(root, &["push"])?
    } else {
        let remote = if remotes.lines().any(|l| l.trim() == "origin") { "origin".to_string() } else { remotes.lines().next().unwrap().trim().to_string() };
        git_write(root, &["push", "-u", &remote, &branch])?
    };
    Ok(Json::obj().with("output", out.trim()).with("info", info(root)?))
}

/// Fast-forward only: never creates merge commits or rewrites local work.
pub fn pull(root: &Path) -> DbResult<Json> {
    current_branch(root)?;
    let out = git_write(root, &["pull", "--ff-only"]).map_err(|e| {
        if e.contains("Not possible to fast-forward") || e.contains("diverge") {
            format!("Your local branch and GitHub both have new commits. Resolve this in your Git tool (merge or rebase); Project Vault only does fast-forward pulls.\n\n{e}")
        } else {
            e
        }
    })?;
    Ok(Json::obj().with("output", out.trim()).with("info", info(root)?))
}

pub fn fetch(root: &Path) -> DbResult<Json> {
    git_write(root, &["fetch", "--prune"])?;
    info(root)
}

pub fn valid_remote_url(url: &str) -> bool {
    let u = url.trim();
    let ok_chars = u.chars().all(|c| c.is_ascii_alphanumeric() || "-._~:/@".contains(c));
    ok_chars && !u.starts_with('-') && (u.starts_with("https://") || u.starts_with("git@") || u.starts_with("ssh://"))
}

pub fn set_remote(root: &Path, url: &str) -> DbResult<Json> {
    let url = url.trim();
    if !valid_remote_url(url) {
        return Err("Use the repository URL GitHub shows, e.g. https://github.com/you/repo.git".into());
    }
    let has_origin = git(root, &["remote"])?.lines().any(|l| l.trim() == "origin");
    if has_origin {
        git_write(root, &["remote", "set-url", "origin", url])?;
    } else {
        git_write(root, &["remote", "add", "origin", url])?;
    }
    info(root)
}

pub fn gh_status(root: &Path) -> Json {
    match gh(root, &["auth", "status"]) {
        Ok(t) => Json::obj().with("installed", true).with("loggedIn", true).with("detail", t.trim()),
        Err(e) if e.starts_with("GitHub CLI (gh) is not available") => Json::obj().with("installed", false).with("loggedIn", false),
        Err(e) => Json::obj().with("installed", true).with("loggedIn", false).with("detail", e),
    }
}

/// Creates a GitHub repository with the GitHub CLI and pushes to it.
pub fn gh_create(root: &Path, name: &str, private: bool, description: &str) -> DbResult<Json> {
    let name = name.trim();
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c)) {
        return Err("Repository names may contain letters, numbers, - _ and .".into());
    }
    let src = root.to_string_lossy().to_string();
    let mut args = vec!["repo", "create", name, if private { "--private" } else { "--public" }, "--source", &src, "--remote", "origin", "--push"];
    if !description.trim().is_empty() {
        args.extend(["--description", description.trim()]);
    }
    let out = gh(root, &args)?;
    Ok(Json::obj().with("output", out.trim()).with("info", info(root)?))
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

        // write path: commit without identity is refused with a clear message
        let id_missing = identity(r).unwrap().str_of("name").unwrap_or_default().is_empty();
        if id_missing {
            assert!(commit(r, "msg", &[]).is_err());
        }
        set_identity(r, "Dev", "dev@example.com").unwrap();
        assert!(commit(r, "  ", &[]).is_err());
        commit(r, "Second line", &[]).unwrap();
        assert_eq!(log(r, None, 1).unwrap().as_arr().unwrap()[0].str_of("subject").unwrap(), "Second line");
        assert!(push(r).is_err(), "no remote");
        // push to a local bare remote
        let bare = t.path().join("remote.git");
        std::fs::create_dir_all(&bare).unwrap();
        git(&bare, &["init", "--bare", "-q"]).unwrap();
        assert!(set_remote(r, "file:///x").is_err());
        assert!(set_remote(r, "--upload-pack=evil").is_err());
        git_write(r, &["remote", "add", "origin", &bare.to_string_lossy()]).unwrap();
        push(r).unwrap();
        assert!(info(r).unwrap().get("upstream").is_some());
        pull(r).unwrap();
    }
}

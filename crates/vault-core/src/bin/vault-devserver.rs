//! Development/test bridge: serves the built UI and exposes the same
//! `api::call` used by the desktop app over HTTP on 127.0.0.1 only.
//!
//!   vault-devserver <data-dir> <ui-dist-dir> [port]
//!
//! POST /api/<command>  body = JSON args  → JSON envelope
//! GET  /...            static files from ui-dist-dir
//!
//! Not shipped in the installer. Used to run the real UI against the real
//! core in a browser (automated UI tests, development without Tauri).

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use vault_core::{api, util::safe_join, Vault};

fn respond(s: &mut TcpStream, code: &str, ctype: &str, body: &[u8]) {
    let _ = write!(s, "HTTP/1.1 {code}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n", body.len());
    let _ = s.write_all(body);
}

fn handle(mut s: TcpStream, vault: &Mutex<Vault>, ui: &Path) {
    let mut r = BufReader::new(s.try_clone().unwrap());
    let mut line = String::new();
    if r.read_line(&mut line).is_err() {
        return;
    }
    let mut parts = line.split_whitespace();
    let (method, path) = (parts.next().unwrap_or(""), parts.next().unwrap_or("/").to_string());
    let mut len = 0usize;
    loop {
        let mut h = String::new();
        if r.read_line(&mut h).is_err() || h.trim().is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            if k.eq_ignore_ascii_case("content-length") {
                len = v.trim().parse().unwrap_or(0);
            }
        }
    }
    if let Some(cmd) = path.strip_prefix("/api/") {
        if method != "POST" {
            return respond(&mut s, "405 Method Not Allowed", "text/plain", b"POST only");
        }
        let mut body = vec![0u8; len.min(64 * 1024 * 1024)];
        if r.read_exact(&mut body).is_err() {
            return respond(&mut s, "400 Bad Request", "text/plain", b"short body");
        }
        let dbg = std::env::var_os("PV_WATCH_DEBUG").is_some();
        let t0 = std::time::Instant::now();
        let out = {
            let v = vault.lock().unwrap_or_else(|e| e.into_inner());
            let t1 = t0.elapsed();
            let o = api::call(&v, cmd, &String::from_utf8_lossy(&body));
            if dbg && (t0.elapsed().as_millis() > 1500 || t1.as_millis() > 1500) {
                eprintln!("[req] {cmd}: waited {t1:?}, total {:?}", t0.elapsed());
            }
            o
        };
        return respond(&mut s, "200 OK", "application/json", out.as_bytes());
    }
    let rel = path.split('?').next().unwrap_or("/").trim_start_matches('/');
    let rel = if rel.is_empty() { "index.html" } else { rel };
    let file = safe_join(ui, rel).ok().filter(|p| p.is_file()).unwrap_or_else(|| ui.join("index.html"));
    let ctype = match file.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" => "text/javascript",
        "css" => "text/css",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        _ => "application/octet-stream",
    };
    match std::fs::read(&file) {
        Ok(b) => respond(&mut s, "200 OK", ctype, &b),
        Err(_) => respond(&mut s, "404 Not Found", "text/plain", b"not found"),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: vault-devserver <data-dir> <ui-dist-dir> [port]");
        std::process::exit(2);
    }
    let vault = Vault::open(Path::new(&args[1])).unwrap_or_else(|e| {
        eprintln!("cannot open vault: {e}");
        std::process::exit(1)
    });
    let ui = PathBuf::from(&args[2]);
    let port = args.get(3).cloned().unwrap_or_else(|| "1420".into());
    let l = TcpListener::bind(format!("127.0.0.1:{port}")).expect("bind");
    eprintln!("Mothball dev server on http://127.0.0.1:{port}");
    let vault = Arc::new(Mutex::new(vault));
    {
        // background change watching, same as the desktop app
        let v = vault.clone();
        std::thread::spawn(move || {
            let mut w = vault_core::watch::Watcher::default();
            loop {
                let wait = vault_core::watch::pass(&mut w, |f| f(&v.lock().unwrap_or_else(|p| p.into_inner())));
                std::thread::sleep(wait);
            }
        });
    }
    {
        // printer status; the dev server talks plain TCP (to a fake printer in tests)
        let v = vault.clone();
        let locker: vault_core::printers::Locker = Arc::new(move |f: &mut dyn FnMut(&Vault)| f(&v.lock().unwrap_or_else(|p| p.into_inner())));
        vault_core::printers::start(vault_core::printers::plain_tcp_connector(), locker.clone());
        // backup drive: scheduled runs and restore jobs, copying without the lock
        vault_core::drive::start(locker);
    }
    for s in l.incoming().flatten() {
        let (v, ui) = (vault.clone(), ui.clone());
        std::thread::spawn(move || handle(s, &v, &ui));
    }
}

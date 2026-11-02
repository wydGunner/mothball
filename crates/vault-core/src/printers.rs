//! Bambu Lab printer status over the local network (read-only).
//!
//! The printer runs an MQTT broker on port 8883 (TLS). With the username
//! `bblp` and the LAN access code shown on the printer, a client may subscribe
//! to `device/<serial>/report`, where the printer publishes its state as JSON.
//! Since Bambu's 2025 "authorization" firmware, unsigned clients can still read
//! this; only commands that control the printer need Bambu's own software or
//! Developer Mode. Mothball never sends control commands. The one message it
//! publishes is `pushall` (asks for a full report); a printer that ignores it
//! still sends incremental reports, which are merged here.
//!
//! The core speaks MQTT over any byte stream. The TLS layer is injected by the
//! host (the desktop shell uses rustls with the printer's certificate pinned on
//! first use); the dev server and tests use plain TCP against a fake printer.
//!
//! When a job ends (finished, failed or stopped) it is recorded once in the
//! printer history and, if it can be matched to a project, appended to that
//! project's print log and timeline.

use crate::db::DbResult;
use crate::json::Json;
use crate::util::{new_id, now_ms};
use crate::{p, timeline, watch, Vault};
use std::collections::{BTreeMap, HashMap};
use std::io::{ErrorKind, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const PORT: u16 = 8883;
const HISTORY_MAX: usize = 300;
/// A finished job first seen after the fact (app was closed while it ran) is
/// still logged if it started within this window.
const LATE_WINDOW_MS: i64 = 7 * 24 * 3600 * 1000;

// ---------------------------------------------------------------- host hooks

pub trait Stream: Read + Write + Send {}
impl<T: Read + Write + Send> Stream for T {}

/// What a connector needs to open a connection.
pub struct Endpoint {
    pub host: String,
    pub port: u16,
    pub serial: String,
    /// SHA-256 (hex) of the certificate seen on the first successful
    /// connection; a different certificate must be refused.
    pub pinned_sha256: Option<String>,
}

pub struct Connected {
    /// Must have a read timeout of about a second, so the worker can notice
    /// shutdown and send keep-alives; timeouts are not treated as errors.
    pub stream: Box<dyn Stream>,
    /// Certificate fingerprint presented by the printer (None for plain TCP).
    pub cert_sha256: Option<String>,
}

pub type Connector = Arc<dyn Fn(&Endpoint) -> Result<Connected, String> + Send + Sync>;
/// Runs a closure with the vault locked; does nothing when no vault is open.
pub type Locker = Arc<dyn Fn(&mut dyn FnMut(&Vault)) + Send + Sync>;

/// Plain TCP, for the dev server and tests only (real printers require TLS).
pub fn plain_tcp_connector() -> Connector {
    Arc::new(|e: &Endpoint| {
        let s = std::net::TcpStream::connect((e.host.as_str(), e.port)).map_err(|err| format!("Cannot reach the printer at {}: {err}", e.host))?;
        s.set_read_timeout(Some(Duration::from_millis(500))).ok();
        Ok(Connected { stream: Box::new(s), cert_sha256: None })
    })
}

// ---------------------------------------------------------------- settings

#[derive(Clone, Debug, PartialEq)]
pub struct Printer {
    pub id: String,
    pub name: String,
    pub host: String,
    pub serial: String,
    pub access_code: String,
    pub cert_sha256: Option<String>,
}

fn meta_get(v: &Vault, key: &str) -> DbResult<Json> {
    let s = v.db.one("SELECT value FROM meta WHERE key=?", p![key])?.map(|r| r.s("value")).unwrap_or_default();
    Ok(Json::parse(&s).unwrap_or(Json::Null))
}
fn meta_set(v: &Vault, key: &str, j: &Json) -> DbResult<()> {
    v.db.exec("INSERT OR REPLACE INTO meta(key,value) VALUES (?,?)", p![key, j.to_string()])?;
    Ok(())
}

pub fn configs(v: &Vault) -> DbResult<Vec<Printer>> {
    let j = meta_get(v, "printers")?;
    Ok(j.as_arr()
        .map(|a| {
            a.iter()
                .map(|x| Printer {
                    id: x.str_of("id").unwrap_or_default(),
                    name: x.str_of("name").unwrap_or_default(),
                    host: x.str_of("host").unwrap_or_default(),
                    serial: x.str_of("serial").unwrap_or_default(),
                    access_code: x.str_of("accessCode").unwrap_or_default(),
                    cert_sha256: x.str_of("certSha256"),
                })
                .filter(|p| !p.id.is_empty())
                .collect()
        })
        .unwrap_or_default())
}

fn save_configs(v: &Vault, list: &[Printer]) -> DbResult<()> {
    let a = Json::Arr(
        list.iter()
            .map(|p| {
                Json::obj()
                    .with("id", p.id.as_str())
                    .with("name", p.name.as_str())
                    .with("host", p.host.as_str())
                    .with("serial", p.serial.as_str())
                    .with("accessCode", p.access_code.as_str())
                    .with("certSha256", p.cert_sha256.clone())
            })
            .collect(),
    );
    meta_set(v, "printers", &a)
}

/// "192.168.1.5" → port 8883; "127.0.0.1:1883" keeps its port (dev/tests).
fn split_host(host: &str) -> (String, u16) {
    let h = host.trim();
    if let Some((a, b)) = h.rsplit_once(':') {
        if !a.contains(':') {
            if let Ok(port) = b.parse() {
                return (a.to_string(), port);
            }
        }
    }
    (h.to_string(), PORT)
}

pub fn save(v: &Vault, a: &Json) -> DbResult<Json> {
    let mut list = configs(v)?;
    let name = a.str_of("name").unwrap_or_default().trim().to_string();
    let host = a.str_of("host").unwrap_or_default().trim().to_string();
    let serial = a.str_of("serial").unwrap_or_default().trim().to_uppercase();
    let code = a.str_of("accessCode").map(|c| c.trim().to_string()).filter(|c| !c.is_empty());
    if host.is_empty() {
        return Err("Enter the printer's IP address (shown in its network settings).".into());
    }
    if serial.is_empty() {
        return Err("Enter the printer's serial number.".into());
    }
    let id = match a.str_of("id").filter(|s| !s.is_empty()) {
        Some(id) => {
            let p = list.iter_mut().find(|p| p.id == id).ok_or("Printer not found")?;
            // a different device at that address must not inherit the old pin
            if p.host != host || p.serial != serial {
                p.cert_sha256 = None;
            }
            p.name = if name.is_empty() { serial.clone() } else { name };
            p.host = host;
            p.serial = serial;
            if let Some(c) = code {
                p.access_code = c;
            }
            id
        }
        None => {
            let c = code.ok_or("Enter the LAN access code shown on the printer.")?;
            if list.iter().any(|p| p.serial == serial) {
                return Err("That printer is already added.".into());
            }
            let id = new_id();
            list.push(Printer { id: id.clone(), name: if name.is_empty() { serial.clone() } else { name }, host, serial, access_code: c, cert_sha256: None });
            id
        }
    };
    save_configs(v, &list)?;
    Ok(Json::obj().with("id", id))
}

pub fn remove(v: &Vault, id: &str) -> DbResult<Json> {
    let mut list = configs(v)?;
    list.retain(|p| p.id != id);
    save_configs(v, &list)?;
    live().lock().unwrap_or_else(|e| e.into_inner()).remove(id);
    Ok(Json::obj().with("ok", true))
}

/// Forget the pinned certificate (after a firmware update replaced it).
pub fn reset_certificate(v: &Vault, id: &str) -> DbResult<Json> {
    let mut list = configs(v)?;
    list.iter_mut().find(|p| p.id == id).ok_or("Printer not found")?.cert_sha256 = None;
    save_configs(v, &list)?;
    Ok(Json::obj().with("ok", true))
}

fn pin_certificate(v: &Vault, id: &str, sha: &str) -> DbResult<()> {
    let mut list = configs(v)?;
    if let Some(p) = list.iter_mut().find(|p| p.id == id) {
        if p.cert_sha256.is_none() {
            p.cert_sha256 = Some(sha.to_string());
            save_configs(v, &list)?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------- live state

#[derive(Clone, Debug)]
pub struct Live {
    /// connecting | online | offline | error
    pub state: String,
    pub message: Option<String>,
    /// The printer's report ("print" object), merged across incremental updates.
    pub report: Json,
    pub updated_at: i64,
}

fn live() -> &'static Mutex<HashMap<String, Live>> {
    static L: std::sync::OnceLock<Mutex<HashMap<String, Live>>> = std::sync::OnceLock::new();
    L.get_or_init(|| Mutex::new(HashMap::new()))
}

fn set_live(id: &str, f: impl FnOnce(&mut Live)) {
    let mut m = live().lock().unwrap_or_else(|e| e.into_inner());
    let e = m.entry(id.to_string()).or_insert_with(|| Live { state: "connecting".into(), message: None, report: Json::obj(), updated_at: 0 });
    f(e);
}

fn num(r: &Json, k: &str) -> Option<f64> {
    match r.get(k)? {
        Json::Num(n) => Some(*n),
        Json::Str(s) => s.trim().parse().ok(),
        _ => None,
    }
}

/// Job name as the printer reports it, tidied for display and matching.
fn job_name(r: &Json) -> String {
    let raw = r.str_of("subtask_name").filter(|s| !s.trim().is_empty()).or_else(|| {
        r.str_of("gcode_file").map(|g| g.rsplit(['/', '\\']).next().unwrap_or(&g).to_string())
    });
    let mut s = raw.unwrap_or_default().trim().to_string();
    for suffix in [".gcode.3mf", ".3mf", ".gcode"] {
        if s.to_lowercase().ends_with(suffix) {
            s.truncate(s.len() - suffix.len());
        }
    }
    s
}

/// Bambu shows print errors as two hex groups, e.g. 0300-4000.
fn error_code(r: &Json) -> Option<String> {
    let n = num(r, "print_error")? as i64;
    (n != 0).then(|| format!("{:04X}-{:04X}", (n >> 16) & 0xFFFF, n & 0xFFFF))
}

fn status_json(p: &Printer, l: Option<&Live>) -> Json {
    let mut j = Json::obj()
        .with("id", p.id.as_str())
        .with("name", p.name.as_str())
        .with("host", p.host.as_str())
        .with("serial", p.serial.as_str())
        .with("hasCode", !p.access_code.is_empty())
        .with("certPinned", p.cert_sha256.is_some());
    let Some(l) = l else { return j.with("state", "connecting") };
    j.set("state", l.state.as_str());
    j.set("message", l.message.clone());
    j.set("updatedAt", l.updated_at);
    let r = &l.report;
    if l.state == "online" {
        let stage = r.str_of("gcode_state").unwrap_or_default();
        j.set("stage", stage.to_lowercase());
        j.set("job", Some(job_name(r)).filter(|s| !s.is_empty()));
        j.set("progress", num(r, "mc_percent").map(|n| n as i64));
        j.set("remainingMin", num(r, "mc_remaining_time").map(|n| n as i64));
        j.set("layer", num(r, "layer_num").map(|n| n as i64));
        j.set("totalLayers", num(r, "total_layer_num").map(|n| n as i64));
        j.set("nozzle", num(r, "nozzle_temper").map(|n| n.round() as i64));
        j.set("bed", num(r, "bed_temper").map(|n| n.round() as i64));
        j.set("error", error_code(r));
        j.set("hms", r.get("hms").and_then(|h| h.as_arr()).map(|a| a.len()).unwrap_or(0));
    }
    j
}

pub fn list(v: &Vault) -> DbResult<Json> {
    let m = live().lock().unwrap_or_else(|e| e.into_inner());
    Ok(Json::Arr(configs(v)?.iter().map(|p| status_json(p, m.get(&p.id))).collect()))
}

// ---------------------------------------------------------------- MQTT 3.1.1 (the part needed here)

fn put_str(out: &mut Vec<u8>, s: &str) {
    out.extend_from_slice(&(s.len() as u16).to_be_bytes());
    out.extend_from_slice(s.as_bytes());
}

fn packet(kind: u8, body: &[u8]) -> Vec<u8> {
    let mut out = vec![kind];
    let mut n = body.len();
    loop {
        let mut b = (n % 128) as u8;
        n /= 128;
        if n > 0 {
            b |= 0x80;
        }
        out.push(b);
        if n == 0 {
            break;
        }
    }
    out.extend_from_slice(body);
    out
}

pub fn connect_packet(client_id: &str, user: &str, pass: &str, keepalive: u16) -> Vec<u8> {
    let mut b = Vec::new();
    put_str(&mut b, "MQTT");
    b.push(4); // protocol level 3.1.1
    b.push(0xC2); // username, password, clean session
    b.extend_from_slice(&keepalive.to_be_bytes());
    put_str(&mut b, client_id);
    put_str(&mut b, user);
    put_str(&mut b, pass);
    packet(0x10, &b)
}

pub fn subscribe_packet(id: u16, topic: &str) -> Vec<u8> {
    let mut b = id.to_be_bytes().to_vec();
    put_str(&mut b, topic);
    b.push(0); // QoS 0
    packet(0x82, &b)
}

pub fn publish_packet(topic: &str, payload: &[u8]) -> Vec<u8> {
    let mut b = Vec::new();
    put_str(&mut b, topic);
    b.extend_from_slice(payload);
    packet(0x30, &b)
}

#[derive(Debug, PartialEq)]
pub enum Packet {
    ConnAck(u8),
    SubAck,
    Publish { topic: String, payload: Vec<u8> },
    PingResp,
    Other(u8),
}

/// Parses one complete packet from the front of `buf`; returns it and its length.
pub fn parse_packet(buf: &[u8]) -> Result<Option<(Packet, usize)>, String> {
    if buf.len() < 2 {
        return Ok(None);
    }
    let (mut len, mut mul, mut i) = (0usize, 1usize, 1usize);
    loop {
        let Some(&b) = buf.get(i) else { return Ok(None) };
        len += (b & 0x7F) as usize * mul;
        i += 1;
        if b & 0x80 == 0 {
            break;
        }
        mul *= 128;
        if i > 4 {
            return Err("malformed packet length".into());
        }
    }
    if len > 16 * 1024 * 1024 {
        return Err("packet too large".into());
    }
    if buf.len() < i + len {
        return Ok(None);
    }
    let body = &buf[i..i + len];
    let kind = buf[0];
    let p = match kind >> 4 {
        2 => Packet::ConnAck(*body.get(1).unwrap_or(&255)),
        9 => Packet::SubAck,
        13 => Packet::PingResp,
        3 => {
            if body.len() < 2 {
                return Err("malformed publish".into());
            }
            let tl = u16::from_be_bytes([body[0], body[1]]) as usize;
            if body.len() < 2 + tl {
                return Err("malformed publish".into());
            }
            let topic = String::from_utf8_lossy(&body[2..2 + tl]).to_string();
            let qos = (kind >> 1) & 3;
            let start = 2 + tl + if qos > 0 { 2 } else { 0 };
            Packet::Publish { topic, payload: body.get(start..).unwrap_or(&[]).to_vec() }
        }
        _ => Packet::Other(kind),
    };
    Ok(Some((p, i + len)))
}

/// Shallow-recursive merge: objects merge, everything else replaces.
fn merge(into: &mut Json, from: &Json) {
    if let (Json::Obj(a), Json::Obj(b)) = (&mut *into, from) {
        for (k, v) in b {
            match (a.get_mut(k), v) {
                (Some(cur @ Json::Obj(_)), Json::Obj(_)) => merge(cur, v),
                _ => {
                    a.insert(k.clone(), v.clone());
                }
            }
        }
    } else {
        *into = from.clone();
    }
}

// ---------------------------------------------------------------- job tracking

#[derive(Clone, Debug, PartialEq)]
pub struct Finished {
    pub key: String,
    pub name: String,
    pub file: String,
    /// success | failure | stopped
    pub result: String,
    pub error: Option<String>,
    pub started_at: Option<i64>,
    pub ended_at: i64,
    /// True when the job was never seen running (the app was closed).
    pub late: bool,
}

#[derive(Default)]
pub struct Tracker {
    running: Option<(String, String, String, Option<i64>)>, // key, name, file, start
    last_final: Option<String>,
}

fn start_ms(r: &Json) -> Option<i64> {
    let s = num(r, "gcode_start_time")? as i64;
    (s > 0).then(|| if s < 10_000_000_000 { s * 1000 } else { s })
}

fn job_key(r: &Json, name: &str) -> String {
    match start_ms(r) {
        Some(s) => format!("{name}|{s}"),
        // no start time: one job per name per day is the best that can be told apart
        None => format!("{name}|day{}", now_ms() / 86_400_000),
    }
}

impl Tracker {
    /// Feed the merged report; returns a job that just ended.
    pub fn update(&mut self, r: &Json, now: i64) -> Option<Finished> {
        let stage = r.str_of("gcode_state").unwrap_or_default().to_uppercase();
        let name = job_name(r);
        let file = r.str_of("gcode_file").unwrap_or_default();
        match stage.as_str() {
            "PREPARE" | "RUNNING" | "PAUSE" | "SLICING" if !name.is_empty() => {
                let key = job_key(r, &name);
                if self.running.as_ref().map(|x| &x.0) != Some(&key) {
                    self.running = Some((key, name, file, start_ms(r)));
                }
                None
            }
            "FINISH" | "FAILED" => {
                let (key, name, file, start, late) = match self.running.take() {
                    Some((k, n, f, s)) => (k, n, f, s, false),
                    None if !name.is_empty() => (job_key(r, &name), name, file, start_ms(r), true),
                    None => return None,
                };
                if self.last_final.as_deref() == Some(&key) {
                    return None;
                }
                self.last_final = Some(key.clone());
                let ok = stage == "FINISH" && error_code(r).is_none();
                Some(Finished {
                    key,
                    name,
                    file,
                    result: if ok { "success" } else { "failure" }.into(),
                    error: error_code(r),
                    started_at: start,
                    ended_at: now,
                    late,
                })
            }
            "IDLE" => {
                // went idle without finishing: stopped from the printer
                let (key, name, file, start) = self.running.take()?;
                self.last_final = Some(key.clone());
                Some(Finished { key, name, file, result: "stopped".into(), error: error_code(r), started_at: start, ended_at: now, late: false })
            }
            _ => None,
        }
    }
}

// ---------------------------------------------------------------- recording

fn history(v: &Vault) -> DbResult<Vec<Json>> {
    Ok(meta_get(v, "printer-history")?.as_arr().cloned().unwrap_or_default())
}

fn rules(v: &Vault) -> DbResult<BTreeMap<String, String>> {
    let j = meta_get(v, "printer-rules")?;
    let mut m = BTreeMap::new();
    if let Json::Obj(o) = j {
        for (k, x) in o {
            if let Some(s) = x.as_str() {
                m.insert(k, s.to_string());
            }
        }
    }
    Ok(m)
}

fn norm(s: &str) -> String {
    let s = s.trim().to_lowercase();
    // Bambu Studio may append the plate: "desk cat_plate_2"
    match s.rfind("_plate_") {
        Some(i) if s[i + 7..].chars().all(|c| c.is_ascii_digit()) && i + 7 < s.len() => s[..i].to_string(),
        _ => s,
    }
}

/// The project a job belongs to: a remembered rule, else the one project that
/// has a model or plate file with the job's name (3D-printing projects first).
pub fn match_project(v: &Vault, name: &str) -> DbResult<Option<String>> {
    let n = norm(name);
    if n.is_empty() {
        return Ok(None);
    }
    if let Some(pid) = rules(v)?.get(&n) {
        if v.db.one("SELECT 1 FROM projects WHERE id=?", p![pid.as_str()])?.is_some() {
            return Ok(Some(pid.clone()));
        }
    }
    let rows = v.db.query(
        "SELECT DISTINCT f.project_id, p.category, f.name FROM files f JOIN projects p ON p.id=f.project_id
         WHERE f.state='present' AND f.ext IN ('3mf','gcode','stl','obj','step','stp','f3d','scad','amf','ply')",
        p![],
    )?;
    let mut hits: Vec<(String, bool)> = Vec::new();
    for r in rows {
        let file = r.s("name");
        let stem = file.to_lowercase();
        let stem = stem.strip_suffix(".gcode.3mf").or_else(|| stem.rsplit_once('.').map(|x| x.0)).unwrap_or(&stem).to_string();
        if norm(&stem) == n && !hits.iter().any(|h| h.0 == r.s("project_id")) {
            hits.push((r.s("project_id"), r.s("category") == "3d-printing"));
        }
    }
    let printing: Vec<_> = hits.iter().filter(|h| h.1).collect();
    Ok(match (hits.len(), printing.len()) {
        (1, _) => Some(hits[0].0.clone()),
        (_, 1) => Some(printing[0].0.clone()),
        _ => None,
    })
}

fn fmt_minutes(m: i64) -> String {
    if m >= 60 { format!("{}h {:02}m", m / 60, m % 60) } else { format!("{m}m") }
}

fn utc_date(ms: i64) -> String {
    // civil-from-days (Howard Hinnant), UTC
    let z = ms.div_euclid(86_400_000) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{y:04}-{m:02}-{d:02}")
}

/// Adds the job to the project's print log and timeline.
fn log_to_project(v: &Vault, pid: &str, h: &Json) -> DbResult<()> {
    let row = v.db.one("SELECT module_json FROM projects WHERE id=?", p![pid])?.ok_or("Project not found")?;
    let mut module = Json::parse(&row.s("module_json")).unwrap_or(Json::obj());
    let mut log = module.get("printLog").and_then(|l| l.as_arr()).cloned().unwrap_or_default();
    let key = h.str_of("key").unwrap_or_default();
    if log.iter().any(|e| e.str_of("key").as_deref() == Some(key.as_str())) {
        return Ok(());
    }
    let result = h.str_of("result").unwrap_or_default();
    let printer = h.str_of("printer").unwrap_or_default();
    let name = h.str_of("name").unwrap_or_default();
    let mins = h.get("minutes").and_then(|m| m.as_i64());
    let ended = h.get("endedAt").and_then(|x| x.as_i64()).unwrap_or_else(now_ms);
    let mut notes = format!("{name} on {printer}");
    match (result.as_str(), h.str_of("error")) {
        ("stopped", _) => notes.push_str(" (stopped before finishing)"),
        (_, Some(e)) => notes.push_str(&format!(" (error {e})")),
        _ => {}
    }
    log.push(
        Json::obj()
            .with("date", utc_date(ended))
            .with("at", ended)
            .with("result", if result == "success" { "success" } else { "failure" })
            .with("notes", notes)
            .with("grams", "")
            .with("time", mins.map(fmt_minutes).unwrap_or_default())
            .with("source", "printer")
            .with("key", key),
    );
    module.set("printLog", Json::Arr(log));
    v.db.exec("UPDATE projects SET module_json=? WHERE id=?", p![module.to_string(), pid])?;
    let summary = match result.as_str() {
        "success" => format!("Printed {name}"),
        "stopped" => format!("Print stopped: {name}"),
        _ => format!("Print failed: {name}"),
    };
    let detail = format!("{printer}{}", mins.map(|m| format!(" · {}", fmt_minutes(m))).unwrap_or_default());
    timeline::add(v, pid, "print", &summary, &detail, None, false)?;
    Ok(())
}

/// Stores a finished job once. Returns false when it was already recorded or
/// is a stale "late" finish.
pub fn record(v: &Vault, printer: &Printer, f: &Finished) -> DbResult<bool> {
    let mut hist = history(v)?;
    if hist.iter().any(|h| h.str_of("key").as_deref() == Some(f.key.as_str()) && h.str_of("printerId").as_deref() == Some(printer.id.as_str())) {
        return Ok(false);
    }
    if f.late {
        // only if we know it started recently; an old FINISH screen is not news
        match f.started_at {
            Some(s) if now_ms() - s < LATE_WINDOW_MS => {}
            _ => return Ok(false),
        }
    }
    let pid = match_project(v, &f.name)?;
    let minutes = f.started_at.filter(|_| !f.late).map(|s| ((f.ended_at - s) / 60_000).max(0));
    let h = Json::obj()
        .with("key", f.key.as_str())
        .with("printerId", printer.id.as_str())
        .with("printer", printer.name.as_str())
        .with("name", f.name.as_str())
        .with("file", f.file.as_str())
        .with("result", f.result.as_str())
        .with("error", f.error.clone())
        .with("startedAt", f.started_at)
        .with("endedAt", f.ended_at)
        .with("minutes", minutes)
        .with("projectId", pid.clone());
    if let Some(pid) = &pid {
        log_to_project(v, pid, &h)?;
    }
    hist.insert(0, h);
    hist.truncate(HISTORY_MAX);
    meta_set(v, "printer-history", &Json::Arr(hist))?;
    watch::bump(v)?;
    Ok(true)
}

pub fn history_json(v: &Vault, limit: usize) -> DbResult<Json> {
    let names: HashMap<String, String> = v.db.query("SELECT id, name FROM projects", p![])?.into_iter().map(|r| (r.s("id"), r.s("name"))).collect();
    let out = history(v)?
        .into_iter()
        .filter(|h| h.get("dismissed").and_then(|d| d.as_bool()) != Some(true))
        .take(limit)
        .map(|mut h| {
            if let Some(n) = h.str_of("projectId").and_then(|id| names.get(&id).cloned()) {
                h.set("projectName", n);
            } else if h.str_of("projectId").is_some() {
                h.set("projectId", Json::Null); // project was removed
            }
            h
        })
        .collect();
    Ok(Json::Arr(out))
}

/// Assign a recorded job to a project (or dismiss it with no project). With
/// `remember`, later jobs with the same name go to that project automatically.
pub fn assign(v: &Vault, key: &str, project_id: Option<&str>, remember: bool) -> DbResult<Json> {
    let mut hist = history(v)?;
    let h = hist.iter_mut().find(|h| h.str_of("key").as_deref() == Some(key)).ok_or("That print is no longer in the history")?;
    match project_id {
        Some(pid) => {
            v.db.one("SELECT 1 FROM projects WHERE id=?", p![pid])?.ok_or("Project not found")?;
            h.set("projectId", pid);
            h.remove("dismissed");
            let snapshot = h.clone();
            if remember {
                let mut r = meta_get(v, "printer-rules")?;
                if !matches!(r, Json::Obj(_)) {
                    r = Json::obj();
                }
                r.set(&norm(&snapshot.str_of("name").unwrap_or_default()), pid);
                meta_set(v, "printer-rules", &r)?;
            }
            log_to_project(v, pid, &snapshot)?;
        }
        None => h.set("dismissed", true),
    }
    meta_set(v, "printer-history", &Json::Arr(hist))?;
    watch::bump(v)?;
    Ok(Json::obj().with("ok", true))
}

// ---------------------------------------------------------------- workers

struct Worker {
    config: Printer,
    stop: Arc<AtomicBool>,
}

/// Starts the background printer connections. Call once per process. The
/// returned flag stops everything when set (used by tests).
pub fn start(connector: Connector, locker: Locker) -> Arc<AtomicBool> {
    let shutdown = Arc::new(AtomicBool::new(false));
    let down = shutdown.clone();
    std::thread::spawn(move || {
        let mut workers: HashMap<String, Worker> = HashMap::new();
        loop {
            if down.load(Ordering::SeqCst) {
                workers.values().for_each(|w| w.stop.store(true, Ordering::SeqCst));
                return;
            }
            let mut list: Option<Vec<Printer>> = None;
            locker(&mut |v: &Vault| list = configs(v).ok());
            let list = list.unwrap_or_default();
            // stop workers whose printer was removed or edited
            workers.retain(|id, w| {
                let same = list.iter().any(|p| &p.id == id && p.host == w.config.host && p.serial == w.config.serial && p.access_code == w.config.access_code && (p.cert_sha256.is_none() || p.cert_sha256 == w.config.cert_sha256 || w.config.cert_sha256.is_none()));
                if !same {
                    w.stop.store(true, Ordering::SeqCst);
                }
                same
            });
            for p in list {
                if !workers.contains_key(&p.id) {
                    let stop = Arc::new(AtomicBool::new(false));
                    let (c, l, s, cfg) = (connector.clone(), locker.clone(), stop.clone(), p.clone());
                    std::thread::spawn(move || run_worker(cfg, c, l, s));
                    workers.insert(p.id.clone(), Worker { config: p, stop });
                }
            }
            sleep_unless_stopped(&down, Duration::from_secs(2));
        }
    });
    shutdown
}

fn sleep_unless_stopped(stop: &AtomicBool, d: Duration) {
    let until = Instant::now() + d;
    while Instant::now() < until && !stop.load(Ordering::SeqCst) {
        std::thread::sleep(Duration::from_millis(200));
    }
}

fn run_worker(mut cfg: Printer, connector: Connector, locker: Locker, stop: Arc<AtomicBool>) {
    let mut tracker = Tracker::default();
    let mut backoff = Duration::from_secs(5);
    while !stop.load(Ordering::SeqCst) {
        set_live(&cfg.id, |l| {
            if l.state != "error" {
                l.state = "connecting".into();
            }
        });
        let (host, port) = split_host(&cfg.host);
        let ep = Endpoint { host, port, serial: cfg.serial.clone(), pinned_sha256: cfg.cert_sha256.clone() };
        let outcome = connector(&ep).and_then(|c| {
            if let (None, Some(sha)) = (&cfg.cert_sha256, &c.cert_sha256) {
                cfg.cert_sha256 = Some(sha.clone());
                let (id, sha) = (cfg.id.clone(), sha.clone());
                locker(&mut |v: &Vault| {
                    let _ = pin_certificate(v, &id, &sha);
                });
            }
            session(&cfg, c.stream, &locker, &stop, &mut tracker)
        });
        if stop.load(Ordering::SeqCst) {
            break;
        }
        match outcome {
            Ok(()) => {
                backoff = Duration::from_secs(5);
                set_live(&cfg.id, |l| {
                    l.state = "offline".into();
                    l.message = Some("Connection to the printer dropped; reconnecting.".into());
                });
            }
            Err(e) => {
                set_live(&cfg.id, |l| {
                    l.state = "error".into();
                    l.message = Some(e);
                });
                backoff = (backoff * 2).min(Duration::from_secs(60));
            }
        }
        sleep_unless_stopped(&stop, backoff);
    }
}

fn io_err(e: std::io::Error) -> String {
    format!("Connection error: {e}")
}

/// One connected session. Ok(()) = the connection dropped after working.
fn session(cfg: &Printer, mut s: Box<dyn Stream>, locker: &Locker, stop: &AtomicBool, tracker: &mut Tracker) -> Result<(), String> {
    let client = format!("mothball-{}", &new_id()[..8]);
    s.write_all(&connect_packet(&client, "bblp", &cfg.access_code, 60)).map_err(io_err)?;
    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 16 * 1024];
    let mut connected = false;
    let started = Instant::now();
    let mut last_rx = Instant::now();
    let mut last_ping = Instant::now();
    let report_topic = format!("device/{}/report", cfg.serial);
    loop {
        if stop.load(Ordering::SeqCst) {
            let _ = s.write_all(&[0xE0, 0]);
            return Ok(());
        }
        match s.read(&mut chunk) {
            Ok(0) => {
                return if connected { Ok(()) } else { Err("The printer closed the connection. Check the IP address and that LAN mode is on.".into()) };
            }
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                last_rx = Instant::now();
            }
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted) => {}
            Err(e) => return if connected { Ok(()) } else { Err(io_err(e)) },
        }
        while let Some((pk, used)) = parse_packet(&buf)? {
            buf.drain(..used);
            match pk {
                Packet::ConnAck(0) => {
                    connected = true;
                    s.write_all(&subscribe_packet(1, &report_topic)).map_err(io_err)?;
                    let push = br#"{"pushing":{"sequence_id":"0","command":"pushall","version":1,"push_target":1}}"#;
                    s.write_all(&publish_packet(&format!("device/{}/request", cfg.serial), push)).map_err(io_err)?;
                    set_live(&cfg.id, |l| {
                        l.state = "online".into();
                        l.message = None;
                        l.updated_at = now_ms();
                    });
                }
                Packet::ConnAck(4) | Packet::ConnAck(5) => {
                    return Err("The printer refused the access code. Check the LAN access code on the printer's screen (it changes when LAN mode is switched on or off).".into());
                }
                Packet::ConnAck(rc) => return Err(format!("The printer refused the connection (code {rc}).")),
                Packet::Publish { topic, payload } if topic == report_topic => {
                    let Ok(j) = Json::parse(&String::from_utf8_lossy(&payload)) else { continue };
                    let Some(print) = j.get("print") else { continue };
                    let mut merged = Json::obj();
                    set_live(&cfg.id, |l| {
                        if !matches!(l.report, Json::Obj(_)) {
                            l.report = Json::obj();
                        }
                        merge(&mut l.report, print);
                        l.state = "online".into();
                        l.message = None;
                        l.updated_at = now_ms();
                        merged = l.report.clone();
                    });
                    if let Some(done) = tracker.update(&merged, now_ms()) {
                        locker(&mut |v: &Vault| {
                            if let Err(e) = record(v, cfg, &done) {
                                eprintln!("[printers] could not record print: {e}");
                            }
                        });
                    }
                }
                _ => {}
            }
        }
        if !connected && started.elapsed() > Duration::from_secs(15) {
            return Err("The printer did not answer. Check the IP address and that the printer is on.".into());
        }
        if connected && last_ping.elapsed() > Duration::from_secs(30) {
            s.write_all(&[0xC0, 0]).map_err(io_err)?;
            last_ping = Instant::now();
        }
        if last_rx.elapsed() > Duration::from_secs(120) {
            return Ok(()); // silent for too long; reconnect
        }
    }
}

// ---------------------------------------------------------------- tests

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{TcpListener, TcpStream};

    fn read_packet(s: &mut TcpStream, buf: &mut Vec<u8>) -> Option<(u8, Vec<u8>)> {
        let mut chunk = [0u8; 4096];
        loop {
            if buf.len() >= 2 {
                let (mut len, mut mul, mut i) = (0usize, 1usize, 1usize);
                loop {
                    let b = *buf.get(i)?;
                    len += (b & 0x7F) as usize * mul;
                    i += 1;
                    if b & 0x80 == 0 {
                        break;
                    }
                    mul *= 128;
                }
                if buf.len() >= i + len {
                    let kind = buf[0];
                    let body = buf[i..i + len].to_vec();
                    buf.drain(..i + len);
                    return Some((kind, body));
                }
            }
            let n = s.read(&mut chunk).ok()?;
            if n == 0 {
                return None;
            }
            buf.extend_from_slice(&chunk[..n]);
        }
    }

    /// A pretend printer: checks the login, then sends the scripted reports.
    fn fake_printer(code: &'static str, serial: &'static str, sessions: Vec<Vec<String>>) -> (u16, std::thread::JoinHandle<Vec<String>>) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let h = std::thread::spawn(move || {
            let mut seen = Vec::new();
            for reports in sessions {
                let (mut s, _) = l.accept().unwrap();
                let mut buf = Vec::new();
                let (kind, body) = read_packet(&mut s, &mut buf).unwrap();
                assert_eq!(kind, 0x10);
                let pass_ok = body.windows(code.len()).any(|w| w == code.as_bytes()) && body.windows(4).any(|w| w == b"bblp");
                s.write_all(&[0x20, 2, 0, if pass_ok { 0 } else { 5 }]).unwrap();
                if !pass_ok {
                    seen.push("refused".into());
                    continue;
                }
                let (kind, body) = read_packet(&mut s, &mut buf).unwrap();
                assert_eq!(kind, 0x82);
                seen.push(format!("sub {}", String::from_utf8_lossy(&body[4..body.len() - 1])));
                s.write_all(&[0x90, 3, 0, 1, 0]).unwrap();
                let (kind, body) = read_packet(&mut s, &mut buf).unwrap();
                assert_eq!(kind >> 4, 3);
                seen.push(if String::from_utf8_lossy(&body).contains("pushall") { "pushall".into() } else { "publish?".into() });
                for r in reports {
                    s.write_all(&publish_packet(&format!("device/{serial}/report"), r.as_bytes())).unwrap();
                    std::thread::sleep(Duration::from_millis(150));
                }
                std::thread::sleep(Duration::from_millis(300));
            }
            seen
        });
        (port, h)
    }

    fn rep(stage: &str, name: &str, pct: i64, start: i64, err: i64) -> String {
        format!(r#"{{"print":{{"command":"push_status","gcode_state":"{stage}","subtask_name":"{name}","gcode_file":"/data/Metadata/plate_1.gcode","mc_percent":{pct},"mc_remaining_time":42,"layer_num":10,"total_layer_num":200,"gcode_start_time":"{start}","print_error":{err},"nozzle_temper":219.6,"bed_temper":55.0}}}}"#)
    }

    fn wait_for(mut f: impl FnMut() -> bool) {
        let t = Instant::now();
        while !f() {
            assert!(t.elapsed() < Duration::from_secs(20), "timed out");
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    fn setup(port: u16, code: &str, serial: &str) -> (crate::testutil::TempDir, Arc<Mutex<Vault>>, String, Locker) {
        let (t, v) = crate::testutil::vault();
        let folder = t.0.join("Desk Cat");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("Desk Cat.3mf"), b"PK").unwrap();
        let pr = crate::api::dispatch(&v, "projects.create", &Json::obj().with("name", "Desk Cat").with("category", "3d-printing").with("linkPath", folder.to_string_lossy().to_string())).unwrap();
        let pid = pr.str_of("id").unwrap();
        crate::api::dispatch(&v, "files.scan", &Json::obj().with("projectId", pid.as_str())).ok();
        save(&v, &Json::obj().with("name", "X2D").with("host", format!("127.0.0.1:{port}")).with("serial", serial).with("accessCode", code)).unwrap();
        let v = Arc::new(Mutex::new(v));
        let vv = v.clone();
        let locker: Locker = Arc::new(move |f: &mut dyn FnMut(&Vault)| f(&vv.lock().unwrap()));
        (t, v, pid, locker)
    }

    #[test]
    fn mqtt_codec_round_trips() {
        let big = vec![b'x'; 300];
        let pkt = publish_packet("device/S/report", &big);
        let (p, used) = parse_packet(&pkt).unwrap().unwrap();
        assert_eq!(used, pkt.len());
        assert_eq!(p, Packet::Publish { topic: "device/S/report".into(), payload: big });
        assert_eq!(parse_packet(&pkt[..pkt.len() - 1]).unwrap(), None);
        assert_eq!(parse_packet(&[0x20, 2, 0, 5]).unwrap().unwrap().0, Packet::ConnAck(5));
    }

    #[test]
    fn tracker_reports_each_job_once() {
        let mut t = Tracker::default();
        let r = |s: &str, e: i64| Json::parse(&rep(s, "Desk Cat_plate_1", 50, 1_790_000_000, e)).unwrap().get("print").unwrap().clone();
        assert_eq!(t.update(&r("RUNNING", 0), 1), None);
        let f = t.update(&r("FINISH", 0), 2).unwrap();
        assert_eq!((f.result.as_str(), f.late, f.started_at), ("success", false, Some(1_790_000_000_000)));
        assert_eq!(t.update(&r("FINISH", 0), 3), None, "repeated reports of the same finish are not new jobs");
        let mut t2 = Tracker::default();
        let f = t2.update(&r("FAILED", 0x0300_4000), 5).unwrap();
        assert_eq!((f.result.as_str(), f.error.as_deref(), f.late), ("failure", Some("0300-4000"), true));
        let mut t3 = Tracker::default();
        t3.update(&r("RUNNING", 0), 1);
        assert_eq!(t3.update(&r("IDLE", 0), 2).unwrap().result, "stopped");
        assert_eq!(norm("Desk Cat_plate_12"), "desk cat");
        assert_eq!(utc_date(0), "1970-01-01");
        assert_eq!(utc_date(1_793_577_600_000), "2026-11-02");
        assert_eq!(utc_date(1_709_164_800_000), "2024-02-29");
    }

    #[test]
    fn records_a_finished_print_into_the_matching_project() {
        let serial = "TESTSERIAL01";
        let began = now_ms() / 1000 - 3600;
        let s1 = vec![rep("RUNNING", "Desk Cat", 40, began, 0), rep("FINISH", "Desk Cat", 100, began, 0)];
        // after a reconnect the printer still shows the same finished job
        let s2 = vec![rep("FINISH", "Desk Cat", 100, began, 0)];
        let (port, fake) = fake_printer("12345678", serial, vec![s1, s2]);
        let (_t, v, pid, locker) = setup(port, "12345678", serial);
        let stop = start(plain_tcp_connector(), locker.clone());
        wait_for(|| history_json(&v.lock().unwrap(), 10).unwrap().as_arr().map(|a| !a.is_empty()).unwrap_or(false));
        // live status was online and parsed
        wait_for(|| list(&v.lock().unwrap()).unwrap().as_arr().unwrap()[0].str_of("state").as_deref() != Some("connecting"));
        let seen = fake.join().unwrap();
        assert_eq!(seen[..2], [format!("sub device/{serial}/report"), "pushall".to_string()]);
        std::thread::sleep(Duration::from_millis(500));
        let vg = v.lock().unwrap();
        let h = history_json(&vg, 10).unwrap();
        let a = h.as_arr().unwrap();
        assert_eq!(a.len(), 1, "one job, recorded once across the reconnect: {}", h.to_string());
        assert_eq!(a[0].str_of("projectId").as_deref(), Some(pid.as_str()));
        assert_eq!(a[0].str_of("result").as_deref(), Some("success"));
        let p = crate::api::dispatch(&vg, "projects.get", &Json::obj().with("id", pid.as_str())).unwrap();
        let log = p.get("module").unwrap().get("printLog").unwrap().as_arr().unwrap().clone();
        assert_eq!(log.len(), 1);
        assert_eq!(log[0].str_of("result").as_deref(), Some("success"));
        assert!(log[0].str_of("notes").unwrap().contains("Desk Cat on X2D"));
        assert!(log[0].str_of("time").unwrap().starts_with("1h"), "{}", log[0].to_string());
        let tl = vg.db.one("SELECT summary FROM timeline WHERE project_id=? AND kind='print'", p![pid.as_str()]).unwrap().unwrap();
        assert_eq!(tl.s("summary"), "Printed Desk Cat");
        drop(vg);
        stop.store(true, Ordering::SeqCst);
    }

    #[test]
    fn wrong_access_code_is_explained_and_unmatched_prints_can_be_assigned() {
        let serial = "TESTSERIAL02";
        let (port, fake) = fake_printer("right-code", serial, vec![vec![]]);
        let (_t, v, pid, locker) = setup(port, "wrong-code", serial);
        let stop = start(plain_tcp_connector(), locker);
        wait_for(|| list(&v.lock().unwrap()).unwrap().as_arr().unwrap().iter().any(|p| p.str_of("message").unwrap_or_default().contains("access code")));
        assert_eq!(fake.join().unwrap(), vec!["refused".to_string()]);
        stop.store(true, Ordering::SeqCst);

        // a job whose name matches no file stays unassigned until assigned
        let vg = v.lock().unwrap();
        let printer = configs(&vg).unwrap().remove(0);
        let f = Finished { key: "Benchy|1".into(), name: "Benchy".into(), file: String::new(), result: "failure".into(), error: Some("0300-4000".into()), started_at: Some(now_ms() - 600_000), ended_at: now_ms(), late: false };
        assert!(record(&vg, &printer, &f).unwrap());
        assert!(!record(&vg, &printer, &f).unwrap(), "same job again is ignored");
        assert_eq!(history_json(&vg, 10).unwrap().as_arr().unwrap()[0].get("projectId"), None);
        assign(&vg, "Benchy|1", Some(&pid), true).unwrap();
        assert_eq!(match_project(&vg, "benchy_plate_3").unwrap().as_deref(), Some(pid.as_str()), "remembered for next time");
        let p = crate::api::dispatch(&vg, "projects.get", &Json::obj().with("id", pid.as_str())).unwrap();
        let log = p.get("module").unwrap().get("printLog").unwrap().as_arr().unwrap().clone();
        assert!(log[0].str_of("notes").unwrap().contains("error 0300-4000"));
        assign(&vg, "Benchy|1", Some(&pid), false).unwrap();
        assert_eq!(crate::api::dispatch(&vg, "projects.get", &Json::obj().with("id", pid.as_str())).unwrap().get("module").unwrap().get("printLog").unwrap().as_arr().unwrap().len(), 1, "assigning twice logs once");
        // stale finish screens (job started long ago) are not logged
        let old = Finished { key: "Old|1".into(), started_at: Some(now_ms() - 30 * 86_400_000), late: true, ..f.clone() };
        assert!(!record(&vg, &printer, &old).unwrap());
    }

}

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

/// "FFFFFFFF" / "#ffffff" → "#FFFFFF" (alpha dropped); None if not a colour.
fn hex_color(s: &str) -> Option<String> {
    let h = s.trim().trim_start_matches('#');
    (h.len() >= 6 && h[..6].chars().all(|c| c.is_ascii_hexdigit())).then(|| format!("#{}", h[..6].to_uppercase()))
}

fn int_of(j: &Json, k: &str) -> Option<i64> {
    match j.get(k)? {
        Json::Num(n) => Some(*n as i64),
        Json::Str(s) => s.trim().parse().ok(),
        _ => None,
    }
}

/// Spools the printer reports: AMS slots (ams.ams[].tray[]) and external
/// spools (vt_tray, or vir_slot on dual-nozzle printers). Each has a stable
/// key ("a0.2", "ext0"), type, brand, colour, remaining % (Bambu RFID spools
/// only) and whether it's feeding now.
pub fn spools(r: &Json) -> Vec<Json> {
    let mut active: Vec<String> = Vec::new();
    let ams = r.get("ams");
    // single-nozzle printers: ams.tray_now = unit*4 + slot (254 external, 255 none)
    if let Some(n) = ams.and_then(|a| int_of(a, "tray_now")) {
        match n {
            0..=127 => active.push(format!("a{}.{}", n / 4, n % 4)),
            254 => active.push("ext0".into()),
            _ => {}
        }
    }
    // dual-nozzle printers (H2D, X2D): device.extruder.info[].snow = (ams id << 8) | slot
    if let Some(list) = r.get("device").and_then(|d| d.get("extruder")).and_then(|e| e.get("info")).and_then(|i| i.as_arr()) {
        for e in list {
            if let Some(snow) = int_of(e, "snow") {
                let (unit, slot) = (snow >> 8, snow & 0xFF);
                if unit == 255 && slot != 255 {
                    active.push(format!("ext{}", slot.min(1)));
                } else if unit < 255 && slot < 255 {
                    active.push(format!("a{unit}.{slot}"));
                }
            }
        }
    }
    let mut out = Vec::new();
    let mut push = |key: String, label: String, t: &Json| {
        let ty = t.str_of("tray_type").unwrap_or_default();
        if ty.trim().is_empty() {
            return; // empty slot
        }
        let remain = int_of(t, "remain").filter(|n| (0..=100).contains(n));
        out.push(
            Json::obj()
                .with("key", key.as_str())
                .with("slot", label)
                .with("type", ty.trim())
                .with("brand", t.str_of("tray_sub_brands").map(|b| b.trim().to_string()).filter(|b| !b.is_empty()))
                .with("color", t.str_of("tray_color").and_then(|c| hex_color(&c)))
                .with("remain", remain)
                .with("weight", int_of(t, "tray_weight").filter(|w| *w > 0))
                .with("active", active.contains(&key)),
        );
    };
    for unit in ams.and_then(|a| a.get("ams")).and_then(|a| a.as_arr()).cloned().unwrap_or_default() {
        let uid = int_of(&unit, "id").unwrap_or(0);
        // AMS HT units report ids from 128
        let name = if uid >= 128 { format!("AMS HT {}", uid - 127) } else { format!("AMS {}", uid + 1) };
        for t in unit.get("tray").and_then(|t| t.as_arr()).cloned().unwrap_or_default() {
            let sid = int_of(&t, "id").unwrap_or(0);
            push(format!("a{uid}.{sid}"), if uid >= 128 { name.clone() } else { format!("{name} · slot {}", sid + 1) }, &t);
        }
    }
    if let Some(slots) = r.get("vir_slot").and_then(|v| v.as_arr()) {
        for (i, t) in slots.iter().enumerate() {
            push(format!("ext{i}"), if slots.len() > 1 { format!("External spool {}", i + 1) } else { "External spool".into() }, t);
        }
    } else if let Some(t) = r.get("vt_tray") {
        push("ext0".into(), "External spool".into(), t);
    }
    out
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
        j.set("spools", Json::Arr(spools(r)));
    }
    j
}

/// The printer's last full report, for troubleshooting (no access code in it).
pub fn raw(v: &Vault, id: &str) -> DbResult<Json> {
    configs(v)?.iter().find(|p| p.id == id).ok_or("Printer not found")?;
    let m = live().lock().unwrap_or_else(|e| e.into_inner());
    Ok(m.get(id).map(|l| l.report.clone()).unwrap_or(Json::obj()))
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
    /// Spools used: type, brand, colour, and grams where the spool's level is known.
    pub filaments: Vec<Json>,
}

#[derive(Default)]
pub struct Tracker {
    running: Option<(String, String, String, Option<i64>)>, // key, name, file, start
    last_final: Option<String>,
    /// spool levels when the job started, and every spool seen feeding during it
    start_spools: Vec<Json>,
    used: std::collections::BTreeMap<String, Json>,
    /// first seen already well under way (app opened mid-print): the starting
    /// spool levels are unknown, so measured grams would undercount
    joined_late: bool,
}

impl Tracker {
    fn note_spools(&mut self, r: &Json) {
        for sp in spools(r) {
            if sp.get("active").and_then(|a| a.as_bool()) == Some(true) {
                self.used.insert(sp.str_of("key").unwrap_or_default(), sp);
            }
        }
    }

    /// What this job used: spools seen feeding, plus spools whose level dropped.
    fn usage(&mut self, r: &Json) -> Vec<Json> {
        let now = spools(r);
        let mut out = Vec::new();
        let mut keys: Vec<String> = self.used.keys().cloned().collect();
        for sp in &self.start_spools {
            let k = sp.str_of("key").unwrap_or_default();
            let before = sp.get("remain").and_then(|x| x.as_i64());
            let after = now.iter().find(|n| n.str_of("key").as_deref() == Some(k.as_str())).and_then(|n| n.get("remain")).and_then(|x| x.as_i64());
            if matches!((before, after), (Some(b), Some(a)) if a < b) && !keys.contains(&k) {
                keys.push(k);
            }
        }
        for k in keys {
            let start = self.start_spools.iter().find(|x| x.str_of("key").as_deref() == Some(k.as_str()));
            let end = now.iter().find(|x| x.str_of("key").as_deref() == Some(k.as_str()));
            let Some(info) = end.or(start).or_else(|| self.used.get(&k)) else { continue };
            let grams = if self.joined_late { None } else { match (start.and_then(|s| s.get("remain")).and_then(|x| x.as_i64()), end.and_then(|e| e.get("remain")).and_then(|x| x.as_i64()), info.get("weight").and_then(|w| w.as_i64())) {
                (Some(b), Some(a), Some(w)) if b > a => Some(((b - a) as f64 * w as f64 / 100.0).round() as i64),
                _ => None,
            } };
            out.push(Json::obj().with("type", info.str_of("type")).with("brand", info.str_of("brand")).with("color", info.str_of("color")).with("slot", info.str_of("slot")).with("grams", grams));
        }
        self.used.clear();
        self.start_spools.clear();
        out
    }
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
                    self.start_spools = spools(r);
                    self.used.clear();
                    self.joined_late = num(r, "mc_percent").unwrap_or(0.0) > 2.0;
                }
                self.note_spools(r);
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
                let filaments = if late { Vec::new() } else { self.usage(r) };
                Some(Finished {
                    key,
                    name,
                    file,
                    result: if ok { "success" } else { "failure" }.into(),
                    error: error_code(r),
                    started_at: start,
                    ended_at: now,
                    late,
                    filaments,
                })
            }
            "IDLE" => {
                // went idle without finishing: stopped from the printer
                let (key, name, file, start) = self.running.take()?;
                self.last_final = Some(key.clone());
                let filaments = self.usage(r);
                Some(Finished { key, name, file, result: "stopped".into(), error: error_code(r), started_at: start, ended_at: now, late: false, filaments })
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
    let fils = h.get("filaments").and_then(|x| x.as_arr()).cloned().unwrap_or_default();
    let fils = if fils.is_empty() { h.get("estimatedFilaments").and_then(|x| x.as_arr()).cloned().unwrap_or_default() } else { fils };
    let fil_text: Vec<String> = fils.iter().map(|x| x.str_of("brand").or_else(|| x.str_of("type")).unwrap_or_default()).filter(|t| !t.is_empty()).collect();
    if !fil_text.is_empty() {
        let mut uniq = fil_text.clone();
        uniq.dedup();
        notes.push_str(&format!(" · {}", uniq.join(", ")));
    }
    let grams = h.get("grams").and_then(|g| g.as_i64()).map(|g| g.to_string())
        .or_else(|| h.get("estimatedGrams").and_then(|g| g.as_f64()).map(|g| format!("~{}", g.round() as i64)))
        .unwrap_or_default();
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
            .with("grams", grams)
            .with("filaments", Json::Arr(fils.clone()))
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

/// The slicer's filament estimate for this job, from the project's sliced
/// .3mf with the job's name (plate from the printer's gcode file name).
fn sliced_estimate(v: &Vault, pid: &str, f: &Finished) -> Option<(f64, Vec<Json>)> {
    let n = norm(&f.name);
    let rows = v.db.query("SELECT f.rel_path, f.name, p.root_path FROM files f JOIN projects p ON p.id=f.project_id WHERE f.project_id=? AND f.ext='3mf' AND f.state='present'", p![pid]).ok()?;
    let row = rows.into_iter().find(|r| {
        let l = r.s("name").to_lowercase();
        norm(l.strip_suffix(".gcode.3mf").or_else(|| l.strip_suffix(".3mf")).unwrap_or(&l)) == n
    })?;
    let path = crate::util::safe_join(std::path::Path::new(&row.s("root_path")), &row.s("rel_path")).ok()?;
    let info = crate::models::threemf_info(&path).ok()?;
    let plates = info.get("plates")?.as_arr()?.clone();
    // "/data/Metadata/plate_2.gcode" → 2
    let want = f.file.rsplit(['/', '\\']).next().and_then(|x| x.strip_prefix("plate_")).and_then(|x| x.split('.').next()).and_then(|x| x.parse::<i64>().ok());
    let plate = match want {
        Some(i) => plates.iter().find(|p| p.get("index").and_then(|x| x.as_i64()) == Some(i)).cloned(),
        None if plates.len() == 1 => plates.first().cloned(),
        None => None,
    }?;
    let fils: Vec<Json> = plate.get("filaments").and_then(|x| x.as_arr()).cloned().unwrap_or_default().into_iter()
        .filter(|x| x.get("usedG").and_then(|g| g.as_f64()).unwrap_or(0.0) > 0.0)
        .map(|x| Json::obj().with("type", x.str_of("type")).with("color", x.str_of("color").and_then(|c| hex_color(&c))).with("grams", (x.get("usedG").and_then(|g| g.as_f64()).unwrap_or(0.0) * 10.0).round() / 10.0))
        .collect();
    let total = plate.get("weightG").and_then(|x| x.as_f64()).filter(|w| *w > 0.0).unwrap_or_else(|| fils.iter().filter_map(|x| x.get("grams").and_then(|g| g.as_f64())).sum());
    (total > 0.0).then_some(((total * 10.0).round() / 10.0, fils))
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
    let measured: i64 = f.filaments.iter().filter_map(|x| x.get("grams").and_then(|g| g.as_i64())).sum();
    let estimate = pid.as_deref().and_then(|p| sliced_estimate(v, p, f));
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
        .with("filaments", Json::Arr(f.filaments.clone()))
        .with("grams", (measured > 0).then_some(measured))
        .with("estimatedGrams", estimate.as_ref().map(|e| e.0))
        .with("estimatedFilaments", estimate.map(|e| Json::Arr(e.1)))
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

// ---------------------------------------------------------------- discovery

/// Bambu printers announce themselves every few seconds with an SSDP NOTIFY
/// (from port 1900 to ports 2021 and 1990; broadcast and 239.255.255.250)
/// carrying their IP (`Location`) and serial (`USN`). Mothball listens so a
/// printer whose address changed (DHCP) is found again by its serial, and so
/// printers not added yet can be offered in Settings.
pub const DISCOVERY_PORTS: [u16; 2] = [2021, 1990];

#[derive(Clone, Debug, PartialEq)]
pub struct Found {
    pub serial: String,
    pub ip: String,
    pub name: String,
    pub model: String,
    pub at: i64,
}

fn found() -> &'static Mutex<HashMap<String, Found>> {
    static F: std::sync::OnceLock<Mutex<HashMap<String, Found>>> = std::sync::OnceLock::new();
    F.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Reads a printer announcement; None for anything else.
pub fn parse_announcement(msg: &str, sender: std::net::IpAddr) -> Option<Found> {
    let mut h: HashMap<String, String> = HashMap::new();
    let mut lines = msg.lines();
    let first = lines.next()?.trim().to_uppercase();
    if !(first.starts_with("NOTIFY") || first.starts_with("HTTP/1.1 200")) {
        return None;
    }
    for l in lines {
        if let Some((k, v)) = l.split_once(':') {
            h.insert(k.trim().to_lowercase(), v.trim().to_string());
        }
    }
    let kind = h.get("nt").or(h.get("st"))?;
    if !kind.contains("bambulab-com:device:3dprinter") {
        return None;
    }
    let serial = h.get("usn")?.trim().to_uppercase();
    if serial.is_empty() || serial.len() > 64 {
        return None;
    }
    // the printer's own address; fall back to where the packet came from
    let ip = h.get("location").and_then(|l| l.trim().parse::<std::net::Ipv4Addr>().ok()).map(|a| a.to_string()).unwrap_or_else(|| sender.to_string());
    Some(Found {
        serial,
        ip,
        name: h.get("devname.bambu.com").cloned().unwrap_or_default(),
        model: h.get("devmodel.bambu.com").cloned().unwrap_or_default(),
        at: now_ms(),
    })
}

/// If a configured printer with this serial is at a different address now,
/// switch to it. Its pinned certificate stays: a device that merely claims the
/// serial still has to present the same certificate.
pub fn follow_address(v: &Vault, f: &Found) -> DbResult<bool> {
    let mut list = configs(v)?;
    let Some(p) = list.iter_mut().find(|p| p.serial.eq_ignore_ascii_case(&f.serial)) else { return Ok(false) };
    if p.host.contains(':') || p.host == f.ip {
        return Ok(false); // an explicit host:port (tests, port forwarding) is left alone
    }
    let (id, old) = (p.id.clone(), p.host.clone());
    p.host = f.ip.clone();
    save_configs(v, &list)?;
    set_live(&id, |l| {
        l.state = "connecting".into();
        l.message = Some(format!("Found at a new address ({} → {}); reconnecting.", old, f.ip));
    });
    Ok(true)
}

/// Printers seen on the network recently, and whether each is already added.
pub fn found_json(v: &Vault) -> DbResult<Json> {
    let cfg = configs(v)?;
    let cutoff = now_ms() - 10 * 60_000;
    let m = found().lock().unwrap_or_else(|e| e.into_inner());
    let mut out: Vec<Json> = m.values().filter(|f| f.at > cutoff).map(|f| {
        Json::obj().with("serial", f.serial.as_str()).with("ip", f.ip.as_str()).with("name", f.name.as_str()).with("model", f.model.as_str()).with("seenAt", f.at)
            .with("added", cfg.iter().any(|p| p.serial.eq_ignore_ascii_case(&f.serial)))
    }).collect();
    out.sort_by(|a, b| a.str_of("name").cmp(&b.str_of("name")));
    Ok(Json::Arr(out))
}

/// Listens on the given ports (whichever are free) until `stop` is set.
pub fn start_discovery(ports: &[u16], locker: Locker, stop: Arc<AtomicBool>) -> usize {
    let mut listening = 0;
    for &port in ports {
        let Ok(sock) = std::net::UdpSocket::bind(("0.0.0.0", port)) else { continue }; // e.g. Bambu Studio has it
        let _ = sock.join_multicast_v4(&std::net::Ipv4Addr::new(239, 255, 255, 250), &std::net::Ipv4Addr::UNSPECIFIED);
        let _ = sock.set_read_timeout(Some(Duration::from_millis(500)));
        listening += 1;
        let (l, stop) = (locker.clone(), stop.clone());
        std::thread::spawn(move || {
            let mut buf = [0u8; 2048];
            while !stop.load(Ordering::SeqCst) {
                let Ok((n, from)) = sock.recv_from(&mut buf) else { continue };
                let Some(f) = parse_announcement(&String::from_utf8_lossy(&buf[..n]), from.ip()) else { continue };
                let moved = {
                    let mut m = found().lock().unwrap_or_else(|e| e.into_inner());
                    let prev = m.insert(f.serial.clone(), f.clone());
                    prev.map(|p| p.ip != f.ip).unwrap_or(true)
                };
                if moved {
                    l(&mut |v: &Vault| {
                        let _ = follow_address(v, &f);
                    });
                }
            }
        });
    }
    listening
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
    start_discovery(&DISCOVERY_PORTS, locker.clone(), shutdown.clone());
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

    fn ams_report(stage: &str, remain0: i64, remain1: i64, tray_now: &str) -> Json {
        Json::parse(&format!(r#"{{"gcode_state":"{stage}","subtask_name":"Dry Pods","gcode_file":"/data/Metadata/plate_2.gcode","gcode_start_time":"1790000000",
          "ams":{{"tray_now":"{tray_now}","ams":[{{"id":"0","tray":[
            {{"id":"0","tray_type":"PLA","tray_sub_brands":"PLA Basic","tray_color":"FF6A13FF","remain":{remain0},"tray_weight":"1000"}},
            {{"id":"1","tray_type":"PETG","tray_sub_brands":"PETG HF","tray_color":"000000FF","remain":{remain1},"tray_weight":"1000"}},
            {{"id":"2","tray_type":"","tray_color":"00000000"}},
            {{"id":"3","tray_type":"PLA","tray_color":"FFFFFFFF","remain":-1}}]}}]}},
          "vt_tray":{{"id":"254","tray_type":"TPU","tray_color":"2850E0FF","remain":-1}}}}"#)).unwrap()
    }

    #[test]
    fn reads_loaded_spools_and_what_a_print_used() {
        let r = ams_report("RUNNING", 80, 50, "0");
        let sp = spools(&r);
        assert_eq!(sp.len(), 4, "empty slot skipped: {}", Json::Arr(sp.clone()).to_string());
        assert_eq!(sp[0].str_of("slot").as_deref(), Some("AMS 1 · slot 1"));
        assert_eq!(sp[0].str_of("color").as_deref(), Some("#FF6A13"));
        assert_eq!(sp[0].get("active").and_then(|a| a.as_bool()), Some(true));
        assert_eq!(sp[2].get("remain"), None, "non-RFID spool: level unknown");
        assert_eq!(sp[3].str_of("slot").as_deref(), Some("External spool"));
        // dual-nozzle printers say which slot feeds each nozzle
        let mut dual = ams_report("RUNNING", 80, 50, "255");
        dual.set("device", Json::parse(r#"{"extruder":{"info":[{"id":0,"snow":1},{"id":1,"snow":65280}]}}"#).unwrap());
        let act: Vec<String> = spools(&dual).iter().filter(|x| x.get("active").and_then(|a| a.as_bool()) == Some(true)).filter_map(|x| x.str_of("key")).collect();
        assert_eq!(act, vec!["a0.1".to_string(), "ext0".to_string()]);

        // a job: starts on slot 1, switches to slot 2; levels drop 80→74 and 50→48
        let mut t = Tracker::default();
        assert_eq!(t.update(&ams_report("RUNNING", 80, 50, "0"), 1), None);
        t.update(&ams_report("RUNNING", 77, 50, "1"), 2);
        let f = t.update(&ams_report("FINISH", 74, 48, "255"), 3).unwrap();
        let used: Vec<(String, Option<i64>)> = f.filaments.iter().map(|x| (x.str_of("brand").unwrap_or_default(), x.get("grams").and_then(|g| g.as_i64()))).collect();
        assert_eq!(used, vec![("PLA Basic".to_string(), Some(60)), ("PETG HF".to_string(), Some(20))]);

        // joined mid-print (app opened at 40%): spools are listed, grams aren't guessed
        let mut late = Tracker::default();
        let mut mid = ams_report("RUNNING", 77, 50, "0");
        mid.set("mc_percent", 40);
        late.update(&mid, 1);
        let f = late.update(&ams_report("FINISH", 74, 50, "255"), 2).unwrap();
        assert_eq!(f.filaments.len(), 1);
        assert_eq!(f.filaments[0].get("grams"), None);
    }

    #[test]
    fn slicer_estimate_comes_from_the_matching_plate() {
        let (t, v) = crate::testutil::vault();
        let folder = t.0.join("Dry Pods");
        let src = t.0.join("3mf-src/Metadata");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(src.join("slice_info.config"), r##"<config><plate><metadata key="index" value="1"/><metadata key="weight" value="5"/><filament id="1" type="PLA" color="#FFFFFF" used_g="5"/></plate><plate><metadata key="index" value="2"/><metadata key="weight" value="83.4"/><filament id="1" type="PLA" color="#FF6A13" used_g="61.2"/><filament id="2" type="PETG" color="#000000" used_g="22.2"/></plate></config>"##).unwrap();
        crate::zip::zip_dir(&t.0.join("3mf-src"), &folder.join("Dry Pods.3mf")).unwrap();
        let pid = crate::api::dispatch(&v, "projects.create", &Json::obj().with("name", "Dry Pods").with("category", "3d-printing").with("linkPath", folder.to_string_lossy().to_string())).unwrap().str_of("id").unwrap();
        crate::files::scan(&v, &pid).unwrap();
        save(&v, &Json::obj().with("name", "X2D").with("host", "127.0.0.1:1").with("serial", "S1").with("accessCode", "x")).unwrap();
        let printer = configs(&v).unwrap().remove(0);
        // no spool levels known (non-Bambu spools): only the slicer's estimate
        let f = Finished { key: "Dry Pods|1".into(), name: "Dry Pods".into(), file: "/data/Metadata/plate_2.gcode".into(), result: "success".into(), error: None, started_at: Some(now_ms() - 3_600_000), ended_at: now_ms(), late: false, filaments: vec![] };
        assert!(record(&v, &printer, &f).unwrap());
        let h = history_json(&v, 5).unwrap().as_arr().unwrap()[0].clone();
        assert_eq!(h.get("estimatedGrams").and_then(|g| g.as_f64()), Some(83.4));
        assert_eq!(h.get("estimatedFilaments").unwrap().as_arr().unwrap().len(), 2);
        let log = crate::projects::get(&v, &pid).unwrap().get("module").unwrap().get("printLog").unwrap().as_arr().unwrap()[0].clone();
        assert_eq!(log.str_of("grams").as_deref(), Some("~83"), "estimate marked with ~");
        assert!(log.str_of("notes").unwrap().contains("PLA, PETG"), "{}", log.to_string());
        assert_eq!(log.get("filaments").unwrap().as_arr().unwrap()[0].str_of("color").as_deref(), Some("#FF6A13"));
    }

    const ANNOUNCE: &str = "NOTIFY * HTTP/1.1\r\nHOST: 239.255.255.250:1900\r\nServer: UPnP/1.0\r\nLocation: 10.0.0.41\r\nNT: urn:bambulab-com:device:3dprinter:1\r\nUSN: 0938abc123\r\nCache-Control: max-age=1800\r\nDevModel.bambu.com: N6\r\nDevName.bambu.com: Big Boi\r\nDevConnect.bambu.com: lan\r\n\r\n";

    #[test]
    fn reads_printer_announcements_and_ignores_other_ssdp() {
        let from: std::net::IpAddr = "10.0.0.41".parse().unwrap();
        let f = parse_announcement(ANNOUNCE, from).unwrap();
        assert_eq!((f.serial.as_str(), f.ip.as_str(), f.name.as_str(), f.model.as_str()), ("0938ABC123", "10.0.0.41", "Big Boi", "N6"));
        let router = "NOTIFY * HTTP/1.1\r\nNT: urn:schemas-upnp-org:device:InternetGatewayDevice:1\r\nUSN: uuid:x\r\nLocation: http://10.0.0.1:5000/\r\n\r\n";
        assert_eq!(parse_announcement(router, from), None);
        let bad_loc = ANNOUNCE.replace("Location: 10.0.0.41", "Location: http://evil/");
        assert_eq!(parse_announcement(&bad_loc, "10.0.0.77".parse().unwrap()).unwrap().ip, "10.0.0.77", "falls back to the sender");
    }

    #[test]
    fn a_printer_that_changed_address_is_followed_and_keeps_its_certificate_pin() {
        let (_t, v) = crate::testutil::vault();
        save(&v, &Json::obj().with("name", "Big Boi").with("host", "10.0.0.38").with("serial", "0938abc123").with("accessCode", "x")).unwrap();
        let id = configs(&v).unwrap()[0].id.clone();
        pin_certificate(&v, &id, "ab12").unwrap();
        let v = Arc::new(Mutex::new(v));
        let vv = v.clone();
        let locker: Locker = Arc::new(move |f: &mut dyn FnMut(&Vault)| f(&vv.lock().unwrap()));
        // listen on a free port and announce to it, like the printer would
        let port = { let s = std::net::UdpSocket::bind("127.0.0.1:0").unwrap(); s.local_addr().unwrap().port() };
        let stop = Arc::new(AtomicBool::new(false));
        assert_eq!(start_discovery(&[port], locker, stop.clone()), 1);
        let tx = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let t0 = Instant::now();
        loop {
            tx.send_to(ANNOUNCE.as_bytes(), ("127.0.0.1", port)).unwrap();
            std::thread::sleep(Duration::from_millis(100));
            let p = configs(&v.lock().unwrap()).unwrap().remove(0);
            if p.host == "10.0.0.41" {
                assert_eq!(p.cert_sha256.as_deref(), Some("ab12"), "the pin survives the move");
                break;
            }
            assert!(t0.elapsed() < Duration::from_secs(10), "address never followed");
        }
        let f = found_json(&v.lock().unwrap()).unwrap();
        assert!(f.as_arr().unwrap().iter().any(|x| x.str_of("serial").as_deref() == Some("0938ABC123") && x.get("added").and_then(|a| a.as_bool()) == Some(true)));
        stop.store(true, Ordering::SeqCst);
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
        let f = Finished { key: "Benchy|1".into(), name: "Benchy".into(), file: String::new(), result: "failure".into(), error: Some("0300-4000".into()), started_at: Some(now_ms() - 600_000), ended_at: now_ms(), late: false, filaments: vec![] };
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

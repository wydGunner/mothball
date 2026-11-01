//! Small std-only utilities: time, IDs, SHA-256, path helpers.

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Milliseconds since the Unix epoch.
pub fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

pub fn system_time_ms(t: SystemTime) -> i64 {
    match t.duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_millis() as i64,
        Err(e) => -(e.duration().as_millis() as i64),
    }
}

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// 128-bit random-ish identifier rendered as UUID v4 text.
///
/// Entropy comes from std's per-process randomly seeded SipHash keys, mixed
/// with time and a monotonically increasing counter, so IDs are unique within
/// a process and collision-resistant across processes. IDs are never derived
/// from paths: a file's identity survives renames and moves.
pub fn new_id() -> String {
    let c = COUNTER.fetch_add(1, Ordering::Relaxed);
    let t = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let mut parts = [0u64; 2];
    for (i, p) in parts.iter_mut().enumerate() {
        let mut h = RandomState::new().build_hasher();
        h.write_u128(t);
        h.write_u64(c);
        h.write_u64(i as u64);
        h.write_u32(std::process::id());
        *p = h.finish();
    }
    let mut b = [0u8; 16];
    b[..8].copy_from_slice(&parts[0].to_le_bytes());
    b[8..].copy_from_slice(&parts[1].to_le_bytes());
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
}

/// `2026-11-01 20:37 UTC` from Unix milliseconds (civil-from-days, Howard Hinnant).
pub fn iso_utc(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    let (days, rem) = (secs.div_euclid(86400), secs.rem_euclid(86400));
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02} UTC", rem / 3600, rem % 3600 / 60)
}

// ---------------------------------------------------------------- SHA-256

const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

pub struct Sha256 {
    h: [u32; 8],
    buf: [u8; 64],
    len: usize,
    total: u64,
}

impl Default for Sha256 {
    fn default() -> Self {
        Self::new()
    }
}

impl Sha256 {
    pub fn new() -> Self {
        Sha256 {
            h: [
                0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
                0x5be0cd19,
            ],
            buf: [0; 64],
            len: 0,
            total: 0,
        }
    }
    fn block(&mut self, blk: &[u8; 64]) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([blk[4 * i], blk[4 * i + 1], blk[4 * i + 2], blk[4 * i + 3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = self.h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = h.wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (x, y) in self.h.iter_mut().zip([a, b, c, d, e, f, g, h]) {
            *x = x.wrapping_add(y);
        }
    }
    pub fn update(&mut self, mut data: &[u8]) {
        self.total += data.len() as u64;
        if self.len > 0 {
            let n = (64 - self.len).min(data.len());
            self.buf[self.len..self.len + n].copy_from_slice(&data[..n]);
            self.len += n;
            data = &data[n..];
            if self.len == 64 {
                let b = self.buf;
                self.block(&b);
                self.len = 0;
            }
        }
        while data.len() >= 64 {
            let mut b = [0u8; 64];
            b.copy_from_slice(&data[..64]);
            self.block(&b);
            data = &data[64..];
        }
        if !data.is_empty() {
            self.buf[..data.len()].copy_from_slice(data);
            self.len = data.len();
        }
    }
    pub fn hex(mut self) -> String {
        let bits = self.total.wrapping_mul(8);
        let mut pad = vec![0x80u8];
        while (self.len + pad.len()) % 64 != 56 {
            pad.push(0);
        }
        let total = self.total;
        self.update(&pad);
        self.update(&bits.to_be_bytes());
        self.total = total;
        self.h.iter().map(|x| format!("{x:08x}")).collect()
    }
}

pub fn sha256_bytes(b: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(b);
    h.hex()
}

pub fn sha256_file(p: &Path) -> std::io::Result<String> {
    let mut f = std::fs::File::open(p)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.hex())
}

// ---------------------------------------------------------------- paths

/// Converts a filesystem path relative to `root` into the canonical stored
/// form: forward slashes, no `.`/`..`, no leading slash.
pub fn rel_path(root: &Path, p: &Path) -> Option<String> {
    let r = p.strip_prefix(root).ok()?;
    let mut parts = Vec::new();
    for c in r.components() {
        match c {
            Component::Normal(s) => parts.push(s.to_string_lossy().to_string()),
            Component::CurDir => {}
            _ => return None,
        }
    }
    Some(parts.join("/"))
}

/// Joins a stored relative path onto a root, refusing anything that could
/// escape the root (absolute paths, `..`, drive prefixes). This is the single
/// gate every user- or archive-supplied relative path must go through.
pub fn safe_join(root: &Path, rel: &str) -> Result<PathBuf, String> {
    if rel.contains('\0') {
        return Err("path contains NUL".into());
    }
    let mut out = root.to_path_buf();
    for seg in rel.split(['/', '\\']) {
        match seg {
            "" | "." => continue,
            ".." => return Err(format!("path escapes project root: {rel}")),
            s if s.contains(':') => return Err(format!("invalid path segment: {s}")),
            s => out.push(s),
        }
    }
    if !out.starts_with(root) {
        return Err(format!("path escapes project root: {rel}"));
    }
    Ok(out)
}

/// A destination path that does not exist yet: `name.ext`, `name (2).ext`, ...
/// Used so imports and restores never overwrite an existing file.
pub fn unique_dest(dir: &Path, file_name: &str) -> PathBuf {
    let cand = dir.join(file_name);
    if !cand.exists() {
        return cand;
    }
    let (stem, ext) = match file_name.rfind('.') {
        Some(i) if i > 0 => (&file_name[..i], &file_name[i..]),
        _ => (file_name, ""),
    };
    for n in 2.. {
        let c = dir.join(format!("{stem} ({n}){ext}"));
        if !c.exists() {
            return c;
        }
    }
    unreachable!()
}

/// Writes a file atomically: write to a sibling temp file, fsync, rename.
pub fn atomic_write(path: &Path, data: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(
        ".{}.tmp-{}",
        path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default(),
        &new_id()[..8]
    ));
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(data)?;
        f.sync_all()?;
    }
    match std::fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

pub fn slugify(s: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for c in s.chars() {
        if c.is_alphanumeric() {
            out.extend(c.to_lowercase());
            dash = false;
        } else if !dash && !out.is_empty() {
            out.push('-');
            dash = true;
        }
    }
    let t = out.trim_end_matches('-').to_string();
    if t.is_empty() { "project".into() } else { t }
}

pub fn extension_of(name: &str) -> String {
    match name.rfind('.') {
        Some(i) if i > 0 && i + 1 < name.len() => name[i + 1..].to_lowercase(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sha256_vectors() {
        assert_eq!(sha256_bytes(b""), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        assert_eq!(sha256_bytes(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        let long = b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq";
        assert_eq!(sha256_bytes(long), "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1");
        let mut h = Sha256::new();
        for _ in 0..1000 {
            h.update(&[b'a'; 1000]);
        }
        assert_eq!(h.hex(), "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0");
    }
    #[test]
    fn ids_unique_and_shaped() {
        let mut s = std::collections::HashSet::new();
        for _ in 0..10000 {
            let id = new_id();
            assert_eq!(id.len(), 36);
            assert_eq!(&id[14..15], "4");
            assert!(s.insert(id));
        }
    }
    #[test]
    fn safe_join_blocks_traversal() {
        let root = Path::new("/r");
        assert!(safe_join(root, "../x").is_err());
        assert!(safe_join(root, "a/../../x").is_err());
        assert!(safe_join(root, "C:/Windows").is_err());
        assert!(safe_join(root, "a\\..\\..\\b").is_err());
        assert_eq!(safe_join(root, "/a/b").unwrap(), PathBuf::from("/r/a/b"));
        assert_eq!(safe_join(root, "a\\b.txt").unwrap(), PathBuf::from("/r/a/b.txt"));
    }
    #[test]
    fn iso() {
        assert_eq!(iso_utc(0), "1970-01-01 00:00 UTC");
        assert_eq!(iso_utc(1_793_565_420_000), "2026-11-01 20:37 UTC");
        assert_eq!(iso_utc(951_782_400_000), "2000-02-29 00:00 UTC");
    }
    #[test]
    fn misc() {
        assert_eq!(slugify("OMEN v2!"), "omen-v2");
        assert_eq!(extension_of("a.Tar.GZ"), "gz");
        assert_eq!(extension_of(".gitignore"), "");
    }
}

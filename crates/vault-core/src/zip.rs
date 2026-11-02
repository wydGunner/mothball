//! Minimal ZIP support with no dependencies.
//!
//! * Reading: central directory, STORE and DEFLATE entries (a small inflater
//!   following RFC 1951), CRC-32 checked on every read. Used for .3mf files.
//! * Writing: STORE entries (no compression) with CRC-32, used for backup
//!   exports. Archives larger than 4 GB (ZIP64) are refused with a clear error.

use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

// ------------------------------------------------------------------ CRC-32

fn crc_table() -> &'static [u32; 256] {
    static T: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();
    T.get_or_init(|| {
        let mut t = [0u32; 256];
        for (i, e) in t.iter_mut().enumerate() {
            let mut c = i as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            }
            *e = c;
        }
        t
    })
}

pub struct Crc32(u32);
impl Crc32 {
    pub fn new() -> Self {
        Crc32(0xFFFF_FFFF)
    }
    pub fn update(&mut self, data: &[u8]) {
        let t = crc_table();
        for &b in data {
            self.0 = t[((self.0 ^ b as u32) & 0xFF) as usize] ^ (self.0 >> 8);
        }
    }
    pub fn finish(&self) -> u32 {
        self.0 ^ 0xFFFF_FFFF
    }
}
impl Default for Crc32 {
    fn default() -> Self {
        Self::new()
    }
}
pub fn crc32(data: &[u8]) -> u32 {
    let mut c = Crc32::new();
    c.update(data);
    c.finish()
}

// ------------------------------------------------------------------ inflate

struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
    bit: u32,
    nbits: u32,
}
impl<'a> Bits<'a> {
    fn need(&mut self, n: u32) -> Result<u32, String> {
        while self.nbits < n {
            let b = *self.data.get(self.pos).ok_or("truncated deflate stream")? as u32;
            self.pos += 1;
            self.bit |= b << self.nbits;
            self.nbits += 8;
        }
        let v = self.bit & ((1u32 << n) - 1);
        self.bit >>= n;
        self.nbits -= n;
        Ok(v)
    }
}

struct Huff {
    count: [u16; 16],
    symbol: Vec<u16>,
}
impl Huff {
    fn new(lengths: &[u8]) -> Result<Huff, String> {
        let mut count = [0u16; 16];
        for &l in lengths {
            count[l as usize] += 1;
        }
        count[0] = 0;
        let mut offs = [0u16; 16];
        for i in 1..16 {
            offs[i] = offs[i - 1] + count[i - 1];
        }
        let mut symbol = vec![0u16; lengths.len()];
        for (s, &l) in lengths.iter().enumerate() {
            if l != 0 {
                symbol[offs[l as usize] as usize] = s as u16;
                offs[l as usize] += 1;
            }
        }
        Ok(Huff { count, symbol })
    }
    fn decode(&self, b: &mut Bits) -> Result<u16, String> {
        let (mut code, mut first, mut index) = (0i32, 0i32, 0i32);
        for len in 1..16 {
            code |= b.need(1)? as i32;
            let count = self.count[len] as i32;
            if code - count < first {
                return Ok(self.symbol[(index + (code - first)) as usize]);
            }
            index += count;
            first += count;
            first <<= 1;
            code <<= 1;
        }
        Err("invalid Huffman code".into())
    }
}

const LBASE: [u16; 29] = [3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227, 258];
const LEXT: [u8; 29] = [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0];
const DBASE: [u16; 30] = [1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577];
const DEXT: [u8; 30] = [0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13];

/// Decompresses a raw DEFLATE stream. `limit` caps the output size.
pub fn inflate(data: &[u8], limit: usize) -> Result<Vec<u8>, String> {
    let mut out: Vec<u8> = Vec::new();
    let mut b = Bits { data, pos: 0, bit: 0, nbits: 0 };
    loop {
        let last = b.need(1)?;
        let kind = b.need(2)?;
        match kind {
            0 => {
                b.bit = 0;
                b.nbits = 0;
                if b.pos + 4 > data.len() {
                    return Err("truncated stored block".into());
                }
                let len = u16::from_le_bytes([data[b.pos], data[b.pos + 1]]) as usize;
                b.pos += 4;
                let end = b.pos + len;
                if end > data.len() {
                    return Err("truncated stored block".into());
                }
                out.extend_from_slice(&data[b.pos..end]);
                b.pos = end;
            }
            1 | 2 => {
                let (lit, dist) = if kind == 1 {
                    let mut l = [0u8; 288];
                    for (i, x) in l.iter_mut().enumerate() {
                        *x = if i < 144 { 8 } else if i < 256 { 9 } else if i < 280 { 7 } else { 8 };
                    }
                    (Huff::new(&l)?, Huff::new(&[5u8; 30])?)
                } else {
                    let nlen = b.need(5)? as usize + 257;
                    let ndist = b.need(5)? as usize + 1;
                    let ncode = b.need(4)? as usize + 4;
                    const ORDER: [usize; 19] = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];
                    let mut cl = [0u8; 19];
                    for &o in ORDER.iter().take(ncode) {
                        cl[o] = b.need(3)? as u8;
                    }
                    let ch = Huff::new(&cl)?;
                    let mut lens = vec![0u8; nlen + ndist];
                    let mut i = 0;
                    while i < nlen + ndist {
                        let sym = ch.decode(&mut b)?;
                        match sym {
                            0..=15 => {
                                lens[i] = sym as u8;
                                i += 1;
                            }
                            16 => {
                                let prev = *lens.get(i.wrapping_sub(1)).ok_or("repeat with no previous length")?;
                                for _ in 0..3 + b.need(2)? {
                                    *lens.get_mut(i).ok_or("too many lengths")? = prev;
                                    i += 1;
                                }
                            }
                            17 => i += 3 + b.need(3)? as usize,
                            _ => i += 11 + b.need(7)? as usize,
                        }
                    }
                    if i > nlen + ndist {
                        return Err("too many lengths".into());
                    }
                    (Huff::new(&lens[..nlen])?, Huff::new(&lens[nlen..])?)
                };
                loop {
                    let sym = lit.decode(&mut b)?;
                    if sym < 256 {
                        out.push(sym as u8);
                    } else if sym == 256 {
                        break;
                    } else {
                        let s = (sym - 257) as usize;
                        if s >= 29 {
                            return Err("bad length symbol".into());
                        }
                        let len = LBASE[s] as usize + b.need(LEXT[s] as u32)? as usize;
                        let d = dist.decode(&mut b)? as usize;
                        if d >= 30 {
                            return Err("bad distance symbol".into());
                        }
                        let dd = DBASE[d] as usize + b.need(DEXT[d] as u32)? as usize;
                        if dd > out.len() {
                            return Err("distance too far back".into());
                        }
                        let start = out.len() - dd;
                        for k in 0..len {
                            let c = out[start + k];
                            out.push(c);
                        }
                    }
                    if out.len() > limit {
                        return Err("entry is larger than allowed".into());
                    }
                }
            }
            _ => return Err("invalid deflate block type".into()),
        }
        if out.len() > limit {
            return Err("entry is larger than allowed".into());
        }
        if last == 1 {
            return Ok(out);
        }
    }
}

// ------------------------------------------------------------------ reading

#[derive(Clone, Debug)]
pub struct Entry {
    pub name: String,
    pub method: u16,
    pub crc: u32,
    pub csize: u64,
    pub usize: u64,
    offset: u64,
}

pub struct Archive<R: Read + Seek> {
    r: R,
    pub entries: Vec<Entry>,
}

fn u16le(b: &[u8], i: usize) -> u16 {
    u16::from_le_bytes([b[i], b[i + 1]])
}
fn u32le(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}

impl Archive<std::fs::File> {
    pub fn open(path: &Path) -> Result<Self, String> {
        Archive::new(std::fs::File::open(path).map_err(|e| e.to_string())?)
    }
}

impl<R: Read + Seek> Archive<R> {
    pub fn new(mut r: R) -> Result<Self, String> {
        let len = r.seek(SeekFrom::End(0)).map_err(|e| e.to_string())?;
        let tail = len.min(65_557);
        r.seek(SeekFrom::Start(len - tail)).map_err(|e| e.to_string())?;
        let mut buf = vec![0u8; tail as usize];
        r.read_exact(&mut buf).map_err(|e| e.to_string())?;
        let eocd = (0..buf.len().saturating_sub(21)).rev().find(|&i| u32le(&buf, i) == 0x0605_4b50).ok_or("not a ZIP file")?;
        let n = u16le(&buf, eocd + 10) as usize;
        let cd_size = u32le(&buf, eocd + 12) as u64;
        let cd_off = u32le(&buf, eocd + 16) as u64;
        if cd_off == 0xFFFF_FFFF {
            return Err("ZIP64 archives are not supported".into());
        }
        r.seek(SeekFrom::Start(cd_off)).map_err(|e| e.to_string())?;
        let mut cd = vec![0u8; cd_size as usize];
        r.read_exact(&mut cd).map_err(|e| format!("central directory: {e}"))?;
        let mut entries = Vec::with_capacity(n);
        let mut p = 0;
        for _ in 0..n {
            if p + 46 > cd.len() || u32le(&cd, p) != 0x0201_4b50 {
                return Err("damaged central directory".into());
            }
            let nl = u16le(&cd, p + 28) as usize;
            let xl = u16le(&cd, p + 30) as usize;
            let cl = u16le(&cd, p + 32) as usize;
            let name = String::from_utf8_lossy(&cd[p + 46..(p + 46 + nl).min(cd.len())]).to_string();
            entries.push(Entry {
                name,
                method: u16le(&cd, p + 10),
                crc: u32le(&cd, p + 16),
                csize: u32le(&cd, p + 20) as u64,
                usize: u32le(&cd, p + 24) as u64,
                offset: u32le(&cd, p + 42) as u64,
            });
            p += 46 + nl + xl + cl;
        }
        Ok(Archive { r, entries })
    }

    pub fn find(&self, name: &str) -> Option<Entry> {
        self.entries.iter().find(|e| e.name.eq_ignore_ascii_case(name)).cloned()
    }

    /// Reads and checks one entry. `limit` caps the uncompressed size.
    pub fn read(&mut self, e: &Entry, limit: usize) -> Result<Vec<u8>, String> {
        if e.usize as usize > limit {
            return Err(format!("{} is too large ({} bytes)", e.name, e.usize));
        }
        self.r.seek(SeekFrom::Start(e.offset)).map_err(|x| x.to_string())?;
        let mut h = [0u8; 30];
        self.r.read_exact(&mut h).map_err(|x| x.to_string())?;
        if u32le(&h, 0) != 0x0403_4b50 {
            return Err("damaged local header".into());
        }
        let skip = u16le(&h, 26) as i64 + u16le(&h, 28) as i64;
        self.r.seek(SeekFrom::Current(skip)).map_err(|x| x.to_string())?;
        let mut raw = vec![0u8; e.csize as usize];
        self.r.read_exact(&mut raw).map_err(|x| x.to_string())?;
        let data = match e.method {
            0 => raw,
            8 => inflate(&raw, limit)?,
            m => return Err(format!("unsupported compression method {m}")),
        };
        if crc32(&data) != e.crc {
            return Err(format!("{}: CRC mismatch", e.name));
        }
        Ok(data)
    }
}

// ------------------------------------------------------------------ writing

/// Writes every file under `dir` into a new ZIP at `dest` (STORE, no
/// compression), then reads the archive back and checks every entry's CRC.
pub fn zip_dir(dir: &Path, dest: &Path) -> Result<u64, String> {
    let walk = crate::files::walk(dir, &crate::files::Ignore::parse(""));
    if !walk.errors.is_empty() {
        return Err(format!("could not read: {}", walk.errors.join("; ")));
    }
    let mut files: Vec<String> = walk.files.iter().map(|(r, _)| r.clone()).collect();
    files.sort();
    let tmp = dest.with_extension("zip.partial");
    let res = (|| -> Result<u64, String> {
        let mut out = std::io::BufWriter::new(std::fs::File::create(&tmp).map_err(|e| e.to_string())?);
        let mut central = Vec::new();
        let mut off: u64 = 0;
        for rel in &files {
            let data = std::fs::read(dir.join(rel)).map_err(|e| format!("{rel}: {e}"))?;
            let name = rel.replace('\\', "/");
            if off + data.len() as u64 + 30 + name.len() as u64 > u32::MAX as u64 {
                return Err("the backup is larger than 4 GB, which this ZIP export doesn't support; use the backup folder instead".into());
            }
            let crc = crc32(&data);
            let mut lh = Vec::with_capacity(30 + name.len());
            lh.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
            lh.extend_from_slice(&20u16.to_le_bytes()); // version needed
            lh.extend_from_slice(&0x0800u16.to_le_bytes()); // UTF-8 names
            lh.extend_from_slice(&0u16.to_le_bytes()); // store
            lh.extend_from_slice(&[0, 0, 0x21, 0]); // time/date (1980-01-01)
            lh.extend_from_slice(&crc.to_le_bytes());
            lh.extend_from_slice(&(data.len() as u32).to_le_bytes());
            lh.extend_from_slice(&(data.len() as u32).to_le_bytes());
            lh.extend_from_slice(&(name.len() as u16).to_le_bytes());
            lh.extend_from_slice(&0u16.to_le_bytes());
            lh.extend_from_slice(name.as_bytes());
            out.write_all(&lh).map_err(|e| e.to_string())?;
            out.write_all(&data).map_err(|e| e.to_string())?;
            let mut ch = Vec::new();
            ch.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
            ch.extend_from_slice(&20u16.to_le_bytes());
            ch.extend_from_slice(&20u16.to_le_bytes());
            ch.extend_from_slice(&0x0800u16.to_le_bytes());
            ch.extend_from_slice(&0u16.to_le_bytes());
            ch.extend_from_slice(&[0, 0, 0x21, 0]);
            ch.extend_from_slice(&crc.to_le_bytes());
            ch.extend_from_slice(&(data.len() as u32).to_le_bytes());
            ch.extend_from_slice(&(data.len() as u32).to_le_bytes());
            ch.extend_from_slice(&(name.len() as u16).to_le_bytes());
            ch.extend_from_slice(&0u16.to_le_bytes()); // extra len
            ch.extend_from_slice(&0u16.to_le_bytes()); // comment len
            ch.extend_from_slice(&0u16.to_le_bytes()); // disk
            ch.extend_from_slice(&0u16.to_le_bytes()); // internal attrs
            ch.extend_from_slice(&0u32.to_le_bytes()); // external attrs
            ch.extend_from_slice(&(off as u32).to_le_bytes());
            ch.extend_from_slice(name.as_bytes());
            central.extend_from_slice(&ch);
            off += lh.len() as u64 + data.len() as u64;
        }
        if files.len() > 0xFFFF {
            return Err("too many files for a ZIP export (over 65,535)".into());
        }
        out.write_all(&central).map_err(|e| e.to_string())?;
        let mut eocd = Vec::new();
        eocd.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
        eocd.extend_from_slice(&[0u8; 4]);
        eocd.extend_from_slice(&(files.len() as u16).to_le_bytes());
        eocd.extend_from_slice(&(files.len() as u16).to_le_bytes());
        eocd.extend_from_slice(&(central.len() as u32).to_le_bytes());
        eocd.extend_from_slice(&(off as u32).to_le_bytes());
        eocd.extend_from_slice(&0u16.to_le_bytes());
        out.write_all(&eocd).map_err(|e| e.to_string())?;
        out.flush().map_err(|e| e.to_string())?;
        drop(out);
        // read back and check every entry
        let mut a = Archive::open(&tmp)?;
        if a.entries.len() != files.len() {
            return Err("ZIP verification failed: entry count".into());
        }
        for e in a.entries.clone() {
            a.read(&e, usize::MAX)?;
        }
        Ok(std::fs::metadata(&tmp).map(|m| m.len()).unwrap_or(0))
    })();
    match res {
        Ok(n) => {
            std::fs::rename(&tmp, dest).map_err(|e| e.to_string())?;
            Ok(n)
        }
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const DYN: [u8; 332] = [11, 201, 72, 85, 40, 44, 205, 76, 206, 86, 72, 42, 202, 47, 207, 83, 72, 203, 175, 80, 200, 42, 205, 45, 40, 86, 200, 47, 75, 45, 82, 40, 1, 74, 231, 36, 86, 85, 42, 164, 228, 167, 235, 41, 132, 140, 42, 30, 85, 60, 170, 120, 84, 241, 168, 226, 81, 197, 195, 75, 49, 3, 35, 19, 51, 11, 43, 27, 59, 7, 39, 23, 55, 15, 47, 31, 191, 128, 160, 144, 176, 136, 168, 152, 184, 132, 164, 148, 180, 140, 172, 156, 188, 130, 162, 146, 178, 138, 170, 154, 186, 134, 166, 150, 182, 142, 174, 158, 190, 129, 161, 145, 177, 137, 169, 153, 185, 133, 165, 149, 181, 141, 173, 157, 189, 131, 163, 147, 179, 139, 171, 155, 187, 135, 167, 151, 183, 143, 175, 159, 127, 64, 96, 80, 112, 72, 104, 88, 120, 68, 100, 84, 116, 76, 108, 92, 124, 66, 98, 82, 114, 74, 106, 90, 122, 70, 102, 86, 118, 78, 110, 94, 126, 65, 97, 81, 113, 73, 105, 89, 121, 69, 101, 85, 117, 77, 109, 93, 125, 67, 99, 83, 115, 75, 107, 91, 123, 71, 103, 87, 119, 79, 111, 95, 255, 132, 137, 147, 38, 79, 153, 58, 109, 250, 140, 153, 179, 102, 207, 153, 59, 111, 254, 130, 133, 139, 22, 47, 89, 186, 108, 249, 138, 149, 171, 86, 175, 89, 187, 110, 253, 134, 141, 155, 54, 111, 217, 186, 109, 251, 142, 157, 187, 118, 239, 217, 187, 111, 255, 129, 131, 135, 14, 31, 57, 122, 236, 248, 137, 147, 167, 78, 159, 57, 123, 238, 252, 133, 139, 151, 46, 95, 185, 122, 237, 250, 141, 155, 183, 110, 223, 185, 123, 239, 254, 131, 135, 143, 30, 63, 121, 250, 236, 249, 139, 151, 175, 94, 191, 121, 251, 238, 253, 135, 143, 159, 62, 127, 249, 250, 237, 251, 143, 159, 191, 126, 255, 249, 251, 239, 63, 0];
    #[test]
    fn crc_vector() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }
    #[test]
    fn inflate_fixed_and_dynamic() {
        // "hello hello hello hello\n" compressed with zlib's raw deflate (fixed Huffman)
        let fixed = [203, 72, 205, 201, 201, 87, 200, 64, 39, 185, 0];
        assert_eq!(inflate(&fixed, 1000).unwrap(), b"hello hello hello hello\n");
        // dynamic Huffman (zlib level 9), with back-references and all byte values
        let mut text = b"The quick brown fox jumps over the lazy dog. ".repeat(40);
        text.extend((0..=255u8).collect::<Vec<_>>());
        let dynamic: &[u8] = &DYN;
        assert_eq!(inflate(dynamic, 100_000).unwrap(), text);
        // stored block
        let stored = [0x01, 0x03, 0x00, 0xfc, 0xff, b'a', b'b', b'c'];
        assert_eq!(inflate(&stored, 10).unwrap(), b"abc");
    }
    #[test]
    fn zip_roundtrip() {
        let t = std::env::temp_dir().join(format!("pv-zip-{}", &crate::util::new_id()[..8]));
        std::fs::create_dir_all(t.join("src/a")).unwrap();
        std::fs::write(t.join("src/a/one.txt"), "one").unwrap();
        std::fs::write(t.join("src/two.bin"), vec![7u8; 5000]).unwrap();
        zip_dir(&t.join("src"), &t.join("out.zip")).unwrap();
        let mut a = Archive::open(&t.join("out.zip")).unwrap();
        let e = a.find("a/one.txt").unwrap();
        assert_eq!(a.read(&e, 100).unwrap(), b"one");
        let _ = std::fs::remove_dir_all(&t);
    }
}

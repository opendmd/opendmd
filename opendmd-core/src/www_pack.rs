//! The www pack: the web app as one small read-only archive, which the
//! firmware memory-maps out of its `www` flash partition and serves slices
//! of straight from the flash cache. `docs/www-pack.md` is the format's
//! description; this module is its one implementation — the packer
//! (`opendmd-pack www`) and the firmware's reader both use it.
//!
//! No dependencies, no allocation on the read side: [`Pack::parse`] borrows
//! the bytes it is given (on the device, the mapped partition) and every
//! lookup hands back a slice of them.
//!
//! ```text
//! offset  size  field
//!      0     4  magic "ODWP"
//!      4     2  format version (1)
//!      6     2  header length (64)
//!      8     4  total length of the pack, header included
//!     12     4  CRC-32 (IEEE, as zlib) of bytes [header length, total length)
//!     16     2  entry count
//!     18     1  version string length (<= 44)
//!     19     1  reserved (0)
//!     20    44  version string, UTF-8, zero-padded (`git describe` of the app)
//!     64        entries: count x 16 B, sorted by path (bytewise, strictly)
//!               path strings, back to back, UTF-8, no terminators
//!               zero padding to 4
//!               file data, each file starting on a 4-byte boundary
//!
//! entry: path offset u32, data offset u32, data length u32,
//!        path length u16, flags u16 (bit 0: data is gzip)
//! ```
//!
//! Everything little-endian; offsets are from the start of the pack.

use std::fmt;

/// `b"ODWP"`: OpenDMD Www Pack.
pub const MAGIC: [u8; 4] = *b"ODWP";
/// The format this module reads and writes.
pub const FORMAT: u16 = 1;
/// The header's length, and where the entries start.
pub const HEADER_LEN: usize = 64;
/// One entry's length.
pub const ENTRY_LEN: usize = 16;
/// The longest version string the header holds.
pub const VERSION_MAX: usize = 44;
/// Entry flag: the data is gzip, to be sent with `Content-Encoding: gzip`.
pub const FLAG_GZIP: u16 = 1;

/// Why bytes are not a pack, or not a good one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PackError {
    /// Fewer bytes than a header, or than the header's total length.
    Truncated {
        have: usize,
        need: usize,
    },
    BadMagic,
    UnsupportedFormat(u16),
    BadHeader(&'static str),
    /// Entry `index` points outside the pack or is malformed.
    BadEntry {
        index: usize,
        why: &'static str,
    },
    /// Paths out of order or repeated (at entry `index`).
    Unsorted {
        index: usize,
    },
    CrcMismatch {
        stored: u32,
        computed: u32,
    },
    /// Writer only.
    Invalid(String),
}

impl fmt::Display for PackError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PackError::Truncated { have, need } => {
                write!(f, "truncated: {have} B, the pack needs {need} B")
            }
            PackError::BadMagic => f.write_str("not a www pack (bad magic)"),
            PackError::UnsupportedFormat(v) => write!(f, "unsupported pack format {v}"),
            PackError::BadHeader(why) => write!(f, "bad header: {why}"),
            PackError::BadEntry { index, why } => write!(f, "bad entry {index}: {why}"),
            PackError::Unsorted { index } => {
                write!(f, "entry {index} is out of order or repeated")
            }
            PackError::CrcMismatch { stored, computed } => write!(
                f,
                "CRC mismatch: header says {stored:08x}, contents are {computed:08x}"
            ),
            PackError::Invalid(why) => f.write_str(why),
        }
    }
}

impl std::error::Error for PackError {}

fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

/// The fixed 64-byte header on its own: what the firmware checks from the
/// first bytes of an upload before it erases anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    pub total_len: u32,
    pub crc32: u32,
    pub count: u16,
    version: [u8; VERSION_MAX],
    version_len: u8,
}

impl Header {
    /// Parse and sanity-check the header (magic, format, lengths). Says
    /// nothing about the entries or the CRC.
    pub fn parse(bytes: &[u8]) -> Result<Header, PackError> {
        if bytes.len() < HEADER_LEN {
            return Err(PackError::Truncated {
                have: bytes.len(),
                need: HEADER_LEN,
            });
        }
        if bytes[..4] != MAGIC {
            return Err(PackError::BadMagic);
        }
        let format = u16_at(bytes, 4);
        if format != FORMAT {
            return Err(PackError::UnsupportedFormat(format));
        }
        if u16_at(bytes, 6) as usize != HEADER_LEN {
            return Err(PackError::BadHeader("header length is not 64"));
        }
        let total_len = u32_at(bytes, 8);
        let crc32 = u32_at(bytes, 12);
        let count = u16_at(bytes, 16);
        let version_len = bytes[18];
        if version_len as usize > VERSION_MAX {
            return Err(PackError::BadHeader("version string too long"));
        }
        let min = HEADER_LEN + count as usize * ENTRY_LEN;
        if (total_len as usize) < min {
            return Err(PackError::BadHeader(
                "total length is shorter than the entry table",
            ));
        }
        let mut version = [0u8; VERSION_MAX];
        version.copy_from_slice(&bytes[20..20 + VERSION_MAX]);
        if std::str::from_utf8(&version[..version_len as usize]).is_err() {
            return Err(PackError::BadHeader("version string is not UTF-8"));
        }
        Ok(Header {
            total_len,
            crc32,
            count,
            version,
            version_len,
        })
    }

    /// The web app's version string (`git describe`).
    pub fn version(&self) -> &str {
        // Checked in `parse`.
        std::str::from_utf8(&self.version[..self.version_len as usize]).unwrap_or("")
    }
}

/// One file in a pack.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entry<'a> {
    /// As the browser asks for it, without a leading slash:
    /// `index.html`, `js/smooth.js`, `pkg/opendmd_web_bg.wasm`.
    pub path: &'a str,
    /// The bytes to send: gzip when [`Entry::gzip`].
    pub data: &'a [u8],
    pub gzip: bool,
}

/// A parsed pack borrowing its bytes. Every entry was bounds-checked by
/// [`Pack::parse`]; the CRC is checked separately ([`Pack::verify`]), so a
/// caller can say which of the two failed.
#[derive(Debug, Clone, Copy)]
pub struct Pack<'a> {
    bytes: &'a [u8],
    header: Header,
}

impl<'a> Pack<'a> {
    /// `bytes` may run past the pack (a whole flash partition): only the
    /// header's total length is looked at. Checks the header, every entry's
    /// ranges, the paths' UTF-8 and their order.
    pub fn parse(bytes: &'a [u8]) -> Result<Pack<'a>, PackError> {
        let header = Header::parse(bytes)?;
        let total = header.total_len as usize;
        if bytes.len() < total {
            return Err(PackError::Truncated {
                have: bytes.len(),
                need: total,
            });
        }
        let bytes = &bytes[..total];
        let pack = Pack { bytes, header };
        let mut prev: Option<&[u8]> = None;
        for index in 0..header.count as usize {
            let (path, _, _) = pack.raw_entry(index)?;
            if prev.is_some_and(|p| p >= path) {
                return Err(PackError::Unsorted { index });
            }
            prev = Some(path);
        }
        Ok(pack)
    }

    pub fn header(&self) -> &Header {
        &self.header
    }

    pub fn version(&self) -> &str {
        self.header.version()
    }

    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.header.count == 0
    }

    pub fn count(&self) -> usize {
        self.header.count as usize
    }

    /// The CRC over everything after the header, computed over the borrowed
    /// bytes in one pass with no buffer.
    pub fn computed_crc(&self) -> u32 {
        crc32(&self.bytes[HEADER_LEN..])
    }

    /// Compare [`Pack::computed_crc`] with the header's.
    pub fn verify(&self) -> Result<(), PackError> {
        let computed = self.computed_crc();
        if computed == self.header.crc32 {
            Ok(())
        } else {
            Err(PackError::CrcMismatch {
                stored: self.header.crc32,
                computed,
            })
        }
    }

    /// Entry `index`'s path bytes, data range and flags, bounds-checked.
    fn raw_entry(&self, index: usize) -> Result<(&'a [u8], &'a [u8], u16), PackError> {
        let b = self.bytes;
        let at = HEADER_LEN + index * ENTRY_LEN;
        let bad = |why| PackError::BadEntry { index, why };
        if at + ENTRY_LEN > b.len() {
            return Err(bad("outside the pack"));
        }
        let path_off = u32_at(b, at) as usize;
        let data_off = u32_at(b, at + 4) as usize;
        let data_len = u32_at(b, at + 8) as usize;
        let path_len = u16_at(b, at + 12) as usize;
        let flags = u16_at(b, at + 14);
        let table_end = HEADER_LEN + self.header.count as usize * ENTRY_LEN;
        if path_len == 0 {
            return Err(bad("empty path"));
        }
        if path_off < table_end || path_off.checked_add(path_len).is_none_or(|e| e > b.len()) {
            return Err(bad("path outside the pack"));
        }
        if data_off < table_end || data_off.checked_add(data_len).is_none_or(|e| e > b.len()) {
            return Err(bad("data outside the pack"));
        }
        if flags & !FLAG_GZIP != 0 {
            return Err(bad("unknown flags"));
        }
        let path = &b[path_off..path_off + path_len];
        if std::str::from_utf8(path).is_err() {
            return Err(bad("path is not UTF-8"));
        }
        Ok((path, &b[data_off..data_off + data_len], flags))
    }

    /// Entry `index` (0..count), in path order.
    pub fn entry(&self, index: usize) -> Option<Entry<'a>> {
        let (path, data, flags) = self.raw_entry(index).ok()?;
        Some(Entry {
            path: std::str::from_utf8(path).ok()?,
            data,
            gzip: flags & FLAG_GZIP != 0,
        })
    }

    pub fn entries(&self) -> impl Iterator<Item = Entry<'a>> + '_ {
        (0..self.count()).filter_map(|i| self.entry(i))
    }

    /// The file at `path` (no leading slash), by binary search.
    pub fn get(&self, path: &str) -> Option<Entry<'a>> {
        let want = path.as_bytes();
        let (mut lo, mut hi) = (0usize, self.count());
        while lo < hi {
            let mid = (lo + hi) / 2;
            let (p, _, _) = self.raw_entry(mid).ok()?;
            match p.cmp(want) {
                std::cmp::Ordering::Less => lo = mid + 1,
                std::cmp::Ordering::Greater => hi = mid,
                std::cmp::Ordering::Equal => return self.entry(mid),
            }
        }
        None
    }
}

/// A file to put in a pack.
#[derive(Debug, Clone)]
pub struct PackFile {
    /// As the browser asks for it, without a leading slash.
    pub path: String,
    pub data: Vec<u8>,
    /// `data` is gzip.
    pub gzip: bool,
}

/// Build a pack: files sorted by path, data 4-byte aligned, the CRC filled
/// in. Refuses a duplicate or empty path, a leading slash, a version string
/// over [`VERSION_MAX`] bytes, and anything past `u32`.
pub fn build(version: &str, mut files: Vec<PackFile>) -> Result<Vec<u8>, PackError> {
    let invalid = |s: String| Err(PackError::Invalid(s));
    if version.len() > VERSION_MAX {
        return invalid(format!(
            "version string '{version}' is {} B, the header holds {VERSION_MAX}",
            version.len()
        ));
    }
    if files.len() > u16::MAX as usize {
        return invalid(format!("{} files, a pack holds {}", files.len(), u16::MAX));
    }
    files.sort_by(|a, b| a.path.as_bytes().cmp(b.path.as_bytes()));
    for (i, f) in files.iter().enumerate() {
        if f.path.is_empty() || f.path.starts_with('/') || f.path.len() > u16::MAX as usize {
            return invalid(format!("bad path '{}'", f.path));
        }
        if i > 0 && files[i - 1].path == f.path {
            return invalid(format!("'{}' is in the pack twice", f.path));
        }
    }
    let align = |n: usize| (n + 3) & !3;
    let table_end = HEADER_LEN + files.len() * ENTRY_LEN;
    let strings: usize = files.iter().map(|f| f.path.len()).sum();
    let mut data_at = align(table_end + strings);
    let mut layout = Vec::with_capacity(files.len());
    let mut path_at = table_end;
    for f in &files {
        layout.push((path_at, data_at));
        path_at += f.path.len();
        data_at = align(data_at + f.data.len());
    }
    // The last file is not padded.
    let total = files.last().map_or(align(table_end + strings), |f| {
        layout.last().unwrap().1 + f.data.len()
    });
    if total > u32::MAX as usize {
        return invalid(format!("{total} B is past what a pack can address"));
    }
    let mut out = vec![0u8; total];
    out[..4].copy_from_slice(&MAGIC);
    out[4..6].copy_from_slice(&FORMAT.to_le_bytes());
    out[6..8].copy_from_slice(&(HEADER_LEN as u16).to_le_bytes());
    out[8..12].copy_from_slice(&(total as u32).to_le_bytes());
    out[16..18].copy_from_slice(&(files.len() as u16).to_le_bytes());
    out[18] = version.len() as u8;
    out[20..20 + version.len()].copy_from_slice(version.as_bytes());
    for (i, (f, &(p, d))) in files.iter().zip(&layout).enumerate() {
        let at = HEADER_LEN + i * ENTRY_LEN;
        out[at..at + 4].copy_from_slice(&(p as u32).to_le_bytes());
        out[at + 4..at + 8].copy_from_slice(&(d as u32).to_le_bytes());
        out[at + 8..at + 12].copy_from_slice(&(f.data.len() as u32).to_le_bytes());
        out[at + 12..at + 14].copy_from_slice(&(f.path.len() as u16).to_le_bytes());
        let flags = if f.gzip { FLAG_GZIP } else { 0 };
        out[at + 14..at + 16].copy_from_slice(&flags.to_le_bytes());
        out[p..p + f.path.len()].copy_from_slice(f.path.as_bytes());
        out[d..d + f.data.len()].copy_from_slice(&f.data);
    }
    let crc = crc32(&out[HEADER_LEN..]);
    out[12..16].copy_from_slice(&crc.to_le_bytes());
    Ok(out)
}

/// CRC-32/ISO-HDLC (zlib's `crc32`, gzip's, PNG's), table-driven. The table
/// is built at compile time, so on the device it lives in flash (1 KB of
/// rodata) and costs no heap.
pub fn crc32(data: &[u8]) -> u32 {
    let mut c = Crc32::new();
    c.update(data);
    c.finish()
}

/// [`crc32`] in pieces.
#[derive(Debug, Clone, Copy)]
pub struct Crc32(u32);

impl Default for Crc32 {
    fn default() -> Self {
        Self::new()
    }
}

impl Crc32 {
    pub fn new() -> Self {
        Crc32(!0)
    }

    pub fn update(&mut self, data: &[u8]) {
        let mut c = self.0;
        for &b in data {
            c = CRC_TABLE[((c ^ b as u32) & 0xff) as usize] ^ (c >> 8);
        }
        self.0 = c;
    }

    pub fn finish(&self) -> u32 {
        !self.0
    }
}

static CRC_TABLE: [u32; 256] = crc_table();

const fn crc_table() -> [u32; 256] {
    let mut t = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
            k += 1;
        }
        t[i] = c;
        i += 1;
    }
    t
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files() -> Vec<PackFile> {
        vec![
            PackFile {
                path: "pkg-art/opendmd_art_web_bg.wasm".into(),
                data: (0..5000u32).map(|i| (i * 7) as u8).collect(),
                gzip: true,
            },
            PackFile {
                path: "index.html".into(),
                data: b"<!doctype html>".to_vec(),
                gzip: true,
            },
            PackFile {
                path: "js/smooth.js".into(),
                data: b"x".to_vec(),
                gzip: false,
            },
            PackFile {
                path: "art/SNES.json".into(),
                data: Vec::new(),
                gzip: false,
            },
        ]
    }

    #[test]
    fn crc_matches_the_standard_check_value() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b""), 0);
        let mut c = Crc32::new();
        c.update(b"1234");
        c.update(b"56789");
        assert_eq!(c.finish(), 0xCBF4_3926);
    }

    #[test]
    fn round_trips() {
        let input = files();
        let bytes = build("v0.3.0-4-gabcdef0-dirty", input.clone()).unwrap();
        // Trailing bytes (the rest of a flash partition) are ignored.
        let mut padded = bytes.clone();
        padded.extend_from_slice(&[0xff; 4096]);
        let pack = Pack::parse(&padded).unwrap();
        pack.verify().unwrap();
        assert_eq!(pack.version(), "v0.3.0-4-gabcdef0-dirty");
        assert_eq!(pack.len(), bytes.len());
        assert_eq!(pack.count(), 4);
        for f in &input {
            let e = pack.get(&f.path).unwrap();
            assert_eq!(e.path, f.path);
            assert_eq!(e.data, &f.data[..]);
            assert_eq!(e.gzip, f.gzip);
            assert_eq!((e.data.as_ptr() as usize - padded.as_ptr() as usize) % 4, 0);
        }
        assert!(pack.get("nope.js").is_none());
        assert!(pack.get("").is_none());
        assert!(pack.get("/index.html").is_none());
        let paths: Vec<_> = pack.entries().map(|e| e.path).collect();
        assert_eq!(
            paths,
            [
                "art/SNES.json",
                "index.html",
                "js/smooth.js",
                "pkg-art/opendmd_art_web_bg.wasm"
            ]
        );
        let h = Header::parse(&bytes[..HEADER_LEN]).unwrap();
        assert_eq!(h.total_len as usize, bytes.len());
        assert_eq!(h.count, 4);
    }

    #[test]
    fn empty_pack() {
        let bytes = build("", Vec::new()).unwrap();
        assert_eq!(bytes.len(), HEADER_LEN);
        let pack = Pack::parse(&bytes).unwrap();
        pack.verify().unwrap();
        assert!(pack.is_empty());
        assert!(pack.get("index.html").is_none());
    }

    #[test]
    fn damage_is_detected() {
        let bytes = build("v1", files()).unwrap();
        // A flipped data byte: parses, fails the CRC.
        let mut b = bytes.clone();
        let last = b.len() - 1;
        b[last] ^= 1;
        assert!(matches!(
            Pack::parse(&b).unwrap().verify(),
            Err(PackError::CrcMismatch { .. })
        ));
        // Cut short.
        assert!(matches!(
            Pack::parse(&bytes[..bytes.len() - 1]),
            Err(PackError::Truncated { .. })
        ));
        // Erased flash.
        assert_eq!(Pack::parse(&[0xff; 128]).unwrap_err(), PackError::BadMagic);
        let mut b = bytes.clone();
        b[4] = 2;
        assert_eq!(
            Pack::parse(&b).unwrap_err(),
            PackError::UnsupportedFormat(2)
        );
        // An entry pointing past the end.
        let mut b = bytes.clone();
        b[HEADER_LEN + 8..HEADER_LEN + 12].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(matches!(
            Pack::parse(&b),
            Err(PackError::BadEntry { index: 0, .. })
        ));
        // Two entries swapped: out of order.
        let mut b = bytes.clone();
        let (e0, e1) = (HEADER_LEN, HEADER_LEN + ENTRY_LEN);
        let first: Vec<u8> = b[e0..e1].to_vec();
        b.copy_within(e1..e1 + ENTRY_LEN, e0);
        b[e1..e1 + ENTRY_LEN].copy_from_slice(&first);
        assert_eq!(
            Pack::parse(&b).unwrap_err(),
            PackError::Unsorted { index: 1 }
        );
    }

    #[test]
    fn builder_refuses_bad_input() {
        let dup = vec![
            PackFile {
                path: "a".into(),
                data: vec![],
                gzip: false,
            };
            2
        ];
        assert!(build("v", dup).is_err());
        let slash = vec![PackFile {
            path: "/a".into(),
            data: vec![],
            gzip: false,
        }];
        assert!(build("v", slash).is_err());
        assert!(build(&"x".repeat(VERSION_MAX + 1), Vec::new()).is_err());
        assert!(build(&"x".repeat(VERSION_MAX), Vec::new()).is_ok());
    }
}

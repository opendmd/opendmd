//! The playlist index as a file on the card, next to the manifest it indexes.
//!
//! [`FileCatalog`](crate::engine::FileCatalog) keeps one `u32` byte offset per
//! matching manifest line in a `Vec` — 40,532 B for the stock card, and
//! building it passes through a 64 KB allocation as the `Vec` doubles. On the
//! ESP32 that is the largest single block after the panel's DMA buffer. This
//! module writes exactly the same offsets to a sidecar file instead, and
//! [`IndexedCatalog`] seeks into it, so the playlist costs a couple of file
//! handles and one [`Entry`] at a time.
//!
//! # Format (`<manifest>.idx`, little-endian)
//!
//! | offset | size | field |
//! |---|---|---|
//! | 0 | 4 | magic `b"ODMX"` |
//! | 4 | 2 | `version` = 1 |
//! | 6 | 4 | `manifest_len`: byte length of the manifest it was built from |
//! | 10 | 8 | `filter_fingerprint`: [`Filter::fingerprint`] of the filter |
//! | 18 | 4 | `count`: number of offsets that follow |
//! | 22 | 10 | zero padding, reserved |
//! | 32 | 4 × `count` | byte offset of each matching line, manifest order |
//!
//! The offsets are what [`index_matching`](crate::manifest::index_matching)
//! returns, bit for bit.
//!
//! # Staleness
//!
//! [`IndexHeader::is_valid_for`] compares the manifest's length and the
//! filter's fingerprint. That is a cheap check, not a content hash: a manifest
//! rewritten with *exactly* the same length would pass it. Anything that writes
//! a manifest must therefore delete its `.idx` first, and the length check is
//! the backstop for tools that don't (a PC editing the card, say). The decision
//! is the caller's — [`IndexedCatalog::open`] only checks that the file is an
//! index at all.

use crate::engine::Catalog;
use crate::filter::Filter;
use crate::manifest::{self, Entry};
use std::io::{self, BufRead, BufReader, BufWriter, Read, Seek, SeekFrom, Write};

/// First four bytes of every index file.
pub const MAGIC: [u8; 4] = *b"ODMX";
/// The format version this build reads and writes.
pub const VERSION: u16 = 1;
/// Bytes before the first offset: the 22-byte header, padded for future fields.
pub const HEADER_LEN: u64 = 32;

/// The fixed-size header at the start of an index file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IndexHeader {
    pub version: u16,
    /// Byte length of the manifest the offsets point into.
    pub manifest_len: u32,
    /// [`Filter::fingerprint`] of the filter the offsets were selected by.
    pub filter_fingerprint: u64,
    /// Number of `u32` offsets after the header.
    pub count: u32,
}

impl IndexHeader {
    /// A current-version header.
    pub fn new(manifest_len: u32, filter: &Filter, count: u32) -> Self {
        IndexHeader {
            version: VERSION,
            manifest_len,
            filter_fingerprint: filter.fingerprint(),
            count,
        }
    }

    /// Read and check the magic and the version. Everything else is data.
    pub fn read(r: &mut impl Read) -> io::Result<Self> {
        let mut b = [0u8; HEADER_LEN as usize];
        r.read_exact(&mut b)?;
        if b[0..4] != MAGIC {
            return Err(invalid("not a playlist index (bad magic)".to_string()));
        }
        let version = u16::from_le_bytes([b[4], b[5]]);
        if version != VERSION {
            return Err(invalid(format!(
                "playlist index version {version}, this build reads {VERSION}"
            )));
        }
        Ok(IndexHeader {
            version,
            manifest_len: u32::from_le_bytes(b[6..10].try_into().unwrap()),
            filter_fingerprint: u64::from_le_bytes(b[10..18].try_into().unwrap()),
            count: u32::from_le_bytes(b[18..22].try_into().unwrap()),
        })
    }

    /// Write all [`HEADER_LEN`] bytes, padding included.
    pub fn write(&self, w: &mut impl Write) -> io::Result<()> {
        let mut b = [0u8; HEADER_LEN as usize];
        b[0..4].copy_from_slice(&MAGIC);
        b[4..6].copy_from_slice(&self.version.to_le_bytes());
        b[6..10].copy_from_slice(&self.manifest_len.to_le_bytes());
        b[10..18].copy_from_slice(&self.filter_fingerprint.to_le_bytes());
        b[18..22].copy_from_slice(&self.count.to_le_bytes());
        w.write_all(&b)
    }

    /// Does this index answer `filter` over a manifest of `manifest_len` bytes?
    ///
    /// See the module docs for what the length check does and does not catch.
    pub fn is_valid_for(&self, manifest_len: u32, filter: &Filter) -> bool {
        self.version == VERSION
            && self.manifest_len == manifest_len
            && self.filter_fingerprint == filter.fingerprint()
    }

    /// Total file length an index with this header must have.
    pub fn file_len(&self) -> u64 {
        HEADER_LEN + 4 * u64::from(self.count)
    }
}

fn invalid(msg: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg)
}

/// Build an index of `manifest` under `filter` into `out`, in one streaming
/// pass, and return the header written.
///
/// Memory is bounded by the two buffers — the caller's `BufRead` and a 2 KB
/// `BufWriter` here — plus one line and one [`Entry`], exactly like
/// [`manifest::scan`], which this is built on. Nothing grows with the card.
///
/// `out` must be `Seek` because the count is not known until the pass is
/// over: a header with `count: 0` is written first, the offsets are streamed
/// after it, and then the header is rewritten in place. The index starts at
/// `out`'s position on entry (normally 0, a fresh file) and `out` is left
/// positioned at its end. If this returns an error, whatever was written is
/// garbage; write to a temporary file and rename it into place on success.
///
/// `manifest_len` is recorded, not checked: it is the caller's `stat` of the
/// file `manifest` reads, and it is what [`IndexHeader::is_valid_for`] will be
/// compared against later.
pub fn write_index<R: BufRead, W: Write + Seek>(
    manifest: R,
    manifest_len: u32,
    filter: &Filter,
    out: W,
) -> io::Result<IndexHeader> {
    let mut out = BufWriter::with_capacity(2048, out);
    let start = out.stream_position()?;
    let mut header = IndexHeader::new(manifest_len, filter, 0);
    header.write(&mut out)?;

    // `scan`'s visitor cannot return an error, so the first write failure is
    // parked here and every later line is skipped.
    let mut failed: Option<io::Error> = None;
    let mut count: u32 = 0;
    manifest::scan(manifest, |offset, entry| {
        if failed.is_some() || !filter.matches(entry) {
            return;
        }
        match out.write_all(&offset.to_le_bytes()) {
            Ok(()) => count += 1,
            Err(e) => failed = Some(e),
        }
    })?;
    if let Some(e) = failed {
        return Err(e);
    }

    header.count = count;
    // `BufWriter::seek` flushes first, so the offsets are all out before the
    // header is patched over its placeholder.
    out.seek(SeekFrom::Start(start))?;
    header.write(&mut out)?;
    out.seek(SeekFrom::Start(start + header.file_len()))?;
    out.flush()?;
    Ok(header)
}

/// A [`Catalog`] over a manifest and its on-card index: `len` comes from the
/// header, `entry(i)` reads one `u32` from the index and one line from the
/// manifest.
///
/// The resident cost is the two readers — a 1 KB `BufReader` on the manifest,
/// and whatever `I` is (a bare `File` on the firmware, since each lookup reads
/// exactly four bytes) — and the header. No per-entry allocation beyond the
/// returned [`Entry`].
pub struct IndexedCatalog<M: Read + Seek, I: Read + Seek> {
    manifest: BufReader<M>,
    index: I,
    header: IndexHeader,
}

impl<M: Read + Seek, I: Read + Seek> IndexedCatalog<M, I> {
    /// Read the header from the start of `index` and check it is a complete
    /// index of this format: magic, version, and a file long enough to hold
    /// `count` offsets (a torn write would otherwise surface much later as a
    /// failed `entry`).
    ///
    /// Whether it is a *current* index of this manifest under the caller's
    /// filter is not checked here — that is [`IndexHeader::is_valid_for`], and
    /// the caller's decision.
    pub fn open(manifest: M, mut index: I) -> io::Result<Self> {
        index.seek(SeekFrom::Start(0))?;
        let header = IndexHeader::read(&mut index)?;
        let len = index.seek(SeekFrom::End(0))?;
        if len < header.file_len() {
            return Err(invalid(format!(
                "playlist index truncated: {len} B, header promises {} B",
                header.file_len()
            )));
        }
        Ok(IndexedCatalog {
            manifest: BufReader::with_capacity(1024, manifest),
            index,
            header,
        })
    }

    pub fn header(&self) -> &IndexHeader {
        &self.header
    }

    /// The byte offset of the `i`-th matching line.
    pub fn offset(&mut self, i: usize) -> io::Result<u32> {
        if i >= self.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("catalog index {i} out of range"),
            ));
        }
        self.index
            .seek(SeekFrom::Start(HEADER_LEN + 4 * i as u64))?;
        let mut b = [0u8; 4];
        self.index.read_exact(&mut b)?;
        Ok(u32::from_le_bytes(b))
    }
}

impl<M: Read + Seek, I: Read + Seek> Catalog for IndexedCatalog<M, I> {
    fn len(&self) -> usize {
        self.header.count as usize
    }

    fn entry(&mut self, i: usize) -> io::Result<Entry> {
        let offset = self.offset(i)?;
        manifest::entry_at(&mut self.manifest, offset)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::FileCatalog;
    use crate::manifest::index_matching;
    use std::io::Cursor;

    const MANIFEST: &str = "\u{feff}/gifs/Consoles/SNES_Plok_08.gif\r\n\
                            \r\n\
                            /gifs/Arcade/ARCADE_Pac-Man_01.gif   \r\n\
                            /gifs/Consoles/MD_Sonic3_RattenJager.gif\n\
                            /gifs/Consoles/SNES_Donkey_Kong_Country_05.gif\n\
                            /gifs/XMAS/XMAS_Tree_02.gif\n";

    fn manifest() -> Cursor<Vec<u8>> {
        Cursor::new(MANIFEST.as_bytes().to_vec())
    }

    fn build(filter: &Filter) -> (IndexHeader, Vec<u8>) {
        let mut out = Cursor::new(Vec::new());
        let header = write_index(manifest(), MANIFEST.len() as u32, filter, &mut out).unwrap();
        (header, out.into_inner())
    }

    #[test]
    fn round_trip_matches_the_in_ram_catalog() {
        for filter in [
            Filter::new(),
            Filter::new().platforms(["SNES", "MD"]),
            Filter::new().exclude_flag("RattenJager"),
            Filter::new().platforms(Vec::<String>::new()),
        ] {
            let (header, bytes) = build(&filter);
            let want = index_matching(manifest(), &filter).unwrap();
            assert_eq!(header.count as usize, want.len());
            assert_eq!(bytes.len() as u64, header.file_len());

            let mut ram = FileCatalog::build(manifest(), &filter).unwrap();
            let mut disk = IndexedCatalog::open(manifest(), Cursor::new(bytes)).unwrap();
            assert_eq!(disk.header(), &header);
            assert_eq!(disk.len(), ram.len());
            // Backwards, so every lookup is a real seek on both files.
            for i in (0..disk.len()).rev() {
                assert_eq!(disk.offset(i).unwrap(), want[i]);
                assert_eq!(disk.entry(i).unwrap(), ram.entry(i).unwrap());
            }
            assert!(disk.entry(disk.len()).is_err());
        }
    }

    #[test]
    fn count_is_patched_into_the_header() {
        let filter = Filter::new().platforms(["SNES"]);
        let (header, bytes) = build(&filter);
        assert_eq!(header.count, 2);
        // The placeholder was 0; the file must carry the real count.
        assert_eq!(u32::from_le_bytes(bytes[18..22].try_into().unwrap()), 2);
        assert_eq!(&bytes[0..4], b"ODMX");
        assert_eq!(u16::from_le_bytes([bytes[4], bytes[5]]), 1);
        assert_eq!(
            u32::from_le_bytes(bytes[6..10].try_into().unwrap()),
            MANIFEST.len() as u32
        );
        assert_eq!(
            u64::from_le_bytes(bytes[10..18].try_into().unwrap()),
            filter.fingerprint()
        );
        assert!(bytes[22..32].iter().all(|b| *b == 0), "padding not zeroed");
        assert_eq!(IndexHeader::read(&mut Cursor::new(&bytes)).unwrap(), header);
    }

    #[test]
    fn writes_at_the_writers_position_and_ends_there() {
        // An index written after some leading bytes: the header is patched at
        // the start position, not at 0, and the writer is left at the end.
        let mut out = Cursor::new(b"junk".to_vec());
        out.seek(SeekFrom::End(0)).unwrap();
        let header = write_index(manifest(), 1, &Filter::new(), &mut out).unwrap();
        assert_eq!(out.position(), 4 + header.file_len());
        let bytes = out.into_inner();
        assert_eq!(&bytes[0..4], b"junk");
        let read = IndexHeader::read(&mut Cursor::new(&bytes[4..])).unwrap();
        assert_eq!(read, header);
        assert_eq!(read.count, 5);
    }

    #[test]
    fn an_empty_manifest_is_a_valid_empty_index() {
        let mut out = Cursor::new(Vec::new());
        let header = write_index(Cursor::new(Vec::new()), 0, &Filter::new(), &mut out).unwrap();
        assert_eq!(header.count, 0);
        let cat = IndexedCatalog::open(Cursor::new(Vec::new()), out).unwrap();
        assert!(cat.is_empty());
    }

    #[test]
    fn header_validation() {
        let filter = Filter::new().platforms(["MD", "SNES"]);
        let (header, bytes) = build(&filter);

        let mut bad_magic = bytes.clone();
        bad_magic[0] = b'X';
        let e = IndexHeader::read(&mut Cursor::new(&bad_magic)).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::InvalidData);
        assert!(IndexedCatalog::open(manifest(), Cursor::new(bad_magic)).is_err());

        let mut bad_version = bytes.clone();
        bad_version[4..6].copy_from_slice(&2u16.to_le_bytes());
        let e = IndexHeader::read(&mut Cursor::new(&bad_version)).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::InvalidData);
        assert!(IndexedCatalog::open(manifest(), Cursor::new(bad_version)).is_err());

        // Too short to be a header at all.
        assert!(IndexHeader::read(&mut Cursor::new(&bytes[..10])).is_err());
        // A whole header but offsets missing: open refuses it.
        let truncated = bytes[..bytes.len() - 2].to_vec();
        assert!(IndexedCatalog::open(manifest(), Cursor::new(truncated)).is_err());

        // Staleness is the caller's call, and each input moves it.
        let len = MANIFEST.len() as u32;
        assert!(header.is_valid_for(len, &filter));
        assert!(header.is_valid_for(len, &Filter::new().platforms(["snes", "md"])));
        assert!(!header.is_valid_for(len + 1, &filter));
        assert!(!header.is_valid_for(len, &Filter::new()));
        assert!(!header.is_valid_for(len, &filter.clone().exclude_flag("INVERT")));
    }
}

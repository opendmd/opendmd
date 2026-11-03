//! FAT32 directory entries read raw: the device's side index of where each
//! GIF's directory entry sits (`firmware/src/fatidx.rs`, CLAUDE.md "The FAT
//! index", `docs/research/fat-index.md`).
//!
//! A FAT open is a linear scan of the directory (~0.55 ms per 512 B sector
//! over SPI: hundreds of ms in a folder of thousands). The index records,
//! per file, the sector and slot of its 8.3 entry; an open reads that sector
//! and [`verify`]s the entry against the expected name exactly as FatFs's
//! own lookup would accept it (its long name, or its 8.3 name), so a stale
//! record costs a fallback to the path open, never a wrong file.
//!
//! Everything here works on 512-byte sectors handed in by the caller, which
//! owns locking and caching (on the device, FatFs's volume mutex and its
//! window). Names are ASCII only: a long name with anything else is not
//! indexed (the device's CP437 API mangles those paths anyway).

use std::io;

/// Sector size: SD cards, and FatFs as configured (`CONFIG_WL_SECTOR_SIZE_512`).
pub const SS: usize = 512;
/// The index file's magic and version.
pub const MAGIC: &[u8; 4] = b"FIDX";
pub const VERSION: u16 = 3;
/// Header bytes, then a `u32` fanout of [`FANOUT`] entries.
pub const HEADER: usize = 32;
pub const FANOUT: usize = 257;
pub const RECORDS_AT: u64 = (HEADER + FANOUT * 4) as u64;

/// The volume's layout, from FatFs's `FATFS` (or a boot sector).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Geometry {
    /// Sectors per cluster.
    pub csize: u32,
    /// First sector of the (first) FAT.
    pub fatbase: u32,
    /// First sector of cluster 2.
    pub database: u32,
    /// Clusters + 2.
    pub n_fatent: u32,
}

impl Geometry {
    fn rel(&self, cluster: u32) -> u32 {
        (cluster - 2) * self.csize
    }
}

/// FNV-1a over a card path, ASCII-lowercased (FatFs matches names without
/// regard to ASCII case, so the index does too): the index key.
pub fn hash(path: &[u8]) -> u32 {
    hash_more(0x811c_9dc5, path)
}

/// [`hash`] continued over more bytes (a directory, then a name).
pub fn hash_more(h: u32, bytes: &[u8]) -> u32 {
    bytes
        .iter()
        .fold(h, |h, &b| (h ^ b.to_ascii_lowercase() as u32).wrapping_mul(0x0100_0193))
}

/// A record's location word: the sector relative to the data region, and
/// the slot (0-15) of the 8.3 entry in it. `None` past 128 GiB.
pub fn loc(rel: u32, slot: usize) -> Option<u32> {
    (rel < 1 << 28).then_some(rel << 4 | slot as u32)
}

const LFN_OFS: [usize; 13] = [1, 3, 5, 7, 9, 14, 16, 18, 20, 22, 24, 28, 30];

fn sum(e: &[u8]) -> u8 {
    e[..11].iter().fold(0u8, |s, &b| s.rotate_right(1).wrapping_add(b))
}

/// A live file entry (not deleted, not LFN, not a directory or label).
fn is_file(e: &[u8]) -> bool {
    e[0] != 0 && e[0] != 0xE5 && e[11] & 0x3F != 0x0F && e[11] & 0x18 == 0
}

/// What [`verify`] found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verify {
    /// It is the file: its first cluster, size and attributes.
    Ok { cluster: u32, size: u32, attr: u8 },
    /// Not this file (stale record).
    Mismatch,
    /// The long name starts in the sector before; call again with it.
    NeedPrev,
}

/// Is slot `slot` of `cur` the live entry of a file called `name` (ASCII
/// case ignored, as FatFs's own lookup does)? `prev` is the directory
/// sector before `cur` (needed only when the long name starts there:
/// [`Verify::NeedPrev`]). This proves the name at that slot, not which
/// directory the slot belongs to: the index's directory list
/// ([`dirs_match`]) covers that.
pub fn verify(cur: &[u8], prev: Option<&[u8]>, slot: usize, name: &[u8]) -> Verify {
    let e = &cur[slot * 32..][..32];
    if !is_file(e) {
        return Verify::Mismatch;
    }
    let ok = Verify::Ok {
        cluster: (u16::from_le_bytes([e[20], e[21]]) as u32) << 16 | u16::from_le_bytes([e[26], e[27]]) as u32,
        size: u32::from_le_bytes([e[28], e[29], e[30], e[31]]),
        attr: e[11],
    };
    // Only long names are indexed: a file with none (0.5% of the stock
    // card's names fit 8.3 exactly) is opened by path.
    let nent = name.len().div_ceil(13);
    if nent == 0 || nent > 20 || nent > slot + 16 {
        return Verify::Mismatch;
    }
    if nent > slot && prev.is_none() {
        return Verify::NeedPrev;
    }
    let s = sum(e);
    for k in 1..=nent {
        let l = if k <= slot {
            &cur[(slot - k) * 32..][..32]
        } else {
            &prev.unwrap()[(16 + slot - k) * 32..][..32]
        };
        let ord = k as u8 | if k == nent { 0x40 } else { 0 };
        if l[11] & 0x3F != 0x0F || l[13] != s || l[0] != ord {
            return Verify::Mismatch;
        }
        for (j, &o) in LFN_OFS.iter().enumerate() {
            let pos = (k - 1) * 13 + j;
            let want = match pos.cmp(&name.len()) {
                std::cmp::Ordering::Less => name[pos].to_ascii_lowercase() as u16,
                std::cmp::Ordering::Equal => 0,
                std::cmp::Ordering::Greater => 0xFFFF,
            };
            let got = u16::from_le_bytes([l[o], l[o + 1]]);
            let got = if got < 0x80 { (got as u8).to_ascii_lowercase() as u16 } else { got };
            if got != want {
                return Verify::Mismatch;
            }
        }
    }
    ok
}

/// The cluster after `cluster` in its FAT32 chain (`None` at the end).
pub fn next_cluster(
    geo: &Geometry,
    cluster: u32,
    read: &mut dyn FnMut(u32, &mut [u8]) -> io::Result<()>,
    buf: &mut [u8],
) -> io::Result<Option<u32>> {
    read(geo.fatbase + cluster / 128, &mut buf[..SS])?;
    let o = (cluster as usize % 128) * 4;
    let v = u32::from_le_bytes([buf[o], buf[o + 1], buf[o + 2], buf[o + 3]]) & 0x0FFF_FFFF;
    Ok((v >= 2 && v < geo.n_fatent).then_some(v))
}

/// One file found by [`walk`]: its name, location word, and the sector
/// before its own when the long name starts in the previous cluster, which
/// a record could not locate (the device leaves those out: they open by
/// path, ~0.3% of a stock card).
pub struct Found<'a> {
    pub name: &'a [u8],
    pub loc: u32,
    pub prev_rel: Option<u32>,
}

/// One subdirectory found by [`walk_dirs`]: its name as FatFs lists it (the
/// long name, else the 8.3 name with its NT case bits) and first cluster.
pub struct DirFound<'a> {
    pub name: &'a [u8],
    pub cluster: u32,
}

/// An entry [`scan`] found: a live file or directory with an ASCII name.
struct Entry<'a> {
    name: &'a [u8],
    /// The name is a long one (else the 8.3 name).
    long: bool,
    dir: bool,
    cluster: u32,
    loc: Option<u32>,
    prev_rel: Option<u32>,
}

/// The 8.3 name of entry `e` as FatFs shows it (`NAME.EXT`, the NT case
/// bits applied) into `out`: its length, `None` if not ASCII.
fn short_name(e: &[u8], out: &mut [u8; 12]) -> Option<usize> {
    let mut n = 0;
    let base = e[..8].iter().rposition(|&b| b != b' ').map_or(0, |p| p + 1);
    let ext = e[8..11].iter().rposition(|&b| b != b' ').map_or(0, |p| p + 1);
    for (i, &b) in e[..base].iter().enumerate() {
        // 0x05 stands for a leading 0xE5 (not ASCII either way).
        let b = if i == 0 && b == 0x05 { 0xE5 } else { b };
        out[n] = if e[12] & 0x08 != 0 { b.to_ascii_lowercase() } else { b };
        n += 1;
    }
    if ext > 0 {
        out[n] = b'.';
        n += 1;
        for &b in &e[8..8 + ext] {
            out[n] = if e[12] & 0x10 != 0 { b.to_ascii_lowercase() } else { b };
            n += 1;
        }
    }
    out[..n].is_ascii().then_some(n)
}

/// Every live entry of a directory, from its first cluster, in directory
/// order. The first read is one sector (a copy's bucket rarely fills more),
/// then batches of `scratch`'s size; the FAT sector between clusters is
/// read into `scratch` too. Allocates nothing.
fn scan(
    geo: &Geometry,
    first: u32,
    read: &mut dyn FnMut(u32, &mut [u8]) -> io::Result<()>,
    scratch: &mut [u8],
    emit: &mut dyn FnMut(Entry) -> io::Result<()>,
) -> io::Result<u32> {
    let batch = (scratch.len() / SS).max(1) as u32;
    let mut lfn = [0u8; 260];
    let mut sfn = [0u8; 12];
    let (mut want, mut chk, mut nent, mut ascii) = (0u8, 0u8, 0usize, true);
    let mut sectors = 0;
    let mut prev_rel = u32::MAX;
    let mut cluster = Some(first);
    let mut step = 1;
    while let Some(c) = cluster {
        if c < 2 || c >= geo.n_fatent {
            return Err(io::Error::other("directory cluster out of range"));
        }
        let first_rel = geo.rel(c);
        let mut s = 0;
        while s < geo.csize {
            let n = step.min(geo.csize - s);
            step = batch;
            read(geo.database + first_rel + s, &mut scratch[..n as usize * SS])?;
            sectors += n;
            for k in 0..n {
                let rel = first_rel + s + k;
                let sec = &scratch[k as usize * SS..][..SS];
                for slot in 0..16 {
                    let e = &sec[slot * 32..][..32];
                    match e[0] {
                        0x00 => return Ok(sectors),
                        0xE5 => {
                            want = 0;
                            continue;
                        }
                        _ => {}
                    }
                    if e[11] & 0x3F == 0x0F {
                        let ord = e[0] & 0x3F;
                        if e[0] & 0x40 != 0 && (1..=20).contains(&ord) {
                            (chk, nent, ascii) = (e[13], ord as usize, true);
                            lfn[..ord as usize * 13].fill(0);
                        } else if want == 0 || ord != want || e[13] != chk {
                            want = 0;
                            nent = 0;
                            continue;
                        }
                        for (j, &o) in LFN_OFS.iter().enumerate() {
                            let u = u16::from_le_bytes([e[o], e[o + 1]]);
                            let at = (ord as usize - 1) * 13 + j;
                            match u {
                                0 | 0xFFFF => {}
                                1..=0x7F => lfn[at] = u as u8,
                                _ => ascii = false,
                            }
                        }
                        want = ord - 1;
                        continue;
                    }
                    let long = want == 0 && nent > 0 && chk == sum(e);
                    let lfn_n = std::mem::take(&mut nent);
                    want = 0;
                    // Live, not a long-name entry, not the volume label.
                    if e[11] & 0x08 != 0 {
                        continue;
                    }
                    let cluster = (u16::from_le_bytes([e[20], e[21]]) as u32) << 16 | u16::from_le_bytes([e[26], e[27]]) as u32;
                    let dir = e[11] & 0x10 != 0;
                    // FatFs shows the long name when there is one: one that is
                    // not ASCII is left out, not shown by its 8.3 alias.
                    let name: &[u8] = if long {
                        if !ascii {
                            continue;
                        }
                        let end = lfn[..lfn_n * 13].iter().position(|&b| b == 0).unwrap_or(lfn_n * 13);
                        &lfn[..end]
                    } else {
                        match short_name(e, &mut sfn) {
                            Some(n) => &sfn[..n],
                            None => continue,
                        }
                    };
                    // The long name reaches back past this sector: fine
                    // inside a cluster (the sector before is rel - 1), an
                    // extra record across a cluster boundary, and not
                    // indexed past one sector back.
                    let back = if long { lfn_n.saturating_sub(slot) } else { 0 };
                    let loc = if back > 16 { None } else { loc(rel, slot) };
                    let prev = (back > 0 && rel.is_multiple_of(geo.csize)).then_some(prev_rel);
                    emit(Entry { name, long, dir, cluster, loc, prev_rel: prev })?;
                }
                prev_rel = rel;
            }
            s += n;
        }
        cluster = next_cluster(geo, c, read, &mut scratch[..SS])?;
    }
    Ok(sectors)
}

/// Walk a directory from its first cluster: every file with an ASCII long
/// name, in directory order. `read(lba, buf)` fills `buf` (a whole
/// number of sectors); `scratch` is a multiple of [`SS`] (the batch size).
/// Returns the sectors read. Allocates nothing.
pub fn walk(
    geo: &Geometry,
    first: u32,
    read: &mut dyn FnMut(u32, &mut [u8]) -> io::Result<()>,
    scratch: &mut [u8],
    emit: &mut dyn FnMut(Found) -> io::Result<()>,
) -> io::Result<u32> {
    scan(geo, first, read, scratch, &mut |e| match e {
        // Only long names are indexed (a name that fits 8.3 opens by path).
        Entry { dir: false, long: true, loc: Some(loc), name, prev_rel, .. } => emit(Found { name, loc, prev_rel }),
        _ => Ok(()),
    })
}

/// Every subdirectory of a directory (not `.` or `..`) with an ASCII name,
/// in directory order, as [`walk`] reads them. Allocates nothing: a caller
/// walks what it finds with a scratch of its own.
pub fn walk_dirs(
    geo: &Geometry,
    first: u32,
    read: &mut dyn FnMut(u32, &mut [u8]) -> io::Result<()>,
    scratch: &mut [u8],
    emit: &mut dyn FnMut(DirFound) -> io::Result<()>,
) -> io::Result<u32> {
    scan(geo, first, read, scratch, &mut |e| match e {
        Entry { dir: true, name, cluster, .. } if name != b"." && name != b".." && (2..geo.n_fatent).contains(&cluster) => {
            emit(DirFound { name, cluster })
        }
        _ => Ok(()),
    })
}

/// A directory the index covers ([`each_indexed_dir`]): its card path
/// (`/gifs/<folder>` or `/cache/<category>/<bucket>`), key ([`hash`] of
/// the path) and first cluster.
pub struct IndexedDir<'a> {
    pub path: &'a str,
    pub key: u32,
    pub first: u32,
    /// A `/gifs/` folder (else a copies' bucket).
    pub gif: bool,
}

/// `read(lba, buf)`: fill `buf` (whole sectors) from `lba`.
pub type ReadSectors<'a> = dyn FnMut(u32, &mut [u8]) -> io::Result<()> + 'a;

/// The longest path [`each_indexed_dir`] builds: `/cache/` and two names
/// of 255.
const PATH_MAX: usize = 7 + 255 + 1 + 255;

/// Every directory the index covers, read raw from the card: each
/// `/gifs/<folder>` (from `/gifs`'s first cluster, `gifs`), then each copy
/// bucket `/cache/<category>/<bucket>` (`cache`; categories starting with
/// `.` are not the cache's). `f(dir, read)` gets the directory and `read`
/// to walk it with (a scratch of its own). `bufs` (two sectors) holds the
/// two directory levels being read. Streams: nothing is kept or allocated,
/// whatever the number of folders and buckets (a card's cache has up to 256
/// buckets a category: kept as a list they were the rig's 64 KB allocation
/// that failed). Directories with a name that is not ASCII are left out
/// (their files open by path).
pub fn each_indexed_dir(
    geo: &Geometry,
    gifs: Option<u32>,
    cache: Option<u32>,
    read: &mut ReadSectors,
    bufs: &mut [u8],
    f: &mut dyn FnMut(&IndexedDir, &mut ReadSectors) -> io::Result<()>,
) -> io::Result<()> {
    // Two sectors, then the two paths: off the stack (the device's build
    // task runs FatFs and the SD driver under three levels of this).
    if bufs.len() < INDEXED_DIR_BUFS {
        return Err(io::Error::other("each_indexed_dir: bufs too short"));
    }
    let (b0, rest) = bufs.split_at_mut(SS);
    let (b1, rest) = rest.split_at_mut(SS);
    let (path, rest) = rest.split_at_mut(PATH_MAX);
    let sub = &mut rest[..PATH_MAX];
    // One reader shared by the nested walks (each read is a call of its own).
    let read = std::cell::RefCell::new(read);
    let mut r0 = |l: u32, b: &mut [u8]| (read.borrow_mut())(l, b);
    if let Some(first) = gifs {
        walk_dirs(geo, first, &mut r0, b0, &mut |d| {
            let n = put(path, 0, b"/gifs/");
            let n = put(path, n, d.name);
            // ASCII (`scan`), so a str.
            let p = std::str::from_utf8(&path[..n]).map_err(io::Error::other)?;
            let dir = IndexedDir { path: p, key: hash(p.as_bytes()), first: d.cluster, gif: true };
            f(&dir, &mut |l, b| (read.borrow_mut())(l, b))
        })?;
    }
    if let Some(first) = cache {
        walk_dirs(geo, first, &mut r0, b0, &mut |cat| {
            if cat.name.starts_with(b".") {
                return Ok(());
            }
            let n = put(sub, 0, b"/cache/");
            let n = put(sub, n, cat.name);
            let n = put(sub, n, b"/");
            let mut r1 = |l: u32, b: &mut [u8]| (read.borrow_mut())(l, b);
            walk_dirs(geo, cat.cluster, &mut r1, b1, &mut |bucket| {
                let m = put(sub, n, bucket.name);
                let p = std::str::from_utf8(&sub[..m]).map_err(io::Error::other)?;
                let dir = IndexedDir { path: p, key: hash(p.as_bytes()), first: bucket.cluster, gif: false };
                f(&dir, &mut |l, b| (read.borrow_mut())(l, b))
            })
            .map(drop)
        })?;
    }
    Ok(())
}

/// The bytes [`each_indexed_dir`]'s `bufs` must have: two directory
/// sectors and two paths.
pub const INDEXED_DIR_BUFS: usize = 2 * SS + 2 * PATH_MAX;

fn put(buf: &mut [u8], at: usize, s: &[u8]) -> usize {
    let n = s.len().min(buf.len() - at);
    buf[at..at + n].copy_from_slice(&s[..n]);
    at + n
}

/// The card's [`Walker`]: every directory [`each_indexed_dir`] finds, its
/// files walked with `scratch`. Holds nothing but what it is lent.
pub struct CardWalker<'a> {
    pub geo: Geometry,
    /// `/gifs`'s and `/cache`'s first clusters.
    pub gifs: Option<u32>,
    pub cache: Option<u32>,
    pub read: &'a mut dyn FnMut(u32, &mut [u8]) -> io::Result<()>,
    /// [`INDEXED_DIR_BUFS`]: the directory levels and paths.
    pub bufs: &'a mut [u8],
    /// The files' batch.
    pub scratch: &'a mut [u8],
    /// [`Walker::between`].
    pub between: &'a mut dyn FnMut() -> io::Result<()>,
}

impl Walker for CardWalker<'_> {
    fn walk(&mut self, dir: &mut dyn FnMut(u32, u32), rec: &mut dyn FnMut(u32, u32)) -> io::Result<()> {
        let CardWalker { geo, gifs, cache, read, bufs, scratch, .. } = self;
        each_indexed_dir(geo, *gifs, *cache, *read, bufs, &mut |d, read| {
            dir(d.key, d.first);
            let seed = hash_more(d.key, b"/");
            walk(geo, d.first, read, scratch, &mut |f| {
                if f.prev_rel.is_none() {
                    rec(hash_more(seed, f.name), f.loc);
                }
                Ok(())
            })
            .map(drop)
        })
    }

    fn dirs(&mut self, dir: &mut dyn FnMut(u32, u32)) -> io::Result<()> {
        let CardWalker { geo, gifs, cache, read, bufs, .. } = self;
        each_indexed_dir(geo, *gifs, *cache, *read, bufs, &mut |d, _| {
            dir(d.key, d.first);
            Ok(())
        })
    }

    fn between(&mut self) -> io::Result<()> {
        (self.between)()
    }
}

/// The index header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    pub count: u32,
    pub n_fatent: u32,
    pub database: u32,
    /// Directories indexed: `(hash of the card path, first cluster)`
    /// pairs after the records.
    pub dirs: u32,
}

impl Header {
    pub fn encode(&self) -> [u8; HEADER] {
        let mut h = [0u8; HEADER];
        h[..4].copy_from_slice(MAGIC);
        h[4..6].copy_from_slice(&VERSION.to_le_bytes());
        for (i, v) in [self.count, self.n_fatent, self.database, self.dirs].iter().enumerate() {
            h[8 + i * 4..12 + i * 4].copy_from_slice(&v.to_le_bytes());
        }
        h
    }

    pub fn decode(h: &[u8]) -> Option<Header> {
        if h.len() < HEADER || &h[..4] != MAGIC || h[4..6] != VERSION.to_le_bytes() {
            return None;
        }
        let w = |i: usize| u32::from_le_bytes(h[8 + i * 4..12 + i * 4].try_into().unwrap());
        Some(Header { count: w(0), n_fatent: w(1), database: w(2), dirs: w(3) })
    }
}

/// What [`build_index`] walks: the indexed directories, again and again.
pub trait Walker {
    /// Walk every indexed directory: `dir(key, first cluster)` for each,
    /// then `rec(hash, loc)` for each of its files.
    fn walk(&mut self, dir: &mut dyn FnMut(u32, u32), rec: &mut dyn FnMut(u32, u32)) -> io::Result<()>;
    /// Every indexed directory alone (`dir(key, first cluster)`), as
    /// [`Walker::walk`] finds them.
    fn dirs(&mut self, dir: &mut dyn FnMut(u32, u32)) -> io::Result<()>;
    /// Between walks: `Err` stops the build (the device: the heap the player
    /// needs is short). The default goes on.
    fn between(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// The error of a build whose directories changed between its walks.
pub const DIRS_CHANGED: &str = "the indexed directories changed during the build";

/// Write the index: the first walk counts; then each walk gathers the
/// buckets that fit `group` (8 bytes a record) and writes them, so no
/// temporary file and no sort; then the directories, streamed. Every walk
/// must find the same directories (a [`DirSet`] each), or the build stops
/// with [`DIRS_CHANGED`]. Allocates nothing: what it holds is `group` and a
/// few KB of stack, whatever the card holds. Returns (records, walks).
pub fn build_index(
    out: &mut dyn FnMut(&[u8]) -> io::Result<()>,
    geo: &Geometry,
    walker: &mut dyn Walker,
    group: &mut [u8],
) -> io::Result<(u32, u32)> {
    let mut fanout = [0u32; FANOUT];
    let mut set = DirSet::default();
    walker.walk(&mut |k, c| set.add((k, c)), &mut |h, _| fanout[(h >> 24) as usize + 1] += 1)?;
    for b in 0..256 {
        fanout[b + 1] += fanout[b];
    }
    let changed = || io::Error::other(DIRS_CHANGED);
    let h = Header {
        count: fanout[256],
        n_fatent: geo.n_fatent,
        database: geo.database,
        dirs: set.n,
    };
    out(&h.encode())?;
    for v in fanout {
        out(&v.to_le_bytes())?;
    }
    let cap = (group.len() / 8) as u32;
    let (mut b0, mut walks) = (0usize, 1);
    while b0 < 256 {
        let mut b1 = b0;
        while b1 < 256 && fanout[b1 + 1] - fanout[b0] <= cap {
            b1 += 1;
        }
        if b1 == b0 {
            return Err(io::Error::other("index bucket larger than a pass"));
        }
        if fanout[b1] > fanout[b0] {
            walker.between()?;
            let mut fill = [0u32; 256];
            // A file gone since the count leaves its slot zero (a record no
            // lookup's key matches but bucket 0's key 0, which then fails
            // its entry check): no stale record from the last pass.
            group[..(fanout[b1] - fanout[b0]) as usize * 8].fill(0);
            let mut again = DirSet::default();
            walks += 1;
            walker.walk(&mut |k, c| again.add((k, c)), &mut |h, l| {
                let b = (h >> 24) as usize;
                if (b0..b1).contains(&b) && fill[b] < fanout[b + 1] - fanout[b] {
                    let at = (fanout[b] - fanout[b0] + fill[b]) as usize * 8;
                    group[at..at + 4].copy_from_slice(&h.to_le_bytes());
                    group[at + 4..at + 8].copy_from_slice(&l.to_le_bytes());
                    fill[b] += 1;
                }
            })?;
            // A directory added, gone or moved since the count: its
            // records may be in no walk, or in the wrong directory's check.
            if again != set {
                return Err(changed());
            }
            out(&group[..(fanout[b1] - fanout[b0]) as usize * 8])?;
        }
        b0 = b1;
    }
    let mut last = DirSet::default();
    let mut failed = None;
    walker.dirs(&mut |k, c| {
        last.add((k, c));
        if failed.is_none() {
            failed = out(&k.to_le_bytes()).and_then(|()| out(&c.to_le_bytes())).err();
        }
    })?;
    if let Some(e) = failed {
        return Err(e);
    }
    if last != set {
        return Err(changed());
    }
    Ok((h.count, walks))
}

/// Are the directories indexed (`(hash of the card path, first cluster)`)
/// the ones on the card now? A directory renamed, replaced or added on a PC
/// keeps its entries where they were, so [`verify`] alone would accept a
/// record that now names a file in another directory.
/// The directories an index was built over, as (key, first cluster)
/// pairs, summed so two sets compare without keeping either (a card has up
/// to 256 copy buckets a category): order does not matter.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct DirSet {
    n: u32,
    sum: u32,
    xor: u32,
}

impl DirSet {
    pub fn add(&mut self, (key, cluster): (u32, u32)) {
        let m = hash_more(key, &cluster.to_le_bytes());
        self.n += 1;
        self.sum = self.sum.wrapping_add(m);
        self.xor ^= m.rotate_left(7) ^ key;
    }
}

pub fn dirs_match(indexed: &[(u32, u32)], now: &[(u32, u32)]) -> bool {
    indexed.len() == now.len() && now.iter().all(|d| indexed.contains(d))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dir_sets_compare_whatever_the_order() {
        let (mut a, mut b, mut c) = (DirSet::default(), DirSet::default(), DirSet::default());
        for p in [(1, 10), (2, 20), (3, 30)] {
            a.add(p);
        }
        for p in [(3, 30), (1, 10), (2, 20)] {
            b.add(p);
        }
        for p in [(1, 10), (2, 20), (3, 31)] {
            c.add(p);
        }
        assert_eq!(a, b);
        assert_ne!(a, c, "a bucket's cluster reused elsewhere");
        let mut d = DirSet::default();
        d.add((1, 10));
        d.add((2, 20));
        assert_ne!(a, d, "one missing");
    }

    fn lfn_entry(ord: u8, chk: u8, chars: &[u16]) -> [u8; 32] {
        let mut e = [0u8; 32];
        e[0] = ord;
        e[11] = 0x0F;
        e[13] = chk;
        for (j, &o) in LFN_OFS.iter().enumerate() {
            e[o..o + 2].copy_from_slice(&chars[j].to_le_bytes());
        }
        e
    }

    /// A directory sector pair holding `name` (long) ending at `slot` of
    /// the second sector, with a given 8.3 name.
    fn place(name: &str, sfn: &[u8; 11], slot: usize) -> ([u8; SS], [u8; SS]) {
        let (mut prev, mut cur) = ([0u8; SS], [0u8; SS]);
        let mut e = [0u8; 32];
        e[..11].copy_from_slice(sfn);
        e[11] = 0x20;
        e[26] = 7;
        e[28] = 100;
        cur[slot * 32..slot * 32 + 32].copy_from_slice(&e);
        let s = sum(&e);
        let n: Vec<u8> = name.bytes().collect();
        let nent = n.len().div_ceil(13);
        for k in 1..=nent {
            let chars: Vec<u16> = (0..13)
                .map(|j| {
                    let p = (k - 1) * 13 + j;
                    match p.cmp(&n.len()) {
                        std::cmp::Ordering::Less => n[p] as u16,
                        std::cmp::Ordering::Equal => 0,
                        std::cmp::Ordering::Greater => 0xFFFF,
                    }
                })
                .collect();
            let l = lfn_entry(k as u8 | if k == nent { 0x40 } else { 0 }, s, &chars);
            if k <= slot {
                cur[(slot - k) * 32..][..32].copy_from_slice(&l);
            } else {
                prev[(16 + slot - k) * 32..][..32].copy_from_slice(&l);
            }
        }
        (prev, cur)
    }

    #[test]
    fn verifies_a_long_name_in_one_sector_and_across_two() {
        let name = "NES_Super_Mario_Bros_3_Title.gif";
        let (p, c) = place(name, b"NES_SU~1GIF", 5);
        assert_eq!(verify(&c, None, 5, name.as_bytes()), Verify::Ok { cluster: 7, size: 100, attr: 0x20 });
        assert_eq!(verify(&c, None, 5, b"NES_Super_Mario_Bros_4_Title.gif"), Verify::Mismatch);
        assert_eq!(verify(&c, None, 5, b"NES_Super_Mario_Bros_3_Titl.gif"), Verify::Mismatch);
        let (p2, c2) = place(name, b"NES_SU~1GIF", 1);
        assert_eq!(verify(&c2, None, 1, name.as_bytes()), Verify::NeedPrev);
        assert_eq!(verify(&c2, Some(&p2), 1, name.as_bytes()), Verify::Ok { cluster: 7, size: 100, attr: 0x20 });
        // The wrong sector before: no match.
        assert_eq!(verify(&c2, Some(&p), 1, name.as_bytes()), Verify::Mismatch);
    }

    #[test]
    fn short_names_alone_are_not_indexed_and_deleted_entries_never_match() {
        let mut c = [0u8; SS];
        c[..11].copy_from_slice(b"FI0001  GIF");
        c[11] = 0x20;
        // No long name before it: it opens by path.
        assert_eq!(verify(&c, Some(&[0u8; SS]), 0, b"FI0001.GIF"), Verify::Mismatch);
        let name = "A_long_name_here.gif";
        let (p, mut c) = place(name, b"A_LONG~1GIF", 3);
        assert!(matches!(verify(&c, Some(&p), 3, name.as_bytes()), Verify::Ok { .. }));
        c[3 * 32] = 0xE5;
        assert_eq!(verify(&c, Some(&p), 3, name.as_bytes()), Verify::Mismatch);
    }

    /// A walker over made-up records: `n` files in one directory (9, 10);
    /// `moved` moves that directory after the first walk.
    struct Fake {
        n: u32,
        walks: u32,
        moved: bool,
    }

    impl Walker for Fake {
        fn walk(&mut self, dir: &mut dyn FnMut(u32, u32), rec: &mut dyn FnMut(u32, u32)) -> io::Result<()> {
            dir(9, if self.moved && self.walks > 0 { 11 } else { 10 });
            self.walks += 1;
            for i in 0..self.n {
                rec(hash(format!("/gifs/A/{i}.gif").as_bytes()), i);
            }
            Ok(())
        }
        fn dirs(&mut self, dir: &mut dyn FnMut(u32, u32)) -> io::Result<()> {
            dir(9, 10);
            Ok(())
        }
    }

    #[test]
    fn builds_an_index_in_bucket_order_over_several_walks() {
        let key = |i: u32| hash(format!("/gifs/A/{i}.gif").as_bytes());
        let geo = Geometry { csize: 64, fatbase: 32, database: 1000, n_fatent: 9999 };
        let mut out = Vec::new();
        let mut group = vec![0u8; 512 * 8];
        let mut put = |b: &[u8]| -> io::Result<()> {
            out.extend_from_slice(b);
            Ok(())
        };
        let mut w = Fake { n: 3000, walks: 0, moved: false };
        let (count, walks) = build_index(&mut put, &geo, &mut w, &mut group).unwrap();
        assert_eq!(count, 3000);
        assert!(walks >= 7, "{walks}");
        assert_eq!(walks, w.walks);
        let h = Header::decode(&out).unwrap();
        assert_eq!((h.count, h.database, h.dirs), (3000, 1000, 1));
        let fan = |b: usize| u32::from_le_bytes(out[HEADER + b * 4..][..4].try_into().unwrap());
        for i in 0..3000u32 {
            let k = key(i);
            let b = (k >> 24) as usize;
            let found = (fan(b)..fan(b + 1)).any(|r| {
                let at = RECORDS_AT as usize + r as usize * 8;
                out[at..at + 4] == k.to_le_bytes() && out[at + 4..at + 8] == i.to_le_bytes()
            });
            assert!(found, "{i}");
        }
        assert_eq!(out.len(), RECORDS_AT as usize + 3000 * 8 + 8);
        assert_eq!(out[out.len() - 8..], [9, 0, 0, 0, 10, 0, 0, 0]);
    }

    #[test]
    fn a_directory_moved_between_walks_stops_the_build() {
        let geo = Geometry { csize: 64, fatbase: 32, database: 1000, n_fatent: 9999 };
        let mut put = |_: &[u8]| -> io::Result<()> { Ok(()) };
        let mut group = vec![0u8; 512 * 8];
        let e = build_index(&mut put, &geo, &mut Fake { n: 3000, walks: 0, moved: true }, &mut group).unwrap_err();
        assert_eq!(e.to_string(), DIRS_CHANGED);
    }

    #[test]
    fn a_stop_between_walks_ends_the_build_with_its_error() {
        struct Short(Fake);
        impl Walker for Short {
            fn walk(&mut self, d: &mut dyn FnMut(u32, u32), r: &mut dyn FnMut(u32, u32)) -> io::Result<()> {
                self.0.walk(d, r)
            }
            fn dirs(&mut self, d: &mut dyn FnMut(u32, u32)) -> io::Result<()> {
                self.0.dirs(d)
            }
            fn between(&mut self) -> io::Result<()> {
                if self.0.walks >= 3 { Err(io::Error::other("heap short")) } else { Ok(()) }
            }
        }
        let geo = Geometry { csize: 64, fatbase: 32, database: 1000, n_fatent: 9999 };
        let mut put = |_: &[u8]| -> io::Result<()> { Ok(()) };
        let mut group = vec![0u8; 512 * 8];
        let mut w = Short(Fake { n: 3000, walks: 0, moved: false });
        assert_eq!(build_index(&mut put, &geo, &mut w, &mut group).unwrap_err().to_string(), "heap short");
        assert_eq!(w.0.walks, 3);
    }

    #[test]
    fn short_names_read_as_fatfs_shows_them() {
        let mut out = [0u8; 12];
        let mut e = [0u8; 32];
        e[..11].copy_from_slice(b"1B         ");
        e[12] = 0x08; // NT: lower-case name
        let n = short_name(&e, &mut out).unwrap();
        assert_eq!(&out[..n], b"1b");
        e[..11].copy_from_slice(b"ARCADE  GIF");
        e[12] = 0x10; // NT: lower-case extension
        let n = short_name(&e, &mut out).unwrap();
        assert_eq!(&out[..n], b"ARCADE.gif");
        e[0] = 0x05;
        assert_eq!(short_name(&e, &mut out), None, "0xE5 first: not ASCII");
    }

    #[test]
    fn names_compare_without_regard_to_ascii_case_as_fatfs_does() {
        let name = "NES_Super_Mario_Bros_3_Title.gif";
        let (_, c) = place(name, b"NES_SU~1GIF", 5);
        assert!(matches!(verify(&c, None, 5, b"nes_super_mario_bros_3_title.GIF"), Verify::Ok { .. }));
        assert_eq!(hash(b"/gifs/A/X.gif"), hash(b"/GIFS/a/x.GIF"));
        assert_eq!(hash_more(hash(b"/gifs/A/"), b"X.gif"), hash(b"/gifs/a/x.gif"));
    }

    #[test]
    fn a_255_character_name_needs_twenty_entries() {
        let name: String = std::iter::repeat_n('n', 251).chain(".gif".chars()).collect();
        assert_eq!(name.len(), 255);
        // 20 entries before the 8.3 one: slot 15 of the sector, 15 entries
        // in it and 5 in the sector before.
        let (p, c) = place(&name, b"NNNNNN~1GIF", 15);
        assert_eq!(verify(&c, None, 15, name.as_bytes()), Verify::NeedPrev);
        assert!(matches!(verify(&c, Some(&p), 15, name.as_bytes()), Verify::Ok { .. }));
        // A 21st entry is never a FAT long name.
        let long: String = std::iter::repeat_n('n', 256).collect();
        assert_eq!(verify(&c, Some(&p), 15, long.as_bytes()), Verify::Mismatch);
    }

    #[test]
    fn a_bucket_larger_than_the_group_is_an_error() {
        struct One;
        impl Walker for One {
            fn walk(&mut self, _: &mut dyn FnMut(u32, u32), rec: &mut dyn FnMut(u32, u32)) -> io::Result<()> {
                for i in 0..10u32 {
                    rec(0x0700_0000 | i, i); // one bucket, 10 records
                }
                Ok(())
            }
            fn dirs(&mut self, _: &mut dyn FnMut(u32, u32)) -> io::Result<()> {
                Ok(())
            }
        }
        let geo = Geometry { csize: 1, fatbase: 1, database: 2, n_fatent: 100 };
        let mut put = |_: &[u8]| -> io::Result<()> { Ok(()) };
        let mut group = vec![0u8; 8 * 4];
        assert!(build_index(&mut put, &geo, &mut One, &mut group).is_err());
    }

    #[test]
    fn another_version_or_magic_is_refused() {
        let h = Header { count: 1, n_fatent: 5, database: 6, dirs: 2 };
        let mut b = h.encode();
        assert_eq!(Header::decode(&b), Some(h));
        b[4..6].copy_from_slice(&2u16.to_le_bytes());
        assert_eq!(Header::decode(&b), None);
        let mut b = h.encode();
        b[0] = b'X';
        assert_eq!(Header::decode(&b), None);
        assert_eq!(Header::decode(&h.encode()[..HEADER - 1]), None);
    }

    #[test]
    fn the_directory_list_must_match_exactly() {
        let a = [(1, 10), (2, 20)];
        assert!(dirs_match(&a, &[(2, 20), (1, 10)]));
        assert!(!dirs_match(&a, &[(1, 10), (2, 21)])); // a directory replaced
        assert!(!dirs_match(&a, &[(1, 10)])); // one gone
        assert!(!dirs_match(&a, &[(1, 10), (2, 20), (3, 30)])); // one added
    }
}

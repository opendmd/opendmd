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
/// the slot (0-15) of the 8.3 entry in it. `None` past 128 GiB (less a sector).
pub fn loc(rel: u32, slot: usize) -> Option<u32> {
    // (Never all ones: the build's temporary records mark a directory so.)
    (rel < (1 << 28) - 1).then_some(rel << 4 | slot as u32)
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

/// Directory entries in order, one at a time: the long-name state between
/// them (it may span sectors and clusters). Starts clean, so a walk may
/// begin at a directory's start or just after any 8.3 entry.
struct Decoder {
    lfn: [u8; 260],
    sfn: [u8; 12],
    want: u8,
    chk: u8,
    nent: usize,
    ascii: bool,
}

/// What [`Decoder::feed`] made of an entry.
enum Fed<'a> {
    /// The directory's end (an entry starting 0x00).
    End,
    /// Nothing to show (a long-name part, a deleted entry, the label, a
    /// name that is not ASCII).
    Skip,
    Entry(Entry<'a>),
}

impl Decoder {
    fn new() -> Decoder {
        Decoder { lfn: [0; 260], sfn: [0; 12], want: 0, chk: 0, nent: 0, ascii: true }
    }

    /// Entry `e`, slot `slot` of the sector `rel` (from the data region);
    /// `prev_rel` the sector read before it.
    fn feed(&mut self, geo: &Geometry, e: &[u8], rel: u32, slot: usize, prev_rel: u32) -> Fed<'_> {
        match e[0] {
            0x00 => return Fed::End,
            0xE5 => {
                self.want = 0;
                return Fed::Skip;
            }
            _ => {}
        }
        if e[11] & 0x3F == 0x0F {
            let ord = e[0] & 0x3F;
            if e[0] & 0x40 != 0 && (1..=20).contains(&ord) {
                (self.chk, self.nent, self.ascii) = (e[13], ord as usize, true);
                self.lfn[..ord as usize * 13].fill(0);
            } else if self.want == 0 || ord != self.want || e[13] != self.chk {
                self.want = 0;
                self.nent = 0;
                return Fed::Skip;
            }
            for (j, &o) in LFN_OFS.iter().enumerate() {
                let u = u16::from_le_bytes([e[o], e[o + 1]]);
                let at = (ord as usize - 1) * 13 + j;
                match u {
                    0 | 0xFFFF => {}
                    1..=0x7F => self.lfn[at] = u as u8,
                    _ => self.ascii = false,
                }
            }
            self.want = ord - 1;
            return Fed::Skip;
        }
        let long = self.want == 0 && self.nent > 0 && self.chk == sum(e);
        let lfn_n = std::mem::take(&mut self.nent);
        self.want = 0;
        // Live, not a long-name entry, not the volume label.
        if e[11] & 0x08 != 0 {
            return Fed::Skip;
        }
        let cluster = (u16::from_le_bytes([e[20], e[21]]) as u32) << 16 | u16::from_le_bytes([e[26], e[27]]) as u32;
        let dir = e[11] & 0x10 != 0;
        // FatFs shows the long name when there is one: one that is not
        // ASCII is left out, not shown by its 8.3 alias.
        let name: &[u8] = if long {
            if !self.ascii {
                return Fed::Skip;
            }
            let end = self.lfn[..lfn_n * 13].iter().position(|&b| b == 0).unwrap_or(lfn_n * 13);
            &self.lfn[..end]
        } else {
            match short_name(e, &mut self.sfn) {
                Some(n) => &self.sfn[..n],
                None => return Fed::Skip,
            }
        };
        // The long name reaches back past this sector: fine inside a cluster
        // (the sector before is rel - 1), an extra record across a cluster
        // boundary, and not indexed past one sector back.
        let back = if long { lfn_n.saturating_sub(slot) } else { 0 };
        let loc = if back > 16 { None } else { loc(rel, slot) };
        let prev = (back > 0 && rel.is_multiple_of(geo.csize)).then_some(prev_rel);
        Fed::Entry(Entry { name, long, dir, cluster, loc, prev_rel: prev })
    }
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
    let mut dec = Decoder::new();
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
                    match dec.feed(geo, &sec[slot * 32..][..32], rel, slot, prev_rel) {
                        Fed::End => return Ok(sectors),
                        Fed::Skip => {}
                        Fed::Entry(e) => emit(e)?,
                    }
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

/// `read(lba, buf)`: fill `buf` (whole sectors) from `lba`.
pub type ReadSectors<'a> = dyn FnMut(u32, &mut [u8]) -> io::Result<()> + 'a;

/// A place in a directory that a walk can stop at and start again from:
/// an entry of a cluster (`index` 0..`csize * 16`; at the end, the chain is
/// followed). Always the directory's start or just after an 8.3 entry, so
/// no long name is cut in two.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DirPos {
    cluster: u32,
    index: u32,
    done: bool,
}

impl DirPos {
    pub fn start(first: u32) -> DirPos {
        DirPos { cluster: first, index: 0, done: false }
    }
}

/// The next subdirectory (not `.` or `..`, an ASCII name) from `pos`, and
/// `pos` moved past it: its name into `name` and (name length, first
/// cluster); `None` at the directory's end. `buf` is one sector, `buf_lba`
/// the sector it holds (`u32::MAX`: none), so a directory of short entries
/// (a category's buckets) is read once, not once a bucket. Allocates
/// nothing.
pub fn next_dir(
    geo: &Geometry,
    pos: &mut DirPos,
    read: &mut ReadSectors,
    buf: &mut [u8],
    buf_lba: &mut u32,
    name: &mut [u8; 255],
) -> io::Result<Option<(usize, u32)>> {
    let mut dec = Decoder::new();
    let per = geo.csize * 16;
    let mut prev_rel = u32::MAX;
    while !pos.done {
        if pos.index >= per {
            *buf_lba = u32::MAX;
            match next_cluster(geo, pos.cluster, read, &mut buf[..SS])? {
                Some(c) => *pos = DirPos::start(c),
                None => pos.done = true,
            }
            continue;
        }
        if pos.cluster < 2 || pos.cluster >= geo.n_fatent {
            return Err(io::Error::other("directory cluster out of range"));
        }
        let rel = geo.rel(pos.cluster) + pos.index / 16;
        let lba = geo.database + rel;
        if *buf_lba != lba {
            read(lba, &mut buf[..SS])?;
            *buf_lba = lba;
        }
        let slot = (pos.index % 16) as usize;
        let fed = dec.feed(geo, &buf[slot * 32..][..32], rel, slot, prev_rel);
        if slot == 15 {
            prev_rel = rel;
        }
        match fed {
            Fed::End => pos.done = true,
            Fed::Skip => pos.index += 1,
            Fed::Entry(e) => {
                pos.index += 1;
                if e.dir && e.name != b"." && e.name != b".." && (2..geo.n_fatent).contains(&e.cluster) {
                    let n = e.name.len().min(255);
                    name[..n].copy_from_slice(&e.name[..n]);
                    return Ok(Some((n, e.cluster)));
                }
            }
        }
    }
    Ok(None)
}

/// A directory the index covers, from [`Cursor::next`]: its key ([`hash`]
/// of its card path), first cluster, and whether it is a `/gifs/` folder
/// (its name then in the cursor's name buffer; else a copies' bucket).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NextDir {
    pub key: u32,
    pub first: u32,
    pub gif: bool,
    pub name_len: usize,
}

/// The directories the index covers, one at a time, resumable: every
/// `/gifs/<folder>` (from `/gifs`'s first cluster), then every copy bucket
/// `/cache/<category>/<bucket>` (categories starting with `.` are not the
/// cache's). Read raw; what it keeps is two places and two sectors, whatever
/// the card holds (a card's cache has up to 256 buckets a category: kept as
/// a list they were the rig's 64 KB allocation that failed, 2026-11-03).
/// Directories with a name that is not ASCII are left out (their files
/// open by path).
pub struct Cursor {
    gifs: Option<u32>,
    cache: Option<u32>,
    /// 0: `/gifs`; 1: `/cache`; 2: done.
    stage: u8,
    root: DirPos,
    /// The category being read and its key so far (`/cache/<cat>/`).
    cat: Option<(DirPos, u32)>,
    root_lba: u32,
    cat_lba: u32,
    root_buf: [u8; SS],
    cat_buf: [u8; SS],
    /// The last `/gifs/` folder's name, and a category's.
    pub name: [u8; 255],
    cat_name: [u8; 255],
}

impl Cursor {
    /// `gifs`, `cache`: those directories' first clusters (`None`: absent).
    pub fn new(gifs: Option<u32>, cache: Option<u32>) -> Cursor {
        let mut c = Cursor {
            gifs,
            cache,
            stage: 0,
            root: DirPos::start(0),
            cat: None,
            root_lba: u32::MAX,
            cat_lba: u32::MAX,
            root_buf: [0; SS],
            cat_buf: [0; SS],
            name: [0; 255],
            cat_name: [0; 255],
        };
        c.enter(0);
        c
    }

    fn enter(&mut self, stage: u8) {
        self.stage = stage;
        self.root_lba = u32::MAX;
        self.cat = None;
        match (stage, self.gifs, self.cache) {
            (0, Some(g), _) => self.root = DirPos::start(g),
            (0, None, _) => self.enter(1),
            (1, _, Some(c)) => self.root = DirPos::start(c),
            (1, _, None) => self.stage = 2,
            _ => self.stage = 2,
        }
    }

    /// Forget the sectors held: the card may have changed since (a walk
    /// stopped and started again later re-reads them).
    pub fn refresh(&mut self) {
        self.root_lba = u32::MAX;
        self.cat_lba = u32::MAX;
    }

    /// Whether every directory has been given.
    pub fn done(&self) -> bool {
        self.stage == 2
    }

    /// The next directory, `None` once all are given.
    pub fn next(&mut self, geo: &Geometry, read: &mut ReadSectors) -> io::Result<Option<NextDir>> {
        loop {
            match self.stage {
                0 => match next_dir(geo, &mut self.root, read, &mut self.root_buf, &mut self.root_lba, &mut self.name)? {
                    Some((n, first)) => {
                        let key = hash_more(hash(b"/gifs/"), &self.name[..n]);
                        return Ok(Some(NextDir { key, first, gif: true, name_len: n }));
                    }
                    None => self.enter(1),
                },
                1 => {
                    if let Some((pos, seed)) = self.cat.as_mut() {
                        let mut bucket = [0u8; 255];
                        match next_dir(geo, pos, read, &mut self.cat_buf, &mut self.cat_lba, &mut bucket)? {
                            Some((n, first)) => {
                                let key = hash_more(*seed, &bucket[..n]);
                                return Ok(Some(NextDir { key, first, gif: false, name_len: 0 }));
                            }
                            None => self.cat = None,
                        }
                        continue;
                    }
                    match next_dir(geo, &mut self.root, read, &mut self.root_buf, &mut self.root_lba, &mut self.cat_name)? {
                        Some((n, first)) => {
                            if self.cat_name[0] == b'.' {
                                continue;
                            }
                            let seed = hash_more(hash_more(hash(b"/cache/"), &self.cat_name[..n]), b"/");
                            self.cat = Some((DirPos::start(first), seed));
                            self.cat_lba = u32::MAX;
                        }
                        None => self.stage = 2,
                    }
                }
                _ => return Ok(None),
            }
        }
    }
}

/// The build's temporary records (`walk`'s output, read back by
/// [`gather`] once per group of buckets): 8 bytes each, a file's
/// `(hash, loc)`, or a directory: [`TMP_DIR`] then `(key, first cluster)`.
/// One walk of the card then serves every pass.
pub const TMP_DIR: [u8; 8] = [0xFF; 8];

/// `loc` never makes [`TMP_DIR`]'s location word (see [`loc`]).
pub fn tmp_record(h: u32, loc: u32) -> [u8; 8] {
    let mut r = [0u8; 8];
    r[..4].copy_from_slice(&h.to_le_bytes());
    r[4..].copy_from_slice(&loc.to_le_bytes());
    r
}

/// One pass over the temporary records (`read(buf)` gives the next bytes,
/// 0 at the end): the buckets from `b0` that fit `group` (8 bytes a
/// record), gathered in bucket order into `group`. Returns the bucket after
/// the last one gathered and the bytes of `group` filled. Every bucket's
/// count must match `fanout` (the walk's). Allocates nothing.
pub fn gather(
    read: &mut dyn FnMut(&mut [u8]) -> io::Result<usize>,
    fanout: &[u32; FANOUT],
    b0: usize,
    group: &mut [u8],
    chunk: &mut [u8],
) -> io::Result<(usize, usize)> {
    let cap = (group.len() / 8) as u32;
    let mut b1 = b0;
    while b1 < 256 && fanout[b1 + 1] - fanout[b0] <= cap {
        b1 += 1;
    }
    if b1 == b0 {
        return Err(io::Error::other("index bucket larger than a pass"));
    }
    let n = (fanout[b1] - fanout[b0]) as usize * 8;
    if n == 0 {
        return Ok((b1, 0));
    }
    let mut fill = [0u16; 256];
    let mut dir_next = false;
    let len = chunk.len() / 8 * 8;
    let chunk = &mut chunk[..len];
    let mut have = 0usize;
    loop {
        let got = read(&mut chunk[have..])?;
        let end = have + got;
        let whole = end / 8 * 8;
        for r in chunk[..whole].as_chunks::<8>().0 {
            if dir_next {
                dir_next = false;
                continue;
            }
            if *r == TMP_DIR {
                dir_next = true;
                continue;
            }
            let h = u32::from_le_bytes(r[..4].try_into().unwrap());
            let b = (h >> 24) as usize;
            if (b0..b1).contains(&b) {
                let f = &mut fill[b];
                if *f as u32 >= fanout[b + 1] - fanout[b] {
                    return Err(io::Error::other("temporary records past the count"));
                }
                let at = (fanout[b] - fanout[b0] + *f as u32) as usize * 8;
                group[at..at + 8].copy_from_slice(r);
                *f += 1;
            }
        }
        chunk.copy_within(whole..end, 0);
        have = end - whole;
        if got == 0 {
            break;
        }
    }
    if (b0..b1).any(|b| fill[b] as u32 != fanout[b + 1] - fanout[b]) {
        return Err(io::Error::other("temporary records short of the count"));
    }
    Ok((b1, n))
}

/// The directories in the temporary records, in order (`each(key,
/// cluster)`), for the index's trailer. Allocates nothing.
pub fn tmp_dirs(read: &mut dyn FnMut(&mut [u8]) -> io::Result<usize>, chunk: &mut [u8], each: &mut dyn FnMut(u32, u32) -> io::Result<()>) -> io::Result<()> {
    let len = chunk.len() / 8 * 8;
    let chunk = &mut chunk[..len];
    let mut have = 0usize;
    let mut dir_next = false;
    loop {
        let got = read(&mut chunk[have..])?;
        let end = have + got;
        let whole = end / 8 * 8;
        for r in chunk[..whole].as_chunks::<8>().0 {
            if dir_next {
                dir_next = false;
                each(u32::from_le_bytes(r[..4].try_into().unwrap()), u32::from_le_bytes(r[4..].try_into().unwrap()))?;
            } else if *r == TMP_DIR {
                dir_next = true;
            }
        }
        chunk.copy_within(whole..end, 0);
        have = end - whole;
        if got == 0 {
            return Ok(());
        }
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
    /// Directories added.
    pub fn len(&self) -> u32 {
        self.n
    }

    pub fn is_empty(&self) -> bool {
        self.n == 0
    }

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

    /// Temporary records for `n` files in one directory (9, 10), and their
    /// fanout.
    fn tmp(n: u32) -> (Vec<u8>, [u32; FANOUT]) {
        let mut t = TMP_DIR.to_vec();
        t.extend_from_slice(&tmp_record(9, 10));
        let mut fanout = [0u32; FANOUT];
        for i in 0..n {
            let h = hash(format!("/gifs/A/{i}.gif").as_bytes());
            t.extend_from_slice(&tmp_record(h, i));
            fanout[(h >> 24) as usize + 1] += 1;
        }
        for b in 0..256 {
            fanout[b + 1] += fanout[b];
        }
        (t, fanout)
    }

    /// `read` over a byte slice, `step` bytes at most a call (not a
    /// multiple of 8: records cut across reads).
    fn reader(t: &[u8], step: usize) -> impl FnMut(&mut [u8]) -> io::Result<usize> + '_ {
        let mut at = 0;
        move |buf: &mut [u8]| {
            let n = buf.len().min(step).min(t.len() - at);
            buf[..n].copy_from_slice(&t[at..at + n]);
            at += n;
            Ok(n)
        }
    }

    #[test]
    fn passes_over_the_temporary_records_gather_every_bucket_in_order() {
        let (t, fanout) = tmp(3000);
        let mut group = vec![0u8; 512 * 8];
        let mut chunk = vec![0u8; 1024];
        let mut recs = Vec::new();
        let (mut b0, mut passes) = (0, 0);
        while b0 < 256 {
            let (b1, n) = gather(&mut reader(&t, 333), &fanout, b0, &mut group, &mut chunk).unwrap();
            assert!(b1 > b0);
            recs.extend_from_slice(&group[..n]);
            b0 = b1;
            passes += 1;
        }
        assert!(passes >= 6, "{passes}");
        assert_eq!(recs.len(), 3000 * 8);
        for i in 0..3000u32 {
            let k = hash(format!("/gifs/A/{i}.gif").as_bytes());
            let b = (k >> 24) as usize;
            let found = (fanout[b]..fanout[b + 1]).any(|r| recs[r as usize * 8..][..8] == tmp_record(k, i));
            assert!(found, "{i}");
        }
        let mut dirs = Vec::new();
        tmp_dirs(&mut reader(&t, 100), &mut chunk, &mut |k, c| {
            dirs.push((k, c));
            Ok(())
        })
        .unwrap();
        assert_eq!(dirs, [(9, 10)]);
    }

    #[test]
    fn records_that_do_not_match_the_count_are_an_error() {
        let (t, mut fanout) = tmp(300);
        for v in fanout[1..].iter_mut() {
            *v += 1; // one more in bucket 0 than the records hold
        }
        let mut group = vec![0u8; 8 * 1024];
        let mut chunk = vec![0u8; 512];
        assert!(gather(&mut reader(&t, 512), &fanout, 0, &mut group, &mut chunk).is_err());
    }

    #[test]
    fn no_location_word_is_the_temporary_directory_mark() {
        assert_eq!(loc((1 << 28) - 1, 15), None);
        assert_eq!(loc((1 << 28) - 2, 15), Some(((1 << 28) - 2) << 4 | 15));
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
        let mut t = Vec::new();
        let mut fanout = [0u32; FANOUT];
        for i in 0..10u32 {
            t.extend_from_slice(&tmp_record(0x0700_0000 | i, i)); // one bucket, 10 records
        }
        for v in fanout[8..].iter_mut() {
            *v = 10;
        }
        let mut group = vec![0u8; 8 * 4];
        let mut chunk = vec![0u8; 64];
        // Buckets 0-6 are empty and fit; bucket 7 does not.
        assert_eq!(gather(&mut reader(&t, 64), &fanout, 0, &mut group, &mut chunk).unwrap(), (7, 0));
        assert!(gather(&mut reader(&t, 64), &fanout, 7, &mut group, &mut chunk).is_err());
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

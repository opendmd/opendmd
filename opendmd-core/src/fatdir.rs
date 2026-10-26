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
/// Most extra ("previous sector") records an index carries.
pub const MAX_EXTRAS: usize = 256;

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
/// before its own when the long name starts in the previous cluster (the
/// index cannot compute that one: an extra record).
pub struct Found<'a> {
    pub name: &'a [u8],
    pub loc: u32,
    pub prev_rel: Option<u32>,
}

/// Walk a directory from its first cluster: every file with an ASCII long
/// name, in directory order. `read(lba, buf)` fills `buf` (a whole
/// number of sectors); `scratch` is a multiple of [`SS`] (the batch size).
/// Returns the sectors read.
pub fn walk(
    geo: &Geometry,
    first: u32,
    read: &mut dyn FnMut(u32, &mut [u8]) -> io::Result<()>,
    scratch: &mut [u8],
    emit: &mut dyn FnMut(Found) -> io::Result<()>,
) -> io::Result<u32> {
    let batch = (scratch.len() / SS).max(1) as u32;
    let mut lfn = [0u8; 260];
    let (mut want, mut chk, mut nent, mut ascii) = (0u8, 0u8, 0usize, true);
    let mut sectors = 0;
    let mut prev_rel = u32::MAX;
    let mut fat = [0u8; SS];
    let mut cluster = Some(first);
    while let Some(c) = cluster {
        let first_rel = geo.rel(c);
        let mut s = 0;
        while s < geo.csize {
            let n = batch.min(geo.csize - s);
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
                    if !is_file(e) {
                        continue;
                    }
                    if !long || !ascii {
                        continue;
                    }
                    let end = lfn[..lfn_n * 13].iter().position(|&b| b == 0).unwrap_or(lfn_n * 13);
                    let name = &lfn[..end];
                    let Some(l) = loc(rel, slot) else { continue };
                    // The long name reaches back past this sector: fine
                    // inside a cluster (the sector before is rel - 1), an
                    // extra record across a cluster boundary, and not
                    // indexed past one sector back.
                    let back = lfn_n.saturating_sub(slot);
                    if back > 16 {
                        continue;
                    }
                    let prev = (back > 0 && rel.is_multiple_of(geo.csize)).then_some(prev_rel);
                    emit(Found { name, loc: l, prev_rel: prev })?;
                }
                prev_rel = rel;
            }
            s += n;
        }
        cluster = next_cluster(geo, c, read, &mut fat)?;
    }
    Ok(sectors)
}

/// The index header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    pub count: u32,
    pub extras: u32,
    pub n_fatent: u32,
    pub database: u32,
    /// Directories indexed: `(hash of the card path, first cluster)`
    /// pairs after the extras.
    pub dirs: u32,
}

impl Header {
    pub fn encode(&self) -> [u8; HEADER] {
        let mut h = [0u8; HEADER];
        h[..4].copy_from_slice(MAGIC);
        h[4..6].copy_from_slice(&VERSION.to_le_bytes());
        for (i, v) in [self.count, self.extras, self.n_fatent, self.database, self.dirs].iter().enumerate() {
            h[8 + i * 4..12 + i * 4].copy_from_slice(&v.to_le_bytes());
        }
        h
    }

    pub fn decode(h: &[u8]) -> Option<Header> {
        if h.len() < HEADER || &h[..4] != MAGIC || h[4..6] != VERSION.to_le_bytes() {
            return None;
        }
        let w = |i: usize| u32::from_le_bytes(h[8 + i * 4..12 + i * 4].try_into().unwrap());
        Some(Header { count: w(0), extras: w(1), n_fatent: w(2), database: w(3), dirs: w(4) })
    }
}

/// Walks every indexed directory, calling `emit(hash, loc, prev_rel)` per file.
pub type WalkAll<'a> = dyn FnMut(&mut dyn FnMut(u32, u32, Option<u32>)) -> io::Result<()> + 'a;

/// Write the index: `walk_all(emit)` walks every indexed directory, calling
/// `emit(hash, loc, prev_rel)` per file. The first walk counts; then each
/// walk gathers the buckets that fit `group` (8 bytes a record) and writes
/// them, so no temporary file and no sort. Returns (records, walks).
pub fn build_index(
    out: &mut dyn FnMut(&[u8]) -> io::Result<()>,
    geo: &Geometry,
    walk_all: &mut WalkAll,
    dirs: &[(u32, u32)],
    group: &mut [u8],
) -> io::Result<(u32, u32)> {
    let mut fanout = [0u32; FANOUT];
    let mut extras: Vec<(u32, u32)> = Vec::new();
    walk_all(&mut |h, l, p| {
        fanout[(h >> 24) as usize + 1] += 1;
        if let Some(p) = p.filter(|_| extras.len() < MAX_EXTRAS) {
            extras.push((l, p));
        }
    })?;
    for b in 0..256 {
        fanout[b + 1] += fanout[b];
    }
    let h = Header {
        count: fanout[256],
        extras: extras.len() as u32,
        n_fatent: geo.n_fatent,
        database: geo.database,
        dirs: dirs.len() as u32,
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
            let mut fill = [0u32; 256];
            walks += 1;
            walk_all(&mut |h, l, _| {
                let b = (h >> 24) as usize;
                if (b0..b1).contains(&b) && fill[b] < fanout[b + 1] - fanout[b] {
                    let at = (fanout[b] - fanout[b0] + fill[b]) as usize * 8;
                    group[at..at + 4].copy_from_slice(&h.to_le_bytes());
                    group[at + 4..at + 8].copy_from_slice(&l.to_le_bytes());
                    fill[b] += 1;
                }
            })?;
            out(&group[..(fanout[b1] - fanout[b0]) as usize * 8])?;
        }
        b0 = b1;
    }
    for &(a, b) in extras.iter().chain(dirs) {
        out(&a.to_le_bytes())?;
        out(&b.to_le_bytes())?;
    }
    Ok((h.count, walks))
}

/// Are the directories indexed (`(hash of the card path, first cluster)`)
/// the ones on the card now? A directory renamed, replaced or added on a PC
/// keeps its entries where they were, so [`verify`] alone would accept a
/// record that now names a file in another directory.
pub fn dirs_match(indexed: &[(u32, u32)], now: &[(u32, u32)]) -> bool {
    indexed.len() == now.len() && now.iter().all(|d| indexed.contains(d))
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn builds_an_index_in_bucket_order_over_several_walks() {
        let key = |i: u32| hash(format!("/gifs/A/{i}.gif").as_bytes());
        let geo = Geometry { csize: 64, fatbase: 32, database: 1000, n_fatent: 9999 };
        let mut out = Vec::new();
        let mut group = vec![0u8; 512 * 8];
        let mut walk = |emit: &mut dyn FnMut(u32, u32, Option<u32>)| -> io::Result<()> {
            for i in 0..3000u32 {
                emit(key(i), i, (i == 7).then_some(6));
            }
            Ok(())
        };
        let mut put = |b: &[u8]| -> io::Result<()> {
            out.extend_from_slice(b);
            Ok(())
        };
        let (count, walks) = build_index(&mut put, &geo, &mut walk, &[(9, 10)], &mut group).unwrap();
        assert_eq!(count, 3000);
        assert!(walks >= 7, "{walks}");
        let h = Header::decode(&out).unwrap();
        assert_eq!((h.count, h.extras, h.database, h.dirs), (3000, 1, 1000, 1));
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
        assert_eq!(out.len(), RECORDS_AT as usize + 3000 * 8 + 16);
        assert_eq!(out[out.len() - 16..out.len() - 8], [7, 0, 0, 0, 6, 0, 0, 0]);
        assert_eq!(out[out.len() - 8..], [9, 0, 0, 0, 10, 0, 0, 0]);
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
        let geo = Geometry { csize: 1, fatbase: 1, database: 2, n_fatent: 100 };
        let mut walk = |emit: &mut dyn FnMut(u32, u32, Option<u32>)| -> io::Result<()> {
            for i in 0..10u32 {
                emit(0x0700_0000 | i, i, None); // one bucket, 10 records
            }
            Ok(())
        };
        let mut put = |_: &[u8]| -> io::Result<()> { Ok(()) };
        let mut group = vec![0u8; 8 * 4];
        assert!(build_index(&mut put, &geo, &mut walk, &[], &mut group).is_err());
    }

    #[test]
    fn another_version_or_magic_is_refused() {
        let h = Header { count: 1, extras: 0, n_fatent: 5, database: 6, dirs: 2 };
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

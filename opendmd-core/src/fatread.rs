//! A FAT32 file read by sector, without FatFs: the player's reads that
//! never wait on FatFs's volume lock (`firmware/src/fatidx.rs` "raw
//! reads", display-isolation 2026-11-02).
//!
//! A FatFs call that walks a big directory (creating or deleting a file in
//! `gifs/Consoles`, a rename, a `stat` of a name that is not there) holds
//! the volume lock for the whole walk: ~330 ms at 20 MHz. The player's next
//! read waited it out, a frame ~0.3 s late. A file the player has open is
//! read-only and nothing changes its cluster chain while it is open (every
//! writer has the player let go of it first, `card::Open`), so its chain is
//! read once at open, into [`Extents`], and its data sectors are then read
//! through the card's own lock only, which a directory walk takes one
//! sector at a time.
//!
//! Everything here works on 512-byte sectors through a caller's reader
//! ([`Sectors`]), which owns locking (on the device, the card's I/O mutex)
//! and errors. FAT32 only (the card the player reads is), 512-byte sectors
//! only (`CONFIG_WL_SECTOR_SIZE_512`).

use crate::fatdir::{Geometry, SS};
use std::io;

/// The FAT32 value at or past which an entry ends its chain.
const EOC: u32 = 0x0FFF_FFF8;
/// The FAT32 value of a bad cluster.
const BAD: u32 = 0x0FFF_FFF7;

/// Read sectors: `count` of them from `lba` into `buf` (exactly
/// `count * 512` bytes).
pub trait Sectors {
    fn read(&mut self, lba: u32, count: u32, buf: &mut [u8]) -> io::Result<()>;
}

impl<F: FnMut(u32, u32, &mut [u8]) -> io::Result<()>> Sectors for F {
    fn read(&mut self, lba: u32, count: u32, buf: &mut [u8]) -> io::Result<()> {
        self(lba, count, buf)
    }
}

fn bad(why: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, why)
}

/// A file's clusters as runs of consecutive ones: `(first cluster, count)`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Extents {
    runs: Vec<(u32, u32)>,
    /// Sectors per cluster and the data region's first sector.
    csize: u32,
    database: u32,
    /// The file's size, bytes.
    size: u32,
}

impl Extents {
    /// The runs, for tests and logs.
    pub fn runs(&self) -> &[(u32, u32)] {
        &self.runs
    }

    pub fn size(&self) -> u32 {
        self.size
    }

    /// The sector holding byte `off` of the file and how many sectors
    /// follow it contiguously on the card (itself included), within the
    /// file's clusters; `None` at or past the end of the file.
    pub fn map(&self, off: u64) -> Option<(u32, u32)> {
        if off >= self.size as u64 {
            return None;
        }
        let cs = self.csize as u64;
        let mut sector = off / SS as u64;
        for &(first, n) in &self.runs {
            let len = n as u64 * cs;
            if sector < len {
                let lba = self.database as u64 + (first as u64 - 2) * cs + sector;
                // Contiguous to the run's end, and no further than the
                // file's last sector.
                let last = (self.size as u64 - 1) / SS as u64;
                let left = (len - sector).min(last - off / SS as u64 + 1);
                return Some((lba as u32, left as u32));
            }
            sector -= len;
        }
        None
    }
}

/// Read the chain of a file of `size` bytes starting at `first` into
/// [`Extents`]: exactly the clusters its size needs, each checked to be a
/// data cluster (not free, not bad, not past the volume), and a loop or a
/// chain that ends early refused. `read_fat` reads one FAT sector (the
/// device: under the volume lock, through FatFs's window, which may hold
/// a newer copy than the card).
pub fn chain(
    geo: &Geometry,
    first: u32,
    size: u32,
    read_fat: &mut dyn FnMut(u32, &mut [u8]) -> io::Result<()>,
) -> io::Result<Extents> {
    let cluster_bytes = geo.csize as u64 * SS as u64;
    if geo.csize == 0 || cluster_bytes == 0 {
        return Err(bad("no cluster size"));
    }
    let need = (size as u64).div_ceil(cluster_bytes);
    let mut ext = Extents { runs: Vec::new(), csize: geo.csize, database: geo.database, size };
    if need == 0 {
        return Ok(ext);
    }
    if need > geo.n_fatent as u64 {
        return Err(bad("file larger than the volume"));
    }
    let valid = |c: u32| c >= 2 && c < geo.n_fatent;
    let mut fat = [0u8; SS];
    let mut fat_lba = u32::MAX;
    let mut cl = first;
    for k in 0..need {
        if !valid(cl) {
            return Err(bad("chain leaves the volume"));
        }
        match ext.runs.last_mut() {
            Some((f, n)) if *f + *n == cl => *n += 1,
            _ => ext.runs.push((cl, 1)),
        }
        if k + 1 == need {
            break;
        }
        let lba = geo.fatbase + cl / 128;
        if lba != fat_lba {
            read_fat(lba, &mut fat)?;
            fat_lba = lba;
        }
        let o = (cl as usize % 128) * 4;
        let next = u32::from_le_bytes([fat[o], fat[o + 1], fat[o + 2], fat[o + 3]]) & 0x0FFF_FFFF;
        if next >= EOC {
            return Err(bad("chain shorter than the file"));
        }
        if next == BAD || next < 2 {
            return Err(bad("chain reaches a free or bad cluster"));
        }
        cl = next;
    }
    // A loop would revisit a cluster: the runs cover `need` clusters, so
    // any overlap between them is one.
    let mut sorted = ext.runs.clone();
    sorted.sort_unstable();
    if sorted.windows(2).any(|w| w[0].0 + w[0].1 > w[1].0) {
        return Err(bad("chain loops"));
    }
    Ok(ext)
}

/// A file read through its [`Extents`]: whole sectors straight into the
/// caller's buffer, partial ones through one sector held here.
pub struct RawFile {
    ext: Extents,
    pos: u64,
    held: [u8; SS],
    held_lba: Option<u32>,
}

impl RawFile {
    pub fn new(ext: Extents) -> RawFile {
        RawFile { ext, pos: 0, held: [0; SS], held_lba: None }
    }

    pub fn pos(&self) -> u64 {
        self.pos
    }

    pub fn size(&self) -> u64 {
        self.ext.size as u64
    }

    /// Move to `pos` (past the end is allowed: reads there give 0).
    /// No card access.
    pub fn seek(&mut self, pos: u64) {
        self.pos = pos;
    }

    /// Read up to `buf.len()` bytes at the position. On an error the
    /// position is where it was before the call (nothing of this call
    /// counted), so a caller can read the same bytes another way.
    pub fn read(&mut self, buf: &mut [u8], dev: &mut dyn Sectors) -> io::Result<usize> {
        let start = self.pos;
        let mut done = 0usize;
        let r = self.read_inner(buf, dev, &mut done);
        match r {
            Ok(()) => {
                self.pos = start + done as u64;
                Ok(done)
            }
            Err(e) => {
                self.pos = start;
                self.held_lba = None;
                Err(e)
            }
        }
    }

    fn read_inner(&mut self, buf: &mut [u8], dev: &mut dyn Sectors, done: &mut usize) -> io::Result<()> {
        let size = self.ext.size as u64;
        while *done < buf.len() {
            let at = self.pos + *done as u64;
            if at >= size {
                break;
            }
            let Some((lba, run)) = self.ext.map(at) else {
                break;
            };
            let within = (at % SS as u64) as usize;
            let want = (buf.len() - *done).min((size - at) as usize);
            if within == 0 && want >= SS {
                // Whole sectors, as many as are contiguous and wanted.
                let n = (want / SS).min(run as usize) as u32;
                let bytes = n as usize * SS;
                dev.read(lba, n, &mut buf[*done..*done + bytes])?;
                *done += bytes;
                continue;
            }
            if self.held_lba != Some(lba) {
                self.held_lba = None;
                dev.read(lba, 1, &mut self.held)?;
                self.held_lba = Some(lba);
            }
            let n = (SS - within).min(want);
            buf[*done..*done + n].copy_from_slice(&self.held[within..within + n]);
            *done += n;
        }
        Ok(())
    }
}

/// The files open for raw reads, by card path: what a writer checks before
/// it changes, replaces, renames or deletes a file (`OpenSet::holds`).
/// FAT keeps no table of open files (`FF_FS_LOCK 0`), and a raw read of a
/// file whose clusters were freed would read whatever was written there
/// next, so a writer must never touch a file in here: it asks the player to
/// let go and checks again, and refuses if the file is still held.
pub struct OpenSet<const N: usize> {
    paths: [Option<String>; N],
}

impl<const N: usize> Default for OpenSet<N> {
    fn default() -> Self {
        OpenSet { paths: std::array::from_fn(|_| None) }
    }
}

/// Two card paths name the same file: FAT compares names without regard
/// to ASCII case.
pub fn same_path(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

impl<const N: usize> OpenSet<N> {
    /// Note `path` open; its slot, or `None` when all are taken (the open
    /// then reads through FatFs as before).
    pub fn insert(&mut self, path: &str) -> Option<usize> {
        let k = self.paths.iter().position(Option::is_none)?;
        self.paths[k] = Some(path.to_string());
        Some(k)
    }

    /// Note `path` open in slot `k` (the caller's own slot numbering).
    pub fn put(&mut self, k: usize, path: &str) {
        if let Some(p) = self.paths.get_mut(k) {
            *p = Some(path.to_string());
        }
    }

    /// Whether a file under the directory `dir` is open here.
    pub fn holds_under(&self, dir: &str) -> bool {
        let dir = dir.trim_end_matches('/');
        self.paths.iter().flatten().any(|p| {
            p.len() > dir.len() + 1 && p.as_bytes()[dir.len()] == b'/' && same_path(&p[..dir.len()], dir)
        })
    }

    /// The file in slot `k` was closed.
    pub fn remove(&mut self, k: usize) {
        if let Some(p) = self.paths.get_mut(k) {
            *p = None;
        }
    }

    /// Whether `path` is open here.
    pub fn holds(&self, path: &str) -> bool {
        self.paths.iter().flatten().any(|p| same_path(p, path))
    }

    pub fn len(&self) -> usize {
        self.paths.iter().flatten().count()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn geo(csize: u32) -> Geometry {
        Geometry { csize, fatbase: 32, database: 1000, n_fatent: 1000 }
    }

    /// A FAT (sectors from `fatbase`) with these chains written in.
    fn fat_with(chains: &[&[u32]]) -> std::collections::HashMap<u32, [u8; SS]> {
        let mut m = std::collections::HashMap::new();
        for ch in chains {
            for (k, &c) in ch.iter().enumerate() {
                let v = ch.get(k + 1).copied().unwrap_or(0x0FFF_FFFF);
                let s: &mut [u8; SS] = m.entry(32 + c / 128).or_insert([0; SS]);
                s[(c as usize % 128) * 4..][..4].copy_from_slice(&v.to_le_bytes());
            }
        }
        m
    }

    fn read_from(m: &std::collections::HashMap<u32, [u8; SS]>) -> impl FnMut(u32, &mut [u8]) -> io::Result<()> + '_ {
        move |lba, b| {
            b[..SS].copy_from_slice(m.get(&lba).unwrap_or(&[0; SS]));
            Ok(())
        }
    }

    #[test]
    fn runs_merge_and_break_where_the_chain_jumps() {
        let m = fat_with(&[&[5, 6, 7, 20, 21, 9]]);
        let e = chain(&geo(1), 5, 6 * 512, &mut read_from(&m)).unwrap();
        assert_eq!(e.runs(), &[(5, 3), (20, 2), (9, 1)]);
        // Byte 0 is cluster 5's sector; 3 contiguous.
        assert_eq!(e.map(0), Some((1000 + 3, 3)));
        assert_eq!(e.map(3 * 512), Some((1000 + 18, 2)));
        assert_eq!(e.map(6 * 512), None);
    }

    #[test]
    fn only_the_clusters_the_size_needs_are_walked() {
        // The chain goes on (a file truncated in place leaves it so on some
        // writers): the size decides.
        let m = fat_with(&[&[5, 6, 7, 8]]);
        let e = chain(&geo(4), 5, 4 * 512 + 1, &mut read_from(&m)).unwrap();
        assert_eq!(e.runs(), &[(5, 2)]);
        // The last mapped sector is the file's last.
        assert_eq!(e.map(4 * 512), Some((1000 + 4 * 4, 1)));
    }

    #[test]
    fn a_short_chain_a_loop_and_free_clusters_are_refused() {
        let m = fat_with(&[&[5, 6]]);
        assert!(chain(&geo(1), 5, 3 * 512, &mut read_from(&m)).is_err(), "ends early");
        let mut l = fat_with(&[&[5, 6, 7]]);
        // 7 -> 5: a loop.
        l.get_mut(&32).unwrap()[7 * 4..][..4].copy_from_slice(&5u32.to_le_bytes());
        assert!(chain(&geo(1), 5, 5 * 512, &mut read_from(&l)).is_err(), "loops");
        let f = fat_with(&[]);
        assert!(chain(&geo(1), 5, 2 * 512, &mut read_from(&f)).is_err(), "free");
        assert!(chain(&geo(1), 1, 512, &mut read_from(&f)).is_err(), "cluster 1");
        assert!(chain(&geo(1), 5000, 512, &mut read_from(&f)).is_err(), "past the volume");
    }

    #[test]
    fn a_writer_sees_every_file_held_until_it_is_let_go() {
        let mut o = OpenSet::<2>::default();
        let a = o.insert("/sdcard/gifs/Consoles/NES_Mario_01.gif").unwrap();
        let b = o.insert("/sdcard/cache/Consoles/3f/NES_Mario_01.gif").unwrap();
        assert!(o.insert("/sdcard/x.gif").is_none(), "full: the third reads through FatFs");
        // FAT's names are case-blind, so is the check.
        assert!(o.holds("/sdcard/GIFS/consoles/nes_mario_01.GIF"));
        assert!(o.holds("/sdcard/cache/Consoles/3f/NES_Mario_01.gif"));
        assert!(!o.holds("/sdcard/gifs/Consoles/NES_Mario_02.gif"));
        o.remove(a);
        assert!(!o.holds("/sdcard/gifs/Consoles/NES_Mario_01.gif"));
        assert!(o.holds("/sdcard/cache/Consoles/3f/NES_Mario_01.gif"));
        assert!(o.holds_under("/sdcard/CACHE/consoles"));
        assert!(o.holds_under("/sdcard/cache/"));
        assert!(!o.holds_under("/sdcard/cach"), "a prefix of a name is not its directory");
        o.remove(b);
        assert!(o.is_empty());
        o.put(1, "/sdcard/gifs/A/x.gif");
        assert!(o.holds("/sdcard/gifs/a/X.GIF"));
    }

    #[test]
    fn an_empty_file_has_no_runs() {
        let f = fat_with(&[]);
        let e = chain(&geo(8), 0, 0, &mut read_from(&f)).unwrap();
        assert!(e.runs().is_empty());
        assert_eq!(e.map(0), None);
    }
}

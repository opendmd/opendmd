//! A FAT32 file read by sector, without FatFs: the player's reads that
//! never wait on FatFs's volume lock (`firmware/src/fatidx.rs` "raw
//! reads", display-isolation 2026-11-02).
//!
//! A FatFs call that walks a big directory (creating or deleting a file in
//! `gifs/Consoles`, a rename, a `stat` of a name that is not there) holds
//! the volume lock for the whole walk: ~330 ms at 20 MHz. The player's next
//! read waited it out, a frame ~0.3 s late. A file the player has open is
//! read-only and nothing changes its cluster chain while it is open (every
//! writer registers a write intent and has the player let go of it first,
//! `OpenSet`), so its chain is
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

/// The most runs a file read raw may have: past it the file is read by
/// FatFs. A fragmented file costs a FAT sector a jump at open, under the
/// volume lock and on the player's core, and a run of heap; 64 runs is a
/// few FAT sectors and 512 B.
pub const MAX_RUNS: usize = 64;

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
        let len = ext.runs.len();
        match ext.runs.last_mut() {
            Some((f, n)) if *f + *n == cl => *n += 1,
            _ if len == MAX_RUNS => return Err(bad("too fragmented to read raw")),
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
///
/// **Write intents** close the gap between that check and the write: a
/// writer registers an intent for the path (or a whole tree) *before* it
/// waits, under the same lock as every open, and keeps it until its write
/// is done. A player's open of anything an intent covers is refused
/// outright ([`OpenSet::insert_unless_covered`]): not raw, and not through
/// FatFs either, whose read of a chain freed under it fails in a way that
/// looks like the card's. So once the writer has seen the path not held,
/// the player cannot open it until the write is done.
pub struct OpenSet<const N: usize> {
    paths: [Option<String>; N],
    intents: Vec<(u32, String, bool)>,
    next_intent: u32,
}

impl<const N: usize> Default for OpenSet<N> {
    fn default() -> Self {
        OpenSet { paths: std::array::from_fn(|_| None), intents: Vec::new(), next_intent: 1 }
    }
}

/// `dir` (a directory's path) contains `path`.
fn under(path: &str, dir: &str) -> bool {
    let dir = dir.trim_end_matches('/');
    path.len() > dir.len() + 1 && path.as_bytes()[dir.len()] == b'/' && same_path(&path[..dir.len()], dir)
}

/// Whether a card path is spelled the one way FatFs and the open set agree
/// on: no empty, `.` or `..` component, none ending in a dot or a space
/// (FatFs drops them), and no 8.3 alias (`NES_MA~1.GIF`, which names a
/// file whose long name is something else). A path that is not is never
/// read raw, and the API refuses it for writes.
pub fn is_canonical(path: &str) -> bool {
    let p = path.trim_start_matches('/');
    if p.is_empty() {
        return true;
    }
    p.split('/').all(|c| {
        !c.is_empty() && c != "." && c != ".." && !c.ends_with('.') && !c.ends_with(' ') && !is_short_alias(c)
    })
}

/// `BASE~N.EXT` as FAT makes an 8.3 alias: a base of at most 8 with `~`
/// and digits at its end, an extension of at most 3.
fn is_short_alias(c: &str) -> bool {
    let (base, ext) = match c.rsplit_once('.') {
        Some((b, e)) => (b, e),
        None => (c, ""),
    };
    let Some(t) = base.rfind('~') else { return false };
    base.len() <= 8 && ext.len() <= 3 && t + 1 < base.len() && base[t + 1..].bytes().all(|b| b.is_ascii_digit())
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

    /// [`put`](Self::put) unless a write intent covers `path`: true if
    /// noted (the player may open it: raw if [`is_canonical`]); false, and
    /// it must not be opened at all — a write is about to change or remove
    /// it, and even FatFs would read a chain freed under it (an error that
    /// is not the card's).
    pub fn insert_unless_covered(&mut self, k: usize, path: &str) -> bool {
        if self.covered(path) {
            return false;
        }
        self.put(k, path);
        true
    }

    /// Register a writer's intent for `path`, or with `tree` for every file
    /// under the directory `path`: its id, for [`end_write`](Self::end_write).
    pub fn begin_write(&mut self, path: &str, tree: bool) -> u32 {
        let id = self.next_intent;
        self.next_intent = self.next_intent.wrapping_add(1).max(1);
        self.intents.push((id, path.to_string(), tree));
        id
    }

    /// The write `id` is done.
    pub fn end_write(&mut self, id: u32) {
        self.intents.retain(|(i, _, _)| *i != id);
    }

    /// Whether a write intent covers `path`.
    pub fn covered(&self, path: &str) -> bool {
        self.intents.iter().any(|(_, p, tree)| same_path(p, path) || (*tree && under(path, p)))
    }

    /// Whether a file under the directory `dir` is open here.
    pub fn holds_under(&self, dir: &str) -> bool {
        self.paths.iter().flatten().any(|p| under(p, dir))
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

/// Why [`Slots::claim`] did not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Claim {
    /// Every slot is taken.
    Full,
    /// A write intent covers the path: not opened.
    Refused,
}

/// The device's open-file slots (`fatidx`'s `/fidx/<k>`): each a file
/// handle and, read raw, its raw state. The order matters: a slot is
/// claimed by publishing its file, then its raw state; it is released by
/// taking its raw state first (and the caller's open-set entry, in
/// `on_raw_cleared`), the file last — so a slot another task claims never
/// has the old file's raw state (its extents) in it.
pub struct Slots<F, R, const N: usize> {
    files: [std::sync::atomic::AtomicPtr<F>; N],
    raws: [std::sync::atomic::AtomicPtr<R>; N],
}

impl<F, R, const N: usize> Default for Slots<F, R, N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<F, R, const N: usize> Slots<F, R, N> {
    pub const fn new() -> Self {
        use std::sync::atomic::AtomicPtr;
        Slots {
            files: [const { AtomicPtr::new(std::ptr::null_mut()) }; N],
            raws: [const { AtomicPtr::new(std::ptr::null_mut()) }; N],
        }
    }

    /// Put `file` (non-null) in a free slot, `raw` with it. With `note`
    /// (the player's opens), `note(k)` runs once the slot is ours and before
    /// its raw state is published: the caller notes the path in its open set
    /// there, unless a write intent covers it — false, and the slot is given
    /// back ([`Claim::Refused`]). `file` stays the caller's when it is not
    /// claimed.
    pub fn claim(&self, file: *mut F, raw: Option<Box<R>>, note: Option<&mut dyn FnMut(usize) -> bool>) -> Result<usize, Claim> {
        use std::sync::atomic::Ordering::{AcqRel, Acquire, Release};
        for k in 0..N {
            if self.files[k].compare_exchange(std::ptr::null_mut(), file, AcqRel, Acquire).is_ok() {
                if let Some(note) = note {
                    if !note(k) {
                        // Nothing else has seen slot k: free again.
                        self.files[k].store(std::ptr::null_mut(), Release);
                        return Err(Claim::Refused);
                    }
                }
                if let Some(r) = raw {
                    self.raws[k].store(Box::into_raw(r), Release);
                }
                return Ok(k);
            }
        }
        Err(Claim::Full)
    }

    /// The file in slot `k` (null when free).
    pub fn file(&self, k: usize) -> *mut F {
        self.files.get(k).map_or(std::ptr::null_mut(), |p| p.load(std::sync::atomic::Ordering::Acquire))
    }

    /// Slot `k`'s raw state (null when none): the slot's user's alone.
    pub fn raw(&self, k: usize) -> *mut R {
        self.raws.get(k).map_or(std::ptr::null_mut(), |p| p.load(std::sync::atomic::Ordering::Acquire))
    }

    /// Free slot `k`: its raw state taken first, then `on_release(k)` (the
    /// caller's open-set entry goes), the file last. The file pointer, and
    /// the raw state, are the caller's to free. `None` if the slot was free.
    pub fn release(&self, k: usize, on_release: impl FnOnce(usize)) -> Option<(*mut F, Option<Box<R>>)> {
        use std::sync::atomic::Ordering::AcqRel;
        if self.file(k).is_null() {
            return None;
        }
        let r = self.raws[k].swap(std::ptr::null_mut(), AcqRel);
        // SAFETY: only `claim` stores, from `Box::into_raw`; swapped out,
        // nothing else has it.
        let raw = (!r.is_null()).then(|| unsafe { Box::from_raw(r) });
        on_release(k);
        let f = self.files[k].swap(std::ptr::null_mut(), AcqRel);
        (!f.is_null()).then_some((f, raw))
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
    fn a_claim_under_a_write_intent_is_refused_and_frees_its_slot() {
        let slots: Slots<u32, u32, 1> = Slots::new();
        let mut open = OpenSet::<1>::default();
        let w = open.begin_write("/sdcard/cache/a.gif", false);
        let mut f = 7u32;
        let mut note = |k| open.insert_unless_covered(k, "/sdcard/cache/a.gif");
        assert_eq!(slots.claim(&mut f, Some(Box::new(1)), Some(&mut note)), Err(Claim::Refused));
        assert!(slots.file(0).is_null() && slots.raw(0).is_null(), "the refused slot is free again");
        open.end_write(w);
        let mut note = |k| open.insert_unless_covered(k, "/sdcard/cache/a.gif");
        assert_eq!(slots.claim(&mut f, None, Some(&mut note)), Ok(0));
        let mut g = 8u32;
        assert_eq!(slots.claim(&mut g, None, None), Err(Claim::Full));
    }

    #[test]
    fn an_open_under_a_write_intent_is_refused() {
        let mut o = OpenSet::<4>::default();
        let w = o.begin_write("/sdcard/gifs/Consoles/NES_Mario_01.gif", false);
        // The switch's open of that very file (any spelling of its case):
        // refused (the engine skips it, a copy falls back to its original).
        assert!(!o.insert_unless_covered(0, "/sdcard/GIFS/Consoles/nes_mario_01.gif"));
        assert!(!o.holds("/sdcard/gifs/Consoles/NES_Mario_01.gif"), "a refused open is not held");
        // Another file opens raw meanwhile.
        assert!(o.insert_unless_covered(1, "/sdcard/gifs/Consoles/NES_Mario_02.gif"));
        o.end_write(w);
        assert!(o.insert_unless_covered(0, "/sdcard/gifs/Consoles/NES_Mario_01.gif"));
    }

    #[test]
    fn a_tree_intent_covers_every_file_under_it_and_nothing_beside() {
        let mut o = OpenSet::<4>::default();
        let w = o.begin_write("/sdcard/cache", true);
        assert!(o.covered("/sdcard/cache/Consoles/3f/NES_Mario_01.gif"));
        assert!(o.covered("/sdcard/CACHE/x.gif"));
        assert!(!o.covered("/sdcard/cached.gif"), "a name that starts like the directory");
        assert!(!o.covered("/sdcard/gifs/Consoles/NES_Mario_01.gif"));
        assert!(!o.insert_unless_covered(0, "/sdcard/cache/Arcade/00/MAME_Pac_01.gif"));
        // Two writers: one ending leaves the other's intent.
        let w2 = o.begin_write("/sdcard/cache/Arcade/00/MAME_Pac_01.gif", false);
        o.end_write(w);
        assert!(o.covered("/sdcard/cache/Arcade/00/MAME_Pac_01.gif"));
        assert!(!o.covered("/sdcard/cache/Consoles/3f/NES_Mario_01.gif"));
        o.end_write(w2);
        assert!(!o.covered("/sdcard/cache/Arcade/00/MAME_Pac_01.gif"));
    }

    #[test]
    fn only_canonical_spellings_are_read_raw() {
        for good in ["/sdcard/gifs/Consoles/NES_Mario_01.gif", "/gifs/a/b.gif", "/", "/sdcard/gifs/Consoles/Super~Mario_01.gif"] {
            assert!(is_canonical(good), "{good}");
        }
        for bad in [
            "/sdcard/gifs//Consoles/x.gif",
            "/sdcard/gifs/Consoles/x.gif.",
            "/sdcard/gifs/Consoles/x.gif ",
            "/sdcard/gifs/./Consoles/x.gif",
            "/sdcard/gifs/../x.gif",
            "/sdcard/gifs/Consoles/NES_MA~1.GIF",
            "/sdcard/gifs/CONSOL~2/x.gif",
        ] {
            assert!(!is_canonical(bad), "{bad}");
        }
        // An alias is noted (held, so a writer waits for it) but is never
        // read raw: the caller asks `is_canonical`.
        let mut o = OpenSet::<2>::default();
        assert!(o.insert_unless_covered(0, "/sdcard/gifs/Consoles/NES_MA~1.GIF"));
        assert!(o.holds("/sdcard/gifs/Consoles/nes_ma~1.gif"));
    }

    #[test]
    fn a_file_in_more_than_max_runs_pieces_is_left_to_fatfs() {
        // Every other cluster: as many runs as clusters.
        let ch: Vec<u32> = (0..(MAX_RUNS as u32 + 1)).map(|k| 10 + 2 * k).collect();
        let m = fat_with(&[&ch]);
        assert!(chain(&geo(1), 10, (MAX_RUNS as u32 + 1) * 512, &mut read_from(&m)).is_err());
        let ok: Vec<u32> = ch[..MAX_RUNS].to_vec();
        let m = fat_with(&[&ok]);
        assert_eq!(chain(&geo(1), 10, MAX_RUNS as u32 * 512, &mut read_from(&m)).unwrap().runs().len(), MAX_RUNS);
    }

    /// Many tasks opening and closing at once: a slot's raw state is
    /// always its own file's (closing takes the raw state before it frees
    /// the slot), and the open set never holds a path no slot has.
    #[test]
    fn a_slot_never_shows_another_files_raw_state() {
        use std::sync::{Arc, Mutex};
        let slots: Arc<Slots<u64, u64, 3>> = Arc::new(Slots::new());
        let open: Arc<Mutex<OpenSet<3>>> = Arc::new(Mutex::new(OpenSet::default()));
        let threads: Vec<_> = (0..6u64)
            .map(|t| {
                let (slots, open) = (slots.clone(), open.clone());
                std::thread::spawn(move || {
                    for i in 0..5000u64 {
                        let id = t * 1_000_000 + i + 1;
                        let file = Box::into_raw(Box::new(id));
                        let path = format!("/f/{id}");
                        let mut note = |k| open.lock().unwrap().insert_unless_covered(k, &path);
                        let Ok(k) = slots.claim(file, Some(Box::new(id)), Some(&mut note)) else {
                            // SAFETY: never published.
                            drop(unsafe { Box::from_raw(file) });
                            continue;
                        };
                        // While ours: our file, our raw state, our path.
                        let r = slots.raw(k);
                        assert!(!r.is_null());
                        // SAFETY: slot k is ours until released.
                        assert_eq!(unsafe { *r }, id, "slot {k} has another file's raw state");
                        assert_eq!(unsafe { *slots.file(k) }, id);
                        assert!(open.lock().unwrap().holds(&path));
                        let (f, raw) = slots.release(k, |k| open.lock().unwrap().remove(k)).unwrap();
                        assert_eq!(raw.map(|b| *b), Some(id));
                        // SAFETY: released: ours to free.
                        assert_eq!(unsafe { *Box::from_raw(f) }, id);
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        assert!(open.lock().unwrap().is_empty());
        for k in 0..3 {
            assert!(slots.file(k).is_null() && slots.raw(k).is_null());
        }
    }

    #[test]
    fn an_empty_file_has_no_runs() {
        let f = fat_with(&[]);
        let e = chain(&geo(8), 0, 0, &mut read_from(&f)).unwrap();
        assert!(e.runs().is_empty());
        assert_eq!(e.map(0), None);
    }
}

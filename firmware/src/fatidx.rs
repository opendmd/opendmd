//! The FAT index: GIFs opened without a directory scan (CLAUDE.md "The FAT
//! index"; `opendmd_core::fatdir` has the on-card format and the entry
//! checks, `docs/research/fat-index.md` the measurements).
//!
//! `/cache/.fatidx` records where each `/gifs/<dir>/<name>` file's 8.3
//! directory entry is. [`open`] reads that one sector under FatFs's volume
//! mutex (from FatFs's window when it holds it: it may be newer than the
//! card), checks the entry spells the expected name, and fills a read-only
//! FatFs `FIL` from it the way `f_open` does. With `CONFIG_FATFS_FS_LOCK=0`
//! FatFs keeps no open-file table, so that `FIL` is as good as its own. The
//! `FIL` is handed out as an ordinary `std::fs::File` through a tiny VFS at
//! `/fidx` (read, lseek, fstat, close on FatFs's own `f_read`/`f_lseek`/
//! `f_close`), so the player and the server keep their one file type.
//! Anything that does not match falls back to the path open.
//!
//! **Raw reads** (the player's files only, [`open_player`]): the file's
//! cluster chain is read at open, under the volume lock, into
//! `opendmd_core::fatread::Extents`, and its data sectors are then read
//! through `ff_disk_read` alone (the card's I/O mutex, `cardspeed`), never
//! FatFs's volume mutex: a FatFs call walking a big directory on CPU0
//! (a create, delete or rename in `gifs/Consoles`, ~330 ms under that
//! lock) interleaves with the player sector by sector instead of making a
//! frame late. A raw-read file is in [`holds`] while open. Every writer
//! first registers a write intent for what it will touch ([`intend`],
//! [`intend_tree`]), under the same lock as every open, so nothing it
//! covers opens raw from then on (FatFs reads it); then has the player let
//! go and waits until the file is not held; then writes, then drops the
//! intent (`http::release_for_write`). A raw-read file's chain therefore
//! cannot change under its reads. Files of more than
//! `fatread::MAX_RUNS` runs, and paths not spelled canonically (an 8.3
//! alias, a trailing dot), are read by FatFs. A failed raw read falls back to FatFs's `f_read` from the
//! same position for the rest of the file's life.
//!
//! The index is rebuilt on the network worker (CPU0) when it is missing,
//! after the API changed something under `/gifs` (30 s quiet first), or when
//! opens keep missing it (a card edited on a PC): never with a transfer in
//! flight or the cache builder's task up, never without the heap.

use esp_idf_sys as sys;
use opendmd_core::fatdir::{self, Geometry, Header, Verify, SS};
use opendmd_core::fatread;
use std::fs::{self, File};
use std::io::{self, BufWriter, Read, Seek, SeekFrom, Write};
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU32, Ordering::Relaxed};
use std::sync::Mutex;
use std::time::Duration;

use crate::config;

const INDEX: &str = "/sdcard/cache/.fatidx";
const NEW: &str = "/sdcard/cache/.fatidx.new";
/// The GIFs' names (`<card path>` a line, `/gifs/<dir>/<name>`), written by
/// each build: what naming a new GIF needs without a directory listing
/// (`GET /api/files?dir=…&names=1`, [`names_json`]).
const NAMES: &str = "/sdcard/cache/.names";
const NAMES_NEW: &str = "/sdcard/cache/.names.new";
/// The names file matches the card (built, nothing since but what
/// [`NAMES_DELTA`] holds).
static NAMES_OK: AtomicBool = AtomicBool::new(false);
/// A current index was loaded with no names file beside it: walk the
/// GIFs' directories for one alone ([`build_names`]) once things are quiet.
static NAMES_WANTED: AtomicBool = AtomicBool::new(false);
/// GIFs written (true) or removed (false) through the API since the names
/// file was built, by card path, newest last, with the uptime noted.
static NAMES_DELTA: Mutex<Vec<(String, bool, u32)>> = Mutex::new(Vec::new());
/// Files opened through the index at once (the player's two, the server's,
/// and room).
/// The file on screen, the one going out under a transition, the next one
/// opened ahead, the playlist's two files, and the server's.
const SLOTS: usize = 8;
const SLOT_PATHS: [&str; SLOTS] = ["/fidx/0", "/fidx/1", "/fidx/2", "/fidx/3", "/fidx/4", "/fidx/5", "/fidx/6", "/fidx/7"];


/// A slot's raw reads: its extents and position, the drive, and whether a
/// raw read failed (FatFs's `f_read` from then on).
struct Raw {
    file: fatread::RawFile,
    pdrv: u8,
    broken: bool,
    /// A GIF the player reads (the read-ahead may follow it), not a
    /// playlist's file ([`open_catalog_file`]).
    follow: bool,
}
/// The slots: each a FIL and, read raw, its [`Raw`] (`fatread::Slots`:
/// claimed file first, released raw state first).
static TABLE: fatread::Slots<sys::FIL, Raw, SLOTS> = fatread::Slots::new();
/// Opens that found every slot busy (read by path, through FatFs).
static SLOTS_FULL: AtomicU32 = AtomicU32::new(0);
/// The player's opens refused because a write intent covered the file
/// (the engine skipped it, or played the original for its copy).
static INTENT_REFUSED: AtomicU32 = AtomicU32::new(0);

/// Whether a write intent covers `full`.
pub fn covered(full: &str) -> bool {
    open_set(|o| o.covered(full))
}

/// The player's open of a file a write is about to change: refused, never
/// by path either (`ENOENT`: the engine skips it).
fn refused() -> io::Error {
    INTENT_REFUSED.fetch_add(1, Relaxed);
    errno(sys::ENOENT)
}
/// The raw-read files open, by path (`holds`).
static OPEN: Mutex<Option<fatread::OpenSet<SLOTS>>> = Mutex::new(None);
/// Raw reads done, and those that failed and fell back (`status_json`).
static RAW_READS: AtomicU32 = AtomicU32::new(0);
static RAW_FALLBACKS: AtomicU32 = AtomicU32::new(0);

fn open_set<T>(f: impl FnOnce(&mut fatread::OpenSet<SLOTS>) -> T) -> T {
    let mut g = OPEN.lock().unwrap_or_else(|p| p.into_inner());
    f(g.get_or_insert_with(Default::default))
}

/// Whether the player has `full` (`/sdcard/...`) open for raw reads: a
/// writer must not change, replace, rename or delete it until it is not.
pub fn holds(full: &str) -> bool {
    open_set(|o| o.holds(full))
}

/// Whether the player has any file under the directory `full` open.
pub fn holds_under(full: &str) -> bool {
    open_set(|o| o.holds_under(full))
}

/// A writer's intent (`fatread::OpenSet`'s write intents): while it lives,
/// nothing it covers is opened for raw reads (the player reads it through
/// FatFs). Taken *before* the writer waits for the player to let go, and
/// kept until the write is done.
#[must_use]
pub struct WriteIntent(u32);

impl Drop for WriteIntent {
    fn drop(&mut self) {
        let id = self.0;
        open_set(|o| o.end_write(id));
    }
}

/// An intent for the file `full` (`/sdcard/...`).
pub fn intend(full: &str) -> WriteIntent {
    WriteIntent(open_set(|o| o.begin_write(full, false)))
}

/// An intent for every file under the directory `full`.
pub fn intend_tree(full: &str) -> WriteIntent {
    WriteIntent(open_set(|o| o.begin_write(full, true)))
}
static VFS_UP: AtomicBool = AtomicBool::new(false);
/// A rebuild is wanted (no index, the API changed `/gifs`, too many misses).
static DIRTY: AtomicBool = AtomicBool::new(true);
/// Uptime (ms, wrapping) of the last change through the API, and of the
/// last build's start; 0 = never.
static TOUCHED_MS: AtomicU32 = AtomicU32::new(0);
static BUILT_MS: AtomicU32 = AtomicU32::new(0);
static HITS: AtomicU32 = AtomicU32::new(0);
static MISSES: AtomicU32 = AtomicU32::new(0);
static RECORDS: AtomicU32 = AtomicU32::new(0);
static BUILDS: AtomicU32 = AtomicU32::new(0);
/// Look for the card's index again on the next turn (the handover).
static RELOAD: AtomicBool = AtomicBool::new(false);
/// A build's task is up (one at a time).
static BUILDING: AtomicBool = AtomicBool::new(false);
/// The last build failed: the next waits `FATIDX_MIN_GAP_S` even for a
/// missing index (a failing card must not be walked in a loop).
static FAILED: AtomicBool = AtomicBool::new(false);

struct Loaded {
    file: File,
    fanout: Box<[u32; fatdir::FANOUT]>,
    /// The index file read raw (its extents, the drive): a lookup by the
    /// player never waits on FatFs's volume lock. `None`: through `file`.
    raw: Option<(fatread::RawFile, u8)>,
}

/// Files written, renamed or removed through the API or the cache builder
/// since the index was built: (key, the 8.3 entry's location, or `None`
/// for gone), newest last, and the uptime it was noted at. Looked at before
/// the index file, so a new or renamed file stays an index hit until the
/// next build folds it in (`note_written`, `note_gone`).
static DELTA: Mutex<Vec<(u32, Option<u32>, u32)>> = Mutex::new(Vec::new());
/// Past this many, the index is rebuilt (when quiet, as before).
const DELTA_MAX: usize = 256;

/// `full` (`/sdcard/...`) was just written (or renamed to): its entry's
/// location found by an open of our own (a directory walk under FatFs's
/// lock, here on CPU0: the player's index opens no longer wait on it) and
/// noted for lookups.
pub fn note_written(full: &str) {
    let Some(card) = full.strip_prefix(config::SD_MOUNT).filter(|c| c.starts_with("/gifs/") || c.starts_with("/cache/")) else {
        return;
    };
    let Ok((fs, geo)) = volume() else { return };
    let Ok(path) = std::ffi::CString::new(format!("0:{card}")) else { return };
    // SAFETY: a zeroed FIL f_open fills and f_close leaves; `dir_ptr` points
    // into the volume's window (the entry's slot in its sector) and is read
    // before the FIL is closed.
    let loc = unsafe {
        let mut f: sys::FIL = std::mem::zeroed();
        if sys::f_open(&mut f, path.as_ptr(), sys::FA_READ as u8) != sys::FRESULT_FR_OK {
            return;
        }
        let slot = ((f.dir_ptr as usize).wrapping_sub(std::ptr::addr_of!((*fs).win) as usize) / 32) & 15;
        let rel = f.dir_sect.wrapping_sub(geo.database);
        sys::f_close(&mut f);
        fatdir::loc(rel, slot)
    };
    push_delta(fatdir::hash(card.as_bytes()), loc);
    note_name(card, true);
}

/// `full` was removed (or renamed from): lookups find no record for it.
pub fn note_gone(full: &str) {
    if let Some(card) = full.strip_prefix(config::SD_MOUNT).filter(|c| c.starts_with("/gifs/") || c.starts_with("/cache/")) {
        push_delta(fatdir::hash(card.as_bytes()), None);
        note_name(card, false);
    }
}

/// The most [`NAMES_DELTA`] holds, in bytes of names: past it the names
/// file is not trusted until the next build (names=1 lists instead). Names
/// are up to 255 bytes: a count alone would let it reach tens of KB.
const NAMES_DELTA_BYTES: usize = 4 * 1024;

/// A GIF's name came or went (`/gifs/...` only), for [`names_json`].
fn note_name(card: &str, present: bool) {
    if !card.starts_with("/gifs/") {
        return;
    }
    let mut d = NAMES_DELTA.lock().unwrap_or_else(|p| p.into_inner());
    d.retain(|(p, _, _)| !p.eq_ignore_ascii_case(card));
    let bytes: usize = d.iter().map(|(p, _, _)| p.len()).sum::<usize>() + card.len();
    if d.len() >= DELTA_MAX || bytes > NAMES_DELTA_BYTES {
        // Past this the names file is not trusted until the next build.
        *d = Vec::new();
        NAMES_OK.store(false, Relaxed);
        return;
    }
    d.push((card.to_string(), present, uptime_ms()));
}

/// `GET /api/files?dir=/gifs/<dir>&names=1`: a `/gifs/` folder's names
/// from the names file (that folder's bytes alone,
/// `opendmd_core::names_file`) and what changed since, for streaming as
/// the listing's JSON: no directory walk. `None` when the file cannot be
/// trusted (no build yet, a folder added or removed since): list it
/// instead.
pub struct Names {
    /// The folder's lines in the names file.
    file: std::io::BufReader<io::Take<File>>,
    /// Names written, renamed or removed in it since (present or not).
    delta: Vec<(String, bool)>,
}

impl Names {
    /// Each name, the file's (less what changed since) then the ones
    /// written since; stops at `f`'s first error.
    pub fn each(mut self, f: &mut dyn FnMut(&str) -> io::Result<()>) -> io::Result<()> {
        use std::io::BufRead;
        let mut line = String::new();
        loop {
            line.clear();
            // At most a name's length a line (a damaged file stops here).
            let max = opendmd_core::names_file::NAME_MAX as u64 + 1;
            let got = (&mut self.file).take(max).read_line(&mut line)?;
            if got == 0 {
                break;
            }
            if !line.ends_with('\n') && got as u64 == max {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "names file: a line past a name's length"));
            }
            let n = line.trim_end_matches('\n');
            if n.is_empty() || self.delta.iter().any(|(d, _)| d.eq_ignore_ascii_case(n)) {
                continue;
            }
            f(n)?;
        }
        for (n, present) in &self.delta {
            if *present {
                f(n)?;
            }
        }
        Ok(())
    }
}

pub fn names(card_dir: &str) -> Option<Names> {
    let dir = card_dir.trim_end_matches('/');
    if !NAMES_OK.load(Relaxed) || !dir.starts_with("/gifs/") || dir[6..].contains('/') {
        return None;
    }
    let mut f = File::open(NAMES).ok()?;
    let (_, len) = opendmd_core::names_file::find(&mut f, dir).ok()??;
    let mut delta = Vec::new();
    for (p, present, _) in NAMES_DELTA.lock().unwrap_or_else(|p| p.into_inner()).iter() {
        let Some((d, n)) = p.rsplit_once('/') else { continue };
        if d.eq_ignore_ascii_case(dir) {
            delta.push((n.to_string(), *present));
        }
    }
    Some(Names { file: std::io::BufReader::with_capacity(512, f.take(len as u64)), delta })
}

/// A names file of the current layout is at [`NAMES`].
fn names_present() -> bool {
    File::open(NAMES).is_ok_and(|mut f| opendmd_core::names_file::trailer(&mut f).is_ok_and(|t| t.is_some()))
}

fn push_delta(h: u32, loc: Option<u32>) {
    let mut d = DELTA.lock().unwrap_or_else(|p| p.into_inner());
    // Its whole 3 KB once (not doubled to 6 KB on the way), or none: a
    // change not noted is a stale record, which the entry check catches.
    if d.capacity() == 0 && d.try_reserve_exact(DELTA_MAX + 1).is_err() {
        DELTA_OVERFLOWED.store(true, Relaxed);
        DIRTY.store(true, Relaxed);
        return;
    }
    d.retain(|(k, _, _)| *k != h);
    d.push((h, loc, uptime_ms()));
    if d.len() > DELTA_MAX {
        d.remove(0);
        // Changes lost from it: no "absent" answers until the next build.
        DELTA_OVERFLOWED.store(true, Relaxed);
        // Too much since the build: rebuild when quiet.
        TOUCHED_MS.store(uptime_ms(), Relaxed);
        DIRTY.store(true, Relaxed);
    }
    DELTA_LEN.store(d.len() as u32, Relaxed);
}
static DELTA_LEN: AtomicU32 = AtomicU32::new(0);
/// The index loaded was built this boot (`indexed_now`'s absent answer).
static BUILT_THIS_BOOT: AtomicBool = AtomicBool::new(false);
/// The delta dropped changes since the build in progress or loaded began.
static DELTA_OVERFLOWED: AtomicBool = AtomicBool::new(false);

/// `Some(None)`: looked, no usable index.
static LOADED: Mutex<Option<Option<Loaded>>> = Mutex::new(None);
static FS: AtomicPtr<sys::FATFS> = AtomicPtr::new(std::ptr::null_mut());

fn uptime_ms() -> u32 {
    // SAFETY: a plain IDF call.
    ((unsafe { sys::esp_timer_get_time() } / 1000) as u32).max(1)
}

/// Milliseconds from `then` to `now` (wrapping uptime).
fn since(now: u32, then: u32) -> u32 {
    now.wrapping_sub(then)
}

// The FIL this module fills, and the window it reads, must be FatFs's own
// arrays of 512 B (`CONFIG_WL_SECTOR_SIZE_512`, no dynamic buffers). With
// `CONFIG_FATFS_USE_DYN_BUFFERS` (an option on IDF 6) `buf` and `win` are
// pointers f_open/f_mount allocate: a zeroed FIL would have none. That
// build must fail here, not read through a null buffer.
const _: fn(&sys::FIL) -> &[u8; SS] = |f| &f.buf;
const _: fn(&sys::FATFS) -> &[u8; SS] = |f| &f.win;

fn errno(code: u32) -> io::Error {
    io::Error::from_raw_os_error(code as i32)
}

/// The mounted volume and its layout (the pointer lives as long as the mount).
fn volume() -> io::Result<(*mut sys::FATFS, Geometry)> {
    let mut fs = FS.load(Relaxed);
    if fs.is_null() {
        fs = open_dir("/")?.0;
        FS.store(fs, Relaxed);
    }
    // SAFETY: the mounted volume's fields, fixed while it is mounted.
    unsafe {
        if (*fs).fs_type != 3 {
            return Err(errno(sys::ENOTSUP));
        }
        Ok((
            fs,
            Geometry { csize: (*fs).csize as u32, fatbase: (*fs).fatbase, database: (*fs).database, n_fatent: (*fs).n_fatent },
        ))
    }
}

/// `f_opendir` a card directory: its volume and first cluster.
fn open_dir(card_dir: &str) -> io::Result<(*mut sys::FATFS, u32)> {
    let path = std::ffi::CString::new(format!("0:{card_dir}")).map_err(|_| errno(sys::EINVAL))?;
    // SAFETY: plain data FatFs fills; closing only invalidates it (FF_FS_LOCK 0).
    unsafe {
        let mut d: sys::FF_DIR = std::mem::zeroed();
        if sys::f_opendir(&mut d, path.as_ptr()) != sys::FRESULT_FR_OK {
            return Err(errno(sys::ENOENT));
        }
        // Read before closing: `f_closedir` clears `obj.fs`.
        let out = (d.obj.fs, d.obj.sclust);
        sys::f_closedir(&mut d);
        if out.0.is_null() {
            return Err(errno(sys::EIO));
        }
        Ok(out)
    }
}

/// FatFs's volume mutex, as every FatFs call takes it.
struct VolLock(i32);
impl VolLock {
    fn take(fs: *mut sys::FATFS) -> io::Result<VolLock> {
        // SAFETY: the volume's own mutex index.
        let vol = unsafe { (*fs).ldrv } as i32;
        if unsafe { sys::ff_mutex_take(vol) } == 0 {
            return Err(errno(sys::ETIMEDOUT));
        }
        Ok(VolLock(vol))
    }
}
impl Drop for VolLock {
    fn drop(&mut self) {
        // SAFETY: taken in `take`.
        unsafe { sys::ff_mutex_give(self.0) };
    }
}

/// Sectors from `lba` into `buf` from the card itself, through its I/O lock
/// only (no volume lock, not FatFs's window): the player's index opens.
fn card_read(fs: *mut sys::FATFS, lba: u32, buf: &mut [u8]) -> io::Result<()> {
    let n = buf.len() / SS;
    // SAFETY: the mounted volume's drive; buf is n sectors.
    unsafe {
        if sys::ff_disk_read((*fs).pdrv, buf.as_mut_ptr(), lba, n as u32) != 0 {
            return Err(errno(sys::EIO));
        }
    }
    Ok(())
}

/// Sectors from `lba` into `buf`, the lock held: FatFs's window (perhaps
/// dirty) wins over the card for the sector it holds.
fn read_locked(fs: *mut sys::FATFS, lba: u32, buf: &mut [u8]) -> io::Result<()> {
    let n = buf.len() / SS;
    // SAFETY: lock held by the caller; buf is n sectors.
    unsafe {
        if sys::ff_disk_read((*fs).pdrv, buf.as_mut_ptr(), lba, n as u32) != 0 {
            return Err(errno(sys::EIO));
        }
        let w = (*fs).winsect;
        if (lba..lba + n as u32).contains(&w) {
            let at = (w - lba) as usize * SS;
            std::ptr::copy_nonoverlapping(std::ptr::addr_of!((*fs).win).cast::<u8>(), buf[at..].as_mut_ptr(), SS);
        }
    }
    Ok(())
}

/// The mounted volume's boot sector, read as it is (`card::volume`): its
/// serial and label, for the card's identity (`sd.card_id`). One sector
/// under the volume mutex; nothing is written (a card shared with DMDoS is
/// left as it was). Works for FAT12/16 too, unlike the index.
pub fn volume_id() -> io::Result<Option<opendmd_core::card::Volume>> {
    let (fs, _) = open_dir("/")?;
    let mut b = [0u8; SS];
    {
        let _l = VolLock::take(fs)?;
        // SAFETY: the mounted volume's field, fixed while it is mounted.
        read_locked(fs, unsafe { (*fs).volbase }, &mut b)?;
    }
    Ok(opendmd_core::card::volume(&b))
}

// ---- the VFS: a FIL we filled, as a file descriptor -------------------------

fn fil(fd: i32) -> *mut sys::FIL {
    TABLE.file(fd as usize)
}

unsafe fn fail(code: u32) -> i32 {
    *sys::__errno() = code as i32;
    -1
}

fn fr_code(r: sys::FRESULT) -> u32 {
    match r {
        sys::FRESULT_FR_TIMEOUT => sys::ETIMEDOUT,
        sys::FRESULT_FR_NOT_READY => sys::ENODEV,
        // The file's FAT structure is not what it was (its chain freed by a
        // delete under a reader): never evidence against the card
        // (`sdhealth::is_card_error` counts EIO, ETIMEDOUT, ENODEV).
        sys::FRESULT_FR_INT_ERR | sys::FRESULT_FR_NO_FILE | sys::FRESULT_FR_INVALID_OBJECT => sys::ESTALE,
        _ => sys::EIO,
    }
}

unsafe extern "C" fn v_open(path: *const core::ffi::c_char, _flags: i32, _mode: i32) -> i32 {
    // "/<slot>": the slot `open_at` just filled.
    match std::ffi::CStr::from_ptr(path).to_bytes() {
        &[b'/', d] if d.is_ascii_digit() && !fil((d - b'0') as i32).is_null() => (d - b'0') as i32,
        _ => fail(sys::ENOENT),
    }
}

unsafe extern "C" fn v_read(fd: i32, dst: *mut core::ffi::c_void, size: usize) -> isize {
    let f = fil(fd);
    if f.is_null() {
        return fail(sys::EBADF) as isize;
    }
    let r = TABLE.raw(fd as usize);
    if !r.is_null() && !(*r).broken {
        let raw = &mut *r;
        let pdrv = raw.pdrv;
        let mut dev = |lba: u32, count: u32, b: &mut [u8]| -> io::Result<()> {
            // SAFETY: `b` is `count` sectors; the drive is the mounted card's.
            if sys::ff_disk_read(pdrv, b.as_mut_ptr(), lba, count) != 0 {
                return Err(errno(sys::EIO));
            }
            Ok(())
        };
        let buf = std::slice::from_raw_parts_mut(dst.cast::<u8>(), size);
        // The display core's reads: from the read-ahead when it has them.
        let player = raw.follow && crate::cardspeed::on_display_core();
        if player {
            if let Some(n) = crate::readahead::serve(fd as usize, &mut raw.file, pdrv, buf) {
                return n as isize;
            }
        }
        match raw.file.read(buf, &mut dev) {
            Ok(n) => {
                RAW_READS.fetch_add(1, Relaxed);
                if player {
                    crate::readahead::moved(fd as usize, raw.file.pos());
                }
                return n as isize;
            }
            Err(e) => {
                // A card error, counted as any read's is (`frametime`
                // sees the fallback's result, which may succeed).
                crate::sdhealth::read_failed(crate::sdhealth::Kind::Read, raw.file.pos() > 0, "the playing file (raw)", &e);
                // FatFs from here, from the same position: its chain walk
                // starts again at the file's first cluster (`fptr` 0) and
                // its buffer is reloaded (no data sector is 0).
                raw.broken = true;
                RAW_FALLBACKS.fetch_add(1, Relaxed);
                (*f).fptr = 0;
                (*f).clust = 0;
                (*f).sect = 0;
                let r = sys::f_lseek(f, raw.file.pos() as sys::FSIZE_t);
                if r != sys::FRESULT_FR_OK {
                    return fail(fr_code(r)) as isize;
                }
            }
        }
    }
    let mut br: sys::UINT = 0;
    match sys::f_read(f, dst, size as sys::UINT, &mut br) {
        sys::FRESULT_FR_OK => br as isize,
        r => fail(fr_code(r)) as isize,
    }
}

unsafe extern "C" fn v_lseek(fd: i32, off: sys::off_t, whence: i32) -> sys::off_t {
    let f = fil(fd);
    if f.is_null() {
        return fail(sys::EBADF) as sys::off_t;
    }
    let r = TABLE.raw(fd as usize);
    let raw = (!r.is_null() && !(*r).broken).then(|| &mut *r);
    let base = match whence as u32 {
        sys::SEEK_CUR => raw.as_ref().map_or((*f).fptr as i64, |r| r.file.pos() as i64),
        sys::SEEK_END => (*f).obj.objsize as i64,
        _ => 0,
    };
    let to = base + off as i64;
    if to < 0 {
        return fail(sys::EINVAL) as sys::off_t;
    }
    // Raw: a position only (FatFs's is set if a read ever falls back).
    if let Some(raw) = raw {
        raw.file.seek(to as u64);
        return to as sys::off_t;
    }
    match sys::f_lseek(f, to as sys::FSIZE_t) {
        sys::FRESULT_FR_OK => (*f).fptr as sys::off_t,
        r => fail(fr_code(r)) as sys::off_t,
    }
}

unsafe extern "C" fn v_fstat(fd: i32, st: *mut sys::stat) -> i32 {
    let f = fil(fd);
    if f.is_null() {
        return fail(sys::EBADF);
    }
    std::ptr::write_bytes(st, 0, 1);
    (*st).st_mode = sys::S_IFREG as sys::mode_t;
    (*st).st_size = (*f).obj.objsize as sys::off_t;
    0
}

// FatFs keeps no record of an open file (no share lock): a FIL is FatFs's
// only on its own side.
const _: () = assert!(sys::CONFIG_FATFS_FS_LOCK == 0);

/// Close a slot's FIL and free it. Ours are opened for reading only, and
/// with no share lock (above) `f_close` of such a FIL does nothing but take
/// the volume lock twice (its `f_sync` and its own check) to mark it closed;
/// so it is freed without: the player's close of the outgoing GIF at a
/// switch (in the tick) waited there behind a card write holding the lock,
/// hundreds of ms during saves (the rig's "transition" misses). A FIL
/// opened for writing (none today) still goes through `f_close`.
///
/// # Safety
/// `f` is a FIL from `Box` that `f_open` filled, used by nothing else.
unsafe fn free_fil(f: *mut sys::FIL) {
    // SAFETY: as the caller's.
    unsafe {
        if (*f).flag & sys::FA_WRITE as u8 != 0 {
            sys::f_close(f);
        }
        drop(Box::from_raw(f));
    }
}

unsafe extern "C" fn v_close(fd: i32) -> i32 {
    // The raw state and the open-set entry go before the slot is free
    // (`fatread::Slots::release`): another task's open can then never find
    // this file's extents in it. The read-ahead forgets it first.
    crate::readahead::closed(fd as usize);
    match TABLE.release(fd as usize, |k| open_set(|o| o.remove(k))) {
        Some((f, _raw)) => {
            free_fil(f);
            0
        }
        None => fail(sys::EBADF),
    }
}

fn register_vfs() -> bool {
    if VFS_UP.load(Relaxed) {
        return true;
    }
    // SAFETY: a zeroed table with the five calls std's File makes; IDF
    // copies it.
    let ok = unsafe {
        let mut v: sys::esp_vfs_t = std::mem::zeroed();
        v.flags = sys::ESP_VFS_FLAG_DEFAULT as i32;
        v.__bindgen_anon_6.open = Some(v_open);
        v.__bindgen_anon_3.read = Some(v_read);
        v.__bindgen_anon_2.lseek = Some(v_lseek);
        v.__bindgen_anon_8.fstat = Some(v_fstat);
        v.__bindgen_anon_7.close = Some(v_close);
        sys::esp_vfs_register(c"/fidx".as_ptr(), &v, std::ptr::null_mut()) == sys::ESP_OK
    };
    VFS_UP.store(ok, Relaxed);
    ok
}

// ---- opening ----------------------------------------------------------------

/// Open a card file (`/sdcard/...`): through the index for `/gifs/` files
/// when it has them, else by path. Errors are the path open's (or a card
/// error from the index's own read, which `sdhealth` must see).
pub fn open(full: &str) -> io::Result<File> {
    match indexed(full) {
        Some(r) => r,
        None => File::open(full),
    }
}

/// Through the index only: `None` when it cannot (not indexed, stale, no
/// index), never a directory scan. For opening an original ahead.
pub fn indexed(full: &str) -> Option<io::Result<File>> {
    indexed_as(full, false)
}

/// [`indexed`] for the player: the file read raw ([`open_player`]).
pub fn indexed_player(full: &str) -> Option<io::Result<File>> {
    if covered(full) {
        return Some(Err(refused()));
    }
    indexed_as(full, true)
}

/// Open a file for the player (`/sdcard/...`): through the index for a
/// `/gifs/` file when it has it, else by path, read raw when a slot is
/// free (the module docs), else as [`open`] does.
pub fn open_player(full: &str) -> io::Result<File> {
    if covered(full) {
        return Err(refused());
    }
    // The display core's opens, timed by route (`misses.recent`): the index
    // (no volume lock), else by path (FatFs's directory walk, behind the
    // volume lock: a save's walk of a big folder holds it).
    let display = crate::cardspeed::on_display_core();
    let t = now_us();
    let indexed = indexed_as(full, true);
    if display {
        OPEN_INDEX_US.fetch_add(now_us().wrapping_sub(t), Relaxed);
    }
    if let Some(r) = indexed {
        return r;
    }
    let t = now_us();
    let r = open_by_path(full);
    if display {
        let us = now_us().wrapping_sub(t);
        OPEN_PATH_US.fetch_add(us, Relaxed);
        PATH_OPENS.fetch_add(1, Relaxed);
        PATH_OPEN_MAX_US.fetch_max(us, Relaxed);
    }
    r
}

/// [`open_player`] by path: FatFs's directory walk, read raw after.
#[inline(never)]
fn open_by_path(full: &str) -> io::Result<File> {
    if crate::CARD_READY.load(Relaxed) && VFS_UP.load(Relaxed) {
        if let Some(f) = open_path_raw(full)? {
            return Ok(f);
        }
    }
    File::open(full)
}

/// The display core's open time since the last call: `(index route µs, by
/// path µs)`, and reset (the main loop, per frame).
pub fn take_open_split() -> (u32, u32) {
    (OPEN_INDEX_US.swap(0, Relaxed), OPEN_PATH_US.swap(0, Relaxed))
}

static OPEN_INDEX_US: AtomicU32 = AtomicU32::new(0);
static OPEN_PATH_US: AtomicU32 = AtomicU32::new(0);
/// The display core's opens by path (the index could not open the file),
/// the longest, and why the index did not: busy (a build swapping it in),
/// not loaded.
static PATH_OPENS: AtomicU32 = AtomicU32::new(0);
static PATH_OPEN_MAX_US: AtomicU32 = AtomicU32::new(0);
static INDEX_BUSY: AtomicU32 = AtomicU32::new(0);
static INDEX_UNLOADED: AtomicU32 = AtomicU32::new(0);
/// Bumped at every index swap: a lookup that read the old one's records
/// after letting go of the lock drops them.
static LOADED_GEN: AtomicU32 = AtomicU32::new(0);
/// Copies found absent by the index (no path open).
static ABSENT: AtomicU32 = AtomicU32::new(0);

fn now_us() -> u32 {
    // SAFETY: a plain clock read.
    unsafe { sys::esp_timer_get_time() as u32 }
}

/// By path into a FIL of ours (FatFs's `f_open`: the directory walk the
/// path open does), its chain read, read raw. `Ok(None)`: no free slot,
/// or the volume is not FAT32.
fn open_path_raw(full: &str) -> io::Result<Option<File>> {
    Ok(open_path_raw_as(full, true)?.map(|(f, _)| f))
}

/// A playlist's manifest or index (`main::open_catalog`), read raw: the
/// lookup of the next file to play (on the display core, `open` time) read
/// FatFs's way, behind the volume lock, which a save's walk of a big folder
/// holds for seconds on a slow card (iso15: a 750 ms open, none of it the
/// index's or a path open's). Opened by path (once, when the playlist
/// loads), not noted in the open set: only the main task writes these
/// files, and it lets the playlist go first (`retire`); a file deleted
/// under it reads stale bytes, as FatFs's handle would. FatFs's handle when
/// it cannot be read raw.
pub fn open_catalog_file(full: &str) -> io::Result<File> {
    if crate::CARD_READY.load(Relaxed) && register_vfs() {
        if let Some((f, raw)) = open_path_raw_as(full, false)? {
            if raw {
                return Ok(f);
            }
            // Opened, but FatFs reads it (its chain past 64 runs, or not
            // canonical): its reads wait for the volume lock.
            CATALOG_FATFS.fetch_add(1, Relaxed);
            log::warn!("{full}: the playlist file is read through FatFs (its lookups wait for the volume lock)");
            return Ok(f);
        }
    }
    CATALOG_FATFS.fetch_add(1, Relaxed);
    File::open(full)
}

/// Playlist files read through FatFs, not raw (`fat_index.catalog_fatfs`).
static CATALOG_FATFS: AtomicU32 = AtomicU32::new(0);


/// [`open_path_raw`]; `player`: noted in the open set as the player's (a
/// GIF), else not (a playlist file).
/// The file and whether it reads raw (else FatFs reads it).
fn open_path_raw_as(full: &str, player: bool) -> io::Result<Option<(File, bool)>> {
    let Some(card) = full.strip_prefix(config::SD_MOUNT).filter(|c| c.starts_with('/')) else {
        return Ok(None);
    };
    let Ok((fs, geo)) = volume() else {
        return Ok(None);
    };
    let path = std::ffi::CString::new(format!("0:{card}")).map_err(|_| errno(sys::EINVAL))?;
    // SAFETY: a zeroed FIL f_open fills; freed below or by `v_close`.
    let f = unsafe {
        let p = std::alloc::alloc_zeroed(std::alloc::Layout::new::<sys::FIL>()) as *mut sys::FIL;
        if p.is_null() {
            return Err(errno(sys::ENOMEM));
        }
        let r = sys::f_open(p, path.as_ptr(), sys::FA_READ as u8);
        if r != sys::FRESULT_FR_OK {
            drop(Box::from_raw(p));
            return Err(match r {
                sys::FRESULT_FR_NO_FILE | sys::FRESULT_FR_NO_PATH | sys::FRESULT_FR_INVALID_NAME => errno(sys::ENOENT),
                r => errno(fr_code(r)),
            });
        }
        p
    };
    // SAFETY: the FIL f_open filled.
    let (first, size) = unsafe { ((*f).obj.sclust, (*f).obj.objsize as u32) };
    let ext = VolLock::take(fs).and_then(|_l| fatread::chain(&geo, first, size, &mut |lba, b| read_locked(fs, lba, b)));
    // SAFETY: the mounted volume's drive number.
    let pdrv = unsafe { (*fs).pdrv };
    let raw = match ext {
        Ok(ext) if fatread::is_canonical(card) => Some(Raw { file: fatread::RawFile::new(ext), pdrv, broken: false, follow: player }),
        Ok(_) => None,
        // As in `open_at`: FatFs reads this one.
        Err(e) => {
            log::warn!("fat index: {full}: no raw reads ({e}); FatFs reads it");
            None
        }
    };
    let is_raw = raw.is_some();
    Ok(hand_out(f, player.then_some(full), raw, None)?.map(|f| (f, is_raw)))
}

/// Put an open FIL in a free slot and open it there as a `File`; with
/// `raw`, read raw and noted in [`holds`]. `Ok(None)`: every slot busy
/// (the FIL is freed; the caller opens by path).
fn hand_out(f: *mut sys::FIL, player: Option<&str>, raw: Option<Raw>, since: Option<u32>) -> io::Result<Option<File>> {
    // A player's open is noted in the open set under its lock unless a
    // write intent covers the path, the check and the note one step; then
    // it is not opened at all (`fatread::OpenSet`).
    let mut again = false;
    let mut note = |k: usize| {
        player.is_some_and(|full| {
            open_set(|o| match since {
                Some(seq) => match o.note_since(k, full, seq) {
                    fatread::Note::Noted => true,
                    fatread::Note::Covered => false,
                    fatread::Note::Since => {
                        again = true;
                        false
                    }
                },
                None => o.insert_unless_covered(k, full),
            })
        })
    };
    let claimed = TABLE.claim(f, raw.map(Box::new), player.map(|_| &mut note as &mut dyn FnMut(usize) -> bool));
    let k = match claimed {
        Ok(k) => k,
        Err(why) => {
            // SAFETY: never handed out.
            unsafe { free_fil(f) };
            return match (why, player) {
                // A write began during a lock-free open (now over, or the
                // ring overflowed): the caller opens again under the lock.
                (fatread::Claim::Refused, _) if again => Err(errno(sys::EAGAIN)),
                (fatread::Claim::Refused, _) => {
                    INTENT_REFUSED.fetch_add(1, Relaxed);
                    Err(errno(sys::ENOENT))
                }
                (fatread::Claim::Full, p) => {
                    if SLOTS_FULL.fetch_add(1, Relaxed) == 0 {
                        log::warn!("fat index: all {SLOTS} /fidx slots busy");
                    }
                    // The player's opens are never by path behind the open
                    // set's back: refused (the engine skips the file).
                    if p.is_some() {
                        Err(errno(sys::EBUSY))
                    } else {
                        Ok(None)
                    }
                }
            };
        }
    };
    match File::open(SLOT_PATHS[k]) {
        Ok(file) => Ok(Some(file)),
        Err(e) => {
            if let Some((f, _)) = TABLE.release(k, |k| open_set(|o| o.remove(k))) {
                // SAFETY: never handed out.
                unsafe { free_fil(f) };
            }
            Err(e)
        }
    }
}

fn indexed_as(full: &str, raw: bool) -> Option<io::Result<File>> {
    // Not before the handover: the card may still be remounted (`unmounted`).
    if !crate::CARD_READY.load(Relaxed) {
        return None;
    }
    indexed_now(full, raw)
}

/// [`indexed_as`] without the handover check (psi-probe's self-check, on
/// the main task with the card already its own).
fn indexed_now(full: &str, raw: bool) -> Option<io::Result<File>> {
    let card = full.strip_prefix(config::SD_MOUNT).filter(|c| c.starts_with("/gifs/") || c.starts_with("/cache/"))?;
    let name = card.rsplit('/').next()?;
    if !VFS_UP.load(Relaxed) || !name.is_ascii() || name.len() > 255 {
        return None;
    }
    let h = fatdir::hash(card.as_bytes());
    // Candidates, copied out so the index lock is not held across the card
    // reads below.
    let mut cands = [0u32; 8];
    let mut nc = 0;
    // The loaded index was asked and its whole bucket read (so no record
    // means none in it).
    let mut whole = false;
    // A file written, renamed or removed since the build: the delta knows.
    let delta_lock = DELTA.try_lock();
    // (Busy: a copy written since the build may be in it, unknown here.)
    let delta_known = delta_lock.is_ok();
    let delta = delta_lock.ok().and_then(|d| d.iter().rev().find(|(k, _, _)| *k == h).map(|(_, l, _)| *l));
    if let Some(loc) = delta {
        // Removed (or renamed from) since the build: not there, without a
        // directory walk to find that out.
        let Some(loc) = loc else {
            return Some(Err(errno(sys::ENOENT)));
        };
        cands[0] = loc;
        nc = 1;
    } else {
        // Never wait for the index (a build swapping it in, or another
        // task's lookup): this may be the display task. Busy means a path open.
        let Ok(mut g) = LOADED.try_lock() else {
            if raw {
                INDEX_BUSY.fetch_add(1, Relaxed);
            }
            return None;
        };
        // Not loaded yet: the network worker loads it (`tick`); a load
        // here (its directory walks under the volume lock) could be on the
        // display task.
        let Some(l) = g.as_mut().and_then(Option::as_mut) else {
            if raw {
                INDEX_UNLOADED.fetch_add(1, Relaxed);
            }
            return None;
        };
        let b = (h >> 24) as usize;
        // A bucket holds ~1/256 of the records (~40 on a stock card); past
        // 256 (2 KB) the rest of it is not looked at, and those names open
        // by path.
        let (lo, hi) = (l.fanout[b], l.fanout[b + 1].min(l.fanout[b] + 256));
        whole = l.fanout[b + 1] - l.fanout[b] <= 256;
        let len = (hi - lo) as usize * 8;
        let at = fatdir::RECORDS_AT + lo as u64 * 8;
        // The records' sectors are mapped under the index's lock and read
        // after it (a lookup that found the lock held went by path, behind
        // the volume lock); an index swapped in meanwhile drops what was
        // read (its old file may be freed).
        let got: io::Result<Vec<u8>> = match l.raw.as_ref() {
            Some((rf, pdrv)) => {
                let (pdrv, gen) = (*pdrv, LOADED_GEN.load(Relaxed));
                let first = at - at % SS as u64;
                let sectors = (at + len as u64).div_ceil(SS as u64) - first / SS as u64;
                let mut runs = [(0u32, 0u32); 8];
                let mut nr = 0;
                let mut mapped = 0u64;
                while mapped < sectors && nr < runs.len() {
                    let Some((lba, run)) = rf.extents().map(first + mapped * SS as u64) else { break };
                    let n = (run as u64).min(sectors - mapped);
                    runs[nr] = (lba, n as u32);
                    nr += 1;
                    mapped += n;
                }
                drop(g);
                if mapped < sectors {
                    Err(io::Error::from(io::ErrorKind::UnexpectedEof))
                } else {
                    let mut buf = vec![0u8; sectors as usize * SS];
                    let mut done = 0usize;
                    let mut r = Ok(());
                    for &(lba, n) in &runs[..nr] {
                        let bytes = n as usize * SS;
                        // SAFETY: `bytes` of `buf` from `done`; the mounted card's drive.
                        if unsafe { sys::ff_disk_read(pdrv, buf[done..done + bytes].as_mut_ptr(), lba, n) } != 0 {
                            r = Err(errno(sys::EIO));
                            break;
                        }
                        done += bytes;
                    }
                    if LOADED_GEN.load(Relaxed) != gen {
                        return None;
                    }
                    r.map(|()| {
                        let from = (at - first) as usize;
                        buf[from..from + len].to_vec()
                    })
                }
            }
            None => {
                let mut recs = vec![0u8; len];
                let r = l.file.seek(SeekFrom::Start(at)).and_then(|_| l.file.read_exact(&mut recs));
                drop(g);
                r.map(|()| recs)
            }
        };
        let recs = match got {
            Ok(r) => r,
            Err(e) => {
                if crate::sdhealth::is_card_error(&e) {
                    return Some(Err(e));
                }
                // Anything else (a short or damaged index): drop it, open by
                // path (if it is still the one read).
                if let Ok(mut g) = LOADED.try_lock() {
                    *g = Some(None);
                }
                DIRTY.store(true, Relaxed);
                return None;
            }
        };
        for r in recs.chunks_exact(8).filter(|r| r[..4] == h.to_le_bytes()).take(cands.len()) {
            cands[nc] = u32::from_le_bytes(r[4..].try_into().unwrap());
            nc += 1;
        }
    }
    for &loc in &cands[..nc] {
        match open_at(loc, name.as_bytes(), raw.then_some(full)) {
            Ok(Some(f)) => {
                HITS.fetch_add(1, Relaxed);
                return Some(Ok(f));
            }
            Ok(None) => {}
            // A write of this file began during the open: open it again,
            // by path under the lock (no miss: the record was right).
            Err(e) if e.raw_os_error() == Some(sys::EAGAIN as i32) => return None,
            Err(e) => return Some(Err(e)),
        }
    }
    // A prepared copy the index has no record of is not there: only this
    // device writes `/cache`, and every copy it writes is in the delta. The
    // player tried the copy first for every GIF without one, by path: a
    // walk of its bucket behind the volume lock (iso15: most of its 52 path
    // opens, up to 305 ms on the slow card). `/gifs` keeps the path open:
    // a PC may have added a GIF since the build.
    //
    // Only while that holds: the index was built this boot (one from an
    // earlier boot lacks the copies written after it, whose delta was in
    // RAM, and any a PC added: `tick` rebuilds once a boot), and the delta
    // has kept every change since (past 256 its oldest go). Only a copy's
    // path, `/cache/<category>/<bucket>/<name>`: the files at `/cache`'s
    // root (`.failed`, `.names`, ...) are not indexed.
    if nc == 0
        && whole
        && delta_known
        && BUILT_THIS_BOOT.load(Relaxed)
        && !DELTA_OVERFLOWED.load(Relaxed)
        && card.starts_with("/cache/")
        && card.matches('/').count() == 4
        && fatread::is_canonical(card)
    {
        ABSENT.fetch_add(1, Relaxed);
        return Some(Err(errno(sys::ENOENT)));
    }
    // Only a record that no longer verifies says the index is stale: a name
    // it never had (an 8.3-only or non-ASCII name, a file added since) is
    // just opened by path, and must not rebuild it every 10 min.
    if nc > 0 && MISSES.fetch_add(1, Relaxed) + 1 >= config::FATIDX_STALE_REBUILD {
        DIRTY.store(true, Relaxed);
    }
    None
}

/// Verify the entry at `loc` and open it (`Ok(None)`: not this file).
fn open_at(loc: u32, name: &[u8], raw: Option<&str>) -> io::Result<Option<File>> {
    let (fs, geo) = volume()?;
    let (rel, slot) = (loc >> 4, (loc & 15) as usize);
    if rel as u64 >= (geo.n_fatent as u64 - 2) * geo.csize as u64 {
        return Ok(None);
    }
    let mut buf = vec![0u8; 2 * SS];
    let (pbuf, cur) = buf.split_at_mut(SS);
    // The player's opens read the card without FatFs's volume lock (the
    // card's I/O lock only, `card_read`): a FatFs call walking a big
    // directory meanwhile no longer holds them up. Coherent for any file no
    // write is changing (tests/fatopen_mixed.rs: every mixture of a
    // create's, delete's, rename's or directory growth's sectors from before
    // and after opens the untouched files the same); a write that covers
    // this file and began since `since` refuses the open at its note
    // (`OpenSet::insert_unless_covered_since`). Other opens (the server's,
    // the builder's) take the lock and read through FatFs's window.
    let player = raw.is_some();
    let since = player.then(|| open_set(|o| o.seq()));
    let lock = if player { None } else { Some(VolLock::take(fs)?) };
    let read = |lba: u32, b: &mut [u8]| if player { card_read(fs, lba, b) } else { read_locked(fs, lba, b) };
    read(geo.database + rel, cur)?;
    let mut v = fatdir::verify(cur, None, slot, name);
    // Indexed names never start in the previous cluster: the sector before
    // is in this one.
    if v == Verify::NeedPrev && !rel.is_multiple_of(geo.csize) {
        read(geo.database + rel - 1, pbuf)?;
        v = fatdir::verify(cur, Some(pbuf), slot, name);
    }
    let Verify::Ok { cluster, size, attr } = v else {
        return Ok(None);
    };
    // SAFETY: a zeroed FIL filled as f_open fills one for reading, still
    // under the volume lock; it is FatFs's from here (f_read/f_lseek/f_close).
    let f = unsafe {
        let p = std::alloc::alloc_zeroed(std::alloc::Layout::new::<sys::FIL>()) as *mut sys::FIL;
        if p.is_null() {
            return Err(errno(sys::ENOMEM));
        }
        (*p).obj.fs = fs;
        (*p).obj.id = (*fs).id;
        (*p).obj.attr = attr;
        (*p).obj.sclust = cluster;
        (*p).obj.objsize = size;
        (*p).flag = sys::FA_READ as u8;
        (*p).dir_sect = geo.database + rel;
        p
    };
    // The player's: its chain, still under the lock (the FAT sectors
    // through the window).
    let raw_state = match raw {
        Some(full) => match fatread::chain(&geo, cluster, size, &mut |lba, b| read(lba, b)) {
            // SAFETY: the mounted volume's drive number.
            Ok(ext) if fatread::is_canonical(full) => Some(Raw { file: fatread::RawFile::new(ext), pdrv: unsafe { (*fs).pdrv }, broken: false, follow: true }),
            Ok(_) => None,
            // A chain that does not add up (or a card error reading it):
            // FatFs reads this one, as before.
            Err(e) => {
                log::warn!("fat index: {full}: no raw reads ({e}); FatFs reads it");
                None
            }
        },
        None => None,
    };
    drop(lock);
    hand_out(f, raw, raw_state, since)
}

/// The index file at `path` (`/sdcard/...`: [`INDEX`], or a build's [`NEW`]
/// before it is renamed: its handle and extents stay good across the rename).
fn load(path: &str) -> io::Result<Loaded> {
    load_as(path, None)
}

/// [`load`]; `known`: the directories as a build just walked them (no walk
/// again: a directory changed since aborts the build, `dir_changed`).
fn load_as(path: &str, known: Option<fatdir::DirSet>) -> io::Result<Loaded> {
    let mut file = File::open(path)?;
    let mut raw = vec![0u8; fatdir::HEADER + fatdir::FANOUT * 4];
    file.read_exact(&mut raw)?;
    let (_, geo) = volume()?;
    let h = Header::decode(&raw).filter(|h| h.n_fatent == geo.n_fatent && h.database == geo.database);
    let h = h.ok_or_else(|| errno(sys::EINVAL))?;
    let mut fanout = Box::new([0u32; fatdir::FANOUT]);
    for (i, c) in raw[fatdir::HEADER..].chunks_exact(4).enumerate() {
        fanout[i] = u32::from_le_bytes(c.try_into().unwrap());
    }
    file.seek(SeekFrom::Start(fatdir::RECORDS_AT + h.count as u64 * 8))?;
    let mut e = [0u8; 8];
    let pair = |e: &[u8; 8]| (u32::from_le_bytes(e[..4].try_into().unwrap()), u32::from_le_bytes(e[4..].try_into().unwrap()));
    // The directories it was built over must be the ones there now: a
    // directory renamed or replaced on a PC leaves its entries where they
    // were, and a record would then name a file in another directory.
    let mut built = fatdir::DirSet::default();
    for _ in 0..h.dirs.min(1 << 16) {
        file.read_exact(&mut e)?;
        built.add(pair(&e));
    }
    let now = match known {
        Some(k) => k,
        None => dir_set()?,
    };
    if built != now {
        DIRTY.store(true, Relaxed);
        return Err(errno(sys::ESTALE));
    }
    RECORDS.store(h.count, Relaxed);
    MISSES.store(0, Relaxed);
    let (t, b) = (TOUCHED_MS.load(Relaxed), BUILT_MS.load(Relaxed));
    // Current unless the API changed something after the build started.
    if t == 0 || (b != 0 && since(b, t) as i32 > 0) {
        DIRTY.store(false, Relaxed);
    }
    // The names file of the same build is as current as the index (the
    // directories are the ones it walked, and this boot's changes are
    // noted as they happen); an index without one, from a build before
    // names files, gets one by a walk of its own.
    if names_present() {
        NAMES_OK.store(true, Relaxed);
    } else {
        NAMES_WANTED.store(true, Relaxed);
    }
    // Its extents, for the player's lookups without the volume lock.
    let raw = index_extents(path);
    Ok(Loaded { file, fanout, raw })
}

/// The index file's extents (its own FIL's first cluster and size, its
/// chain under the volume lock): `None` if anything does not add up.
fn index_extents(file: &str) -> Option<(fatread::RawFile, u8)> {
    let (fs, geo) = volume().ok()?;
    let path = std::ffi::CString::new(format!("0:{}", file.strip_prefix(config::SD_MOUNT)?)).ok()?;
    // SAFETY: a zeroed FIL f_open fills; closed before return.
    let (first, size) = unsafe {
        let mut f: sys::FIL = std::mem::zeroed();
        if sys::f_open(&mut f, path.as_ptr(), sys::FA_READ as u8) != sys::FRESULT_FR_OK {
            return None;
        }
        let r = (f.obj.sclust, f.obj.objsize as u32);
        sys::f_close(&mut f);
        r
    };
    let ext = VolLock::take(fs).ok().and_then(|_l| fatread::chain(&geo, first, size, &mut |lba, b| read_locked(fs, lba, b)).ok())?;
    // SAFETY: the mounted volume's drive number.
    Some((fatread::RawFile::new(ext), unsafe { (*fs).pdrv }))
}

// ---- keeping it current ------------------------------------------------------

/// The card is being unmounted (`SdCard`'s drop, before the volume goes):
/// forget the volume and the index file, so nothing reads a freed `FATFS`.
/// This relies on there being no unmount after `CARD_READY`: before the
/// handover nothing has an index file open and no build runs (both wait for
/// `CARD_READY`); after it the card is only ever dropped by a restart. Open
/// `/fidx` FILs and a build in flight are not closed here.
pub fn unmounted() {
    abort_job();
    FS.store(std::ptr::null_mut(), Relaxed);
    BUILT_THIS_BOOT.store(false, Relaxed);
    *LOADED.lock().unwrap() = None;
    FAILED.store(false, Relaxed);
    BUILT_MS.store(0, Relaxed);
    RECORDS.store(0, Relaxed);
    DIRTY.store(true, Relaxed);
}

/// The card is the player's now (the handover): look for its index on the
/// next turn, also when it mounted late after a boot with no card.
pub fn mounted() {
    RELOAD.store(true, Relaxed);
}

/// The API changed `path` (an upload, move or delete): rebuild once things
/// are quiet. Only `/gifs` is indexed.
pub fn touched(path: &str) {
    // The delta keeps lookups current (`note_written`, `note_gone`); this
    // only says the API is busy (a build waits for a quiet spell).
    if path.strip_prefix(config::SD_MOUNT).unwrap_or(path).starts_with("/gifs/") {
        TOUCHED_MS.store(uptime_ms(), Relaxed);
    }
}

/// A directory under `/gifs` is going or coming: stop using the index at
/// once (its records may now point into another directory) and rebuild
/// when things are quiet.
pub fn dir_changed() {
    abort_job();
    NAMES_OK.store(false, Relaxed);
    TOUCHED_MS.store(uptime_ms(), Relaxed);
    DIRTY.store(true, Relaxed);
    *LOADED.lock().unwrap() = Some(None);
}

/// The network worker's turn: rebuild when wanted, quiet and affordable;
/// a build under way goes on by one slice ([`slice`]).
pub fn tick() {
    if !crate::CARD_READY.load(Relaxed) || !register_vfs() {
        return;
    }
    #[cfg(feature = "emu-ballast")]
    ballast();
    if job_running() {
        run_slice();
        return;
    }
    // The first look at the card decides whether there is an index at all.
    let missing = {
        let need = {
            let mut g = LOADED.lock().unwrap();
            if RELOAD.swap(false, Relaxed) {
                *g = None;
                DIRTY.store(true, Relaxed);
            }
            g.is_none()
        };
        if need {
            // Loaded without the index's lock: its directory walks wait
            // behind card writes for the volume lock, and the player's
            // lookups meanwhile must not find the lock taken (they go by
            // path while there is no index, as they would anyway).
            let l = load(INDEX).ok();
            // An index from an earlier boot: rebuilt once this boot, when
            // quiet (the copies written since its build were noted in RAM
            // only; a PC may have added some).
            if l.is_some() && !BUILT_THIS_BOOT.load(Relaxed) {
                DIRTY.store(true, Relaxed);
            }
            let mut g = LOADED.lock().unwrap();
            if g.is_none() {
                *g = Some(l);
            }
        }
        LOADED.lock().unwrap().as_ref().is_some_and(Option::is_none)
    };
    // A current index with no names file: the names alone.
    let names_only = !DIRTY.load(Relaxed) && !missing && NAMES_WANTED.load(Relaxed);
    if !DIRTY.load(Relaxed) && !names_only {
        return;
    }
    let now = uptime_ms();
    let touched = TOUCHED_MS.load(Relaxed);
    let quiet = touched == 0 || since(now, touched) >= config::FATIDX_QUIET_S * 1000;
    // A missing index, or one the API changed since the last build, is
    // built once things are quiet; one gone stale by misses, or after a
    // failed build, waits the gap (a card whose names cannot all be indexed,
    // or a failing card, must not be walked in a loop).
    let built = BUILT_MS.load(Relaxed);
    let changed = touched != 0 && (built == 0 || since(touched, built) as i32 >= 0);
    let spaced = ((missing || changed) && !FAILED.load(Relaxed))
        || built == 0
        || since(now, built) >= config::FATIDX_MIN_GAP_S * 1000;
    if !quiet
        || !spaced
        || now < config::FATIDX_BOOT_DELAY_S * 1000
        || crate::cache_builder::task_up()
        || crate::cache_builder::transfer_in_flight()
        || (crate::state::free_heap() as usize) < config::FATIDX_START_HEAP
        || !crash_allows(now)
    {
        return;
    }
    match Job::new(names_only) {
        Ok(job) => {
            if !names_only {
                BUILT_MS.store(now, Relaxed);
                // The walk starts now: the delta keeps what changes from here on.
                DELTA_OVERFLOWED.store(false, Relaxed);
            }
            log::info!(
                "fat index: {} started (heap {} B free, largest {} B)",
                if names_only { "names file" } else { "build" },
                crate::state::free_heap(),
                crate::state::largest_free_block()
            );
            *JOB.lock().unwrap_or_else(|p| p.into_inner()) = Some(job);
            BUILDING.store(true, Relaxed);
            run_slice();
        }
        Err(e) => {
            log::warn!("fat index: no build started ({e})");
            if names_only {
                NAMES_WANTED.store(false, Relaxed);
            } else {
                FAILED.store(true, Relaxed);
                BUILT_MS.store(now, Relaxed);
            }
        }
    }
}

/// The emulator's spare-sized heap (feature `emu-ballast`): once, 12 s
/// after boot (playing), heap taken in 2 KB blocks and kept until ~45 KB is
/// free, as on the spare while it plays.
#[cfg(feature = "emu-ballast")]
fn ballast() {
    static DONE: AtomicBool = AtomicBool::new(false);
    if uptime_ms() < 12_000 || DONE.swap(true, Relaxed) {
        return;
    }
    let mut taken = 0usize;
    while crate::state::free_heap() as usize > 47 * 1024 {
        std::mem::forget(vec![1u8; 2048]);
        taken += 2048;
    }
    log::info!(
        "ballast: {taken} B taken; heap {} B free, largest {} B",
        crate::state::free_heap(),
        crate::state::largest_free_block()
    );
}

/// `/gifs`'s and `/cache`'s first clusters (no `/cache`: no buckets).
fn roots() -> io::Result<(Option<u32>, Option<u32>)> {
    Ok((Some(open_dir("/gifs")?.1), open_dir("/cache").ok().map(|d| d.1)))
}

/// A buffer of `n` bytes, or `OutOfMemory` (never an abort).
fn try_buf(n: usize) -> io::Result<Vec<u8>> {
    let mut v = Vec::new();
    v.try_reserve_exact(n).map_err(|_| heap_short())?;
    v.resize(n, 0);
    Ok(v)
}

fn heap_short() -> io::Error {
    io::Error::new(io::ErrorKind::OutOfMemory, "heap short: the build waits")
}

/// A box of a `T` made in place by `make`, or `OutOfMemory`.
fn try_box<T>(make: impl FnOnce() -> T) -> io::Result<Box<T>> {
    let layout = std::alloc::Layout::new::<T>();
    // SAFETY: a fresh allocation of T's layout, written once before the Box
    // takes it.
    unsafe {
        let p = std::alloc::alloc(layout) as *mut T;
        if p.is_null() {
            return Err(heap_short());
        }
        p.write(make());
        Ok(Box::from_raw(p))
    }
}

/// The volume read the build and the load use: the volume lock for the
/// read alone. `pace`: a 1 ms pause after each (the player's reads first).
fn volume_read(fs: *mut sys::FATFS, pace: bool) -> impl FnMut(u32, &mut [u8]) -> io::Result<()> {
    move |lba: u32, buf: &mut [u8]| -> io::Result<()> {
        BUILD_FLOOR.fetch_min(crate::state::free_heap(), Relaxed);
        {
            let _l = VolLock::take(fs)?;
            read_locked(fs, lba, buf)?;
        }
        if pace {
            std::thread::sleep(Duration::from_millis(1));
        }
        Ok(())
    }
}

/// The directories the index covers, as [`load`] compares them with an
/// index's (`fatdir::Cursor`, read raw as the build reads them: the
/// directories' own entries, not their contents).
fn dir_set() -> io::Result<fatdir::DirSet> {
    let (fs, geo) = volume()?;
    let (gifs, cache) = roots()?;
    let mut cursor = try_box(|| fatdir::Cursor::new(gifs, cache))?;
    let mut read = volume_read(fs, false);
    let mut set = fatdir::DirSet::default();
    while let Some(d) = cursor.next(&geo, &mut read)? {
        set.add((d.key, d.first));
    }
    Ok(set)
}

/// The most `/gifs` folders a names file is written for: its table is
/// 12 B a folder in RAM until it is written. More: no names file (names=1
/// lists instead).
const NAMES_MAX_FOLDERS: usize = 512;

/// [`NAMES_NEW`] in place of [`NAMES`], trusted from now on with the
/// changes noted since `started` (ms uptime, the walk's start).
fn install_names(started: u32) -> bool {
    let _ = fs::remove_file(NAMES);
    if fs::rename(NAMES_NEW, NAMES).is_err() {
        return false;
    }
    NAMES_DELTA.lock().unwrap_or_else(|p| p.into_inner()).retain(|(_, _, at)| since(*at, started) as i32 >= 0);
    NAMES_WANTED.store(false, Relaxed);
    NAMES_OK.store(true, Relaxed);
    true
}

// ---- the build: a job in slices ------------------------------------------------

/// The build's temporary records (`fatdir::TMP_DIR`, `fatdir::gather`).
const TMP: &str = "/sdcard/cache/.fatidx.tmp";

/// The lowest free heap a build's card reads saw (`fat_index.build_floor`),
/// and its largest group (bytes).
static BUILD_FLOOR: AtomicU32 = AtomicU32::new(0);
static BUILD_GROUP: AtomicU32 = AtomicU32::new(0);

/// Uptime (ms) until which the player needs nothing of the card or the heap
/// beyond what it holds (`Engine::quiet_for`, from the main loop): the
/// build works only before it.
static QUIET_UNTIL: AtomicU32 = AtomicU32::new(0);

/// The main loop's word on the player (each tick): quiet for `d` from now.
pub fn set_quiet(d: Duration) {
    QUIET_UNTIL.store(uptime_ms().wrapping_add(d.as_millis().min(60_000) as u32), Relaxed);
}

enum Stage {
    /// One walk of every indexed directory: the names, the temporary
    /// records, the count.
    Walk,
    /// Passes over the temporary records, from this bucket.
    Passes(usize),
    /// The directories after the records.
    Trailer,
    Done,
}

/// A build under way, between the network worker's slices: what it holds
/// is this (the cursor's two sectors, the fanout: ~3 KB) and the names'
/// table, whatever the card holds. A slice takes its buffers and gives them
/// back.
struct Job {
    names_only: bool,
    cursor: Box<fatdir::Cursor>,
    fanout: Box<[u32; fatdir::FANOUT]>,
    set: fatdir::DirSet,
    stage: Stage,
    tmp: Option<File>,
    names: Option<opendmd_core::names_file::Writer<File>>,
    names_folders: usize,
    out: Option<File>,
    started: u32,
    slices: u32,
    passes: u32,
    cut0: u32,
}

static JOB: Mutex<Option<Box<Job>>> = Mutex::new(None);

fn job_running() -> bool {
    JOB.lock().unwrap_or_else(|p| p.into_inner()).is_some()
}

impl Job {
    fn new(names_only: bool) -> io::Result<Box<Job>> {
        volume()?;
        let _ = fs::create_dir(config::CACHE_ROOT);
        let (gifs, cache) = roots()?;
        let cursor = try_box(|| fatdir::Cursor::new(gifs, if names_only { None } else { cache }))?;
        let fanout = try_box(|| [0u32; fatdir::FANOUT])?;
        let tmp = if names_only { None } else { Some(File::create(TMP)?) };
        let names = Some(opendmd_core::names_file::Writer::new(File::create(NAMES_NEW)?));
        BUILD_FLOOR.store(crate::state::free_heap(), Relaxed);
        BUILD_GROUP.store(0, Relaxed);
        try_box(|| Job {
            names_only,
            cursor,
            fanout,
            set: fatdir::DirSet::default(),
            stage: Stage::Walk,
            tmp,
            names,
            names_folders: 0,
            out: None,
            started: uptime_ms(),
            slices: 0,
            passes: 0,
            cut0: crate::TRANSITIONS_CUT.load(Relaxed),
        })
    }

    /// Work until `until` (uptime ms) or the heap is short: `Ok(true)` when
    /// the job is done. Stops only between directories and between passes.
    fn slice(&mut self, until: u32) -> io::Result<bool> {
        let (fs, geo) = volume()?;
        self.slices += 1;
        let time_left = || (until.wrapping_sub(uptime_ms()) as i32) > 0;
        let heap_ok = || crate::state::free_heap() as usize >= config::FATIDX_SLICE_KEEP;
        loop {
            match self.stage {
                Stage::Walk => {
                    let mut scratch = try_buf(config::FATIDX_SCRATCH)?;
                    let mut read = volume_read(fs, true);
                    self.cursor.refresh();
                    let mut w = self.tmp.as_ref().map(|f| BufWriter::with_capacity(512, f));
                    loop {
                        if !time_left() || !heap_ok() {
                            if let Some(w) = w.as_mut() {
                                w.flush()?;
                            }
                            return Ok(false);
                        }
                        let Some(d) = self.cursor.next(&geo, &mut read)? else { break };
                        self.set.add((d.key, d.first));
                        if let Some(w) = w.as_mut() {
                            w.write_all(&fatdir::TMP_DIR)?;
                            w.write_all(&fatdir::tmp_record(d.key, d.first))?;
                        }
                        let mut names = None;
                        if d.gif && self.names.is_some() {
                            self.names_folders += 1;
                            if self.names_folders > NAMES_MAX_FOLDERS {
                                log::warn!("fat index: more than {NAMES_MAX_FOLDERS} folders: no names file");
                                self.names = None;
                            } else if let (Some(n), Ok(name)) = (self.names.as_mut(), std::str::from_utf8(&self.cursor.name[..d.name_len])) {
                                n.begin(&format!("/gifs/{name}"));
                                names = Some(n);
                            }
                        }
                        let seed = fatdir::hash_more(d.key, b"/");
                        let fanout = &mut self.fanout;
                        let mut failed = None;
                        let index = !self.names_only;
                        fatdir::walk(&geo, d.first, &mut read, &mut scratch, &mut |f| {
                            if let Some(n) = names.as_mut() {
                                if let Err(e) = n.name(f.name) {
                                    failed.get_or_insert(e);
                                }
                            }
                            if index && f.prev_rel.is_none() {
                                let h = fatdir::hash_more(seed, f.name);
                                if let Some(w) = w.as_mut() {
                                    if let Err(e) = w.write_all(&fatdir::tmp_record(h, f.loc)) {
                                        failed.get_or_insert(e);
                                    }
                                }
                                fanout[(h >> 24) as usize + 1] += 1;
                            }
                            Ok(())
                        })?;
                        if let Some(e) = failed {
                            return Err(e);
                        }
                    }
                    // The walk is done.
                    if let Some(mut w) = w.take() {
                        w.flush()?;
                    }
                    drop(w);
                    self.tmp = None;
                    if let Some(n) = self.names.take() {
                        if let Err(e) = n.finish() {
                            log::warn!("fat index: no names file ({e})");
                            self.names_folders = usize::MAX;
                        }
                    } else {
                        self.names_folders = usize::MAX;
                    }
                    if self.names_only {
                        self.stage = Stage::Done;
                        continue;
                    }
                    for b in 0..256 {
                        self.fanout[b + 1] += self.fanout[b];
                    }
                    let h = Header { count: self.fanout[256], n_fatent: geo.n_fatent, database: geo.database, dirs: self.set.len() };
                    let mut out = File::create(NEW)?;
                    out.write_all(&h.encode())?;
                    for v in self.fanout.iter() {
                        out.write_all(&v.to_le_bytes())?;
                    }
                    self.out = Some(out);
                    self.stage = Stage::Passes(0);
                }
                Stage::Passes(b0) => {
                    if b0 >= 256 {
                        self.stage = Stage::Trailer;
                        continue;
                    }
                    if !time_left() || !heap_ok() {
                        return Ok(false);
                    }
                    let mut group = take_group()?;
                    let mut chunk = try_buf(config::FATIDX_SCRATCH)?;
                    let mut f = File::open(TMP)?;
                    let (b1, n) = fatdir::gather(&mut |b| f.read(b), &self.fanout, b0, &mut group, &mut chunk)?;
                    self.out.as_mut().ok_or_else(|| errno(sys::EIO))?.write_all(&group[..n])?;
                    BUILD_GROUP.fetch_max(group.len() as u32, Relaxed);
                    self.passes += 1;
                    self.stage = Stage::Passes(b1);
                }
                Stage::Trailer => {
                    let mut chunk = try_buf(config::FATIDX_SCRATCH)?;
                    let mut f = File::open(TMP)?;
                    let out = self.out.as_mut().ok_or_else(|| errno(sys::EIO))?;
                    let mut set = fatdir::DirSet::default();
                    let mut pair = [0u8; 8];
                    fatdir::tmp_dirs(&mut |b| f.read(b), &mut chunk, &mut |k, c| {
                        set.add((k, c));
                        pair[..4].copy_from_slice(&k.to_le_bytes());
                        pair[4..].copy_from_slice(&c.to_le_bytes());
                        out.write_all(&pair)
                    })?;
                    if set != self.set {
                        return Err(io::Error::other("the temporary records' directories are not the walk's"));
                    }
                    out.flush()?;
                    self.out = None;
                    self.stage = Stage::Done;
                }
                Stage::Done => return Ok(true),
            }
        }
    }
}

/// A pass's group: as much of [`config::FATIDX_GROUP`] as leaves
/// [`config::FATIDX_SLICE_KEEP`] free (held for one slice, inside the
/// player's quiet window), halved down to [`config::FATIDX_GROUP_MIN`];
/// short of that the build waits.
fn take_group() -> io::Result<Vec<u8>> {
    let free = crate::state::free_heap() as usize;
    let mut n = free
        .saturating_sub(config::FATIDX_SLICE_KEEP + config::FATIDX_SCRATCH)
        .min(crate::state::largest_free_block().saturating_sub(1024))
        .min(config::FATIDX_GROUP)
        & !7;
    while n >= config::FATIDX_GROUP_MIN {
        if let Ok(v) = try_buf(n) {
            BUILD_FLOOR.fetch_min(crate::state::free_heap().saturating_sub(config::FATIDX_SCRATCH as u32), Relaxed);
            return Ok(v);
        }
        n = (n / 2) & !7;
    }
    Err(heap_short())
}

/// One slice of the build under way, if the player is quiet for long enough
/// and nothing the build must not run beside is up; the job finished or
/// dropped as it ends.
fn run_slice() {
    let now = uptime_ms();
    let quiet = QUIET_UNTIL.load(Relaxed);
    let until = if (quiet.wrapping_sub(now) as i32) > 0 { quiet.min(now + config::FATIDX_SLICE_MS) } else { now };
    if until.wrapping_sub(now) < config::FATIDX_SLICE_MIN_MS
        || crate::cache_builder::task_up()
        || crate::cache_builder::transfer_in_flight()
        || (crate::state::free_heap() as usize) < config::FATIDX_SLICE_KEEP
    {
        return;
    }
    let mut g = JOB.lock().unwrap_or_else(|p| p.into_inner());
    let Some(job) = g.as_mut() else { return };
    guard(0).store(GUARD_MAGIC, Relaxed);
    let r = job.slice(until);
    guard(0).store(0, Relaxed);
    match r {
        Ok(false) => {}
        Err(e) if e.kind() == io::ErrorKind::OutOfMemory => {}
        Ok(true) => {
            let job = g.take().unwrap();
            drop(g);
            finish(job);
        }
        Err(e) => {
            let job = g.take().unwrap();
            drop(g);
            log::warn!("fat index: {e}");
            abort(job, true);
        }
    }
}

/// Drop a build: its files closed, then deleted (never a write handle
/// closed after its file went: its sync would write the entry back);
/// `failed`: the next waits the gap.
fn abort(job: Box<Job>, failed: bool) {
    let names_only = job.names_only;
    drop(job);
    let _ = fs::remove_file(TMP);
    let _ = fs::remove_file(NEW);
    let _ = fs::remove_file(NAMES_NEW);
    BUILDING.store(false, Relaxed);
    if names_only {
        // Not again this boot: names=1 lists instead.
        NAMES_WANTED.store(false, Relaxed);
        return;
    }
    DIRTY.store(true, Relaxed);
    FAILED.store(failed, Relaxed);
    BUILDS.fetch_add(1, Relaxed);
}

/// Drop the build under way (the card is going, a `/gifs` directory came or
/// went): rebuilt when due, not counted as failed.
fn abort_job() {
    let job = JOB.lock().unwrap_or_else(|p| p.into_inner()).take();
    if let Some(job) = job {
        abort(job, false);
    }
}

/// A build's last step: the new index in place and swapped in.
fn finish(job: Box<Job>) {
    let names_ok = job.names_folders != usize::MAX;
    let names_only = job.names_only;
    let (passes, slices, started, cut0, set) = (job.passes, job.slices, job.started, job.cut0, job.set);
    drop(job);
    if names_only {
        if names_ok && install_names(started) {
            log::info!("fat index: names file written");
        } else {
            NAMES_WANTED.store(false, Relaxed);
        }
        BUILDING.store(false, Relaxed);
        return;
    }
    let _ = fs::remove_file(TMP);
    // The new one is loaded from its own name (with the walk's directories:
    // one changed since would have dropped the build) before the index's
    // lock is taken, and swapped in under it; the old one is closed after
    // (its close takes the volume lock), then the files renamed. The
    // player's lookups never find the lock held for longer than the swap.
    let fresh = load_as(NEW, Some(set));
    let ok = fresh.is_ok();
    if let Err(e) = &fresh {
        log::warn!("fat index: the new index did not load ({e})");
    }
    BUILT_THIS_BOOT.store(ok, Relaxed);
    let old = LOADED.lock().unwrap().replace(fresh.ok());
    LOADED_GEN.fetch_add(1, Relaxed);
    drop(old);
    let _ = fs::remove_file(INDEX);
    let renamed = fs::rename(NEW, INDEX).is_ok();
    if names_ok {
        install_names(BUILT_MS.load(Relaxed));
    } else {
        let _ = fs::remove_file(NAMES_NEW);
    }
    // What the walks saw is in the index now; changes noted since its
    // start stay in the delta.
    let built = BUILT_MS.load(Relaxed);
    {
        let mut d = DELTA.lock().unwrap_or_else(|p| p.into_inner());
        d.retain(|(_, _, at)| since(*at, built) as i32 >= 0);
        DELTA_LEN.store(d.len() as u32, Relaxed);
    }
    FAILED.store(!(ok && renamed), Relaxed);
    if ok && renamed {
        // A good build: the crashes before it no longer count.
        guard(1).store(crashes_word(0), Relaxed);
        AFTER_CRASH.store(0, Relaxed);
    } else {
        DIRTY.store(true, Relaxed);
    }
    BUILDS.fetch_add(1, Relaxed);
    BUILDING.store(false, Relaxed);
    // SAFETY: this task's own stack high-water mark (the network worker's).
    let unused = unsafe { sys::uxTaskGetStackHighWaterMark(std::ptr::null_mut()) };
    log::info!(
        "fat index: built {} records ({} directories) in one walk and {passes} passes, largest group {} B, {slices} slices, {} s; heap floor {} B during the build; transitions cut during it {}; {} B of the worker's stack unused; stale {}",
        RECORDS.load(Relaxed),
        set.len(),
        BUILD_GROUP.load(Relaxed),
        since(uptime_ms(), started) / 1000,
        BUILD_FLOOR.load(Relaxed),
        crate::TRANSITIONS_CUT.load(Relaxed).wrapping_sub(cut0),
        unused,
        DIRTY.load(Relaxed)
    );
}

/// The whole build now, on the caller's task, quiet or not (psi-probe).
#[cfg(feature = "psi-probe")]
fn build() -> io::Result<()> {
    abort_job();
    BUILT_MS.store(uptime_ms(), Relaxed);
    let mut job = Job::new(false)?;
    loop {
        match job.slice(uptime_ms().wrapping_add(1 << 30)) {
            Ok(true) => break,
            Ok(false) => continue,
            Err(e) => {
                abort(job, true);
                return Err(e);
            }
        }
    }
    finish(job);
    Ok(())
}

// ---- no crash loop -----------------------------------------------------------

/// A build slice in flight, and the builds in a row that ended in a crash,
/// kept across a restart in RTC slow memory (`.rtc_noinit`, beside
/// `restarts`' ring; plain word loads and stores only). [`GUARD_MAGIC`] in
/// the first word while a slice runs; the count in the second with its
/// complement.
#[link_section = ".rtc_noinit"]
static mut GUARD: [u32; 2] = [0; 2];
const GUARD_MAGIC: u32 = 0x4644_5842;
/// The builds in a row that crashed the board, as this boot found them.
static AFTER_CRASH: AtomicU32 = AtomicU32::new(0);

fn guard(i: usize) -> &'static AtomicU32 {
    // SAFETY: a word of `GUARD`, only ever accessed through this view
    // (loads and stores).
    unsafe { AtomicU32::from_ptr((&raw mut GUARD).cast::<u32>().add(i)) }
}

fn crashes_word(n: u32) -> u32 {
    (n & 0xFFFF) | (!n & 0xFFFF) << 16
}

fn crashes_stored() -> u32 {
    let w = guard(1).load(Relaxed);
    if w >> 16 == !w & 0xFFFF { w & 0xFFFF } else { 0 }
}

/// First thing at boot (`main`, after `restarts::boot`): a build slice that
/// was in flight when the board crashed counts against the next build; a
/// power-on (RTC memory lost) starts again from none. A planned restart
/// mid-build (an update, a reboot) counts nothing.
pub fn boot(crashed: bool, power_on: bool) {
    let in_flight = guard(0).load(Relaxed) == GUARD_MAGIC;
    let mut n = if power_on { 0 } else { crashes_stored() };
    if in_flight && crashed {
        n += 1;
    }
    guard(0).store(0, Relaxed);
    guard(1).store(crashes_word(n), Relaxed);
    AFTER_CRASH.store(n, Relaxed);
    if n >= config::FATIDX_CRASHES_OFF {
        log::warn!("fat index: {n} builds in a row ended in a crash: none this boot (until a power-on); GIFs open by path");
    } else if n > 0 {
        log::warn!(
            "fat index: the last build ended in a crash ({n} in a row): the next waits {} min after boot; GIFs open by path meanwhile",
            after_crash_wait_s(n) / 60
        );
    }
}

fn after_crash_wait_s(n: u32) -> u32 {
    config::FATIDX_AFTER_CRASH_S << (n.max(1) - 1).min(4)
}

/// May a build start now, after the crashes [`boot`] found?
fn crash_allows(now_ms: u32) -> bool {
    let n = AFTER_CRASH.load(Relaxed);
    n == 0 || (n < config::FATIDX_CRASHES_OFF && now_ms / 1000 >= after_crash_wait_s(n))
}

/// `/api/status`'s `"sd"` part: `,"fat_index":{...}`.
pub fn status_json() -> String {
    format!(
        r#","fat_index":{{"files":{},"hits":{},"misses":{},"builds":{},"stale":{},"raw_open":{},"raw_reads":{},"raw_fallbacks":{},"slots_full":{},"intent_refused":{},"delta":{},"path_opens":{},"path_open_max_us":{},"index_busy":{},"index_unloaded":{},"absent":{},"catalog_fatfs":{},"building":{},"build_floor":{},"build_group":{},"after_crash":{}}}"#,
        RECORDS.load(Relaxed),
        HITS.load(Relaxed),
        MISSES.load(Relaxed),
        BUILDS.load(Relaxed),
        DIRTY.load(Relaxed),
        open_set(|o| o.len()),
        RAW_READS.load(Relaxed),
        RAW_FALLBACKS.load(Relaxed),
        SLOTS_FULL.load(Relaxed),
        INTENT_REFUSED.load(Relaxed),
        DELTA_LEN.load(Relaxed),
        PATH_OPENS.load(Relaxed),
        PATH_OPEN_MAX_US.load(Relaxed),
        INDEX_BUSY.load(Relaxed),
        INDEX_UNLOADED.load(Relaxed),
        ABSENT.load(Relaxed),
        CATALOG_FATFS.load(Relaxed),
        job_running(),
        BUILD_FLOOR.load(Relaxed),
        BUILD_GROUP.load(Relaxed),
        AFTER_CRASH.load(Relaxed),
    )
}

/// psi-probe: every `/gifs/<folder>`'s names from the names file (after
/// [`raw_self_check`]'s build) against the folder's listing: (folders that
/// match, folders that differ, names in all).
#[cfg(feature = "psi-probe")]
#[inline(never)]
pub fn names_self_check(root: &str) -> (usize, usize, usize) {
    let (mut ok, mut bad, mut total) = (0, 0, 0);
    let Ok(dirs) = fs::read_dir(format!("{root}/gifs")) else { return (0, 0, 0) };
    for d in dirs.flatten() {
        if !d.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        let name = d.file_name().to_string_lossy().into_owned();
        let card = format!("/gifs/{name}");
        let mut want: Vec<String> = fs::read_dir(d.path())
            .map(|r| r.flatten().filter(|e| e.file_type().is_ok_and(|t| t.is_file())).map(|e| e.file_name().to_string_lossy().into_owned()).collect())
            .unwrap_or_default();
        let mut got = Vec::new();
        let read = names(&card).map(|n| n.each(&mut |s| {
            got.push(s.to_string());
            Ok(())
        }));
        want.sort();
        got.sort();
        total += got.len();
        if matches!(read, Some(Ok(()))) && got == want {
            ok += 1;
        } else {
            bad += 1;
            log::warn!("names {card}: {} from the names file, {} listed", got.len(), want.len());
        }
    }
    (ok, bad, total)
}

/// psi-probe: the read-ahead (`readahead`) serving the main task's raw
/// reads of the playlist's largest GIF, read as the player
/// reads one (2 KB at a time, a frame apart, up to 96 KB), byte for byte
/// against FatFs's reads of the same file.
#[cfg(feature = "psi-probe")]
pub struct ReadaheadCheck {
    /// The filling task still up after the idle time.
    pub running: bool,
    pub bytes: usize,
    pub same: bool,
    pub hits: u32,
    pub underruns: u32,
    pub fetches: u32,
    /// The heap the read-ahead took while it was held: the filling task's
    /// stack, its 8 KB ring, the 2 KB block being read, the file's extents.
    pub ring_heap: u32,
    /// The heap after its 5 s idle, against before the files were opened
    /// (the task's stack is freed by FreeRTOS's idle task a while later:
    /// a second check starts within 200 B of the first).
    pub left_heap: i64,
    /// Free heap before the read.
    pub heap: u32,
}

#[cfg(feature = "psi-probe")]
#[inline(never)]
pub fn readahead_self_check(root: &str) -> Option<ReadaheadCheck> {
    let list = fs::read_to_string(format!("{root}/filelist.txt")).ok()?;
    let path = list
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with("/gifs/"))
        .map(|l| format!("{root}{l}"))
        .max_by_key(|p| fs::metadata(p).map(|m| m.len()).unwrap_or(0))?;
    let (_, h0, u0, f0) = crate::readahead::probe_counts();
    let heap0 = crate::state::free_heap();
    let mut raw = open_path_raw(&path).ok()??;
    let mut fat = File::open(&path).ok()?;
    let (mut a, mut b) = (vec![0u8; 2048], vec![0u8; 2048]);
    // With both files open and the buffers taken: what the read-ahead takes
    // is measured from here.
    let heap1 = crate::state::free_heap();
    let (mut same, mut bytes, mut low) = (true, 0usize, u32::MAX);
    while bytes < 96 * 1024 {
        let n = raw.read(&mut a).ok()?;
        if n == 0 {
            break;
        }
        if fat.read_exact(&mut b[..n]).is_err() || a[..n] != b[..n] {
            same = false;
            break;
        }
        bytes += n;
        // (The network worker's turn, which starts the filling task.)
        crate::readahead::tick();
        if crate::readahead::probe_counts().0 {
            low = low.min(crate::state::free_heap());
        }
        std::thread::sleep(Duration::from_millis(30));
    }
    drop(raw);
    drop(fat);
    // Its idle time out: the task and the ring gone.
    std::thread::sleep(Duration::from_millis(5_300));
    let left_heap = heap0 as i64 - crate::state::free_heap() as i64;
    let (_, h1, u1, f1) = crate::readahead::probe_counts();
    Some(ReadaheadCheck {
        running: crate::readahead::probe_running(),
        bytes,
        same,
        hits: h1 - h0,
        underruns: u1 - u0,
        fetches: f1 - f0,
        left_heap,
        heap: heap0,
        // (The file's open, its FIL and raw state, is in `heap0`.)
        ring_heap: if low == u32::MAX { 0 } else { heap1.saturating_sub(low) },
    })
}

/// psi-probe (the emulator lane): read up to `max` of the playlist's GIFs
/// and their prepared copies raw ([`open_player`]'s path open, chunks of
/// odd sizes and a seek) and through FatFs, and compare. (files read raw,
/// files that differ, files FatFs read but raw could not open, of the
/// first those opened through the index's `open_at`.)
#[cfg(feature = "psi-probe")]
#[inline(never)]
pub fn raw_self_check(root: &str, max: usize) -> (usize, usize, usize, usize) {
    if !register_vfs() {
        return (0, 0, 0, 0);
    }
    let list = fs::read_to_string(format!("{root}/filelist.txt")).unwrap_or_default();
    let mut paths = Vec::new();
    for line in list.lines().map(str::trim).filter(|l| l.starts_with("/gifs/")).take(max) {
        paths.push(format!("{root}{line}"));
        if let Some(c) = opendmd_core::layout::cache_path(line) {
            paths.push(format!("{root}{c}"));
        }
    }
    // The index route too (`open_at`): built now if the card has none.
    let indexed_ok = { let _ = build(); true };
    let (mut ok, mut bad, mut no_raw) = (0, 0, 0);
    let mut via_index = 0;
    let mut tries = Vec::new();
    for full in paths {
        tries.push((full.clone(), false));
        if indexed_ok && (full.starts_with(&format!("{root}/gifs/")) || full.starts_with(&format!("{root}/cache/"))) {
            tries.push((full, true));
        }
    }
    for (full, index) in tries {
        let Ok(want) = fs::read(&full) else { continue };
        let opened = if index {
            match indexed_now(&full, true) {
                Some(Ok(f)) => {
                    via_index += 1;
                    Ok(Some(f))
                }
                _ => Ok(None),
            }
        } else {
            open_path_raw(&full)
        };
        let mut f = match opened {
            Ok(Some(f)) => f,
            _ => {
                no_raw += 1;
                continue;
            }
        };
        let mut got = Vec::with_capacity(want.len());
        let sizes = [1usize, 700, 512, 3000, 513, 64];
        let mut buf = vec![0u8; 3000];
        let mut k = 0;
        let read_ok = loop {
            let n = sizes[k % sizes.len()];
            k += 1;
            match f.read(&mut buf[..n]) {
                Ok(0) => break true,
                Ok(m) => got.extend_from_slice(&buf[..m]),
                Err(_) => break false,
            }
        };
        let third = want.len() / 3;
        let mut mid = vec![0u8; 1500.min(want.len() - third)];
        let seek_ok = f.seek(SeekFrom::Start(third as u64)).is_ok() && f.read_exact(&mut mid).is_ok() && mid[..] == want[third..third + mid.len()];
        if read_ok && got == want && seek_ok {
            ok += 1;
        } else {
            log::warn!("fat index: raw read of {full} differs from FatFs's");
            bad += 1;
        }
    }
    (ok, bad, no_raw, via_index)
}

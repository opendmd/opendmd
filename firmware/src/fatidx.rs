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
/// Files opened through the index at once (the player's two, the server's,
/// and room).
const SLOTS: usize = 6;
const SLOT_PATHS: [&str; SLOTS] = ["/fidx/0", "/fidx/1", "/fidx/2", "/fidx/3", "/fidx/4", "/fidx/5"];


/// A slot's raw reads: its extents and position, the drive, and whether a
/// raw read failed (FatFs's `f_read` from then on).
struct Raw {
    file: fatread::RawFile,
    pdrv: u8,
    broken: bool,
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
}

/// `full` was removed (or renamed from): lookups find no record for it.
pub fn note_gone(full: &str) {
    if let Some(card) = full.strip_prefix(config::SD_MOUNT).filter(|c| c.starts_with("/gifs/") || c.starts_with("/cache/")) {
        push_delta(fatdir::hash(card.as_bytes()), None);
    }
}

fn push_delta(h: u32, loc: Option<u32>) {
    let mut d = DELTA.lock().unwrap_or_else(|p| p.into_inner());
    d.retain(|(k, _, _)| *k != h);
    d.push((h, loc, uptime_ms()));
    if d.len() > DELTA_MAX {
        d.remove(0);
        // Too much since the build: rebuild when quiet.
        TOUCHED_MS.store(uptime_ms(), Relaxed);
        DIRTY.store(true, Relaxed);
    }
    DELTA_LEN.store(d.len() as u32, Relaxed);
}
static DELTA_LEN: AtomicU32 = AtomicU32::new(0);

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
        match raw.file.read(buf, &mut dev) {
            Ok(n) => {
                RAW_READS.fetch_add(1, Relaxed);
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

unsafe extern "C" fn v_close(fd: i32) -> i32 {
    // The raw state and the open-set entry go before the slot is free
    // (`fatread::Slots::release`): another task's open can then never find
    // this file's extents in it.
    match TABLE.release(fd as usize, |k| open_set(|o| o.remove(k))) {
        Some((f, _raw)) => {
            sys::f_close(f);
            drop(Box::from_raw(f));
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
    if let Some(r) = indexed_as(full, true) {
        return r;
    }
    if crate::CARD_READY.load(Relaxed) && VFS_UP.load(Relaxed) {
        if let Some(f) = open_path_raw(full)? {
            return Ok(f);
        }
    }
    File::open(full)
}

/// By path into a FIL of ours (FatFs's `f_open`: the directory walk the
/// path open does), its chain read, read raw. `Ok(None)`: no free slot,
/// or the volume is not FAT32.
fn open_path_raw(full: &str) -> io::Result<Option<File>> {
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
        Ok(ext) if fatread::is_canonical(card) => Some(Raw { file: fatread::RawFile::new(ext), pdrv, broken: false }),
        Ok(_) => None,
        // As in `open_at`: FatFs reads this one.
        Err(e) => {
            log::warn!("fat index: {full}: no raw reads ({e}); FatFs reads it");
            None
        }
    };
    hand_out(f, Some(full), raw, None)
}

/// Put an open FIL in a free slot and open it there as a `File`; with
/// `raw`, read raw and noted in [`holds`]. `Ok(None)`: every slot busy
/// (the FIL is freed; the caller opens by path).
fn hand_out(f: *mut sys::FIL, player: Option<&str>, raw: Option<Raw>, since: Option<u32>) -> io::Result<Option<File>> {
    // A player's open is noted in the open set under its lock unless a
    // write intent covers the path, the check and the note one step; then
    // it is not opened at all (`fatread::OpenSet`).
    let mut note = |k: usize| {
        player.is_some_and(|full| {
            open_set(|o| match since {
                Some(seq) => o.insert_unless_covered_since(k, full, seq),
                None => o.insert_unless_covered(k, full),
            })
        })
    };
    let claimed = TABLE.claim(f, raw.map(Box::new), player.map(|_| &mut note as &mut dyn FnMut(usize) -> bool));
    let k = match claimed {
        Ok(k) => k,
        Err(why) => {
            // SAFETY: never handed out.
            unsafe {
                sys::f_close(f);
                drop(Box::from_raw(f));
            }
            return match (why, player) {
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
                unsafe {
                    sys::f_close(f);
                    drop(Box::from_raw(f));
                }
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
    // A file written, renamed or removed since the build: the delta knows.
    let delta = DELTA.try_lock().ok().and_then(|d| d.iter().rev().find(|(k, _, _)| *k == h).map(|(_, l, _)| *l));
    if let Some(loc) = delta {
        let loc = loc?;
        cands[0] = loc;
        nc = 1;
    } else {
        // Never wait for the index (a build swapping it in, or another
        // task's lookup): this may be the display task. Busy means a path open.
        let Ok(mut g) = LOADED.try_lock() else {
            return None;
        };
        if g.is_none() {
            *g = Some(load().ok());
        }
        let l = g.as_mut().unwrap().as_mut()?;
        let b = (h >> 24) as usize;
        // A bucket holds ~1/256 of the records (~40 on a stock card); past
        // 256 (2 KB) the rest of it is not looked at, and those names open
        // by path.
        let (lo, hi) = (l.fanout[b], l.fanout[b + 1].min(l.fanout[b] + 256));
        let mut recs = vec![0u8; (hi - lo) as usize * 8];
        let at = fatdir::RECORDS_AT + lo as u64 * 8;
        let got = match l.raw.as_mut() {
            // Raw: the card's I/O lock only.
            Some((rf, pdrv)) => {
                let pdrv = *pdrv;
                rf.seek(at);
                let mut dev = |lba: u32, n: u32, b: &mut [u8]| -> io::Result<()> {
                    // SAFETY: `b` is `n` sectors; the mounted card's drive.
                    if unsafe { sys::ff_disk_read(pdrv, b.as_mut_ptr(), lba, n) } != 0 {
                        return Err(errno(sys::EIO));
                    }
                    Ok(())
                };
                let mut done = 0;
                let mut r = Ok(());
                while done < recs.len() {
                    match rf.read(&mut recs[done..], &mut dev) {
                        Ok(0) => {
                            r = Err(io::Error::from(io::ErrorKind::UnexpectedEof));
                            break;
                        }
                        Ok(n) => done += n,
                        Err(e) => {
                            r = Err(e);
                            break;
                        }
                    }
                }
                r
            }
            None => l.file.seek(SeekFrom::Start(at)).and_then(|_| l.file.read_exact(&mut recs)),
        };
        if let Err(e) = got {
            if crate::sdhealth::is_card_error(&e) {
                return Some(Err(e));
            }
            // Anything else (a short or damaged index): drop it, open by path.
            *g = Some(None);
            DIRTY.store(true, Relaxed);
            return None;
        }
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
            Err(e) => return Some(Err(e)),
        }
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
            Ok(ext) if fatread::is_canonical(full) => Some(Raw { file: fatread::RawFile::new(ext), pdrv: unsafe { (*fs).pdrv }, broken: false }),
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

fn load() -> io::Result<Loaded> {
    let mut file = File::open(INDEX)?;
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
    let mut dirs = Vec::with_capacity(h.dirs as usize);
    for _ in 0..h.dirs.min(8192) {
        file.read_exact(&mut e)?;
        dirs.push(pair(&e));
    }
    let now: Vec<(u32, u32)> = indexed_dirs()?.iter().map(|(d, c)| (fatdir::hash(d.as_bytes()), *c)).collect();
    if !fatdir::dirs_match(&dirs, &now) {
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
    // Its extents, for the player's lookups without the volume lock.
    let raw = index_extents();
    Ok(Loaded { file, fanout, raw })
}

/// The index file's extents (its own FIL's first cluster and size, its
/// chain under the volume lock): `None` if anything does not add up.
fn index_extents() -> Option<(fatread::RawFile, u8)> {
    let (fs, geo) = volume().ok()?;
    let path = std::ffi::CString::new(format!("0:{}", INDEX.strip_prefix(config::SD_MOUNT)?)).ok()?;
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
    FS.store(std::ptr::null_mut(), Relaxed);
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
    TOUCHED_MS.store(uptime_ms(), Relaxed);
    DIRTY.store(true, Relaxed);
    *LOADED.lock().unwrap() = Some(None);
}

/// The network worker's turn: rebuild when wanted, quiet and affordable.
pub fn tick() {
    if !crate::CARD_READY.load(Relaxed) || !register_vfs() {
        return;
    }
    // The first look at the card decides whether there is an index at all.
    let missing = {
        let mut g = LOADED.lock().unwrap();
        if RELOAD.swap(false, Relaxed) {
            *g = None;
            DIRTY.store(true, Relaxed);
        }
        if g.is_none() {
            *g = Some(load().ok());
        }
        g.as_ref().is_some_and(Option::is_none)
    };
    if !DIRTY.load(Relaxed) {
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
        || (crate::state::free_heap() as usize) < config::FATIDX_BUILD_HEAP
        || crate::state::largest_free_block() < 24 * 1024
    {
        return;
    }
    if BUILDING.swap(true, Relaxed) {
        return;
    }
    BUILT_MS.store(now, Relaxed);
    // Its own short-lived task (CPU0, the cache builder's priority): the
    // walk's FatFs/SD calls are deep, and the network worker's stack is
    // sized for its own work; and the worker keeps polling meanwhile.
    let spawned = crate::task::spawn_core0(
        b"fat-index\0",
        config::FATIDX_STACK,
        config::BUILDER_PRIORITY,
        Box::new(|| {
            let r = build();
            FAILED.store(r.is_err(), Relaxed);
            if let Err(e) = r {
                log::warn!("fat index: {e}");
                // Walked again only after the gap.
                DIRTY.store(true, Relaxed);
            }
            BUILDS.fetch_add(1, Relaxed);
            BUILDING.store(false, Relaxed);
        }),
    );
    if spawned.is_err() {
        // No task (no heap for its stack): wait out the gap, not a retry
        // every turn.
        FAILED.store(true, Relaxed);
        BUILDING.store(false, Relaxed);
    }
}

/// Every `/gifs/<dir>`: its card path and first cluster.
/// Every directory the index covers: `/gifs/<dir>` and the prepared copies'
/// buckets, `/cache/<category>/<bucket>` (what plays is mostly copies).
fn indexed_dirs() -> io::Result<Vec<(String, u32)>> {
    let mut dirs = gif_dirs()?;
    let root = format!("{}/cache", config::SD_MOUNT);
    let Ok(cats) = fs::read_dir(&root) else { return Ok(dirs) };
    for c in cats.flatten() {
        let cat = c.file_name().to_string_lossy().into_owned();
        if cat.starts_with('.') || !c.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        let Ok(buckets) = fs::read_dir(format!("{root}/{cat}")) else { continue };
        for b in buckets.flatten() {
            if !b.file_type().is_ok_and(|t| t.is_dir()) {
                continue;
            }
            let dir = format!("/cache/{cat}/{}", b.file_name().to_string_lossy());
            if let Ok((_, first)) = open_dir(&dir) {
                dirs.push((dir, first));
            }
        }
    }
    Ok(dirs)
}

fn gif_dirs() -> io::Result<Vec<(String, u32)>> {
    let mut dirs = Vec::new();
    for e in fs::read_dir(format!("{}/gifs", config::SD_MOUNT))? {
        let e = e?;
        if e.file_type()?.is_dir() {
            let dir = format!("/gifs/{}", e.file_name().to_string_lossy());
            let first = open_dir(&dir)?.1;
            dirs.push((dir, first));
        }
    }
    Ok(dirs)
}

fn build() -> io::Result<()> {
    let (fs, geo) = volume()?;
    // Cleared first: a change during the walks marks it again.
    DIRTY.store(false, Relaxed);
    let dirs = indexed_dirs()?;
    let listed: Vec<(u32, u32)> = dirs.iter().map(|(d, c)| (fatdir::hash(d.as_bytes()), *c)).collect();
    let _ = fs::create_dir(config::CACHE_ROOT);
    let mut scratch = vec![0u8; 8 * SS];
    let mut read = |lba: u32, buf: &mut [u8]| -> io::Result<()> {
        {
            let _l = VolLock::take(fs)?;
            read_locked(fs, lba, buf)?;
        }
        // Between batches: the player's reads go first.
        std::thread::sleep(Duration::from_millis(1));
        Ok(())
    };
    let mut walk_all = |emit: &mut dyn FnMut(u32, u32)| -> io::Result<()> {
        for (dir, first) in &dirs {
            let first = *first;
            let seed = fatdir::hash_more(fatdir::hash(dir.as_bytes()), b"/");
            fatdir::walk(&geo, first, &mut read, &mut scratch, &mut |f| {
                if f.prev_rel.is_none() {
                    emit(fatdir::hash_more(seed, f.name), f.loc);
                }
                Ok(())
            })?;
        }
        Ok(())
    };
    // The records gathered per walk: up to 16 KB (2,048 records), less if
    // the largest block is short. On the rig a 32 KB group took the sampled
    // floor to 19 KB during a build; 16 KB costs two or three more walks.
    let n = crate::state::largest_free_block().saturating_sub(16 * 1024).clamp(4 * 1024, config::FATIDX_GROUP);
    let mut group = vec![0u8; n];
    let mut out = BufWriter::with_capacity(1024, File::create(NEW)?);
    fatdir::build_index(&mut |b| out.write_all(b), &geo, &mut walk_all, &listed, &mut group)?;
    out.flush()?;
    drop(out);
    // Nothing reopens the old index between its removal and the rename,
    // and the new one is loaded here, on the build's task, not by the next
    // open (which may be the display task's).
    let mut g = LOADED.lock().unwrap();
    *g = None;
    let _ = fs::remove_file(INDEX);
    fs::rename(NEW, INDEX)?;
    *g = Some(load().ok());
    // What the walks saw is in the index now; changes noted since its
    // start stay in the delta.
    let started = BUILT_MS.load(Relaxed);
    let mut d = DELTA.lock().unwrap_or_else(|p| p.into_inner());
    d.retain(|(_, _, at)| since(*at, started) as i32 >= 0);
    DELTA_LEN.store(d.len() as u32, Relaxed);
    Ok(())
}

/// `/api/status`'s `"sd"` part: `,"fat_index":{...}`.
pub fn status_json() -> String {
    format!(
        r#","fat_index":{{"files":{},"hits":{},"misses":{},"builds":{},"stale":{},"raw_open":{},"raw_reads":{},"raw_fallbacks":{},"slots_full":{},"intent_refused":{},"delta":{}}}"#,
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
    )
}

/// psi-probe (the emulator lane): read up to `max` of the playlist's GIFs
/// and their prepared copies raw ([`open_player`]'s path open, chunks of
/// odd sizes and a seek) and through FatFs, and compare. (files read raw,
/// files that differ, files FatFs read but raw could not open, of the
/// first those opened through the index's `open_at`.)
#[cfg(feature = "psi-probe")]
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
        if indexed_ok && full.starts_with(&format!("{root}/gifs/")) {
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

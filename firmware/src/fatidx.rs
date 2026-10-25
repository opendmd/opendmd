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
//! The index is rebuilt on the network worker (CPU0) when it is missing,
//! after the API changed something under `/gifs` (30 s quiet first), or when
//! opens keep missing it (a card edited on a PC): never with a transfer in
//! flight or the cache builder's task up, never without the heap.

use esp_idf_sys as sys;
use opendmd_core::fatdir::{self, Geometry, Header, Verify, SS};
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

static FILES: [AtomicPtr<sys::FIL>; SLOTS] = [const { AtomicPtr::new(std::ptr::null_mut()) }; SLOTS];
static VFS_UP: AtomicBool = AtomicBool::new(false);
/// A rebuild is wanted (no index, the API changed `/gifs`, too many misses).
static DIRTY: AtomicBool = AtomicBool::new(true);
/// Uptime (s) of the last change through the API, and of the last build.
static TOUCHED_S: AtomicU32 = AtomicU32::new(0);
static BUILT_S: AtomicU32 = AtomicU32::new(0);
static HITS: AtomicU32 = AtomicU32::new(0);
static MISSES: AtomicU32 = AtomicU32::new(0);
static RECORDS: AtomicU32 = AtomicU32::new(0);
static BUILDS: AtomicU32 = AtomicU32::new(0);
static BUILD_MS: AtomicU32 = AtomicU32::new(0);
/// A build's thread is up (one at a time); its stack never used, last build.
static BUILDING: AtomicBool = AtomicBool::new(false);
static BUILD_STACK_FREE: AtomicU32 = AtomicU32::new(0);
/// The last build failed: the next waits `FATIDX_MIN_GAP_S` even for a
/// missing index (a failing card must not be walked in a loop).
static FAILED: AtomicBool = AtomicBool::new(false);

struct Loaded {
    file: File,
    fanout: Box<[u32; fatdir::FANOUT]>,
    extras: Vec<(u32, u32)>,
}

/// `Some(None)`: looked, no usable index.
static LOADED: Mutex<Option<Option<Loaded>>> = Mutex::new(None);
static FS: AtomicPtr<sys::FATFS> = AtomicPtr::new(std::ptr::null_mut());

fn uptime_s() -> u32 {
    // SAFETY: a plain IDF call.
    (unsafe { sys::esp_timer_get_time() } / 1_000_000) as u32
}

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

// ---- the VFS: a FIL we filled, as a file descriptor -------------------------

fn fil(fd: i32) -> *mut sys::FIL {
    FILES.get(fd as usize).map_or(std::ptr::null_mut(), |p| p.load(Relaxed))
}

unsafe fn fail(code: u32) -> i32 {
    *sys::__errno() = code as i32;
    -1
}

fn fr_code(r: sys::FRESULT) -> u32 {
    match r {
        sys::FRESULT_FR_TIMEOUT => sys::ETIMEDOUT,
        sys::FRESULT_FR_NOT_READY => sys::ENODEV,
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
    let base = match whence as u32 {
        sys::SEEK_CUR => (*f).fptr as i64,
        sys::SEEK_END => (*f).obj.objsize as i64,
        _ => 0,
    };
    let to = base + off as i64;
    if to < 0 {
        return fail(sys::EINVAL) as sys::off_t;
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
    match FILES.get(fd as usize).map(|p| p.swap(std::ptr::null_mut(), Relaxed)) {
        Some(f) if !f.is_null() => {
            sys::f_close(f);
            drop(Box::from_raw(f));
            0
        }
        _ => fail(sys::EBADF),
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
    let card = full.strip_prefix(config::SD_MOUNT).filter(|c| c.starts_with("/gifs/"))?;
    let name = card.rsplit('/').next()?;
    if !VFS_UP.load(Relaxed) || !name.is_ascii() || name.len() > 255 {
        return None;
    }
    let h = fatdir::hash(card.as_bytes());
    // Candidates (loc, the extra "sector before" if any), copied out so the
    // index lock is not held across the card reads below.
    let mut cands = [(0u32, u32::MAX); 8];
    let mut nc = 0;
    {
        let mut g = LOADED.lock().unwrap();
        if g.is_none() {
            *g = Some(load().ok());
        }
        let l = g.as_mut().unwrap().as_mut()?;
        let b = (h >> 24) as usize;
        let (lo, hi) = (l.fanout[b], l.fanout[b + 1].min(l.fanout[b] + 128));
        let mut recs = vec![0u8; (hi - lo) as usize * 8];
        if let Err(e) = l.file.seek(SeekFrom::Start(fatdir::RECORDS_AT + lo as u64 * 8)).and_then(|_| l.file.read_exact(&mut recs)) {
            return Some(Err(e));
        }
        for r in recs.chunks_exact(8).filter(|r| r[..4] == h.to_le_bytes()).take(cands.len()) {
            let loc = u32::from_le_bytes(r[4..].try_into().unwrap());
            let prev = l.extras.iter().find(|e| e.0 == loc).map_or(u32::MAX, |e| e.1);
            cands[nc] = (loc, prev);
            nc += 1;
        }
    }
    for &(loc, prev) in &cands[..nc] {
        match open_at(loc, prev, name.as_bytes()) {
            Ok(Some(f)) => {
                HITS.fetch_add(1, Relaxed);
                return Some(Ok(f));
            }
            Ok(None) => {}
            Err(e) => return Some(Err(e)),
        }
    }
    if MISSES.fetch_add(1, Relaxed) + 1 >= config::FATIDX_STALE_REBUILD {
        DIRTY.store(true, Relaxed);
    }
    None
}

/// Verify the entry at `loc` and open it (`Ok(None)`: not this file).
fn open_at(loc: u32, prev: u32, name: &[u8]) -> io::Result<Option<File>> {
    let (fs, geo) = volume()?;
    let (rel, slot) = (loc >> 4, (loc & 15) as usize);
    if rel >= (geo.n_fatent - 2) * geo.csize {
        return Ok(None);
    }
    let mut buf = vec![0u8; 2 * SS];
    let (pbuf, cur) = buf.split_at_mut(SS);
    let lock = VolLock::take(fs)?;
    read_locked(fs, geo.database + rel, cur)?;
    let mut v = fatdir::verify(cur, None, slot, name);
    if v == Verify::NeedPrev {
        let p = if !rel.is_multiple_of(geo.csize) { rel - 1 } else { prev };
        if p == u32::MAX {
            return Ok(None);
        }
        read_locked(fs, geo.database + p, pbuf)?;
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
    drop(lock);
    for (slot, path) in FILES.iter().zip(SLOT_PATHS) {
        if slot.compare_exchange(std::ptr::null_mut(), f, Relaxed, Relaxed).is_ok() {
            return match File::open(path) {
                Ok(file) => Ok(Some(file)),
                Err(e) => {
                    slot.store(std::ptr::null_mut(), Relaxed);
                    // SAFETY: never handed out.
                    drop(unsafe { Box::from_raw(f) });
                    Err(e)
                }
            };
        }
    }
    // SAFETY: never handed out; every slot busy, open by path instead.
    drop(unsafe { Box::from_raw(f) });
    Ok(None)
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
    let mut extras = Vec::with_capacity(h.extras as usize);
    file.seek(SeekFrom::Start(fatdir::RECORDS_AT + h.count as u64 * 8))?;
    let mut e = [0u8; 8];
    for _ in 0..h.extras.min(fatdir::MAX_EXTRAS as u32) {
        file.read_exact(&mut e)?;
        extras.push((u32::from_le_bytes(e[..4].try_into().unwrap()), u32::from_le_bytes(e[4..].try_into().unwrap())));
    }
    RECORDS.store(h.count, Relaxed);
    MISSES.store(0, Relaxed);
    if TOUCHED_S.load(Relaxed) <= BUILT_S.load(Relaxed) {
        DIRTY.store(false, Relaxed);
    }
    Ok(Loaded { file, fanout, extras })
}

// ---- keeping it current ------------------------------------------------------

/// The API changed `path` (an upload, move or delete): rebuild once things
/// are quiet. Only `/gifs` is indexed.
pub fn touched(path: &str) {
    if path.strip_prefix(config::SD_MOUNT).unwrap_or(path).starts_with("/gifs/") {
        TOUCHED_S.store(uptime_s().max(1), Relaxed);
        DIRTY.store(true, Relaxed);
    }
}

/// The network worker's turn: rebuild when wanted, quiet and affordable.
pub fn tick() {
    if !register_vfs() {
        return;
    }
    // The first look at the card decides whether there is an index at all.
    let missing = {
        let mut g = LOADED.lock().unwrap();
        if g.is_none() {
            *g = Some(load().ok());
        }
        g.as_ref().is_some_and(Option::is_none)
    };
    if !DIRTY.load(Relaxed) {
        return;
    }
    let now = uptime_s();
    let quiet = now.saturating_sub(TOUCHED_S.load(Relaxed)) >= config::FATIDX_QUIET_S;
    // A missing index is built at once; a stale one waits its turn (a card
    // whose names cannot all be indexed must not rebuild in a loop).
    let spaced = (missing && !FAILED.load(Relaxed)) || now.saturating_sub(BUILT_S.load(Relaxed)) >= config::FATIDX_MIN_GAP_S;
    if !quiet
        || !spaced
        || now < config::FATIDX_BOOT_DELAY_S
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
    BUILT_S.store(now, Relaxed);
    // Its own short-lived task (CPU0, the cache builder's priority): the
    // walk's FatFs/SD calls are deep, and the network worker's stack is
    // sized for its own work; and the worker keeps polling meanwhile.
    use esp_idf_hal::task::thread::ThreadSpawnConfiguration;
    let previous = ThreadSpawnConfiguration::get();
    let _ = ThreadSpawnConfiguration {
        name: Some(b"fat-index\0"),
        stack_size: config::FATIDX_STACK,
        priority: config::BUILDER_PRIORITY,
        pin_to_core: Some(esp_idf_hal::cpu::Core::Core0),
        ..Default::default()
    }
    .set();
    let spawned = std::thread::Builder::new().stack_size(config::FATIDX_STACK).spawn(|| {
        let t = std::time::Instant::now();
        let r = build();
        FAILED.store(r.is_err(), Relaxed);
        if let Err(e) = r {
            log::warn!("fat index: {e}");
            // Walked again only after the gap.
            DIRTY.store(true, Relaxed);
        }
        BUILD_MS.store(t.elapsed().as_millis() as u32, Relaxed);
        BUILDS.fetch_add(1, Relaxed);
        // SAFETY: the calling task's own high-water mark.
        BUILD_STACK_FREE.store(unsafe { sys::uxTaskGetStackHighWaterMark(std::ptr::null_mut()) }, Relaxed);
        BUILDING.store(false, Relaxed);
    });
    let _ = previous.unwrap_or_default().set();
    if spawned.is_err() {
        BUILDING.store(false, Relaxed);
    }
}

fn build() -> io::Result<()> {
    let (fs, geo) = volume()?;
    // Cleared first: a change during the walks marks it again.
    DIRTY.store(false, Relaxed);
    let mut dirs = Vec::new();
    for e in fs::read_dir(format!("{}/gifs", config::SD_MOUNT))? {
        let e = e?;
        if e.file_type()?.is_dir() {
            dirs.push(format!("/gifs/{}", e.file_name().to_string_lossy()));
        }
    }
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
    let mut walk_all = |emit: &mut dyn FnMut(u32, u32, Option<u32>)| -> io::Result<()> {
        for dir in &dirs {
            let (_, first) = open_dir(dir)?;
            let seed = fatdir::hash_more(fatdir::hash(dir.as_bytes()), b"/");
            fatdir::walk(&geo, first, &mut read, &mut scratch, &mut |f| {
                emit(fatdir::hash_more(seed, f.name), f.loc, f.prev_rel);
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
    fatdir::build_index(&mut |b| out.write_all(b), &geo, &mut walk_all, &mut group)?;
    out.flush()?;
    drop(out);
    // Nothing reopens the old index between its removal and the rename.
    let mut g = LOADED.lock().unwrap();
    *g = None;
    let _ = fs::remove_file(INDEX);
    fs::rename(NEW, INDEX)
}

/// `/api/status`'s `"sd"` part: `,"fat_index":{...}`.
pub fn status_json() -> String {
    format!(
        r#","fat_index":{{"files":{},"hits":{},"misses":{},"builds":{},"build_ms":{},"stack_free":{},"stale":{}}}"#,
        RECORDS.load(Relaxed),
        HITS.load(Relaxed),
        MISSES.load(Relaxed),
        BUILDS.load(Relaxed),
        BUILD_MS.load(Relaxed),
        BUILD_STACK_FREE.load(Relaxed),
        DIRTY.load(Relaxed),
    )
}

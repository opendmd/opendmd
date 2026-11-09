//! The card's Formulas prepared on the display: every `/formulas/<stem>.psi`
//! whose compiled `<stem>.psb` beside it is missing or stale is compiled
//! here, in the background once the card is the player's (the boot task,
//! CPU0, after the handover), so a card filled on a computer plays its
//! Formulas with no web app, and a firmware update that changed the
//! compiled format (`psi::psb::ABI`) does not leave them unplayable. Until
//! one is ready the playlist skips it, and a look or start-up show of it
//! falls back; a look picks it up once it is written
//! (`formula_look::file_changed`, as a save does).
//!
//! **One pass over the directory.** FatFs's `f_readdir` gives each entry's
//! name, size and date at once: every lookup by name is a linear scan of
//! the directory, so nothing is opened or `stat`ed per file except a
//! `.psb` whose header must be read (a mark, below, or every `.psb` when
//! the format may have changed: `/formulas/.psb-abi` records the ABI the
//! last full check ran under, and when it is this player's no header is
//! read at all).
//!
//! **Stale** is: no `.psb`; one older than its `.psi` (FAT's 2 s stamps);
//! one not of this player's format (when checked). Every `.psb` written
//! here is given its `.psi`'s date (`f_utime`, FatFs's own fields: no
//! clock needed, none with WiFi off), so it is current until the source
//! changes. An empty `.psb` not older than its source is the web app's
//! record of a source that does not compile: not tried again until it
//! changes.
//!
//! **No crash loop.** Before a source is compiled its `.psb` is written as
//! a mark ([`MARK`], `PSB` with ABI 0, and the source's size and date): a
//! compile that crashes the board leaves it, and the next start counts the
//! source as one that does not compile, until the source changes. A
//! compile error leaves the mark too. A crash anywhere else in the pass (a
//! count in RTC memory, [`boot`]) stops it after
//! [`config::CARD_COMPILE_CRASHES_OFF`] starts in a row, until a power-on
//! or an update. An unreadable or too long source is marked the same way.
//! A compile's writes: the mark, the result over it, one `f_utime`.
//!
//! **One at a time, within the heap, giving way.** Each compile runs on a
//! task of its own ([`STACK`], `psi_stack::run_with`, CPU0 at the boot
//! task's priority 1, under the network and the httpd) and starts only if
//! the heap has its stack, two programs, `psi::compile_heap_estimate` of
//! the source and [`RESERVE`] free, with a block for the stack; otherwise
//! the network worker tries it again later ([`tick`]: only once the heap
//! has room, [`RETRY_OFFLINE`]/[`RETRY_ONLINE`]), then it is left for the
//! next start. Between files the pass waits for a transfer on the card
//! to end. The bytes are `psi::build`'s: what the web app and `psi
//! build` write.
//!
//! The counts go to the log and to `/api/status` (`card_formulas`).

use opendmd_core::psi;
use std::fs;
use std::io::Read;
use std::sync::atomic::AtomicU32;
use std::sync::atomic::Ordering::{Relaxed, SeqCst};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::config;

/// The compile task's stack. The parser recurses up to `psi::MAX_DEPTH`
/// levels: the deepest of the lane's Formulas nested to the cap (calls,
/// brackets, `?:`, `if` in `draw`, user functions, sprite reads) used
/// ~20 KB of it; `emu-offline` asserts 8 KB left for each.
pub const STACK: usize = 32 * 1024;
/// The heap a compile leaves for everyone else: what the network's
/// transients take while the panel plays, the figure the other CPU0 work
/// on the card leaves too (`config::FATIDX_SLICE_KEEP`), and above the
/// player's own yield lines (`config::AHEAD_YIELD_HEAP`,
/// `config::GLOW_YIELD_HEAP`), so a compile never makes the panel shed its
/// read-ahead or glow.
const RESERVE: usize = 20 * 1024;
/// At most this many `.psi` files are looked at in one pass.
const MAX_FILES: usize = 256;
/// A compile the heap could not hold is tried again after this long, this
/// many times, and only once the heap has risen to what the least of them
/// needs; online (where the web app prepares Formulas too, and the heap is
/// the network's as well) fewer, further apart.
const RETRY_OFFLINE: (Duration, u32) = (Duration::from_secs(30), 10);
const RETRY_ONLINE: (Duration, u32) = (Duration::from_secs(60), 2);
/// After a late frame, compiles wait this long.
const AFTER_LATE: Duration = Duration::from_secs(5);
/// Card reads and writes go in pieces of this size, the CPU yielded
/// between them.
const CHUNK: usize = 2 * 1024;
/// Compiled files the player refused (`engine::set_formula_refused`), to be
/// prepared again: the card's list of them (hashes of their paths).
const REDO_FILE: &str = ".psb-redo";
/// The mark written before a compile: `PSB`, ABI 0, then the source's
/// size and FAT date (date << 16 | time), little-endian.
const MARK: &[u8; 4] = b"PSB\0";
const MARK_LEN: u32 = 12;
/// The ABI the last full check of the card's `.psb` headers ran under.
const ABI_FILE: &str = ".psb-abi";

/// What a mount found and did.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct Counts {
    /// `.psi` files seen.
    pub sources: u32,
    /// Their `.psb` already current.
    pub current: u32,
    /// Compiled now.
    pub prepared: u32,
    /// Do not compile (an empty `.psb` or a mark beside them says so).
    pub failed: u32,
    /// Not compiled: the heap stayed short (tried again at the next start).
    pub short: u32,
    /// Free heap at the lowest point measured around a compile (B; 0: none).
    pub heap_low: u32,
    /// The least of the compile task's stack left unused (B; 0: none).
    pub stack_left: u32,
    /// The least heap a compile that was short needed (B; 0: none).
    pub min_need: u32,
}

static LAST: Mutex<Option<Counts>> = Mutex::new(None);

/// The mount compile in flight (`GUARD_MAGIC` in the first word) and the
/// boots in a row it was in flight at a crash (the second, with its
/// complement), kept across a restart in RTC slow memory (`.rtc_noinit`;
/// plain word loads and stores only), as `fatidx`'s guard.
#[link_section = ".rtc_noinit"]
static mut GUARD: [u32; 2] = [0; 2];
const GUARD_MAGIC: u32 = 0x5053_4243;
/// The crashes in a row this boot found.
static CRASHES: AtomicU32 = AtomicU32::new(0);

fn guard(i: usize) -> &'static AtomicU32 {
    // SAFETY: a word of `GUARD`, only ever accessed through this view
    // (loads and stores).
    unsafe { AtomicU32::from_ptr((&raw mut GUARD).cast::<u32>().add(i)) }
}

fn crashes_word(n: u32) -> u32 {
    (n & 0xFFFF) | (!n & 0xFFFF) << 16
}

/// First thing at boot (`main`, beside `fatidx::boot`): a mount compile in
/// flight when the board crashed counts; a power-on (RTC memory lost)
/// starts again from none.
pub fn boot(crashed: bool, power_on: bool) {
    let w = guard(1).load(Relaxed);
    let stored = if w >> 16 == !w & 0xFFFF { w & 0xFFFF } else { 0 };
    let mut n = if power_on { 0 } else { stored };
    if guard(0).load(Relaxed) == GUARD_MAGIC && crashed {
        n += 1;
    }
    guard(0).store(0, Relaxed);
    guard(1).store(crashes_word(n), Relaxed);
    CRASHES.store(n, Relaxed);
    if n > 0 {
        log::warn!("formulas: the card's Formulas were being prepared when the board crashed ({n} in a row)");
    }
}

/// A new firmware booted (`restarts::booted_after_update`): the crashes
/// counted were the old one's.
pub fn forget_crashes() {
    guard(1).store(crashes_word(0), Relaxed);
}

/// `,"card_formulas":{…}` for `/api/status` (null before a card mounted).
pub fn status_json() -> String {
    match *LAST.lock().unwrap_or_else(|p| p.into_inner()) {
        None => r#","card_formulas":null"#.into(),
        Some(c) => format!(
            r#","card_formulas":{{"sources":{},"current":{},"prepared":{},"failed":{},"short_of_memory":{}}}"#,
            c.sources, c.current, c.prepared, c.failed, c.short
        ),
    }
}

/// A directory entry as `f_readdir` gives it.
struct Entry {
    name: String,
    size: u32,
    /// FAT date << 16 | time: compares as the time does.
    stamp: u32,
    fdate: u16,
    ftime: u16,
}

/// Every file in `card_dir` (card-absolute), in one pass.
fn list(card_dir: &str) -> Vec<Entry> {
    use esp_idf_sys as sys;
    let mut out = Vec::new();
    let Ok(path) = std::ffi::CString::new(format!("0:{card_dir}")) else {
        return out;
    };
    // SAFETY: plain data FatFs fills, the directory closed before return.
    unsafe {
        let mut d: sys::FF_DIR = std::mem::zeroed();
        if sys::f_opendir(&mut d, path.as_ptr()) != sys::FRESULT_FR_OK {
            return out;
        }
        let mut fi: sys::FILINFO = std::mem::zeroed();
        while sys::f_readdir(&mut d, &mut fi) == sys::FRESULT_FR_OK && fi.fname[0] != 0 {
            if fi.fattrib as u32 & sys::AM_DIR == 0 {
                let name = std::ffi::CStr::from_ptr(fi.fname.as_ptr());
                if let Ok(name) = name.to_str() {
                    out.push(Entry {
                        name: name.to_string(),
                        size: fi.fsize as u32,
                        stamp: (u32::from(fi.fdate) << 16) | u32::from(fi.ftime),
                        fdate: fi.fdate,
                        ftime: fi.ftime,
                    });
                }
            }
            if out.len() >= 4 * MAX_FILES {
                break;
            }
        }
        sys::f_closedir(&mut d);
    }
    out
}

/// One `.psi` and its `.psb`'s state.
enum State {
    Current,
    /// A known non-compile: an empty `.psb` not older than its source, or
    /// this module's mark for the source as it is.
    Failed,
    Stale(&'static str),
}

/// The first `n` bytes of `path` (a header).
fn head(path: &str, n: usize) -> Vec<u8> {
    let mut b = vec![0u8; n];
    let got = fs::File::open(path).and_then(|mut f| f.read(&mut b)).unwrap_or(0);
    b.truncate(got);
    b
}

/// The mark for `psi` (as listed).
fn mark_bytes(psi: &Entry) -> [u8; MARK_LEN as usize] {
    let mut m = [0u8; MARK_LEN as usize];
    m[..4].copy_from_slice(MARK);
    m[4..8].copy_from_slice(&psi.size.to_le_bytes());
    m[8..12].copy_from_slice(&psi.stamp.to_le_bytes());
    m
}

/// `psb`'s state against `psi`. `check_abi`: the format may have changed
/// since the last full check (read the header).
fn state(psb: Option<&Entry>, psb_path: &str, psi: &Entry, check_abi: bool) -> State {
    let Some(b) = psb else {
        return State::Stale("not prepared");
    };
    // A mark first: its own date is the clock's (1980 with none).
    if b.size == MARK_LEN {
        let h = head(psb_path, MARK_LEN as usize);
        if h.len() == MARK_LEN as usize && &h[..4] == MARK {
            return if h[..] == mark_bytes(psi)[..] { State::Failed } else { State::Stale("its source changed") };
        }
    }
    if b.stamp < psi.stamp {
        return State::Stale("older than its source");
    }
    if b.size == 0 {
        return State::Failed;
    }
    if !check_abi {
        return State::Current;
    }
    let h = head(psb_path, 4);
    if h.len() < 4 || &h[..3] != b"PSB" {
        State::Stale("not a compiled Formula")
    } else if h[3] != psi::psb::ABI {
        State::Stale("prepared for another version")
    } else {
        State::Current
    }
}

/// Give `path` (filesystem) `psi`'s FAT date: the `.psb` as new as its source.
fn stamp(path: &str, psi: &Entry) {
    use esp_idf_sys as sys;
    let card = path.strip_prefix(config::SD_MOUNT).unwrap_or(path);
    let Ok(c) = std::ffi::CString::new(format!("0:{card}")) else {
        return;
    };
    // SAFETY: a zeroed FILINFO with the date fields set; a NUL-terminated path.
    let r = unsafe {
        let mut fi: sys::FILINFO = std::mem::zeroed();
        fi.fdate = psi.fdate;
        fi.ftime = psi.ftime;
        sys::f_utime(c.as_ptr(), &fi)
    };
    if r != sys::FRESULT_FR_OK {
        log::warn!("formulas: could not date {path} (FatFs {r}): it is prepared again at the next start");
    }
}

/// The source, or why not (too long, unreadable).
fn read_source(path: &str) -> Result<String, String> {
    let mut f = fs::File::open(path).map_err(|e| format!("could not be read: {e}"))?;
    let mut b = Vec::new();
    let mut chunk = [0u8; 256];
    let mut since_yield = 0;
    loop {
        let n = f.read(&mut chunk).map_err(|e| format!("could not be read: {e}"))?;
        if n == 0 {
            break;
        }
        b.extend_from_slice(&chunk[..n]);
        if b.len() > psi::MAX_SOURCE {
            return Err(format!("is longer than {} KB", psi::MAX_SOURCE / 1024));
        }
        since_yield += n;
        if since_yield >= CHUNK {
            since_yield = 0;
            std::thread::yield_now();
        }
    }
    String::from_utf8(b).map_err(|_| "is not UTF-8 text".to_string())
}

/// The heap a compile of `src` needs, and whether it is there now
/// (`Err`: free, needed, largest block).
fn fits(src: &str) -> Result<(), (u32, usize, usize)> {
    // (`need` with `RESERVE`, the network's transients and the player's
    // yield lines; see there.)
    use crate::state::{free_heap, largest_free_block};
    let need = STACK + 512 + 2 * std::mem::size_of::<psi::Program>() + psi::compile_heap_estimate(src) + RESERVE;
    let (free, block) = (free_heap(), largest_free_block());
    if free as usize >= need && block >= STACK.max(config::PSI_COMPILE_BLOCK) {
        Ok(())
    } else {
        Err((free, need, block))
    }
}

/// The `.psb` being written now (an FNV-1a hash of its stem's card path,
/// lower case; 0: none): `PUT`/`DELETE`/a move in `/api/files` on the
/// same Formula waits for it (`http::release_for_write`). With
/// `cache_builder::busy()`'s count (which every such handler holds first)
/// it is a Dekker pair, both `SeqCst`: one side always sees the other.
static WRITING: AtomicU32 = AtomicU32::new(0);
/// When the panel last missed a frame (ms since boot, 0: never).
static LATE_AT: AtomicU32 = AtomicU32::new(0);
/// Compiled files the player refused, waiting for [`tick`] to note them
/// on the card (hashes as [`WRITING`]'s).
static REDO: [AtomicU32; 4] = [const { AtomicU32::new(0) }; 4];

fn now_ms() -> u32 {
    // SAFETY: reads the system timer. Never 0.
    (unsafe { esp_idf_sys::esp_timer_get_time() } / 1000).max(1) as u32
}

/// FNV-1a over `card` (a Formula's card path without its extension), lower
/// case; never 0.
fn key(card: &str) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for b in card.bytes() {
        h = (h ^ u32::from(b.to_ascii_lowercase())).wrapping_mul(0x0100_0193);
    }
    h.max(1)
}

/// A filesystem or card path in `/formulas`, without its extension, as
/// [`key`] takes it.
fn formula_key(path: &str) -> Option<u32> {
    let card = path.strip_prefix(config::SD_MOUNT).unwrap_or(path);
    if !card.get(..10).is_some_and(|p| p.eq_ignore_ascii_case("/formulas/")) {
        return None;
    }
    let stem = card.rsplit_once('.').map_or(card, |(a, _)| a);
    Some(key(stem))
}

/// The display loop missed a frame (`main`): compiles wait a moment.
pub fn note_late() {
    LATE_AT.store(now_ms(), Relaxed);
}

/// `http::release_for_write`: wait (up to 1 s) while this module writes
/// the Formula `fs_path` names (or, for `/formulas` itself or above, any).
/// False: still writing.
pub fn wait_not_writing(fs_path: &str) -> bool {
    let card = fs_path.strip_prefix(config::SD_MOUNT).unwrap_or(fs_path).trim_end_matches('/');
    let any = card.is_empty() || card.eq_ignore_ascii_case("/formulas");
    let mine = if any { None } else { formula_key(card) };
    if !any && mine.is_none() {
        return true;
    }
    for _ in 0..50 {
        let w = WRITING.load(SeqCst);
        if w == 0 || mine.is_some_and(|k| k != w) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

/// The engine refused a card Formula's compiled file (`engine::
/// set_formula_refused`, the playing task): noted for [`tick`]. No heap,
/// no lock.
pub fn refused(path: &str) {
    let Some((file, _)) = opendmd_core::synth::formula_file(path) else {
        return;
    };
    let Some(k) = formula_key(file) else {
        return;
    };
    for slot in &REDO {
        match slot.compare_exchange(0, k, Relaxed, Relaxed) {
            Ok(_) => return,
            Err(v) if v == k => return,
            Err(_) => {}
        }
    }
}

/// The network worker's turn (CPU0): a retry of the compiles the heap
/// could not hold, when due ([`retry_tick`]); and refused compiled files
/// noted on the card, so the next start prepares them again (a cut write,
/// another version's copy). Nothing to do almost always.
pub fn tick() {
    retry_tick();
    if REDO.iter().all(|s| s.load(Relaxed) == 0) || !crate::CARD_READY.load(Relaxed) {
        return;
    }
    let mut line = String::new();
    for slot in &REDO {
        let k = slot.swap(0, Relaxed);
        if k != 0 {
            line.push_str(&format!("{k:08x}\n"));
        }
    }
    let path = format!("{}/formulas/{REDO_FILE}", config::SD_MOUNT);
    let _ = fs::OpenOptions::new().create(true).append(true).open(&path).and_then(|mut f| {
        use std::io::Write;
        f.write_all(line.as_bytes())
    });
    log::info!("formulas: a compiled file the panel refused is prepared again at the next start");
}

/// Prepare the card's Formulas (the boot task once the card is handed
/// over, CPU0): a pass; the compiles the heap could not hold are tried
/// again by the network worker ([`tick`], see [`RETRY_OFFLINE`]). `settings` for a look that names one
/// ([`crate::formula_look::file_changed`]) and the network setting.
#[optimize(size)] // (CPU0, after the handover: no frame runs it)
#[inline(never)]
pub fn prepare_after_handover(settings: &Arc<Mutex<crate::nvs_config::Settings>>) -> Counts {
    let t0 = Instant::now();
    let n = CRASHES.load(Relaxed);
    if n >= config::CARD_COMPILE_CRASHES_OFF {
        log::warn!("formulas: {n} starts in a row crashed while the card's Formulas were prepared: not prepared this start (until a power-on or an update)");
        return Counts::default();
    }
    let online = settings.lock().map(|s| s.network).unwrap_or(true);
    let (wait, tries) = if online { RETRY_ONLINE } else { RETRY_OFFLINE };
    let mut shorts = Vec::new();
    let c = pass(settings, &mut shorts, true);
    publish(c);
    if c.short > 0 && tries > 0 {
        // The retries are the network worker's ([`tick`]): the boot task
        // ends now and its stack goes back to the heap.
        *RETRY.lock().unwrap_or_else(|p| p.into_inner()) = Some(Retry {
            settings: settings.clone(),
            counts: c,
            shorts,
            wait,
            left: tries,
            next: Instant::now() + wait,
            t0,
        });
        return c;
    }
    finish(c, &shorts, t0);
    c
}

/// Compiles still to be tried again (the network worker's [`tick`]).
struct Retry {
    settings: Arc<Mutex<crate::nvs_config::Settings>>,
    counts: Counts,
    shorts: Vec<String>,
    wait: Duration,
    left: u32,
    next: Instant,
    t0: Instant,
}
static RETRY: Mutex<Option<Retry>> = Mutex::new(None);

/// One retry, when due and the heap has room for the least of them.
#[optimize(size)]
#[inline(never)]
fn retry_tick() {
    let mut g = RETRY.lock().unwrap_or_else(|p| p.into_inner());
    let Some(r) = g.as_mut() else { return };
    if Instant::now() < r.next {
        return;
    }
    r.left -= 1;
    r.next = Instant::now() + r.wait;
    if crate::state::free_heap() >= r.counts.min_need {
        let mut shorts = Vec::new();
        // (Not waiting for the card: this is the worker's loop; a busy card
        // leaves the rest for the next retry.)
        let again = pass(&r.settings, &mut shorts, false);
        let c = r.counts;
        // The later pass counts the earlier passes' compiles as ready:
        // they are this start's prepared, not its ready.
        r.counts = Counts {
            heap_low: low(c.heap_low, again.heap_low),
            stack_left: low(c.stack_left, again.stack_left),
            prepared: c.prepared + again.prepared,
            current: again.current.saturating_sub(c.prepared),
            min_need: if again.min_need == 0 { c.min_need } else { again.min_need },
            ..again
        };
        r.shorts = shorts;
        publish(r.counts);
    }
    if r.counts.short == 0 || r.left == 0 {
        if let Some(r) = g.take() {
            finish(r.counts, &r.shorts, r.t0);
        }
    }
}

/// The pass's end: the ones left short (once) and the summary.
fn finish(c: Counts, shorts: &[String], t0: Instant) {
    if !shorts.is_empty() {
        log::warn!(
            "formulas: short of memory for {} (the least needs {} B free; {} B free): prepared at the next start",
            shorts.join(", "),
            c.min_need,
            crate::state::free_heap()
        );
    }
    if c.sources > 0 {
        log::info!(
            "formulas: {} on the card: {} ready, {} prepared now, {} do not compile, {} short of memory, in {} ms",
            c.sources,
            c.current,
            c.prepared,
            c.failed,
            c.short,
            t0.elapsed().as_millis()
        );
    }
}

fn publish(c: Counts) {
    *LAST.lock().unwrap_or_else(|p| p.into_inner()) = Some(c);
}

/// The least of two figures, 0 meaning none.
fn low(a: u32, b: u32) -> u32 {
    match (a, b) {
        (0, x) | (x, 0) => x,
        (a, b) => a.min(b),
    }
}

/// `name`'s stem if it ends in `ext` (any case), on a char boundary.
fn stem<'a>(name: &'a str, ext: &str) -> Option<&'a str> {
    let at = name.len().checked_sub(ext.len())?;
    (at > 0 && name.get(at..).is_some_and(|e| e.eq_ignore_ascii_case(ext))).then(|| &name[..at])
}

/// Write `bytes` as `path` (created or truncated: one lookup), in
/// [`CHUNK`]s with the CPU yielded between them.
fn put(path: &str, bytes: &[u8]) -> bool {
    use std::io::Write;
    let r = fs::File::create(path).and_then(|mut f| {
        for c in bytes.chunks(CHUNK) {
            f.write_all(c)?;
            std::thread::yield_now();
        }
        Ok(())
    });
    match r {
        Ok(()) => {
            crate::fatidx::note_written(path);
            true
        }
        Err(e) => {
            log::warn!("formulas: writing {path} failed ({e}); not prepared");
            false
        }
    }
}

/// Wait while the card is someone else's (a file going to or from it, the
/// cache builder at work) or the panel just missed a frame. False: it
/// stayed busy for two minutes (the rest waits for the next start).
fn give_way(wait: bool) -> bool {
    for _ in 0..if wait { 1200 } else { 1 } {
        let late = LATE_AT.load(Relaxed);
        let recent_late = late != 0 && now_ms().wrapping_sub(late) < AFTER_LATE.as_millis() as u32;
        if !crate::cache_builder::transfer_in_flight() && !crate::cache_builder::task_up() && !recent_late {
            std::thread::sleep(Duration::from_millis(20));
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

/// This module writes the Formula `k` now ([`WRITING`]): unless a file
/// transfer has begun (it goes first). Cleared by the guard.
fn claim(k: u32) -> Option<Writing> {
    WRITING.store(k, SeqCst);
    if crate::cache_builder::transfer_in_flight() {
        WRITING.store(0, SeqCst);
        return None;
    }
    Some(Writing)
}

struct Writing;
impl Drop for Writing {
    fn drop(&mut self) {
        WRITING.store(0, SeqCst);
    }
}

#[optimize(size)]
#[inline(never)]
fn pass(settings: &Arc<Mutex<crate::nvs_config::Settings>>, shorts: &mut Vec<String>, wait: bool) -> Counts {
    // In flight from the listing to the last write: a crash in between
    // counts against the next start ([`boot`]); none outside it.
    guard(0).store(GUARD_MAGIC, Relaxed);
    let c = pass_in(settings, shorts, wait);
    guard(0).store(0, Relaxed);
    guard(1).store(crashes_word(0), Relaxed);
    c
}

#[optimize(size)]
#[inline(never)]
fn pass_in(settings: &Arc<Mutex<crate::nvs_config::Settings>>, shorts: &mut Vec<String>, wait: bool) -> Counts {
    let card_dir = "/formulas";
    let dir = format!("{}{card_dir}", config::SD_MOUNT);
    let entries = list(card_dir);
    let n_sources = entries.iter().filter(|e| stem(&e.name, ".psi").is_some()).count();
    // (In the directory's order: a sort here cost 6 KB of flash.)
    let sources = entries.iter().filter(|e| stem(&e.name, ".psi").is_some()).take(MAX_FILES);
    let abi_path = format!("{dir}/{ABI_FILE}");
    let redo_path = format!("{dir}/{REDO_FILE}");
    let redo: Vec<u32> = if entries.iter().any(|e| e.name == REDO_FILE) {
        fs::read_to_string(&redo_path)
            .unwrap_or_default()
            .lines()
            .filter_map(|l| u32::from_str_radix(l.trim(), 16).ok())
            .collect()
    } else {
        Vec::new()
    };
    let abi_same = redo.is_empty()
        && entries.iter().any(|e| e.name == ABI_FILE)
        && head(&abi_path, 4).first().is_some_and(|&b| b == psi::psb::ABI);
    let compiled = |base: &str| {
        entries.iter().find(|e| stem(&e.name, ".psb").is_some_and(|s| s.eq_ignore_ascii_case(base)))
    };
    let mut c = Counts::default();
    let mut stopped = false;
    for psi_entry in sources {
        c.sources += 1;
        if stopped {
            c.short += 1;
            continue;
        }
        let name = &psi_entry.name;
        let base = stem(name, ".psi").unwrap_or(name);
        let psi_path = format!("{dir}/{name}");
        let psb_path = format!("{dir}/{base}.psb");
        let k = key(&format!("{card_dir}/{base}"));
        let checked = if redo.contains(&k) {
            State::Stale("refused where it played")
        } else {
            state(compiled(base), &psb_path, psi_entry, !abi_same)
        };
        let why = match checked {
            State::Current => {
                c.current += 1;
                continue;
            }
            State::Failed => {
                c.failed += 1;
                continue;
            }
            State::Stale(why) => why,
        };
        if !give_way(wait) {
            if wait {
                log::warn!("formulas: the card stayed busy; the rest is prepared later");
            }
            stopped = true;
            c.short += 1;
            continue;
        }
        let src = match read_source(&psi_path) {
            Ok(s) => s,
            Err(e) => {
                // Marked, so it is not read again every start.
                log::warn!("formulas: /formulas/{name} {e}: not prepared");
                if let Some(_w) = claim(k) {
                    put(&psb_path, &mark_bytes(psi_entry));
                }
                c.failed += 1;
                continue;
            }
        };
        if let Err((_free, need, _block)) = fits(&src) {
            c.min_need = if c.min_need == 0 { need as u32 } else { c.min_need.min(need as u32) };
            shorts.push(format!("/formulas/{name}"));
            c.short += 1;
            continue;
        }
        let Some(_writing) = claim(k) else {
            // A file transfer began: this one at the next pass.
            shorts.push(format!("/formulas/{name}"));
            c.short += 1;
            continue;
        };
        // Marked first: a compile that crashes the board leaves it so, and
        // the next start moves on.
        if !put(&psb_path, &mark_bytes(psi_entry)) {
            c.failed += 1;
            continue;
        }
        let before = crate::state::free_heap();
        let started = Instant::now();
        let built = crate::psi_stack::run_with(STACK, || psi::build(&src));
        let took = started.elapsed().as_millis();
        drop(src);
        let Some((built, left)) = built else {
            log::warn!("formulas: /formulas/{name} not prepared: no task to compile on ({} B free)", crate::state::free_heap());
            let _ = fs::remove_file(&psb_path);
            shorts.push(format!("/formulas/{name}"));
            c.short += 1;
            continue;
        };
        c.stack_left = low(c.stack_left, left);
        let heap_low = crate::psi_stack::last_low();
        c.heap_low = low(c.heap_low, heap_low);
        let bytes = match built {
            Ok(b) => b,
            Err(e) if e.msg == psi::NO_MEMORY => {
                log::warn!("formulas: /formulas/{name} not prepared: the compile ran short of memory");
                let _ = fs::remove_file(&psb_path);
                shorts.push(format!("/formulas/{name}"));
                c.short += 1;
                continue;
            }
            Err(e) => {
                // (The mark written before stays: it says so.)
                log::warn!("formulas: /formulas/{name} does not compile ({}); it is skipped where it plays", e.msg);
                c.failed += 1;
                continue;
            }
        };
        if !put(&psb_path, &bytes) {
            c.failed += 1;
            continue;
        }
        stamp(&psb_path, psi_entry);
        drop(_writing);
        c.prepared += 1;
        log::info!(
            "formulas: /formulas/{name} prepared for the panel ({why}): {} B in {took} ms; heap {before} B free before, {heap_low} B at the lowest; compile stack {left} B left",
            bytes.len()
        );
        // A look (or the start-up show) that names it builds again now.
        crate::formula_look::file_changed(settings, &format!("{card_dir}/{base}.psb"), false);
    }
    // Every header checked under this ABI (and every source looked at):
    // record it, so the next start reads none; the refused ones are done.
    if !abi_same && c.short == 0 && n_sources < MAX_FILES {
        let _ = fs::write(&abi_path, [psi::psb::ABI, b'\n']);
    }
    if !redo.is_empty() {
        let _ = fs::remove_file(&redo_path);
    }
    c
}

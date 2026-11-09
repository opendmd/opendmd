//! The card's Formulas prepared on the display: every `/formulas/<stem>.psi`
//! whose compiled `<stem>.psb` beside it is missing or stale is compiled
//! here, when the card is mounted (the boot task, CPU0, before the card is
//! the player's), so a card filled on a computer plays its Formulas with no
//! web app, and a firmware update that changed the compiled format
//! (`psi::psb::ABI`) does not leave them unplayable.
//!
//! **Stale** is: no `.psb`; one whose header is not this player's format
//! (`PSB` and the ABI byte); or one older than its `.psi` (FAT's 2 s
//! stamps). Every `.psb` written here is given its `.psi`'s time
//! (`utime`), so it is current until the source changes, whatever the
//! display's own clock says (none with WiFi off: FatFs stamps 1980). An
//! empty `.psb` not older than its source is the web app's (and this
//! module's) record of a source that does not compile: not tried again
//! until the source changes.
//!
//! **No crash loop.** Before a source is compiled its `.psb` is written
//! empty (with the source's time): a compile that crashes the board leaves
//! the source marked as one that does not compile, so the next boot moves
//! on. A crash anywhere else in the mount compile (a count in RTC memory,
//! [`boot`]) stops it after [`config::CARD_COMPILE_CRASHES_OFF`] boots in a
//! row, until a power-on or an update. A source that cannot be read, or is
//! too long, is marked the same way.
//!
//! **Cheap when nothing changed.** One listing of `/formulas` gives every
//! `.psi` and `.psb`; a `.psi` with no `.psb` is compiled without another
//! lookup, and only a pair is checked (two `stat`s and a 4-byte read).
//!
//! **One at a time, within the heap.** Each compile runs on a task of its
//! own ([`STACK`], `psi_stack::run_with`) and starts only if the heap has its
//! stack, two programs, `psi::compile_heap_estimate` of the source and
//! [`RESERVE`] free, with a block for the stack; otherwise it is refused
//! (logged, counted, tried again at the next mount). The compiler's lists
//! grow fallibly besides, so a misjudged estimate is a refusal, not an
//! abort. The bytes are `psi::build`'s: what the web app and `psi build`
//! write.
//!
//! The counts go to the log and to `/api/status` (`card_formulas`).

use opendmd_core::psi;
use std::fs;
use std::io::Read;
use std::sync::Mutex;
use std::time::{Instant, SystemTime};

use crate::config;

/// The compile task's stack. The parser recurses up to `psi::MAX_DEPTH`
/// levels: the deepest of the lane's Formulas nested to the cap (calls,
/// brackets, `?:`, `if` in `draw`, user functions, sprite reads) used
/// ~20 KB of it; `emu-offline` asserts 8 KB left for each.
pub const STACK: usize = 32 * 1024;
/// The heap a compile leaves for everyone else (the show on the panel, the
/// error's text, the allocator's overhead).
const RESERVE: usize = 8 * 1024;
/// At most this many `.psi` files are looked at in one mount.
const MAX_FILES: usize = 256;

/// What a mount found and did.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct Counts {
    /// `.psi` files seen.
    pub sources: u32,
    /// Their `.psb` already current.
    pub current: u32,
    /// Compiled now.
    pub prepared: u32,
    /// Do not compile (an empty `.psb` beside them says so).
    pub failed: u32,
    /// Not compiled: the heap was short (tried again at the next mount).
    pub short: u32,
    /// Free heap at the lowest point measured around a compile (B; 0: none).
    pub heap_low: u32,
    /// The least of the compile task's stack left unused (B; 0: none).
    pub stack_left: u32,
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
static CRASHES: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

fn guard(i: usize) -> &'static core::sync::atomic::AtomicU32 {
    // SAFETY: a word of `GUARD`, only ever accessed through this view
    // (loads and stores).
    unsafe { core::sync::atomic::AtomicU32::from_ptr((&raw mut GUARD).cast::<u32>().add(i)) }
}

fn crashes_word(n: u32) -> u32 {
    (n & 0xFFFF) | (!n & 0xFFFF) << 16
}

/// First thing at boot (`main`, beside `fatidx::boot`): a mount compile in
/// flight when the board crashed counts; a power-on (RTC memory lost)
/// starts again from none.
pub fn boot(crashed: bool, power_on: bool) {
    use core::sync::atomic::Ordering::Relaxed;
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
    use core::sync::atomic::Ordering::Relaxed;
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

/// One `.psi` and its `.psb`'s state.
enum State {
    Current,
    /// A known non-compile (an empty `.psb` not older than the source).
    Failed,
    Stale(&'static str),
}

fn mtime(path: &str) -> Option<SystemTime> {
    fs::metadata(path).ok()?.modified().ok()
}

/// The `.psb`'s state against its `.psi` (`psi_time`).
fn state(psb: &str, psi_time: Option<SystemTime>) -> State {
    let Ok(mut f) = fs::File::open(psb) else {
        return State::Stale("not prepared");
    };
    let mut head = [0u8; 4];
    let n = f.read(&mut head).unwrap_or(0);
    drop(f);
    // Older than its source by more than FAT's 2 s: written before the
    // source last changed.
    let older = match (mtime(psb), psi_time) {
        (Some(b), Some(s)) => b + std::time::Duration::from_secs(2) < s,
        _ => false,
    };
    if older {
        return State::Stale("older than its source");
    }
    if n == 0 {
        return State::Failed;
    }
    if n < 4 || &head[..3] != b"PSB" {
        return State::Stale("not a compiled Formula");
    }
    if head[3] != psi::psb::ABI {
        return State::Stale("prepared for another version");
    }
    State::Current
}

/// Give `path` the time `t` (`utime`): the `.psb` as new as its `.psi`.
fn stamp(path: &str, t: SystemTime) {
    let Ok(secs) = t.duration_since(SystemTime::UNIX_EPOCH) else {
        return;
    };
    let Ok(c) = std::ffi::CString::new(path) else {
        return;
    };
    let times = esp_idf_sys::utimbuf {
        actime: secs.as_secs() as _,
        modtime: secs.as_secs() as _,
    };
    // SAFETY: a NUL-terminated path and a filled `utimbuf`, both alive for the call.
    if unsafe { esp_idf_sys::utime(c.as_ptr(), &times) } != 0 {
        log::warn!("formulas: could not set the time of {path}: it is prepared again at the next mount");
    }
}

/// Write `bytes` as `path`, through a temporary file renamed into place.
fn write(path: &str, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = format!("{path}.tmp");
    fs::write(&tmp, bytes)?;
    // (FAT's rename does not replace.)
    let _ = fs::remove_file(path);
    fs::rename(&tmp, path)
}

/// The source, or why not (too long, unreadable).
fn read_source(path: &str) -> Result<String, String> {
    let f = fs::File::open(path).map_err(|e| format!("could not be read: {e}"))?;
    let mut src = String::new();
    let max = psi::MAX_SOURCE as u64;
    f.take(max + 1)
        .read_to_string(&mut src)
        .map_err(|e| format!("could not be read: {e}"))?;
    if src.len() as u64 > max {
        return Err(format!("is longer than {} KB", psi::MAX_SOURCE / 1024));
    }
    Ok(src)
}

/// The heap a compile of `src` needs, and whether it is there now
/// (`Err`: free, needed, largest block).
fn fits(src: &str) -> Result<(), (u32, usize, usize)> {
    use crate::state::{free_heap, largest_free_block};
    let need = STACK + 512 + 2 * std::mem::size_of::<psi::Program>() + psi::compile_heap_estimate(src) + RESERVE;
    let (free, block) = (free_heap(), largest_free_block());
    if free as usize >= need && block >= STACK.max(config::PSI_COMPILE_BLOCK) {
        Ok(())
    } else {
        Err((free, need, block))
    }
}

/// Prepare the mounted card's Formulas (the boot task, before the
/// handover): see the module docs.
#[optimize(size)] // (CPU0, boot: no frame runs it)
#[inline(never)]
pub fn prepare_at_mount() -> Counts {
    let t0 = Instant::now();
    let dir = format!("{}/formulas", config::SD_MOUNT);
    {
        use core::sync::atomic::Ordering::Relaxed;
        let n = CRASHES.load(Relaxed);
        if n >= config::CARD_COMPILE_CRASHES_OFF {
            log::warn!("formulas: {n} starts in a row crashed while the card's Formulas were prepared: not prepared this start (until a power-on or an update)");
            return Counts::default();
        }
        guard(0).store(GUARD_MAGIC, Relaxed);
    }
    let c = prepare_in(&dir);
    {
        use core::sync::atomic::Ordering::Relaxed;
        guard(0).store(0, Relaxed);
        guard(1).store(crashes_word(0), Relaxed);
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
    *LAST.lock().unwrap_or_else(|p| p.into_inner()) = Some(c);
    c
}

/// `path` written empty with the source's time (`t`): the mark of a source
/// that does not compile (or not yet). False: it could not be written.
fn mark(path: &str, t: Option<SystemTime>) -> bool {
    match write(path, &[]) {
        Ok(()) => {
            if let Some(t) = t {
                stamp(path, t);
            }
            true
        }
        Err(e) => {
            log::warn!("formulas: {path}: writing it failed ({e}); not prepared");
            false
        }
    }
}

/// `name`'s stem if it ends in `ext` (any case), on a char boundary.
fn stem<'a>(name: &'a str, ext: &str) -> Option<&'a str> {
    let at = name.len().checked_sub(ext.len())?;
    (at > 0 && name.get(at..).is_some_and(|e| e.eq_ignore_ascii_case(ext))).then(|| &name[..at])
}

#[optimize(size)]
#[inline(never)]
fn prepare_in(dir: &str) -> Counts {
    // One listing: the sources, and which have a compiled file (by stem,
    // as FAT compares names: case aside).
    let mut names: Vec<String> = Vec::new();
    let mut compiled: Vec<String> = Vec::new();
    if let Ok(rd) = fs::read_dir(dir) {
        for e in rd.filter_map(|e| e.ok()) {
            let Ok(n) = e.file_name().into_string() else {
                continue;
            };
            if stem(&n, ".psi").is_some() {
                if names.len() < MAX_FILES {
                    names.push(n);
                }
            } else if let Some(s) = stem(&n, ".psb") {
                compiled.push(s.to_ascii_lowercase());
            }
        }
    }
    names.sort_unstable();
    compiled.sort_unstable();
    let mut c = Counts::default();
    for name in &names {
        c.sources += 1;
        let base = stem(name, ".psi").unwrap_or(name);
        let psi_path = format!("{dir}/{name}");
        let psb_path = format!("{dir}/{base}.psb");
        let paired = compiled.binary_search(&base.to_ascii_lowercase()).is_ok();
        let psi_time = mtime(&psi_path);
        let checked = if paired { state(&psb_path, psi_time) } else { State::Stale("not prepared") };
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
        let src = match read_source(&psi_path) {
            Ok(s) => s,
            Err(e) => {
                // Marked, so it is not read again every start.
                log::warn!("formulas: /formulas/{name} {e}: not prepared");
                mark(&psb_path, psi_time);
                c.failed += 1;
                continue;
            }
        };
        if let Err((free, need, block)) = fits(&src) {
            log::warn!(
                "formulas: /formulas/{name} not prepared ({why}): short of memory, {free} B free (needs {need}), largest block {block} B; tried again at the next start"
            );
            c.short += 1;
            continue;
        }
        // Marked as not compiling first: a compile that crashes the board
        // leaves it so, and the next start moves on.
        if !mark(&psb_path, psi_time) {
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
            c.short += 1;
            continue;
        };
        c.stack_left = if c.stack_left == 0 { left } else { c.stack_left.min(left) };
        let low = crate::psi_stack::last_low();
        c.heap_low = if c.heap_low == 0 { low } else { c.heap_low.min(low) };
        let bytes = match built {
            Ok(b) => b,
            Err(e) if e.msg == psi::NO_MEMORY => {
                log::warn!("formulas: /formulas/{name} not prepared: the compile ran short of memory; tried again at the next start");
                let _ = fs::remove_file(&psb_path);
                c.short += 1;
                continue;
            }
            Err(e) => {
                // (The empty file written before is the mark.)
                log::warn!("formulas: /formulas/{name} does not compile ({}); it is skipped where it plays", e.msg);
                c.failed += 1;
                continue;
            }
        };
        match write(&psb_path, &bytes) {
            Ok(()) => {
                if let Some(t) = psi_time {
                    stamp(&psb_path, t);
                }
                {
                    c.prepared += 1;
                    log::info!(
                        "formulas: /formulas/{name} prepared for the panel ({why}): {} B in {took} ms; heap {before} B free before, {low} B at the lowest; compile stack {left} B left",
                        bytes.len()
                    );
                }
            }
            Err(e) => {
                log::warn!("formulas: /formulas/{name}: writing its compiled file failed ({e})");
                c.failed += 1;
            }
        }
    }
    c
}

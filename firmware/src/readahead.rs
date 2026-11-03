//! The playing GIF's read-ahead: 8 KB of the file the display core is
//! reading raw (`fatidx`), kept ahead of its position by a task on CPU0 and
//! served to its reads from memory (`opendmd_core::readahead::Ring`).
//!
//! The marginal card in the spare (iso14) had stretches where a read took
//! ten times its time (~11 ms a block, ~20 ms at worst) and others where a
//! read queued ~45 ms behind a write's busy time. The display core read the
//! card in its frame for both; with this it reads memory, and the filling
//! task takes the wait.
//!
//! - Which file: the slot the display core reads twice running (a copy
//!   opened ahead reads once or twice and then waits; the file on screen
//!   reads on). The display core clones the slot's extents when it starts
//!   following one (an allocation at a file change, never per frame).
//! - The display core never waits here: its lock is tried; busy means a
//!   read from the card, as before.
//! - A slot's close forgets it before the slot is free (`fatidx::v_close`),
//!   and a block read for a slot that was closed meanwhile is dropped (the
//!   generation changed), so no freed cluster's bytes are ever served.
//! - The filling task is resident: started once (4 KB of stack, kept), it
//!   waits on its FreeRTOS task notification while no file is followed, and
//!   the network worker wakes it ([`tick`]). Until 2026-11-03 it was a task
//!   started for each file, which ended after it: every start and end cost
//!   a std thread's ~190 B that never came back (`task::spawn_core0`), the
//!   spare's heap filling with small blocks GIF by GIF. Its ring (8 KB) and
//!   block (2 KB) are taken when a file is followed and the heap has room,
//!   and given back at a switch (before the next decoder and the
//!   transition) and after 5 s without a read (a Formula plays from
//!   memory). A ring kept for good would cost the heap 10 KB at all times:
//!   on the spare (38-45 KB free while playing) that is the transition
//!   gate's margin.

use opendmd_core::fatread::{Extents, RawFile};
use opendmd_core::readahead::{Ring, BLOCK};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering::{Relaxed, SeqCst}};
use std::sync::Mutex;
use std::time::Duration;

/// Blocks of [`BLOCK`] bytes: 8 KB.
const BLOCKS: usize = 4;
/// Given back after this long without a read from the display core.
const IDLE_MS: u32 = 5_000;
/// Taken only with this much heap free beyond it, and an 8 KB block: a
/// transition's 24 KB and its 12 KB reserve, and some. (Given back at the
/// file's close, before the next file's transition asks for its buffers.)
const MIN_HEAP: u32 = 40 * 1024;
const SS: usize = 512;

struct State {
    /// The slot followed, and its generation (bumped at every change).
    slot: Option<usize>,
    gen: u32,
    /// A slot the display core read once, not followed yet.
    candidate: Option<usize>,
    /// The display core's position in the followed file, and its size.
    pos: u64,
    size: u64,
    ext: Option<Extents>,
    pdrv: u8,
    ring: Option<Ring<BLOCKS>>,
    last_read_ms: u32,
    /// A block has been fetched since the slot or position changed: a read
    /// it cannot serve now is an underrun.
    primed: bool,
    /// When [`release`] last gave the ring back (ms uptime).
    released_ms: u32,
}

static STATE: Mutex<State> = Mutex::new(State {
    slot: None,
    gen: 0,
    candidate: None,
    pos: 0,
    size: 0,
    ext: None,
    pdrv: 0,
    ring: None,
    last_read_ms: 0,
    primed: false,
    released_ms: 0,
});
/// The ring is held and a file followed (for the status without the lock).
static ACTIVE: AtomicBool = AtomicBool::new(false);
/// [`release`] found the lock busy: the task gives the ring back.
static RELEASE: AtomicBool = AtomicBool::new(false);
/// Slots closed while the lock was busy (bit `k`): see [`closed`].
static CLOSE_PENDING: AtomicU32 = AtomicU32::new(0);
/// After [`release`], the ring is not taken again for this long: the next
/// file's decoder and its transition's buffers come first (a dissolve is
/// 400 ms).
const HOLD_OFF_MS: u32 = 1_000;

/// The filling task is working (set by [`tick`], cleared by the task under
/// the state's lock as it decides to wait).
static RUNNING: AtomicBool = AtomicBool::new(false);
/// The resident filling task, once started (its FreeRTOS handle).
static TASK: std::sync::atomic::AtomicPtr<core::ffi::c_void> = std::sync::atomic::AtomicPtr::new(std::ptr::null_mut());
static SPAWNS: AtomicU32 = AtomicU32::new(0);
/// Wake-ups of the resident task (a file followed again).
static WAKES: AtomicU32 = AtomicU32::new(0);
static HITS: AtomicU32 = AtomicU32::new(0);
static UNDERRUNS: AtomicU32 = AtomicU32::new(0);
static FETCHES: AtomicU32 = AtomicU32::new(0);
static TAKEN: AtomicU32 = AtomicU32::new(0);
static NO_HEAP: AtomicU32 = AtomicU32::new(0);
static ERRORS: AtomicU32 = AtomicU32::new(0);
static BUSY: AtomicU32 = AtomicU32::new(0);
/// The least read ahead after a served read, since the last status.
static FILL_MIN: AtomicU32 = AtomicU32::new(u32::MAX);
static FILL: AtomicU32 = AtomicU32::new(0);
static STACK_LOW: AtomicU32 = AtomicU32::new(u32::MAX);

fn now_ms() -> u32 {
    // SAFETY: a plain clock read.
    (unsafe { esp_idf_sys::esp_timer_get_time() } / 1000) as u32
}

/// The network worker's turn: the filling task started when the display
/// core follows a file and the task is not up.
pub fn tick() {
    {
        let Some(s) = try_locked() else { return };
        let wanted = s.slot.is_some() && now_ms().wrapping_sub(s.last_read_ms) < IDLE_MS;
        if !wanted || RUNNING.load(Relaxed) {
            return;
        }
        RUNNING.store(true, Relaxed);
    }
    let task = TASK.load(SeqCst);
    if !task.is_null() {
        // SAFETY: the resident task's handle (it never ends): its
        // notification count raised, which its wait takes.
        unsafe { esp_idf_sys::xTaskGenericNotify(task as esp_idf_sys::TaskHandle_t, 0, 0, esp_idf_sys::eNotifyAction_eIncrement, std::ptr::null_mut()) };
        WAKES.fetch_add(1, Relaxed);
        return;
    }
    let spawned = crate::task::spawn_core0(
        b"readahead\0",
        crate::config::READAHEAD_STACK,
        crate::config::READAHEAD_PRIORITY,
        Box::new(run),
    );
    match spawned {
        Ok(()) => {
            SPAWNS.fetch_add(1, Relaxed);
        }
        Err(e) => {
            log::warn!("read-ahead: no task ({e})");
            NO_HEAP.fetch_add(1, Relaxed);
            RUNNING.store(false, Relaxed);
        }
    }
}

/// The display core's raw read of slot `k` at `file`'s position: served
/// from the ring (`Some(bytes)`, the position moved on), or `None` (read the
/// card, then [`moved`]).
#[inline(never)]
pub fn serve(k: usize, file: &mut RawFile, pdrv: u8, buf: &mut [u8]) -> Option<usize> {
    let Some(mut s) = try_locked() else {
        BUSY.fetch_add(1, Relaxed);
        return None;
    };
    s.last_read_ms = now_ms();
    let pos = file.pos();
    if s.slot != Some(k) {
        if s.candidate == Some(k) {
            // Read twice running: the file on screen. Followed from here.
            s.slot = Some(k);
            s.gen = s.gen.wrapping_add(1);
            s.ext = Some(file.extents().clone());
            s.size = file.size();
            s.pdrv = pdrv;
            s.pos = pos;
            s.primed = false;
            if let Some(r) = s.ring.as_mut() {
                r.clear();
            }
        } else {
            s.candidate = Some(k);
        }
        return None;
    }
    if pos != s.pos {
        // A seek (the GIF looping): the ring refills from there.
        s.primed = false;
    }
    s.pos = pos;
    let n = s.ring.as_ref().map_or(0, |r| r.serve(pos, buf));
    if n == 0 {
        if s.ring.is_some() && s.primed && pos < s.size {
            UNDERRUNS.fetch_add(1, Relaxed);
        }
        return None;
    }
    let to = pos + n as u64;
    file.seek(to);
    s.pos = to;
    HITS.fetch_add(1, Relaxed);
    let ahead = s.ring.as_ref().map_or(0, |r| r.ahead(to)).min(u32::MAX as u64) as u32;
    FILL.store(ahead, Relaxed);
    FILL_MIN.fetch_min(ahead, Relaxed);
    Some(n)
}

/// After a read of slot `k` from the card: the display core is at `pos`.
#[inline(never)]
pub fn moved(k: usize, pos: u64) {
    if let Some(mut s) = try_locked() {
        if s.slot == Some(k) {
            s.pos = pos;
        }
    }
}

/// Slot `k` is closing (before it is free): forget it. The display core
/// never waits here: with the lock busy (the filling task, at a higher
/// priority than the main task, so no inheritance helps), the slot is
/// marked in [`CLOSE_PENDING`], and whoever takes the lock next forgets it
/// before anything else, so a block of the old file is never served for a
/// new one in the same slot.
#[inline(never)]
pub fn closed(k: usize) {
    match STATE.try_lock() {
        Ok(mut s) => {
            let freed = apply_pending(&mut s);
            let more = forget(&mut s, k);
            drop(s);
            drop((freed, more));
        }
        Err(_) => {
            CLOSE_PENDING.fetch_or(1 << k, SeqCst);
        }
    }
}

/// What a forgotten slot leaves to free (after the lock).
type Freed = (Option<Extents>, Option<Ring<BLOCKS>>);

fn forget(s: &mut State, k: usize) -> Freed {
    if s.candidate == Some(k) {
        s.candidate = None;
    }
    if s.slot != Some(k) {
        return (None, None);
    }
    s.slot = None;
    s.gen = s.gen.wrapping_add(1);
    ACTIVE.store(false, Relaxed);
    (s.ext.take(), s.ring.take())
}

/// The closes [`closed`] could not make: made now, by whoever holds the
/// lock, before it reads or changes anything else.
fn apply_pending(s: &mut State) -> Vec<Freed> {
    let bits = CLOSE_PENDING.swap(0, SeqCst);
    if bits == 0 {
        return Vec::new();
    }
    (0..32).filter(|k| bits & (1 << k) != 0).map(|k| forget(s, k)).collect()
}

/// The state, the closes pending made (the filling task, which may wait).
fn locked() -> std::sync::MutexGuard<'static, State> {
    let mut s = STATE.lock().unwrap_or_else(|p| p.into_inner());
    // (Freed under the lock: rare, and not the display core.)
    drop(apply_pending(&mut s));
    s
}

/// The state if free, the closes pending made (the display core: never
/// waits; a pending close frees at most the ring, once).
fn try_locked() -> Option<std::sync::MutexGuard<'static, State>> {
    let mut s = STATE.try_lock().ok()?;
    drop(apply_pending(&mut s));
    Some(s)
}

/// The main loop is opening a file (a switch, or the next file ahead):
/// give the ring back now, before the new decoder and the transition's
/// 24 KB ask the heap (the file on screen stays open through a transition,
/// so its close is too late), and take it again only after
/// [`HOLD_OFF_MS`]. The display core: the lock is tried (busy: the task
/// gives it back at its next look, `RELEASE`).
#[inline(never)]
pub fn release() {
    let ring = match try_locked() {
        Some(mut s) => {
            s.released_ms = now_ms();
            ACTIVE.store(false, Relaxed);
            s.ring.take()
        }
        None => {
            RELEASE.store(true, Relaxed);
            None
        }
    };
    drop(ring);
}

/// One block to fetch: its offset and length, the card sectors (lba,
/// count) it is in, the drive and the generation it is for.
struct Fetch {
    off: u64,
    len: usize,
    runs: [(u32, u32); BLOCK / SS],
    nruns: usize,
    pdrv: u8,
    gen: u32,
}

enum Next {
    Fetch(Fetch),
    /// Full: look again soon.
    Wait,
    /// No room for the ring: look again in a while (`no_heap` counts
    /// these looks).
    NoHeap,
    /// Nothing followed (or idle): given back.
    Idle,
}

fn plan() -> Next {
    // The lock is held only for bookkeeping: the heap's checks, the 8 KB
    // allocation (and its zeroing) and frees are done outside it, so the
    // display core's `closed` never waits behind them.
    let need_ring = {
        let mut s = locked();
        if RELEASE.swap(false, Relaxed) {
            s.released_ms = now_ms();
            let ring = s.ring.take();
            ACTIVE.store(false, Relaxed);
            drop(s);
            drop(ring);
            return Next::Wait;
        }
        let idle = s.slot.is_none() || now_ms().wrapping_sub(s.last_read_ms) >= IDLE_MS;
        if idle {
            let ring = s.ring.take();
            ACTIVE.store(false, Relaxed);
            // Waiting from here (under the lock): `tick` wakes it when wanted.
            RUNNING.store(false, Relaxed);
            drop(s);
            drop(ring);
            return Next::Idle;
        }
        if s.ring.is_none() && now_ms().wrapping_sub(s.released_ms) < HOLD_OFF_MS {
            // Given back at a switch: not taken again while the next file's
            // decoder and transition take theirs.
            return Next::Wait;
        }
        s.ring.is_none().then_some(s.gen)
    };
    if let Some(gen) = need_ring {
        let heap = crate::state::free_heap();
        if heap < MIN_HEAP + (BLOCKS * BLOCK) as u32 || crate::state::largest_free_block() < BLOCKS * BLOCK {
            NO_HEAP.fetch_add(1, Relaxed);
            return Next::NoHeap;
        }
        let Some(ring) = Ring::try_new() else {
            NO_HEAP.fetch_add(1, Relaxed);
            return Next::NoHeap;
        };
        let mut s = locked();
        if s.gen != gen || s.ring.is_some() || s.slot.is_none() {
            // Changed meanwhile: freed outside the lock, asked again.
            drop(s);
            drop(ring);
            return Next::Wait;
        }
        s.ring = Some(ring);
        ACTIVE.store(true, Relaxed);
        TAKEN.fetch_add(1, Relaxed);
    }
    let s = locked();
    let (pos, size) = (s.pos, s.size);
    let Some(off) = s.ring.as_ref().and_then(|r| r.want(pos, size)) else {
        return Next::Wait;
    };
    let Some(ext) = s.ext.as_ref() else { return Next::Wait };
    let len = (size - off).min(BLOCK as u64) as usize;
    // The block's sectors, as runs of consecutive ones on the card.
    let mut f = Fetch { off, len, runs: [(0, 0); BLOCK / SS], nruns: 0, pdrv: s.pdrv, gen: s.gen };
    let sectors = len.div_ceil(SS) as u32;
    let mut done = 0u32;
    while done < sectors {
        let Some((lba, run)) = ext.map(off + done as u64 * SS as u64) else {
            return Next::Wait;
        };
        let n = run.min(sectors - done);
        f.runs[f.nruns] = (lba, n);
        f.nruns += 1;
        done += n;
    }
    Next::Fetch(f)
}

fn run() {
    // SAFETY: this task's own handle, kept for `tick`'s wake-ups.
    TASK.store(unsafe { esp_idf_sys::xTaskGetCurrentTaskHandle() } as *mut core::ffi::c_void, SeqCst);
    let mut scratch: Option<Vec<u8>> = None;
    // SAFETY: the running task's own stack mark.
    let stack_low = || unsafe { esp_idf_sys::uxTaskGetStackHighWaterMark(core::ptr::null_mut()) } as u32;
    loop {
        match plan() {
            Next::Idle => {
                // Nothing followed: the block back, and wait for `tick`
                // (a wake given before this wait is kept by the count).
                scratch = None;
                // SAFETY: a wait on this task's own notification.
                unsafe { esp_idf_sys::ulTaskGenericNotifyTake(0, 1, u32::MAX) };
            }
            Next::Wait => std::thread::sleep(Duration::from_millis(5)),
            Next::NoHeap => std::thread::sleep(Duration::from_millis(200)),
            Next::Fetch(f) => {
                let buf = scratch.get_or_insert_with(|| vec![0u8; BLOCK]);
                let mut at = 0usize;
                let mut ok = true;
                for &(lba, n) in &f.runs[..f.nruns] {
                    let bytes = n as usize * SS;
                    // SAFETY: `bytes` of `buf` from `at` (the block's
                    // sectors, at most BLOCK); the mounted card's drive.
                    if unsafe { esp_idf_sys::ff_disk_read(f.pdrv, buf[at..at + bytes].as_mut_ptr(), lba, n) } != 0 {
                        ok = false;
                        break;
                    }
                    at += bytes;
                }
                if !ok {
                    // The card's error: the display core's own read meets it
                    // (and counts it); this waits a little.
                    ERRORS.fetch_add(1, Relaxed);
                    std::thread::sleep(Duration::from_millis(100));
                    continue;
                }
                let mut s = locked();
                if s.gen == f.gen && s.slot.is_some() {
                    let pos = s.pos;
                    if let Some(r) = s.ring.as_mut() {
                        if r.put(f.off, &buf[..f.len], pos) {
                            s.primed = true;
                            FETCHES.fetch_add(1, Relaxed);
                        }
                    }
                }
            }
        }
        STACK_LOW.fetch_min(stack_low(), Relaxed);
    }
}

/// `/api/status`'s card part: `,"readahead":{...}`.
pub fn status_json() -> String {
    let active = ACTIVE.load(Relaxed);
    let fill = match STATE.try_lock() {
        Ok(s) if CLOSE_PENDING.load(SeqCst) == 0 => s.ring.as_ref().map_or(0, |r| r.ahead(s.pos)),
        _ => FILL.load(Relaxed) as u64,
    };
    let fill_min = FILL_MIN.swap(u32::MAX, Relaxed);
    format!(
        r#","readahead":{{"active":{active},"bytes":{},"fill":{fill},"fill_min":{},"hits":{},"underruns":{},"fetches":{},"taken":{},"no_heap":{},"busy":{},"errors":{},"task_starts":{},"wakes":{},"stack_unused":{}}}"#,
        BLOCKS * BLOCK,
        if fill_min == u32::MAX { "null".to_string() } else { fill_min.to_string() },
        HITS.load(Relaxed),
        UNDERRUNS.load(Relaxed),
        FETCHES.load(Relaxed),
        TAKEN.load(Relaxed),
        NO_HEAP.load(Relaxed),
        BUSY.load(Relaxed),
        ERRORS.load(Relaxed),
        SPAWNS.load(Relaxed),
        WAKES.load(Relaxed),
        match STACK_LOW.load(Relaxed) {
            u32::MAX => "null".to_string(),
            n => n.to_string(),
        }
    )
}

/// The ring's state for the probe: (held now, hits, underruns, fetches).
#[cfg(feature = "psi-probe")]
pub fn probe_running() -> bool {
    RUNNING.load(Relaxed)
}

#[cfg(feature = "psi-probe")]
pub fn probe_counts() -> (bool, u32, u32, u32) {
    let held = STATE.lock().map(|s| s.ring.is_some()).unwrap_or(false);
    (held, HITS.load(Relaxed), UNDERRUNS.load(Relaxed), FETCHES.load(Relaxed))
}

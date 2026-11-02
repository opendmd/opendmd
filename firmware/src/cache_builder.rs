//! The playback cache, prepared by the device itself while it is idle.
//!
//! A prepared copy (`docs/cache-format.md`: a GIF with its LZW capped at 9
//! bits, delta frames and the `OPENDMD1LZW` hint) plays with ~16 KB of
//! decoder heap and reads ~40% fewer bytes than its original. The browser
//! can make copies ("Prepare remaining files"), but a user should not have
//! to: this task walks the playlist in play and makes the missing ones with
//! [`gifslim::recode_gif_streaming`] — byte-identical to what the browser and
//! `opendmd-pack` write — when the panel can spare it.
//!
//! # When it runs: the pace
//!
//! The main task decides, every loop iteration, and publishes a [`Pace`]
//! here ([`publish`]):
//!
//! - **Full** — night mode has the panel dark, the player is paused, or there
//!   is no playlist to play: nothing is animating, so work as fast as the
//!   card allows (a 1 ms sleep every ~20 ms of work so IDLE0 still runs).
//! - **Drip** — `auto_prepare = "always"` (opt-in) and a *prepared*
//!   file is on screen (an original's decoder and the encoder do not both
//!   fit: `opendmd_core::auto_prepare`, builder-a), no transition running,
//!   not the new file's first 500 ms, and no hold after a late frame
//!   (`opendmd_core::auto_prepare::DripGuard`):
//!   [`config::BUILDER_DRIP_PAUSE`] of sleep before every 2 KB of card I/O
//!   and every `stat`, so the player's reads rarely wait for the FAT lock.
//! - **Stop(reason)** — anything else (`auto_prepare = "off"`, `idle` and
//!   playing, an original on screen, a GIF change, a late-frame hold, a
//!   playlist rebuild, and —
//!   set from [`crate::sd::SdSource::open`] — the player opening its next
//!   file).
//!
//! On top of the pace, a transfer in flight ([`busy`]: a stream-task file
//! GET, an upload, a listing) stops it, and so does free heap under
//! [`config::BUILDER_MIN_HEAP`] (at the full pace
//! [`config::BUILDER_MIN_HEAP_FULL`]) before a file (or under
//! [`config::BUILDER_ABORT_HEAP`] during one).
//!
//! **A stop abandons the file in hand and frees everything** — the encoder
//! (see [`config::BUILDER_ENCODER_PEAK`] for its size), which the player needs
//! back the moment its next file turns out to be an original. The check runs before
//! every frame and every 2 KB of card I/O, and the player's open *waits* for
//! the release ([`player_opening`], bounded by
//! [`config::BUILDER_RELEASE_WAIT`]); the time from request to release is in
//! the heartbeat and in the abandon log line. The file is started again from
//! the beginning next time — or, after [`config::BUILDER_DRIP_TRIES`] tries
//! at the drip pace, left for the next full-speed pass.
//!
//! **The task only exists while it may work** (~9.4 KB of stack and TCB): any
//! stop ends it, the player's open waits for it to be gone, and the main loop
//! starts a new one when the pace allows ([`publish`], at most once a second).
//! Its place in the playlist survives in [`WALK`].
//!
//! # The work list
//!
//! The active manifest, in order, read a line at a time (no list in RAM):
//! each `/gifs/…gif` entry that passes the playlist's platform filter, whose
//! copy is not at `layout::cache_path` (one `stat` in a small bucket
//! directory) and which has not failed before. Failures (a GIF that will not
//! decode, a missing file) are appended to [`config::BUILDER_FAILED_PATH`],
//! `path<TAB>reason` a line, and skipped by every later pass. So is a GIF
//! with more than 256 colours, with [`gifslim::ORIGINAL_REASON`]: it gets no
//! copy anywhere (copies are exact or absent) and plays as it is, so it
//! counts as done — in the cache count, not in `failed`. A copy is
//! written to `<copy>.tmp` and renamed into place;
//! the cache count is bumped per file ([`cache::increment_prepared`]) and
//! recounted at the end of a pass. Nothing else persists: after a reboot the
//! walk starts again at the top, skipping what exists.

use opendmd_core::filter::Filter;
use opendmd_core::gifslim;
use opendmd_core::layout::{self, Prepared};
use opendmd_core::manifest::Entry;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering::SeqCst};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};


use crate::cache;
use crate::config;
use crate::state::{free_heap, largest_free_block};

/// Why the builder is not running. The API's `reason` strings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Reason {
    Off = 1,
    Playing = 2,
    /// An original on screen: its decoder and the encoder do not both fit.
    GifOnScreen = 3,
    LowMemory = 4,
    Busy = 5,
    Transition = 6,
    Reindex = 7,
    NoPlaylist = 8,
    /// The player is opening its next file ([`player_opening`]).
    Opening = 9,
    /// Reading the playlist off the card failed; retried after a back-off.
    CardError = 10,
    /// A frame was late while dripping (or at the GIF change that stopped
    /// it): held off for a while (`opendmd_core::auto_prepare::DripGuard`).
    LateFrame = 11,
}

impl Reason {
    fn from_u8(v: u8) -> Option<Reason> {
        Some(match v {
            1 => Reason::Off,
            2 => Reason::Playing,
            3 => Reason::GifOnScreen,
            4 => Reason::LowMemory,
            5 => Reason::Busy,
            6 => Reason::Transition,
            7 => Reason::Reindex,
            8 => Reason::NoPlaylist,
            9 => Reason::Opening,
            10 => Reason::CardError,
            11 => Reason::LateFrame,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Reason::Off => "off",
            // A file open is over in milliseconds; to a user it is playing.
            Reason::Playing | Reason::Opening => "playing",
            Reason::GifOnScreen => "gif on screen",
            Reason::LateFrame => "late frame",
            Reason::LowMemory => "low memory",
            Reason::Busy => "busy",
            Reason::Transition => "transition",
            Reason::Reindex => "reindex",
            Reason::CardError => "card error",
            Reason::NoPlaylist => "no playlist",
        }
    }
}

/// What the main task allows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pace {
    Stop(Reason),
    Drip,
    Full,
}

const PACE_STOP: u8 = 0;
const PACE_DRIP: u8 = 1;
const PACE_FULL: u8 = 2;

/// The pace (`PACE_*`) and, when stopped, the [`Reason`].
static PACE: AtomicU8 = AtomicU8::new(PACE_STOP);
static WHY: AtomicU8 = AtomicU8::new(Reason::Off as u8);
/// The builder has encoder state allocated. Set before it allocates and
/// re-checked against [`PACE`] (both `SeqCst`), so a stop and a start can
/// never both miss each other.
static HOLDING: AtomicBool = AtomicBool::new(false);
/// Transfers in flight ([`busy`]).
static BUSY: AtomicU32 = AtomicU32::new(0);
/// When the last stop, and the last transfer, were asked for (ms since
/// boot): the abandon log line reports how long the builder took to notice.
/// And when the last transfer ended: the builder waits
/// [`config::BUILDER_SETTLE`] after it, so a browser fetching files one after
/// another does not restart (and abandon) a file between each.
static STOP_AT: AtomicU32 = AtomicU32::new(0);
static BUSY_AT: AtomicU32 = AtomicU32::new(0);
static BUSY_END: AtomicU32 = AtomicU32::new(0);
/// The main task's waits for a release in [`player_opening`]: count, worst,
/// and how many gave up.
static RELEASE_WAITS: AtomicU32 = AtomicU32::new(0);
static RELEASE_MAX_US: AtomicU32 = AtomicU32::new(0);
static RELEASE_TIMEOUTS: AtomicU32 = AtomicU32::new(0);
/// `auto_prepare` as the main task last published it (0 idle, 1 always, 2 off).
static MODE: AtomicU8 = AtomicU8::new(0);
/// The least stack the task has had left.
static STACK_LOW: AtomicU32 = AtomicU32::new(u32::MAX);

/// Milliseconds since boot (wraps after 49 days).
fn now_ms() -> u32 {
    // SAFETY: an argument-free read of the IDF's monotonic timer.
    ((unsafe { esp_idf_sys::esp_timer_get_time() }) / 1000) as u32
}

fn ms_since(stamp: u32) -> u32 {
    now_ms().wrapping_sub(stamp)
}

/// The main task's decision for this iteration, and the configured mode
/// (for the API). Cheap: two or three atomic stores.
pub fn publish(pace: Pace, mode: crate::nvs_config::AutoPrepare) {
    MODE.store(mode as u8, SeqCst);
    let (p, why) = match pace {
        Pace::Stop(r) => (PACE_STOP, r as u8),
        Pace::Drip => (PACE_DRIP, 0),
        Pace::Full => (PACE_FULL, 0),
    };
    if PACE.swap(p, SeqCst) != PACE_STOP && p == PACE_STOP {
        STOP_AT.store(now_ms(), SeqCst);
    }
    WHY.store(why, SeqCst);
    if p != PACE_STOP {
        maybe_start();
    }
}

/// The player is about to open a file (called by `SdSource::open`, on the
/// main task): stop the builder and wait until it has let go of its heap,
/// because the file may be a GIF whose decoder needs it. Bounded by
/// [`config::BUILDER_RELEASE_WAIT`]; normally one frame or one 2 KB chunk.
pub fn player_opening() {
    publish(Pace::Stop(Reason::Opening), mode());
    if !ALIVE.load(SeqCst) {
        return;
    }
    let t = Instant::now();
    // The encoder's heap, which a GIF's decoder may need: wait for it. The
    // task runs at priority 1 on CPU0, under everything that answers the
    // network, so it is lifted to their level until it lets go — otherwise
    // a busy CPU0 (the cache count, a burst of requests) can keep it from
    // noticing for as long as that lasts.
    if HOLDING.load(SeqCst) {
        set_priority(config::BUILDER_RELEASE_PRIORITY);
        while HOLDING.load(SeqCst) {
            if t.elapsed() >= config::BUILDER_RELEASE_WAIT {
                RELEASE_TIMEOUTS.fetch_add(1, SeqCst);
                break;
            }
            thread::sleep(Duration::from_millis(1));
        }
    }
    // Then, briefly, its stack: with the task's 8.9 KB still taken, the
    // transition into a GIF (24 KB + the decoder's first 34 KB, 12 KB to
    // spare) is cut. Not worth more than a few ticks of the frame loop.
    let t2 = Instant::now();
    while ALIVE.load(SeqCst) && t2.elapsed() < config::BUILDER_EXIT_WAIT {
        thread::sleep(Duration::from_millis(1));
    }
    if !ALIVE.load(SeqCst) {
        // One tick for IDLE0 to reap the task and free its stack.
        thread::sleep(Duration::from_millis(1));
    }
    RELEASE_WAITS.fetch_add(1, SeqCst);
    RELEASE_MAX_US.fetch_max(t.elapsed().as_micros() as u32, SeqCst);
}

/// Set by [`quiesce`]: the device is about to restart, and the builder must
/// not start (or carry on with) a file.
static SHUTTING_DOWN: AtomicBool = AtomicBool::new(false);

/// Before a restart (`POST /api/reboot`, OTA, the fresh-heap reboot): stop
/// the builder for good and wait, up to `timeout`, for its task to be gone
/// — which means the file in hand was either finished (synced, renamed) or
/// abandoned (its `.tmp` removed). A reset in the middle of a card write is
/// what can leave a card that times out at the next mount (see
/// `sd::SdBus`). True if nothing is running any more.
pub fn quiesce(timeout: Duration) -> bool {
    SHUTTING_DOWN.store(true, SeqCst);
    STOP_AT.store(now_ms(), SeqCst);
    if !ALIVE.load(SeqCst) {
        return true;
    }
    set_priority(config::BUILDER_RELEASE_PRIORITY);
    let t = Instant::now();
    while ALIVE.load(SeqCst) && t.elapsed() < timeout {
        thread::sleep(Duration::from_millis(5));
    }
    let gone = !ALIVE.load(SeqCst);
    log::info!(
        "cache builder: {} before the restart ({} ms)",
        if gone { "stopped" } else { "still running" },
        t.elapsed().as_millis()
    );
    gone
}

/// The abandon line of the last task, for the next one to print.
static DEFERRED_LOG: Mutex<Option<String>> = Mutex::new(None);

/// The running task's FreeRTOS handle (0: none), cleared under the lock
/// before the task ends so [`set_priority`] never touches a dead one.
static HANDLE: Mutex<usize> = Mutex::new(0);

fn set_priority(priority: u8) {
    let h = HANDLE.lock().unwrap_or_else(|p| p.into_inner());
    if *h != 0 {
        // SAFETY: the handle is the live task's (it clears it, under this
        // lock, before it returns), and changing a priority is always safe.
        unsafe {
            esp_idf_sys::vTaskPrioritySet(*h as esp_idf_sys::TaskHandle_t, u32::from(priority))
        };
    }
}

fn mode() -> crate::nvs_config::AutoPrepare {
    crate::nvs_config::AutoPrepare::from_u8(MODE.load(SeqCst))
}

/// Held for the length of a transfer that shares the card with the player
/// (a streamed file, an upload, a listing): the builder stops while any is.
pub struct Busy;

pub fn busy() -> Busy {
    if BUSY.fetch_add(1, SeqCst) == 0 {
        BUSY_AT.store(now_ms(), SeqCst);
    }
    Busy
}

impl Drop for Busy {
    fn drop(&mut self) {
        BUSY_END.store(now_ms(), SeqCst);
        BUSY.fetch_sub(1, SeqCst);
    }
}

/// When the task last took a drip-paced step (ms since boot; 0 = never).
static DRIP_AT: AtomicU32 = AtomicU32::new(0);

/// A late frame now may be the drip's doing (`main`'s `DripGuard`): the
/// task is up at the drip pace, or took a drip step within `window` — the
/// GIF change that stopped it (the open waited for it to let go, and the
/// next file was not opened ahead while it was up).
pub fn dripped_within(window: Duration) -> bool {
    (ALIVE.load(SeqCst) && PACE.load(SeqCst) == PACE_DRIP)
        || (DRIP_AT.load(SeqCst) != 0 && ms_since(DRIP_AT.load(SeqCst)) < window.as_millis() as u32)
}

/// Drip holds after late frames since boot (the heartbeat's `late holds`).
static LATE_HOLDS: AtomicU32 = AtomicU32::new(0);

pub fn note_late_hold() {
    LATE_HOLDS.fetch_add(1, SeqCst);
}

/// A transfer that shares the card is in flight (a [`Busy`] is held): the
/// player does not open the next file ahead meanwhile (`main.rs`), as its
/// directory scans would queue behind the transfer's.
pub fn transfer_in_flight() -> bool {
    BUSY.load(SeqCst) > 0
}

/// Why the builder may not work right now, if it may not.
fn blocked() -> Option<Reason> {
    if SHUTTING_DOWN.load(SeqCst) {
        return Some(Reason::Busy);
    }
    if PACE.load(SeqCst) == PACE_STOP {
        return Some(Reason::from_u8(WHY.load(SeqCst)).unwrap_or(Reason::Off));
    }
    let settling = BUSY_END.load(SeqCst) != 0
        && ms_since(BUSY_END.load(SeqCst)) < config::BUILDER_SETTLE.as_millis() as u32;
    (BUSY.load(SeqCst) > 0 || settling).then_some(Reason::Busy)
}

// ---------------------------------------------------------------------------
// What the API and the heartbeat report
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Idle,
    Running,
    Paused,
    Done,
}

impl State {
    fn as_str(self) -> &'static str {
        match self {
            State::Idle => "idle",
            State::Running => "running",
            State::Paused => "paused",
            State::Done => "done",
        }
    }
}

struct Report {
    state: State,
    reason: Option<Reason>,
    /// The pace of the file in hand (`"full"`/`"drip"`).
    policy: Option<&'static str>,
    session: u32,
    failed: u32,
    aborted: u32,
    full: (f32, u32),
    drip: (f32, u32),
    /// The GIF being prepared.
    current: Option<String>,
    /// Bytes written to the card this session, and the seconds spent
    /// encoding (for the throughput).
    written: u64,
    busy_secs: f32,
}

static REPORT: Mutex<Report> = Mutex::new(Report {
    state: State::Idle,
    reason: Some(Reason::Off),
    policy: None,
    session: 0,
    failed: 0,
    aborted: 0,
    full: (0.0, 0),
    drip: (0.0, 0),
    current: None,
    written: 0,
    busy_secs: 0.0,
});

fn report() -> std::sync::MutexGuard<'static, Report> {
    REPORT.lock().unwrap_or_else(|p| p.into_inner())
}

fn set_state(state: State, reason: Option<Reason>) {
    let mut r = report();
    r.state = state;
    r.reason = reason;
    if state != State::Running {
        r.policy = None;
        r.current = None;
    }
}

/// The state to report: the task's own while it runs; otherwise what the
/// pace and the walk say (the task is started again when there is work).
fn view(r: &Report) -> (State, Option<Reason>) {
    if ALIVE.load(SeqCst) {
        return (r.state, r.reason);
    }
    let done = WALK
        .try_lock()
        .is_ok_and(|w| w.as_ref().is_some_and(Walk::finished));
    match blocked() {
        Some(why @ (Reason::Off | Reason::NoPlaylist)) => (State::Idle, Some(why)),
        _ if done => (State::Done, None),
        Some(why) => (State::Paused, Some(why)),
        None if (free_heap() as usize) < spawn_heap() => (State::Paused, Some(Reason::LowMemory)),
        None => (State::Paused, None),
    }
}

fn avg((sum, n): (f32, u32)) -> Option<f32> {
    (n > 0).then(|| sum / n as f32)
}

fn opt_f(v: Option<f32>) -> String {
    v.map_or_else(|| "null".to_string(), |v| format!("{v:.1}"))
}

/// `/api/cache`'s `auto` object. `total`/`prepared` are the count's.
pub fn auto_json(total: usize, prepared: Option<usize>) -> String {
    let r = report();
    let full = avg(r.full);
    let eta = match (prepared, full) {
        (Some(p), Some(a)) => {
            let left = total.saturating_sub(p + r.failed as usize);
            format!("{:.0}", left as f32 * a)
        }
        _ => "null".to_string(),
    };
    let (state, reason) = view(&r);
    format!(
        r#"{{"mode":"{}","state":"{}","reason":{},"policy":{},"session":{},"failed":{},"avg_full_s":{},"avg_drip_s":{},"eta_s":{eta},"current":{}}}"#,
        mode().as_str(),
        state.as_str(),
        reason.map_or_else(|| "null".to_string(), |x| format!("\"{}\"", x.as_str())),
        r.policy
            .map_or_else(|| "null".to_string(), |p| format!("\"{p}\"")),
        r.session,
        r.failed,
        opt_f(full),
        opt_f(avg(r.drip)),
        r.current
            .as_deref()
            .map_or_else(|| "null".to_string(), crate::httpd::json_string),
    )
}

/// `/api/status`'s `cache` object.
pub fn status_json(total: usize, prepared: Option<usize>) -> String {
    format!(
        r#"{{"state":"{}","prepared":{},"total":{total}}}"#,
        view(&report()).0.as_str(),
        prepared.map_or_else(|| "null".to_string(), |p| p.to_string())
    )
}

/// The heartbeat's part: `builder: <state> <n>/<total> <avg s/file> (…)`.
pub fn heartbeat(total: usize, prepared: Option<usize>) -> String {
    let r = report();
    let (state, reason) = view(&r);
    let pace = match state {
        State::Running => r.policy.unwrap_or("?").to_string(),
        _ => reason.map_or_else(String::new, |x| x.as_str().to_string()),
    };
    let per = match (r.policy, avg(r.full), avg(r.drip)) {
        (Some("drip"), _, Some(d)) => format!("{d:.1} s/file"),
        (_, Some(f), _) => format!("{f:.1} s/file"),
        (_, None, Some(d)) => format!("{d:.1} s/file (drip)"),
        _ => "- s/file".to_string(),
    };
    let rate = if r.busy_secs > 0.0 {
        r.written as f32 / 1024.0 / r.busy_secs
    } else {
        0.0
    };
    let waits = RELEASE_WAITS.load(SeqCst);
    format!(
        ", builder: {} {pace} {}/{total} {per} (session {}, failed {}, abandoned {}, {:.0} KB written at {rate:.0} KB/s, \
         release waits {waits} max {:.1} ms timeouts {}, late holds {}, stack {} B unused, task {})",
        state.as_str(),
        prepared.map_or_else(|| "?".to_string(), |p| p.to_string()),
        r.session,
        r.failed,
        r.aborted,
        r.written as f32 / 1024.0,
        RELEASE_MAX_US.swap(0, SeqCst) as f32 / 1000.0,
        RELEASE_TIMEOUTS.load(SeqCst),
        LATE_HOLDS.load(SeqCst),
        match STACK_LOW.load(SeqCst) {
            u32::MAX => "-".to_string(),
            n => n.to_string(),
        },
        if ALIVE.load(SeqCst) {
            format!("up, {} starts", SPAWNS.load(SeqCst))
        } else {
            format!("down, {} starts", SPAWNS.load(SeqCst))
        },
    )
}

// ---------------------------------------------------------------------------
// The task
// ---------------------------------------------------------------------------

/// Start the task, pinned to CPU0 below the stream task. Called from `main`
/// before WiFi, while the heap still has a contiguous block for the stack.
fn spawn() -> anyhow::Result<()> {
    let before = free_heap();
    crate::task::spawn_core0(b"cache-builder\0", config::BUILDER_STACK, config::BUILDER_PRIORITY, Box::new(run))?;
    // Logged by the task itself: this runs inside the main task's frame.
    SPAWN_COST.store(before.saturating_sub(free_heap()), SeqCst);
    Ok(())
}

/// The task exists. It exits as soon as there is nothing it may do, giving
/// its stack back, and [`publish`] starts it again when there is.
static ALIVE: AtomicBool = AtomicBool::new(false);
/// The task exists (for attributing a heap dip seen elsewhere).
pub fn task_up() -> bool {
    ALIVE.load(SeqCst)
}

/// Heap the last start cost (logged by the task).
static SPAWN_COST: AtomicU32 = AtomicU32::new(0);
/// When a start was last tried (ms since boot), to space out attempts.
static SPAWN_AT: AtomicU32 = AtomicU32::new(0);
static SPAWNS: AtomicU32 = AtomicU32::new(0);
/// The walk survives the task: a restart carries on where it stopped.
static WALK: Mutex<Option<Walk>> = Mutex::new(None);
/// A count has been asked for (once per boot).
static COUNTED: AtomicBool = AtomicBool::new(false);
/// The failure file was cleared ([`clear_failed`]): the walk forgets its
/// copy of the list at its next step.
static FAILED_CLEARED: AtomicBool = AtomicBool::new(false);
/// GIFs added to the failure file from outside ([`add_failed`]), as
/// `layout::prepared_key`s: the walk takes them into its list at its next
/// step.
/// The flag: the line counts as a failure (not a GIF that plays as it is,
/// nor a renamed line already counted).
static FAILED_ADDED: Mutex<Vec<(u32, bool)>> = Mutex::new(Vec::new());
/// Held for every read-modify-write of the failure file: the builder
/// appends from its task, `POST /api/cache/failed` from httpd's, and FATFS
/// (built without `FF_FS_LOCK`) does not keep two appenders apart.
static FAILED_FILE: Mutex<()> = Mutex::new(());

/// Free heap needed to start a file, the task running: less at the full
/// pace, when nothing animates ([`config::BUILDER_MIN_HEAP_FULL`]).
fn min_heap() -> u32 {
    if PACE.load(SeqCst) == PACE_FULL {
        config::BUILDER_MIN_HEAP_FULL
    } else {
        config::BUILDER_MIN_HEAP
    }
}

/// Free heap needed to start the task: [`min_heap`] for a file once it
/// runs, plus its stack and ~1 KB of TCB and pthread bookkeeping (the task
/// costs ~9.4 KB).
fn spawn_heap() -> usize {
    min_heap() as usize + config::BUILDER_STACK + 1024
}

/// Start the task if the pace allows work, it is not running, and there may
/// be something to do. Called with every [`publish`] (main task): cheap
/// unless it actually starts one, and attempts are at least
/// [`config::BUILDER_RESPAWN`] apart.
fn maybe_start() {
    if ALIVE.load(SeqCst) || blocked().is_some() {
        return;
    }
    if ms_since(SPAWN_AT.load(SeqCst)) < config::BUILDER_RESPAWN.as_millis() as u32 {
        return;
    }
    SPAWN_AT.store(now_ms(), SeqCst);
    // Finished, and nothing has changed since: no need.
    let done = WALK
        .try_lock()
        .is_ok_and(|w| w.as_ref().is_some_and(Walk::finished));
    // Room for the task's stack and, after it, for an encoder.
    if done || (free_heap() as usize) < spawn_heap() {
        return;
    }
    ALIVE.store(true, SeqCst);
    if spawn().is_err() {
        ALIVE.store(false, SeqCst);
    }
}

/// Where the walk is in the playlist.
struct Walk {
    manifest: String,
    platforms: String,
    filter: Filter,
    /// Byte offset of the next line to look at.
    offset: u64,
    /// `layout::prepared_key` of every GIF in the failure file.
    failed: Vec<u32>,
    /// How many of `failed` are not failures: GIFs that play as they are
    /// ([`gifslim::plays_original`]), and renamed lines. `failed` in the
    /// API is the rest.
    originals: usize,
    /// The pass reached the end; the cache generation it recounted at.
    done_gen: Option<u32>,
    /// Files abandoned [`config::BUILDER_DRIP_TRIES`] times at the drip pace
    /// (too big for one file's turn on screen): skipped while dripping and
    /// left for the next full-speed pass.
    deferred: Vec<u32>,
    /// The file last abandoned at the drip pace, and how many times.
    tries: (u32, u32),
    /// Copies found already there, and made, in this pass.
    found: u32,
    made: u32,
    started: Instant,
}

impl Walk {
    /// The pass is over and nothing it could still do has changed: the
    /// copies and playlist are as they were, and nothing deferred waits for
    /// a full-speed pace that has now come.
    fn finished(&self) -> bool {
        self.done_gen == Some(cache::generation())
            && (self.deferred.is_empty() || PACE.load(SeqCst) != PACE_FULL)
    }

    fn new(manifest: String, platforms: String) -> Walk {
        let list: Vec<&str> = platforms
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();
        let filter = if list.is_empty() {
            Filter::new()
        } else {
            Filter::new().platforms(list)
        };
        let (failed, originals) = load_failed();
        report().failed = (failed.len() - originals) as u32;
        log::info!(
            "cache builder: new pass over {manifest}{}, {} known failures",
            if platforms.is_empty() {
                String::new()
            } else {
                format!(" (platforms {platforms})")
            },
            failed.len()
        );
        Walk {
            manifest,
            platforms,
            filter,
            offset: 0,
            failed,
            originals,
            done_gen: None,
            deferred: Vec::new(),
            tries: (0, 0),
            found: 0,
            made: 0,
            started: Instant::now(),
        }
    }
}

fn run() {
    if let Some(line) = DEFERRED_LOG
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .take()
    {
        log::info!("{line}");
    }
    log::info!(
        "cache builder: task started on CPU0 ({} B stack, priority {}, cost {} B of heap; start #{})",
        config::BUILDER_STACK,
        config::BUILDER_PRIORITY,
        SPAWN_COST.load(SeqCst),
        SPAWNS.fetch_add(1, SeqCst) + 1
    );
    // SAFETY: an argument-free query about the calling task.
    *HANDLE.lock().unwrap_or_else(|p| p.into_inner()) =
        unsafe { esp_idf_sys::xTaskGetCurrentTaskHandle() } as usize;
    let mut walk = WALK.lock().unwrap_or_else(|p| p.into_inner()).take();
    let (state, why) = work(&mut walk);
    set_state(state, why);
    *WALK.lock().unwrap_or_else(|p| p.into_inner()) = walk;
    httpd_stack_low_water();
    *HANDLE.lock().unwrap_or_else(|p| p.into_inner()) = 0;
    // The stack goes back as soon as IDLE0 reaps the task; `publish` starts
    // another when there is work it may do. Nothing is logged here: the
    // player may be waiting for this ([`player_opening`]).
    ALIVE.store(false, SeqCst);
}

/// Prepare files until there is nothing the builder may do; why it stopped.
/// The task never waits around holding its stack: any stop ends it.
fn work(walk: &mut Option<Walk>) -> (State, Option<Reason>) {
    // Since when this task has been waiting for a cache count (step 4).
    let mut count_wait: Option<Instant> = None;
    loop {
        httpd_stack_low_water();
        // 1. Allowed at all?
        if let Some(why) = blocked() {
            return match why {
                Reason::Off | Reason::NoPlaylist => (State::Idle, Some(why)),
                _ if walk.as_ref().is_some_and(Walk::finished) => (State::Done, None),
                _ => (State::Paused, Some(why)),
            };
        }
        // 2. The playlist in play; a new one starts a new pass.
        let (manifest, platforms) = cache::playlist();
        if manifest.is_empty() {
            return (State::Idle, Some(Reason::NoPlaylist));
        }
        if !COUNTED.swap(true, SeqCst) {
            // The ETA and the card's sentence want a count: ask for one
            // (it runs on the network worker) the first time there is work.
            let _ = cache::snapshot();
        }
        let cleared = FAILED_CLEARED.swap(false, SeqCst);
        let w = match walk {
            Some(w) if w.manifest == manifest && w.platforms == platforms => w,
            _ => walk.insert(Walk::new(manifest, platforms)),
        };
        if cleared {
            // Retried where the walk meets them next: later in this pass,
            // or in the next one (the recount `clear_failed` asked for ends
            // a finished pass).
            w.failed.clear();
            w.originals = 0;
            report().failed = 0;
        }
        let added = std::mem::take(&mut *FAILED_ADDED.lock().unwrap_or_else(|p| p.into_inner()));
        if !added.is_empty() {
            for (key, counts) in added {
                if !w.failed.contains(&key) && w.failed.len() < config::BUILDER_MAX_FAILED {
                    w.failed.push(key);
                    w.originals += usize::from(!counts);
                }
            }
            report().failed = (w.failed.len() - w.originals) as u32;
        }
        // 3. Done: over until the copies or the playlist change.
        if w.done_gen.is_some() {
            if w.finished() {
                return (State::Done, None);
            }
            *w = Walk::new(w.manifest.clone(), w.platforms.clone());
        }
        // 4. Not while the `/api/cache` count runs (on the network worker,
        //    which this very loop asks for at its first file). The count
        //    sizes its key slice (up to 12 KB) to the heap free when it
        //    starts, so an encoder starting beside it took the rig's heap to
        //    5,284 B (largest block 3 KB: the night floors of ~6 KB); and
        //    reading the manifest flat out at the worker's priority it holds
        //    the FAT lock so much that this task's reads waited out FATFS's
        //    10 s lock timeout (`ETIMEDOUT`, "card error ... backing off").
        //    A count is seconds (a full card, ~25 s); wait for it — up to
        //    [`config::BUILDER_COUNT_WAIT`], in case the worker is held up
        //    elsewhere (a WiFi reconnect) and the count never comes.
        if cache::counting() {
            let since = *count_wait.get_or_insert_with(Instant::now);
            if since.elapsed() < config::BUILDER_COUNT_WAIT {
                thread::sleep(Duration::from_millis(250));
                continue;
            }
        } else {
            count_wait = None;
        }
        // 5. Heap for a whole encoder, with the player's margin left.
        if free_heap() < min_heap() || largest_free_block() < 16 * 1024 {
            return (State::Paused, Some(Reason::LowMemory));
        }
        // 6. The next file that needs a copy.
        match next_candidate(w) {
            Ok(Some((line_at, next_at, gif, copy))) => {
                match prepare(&gif, &copy) {
                    Outcome::Made => {
                        w.made += 1;
                        w.offset = next_at;
                    }
                    Outcome::Failed(why) => {
                        record_failure(&gif, &why);
                        w.failed.push(layout::prepared_key(&gif));
                        if gifslim::plays_original(&why) {
                            // Done: it plays as it is.
                            w.originals += 1;
                            cache::increment_prepared();
                        }
                        report().failed = (w.failed.len() - w.originals) as u32;
                        w.offset = next_at;
                    }
                    // Stopped part way: the same file is next again —
                    // unless it keeps missing its window at the drip pace.
                    Outcome::Abandoned { drip } => {
                        let key = layout::prepared_key(&gif);
                        let n = if drip && w.tries.0 == key {
                            w.tries.1 + 1
                        } else {
                            u32::from(drip)
                        };
                        w.tries = (key, n);
                        if n >= config::BUILDER_DRIP_TRIES {
                            if let Some(line) = DEFERRED_LOG
                                .lock()
                                .unwrap_or_else(|p| p.into_inner())
                                .as_mut()
                            {
                                line.push_str(&format!(
                                    "; abandoned {n} times at the drip pace, left for full speed"
                                ));
                            }
                            w.deferred.push(key);
                            w.offset = next_at;
                        } else {
                            w.offset = line_at;
                        }
                    }
                    Outcome::NotStarted => w.offset = line_at,
                    Outcome::CardError => {
                        w.offset = line_at;
                        thread::sleep(config::BUILDER_CARD_BACKOFF);
                    }
                }
            }
            Ok(None) => {
                // End of the playlist: recount once, and remember the
                // generation *after* our own invalidation.
                cache::invalidate();
                w.done_gen = Some(cache::generation());
                let r = report();
                log::info!(
                    "cache builder: pass over {} done in {} s: {} made, {} already there, {} left for full speed, {} known failures; session {} made, {:.0} KB written",
                    w.manifest,
                    w.started.elapsed().as_secs(),
                    w.made,
                    w.found,
                    w.deferred.len(),
                    w.failed.len(),
                    r.session,
                    r.written as f32 / 1024.0
                );
                drop(r);
                return (State::Done, None);
            }
            // Paused mid-scan: carry on from the same place next time.
            Err(Stopped) => {}
        }
    }
}

fn httpd_stack_low_water() {
    crate::httpd::stack_low_water("cache-builder", &STACK_LOW);
}

/// The walk stopped because the builder was stopped.
struct Stopped;

/// Wait out the pace before a piece of card work (2 KB of I/O, a `stat`):
/// `Err` if stopped. At the drip pace that is [`config::BUILDER_DRIP_PAUSE`]
/// — the card (FAT lock, SPI bus) is what the builder shares with the
/// player; its CPU is CPU0's, the player's CPU1.
fn pace() -> Result<(), Reason> {
    check()?;
    if PACE.load(SeqCst) == PACE_DRIP {
        // In 1 ms steps, so a stop is seen within a tick, not a pause.
        for _ in 0..config::BUILDER_DRIP_PAUSE.as_millis() {
            if blocked().is_some() {
                break;
            }
            thread::sleep(Duration::from_millis(1));
        }
        LAST_SLEEP.store(now_ms(), SeqCst);
        DRIP_AT.store(now_ms().max(1), SeqCst);
    }
    breathe();
    Ok(())
}

/// Between two frames (CPU work only): `Err` if stopped.
fn check() -> Result<(), Reason> {
    if let Some(why) = blocked() {
        return Err(why);
    }
    if free_heap() < config::BUILDER_ABORT_HEAP {
        return Err(Reason::LowMemory);
    }
    breathe();
    Ok(())
}

static LAST_SLEEP: AtomicU32 = AtomicU32::new(0);

/// One tick of sleep every ~20 ms of work, so IDLE0 (and the task watchdog)
/// get their turn: a yield alone only goes to equal or higher priorities.
fn breathe() {
    if ms_since(LAST_SLEEP.load(SeqCst)) >= 20 {
        thread::sleep(Duration::from_millis(1));
        LAST_SLEEP.store(now_ms(), SeqCst);
    } else {
        thread::yield_now();
    }
}

/// The next playlist entry that needs a copy, from `w.offset` on:
/// `(offset of its line, offset of the line after, GIF path, copy path)`,
/// or `None` at the end. Counts copies it finds already there.
fn next_candidate(w: &mut Walk) -> Result<Option<(u64, u64, String, String)>, Stopped> {
    let mut report_state = true;
    let file = match File::open(&w.manifest) {
        Ok(f) => f,
        Err(e) => {
            log::warn!("cache builder: {}: {e}", w.manifest);
            set_state(State::Idle, Some(Reason::NoPlaylist));
            thread::sleep(config::BUILDER_CARD_BACKOFF);
            return Err(Stopped);
        }
    };
    let mut reader = BufReader::with_capacity(512, file);
    if reader.seek(SeekFrom::Start(w.offset)).is_err() {
        return Ok(None);
    }
    let parse = !w.filter.is_unconstrained();
    let mut line = String::new();
    let mut at = w.offset;
    let mut lines = 0u32;
    loop {
        // Lines that need no `stat` (filtered out, failed, prepared ones are
        // not among them) still cost reads: check every so often.
        lines += 1;
        if lines.is_multiple_of(32) {
            if let Err(why) = pace() {
                w.offset = at;
                set_state(State::Paused, Some(why));
                return Err(Stopped);
            }
        }
        line.clear();
        let n = match reader.read_line(&mut line) {
            Ok(0) => return Ok(None),
            Ok(n) => n as u64,
            Err(e) => {
                // Not the end of the playlist: a card read that failed (seen
                // on the rig: ETIMEDOUT). Taking it for the end finished the
                // pass as "done" with thousands of files left; instead the
                // line is read again after a back-off.
                log::warn!(
                    "cache builder: reading {} at byte {at}: {e}; retrying after {} s",
                    w.manifest,
                    config::BUILDER_CARD_BACKOFF.as_secs()
                );
                w.offset = at;
                set_state(State::Paused, Some(Reason::CardError));
                thread::sleep(config::BUILDER_CARD_BACKOFF);
                return Err(Stopped);
            }
        };
        let (line_at, next_at) = (at, at + n);
        at = next_at;
        w.offset = next_at;
        let path = line
            .strip_prefix('\u{feff}')
            .unwrap_or(&line)
            .trim_end_matches(['\n', '\r', ' ', '\t']);
        if path.is_empty() || (parse && !w.filter.matches(&Entry::from_path(path))) {
            continue;
        }
        let Prepared::Candidate { .. } = layout::classify(path) else {
            continue;
        };
        let key = layout::prepared_key(path);
        if w.failed.contains(&key) || (PACE.load(SeqCst) == PACE_DRIP && w.deferred.contains(&key))
        {
            continue;
        }
        let Some(copy) = layout::cache_path(path) else {
            continue;
        };
        // A `stat` is a directory scan under the FAT lock: paced like I/O.
        if let Err(why) = pace() {
            w.offset = line_at;
            set_state(State::Paused, Some(why));
            return Err(Stopped);
        }
        if report_state {
            report_state = false;
            let mut r = report();
            r.state = State::Running;
            r.reason = None;
        }
        if fs::metadata(format!("{}{copy}", config::SD_MOUNT)).is_ok() {
            w.found += 1;
            continue;
        }
        return Ok(Some((line_at, next_at, path.to_string(), copy)));
    }
}

enum Outcome {
    Made,
    Failed(String),
    Abandoned {
        drip: bool,
    },
    /// Stopped between choosing the file and taking the heap.
    NotStarted,
    CardError,
}

/// A card file read or written through [`pace`]: at most one
/// [`config::IO_CHUNK`] per call, a pace check before each, and the first
/// real I/O error remembered (the decoder turns every error into
/// `InvalidData`, so this is how a card fault is told from a bad GIF).
struct Paced {
    file: File,
    stopped: Option<Reason>,
    io_error: Option<io::Error>,
    bytes: u64,
}

impl Paced {
    fn new(file: File) -> Paced {
        Paced {
            file,
            stopped: None,
            io_error: None,
            bytes: 0,
        }
    }

    fn check(&mut self) -> io::Result<()> {
        pace().map_err(|why| {
            self.stopped = Some(why);
            io::Error::other("stopped")
        })
    }

    fn note<T>(&mut self, r: io::Result<T>) -> io::Result<T> {
        if let Err(e) = &r {
            if self.io_error.is_none() {
                self.io_error = Some(io::Error::new(e.kind(), e.to_string()));
            }
        }
        r
    }
}

impl Read for Paced {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.check()?;
        let n = buf.len().min(config::IO_CHUNK);
        let r = self.file.read(&mut buf[..n]);
        self.note(r)
    }
}

impl Write for Paced {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.check()?;
        let n = buf.len().min(config::IO_CHUNK);
        let r = self.file.write(&buf[..n]);
        let n = self.note(r)?;
        self.bytes += n as u64;
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        let r = self.file.flush();
        self.note(r)
    }
}

impl Seek for Paced {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let r = self.file.seek(pos);
        self.note(r)
    }
}

/// A file is in hand: the encoder's heap is allocated (or about to be). The
/// `/api/cache` count waits for this to clear before it sizes its key slice
/// ([`crate::cache::run_count`]).
pub fn holding() -> bool {
    HOLDING.load(SeqCst)
}

/// Take the heap, or find out we may not after all.
fn hold() -> bool {
    HOLDING.store(true, SeqCst);
    if blocked().is_some() {
        HOLDING.store(false, SeqCst);
        return false;
    }
    true
}

/// Make one copy. Everything allocated here is dropped before `HOLDING` is
/// cleared, whatever the outcome.
fn prepare(gif: &str, copy: &str) -> Outcome {
    if !hold() {
        return Outcome::NotStarted;
    }
    let pace_name = if PACE.load(SeqCst) == PACE_DRIP {
        "drip"
    } else {
        "full"
    };
    {
        let mut r = report();
        r.state = State::Running;
        r.reason = None;
        r.policy = Some(pace_name);
        r.current = Some(gif.to_string());
    }
    let started = Instant::now();
    let tmp = format!("{}{copy}.tmp", config::SD_MOUNT);
    let mut frames = 0u32;
    let outcome = encode_one(gif, copy, &tmp, &mut frames);
    let secs = started.elapsed().as_secs_f32();
    HOLDING.store(false, SeqCst);

    let mut r = report();
    r.current = None;
    match &outcome {
        Ok((gif_bytes, copy_bytes, written)) => {
            r.session += 1;
            r.written += written;
            r.busy_secs += secs;
            let slot = if pace_name == "drip" {
                &mut r.drip
            } else {
                &mut r.full
            };
            slot.0 += secs;
            slot.1 += 1;
            let session = r.session;
            drop(r);
            cache::increment_prepared();
            log::info!(
                "cache builder: [{session}] {gif} prepared ({pace_name}): {frames} frames, {} KB -> {} KB in {:.2} s, heap {} B free",
                gif_bytes / 1024,
                copy_bytes / 1024,
                secs,
                free_heap()
            );
            Outcome::Made
        }
        Err(Fail::Stopped(why)) => {
            r.aborted += 1;
            drop(r);
            let asked = match why {
                Reason::Busy => ms_since(BUSY_AT.load(SeqCst)),
                _ => ms_since(STOP_AT.load(SeqCst)),
            };
            // Printed when the task next starts: the player may be waiting
            // for this one to be gone, and a UART line is ~15 ms.
            *DEFERRED_LOG.lock().unwrap_or_else(|p| p.into_inner()) = Some(format!(
                "cache builder: paused ({}) {asked} ms after it was asked; abandoned {gif} after {frames} frames, {secs:.2} s",
                why.as_str(),
            ));
            set_state(State::Paused, Some(*why));
            Outcome::Abandoned {
                drip: pace_name == "drip",
            }
        }
        Err(Fail::Bad(why)) => {
            drop(r);
            log::warn!("cache builder: {gif} cannot be prepared here: {why}");
            Outcome::Failed(why.clone())
        }
        Err(Fail::Card(e)) => {
            drop(r);
            log::warn!("cache builder: card error on {gif}: {e}; backing off");
            Outcome::CardError
        }
    }
}

enum Fail {
    Stopped(Reason),
    /// The GIF itself: recorded, never retried.
    Bad(String),
    /// The card: not the GIF's fault, retried later.
    Card(String),
}

/// Encode `gif` into `tmp` and rename it to `copy`: `(GIF bytes, copy
/// bytes, bytes written)`. The temp file is removed on every failure.
fn encode_one(gif: &str, copy: &str, tmp: &str, frames: &mut u32) -> Result<(u64, u64, u64), Fail> {
    // Through the FAT index when it has the original: no directory scan.
    let src = match crate::fatidx::open(&format!("{}{gif}", config::SD_MOUNT)) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Err(Fail::Bad("not found".into())),
        Err(e) => return Err(Fail::Card(e.to_string())),
    };
    let mut src = Paced::new(src);
    // The logical screen, before anything is allocated for it: the card has
    // 132x32 and 135x32 strays, nothing bigger, and a huge one would not fit.
    let mut head = [0u8; 10];
    if src.read_exact(&mut head).is_err() || &head[..3] != b"GIF" {
        return Err(match src.stopped {
            Some(why) => Fail::Stopped(why),
            None if src.io_error.is_some() => Fail::Card(format!("{:?}", src.io_error)),
            None => Fail::Bad("not a GIF".into()),
        });
    }
    let (w, h) = (
        u16::from_le_bytes([head[6], head[7]]) as usize,
        u16::from_le_bytes([head[8], head[9]]) as usize,
    );
    if w * h > config::BUILDER_MAX_PIXELS {
        return Err(Fail::Bad(format!("{w}x{h} is too large to prepare here")));
    }
    // Wider than the panel (the card's 132x32 and 135x32 strays): the
    // encoder peaks ~13 KB higher than the start threshold allows for, so
    // such a file is only started with its own peak plus the margin free
    // (~75 KB, which this board never has while the builder runs); else
    // it is left to the browser like the other files too large for here.
    if w * h > 128 * 32
        && free_heap() < config::BUILDER_WIDE_ENCODER_PEAK + config::BUILDER_PEAK_MARGIN
    {
        return Err(Fail::Bad(format!("{w}x{h} is too large to prepare here")));
    }
    if src.seek(SeekFrom::Start(0)).is_err() {
        return Err(Fail::Card("seek".into()));
    }

    if let Some(dir) = std::path::Path::new(tmp).parent() {
        if let Err(e) = fs::create_dir_all(dir) {
            return Err(Fail::Card(format!("creating {}: {e}", dir.display())));
        }
    }
    let out = match OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(tmp)
    {
        Ok(f) => f,
        Err(e) => return Err(Fail::Card(format!("creating {tmp}: {e}"))),
    };
    let mut out = Paced::new(out);
    let result = {
        // The recoder writes a few bytes at a time: one 2 KB card write per
        // buffer-full instead.
        let mut w = BufWriter::with_capacity(config::IO_CHUNK, &mut out);
        // Boxed like the player's files, so the decoders the recoder uses
        // are the very ones playback already links (one copy in the image).
        let source: Box<dyn opendmd_core::ReadSeek + '_> = Box::new(&mut src);
        let r = gifslim::recode_gif_streaming_dyn(
            source,
            &mut w,
            &gifslim::PREPARED,
            // Interlaced frames or `Previous` disposal (5 files on the stock
            // card): the `gif` crate decodes those, only if its heap is
            // free now (see the constant).
            &mut || {
                free_heap() >= config::BUILDER_FULL_DECODER_HEAP
                    && largest_free_block() >= 17 * 1024
            },
            &mut || {
                *frames += 1;
                check().map_err(|why| {
                    OUT_STOP.store(why as u8, SeqCst);
                    io::Error::other("stopped")
                })
            },
        );
        r.and_then(|s| w.flush().map(|()| s))
    };
    // (`frames` counted both passes' steps; the log wants the file's.)
    if let Ok(st) = &result {
        *frames = st.frames;
    }
    // `between`'s stop is kept apart, since `out` is borrowed while it runs.
    let stopped = src
        .stopped
        .or(out.stopped)
        .or(Reason::from_u8(OUT_STOP.swap(0, SeqCst)));
    let failure = match &result {
        Ok(_) => None,
        Err(_) if stopped.is_some() => stopped.map(Fail::Stopped),
        // No copy on purpose: the GIF plays as it is (exactly; a copy
        // would have to be quantised). Recorded so no one tries again.
        Err(e) if gifslim::is_too_many_colours(e) => {
            Some(Fail::Bad(gifslim::ORIGINAL_REASON.into()))
        }
        Err(e) if gifslim::is_unsupported(e) || gifslim::is_needs_full_decoder(e) => {
            Some(Fail::Bad(format!("{e} (too large to prepare here)")))
        }
        Err(_) if out.io_error.is_some() || src.io_error.is_some() => {
            let e = out.io_error.take().or(src.io_error.take());
            Some(Fail::Card(format!("{e:?}")))
        }
        Err(e) => Some(Fail::Bad(e.to_string())),
    };
    let synced = match &result {
        // `f_sync`: the data and the FAT on the card before the rename, so a
        // reset afterwards cannot leave a copy that is half there.
        Ok(_) if failure.is_none() => out.file.sync_all(),
        _ => Ok(()),
    };
    let written = out.bytes;
    // Closed before anything is removed or renamed: FAT must never have an
    // open file unlinked under it.
    drop(out);
    drop(src);
    let fail = |f: Fail| {
        let _ = fs::remove_file(tmp);
        Err(f)
    };
    if let Some(f) = failure {
        return fail(f);
    }
    let Ok(stats) = result else {
        return fail(Fail::Bad("encode failed".into()));
    };
    if let Err(e) = synced {
        return fail(Fail::Card(format!("syncing {tmp}: {e}")));
    }
    let dest = format!("{}{copy}", config::SD_MOUNT);
    if let Err(e) = fs::rename(tmp, &dest) {
        // Someone else (the browser) put a copy there meanwhile: keep theirs.
        if fs::metadata(&dest).is_ok() {
            let _ = fs::remove_file(tmp);
        } else {
            return fail(Fail::Card(format!("rename to {dest}: {e}")));
        }
    }
    // Its entry, for the index's lookups (the player opens copies).
    crate::fatidx::note_written(&dest);
    Ok((stats.gif_bytes, stats.out_bytes, written))
}

static OUT_STOP: AtomicU8 = AtomicU8::new(0);

/// The failure file's GIFs, as `layout::prepared_key`s, and how many of
/// them play as they are ([`gifslim::plays_original`]).
fn load_failed() -> (Vec<u32>, usize) {
    let Ok(f) = File::open(config::BUILDER_FAILED_PATH) else {
        return (Vec::new(), 0);
    };
    let (mut keys, mut originals) = (Vec::new(), 0);
    for line in BufReader::with_capacity(512, f).lines() {
        let Ok(line) = line else { break };
        let (path, reason) = line.split_once('\t').unwrap_or((&line, ""));
        let path = path.trim();
        if !path.is_empty() && keys.len() < config::BUILDER_MAX_FAILED {
            keys.push(layout::prepared_key(path));
            originals += usize::from(gifslim::plays_original(reason));
        }
    }
    (keys, originals)
}

/// `GET /api/cache/failed`: the failure file as
/// `[{"path":"/gifs/…","reason":"…"}]`, a line at a time (`[]` if there is
/// none). The file holds at most [`config::BUILDER_MAX_FAILED`] paths that
/// count, but is streamed whatever its length.
pub fn stream_failed(req: &mut crate::httpd::Req) -> anyhow::Result<()> {
    use crate::httpd::json_string;
    req.header("Cache-Control", "no-cache")?;
    req.begin_chunked("200 OK", "application/json")?;
    let mut out = String::with_capacity(config::IO_CHUNK + 256);
    out.push('[');
    let mut n = 0usize;
    if let Ok(f) = File::open(config::BUILDER_FAILED_PATH) {
        for line in BufReader::with_capacity(512, f).lines() {
            let line = line?;
            let (path, reason) = line.split_once('\t').unwrap_or((&line, ""));
            let path = path.trim();
            if path.is_empty() {
                continue;
            }
            if n > 0 {
                out.push(',');
            }
            n += 1;
            out.push_str(&format!(
                r#"{{"path":{},"reason":{}}}"#,
                json_string(path),
                json_string(reason.trim())
            ));
            if out.len() >= config::IO_CHUNK {
                req.chunk(out.as_bytes())?;
                out.clear();
                crate::files::wait_for_heap()?;
            }
        }
    }
    out.push(']');
    req.chunk(out.as_bytes())?;
    req.finish()
}

/// `DELETE /api/cache/failed` (and a `DELETE` of the file itself): forget
/// every recorded failure, so a GIF that was replaced is tried again. The
/// file goes; the walk drops its copy of the list at its next step; a
/// recount is asked for, which also ends a finished pass so a new one starts.
/// Returns whether there was a file.
pub fn clear_failed() -> io::Result<bool> {
    let _file = FAILED_FILE.lock().unwrap_or_else(|p| p.into_inner());
    FAILED_ADDED
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clear();
    let existed = match fs::remove_file(config::BUILDER_FAILED_PATH) {
        Ok(()) => true,
        Err(e) if e.kind() == io::ErrorKind::NotFound => false,
        Err(e) => return Err(e),
    };
    FAILED_CLEARED.store(true, SeqCst);
    report().failed = 0;
    cache::invalidate();
    log::info!("cache builder: failure list cleared");
    Ok(existed)
}

/// `POST /api/cache/failed`: record that `gif` cannot be prepared (the
/// browser could not either), or — `reason` [`gifslim::plays_original`] —
/// that it has no copy on purpose and plays as it is (counted as done: a
/// recount is asked for). Once per path — compared case-insensitively,
/// as FAT names are. `reason` is kept on one line, at most 200 characters.
/// Returns whether a line was added. The builder skips it from its next step.
pub fn add_failed(gif: &str, reason: &str) -> io::Result<bool> {
    let reason: String = reason
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(200)
        .collect();
    let _file = FAILED_FILE.lock().unwrap_or_else(|p| p.into_inner());
    // Already there? And does the file end with a newline (a hand-edited one
    // may not)?
    let mut ends_in_newline = true;
    match File::open(config::BUILDER_FAILED_PATH) {
        Ok(f) => {
            let mut reader = BufReader::with_capacity(512, f);
            let mut line = Vec::with_capacity(128);
            loop {
                line.clear();
                if reader.read_until(b'\n', &mut line)? == 0 {
                    break;
                }
                ends_in_newline = line.ends_with(b"\n");
                let path = line.split(|&b| b == b'\t').next().unwrap_or(&[]);
                if path.trim_ascii().eq_ignore_ascii_case(gif.as_bytes()) {
                    return Ok(false);
                }
            }
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    fs::create_dir_all(config::CACHE_ROOT)?;
    let mut f = OpenOptions::new()
        .create(true)
        .append(true)
        .open(config::BUILDER_FAILED_PATH)?;
    let sep = if ends_in_newline { "" } else { "\n" };
    writeln!(f, "{sep}{gif}\t{reason}")?;
    drop(f);
    let original = gifslim::plays_original(&reason);
    FAILED_ADDED
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .push((layout::prepared_key(gif), !original));
    if original {
        cache::invalidate();
    } else {
        report().failed += 1;
    }
    log::info!("cache builder: {gif} recorded as failed from outside ({reason})");
    Ok(true)
}

fn record_failure(gif: &str, why: &str) {
    let _file = FAILED_FILE.lock().unwrap_or_else(|p| p.into_inner());
    let _ = fs::create_dir_all(config::CACHE_ROOT);
    let appended = OpenOptions::new()
        .create(true)
        .append(true)
        .open(config::BUILDER_FAILED_PATH)
        .and_then(|mut f| writeln!(f, "{gif}\t{why}"));
    if let Err(e) = appended {
        log::warn!("cache builder: recording the failure of {gif}: {e}");
    }
}

/// `PUT` of a GIF (`/gifs/….gif`): the file is new, so its failure line —
/// a failure, or "plays as it is" — goes, and the count is asked again (a
/// GIF that played as it is no longer counts as done). The walk under way
/// may still skip it; the next pass reads the file afresh.
pub fn drop_failed(gif: &str) {
    if layout::cache_path(gif).is_none() {
        return;
    }
    let _file = FAILED_FILE.lock().unwrap_or_else(|p| p.into_inner());
    match crate::files::rename_line(config::BUILDER_FAILED_PATH, gif, None) {
        Ok(true) => {
            log::info!("cache builder: {gif} replaced, its failure line dropped");
            cache::invalidate();
        }
        Ok(false) => {}
        Err(e) => log::warn!("cache builder: dropping the failure line of {gif}: {e}"),
    }
}

/// `POST /api/files/move`: the failure line of GIF `from`, if there is one,
/// now names `to` (and the walk skips `to` from its next step). Whether a
/// line was renamed.
pub fn rename_failed(from: &str, to: &str) -> io::Result<bool> {
    let _file = FAILED_FILE.lock().unwrap_or_else(|p| p.into_inner());
    let renamed = crate::files::rename_line(config::BUILDER_FAILED_PATH, from, Some(to))?;
    if renamed {
        FAILED_ADDED
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push((layout::prepared_key(to), false));
    }
    Ok(renamed)
}

//! SD and frame timing probe: where the display loop's frame-deadline misses
//! come from.
//!
//! **Off by default, and free when off.** Built only with `--features probe`
//! (`make fw-build FW_FEATURES=probe`; add `probe-http` for the httpd lines).
//! Every hook is guarded by [`ENABLED`] / [`HTTP_LOG`], which are `cfg!`
//! constants, so in a default build the branches — and the clock reads
//! behind [`stamp`] / [`http_stamp`], which return `None` — are removed by the
//! optimiser: no timing call is left on the frame path.
//!
//! What it measures, when on: `File::open` in [`crate::sd::SdSource`], every
//! read and rewind through [`TimedFile`], every catalog lookup through
//! [`TimedCatalog`], and the blit (time and pixels pushed) — accumulated
//!
//! - **per iteration** of the frame loop ([`Tick`]), so a deadline miss can say
//!   what that iteration spent its time on;
//! - **per heartbeat** ([`period_summary`]), for the heartbeat's `sd:` part.
//!
//! `tools/sdprobe-report.py` turns a serial capture of those lines into the
//! tables (open latency per directory, misses by cause, blit percentiles).
//!
//! Only the main task touches these counters (the httpd side logs its own
//! timings directly), so `Relaxed` atomics are plenty; they are atomics only
//! so they can be statics.
//!
//! **Logging is deferred.** A UART line at 115200 baud costs ~87 µs a byte,
//! and the console write blocks, so an `info!` inside the timed region would
//! be counted as lateness. Open events and miss reports are queued and
//! printed at the top of the next loop iteration, after the frame's sleep,
//! which the lateness metric does not measure.

use opendmd_core::engine::Catalog;
use opendmd_core::manifest::Entry;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::sync::atomic::{AtomicU32, Ordering::Relaxed};
use std::sync::Mutex;
use std::time::Instant;

/// Master switch: the `probe` cargo feature.
pub const ENABLED: bool = cfg!(feature = "probe");

/// Also log every httpd `open`/listing/transfer (info): the `probe-http`
/// feature. Separate because those lines cost UART time on the other core too.
pub const HTTP_LOG: bool = ENABLED && cfg!(feature = "probe-http");

/// A start time for [`elapsed_us`], or `None` (no clock read) when the probe
/// is off.
#[inline(always)]
pub fn stamp() -> Option<Instant> {
    ENABLED.then(Instant::now)
}

/// [`stamp`] for the httpd side, which is gated on [`HTTP_LOG`].
#[inline(always)]
pub fn http_stamp() -> Option<Instant> {
    HTTP_LOG.then(Instant::now)
}

/// Microseconds since `t`, saturated; 0 for `None`.
#[inline(always)]
pub fn elapsed_us(t: Option<Instant>) -> u32 {
    t.map_or(0, us_since)
}

macro_rules! counters {
    ($($name:ident),* $(,)?) => { $(static $name: AtomicU32 = AtomicU32::new(0);)* };
}

// Per iteration of the frame loop.
counters!(
    T_OPENS,
    T_OPEN_US,
    T_READS,
    T_READ_US,
    T_READ_MAX_US,
    T_CAT_US
);
// Per heartbeat.
counters!(
    P_OPENS,
    P_OPEN_US,
    P_OPEN_MAX_US,
    P_REWINDS,
    P_READS,
    P_READ_MAX_US,
    P_R5,
    P_R20,
    P_R100,
    P_CAT_MAX_US,
    P_FRAMES,
    P_WORK_MAX_US,
    P_MISS_OPEN,
    P_MISS_NOOPEN,
    P_BLIT_PX
);
// For the life of the run.
counters!(A_OPENS, A_OPEN_MAX_US, A_REWINDS);

/// Blit-time histogram buckets: 100 µs wide, the last one open-ended.
const BLIT_BUCKETS: usize = 256;
const BLIT_BUCKET_US: u32 = 100;

/// Per-heartbeat and whole-run blit histograms, plus the maxima (µs).
struct BlitHist {
    period: [u32; BLIT_BUCKETS],
    run: [u32; BLIT_BUCKETS],
    period_max: u32,
    run_max: u32,
}

impl BlitHist {
    const fn new() -> Self {
        BlitHist {
            period: [0; BLIT_BUCKETS],
            run: [0; BLIT_BUCKETS],
            period_max: 0,
            run_max: 0,
        }
    }

    fn add(&mut self, us: u32) {
        let b = ((us / BLIT_BUCKET_US) as usize).min(BLIT_BUCKETS - 1);
        self.period[b] += 1;
        self.run[b] += 1;
        self.period_max = self.period_max.max(us);
        self.run_max = self.run_max.max(us);
    }

    /// `p50 Xus max Yus (run p50 Zus max Wus)`, then the period is reset.
    fn take_summary(&mut self) -> String {
        let s = format!(
            "p50 {}us max {}us (run p50 {}us max {}us)",
            hist_p50(&self.period),
            self.period_max,
            hist_p50(&self.run),
            self.run_max,
        );
        self.period = [0; BLIT_BUCKETS];
        self.period_max = 0;
        s
    }
}

static BLIT: Mutex<BlitHist> = Mutex::new(BlitHist::new());
/// The same histogram for the tick's CPU time (tick minus SD open, read and
/// catalog time: decode + composite), over frames that opened no file.
static DECODE: Mutex<BlitHist> = Mutex::new(BlitHist::new());

/// A transition's steps: the tick of a step that decoded nothing (so: the
/// composite alone), and the blit of every step (the composited picture).
static TRANS_TICK: Mutex<BlitHist> = Mutex::new(BlitHist::new());
static TRANS_BLIT: Mutex<BlitHist> = Mutex::new(BlitHist::new());
counters!(P_TRUNS, P_TSTEPS, P_TPX, LAST_FREE);

/// The transition in progress: when it started, its steps, and the heap
/// around it.
struct TransRun {
    start: Instant,
    steps: u32,
    before: u32,
    min: u32,
}

static TRANS_RUN: Mutex<Option<TransRun>> = Mutex::new(None);

/// Account a frame for the transition statistics: `transitioning` is the
/// tick's flag (false on the frame that ends one), `decoded` whether the
/// file underneath drew, `free` the heap after the blit. At the end of each
/// transition a line gives its steps, time and heap: free before it (the
/// frame before), the lowest during, and after.
pub fn transition_frame(transitioning: bool, decoded: bool, t: &FrameTimes, free: u32) {
    if !ENABLED {
        return;
    }
    let before = LAST_FREE.swap(free, Relaxed);
    let Ok(mut run) = TRANS_RUN.lock() else {
        return;
    };
    if !transitioning && run.is_none() {
        return;
    }
    P_TSTEPS.fetch_add(1, Relaxed);
    P_TPX.fetch_add(t.blit_px, Relaxed);
    if let Ok(mut h) = TRANS_BLIT.lock() {
        h.add(t.blit_us);
    }
    if !decoded {
        if let Ok(mut h) = TRANS_TICK.lock() {
            h.add(t.tick_us);
        }
    }
    let r = run.get_or_insert_with(|| TransRun {
        start: Instant::now(),
        steps: 0,
        before,
        min: free,
    });
    r.steps += 1;
    r.min = r.min.min(free);
    if !transitioning {
        let r = run.take().unwrap();
        P_TRUNS.fetch_add(1, Relaxed);
        defer(format!(
            "transition: {} steps in {} ms, heap free {} B before, {} B lowest during, {} B after (dip {} B)",
            r.steps,
            r.start.elapsed().as_millis(),
            r.before,
            r.min,
            free,
            free.saturating_sub(r.min),
        ));
    }
}

/// Median of a histogram, as the bucket's midpoint in µs.
fn hist_p50(h: &[u32; BLIT_BUCKETS]) -> u32 {
    let n: u32 = h.iter().sum();
    let mut seen = 0;
    for (i, c) in h.iter().enumerate() {
        seen += c;
        if n > 0 && seen * 2 >= n {
            return i as u32 * BLIT_BUCKET_US + BLIT_BUCKET_US / 2;
        }
    }
    0
}

/// Log lines waiting for the untimed part of the loop.
static PENDING: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn us_since(t: Instant) -> u32 {
    t.elapsed().as_micros().min(u32::MAX as u128) as u32
}

fn max(a: &AtomicU32, v: u32) {
    a.fetch_max(v, Relaxed);
}

/// `File::open` on the main task, timed when the probe is on.
pub fn open(full: &str) -> io::Result<File> {
    if !ENABLED {
        return File::open(full);
    }
    let t = Instant::now();
    let r = File::open(full);
    let us = us_since(t);
    T_OPENS.fetch_add(1, Relaxed);
    T_OPEN_US.fetch_add(us, Relaxed);
    P_OPENS.fetch_add(1, Relaxed);
    P_OPEN_US.fetch_add(us, Relaxed);
    max(&P_OPEN_MAX_US, us);
    A_OPENS.fetch_add(1, Relaxed);
    max(&A_OPEN_MAX_US, us);
    defer(format!(
        "sdopen {us} us {}{full}",
        if r.is_err() { "(failed) " } else { "" }
    ));
    r
}

/// A `File` whose every `read` is timed into the tick and period counters,
/// and whose rewinds (the engine looping a file) are counted.
pub struct TimedFile(pub File);

impl Read for TimedFile {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let t = Instant::now();
        let r = self.0.read(buf);
        let us = us_since(t);
        T_READS.fetch_add(1, Relaxed);
        T_READ_US.fetch_add(us, Relaxed);
        max(&T_READ_MAX_US, us);
        P_READS.fetch_add(1, Relaxed);
        max(&P_READ_MAX_US, us);
        if us >= 5_000 {
            P_R5.fetch_add(1, Relaxed);
        }
        if us >= 20_000 {
            P_R20.fetch_add(1, Relaxed);
        }
        if us >= 100_000 {
            P_R100.fetch_add(1, Relaxed);
        }
        r
    }
}

impl Seek for TimedFile {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        if pos == SeekFrom::Start(0) {
            P_REWINDS.fetch_add(1, Relaxed);
            A_REWINDS.fetch_add(1, Relaxed);
        }
        self.0.seek(pos)
    }
}

/// The playlist lookup (index seek + manifest line) timed, since it is SD I/O
/// on the same path as the open.
pub struct TimedCatalog(pub Box<dyn Catalog>);

impl Catalog for TimedCatalog {
    fn len(&self) -> usize {
        self.0.len()
    }

    fn entry(&mut self, i: usize) -> io::Result<Entry> {
        let t = Instant::now();
        let r = self.0.entry(i);
        let us = us_since(t);
        T_CAT_US.fetch_add(us, Relaxed);
        max(&P_CAT_MAX_US, us);
        r
    }
}

fn defer(line: String) {
    if let Ok(mut q) = PENDING.lock() {
        // Bounded: a runaway could otherwise eat the heap.
        if q.len() < 16 {
            q.push(line);
        }
    }
}

/// Print whatever the timed region queued. Call where lateness is not measured.
pub fn flush() {
    if !ENABLED {
        return;
    }
    let lines = match PENDING.lock() {
        Ok(mut q) if !q.is_empty() => std::mem::take(&mut *q),
        _ => return,
    };
    for l in lines {
        log::info!("{l}");
    }
}

/// Snapshot of one loop iteration's SD work, taken when the frame is judged.
#[derive(Default, Clone, Copy)]
pub struct Tick {
    pub opens: u32,
    pub open_us: u32,
    pub reads: u32,
    pub read_us: u32,
    pub read_max_us: u32,
    pub cat_us: u32,
}

/// Zero the per-iteration counters. Call just before `engine.tick`.
pub fn tick_begin() {
    if !ENABLED {
        return;
    }
    for c in [
        &T_OPENS,
        &T_OPEN_US,
        &T_READS,
        &T_READ_US,
        &T_READ_MAX_US,
        &T_CAT_US,
    ] {
        c.store(0, Relaxed);
    }
}

pub fn tick_snapshot() -> Tick {
    Tick {
        opens: T_OPENS.load(Relaxed),
        open_us: T_OPEN_US.load(Relaxed),
        reads: T_READS.load(Relaxed),
        read_us: T_READ_US.load(Relaxed),
        read_max_us: T_READ_MAX_US.load(Relaxed),
        cat_us: T_CAT_US.load(Relaxed),
    }
}

/// Where one frame iteration's time went, all in µs from the tick's `now`.
pub struct FrameTimes {
    pub tick_us: u32,
    pub blit_us: u32,
    /// Pixels the blit pushed (its dirty count).
    pub blit_px: u32,
    pub log_us: u32,
    pub work_us: u32,
    pub delay_ms: u64,
}

/// Account a frame; on a miss, queue a line that attributes it.
pub fn frame(t: &FrameTimes, late_ms: Option<u64>, is_new: bool, path: &str) {
    if !ENABLED {
        return;
    }
    let s = tick_snapshot();
    P_FRAMES.fetch_add(1, Relaxed);
    P_BLIT_PX.fetch_add(t.blit_px, Relaxed);
    max(&P_WORK_MAX_US, t.work_us);
    if let Ok(mut h) = BLIT.lock() {
        h.add(t.blit_us);
    }
    // The tick's own time minus SD time is (roughly) decode + composite.
    let cpu_us = t
        .tick_us
        .saturating_sub(s.open_us)
        .saturating_sub(s.read_us)
        .saturating_sub(s.cat_us);
    if s.opens == 0 {
        if let Ok(mut h) = DECODE.lock() {
            h.add(cpu_us);
        }
    }
    let Some(late) = late_ms else { return };
    if s.opens > 0 {
        P_MISS_OPEN.fetch_add(1, Relaxed);
    } else {
        P_MISS_NOOPEN.fetch_add(1, Relaxed);
    }
    defer(format!(
        "miss late={late}ms delay={}ms work={}us tick={}us new={} opens={} open={}us cat={}us \
         reads={} rd_tot={}us rd_max={}us cpu~{}us blit={}us px={} log={}us @{} {path}",
        t.delay_ms,
        t.work_us,
        t.tick_us,
        is_new as u8,
        s.opens,
        s.open_us,
        s.cat_us,
        s.reads,
        s.read_us,
        s.read_max_us,
        cpu_us,
        t.blit_us,
        t.blit_px,
        t.log_us,
        // SAFETY: argument-free read of the log clock, for correlating with
        // the httpd lines by their `(ms)` stamps.
        unsafe { esp_idf_sys::esp_log_timestamp() },
    ));
}

/// The heartbeat's probe fields; resets the period counters. Empty when off.
pub fn period_summary() -> String {
    if !ENABLED {
        return String::new();
    }
    let take = |c: &AtomicU32| c.swap(0, Relaxed);
    let opens = take(&P_OPENS);
    let open_us = take(&P_OPEN_US);
    let frames = take(&P_FRAMES);
    let px = take(&P_BLIT_PX);
    let blit = match BLIT.lock() {
        Ok(mut h) => format!(
            "blit {} px/frame {}",
            h.take_summary(),
            px.checked_div(frames).unwrap_or(0),
        ),
        Err(_) => String::new(),
    };
    let decode = DECODE
        .lock()
        .map(|mut h| format!("decode {}, ", h.take_summary()))
        .unwrap_or_default();
    let steps = take(&P_TSTEPS);
    let trans = format!(
        "transitions {} steps {steps} composite-only tick {} blit {} px/step {}, ",
        take(&P_TRUNS),
        TRANS_TICK
            .lock()
            .map(|mut h| h.take_summary())
            .unwrap_or_default(),
        TRANS_BLIT
            .lock()
            .map(|mut h| h.take_summary())
            .unwrap_or_default(),
        take(&P_TPX).checked_div(steps).unwrap_or(0),
    );
    format!(
        " | sd: frames {frames} work-max {}us, {blit}, {decode}{trans}opens {opens} avg {}us max {}us \
         (all-time {} max {}us), rewinds {} (all-time {}), cat-max {}us, reads {} max {}us \
         >=5ms {} >=20ms {} >=100ms {}, misses open {} no-open {}",
        take(&P_WORK_MAX_US),
        open_us.checked_div(opens).unwrap_or(0),
        take(&P_OPEN_MAX_US),
        A_OPENS.load(Relaxed),
        A_OPEN_MAX_US.load(Relaxed),
        take(&P_REWINDS),
        A_REWINDS.load(Relaxed),
        take(&P_CAT_MAX_US),
        take(&P_READS),
        take(&P_READ_MAX_US),
        take(&P_R5),
        take(&P_R20),
        take(&P_R100),
        take(&P_MISS_OPEN),
        take(&P_MISS_NOOPEN),
    )
}

/// httpd-side timer: `http <what> <us> us <detail>` at info, when
/// [`HTTP_LOG`] is on.
pub fn http_log(what: &str, t: Option<Instant>, detail: &str) {
    if HTTP_LOG {
        log::info!("http {what} {} us {detail}", elapsed_us(t));
    }
}

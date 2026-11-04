//! `/api/status` `net` (net-wedge2): the network side's heap floor and
//! lwIP's state, for measuring the spare under simultaneous uploads.
//!
//! - **The heap as the network side meets it** ([`sample`]): the display
//!   loop's `heap.floor` is read on CPU1 between frames, and `idf_min` says
//!   nothing about who. These samples are taken on CPU0 where the network's
//!   allocations peak: a status answer's pieces, an upload's chunks, a
//!   refusal, a request's start, the network worker's turn. The least of
//!   them since boot and over the last 30-60 s, each with where it was and
//!   what lwIP held just after (the walk posted at the low).
//! - **lwIP** (`components/hub75_shim/netstat.c`, walked on its own thread
//!   once a second by the network worker): pcbs, TIME_WAIT pcbs, received
//!   frames held (each one of the WiFi driver's six receive buffers), bytes
//!   queued to send (kept until the client's ACK), and the most
//!   retransmissions of one segment; now and the most over the window.
//! - **Dead connections aborted at close** ([`close_dead`]).

use std::sync::atomic::{AtomicU32, Ordering::Relaxed};

extern "C" {
    fn opendmd_netstat_post();
    fn opendmd_close_dead(fd: i32);
    static opendmd_netstat: [i32; 6];
    static opendmd_netstat_aborted: i32;
}

/// Where a heap sample was taken.
#[derive(Clone, Copy)]
#[repr(u32)]
pub enum At {
    Worker = 0,
    Request = 1,
    Status = 2,
    Refusal = 3,
    Upload = 4,
}
const AT: [&str; 5] = ["worker", "request", "status", "refusal", "upload"];

/// A window of [`WINDOW_S`]: the current one and the one before are shown.
const WINDOW_S: u32 = 30;

struct Low {
    free: AtomicU32,
    at: AtomicU32,
    uploads: AtomicU32,
    up_s: AtomicU32,
    /// The walk that counts what lwIP held at it, and those counts
    /// (rx frames, tx bytes); `u32::MAX` until it has run.
    walk: AtomicU32,
    rx: AtomicU32,
    tx: AtomicU32,
}

#[allow(clippy::declare_interior_mutable_const)]
const LOW0: Low = Low {
    free: AtomicU32::new(u32::MAX),
    at: AtomicU32::new(0),
    uploads: AtomicU32::new(0),
    up_s: AtomicU32::new(0),
    walk: AtomicU32::new(0),
    rx: AtomicU32::new(u32::MAX),
    tx: AtomicU32::new(u32::MAX),
};

/// Since boot, this window, the last window.
static LOWS: [Low; 3] = [LOW0, LOW0, LOW0];
static WINDOW: AtomicU32 = AtomicU32::new(0);
/// lwIP's most over this window and the last: rx frames, tx bytes, rtx.
static MAX: [[AtomicU32; 3]; 2] = [[const { AtomicU32::new(0) }; 3], [const { AtomicU32::new(0) }; 3]];

fn uptime_s() -> u32 {
    // SAFETY: a plain clock read.
    (unsafe { esp_idf_sys::esp_timer_get_time() } / 1_000_000) as u32
}

fn walks() -> u32 {
    // SAFETY: a C int written by the walk on the tcpip thread.
    unsafe { std::ptr::read_volatile(&opendmd_netstat[5]) as u32 }
}

fn counts() -> [u32; 5] {
    // SAFETY: as above.
    std::array::from_fn(|k| unsafe { std::ptr::read_volatile(&opendmd_netstat[k]) }.max(0) as u32)
}

/// Roll the windows when the current one is over: the current becomes the
/// last (or an empty one, two windows on), and a new one starts.
fn roll(now_s: u32) {
    let w = now_s / WINDOW_S;
    let was = WINDOW.load(Relaxed);
    if w == was || WINDOW.compare_exchange(was, w, Relaxed, Relaxed).is_err() {
        return;
    }
    let keep = w == was + 1;
    let (cur, last) = (&LOWS[1], &LOWS[2]);
    for (a, b) in [
        (&last.at, &cur.at),
        (&last.uploads, &cur.uploads),
        (&last.up_s, &cur.up_s),
        (&last.walk, &cur.walk),
        (&last.rx, &cur.rx),
        (&last.tx, &cur.tx),
    ] {
        a.store(b.load(Relaxed), Relaxed);
    }
    last.free.store(if keep { cur.free.load(Relaxed) } else { u32::MAX }, Relaxed);
    cur.free.store(u32::MAX, Relaxed);
    for (l, c) in MAX[1].iter().zip(&MAX[0]) {
        l.store(if keep { c.load(Relaxed) } else { 0 }, Relaxed);
        c.store(0, Relaxed);
    }
}

/// A heap reading on the network side.
#[inline(never)]
pub fn sample(at: At) {
    let free = crate::state::free_heap();
    if free >= LOWS[1].free.load(Relaxed) && free >= LOWS[0].free.load(Relaxed) {
        return;
    }
    let now = uptime_s();
    roll(now);
    let mut posted = None;
    for low in &LOWS[..2] {
        if free < low.free.load(Relaxed) {
            low.free.store(free, Relaxed);
            low.at.store(at as u32, Relaxed);
            low.uploads.store(crate::stream::uploads(), Relaxed);
            low.up_s.store(now, Relaxed);
            let seq = *posted.get_or_insert_with(|| {
                let seq = walks().wrapping_add(1);
                // SAFETY: posts a count to lwIP's thread; no arguments.
                unsafe { opendmd_netstat_post() };
                seq
            });
            low.walk.store(seq, Relaxed);
            low.rx.store(u32::MAX, Relaxed);
            low.tx.store(u32::MAX, Relaxed);
        }
    }
}

/// The counts of the walk posted at a low, once it has run.
fn settle() {
    let done = walks();
    let c = counts();
    for low in &LOWS {
        if low.rx.load(Relaxed) == u32::MAX && low.free.load(Relaxed) != u32::MAX && done.wrapping_sub(low.walk.load(Relaxed)) < 0x8000_0000 {
            low.rx.store(c[2], Relaxed);
            low.tx.store(c[3], Relaxed);
        }
    }
}

/// The network worker's turn (about once a second): fold the last walk into
/// the window's most, post the next.
pub fn tick() {
    sample(At::Worker);
    roll(uptime_s());
    settle();
    let c = counts();
    for (k, v) in [c[2], c[3], c[4]].into_iter().enumerate() {
        MAX[0][k].fetch_max(v, Relaxed);
    }
    // SAFETY: posts a count to lwIP's thread; no arguments.
    unsafe { opendmd_netstat_post() };
}

/// Close `fd` (an httpd session's socket), aborting it first if the client
/// has closed its end with data of ours still unacknowledged.
pub fn close_dead(fd: i32) {
    // SAFETY: queued to the tcpip thread ahead of the close below, which
    // looks the socket up there while it is still open.
    unsafe { opendmd_close_dead(fd) };
}

fn low_json(low: &Low) -> String {
    let free = low.free.load(Relaxed);
    if free == u32::MAX {
        return "null".into();
    }
    let opt = |v: u32| if v == u32::MAX { "null".to_string() } else { v.to_string() };
    format!(
        r#"{{"free":{free},"at":"{}","uploads":{},"up_s":{},"rx_frames":{},"tx_bytes":{}}}"#,
        AT[(low.at.load(Relaxed) as usize).min(AT.len() - 1)],
        low.uploads.load(Relaxed),
        low.up_s.load(Relaxed),
        opt(low.rx.load(Relaxed)),
        opt(low.tx.load(Relaxed)),
    )
}

/// `{"heap_low":{...},"heap_low_recent":[now, last],"lwip":{...},"lwip_max":[...],"aborted_at_close":N}`.
pub fn json() -> String {
    roll(uptime_s());
    settle();
    let c = counts();
    let max = |w: usize| {
        format!(
            r#"{{"rx_frames":{},"tx_bytes":{},"rtx":{}}}"#,
            MAX[w][0].load(Relaxed),
            MAX[w][1].load(Relaxed),
            MAX[w][2].load(Relaxed)
        )
    };
    format!(
        r#"{{"heap_low":{},"heap_low_recent":[{},{}],"window_s":{WINDOW_S},"lwip":{{"pcbs":{},"time_wait":{},"rx_frames":{},"tx_bytes":{},"rtx":{}}},"lwip_max":[{},{}],"aborted_at_close":{}}}"#,
        low_json(&LOWS[0]),
        low_json(&LOWS[1]),
        low_json(&LOWS[2]),
        c[0],
        c[1],
        c[2],
        c[3],
        c[4],
        max(0),
        max(1),
        // SAFETY: a C int.
        unsafe { std::ptr::read_volatile(&opendmd_netstat_aborted) },
    )
}

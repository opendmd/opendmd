//! A cooperative quiet window (display-isolation, 2026-11-02): CPU0's heavy
//! loops step aside while a raced Formula's frame is in its sensitive
//! stretch on CPU1.
//!
//! The display core and CPU0 share the one SPI flash bus (DIO, 40 MHz): a
//! cache miss on CPU1 waits behind CPU0's. On the spare, any CPU0 activity
//! (status reads, uploads, the web app's files) stretched a raced frame's
//! flash-resident start (the loop, the tick, the draw stage) by 15-25 %
//! and made starts late. Priority cannot help (it is the bus, not the
//! scheduler). So CPU1 says when the stretch that matters runs — from its
//! wake ([`begin`], when the Formula on screen races) to the frame's last
//! pair written ([`end`]) — and CPU0's chunk loops ([`wait`]: file sends
//! and uploads, the web app's files, the status JSON) wait for the gap,
//! never longer than [`MAX_WAIT`] a call (one frame and a bit: nothing
//! starves), and not at all while nothing races (a GIF, the panel off).
//!
//! Off by default; `PUT /api/quiet {"on":true}` turns it on for an A/B
//! (not saved: a restart turns it off).

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering::Relaxed};
use std::time::{Duration, Instant};

/// The gate is in use.
static ON: AtomicBool = AtomicBool::new(false);
/// When the display's sensitive stretch began (µs clock, `| 1`: never 0),
/// 0 outside it. CPU1 writes it, CPU0 reads it.
static SINCE: AtomicU32 = AtomicU32::new(0);
/// [`wait`] calls that waited, the µs they waited (wrapping), and those
/// cut off by [`MAX_WAIT`].
static WAITS: AtomicU32 = AtomicU32::new(0);
static WAIT_US: AtomicU32 = AtomicU32::new(0);
static CUT: AtomicU32 = AtomicU32::new(0);

/// The longest one [`wait`] waits.
pub const MAX_WAIT: Duration = Duration::from_millis(12);
/// A stretch older than this is taken as over (a frame that was not raced
/// after all never calls [`end`]).
const STALE_US: u32 = 12_000;

fn now_us() -> u32 {
    // SAFETY: argument-free read of the µs clock.
    unsafe { esp_idf_sys::esp_timer_get_time() as u32 }
}

pub fn set(on: bool) {
    ON.store(on, Relaxed);
    if !on {
        SINCE.store(0, Relaxed);
    }
}

/// CPU1, at its wake, when the Formula on screen races: the stretch begins.
#[cfg_attr(not(feature = "psi"), allow(dead_code))]
pub fn begin() {
    if ON.load(Relaxed) {
        SINCE.store(now_us() | 1, Relaxed);
    }
}

/// CPU1, when the raced frame's last pair is written.
pub fn end() {
    SINCE.store(0, Relaxed);
}

/// CPU0, between chunks of heavy work: wait while the display is in its
/// sensitive stretch (bounded).
pub fn wait() {
    if !ON.load(Relaxed) {
        return;
    }
    let mut t0 = None;
    loop {
        let since = SINCE.load(Relaxed);
        if since == 0 || now_us().wrapping_sub(since) > STALE_US {
            break;
        }
        let start = *t0.get_or_insert_with(Instant::now);
        if start.elapsed() >= MAX_WAIT {
            CUT.fetch_add(1, Relaxed);
            break;
        }
        // One tick (1 ms): CPU0's other tasks run meanwhile.
        std::thread::sleep(Duration::from_millis(1));
    }
    if let Some(t) = t0 {
        WAITS.fetch_add(1, Relaxed);
        WAIT_US.fetch_add(t.elapsed().as_micros() as u32, Relaxed);
    }
}

/// `GET /api/quiet`: `{"on", "waits", "wait_us", "cut"}`.
pub fn json() -> String {
    format!(
        r#"{{"on":{},"waits":{},"wait_us":{},"cut":{}}}"#,
        ON.load(Relaxed),
        WAITS.load(Relaxed),
        WAIT_US.load(Relaxed),
        CUT.load(Relaxed)
    )
}

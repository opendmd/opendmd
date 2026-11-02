//! Adaptive upload pacing (the owner's rule: non-display work must never
//! cost the display a frame; a slowdown inside its headroom is fine). While
//! a Formula plays close to its frame budget — the governor's mean render
//! over 60 % of the frame time it holds the Formula to, until it falls
//! under 50 % — an upload's body is read at [`RATE`] at most (the stream
//! task, CPU0, sleeping between its reads: TCP's window then closes by
//! itself and Wi-Fi, lwIP and the card's writes all slow with it). Full
//! speed otherwise, and always with GIFs or the panel dark.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering::Relaxed};
use std::time::Duration;

/// The paced rate, bytes a second.
pub const RATE: u32 = 96 * 1024;
#[cfg_attr(not(feature = "psi"), allow(dead_code))]
const ON_PCT: u32 = 60;
#[cfg_attr(not(feature = "psi"), allow(dead_code))]
const OFF_PCT: u32 = 50;

/// A Formula is on the panel now (the main loop, at each frame).
pub static FORMULA_ON: AtomicBool = AtomicBool::new(false);
static PACED: AtomicBool = AtomicBool::new(false);
/// Milliseconds slept for pacing since boot.
static PACED_MS: AtomicU32 = AtomicU32::new(0);

/// Whether uploads are paced now (with its hysteresis).
fn active() -> bool {
    let on = PACED.load(Relaxed);
    let now = FORMULA_ON.load(Relaxed) && {
        #[cfg(feature = "psi")]
        {
            let (avg_us, frame_ms, _, _) = opendmd_core::fx_psi::stats();
            let budget = frame_ms.max(1) * 1000;
            let pct = if on { OFF_PCT } else { ON_PCT };
            avg_us as u64 * 100 > budget as u64 * pct as u64
        }
        #[cfg(not(feature = "psi"))]
        {
            false
        }
    };
    if now != on {
        PACED.store(now, Relaxed);
        log::info!("uploads: {}", if now { "paced (a Formula near its frame budget)" } else { "full speed" });
    }
    now
}

/// An upload's reader, after `bytes` of its body: wait as the rate asks
/// (`debt_us` carries what was too short to sleep).
pub fn after_read(bytes: usize, debt_us: &mut u64) {
    if !active() {
        *debt_us = 0;
        return;
    }
    *debt_us += bytes as u64 * 1_000_000 / RATE as u64;
    if *debt_us >= 2_000 {
        let ms = *debt_us / 1000;
        std::thread::sleep(Duration::from_millis(ms));
        PACED_MS.fetch_add(ms as u32, Relaxed);
        *debt_us -= ms * 1000;
    }
}

/// `"uploads":{"paced","rate_kb_s","paced_ms"}` for the status.
pub fn status_json() -> String {
    format!(
        r#""uploads":{{"paced":{},"rate_kb_s":{},"paced_ms":{}}}"#,
        PACED.load(Relaxed),
        RATE / 1024,
        PACED_MS.load(Relaxed)
    )
}

//! The display loop's clock: µs since boot from `esp_timer_get_time`
//! (IRAM, 64 bits: no wrap in practice), in place of `Instant` /
//! `clock_gettime` (newlib's path through `Timespec` and its helpers, from
//! flash: ~2-4 % of a raced frame's instructions in esp32sim's block
//! profile, display-isolation 2026-11-02). Both read esp_timer: the same
//! monotonic clock, so the times are the ones `Instant` gave.

use std::time::Duration;

/// Now, in µs since boot.
#[inline(always)]
pub fn now_us() -> u64 {
    // SAFETY: no arguments; esp_timer is up before `main`.
    unsafe { esp_idf_sys::esp_timer_get_time() as u64 }
}

/// µs from `from` to `to` (0 if `to` is earlier), saturated to `u32` like
/// the loop's `us(Duration)`.
#[inline(always)]
pub fn span(from: u64, to: u64) -> u32 {
    to.saturating_sub(from).min(u64::from(u32::MAX)) as u32
}

/// µs since `t` (as [`span`]).
#[inline(always)]
pub fn since(t: u64) -> u32 {
    span(t, now_us())
}

/// The time since `t`, as `Instant::elapsed` gave it.
#[inline(always)]
pub fn elapsed(t: u64) -> Duration {
    Duration::from_micros(now_us().saturating_sub(t))
}

/// `t` plus `d` on the µs clock.
#[inline(always)]
pub fn after(t: u64, d: Duration) -> u64 {
    t.saturating_add(d.as_micros().min(u128::from(u64::MAX)) as u64)
}

/// The engine's epoch (boot, near enough): its times are `Duration`s since
/// this. `Copy`, shared with the status (`/api/status`'s `t_ms`).
#[derive(Clone, Copy, Debug)]
pub struct Epoch(u64);

impl Epoch {
    pub fn now() -> Self {
        Epoch(now_us())
    }

    /// The time since the epoch now.
    #[inline(always)]
    pub fn elapsed(&self) -> Duration {
        self.at(now_us())
    }

    /// The time since the epoch at `t` (µs clock), for a time the loop has
    /// already read.
    #[inline(always)]
    pub fn at(&self, t: u64) -> Duration {
        Duration::from_micros(t.saturating_sub(self.0))
    }
}

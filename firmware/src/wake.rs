//! The display loop's sleep, ended on time and on CPU1's own terms
//! (road-cost 2026-10-29; display-isolation 2026-11-02).
//!
//! `thread::sleep` is a `vTaskDelay` of whole 1 ms ticks rounded up, so
//! the loop woke 0.9 ms late on average (1.7 ms at worst) before every
//! frame. Since road-cost a one-shot `esp_timer` gave a binary semaphore the
//! loop blocks on; it was dispatched from the esp_timer *task*, on CPU0 at
//! priority 22, so whatever kept CPU0 busy (the Wi-Fi task, `/api/status`
//! answered a dozen times a second) delayed the display's wake: on the
//! spare, 484 late starts of a raced Formula in 4,956 frames under status
//! polls, against 7 idle.
//!
//! Now the timer is dispatched from its ISR (`ESP_TIMER_ISR`, the callback
//! in IRAM giving the semaphore from the interrupt) and set
//! [`SPIN_US`] early; the loop then spins the rest on CPU1's own clock, so
//! the moment it goes on is CPU1's alone. It still blocks (yields CPU1, its
//! idle task runs) for all but the last [`SPIN_US`]. [`stats`] says how
//! late it woke. Nothing allocates after [`Waker::new`]; if the timer
//! cannot be made, the loop sleeps as before.

use esp_idf_sys as sys;
use std::sync::atomic::{AtomicU32, Ordering::Relaxed};
use std::time::Instant;

/// How long before the frame is due the timer fires; the rest is spun.
/// An ISR on CPU0 can be held off by a flash operation or a higher
/// interrupt: this is the margin for that.
pub const SPIN_US: u64 = 400;

/// A wake this late after its time (µs) is counted late.
pub const LATE_US: u32 = 100;

// Since boot: wakes, those late, the latest (µs), and the µs spun.
static WAKES: AtomicU32 = AtomicU32::new(0);
static LATE: AtomicU32 = AtomicU32::new(0);
static MAX_US: AtomicU32 = AtomicU32::new(0);
static SPUN_US: AtomicU32 = AtomicU32::new(0);
/// When the loop last went on after its sleep (or found its frame already
/// due), on the µs clock (`esp_timer_get_time`, wrapping): what the beam's
/// start measures the time before it from ([`woke_at_us`]).
static WOKE_AT: AtomicU32 = AtomicU32::new(0);

/// [`WOKE_AT`].
pub fn woke_at_us() -> u32 {
    WOKE_AT.load(Relaxed)
}

fn now_us() -> u32 {
    // SAFETY: argument-free read of the µs clock.
    unsafe { sys::esp_timer_get_time() as u32 }
}

/// `/api/status` `wake`: (wakes, late ones, the latest µs, µs spun), since
/// boot (the last two wrapping or saturating).
pub fn stats() -> (u32, u32, u32, u32) {
    (WAKES.load(Relaxed), LATE.load(Relaxed), MAX_US.load(Relaxed), SPUN_US.load(Relaxed))
}

pub struct Waker {
    timer: sys::esp_timer_handle_t,
    sem: sys::QueueHandle_t,
}

/// The timer's callback, run in its interrupt (CPU0): the semaphore given
/// from the ISR, a yield asked for if that woke the display.
#[link_section = ".iram1.opendmd_wake"]
unsafe extern "C" fn give(arg: *mut core::ffi::c_void) {
    let mut woken: sys::BaseType_t = 0;
    // SAFETY: `arg` is the loop's semaphore, alive for the program; the
    // FromISR call is the one an ISR may make.
    unsafe {
        sys::xQueueGiveFromISR(arg as sys::QueueHandle_t, &mut woken);
        if woken != 0 {
            sys::esp_timer_isr_dispatch_need_yield();
        }
    }
}

impl Waker {
    /// `None` without heap for the semaphore or the timer.
    pub fn new() -> Option<Waker> {
        // (xSemaphoreCreateBinary: a queue of one empty item, type 3.)
        // SAFETY: plain FreeRTOS / esp_timer calls; the handles are kept
        // for the program's life (the display loop never returns).
        unsafe {
            let sem = sys::xQueueGenericCreate(1, 0, 3);
            if sem.is_null() {
                return None;
            }
            let args = sys::esp_timer_create_args_t {
                callback: Some(give),
                arg: sem as *mut core::ffi::c_void,
                dispatch_method: sys::esp_timer_dispatch_t_ESP_TIMER_ISR,
                name: c"frame_wake".as_ptr(),
                skip_unhandled_events: false,
            };
            let mut timer: sys::esp_timer_handle_t = core::ptr::null_mut();
            if sys::esp_timer_create(&args, &mut timer) != sys::ESP_OK {
                sys::vQueueDelete(sem);
                return None;
            }
            Some(Waker { timer, sem })
        }
    }

    /// Block until [`SPIN_US`] before `at`, then spin to `at` (at once if
    /// it has passed). One `Instant::now` to place `at` on the µs clock;
    /// the spin and the count read `esp_timer_get_time` (IRAM), not
    /// `Instant::now` (`clock_gettime` and its helpers, from flash: ~4 % of
    /// a raced Formula's frame instructions spun on the flash bus).
    pub fn sleep_until(&self, at: Instant) {
        let left = at.saturating_duration_since(Instant::now());
        let start = now_us();
        if left.is_zero() {
            // (Already due: the frame ran over, not the wake.)
            WOKE_AT.store(start, Relaxed);
            return;
        }
        let us = left.as_micros().min(u128::from(u32::MAX / 2)) as u32;
        let due = start.wrapping_add(us);
        if u64::from(us) > SPIN_US {
            let fire = u64::from(us) - SPIN_US;
            // A tick or two past the timer: the wait ends even if it never
            // fires.
            let ticks = (fire / 1000 + 2) as u32;
            // SAFETY: handles made in `new`. A give left over from a wait
            // that ended by its timeout is taken first, so it cannot end
            // this one.
            unsafe {
                sys::esp_timer_stop(self.timer);
                sys::xQueueSemaphoreTake(self.sem, 0);
                if sys::esp_timer_start_once(self.timer, fire) != sys::ESP_OK {
                    std::thread::sleep(left);
                    note(due);
                    return;
                }
                sys::xQueueSemaphoreTake(self.sem, ticks);
            }
        }
        let spin = now_us();
        while (now_us().wrapping_sub(due) as i32) < 0 {
            core::hint::spin_loop();
        }
        SPUN_US.fetch_add(now_us().wrapping_sub(spin), Relaxed);
        note(due);
    }
}

/// Count a wake against its time (`due`, on the µs clock).
fn note(due: u32) {
    let now = now_us();
    WOKE_AT.store(now, Relaxed);
    let late = (now.wrapping_sub(due) as i32).max(0) as u32;
    WAKES.fetch_add(1, Relaxed);
    if late > LATE_US {
        LATE.fetch_add(1, Relaxed);
    }
    MAX_US.fetch_max(late, Relaxed);
}

/// Sleep until `at` with `w`, else as the loop did before.
pub fn sleep_until(w: Option<&Waker>, at: Instant) {
    match w {
        Some(w) => w.sleep_until(at),
        None => std::thread::sleep(at.saturating_duration_since(Instant::now())),
    }
}

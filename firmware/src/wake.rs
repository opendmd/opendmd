//! The display loop's sleep, ended on time (road-cost, 2026-10-29).
//!
//! `thread::sleep` is a `vTaskDelay` of whole 1 ms ticks rounded up, so
//! the loop woke 0.9 ms late on average (1.7 ms at worst) before every
//! frame: measured on the spare (roadprobe-b), a Formula's 10 ms frame
//! started 1.2-1.7 ms after it was due, which a heavy 100 fps Formula
//! could not spare. Here a one-shot `esp_timer` (µs, dispatched from the
//! esp_timer task on CPU0) gives a binary semaphore the loop blocks on, so
//! it still yields CPU1 while it waits and wakes within tens of µs.
//! Nothing allocates after [`Waker::new`]; if the timer cannot be made,
//! the loop sleeps as before.

use esp_idf_sys as sys;
use std::time::Instant;

pub struct Waker {
    timer: sys::esp_timer_handle_t,
    sem: sys::QueueHandle_t,
}

unsafe extern "C" fn give(arg: *mut core::ffi::c_void) {
    // (xSemaphoreGive: a send of no item to the back.)
    // SAFETY: `arg` is the loop's semaphore, alive for the program.
    unsafe { sys::xQueueGenericSend(arg as sys::QueueHandle_t, core::ptr::null(), 0, 0) };
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
                dispatch_method: sys::esp_timer_dispatch_t_ESP_TIMER_TASK,
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

    /// Block until `at` (at once if it has passed).
    pub fn sleep_until(&self, at: Instant) {
        let left = at.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return;
        }
        let us = left.as_micros().min(u128::from(u32::MAX)) as u64;
        // A tick or two past the timer: the wait ends even if it never fires.
        let ticks = (us / 1000 + 2) as u32;
        // SAFETY: handles made in `new`. A give left over from a wait that
        // ended by its timeout is taken first, so it cannot end this one.
        unsafe {
            sys::esp_timer_stop(self.timer);
            sys::xQueueSemaphoreTake(self.sem, 0);
            if sys::esp_timer_start_once(self.timer, us) != sys::ESP_OK {
                std::thread::sleep(left);
                return;
            }
            sys::xQueueSemaphoreTake(self.sem, ticks);
        }
    }
}

/// Sleep until `at` with `w`, else as the loop did before.
pub fn sleep_until(w: Option<&Waker>, at: Instant) {
    match w {
        Some(w) => w.sleep_until(at),
        None => std::thread::sleep(at.saturating_duration_since(Instant::now())),
    }
}

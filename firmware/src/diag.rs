//! Heap-corruption diagnostics: the `diag-heap` build only.
//!
//! **Off by default, and compiled out when off**: without the feature every
//! function here is an empty `#[inline(always)]` body. `make fw-build
//! FW_FEATURES=diag-heap` builds it, and the Makefile then adds
//! `sdkconfig.diag` to the ESP-IDF configuration: comprehensive heap
//! poisoning (a canary before and after every block, freed memory filled
//! with 0xFE and checked), and a watchpoint on the last 32 B of whichever
//! task's stack is running (`CONFIG_FREERTOS_WATCHPOINT_END_OF_STACK`).
//!
//! What this module adds on top: [`check`] walks every heap
//! (`heap_caps_check_integrity_all`) at the sites that mattered to the
//! corruption it was written for (CLAUDE.md, Gotchas: "No `std::sync::mpsc`
//! at all" — which turned out to be register corruption, not the heap's):
//! the start and end of every HTTP handler, of every streamed download and
//! upload, around every Follow RePlayOS poll (the connection's whole
//! life), once a second on the network worker, and in the heartbeat. A
//! corrupt heap prints IDF's own report (the block and what is wrong with
//! it), then one line naming the site, then aborts — so the backtrace is
//! taken where it was found, and the site before it bounds where it happened.
//!
//! The walk holds each heap's lock for its duration (5-13 ms measured with
//! comprehensive poisoning: every free byte is checked), so it costs the
//! display loop some deadline misses: a diagnostic build is not for judging
//! frame timing.

#[cfg(feature = "diag-heap")]
mod on {
    use std::sync::atomic::{AtomicU32, Ordering::Relaxed};

    static CHECKS: AtomicU32 = AtomicU32::new(0);
    static WORST_US: AtomicU32 = AtomicU32::new(0);

    pub fn check(site: &'static str) {
        // SAFETY: argument-free clock reads and a heap walk that takes the
        // heaps' own locks.
        let t0 = unsafe { esp_idf_sys::esp_timer_get_time() };
        let ok = unsafe { esp_idf_sys::heap_caps_check_integrity_all(true) };
        let us = (unsafe { esp_idf_sys::esp_timer_get_time() } - t0) as u32;
        CHECKS.fetch_add(1, Relaxed);
        WORST_US.fetch_max(us, Relaxed);
        if !ok {
            // Synchronously, through the ROM: the log ring may never drain.
            // SAFETY: a NUL-terminated format and a NUL-terminated argument.
            unsafe {
                let mut name = [0u8; 48];
                let n = site.len().min(name.len() - 1);
                name[..n].copy_from_slice(&site.as_bytes()[..n]);
                esp_idf_sys::esp_rom_printf(
                    c"\nDIAG: heap corrupt, found at %s\n".as_ptr(),
                    name.as_ptr(),
                );
                crate::restarts::diag(site);
                esp_idf_sys::abort();
            }
        }
    }

    /// Make `n` channels in a row on the calling task (the httpd task:
    /// CPU0, WiFi's interrupts), each used once as a reply channel is —
    /// `std::sync::mpsc`'s (`which` = "std") or [`crate::chan`]'s — to show
    /// the one corrupting its caller and the other not. With std the task
    /// is expected to panic within some thousands of calls.
    pub fn channel_torture(n: u32, which: &str) -> String {
        use std::hint::black_box;
        type Reply = Result<(), crate::state::PlayError>;
        let t0 = std::time::Instant::now();
        let mut sum = 0u32;
        for i in 0..n {
            if which == "std" {
                let (tx, rx) = std::sync::mpsc::channel::<Reply>();
                let _ = black_box(&tx).send(Ok(()));
                sum = sum.wrapping_add(u32::from(black_box(&rx).try_recv().is_ok()));
            } else {
                let (tx, rx) = crate::chan::channel::<Reply>();
                let _ = black_box(&tx).send(Ok(()));
                sum = sum.wrapping_add(u32::from(black_box(&rx).try_recv().is_ok()));
            }
            black_box(i);
        }
        let line = format!(
            "{n} {which} channels in {} ms, {sum} replies\n",
            t0.elapsed().as_millis()
        );
        log::info!("diag: {}", line.trim_end());
        line
    }

    pub fn summary() -> String {
        format!(
            ", diag: {} heap checks, worst {} us",
            CHECKS.load(Relaxed),
            WORST_US.swap(0, Relaxed)
        )
    }
}

/// Walk every heap; on corruption print the report and `site`, and abort.
#[cfg(feature = "diag-heap")]
pub use on::check;

/// The heartbeat's part: checks so far, the slowest since the last one.
#[cfg(feature = "diag-heap")]
pub use on::summary;

/// `GET /api/status?diag_chan=N&impl=std|ours` (diag build only).
#[cfg(feature = "diag-heap")]
pub use on::channel_torture;

#[cfg(not(feature = "diag-heap"))]
#[inline(always)]
pub fn check(_site: &'static str) {}

#[cfg(not(feature = "diag-heap"))]
#[inline(always)]
pub fn summary() -> &'static str {
    ""
}

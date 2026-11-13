//! The ESP32-S3 board's start-up (S3 port phase 3; `cfg(esp32s3)` only,
//! declared at the end of `main.rs` so that the classic build stays byte for
//! byte what it was). Called once, right before `Panel::new`
//! ([`before_panel`]):
//!
//! - **The scan register the beam racer reads** (`hub75_scan_set_source`):
//!   NVS `scan_reg` (`dscr`, `bf0`, `bf1` or `state`), else the build's
//!   `OPENDMD_S3_SCAN_REG`, else `dscr` (GDMA's OUT_DSCR, the emulator's
//!   "being sent"). Which one silicon means is the bench's to say:
//! - **The address-pin check** (bench and test builds: `bench`,
//!   `nocard-test`): at scan init the shim samples the address lines beside
//!   each register and logs `scan: register … agrees with the address pins
//!   k/n` for each.
//! - **The brownout-loop guard.** A count in RTC memory of brownout resets in
//!   a row, each before the last boot's safe start was over
//!   (`config::SAFE_START`, later if the network came late): any other reset
//!   (a power-on, the button, a restart, a crash) clears it, and so does a
//!   boot that lives past its safe start. At [`POWER_SAFE_AT`] the boot is
//!   **power-safe**: the panel is not started (OE driven high, the other
//!   HUB75 lines undriven), WiFi and the web app come up as ever, and
//!   `/api/status` `reset_note` says why ([`NOTE`]; the web app shows a
//!   banner for it). The bench's brownout loops (2026-11-13) came from
//!   driving the panel's lines into an unpowered panel; this lets such a
//!   board be reached instead of looping.

use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering::Relaxed};
use std::time::Duration;

use esp_idf_sys::{self as sys, hub75};
use opendmd_core::restarts::Reset;

use crate::nvs_config::Config;

/// Brownout resets in a row that make a boot power-safe.
pub const POWER_SAFE_AT: u32 = 3;

/// `reset_note` of a power-safe boot (the web app's banner keys on its
/// `power-safe:` prefix).
pub const NOTE: &str = "power-safe: the panel was not started after 3 brownouts in a row: its power supply is missing or too weak";

/// This boot is power-safe.
static POWER_SAFE: AtomicBool = AtomicBool::new(false);

/// The guard's count, tagged: `GUARD_TAG | n`. RTC slow memory, kept across
/// every reset but a power-on (and read through volatile accesses).
#[link_section = ".rtc_noinit"]
static mut GUARD: u32 = 0;
const GUARD_TAG: u32 = 0xB0B0_0000;

fn guard_load() -> u32 {
    // SAFETY: a static's own word, aligned; only these two functions touch it.
    let v = unsafe { core::ptr::read_volatile(&raw const GUARD) };
    if v & 0xFFFF_0000 == GUARD_TAG {
        v & 0xFFFF
    } else {
        0
    }
}

fn guard_store(n: u32) {
    // SAFETY: as `guard_load`.
    unsafe { core::ptr::write_volatile(&raw mut GUARD, GUARD_TAG | n.min(0xFFFF)) };
}

/// `/api/status` `reset_note` for a power-safe boot (before the restart
/// record's own note: `restarts::reset_note`).
pub fn reset_note() -> Option<&'static str> {
    POWER_SAFE.load(Relaxed).then_some(NOTE)
}

/// Once, before `Panel::new`: the scan register, the pin check, the guard.
#[inline(never)]
pub fn before_panel(reset: Reset, nvs: &Config) {
    // The guard first: a power-safe boot starts no panel.
    #[cfg(feature = "nocard-test")]
    let faked = test::fake_brownout(reset, nvs);
    #[cfg(not(feature = "nocard-test"))]
    let faked = false;
    let brownout = reset == Reset::Brownout || faked;
    let n = if brownout { guard_load() + 1 } else { 0 };
    guard_store(n);
    if n >= POWER_SAFE_AT {
        POWER_SAFE.store(true, Relaxed);
        // SAFETY: a flag the shim reads in hub75_init, which has not run.
        unsafe { hub75::hub75_set_power_safe(true) };
        log::warn!(
            "power-safe: panel not started: {n} brownout resets in a row{}, each within safe start: the panel's power supply is missing or too weak (WiFi and the web app as ever; a power-on or any other restart tries the panel again)",
            if faked { " (faked: nocard-test)" } else { "" }
        );
    } else if n > 0 {
        log::warn!(
            "brownout guard: {n} brownout reset{} in a row{} (power-safe at {POWER_SAFE_AT}); cleared once safe start is over",
            if n == 1 { "" } else { "s" },
            if faked { " (faked: nocard-test)" } else { "" }
        );
    }
    if n > 0 {
        arm_clear();
    }

    let (src, from) = match nvs.s3_string("scan_reg").as_deref().and_then(scan_source) {
        Some(s) => (s, "NVS scan_reg"),
        None => match option_env!("OPENDMD_S3_SCAN_REG").and_then(scan_source) {
            Some(s) => (s, "the build's OPENDMD_S3_SCAN_REG"),
            None => (0, "the default"),
        },
    };
    log::info!("scan: the racer reads GDMA's {} ({from})", SCAN_REGS[src as usize].1);
    // SAFETY: plain flags the shim reads in hub75_scan_init, not yet run.
    unsafe {
        hub75::hub75_scan_set_source(src);
        hub75::hub75_scan_set_pins_check(cfg!(any(feature = "bench", feature = "nocard-test")));
    }
}

/// `scan_reg`'s values and the registers they name (the shim's order).
const SCAN_REGS: [(&str, &str); 4] =
    [("dscr", "OUT_DSCR"), ("bf0", "OUT_DSCR_BF0"), ("bf1", "OUT_DSCR_BF1"), ("state", "OUT_STATE")];

fn scan_source(v: &str) -> Option<u8> {
    let v = v.trim();
    SCAN_REGS.iter().position(|(k, _)| *k == v).map(|i| i as u8)
}

/// Clear the guard once this boot's safe start is over (`SafeStart::poll`
/// stores 0 in `SAFE_START_UNTIL` then; never before `config::SAFE_START`):
/// a one-second esp_timer, stopped once it has.
fn arm_clear() {
    static TIMER: AtomicPtr<sys::esp_timer> = AtomicPtr::new(core::ptr::null_mut());
    unsafe extern "C" fn tick(_: *mut core::ffi::c_void) {
        // SAFETY: an argument-free clock read.
        let up = Duration::from_micros(unsafe { sys::esp_timer_get_time() } as u64);
        if up < crate::config::SAFE_START || crate::state::SAFE_START_UNTIL.load(Relaxed) != 0 {
            return;
        }
        guard_store(0);
        log::info!("brownout guard: cleared (safe start over at {} s)", up.as_secs());
        // SAFETY: this timer's own handle; a periodic timer may be stopped
        // from its callback (it is never deleted).
        unsafe { sys::esp_timer_stop(TIMER.load(Relaxed)) };
    }
    let args = sys::esp_timer_create_args_t {
        callback: Some(tick),
        arg: core::ptr::null_mut(),
        dispatch_method: sys::esp_timer_dispatch_t_ESP_TIMER_TASK,
        name: c"brownout-guard".as_ptr(),
        skip_unhandled_events: true,
    };
    let mut handle: sys::esp_timer_handle_t = core::ptr::null_mut();
    // SAFETY: `args` outlives the call; the timer lives for the boot, its
    // handle stored before it first fires.
    let ok = unsafe {
        sys::esp_timer_create(&args, &mut handle) == sys::ESP_OK && {
            TIMER.store(handle, Relaxed);
            sys::esp_timer_start_periodic(handle, 1_000_000) == sys::ESP_OK
        }
    };
    if !ok {
        log::warn!("brownout guard: no timer; the count is kept until another kind of reset");
    }
}

/// The power-safe lane's test hook (`nocard-test` images only; `make
/// emu-s3-power-safe`): NVS `ps_test` = N fakes N brownout resets in a row
/// after the power-on, each a restart ~3 s after the boot before it (well
/// inside safe start), so boot N+1 is the Nth "brownout". The count left is
/// in RTC memory.
#[cfg(feature = "nocard-test")]
mod test {
    use super::*;

    #[link_section = ".rtc_noinit"]
    static mut LEFT: u32 = 0;
    const TAG: u32 = 0xFA4E_0000;

    fn left() -> u32 {
        // SAFETY: a static's own word, read and written only here.
        let v = unsafe { core::ptr::read_volatile(&raw const LEFT) };
        if v & 0xFFFF_0000 == TAG {
            v & 0xFFFF
        } else {
            0
        }
    }

    fn set_left(n: u32) {
        // SAFETY: as `left`.
        unsafe { core::ptr::write_volatile(&raw mut LEFT, TAG | n) };
    }

    /// `ps_test` is set: the lane's, not the no-card test's, boot
    /// (`nocard_test::spawn` stands aside).
    pub fn on(nvs: &Config) -> bool {
        nvs.s3_string("ps_test").is_some_and(|v| v.trim().parse::<u32>().is_ok_and(|n| n > 0))
    }

    /// Whether this boot counts as a brownout reset (and the next restart
    /// is armed while fakes are left).
    pub fn fake_brownout(reset: Reset, nvs: &Config) -> bool {
        if !on(nvs) {
            return false;
        }
        super::PS_TEST.store(true, Relaxed);
        let faked = if reset == Reset::PowerOn {
            let n = nvs.s3_string("ps_test").and_then(|v| v.trim().parse::<u32>().ok()).unwrap_or(0);
            set_left(n);
            log::info!("ps-test: power-on; {n} brownout resets to fake");
            false
        } else if reset == Reset::Software && left() > 0 {
            set_left(left() - 1);
            true
        } else {
            false
        };
        if left() > 0 {
            let _ = crate::task::spawn_core0(
                b"ps-test\0",
                4 * 1024,
                2,
                Box::new(|| {
                    std::thread::sleep(Duration::from_secs(3));
                    log::info!("ps-test: faking a brownout reset ({} left)", left());
                    crate::restart(opendmd_core::restarts::Reason::ApiReboot);
                }),
            );
        }
        faked
    }
}

/// The power-safe lane runs this boot (`ps_test` in NVS, `nocard-test`
/// images): the no-card test stands aside.
#[cfg(feature = "nocard-test")]
static PS_TEST: AtomicBool = AtomicBool::new(false);

#[cfg(feature = "nocard-test")]
pub fn ps_test() -> bool {
    PS_TEST.load(Relaxed)
}

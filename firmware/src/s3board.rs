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
//! - **WiFi's TX power capped while it connects** ([`tx_cap`]): the bench's
//!   two brownouts of 2026-11-13 came ~9.3 s after boot, right after
//!   `wifi:state: assoc -> run` (the handshake and the first TX burst), on a
//!   USB supply, not at the panel's start. From WiFi's start until it has an
//!   address (and again from each disconnect) the TX power is capped at
//!   11 dBm, then put back to what it was; a power-safe boot keeps it capped
//!   all boot (a TX brownout loop would otherwise go on looping).

use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering::Relaxed};
use std::time::Duration;

use esp_idf_sys::{self as sys, hub75};
use opendmd_core::restarts::Reset;

use crate::nvs_config::Config;

/// Brownout resets in a row that make a boot power-safe.
pub const POWER_SAFE_AT: u32 = 3;

/// `reset_note` of a power-safe boot (the web app's banner keys on its
/// `power-safe:` prefix).
pub const NOTE: &str = "power-safe: the power supply dipped repeatedly (the panel was not started; a weak USB supply can do this too)";

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
            "power-safe: panel not started: {n} brownout resets in a row{}, each within safe start: the power supply dipped repeatedly (a weak USB supply can do this too; WiFi's TX power capped for this boot; WiFi and the web app as ever; a power-on or any other restart tries the panel again)",
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
    tx_cap::arm();

    let (src, from) = match nvs.s3_string("scan_reg").as_deref().and_then(scan_source) {
        Some(s) => (s, "NVS scan_reg"),
        None => match option_env!("OPENDMD_S3_SCAN_REG").and_then(scan_source) {
            Some(s) => (s, "the build's OPENDMD_S3_SCAN_REG"),
            None => (0, "the default"),
        },
    };
    log::info!("scan: the racer reads GDMA's {} ({from})", SCAN_REGS[src as usize].1);
    crate::s3panel::before_panel(nvs);
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
/// inside safe start), so the boots after it count as brownouts. The first
/// faked boot (with N >= 2) instead lives until safe start is over and the
/// guard has cleared, then restarts: the next boot counts 1 again. The
/// fakes left and N are in RTC memory.
#[cfg(feature = "nocard-test")]
mod test {
    use super::*;

    #[link_section = ".rtc_noinit"]
    static mut LEFT: u32 = 0;
    const TAG: u32 = 0xFA4E_0000;

    /// (fakes left, N), both under 256.
    fn left() -> (u32, u32) {
        // SAFETY: a static's own word, read and written only here.
        let v = unsafe { core::ptr::read_volatile(&raw const LEFT) };
        if v & 0xFFFF_0000 == TAG {
            (v & 0xFF, (v >> 8) & 0xFF)
        } else {
            (0, 0)
        }
    }

    fn set_left(n: u32, total: u32) {
        // SAFETY: as `left`.
        unsafe { core::ptr::write_volatile(&raw mut LEFT, TAG | (total.min(255) << 8) | n.min(255)) };
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
        let (was, total) = left();
        let faked = if reset == Reset::PowerOn {
            let n = nvs.s3_string("ps_test").and_then(|v| v.trim().parse::<u32>().ok()).unwrap_or(0);
            set_left(n, n);
            log::info!("ps-test: power-on; {n} brownout resets to fake");
            false
        } else if reset == Reset::Software && was > 0 {
            set_left(was - 1, total);
            true
        } else {
            false
        };
        let (now, total) = left();
        // The first faked boot waits for the guard to clear.
        let slow = faked && total >= 2 && now + 1 == total;
        if slow {
            // (Said here, not on the thread: two tasks logging at once this
            // early interleave on the console.)
            log::info!("ps-test: this boot lives past safe start (the guard should clear)");
        }
        if now > 0 {
            let _ = crate::task::spawn_core0(
                b"ps-test\0",
                4 * 1024,
                2,
                Box::new(move || {
                    if slow {
                        for _ in 0..600 {
                            if super::guard_load() == 0 {
                                break;
                            }
                            std::thread::sleep(Duration::from_millis(500));
                        }
                        std::thread::sleep(Duration::from_secs(1));
                    } else {
                        std::thread::sleep(Duration::from_secs(3));
                    }
                    log::info!("ps-test: faking a brownout reset ({} left)", left().0);
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

/// The logger's hook (`logring`'s `log`, S3 only): `main`'s "panel up: …"
/// line is replaced: on a power-safe boot (it follows `Panel::new`, which
/// started no panel) by one saying the panel was not started; else by the
/// same line with the buffers the panel runs with (`s3panel`) where it says
/// the panel config's `double_buff=false` (the S3's double buffering is the
/// shim's, not that config's). True: the record is not to be logged.
pub fn swallow(record: &log::Record) -> bool {
    use core::fmt::Write;
    static DONE: AtomicBool = AtomicBool::new(false);
    if DONE.load(Relaxed) || record.level() != log::Level::Info
        || record.target() != "opendmd_firmware"
    {
        return false;
    }
    /// The message's first bytes, no more.
    struct Head([u8; 10], usize);
    impl Write for Head {
        fn write_str(&mut self, s: &str) -> core::fmt::Result {
            for &b in s.as_bytes() {
                if self.1 == self.0.len() {
                    return Err(core::fmt::Error);
                }
                self.0[self.1] = b;
                self.1 += 1;
            }
            Ok(())
        }
    }
    let mut head = Head([0; 10], 0);
    let _ = write!(head, "{}", record.args());
    if &head.0[..head.1] != b"panel up: " {
        return false;
    }
    DONE.store(true, Relaxed);
    if POWER_SAFE.load(Relaxed) {
        log::warn!("panel: not started (power-safe): no driver, no DMA, every HUB75 pin an input with OE pulled up; WiFi and the web app as ever");
        return true;
    }
    let line = format!("{}", record.args());
    log::info!(target: "opendmd_firmware", "{}", line.replace("double_buff=false", crate::s3panel::mode()));
    true
}

// TODO(s3 follow-up): the same-line hooks (main.rs's `panel_cfg` line,
// `restarts::reset_note`, `logring`'s `log`, hub75_init's first line in
// hub75_shim.cpp) keep the classic's line tables byte-identical; unfold
// them into ordinary statements the next time the classic build may change
// anyway (a release that moves those files' lines for other reasons).

/// WiFi's TX power capped from the station's start (and each disconnect)
/// until it has an address, then put back; capped all boot when power-safe.
/// Handlers on the default event loop (`main` has taken it before
/// `before_panel`), run on the event task.
mod tx_cap {
    use super::*;
    use std::sync::atomic::AtomicI32;

    /// 11 dBm, in esp_wifi_set_max_tx_power's quarter dBm.
    const CAP: i8 = 44;
    /// What the power was before the first cap (quarter dBm; 0: not read).
    static SAVED: AtomicI32 = AtomicI32::new(0);

    fn cap(why: &str) {
        let mut now: i8 = 0;
        // SAFETY: plain calls into the WiFi driver after its start.
        unsafe {
            if sys::esp_wifi_get_max_tx_power(&mut now) == sys::ESP_OK && now > CAP && SAVED.load(Relaxed) == 0 {
                SAVED.store(i32::from(now), Relaxed);
            }
            let e = sys::esp_wifi_set_max_tx_power(CAP);
            if e == sys::ESP_OK {
                log::info!("wifi: TX power capped at {} dBm {why} (was {} dBm; brownout mitigation)", CAP / 4, now / 4);
            } else {
                log::warn!("wifi: TX power cap {why} refused ({e})");
            }
        }
    }

    fn restore() {
        let saved = SAVED.load(Relaxed);
        if saved <= 0 || POWER_SAFE.load(Relaxed) {
            return;
        }
        // SAFETY: as above.
        if unsafe { sys::esp_wifi_set_max_tx_power(saved as i8) } == sys::ESP_OK {
            log::info!("wifi: connected: TX power back to {} dBm", saved / 4);
        }
    }

    unsafe extern "C" fn on_event(
        _arg: *mut core::ffi::c_void,
        base: sys::esp_event_base_t,
        id: i32,
        _data: *mut core::ffi::c_void,
    ) {
        // SAFETY: the event bases are the driver's statics.
        let (wifi, ip) = unsafe { (sys::WIFI_EVENT, sys::IP_EVENT) };
        if base == wifi && id == sys::wifi_event_t_WIFI_EVENT_STA_START as i32 {
            cap(if POWER_SAFE.load(Relaxed) { "for this boot (power-safe)" } else { "until connected" });
        } else if base == wifi && id == sys::wifi_event_t_WIFI_EVENT_STA_DISCONNECTED as i32 {
            // (Quietly: a reconnect loop would say it every few seconds.)
            // SAFETY: as above.
            unsafe { sys::esp_wifi_set_max_tx_power(CAP) };
        } else if base == ip && id == sys::ip_event_t_IP_EVENT_STA_GOT_IP as i32 {
            restore();
        }
    }

    pub fn arm() {
        // SAFETY: handlers for the default loop's events, registered once;
        // the bases are the driver's statics.
        let ok = unsafe {
            let h = Some(on_event as unsafe extern "C" fn(_, _, _, _));
            sys::esp_event_handler_register(sys::WIFI_EVENT, sys::wifi_event_t_WIFI_EVENT_STA_START as i32, h, core::ptr::null_mut()) == sys::ESP_OK
                && sys::esp_event_handler_register(sys::WIFI_EVENT, sys::wifi_event_t_WIFI_EVENT_STA_DISCONNECTED as i32, h, core::ptr::null_mut()) == sys::ESP_OK
                && sys::esp_event_handler_register(sys::IP_EVENT, sys::ip_event_t_IP_EVENT_STA_GOT_IP as i32, h, core::ptr::null_mut()) == sys::ESP_OK
        };
        if !ok {
            log::warn!("wifi: no TX power cap (the event handlers could not be registered)");
        }
    }
}

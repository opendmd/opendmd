//! A WiFi outage on real hardware, on request (feature `wifi-outage-test`):
//! never in a release. `PUT /api/config {"wifi_outage_s": N}` (1..7200,
//! live only, never saved; `wifi_outage_hour_s` 60..3600 makes the hour's
//! `wifi_stuck` restart that long, for a quick run) drops the association
//! and has the reconnect look for a name nobody answers to (the saved name
//! with [`AWAY`] after it), the station's side of a network gone away, as
//! the emulator's soak lane does (`wifi_soak`'s `away` step). Nothing else
//! changes: playback keeps running, and the reconnect's rounds, driver
//! restarts, heap rule and hour's restart run as they would. The saved
//! WiFi details are never touched: the name is changed in the worker's
//! copy only.
//!
//! The outage's time left is kept in RTC memory (as the soak lane's next
//! boot is): the boot after a restart looks for the absent name until it
//! is over, then the real name is given back and it rejoins.
//!
//! Console lines start `wifi-outage:` and are short (the test panel's
//! serial link corrupts long reads): the start, each driver restart, one a
//! minute while it lasts, the restart, the end and the rejoin.

use std::sync::atomic::{AtomicU32, Ordering::Relaxed};

use opendmd_core::wifi_reconnect::Reconnect;

/// What the name gets while the outage lasts.
pub const AWAY: &str = "-outage";

/// A request from `PUT /api/config` (seconds, 0: none) and its short hour.
static REQ_S: AtomicU32 = AtomicU32::new(0);
static REQ_HOUR_S: AtomicU32 = AtomicU32::new(0);
/// The outage's end (uptime ms, 0: none under way), its hour (ms, 0: the
/// real one), when it began on this boot, and the last minute's line.
static END: AtomicU32 = AtomicU32::new(0);
static HOUR_MS: AtomicU32 = AtomicU32::new(0);
static SAID: AtomicU32 = AtomicU32::new(0);
/// When it ended (uptime ms, 0: no rejoin to wait for).
static OVER_AT: AtomicU32 = AtomicU32::new(0);
/// The display's deadline misses at the last look (from `Status`).
static MISSES: AtomicU32 = AtomicU32::new(0);

/// `[MAGIC, ms left, hour ms]`, RTC slow memory: kept across a restart,
/// lost at a power-on; plain word loads and stores.
#[link_section = ".rtc_noinit"]
static mut KEPT: [u32; 3] = [0; 3];
const MAGIC: u32 = 0x574f_5554;

fn kept(i: usize) -> &'static AtomicU32 {
    // SAFETY: a word of `KEPT`, only ever accessed through this view.
    unsafe { AtomicU32::from_ptr((&raw mut KEPT).cast::<u32>().add(i)) }
}

fn now_ms() -> u32 {
    // SAFETY: an argument-free read of the IDF's microsecond clock.
    (unsafe { esp_idf_sys::esp_timer_get_time() } / 1000) as u32
}

fn left_ms(now: u32) -> u32 {
    let end = END.load(Relaxed);
    if end == 0 {
        0
    } else {
        (end.wrapping_sub(now) as i32).max(0) as u32
    }
}

fn heap() -> (u32, usize) {
    (crate::state::free_heap(), crate::state::largest_free_block())
}

/// `PUT /api/config`'s `wifi_outage_s` (checked there), taken by the
/// network worker's next turn.
pub fn request(s: u32, hour_s: u32) {
    REQ_HOUR_S.store(hour_s, Relaxed);
    REQ_S.store(s, Relaxed);
}

/// At the worker's start: the name the boot joins with (the absent one
/// while an outage kept across the restart lasts).
pub fn boot_login(mut ssid: String) -> String {
    let left = if kept(0).load(Relaxed) == MAGIC { kept(1).load(Relaxed) } else { 0 };
    kept(0).store(0, Relaxed);
    if left > 0 && !ssid.is_empty() {
        ssid.push_str(AWAY);
        HOUR_MS.store(kept(2).load(Relaxed), Relaxed);
        END.store(now_ms().wrapping_add(left).max(1), Relaxed);
        log::warn!("wifi-outage: boot, {} s left", left / 1000);
    }
    ssid
}

/// A driver restart (`Step::Reset`) with the outage under way.
pub fn on_reset(rounds: u32) {
    if END.load(Relaxed) == 0 {
        return;
    }
    let (f, l) = heap();
    log::warn!(
        "wifi-outage: reset rd {rounds} try {} miss {} heap {f}/{l}",
        crate::wifi::reconnect_tries(),
        MISSES.load(Relaxed)
    );
}

/// The firmware restarting to rejoin with the outage under way: its time
/// left kept for the next boot.
pub fn on_restart(heap_rule: bool) {
    let now = now_ms();
    if END.load(Relaxed) == 0 {
        return;
    }
    keep(now);
    log::warn!(
        "wifi-outage: restart {} try {} miss {} {} s left",
        if heap_rule { "wifi_heap" } else { "wifi_stuck" },
        crate::wifi::reconnect_tries(),
        MISSES.load(Relaxed),
        left_ms(now) / 1000
    );
    crate::logring::flush(std::time::Duration::from_millis(300));
}

fn keep(now: u32) {
    kept(1).store(left_ms(now).max(1), Relaxed);
    kept(2).store(HOUR_MS.load(Relaxed), Relaxed);
    kept(0).store(MAGIC, Relaxed);
}

/// One look, from the network worker's loop (about once a second).
/// `ssid`: the name the reconnect joins (changed while the outage lasts);
/// `joined`: up with an address; `misses`: the display's deadline misses.
pub fn tick(ssid: &mut String, again: &mut Reconnect, joined: bool, misses: u64) {
    let now = now_ms();
    MISSES.store(misses as u32, Relaxed);
    let asked = REQ_S.swap(0, Relaxed);
    if asked > 0 {
        if END.load(Relaxed) != 0 {
            log::warn!("wifi-outage: already under way, {} s left", left_ms(now) / 1000);
        } else if ssid.is_empty() {
            log::warn!("wifi-outage: no saved network");
        } else {
            let (f, l) = heap();
            HOUR_MS.store(REQ_HOUR_S.load(Relaxed) * 1000, Relaxed);
            END.store(now.wrapping_add(asked * 1000).max(1), Relaxed);
            OVER_AT.store(0, Relaxed);
            SAID.store(now, Relaxed);
            ssid.push_str(AWAY);
            log::warn!(
                "wifi-outage: start {asked} s hour {} s miss {misses} heap {f}/{l}",
                match HOUR_MS.load(Relaxed) {
                    0 => 3600,
                    h => h / 1000,
                }
            );
            // SAFETY: an argument-free driver call on a started driver.
            unsafe { esp_idf_sys::esp_wifi_disconnect() };
        }
    }
    if END.load(Relaxed) != 0 {
        let hour = HOUR_MS.load(Relaxed);
        if hour > 0 {
            again.set_down_stuck_ms(hour);
        }
        if left_ms(now) == 0 {
            END.store(0, Relaxed);
            kept(0).store(0, Relaxed);
            if let Some(n) = ssid.strip_suffix(AWAY).map(str::len) {
                ssid.truncate(n);
            }
            OVER_AT.store(now.max(1), Relaxed);
            log::warn!("wifi-outage: over, name back, try {} miss {misses}", crate::wifi::reconnect_tries());
        } else {
            keep(now);
            if now.wrapping_sub(SAID.load(Relaxed)) >= 60_000 {
                SAID.store(now, Relaxed);
                let (f, l) = heap();
                log::warn!(
                    "wifi-outage: {} s left try {} miss {misses} heap {f}/{l}",
                    left_ms(now) / 1000,
                    crate::wifi::reconnect_tries()
                );
            }
        }
    }
    let over = OVER_AT.load(Relaxed);
    if over != 0 && joined {
        OVER_AT.store(0, Relaxed);
        let (f, l) = heap();
        log::warn!("wifi-outage: rejoined {} s after, miss {misses} heap {f}/{l}", now.wrapping_sub(over) / 1000);
    }
}

//! Wall-clock time for the schedule: SNTP, the timezone, local time.
//!
//! Lifted from `~/projects/stripe-dashboard/src/time_sync.rs`: SNTP is
//! started by the network worker once the station has an address
//! ([`start_sntp`], kept alive there), and the timezone is a POSIX TZ string
//! (`GMT0IST,M3.5.0/1,M10.5.0` for Europe/Dublin) put in the C environment
//! with `setenv` + `tzset` ([`set_timezone`]), so newlib's `localtime_r`
//! does the DST arithmetic. The string lives in NVS (`tz`), with the name the
//! UI chose it by (`tz_name`) for display only.
//!
//! Until the first sync the clock reads 1970, and the schedule is simply not
//! evaluated ([`local_now`] is `None`): the panel plays as configured.

use esp_idf_svc::sntp::{EspSntp, SntpConf};
use opendmd_core::schedule::LocalTime;
use std::sync::atomic::{AtomicU32, Ordering::Relaxed};
use std::sync::Mutex;

/// Unix seconds of the last SNTP sync, 0 until the first.
static LAST_SYNC: AtomicU32 = AtomicU32::new(0);
/// The zone's display name, for the sync log line.
static TZ_NAME: Mutex<String> = Mutex::new(String::new());

/// Anything before this is "never synced" (the RTC starts at 1970).
const VALID_AFTER: i64 = 1_704_067_200; // 2024-01-01

/// Put `tz` (a POSIX TZ string) into effect for every later `localtime_r`.
/// `name` is only remembered for logs.
pub fn set_timezone(tz: &str, name: &str) {
    let tz = if tz.trim().is_empty() { "UTC0" } else { tz };
    let (Ok(key), Ok(value)) = (std::ffi::CString::new("TZ"), std::ffi::CString::new(tz)) else {
        log::warn!("time: timezone '{tz}' has a NUL in it; ignored");
        return;
    };
    // SAFETY: two valid NUL-terminated strings; newlib copies them. `tzset`
    // takes no arguments. Called from the main task; the httpd's reads of
    // local time race only with a zone change, and see the old or the new.
    unsafe {
        esp_idf_sys::setenv(key.as_ptr(), value.as_ptr(), 1);
        esp_idf_sys::tzset();
    }
    if let Ok(mut n) = TZ_NAME.lock() {
        *n = if name.is_empty() {
            tz.to_string()
        } else {
            name.to_string()
        };
    }
}

fn unix_now() -> i64 {
    // SAFETY: `time(NULL)` has no preconditions.
    unsafe { esp_idf_sys::time(std::ptr::null_mut()) }
}

/// The local broken-down time, if the clock has been set.
fn local_tm() -> Option<esp_idf_sys::tm> {
    let now = unix_now();
    if now < VALID_AFTER {
        return None;
    }
    // SAFETY: `tm` is plain old data; `localtime_r` fills it from `now`.
    let mut tm: esp_idf_sys::tm = unsafe { std::mem::zeroed() };
    let ok = unsafe { !esp_idf_sys::localtime_r(&now, &mut tm).is_null() };
    ok.then_some(tm)
}

/// The local time Formulas read (`hour`, `minute`, …: `opendmd_core::synth`,
/// installed at boot with `synth::set_wall_clock`); `None` before the
/// first sync. Called on the main task: `gettimeofday` + `localtime_r`.
pub fn wall_now() -> Option<opendmd_core::synth::WallTime> {
    // SAFETY: `timeval` is plain old data; a null zone is allowed.
    let mut tv: esp_idf_sys::timeval = unsafe { std::mem::zeroed() };
    if unsafe { esp_idf_sys::gettimeofday(&mut tv, std::ptr::null_mut()) } != 0 {
        return None;
    }
    let now = tv.tv_sec;
    if now < VALID_AFTER {
        return None;
    }
    // SAFETY: as in `local_tm`.
    let mut tm: esp_idf_sys::tm = unsafe { std::mem::zeroed() };
    if unsafe { esp_idf_sys::localtime_r(&now, &mut tm).is_null() } {
        return None;
    }
    Some(opendmd_core::synth::WallTime {
        year: (tm.tm_year + 1900).clamp(0, 9999) as u16,
        month: (tm.tm_mon + 1) as u8,
        day: tm.tm_mday as u8,
        weekday: if tm.tm_wday == 0 { 7 } else { tm.tm_wday as u8 },
        hour: tm.tm_hour as u8,
        minute: tm.tm_min as u8,
        second: tm.tm_sec.min(59) as u8,
        millis: (tv.tv_usec / 1000).clamp(0, 999) as u16,
    })
}

/// Unix seconds (UTC) and the local day of the year (0 on 1 January), for
/// the sky's inputs (`inputs.rs`); `None` before the first sync.
#[optimize(size)] // (CPU0, once a minute)
#[inline(never)]
pub fn utc_and_yday() -> Option<(i64, i64)> {
    let now = unix_now();
    if now < VALID_AFTER {
        return None;
    }
    Some((now, i64::from(local_tm()?.tm_yday)))
}

/// Local weekday and minute, for the schedule; `None` before the first sync.
pub fn local_now() -> Option<LocalTime> {
    local_tm().map(|tm| LocalTime::from_tm(tm.tm_wday, tm.tm_hour, tm.tm_min))
}

/// `"HH:MM"` local, or `None` before the first sync.
pub fn hhmm_now() -> Option<String> {
    hhmm_in(0)
}

/// `"HH:MM"` local, `minutes` from now; `None` before the first sync.
pub fn hhmm_in(minutes: u32) -> Option<String> {
    let t = unix_now() + i64::from(minutes) * 60;
    if t < VALID_AFTER {
        return None;
    }
    // SAFETY: as in `local_tm`.
    let mut tm: esp_idf_sys::tm = unsafe { std::mem::zeroed() };
    let ok = unsafe { !esp_idf_sys::localtime_r(&t, &mut tm).is_null() };
    ok.then(|| format!("{:02}:{:02}", tm.tm_hour, tm.tm_min))
}

/// `"YYYY-MM-DD HH:MM:SS"` local, for logs.
fn stamp() -> String {
    match local_tm() {
        Some(tm) => format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
            tm.tm_year + 1900,
            tm.tm_mon + 1,
            tm.tm_mday,
            tm.tm_hour,
            tm.tm_min,
            tm.tm_sec
        ),
        None => "unset".to_string(),
    }
}

/// Seconds since the last SNTP sync, `None` if there has not been one.
pub fn synced_ago_secs() -> Option<u64> {
    match LAST_SYNC.load(Relaxed) {
        0 => None,
        t => Some((unix_now().max(0) as u64).saturating_sub(u64::from(t))),
    }
}

/// Syncs so far, and how many of them [`log_new_syncs`] has reported.
static SYNCS: AtomicU32 = AtomicU32::new(0);
static SYNCS_LOGGED: AtomicU32 = AtomicU32::new(0);

/// Start SNTP (pool.ntp.org, the IDF default, polled hourly). Call once,
/// from the network worker, after the station is up; keep the handle alive.
///
/// The callback runs on lwip's tcpip task, whose stack is small, so it only
/// records the time; the worker logs it ([`log_new_syncs`]).
pub fn start_sntp() -> Option<EspSntp<'static>> {
    let conf = SntpConf::default();
    match EspSntp::new_with_callback(&conf, |_| {
        LAST_SYNC.store(unix_now().max(1) as u32, Relaxed);
        SYNCS.fetch_add(1, Relaxed);
    }) {
        Ok(sntp) => {
            log::info!("[net] SNTP started");
            Some(sntp)
        }
        Err(e) => {
            log::warn!("[net] SNTP failed to start: {e}");
            None
        }
    }
}

/// Log a sync that has happened since the last call: the local time and the
/// zone. Called from the network worker's poll loop.
pub fn log_new_syncs() {
    let n = SYNCS.load(Relaxed);
    let seen = SYNCS_LOGGED.swap(n, Relaxed);
    if n != seen {
        let name = TZ_NAME.lock().map(|n| n.clone()).unwrap_or_default();
        log::info!(
            "time: NTP {} local time {} ({name})",
            if seen == 0 { "synced," } else { "resynced," },
            stamp()
        );
    }
}

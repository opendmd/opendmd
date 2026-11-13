//! The ESP32-S3's panel output, double-buffered (S3 port phase 3;
//! `cfg(esp32s3)` only, declared at the end of `main.rs` so that the classic
//! build stays byte for byte what it was).
//!
//! On the bench (2026-11-13) racing the beam removed the S3's tearing, but
//! waiting for the beam was the cost: ~84% of blits waited, ~13 ms on
//! average, up to a refresh. The S3 has the internal heap for a second frame
//! buffer and descriptor ring (~38 KB) where WiFi's and lwIP's buffers go to
//! PSRAM (the carrier image: ~110-150 KB internal free while playing,
//! single-buffered), so there its driver runs double-buffered by default
//! (the plain S3 build, WiFi's buffers internal, single: ~38 KB free while
//! playing with no WiFi in the emulator, ~7 KB double) (`hub75_set_buffers`, `hub75_shim.cpp`'s end): every
//! write goes into the back buffer, `hub75_flip` (the end of every push)
//! presents it at the end of the refresh being sent, and the back buffer is
//! not written again until GDMA's OUT_DSCR is seen in the new front ring;
//! then the pairs the last frame wrote are copied across, so the back buffer
//! always holds what is shown plus what was written since — the panel code
//! above it pushes only what changed, as single-buffered. No beam is raced
//! (the scan position reads unknown: no pacer, no Formula beam hooks;
//! Formulas go up through the push as GIFs do), and a push waits only when
//! the last flip has not switched yet (frames closer together than a
//! refresh): that wait is the blit's (`Panel::last_wait_us`), so the frame
//! deadline accounting sees it.
//!
//! - **The switch** (NVS `panel_bufs`, `1` or `2`; else the build's
//!   `OPENDMD_S3_PANEL_BUFFERS`; else the build's default, above): `1` runs single-buffered with the
//!   beam raced, as before; `POST /api/debug/panel?buffers=1|2|default`
//!   writes it and restarts (the bench's single+race vs double).
//! - **The fallback**: too little internal DMA heap at start-up (the second
//!   buffer and ring, and `S3_DBUF_KEEP` left after them), or a start that
//!   fails double-buffered, runs single-buffered with the beam raced, and
//!   says why (the console, `scan.buffers_why`).
//! - **`/api/status` `scan`** says the mode: `buffers` 2 with the flips'
//!   counters ([`scan_json`]), or `buffers` 1 before the classic's fields.
//!   A `s3panel:` console line every 30 s (with `s3mem`'s).

use std::sync::atomic::{AtomicPtr, Ordering::Relaxed};
use std::time::Duration;

use esp_idf_sys::{self as sys, hub75};

use crate::httpd::{query_param, Method, Respond, Server};
use crate::nvs_config::Config;

/// The NVS key (namespace `opendmd`).
const KEY: &str = "panel_bufs";

/// What the shim says now.
fn stats() -> hub75::hub75_s3_dbuf_t {
    // SAFETY: plain integers (all zeros is a value), copied out; any task.
    let mut s: hub75::hub75_s3_dbuf_t = unsafe { core::mem::zeroed() };
    // SAFETY: as above.
    unsafe { hub75::hub75_s3_dbuf(&mut s) };
    s
}

fn parse(v: &str) -> Option<u8> {
    match v.trim() {
        "1" => Some(1),
        "2" => Some(2),
        _ => None,
    }
}

/// Once, before `Panel::new` (`s3board::before_panel`): how many buffers to
/// ask the shim for.
#[inline(never)]
pub fn before_panel(nvs: &Config) {
    let (n, from) = match nvs.s3_string(KEY).as_deref().and_then(parse) {
        Some(n) => (n, "NVS panel_bufs"),
        None => match option_env!("OPENDMD_S3_PANEL_BUFFERS").and_then(parse) {
            Some(n) => (n, "the build's OPENDMD_S3_PANEL_BUFFERS"),
            None => (0, "the build's default: double where WiFi's buffers are in PSRAM"),
        },
    };
    log::info!(
        "panel: {} ({from})",
        match n {
            2 => "double buffering asked",
            1 => "single-buffered asked, the beam raced",
            _ => "buffers not asked",
        }
    );
    // SAFETY: a flag the shim reads in hub75_init, which has not run.
    unsafe { hub75::hub75_set_buffers(n) };
}

/// Before a push writes (`panel::push`, on its `waited` line): the last
/// flip's switch, waited for (µs; 0 when it had happened or single-buffered).
#[inline]
pub fn wait_back() -> u32 {
    // SAFETY: the display task's, as every writer; no arguments.
    unsafe { hub75::hub75_s3_back_ready() }
}

/// The measured refresh (µs) while double-buffered (`panel::refresh_us`'s
/// hook: the scan's row time is 0 then), else `None` (the scan's).
pub fn refresh_us() -> Option<u32> {
    let s = stats();
    (s.buffers == 2).then_some(s.refresh_us)
}

fn why(s: &hub75::hub75_s3_dbuf_t) -> &'static str {
    match u32::from(s.why) {
        hub75::HUB75_S3_WHY_OK => "",
        hub75::HUB75_S3_WHY_ASKED => "asked (panel_bufs 1)",
        hub75::HUB75_S3_WHY_HEAP => "too little internal DMA heap",
        hub75::HUB75_S3_WHY_BEGIN => "the double-buffered start failed",
        hub75::HUB75_S3_WHY_RING => "no second descriptor ring",
        hub75::HUB75_S3_WHY_BUILD => "the build's default (WiFi's buffers in internal RAM)",
        _ => "no panel",
    }
}

/// The task inside [`scan_json`]'s call of `panel::scan_json` (for the
/// classic's fields; null none).
static INNER: AtomicPtr<core::ffi::c_void> = AtomicPtr::new(core::ptr::null_mut());

/// `panel::scan_json`'s hook (its first line): double-buffered, the flips'
/// counters (`{"buffers":2,"refresh_us","flips","flips_skipped",
/// "switched","flip_waits","flip_wait_us","flip_wait_max_us","flips_late",
/// "flips_late_wrap","flip_timeouts","back_hits","copied_pairs","fb_bytes","desc_bytes",
/// "heap":{…},"beam"}`); single-buffered, `"buffers":1,"buffers_why"` put
/// before the classic's fields (`null` stays `null`); `None` from inside
/// that (the classic's own body).
pub fn scan_json(beam: fn() -> String) -> Option<String> {
    // SAFETY: an argument-free FreeRTOS query.
    let me = unsafe { sys::xTaskGetCurrentTaskHandle() }.cast::<core::ffi::c_void>();
    if INNER.load(Relaxed) == me {
        return None;
    }
    let s = stats();
    if s.buffers == 2 {
        return Some(format!(
            r#"{{"buffers":2,"refresh_us":{},"flips":{},"flips_skipped":{},"switched":{},"flip_waits":{},"flip_wait_us":{},"flip_wait_max_us":{},"flips_late":{},"flips_late_wrap":{},"flip_timeouts":{},"back_hits":{},"copied_pairs":{},"fb_bytes":{},"desc_bytes":{},"heap":{},"beam":{}}}"#,
            s.refresh_us,
            s.flips,
            s.skipped,
            s.switched,
            s.waits,
            s.wait_us,
            s.wait_max_us,
            s.late,
            s.late_wrap,
            s.timeouts,
            s.back_hits,
            s.copied_pairs,
            s.fb_bytes,
            s.desc_bytes,
            heap_json(&s),
            beam(),
        ));
    }
    // (Another task inside at the same moment: the classic's fields alone.)
    if INNER.compare_exchange(core::ptr::null_mut(), me, Relaxed, Relaxed).is_err() {
        return None;
    }
    let classic = crate::panel::scan_json();
    INNER.store(core::ptr::null_mut(), Relaxed);
    Some(match classic.strip_prefix('{') {
        Some(rest) => format!(r#"{{"buffers":1,"buffers_why":"{}","heap":{},{rest}"#, why(&s), heap_json(&s)),
        None => classic,
    })
}

fn heap_json(s: &hub75::hub75_s3_dbuf_t) -> String {
    format!(
        r#"{{"before":{},"largest_before":{},"after":{},"largest_after":{}}}"#,
        s.heap_before, s.largest_before, s.heap_after, s.largest_after
    )
}

/// The console line (`s3mem::tick`, every 30 s).
pub fn line() -> String {
    let s = stats();
    if s.buffers == 2 {
        format!(
            "s3panel: buffers 2, flips {}, switched {}, skipped {}, waits {} ({} us in all, max {}), late {} (at the wrap {}), timeouts {}, back hits {}, pairs copied {}",
            s.flips, s.switched, s.skipped, s.waits, s.wait_us, s.wait_max_us, s.late, s.late_wrap, s.timeouts, s.back_hits, s.copied_pairs
        )
    } else {
        format!("s3panel: buffers 1 ({}), the beam raced", why(&s))
    }
}

/// `GET /api/debug/panel` (the scan's JSON) and `POST
/// /api/debug/panel?buffers=1|2|default` (NVS `panel_bufs` written or
/// erased, then a restart): the bench's switch. Registered by `s3mem`.
pub fn routes(server: &mut Server) -> anyhow::Result<()> {
    server.route("/api/debug/panel", Method::Get, |req| req.json(&crate::panel::scan_json()))?;
    server.route("/api/debug/panel", Method::Post, |req| {
        let Some(v) = query_param(req.query(), "buffers") else {
            return req.bad_request("buffers=1, 2 or default");
        };
        let value = match v.as_str() {
            "default" => None,
            v if parse(v).is_some() => Some(v.to_string()),
            _ => return req.bad_request("buffers=1, 2 or default"),
        };
        if let Err(e) = nvs_set(value.as_deref()) {
            return req.error(500, "500 Internal Server Error", &format!("NVS: {e}"));
        }
        log::warn!("panel: panel_bufs {} (POST /api/debug/panel): restarting", value.as_deref().unwrap_or("erased"));
        req.json(&format!(r#"{{"ok":true,"panel_bufs":{},"restarting":true}}"#, value.as_deref().unwrap_or("null")))?;
        std::thread::sleep(Duration::from_millis(500));
        crate::restart(opendmd_core::restarts::Reason::ApiReboot);
    })?;
    Ok(())
}

/// `panel_bufs` written (or erased: `None`) through a handle of its own.
fn nvs_set(value: Option<&str>) -> Result<(), sys::esp_err_t> {
    let mut h: sys::nvs_handle_t = 0;
    // SAFETY: C strings that outlive the calls; the handle is closed below.
    unsafe {
        let e = sys::nvs_open(c"opendmd".as_ptr(), sys::nvs_open_mode_t_NVS_READWRITE, &mut h);
        if e != sys::ESP_OK {
            return Err(e);
        }
        let e = match value {
            Some(v) => {
                let v = std::ffi::CString::new(v).unwrap_or_default();
                sys::nvs_set_str(h, c"panel_bufs".as_ptr(), v.as_ptr())
            }
            None => match sys::nvs_erase_key(h, c"panel_bufs".as_ptr()) {
                sys::ESP_ERR_NVS_NOT_FOUND => sys::ESP_OK,
                e => e,
            },
        };
        let e = if e == sys::ESP_OK { sys::nvs_commit(h) } else { e };
        sys::nvs_close(h);
        if e == sys::ESP_OK {
            Ok(())
        } else {
            Err(e)
        }
    }
}

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
//! - **Refused where it would not fit**: `panel_bufs` 2 in a build whose
//!   default is single (WiFi's buffers internal) is refused and logged (the
//!   heap check at `hub75_init` runs before WiFi exists, so it would let it
//!   through and leave ~7 KB once WiFi runs, every boot), unless
//!   `OPENDMD_S3_DBUF_ANY` at the build or a test build ([`TEST_HOOKS`]).
//! - **Test builds**: NVS `dbuf_test` `heap` or `ring` forces the fallback to
//!   single buffering (the emulator's nocard lane runs `ring`), and
//!   [`self_test`] checks the two frame buffers' contents at start-up.
//! - **`/api/status` `scan`** says the mode: `buffers` 2 with the flips'
//!   counters ([`scan_json`]), or `buffers` 1 before the classic's fields.
//!   A `s3panel:` console line every 30 s (with `s3mem`'s).

use std::sync::atomic::{AtomicPtr, Ordering::Relaxed};
use std::time::Duration;

use esp_idf_sys::{self as sys, hub75};

use crate::httpd::{Method, Respond, Server};
use crate::nvs_config::Config;
use crate::panel::Panel;
use std::sync::atomic::AtomicU32;

/// The NVS key (namespace `opendmd`).
const KEY: &str = "panel_bufs";

/// Test builds: `bench` and `nocard-test` images, the probes, and the
/// emulator's S3 images (`OPENDMD_S3_TEST_HOOKS` at the build, the
/// Makefile's `EMU_S3_BUILD`): the forced fallbacks (NVS `dbuf_test`), the
/// buffer check at start-up ([`self_test`]), and `panel_bufs` 2 honoured
/// whatever the build's default.
const TEST_HOOKS: bool = cfg!(any(feature = "bench", feature = "nocard-test", feature = "probe", feature = "psi-probe"))
    || option_env!("OPENDMD_S3_TEST_HOOKS").is_some();
/// `panel_bufs` 2 honoured in a build whose default is single (WiFi's
/// buffers internal): `OPENDMD_S3_DBUF_ANY` at the build, or a test build.
const DBUF_ANY: bool = TEST_HOOKS || option_env!("OPENDMD_S3_DBUF_ANY").is_some();

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
    let (mut n, mut from) = match nvs.s3_string(KEY).as_deref().and_then(parse) {
        Some(n) => (n, "NVS panel_bufs"),
        None => match option_env!("OPENDMD_S3_PANEL_BUFFERS").and_then(parse) {
            Some(n) => (n, "the build's OPENDMD_S3_PANEL_BUFFERS"),
            None => (0, "the build's default: double where WiFi's buffers are in PSRAM"),
        },
    };
    // The heap check at hub75_init runs before WiFi is up, so it would let
    // 2 through on the plain build and leave ~7 KB of internal heap once
    // WiFi runs, every boot (the key persists): refused there.
    if n == 2 && stats().build_buffers < 2 && !DBUF_ANY {
        log::warn!(
            "panel: {from} asks for double buffering, refused: this build keeps WiFi's buffers in internal RAM (~7 KB of internal heap would be left once WiFi runs); single-buffered, the beam raced (OPENDMD_S3_DBUF_ANY=1 at the build allows it)"
        );
        (n, from) = (0, "refused: the build's default");
    }
    if TEST_HOOKS {
        let test = match nvs.s3_string("dbuf_test").as_deref().map(str::trim) {
            Some("heap") => hub75::HUB75_S3_TEST_HEAP,
            Some("ring") => hub75::HUB75_S3_TEST_RING,
            _ => hub75::HUB75_S3_TEST_NONE,
        };
        if test != hub75::HUB75_S3_TEST_NONE {
            log::warn!("panel: dbuf_test {}: the double-buffered start forced to fall back (a test build's hook)", if test == hub75::HUB75_S3_TEST_HEAP { "heap" } else { "ring" });
            // SAFETY: a flag the shim reads in hub75_init, which has not run.
            unsafe { hub75::hub75_s3_set_test(test as u8) };
        }
    }
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
/// flip's switch, waited for (µs; 0 when it had happened or single-buffered),
/// and any `Panel::clear` waited since the last push ([`clear_wait`]: its
/// wait is the next blit's, usually the same frame's).
#[inline]
pub fn wait_back() -> u32 {
    // SAFETY: the display task's, as every writer; no arguments.
    unsafe { hub75::hub75_s3_back_ready() }.wrapping_add(CLEAR_WAIT.swap(0, Relaxed))
}

/// µs `Panel::clear` waited for a switch since the last push.
static CLEAR_WAIT: AtomicU32 = AtomicU32::new(0);

/// `Panel::clear`'s first step (its first line): the switch waited for
/// before `hub75_clear` writes, kept for the next push's wait.
#[inline]
pub fn clear_wait() {
    // SAFETY: as `wait_back`.
    let w = unsafe { hub75::hub75_s3_back_ready() };
    CLEAR_WAIT.fetch_add(w, Relaxed);
}

/// The measured refresh (µs) while double-buffered (`panel::refresh_us`'s
/// hook: the scan's row time is 0 then), else `None` (the scan's).
pub fn refresh_us() -> Option<u32> {
    let s = stats();
    (s.buffers == 2).then_some(s.refresh_us)
}

/// The buffers, for `main`'s "panel up:" line (`s3board::swallow`).
pub fn mode() -> &'static str {
    if stats().buffers == 2 {
        "buffers=2 (double-buffered)"
    } else {
        "buffers=1 (single-buffered, the beam raced)"
    }
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
/// "flips_late_wrap","flip_timeouts","back_hits","copied_pairs",
/// "shown":{"1","2","3+"} (the refreshes each frame was shown, from its
/// switch to the next), "fb_bytes","back_hits","copied_pairs","fb_bytes","desc_bytes",
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
            r#"{{"buffers":2,"refresh_us":{},"flips":{},"flips_skipped":{},"switched":{},"flip_waits":{},"flip_wait_us":{},"flip_wait_max_us":{},"flips_late":{},"flips_late_wrap":{},"flip_timeouts":{},"back_hits":{},"copied_pairs":{},"shown":{{"1":{},"2":{},"3+":{}}},"fb_bytes":{},"desc_bytes":{},"heap":{},"beam":{}}}"#,
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
            s.shown_1,
            s.shown_2,
            s.shown_3plus,
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
            "s3panel: buffers 2, flips {}, switched {}, skipped {}, waits {} ({} us in all, max {}), late {} (at the wrap {}), timeouts {}, back hits {}, pairs copied {}, shown 1/2/3+ refreshes {}/{}/{}",
            s.flips, s.switched, s.skipped, s.waits, s.wait_us, s.wait_max_us, s.late, s.late_wrap, s.timeouts, s.back_hits, s.copied_pairs, s.shown_1, s.shown_2, s.shown_3plus
        )
    } else {
        format!("s3panel: buffers 1 ({}), the beam raced", why(&s))
    }
}

/// `GET /api/debug/panel` (the scan's JSON) and, in `bench` builds, `POST
/// /api/debug/panel?buffers=1|2|default` (NVS `panel_bufs` written or
/// erased, then a restart as `POST /api/reboot`'s): the bench's switch.
/// Registered by `s3mem`.
pub fn routes(server: &mut Server) -> anyhow::Result<()> {
    server.route("/api/debug/panel", Method::Get, |req| req.json(&crate::panel::scan_json()))?;
    #[cfg(feature = "bench")]
    server.route("/api/debug/panel", Method::Post, |req| {
        use crate::httpd::query_param;
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
        crate::http::reboot_after_response(opendmd_core::restarts::Reason::ApiReboot)
    })?;
    Ok(())
}

/// `panel_bufs` written (or erased: `None`) through a handle of its own.
#[cfg(feature = "bench")]
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

/// The buffer check (test builds, double-buffered; `main`, right after
/// `Panel::new`, on its line): 24 pushes of random rectangles with a clear,
/// a brightness change and the glow on for some of them (its steps push
/// what fades), then the last flip resolved and the front and back frame
/// buffers' checksums compared: they must agree (the back buffer is what is
/// shown plus nothing written since). Then the red variant: one more push
/// whose resolve skips one row pair's copy across, which must leave them
/// different. The panel is cleared after, and the brightness and the look
/// put back.
#[inline(never)]
pub fn self_test(panel: &mut Panel) {
    use opendmd_core::canvas::{Canvas, Rgb, HEIGHT, WIDTH};
    use opendmd_core::look::{GlowLevel, Look};
    if !TEST_HOOKS || stats().buffers != 2 {
        return;
    }
    let mut r: u32 = 0x2545_F491;
    let mut next = move || {
        r ^= r << 13;
        r ^= r >> 17;
        r ^= r << 5;
        r
    };
    let bright = panel.brightness();
    let mut c = Canvas::new();
    let rects = |c: &mut Canvas, next: &mut dyn FnMut() -> u32, n: u32| {
        for _ in 0..n {
            let (x0, y0) = (next() as usize % WIDTH, next() as usize % HEIGHT);
            let (w, h) = (1 + next() as usize % 48, 1 + next() as usize % 12);
            let v = next();
            let col = Rgb::new(v as u8, (v >> 8) as u8, (v >> 16) as u8);
            for y in y0..(y0 + h).min(HEIGHT) {
                for x in x0..(x0 + w).min(WIDTH) {
                    c.set(x, y, col);
                }
            }
        }
    };
    const PUSHES: u32 = 24;
    for i in 0..PUSHES {
        match i {
            6 => {
                panel.clear();
                c.dirty_mut().mark_all();
            }
            10 => panel.set_brightness(bright / 2 + 1),
            14 => {
                let _ = panel.set_display(Look::Colour, GlowLevel::Long);
            }
            20 => {
                let _ = panel.set_display(Look::Colour, GlowLevel::Off);
                c.dirty_mut().mark_all();
            }
            _ => {}
        }
        let n = 1 + next() % 4;
        rects(&mut c, &mut next, n);
        panel.blit(&mut c);
        std::thread::sleep(Duration::from_millis(15));
    }
    let (mut f, mut b) = (0u32, 0u32);
    // SAFETY: out-parameters; the display task's, as every writer.
    let ok = unsafe { hub75::hub75_s3_fb_sums(&mut f, &mut b) };
    log::info!(
        "s3panel: buffer check: {PUSHES} pushes (random rectangles, a clear, a brightness change, glow steps): front {f:08x}, back {b:08x}: {}",
        if ok && f == b { "agree" } else { "DIFFER" }
    );
    // The red variant: every pair written, one pair's copy skipped.
    let pair = (next() % (HEIGHT as u32 / 2)) as u8;
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let v = next();
            c.set(x, y, Rgb::new(v as u8, (v >> 8) as u8, (v >> 16) as u8 | 1));
        }
    }
    // SAFETY: a flag the shim's next resolve reads.
    unsafe { hub75::hub75_s3_skip_copy(pair) };
    panel.blit(&mut c);
    // SAFETY: as above.
    let ok = unsafe { hub75::hub75_s3_fb_sums(&mut f, &mut b) };
    log::info!(
        "s3panel: buffer check red (pair {pair}'s copy skipped): front {f:08x}, back {b:08x}: {}",
        if ok && f != b { "differ, as they must" } else { "AGREE: the check sees nothing" }
    );
    // SAFETY: as above (no skip left over).
    unsafe { hub75::hub75_s3_skip_copy(u8::MAX) };
    panel.set_brightness(bright);
    panel.clear();
}

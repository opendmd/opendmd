//! The HUB75 panel: a safe wrapper over the `hub75_shim` C component.
//!
//! `hub75_shim` is a pure-C facade (see
//! `components/hub75_shim/include/hub75_shim.h`) over
//! mrcodetastic/ESP32-HUB75-MatrixPanel-I2S-DMA 3.0.14, bound into
//! `esp_idf_sys::hub75` by esp-idf-sys's `extra_components` mechanism.
//!
//! Every `unsafe` call into that driver lives in this file and nowhere else.

use anyhow::{bail, Result};
use esp_idf_sys::hub75;
use opendmd_core::canvas::{Canvas, DirtyMap, Rgb, HEIGHT, WIDTH};
use opendmd_core::look::{tint, Glow, GlowLevel, Look, GLOW_STEP};
use opendmd_core::panel_timing::PanelTiming;
use opendmd_core::scan::{self, Judge, Plan, PAIRS};
use std::sync::atomic::Ordering::Relaxed;
use std::time::{Duration, Instant};

use crate::config;
use crate::state::free_heap;

/// Panel geometry, pin map and timing, as Rust-side plain data.
#[derive(Clone, Copy, Debug)]
pub struct PanelConfig {
    pub width: u16,
    pub height: u16,
    pub chain: u16,
    pub driver: i32,
    pub double_buffer: bool,
    pub i2s_speed: i32,
    pub latch_blanking: u8,
    pub clk_phase: bool,
    pub min_refresh_rate: u8,
    pub brightness: u8,
}

impl PanelConfig {
    /// The DMDos board v3 geometry (see `config`) with these timing
    /// settings (`/api/config` `panel`; the defaults are what the firmware
    /// always ran).
    pub fn new(t: &PanelTiming) -> Self {
        PanelConfig {
            width: config::PANEL_WIDTH,
            height: config::PANEL_HEIGHT,
            chain: config::PANEL_CHAIN,
            driver: t.driver.code(),
            double_buffer: config::DOUBLE_BUFFER,
            // The shim's HUB75_SPEED_* are the literal Hz.
            i2s_speed: i32::from(t.i2s_speed) * 1_000_000,
            latch_blanking: t.latch_blanking,
            clk_phase: t.clk_phase,
            min_refresh_rate: t.min_refresh,
            brightness: config::BRIGHTNESS,
        }
    }

    /// Total addressable width: one module times the chain length.
    pub fn surface_width(&self) -> u16 {
        self.width * self.chain
    }

    fn to_ffi(self) -> hub75::hub75_cfg_t {
        hub75::hub75_cfg_t {
            mx_width: self.width,
            mx_height: self.height,
            chain_length: self.chain,
            // hardware/PINOUT.md, measured 2026-09-22.
            r1: config::PIN_R1,
            g1: config::PIN_G1,
            b1: config::PIN_B1,
            r2: config::PIN_R2,
            g2: config::PIN_G2,
            b2: config::PIN_B2,
            a: config::PIN_A,
            b: config::PIN_B,
            c: config::PIN_C,
            d: config::PIN_D,
            e: config::PIN_E,
            lat: config::PIN_LAT,
            oe: config::PIN_OE,
            clk: config::PIN_CLK,
            driver: self.driver,
            double_buff: self.double_buffer,
            i2sspeed: self.i2s_speed,
            latch_blanking: self.latch_blanking,
            clkphase: self.clk_phase,
            min_refresh_rate: self.min_refresh_rate,
        }
    }
}

/// A live HUB75 panel. Constructing one starts the I2S DMA engine; there can
/// only be one (the shim holds a single static instance).
pub struct Panel {
    cfg: PanelConfig,
    /// The brightness ceiling (`max_brightness`): every `set_brightness` is
    /// clamped to it, so no caller can drive the panel above it. Starts at
    /// the safe default until `main` has read the settings.
    max_brightness: u8,
    /// What the last `set_brightness` asked for, before the clamp.
    wanted_brightness: u8,
    /// The DMD tint, applied to every pixel pushed (`opendmd_core::look`).
    look: Look,
    /// Phosphor glow: what the panel shows, while it is held (12 KB).
    glow: Glow,
    /// When the glow last stepped.
    stepped: Instant,
    /// The tint changed: the next blit pushes every pixel (with glow on,
    /// the copy's; the canvas's dirty map is not what decides then).
    repaint: bool,
    /// Glow counters since the last heartbeat: steps, their total and
    /// worst time (µs), pixels they pushed; and allocations since boot.
    pub glow_stats: GlowStats,
    /// Refresh-synchronised blits (`/api/config` `panel_sync`) and the
    /// seam count, whether or not they are on.
    sync: ScanSync,
}

/// The scan position is read, blits judged and (with `panel_sync`)
/// synchronised only in builds with the `panel-sync` feature.
const SCAN: bool = cfg!(feature = "panel-sync");

/// The scan, for refresh-synchronised blits ([`opendmd_core::scan`]):
/// whether to wait for a clear start, and the scan time of a row pair (0:
/// the shim cannot read the scan position, so nothing waits and nothing
/// is counted).
#[derive(Default, Clone, Copy, Debug)]
struct ScanSync {
    on: bool,
    row_us: u32,
}

/// The scan's counters since boot (`/api/status` `scan`, [`scan_json`]):
/// blits the scan position was read for, and of them the ones that left a
/// seam (a row pair first shown complete a refresh after another) and the
/// ones that wrote a pair while it was being scanned; with `panel_sync`
/// on, how many waited, for how long in all (µs, wrapping) and at most,
/// and how many could not (no clear start within the frame's budget).
/// Relaxed atomics: the main
/// task writes them, `GET /api/status` reads them.
mod counters {
    use std::sync::atomic::AtomicU32;
    pub static BLITS: AtomicU32 = AtomicU32::new(0);
    pub static TORN: AtomicU32 = AtomicU32::new(0);
    pub static IN_PAIR: AtomicU32 = AtomicU32::new(0);
    pub static WAITED: AtomicU32 = AtomicU32::new(0);
    pub static WAIT_US: AtomicU32 = AtomicU32::new(0);
    pub static WAIT_MAX_US: AtomicU32 = AtomicU32::new(0);
    pub static GAVE_UP: AtomicU32 = AtomicU32::new(0);
    pub static ROW_US: AtomicU32 = AtomicU32::new(0);
    pub static SYNC_ON: AtomicU32 = AtomicU32::new(0);
}

/// `/api/status` `scan`: `null` when the scan position cannot be read,
/// else `{"row_us","sync","blits","torn","in_pair","waited","wait_us",
/// "wait_max_us","gave_up"}` (counts since boot).
pub fn scan_json() -> String {
    use counters::*;
    use std::sync::atomic::Ordering::Relaxed;
    let row_us = ROW_US.load(Relaxed);
    if !SCAN || row_us == 0 {
        return "null".into();
    }
    format!(
        r#"{{"row_us":{row_us},"sync":{},"blits":{},"torn":{},"in_pair":{},"waited":{},"wait_us":{},"wait_max_us":{},"gave_up":{}}}"#,
        SYNC_ON.load(Relaxed) != 0,
        BLITS.load(Relaxed),
        TORN.load(Relaxed),
        IN_PAIR.load(Relaxed),
        WAITED.load(Relaxed),
        WAIT_US.load(Relaxed),
        WAIT_MAX_US.load(Relaxed),
        GAVE_UP.load(Relaxed),
    )
}

/// Cost bounds for writing a row pair (µs, 240 MHz): the least a pair
/// dirty end to end takes (`hub75_draw_row_pair`: 74 µs measured), and the
/// most any pair takes, untinted and through the tint's staging rows
/// (spans ~0.68 µs a pixel measured). Wide on purpose: `Judge` counts
/// what really happened.
const PAIR_MIN_US: u32 = 50;
const PAIR_MAX_US: u32 = 270;
const PAIR_MAX_US_TINTED: u32 = 450;
/// Keep this far (µs of scan) from a pair while writing it: the
/// descriptor register may already show the next one (16 µs), and
/// reading it is not instant.
const SYNC_MARGIN_US: u32 = 40;
/// Where a clear start is searched, every this many µs of scan.
const SYNC_STEP_US: u32 = 16;
/// Sleep (1 ms ticks) through a wait only down to this much before the
/// clear start; spin the rest on the scan position.
const SYNC_SPIN_US: u32 = 2_200;

/// What the glow cost since the last heartbeat (`Panel::glow_summary`).
#[derive(Default, Clone, Copy, Debug)]
pub struct GlowStats {
    pub steps: u32,
    pub step_us: u64,
    pub step_max_us: u32,
    pub step_px: u64,
    /// Times the 12 KB copy was taken and let go, since boot, and of the
    /// latter how many to low heap ([`Panel::glow_yield`]).
    pub holds: u32,
    pub releases: u32,
    pub yields: u32,
}

/// Two staging rows for a tinted push: the shim reads packed RGB888 rows,
/// and a tint is written into these (on the main task's stack, 768 B) just
/// for the pixels being pushed — no buffer is held.
type Row = [u8; WIDTH * 3];

fn tint_into(dst: &mut Row, src: &[u8], x0: usize, x1: usize, look: Look) {
    for x in x0..x1 {
        let o = x * 3;
        let p = tint(Rgb::new(src[o], src[o + 1], src[o + 2]), look);
        dst[o] = p.r;
        dst[o + 1] = p.g;
        dst[o + 2] = p.b;
    }
}

/// The packed RGB888 bytes of a pixel slice (`Rgb` is `#[repr(C)]`, three
/// `u8`s; see `Canvas::as_bytes`).
fn rgb_bytes(p: &[Rgb]) -> &[u8] {
    // SAFETY: as `Canvas::as_bytes`: `Rgb` is exactly three `u8` with
    // alignment 1 (asserted in `opendmd_core::canvas`).
    unsafe { core::slice::from_raw_parts(p.as_ptr().cast::<u8>(), p.len() * 3) }
}

/// Turn the panel off: OE driven high as a plain GPIO, the DMA stopped
/// (`hub75_blank`). Callable from any task, before or after [`Panel::new`];
/// the panel stays dark until the next boot's `Panel::new`. For
/// [`crate::restart`]; the shim blanks again at the next boot's start-up
/// (`hub75_early_blank`), before `main`.
pub fn blank() {
    // SAFETY: register writes and a flag; no heap, no pointer arguments.
    unsafe { hub75::hub75_blank(config::PIN_OE) };
}

impl Panel {
    pub fn new(cfg: &PanelConfig) -> Result<Panel> {
        let surface = cfg.surface_width() as usize;
        if surface != WIDTH || cfg.height as usize != HEIGHT {
            bail!(
                "panel is {}x{} but the canvas is {WIDTH}x{HEIGHT}",
                surface,
                cfg.height
            );
        }

        let ffi = cfg.to_ffi();
        // SAFETY: `ffi` is a fully initialised POD that outlives the call; the
        // shim copies it into the driver's own config before returning.
        if !unsafe { hub75::hub75_init(&ffi) } {
            bail!("hub75_init failed (DMA allocation, or begin() refused the config)");
        }

        let mut panel = Panel {
            cfg: *cfg,
            max_brightness: config::BRIGHTNESS_SAFE_MAX,
            wanted_brightness: cfg.brightness,
            look: Look::Colour,
            glow: Glow::new(GlowLevel::Off),
            stepped: Instant::now(),
            repaint: false,
            glow_stats: GlowStats::default(),
            sync: ScanSync {
                on: false,
                // SAFETY: the shim's set-up and a value it set; after init.
                row_us: if SCAN {
                    unsafe {
                        hub75::hub75_scan_init();
                        hub75::hub75_scan_row_us()
                    }
                } else {
                    0
                },
            },
        };
        counters::ROW_US.store(panel.sync.row_us, std::sync::atomic::Ordering::Relaxed);
        panel.set_brightness(cfg.brightness);
        panel.clear();
        Ok(panel)
    }

    /// Push what changed on the canvas into the panel's DMA buffer, then
    /// forget it: only the pixels the canvas's [`DirtyMap`] marks are sent,
    /// row by row, one span at a time.
    ///
    /// The panel is single-buffered (`config::DOUBLE_BUFFER`), so the DMA
    /// buffer still holds everything drawn before and a clean pixel needs no
    /// work at all. That matters because pushing a pixel is the expensive
    /// part of a frame: 13.6 ms for all 4,096 through the driver's
    /// `drawPixelRGB888`, more than a 20 ms frame's decode. The shim's
    /// `hub75_draw_span` writes the same bits with the per-row work hoisted
    /// (see `hub75_shim.cpp`), and a GIF or ODM frame usually repaints a
    /// fraction of the screen, so most frames cost a fraction of that. A frame
    /// that changes every pixel (the first of each file, and each loop's
    /// first, which clear the canvas) still pushes all of them.
    ///
    /// The canvas *is* the staging buffer. `Rgb` is `#[repr(C)]` with three
    /// `u8` fields, so `Canvas::as_bytes` is already exactly the packed RGB888
    /// the shim wants.
    ///
    /// Returns the number of pixels pushed. Never waits for the scan; see
    /// [`Panel::blit_within`].
    pub fn blit(&mut self, canvas: &mut Canvas) -> u32 {
        self.blit_within(canvas, 0)
    }

    /// [`Panel::blit`], but with `panel_sync` on it may first wait up to
    /// `budget_us` for a scan position from which the write shows no seam
    /// (`opendmd_core::scan`); with none in reach it writes at once.
    #[inline(never)]
    pub fn blit_within(&mut self, canvas: &mut Canvas, budget_us: u32) -> u32 {
        // Pushing only what changed relies on the buffer being written still
        // holding the previous frame, which is only true single-buffered: a
        // double-buffered back buffer is two frames old.
        if self.cfg.double_buffer {
            canvas.dirty_mut().mark_all();
        }
        if canvas.dirty().is_clean() && !self.glow.is_decaying() {
            return 0;
        }
        // Glow takes its copy only with the heap to spare (and keeps it
        // until `glow_yield` sees the heap low).
        let glowing = self.glow.level() != GlowLevel::Off
            && (self.glow.is_held() || free_heap() as usize >= config::GLOW_TAKE_HEAP);
        let pushed = if !glowing {
            // (The caller marked everything for a new tint.)
            self.repaint = false;
            push(
                canvas.as_bytes(),
                canvas.dirty(),
                self.look,
                &mut self.sync,
                budget_us,
            )
        } else {
            // Glow: a step if one is due, then the new picture over what
            // is shown (`max`), and push what that changed.
            let mut changed = DirtyMap::default();
            if std::mem::take(&mut self.repaint) {
                changed.mark_all();
            }
            let t = Instant::now();
            if t.duration_since(self.stepped) >= GLOW_STEP {
                self.glow.step(canvas, &mut changed);
                self.stepped = t;
            }
            let held = self.glow.is_held();
            if self.glow.frame(canvas, &mut changed) {
                if !held {
                    self.glow_stats.holds += 1;
                    self.stepped = t;
                }
                let shown = self.glow.shown().unwrap_or(canvas.as_slice());
                push(
                    rgb_bytes(shown),
                    &changed,
                    self.look,
                    &mut self.sync,
                    budget_us,
                )
            } else {
                // No room for the copy: the picture as it is.
                push(
                    canvas.as_bytes(),
                    canvas.dirty(),
                    self.look,
                    &mut self.sync,
                    budget_us,
                )
            }
        };
        canvas.dirty_mut().clear();
        pushed
    }

    /// The tint and the glow (`/api/config` `look`, `glow`). True when what
    /// is on the panel no longer matches: the caller marks the picture
    /// dirty and blits it again (every pixel for a new tint; the fading
    /// rows for a glow turned off mid-fade).
    pub fn set_display(&mut self, look: Look, glow: GlowLevel) -> bool {
        let tinted = look != self.look;
        self.look = look;
        self.repaint |= tinted;
        // Live effects hold to 50 fps under the glow (its blit and a
        // 100 fps effect do not fit a 10 ms frame).
        opendmd_core::fx::set_frame_floor(opendmd_core::fx::glow_floor(glow));
        let was_held = self.glow.is_held();
        let refade = self.glow.set_level(glow);
        if was_held && !self.glow.is_held() {
            self.glow_stats.releases += 1;
        }
        tinted || refade
    }

    /// A new file: if the glow's copy could not be allocated before, try
    /// again at its first frame.
    pub fn glow_retry(&mut self) {
        self.glow.retry();
    }

    /// Heap the glow holds now: 12,288 B or 0.
    pub fn glow_bytes(&self) -> usize {
        self.glow.held_bytes()
    }

    /// How long until the glow's next step, while a dot is fading (the
    /// frame loop sleeps no longer than this); `None` when nothing fades.
    pub fn glow_wait(&self) -> Option<Duration> {
        self.glow
            .is_decaying()
            .then(|| GLOW_STEP.saturating_sub(self.stepped.elapsed()))
    }

    /// Between frames: step the glow if a step is due and push what it
    /// changed (`canvas` is the picture on display, unchanged since the
    /// last blit). Returns the µs it took (0 if nothing was due).
    pub fn glow_tick(&mut self, canvas: &Canvas) -> u32 {
        if !self.glow.is_decaying() {
            return 0;
        }
        let t = Instant::now();
        if t.duration_since(self.stepped) < GLOW_STEP {
            return 0;
        }
        self.stepped = t;
        let mut changed = DirtyMap::default();
        self.glow.step(canvas, &mut changed);
        let px = match self.glow.shown() {
            Some(shown) => push(rgb_bytes(shown), &changed, self.look, &mut self.sync, 0),
            None => 0,
        };
        let us = t.elapsed().as_micros() as u32;
        let g = &mut self.glow_stats;
        g.steps += 1;
        g.step_us += u64::from(us);
        g.step_max_us = g.step_max_us.max(us);
        g.step_px += u64::from(px);
        us.max(1)
    }

    /// Nothing is playing (paused, no playlist): once the fade is over, let
    /// the glow's copy go — the panel shows exactly `canvas` then, and a
    /// frame takes it again.
    pub fn glow_settle(&mut self) {
        if self.glow.is_held() && !self.glow.is_decaying() {
            self.glow.release();
            self.glow_stats.releases += 1;
        }
    }

    /// The heap is low (under `config::GLOW_YIELD_HEAP`): give the glow's
    /// copy back now, pushing the rows that were still fading as `canvas`
    /// (the picture on display) so nothing is left half-faded. True if it
    /// let go. Called by the main loop every iteration; a heap read while
    /// the copy is held, nothing otherwise.
    pub fn glow_yield(&mut self, canvas: &Canvas) -> bool {
        if !self.glow.is_held() || free_heap() as usize >= config::GLOW_YIELD_HEAP {
            return false;
        }
        let rows = self.glow.release();
        let mut map = DirtyMap::default();
        for y in 0..HEIGHT {
            if rows & (1 << y) != 0 {
                map.mark_span(y, 0, WIDTH);
            }
        }
        push(canvas.as_bytes(), &map, self.look, &mut self.sync, 0);
        self.glow_stats.releases += 1;
        self.glow_stats.yields += 1;
        true
    }

    /// The glow's heartbeat piece (empty while off): what it holds, and the
    /// steps since the last call.
    pub fn glow_summary(&mut self) -> String {
        if self.glow.level() == GlowLevel::Off && self.look == Look::Colour {
            return String::new();
        }
        let g = std::mem::take(&mut self.glow_stats);
        // Boot totals survive the reset.
        self.glow_stats.holds = g.holds;
        self.glow_stats.releases = g.releases;
        self.glow_stats.yields = g.yields;
        format!(
            ", display {}/{}: glow holds {} B (taken {}, let go {}, {} to low heap), {} steps (avg {} us, max {} us, {} px)",
            self.look.name(),
            self.glow.level().name(),
            self.glow.held_bytes(),
            g.holds,
            g.releases,
            g.yields,
            g.steps,
            g.step_us.checked_div(u64::from(g.steps)).unwrap_or(0),
            g.step_max_us,
            g.step_px
        )
    }

    /// Refresh-synchronised blits on or off (`/api/config` `panel_sync`).
    pub fn set_sync(&mut self, on: bool) {
        self.sync.on = on;
        counters::SYNC_ON.store(u32::from(on), std::sync::atomic::Ordering::Relaxed);
    }

    /// Time the ways of getting a canvas into the DMA buffer, and check that
    /// the span writer produces bit-for-bit what the driver's own per-pixel
    /// path does. Probe builds run this once at boot, before anything else
    /// is on screen; it leaves the panel cleared.
    pub fn bench(&mut self) {
        use std::time::Instant;
        // A busy image with every channel value in use.
        let mut canvas = Canvas::new();
        for y in 0..HEIGHT {
            for x in 0..WIDTH {
                let v = (x * 7 + y * 13) as u8;
                canvas.set(x, y, Rgb::new(v, v.wrapping_mul(3), v ^ 0x5a));
            }
        }
        let bytes = canvas.as_bytes();
        const N: u32 = 10;
        let time = |f: &mut dyn FnMut()| {
            let t = Instant::now();
            for _ in 0..N {
                f();
            }
            t.elapsed().as_micros() as u32 / N
        };
        let full_rows = || {
            for y in 0..HEIGHT {
                let row = bytes[y * WIDTH * 3..].as_ptr();
                // SAFETY: as in `blit`.
                unsafe { hub75::hub75_draw_span(row, y as u16, 0, WIDTH as u16) };
            }
        };

        // SAFETY (all blocks below): the shim reads `bytes` only within
        // WIDTH * HEIGHT * 3 and the coordinates are in range.
        let legacy_us = time(&mut || unsafe {
            hub75::hub75_draw_rgb888(bytes.as_ptr(), WIDTH as u16, HEIGHT as u16)
        });
        let legacy_sum = unsafe { hub75::hub75_fb_checksum() };
        let per_pixel_us = time(&mut || {
            for y in 0..HEIGHT {
                for x in 0..WIDTH {
                    let o = (y * WIDTH + x) * 3;
                    unsafe {
                        hub75::hub75_set_pixel(
                            x as u16,
                            y as u16,
                            bytes[o],
                            bytes[o + 1],
                            bytes[o + 2],
                        )
                    };
                }
            }
        });
        self.clear();
        let span_us = time(&mut || full_rows());
        let span_sum = unsafe { hub75::hub75_fb_checksum() };
        self.clear();
        let pair_us = time(&mut || {
            for y in 0..HEIGHT / 2 {
                let (t, b) = (
                    &bytes[y * WIDTH * 3..],
                    &bytes[(y + HEIGHT / 2) * WIDTH * 3..],
                );
                unsafe { hub75::hub75_draw_row_pair(t.as_ptr(), b.as_ptr(), y as u16) };
            }
        });
        let pair_sum = unsafe { hub75::hub75_fb_checksum() };
        // One 16x16 sprite's worth, the typical partial frame.
        let sprite_us = time(&mut || {
            for y in 8..24 {
                let row = bytes[y * WIDTH * 3..].as_ptr();
                unsafe { hub75::hub75_draw_span(row, y as u16, 56, 72) };
            }
        });
        log::info!(
            "blit bench (us per 128x32 frame, mean of {N}): driver drawPixelRGB888 {legacy_us}, \
             FFI set_pixel {per_pixel_us}, draw_span full {span_us}, draw_row_pair full {pair_us}, \
             draw_span 16x16 {sprite_us}; fb checksum driver {legacy_sum:08x} span {span_sum:08x} \
             pair {pair_sum:08x} -> {}",
            if legacy_sum == span_sum && legacy_sum == pair_sum {
                "identical"
            } else {
                "MISMATCH"
            }
        );
        self.clear();
    }

    /// Apply `min(brightness, max_brightness)`. The one place panel
    /// brightness is set, so the ceiling holds for play, the schedule's
    /// level, boot and anything added later.
    pub fn set_brightness(&mut self, brightness: u8) {
        self.wanted_brightness = brightness;
        let applied = opendmd_core::brightness::effective(brightness, self.max_brightness);
        self.cfg.brightness = applied;
        // SAFETY: no arguments to get wrong; a no-op before init.
        unsafe { hub75::hub75_set_brightness(applied) };
    }

    /// What the panel is driven at now (after the ceiling).
    pub fn brightness(&self) -> u8 {
        self.cfg.brightness
    }

    /// Set the ceiling and re-apply the last requested brightness under it.
    pub fn set_max_brightness(&mut self, max: u8) {
        self.max_brightness = max;
        self.set_brightness(self.wanted_brightness);
    }

    /// Blank the DMA buffer. The canvas does not know: a caller that clears
    /// the panel under a canvas it keeps blitting must `mark_all` it.
    pub fn clear(&mut self) {
        // Dark: nothing to fade, and the copy's 12 KB goes back.
        if self.glow.is_held() {
            self.glow.release();
            self.glow_stats.releases += 1;
        }
        // SAFETY: as above.
        unsafe {
            hub75::hub75_clear();
            hub75::hub75_flip();
        }
    }

    /// The live half of the panel timing: the clock edge (the CLK pin's
    /// inversion in the GPIO matrix) and the OE blanking around LAT (the OE
    /// bits re-rendered at the current brightness). Neither reallocates nor
    /// stops the DMA; the picture in the buffer stays. The rest of
    /// [`PanelTiming`] only takes effect at boot (`Panel::new`).
    pub fn set_live_timing(&mut self, t: &PanelTiming) {
        if t.clk_phase != self.cfg.clk_phase {
            // SAFETY: one GPIO-matrix register write; a no-op before init.
            unsafe { hub75::hub75_set_clk_phase(t.clk_phase) };
            self.cfg.clk_phase = t.clk_phase;
        }
        if t.latch_blanking != self.cfg.latch_blanking {
            // SAFETY: rewrites OE bits inside the driver's own buffer; the
            // shim range-checks the value.
            unsafe { hub75::hub75_set_latch_blanking(t.latch_blanking) };
            self.cfg.latch_blanking = t.latch_blanking;
        }
    }

    /// Scan rate the driver actually achieved, in Hz (0 before init).
    pub fn refresh_rate(&self) -> u8 {
        // SAFETY: reads one `int` out of the driver instance.
        unsafe { hub75::hub75_refresh_rate() }
    }
}

/// Push the pixels `map` marks from `bytes` (packed RGB888, a whole
/// canvas) into the panel's DMA buffer, tinted by `look`, row pair by row
/// pair in ascending order (rows y and y + 16 share every DMA word): a
/// pair dirty end to end through `hub75_draw_row_pair`, anything else as
/// spans. Returns the pixels pushed. With [`Look::Colour`] the shim reads
/// `bytes` directly; otherwise the pixels go through two staging rows on
/// the stack.
///
/// With the scan readable, the position before and after each pair is
/// read and the blit judged (`counters`: did it leave a seam). With
/// `sync.on` and a `budget_us`, it first waits, up to the budget, for a
/// start from which it leaves none (`opendmd_core::scan`).
fn push(bytes: &[u8], map: &DirtyMap, look: Look, sync: &mut ScanSync, budget_us: u32) -> u32 {
    debug_assert_eq!(bytes.len(), WIDTH * HEIGHT * 3);
    let rows = map.row_mask();
    if rows == 0 {
        return 0;
    }
    const HALF: usize = HEIGHT / 2;
    let pairs = (rows & 0xFFFF) | (rows >> HALF);
    let row_of = |y: usize| &bytes[y * WIDTH * 3..(y + 1) * WIDTH * 3];
    let tinted = look != Look::Colour;
    let full_row = |y: usize| rows & (1 << y) != 0 && map.row_spans(y).eq([(0, WIDTH)]);
    if SCAN && sync.on && budget_us > 0 && sync.row_us > 0 {
        let full = (0..HALF)
            .filter(|&p| full_row(p) && full_row(p + HALF))
            .fold(0, |m, p| m | 1 << p);
        wait_for_scan(full, pairs, tinted, sync.row_us, budget_us);
    }
    let scan = SCAN && sync.row_us > 0;
    let mut judge = Judge::new(sync.row_us);
    let (mut a, mut b): (Row, Row) = ([0; WIDTH * 3], [0; WIDTH * 3]);
    let mut pushed = 0;
    for p in 0..HALF {
        if pairs & (1 << p) == 0 {
            continue;
        }
        let before = if scan { scan_pos() } else { None };
        let (top, bottom) = (p, p + HALF);
        if full_row(top) && full_row(bottom) {
            let (t, b) = if tinted {
                tint_into(&mut a, row_of(top), 0, WIDTH, look);
                tint_into(&mut b, row_of(bottom), 0, WIDTH, look);
                (a.as_slice(), b.as_slice())
            } else {
                (row_of(top), row_of(bottom))
            };
            // SAFETY: two full rows (WIDTH * 3 bytes each), p < HEIGHT / 2;
            // read only.
            unsafe { hub75::hub75_draw_row_pair(t.as_ptr(), b.as_ptr(), p as u16) };
            pushed += 2 * WIDTH as u32;
        } else {
            for y in [top, bottom] {
                if rows & (1 << y) == 0 {
                    continue;
                }
                let row = row_of(y);
                for (x0, x1) in map.row_spans(y) {
                    let src = if tinted {
                        tint_into(&mut a, row, x0, x1, look);
                        a.as_slice()
                    } else {
                        row
                    };
                    // SAFETY: `src` is one full row (WIDTH * 3 bytes) and
                    // x0 < x1 <= WIDTH, so the shim reads only inside it;
                    // it only reads. y < HEIGHT.
                    unsafe { hub75::hub75_draw_span(src.as_ptr(), y as u16, x0 as u16, x1 as u16) };
                    pushed += (x1 - x0) as u32;
                }
            }
        }
        if let (Some(before), Some(after)) = (before, scan.then(scan_pos).flatten()) {
            judge.pair(p, before, after);
        }
    }
    if scan {
        counters::BLITS.fetch_add(1, Relaxed);
        counters::TORN.fetch_add(u32::from(judge.straddled), Relaxed);
        counters::IN_PAIR.fetch_add(u32::from(judge.in_pair), Relaxed);
    }
    // SAFETY: no arguments; a no-op when single-buffered.
    unsafe { hub75::hub75_flip() };
    #[cfg(feature = "probe")]
    crate::state::PIXELS_PUSHED.fetch_add(pushed, std::sync::atomic::Ordering::Relaxed);
    pushed
}

/// The scan position, µs of scan time into the refresh, if the shim can
/// read it.
fn scan_pos() -> Option<u32> {
    // SAFETY: one register read in the shim; no arguments.
    let p = unsafe { hub75::hub75_scan_pos() };
    (p != u32::MAX).then_some(p)
}

/// Before a synchronised push: plan its writes (cost bounds per pair: a
/// pair dirty end to end at least [`PAIR_MIN_US`], anything else at least
/// nothing; any pair at most [`PAIR_MAX_US`], more tinted), and if the scan
/// is where they would leave a seam, wait for a clear start — asleep in
/// 1 ms ticks while it is far, then spinning on the scan position —
/// unless it is further than `budget_us`.
#[inline(never)]
fn wait_for_scan(full: u32, pairs: u32, tinted: bool, row_us: u32, budget_us: u32) {
    let max = if tinted {
        PAIR_MAX_US_TINTED
    } else {
        PAIR_MAX_US
    };
    let mut plan = Plan::new();
    for p in (0..PAIRS).filter(|p| pairs & (1 << p) != 0) {
        let min = if full & (1 << p) != 0 { PAIR_MIN_US } else { 0 };
        plan.push(p, min, max);
    }
    let Some(pos) = scan_pos() else { return };
    let wait = match scan::wait_us(pos, &plan, row_us, SYNC_MARGIN_US, SYNC_STEP_US) {
        Some(0) => return,
        Some(w) if w <= budget_us => w,
        _ => {
            counters::GAVE_UP.fetch_add(1, Relaxed);
            return;
        }
    };
    // SAFETY (both): no arguments; the timer is up long before the panel.
    let now = || unsafe { esp_idf_sys::esp_timer_get_time() } as u32;
    let t = now();
    let asleep_ms = wait.saturating_sub(SYNC_SPIN_US) / 1000;
    if asleep_ms > 0 {
        // SAFETY: a FreeRTOS delay of the calling task (1 ms ticks).
        unsafe { esp_idf_sys::vTaskDelay(asleep_ms) };
    }
    // Spin until the scan reaches a clear start; past the wait plus a
    // refresh (it cannot be read any more, or a sleep overran), give up.
    let limit = wait + row_us * PAIRS as u32;
    loop {
        match scan_pos() {
            Some(p) if scan::clear_at(p, &plan, row_us, SYNC_MARGIN_US) => break,
            Some(_) if now().wrapping_sub(t) < limit => core::hint::spin_loop(),
            _ => {
                counters::GAVE_UP.fetch_add(1, Relaxed);
                break;
            }
        }
    }
    let us = now().wrapping_sub(t);
    counters::WAITED.fetch_add(1, Relaxed);
    counters::WAIT_US.fetch_add(us, Relaxed);
    counters::WAIT_MAX_US.fetch_max(us, Relaxed);
}

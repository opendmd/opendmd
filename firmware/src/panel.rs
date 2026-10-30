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
use opendmd_core::scan::{Beam, Budget, Judge, Pacer, Reading, SPLIT_LEAD_US};
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
    /// What the last blit waited for the scan, µs.
    last_wait_us: u32,
    /// The last blit's waits ran past its budget's `fit_us` on purpose
    /// ([`Budget`]: a fast source's frame kept on a refresh boundary).
    last_stretched: bool,
}

/// The scan, for refresh-synchronised blits ([`opendmd_core::scan`]):
/// whether to wait for a clear start, and the scan time of a row pair (0:
/// the shim cannot read the scan position, so nothing waits and nothing
/// is counted).
#[derive(Default, Clone, Copy, Debug)]
struct ScanSync {
    on: bool,
    /// The bottom half a refresh after the top (`panel_split`; only with
    /// `on`): see [`Pacer::next`].
    split: bool,
    row_us: u32,
}

/// The scan's counters since boot (`/api/status` `scan`, [`scan_json`]):
/// blits the scan position was read for, and of them the ones that left a
/// seam (a row pair first shown complete a refresh after another) and the
/// ones that wrote a pair while it was being scanned; with `panel_sync`
/// on, how many waited, for how long in all (µs, wrapping) and at most,
/// and how many could not (no clear start within the frame's budget);
/// how many stretched past the next frame's time on purpose ([`Budget`])
/// and how many frames were late only for that (not deadline misses).
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
    pub static SPLIT: AtomicU32 = AtomicU32::new(0);
    pub static SPLIT_OK: AtomicU32 = AtomicU32::new(0);
    pub static UNSPLIT: AtomicU32 = AtomicU32::new(0);
    pub static SPIN_US: AtomicU32 = AtomicU32::new(0);
    pub static STRETCHED: AtomicU32 = AtomicU32::new(0);
    pub static LATE: AtomicU32 = AtomicU32::new(0);
    pub static SPLIT_ON: AtomicU32 = AtomicU32::new(0);
    pub static ROW_US: AtomicU32 = AtomicU32::new(0);
    pub static SYNC_ON: AtomicU32 = AtomicU32::new(0);
}

/// `/api/status` `scan`: `null` when the scan position cannot be read,
/// else `{"row_us","sync","blits","torn","in_pair","waited","wait_us",
/// "wait_max_us","gave_up","split_on","split","split_ok","unsplit",
/// "spin_us","stretched","late","retimed","beam"}`
/// (counts since boot; `beam` [`beam::json`]).
pub fn scan_json() -> String {
    use counters::*;
    let row_us = ROW_US.load(Relaxed);
    if row_us == 0 {
        return "null".into();
    }
    format!(
        r#"{{"row_us":{row_us},"sync":{},"blits":{},"torn":{},"in_pair":{},"waited":{},"wait_us":{},"wait_max_us":{},"gave_up":{},"split_on":{},"split":{},"split_ok":{},"unsplit":{},"spin_us":{},"stretched":{},"late":{},"retimed":{},"beam":{}}}"#,
        SYNC_ON.load(Relaxed) != 0,
        BLITS.load(Relaxed),
        TORN.load(Relaxed),
        IN_PAIR.load(Relaxed),
        WAITED.load(Relaxed),
        WAIT_US.load(Relaxed),
        WAIT_MAX_US.load(Relaxed),
        GAVE_UP.load(Relaxed),
        SPLIT_ON.load(Relaxed) != 0,
        SPLIT.load(Relaxed),
        SPLIT_OK.load(Relaxed),
        UNSPLIT.load(Relaxed),
        SPIN_US.load(Relaxed),
        STRETCHED.load(Relaxed),
        LATE.load(Relaxed),
        opendmd_core::engine::FX_RETIMED.load(Relaxed),
        beam::json(),
    )
}

/// `scan.beam` alone (psi-probe's play lines).
#[cfg(feature = "psi-probe")]
pub fn beam_json() -> String {
    beam::json()
}

/// The longest one row pair's write may take (µs, 240 MHz), untinted and
/// through the tint's staging rows: what the pacer allows each pair
/// before the scan reaches it (`hub75_draw_row_pair` 74 µs measured for a
/// full pair; spans ~0.68 µs a pixel). `Judge` counts what really happened.
const PAIR_WRITE_US: u32 = 300;
const PAIR_WRITE_US_TINTED: u32 = 450;
/// Keep this far (µs of scan) from a pair's slot while writing it. The
/// scan reading's own error (up to ~102 µs, see `hub75_scan_pos`) is
/// absorbed by [`PAIR_WRITE_US`]'s slack, not by this.
const SYNC_MARGIN_US: u32 = 30;
/// A wait longer than this sleeps a tick (`vTaskDelay(1)`: until the next
/// 1 ms tick, so 0-1 ms) and looks again; shorter ones spin on the scan
/// position. Waking late only delays a pair inside its window (most of a
/// refresh: from its slot before the target to its slot in it), so only a
/// pass's last pair, whose free time the budget was checked against,
/// spins through longer waits (up to [`SYNC_SPIN_LAST_US`]).
const SYNC_SLEEP_US: u32 = 200;
const SYNC_SPIN_LAST_US: u32 = 1_500;

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
            last_wait_us: 0,
            last_stretched: false,
            sync: ScanSync {
                on: true,
                split: true,
                // SAFETY: the shim's set-up and a value it set; after init.
                row_us: unsafe {
                    hub75::hub75_scan_init();
                    hub75::hub75_scan_row_us()
                },
            },
        };
        counters::ROW_US.store(panel.sync.row_us, Relaxed);
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
        self.blit_within(canvas, Budget::NONE)
    }

    /// [`Panel::blit`], but with `panel_sync` on the writes are paced to
    /// the scan so that one refresh shows all of them (`opendmd_core::scan`
    /// [`Pacer`]): pairs the scan has passed at once, the others as it
    /// passes them — waits of up to a refresh (~8 ms, ~3.4 ms on average),
    /// a tick's sleep at a time and spinning only the last 200 µs (1.5 ms
    /// for the last pair). With `panel_split` the bottom rows follow a
    /// refresh later (up to ~16 ms in all), when that fits `budget`'s
    /// `fit_us`. If the waits would run past the budget (with a fast
    /// source's stretch, [`Budget::limit_us`]) it writes at once, as
    /// unsynchronised; the wait is [`Panel::last_wait_us`], a stretch
    /// [`Panel::last_stretched`]. The budget counts from the call (a glow
    /// step's work inside it comes out of it).
    #[inline(never)]
    pub fn blit_within(&mut self, canvas: &mut Canvas, budget: Budget) -> u32 {
        let start = now_us();
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
        let (pushed, waited) = if !glowing {
            // (The caller marked everything for a new tint.)
            self.repaint = false;
            push(
                canvas.as_bytes(),
                canvas.dirty(),
                self.look,
                &mut self.sync,
                budget,
                start,
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
                    budget,
                    start,
                )
            } else {
                // No room for the copy: the picture as it is.
                push(
                    canvas.as_bytes(),
                    canvas.dirty(),
                    self.look,
                    &mut self.sync,
                    budget,
                    start,
                )
            }
        };
        canvas.dirty_mut().clear();
        self.last_wait_us = waited.us;
        self.last_stretched = waited.stretched;
        pushed
    }

    /// The µs the last [`Panel::blit_within`] spent waiting for the scan
    /// (part of its time; the rest is writing).
    pub fn last_wait_us(&self) -> u32 {
        self.last_wait_us
    }

    /// The last [`Panel::blit_within`] waited past its budget's `fit_us`
    /// on purpose (a fast source's frame kept on a refresh boundary,
    /// [`Budget`]).
    pub fn last_stretched(&self) -> bool {
        self.last_stretched
    }

    /// A frame was late only because its blit stretched (the main loop's
    /// judgement: on time but for the wait): counted as `scan` `late`,
    /// not as a deadline miss.
    pub fn note_late(&self) {
        counters::LATE.fetch_add(1, Relaxed);
    }

    /// The tint and the glow (`/api/config` `look`, `glow`). True when what
    /// is on the panel no longer matches: the caller marks the picture
    /// dirty and blits it again (every pixel for a new tint; the fading
    /// rows for a glow turned off mid-fade).
    pub fn set_display(&mut self, look: Look, glow: GlowLevel) -> bool {
        let tinted = look != self.look;
        self.look = look;
        beam::set_look(look);
        self.repaint |= tinted;
        // Formulas hold to 50 fps under the glow (its blit and a
        // 100 fps Formula do not fit a 10 ms frame).
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
    /// last blit). Returns the µs it took, any wait for the scan included
    /// (0 if nothing was due); its `glow_stats` leave the wait out.
    /// With `panel_sync` the push may wait within `budget` for the scan
    /// ([`Panel::blit_within`]).
    pub fn glow_tick(&mut self, canvas: &Canvas, budget: Budget) -> u32 {
        if !self.glow.is_decaying() {
            return 0;
        }
        let t = Instant::now();
        if t.duration_since(self.stepped) < GLOW_STEP {
            return 0;
        }
        self.stepped = t;
        let start = now_us();
        let mut changed = DirtyMap::default();
        self.glow.step(canvas, &mut changed);
        let (px, waited) = match self.glow.shown() {
            Some(shown) => push(
                rgb_bytes(shown),
                &changed,
                self.look,
                &mut self.sync,
                budget,
                start,
            ),
            None => (0, Waited::default()),
        };
        let waited = waited.us;
        // The step's own cost: without the wait for the scan.
        let us = (t.elapsed().as_micros() as u32).saturating_sub(waited);
        let g = &mut self.glow_stats;
        g.steps += 1;
        g.step_us += u64::from(us);
        g.step_max_us = g.step_max_us.max(us);
        g.step_px += u64::from(px);
        us.max(1) + waited
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
        push(
            canvas.as_bytes(),
            &map,
            self.look,
            &mut self.sync,
            Budget::NONE,
            0,
        );
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
    pub fn set_sync(&mut self, on: bool, split: bool) {
        self.sync.on = on;
        self.sync.split = split;
        counters::SPLIT_ON.store(u32::from(on && split), Relaxed);
        counters::SYNC_ON.store(u32::from(on), Relaxed);
    }

    /// Racing the beam for Formulas (`/api/config` `formula_beam`, the
    /// spike; console-lessons §2.6): the player's hooks installed in
    /// `fx_psi` (or taken out). Only with the scan readable.
    pub fn set_beam(&mut self, on: bool) {
        let on = on && self.sync.row_us > 0;
        beam::set(on, self.sync.row_us, self.look);
        #[cfg(feature = "psi")]
        opendmd_core::fx_psi::set_beam(on.then_some(opendmd_core::fx_psi::BeamHooks {
            begin: beam::begin,
            pair: beam::pair,
            end: beam::end,
        }));
    }

    /// Whether a Formula's frame may be raced as far as the panel goes:
    /// the glow composites a whole picture (its frames go up as before).
    #[cfg_attr(not(feature = "psi"), allow(dead_code))]
    pub fn beam_ok(&self) -> bool {
        self.glow.level() == GlowLevel::Off
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
/// spans. Returns the pixels pushed and the µs spent waiting for the scan.
/// With [`Look::Colour`] the shim reads
/// `bytes` directly; otherwise the pixels go through two staging rows on
/// the stack.
///
/// With the scan readable, the position before and after each pair is
/// read and the blit judged (`counters`: did it leave a seam). With
/// `sync.on` and a `budget` (counted from `start`, the clock when the
/// blit began), each pair waits for the [`Pacer`] (the scan to leave its
/// slot before the target refresh), unless the waits would run past the
/// budget's limit (its `fit_us`, or for a fast source its stretch too).
fn push(
    bytes: &[u8],
    map: &DirtyMap,
    look: Look,
    sync: &mut ScanSync,
    budget: Budget,
    start: u32,
) -> (u32, Waited) {
    debug_assert_eq!(bytes.len(), WIDTH * HEIGHT * 3);
    let rows = map.row_mask();
    if rows == 0 {
        return (0, Waited::default());
    }
    const HALF: usize = HEIGHT / 2;
    let (top_rows, bottom_rows) = (rows & 0xFFFF, rows >> HALF);
    let pairs = top_rows | bottom_rows;
    let tinted = look != Look::Colour;
    let scan = sync.row_us > 0;
    // What is left of the budget now (a glow step may have spent some).
    let spent = now_us().wrapping_sub(start);
    let cycle = sync.row_us * opendmd_core::scan::PAIRS as u32;
    let fit_us = budget.fit_us().saturating_sub(spent);
    let limit_us = budget.limit_us(cycle).saturating_sub(spent);
    let plan = if !(scan && sync.on && budget.sync) {
        None
    } else if limit_us > 0 {
        pace_plan(pairs, tinted, sync.row_us, limit_us)
    } else {
        // A frame with no time left at all: not in budget either.
        counters::GAVE_UP.fetch_add(1, Relaxed);
        None
    };
    // Past `fit_us`: a fast source's frame kept on a refresh boundary,
    // the next frame late on purpose ([`Budget`]).
    let stretched = plan.is_some_and(|(_, took)| took > fit_us);
    if stretched {
        counters::STRETCHED.fetch_add(1, Relaxed);
    }
    let pacer = plan.map(|(p, _)| p);
    // Split: the top half now (for refresh T), the bottom half paced to
    // T + 1 — if that still ends inside the budget (never stretched).
    let bottom = pacer
        .filter(|_| sync.split && bottom_rows != 0)
        .and_then(|top| {
            let next = top.next();
            let last = 31 - bottom_rows.leading_zeros() as usize;
            let ends = next.free_at(last).wrapping_sub(now_us());
            let fits = ends < fit_us.saturating_sub(PAIR_WRITE_US);
            if !fits {
                counters::UNSPLIT.fetch_add(1, Relaxed);
            }
            fits.then_some(next)
        });
    let mut judge = pacer.map_or(Judge::new(sync.row_us), |p| Judge::with_clock(p.clock()));
    let mut waited = 0;
    let mut pushed = 0;
    // The first pass: every dirty pair, or only the top rows when split.
    let first = if bottom.is_some() { top_rows } else { pairs };
    let first_last = 31usize.saturating_sub(first.leading_zeros() as usize);
    for p in (0..HALF).filter(|p| first & (1 << p) != 0) {
        if let Some(pacer) = &pacer {
            waited += pace(pacer, p, p == first_last);
        }
        let before = if scan { read_scan() } else { None };
        pushed += if bottom.is_some() {
            write_row(bytes, map, p, look)
        } else {
            write_pair(bytes, map, p, look)
        };
        if let (Some(before), Some(after)) = (before, scan.then(read_scan).flatten()) {
            judge.pair(p, before, after);
        }
    }
    let mut torn = judge.straddled;
    let mut in_pair = judge.in_pair;
    // The second pass: the bottom rows, a refresh after the top's.
    if let Some(next) = bottom {
        let t = now_us();
        let first_bottom = bottom_rows.trailing_zeros() as usize;
        let last = 31 - bottom_rows.leading_zeros() as usize;
        let lead = next.free_at(last).wrapping_sub(SPLIT_LEAD_US);
        let from = if lead.wrapping_sub(next.free_at(first_bottom)) < u32::MAX / 2 {
            lead
        } else {
            next.free_at(first_bottom)
        };
        let asleep = from.wrapping_sub(now_us());
        if (1000..u32::MAX / 2).contains(&asleep) {
            // SAFETY: a FreeRTOS delay of the calling task (1 ms ticks).
            unsafe { esp_idf_sys::vTaskDelay(asleep / 1000) };
        }
        let mut second = Judge::with_clock(next.clock());
        for p in (0..HALF).filter(|p| bottom_rows & (1 << p) != 0) {
            pace(&next, p, p == last);
            let before = read_scan();
            pushed += write_row(bytes, map, p + HALF, look);
            if let (Some(before), Some(after)) = (before, read_scan()) {
                second.pair(p, before, after);
            }
        }
        waited += now_us().wrapping_sub(t);
        let top_refresh = judge.refresh().unwrap_or(next.target() - 1);
        let ok = !second.straddled && second.refresh() == Some(top_refresh + 1);
        torn |= second.straddled;
        in_pair |= second.in_pair;
        counters::SPLIT.fetch_add(1, Relaxed);
        counters::SPLIT_OK.fetch_add(u32::from(ok && !torn), Relaxed);
    }
    // (Nothing read, e.g. after `hub75_blank`: nothing to count.)
    if scan && (judge.refresh().is_some() || torn) {
        counters::BLITS.fetch_add(1, Relaxed);
        counters::TORN.fetch_add(u32::from(torn), Relaxed);
        counters::IN_PAIR.fetch_add(u32::from(in_pair), Relaxed);
        if waited > 0 {
            counters::WAITED.fetch_add(1, Relaxed);
            counters::WAIT_US.fetch_add(waited, Relaxed);
            counters::WAIT_MAX_US.fetch_max(waited, Relaxed);
        }
    }
    // SAFETY: no arguments; a no-op when single-buffered.
    unsafe { hub75::hub75_flip() };
    #[cfg(feature = "probe")]
    crate::state::PIXELS_PUSHED.fetch_add(pushed, Relaxed);
    (
        pushed,
        Waited {
            us: waited,
            stretched,
        },
    )
}

/// What a [`push`] spent waiting for the scan (µs), and whether that ran
/// past its budget's `fit_us` on purpose.
#[derive(Default, Clone, Copy, Debug)]
struct Waited {
    us: u32,
    stretched: bool,
}

/// The dirty pixels of row `y` alone (its half of the pair's DMA words),
/// as spans, tinted. Returns the pixels pushed.
fn write_row(bytes: &[u8], map: &DirtyMap, y: usize, look: Look) -> u32 {
    if map.row_mask() & (1 << y) == 0 {
        return 0;
    }
    let row = &bytes[y * WIDTH * 3..(y + 1) * WIDTH * 3];
    let mut a: Row = [0; WIDTH * 3];
    let mut pushed = 0;
    for (x0, x1) in map.row_spans(y) {
        let src = if look != Look::Colour {
            tint_into(&mut a, row, x0, x1, look);
            a.as_slice()
        } else {
            row
        };
        // SAFETY: `src` is one full row (WIDTH * 3 bytes) and x0 < x1 <=
        // WIDTH, so the shim reads only inside it; it only reads. y < HEIGHT.
        unsafe { hub75::hub75_draw_span(src.as_ptr(), y as u16, x0 as u16, x1 as u16) };
        pushed += (x1 - x0) as u32;
    }
    pushed
}

/// Row pair `p` (rows p and p + 16): through `hub75_draw_row_pair` when
/// both are dirty end to end (one pass over the shared words), else each
/// row's spans. Returns the pixels pushed.
fn write_pair(bytes: &[u8], map: &DirtyMap, p: usize, look: Look) -> u32 {
    const HALF: usize = HEIGHT / 2;
    let rows = map.row_mask();
    let full = |y: usize| rows & (1 << y) != 0 && map.row_spans(y).eq([(0, WIDTH)]);
    if !(full(p) && full(p + HALF)) {
        return write_row(bytes, map, p, look) + write_row(bytes, map, p + HALF, look);
    }
    let row_of = |y: usize| &bytes[y * WIDTH * 3..(y + 1) * WIDTH * 3];
    let (mut a, mut b): (Row, Row) = ([0; WIDTH * 3], [0; WIDTH * 3]);
    let (t, b) = if look != Look::Colour {
        tint_into(&mut a, row_of(p), 0, WIDTH, look);
        tint_into(&mut b, row_of(p + HALF), 0, WIDTH, look);
        (a.as_slice(), b.as_slice())
    } else {
        (row_of(p), row_of(p + HALF))
    };
    // SAFETY: two full rows (WIDTH * 3 bytes each), p < HEIGHT / 2; read
    // only.
    unsafe { hub75::hub75_draw_row_pair(t.as_ptr(), b.as_ptr(), p as u16) };
    2 * WIDTH as u32
}

/// The scan position (µs of scan time into the refresh) and the clock, if
/// the shim can read the position.
fn read_scan() -> Option<Reading> {
    // SAFETY: one register read in the shim; no arguments.
    let pos = unsafe { hub75::hub75_scan_pos() };
    (pos != u32::MAX).then(|| Reading { pos, at: now_us() })
}

/// The microsecond clock, wrapping.
fn now_us() -> u32 {
    // SAFETY: no arguments; the timer is up long before the panel.
    unsafe { esp_idf_sys::esp_timer_get_time() as u32 }
}

/// A synchronised push's pacer and how long its waits run
/// ([`Pacer::plan`]), or `None` (counted as not in budget) when they would
/// run past `budget_us`.
#[inline(never)]
fn pace_plan(pairs: u32, tinted: bool, row_us: u32, budget_us: u32) -> Option<(Pacer, u32)> {
    let now = read_scan()?;
    let write_us = if tinted {
        PAIR_WRITE_US_TINTED
    } else {
        PAIR_WRITE_US
    };
    let plan = Pacer::plan(row_us, now, pairs, write_us, SYNC_MARGIN_US, budget_us);
    if plan.is_none() {
        counters::GAVE_UP.fetch_add(1, Relaxed);
    }
    plan
}

/// Wait until the pacer lets pair `pair` be written: a tick's sleep at a
/// time while the wait is over [`SYNC_SLEEP_US`] (for `last`, the pass's
/// last pair, only while it is over [`SYNC_SPIN_LAST_US`]), spinning on
/// the scan position otherwise. Returns the µs waited; the spinning goes
/// into `counters::SPIN_US`. Past two refreshes and 5 ms (the scan reading
/// gone wrong) it stops waiting.
#[inline(never)]
fn pace(pacer: &Pacer, pair: usize, last: bool) -> u32 {
    let t = now_us();
    let give_up = 2 * pacer.clock().cycle() + 5_000;
    let sleep_over = if last {
        SYNC_SPIN_LAST_US
    } else {
        SYNC_SLEEP_US
    };
    let mut spin_from = None;
    let mut spun = 0;
    while let Some(now) = read_scan() {
        let wait = pacer.wait(pair, now);
        if wait == 0 || now.at.wrapping_sub(t) > give_up {
            break;
        }
        if wait > sleep_over {
            if let Some(f) = spin_from.take() {
                spun += now.at.wrapping_sub(f);
            }
            // SAFETY: a FreeRTOS delay of the calling task, to the next
            // 1 ms tick (0-1 ms).
            unsafe { esp_idf_sys::vTaskDelay(1) };
        } else {
            spin_from.get_or_insert(now.at);
            core::hint::spin_loop();
        }
    }
    let end = now_us();
    if let Some(f) = spin_from {
        spun += end.wrapping_sub(f);
    }
    if spun > 0 {
        counters::SPIN_US.fetch_add(spun, Relaxed);
    }
    end.wrapping_sub(t)
}

/// Racing the beam (`formula_beam`; console-lessons §2.6, §8;
/// `opendmd_core::scan::Beam`): `fx_psi`'s hooks. A Formula's frame is
/// drawn in the panel's pair order and each pair written here as soon as
/// it is drawn, after the scan has left its slot in the refresh before the
/// target and before it comes back: no wait for a clear start, no second
/// pass, no stretch, no tear. The main task only (the display loop).
/// (Without Formulas, `psi`, only its counters are read.)
#[cfg_attr(not(feature = "psi"), allow(dead_code))]
mod beam {
    use super::{now_us, read_scan, write_pair, Beam, Look, Relaxed};
    use opendmd_core::canvas::DirtyMap;
    use opendmd_core::scan::PAIRS;
    use std::cell::UnsafeCell;
    use std::sync::atomic::AtomicU32;

    /// What one pair's write is allowed in the plan (µs): ~74 measured for
    /// a full pair from flash, plus the scan reading's error (a descriptor,
    /// up to ~102 µs).
    const WRITE_US: u32 = 180;
    const WRITE_US_TINTED: u32 = 330;
    /// Kept from a slot's edges (the pacer's `SYNC_MARGIN_US`).
    const MARGIN_US: u32 = 30;

    struct State {
        on: bool,
        row_us: u32,
        look: Look,
        frame: Option<Frame>,
        /// When the first and the last pair were drawn after a frame's
        /// start (µs, waits left out): running means, 1/4 a frame (0: none
        /// yet). What the plan aims the start with.
        first_us: u32,
        last_us: u32,
    }

    #[derive(Clone, Copy)]
    struct Frame {
        beam: Beam,
        err: i32,
        /// The clock when drawing started (after any wait for the window).
        start: u32,
        /// µs waited for the scan: at the start, and for pairs.
        waited: u32,
        pair_waited: u32,
        first: u32,
        last: u32,
        late: bool,
    }

    /// The main task's alone (the display loop calls every function here
    /// but [`json`], which reads only the atomics): no lock a pair.
    struct Cell(UnsafeCell<State>);
    // SAFETY: only the main task touches the state (see above).
    unsafe impl Sync for Cell {}

    static STATE: Cell = Cell(UnsafeCell::new(State {
        on: false,
        row_us: 0,
        look: Look::Colour,
        frame: None,
        first_us: 0,
        last_us: 0,
    }));

    /// The state. Main task only, and no two of these alive at once (each
    /// function here takes it once and calls nothing that takes it).
    #[allow(clippy::mut_from_ref)]
    fn state() -> &'static mut State {
        // SAFETY: as above.
        unsafe { &mut *STATE.0.get() }
    }

    fn mean(m: u32, v: u32) -> u32 {
        if m == 0 {
            v
        } else {
            m - m / 4 + v / 4
        }
    }

    /// Since boot (`/api/status` `scan.beam`, [`json`]).
    static FRAMES: AtomicU32 = AtomicU32::new(0);
    static LATE_FRAMES: AtomicU32 = AtomicU32::new(0);
    static LATE_PAIRS: AtomicU32 = AtomicU32::new(0);
    static WAITS: AtomicU32 = AtomicU32::new(0);
    static PAIR_WAITS: AtomicU32 = AtomicU32::new(0);
    static WAIT_US: AtomicU32 = AtomicU32::new(0);
    static WRITE_TOTAL_US: AtomicU32 = AtomicU32::new(0);
    static PAIR_TOTAL_US: AtomicU32 = AtomicU32::new(0);
    static UNREAD: AtomicU32 = AtomicU32::new(0);
    static ON: AtomicU32 = AtomicU32::new(0);
    static FIRST_US: AtomicU32 = AtomicU32::new(0);
    static LAST_US: AtomicU32 = AtomicU32::new(0);

    pub(super) fn set(on: bool, row_us: u32, look: Look) {
        let st = state();
        st.on = on;
        st.row_us = row_us;
        st.look = look;
        st.frame = None;
        st.first_us = 0;
        st.last_us = 0;
        ON.store(u32::from(on), Relaxed);
    }

    pub(super) fn set_look(look: Look) {
        state().look = look;
    }

    /// `{"on","frames","late_frames","late_pairs","waits","pair_waits",
    /// "wait_us","write_us","pair_us","first_us","last_us","unread"}`:
    /// raced frames since boot, those with a pair written past its window
    /// (`late_pairs` in all: each shows a refresh late, a seam for that
    /// refresh), the frames that waited for their window at the start and
    /// the pairs that waited for theirs (µs in all, wrapping), the µs spent
    /// writing pairs and in the pair hook in all (writes included;
    /// wrapping), the running means of when a frame's first and last pair
    /// are drawn (µs after its start), and the frames not raced because the
    /// scan could not be read.
    pub(super) fn json() -> String {
        format!(
            r#"{{"on":{},"frames":{},"late_frames":{},"late_pairs":{},"waits":{},"pair_waits":{},"wait_us":{},"write_us":{},"pair_us":{},"first_us":{},"last_us":{},"unread":{}}}"#,
            ON.load(Relaxed) != 0,
            FRAMES.load(Relaxed),
            LATE_FRAMES.load(Relaxed),
            LATE_PAIRS.load(Relaxed),
            WAITS.load(Relaxed),
            PAIR_WAITS.load(Relaxed),
            WAIT_US.load(Relaxed),
            WRITE_TOTAL_US.load(Relaxed),
            PAIR_TOTAL_US.load(Relaxed),
            FIRST_US.load(Relaxed),
            LAST_US.load(Relaxed),
            UNREAD.load(Relaxed),
        )
    }

    /// Wait `us`: whole ticks asleep while more than 1.5 ms is left, then a
    /// spin on the clock. Returns the µs waited.
    #[cold]
    fn wait_for(us: u32) -> u32 {
        let t = now_us();
        loop {
            let gone = now_us().wrapping_sub(t);
            if gone >= us {
                return gone;
            }
            if us - gone > 1_500 {
                // SAFETY: a FreeRTOS delay of the calling task (1 ms ticks).
                unsafe { esp_idf_sys::vTaskDelay(1) };
            } else {
                core::hint::spin_loop();
            }
        }
    }

    /// `fx_psi::BeamHooks::begin`.
    pub(super) fn begin(est_us: u32) -> Option<u32> {
        let st = state();
        if !st.on || st.row_us == 0 {
            return None;
        }
        let Some(now) = read_scan() else {
            UNREAD.fetch_add(1, Relaxed);
            return None;
        };
        let write = if st.look == Look::Colour { WRITE_US } else { WRITE_US_TINTED };
        // Measured, else from the governor's mean (a frame's render is at
        // least its pairs' writes).
        let last = if st.last_us > 0 { st.last_us } else { est_us.max(PAIRS as u32 * write) };
        let first = if st.first_us > 0 { st.first_us } else { last / PAIRS as u32 };
        let plan = Beam::plan(st.row_us, now, first, last, write, MARGIN_US);
        let mut waited = 0;
        if plan.wait_us > 0 {
            waited = wait_for(plan.wait_us);
            WAITS.fetch_add(1, Relaxed);
            WAIT_US.fetch_add(waited, Relaxed);
        }
        st.frame = Some(Frame {
            beam: plan.beam,
            err: plan.err,
            start: now_us(),
            waited,
            pair_waited: 0,
            first: 0,
            last: 0,
            late: false,
        });
        Some(st.row_us * PAIRS as u32)
    }

    /// A pair early for its window: spin on the scan until it opens.
    /// Returns the µs waited.
    #[cold]
    #[inline(never)]
    fn wait_window(beam: &Beam, p: usize, w: u32) -> u32 {
        let t = now_us();
        while read_scan().is_some_and(|r| beam.wait(p, r) > 0) && now_us().wrapping_sub(t) < w + 1_000 {
            core::hint::spin_loop();
        }
        let waited = now_us().wrapping_sub(t);
        PAIR_WAITS.fetch_add(1, Relaxed);
        WAIT_US.fetch_add(waited, Relaxed);
        waited
    }

    /// `fx_psi::BeamHooks::pair`: one scan reading before (is its window
    /// open?) and one after (did the write end inside it?).
    pub(super) fn pair(p: usize, bytes: &[u8], dirty: &DirtyMap) {
        let t = now_us();
        let st = state();
        let look = st.look;
        let Some(f) = st.frame.as_mut() else {
            write_pair(bytes, dirty, p, look);
            return;
        };
        let ready = t.wrapping_sub(f.start).saturating_sub(f.pair_waited);
        if p == 0 {
            f.first = ready;
        } else if p == PAIRS - 1 {
            f.last = ready;
        }
        if dirty.row_mask() & ((1u32 << p) | (1 << (p + PAIRS))) == 0 {
            return;
        }
        if let Some(w) = read_scan().map(|r| f.beam.wait(p, r)).filter(|&w| w > 0) {
            let waited = wait_window(&f.beam, p, w);
            f.waited += waited;
            f.pair_waited += waited;
        }
        let w0 = now_us();
        write_pair(bytes, dirty, p, look);
        let w1 = now_us();
        WRITE_TOTAL_US.fetch_add(w1.wrapping_sub(w0), Relaxed);
        if read_scan().is_some_and(|a| f.beam.late(p, a)) {
            f.late = true;
            LATE_PAIRS.fetch_add(1, Relaxed);
        }
        PAIR_TOTAL_US.fetch_add(now_us().wrapping_sub(t), Relaxed);
    }

    /// `fx_psi::BeamHooks::end`.
    pub(super) fn end() -> (i32, u32) {
        let st = state();
        let Some(f) = st.frame.take() else {
            return (0, 0);
        };
        if f.last > 0 {
            st.first_us = mean(st.first_us, f.first);
            st.last_us = mean(st.last_us, f.last);
            FIRST_US.store(st.first_us, Relaxed);
            LAST_US.store(st.last_us, Relaxed);
        }
        FRAMES.fetch_add(1, Relaxed);
        LATE_FRAMES.fetch_add(u32::from(f.late), Relaxed);
        (f.err, f.waited)
    }
}

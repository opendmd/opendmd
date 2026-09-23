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
use opendmd_core::canvas::{Canvas, Rgb, HEIGHT, WIDTH};

use crate::config;

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

impl Default for PanelConfig {
    /// The measured DMDos board v3 configuration (see `config`).
    fn default() -> Self {
        PanelConfig {
            width: config::PANEL_WIDTH,
            height: config::PANEL_HEIGHT,
            chain: config::PANEL_CHAIN,
            driver: config::DRIVER,
            double_buffer: config::DOUBLE_BUFFER,
            i2s_speed: config::I2S_SPEED,
            latch_blanking: config::LATCH_BLANKING,
            clk_phase: config::CLK_PHASE,
            min_refresh_rate: config::MIN_REFRESH_RATE,
            brightness: config::BRIGHTNESS,
        }
    }
}

impl PanelConfig {
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

        let mut panel = Panel { cfg: *cfg };
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
    /// Returns the number of pixels pushed.
    pub fn blit(&mut self, canvas: &mut Canvas) -> u32 {
        // Pushing only what changed relies on the buffer being written still
        // holding the previous frame, which is only true single-buffered: a
        // double-buffered back buffer is two frames old.
        if self.cfg.double_buffer {
            canvas.dirty_mut().mark_all();
        }
        let mut rows = canvas.dirty().row_mask();
        if rows == 0 {
            return 0;
        }
        let bytes = canvas.as_bytes();
        debug_assert_eq!(bytes.len(), WIDTH * HEIGHT * 3);
        let row_of = |y: usize| &bytes[y * WIDTH * 3..(y + 1) * WIDTH * 3];
        let mut pushed = 0;
        // Rows y and y + HALF share every DMA word, so when both are dirty
        // end to end they go in one pass.
        const HALF: usize = HEIGHT / 2;
        for y in 0..HALF {
            let pair = (1u32 << y) | (1u32 << (y + HALF));
            if rows & pair == pair
                && canvas.dirty().row_spans(y).eq([(0, WIDTH)])
                && canvas.dirty().row_spans(y + HALF).eq([(0, WIDTH)])
            {
                // SAFETY: two full canvas rows, y < HEIGHT / 2; read only.
                unsafe {
                    hub75::hub75_draw_row_pair(
                        row_of(y).as_ptr(),
                        row_of(y + HALF).as_ptr(),
                        y as u16,
                    )
                };
                rows &= !pair;
                pushed += 2 * WIDTH as u32;
            }
        }
        while rows != 0 {
            let y = rows.trailing_zeros() as usize;
            rows &= rows - 1;
            let row = row_of(y);
            for (x0, x1) in canvas.dirty().row_spans(y) {
                // SAFETY: `row` is one full canvas row (WIDTH * 3 bytes) and
                // x0 < x1 <= WIDTH, so the shim reads only inside it; it only
                // reads. y < HEIGHT.
                unsafe { hub75::hub75_draw_span(row.as_ptr(), y as u16, x0 as u16, x1 as u16) };
                pushed += (x1 - x0) as u32;
            }
        }
        // SAFETY: no arguments; a no-op when single-buffered.
        unsafe { hub75::hub75_flip() };
        canvas.dirty_mut().clear();
        pushed
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

    pub fn set_brightness(&mut self, brightness: u8) {
        self.cfg.brightness = brightness;
        // SAFETY: no arguments to get wrong; a no-op before init.
        unsafe { hub75::hub75_set_brightness(brightness) };
    }

    /// Blank the DMA buffer. The canvas does not know: a caller that clears
    /// the panel under a canvas it keeps blitting must `mark_all` it.
    pub fn clear(&mut self) {
        // SAFETY: as above.
        unsafe {
            hub75::hub75_clear();
            hub75::hub75_flip();
        }
    }

    /// Scan rate the driver actually achieved, in Hz (0 before init).
    pub fn refresh_rate(&self) -> u8 {
        // SAFETY: reads one `int` out of the driver instance.
        unsafe { hub75::hub75_refresh_rate() }
    }
}

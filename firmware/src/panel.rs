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
use opendmd_core::canvas::{Canvas, HEIGHT, WIDTH};

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

    /// Push a whole canvas and swap buffers: one FFI call to draw it, one flip
    /// to make it visible, so a frame is never half-drawn on screen.
    ///
    /// The canvas *is* the staging buffer. `Rgb` is `#[repr(C)]` with three
    /// `u8` fields, so `Canvas::as_bytes` is already exactly the packed RGB888
    /// the shim wants — the 12 KB copy this used to make was the same bytes in
    /// a different place. Dropping it gave back 12 KB of heap on a board that
    /// has ~30 KB free while playing over WiFi, and took a memcpy out of every
    /// frame.
    pub fn blit(&mut self, canvas: &Canvas) {
        debug_assert_eq!(canvas.as_bytes().len(), WIDTH * HEIGHT * 3);
        // SAFETY: `as_bytes` is exactly WIDTH * HEIGHT * 3 bytes, which is what
        // the shim reads for these dimensions, and the shim only reads it.
        unsafe {
            hub75::hub75_draw_rgb888(canvas.as_bytes().as_ptr(), WIDTH as u16, HEIGHT as u16);
            hub75::hub75_flip();
        }
    }

    pub fn set_brightness(&mut self, brightness: u8) {
        self.cfg.brightness = brightness;
        // SAFETY: no arguments to get wrong; a no-op before init.
        unsafe { hub75::hub75_set_brightness(brightness) };
    }

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

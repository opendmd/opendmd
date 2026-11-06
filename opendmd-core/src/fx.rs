//! What a Formula is to the player: an [`Effect`] (a loop of frames drawn
//! into the canvas), played as a virtual file. The built-in effects
//! (Tunnel, Rotozoom, Starfield, Wireframe) were native code here until
//! 1.0.0-beta.11; they are Formulas now (`opendmd-web/formulas/`: the gallery's),
//! and the one effect left is [`Fx::Psi`], a formula's
//! `@formula?src=…` path ([`crate::synth::FORMULA`], [`crate::fx_psi`]).
//! A retired `@fx:<name>` path names nothing: a playlist line is skipped
//! with a log line ([`crate::synth::is_retired_effect`]).
//!
//! What stays here is what Formulas, the glow and the start-up show
//! borrow: the frame floor the glow sets ([`set_frame_floor`]), the Q14
//! sine ([`SIN`]) and the five palettes ([`PALETTES`]) Psi is lent (one
//! copy on the device, in DRAM: Formulas read them per pixel), and two
//! small helpers the start-up show uses (`hash`, `scale`).
//!
//! **Loops and the dirty map.** An effect is periodic: a frame is a pure
//! function of its time in the loop, so the same frame index gives the
//! same picture on the device, in the simulator and in the browser, and
//! the engine loops it in place (`rewind`). An effect marks what it
//! changes in the canvas's dirty map; `Panel::blit` pushes only that.

use crate::canvas::{Canvas, Rgb};
#[cfg(not(test))]
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Duration;

/// The shortest frame an effect may take (ms; 0 = as asked): see
/// [`set_frame_floor`].
#[cfg(not(test))]
static FLOOR_MS: AtomicU8 = AtomicU8::new(0);

/// Hold every effect to frames of at least `ms`. The firmware, the web
/// previews and the simulator set 20 while the phosphor glow is on
/// (`look::GlowLevel` not `Off`) and 0 otherwise: the glow's blit (~5 ms
/// with a full-screen effect under it) plus a 100 fps effect's render does
/// not fit a 10 ms frame (test panel: +717 misses in 120 s, every one under 5 ms
/// late), while at 50 fps it costs none. The effect's loop keeps its
/// length (a change mid-loop moves it once to the matching phase).
pub fn set_frame_floor(ms: u8) {
    #[cfg(test)]
    TEST_FLOOR.with(|f| f.set(ms));
    #[cfg(not(test))]
    FLOOR_MS.store(ms, Ordering::Relaxed);
}

#[cfg(not(test))]
pub(crate) fn floor() -> u8 {
    FLOOR_MS.load(Ordering::Relaxed)
}

// Tests run in parallel: each thread its own floor.
#[cfg(test)]
thread_local! {
    static TEST_FLOOR: std::cell::Cell<u8> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(crate) fn floor() -> u8 {
    TEST_FLOOR.with(|f| f.get())
}

/// The floor [`set_frame_floor`] asks for the glow level.
pub fn glow_floor(glow: crate::look::GlowLevel) -> u8 {
    if glow == crate::look::GlowLevel::Off {
        0
    } else {
        20
    }
}


/// What every effect is: a name, a loop, and a frame at time `t` into the
/// loop, which returns how long it holds (the engine treats it as a GIF's
/// frame delay).
pub trait Effect {
    /// The effect's name (`psi`).
    fn name(&self) -> &'static str;
    /// Frames in one loop.
    fn loop_frames(&self) -> u32;
    /// Draw the frame at `t` (a multiple of the frame delay, `< loop`) into
    /// `canvas`, marking what it changes; the canvas holds the frame before
    /// it (the one at `t - delay`, or the loop's last at `t = 0`), except on
    /// the first call after opening.
    fn frame(&mut self, t: Duration, canvas: &mut Canvas) -> Duration;
}

// --- the tables Psi is lent (all `static`, built at compile time) ---------

const PI: f64 = std::f64::consts::PI;
const TAU: f64 = std::f64::consts::TAU;

const fn round(v: f64) -> i32 {
    if v >= 0.0 {
        (v + 0.5) as i32
    } else {
        (v - 0.5) as i32
    }
}

const fn sin_f(x: f64) -> f64 {
    let mut x = x - ((x / TAU) as i64 as f64) * TAU;
    if x > PI {
        x -= TAU;
    }
    if x < -PI {
        x += TAU;
    }
    let (mut term, mut sum, mut k) = (x, x, 1);
    while k < 14 {
        term *= -x * x / ((2 * k) as f64 * (2 * k + 1) as f64);
        sum += term;
        k += 1;
    }
    sum
}

/// `sin(2π i / 256)` in Q14 (±16384): the sine Psi is lent.
/// In DRAM on the ESP32 (512 B), with the palettes: Formulas read them per
/// pixel, and read through the flash cache their time moved with the
/// build's layout (opendmd-notes docs/research/psi-speed.md).
#[cfg_attr(target_os = "espidf", link_section = ".dram1.psi")]
pub static SIN: [i16; 256] = {
    let mut t = [0i16; 256];
    let mut i = 0;
    while i < 256 {
        t[i] = round(sin_f(i as f64 * TAU / 256.0) * 16384.0) as i16;
        i += 1;
    }
    t
};


/// A small integer hash (lowbias32), for the start-up show.
pub(crate) const fn hash(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^= x >> 16;
    x
}


const PAL_LEN: usize = 32;

/// 32 entries from nine keys, four linear steps between each pair.
const fn expand(k: [[u8; 3]; 9]) -> [Rgb; PAL_LEN] {
    let mut out = [Rgb::BLACK; PAL_LEN];
    let mut i = 0;
    while i < PAL_LEN {
        let (a, b, f) = (k[i / 4], k[i / 4 + 1], (i % 4) as i32);
        out[i] = Rgb::new(
            (a[0] as i32 + (b[0] as i32 - a[0] as i32) * f / 4) as u8,
            (a[1] as i32 + (b[1] as i32 - a[1] as i32) * f / 4) as u8,
            (a[2] as i32 + (b[2] as i32 - a[2] as i32) * f / 4) as u8,
        );
        i += 1;
    }
    out
}

/// The five palettes Psi is lent, in its order: fire, ice, chrono, amber,
/// rainbow (32 entries of 8-bit RGB from nine keys, a black band; chrono
/// and rainbow cyclic). In DRAM on the ESP32, with [`SIN`].
#[cfg_attr(target_os = "espidf", link_section = ".dram1.psi")]
pub static PALETTES: [[Rgb; PAL_LEN]; 5] = [
    // fire
    expand([
        [0, 0, 0],
        [0, 0, 0],
        [72, 0, 0],
        [150, 12, 0],
        [220, 56, 0],
        [255, 112, 0],
        [255, 184, 24],
        [255, 236, 120],
        [255, 255, 255],
    ]),
    // ice
    expand([
        [0, 0, 0],
        [0, 0, 0],
        [0, 12, 64],
        [0, 40, 128],
        [0, 88, 192],
        [24, 150, 240],
        [110, 205, 255],
        [196, 238, 255],
        [255, 255, 255],
    ]),
    // chrono: black, purple, blue, white and back (cyclic)
    expand([
        [0, 0, 0],
        [48, 0, 88],
        [112, 0, 176],
        [72, 40, 232],
        [32, 120, 255],
        [176, 200, 255],
        [255, 255, 255],
        [128, 56, 224],
        [0, 0, 0],
    ]),
    // amber
    expand([
        [0, 0, 0],
        [0, 0, 0],
        [48, 22, 0],
        [96, 46, 0],
        [148, 76, 6],
        [200, 108, 14],
        [240, 146, 36],
        [255, 188, 88],
        [255, 222, 160],
    ]),
    // rainbow, through black (cyclic)
    expand([
        [0, 0, 0],
        [255, 0, 0],
        [255, 160, 0],
        [200, 255, 0],
        [0, 255, 80],
        [0, 160, 255],
        [80, 0, 255],
        [255, 0, 200],
        [0, 0, 0],
    ]),
];


/// `c` at `k` / 256.
#[inline]
pub(crate) fn scale(c: Rgb, k: u32) -> Rgb {
    Rgb::new(
        ((u32::from(c.r) * k) >> 8) as u8,
        ((u32::from(c.g) * k) >> 8) as u8,
        ((u32::from(c.b) * k) >> 8) as u8,
    )
}


/// The prefix every built-in effect's path had (`@fx:tunnel?…`): gone, see
/// [`crate::synth::is_retired_effect`].
pub const RETIRED_PREFIX: &str = "@fx:";

// --- the effect a path names -------------------------------------------------

/// The effect a path names: a Psi formula (the `psi` feature), whose
/// program and working memory are one allocation made when it is opened
/// ([`crate::fx_psi`]). Without `psi` there is none (the main wasm module:
/// the browser previews Formulas through its own module).
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(not(feature = "psi"), derive(Copy))]
pub enum Fx {
    #[cfg(feature = "psi")]
    Psi(crate::fx_psi::PsiBox),
}

const _: () = assert!(std::mem::size_of::<Fx>() <= 512);

impl Fx {
    fn effect(&mut self) -> &mut dyn Effect {
        match *self {
            #[cfg(feature = "psi")]
            Fx::Psi(ref mut e) => &mut **e,
        }
    }

    pub fn name(&self) -> &'static str {
        match *self {
            #[cfg(feature = "psi")]
            Fx::Psi(ref e) => e.name(),
        }
    }

    /// What the Library and `now_playing` call it.
    pub fn title(&self) -> &'static str {
        match *self {
            #[cfg(feature = "psi")]
            Fx::Psi(_) => "Formula",
        }
    }

    pub fn loop_frames(&mut self) -> u32 {
        self.effect().loop_frames()
    }

    /// One loop's length as played: what [`crate::synth::SynthSource`]
    /// counts frame times against (a formula's frames may be held longer
    /// than [`delay`](Self::delay): `fx_psi::Governor`).
    pub fn loop_time(&mut self) -> Duration {
        match *self {
            #[cfg(feature = "psi")]
            Fx::Psi(ref e) => e.loop_time(),
        }
    }

    /// The frame delay.
    pub fn delay(&self) -> Duration {
        match *self {
            #[cfg(feature = "psi")]
            Fx::Psi(ref e) => e.delay(),
        }
    }

    pub fn frame(&mut self, t: Duration, canvas: &mut Canvas) -> Duration {
        self.effect().frame(t, canvas)
    }

    /// The formula a `@formula?src=…` path names, `None` if it names none
    /// or does not compile (always `None` without the `psi` feature).
    pub fn parse(path: &str) -> Option<Fx> {
        if !crate::synth::is_formula(path) {
            return None;
        }
        #[cfg(feature = "psi")]
        {
            crate::fx_psi::PsiFx::from_query(&path[crate::synth::FORMULA.len()..])
                .ok()
                .map(Fx::Psi)
        }
        #[cfg(not(feature = "psi"))]
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables() {
        assert_eq!(SIN[0], 0);
        assert_eq!(SIN[64], 16384);
        assert_eq!(SIN[128], 0);
        assert_eq!(SIN[192], -16384);
        assert_eq!(SIN[32], 11585);
        for p in &PALETTES {
            assert_eq!(p[0], Rgb::BLACK);
        }
        // Nothing inline: no effect without Psi, a pointer with it.
        #[cfg(not(feature = "psi"))]
        assert_eq!(std::mem::size_of::<Fx>(), 0);
        #[cfg(feature = "psi")]
        assert!(std::mem::size_of::<Fx>() <= 16);
    }

    #[test]
    fn retired_effects_and_other_paths_name_nothing() {
        for p in ["@fx:tunnel", "@fx:psi?src=gray(x)", "@fx:wire?shape=cube", "@formulax?src=1", "@card:A|", "/gifs/a.gif"] {
            assert!(Fx::parse(p).is_none(), "{p}");
        }
    }

    #[test]
    fn the_glow_floor() {
        assert_eq!(glow_floor(crate::look::GlowLevel::Off), 0);
        assert_eq!(glow_floor(crate::look::GlowLevel::Long), 20);
        set_frame_floor(20);
        assert_eq!(floor(), 20);
        set_frame_floor(0);
        assert_eq!(floor(), 0);
    }
}

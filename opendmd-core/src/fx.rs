//! Live effects: pictures computed every frame, played as virtual files.
//!
//! `@fx:<name>?k=v&...` is a virtual file like `@clock` ([`crate::synth`]
//! parses it, [`SynthSource`](crate::synth::SynthSource) plays it): it gets
//! transitions, dwell, `now_playing`, the browser preview and the looks for
//! free, because an effect only ever draws frames into the canvas. Four
//! ship: [`Tunnel`] (the per-pixel angle/depth tunnel in two layers, lit,
//! its palette cycling: the Chrono Trigger time warp), [`Rotozoom`] (Mode
//! 7: two textures turned and scaled against each other with the warp's
//! sine wobble, or `floor=1` as a perspective floor), [`Starfield`] (warp
//! streaks) and [`Wire`] (wireframe solids lit by depth, a 4D tesseract, a
//! sphere-torus-flower morph).
//!
//! **The rules.** No heap: an effect's state is its options and a flag,
//! inline in the decoder ([`Fx`] is asserted ≤ 512 B; it is 10 B), and
//! every table — the sine, the palettes, the textures, the tunnel's
//! per-pixel lookup — is a `static` computed at compile time by a `const
//! fn`, so it lives in flash on the device (13,338 B in all,
//! [`TABLE_BYTES`]); what a frame needs besides is on the stack (≤ ~1.3
//! KB: the tunnel's lit palette, the wobble's column table, the wire's
//! vertices).
//! The frame path is integer and fixed-point: a loop's phase is 0..65,536,
//! sines are Q14 from [`SIN`] (256 entries, interpolated by `sin16`), the
//! tunnel's angles 1,024 to a turn and its depths quarter texels, texture
//! coordinates 16.16.
//!
//! **Loops.** Every effect is periodic: a frame is a pure function of its
//! phase in the loop (`n` of `N` frames), and everything that moves
//! completes a whole number of cycles per loop (a turn, a texture's
//! length, a palette), so frame `N` would equal frame 0. A loop is what
//! dwell counts ([`Fx::loop_frames`]; `speed=` shortens it), and the engine
//! loops an effect in place (`rewind`), seamlessly. The same frame index
//! gives the same picture on the device, in the simulator and in the
//! browser, and seeking (the preview following the panel) is decoding the
//! frames in between.
//!
//! **The dirty map.** The full-frame effects (tunnel, rotozoom) write every
//! pixel and mark every pixel — `Panel::blit` then pushes whole row pairs,
//! its cheapest path. The sparse ones (starfield, wire) first draw the
//! previous frame again in black (it is a pure function of the previous
//! phase, so nothing is remembered) and then the new one, writing only
//! pixels that differ; the first frame after opening clears the canvas.
//!
//! **Frame rate.** Every effect defaults to 100 fps (10 ms frames, the
//! panel's refresh rate; `fps=50|25` for 20 or 40 ms): each renders in
//! ≤ 3.5 ms p90 on the rig, which leaves the frame room for the ~1.3 ms
//! blit. The 20 ms floor is the GIF decoders' (`player::MIN_DELAY_MS`), not
//! the engine's, so a virtual file's delay is taken as it is; the engine
//! keeps an effect's schedule rather than timing each frame from its tick
//! (`Engine::advance`), and the glow holds effects to 50 fps
//! ([`set_frame_floor`]).
//!
//! **Palettes** are 32 entries of 8-bit RGB from nine keys, with a black
//! band, and cyclic where cycling is meant (chrono, rainbow): smooth
//! gradients band on the panel at low brightness, so they are stepped.
//! The panel's tint ([`crate::look`]) applies afterwards as to any picture.

use crate::canvas::{Canvas, Rgb, HEIGHT, WIDTH};
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
/// not fit a 10 ms frame (rig: +717 misses in 120 s, every one under 5 ms
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

/// The prefix of an effect's path: `@fx:tunnel?palette=chrono&cycle=1`.
pub const PREFIX: &str = "@fx:";

/// What every effect is: a name, a loop, and a frame at time `t` into the
/// loop, which returns how long it holds (the engine treats it as a GIF's
/// frame delay).
pub trait Effect {
    /// The effect's name in its path (`tunnel`, ...).
    fn name(&self) -> &'static str;
    /// Frames in one loop.
    fn loop_frames(&self) -> u32;
    /// Draw the frame at `t` (a multiple of the frame delay, `< loop`) into
    /// `canvas`, marking what it changes; the canvas holds the frame before
    /// it (the one at `t - delay`, or the loop's last at `t = 0`), except on
    /// the first call after opening.
    fn frame(&mut self, t: Duration, canvas: &mut Canvas) -> Duration;
}

// --- tables (all `static`, built at compile time) -------------------------

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

const fn sqrt_f(x: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    let mut r = if x > 1.0 { x } else { 1.0 };
    let mut i = 0;
    while i < 40 {
        r = 0.5 * (r + x / r);
        i += 1;
    }
    r
}

/// atan of `t` in `[0, 1]`: halve the angle twice, then the series.
const fn atan01(t: f64) -> f64 {
    let t1 = t / (1.0 + sqrt_f(1.0 + t * t));
    let t2 = t1 / (1.0 + sqrt_f(1.0 + t1 * t1));
    let (mut term, mut sum, mut k) = (t2, t2, 1);
    while k < 16 {
        term *= -t2 * t2;
        sum += term / (2 * k + 1) as f64;
        k += 1;
    }
    4.0 * sum
}

/// atan2 for `y, x > 0`: `[0, π/2)`.
const fn atan2_q(y: f64, x: f64) -> f64 {
    if y <= x {
        atan01(y / x)
    } else {
        PI / 2.0 - atan01(x / y)
    }
}

/// `sin(2π i / 256)` in Q14 (±16384): the one sine every effect shares.
pub static SIN: [i16; 256] = {
    let mut t = [0i16; 256];
    let mut i = 0;
    while i < 256 {
        t[i] = round(sin_f(i as f64 * TAU / 256.0) * 16384.0) as i16;
        i += 1;
    }
    t
};

#[inline]
fn sin(a: u8) -> i32 {
    i32::from(SIN[usize::from(a)])
}

/// The sine at `ph` (65,536 to a turn), Q14, interpolated between the
/// table's entries: smooth motion at any speed.
#[inline]
fn sin16(ph: u32) -> i32 {
    let (i, f) = ((ph >> 8) as u8, (ph & 255) as i32);
    let (a, b) = (sin(i), sin(i.wrapping_add(1)));
    a + (((b - a) * f) >> 8)
}

#[inline]
fn cos16(ph: u32) -> i32 {
    sin16(ph.wrapping_add(16384))
}

/// A small integer hash (lowbias32), for the textures and the stars.
pub(crate) const fn hash(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^= x >> 16;
    x
}

/// The palettes, by `palette=` name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Palette {
    Fire,
    Ice,
    /// Purple, blue and white, cyclic: the time warp.
    Chrono,
    Amber,
    Rainbow,
}

impl Palette {
    pub const ALL: [Palette; 5] = [
        Palette::Fire,
        Palette::Ice,
        Palette::Chrono,
        Palette::Amber,
        Palette::Rainbow,
    ];

    pub fn name(self) -> &'static str {
        ["fire", "ice", "chrono", "amber", "rainbow"][self as usize]
    }

    fn from_name(v: &[u8]) -> Option<Palette> {
        Some(match v {
            b"fire" => Palette::Fire,
            b"ice" => Palette::Ice,
            b"chrono" => Palette::Chrono,
            b"amber" | b"mono-amber" => Palette::Amber,
            b"rainbow" => Palette::Rainbow,
            _ => return None,
        })
    }

    pub(crate) fn table(self) -> &'static [Rgb; PAL_LEN] {
        &PALETTES[self as usize]
    }
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

pub(crate) static PALETTES: [[Rgb; PAL_LEN]; 5] = [
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

/// Texture size (texels): `TEX_W x TEX_H` bytes, values 0..=255 (a
/// palette index × 8). 64 x 32, not 64 x 64: see [`TABLES`].
const TEX_W: usize = 64;
const TEX_H: usize = 32;
const TEX: usize = TEX_W * TEX_H;

/// Texels are palette indices × 8: the low bits would never show, and
/// clearing them makes the tables compress (the browser downloads them).
const STEP: u8 = !7;

/// The textures, by `texture=` name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Texture {
    Checker,
    Rings,
    Noise,
}

impl Texture {
    fn from_name(v: &[u8]) -> Option<Texture> {
        Some(match v {
            b"checker" => Texture::Checker,
            b"rings" => Texture::Rings,
            b"noise" => Texture::Noise,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        ["checker", "rings", "noise"][self as usize]
    }

    fn table(self) -> &'static [u8; TEX] {
        &TABLES.textures[self as usize]
    }
}

const fn checker() -> [u8; TEX] {
    let mut t = [0u8; TEX];
    let mut i = 0;
    while i < TEX {
        let (u, v) = (i % TEX_W, i / TEX_W);
        let odd = ((u >> 3) ^ (v >> 3)) & 1;
        // Flat cells a palette apart (with a one-step bevel on two
        // edges), so a non-monotonic palette still reads as a checker.
        let edge = (u & 7 == 0 || v & 7 == 0) as usize;
        t[i] = (if odd == 1 { 200 } else { 72 } - edge * 16) as u8;
        i += 1;
    }
    t
}

const fn rings() -> [u8; TEX] {
    let mut t = [0u8; TEX];
    let mut i = 0;
    while i < TEX {
        let du = (i % TEX_W) as f64 - 31.5;
        let dv = (i / TEX_W) as f64 - 15.5;
        let d = sqrt_f(du * du + dv * dv);
        t[i] = (round(d * 16.0) & 255) as u8 & STEP;
        i += 1;
    }
    t
}

/// Value noise that tiles: an 8-texel lattice plus a 4-texel one at half
/// the weight, smoothstepped, scaled to 0..=255 — computed at half the
/// resolution and doubled (2x2 texels), which halves what it costs the
/// browser's download (gzip finds the repeats) for no visible difference.
const fn noise() -> [u8; TEX] {
    const fn lattice(cell: usize, i: usize, j: usize, seed: u32) -> i32 {
        let (cu, cv) = (TEX_W / 2 / cell, TEX_H / 2 / cell);
        let (i, j) = (i % cu, j % cv);
        (hash((i + j * cu) as u32 ^ seed) & 255) as i32
    }
    const fn octave(u: usize, v: usize, cell: usize, seed: u32) -> i32 {
        let cells = cell;
        let (i, j) = (u / cell, v / cell);
        // smoothstep weights in 1/256
        let fu = ((u % cell) * 256 / cell) as i32;
        let fv = ((v % cell) * 256 / cell) as i32;
        let su = fu * fu * (768 - 2 * fu) / 65536;
        let sv = fv * fv * (768 - 2 * fv) / 65536;
        let a = lattice(cells, i, j, seed);
        let b = lattice(cells, i + 1, j, seed);
        let c = lattice(cells, i, j + 1, seed);
        let d = lattice(cells, i + 1, j + 1, seed);
        let top = a + (b - a) * su / 256;
        let bot = c + (d - c) * su / 256;
        top + (bot - top) * sv / 256
    }
    let mut raw = [0i32; TEX];
    let (mut lo, mut hi) = (i32::MAX, i32::MIN);
    let mut i = 0;
    while i < TEX {
        let (u, v) = (i % TEX_W / 2, i / TEX_W / 2);
        let n = 2 * octave(u, v, 4, 0x5eed) + octave(u, v, 2, 0xbeef);
        raw[i] = n;
        if n < lo {
            lo = n;
        }
        if n > hi {
            hi = n;
        }
        i += 1;
    }
    let mut t = [0u8; TEX];
    let mut i = 0;
    while i < TEX {
        t[i] = ((raw[i] - lo) * 255 / (hi - lo)) as u8 & STEP;
        i += 1;
    }
    t
}

/// The tunnel's lookup covers one quadrant around the vanishing point,
/// with room for the drift: `QW x QH` pixels (a pixel's offset from the
/// centre, mirrored into the quadrant).
const QW: usize = WIDTH / 2 + DRIFT_X as usize;
const QH: usize = HEIGHT / 2 + DRIFT_Y as usize;
/// How far `drift=1` moves the vanishing point (px, each way).
const DRIFT_X: i32 = 16;
const DRIFT_Y: i32 = 8;
/// Depth = `TUNNEL_K / r`, with `r` measured on an ellipse twice as tall
/// as wide (the panel is 4:1; a round tunnel would be all sides).
const TUNNEL_K: f64 = 1024.0;

/// Per quadrant pixel: the angle (0..256 of the 1,024 in a turn); and the
/// depth in quarter texels (`4 TUNNEL_K / r`, 1,023 at most) in the low ten
/// bits with the fog level (0 = black .. 3 = full) in the top two. The
/// quarters are what make the travel and the turn smooth: a whole-texel
/// table steps visibly at the edges.
#[repr(C)]
struct TunnelTable {
    angle: [u8; QW * QH],
    depth: [u16; QW * QH],
}

const fn tunnel_table() -> TunnelTable {
    let mut t = TunnelTable {
        angle: [0; QW * QH],
        depth: [0; QW * QH],
    };
    let mut i = 0;
    while i < QW * QH {
        let fx = (i % QW) as f64 + 0.5;
        let fy = 2.0 * ((i / QW) as f64 + 0.5);
        let a = round(atan2_q(fy, fx) * 1024.0 / TAU);
        let a = if a > 255 { 255 } else { a };
        let d = round(4.0 * TUNNEL_K / sqrt_f(fx * fx + fy * fy));
        let d = if d > 1023 { 1023 } else { d };
        let fog = if d < 256 {
            3
        } else if d < 384 {
            2
        } else if d < 512 {
            1
        } else {
            0
        };
        t.angle[i] = a as u8;
        t.depth[i] = (d | (fog << 14)) as u16;
        i += 1;
    }
    t
}

/// The tunnel's table and the textures, together: the tunnel reads its
/// table row by row and a texture at random, both through the flash cache.
/// Apart, wherever the linker put them, a texture that happened to sit a
/// multiple of the cache's way size from the table fought it (and the code)
/// for the same cache sets: `texture=noise` (then the third 4 KB texture)
/// cost 3.4-4.2 ms a frame on the rig against 2.2 ms for the checker,
/// and swapping the two textures' places swapped their costs. One struct
/// of 11,904 B (hence 64 x 32 textures) is smaller than a way (16 KB), so
/// the working set can no longer evict itself.
#[repr(C)]
struct Tables {
    tunnel: TunnelTable,
    textures: [[u8; TEX]; 3],
}

// Smaller than a way of the ESP32's flash cache (see above).
const _: () = assert!(std::mem::size_of::<Tables>() < 16 * 1024);

static TABLES: Tables = Tables {
    tunnel: tunnel_table(),
    textures: [checker(), rings(), noise()],
};

/// Every effect table's size in bytes (flash on the device).
pub const TABLE_BYTES: usize = std::mem::size_of::<[i16; 256]>()
    + std::mem::size_of::<[[Rgb; PAL_LEN]; 5]>()
    + std::mem::size_of::<Tables>()
    + std::mem::size_of::<[u8; 256]>()
    + WIRE_BYTES;

// The flash the effects may take.
const _: () = assert!(TABLE_BYTES <= 24 * 1024);

// --- options ----------------------------------------------------------------

/// What every effect takes: `speed=1..4` (default 2: a loop is its base
/// length / speed), `fps=100|50|25` (the effect's default: see each),
/// `palette=` and `cycle=0|1` (palette cycling: one full turn of the
/// palette a loop).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Common {
    pub speed: u8,
    /// The frame delay: 10, 20 or 40 ms.
    pub delay_ms: u8,
    pub palette: Palette,
    pub cycle: bool,
}

impl Common {
    /// The frame delay in ms: as asked, or the floor if that is longer.
    fn ms(&self) -> u32 {
        u32::from(self.delay_ms.max(floor()))
    }

    fn delay(&self) -> Duration {
        Duration::from_millis(u64::from(self.ms()))
    }

    /// Frames in a loop whose length at speed 1 and 50 fps is `base`
    /// frames (a multiple of 24, so every speed and frame rate divide it).
    fn frames(&self, base: u32) -> u32 {
        base * 20 / self.ms() / u32::from(self.speed)
    }

    /// The frame index at `t`, and one step back (the loop's last frame
    /// before frame 0), in a loop of `n` frames.
    fn index(&self, t: Duration, n: u32) -> (u32, u32) {
        // (u32 maths: a u128 division from `as_millis` costs the wasm ~1 KB.)
        let ms = (t.as_secs() as u32).wrapping_mul(1000) + t.subsec_millis();
        let i = ms / self.ms() % n;
        (i, (i + n - 1) % n)
    }

    /// The palette entry for a texel value (plus the cycle's offset).
    #[inline]
    fn colour(&self, pal: &[Rgb; PAL_LEN], v: u8, cyc: u8) -> Rgb {
        pal[usize::from(v.wrapping_add(cyc) >> 3)]
    }
}

/// The phase of frame `i` of `n`: 0..65536 for a whole loop.
fn phase(i: u32, n: u32) -> u32 {
    (u64::from(i) * 65536 / u64::from(n)) as u32
}

/// `c` at `k` / 256.
#[inline]
pub(crate) fn scale(c: Rgb, k: u32) -> Rgb {
    Rgb::new(
        ((u32::from(c.r) * k) >> 8) as u8,
        ((u32::from(c.g) * k) >> 8) as u8,
        ((u32::from(c.b) * k) >> 8) as u8,
    )
}

/// The tunnel's light by depth (quarter texels / 4): full at the mouth
/// (r > 21), falling off as the square into the dark (0 from r = 8, where
/// the fog blacks the pixel out anyway).
static DEPTH_LIGHT: [u8; 256] = {
    let mut t = [0u8; 256];
    let mut i = 0;
    while i < 128 {
        let f = if i < 48 { 1.0 } else { (128 - i) as f64 / 80.0 };
        t[i] = round(255.0 * f * f) as u8;
        i += 1;
    }
    t
};

// --- the tunnel ------------------------------------------------------------

/// `@fx:tunnel[?speed&fps&palette&cycle&twist=0..3&texture=checker|rings|noise&drift=1&layers=1|2]`:
/// every pixel's angle and depth from the tunnel table (`TABLES`), the
/// texture at (angle + the turn, depth + the travel), and (`layers=2`, the
/// default) a second texture — noise, or rings under a noise tunnel — at
/// four times the angle and half the depth, turning the other way, a
/// quarter of the mix; lit by depth (the mouth bright, fading into the dark) and by
/// a light going round the other way. A loop (4.8 s at speed 2) is one
/// turn, one texture length of travel and, with `cycle=1` (the default),
/// one turn of the palette; `drift=1` moves the vanishing point on a figure
/// of eight (±16, ±8 px); `twist` spirals it. 100 fps by default.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tunnel {
    pub common: Common,
    pub twist: u8,
    pub texture: Texture,
    pub drift: bool,
    pub layers: u8,
}

impl Effect for Tunnel {
    fn name(&self) -> &'static str {
        "tunnel"
    }

    fn loop_frames(&self) -> u32 {
        self.common.frames(480)
    }

    fn frame(&mut self, t: Duration, canvas: &mut Canvas) -> Duration {
        let n = self.loop_frames();
        let p = phase(self.common.index(t, n).0, n);
        let cyc = if self.common.cycle { (p >> 8) as u8 } else { 0 };
        // A turn (1,024) and a texture's length (256 quarters) a loop.
        let (turn, travel) = ((p >> 6) as u16, (p >> 8) as u16);
        let (cx, cy) = if self.drift {
            (
                WIDTH as i32 / 2 + ((sin16(p) * DRIFT_X) >> 14),
                HEIGHT as i32 / 2 + ((sin16(p * 2) * DRIFT_Y) >> 14),
            )
        } else {
            (WIDTH as i32 / 2, HEIGHT as i32 / 2)
        };
        let tex = self.texture.table();
        let second = match self.texture {
            Texture::Noise => Texture::Rings,
            _ => Texture::Noise,
        }
        .table();
        let layered = self.layers > 1;
        // The palette, cycled, at eight light levels: a pixel's light is
        // an index, not three multiplies.
        let mut lit = [[0u8; 3]; 8 * PAL_LEN];
        for (i, c) in lit.iter_mut().enumerate() {
            let c0 = self
                .common
                .colour(self.common.palette.table(), (i % PAL_LEN * 8) as u8, cyc);
            let c1 = scale(c0, 32 * (i / PAL_LEN) as u32 + 32);
            *c = [c1.r, c1.g, c1.b];
        }
        let twist = u16::from(self.twist);
        // The light going round (the other way), by angle / 4: 0.6..1.
        let mut side = [0u8; 256];
        for (a, s) in side.iter_mut().enumerate() {
            let ph = ((a as u32) << 8).wrapping_add(p);
            *s = (204 + ((cos16(ph) * 51) >> 14)) as u8;
        }
        let (bytes, dirty) = canvas.bytes_and_dirty_mut();
        let texel = |q: u16, e: u16, mirror: u16, up: bool| -> [u8; 3] {
            if e >> 14 == 0 {
                return [0; 3];
            }
            let d = e & 1023;
            // The quadrant's angle mirrored into the pixel's quadrant
            // (`mirror` 0 right of the centre, 512 left; `up` above it).
            let q = if up { 1024 - q } else { q };
            let ang = if mirror == 0 {
                q & 1023
            } else {
                mirror.wrapping_sub(q) & 1023
            };
            // The texture goes round twice (64 texels, half a turn each)
            // and is a texel deep per 4 depth quarters.
            let u = ((ang + turn + ((d * twist) >> 2)) & 1023) >> 3 & 63;
            let v = (d + travel) >> 2 & (TEX_H as u16 - 1);
            let mut val = tex[usize::from(v) << 6 | usize::from(u)];
            if layered {
                // Four times round, half as deep, the other way.
                let u2 = ((4 * ang).wrapping_sub(turn) & 1023) >> 4;
                let v2 = ((d >> 1).wrapping_sub(travel)) >> 2 & (TEX_H as u16 - 1);
                // Three parts the first to one the second: the second is
                // texture in the first, not a blur of both.
                let b = second[usize::from(v2) << 6 | usize::from(u2)];
                val = ((u16::from(val) * 3 + u16::from(b)) >> 2) as u8;
            }
            let k = u32::from(DEPTH_LIGHT[usize::from(d >> 2)])
                * u32::from(side[usize::from(ang >> 2)]);
            lit[(k >> 13) as usize * PAL_LEN + usize::from(val >> 3)]
        };
        for y in 0..HEIGHT as i32 {
            let (ay, up) = if y >= cy {
                (y - cy, false)
            } else {
                (cy - 1 - y, true)
            };
            let row = (ay as usize).min(QH - 1) * QW;
            let quad = TABLES.tunnel.angle[row..row + QW]
                .iter()
                .zip(&TABLES.tunnel.depth[row..row + QW]);
            let out = bytes[y as usize * WIDTH * 3..(y as usize + 1) * WIDTH * 3]
                .as_chunks_mut::<3>()
                .0;
            // Right of the centre ax = x - cx, left ax = cx - 1 - x: the
            // quadrant's row forwards, then backwards.
            let (left, right) = out.split_at_mut(cx as usize);
            for (px, (&q, &e)) in right.iter_mut().zip(quad.clone()) {
                *px = texel(u16::from(q), e, 0, up);
            }
            for (px, (&q, &e)) in left.iter_mut().rev().zip(quad) {
                *px = texel(u16::from(q), e, 512, up);
            }
        }
        dirty.mark_all();
        self.common.delay()
    }
}

// --- rotozoom ----------------------------------------------------------------

/// `@fx:rotozoom[?speed&fps&palette&cycle&zoom=1..4&texture=..&floor=1&wobble=0..3&layers=1|2]`:
/// Mode 7. Flat: the texture turns once a loop (9.6 s at speed 2) about
/// the panel's centre, breathing between 0.5x and 1.5x of `zoom`
/// (magnification) twice a loop, while the centre pans two textures across
/// and one down; with `layers=2` (the default) a second texture turns the
/// other way twice as fast at another scale, the two averaged. `wobble`
/// (default 2) is the time warp's sine displacement: each row slides
/// sideways and each column up and down on waves that travel. `floor=1`:
/// the texture as a floor running to a horizon at the top, each row scaled
/// by its distance (the far rows fogged), turning and gliding. 100 fps by
/// default.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rotozoom {
    pub common: Common,
    pub zoom: u8,
    pub texture: Texture,
    pub floor: bool,
    pub wobble: u8,
    pub layers: u8,
}

impl Effect for Rotozoom {
    fn name(&self) -> &'static str {
        "rotozoom"
    }

    fn loop_frames(&self) -> u32 {
        self.common.frames(960)
    }

    fn frame(&mut self, t: Duration, canvas: &mut Canvas) -> Duration {
        let n = self.loop_frames();
        let p = phase(self.common.index(t, n).0, n);
        let cyc = if self.common.cycle { (p >> 8) as u8 } else { 0 };
        let (s, c) = (i64::from(sin16(p)), i64::from(cos16(p)));
        // The second layer turns the other way, twice a loop.
        let (s2, c2) = (-i64::from(sin16(p * 2)), i64::from(cos16(p * 2)));
        // The centre, in 16.16 texels: two textures across and one down a loop.
        let (cu, cv) = (i64::from(p) << 7, i64::from(p) << 6);
        let zoom = i64::from(self.zoom);
        let tex = self.texture.table();
        let second = match self.texture {
            Texture::Noise => Texture::Checker,
            _ => Texture::Noise,
        }
        .table();
        // (A floor is one plane: no second layer on it.)
        let layered = self.layers > 1 && !self.floor;
        let pal = self.common.palette.table();
        // The wobble: up to 4.5 texels (16.16), rows and columns on waves
        // that go round 3 and 2 times a loop.
        let amp = (i32::from(self.wobble) * 3) << 15;
        let mut col = [0i32; WIDTH];
        for (x, o) in col.iter_mut().enumerate() {
            *o = ((sin16((x as u32 * 1536).wrapping_sub(p * 2)) as i64 * i64::from(amp)) >> 14)
                as i32;
        }
        let (bytes, dirty) = canvas.bytes_and_dirty_mut();
        let half_w = WIDTH as i64 / 2;
        let half_h = HEIGHT as i64 / 2;
        for y in 0..HEIGHT {
            let wob =
                (i64::from(sin16((y as u32 * 2048).wrapping_add(p * 3))) * i64::from(amp)) >> 14;
            // This row's texels per pixel (16.16) and where its left end is.
            let (sc, u0, v0, fog) = if self.floor {
                // Row y is at distance ~ 1 / (y + 2): 2/zoom texels a pixel
                // at the top, ~0.12/zoom at the bottom.
                let sc = (8i64 << 16) / ((y as i64 + 2) * zoom);
                // Ahead (along (s, c)) by the row's distance, 64 px in.
                let dist = sc * 64;
                let fu = cu + ((s * dist) >> 14);
                let fv = cv - ((c * dist) >> 14);
                let fog = match y {
                    0..=2 => 0,
                    3..=5 => 96,
                    6..=9 => 160,
                    _ => 256,
                };
                (
                    sc,
                    fu - ((c * sc * half_w) >> 14),
                    fv - ((s * sc * half_w) >> 14),
                    fog,
                )
            } else {
                // 0.5x..1.5x of the zoom, twice a loop.
                let breathe = 256 + (i64::from(sin16(p * 2)) >> 7);
                let sc = (65536 * 256 / breathe) / zoom;
                let dy = y as i64 - half_h;
                let u0 = cu - ((c * sc * half_w) >> 14) - ((s * sc * dy) >> 14);
                let v0 = cv - ((s * sc * half_w) >> 14) + ((c * sc * dy) >> 14);
                (sc, u0, v0, 256)
            };
            let row = &mut bytes[y * WIDTH * 3..(y + 1) * WIDTH * 3];
            if fog == 0 {
                row.fill(0);
                continue;
            }
            let du = ((c * sc) >> 14) as i32;
            let dv = ((s * sc) >> 14) as i32;
            let (mut u, mut v) = ((u0 + wob) as i32, v0 as i32);
            // The second layer: 3/4 of the scale, about the same centre.
            let sc2 = sc * 3 / 4;
            let dy = y as i64 - half_h;
            let (du2, dv2) = (((c2 * sc2) >> 14) as i32, ((s2 * sc2) >> 14) as i32);
            let mut u2 = (cv - ((c2 * sc2 * half_w) >> 14) - ((s2 * sc2 * dy) >> 14) + wob) as i32;
            let mut v2 = (cu - ((s2 * sc2 * half_w) >> 14) + ((c2 * sc2 * dy) >> 14)) as i32;
            for (px, &o) in row.as_chunks_mut::<3>().0.iter_mut().zip(&col) {
                let at = |u: i32, v: i32| {
                    (((v >> 16) as usize & (TEX_H - 1)) << 6) | ((u >> 16) as usize & 63)
                };
                let mut val = tex[at(u, v + o)];
                if layered {
                    val = ((u16::from(val) + u16::from(second[at(u2, v2 + o)])) >> 1) as u8;
                    u2 = u2.wrapping_add(du2);
                    v2 = v2.wrapping_add(dv2);
                }
                let mut colour = self.common.colour(pal, val, cyc);
                if fog < 256 {
                    colour = scale(colour, fog);
                }
                *px = [colour.r, colour.g, colour.b];
                u = u.wrapping_add(du);
                v = v.wrapping_add(dv);
            }
        }
        dirty.mark_all();
        self.common.delay()
    }
}

// --- the starfield -----------------------------------------------------------

/// Depth units of the starfield's field; a loop is one pass through it.
const STAR_DEPTH: i32 = 4096;

/// `@fx:starfield[?speed&fps&palette&density=1..3&warp=0|1]`: 96, 192
/// (the default) or 288 stars (from a hash of their number: no table)
/// flying at the viewer, brighter as they near (the palette by depth, so
/// they fade in from black); with `warp=1` (the default) each is a streak
/// from where it was, fading along its length. A loop (2.4 s at speed 2)
/// is one pass through the field's depth. 100 fps by default.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Starfield {
    pub common: Common,
    pub density: u8,
    pub warp: bool,
    painted: bool,
}

impl Starfield {
    fn stars(&self) -> u32 {
        96 * u32::from(self.density)
    }

    /// Star `k` at phase `p`: its head and tail on screen and its colour;
    /// `None` when it is off the panel or still black.
    fn star(&self, k: u32, p: u32) -> Option<(End, End, Rgb)> {
        let h = hash(k.wrapping_mul(0x9e37_79b9) ^ 0x57a2);
        let sx = (h & 4095) as i32 - 2048;
        let sy = ((h >> 12) & 1023) as i32 - 512;
        let z0 = (hash(h) & 4095) as i32;
        let z = (z0 - (p >> 4) as i32).rem_euclid(STAR_DEPTH);
        let project = |z: i32| {
            let zz = (z >> 4) + 4;
            (
                WIDTH as i32 / 2 + sx * 2 / zz,
                HEIGHT as i32 / 2 + sy * 2 / zz,
            )
        };
        let head = project(z);
        if !(0..WIDTH as i32).contains(&head.0) || !(0..HEIGHT as i32).contains(&head.1) {
            return None;
        }
        let c = self.common.palette.table()[((255 - (z >> 4)) >> 3) as usize];
        if c == Rgb::BLACK {
            return None;
        }
        // The tail: where it was a few frames ago (longer the faster),
        // dimmer; no streak, the star alone.
        let tail = if self.warp {
            let (x, y) = project((z + 48 + 48 * i32::from(self.common.speed)).min(STAR_DEPTH - 1));
            (x, y, 48)
        } else {
            (head.0, head.1, 256)
        };
        Some(((head.0, head.1, 256), tail, c))
    }

    fn draw(&self, p: u32, canvas: &mut Canvas, erase: bool) {
        for k in 0..self.stars() {
            if let Some((head, tail, c)) = self.star(k, p) {
                let c = if erase { Rgb::BLACK } else { c };
                line(canvas, tail, head, c);
            }
        }
    }
}

impl Effect for Starfield {
    fn name(&self) -> &'static str {
        "starfield"
    }

    fn loop_frames(&self) -> u32 {
        self.common.frames(240)
    }

    fn frame(&mut self, t: Duration, canvas: &mut Canvas) -> Duration {
        let n = self.loop_frames();
        let (i, prev) = self.common.index(t, n);
        if std::mem::replace(&mut self.painted, true) {
            self.draw(phase(prev, n), canvas, true);
        } else {
            canvas.clear();
        }
        self.draw(phase(i, n), canvas, false);
        self.common.delay()
    }
}

/// A line's end: x, y and its brightness in 256ths.
pub(crate) type End = (i32, i32, i32);

/// A line from `a` to `b` (x, y, brightness in 256ths), `c` scaled by the
/// brightness interpolated along it in eight steps (strong steps: the
/// panel bands smooth ones anyway), clipped to the canvas, writing only
/// pixels that differ. (Bresenham; the brightness in 16.16.)
pub(crate) fn line(canvas: &mut Canvas, a: End, b: End, c: Rgb) {
    let (x1, y1) = (b.0, b.1);
    let (dx, dy) = ((x1 - a.0).abs(), -(y1 - a.1).abs());
    let (sx, sy) = (if a.0 < x1 { 1 } else { -1 }, if a.1 < y1 { 1 } else { -1 });
    let steps = dx.max(-dy).max(1);
    let (mut x, mut y, mut err) = (a.0, a.1, dx + dy);
    let mut shades = [Rgb::BLACK; 8];
    for (i, s) in shades.iter_mut().enumerate() {
        *s = scale(c, 32 * i as u32 + 32);
    }
    let mut k = a.2.clamp(0, 255) << 16;
    let dk = ((b.2.clamp(0, 255) << 16) - k) / steps;
    // Bounded: a projected line is at most a few hundred pixels.
    for _ in 0..1024 {
        if (0..WIDTH as i32).contains(&x) && (0..HEIGHT as i32).contains(&y) {
            let (ux, uy) = (x as usize, y as usize);
            let col = shades[(k >> 21) as usize & 7];
            if canvas.get(ux, uy) != col {
                canvas.set(ux, uy, col);
            }
        }
        if x == x1 && y == y1 {
            break;
        }
        k += dk;
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x += sx;
        }
        if e2 <= dx {
            err += dx;
            y += sy;
        }
    }
}

// --- wireframes -------------------------------------------------------------

/// The solids, by `shape=` name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Cube,
    Octahedron,
    Icosahedron,
    /// Twelve sections of six around the tube.
    Torus,
    /// The 4D hypercube, turning in four dimensions.
    Tesseract,
    /// A sphere opening into a torus, then a flower, and back.
    Morph,
}

impl Shape {
    fn from_name(v: &[u8]) -> Option<Shape> {
        Some(match v {
            b"cube" => Shape::Cube,
            b"octahedron" | b"octa" => Shape::Octahedron,
            b"icosahedron" | b"ico" => Shape::Icosahedron,
            b"torus" => Shape::Torus,
            b"tesseract" => Shape::Tesseract,
            b"morph" => Shape::Morph,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        [
            "cube",
            "octahedron",
            "icosahedron",
            "torus",
            "tesseract",
            "morph",
        ][self as usize]
    }
}

const C: i8 = 34;
static CUBE_V: [[i8; 3]; 8] = [
    [-C, -C, -C],
    [C, -C, -C],
    [C, C, -C],
    [-C, C, -C],
    [-C, -C, C],
    [C, -C, C],
    [C, C, C],
    [-C, C, C],
];
static CUBE_E: [[u8; 2]; 12] = [
    [0, 1],
    [1, 2],
    [2, 3],
    [3, 0],
    [4, 5],
    [5, 6],
    [6, 7],
    [7, 4],
    [0, 4],
    [1, 5],
    [2, 6],
    [3, 7],
];
const O: i8 = 56;
static OCTA_V: [[i8; 3]; 6] = [
    [O, 0, 0],
    [-O, 0, 0],
    [0, O, 0],
    [0, -O, 0],
    [0, 0, O],
    [0, 0, -O],
];
static OCTA_E: [[u8; 2]; 12] = [
    [0, 2],
    [0, 3],
    [0, 4],
    [0, 5],
    [1, 2],
    [1, 3],
    [1, 4],
    [1, 5],
    [2, 4],
    [4, 3],
    [3, 5],
    [5, 2],
];

/// The icosahedron: (0, ±1, ±φ) and its cyclic permutations, scaled to a
/// radius of ~50; edges between the vertices 2 apart (30 of them).
const fn icosahedron() -> ([[i8; 3]; 12], [[u8; 2]; 30]) {
    let phi = std::f64::consts::GOLDEN_RATIO;
    let s = 26.0;
    let mut v = [[0i8; 3]; 12];
    let mut raw = [[0.0f64; 3]; 12];
    let mut i = 0;
    while i < 12 {
        let (a, b) = (
            if i & 1 == 0 { 1.0 } else { -1.0 },
            if i & 2 == 0 { phi } else { -phi },
        );
        let p = match i / 4 {
            0 => [0.0, a, b],
            1 => [a, b, 0.0],
            _ => [b, 0.0, a],
        };
        raw[i] = p;
        v[i] = [
            round(p[0] * s) as i8,
            round(p[1] * s) as i8,
            round(p[2] * s) as i8,
        ];
        i += 1;
    }
    let mut e = [[0u8; 2]; 30];
    let (mut n, mut i) = (0, 0);
    while i < 12 {
        let mut j = i + 1;
        while j < 12 {
            let (a, b) = (raw[i], raw[j]);
            let d = (a[0] - b[0]) * (a[0] - b[0])
                + (a[1] - b[1]) * (a[1] - b[1])
                + (a[2] - b[2]) * (a[2] - b[2]);
            if d > 3.9 && d < 4.1 && n < 30 {
                e[n] = [i as u8, j as u8];
                n += 1;
            }
            j += 1;
        }
        i += 1;
    }
    (v, e)
}

static ICOSA: ([[i8; 3]; 12], [[u8; 2]; 30]) = icosahedron();

/// The torus and the morph: a grid of `SEGS` sections around the ring and
/// `RINGS` points around the tube, edges along and around (a torus's
/// topology: the sphere is the torus with no hole, its surface covered
/// twice). Vertex `s * RINGS + r`.
const RINGS: usize = 6;
const SEGS: usize = 12;
const GRID_V: usize = RINGS * SEGS;
/// The tesseract's 16 vertices are the bits of their index.
const MAX_V: usize = GRID_V;

const WIRE_BYTES: usize = std::mem::size_of::<[[i8; 3]; 8]>()
    + std::mem::size_of::<[[u8; 2]; 12]>() * 2
    + std::mem::size_of::<[[i8; 3]; 6]>()
    + std::mem::size_of::<([[i8; 3]; 12], [[u8; 2]; 30])>();

/// `@fx:wire[?speed&fps&shape=cube|octahedron|icosahedron|torus|tesseract|morph&color=RRGGBB&shade=0]`:
/// a wireframe turning once about the vertical axis and once about the
/// horizontal a loop (4.8 s at speed 2) while it sweeps across the panel
/// and back; the tesseract also turns once in 4D, and the morph (the
/// default) goes from a sphere to a torus to a flower and back. With
/// `shade` (the default) every pixel of a line is lit by its depth (eight
/// steps), the far edges drawn first. 100 fps by default.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Wire {
    pub common: Common,
    pub shape: Shape,
    pub colour: Rgb,
    pub shade: bool,
    painted: bool,
}

/// A vertex on screen: x, y, and depth (larger is further).
type Pt = (i16, i16, i16);

impl Wire {
    /// The model's vertices at phase `p` (model units, ~±60), into `out`;
    /// how many.
    fn model(&self, p: u32, out: &mut [(i32, i32, i32); MAX_V]) -> usize {
        let fixed = |v: &[[i8; 3]], out: &mut [(i32, i32, i32); MAX_V]| {
            for (o, v) in out.iter_mut().zip(v) {
                *o = (i32::from(v[0]), i32::from(v[1]), i32::from(v[2]));
            }
            v.len()
        };
        match self.shape {
            Shape::Cube => fixed(&CUBE_V, out),
            Shape::Octahedron => fixed(&OCTA_V, out),
            Shape::Icosahedron => fixed(&ICOSA.0, out),
            Shape::Tesseract => {
                // ±30 in four dimensions, turned in the x-w plane once a
                // loop, then seen from w = 90 (nearer w is bigger).
                let (s, c) = (sin16(p), cos16(p));
                for (i, o) in out.iter_mut().take(16).enumerate() {
                    let b = |k: usize| if i >> k & 1 == 1 { 30 } else { -30 };
                    let (x, y, z, w) = (b(0), b(1), b(2), b(3));
                    let (x, w) = ((x * c - w * s) >> 14, (x * s + w * c) >> 14);
                    let k = 90 * 256 / (90 - w / 2);
                    *o = ((x * k) >> 8, (y * k) >> 8, (z * k) >> 8);
                }
                16
            }
            Shape::Torus | Shape::Morph => {
                // (R, r): the ring's radius and the tube's, and how much
                // the tube flowers (16ths) with the section's angle.
                let (big, small, petal) = if self.shape == Shape::Torus {
                    (48, 18, 0)
                } else {
                    // Sphere (0, 56) -> torus (46, 18) -> flower -> sphere,
                    // eased: thirds of the loop.
                    let third = (p % 21_846) * 3;
                    let e = (16384 - cos16(third / 2)) >> 1; // 0..16384
                    let mix = |a: i32, b: i32| a + (((b - a) * e) >> 14);
                    match p / 21_846 {
                        0 => (mix(0, 46), mix(56, 18), 0),
                        1 => (46, 18, mix(0, 12)),
                        _ => (mix(46, 0), mix(18, 56), mix(12, 0)),
                    }
                };
                for (k, o) in out.iter_mut().enumerate() {
                    let (s, r) = ((k / RINGS) as u32, (k % RINGS) as u32);
                    let th = s * 65_536 / SEGS as u32;
                    let ph = r * 65_536 / RINGS as u32 + 5461;
                    let tube = small * (16 + ((sin16(th * 4) * petal) >> 14)) / 16;
                    let rad = big + ((tube * cos16(ph)) >> 14);
                    *o = (
                        (rad * cos16(th)) >> 14,
                        (tube * sin16(ph)) >> 14,
                        (rad * sin16(th)) >> 14,
                    );
                }
                GRID_V
            }
        }
    }

    /// How many edge slots [`edge`](Self::edge) takes.
    fn edge_slots(&self) -> usize {
        match self.shape {
            Shape::Cube | Shape::Octahedron => 12,
            Shape::Icosahedron => 30,
            Shape::Tesseract => 64,
            Shape::Torus | Shape::Morph => 2 * GRID_V,
        }
    }

    /// Edge slot `i`: an `(a, b)` vertex pair, from the tables or
    /// generated; `None` for a slot that is not an edge.
    fn edge(&self, i: usize) -> Option<(usize, usize)> {
        let table = |e: &[[u8; 2]]| e.get(i).map(|e| (usize::from(e[0]), usize::from(e[1])));
        match self.shape {
            Shape::Cube => table(&CUBE_E),
            Shape::Octahedron => table(&OCTA_E),
            Shape::Icosahedron => table(&ICOSA.1),
            // Each vertex to the one that differs in bit i % 4, once.
            Shape::Tesseract => {
                let (v, bit) = (i / 4, i % 4);
                (v >> bit & 1 == 0).then_some((v, v | 1 << bit))
            }
            // Around the tube, and along to the next section.
            Shape::Torus | Shape::Morph => {
                let k = i / 2;
                let (s, r) = (k / RINGS, k % RINGS);
                Some(if i.is_multiple_of(2) {
                    (k, s * RINGS + (r + 1) % RINGS)
                } else {
                    (k, (s + 1) % SEGS * RINGS + r)
                })
            }
        }
    }

    /// The vertices on screen at phase `p`.
    fn project(&self, p: u32, pts: &mut [Pt; MAX_V]) {
        let mut m = [(0, 0, 0); MAX_V];
        let n = self.model(p, &mut m);
        let (sa, ca) = (sin16(p), cos16(p));
        // The second axis at another rate would not be periodic; the same
        // rate from a sixth of a turn on keeps it lively.
        let q = p + 10_240;
        let (sb, cb) = (sin16(q), cos16(q));
        let cx = WIDTH as i32 / 2 + ((sa * 40) >> 14);
        let cy = HEIGHT as i32 / 2;
        for (o, &(x, y, z)) in pts.iter_mut().zip(&m[..n]) {
            let x1 = (x * ca + z * sa) >> 14;
            let z1 = (z * ca - x * sa) >> 14;
            let y2 = (y * cb - z1 * sb) >> 14;
            let z2 = (y * sb + z1 * cb) >> 14;
            let w = z2 + 128;
            *o = (
                (cx + x1 * 36 / w) as i16,
                (cy + y2 * 36 / w) as i16,
                z2 as i16,
            );
        }
    }

    fn draw(&self, p: u32, canvas: &mut Canvas, erase: bool) {
        let mut pts = [(0, 0, 0); MAX_V];
        self.project(p, &mut pts);
        let c = if erase { Rgb::BLACK } else { self.colour };
        // Depth -60..60 to brightness 256..48; flat: all 256.
        let lit = |z: i16| {
            if self.shade {
                (152 - i32::from(z) * 26 / 15).clamp(40, 256)
            } else {
                256
            }
        };
        // The far half of the edges first, then the near.
        for near in [false, true] {
            for (a, b) in (0..self.edge_slots()).filter_map(|i| self.edge(i)) {
                let (a, b) = (pts[a], pts[b]);
                if (i32::from(a.2) + i32::from(b.2) < 0) == near {
                    line(
                        canvas,
                        (a.0.into(), a.1.into(), lit(a.2)),
                        (b.0.into(), b.1.into(), lit(b.2)),
                        c,
                    );
                }
            }
        }
    }
}

impl Effect for Wire {
    fn name(&self) -> &'static str {
        "wire"
    }

    fn loop_frames(&self) -> u32 {
        self.common.frames(480)
    }

    fn frame(&mut self, t: Duration, canvas: &mut Canvas) -> Duration {
        let n = self.loop_frames();
        let (i, prev) = self.common.index(t, n);
        if std::mem::replace(&mut self.painted, true) {
            self.draw(phase(prev, n), canvas, true);
        } else {
            canvas.clear();
        }
        self.draw(phase(i, n), canvas, false);
        self.common.delay()
    }
}

// --- the effect a path names -------------------------------------------------

/// One of the effects, inline (no box: nothing on the heap) — but for a
/// Psi formula (the `psi` feature), whose program and working memory are
/// one allocation made when it is opened ([`crate::fx_psi`]).
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(not(feature = "psi"), derive(Copy))]
pub enum Fx {
    Tunnel(Tunnel),
    Rotozoom(Rotozoom),
    Starfield(Starfield),
    Wire(Wire),
    #[cfg(feature = "psi")]
    Psi(crate::fx_psi::PsiBox),
}

const _: () = assert!(std::mem::size_of::<Fx>() <= 512);

impl Fx {
    fn effect(&mut self) -> &mut dyn Effect {
        match self {
            Fx::Tunnel(e) => e,
            Fx::Rotozoom(e) => e,
            Fx::Starfield(e) => e,
            Fx::Wire(e) => e,
            #[cfg(feature = "psi")]
            Fx::Psi(e) => &mut **e,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Fx::Tunnel(e) => e.name(),
            Fx::Rotozoom(e) => e.name(),
            Fx::Starfield(e) => e.name(),
            Fx::Wire(e) => e.name(),
            #[cfg(feature = "psi")]
            Fx::Psi(e) => e.name(),
        }
    }

    /// What the Library and `now_playing` call it.
    pub fn title(&self) -> &'static str {
        match self {
            Fx::Tunnel(_) => "Tunnel",
            Fx::Rotozoom(_) => "Rotozoom",
            Fx::Starfield(_) => "Starfield",
            Fx::Wire(_) => "Wireframe",
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
        #[cfg(feature = "psi")]
        if let Fx::Psi(e) = self {
            return e.loop_time();
        }
        self.delay() * self.loop_frames()
    }

    /// The frame delay (20 ms, or 40 with `fps=25`).
    pub fn delay(&self) -> Duration {
        match self {
            Fx::Tunnel(e) => e.common.delay(),
            Fx::Rotozoom(e) => e.common.delay(),
            Fx::Starfield(e) => e.common.delay(),
            Fx::Wire(e) => e.common.delay(),
            #[cfg(feature = "psi")]
            Fx::Psi(e) => e.delay(),
        }
    }

    pub fn frame(&mut self, t: Duration, canvas: &mut Canvas) -> Duration {
        self.effect().frame(t, canvas)
    }

    /// The effect an `@fx:` path names, `None` if it names none. Unknown
    /// options are ignored and bad values keep the default, as `@clock`'s.
    pub fn parse(path: &str) -> Option<Fx> {
        let rest = path.strip_prefix(PREFIX)?;
        let (name, query) = rest.split_once('?').unwrap_or((rest, ""));
        #[cfg(feature = "psi")]
        if name == "psi" {
            return crate::fx_psi::PsiFx::from_query(query).ok().map(Fx::Psi);
        }
        let (palette, cycle) = match name {
            "starfield" => (Palette::Ice, false),
            "rotozoom" => (Palette::Fire, false),
            _ => (Palette::Chrono, true),
        };
        // Every default is 100 fps: each renders in ≤ 5 ms (CLAUDE.md,
        // "Live effects").
        let mut common = Common {
            speed: 2,
            delay_ms: 10,
            palette,
            cycle,
        };
        let mut fx = match name {
            "tunnel" => Fx::Tunnel(Tunnel {
                common,
                twist: 1,
                texture: Texture::Checker,
                drift: false,
                layers: 2,
            }),
            "rotozoom" => Fx::Rotozoom(Rotozoom {
                common,
                zoom: 2,
                texture: Texture::Checker,
                floor: false,
                wobble: 2,
                layers: 2,
            }),
            "starfield" => Fx::Starfield(Starfield {
                common,
                density: 2,
                warp: true,
                painted: false,
            }),
            "wire" => Fx::Wire(Wire {
                common,
                shape: Shape::Morph,
                colour: Rgb::new(0x40, 0xc0, 0xff),
                shade: true,
                painted: false,
            }),
            _ => return None,
        };
        let small = |v: &[u8], lo: u8, hi: u8| match v {
            [d @ b'0'..=b'9'] if (lo..=hi).contains(&(d - b'0')) => Some(d - b'0'),
            _ => None,
        };
        for kv in query.as_bytes().split(|&b| b == b'&') {
            let (k, v) = match kv.iter().position(|&b| b == b'=') {
                Some(i) => (&kv[..i], &kv[i + 1..]),
                None => (kv, &kv[kv.len()..]),
            };
            let on = v == b"1";
            match (k, &mut fx) {
                (b"speed", _) => common.speed = small(v, 1, 4).unwrap_or(common.speed),
                (b"fps", _) => {
                    common.delay_ms = match v {
                        b"100" => 10,
                        b"50" => 20,
                        b"25" => 40,
                        _ => common.delay_ms,
                    }
                }
                (b"palette", _) => common.palette = Palette::from_name(v).unwrap_or(common.palette),
                (b"cycle", _) => common.cycle = on,
                (b"twist", Fx::Tunnel(e)) => e.twist = small(v, 0, 3).unwrap_or(e.twist),
                (b"drift", Fx::Tunnel(e)) => e.drift = on,
                (b"texture", Fx::Tunnel(Tunnel { texture, .. }))
                | (b"texture", Fx::Rotozoom(Rotozoom { texture, .. })) => {
                    *texture = Texture::from_name(v).unwrap_or(*texture)
                }
                (b"zoom", Fx::Rotozoom(e)) => e.zoom = small(v, 1, 4).unwrap_or(e.zoom),
                (b"floor", Fx::Rotozoom(e)) => e.floor = on,
                (b"wobble", Fx::Rotozoom(e)) => e.wobble = small(v, 0, 3).unwrap_or(e.wobble),
                (b"layers", Fx::Tunnel(Tunnel { layers, .. }))
                | (b"layers", Fx::Rotozoom(Rotozoom { layers, .. })) => {
                    *layers = small(v, 1, 2).unwrap_or(*layers)
                }
                (b"density", Fx::Starfield(e)) => e.density = small(v, 1, 3).unwrap_or(e.density),
                (b"warp", Fx::Starfield(e)) => e.warp = v != b"0",
                (b"shape", Fx::Wire(e)) => e.shape = Shape::from_name(v).unwrap_or(e.shape),
                (b"color", Fx::Wire(e)) => {
                    e.colour = crate::synth::parse_colour(v).unwrap_or(e.colour)
                }
                (b"shade", Fx::Wire(e)) => e.shade = v != b"0",
                _ => {}
            }
        }
        match &mut fx {
            Fx::Tunnel(e) => e.common = common,
            Fx::Rotozoom(e) => e.common = common,
            Fx::Starfield(e) => e.common = common,
            Fx::Wire(e) => e.common = common,
            #[cfg(feature = "psi")]
            Fx::Psi(_) => {}
        }
        Some(fx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frames(path: &str, count: u32) -> Vec<Canvas> {
        let mut fx = Fx::parse(path).unwrap();
        let d = fx.delay();
        let mut c = Canvas::new();
        c.fill(Rgb::new(7, 7, 7));
        (0..count)
            .map(|i| {
                fx.frame(d * i, &mut c);
                c.clone()
            })
            .collect()
    }

    #[test]
    fn tables() {
        assert_eq!(SIN[0], 0);
        assert_eq!(SIN[64], 16384);
        assert_eq!(SIN[128], 0);
        assert_eq!(SIN[192], -16384);
        assert_eq!(SIN[32], 11585);
        // The tunnel's angles run 0..256 (of 1,024) across the quadrant,
        // the depth falls with the distance, the centre is dark.
        assert_eq!(TABLES.tunnel.depth[0], 1023);
        assert!(TABLES.tunnel.angle[QW - 1] <= 2 && TABLES.tunnel.angle[(QH - 1) * QW] > 250);
        assert!(TABLES.tunnel.depth[QW - 1] >> 14 == 3 && TABLES.tunnel.depth[QW - 1] & 1023 < 64);
        assert_eq!(TABLE_BYTES, 13_338);
        // The whole state of an effect, inline in the decoder.
        #[cfg(not(feature = "psi"))]
        assert_eq!(std::mem::size_of::<Fx>(), 10);
        // A formula's state is behind a pointer (`fx_psi`).
        #[cfg(feature = "psi")]
        assert!(std::mem::size_of::<Fx>() <= 16);
        for p in &PALETTES {
            assert_eq!(p[0], Rgb::BLACK);
        }
        let n = TABLES.textures[2];
        assert!(n.contains(&0) && n.contains(&248));
    }

    #[test]
    fn parse_options() {
        let Some(Fx::Tunnel(t)) = Fx::parse("@fx:tunnel") else {
            panic!()
        };
        assert_eq!(
            (t.common, t.twist, t.texture, t.drift, t.layers),
            (
                Common {
                    speed: 2,
                    delay_ms: 10,
                    palette: Palette::Chrono,
                    cycle: true
                },
                1,
                Texture::Checker,
                false,
                2
            )
        );
        let Some(Fx::Tunnel(t)) = Fx::parse(
            "@fx:tunnel?speed=4&fps=25&palette=fire&cycle=0&twist=0&texture=noise&drift=1&zoom=3&layers=1",
        ) else {
            panic!()
        };
        assert_eq!(
            (t.common, t.twist, t.texture, t.drift, t.layers),
            (
                Common {
                    speed: 4,
                    delay_ms: 40,
                    palette: Palette::Fire,
                    cycle: false
                },
                0,
                Texture::Noise,
                true,
                1
            )
        );
        let Some(Fx::Rotozoom(r)) =
            Fx::parse("@fx:rotozoom?zoom=9&speed=0&texture=rings&floor=1&fps=50&wobble=3")
        else {
            panic!()
        };
        assert_eq!(
            (
                r.zoom,
                r.common.speed,
                r.common.delay_ms,
                r.texture,
                r.floor,
                r.wobble,
                r.layers
            ),
            (2, 2, 20, Texture::Rings, true, 3, 2)
        );
        let Some(Fx::Wire(w)) = Fx::parse("@fx:wire?shape=torus&color=ff0000&shade=0") else {
            panic!()
        };
        assert_eq!(
            (w.shape, w.colour, w.shade),
            (Shape::Torus, Rgb::new(255, 0, 0), false)
        );
        let Some(Fx::Starfield(s)) = Fx::parse("@fx:starfield?density=3&warp=0") else {
            panic!()
        };
        assert_eq!(
            (s.stars(), s.warp, s.common.palette),
            (288, false, Palette::Ice)
        );
        let Some(Fx::Wire(w)) = Fx::parse("@fx:wire?fps=7") else {
            panic!()
        };
        assert_eq!((w.shape, w.common.delay_ms), (Shape::Morph, 10));
        assert_eq!(Fx::parse("@fx:nothing"), None);
        assert_eq!(Fx::parse("@clock"), None);
    }

    #[test]
    fn loops_are_whole_and_seamless() {
        for path in [
            "@fx:tunnel?drift=1&cycle=1&twist=3",
            "@fx:rotozoom?cycle=1",
            "@fx:rotozoom?floor=1",
            "@fx:starfield?warp=1",
            "@fx:wire?shape=torus",
            "@fx:wire?shape=tesseract",
            "@fx:wire?shade=1",
        ] {
            for q in ["", "&speed=1", "&speed=3&fps=25", "&speed=4&fps=50"] {
                let path = format!("{path}{q}");
                let mut fx = Fx::parse(&path).unwrap();
                let n = fx.loop_frames();
                let d = fx.delay();
                assert_eq!(u64::from(n) * d.as_millis() as u64 % 20, 0);
                // Frame n (one loop on) is frame 0, drawn over frame n - 1.
                let (mut a, mut b) = (Canvas::new(), Canvas::new());
                fx.frame(Duration::ZERO, &mut a);
                let mut fx2 = Fx::parse(&path).unwrap();
                fx2.frame(d * (n - 1), &mut b);
                b.dirty_mut().clear();
                fx2.frame(Duration::ZERO, &mut b);
                assert_eq!(a, b, "{path}");
                // And a phase that moved on changes the picture.
                fx2.frame(d * (n / 3), &mut b);
                assert_ne!(a, b, "{path}");
            }
        }
    }

    #[test]
    fn the_glow_floor_holds_effects_to_50_fps() {
        let mut fx = Fx::parse("@fx:tunnel").unwrap();
        assert_eq!(
            (fx.delay(), fx.loop_frames()),
            (Duration::from_millis(10), 480)
        );
        set_frame_floor(glow_floor(crate::look::GlowLevel::Long));
        let slow = (fx.delay(), fx.loop_frames());
        let mut fifty = Fx::parse("@fx:tunnel?fps=50").unwrap();
        let (mut a, mut b) = (Canvas::new(), Canvas::new());
        fx.frame(Duration::from_millis(1000), &mut a);
        fifty.frame(Duration::from_millis(1000), &mut b);
        set_frame_floor(glow_floor(crate::look::GlowLevel::Off));
        assert_eq!(slow, (Duration::from_millis(20), 240));
        // The same loop at half the frames: the same picture at the same time.
        assert_eq!(a, b);
        assert_eq!(fx.delay(), Duration::from_millis(10));
    }

    #[test]
    fn sparse_effects_mark_exactly_what_they_leave() {
        for path in [
            "@fx:starfield?warp=1&density=3",
            "@fx:starfield?warp=0",
            "@fx:wire?shape=torus",
            "@fx:wire?shape=icosahedron",
            "@fx:wire?shape=tesseract&shade=0",
            "@fx:wire",
        ] {
            let mut fx = Fx::parse(path).unwrap();
            let d = fx.delay();
            let mut c = Canvas::new();
            c.fill(Rgb::new(9, 9, 9));
            fx.frame(Duration::ZERO, &mut c);
            // The first frame starts from black.
            assert!(c.as_slice().iter().all(|p| *p != Rgb::new(9, 9, 9)));
            for i in 1..fx.loop_frames() + 5 {
                let before = c.clone();
                c.dirty_mut().clear();
                fx.frame(d * i, &mut c);
                // Same as drawing it on a fresh canvas: nothing left behind.
                let mut fresh = Fx::parse(path).unwrap();
                let mut want = Canvas::new();
                fresh.frame(d * i, &mut want);
                assert_eq!(c, want, "{path} frame {i}");
                for (k, (a, b)) in before.as_slice().iter().zip(c.as_slice()).enumerate() {
                    if a != b {
                        assert!(c.dirty().is_dirty(k % WIDTH, k / WIDTH), "{path} {i}");
                    }
                }
            }
        }
    }

    #[test]
    fn full_frame_effects_mark_everything_and_fill_the_panel() {
        for path in [
            "@fx:tunnel",
            "@fx:rotozoom",
            "@fx:rotozoom?floor=1&texture=noise",
        ] {
            let f = frames(path, 3);
            for c in &f {
                assert_eq!(c.dirty().count() as usize, WIDTH * HEIGHT);
                let lit = c.as_slice().iter().filter(|p| **p != Rgb::BLACK).count();
                assert!(lit > WIDTH * HEIGHT / 2, "{path}: {lit}");
            }
            assert_ne!(f[0], f[1]);
        }
        // The tunnel's centre is dark and the drift moves it.
        let f = frames("@fx:tunnel", 1);
        assert_eq!(f[0].get(64, 16), Rgb::BLACK);
    }

    #[test]
    fn lit_pixels_are_plausible() {
        let lit = |c: &Canvas| c.as_slice().iter().filter(|p| **p != Rgb::BLACK).count();
        let s = frames("@fx:starfield", 60);
        assert!(
            s.iter().all(|c| lit(c) > 40 && lit(c) < 1200),
            "{:?}",
            s.iter().map(lit).collect::<Vec<_>>()
        );
        let w = frames("@fx:wire", 60);
        assert!(
            w.iter().all(|c| lit(c) > 40 && lit(c) < 1500),
            "{:?}",
            w.iter().map(lit).collect::<Vec<_>>()
        );
        for shape in ["cube", "octahedron", "icosahedron", "torus", "tesseract"] {
            let w = frames(&format!("@fx:wire?shape={shape}"), 5);
            assert!(w.iter().all(|c| lit(c) > 40));
        }
    }

    /// Host time per frame (a rough guide; the rig's figures are in
    /// CLAUDE.md): `cargo test --release -- --ignored fx_cost --nocapture`.
    #[test]
    #[ignore]
    fn fx_cost() {
        for path in [
            "@fx:tunnel",
            "@fx:tunnel?layers=1",
            "@fx:rotozoom",
            "@fx:rotozoom?layers=1&wobble=0",
            "@fx:rotozoom?floor=1",
            "@fx:starfield",
            "@fx:starfield?density=3",
            "@fx:wire",
            "@fx:wire?shape=tesseract",
        ] {
            let mut fx = Fx::parse(path).unwrap();
            let d = fx.delay();
            let mut c = Canvas::new();
            let t0 = std::time::Instant::now();
            for i in 0..2000 {
                fx.frame(d * i, &mut c);
            }
            println!("{path}: {:?} a frame", t0.elapsed() / 2000);
        }
    }
}

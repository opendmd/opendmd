//! An LED-panel renderer for the simulator: draws what the real display looks
//! like rather than a grid of flat squares.
//!
//! # Where the numbers come from
//!
//! The panel is two chained P4 HUB75 modules (128x32, 4 mm pitch,
//! SMD2121-class RGB LEDs). The owner photographed it close up —
//! `hardware/IMG_0495.jpeg`, 3928x2799 — and measured, in units of the LED
//! pitch:
//!
//! - The lit dot's full-width-at-half-maximum is **0.40 of the pitch
//!   horizontally and 0.44 vertically**: a round-ish emitter, slightly taller
//!   than wide because of the package. That reads as a disc of diameter
//!   ~0.42 pitch, which is [`DOT_DIAMETER`], with [`DOT_ASPECT`] carrying the
//!   0.44/0.40 stretch. (The package itself is ~2.1 mm on a 4 mm pitch, 0.52,
//!   but the *lit core* is smaller than the package.)
//! - Around each lit dot is a soft **bloom** that has fallen to the mask level
//!   by roughly 0.7-0.8 pitch from centre — [`BLOOM_RADIUS`]. Bright
//!   neighbours' halos visibly overlap, so the kernel is two pitches across
//!   and is *accumulated*, not blitted.
//! - The **mask between LEDs is near black**: 10th-percentile luminance 5/255
//!   ([`MASK_LEVEL`]), median 15/255 across a mostly-lit frame — the median is
//!   overlapping bloom, not the mask. Unlit LEDs are barely visible as
//!   slightly-less-black squares, ~10/255 over about half a pitch
//!   ([`UNLIT_LEVEL`], [`PACKAGE_SIZE`]).
//! - The LEDs are driven **linearly** (BCM/PWM), so a mid code value looks
//!   brighter to the eye than the same sRGB value does on a monitor. So the
//!   renderer decodes canvas bytes to linear light with `--gamma`, accumulates
//!   in linear, and re-encodes at the end.
//!
//! # Shape of the work
//!
//! One kernel is precomputed per scale, `(2*scale)^2` taps, and every lit LED
//! adds `intensity * kernel` into a float RGB accumulator that is reused
//! across frames. At scale 10 that is 4096 LEDs x 400 taps — under two million
//! multiply-adds, and black LEDs (most of a DMD frame) are skipped outright.

use opendmd_core::canvas::{Canvas, HEIGHT, WIDTH};

/// Diameter of the lit emitter, in pitches. Mean of the measured 0.40 / 0.44
/// FWHM; see the module docs.
pub const DOT_DIAMETER: f32 = 0.42;
/// Vertical/horizontal aspect of the emitter: 0.44 / 0.40. Slightly tall,
/// which is what the package does in the photo.
pub const DOT_ASPECT: f32 = 1.10;
/// Radius, in pitches, at which the bloom has fallen to the mask level.
pub const BLOOM_RADIUS: f32 = 0.75;
/// Peak of the bloom term relative to the emitter core. Chosen so a lone lit
/// LED's halo is clearly visible but a field of lit LEDs does not wash out.
pub const BLOOM_STRENGTH: f32 = 0.22;
/// The black mask between LEDs, as a display (sRGB) level. 10th-percentile
/// luminance of the photo was 5/255.
pub const MASK_LEVEL: f32 = 5.0 / 255.0;
/// An unlit LED package, as a display (sRGB) level: slightly less black than
/// the mask around it.
pub const UNLIT_LEVEL: f32 = 10.0 / 255.0;
/// Side of the unlit package square, in pitches.
pub const PACKAGE_SIZE: f32 = 0.5;
/// Width of the anti-aliased edge of the emitter disc, in window pixels.
const EDGE_SOFTNESS: f32 = 1.0;
/// Below this scale there are too few window pixels per LED for any of the
/// above to survive; the flat renderer is used instead.
pub const MIN_LED_SCALE: u32 = 6;
/// Entries in the linear -> display encoding table.
const ENCODE_STEPS: usize = 4096;

/// Renders a [`Canvas`] as an LED panel into a `0x00RRGGBB` framebuffer.
///
/// Everything that depends only on the scale — the kernel, the mask-and-
/// package backdrop, the gamma tables — is built once in [`LedRenderer::new`].
/// [`LedRenderer::render`] allocates nothing.
pub struct LedRenderer {
    scale: usize,
    width: usize,
    height: usize,
    /// `kernel_size^2` taps, one LED's contribution centred in the middle.
    kernel: Vec<f32>,
    kernel_size: usize,
    /// Offset from an LED's cell origin to the kernel's top-left corner.
    kernel_origin: isize,
    /// Mask + unlit packages, in linear light, RGB interleaved. Copied over
    /// the accumulator at the start of every frame.
    backdrop: Vec<f32>,
    /// Reused across frames; never reallocated.
    accum: Vec<f32>,
    /// `(v/255)^gamma * gain` for each of the 256 canvas byte values.
    decode: [f32; 256],
    /// Linear `[0,1]` -> display byte.
    encode: Vec<u8>,
}

impl LedRenderer {
    /// `scale` is window pixels per LED; `gamma` decodes canvas bytes to
    /// linear light (2.2 is the usual sRGB-ish approximation) and `gain`
    /// scales the result so the owner can match the panel's real brightness.
    /// `bloom` is the halo strength (see [`BLOOM_STRENGTH`] for the default).
    pub fn new(scale: usize, gamma: f32, gain: f32, bloom_strength: f32) -> Self {
        assert!(scale > 0, "scale must be positive");
        let s = scale as f32;
        let kernel_size = 2 * scale;

        // The LED's centre sits at `scale/2` inside its cell; the kernel's
        // top-left goes a whole pitch left and up of that, so bloom can spill
        // into all eight neighbours.
        let kernel_origin = (scale / 2) as isize - scale as isize;
        let centre = s / 2.0 - kernel_origin as f32;

        // Emitter half-axes, and the bloom's reach.
        let rx = DOT_DIAMETER * s / 2.0;
        let ry = rx * DOT_ASPECT;
        let bloom = BLOOM_RADIUS * s;

        let mut kernel = vec![0.0f32; kernel_size * kernel_size];
        for j in 0..kernel_size {
            let dy = j as f32 + 0.5 - centre;
            for i in 0..kernel_size {
                let dx = i as f32 + 0.5 - centre;

                // Anti-aliased ellipse: measure the distance to the edge along
                // the ray through the sample, so EDGE_SOFTNESS stays in pixels.
                let norm = ((dx / rx).powi(2) + (dy / ry).powi(2)).sqrt();
                let core = if norm <= f32::EPSILON {
                    1.0
                } else {
                    let r = (dx * dx + dy * dy).sqrt();
                    let edge = r / norm; // radius of the ellipse in this direction
                    (0.5 + (edge - r) / EDGE_SOFTNESS).clamp(0.0, 1.0)
                };

                // Bloom falls smoothly to zero at BLOOM_RADIUS. It only fills
                // in what the core does not already cover, so a lit LED's peak
                // is exactly 1.0 and `gain` means what it says.
                let r = (dx * dx + dy * dy).sqrt();
                let halo = if r < bloom {
                    let t = r / bloom;
                    bloom_strength * (1.0 - t) * (1.0 - t)
                } else {
                    0.0
                };

                kernel[j * kernel_size + i] = core + halo * (1.0 - core);
            }
        }

        let width = WIDTH * scale;
        let height = HEIGHT * scale;

        // Backdrop: the near-black mask everywhere, plus a slightly brighter
        // square where each unlit package sits. Both are measured as display
        // levels, so they are decoded to linear with the same gamma.
        let mask_linear = MASK_LEVEL.powf(gamma);
        let package_extra = UNLIT_LEVEL.powf(gamma) - mask_linear;
        let mut backdrop = vec![mask_linear; width * height * 3];
        let pkg = (PACKAGE_SIZE * s).round().max(1.0) as usize;
        let pad = (scale - pkg.min(scale)) / 2;
        for y in 0..HEIGHT {
            for j in 0..pkg.min(scale) {
                let row = (y * scale + pad + j) * width;
                for x in 0..WIDTH {
                    for i in 0..pkg.min(scale) {
                        let idx = (row + x * scale + pad + i) * 3;
                        backdrop[idx] += package_extra;
                        backdrop[idx + 1] += package_extra;
                        backdrop[idx + 2] += package_extra;
                    }
                }
            }
        }

        let mut decode = [0.0f32; 256];
        for (v, out) in decode.iter_mut().enumerate() {
            *out = (v as f32 / 255.0).powf(gamma) * gain;
        }

        // Re-encoding with powf per subpixel would be ~1.2 M calls a frame at
        // scale 10; a table is exact enough for 8-bit output.
        let inv = 1.0 / gamma;
        let encode = (0..=ENCODE_STEPS)
            .map(|i| {
                let lin = i as f32 / ENCODE_STEPS as f32;
                (lin.powf(inv) * 255.0 + 0.5).clamp(0.0, 255.0) as u8
            })
            .collect();

        LedRenderer {
            scale,
            width,
            height,
            kernel,
            kernel_size,
            kernel_origin,
            backdrop,
            accum: vec![0.0; width * height * 3],
            decode,
            encode,
        }
    }

    /// Render `canvas` into `out`, which must be `width * height` long.
    pub fn render(&mut self, canvas: &Canvas, out: &mut [u32]) {
        assert_eq!(out.len(), self.width * self.height, "framebuffer size");

        self.accum.copy_from_slice(&self.backdrop);

        let k = self.kernel_size;
        for y in 0..HEIGHT {
            for x in 0..WIDTH {
                let px = canvas.get(x, y);
                let (r, g, b) = (
                    self.decode[px.r as usize],
                    self.decode[px.g as usize],
                    self.decode[px.b as usize],
                );
                // Most of a DMD frame is black; skipping it is most of the win.
                if r == 0.0 && g == 0.0 && b == 0.0 {
                    continue;
                }

                let ox = (x * self.scale) as isize + self.kernel_origin;
                let oy = (y * self.scale) as isize + self.kernel_origin;
                // Clip once per LED rather than testing every tap.
                let i0 = (-ox).max(0) as usize;
                let i1 = k.min((self.width as isize - ox).max(0) as usize);
                let j0 = (-oy).max(0) as usize;
                let j1 = k.min((self.height as isize - oy).max(0) as usize);

                for j in j0..j1 {
                    let krow = j * k;
                    let arow =
                        ((oy + j as isize) as usize * self.width + (ox + i0 as isize) as usize) * 3;
                    for i in i0..i1 {
                        let kv = self.kernel[krow + i];
                        if kv == 0.0 {
                            continue;
                        }
                        let idx = arow + (i - i0) * 3;
                        self.accum[idx] += r * kv;
                        self.accum[idx + 1] += g * kv;
                        self.accum[idx + 2] += b * kv;
                    }
                }
            }
        }

        let last = ENCODE_STEPS as f32;
        let (chunks, _) = self.accum.as_chunks::<3>();
        for (pixel, chunk) in out.iter_mut().zip(chunks) {
            let r = self.encode[(chunk[0].clamp(0.0, 1.0) * last) as usize] as u32;
            let g = self.encode[(chunk[1].clamp(0.0, 1.0) * last) as usize] as u32;
            let b = self.encode[(chunk[2].clamp(0.0, 1.0) * last) as usize] as u32;
            *pixel = (r << 16) | (g << 8) | b;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opendmd_core::canvas::Rgb;

    fn render(canvas: &Canvas, scale: usize) -> (Vec<u32>, usize) {
        let mut renderer = LedRenderer::new(scale, 2.2, 1.0, BLOOM_STRENGTH);
        let (w, h) = (WIDTH * scale, HEIGHT * scale);
        let mut buf = vec![0u32; w * h];
        renderer.render(canvas, &mut buf);
        (buf, w)
    }

    #[test]
    fn dark_canvas_sits_at_the_mask_and_package_levels() {
        let canvas = Canvas::new();
        let (buf, w) = render(&canvas, 10);
        // Cell corner: bare mask.
        let corner = buf[0] & 0xff;
        assert!(corner <= 6, "mask too bright: {corner}");
        // Cell centre: the unlit package.
        let centre = buf[5 * w + 5] & 0xff;
        assert!((8..=12).contains(&centre), "package level: {centre}");
    }

    #[test]
    fn a_lit_led_peaks_at_its_centre_and_blooms_into_the_gap() {
        let mut canvas = Canvas::new();
        canvas.set(4, 2, Rgb::new(255, 255, 255));
        let (buf, w) = render(&canvas, 10);
        let centre = buf[(2 * 10 + 5) * w + 4 * 10 + 5] & 0xff;
        assert_eq!(centre, 255);
        // Just outside the emitter, still inside the bloom, is lifted well
        // above the mask but nowhere near the core.
        let halo = buf[(2 * 10 + 5) * w + 4 * 10 + 11] & 0xff;
        assert!(halo > 20 && halo < 200, "halo level: {halo}");
        // A pitch and a half away the bloom is gone.
        let far = buf[(2 * 10 + 5) * w + 4 * 10 + 20] & 0xff;
        assert!(far < 20, "bloom reaches too far: {far}");
    }

    #[test]
    fn colour_is_preserved_and_gain_scales_it() {
        let mut canvas = Canvas::new();
        canvas.set(0, 0, Rgb::new(255, 0, 0));
        let (buf, w) = render(&canvas, 10);
        let p = buf[5 * w + 5];
        assert_eq!(p >> 16, 255);
        assert!((p >> 8) & 0xff <= 12);

        let mut dim = LedRenderer::new(10, 2.2, 0.25, BLOOM_STRENGTH);
        let (dw, dh) = (WIDTH * 10, HEIGHT * 10);
        let mut buf = vec![0u32; dw * dh];
        dim.render(&canvas, &mut buf);
        let r = (buf[5 * dw + 5] >> 16) & 0xff;
        // 0.25 of full linear intensity is ~0.5 after encoding at gamma 2.2.
        assert!((120..=145).contains(&r), "gain 0.25 gave {r}");
    }

    #[test]
    fn renders_at_every_supported_scale() {
        let mut canvas = Canvas::new();
        canvas.set(WIDTH - 1, HEIGHT - 1, Rgb::new(255, 255, 255));
        canvas.set(0, 0, Rgb::new(0, 255, 0));
        for scale in [MIN_LED_SCALE as usize, 7, 8, 11, 16, 32] {
            let (buf, w) = render(&canvas, scale);
            assert_eq!(buf.len(), w * HEIGHT * scale);
        }
    }
}

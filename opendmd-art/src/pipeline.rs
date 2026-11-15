//! Picture in, 128x32 frames out: the Studio's pixel work, with no browser
//! types in it (tested natively and under node).
//!
//! The stages, in the order the Studio runs them (`docs/art-pipeline.md` §3
//! has the reasoning and the pictures):
//!
//! 1. **Stage** ([`stage`]): the picture the 128x32 window moves over.
//!    - *Pixels* ([`Fit::Pixels`]): undo a nearest-neighbour 2x/3x/...
//!      upscale ([`native_scale`], [`native`]) and use the pixels 1:1.
//!    - *Pixels ½* ([`Fit::Half`]): the same, then [`down_mode`] 2:1 (the
//!      commonest colour of each 2x2 block: no invented colours, 1 px
//!      outlines survive).
//!    - *Fit* ([`Fit::Smooth`]): logos and photos — the transparent border
//!      trimmed, area-resampled ([`resize_area`]) to 30 px high (or to the
//!      width when that is tighter), or to 32 px high and panned when it is
//!      a long wordmark.
//!    - Optionally, first, [`lift_dark`]: black strokes vanish on a black
//!      panel, so dark greys are flipped light (unless they are most of the
//!      picture, a backing plate).
//!    - Then flattened onto black and padded (centred) to at least 128x32.
//! 2. **Window** ([`best_window`]): where the interest is — luma gradient
//!    plus "not the background colour", over an integral image, with a mild
//!    pull to the centre.
//! 3. **Motion** ([`windows`], [`delays`]): still (5 s), pan left to right,
//!    or scroll top to bottom, 1 px a frame, with holds on the end frames.
//! 4. **Look** ([`render`]): background to black ([`key_background`], the
//!    commonest colour of what is shown), brightness, and one palette for
//!    the whole animation ([`spread_pick`]) — exact when it fits, else picked
//!    for spread so a 1-pixel accent survives.
//!
//! Coordinates are `usize`, pixels `[r, g, b, a]`.

use std::collections::HashMap;

/// The panel.
pub const W: usize = 128;
pub const H: usize = 32;

/// A still, in milliseconds (the engine's dwell decides how long it stays).
pub const STILL_MS: u32 = 5_000;

pub type Px = [u8; 4];

/// An RGBA picture.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    pub w: usize,
    pub h: usize,
    pub px: Vec<Px>,
}

impl Image {
    /// Opaque black.
    pub fn new(w: usize, h: usize) -> Image {
        Image {
            w,
            h,
            px: vec![[0, 0, 0, 255]; w * h],
        }
    }

    /// From `ImageData`-style bytes (RGBA, row-major). `None` if the length
    /// does not match.
    pub fn from_rgba(bytes: &[u8], w: usize, h: usize) -> Option<Image> {
        (w > 0 && h > 0 && bytes.len() == w * h * 4).then(|| Image {
            w,
            h,
            px: bytes.as_chunks::<4>().0.to_vec(),
        })
    }

    pub fn to_rgba(&self) -> Vec<u8> {
        self.px.iter().flatten().copied().collect()
    }

    pub fn at(&self, x: usize, y: usize) -> Px {
        self.px[y * self.w + x]
    }

    /// The `w` x `h` rectangle at (`x`, `y`), which must lie inside.
    pub fn crop(&self, x: usize, y: usize, w: usize, h: usize) -> Image {
        let mut px = Vec::with_capacity(w * h);
        for row in y..y + h {
            px.extend_from_slice(&self.px[row * self.w + x..row * self.w + x + w]);
        }
        Image { w, h, px }
    }
}

pub fn rgb_key(p: Px) -> u32 {
    (p[0] as u32) << 16 | (p[1] as u32) << 8 | p[2] as u32
}

fn luma(p: Px) -> f32 {
    0.299 * p[0] as f32 + 0.587 * p[1] as f32 + 0.114 * p[2] as f32
}

fn l1(a: Px, b: Px) -> u32 {
    a[..3]
        .iter()
        .zip(&b[..3])
        .map(|(x, y)| x.abs_diff(*y) as u32)
        .sum()
}

// --- Native pixels --------------------------------------------------------

/// A detected nearest-neighbour upscale: every `k` x `k` block starting at
/// (`x0` + i·k, `y0` + j·k) is one source pixel. `k` = 1: none found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scale {
    pub k: usize,
    pub x0: usize,
    pub y0: usize,
}

const KMAX: usize = 8;

/// Strong-edge counts per boundary along one axis: `e[i]` counts the
/// neighbour pairs (i-1, i) whose colours differ by more than the threshold
/// (the 75th percentile of the non-zero differences, at least 60, so JPEG
/// ringing and bilinear ramps — weak edges everywhere, off the grid — do not
/// count).
fn edges(img: &Image, horizontal: bool) -> Vec<u32> {
    let (n, m) = if horizontal {
        (img.w, img.h)
    } else {
        (img.h, img.w)
    };
    let pair = |i: usize, j: usize| {
        let (a, b) = if horizontal {
            (img.at(i, j), img.at(i - 1, j))
        } else {
            (img.at(j, i), img.at(j, i - 1))
        };
        l1(a, b)
    };
    let mut hist = [0u32; 766];
    for i in 1..n {
        for j in 0..m {
            hist[pair(i, j) as usize] += 1;
        }
    }
    let nonzero: u32 = hist[1..].iter().sum();
    let mut thr = 60;
    if nonzero > 0 {
        // Nearest rank.
        let rank = (nonzero as f64 * 0.75).ceil().max(1.0) as u32;
        let mut seen = 0;
        for (v, c) in hist.iter().enumerate().skip(1) {
            seen += c;
            if seen >= rank {
                thr = thr.max(v as u32);
                break;
            }
        }
    }
    let mut e = vec![0u32; n];
    for (i, slot) in e.iter_mut().enumerate().skip(1) {
        *slot = (0..m).filter(|&j| pair(i, j) > thr).count() as u32;
    }
    e
}

/// The largest k (8..2) with more than 80% of the edge weight on one phase.
fn axis_scale(e: &[u32]) -> (usize, usize) {
    if e.iter().filter(|&&c| c > 0).count() < 8 {
        return (1, 0);
    }
    let total: u64 = e.iter().map(|&c| c as u64).sum();
    for k in (2..=KMAX).rev() {
        let mut hist = [0u64; KMAX];
        for (i, &c) in e.iter().enumerate() {
            hist[i % k] += c as u64;
        }
        let (ph, best) =
            hist[..k]
                .iter()
                .enumerate()
                .fold((0, 0), |acc, (i, &v)| if v > acc.1 { (i, v) } else { acc });
        if best as f64 > 0.8 * total as f64 {
            return (k, ph);
        }
    }
    (1, 0)
}

/// Whether the picture really is flat within `k` x `k` blocks at that
/// phase: at least 90% of the whole blocks within a JPEG-sized tolerance of
/// their centre. Guards against a grid of large tiles passing for a big
/// upscale (edges only every 16 px in a 2x copy of 8 px tiles).
fn blocks_flat(img: &Image, k: usize, x0: usize, y0: usize) -> bool {
    let (bw, bh) = ((img.w - x0) / k, (img.h - y0) / k);
    if bw == 0 || bh == 0 {
        return false;
    }
    // Every block in a small picture; a sample of rows and columns in a big one.
    let step = |n: usize| (n / 160).max(1);
    let (sx, sy) = (step(bw), step(bh));
    let (mut flat, mut all) = (0u32, 0u32);
    for by in (0..bh).step_by(sy) {
        for bx in (0..bw).step_by(sx) {
            let (ox, oy) = (x0 + bx * k, y0 + by * k);
            let c = img.at(ox + k / 2, oy + k / 2);
            let ok = (0..k).all(|j| {
                (0..k).all(|i| {
                    let p = img.at(ox + i, oy + j);
                    l1(p, c) <= 48 && p[3].abs_diff(c[3]) <= 48
                })
            });
            flat += ok as u32;
            all += 1;
        }
    }
    flat as f64 >= 0.9 * all as f64
}

/// Detect a nearest-neighbour integer upscale (2x and 3x screenshots are
/// everywhere on the web): where the strong colour edges fall, modulo k,
/// on both axes; square pixels only; then checked block by block (falling
/// back to a divisor of k). A 2.5x bilinear copy is refused (k = 1).
pub fn native_scale(img: &Image) -> Scale {
    let none = Scale { k: 1, x0: 0, y0: 0 };
    if img.w < 16 || img.h < 16 {
        return none;
    }
    let (kx, px) = axis_scale(&edges(img, true));
    let (ky, py) = axis_scale(&edges(img, false));
    if kx < 2 || ky < 2 {
        return none;
    }
    let k = kx.min(ky);
    (2..=k)
        .rev()
        .filter(|d| k % d == 0)
        .map(|d| Scale {
            k: d,
            x0: px % d,
            y0: py % d,
        })
        .find(|s| blocks_flat(img, s.k, s.x0, s.y0))
        .unwrap_or(none)
}

/// One pixel per block (its centre).
pub fn native(img: &Image, s: Scale) -> Image {
    if s.k <= 1 {
        return img.clone();
    }
    let (w, h) = ((img.w - s.x0) / s.k, (img.h - s.y0) / s.k);
    let mut out = Image::new(w, h);
    for y in 0..h {
        for x in 0..w {
            out.px[y * w + x] = img.at(s.x0 + x * s.k + s.k / 2, s.y0 + y * s.k + s.k / 2);
        }
    }
    out
}

// --- Where the interest is ----------------------------------------------

/// Per-pixel interest: the luma gradient (capped) plus a bonus for not
/// being the dominant colour (at 4 bits a channel). Transparent: nothing.
pub fn detail(img: &Image) -> Vec<f32> {
    let key =
        |p: Px| ((p[0] >> 4) as usize) << 8 | ((p[1] >> 4) as usize) << 4 | (p[2] >> 4) as usize;
    let mut counts = vec![0u32; 4096];
    for &p in &img.px {
        counts[key(p)] += 1;
    }
    let bg = counts
        .iter()
        .enumerate()
        .fold((0, 0), |acc, (i, &c)| if c > acc.1 { (i, c) } else { acc })
        .0;
    let mut out = vec![0f32; img.w * img.h];
    for y in 0..img.h {
        for x in 0..img.w {
            let p = img.at(x, y);
            if p[3] == 0 {
                continue;
            }
            let l = luma(p);
            let gx = if x + 1 < img.w {
                (luma(img.at(x + 1, y)) - l).abs()
            } else {
                0.0
            };
            let gy = if y + 1 < img.h {
                (luma(img.at(x, y + 1)) - l).abs()
            } else {
                0.0
            };
            out[y * img.w + x] = (gx + gy).min(96.0) + if key(p) != bg { 24.0 } else { 0.0 };
        }
    }
    out
}

/// Top-left of the `w` x `h` window with the most [`detail`], mildly
/// biased towards the centre (`bias` 0.15 in the Studio). A picture smaller
/// than the window gives (0, 0).
pub fn best_window(img: &Image, w: usize, h: usize, bias: f64) -> (usize, usize) {
    let (w, h) = (w.min(img.w), h.min(img.h));
    let d = detail(img);
    let iw = img.w + 1;
    let mut s = vec![0f64; iw * (img.h + 1)];
    for y in 0..img.h {
        let mut row = 0f64;
        for x in 0..img.w {
            row += d[y * img.w + x] as f64;
            s[(y + 1) * iw + x + 1] = s[y * iw + x + 1] + row;
        }
    }
    let (cx, cy) = ((img.w - w) as f64 / 2.0, (img.h - h) as f64 / 2.0);
    // Ties (a flat picture scores 0 everywhere) go to the most central.
    let (mut best, mut best_dist, mut at) = (-1.0, f64::MAX, (0, 0));
    for y in 0..=img.h - h {
        for x in 0..=img.w - w {
            let t =
                s[(y + h) * iw + x + w] - s[y * iw + x + w] - s[(y + h) * iw + x] + s[y * iw + x];
            let dist = ((x as f64 - cx) / img.w as f64).hypot((y as f64 - cy) / img.h as f64);
            let score = t * (1.0 - bias * dist);
            if score > best || (score == best && dist < best_dist) {
                (best, best_dist, at) = (score, dist, (x, y));
            }
        }
    }
    at
}

// --- Resampling -----------------------------------------------------------

/// Integer downscale for pixel art: each output pixel is the commonest
/// colour of its `f` x `f` block, so no colour is invented. A tie goes to
/// the colour furthest from the block's mean, which keeps 1 px outlines
/// (an outline pixel against a fill is always a tie in a 2x2 block).
pub fn down_mode(img: &Image, f: usize) -> Image {
    let (w, h) = (img.w / f, img.h / f);
    if w == 0 || h == 0 {
        return img.clone();
    }
    let mut out = Image::new(w, h);
    let mut block: Vec<(u32, Px)> = Vec::with_capacity(f * f);
    for y in 0..h {
        for x in 0..w {
            block.clear();
            let mut mean = [0f32; 3];
            for j in 0..f {
                for i in 0..f {
                    let p = img.at(x * f + i, y * f + j);
                    for c in 0..3 {
                        mean[c] += p[c] as f32;
                    }
                    block.push((u32::from_be_bytes(p), p));
                }
            }
            for m in &mut mean {
                *m /= (f * f) as f32;
            }
            block.sort_unstable_by_key(|b| b.0);
            // Runs of equal colours, in ascending colour order.
            let mut best: Option<(usize, f32, Px)> = None;
            let mut i = 0;
            while i < block.len() {
                let run = block[i..].iter().take_while(|b| b.0 == block[i].0).count();
                let p = block[i].1;
                let dist: f32 = (0..3).map(|c| (p[c] as f32 - mean[c]).abs()).sum();
                let better = match best {
                    None => true,
                    Some((n, d, _)) => run > n || (run == n && dist > d),
                };
                if better {
                    best = Some((run, dist, p));
                }
                i += run;
            }
            out.px[y * w + x] = best.map_or([0, 0, 0, 255], |b| b.2);
        }
    }
    out
}

/// Source spans for an area resample of `n` samples to `m`: per output
/// sample, `(source index, weight)` with the weights summing to 1.
fn spans(n: usize, m: usize) -> Vec<Vec<(usize, f32)>> {
    let scale = n as f64 / m as f64;
    (0..m)
        .map(|o| {
            let (a, b) = (o as f64 * scale, (o + 1) as f64 * scale);
            let mut v = Vec::new();
            let mut i = a.floor() as usize;
            while (i as f64) < b && i < n {
                let cover = (b.min(i as f64 + 1.0) - a.max(i as f64)) / scale;
                if cover > 1e-9 {
                    v.push((i, cover as f32));
                }
                i += 1;
            }
            v
        })
        .collect()
}

/// Area (box) resample on premultiplied RGBA: right for smooth sources —
/// logos, photos — where averaging is what the eye does anyway. Never used
/// on pixel art (see [`down_mode`]).
pub fn resize_area(img: &Image, w: usize, h: usize) -> Image {
    let (w, h) = (w.max(1), h.max(1));
    let pre = |p: Px| {
        let a = p[3] as f32 / 255.0;
        [
            p[0] as f32 * a,
            p[1] as f32 * a,
            p[2] as f32 * a,
            p[3] as f32,
        ]
    };
    let sx = spans(img.w, w);
    let sy = spans(img.h, h);
    let mut tmp = vec![[0f32; 4]; w * img.h];
    for y in 0..img.h {
        for (x, span) in sx.iter().enumerate() {
            let mut acc = [0f32; 4];
            for &(i, wt) in span {
                let p = pre(img.at(i, y));
                for c in 0..4 {
                    acc[c] += p[c] * wt;
                }
            }
            tmp[y * w + x] = acc;
        }
    }
    let mut out = Image::new(w, h);
    for (y, span) in sy.iter().enumerate() {
        for x in 0..w {
            let mut acc = [0f32; 4];
            for &(j, wt) in span {
                let p = tmp[j * w + x];
                for c in 0..4 {
                    acc[c] += p[c] * wt;
                }
            }
            let a = acc[3];
            let px = if a < 0.5 {
                [0, 0, 0, 0]
            } else {
                let k = 255.0 / a;
                let ch = |v: f32| (v * k).round().clamp(0.0, 255.0) as u8;
                [
                    ch(acc[0]),
                    ch(acc[1]),
                    ch(acc[2]),
                    a.round().min(255.0) as u8,
                ]
            };
            out.px[y * w + x] = px;
        }
    }
    out
}

/// The bounding box of the pixels that are not fully transparent (the
/// whole picture if none are, or all are).
pub fn opaque_box(img: &Image) -> (usize, usize, usize, usize) {
    let (mut x0, mut y0, mut x1, mut y1) = (img.w, img.h, 0, 0);
    for y in 0..img.h {
        for x in 0..img.w {
            if img.at(x, y)[3] > 0 {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x + 1);
                y1 = y1.max(y + 1);
            }
        }
    }
    if x1 == 0 {
        (0, 0, img.w, img.h)
    } else {
        (x0, y0, x1 - x0, y1 - y0)
    }
}

/// The picture with its transparent border cut off.
pub fn trim(img: &Image) -> Image {
    let (x, y, w, h) = opaque_box(img);
    img.crop(x, y, w, h)
}

// --- Logos ------------------------------------------------------------------

/// Dark, unsaturated and opaque: a stroke that disappears on the panel.
fn is_dark(p: Px) -> bool {
    let (mx, mn) = (p[0].max(p[1]).max(p[2]), p[0].min(p[1]).min(p[2]));
    p[3] > 127 && luma(p) < 70.0 && mx - mn < 60
}

/// Black or grey lettering vanishes on a black panel (the Saturn and 3DS
/// wordmarks). Flip the lightness of dark, unsaturated, opaque pixels —
/// unless they cover more than 60% of the opaque box, which makes them a
/// backing plate (the Mega Drive logo's black square stays black, and the
/// red "M" and green "D" stand on it). Run at full resolution, before any
/// resample.
pub fn lift_dark(img: &Image) -> Image {
    if should_lift(img) {
        flip_dark(img)
    } else {
        img.clone()
    }
}

/// [`lift_dark`]'s decision on its own: there are dark strokes, and they
/// are not a backing plate. An animation decides once, from its first
/// frame, so the strokes do not flicker between frames.
pub fn should_lift(img: &Image) -> bool {
    let (_, _, w, h) = opaque_box(img);
    let dark = img.px.iter().filter(|&&p| is_dark(p)).count();
    dark > 0 && dark as f64 <= 0.6 * (w * h) as f64
}

/// Flip the lightness of every dark stroke pixel, unconditionally.
pub fn flip_dark(img: &Image) -> Image {
    let mut out = img.clone();
    for p in &mut out.px {
        if is_dark(*p) {
            *p = [255 - p[0], 255 - p[1], 255 - p[2], p[3]];
        }
    }
    out
}

// --- The stage --------------------------------------------------------------

/// How the picture meets the panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fit {
    /// Native pixels 1:1.
    Pixels,
    /// Native pixels, 2:1 by [`down_mode`].
    Half,
    /// Smooth: trimmed and area-resampled to the panel's height.
    Smooth,
}

impl Fit {
    pub fn parse(s: &str) -> Fit {
        match s {
            "half" => Fit::Half,
            "fit" => Fit::Smooth,
            _ => Fit::Pixels,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Fit::Pixels => "pixels",
            Fit::Half => "half",
            Fit::Smooth => "fit",
        }
    }
}

/// A logo's size on the panel: 30 px high (a 1 px margin) or 126 wide if
/// that is tighter — or, for a long wordmark (wider than 192 at 32 high),
/// the full 32 px high, to pan across.
pub fn smooth_size(w: usize, h: usize) -> (usize, usize) {
    let (w, h) = (w as f64, h as f64);
    if w * 32.0 / h > 192.0 {
        return (((w * 32.0 / h).round() as usize).max(1), H);
    }
    let s = (30.0 / h).min(126.0 / w);
    (
        ((w * s).round() as usize).max(1),
        ((h * s).round() as usize).max(1),
    )
}

/// Composite onto black (the panel has no "behind") and centre on a canvas
/// at least the panel's size.
pub fn flatten_pad(img: &Image) -> Image {
    let (w, h) = (img.w.max(W), img.h.max(H));
    let (ox, oy) = ((w - img.w) / 2, (h - img.h) / 2);
    let mut out = Image::new(w, h);
    for y in 0..img.h {
        for x in 0..img.w {
            let p = img.at(x, y);
            let a = p[3] as u32;
            let c = |v: u8| ((v as u32 * a + 127) / 255) as u8;
            out.px[(y + oy) * w + x + ox] = [c(p[0]), c(p[1]), c(p[2]), 255];
        }
    }
    out
}

/// The picture the window moves over: see the module docs.
pub fn stage(raw: &Image, scale: Scale, fit: Fit, lift: bool) -> Image {
    let lifted;
    let base = if lift {
        lifted = lift_dark(raw);
        &lifted
    } else {
        raw
    };
    let img = match fit {
        Fit::Pixels => native(base, scale),
        Fit::Half => down_mode(&native(base, scale), 2),
        Fit::Smooth => {
            let t = trim(base);
            let (w, h) = smooth_size(t.w, t.h);
            resize_area(&t, w, h)
        }
    };
    flatten_pad(&img)
}

/// One frame of an animation as a stage: [`stage`] for a picture already
/// at its native pixels (an animation's frames are reduced on import), the
/// dark strokes flipped when the animation decided so ([`should_lift`] on
/// its first frame). Frames are opaque (a GIF's screen has no alpha), so
/// *Fit* never trims and every frame gets the same size.
pub fn stage_frame(img: &Image, fit: Fit, flip: bool) -> Image {
    let flipped;
    let base = if flip {
        flipped = flip_dark(img);
        &flipped
    } else {
        img
    };
    let out = match fit {
        Fit::Pixels => return flatten_pad(base),
        Fit::Half => down_mode(base, 2),
        Fit::Smooth => {
            let (w, h) = smooth_size(base.w, base.h);
            resize_area(base, w, h)
        }
    };
    flatten_pad(&out)
}

/// The fit the Studio starts with. A transparent picture with no pixel grid
/// is a logo, and a big one (over 640 px) a photo or a rescaled screenshot:
/// both Fit. Otherwise native pixels — at 2:1 when the native picture is
/// at least twice the panel and the ½ window holds more distinct colours
/// than the 1:1 one (a title screen's busiest 1:1 spot is often its menu
/// text; at ½ the wordmark wins).
pub fn auto_fit(raw: &Image, scale: Scale) -> Fit {
    let clear = raw.px.iter().filter(|p| p[3] < 128).count();
    if scale.k == 1 && (clear * 20 > raw.px.len() || raw.w.max(raw.h) > 640) {
        return Fit::Smooth;
    }
    let n = native(raw, scale);
    if n.w < 2 * W || n.h < 2 * H {
        return Fit::Pixels;
    }
    let colours_in = |img: &Image| {
        let (x, y) = best_window(img, W, H, 0.15);
        let win = img.crop(x, y, W.min(img.w), H.min(img.h));
        let mut seen: Vec<u32> = win.px.iter().map(|&p| rgb_key(p)).collect();
        seen.sort_unstable();
        seen.dedup();
        seen.len()
    };
    let one = colours_in(&flatten_pad(&n));
    let half = colours_in(&flatten_pad(&down_mode(&n, 2)));
    if half > one {
        Fit::Half
    } else {
        Fit::Pixels
    }
}

// --- Motion -----------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Motion {
    Still,
    /// Left to right.
    Pan,
    /// Top to bottom.
    Scroll,
}

impl Motion {
    pub fn parse(s: &str) -> Motion {
        match s {
            "pan" => Motion::Pan,
            "scroll" => Motion::Scroll,
            _ => Motion::Still,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Motion::Still => "still",
            Motion::Pan => "pan",
            Motion::Scroll => "scroll",
        }
    }
}

/// Motion from the stage's shape: wide and short pans, tall and narrow
/// scrolls, anything else is a still.
pub fn auto_motion(w: usize, h: usize) -> Motion {
    if w * 2 > W * 3 && h <= H + 8 {
        Motion::Pan
    } else if h > H * 3 && w <= W + 8 {
        Motion::Scroll
    } else {
        Motion::Still
    }
}

/// Where the window starts for a motion: the busiest 128x32 for a still,
/// the busiest 32-row band for a pan (from the left), the busiest 128-px
/// column for a scroll (from the top).
pub fn auto_window(stage: &Image, motion: Motion) -> (usize, usize) {
    match motion {
        Motion::Still => best_window(stage, W, H, 0.15),
        Motion::Pan => (0, best_window(stage, stage.w, H, 0.15).1),
        Motion::Scroll => (best_window(stage, W, stage.h, 0.15).0, 0),
    }
}

/// A rectangle: x, y, w, h.
pub type Rect = (usize, usize, usize, usize);

/// Every window's top-left, in order, and the rectangle they cover
/// together (what is shown at some point: where the background key and
/// the palette look). `x`/`y` are clamped; pan ignores `x`, scroll `y`.
pub fn windows(
    sw: usize,
    sh: usize,
    motion: Motion,
    x: usize,
    y: usize,
) -> (Rect, Vec<(usize, usize)>) {
    let (x, y) = (x.min(sw - W), y.min(sh - H));
    match motion {
        Motion::Still => ((x, y, W, H), vec![(x, y)]),
        Motion::Pan => ((0, y, sw, H), (0..=sw - W).map(|x| (x, y)).collect()),
        Motion::Scroll => ((x, 0, W, sh), (0..=sh - H).map(|y| (x, y)).collect()),
    }
}

/// Frame delays in ms: a still (or a move with nowhere to go) is one frame
/// of [`STILL_MS`]; a move is `step_ms` a frame with the first and last
/// held for `hold_ms` (never shorter than a step).
pub fn delays(n: usize, motion: Motion, step_ms: u32, hold_ms: u32) -> Vec<u32> {
    if motion == Motion::Still || n <= 1 {
        return vec![STILL_MS; n.min(1)];
    }
    let step = step_ms.max(20);
    let mut d = vec![step; n];
    d[0] = hold_ms.max(step);
    d[n - 1] = hold_ms.max(step);
    d
}

// --- Look ---------------------------------------------------------------------

/// "Background to black": the commonest colour of `img` and everything
/// within `tol` of it on every channel become black. Returns that colour.
pub fn key_background(img: &mut Image, tol: u8) -> Option<[u8; 3]> {
    let bgc = commonest(img)?;
    key_colour(img, bgc, tol);
    Some(bgc)
}

/// The commonest colour of `img` (ties: the lower `0xRRGGBB`).
pub fn commonest(img: &Image) -> Option<[u8; 3]> {
    let mut counts: HashMap<u32, u32> = HashMap::new();
    for &p in &img.px {
        *counts.entry(rgb_key(p)).or_default() += 1;
    }
    let (&bg, _) = counts
        .iter()
        .max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(a.0)))?;
    Some([(bg >> 16) as u8, (bg >> 8) as u8, bg as u8])
}

/// Everything within `tol` of `bgc` on every channel becomes black.
pub fn key_colour(img: &mut Image, bgc: [u8; 3], tol: u8) {
    for p in &mut img.px {
        if (0..3).all(|c| p[c].abs_diff(bgc[c]) <= tol) {
            *p = [0, 0, 0, p[3]];
        }
    }
}

/// Brightness as a gain on the pixels, in percent (the panel's own
/// brightness is a device setting).
pub fn gain(img: &mut Image, percent: u32) {
    if percent == 100 {
        return;
    }
    for p in &mut img.px {
        for c in &mut p[..3] {
            *c = ((*c as u32 * percent + 50) / 100).min(255) as u8;
        }
    }
}

/// How many colours the GIF may use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Colours {
    /// Exact when the animation has 256 or fewer, else 256 picked for spread.
    Auto,
    Max(usize),
}

impl Colours {
    pub fn from_count(n: u32) -> Colours {
        if n == 0 {
            Colours::Auto
        } else {
            Colours::Max((n as usize).clamp(2, 256))
        }
    }
    pub fn max(self) -> usize {
        match self {
            Colours::Auto => 256,
            Colours::Max(n) => n,
        }
    }
}

/// The colours of `img` with their counts, most used first (ties: lower
/// value first), as `(0xRRGGBB, count)`.
pub fn colour_counts(img: &Image) -> Vec<(u32, u32)> {
    let mut counts: HashMap<u32, u32> = HashMap::new();
    for &p in &img.px {
        *counts.entry(rgb_key(p)).or_default() += 1;
    }
    let mut v: Vec<(u32, u32)> = counts.into_iter().collect();
    v.sort_unstable_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    v
}

fn split(c: u32) -> [i32; 3] {
    [
        (c >> 16) as i32 & 255,
        (c >> 8) as i32 & 255,
        c as i32 & 255,
    ]
}

fn dist2(a: [i32; 3], b: [i32; 3]) -> i32 {
    (0..3).map(|i| (a[i] - b[i]) * (a[i] - b[i])).sum()
}

/// At most `max` of the picture's own colours: all of them when they fit;
/// else the most used first, then again and again whichever is furthest
/// from everything picked so far, weighted by log(1 + uses). Count-driven
/// quantisers (median cut, NeuQuant) drop the rare pixels that matter — the
/// yellow coin, the blue gem, the green "D" — and this keeps them. `counts`
/// as [`colour_counts`] gives them.
pub fn spread_pick(counts: &[(u32, u32)], max: usize) -> Vec<u32> {
    if counts.len() <= max {
        return counts.iter().map(|c| c.0).collect();
    }
    let rgb: Vec<[i32; 3]> = counts.iter().map(|c| split(c.0)).collect();
    let wt: Vec<f64> = counts.iter().map(|c| (c.1 as f64).ln_1p()).collect();
    let mut picked = vec![0usize];
    let mut dmin: Vec<i32> = rgb.iter().map(|&p| dist2(p, rgb[0])).collect();
    while picked.len() < max {
        let (mut bi, mut bv) = (0, -1.0);
        for (i, &d) in dmin.iter().enumerate() {
            let v = (d as f64).sqrt() * wt[i];
            if v > bv {
                bv = v;
                bi = i;
            }
        }
        if bv <= 0.0 {
            break;
        }
        picked.push(bi);
        for (i, d) in dmin.iter_mut().enumerate() {
            *d = (*d).min(dist2(rgb[i], rgb[bi]));
        }
    }
    picked.iter().map(|&i| counts[i].0).collect()
}

/// Nearest-colour lookup into a palette, memoised.
pub struct Mapper {
    pal: Vec<[i32; 3]>,
    memo: HashMap<u32, u8>,
}

impl Mapper {
    pub fn new(palette: &[u32]) -> Mapper {
        Mapper {
            pal: palette.iter().map(|&c| split(c)).collect(),
            memo: HashMap::new(),
        }
    }
    pub fn index(&mut self, c: u32) -> u8 {
        let pal = &self.pal;
        *self.memo.entry(c).or_insert_with(|| {
            let p = split(c);
            let mut best = (0usize, i32::MAX);
            for (i, &q) in pal.iter().enumerate() {
                let d = dist2(p, q);
                if d < best.1 {
                    best = (i, d);
                }
            }
            best.0 as u8
        })
    }
}

/// Everything the Studio's controls decide.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Plan {
    pub motion: Motion,
    pub x: usize,
    pub y: usize,
    pub step_ms: u32,
    pub hold_ms: u32,
    pub key: bool,
    /// Brightness, percent.
    pub gain: u32,
    pub colours: Colours,
}

impl Default for Plan {
    /// The defaults from the pack: 1 px per 40 ms, 1.5 s holds.
    fn default() -> Plan {
        Plan {
            motion: Motion::Still,
            x: 0,
            y: 0,
            step_ms: 40,
            hold_ms: 1_500,
            key: false,
            gain: 100,
            colours: Colours::Auto,
        }
    }
}

/// 128x32 frames over one global palette.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Animation {
    /// `0xRRGGBB`, at most 256.
    pub palette: Vec<u32>,
    /// `W * H` palette indices each.
    pub frames: Vec<Vec<u8>>,
    pub delays_ms: Vec<u32>,
}

impl Animation {
    /// One frame as RGB bytes (what a decoder must show).
    pub fn rgb(&self, i: usize) -> Vec<u8> {
        self.frames[i]
            .iter()
            .flat_map(|&k| {
                let c = self.palette[k as usize];
                [(c >> 16) as u8, (c >> 8) as u8, c as u8]
            })
            .collect()
    }
    pub fn duration_ms(&self) -> u64 {
        self.delays_ms.iter().map(|&d| d as u64).sum()
    }
}

/// The animation for a stage (see [`stage`]) and a plan. The background
/// key, brightness and palette are worked out over what the windows cover
/// together, once; each frame is then a crop of its indices.
pub fn render(stage: &Image, plan: &Plan) -> Animation {
    render_with(stage, plan, None)
}

/// [`render`], with a text overlay drawn over every frame (after the Look:
/// the text keeps its colour; its colour joins the palette).
pub fn render_with(stage: &Image, plan: &Plan, text: Option<&Overlay>) -> Animation {
    let (rect, wins, region) = looked(stage, plan);
    let mask = text.map(|t| (t.mask(), t.rgb()));
    let mut counts = colour_counts(&region);
    if let Some((m, c)) = &mask {
        let lit = m.iter().filter(|&&b| b).count() as u32 * wins.len() as u32;
        match counts.iter_mut().find(|e| e.0 == *c) {
            Some(e) => e.1 += lit,
            None if lit > 0 => counts.push((*c, lit)),
            None => {}
        }
        counts.sort_unstable_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    }
    let palette = spread_pick(&counts, plan.colours.max());
    let mut map = Mapper::new(&palette);
    let idx: Vec<u8> = region.px.iter().map(|&p| map.index(rgb_key(p))).collect();
    let text_idx = mask.as_ref().map(|(_, c)| map.index(*c));
    let frames = wins
        .iter()
        .map(|&(x, y)| {
            let (ox, oy) = (x - rect.0, y - rect.1);
            let mut f = Vec::with_capacity(W * H);
            for row in oy..oy + H {
                f.extend_from_slice(&idx[row * rect.2 + ox..row * rect.2 + ox + W]);
            }
            if let (Some((m, _)), Some(k)) = (&mask, text_idx) {
                for (p, &on) in f.iter_mut().zip(m) {
                    if on {
                        *p = k;
                    }
                }
            }
            f
        })
        .collect::<Vec<_>>();
    let delays_ms = delays(frames.len(), plan.motion, plan.step_ms, plan.hold_ms);
    Animation {
        palette,
        frames,
        delays_ms,
    }
}

/// What the windows cover together, with the background key and the
/// brightness applied: the rectangle, each window's top-left, the pixels.
fn looked(stage: &Image, plan: &Plan) -> (Rect, Vec<(usize, usize)>, Image) {
    let (rect, wins) = windows(stage.w, stage.h, plan.motion, plan.x, plan.y);
    let mut region = stage.crop(rect.0, rect.1, rect.2, rect.3);
    if plan.key {
        key_background(&mut region, 12);
    }
    gain(&mut region, plan.gain);
    (rect, wins, region)
}

/// [`render_with`] in full colour, with no palette: each frame's `W * H`
/// pixels, and the delays (the site's Studio keeps every colour a picture
/// has; `plan.colours` is not used).
pub fn render_exact(stage: &Image, plan: &Plan, text: Option<&Overlay>) -> (Vec<Image>, Vec<u32>) {
    let (rect, wins, region) = looked(stage, plan);
    let mask = text.map(|t| (t.mask(), t.rgb()));
    let frames: Vec<Image> = wins
        .iter()
        .map(|&(x, y)| {
            let mut f = region.crop(x - rect.0, y - rect.1, W, H);
            if let Some((m, c)) = &mask {
                for (p, &on) in f.px.iter_mut().zip(m) {
                    if on {
                        *p = [(c >> 16) as u8, (c >> 8) as u8, *c as u8, 255];
                    }
                }
            }
            f
        })
        .collect();
    let d = delays(frames.len(), plan.motion, plan.step_ms, plan.hold_ms);
    (frames, d)
}

// --- Text -----------------------------------------------------------------------

/// A line of text over the frames, in one of the core's two panel fonts
/// (`opendmd_core::text`: 4x6 and 5x7), at one of nine anchors (0..=8,
/// row-major from the top left, a 1 px margin) plus a nudge, over the
/// source frames `from..=to` (an animation; a still's frames are all
/// covered).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Overlay {
    pub msg: String,
    pub small: bool,
    /// `0xRRGGBB`.
    pub colour: u32,
    pub anchor: u8,
    pub dx: i32,
    pub dy: i32,
    pub from: usize,
    pub to: usize,
}

impl Overlay {
    fn font(&self) -> opendmd_core::text::Font {
        if self.small {
            opendmd_core::text::Font::Small
        } else {
            opendmd_core::text::Font::Normal
        }
    }

    /// Top-left of the text on the panel.
    pub fn origin(&self) -> (i32, i32) {
        let f = self.font();
        let (w, h) = (
            opendmd_core::text::text_width(f, &self.msg, 1) as i32,
            f.height() as i32,
        );
        let at = |slot: u8, room: i32| match slot {
            0 => 1,
            1 => room / 2,
            _ => room - 1,
        };
        (
            at(self.anchor % 3, W as i32 - w) + self.dx,
            at((self.anchor / 3).min(2), H as i32 - h) + self.dy,
        )
    }

    /// The lit pixels, `W * H`, row-major.
    pub fn mask(&self) -> Vec<bool> {
        use opendmd_core::canvas::{Canvas, Rgb};
        let mut c = Canvas::new();
        let (x, y) = self.origin();
        opendmd_core::text::draw_text_with(
            &mut c,
            x,
            y,
            &self.msg,
            Rgb::new(255, 255, 255),
            self.font(),
            1,
        );
        (0..W * H).map(|i| c.get(i % W, i / W).r != 0).collect()
    }

    pub fn rgb(&self) -> u32 {
        self.colour & 0xFF_FFFF
    }

    pub fn covers(&self, frame: usize) -> bool {
        (self.from..=self.to).contains(&frame)
    }

    /// Draw it (given its [`mask`](Self::mask)) over a 128x32 frame.
    pub fn draw(&self, mask: &[bool], img: &mut Image) {
        let c = self.rgb();
        let px = [(c >> 16) as u8, (c >> 8) as u8, c as u8, 255];
        for (p, &on) in img.px.iter_mut().zip(mask) {
            if on {
                *p = px;
            }
        }
    }
}

// --- Timeline -------------------------------------------------------------------

/// A GIF frame's delay as the player shows it (`opendmd_core::player`): the
/// file's hundredths, 0 meaning 100 ms, never under 20 ms.
pub fn player_delay(hundredths: u16) -> u32 {
    if hundredths == 0 {
        100
    } else {
        (hundredths as u32 * 10).max(20)
    }
}

/// The shortest delay written (the player's floor).
pub const MIN_DELAY_MS: u32 = 20;

/// What the timeline decides: the source frames `from..=to` (trim) minus
/// the deleted ones, played at `speed` percent, backwards, and/or there and
/// back again.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edit {
    pub from: usize,
    pub to: usize,
    /// Percent: 25 (a quarter speed) to 400 (4x).
    pub speed: u32,
    pub reverse: bool,
    pub pingpong: bool,
    /// Source frames taken out (any order).
    pub deleted: Vec<usize>,
}

impl Edit {
    /// Every frame, as it came.
    pub fn all(n: usize) -> Edit {
        Edit {
            from: 0,
            to: n.saturating_sub(1),
            speed: 100,
            reverse: false,
            pingpong: false,
            deleted: Vec::new(),
        }
    }
}

/// A delay at a speed: `ms * 100 / speed`, rounded to the 10 ms a GIF can
/// hold, never under [`MIN_DELAY_MS`].
pub fn scaled_delay(ms: u32, speed: u32) -> u32 {
    let speed = speed.clamp(25, 400) as u64;
    let v = (ms as u64 * 100 + speed * 5) / (speed * 10) * 10;
    (v as u32).max(MIN_DELAY_MS)
}

/// The edited animation as `(source frame, delay ms)` in play order.
/// Trim and delete pick the frames (in source order), reverse turns them
/// round, ping-pong appends them backwards without repeating either end
/// (`0 1 2 3` -> `0 1 2 3 2 1`, then the loop starts at 0 again). Delays
/// are the source's ([`player_delay`]) scaled by [`scaled_delay`]. Empty
/// when nothing is left.
pub fn sequence(delays_ms: &[u32], e: &Edit) -> Vec<(usize, u32)> {
    let n = delays_ms.len();
    if n == 0 {
        return Vec::new();
    }
    let mut gone = vec![false; n];
    for &d in &e.deleted {
        if d < n {
            gone[d] = true;
        }
    }
    let (a, b) = (e.from.min(n - 1), e.to.min(n - 1));
    let (a, b) = (a.min(b), a.max(b));
    let mut base: Vec<usize> = (a..=b).filter(|&i| !gone[i]).collect();
    if e.reverse {
        base.reverse();
    }
    if e.pingpong && base.len() > 2 {
        let back: Vec<usize> = base[1..base.len() - 1].iter().rev().copied().collect();
        base.extend(back);
    }
    base.into_iter()
        .map(|i| (i, scaled_delay(delays_ms[i], e.speed)))
        .collect()
}

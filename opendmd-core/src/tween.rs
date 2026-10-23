//! **Smooth motion**: the shift tweener from `docs/tweening-experiment.md`,
//! ported from the experiment's Python (`opendmd-web/experiments/tween/
//! common.py` + `tween.py`, the reference) — opt-in per file, applied to the
//! prepared copy only: `recode_gif(tween(original))`. Built only with the
//! `tween` feature (`opendmd-pack` and the art wasm module); the firmware
//! never compiles it.
//!
//! Every consecutive frame pair — the loop's last -> first included — is
//! classified on colour-exact composited frames (static / translation /
//! regional / cut / other). A translation or regional pair whose integer
//! shift explains ≥ 97% of its changed pixels ([`TWEEN_FIT`]) and whose two
//! ends agree about every intermediate step ([`INCONS_MAX`]) gets
//! `|shift| - 1` in-between frames, as many as its delay splits into
//! ≥ 20 ms parts on the GIF's 10 ms grid, unless the delay is a hold
//! (above `max(2 × median, 200 ms)`). Every in-between pixel is a pixel of
//! one end moved by whole pixels: nothing is blended, no colour is new, and
//! the total duration is unchanged.
//!
//! [`Smooth`] does it in steps (the browser yields between them): decode +
//! analyse, then write the tweened frames as a plain GIF (every frame whole,
//! LZW at 12 bits) that [`crate::gifslim::recode_gif`] turns into the copy,
//! exactly as it would any GIF. [`smooth_gif`] is the whole pipeline in one
//! call, and marks the copy (`OPENDMD1TWN`, [`insert_marker`] /
//! [`marker`]; `docs/cache-format.md`).
//!
//! Frames are the GIF's logical screen as decoded (a 135x32 stray stays
//! 135x32; the experiment centre-cropped to 128x32 first — the one place
//! the two can differ).

use crate::canvas::Canvas;
use crate::engine::ReadSeek;
use crate::gifslim::{
    self, GifSlim, LzwEncoder, RecodeOptions, RecodeStats, SubBlocks, HINT_APP_ID, MAX_CAP,
};
use crate::player::GifSource;
use std::io::{self, Cursor, Seek, SeekFrom, Write};
use std::rc::Rc;

/// Shift search radius, px (both axes).
pub const RADIUS: i32 = 8;
/// At most this share of the pixels changes: a static pair.
const STATIC_MAX: f64 = 0.005;
/// A global shift explains this much of the changed pixels *and* of the
/// whole frame: translation.
const TRANS_MIN: f64 = 0.90;
/// One connected region holds 90% of the changes and a shift explains this
/// much of it: regional.
const REGION_MIN: f64 = 0.80;
/// More than this share changed and no fit: a cut.
const CUT_MIN: f64 = 0.60;
/// The tweener's own fit, stricter than the classifier's.
pub const TWEEN_FIT: f64 = 0.97;
/// The consistency gate: the worst step's disagreements between A moved
/// forward and B moved back, as a share of the pair's changed pixels.
pub const INCONS_MAX: f64 = 0.25;
/// Changed pixels tested per shift (a strided subsample).
const SAMPLE: usize = 512;
/// The shortest frame a split may make, ms (two panel refreshes).
pub const MIN_PART_MS: u32 = 20;
/// A delay above `max(2 × median, HOLD_MIN_MS)` is a pause, not motion.
pub const HOLD_MIN_MS: u32 = 200;
/// The UI offers Smooth motion when at least this share (%) of the pairs
/// would be tweened.
pub const OFFER_PERCENT: u32 = 10;
/// The copy's marker: an application extension right after the
/// `OPENDMD1LZW` hint (identifier `OPENDMD1`, authentication code `TWN`).
pub const MARKER_APP_ID: &[u8; 11] = b"OPENDMD1TWN";
/// The marker's version byte.
pub const MARKER_VERSION: u8 = 1;

fn invalid(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.into())
}

// ---------------------------------------------------------------------------
// The classifier
// ---------------------------------------------------------------------------

/// A pair's motion class (the experiment's, applied in this order).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    Static,
    Translation,
    Regional,
    Cut,
    Other,
}

impl Class {
    pub fn name(self) -> &'static str {
        match self {
            Class::Static => "static",
            Class::Translation => "translation",
            Class::Regional => "regional",
            Class::Cut => "cut",
            Class::Other => "other",
        }
    }
}

/// One frame pair: frame `i` -> frame `i + 1` (the last pair wraps to 0).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pair {
    pub class: Class,
    /// `(dx, dy)` for translation and regional pairs, else `(0, 0)`.
    pub shift: (i32, i32),
    /// Regional: the changed region's box, `[y0, y1, x0, x1)`.
    pub bbox: [usize; 4],
    /// The share of changed pixels the shift explains (0 when no shift).
    pub fit: f64,
    /// Passed the tweener's gates (fit and consistency), before the hold
    /// rule and the delay split.
    pub gate: bool,
    /// In-betweens this pair gets: the final decision.
    pub k: u32,
    /// Frame `i`'s delay after the player's rules, ms.
    pub delay_ms: u32,
}

/// A frame as colour codes (`r << 16 | g << 8 | b`), row-major.
struct Img<'a> {
    px: &'a [u32],
    w: usize,
    h: usize,
}

impl Img<'_> {
    /// `self[x - dx, y - dy]` if that is in frame — the experiment's
    /// `_shifted(img, dx, dy)[p]`.
    #[inline]
    fn from(&self, x: usize, y: usize, dx: i32, dy: i32) -> Option<u32> {
        let sx = x as i64 - dx as i64;
        let sy = y as i64 - dy as i64;
        (sx >= 0 && sy >= 0 && (sx as usize) < self.w && (sy as usize) < self.h)
            .then(|| self.px[sy as usize * self.w + sx as usize])
    }
}

/// Offsets within `±RADIUS`, ordered by (Chebyshev, Manhattan) magnitude
/// and otherwise as generated (`dy` outer, `dx` inner) — the experiment's
/// stable sort, so the first best shift on a tie is the smallest. Built at
/// compile time (no sort, no lazy static in the wasm).
const SHIFTS: [(i8, i8); 288] = {
    let mut out = [(0i8, 0i8); 288];
    let mut n = 0;
    let mut cheb = 1;
    while cheb <= RADIUS {
        let mut man = cheb;
        while man <= 2 * cheb {
            let mut dy = -RADIUS;
            while dy <= RADIUS {
                let mut dx = -RADIUS;
                while dx <= RADIUS {
                    let (ax, ay) = (if dx < 0 { -dx } else { dx }, if dy < 0 { -dy } else { dy });
                    if (if ax > ay { ax } else { ay }) == cheb && ax + ay == man {
                        out[n] = (dx as i8, dy as i8);
                        n += 1;
                    }
                    dx += 1;
                }
                dy += 1;
            }
            man += 1;
        }
        cheb += 1;
    }
    assert!(n == 288);
    out
};

/// Outside the frame in a [`Padded`] image: never a colour code (those are
/// 24-bit), so it equals nothing a frame holds.
const OUTSIDE: u32 = u32::MAX;

/// A frame with a border of [`RADIUS`] pixels of [`OUTSIDE`] on every side
/// (the experiment's `_pad`): any read shifted by at most the radius is a
/// plain index, no bounds check, which is what makes the shift search fast.
struct Padded {
    px: Vec<u32>,
    wp: usize,
}

impl Padded {
    fn new(img: &Img) -> Self {
        let r = RADIUS as usize;
        let wp = img.w + 2 * r;
        let mut px = vec![OUTSIDE; wp * (img.h + 2 * r)];
        for y in 0..img.h {
            px[(y + r) * wp + r..(y + r) * wp + r + img.w]
                .copy_from_slice(&img.px[y * img.w..(y + 1) * img.w]);
        }
        Padded { px, wp }
    }

    /// Where frame pixel `p` of a `w`-wide frame is.
    #[inline]
    fn at(&self, p: usize, w: usize) -> usize {
        let r = RADIUS as usize;
        (p / w + r) * self.wp + p % w + r
    }

    /// The index offset of a shift.
    #[inline]
    fn offset(&self, (dx, dy): (i32, i32)) -> isize {
        dy as isize * self.wp as isize + dx as isize
    }
}

/// The best shift for the changed pixels `idx` (raster positions): a pixel
/// is explained if it arrived (`B[p] == A[p - s]`) or left (`A[p] ==
/// B[p + s]`). Returns (share explained among the pixels with either end in
/// frame, shift); a shift that leaves less than half the sample in frame
/// scores 0. At most [`SAMPLE`] pixels, strided like the experiment's.
///
/// Only a share of at least `floor` matters to the caller (the class's
/// threshold): when no shift reaches it, the share returned is below it but
/// not necessarily the best (the search stops counting a shift as soon as
/// it cannot reach the floor, or beat the best so far). When one does, the
/// result is exactly the full search's: the first shift with the best share.
fn best_shift(
    a: &Img,
    b: &Img,
    pa: &Padded,
    pb: &Padded,
    idx: &[u32],
    floor: f64,
) -> (f64, (i32, i32)) {
    let w = a.w;
    let sampled: Vec<u32>;
    let sel: &[u32] = if idx.len() > SAMPLE {
        let step = idx.len() as f64 / SAMPLE as f64;
        sampled = (0..SAMPLE)
            .map(|i| idx[(i as f64 * step) as usize])
            .collect();
        &sampled
    } else {
        idx
    };
    // Per sample: its padded index, A[p], B[p].
    let pts: Vec<(isize, u32, u32)> = sel
        .iter()
        .map(|&p| {
            let p = p as usize;
            (pa.at(p, w) as isize, a.px[p], b.px[p])
        })
        .collect();
    let n = pts.len() as u32;
    let half = 0.5 * n as f64;
    let (mut best, mut best_s) = (f64::NEG_INFINITY, (0, 0));
    'shifts: for &(dx, dy) in &SHIFTS {
        let s = (dx as i32, dy as i32);
        let off = pa.offset(s);
        let (mut m, mut nin, mut done) = (0u32, 0u32, 0u32);
        for chunk in pts.chunks(32) {
            // However the rest turns out, this shift's share is at most
            // (m + r) / (nin + r) with r samples left (rounding is
            // monotonic): if that is below the floor, or cannot beat the
            // best so far (a strict improvement is needed), the rest need
            // not be counted.
            let r = n - done;
            let bound = (m + r) as f64 / (nin + r).max(1) as f64;
            if bound < floor || bound <= best {
                continue 'shifts;
            }
            for &(pp, av, bv) in chunk {
                // A[p - s] and B[p + s]; OUTSIDE equals no colour.
                let from = pa.px[(pp - off) as usize];
                let to = pb.px[(pp + off) as usize];
                nin += (from != OUTSIDE || to != OUTSIDE) as u32;
                m += (from == bv || to == av) as u32;
            }
            done += chunk.len() as u32;
        }
        let share = if nin as f64 >= half {
            m as f64 / nin.max(1) as f64
        } else {
            0.0
        };
        if share > best {
            best = share;
            best_s = s;
        }
    }
    (best.max(0.0), best_s)
}

/// Share of the whole frame (where `A[p - s]` is in frame) with `B[p] ==
/// A[p - s]`: a sprite over a static textured background explains its own
/// changes with its shift, but the background does not follow it.
fn frame_fit(b: &Img, pa: &Padded, s: (i32, i32)) -> f64 {
    let off = pa.offset(s);
    let (mut ok, mut eq) = (0u32, 0u32);
    for (p, &bv) in b.px.iter().enumerate() {
        let v = pa.px[(pa.at(p, b.w) as isize - off) as usize];
        ok += (v != OUTSIDE) as u32;
        eq += (v == bv) as u32;
    }
    eq as f64 / ok.max(1) as f64
}

/// `scipy.ndimage.binary_dilation(mask, iterations=r)` with its default
/// cross: every pixel within Manhattan distance `r` of a set one.
fn dilate(mask: &[bool], w: usize, h: usize, r: u32) -> Vec<bool> {
    let mut cur = mask.to_vec();
    for _ in 0..r {
        let mut next = cur.clone();
        for y in 0..h {
            for x in 0..w {
                if cur[y * w + x] {
                    continue;
                }
                let p = y * w + x;
                if (x > 0 && cur[p - 1])
                    || (x + 1 < w && cur[p + 1])
                    || (y > 0 && cur[p - w])
                    || (y + 1 < h && cur[p + w])
                {
                    next[p] = true;
                }
            }
        }
        cur = next;
    }
    cur
}

/// 8-connected components, numbered 1.. in raster order of each one's first
/// pixel (`scipy.ndimage.label` with a 3x3 structure).
fn label8(mask: &[bool], w: usize, h: usize) -> (Vec<u32>, u32) {
    let mut lab = vec![0u32; mask.len()];
    let mut n = 0;
    let mut stack = Vec::new();
    for start in 0..mask.len() {
        if !mask[start] || lab[start] != 0 {
            continue;
        }
        n += 1;
        lab[start] = n;
        stack.push(start);
        while let Some(p) = stack.pop() {
            let (x, y) = ((p % w) as i64, (p / w) as i64);
            for ny in y - 1..=y + 1 {
                for nx in x - 1..=x + 1 {
                    if nx < 0 || ny < 0 || nx >= w as i64 || ny >= h as i64 {
                        continue;
                    }
                    let q = ny as usize * w + nx as usize;
                    if mask[q] && lab[q] == 0 {
                        lab[q] = n;
                        stack.push(q);
                    }
                }
            }
        }
    }
    (lab, n)
}

/// Classify one pair (no gates): the experiment's `classify`.
fn classify(a: &Img, b: &Img) -> Pair {
    let (w, h) = (a.w, a.h);
    let total = w * h;
    let diff: Vec<bool> = a.px.iter().zip(b.px).map(|(x, y)| x != y).collect();
    let idx: Vec<u32> = (0..total as u32).filter(|&p| diff[p as usize]).collect();
    let n = idx.len();
    let mut pair = Pair {
        class: Class::Static,
        shift: (0, 0),
        bbox: [0; 4],
        fit: 0.0,
        gate: false,
        k: 0,
        delay_ms: 0,
    };
    if n as f64 <= STATIC_MAX * total as f64 {
        return pair;
    }
    let (pa, pb) = (Padded::new(a), Padded::new(b));
    let (share, s) = best_shift(a, b, &pa, &pb, &idx, TRANS_MIN);
    if share >= TRANS_MIN && frame_fit(b, &pa, s) >= TRANS_MIN {
        pair.class = Class::Translation;
        pair.shift = s;
        pair.fit = share;
        return pair;
    }
    // One connected changed region (8-connected, gaps of 2 px bridged).
    let grown = dilate(&diff, w, h, 2);
    let (lab, nlab) = label8(&grown, w, h);
    if nlab >= 1 {
        let mut sizes = vec![0usize; nlab as usize + 1];
        for &p in &idx {
            sizes[lab[p as usize] as usize] += 1;
        }
        // argmax: the first (lowest-numbered) on a tie.
        let mut big = 0;
        for (i, &s) in sizes.iter().enumerate() {
            if s > sizes[big] {
                big = i;
            }
        }
        if sizes[big] as f64 >= 0.9 * n as f64 {
            let region: Vec<u32> = idx
                .iter()
                .copied()
                .filter(|&p| lab[p as usize] as usize == big)
                .collect();
            let (share, s) = best_shift(a, b, &pa, &pb, &region, REGION_MIN);
            if share >= REGION_MIN {
                let (mut y0, mut y1, mut x0, mut x1) = (usize::MAX, 0, usize::MAX, 0);
                for &p in &region {
                    let (x, y) = (p as usize % w, p as usize / w);
                    y0 = y0.min(y);
                    y1 = y1.max(y + 1);
                    x0 = x0.min(x);
                    x1 = x1.max(x + 1);
                }
                pair.class = Class::Regional;
                pair.shift = s;
                pair.bbox = [y0, y1, x0, x1];
                pair.fit = share;
                return pair;
            }
        }
    }
    pair.class = if n as f64 > CUT_MIN * total as f64 {
        Class::Cut
    } else {
        Class::Other
    };
    pair
}

/// Chebyshev length of a shift: the pixels an in-between can step by.
fn magnitude((dx, dy): (i32, i32)) -> u32 {
    dx.unsigned_abs().max(dy.unsigned_abs())
}

/// The regional pair's box grown by the shift, which covers the path.
fn region_mask(bbox: [usize; 4], n: u32, w: usize, h: usize) -> Vec<bool> {
    let [y0, y1, x0, x1] = bbox;
    let n = n as usize;
    let mut m = vec![false; w * h];
    for y in y0.saturating_sub(n)..(y1 + n).min(h) {
        for x in x0.saturating_sub(n)..(x1 + n).min(w) {
            m[y * w + x] = true;
        }
    }
    m
}

/// The moving object of a regional pair: pixels of the region within `n`
/// of a change that move with the shift (`A[q] == B[q + s]`).
fn object(a: &Img, b: &Img, region: &[bool], s: (i32, i32), n: u32) -> Vec<bool> {
    let changed: Vec<bool> = a.px.iter().zip(b.px).map(|(x, y)| x != y).collect();
    let near = dilate(&changed, a.w, a.h, n);
    let mut oa = vec![false; a.px.len()];
    for y in 0..a.h {
        for x in 0..a.w {
            let p = y * a.w + x;
            oa[p] = near[p] && region[p] && b.from(x, y, -s.0, -s.1) == Some(a.px[p]);
        }
    }
    oa
}

/// `round()` as Python does it: halves to even.
fn py_round(v: f64) -> i32 {
    v.round_ties_even() as i32
}

/// The consistency gate's measure: the worst, over the shift's integer
/// steps, of the pixels where A moved forward and B moved back disagree
/// (where the pair changes or the step changes the pixel; for a regional
/// pair only where the moving object lands), as a share of the pair's
/// changed pixels. Rigid motion scores 0.
fn inconsistency(a: &Img, b: &Img, pair: &Pair) -> f64 {
    let (dx, dy) = pair.shift;
    let n = magnitude(pair.shift);
    let (w, h) = (a.w, a.h);
    let nch = a.px.iter().zip(b.px).filter(|(x, y)| x != y).count().max(1);
    let oa = (pair.class == Class::Regional).then(|| {
        let region = region_mask(pair.bbox, n, w, h);
        object(a, b, &region, pair.shift, n)
    });
    let mut worst = 0.0f64;
    for t in 1..n as i32 {
        let ox = py_round((t * dx) as f64 / n as f64);
        let oy = py_round((t * dy) as f64 / n as f64);
        let mut bad = 0usize;
        for y in 0..h {
            for x in 0..w {
                let p = y * w + x;
                let (Some(sa), Some(sb)) = (a.from(x, y, ox, oy), b.from(x, y, ox - dx, oy - dy))
                else {
                    continue;
                };
                if sa == sb || !(a.px[p] != b.px[p] || sa != a.px[p]) {
                    continue;
                }
                if let Some(oa) = &oa {
                    let sx = x as i64 - ox as i64;
                    let sy = y as i64 - oy as i64;
                    let inside = sx >= 0 && sy >= 0 && (sx as usize) < w && (sy as usize) < h;
                    if !(inside && oa[sy as usize * w + sx as usize]) {
                        continue;
                    }
                }
                bad += 1;
            }
        }
        worst = worst.max(bad as f64 / nch as f64);
    }
    worst
}

/// Split `d` ms into `k + 1` parts on the GIF's 10 ms grid, each ≥ 20 ms,
/// the remainder going to the first parts; `None` if it cannot be done.
pub fn split_delay(d: u32, k: u32) -> Option<Vec<u32>> {
    let cs = d / 10;
    if k < 1 || cs < 2 * (k + 1) || !d.is_multiple_of(10) {
        return None;
    }
    let (base, rem) = (cs / (k + 1), cs % (k + 1));
    Some((0..=k).map(|i| (base + (i < rem) as u32) * 10).collect())
}

/// How many in-betweens a pair with this shift and delay gets: one per
/// extra pixel of motion, as many as the delay splits into ≥ 20 ms parts,
/// none on a hold.
pub fn n_inbetweens(shift: (i32, i32), d: u32, hold_limit: u32) -> u32 {
    if d > hold_limit {
        return 0;
    }
    let mut k = magnitude(shift).saturating_sub(1);
    while k >= 1 && split_delay(d, k).is_none() {
        k -= 1;
    }
    k
}

/// The hold limit: `max(2 × int(median delay), 200 ms)` (numpy's median:
/// the mean of the middle two for an even count).
pub fn hold_limit(delays_ms: &[u32]) -> u32 {
    if delays_ms.is_empty() {
        return HOLD_MIN_MS;
    }
    let mut d = delays_ms.to_vec();
    d.sort_unstable();
    let n = d.len();
    let median = if n % 2 == 1 {
        d[n / 2]
    } else {
        (d[n / 2 - 1] + d[n / 2]) / 2
    };
    (2 * median).max(HOLD_MIN_MS)
}

/// The player's delay rule: 0 -> 100 ms, else floored at 20 ms.
pub fn player_delay_ms(raw: u16) -> u32 {
    if raw == 0 {
        100
    } else {
        (raw as u32 * 10).max(20)
    }
}

/// One in-between at fraction `f` of the pair's integer shift, into `out`.
/// Every pixel is a pixel of A or B from somewhere; nothing is blended.
///
/// Global translation: A moved by `o = round(f × shift)`, the strip entering
/// the frame from B moved back by `shift - o`; where both ends are in frame
/// and disagree (a static HUD, a flicker), a pixel the same in A and B
/// keeps it, otherwise the nearer end in time shows.
///
/// Regional (a sprite over a background), in layers: the object (the part
/// of the region that moves with the shift) is pasted at `o` over a
/// background that is A where the object is not in A, else B where it is
/// not in B, else the nearer end. Outside the region, A.
fn inbetween(a: &Img, b: &Img, pair: &Pair, f: f64, out: &mut Vec<u32>) {
    let (dx, dy) = pair.shift;
    let (w, h) = (a.w, a.h);
    let ox = py_round(f * dx as f64);
    let oy = py_round(f * dy as f64);
    let near = if f < 0.5 { a } else { b };
    out.clear();
    if pair.class != Class::Regional {
        for y in 0..h {
            for x in 0..w {
                let p = y * w + x;
                let sa = a.from(x, y, ox, oy);
                let sb = b.from(x, y, ox - dx, oy - dy);
                let v = match (sa, sb) {
                    (Some(va), Some(vb)) if va == vb => va,
                    (Some(va), None) => va,
                    (None, Some(vb)) => vb,
                    _ if a.px[p] == b.px[p] => a.px[p],
                    _ => near.px[p],
                };
                out.push(v);
            }
        }
        return;
    }
    let n = magnitude(pair.shift);
    let region = region_mask(pair.bbox, n, w, h);
    let oa = object(a, b, &region, pair.shift, n);
    let at = |m: &[bool], x: usize, y: usize, sx: i32, sy: i32| -> bool {
        let qx = x as i64 - sx as i64;
        let qy = y as i64 - sy as i64;
        qx >= 0
            && qy >= 0
            && (qx as usize) < w
            && (qy as usize) < h
            && m[qy as usize * w + qx as usize]
    };
    for y in 0..h {
        for x in 0..w {
            let p = y * w + x;
            if !region[p] {
                out.push(a.px[p]);
                continue;
            }
            if at(&oa, x, y, ox, oy) {
                // In frame by construction.
                out.push(a.from(x, y, ox, oy).unwrap_or(a.px[p]));
                continue;
            }
            let v = if !oa[p] {
                a.px[p]
            } else if !at(&oa, x, y, dx, dy) {
                b.px[p]
            } else {
                near.px[p]
            };
            out.push(v);
        }
    }
}

// ---------------------------------------------------------------------------
// Decoding: composited logical screens as colour codes
// ---------------------------------------------------------------------------

type Reader = Box<dyn ReadSeek>;

// One per file being smoothed, so the variants' sizes do not matter.
#[allow(clippy::large_enum_variant)]
enum Src {
    Slim(GifSlim<Reader>),
    Full(GifSource<Reader>),
}

/// The source's frames, one at a time, as the recoder decodes them: gifslim
/// at cap 12, the `gif` crate for what gifslim does not decode (the frames
/// already handed out are decoded again and skipped).
struct Frames {
    src: Option<Src>,
    canvas: Canvas,
    delivered: u64,
}

impl Frames {
    fn open(bytes: &Rc<[u8]>) -> io::Result<Self> {
        let r: Reader = Box::new(Cursor::new(bytes.clone()));
        Ok(Frames {
            src: Some(Src::Slim(GifSlim::with_cap(r, MAX_CAP)?)),
            canvas: Canvas::new(),
            delivered: 0,
        })
    }

    fn size(&self) -> (usize, usize) {
        match self.src.as_ref() {
            Some(Src::Slim(s)) => s.logical_size(),
            Some(Src::Full(s)) => s.logical_size(),
            None => (0, 0),
        }
    }

    fn repeat(&self) -> gif::Repeat {
        match self.src.as_ref() {
            Some(Src::Slim(s)) => s.repeat(),
            Some(Src::Full(s)) => s.repeat(),
            None => gif::Repeat::Finite(0),
        }
    }

    /// The next frame's codes into `out` and its raw delay (hundredths).
    fn next(&mut self, out: &mut Vec<u32>) -> io::Result<Option<u16>> {
        loop {
            let res = match self.src.as_mut() {
                Some(Src::Slim(s)) => s.next_frame(&mut self.canvas).map(|f| f.map(|_| ())),
                Some(Src::Full(s)) => s.next_frame(&mut self.canvas).map(|f| f.map(|_| ())),
                None => return Err(invalid("decoder lost")),
            };
            match res {
                Ok(Some(())) => break,
                Ok(None) => return Ok(None),
                Err(e)
                    if matches!(self.src, Some(Src::Slim(_)))
                        && (gifslim::is_unsupported(&e) || gifslim::is_needs_full_decoder(&e)) =>
                {
                    let Some(Src::Slim(s)) = self.src.take() else {
                        unreachable!()
                    };
                    let mut r = s.into_inner();
                    r.seek(SeekFrom::Start(0))?;
                    let mut g = GifSource::new(r)?;
                    self.canvas = Canvas::new();
                    for _ in 0..self.delivered {
                        if g.next_frame(&mut self.canvas)?.is_none() {
                            return Err(invalid("the gif crate found fewer frames"));
                        }
                    }
                    self.src = Some(Src::Full(g));
                }
                Err(e) => return Err(e),
            }
        }
        let (screen, raw) = match self.src.as_ref() {
            Some(Src::Slim(s)) => (s.screen(&self.canvas), s.raw_delay()),
            Some(Src::Full(s)) => (s.screen(&self.canvas), s.raw_delay()),
            None => unreachable!(),
        };
        out.clear();
        out.extend(
            screen
                .as_chunks::<3>()
                .0
                .iter()
                .map(|c| u32::from_be_bytes([0, c[0], c[1], c[2]])),
        );
        self.delivered += 1;
        Ok(Some(raw))
    }
}

// ---------------------------------------------------------------------------
// The file: analysis, the tweened frames, the copy
// ---------------------------------------------------------------------------

/// A set of up to 256 colour codes: open addressing over 512 slots (no
/// hasher, no allocation per insert).
struct ColourSet {
    slots: Vec<u32>,
    len: usize,
}

impl ColourSet {
    fn new() -> Self {
        ColourSet {
            slots: vec![OUTSIDE; 512],
            len: 0,
        }
    }

    /// Add `c`; false if it would be the 257th.
    fn insert(&mut self, c: u32) -> bool {
        let mut i = (c.wrapping_mul(0x9E37_79B1) >> 23) as usize & 511;
        while self.slots[i] != OUTSIDE {
            if self.slots[i] == c {
                return true;
            }
            i = (i + 1) & 511;
        }
        if self.len == 256 {
            return false;
        }
        self.slots[i] = c;
        self.len += 1;
        true
    }

    fn sorted(&self) -> Vec<u32> {
        let mut v: Vec<u32> = self
            .slots
            .iter()
            .copied()
            .filter(|&c| c != OUTSIDE)
            .collect();
        v.sort_unstable();
        v
    }
}

/// What the analysis found, for the UI and the CLI.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Summary {
    pub width: u32,
    pub height: u32,
    /// Frames in the source.
    pub frames: u32,
    /// Frame pairs, the loop's last -> first included (= frames, or 0 for a
    /// single frame).
    pub pairs: u32,
    /// Pairs that get in-betweens.
    pub tweened: u32,
    /// In-between frames added.
    pub added: u32,
    /// Total duration, ms (the player's rules; unchanged by tweening).
    pub duration_ms: u64,
}

impl Summary {
    /// Tweened pairs as a whole percentage of the pairs, rounded down (so a
    /// file shown at 10% really is at the threshold).
    pub fn percent(&self) -> u32 {
        if self.pairs == 0 {
            0
        } else {
            (100 * self.tweened as u64 / self.pairs as u64) as u32
        }
    }

    /// Worth offering: at least [`OFFER_PERCENT`] of the pairs move in
    /// steps of 2 px or more that the tweener takes.
    pub fn offered(&self) -> bool {
        self.pairs > 0 && 100 * self.tweened as u64 >= OFFER_PERCENT as u64 * self.pairs as u64
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Analysing,
    Analysed,
    Tweening,
    Tweened,
}

/// A file being smoothed, in steps: [`analyse`](Self::analyse) until it
/// says done, then [`tween`](Self::tween) until it says done, then
/// [`tweened_gif`](Self::tweened_gif) (a plain GIF of the tweened frames —
/// what the preview plays) or [`prepare`](Self::prepare) (the copy).
/// Deterministic: the same bytes always give the same frames and copy.
pub struct Smooth {
    bytes: Rc<[u8]>,
    frames: Option<Frames>,
    phase: Phase,
    width: usize,
    height: usize,
    repeat: gif::Repeat,
    first: Vec<u32>,
    prev: Vec<u32>,
    cur: Vec<u32>,
    raw: Vec<u16>,
    pairs: Vec<Pair>,
    /// Every colour of the source (`None` past 256: the tweened GIF then
    /// gives each frame its own palette).
    colours: Option<ColourSet>,
    // The tweened GIF being written, and its global palette (sorted).
    out: Vec<u8>,
    palette: Vec<u32>,
    lzw: Option<LzwEncoder>,
    emitted: u32,
    scratch: Vec<u32>,
}

impl Smooth {
    pub fn new(bytes: impl Into<Rc<[u8]>>) -> io::Result<Self> {
        let bytes = bytes.into();
        let frames = Frames::open(&bytes)?;
        let (width, height) = frames.size();
        Ok(Smooth {
            bytes,
            frames: Some(frames),
            phase: Phase::Analysing,
            width,
            height,
            repeat: gif::Repeat::Finite(0),
            first: Vec::new(),
            prev: Vec::new(),
            cur: Vec::new(),
            raw: Vec::new(),
            pairs: Vec::new(),
            colours: Some(ColourSet::new()),
            out: Vec::new(),
            palette: Vec::new(),
            lzw: None,
            emitted: 0,
            scratch: Vec::new(),
        })
    }

    fn img<'a>(&self, px: &'a [u32]) -> Img<'a> {
        Img {
            px,
            w: self.width,
            h: self.height,
        }
    }

    /// Classify one pair and apply the gates that do not depend on the
    /// whole file (the hold rule does; it is applied at the end). The
    /// consistency check runs only where some split of the delay exists.
    fn judge(&self, a: &[u32], b: &[u32], delay_ms: u32) -> Pair {
        let (a, b) = (self.img(a), self.img(b));
        let mut p = classify(&a, &b);
        p.delay_ms = delay_ms;
        p.gate = matches!(p.class, Class::Translation | Class::Regional)
            && p.fit >= TWEEN_FIT
            && n_inbetweens(p.shift, delay_ms, u32::MAX) >= 1
            && inconsistency(&a, &b, &p) <= INCONS_MAX;
        p
    }

    /// Decode and classify up to `max_frames` more frames. `Ok(true)` once
    /// the whole file is analysed ([`summary`](Self::summary) is then
    /// final).
    pub fn analyse(&mut self, max_frames: u32) -> io::Result<bool> {
        if self.phase != Phase::Analysing {
            return Ok(true);
        }
        for _ in 0..max_frames.max(1) {
            let frames = self
                .frames
                .as_mut()
                .ok_or_else(|| invalid("decoder lost"))?;
            let mut cur = std::mem::take(&mut self.cur);
            let Some(raw) = frames.next(&mut cur)? else {
                self.cur = cur;
                self.finish_analysis()?;
                return Ok(true);
            };
            let repeat = frames.repeat();
            self.see_colours(&cur);
            let i = self.raw.len();
            self.raw.push(raw);
            if i == 0 {
                self.first = cur.clone();
                self.repeat = repeat;
            } else {
                let p = self.judge(&self.prev, &cur, player_delay_ms(self.raw[i - 1]));
                self.pairs.push(p);
            }
            self.cur = std::mem::replace(&mut self.prev, cur);
        }
        Ok(false)
    }

    /// Record a frame's colours (given up past 256).
    fn see_colours(&mut self, px: &[u32]) {
        let Some(set) = self.colours.as_mut() else {
            return;
        };
        let mut last = OUTSIDE;
        for &c in px {
            if c != last {
                last = c;
                if !set.insert(c) {
                    self.colours = None;
                    return;
                }
            }
        }
    }

    fn finish_analysis(&mut self) -> io::Result<()> {
        let n = self.raw.len();
        if n == 0 {
            return Err(invalid("GIF has no frames"));
        }
        if n >= 2 {
            let last = std::mem::take(&mut self.prev);
            let p = self.judge(&last, &self.first, player_delay_ms(self.raw[n - 1]));
            self.pairs.push(p);
            self.prev = last;
        }
        let delays: Vec<u32> = self.raw.iter().map(|&r| player_delay_ms(r)).collect();
        let hold = hold_limit(&delays);
        for p in &mut self.pairs {
            p.k = if p.gate {
                n_inbetweens(p.shift, p.delay_ms, hold)
            } else {
                0
            };
        }
        self.frames = None;
        self.phase = Phase::Analysed;
        Ok(())
    }

    pub fn analysed(&self) -> bool {
        !matches!(self.phase, Phase::Analysing)
    }

    /// The per-pair decisions (pair `i` is frame `i` -> `i + 1`, the last
    /// wraps), once analysed.
    pub fn pairs(&self) -> &[Pair] {
        &self.pairs
    }

    /// After the analysis: the GIF has at most 256 colours, so a smoothed
    /// copy can be made (exact). Past 256 there is no copy of any kind and
    /// the GIF plays as it is, so Smooth motion is not offered.
    pub fn fits_palette(&self) -> bool {
        self.colours.is_some()
    }

    pub fn summary(&self) -> Summary {
        let tweened = self.pairs.iter().filter(|p| p.k > 0).count() as u32;
        let added = self.pairs.iter().map(|p| p.k).sum();
        Summary {
            width: self.width as u32,
            height: self.height as u32,
            frames: self.raw.len() as u32,
            pairs: self.pairs.len() as u32,
            tweened,
            added,
            duration_ms: self.raw.iter().map(|&r| player_delay_ms(r) as u64).sum(),
        }
    }

    // --- the tweened GIF --------------------------------------------------

    fn begin_gif(&mut self) -> io::Result<()> {
        let (w, h) = (self.width, self.height);
        if w > u16::MAX as usize || h > u16::MAX as usize {
            return Err(invalid("GIF screen too large"));
        }
        let mut head = [0u8; 13];
        head[..6].copy_from_slice(b"GIF89a");
        head[6..8].copy_from_slice(&(w as u16).to_le_bytes());
        head[8..10].copy_from_slice(&(h as u16).to_le_bytes());
        // Every colour of an in-between is a colour of the source, so one
        // global table holds them when the source's does.
        if let Some(set) = &self.colours {
            // Sorted: the order is the palette's, and deterministic.
            let pal = set.sorted();
            let (table, bits) = table_size(pal.len());
            head[10] = 0x80 | (bits - 1);
            self.out.extend_from_slice(&head);
            for &c in &pal {
                self.out.extend_from_slice(&c.to_be_bytes()[1..]);
            }
            self.out.resize(self.out.len() + 3 * (table - pal.len()), 0);
            self.palette = pal;
        } else {
            self.out.extend_from_slice(&head);
        }
        if self.repeat != gif::Repeat::Finite(0) {
            let n = match self.repeat {
                gif::Repeat::Finite(n) => n,
                gif::Repeat::Infinite => 0,
            }
            .to_le_bytes();
            self.out.extend_from_slice(&[0x21, 0xFF, 11]);
            self.out.extend_from_slice(b"NETSCAPE2.0");
            self.out.extend_from_slice(&[3, 1, n[0], n[1], 0]);
        }
        self.lzw = Some(LzwEncoder::new(MAX_CAP));
        Ok(())
    }

    /// One whole, opaque frame of the tweened GIF.
    fn emit(&mut self, px: &[u32], raw_delay: u16) -> io::Result<()> {
        let local = match &self.colours {
            Some(_) => None::<Vec<u32>>,
            None => {
                let mut pal = px.to_vec();
                pal.sort_unstable();
                pal.dedup();
                if pal.len() > 256 {
                    return Err(invalid(
                        "a frame has more than 256 colours: this GIF cannot be smoothed",
                    ));
                }
                Some(pal)
            }
        };
        let pal: &[u32] = local.as_deref().unwrap_or(&self.palette);
        let (table, bits) = table_size(pal.len());
        let d = raw_delay.to_le_bytes();
        let out = &mut self.out;
        out.extend_from_slice(&[0x21, 0xF9, 4, 0, d[0], d[1], 0, 0]);
        let mut desc = [0u8; 10];
        desc[0] = 0x2C;
        desc[5..7].copy_from_slice(&(self.width as u16).to_le_bytes());
        desc[7..9].copy_from_slice(&(self.height as u16).to_le_bytes());
        if local.is_some() {
            desc[9] = 0x80 | (bits - 1);
            out.extend_from_slice(&desc);
            for &c in pal {
                out.extend_from_slice(&c.to_be_bytes()[1..]);
            }
            out.resize(out.len() + 3 * (table - pal.len()), 0);
        } else {
            out.extend_from_slice(&desc);
        }
        out.push(bits);
        let lzw = self.lzw.as_mut().ok_or_else(|| invalid("GIF not begun"))?;
        let mut blocks = SubBlocks::new(out);
        lzw.begin(bits, &mut blocks)?;
        let mut last = (u32::MAX, 0u8);
        for &c in px {
            if c != last.0 {
                let q = pal
                    .binary_search(&c)
                    .map_err(|_| invalid("a colour the analysis did not see"))?;
                last = (c, q as u8);
            }
            lzw.push(last.1, &mut blocks)?;
        }
        lzw.end(&mut blocks)?;
        blocks.finish()?;
        self.emitted += 1;
        Ok(())
    }

    /// Frame `i` (`px`), then — once frame `i + 1` is known (`next`) — the
    /// in-betweens of pair `i`.
    fn emit_pair(&mut self, i: usize, a: &[u32], b: &[u32]) -> io::Result<()> {
        let p = self.pairs[i];
        if p.k == 0 {
            return Ok(());
        }
        let parts = split_delay(p.delay_ms, p.k).ok_or_else(|| invalid("delay split"))?;
        let mut buf = std::mem::take(&mut self.scratch);
        for t in 1..=p.k {
            let f = t as f64 / (p.k + 1) as f64;
            inbetween(&self.img(a), &self.img(b), &p, f, &mut buf);
            self.emit(&buf, (parts[t as usize] / 10) as u16)?;
        }
        self.scratch = buf;
        Ok(())
    }

    /// Frame `i`'s own delay in the tweened GIF: the source's raw value, or
    /// the first part of its split.
    fn head_delay(&self, i: usize) -> u16 {
        match self.pairs.get(i) {
            Some(p) if p.k > 0 => {
                split_delay(p.delay_ms, p.k).map_or(self.raw[i], |v| (v[0] / 10) as u16)
            }
            _ => self.raw[i],
        }
    }

    /// Write up to `max_frames` more source frames (and their in-betweens)
    /// of the tweened GIF. `Ok(true)` once it is complete.
    pub fn tween(&mut self, max_frames: u32) -> io::Result<bool> {
        match self.phase {
            Phase::Analysing => return Err(invalid("not analysed yet")),
            Phase::Tweened => return Ok(true),
            Phase::Analysed => {
                self.frames = Some(Frames::open(&self.bytes)?);
                self.out.clear();
                self.emitted = 0;
                self.begin_gif()?;
                self.phase = Phase::Tweening;
            }
            Phase::Tweening => {}
        }
        let n = self.raw.len();
        for _ in 0..max_frames.max(1) {
            let frames = self
                .frames
                .as_mut()
                .ok_or_else(|| invalid("decoder lost"))?;
            let mut cur = std::mem::take(&mut self.cur);
            let i = frames.delivered as usize;
            let Some(_) = frames.next(&mut cur)? else {
                if i != n {
                    return Err(invalid("frame count differs between passes"));
                }
                self.cur = cur;
                // The loop's last -> first pair.
                if n >= 2 {
                    let (last, first) = (
                        std::mem::take(&mut self.prev),
                        std::mem::take(&mut self.first),
                    );
                    self.emit_pair(n - 1, &last, &first)?;
                    self.prev = last;
                    self.first = first;
                }
                self.out.push(0x3B);
                self.frames = None;
                self.lzw = None;
                self.phase = Phase::Tweened;
                return Ok(true);
            };
            if i >= n {
                return Err(invalid("frame count differs between passes"));
            }
            if i > 0 {
                let prev = std::mem::take(&mut self.prev);
                self.emit_pair(i - 1, &prev, &cur)?;
                self.prev = prev;
            }
            let d = self.head_delay(i);
            self.emit(&cur, d)?;
            self.cur = std::mem::replace(&mut self.prev, cur);
        }
        Ok(false)
    }

    pub fn tweened(&self) -> bool {
        self.phase == Phase::Tweened
    }

    /// The tweened frames as a plain GIF (every frame whole, 12-bit LZW):
    /// playable as it is (the preview), and what [`prepare`](Self::prepare)
    /// recodes. Empty until [`tween`](Self::tween) is done.
    pub fn tweened_gif(&self) -> &[u8] {
        if self.tweened() {
            &self.out
        } else {
            &[]
        }
    }

    /// Frames in the tweened GIF.
    pub fn tweened_frames(&self) -> u32 {
        self.emitted
    }

    /// The copy: the tweened GIF through [`gifslim::recode_gif`] with
    /// `opts`, then the `OPENDMD1TWN` marker.
    pub fn prepare(&self, opts: &RecodeOptions) -> io::Result<(Vec<u8>, RecodeStats)> {
        if !self.tweened() {
            return Err(invalid("not tweened yet"));
        }
        let mut copy = Vec::with_capacity(self.out.len() / 2);
        let stats = gifslim::recode_gif(&self.out[..], &mut copy, opts)?;
        let s = self.summary();
        insert_marker(&mut copy, s.tweened, s.added)?;
        Ok((copy, stats))
    }
}

/// A colour table's padded size and its code size (≥ 2 bits, as the
/// recoder writes it).
fn table_size(entries: usize) -> (usize, u8) {
    let table = entries.max(4).next_power_of_two();
    (table, table.trailing_zeros() as u8)
}

/// The whole pipeline: analyse, tween, recode, mark. Returns the copy's
/// stats and the analysis. A copy is marked even when nothing was tweened
/// (so a browser checking the list sees it was made smooth).
pub fn smooth_gif(
    gif: &[u8],
    mut out: impl Write,
    opts: &RecodeOptions,
) -> io::Result<(RecodeStats, Summary)> {
    let mut s = Smooth::new(gif)?;
    while !s.analyse(u32::MAX)? {}
    while !s.tween(u32::MAX)? {}
    let (copy, mut stats) = s.prepare(opts)?;
    out.write_all(&copy)?;
    stats.out_bytes = copy.len() as u64;
    stats.gif_bytes = gif.len() as u64;
    Ok((stats, s.summary()))
}

// ---------------------------------------------------------------------------
// The list of smoothed files
// ---------------------------------------------------------------------------

/// The card's list of GIFs whose copy is smoothed: one card-absolute GIF
/// path a line (`/gifs/Other/X.gif`), UTF-8, LF. Beside the playlists and
/// outside `cache/`, so wiping the cache keeps the choice; hidden (a dot
/// name, like `/sets/.ctx`) and without `.txt` or `.meta`, so no one takes
/// it for a playlist or a playlist's overrides.
pub const LIST_PATH: &str = "/sets/.smooth";

/// The list's paths: every non-empty line, trimmed (a CRLF file reads the
/// same). Duplicates are kept; compare them case-insensitively (FAT does).
pub fn parse_list(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect()
}

/// The list file for `paths`: each on its own LF-terminated line.
pub fn render_list<S: AsRef<str>>(paths: &[S]) -> String {
    paths.iter().map(|p| format!("{}\n", p.as_ref())).collect()
}

// ---------------------------------------------------------------------------
// The marker
// ---------------------------------------------------------------------------

/// What a smoothed copy's marker says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Marker {
    /// Pairs tweened (saturating at `u16::MAX`).
    pub pairs: u16,
    /// Frames added (saturating).
    pub added: u16,
}

/// The marker block: `21 FF 0B "OPENDMD1TWN" 05 01 <pairs u16 LE> <added
/// u16 LE> 00` — 21 bytes.
pub fn marker_block(pairs: u32, added: u32) -> [u8; 21] {
    let mut b = [0u8; 21];
    b[..3].copy_from_slice(&[0x21, 0xFF, 11]);
    b[3..14].copy_from_slice(MARKER_APP_ID);
    b[14] = 5;
    b[15] = MARKER_VERSION;
    b[16..18].copy_from_slice(&(pairs.min(u16::MAX as u32) as u16).to_le_bytes());
    b[18..20].copy_from_slice(&(added.min(u16::MAX as u32) as u16).to_le_bytes());
    b
}

/// Where a GIF's extensions start: after the header and the global table.
fn after_palette(gif: &[u8]) -> Option<usize> {
    if gif.len() < 13 || !gif.starts_with(b"GIF8") {
        return None;
    }
    let flags = gif[10];
    let table = if flags & 0x80 != 0 {
        3usize << ((flags & 7) + 1)
    } else {
        0
    };
    Some(13 + table)
}

/// Put the marker right after the `OPENDMD1LZW` hint (or, with no hint,
/// right after the global palette).
pub fn insert_marker(copy: &mut Vec<u8>, pairs: u32, added: u32) -> io::Result<()> {
    let mut at = after_palette(copy).ok_or_else(|| invalid("not a GIF"))?;
    if copy.len() >= at + 19
        && copy[at..at + 3] == [0x21, 0xFF, 11]
        && copy[at + 3..at + 14].eq_ignore_ascii_case(HINT_APP_ID)
    {
        at += 19;
    }
    if copy.len() < at {
        return Err(invalid("GIF is truncated"));
    }
    copy.splice(at..at, marker_block(pairs, added));
    Ok(())
}

/// The marker, if the file carries one among the extensions before its
/// first frame. Needs only the file's first bytes (the header, the palette
/// and those blocks: ≤ ~830 B for a copy); `None` if they are not all
/// there.
pub fn marker(gif: &[u8]) -> Option<Marker> {
    let mut at = after_palette(gif)?;
    loop {
        match *gif.get(at)? {
            0x21 => {
                let label = *gif.get(at + 1)?;
                at += 2;
                if label == 0xFF && gif.get(at) == Some(&11) {
                    let id = gif.get(at + 1..at + 12)?;
                    at += 12;
                    if id.eq_ignore_ascii_case(MARKER_APP_ID) {
                        let len = *gif.get(at)? as usize;
                        let d = gif.get(at + 1..at + 1 + len)?;
                        if len >= 5 && d[0] == MARKER_VERSION {
                            return Some(Marker {
                                pairs: u16::from_le_bytes([d[1], d[2]]),
                                added: u16::from_le_bytes([d[3], d[4]]),
                            });
                        }
                    }
                }
                // Skip the (rest of the) sub-blocks.
                loop {
                    let len = *gif.get(at)? as usize;
                    at += 1 + len;
                    if len == 0 {
                        break;
                    }
                }
            }
            _ => return None,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gifslim::PREPARED;
    use std::borrow::Cow;
    use std::time::Duration;

    const W: usize = 128;
    const H: usize = 32;

    /// A GIF from frames of palette indices (whole frames, global palette).
    fn gif_of(frames: &[Vec<u8>], delays: &[u16], pal: &[[u8; 3]]) -> Vec<u8> {
        let flat: Vec<u8> = pal.iter().flatten().copied().collect();
        let mut out = Vec::new();
        {
            let mut e = gif::Encoder::new(&mut out, W as u16, H as u16, &flat).unwrap();
            e.set_repeat(gif::Repeat::Infinite).unwrap();
            for (f, &d) in frames.iter().zip(delays) {
                e.write_frame(&gif::Frame {
                    width: W as u16,
                    height: H as u16,
                    delay: d,
                    buffer: Cow::Borrowed(f),
                    ..Default::default()
                })
                .unwrap();
            }
        }
        out
    }

    /// A textured picture wider than the screen, so a scroll brings new
    /// columns in; 16 colours, irregular so no small shift fits by chance.
    fn texture(x: i64, y: i64) -> u8 {
        let v = (x * 7 + y * 13 + (x * x) % 11 + (x * y) % 5) as u64;
        (v.wrapping_mul(2654435761) >> 7 & 15) as u8
    }

    fn palette16() -> Vec<[u8; 3]> {
        (0..16u8)
            .map(|i| [i * 16, 255 - i * 13, i.wrapping_mul(37)])
            .collect()
    }

    fn scroll(nframes: usize, step: i64, delay: u16) -> Vec<u8> {
        let frames: Vec<Vec<u8>> = (0..nframes as i64)
            .map(|f| {
                (0..H as i64)
                    .flat_map(|y| (0..W as i64).map(move |x| texture(x + f * step, y)))
                    .collect()
            })
            .collect();
        gif_of(&frames, &vec![delay; nframes], &palette16())
    }

    /// Every frame and delay (as the player plays them) of a GIF.
    fn play(gif: &[u8]) -> Vec<(Vec<u8>, Duration)> {
        let mut g = GifSource::new(gif).unwrap();
        let mut c = Canvas::new();
        let mut v = Vec::new();
        while let Some(d) = g.next_frame(&mut c).unwrap() {
            v.push((c.as_bytes().to_vec(), d));
        }
        v
    }

    fn analysed(gif: &[u8]) -> Smooth {
        let mut s = Smooth::new(gif.to_vec()).unwrap();
        while !s.analyse(7).unwrap() {}
        s
    }

    fn tweened(gif: &[u8]) -> Smooth {
        let mut s = analysed(gif);
        while !s.tween(5).unwrap() {}
        s
    }

    #[test]
    fn shift_order_matches_the_experiment() {
        // The experiment's list: generated dy-outer, dx-inner, then a
        // stable sort by (Chebyshev, Manhattan).
        let mut want = Vec::new();
        for dy in -RADIUS..=RADIUS {
            for dx in -RADIUS..=RADIUS {
                if (dx, dy) != (0, 0) {
                    want.push((dx as i8, dy as i8));
                }
            }
        }
        want.sort_by_key(|&(dx, dy)| (dx.abs().max(dy.abs()), dx.abs() + dy.abs()));
        assert_eq!(SHIFTS.to_vec(), want);
        assert_eq!(&SHIFTS[..4], &[(0, -1), (-1, 0), (1, 0), (0, 1)]);
    }

    #[test]
    fn delays_split_on_the_ten_ms_grid() {
        assert_eq!(split_delay(50, 1), Some(vec![30, 20]));
        assert_eq!(split_delay(100, 2), Some(vec![40, 30, 30]));
        assert_eq!(split_delay(30, 1), None);
        assert_eq!(split_delay(40, 1), Some(vec![20, 20]));
        assert_eq!(split_delay(45, 1), None);
        // 4 px in 50 ms: 3 in-betweens would need 80 ms, so 1.
        assert_eq!(n_inbetweens((4, 0), 50, 200), 1);
        assert_eq!(n_inbetweens((4, 0), 80, 200), 3);
        assert_eq!(n_inbetweens((2, 0), 300, 200), 0);
        assert_eq!(n_inbetweens((1, 0), 100, 200), 0);
        assert_eq!(hold_limit(&[50, 50, 50, 1000]), 200);
        assert_eq!(hold_limit(&[150, 150, 170, 300]), 320);
        assert_eq!(hold_limit(&[150, 155]), 304);
        assert_eq!(py_round(0.5), 0);
        assert_eq!(py_round(1.5), 2);
        assert_eq!(py_round(-0.5), 0);
    }

    #[test]
    fn a_two_px_scroll_gets_exact_inbetweens_and_30_20_splits() {
        let n = 12;
        let gif = scroll(n, 2, 5);
        let s = tweened(&gif);
        let sum = s.summary();
        // Every pair but the wrap (a jump back) is a 2 px translation.
        let pairs = s.pairs();
        for p in &pairs[..n - 1] {
            assert_eq!(
                (p.class, p.shift, p.k),
                (Class::Translation, (-2, 0), 1),
                "{p:?}"
            );
        }
        assert_eq!(pairs[n - 1].k, 0, "the wrap is not a 2 px step");
        assert_eq!((sum.tweened, sum.added), (n as u32 - 1, n as u32 - 1));
        assert!(sum.offered());
        let out = play(s.tweened_gif());
        assert_eq!(out.len(), 2 * n - 1);
        // The in-between of frames f and f+1 is exactly the picture at
        // f*2+1: the true intermediate scroll position.
        let one = scroll(2 * n, 1, 5);
        let truth = play(&one);
        for (j, (px, d)) in out.iter().enumerate() {
            assert_eq!(px, &truth[j].0, "frame {j}");
            let want = if j == out.len() - 1 {
                50
            } else if j % 2 == 0 {
                30
            } else {
                20
            };
            assert_eq!(*d, Duration::from_millis(want), "frame {j}");
        }
        let total: Duration = out.iter().map(|f| f.1).sum();
        assert_eq!(total, Duration::from_millis(50 * n as u64));
    }

    #[test]
    fn a_cut_is_untouched() {
        let pal = palette16();
        let a: Vec<u8> = (0..W * H)
            .map(|p| texture(p as i64 % 128, p as i64 / 128))
            .collect();
        let b: Vec<u8> = (0..W * H)
            .map(|p| (texture(p as i64 / 128, p as i64 % 128) + 5) % 16)
            .collect();
        let gif = gif_of(&[a, b], &[10, 10], &pal);
        let s = tweened(&gif);
        assert!(s.pairs().iter().all(|p| p.k == 0));
        assert_eq!(s.pairs()[0].class, Class::Cut);
        assert_eq!(s.summary().added, 0);
        assert_eq!(play(s.tweened_gif()), play(&gif));
    }

    #[test]
    fn a_hold_is_protected() {
        // A 2 px scroll at 50 ms with one 1 s pause: that pair is a hold.
        let mut gif_frames = Vec::new();
        let mut delays = Vec::new();
        for f in 0..10i64 {
            gif_frames.push(
                (0..H as i64)
                    .flat_map(|y| (0..W as i64).map(move |x| texture(x + f * 2, y)))
                    .collect::<Vec<u8>>(),
            );
            delays.push(if f == 4 { 100 } else { 5 });
        }
        let gif = gif_of(&gif_frames, &delays, &palette16());
        let s = tweened(&gif);
        let p = s.pairs();
        assert_eq!(p[4].class, Class::Translation);
        assert!(p[4].gate);
        assert_eq!(p[4].k, 0, "1000 ms > max(2 x 50, 200): a pause");
        assert_eq!(p[3].k, 1);
        let before: Duration = play(&gif).iter().map(|f| f.1).sum();
        let after: Duration = play(s.tweened_gif()).iter().map(|f| f.1).sum();
        assert_eq!(before, after, "total duration preserved");
    }

    #[test]
    fn the_consistency_gate_rejects_a_row_by_row_wipe() {
        // Bomb Jack's text wipe: a block of text on black erased two rows
        // at a time from the bottom. A (0, -2) shift explains every changed
        // pixel (the black below "arrives") and 90% of the frame, but no
        // in-between of it is a real picture. The Python reference scores
        // these three pairs 1.5625, 1.4375 and 1.0.
        let pal = [[0, 0, 0], [200, 180, 40], [20, 90, 220]];
        let frames: Vec<Vec<u8>> = (0..4i64)
            .map(|f| {
                (0..H as i64)
                    .flat_map(|y| {
                        (0..W as i64).map(move |x| {
                            if (8..16 - 2 * f).contains(&y) && (40..80).contains(&x) {
                                1 + texture(x, y) % 2
                            } else {
                                0
                            }
                        })
                    })
                    .collect()
            })
            .collect();
        let gif = gif_of(&frames, &[5; 4], &pal);
        let s = analysed(&gif);
        let want = [1.5625, 1.4375, 1.0];
        for (i, p) in s.pairs()[..3].iter().enumerate() {
            assert_eq!(
                (p.class, p.shift, p.fit),
                (Class::Translation, (0, -2), 1.0),
                "{p:?}"
            );
            let codes = |f: &Vec<u8>| -> Vec<u32> {
                f.iter()
                    .map(|&q| {
                        let c = pal[q as usize];
                        u32::from_be_bytes([0, c[0], c[1], c[2]])
                    })
                    .collect()
            };
            let (a, b) = (codes(&frames[i]), codes(&frames[i + 1]));
            let inc = inconsistency(&Img { px: &a, w: W, h: H }, &Img { px: &b, w: W, h: H }, p);
            assert_eq!(inc, want[i]);
            assert!(!p.gate && p.k == 0, "the gate must reject it: {p:?}");
        }
    }

    #[test]
    fn past_256_colours_there_is_no_smoothed_copy_either() {
        // A 2 px scroll whose 20 frames each have their own 16 colours (320
        // in all): the tweened frames are exact, but their copy could only
        // be a lossy one, so there is none; the GIF plays as it is.
        let mut gif = Vec::new();
        {
            let mut e = gif::Encoder::new(&mut gif, W as u16, H as u16, &[]).unwrap();
            for f in 0..20i64 {
                let pal: Vec<u8> = (0..16u8)
                    .flat_map(|i| [i * 16, f as u8 * 12, 255 - i * 13])
                    .collect();
                let px: Vec<u8> = (0..H as i64)
                    .flat_map(|y| (0..W as i64).map(move |x| texture(x + f * 2, y)))
                    .collect();
                e.write_frame(&gif::Frame {
                    width: W as u16,
                    height: H as u16,
                    delay: 5,
                    palette: Some(pal),
                    buffer: Cow::Owned(px),
                    ..Default::default()
                })
                .unwrap();
            }
        }
        assert!(!analysed(&gif).fits_palette());
        assert!(analysed(&scroll(8, 2, 5)).fits_palette());
        let mut copy = Vec::new();
        let e = smooth_gif(&gif, &mut copy, &PREPARED).unwrap_err();
        assert!(gifslim::is_too_many_colours(&e), "{e}");
        assert!(copy.is_empty());
    }

    #[test]
    fn the_copy_is_the_recoded_tween_and_carries_the_marker() {
        let gif = scroll(8, 2, 5);
        let mut copy = Vec::new();
        let (stats, sum) = smooth_gif(&gif, &mut copy, &PREPARED).unwrap();
        assert_eq!(stats.frames, 15);
        assert_eq!(
            marker(&copy),
            Some(Marker {
                pairs: sum.tweened as u16,
                added: sum.added as u16
            })
        );
        // Right after the 19-byte hint.
        let at = after_palette(&copy).unwrap();
        assert_eq!(&copy[at + 3..at + 14], HINT_APP_ID);
        assert_eq!(&copy[at + 22..at + 33], MARKER_APP_ID);
        // Exactly recode(tweened GIF) plus the 21 bytes.
        let s = tweened(&gif);
        let mut plain = Vec::new();
        gifslim::recode_gif(s.tweened_gif(), &mut plain, &PREPARED).unwrap();
        let mut marked = plain.clone();
        insert_marker(&mut marked, sum.tweened, sum.added).unwrap();
        assert_eq!(copy, marked);
        assert_eq!(copy.len(), plain.len() + 21);
        assert_eq!(marker(&plain), None);
        // Deterministic.
        let mut again = Vec::new();
        smooth_gif(&gif, &mut again, &PREPARED).unwrap();
        assert_eq!(copy, again);
        // A marked copy plays identically through gifslim and the gif
        // crate, and as the unmarked one does.
        let want = play(&plain);
        assert_eq!(play(&copy), want);
        let mut slim = GifSlim::new(&copy[..]).unwrap();
        assert_eq!(slim.cap(), 9);
        let mut c = Canvas::new();
        let mut got = Vec::new();
        while let Some(d) = slim.next_frame(&mut c).unwrap() {
            got.push((c.as_bytes().to_vec(), d));
        }
        assert_eq!(got, want);
        // The marker is found from the first bytes alone.
        assert!(marker(&copy[..at + 60]).is_some());
        assert_eq!(marker(&copy[..at + 25]), None);
    }

    #[test]
    fn a_regional_sprite_is_tweened_in_layers() {
        // A 6x6 sprite moving 3 px right per frame over a static checkered
        // background (a column never equals the one 3 px on, so no stray
        // background pixel "moves with" the sprite: the in-betweens are
        // exact; on real art such coincidences are the small glitches the
        // experiment saw on Honeycam).
        let pal = palette16();
        let frames: Vec<Vec<u8>> = (0..6i64)
            .map(|f| {
                (0..H as i64)
                    .flat_map(|y| {
                        (0..W as i64).map(move |x| {
                            let sx = 10 + 3 * f;
                            if (sx..sx + 6).contains(&x) && (12..18).contains(&y) {
                                15 - ((x - sx + y) % 3) as u8
                            } else {
                                ((x % 2) * 2 + y % 2) as u8
                            }
                        })
                    })
                    .collect()
            })
            .collect();
        let gif = gif_of(&frames, &[8; 6], &pal);
        let s = tweened(&gif);
        let p = s.pairs()[0];
        assert_eq!((p.class, p.shift, p.k), (Class::Regional, (3, 0), 2));
        let out = play(s.tweened_gif());
        // The in-betweens show the sprite 1 and 2 px along, background
        // intact: the frames of a 1 px-per-step version.
        let step: Vec<Vec<u8>> = (0..3i64)
            .map(|f| {
                (0..H as i64)
                    .flat_map(|y| {
                        (0..W as i64).map(move |x| {
                            let sx = 10 + f;
                            if (sx..sx + 6).contains(&x) && (12..18).contains(&y) {
                                15 - ((x - sx + y) % 3) as u8
                            } else {
                                ((x % 2) * 2 + y % 2) as u8
                            }
                        })
                    })
                    .collect()
            })
            .collect();
        let truth = play(&gif_of(&step, &[8; 3], &pal));
        assert_eq!(out[1].0, truth[1].0);
        assert_eq!(out[2].0, truth[2].0);
        assert_eq!(
            out[..3].iter().map(|f| f.1.as_millis()).collect::<Vec<_>>(),
            vec![30, 30, 20]
        );
    }

    #[test]
    fn a_single_frame_and_a_static_file_are_left_alone() {
        let pal = palette16();
        let a: Vec<u8> = (0..W * H).map(|p| (p % 16) as u8).collect();
        let one = gif_of(std::slice::from_ref(&a), &[0], &pal);
        let s = tweened(&one);
        assert_eq!(s.summary().pairs, 0);
        assert_eq!(play(s.tweened_gif()), play(&one));
        let two = gif_of(&[a.clone(), a], &[3, 7], &pal);
        let s = tweened(&two);
        assert_eq!(s.pairs()[0].class, Class::Static);
        assert!(!s.summary().offered());
        assert_eq!(play(s.tweened_gif()), play(&two));
    }
}

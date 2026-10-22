//! Q16.16 arithmetic: the one definition of every operation, shared by the
//! constant folder, the scalar (per-frame, per-row) evaluator and the lane
//! loops, so a value is the same bits however it was computed — on the
//! ESP32, on the host and in wasm (integer maths only; no float anywhere).
//!
//! **Overflow.** Values are i32 Q16.16: −32,768 .. 32,767.99998. `+ - *
//! /`, negation and `abs` *saturate* (clamp to the ends) rather than wrap,
//! so `t*t` after three minutes is "very big", not a sudden negative; the
//! phase functions (`sin`, `tri`, ...) only read the fractional turn, so
//! they never overflow. Division by zero gives the saturated value of the
//! numerator's sign (0 for 0 / 0); `%` by zero gives 0.

use crate::tables::{Tables, ATAN};

pub const ONE: i32 = 1 << 16;

#[inline(always)]
pub fn sat(v: i64) -> i32 {
    if v > i32::MAX as i64 {
        i32::MAX
    } else if v < i32::MIN as i64 {
        i32::MIN
    } else {
        v as i32
    }
}

#[inline(always)]
pub fn add(a: i32, b: i32) -> i32 {
    a.saturating_add(b)
}

#[inline(always)]
pub fn sub(a: i32, b: i32) -> i32 {
    a.saturating_sub(b)
}

#[inline(always)]
pub fn mul(a: i32, b: i32) -> i32 {
    sat((a as i64 * b as i64) >> 16)
}

#[inline(always)]
pub fn div(a: i32, b: i32) -> i32 {
    if b == 0 {
        return match a {
            0 => 0,
            a if a > 0 => i32::MAX,
            _ => i32::MIN,
        };
    }
    sat(((a as i64) << 16) / b as i64)
}

/// `1 / a`, what a division by a per-frame or per-row value becomes (one
/// division per frame or row, then a multiply per pixel).
#[inline(always)]
pub fn recip(a: i32) -> i32 {
    div(ONE, a)
}

/// Floored modulo (the result takes the divisor's sign, as GLSL's `mod`):
/// both are Q16.16, so it is the raw integers' remainder.
#[inline(always)]
pub fn modulo(a: i32, b: i32) -> i32 {
    if b == 0 {
        return 0;
    }
    let r = a.wrapping_rem(b);
    if r != 0 && ((r < 0) != (b < 0)) {
        r.wrapping_add(b)
    } else {
        r
    }
}

/// `a % m` for a positive power-of-two `m` (raw): `a & (m - 1)`, which is
/// the floored modulo. `b` is the mask.
#[inline(always)]
pub fn mask(a: i32, b: i32) -> i32 {
    a & b
}

#[inline(always)]
fn b2f(b: bool) -> i32 {
    if b {
        ONE
    } else {
        0
    }
}

#[inline(always)]
pub fn lt(a: i32, b: i32) -> i32 {
    b2f(a < b)
}
#[inline(always)]
pub fn le(a: i32, b: i32) -> i32 {
    b2f(a <= b)
}
#[inline(always)]
pub fn gt(a: i32, b: i32) -> i32 {
    b2f(a > b)
}
#[inline(always)]
pub fn ge(a: i32, b: i32) -> i32 {
    b2f(a >= b)
}
#[inline(always)]
pub fn eq(a: i32, b: i32) -> i32 {
    b2f(a == b)
}
#[inline(always)]
pub fn ne(a: i32, b: i32) -> i32 {
    b2f(a != b)
}
#[inline(always)]
pub fn and(a: i32, b: i32) -> i32 {
    b2f(a != 0 && b != 0)
}
#[inline(always)]
pub fn or(a: i32, b: i32) -> i32 {
    b2f(a != 0 || b != 0)
}
#[inline(always)]
pub fn not(a: i32) -> i32 {
    b2f(a == 0)
}
#[inline(always)]
pub fn min(a: i32, b: i32) -> i32 {
    a.min(b)
}
#[inline(always)]
pub fn max(a: i32, b: i32) -> i32 {
    a.max(b)
}
/// GLSL's `step(edge, x)`: 1 where `x >= edge`.
#[inline(always)]
pub fn step(e: i32, x: i32) -> i32 {
    b2f(x >= e)
}
#[inline(always)]
pub fn neg(a: i32) -> i32 {
    a.saturating_neg()
}
#[inline(always)]
pub fn abs(a: i32) -> i32 {
    a.saturating_abs()
}
#[inline(always)]
pub fn floor(a: i32) -> i32 {
    a & !0xffff
}
#[inline(always)]
pub fn frac(a: i32) -> i32 {
    a & 0xffff
}

/// The sine of a phase in turns (the fraction only: 65,536 steps a turn,
/// core's `sin16`), −1..1.
#[inline(always)]
pub fn sin(t: &Tables, a: i32) -> i32 {
    let ph = a as u32 & 0xffff;
    let (i, f) = ((ph >> 8) as usize, (ph & 255) as i32);
    let s0 = t.sin[i] as i32;
    let s1 = t.sin[(i + 1) & 255] as i32;
    (s0 + (((s1 - s0) * f) >> 8)) << 2
}

#[inline(always)]
pub fn cos(t: &Tables, a: i32) -> i32 {
    sin(t, a.wrapping_add(ONE / 4))
}

/// The triangle wave in step with `sin`: 0 at 0, 1 at ¼, −1 at ¾.
#[inline(always)]
pub fn tri(a: i32) -> i32 {
    let p = a & 0xffff;
    if p < 0x4000 {
        p << 2
    } else if p < 0xc000 {
        (2 * ONE) - (p << 2)
    } else {
        (p << 2) - 4 * ONE
    }
}

/// The sawtooth: −1 at 0 rising to 1 at the end of the turn.
#[inline(always)]
pub fn saw(a: i32) -> i32 {
    ((a & 0xffff) << 1) - ONE
}

/// The square wave: 1 for the first half turn, −1 for the second.
#[inline(always)]
pub fn sqr(a: i32) -> i32 {
    if a & 0x8000 == 0 {
        ONE
    } else {
        -ONE
    }
}

/// ⌊√v⌋ of a u32, bit by bit (16 rounds).
#[inline(always)]
fn isqrt32(mut v: u32) -> u32 {
    let mut r = 0u32;
    let mut bit = 1u32 << 30;
    while bit > v {
        bit >>= 2;
    }
    while bit != 0 {
        if v >= r + bit {
            v -= r + bit;
            r = (r >> 1) + bit;
        } else {
            r >>= 1;
        }
        bit >>= 2;
    }
    r
}

/// √ of a Q32 value (u64) as Q16, 16 significant bits: normalised so the
/// top two bits are in the high word, one 32-bit square root, shifted back.
#[inline(always)]
fn sqrt_q32(v: u64) -> i32 {
    if v == 0 {
        return 0;
    }
    let s = v.leading_zeros() & !1;
    let r = isqrt32(((v << s) >> 32) as u32) as u64;
    // √(v) = √(v << s >> 32) · 2^16 / 2^(s/2)
    let sh = 16 - (s / 2) as i32;
    let out = if sh >= 0 { r << sh } else { r >> -sh };
    sat(out as i64)
}

#[inline(always)]
pub fn sqrt(a: i32) -> i32 {
    if a <= 0 {
        return 0;
    }
    // a is Q16; a << 16 is Q32.
    sqrt_q32((a as u64) << 16)
}

/// √(a² + b²).
#[inline(always)]
pub fn len(a: i32, b: i32) -> i32 {
    let (a, b) = (a.unsigned_abs() as u64, b.unsigned_abs() as u64);
    sqrt_q32((a * a).saturating_add(b * b))
}

/// The angle of (x, y) in turns, −½..½ (0 along +x, ¼ along +y): the
/// eighth-turn table, interpolated, folded into the octant.
#[inline(always)]
pub fn atan2(y: i32, x: i32) -> i32 {
    let (ax, ay) = (x.unsigned_abs(), y.unsigned_abs());
    let (mut lo, mut hi) = if ay > ax { (ax, ay) } else { (ay, ax) };
    if hi == 0 {
        return 0;
    }
    let bits = 32 - hi.leading_zeros();
    if bits > 15 {
        lo >>= bits - 15;
        hi >>= bits - 15;
        if hi == 0 {
            return 0;
        }
    }
    let r = (lo << 16) / hi; // 0..=65536
    let (i, f) = ((r >> 8) as usize, (r & 255) as i32);
    let (t0, t1) = (ATAN[i] as i32, ATAN[(i + 1).min(256)] as i32);
    let mut ang = t0 + (((t1 - t0) * f) >> 8);
    if ay > ax {
        ang = ONE / 4 - ang;
    }
    if x < 0 {
        ang = ONE / 2 - ang;
    }
    if y < 0 {
        -ang
    } else {
        ang
    }
}

/// lowbias32: a small integer hash.
#[inline(always)]
pub fn mix32(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^= x >> 16;
    x
}

/// A pseudo-random 0..1 from two values (their exact bits).
#[inline(always)]
pub fn hash(a: i32, b: i32) -> i32 {
    (mix32(a as u32 ^ mix32(b as u32 ^ 0x9e37_79b9)) >> 16) as i32
}

#[inline(always)]
fn lattice(x: i32, y: i32, z: i32) -> i32 {
    let h = (x as u32).wrapping_mul(0x8da6_b343)
        ^ (y as u32).wrapping_mul(0xd816_3841)
        ^ (z as u32).wrapping_mul(0xcb1a_b31f);
    (mix32(h) >> 16) as i32
}

/// The smooth weight 3f² − 2f³ of a fraction 0..1 (Q16).
#[inline(always)]
fn fade(f: i32) -> i32 {
    let f2 = ((f as i64 * f as i64) >> 16) as i32;
    ((f2 as i64 * (3 * ONE - 2 * f) as i64) >> 16) as i32
}

#[inline(always)]
fn lerp_u(a: i32, b: i32, t: i32) -> i32 {
    a + (((b - a) as i64 * t as i64) >> 16) as i32
}

/// 3D value noise, 0..1: a hash at every integer lattice point, blended
/// with the smooth weight. One lattice cell per unit, so `noise(x*8, ...)`
/// has eight cells across the panel.
#[inline(always)]
pub fn noise(x: i32, y: i32, z: i32) -> i32 {
    let (ix, iy, iz) = (x >> 16, y >> 16, z >> 16);
    let (fx, fy, fz) = (fade(x & 0xffff), fade(y & 0xffff), fade(z & 0xffff));
    let (jx, jy, jz) = (ix.wrapping_add(1), iy.wrapping_add(1), iz.wrapping_add(1));
    let x00 = lerp_u(lattice(ix, iy, iz), lattice(jx, iy, iz), fx);
    let x10 = lerp_u(lattice(ix, jy, iz), lattice(jx, jy, iz), fx);
    let x01 = lerp_u(lattice(ix, iy, jz), lattice(jx, iy, jz), fx);
    let x11 = lerp_u(lattice(ix, jy, jz), lattice(jx, jy, jz), fx);
    let y0 = lerp_u(x00, x10, fy);
    let y1 = lerp_u(x01, x11, fy);
    lerp_u(y0, y1, fz)
}

#[inline(always)]
pub fn select(c: i32, a: i32, b: i32) -> i32 {
    if c != 0 {
        a
    } else {
        b
    }
}

#[inline(always)]
pub fn clamp(x: i32, lo: i32, hi: i32) -> i32 {
    x.max(lo).min(hi)
}

/// `a + (b − a)·t`.
#[inline(always)]
pub fn mix(a: i32, b: i32, t: i32) -> i32 {
    add(a, mul(sub(b, a), t))
}

/// `smoothstep(e0, e1, x)` with `inv` = 1 / (e1 − e0) worked out once:
/// t = clamp((x − e0)·inv, 0, 1), t²(3 − 2t).
#[inline(always)]
pub fn smooth(x: i32, e0: i32, inv: i32) -> i32 {
    let t = clamp(mul(sub(x, e0), inv), 0, ONE);
    let t2 = ((t as i64 * t as i64) >> 16) as i32;
    ((t2 as i64 * (3 * ONE - 2 * t) as i64) >> 16) as i32
}

/// A Q16 colour channel 0..1 as 0..255, rounded.
#[inline(always)]
pub fn chan(v: i32) -> u8 {
    let c = clamp(v, 0, ONE);
    ((c * 255 + 0x8000) >> 16) as u8
}

/// HSV (hue in turns, the fraction only; saturation and value 0..1) to
/// 8-bit RGB.
#[inline(always)]
pub fn hsv(h: i32, s: i32, v: i32) -> [u8; 3] {
    let s = clamp(s, 0, ONE);
    let v = clamp(v, 0, ONE);
    let h6 = (h & 0xffff) * 6; // 0..393,210
    let sector = h6 >> 16;
    let f = h6 & 0xffff;
    let p = ((v as i64 * (ONE - s) as i64) >> 16) as i32;
    let q = ((v as i64 * (ONE as i64 - ((s as i64 * f as i64) >> 16))) >> 16) as i32;
    let t = ((v as i64 * (ONE as i64 - ((s as i64 * (ONE - f) as i64) >> 16))) >> 16) as i32;
    let (r, g, b) = match sector {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    };
    [chan(r), chan(g), chan(b)]
}

/// A palette entry for k in 0..1 (clamped): 32 stepped entries.
#[inline(always)]
pub fn pal(p: &[[u8; 3]; 32], k: i32) -> [u8; 3] {
    let i = (clamp(k, 0, ONE) >> 11).min(31) as usize;
    p[i]
}

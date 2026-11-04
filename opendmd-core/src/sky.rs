//! The sky over the panel: the sun's height, the moon's phase, the season
//! and the day of the year, from the clock and an approximate location, for
//! Formulas' built-in inputs (`daylight`, `sun.alt`, `moon.phase`, `season`,
//! `yearday`: `docs/research/psi-inputs.md` §3.1).
//!
//! **Integer arithmetic only**, so the panel (the ESP32's FPU) and the
//! browser (wasm) get the same numbers bit for bit: angles are turns in
//! fixed point (a `u32` is one turn, wrapping), the sine a polynomial, the
//! arc sine a CORDIC. This file has no dependencies: the Formulas wasm
//! module (`opendmd-psi-web`) includes it as it is (`#[path]`), the
//! firmware through `opendmd-core`.
//!
//! **The sun** is the Astronomical Almanac's low-precision position (mean
//! longitude and anomaly, the equation of the centre's two terms, a fixed
//! obliquity, Greenwich mean sidereal time), good to ~0.01° in the sun's
//! place between 1950 and 2050: far below a pixel of any look. Refraction
//! is left out (it lifts the sun ~0.5° at the horizon, a minute or two of
//! a sunset). **The moon's phase** is the time since a known new moon over
//! the mean synodic month (±~0.5 day: enough for a look). **The season**
//! is the sun's ecliptic longitude from the December solstice (half a turn
//! on in the southern hemisphere).

/// A location: latitude and longitude in hundredths of a degree (north
/// and east positive), what Settings keeps (0.01°, ~1 km).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Place {
    pub lat_c: i32,
    pub lon_c: i32,
}

impl Place {
    /// A place from degrees, rounded to 0.01° and held to the globe
    /// (latitude ±90, longitude ±180). `None` for a number that isn't one.
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn from_degrees(lat: f64, lon: f64) -> Option<Place> {
        if !lat.is_finite() || !lon.is_finite() || lat.abs() > 90.0 || lon.abs() > 180.0 {
            return None;
        }
        // (Rounded half away from zero; no `f64::round` dependence beyond
        // what both targets share.)
        let r = |v: f64| (v * 100.0 + if v < 0.0 { -0.5 } else { 0.5 }) as i32;
        Some(Place {
            lat_c: r(lat),
            lon_c: r(lon),
        })
    }
}

/// Unix time of J2000.0 (2000-01-01 12:00 UTC; TT's 64 s left out).
const J2000: i64 = 946_728_000;
const DAY: i64 = 86_400;

/// One turn in the Q48 the sun's linear terms are worked out in.
const Q48: i64 = 1 << 48;

/// `c0 + rate·n` turns for `n` days (and `secs` more) since J2000, as Q32
/// turns: `c0` and `rate` in Q48 turns (the rate a day, its whole turns
/// left out).
#[cfg_attr(target_arch = "xtensa", optimize(size))]
#[inline(never)]
fn lin(c0: i64, rate: i64, days: i64, secs: i64) -> u32 {
    let v = c0
        .wrapping_add(rate.wrapping_mul(days))
        .wrapping_add(rate * secs / DAY);
    ((v & (Q48 - 1)) >> 16) as u32
}

/// A number as Q48 turns from turns (only for the constants below).
const fn q48(turns: f64) -> i64 {
    (turns * Q48 as f64) as i64
}

// The constants (degrees in the comments, turns here).
/// Mean longitude 280.460° + 0.9856474°/day.
const L0: i64 = q48(280.460 / 360.0);
const L1: i64 = q48(0.985_647_4 / 360.0);
/// Mean anomaly 357.528° + 0.9856003°/day.
const G0: i64 = q48(357.528 / 360.0);
const G1: i64 = q48(0.985_600_3 / 360.0);
/// Greenwich mean sidereal time 280.46061837° + 360.98564736629°/day
/// (its whole turn a day left out: `sun` adds it for the day's part).
const S0: i64 = q48(280.460_618_37 / 360.0);
const S1: i64 = q48(0.985_647_366_29 / 360.0);

/// Q30 fixed point (1.0 = 2^30).
const ONE30: i64 = 1 << 30;

/// sin and cos of `a` (Q32 turns), Q30: the quarter turn's polynomial
/// (Taylor to θ⁷ and θ⁸ on ±π/4: within 4e-7).
#[cfg_attr(target_arch = "xtensa", optimize(size))]
#[inline(never)]
pub fn sin_cos(a: u32) -> (i64, i64) {
    // The nearest quarter turn: a + 1/8 turn, its top two bits.
    let b = a.wrapping_add(1 << 29);
    let quarter = b >> 30;
    // The rest, −1/8..1/8 turn, as Q30 radians (2π/2^32 a unit, in Q30:
    // 2π·2^30/2^32 = π/2, 1,686,629,713 / 2^30).
    let r = (b & ((1 << 30) - 1)) as i64 - (1 << 29);
    let th = (r * 1_686_629_713) >> 30;
    // (A product of two Q30 numbers.)
    let m = |a: i64, b: i64| (a * b) >> 30;
    let t2 = m(th, th);
    // sin θ = θ(1 − θ²/6(1 − θ²/20(1 − θ²/42)))
    let s = m(th, ONE30 - m(t2, ONE30 - m(t2, ONE30 - t2 / 42) / 20) / 6);
    // cos θ = 1 − θ²/2(1 − θ²/12(1 − θ²/30(1 − θ²/56)))
    let c = ONE30 - m(t2, ONE30 - m(t2, ONE30 - m(t2, ONE30 - t2 / 56) / 30) / 12) / 2;
    // Each quarter turn: (s, c) → (c, −s).
    match quarter {
        0 => (s, c),
        1 => (c, -s),
        2 => (-s, -c),
        _ => (-c, s),
    }
}

/// atan(2^-i) in Q32 turns, i = 0..: the CORDIC's angles.
const ATANS: [u32; 30] = {
    let mut t = [0u32; 30];
    // atan(2^-i)/2π · 2^32, from the series (exact to the last unit for
    // i ≥ 1; i = 0 is 1/8 turn).
    t[0] = 1 << 29;
    let mut i = 1;
    while i < 30 {
        // atan(x) = x − x³/3 + x⁵/5 − …, x = 2^-i, in f64 (const).
        let x = 1.0 / (1u64 << i) as f64;
        let mut s = 0.0;
        let mut p = x;
        let mut k = 0;
        while k < 12 {
            let term = p / (2 * k + 1) as f64;
            s += if k % 2 == 0 { term } else { -term };
            p *= x * x;
            k += 1;
        }
        t[i] = (s / core::f64::consts::TAU * 4_294_967_296.0 + 0.5) as u32;
        i += 1;
    }
    t
};

/// atan2(y, x) for x ≥ 0 (Q30 or any common scale), Q32 turns as a signed
/// number (−¼..¼): the CORDIC's vectoring.
#[cfg_attr(target_arch = "xtensa", optimize(size))]
#[inline(never)]
fn atan2_right(mut y: i64, mut x: i64) -> i32 {
    let mut a: i64 = 0;
    for (i, &t) in ATANS.iter().enumerate() {
        let (dx, dy) = (y >> i, x >> i);
        if y > 0 {
            x += dx;
            y -= dy;
            a += t as i64;
        } else {
            x -= dx;
            y += dy;
            a -= t as i64;
        }
    }
    a as i32
}

/// The integer square root of `v` (floor).
#[cfg_attr(target_arch = "xtensa", optimize(size))]
#[inline(never)]
fn isqrt(v: u64) -> u64 {
    if v < 2 {
        return v;
    }
    let mut r = 0u64;
    let mut bit = 1u64 << 62;
    let mut n = v;
    while bit > n {
        bit >>= 2;
    }
    while bit != 0 {
        if n >= r + bit {
            n -= r + bit;
            r = (r >> 1) + bit;
        } else {
            r >>= 1;
        }
        bit >>= 2;
    }
    r
}

/// arcsin of `s` (Q30, held to ±1), Q32 turns signed (−¼..¼).
#[cfg_attr(target_arch = "xtensa", optimize(size))]
#[inline(never)]
fn asin(s: i64) -> i32 {
    let s = s.clamp(-ONE30, ONE30);
    let c = isqrt((ONE30 * ONE30 - s * s) as u64) as i64;
    atan2_right(s, c)
}

/// The sun for `unix` (seconds, UTC): its ecliptic longitude λ (Q32 turns)
/// and, at `place`, its altitude (Q32 turns, signed).
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn sun(unix: i64, place: Place) -> (u32, i32) {
    let d = unix - J2000;
    let (days, secs) = (d.div_euclid(DAY), d.rem_euclid(DAY));
    let l = lin(L0, L1, days, secs);
    let g = lin(G0, G1, days, secs);
    // λ = L + 1.915° sin g + 0.020° sin 2g.
    let (sg, _) = sin_cos(g);
    let (s2g, _) = sin_cos(g.wrapping_mul(2));
    let k1 = (1.915 / 360.0 * 4_294_967_296.0) as i64;
    let k2 = (0.020 / 360.0 * 4_294_967_296.0) as i64;
    let lam = l.wrapping_add(((k1 * sg + k2 * s2g) >> 30) as i32 as u32);
    // The obliquity, 23.439°.
    let (se, ce) = sin_cos((23.439 / 360.0 * 4_294_967_296.0) as u32);
    let (sl, cl) = sin_cos(lam);
    // Local sidereal time θ: GMST + the longitude.
    let lon = (place.lon_c as i64 * 4_294_967_296 / 36_000) as i32 as u32;
    // (The sidereal day's whole turn a day, left out of `S1`, is a turn a
    // day on the day's part too.)
    let spin = (secs * 4_294_967_296 / DAY) as u32;
    let th = lin(S0, S1, days, secs).wrapping_add(spin).wrapping_add(lon);
    let (st, ct) = sin_cos(th);
    let lat = (place.lat_c as i64 * 4_294_967_296 / 36_000) as i32 as u32;
    let (sp, cp) = sin_cos(lat);
    // sin(alt) = sin φ sin δ + cos φ cos δ cos H, with sin δ = sin ε sin λ
    // and cos δ cos H = cos θ cos λ + sin θ cos ε sin λ (α never needed).
    let sin_d = (se * sl) >> 30;
    let cdch = (ct * cl + st * ((ce * sl) >> 30)) >> 30;
    let s = ((sp * sin_d) >> 30) + ((cp * cdch) >> 30);
    (lam, asin(s))
}

/// Q16.16 of a Q32 turn count (signed).
fn q16_of(a: i32) -> i32 {
    a >> 16
}

/// `sun.alt`: the sun's altitude in turns, Q16.16 (−0.25..0.25).
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn sun_alt(unix: i64, place: Place) -> i32 {
    q16_of(sun(unix, place).1)
}

/// `daylight` for an altitude (Q16.16 turns): `smoothstep` from −6° to +6°
/// (civil twilight), Q16.16 0..1.
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn daylight(alt: i32) -> i32 {
    // −6° and +6° in Q16.16 turns: ∓65536/60.
    let (lo, span) = (-65_536 / 60, 2 * 65_536 / 60);
    let t = ((alt as i64 - lo as i64) * 65_536 / span as i64).clamp(0, 65_536);
    // t²(3 − 2t)
    ((((t * t) >> 16) * (3 * 65_536 - 2 * t)) >> 16) as i32
}

/// `season` from the sun's ecliptic longitude (Q32 turns): 0 at the
/// December solstice (λ = ¾ turn), ½ at the June one; half a turn on in the
/// southern hemisphere. Q16.16 0..1.
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn season(lam: u32, place: Place) -> i32 {
    let mut s = lam.wrapping_sub(3 << 30);
    if place.lat_c < 0 {
        s = s.wrapping_add(1 << 31);
    }
    (s >> 16) as i32
}

/// A known new moon: 2000-01-06 18:14 UTC.
const NEW_MOON: i64 = 947_182_440;
/// The mean synodic month, ms (29.530588853 days).
const SYNODIC_MS: i64 = 2_551_442_877;

/// `moon.phase` for `unix`: 0 new, ½ full, Q16.16 0..1.
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn moon_phase(unix: i64) -> i32 {
    let ms = (unix - NEW_MOON).wrapping_mul(1000).rem_euclid(SYNODIC_MS);
    (ms * 65_536 / SYNODIC_MS) as i32
}

/// `yearday` for a local time (`unix` plus the UTC offset, seconds): the
/// day of the year, 0 on 1 January (365 on 31 December of a leap year).
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn yearday(local: i64) -> u16 {
    let days = local.div_euclid(DAY);
    // Howard Hinnant's civil_from_days: the year of `days`, then 1 January.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy_mar = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy_mar + 2) / 153;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    // Days from 1 March to the date, then from 1 January.
    let before_march = if leap { 60 } else { 59 };
    if m <= 2 {
        (doy_mar - 306) as u16
    } else {
        (doy_mar + before_march) as u16
    }
}

/// Every sky input at once for `unix` (UTC seconds), the local time's
/// `utc_offset` (seconds) and `place` if one is set: (`daylight`,
/// `sun.alt`, `moon.phase`, `season`, `yearday`), Q16.16; `None` where it
/// can't be known (no place: the sun and the season).
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn inputs(unix: i64, utc_offset: i64, place: Option<Place>) -> Sky {
    inputs_but_yearday(unix, i32::from(yearday(unix + utc_offset)) << 16, place)
}

/// [`inputs`] with the day of the year given (Q16.16; the panel's calendar
/// has it).
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn inputs_but_yearday(unix: i64, yearday: i32, place: Option<Place>) -> Sky {
    let sun = place.map(|p| {
        let (lam, alt) = sun(unix, p);
        (q16_of(alt), season(lam, p))
    });
    Sky {
        daylight: sun.map(|s| daylight(s.0)),
        sun_alt: sun.map(|s| s.0),
        moon_phase: moon_phase(unix),
        season: sun.map(|s| s.1),
        yearday,
    }
}

/// The sky inputs for a moment ([`inputs`]), Q16.16.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sky {
    pub daylight: Option<i32>,
    pub sun_alt: Option<i32>,
    pub moon_phase: i32,
    pub season: Option<i32>,
    pub yearday: i32,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deg(q32: i32) -> f64 {
        q32 as f64 / 4_294_967_296.0 * 360.0
    }

    /// The same algorithm in f64: what the integers must stay near.
    fn reference(unix: i64, lat: f64, lon: f64) -> (f64, f64) {
        let n = (unix - J2000) as f64 / 86_400.0;
        let r = |d: f64| d.to_radians();
        let l = 280.460 + 0.985_647_4 * n;
        let g = r(357.528 + 0.985_600_3 * n);
        let lam = r(l + 1.915 * g.sin() + 0.020 * (2.0 * g).sin());
        let eps = r(23.439);
        let dec = (eps.sin() * lam.sin()).asin();
        let ra = (eps.cos() * lam.sin()).atan2(lam.cos());
        let gmst = r(280.460_618_37 + 360.985_647_366_29 * n);
        let h = gmst + r(lon) - ra;
        let alt = (r(lat).sin() * dec.sin() + r(lat).cos() * dec.cos() * h.cos()).asin();
        (lam.to_degrees().rem_euclid(360.0), alt.to_degrees())
    }

    #[test]
    fn sin_cos_is_within_a_millionth() {
        for k in 0..4096u32 {
            let a = k.wrapping_mul(1_048_573);
            let (s, c) = sin_cos(a);
            let x = a as f64 / 4_294_967_296.0 * core::f64::consts::TAU;
            assert!((s as f64 / ONE30 as f64 - x.sin()).abs() < 1e-6, "sin {a}");
            assert!((c as f64 / ONE30 as f64 - x.cos()).abs() < 1e-6, "cos {a}");
        }
    }

    #[test]
    fn asin_is_within_a_hundredth_of_a_degree() {
        for k in -100..=100 {
            let s = k as f64 / 100.0;
            let got = deg(asin((s * ONE30 as f64) as i64));
            assert!((got - s.asin().to_degrees()).abs() < 0.01, "asin {s}: {got}");
        }
    }

    #[test]
    fn the_sun_is_the_almanacs_to_a_hundredth_of_a_degree() {
        let places = [(51.5, -0.13), (-33.87, 151.21), (64.13, -21.9), (0.0, 0.0), (40.71, -74.0)];
        // Every 7 hours 13 minutes over ten years from 2020.
        let mut t = 1_577_836_800i64;
        while t < 1_893_456_000 {
            for &(lat, lon) in &places {
                let p = Place::from_degrees(lat, lon).unwrap();
                let (lam, alt) = sun(t, p);
                let (rl, ra) = reference(t, p.lat_c as f64 / 100.0, p.lon_c as f64 / 100.0);
                assert!((deg(alt) - ra).abs() < 0.01, "alt at {t} {lat},{lon}: {} vs {ra}", deg(alt));
                let dl = (lam as f64 / 4_294_967_296.0 * 360.0 - rl).rem_euclid(360.0);
                assert!(dl.min(360.0 - dl) < 0.01, "λ at {t}");
            }
            t += 7 * 3600 + 13 * 60;
        }
    }

    #[test]
    fn a_known_day() {
        // London, 2026-06-21 12:00 UTC: the sun ~62° up (noon at 12:01).
        let london = Place::from_degrees(51.51, -0.13).unwrap();
        let alt = deg(sun(1_782_043_200, london).1);
        assert!((61.0..63.0).contains(&alt), "{alt}");
        // ... and at 00:00 UTC, ~15° under the horizon: dark.
        let a = sun_alt(1_782_000_000, london);
        assert!(a < -65_536 / 30, "{a}");
        assert_eq!(daylight(a), 0);
        assert_eq!(daylight(65_536 / 8), 65_536);
        assert_eq!(daylight(0), 32_768);
        // Midsummer in the north, midwinter in the south.
        let (lam, _) = sun(1_782_043_200, london);
        let s = season(lam, london);
        assert!((s - 32_768).abs() < 400, "{s}");
        let sydney = Place::from_degrees(-33.87, 151.21).unwrap();
        let s = season(lam, sydney);
        assert!(!(400..=65_536 - 400).contains(&s), "{s}");
    }

    #[test]
    fn the_moon_and_the_calendar() {
        // Full moon 2026-03-03 11:38 UTC; new moon 2026-03-19 01:23 UTC.
        let full = moon_phase(1_772_537_880);
        assert!((full - 32_768).abs() < 1_500, "{full}");
        let new = moon_phase(1_773_883_380);
        assert!(!(1_500..=65_536 - 1_500).contains(&new), "{new}");
        assert_eq!(yearday(946_684_800), 0); // 2000-01-01
        assert_eq!(yearday(951_782_400), 59); // 2000-02-29
        assert_eq!(yearday(978_220_800), 365); // 2000-12-31
        assert_eq!(yearday(1_782_000_000), 171); // 2026-06-21
        assert_eq!(yearday(-86_400), 364); // 1969-12-31
        assert_eq!(Place::from_degrees(51.506, -0.125), Some(Place { lat_c: 5151, lon_c: -13 }));
        assert_eq!(Place::from_degrees(91.0, 0.0), None);
    }
}

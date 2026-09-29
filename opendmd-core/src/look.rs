//! How the panel shows a picture, after everything else: the DMD tint
//! ("Look") and the phosphor glow. Both are the last step before the LEDs —
//! the firmware applies them in `Panel::blit`, and `opendmd-render`'s
//! `PanelModel` applies them the same way for the simulator and the browser
//! preview — so transitions, virtual files and every decoder pass through
//! them unchanged.
//!
//! **Tint.** A [`Look`] other than [`Look::Colour`] maps a pixel to its
//! luminance ([`luma`]: Rec. 601 weights in integers, `77 R + 150 G + 29 B`
//! over 256) and shows the look's colour scaled by it ([`tint`]): the
//! classic single-colour plasma DMD. A few multiplies per pushed pixel and no
//! buffer.
//!
//! **Glow.** A plasma DMD's dots fade out rather than switch off. With a
//! [`GlowLevel`] other than off, [`Glow`] keeps what the panel shows (a 12 KB
//! RGB copy, [`GLOW_BYTES`]) and every [`GLOW_STEP`] (10 ms) moves each pixel
//! to `max(new, decay(shown))`, per channel, where `decay` multiplies the
//! pixel's *linear* light by `k` — 0.5 for [`GlowLevel::Short`], 0.7 for
//! [`GlowLevel::Long`]. With a display gamma of 2.2 that is the byte times
//! `k^(1/2.2)` (0.730, 0.850), done as a 256-entry table per level
//! ([`decay_table`], built at compile time from an integer multiplier: no
//! float anywhere on the frame path). Values that would fall under
//! [`DECAY_FLOOR`] go to 0, so a dot is gone after 11 steps (110 ms) short
//! and 20 steps (200 ms) long, and the work stops: a row bitmap
//! ([`Glow::decaying_rows`]) keeps the decay pass to the rows that still
//! have a pixel above its target, and a static picture costs nothing once
//! it is empty. A dot that gets brighter shows the new value at once
//! (`max`), as a phosphor does.
//!
//! The copy is allocated fallibly on the first frame after glow is turned
//! on ([`Glow::frame`]) and dropped by [`Glow::release`] — the firmware
//! releases it when the panel goes dark, when nothing is playing and the
//! decay is over, and when glow is turned off. With glow off nothing is
//! held.

use crate::canvas::{Canvas, DirtyMap, Rgb, HEIGHT, PIXELS, WIDTH};
use std::time::Duration;

/// A monochrome tint, or the picture's own colours.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Look {
    /// As decoded (the default).
    #[default]
    Colour,
    /// The classic plasma DMD orange.
    Amber,
    Red,
    Green,
    Blue,
    White,
}

impl Look {
    pub const ALL: [Look; 6] = [
        Look::Colour,
        Look::Amber,
        Look::Red,
        Look::Green,
        Look::Blue,
        Look::White,
    ];

    /// The name `/api/config` uses.
    pub const fn name(self) -> &'static str {
        match self {
            Look::Colour => "colour",
            Look::Amber => "amber",
            Look::Red => "red",
            Look::Green => "green",
            Look::Blue => "blue",
            Look::White => "white",
        }
    }

    /// From a name (`color` is accepted for `colour`).
    pub fn from_name(s: &str) -> Option<Look> {
        match s {
            "color" => Some(Look::Colour),
            _ => Look::ALL.into_iter().find(|l| l.name() == s),
        }
    }

    /// The colour a full-luminance pixel shows, `None` for [`Look::Colour`].
    pub const fn colour(self) -> Option<Rgb> {
        match self {
            Look::Colour => None,
            // PinMAME's default DMD colour, the orange of a neon plasma
            // display.
            Look::Amber => Some(Rgb::new(255, 88, 32)),
            Look::Red => Some(Rgb::new(255, 0, 0)),
            Look::Green => Some(Rgb::new(0, 255, 0)),
            Look::Blue => Some(Rgb::new(0, 0, 255)),
            Look::White => Some(Rgb::new(255, 255, 255)),
        }
    }
}

/// Rec. 601 luminance in integers: `(77 R + 150 G + 29 B + 128) / 256`
/// (the weights 0.299, 0.587, 0.114, scaled to sum to 256). White is 255.
pub const fn luma(p: Rgb) -> u8 {
    ((77 * p.r as u32 + 150 * p.g as u32 + 29 * p.b as u32 + 128) >> 8) as u8
}

/// `c` scaled by `l / 255`, rounded.
const fn scale(c: u8, l: u8) -> u8 {
    ((c as u32 * l as u32 + 127) / 255) as u8
}

/// The pixel as `look` shows it: itself for [`Look::Colour`], else the
/// look's colour scaled by the pixel's [`luma`].
pub const fn tint(p: Rgb, look: Look) -> Rgb {
    match look.colour() {
        None => p,
        Some(c) => {
            let l = luma(p);
            Rgb::new(scale(c.r, l), scale(c.g, l), scale(c.b, l))
        }
    }
}

/// Phosphor glow: how long a dot takes to fade.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum GlowLevel {
    #[default]
    Off,
    /// Linear light × 0.5 per [`GLOW_STEP`]: gone in 110 ms.
    Short,
    /// Linear light × 0.7 per [`GLOW_STEP`]: gone in 200 ms.
    Long,
}

impl GlowLevel {
    pub const ALL: [GlowLevel; 3] = [GlowLevel::Off, GlowLevel::Short, GlowLevel::Long];

    pub const fn name(self) -> &'static str {
        match self {
            GlowLevel::Off => "off",
            GlowLevel::Short => "short",
            GlowLevel::Long => "long",
        }
    }

    pub fn from_name(s: &str) -> Option<GlowLevel> {
        GlowLevel::ALL.into_iter().find(|l| l.name() == s)
    }

    /// This level's decay table (`None` when off).
    pub fn table(self) -> Option<&'static [u8; 256]> {
        match self {
            GlowLevel::Off => None,
            GlowLevel::Short => Some(&DECAY_SHORT),
            GlowLevel::Long => Some(&DECAY_LONG),
        }
    }
}

/// One decay step.
pub const GLOW_STEP: Duration = Duration::from_millis(10);
/// The shown copy glow keeps: one RGB byte triple per pixel.
pub const GLOW_BYTES: usize = PIXELS * 3;
/// A decayed value under this goes to 0 (the dot is out).
pub const DECAY_FLOOR: u8 = 8;
/// Assumed display gamma for "linear light".
pub const GLOW_GAMMA: f64 = 2.2;
/// `k^(1/2.2)` in 16.16 fixed point: 0.5 -> 0.72974, 0.7 -> 0.85033.
pub const SHORT_MULT: u32 = 47_824;
pub const LONG_MULT: u32 = 55_727;

/// `v -> floor(v * mult / 65536)`, 0 under [`DECAY_FLOOR`].
pub const fn decay_table(mult: u32) -> [u8; 256] {
    let mut t = [0u8; 256];
    let mut v = 0;
    while v < 256 {
        let d = (v as u32 * mult) >> 16;
        t[v] = if d < DECAY_FLOOR as u32 { 0 } else { d as u8 };
        v += 1;
    }
    t
}

pub static DECAY_SHORT: [u8; 256] = decay_table(SHORT_MULT);
pub static DECAY_LONG: [u8; 256] = decay_table(LONG_MULT);

fn cmax(a: Rgb, b: Rgb) -> Rgb {
    Rgb::new(a.r.max(b.r), a.g.max(b.g), a.b.max(b.b))
}

fn decayed(p: Rgb, t: &[u8; 256]) -> Rgb {
    Rgb::new(t[p.r as usize], t[p.g as usize], t[p.b as usize])
}

/// What the panel shows under glow; see the module docs. Holds nothing
/// until [`frame`](Self::frame) first runs with glow on.
#[derive(Default)]
pub struct Glow {
    level: GlowLevel,
    /// What is on the panel (before the tint), `PIXELS` long; `None` = not
    /// held (off, released, or the allocation failed).
    shown: Option<Vec<Rgb>>,
    /// Bit `y`: row `y` has a pixel above the picture it is decaying to.
    decaying: u32,
    /// The allocation failed once: not tried again until the level changes
    /// or [`release`](Self::release).
    refused: bool,
    /// The copy was just taken: the next frame pass pushes what changed.
    fresh: bool,
}

impl Glow {
    pub fn new(level: GlowLevel) -> Self {
        Glow {
            level,
            ..Glow::default()
        }
    }

    pub fn level(&self) -> GlowLevel {
        self.level
    }

    /// Change the level; turning it off lets the copy go. True when the
    /// panel may no longer match the picture (glow was held and turned off,
    /// or the level changed mid-decay): repaint it.
    pub fn set_level(&mut self, level: GlowLevel) -> bool {
        if level == self.level {
            return false;
        }
        self.level = level;
        self.refused = false;
        if level == GlowLevel::Off {
            return self.release() != 0;
        }
        false
    }

    /// Let the copy go. Returns the rows that were still decaying: the
    /// panel shows them brighter than the picture, so the caller pushes
    /// those rows of the picture again.
    pub fn release(&mut self) -> u32 {
        self.shown = None;
        self.refused = false;
        std::mem::take(&mut self.decaying)
    }

    /// After an allocation failed: try again at the next frame (the
    /// firmware asks at each new file, when the heap has changed hands).
    pub fn retry(&mut self) {
        self.refused = false;
    }

    /// The copy is held (12 KB of heap).
    pub fn is_held(&self) -> bool {
        self.shown.is_some()
    }

    /// Bytes held: [`GLOW_BYTES`] or 0.
    pub fn held_bytes(&self) -> usize {
        if self.is_held() {
            GLOW_BYTES
        } else {
            0
        }
    }

    /// Rows with a pixel still fading.
    pub fn decaying_rows(&self) -> u32 {
        self.decaying
    }

    pub fn is_decaying(&self) -> bool {
        self.decaying != 0
    }

    /// What the panel shows (before the tint), while held.
    pub fn shown(&self) -> Option<&[Rgb]> {
        self.shown.as_deref()
    }

    /// A new picture: every pixel `canvas`'s dirty map marks becomes
    /// `max(new, shown)`, and each one whose shown value changed is marked
    /// in `push`. False when glow is not in effect — off, or no room for
    /// the copy — and the caller shows `canvas` as it is. The first call
    /// with glow on allocates the copy (fallibly) as `canvas`, and marks
    /// its dirty pixels.
    pub fn frame(&mut self, canvas: &Canvas, push: &mut DirtyMap) -> bool {
        if !self.hold(canvas) {
            return false;
        }
        if self.fresh(canvas, push, false) {
            return true;
        }
        let mut rows = canvas.dirty().row_mask();
        while rows != 0 {
            let y = rows.trailing_zeros() as usize;
            rows &= rows - 1;
            for (x0, x1) in canvas.dirty().row_spans(y) {
                self.over(canvas, push, y, x0, x1);
            }
        }
        true
    }

    /// [`frame`](Self::frame) over every pixel, for a caller with no dirty
    /// map to go by (the simulator and the browser preview). The result is
    /// the same: a pixel the map does not mark is unchanged, and `max(new,
    /// shown)` is then `shown`.
    pub fn frame_all(&mut self, canvas: &Canvas, push: &mut DirtyMap) -> bool {
        if !self.hold(canvas) {
            return false;
        }
        if !self.fresh(canvas, push, true) {
            for y in 0..HEIGHT {
                self.over(canvas, push, y, 0, WIDTH);
            }
        }
        true
    }

    /// Glow is on and the copy is held — allocating it (fallibly) as the
    /// picture if it was not; false if it cannot be.
    fn hold(&mut self, canvas: &Canvas) -> bool {
        if self.level == GlowLevel::Off {
            return false;
        }
        if self.shown.is_none() && !self.refused {
            let mut v = Vec::new();
            if v.try_reserve_exact(PIXELS).is_err() {
                self.refused = true;
                return false;
            }
            v.extend_from_slice(canvas.as_slice());
            self.shown = Some(v);
            self.fresh = true;
        }
        self.shown.is_some()
    }

    /// The copy was just taken as the picture: the panel still shows the
    /// old values of what changed, so push those (everything, for `all`).
    fn fresh(&mut self, canvas: &Canvas, push: &mut DirtyMap, all: bool) -> bool {
        if !std::mem::take(&mut self.fresh) {
            return false;
        }
        if all {
            push.mark_all();
        } else {
            for y in 0..HEIGHT {
                for (x0, x1) in canvas.dirty().row_spans(y) {
                    push.mark_span(y, x0, x1);
                }
            }
        }
        true
    }

    /// `x0..x1` of row `y`: `max(new, shown)`, pushing what changed.
    fn over(&mut self, canvas: &Canvas, push: &mut DirtyMap, y: usize, x0: usize, x1: usize) {
        let Some(shown) = self.shown.as_mut() else {
            return;
        };
        let pixels = canvas.as_slice();
        let mut still = false;
        for x in x0..x1 {
            let i = y * WIDTH + x;
            let (n, s) = (pixels[i], shown[i]);
            let m = cmax(n, s);
            if m != s {
                shown[i] = m;
                push.mark(x, y);
            }
            still |= m != n;
        }
        if still {
            self.decaying |= 1 << y;
        }
    }

    /// One [`GLOW_STEP`]: every pixel of a decaying row that is above its
    /// target in `canvas` goes to `max(target, decay(shown))`, marked in
    /// `push`; a row with nothing left above its target stops decaying.
    /// Nothing when nothing decays.
    pub fn step(&mut self, canvas: &Canvas, push: &mut DirtyMap) {
        let (Some(table), Some(shown)) = (self.level.table(), self.shown.as_mut()) else {
            return;
        };
        let pixels = canvas.as_slice();
        let mut rows = self.decaying;
        while rows != 0 {
            let y = rows.trailing_zeros() as usize;
            rows &= rows - 1;
            let mut still = false;
            for x in 0..WIDTH {
                let i = y * WIDTH + x;
                let (n, s) = (pixels[i], shown[i]);
                if s == n {
                    continue;
                }
                let d = cmax(n, decayed(s, table));
                if d != s {
                    shown[i] = d;
                    push.mark(x, y);
                }
                still |= d != n;
            }
            if !still {
                self.decaying &= !(1 << y);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tint_pinned_values() {
        let px = [
            Rgb::new(255, 255, 255),
            Rgb::new(0, 0, 0),
            Rgb::new(255, 0, 0),
            Rgb::new(0, 255, 0),
            Rgb::new(0, 0, 255),
            Rgb::new(128, 128, 128),
            Rgb::new(200, 100, 50),
        ];
        assert_eq!(px.map(luma), [255, 0, 77, 149, 29, 128, 124]);
        assert!(px.iter().all(|p| tint(*p, Look::Colour) == *p));
        let amber = px.map(|p| tint(p, Look::Amber));
        assert_eq!(
            amber,
            [
                Rgb::new(255, 88, 32),
                Rgb::new(0, 0, 0),
                Rgb::new(77, 27, 10),
                Rgb::new(149, 51, 19),
                Rgb::new(29, 10, 4),
                Rgb::new(128, 44, 16),
                Rgb::new(124, 43, 16),
            ]
        );
        assert_eq!(
            tint(Rgb::new(200, 100, 50), Look::Green),
            Rgb::new(0, 124, 0)
        );
        assert_eq!(
            tint(Rgb::new(200, 100, 50), Look::White),
            Rgb::new(124, 124, 124)
        );
        assert_eq!(tint(Rgb::new(200, 100, 50), Look::Red), Rgb::new(124, 0, 0));
        assert_eq!(
            tint(Rgb::new(200, 100, 50), Look::Blue),
            Rgb::new(0, 0, 124)
        );
    }

    #[test]
    fn names_round_trip() {
        for l in Look::ALL {
            assert_eq!(Look::from_name(l.name()), Some(l));
        }
        assert_eq!(Look::from_name("color"), Some(Look::Colour));
        assert_eq!(Look::from_name("purple"), None);
        for g in GlowLevel::ALL {
            assert_eq!(GlowLevel::from_name(g.name()), Some(g));
        }
        assert_eq!(GlowLevel::from_name("medium"), None);
    }

    #[test]
    fn decay_tables_are_linear_light_times_k() {
        for (table, k) in [(&DECAY_SHORT, 0.5f64), (&DECAY_LONG, 0.7)] {
            for (v, &out) in table.iter().enumerate() {
                let exact =
                    255.0 * ((v as f64 / 255.0).powf(GLOW_GAMMA) * k).powf(1.0 / GLOW_GAMMA);
                let want = if exact.floor() < f64::from(DECAY_FLOOR) {
                    0.0
                } else {
                    exact.floor()
                };
                let got = f64::from(out);
                assert!((got - want).abs() <= 1.0, "k {k} v {v}: {got} vs {want}");
                assert!(v == 0 || usize::from(out) < v, "k {k} v {v} does not fall");
            }
        }
        assert_eq!(DECAY_SHORT[255], 186);
        assert_eq!(DECAY_LONG[255], 216);
        let steps = |t: &[u8; 256]| {
            let (mut v, mut n) = (255u8, 0);
            while v != 0 {
                v = t[v as usize];
                n += 1;
            }
            n
        };
        assert_eq!(steps(&DECAY_SHORT), 11);
        assert_eq!(steps(&DECAY_LONG), 20);
    }

    fn lit(c: &mut Canvas, x: usize, y: usize, p: Rgb) {
        c.set(x, y, p);
    }

    #[test]
    fn glow_off_holds_nothing() {
        let mut g = Glow::new(GlowLevel::Off);
        let mut c = Canvas::new();
        lit(&mut c, 3, 4, Rgb::new(255, 255, 255));
        let mut push = DirtyMap::default();
        assert!(!g.frame(&c, &mut push));
        assert!(!g.is_held() && push.is_clean());
    }

    #[test]
    fn a_dot_that_goes_out_fades_and_the_work_stops() {
        let mut g = Glow::new(GlowLevel::Short);
        let mut c = Canvas::new();
        lit(&mut c, 3, 4, Rgb::new(255, 128, 0));
        let mut push = DirtyMap::default();
        // First frame: the copy is the picture; what changed is pushed.
        assert!(g.frame(&c, &mut push));
        assert!(g.is_held() && push.is_dirty(3, 4) && !g.is_decaying());
        c.dirty_mut().clear();
        // The dot goes out: it stays lit until the decay steps take it.
        lit(&mut c, 3, 4, Rgb::BLACK);
        let mut push = DirtyMap::default();
        assert!(g.frame(&c, &mut push));
        assert!(push.is_clean(), "nothing brighter, nothing pushed");
        assert_eq!(g.decaying_rows(), 1 << 4);
        c.dirty_mut().clear();
        let mut seen = vec![g.shown().unwrap()[4 * WIDTH + 3]];
        while g.is_decaying() {
            let mut push = DirtyMap::default();
            g.step(&c, &mut push);
            assert!(push.is_dirty(3, 4));
            assert_eq!(push.count(), 1);
            seen.push(g.shown().unwrap()[4 * WIDTH + 3]);
        }
        assert_eq!(seen[1], Rgb::new(186, 93, 0));
        assert_eq!(*seen.last().unwrap(), Rgb::BLACK);
        assert_eq!(seen.len(), 12);
        // Decay over: a step does nothing.
        let mut push = DirtyMap::default();
        g.step(&c, &mut push);
        assert!(push.is_clean());
        assert_eq!(g.release(), 0);
        assert!(!g.is_held());
    }

    #[test]
    fn brighter_is_at_once_and_the_decay_stops_at_the_new_picture() {
        let mut g = Glow::new(GlowLevel::Long);
        let mut c = Canvas::new();
        c.fill(Rgb::new(200, 200, 200));
        let mut push = DirtyMap::default();
        g.frame(&c, &mut push);
        c.dirty_mut().clear();
        // Dimmer to 100: decays from 200 and stops at 100, never below.
        c.fill(Rgb::new(100, 50, 250));
        let mut push = DirtyMap::default();
        g.frame(&c, &mut push);
        // Blue got brighter: shown at once.
        assert_eq!(g.shown().unwrap()[0], Rgb::new(200, 200, 250));
        assert_eq!(push.count() as usize, PIXELS);
        let mut n = 0;
        while g.is_decaying() {
            let mut push = DirtyMap::default();
            g.step(&c, &mut push);
            n += 1;
            assert!(n < 30);
        }
        assert!(g
            .shown()
            .unwrap()
            .iter()
            .all(|p| *p == Rgb::new(100, 50, 250)));
        // A frame over every pixel gives the same as the dirty one.
        let mut a = Glow::new(GlowLevel::Short);
        let mut b = Glow::new(GlowLevel::Short);
        let mut c = Canvas::new();
        c.fill(Rgb::new(90, 20, 200));
        let (mut pa, mut pb) = (DirtyMap::default(), DirtyMap::default());
        a.frame(&c, &mut pa);
        b.frame_all(&c, &mut pb);
        c.dirty_mut().clear();
        c.set(5, 5, Rgb::BLACK);
        c.set(6, 30, Rgb::new(255, 255, 255));
        let (mut pa, mut pb) = (DirtyMap::default(), DirtyMap::default());
        a.frame(&c, &mut pa);
        b.frame_all(&c, &mut pb);
        assert_eq!(a.shown(), b.shown());
        assert_eq!(a.decaying_rows(), b.decaying_rows());
        assert_eq!(pa.count(), pb.count());
    }

    #[test]
    fn turning_it_off_releases_and_asks_for_a_repaint_mid_decay() {
        let mut g = Glow::new(GlowLevel::Short);
        let mut c = Canvas::new();
        c.fill(Rgb::new(255, 255, 255));
        g.frame(&c, &mut DirtyMap::default());
        c.dirty_mut().clear();
        c.clear();
        g.frame(&c, &mut DirtyMap::default());
        assert!(g.is_decaying());
        assert!(g.set_level(GlowLevel::Off));
        assert!(!g.is_held());
        assert!(!g.set_level(GlowLevel::Off));
    }
}

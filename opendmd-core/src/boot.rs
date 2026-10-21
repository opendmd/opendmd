//! The startup sequence ("boot show") and the status card.
//!
//! Both are built on the wordmark: "OpenDMD" in the panel's big font (the
//! 5x7 font at twice its size, what `@text` draws by default: 68 x 14 px),
//! amber (`ffaa00`) on black, centred across the panel.
//!
//! **The show** plays on the panel from the moment it lights until the
//! first GIF can take over. The name powers on as a dot-matrix display
//! does: its dots light in a sweep from left to right, each with a little
//! jitter, and each warms up like a phosphor (a dull ember, a brief hot
//! peak, then settling to amber); a single glint sweeps across it once
//! every dot has settled; and a status line fades in underneath in the
//! small font: the version first, then the most useful thing there is to
//! say (the WiFi network to join in setup mode, a card problem in plain
//! words, the address once WiFi is up). [`ShowLength::Short`] is a 1.3 s
//! sting (the boot it covers takes ~0.4 s with a current playlist index);
//! `Full` is 5.6 s, the same idea paced slower, with the letters first
//! glowing faintly like a cold filament and then lighting LED by LED rather
//! than dot by dot. Either holds its last frame (a glint every
//! [`HOLD_GLINT_MS`]) for as long as the card takes. The firmware ends it
//! at `max(minimum, card ready)`, and after [`CAP_MS`] shows the card
//! instead.
//!
//! **The status card** ([`card`]) is what the panel shows whenever there is
//! no card to play from: the same wordmark, a few pixels higher, and two
//! lines — the problem in plain words (warm white) and where to go (amber:
//! the address, or the setup network to join). Calm and looping
//! ([`CARD_LOOP_MS`]: a glint every 4.1 s). Arriving from the show
//! ([`INTRO_MS`]) the name glides up into place, the status line fades out
//! and the two lines fade in.
//!
//! **Pure functions of time.** A frame is a function of the time since the
//! scene began and the [`Info`] strings: nothing is remembered between
//! frames, nothing is allocated (the text is drawn from up to two string
//! slices), and the name is drawn from the font's own rows ([`crate::text`],
//! pixel for pixel what `@text?msg=OpenDMD&scroll=0&color=ffaa00` draws,
//! unit-tested). So the firmware's show, the simulator and the browser
//! draw the same pixels: the virtual file [`PATH`]
//! (`@boot[?len=short|full|card&v=<version>&ip=<address>&ap=<network>&note=<text>]`,
//! parsed by [`crate::synth`]) plays the show once (or the card for one
//! loop) at the effects' frame rate. A frame clears the canvas and walks
//! the name's 68 x 14 box and one or two lines of text: ~1,000 pixels of
//! integer arithmetic.

use crate::canvas::{Canvas, Rgb, WIDTH};
use crate::fx;
use crate::text::{self, Font};
use std::time::Duration;

/// The virtual file that plays the show (see the module docs).
pub const PATH: &str = "@boot";

/// The `boot_show` setting: how long a show the panel plays at start-up.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ShowLength {
    /// No show: the panel stays dark until the first GIF (the status card
    /// still appears if there is no card).
    Off,
    /// 1.3 s. The default.
    #[default]
    Short,
    /// 5.6 s.
    Full,
}

impl ShowLength {
    pub fn from_name(s: &str) -> Option<ShowLength> {
        Some(match s {
            "off" => ShowLength::Off,
            "short" => ShowLength::Short,
            "full" => ShowLength::Full,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        ["off", "short", "full"][self as usize]
    }

    /// The shortest the show runs, in ms (0 for `Off`).
    pub fn min_ms(self) -> u32 {
        match self {
            ShowLength::Off => 0,
            len => beats(len).min,
        }
    }
}

/// How long the show may wait for the card before the status card takes
/// over (ms since the show began).
pub const CAP_MS: u32 = 8_000;
/// The status card's loop (four glints).
pub const CARD_LOOP_MS: u32 = 16_384;
/// The card's arrival from the show: the name gliding up into place, the
/// status line going out, the card's lines coming up.
pub const INTRO_MS: u32 = 700;
/// While the show holds its last frame, a glint this often.
pub const HOLD_GLINT_MS: u32 = 3_000;

/// The show's timeline, in ms.
struct Beats {
    /// The name's dots glow faintly (a cold filament) from `ember.0`, fully
    /// by `ember.1`; `(0, 0)` for none.
    ember: (u32, u32),
    /// The first dot lights; the sweep's last column starts `sweep` later,
    /// each dot up to `jitter` after its column; a dot's warm-up.
    on: u32,
    sweep: u32,
    jitter: u32,
    warm: u32,
    /// Each LED (panel pixel) on its own time rather than each font dot.
    led: bool,
    /// The first glint; the status line comes up.
    glint: u32,
    status: u32,
    /// The shortest show.
    min: u32,
}

const SHORT: Beats = Beats {
    ember: (0, 0),
    on: 60,
    sweep: 420,
    jitter: 90,
    warm: 260,
    led: false,
    glint: 740,
    status: 900,
    min: 1_300,
};

const FULL: Beats = Beats {
    ember: (250, 1_150),
    on: 1_150,
    sweep: 1_400,
    jitter: 450,
    warm: 420,
    led: true,
    glint: 3_550,
    status: 3_850,
    min: 5_600,
};

fn beats(len: ShowLength) -> &'static Beats {
    match len {
        ShowLength::Full => &FULL,
        _ => &SHORT,
    }
}

/// The show's beat at `t` (for the firmware's log): `warm-up` (dark, or
/// the name's faint glow in the full show), `power-on` (its dots
/// lighting), `glint`, `status line`, then `hold` from the minimum on.
pub fn phase(len: ShowLength, t: u32) -> &'static str {
    // (In this order in both timelines: `phases_in_order` checks.)
    let b = beats(len);
    [
        (b.min, "hold"),
        (b.status, "status line"),
        (b.glint, "glint"),
        (b.on, "power-on"),
    ]
    .iter()
    .find(|&&(at, _)| t >= at)
    .map_or("warm-up", |&(_, name)| name)
}

/// What the status line and the card say: the firmware's version, its
/// address once WiFi is up, the setup network while it runs one, and a
/// card problem in plain words. Empty = not known / none.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Info<'a> {
    pub version: &'a str,
    pub ip: &'a str,
    pub ap: &'a str,
    pub note: &'a str,
}

/// A line of text in two parts drawn one after the other, so nothing is
/// joined (or allocated).
pub type Parts<'a> = [&'a str; 2];

const NONE: Parts = ["", ""];

impl<'a> Info<'a> {
    fn version_line(&self) -> Parts<'a> {
        if self.version.is_empty() {
            NONE
        } else {
            ["v", self.version]
        }
    }

    fn join(&self) -> Parts<'a> {
        ["Join WiFi ", self.ap]
    }

    /// The show's status line once the version has been up: the setup
    /// network to join, else the card problem, else the address, else the
    /// version.
    pub fn show_line(&self) -> Parts<'a> {
        if !self.ap.is_empty() {
            self.join()
        } else if !self.note.is_empty() {
            [self.note, ""]
        } else if !self.ip.is_empty() {
            [self.ip, ""]
        } else {
            self.version_line()
        }
    }

    /// The card's two lines: the problem (or the version), then where to
    /// go (the setup network, or the address; the version if the first
    /// line took the problem and there is no address yet).
    pub fn card_lines(&self) -> (Parts<'a>, Parts<'a>) {
        let first = if self.note.is_empty() {
            self.version_line()
        } else {
            [self.note, ""]
        };
        let second = if !self.ap.is_empty() {
            self.join()
        } else if !self.ip.is_empty() {
            [self.ip, ""]
        } else if !self.note.is_empty() {
            self.version_line()
        } else {
            NONE
        };
        (first, second)
    }
}

/// The frame delay of the show and the card as a virtual file: 10 ms (the
/// effects' 100 fps), or the effects' floor while the glow is on.
pub fn frame_delay() -> Duration {
    Duration::from_millis(u64::from(fx::floor().max(10)))
}

/// 0..=256: 0 at or before `a`, 256 at or after `b`, linear between.
fn ramp(t: u32, a: u32, b: u32) -> u32 {
    if t <= a {
        0
    } else if t >= b {
        256
    } else {
        (t - a) * 256 / (b - a)
    }
}

// --- the wordmark -------------------------------------------------------------

const LOGO: &[u8; 7] = b"OpenDMD";
/// The big font: 5x7 at twice its size.
const SCALE: i32 = 2;
/// The name in font dots (7 characters, no trailing gap), and on the panel.
const DOTS_W: i32 = 7 * Font::Normal.advance() as i32 - 1;
const LOGO_W: i32 = DOTS_W * SCALE;
const LOGO_H: i32 = Font::Normal.height() as i32 * SCALE;
const LOGO_X: i32 = (WIDTH as i32 - LOGO_W) / 2;
/// The name's top row in the show (over one line) and on the card (over
/// two).
const SHOW_Y: i32 = 5;
const CARD_Y: i32 = 2;
/// The text's top rows: the show's status line, the card's two lines.
const SHOW_LINE_Y: i32 = 23;
const CARD_LINE_Y: [i32; 2] = [18, 25];

/// The wordmark's amber, as on the site.
const AMBER: Rgb = Rgb::new(255, 170, 0);
/// A dot's warm-up: a dull ember, a hot peak, amber; and the faint glow of
/// the full show's cold filament.
const EMBER: Rgb = Rgb::new(120, 30, 0);
const HOT: Rgb = Rgb::new(255, 214, 130);
const COLD: Rgb = Rgb::new(44, 10, 0);
/// The glint's light, at most `GLINT_K` / 256 over the amber.
const GLINT: Rgb = Rgb::new(255, 244, 210);
const GLINT_K: u32 = 200;
/// The text: the status line and the card's second line, amber
/// (`text::SUBTITLE_COLOUR`); a problem on the card, warm white
/// (`text::TITLE_COLOUR`).
const LINE: Rgb = text::SUBTITLE_COLOUR;
const PROBLEM: Rgb = text::TITLE_COLOUR;

/// Whether font dot `(u, v)` of the name is lit.
fn dot(u: i32, v: i32) -> bool {
    let rows = Font::Normal.rows(LOGO[(u / 5) as usize] as char);
    rows[v as usize] & (0x80 >> (u % 5)) != 0
}

/// Draw the name with its top on row `y0`, each lit panel pixel `(x, y)`
/// (of font dot `(u, v)`) in `colour(x, y, u, v)`.
fn logo(canvas: &mut Canvas, y0: i32, colour: impl Fn(i32, i32, i32, i32) -> Rgb) {
    for y in y0.max(0)..y0 + LOGO_H {
        let v = (y - y0) / SCALE;
        for x in LOGO_X..LOGO_X + LOGO_W {
            let u = (x - LOGO_X) / SCALE;
            if dot(u, v) {
                canvas.set(x as usize, y as usize, colour(x, y, u, v));
            }
        }
    }
}

/// `a` towards `b` by `k` / 256.
fn mix(a: Rgb, b: Rgb, k: u32) -> Rgb {
    let m = |x: u8, y: u8| ((u32::from(x) * (256 - k) + u32::from(y) * k) >> 8) as u8;
    Rgb::new(m(a.r, b.r), m(a.g, b.g), m(a.b, b.b))
}

/// A dot `age` ms into its warm-up of `w`: from a dull ember up to a hot
/// peak at a third of the way, then cooling to amber.
fn warm(age: u32, w: u32) -> Rgb {
    let k = ramp(age, 0, w);
    if k < 85 {
        mix(EMBER, HOT, k * 3)
    } else {
        mix(HOT, AMBER, (k - 85) * 256 / 171)
    }
}

/// The glint `p` ms into its pass (none after 520 ms): a soft band 7 px
/// wide leaning like a `/`, sweeping the name from left to right.
/// Returns its light at `(x, y)` (/ 256) for a name whose top is `y0`.
fn glint(p: u32, x: i32, y: i32, y0: i32) -> u32 {
    const PASS: u32 = 520;
    if p >= PASS {
        return 0;
    }
    let gx = LOGO_X - 10 + ((LOGO_W + 20) as u32 * p / PASS) as i32;
    let d = (x - gx + (y - y0 - LOGO_H / 2) / 2).unsigned_abs();
    (4u32.saturating_sub(d)) * GLINT_K / 4
}

/// The amber name with the glint `p` ms into its pass.
fn glinting(c: Rgb, p: u32, x: i32, y: i32, y0: i32) -> Rgb {
    match glint(p, x, y, y0) {
        0 => c,
        k => mix(c, GLINT, k),
    }
}

fn parts_width(p: &Parts) -> i32 {
    let n = p[0].chars().count() + p[1].chars().count();
    (n as i32 * Font::Small.advance() as i32 - 1).max(0)
}

/// `p` in the small font, centred, its top at `y`.
fn line(canvas: &mut Canvas, y: i32, p: &Parts, colour: Rgb) {
    let w = parts_width(p);
    if w == 0 || colour == Rgb::BLACK {
        return;
    }
    let x = (WIDTH as i32 - w) / 2;
    let w0 = text::draw_text_with(canvas, x, y, p[0], colour, Font::Small, 1);
    let gap = if p[0].is_empty() { 0 } else { 1 };
    text::draw_text_with(canvas, x + w0 as i32 + gap, y, p[1], colour, Font::Small, 1);
}

/// The show's status line at `t`, and its light (/ 256): the version,
/// fading in over 300 ms, then 1.2 s later what matters (a 150 ms dip out
/// and back in, if that is different).
fn status<'a>(b: &Beats, t: u32, info: &Info<'a>) -> (Parts<'a>, u32) {
    let swap = b.status + 1_200;
    let (version, then) = (info.version_line(), info.show_line());
    let text = if t < swap { version } else { then };
    let mut k = ramp(t, b.status, b.status + 300);
    if version != then {
        k = k.min(t.abs_diff(swap).min(150) * 256 / 150);
    }
    (text, k)
}

// --- the scenes ---------------------------------------------------------------

/// The show's frame at `t` ms (`len` `Off` draws the short one). Every
/// pixel is written and marked.
pub fn show(canvas: &mut Canvas, len: ShowLength, t: u32, info: &Info) {
    let b = beats(len);
    canvas.clear();
    let cold = if b.ember.1 > 0 {
        fx::scale(COLD, ramp(t, b.ember.0, b.ember.1))
    } else {
        Rgb::BLACK
    };
    // The glint: the first pass, then one every `HOLD_GLINT_MS`.
    let p = t
        .checked_sub(b.glint)
        .map_or(u32::MAX, |d| d % HOLD_GLINT_MS);
    let last = (LOGO_W - 1) as u32;
    logo(canvas, SHOW_Y, |x, y, u, v| {
        // The sweep by column (of LEDs or of dots), a jitter from a hash.
        let (col, key) = if b.led {
            ((x - LOGO_X) as u32, (y * 128 + x) as u32)
        } else {
            ((u * SCALE) as u32, (v * 64 + u) as u32)
        };
        let at = b.on + col * b.sweep / last + fx::hash(key ^ 0xb007) % b.jitter;
        match t.checked_sub(at) {
            None => cold,
            Some(age) if age < b.warm => warm(age, b.warm),
            Some(_) => glinting(AMBER, p, x, y, SHOW_Y),
        }
    });
    if t >= b.status {
        let (text, k) = status(b, t, info);
        line(canvas, SHOW_LINE_Y, &text, fx::scale(LINE, k));
    }
    canvas.dirty_mut().mark_all();
}

/// The status card at `t` ms into it. `from_show`: it follows the show
/// (its length and the show's time when the card took over), so it
/// arrives from it ([`INTRO_MS`]) rather than cutting in. Every pixel is
/// written and marked.
pub fn card(canvas: &mut Canvas, t: u32, info: &Info, from_show: Option<(ShowLength, u32)>) {
    canvas.clear();
    let arrive = match from_show {
        Some(_) => ramp(t, 0, INTRO_MS),
        None => 256,
    };
    // The show's status line going out over the first half.
    if let Some((len, at)) = from_show.filter(|_| arrive < 128) {
        let (text, k) = status(beats(len), at + t, info);
        let out = 256 - arrive * 2;
        line(canvas, SHOW_LINE_Y, &text, fx::scale(LINE, k * out / 256));
    }
    // The name gliding up, easing out; a glint every 4,096 ms, the first
    // 1.5 s in.
    let e = 256 - (256 - arrive) * (256 - arrive) / 256;
    let y0 = SHOW_Y - ((SHOW_Y - CARD_Y) as u32 * e).div_ceil(256) as i32;
    let p = (t + 4_096 - 1_500) % 4_096;
    logo(canvas, y0, |x, y, _, _| glinting(AMBER, p, x, y, y0));
    // The lines coming up over the second half.
    let k = ramp(arrive, 128, 256);
    let (first, second) = info.card_lines();
    let c1 = if info.note.is_empty() { LINE } else { PROBLEM };
    line(canvas, CARD_LINE_Y[0], &first, fx::scale(c1, k));
    line(canvas, CARD_LINE_Y[1], &second, fx::scale(LINE, k));
    canvas.dirty_mut().mark_all();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::HEIGHT;

    const INFO: Info = Info {
        version: "0.3.0",
        ip: "192.168.1.69",
        ap: "",
        note: "",
    };

    fn lit(c: &Canvas) -> usize {
        c.as_slice().iter().filter(|p| **p != Rgb::BLACK).count()
    }

    /// The name's panel pixels: what `text::draw_text_with` draws in the
    /// big font with its top-left at `(LOGO_X, y0)`.
    fn wordmark(y0: i32) -> Canvas {
        let mut c = Canvas::new();
        text::draw_text_with(&mut c, LOGO_X, y0, "OpenDMD", AMBER, Font::Normal, 2);
        c
    }

    /// `c`'s rows `y0..y0 + LOGO_H` only (the name's band).
    fn band(c: &Canvas, y0: i32) -> Vec<Rgb> {
        (y0 as usize..(y0 + LOGO_H) as usize)
            .flat_map(|y| (0..WIDTH).map(move |x| c.get(x, y)))
            .collect()
    }

    #[test]
    fn phases_in_order() {
        for len in [ShowLength::Short, ShowLength::Full] {
            let mut seen: Vec<&str> = Vec::new();
            for t in (0..7_000).step_by(10) {
                let n = phase(len, t);
                if seen.last() != Some(&n) {
                    seen.push(n);
                }
            }
            assert_eq!(
                seen,
                ["warm-up", "power-on", "glint", "status line", "hold"],
                "{len:?}"
            );
        }
    }

    #[test]
    fn the_virtual_file_plays_the_show_once_with_its_text() {
        use crate::synth::{self, SynthSource};
        assert_eq!(synth::title(PATH).as_deref(), Some("Startup"));
        let count = |path: &str| {
            let mut src = SynthSource::new(synth::open(path).unwrap()).unwrap();
            let mut c = Canvas::new();
            let mut n = 0;
            while let Some(d) = src.next_frame(&mut c).unwrap() {
                assert_eq!(d, Duration::from_millis(10));
                n += 1;
            }
            (n, c)
        };
        assert_eq!(count(PATH).0, 130);
        assert_eq!(count("@boot?len=full").0, 560);
        let (n, last) =
            count("@boot?len=card&v=0.3.0&note=Insert%20a%20microSD%20card&ip=192.168.1.69");
        assert_eq!(n, CARD_LOOP_MS.div_ceil(10));
        // The card's last frame, as `card` draws it with that text.
        let mut want = Canvas::new();
        card(
            &mut want,
            (n - 1) * 10,
            &Info {
                note: "Insert a microSD card",
                ..INFO
            },
            None,
        );
        assert_eq!(last, want);
    }

    #[test]
    fn names_round_trip() {
        for l in [ShowLength::Off, ShowLength::Short, ShowLength::Full] {
            assert_eq!(ShowLength::from_name(l.name()), Some(l));
        }
        assert_eq!(ShowLength::from_name("long"), None);
        assert_eq!(ShowLength::default(), ShowLength::Short);
        assert_eq!(ShowLength::Off.min_ms(), 0);
        assert_eq!(ShowLength::Short.min_ms(), 1_300);
        assert_eq!(ShowLength::Full.min_ms(), 5_600);
    }

    #[test]
    fn every_beat_is_repeatable_and_the_name_only_grows() {
        let mark = wordmark(SHOW_Y);
        for len in [ShowLength::Short, ShowLength::Full] {
            let b = beats(len);
            let mut before = 0;
            for t in (0..=len.min_ms() + 3_000).step_by(10) {
                let mut a = Canvas::new();
                show(&mut a, len, t, &INFO);
                // Pure: the same picture over whatever was there.
                let mut c = Canvas::new();
                c.fill(Rgb::new(9, 9, 9));
                show(&mut c, len, t, &INFO);
                assert_eq!(a, c, "{len:?} at {t} ms");
                // Nothing but the name's pixels above the status line.
                for y in 0..SHOW_LINE_Y as usize {
                    for x in 0..WIDTH {
                        if mark.get(x, y) == Rgb::BLACK {
                            assert_eq!(a.get(x, y), Rgb::BLACK, "{len:?} at {t} ms ({x}, {y})");
                        }
                    }
                }
                let n = lit(&a);
                if t < b.status {
                    assert!(n >= before, "{len:?} at {t} ms: {n} lit after {before}");
                    before = n;
                }
                // Every dot lit once the sweep has passed.
                if t >= b.on + b.sweep + b.jitter {
                    assert!(n >= lit(&mark), "{len:?} at {t} ms: {n} lit");
                }
            }
        }
    }

    #[test]
    fn the_full_show_glows_before_it_lights() {
        let b = beats(ShowLength::Full);
        let mut c = Canvas::new();
        show(&mut c, ShowLength::Full, b.on - 1, &INFO);
        assert_eq!(lit(&c), lit(&wordmark(SHOW_Y)));
        let glow = c.get(LOGO_X as usize, SHOW_Y as usize + 4);
        assert!(glow != Rgb::BLACK && glow.r < 64, "{glow:?}");
        assert!(c.as_slice().iter().all(|&p| p == Rgb::BLACK || p == glow));
        let mut short = Canvas::new();
        show(&mut short, ShowLength::Short, SHORT.on - 1, &INFO);
        assert_eq!(lit(&short), 0);
    }

    #[test]
    fn the_name_settles_as_the_big_font_draws_it() {
        // Between glints, after the power-on, the name is exactly what
        // `text::draw_text_with` draws in the big font, in amber.
        for len in [ShowLength::Short, ShowLength::Full] {
            let b = beats(len);
            let t = b.glint + 1_000;
            let mut shown = Canvas::new();
            show(&mut shown, len, t, &INFO);
            let settled = band(&shown, SHOW_Y);
            assert_eq!(settled, band(&wordmark(SHOW_Y), SHOW_Y), "{len:?}");
            // And mid-glint, lighter only on the name's pixels.
            let mut g = Canvas::new();
            show(&mut g, len, b.glint + HOLD_GLINT_MS + 260, &INFO);
            let lighter = band(&g, SHOW_Y)
                .iter()
                .zip(&settled)
                .filter(|(a, b)| a != b)
                .inspect(|(a, b)| assert!(a.b > b.b && **b == AMBER))
                .count();
            assert!(lighter > 20, "{len:?}: {lighter} glinting");
        }
    }

    #[test]
    fn the_wordmark_is_what_text_draws() {
        // `@text?msg=OpenDMD&scroll=0&color=ffaa00`, the site's wordmark,
        // is the same pixels as the card's name, moved.
        use crate::synth::{self, SynthSource};
        let mut src =
            SynthSource::new(synth::open("@text?msg=OpenDMD&scroll=0&color=ffaa00").unwrap())
                .unwrap();
        let mut site = Canvas::new();
        src.next_frame(&mut site).unwrap();
        let y = (HEIGHT as i32 - LOGO_H) / 2;
        let mut c = Canvas::new();
        card(&mut c, 0, &Info::default(), None);
        assert_eq!(band(&site, y), band(&c, CARD_Y));
        assert_eq!(lit(&site), lit(&c));
    }

    #[test]
    fn the_status_line_says_the_version_then_what_matters() {
        let at = |info: &Info, t: u32| {
            let mut c = Canvas::new();
            show(&mut c, ShowLength::Short, t, info);
            (SHOW_LINE_Y as usize..HEIGHT)
                .flat_map(|y| (0..WIDTH).map(move |x| (x, y)))
                .filter(|&(x, y)| c.get(x, y) != Rgb::BLACK)
                .count()
        };
        let b = beats(ShowLength::Short);
        let quiet = Info { ip: "", ..INFO };
        // Not yet, then up (the version).
        assert_eq!(at(&INFO, b.status - 10), 0);
        assert!(at(&INFO, b.status + 400) > 0);
        assert_eq!(status(b, b.status + 400, &INFO), (["v", "0.3.0"], 256));
        // Then the address, dipping out and back in; no dip if it's the same.
        assert_eq!(status(b, b.status + 1_200, &INFO).1, 0);
        assert_eq!(
            status(b, b.status + 1_500, &INFO),
            (["192.168.1.69", ""], 256)
        );
        assert_eq!(status(b, b.status + 1_200, &quiet), (["v", "0.3.0"], 256));
        assert_eq!(INFO.show_line(), ["192.168.1.69", ""]);
        assert_eq!(quiet.show_line(), ["v", "0.3.0"]);
        let ap = Info {
            ap: "OpenDMD-Setup",
            note: "Insert a microSD card",
            ..INFO
        };
        assert_eq!(ap.show_line(), ["Join WiFi ", "OpenDMD-Setup"]);
        assert_eq!(
            ap.card_lines(),
            (
                ["Insert a microSD card", ""],
                ["Join WiFi ", "OpenDMD-Setup"]
            )
        );
        let no_card = Info {
            note: "Insert a microSD card",
            ..quiet
        };
        assert_eq!(
            no_card.card_lines(),
            (["Insert a microSD card", ""], ["v", "0.3.0"])
        );
        assert_eq!(
            Info {
                note: "Insert a microSD card",
                ..INFO
            }
            .card_lines()
            .1,
            ["192.168.1.69", ""]
        );
        // Every line the firmware can show fits the panel.
        for p in [ap.show_line(), ap.card_lines().0, ap.card_lines().1] {
            assert!(parts_width(&p) <= WIDTH as i32, "{p:?}");
        }
    }

    #[test]
    fn the_card_loops_and_arrives_from_the_show() {
        let info = Info {
            note: "Insert a microSD card",
            ..INFO
        };
        let draw = |t: u32, from| {
            let mut c = Canvas::new();
            card(&mut c, t, &info, from);
            c
        };
        // A loop later, the same picture; and it moves (the glint).
        for t in [0, 1_000, 1_700] {
            assert_eq!(draw(t, None), draw(t + CARD_LOOP_MS, None));
        }
        assert_ne!(draw(1_000, None), draw(1_700, None));
        assert!(lit(&draw(0, None)) > 300);
        // Arriving: the name where the show had it, the show's status line
        // still there and the card's lines not yet; settled at the end, as
        // the card drawn on its own.
        let from = Some((ShowLength::Short, ShowLength::Short.min_ms()));
        assert_eq!(band(&draw(0, from), SHOW_Y), band(&wordmark(SHOW_Y), SHOW_Y));
        let row = |c: &Canvas, y: i32| (0..WIDTH).any(|x| c.get(x, y as usize) != Rgb::BLACK);
        assert!(row(&draw(0, from), SHOW_LINE_Y + 1));
        assert!(!row(&draw(0, from), CARD_LINE_Y[0] + 1));
        assert_eq!(draw(INTRO_MS, from), draw(INTRO_MS, None));
        // The problem in warm white, the address in amber.
        let c = draw(1_000, None);
        assert!((18..24).any(|y| (0..WIDTH).any(|x| c.get(x, y) == PROBLEM)));
        assert!((25..31).any(|y| (0..WIDTH).any(|x| c.get(x, y) == LINE)));
        assert_eq!(band(&c, CARD_Y), band(&wordmark(CARD_Y), CARD_Y));
    }
}

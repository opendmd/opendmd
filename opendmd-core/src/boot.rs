//! The startup sequence ("boot show") and the status card.
//!
//! Both are built on the wordmark: "OpenDMD" in the panel's big font (the
//! 5x7 font at twice its size: 68 x 14 px) in two colours, "Open" warm
//! white (`ffd696`) and "DMD" amber (`ffaa00`), on black, centred across
//! the panel.
//!
//! **The show** plays on the panel from the moment it lights until the
//! first GIF can take over. The name powers on as a dot-matrix display
//! does: its dots light in a sweep from left to right, each with a little
//! jitter, and each warms up like a phosphor (a dull ember, a brief hot
//! peak, then settling to its colour); a single glint sweeps across it once
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
//! pixel for pixel what `text::draw_text_with` draws, unit-tested). So the firmware's show, the simulator and the browser
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

    /// The shortest the show runs on the first boot of a new version, when
    /// its status line says "Updated to v…" ([`Info::updated`]) for
    /// [`UPDATED_MS`]: 3.9 s short, 6.85 s full. `Off` plays the short show
    /// for that boot (see [`ShowLength::for_update`]).
    pub fn min_ms_updated(self) -> u32 {
        let b = beats(self);
        b.min.max(b.status + UPDATED_MS)
    }

    /// The show for the first boot of a new version: `Off` plays the short
    /// one (the panel says it updated, then stays dark again from the next
    /// boot on).
    pub fn for_update(self) -> ShowLength {
        match self {
            ShowLength::Off => ShowLength::Short,
            len => len,
        }
    }
}

/// How long the show's status line says "Updated to v…" on the first boot
/// of a new version, from the moment it comes up.
pub const UPDATED_MS: u32 = 3_000;

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
    /// The first boot of a new version: the status line says "Updated to
    /// v<version>" ([`updated_line`]) for [`UPDATED_MS`] instead of the
    /// version, then what matters.
    pub updated: bool,
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

    /// The status line's first words: the version, or on the first boot of
    /// a new one, that it updated.
    fn first_line(&self) -> Parts<'a> {
        if self.updated && !self.version.is_empty() {
            updated_line(self.version)
        } else {
            self.version_line()
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

/// The wordmark's amber ("DMD"), as on the site.
const AMBER: Rgb = Rgb::new(255, 170, 0);
/// The wordmark's warm white ("Open"): the card's title white
/// (`text::TITLE_COLOUR`), the O of the mark beside the amber D.
const WHITE: Rgb = text::TITLE_COLOUR;
/// How many of the name's letters are "Open", in [`WHITE`].
const OPEN: i32 = 4;
/// A dot's warm-up: a dull ember, a hot peak (each part's own), its
/// colour; and the faint glow of the full show's cold filament.
const EMBER: Rgb = Rgb::new(120, 30, 0);
const HOT: Rgb = Rgb::new(255, 214, 130);
const COLD: Rgb = Rgb::new(44, 10, 0);
/// The glint's light, at most `GLINT_K` / 256 over the name's colour; also
/// "Open"'s hot peak, hotter than its white as `HOT` is than the amber.
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

/// Font column `u`'s colour and warm-up peak: "Open" warm white, "DMD"
/// amber.
fn part(u: i32) -> (Rgb, Rgb) {
    if u / Font::Normal.advance() as i32 >= OPEN {
        (AMBER, HOT)
    } else {
        (WHITE, GLINT)
    }
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

/// A dot of font column `u`, `age` ms into its warm-up of `w`: from a dull
/// ember up to its part's hot peak at a third of the way, then cooling to
/// its part's colour.
fn warm(u: i32, age: u32, w: u32) -> Rgb {
    let (colour, hot) = part(u);
    let k = ramp(age, 0, w);
    if k < 85 {
        mix(EMBER, hot, k * 3)
    } else {
        mix(hot, colour, (k - 85) * 256 / 171)
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

/// The name's colour `c` with the glint `p` ms into its pass.
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
/// and back in, if that is different). On the first boot of a new version,
/// "Updated to v…" instead of the version, for [`UPDATED_MS`].
fn status<'a>(b: &Beats, t: u32, info: &Info<'a>) -> (Parts<'a>, u32) {
    let swap = b.status + if info.updated { UPDATED_MS } else { 1_200 };
    let (version, then) = (info.first_line(), info.show_line());
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
            Some(age) if age < b.warm => warm(u, age, b.warm),
            Some(_) => glinting(part(u).0, p, x, y, SHOW_Y),
        }
    });
    if t >= b.status {
        let (text, k) = status(b, t, info);
        line(canvas, SHOW_LINE_Y, &text, fx::scale(LINE, k));
    }
    canvas.dirty_mut().mark_all();
}

// --- a Formula as the show ------------------------------------------------------

/// The information line over a Formula show at `t`, and its light (/ 256):
/// only when there is something to say, never the version. On the first
/// boot of a new version "Updated to v…" for [`UPDATED_MS`], then (a 150 ms
/// dip out and back in) the setup network to join, a card problem, or the
/// address; else nothing. It comes up at the show's status beat, as the
/// wordmark's line does.
fn formula_status<'a>(b: &Beats, t: u32, info: &Info<'a>) -> (Parts<'a>, u32) {
    let then = if !info.ap.is_empty() {
        info.join()
    } else if !info.note.is_empty() {
        [info.note, ""]
    } else if !info.ip.is_empty() {
        [info.ip, ""]
    } else {
        NONE
    };
    let mut k = ramp(t, b.status, b.status + 300);
    if !info.updated || info.version.is_empty() {
        return (then, k);
    }
    let (first, swap) = (updated_line(info.version), b.status + UPDATED_MS);
    let text = if t < swap { first } else { then };
    if first != then {
        k = k.min(t.abs_diff(swap).min(150) * 256 / 150);
    }
    (text, k)
}

/// The information line over a Formula show's frame at `t` (the Formula
/// drawn into `canvas` first): when there is something to say
/// ([`formula_status`]), the band behind it dimmed (a Formula covers every
/// pixel) and the line over it, in the wordmark's place and colour. Marks
/// what it changes; draws nothing otherwise.
pub fn formula_line(canvas: &mut Canvas, len: ShowLength, t: u32, info: &Info) {
    let b = beats(len);
    if t < b.status {
        return;
    }
    let (text, k) = formula_status(b, t, info);
    let w = parts_width(&text);
    if w == 0 || k == 0 {
        return;
    }
    // A margin of 2 px around the text, dimmed to a quarter at full light.
    let x0 = ((WIDTH as i32 - w) / 2 - 2).max(0) as usize;
    let x1 = (x0 + w as usize + 4).min(WIDTH);
    let y0 = (SHOW_LINE_Y - 1) as usize;
    let y1 = y0 + Font::Small.height() + 2;
    let keep = 256 - k * 3 / 4;
    for y in y0..y1 {
        for x in x0..x1 {
            let c = canvas.get(x, y);
            canvas.set(x, y, fx::scale(c, keep));
        }
    }
    line(canvas, SHOW_LINE_Y, &text, fx::scale(LINE, k));
}

/// How heavy a Formula may be to play as the start-up show (its frame,
/// estimated, µs): the look's limit, inside the show's 10 ms frames with
/// room for the line and the blit.
#[cfg(feature = "psi")]
pub const FORMULA_MAX_EST_US: u32 = crate::fx_psi::LOOK_MAX_EST_US;

/// A Formula as the start-up show (the `boot_formula` setting): drawn in
/// the wordmark's place, the information line over it ([`formula_line`]),
/// for as long as the wordmark's show would run. Pure in `t` as the show
/// is, so the firmware's start-up and `@boot?formula=…` draw the same
/// frames: the Formula's frame of its grid at or before `t`, drawn only
/// when that frame changes.
#[cfg(feature = "psi")]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FormulaShow {
    fx: crate::fx::Fx,
    /// The grid frame last drawn.
    last: Option<u32>,
}

#[cfg(feature = "psi")]
impl FormulaShow {
    /// The show for a bundled Formula by name (`@formula:<stem>`) or an
    /// inline one (`@formula?psb=…`), or why not (it does not compile, is
    /// too heavy, the heap is short: `psi::NO_MEMORY`). A Formula on the
    /// card is read by the player: [`FormulaShow::from_bytes`].
    pub fn open(path: &str) -> Result<FormulaShow, crate::psi::Error> {
        use crate::synth::{formula_ref, FormulaRef, FORMULA};
        match formula_ref(path) {
            Some(FormulaRef::Bundled { .. } | FormulaRef::Inline) => {
                Self::checked(crate::fx_psi::PsiFx::from_query(&path[FORMULA.len()..])?)
            }
            Some(FormulaRef::File { .. }) => Err(err("a Formula on the card is read from the card")),
            None => Err(err("that is not a Formula")),
        }
    }

    /// The show for a compiled Formula read from its file (`.psb`) with its
    /// options (`a=0.5&fps=…`).
    pub fn from_bytes(psb: &[u8], opts: &str) -> Result<FormulaShow, crate::psi::Error> {
        Self::checked(crate::fx_psi::PsiFx::from_bytes(psb, opts)?)
    }

    fn checked(fx: crate::fx_psi::PsiBox) -> Result<FormulaShow, crate::psi::Error> {
        let est = fx.program().esp32_us();
        if est > FORMULA_MAX_EST_US {
            return Err(err(&format!(
                "this Formula is too heavy for the start-up show (about {} ms a frame; the limit is {} ms)",
                est.div_ceil(1000),
                FORMULA_MAX_EST_US / 1000
            )));
        }
        Ok(FormulaShow {
            fx: crate::fx::Fx::Psi(fx),
            last: None,
        })
    }

    /// Its `seed` (tests: a Formula that never repeats plays a new one at
    /// each open).
    #[doc(hidden)]
    pub fn set_seed(&mut self, seed: i32) {
        let crate::fx::Fx::Psi(f) = &mut self.fx;
        f.set_seed(seed);
    }

    /// Draw the show's frame at `t` ms into `canvas` (the canvas holds the
    /// frame this drew last, or anything before the first): the Formula's
    /// frame on its grid, the line over it. False when that frame is the
    /// one already drawn (nothing changed).
    pub fn frame(&mut self, canvas: &mut Canvas, len: ShowLength, t: u32, info: &Info) -> bool {
        let grid = (self.fx.delay().as_millis() as u32).max(1);
        let n = t / grid;
        if self.last == Some(n) {
            return false;
        }
        if self.last.is_none() {
            canvas.dirty_mut().mark_all();
        }
        self.last = Some(n);
        let at = n * grid;
        self.fx.frame(Duration::from_millis(u64::from(at)), canvas);
        formula_line(canvas, len, at, info);
        true
    }
}

#[cfg(feature = "psi")]
fn err(msg: &str) -> crate::psi::Error {
    crate::psi::Error {
        start: 0,
        end: 0,
        msg: msg.to_string(),
    }
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
    logo(canvas, y0, |x, y, u, _| glinting(part(u).0, p, x, y, y0));
    // The lines coming up over the second half.
    let k = ramp(arrive, 128, 256);
    let (first, second) = info.card_lines();
    let c1 = if info.note.is_empty() { LINE } else { PROBLEM };
    line(canvas, CARD_LINE_Y[0], &first, fx::scale(c1, k));
    line(canvas, CARD_LINE_Y[1], &second, fx::scale(LINE, k));
    canvas.dirty_mut().mark_all();
}

/// "Updated to v1.0.0-beta.17": the whole version when it fits the small
/// font's 32 characters, else the part after its first `-`
/// ("Updated to beta.16-us-1a2b3c"), else just "Updated". Slices of
/// `version`, nothing joined.
pub fn updated_line(version: &str) -> Parts<'_> {
    fitted("Updated to v", "Updated to ", "Updated", version)
}

/// `[lead, version]` if that fits one line of the small font, else `[short,
/// version's tail after its first '-']`, else `[bare, ""]`.
fn fitted<'a>(lead: &'a str, short: &'a str, bare: &'a str, version: &'a str) -> Parts<'a> {
    const MAX: usize = WIDTH / Font::Small.advance();
    let fits = |a: &str, b: &str| a.chars().count() + b.chars().count() <= MAX;
    let tail = version.split_once('-').map_or("", |(_, t)| t);
    if fits(lead, version) {
        [lead, version]
    } else if !tail.is_empty() && fits(short, tail) {
        [short, tail]
    } else {
        [bare, ""]
    }
}

/// What a firmware update has reached, for [`update_notice`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpdateStage {
    /// The bundle is being received and checked.
    Receiving,
    /// Checked and staged: the restart into the updater is next, which
    /// shows the same picture while it installs ([`installing`]).
    Installing,
    /// The same, for a board whose updater predates the panel
    /// ([`updater_lights_panel`] false): it says the panel goes dark.
    InstallingDark,
}

/// The first updater that shows a picture while it installs: its app
/// descriptor's version is `"updater <n>"` (`ota/updater/sdkconfig.defaults`
/// `CONFIG_APP_PROJECT_VER`), and this is the lowest such `<n>`.
pub const UPDATER_PANEL_VERSION: u32 = 2;

/// The updater's number from its app descriptor's version (`"updater 2"` ->
/// 2); `None` for anything else.
pub fn updater_number(version: &str) -> Option<u32> {
    let n = version.trim().strip_prefix("updater ")?;
    if n.is_empty() || !n.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    n.parse().ok()
}

/// Whether the updater whose app descriptor says `version` lights the panel
/// while it installs. An unreadable or unknown version is taken as not:
/// the firmware's screen then says the panel goes dark.
pub fn updater_lights_panel(version: Option<&str>) -> bool {
    version
        .and_then(updater_number)
        .is_some_and(|n| n >= UPDATER_PANEL_VERSION)
}

/// The panel while a firmware update is received and installed: a still
/// picture, drawn once per stage. The stage in the big font (amber, top row
/// 2, as on the status card), then two lines of the small font (rows 18
/// and 25): "to v<version>" in amber (installing: [`INSTALL_DOTS`], all
/// three lit; installing with an updater that leaves the panel dark: "Dark
/// while it installs") and "Don't unplug" in warm white. Every pixel is written and
/// marked.
pub fn update_notice(canvas: &mut Canvas, stage: UpdateStage, version: &str) {
    match stage {
        UpdateStage::Receiving => notice(
            canvas,
            "Updating",
            if version.is_empty() {
                NONE
            } else {
                fitted("to v", "to ", "", version)
            },
        ),
        UpdateStage::Installing => installing(canvas, [true; INSTALL_DOTS.len()]),
        UpdateStage::InstallingDark => notice(canvas, "Installing", ["Dark while it installs", ""]),
    }
}

/// Installing's middle line: three dots, each a 2x2 block at its top-left
/// corner, 4 columns apart and centred, in the middle line's amber. The
/// firmware's own screen lights all three; the updater, which shows
/// [`installing`] with none lit (`ota/updater/main/picture.h`, made by
/// `crate::updater_picture`), lights one at a time, left to right.
pub const INSTALL_DOTS: [(usize, usize); 3] = [(59, 20), (63, 20), (67, 20)];
/// A dot's side, pixels.
pub const INSTALL_DOT: usize = 2;
/// The dots' colour: the middle line's.
pub const INSTALL_DOT_COLOUR: Rgb = LINE;

/// "Installing" over the middle line's dots (`lit`) over "Don't unplug":
/// the picture the panel holds from the bundle being staged until the new
/// firmware starts, first drawn by the firmware, then by the updater from
/// its build-time copy while it copies the image into flash.
pub fn installing(canvas: &mut Canvas, lit: [bool; INSTALL_DOTS.len()]) {
    notice(canvas, "Installing", NONE);
    for (&(x0, y0), on) in INSTALL_DOTS.iter().zip(lit) {
        if on {
            for y in y0..y0 + INSTALL_DOT {
                for x in x0..x0 + INSTALL_DOT {
                    canvas.set(x, y, INSTALL_DOT_COLOUR);
                }
            }
        }
    }
    canvas.dirty_mut().mark_all();
}

/// [`update_notice`]'s layout: `title` big and centred, `first` on the
/// middle line, "Don't unplug" under it.
fn notice(canvas: &mut Canvas, title: &str, first: Parts) {
    canvas.clear();
    let w = text::text_width(Font::Normal, title, 2) as i32;
    text::draw_text_with(
        canvas,
        (WIDTH as i32 - w) / 2,
        CARD_Y,
        title,
        AMBER,
        Font::Normal,
        2,
    );
    line(canvas, CARD_LINE_Y[0], &first, LINE);
    line(canvas, CARD_LINE_Y[1], &["Don't unplug", ""], PROBLEM);
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
        updated: false,
    };

    fn lit(c: &Canvas) -> usize {
        c.as_slice().iter().filter(|p| **p != Rgb::BLACK).count()
    }

    /// The name's panel pixels: what `text::draw_text_with` draws in the
    /// big font with its top-left at `(x0, y0)`, "Open" warm white and
    /// "DMD" amber.
    fn wordmark_at(x0: i32, y0: i32) -> Canvas {
        let mut c = Canvas::new();
        text::draw_text_with(&mut c, x0, y0, "Open", WHITE, Font::Normal, 2);
        let dx = 4 * Font::Normal.advance() as i32 * 2;
        text::draw_text_with(&mut c, x0 + dx, y0, "DMD", AMBER, Font::Normal, 2);
        c
    }

    fn wordmark(y0: i32) -> Canvas {
        wordmark_at(LOGO_X, y0)
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
                .inspect(|(a, b)| assert!(a.b > b.b && (**b == AMBER || **b == WHITE)))
                .count();
            assert!(lighter > 20, "{len:?}: {lighter} glinting");
        }
    }

    #[test]
    fn the_wordmark_is_what_text_draws() {
        // "OpenDMD" in the big font, two colours, centred: the same pixels
        // as the card's name, moved (the site's wordmark is cut from the
        // card).
        let y = (HEIGHT as i32 - LOGO_H) / 2;
        let w = text::text_width(Font::Normal, "OpenDMD", 2) as i32;
        let site = wordmark_at((WIDTH as i32 - w) / 2, y);
        let mut c = Canvas::new();
        card(&mut c, 0, &Info::default(), None);
        assert_eq!(band(&site, y), band(&c, CARD_Y));
        assert_eq!(lit(&site), lit(&c));
    }

    #[test]
    fn open_is_warm_white_and_dmd_amber() {
        // The settled name (show and card) in exactly two colours: every
        // lit pixel left of the D warm white, from the D on amber.
        assert_eq!(WHITE, Rgb::new(255, 214, 150));
        let split = LOGO_X + OPEN * Font::Normal.advance() as i32 * SCALE;
        let mut show_c = Canvas::new();
        show(&mut show_c, ShowLength::Short, SHORT.glint + 1_000, &INFO);
        let mut card_c = Canvas::new();
        card(&mut card_c, 0, &INFO, None);
        for (c, y0) in [(&show_c, SHOW_Y), (&card_c, CARD_Y)] {
            let (mut white, mut amber) = (0, 0);
            for y in y0..y0 + LOGO_H {
                for x in 0..WIDTH as i32 {
                    match c.get(x as usize, y as usize) {
                        Rgb::BLACK => {}
                        p if x < split => {
                            assert_eq!(p, WHITE, "({x}, {y})");
                            white += 1;
                        }
                        p => {
                            assert_eq!(p, AMBER, "({x}, {y})");
                            amber += 1;
                        }
                    }
                }
            }
            assert!(white > 50 && amber > 50, "{white} white, {amber} amber");
        }
        // Each part warms up through its own hot peak, bluer than its colour.
        assert_eq!(warm(0, 300, 300), WHITE);
        assert_eq!(warm(OPEN * 5, 300, 300), AMBER);
        assert!(warm(0, 100, 300).b > WHITE.b && warm(OPEN * 5, 100, 300).b > AMBER.b);
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

    #[test]
    fn an_updated_boot_says_so_for_three_seconds() {
        assert_eq!(
            updated_line("1.0.0-beta.17"),
            ["Updated to v", "1.0.0-beta.17"]
        );
        // A development build's long version: the part after its first '-'.
        assert_eq!(
            updated_line("1.0.0-beta.16-us-1a2b3c"),
            ["Updated to ", "beta.16-us-1a2b3c"]
        );
        assert_eq!(
            updated_line("1.0.0-a-very-long-pre-release-name"),
            ["Updated", ""]
        );
        assert_eq!(updated_line("1.0.0"), ["Updated to v", "1.0.0"]);
        for v in [
            "1.0.0-beta.17",
            "1.0.0-beta.16-us-1a2b3c",
            "10.20.30-rc.123+abc",
        ] {
            assert!(parts_width(&updated_line(v)) <= WIDTH as i32, "{v}");
        }
        assert_eq!(ShowLength::Short.min_ms_updated(), 900 + UPDATED_MS);
        assert_eq!(ShowLength::Full.min_ms_updated(), 3_850 + UPDATED_MS);
        assert_eq!(ShowLength::Off.for_update(), ShowLength::Short);
        assert_eq!(ShowLength::Full.for_update(), ShowLength::Full);
        let b = beats(ShowLength::Short);
        let up = Info {
            version: "1.0.0-beta.17",
            updated: true,
            ..INFO
        };
        assert_eq!(status(b, b.status - 10, &up).1, 0);
        // Up from the status beat, and still there where an ordinary boot
        // has long moved on to the address, until UPDATED_MS.
        for t in [
            b.status + 400,
            b.status + 1_500,
            b.status + UPDATED_MS - 200,
        ] {
            assert_eq!(
                status(b, t, &up),
                (["Updated to v", "1.0.0-beta.17"], 256),
                "{t}"
            );
        }
        assert_eq!(
            status(b, b.status + UPDATED_MS + 400, &up).0,
            ["192.168.1.69", ""]
        );
        // The virtual file plays it for its longer minimum.
        use crate::synth::{self, SynthSource};
        let mut src = SynthSource::new(synth::open("@boot?v=1.0.0-beta.17&up=1").unwrap()).unwrap();
        let mut c = Canvas::new();
        let mut n = 0;
        while src.next_frame(&mut c).unwrap().is_some() {
            n += 1;
        }
        assert_eq!(n, ShowLength::Short.min_ms_updated() / 10);
        let mut want = Canvas::new();
        show(
            &mut want,
            ShowLength::Short,
            (n - 1) * 10,
            &Info { ip: "", ..up },
        );
        assert_eq!(c, want);
    }

    #[test]
    fn which_updaters_light_the_panel() {
        assert_eq!(updater_number("updater 1"), Some(1));
        assert_eq!(updater_number("updater 12"), Some(12));
        for bad in ["", "updater", "updater ", "updater x", "updater -1", "1", "Updater 2", "updater 2a"] {
            assert_eq!(updater_number(bad), None, "{bad:?}");
        }
        assert!(!updater_lights_panel(None));
        assert!(!updater_lights_panel(Some("updater 1")));
        assert!(!updater_lights_panel(Some("garbage")));
        assert!(updater_lights_panel(Some("updater 2")));
        assert!(updater_lights_panel(Some("updater 3")));
        // The updater this tree builds is one that does.
        let defaults = include_str!("../../ota/updater/sdkconfig.defaults");
        let ver = defaults
            .lines()
            .find_map(|l| l.strip_prefix("CONFIG_APP_PROJECT_VER="))
            .map(|v| v.trim_matches('"'))
            .expect("the updater's version");
        assert!(updater_lights_panel(Some(ver)), "{ver}");
        // The two Installing screens differ only in the middle line.
        let (mut lit, mut dark) = (Canvas::new(), Canvas::new());
        update_notice(&mut lit, UpdateStage::Installing, "");
        update_notice(&mut dark, UpdateStage::InstallingDark, "");
        for y in (0..18).chain(24..HEIGHT) {
            for x in 0..WIDTH {
                assert_eq!(lit.get(x, y), dark.get(x, y), "{x},{y}");
            }
        }
        assert_ne!(lit, dark);
    }

    #[test]
    fn the_update_message_fits_and_is_still() {
        for (stage, v) in [
            (UpdateStage::Receiving, "1.0.0-beta.17"),
            (UpdateStage::Receiving, "1.0.0-beta.16-us-1a2b3c"),
            (UpdateStage::Receiving, ""),
            (UpdateStage::Installing, "1.0.0-beta.17"),
            (UpdateStage::InstallingDark, "1.0.0-beta.17"),
        ] {
            let mut a = Canvas::new();
            update_notice(&mut a, stage, v);
            let mut b = Canvas::new();
            b.fill(Rgb::new(9, 9, 9));
            update_notice(&mut b, stage, v);
            assert_eq!(a, b, "{stage:?} {v}: not the same over another picture");
            // Nothing in the outermost columns: every line fits.
            for y in 0..HEIGHT {
                assert_eq!(a.get(0, y), Rgb::BLACK, "{stage:?} {v} row {y}");
                assert_eq!(a.get(WIDTH - 1, y), Rgb::BLACK, "{stage:?} {v} row {y}");
            }
            // The title in amber, "Don't unplug" in warm white at the bottom.
            assert!((CARD_Y as usize..CARD_Y as usize + 14)
                .any(|y| (0..WIDTH).any(|x| a.get(x, y) == AMBER)));
            assert!((25..31).any(|y| (0..WIDTH).any(|x| a.get(x, y) == PROBLEM)));
            let middle = (18..24).any(|y| (0..WIDTH).any(|x| a.get(x, y) == LINE));
            assert_eq!(
                middle,
                !v.is_empty() || stage != UpdateStage::Receiving,
                "{stage:?} {v}"
            );
        }
        for t in ["Updating", "Installing"] {
            assert!(text::text_width(Font::Normal, t, 2) < WIDTH, "{t}");
        }
        assert_eq!(
            fitted("to v", "to ", "", "1.0.0-beta.17"),
            ["to v", "1.0.0-beta.17"]
        );
        // The virtual file draws the same picture.
        use crate::synth::{self, SynthSource};
        let mut src =
            SynthSource::new(synth::open("@boot?len=installing&v=1.0.0-beta.17").unwrap()).unwrap();
        let mut c = Canvas::new();
        assert!(src.next_frame(&mut c).unwrap().is_some());
        let mut want = Canvas::new();
        update_notice(&mut want, UpdateStage::Installing, "1.0.0-beta.17");
        assert_eq!(c, want);
    }

    /// `s` as a query value (what the web app's URLSearchParams sends).
    #[cfg(feature = "psi")]
    fn enc(s: &str) -> String {
        s.bytes()
            .map(|b| match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' => (b as char).to_string(),
                _ => format!("%{b:02X}"),
            })
            .collect()
    }

    /// Every frame of `@boot?…` as played.
    #[cfg(feature = "psi")]
    fn frames_of(path: &str) -> Vec<Canvas> {
        use crate::synth::{self, SynthSource};
        let mut src = SynthSource::new(synth::open(path).unwrap()).unwrap();
        let mut c = Canvas::new();
        let mut out = Vec::new();
        while let Some(d) = src.next_frame(&mut c).unwrap() {
            assert_eq!(d, Duration::from_millis(10));
            out.push(c.clone());
        }
        out
    }

    /// A Formula as the start-up show: `@boot?formula=` draws, frame for
    /// frame, what the panel's start-up draws (`FormulaShow::frame` at the
    /// show's times), the Formula with the line over it only when there is
    /// something to say, and runs as long as the wordmark would.
    #[cfg(feature = "psi")]
    #[test]
    fn a_formula_as_the_show_draws_what_the_panel_draws() {
        crate::synth::install_test_bundled();
        // (Reads no seed: two opens draw the same.)
        let inline = crate::fx_psi::path("loop 2\nfps 50\nrgb(frac(x + t/2), y, 0.5)", "");
        for (formula, len, text, updated) in [
            ("@formula:red", ShowLength::Short, "&ap=OpenDMD-Setup", false),
            (inline.as_str(), ShowLength::Short, "&ip=192.168.1.69", false),
            (inline.as_str(), ShowLength::Full, "&note=Insert%20a%20microSD%20card", true),
            (inline.as_str(), ShowLength::Short, "", false),
        ] {
            let path = format!(
                "@boot?len={}&v=1.0.1{text}{}&formula={}",
                len.name(),
                if updated { "&up=1" } else { "" },
                enc(formula)
            );
            let got = frames_of(&path);
            let want_n = if updated { len.min_ms_updated() } else { len.min_ms() } / 10;
            assert_eq!(got.len(), want_n as usize, "{path}");
            let info = Info {
                version: "1.0.1",
                ip: if text.contains("ip=") { "192.168.1.69" } else { "" },
                ap: if text.contains("ap=") { "OpenDMD-Setup" } else { "" },
                note: if text.contains("note=") { "Insert a microSD card" } else { "" },
                updated,
            };
            let mut show = FormulaShow::open(formula).unwrap();
            let mut plain = FormulaShow::open(formula).unwrap();
            let (mut c, mut bare) = (Canvas::new(), Canvas::new());
            let none = Info { version: "1.0.1", ..Info::default() };
            for (i, g) in got.iter().enumerate() {
                let t = i as u32 * 10;
                show.frame(&mut c, len, t, &info);
                plain.frame(&mut bare, len, t, &none);
                assert_eq!(g.as_bytes(), c.as_bytes(), "{path} at {t} ms");
                // Before the status beat the Formula alone; with nothing
                // to say, never anything else (no version line).
                let lined = g.as_bytes() != bare.as_bytes();
                let b = beats(len);
                if t < b.status || (text.is_empty() && !updated) {
                    assert!(!lined, "{path}: a line at {t} ms");
                } else if t >= b.status + 300 && (!updated || t + 150 < b.status + UPDATED_MS) {
                    assert!(lined, "{path}: no line at {t} ms");
                }
            }
        }
    }

    /// A value that names no Formula that builds plays the wordmark, as the
    /// panel falls back to it; one too heavy for the show is refused.
    #[cfg(feature = "psi")]
    #[test]
    fn a_formula_that_does_not_build_plays_the_wordmark() {
        crate::synth::install_test_bundled();
        let wordmark = frames_of("@boot?len=short&v=1.0.1&ip=192.168.1.69");
        let heavy = {
            let terms: Vec<String> = (1..=14)
                .map(|k| format!("step(0.5, frac(x*{} + y*{} + t*{}))*{}", k + 2, (k * 7) % 13 + 1, k % 5 + 1, k % 3 + 1))
                .collect();
            crate::fx_psi::path(&format!("loop 4\ng = {}\npal(rainbow, frac(g/40))", terms.join(" + ")), "")
        };
        for bad in ["@formula:nope", "@formula?psb=AAAA", "/formulas/mine.psi", "/gifs/a.gif", heavy.as_str()] {
            let got = frames_of(&format!("@boot?len=short&v=1.0.1&ip=192.168.1.69&formula={}", enc(bad)));
            assert!(got == wordmark, "{bad}: not the wordmark");
            assert!(FormulaShow::open(bad).is_err(), "{bad}");
        }
        let e = FormulaShow::open(&heavy).unwrap_err();
        assert!(e.msg.starts_with("this Formula is too heavy for the start-up show (about "), "{}", e.msg);
    }

    /// Short of memory, a Formula for the show is refused with
    /// `psi::NO_MEMORY`, which the firmware tries again a second later
    /// (`fx_psi::look_retry`); then it builds.
    #[cfg(feature = "psi")]
    #[test]
    fn a_formula_short_of_memory_is_tried_again() {
        crate::synth::install_test_bundled();
        crate::fx_psi::refuse_loads(1);
        let e = FormulaShow::open("@formula:red").unwrap_err();
        assert_eq!(e.msg, crate::psi::NO_MEMORY);
        assert!(crate::fx_psi::look_retry(&e).is_some());
        assert!(FormulaShow::open("@formula:red").is_ok());
        // A real error is not tried again.
        let e = FormulaShow::open("@formula:nope").unwrap_err();
        assert!(crate::fx_psi::look_retry(&e).is_none(), "{}", e.msg);
    }

    /// `synth::boot_line` (the web app's preview) draws the line as the
    /// show does over the same frame.
    #[cfg(feature = "psi")]
    #[test]
    fn the_line_alone_is_the_shows() {
        crate::synth::install_test_bundled();
        let info = Info { version: "1.0.1", ap: "OpenDMD-Setup", ..Info::default() };
        let mut show = FormulaShow::open("@formula:red").unwrap();
        let mut bare = FormulaShow::open("@formula:red").unwrap();
        for t in [0, 900, 1_000, 1_250] {
            let (mut a, mut b) = (Canvas::new(), Canvas::new());
            show.frame(&mut a, ShowLength::Short, t, &info);
            bare.frame(&mut b, ShowLength::Short, t, &Info::default());
            crate::synth::boot_line(&mut b, "@boot?len=short&v=1.0.1&ap=OpenDMD-Setup&formula=x", t);
            assert_eq!(a, b, "at {t} ms");
        }
    }

    /// Deleting a card Formula clears a setting that names it (the look and
    /// the start-up show alike); writing its `.psb` rebuilds; other files
    /// and other values are left alone.
    #[test]
    fn a_deleted_formula_clears_the_setting_that_names_it() {
        use crate::synth::{file_change, FileChange};
        let v = "/formulas/My-Show.psi?a=0.5";
        assert_eq!(file_change(v, "/formulas/my-show.psi", true), Some(FileChange::Gone));
        assert_eq!(file_change(v, "/formulas/my-show.psi", false), None);
        assert_eq!(file_change(v, "/formulas/MY-SHOW.PSB", false), Some(FileChange::Rebuild));
        assert_eq!(file_change(v, "/formulas/my-show.psb", true), Some(FileChange::Rebuild));
        assert_eq!(file_change(v, "/formulas/other.psi", true), None);
        for other in ["", "@formula:starfield", "@formula?psb=AAAA"] {
            assert_eq!(file_change(other, "/formulas/my-show.psi", true), None, "{other}");
        }
    }
}

//! The startup sequence ("boot show") and the status card.
//!
//! **The show** plays on the panel from the moment it lights until the
//! first GIF can take over: a warp (stars from a hash, as the starfield
//! effect's, accelerating from a drift into hyperspace streaks) that jumps
//! into the time-warp tunnel ([`crate::fx::Tunnel`], chrono palette) with a
//! flash, the name "OpenDMD" (the 5x7 font, sampled at any scale) flying out
//! of the tunnel's vanishing point to three times its size, overshooting a
//! little and settling, a glint across it, and a status line in the small
//! font: the version first, then the most useful thing there is to say (the
//! WiFi network to join in setup mode, a card problem in plain words, the
//! address once WiFi is up). [`ShowLength::Short`] is 1.3 s (a sting: the
//! boot it covers takes ~0.4 s with a current playlist index), `Full` 5.6 s
//! (a longer warp and a slower arrival); either holds its last beat (the
//! tunnel dimmed behind the name, a glint every 2.4 s) for as long as the
//! card takes. The firmware ends it at `max(minimum, card ready)`, and
//! after [`CAP_MS`] shows the card instead.
//!
//! **The status card** ([`card`]) is what the panel shows whenever there is
//! no card to play from: the name at twice its size over a slow, dim star
//! drift, and two lines — the problem in plain words (amber) and where to
//! go (the address, or the setup network to join). Calm and looping
//! ([`CARD_LOOP_MS`]).
//!
//! **Pure functions of time.** A frame is a function of the time since the
//! scene began and the [`Info`] strings: nothing is remembered between
//! frames, nothing is allocated (the text is drawn from up to two string
//! slices), and every table is the effects' (`fx`: the sine, the palettes,
//! the tunnel's). So the firmware's show, the simulator and the browser
//! draw the same pixels: the virtual file [`PATH`]
//! (`@boot[?len=short|full|card&v=<version>&ip=<address>&ap=<network>&note=<text>]`,
//! parsed by [`crate::synth`]) plays the show once (or the card for one
//! loop) at the effects' frame rate.

use crate::canvas::{Canvas, Rgb, HEIGHT, WIDTH};
use crate::fx::{self, Common, Effect, Palette, Texture, Tunnel};
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
/// The status card's loop (the stars' pass through the field; the glint's
/// period divides it).
pub const CARD_LOOP_MS: u32 = 16_384;
/// The card's arrival from the show: the name shrinking into place, the
/// tunnel going out, the text coming up.
pub const INTRO_MS: u32 = 700;

/// The show's timeline, in ms.
struct Beats {
    /// The jump: the tunnel fully in, the flash at its peak.
    warp: u32,
    /// The name leaves the vanishing point, and has settled.
    logo0: u32,
    logo1: u32,
    /// The status line comes up.
    status: u32,
    /// The shortest show.
    min: u32,
    stars: u32,
}

const SHORT: Beats = Beats {
    warp: 600,
    logo0: 480,
    logo1: 1_100,
    status: 950,
    min: 1_300,
    stars: 160,
};

const FULL: Beats = Beats {
    warp: 2_800,
    logo0: 2_650,
    logo1: 4_400,
    status: 4_200,
    min: 5_600,
    stars: 240,
};

fn beats(len: ShowLength) -> &'static Beats {
    match len {
        ShowLength::Full => &FULL,
        _ => &SHORT,
    }
}

/// The show's beat at `t` (for the firmware's log): `warp`, `name` (it
/// leaves the vanishing point), `jump` (into the tunnel), `status line`,
/// `name settled`, then `hold` from the minimum on.
pub fn phase(len: ShowLength, t: u32) -> &'static str {
    // (In this order in both timelines: `phases_in_order` checks.)
    let b = beats(len);
    [
        (b.min, "hold"),
        (b.logo1, "name settled"),
        (b.status, "status line"),
        (b.warp, "jump"),
        (b.logo0, "name"),
    ]
    .iter()
    .find(|&&(at, _)| t >= at)
    .map_or("warp", |&(_, name)| name)
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

// --- the pieces -------------------------------------------------------------

/// The tunnel at `t` of its own time, its light at `level` / 256.
fn tunnel(canvas: &mut Canvas, t: u32, level: u32) {
    let mut tunnel = Tunnel {
        common: Common {
            speed: 2,
            delay_ms: 10,
            palette: Palette::Chrono,
            cycle: true,
        },
        twist: 1,
        texture: Texture::Checker,
        drift: false,
        layers: 2,
    };
    tunnel.frame(Duration::from_millis(u64::from(t)), canvas);
    if level < 256 {
        for b in canvas.as_mut_bytes() {
            *b = ((u32::from(*b) * level) >> 8) as u8;
        }
    }
}

/// The tunnel's own time at `t` of the show: twice real time in the warp,
/// easing to real time over the 700 ms before the name settles.
fn tunnel_time(b: &Beats, t: u32) -> u32 {
    const D: u32 = 700;
    let a = b.logo1 - D;
    if t <= a {
        2 * t
    } else if t <= b.logo1 {
        let d = t - a;
        2 * a + 2 * d - d * d / (2 * D)
    } else {
        2 * a + 2 * D - D / 2 + (t - b.logo1)
    }
}

/// The tunnel's light in the show: in over the 400 ms to the jump, dimmed to
/// 90 / 256 behind the name over the last two thirds of its flight.
fn tunnel_level(b: &Beats, t: u32) -> u32 {
    let up = ramp(t, b.warp - 400, b.warp);
    let dim = (b.logo1 - b.logo0) * 2 / 3;
    up.saturating_sub((256 - 90) * ramp(t, b.logo1 - dim, b.logo1) / 256)
}

/// Depth units of the stars' field.
const DEPTH: i32 = 4096;

/// Star `k`'s place: x and y spread, and its depth at travel 0.
fn star(k: u32) -> (i32, i32, i32) {
    let h = fx::hash(k.wrapping_mul(0x9e37_79b9) ^ 0xb007);
    (
        (h & 4095) as i32 - 2048,
        ((h >> 12) & 1023) as i32 - 512,
        (fx::hash(h) & 4095) as i32,
    )
}

fn project(sx: i32, sy: i32, z: i32) -> (i32, i32) {
    let zz = (z >> 4) + 4;
    (
        WIDTH as i32 / 2 + sx * 2 / zz,
        HEIGHT as i32 / 2 + sy * 2 / zz,
    )
}

/// `n` stars at `travel` depth units in, each a streak `len` units long,
/// coloured by nearness from the chrono palette's first 24 entries (black,
/// purple, blue, white) at `level` / 256.
fn stars(canvas: &mut Canvas, n: u32, travel: i32, len: i32, level: u32) {
    let pal = Palette::Chrono.table();
    for k in 0..n {
        let (sx, sy, z0) = star(k);
        let z = (z0 - travel).rem_euclid(DEPTH);
        let head = project(sx, sy, z);
        if !(0..WIDTH as i32).contains(&head.0) || !(0..HEIGHT as i32).contains(&head.1) {
            continue;
        }
        let c = pal[(((DEPTH - 1 - z) * 24) >> 12) as usize];
        if c == Rgb::BLACK {
            continue;
        }
        let tail = project(sx, sy, (z + len).min(DEPTH - 1));
        fx::line(
            canvas,
            (tail.0, tail.1, 40),
            (head.0, head.1, 256),
            fx::scale(c, level),
        );
    }
}

/// The warp's stars at `t`: a drift accelerating as the cube of time into
/// hyperspace (travel `0.6 t + 3 warp (t / warp)^4` depth units), streaks
/// growing with the speed, fading in from black and out into the jump.
fn warp_stars(canvas: &mut Canvas, b: &Beats, t: u32) {
    let tau = u64::from(t.min(b.warp + 300)) * 1024 / u64::from(b.warp);
    let travel = u64::from(t) * 6 / 10 + ((3 * u64::from(b.warp) * tau.pow(4)) >> 40);
    let len = (18 + ((360 * tau.pow(3)) >> 30)).min(1800) as i32;
    let level = ramp(t, 0, 200) * (256 - ramp(t, b.warp - 250, b.warp + 250)) / 256;
    if level > 0 {
        stars(canvas, b.stars, (travel % DEPTH as u64) as i32, len, level);
    }
}

/// The jump: a lavender-white flash over everything, up in 60 ms to the
/// warp beat and out over 200 ms.
fn flash(canvas: &mut Canvas, b: &Beats, t: u32) {
    let f = if t + 60 <= b.warp || t >= b.warp + 200 {
        return;
    } else if t < b.warp {
        170 * (t + 60 - b.warp) / 60
    } else {
        170 * (b.warp + 200 - t) / 200
    };
    for px in canvas.as_mut_bytes().as_chunks_mut::<3>().0 {
        px[0] = px[0].saturating_add((f * 3 / 4) as u8);
        px[1] = px[1].saturating_add((f * 3 / 4) as u8);
        px[2] = px[2].saturating_add(f as u8);
    }
}

const LOGO: &[u8; 7] = b"OpenDMD";
/// The name's size in font pixels (5x7, 7 characters, no trailing gap).
const LOGO_W: i32 = 34;
const LOGO_H: i32 = 7;
/// The name's colour by font row: chrome, from pale blue through white to
/// violet.
const GRAD: [Rgb; 7] = [
    Rgb::new(190, 215, 255),
    Rgb::new(255, 255, 255),
    Rgb::new(225, 225, 255),
    Rgb::new(185, 170, 255),
    Rgb::new(150, 115, 255),
    Rgb::new(120, 75, 235),
    Rgb::new(95, 45, 205),
];

/// Where and how the name is drawn: its scale (Q8: 256 = one font pixel a
/// panel pixel), its centre (Q8 pixels), its light (/ 256), and the glint's
/// x (a band 6 px wide leaning with the rows; off the panel for none).
struct Logo {
    s: i32,
    cx: i32,
    cy: i32,
    level: u32,
    glint: i32,
}

impl Logo {
    /// The font pixel under panel pixel `(x, y)`: its row, if lit.
    fn at(&self, x: i32, y: i32) -> Option<usize> {
        let u = x * 256 + 128 - (self.cx - LOGO_W * self.s / 2);
        let v = y * 256 + 128 - (self.cy - LOGO_H * self.s / 2);
        if u < 0 || v < 0 {
            return None;
        }
        let (fu, fv) = (u / self.s, v / self.s);
        if fu >= LOGO_W || fv >= LOGO_H {
            return None;
        }
        let rows = Font::Normal.rows(LOGO[(fu / 5) as usize] as char);
        (rows[fv as usize] & (0x80 >> (fu % 5)) != 0).then_some(fv as usize)
    }

    fn draw(&self, canvas: &mut Canvas) {
        if self.s < 16 || self.level == 0 {
            return;
        }
        // The box, a pixel wider all round for the outline.
        let (hw, hh) = (LOGO_W * self.s / 2, LOGO_H * self.s / 2);
        let x0 = ((self.cx - hw) >> 8) - 1;
        let x1 = ((self.cx + hw) >> 8) + 1;
        let y0 = ((self.cy - hh) >> 8) - 1;
        let y1 = ((self.cy + hh) >> 8) + 1;
        let outline = self.s >= 256;
        for y in y0.max(0)..=y1.min(HEIGHT as i32 - 1) {
            for x in x0.max(0)..=x1.min(WIDTH as i32 - 1) {
                let c = match self.at(x, y) {
                    Some(row) => {
                        let mut c = GRAD[row];
                        let g = x - self.glint - (y - (self.cy >> 8)) / 2;
                        if (0..6).contains(&g) {
                            c = mix(c, Rgb::new(255, 255, 255), 170);
                        }
                        fx::scale(c, self.level)
                    }
                    // A dark rim, so the name reads over the tunnel.
                    None if outline
                        && [(1, 0), (-1, 0), (0, 1), (0, -1)]
                            .iter()
                            .any(|&(dx, dy)| self.at(x + dx, y + dy).is_some()) =>
                    {
                        Rgb::BLACK
                    }
                    None => continue,
                };
                canvas.set(x as usize, y as usize, c);
            }
        }
    }
}

/// `a` towards `b` by `k` / 256.
fn mix(a: Rgb, b: Rgb, k: u32) -> Rgb {
    let m = |x: u8, y: u8| ((u32::from(x) * (256 - k) + u32::from(y) * k) >> 8) as u8;
    Rgb::new(m(a.r, b.r), m(a.g, b.g), m(a.b, b.b))
}

/// The glint's x at `t` ms into its cycle of `period`: a 600 ms sweep from
/// off the left to off the right, then nothing.
fn glint(t: u32, period: u32) -> i32 {
    let p = t % period;
    if p < 600 {
        -20 + (p * 168 / 600) as i32
    } else {
        -1000
    }
}

/// The status line's colour, and the card's: lavender, amber (a problem),
/// pale blue.
const LAVENDER: Rgb = Rgb::new(176, 160, 255);
const AMBER: Rgb = Rgb::new(255, 176, 64);
const PALE: Rgb = Rgb::new(140, 170, 255);

fn parts_width(p: &Parts) -> i32 {
    let n = p[0].chars().count() + p[1].chars().count();
    (n as i32 * Font::Small.advance() as i32 - 1).max(0)
}

/// `p` in the small font, centred, its top at `y`, on a black box a pixel
/// bigger than the text.
fn line(canvas: &mut Canvas, y: i32, p: &Parts, colour: Rgb) {
    let w = parts_width(p);
    if w == 0 {
        return;
    }
    let x = (WIDTH as i32 - w) / 2;
    for yy in (y - 1).max(0)..(y + Font::Small.height() as i32).min(HEIGHT as i32) {
        for xx in (x - 1).max(0)..(x + w + 1).min(WIDTH as i32) {
            canvas.set(xx as usize, yy as usize, Rgb::BLACK);
        }
    }
    let w0 = text::draw_text_with(canvas, x, y, p[0], colour, Font::Small, 1);
    let gap = if p[0].is_empty() { 0 } else { 1 };
    text::draw_text_with(canvas, x + w0 as i32 + gap, y, p[1], colour, Font::Small, 1);
}

// --- the scenes ---------------------------------------------------------------

/// The show's frame at `t` ms (`len` `Off` draws the short one). Every
/// pixel is written and marked.
pub fn show(canvas: &mut Canvas, len: ShowLength, t: u32, info: &Info) {
    let b = beats(len);
    let level = tunnel_level(b, t);
    if level > 0 {
        tunnel(canvas, tunnel_time(b, t), level);
    } else {
        canvas.clear();
    }
    if t < b.warp + 250 {
        warp_stars(canvas, b, t);
    }
    flash(canvas, b, t);
    if t >= b.logo0 {
        // Out of the depth: the scale is 3 x (1 / z), z from 16 to 0.9 in
        // the first 85% of the flight; then back from the overshoot (1.11x)
        // to 3x, easing out.
        let u = ramp(t, b.logo0, b.logo1) as i32;
        let s = if u < 218 {
            let z = 4096 - (4096 - 230) * u / 218;
            768 * 256 / z
        } else {
            let e = (u - 218) * 256 / 38;
            let e = 256 - (256 - e) * (256 - e) / 256;
            855 - 87 * e / 256
        };
        // From the vanishing point (row 16) up to the name's place (its
        // top on row 2) as it grows; brighter as it nears.
        let near = s.min(768);
        Logo {
            s,
            cx: 64 * 256,
            cy: 16 * 256 - 896 * near / 768,
            level: (s as u32 / 2).min(256),
            glint: if t + 150 >= b.logo1 {
                glint(t + 150 - b.logo1, 2_400)
            } else {
                -1000
            },
        }
        .draw(canvas);
    }
    if t >= b.status {
        let fade = ramp(t, b.status, b.status + 250);
        let text = if t < b.status + 1_200 {
            info.version_line()
        } else {
            info.show_line()
        };
        line(canvas, 26, &text, fx::scale(LAVENDER, fade));
    }
    canvas.dirty_mut().mark_all();
}

/// The status card at `t` ms into it. `from_show`: it follows the show
/// (its length and the show's time when the card took over), so it
/// arrives from it ([`INTRO_MS`]) rather than cutting in. Every pixel is
/// written and marked.
pub fn card(canvas: &mut Canvas, t: u32, info: &Info, from_show: Option<(ShowLength, u32)>) {
    let arrive = match from_show {
        Some(_) => ramp(t, 0, INTRO_MS),
        None => 256,
    };
    match from_show {
        Some((len, at)) if arrive < 256 => {
            let b = beats(len);
            let level = tunnel_level(b, at + t) * (256 - arrive) / 256;
            tunnel(canvas, tunnel_time(b, at + t), level);
        }
        _ => canvas.clear(),
    }
    // A slow, dim drift: 64 stars, one pass through the field a loop.
    stars(canvas, 64, (t / 4) as i32, 64, 128 * arrive / 256);
    // The name: from the show's 3x at row 12.5 to 2x at row 8.
    let s = 768 - 256 * arrive as i32 / 256;
    Logo {
        s,
        cx: 64 * 256,
        cy: 3200 - 1152 * arrive as i32 / 256,
        level: 256,
        glint: glint(t % 4_096, 4_096),
    }
    .draw(canvas);
    let (first, second) = info.card_lines();
    let c1 = if info.note.is_empty() {
        LAVENDER
    } else {
        AMBER
    };
    line(canvas, 17, &first, fx::scale(c1, arrive));
    line(canvas, 25, &second, fx::scale(PALE, arrive));
    canvas.dirty_mut().mark_all();
}

#[cfg(test)]
mod tests {
    use super::*;

    const INFO: Info = Info {
        version: "0.3.0",
        ip: "192.168.1.69",
        ap: "",
        note: "",
    };

    fn lit(c: &Canvas) -> usize {
        c.as_slice().iter().filter(|p| **p != Rgb::BLACK).count()
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
                [
                    "warp",
                    "name",
                    "jump",
                    "status line",
                    "name settled",
                    "hold"
                ],
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
    fn every_beat_draws_something_and_is_repeatable() {
        for len in [ShowLength::Short, ShowLength::Full] {
            for t in (0..=len.min_ms() + 3_000).step_by(50) {
                let mut a = Canvas::new();
                show(&mut a, len, t, &INFO);
                // Pure: the same picture over whatever was there.
                let mut b = Canvas::new();
                b.fill(Rgb::new(9, 9, 9));
                show(&mut b, len, t, &INFO);
                assert_eq!(a, b, "{len:?} at {t} ms");
                if t >= 100 {
                    assert!(lit(&a) > 20, "{len:?} at {t} ms: {} lit", lit(&a));
                }
            }
        }
    }

    #[test]
    fn the_name_settles_at_exactly_three_times_the_font() {
        // At the end of the flight, between glints, the name is the 5x7
        // font at scale 3 with its top-left at (13, 2): what
        // `text::draw_text_with` draws there.
        let b = beats(ShowLength::Short);
        let t = b.logo1 + 1_000;
        let mut shown = Canvas::new();
        show(&mut shown, ShowLength::Short, t, &INFO);
        let mut want = Canvas::new();
        text::draw_text_with(
            &mut want,
            13,
            2,
            "OpenDMD",
            Rgb::new(1, 1, 1),
            Font::Normal,
            3,
        );
        let logo = Logo {
            s: 768,
            cx: 64 * 256,
            cy: 3200,
            level: 256,
            glint: -1000,
        };
        for y in 0..HEIGHT {
            for x in 0..WIDTH {
                let drawn = want.get(x, y) != Rgb::BLACK;
                assert_eq!(logo.at(x as i32, y as i32).is_some(), drawn, "({x}, {y})");
                if drawn {
                    assert!(GRAD.contains(&shown.get(x, y)), "({x}, {y})");
                }
            }
        }
    }

    #[test]
    fn the_status_line_says_the_version_then_what_matters() {
        let at = |info: &Info, t: u32| {
            let mut c = Canvas::new();
            show(&mut c, ShowLength::Short, t, info);
            (21..HEIGHT)
                .flat_map(|y| (0..WIDTH).map(move |x| (x, y)))
                .filter(|&(x, y)| c.get(x, y) != Rgb::BLACK && y >= 25)
                .count()
        };
        let b = beats(ShowLength::Short);
        let quiet = Info { ip: "", ..INFO };
        // Up (the version, on its black box).
        assert!(at(&INFO, b.status + 400) > 0);
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
        // A loop later, the same picture (after the arrival).
        assert_eq!(draw(1_000, None), draw(1_000 + CARD_LOOP_MS, None));
        assert!(lit(&draw(0, None)) > 100);
        // Arriving: the show's tunnel still there at first, gone after.
        let from = Some((ShowLength::Short, CAP_MS));
        assert!(
            lit(&draw(0, from)) > lit(&draw(0, None)) + 1_000,
            "{}",
            lit(&draw(0, from))
        );
        assert_eq!(draw(INTRO_MS, from), draw(INTRO_MS, None));
        // The amber line is there.
        let c = draw(2_000, None);
        assert!((17..23).any(|y| (0..WIDTH).any(|x| c.get(x, y) == AMBER)));
        assert!((25..31).any(|y| (0..WIDTH).any(|x| c.get(x, y) == PALE)));
    }
}

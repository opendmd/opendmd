//! Virtual files: pictures the engine plays like a GIF but that are drawn,
//! not read — the power-on effect and title cards.
//!
//! A virtual file has a path starting with `@` ([`POWER_ON`], or
//! [`card_path`]`(title, subtitle)` = `@card:<title>|<subtitle>`). The engine
//! asks [`open`] before its [`Source`](crate::engine::Source): for a virtual
//! path it gets a few bytes in memory — the magic [`MAGIC`] and the path —
//! which [`FrameSource`](crate::player::FrameSource) sniffs like any file
//! and hands to [`SynthSource`]. So everything a file gets, a virtual file
//! gets too: a transition in and out, the dwell policy (the sequence plays
//! them with `loops=1`), `now_playing` and its position in `/api/status`,
//! and the browser preview, which plays the same path through the same core
//! (it has no bytes to fetch). Nothing touches the card and nothing is
//! allocated beyond the path.
//!
//! This is how the "overlay" for title cards and the power-on effect is
//! realised: rather than a second layer composited over the decoder's
//! output (which would need its own 12 KB canvas and its own timing), the
//! card *is* the file on screen for its dwell, and the compositor's
//! transition is what joins it to the files either side.

use crate::canvas::{Canvas, Rgb, HEIGHT, WIDTH};
use crate::text;
use std::io::{self, Cursor, Read};
use std::time::Duration;

/// First four bytes of a virtual file's in-memory stream.
pub const MAGIC: [u8; 4] = *b"ODMV";
/// The power-on effect's path.
pub const POWER_ON: &str = "@power-on";
/// The prefix of a title card's path.
pub const CARD_PREFIX: &str = "@card:";
/// Frames in the power-on effect, [`POWER_ON_FRAME`] apart: 700 ms.
pub const POWER_ON_FRAMES: u32 = 35;
pub const POWER_ON_FRAME: Duration = Duration::from_millis(20);
/// How long a title card holds its one frame (one loop).
pub const CARD_DWELL: Duration = Duration::from_millis(3000);
/// A virtual path longer than this is refused (it is only ever built here).
const MAX_PATH: usize = 512;

/// Is `path` a virtual file?
pub fn is_virtual(path: &str) -> bool {
    path.starts_with('@')
}

/// The path of a title card with these two lines (`subtitle` may be
/// empty). `|` and control characters in either become spaces.
pub fn card_path(title: &str, subtitle: &str) -> String {
    let clean = |s: &str| -> String {
        s.chars()
            .map(|c| if c == '|' || c.is_control() { ' ' } else { c })
            .collect::<String>()
            .trim()
            .to_string()
    };
    let mut p = format!("{CARD_PREFIX}{}|{}", clean(title), clean(subtitle));
    if p.len() > MAX_PATH {
        let mut cut = MAX_PATH;
        while !p.is_char_boundary(cut) {
            cut -= 1;
        }
        p.truncate(cut);
    }
    p
}

/// What a virtual path shows, for `now_playing.title`: a card's title, or
/// "Power on". `None` for anything else.
pub fn title(path: &str) -> Option<String> {
    match parse(path)? {
        Spec::PowerOn => Some("Power on".to_string()),
        Spec::Card { title, .. } => Some(title),
    }
}

/// The in-memory stream for a virtual `path`, or `None` if it is not one
/// (the caller opens it from its source as usual). An `@` path this build
/// does not know is still returned, and fails to open as `InvalidData`.
pub fn open(path: &str) -> Option<Cursor<Vec<u8>>> {
    if !is_virtual(path) {
        return None;
    }
    let mut bytes = Vec::with_capacity(4 + path.len());
    bytes.extend_from_slice(&MAGIC);
    bytes.extend_from_slice(path.as_bytes());
    Some(Cursor::new(bytes))
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Spec {
    PowerOn,
    Card { title: String, subtitle: String },
}

fn parse(path: &str) -> Option<Spec> {
    if path == POWER_ON {
        return Some(Spec::PowerOn);
    }
    let rest = path.strip_prefix(CARD_PREFIX)?;
    let (title, subtitle) = rest.split_once('|').unwrap_or((rest, ""));
    Some(Spec::Card {
        title: title.to_string(),
        subtitle: subtitle.to_string(),
    })
}

/// The decoder for a virtual file: draws its frames into the canvas under
/// the same contract as [`GifSource`](crate::player::GifSource) — the
/// canvas is cleared by the first frame, only changed pixels are written and
/// marked, `Ok(None)` at the end of the stream.
pub struct SynthSource<R: Read> {
    reader: R,
    spec: Spec,
    frames: u64,
    /// The power-on effect's ignition times, while it plays.
    table: Option<Vec<u8>>,
}

impl<R: Read> SynthSource<R> {
    /// `reader` is the whole stream, magic included (as `FrameSource`
    /// hands it over).
    pub fn new(mut reader: R) -> io::Result<Self> {
        let mut raw = Vec::new();
        (&mut reader)
            .take(MAX_PATH as u64 + 4)
            .read_to_end(&mut raw)?;
        let spec = raw
            .strip_prefix(&MAGIC)
            .and_then(|p| std::str::from_utf8(p).ok())
            .and_then(parse)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "unknown virtual file"))?;
        Ok(SynthSource {
            reader,
            spec,
            frames: 0,
            table: None,
        })
    }

    pub fn into_inner(self) -> R {
        self.reader
    }

    pub fn frames_decoded(&self) -> u64 {
        self.frames
    }

    pub fn logical_size(&self) -> (usize, usize) {
        (WIDTH, HEIGHT)
    }

    pub fn next_frame(&mut self, canvas: &mut Canvas) -> io::Result<Option<Duration>> {
        let delay = match &self.spec {
            Spec::PowerOn => {
                if self.frames >= u64::from(POWER_ON_FRAMES) {
                    self.table = None;
                    return Ok(None);
                }
                if self.frames == 0 {
                    canvas.clear();
                    self.table = ignition_table();
                }
                power_on_frame(self.frames as u32, canvas, self.table.as_deref());
                POWER_ON_FRAME
            }
            Spec::Card { title, subtitle } => {
                if self.frames >= 1 {
                    return Ok(None);
                }
                text::title_card(canvas, title, subtitle);
                CARD_DWELL
            }
        };
        self.frames += 1;
        Ok(Some(delay))
    }
}

/// When (ms into the effect) each dot ignites spreads outward from the
/// centre over this long...
const SPREAD_MS: f32 = 400.0;
/// ...then everything fades to black over the last this long.
const FADE_MS: f32 = 180.0;

/// Per-pixel jitter in the ignition time (0..=`JITTER_MS`), so the front is
/// a scatter of dots rather than a clean ring.
const JITTER_MS: f32 = 40.0;

/// Ignition times are kept in these units (ms): 440 ms at most fits a `u8`.
const TICK_MS: u32 = 4;
/// Ages the colour table covers, in [`TICK_MS`] units (the whole effect).
const AGES: usize = 176;

/// When the dot at `(x, y)` ignites, in [`TICK_MS`] units from the start:
/// an ease-out spread over an ellipse fitted to the 4:1 panel, plus a
/// per-pixel jitter.
fn ignite(x: usize, y: usize) -> u8 {
    let (cx, cy) = ((WIDTH as f32 - 1.0) / 2.0, (HEIGHT as f32 - 1.0) / 2.0);
    // The corner is at radius sqrt(2): scale so it ignites just inside the
    // spread.
    let reach = std::f32::consts::SQRT_2 * 1.02;
    let dx = (x as f32 - cx) / (WIDTH as f32 / 2.0);
    let dy = (y as f32 - cy) / (HEIGHT as f32 / 2.0);
    let d = (dx * dx + dy * dy).sqrt() / reach;
    // r(u) = 1 - (1 - u)^2, inverted: the fraction of the spread at which
    // radius d is reached.
    let u = 1.0 - (1.0 - d.min(1.0)).sqrt();
    let jitter = (crate::compositor::dissolve_rank(y * WIDTH + x) % 64) as f32 / 63.0 * JITTER_MS;
    ((u * SPREAD_MS + jitter) / TICK_MS as f32).round() as u8
}

/// Every pixel's [`ignite`] time, computed once per play (4 KB, for the
/// effect's 700 ms): the per-pixel square roots cost ~40 ms a frame on the
/// ESP32, twice the frame. `None` if the heap will not give 4 KB — then each
/// frame computes them again, slowly but correctly.
fn ignition_table() -> Option<Vec<u8>> {
    let mut v = Vec::new();
    v.try_reserve_exact(WIDTH * HEIGHT).ok()?;
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            v.push(ignite(x, y));
        }
    }
    Some(v)
}

/// Frame `f` of the power-on effect: dots ignite outward from the centre
/// ([`ignite`]), each flashing white-hot and settling to amber (the
/// overshoot), and the whole field fades to black at the end, so the next
/// file transitions in from a dark panel. A pure function of `(f, x, y)`;
/// the colour for each age is worked out once per frame (a 176-entry table),
/// so a pixel costs a subtraction and a lookup.
fn power_on_frame(f: u32, canvas: &mut Canvas, table: Option<&[u8]>) {
    let t_ms = f * POWER_ON_FRAME.as_millis() as u32;
    let t = t_ms as f32;
    let total = POWER_ON_FRAMES as f32 * POWER_ON_FRAME.as_millis() as f32;
    let fade = ((total - FADE_MS - t) / FADE_MS + 1.0).clamp(0.0, 1.0);
    let mut lut = [Rgb::BLACK; AGES];
    for (a, c) in lut.iter_mut().enumerate() {
        // Flash then settle: 1.35 at ignition, 0.55 at rest.
        let k = 1.0 + (a as u32 * TICK_MS) as f32 / 60.0;
        *c = glow((0.55 + 0.8 / (k * k)) * fade);
    }
    let now = t_ms / TICK_MS;
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let at = match table {
                Some(t) => t[y * WIDTH + x],
                None => ignite(x, y),
            };
            let colour = match now.checked_sub(u32::from(at)) {
                None => Rgb::BLACK,
                Some(age) => lut[(age as usize).min(AGES - 1)],
            };
            if canvas.get(x, y) != colour {
                canvas.set(x, y, colour);
            }
        }
    }
}

/// Amber for a level up to 1, blending to white above it (a dot that has
/// just ignited is hotter than one at rest).
fn glow(level: f32) -> Rgb {
    let amber = [255.0, 176.0, 80.0];
    let base = level.min(1.0);
    let hot = ((level - 1.0) / 0.35).clamp(0.0, 1.0);
    let c = |i: usize| -> u8 {
        let v = amber[i] * base + (255.0 - amber[i]) * hot;
        v.round().clamp(0.0, 255.0) as u8
    };
    Rgb::new(c(0), c(1), c(2))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::player::FrameSource;

    fn play(path: &str) -> (Vec<Canvas>, Vec<Duration>) {
        let mut src = FrameSource::open(open(path).unwrap()).unwrap();
        assert_eq!(src.kind(), "synth");
        let mut canvas = Canvas::new();
        canvas.fill(Rgb::new(1, 2, 3));
        let (mut frames, mut delays) = (Vec::new(), Vec::new());
        while let Some(d) = src.next_frame(&mut canvas).unwrap() {
            frames.push(canvas.clone());
            delays.push(d);
        }
        assert_eq!(src.frames_decoded(), frames.len() as u64);
        (frames, delays)
    }

    #[test]
    fn paths() {
        assert!(is_virtual(POWER_ON) && !is_virtual("/gifs/x.gif"));
        assert!(open("/gifs/x.gif").is_none());
        let p = card_path("Sonic | 3\n", "Mega Drive");
        assert_eq!(p, "@card:Sonic   3|Mega Drive");
        assert_eq!(title(&p).as_deref(), Some("Sonic   3"));
        assert_eq!(title(POWER_ON).as_deref(), Some("Power on"));
        assert_eq!(title("@nonsense"), None);
        assert!(card_path(&"é".repeat(400), "").len() <= MAX_PATH);
        // An unknown `@` path opens as a stream but not as a file.
        assert!(FrameSource::open(open("@nonsense").unwrap()).is_err());
    }

    #[test]
    fn a_card_is_one_frame_of_its_dwell() {
        let (frames, delays) = play(&card_path("Sonic the Hedgehog 3", "Mega Drive"));
        assert_eq!(delays, vec![CARD_DWELL]);
        let mut want = Canvas::new();
        text::title_card(&mut want, "Sonic the Hedgehog 3", "Mega Drive");
        assert_eq!(frames[0], want);
    }

    #[test]
    fn power_on_ignites_outward_and_ends_dark() {
        let (frames, delays) = play(POWER_ON);
        assert_eq!(frames.len(), POWER_ON_FRAMES as usize);
        assert!(delays.iter().all(|d| *d == POWER_ON_FRAME));
        let total: Duration = delays.iter().sum();
        assert_eq!(total, Duration::from_millis(700));
        let lit = |c: &Canvas| c.as_slice().iter().filter(|p| **p != Rgb::BLACK).count();
        // Starts from black (the fill before it is cleared), the centre first.
        assert!(lit(&frames[0]) < 40);
        let centre = |c: &Canvas| c.get(63, 15) != Rgb::BLACK;
        let corner = |c: &Canvas| c.get(0, 0) != Rgb::BLACK;
        let first_centre = frames.iter().position(centre).unwrap();
        let first_corner = frames.iter().position(corner).unwrap();
        assert!(first_centre < 3 && first_corner > first_centre + 10);
        // Coverage grows through the spread, then the whole panel is lit.
        let counts: Vec<usize> = frames.iter().map(lit).collect();
        assert!(counts[..22].windows(2).all(|w| w[1] >= w[0]), "{counts:?}");
        assert_eq!(counts[24], WIDTH * HEIGHT);
        // Overshoot: a freshly ignited dot is brighter than one at rest.
        let c = frames[first_centre].get(63, 15);
        let rest = frames[20].get(63, 15);
        assert!(c.g > rest.g && c.b > rest.b, "{c:?} vs {rest:?}");
        // The last frame is nearly dark, the brightest dot well down.
        let max = frames
            .last()
            .unwrap()
            .as_slice()
            .iter()
            .map(|p| p.r)
            .max()
            .unwrap();
        assert!(max < 40, "{max}");
        // Deterministic.
        assert_eq!(play(POWER_ON).0, frames);
    }

    #[test]
    fn power_on_without_the_table_is_the_same() {
        let table = ignition_table().unwrap();
        for f in [0, 5, 12, 20, 34] {
            let (mut a, mut b) = (Canvas::new(), Canvas::new());
            power_on_frame(f, &mut a, Some(&table));
            power_on_frame(f, &mut b, None);
            assert_eq!(a, b, "frame {f}");
        }
        assert!(table.iter().all(|t| u32::from(*t) * TICK_MS <= 440));
    }

    #[test]
    fn power_on_marks_only_what_changes() {
        let mut src = FrameSource::open(open(POWER_ON).unwrap()).unwrap();
        let mut canvas = Canvas::new();
        let mut prev = canvas.clone();
        while src.next_frame(&mut canvas).unwrap().is_some() {
            let changed = canvas
                .as_slice()
                .iter()
                .zip(prev.as_slice())
                .filter(|(a, b)| a != b)
                .count() as u32;
            if src.frames_decoded() > 1 {
                assert_eq!(canvas.dirty().count(), changed);
            }
            canvas.dirty_mut().clear();
            prev = canvas.clone();
        }
    }

    #[test]
    fn loops_by_rewinding() {
        use std::io::{Seek, SeekFrom};
        let mut src = FrameSource::open(open(POWER_ON).unwrap()).unwrap();
        let mut canvas = Canvas::new();
        while src.next_frame(&mut canvas).unwrap().is_some() {}
        let mut r = src.into_inner();
        r.seek(SeekFrom::Start(0)).unwrap();
        let mut again = FrameSource::open(r).unwrap();
        assert!(again.next_frame(&mut canvas).unwrap().is_some());
    }
}

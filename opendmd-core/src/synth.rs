//! Virtual files: pictures the engine plays like a GIF but that are drawn,
//! not read — the power-on effect, title cards, the live clock and the
//! scrolling text.
//!
//! A virtual file has a path starting with `@` ([`POWER_ON`],
//! [`card_path`]`(title, subtitle)` = `@card:<title>|<subtitle>`,
//! [`CLOCK`]`[?options]` and `@text?msg=<url-encoded>[&options]` — see
//! [`ClockSpec`] and [`TextSpec`] for the options). The engine
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
//!
//! **The live ones.** `@clock` and `@text` are drawn every frame from their
//! options (and, for the clock, the local time from [`set_wall_clock`]'s
//! source: the firmware's SNTP clock, the browser's `Date`, the host's clock
//! in the simulator — `--:--` until it is set). Both only write the pixels
//! that change (the dirty map), so the clock costs nothing between changes
//! of the string it shows: it ticks once a second, on the second, and draws
//! only when the minute (or the second, with `seconds=1`) moves. A clock
//! "loop" is one second and a text loop one pass across the panel, so dwell
//! applies as to any file (a clock under `dwell=60` shows for a minute); a
//! loop restarts in place ([`SynthSource::rewind`]: no reopen, no
//! allocation). Their state is a few words and the message's bytes inside
//! the decoder; nothing is allocated per frame.
//!
//! **Effects.** `@fx:<name>[?options]` ([`crate::fx`]) is drawn every
//! frame the same way: frame `n` of the effect's loop at `t = n × delay`,
//! the loop's length what dwell counts, rewound in place.

use crate::canvas::{Canvas, Rgb, HEIGHT, WIDTH};
use crate::text::{self, Font};
use std::io::{self, Cursor, Read};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

/// First four bytes of a virtual file's in-memory stream.
pub const MAGIC: [u8; 4] = *b"SYNV";
/// The power-on effect's path.
pub const POWER_ON: &str = "@power-on";
/// The prefix of a title card's path.
pub const CARD_PREFIX: &str = "@card:";
/// Frames in the power-on effect, [`POWER_ON_FRAME`] apart: 700 ms.
pub const POWER_ON_FRAMES: u32 = 35;
pub const POWER_ON_FRAME: Duration = Duration::from_millis(20);
/// How long a title card holds its one frame (one loop).
pub const CARD_DWELL: Duration = Duration::from_millis(3000);
/// A virtual path longer than this is refused.
const MAX_PATH: usize = 512;
/// The live clock's path (options after `?`: see [`ClockSpec`]).
pub const CLOCK: &str = "@clock";
/// The scrolling text's path prefix: `@text?msg=...` (see [`TextSpec`]).
pub const TEXT: &str = "@text";
/// A text message is cut to this many characters.
pub const TEXT_MAX: usize = 200;
/// The scrolling text's default and fastest step (ms per pixel).
pub const TEXT_STEP_MS: u64 = 40;
pub const TEXT_STEP_MIN_MS: u64 = 20;
const TEXT_STEP_MAX_MS: u64 = 1000;
/// How long a static text or a clock frame holds when nothing moves.
const HOLD: Duration = Duration::from_millis(1000);

/// Local wall-clock time, as the live clock shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct WallTime {
    pub year: u16,
    /// 1..=12
    pub month: u8,
    /// 1..=31
    pub day: u8,
    /// ISO: 1 = Monday .. 7 = Sunday.
    pub weekday: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    /// Into the second, 0..1000.
    pub millis: u16,
}

impl WallTime {
    /// From seconds since 1970-01-01 in the zone wanted (the caller adds
    /// its UTC offset) and the milliseconds into the second: the proleptic
    /// Gregorian calendar (Howard Hinnant's `civil_from_days`).
    pub fn from_unix(secs: i64, millis: u16) -> WallTime {
        let days = secs.div_euclid(86_400);
        let rem = secs.rem_euclid(86_400);
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = doy - (153 * mp + 2) / 5 + 1;
        let month = if mp < 10 { mp + 3 } else { mp - 9 };
        let year = yoe + era * 400 + i64::from(month <= 2);
        // 1970-01-01 was a Thursday (ISO 4).
        let weekday = (days + 3).rem_euclid(7) + 1;
        WallTime {
            year: year.clamp(0, 9999) as u16,
            month: month as u8,
            day: day as u8,
            weekday: weekday as u8,
            hour: (rem / 3600) as u8,
            minute: (rem / 60 % 60) as u8,
            second: (rem % 60) as u8,
            millis: millis.min(999),
        }
    }
}

/// Where the live clock reads the local time: `None` while it is not known
/// (the firmware before its first SNTP sync).
pub type WallClock = fn() -> Option<WallTime>;

/// The installed [`WallClock`] as an address (0: none). An atomic rather
/// than a lock: it is written once at start-up and read once a second, and
/// a lock costs the web module ~0.3 KB.
static WALL: AtomicUsize = AtomicUsize::new(0);

/// Install the local-time source the live clock reads (the firmware's
/// `clock.rs`, the browser's `Date`, the simulator's host clock). Until one
/// is installed, and whenever it answers `None`, the clock shows `--:--`.
pub fn set_wall_clock(clock: Option<WallClock>) {
    WALL.store(clock.map_or(0, |f| f as usize), Ordering::Relaxed);
}

fn wall_now() -> Option<WallTime> {
    match WALL.load(Ordering::Relaxed) {
        0 => None,
        a => {
            // SAFETY: `a` was stored by `set_wall_clock` from a `WallClock`
            // (a fn pointer, the size of a `usize` on every target here),
            // and fn pointers are never null, so 0 means none.
            let f = unsafe { std::mem::transmute::<usize, WallClock>(a) };
            f()
        }
    }
}

/// The live clock's options: `@clock?style=big|small&seconds=1&date=1&h12=1&color=RRGGBB`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClockSpec {
    /// `style=big` (the default): HH:MM in the 5x7 font at 4x (3x with the
    /// seconds or the date); `style=small`: at 2x.
    pub big: bool,
    pub seconds: bool,
    /// The date under the time (`Mon 28 Sep`).
    pub date: bool,
    /// 12-hour clock.
    pub h12: bool,
    pub colour: Rgb,
}

impl Default for ClockSpec {
    fn default() -> Self {
        ClockSpec {
            big: true,
            seconds: false,
            date: false,
            h12: false,
            colour: text::TITLE_COLOUR,
        }
    }
}

/// The scrolling text's options:
/// `@text?msg=<url-encoded>&speed=<ms per px>&font=big|normal|small&color=RRGGBB&scroll=0`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextSpec {
    /// Folded to the fonts' ASCII ([`text::fold_char`]), at most
    /// [`TEXT_MAX`] characters.
    pub msg: Vec<u8>,
    /// One pixel per `step` (default 40 ms, at least 20).
    pub step: Duration,
    /// `font=big` (the default: 5x7 at 2x), `normal` (5x7), `small` (4x6).
    pub font: Font,
    pub scale: usize,
    pub colour: Rgb,
    /// `scroll=0`: shown still and centred if it fits (it scrolls anyway if
    /// it does not).
    pub scroll: bool,
}

/// The path of a scrolling text showing `msg` with the default options.
pub fn text_path(msg: &str) -> String {
    let mut p = format!("{TEXT}?msg=");
    for b in msg.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            p.push(b as char);
        } else {
            p.push_str(&format!("%{b:02X}"));
        }
        if p.len() > MAX_PATH - 3 {
            break;
        }
    }
    p
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// `application/x-www-form-urlencoded` decoding: `%XX` and `+` for a space;
/// invalid UTF-8 decodes to nothing.
fn url_decode(b: &[u8]) -> String {
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let c = match b[i] {
            b'+' => b' ',
            b'%' if i + 2 < b.len() => match (hex_val(b[i + 1]), hex_val(b[i + 2])) {
                (Some(h), Some(l)) => {
                    i += 2;
                    h << 4 | l
                }
                _ => b'%',
            },
            c => c,
        };
        out.push(c);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_default()
}

/// `RRGGBB` (a `#` in front, `%23` in a URL, is allowed).
pub(crate) fn parse_colour(s: &[u8]) -> Option<Rgb> {
    let s = s.strip_prefix(b"%23").or(s.strip_prefix(b"#")).unwrap_or(s);
    let mut v = 0u32;
    for &b in s {
        v = v << 4 | u32::from(hex_val(b)?);
    }
    (s.len() == 6).then(|| Rgb::new((v >> 16) as u8, (v >> 8) as u8, v as u8))
}

/// A small decimal, `None` if it is not one.
fn parse_u16(s: &[u8]) -> Option<u16> {
    if s.is_empty() || s.len() > 4 {
        return None;
    }
    s.iter().try_fold(0u16, |n, &b| {
        b.is_ascii_digit().then(|| n * 10 + u16::from(b - b'0'))
    })
}

/// The `key=value` pairs after `?`, as bytes (only the message needs
/// decoding).
fn options(query: &str) -> impl Iterator<Item = (&[u8], &[u8])> {
    query
        .as_bytes()
        .split(|&b| b == b'&')
        .map(|kv| match kv.iter().position(|&b| b == b'=') {
            Some(i) => (&kv[..i], &kv[i + 1..]),
            None => (kv, &kv[kv.len()..]),
        })
}

fn parse_clock(query: &str) -> ClockSpec {
    let mut c = ClockSpec::default();
    for (k, v) in options(query) {
        let on = v == b"1";
        match k {
            b"style" => c.big = v != b"small",
            b"seconds" => c.seconds = on,
            b"date" => c.date = on,
            b"h12" => c.h12 = on,
            b"color" => c.colour = parse_colour(v).unwrap_or(c.colour),
            _ => {}
        }
    }
    c
}

fn parse_text(query: &str) -> TextSpec {
    let mut t = TextSpec {
        msg: Vec::new(),
        step: Duration::from_millis(TEXT_STEP_MS),
        font: Font::Normal,
        scale: 2,
        colour: text::TITLE_COLOUR,
        scroll: true,
    };
    for (k, v) in options(query) {
        match k {
            b"msg" => {
                t.msg.clear();
                for c in url_decode(v).trim().chars().take(TEXT_MAX) {
                    t.msg.push(text::fold_char(c) as u8);
                }
            }
            b"speed" => {
                if let Some(ms) = parse_u16(v) {
                    t.step = Duration::from_millis(
                        u64::from(ms).clamp(TEXT_STEP_MIN_MS, TEXT_STEP_MAX_MS),
                    );
                }
            }
            b"font" => {
                (t.font, t.scale) = match v {
                    b"small" => (Font::Small, 1),
                    b"normal" => (Font::Normal, 1),
                    _ => (Font::Normal, 2),
                }
            }
            b"color" => t.colour = parse_colour(v).unwrap_or(t.colour),
            b"scroll" => t.scroll = v != b"0",
            _ => {}
        }
    }
    t
}

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

/// What a virtual path shows, for `now_playing.title` and the Library: a
/// card's title, "Power on", "Clock", or a text's message. `None` for
/// anything else.
pub fn title(path: &str) -> Option<String> {
    match parse(path)? {
        Spec::PowerOn => Some("Power on".to_string()),
        Spec::Card { title, .. } => Some(title),
        Spec::Clock(_) => Some("Clock".to_string()),
        Spec::Text(t) => Some(t.msg.iter().map(|&b| b as char).collect()),
        Spec::Fx(f) => Some(f.title().to_string()),
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
    Clock(ClockSpec),
    Text(TextSpec),
    Fx(crate::fx::Fx),
}

fn parse(path: &str) -> Option<Spec> {
    if path == POWER_ON {
        return Some(Spec::PowerOn);
    }
    if path.starts_with(crate::fx::PREFIX) {
        return crate::fx::Fx::parse(path).map(Spec::Fx);
    }
    let (base, query) = path.split_once('?').unwrap_or((path, ""));
    if base == CLOCK {
        return Some(Spec::Clock(parse_clock(query)));
    }
    if base == TEXT {
        return Some(Spec::Text(parse_text(query)));
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
    /// A live file has painted the whole canvas once (it then only
    /// repaints what it moves); kept across [`rewind`](Self::rewind).
    painted: bool,
    /// What the clock shows now (the fields it draws, packed).
    shows: u64,
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
            painted: false,
            shows: u64::MAX,
        })
    }

    /// Start again from the first frame without reopening: a loop. The live
    /// files keep what they know of the canvas (it still shows their last
    /// frame), so a clock's next second draws only what changed.
    pub fn rewind(&mut self) {
        self.frames = 0;
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
        let delay = match &mut self.spec {
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
            Spec::Clock(c) => {
                if self.frames >= 1 {
                    return Ok(None);
                }
                let now = wall_now();
                let shows = clock_key(c, now);
                if !self.painted || shows != self.shows {
                    draw_clock(canvas, c, now);
                    self.painted = true;
                    self.shows = shows;
                }
                // On the next second.
                now.map_or(HOLD, |t| {
                    Duration::from_millis((1000 - u64::from(t.millis.min(999))).max(20))
                })
            }
            Spec::Fx(f) => {
                if self.frames >= u64::from(f.loop_frames()) {
                    return Ok(None);
                }
                f.frame(f.delay() * self.frames as u32, canvas)
            }
            Spec::Text(t) => {
                let width = text::text_width(t.font, bytes_str(&t.msg), t.scale) as i32;
                let still = !t.scroll && width <= WIDTH as i32;
                let steps = if still {
                    1
                } else {
                    width as u64 + WIDTH as u64 + 1
                };
                if self.frames >= steps {
                    return Ok(None);
                }
                let x = if still {
                    (WIDTH as i32 - width) / 2
                } else {
                    WIDTH as i32 - self.frames as i32
                };
                let h = (t.font.height() * t.scale) as i32;
                let y = (HEIGHT as i32 - h) / 2 + i32::from(t.scale == 1);
                let line = Line {
                    x,
                    y,
                    text: &t.msg,
                    font: t.font,
                    scale: t.scale,
                    colour: t.colour,
                };
                // The whole canvas once (it held another picture), then
                // only the band the text moves in.
                let rows = if self.painted {
                    y.max(0) as usize..((y + h) as usize).min(HEIGHT)
                } else {
                    0..HEIGHT
                };
                paint(canvas, &[line], rows);
                self.painted = true;
                if still {
                    HOLD
                } else {
                    t.step
                }
            }
        };
        self.frames += 1;
        Ok(Some(delay))
    }
}

/// The message bytes as the `&str` the text layout helpers take (they are
/// ASCII, folded by [`text::fold_char`]).
fn bytes_str(b: &[u8]) -> &str {
    std::str::from_utf8(b).unwrap_or("")
}

/// One line of text at a pixel position.
struct Line<'a> {
    x: i32,
    y: i32,
    text: &'a [u8],
    font: Font,
    scale: usize,
    colour: Rgb,
}

impl Line<'_> {
    /// Whether the line lights `(x, y)`: exactly the pixels
    /// [`text::draw_text_with`] would draw.
    fn lights(&self, x: i32, y: i32) -> bool {
        let (dx, dy) = (x - self.x, y - self.y);
        if dx < 0 || dy < 0 {
            return false;
        }
        let (dx, dy) = (dx as usize / self.scale, dy as usize / self.scale);
        let adv = self.font.advance();
        if dy >= self.font.height() {
            return false;
        }
        match self.text.get(dx / adv) {
            Some(&c) => self.font.rows(c as char)[dy] & (0x80 >> (dx % adv)) != 0,
            None => false,
        }
    }
}

/// Make `rows` of the canvas exactly `lines` on black, writing (and so
/// marking) only the pixels that differ.
fn paint(canvas: &mut Canvas, lines: &[Line], rows: std::ops::Range<usize>) {
    for y in rows {
        for x in 0..WIDTH {
            let c = lines
                .iter()
                .find(|l| l.lights(x as i32, y as i32))
                .map_or(Rgb::BLACK, |l| l.colour);
            if canvas.get(x, y) != c {
                canvas.set(x, y, c);
            }
        }
    }
}

const WEEKDAYS: [&[u8; 3]; 7] = [b"Mon", b"Tue", b"Wed", b"Thu", b"Fri", b"Sat", b"Sun"];
const MONTHS: [&[u8; 3]; 12] = [
    b"Jan", b"Feb", b"Mar", b"Apr", b"May", b"Jun", b"Jul", b"Aug", b"Sep", b"Oct", b"Nov", b"Dec",
];

/// Everything the clock draws, packed: when it changes, the clock redraws.
fn clock_key(c: &ClockSpec, t: Option<WallTime>) -> u64 {
    let Some(t) = t else {
        return u64::MAX - 1;
    };
    let s = if c.seconds { u64::from(t.second) } else { 0 };
    let d = if c.date {
        (u64::from(t.weekday) << 16) | (u64::from(t.month) << 8) | u64::from(t.day)
    } else {
        0
    };
    (d << 24) | (u64::from(t.hour) << 16) | (u64::from(t.minute) << 8) | s
}

fn push_2(buf: &mut [u8; 16], n: &mut usize, v: u8, pad: u8) {
    if v >= 10 || pad != 0 {
        buf[*n] = if v >= 10 { b'0' + v / 10 } else { pad };
        *n += 1;
    }
    buf[*n] = b'0' + v % 10;
    *n += 1;
}

/// The clock as it is at `t` (`--:--` without a time): see [`ClockSpec`].
fn draw_clock(canvas: &mut Canvas, c: &ClockSpec, t: Option<WallTime>) {
    let (mut time, mut tn) = ([0u8; 16], 0);
    let (mut date, mut dn) = ([0u8; 16], 0);
    match t {
        Some(t) => {
            let hour = if c.h12 {
                match t.hour % 12 {
                    0 => 12,
                    h => h,
                }
            } else {
                t.hour
            };
            push_2(&mut time, &mut tn, hour, if c.h12 { 0 } else { b'0' });
            time[tn] = b':';
            tn += 1;
            push_2(&mut time, &mut tn, t.minute, b'0');
            if c.seconds {
                time[tn] = b':';
                tn += 1;
                push_2(&mut time, &mut tn, t.second, b'0');
            }
            if c.date {
                let wd = WEEKDAYS[usize::from(t.weekday.clamp(1, 7)) - 1];
                let mon = MONTHS[usize::from(t.month.clamp(1, 12)) - 1];
                date[..3].copy_from_slice(wd);
                date[3] = b' ';
                dn = 4;
                push_2(&mut date, &mut dn, t.day, 0);
                date[dn] = b' ';
                date[dn + 1..dn + 4].copy_from_slice(mon);
                dn += 4;
            }
        }
        None => {
            let s: &[u8] = if c.seconds { b"--:--:--" } else { b"--:--" };
            time[..s.len()].copy_from_slice(s);
            tn = s.len();
        }
    }
    let (time, date) = (&time[..tn], &date[..dn]);
    let dated = c.date && !date.is_empty();
    let scale = match (c.big, c.seconds || c.date) {
        (true, false) => 4,
        (true, true) => 3,
        (false, _) => 2,
    };
    let f = Font::Normal;
    let tw = text::text_width(f, bytes_str(time), scale) as i32;
    let dw = text::text_width(f, bytes_str(date), 1) as i32;
    // Centred on the glyphs' bodies (6 of the font's 7 rows; the 7th is the
    // descender, blank in digits): the time alone, or the time with the
    // date (5x7) 3 px under it.
    let body = (6 * scale) as i32;
    let block = if dated { body + 3 + 6 } else { body };
    let ty = (HEIGHT as i32 - block) / 2;
    let lines = [
        Line {
            x: (WIDTH as i32 - tw) / 2,
            y: ty,
            text: time,
            font: f,
            scale,
            colour: c.colour,
        },
        Line {
            x: (WIDTH as i32 - dw) / 2,
            y: ty + body + 3,
            text: date,
            font: f,
            scale: 1,
            colour: c.colour,
        },
    ];
    paint(canvas, &lines[..if dated { 2 } else { 1 }], 0..HEIGHT);
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

    thread_local! {
        static TEST_TIME: std::cell::Cell<Option<WallTime>> = const { std::cell::Cell::new(None) };
    }

    fn test_clock() -> Option<WallTime> {
        TEST_TIME.with(|t| t.get())
    }

    fn at(h: u8, m: u8, s: u8, ms: u16) {
        set_wall_clock(Some(test_clock));
        TEST_TIME.with(|t| {
            t.set(Some(WallTime {
                year: 2026,
                month: 9,
                day: 28,
                weekday: 1,
                hour: h,
                minute: m,
                second: s,
                millis: ms,
            }))
        });
    }

    fn lit(c: &Canvas) -> usize {
        c.as_slice().iter().filter(|p| **p != Rgb::BLACK).count()
    }

    /// The canvas as text rows, `#` lit: the reference drawing.
    fn reference(lines: &[(i32, i32, &str, Font, usize)]) -> Canvas {
        let mut c = Canvas::new();
        for (x, y, t, f, s) in lines {
            text::draw_text_with(&mut c, *x, *y, t, text::TITLE_COLOUR, *f, *s);
        }
        c
    }

    #[test]
    fn wall_time_from_unix() {
        // 2026-09-28 13:05:09 UTC, a Monday.
        let t = WallTime::from_unix(1_790_600_709, 250);
        assert_eq!(
            t,
            WallTime {
                year: 2026,
                month: 9,
                day: 28,
                weekday: 1,
                hour: 13,
                minute: 5,
                second: 9,
                millis: 250
            }
        );
        let e = WallTime::from_unix(0, 0);
        assert_eq!((e.year, e.month, e.day, e.weekday), (1970, 1, 1, 4));
        let leap = WallTime::from_unix(951_782_400, 0); // 2000-02-29
        assert_eq!((leap.year, leap.month, leap.day), (2000, 2, 29));
    }

    #[test]
    fn options_parse_and_paths() {
        assert!(is_virtual(CLOCK));
        assert_eq!(title(CLOCK).as_deref(), Some("Clock"));
        assert_eq!(title("@clock?style=small&date=1").as_deref(), Some("Clock"));
        let p = text_path("Hello, wörld & 100%");
        assert_eq!(p, "@text?msg=Hello%2C%20w%C3%B6rld%20%26%20100%25");
        assert_eq!(title(&p).as_deref(), Some("Hello, world & 100%"));
        assert_eq!(url_decode(b"a+b%41%zz%4"), "a bA%zz%4");
        assert_eq!(parse_colour(b"#ff8000"), Some(Rgb::new(255, 128, 0)));
        assert_eq!(parse_colour(b"%23FF8000"), Some(Rgb::new(255, 128, 0)));
        assert_eq!(parse_colour(b"fff"), None);
        assert_eq!(parse_u16(b"40"), Some(40));
        assert_eq!(parse_u16(b"4x"), None);
        let Some(Spec::Text(t)) = parse("@text?msg=Hi&speed=5&font=small&color=00ff00&scroll=0")
        else {
            panic!()
        };
        assert_eq!(t.msg, b"Hi");
        assert_eq!(t.step, Duration::from_millis(TEXT_STEP_MIN_MS));
        assert_eq!(
            (t.font, t.scale, t.colour, t.scroll),
            (Font::Small, 1, Rgb::new(0, 255, 0), false)
        );
        let Some(Spec::Clock(c)) = parse("@clock?style=small&seconds=1&date=1&h12=1") else {
            panic!()
        };
        assert!(!c.big && c.seconds && c.date && c.h12);
        assert_eq!(parse("@clock"), Some(Spec::Clock(ClockSpec::default())));
        let long = "x".repeat(400);
        let Some(Spec::Text(t)) = parse(&format!("@text?msg={long}")) else {
            panic!()
        };
        assert_eq!(t.msg.len(), TEXT_MAX);
        let e = crate::manifest::Entry::from_path("@text?msg=Game%20over");
        assert_eq!(
            (e.title.as_str(), e.platform, e.category),
            ("Game over", None, None)
        );
    }

    #[test]
    fn clock_draws_the_time_and_redraws_only_on_change() {
        at(9, 5, 30, 400);
        let mut src = FrameSource::open(open(CLOCK).unwrap()).unwrap();
        let mut canvas = Canvas::new();
        canvas.fill(Rgb::new(9, 9, 9));
        let d = src.next_frame(&mut canvas).unwrap().unwrap();
        // On the next second.
        assert_eq!(d, Duration::from_millis(600));
        // HH:MM at 4x, centred on the glyph bodies.
        let want = reference(&[((128 - 96) / 2, 4, "09:05", Font::Normal, 4)]);
        assert_eq!(canvas, want);
        // One frame per loop; the loop restarts in place and draws nothing
        // while the minute is the same.
        assert!(src.next_frame(&mut canvas).unwrap().is_none());
        assert!(src.rewind_virtual());
        canvas.dirty_mut().clear();
        at(9, 5, 31, 0);
        assert_eq!(
            src.next_frame(&mut canvas).unwrap(),
            Some(Duration::from_millis(1000))
        );
        assert!(canvas.dirty().is_clean());
        // The minute moves: only the changed pixels are written.
        at(9, 6, 0, 0);
        assert!(src.next_frame(&mut canvas).unwrap().is_none());
        src.rewind_virtual();
        src.next_frame(&mut canvas).unwrap().unwrap();
        let want2 = reference(&[((128 - 96) / 2, 4, "09:06", Font::Normal, 4)]);
        assert_eq!(canvas, want2);
        let changed = want
            .as_slice()
            .iter()
            .zip(want2.as_slice())
            .filter(|(a, b)| a != b)
            .count() as u32;
        assert_eq!(canvas.dirty().count(), changed);
        assert!(changed > 0 && (changed as usize) < lit(&want2));
    }

    #[test]
    fn clock_styles_and_no_time() {
        at(21, 7, 4, 0);
        let (f, _) = play_first("@clock?seconds=1&date=1");
        let want = reference(&[
            ((128 - 117) / 2, 2, "21:07:04", Font::Normal, 3),
            ((128 - 49) / 2, 23, "Mon 28 Sep", Font::Normal, 1),
        ]);
        assert_eq!(f, want);
        let (f, _) = play_first("@clock?style=small&h12=1");
        assert_eq!(
            f,
            reference(&[((128 - 19 * 2) / 2, 10, "9:07", Font::Normal, 2)])
        );
        set_wall_clock(None);
        let (f, d) = play_first(CLOCK);
        assert_eq!(
            f,
            reference(&[((128 - 96) / 2, 4, "--:--", Font::Normal, 4)])
        );
        assert_eq!(d, Duration::from_millis(1000));
    }

    fn play_first(path: &str) -> (Canvas, Duration) {
        let mut src = FrameSource::open(open(path).unwrap()).unwrap();
        let mut canvas = Canvas::new();
        let d = src.next_frame(&mut canvas).unwrap().unwrap();
        (canvas, d)
    }

    #[test]
    fn text_scrolls_a_pixel_a_step_and_loops() {
        let path = "@text?msg=OpenDMD&speed=30";
        let mut src = FrameSource::open(open(path).unwrap()).unwrap();
        let mut canvas = Canvas::new();
        canvas.fill(Rgb::new(50, 0, 0));
        let w = 68; // 7 characters of 5x7 at 2x
        let mut frames = 0;
        while let Some(d) = src.next_frame(&mut canvas).unwrap() {
            assert_eq!(d, Duration::from_millis(30));
            let x = 128 - frames;
            assert_eq!(
                canvas,
                reference(&[(x, 9, "OpenDMD", Font::Normal, 2)]),
                "step {frames}"
            );
            if frames > 0 {
                // Only the band rows are touched after the first frame.
                assert!(canvas.dirty().row_mask() & !(((1u32 << 14) - 1) << 9) == 0);
            }
            canvas.dirty_mut().clear();
            frames += 1;
        }
        assert_eq!(frames, w + 129);
        // A loop starts over, in place.
        assert!(src.rewind_virtual());
        assert!(src.next_frame(&mut canvas).unwrap().is_some());
        assert!(canvas.dirty().is_clean(), "the first step is off the panel");
        // Static when asked and it fits.
        let (f, d) = play_first("@text?msg=Hi&scroll=0&font=normal");
        assert_eq!(d, HOLD);
        assert_eq!(f, reference(&[((128 - 9) / 2, 13, "Hi", Font::Normal, 1)]));
    }

    #[test]
    fn live_files_in_the_engine_respect_dwell() {
        use crate::engine::{DwellPolicy, Engine, FileCatalog, Source, TickOutcome};
        use crate::filter::Filter;
        use crate::scheduler::{Mode, Scheduler};
        struct NoFiles;
        impl Source for NoFiles {
            fn open(&self, p: &str) -> io::Result<Box<dyn crate::engine::ReadSeek>> {
                Err(io::Error::new(io::ErrorKind::NotFound, p.to_string()))
            }
        }
        at(10, 0, 0, 0);
        let m = "@clock\n@text?msg=Hi\n".to_string();
        let cat = FileCatalog::build(Cursor::new(m.into_bytes()), &Filter::new()).unwrap();
        let dwell = DwellPolicy {
            min: Duration::from_secs(3),
            max: Duration::from_secs(30),
            min_loops: 1,
        };
        let mut e = Engine::new(
            Box::new(cat),
            Scheduler::new(2, Mode::Sequential, 0),
            dwell,
            Box::new(NoFiles),
        );
        let mut canvas = Canvas::new();
        let mut t = Duration::ZERO;
        let mut starts = Vec::new();
        while t < Duration::from_secs(12) {
            if let TickOutcome::Frame {
                playing, started, ..
            } = e.tick(t, &mut canvas).unwrap()
            {
                if started {
                    starts.push((t.as_millis(), playing.path, playing.title));
                }
            }
            t += Duration::from_millis(10);
        }
        // The clock holds for its 3 s (three one-second loops), then the text.
        assert_eq!(starts[0], (0, "@clock".to_string(), "Clock".to_string()));
        assert_eq!(starts[1].1, "@text?msg=Hi");
        assert_eq!(starts[1].2, "Hi");
        assert_eq!(starts[1].0, 3000);
    }

    #[test]
    fn effects_play_by_the_loop_under_dwell() {
        use crate::engine::{DwellPolicy, Engine, FileCatalog, Source, TickOutcome};
        use crate::filter::Filter;
        use crate::scheduler::{Mode, Scheduler};
        struct NoFiles;
        impl Source for NoFiles {
            fn open(&self, p: &str) -> io::Result<Box<dyn crate::engine::ReadSeek>> {
                Err(io::Error::new(io::ErrorKind::NotFound, p.to_string()))
            }
        }
        assert_eq!(title("@fx:tunnel?cycle=1").as_deref(), Some("Tunnel"));
        assert_eq!(title("@fx:nope"), None);
        let m = "@fx:starfield?fps=50\n@fx:wire?speed=4\n".to_string();
        let cat = FileCatalog::build(Cursor::new(m.into_bytes()), &Filter::new()).unwrap();
        let dwell = DwellPolicy {
            min: Duration::from_secs(3),
            max: Duration::from_secs(30),
            min_loops: 1,
        };
        let mut e = Engine::new(
            Box::new(cat),
            Scheduler::new(2, Mode::Sequential, 0),
            dwell,
            Box::new(NoFiles),
        );
        let mut canvas = Canvas::new();
        let mut t = Duration::ZERO;
        let mut starts = Vec::new();
        let mut frames = 0;
        while t < Duration::from_secs(8) {
            if let TickOutcome::Frame {
                playing, started, ..
            } = e.tick(t, &mut canvas).unwrap()
            {
                frames += 1;
                if started {
                    starts.push((t.as_millis(), playing.path));
                }
            }
            t += Duration::from_millis(5);
        }
        // The starfield's loop is 2.4 s: two loops cover the 3 s minimum.
        assert_eq!(starts[0], (0, "@fx:starfield?fps=50".to_string()));
        assert_eq!(starts[1], (4800, "@fx:wire?speed=4".to_string()));
        // 4.8 s at 50 frames a second, then 3.2 s at the default 100.
        assert!((555..=565).contains(&frames), "{frames}");
    }

    #[test]
    fn effects_keep_their_schedule_when_ticked_late() {
        use crate::engine::{DwellPolicy, Engine, FileCatalog, Source, TickOutcome};
        use crate::filter::Filter;
        use crate::scheduler::{Mode, Scheduler};
        struct NoFiles;
        impl Source for NoFiles {
            fn open(&self, p: &str) -> io::Result<Box<dyn crate::engine::ReadSeek>> {
                Err(io::Error::new(io::ErrorKind::NotFound, p.to_string()))
            }
        }
        // Ticked every 3 ms, a 10 ms frame is seen 0-2 ms late: timed from
        // the tick it would come every 12 ms (83 fps); it keeps 100.
        for (path, want) in [("@fx:starfield", 600), ("@text?msg=Hi&speed=20", 286)] {
            let cat = FileCatalog::build(
                Cursor::new(format!("{path}\n").into_bytes()),
                &Filter::new(),
            )
            .unwrap();
            let dwell = DwellPolicy {
                min: Duration::from_secs(60),
                max: Duration::from_secs(60),
                min_loops: 1,
            };
            let mut e = Engine::new(
                Box::new(cat),
                Scheduler::new(1, Mode::Sequential, 0),
                dwell,
                Box::new(NoFiles),
            );
            let mut canvas = Canvas::new();
            let mut frames = 0;
            for i in 0..2000u64 {
                if let TickOutcome::Frame { .. } =
                    e.tick(Duration::from_millis(i * 3), &mut canvas).unwrap()
                {
                    frames += 1;
                }
            }
            // (A text, not an effect, is still timed from the tick: 21 ms.)
            assert!((want - 5..=want + 5).contains(&frames), "{path}: {frames}");
        }
    }

    #[test]
    fn loops_by_rewinding() {
        use std::io::{Seek, SeekFrom};
        let mut src = FrameSource::open(open(POWER_ON).unwrap()).unwrap();
        let mut canvas = Canvas::new();
        while src.next_frame(&mut canvas).unwrap().is_some() {}
        let mut r = src.into_inner().unwrap();
        r.seek(SeekFrom::Start(0)).unwrap();
        let mut again = FrameSource::open(r).unwrap();
        assert!(again.next_frame(&mut canvas).unwrap().is_some());
    }
}

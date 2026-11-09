//! Virtual files: pictures the engine plays like a GIF but that are drawn,
//! not read — the power-on effect, title cards, the start-up show and
//! Formulas.
//!
//! A virtual file has a path starting with `@` ([`POWER_ON`],
//! [`card_path`]`(title, subtitle)` = `@card:<title>|<subtitle>`,
//! `@boot[?options]` and the Formulas' `@formula…`). The engine
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
//! **Formulas.** `@formula?src=…` ([`crate::fx`], [`crate::fx_psi`]) is
//! drawn every frame: frame `n` of the formula's loop at
//! `t = n × delay`, the loop's length what dwell counts, rewound in place.

use crate::canvas::{Canvas, Rgb, HEIGHT, WIDTH};
use crate::text;
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
/// A virtual path longer than this is refused (a formula's may be
/// [`MAX_FORMULA_PATH`]).
pub(crate) const MAX_PATH: usize = 512;
/// A formula's inline path (`@formula?psb=…`: the compiled Formula,
/// base64url) may be this long (`POST /api/play`'s body is 8 KB at most).
/// `POST /api/play` refuses a longer one with a 413. Playlists and looks
/// name Formulas by reference instead ([`formula_ref`]): a bundled one by
/// name, one of the user's by its `.psi` file on the card (played from the
/// compiled `.psb` beside it).
pub const MAX_FORMULA_PATH: usize = 6144;
/// An inline formula as the Look setting (`look_formula`, kept in NVS) may be
/// this long; the web app sets looks by reference.
pub const MAX_LOOK_PATH: usize = 2048;
/// A compiled Formula's file (`.psb`, beside its `.psi`) may be this long.
pub const MAX_FORMULA_FILE: usize = 16 * 1024;
const MAX_READ: usize = if cfg!(feature = "psi") {
    MAX_FORMULA_PATH + MAX_FORMULA_FILE * 4 / 3 + 16
} else {
    MAX_PATH
};
/// A Formula's name on its path (`&name=`) is cut to this many characters.
pub const MAX_FORMULA_NAME: usize = 60;
/// A formula's path starts with this: `@formula?psb=<the compiled
/// Formula, base64url>[&a=&b=&c=][&fps=][&name=]` ([`crate::fx_psi`]).
pub const FORMULA: &str = "@formula";

/// Is `path` a built-in effect's path from before they were Formulas
/// (`@fx:tunnel?…`, and `@fx:psi?…`, the Formula path's old name)? It names
/// nothing now: a playlist line is skipped with a log line
/// ([`crate::manifest::scan`]), `POST /api/play` answers 415.
pub fn is_retired_effect(path: &str) -> bool {
    path.starts_with(crate::fx::RETIRED_PREFIX)
}

/// Is `path` a formula played from its path: inline (`@formula?src=…`) or a
/// bundled one by name (`@formula:<name>[?a=&b=&c=&fps=]`)? A `.psi` file on
/// the card is a formula too, but read from the card: [`formula_file`].
pub fn is_formula(path: &str) -> bool {
    path.strip_prefix(FORMULA)
        .is_some_and(|r| r.is_empty() || r.starts_with('?') || r.starts_with(':'))
}

/// A bundled Formula's path starts with this: `@formula:<name>`, the name its
/// file's stem in `opendmd-web/formulas/` (`[a-z0-9-]`, 1-40).
pub const FORMULA_REF: &str = "@formula:";
/// A user's Formula on the card: a file with this extension, played as
/// `/formulas/<stem>.psi[?a=&b=&c=&fps=]` from the compiled Formula beside
/// it, [`COMPILED_EXT`] (the player never compiles: the web app writes both).
pub const FORMULA_EXT: &str = ".psi";
/// A compiled Formula's file: `<stem>.psb` beside `<stem>.psi`.
pub const COMPILED_EXT: &str = ".psb";

/// The compiled file for a Formula's `.psi` path (`/formulas/x.psi` ->
/// `/formulas/x.psb`).
pub fn compiled_path(file: &str) -> String {
    let stem = &file[..file.len().saturating_sub(FORMULA_EXT.len())];
    format!("{stem}{COMPILED_EXT}")
}

/// How a path names a Formula ([`formula_ref`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormulaRef<'a> {
    /// `@formula?psb=…`: the compiled Formula on the path.
    Inline,
    /// `@formula:<name>?<opts>`: one of the bundled Formulas (a name that
    /// is not a valid stem is still this: it names none, a 404).
    Bundled { name: &'a str, opts: &'a str },
    /// `<file>.psi?<opts>`: a file on the card.
    File { file: &'a str, opts: &'a str },
}

/// How `path` names a Formula, or `None` if it names none.
pub fn formula_ref(path: &str) -> Option<FormulaRef<'_>> {
    if let Some(rest) = path.strip_prefix(FORMULA_REF) {
        let (name, opts) = rest.split_once('?').unwrap_or((rest, ""));
        return Some(FormulaRef::Bundled { name, opts });
    }
    if is_formula(path) {
        return Some(FormulaRef::Inline);
    }
    formula_file(path).map(|(file, opts)| FormulaRef::File { file, opts })
}

/// A card path to a `.psi` file, maybe with options after `?` (a FAT name
/// has no `?`): the file and the options.
pub fn formula_file(path: &str) -> Option<(&str, &str)> {
    if !path.starts_with('/') {
        return None;
    }
    let (file, opts) = path.split_once('?').unwrap_or((path, ""));
    let n = file.len();
    let ext = FORMULA_EXT.len();
    (n > ext + 1
        && file.is_char_boundary(n - ext)
        && file[n - ext..].eq_ignore_ascii_case(FORMULA_EXT)
        && !file.ends_with("/.psi"))
    .then_some((file, opts))
}

/// Is `name` a bundled Formula's name as written (`[a-z0-9-]`, 1-40)?
pub fn valid_formula_name(name: &str) -> bool {
    (1..=40).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// The bundled Formulas, (name, compiled Formula: `opendmd_psi::psb`), as
/// the host built them in (the firmware's from `opendmd-web/formulas/`'s
/// `-- bundle` files, compiled at build time): what `@formula:<name>`
/// plays. Nothing until [`set_bundled`].
pub type Bundled = &'static [(&'static str, &'static [u8])];
static BUNDLED: std::sync::OnceLock<Bundled> = std::sync::OnceLock::new();

/// Install the bundled Formulas (once; later calls are ignored).
pub fn set_bundled(list: Bundled) {
    let _ = BUNDLED.set(list);
}

/// A bundled Formula's compiled bytes by name.
pub fn bundled(name: &str) -> Option<&'static [u8]> {
    BUNDLED
        .get()?
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, s)| *s)
}

/// The Formula the editor sent to the panel (`POST /api/play/formula`):
/// played as `@formula:~sent[?a=&b=&c=&fps=]`, its compiled bytes held in
/// memory ([`set_sent`]), so a Formula's length never rides on its path.
/// One at a time: each one sent replaces the last, and it is let go once
/// the panel moves on to anything else. Not kept in Recently played.
pub const SENT_NAME: &str = "~sent";
pub const SENT: &str = "@formula:~sent";
type Held = std::sync::Mutex<Option<std::sync::Arc<Vec<u8>>>>;
static SENT_PSB: Held = std::sync::Mutex::new(None);

/// Hold `psb` as the sent Formula (`None`: let the last one go).
pub fn set_sent(psb: Option<Vec<u8>>) {
    let old = std::mem::replace(
        &mut *SENT_PSB.lock().unwrap_or_else(|p| p.into_inner()),
        psb.map(std::sync::Arc::new),
    );
    drop(old);
}

/// The sent Formula's compiled bytes, while it is held.
pub fn sent() -> Option<std::sync::Arc<Vec<u8>>> {
    SENT_PSB.lock().unwrap_or_else(|p| p.into_inner()).clone()
}

/// Is `path` the sent Formula's ([`SENT`], maybe with options)?
pub fn is_sent(path: &str) -> bool {
    path.strip_prefix(SENT)
        .is_some_and(|r| r.is_empty() || r.starts_with('?'))
}

/// The names of the bundled Formulas, in the order installed.
pub fn bundled_names() -> impl Iterator<Item = &'static str> {
    BUNDLED.get().into_iter().flat_map(|l| l.iter().map(|(n, _)| *n))
}

/// A Formula's name from a file stem or a bundled name, for when its
/// `name "…"` line is not at hand: `endless-road` -> "Endless road".
pub fn stem_title(stem: &str) -> String {
    // (Stems are `[a-z0-9-]`: ASCII is enough, and keeps Unicode's case
    // tables out of the firmware.)
    let mut t = String::with_capacity(stem.len().min(MAX_FORMULA_NAME));
    for c in stem.chars().take(MAX_FORMULA_NAME) {
        let c = if c == '-' || c == '_' { ' ' } else { c };
        t.push(if t.is_empty() { c.to_ascii_uppercase() } else { c });
    }
    let t = t.trim();
    if t.is_empty() {
        "Formula".to_string()
    } else {
        t.to_string()
    }
}

/// A virtual stream holding a compiled Formula (read from a `.psb` file)
/// and its options: what the engine plays for a [`FormulaRef::File`]. The
/// marker can't come from a path ([`open`] refuses control characters).
pub(crate) const TEXT_MARK: &str = "@formula\u{1}";

/// The in-memory stream for a compiled Formula's bytes with `opts`
/// (`a=0.5&fps=…`): base64url, so the stream stays text.
pub fn open_formula_bytes(opts: &str, psb: &[u8]) -> Cursor<Vec<u8>> {
    let b64 = to_base64(psb);
    let mut bytes = Vec::with_capacity(4 + TEXT_MARK.len() + opts.len() + 1 + b64.len());
    bytes.extend_from_slice(&MAGIC);
    bytes.extend_from_slice(TEXT_MARK.as_bytes());
    bytes.extend_from_slice(opts.as_bytes());
    bytes.push(1);
    bytes.extend_from_slice(b64.as_bytes());
    Cursor::new(bytes)
}

/// Bytes as base64url without padding (a path's `psb=`).
pub fn to_base64(b: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut s = String::with_capacity(b.len() * 4 / 3 + 3);
    for ch in b.chunks(3) {
        let n = ch.iter().enumerate().fold(0u32, |n, (i, &v)| n | (v as u32) << (16 - 8 * i));
        for i in 0..=ch.len() {
            s.push(A[(n >> (18 - 6 * i) & 63) as usize] as char);
        }
    }
    s
}

/// [`to_base64`]'s inverse (`None` on anything else).
pub fn from_base64(s: &[u8]) -> Option<Vec<u8>> {
    let val = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'-' => 62,
            b'_' => 63,
            _ => return None,
        } as u32)
    };
    let mut o = Vec::with_capacity(s.len() * 3 / 4);
    for ch in s.chunks(4) {
        if ch.len() == 1 {
            return None;
        }
        let mut n = 0u32;
        for (i, &c) in ch.iter().enumerate() {
            n |= val(c)? << (18 - 6 * i);
        }
        for i in 0..ch.len() - 1 {
            o.push((n >> (16 - 8 * i)) as u8);
        }
    }
    Some(o)
}

/// A compiled Formula's name, read from its header without loading it
/// (`opendmd_psi::psb`'s format: "PSB", the ABI, flags, then the name).
pub fn compiled_name(psb: &[u8]) -> Option<String> {
    if psb.len() < 6 || &psb[..3] != b"PSB" {
        return None;
    }
    let n = psb[5] as usize;
    let name = std::str::from_utf8(psb.get(6..6 + n)?).ok()?;
    let name: String = name.chars().filter(|c| !c.is_control()).take(MAX_FORMULA_NAME).collect();
    (!name.trim().is_empty()).then_some(name)
}
/// Is `path` one of the live items there were before Formulas and GIFs
/// were all there is (`@clock[?…]`, `@text?msg=…`)? It names nothing now: a
/// playlist line is skipped with a log line ([`crate::manifest::scan`]),
/// `POST /api/play` answers 415.
pub fn is_retired_live(path: &str) -> bool {
    let base = path.split_once('?').map_or(path, |(b, _)| b);
    base == "@clock" || base == "@text"
}

/// Local wall-clock time, as Formulas read it (`hour`, `minute`, …).
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

/// Where Formulas read the local time: `None` while it is not known
/// (the firmware before its first SNTP sync).
pub type WallClock = fn() -> Option<WallTime>;

/// The installed [`WallClock`] as an address (0: none). An atomic rather
/// than a lock: it is written once at start-up and read once a second, and
/// a lock costs the web module ~0.3 KB.
static WALL: AtomicUsize = AtomicUsize::new(0);

/// Install the local-time source Formulas read (the firmware's
/// `clock.rs`, the browser's `Date`, the simulator's host clock). Until one
/// is installed, and whenever it answers `None`, the time is not known.
pub fn set_wall_clock(clock: Option<WallClock>) {
    WALL.store(clock.map_or(0, |f| f as usize), Ordering::Relaxed);
}

#[cfg(feature = "psi")]
pub(crate) fn wall_now() -> Option<WallTime> {
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
pub(crate) fn url_decode(b: &[u8]) -> String {
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

/// The `key=value` pairs after `?`, as bytes (only text needs decoding).
fn options(query: &str) -> impl Iterator<Item = (&[u8], &[u8])> {
    query
        .as_bytes()
        .split(|&b| b == b'&')
        .map(|kv| match kv.iter().position(|&b| b == b'=') {
            Some(i) => (&kv[..i], &kv[i + 1..]),
            None => (kv, &kv[kv.len()..]),
        })
}

/// Is `path` the start-up show with a Formula (`@boot?…&formula=…`), which
/// may be as long as a Formula's path?
fn is_boot_formula(path: &str) -> bool {
    path.len() <= MAX_FORMULA_PATH
        && path
            .strip_prefix(crate::boot::PATH)
            .and_then(|r| r.strip_prefix('?'))
            .is_some_and(|q| options(q).any(|(k, _)| k == b"formula"))
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
/// card's title, "Power on", "Startup", or a Formula's name
/// (its `name "…"` line; else `&name=` on its path, as an older playlist
/// entry has it; else "Formula"). `None` for anything else.
pub fn title(path: &str) -> Option<String> {
    // From the compiled Formula's header, without loading it, and in builds
    // without Psi (the web app's Recently played). The player ignores `name`.
    match formula_ref(path) {
        Some(FormulaRef::Bundled { name, .. }) if name == SENT_NAME => {
            return Some(
                sent()
                    .and_then(|b| compiled_name(&b))
                    .unwrap_or_else(|| "Formula".into()),
            );
        }
        Some(FormulaRef::Bundled { name, .. }) => {
            return Some(bundled(name).and_then(compiled_name).unwrap_or_else(|| stem_title(name)));
        }
        // A file's `name` line is read when it opens (the engine reports
        // it then: `FrameSource::formula_name`); till then, its stem.
        Some(FormulaRef::File { file, .. }) => {
            let stem = file.rsplit('/').next().unwrap_or(file);
            return Some(stem_title(&stem[..stem.len() - FORMULA_EXT.len()]));
        }
        _ => {}
    }
    if is_formula(path) {
        let q = || path[FORMULA.len()..].trim_start_matches('?').split('&');
        let clean = |n: String| {
            Some(
                n.chars()
                    .filter(|c| !c.is_control())
                    .take(MAX_FORMULA_NAME)
                    .collect::<String>(),
            )
            .filter(|n| !n.trim().is_empty())
        };
        // The name is in the first 330 characters of the base64.
        let name = q()
            .find_map(|kv| kv.strip_prefix("psb="))
            .and_then(|v| from_base64(&v.as_bytes()[..v.len().min(332) & !3]))
            .and_then(|b| compiled_name(&b))
            .and_then(clean)
            .or_else(|| {
                q().find_map(|kv| kv.strip_prefix("name="))
                    .map(|v| url_decode(v.as_bytes()))
                    .and_then(clean)
            });
        return Some(name.unwrap_or_else(|| "Formula".to_string()));
    }
    match parse(path)? {
        Spec::PowerOn => Some("Power on".to_string()),
        Spec::Card { title, .. } => Some(title),
        Spec::Fx(f) => Some(f.title().to_string()),
        Spec::Boot(_) => Some("Startup".to_string()),
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
    // (A control character names nothing, and would pass for the text
    // form's marker: `open_formula_text`.)
    if !path.bytes().any(|b| b < 0x20) {
        bytes.extend_from_slice(path.as_bytes());
    }
    Some(Cursor::new(bytes))
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Spec {
    PowerOn,
    Card { title: String, subtitle: String },
    Fx(crate::fx::Fx),
    Boot(BootSpec),
}

/// `@boot[?len=short|full|card|updating|installing&v=&ip=&ap=&note=&up=1&formula=]`:
/// the start-up show once (`len`, default short; `up=1`: the first boot of
/// a new version, "Updated to v…"), one loop of the status card, or two
/// seconds of a firmware update's still message, with its text from the
/// options ([`crate::boot`]). `formula=` (a bundled Formula by name or an
/// inline one, URL-encoded: what `boot_formula` holds) plays it as the show
/// in the wordmark's place ([`crate::boot::FormulaShow`]); one that does not
/// build, a card file's (read by the player, not from a path) or any without
/// the `psi` feature plays the wordmark, as the panel falls back to it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BootSpec {
    /// The status card instead of the show.
    card: bool,
    /// A firmware update's message instead ([`crate::boot::update_notice`]).
    notice: Option<crate::boot::UpdateStage>,
    /// The first boot of a new version ([`crate::boot::Info::updated`]).
    updated: bool,
    len: crate::boot::ShowLength,
    version: String,
    ip: String,
    ap: String,
    note: String,
    /// `formula=`, as given.
    formula: String,
    /// It, built (by [`SynthSource::new`]).
    #[cfg(feature = "psi")]
    show: Option<crate::boot::FormulaShow>,
}

fn parse_boot(query: &str) -> BootSpec {
    use crate::boot::ShowLength;
    let mut b = BootSpec {
        card: false,
        notice: None,
        updated: false,
        len: ShowLength::Short,
        version: String::new(),
        ip: String::new(),
        ap: String::new(),
        note: String::new(),
        formula: String::new(),
        #[cfg(feature = "psi")]
        show: None,
    };
    for (k, v) in options(query) {
        // One line of the small font across the panel at most.
        let text = || {
            url_decode(v)
                .trim()
                .chars()
                .map(text::fold_char)
                .take(32)
                .collect()
        };
        match k {
            b"len" => {
                b.notice = match v {
                    b"updating" => Some(crate::boot::UpdateStage::Receiving),
                    b"installing" => Some(crate::boot::UpdateStage::Installing),
                    b"installing-dark" => Some(crate::boot::UpdateStage::InstallingDark),
                    _ => None,
                };
                (b.card, b.len) = (
                    v == b"card",
                    if v == b"full" {
                        ShowLength::Full
                    } else {
                        ShowLength::Short
                    },
                )
            }
            b"v" => b.version = text(),
            b"ip" => b.ip = text(),
            b"ap" => b.ap = text(),
            b"note" => b.note = text(),
            b"up" => b.updated = v == b"1",
            b"formula" => b.formula = url_decode(v),
            _ => {}
        }
    }
    b
}

fn parse(path: &str) -> Option<Spec> {
    if path == POWER_ON {
        return Some(Spec::PowerOn);
    }
    if is_formula(path) {
        return crate::fx::Fx::parse(path).map(Spec::Fx);
    }
    let (base, query) = path.split_once('?').unwrap_or((path, ""));
    if base == crate::boot::PATH {
        return Some(Spec::Boot(parse_boot(query)));
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
    /// An effect's position in its loop: the sum of the frame times it
    /// returned (a formula's governor may hold frames longer).
    elapsed: Duration,
    /// A Formula read from its file: its `name` line.
    name: Option<String>,
}

impl<R: Read> SynthSource<R> {
    /// `reader` is the whole stream, magic included (as `FrameSource`
    /// hands it over).
    pub fn new(mut reader: R) -> io::Result<Self> {
        let mut raw = Vec::new();
        (&mut reader)
            .take(MAX_READ as u64 + 4)
            .read_to_end(&mut raw)?;
        let path = raw
            .strip_prefix(&MAGIC)
            .and_then(|p| std::str::from_utf8(p).ok())
            .unwrap_or("");
        // A compiled Formula, read from its file (`open_formula_bytes`).
        if let Some(rest) = path.strip_prefix(TEXT_MARK) {
            let (opts, text) = rest.split_once('\u{1}').unwrap_or((rest, ""));
            #[cfg(not(feature = "psi"))]
            {
                let _ = (opts, text);
                return Err(io::Error::new(io::ErrorKind::InvalidData, "unknown virtual file"));
            }
            #[cfg(feature = "psi")]
            {
                let psb = from_base64(text.as_bytes()).unwrap_or_default();
                let spec = crate::fx_psi::PsiFx::from_bytes(&psb, opts)
                    .map(|f| Spec::Fx(crate::fx::Fx::Psi(f)))
                    .map_err(crate::fx_psi::io_error)?;
                let name = compiled_name(&psb);
                return Ok(SynthSource {
                    reader,
                    spec,
                    frames: 0,
                    table: None,
                    elapsed: Duration::ZERO,
                    name,
                });
            }
        }
        // Only a formula (or the show with one) may be longer than a
        // virtual path.
        let path = if path.len() > MAX_PATH && !is_formula(path) && !is_boot_formula(path) {
            ""
        } else {
            path
        };
        let Some(mut spec) = parse(path) else {
            #[cfg(feature = "psi")]
            if let Some(e) = crate::fx_psi::open_error(path) {
                return Err(e);
            }
            return Err(io::Error::new(io::ErrorKind::InvalidData, "unknown virtual file"));
        };
        // The show's Formula, if it builds (else the wordmark, as on the
        // panel).
        #[cfg(feature = "psi")]
        if let Spec::Boot(b) = &mut spec {
            if !b.formula.is_empty() {
                b.show = crate::boot::FormulaShow::open(&b.formula).ok();
            }
        }
        #[cfg(not(feature = "psi"))]
        let _ = &mut spec;
        Ok(SynthSource {
            reader,
            spec,
            frames: 0,
            table: None,
            elapsed: Duration::ZERO,
            name: None,
        })
    }

    /// A Formula read from its file: its `name "…"` line (what the engine
    /// reports as the title), if it has one.
    pub fn formula_name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// A formula on screen takes `path`'s formula in place, keeping its
    /// clock and memory (a live edit: no transition, no restart). `None` if
    /// this or `path` is not a formula; `Some(Err)` (`InvalidInput`, the
    /// compiler's sentence) if it does not compile.
    #[cfg(feature = "psi")]
    pub fn retune(&mut self, path: &str) -> Option<io::Result<()>> {
        let Spec::Fx(crate::fx::Fx::Psi(cur)) = &mut self.spec else {
            return None;
        };
        if !is_formula(path) {
            return None;
        }
        Some(cur.retune_from(&path[FORMULA.len()..]).map_err(crate::fx_psi::io_error))
    }

    /// Start again from the first frame without reopening: a loop.
    pub fn rewind(&mut self) {
        self.frames = 0;
        self.elapsed = Duration::ZERO;
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
            Spec::Fx(f) => {
                if self.elapsed >= f.loop_time() {
                    return Ok(None);
                }
                let d = f.frame(self.elapsed, canvas);
                self.elapsed += d;
                d
            }
            Spec::Boot(b) => {
                let delay = crate::boot::frame_delay();
                let ms = delay.as_millis() as u32;
                let t = self.frames as u32 * ms;
                let length = if b.notice.is_some() {
                    NOTICE_MS
                } else if b.card {
                    crate::boot::CARD_LOOP_MS
                } else if b.updated {
                    b.len.min_ms_updated()
                } else {
                    b.len.min_ms()
                };
                if t >= length {
                    return Ok(None);
                }
                let info = crate::boot::Info {
                    version: &b.version,
                    ip: &b.ip,
                    ap: &b.ap,
                    note: &b.note,
                    updated: b.updated,
                };
                #[cfg(feature = "psi")]
                let shown = match (&mut b.show, b.notice.is_none() && !b.card) {
                    (Some(s), true) => {
                        s.frame(canvas, b.len, t, &info);
                        true
                    }
                    _ => false,
                };
                #[cfg(not(feature = "psi"))]
                let shown = false;
                if !shown {
                    if let Some(stage) = b.notice {
                        crate::boot::update_notice(canvas, stage, &b.version);
                    } else if b.card {
                        crate::boot::card(canvas, t, &info, None);
                    } else {
                        crate::boot::show(canvas, b.len, t, &info);
                    }
                }
                delay
            }
        };
        self.frames += 1;
        Ok(Some(delay))
    }
}

/// The information line of `@boot?…` (its `len`, `v`, `ip`, `ap`, `note`
/// and `up`) at `t` ms over a Formula's frame already in `canvas`
/// ([`crate::boot::formula_line`]): how the web app, whose main module
/// draws no Formulas, previews a Formula start-up show over the Formulas
/// module's frame. Nothing for another path.
pub fn boot_line(canvas: &mut Canvas, path: &str, t: u32) {
    let Some(q) = path.strip_prefix(crate::boot::PATH) else { return };
    if !(q.is_empty() || q.starts_with('?')) {
        return;
    }
    let b = parse_boot(q.trim_start_matches('?'));
    let info = crate::boot::Info {
        version: &b.version,
        ip: &b.ip,
        ap: &b.ap,
        note: &b.note,
        updated: b.updated,
    };
    crate::boot::formula_line(canvas, b.len, t, &info);
}

/// How long `@boot?len=updating|installing` plays its still message.
const NOTICE_MS: u32 = 2_000;

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

/// The bundled list every test installs (the list is set once per
/// process), compiled here.
#[cfg(all(test, feature = "psi"))]
pub(crate) fn install_test_bundled() {
    #[cfg(feature = "psi")]
    {
        let list: Vec<(&'static str, &'static [u8])> = [
            ("red", "name \"Red one\"\nrgb(1, 0, 0)"),
            ("slider", "control k = 0.25\nrgb(k, 0, 0)"),
        ]
        .iter()
        .map(|&(n, s)| (n, &*Box::leak(psi_build(s).into_boxed_slice())))
        .collect();
        set_bundled(Box::leak(list.into_boxed_slice()));
    }
}

/// A source compiled for the player (tests).
#[cfg(all(test, feature = "psi"))]
pub(crate) fn psi_build(src: &str) -> Vec<u8> {
    opendmd_psi::build(src).unwrap_or_else(|e| panic!("{}", e.render(src)))
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
    fn the_live_items_are_gone() {
        // `@clock` and `@text` name nothing now: no title, no picture.
        for p in ["@clock", "@clock?style=small&date=1", "@text?msg=Hi&scroll=0"] {
            assert!(is_retired_live(p) && is_virtual(p), "{p}");
            assert_eq!(title(p), None, "{p}");
            assert!(SynthSource::new(open(p).unwrap()).is_err(), "{p}");
        }
        for p in ["@clockx", "@texts", "@card:Clock|", "@boot", "/gifs/@clock.gif"] {
            assert!(!is_retired_live(p), "{p}");
        }
        assert_eq!(url_decode(b"a+b%41%zz%4"), "a bA%zz%4");
    }

    #[test]
    fn formula_titles_and_retired_effects() {
        // A retired built-in effect names nothing.
        assert_eq!(title("@fx:tunnel?cycle=1"), None);
        assert_eq!(title("@fx:psi?src=gray(x)"), None);
        assert!(is_retired_effect("@fx:psi?src=gray(x)") && !is_retired_effect("@formula?src=gray(x)"));
        // A formula is "Formula", compiled or not, in any build.
        assert_eq!(title("@formula?src=gray(x)&a=0.5").as_deref(), Some("Formula"));
        // A playlist entry carries its name: decoded, cut to 60, no controls.
        assert_eq!(
            title("@formula?src=gray(x)&a=0.5&name=Star+field%21").as_deref(),
            Some("Star field!")
        );
        assert_eq!(
            title("@formula?src=name%3Dx&name=").as_deref(),
            Some("Formula")
        );
        assert_eq!(
            title("@formula?src=gray(x)&name=%0A+").as_deref(),
            Some("Formula")
        );
        let long = format!("@formula?src=gray(x)&name={}", "é".repeat(80));
        assert_eq!(title(&long).unwrap(), "é".repeat(MAX_FORMULA_NAME));
        assert!(
            is_formula("@formula") && !is_formula("@formulax?src=1") && !is_formula("@fx:psi?src=1")
        );
    }

    /// A 2.4 s formula at 50 fps, then a 1.6 s one at 100.
    #[cfg(feature = "psi")]
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
        let a = crate::fx_psi::path("loop 2.4\nfps 50\ngray(frac(x + t/2.4))", "");
        let b = crate::fx_psi::path("loop 1.6\nfps 100\ngray(frac(y + t/1.6))", "");
        let m = format!("{a}\n{b}\n");
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
        // The first loop is 2.4 s: two loops cover the 3 s minimum.
        assert_eq!(starts[0], (0, a.into()));
        assert_eq!(starts[1], (4800, b.into()));
        // 4.8 s at 50 frames a second, then 3.2 s at the default 100.
        assert!((555..=565).contains(&frames), "{frames}");
    }

    #[cfg(feature = "psi")]
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
        let formula = crate::fx_psi::path("loop 2.4\nfps 100\ngray(frac(x + t/2.4))", "");
        for (path, want) in [(formula, 600)] {
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

    /// A playlist entry's `&name=` is for the playlist and the Now card: the
    /// player ignores it (the same frames as the path without it).
    #[cfg(feature = "psi")]
    #[test]
    fn a_formula_name_on_the_path_plays_the_same() {
        let src = "loop 1\nhsv(x + t, 1, a)";
        let frames = |p: &str| {
            let mut s = SynthSource::new(open(p).unwrap()).unwrap();
            let mut c = Canvas::new();
            let mut all = Vec::new();
            while s.next_frame(&mut c).unwrap().is_some() {
                all.extend((0..WIDTH * HEIGHT).map(|i| c.get(i % WIDTH, i / WIDTH)));
            }
            all
        };
        let plain = crate::fx_psi::path(src, "a=0.5");
        let named = crate::fx_psi::path(src, "a=0.5&name=Rainbow+%26+co");
        assert_eq!(title(&named).as_deref(), Some("Rainbow & co"));
        assert_eq!(frames(&plain), frames(&named));
    }

    #[cfg(feature = "psi")]
    #[test]
    fn a_formulas_title_is_its_name_line() {
        let p = |src: &str, rest: &str| crate::fx_psi::path(src, rest.trim_start_matches('&'));
        assert_eq!(title(&p("name \"Big clock\"; gray(x)", "")).as_deref(), Some("Big clock"));
        // Before `&name=`, which older lines carry.
        assert_eq!(title(&p("name \"Mine\"; gray(x)", "&name=Old")).as_deref(), Some("Mine"));
        assert_eq!(title(&p("gray(x)", "&name=Old")).as_deref(), Some("Old"));
        assert_eq!(title(&p("psi 1; control k = 0.5 \"a; b -- c\"; name \"Ring; road\"; gray(k)", "")).as_deref(), Some("Ring; road"));
        assert_eq!(title(&p("gray(x)", "")).as_deref(), Some("Formula"));
        // Empty or only spaces: the next answer.
        assert_eq!(title(&p("name \" \"; gray(x)", "&name=Kept")).as_deref(), Some("Kept"));
        // Cut to 60 characters (the compiler refuses longer).
        let long = p(&format!("name \"{}\"; gray(x)", "é".repeat(60)), "");
        assert_eq!(title(&long).unwrap(), "é".repeat(MAX_FORMULA_NAME));
    }

    #[test]
    fn formulas_by_reference_parse() {
        use FormulaRef::*;
        assert_eq!(formula_ref("@formula:endless-road"), Some(Bundled { name: "endless-road", opts: "" }));
        assert_eq!(
            formula_ref("@formula:endless-road?a=0.4&fps=50"),
            Some(Bundled { name: "endless-road", opts: "a=0.4&fps=50" })
        );
        assert_eq!(formula_ref("@formula?src=gray(x)"), Some(Inline));
        assert_eq!(formula_ref("@formula"), Some(Inline));
        assert_eq!(
            formula_ref("/formulas/my-road.psi?a=1"),
            Some(File { file: "/formulas/my-road.psi", opts: "a=1" })
        );
        assert_eq!(
            formula_ref("/formulas/X.PSI"),
            Some(File { file: "/formulas/X.PSI", opts: "" })
        );
        for not in ["/gifs/a.gif", "/gifs/a.gif?a=1", "formulas/x.psi", "/formulas/.psi", "/x.psix", "@formulax?src=1", "@clock", ""] {
            assert_eq!(formula_ref(not), None, "{not}");
        }
        assert!(is_formula("@formula:x") && !is_formula("/formulas/x.psi"));
        assert!(valid_formula_name("endless-road") && valid_formula_name("a1"));
        for bad in ["", "Endless", "../x", "a/b", "a b", "a.psi", &"a".repeat(41)] {
            assert!(!valid_formula_name(bad), "{bad}");
        }
        assert_eq!(stem_title("endless-road"), "Endless road");
        assert_eq!(stem_title("my_formula-2"), "My formula 2");
        assert_eq!(stem_title("-"), "Formula");
    }

    #[cfg(feature = "psi")]
    #[test]
    fn a_formula_by_reference_has_a_title() {
        install_test_bundled();
        assert_eq!(title("@formula:red?a=1").as_deref(), Some("Red one"));
        // No `name` line: from the name.
        assert_eq!(title("@formula:slider").as_deref(), Some("Slider"));
        // Not bundled: still from the name (it plays nothing: a 404).
        assert_eq!(title("@formula:big-clock").as_deref(), Some("Big clock"));
        assert_eq!(title("/formulas/my-road.psi?a=0.5").as_deref(), Some("My road"));
        let e = crate::manifest::Entry::from_path("/formulas/my-road.psi");
        assert_eq!((e.title.as_str(), e.category), ("My road", None));
        assert_eq!(crate::manifest::Entry::from_path("@formula:red").title, "Red one");
    }

    #[test]
    fn a_path_with_a_control_character_names_nothing() {
        // (It would pass for a Formula's text: `open_formula_text`.)
        let mut raw = Vec::new();
        open("@formula\u{1}\u{1}gray(x)").unwrap().read_to_end(&mut raw).unwrap();
        assert_eq!(raw, MAGIC);
        assert!(SynthSource::new(open("@formula\u{1}\u{1}gray(x)").unwrap()).is_err());
    }
}

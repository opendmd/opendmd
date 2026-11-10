//! Per-entry overrides: what a user said about one GIF *in one playlist* —
//! favourite, play less, play longer, loop N times.
//!
//! # Card format
//!
//! A sidecar text file beside the playlist: `/sets/<name>.meta` for
//! `/sets/<name>.txt`, `/filelist.meta` for the default `/filelist.txt`, and
//! `/filelist.<x>.meta` for any other root manifest `/filelist.<x>` (see
//! [`meta_path`]). One line per overridden entry: the GIF's path exactly as
//! the playlist names it, then tab-separated fields:
//!
//! ```text
//! /gifs/Consoles/SNES_Plok_08.gif\tw=3
//! /gifs/Consoles/MD_Sonic3_spritefox11.gif\tdwell=45\tloops=3
//! /gifs/Other/X.gif\tdwell=2x
//! ```
//!
//! | field | meaning |
//! |---|---|
//! | `w=<f>` | relative weight in shuffle (`3` = favourite, `0.5` = play less; 1 is the default) |
//! | `dwell=<s>` | holds the panel for `s` seconds (1..=86,400) |
//! | `dwell=<k>x` | the configured dwell times `k` (0 < k <= 100) |
//! | `loops=<n>` | plays exactly `n` loops (1..=1,000) and moves on |
//!
//! Parsing is tolerant in the manifest parser's way (BOM, CRLF, blank lines,
//! `#` comment lines, trailing whitespace, unknown or malformed fields are
//! skipped: earlier builds' `never` among them, so a GIF once hidden plays
//! again); [`render_meta`] writes the canonical form — fields in the order
//! `w`, `dwell`, `loops`, tab-separated, LF-terminated lines — and a
//! canonical file round-trips byte for byte.
//!
//! # How the player uses it
//!
//! The playlist index build ([`crate::index::write_index_with_overrides`])
//! holds an [`OverrideSet`] — a 64-bit key per overridden path, never the
//! paths themselves — and resolves it against the playlist while it streams
//! the manifest, producing [`Overrides`]: `(playlist index, Override)` sorted
//! by index. Those are stored in the `.idx` after the offsets, so reopening a
//! current index costs no second pass. The engine turns `w` into the
//! scheduler's sparse weights and consults `dwell`/`loops` when it opens
//! an entry.

use std::io::{self, BufRead};

/// At most this many overrides per playlist are honoured; the rest are
/// logged and ignored.
pub const MAX_OVERRIDES: usize = 1024;

/// [`fingerprint`] of "no meta file at all".
pub const NO_META: u64 = 0;

/// How long one entry holds the panel, if not the configured dwell.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Dwell {
    /// Exactly this many seconds.
    Secs(u32),
    /// The configured dwell, times this.
    Times(f32),
}

/// What a user said about one entry. `Default` is "nothing".
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Override {
    /// Relative weight in shuffle; `None` is the default 1.
    pub weight: Option<f32>,
    pub dwell: Option<Dwell>,
    /// Plays exactly this many loops.
    pub loops: Option<u16>,
}

impl Override {
    /// Nothing is overridden: the line would be meaningless.
    pub fn is_empty(&self) -> bool {
        *self == Override::default()
    }

    /// Does it change how often the entry is drawn?
    pub fn affects_weight(&self) -> bool {
        self.weight.is_some()
    }

    /// Does it change how long the entry plays?
    pub fn affects_dwell(&self) -> bool {
        self.dwell.is_some() || self.loops.is_some()
    }

    /// Is every value in range — what the parser would have accepted? (For
    /// overrides built by hand, e.g. from the web app's JSON.)
    pub fn is_valid(&self) -> bool {
        self.weight.is_none_or(valid_weight)
            && match self.dwell {
                Some(Dwell::Secs(s)) => valid_secs(s),
                Some(Dwell::Times(k)) => valid_times(k),
                None => true,
            }
            && self.loops.is_none_or(valid_loops)
    }

    /// Apply one `key=value` field. False if it was not
    /// understood (unknown key, bad value); the override is then unchanged.
    fn apply_field(&mut self, field: &str) -> bool {
        let (key, value) = crate::sidecar::key_value(field);
        match (key, value) {
            ("w", Some(v)) => match parse_decimal(v) {
                Some(w) if valid_weight(w) => self.weight = Some(w),
                _ => return false,
            },
            ("dwell", Some(v)) => {
                if let Some(k) = v.strip_suffix(['x', 'X']) {
                    match parse_decimal(k) {
                        Some(k) if valid_times(k) => self.dwell = Some(Dwell::Times(k)),
                        _ => return false,
                    }
                } else {
                    match v.parse::<u32>() {
                        Ok(s) if valid_secs(s) => self.dwell = Some(Dwell::Secs(s)),
                        _ => return false,
                    }
                }
            }
            ("loops", Some(v)) => match v.parse::<u16>() {
                Ok(n) if valid_loops(n) => self.loops = Some(n),
                _ => return false,
            },
            _ => return false,
        }
        true
    }

    /// The canonical fields, each preceded by a tab.
    fn render_fields(&self, out: &mut String) {
        if let Some(w) = self.weight {
            out.push_str("\tw=");
            push_decimal(out, w);
        }
        match self.dwell {
            Some(Dwell::Secs(s)) => {
                out.push_str("\tdwell=");
                out.push_str(&s.to_string());
            }
            Some(Dwell::Times(k)) => {
                out.push_str("\tdwell=");
                push_decimal(out, k);
                out.push('x');
            }
            None => {}
        }
        if let Some(n) = self.loops {
            out.push_str("\tloops=");
            out.push_str(&n.to_string());
        }
    }
}

fn valid_weight(w: f32) -> bool {
    (0.0..=1000.0).contains(&w)
}

fn valid_times(k: f32) -> bool {
    k > 0.0 && k <= 100.0
}

fn valid_secs(s: u32) -> bool {
    (1..=86_400).contains(&s)
}

fn valid_loops(n: u16) -> bool {
    (1..=1000).contains(&n)
}

/// `123`, `0.5`, `.25`, `2.` — a non-negative decimal with at most three
/// places and at most six digits before the point. Deliberately not
/// `str::parse::<f32>`, whose code (and `f32`'s `Display`) the web app's
/// wasm would otherwise carry just for this.
fn parse_decimal(s: &str) -> Option<f32> {
    let (int, frac) = s.split_once('.').unwrap_or((s, ""));
    if (int.is_empty() && frac.is_empty())
        || int.len() > 6
        || frac.len() > 3
        || !int.bytes().chain(frac.bytes()).all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let digits = |t: &str| t.bytes().fold(0u32, |n, b| n * 10 + u32::from(b - b'0'));
    let milli = digits(int) * 1000 + digits(frac) * 10u32.pow(3 - frac.len() as u32);
    Some(milli as f32 / 1000.0)
}

/// The canonical text of a non-negative value, as the `.meta` writes it
/// (and a JSON number reads): `3`, `0.5`, `1.25`.
pub fn format_decimal(v: f32) -> String {
    let mut out = String::new();
    push_decimal(&mut out, v);
    out
}

/// The canonical text of a non-negative value: rounded to three places,
/// trailing zeros (and a bare point) dropped — `3`, `0.5`, `1.25`.
fn push_decimal(out: &mut String, v: f32) {
    let milli = (v.max(0.0) * 1000.0 + 0.5) as u32;
    out.push_str(&(milli / 1000).to_string());
    let frac = milli % 1000;
    if frac != 0 {
        let mut f = format!("{frac:03}");
        while f.ends_with('0') {
            f.pop();
        }
        out.push('.');
        out.push_str(&f);
    }
}

/// One line of a `.meta` file.
#[derive(Clone, Debug, PartialEq)]
pub struct MetaLine {
    pub path: String,
    pub ov: Override,
}

/// Parse one raw line: `None` for a blank line, a comment, or a line that
/// overrides nothing it understood.
fn parse_line(raw: &str) -> Option<(&str, Override)> {
    let (path, parts) = crate::sidecar::split_line(raw)?;
    let mut ov = Override::default();
    for field in parts {
        if !field.trim().is_empty() && !ov.apply_field(field) {
            log::debug!("{path}: ignoring override field '{field}'");
        }
    }
    (!ov.is_empty()).then_some((path, ov))
}

/// Parse a whole `.meta` file. Later lines for the same path replace earlier
/// ones (as the player resolves them); order is otherwise preserved.
pub fn parse_meta(text: &str) -> Vec<MetaLine> {
    let mut out: Vec<MetaLine> = Vec::new();
    for raw in text.split('\n') {
        if let Some((path, ov)) = parse_line(raw) {
            match out.iter_mut().find(|l| l.path == path) {
                Some(l) => l.ov = ov,
                None => out.push(MetaLine {
                    path: path.to_string(),
                    ov,
                }),
            }
        }
    }
    out
}

/// The canonical text of `lines`: one LF-terminated line each, empty
/// overrides dropped.
pub fn render_meta(lines: &[MetaLine]) -> String {
    let mut out = String::with_capacity(lines.len() * 64);
    for l in lines.iter().filter(|l| !l.ov.is_empty()) {
        out.push_str(&l.path);
        l.ov.render_fields(&mut out);
        out.push('\n');
    }
    out
}

/// Stream a `.meta` file one line at a time: `visit(path, override)` for
/// every line that overrides something. One line in memory at a time.
pub fn scan_meta<R: BufRead>(
    mut reader: R,
    mut visit: impl FnMut(&str, Override),
) -> io::Result<()> {
    let mut raw = String::new();
    loop {
        raw.clear();
        if reader.read_line(&mut raw)? == 0 {
            return Ok(());
        }
        if let Some((path, ov)) = parse_line(&raw) {
            visit(path, ov);
        }
    }
}

pub use crate::sidecar::{fingerprint, path_key};

/// A parsed `.meta`, held only while the index is built: a sorted
/// `(path_key, Override)` per line — 8 bytes of key instead of a path.
#[derive(Clone, Debug, Default)]
pub struct OverrideSet {
    entries: Vec<(u64, Override)>,
    /// Lines past [`MAX_OVERRIDES`] that were dropped.
    pub dropped: usize,
}

impl OverrideSet {
    pub fn new() -> Self {
        Self::default()
    }

    /// Stream-parse a `.meta` file. A later line for the same path replaces
    /// an earlier one; lines past [`MAX_OVERRIDES`] are counted and dropped.
    pub fn read<R: BufRead>(reader: R) -> io::Result<Self> {
        let mut set = OverrideSet::new();
        scan_meta(reader, |path, ov| set.insert(path, ov))?;
        if set.dropped > 0 {
            log::warn!(
                "{} overrides past the first {MAX_OVERRIDES} ignored",
                set.dropped
            );
        }
        Ok(set)
    }

    pub fn from_lines(lines: &[MetaLine]) -> Self {
        let mut set = OverrideSet::new();
        for l in lines {
            set.insert(&l.path, l.ov);
        }
        set
    }

    pub fn insert(&mut self, path: &str, ov: Override) {
        let key = path_key(path);
        match self.entries.binary_search_by_key(&key, |(k, _)| *k) {
            Ok(i) => self.entries[i].1 = ov,
            Err(_) if self.entries.len() >= MAX_OVERRIDES => self.dropped += 1,
            Err(i) => self.entries.insert(i, (key, ov)),
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The override for a playlist path, if there is one.
    pub fn get(&self, path: &str) -> Option<Override> {
        let key = path_key(path);
        self.entries
            .binary_search_by_key(&key, |(k, _)| *k)
            .ok()
            .map(|i| self.entries[i].1)
    }
}

/// Overrides resolved against one playlist: `(index, Override)` sorted by
/// index, at most [`MAX_OVERRIDES`] of them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Overrides {
    entries: Vec<(u32, Override)>,
}

impl Overrides {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add the override for playlist index `index`. Indices must arrive in
    /// increasing order (they do, from a manifest pass); an out-of-order one
    /// or one past the cap is refused.
    pub fn push(&mut self, index: u32, ov: Override) -> bool {
        if self.entries.len() >= MAX_OVERRIDES
            || self.entries.last().is_some_and(|(i, _)| *i >= index)
        {
            return false;
        }
        self.entries.push((index, ov));
        true
    }

    pub fn get(&self, index: usize) -> Option<&Override> {
        let index = u32::try_from(index).ok()?;
        self.entries
            .binary_search_by_key(&index, |(i, _)| *i)
            .ok()
            .map(|k| &self.entries[k].1)
    }

    pub fn iter(&self) -> impl Iterator<Item = &(u32, Override)> {
        self.entries.iter()
    }

    /// The overrides themselves, mutably (their indices stay as they are).
    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut Override> {
        self.entries.iter_mut().map(|(_, o)| o)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// `(weighted, timed)` counts, for logs and status.
    pub fn summary(&self) -> (usize, usize) {
        let weighted = self
            .entries
            .iter()
            .filter(|(_, o)| o.weight.is_some())
            .count();
        let timed = self
            .entries
            .iter()
            .filter(|(_, o)| o.affects_dwell())
            .count();
        (weighted, timed)
    }
}

/// Bytes per override record in a playlist index.
pub const RECORD_LEN: usize = 16;

// Bit 1 was `never` (hidden from the playlist), retired: an index an
// earlier build wrote may still set it, and decoding ignores it.
const F_WEIGHT: u8 = 2;
const F_DWELL_SECS: u8 = 4;
const F_DWELL_TIMES: u8 = 8;

/// One resolved override as it is stored in a `.idx` (little-endian):
/// `index: u32`, `weight: f32`, `dwell: u32` (seconds, or the multiplier's
/// `f32` bits), `loops: u16` (0 = none), `flags: u8`, one reserved byte.
pub fn encode_record(index: u32, ov: &Override) -> [u8; RECORD_LEN] {
    let mut b = [0u8; RECORD_LEN];
    let mut flags = 0u8;
    b[0..4].copy_from_slice(&index.to_le_bytes());
    if let Some(w) = ov.weight {
        flags |= F_WEIGHT;
        b[4..8].copy_from_slice(&w.to_le_bytes());
    }
    match ov.dwell {
        Some(Dwell::Secs(s)) => {
            flags |= F_DWELL_SECS;
            b[8..12].copy_from_slice(&s.to_le_bytes());
        }
        Some(Dwell::Times(k)) => {
            flags |= F_DWELL_TIMES;
            b[8..12].copy_from_slice(&k.to_le_bytes());
        }
        None => {}
    }
    b[12..14].copy_from_slice(&ov.loops.unwrap_or(0).to_le_bytes());
    b[14] = flags;
    b
}

/// The inverse of [`encode_record`].
pub fn decode_record(b: &[u8; RECORD_LEN]) -> (u32, Override) {
    let u32_at = |i: usize| u32::from_le_bytes(b[i..i + 4].try_into().unwrap());
    let flags = b[14];
    let loops = u16::from_le_bytes([b[12], b[13]]);
    let ov = Override {
        weight: (flags & F_WEIGHT != 0).then(|| f32::from_bits(u32_at(4))),
        dwell: if flags & F_DWELL_SECS != 0 {
            Some(Dwell::Secs(u32_at(8)))
        } else if flags & F_DWELL_TIMES != 0 {
            Some(Dwell::Times(f32::from_bits(u32_at(8))))
        } else {
            None
        },
        loops: (loops > 0).then_some(loops),
    };
    (u32_at(0), ov)
}

/// The `.meta` path for a manifest path: `…/<name>.txt` -> `…/<name>.meta`,
/// anything else (`/filelist.hall`) -> `/filelist.hall.meta`. Works on card
/// paths and on filesystem paths alike.
pub fn meta_path(manifest: &str) -> String {
    match manifest.strip_suffix(".txt") {
        Some(stem) => format!("{stem}.meta"),
        None => format!("{manifest}.meta"),
    }
}

/// The manifest a `.meta` path belongs to, the inverse of [`meta_path`]:
/// `/sets/<n>.meta` -> `/sets/<n>.txt` (sets are always `.txt`),
/// `/filelist.meta` -> `/filelist.txt`, `/filelist.<x>.meta` ->
/// `/filelist.<x>`. `None` if it is not a `.meta` path.
pub fn manifest_for_meta(meta: &str) -> Option<String> {
    let stem = meta.strip_suffix(".meta")?;
    let (dir, name) = stem.rsplit_once('/').unwrap_or(("", stem));
    if name.is_empty() {
        return None;
    }
    let in_sets = dir.rsplit('/').next() == Some("sets");
    Some(if in_sets || name == "filelist" {
        format!("{stem}.txt")
    } else {
        stem.to_string()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const CANONICAL: &str = "/gifs/Consoles/SNES_Plok_08.gif\tw=3\n\
                             /gifs/Arcade/ARCADE_Pac-Man_01.gif\tw=0\n\
                             /gifs/NEW 2021/MD_Sonic3_spritefox11.gif\tdwell=45\tloops=3\n\
                             /gifs/Other/X.gif\tw=0.5\tdwell=2x\n\
                             /gifs/Other/Y.gif\tdwell=1.5x\n";

    #[test]
    fn canonical_round_trips_byte_for_byte() {
        let lines = parse_meta(CANONICAL);
        assert_eq!(lines.len(), 5);
        assert_eq!(lines[0].ov.weight, Some(3.0));
        assert_eq!(lines[1].ov.weight, Some(0.0));
        assert_eq!(lines[2].path, "/gifs/NEW 2021/MD_Sonic3_spritefox11.gif");
        assert_eq!(lines[2].ov.dwell, Some(Dwell::Secs(45)));
        assert_eq!(lines[2].ov.loops, Some(3));
        assert_eq!(lines[3].ov.dwell, Some(Dwell::Times(2.0)));
        assert_eq!(lines[4].ov.dwell, Some(Dwell::Times(1.5)));
        assert_eq!(render_meta(&lines), CANONICAL);
    }

    #[test]
    fn parsing_is_tolerant_and_rendering_canonical() {
        let messy = "\u{feff}# hand-edited\r\n\
                     \r\n\
                     /gifs/A/NES_A_01.gif\tloops=2\tnever\tw=0  \r\n\
                     /gifs/A/NES_B_01.gif\tbogus=1\tw=abc\tdwell=0\n\
                     /gifs/A/NES_C_01.gif\tw=2\tcolour=red\n\
                     /gifs/A/NES_C_01.gif\tw=4\n\
                     /gifs/A/NES_D_01.gif";
        let lines = parse_meta(messy);
        // B has nothing valid, D has no fields: both dropped. C's later line
        // wins. A's `never` (retired) is skipped like any unknown field.
        assert_eq!(lines.len(), 2);
        assert_eq!(
            render_meta(&lines),
            "/gifs/A/NES_A_01.gif\tw=0\tloops=2\n/gifs/A/NES_C_01.gif\tw=4\n"
        );
        assert_eq!(
            render_meta(&parse_meta(&render_meta(&lines))),
            render_meta(&lines)
        );
    }

    #[test]
    fn decimals_parse_and_print_canonically() {
        for (text, value, canon) in [
            ("3", 3.0, "3"),
            ("0.5", 0.5, "0.5"),
            (".25", 0.25, "0.25"),
            ("2.", 2.0, "2"),
            ("1.250", 1.25, "1.25"),
            ("0.333", 0.333, "0.333"),
            ("1000", 1000.0, "1000"),
        ] {
            let v = parse_decimal(text).unwrap();
            assert_eq!(v, value, "{text}");
            let mut out = String::new();
            push_decimal(&mut out, v);
            assert_eq!(out, canon);
        }
        for bad in [
            "", ".", "1.2345", "1e3", "-1", "+1", "1,5", "1234567", "0x1",
        ] {
            assert_eq!(parse_decimal(bad), None, "{bad}");
        }
    }

    #[test]
    fn hand_built_overrides_are_range_checked() {
        let ok = Override {
            weight: Some(0.5),
            dwell: Some(Dwell::Times(2.0)),
            loops: Some(3),
        };
        assert!(ok.is_valid());
        for bad in [
            Override {
                weight: Some(-1.0),
                ..ok
            },
            Override {
                weight: Some(f32::NAN),
                ..ok
            },
            Override {
                dwell: Some(Dwell::Secs(0)),
                ..ok
            },
            Override {
                dwell: Some(Dwell::Times(0.0)),
                ..ok
            },
            Override {
                loops: Some(0),
                ..ok
            },
        ] {
            assert!(!bad.is_valid(), "{bad:?}");
        }
    }

    #[test]
    fn out_of_range_values_are_refused() {
        for bad in [
            "w=-1",
            "w=inf",
            "w=NaN",
            "w=1e3",
            "dwell=0",
            "dwell=86401",
            "dwell=0x",
            "dwell=101x",
            "loops=0",
            "loops=1001",
            "never",
            "never=1",
        ] {
            assert!(
                parse_meta(&format!("/g.gif\t{bad}\n")).is_empty(),
                "{bad} was accepted"
            );
        }
    }

    #[test]
    fn scan_meta_matches_parse_meta() {
        let mut seen = Vec::new();
        scan_meta(CANONICAL.as_bytes(), |p, ov| seen.push((p.to_string(), ov))).unwrap();
        let parsed: Vec<_> = parse_meta(CANONICAL)
            .into_iter()
            .map(|l| (l.path, l.ov))
            .collect();
        assert_eq!(seen, parsed);
    }

    #[test]
    fn records_round_trip() {
        for l in parse_meta(CANONICAL) {
            let rec = encode_record(77, &l.ov);
            assert_eq!(decode_record(&rec), (77, l.ov));
        }
        // An earlier build's hidden GIF (bit 1, nothing else): no override.
        let mut rec = encode_record(5, &Override::default());
        rec[14] = 1;
        assert!(decode_record(&rec).1.is_empty());
    }

    #[test]
    fn fingerprint_sees_every_byte_and_never_collides_with_no_meta() {
        let a = fingerprint(CANONICAL.as_bytes()).unwrap();
        let b = fingerprint(CANONICAL.replace("w=3", "w=4").as_bytes()).unwrap();
        let c = fingerprint(&b""[..]).unwrap();
        assert_ne!(a, b);
        assert_ne!(c, NO_META);
        assert_ne!(a, c);
        assert_eq!(a, fingerprint(CANONICAL.as_bytes()).unwrap());
    }

    #[test]
    fn override_set_is_case_insensitive_capped_and_last_wins() {
        let mut s = OverrideSet::new();
        let fav = Override {
            weight: Some(3.0),
            ..Default::default()
        };
        let less = Override {
            weight: Some(0.5),
            ..Default::default()
        };
        s.insert("/gifs/A/X.gif", fav);
        s.insert("/GIFS/a/x.GIF", less);
        assert_eq!(s.len(), 1);
        assert_eq!(s.get("/gifs/A/X.gif"), Some(less));
        assert_eq!(s.get("/gifs/A/Y.gif"), None);
        for i in 0..MAX_OVERRIDES + 5 {
            s.insert(&format!("/gifs/B/{i}.gif"), fav);
        }
        assert_eq!(s.len(), MAX_OVERRIDES);
        assert_eq!(s.dropped, 6);
    }

    #[test]
    fn overrides_stay_sorted_and_capped() {
        let ov = Override {
            weight: Some(0.0),
            ..Default::default()
        };
        let mut o = Overrides::new();
        assert!(o.push(3, ov));
        assert!(!o.push(3, ov));
        assert!(!o.push(1, ov));
        assert!(o.push(9, ov));
        assert_eq!(o.get(9), Some(&ov));
        assert_eq!(o.get(4), None);
        assert_eq!(o.summary(), (2, 0));
    }

    #[test]
    fn meta_paths_pair_with_their_manifests() {
        for (manifest, meta) in [
            ("/filelist.txt", "/filelist.meta"),
            ("/filelist.hall", "/filelist.hall.meta"),
            ("/sets/consoles.txt", "/sets/consoles.meta"),
            ("/sets/v1.2.txt", "/sets/v1.2.meta"),
            ("/sdcard/sets/odmb.txt", "/sdcard/sets/odmb.meta"),
            ("/sdcard/filelist.txt", "/sdcard/filelist.meta"),
        ] {
            assert_eq!(meta_path(manifest), meta);
            assert_eq!(manifest_for_meta(meta).as_deref(), Some(manifest));
        }
        assert_eq!(manifest_for_meta("/gifs/x.gif"), None);
    }
}

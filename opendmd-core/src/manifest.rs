//! `filelist.txt` parsing and the filename tag grammar.
//!
//! # Card format
//!
//! `filelist.txt` at the card root is one path per line, LF-separated, each
//! path absolute-from-the-card-root with a leading slash:
//!
//! ```text
//! /gifs/Consoles/SNES_Plok_08.gif
//! ```
//!
//! Order is insertion order, not sorted; the stock firmware walks it
//! sequentially. `filelist.hall` and `filelist.xmas` are seasonal subsets in
//! the same format. Parsing is deliberately tolerant (BOM, CRLF, blank lines,
//! trailing whitespace); [`render_manifest`] always emits the canonical form:
//! one path per line, LF-terminated (as the real card file is), paths
//! byte-identical to input.
//!
//! # Tag grammar
//!
//! Filenames are `PLATFORM_Title_Words_NN[_FLAG].gif`. It is a convention, not
//! a spec — the rules below were derived from the real 10k-file card and are
//! best-effort. Precedence, applied to the `_`-separated tokens of the stem:
//!
//! 1. **Season prefix.** A leading `HALLOWEEN` / `XMAS` token is recorded as a
//!    flag and consumed *only if a known platform prefix follows it*
//!    (`HALLOWEEN_ARCADE_GoldenAxe16.gif` -> platform `ARCADE`, flag
//!    `HALLOWEEN`). Otherwise it is treated as the platform itself. The
//!    directory category is the authoritative source of seasonality anyway —
//!    the flag is a convenience for files that get copied out of `Halloween/`.
//! 2. **Platform prefix.** Multi-token prefixes first ([`MULTI_PLATFORMS`],
//!    which is also where `NINTENDO_DS` is normalised to `NDS`), then the
//!    single-token list ([`PLATFORMS`]), matched exactly (upper case). Nothing else is a
//!    platform: an unlisted first token (`THE_80s_90s_…`, `HDB_Happy_…`,
//!    `Nes_Gradius`) leaves `platform: None` and the whole stem becomes the
//!    title. The list is closed on purpose: every all-caps first word used to
//!    count, which turned `THE`, `HDB`, `LOGO` and `POKEMON` into platforms
//!    on the real card; a file name carries no other way to tell a platform
//!    from a word, and the parser sees one name at a time (the firmware
//!    filters a 10k-line playlist line by line), so it cannot count either.
//! 3. **Leading keyword flags.** `LOGO`, `INTRO`, `INVERT` etc. directly after
//!    the platform (`MD_LOGO_Sega31.gif`).
//! 4. **Trailing tags**, consumed right-to-left: keyword flags, a standalone
//!    all-digit token as the variant, and author handles.
//! 5. **Glued variant.** If no standalone variant was found, trailing digits on
//!    the last token count as the variant *only when there are two or more of
//!    them* — `SNES_Asterix01` is Asterix variant 1, but `MD_Sonic3_...` keeps
//!    `Sonic3` whole, because a single glued digit is almost always a sequel
//!    number.
//!
//! Author handles cannot be told from title words in general (`StarWars01`,
//! `SuperMario` and `RattenJager` are all trailing CamelCase), so a generic
//! CamelCase rule mangles titles. Instead a token is an author if it starts
//! with a lowercase letter (`sephirot68`, `clivefrog`, `zzoomm`) or is in the
//! small [`KNOWN_AUTHORS`] list, and only when consuming it leaves a non-empty
//! title. Flags are stored verbatim; all flag comparisons are ASCII
//! case-insensitive.

use crate::filter::Filter;
use std::io::{self, BufRead, Read, Seek, SeekFrom};

/// The single-token platform prefixes: the only first words read as a
/// platform (with [`MULTI_PLATFORMS`]). What the DMDoS card uses, the tags
/// Follow RePlayOS maps systems to, `FX` (Create Art's recordings of the
/// effects), and a few common systems the card lacks. Adding one is safe
/// (files with that prefix gain it); removing one takes it off every file.
#[rustfmt::skip]
pub const PLATFORMS: &[&str] = &[
    // Nintendo
    "NES", "SNES", "N64", "GAMECUBE", "WII", "SWITCH", "GB", "GBC", "GBA", "NDS", "3DS",
    // Sega
    "SG1000", "SMS", "MD", "32X", "SATURN", "DREAMCAST", "GAMEGEAR",
    // Sony, Microsoft
    "PS", "PS1", "PSX", "PS2", "PS3", "PS4", "PSP", "VITA", "XBOX",
    // Other consoles and handhelds
    "PCE", "NGP", "NGPC", "NEOGEO", "LYNX", "JAGUAR", "3DO", "WONDERSWAN", "COLECOVISION",
    "INTELLIVISION", "VECTREX",
    // Computers
    "AMIGA", "C64", "ATARI", "ATARI800", "ATARIST", "AMSTRAD", "SPECTRUM", "ZX", "MSX", "MSX2",
    "X68000", "PC", "DOS", "MAC",
    // Phones, fantasy consoles, our own
    "IOS", "MOBILE", "PICO-8", "FX",
    // DMDoS's catch-alls
    "ARCADE", "PINBALL", "OTHER", "MOVIES", "HALLOWEEN", "XMAS",
];

/// Multi-token platform prefixes, as `(tokens, canonical name)`. Matched before
/// [`PLATFORMS`], longest first.
pub const MULTI_PLATFORMS: &[(&[&str], &str)] = &[
    (&["NINTENDO", "DS"], "NDS"),
    (&["VIRTUAL", "BOY"], "VIRTUAL_BOY"),
    (&["SEGA", "MEGACD"], "SEGA_MEGACD"),
    (&["SEGA", "PICO"], "SEGA_PICO"),
];

/// Season tokens that become a flag when a platform prefix follows them.
pub const SEASONS: &[&str] = &["HALLOWEEN", "XMAS"];

/// Tokens that are flags wherever they appear at either end of the tag run.
pub const KEYWORD_FLAGS: &[&str] = &[
    "INVERT", "INTRO", "LOGO", "SLIDE", "STORY", "ANIM", "STATIC",
];

/// Prolific GIF authors whose handles are capitalised and would otherwise be
/// mistaken for title words.
pub const KNOWN_AUTHORS: &[&str] = &["RattenJager", "OMEGAeFex"];

/// One line of a `filelist.*` file, with its filename tags parsed out.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Entry {
    /// The path exactly as it appeared in the manifest, leading slash intact.
    pub path: String,
    /// Directory under `gifs/` (or, for a prepared copy, under `cache/`, the
    /// hash bucket skipped), e.g. `Consoles` or `NEW 2021`.
    pub category: Option<String>,
    pub platform: Option<String>,
    pub title: String,
    pub variant: Option<u32>,
    /// Season markers, `INVERT`-style keywords and author handles, verbatim.
    pub flags: Vec<String>,
}

impl Entry {
    /// Parse a manifest path into an entry. Never fails: worst case the title
    /// is the whole filename stem.
    pub fn from_path(path: &str) -> Entry {
        // A virtual file (`@clock`, `@text?msg=…`): what it shows is its
        // title; no category, no platform.
        if crate::synth::is_virtual(path) {
            return Entry {
                path: path.to_string(),
                category: None,
                platform: None,
                title: crate::synth::title(path).unwrap_or_else(|| path.to_string()),
                variant: None,
                flags: Vec::new(),
            };
        }
        let category = category_of(path);
        let stem = stem_of(path);
        let mut entry = parse_stem(stem);
        entry.path = path.to_string();
        entry.category = category;
        entry
    }

    /// ASCII case-insensitive flag test.
    pub fn has_flag(&self, flag: &str) -> bool {
        self.flags.iter().any(|f| f.eq_ignore_ascii_case(flag))
    }

    /// Title reduced to lowercase alphanumerics, for fuzzy matching.
    pub fn normalized_title(&self) -> String {
        normalize_title(&self.title)
    }
}

/// Lowercase, alphanumerics only: `"Donkey Kong Country"` -> `"donkeykongcountry"`.
pub fn normalize_title(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// Parse a whole `filelist.*` file. Tolerates a UTF-8 BOM, CRLF, blank lines
/// and trailing whitespace; preserves order and duplicates.
pub fn parse_manifest(text: &str) -> Vec<Entry> {
    text.strip_prefix('\u{feff}')
        .unwrap_or(text)
        .split('\n')
        .map(|line| line.trim_end_matches(['\r', ' ', '\t']))
        .filter(|line| !line.is_empty() && !retired(line))
        .map(Entry::from_path)
        .collect()
}

/// Is `line` a built-in effect's (`@fx:tunnel?…`, `@fx:psi?…`), from before
/// they were Formulas? It names nothing now: [`scan`] and
/// [`parse_manifest`] skip it with one log line each time they read it, so a
/// stale playlist plays the rest (`@formula?src=…` is a Formula's line).
fn retired(line: &str) -> bool {
    if !crate::synth::is_retired_effect(line) {
        return false;
    }
    let cut = line.char_indices().nth(48).map_or(line, |(i, _)| &line[..i]);
    log::warn!(
        "playlist: skipping {cut:?}{}: the built-in effects are gone (Formulas replace them)",
        if cut.len() < line.len() { "…" } else { "" }
    );
    true
}

/// Render entries back to card format: one path per line, each terminated
/// by LF, matching the real card's `filelist.txt` byte for byte. An empty
/// entry list renders as an empty string.
pub fn render_manifest(entries: &[Entry]) -> String {
    let mut out = String::with_capacity(entries.len() * 48);
    for e in entries {
        out.push_str(&e.path);
        out.push('\n');
    }
    out
}

/// Longest manifest line we will read back through [`entry_at`].
///
/// The real card's longest path is ~90 bytes; FAT long filenames cap a single
/// component at 255. 512 covers both with room to spare, and keeping it a
/// stack array is what makes `entry_at` allocation-free on the firmware.
const MAX_LINE: usize = 512;
/// Longest line read at all: a Formula's path (`@formula?src=…`, up to
/// [`MAX_FORMULA_PATH`](crate::synth::MAX_FORMULA_PATH)) with some trailing
/// whitespace. Only a line past [`MAX_LINE`] goes to the heap for the rest.
const MAX_LONG_LINE: usize = crate::synth::MAX_FORMULA_PATH + 64;

/// Strip a leading BOM and any trailing CR / space / tab from one raw line.
///
/// The BOM is only ever on the first line of a file; stripping it here costs
/// nothing and keeps [`entry_at`] correct when offset 0 is a matching entry.
fn trim_line(raw: &str) -> &str {
    raw.strip_prefix('\u{feff}')
        .unwrap_or(raw)
        .trim_end_matches(['\n', '\r', ' ', '\t'])
}

/// Stream a `filelist.*` file, calling `visit(offset, entry)` for every
/// non-blank line, where `offset` is the **byte offset of the start of that
/// line** in the reader.
///
/// This is the memory-bounded counterpart of [`parse_manifest`]: exactly one
/// line and one [`Entry`] are alive at a time, and the entry is dropped as
/// soon as `visit` returns. Tolerance matches `parse_manifest` exactly — a
/// leading BOM (skipped, but offsets stay absolute file offsets), CRLF, blank
/// lines and trailing whitespace — and so does skipping a retired effect's
/// line (`@fx:…`, logged).
pub fn scan<R: BufRead>(mut reader: R, mut visit: impl FnMut(u32, &Entry)) -> io::Result<()> {
    let mut raw = String::new();
    let mut offset: u64 = 0;
    loop {
        raw.clear();
        let read = reader.read_line(&mut raw)?;
        if read == 0 {
            return Ok(());
        }
        let start = offset;
        offset += read as u64;
        let line = trim_line(&raw);
        if line.is_empty() || retired(line) {
            continue;
        }
        visit(start as u32, &Entry::from_path(line));
    }
}

/// One streaming pass returning the byte offset of the start of every line
/// whose entry passes `filter`, in manifest order.
///
/// The result is the whole memory cost of a playlist: 4 bytes per matching
/// entry (~40 KB for the real 10,133-entry card, unfiltered) instead of the
/// ~590 bytes per entry a `Vec<Entry>` costs. Pair it with [`entry_at`], or
/// use [`FileCatalog`](crate::engine::FileCatalog), which owns both.
pub fn index_matching<R: BufRead>(reader: R, filter: &Filter) -> io::Result<Vec<u32>> {
    let mut offsets = Vec::new();
    scan(reader, |offset, entry| {
        if filter.matches(entry) {
            offsets.push(offset);
        }
    })?;
    Ok(offsets)
}

/// Read back the single entry whose line starts at `offset`, as produced by
/// [`index_matching`].
///
/// Seeks, reads at most one line into a stack buffer and parses it; no
/// allocation beyond the returned [`Entry`]'s own strings — but for a line
/// longer than [`MAX_LINE`] (a Formula's path), whose rest is read onto the
/// heap, up to [`MAX_LONG_LINE`] in all.
pub fn entry_at<R: Read + Seek>(reader: &mut R, offset: u32) -> io::Result<Entry> {
    reader.seek(SeekFrom::Start(offset as u64))?;

    let mut buf = [0u8; MAX_LINE];
    let mut filled = 0usize;
    while filled < buf.len() {
        match reader.read(&mut buf[filled..])? {
            0 => break,
            n => filled += n,
        }
        if buf[..filled].contains(&b'\n') {
            break;
        }
    }

    let long;
    let bytes = match buf[..filled].iter().position(|b| *b == b'\n') {
        Some(end) => &buf[..end],
        None if filled < MAX_LINE => &buf[..filled],
        None => {
            let mut v = Vec::with_capacity(MAX_LONG_LINE);
            v.extend_from_slice(&buf);
            reader
                .by_ref()
                .take((MAX_LONG_LINE - MAX_LINE) as u64)
                .read_to_end(&mut v)?;
            v.truncate(v.iter().position(|b| *b == b'\n').unwrap_or(v.len()));
            long = v;
            &long[..]
        }
    };
    let raw =
        std::str::from_utf8(bytes).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let line = trim_line(raw);
    if line.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("no manifest line at offset {offset}"),
        ));
    }
    Ok(Entry::from_path(line))
}

/// The category is the first directory under `gifs/` — or under `cache/`,
/// the prepared copies (`/cache/<Category>/<xx>/<name>.gif`, see
/// `docs/cache-format.md`), where the directory holding the file is a
/// two-hex-digit hash bucket and never the category. So
/// `/cache/Consoles/38/X.gif` is `Consoles`, like `/gifs/Consoles/X.gif`,
/// and a loose `/cache/63/X.gif` (from a GIF directly in `gifs/`) has none.
fn category_of(path: &str) -> Option<String> {
    let mut parts = path.split('/').filter(|p| !p.is_empty());
    let first = parts.next()?;
    let second = parts.next()?;
    // Only a directory counts as a category; `/gifs/foo.gif` has none.
    parts.next()?;
    if first.eq_ignore_ascii_case("gifs") {
        return Some(second.to_string());
    }
    if first.eq_ignore_ascii_case("cache") {
        // `/cache/<xx>/<name>`: `second` is the bucket of an uncategorised file.
        let loose = parts.next().is_none() && is_bucket(second);
        return (!loose).then(|| second.to_string());
    }
    None
}

/// Exactly two lowercase hex digits: a bucket directory in the `cache/` tree.
fn is_bucket(dir: &str) -> bool {
    dir.len() == 2
        && dir
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Filename minus directories, minus any trailing `.gif` extensions and the
/// stray dots around them (`..._01..gif` is real).
fn stem_of(path: &str) -> &str {
    let name = path.rsplit('/').next().unwrap_or(path);
    let mut stem = name;
    loop {
        let trimmed = stem.trim_end_matches('.');
        // `get`, not indexing: a name ending in a multi-byte character must
        // not panic on a non-boundary slice.
        let cut = trimmed.len().saturating_sub(4);
        let next = match trimmed.get(cut..) {
            Some(ext) if ext.eq_ignore_ascii_case(".gif") => &trimmed[..cut],
            _ => trimmed,
        };
        if next.len() == stem.len() {
            return next;
        }
        stem = next;
    }
}

fn is_keyword_flag(token: &str) -> bool {
    KEYWORD_FLAGS.iter().any(|k| token.eq_ignore_ascii_case(k))
}

/// A listed platform, exactly as listed: the parser is case-sensitive
/// (`Nes_Gradius` has no platform).
fn is_known_platform(token: &str) -> bool {
    PLATFORMS.contains(&token)
}

/// The listed platform a builder argument names, in any case (`snes` ->
/// `SNES`): what a user picks or types is written as listed.
#[cfg(feature = "names")]
fn known_platform(token: &str) -> Option<&'static str> {
    PLATFORMS
        .iter()
        .copied()
        .find(|p| p.eq_ignore_ascii_case(token))
}

/// Author handle: a known handle, or any token starting with a lowercase
/// ASCII letter (title words that start lowercase, like `of`, are short and
/// never last).
fn is_author(token: &str) -> bool {
    if KNOWN_AUTHORS.iter().any(|a| token.eq_ignore_ascii_case(a)) {
        return true;
    }
    token.len() >= 3 && token.starts_with(|c: char| c.is_ascii_lowercase())
}

fn parse_stem(stem: &str) -> Entry {
    let tokens: Vec<&str> = stem.split('_').filter(|t| !t.is_empty()).collect();
    let mut flags: Vec<String> = Vec::new();
    let mut variant: Option<u32> = None;

    if tokens.is_empty() {
        return Entry {
            title: stem.to_string(),
            ..Default::default()
        };
    }

    let mut rest: &[&str] = &tokens;

    // 1. Season prefix, but only in front of a real platform prefix.
    if rest.len() >= 2 && SEASONS.contains(&rest[0]) {
        let tail = &rest[1..];
        let followed_by_platform = is_known_platform(tail[0])
            || MULTI_PLATFORMS
                .iter()
                .any(|(toks, _)| tail.len() > toks.len() && tail.starts_with(toks));
        if followed_by_platform {
            flags.push(rest[0].to_string());
            rest = tail;
        }
    }

    // 2. Platform prefix. Multi-token first; never consume every token.
    let mut platform: Option<String> = None;
    for (toks, name) in MULTI_PLATFORMS {
        if rest.len() > toks.len() && rest.starts_with(toks) {
            platform = Some((*name).to_string());
            rest = &rest[toks.len()..];
            break;
        }
    }
    if platform.is_none() && rest.len() > 1 && is_known_platform(rest[0]) {
        platform = Some(rest[0].to_string());
        rest = &rest[1..];
    }

    // The whole stem is the title when there was no usable prefix.
    if platform.is_none() && flags.is_empty() {
        return Entry {
            title: tokens.join(" "),
            ..Default::default()
        };
    }

    // 3. Leading keyword flags.
    while rest.len() > 1 && is_keyword_flag(rest[0]) {
        flags.push(rest[0].to_string());
        rest = &rest[1..];
    }

    // 4. Trailing tags, right to left.
    while let Some(last) = rest.last().copied() {
        if rest.len() < 2 {
            break;
        }
        if is_keyword_flag(last) {
            flags.push(last.to_string());
        } else if variant.is_none() && last.chars().all(|c| c.is_ascii_digit()) {
            variant = last.parse().ok();
        } else if variant.is_none() && is_author(last) {
            flags.push(last.to_string());
        } else {
            break;
        }
        rest = &rest[..rest.len() - 1];
    }

    // 5. Glued variant: two or more trailing digits on the final token.
    let mut words: Vec<String> = rest.iter().map(|t| t.to_string()).collect();
    if variant.is_none() {
        if let Some(last) = words.last_mut() {
            let digits = last.len() - last.trim_end_matches(|c: char| c.is_ascii_digit()).len();
            let head_len = last.len() - digits;
            if digits >= 2 && head_len > 0 {
                if let Ok(v) = last[head_len..].parse::<u32>() {
                    variant = Some(v);
                    last.truncate(head_len);
                }
            }
        }
    }

    let mut title = words.join(" ").trim().to_string();
    if title.is_empty() {
        title = tokens.join(" ");
    }

    Entry {
        path: String::new(),
        category: None,
        platform,
        title,
        variant,
        flags,
    }
}

// --- Building a name from fields ------------------------------------------
//
// The inverse of the parser: what the web app's "Add GIFs" and "Edit
// details" write. The metadata of a GIF *is* its path — there is no other
// store — so a rename is how a title, platform, category or author changes.

/// Longest file name [`build_name`] writes, `.gif` included. The card's
/// longest is ~70 bytes; FAT's limit is 255 UTF-16 units per component, and
/// the prepared copy's path (`/cache/<Category>/<xx>/<name>`) and every
/// playlist line must stay well inside [`MAX_LINE`].
#[cfg(feature = "names")]
pub const MAX_NAME_LEN: usize = 120;
/// Longest category (directory under `gifs/`) [`build_name`] accepts.
#[cfg(feature = "names")]
pub const MAX_CATEGORY_LEN: usize = 48;

/// Why fields cannot be written as a name that reads back as the same
/// fields. [`NameError::code`] is a stable string for the web app.
#[cfg(feature = "names")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NameError {
    /// No title (or nothing but separators).
    EmptyTitle,
    /// A character FAT or the grammar cannot take: `"*/:<>?\|`, `_`
    /// (the separator; spaces become `_`), control or non-ASCII.
    TitleChar,
    /// Not a platform the parser would read back: one of [`PLATFORMS`] or a
    /// multi-token name (`VIRTUAL_BOY`).
    Platform,
    /// Empty, too long, or a character a directory name cannot have.
    Category,
    /// An author handle is one token of `A-Za-z0-9.-`, at least three long,
    /// not a keyword (`INTRO`) nor all digits.
    Author,
    /// A number or an author needs a platform: without one the parser reads
    /// the whole name as the title.
    NeedsPlatform,
    /// Longer than [`MAX_NAME_LEN`].
    TooLong,
    /// Built, but the parser would read it back differently (a title word
    /// taken for a tag: `Toy Story` -> `Toy` + `Story`, or `SNES Classics`
    /// with no platform -> platform `SNES`). Reword.
    ReadsBack,
}

#[cfg(feature = "names")]
impl NameError {
    pub fn code(self) -> &'static str {
        match self {
            NameError::EmptyTitle => "title",
            NameError::TitleChar => "title-char",
            NameError::Platform => "platform",
            NameError::Category => "category",
            NameError::Author => "author",
            NameError::NeedsPlatform => "needs-platform",
            NameError::TooLong => "too-long",
            NameError::ReadsBack => "reads-back",
        }
    }
}

/// A GIF's metadata, as its path carries it: the fields [`build_name`]
/// takes, plus any other flags (`INVERT`, `LOGO`, a season) the path had,
/// which a rename keeps.
#[cfg(feature = "names")]
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct NameFields {
    pub title: String,
    pub platform: Option<String>,
    /// The directory under `gifs/` (empty for a GIF directly in `gifs/`).
    pub category: String,
    pub ordinal: Option<u32>,
    pub author: Option<String>,
    /// Keyword and season flags, verbatim.
    pub flags: Vec<String>,
}

#[cfg(feature = "names")]
impl NameFields {
    /// The fields of a card path (the parser's view, with the author
    /// split out of the flags and the title's whitespace collapsed). The inverse of [`NameFields::path`].
    pub fn from_path(path: &str) -> NameFields {
        // `Entry::from_path` without its virtual-file branch: a name is
        // never one, and the art module that carries this must not pull in
        // the live items' titles.
        let mut e = parse_stem(stem_of(path));
        e.category = category_of(path);
        let mut author = None;
        let mut flags = Vec::new();
        for f in e.flags {
            let other = is_keyword_flag(&f) || SEASONS.contains(&f.as_str());
            if author.is_none() && !other {
                author = Some(f.trim().to_string());
            } else {
                flags.push(f);
            }
        }
        NameFields {
            // As it would be written: runs of whitespace are one separator.
            title: e.title.split_whitespace().collect::<Vec<_>>().join(" "),
            platform: e.platform,
            category: e.category.unwrap_or_default(),
            ordinal: e.variant,
            author,
            flags,
        }
    }

    /// The card path these fields are written as: `/gifs/<Category>/` +
    /// [`NameFields::file_name`].
    pub fn path(&self) -> Result<String, NameError> {
        let name = self.file_name()?;
        Ok(if self.category.is_empty() {
            format!("/gifs/{name}")
        } else {
            format!("/gifs/{}/{name}", self.category)
        })
    }

    /// `[SEASON_]PLATFORM_Title_Words_NN[_author][_FLAG…].gif`, checked by
    /// reading it back with [`Entry::from_path`]: `Ok` only if the parser
    /// gives exactly these fields (the title with its whitespace collapsed,
    /// an author's first letter lowercased unless it is a known handle).
    pub fn file_name(&self) -> Result<String, NameError> {
        let words: Vec<&str> = self.title.split_whitespace().collect();
        if words.is_empty() {
            return Err(NameError::EmptyTitle);
        }
        if !words.iter().all(|w| w.chars().all(name_char)) || self.title.starts_with('.') {
            return Err(NameError::TitleChar);
        }
        let cat = self.category.as_str();
        if cat.len() > MAX_CATEGORY_LEN
            || cat.trim() != cat
            || cat.starts_with('.')
            || !cat.chars().all(|c| c == '_' || c == ' ' || name_char(c))
        {
            return Err(NameError::Category);
        }
        let author = match self
            .author
            .as_deref()
            .map(str::trim)
            .filter(|a| !a.is_empty())
        {
            None => None,
            Some(a) => Some(author_handle(a).ok_or(NameError::Author)?),
        };
        let mut parts: Vec<&str> = Vec::new();
        let season = self.flags.iter().find(|f| SEASONS.contains(&f.as_str()));
        // A listed platform in any case is written as listed (`snes` -> `SNES`).
        let platform = match self
            .platform
            .as_deref()
            .map(str::trim)
            .filter(|p| !p.is_empty())
        {
            None => None,
            Some(p) => Some(
                known_platform(p)
                    .or_else(|| {
                        MULTI_PLATFORMS
                            .iter()
                            .map(|(_, n)| *n)
                            .find(|n| n.eq_ignore_ascii_case(p))
                    })
                    .ok_or(NameError::Platform)?,
            ),
        };
        match platform {
            Some(p) => {
                if let Some(s) = season {
                    parts.push(s);
                }
                parts.push(p);
            }
            None if self.ordinal.is_some() || author.is_some() || !self.flags.is_empty() => {
                return Err(NameError::NeedsPlatform)
            }
            None => {}
        }
        parts.extend(&words);
        let ordinal = self.ordinal.map(|n| format!("{n:02}"));
        parts.extend(ordinal.as_deref());
        parts.extend(author.as_deref());
        // Trailing tags are read right to left: written reversed, they read
        // back in this order.
        parts.extend(
            self.flags
                .iter()
                .rev()
                .filter(|f| !SEASONS.contains(&f.as_str()))
                .map(String::as_str),
        );
        let name = parts.join("_") + ".gif";
        if name.len() > MAX_NAME_LEN {
            return Err(NameError::TooLong);
        }
        // The one rule that matters: it reads back as what was asked for.
        let back = NameFields::from_path(&format!("/gifs/x/{name}"));
        let same_flags = back.flags.len() == self.flags.len()
            && self
                .flags
                .iter()
                .all(|f| back.flags.iter().any(|b| b.eq_ignore_ascii_case(f)));
        if back.title != words.join(" ")
            || back.platform.as_deref() != platform
            || back.ordinal != self.ordinal
            || back.author != author
            || !same_flags
        {
            return Err(NameError::ReadsBack);
        }
        Ok(name)
    }
}

/// The card path for a GIF with these fields (see [`NameFields::file_name`]
/// for the form and the checks). `category` is the directory under `gifs/`.
#[cfg(feature = "names")]
pub fn build_name(
    title: &str,
    platform: Option<&str>,
    category: &str,
    ordinal: Option<u32>,
    author: Option<&str>,
) -> Result<String, NameError> {
    NameFields {
        title: title.to_string(),
        platform: platform.map(str::to_string),
        category: category.to_string(),
        ordinal,
        author: author.map(str::to_string),
        flags: Vec::new(),
    }
    .path()
}

/// A character a title word (or a category) may have: printable ASCII
/// except FAT's `"*/:<>?\|`, and not `_`, the grammar's separator.
#[cfg(feature = "names")]
fn name_char(c: char) -> bool {
    c.is_ascii_graphic() && !"\"*/:<>?\\|_".contains(c)
}

/// An author as the grammar reads one back: a known handle (any case) as
/// it is, else
/// one `A-Za-z0-9.-` token of three or more with its first letter
/// lowercased (`Ross` -> `ross`); not a keyword flag, not all digits.
#[cfg(feature = "names")]
fn author_handle(a: &str) -> Option<String> {
    if KNOWN_AUTHORS.iter().any(|k| k.eq_ignore_ascii_case(a)) {
        return Some(a.to_string());
    }
    let ok = a.len() >= 3
        && a.starts_with(|c: char| c.is_ascii_alphabetic())
        && a.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.')
        && !is_keyword_flag(a);
    ok.then(|| {
        let mut s = a.to_string();
        s[..1].make_ascii_lowercase();
        s
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn p(path: &str) -> Entry {
        Entry::from_path(path)
    }

    fn check(
        name: &str,
        platform: Option<&str>,
        title: &str,
        variant: Option<u32>,
        flags: &[&str],
    ) {
        let e = p(&format!("/gifs/Consoles/{name}"));
        assert_eq!(e.platform.as_deref(), platform, "platform of {name}");
        assert_eq!(e.title, title, "title of {name}");
        assert_eq!(e.variant, variant, "variant of {name}");
        let got: Vec<&str> = e.flags.iter().map(|s| s.as_str()).collect();
        assert_eq!(got, flags, "flags of {name}");
    }

    /// Real names from the card (`~/scratch/dmdos-card/filelist.txt`),
    /// picked for their variety: glued and standalone numbers, authors
    /// known and lowercase, keyword flags leading and trailing, seasons,
    /// multi-token platforms, no platform, a space inside a token, a
    /// trailing `..gif`.
    #[cfg(feature = "names")]
    pub(crate) const CORPUS: &[&str] = &[
        "/gifs/Consoles/SNES_Donkey_Kong_Country_05.gif",
        "/gifs/Consoles/MD_Sonic3_RattenJager.gif",
        "/gifs/Consoles/SNES_Asterix01.gif",
        "/gifs/Consoles/SNES_Mother02_RattenJager.gif",
        "/gifs/NEW 2021/NINTENDO_DS_Mario_Kart_DS_01..gif",
        "/gifs/Consoles/NES_Unknow_01_Invert.gif",
        "/gifs/Halloween/HALLOWEEN_ARCADE_GoldenAxe16.gif",
        "/gifs/Halloween/HALLOWEEN_Dracula_01.gif",
        "/gifs/Consoles/SMS_Aladdin_Intro.gif",
        "/gifs/Consoles/PICO-8_Downstream_Dream_02.gif",
        "/gifs/Consoles/MD_LOGO_Sega31.gif",
        "/gifs/Consoles/NES_The_Ren_n_Stimpy_Show.gif",
        "/gifs/Consoles/PSP_Parasite_Eve_01__sephirot68.gif",
        "/gifs/Consoles/Nes_Gradius.gif",
        "/gifs/Spectrum_Slide/Exolon2.gif",
        "/gifs/Pinball_Short/PINBALL_SHORT_STTNG30.gif",
        "/gifs/The_80s_90s/The_80s_90s_MacGyver_02_sephirot68.gif",
        "/gifs/Consoles/GBA_CT_Special_Forces_3_Bioterror_09.gif",
        "/gifs/Other/OTHER_NINTENDO_SuperMarioBads03.gif",
        "/gifs/Pinball_Short/PINBALL_SHORT_JurassicPark_NEW_02.gif",
        "/gifs/Arcade/ARCADE_Green_Beret_02_RattenJager.gif",
        "/gifs/Consoles/NES_King_Kong_2_01.gif",
        "/gifs/Consoles/MD_RoiLion06_raph.gif",
        "/gifs/Consoles/SNES_MegaManHelmet02Title.gif",
        "/gifs/Arcade/ARCADE_SunsetRiders11Intro02.gif",
        "/gifs/Consoles/SNES_The_Great_Battle_IV.gif",
        "/gifs/Arcade/ARCADE_NEOGEO_Metal_Slug_X_02_INVERT.gif",
        "/gifs/Consoles/GAMEGEAR_Columns_01_bd.azerty.p.gif",
        "/gifs/Other/OTHER_037.gif",
        "/gifs/Movies/MOVIES_HORROR_Leatherface_pandemo.gif",
        "/gifs/Pinball_Short/PINBALL_SHORT_ID4_33.gif",
        "/gifs/Computers/PC_Leilanis Island_42.gif",
        "/gifs/Consoles/NGPC_CardFightersClashII05_RattenJager.gif",
        "/gifs/Arcade/ARCADE_LOGO_NEOGEO_Snk01.gif",
        "/gifs/Consoles/MD_Chase_HQ_II_01_Clivefrog.gif",
        "/gifs/Other/OTHER_SCROLL_Xmen01.gif",
        "/gifs/Consoles/MD_Demons_Of_Asteborg_01_sephirot68.gif",
        "/gifs/Consoles/SNES_Street_Fighter_II_Turbo_03.gif",
        "/gifs/Arcade/ARCADE_Pac-Man_01.gif",
        "/gifs/Consoles/VIRTUAL_BOY_Mario_Clash_01.gif",
        "/gifs/Consoles/SEGA_MEGACD_Snatcher_02.gif",
        "/gifs/Xmas/XMAS_SNES_Christmas_Tree_01.gif",
    ];

    #[cfg(feature = "names")]
    #[test]
    fn built_names_read_back_as_the_same_fields() {
        let mut built = 0;
        for &path in CORPUS {
            let f = NameFields::from_path(path);
            match f.path() {
                Ok(p) => {
                    built += 1;
                    assert_eq!(NameFields::from_path(&p), f, "{path} -> {p}");
                    // Building again from what it reads back is a fixed point.
                    assert_eq!(NameFields::from_path(&p).path().as_deref(), Ok(p.as_str()));
                }
                Err(e) => panic!("{path}: {e:?} for {f:?}"),
            }
        }
        assert_eq!(built, CORPUS.len());
    }

    #[cfg(feature = "names")]
    #[test]
    fn every_listed_platform_round_trips() {
        let multi = MULTI_PLATFORMS.iter().map(|(_, n)| *n);
        for p in PLATFORMS.iter().copied().chain(multi) {
            // The builder takes the argument in any case, writes it as listed.
            for given in [p.to_string(), p.to_ascii_lowercase()] {
                let path = build_name("Super Game", Some(&given), "C", Some(3), Some("ross"))
                    .unwrap_or_else(|e| panic!("{p}: {e:?}"));
                assert_eq!(path, format!("/gifs/C/{p}_Super_Game_03_ross.gif"));
                let e = Entry::from_path(&path);
                assert_eq!(e.platform.as_deref(), Some(p), "{path}");
                assert_eq!(e.title, "Super Game", "{path}");
                assert_eq!(e.variant, Some(3), "{path}");
            }
            // A name in another case is no platform, so the title keeps it.
            if !p.contains('_') && p.chars().any(|c| c.is_ascii_uppercase()) {
                let low = p.to_ascii_lowercase();
                let e = Entry::from_path(&format!("/gifs/C/{low}_Super_Game_03.gif"));
                assert_eq!(e.platform, None, "{low}");
            }
        }
        // A title starting with a platform's name in another case is fine.
        assert_eq!(
            build_name("Pinball Wizard", None, "C", None, None).as_deref(),
            Ok("/gifs/C/Pinball_Wizard.gif")
        );
    }

    #[cfg(feature = "names")]
    #[test]
    fn canonical_names_are_rebuilt_byte_for_byte() {
        for path in [
            "/gifs/Consoles/SNES_Donkey_Kong_Country_05.gif",
            "/gifs/Consoles/MD_Sonic3_RattenJager.gif",
            "/gifs/Consoles/PICO-8_Downstream_Dream_02.gif",
            "/gifs/Consoles/SNES_Mega_Man_X_08_sephirot68.gif",
            "/gifs/Arcade/ARCADE_NEOGEO_Metal_Slug_X_02_INVERT.gif",
            "/gifs/Spectrum_Slide/Exolon2.gif",
            "/gifs/NEW 2021/NES_King_Kong_2_01.gif",
        ] {
            assert_eq!(NameFields::from_path(path).path().as_deref(), Ok(path));
        }
        // Not canonical, same fields: the number padded and last, the space
        // an underscore, the stray dot gone, NINTENDO_DS written NDS.
        for (path, want) in [
            (
                "/gifs/Consoles/SNES_Asterix01.gif",
                "/gifs/Consoles/SNES_Asterix_01.gif",
            ),
            (
                "/gifs/Computers/PC_Leilanis Island_42.gif",
                "/gifs/Computers/PC_Leilanis_Island_42.gif",
            ),
            (
                "/gifs/N/NINTENDO_DS_Mario_Kart_DS_01..gif",
                "/gifs/N/NDS_Mario_Kart_DS_01.gif",
            ),
            (
                "/gifs/Consoles/MD_LOGO_Sega31.gif",
                "/gifs/Consoles/MD_Sega_31_LOGO.gif",
            ),
            (
                "/gifs/H/HALLOWEEN_ARCADE_GoldenAxe16.gif",
                "/gifs/H/HALLOWEEN_ARCADE_GoldenAxe_16.gif",
            ),
            (
                "/gifs/C/PSP_Parasite_Eve_01__sephirot68.gif",
                "/gifs/C/PSP_Parasite_Eve_01_sephirot68.gif",
            ),
        ] {
            assert_eq!(
                NameFields::from_path(path).path().as_deref(),
                Ok(want),
                "{path}"
            );
        }
    }

    #[cfg(feature = "names")]
    #[test]
    fn build_name_writes_the_grammar() {
        let b = build_name;
        assert_eq!(
            b("Super Metroid", Some("SNES"), "Consoles", Some(1), None).as_deref(),
            Ok("/gifs/Consoles/SNES_Super_Metroid_01.gif")
        );
        // A title ending in a number stays a title, with the number after it.
        assert_eq!(
            b(
                "Streets of Rage 2",
                Some("MD"),
                "Consoles",
                Some(3),
                Some("ross")
            )
            .as_deref(),
            Ok("/gifs/Consoles/MD_Streets_of_Rage_2_03_ross.gif")
        );
        // Whitespace collapses; an author is written the way it reads back.
        assert_eq!(
            b(
                "  Sunset   Riders ",
                Some("ARCADE"),
                "Arcade",
                Some(12),
                Some("Ross")
            )
            .as_deref(),
            Ok("/gifs/Arcade/ARCADE_Sunset_Riders_12_ross.gif")
        );
        assert_eq!(
            b(
                "my art",
                Some("OTHER"),
                "NEW 2021",
                Some(2),
                Some("rattenjager")
            )
            .as_deref(),
            Ok("/gifs/NEW 2021/OTHER_my_art_02_rattenjager.gif")
        );
        assert_eq!(
            b("Ms. Pac-Man", Some("ARCADE"), "Arcade", Some(1), None).as_deref(),
            Ok("/gifs/Arcade/ARCADE_Ms._Pac-Man_01.gif")
        );
        assert_eq!(
            b("VB Thing", Some("VIRTUAL_BOY"), "C", Some(1), None).as_deref(),
            Ok("/gifs/C/VIRTUAL_BOY_VB_Thing_01.gif")
        );
        // No platform: the whole name is the title, so no number or author.
        assert_eq!(
            b("Sunset", None, "Mine", None, None).as_deref(),
            Ok("/gifs/Mine/Sunset.gif")
        );
        assert_eq!(
            b("my sunset 2", None, "Mine", None, None).as_deref(),
            Ok("/gifs/Mine/my_sunset_2.gif")
        );
        assert_eq!(
            b("Sunset", None, "Mine", Some(1), None),
            Err(NameError::NeedsPlatform)
        );
        assert_eq!(
            b("Sunset", None, "Mine", None, Some("ross")),
            Err(NameError::NeedsPlatform)
        );

        // What cannot be written, or would not read back.
        assert_eq!(
            b("  ", Some("SNES"), "C", Some(1), None),
            Err(NameError::EmptyTitle)
        );
        for bad in [
            "a_b",
            "a/b",
            "what?",
            "a:b",
            "caf\u{e9}",
            "a*b",
            "tab\u{7f}",
            ".hidden",
        ] {
            assert_eq!(
                b(bad, Some("SNES"), "C", Some(1), None),
                Err(NameError::TitleChar),
                "{bad}"
            );
        }
        // A listed platform in any case is written as listed.
        assert_eq!(
            b("X", Some("Snes"), "C", Some(1), None).as_deref(),
            Ok("/gifs/C/SNES_X_01.gif")
        );
        assert_eq!(
            b("X", Some("SN ES"), "C", Some(1), None),
            Err(NameError::Platform)
        );
        for cat in ["", "a/b", " x", ".x", "x?"] {
            assert_eq!(
                b("X", Some("SNES"), cat, Some(1), None).map(|_| ()),
                if cat.is_empty() {
                    Ok(())
                } else {
                    Err(NameError::Category)
                },
                "{cat:?}"
            );
        }
        assert_eq!(
            b("X", Some("SNES"), "", Some(1), None).as_deref(),
            Ok("/gifs/SNES_X_01.gif")
        );
        for bad in ["ab", "intro", "123", "a b", "-ab"] {
            assert_eq!(
                b("X", Some("SNES"), "C", Some(1), Some(bad)),
                Err(NameError::Author),
                "{bad}"
            );
        }
        assert_eq!(
            b(&"Long ".repeat(30), Some("SNES"), "C", Some(1), None),
            Err(NameError::TooLong)
        );
        // A title word the grammar takes for a tag.
        assert_eq!(
            b("Toy Story", Some("SNES"), "C", Some(1), None),
            Err(NameError::ReadsBack)
        );
        assert_eq!(
            b("Intro Stage", Some("SNES"), "C", Some(1), None),
            Err(NameError::ReadsBack)
        );
        assert_eq!(
            b("SNES Classics", None, "C", None, None),
            Err(NameError::ReadsBack)
        );
        // A first word that is not a listed platform is just a word.
        assert_eq!(
            b("THE END", None, "C", None, None),
            Ok("/gifs/C/THE_END.gif".into())
        );
        assert_eq!(
            b("Mine Storm", Some("VCS"), "C", Some(1), None),
            Err(NameError::Platform)
        );
        assert_eq!(
            b("Doom 64", Some("N64"), "C", None, None),
            Err(NameError::ReadsBack)
        );
        assert_eq!(
            b("Doom 64", Some("N64"), "C", Some(1), None).as_deref(),
            Ok("/gifs/C/N64_Doom_64_01.gif")
        );
        assert_eq!(
            b("ARCADE Night", Some("HALLOWEEN"), "C", Some(1), None),
            Err(NameError::ReadsBack)
        );
        // Error codes are what the web app maps to sentences.
        assert_eq!(NameError::ReadsBack.code(), "reads-back");
    }

    #[test]
    fn real_filenames_from_the_card() {
        check(
            "SNES_Donkey_Kong_Country_05.gif",
            Some("SNES"),
            "Donkey Kong Country",
            Some(5),
            &[],
        );
        check(
            "MD_Sonic3_RattenJager.gif",
            Some("MD"),
            "Sonic3",
            None,
            &["RattenJager"],
        );
        check("SNES_Asterix01.gif", Some("SNES"), "Asterix", Some(1), &[]);
        check(
            "SNES_Mother02_RattenJager.gif",
            Some("SNES"),
            "Mother",
            Some(2),
            &["RattenJager"],
        );
        check(
            "NINTENDO_DS_Mario_Kart_DS_01..gif",
            Some("NDS"),
            "Mario Kart DS",
            Some(1),
            &[],
        );
        check(
            "NES_Unknow_01_Invert.gif",
            Some("NES"),
            "Unknow",
            Some(1),
            &["Invert"],
        );
        check(
            "HALLOWEEN_ARCADE_GoldenAxe16.gif",
            Some("ARCADE"),
            "GoldenAxe",
            Some(16),
            &["HALLOWEEN"],
        );
        check(
            "SMS_Aladdin_Intro.gif",
            Some("SMS"),
            "Aladdin",
            None,
            &["Intro"],
        );
        check(
            "PICO-8_Downstream_Dream_02.gif",
            Some("PICO-8"),
            "Downstream Dream",
            Some(2),
            &[],
        );
        check(
            "GBA_Wario_Ware_Inc_04.gif",
            Some("GBA"),
            "Wario Ware Inc",
            Some(4),
            &[],
        );
        check(
            "MD_LOGO_Sega31.gif",
            Some("MD"),
            "Sega",
            Some(31),
            &["LOGO"],
        );
        check(
            "NES_The_Ren_n_Stimpy_Show.gif",
            Some("NES"),
            "The Ren n Stimpy Show",
            None,
            &[],
        );
        check("SNES_StarFox08.gif", Some("SNES"), "StarFox", Some(8), &[]);
    }

    #[test]
    fn invert_flag_is_case_insensitive() {
        for name in [
            "NES_Unknow_01_Invert.gif",
            "NES_Unknow_01_INVERT.gif",
            "NES_Unknow_01_invert.gif",
        ] {
            let e = p(&format!("/gifs/Consoles/{name}"));
            assert!(e.has_flag("INVERT"), "{name}");
            assert_eq!(e.title, "Unknow");
        }
    }

    #[test]
    fn only_the_gif_extension_is_stripped() {
        assert_eq!(p("/gifs/X/SNES_Plok_08.GIF").title, "Plok");
        // Only the extension is stripped, and a multi-byte tail cannot panic.
        assert_eq!(p("/gifs/X/SNES_gif_Test").title, "gif Test");
        assert_eq!(p("/gifs/X/SNES_Caf\u{e9}").title, "Caf\u{e9}");
    }

    #[test]
    fn category_comes_from_the_directory() {
        assert_eq!(
            p("/gifs/NEW 2021/SNES_Test_01.gif").category.as_deref(),
            Some("NEW 2021")
        );
        assert_eq!(
            p("/gifs/Pinball_Short/PINBALL_Addams_01.gif")
                .category
                .as_deref(),
            Some("Pinball_Short")
        );
        // Not under gifs/, and no subdirectory: no category.
        assert_eq!(p("/logos/boot.gif").category, None);
        assert_eq!(p("/gifs/loose.gif").category, None);
    }

    #[test]
    fn a_prepared_copy_takes_the_category_not_the_bucket() {
        for (gif, copy) in [
            (
                "/gifs/Consoles/MD_Sonic3_RattenJager.gif",
                "/cache/Consoles/38/MD_Sonic3_RattenJager.gif",
            ),
            (
                "/gifs/NEW 2021/NINTENDO_DS_Mario_Kart_DS_01..gif",
                "/cache/NEW 2021/1a/NINTENDO_DS_Mario_Kart_DS_01..gif",
            ),
            (
                "/gifs/Pinball_Short/PINBALL_SHORT_WWF_40.gif",
                "/cache/Pinball_Short/fc/PINBALL_SHORT_WWF_40.gif",
            ),
            // A category that itself looks like a bucket is still a category.
            ("/gifs/ab/SNES_Plok_08.gif", "/cache/ab/68/SNES_Plok_08.gif"),
        ] {
            let (g, c) = (p(gif), p(copy));
            assert_eq!(c.path, copy);
            assert_eq!(
                Entry {
                    path: g.path.clone(),
                    ..c
                },
                g,
                "{copy}"
            );
        }
        // Nested GIF directories: the first one is the category, as under gifs/.
        assert_eq!(p("/cache/A/B/af/x.gif").category.as_deref(), Some("A"));
        assert_eq!(p("/gifs/A/B/x.gif").category.as_deref(), Some("A"));
        // Loose: only a bucket between cache/ and the file, so no category.
        assert_eq!(p("/cache/af/x.gif").category, None);
        assert_eq!(p("/cache/x.gif").category, None);
        // Not a bucket (upper case, three digits): an unbucketed category.
        assert_eq!(p("/cache/Arcade/x.gif").category.as_deref(), Some("Arcade"));
        assert_eq!(p("/cache/7E/x.gif").category.as_deref(), Some("7E"));
    }

    #[test]
    fn season_prefix_only_wins_in_front_of_a_platform() {
        let e = p("/gifs/Halloween/HALLOWEEN_Dracula_01.gif");
        assert_eq!(e.platform.as_deref(), Some("HALLOWEEN"));
        assert_eq!(e.title, "Dracula");
        assert!(e.flags.is_empty());
    }

    #[test]
    fn unknown_prefixes() {
        // A listed platform the original spec list missed.
        let e = p("/gifs/Consoles/PSP_Parasite_Eve_01__sephirot68.gif");
        assert_eq!(e.platform.as_deref(), Some("PSP"));
        assert_eq!(e.title, "Parasite Eve");
        assert_eq!(e.variant, Some(1));
        assert_eq!(e.flags, vec!["sephirot68"]);

        // An unlisted all-caps first word is part of the title, which keeps
        // every word (no number or author without a platform).
        for (path, title) in [
            (
                "/gifs/The_80s_90s/THE_80s_90s_SCROLL_Judge_Dredd_01_sephirot68.gif",
                "THE 80s 90s SCROLL Judge Dredd 01 sephirot68",
            ),
            (
                "/gifs/Happy_Birthday/HDB_Happy_Birday_03.gif",
                "HDB Happy Birday 03",
            ),
            ("/gifs/Consoles/POKEMON_MINI_01.gif", "POKEMON MINI 01"),
            ("/gifs/Consoles/LOGO_SATURN01.gif", "LOGO SATURN01"),
            (
                "/gifs/XMAS/XMAX_PICO-8_Santa_Panic_01.gif",
                "XMAX PICO-8 Santa Panic 01",
            ),
        ] {
            let e = p(path);
            assert_eq!(e.platform, None, "{path}");
            assert_eq!(e.title, title, "{path}");
            assert_eq!(e.variant, None, "{path}");
            assert!(e.flags.is_empty(), "{path}");
        }

        // Parsing is case-sensitive: mixed case is not a platform.
        let e = p("/gifs/Consoles/Nes_Gradius.gif");
        assert_eq!(e.platform, None);
        assert_eq!(e.title, "Nes Gradius");

        // A season in front of an unlisted word is the platform itself.
        let e = p("/gifs/XMAS/XMAS_SANTA_Panic_01.gif");
        assert_eq!(e.platform.as_deref(), Some("XMAS"));
        assert_eq!(e.title, "SANTA Panic");
        assert_eq!(e.variant, Some(1));
        assert!(e.flags.is_empty());

        // No underscore at all.
        let e = p("/gifs/Spectrum_Slide/Exolon2.gif");
        assert_eq!(e.platform, None);
        assert_eq!(e.title, "Exolon2");
    }

    #[test]
    fn a_season_in_front_of_a_listed_platform_is_a_flag() {
        let e = p("/gifs/Halloween/HALLOWEEN_PSX_Castlevania-Symphony_of_the_Night_01.gif");
        assert_eq!(e.platform.as_deref(), Some("PSX"));
        assert_eq!(e.title, "Castlevania-Symphony of the Night");
        assert_eq!(e.flags, vec!["HALLOWEEN"]);
        // `ATARI_ST`, `ZX_SPECTRUM`: the second word stays in the title.
        let e = p("/gifs/Computers/ATARI_ST_Xenon_2_01.gif");
        assert_eq!(e.platform.as_deref(), Some("ATARI"));
        assert_eq!(e.title, "ST Xenon 2");
    }

    #[test]
    fn normalize_title_strips_to_alphanumerics() {
        assert_eq!(normalize_title("Donkey Kong Country"), "donkeykongcountry");
        assert_eq!(normalize_title("PICO-8"), "pico8");
        assert_eq!(normalize_title("  "), "");
    }

    #[test]
    fn manifest_roundtrip_is_byte_exact() {
        let card = "/gifs/Consoles/SNES_Plok_08.gif\n\
                    /gifs/Arcade/ARCADE_Pac-Man_01.gif\n\
                    /gifs/NEW 2021/MD_Sonic3_RattenJager.gif\n";
        let entries = parse_manifest(card);
        assert_eq!(entries.len(), 3);
        assert_eq!(render_manifest(&entries), card);
    }

    #[test]
    fn parsing_tolerates_bom_crlf_and_blank_lines() {
        let messy = "\u{feff}/gifs/Consoles/SNES_Plok_08.gif\r\n\
                     \r\n\
                     /gifs/Arcade/ARCADE_Pac-Man_01.gif   \r\n";
        let entries = parse_manifest(messy);
        assert_eq!(entries.len(), 2);
        // Leading slashes preserved, output is LF-terminated lines.
        assert_eq!(
            render_manifest(&entries),
            "/gifs/Consoles/SNES_Plok_08.gif\n/gifs/Arcade/ARCADE_Pac-Man_01.gif\n"
        );
        assert!(!render_manifest(&entries).contains("\r"));
        assert!(entries.iter().all(|e| e.path.starts_with('/')));
    }

    #[test]
    fn empty_manifest() {
        assert!(parse_manifest("").is_empty());
        assert_eq!(render_manifest(&[]), "");
        assert!(index_matching(Cursor::new(Vec::new()), &Filter::new())
            .unwrap()
            .is_empty());
    }

    const MESSY: &str = "\u{feff}/gifs/Consoles/SNES_Plok_08.gif\r\n\
                         \r\n\
                         /gifs/Arcade/ARCADE_Pac-Man_01.gif   \r\n\
                         /gifs/Consoles/MD_Sonic3_RattenJager.gif\n";

    fn cursor(text: &str) -> Cursor<Vec<u8>> {
        Cursor::new(text.as_bytes().to_vec())
    }

    #[test]
    fn index_matching_offsets_point_at_line_starts() {
        let offsets = index_matching(cursor(MESSY), &Filter::new()).unwrap();
        assert_eq!(offsets.len(), 3);
        // Absolute file offsets: the BOM is skipped for parsing but still
        // occupies the first three bytes, and the blank line is not indexed.
        let bytes = MESSY.as_bytes();
        assert_eq!(offsets[0], 0);
        for off in &offsets {
            let o = *off as usize;
            assert!(
                o == 0 || bytes[o - 1] == b'\n',
                "offset {o} is not a line start"
            );
        }
        // And they really are the offsets of the three non-blank lines.
        let mut want = Vec::new();
        let mut at = 0u32;
        for raw in MESSY.split_inclusive('\n') {
            if !trim_line(raw).is_empty() {
                want.push(at);
            }
            at += raw.len() as u32;
        }
        assert_eq!(offsets, want);
    }

    #[test]
    fn index_matching_agrees_with_parse_plus_filter() {
        for filter in [
            Filter::new(),
            Filter::new().platforms(["SNES"]),
            Filter::new().exclude_flag("RattenJager"),
            Filter::new().platforms(Vec::<String>::new()),
        ] {
            let entries = parse_manifest(MESSY);
            let want: Vec<&Entry> = filter
                .apply(&entries)
                .iter()
                .map(|i| &entries[*i])
                .collect();
            let offsets = index_matching(cursor(MESSY), &filter).unwrap();
            assert_eq!(offsets.len(), want.len());
            let mut r = cursor(MESSY);
            for (off, expected) in offsets.iter().zip(want) {
                assert_eq!(&entry_at(&mut r, *off).unwrap(), expected);
            }
        }
    }

    /// A playlist from before the built-in effects were Formulas: its
    /// `@fx:` lines (the natives', and the Formula path's old `@fx:psi`
    /// name) are skipped, each with one log line, and the rest plays.
    #[test]
    fn retired_effect_lines_are_skipped_and_logged() {
        use std::sync::Mutex;
        static SEEN: Mutex<Vec<String>> = Mutex::new(Vec::new());
        struct Capture;
        impl log::Log for Capture {
            fn enabled(&self, _: &log::Metadata) -> bool {
                true
            }
            fn log(&self, r: &log::Record) {
                let m = r.args().to_string();
                // Other tests log too, on other threads: only ours.
                if m.contains("retired-test") {
                    SEEN.lock().unwrap().push(format!("{} {m}", r.level()));
                }
            }
            fn flush(&self) {}
        }
        if log::set_logger(&Capture).is_ok() {
            log::set_max_level(log::LevelFilter::Warn);
        }
        let text = "/gifs/Consoles/SNES_Plok_08.gif\n\
                    @fx:tunnel?palette=fire&retired-test=1\n\
                    @clock\n\
                    @fx:psi?src=gray(x)&name=retired-test\r\n\
                    @formula?src=gray(x)&name=Kept\n";
        let want = [
            "/gifs/Consoles/SNES_Plok_08.gif",
            "@clock",
            "@formula?src=gray(x)&name=Kept",
        ];
        let paths = |v: Vec<Entry>| v.into_iter().map(|e| e.path).collect::<Vec<_>>();
        assert_eq!(paths(parse_manifest(text)), want);
        let offsets = index_matching(cursor(text), &Filter::new()).unwrap();
        let mut r = cursor(text);
        let read: Vec<_> = offsets.iter().map(|&o| entry_at(&mut r, o).unwrap()).collect();
        assert_eq!(paths(read), want);
        // One line for each, each time the playlist is read (twice here).
        let seen = SEEN.lock().unwrap().clone();
        assert_eq!(seen.len(), 4, "{seen:#?}");
        assert!(seen.iter().all(|m| m.starts_with("WARN playlist: skipping \"@fx:")), "{seen:#?}");
        assert!(seen[0].contains("@fx:tunnel?palette=fire&retired-test=1"));
        assert!(seen[0].ends_with("the built-in effects are gone (Formulas replace them)"));
    }

    /// A Formula in a playlist: its path may be up to 2,048 B (past the
    /// stack buffer); it reads back whole, and the lines around it too.
    #[test]
    fn a_formula_line_reads_back_whole() {
        let mut src = String::from("@formula?src=loop+8");
        while src.len() < crate::synth::MAX_FORMULA_PATH - 40 {
            src.push_str("%3B+v+%3D+x+%2A+2+%23f80");
        }
        src.push_str("&a=0.5&name=Star+field");
        assert!(src.len() > MAX_LINE && src.len() <= crate::synth::MAX_FORMULA_PATH);
        let text = format!(
            "/gifs/Consoles/SNES_Plok_08.gif\n{src}\r\n/gifs/Arcade/ARCADE_Pac-Man_01.gif\n"
        );
        let offsets = index_matching(cursor(&text), &Filter::new()).unwrap();
        assert_eq!(offsets.len(), 3);
        let mut r = cursor(&text);
        let e = entry_at(&mut r, offsets[1]).unwrap();
        assert_eq!(e.path, src);
        assert_eq!(e.title, "Star field");
        assert_eq!(
            entry_at(&mut r, offsets[2]).unwrap().path,
            "/gifs/Arcade/ARCADE_Pac-Man_01.gif"
        );
        // The same as the whole file parsed at once; and as the last line,
        // with no newline after it.
        assert_eq!(parse_manifest(&text)[1], e);
        let last = format!("/gifs/x.gif\n{src}");
        assert_eq!(entry_at(&mut cursor(&last), 12).unwrap().path, src);
        // A line past any path's limit is cut, never read on without end.
        let huge = "/gifs/".to_string() + &"x".repeat(10_000);
        assert_eq!(
            entry_at(&mut cursor(&huge), 0).unwrap().path.len(),
            MAX_LONG_LINE
        );
    }

    #[test]
    fn entry_at_rejects_a_blank_or_past_the_end_offset() {
        let mut r = cursor(MESSY);
        // The blank line between the first two entries.
        let blank = MESSY.find("\r\n\r\n").unwrap() as u32 + 2;
        assert!(entry_at(&mut r, blank).is_err());
        assert!(entry_at(&mut r, MESSY.len() as u32).is_err());
    }
}

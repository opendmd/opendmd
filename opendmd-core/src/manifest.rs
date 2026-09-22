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
//!    known single-token list ([`PLATFORMS`]). An unknown first token is still
//!    taken as the platform if it looks like one — all `A-Z`, `0-9`, `-`, with
//!    at least one letter (`PICO-8` qualifies). Anything else leaves
//!    `platform: None` and the whole stem becomes the title.
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

/// Known single-token platform prefixes.
pub const PLATFORMS: &[&str] = &[
    "SNES",
    "NES",
    "MD",
    "GBA",
    "GBC",
    "GB",
    "SMS",
    "PCE",
    "PS1",
    "N64",
    "SATURN",
    "GAMEGEAR",
    "NGP",
    "NGPC",
    "NDS",
    "PICO-8",
    "AMIGA",
    "C64",
    "ATARI",
    "AMSTRAD",
    "PC",
    "ARCADE",
    "PINBALL",
    "OTHER",
    "MOVIES",
    "HALLOWEEN",
    "XMAS",
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
    /// Directory under `gifs/`, e.g. `Consoles` or `NEW 2021`.
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
        .filter(|line| !line.is_empty())
        .map(Entry::from_path)
        .collect()
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
/// lines and trailing whitespace.
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
        if line.is_empty() {
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
/// allocation beyond the returned [`Entry`]'s own strings.
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

    let end = buf[..filled]
        .iter()
        .position(|b| *b == b'\n')
        .unwrap_or(filled);
    let raw = std::str::from_utf8(&buf[..end])
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let line = trim_line(raw);
    if line.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("no manifest line at offset {offset}"),
        ));
    }
    Ok(Entry::from_path(line))
}

fn category_of(path: &str) -> Option<String> {
    let mut parts = path.split('/').filter(|p| !p.is_empty());
    let first = parts.next()?;
    if !first.eq_ignore_ascii_case("gifs") {
        return None;
    }
    let second = parts.next()?;
    // Only a directory counts as a category; `/gifs/foo.gif` has none.
    parts.next()?;
    Some(second.to_string())
}

/// Filename minus directories, minus any trailing `.gif` extensions and the
/// stray dots around them (`..._01..gif` is real).
fn stem_of(path: &str) -> &str {
    let name = path.rsplit('/').next().unwrap_or(path);
    let mut stem = name;
    loop {
        let trimmed = stem.trim_end_matches('.');
        let next =
            if trimmed.len() >= 4 && trimmed[trimmed.len() - 4..].eq_ignore_ascii_case(".gif") {
                &trimmed[..trimmed.len() - 4]
            } else {
                trimmed
            };
        if next.len() == stem.len() {
            return next;
        }
        stem = next;
    }
}

/// `A-Z`, `0-9` and `-` only, with at least one letter.
fn looks_like_platform(token: &str) -> bool {
    !token.is_empty()
        && token.chars().any(|c| c.is_ascii_uppercase())
        && token
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '-')
}

fn is_keyword_flag(token: &str) -> bool {
    KEYWORD_FLAGS.iter().any(|k| token.eq_ignore_ascii_case(k))
}

fn is_known_platform(token: &str) -> bool {
    PLATFORMS.contains(&token)
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
    if platform.is_none()
        && rest.len() > 1
        && (is_known_platform(rest[0]) || looks_like_platform(rest[0]))
    {
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
    fn season_prefix_only_wins_in_front_of_a_platform() {
        let e = p("/gifs/Halloween/HALLOWEEN_Dracula_01.gif");
        assert_eq!(e.platform.as_deref(), Some("HALLOWEEN"));
        assert_eq!(e.title, "Dracula");
        assert!(e.flags.is_empty());
    }

    #[test]
    fn unknown_prefixes() {
        // All-caps token is taken as a platform even though it is not listed.
        let e = p("/gifs/Consoles/PSP_Parasite_Eve_01__sephirot68.gif");
        assert_eq!(e.platform.as_deref(), Some("PSP"));
        assert_eq!(e.title, "Parasite Eve");
        assert_eq!(e.variant, Some(1));
        assert_eq!(e.flags, vec!["sephirot68"]);

        // Mixed case is not a platform: whole stem becomes the title.
        let e = p("/gifs/Consoles/Nes_Gradius.gif");
        assert_eq!(e.platform, None);
        assert_eq!(e.title, "Nes Gradius");

        // No underscore at all.
        let e = p("/gifs/Spectrum_Slide/Exolon2.gif");
        assert_eq!(e.platform, None);
        assert_eq!(e.title, "Exolon2");
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

    #[test]
    fn entry_at_rejects_a_blank_or_past_the_end_offset() {
        let mut r = cursor(MESSY);
        // The blank line between the first two entries.
        let blank = MESSY.find("\r\n\r\n").unwrap() as u32 + 2;
        assert!(entry_at(&mut r, blank).is_err());
        assert!(entry_at(&mut r, MESSY.len() as u32).is_err());
    }
}

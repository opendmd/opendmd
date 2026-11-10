//! The card's path-keyed sidecar files, shared: a line per GIF (or per
//! playlist), the path exactly as a playlist names it, then tab-separated
//! `key=value` fields.
//!
//! ```text
//! /gifs/Consoles/SNES_Plok_08.gif\tw=3
//! /gifs/Consoles/SNES_Plok_08.gif\tr=4\tt=boss,platformer
//! ```
//!
//! A playlist's overrides (`.meta`, [`crate::overrides`]), the user's tags
//! and ratings (`/sets/.tags`, `crate::tags`) and the web app's smart
//! playlists (`/sets/.smart`) are all this shape. Parsing is tolerant in
//! the manifest parser's way: a BOM, CRLF, blank lines, `#` comment lines
//! and trailing whitespace are skipped. Paths are keys compared ASCII
//! case-insensitively (FAT does), through [`path_key`] on the device.

use std::io;

/// One raw line split into its path and its fields: `None` for a blank
/// line, a comment or a line with no path. Fields are not trimmed.
pub fn split_line(raw: &str) -> Option<(&str, std::str::Split<'_, char>)> {
    let line = raw
        .strip_prefix('\u{feff}')
        .unwrap_or(raw)
        .trim_end_matches(['\n', '\r', ' ', '\t']);
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let mut parts = line.split('\t');
    let path = parts.next()?.trim_end();
    (!path.is_empty()).then_some((path, parts))
}

/// A field's key and value: `w=3` is `("w", Some("3"))`, `unrated` is
/// `("unrated", None)`; both trimmed.
pub fn key_value(field: &str) -> (&str, Option<&str>) {
    match field.split_once('=') {
        Some((k, v)) => (k.trim(), Some(v.trim())),
        None => (field.trim(), None),
    }
}

/// FNV-1a 64 over `bytes`, continuing from `h`.
pub(crate) fn fnv1a64(mut h: u64, bytes: &[u8]) -> u64 {
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

pub(crate) const FNV64_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;

/// The fingerprint of a sidecar's bytes, recorded in the playlist index so
/// that a changed file invalidates it: FNV-1a 64 over the content, mixed
/// with its length. Never 0 (which stands for "no file":
/// [`crate::overrides::NO_META`]).
pub fn fingerprint<R: io::Read>(mut reader: R) -> io::Result<u64> {
    let mut h = FNV64_OFFSET;
    let mut len: u64 = 0;
    let mut buf = [0u8; 512];
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h = fnv1a64(h, &buf[..n]);
        len += n as u64;
    }
    let fp = fnv1a64(h, &len.to_le_bytes());
    Ok(if fp == 0 { 1 } else { fp })
}

/// The key a path is held under: FNV-1a 64 of the ASCII-lowercased path
/// (FAT compares names case-insensitively, so the player does too).
pub fn path_key(path: &str) -> u64 {
    let mut h = FNV64_OFFSET;
    for b in path.bytes() {
        h = fnv1a64(h, &[b.to_ascii_lowercase()]);
    }
    h
}

/// One line of a sidecar, its fields as they were (for a file whose fields
/// the caller interprets: the web app's `/sets/.smart`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    pub path: String,
    /// `(key, value)`, in file order; a bare field has an empty value.
    pub fields: Vec<(String, String)>,
}

/// Every line of `text` with at least one field; a later line for the same
/// path (case-insensitively) replaces an earlier one, in the earlier one's
/// place.
pub fn parse(text: &str) -> Vec<Line> {
    let mut out: Vec<Line> = Vec::new();
    for raw in text.split('\n') {
        let Some((path, parts)) = split_line(raw) else {
            continue;
        };
        let fields: Vec<(String, String)> = parts
            .filter(|f| !f.trim().is_empty())
            .map(|f| {
                let (k, v) = key_value(f);
                (k.to_string(), v.unwrap_or("").to_string())
            })
            .filter(|(k, _)| !k.is_empty())
            .collect();
        if fields.is_empty() {
            continue;
        }
        match out.iter_mut().find(|l| l.path.eq_ignore_ascii_case(path)) {
            Some(l) => {
                *l = Line {
                    path: path.to_string(),
                    fields,
                }
            }
            None => out.push(Line {
                path: path.to_string(),
                fields,
            }),
        }
    }
    out
}

/// The canonical text of `lines`: sorted by path (case-insensitively, then
/// as written), fields in the order given, tabs and line breaks in a value
/// made spaces, LF-terminated. Lines without fields are dropped, and a
/// later line for a path already written is too.
pub fn render(lines: &[Line]) -> String {
    let mut sorted: Vec<&Line> = lines
        .iter()
        .filter(|l| !l.fields.is_empty() && !l.path.is_empty())
        .collect();
    sorted.sort_by(|a, b| {
        a.path
            .to_ascii_lowercase()
            .cmp(&b.path.to_ascii_lowercase())
            .then_with(|| a.path.cmp(&b.path))
    });
    sorted.dedup_by(|b, a| a.path.eq_ignore_ascii_case(&b.path));
    let clean = |s: &str| s.replace(['\t', '\n', '\r'], " ");
    let mut out = String::new();
    for l in sorted {
        out.push_str(&clean(&l.path));
        for (k, v) in &l.fields {
            out.push('\t');
            out.push_str(&clean(k));
            if !v.is_empty() {
                out.push('=');
                out.push_str(&clean(v));
            }
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_tolerantly() {
        assert!(split_line("").is_none());
        assert!(split_line("# a comment").is_none());
        assert!(split_line("\t\r\n").is_none());
        let (p, f) = split_line("\u{feff}/gifs/A/x.gif\tw=3\t q=a b \r\n").unwrap();
        assert_eq!(p, "/gifs/A/x.gif");
        assert_eq!(f.collect::<Vec<_>>(), ["w=3", " q=a b"]);
        assert_eq!(key_value(" q = a b "), ("q", Some("a b")));
        assert_eq!(key_value("unrated"), ("unrated", None));
    }

    #[test]
    fn parse_and_render_round_trip() {
        let text = "/sets/b.txt\tq=platform:md\tat=5\n# x\n/sets/A.txt\tq=tag:x\n/SETS/b.TXT\tq=rating>=4\n/sets/c.txt\n";
        let lines = parse(text);
        assert_eq!(lines.len(), 2, "c has no fields; b's later line wins");
        assert_eq!(lines[0].path, "/SETS/b.TXT");
        assert_eq!(
            lines[0].fields,
            [("q".to_string(), "rating>=4".to_string())]
        );
        let out = render(&lines);
        assert_eq!(out, "/sets/A.txt\tq=tag:x\n/SETS/b.TXT\tq=rating>=4\n");
        assert_eq!(render(&parse(&out)), out, "canonical text is a fixed point");
        let odd = [Line {
            path: "/sets/x.txt".into(),
            fields: vec![
                ("q".into(), "a\tb\nc".into()),
                ("flag".into(), String::new()),
            ],
        }];
        assert_eq!(render(&odd), "/sets/x.txt\tq=a b c\tflag\n");
    }

    #[test]
    fn path_key_ignores_case() {
        assert_eq!(path_key("/gifs/A/X.gif"), path_key("/GIFS/a/x.GIF"));
        assert_ne!(path_key("/gifs/A/X.gif"), path_key("/gifs/A/Y.gif"));
    }
}

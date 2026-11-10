//! A GIF's tags and rating: the user's own words about a GIF, kept for
//! the whole card in one file, `/sets/.tags`, and the *details* its name
//! already says (platform, folder, Logo/Intro/…, author), read here the
//! one way the Library, the search and `opendmd-pack` all use.
//!
//! # Card format
//!
//! A sidecar ([`crate::sidecar`]): one line per GIF, the path as playlists
//! name it, then tab-separated fields:
//!
//! ```text
//! /gifs/Arcade/ARCADE_Bubble_Bobble_02.gif\tt=co-op
//! /gifs/Consoles/SNES_Plok_08.gif\tr=4\tt=boss,platformer
//! /gifs/Other/X.gif\tr=1
//! ```
//!
//! | field | meaning |
//! |---|---|
//! | `r=<1..5>` | the star rating; no field is unrated (`0`, `6` and the like are skipped) |
//! | `t=<a,b,…>` | the user's tags: at most [`MAX_TAGS`], each [`clean_tag`]'s shape |
//!
//! Tolerant as the other sidecars (unknown or malformed fields skipped, a
//! later line for a path wins); [`render_tags`] writes the canonical form —
//! lines sorted by path (case-insensitively), `r` then `t`, tags sorted —
//! so two browsers holding the same tags write the same bytes, and a line
//! with nothing left in it is dropped. Paths are compared ASCII
//! case-insensitively (FAT does).
//!
//! The player never reads it: smart playlists are written out as ordinary
//! playlists by the web app ([`crate::query`]).

use crate::manifest::{Entry, KEYWORD_FLAGS, MULTI_PLATFORMS, PLATFORMS, SEASONS};
use crate::sidecar::{self, path_cmp};

/// At most this many tags per GIF; the rest are dropped.
pub const MAX_TAGS: usize = 16;
/// The longest tag, in characters.
pub const MAX_TAG_LEN: usize = 24;

/// A tag as it is kept: ASCII-lowercased, trimmed, inner runs of spaces made
/// one; 1..=[`MAX_TAG_LEN`] characters of `a-z 0-9` and ` _ . + ' -`,
/// starting with a letter or a digit. `None` if it is not one.
pub fn clean_tag(s: &str) -> Option<String> {
    let mut out = String::with_capacity(s.len());
    for w in s.split_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&w.to_ascii_lowercase());
    }
    let ok = !out.is_empty()
        && out.len() <= MAX_TAG_LEN
        && out.as_bytes()[0].is_ascii_alphanumeric()
        && out
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b" _.+'-".contains(&b));
    ok.then_some(out)
}

/// What the user said about one GIF.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct TagLine {
    pub path: String,
    /// 1..=5 stars; `None` is unrated.
    pub rating: Option<u8>,
    /// Sorted, no two alike, each [`clean_tag`]'s shape.
    pub tags: Vec<String>,
}

impl TagLine {
    /// Nothing said: the line would be dropped.
    pub fn is_empty(&self) -> bool {
        self.rating.is_none() && self.tags.is_empty()
    }

    pub fn has_tag(&self, tag: &str) -> bool {
        self.tags.iter().any(|t| t == tag)
    }

    /// Ratings out of range dropped; tags cleaned, sorted, deduplicated and
    /// capped at [`MAX_TAGS`].
    fn normalise(&mut self) {
        self.rating = self.rating.filter(|r| (1..=5).contains(r));
        let mut tags: Vec<String> = self.tags.iter().filter_map(|t| clean_tag(t)).collect();
        sidecar::sort_by(&mut tags, |a, b| a < b);
        tags.dedup();
        tags.truncate(MAX_TAGS);
        self.tags = tags;
    }
}

fn parse_line(raw: &str) -> Option<TagLine> {
    let (path, parts) = sidecar::split_line(raw)?;
    let mut line = TagLine {
        path: path.to_string(),
        ..TagLine::default()
    };
    for field in parts {
        match sidecar::key_value(field) {
            ("r", Some(v)) => match v.parse::<u8>() {
                Ok(r) if (1..=5).contains(&r) => line.rating = Some(r),
                _ => log::debug!("{path}: ignoring rating '{v}'"),
            },
            ("t", Some(v)) => line.tags = v.split(',').map(str::to_string).collect(),
            ("", None) => {}
            _ => log::debug!("{path}: ignoring field '{field}'"),
        }
    }
    line.normalise();
    (!line.is_empty()).then_some(line)
}

/// Every line of a `.tags` file that says something; a later line for the
/// same path (case-insensitively) replaces an earlier one.
pub fn parse_tags(text: &str) -> Vec<TagLine> {
    TagSet::from_text(text).lines
}

/// The canonical text of `lines` (see the module docs).
pub fn render_tags(lines: &[TagLine]) -> String {
    let mut set = TagSet::default();
    for l in lines {
        set.set(&l.path, l.rating, &l.tags);
    }
    set.render()
}

/// The whole file: its lines sorted by path (case-insensitively).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TagSet {
    lines: Vec<TagLine>,
}

impl TagSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_text(text: &str) -> Self {
        let mut set = TagSet::new();
        for line in text.split('\n').filter_map(parse_line) {
            set.put(line);
        }
        set
    }

    fn find(&self, path: &str) -> Result<usize, usize> {
        self.lines.binary_search_by(|l| path_cmp(&l.path, path))
    }

    /// A line in its place, replacing one for the same path.
    fn put(&mut self, line: TagLine) {
        match self.find(&line.path) {
            Ok(i) => self.lines[i] = line,
            Err(i) => self.lines.insert(i, line),
        }
    }

    fn take(&mut self, path: &str) -> Option<TagLine> {
        self.find(path).ok().map(|i| self.lines.remove(i))
    }

    pub fn len(&self) -> usize {
        self.lines.len()
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    pub fn get(&self, path: &str) -> Option<&TagLine> {
        self.find(path).ok().map(|i| &self.lines[i])
    }

    /// Replace what is said about `path` (tags cleaned; ones that are not
    /// tags dropped); nothing at all removes its line.
    pub fn set(&mut self, path: &str, rating: Option<u8>, tags: &[String]) {
        let mut line = TagLine {
            path: path.to_string(),
            rating,
            tags: tags.to_vec(),
        };
        line.normalise();
        if line.is_empty() || path.is_empty() {
            self.take(path);
        } else {
            self.put(line);
        }
    }

    /// A GIF renamed or moved: its line follows. Onto a path that has one,
    /// the tags are merged and the moved GIF's rating wins (if it has one).
    /// False if `from` had nothing.
    pub fn rename(&mut self, from: &str, to: &str) -> bool {
        let Some(mut line) = self.take(from) else {
            return false;
        };
        if let Some(old) = self.take(to) {
            line.rating = line.rating.or(old.rating);
            line.tags.extend(old.tags);
        }
        line.path = to.to_string();
        line.normalise();
        self.put(line);
        true
    }

    /// A GIF deleted: its line goes. False if it had none.
    pub fn drop_path(&mut self, path: &str) -> bool {
        self.take(path).is_some()
    }

    pub fn iter(&self) -> impl Iterator<Item = &TagLine> {
        self.lines.iter()
    }

    /// Every tag in use with the number of GIFs that have it, most first
    /// (then by name).
    pub fn tag_counts(&self) -> Vec<(String, usize)> {
        let mut out: Vec<(String, usize)> = Vec::new();
        for t in self.lines.iter().flat_map(|l| &l.tags) {
            match out.iter_mut().find(|(u, _)| u == t) {
                Some((_, n)) => *n += 1,
                None => out.push((t.clone(), 1)),
            }
        }
        sidecar::sort_by(&mut out, |a, b| a.1 > b.1 || (a.1 == b.1 && a.0 < b.0));
        out
    }

    /// The canonical file.
    pub fn render(&self) -> String {
        let mut out = String::with_capacity(self.lines.len() * 64);
        for l in &self.lines {
            out.push_str(&l.path);
            if let Some(r) = l.rating {
                out.push_str("\tr=");
                out.push(char::from(b'0' + r));
            }
            if !l.tags.is_empty() {
                out.push_str("\tt=");
                out.push_str(&l.tags.join(","));
            }
            out.push('\n');
        }
        out
    }
}

// --- What the name says ---------------------------------------------------

/// The keyword details a name (or its folder) can carry, as the search
/// spells them after `is:`. `short` comes only from a folder (`Pinball_Short`).
pub const IS_WORDS: &[&str] = &[
    "invert",
    "intro",
    "logo",
    "slide",
    "story",
    "anim",
    "static",
    "halloween",
    "xmas",
    "short",
];

/// The catch-all platforms a folder never lends (`Arcade/`, `Halloween/`:
/// what they hold says nothing about a platform the name left out).
const NOT_LENT: &[&str] = &["ARCADE", "PINBALL", "OTHER", "MOVIES", "HALLOWEEN", "XMAS"];

/// A GIF's details as its name and folder give them: what the search's
/// `platform:`, `folder:`, `is:` and `by:` test, and what the Library shows.
/// Never the user's tags (those are [`TagLine`]).
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Details {
    /// The name's platform, else one the folder's first word names
    /// (`Spectrum_Slide/` lends `SPECTRUM`), never a catch-all.
    pub platform: Option<String>,
    pub folder: Option<String>,
    /// [`IS_WORDS`] it has, lowercased, in that order.
    pub is: Vec<&'static str>,
    /// The author handle in the name.
    pub by: Option<String>,
}

fn is_listed_platform(p: &str) -> bool {
    PLATFORMS.contains(&p) || MULTI_PLATFORMS.iter().any(|(_, n)| *n == p)
}

/// The details of `e` (see [`Details`]).
pub fn details(e: &Entry) -> Details {
    let folder = e.category.clone();
    let words: Vec<&str> = folder
        .as_deref()
        .unwrap_or("")
        .split(['_', ' '])
        .filter(|w| !w.is_empty())
        .collect();
    let platform = e.platform.clone().or_else(|| {
        let first = words.first()?.to_ascii_uppercase();
        (is_listed_platform(&first) && !NOT_LENT.contains(&first.as_str())).then_some(first)
    });
    let last = words
        .last()
        .map(|w| w.to_ascii_lowercase())
        .filter(|_| words.len() > 1);
    let in_folder = |c: &str| folder.as_deref().is_some_and(|f| f.eq_ignore_ascii_case(c));
    let is = IS_WORDS
        .iter()
        .copied()
        .filter(|w| match *w {
            "halloween" => {
                e.has_flag("HALLOWEEN")
                    || in_folder("Halloween")
                    || e.platform.as_deref() == Some("HALLOWEEN")
            }
            "xmas" => {
                e.has_flag("XMAS") || in_folder("XMAS") || e.platform.as_deref() == Some("XMAS")
            }
            "slide" | "story" if e.has_flag(&w.to_ascii_uppercase()) => true,
            "slide" | "story" | "short" => last.as_deref() == Some(*w),
            _ => e.has_flag(&w.to_ascii_uppercase()),
        })
        .collect();
    let by = e
        .flags
        .iter()
        .find(|f| {
            !KEYWORD_FLAGS
                .iter()
                .chain(SEASONS)
                .any(|k| k.eq_ignore_ascii_case(f))
        })
        .cloned();
    Details {
        platform,
        folder,
        is,
        by,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleans_tags() {
        assert_eq!(clean_tag("  Boss  Fight "), Some("boss fight".into()));
        assert_eq!(clean_tag("co-op"), Some("co-op".into()));
        assert_eq!(clean_tag("c++"), Some("c++".into()));
        assert_eq!(clean_tag("90's"), Some("90's".into()));
        assert_eq!(clean_tag("a\tb"), Some("a b".into()), "a tab is a space");
        for bad in ["", "   ", "-x", "a,b", "é", "x:y", "\"q\"", &"a".repeat(25)] {
            assert_eq!(clean_tag(bad), None, "{bad:?}");
        }
        assert_eq!(clean_tag(&"a".repeat(24)).map(|t| t.len()), Some(24));
    }

    #[test]
    fn canonical_file_round_trips() {
        let text = "\u{feff}# mine\r\n/gifs/Consoles/SNES_Plok_08.gif\tt=Platformer,boss\tr=4\r\n\
            /gifs/Arcade/ARCADE_Bubble_Bobble_02.gif\tt=co-op\tx=1\n\
            \n/gifs/Other/X.gif\tr=1\n";
        let set = TagSet::from_text(text);
        let out = set.render();
        assert_eq!(
            out,
            "/gifs/Arcade/ARCADE_Bubble_Bobble_02.gif\tt=co-op\n\
             /gifs/Consoles/SNES_Plok_08.gif\tr=4\tt=boss,platformer\n\
             /gifs/Other/X.gif\tr=1\n"
        );
        assert_eq!(TagSet::from_text(&out).render(), out, "a fixed point");
        assert_eq!(render_tags(&parse_tags(&out)), out);
    }

    #[test]
    fn refuses_bad_ratings_and_tags() {
        let set = TagSet::from_text(
            "/a.gif\tr=0\n/b.gif\tr=6\n/c.gif\tr=x\tt=ok,bad:one,,é\n/d.gif\tr=5\tr=2\n",
        );
        assert!(set.get("/a.gif").is_none(), "r=0 says nothing");
        assert!(set.get("/b.gif").is_none());
        let c = set.get("/c.gif").unwrap();
        assert_eq!((c.rating, c.tags.clone()), (None, vec!["ok".to_string()]));
        assert_eq!(
            set.get("/d.gif").unwrap().rating,
            Some(2),
            "the later field wins"
        );
        let many: Vec<String> = (0..20).map(|i| format!("t{i:02}")).collect();
        let mut s = TagSet::new();
        s.set("/x.gif", None, &many);
        assert_eq!(s.get("/x.gif").unwrap().tags.len(), MAX_TAGS);
    }

    #[test]
    fn later_line_wins_case_insensitively() {
        let set = TagSet::from_text("/gifs/A/X.gif\tr=2\n/GIFS/a/x.GIF\tt=late\n");
        assert_eq!(set.len(), 1);
        let l = set.get("/gifs/a/x.gif").unwrap();
        assert_eq!((l.path.as_str(), l.rating), ("/GIFS/a/x.GIF", None));
    }

    #[test]
    fn set_rename_and_drop() {
        let mut s = TagSet::new();
        s.set(
            "/gifs/A/x.gif",
            Some(3),
            &["b".into(), "A".into(), "a".into()],
        );
        assert_eq!(s.get("/GIFS/A/X.GIF").unwrap().tags, ["a", "b"]);
        s.set("/gifs/B/y.gif", Some(5), &["c".into()]);
        assert!(
            s.rename("/gifs/a/X.gif", "/gifs/B/Y.gif"),
            "case-insensitive keys"
        );
        let y = s.get("/gifs/B/y.gif").unwrap();
        assert_eq!(
            (y.path.as_str(), y.rating, y.tags.clone()),
            (
                "/gifs/B/Y.gif",
                Some(3),
                vec!["a".into(), "b".into(), "c".into()]
            )
        );
        assert!(s.get("/gifs/A/x.gif").is_none());
        assert!(!s.rename("/gifs/none.gif", "/gifs/z.gif"));
        s.set("/gifs/B/Y.gif", None, &[]);
        assert!(s.is_empty(), "nothing said removes the line");
        s.set("/gifs/C/z.gif", Some(1), &[]);
        assert!(s.drop_path("/gifs/c/Z.gif"));
        assert!(!s.drop_path("/gifs/c/Z.gif"));
    }

    #[test]
    fn counts_tags() {
        let s = TagSet::from_text("/a.gif\tt=x,y\n/b.gif\tt=y\n/c.gif\tt=z\n");
        assert_eq!(
            s.tag_counts(),
            [
                ("y".to_string(), 2),
                ("x".to_string(), 1),
                ("z".to_string(), 1)
            ]
        );
    }

    fn d(path: &str) -> Details {
        details(&Entry::from_path(path))
    }

    #[test]
    fn details_from_the_name() {
        let x = d("/gifs/Consoles/MD_Sonic_Story_03_pixelkid42.gif");
        assert_eq!(x.platform.as_deref(), Some("MD"));
        assert_eq!(x.folder.as_deref(), Some("Consoles"));
        assert_eq!(x.by.as_deref(), Some("pixelkid42"));
        assert_eq!(x.is, ["story"]);
        let x = d("/gifs/Consoles/SNES_LOGO_Nintendo_01.gif");
        assert_eq!(
            (x.platform.as_deref(), x.is.clone(), x.by.clone()),
            (Some("SNES"), vec!["logo"], None)
        );
        let x = d("/gifs/Arcade/HALLOWEEN_ARCADE_Ghosts_16.gif");
        assert_eq!(
            (x.platform.as_deref(), x.is.clone()),
            (Some("ARCADE"), vec!["halloween"])
        );
    }

    #[test]
    fn details_the_folder_lends() {
        // A platform from the folder's first word, when the name has none.
        let x = d("/gifs/Spectrum_Slide/Jet_Set_Willy_01.gif");
        assert_eq!(
            (x.platform.as_deref(), x.is.clone()),
            (Some("SPECTRUM"), vec!["slide"])
        );
        // The name's own platform wins.
        assert_eq!(
            d("/gifs/Spectrum/C64_Elite_01.gif").platform.as_deref(),
            Some("C64")
        );
        // Never a catch-all, and never from a word that is not a platform.
        assert_eq!(d("/gifs/Arcade/Some_Thing.gif").platform, None);
        assert_eq!(d("/gifs/NEW 2021/1.gif").platform, None);
        assert_eq!(d("/gifs/Pixel_Art_Slide/Cat_01.gif").is, ["slide"]);
        let x = d("/gifs/Pinball_Short/PINBALL_Addams_Family_02.gif");
        assert_eq!(
            (x.platform.as_deref(), x.is.clone()),
            (Some("PINBALL"), vec!["short"])
        );
        assert_eq!(
            d("/gifs/Pinball_Story/PINBALL_Twilight_Zone_01.gif").is,
            ["story"]
        );
        // The season folders are seasons.
        assert_eq!(
            d("/gifs/Halloween/NES_Castlevania_01.gif").is,
            ["halloween"]
        );
        assert_eq!(d("/gifs/XMAS/Snow_01.gif").is, ["xmas"]);
        // A one-word folder lends no keyword ("Slide/" is a name, not a suffix).
        assert!(d("/gifs/Slide/NES_Mario_01.gif").is.is_empty());
    }
}

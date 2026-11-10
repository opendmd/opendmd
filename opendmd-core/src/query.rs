//! The search: what the Library's search box and a smart playlist both
//! read, typed as words.
//!
//! ```text
//! sonic platform:md,snes -is:logo tag:boss rating>=4 limit:20 shuffle
//! ```
//!
//! Terms are separated by spaces and must all hold; a term's values,
//! separated by commas, are alternatives. A leading `-` turns a term
//! around. Case does not matter; a value with a space or a comma goes in
//! double quotes (`title:"donkey kong",mario`).
//!
//! | term | holds when |
//! |---|---|
//! | `platform:md` | the GIF's platform ([`crate::tags::Details`]); a console name works too (`platform:"mega drive"`) |
//! | `folder:consoles` | its folder (letters and digits compared: `folder:"best of top 30"`) |
//! | `tag:boss` | the user gave it that tag (`/sets/.tags`) |
//! | `is:logo` | its name (or folder) says so: [`crate::tags::IS_WORDS`] |
//! | `title:sonic` | its title has the words in it |
//! | `by:pixelkid42` | its author |
//! | `rating>=4`, `rating:5`, `rating<3`, … | its stars (`>`, `>=`, `<`, `<=`, `=`, `:`) |
//! | `unrated` | no stars |
//! | anything else | a free word: the title (letters and digits) or the file name has it |
//!
//! `shuffle`, `seed:N` and `limit:N` are not tests but how a smart
//! playlist is made: shuffled with the scheduler's generator, the first N
//! kept. A word followed by `:` that is none of the keys above is a free
//! word too.
//!
//! [`Query::render`] writes the canonical text, which parses back to the
//! same query; [`Query::describe`] the line a person reads.

use crate::follow::console_name;
use crate::manifest::{normalize_title, Entry, MULTI_PLATFORMS, PLATFORMS};
use crate::scheduler::Rng;
use crate::tags::{clean_tag, details, TagLine, TagSet, IS_WORDS};

/// How a rating is compared.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cmp {
    Eq,
    Lt,
    Le,
    Gt,
    Ge,
}

impl Cmp {
    fn text(self) -> &'static str {
        match self {
            Cmp::Eq => ":",
            Cmp::Lt => "<",
            Cmp::Le => "<=",
            Cmp::Gt => ">",
            Cmp::Ge => ">=",
        }
    }

    fn holds(self, have: u8, want: u8) -> bool {
        match self {
            Cmp::Eq => have == want,
            Cmp::Lt => have < want,
            Cmp::Le => have <= want,
            Cmp::Gt => have > want,
            Cmp::Ge => have >= want,
        }
    }
}

/// What a term tests.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Key {
    /// Upper case, canonical (`MD`).
    Platform,
    Folder,
    /// Cleaned ([`clean_tag`]).
    Tag,
    /// One of [`IS_WORDS`].
    Is,
    Title,
    By,
    Rating(Cmp),
    Unrated,
}

impl Key {
    fn name(&self) -> &'static str {
        match self {
            Key::Platform => "platform",
            Key::Folder => "folder",
            Key::Tag => "tag",
            Key::Is => "is",
            Key::Title => "title",
            Key::By => "by",
            Key::Rating(_) => "rating",
            Key::Unrated => "unrated",
        }
    }
}

/// One test: any of `values` (none for [`Key::Unrated`]), turned around by
/// `neg`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Term {
    pub neg: bool,
    pub key: Key,
    pub values: Vec<String>,
}

/// A parsed search.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Query {
    pub terms: Vec<Term>,
    /// The free words, as typed, one space between them.
    pub text: String,
    pub shuffle: bool,
    pub seed: u64,
    pub limit: Option<usize>,
}

/// Why a search could not be read: `at` is the byte offset of the term.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueryError {
    pub at: usize,
    pub message: String,
}

fn err<T>(at: usize, message: impl Into<String>) -> Result<T, QueryError> {
    Err(QueryError {
        at,
        message: message.into(),
    })
}

/// The search's words: `(offset, text)` per space-separated token, a quoted
/// run kept whole (quotes in).
fn tokens(s: &str) -> Result<Vec<(usize, &str)>, QueryError> {
    let mut out = Vec::new();
    let mut start = None;
    let mut quote = None;
    for (i, c) in s.char_indices() {
        match c {
            '"' => {
                start.get_or_insert(i);
                quote = if quote.is_some() { None } else { Some(i) };
            }
            c if c.is_whitespace() && quote.is_none() => {
                if let Some(b) = start.take() {
                    out.push((b, &s[b..i]));
                }
            }
            _ => {
                start.get_or_insert(i);
            }
        }
    }
    if let Some(q) = quote {
        return err(q, "a \" with no \" to close it");
    }
    if let Some(b) = start {
        out.push((b, &s[b..]));
    }
    Ok(out)
}

/// A term's comma-separated values, quotes taken off, blanks dropped.
fn values(v: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    for c in v.chars() {
        match c {
            '"' => quoted = !quoted,
            ',' if !quoted => out.push(std::mem::take(&mut cur)),
            c => cur.push(c),
        }
    }
    out.push(cur);
    out.into_iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// A platform as the search holds it: a listed one in upper case, else the
/// one whose console name it is, else as typed in upper case (it matches
/// nothing).
fn platform_value(v: &str) -> String {
    let up = v.to_ascii_uppercase().replace(' ', "_");
    if PLATFORMS.contains(&up.as_str()) || MULTI_PLATFORMS.iter().any(|(_, n)| *n == up) {
        return up;
    }
    let want = normalize_title(v);
    PLATFORMS
        .iter()
        .copied()
        .chain(MULTI_PLATFORMS.iter().map(|(_, n)| *n))
        .find(|p| {
            let n = console_name(p);
            !n.is_empty() && normalize_title(n) == want
        })
        .map_or(up, str::to_string)
}

/// `is:` words people may type for the name's own (`still` for `static`).
fn is_value(v: &str) -> Option<&'static str> {
    let v = v.to_ascii_lowercase();
    let v = match v.as_str() {
        "inverted" => "invert",
        "animated" => "anim",
        "still" => "static",
        "christmas" => "xmas",
        v => v,
    };
    IS_WORDS.iter().copied().find(|w| *w == v)
}

/// Read a search (see the module docs).
pub fn parse_query(s: &str) -> Result<Query, QueryError> {
    let mut q = Query::default();
    let mut words: Vec<String> = Vec::new();
    for (at, tok) in tokens(s)? {
        let (neg, body) = match tok.strip_prefix('-') {
            Some(b) if !b.is_empty() => (true, b),
            _ => (false, tok),
        };
        let lower = body.to_ascii_lowercase();
        // rating>=4, rating<3, rating=5, rating:4,5
        if let Some(rest) = lower.strip_prefix("rating") {
            let (cmp, v) = [
                (">=", Cmp::Ge),
                ("<=", Cmp::Le),
                (">", Cmp::Gt),
                ("<", Cmp::Lt),
                ("=", Cmp::Eq),
                (":", Cmp::Eq),
            ]
            .iter()
            .find_map(|(op, c)| rest.strip_prefix(op).map(|v| (*c, v)))
            .map_or((None, ""), |(c, v)| (Some(c), v));
            if let Some(cmp) = cmp {
                let vals = values(v);
                if vals.is_empty() || vals.iter().any(|x| !matches!(x.parse::<u8>(), Ok(1..=5))) {
                    return err(at, "a rating is a number of stars from 1 to 5");
                }
                if cmp != Cmp::Eq && vals.len() > 1 {
                    return err(at, "compare a rating with one number");
                }
                q.terms.push(Term {
                    neg,
                    key: Key::Rating(cmp),
                    values: vals,
                });
                continue;
            }
        }
        match lower.as_str() {
            "unrated" => {
                q.terms.push(Term {
                    neg,
                    key: Key::Unrated,
                    values: Vec::new(),
                });
                continue;
            }
            "shuffle" if !neg => {
                q.shuffle = true;
                continue;
            }
            _ => {}
        }
        let Some((k, v)) = body.split_once(':') else {
            words.push(tok.replace('"', ""));
            continue;
        };
        let key = match k.to_ascii_lowercase().as_str() {
            "platform" => Key::Platform,
            "folder" => Key::Folder,
            "tag" => Key::Tag,
            "is" => Key::Is,
            "title" => Key::Title,
            "by" => Key::By,
            "seed" | "limit" if !neg => {
                let n = v
                    .trim()
                    .parse::<u64>()
                    .ok()
                    .filter(|n| *n > 0 || k.eq_ignore_ascii_case("seed"));
                let Some(n) = n else {
                    return err(
                        at,
                        format!("{} takes a whole number", k.to_ascii_lowercase()),
                    );
                };
                if k.eq_ignore_ascii_case("seed") {
                    q.seed = n;
                } else {
                    q.limit = Some(usize::try_from(n).unwrap_or(usize::MAX));
                }
                continue;
            }
            _ => {
                words.push(tok.replace('"', ""));
                continue;
            }
        };
        let mut vals = values(v);
        if vals.is_empty() {
            return err(at, format!("{}: needs something after it", key.name()));
        }
        match key {
            Key::Platform => vals = vals.iter().map(|v| platform_value(v)).collect(),
            Key::Tag => {
                let Some(c) = vals
                    .iter()
                    .map(|v| clean_tag(v))
                    .collect::<Option<Vec<_>>>()
                else {
                    return err(
                        at,
                        "a tag is letters, digits, spaces and _ . + ' - (24 at most)",
                    );
                };
                vals = c;
            }
            Key::Is => {
                let Some(c) = vals
                    .iter()
                    .map(|v| is_value(v).map(str::to_string))
                    .collect::<Option<Vec<_>>>()
                else {
                    return err(at, format!("is: takes {}", IS_WORDS.join(", ")));
                };
                vals = c;
            }
            _ => {}
        }
        q.terms.push(Term {
            neg,
            key,
            values: vals,
        });
    }
    q.text = words.join(" ");
    Ok(q)
}

/// A value as the canonical text writes it: quoted when it has a space, a
/// comma or a colon.
fn push_value(out: &mut String, v: &str) {
    if v.contains([' ', ',', ':']) || v.is_empty() {
        out.push('"');
        out.push_str(v);
        out.push('"');
    } else {
        out.push_str(v);
    }
}

impl Query {
    /// Does it test anything but free words (what the Library's plain
    /// search does on its own)?
    pub fn is_keyed(&self) -> bool {
        !self.terms.is_empty()
    }

    /// Does it read the user's tags or ratings (so a change to them can
    /// change what it finds)?
    pub fn uses_tags(&self) -> bool {
        self.terms
            .iter()
            .any(|t| matches!(t.key, Key::Tag | Key::Rating(_) | Key::Unrated))
    }

    /// Does the GIF at `e` pass, with `user` what the user said about it?
    pub fn matches(&self, e: &Entry, user: Option<&TagLine>) -> bool {
        if !self.text.is_empty() {
            let want = normalize_title(&self.text);
            let lower = self.text.to_ascii_lowercase();
            let name = e.path.rsplit('/').next().unwrap_or("").to_ascii_lowercase();
            if !(normalize_title(&e.title).contains(&want) || name.contains(&lower)) {
                return false;
            }
        }
        if self.terms.is_empty() {
            return true;
        }
        let d = details(e);
        let rating = user.and_then(|u| u.rating);
        self.terms.iter().all(|t| {
            let any = |f: &dyn Fn(&str) -> bool| t.values.iter().any(|v| f(v));
            let hit = match &t.key {
                Key::Platform => d
                    .platform
                    .as_deref()
                    .is_some_and(|p| any(&|v| v.eq_ignore_ascii_case(p))),
                Key::Folder => d
                    .folder
                    .as_deref()
                    .is_some_and(|f| any(&|v| normalize_title(v) == normalize_title(f))),
                Key::Tag => user.is_some_and(|u| any(&|v| u.has_tag(v))),
                Key::Is => any(&|v| d.is.contains(&v)),
                Key::Title => {
                    let title = normalize_title(&e.title);
                    any(&|v| title.contains(&normalize_title(v)))
                }
                Key::By => {
                    d.by.as_deref()
                        .is_some_and(|b| any(&|v| v.eq_ignore_ascii_case(b)))
                }
                Key::Rating(cmp) => {
                    rating.is_some_and(|r| any(&|v| v.parse().is_ok_and(|w| cmp.holds(r, w))))
                }
                Key::Unrated => rating.is_none(),
            };
            hit != t.neg
        })
    }

    /// Does the playlist line `path` pass? Only GIFs do: a title card or a
    /// Formula is never found.
    pub fn matches_path(&self, path: &str, tags: Option<&TagSet>) -> bool {
        !path.starts_with('@')
            && crate::synth::formula_file(path).is_none()
            && self.matches(&Entry::from_gif_path(path), tags.and_then(|t| t.get(path)))
    }

    /// The paths among `paths` that pass ([`Query::matches_path`]), with
    /// `tags` the card's `/sets/.tags`; shuffled (the scheduler's
    /// splitmix64, so a seed means the same order everywhere) and cut to the
    /// limit as asked.
    pub fn run<'a>(
        &self,
        paths: impl Iterator<Item = &'a str>,
        tags: Option<&TagSet>,
    ) -> Vec<String> {
        let mut out: Vec<String> = paths
            .filter(|p| self.matches_path(p, tags))
            .map(str::to_string)
            .collect();
        if self.shuffle {
            let mut rng = Rng::new(self.seed);
            for i in (1..out.len()).rev() {
                let j = (rng.next_u64() % (i as u64 + 1)) as usize;
                out.swap(i, j);
            }
        }
        if let Some(n) = self.limit {
            out.truncate(n);
        }
        out
    }

    /// The canonical text: the free words, the terms in order, then
    /// `shuffle`, `seed:` and `limit:`. It parses back to the same query.
    pub fn render(&self) -> String {
        let mut out = String::new();
        let sep = |out: &mut String| {
            if !out.is_empty() {
                out.push(' ');
            }
        };
        for w in self.text.split_whitespace() {
            sep(&mut out);
            push_value(&mut out, w);
        }
        for t in &self.terms {
            sep(&mut out);
            if t.neg {
                out.push('-');
            }
            out.push_str(t.key.name());
            match &t.key {
                Key::Unrated => continue,
                Key::Rating(c) => out.push_str(c.text()),
                _ => out.push(':'),
            }
            for (i, v) in t.values.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                let v = if t.key == Key::Platform {
                    v.to_ascii_lowercase()
                } else {
                    v.clone()
                };
                push_value(&mut out, &v);
            }
        }
        if self.shuffle {
            sep(&mut out);
            out.push_str("shuffle");
        }
        if self.seed != 0 {
            sep(&mut out);
            out.push_str(&format!("seed:{}", self.seed));
        }
        if let Some(n) = self.limit {
            sep(&mut out);
            out.push_str(&format!("limit:{n}"));
        }
        out
    }

    /// What it finds, in words: "Mega Drive, SNES · ★ 4+ · tagged boss".
    pub fn describe(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if !self.text.is_empty() {
            parts.push(format!("“{}”", self.text));
        }
        for t in &self.terms {
            let list = |f: &dyn Fn(&str) -> String| {
                t.values.iter().map(|v| f(v)).collect::<Vec<_>>().join(", ")
            };
            let s = match &t.key {
                Key::Platform => list(&|v| {
                    let n = console_name(v);
                    if n.is_empty() {
                        v.to_string()
                    } else {
                        n.to_string()
                    }
                }),
                Key::Folder => format!("folder {}", list(&|v| v.to_string())),
                Key::Tag => format!("tagged {}", list(&|v| v.to_string())),
                Key::Is => list(&|v| {
                    match v {
                        "invert" => "Inverted",
                        "anim" => "Animated",
                        "static" => "Still",
                        "xmas" => "Christmas",
                        "intro" => "Intro",
                        "logo" => "Logo",
                        "slide" => "Slide",
                        "story" => "Story",
                        "halloween" => "Halloween",
                        "short" => "Short",
                        v => v,
                    }
                    .to_string()
                }),
                Key::Title => format!("title {}", list(&|v| format!("“{v}”"))),
                Key::By => format!("by {}", list(&|v| v.to_string())),
                Key::Rating(c) => {
                    let n = list(&|v| v.to_string());
                    match c {
                        Cmp::Eq => format!("★ {n}"),
                        Cmp::Ge => format!("★ {n}+"),
                        Cmp::Gt => format!("★ more than {n}"),
                        Cmp::Le => format!("★ {n} or less"),
                        Cmp::Lt => format!("★ under {n}"),
                    }
                }
                Key::Unrated => "unrated".to_string(),
            };
            parts.push(if t.neg { format!("not {s}") } else { s });
        }
        if self.shuffle {
            parts.push("shuffled".into());
        }
        if let Some(n) = self.limit {
            parts.push(format!("at most {n}"));
        }
        if parts.is_empty() {
            "every GIF".into()
        } else {
            parts.join(" · ")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(s: &str) -> Query {
        parse_query(s).unwrap_or_else(|e| panic!("{s:?}: {e:?}"))
    }

    #[test]
    fn parses_every_key() {
        let x = q("sonic  Platform:MD,\"mega drive\" -is:Logo tag:\"Boss Fight\",co-op rating>=4 by:pixelkid42 folder:consoles title:\"donkey kong\" unrated shuffle seed:7 limit:20");
        assert_eq!(x.text, "sonic");
        assert!(x.shuffle);
        assert_eq!((x.seed, x.limit), (7, Some(20)));
        let keys: Vec<(&Key, bool, Vec<&str>)> = x
            .terms
            .iter()
            .map(|t| (&t.key, t.neg, t.values.iter().map(String::as_str).collect()))
            .collect();
        assert_eq!(
            keys,
            [
                (&Key::Platform, false, vec!["MD", "MD"]),
                (&Key::Is, true, vec!["logo"]),
                (&Key::Tag, false, vec!["boss fight", "co-op"]),
                (&Key::Rating(Cmp::Ge), false, vec!["4"]),
                (&Key::By, false, vec!["pixelkid42"]),
                (&Key::Folder, false, vec!["consoles"]),
                (&Key::Title, false, vec!["donkey kong"]),
                (&Key::Unrated, false, vec![]),
            ]
        );
        assert!(x.is_keyed() && x.uses_tags());
        assert!(!q("sonic 3").is_keyed());
        assert_eq!(q("sonic 3").text, "sonic 3");
        assert!(!q("platform:md is:logo").uses_tags());
    }

    #[test]
    fn unknown_keys_are_free_words() {
        let x = q("re:zero \"two words\"");
        assert_eq!(x.text, "re:zero two words");
        assert!(!x.is_keyed());
    }

    #[test]
    fn errors_say_where() {
        let e = |s: &str| parse_query(s).unwrap_err();
        assert_eq!(e("mario rating>=9").at, 6);
        assert_eq!(e("rating:").at, 0);
        assert_eq!(e("rating>4,5").message, "compare a rating with one number");
        assert_eq!(e("x \"open").at, 2);
        assert_eq!(e("  tag:").at, 2);
        assert_eq!(
            e("tag:a:b").message,
            "a tag is letters, digits, spaces and _ . + ' - (24 at most)"
        );
        assert!(e("is:wobbly").message.starts_with("is: takes invert"));
        assert_eq!(e("limit:0").message, "limit takes a whole number");
        assert_eq!(e("seed:x").at, 0);
    }

    #[test]
    fn render_round_trips() {
        for s in [
            "",
            "sonic",
            "platform:md,snes",
            "tag:\"boss fight\" -is:logo rating:4,5",
            "mario folder:\"best of top 30\" rating<3 -unrated by:pixelkid42",
            "title:\"donkey kong\" shuffle seed:3 limit:10",
        ] {
            let a = q(s);
            let r = a.render();
            assert_eq!(q(&r), a, "{s:?} -> {r:?}");
            assert_eq!(q(&r).render(), r);
        }
        assert_eq!(
            q("  Platform:\"Mega Drive\"  RATING>=4 ").render(),
            "platform:md rating>=4"
        );
    }

    #[test]
    fn describes() {
        assert_eq!(
            q("platform:md,snes rating>=4 tag:boss").describe(),
            "Mega Drive, Super Nintendo · ★ 4+ · tagged boss"
        );
        assert_eq!(q("-is:logo limit:5").describe(), "not Logo · at most 5");
        assert_eq!(q("").describe(), "every GIF");
    }

    const PATHS: &[&str] = &[
        "/gifs/Consoles/MD_Sonic_01.gif",
        "/gifs/Consoles/MD_LOGO_Sega_02.gif",
        "/gifs/Consoles/SNES_Plok_08_pixelkid42.gif",
        "/gifs/Arcade/ARCADE_Bubble_Bobble_02.gif",
        "/gifs/Spectrum_Slide/Jet_Set_Willy_01.gif",
        "/gifs/NEW 2021/7.gif",
        "@formula:plasma",
        "/formulas/mine.psi?a=1",
    ];

    fn run(s: &str, tags: &TagSet) -> Vec<String> {
        q(s).run(PATHS.iter().copied(), Some(tags))
    }

    #[test]
    fn evaluates_the_name() {
        let none = TagSet::new();
        let n = |s: &str| run(s, &none).len();
        assert_eq!(n(""), 6, "GIFs only: never a Formula line");
        assert_eq!(n("platform:md"), 2);
        assert_eq!(n("platform:\"mega drive\" -is:logo"), 1);
        assert_eq!(n("platform:spectrum"), 1, "the folder lends it");
        assert_eq!(n("is:slide"), 1);
        assert_eq!(n("folder:consoles,arcade"), 4);
        assert_eq!(n("folder:\"new 2021\""), 1);
        assert_eq!(n("-folder:consoles"), 3);
        assert_eq!(n("by:PIXELKID42"), 1);
        assert_eq!(n("title:bubble"), 1);
        assert_eq!(n("bobble"), 1);
        assert_eq!(n("7.gif"), 1, "free words match the file name too");
        assert_eq!(n("platform:md platform:snes"), 0, "terms are AND-ed");
        assert_eq!(n("unrated"), 6, "no tags file: nothing is rated");
        assert_eq!(n("tag:boss"), 0);
        assert_eq!(
            q("platform:md").run(PATHS.iter().copied(), None).len(),
            2,
            "without a tags file"
        );
    }

    #[test]
    fn evaluates_tags_and_ratings() {
        let tags = TagSet::from_text(
            "/gifs/consoles/md_sonic_01.gif\tr=5\tt=boss,speed\n\
             /gifs/Consoles/SNES_Plok_08_pixelkid42.gif\tr=3\tt=boss\n\
             /gifs/Arcade/ARCADE_Bubble_Bobble_02.gif\tt=co-op\n",
        );
        let n = |s: &str| run(s, &tags).len();
        assert_eq!(n("tag:boss"), 2, "paths compared without case");
        assert_eq!(n("tag:boss -tag:speed"), 1);
        assert_eq!(n("tag:speed,co-op"), 2);
        assert_eq!(n("rating>=4"), 1);
        assert_eq!(n("rating>=3"), 2);
        assert_eq!(n("rating>3"), 1);
        assert_eq!(n("rating<4"), 1, "an unrated GIF is never under a number");
        assert_eq!(n("rating<=5"), 2);
        assert_eq!(n("rating:3,5"), 2);
        assert_eq!(n("rating=4"), 0);
        assert_eq!(n("unrated"), 4);
        assert_eq!(n("-unrated"), 2);
        assert_eq!(n("platform:md rating:5 tag:boss"), 1);
    }

    #[test]
    fn shuffles_with_a_seed_and_limits() {
        let none = TagSet::new();
        let a = run("shuffle seed:9", &none);
        assert_eq!(a, run("shuffle seed:9", &none), "same seed, same order");
        let mut sorted = a.clone();
        sorted.sort();
        let mut all: Vec<String> = PATHS[..6].iter().map(|s| s.to_string()).collect();
        all.sort();
        assert_eq!(sorted, all);
        assert_eq!(run("shuffle seed:9 limit:2", &none), a[..2]);
        assert_eq!(run("limit:2", &none), PATHS[..2]);
        assert_eq!(run("limit:99", &none).len(), 6);
    }
}

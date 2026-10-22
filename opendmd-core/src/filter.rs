//! Selecting a subset of the manifest.
//!
//! All string matching here is ASCII case-insensitive; titles additionally go
//! through [`normalize_title`](crate::manifest::normalize_title), so a user can
//! type `donkeykong` and match `Donkey Kong Country`.

use crate::manifest::{normalize_title, Entry, SEASONS};
use std::collections::HashSet;

fn upper_set<I, S>(items: I) -> HashSet<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    items
        .into_iter()
        .map(|s| s.as_ref().to_uppercase())
        .collect()
}

/// A playlist filter. Every field is optional; the default matches everything.
///
/// `None` means "no constraint"; an empty `Some(set)` matches nothing, which is
/// the useful behaviour when a user passes an empty `--platforms` list.
#[derive(Clone, Debug, Default)]
pub struct Filter {
    /// Uppercased platform names. An entry matches by its platform, or, for
    /// a season (`HALLOWEEN`, `XMAS`), by carrying that season's flag: a
    /// `HALLOWEEN_PSX_…` file is platform `PSX` and still plays under a
    /// `HALLOWEEN` filter. Anything else with no platform never matches.
    pub platforms: Option<HashSet<String>>,
    /// Uppercased category names to keep.
    pub categories_include: Option<HashSet<String>>,
    /// Uppercased category names to drop (applied after the include set).
    pub categories_exclude: HashSet<String>,
    /// Normalized substrings; an entry matches if *any* of them is contained in
    /// its normalized title.
    pub titles: Option<Vec<String>>,
    /// Uppercased flags; an entry with any of them is dropped.
    pub exclude_flags: HashSet<String>,
}

impl Filter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn platforms<I, S>(mut self, platforms: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.platforms = Some(upper_set(platforms));
        self
    }

    pub fn include_categories<I, S>(mut self, categories: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.categories_include = Some(upper_set(categories));
        self
    }

    pub fn exclude_categories<I, S>(mut self, categories: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.categories_exclude.extend(upper_set(categories));
        self
    }

    /// Add one title substring. Repeatable; the set is OR-ed.
    pub fn title(mut self, substring: &str) -> Self {
        let needle = normalize_title(substring);
        self.titles.get_or_insert_with(Vec::new).push(needle);
        self
    }

    pub fn exclude_flag(mut self, flag: &str) -> Self {
        self.exclude_flags.insert(flag.to_uppercase());
        self
    }

    pub fn exclude_flags<I, S>(mut self, flags: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.exclude_flags.extend(upper_set(flags));
        self
    }

    /// True if this entry survives the filter.
    /// True when every entry matches (`Filter::new()` with nothing added),
    /// so a caller can skip parsing entries it only needed for the filter.
    pub fn is_unconstrained(&self) -> bool {
        self.platforms.is_none()
            && self.categories_include.is_none()
            && self.categories_exclude.is_empty()
            && self.titles.is_none()
            && self.exclude_flags.is_empty()
    }

    pub fn matches(&self, entry: &Entry) -> bool {
        if let Some(wanted) = &self.platforms {
            let by_platform = entry
                .platform
                .as_ref()
                .is_some_and(|p| wanted.contains(&p.to_uppercase()));
            let by_season = || {
                entry.flags.iter().any(|f| {
                    SEASONS.iter().any(|s| f.eq_ignore_ascii_case(s))
                        && wanted.contains(&f.to_uppercase())
                })
            };
            if !by_platform && !by_season() {
                return false;
            }
        }

        let category = entry.category.as_ref().map(|c| c.to_uppercase());
        if let Some(wanted) = &self.categories_include {
            match &category {
                Some(c) if wanted.contains(c) => {}
                _ => return false,
            }
        }
        if let Some(c) = &category {
            if self.categories_exclude.contains(c) {
                return false;
            }
        }

        if !self.exclude_flags.is_empty()
            && entry
                .flags
                .iter()
                .any(|f| self.exclude_flags.contains(&f.to_uppercase()))
        {
            return false;
        }

        if let Some(needles) = &self.titles {
            let haystack = normalize_title(&entry.title);
            if !needles.iter().any(|n| haystack.contains(n.as_str())) {
                return false;
            }
        }

        true
    }

    /// A stable 64-bit identity for what this filter selects: FNV-1a over a
    /// canonical serialisation.
    ///
    /// The on-card playlist index (`<manifest>.idx`, see [`crate::index`])
    /// records the fingerprint of the filter it was built with, so a device can
    /// tell at boot whether the index on the card still answers its current
    /// settings. That only works if the value is identical across runs, builds
    /// and platforms, so nothing here depends on `HashSet` iteration order or
    /// on `std`'s randomly-keyed hasher: every set is sorted (and titles
    /// deduplicated, since they are OR-ed) before it is hashed, strings are
    /// length-prefixed so `["ab","c"]` and `["a","bc"]` differ, and `None`
    /// ("no constraint") is distinct from `Some(empty)` ("match nothing").
    ///
    /// The salt's number goes up whenever the same filter can select
    /// different lines (the name grammar or the matching changed), so every
    /// index on a card is rebuilt once after such an update rather than
    /// trusted: `/2` is the closed platform list and season matching.
    pub fn fingerprint(&self) -> u64 {
        let mut h = Fnv1a::new();
        h.bytes(b"opendmd-filter/2");
        h.opt_set(self.platforms.as_ref().map(sorted));
        h.opt_set(self.categories_include.as_ref().map(sorted));
        h.opt_set(Some(sorted(&self.categories_exclude)));
        h.opt_set(self.titles.as_ref().map(|t| {
            let mut v: Vec<&str> = t.iter().map(String::as_str).collect();
            v.sort_unstable();
            v.dedup();
            v
        }));
        h.opt_set(Some(sorted(&self.exclude_flags)));
        h.finish()
    }

    /// Indices (into `entries`) of everything that survives, in manifest order.
    pub fn apply(&self, entries: &[Entry]) -> Vec<usize> {
        entries
            .iter()
            .enumerate()
            .filter(|(_, e)| self.matches(e))
            .map(|(i, _)| i)
            .collect()
    }
}

fn sorted(set: &HashSet<String>) -> Vec<&str> {
    let mut v: Vec<&str> = set.iter().map(String::as_str).collect();
    v.sort_unstable();
    v
}

/// 64-bit FNV-1a. Hand-rolled because the value is persisted on the card:
/// `std`'s `DefaultHasher` is explicitly allowed to change between releases.
struct Fnv1a(u64);

impl Fnv1a {
    fn new() -> Self {
        Fnv1a(0xcbf2_9ce4_8422_2325)
    }

    fn bytes(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.0 ^= u64::from(*b);
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }

    fn u32(&mut self, v: u32) {
        self.bytes(&v.to_le_bytes());
    }

    /// One field: a presence byte, then a count and each length-prefixed item.
    fn opt_set(&mut self, items: Option<Vec<&str>>) {
        match items {
            None => self.bytes(&[0]),
            Some(items) => {
                self.bytes(&[1]);
                self.u32(items.len() as u32);
                for item in items {
                    self.u32(item.len() as u32);
                    self.bytes(item.as_bytes());
                }
            }
        }
    }

    fn finish(&self) -> u64 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::parse_manifest;

    fn fixture() -> Vec<Entry> {
        parse_manifest(
            "/gifs/Consoles/SNES_Donkey_Kong_Country_05.gif\n\
             /gifs/Consoles/MD_Sonic3_RattenJager.gif\n\
             /gifs/Consoles/NES_Unknow_01_Invert.gif\n\
             /gifs/Arcade/ARCADE_Pac-Man_01.gif\n\
             /gifs/XMAS/XMAS_Tree_02.gif",
        )
    }

    fn paths(entries: &[Entry], idx: &[usize]) -> Vec<String> {
        idx.iter().map(|i| entries[*i].path.clone()).collect()
    }

    #[test]
    fn default_filter_keeps_everything() {
        let e = fixture();
        assert_eq!(Filter::new().apply(&e).len(), 5);
    }

    #[test]
    fn platforms() {
        let e = fixture();
        let idx = Filter::new().platforms(["md", "SNES"]).apply(&e);
        assert_eq!(
            paths(&e, &idx),
            vec![
                "/gifs/Consoles/SNES_Donkey_Kong_Country_05.gif",
                "/gifs/Consoles/MD_Sonic3_RattenJager.gif"
            ]
        );
    }

    #[test]
    fn title_substring_uses_normalization() {
        let e = fixture();
        let idx = Filter::new().title("donkeykong").apply(&e);
        assert_eq!(
            paths(&e, &idx),
            vec!["/gifs/Consoles/SNES_Donkey_Kong_Country_05.gif"]
        );
        // Spaces and case in the user's query don't matter either.
        let idx = Filter::new().title("Pac Man").apply(&e);
        assert_eq!(paths(&e, &idx), vec!["/gifs/Arcade/ARCADE_Pac-Man_01.gif"]);
        // Repeatable, OR-ed.
        let idx = Filter::new().title("donkeykong").title("sonic").apply(&e);
        assert_eq!(idx.len(), 2);
    }

    #[test]
    fn exclude_invert() {
        let e = fixture();
        let idx = Filter::new().exclude_flag("INVERT").apply(&e);
        assert_eq!(idx.len(), 4);
        assert!(!paths(&e, &idx).iter().any(|p| p.contains("Unknow")));
    }

    #[test]
    fn categories_include_and_exclude() {
        let e = fixture();
        let idx = Filter::new().include_categories(["consoles"]).apply(&e);
        assert_eq!(idx.len(), 3);
        let idx = Filter::new()
            .exclude_categories(["XMAS", "Arcade"])
            .apply(&e);
        assert_eq!(idx.len(), 3);
    }

    #[test]
    fn empty_platform_set_matches_nothing() {
        let e = fixture();
        assert!(Filter::new()
            .platforms(Vec::<String>::new())
            .apply(&e)
            .is_empty());
    }

    #[test]
    fn a_season_filter_keeps_files_that_have_their_own_platform() {
        let psx = Entry::from_path("/gifs/Halloween/HALLOWEEN_PSX_Castlevania_01.gif");
        let own = Entry::from_path("/gifs/Halloween/HALLOWEEN_Dracula_01.gif");
        let xmas = Entry::from_path("/gifs/XMAS/XMAS_SNES_Santa_01.gif");
        let plain = Entry::from_path("/gifs/Consoles/PSX_Castlevania_01.gif");
        // An author flag that happens to read like a platform is not a season.
        let by = Entry::from_path("/gifs/Consoles/MD_Sonic_01_snes.gif");
        assert_eq!(psx.platform.as_deref(), Some("PSX"));
        let hw = Filter::new().platforms(["halloween"]);
        assert!(hw.matches(&psx) && hw.matches(&own));
        assert!(!hw.matches(&xmas) && !hw.matches(&plain));
        let x = Filter::new().platforms(["XMAS"]);
        assert!(x.matches(&xmas) && !x.matches(&psx));
        // The file's own platform still matches it.
        let p = Filter::new().platforms(["PSX"]);
        assert!(p.matches(&psx) && p.matches(&plain) && !p.matches(&own));
        assert!(!Filter::new().platforms(["SNES"]).matches(&by));
    }

    #[test]
    fn fingerprint_ignores_construction_order() {
        let a = Filter::new()
            .platforms(["MD", "SNES", "NES", "GBA", "PCE"])
            .include_categories(["Consoles", "Arcade"])
            .exclude_categories(["XMAS", "Halloween"])
            .title("sonic")
            .title("Donkey Kong")
            .exclude_flags(["INVERT", "LOGO", "INTRO"]);
        let b = Filter::new()
            .exclude_flags(["intro", "logo"])
            .exclude_flag("invert")
            .title("donkeykong")
            .title("Sonic")
            .title("sonic")
            .exclude_categories(["halloween"])
            .exclude_categories(["xmas"])
            .include_categories(["arcade", "consoles"])
            .platforms(["pce", "gba", "nes", "snes", "md"]);
        assert_eq!(a.fingerprint(), b.fingerprint());
        // Stable across calls, not just across equal values.
        assert_eq!(a.fingerprint(), a.clone().fingerprint());
        assert_eq!(Filter::new().fingerprint(), Filter::default().fingerprint());
    }

    #[test]
    fn fingerprint_changes_with_every_field() {
        let base = Filter::new()
            .platforms(["MD", "SNES"])
            .include_categories(["Consoles"])
            .exclude_categories(["XMAS"])
            .title("sonic")
            .exclude_flag("INVERT");
        let variants = [
            Filter::new(),
            Filter::new().platforms(Vec::<String>::new()),
            base.clone().platforms(["MD"]),
            base.clone().platforms(["MD", "SNES", "NES"]),
            base.clone().include_categories(["Arcade"]),
            {
                let mut f = base.clone();
                f.categories_include = None;
                f
            },
            base.clone().exclude_categories(["Halloween"]),
            base.clone().title("mario"),
            {
                let mut f = base.clone();
                f.titles = None;
                f
            },
            base.clone().exclude_flag("LOGO"),
        ];
        let mut seen = vec![base.fingerprint()];
        for v in &variants {
            let fp = v.fingerprint();
            assert!(!seen.contains(&fp), "collision for {v:?}");
            seen.push(fp);
        }
    }

    #[test]
    fn fingerprint_separates_items() {
        // Length prefixes: ["AB","C"] is not ["A","BC"], and a set in one field
        // is not the same set in another.
        let a = Filter::new().platforms(["AB", "C"]);
        let b = Filter::new().platforms(["A", "BC"]);
        assert_ne!(a.fingerprint(), b.fingerprint());
        let c = Filter::new().exclude_categories(["MD"]);
        let d = Filter::new().exclude_flag("MD");
        assert_ne!(c.fingerprint(), d.fingerprint());
    }
}

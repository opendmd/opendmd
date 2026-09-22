//! Selecting a subset of the manifest.
//!
//! All string matching here is ASCII case-insensitive; titles additionally go
//! through [`normalize_title`](crate::manifest::normalize_title), so a user can
//! type `donkeykong` and match `Donkey Kong Country`.

use crate::manifest::{normalize_title, Entry};
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
    /// Uppercased platform names; an entry with no platform never matches.
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
    pub fn matches(&self, entry: &Entry) -> bool {
        if let Some(wanted) = &self.platforms {
            match &entry.platform {
                Some(p) if wanted.contains(&p.to_uppercase()) => {}
                _ => return false,
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
}

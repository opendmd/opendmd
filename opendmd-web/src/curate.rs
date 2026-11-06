//! Curation logic behind the page's Library, Sets and Convert views, with no
//! browser types in it so it is tested natively as well as under node.
//!
//! Everything here is a thin layer over `opendmd-core` — the tag parser,
//! [`Filter`] and the scheduler's [`Rng`] — so the browser selects and
//! orders files exactly the way the firmware does.

use opendmd_core::filter::Filter;
use opendmd_core::manifest::Entry;
use opendmd_core::scheduler::Rng;

/// What the Sets view's "build from filters" form produces. Empty lists mean
/// "no constraint" here — the page's reading of an empty chip row — rather
/// than [`Filter`]'s "match nothing" for an empty `Some(set)`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FilterSpec {
    pub platforms: Vec<String>,
    pub categories: Vec<String>,
    pub exclude_categories: Vec<String>,
    /// OR-ed title substrings, normalised by the core.
    pub titles: Vec<String>,
    pub exclude_flags: Vec<String>,
    /// Fisher-Yates over the matches with the scheduler's splitmix64.
    pub shuffle: bool,
    pub seed: u64,
    /// Keep at most this many (after shuffling).
    pub limit: Option<usize>,
}

impl FilterSpec {
    pub fn filter(&self) -> Filter {
        let mut f = Filter::new();
        if !self.platforms.is_empty() {
            f = f.platforms(&self.platforms);
        }
        if !self.categories.is_empty() {
            f = f.include_categories(&self.categories);
        }
        if !self.exclude_categories.is_empty() {
            f = f.exclude_categories(&self.exclude_categories);
        }
        for t in self.titles.iter().filter(|t| !t.trim().is_empty()) {
            f = f.title(t);
        }
        if !self.exclude_flags.is_empty() {
            f = f.exclude_flags(&self.exclude_flags);
        }
        f
    }
}

/// The non-blank lines of a manifest, tolerating a BOM, CRLF and trailing
/// whitespace exactly as `parse_manifest` does.
pub fn manifest_lines(text: &str) -> impl Iterator<Item = &str> {
    text.strip_prefix('\u{feff}')
        .unwrap_or(text)
        .split('\n')
        .map(|l| l.trim_end_matches(['\r', ' ', '\t']))
        .filter(|l| !l.is_empty())
}

/// The manifest paths that pass `spec`, in manifest order unless shuffled.
pub fn filter_paths(text: &str, spec: &FilterSpec) -> Vec<String> {
    let filter = spec.filter();
    let mut out: Vec<String> = manifest_lines(text)
        .filter(|p| filter.matches(&Entry::from_path(p)))
        .map(str::to_string)
        .collect();
    if spec.shuffle {
        shuffle(&mut out, spec.seed);
    }
    if let Some(n) = spec.limit {
        out.truncate(n);
    }
    out
}

/// Fisher-Yates with the scheduler's generator, so a seed means the same
/// order everywhere.
pub fn shuffle<T>(items: &mut [T], seed: u64) {
    let mut rng = Rng::new(seed);
    for i in (1..items.len()).rev() {
        let j = (rng.next_u64() % (i as u64 + 1)) as usize;
        items.swap(i, j);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    const MANIFEST: &str = "\u{feff}/gifs/Consoles/SNES_Plok_08.gif\r\n\
        /gifs/Consoles/MD_Sonic3_spritefox11.gif\n\
        \n\
        /gifs/Arcade/ARCADE_Pac-Man_01.gif\n\
        /gifs/NEW 2021/SNES_Donkey_Kong_Country_04.gif\n\
        /gifs/Halloween/HALLOWEEN_Dracula_01.gif  \n";

    #[test]
    fn filters_by_platform_category_and_title() {
        let spec = FilterSpec {
            platforms: vec!["md".into(), "SNES".into()],
            ..Default::default()
        };
        assert_eq!(
            filter_paths(MANIFEST, &spec),
            [
                "/gifs/Consoles/SNES_Plok_08.gif",
                "/gifs/Consoles/MD_Sonic3_spritefox11.gif",
                "/gifs/NEW 2021/SNES_Donkey_Kong_Country_04.gif",
            ]
        );
        let spec = FilterSpec {
            categories: vec!["new 2021".into(), "Halloween".into()],
            titles: vec!["donkey kong".into()],
            ..Default::default()
        };
        assert_eq!(
            filter_paths(MANIFEST, &spec),
            ["/gifs/NEW 2021/SNES_Donkey_Kong_Country_04.gif"]
        );
        // Empty lists are no constraint at all.
        assert_eq!(filter_paths(MANIFEST, &FilterSpec::default()).len(), 5);
        let spec = FilterSpec {
            exclude_categories: vec!["Consoles".into()],
            ..Default::default()
        };
        assert_eq!(filter_paths(MANIFEST, &spec).len(), 3);
    }

    #[test]
    fn shuffle_is_a_seeded_permutation() {
        let spec = FilterSpec {
            shuffle: true,
            seed: 7,
            ..Default::default()
        };
        let a = filter_paths(MANIFEST, &spec);
        let b = filter_paths(MANIFEST, &spec);
        assert_eq!(a, b, "same seed, same order");
        let mut sorted_a = a.clone();
        sorted_a.sort();
        let mut plain = filter_paths(MANIFEST, &FilterSpec::default());
        plain.sort();
        assert_eq!(sorted_a, plain, "a permutation, nothing lost");
        let limited = filter_paths(
            MANIFEST,
            &FilterSpec {
                limit: Some(2),
                ..spec
            },
        );
        assert_eq!(limited, a[..2]);
    }
}

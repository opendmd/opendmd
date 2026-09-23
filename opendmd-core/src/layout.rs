//! Where a converted `.odm` file lives on the card: the one implementation of
//! the bucket hash, shared by `opendmd-pack` (desktop) and `opendmd-web`
//! (the browser converter), so the two can never disagree about a path.
//!
//! The **bucketed** layout is `odm/<Category>/<bb>/<name>.odm`, where `<bb>`
//! is [`bucket`] of the `.odm` file name. `<Category>` is the GIF's directory
//! relative to `gifs/`, taken verbatim (spaces, underscores and, for a GIF
//! nested deeper, the `/`-separated path of every directory); a GIF directly
//! in `gifs/` gets no category directory: `odm/<bb>/<name>.odm`. The
//! **beside** layout is `gifs/<Category>/<name>.odm` next to the GIF.
//!
//! Bucketing exists because a FAT directory lookup on the ESP32 is a linear
//! scan (~0.65 ms per 512 B directory sector): `gifs/Consoles` holds 3,781
//! files and costs ~330 ms per open, while 256 buckets leave ~15 files each.
//! See "Card layout" in `docs/odm-format.md`.

/// FNV-1a, 32-bit: offset basis `0x811c9dc5`, prime `0x01000193`; per byte
/// `h ^= b; h *= prime` (wrapping).
pub fn fnv1a32(bytes: &[u8]) -> u32 {
    bytes.iter().fold(0x811c_9dc5, |h: u32, &b| {
        (h ^ u32::from(b)).wrapping_mul(0x0100_0193)
    })
}

/// The bucket directory for an `.odm` file: the low byte of [`fnv1a32`] over
/// the **`.odm` file name** (UTF-8 bytes, extension included, no directories
/// — e.g. `MD_Sonic3_RattenJager.odm`), as two lowercase hex digits.
pub fn bucket(odm_name: &str) -> String {
    format!("{:02x}", fnv1a32(odm_name.as_bytes()) & 0xff)
}

/// The `.odm` file name for a GIF file name: the final `.gif` (any case)
/// replaced by `.odm`, exactly as `Path::with_extension("odm")` does, so
/// `X_01..gif` becomes `X_01..odm`. `None` if the name does not end in
/// `.gif` or is nothing but the extension.
pub fn odm_name(gif_name: &str) -> Option<String> {
    let cut = gif_name.len().checked_sub(4)?;
    if cut == 0 || !gif_name.get(cut..)?.eq_ignore_ascii_case(".gif") {
        return None;
    }
    Some(format!("{}.odm", &gif_name[..cut]))
}

/// Map a GIF's path relative to `gifs/` (`/`-separated, e.g.
/// `Consoles/MD_Sonic3_RattenJager.gif`) to its `.odm` path relative to the
/// card root, no leading slash: `odm/<Category>/<bb>/<name>.odm` when
/// `bucketed`, else `gifs/<Category>/<name>.odm`. `None` if the name does not
/// end in `.gif`.
pub fn odm_rel(gif_rel: &str, bucketed: bool) -> Option<String> {
    let gif_rel = gif_rel.trim_start_matches('/');
    let (dir, name) = match gif_rel.rsplit_once('/') {
        Some((d, n)) => (d, n),
        None => ("", gif_rel),
    };
    let odm = odm_name(name)?;
    let dir = dir.trim_end_matches('/');
    Some(match (bucketed, dir.is_empty()) {
        (false, true) => format!("gifs/{odm}"),
        (false, false) => format!("gifs/{dir}/{odm}"),
        (true, true) => format!("odm/{}/{odm}", bucket(&odm)),
        (true, false) => format!("odm/{dir}/{}/{odm}", bucket(&odm)),
    })
}

/// The bucketed `.odm` path for a card-absolute manifest path:
/// `/gifs/Consoles/X.gif` -> `/odm/Consoles/<bb>/X.odm`. `None` unless the
/// path is a `.gif` under `/gifs/` (the first directory, any case).
pub fn bucketed_path(manifest_path: &str) -> Option<String> {
    let (first, rest) = manifest_path.trim_start_matches('/').split_once('/')?;
    if !first.eq_ignore_ascii_case("gifs") {
        return None;
    }
    Some(format!("/{}", odm_rel(rest, true)?))
}

/// The key the cache count matches an optimised copy by: FNV-1a-32 over the
/// ASCII-lowercased card-absolute bucketed path (`/odm/Consoles/90/X.odm`).
/// Lowercased because FAT names compare case-insensitively, so the name a
/// directory walk returns may differ in case from the manifest's.
pub fn prepared_key(odm_path: &str) -> u32 {
    odm_path.bytes().fold(0x811c_9dc5, |h: u32, b| {
        (h ^ u32::from(b.to_ascii_lowercase())).wrapping_mul(0x0100_0193)
    })
}

/// What a manifest line means for the playback cache.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Prepared {
    /// A `/gifs/….gif` path: its optimised copy would be at
    /// [`bucketed_path`], in bucket `bucket`, matched by `key`
    /// ([`prepared_key`]).
    Candidate { bucket: u8, key: u32 },
    /// A path already naming an optimised copy (`/odm/…`), from a manifest
    /// written before copies were resolved automatically. Counted as prepared.
    Direct,
    /// Anything else (not under `/gifs/`, not a `.gif`): never prepared.
    Never,
}

/// Classify one manifest path. See [`Prepared`].
pub fn classify(manifest_path: &str) -> Prepared {
    let first = manifest_path
        .trim_start_matches('/')
        .split('/')
        .next()
        .unwrap_or("");
    if first.eq_ignore_ascii_case("odm") {
        return Prepared::Direct;
    }
    match bucketed_path(manifest_path) {
        Some(odm) => {
            let name = odm.rsplit('/').next().unwrap_or(&odm);
            Prepared::Candidate {
                bucket: (fnv1a32(name.as_bytes()) & 0xff) as u8,
                key: prepared_key(&odm),
            }
        }
        None => Prepared::Never,
    }
}

/// Totals from [`count_prepared`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CacheCount {
    /// Entries that pass the filter.
    pub total: usize,
    /// Of those, the ones with an optimised copy: a [`Prepared::Candidate`]
    /// whose bucket is in the range and whose key is in `keys`, plus (when
    /// `count_direct`) every [`Prepared::Direct`] entry.
    pub prepared: usize,
}

/// One streaming pass over a manifest counting the entries that pass
/// `filter` and, of those, the ones whose optimised copy is known to exist.
///
/// `keys` is **sorted**: the [`prepared_key`]s of the copies a directory walk
/// found in buckets `buckets.0 ..= buckets.1`. The range exists so a caller
/// with little memory can walk the buckets a slice at a time and call this
/// once per slice, summing `prepared` (and taking `total` from any one call;
/// pass `count_direct` on exactly one of them).
///
/// Lines are only parsed into an [`Entry`](crate::manifest::Entry) (the tag
/// grammar, most of the cost on the ESP32: ~12.7 s of a 10k-line pass there)
/// when the filter needs one; unfiltered, a line is just a path. Tolerance is
/// [`crate::manifest::scan`]'s: BOM, CRLF, blank lines, trailing whitespace.
pub fn count_prepared<R: std::io::BufRead>(
    mut reader: R,
    filter: &crate::filter::Filter,
    buckets: (u8, u8),
    keys: &[u32],
    count_direct: bool,
) -> std::io::Result<CacheCount> {
    let parse = !filter.is_unconstrained();
    let mut c = CacheCount::default();
    let mut raw = String::new();
    loop {
        raw.clear();
        if reader.read_line(&mut raw)? == 0 {
            return Ok(c);
        }
        let line = raw
            .strip_prefix('\u{feff}')
            .unwrap_or(&raw)
            .trim_end_matches(['\n', '\r', ' ', '\t']);
        if line.is_empty() {
            continue;
        }
        if parse && !filter.matches(&crate::manifest::Entry::from_path(line)) {
            continue;
        }
        c.total += 1;
        match classify(line) {
            Prepared::Candidate { bucket, key } => {
                if (buckets.0..=buckets.1).contains(&bucket) && keys.binary_search(&key).is_ok() {
                    c.prepared += 1;
                }
            }
            Prepared::Direct => c.prepared += usize::from(count_direct),
            Prepared::Never => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pinned: these values are baked into every bucketed card in the wild,
    /// so a change here is a format break, not a refactor.
    #[test]
    fn fnv1a32_and_bucket_known_values() {
        // Standard FNV-1a 32 test vectors.
        assert_eq!(fnv1a32(b""), 0x811c_9dc5);
        assert_eq!(fnv1a32(b"a"), 0xe40c_292c);
        assert_eq!(fnv1a32(b"foobar"), 0xbf9c_f968);
        // The worked example in docs/odm-format.md.
        assert_eq!(fnv1a32(b"MD_Sonic3_RattenJager.odm"), 0xd01e_f790);
        assert_eq!(bucket("MD_Sonic3_RattenJager.odm"), "90");
        assert_eq!(fnv1a32(b"SNES_Plok_08.odm"), 0x4ee4_8ec0);
        assert_eq!(bucket("SNES_Plok_08.odm"), "c0");
        // Three paths from the stock card's filelist.odm.txt, as written by
        // `opendmd-pack manifest --layout bucketed`.
        assert_eq!(bucket("Exolon2.odm"), "20");
        assert_eq!(bucket("Abadia_Crimen.odm"), "e2");
        assert_eq!(bucket("Viaje_Centro_Tierra.odm"), "46");
    }

    #[test]
    fn paths_per_layout() {
        let g = "Consoles/MD_Sonic3_RattenJager.gif";
        assert_eq!(
            odm_rel(g, false).unwrap(),
            "gifs/Consoles/MD_Sonic3_RattenJager.odm"
        );
        assert_eq!(
            odm_rel(g, true).unwrap(),
            "odm/Consoles/90/MD_Sonic3_RattenJager.odm"
        );
        // Category kept verbatim, stray dots kept, extension case ignored.
        let b = odm_rel("NEW 2021/SNES_A_01..GIF", true).unwrap();
        assert_eq!(
            b,
            format!("odm/NEW 2021/{}/SNES_A_01..odm", bucket("SNES_A_01..odm"))
        );
        // Nested: every directory is the category; loose: none.
        assert_eq!(
            odm_rel("A/B c/x.gif", true).unwrap(),
            format!("odm/A/B c/{}/x.odm", bucket("x.odm"))
        );
        assert_eq!(
            odm_rel("x.gif", true).unwrap(),
            format!("odm/{}/x.odm", bucket("x.odm"))
        );
        assert_eq!(odm_rel("x.gif", false).unwrap(), "gifs/x.odm");
        assert_eq!(odm_rel("Consoles/x.png", true), None);
        assert_eq!(odm_rel("Consoles/.gif", true), None);
    }

    #[test]
    fn manifest_paths_map_to_bucketed_paths() {
        assert_eq!(
            bucketed_path("/gifs/Spectrum_Slide/Exolon2.gif").unwrap(),
            "/odm/Spectrum_Slide/20/Exolon2.odm"
        );
        assert_eq!(
            bucketed_path("/gifs/Consoles/MD_Sonic3_RattenJager.gif").unwrap(),
            "/odm/Consoles/90/MD_Sonic3_RattenJager.odm"
        );
        assert_eq!(bucketed_path("/odm/Consoles/90/X.odm"), None);
        assert_eq!(bucketed_path("/gifs/Consoles/X.odm"), None);
        assert_eq!(bucketed_path("X.gif"), None);
    }

    #[test]
    fn classify_and_count_prepared() {
        use crate::filter::Filter;
        let a = "/gifs/Consoles/MD_Sonic3_RattenJager.gif";
        let b = "/gifs/NEW 2021/SNES_Plok_08.gif";
        let Prepared::Candidate { bucket, key } = classify(a) else {
            panic!("a GIF under gifs/ is a candidate")
        };
        assert_eq!(bucket, 0x90);
        assert_eq!(
            key,
            prepared_key("/odm/Consoles/90/MD_Sonic3_RattenJager.odm")
        );
        // Case never matters: FAT hands back whatever case is on the card.
        assert_eq!(
            key,
            prepared_key("/ODM/consoles/90/md_sonic3_rattenjager.ODM")
        );
        assert_eq!(classify("/odm/Consoles/90/X.odm"), Prepared::Direct);
        assert_eq!(classify("/sets/x.txt"), Prepared::Never);
        assert_eq!(classify("/gifs/Consoles/x.png"), Prepared::Never);

        let manifest =
            format!("{a}\n{b}\n/odm/Consoles/90/X.odm\n/gifs/Consoles/SNES_Other_01.gif\n");
        let mut keys = vec![prepared_key(&bucketed_path(a).unwrap())];
        keys.sort_unstable();
        let all = Filter::new();
        let c = count_prepared(manifest.as_bytes(), &all, (0, 255), &keys, true).unwrap();
        assert_eq!(
            c,
            CacheCount {
                total: 4,
                prepared: 2
            }
        );
        // The bucket slice excludes a's bucket: only the direct entry counts.
        let c = count_prepared(manifest.as_bytes(), &all, (0, 0x8f), &keys, true).unwrap();
        assert_eq!(
            c,
            CacheCount {
                total: 4,
                prepared: 1
            }
        );
        let c = count_prepared(manifest.as_bytes(), &all, (0x90, 0x90), &keys, false).unwrap();
        assert_eq!(
            c,
            CacheCount {
                total: 4,
                prepared: 1
            }
        );
        // The filter applies to both totals.
        let snes = Filter::new().platforms(["SNES"]);
        let c = count_prepared(manifest.as_bytes(), &snes, (0, 255), &keys, true).unwrap();
        assert_eq!(
            c,
            CacheCount {
                total: 2,
                prepared: 0
            }
        );
    }
}

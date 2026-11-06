//! Where a GIF's prepared copy lives on the card: the one implementation of
//! the bucket rule, shared by `opendmd-pack` (desktop), `opendmd-web` (the
//! browser) and the firmware (which resolves and counts by it), so they can
//! never disagree about a path.
//!
//! The layout is `cache/<Category>/<xx>/<name>.gif`, where `<xx>` is
//! [`bucket`] of the file name. `<Category>` is the GIF's directory relative
//! to `gifs/`, taken verbatim (spaces, underscores and, for a GIF nested
//! deeper, the `/`-separated path of every directory); a GIF directly in
//! `gifs/` gets no category directory: `cache/<xx>/<name>.gif`. The copy
//! keeps the GIF's file name.
//!
//! Bucketing exists because a FAT directory lookup on the ESP32 is a linear
//! scan (~0.65 ms per 512 B directory sector): `gifs/Consoles` holds 3,781
//! files and costs ~330 ms per open, while 256 buckets leave ~15 files each.
//! See `docs/cache-format.md`.

/// FNV-1a, 32-bit: offset basis `0x811c9dc5`, prime `0x01000193`; per byte
/// `h ^= b; h *= prime` (wrapping).
pub fn fnv1a32(bytes: &[u8]) -> u32 {
    bytes.iter().fold(0x811c_9dc5, |h: u32, &b| {
        (h ^ u32::from(b)).wrapping_mul(0x0100_0193)
    })
}

/// The bucket directory for a prepared copy: the low byte of [`fnv1a32`]
/// over its **file name** (UTF-8 bytes, extension included, no directories
/// — e.g. `MD_Sonic3_spritefox11.gif`), as two lowercase hex digits.
pub fn bucket(name: &str) -> String {
    format!("{:02x}", fnv1a32(name.as_bytes()) & 0xff)
}

/// Is `name` a GIF file name (ends in `.gif`, any case, and is more than
/// the extension)?
fn is_gif_name(name: &str) -> bool {
    let Some(cut) = name.len().checked_sub(4) else {
        return false;
    };
    cut > 0
        && name
            .get(cut..)
            .is_some_and(|e| e.eq_ignore_ascii_case(".gif"))
}

/// Map a GIF's path relative to `gifs/` (`/`-separated, e.g.
/// `Consoles/MD_Sonic3_spritefox11.gif`) to its prepared copy's path
/// relative to the card root, no leading slash:
/// `cache/<Category>/<xx>/<name>.gif`. `None` if the name does not end in
/// `.gif`.
pub fn cache_rel(gif_rel: &str) -> Option<String> {
    let gif_rel = gif_rel.trim_start_matches('/');
    let (dir, name) = match gif_rel.rsplit_once('/') {
        Some((d, n)) => (d, n),
        None => ("", gif_rel),
    };
    if !is_gif_name(name) {
        return None;
    }
    let dir = dir.trim_end_matches('/');
    Some(if dir.is_empty() {
        format!("cache/{}/{name}", bucket(name))
    } else {
        format!("cache/{dir}/{}/{name}", bucket(name))
    })
}

/// The prepared copy's path for a card-absolute manifest path:
/// `/gifs/Consoles/X.gif` -> `/cache/Consoles/<xx>/X.gif`. `None` unless
/// the path is a `.gif` under `/gifs/` (the first directory, any case).
pub fn cache_path(manifest_path: &str) -> Option<String> {
    let (first, rest) = manifest_path.trim_start_matches('/').split_once('/')?;
    if !first.eq_ignore_ascii_case("gifs") {
        return None;
    }
    Some(format!("/{}", cache_rel(rest)?))
}

/// The key the cache count matches a prepared copy by: FNV-1a-32 over the
/// ASCII-lowercased card-absolute copy path (`/cache/Consoles/38/X.gif`).
/// Lowercased because FAT names compare case-insensitively, so the name a
/// directory walk returns may differ in case from the manifest's.
pub fn prepared_key(copy_path: &str) -> u32 {
    copy_path.bytes().fold(0x811c_9dc5, |h: u32, b| {
        (h ^ u32::from(b.to_ascii_lowercase())).wrapping_mul(0x0100_0193)
    })
}

/// What a manifest line means for the playback cache.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Prepared {
    /// A `/gifs/….gif` path: its prepared copy would be at [`cache_path`],
    /// in bucket `bucket`, matched by `key` ([`prepared_key`]).
    Candidate { bucket: u8, key: u32 },
    /// A path already naming a prepared copy (`/cache/…`). Counted as
    /// prepared.
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
    if first.eq_ignore_ascii_case("cache") {
        return Prepared::Direct;
    }
    match cache_path(manifest_path) {
        Some(copy) => {
            let name = copy.rsplit('/').next().unwrap_or(&copy);
            Prepared::Candidate {
                bucket: (fnv1a32(name.as_bytes()) & 0xff) as u8,
                key: prepared_key(&copy),
            }
        }
        None => Prepared::Never,
    }
}

/// One `/cache/.failed` line (`path<TAB>reason`): if it says the GIF plays
/// as it is ([`crate::gifslim::plays_original`]: no copy on purpose), the
/// bucket and [`prepared_key`] its copy would have, which the device's
/// count takes as prepared. `None` for any other line.
pub fn plays_original_line(line: &str) -> Option<(u8, u32)> {
    let (path, reason) = line.split_once('\t')?;
    if !crate::gifslim::plays_original(reason) {
        return None;
    }
    match classify(path.trim()) {
        Prepared::Candidate { bucket, key } => Some((bucket, key)),
        _ => None,
    }
}

/// Totals from [`count_prepared`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CacheCount {
    /// Entries that pass the filter.
    pub total: usize,
    /// Of those, the ones with a prepared copy: a [`Prepared::Candidate`]
    /// whose bucket is in the range and whose key is in `keys`, plus (when
    /// `count_direct`) every [`Prepared::Direct`] entry.
    pub prepared: usize,
}

/// One streaming pass over a manifest counting the entries that pass
/// `filter` and, of those, the ones whose prepared copy is known to exist.
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
        // A retired effect's or live item's line is skipped, as `manifest::scan` does
        // (which logs it).
        if line.is_empty()
            || crate::synth::is_retired_effect(line)
            || crate::synth::is_retired_live(line)
            || crate::manifest::skips_formula(line)
        {
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

    #[test]
    fn a_plays_as_is_line_counts_as_its_copy() {
        let gif = "/gifs/Consoles/MD_Sonic3_spritefox11.gif";
        let (bucket, key) = (0x8f, prepared_key(&cache_path(gif).unwrap()));
        let want = Some((bucket, key));
        assert_eq!(
            plays_original_line(&format!("{gif}\tmore than 256 colours: plays as it is")),
            want
        );
        // Earlier firmware's words, and the whitespace a hand edit leaves.
        assert_eq!(
            plays_original_line(&format!(" {gif} \tmore than 256 colours\r")),
            want
        );
        // So a manifest line for that GIF counts as prepared.
        let c = count_prepared(
            format!("{gif}\n/gifs/Consoles/Other.gif\n").as_bytes(),
            &crate::filter::Filter::new(),
            (bucket, bucket),
            &[key],
            true,
        )
        .unwrap();
        assert_eq!((c.total, c.prepared), (2, 1));
        // Anything else is not.
        assert_eq!(plays_original_line(&format!("{gif}\tinvalid LZW")), None);
        assert_eq!(plays_original_line(gif), None);
        assert_eq!(
            plays_original_line("/logos/x.png\tmore than 256 colours"),
            None
        );
    }

    /// Pinned: these values are baked into every prepared card in the wild,
    /// so a change here is a format break, not a refactor.
    #[test]
    fn fnv1a32_and_bucket_known_values() {
        // Standard FNV-1a 32 test vectors.
        assert_eq!(fnv1a32(b""), 0x811c_9dc5);
        assert_eq!(fnv1a32(b"a"), 0xe40c_292c);
        assert_eq!(fnv1a32(b"foobar"), 0xbf9c_f968);
        // The worked example in docs/cache-format.md.
        assert_eq!(fnv1a32(b"MD_Sonic3_spritefox11.gif"), 0x0edd_ec8f);
        assert_eq!(bucket("MD_Sonic3_spritefox11.gif"), "8f");
        assert_eq!(fnv1a32(b"SNES_Plok_08.gif"), 0x10b6_d368);
        assert_eq!(bucket("SNES_Plok_08.gif"), "68");
        assert_eq!(bucket("Exolon2.gif"), "c8");
        assert_eq!(bucket("Abadia_Crimen.gif"), "06");
        assert_eq!(bucket("Viaje_Centro_Tierra.gif"), "c2");
    }

    #[test]
    fn cache_paths() {
        assert_eq!(
            cache_rel("Consoles/MD_Sonic3_spritefox11.gif").unwrap(),
            "cache/Consoles/8f/MD_Sonic3_spritefox11.gif"
        );
        // Category kept verbatim, stray dots kept, the name as it is.
        assert_eq!(
            cache_rel("NEW 2021/SNES_A_01..GIF").unwrap(),
            format!("cache/NEW 2021/{}/SNES_A_01..GIF", bucket("SNES_A_01..GIF"))
        );
        // Nested: every directory is the category; loose: none.
        assert_eq!(
            cache_rel("A/B c/x.gif").unwrap(),
            format!("cache/A/B c/{}/x.gif", bucket("x.gif"))
        );
        assert_eq!(cache_rel("x.gif").unwrap(), "cache/63/x.gif");
        assert_eq!(cache_rel("Consoles/x.png"), None);
        assert_eq!(cache_rel("Consoles/.gif"), None);
    }

    #[test]
    fn manifest_paths_map_to_cache_paths() {
        assert_eq!(
            cache_path("/gifs/Spectrum_Slide/Exolon2.gif").unwrap(),
            "/cache/Spectrum_Slide/c8/Exolon2.gif"
        );
        assert_eq!(
            cache_path("/gifs/Consoles/MD_Sonic3_spritefox11.gif").unwrap(),
            "/cache/Consoles/8f/MD_Sonic3_spritefox11.gif"
        );
        assert_eq!(cache_path("/cache/Consoles/38/X.gif"), None);
        assert_eq!(cache_path("/gifs/Consoles/X.png"), None);
        assert_eq!(cache_path("X.gif"), None);
    }

    #[test]
    fn classify_and_count_prepared() {
        use crate::filter::Filter;
        let a = "/gifs/Consoles/MD_Sonic3_spritefox11.gif";
        let b = "/gifs/NEW 2021/SNES_Plok_08.gif";
        let Prepared::Candidate { bucket, key } = classify(a) else {
            panic!("a GIF under gifs/ is a candidate")
        };
        assert_eq!(bucket, 0x8f);
        assert_eq!(
            key,
            prepared_key("/cache/Consoles/8f/MD_Sonic3_spritefox11.gif")
        );
        // Case never matters: FAT hands back whatever case is on the card.
        assert_eq!(
            key,
            prepared_key("/CACHE/consoles/8f/md_sonic3_spritefox11.GIF")
        );
        assert_eq!(classify("/cache/Consoles/38/X.gif"), Prepared::Direct);
        assert_eq!(classify("/sets/x.txt"), Prepared::Never);
        assert_eq!(classify("/gifs/Consoles/x.png"), Prepared::Never);

        let manifest =
            format!("{a}\n{b}\n/cache/Consoles/38/X.gif\n/gifs/Consoles/SNES_Other_01.gif\n");
        let mut keys = vec![prepared_key(&cache_path(a).unwrap())];
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
        let c = count_prepared(manifest.as_bytes(), &all, (0, 0x8e), &keys, true).unwrap();
        assert_eq!(
            c,
            CacheCount {
                total: 4,
                prepared: 1
            }
        );
        let c = count_prepared(manifest.as_bytes(), &all, (0x8f, 0x8f), &keys, false).unwrap();
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

//! Where `convert` puts an `.odm` file and which path `manifest` writes for it.
//!
//! Two layouts (see "Card layout" in `docs/odm-format.md`):
//!
//! - **beside** (the default): `gifs/<Category>/<name>.odm` next to
//!   `gifs/<Category>/<name>.gif`.
//! - **bucketed**: `odm/<Category>/<bb>/<name>.odm`, where `<bb>` is
//!   [`bucket`] of the `.odm` file name. `<Category>` is the GIF's directory
//!   relative to `gifs/`, taken verbatim (spaces, underscores and, for a GIF
//!   nested deeper, the `/`-separated path of every directory); a GIF directly
//!   in `gifs/` gets no category directory: `odm/<bb>/<name>.odm`.
//!
//! Bucketing exists because a FAT directory lookup on the ESP32 is a linear
//! scan (~0.65 ms per 512 B directory sector): `gifs/Consoles` holds 3,781
//! files and costs ~330 ms per open, while 256 buckets leave ~15 files each.

use clap::ValueEnum;

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Layout {
    /// `<name>.odm` beside each `.gif` under `gifs/`.
    Beside,
    /// `odm/<Category>/<bb>/<name>.odm`, 256 hash buckets per category.
    Bucketed,
}

/// FNV-1a, 32-bit: offset basis `0x811c9dc5`, prime `0x01000193`; per byte
/// `h ^= b; h *= prime` (wrapping).
pub fn fnv1a32(bytes: &[u8]) -> u32 {
    bytes.iter().fold(0x811c_9dc5, |h: u32, &b| {
        (h ^ u32::from(b)).wrapping_mul(0x0100_0193)
    })
}

/// The bucket directory for an `.odm` file: the low byte of
/// [`fnv1a32`] over the **`.odm` file name** (UTF-8 bytes, extension included,
/// no directories — e.g. `MD_Sonic3_RattenJager.odm`), as two lowercase hex
/// digits.
pub fn bucket(odm_name: &str) -> String {
    format!("{:02x}", fnv1a32(odm_name.as_bytes()) & 0xff)
}

/// Map a GIF's path relative to `gifs/` (`/`-separated, e.g.
/// `Consoles/MD_Sonic3_RattenJager.gif`) to its `.odm` path relative to the
/// card root under `layout` (no leading slash). The `.odm` name is the GIF's
/// name with its final `.gif` (any case) replaced by `.odm`, exactly as
/// `Path::with_extension("odm")` does, so `X_01..gif` becomes `X_01..odm`.
/// `None` if the name does not end in `.gif`.
pub fn odm_rel(gif_rel: &str, layout: Layout) -> Option<String> {
    let gif_rel = gif_rel.trim_start_matches('/');
    let (dir, name) = match gif_rel.rsplit_once('/') {
        Some((d, n)) => (d, n),
        None => ("", gif_rel),
    };
    let cut = name.len().checked_sub(4)?;
    if cut == 0 || !name.get(cut..)?.eq_ignore_ascii_case(".gif") {
        return None;
    }
    let odm_name = format!("{}.odm", &name[..cut]);
    let dir = dir.trim_end_matches('/');
    Some(match layout {
        Layout::Beside if dir.is_empty() => format!("gifs/{odm_name}"),
        Layout::Beside => format!("gifs/{dir}/{odm_name}"),
        Layout::Bucketed if dir.is_empty() => format!("odm/{}/{odm_name}", bucket(&odm_name)),
        Layout::Bucketed => format!("odm/{dir}/{}/{odm_name}", bucket(&odm_name)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fnv1a32_known_values() {
        // Standard FNV-1a 32 test vectors.
        assert_eq!(fnv1a32(b""), 0x811c_9dc5);
        assert_eq!(fnv1a32(b"a"), 0xe40c_292c);
        assert_eq!(fnv1a32(b"foobar"), 0xbf9c_f968);
        // The worked example in docs/odm-format.md.
        assert_eq!(fnv1a32(b"MD_Sonic3_RattenJager.odm"), 0xd01e_f790);
        assert_eq!(bucket("MD_Sonic3_RattenJager.odm"), "90");
        assert_eq!(fnv1a32(b"SNES_Plok_08.odm"), 0x4ee4_8ec0);
        assert_eq!(bucket("SNES_Plok_08.odm"), "c0");
    }

    #[test]
    fn paths_per_layout() {
        let g = "Consoles/MD_Sonic3_RattenJager.gif";
        assert_eq!(
            odm_rel(g, Layout::Beside).unwrap(),
            "gifs/Consoles/MD_Sonic3_RattenJager.odm"
        );
        assert_eq!(
            odm_rel(g, Layout::Bucketed).unwrap(),
            "odm/Consoles/90/MD_Sonic3_RattenJager.odm"
        );
        // Category kept verbatim, stray dots kept, extension case ignored.
        let b = odm_rel("NEW 2021/SNES_A_01..GIF", Layout::Bucketed).unwrap();
        assert_eq!(
            b,
            format!("odm/NEW 2021/{}/SNES_A_01..odm", bucket("SNES_A_01..odm"))
        );
        // Nested: every directory is the category; loose: none.
        assert_eq!(
            odm_rel("A/B c/x.gif", Layout::Bucketed).unwrap(),
            format!("odm/A/B c/{}/x.odm", bucket("x.odm"))
        );
        assert_eq!(
            odm_rel("x.gif", Layout::Bucketed).unwrap(),
            format!("odm/{}/x.odm", bucket("x.odm"))
        );
        assert_eq!(odm_rel("x.gif", Layout::Beside).unwrap(), "gifs/x.odm");
        assert_eq!(odm_rel("Consoles/x.png", Layout::Bucketed), None);
        assert_eq!(odm_rel("Consoles/.gif", Layout::Bucketed), None);
    }
}

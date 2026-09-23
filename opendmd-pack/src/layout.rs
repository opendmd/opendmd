//! Where `convert` puts an `.odm` file and which path `manifest` writes for it.
//!
//! Two layouts (see "Card layout" in `docs/odm-format.md`):
//!
//! - **beside** (the default): `gifs/<Category>/<name>.odm` next to
//!   `gifs/<Category>/<name>.gif`.
//! - **bucketed**: `odm/<Category>/<bb>/<name>.odm`.
//!
//! The hash and the path rules live in `opendmd_core::layout`, the one
//! implementation shared with the browser converter in `opendmd-web`; this
//! module only adds the CLI's `--layout` switch on top.

use clap::ValueEnum;

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Layout {
    /// `<name>.odm` beside each `.gif` under `gifs/`.
    Beside,
    /// `odm/<Category>/<bb>/<name>.odm`, 256 hash buckets per category.
    Bucketed,
}

/// Map a GIF's path relative to `gifs/` to its `.odm` path relative to the
/// card root under `layout` (no leading slash). See
/// [`opendmd_core::layout::odm_rel`].
pub fn odm_rel(gif_rel: &str, layout: Layout) -> Option<String> {
    opendmd_core::layout::odm_rel(gif_rel, layout == Layout::Bucketed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use opendmd_core::layout::bucket;

    #[test]
    fn the_cli_layouts_map_onto_the_core_rules() {
        let g = "Consoles/MD_Sonic3_RattenJager.gif";
        assert_eq!(
            odm_rel(g, Layout::Beside).unwrap(),
            "gifs/Consoles/MD_Sonic3_RattenJager.odm"
        );
        assert_eq!(
            odm_rel(g, Layout::Bucketed).unwrap(),
            "odm/Consoles/90/MD_Sonic3_RattenJager.odm"
        );
        assert_eq!(
            odm_rel("x.gif", Layout::Bucketed).unwrap(),
            format!("odm/{}/x.odm", bucket("x.odm"))
        );
    }
}

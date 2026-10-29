//! The Formulas the firmware plays by reference (CLAUDE.md "Formulas by
//! reference"):
//!
//! - **bundled ones by name**, `@formula:<stem>`: the `-- bundle` files of
//!   `opendmd-web/formulas/`, built into the image by build.rs (comments
//!   and blank lines out, about a third smaller) and installed into the core
//!   at boot ([`install`]). The compiler reads them where they are, in
//!   flash: no heap for the source, no read of the www pack (gzipped, and
//!   only ever read on CPU0) and nothing to inflate;
//! - **the user's on the card**, `/formulas/<stem>.psi`: the engine reads
//!   them through the card source as it opens them
//!   (`opendmd_core::fx_psi::read_text`); a look's is read here
//!   ([`read_card`]).

/// (stem, source) for every bundled Formula, in file-name order.
#[cfg(feature = "psi")]
pub const BUNDLED: &[(&str, &str)] = include!(concat!(env!("OUT_DIR"), "/bundled_formulas.rs"));

/// Make `@formula:<stem>` play the bundled Formulas.
pub fn install() {
    #[cfg(feature = "psi")]
    opendmd_core::synth::set_bundled(BUNDLED);
}

/// A Formula file on the card (`/formulas/x.psi`, card-absolute), compacted
/// as it is read, or why not as the sentence a look shows.
#[cfg(feature = "psi")]
pub fn read_card(file: &str) -> Result<String, opendmd_core::psi::Error> {
    let err = |msg: String| opendmd_core::psi::Error { start: 0, end: 0, msg };
    if crate::files::card_path(crate::config::SD_MOUNT, file).is_none() {
        return Err(err("that Formula's path is not on the card".into()));
    }
    let full = format!("{}{file}", crate::config::SD_MOUNT);
    let f = std::fs::File::open(&full).map_err(|e| {
        err(if e.kind() == std::io::ErrorKind::NotFound {
            "that Formula is not on the card".into()
        } else {
            format!("that Formula could not be read: {e}")
        })
    })?;
    opendmd_core::fx_psi::read_text(std::io::BufReader::with_capacity(512, f))
        .map_err(|e| err(e.to_string()))
}

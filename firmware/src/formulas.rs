//! The Formulas the firmware plays by reference (docs/notes/formulas.md
//! "Formulas by reference"), always compiled (`opendmd_psi::psb`): the
//! player never compiles.
//!
//! - **bundled ones by name**, `@formula:<stem>`: the `-- bundle` files of
//!   `opendmd-web/formulas/`, compiled into the image by build.rs and
//!   installed into the core at boot ([`install`]);
//! - **the user's on the card**, `/formulas/<stem>.psi`: played from the
//!   compiled `<stem>.psb` the web app writes beside it (the engine reads
//!   it through the card source; a look's is read here, [`read_card`]).

/// (stem, compiled Formula) for every bundled Formula, in file-name order.
#[cfg(feature = "psi")]
pub const BUNDLED: &[(&str, &[u8])] = include!(concat!(env!("OUT_DIR"), "/bundled_formulas.rs"));

/// The bundled Formulas' sources (squeezed), for psi-probe's compiles.
#[cfg(feature = "psi-probe")]
pub const BUNDLED_SOURCES: &[(&str, &str)] = include!(concat!(env!("OUT_DIR"), "/bundled_sources.rs"));

/// Make `@formula:<stem>` play the bundled Formulas.
pub fn install() {
    #[cfg(feature = "psi")]
    opendmd_core::synth::set_bundled(BUNDLED);
}

/// A Formula on the card (`/formulas/x.psi`, card-absolute): its compiled
/// file's bytes (`/formulas/x.psb`), or why not as the sentence a look
/// shows.
#[cfg(feature = "psi")]
pub fn read_card(file: &str) -> Result<Vec<u8>, opendmd_core::psi::Error> {
    let err = |msg: String| opendmd_core::psi::Error { start: 0, end: 0, msg };
    if crate::files::card_path(crate::config::SD_MOUNT, file).is_none() {
        return Err(err("that Formula's path is not on the card".into()));
    }
    let full = format!("{}{}", crate::config::SD_MOUNT, opendmd_core::synth::compiled_path(file));
    let f = std::fs::File::open(&full).map_err(|e| {
        err(if e.kind() == std::io::ErrorKind::NotFound {
            opendmd_core::fx_psi::NOT_PREPARED.into()
        } else {
            format!("that Formula could not be read: {e}")
        })
    })?;
    opendmd_core::fx_psi::read_psb(f).map_err(|e| err(e.to_string()))
}

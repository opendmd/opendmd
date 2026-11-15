//! Create Art's pipeline: a picture or an animated GIF in, a 128x32 GIF out.
//!
//! [`pipeline`] finds and undoes an upscale, fits the picture to the panel,
//! picks the window and the motion and maps the colours (≤ 256, one global
//! palette); [`anim`] is an animated source, decoded through the player's
//! own decoder and saved in steps; [`encode`] writes the GIF. No browser
//! types. [`anim::Anim`] keeps its frames within the budget it is opened
//! with; decoding takes up to ~12 bytes per source pixel more, the source
//! at most 2048x2048 (see [`anim`]'s "Memory"). The web app's wasm
//! module (`opendmd-art-web`) is a wrapper over this crate, and a native
//! caller links it as it is.

pub mod anim;
pub mod encode;
pub mod pipeline;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_anim;

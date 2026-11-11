//! The Studio's GIF writers (`opendmd.dev/studio/`, `opendmd-web/share/studio/`):
//! a piece of 128x32 frames in, a GIF out. A wasm module of its own
//! (`make web`'s `pkg-studio/`), loaded by the site's Studio page at its
//! first download and never packed for the device (opendmd-pack
//! `www::SITE_ONLY`): the player's web app has no use for it, and its www
//! pack has no room. [`encode`] holds no browser types and is tested
//! natively and under node; this file only converts.

pub mod encode;

#[cfg(test)]
mod tests;

use wasm_bindgen::prelude::*;

/// The GIF of a piece of 256 colours or fewer ([`encode::indexed`]):
/// frames of 128x32 palette indices (`Uint8Array`, back to back) over
/// exactly `palette` (`Uint32Array` of `0xRRGGBB`), one delay each (ms),
/// played `plays` times (0: forever). Throws with why on a bad call.
#[wasm_bindgen(js_name = encodeIndexed)]
pub fn encode_indexed(
    palette: &[u32],
    frames: &[u8],
    delays_ms: &[u32],
    plays: u32,
) -> Result<Vec<u8>, JsError> {
    encode::indexed(palette, frames, delays_ms, plays).map_err(JsError::new)
}

/// The GIF of a piece past 256 colours, exactly ([`encode::wide`]): frames
/// of 128x32 colours (`Uint32Array` of `0xRRGGBB`, back to back), one delay
/// each (ms), played `plays` times (0: forever); each frame a whole picture
/// over its 256 most used colours, then the corrections, so no
/// colour is lost and nothing wipes. Throws with why on a bad call.
#[wasm_bindgen(js_name = encodeWide)]
pub fn encode_wide(frames: &[u32], delays_ms: &[u32], plays: u32) -> Result<Vec<u8>, JsError> {
    encode::wide(frames, delays_ms, plays).map_err(JsError::new)
}

/// The same frames fitted to their 256 most used colours (every other as
/// its nearest), over one table ([`encode::fitted`]): lossy, but the panel
/// can prepare it and smooth it. Throws with why on a bad call.
#[wasm_bindgen(js_name = encodeFitted)]
pub fn encode_fitted(frames: &[u32], delays_ms: &[u32], plays: u32) -> Result<Vec<u8>, JsError> {
    encode::fitted(frames, delays_ms, plays).map_err(JsError::new)
}

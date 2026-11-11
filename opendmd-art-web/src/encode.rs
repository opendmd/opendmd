//! The GIF writer: one global palette (so the prepared copy keeps exactly
//! these colours and never re-quantises), every frame whole, no
//! transparency. (The site's Studio has GIF writers of its own, past 256
//! colours too: opendmd-studio-web.) Delays are in hundredths of a second in the file; the
//! pipeline's are multiples of 10 ms. A still loops forever; an animation
//! from the timeline plays forever or a number of times ([`begin`]).

use crate::pipeline::{Animation, H, W};
use gif::{Encoder, Frame, Repeat};
use std::borrow::Cow;

/// The animation as GIF bytes, looping forever.
pub fn encode(anim: &Animation) -> Vec<u8> {
    let mut enc = begin(&anim.palette, 0);
    for (f, &d) in anim.frames.iter().zip(&anim.delays_ms) {
        frame(&mut enc, f, d);
    }
    finish(enc)
}

/// A GIF's header over `palette` (`0xRRGGBB`), played `plays` times — 0
/// forever (a NETSCAPE loop count of 0), 1 once (no loop block: what
/// players do without one), N: a loop count of N - 1 (N plays in all).
pub fn begin(palette: &[u32], plays: u32) -> Encoder<Vec<u8>> {
    let mut pal: Vec<u8> = palette
        .iter()
        .flat_map(|&c| [(c >> 16) as u8, (c >> 8) as u8, c as u8])
        .collect();
    // A one-colour picture still needs a two-entry table.
    pal.resize(pal.len().max(6), 0);
    // Writing into a Vec cannot fail, and the frames are well formed
    // (W x H indices into a palette of <= 256), so errors are bugs.
    let mut enc = Encoder::new(Vec::new(), W as u16, H as u16, &pal).expect("gif header");
    match plays {
        0 => enc.set_repeat(Repeat::Infinite).expect("gif loop"),
        1 => {}
        n => enc
            .set_repeat(Repeat::Finite((n - 1).min(u16::MAX as u32) as u16))
            .expect("gif loop"),
    }
    enc
}

/// One whole frame of `W * H` palette indices, shown `delay_ms`.
pub fn frame(enc: &mut Encoder<Vec<u8>>, idx: &[u8], delay_ms: u32) {
    let frame = Frame {
        width: W as u16,
        height: H as u16,
        buffer: Cow::Borrowed(idx),
        delay: (delay_ms.saturating_add(5) / 10).clamp(2, u16::MAX as u32) as u16,
        ..Frame::default()
    };
    enc.write_frame(&frame).expect("gif frame");
}

/// The bytes, trailer written.
pub fn finish(enc: Encoder<Vec<u8>>) -> Vec<u8> {
    enc.into_inner().expect("gif trailer")
}

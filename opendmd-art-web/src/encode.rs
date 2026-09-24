//! The GIF writer: one global palette (so the prepared copy keeps exactly
//! these colours and never re-quantises), every frame whole, no
//! transparency, loop forever. Delays are in hundredths of a second in the
//! file; the pipeline's are multiples of 10 ms.

use crate::pipeline::{Animation, H, W};
use gif::{Encoder, Frame, Repeat};
use std::borrow::Cow;

/// The animation as GIF bytes.
pub fn encode(anim: &Animation) -> Vec<u8> {
    let mut pal: Vec<u8> = anim
        .palette
        .iter()
        .flat_map(|&c| [(c >> 16) as u8, (c >> 8) as u8, c as u8])
        .collect();
    // A one-colour picture still needs a two-entry table.
    pal.resize(pal.len().max(6), 0);
    let mut out = Vec::new();
    {
        // Writing into a Vec cannot fail, and the frames are well formed
        // (W x H indices into a palette of <= 256), so errors are bugs.
        let mut enc = Encoder::new(&mut out, W as u16, H as u16, &pal).expect("gif header");
        enc.set_repeat(Repeat::Infinite).expect("gif loop");
        for (f, &d) in anim.frames.iter().zip(&anim.delays_ms) {
            let frame = Frame {
                width: W as u16,
                height: H as u16,
                buffer: Cow::Borrowed(f),
                delay: ((d + 5) / 10).clamp(2, u16::MAX as u32) as u16,
                ..Frame::default()
            };
            enc.write_frame(&frame).expect("gif frame");
        }
    }
    out
}

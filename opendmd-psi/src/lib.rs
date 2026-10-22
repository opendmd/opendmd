//! **Psi** (Ψ): OpenDMD's per-pixel effects language. Users call a script a
//! *Formula*; this crate is the language, its compiler and its VM.
//!
//! A formula is a few lines of arithmetic over the pixel's position and the
//! time, ending in a colour:
//!
//! ```text
//! loop 8
//! v = sin(x*3 + t/8) + sin((y*4 + t/8)*1.3) + sin(len(x-0.5, y-0.5)*6 - t/4)
//! hsv(v/3 + t/8, 1, 1)
//! ```
//!
//! [`compile`] turns the text into a [`Program`] — a fixed-size value of at
//! most [`MAX_OPS`] steps — and [`render`] draws a 128 × 32 frame of it
//! into RGB888 bytes, a row at a time, in Q16.16 integer arithmetic, so the
//! same frame is the same bytes on the ESP32, the host and in wasm. `no_std`;
//! the compiler uses `alloc` (and frees it all before returning), the VM
//! never allocates: its working memory is a caller's [`Scratch`].
//!
//! The language (v0) and the VM's design are in `docs/research/psi-spike.md`.

#![cfg_attr(not(test), no_std)]

extern crate alloc;

mod compile;
pub mod fixed;
mod program;
mod tables;
mod vm;

pub use compile::{compact, compile_into, number, Error, Info, MAX_DEPTH, MAX_NODES, MAX_SOURCE};
pub use program::{Op, Program, H, MAX_OPS, MAX_SCALARS, REGS, W};
pub use tables::{Tables, ATAN};
#[cfg(feature = "tables")]
pub use tables::{OWN, PALETTES, SIN};
pub use vm::{q16, render_with, Inputs, Scratch, Q16_ONE};

/// The lane width the effect uses: one panel row.
pub const LANES: usize = 128;

/// Compile `src` (a new [`Program`] by value: ~1.8 KB; on a small stack use
/// [`compile_into`] on a zeroed allocation instead).
pub fn compile(src: &str) -> Result<Program, Error> {
    let mut p = Program::default();
    compile_into(src, &mut p)?;
    Ok(p)
}

/// [`render_with`] at the default lane width, [`LANES`].
pub fn render(
    prog: &Program,
    scratch: &mut Scratch<LANES>,
    inputs: &Inputs,
    tables: &Tables,
    out: &mut [u8],
) {
    render_with::<LANES>(prog, scratch, inputs, tables, out)
}

/// The three sketches from the research report, as shipped examples and
/// the golden-frame and benchmark set (`--` comments, the report's `//`
/// also accepted).
pub mod sketches {
    pub const PLASMA: &str = "\
loop 8
v = sin(x*3 + t/8) + sin((y*4 + t/8)*1.3) + sin(len(x-0.5, y-0.5)*6 - t/4)
hsv(v/3 + t/8, 1, 1)
";

    pub const FIRE: &str = "\
loop 4
heat = noise(x*8, y*4 + t*2, t) * (1.3 - y)      -- hotter at the bottom (y = 0)
pal(fire, clamp(heat, 0, 1))
";

    pub const RING_CLOCK: &str = "\
loop 60
frame {
  hr = frac(hour/12 + minute/720)       -- the hour hand as a turn
  glow = 0.6 + 0.4*sin(t/2)
}
d = len((x-0.5)*4, (y-0.5))            -- the ring, centred, aspect-corrected
ang = frac(atan2(y-0.5, (x-0.5)*4) + 0.25)
ring = smoothstep(0.08, 0.0, abs(d - 0.8))
hand = step(abs(ang - hr), 0.01) * step(d, 0.8)
rgb(ring*glow*0.3 + hand, ring*glow*0.5, ring*glow)
";

    pub const ALL: [(&str, &str); 3] = [
        ("plasma", PLASMA),
        ("fire", FIRE),
        ("ring-clock", RING_CLOCK),
    ];
}

#[cfg(all(test, feature = "tables"))]
mod tests;

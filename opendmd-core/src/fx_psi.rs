//! `@fx:psi?src=<formula>`: a Psi formula ([`opendmd_psi`]) as a live
//! effect. Behind the `psi` feature (spike; nothing user-facing yet).
//!
//! `@fx:psi?src=<url-encoded source>[&a=0.5&b=..&c=..][&fps=50|100|25]`:
//! the formula compiled when the path is opened (an error makes the path
//! fail to open, as an unknown effect does); `a b c` are its sliders
//! (decimal, default 0); 50 fps by default. A loop is the formula's `loop
//! N` seconds; `t` is the frame's time into it, so the loop is seamless
//! when everything the formula moves completes whole cycles in it.
//!
//! **Memory.** The program (1,808 B) and the VM's working memory
//! ([`opendmd_psi::Scratch`], 8,192 B at 128 lanes) are one allocation made
//! when the path is opened, zeroed and filled in place (never built on the
//! main task's stack); no heap per frame, and [`Fx`](crate::fx::Fx) itself
//! holds only the pointer. The sine and the palettes are lent from
//! [`crate::fx`] (`SIN`, `PALETTES`), so the device carries one copy.

use crate::canvas::{Canvas, Rgb};
use crate::fx::{Effect, PALETTES, SIN};
pub use alloc_box::PsiBox;
use opendmd_psi as psi;
use std::time::Duration;

/// The lane width: one panel row.
pub const LANES: usize = psi::LANES;

/// The compiled formula, its working memory and its options.
pub struct PsiFx {
    prog: psi::Program,
    scratch: psi::Scratch<LANES>,
    delay_ms: u8,
    inputs: psi::Inputs,
}

/// The tables Psi reads, lent from `fx`.
pub fn tables() -> psi::Tables<'static> {
    // `Rgb` is `#[repr(C)]` with three `u8` fields (asserted in `canvas`),
    // so a `[Rgb; 32]` is a `[[u8; 3]; 32]`.
    const _: () = assert!(std::mem::size_of::<[Rgb; 32]>() == 96);
    let pal = |i: usize| -> &'static [[u8; 3]; 32] {
        // SAFETY: same size and alignment 1, every bit pattern valid.
        unsafe { &*PALETTES[i].as_ptr().cast::<[[u8; 3]; 32]>() }
    };
    psi::Tables {
        sin: &SIN,
        palettes: [pal(0), pal(1), pal(2), pal(3), pal(4)],
    }
}

/// The `@fx:psi?src=…` path for a formula: [`psi::compact`]ed, then
/// URL-encoded lightly (a space is `+`; `% & + # ?` and anything outside
/// printable ASCII are `%XX`), with `opts` (`a=0.5&fps=100`) after it.
/// May be longer than a virtual path may be (512 B): check.
pub fn path(src: &str, opts: &str) -> String {
    let mut p = String::from("@fx:psi?src=");
    for b in psi::compact(src).bytes() {
        match b {
            b' ' => p.push('+'),
            b'%' | b'&' | b'+' | b'#' | b'?' => p.push_str(&format!("%{b:02X}")),
            0x21..=0x7e => p.push(b as char),
            _ => p.push_str(&format!("%{b:02X}")),
        }
    }
    if !opts.is_empty() {
        p.push('&');
        p.push_str(opts);
    }
    p
}

impl PsiFx {
    /// The effect for an `@fx:psi` query, or why not.
    pub fn from_query(query: &str) -> Result<PsiBox, psi::Error> {
        let mut src = None;
        let mut delay_ms = 20;
        let mut sliders = [0i32; 3];
        for kv in query.split('&') {
            let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
            match k {
                "src" => src = Some(crate::synth::url_decode(v.as_bytes())),
                "fps" => {
                    delay_ms = match v {
                        "100" => 10,
                        "50" => 20,
                        "25" => 40,
                        _ => delay_ms,
                    }
                }
                "a" | "b" | "c" => {
                    if let Some(q) = psi::number(v) {
                        sliders[usize::from(k.as_bytes()[0] - b'a')] = q;
                    }
                }
                _ => {}
            }
        }
        let src = src.unwrap_or_default();
        let mut fx = PsiBox::new();
        psi::compile_into(&src, &mut fx.prog)?;
        fx.delay_ms = delay_ms;
        fx.inputs.a = sliders[0];
        fx.inputs.b = sliders[1];
        fx.inputs.c = sliders[2];
        Ok(fx)
    }

    fn ms(&self) -> u32 {
        u32::from(self.delay_ms.max(crate::fx::floor()))
    }

    pub fn delay(&self) -> Duration {
        Duration::from_millis(u64::from(self.ms()))
    }

    pub fn program(&self) -> &psi::Program {
        &self.prog
    }
}

impl Effect for PsiFx {
    fn name(&self) -> &'static str {
        "psi"
    }

    fn loop_frames(&self) -> u32 {
        (self.prog.loop_ms() / self.ms()).max(1)
    }

    fn frame(&mut self, t: Duration, canvas: &mut Canvas) -> Duration {
        let n = self.loop_frames();
        let ms = (t.as_secs() as u32)
            .wrapping_mul(1000)
            .wrapping_add(t.subsec_millis());
        let i = ms / self.ms() % n;
        self.inputs.t = psi::Inputs::at_ms(i * self.ms());
        if let Some(now) = crate::synth::wall_now() {
            self.inputs.hour = now.hour;
            self.inputs.minute = now.minute;
            self.inputs.second = now.second;
        }
        let (bytes, dirty) = canvas.bytes_and_dirty_mut();
        psi::render_with::<LANES>(
            &self.prog,
            &mut self.scratch,
            &self.inputs,
            &tables(),
            bytes,
        );
        dirty.mark_all();
        self.delay()
    }
}

mod alloc_box {
    use super::PsiFx;

    /// The one allocation: a `Box<PsiFx>` made zeroed (every field is
    /// integers, so zero is a valid value) and filled in place, so ~10
    /// KB never passes through the stack. `Clone` the same way.
    pub struct PsiBox(Box<PsiFx>);

    impl PsiBox {
        pub fn new() -> PsiBox {
            // SAFETY: `PsiFx` is integers and arrays of integers all the
            // way down (`Program`, `Scratch`, `Inputs`, `u8`): all-zero is
            // a valid value of it.
            let mut b = PsiBox(unsafe { Box::<PsiFx>::new_zeroed().assume_init() });
            b.0.prog.clear();
            b.0.scratch.reset();
            b.0.delay_ms = 20;
            b
        }
    }

    impl Default for PsiBox {
        fn default() -> PsiBox {
            PsiBox::new()
        }
    }

    impl std::ops::Deref for PsiBox {
        type Target = PsiFx;
        fn deref(&self) -> &PsiFx {
            &self.0
        }
    }

    impl std::ops::DerefMut for PsiBox {
        fn deref_mut(&mut self) -> &mut PsiFx {
            &mut self.0
        }
    }

    impl Clone for PsiBox {
        fn clone(&self) -> PsiBox {
            let mut b = PsiBox::new();
            b.0.prog = self.0.prog;
            b.0.delay_ms = self.0.delay_ms;
            b.0.inputs = self.0.inputs;
            b
        }
    }

    impl PartialEq for PsiBox {
        fn eq(&self, o: &PsiBox) -> bool {
            self.0.prog == o.0.prog
                && self.0.delay_ms == o.0.delay_ms
                && self.0.inputs == o.0.inputs
        }
    }

    impl Eq for PsiBox {}

    impl std::fmt::Debug for PsiBox {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "Psi({:?}, {} ms)", self.0.prog, self.0.delay_ms)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fx::Fx;

    fn path(src: &str) -> String {
        super::path(src, "")
    }

    #[test]
    fn core_lends_the_same_tables_psi_has() {
        let t = tables();
        assert_eq!(t.sin, &psi::SIN);
        for i in 0..5 {
            assert_eq!(t.palettes[i], &psi::PALETTES[i], "palette {i}");
        }
    }

    #[test]
    fn a_formula_plays_as_an_effect() {
        for (name, src) in psi::sketches::ALL {
            let mut fx = Fx::parse(&path(src)).unwrap_or_else(|| panic!("{name}"));
            assert_eq!(fx.name(), "psi");
            assert_eq!(fx.title(), "Formula");
            let prog = psi::compile(src).unwrap();
            assert_eq!(fx.loop_frames(), prog.loop_ms() / 20);
            let mut c = Canvas::new();
            // Frame 7 at 50 fps: t = 140 ms; no wall clock in tests.
            fx.frame(Duration::from_millis(140), &mut c);
            let mut want = vec![0u8; psi::W * psi::H * 3];
            let mut s = psi::Scratch::<128>::new();
            let inputs = psi::Inputs {
                t: psi::Inputs::at_ms(140),
                ..Default::default()
            };
            psi::render(&prog, &mut s, &inputs, &psi::OWN, &mut want);
            assert!(c.as_bytes() == want.as_slice(), "{name}");
        }
    }

    #[test]
    fn options_and_errors() {
        let p = super::path("rgb(a, b, c)", "a=0.5&b=1&c=.25&fps=100");
        let Some(Fx::Psi(fx)) = Fx::parse(&p) else {
            panic!("{p}")
        };
        assert_eq!(fx.inputs.a, 1 << 15);
        assert_eq!(fx.inputs.b, 1 << 16);
        assert_eq!(fx.inputs.c, 1 << 14);
        assert_eq!(fx.delay(), Duration::from_millis(10));
        assert!(Fx::parse(&path("gray(tme)")).is_none());
        assert!(Fx::parse("@fx:psi").is_none());
        assert!(Fx::parse(&path("loop 2\ngray(x)")).is_some());
        assert_eq!(
            PsiFx::from_query("src=gray(tme)").unwrap_err().msg,
            "unknown name `tme` — did you mean `t`?"
        );
        // A clone is equal and renders the same.
        let a = Fx::parse(&path(psi::sketches::PLASMA)).unwrap();
        assert_eq!(a.clone(), a);
    }

    #[test]
    fn every_sketch_fits_a_path() {
        for (name, src) in psi::sketches::ALL {
            let p = path(src);
            assert!(p.len() <= crate::synth::MAX_PATH, "{name}: {} B", p.len());
        }
        assert_eq!(
            path("v = x % 2 + 1 & 3 # é"),
            "@fx:psi?src=v+=+x+%25+2+%2B+1+%26+3+%23+%C3%A9"
        );
    }

    #[test]
    fn plays_through_the_synth_source() {
        let p = path(psi::sketches::FIRE);
        assert!(p.len() < crate::synth::MAX_PATH);
        assert_eq!(crate::synth::title(&p).as_deref(), Some("Formula"));
        let mut src = crate::synth::SynthSource::new(crate::synth::open(&p).unwrap()).unwrap();
        let mut c = Canvas::new();
        let mut frames = 0;
        while src.next_frame(&mut c).unwrap().is_some() {
            frames += 1;
        }
        assert_eq!(frames, 4000 / 20);
    }
}

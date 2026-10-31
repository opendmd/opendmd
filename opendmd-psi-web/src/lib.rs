//! The web app's Formulas: [`opendmd_psi`] (the language is Psi) behind a
//! small wasm-bindgen surface — compile a formula, report what it is
//! (steps, loop, sliders, an estimate of its frame time on the panel) or
//! why it does not compile (a plain sentence and where), and draw a frame
//! of it as 128 × 32 RGB888 bytes. A module of its own (`www/pkg-psi/`),
//! loaded only when the Library's Formulas tab opens.
//!
//! The VM is the firmware's, with Psi's own copies of the core's sine and
//! palettes (identical: core's tests check it), so a frame here is the
//! panel's byte for byte. The LED look is the main module's: the page hands
//! the bytes to its `Preview.showRgb`.
//!
//! **`no_std` in wasm, for size.** With std the module was ~37 KB gzipped
//! (std's panic machinery and its allocator); without, ~32 KB.
//! The program, the VM's memory and the frame are statics (one formula at a
//! time: the page compiles what it draws), so the heap holds only the
//! compiler's temporaries and the strings crossing to JS — all freed by the
//! time a call returns — and the allocator (`bump.rs`) is a bump pointer that
//! starts over whenever nothing is live. `make web` builds with the features
//! `bare` (no std, a panic is a trap) and `bump` (the allocator); the
//! tests (which need std) with neither; web-smoke checks that thousands of
//! compiles leave the module's memory as it was.

#![cfg_attr(all(target_arch = "wasm32", feature = "bare"), no_std)]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use core::cell::UnsafeCell;
use opendmd_psi::{
    compile_declared, compile_into, render_rows_ink, Control, Ink, Inputs, Program, Scratch, H, OWN,
    W,
};
use wasm_bindgen::prelude::*;

#[cfg(all(target_arch = "wasm32", feature = "bump"))]
mod bump;

#[cfg(all(target_arch = "wasm32", feature = "bare"))]
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    core::arch::wasm32::unreachable()
}

/// The one formula: the last program that compiled, a spare to compile
/// into, the VM's memory, the frame, and the last error.
struct State {
    prog: Program,
    next: Program,
    scratch: Scratch<128>,
    out: [u8; W * H * 3],
    ok: bool,
    /// The error's sentence (a fixed buffer: nothing stays on the heap).
    err: [u8; 512],
    err_len: usize,
    at: [u32; 2],
    /// A `loop forever` formula's `seed` ([`set_seed`]).
    seed: i32,
}

struct Cell(UnsafeCell<Option<State>>);
// SAFETY: wasm here is single-threaded, and every export takes the state
// for the length of the call only (nothing re-enters).
unsafe impl Sync for Cell {}
static STATE: Cell = Cell(UnsafeCell::new(None));

/// The picture under the formula ([`under`]): a GIF's frame the page puts
/// here for [`frame_over`]. A static of its own, all zero, so it costs the
/// module no bytes (in `State`, `None`'s niche made the whole value data).
struct Under(UnsafeCell<[u8; W * H * 3]>);
// SAFETY: as `Cell`.
unsafe impl Sync for Under {}
static UNDER: Under = Under(UnsafeCell::new([0; W * H * 3]));

/// The draw stage's canvas (`draw { }`): a static of its own, all zero, as
/// [`UNDER`].
struct InkCell(UnsafeCell<Ink>);
// SAFETY: as `Cell`.
unsafe impl Sync for InkCell {}
static INK: InkCell = InkCell(UnsafeCell::new(Ink::new()));

fn state() -> &'static mut State {
    // SAFETY: see `Cell`; the exports below never hold two of these.
    let s = unsafe { &mut *STATE.0.get() };
    s.get_or_insert_with(|| State {
        prog: Program::default(),
        next: Program::default(),
        scratch: Scratch::new(),
        out: [0; W * H * 3],
        ok: false,
        err: [0; 512],
        err_len: 0,
        at: [0; 2],
        seed: 0,
    })
}

/// UTF-16 units in `s[..byte]` (where JS's string indices are).
fn utf16_at(s: &str, byte: usize) -> u32 {
    let mut b = byte.min(s.len());
    while !s.is_char_boundary(b) {
        b -= 1;
    }
    s.get(..b).unwrap_or("").encode_utf16().count() as u32
}

/// Compile `src`. True: it is the formula drawn from now on. False:
/// [`error`] says why and [`error_at`] where, and the last formula that
/// compiled is still the one drawn.
#[wasm_bindgen]
pub fn compile(src: &str) -> bool {
    build(src)
}

/// `src` compiled for the player (`opendmd_psi::psb`: what a card's `.psb`
/// file holds and `@formula?psb=` carries), or empty when it does not
/// compile ([`compile`] says why). The player never compiles.
#[wasm_bindgen(js_name = program)]
pub fn program(src: &str) -> Vec<u8> {
    opendmd_psi::build(src).unwrap_or_default()
}

/// The compiled Formulas' format ([`program`]): the player says its own in
/// `/api/status` (`psi_abi`); bytes for another are refused there.
#[wasm_bindgen]
pub fn abi() -> u8 {
    opendmd_psi::psb::ABI
}

/// [`compile`], once in the module ([`as_look`] calls it twice).
#[inline(never)]
fn build(src: &str) -> bool {
    let s = state();
    match compile_into(src, &mut s.next) {
        Ok(_) => {
            core::mem::swap(&mut s.prog, &mut s.next);
            s.ok = true;
            s.err_len = 0;
            true
        }
        Err(e) => {
            s.at = [utf16_at(src, e.start), utf16_at(src, e.end)];
            fail(&e.msg);
            false
        }
    }
}

/// Keep `msg` as the error's sentence (cut at a char boundary).
fn fail(msg: &str) {
    let s = state();
    let mut n = msg.len().min(s.err.len());
    while !msg.is_char_boundary(n) {
        n -= 1;
    }
    s.err[..n].copy_from_slice(&msg.as_bytes()[..n]);
    s.err_len = n;
}

/// A formula's `look` line set (`blend` one of `over screen add multiply
/// max min`, at `amount` percent: `look`, `look screen 40`), or taken out
/// (`blend` empty): the Formula settings sheet's switch, a text edit
/// (`opendmd_psi::look::set`); nothing is compiled. Empty for another blend.
#[wasm_bindgen]
pub fn look(src: &str, blend: &str, amount: u8) -> String {
    opendmd_psi::look::set(src, blend, amount).unwrap_or_default()
}

/// `src` with its `nth` `word` declaration line (`name`, `about`,
/// `control`: `nth` counts control lines) replaced by `line`, taken out
/// when `line` is empty, or put in at the top when there is none
/// (`opendmd_psi::look::declare`): the settings sheet's edits. Nothing is
/// compiled.
#[wasm_bindgen]
pub fn declare(src: &str, word: &str, nth: usize, line: &str) -> String {
    opendmd_psi::look::declare(src, word, nth, line)
}

/// What the panel draws over every GIF for a formula kept as its look
/// (`fx_psi::FormulaLook`): the composition its `look` line asks for, or
/// the formula itself (no line, or `look` alone), compiled — [`info`] and
/// [`error`] then describe the look. Empty when it does not compile.
#[wasm_bindgen(js_name = asLook)]
pub fn as_look(src: &str) -> String {
    if !build(src) {
        return String::new();
    }
    match opendmd_psi::look::declared(src, &state().prog) {
        None => String::from(src),
        Some(Ok(s)) if build(&s) => s,
        Some(Ok(_)) => String::new(),
        Some(Err(e)) => {
            state().at = [0; 2];
            fail(&e.msg);
            String::new()
        }
    }
}

/// The last compile's error, a sentence ("unknown name `tme` — did you
/// mean `t`?"); empty after a compile that worked.
#[wasm_bindgen]
pub fn error() -> String {
    let s = state();
    // SAFETY: copied from a `String`, cut at a char boundary.
    String::from(unsafe { core::str::from_utf8_unchecked(&s.err[..s.err_len]) })
}

/// Where the last error is: `[start, end]` as JS string indices (the page
/// works out the line and column).
#[wasm_bindgen(js_name = errorAt)]
pub fn error_at() -> Vec<u32> {
    state().at.to_vec()
}

/// What the formula drawn is: `[steps, per frame, per row, per pixel, loop
/// ms (0: it never repeats, `loop forever`), inputs (bits 0-2 the sliders a
/// b c, 3 t, 4 the clock, 5 the picture under it: drawn over a GIF it
/// changes the GIF, 6 `seed`), estimated µs a frame on
/// the panel, layers, steps in `draw { }` (0: it draws nothing), lines and
/// dots it draws a frame, the frame time its `fps N` asks for in ms (10, 20
/// or 40; 0: none, so 20), estimated µs a frame as an effect with its still
/// cache and the JIT (the panel keeps the values it works out from the
/// pixel's position alone, and compiles the pixel stage; never more than the
/// VM's), the still cache's values (0-2, 16 KB each), estimated µs a frame
/// as a look with the JIT (no still cache; never more than [6]), estimated
/// µs a frame as an effect with its still cache on the VM (the gallery
/// checker's weight rule: `opendmd-gallery`'s `MAX_US`, which the site's
/// Worker applies to the gallery files it syncs)]`; empty before anything
/// compiled.
#[wasm_bindgen]
pub fn info() -> Vec<u32> {
    let s = state();
    if !s.ok {
        return Vec::new();
    }
    let p = &s.prog;
    let (f, r, x) = p.counts();
    alloc::vec![
        p.steps() as u32,
        f as u32,
        r as u32,
        x as u32,
        p.loop_ms(),
        u32::from(p.inputs()),
        p.esp32_us(),
        p.layers() as u32,
        p.draw_steps() as u32,
        p.primitives(),
        p.frame_ms().unwrap_or(0),
        p.esp32_us_jit_still(),
        p.still_lanes() as u32,
        p.esp32_us_jit(),
        p.esp32_us_still(),
    ]
}

/// The `seed` a formula that goes on without repeating (`loop forever`) is
/// drawn with from now on, Q16.16 0..1 (65,536 is 1); 0 until set (the
/// panel picks a new one each time it plays).
#[wasm_bindgen(js_name = setSeed)]
pub fn set_seed(seed: i32) {
    state().seed = seed;
}

/// A slider's value that says "not set": the slot plays at the formula's
/// declared default ([`opendmd_psi::Meta::values`]), as on the panel when
/// a path has no `&a=`.
pub const UNSET: i32 = i32::MIN;

/// Draw the frame at `ms` into the loop (the page works it out as the
/// panel does: a whole number of frame delays; for a formula that never
/// repeats, `info()`'s loop 0, `ms` since its preview began) with the wall
/// clock and the sliders (Q16.16: 65,536 is 1; [`UNSET`], -2³¹, for the
/// formula's default); [`pixels`] is where it is. Black before anything
/// compiled.
#[wasm_bindgen]
pub fn frame(ms: u32, hour: u8, minute: u8, second: u8, a: i32, b: i32, c: i32) {
    draw(false, ms, hour, minute, second, a, b, c);
}

/// [`frame`] over the picture at [`under`] (what `ur ug ub ul` read and
/// what shows through the layers): the formula drawn over a GIF's frame.
#[wasm_bindgen(js_name = frameOver)]
pub fn frame_over(ms: u32, hour: u8, minute: u8, second: u8, a: i32, b: i32, c: i32) {
    draw(true, ms, hour, minute, second, a, b, c);
}

#[allow(clippy::too_many_arguments)]
fn draw(over: bool, ms: u32, hour: u8, minute: u8, second: u8, a: i32, b: i32, c: i32) {
    let s = state();
    if s.ok {
        let given = |v: i32| (v != UNSET).then_some(v);
        let [a, b, c] = s.prog.meta().values([given(a), given(b), given(c)]);
        let mut inputs = Inputs {
            hour,
            minute,
            second,
            a,
            b,
            c,
            seed: s.seed,
            ..Inputs::default()
        };
        // A formula that never repeats: `ms` since its preview began.
        inputs.set_ms(&s.prog, u64::from(ms));
        // SAFETY: as `state()`: nothing else holds it during the call.
        let under = over.then(|| unsafe { &*UNDER.0.get() }.as_slice());
        // SAFETY: as `state()`.
        let ink = unsafe { &mut *INK.0.get() };
        render_rows_ink(
            &s.prog,
            &mut s.scratch,
            Some(ink),
            &inputs,
            &OWN,
            under,
            u32::MAX,
            &mut s.out,
        );
    }
}

/// What `src` declares, as JSON (empty when it does not compile: [`compile`]
/// says why): `{"name": "…" | null, "about": "…" | null, "look": [blend,
/// amount] | null, "own": n, "controls": [control…]}` (`own`: how many of
/// the controls are the formula's own `control` lines, the first n; the
/// rest its parts' or read without one), a control for each slot it
/// declares or reads, in slot order: `{"slot": "a", "name", "label",
/// "kind": "slider", "lo", "hi", "default", "declared": true}` or `{…,
/// "kind": "choice", "words": […], "default": 0, …}`. A slot read without
/// a `control` line is `{"slot": "a", "name": "a", "label": "a", "kind":
/// "slider", "lo": 0, "hi": 1, "default": 0, "declared": false}`. Labels,
/// names and words are the source's text (the lexer allows no `"`, `\` or
/// control characters in it, so it is JSON as it is). Numbers to 4
/// decimals. The formula drawn is not changed.
#[wasm_bindgen]
pub fn controls(src: &str) -> String {
    let s = state();
    let Ok(d) = compile_declared(src, &mut s.next) else {
        return String::new();
    };
    let m = *s.next.meta();
    let read = s.next.sliders();
    let text = |r: (u32, u32)| src.get(r.0 as usize..r.1 as usize).unwrap_or("");
    let mut j = String::with_capacity(256);
    let quoted = |j: &mut String, t: &str| {
        j.push('"');
        j.push_str(t);
        j.push('"');
    };
    let opt = |j: &mut String, r: Option<(u32, u32)>| match r {
        Some(r) => quoted(j, text(r)),
        None => j.push_str("null"),
    };
    j.push_str("{\"name\":");
    opt(&mut j, d.name);
    j.push_str(",\"about\":");
    opt(&mut j, d.about);
    j.push_str(",\"look\":");
    match m.look() {
        Some((b, k)) => {
            j.push('[');
            quoted(&mut j, b);
            j.push(',');
            number(&mut j, i32::from(k) << 16);
            j.push(']');
        }
        None => j.push_str("null"),
    }
    j.push_str(",\"own\":");
    j.push(char::from(b'0' + d.own));
    j.push_str(",\"controls\":[");
    let mut first = true;
    for k in 0..3 {
        let c = d.controls[k];
        if c.is_none() && read & (1 << k) == 0 {
            continue;
        }
        if !first {
            j.push(',');
        }
        first = false;
        let slot = ["a", "b", "c"][k];
        j.push_str("{\"slot\":");
        quoted(&mut j, slot);
        let name = c.map_or(slot, |c| text(c.name));
        j.push_str(",\"name\":");
        quoted(&mut j, name);
        j.push_str(",\"label\":");
        quoted(&mut j, c.and_then(|c| c.label).map_or(name, text));
        match m.control(k) {
            Some(Control::Choice { .. }) => {
                j.push_str(",\"kind\":\"choice\",\"words\":[");
                let words = c.and_then(|c| c.words).map_or("", text);
                for (i, w) in words.split(',').enumerate() {
                    if i > 0 {
                        j.push(',');
                    }
                    quoted(&mut j, w.trim_ascii());
                }
                j.push_str("],\"default\":0");
            }
            ctl => {
                let (lo, hi, def) = match ctl {
                    Some(Control::Slider { lo, hi, default }) => (lo, hi, default),
                    _ => (0, 1 << 16, 0),
                };
                j.push_str(",\"kind\":\"slider\",\"lo\":");
                number(&mut j, lo);
                j.push_str(",\"hi\":");
                number(&mut j, hi);
                j.push_str(",\"default\":");
                number(&mut j, def);
            }
        }
        j.push_str(",\"declared\":");
        j.push_str(if c.is_some() { "true}" } else { "false}" });
    }
    j.push_str("]}");
    j
}

/// A Q16.16 as a decimal to 4 places, trailing zeros out (no `format!`:
/// core::fmt is ~8 KB gz of the module).
fn number(j: &mut String, v: i32) {
    let neg = v < 0;
    let m = (v as i64).unsigned_abs();
    // Rounded to 1/10,000.
    let t = (m * 10_000 + (1 << 15)) >> 16;
    if neg && t != 0 {
        j.push('-');
    }
    let (int, mut frac) = (t / 10_000, t % 10_000);
    let mut b = [0u8; 24];
    let mut n = b.len();
    let mut w = int;
    loop {
        n -= 1;
        b[n] = b'0' + (w % 10) as u8;
        w /= 10;
        if w == 0 {
            break;
        }
    }
    for &c in &b[n..] {
        j.push(char::from(c));
    }
    if frac != 0 {
        j.push('.');
        let mut d = 1000;
        while frac != 0 {
            j.push(char::from(b'0' + (frac / d) as u8));
            frac %= d;
            d /= 10;
        }
    }
}

/// Where the page puts the picture [`frame_over`] draws over: 128 × 32
/// RGB888 bytes in the module's memory, top row first (kept until changed).
#[wasm_bindgen]
pub fn under() -> *mut u8 {
    UNDER.0.get().cast()
}

/// Where the frame is in the module's memory: 128 × 32 RGB888 bytes, top
/// row first (a view on it is good until the next call).
#[wasm_bindgen]
pub fn pixels() -> *const u8 {
    state().out.as_ptr()
}

#[cfg(not(target_arch = "wasm32"))]
mod examples;
#[cfg(not(target_arch = "wasm32"))]
pub use examples::{examples, examples_in, folder, split, Example};

#[cfg(test)]
mod tests;

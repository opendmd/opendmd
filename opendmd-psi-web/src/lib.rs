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
use opendmd_psi::{compile_into, render_rows_ink, Ink, Inputs, Program, Scratch, H, OWN, W};
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

/// [`compile`], once in the module ([`look`] calls it twice).
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

/// Any formula as a look (Settings › Look, the editor's "Use as the
/// look"): `src` drawn over the GIF with `blend` (`over screen add
/// multiply max min`) at `amount` percent, as the text the player is sent
/// (`opendmd_psi::look`), compiled — [`info`] and [`error`] then describe
/// the look. Empty when the formula or the look does not compile.
#[wasm_bindgen]
pub fn look(src: &str, blend: &str, amount: u8) -> String {
    if !build(src) {
        return String::new();
    }
    let ms = state().prog.loop_ms();
    match opendmd_psi::look::wrap(src, ms, blend, amount) {
        Ok(s) if build(&s) => s,
        Ok(_) => String::new(),
        Err(e) => {
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
/// ms, inputs (bits 0-2 the sliders a b c, 3 t, 4 the clock, 5 the picture
/// under it: drawn over a GIF it changes the GIF), estimated µs a frame on
/// the panel, layers, steps in `draw { }` (0: it draws nothing), lines and
/// dots it draws a frame, the frame time its `fps N` asks for in ms (10, 20
/// or 40; 0: none, so 20)]`; empty before anything compiled.
#[wasm_bindgen]
pub fn info() -> Vec<u32> {
    let s = state();
    if !s.ok {
        return Vec::new();
    }
    let p = &s.prog;
    let (f, r, x) = p.counts();
    alloc::vec![
        p.len() as u32,
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
    ]
}

/// Draw the frame at `ms` into the loop (the page works it out as the
/// panel does: a whole number of frame delays) with the wall clock and the
/// sliders (Q16.16: 65,536 is 1); [`pixels`] is where it is. Black before
/// anything compiled.
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
        let inputs = Inputs {
            t: Inputs::at_ms(ms),
            hour,
            minute,
            second,
            a,
            b,
            c,
        };
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
pub use examples::{examples, Example};

#[cfg(test)]
mod tests;

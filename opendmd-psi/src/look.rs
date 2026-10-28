//! Any Formula as a look: drawn over every GIF with a blend, at an amount.
//!
//! The web app composes the look's source here (Settings › Look and the
//! Formulas editor's "Use as the look", through `opendmd-psi-web`'s
//! `look()`), and the player draws it as any other formula: nothing in the
//! language or the firmware is new. The text stands alone, as shared
//! Formulas do — the formula pasted in as a part:
//!
//! ```text
//! loop 8                         -- the formula's loop, when it isn't 8 s
//!                                -- (`loop forever` for one that never repeats)
//! part look {
//!   <the formula, indented 2>
//! }
//! layer screen { look(x, y, 0.4) }   -- `look(x, y, 0.4)` alone for over
//! ```
//!
//! - **over at 100%** is the formula itself, unchanged: a formula that
//!   already draws over the GIF is the look it always was, byte for byte.
//!   (One that covers the GIF entirely is still refused there, by the
//!   player: `fx_psi::COVERS`.)
//! - **The loop** goes outside too: inside a part `t` is `t % (its loop)`,
//!   so a part with a loop longer than the formula's (8 s by default) would
//!   never reach its end.
//! - **The amount** is the coverage, `0` to `1` in hundredths; at 100% none
//!   is written (`layer screen { look }`).
//! - **A formula with layers of its own** keeps its blends: as a part it
//!   can only go over (`layer { p }`, or the last line, with coverage: each
//!   of its layers covers that much less). Another blend for one is the
//!   compiler's error, which the web app does not offer.
//! - **`ur ug ub ul`** inside the part read the real pixel's picture, the
//!   GIF: a formula that reads them grades the GIF, then the blend puts
//!   that onto the GIF again.
//!
//! The web app takes a saved look apart again by this shape (js/looks.js
//! `saved`, a regex over the path's `; `-joined text, for a look whose
//! formula that browser doesn't have): change both together.
//!
//! **A formula's own `look` line** (`look screen 40`,
//! docs/research/formula-apps.md §2.1) is this composition written in the
//! formula: [`declared`] gives the composed text the line asks for (the
//! player draws that as the look; played on its own the line is ignored).
//! The line itself is left out of the part ([`wrap`] cuts it), so a look
//! composed from a formula with one is the same text as from the formula
//! without it, and the formula's `control` lines inside the part still
//! describe the slots (the compiler takes a part's controls when the
//! formula's own lines declare none).

use crate::{compile_into, Error, Program};
use alloc::string::String;

/// The blends a look may use, as `layer` names them (`max` and `min` are
/// the web app's Lighten and Darken).
pub const BLENDS: [&str; 6] = ["over", "screen", "add", "multiply", "max", "min"];

/// The part's name in a composed look.
const PART: &str = "look";

/// The look's source: `src` drawn with `blend` (one of [`BLENDS`]) at
/// `amount` percent (0..=100, clamped). Errors: `src` does not compile, or
/// the blend is not one of [`BLENDS`]. The result is not compiled here
/// (a layered formula with a blend other than over is the compiler's
/// error, as is a look too long for the panel).
pub fn compose(src: &str, blend: &str, amount: u8) -> Result<String, Error> {
    // Only its loop and declarations are wanted (boxed: ~2.9 KB).
    let mut p = alloc::boxed::Box::<Program>::default();
    compile_into(src, &mut p)?;
    wrap(src, &p, blend, amount)
}

/// The look a formula's own `look` line asks for, composed as [`compose`]
/// would: `None` when it has none, or it is `look` (over, 100: the formula
/// itself). `prog` is `src` compiled.
pub fn declared(src: &str, prog: &Program) -> Option<Result<String, Error>> {
    let (blend, amount) = prog.meta().look()?;
    (blend != "over" || amount < 100).then(|| wrap(src, prog, blend, amount))
}

/// [`compose`] for a formula already compiled into `prog` (its loop, and
/// where a `look` line is, which is left out). The web module compiles
/// once and calls this.
pub fn wrap(src: &str, prog: &Program, blend: &str, amount: u8) -> Result<String, Error> {
    let loop_ms = prog.loop_ms();
    // Its `look` line, left out.
    let cut = prog.meta().look().and_then(|_| look_statement(src));
    let owned;
    let src = match cut {
        Some((a, e)) => {
            // (Not `concat`: its generic join is ~1 KB of the web module.)
            let mut o = String::with_capacity(src.len());
            o.push_str(src.get(..a).unwrap_or(""));
            o.push_str(src.get(e..).unwrap_or(""));
            owned = o;
            owned.as_str()
        }
        None => src,
    };
    // (No `format!`: core::fmt is ~8 KB gz of the web module.)
    if !BLENDS.contains(&blend) {
        return Err(Error {
            start: 0,
            end: 0,
            msg: String::from("a look's blend is one of over, screen, add, multiply, max, min"),
        });
    }
    let amount = amount.min(100);
    if blend == "over" && amount == 100 {
        return Ok(String::from(src));
    }
    let mut s = String::with_capacity(src.len() + 64);
    if loop_ms == 0 {
        // A formula that never repeats: so does its look.
        s.push_str("loop forever\n");
    } else if loop_ms != 8000 {
        s.push_str("loop ");
        decimal(&mut s, loop_ms / 1000, loop_ms % 1000, 3);
        s.push('\n');
    }
    s.push_str("part look {\n");
    // Its lines, blank ones left out (bytes: `trim` is Unicode's).
    for l in src.split('\n') {
        let l = l.strip_suffix('\r').unwrap_or(l);
        if l.bytes().any(|b| !b.is_ascii_whitespace()) {
            s.push_str("  ");
            s.push_str(l);
            s.push('\n');
        }
    }
    s.push_str("}\n");
    if blend != "over" {
        s.push_str("layer ");
        s.push_str(blend);
        s.push_str(" { ");
    }
    s.push_str(PART);
    if amount < 100 {
        s.push_str("(x, y, ");
        decimal(&mut s, 0, u32::from(amount), 2);
        s.push(')');
    }
    if blend != "over" {
        s.push_str(" }");
    }
    Ok(s)
}

/// Where a formula's `look` line is (from `look` to its `;` or its line's
/// end, a comment after it included): the formula compiled, so its
/// declarations are the statements at its top whose first word is one
/// (`name`, `about` and `control` lines may have text in quotes, with
/// `;` or `--` inside).
fn look_statement(src: &str) -> Option<(usize, usize)> {
    let b = src.as_bytes();
    let mut i = 0;
    loop {
        // The next statement: past spaces, line ends, `;` and comments.
        while i < b.len() {
            match b[i] {
                b' ' | b'\t' | b'\r' | b'\n' | b';' => i += 1,
                b'-' | b'/' if b.get(i + 1) == Some(&b[i]) => {
                    while i < b.len() && b[i] != b'\n' {
                        i += 1;
                    }
                }
                _ => break,
            }
        }
        let start = i;
        while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
            i += 1;
        }
        let word = src.get(start..i)?;
        if !crate::builtins::DECLARATIONS.contains(&word) {
            return None;
        }
        // To the statement's end: `;` or the line's end outside quotes.
        let mut quote = false;
        while i < b.len() && b[i] != b'\n' && (quote || b[i] != b';') {
            if b[i] == b'"' {
                quote = !quote;
            }
            if !quote && (b[i] == b'-' || b[i] == b'/') && b.get(i + 1) == Some(&b[i]) {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
                break;
            }
            i += 1;
        }
        if word == "look" {
            return Some((start, (i + usize::from(b.get(i) == Some(&b';'))).min(b.len())));
        }
    }
}

/// `whole.frac` with `frac` in `places` decimal places, trailing zeros
/// (and a point with none after it) left out.
fn decimal(s: &mut String, whole: u32, frac: u32, places: u32) {
    let mut b = [0u8; 16];
    let mut n = 10;
    let mut w = whole;
    loop {
        n -= 1;
        b[n] = b'0' + (w % 10) as u8;
        w /= 10;
        if w == 0 {
            break;
        }
    }
    let mut end = 10;
    if frac > 0 {
        b[10] = b'.';
        let mut f = frac;
        for i in (0..places as usize).rev() {
            b[11 + i] = b'0' + (f % 10) as u8;
            f /= 10;
        }
        end = 11 + places as usize;
        while b[end - 1] == b'0' {
            end -= 1;
        }
    }
    for &c in &b[n..end] {
        s.push(char::from(c));
    }
}

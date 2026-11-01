//! Any Formula as a look: drawn over every GIF with a blend, at an amount.
//!
//! A formula says it can be a look with its own `look [blend] [amount]`
//! line (the web app's Formula settings sheet writes the line with
//! [`set`], a text edit). The compiler draws it that way itself
//! ([`crate::compile_look`]: its colour put on with the blend, covering
//! that much; a formula with layers goes over only, each layer covering
//! that much less), and [`crate::build`] stores that program beside the
//! formula's own in its psb: the player draws the look as any other
//! formula.
//!
//! - **over at 100%** (`look`) is the formula itself, unchanged.
//! - **`ur ug ub ul`** read the GIF: a formula that reads them grades the
//!   GIF, then the blend puts that onto the GIF again.
//! - **`layer rate N`**: a look with a blend or an amount redraws its
//!   layers every frame.

use crate::{Error, Program};
use alloc::string::String;

pub use crate::meta::BLENDS;

/// `src` as a look drawn with `blend` (one of [`BLENDS`]) at `amount`
/// percent (0..=100, clamped): its `look` line set so ([`set`]) and
/// compiled as the player draws it. Errors: the blend is not one of
/// [`BLENDS`], or the formula does not compile so (layers with a blend
/// other than over, too long for the panel).
#[cfg(feature = "compiler")]
pub fn compose(src: &str, blend: &str, amount: u8) -> Result<Program, Error> {
    let Some(src) = set(src, blend, amount) else {
        return Err(Error {
            start: 0,
            end: 0,
            msg: String::from("a look's blend is one of over, screen, add, multiply, max, min"),
        });
    };
    let mut p = Program::default();
    program(&src, &mut p)?;
    Ok(p)
}

/// The look `src`'s own `look` line asks for (the formula itself when it
/// has none, or `look` alone) compiled into `prog` as the player loads it
/// (narrowed and fused): what the panel draws over every GIF.
#[cfg(feature = "compiler")]
pub fn program(src: &str, prog: &mut Program) -> Result<(), Error> {
    crate::compile_look(src, prog)?;
    prog.finish(true, true);
    Ok(())
}

/// `src` as a function, `fn name(x, y) { … }`, for another formula to call
/// (the editor's `use Plasma as p`): its declarations, `loop` and `fps`
/// lines left out, each `control` a name at its default (`speed = 1`).
/// `None` for one that can't be a function — with layers, a draw stage,
/// tables, sprites, palettes or functions of its own — or that doesn't
/// compile as one.
#[cfg(feature = "compiler")]
pub fn function(src: &str, name: &str) -> Option<String> {
    let mut body = String::new();
    for st in statements(src) {
        let st = st.trim();
        let word: &str = st.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).next().unwrap_or("");
        let rest = st.get(word.len()..).unwrap_or("").trim_start();
        let sets = rest.starts_with('=') && !rest.starts_with("==");
        if st.is_empty() {
            continue;
        }
        let line = match word {
            _ if sets => String::from(st),
            "psi" | "name" | "about" | "look" | "loop" | "fps" => continue,
            "control" => {
                let mut w = rest.split(|c: char| c.is_ascii_whitespace() || c == '=').filter(|w| !w.is_empty());
                let n = w.next()?;
                let after = rest.get(n.len()..).unwrap_or("").trim_start();
                let def = if let Some(v) = after.strip_prefix('=') {
                    // `= 0.4` or `= calm, wild` (the first word: 0).
                    let v = v.trim_start();
                    let num: &str = v.split(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-')).next().unwrap_or("");
                    if num.is_empty() { "0" } else { num }
                } else {
                    // `lo..hi [= v]`
                    match after.split_once('=') {
                        Some((_, v)) => v.trim_start().split(|c: char| c.is_ascii_whitespace() || c == '"').next().unwrap_or("0"),
                        None => after.split("..").next().unwrap_or("0").trim(),
                    }
                };
                let def = if def.is_empty() { "0" } else { def };
                let mut l = String::from(n);
                l.push_str(" = ");
                l.push_str(def);
                l
            }
            "layer" | "draw" | "fn" | "table" | "sprite" | "palette" | "part" | "use" => return None,
            _ => String::from(st),
        };
        for l in line.split('\n') {
            body.push_str("  ");
            body.push_str(l);
            body.push('\n');
        }
    }
    let mut f = String::from("fn ");
    f.push_str(name);
    f.push_str("(x, y) {\n");
    f.push_str(&body);
    f.push('}');
    let mut check = f.clone();
    check.push('\n');
    check.push_str(name);
    check.push_str("(x, y)");
    crate::compile(&check).ok()?;
    Some(f)
}

/// `src`'s statements at its top level (split at line ends and `;` outside
/// brackets, braces and quotes; a comment kept on its own).
#[cfg(feature = "compiler")]
fn statements(src: &str) -> alloc::vec::Vec<&str> {
    let b = src.as_bytes();
    let (mut out, mut start, mut depth, mut quote, mut i) = (alloc::vec::Vec::new(), 0, 0i32, false, 0);
    while i < b.len() {
        match b[i] {
            b'"' => quote = !quote,
            b'-' | b'/' if !quote && b.get(i + 1) == Some(&b[i]) => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            b'(' | b'[' | b'{' if !quote => depth += 1,
            b')' | b']' | b'}' if !quote => depth -= 1,
            b'\n' | b';' if !quote && depth <= 0 => {
                out.push(src.get(start..i).unwrap_or(""));
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    out.push(src.get(start..).unwrap_or(""));
    out
}

/// A formula's `look` line set to `blend` at `amount` percent (`look`
/// alone for over at 100, `look screen`, `look screen 40`, `look over
/// 30`), or taken out when `blend` is empty: a text edit ([`declare`]),
/// the one way the web app writes a look (its settings sheet). `None`
/// for a blend that is not one of [`BLENDS`].
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn set(src: &str, blend: &str, amount: u8) -> Option<String> {
    if blend.is_empty() {
        return Some(declare(src, "look", 0, ""));
    }
    if !BLENDS.contains(&blend) {
        return None;
    }
    let amount = amount.min(100);
    let mut l = String::from("look");
    if blend != "over" || amount < 100 {
        l.push(' ');
        l.push_str(blend);
    }
    if amount < 100 {
        l.push(' ');
        decimal(&mut l, u32::from(amount), 0, 0);
    }
    Some(declare(src, "look", 0, &l))
}

/// `src` with its `nth` declaration whose first word is `word` (`name`,
/// `about`, `control`, `look`; `nth` counts `control`s) replaced by
/// `line`, taken out when `line` is empty, or — when there is none —
/// `line` put in among the declarations at the top, in their usual order
/// (`psi`, `name`, `about`, `control`, `look`). Text only: the result is
/// not compiled (the caller does), and `line` is written as given. The
/// declarations are the statements at the formula's top whose first word
/// is one (blank lines and comments between them skipped), as the
/// compiler requires them to be: each ends at its line's end or at a `;`
/// outside quotes (a path's text has them all on one line, `; `-joined).
#[inline(never)]
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn declare(src: &str, word: &str, nth: usize, line: &str) -> String {
    const ORDER: [&str; 5] = ["psi", "name", "about", "control", "look"];
    let rank = |w: &str| ORDER.iter().position(|&o| o == w).unwrap_or(5);
    let me = rank(word);
    // (Bytes and `get`, never `[a..b]` of a str: a slice's panic brings
    // core::fmt, ~8 KB gz of the web module.)
    let b = src.as_bytes();
    let part = |a: usize, e: usize| src.get(a..e).unwrap_or("");
    // A statement: (start, end before its `;` or line break, past that).
    // The match, and where a new one goes: after the last declaration
    // ranked at or before `word` (and how it ended), else before the first.
    let (mut found, mut after, mut first, mut seen, mut at) = (None, None, None, 0, 0);
    while at < b.len() {
        while at < b.len() && matches!(b[at], b' ' | b'\t' | b'\r' | b'\n' | b';') {
            at += 1;
        }
        let i = at;
        let mut e = i;
        while e < b.len() && (b[e].is_ascii_alphanumeric() || b[e] == b'_') {
            e += 1;
        }
        let w = part(i, e);
        let mut k = e;
        while k < b.len() && matches!(b[k], b' ' | b'\t') {
            k += 1;
        }
        // Its end: `;` or the line's, outside quotes; a comment runs to
        // the line's end.
        let (mut end, mut quote) = (i, false);
        while end < b.len() && b[end] != b'\n' && (quote || b[end] != b';') {
            if !quote && matches!(b[end], b'-' | b'/') && b.get(end + 1) == Some(&b[end]) {
                while end < b.len() && b[end] != b'\n' {
                    end += 1;
                }
                break;
            }
            quote ^= b[end] == b'"';
            end += 1;
        }
        let next = (end + 1).min(b.len());
        if e == i && end > i && matches!(b[i], b'-' | b'/') && b.get(i + 1) == Some(&b[i]) {
            // A comment alone.
            at = next;
            continue;
        }
        if i >= b.len() || rank(w) == 5 || b.get(k) == Some(&b'=') {
            break;
        }
        first.get_or_insert(i);
        if rank(w) <= me {
            after = Some((next, b.get(end) == Some(&b';')));
        }
        if w == word {
            if seen == nth {
                found = Some((i, end, next));
            }
            seen += 1;
        }
        at = next;
    }
    let mut o = String::with_capacity(src.len() + line.len() + 2);
    match found {
        Some((a, e, next)) => {
            o.push_str(part(0, a));
            if line.is_empty() {
                // The statement, its `;` or line break, the spaces after.
                let mut n = next;
                while b.get(e) == Some(&b';') && b.get(n) == Some(&b' ') {
                    n += 1;
                }
                o.push_str(part(n, b.len()));
            } else {
                o.push_str(line);
                o.push_str(part(e, b.len()));
            }
        }
        None if line.is_empty() => o.push_str(src),
        None => {
            let (p, semi) = match after {
                Some(x) => x,
                None => (first.unwrap_or(0), false),
            };
            o.push_str(part(0, p));
            if semi {
                // On a `; `-joined line: one more statement there.
                o.push(' ');
                o.push_str(line);
                o.push(';');
            } else {
                if p > 0 && b.get(p - 1) != Some(&b'\n') {
                    o.push('\n');
                }
                o.push_str(line);
                o.push('\n');
            }
            o.push_str(part(p, b.len()));
        }
    }
    o
}

/// `whole.frac` with `frac` in `places` decimal places, trailing zeros
/// (and a point with none after it) left out.
#[cfg_attr(target_arch = "xtensa", optimize(size))]
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

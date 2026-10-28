//! Source text to [`Program`]: a lexer, a Pratt parser that builds an
//! expression DAG as it goes (constant folding, common subexpressions,
//! division by constants turned into multiplication), a type pass that is
//! really a *level*: constant, per frame, per row, per pixel, so work runs
//! as rarely as its inputs change, and a linear-scan register allocator for
//! the per-pixel lanes.
//!
//! The compiler is the only part of Psi that allocates (its token and node
//! lists, a few KB, freed when it returns); the program it fills is a
//! fixed-size value. Source is capped at [`MAX_SOURCE`] bytes, nodes at
//! [`MAX_NODES`] and nesting at [`MAX_DEPTH`], so compile time and stack
//! are bounded for any input.

use crate::fixed::{self, ONE};
use crate::program::{code, *};
use crate::tables;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// The longest source accepted (bytes).
pub const MAX_SOURCE: usize = 4096;
/// Expression nodes (before dead ones are dropped): bounds compile time.
pub const MAX_NODES: usize = 1024;
/// Brackets and operators nested this deep: bounds the parser's stack.
/// 18 since 2026-10-26: at 24 the deepest formula used 11,808 B of the
/// ESP32's 12 KB compile stack (`psi-probe` on the spare; beta.10's
/// compiler, by its frame sizes, more), ~430 B a level; at 18 ~9.2 KB.
/// Every known formula needs at most 12 (the examples' parts 3 a level).
pub const MAX_DEPTH: usize = 18;

/// A compile error: a byte range of the source and a sentence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    pub start: usize,
    pub end: usize,
    pub msg: String,
}

impl Error {
    #[inline(never)]
    fn new(start: usize, end: usize, msg: impl Into<String>) -> Error {
        Error {
            start,
            end,
            msg: msg.into(),
        }
    }

    /// 1-based line and column of the start.
    pub fn line_col(&self, src: &str) -> (usize, usize) {
        let upto = sub(src, 0, self.start.min(src.len()));
        let line = upto.matches('\n').count() + 1;
        let col = upto.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
        (line, col)
    }

    /// "line 2: msg", the line, and carets under the range.
    pub fn render(&self, src: &str) -> String {
        let (line, col) = self.line_col(src);
        let text = src.lines().nth(line - 1).unwrap_or("");
        let width = sub(src, self.start, self.end.min(src.len()))
            .chars()
            .take_while(|&c| c != '\n')
            .count()
            .max(1);
        format!(
            "line {line}: {}\n  {text}\n  {}{}",
            self.msg,
            " ".repeat(col - 1),
            "^".repeat(width)
        )
    }
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.msg)
    }
}

/// `s[a..b]`, or "" (never a panic: a slice of a `str` that could panic
/// brings ~3 KB of panic formatting into the wasm).
#[inline(never)]
fn sub(s: &str, a: usize, b: usize) -> &str {
    s.get(a..b).unwrap_or("")
}

// --- the lexer ----------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tok {
    Num(i32),
    /// `#rrggbb` (`#rgb` widened): a colour, 0xRRGGBB.
    Hex(u32),
    Name,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Lt,
    Le,
    Gt,
    Ge,
    EqEq,
    Ne,
    And,
    Or,
    Not,
    Question,
    Colon,
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Comma,
    Dot,
    Assign,
    Newline,
    /// A row of a `sprite NAME { … }` block (its key line or a row of
    /// characters), as written: the lexer reads those lines raw.
    Raw,
    Eof,
}

#[derive(Clone, Copy, Debug)]
struct Token {
    tok: Tok,
    start: usize,
    end: usize,
}

fn describe(t: Tok) -> &'static str {
    match t {
        Tok::Num(_) => "a number",
        Tok::Hex(_) => "a colour",
        Tok::Name => "a name",
        Tok::Newline => "the end of the line",
        Tok::Eof => "the end",
        Tok::RParen => "`)`",
        Tok::LParen => "`(`",
        Tok::RBrace => "`}`",
        Tok::LBrace => "`{`",
        Tok::RBracket => "`]`",
        Tok::LBracket => "`[`",
        Tok::Comma => "`,`",
        Tok::Dot => "`.`",
        Tok::Assign => "`=`",
        Tok::Colon => "`:`",
        Tok::Question => "`?`",
        Tok::Raw => "a sprite's row",
        _ => "an operator",
    }
}

/// A decimal number (digits, an optional `.` and more digits; no sign) as
/// Q16.16, rounded to the nearest 1/65,536: how the compiler reads numbers,
/// for the sliders' `a=` options too.
pub fn number(s: &str) -> Option<i32> {
    if s.is_empty()
        || s == "."
        || s.matches('.').count() > 1
        || !s.bytes().all(|b| b.is_ascii_digit() || b == b'.')
    {
        return None;
    }
    let (int, frac) = s.split_once('.').unwrap_or((s, ""));
    let mut v: i64 = 0;
    for b in int.bytes() {
        v = v * 10 + (b - b'0') as i64;
        if v > 32767 {
            return None;
        }
    }
    // The fraction to 9 digits, rounded once.
    let mut num: i64 = 0;
    let mut den: i64 = 1;
    for b in frac.bytes().take(9) {
        num = num * 10 + (b - b'0') as i64;
        den *= 10;
    }
    let f = (num * 65536 + den / 2) / den;
    let r = (v << 16) + f;
    (r <= i32::MAX as i64).then_some(r as i32)
}

#[inline(never)]
fn lex(src: &str) -> Result<Vec<Token>, Error> {
    let b = src.as_bytes();
    let mut out: Vec<Token> = Vec::new();
    let mut i = 0;
    let mut depth = 0usize; // ( ) nesting: newlines inside are spaces
                            // At the start of a line (nothing but spaces before, outside brackets):
                            // where `sprite NAME {` may start a sprite block.
    let mut line_start = true;
    let push_nl = |out: &mut Vec<Token>, at: usize| {
        if !matches!(
            out.last(),
            None | Some(Token {
                tok: Tok::Newline,
                ..
            })
        ) {
            out.push(Token {
                tok: Tok::Newline,
                start: at,
                end: at,
            });
        }
    };
    while i < b.len() {
        let c = b[i];
        let start = i;
        // Comments: `--` or `//` to the end of the line.
        if (c == b'-' && b.get(i + 1) == Some(&b'-')) || (c == b'/' && b.get(i + 1) == Some(&b'/'))
        {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if c == b'\n' || c == b';' {
            if depth == 0 {
                push_nl(&mut out, i);
                line_start = true;
            }
            i += 1;
            continue;
        }
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if line_start && depth == 0 {
            if let Some(j) = sprite_head(b, i) {
                i = sprite_rows(src, i, j, &mut out);
                line_start = false;
                continue;
            }
        }
        line_start = false;
        // `#ff8800`, `#f80`: a colour.
        if c == b'#' {
            i += 1;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
            let h = sub(src, start + 1, i);
            let mut v = Some(0u32);
            for c in h.chars() {
                v = v.zip(c.to_digit(16)).map(|(v, d)| v << 4 | d);
            }
            let (Some(v), 3 | 6) = (v, h.len()) else {
                return Err(Error::new(
                    start,
                    i,
                    "a colour is # and 3 or 6 hex digits (0-9, a-f), as #f80 or #ff8800",
                ));
            };
            // #rgb: each digit twice.
            let v = if h.len() == 3 {
                (v >> 8) * 0x11_0000 + (v >> 4 & 15) * 0x1100 + (v & 15) * 0x11
            } else {
                v
            };
            out.push(Token {
                tok: Tok::Hex(v),
                start,
                end: i,
            });
            continue;
        }
        // `'A'`: a character's code (65), for `glyph`.
        if c == b'\'' {
            if let (Some(&ch @ 32..=126), Some(b'\'')) = (b.get(i + 1), b.get(i + 2)) {
                i += 3;
                out.push(Token {
                    tok: Tok::Num((ch as i32) << 16),
                    start,
                    end: i,
                });
                continue;
            }
            let end = i + sub(src, i + 1, b.len())
                .chars()
                .next()
                .map_or(1, |ch| ch.len_utf8() + 1);
            return Err(Error::new(
                start,
                end.min(b.len()),
                "a character goes in quotes, one at a time: 'A', '7' or ':' (there are no strings)",
            ));
        }
        if c.is_ascii_digit() || (c == b'.' && b.get(i + 1).is_some_and(u8::is_ascii_digit)) {
            while i < b.len() && (b[i].is_ascii_digit() || b[i] == b'.') {
                i += 1;
            }
            let s = sub(src, start, i);
            if s.matches('.').count() > 1 {
                return Err(Error::new(start, i, format!("`{s}` is not a number")));
            }
            if i < b.len() && (b[i].is_ascii_alphabetic() || b[i] == b'_') {
                let mut j = i;
                while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'_') {
                    j += 1;
                }
                return Err(Error::new(
                    start,
                    j,
                    format!(
                        "`{}` is not a number or a name (to multiply, write `{} * {}`)",
                        sub(src, start, j),
                        s,
                        sub(src, i, j)
                    ),
                ));
            }
            let v = number(s)
                .ok_or_else(|| Error::new(start, i, "numbers must be between -32768 and 32767"))?;
            out.push(Token {
                tok: Tok::Num(v),
                start,
                end: i,
            });
            continue;
        }
        // `p.v`: a part's value.
        if c == b'.'
            && b.get(i + 1)
                .is_some_and(|&n| n.is_ascii_alphabetic() || n == b'_')
        {
            i += 1;
            out.push(Token {
                tok: Tok::Dot,
                start,
                end: i,
            });
            continue;
        }
        if c.is_ascii_alphabetic() || c == b'_' {
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
            let tok = match sub(src, start, i) {
                "and" => Tok::And,
                "or" => Tok::Or,
                "not" => Tok::Not,
                _ => Tok::Name,
            };
            out.push(Token { tok, start, end: i });
            continue;
        }
        let two = b.get(i + 1).copied();
        let (tok, n) = match (c, two) {
            (b'<', Some(b'=')) => (Tok::Le, 2),
            (b'>', Some(b'=')) => (Tok::Ge, 2),
            (b'=', Some(b'=')) => (Tok::EqEq, 2),
            (b'!', Some(b'=')) => (Tok::Ne, 2),
            (b'&', Some(b'&')) => (Tok::And, 2),
            (b'|', Some(b'|')) => (Tok::Or, 2),
            (b'+', _) => (Tok::Plus, 1),
            (b'-', _) => (Tok::Minus, 1),
            (b'*', _) => (Tok::Star, 1),
            (b'/', _) => (Tok::Slash, 1),
            (b'%', _) => (Tok::Percent, 1),
            (b'<', _) => (Tok::Lt, 1),
            (b'>', _) => (Tok::Gt, 1),
            (b'!', _) => (Tok::Not, 1),
            (b'?', _) => (Tok::Question, 1),
            (b':', _) => (Tok::Colon, 1),
            (b'(', _) => (Tok::LParen, 1),
            (b')', _) => (Tok::RParen, 1),
            (b'{', _) => (Tok::LBrace, 1),
            (b'}', _) => (Tok::RBrace, 1),
            (b'[', _) => (Tok::LBracket, 1),
            (b']', _) => (Tok::RBracket, 1),
            (b',', _) => (Tok::Comma, 1),
            (b'=', _) => (Tok::Assign, 1),
            _ => {
                let ch = sub(src, i, b.len()).chars().next().unwrap_or('?');
                let end = i + ch.len_utf8();
                let hint = match ch {
                    '^' => " (there is no power operator: multiply, e.g. `x * x`)",
                    '&' | '|' => " (write `and` / `or`)",
                    '"' => " (there are no strings: for a character's code write it in single quotes, 'A')",
                    _ => "",
                };
                return Err(Error::new(
                    i,
                    end,
                    format!("I don't understand `{ch}`{hint}"),
                ));
            }
        };
        match tok {
            Tok::LParen | Tok::LBracket => depth += 1,
            Tok::RParen | Tok::RBracket => depth = depth.saturating_sub(1),
            _ => {}
        }
        i += n;
        out.push(Token { tok, start, end: i });
    }
    push_nl(&mut out, b.len());
    out.push(Token {
        tok: Tok::Eof,
        start: b.len(),
        end: b.len(),
    });
    Ok(out)
}

/// `sprite NAME {` and nothing else (but spaces and a comment) on the line
/// from `i`: where the `{` is.
fn sprite_head(b: &[u8], i: usize) -> Option<usize> {
    let word = |j: usize| j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'_');
    let space = |j: usize| j < b.len() && (b[j] == b' ' || b[j] == b'\t');
    if b.get(i..i + 6) != Some(b"sprite".as_slice()) || !space(i + 6) {
        return None;
    }
    let mut j = i + 6;
    while space(j) {
        j += 1;
    }
    if !(j < b.len() && (b[j].is_ascii_alphabetic() || b[j] == b'_')) {
        return None;
    }
    while word(j) {
        j += 1;
    }
    while space(j) {
        j += 1;
    }
    if b.get(j) != Some(&b'{') {
        return None;
    }
    let brace = j;
    j += 1;
    while space(j) {
        j += 1;
    }
    let comment = matches!(b.get(j..j + 2), Some(b"--") | Some(b"//"));
    (comment || j >= b.len() || b[j] == b'\n' || b[j] == b';').then_some(brace)
}

/// A sprite block from `i` (its `sprite`, the `{` at `brace`): `sprite`,
/// its name and `{` as tokens, then each line (to a newline or `;`, a
/// comment cut off, trimmed, blank ones left out) as a [`Tok::Raw`] up to a
/// line that is `}` alone, its [`Tok::RBrace`]. Where lexing goes on.
fn sprite_rows(src: &str, i: usize, brace: usize, out: &mut Vec<Token>) -> usize {
    let b = src.as_bytes();
    let mut name = i + 6;
    while b[name] == b' ' || b[name] == b'\t' {
        name += 1;
    }
    let mut e = name;
    while e < b.len() && (b[e].is_ascii_alphanumeric() || b[e] == b'_') {
        e += 1;
    }
    for (tok, s, e) in [
        (Tok::Name, i, i + 6),
        (Tok::Name, name, e),
        (Tok::LBrace, brace, brace + 1),
    ] {
        out.push(Token {
            tok,
            start: s,
            end: e,
        });
    }
    // To the end of the `{`'s line (a comment, a newline or `;`).
    let mut k = brace + 1;
    while k < b.len() && b[k] != b'\n' && b[k] != b';' {
        k += 1;
    }
    while k < b.len() {
        k += 1; // past the newline or `;`
        let start = k;
        let mut end = k;
        while k < b.len() && b[k] != b'\n' && b[k] != b';' {
            if matches!(b.get(k..k + 2), Some(b"--") | Some(b"//")) {
                // A comment, to the end of the line (as everywhere else).
                while k < b.len() && b[k] != b'\n' {
                    k += 1;
                }
                break;
            }
            k += 1;
            end = k;
        }
        let (mut s, mut e) = (start, end);
        while s < e && b[s].is_ascii_whitespace() {
            s += 1;
        }
        while e > s && b[e - 1].is_ascii_whitespace() {
            e -= 1;
        }
        if s == e {
            continue;
        }
        if &b[s..e] == b"}" {
            out.push(Token {
                tok: Tok::RBrace,
                start: s,
                end: e,
            });
            return e;
        }
        out.push(Token {
            tok: Tok::Raw,
            start: s,
            end: e,
        });
    }
    k.min(b.len())
}

/// The same formula on one line, as short as it stays readable: comments
/// dropped, runs of spaces made one, lines joined with `; ` (which the
/// language reads as a line break, and ignores inside brackets, as it does
/// a newline). For `@fx:psi?src=` paths, which are 512 B at most.
pub fn compact(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    for line in src.lines() {
        // Cut a comment (`--` or `//`) off the line.
        let b = line.as_bytes();
        let mut end = b.len();
        for i in 0..b.len().saturating_sub(1) {
            if (b[i] == b'-' && b[i + 1] == b'-') || (b[i] == b'/' && b[i + 1] == b'/') {
                end = i;
                break;
            }
        }
        let mut code = String::new();
        for w in sub(line, 0, end).split_whitespace() {
            if !code.is_empty() {
                code.push(' ');
            }
            code.push_str(w);
        }
        if code.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push_str("; ");
        }
        out.push_str(&code);
    }
    out
}

// --- names ----------------------------------------------------------------------

/// The built-in values, their level and what they mean (for errors).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Input {
    X,
    Y,
    Px,
    Py,
    I,
    T,
    Hour,
    Minute,
    Second,
    A,
    B,
    C,
    Ur,
    Ug,
    Ub,
    Ul,
    Ink,
    /// `seed` (a `loop forever` formula's: [`LATER`]-like, a name a formula
    /// sets is that name).
    Seed,
}

const INPUTS: [(&str, Input, &str); 17] = [
    ("x", Input::X, "the pixel's position across, 0 to 1"),
    (
        "y",
        Input::Y,
        "the pixel's height, 0 at the bottom to 1 at the top",
    ),
    ("px", Input::Px, "the pixel's column, 0 to 127"),
    ("py", Input::Py, "the pixel's row from the bottom, 0 to 31"),
    ("i", Input::I, "the pixel's number, 0 to 4095"),
    ("t", Input::T, "seconds into the loop"),
    ("hour", Input::Hour, "the hour, 0 to 23"),
    ("minute", Input::Minute, "the minute"),
    ("second", Input::Second, "the second"),
    ("a", Input::A, "slider a"),
    ("b", Input::B, "slider b"),
    ("c", Input::C, "slider c"),
    (
        "ur",
        Input::Ur,
        "the red of the picture under the formula, 0 to 1",
    ),
    (
        "ug",
        Input::Ug,
        "the green of the picture under the formula, 0 to 1",
    ),
    (
        "ub",
        Input::Ub,
        "the blue of the picture under the formula, 0 to 1",
    ),
    (
        "ul",
        Input::Ul,
        "the brightness of the picture under the formula, 0 to 1",
    ),
    (
        "ink",
        Input::Ink,
        "what `draw { }` put at the pixel, 0 to 1",
    ),
];

/// Values added once people had saved formulas (2026-10-26), about the
/// middle and from the top: `cx = (x − ½)·4`, `cy = y − ½`, `ty = 31 −
/// py`, `r = len(cx, cy)`, `angle = atan2(cy, cx)`, the steps those take
/// written out ([`Parser::centred`]). A name set with one of these is that
/// name ([`Parser::lookup_value`] finds the formula's names first), so a
/// saved `r = …` means what it meant.
const LATER: [&str; 5] = ["cx", "cy", "ty", "r", "angle"];

/// `loop forever`'s loop (Q16 seconds, as `loop N`'s): longer than any.
const FOREVER: i32 = i32::MAX;

/// [`Parser::centred`]'s steps: (op, a, b) over 0 `x`, 1 `y`, 2 `py`, 3
/// ½, 4 four, 5 31 and, from 6, the steps before.
const CENTRED: [(u8, u8, u8); 6] = [
    (code::SUB, 0, 3),   // 6: x - ½
    (code::MUL, 6, 4),   // 7: cx
    (code::SUB, 1, 3),   // 8: cy
    (code::SUB, 5, 2),   // 9: ty
    (code::LEN, 7, 8),   // 10: r
    (code::ATAN2, 8, 7), // 11: angle
];

/// `w` and `h`: constants.
const CONSTS: [(&str, i32); 2] = [("w", (W as i32) << 16), ("h", (H as i32) << 16)];

#[derive(Clone, Copy, PartialEq, Eq)]
enum F {
    Op(u8),
    Smoothstep,
    Fract,
    /// `digit`/`glyph`: three values and an optional font word.
    Glyph(u8),
    /// `pick(k, a, b, …)`.
    Pick,
}

/// Functions: name, arity, what, the op.
const FUNCS: [(&str, usize, F); 25] = [
    ("sin", 1, F::Op(code::SIN)),
    ("cos", 1, F::Op(code::COS)),
    ("tri", 1, F::Op(code::TRI)),
    ("saw", 1, F::Op(code::SAW)),
    ("sqr", 1, F::Op(code::SQR)),
    ("abs", 1, F::Op(code::ABS)),
    ("floor", 1, F::Op(code::FLOOR)),
    ("frac", 1, F::Op(code::FRAC)),
    ("fract", 1, F::Fract),
    ("sqrt", 1, F::Op(code::SQRT)),
    ("min", 2, F::Op(code::MIN)),
    ("max", 2, F::Op(code::MAX)),
    ("step", 2, F::Op(code::STEP)),
    ("hash", 2, F::Op(code::HASH)),
    ("len", 2, F::Op(code::LEN)),
    ("atan2", 2, F::Op(code::ATAN2)),
    ("clamp", 3, F::Op(code::CLAMP)),
    ("mix", 3, F::Op(code::MIX)),
    ("smoothstep", 3, F::Smoothstep),
    ("noise", 3, F::Op(code::NOISE)),
    ("select", 3, F::Op(code::SELECT)),
    ("bit", 2, F::Op(code::BIT)),
    ("digit", 3, F::Glyph(code::DIGIT)),
    ("glyph", 3, F::Glyph(code::GLYPH)),
    ("pick", 0, F::Pick),
];

/// Functions added once people had saved formulas (2026-10-26): a name
/// set with one of these still works as a value (`glyph = …; gray(glyph)`).
const LATER_FUNCS: [&str; 4] = ["bit", "digit", "glyph", "pick"];

/// The fonts `digit` and `glyph` take as a last value: core's 5x7 (the
/// panel's text and clock; 4 × 6 digits; the default), 4x6 (3 × 5 digits)
/// and the bold 3 × 5 digits (digits only). As `Tables::glyphs` takes them.
const FONTS: [(&str, u8); 3] = [
    ("normal", tables::NORMAL),
    ("small", tables::SMALL),
    ("bold", tables::BOLD),
];

const COLOURS: [(&str, usize, u8); 4] = [
    ("rgb", 3, code::OUT_RGB),
    ("hsv", 3, code::OUT_HSV),
    ("pal", 2, code::OUT_PAL),
    ("gray", 1, code::OUT_GRAY),
];

/// Palette names: core's order (fire, ice, chrono, amber, rainbow); `gray`
/// is not a table.
const PALETTES: [(&str, u8); 7] = [
    ("fire", 0),
    ("ice", 1),
    ("ocean", 1),
    ("chrono", 2),
    ("amber", 3),
    ("rainbow", 4),
    ("gray", 255),
];

const KEYWORDS: [&str; 8] = [
    "loop", "frame", "layer", "part", "use", "draw", "for", "table",
];

/// The draw stage's primitives: statements inside `draw { }`, and names a
/// formula may still set (`line = …`), as `ink` is a value it may.
const PRIMITIVES: [&str; 2] = ["line", "dot"];

/// How a layer goes onto what is below it (`layer add { … }`; `over` when
/// none is named).
const BLENDS: [(&str, u8); fixed::BLENDS] = [
    ("over", fixed::BLEND_OVER),
    ("add", fixed::BLEND_ADD),
    ("multiply", fixed::BLEND_MULTIPLY),
    ("screen", fixed::BLEND_SCREEN),
    ("max", fixed::BLEND_MAX),
    ("min", fixed::BLEND_MIN),
];

/// Optimal string alignment distance (edits, adjacent swaps count one),
/// in three rolling rows of bytes (it runs at the deepest point of an
/// error: a 25 × 25 table of words was 5 KB of stack there).
fn distance(a: &str, b: &str) -> usize {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() > 24 || b.len() > 24 {
        return usize::MAX;
    }
    // Row k of the table is r[k % 3]: rows i-2 and i-1 are all row i needs.
    let mut r: [[u8; 25]; 3] = [[0; 25]; 3];
    for (j, v) in r[0].iter_mut().enumerate().take(b.len() + 1) {
        *v = j as u8;
    }
    for i in 1..=a.len() {
        let (prev2, prev) = (r[(i + 1) % 3], r[(i - 1) % 3]);
        let mut cur = [0u8; 25];
        cur[0] = i as u8;
        for j in 1..=b.len() {
            let cost = u8::from(a[i - 1] != b[j - 1]);
            let mut v = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                v = v.min(prev2[j - 2] + 1);
            }
            cur[j] = v;
        }
        r[i % 3] = cur;
    }
    r[a.len() % 3][b.len()] as usize
}

#[cfg(test)]
pub(crate) fn distance_for_tests(a: &str, b: &str) -> usize {
    distance(a, b)
}

/// The closest of `cands` to `name`, if it is close enough to be a typo.
fn suggest<'a>(name: &str, cands: impl Iterator<Item = &'a str>) -> Option<&'a str> {
    let limit = if name.len() <= 3 {
        2
    } else {
        2 + name.len() / 6
    };
    let mut best: Option<(usize, &str)> = None;
    for c in cands {
        let d = distance(name, c);
        if d <= limit && d < c.len().max(name.len()) && best.is_none_or(|(bd, _)| d < bd) {
            best = Some((d, c));
        }
    }
    best.map(|(_, c)| c)
}

fn did_you_mean(s: Option<&str>) -> String {
    s.map_or(String::new(), |s| format!(" — did you mean `{s}`?"))
}

// --- the expression DAG ---------------------------------------------------------

type Id = u16;

const LV_CONST: u8 = 0;
const LV_FRAME: u8 = 1;
/// Inside a `for` of the draw stage (`LV_LOOP2`: one inside another): a
/// value that depends on its counter, worked out each turn.
const LV_LOOP1: u8 = 2;
const LV_LOOP2: u8 = 3;
const LV_ROW: u8 = 4;
const LV_PIXEL: u8 = 5;

const K_CONST: u8 = 200;
const K_INPUT: u8 = 201;
/// A `for` loop's counter (`val`: the loop's number), given a scalar slot
/// when the loop is laid out.
const K_COUNTER: u8 = 202;
/// A value that grows without end plus a small one that varies more than
/// once a frame (`t*0.15 + x`: `a` the wide part, `b` the small one), at
/// the small one's level. Never laid out: whatever takes it is rewritten
/// ([`Dag::wide_op`]) into the wide part's way back to a Q16.16 and the
/// small one (`noise(w + s)` is a `NOISE_O` of `frac(w) + s`).
const K_SPLIT: u8 = 203;
/// A `K_INPUT`'s `val` for `t` in a `loop forever` formula: wide slot 0.
const WIDE_T: i32 = 0x100;

/// E901: a value that grows without end where it can't go.
const GROWS: &str =
    "a value that grows without end (`t` in a `loop forever` formula) can't go in here: \
use frac(t*k), t % n, sin(t*k), noise(t*k, y, z), a comparison, min, max or smoothstep, \
or add, multiply or divide it by values that are the same for every pixel";

#[derive(Clone, Copy, PartialEq, Eq)]
struct Node {
    /// An op code, `K_CONST` (value in `val`) or `K_INPUT` (slot or lane in
    /// `val`).
    code: u8,
    a: Id,
    b: Id,
    c: Id,
    val: i32,
    level: u8,
    /// The input that made this node vary as much as it does (errors).
    why: u8,
}

struct Dag {
    nodes: Vec<Node>,
    /// Open-addressing index of `nodes` for sharing ([`Dag::push`]):
    /// [`SLOTS`] ids, `NO` where empty. A linear scan made compiling a long
    /// formula quadratic (~0.2-0.35 ms on the host, tens of ms on the
    /// ESP32, per live edit).
    index: Vec<Id>,
    /// The multiplies that per-pixel divisions by a constant became, as
    /// (the multiply, the dividend, the divisor): a `floor` of one is
    /// rebuilt with 1/k rounded up ([`fixed::recip_up`]).
    divs: Vec<(Id, Id, i32)>,
    /// Each `pick`'s choices (a `PICK` node's `b` is its index here).
    picks: Vec<Vec<Id>>,
    /// A `loop forever` formula: `t` is wide ([`WIDE_T`]).
    forever: bool,
    /// The wide noises' contexts: each axis's wide part (`NO`: none), the
    /// context `NOISE_O + k` of a `NOISE_O` node.
    ctx: Vec<[Id; 3]>,
    /// The steps a node needs before it that it doesn't read as a value: a
    /// `NOISE_O` node's (`val` the index here) `W_LAT`s.
    deps: Vec<Vec<Id>>,
}

/// Index slots: a power of two, twice [`MAX_NODES`] (4 KB, freed with the
/// rest when `compile_into` returns).
const SLOTS: usize = 2 * MAX_NODES;
const NO: Id = Id::MAX;

impl Node {
    fn hash(&self) -> usize {
        let mut h: u32 = 0x811c_9dc5;
        for v in [
            self.code as u32,
            self.a as u32,
            self.b as u32,
            self.c as u32,
            self.val as u32,
            (self.level as u32) << 8 | self.why as u32,
        ] {
            h = (h ^ v).wrapping_mul(0x0100_0193);
        }
        (h ^ (h >> 15)) as usize
    }
}

impl Dag {
    fn level(&self, id: Id) -> u8 {
        self.nodes[id as usize].level
    }

    #[inline(never)]
    fn konst(&self, id: Id) -> Option<i32> {
        let n = &self.nodes[id as usize];
        (n.code == K_CONST).then_some(n.val)
    }

    #[inline(never)]
    fn push(&mut self, n: Node, at: (usize, usize)) -> Result<Id, Error> {
        let mut slot = n.hash() & (SLOTS - 1);
        loop {
            match self.index[slot] {
                NO => break,
                i if self.nodes[i as usize] == n => return Ok(i),
                _ => slot = (slot + 1) & (SLOTS - 1),
            }
        }
        if self.nodes.len() >= MAX_NODES {
            return Err(Error::new(
                at.0,
                at.1,
                "this formula is too long for the panel — simplify it",
            ));
        }
        self.nodes.push(n);
        let id = (self.nodes.len() - 1) as Id;
        self.index[slot] = id;
        Ok(id)
    }

    #[inline(never)]
    fn constant(&mut self, v: i32, at: (usize, usize)) -> Result<Id, Error> {
        self.push(
            Node {
                code: K_CONST,
                a: 0,
                b: 0,
                c: 0,
                val: v,
                level: LV_CONST,
                why: 0,
            },
            at,
        )
    }

    #[inline(never)]
    fn input(&mut self, inp: Input, at: (usize, usize)) -> Result<Id, Error> {
        let why = INPUTS.iter().position(|e| e.1 == inp).unwrap_or(0) as u8;
        // What is under the formula: loaded into a lane by a step of its own.
        let under = match inp {
            Input::Ur => Some(code::U_R),
            Input::Ug => Some(code::U_G),
            Input::Ub => Some(code::U_B),
            Input::Ul => Some(code::U_L),
            Input::Ink => Some(code::INK),
            _ => None,
        };
        if let Some(c) = under {
            return self.push(
                Node {
                    code: c,
                    a: 0,
                    b: 0,
                    c: 0,
                    val: 0,
                    level: LV_PIXEL,
                    why,
                },
                at,
            );
        }
        let (val, level) = match inp {
            Input::T if self.forever => (WIDE_T, LV_FRAME),
            Input::Seed => (S_SEED as i32, LV_FRAME),
            Input::X => ((LANE | L_X) as i32, LV_PIXEL),
            Input::Px => ((LANE | L_PX) as i32, LV_PIXEL),
            Input::I => ((LANE | L_I) as i32, LV_PIXEL),
            Input::Y => (S_Y as i32, LV_ROW),
            Input::Py => (S_PY as i32, LV_ROW),
            Input::T => (S_T as i32, LV_FRAME),
            Input::Hour => (S_HOUR as i32, LV_FRAME),
            Input::Minute => (S_MINUTE as i32, LV_FRAME),
            Input::Second => (S_SECOND as i32, LV_FRAME),
            Input::A => (S_A as i32, LV_FRAME),
            Input::B => (S_B as i32, LV_FRAME),
            Input::C => (S_C as i32, LV_FRAME),
            Input::Ur | Input::Ug | Input::Ub | Input::Ul | Input::Ink => (0, LV_PIXEL),
        };
        self.push(
            Node {
                code: K_INPUT,
                a: 0,
                b: 0,
                c: 0,
                val,
                level,
                why,
            },
            at,
        )
    }

    /// An op node: folded when its operands are constants, simplified
    /// where that is exact, shared when it already exists.
    #[inline(never)]
    fn op(&mut self, c: u8, args: [Id; 3], at: (usize, usize)) -> Result<Id, Error> {
        let n = code::arity(c);
        if self.forever && args[..n].iter().any(|&x| self.grows(x)) {
            return self.wide_op(c, args, at);
        }
        let ks: [Option<i32>; 3] =
            core::array::from_fn(|i| if i < n { self.konst(args[i]) } else { Some(0) });
        // Fold (all but the table functions, which run once a frame).
        if ks.iter().all(Option::is_some) && !code::is_table(c) {
            let v = eval(
                c,
                ks[0].unwrap_or(0),
                ks[1].unwrap_or(0),
                ks[2].unwrap_or(0),
                None,
            );
            return self.constant(v, at);
        }
        let (a, b) = (args[0], args[1]);
        match (c, ks[0], ks[1]) {
            (code::ADD, _, Some(0)) | (code::SUB, _, Some(0)) | (code::MUL, _, Some(ONE)) => {
                return Ok(a)
            }
            (code::ADD, Some(0), _) | (code::MUL, Some(ONE), _) => return Ok(b),
            (code::MUL, Some(0), _) | (code::MUL, _, Some(0)) => return self.constant(0, at),
            // x / k, x per pixel: x · (1/k), exact for powers of two. Once a
            // frame or a row it stays a true division (exact, and it costs
            // nothing per pixel), so `floor(minute/10)` is 5 at 50.
            (code::DIV, None, Some(k)) if k != 0 && self.level(a) == LV_PIXEL => {
                let r = self.constant(fixed::recip(k), at)?;
                let m = self.op(code::MUL, [a, r, 0], at)?;
                if self.konst(m).is_none() && !self.divs.iter().any(|d| d.0 == m) {
                    self.divs.push((m, a, k));
                }
                return Ok(m);
            }
            // floor(x / k) per pixel: 1/k rounded up instead, so a whole
            // number over a whole number that divides it floors to exactly
            // that (rounded down, floor(px/10) was 1 at px = 20).
            (code::FLOOR, None, _) if self.divs.iter().any(|d| d.0 == a) => {
                let &(_, x, k) = self.divs.iter().find(|d| d.0 == a).unwrap();
                let r = self.constant(fixed::recip_up(k), at)?;
                let m = self.op(code::MUL, [x, r, 0], at)?;
                if m != a {
                    return self.op(code::FLOOR, [m, 0, 0], at);
                }
            }
            // x / s, s per frame or per row: one division there, a multiply
            // per pixel.
            (code::DIV, None, None) if self.level(a) == LV_PIXEL && self.level(b) < LV_PIXEL => {
                let r = self.op(code::RECIP, [b, 0, 0], at)?;
                return self.op(code::MUL, [a, r, 0], at);
            }
            // x % 2^k: a mask.
            (code::MOD, None, Some(k)) if k > 0 && (k as u32).is_power_of_two() => {
                let m = self.constant(k - 1, at)?;
                return self.op(code::MASK, [a, m, 0], at);
            }
            _ => {}
        }
        let mut level = LV_CONST;
        let mut why = 0;
        for &x in &args[..n] {
            let m = &self.nodes[x as usize];
            if m.level > level {
                level = m.level;
                why = m.why;
            }
        }
        if level == LV_CONST {
            // A table (sin, a font) read at a constant: once a frame.
            level = LV_FRAME;
        }
        let mut args = args;
        for x in &mut args[n..] {
            *x = 0;
        }
        self.push(
            Node {
                code: c,
                a: args[0],
                b: args[1],
                c: args[2],
                val: 0,
                level,
                why,
            },
            at,
        )
    }
}

impl Dag {
    /// `NAME[k]`, the table of `len` entries at `start` of `tables`: a
    /// constant `k` is its entry (or 0), any other a `TAB` step at `k`'s
    /// level.
    #[inline(never)]
    fn tab(
        &mut self,
        k: Id,
        start: usize,
        len: usize,
        tables: &[i32],
        at: (usize, usize),
    ) -> Result<Id, Error> {
        if self.grows(k) {
            return Err(Error::new(at.0, at.1, GROWS));
        }
        if let Some(v) = self.konst(k) {
            let i = v >> 16;
            let e = if (0..len as i32).contains(&i) {
                tables.get(start + i as usize).copied().unwrap_or(0)
            } else {
                0
            };
            return self.constant(e, at);
        }
        let m = self.nodes[k as usize];
        self.push(
            Node {
                code: code::TAB,
                a: k,
                b: start as Id,
                c: (len - 1) as Id,
                val: 0,
                level: m.level,
                why: m.why,
            },
            at,
        )
    }

    /// `pick(k, choices…)`: the choice floor(k) counts to from 0, or 0. A
    /// constant `k` is that choice; a per-pixel `k` of choices that are the
    /// same for every pixel, one `PICK` step; otherwise compares and
    /// selects, `k < 0 ? 0 : k < 1 ? a : … : k < n ? z : 0`.
    #[inline(never)]
    fn pick(&mut self, k: Id, opts: Vec<Id>, at: (usize, usize)) -> Result<Id, Error> {
        if opts.iter().any(|&o| self.grows(o)) || (self.grows(k) && !self.is_wide(k)) {
            return Err(Error::new(at.0, at.1, GROWS));
        }
        let zero = self.constant(0, at)?;
        let n = opts.len() as i32;
        if let Some(v) = self.konst(k) {
            let i = v >> 16;
            return Ok(if (0..n).contains(&i) {
                opts[i as usize]
            } else {
                zero
            });
        }
        if self.level(k) < LV_PIXEL || opts.iter().any(|&o| self.level(o) == LV_PIXEL) {
            let mut acc = zero;
            for i in (-1..n).rev() {
                let o = if i < 0 { zero } else { opts[i as usize] };
                let c = self.constant((i + 1) << 16, at)?;
                let c = self.op(code::LT, [k, c, 0], at)?;
                acc = self.op(code::SELECT, [c, o, acc], at)?;
            }
            return Ok(acc);
        }
        // The same choices, the same table (a colour expression reads a
        // pick three times: one node, one step).
        let b = match self.picks.iter().position(|p| *p == opts) {
            Some(j) => j,
            None => {
                self.picks.push(opts);
                self.picks.len() - 1
            }
        };
        let why = self.nodes[k as usize].why;
        self.push(
            Node {
                code: code::PICK,
                a: k,
                b: b as Id,
                c: 0,
                val: 0,
                level: LV_PIXEL,
                why,
            },
            at,
        )
    }
}

/// The compares the other way round (`a < b` is `b > a`).
fn flip(c: u8) -> u8 {
    match c {
        code::LT => code::GT,
        code::GT => code::LT,
        code::LE => code::GE,
        code::GE => code::LE,
        c => c,
    }
}

impl Dag {
    /// A value that grows without end, worked out once a frame: `t` in a
    /// `loop forever` formula and what `W_*` steps make of it.
    #[inline(never)]
    fn is_wide(&self, id: Id) -> bool {
        let n = &self.nodes[id as usize];
        (n.code == K_INPUT && n.val == WIDE_T)
            || (code::is_wide(n.code) && code::wide_ops(n.code).0)
    }

    /// A wide value or one with a wide part ([`K_SPLIT`]).
    #[inline(never)]
    fn grows(&self, id: Id) -> bool {
        self.is_wide(id) || self.nodes[id as usize].code == K_SPLIT
    }

    /// A value the same all frame that doesn't grow: what a wide step may
    /// take as its scalar operand.
    #[inline(never)]
    fn per_frame(&self, id: Id) -> bool {
        !self.grows(id) && self.level(id) <= LV_FRAME
    }

    /// A value's wide part and its small part (either may be missing).
    #[inline(never)]
    fn parts(&self, id: Id) -> (Option<Id>, Option<Id>) {
        let n = &self.nodes[id as usize];
        if n.code == K_SPLIT {
            (Some(n.a), Some(n.b))
        } else if self.is_wide(id) {
            (Some(id), None)
        } else {
            (None, Some(id))
        }
    }

    /// A wide step's node (`imm`: a compare's or min/max's kind in `c`;
    /// `val`: a `W_LAT`'s offset).
    #[inline(never)]
    fn wnode(
        &mut self,
        c: u8,
        a: Id,
        b: Id,
        imm: u8,
        val: i32,
        at: (usize, usize),
    ) -> Result<Id, Error> {
        let n = code::wide_ops(c).1;
        // w ± 0, w · 1, w / 1: w (exactly).
        match (c, self.konst(b)) {
            (code::W_ADDS | code::W_SUBS, Some(0)) | (code::W_MULS | code::W_DIVS, Some(ONE)) => {
                return Ok(a)
            }
            _ => {}
        }
        let (mut level, mut why) = (LV_FRAME, 0);
        for &x in &[a, b][..n] {
            let m = &self.nodes[x as usize];
            if m.level > level {
                (level, why) = (m.level, m.why);
            } else if m.level == level && why == 0 {
                why = m.why;
            }
        }
        self.push(
            Node {
                code: c,
                a,
                b: if n > 1 { b } else { 0 },
                c: imm as Id,
                val,
                level,
                why,
            },
            at,
        )
    }

    /// A wide part plus a small one: a wide value when the small one is the
    /// same all frame, else a [`K_SPLIT`].
    #[inline(never)]
    fn make(&mut self, w: Option<Id>, s: Option<Id>, at: (usize, usize)) -> Result<Id, Error> {
        match (w, s) {
            (Some(w), None) => Ok(w),
            (None, Some(s)) => Ok(s),
            (None, None) => self.constant(0, at),
            (Some(w), Some(s)) if self.per_frame(s) => self.wnode(code::W_ADDS, w, s, 0, 0, at),
            (Some(w), Some(s)) => {
                let m = self.nodes[s as usize];
                self.push(
                    Node {
                        code: K_SPLIT,
                        a: w,
                        b: s,
                        c: 0,
                        val: 0,
                        level: m.level,
                        why: m.why,
                    },
                    at,
                )
            }
        }
    }

    /// The value's fraction-preserving small form: frac(w) for a wide value,
    /// frac(w) + s for a split (`sin`, `frac`, `noise` read only that much
    /// of the wide part: exactly, as long as the sum fits).
    #[inline(never)]
    fn small(&mut self, id: Id, at: (usize, usize)) -> Result<Id, Error> {
        match self.parts(id) {
            (Some(w), None) => self.wnode(code::W_FRAC, w, 0, 0, 0, at),
            (Some(w), Some(s)) => {
                let f = self.wnode(code::W_FRAC, w, 0, 0, 0, at)?;
                self.op(code::ADD, [f, s, 0], at)
            }
            (None, Some(s)) => Ok(s),
            (None, None) => self.constant(0, at),
        }
    }

    /// A wide value held to a Q16.16 (saturated).
    #[inline(never)]
    fn held(&mut self, w: Id, at: (usize, usize)) -> Result<Id, Error> {
        let lo = self.constant(i32::MIN, at)?;
        self.wnode(code::W_MINMAX, w, lo, code::MAX, 0, at)
    }

    /// A wide value (never a split) or E901.
    #[inline(never)]
    fn wide_only(&self, id: Id, at: (usize, usize)) -> Result<Id, Error> {
        if self.is_wide(id) {
            Ok(id)
        } else {
            Err(Error::new(at.0, at.1, GROWS))
        }
    }

    /// Op `c` of operands at least one of which grows without end (`t` in a
    /// `loop forever` formula): rewritten into wide steps once a frame and
    /// the ways back to a Q16.16, each exactly what the Q16.16 operation
    /// gives wherever the values fit one (`crate::fixed`'s `w_*`); E901
    /// where there is no such way (a growing value per pixel, `t*t`,
    /// `sqrt(t)`, …).
    #[inline(never)]
    fn wide_op(&mut self, c: u8, args: [Id; 3], at: (usize, usize)) -> Result<Id, Error> {
        use code::*;
        let [a, b, k3] = args;
        let grows = |d: &Self, x: Id| d.grows(x);
        let bad = || Err(Error::new(at.0, at.1, GROWS));
        match c {
            ADD | SUB => {
                let ((wa, sa), (wb, sb)) = (self.parts(a), self.parts(b));
                // w - s and s - w as they are (no negation that saturates).
                if c == SUB {
                    if let (Some(w), None, None, Some(s)) = (wa, sa, wb, sb) {
                        return if self.per_frame(s) {
                            self.wnode(W_SUBS, w, s, 0, 0, at)
                        } else {
                            let n = self.op(NEG, [s, 0, 0], at)?;
                            self.make(Some(w), Some(n), at)
                        };
                    }
                }
                let w = match (wa, wb) {
                    (Some(x), Some(y)) => {
                        Some(self.wnode(if c == ADD { W_ADD } else { W_SUB }, x, y, 0, 0, at)?)
                    }
                    (Some(x), None) => Some(x),
                    (None, Some(y)) if c == ADD => Some(y),
                    (None, Some(y)) => Some(self.wnode(W_NEG, y, 0, 0, 0, at)?),
                    (None, None) => None,
                };
                let s = match (sa, sb) {
                    (Some(x), Some(y)) => Some(self.op(c, [x, y, 0], at)?),
                    (Some(x), None) => Some(x),
                    (None, Some(y)) if c == ADD => Some(y),
                    (None, Some(y)) => Some(self.op(NEG, [y, 0, 0], at)?),
                    (None, None) => None,
                };
                self.make(w, s, at)
            }
            NEG => {
                let (w, s) = self.parts(a);
                let w = match w {
                    Some(w) => Some(self.wnode(W_NEG, w, 0, 0, 0, at)?),
                    None => None,
                };
                let s = match s {
                    Some(s) => Some(self.op(NEG, [s, 0, 0], at)?),
                    None => None,
                };
                self.make(w, s, at)
            }
            MUL | DIV => {
                let (g, k) = if grows(self, a) { (a, b) } else { (b, a) };
                if !self.per_frame(k) || (c == DIV && g != a) {
                    return bad();
                }
                let (w, s) = self.parts(g);
                let w = match w {
                    Some(w) => {
                        Some(self.wnode(if c == MUL { W_MULS } else { W_DIVS }, w, k, 0, 0, at)?)
                    }
                    None => None,
                };
                let s = match s {
                    Some(s) => Some(self.op(c, [s, k, 0], at)?),
                    None => None,
                };
                self.make(w, s, at)
            }
            MOD => {
                if grows(self, b) || !self.per_frame(b) {
                    return bad();
                }
                match self.parts(a) {
                    (Some(w), None) => self.wnode(W_MOD, w, b, 0, 0, at),
                    // (w + s) % n = (w % n + s) % n.
                    (Some(w), Some(s)) => {
                        let m = self.wnode(W_MOD, w, b, 0, 0, at)?;
                        let x = self.op(ADD, [m, s, 0], at)?;
                        self.op(MOD, [x, b, 0], at)
                    }
                    _ => bad(),
                }
            }
            FRAC | SIN | COS | TRI | SAW | SQR => {
                let x = self.small(a, at)?;
                if c == FRAC && self.parts(a).1.is_none() {
                    return Ok(x);
                }
                self.op(c, [x, 0, 0], at)
            }
            FLOOR => {
                let w = self.wide_only(a, at)?;
                self.wnode(W_FLOOR, w, 0, 0, 0, at)
            }
            LT..=NE => {
                let (x, y, cc) = if grows(self, a) {
                    (a, b, c)
                } else {
                    (b, a, flip(c))
                };
                let x = self.wide_only(x, at)?;
                if self.is_wide(y) {
                    let d = self.wnode(W_SUB, x, y, 0, 0, at)?;
                    let z = self.constant(0, at)?;
                    return self.wnode(W_CMP, d, z, cc, 0, at);
                }
                if !self.per_frame(y) {
                    return bad();
                }
                self.wnode(W_CMP, x, y, cc, 0, at)
            }
            // step(e, x) is x >= e.
            STEP => self.op(GE, [b, a, 0], at),
            NOT => {
                let z = self.constant(0, at)?;
                self.op(EQ, [a, z, 0], at)
            }
            AND | OR => {
                let z = self.constant(0, at)?;
                let mut v = [a, b];
                for x in &mut v {
                    if grows(self, *x) {
                        let w = self.wide_only(*x, at)?;
                        *x = self.wnode(W_CMP, w, z, NE, 0, at)?;
                    }
                }
                self.op(c, [v[0], v[1], 0], at)
            }
            // min, max and clamp of a wide value and any other: of it held
            // to a Q16.16 (the same: the result is held so anyway), or in
            // one step with a value the same all frame.
            MIN | MAX => {
                let (x, y) = if grows(self, a) { (a, b) } else { (b, a) };
                let x = self.wide_only(x, at)?;
                if grows(self, y) {
                    return bad();
                }
                if self.per_frame(y) {
                    return self.wnode(W_MINMAX, x, y, c, 0, at);
                }
                let h = self.held(x, at)?;
                self.op(c, [h, y, 0], at)
            }
            CLAMP => {
                let x = self.wide_only(a, at)?;
                if grows(self, b) || grows(self, k3) {
                    return bad();
                }
                let m = if self.per_frame(b) {
                    self.wnode(W_MINMAX, x, b, MAX, 0, at)?
                } else {
                    let h = self.held(x, at)?;
                    self.op(MAX, [h, b, 0], at)?
                };
                self.op(MIN, [m, k3, 0], at)
            }
            // mix(a, b, t) = a + (b - a)·t, a and b the same all frame.
            MIX => {
                let t = self.wide_only(k3, at)?;
                if !self.per_frame(a) || !self.per_frame(b) {
                    return bad();
                }
                let d = self.op(SUB, [b, a, 0], at)?;
                let m = self.wnode(W_MULS, t, d, 0, 0, at)?;
                self.wnode(W_ADDS, m, a, 0, 0, at)
            }
            // smoothstep's smooth(x, e0, 1/(e1 - e0)): its clamped ramp
            // worked out wide, then the curve of that.
            SMOOTH => {
                let x = self.wide_only(a, at)?;
                if !self.per_frame(b) || !self.per_frame(k3) {
                    return bad();
                }
                let d = self.wnode(W_SUBS, x, b, 0, 0, at)?;
                let m = self.wnode(W_MULS, d, k3, 0, 0, at)?;
                let (z, one) = (self.constant(0, at)?, self.constant(ONE, at)?);
                let lo = self.wnode(W_MINMAX, m, z, MAX, 0, at)?;
                let r = self.op(MIN, [lo, one, 0], at)?;
                self.op(SMOOTH, [r, z, one], at)
            }
            SELECT => {
                if grows(self, b) || grows(self, k3) {
                    return bad();
                }
                let w = self.wide_only(a, at)?;
                let z = self.constant(0, at)?;
                let cnd = self.wnode(W_CMP, w, z, NE, 0, at)?;
                self.op(SELECT, [cnd, b, k3], at)
            }
            HASH => {
                let mut v = [a, b];
                for x in &mut v {
                    if grows(self, *x) {
                        let w = self.wide_only(*x, at)?;
                        *x = self.wnode(W_RED, w, 0, 0, 0, at)?;
                    }
                }
                self.op(HASH, [v[0], v[1], 0], at)
            }
            NOISE => self.noise_o(args, at),
            _ => bad(),
        }
    }

    /// `noise(x, y, z)` with an axis that grows without end: a `NOISE_O`
    /// of the axes' small forms ([`Dag::small`]) in a context whose
    /// lattice offsets `W_LAT` steps set from the wide parts once a frame.
    #[inline(never)]
    fn noise_o(&mut self, args: [Id; 3], at: (usize, usize)) -> Result<Id, Error> {
        // Once a frame, only x wide (`noise(u, 5, 0)`): one step, no
        // context.
        let [x, y, z] = args;
        if self.is_wide(x) && self.per_frame(y) && self.per_frame(z) {
            let m = self.nodes[x as usize];
            return self.push(
                Node {
                    code: code::W_NOISEX,
                    a: x,
                    b: y,
                    c: z,
                    val: 0,
                    level: LV_FRAME,
                    why: m.why,
                },
                at,
            );
        }
        let mut key = [NO; 3];
        let mut small = [0 as Id; 3];
        for k in 0..3 {
            key[k] = self.parts(args[k]).0.unwrap_or(NO);
            small[k] = self.small(args[k], at)?;
        }
        let ctx = match self.ctx.iter().position(|c| *c == key) {
            Some(j) => j,
            None if self.ctx.len() < NOISE_CTX => {
                self.ctx.push(key);
                self.ctx.len() - 1
            }
            None => {
                return Err(Error::new(
                    at.0,
                    at.1,
                    "noise of values that grow without end: four different ones at most — share one, as noise(u + k, …)",
                ))
            }
        };
        let mut lats = Vec::new();
        for (axis, &w) in key.iter().enumerate() {
            if w != NO {
                lats.push(self.wnode(code::W_LAT, w, 0, 0, (ctx * 3 + axis) as i32, at)?);
            }
        }
        let di = match self.deps.iter().position(|d| *d == lats) {
            Some(j) => j,
            None => {
                self.deps.push(lats);
                self.deps.len() - 1
            }
        };
        let (mut level, mut why) = (LV_FRAME, 0);
        for &x in &small {
            let m = &self.nodes[x as usize];
            if m.level > level {
                (level, why) = (m.level, m.why);
            }
        }
        self.push(
            Node {
                code: code::NOISE_O + ctx as u8,
                a: small[0],
                b: small[1],
                c: small[2],
                val: di as i32,
                level,
                why,
            },
            at,
        )
    }
}

// --- the parser -------------------------------------------------------------------

/// Tokens read again for parts drawn at other places (`p(x', y')`, each
/// reading all the part's text, parts in it included), in all: bounds
/// compile time whatever the nesting.
const MAX_WORK: usize = 1536;
/// What a part's level of nesting counts against [`MAX_DEPTH`] (with the
/// brackets and operators around it): bounds the parser's stack.
const PART_DEPTH: usize = 3;

struct Local<'s> {
    name: &'s str,
    id: Id,
    /// Where its name is (the line is worked out only for an error: per
    /// local it made a long formula quadratic).
    at: usize,
    in_frame: bool,
    /// A `for` loop's counter (it can't be set).
    counter: bool,
    /// The scope it was set in ([`Parser::scopes`]).
    scope: u16,
}

/// `table NAME = …`: its entries are `Parser::tables[start..start + len]`.
struct Table<'s> {
    name: &'s str,
    at: usize,
    scope: u16,
    start: usize,
    len: usize,
}

/// `sprite NAME { … }`: its header is `Parser::tables[start]`.
struct Sprite<'s> {
    name: &'s str,
    at: usize,
    scope: u16,
    start: usize,
}

/// The characters a sprite's key may use: printable, not a space, nor
/// what would end its line or start a comment (`; { } , - /`).
fn key_char(c: u8) -> bool {
    c.is_ascii_graphic() && !matches!(c, b';' | b'{' | b'}' | b',' | b'-' | b'/')
}

/// The draw stage as written, in order: loops (their counter node and
/// count) around lines and dots (their values: x0, y0, x1, y1, v0, v1; x,
/// y, v).
#[derive(Clone, Copy)]
enum DItem {
    For(Id, u16),
    End,
    Line([Id; 6]),
    Dot([Id; 3]),
}

/// `part name { … }`: another formula inside this one, pasted in, with
/// names of its own. Its tokens (`body`, just after `{`, to `end`, its `}`)
/// are read where it is written and again for each place it is drawn at.
struct Part<'s> {
    name: &'s str,
    at: usize,
    scope: u16,
    body: usize,
    end: usize,
    /// Its own loop (Q16 seconds; 8 when it sets none, as alone).
    loop_v: i32,
    insts: Vec<Inst>,
}

/// A part read at one place (`None`: where the pixel is): its scope (the
/// values it sets, `p.v`, are the locals set there), its colours and
/// whether they are layers.
struct Inst {
    at: Option<(Id, Id)>,
    scope: u16,
    colours: Vec<Colour>,
    layered: bool,
}

/// What `x y px py i t` mean where the parser is: inside a part drawn
/// elsewhere, the place it is drawn at, and its own time.
#[derive(Clone, Copy)]
struct Env {
    at: Option<(Id, Id)>,
    t: Option<Id>,
    loop_v: i32,
    /// In a colour expression, the channel being read (0 red, 1 green, 2
    /// blue: `#f80` is that channel's value, `rgb(r, g, b)` that value);
    /// `NO_CH` elsewhere.
    ch: u8,
}

const NO_CH: u8 = 3;
/// Inside `rgb(r, g, b)`'s values (for the error a colour there gets).
const IN_RGB: u8 = 4;

struct Parser<'s> {
    src: &'s str,
    toks: Vec<Token>,
    pos: usize,
    dag: Dag,
    locals: Vec<Local<'s>>,
    depth: usize,
    /// The scope being read, and every scope's parent and whether it is a
    /// part's own (which sees nothing outside it). 0 is the formula's; a
    /// layer's sees the names around it.
    scope: u16,
    scopes: Vec<(u16, bool)>,
    parts: Vec<Part<'s>>,
    env: Env,
    work: usize,
    /// `table` entries (Q16.16; the same run of numbers is stored once) and
    /// the tables' names.
    tables: Vec<i32>,
    table_names: Vec<Table<'s>>,
    sprites: Vec<Sprite<'s>>,
    /// The draw stage: its items, the `draw` that wrote them, whether the
    /// lines being read are recorded (not for a part read again at another
    /// place: its draw stage is the one read where it is written), how deep
    /// inside `draw { }` the parser is (0 outside, 1 its own lines, 2 and 3
    /// in loops), the loops' counts around it, loops numbered so far, and
    /// loop turns a frame.
    draw: Vec<DItem>,
    drawn: Option<Token>,
    record: bool,
    in_draw: u8,
    counts: [u32; 2],
    loops: u16,
    turns: u32,
}

/// A colour: an `OUT_*` code, its arguments, the palette, how much of the
/// pixel it covers (`None`: all of it) and how it goes onto what is below.
#[derive(Clone, Copy)]
struct Colour {
    code: u8,
    args: [Id; 3],
    pal: u8,
    alpha: Option<Id>,
    blend: u8,
    /// `pal(P, v) * k`: the palette's colour times k (each channel).
    scale: Option<Id>,
}

/// What a formula's or a part's lines come to: its colours, bottom to top
/// (the last line's, or its layers'), the line that made the top one,
/// whether they are layers, `layer` blocks written, and its `loop`.
struct Body {
    colours: Vec<Colour>,
    top: Option<Token>,
    layered: bool,
    blocks: usize,
    loop_v: Option<i32>,
    /// `fps N`: the frame time it asks for, ms (10, 20 or 40).
    fps: Option<u8>,
}

const BP_TERNARY: u8 = 1;
const BP_UNARY: u8 = 11;
const BP_NOT: u8 = 4;

fn infix_bp(t: Tok) -> Option<(u8, u8, u8)> {
    // (left bp, right bp, op)
    Some(match t {
        Tok::Or => (2, 3, code::OR),
        Tok::And => (3, 4, code::AND),
        Tok::Lt => (5, 6, code::LT),
        Tok::Le => (5, 6, code::LE),
        Tok::Gt => (5, 6, code::GT),
        Tok::Ge => (5, 6, code::GE),
        Tok::EqEq => (5, 6, code::EQ),
        Tok::Ne => (5, 6, code::NE),
        Tok::Plus => (7, 8, code::ADD),
        Tok::Minus => (7, 8, code::SUB),
        Tok::Star => (9, 10, code::MUL),
        Tok::Slash => (9, 10, code::DIV),
        Tok::Percent => (9, 10, code::MOD),
        _ => return None,
    })
}

impl<'s> Parser<'s> {
    /// Token `k`, or the closing `Eof` past the end (a look ahead that runs
    /// off the input never panics).
    #[inline(never)]
    fn tok(&self, k: usize) -> Token {
        self.toks
            .get(k)
            .copied()
            .unwrap_or(self.toks[self.toks.len() - 1])
    }

    #[inline(never)]
    fn peek(&self) -> Token {
        self.toks[self.pos]
    }

    #[inline(never)]
    fn next(&mut self) -> Token {
        let t = self.toks[self.pos];
        if self.pos + 1 < self.toks.len() {
            self.pos += 1;
        }
        t
    }

    fn text(&self, t: Token) -> &'s str {
        sub(self.src, t.start, t.end)
    }

    #[inline(never)]
    fn line_of(&self, at: usize) -> usize {
        sub(self.src, 0, at).matches('\n').count() + 1
    }

    #[cold]
    #[inline(never)]
    fn err<T>(&self, t: Token, msg: impl Into<String>) -> Result<T, Error> {
        Err(Error::new(t.start, t.end, msg))
    }

    // The errors are built out of line (`#[cold]`, never inlined), so the
    // recursive parser's frames carry no `format!` temporaries: the
    // compiler's stack depth is what the firmware's main task must hold.

    #[cold]
    #[inline(never)]
    fn fail(&self, t: Token, msg: &str) -> Error {
        Error::new(t.start, t.end, msg)
    }

    #[cold]
    #[inline(never)]
    fn fail_at(&self, t: Token, msg: String) -> Error {
        Error::new(t.start, t.end, msg)
    }

    #[inline(never)]
    fn expect(&mut self, want: Tok, what: &str) -> Result<Token, Error> {
        let t = self.peek();
        if t.tok == want {
            return Ok(self.next());
        }
        Err(self.expected(t, what))
    }

    #[cold]
    #[inline(never)]
    fn expected(&self, t: Token, what: &str) -> Error {
        self.fail_at(t, format!("expected {what} here, found {}", self.found(t)))
    }

    /// `close` where the `)` of `open` should be: never closed (the newline
    /// was swallowed), or something else in the way.
    #[cold]
    #[inline(never)]
    fn not_closed(&self, open: Token, close: Token, name: &str) -> Error {
        if let Err(e) = self.unclosed(open, close) {
            return e;
        }
        if name.is_empty() {
            return self.expected(close, "`)`");
        }
        self.fail_at(
            close,
            format!(
                "expected `,` or `)` in `{name}(…)`, found {}",
                self.found(close)
            ),
        )
    }

    fn found(&self, t: Token) -> String {
        match t.tok {
            Tok::Newline | Tok::Eof => describe(t.tok).to_string(),
            _ => format!("`{}`", self.text(t)),
        }
    }

    /// A `(` whose `)` was expected at `found` on a later line: the
    /// newline was swallowed, so say which bracket is open.
    #[inline(never)]
    fn unclosed(&self, open: Token, found: Token) -> Result<(), Error> {
        if found.tok == Tok::Eof || self.line_of(found.start) > self.line_of(open.start) {
            return self.err(open, "this `(` is never closed — add `)`");
        }
        Ok(())
    }

    #[inline(never)]
    fn skip_newlines(&mut self) {
        while self.peek().tok == Tok::Newline {
            self.next();
        }
    }

    /// An expression (Pratt): operators binding tighter than `min_bp`.
    #[inline(never)]
    fn expr(&mut self, min_bp: u8) -> Result<Id, Error> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(self.too_deep());
        }
        let r = self.expr_inner(min_bp);
        self.depth -= 1;
        r
    }

    #[cold]
    #[inline(never)]
    fn too_deep(&self) -> Error {
        let t = self.peek();
        // Inside parts, their nesting counts too.
        self.fail(
            t,
            if self.scopes.iter().any(|s| s.1) {
                "parts go too deep here — take one out of another"
            } else {
                "this is nested too deeply — split it into named steps"
            },
        )
    }

    #[cold]
    #[inline(never)]
    fn no_value(&self, t: Token) -> Error {
        match t.tok {
            Tok::Newline | Tok::Eof => self.fail(t, "a value is missing here"),
            _ => self.fail_at(t, format!("expected a value, found {}", self.found(t))),
        }
    }

    fn expr_inner(&mut self, min_bp: u8) -> Result<Id, Error> {
        // After an operator, the expression may continue on the next line.
        self.skip_newlines();
        let t = self.next();
        let mut lhs =
            match t.tok {
                Tok::Num(v) => self.dag.constant(v, (t.start, t.end))?,
                Tok::Minus => {
                    let a = self.expr(BP_UNARY)?;
                    self.dag.op(code::NEG, [a, 0, 0], (t.start, t.end))?
                }
                Tok::Plus => self.expr(BP_UNARY)?,
                Tok::Not => {
                    let a = self.expr(BP_NOT)?;
                    self.dag.op(code::NOT, [a, 0, 0], (t.start, t.end))?
                }
                Tok::LParen => {
                    let e = self.expr(0)?;
                    let close = self.peek();
                    if close.tok != Tok::RParen {
                        return Err(self.not_closed(t, close, ""));
                    }
                    self.next();
                    e
                }
                // `pick(…)` here, not in `name`: there it made every level of
                // nesting's frame bigger (a part called `pick` is still the part).
                Tok::Name if self.peek().tok == Tok::LParen && self.text(t) == "pick" => {
                    self.pick(t)?
                }
                Tok::Name => self.name(t)?,
                Tok::Hex(v) if self.env.ch < NO_CH => self.dag.constant(
                    fixed::byte((v >> (16 - 8 * self.env.ch as u32)) as u8),
                    (t.start, t.end),
                )?,
                Tok::Hex(_) => return self.err(
                    t,
                    if self.env.ch == IN_RGB {
                        "inside `rgb(r, g, b)` the three are values, not colours: \
                         write the colour on its own (`#f80`, `#f80 * k`)"
                    } else {
                        "a colour can't be a value: use it in the colour itself (the last line), \
                         as `#f80`, `#f80 * k`, `mix(#000, #f80, k)` or `rgb(#f80, k)`"
                    },
                ),
                _ => return Err(self.no_value(t)),
            };
        loop {
            let op = self.peek();
            if op.tok == Tok::Question {
                if BP_TERNARY < min_bp {
                    break;
                }
                self.next();
                let a = self.expr(0)?;
                self.skip_newlines();
                self.expect(Tok::Colon, "`:` (as in `cond ? this : that`)")?;
                let b = self.expr(BP_TERNARY)?;
                lhs = self.dag.op(code::SELECT, [lhs, a, b], (op.start, op.end))?;
                continue;
            }
            let Some((lbp, rbp, c)) = infix_bp(op.tok) else {
                break;
            };
            if lbp < min_bp {
                break;
            }
            self.next();
            let rhs = self.expr(rbp)?;
            lhs = self.dag.op(c, [lhs, rhs, 0], (op.start, op.end))?;
        }
        Ok(lhs)
    }

    /// Whether names set in `scope` can be seen here: this scope's and its
    /// parents', up to the part it is in (a part sees nothing outside it).
    fn visible(&self, scope: u16) -> bool {
        let mut s = self.scope;
        loop {
            if s == scope {
                return true;
            }
            let (parent, own) = self.scopes[s as usize];
            if own || s == 0 {
                return false;
            }
            s = parent;
        }
    }

    #[inline(never)]
    fn part_named(&self, name: &str) -> Option<usize> {
        self.parts
            .iter()
            .rposition(|p| p.name == name && self.visible(p.scope))
    }

    /// The candidates for a misspelt value name.
    fn value_names(&self) -> impl Iterator<Item = &str> + '_ {
        INPUTS
            .iter()
            .map(|e| e.0)
            .chain(CONSTS.iter().map(|e| e.0))
            .chain(LATER)
            .chain(
                self.locals
                    .iter()
                    .filter(|l| self.visible(l.scope))
                    .map(|l| l.name),
            )
    }

    fn name(&mut self, t: Token) -> Result<Id, Error> {
        let name = self.text(t);
        let at = (t.start, t.end);
        if let Some(k) = self.part_named(name) {
            return self.part_value(t, k);
        }
        if self.reads_table(name) {
            return self.table_read(t);
        }
        if self.peek().tok == Tok::LParen {
            if name == "rgb" && self.env.ch < NO_CH {
                return self.rgb_channel();
            }
            if let Some(&(_, ar, f)) = FUNCS.iter().find(|e| e.0 == name) {
                if let F::Glyph(c) = f {
                    let (args, font) = self.glyph_args(t, name)?;
                    let c = match font {
                        tables::NORMAL => c,
                        tables::SMALL => c + 2,
                        _ => code::DIGIT_B,
                    };
                    return self.dag.op(c, args, at);
                }
                let args = self.args(t, name, ar)?;
                return match f {
                    F::Op(c) | F::Glyph(c) => self.dag.op(c, args, at),
                    // (`pick` is read in `expr_inner`.)
                    F::Fract | F::Pick => self.dag.op(code::FRAC, args, at),
                    F::Smoothstep => {
                        // smoothstep(e0, e1, x) = smooth(x, e0, 1/(e1-e0))
                        let span = self.dag.op(code::SUB, [args[1], args[0], 0], at)?;
                        let inv = self.dag.op(code::RECIP, [span, 0, 0], at)?;
                        self.dag.op(code::SMOOTH, [args[2], args[0], inv], at)
                    }
                };
            }
        } else if name == "seed"
            && !self.dag.forever
            && !self
                .locals
                .iter()
                .any(|l| l.name == name && self.visible(l.scope))
        {
            // E903.
            return self.err(
                t,
                "`seed` is for formulas that go on without repeating (`loop forever`): a new one each time it plays",
            );
        } else if let Some(id) = self.lookup_value(name) {
            return id;
        }
        Err(self.unknown(t))
    }

    /// Why the name at `t` is not a value here (or, before `(`, a function).
    #[cold]
    #[inline(never)]
    fn unknown(&mut self, t: Token) -> Error {
        let name = self.text(t);
        let msg = if self.peek().tok == Tok::LParen {
            if self.env.ch < NO_CH && COLOURS.iter().any(|e| e.0 == name) {
                format!(
                    "in a sum with colours only `#…` and `rgb(r, g, b)` are colours: \
                     `{name}(…)` can only be a whole line of its own"
                )
            } else if COLOURS.iter().any(|e| e.0 == name) {
                format!(
                    "`{name}(…)` makes the colour, so it can only be the whole last line \
                     (of the formula, or of a layer)"
                )
            } else if PRIMITIVES.contains(&name) {
                format!("`{name}(…)` draws: it goes on a line of its own inside `draw {{ }}`")
            } else if self.lookup_value(name).is_some() {
                format!("`{name}` is a value, not a function")
            } else {
                let s = suggest(
                    name,
                    FUNCS.iter().map(|e| e.0).chain(COLOURS.iter().map(|e| e.0)),
                );
                format!("unknown function `{name}`{}", did_you_mean(s))
            }
        } else if FUNCS.iter().any(|e| e.0 == name) || COLOURS.iter().any(|e| e.0 == name) {
            format!("`{name}` is a function: write `{name}(…)`")
        } else if KEYWORDS.contains(&name) {
            format!("`{name}` can only start a line")
        } else if PALETTES.iter().any(|e| e.0 == name) {
            format!("`{name}` is a palette: use it as `pal({name}, k)` on the last line")
        } else if FONTS.iter().any(|e| e.0 == name) {
            format!("`{name}` is a font: use it as `digit(n, x, y, {name})`")
        } else {
            let s = suggest(name, self.value_names());
            format!("unknown name `{name}`{}", did_you_mean(s))
        };
        self.fail_at(t, msg)
    }

    #[inline(never)]
    fn lookup_value(&mut self, name: &str) -> Option<Result<Id, Error>> {
        if let Some(l) = self
            .locals
            .iter()
            .find(|l| l.name == name && self.visible(l.scope))
        {
            return Some(Ok(l.id));
        }
        if let Some(&(_, inp, _)) = INPUTS.iter().find(|e| e.0 == name) {
            return Some(self.input(inp));
        }
        if let Some(&(_, v)) = CONSTS.iter().find(|e| e.0 == name) {
            return Some(self.dag.constant(v, (0, 0)));
        }
        if name == "seed" {
            return Some(self.input(Input::Seed));
        }
        let k = LATER.iter().position(|&e| e == name)?;
        Some(self.centred(k))
    }

    /// [`LATER`]'s value `k` (unused steps cost nothing: they are dropped).
    #[inline(never)]
    fn centred(&mut self, k: usize) -> Result<Id, Error> {
        let mut v = [0 as Id; 12];
        for (i, inp) in [Input::X, Input::Y, Input::Py].into_iter().enumerate() {
            v[i] = self.input(inp)?;
        }
        for (i, c) in [ONE / 2, 4 * ONE, 31 * ONE].into_iter().enumerate() {
            v[3 + i] = self.dag.constant(c, (0, 0))?;
        }
        for (i, &(c, a, b)) in CENTRED.iter().enumerate() {
            v[6 + i] = self.dag.op(c, [v[a as usize], v[b as usize], 0], (0, 0))?;
        }
        Ok(v[7 + k])
    }

    /// A built-in value here: inside a part, its own `t`, and where it is
    /// drawn — `px = floor(x·127 + ½)`, `py = floor(y·31 + ½)`, `i = (31 −
    /// py)·128 + px`, as on the panel (`x = px/127`).
    #[inline(never)]
    fn input(&mut self, inp: Input) -> Result<Id, Error> {
        let at = (0, 0);
        if let (Input::T, Some(t)) = (inp, self.env.t) {
            return Ok(t);
        }
        let Some((x, y)) = self.env.at else {
            return self.dag.input(inp, at);
        };
        let grid = |p: &mut Self, v: Id, n: i32| -> Result<Id, Error> {
            let k = p.dag.constant(n << 16, at)?;
            let half = p.dag.constant(ONE / 2, at)?;
            let m = p.dag.op(code::MUL, [v, k, 0], at)?;
            let r = p.dag.op(code::ADD, [m, half, 0], at)?;
            p.dag.op(code::FLOOR, [r, 0, 0], at)
        };
        match inp {
            Input::X => Ok(x),
            Input::Y => Ok(y),
            Input::Px => grid(self, x, W as i32 - 1),
            Input::Py => grid(self, y, H as i32 - 1),
            Input::I => {
                let px = grid(self, x, W as i32 - 1)?;
                let py = grid(self, y, H as i32 - 1)?;
                let top = self.dag.constant((H as i32 - 1) << 16, at)?;
                let w = self.dag.constant((W as i32) << 16, at)?;
                let row = self.dag.op(code::SUB, [top, py, 0], at)?;
                let r = self.dag.op(code::MUL, [row, w, 0], at)?;
                self.dag.op(code::ADD, [r, px, 0], at)
            }
            _ => self.dag.input(inp, at),
        }
    }

    /// `( a, b, … )` with exactly `n` values.
    fn args(&mut self, f: Token, name: &str, n: usize) -> Result<[Id; 3], Error> {
        let (a, _) = self.args_or_cover(f, name, n, false)?;
        Ok([a[0], a[1], a[2]])
    }

    /// `( a, b, … )` with `n` values, or with `cover` also `n + 1` (a
    /// colour's last: how much of the pixel it covers); the values and how
    /// many.
    fn args_or_cover(
        &mut self,
        f: Token,
        name: &str,
        n: usize,
        cover: bool,
    ) -> Result<([Id; 4], usize), Error> {
        let open = self.expect(Tok::LParen, "`(`")?;
        let mut out = [0 as Id; 4];
        let mut k = 0;
        if self.peek().tok != Tok::RParen {
            loop {
                let e = self.expr(0)?;
                if k < 4 {
                    out[k] = e;
                }
                k += 1;
                if self.peek().tok == Tok::Comma {
                    self.next();
                    continue;
                }
                break;
            }
        }
        let close = self.peek();
        if close.tok != Tok::RParen {
            return Err(self.not_closed(open, close, name));
        }
        self.next();
        if k != n && !(cover && k == n + 1) {
            return Err(self.arity(f, close, name, n, k, cover));
        }
        Ok((out, k))
    }

    /// `pick(k, a, b, …)` at `t` (out of line: the recursive parser's
    /// frames stay small).
    #[inline(never)]
    fn pick(&mut self, t: Token) -> Result<Id, Error> {
        if self.part_named("pick").is_some() {
            return self.name(t);
        }
        let open = self.next();
        let k = self.expr(0)?;
        let mut v = Vec::new();
        while self.peek().tok == Tok::Comma {
            self.next();
            v.push(self.expr(0)?);
        }
        let close = self.peek();
        if close.tok != Tok::RParen {
            return Err(self.not_closed(open, close, "pick"));
        }
        self.next();
        if v.is_empty() {
            return self.err(
                t,
                "`pick` takes a number, then the values to pick from: `pick(k, a, b, …)`",
            );
        }
        self.dag.pick(k, v, (t.start, t.end))
    }

    /// `digit(n, x, y)` / `glyph(c, x, y)`, with a font (`, small`, …)
    /// last if you like: the three values and the font.
    #[inline(never)]
    fn glyph_args(&mut self, f: Token, name: &str) -> Result<([Id; 3], u8), Error> {
        let open = self.expect(Tok::LParen, "`(`")?;
        let mut out = [0 as Id; 3];
        let (mut k, mut font_of) = (0, tables::NORMAL);
        if self.peek().tok != Tok::RParen {
            loop {
                if k == 3 {
                    let p = self.peek();
                    let font = FONTS
                        .iter()
                        .find(|e| p.tok == Tok::Name && e.0 == self.text(p));
                    let Some(&(_, n)) = font else {
                        return Err(self.no_font(p, name));
                    };
                    if n == tables::BOLD && name != "digit" {
                        return self.err(
                            p,
                            "the bold font has digits only: use `digit(n, x, y, bold)`",
                        );
                    }
                    self.next();
                    font_of = n;
                } else {
                    let e = self.expr(0)?;
                    if let Some(o) = out.get_mut(k) {
                        *o = e;
                    }
                }
                k += 1;
                if self.peek().tok == Tok::Comma {
                    self.next();
                    continue;
                }
                break;
            }
        }
        let close = self.peek();
        if close.tok != Tok::RParen {
            return Err(self.not_closed(open, close, name));
        }
        self.next();
        if k != 3 && k != 4 {
            return Err(self.glyph_arity(f, close, name, k));
        }
        Ok((out, font_of))
    }

    #[cold]
    #[inline(never)]
    fn glyph_arity(&self, f: Token, close: Token, name: &str, k: usize) -> Error {
        let what = if name == "digit" {
            "the digit"
        } else {
            "the character"
        };
        Error::new(
            f.start,
            close.end,
            format!(
                "`{name}` takes 3 values ({what}, a column and a row; and a font, normal, small or bold, if you like), but got {k}"
            ),
        )
    }

    #[cold]
    #[inline(never)]
    fn no_font(&self, t: Token, name: &str) -> Error {
        self.fail_at(
            t,
            format!(
                "the last value of `{name}` is a font: normal (the default), small or bold, found {}",
                self.found(t)
            ),
        )
    }

    /// `name(…)` from `f` to `close` given `k` values for `n` (`cover`: or
    /// `n + 1`).
    #[cold]
    #[inline(never)]
    fn arity(&self, f: Token, close: Token, name: &str, n: usize, k: usize, cover: bool) -> Error {
        let s = if n == 1 { "" } else { "s" };
        // A part: where to draw it.
        let what = if self.part_named(name).is_some() {
            ", where to draw it (x, y)"
        } else {
            ""
        };
        let msg = if cover {
            format!(
                "`{name}` takes {n} value{s}{what} (or {}: the last is how much of the pixel it covers, 0 to 1), but got {k}",
                n + 1
            )
        } else {
            format!("`{name}` takes {n} value{s}{what}, but got {k}")
        };
        Error::new(f.start, close.end, msg)
    }

    /// The colour call at `t` (its name): `rgb(r, g, b)`, `hsv(h, s, v)`,
    /// `pal(name, k)`, `gray(v)`.
    #[inline(never)]
    fn colour(&mut self, t: Token) -> Result<Colour, Error> {
        let name = self.text(t);
        let &(_, ar, c) = COLOURS.iter().find(|e| e.0 == name).unwrap_or(&COLOURS[0]);
        // `rgb(#f80)`, `rgb(mix(…), k)`: a colour, and maybe its coverage.
        if c == code::OUT_RGB && self.has_colour(self.pos + 1, true) {
            let open = self.next();
            let mut col = self.colour_line()?;
            if self.peek().tok == Tok::Comma {
                self.next();
                col.alpha = Some(self.expr(0)?);
            }
            let close = self.peek();
            if close.tok != Tok::RParen {
                self.unclosed(open, close)?;
                return self.err(
                    close,
                    "`rgb` of a colour takes the colour and, if you like, how much of the pixel it covers (0 to 1)",
                );
            }
            self.next();
            return Ok(col);
        }
        if c != code::OUT_PAL {
            if c == code::OUT_RGB {
                self.env.ch = IN_RGB;
            }
            let r = self.args_or_cover(t, name, ar, true);
            self.env.ch = NO_CH;
            let (a, k) = r?;
            return Ok(Colour {
                code: c,
                args: [a[0], a[1], a[2]],
                pal: 0,
                alpha: (k > ar).then_some(a[ar]),
                blend: fixed::BLEND_OVER,
                scale: None,
            });
        }
        self.expect(Tok::LParen, "`(`")?;
        let p = self.peek();
        if p.tok != Tok::Name {
            return self.err(
                p,
                "the first value of `pal` is a palette: fire, ocean, ice, chrono, amber, rainbow or gray",
            );
        }
        self.next();
        let pname = self.text(p);
        let Some(&(_, pal)) = PALETTES.iter().find(|e| e.0 == pname) else {
            let s = suggest(pname, PALETTES.iter().map(|e| e.0));
            return self.err(
                p,
                format!(
                    "unknown palette `{pname}`{} (there are fire, ocean, ice, chrono, amber, rainbow and gray)",
                    did_you_mean(s)
                ),
            );
        };
        self.expect(Tok::Comma, "`,` (as in `pal(fire, k)`)")?;
        let k = self.expr(0)?;
        let mut alpha = None;
        if self.peek().tok == Tok::Comma {
            self.next();
            alpha = Some(self.expr(0)?);
        }
        let close = self.peek();
        if close.tok != Tok::RParen {
            return self.err(
                close,
                format!(
                    "`pal` takes a palette and one value (and how much of the pixel it covers, 0 to 1, if you like); found {}",
                    self.found(close)
                ),
            );
        }
        self.next();
        let (code, pal) = if pal == 255 {
            (code::OUT_GRAY, 0)
        } else {
            (code::OUT_PAL, pal)
        };
        Ok(Colour {
            code,
            args: [k, 0, 0],
            pal,
            alpha,
            blend: fixed::BLEND_OVER,
            scale: None,
        })
    }

    /// Whether there is a colour (`#f80` or `rgb(`) from token `k` to the
    /// end of the line, or with `arg`, of the value (a `,` or `)` that
    /// closes it).
    fn has_colour(&self, mut k: usize, arg: bool) -> bool {
        let mut depth = 0;
        loop {
            let t = self.toks[k];
            match t.tok {
                Tok::Hex(_) => return true,
                Tok::Name if self.toks[k + 1].tok == Tok::LParen && self.text(t) == "rgb" => {
                    return true
                }
                Tok::LParen => depth += 1,
                Tok::RParen if depth > 0 => depth -= 1,
                Tok::Comma if depth > 0 || !arg => {}
                Tok::Comma | Tok::RParen | Tok::Newline | Tok::Eof | Tok::RBrace => return false,
                _ => {}
            }
            k += 1;
        }
    }

    /// A colour expression at the cursor, read once for each channel: its
    /// red, green and blue (`#f80 * k` is `rgb(k, k*0.53, 0)`).
    fn channels(&mut self) -> Result<[Id; 3], Error> {
        let start = self.pos;
        let mut out = [0; 3];
        for (ch, o) in out.iter_mut().enumerate() {
            self.pos = start;
            self.env.ch = ch as u8;
            let r = self.expr(0);
            self.env.ch = NO_CH;
            *o = r?;
        }
        Ok(out)
    }

    /// `rgb(r, g, b)` (the cursor at its `(`) inside a colour expression: the channel's
    /// value (its values are plain values).
    #[inline(never)]
    fn rgb_channel(&mut self) -> Result<Id, Error> {
        let ch = self.env.ch;
        let open = self.next();
        // `rgb(#f80)` inside: the colour; how much it covers is the line's.
        if self.has_colour(self.pos, true) {
            let v = self.expr(0)?;
            if self.peek().tok == Tok::Comma {
                return self.err(
                    self.peek(),
                    "how much a colour covers goes on the whole line: `rgb(mix(…), k)`, not on an `rgb(…)` inside it",
                );
            }
            self.expect(Tok::RParen, "`)`")?;
            return Ok(v);
        }
        // Only this channel's value is read (the others' in their own
        // reading: no values made early, held in registers until then).
        let mut v = 0;
        for i in 0..3u8 {
            if i > 0 {
                self.expect(Tok::Comma, "`,` (`rgb(r, g, b)` takes three values)")?;
            }
            if i == ch {
                self.env.ch = IN_RGB;
                let r = self.expr(0);
                self.env.ch = ch;
                v = r?;
            } else {
                self.skip_value();
            }
        }
        let close = self.peek();
        if close.tok != Tok::RParen {
            return Err(self.not_closed(open, close, "rgb"));
        }
        self.next();
        Ok(v)
    }

    /// Past one value of a call (to its `,` or `)`).
    fn skip_value(&mut self) {
        let mut depth = 0;
        loop {
            match self.peek().tok {
                Tok::LParen => depth += 1,
                Tok::RParen | Tok::Comma if depth == 0 => return,
                Tok::RParen => depth -= 1,
                Tok::Newline | Tok::Eof | Tok::RBrace => return,
                _ => {}
            }
            self.next();
        }
    }

    /// `rgb(…)` or a colour on its own as a colour line: its channels.
    fn colour_line(&mut self) -> Result<Colour, Error> {
        Ok(Colour {
            code: code::OUT_RGB,
            args: self.channels()?,
            pal: 0,
            alpha: None,
            blend: fixed::BLEND_OVER,
            scale: None,
        })
    }

    fn end_of_statement(&mut self) -> Result<(), Error> {
        let t = self.peek();
        match t.tok {
            Tok::Newline | Tok::Eof | Tok::RBrace => {
                if t.tok == Tok::Newline {
                    self.next();
                }
                Ok(())
            }
            Tok::Name | Tok::Num(_) | Tok::LParen => self.err(
                t,
                format!(
                    "expected the end of the line, found {} (missing an operator?)",
                    self.found(t)
                ),
            ),
            _ => self.err(
                t,
                format!("expected the end of the line, found {}", self.found(t)),
            ),
        }
    }

    /// A name about to be set at `t` (a value or a part): not a built-in,
    /// and not one already set here.
    fn new_name(&self, t: Token) -> Result<&'s str, Error> {
        let name = self.text(t);
        // (`ink` came after formulas were saved: a name set with it is that
        // name, as `cx` and the rest.)
        if let Some(e) = INPUTS.iter().find(|e| e.0 == name && e.0 != "ink") {
            return self.err(
                t,
                format!("`{name}` is built in ({}); pick another name", e.2),
            );
        }
        // Functions added after formulas were saved stay settable names
        // (a call is still the function): a saved `glyph = …` compiles.
        if CONSTS.iter().any(|e| e.0 == name)
            || FUNCS
                .iter()
                .any(|e| e.0 == name && !LATER_FUNCS.contains(&name))
            || COLOURS.iter().any(|e| e.0 == name)
            || KEYWORDS.contains(&name)
        {
            return self.err(t, format!("`{name}` is built in; pick another name"));
        }
        if PALETTES.iter().any(|e| e.0 == name) {
            return self.err(
                t,
                format!("`{name}` is a palette's name; pick another name"),
            );
        }
        let local = self
            .locals
            .iter()
            .find(|l| l.name == name && self.visible(l.scope));
        if local.is_some_and(|l| l.counter) {
            return self.err(t, format!("`{name}` is a loop counter: it can't be set"));
        }
        let prev = local
            .map(|l| l.at)
            .or(self.part_named(name).map(|k| self.parts[k].at))
            .or(self.table_named(name).map(|k| self.table_names[k].at))
            .or(self.sprite_named(name).map(|k| self.sprites[k].at));
        if let Some(at) = prev {
            return self.err(
                t,
                format!(
                    "`{name}` is already set on line {} — each name can be set once",
                    self.line_of(at)
                ),
            );
        }
        Ok(name)
    }

    /// `name = expr`.
    fn assignment(&mut self, in_frame: bool) -> Result<(), Error> {
        let t = self.next();
        let name = self.new_name(t)?;
        let eq = self.expect(Tok::Assign, "`=`")?;
        if matches!(self.peek().tok, Tok::Newline | Tok::Eof) {
            return self.err(eq, format!("a value is missing after `=` (`{name} = …`)"));
        }
        let start = self.peek().start;
        let id = self.expr(0)?;
        let end = self.toks[self.pos.saturating_sub(1)].end;
        if in_frame && self.dag.level(id) > LV_FRAME {
            let why = INPUTS[self.dag.nodes[id as usize].why as usize];
            return Err(Error::new(
                start,
                end.max(start),
                format!(
                    "inside frame {{ }} only values that are the same for every pixel can be used, \
                     but this uses `{}` ({})",
                    why.0, why.2
                ),
            ));
        }
        if self.in_draw > 0 {
            self.drawable(id, start, end)?;
        }
        self.locals.push(Local {
            name,
            id,
            at: t.start,
            in_frame,
            counter: false,
            scope: self.scope,
        });
        self.end_of_statement()
    }

    #[inline(never)]
    fn is_assignment(&self) -> bool {
        self.peek().tok == Tok::Name && self.toks[self.pos + 1].tok == Tok::Assign
    }

    /// `fps N` at the cursor: the word `fps` with a number (or nothing)
    /// after it. Not a keyword: `fps = 3` still sets a name, as formulas
    /// written before it may.
    fn is_fps(&self) -> bool {
        let t = self.peek();
        t.tok == Tok::Name
            && self.text(t) == "fps"
            && matches!(
                self.toks[self.pos + 1].tok,
                Tok::Num(_) | Tok::Newline | Tok::RBrace | Tok::Eof
            )
    }

    /// `fps N` (E801): the frame time it asks for, ms. The panel's frame
    /// grids are 10, 20 and 40 ms.
    #[inline(never)]
    fn fps_line(&mut self) -> Result<u8, Error> {
        let kw = self.next();
        let n = self.peek();
        let ms = match n.tok {
            Tok::Num(v) if v == 25 * ONE => 40,
            Tok::Num(v) if v == 50 * ONE => 20,
            Tok::Num(v) if v == 100 * ONE => 10,
            Tok::Num(_) => return self.err(n, "`fps` is 25, 50 or 100 frames a second"),
            _ => {
                return self.err(
                    kw,
                    "`fps` takes 25, 50 or 100 frames a second, e.g. `fps 100`",
                )
            }
        };
        self.next();
        self.end_of_statement()?;
        Ok(ms)
    }

    /// The word starting the statement at the cursor, if it is a keyword
    /// used as one (`loop = 3` is an assignment, and refused as such).
    fn keyword(&self) -> &'s str {
        let t = self.peek();
        if t.tok != Tok::Name || self.toks[self.pos + 1].tok == Tok::Assign {
            return "";
        }
        let w = self.text(t);
        if KEYWORDS.contains(&w) {
            w
        } else {
            ""
        }
    }

    /// Inside a `layer { }` (not a formula's or a part's own lines).
    #[inline(never)]
    fn in_layer(&self) -> bool {
        self.scope != 0 && !self.scopes[self.scope as usize].1
    }

    /// `frame { name = value … }` at `t` (one per formula or part, and one
    /// per layer).
    fn frame_block(&mut self, t: Token, seen: &mut bool) -> Result<(), Error> {
        self.next();
        if *seen {
            return self.err(
                t,
                if self.in_layer() {
                    "a layer can have one `frame { }` block"
                } else {
                    "there can be one `frame { }` block"
                },
            );
        }
        *seen = true;
        self.expect(Tok::LBrace, "`{` (as in `frame { … }`)")?;
        self.skip_newlines();
        while self.peek().tok != Tok::RBrace {
            if self.peek().tok == Tok::Eof {
                let t = self.peek();
                return self.err(t, "`frame {` is never closed: add `}`");
            }
            if !self.is_assignment() {
                let t = self.peek();
                return self.err(t, "only `name = value` lines can go inside frame { }");
            }
            self.assignment(true)?;
            self.skip_newlines();
        }
        self.next();
        self.end_of_statement()
    }

    /// Whether the statement at the cursor is a name, maybe a bracket, and
    /// nothing else (`p` or `p(x*2, y)`: a part's colour, not `p.v + 1`).
    /// (`open`: what a `(` never closed counts as.)
    fn whole_line(&self, open: bool) -> bool {
        let mut k = self.pos + 1;
        if self.toks[k].tok == Tok::LParen {
            let mut depth = 0;
            loop {
                match self.toks[k].tok {
                    Tok::LParen => depth += 1,
                    Tok::RParen => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    Tok::Eof => return open,
                    _ => {}
                }
                k += 1;
            }
            k += 1;
        }
        matches!(self.toks[k].tok, Tok::Newline | Tok::Eof | Tok::RBrace)
    }

    /// A colour line at `t` (its name), a part drawn (`p`, `p(x', y')`,
    /// `p(x', y', k)`), or a bare value as the last line of the formula or
    /// a part (`end` Eof or RBrace) or of a layer (RBrace): its gray. The
    /// colours (several for a part with layers) and whether they are layers.
    fn colour_or_value(&mut self, t: Token, end: Tok) -> Result<(Vec<Colour>, bool), Error> {
        let word = if t.tok == Tok::Name { self.text(t) } else { "" };
        if let Some(k) = self.part_named(word).filter(|_| self.whole_line(false)) {
            self.next();
            let (j, cover) = self.part_at(t, k, true)?;
            self.end_of_statement()?;
            let inst = &self.parts[k].insts[j];
            if inst.colours.is_empty() {
                return self.err(
                    t,
                    format!("part `{word}` has no colour, only values: `{word}.v`"),
                );
            }
            let (mut cs, layered) = (inst.colours.clone(), inst.layered);
            if let Some(k) = cover {
                for c in &mut cs {
                    c.alpha = Some(match c.alpha {
                        Some(a) => self.dag.op(code::MUL, [a, k, 0], (t.start, t.end))?,
                        None => k,
                    });
                }
            }
            return Ok((cs, layered));
        }
        if t.tok == Tok::Name
            && COLOURS.iter().any(|e| e.0 == word)
            && self.toks[self.pos + 1].tok == Tok::LParen
            && (word != "rgb" || self.whole_line(true))
        {
            self.next();
            let mut c = self.colour(t)?;
            let mut e = self.peek();
            // `pal(P, v) * k`: the palette's colour, each channel times k
            // (a light on a palette, in one step); `* k * m` too.
            if word == "pal" && e.tok == Tok::Star && !self.has_colour(self.pos + 1, false) {
                let at = (e.start, e.end);
                let mut k = None;
                while self.peek().tok == Tok::Star {
                    self.next();
                    let v = self.expr(10)?;
                    k = Some(match k {
                        Some(k) => self.dag.op(code::MUL, [k, v, 0], at)?,
                        None => v,
                    });
                }
                if let Some(k) = k {
                    if c.code == code::OUT_GRAY {
                        c.args[0] = self.dag.op(code::MUL, [c.args[0], k, 0], at)?;
                    } else {
                        c.scale = Some(k);
                    }
                }
                e = self.peek();
            }
            if infix_bp(e.tok).is_some() && word != "rgb" {
                return self.err(
                    e,
                        if word == "pal" {
                        String::from(
                            "`pal(…)` can't go into a sum: it can only be scaled, `pal(fire, v) * k` \
                             (`#…` and `rgb(r, g, b)` colours can: `#fff * k`, `mix(#000, #f80, k)`)"
                        )
                    } else {
                        format!(
                            "`{word}(…)` can't go into a sum: only `#…` and `rgb(r, g, b)` colours can \
                             (`#fff * k`, `rgb(r, g, b) * k`)"
                        )
                    },
                );
            }
            self.end_of_statement()?;
            return Ok((alloc::vec![c], false));
        }
        // A colour on its own: `#102040`, `mix(#000, #f80, k)`, `#f80 * k`.
        if self.has_colour(self.pos, false) {
            let c = self.colour_line()?;
            self.end_of_statement()?;
            return Ok((alloc::vec![c], false));
        }
        let id = self.expr(0)?;
        self.end_of_statement()?;
        self.skip_newlines();
        if self.peek().tok != end {
            return Err(Error::new(
                t.start,
                self.toks[self.pos.saturating_sub(1)].start.max(t.end),
                if self.in_layer() {
                    "only a layer's last line can be a bare value (its colour) — did you mean `name = …`?"
                } else {
                    "only the last line can be a bare value (the colour) — did you mean `name = …`?"
                },
            ));
        }
        Ok((
            alloc::vec![Colour {
                code: code::OUT_GRAY,
                args: [id, 0, 0],
                pal: 0,
                alpha: None,
                blend: fixed::BLEND_OVER,
                scale: None,
            }],
            false,
        ))
    }

    /// `layer [blend] { … }` at `kw`: its own values (and `frame { }`),
    /// ending in its colour (several: a part with layers drawn whole).
    #[inline(never)]
    fn layer(&mut self, kw: Token) -> Result<(Vec<Colour>, Token), Error> {
        self.next();
        let mut blend = None;
        let b = self.peek();
        if b.tok == Tok::Name {
            let name = self.text(b);
            let Some(&(_, m)) = BLENDS.iter().find(|e| e.0 == name) else {
                let s = suggest(name, BLENDS.iter().map(|e| e.0));
                return self.err(
                    b,
                    format!(
                        "unknown blend `{name}`{} (a layer goes over, add, multiply, screen, max or min)",
                        did_you_mean(s)
                    ),
                );
            };
            self.next();
            blend = Some(m);
        }
        let open = self.peek();
        if open.tok != Tok::LBrace {
            return self.err(
                open,
                format!(
                    "expected `{{` here (as in `layer add {{ … }}`), found {}",
                    self.found(open)
                ),
            );
        }
        self.next();
        let outer = self.scope;
        self.scopes.push((outer, false));
        self.scope = (self.scopes.len() - 1) as u16;
        let mut seen_frame = false;
        let mut colour: Option<((Vec<Colour>, bool), Token)> = None;
        self.skip_newlines();
        loop {
            let t = self.peek();
            match t.tok {
                Tok::RBrace => break,
                Tok::Eof => return self.err(kw, "this `layer {` is never closed — add `}`"),
                _ => {}
            }
            if let Some((_, ct)) = &colour {
                let ct = *ct;
                return self.err(
                    ct,
                    "the colour has to be the layer's last line (move this to just before `}`)",
                );
            }
            match self.keyword() {
                "loop" => {
                    return self.err(
                        t,
                        "`loop` is the whole formula's: put it on a line of its own, outside the layers",
                    )
                }
                "layer" => {
                    return self.err(
                        t,
                        "a layer can't go inside another — close this one with `}` first",
                    )
                }
                "part" => {
                    return self.err(
                        t,
                        "a part goes outside the layers (draw it in one with `layer { p }`)",
                    )
                }
                _ if self.is_fps() => {
                    return self.err(
                        t,
                        "`fps` is the whole formula's: put it on a line of its own, outside the layers",
                    )
                }
                "frame" => self.frame_block(t, &mut seen_frame)?,
                "draw" | "table" | "for" => return Err(self.misplaced(t)),
                _ if self.is_sprite() => return Err(self.misplaced(t)),
                _ if self.is_assignment() => self.assignment(false)?,
                _ => colour = Some((self.colour_or_value(t, Tok::RBrace)?, t)),
            }
            self.skip_newlines();
        }
        self.next();
        self.scope = outer;
        let Some(((mut cs, layered), ct)) = colour else {
            return self.err(
                kw,
                "this layer has no colour: end it with `rgb(…)`, `hsv(…)`, `pal(…)` or `gray(…)`",
            );
        };
        match (layered, blend) {
            // A part with layers keeps its own blends: it goes over, whole.
            (true, Some(m)) if m != fixed::BLEND_OVER => {
                return self.err(
                    b,
                    "this part has layers of its own: it can only go over, as in `layer { p }`",
                )
            }
            (true, _) => {}
            (false, m) => cs[0].blend = m.unwrap_or(fixed::BLEND_OVER),
        }
        self.end_of_statement()?;
        Ok((cs, ct))
    }

    /// The `loop` the lines from token `k` set (to their `}`, or the end),
    /// read ahead: a part's `t` is its own from its first line.
    fn loop_ahead(&self, mut k: usize) -> Option<i32> {
        let mut depth = 0;
        let mut start = true;
        loop {
            let t = self.toks[k];
            match t.tok {
                Tok::LBrace => depth += 1,
                Tok::RBrace if depth == 0 => return None,
                Tok::RBrace => depth -= 1,
                Tok::Eof => return None,
                Tok::Name if depth == 0 && start && self.text(t) == "loop" => {
                    match self.toks[k + 1].tok {
                        Tok::Num(v) => return Some(v),
                        Tok::Name if self.text(self.toks[k + 1]) == "forever" => {
                            return Some(FOREVER)
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
            start = matches!(t.tok, Tok::Newline | Tok::LBrace | Tok::RBrace);
            k += 1;
        }
    }

    /// `part name { … }` at `kw`: read where it is written (its errors show
    /// there, and `name` / `name.v` drawn where the pixel is are ready).
    #[inline(never)]
    fn part(&mut self, kw: Token) -> Result<(), Error> {
        self.next();
        let n = self.peek();
        if n.tok != Tok::Name {
            return self.err(n, "a part needs a name, as in `part p { … }`");
        }
        let name = self.new_name(n)?;
        self.next();
        let open = self.peek();
        if open.tok != Tok::LBrace {
            return self.err(
                open,
                format!(
                    "expected `{{` here (as in `part {name} {{ … }}`), found {}",
                    self.found(open)
                ),
            );
        }
        let body = self.pos + 1;
        let mut depth = 0;
        let mut end = self.pos;
        loop {
            match self.toks[end].tok {
                Tok::LBrace => depth += 1,
                Tok::RBrace => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                Tok::Eof => return self.err(kw, "this `part {` is never closed — add `}`"),
                _ => {}
            }
            end += 1;
        }
        let loop_v = self.loop_ahead(body).unwrap_or(8 * ONE);
        self.parts.push(Part {
            name,
            at: n.start,
            scope: self.scope,
            body,
            end,
            loop_v,
            insts: Vec::new(),
        });
        self.instance(self.parts.len() - 1, None, kw)?;
        self.pos = end + 1;
        self.end_of_statement()
    }

    /// Part `k` read at `at` (`None`: where the pixel is), for its use at
    /// `by`: once per place, with names of its own that nothing outside it
    /// sees, `t` its own loop's, and `x y px py i` the place's.
    #[inline(never)]
    fn instance(&mut self, k: usize, at: Option<(Id, Id)>, by: Token) -> Result<usize, Error> {
        if let Some(j) = self.parts[k].insts.iter().position(|i| i.at == at) {
            return Ok(j);
        }
        let (body, end, loop_v) = (self.parts[k].body, self.parts[k].end, self.parts[k].loop_v);
        // Read again (where it is written, it is read once, as all text is).
        if at.is_some() {
            self.work += end - body;
        }
        if self.work > MAX_WORK {
            return self.err(
                by,
                "this formula draws its parts in too many places — simplify it",
            );
        }
        self.depth += PART_DEPTH;
        if self.depth > MAX_DEPTH {
            return self.err(by, "parts go too deep here — take one out of another");
        }
        // (What it sets stays, in a scope nothing else sees: `p.v` reads it.
        // Its draw stage is recorded where it is written, not again for
        // each other place.)
        let saved = (self.pos, self.scope, self.env, self.record, self.in_draw);
        self.record &= at.is_none();
        self.in_draw = 0;
        // Its own time: `t` modulo its loop, unless the time here is shorter.
        let t = if self.env.loop_v <= loop_v {
            self.env.t
        } else {
            let a = (by.start, by.end);
            let t0 = self.input(Input::T)?;
            let n = self.dag.constant(loop_v, a)?;
            Some(self.dag.op(code::MOD, [t0, n, 0], a)?)
        };
        self.scopes.push((self.scope, true));
        self.scope = (self.scopes.len() - 1) as u16;
        self.env = Env {
            at: at.or(self.env.at),
            t,
            loop_v,
            ch: NO_CH,
        };
        self.pos = body;
        let b = self.body(Tok::RBrace)?;
        let scope = self.scope;
        (self.pos, self.scope, self.env, self.record, self.in_draw) = saved;
        self.depth -= PART_DEPTH;
        let p = &mut self.parts[k];
        p.insts.push(Inst {
            at,
            scope,
            colours: b.colours,
            layered: b.layered,
        });
        Ok(p.insts.len() - 1)
    }

    /// Part `k` used at `t` (its name, the cursor after it): `p`, or `p(x',
    /// y')` — and as a `colour`, `p(x', y', k)`. Which reading, and the
    /// coverage.
    #[inline(never)]
    fn part_at(&mut self, t: Token, k: usize, colour: bool) -> Result<(usize, Option<Id>), Error> {
        let (mut at, mut cover) = (None, None);
        if self.peek().tok == Tok::LParen {
            let (a, n) = self.args_or_cover(t, self.text(t), 2, colour)?;
            // Where the pixel is already: the reading as written.
            let here = (self.input(Input::X)?, self.input(Input::Y)?);
            at = Some((a[0], a[1])).filter(|&p| p != here);
            cover = (n == 3).then_some(a[2]);
        }
        Ok((self.instance(k, at, t)?, cover))
    }

    /// `p.v` or `p(x', y').v` at `t`: a value part `k` sets.
    #[inline(never)]
    fn part_value(&mut self, t: Token, k: usize) -> Result<Id, Error> {
        let (j, _) = self.part_at(t, k, false)?;
        if self.peek().tok != Tok::Dot {
            return Err(self.not_a_value(t, None));
        }
        self.next();
        let v = self.expect(Tok::Name, "the name of one of the part's values")?;
        let vn = self.text(v);
        let scope = self.parts[k].insts[j].scope;
        match self
            .locals
            .iter()
            .find(|l| l.scope == scope && l.name == vn)
        {
            Some(l) => Ok(l.id),
            None => Err(self.not_a_value(t, Some((v, scope)))),
        }
    }

    /// Part `t` used as a value without `.v`, or `.v` (in `scope`) that it
    /// doesn't set.
    #[cold]
    #[inline(never)]
    fn not_a_value(&self, t: Token, v: Option<(Token, u16)>) -> Error {
        let name = self.text(t);
        let Some((v, scope)) = v else {
            return self.fail_at(
                t,
                format!(
                    "`{name}` is a part: draw it as a whole last line (of the formula, or of a layer), \
                     or use one of its values, as in `{name}.v`"
                ),
            );
        };
        let vn = self.text(v);
        let s = suggest(
            vn,
            self.locals
                .iter()
                .filter(|l| l.scope == scope)
                .map(|l| l.name),
        );
        self.fail_at(
            v,
            format!("part `{name}` sets no value `{vn}`{}", did_you_mean(s)),
        )
    }

    /// `draw`, `table` or `for` at `t` where it can't go (inside a layer;
    /// `for` outside `draw { }`).
    #[cold]
    #[inline(never)]
    fn misplaced(&self, t: Token) -> Error {
        self.fail(
            t,
            match self.text(t) {
                "draw" => {
                    "`draw { }` goes at the top of the formula (or a part), not inside a layer"
                }
                // E911.
                "sprite" => {
                    "a sprite goes at the top of the formula (or a part), not inside a layer"
                }
                "table" => "a table goes at the top of the formula (or a part), not inside a layer",
                _ => "`for` goes inside `draw { }`: `draw { for k < 12 { … } }`",
            },
        )
    }

    /// The table called `name` that can be seen here.
    #[inline(never)]
    fn table_named(&self, name: &str) -> Option<usize> {
        self.table_names
            .iter()
            .rposition(|tb| tb.name == name && self.visible(tb.scope))
    }

    /// `NAME[k]` at `t` (its name), or why not: a table not read with
    /// `[ ]`, or `[ ]` after something that is not a table.
    #[inline(never)]
    fn table_read(&mut self, t: Token) -> Result<Id, Error> {
        let name = self.text(t);
        if let Some(k) = self.sprite_named(name) {
            return self.sprite_read(t, k);
        }
        let Some(k) = self.table_named(name) else {
            return self.err(
                t,
                format!("`[ ]` reads a table: `NAME[k]` — `{name}` is not a table"),
            );
        };
        let open = self.peek();
        if open.tok != Tok::LBracket {
            return self.err(
                t,
                format!("`{name}` is a table: read one of its numbers with `{name}[k]` (k from 0)"),
            );
        }
        self.next();
        let i = self.expr(0)?;
        let close = self.peek();
        if close.tok != Tok::RBracket {
            if close.tok == Tok::Eof || self.line_of(close.start) > self.line_of(open.start) {
                return self.err(open, "this `[` is never closed — add `]`");
            }
            return Err(self.expected(close, "`]`"));
        }
        self.next();
        let (start, len) = (self.table_names[k].start, self.table_names[k].len);
        self.dag
            .tab(i, start, len, &self.tables, (t.start, close.end))
    }

    /// Whether `name` at the cursor reads a table or a sprite (`[` after
    /// it, or a table's or a sprite's name). Out of line: the recursive
    /// parser's frames stay small.
    #[inline(never)]
    fn reads_table(&self, name: &str) -> bool {
        self.peek().tok == Tok::LBracket
            || (!self.table_names.is_empty() && self.table_named(name).is_some())
            || (!self.sprites.is_empty() && self.sprite_named(name).is_some())
    }

    /// The sprite called `name` that can be seen here.
    #[inline(never)]
    fn sprite_named(&self, name: &str) -> Option<usize> {
        self.sprites
            .iter()
            .rposition(|sp| sp.name == name && self.visible(sp.scope))
    }

    /// `NAME[u, v]` at `t`: sprite `k`'s pixel at column `u`, row `v`.
    #[inline(never)]
    fn sprite_read(&mut self, t: Token, k: usize) -> Result<Id, Error> {
        let name = self.text(t);
        let open = self.peek();
        if open.tok != Tok::LBracket {
            // E918.
            return self.err(
                t,
                format!("`{name}` is a sprite: read its pixels with `{name}[u, v]` (column, row from its top left)"),
            );
        }
        self.next();
        let u = self.expr(0)?;
        if self.peek().tok != Tok::Comma {
            let p = self.peek();
            return self.err(
                p,
                format!("a sprite is read at a column and a row: `{name}[u, v]`"),
            );
        }
        self.next();
        let v = self.expr(0)?;
        let close = self.peek();
        if close.tok != Tok::RBracket {
            if close.tok == Tok::Eof || self.line_of(close.start) > self.line_of(open.start) {
                return self.err(open, "this `[` is never closed — add `]`");
            }
            return Err(self.expected(close, "`]`"));
        }
        self.next();
        let at = (t.start, close.end);
        if self.dag.grows(u) || self.dag.grows(v) {
            return Err(Error::new(at.0, at.1, GROWS));
        }
        let start = self.sprites[k].start;
        let (mut level, mut why) = (LV_FRAME, 0);
        for x in [u, v] {
            let m = &self.dag.nodes[x as usize];
            if m.level > level {
                (level, why) = (m.level, m.why);
            }
        }
        self.dag.push(
            Node {
                code: code::SPR,
                a: u,
                b: v,
                c: start as Id,
                val: 0,
                level,
                why,
            },
            at,
        )
    }

    /// Whether the statement at the cursor is a sprite block (`sprite NAME
    /// {` as the lexer reads one: its rows are raw).
    fn is_sprite(&self) -> bool {
        let t = self.peek();
        t.tok == Tok::Name
            && self.text(t) == "sprite"
            && self.tok(self.pos + 1).tok == Tok::Name
            && self.tok(self.pos + 2).tok == Tok::LBrace
            && matches!(
                self.tok(self.pos + 3).tok,
                Tok::Raw | Tok::RBrace | Tok::Eof
            )
    }

    /// `sprite NAME { key …; rows }` at `kw`: its key (characters and the
    /// numbers they read as, up to 16) and its rows of characters (equal
    /// widths up to 64, up to 32 rows), packed into the tables: a header
    /// word, the keys' numbers, the rows at 1, 2 or 4 bits a pixel.
    #[inline(never)]
    fn sprite(&mut self, kw: Token) -> Result<(), Error> {
        self.next();
        let n = self.next();
        let name = self.new_name(n)?;
        self.next(); // `{`
        let mut raws: Vec<Token> = Vec::new();
        loop {
            let t = self.peek();
            match t.tok {
                Tok::Raw => {
                    raws.push(t);
                    self.next();
                }
                Tok::RBrace => {
                    self.next();
                    break;
                }
                // E919.
                _ => {
                    return self.err(
                        kw,
                        "this `sprite {` is never closed — add `}` on a line of its own",
                    )
                }
            }
        }
        let Some((&key, rows)) = raws.split_first() else {
            // E912.
            return self.err(kw, SPRITE_KEY);
        };
        // The key: `key C n, C n, …` (bytes: `str`'s trims are Unicode's).
        let kb = self.text(key).as_bytes();
        if kb.len() < 4 || &kb[..3] != b"key" || !matches!(kb[3], b' ' | b'\t') {
            return self.err(key, SPRITE_KEY);
        }
        let mut chars: Vec<u8> = Vec::new();
        let mut nums: Vec<i32> = Vec::new();
        let mut i = 3;
        while i < kb.len() {
            let ws = |i: &mut usize| {
                while *i < kb.len() && matches!(kb[*i], b' ' | b'\t') {
                    *i += 1;
                }
            };
            ws(&mut i);
            let s = i;
            let mut e = i;
            while e < kb.len() && kb[e] != b',' {
                e += 1;
            }
            let mut te = e;
            while te > s && matches!(kb[te - 1], b' ' | b'\t') {
                te -= 1;
            }
            let bad = Error::new(key.start + s, key.start + te.max(s + 1), SPRITE_KEY_ENTRY);
            if te < s + 3 || !key_char(kb[s]) || !matches!(kb[s + 1], b' ' | b'\t') {
                return Err(bad);
            }
            let mut ns = s + 1;
            while matches!(kb[ns], b' ' | b'\t') {
                ns += 1;
            }
            let neg = kb[ns] == b'-';
            let Some(v) = number(sub(self.src, key.start + ns + usize::from(neg), key.start + te)) else {
                return Err(bad);
            };
            if chars.contains(&kb[s]) {
                return Err(Error::new(key.start + s, key.start + s + 1, format!("`{}` is in the key twice", kb[s] as char)));
            }
            chars.push(kb[s]);
            nums.push(if neg { -v } else { v });
            i = e + 1;
        }
        if chars.len() > 16 {
            return self.err(key, "a sprite's key has 16 characters at most");
        }
        // Its rows.
        let w = rows.first().map_or(0, |r| r.end - r.start);
        if rows.is_empty() || rows.len() > 32 {
            // E916.
            return self.err(kw, "a sprite has 1 to 32 rows of characters, after its key");
        }
        let lb: u32 = match chars.len() {
            0..=2 => 0,
            3..=4 => 1,
            _ => 2,
        };
        let ppw = 32 >> lb;
        let wpr = w.div_ceil(ppw);
        let mut words: Vec<i32> = Vec::with_capacity(1 + chars.len() + rows.len() * wpr);
        words.push(
            (w as u32 | (rows.len() as u32) << 8 | lb << 16 | (chars.len() as u32) << 20) as i32,
        );
        words.extend_from_slice(&nums);
        for r in rows {
            let text = self.text(*r).as_bytes();
            if text.len() != w || w > 64 {
                // E915.
                return self.err(
                    *r,
                    format!(
                        "a sprite's rows are as wide as its first ({w} characters), at most 64"
                    ),
                );
            }
            let mut row = alloc::vec![0u32; wpr];
            for (x, &c) in text.iter().enumerate() {
                let Some(k) = chars.iter().position(|&kc| kc == c) else {
                    // E914.
                    let at = r.start + x;
                    return Err(Error::new(
                        at,
                        at + 1,
                        format!("`{}` is not in the sprite's key", c as char),
                    ));
                };
                row[x / ppw] |= (k as u32) << ((x % ppw) << lb);
            }
            words.extend(row.iter().map(|&v| v as i32));
        }
        let start = match self
            .tables
            .windows(words.len())
            .position(|win| win == words.as_slice())
        {
            Some(k) => k,
            None => {
                if self.tables.len() + words.len() > MAX_TABLE {
                    // E917.
                    return self.err(
                        kw,
                        format!(
                            "sprites and tables hold {MAX_TABLE} numbers in all — this would make {} (a sprite takes about one a 16 pixels, 8 with 5 or more key characters)",
                            self.tables.len() + words.len()
                        ),
                    );
                }
                self.tables.extend_from_slice(&words);
                self.tables.len() - words.len()
            }
        };
        if start > 255 {
            return self.err(kw, "sprites and tables hold 256 numbers in all");
        }
        self.sprites.push(Sprite {
            name,
            at: n.start,
            scope: self.scope,
            start,
        });
        self.end_of_statement()
    }

    /// `table NAME = n, n, …` at `kw`: numbers only (a `,` may end a line
    /// and the numbers go on), 256 in all.
    #[inline(never)]
    fn table(&mut self, kw: Token) -> Result<(), Error> {
        self.next();
        let n = self.peek();
        if n.tok != Tok::Name {
            return self.err(n, "a table needs a name, as in `table T = 1, 2, 3`");
        }
        let name = self.new_name(n)?;
        self.next();
        self.expect(Tok::Assign, "`=` (as in `table T = 1, 2, 3`)")?;
        let mut vals: Vec<i32> = Vec::new();
        loop {
            let neg = self.peek().tok == Tok::Minus;
            if neg {
                self.next();
            }
            let v = self.peek();
            let Tok::Num(x) = v.tok else {
                return self.err(
                    v,
                    "a table holds numbers only, up to 256 in all, as in `table T = 1, -2, 0.5`",
                );
            };
            self.next();
            vals.push(if neg { -x } else { x });
            if self.peek().tok != Tok::Comma {
                break;
            }
            self.next();
            self.skip_newlines();
        }
        // The same numbers already held (a part read again, or two tables
        // alike): stored once.
        let start = match self
            .tables
            .windows(vals.len())
            .position(|w| w == vals.as_slice())
        {
            Some(k) => k,
            None => {
                if self.tables.len() + vals.len() > MAX_TABLE {
                    return self.err(
                        kw,
                        format!(
                            "a table holds numbers only, up to {MAX_TABLE} in all — this would make {}",
                            self.tables.len() + vals.len()
                        ),
                    );
                }
                self.tables.extend_from_slice(&vals);
                self.tables.len() - vals.len()
            }
        };
        self.table_names.push(Table {
            name,
            at: n.start,
            scope: self.scope,
            start,
            len: vals.len(),
        });
        self.end_of_statement()
    }

    /// Whether `id` (a value from `start` to `end`) can be used inside
    /// `draw { }`: the same for every pixel.
    #[inline(never)]
    fn drawable(&self, id: Id, start: usize, end: usize) -> Result<(), Error> {
        if self.dag.level(id) < LV_ROW {
            return Ok(());
        }
        let why = INPUTS[self.dag.nodes[id as usize].why as usize];
        Err(Error::new(
            start,
            end.max(start),
            format!(
                "inside draw {{ }} only values that are the same for every pixel can be used \
                 (no x, y, px, py, i, ur, ug, ub, ul or ink), but this uses `{}` ({})",
                why.0, why.2
            ),
        ))
    }

    /// `draw { … }` at `kw`: once a frame, before the rows — values, `for`
    /// loops, `line(…)` and `dot(…)` into the canvas `ink` reads.
    #[inline(never)]
    fn draw_block(&mut self, kw: Token) -> Result<(), Error> {
        self.next();
        if self.in_layer() {
            return Err(self.misplaced(kw));
        }
        if self.record && self.drawn.is_some() {
            return self.err(
                kw,
                "there can be one `draw { }` block (the formula's or one part's: they share one canvas)",
            );
        }
        let open = self.peek();
        if open.tok != Tok::LBrace {
            return Err(self.expected(open, "`{` (as in `draw { … }`)"));
        }
        self.next();
        if self.record {
            self.drawn = Some(kw);
        }
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(self.too_deep());
        }
        self.in_draw = 1;
        let r = self.draw_lines(kw);
        self.in_draw = 0;
        self.depth -= 1;
        r?;
        self.next();
        self.end_of_statement()
    }

    /// The lines of a `draw { }` or a `for` (at `kw`) up to its `}`.
    fn draw_lines(&mut self, kw: Token) -> Result<(), Error> {
        self.skip_newlines();
        loop {
            let t = self.peek();
            match t.tok {
                Tok::RBrace => return Ok(()),
                Tok::Eof => {
                    return self.err(
                        kw,
                        if self.text(kw) == "for" {
                            "this `for {` is never closed — add `}`"
                        } else {
                            "this `draw {` is never closed — add `}`"
                        },
                    )
                }
                _ => {}
            }
            let word = if t.tok == Tok::Name { self.text(t) } else { "" };
            let next = self.tok(self.pos + 1).tok;
            if next == Tok::Assign {
                self.assignment(false)?;
            } else if word == "for" {
                self.for_loop(t)?;
            } else if PRIMITIVES.contains(&word) && next == Tok::LParen {
                self.primitive(t)?;
            } else if !self.closed() {
                return self.err(
                    kw,
                    if self.text(kw) == "for" {
                        "this `for {` is never closed — add `}`"
                    } else {
                        "this `draw {` is never closed — add `}`"
                    },
                );
            } else {
                return self.err(
                    t,
                    "inside draw { } go `name = value`, `for k < N { … }`, `line(…)` and `dot(…)` lines only",
                );
            }
            self.skip_newlines();
        }
    }

    /// Whether the block the cursor is in has its `}` ahead.
    fn closed(&self) -> bool {
        let mut depth = 0;
        for t in &self.toks[self.pos..] {
            match t.tok {
                Tok::LBrace => depth += 1,
                Tok::RBrace if depth == 0 => return true,
                Tok::RBrace => depth -= 1,
                _ => {}
            }
        }
        false
    }

    /// `for k < N { … }` at `kw`: `k` = 0, 1, … N − 1, N a whole number
    /// from 1 to 1024; two deep at most; 2048 turns a frame in all.
    #[inline(never)]
    fn for_loop(&mut self, kw: Token) -> Result<(), Error> {
        self.next();
        if self.in_draw > 2 {
            return self.err(kw, "`for` loops go two deep at most");
        }
        let (n, lt, num) = (self.peek(), self.tok(self.pos + 1), self.tok(self.pos + 2));
        let count = match (n.tok, lt.tok, num.tok) {
            (Tok::Name, Tok::Lt, Tok::Num(v)) if v & 0xffff == 0 && (ONE..=1024 * ONE).contains(&v) => {
                (v >> 16) as u32
            }
            _ => {
                return self.err(
                    kw,
                    "`for` takes a name and a count, e.g. `for k < 12 { … }` (a whole number from 1 to 1024)",
                )
            }
        };
        let name = self.text(n);
        if self
            .locals
            .iter()
            .any(|l| l.counter && l.name == name && self.visible(l.scope))
        {
            return self.err(n, format!("`{name}` is already a loop counter here"));
        }
        let name = self.new_name(n)?;
        self.pos += 3;
        let open = self.peek();
        if open.tok != Tok::LBrace {
            return Err(self.expected(open, "`{` (as in `for k < 12 { … }`)"));
        }
        self.next();
        let depth = (self.in_draw - 1) as usize;
        let turns = if depth == 0 {
            count
        } else {
            count * self.counts[0]
        };
        if self.record {
            self.turns += turns;
            if self.turns > crate::draw::TURNS {
                return self.err(
                    kw,
                    format!(
                        "this formula draws too much a frame ({} loop turns in all; the limit is {}) — fewer or shorter loops",
                        self.turns,
                        crate::draw::TURNS
                    ),
                );
            }
        }
        self.counts[depth] = count;
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(self.too_deep());
        }
        let outer = self.scope;
        self.scopes.push((outer, false));
        self.scope = (self.scopes.len() - 1) as u16;
        let ctr = self.dag.push(
            Node {
                code: K_COUNTER,
                a: 0,
                b: 0,
                c: 0,
                val: self.loops as i32,
                level: if depth == 0 { LV_LOOP1 } else { LV_LOOP2 },
                why: 0,
            },
            (n.start, n.end),
        )?;
        self.loops += 1;
        self.locals.push(Local {
            name,
            id: ctr,
            at: n.start,
            in_frame: false,
            counter: true,
            scope: self.scope,
        });
        if self.record {
            self.draw.push(DItem::For(ctr, count as u16));
        }
        self.in_draw += 1;
        let r = self.draw_lines(kw);
        self.in_draw -= 1;
        self.scope = outer;
        self.depth -= 1;
        r?;
        self.next();
        if self.record {
            self.draw.push(DItem::End);
        }
        self.end_of_statement()
    }

    /// `line(x0, y0, x1, y1[, v0[, v1]])` or `dot(x, y[, v])` at `t`.
    #[inline(never)]
    fn primitive(&mut self, t: Token) -> Result<(), Error> {
        let line = self.text(t) == "line";
        let (need, most) = if line { (4, 6) } else { (2, 3) };
        self.next();
        let open = self.next();
        let mut v = [0 as Id; 6];
        let mut k = 0;
        if self.peek().tok != Tok::RParen {
            loop {
                let start = self.peek().start;
                let e = self.expr(0)?;
                let end = self.toks[self.pos.saturating_sub(1)].end;
                self.drawable(e, start, end)?;
                if self.dag.grows(e) {
                    // E906.
                    return Err(Error::new(
                        start,
                        end.max(start),
                        "a value that grows without end can't be drawn at: use frac(t*k)*128, t % n or min(t, k)",
                    ));
                }
                if k < 6 {
                    v[k] = e;
                }
                k += 1;
                if self.peek().tok == Tok::Comma {
                    self.next();
                    continue;
                }
                break;
            }
        }
        let close = self.peek();
        if close.tok != Tok::RParen {
            return Err(self.not_closed(open, close, self.text(t)));
        }
        self.next();
        if k < need || k > most {
            return self.err(
                t,
                if line {
                    format!("`line` takes 4 values (x0, y0, x1, y1: pixels, y up from the bottom) and, if you like, the ink at each end (0 to 1), but got {k}")
                } else {
                    format!("`dot` takes 2 values (x, y: pixels, y up from the bottom) and, if you like, its ink (0 to 1), but got {k}")
                },
            );
        }
        let at = (t.start, close.end);
        if line {
            if k < 5 {
                v[4] = self.dag.constant(ONE, at)?;
            }
            if k < 6 {
                v[5] = v[4];
            }
        } else if k < 3 {
            v[2] = self.dag.constant(ONE, at)?;
        }
        if self.record {
            self.draw.push(if line {
                DItem::Line(v)
            } else {
                DItem::Dot([v[0], v[1], v[2]])
            });
        }
        self.end_of_statement()
    }

    /// The lines of a formula (`end` Eof) or a part (`end` RBrace, the
    /// cursor after its `{`): names, `frame { }`, `loop`, parts, layers and
    /// the colour.
    fn body(&mut self, end: Tok) -> Result<Body, Error> {
        let mut b = Body {
            colours: Vec::new(),
            top: None,
            layered: false,
            blocks: 0,
            loop_v: None,
            fps: None,
        };
        let mut last: Option<Token> = None;
        let mut seen_frame = false;
        self.skip_newlines();
        loop {
            let t = self.peek();
            if t.tok == end {
                break;
            }
            if let Some(ct) = last {
                return Err(Error::new(
                    ct.start,
                    ct.end,
                    if end == Tok::Eof {
                        "the colour has to be the last line (move this to the end)"
                    } else {
                        "the colour has to be the part's last line (move this to just before `}`)"
                    },
                ));
            }
            match self.keyword() {
                "loop" => {
                    self.next();
                    if b.loop_v.is_some() {
                        return self.err(t, "`loop` is already set");
                    }
                    let n = self.next();
                    let v = match n.tok {
                        Tok::Num(v) => v,
                        // E907 (none: it is the line's own).
                        Tok::Name if self.text(n) == "forever" => FOREVER,
                        _ => {
                            return self.err(
                                n,
                                "`loop` takes a number of seconds, e.g. `loop 8`, or `forever` for one that goes on without repeating",
                            )
                        }
                    };
                    if v != FOREVER && !(ONE / 10..=3600 * ONE).contains(&v) {
                        return self.err(n, "the loop must be between 0.1 and 3600 seconds");
                    }
                    b.loop_v = Some(v);
                    self.end_of_statement()?;
                }
                _ if self.is_fps() => {
                    let ms = self.fps_line()?;
                    if b.fps.replace(ms).is_some() {
                        return self.err(t, "`fps` is already set");
                    }
                }
                "frame" => self.frame_block(t, &mut seen_frame)?,
                "layer" => {
                    let (cs, ct) = self.layer(t)?;
                    b.colours.extend(cs);
                    b.top = Some(ct);
                    b.layered = true;
                    b.blocks += 1;
                }
                "part" => self.part(t)?,
                "draw" => self.draw_block(t)?,
                "table" => self.table(t)?,
                _ if self.is_sprite() => self.sprite(t)?,
                "for" => return Err(self.misplaced(t)),
                "use" => {
                    return self.err(
                        t,
                        "`use` puts a formula in by its name, as `part p { … }`, when you leave the line",
                    )
                }
                _ if self.is_assignment() => self.assignment(false)?,
                _ => {
                    let (cs, layered) = self.colour_or_value(t, end)?;
                    if b.blocks > 0 {
                        return Err(Error::new(
                            t.start,
                            self.toks[self.pos.saturating_sub(1)].end.max(t.end),
                            "with layers, every colour goes inside one: put this line in `layer { … }`",
                        ));
                    }
                    b.colours = cs;
                    b.layered = layered;
                    b.top = Some(t);
                    last = Some(t);
                }
            }
            self.skip_newlines();
        }
        Ok(b)
    }
}

/// What a compile also reports.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Info {
    /// Values set inside `frame { }` blocks (a part's once for each place
    /// it is drawn at).
    pub frame_values: usize,
    /// `layer` blocks written in the formula's own lines (0: it is one
    /// picture, its last line).
    pub layers: usize,
}

/// Compile `src` into `prog` (which is cleared first). On an error `prog`
/// is left empty.
pub fn compile_into(src: &str, prog: &mut Program) -> Result<Info, Error> {
    compile_layout(src, prog, true)
}

/// [`compile_into`] with (`runs`) or without the column run and the still
/// cache's run: without, every pixel step runs on every pixel of every
/// frame, as before they existed (the same bytes). For measuring a step's
/// cost on the device (`psi-probe`) and the estimate's tests.
#[doc(hidden)]
pub fn compile_layout(src: &str, prog: &mut Program, runs: bool) -> Result<Info, Error> {
    compile_with(src, prog, runs, true)
}

/// [`compile_layout`] with (`narrow`) or without the narrowed pixel steps
/// (`crate::range`: `add_w`, `mul_n`, `shr`, …; the same bytes): without,
/// the checked steps as before them, for `psi-probe`'s before and after.
#[doc(hidden)]
pub fn compile_with(
    src: &str,
    prog: &mut Program,
    runs: bool,
    narrow: bool,
) -> Result<Info, Error> {
    prog.clear();
    let mut r = compile_inner(src, prog, runs);
    if runs && r.as_ref().is_err_and(|e| e.msg == LAYOUT_BUG) {
        debug_assert!(false, "{LAYOUT_BUG}: {src:?}");
        prog.clear();
        r = compile_inner(src, prog, false);
    }
    if r.is_err() {
        prog.clear();
    } else if narrow {
        crate::range::narrow(prog);
    }
    r
}

fn compile_inner(src: &str, prog: &mut Program, runs: bool) -> Result<Info, Error> {
    if src.len() > MAX_SOURCE {
        return Err(Error::new(
            MAX_SOURCE,
            src.len(),
            format!("too long: a formula can be {MAX_SOURCE} characters at most"),
        ));
    }
    let toks = lex(src)?;
    let mut p = Parser {
        src,
        toks,
        pos: 0,
        dag: Dag {
            nodes: Vec::with_capacity(64),
            index: alloc::vec![NO; SLOTS],
            divs: Vec::new(),
            picks: Vec::new(),
            forever: false,
            ctx: Vec::new(),
            deps: Vec::new(),
        },
        locals: Vec::new(),
        depth: 0,
        scope: 0,
        scopes: alloc::vec![(0, false)],
        parts: Vec::new(),
        env: Env {
            at: None,
            t: None,
            loop_v: 0,
            ch: NO_CH,
        },
        work: 0,
        tables: Vec::new(),
        table_names: Vec::new(),
        sprites: Vec::new(),
        draw: Vec::new(),
        drawn: None,
        record: true,
        in_draw: 0,
        counts: [0; 2],
        loops: 0,
        turns: 0,
    };
    p.env.loop_v = p.loop_ahead(0).unwrap_or(8 * ONE);
    p.dag.forever = p.env.loop_v == FOREVER;
    let body = p.body(Tok::Eof)?;
    if let Some(v) = body.loop_v {
        prog.loop_ms = if v == FOREVER {
            0
        } else {
            ((v as i64 * 1000 + ONE as i64 / 2) >> 16) as u32
        };
    }
    // A part's `fps` (a whole formula pasted in) is left to the formula's.
    prog.frame_ms = body.fps.unwrap_or(0);
    let mut layers = body.colours;
    let (Some(ct), false) = (body.top, layers.is_empty()) else {
        let at = src.len();
        let msg = if p.locals.is_empty() && p.parts.is_empty() {
            "empty: write a colour, e.g. `hsv(x + t/4, 1, 1)`"
        } else {
            "the last line should be the colour, e.g. `rgb(r, g, b)`, `hsv(h, s, v)` or `pal(fire, k)`"
        };
        return Err(Error::new(at, at, msg));
    };
    let frame_values = p.locals.iter().filter(|l| l.in_frame).count();
    // `ink` with nothing drawn: always a mistake (unlike `ur` alone).
    if p.drawn.is_none() && p.dag.nodes.iter().any(|m| m.code == code::INK) {
        let t = (0..p.toks.len())
            .map(|k| p.toks[k])
            .find(|&t| t.tok == Tok::Name && p.text(t) == "ink")
            .unwrap_or(ct);
        return Err(Error::new(
            t.start,
            t.end,
            "`ink` needs a `draw { }` block to read from",
        ));
    }
    prog.tables[..p.tables.len()].copy_from_slice(&p.tables);
    if p.dag.forever {
        for c in &mut layers {
            grown_colour(&mut p.dag, c, (ct.start, ct.end))?;
        }
    }
    // A layer under one that covers everything (`over`, coverage 1) is never
    // seen: from the topmost such layer up. If it is the bottom one it is
    // written straight over the pixels (as v0's last line); otherwise the
    // picture under the formula shows through and every layer is blended.
    let dag = &p.dag;
    let opaque = |c: &Colour| {
        c.blend == fixed::BLEND_OVER
            && c.alpha
                .is_none_or(|a| dag.konst(a).is_some_and(|v| v >= ONE))
    };
    if let Some(k) = layers.iter().rposition(opaque) {
        layers.drain(..k);
    }
    let backdrop = !opaque(&layers[0]);
    let mut steps = Vec::with_capacity(layers.len());
    for (k, c) in layers.iter().enumerate() {
        let plain = k == 0 && !backdrop;
        // A scale that is a value over 2, 4, … 2^15 (`floor(q)/8`, a
        // multiply by 1/8): the value, and the division as a shift in the
        // colour step (one step fewer a pixel; the same bytes).
        let (mut scale, mut shift) = (c.scale, 0u8);
        if let Some(sid) = c.scale {
            let m = p.dag.nodes[sid as usize];
            if m.code == code::MUL && m.level == LV_PIXEL {
                for (x, kk) in [(m.a, m.b), (m.b, m.a)] {
                    if let Some(v) = p.dag.konst(kk) {
                        if v > 0 && v < ONE && (v as u32).is_power_of_two() {
                            scale = Some(x);
                            shift = (ONE as u32 / v as u32).trailing_zeros() as u8;
                            break;
                        }
                    }
                }
            }
        }
        let alpha = match (plain, c.alpha) {
            (true, _) => None,
            (false, Some(a)) => Some(a),
            (false, None) => Some(p.dag.constant(ONE, (ct.start, ct.end))?),
        };
        steps.push(Step {
            code: if plain {
                c.code
            } else {
                code::OUT_LAYER + (c.code - code::OUT_RGB) + 4 * c.blend
            },
            args: c.args,
            pal: c.pal,
            alpha,
            scale,
            shift,
        });
    }
    // A limit hit while laying out the steps is the formula's as a whole:
    // point at its (top) colour line.
    emit(&p.dag, &steps, &p.draw, backdrop, runs, prog).map_err(|e| Error {
        start: ct.start,
        end: ct.end,
        ..e
    })?;
    Ok(Info {
        frame_values,
        layers: body.blocks,
    })
}

/// A colour's values that grow without end (a `loop forever` formula's
/// `t`, `gray(t/60)`): an hsv's hue by its fraction (the hue is a turn),
/// the others held to 0..1 (the colour step clamps them so anyway); a
/// coverage, a palette's scale or a value with a part per pixel can't
/// (E905).
fn grown_colour(dag: &mut Dag, c: &mut Colour, at: (usize, usize)) -> Result<(), Error> {
    // Held to 0..1, as the colour step and the coverage hold it anyway.
    let unit = |dag: &mut Dag, v: Id| -> Result<Id, Error> {
        if !dag.is_wide(v) {
            return Err(Error::new(at.0, at.1, GROWN_COLOUR));
        }
        let (z, one) = (dag.constant(0, at)?, dag.constant(ONE, at)?);
        let lo = dag.op(code::MAX, [v, z, 0], at)?;
        dag.op(code::MIN, [lo, one, 0], at)
    };
    let n = code::arity(c.code);
    for k in 0..n {
        let v = c.args[k];
        if !dag.grows(v) {
            continue;
        }
        c.args[k] = if c.code == code::OUT_HSV && k == 0 {
            dag.op(code::FRAC, [v, 0, 0], at)?
        } else {
            unit(dag, v)?
        };
    }
    if let Some(a) = c.alpha.filter(|&a| dag.grows(a)) {
        c.alpha = Some(unit(dag, a)?);
    }
    if c.scale.is_some_and(|k| dag.grows(k)) {
        return Err(Error::new(at.0, at.1, GROWN_COLOUR));
    }
    Ok(())
}

/// E912/E913: a sprite's key.
const SPRITE_KEY: &str =
    "a sprite starts with its key: `key . 0, R 1, D 2` (each character, then the number it reads as)";
const SPRITE_KEY_ENTRY: &str = "a key is a character and its number, as `R 1` or `. -0.5`, separated by commas (not ; { } , - / or a space)";

/// E905.
const GROWN_COLOUR: &str =
    "a colour can't take a value that grows without end and changes across the panel \
(`t` in a `loop forever` formula): use frac(…) of it, or min(t/k, 1)";

/// A colour step to lay out: its code (plain or a layer's), its values,
/// the palette and the coverage (a layer's).
struct Step {
    code: u8,
    args: [Id; 3],
    pal: u8,
    alpha: Option<Id>,
    /// `pal(P, v) * k`'s k, and a shift its value is divided by (`k = x/8`:
    /// x, 3).
    scale: Option<Id>,
    shift: u8,
}

impl Step {
    /// What it reads, in operand order (a palette's scale, then the
    /// coverage last).
    fn reads(&self) -> impl Iterator<Item = Id> + '_ {
        self.args[..code::arity(self.code)]
            .iter()
            .copied()
            .chain(self.scale)
            .chain(self.alpha)
    }
}

/// Lay the reachable nodes out as steps: scalar slots for the constants
/// and the per-frame and per-row values, lane registers for the per-pixel
/// ones (linear scan: a register is free again after its value's last
/// use, and a step never writes a register it reads). The per-pixel values
/// go in the order the layers need them: each layer's colour step right
/// after the values it is the first to read. The draw stage, if something
/// reads `ink`, goes between the per-frame and the per-row steps in the
/// order it is written: each loop's values (those that depend on its
/// counter) at the top of its body.
#[inline(never)]
fn emit(
    dag: &Dag,
    steps: &[Step],
    draw: &[DItem],
    backdrop: bool,
    runs: bool,
    prog: &mut Program,
) -> Result<(), Error> {
    let nodes = &dag.nodes;
    let n = nodes.len();
    let too_long = |steps: usize| {
        Error::new(
            0,
            0,
            format!(
                "this formula is too long for the panel ({steps} steps; the limit is {MAX_OPS}) — simplify it"
            ),
        )
    };
    // Reachability, and the first colour step each value is needed for
    // (children always precede their parents).
    const NONE: usize = usize::MAX;
    let mut need = alloc::vec![NONE; n];
    for (k, st) in steps.iter().enumerate() {
        for a in st.reads() {
            need[a as usize] = need[a as usize].min(k);
        }
    }
    let close = |need: &mut [usize]| {
        for i in (0..n).rev() {
            let m = &nodes[i];
            if need[i] == NONE || m.code >= K_CONST {
                continue;
            }
            for x in reads(dag, i) {
                need[x as usize] = need[x as usize].min(need[i]);
            }
        }
    };
    close(&mut need);
    // The draw stage only when something shown reads `ink` (else, like a
    // value nothing uses, it is left out).
    let inked = (0..n).any(|i| need[i] != NONE && nodes[i].code == code::INK);
    let draw = if inked { draw } else { &[] };
    for item in draw {
        let ids: &[Id] = match item {
            DItem::Line(v) => v,
            DItem::Dot(v) => v,
            _ => &[],
        };
        for &x in ids {
            need[x as usize] = 0;
        }
    }
    close(&mut need);
    if inked {
        prog.uses |= INKED;
    }
    let live = |i: usize| need[i] != NONE;
    // A value that grows without end with a part per pixel, where nothing
    // took it apart (E901).
    if (0..n).any(|i| live(i) && nodes[i].code == K_SPLIT) {
        return Err(Error::new(0, 0, GROWS));
    }
    let is_op = |i: usize| live(i) && nodes[i].code < K_CONST;
    let control: usize = draw
        .iter()
        .map(|d| match d {
            DItem::Line(_) => 2,
            _ => 1,
        })
        .sum();
    let total = (0..n).filter(|&i| is_op(i)).count() + steps.len() + control;
    if total > MAX_OPS {
        return Err(too_long(total));
    }
    // Where each node's value lives.
    let mut loc = alloc::vec![0u8; n];
    // Which nodes have a place yet: a step may read only those (an operand
    // without one would read slot 0, `t`, silently).
    let mut set = alloc::vec![false; n];
    let mut next_slot = S_FIRST_FREE as usize;
    // `seed` has the first free slot; the wide noises' offsets the slots
    // after it (`noise_slot`; `seed`'s kept for it then).
    if (0..n).any(|i| live(i) && nodes[i].code == K_INPUT && nodes[i].val == S_SEED as i32) {
        prog.uses |= SEEDED;
        next_slot += 1;
    }
    let nctx = (0..n)
        .filter(|&i| is_op(i) && is_noise_o(nodes[i].code))
        .map(|i| (nodes[i].code - code::NOISE_O) as usize + 1)
        .max()
        .unwrap_or(0);
    if nctx > 0 {
        next_slot = noise_slot(nctx, 0);
    }
    // Wide slots: 0 is `t`.
    let mut next_wide = 1usize;
    fn new_slot(next_slot: &mut usize, v: Option<i32>, prog: &mut Program) -> Result<u8, Error> {
        if *next_slot >= MAX_SCALARS {
            return Err(Error::new(
                0,
                0,
                format!(
                    "this formula has too many numbers and steps (more than {MAX_SCALARS} values) — simplify it"
                ),
            ));
        }
        prog.init[*next_slot] = v.unwrap_or(0);
        *next_slot += 1;
        Ok((*next_slot - 1) as u8)
    }
    for i in 0..n {
        if live(i) && nodes[i].code == K_CONST {
            loc[i] = new_slot(&mut next_slot, Some(nodes[i].val), prog)?;
            set[i] = true;
        } else if live(i) && nodes[i].code == K_INPUT {
            loc[i] = if nodes[i].val == WIDE_T {
                0
            } else {
                nodes[i].val as u8
            };
            set[i] = true;
            if nodes[i].val as u8 == LANE | L_I {
                prog.uses |= USES_I;
            }
        } else if is_op(i) && (code::U_R..=code::U_L).contains(&nodes[i].code) {
            prog.uses |= UNDER;
        }
    }
    if backdrop {
        prog.uses |= BACKDROP;
    }
    let mut ops: Vec<Op> = Vec::new();
    // A step for node `m` into `d`; a pick's table of its choices' slots
    // goes in constant slots of its own, one for every pick of the same
    // choices (`tabs`: where each went, 0 for not yet; the choices all
    // have a slot by then: they come before it and run once a frame or a
    // row).
    let mut tabs = alloc::vec![0u8; dag.picks.len()];
    let mk = |m: &Node,
              d: u8,
              loc: &[u8],
              set: &[bool],
              ns: &mut usize,
              tabs: &mut [u8],
              prog: &mut Program|
     -> Result<Op, Error> {
        let n = code::arity(m.code);
        let unset = [m.a, m.b, m.c][..n].iter().any(|&x| !set[x as usize])
            || (m.code == code::PICK && dag.picks[m.b as usize].iter().any(|&x| !set[x as usize]))
            || (is_noise_o(m.code) && dag.deps[m.val as usize].iter().any(|&x| !set[x as usize]));
        if unset {
            return Err(Error::new(0, 0, String::from(LAYOUT_BUG)));
        }
        let l = |k: usize, x: Id| if n > k { loc[x as usize] } else { 0 };
        let (mut b, mut c) = (l(1, m.b), l(2, m.c));
        if m.code == code::TAB {
            (b, c) = (m.b as u8, m.c as u8);
        }
        if matches!(m.code, code::W_CMP | code::W_MINMAX | code::SPR) {
            c = m.c as u8;
        }
        if m.code == code::PICK {
            let opts = &dag.picks[m.b as usize];
            let tab = &mut tabs[m.b as usize];
            if *tab == 0 {
                *tab = *ns as u8;
                for &o in opts {
                    new_slot(ns, Some(loc[o as usize] as i32), prog)?;
                }
            }
            (b, c) = (*tab, opts.len() as u8);
        }
        Ok(Op {
            code: m.code,
            d,
            a: l(0, m.a),
            b,
            c,
        })
    };
    // Each loop's own values: those whose deepest counter is its (a value
    // at a loop's level takes the loop of its operand at that level).
    let mut owner = alloc::vec![u16::MAX; n];
    for i in 0..n {
        let m = &nodes[i];
        if m.code == K_COUNTER {
            owner[i] = m.val as u16;
        } else if m.code < K_CONST && (m.level == LV_LOOP1 || m.level == LV_LOOP2) {
            for &x in [m.a, m.b, m.c][..code::arity(m.code)].iter() {
                if nodes[x as usize].level == m.level {
                    owner[i] = owner[x as usize];
                    break;
                }
            }
        }
    }
    for level in [LV_FRAME, LV_ROW] {
        for i in 0..n {
            let m = &nodes[i];
            if is_op(i) && m.level == level {
                // A wide value has a wide slot; a noise's offset its place.
                let d = if code::is_wide(m.code) && code::wide_ops(m.code).0 {
                    if next_wide >= WIDE {
                        return Err(Error::new(
                            0,
                            0,
                            format!("this formula has too many values that grow without end (more than {}) — simplify it", WIDE - 1),
                        ));
                    }
                    next_wide += 1;
                    (next_wide - 1) as u8
                } else if m.code == code::W_LAT {
                    noise_slot(m.val as usize / 3, m.val as usize % 3) as u8
                } else {
                    new_slot(&mut next_slot, None, prog)?
                };
                loc[i] = d;
                set[i] = true;
                ops.push(mk(m, d, &loc, &set, &mut next_slot, &mut tabs, prog)?);
            }
        }
        if level == LV_FRAME {
            prog.frame_end = ops.len().min(MAX_OPS) as u16;
            // The draw stage: FOR (its END patched in when it comes), the
            // loop's values, its lines and inner loops; END back to just
            // after the FOR.
            let mut open: Vec<(usize, u8, u8)> = Vec::new();
            for item in draw {
                match *item {
                    DItem::For(ctr, count) => {
                        let cs = new_slot(&mut next_slot, Some((count as i32) << 16), prog)?;
                        let ks = new_slot(&mut next_slot, None, prog)?;
                        loc[ctr as usize] = ks;
                        set[ctr as usize] = true;
                        open.push((ops.len(), ks, cs));
                        ops.push(Op {
                            code: code::FOR,
                            d: ks,
                            a: cs,
                            b: 0,
                            c: 0,
                        });
                        let lp = nodes[ctr as usize].val as u16;
                        for i in 0..n {
                            if is_op(i) && owner[i] == lp {
                                let d = new_slot(&mut next_slot, None, prog)?;
                                loc[i] = d;
                                set[i] = true;
                                ops.push(mk(
                                    &nodes[i],
                                    d,
                                    &loc,
                                    &set,
                                    &mut next_slot,
                                    &mut tabs,
                                    prog,
                                )?);
                            }
                        }
                    }
                    DItem::End => {
                        let Some((at, ks, cs)) = open.pop() else {
                            continue;
                        };
                        ops[at].b = ops.len().min(255) as u8;
                        ops.push(Op {
                            code: code::END,
                            d: 0,
                            a: ks,
                            b: cs,
                            c: (at + 1).min(255) as u8,
                        });
                    }
                    DItem::Line(v) => {
                        let l = |k: usize| loc[v[k] as usize];
                        ops.push(Op {
                            code: code::LINE_FROM,
                            d: 0,
                            a: l(0),
                            b: l(1),
                            c: l(4),
                        });
                        ops.push(Op {
                            code: code::LINE_TO,
                            d: 0,
                            a: l(2),
                            b: l(3),
                            c: l(5),
                        });
                    }
                    DItem::Dot(v) => ops.push(Op {
                        code: code::DOT,
                        d: 0,
                        a: loc[v[0] as usize],
                        b: loc[v[1] as usize],
                        c: loc[v[2] as usize],
                    }),
                }
            }
            prog.draw_end = ops.len().min(MAX_OPS) as u16;
        } else {
            prog.row_end = ops.len().min(MAX_OPS) as u16;
        }
    }
    // Per pixel: the order (a value, or colour step k), each value's last
    // reader in it, then registers by linear scan. With values for the
    // still cache (`still::choose`), first the per-frame values the cached
    // run reads, the marker, the run (skipped once the cache holds its
    // values), then the rest as usual.
    let px_op = |i: usize| is_op(i) && nodes[i].level == LV_PIXEL;
    // The per-column run: values of `x`/`px` and per-frame values only,
    // worked out on a frame's first row and kept (`Item::Column`), when
    // they are worth a step; then the still cache's values. Each tried
    // with the registers, dropped when they don't fit.
    let col = column_nodes(dag);
    let in_col: Vec<bool> = (0..n).map(|i| px_op(i) && col[i]).collect();
    let col_ns: u32 = (0..n)
        .filter(|&i| in_col[i])
        .map(|i| value_ns(nodes[i].code))
        .sum();
    let none = alloc::vec![false; n];
    let mut layout = None;
    for (use_col, use_still) in [(true, true), (true, false), (false, true), (false, false)] {
        let (use_col, use_still) = (use_col && runs, use_still && runs);
        let use_col = use_col && col_ns >= COLUMN_GAIN_NS;
        let excl = if use_col { &in_col } else { &none };
        let markers = usize::from(use_col);
        let cached = if use_still && total + markers < MAX_OPS {
            choose_still(dag, steps, &px_op, excl)
        } else {
            Vec::new()
        };
        if (use_col && col_ns < COLUMN_GAIN_NS) || (use_still && cached.is_empty() && !use_col) {
            continue;
        }
        // The markers are steps too: at the step limit, the plain layout.
        if total + markers + usize::from(!cached.is_empty()) > MAX_OPS {
            continue;
        }
        let lay = pixel_seq(dag, steps, &px_op, &need, &cached, excl, use_col);
        if let Some(r) = registers(nodes, steps, &lay.seq, &lay.pinned) {
            layout = Some((lay, cached, r));
            break;
        }
    }
    let Some((lay, cached, (regs, used))) = layout else {
        return Err(Error::new(
            0,
            0,
            format!("this formula keeps too many values at once (more than {REGS}) — simplify it"),
        ));
    };
    let (seq, run_len, col_len) = (lay.seq, lay.run_len, lay.col_len);
    let mut marker = None;
    for (at, item) in seq.iter().enumerate() {
        match *item {
            Item::Column => ops.push(Op {
                code: code::COLUMN,
                d: 0,
                a: col_len as u8,
                b: 0,
                c: 0,
            }),
            Item::Still => {
                marker = Some(ops.len());
                ops.push(Op {
                    code: code::STILL,
                    d: cached.len() as u8,
                    a: run_len as u8,
                    b: 0,
                    c: 0,
                });
            }
            Item::Value(i) => {
                let d = LANE | (L_REG0 + regs[at]);
                loc[i] = d;
                set[i] = true;
                ops.push(mk(
                    &nodes[i],
                    d,
                    &loc,
                    &set,
                    &mut next_slot,
                    &mut tabs,
                    prog,
                )?);
            }
            Item::Colour(k) => {
                let st = &steps[k];
                if st.reads().any(|x| !set[x as usize]) {
                    return Err(Error::new(0, 0, String::from(LAYOUT_BUG)));
                }
                let l = |x: Id| loc[x as usize];
                let (kind, _) = code::colour_parts(st.code);
                let cover = st.alpha.map_or(0, l);
                let rgb = kind == code::OUT_RGB || kind == code::OUT_HSV;
                // A scaled palette: the flag on its palette, the scale in c.
                let (pal, scale) = match st.scale {
                    Some(k) if kind == code::OUT_PAL => {
                        (st.pal | PAL_SCALED | (st.shift << PAL_SHIFT), l(k))
                    }
                    _ => (st.pal, 0),
                };
                ops.push(Op {
                    code: st.code,
                    d: if rgb { cover } else { pal },
                    a: l(st.args[0]),
                    b: if rgb { l(st.args[1]) } else { cover },
                    c: if rgb { l(st.args[2]) } else { scale },
                });
            }
        }
        if ops.len() > MAX_OPS {
            return Err(too_long(ops.len()));
        }
    }
    // The marker's skip: the run is every value after it up to the first
    // that isn't in it (`still_seq` puts them together); its registers.
    if let Some(m) = marker {
        debug_assert!(m + run_len < ops.len());
        ops[m].b = loc[cached[0]];
        ops[m].c = cached.get(1).map_or(0, |&c| loc[c]);
    }
    prog.pixel_end = ops.len() as u16;
    prog.ops[..ops.len()].copy_from_slice(&ops);
    prog.n_scalars = next_slot as u8;
    prog.regs = used as u8;
    Ok(())
}

/// What a step reading a value with no place yet says (never seen: a bug in
/// the layout; `compile_layout` then lays the formula out without its
/// runs).
const LAYOUT_BUG: &str = "this formula could not be laid out (a bug in Psi) — please report it";

/// A pixel step to lay out: a value, colour step k, or the still cache's
/// marker.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Item {
    Value(usize),
    Colour(usize),
    Still,
    Column,
}

/// A wide noise's code.
fn is_noise_o(c: u8) -> bool {
    (code::NOISE_O..=code::NOISE_O_LAST).contains(&c)
}

/// A value's operands that are nodes (a pick's choices included, and the
/// steps a wide noise needs before it).
fn reads(dag: &Dag, i: usize) -> impl Iterator<Item = Id> + '_ {
    let m = &dag.nodes[i];
    let k = if m.code < K_CONST {
        code::arity(m.code)
    } else {
        0
    };
    let picks: &[Id] = if m.code == code::PICK {
        &dag.picks[m.b as usize]
    } else if is_noise_o(m.code) {
        &dag.deps[m.val as usize]
    } else {
        &[]
    };
    [m.a, m.b, m.c]
        .into_iter()
        .take(k)
        .chain(picks.iter().copied())
}

/// Registers for the values of `seq` by linear scan (a register is free
/// again after its value's last use, and a step never writes a register it
/// reads): the register of each item (0 for the others) and how many were
/// used, `None` when more than [`REGS`] are live at once.
fn registers(
    nodes: &[Node],
    steps: &[Step],
    seq: &[Item],
    pinned: &[bool],
) -> Option<(Vec<u8>, usize)> {
    let n = nodes.len();
    let mut last_use = alloc::vec![0usize; n];
    for (at, item) in seq.iter().enumerate() {
        match *item {
            Item::Value(i) => {
                let m = &nodes[i];
                for &x in [m.a, m.b, m.c][..code::arity(m.code)].iter() {
                    last_use[x as usize] = at;
                }
            }
            Item::Colour(k) => {
                for x in steps[k].reads() {
                    last_use[x as usize] = at;
                }
            }
            Item::Still | Item::Column => {}
        }
    }
    // A kept column value is never given up: the rows after the first
    // read it without working it out again.
    for (i, l) in last_use.iter_mut().enumerate() {
        if pinned[i] {
            *l = usize::MAX;
        }
    }
    let mut regs = alloc::vec![0u8; seq.len()];
    let mut free: Vec<u8> = (0..REGS as u8).rev().collect();
    let mut holder: [Option<usize>; REGS] = [None; REGS];
    let mut used = 0usize;
    for (at, item) in seq.iter().enumerate() {
        let wrote = match *item {
            Item::Value(i) => {
                let r = free.pop()?;
                used = used.max(REGS - free.len());
                regs[at] = r;
                holder[r as usize] = Some(i);
                Some(i)
            }
            _ => None,
        };
        // Free the registers whose values this step read for the last time
        // (after picking the destination, so it is never one of them).
        for (reg, h) in holder.iter_mut().enumerate() {
            if let Some(v) = *h {
                if Some(v) != wrote && last_use[v] <= at {
                    *h = None;
                    free.push(reg as u8);
                }
            }
        }
    }
    Some((regs, used))
}

/// A cached value must save at least this much a pixel (ns on the ESP32,
/// `program::value_ns`): ~1.6 ms a frame for 16 KB of heap.
const STILL_GAIN_NS: u32 = 400;

/// Still values weighed for the cache, the heaviest by an upper bound
/// (bounds the compile time: each is a walk of the formula per lane).
const STILL_CANDIDATES: usize = 24;

/// Whether each node is *still*: worked out from the pixel's position
/// (`x y px py i`) and constants only, the same every frame.
fn still_nodes(dag: &Dag) -> Vec<bool> {
    let nodes = &dag.nodes;
    let mut still = alloc::vec![false; nodes.len()];
    for i in 0..nodes.len() {
        let m = &nodes[i];
        still[i] = match m.code {
            K_CONST => true,
            K_INPUT => {
                let v = m.val as u8;
                v == LANE | L_X || v == LANE | L_PX || v == LANE | L_I || v == S_Y || v == S_PY
            }
            K_COUNTER | K_SPLIT => false,
            // (A wide noise's offsets move every frame.)
            c if (code::U_R..=code::INK).contains(&c) || is_noise_o(c) => false,
            _ => reads(dag, i).all(|x| still[x as usize]),
        };
    }
    still
}

/// The per-pixel values worked out every frame when the values in
/// `cached` come from the still cache: everything a colour step reads,
/// through anything but a cached value.
fn per_frame(dag: &Dag, steps: &[Step], cached: &[usize]) -> Vec<bool> {
    let n = dag.nodes.len();
    let mut f = alloc::vec![false; n];
    for st in steps {
        for a in st.reads() {
            f[a as usize] = true;
        }
    }
    for i in (0..n).rev() {
        if !f[i] || cached.contains(&i) || dag.nodes[i].code >= K_CONST {
            continue;
        }
        for x in reads(dag, i) {
            f[x as usize] = true;
        }
    }
    f
}

/// The values for the still cache, best first (at most
/// [`crate::still::STILL_LANES`]): still per-pixel values chosen greedily
/// by what they save a pixel per frame, each at least [`STILL_GAIN_NS`].
fn choose_still(
    dag: &Dag,
    steps: &[Step],
    px_op: &dyn Fn(usize) -> bool,
    col: &[bool],
) -> Vec<usize> {
    let nodes = &dag.nodes;
    let n = nodes.len();
    let still = still_nodes(dag);
    // (A value of the column run is worked out once a frame a column
    // anyway: never cached, and nothing to save.)
    let px_op = |i: usize| px_op(i) && !col[i];
    // What each still value's own steps cost at most (shared steps counted
    // in every reader: an upper bound, to leave out the cheap ones early).
    let mut sub = alloc::vec![0u32; n];
    for i in 0..n {
        if px_op(i) && still[i] {
            sub[i] = value_ns(nodes[i].code)
                + reads(dag, i)
                    .map(|x| sub[x as usize])
                    .fold(0u32, u32::saturating_add);
        }
    }
    // Any still value may be the one kept (often one inside: `q` read by
    // `q/64` and `frac(q)`, both still too); the heaviest few are weighed.
    // (The heaviest by a scan, not a sort: a sort's code is ~3 KB of wasm.)
    let mut cands: Vec<usize> = Vec::new();
    for i in (0..n).filter(|&i| px_op(i) && still[i] && sub[i] >= STILL_GAIN_NS) {
        if cands.len() < STILL_CANDIDATES {
            cands.push(i);
        } else if let Some(m) = (0..cands.len()).min_by_key(|&k| sub[cands[k]]) {
            if sub[cands[m]] < sub[i] {
                cands[m] = i;
            }
        }
    }
    if cands.is_empty() {
        return Vec::new();
    }
    let cost = |cached: &[usize]| -> u32 {
        let f = per_frame(dag, steps, cached);
        (0..n)
            .filter(|&i| f[i] && px_op(i) && !cached.contains(&i))
            .map(|i| value_ns(nodes[i].code))
            .sum()
    };
    let mut cached: Vec<usize> = Vec::new();
    let mut now = cost(&cached);
    for _ in 0..crate::still::STILL_LANES {
        let mut best: Option<(usize, u32)> = None;
        for &c in &cands {
            if cached.contains(&c) {
                continue;
            }
            cached.push(c);
            let ok = still_run(dag, steps, &px_op, &cached).is_some();
            let v = cost(&cached);
            cached.pop();
            if !ok {
                continue;
            }
            if best.is_none_or(|(_, b)| v < b) {
                best = Some((c, v));
            }
        }
        match best {
            Some((c, v)) if now.saturating_sub(v) >= STILL_GAIN_NS => {
                cached.push(c);
                now = v;
            }
            _ => break,
        }
    }
    cached
}

/// The still cache's run for `cached` (the cached values and the steps only
/// they need) and the per-frame values the run reads (laid out before its
/// marker), or `None` when those can't go before it: one of them reads a
/// cached value (`ab = abs(ang)` read by the frame and by a cached
/// `sqrt(ab*2)`, with `ang` cached too) — skipping the run would leave it
/// reading a register nothing wrote.
fn still_run(
    dag: &Dag,
    steps: &[Step],
    px: &dyn Fn(usize) -> bool,
    cached: &[usize],
) -> Option<(Vec<bool>, Vec<bool>)> {
    let n = dag.nodes.len();
    let f = per_frame(dag, steps, cached);
    let mut run = alloc::vec![false; n];
    let mut first = alloc::vec![false; n];
    for &c in cached {
        run[c] = true;
    }
    for i in (0..n).rev() {
        if run[i] {
            for x in reads(dag, i) {
                if px(x as usize) && !f[x as usize] {
                    run[x as usize] = true;
                }
            }
        }
    }
    for i in (0..n).rev() {
        if run[i] || first[i] {
            for x in reads(dag, i) {
                let x = x as usize;
                if px(x) && !run[x] {
                    first[x] = true;
                }
            }
        }
    }
    // A value before the marker reads only values before it (or rows,
    // columns, constants): never one in the run.
    let ok = (0..n).all(|i| !first[i] || reads(dag, i).all(|x| !run[x as usize]));
    ok.then_some((run, first))
}

/// A laid-out pixel section: the order, the cached run's and the column
/// run's lengths, and the values kept in their registers for the frame.
struct PixelSeq {
    seq: Vec<Item>,
    run_len: usize,
    col_len: usize,
    pinned: Vec<bool>,
}

/// The pixel order: the column run (with `col`: its marker and every
/// value of `x`, `px` and per-frame values, in node order), then with the
/// still cache the per-frame values the cached run reads, the marker and
/// the run (the cached values and the steps only they need), then the other
/// values before the colour step that first needs them, as without either.
fn pixel_seq(
    dag: &Dag,
    steps: &[Step],
    px_op: &dyn Fn(usize) -> bool,
    need: &[usize],
    cached: &[usize],
    in_col: &[bool],
    col: bool,
) -> PixelSeq {
    let n = dag.nodes.len();
    let mut seq: Vec<Item> = Vec::new();
    let mut pinned = alloc::vec![false; n];
    let mut col_len = 0;
    if col {
        seq.push(Item::Column);
        seq.extend((0..n).filter(|&i| in_col[i]).map(Item::Value));
        col_len = seq.len() - 1;
        // Kept: the column values something after the run reads.
        for i in 0..n {
            if px_op(i) && !in_col[i] {
                for x in reads(dag, i) {
                    pinned[x as usize] |= in_col[x as usize];
                }
            }
        }
        for st in steps {
            for x in st.reads() {
                pinned[x as usize] |= in_col[x as usize];
            }
        }
    }
    let px = |i: usize| px_op(i) && !(col && in_col[i]);
    let mut run = alloc::vec![false; n];
    let mut first = alloc::vec![false; n];
    let mut run_len = 0;
    if !cached.is_empty() {
        // (`choose_still` took only sets this lays out: see `still_run`.)
        if let Some((r, f)) = still_run(dag, steps, &px, cached) {
            (run, first) = (r, f);
            seq.extend((0..n).filter(|&i| first[i]).map(Item::Value));
            seq.push(Item::Still);
            let before = seq.len();
            seq.extend((0..n).filter(|&i| run[i]).map(Item::Value));
            run_len = seq.len() - before;
        }
    }
    for k in 0..steps.len() {
        for i in 0..n {
            if px(i) && need[i] == k && !run[i] && !first[i] {
                seq.push(Item::Value(i));
            }
        }
        seq.push(Item::Colour(k));
    }
    PixelSeq {
        seq,
        run_len,
        col_len,
        pinned,
    }
}

/// The column run is laid out only when its values cost at least this a
/// pixel (ns: its marker is a step of the 256, and its kept values hold
/// registers for the whole row).
const COLUMN_GAIN_NS: u32 = 130;

/// Whether each node is the same down a column: worked out from `x`, `px`,
/// constants and per-frame values only.
fn column_nodes(dag: &Dag) -> Vec<bool> {
    let nodes = &dag.nodes;
    let mut col = alloc::vec![false; nodes.len()];
    for i in 0..nodes.len() {
        let m = &nodes[i];
        col[i] = match m.level {
            LV_CONST | LV_FRAME => true,
            LV_PIXEL => match m.code {
                K_INPUT => {
                    let v = m.val as u8;
                    v == LANE | L_X || v == LANE | L_PX
                }
                K_CONST => true,
                K_COUNTER => false,
                c if (code::U_R..=code::INK).contains(&c) => false,
                _ => reads(dag, i).all(|x| col[x as usize]),
            },
            _ => false,
        };
    }
    col
}

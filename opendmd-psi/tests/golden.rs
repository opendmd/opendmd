//! Golden frames, the never-panic fuzz, and host/wasm parity: these run
//! natively under `cargo test` and compiled to wasm32 under node
//! (`wasm-pack test --node`, `make test-wasm`) with the same expected
//! hashes, so a pass on both is bit-identical output.

use opendmd_psi::golden::{fnv, inputs, picture, DRAWN, FRAMES as GOLDEN, OVER};
use opendmd_psi::{
    compile, render_rows, render_rows_ink, render_rows_still, render_with, sketches, Ink, Program,
    Scratch, Still, FONT_4X6, FONT_5X7, FONT_BOLD, H, MAX_OPS, OWN, W,
};

#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test as test;

fn frame<const N: usize>(p: &Program, ms: u32) -> Vec<u8> {
    let mut s = Scratch::<N>::new();
    let mut out = vec![0u8; W * H * 3];
    render_with::<N>(p, &mut s, &inputs(ms), &OWN, &mut out);
    out
}

/// The frame's hash at both lane widths (which must agree).
fn hash(src: &str, ms: u32) -> u64 {
    let p = compile(src).unwrap_or_else(|e| panic!("{}", e.render(src)));
    let a = frame::<128>(&p, ms);
    let b = frame::<64>(&p, ms);
    assert!(
        a == b,
        "lane widths 128 and 64 differ for {src:?} at {ms} ms"
    );
    fnv(&a)
}

#[test]
fn golden_frames() {
    let mut bad = Vec::new();
    for &(name, src, ms, want) in GOLDEN {
        let got = hash(src, ms);
        if got != want {
            bad.push(format!("    (\"{name}\", …, {ms}, {got:#x}),"));
        }
    }
    assert!(
        bad.is_empty(),
        "golden frames differ (got):\n{}",
        bad.join("\n")
    );
}

fn frame_over<const N: usize>(p: &Program, ms: u32, under: Option<&[u8]>) -> Vec<u8> {
    let mut s = Scratch::<N>::new();
    let mut out = vec![0x5a; W * H * 3];
    render_rows::<N>(p, &mut s, &inputs(ms), &OWN, under, u32::MAX, &mut out);
    out
}

/// A frame with the draw stage's canvas (what every player gives a
/// formula that draws).
fn frame_ink<const N: usize>(p: &Program, ms: u32, under: Option<&[u8]>) -> Vec<u8> {
    let mut s = Scratch::<N>::new();
    let mut ink = Ink::boxed();
    let mut out = vec![0x5a; W * H * 3];
    render_rows_ink::<N>(
        p,
        &mut s,
        Some(&mut ink),
        &inputs(ms),
        &OWN,
        under,
        u32::MAX,
        &mut out,
    );
    out
}

/// Frames at each of `ms` with a still cache (kept on the first, drawn
/// from after: the second frame and on skip the cached steps), checked
/// against [`frame_ink`] without one; also a partial redraw (some rows) from
/// the kept values.
fn still_same<const N: usize>(src: &str, p: &Program, ms: &[u32], under: Option<&[u8]>) {
    let mut s = Scratch::<N>::new();
    let mut ink = Ink::boxed();
    let mut st = Still::new();
    assert!(st.take(p), "{src:?}: no still lanes");
    for (k, &m) in ms.iter().enumerate() {
        let mut out = vec![0x5a; W * H * 3];
        render_rows_still::<N>(
            p,
            &mut s,
            Some(&mut ink),
            Some(&mut st),
            &inputs(m),
            &OWN,
            under,
            u32::MAX,
            &mut out,
        );
        assert!(st.holds(p), "{src:?}: the cache kept nothing");
        let want = frame_ink::<N>(p, m, under);
        assert!(
            out == want,
            "{src:?} at {m} ms (frame {k}, {N} lanes): the still cache drew other bytes"
        );
        // Some rows again, from the kept values (the canvas as left).
        let mut part = out.clone();
        render_rows_still::<N>(
            p,
            &mut s,
            Some(&mut ink),
            Some(&mut st),
            &inputs(m),
            &OWN,
            under,
            0x8000_0101,
            &mut part,
        );
        assert!(part == want, "{src:?} at {m} ms: a partial redraw differs");
    }
}

/// Formulas with values for the still cache (a tunnel, rings, the plasma
/// sketch), and every golden formula that has any: the same bytes with the
/// cache as without, kept and drawn from, at both lane widths.
#[test]
fn the_still_cache_draws_the_same_bytes() {
    const STILL: &[&str] = &[
        "loop 4.8\nd = len(cx, cy*2)\ndepth = 1/d\nu = frac(angle*2 + t/4.8 + depth*a/32)\nv = frac(depth/4 + t/4.8)\ncell = (floor(u*8) + floor(v*4)) % 2\nramp = clamp((d - 0.25)*2.4, 0, 1)\nlayer { pal(chrono, frac(mix(0.28, 0.78, cell) + t/4.8)) }\nlayer multiply { gray(floor(ramp*ramp*(0.8 + 0.2*cos(angle + t/4.8))*8)/8) }",
        "loop 4\npal(rainbow, frac(r*1.5 - t/4))",
        "q = sqrt(px*py) + noise(x*8, y*4, 0)\nhsv(q/64 + t/8, 1, step(0.3, frac(q)))",
        "k = pick(px/32, len(x, y), atan2(y, x), 1/(x + 1), hash(px, py))\ngray(frac(k*3 + t))",
        "loop 2\nd = 1/(len(px - 63.5, (py - 15.5)*2) + 1)\nlayer { gray(frac(d*8 + t/2)) }\nlayer screen { rgb(ur, ug*d, 0, 0.5) }",
        sketches::PLASMA,
        sketches::RING_CLOCK,
    ];
    let mut under = vec![0u8; W * H * 3];
    picture(&mut under);
    let mut cases: Vec<(String, bool)> = STILL.iter().map(|s| (s.to_string(), true)).collect();
    for &(_, src, _, _) in GOLDEN {
        cases.push((src.to_string(), false));
    }
    for &(_, src, _, _, _) in OVER.iter().chain(DRAWN) {
        cases.push((src.to_string(), false));
    }
    let mut cached = 0;
    for (src, must) in &cases {
        let p = compile(src).unwrap_or_else(|e| panic!("{}", e.render(src)));
        if p.still_lanes() == 0 {
            assert!(
                !must,
                "{src:?}: expected values for the still cache\n{}",
                p.listing()
            );
            continue;
        }
        cached += 1;
        assert!(p.esp32_us_still() < p.esp32_us(), "{src:?}");
        let u = p.over().then_some(&under[..]);
        still_same::<128>(src, &p, &[0, 1234, 2500, 777], u);
        still_same::<64>(src, &p, &[0, 1234], u);
    }
    assert!(
        cached >= STILL.len(),
        "{cached} formulas with a still cache"
    );
}

/// Layouts the review found broken (a value before the still marker read a
/// cached one, so it read slot 0, `t`): the same frames as the plain layout,
/// without a cache and with one kept and drawn from.
#[test]
fn a_value_before_the_marker_never_reads_the_run() {
    for src in [
        "loop 4\ncx = px - 63.5\ncy = (15.5 - py)*2\nang = atan2(cy, cx)\nab = abs(ang)\nzz = sqrt(ab*2)\ngray(frac(ang + t/4) * frac(ab + t/4) * frac(zz + t/4))",
        "loop 4\ncx = px - 63.5\ncy = (15.5 - py)*2\nang = atan2(cy, cx)\nab = abs(ang) + 0.5\nzz = len(ab*2, cx)\ngray(frac(ang + t/4) * frac(ab + t/4) * frac(zz + t/4))",
    ] {
        let p = compile(src).unwrap();
        let plain = opendmd_psi::compile_plain(src).unwrap();
        for ms in [0, 700, 2100] {
            assert!(frame_ink::<128>(&p, ms, None) == frame_ink::<128>(&plain, ms, None), "{src:?} at {ms}");
        }
        if p.still_lanes() > 0 {
            still_same::<128>(src, &p, &[0, 700, 2100], None);
        }
    }
}

/// A program changed in place under a cache that kept another's values is
/// drawn as it is, never from the other's values.
#[test]
fn the_still_cache_never_draws_another_programs_values() {
    let a = compile("loop 2\ngray(frac(1/len(cx, cy*2)/4 + t/2))").unwrap();
    let b = compile("loop 2\ngray(frac(1/len(cx, cy*3)/4 + t/2))").unwrap();
    assert_eq!(a.still_lanes(), 1);
    assert_eq!(a.listing().lines().count(), b.listing().lines().count());
    let mut s = Scratch::<128>::new();
    let mut ink = Ink::boxed();
    let mut st = Still::new();
    assert!(st.take(&a));
    let mut out = vec![0u8; W * H * 3];
    render_rows_still::<128>(
        &a,
        &mut s,
        Some(&mut ink),
        Some(&mut st),
        &inputs(500),
        &OWN,
        None,
        u32::MAX,
        &mut out,
    );
    assert!(st.holds(&a) && !st.holds(&b));
    render_rows_still::<128>(
        &b,
        &mut s,
        Some(&mut ink),
        Some(&mut st),
        &inputs(500),
        &OWN,
        None,
        u32::MAX,
        &mut out,
    );
    assert_eq!(out, frame_ink::<128>(&b, 500, None));
    // Only a table's entries changed: not the kept values.
    let ta = compile("loop 4\ntable T = 0.1, 0.5, 0.9, 0.3\nk = T[floor(len(px - 63.5, (15.5 - py)*2)/8)]\ngray(frac(k + t/4))").unwrap();
    let tb = compile("loop 4\ntable T = 0.9, 0.2, 0.1, 0.6\nk = T[floor(len(px - 63.5, (15.5 - py)*2)/8)]\ngray(frac(k + t/4))").unwrap();
    assert!(ta.still_lanes() > 0);
    let mut st2 = Still::new();
    assert!(st2.take(&ta));
    render_rows_still::<128>(
        &ta,
        &mut s,
        Some(&mut ink),
        Some(&mut st2),
        &inputs(500),
        &OWN,
        None,
        u32::MAX,
        &mut out,
    );
    assert!(st2.holds(&ta) && !st2.holds(&tb));
    render_rows_still::<128>(
        &tb,
        &mut s,
        Some(&mut ink),
        Some(&mut st2),
        &inputs(500),
        &OWN,
        None,
        u32::MAX,
        &mut out,
    );
    assert_eq!(out, frame_ink::<128>(&tb, 500, None));
    // A partial redraw of a program the cache doesn't hold runs every step.
    assert!(st.holds(&b));
    render_rows_still::<128>(
        &a,
        &mut s,
        Some(&mut ink),
        Some(&mut st),
        &inputs(500),
        &OWN,
        None,
        0xff,
        &mut out,
    );
    assert_eq!(
        &out[..W * 8 * 3],
        &frame_ink::<128>(&a, 500, None)[..W * 8 * 3]
    );
    // Let go: drawn as without.
    st.release();
    assert!(!st.is_held() && st.held_bytes() == 0);
    render_rows_still::<128>(
        &a,
        &mut s,
        Some(&mut ink),
        Some(&mut st),
        &inputs(9),
        &OWN,
        None,
        u32::MAX,
        &mut out,
    );
    assert_eq!(out, frame_ink::<128>(&a, 9, None));
}

/// Formulas that draw: the same hashes at both lane widths, here and in
/// wasm (and on the ESP32, through `psi-probe`).
#[test]
fn golden_frames_drawn() {
    let mut under = vec![0u8; W * H * 3];
    picture(&mut under);
    let mut bad = Vec::new();
    for &(name, src, ms, over, want) in DRAWN {
        let p = compile(src).unwrap_or_else(|e| panic!("{name}: {}", e.render(src)));
        assert!(p.draws(), "{name} draws nothing");
        let u = over.then_some(under.as_slice());
        let a = frame_ink::<128>(&p, ms, u);
        assert!(
            a == frame_ink::<64>(&p, ms, u),
            "{name}: lane widths differ"
        );
        let got = fnv(&a);
        if got != want {
            bad.push(format!("    (\"{name}\", …, {ms}, {over}, {got:#x}),"));
        }
    }
    assert!(
        bad.is_empty(),
        "drawn golden frames differ (got):\n{}",
        bad.join("\n")
    );
}

/// Layers, and formulas over a picture: the same hashes at both lane
/// widths, here and in wasm (and on the ESP32, through `psi-probe`).
#[test]
fn golden_frames_over() {
    let mut under = vec![0u8; W * H * 3];
    picture(&mut under);
    let mut bad = Vec::new();
    for &(name, src, ms, over, want) in OVER {
        let p = compile(src).unwrap_or_else(|e| panic!("{name}: {}", e.render(src)));
        let u = over.then_some(under.as_slice());
        let a = frame_over::<128>(&p, ms, u);
        assert!(
            a == frame_over::<64>(&p, ms, u),
            "{name}: lane widths differ"
        );
        let got = fnv(&a);
        if got != want {
            bad.push(format!("    (\"{name}\", …, {ms}, {over}, {got:#x}),"));
        }
    }
    assert!(
        bad.is_empty(),
        "golden frames over differ (got):\n{}",
        bad.join("\n")
    );
}

/// Which pixels of `src`'s frame are lit (`gray` of 0 or 1: 255 or 0),
/// `[row from the top][column]`.
fn lit(src: &str) -> Vec<[bool; W]> {
    let p = compile(src).unwrap_or_else(|e| panic!("{}", e.render(src)));
    let f = frame::<128>(&p, 0);
    assert!(f == frame::<64>(&p, 0), "{src}: lane widths differ");
    (0..H)
        .map(|r| {
            core::array::from_fn(|c| {
                let v = f[(r * W + c) * 3];
                assert!(v == 0 || v == 255, "{src}: pixel {c}, {r} is {v}");
                v == 255
            })
        })
        .collect()
}

/// The font's pixel, as `opendmd_core::text` draws it: rows of bytes, the
/// most significant bit leftmost; blank outside the cell or the font.
fn font_pixel(rows: &[u8], w: i32, h: i32, glyph: i32, x: i32, y: i32) -> bool {
    (0..95).contains(&glyph)
        && (0..w).contains(&x)
        && (0..h).contains(&y)
        && rows[(glyph * h + y) as usize] & (0x80 >> x) != 0
}

/// `digit` and `glyph`, in both fonts: every pixel of every character, and
/// around each cell (a column and a row before and after it, which are
/// blank), against core's font tables, per pixel; out-of-range and
/// fractional characters (floored; blank).
#[test]
fn digits_and_glyphs_are_the_fonts() {
    for (font, rows, w, h) in [
        ("", FONT_5X7.as_flattened(), 5, 7),
        (", small", FONT_4X6.as_flattened(), 4, 6),
    ] {
        let (cw, ch) = (w + 2, h + 2); // the cell and a pixel either side
        let per = W as i32 / cw; // cells across
        for first in [0, 1, 2, 3, 4].map(|k| 32 - 2 + k * per * (H as i32 / ch)) {
            // Characters from `first` (some below 32 and above 126), a
            // quarter added (floored away).
            let src = format!(
                "k = {first} + floor(px/{cw}) + floor((31 - py)/{ch})*{per}\n\
                 gray(glyph(k + 0.25, px % {cw} - 1, (31 - py) % {ch} - 1{font}))"
            );
            let got = lit(&src);
            for (r, row) in got.iter().enumerate() {
                for (c, &l) in row.iter().enumerate() {
                    let (c, r) = (c as i32, r as i32);
                    let k = first + c / cw + r / ch * per;
                    let want = font_pixel(rows, w, h, k - 32, c % cw - 1, r % ch - 1);
                    assert_eq!(l, want, "glyph {k} ({src}) at {c}, {r}");
                }
            }
        }
        // Digits: -1 to 10 (blank but 0..9), 0.75 less (floored).
        let src = format!(
            "n = floor(px/{cw}) % 12 - 1.75 + 1\n\
             gray(digit(n, px % {cw} - 1, (31 - py) % {ch} - 1{font}))"
        );
        for (r, row) in lit(&src).iter().enumerate() {
            for (c, &l) in row.iter().enumerate() {
                let (c, r) = (c as i32, r as i32);
                let n = c / cw % 12 - 1;
                let want =
                    (0..10).contains(&n) && font_pixel(rows, w, h, 16 + n, c % cw - 1, r % ch - 1);
                assert_eq!(l, want, "digit {n} ({src}) at {c}, {r}");
            }
        }
        // Once a row (the character a constant, the row from py) and once a
        // frame (all constants: run, not folded).
        for n in [-1, 0, 7, 9, 10] {
            for x in -1..=w {
                let src = format!("gray(digit({n}, {x}, 31 - py{font}))");
                let src = src
                    .replace("digit(-1", "digit(0 - 1")
                    .replace(", -1,", ", 0 - 1,");
                for (r, row) in lit(&src).iter().enumerate() {
                    let want = (0..10).contains(&n) && font_pixel(rows, w, h, 16 + n, x, r as i32);
                    assert!(row.iter().all(|&l| l == want), "{src}: row {r}");
                }
            }
        }
        for (k, x, y) in [(65, 0, 2), (65, 2, 0), (58, 1, 1), (126, 1, 0), (127, 0, 0)] {
            let src = format!("gray(glyph({k}, {x}, {y}{font}))");
            let want = font_pixel(rows, w, h, k - 32, x, y);
            assert!(lit(&src).iter().flatten().all(|&l| l == want), "{src}");
        }
    }
    // The bold digits (digits only, a 4 × 6 cell, the table's 5 rows):
    // every pixel and around each cell, per pixel and once a row.
    let bold = FONT_BOLD.as_flattened();
    let src = "n = floor(px/6) % 12 - 1.75 + 1\n\
               gray(digit(n, px % 6 - 1, (31 - py) % 8 - 1, bold))";
    for (r, row) in lit(src).iter().enumerate() {
        for (c, &l) in row.iter().enumerate() {
            let (c, r) = (c as i32, r as i32);
            let n = c / 6 % 12 - 1;
            let (x, y) = (c % 6 - 1, r % 8 - 1);
            let want = (0..10).contains(&n) && x < 4 && font_pixel(bold, 4, 5, n, x, y);
            assert_eq!(l, want, "bold digit {n} at {c}, {r}");
        }
    }
    for n in [-1, 0, 4, 9, 10] {
        for x in -1..=4 {
            let src = format!("gray(digit({n}, {x}, 31 - py, bold))")
                .replace("digit(-1", "digit(0 - 1")
                .replace(", -1,", ", 0 - 1,");
            for (r, row) in lit(&src).iter().enumerate() {
                let want = (0..10).contains(&n) && font_pixel(bold, 4, 5, n, x, r as i32);
                assert!(row.iter().all(|&l| l == want), "{src}: row {r}");
            }
        }
    }
    // A character in quotes is its code.
    assert!(
        lit("gray('A' == 65 and ' ' == 32 and '~' == 126 and ''' == 39 and '-' - 1 == 44)")
            .iter()
            .flatten()
            .all(|&l| l)
    );
}

/// `bit(n, i)`: bit i of floor(n), two's complement; i floored, below 0
/// nothing; per pixel against the reference, and once (folded) at the edges.
#[test]
fn bits_are_twos_complement() {
    let reference = |n: f64, i: f64| -> bool {
        let (n, i) = (n.floor() as i64, i.floor() as i64);
        i >= 0 && (n >> i.min(40)) & 1 == 1
    };
    let got = lit("gray(bit(px*64.25 - 4096, py*0.75 - 4))");
    for (r, row) in got.iter().enumerate() {
        let py = (H - 1 - r) as f64;
        for (c, &l) in row.iter().enumerate() {
            let (n, i) = (c as f64 * 64.25 - 4096.0, py * 0.75 - 4.0);
            assert_eq!(l, reference(n, i), "bit({n}, {i})");
        }
    }
    for (n, i, want) in [
        ("5", "0", true),
        ("5", "1", false),
        ("5", "2", true),
        ("5", "0.99", true),
        ("5.99", "1", false),
        ("5", "0 - 1", false),
        ("5", "0 - 0.5", false),
        ("0.5", "0", false),
        ("32767", "14", true),
        ("32767", "15", false),
        ("16384", "14", true),
        ("0 - 1", "0", true),
        ("0 - 1", "15", true),
        ("0 - 1", "99", true),
        ("0 - 2", "0", false),
        ("0 - 0.25", "3", true),
        ("0", "99", false),
    ] {
        let src = format!("gray(bit({n}, {i}))");
        assert!(
            lit(&src).iter().flatten().all(|&l| l == want),
            "{src}: want {want}"
        );
    }
}

/// Any set of rows drawn alone is the same bytes as in the whole frame;
/// rows not asked for are left alone.
#[test]
fn rows_alone_are_the_same() {
    let mut under = vec![0u8; W * H * 3];
    picture(&mut under);
    for &(name, src, ms, over, _) in OVER.iter().chain(
        GOLDEN
            .iter()
            .map(|&(n, s, m, _)| (n, s, m, false, 0u64))
            .collect::<Vec<_>>()
            .iter(),
    ) {
        let p = compile(src).unwrap();
        let u = over.then_some(under.as_slice());
        let all = frame_over::<128>(&p, ms, u);
        for rows in [1u32, 0x8000_0001, 0x0f0f_0f0f, 0x1234_5678] {
            let mut s = Scratch::<128>::new();
            let mut out = vec![0x5a; W * H * 3];
            render_rows::<128>(&p, &mut s, &inputs(ms), &OWN, u, rows, &mut out);
            for r in 0..H {
                let row = r * W * 3..(r + 1) * W * 3;
                if rows & (1 << r) != 0 {
                    assert!(out[row.clone()] == all[row], "{name}: row {r}");
                } else {
                    assert!(
                        out[row].iter().all(|&b| b == 0x5a),
                        "{name}: row {r} touched"
                    );
                }
            }
        }
    }
}

#[test]
fn sketches_compile_small() {
    for (name, src) in sketches::ALL {
        let p = compile(src).unwrap();
        assert!(p.len() <= 40, "{name}: {} steps", p.len());
    }
}

/// xorshift64*: a reproducible stream for the fuzz.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

const PIECES: &[&str] = &[
    "x",
    "y",
    "px",
    "py",
    "i",
    "t",
    "w",
    "h",
    "hour",
    "minute",
    "second",
    "a",
    "b",
    "c",
    "0",
    "1",
    "0.5",
    "32767",
    "-",
    "+",
    "*",
    "/",
    "%",
    "<",
    "<=",
    ">",
    ">=",
    "==",
    "!=",
    "and",
    "or",
    "not",
    "?",
    ":",
    "(",
    ")",
    ",",
    "=",
    "\n",
    "{",
    "}",
    "frame",
    "loop",
    "4",
    "sin(",
    "cos(",
    "tri(",
    "saw(",
    "sqr(",
    "abs(",
    "min(",
    "max(",
    "clamp(",
    "floor(",
    "frac(",
    "mix(",
    "step(",
    "smoothstep(",
    "hash(",
    "noise(",
    "len(",
    "atan2(",
    "sqrt(",
    "rgb(",
    "hsv(",
    "pal(",
    "fire",
    "gray(",
    "v",
    "v =",
    "q =",
    "--",
    "//",
    ";",
    " ",
    "ψ",
    "é",
    "@",
    "\"",
    "99999",
    ".",
    "1e3",
    "_",
    "layer",
    "layer {",
    "layer add {",
    "multiply",
    "screen",
    "max",
    "ur",
    "ug",
    "ub",
    "ul",
    "digit(",
    "glyph(",
    "bit(",
    "'A'",
    "'",
    "small",
    "normal",
    "#f80",
    "#ff8800",
    "#",
    "#12",
    "pick(",
    "cx",
    "cy",
    "r",
    "angle",
    "ty",
    "r =",
    "draw {",
    "draw { for",
    "draw { for k <",
    "draw { for k < 3 {",
    "draw {\nfor k",
    "draw",
    "for k < 12 {",
    "for j < 3 {",
    "for",
    "k",
    "<",
    "line(",
    "dot(",
    "ink",
    "table T = 1, 2, -3",
    "table",
    "T[",
    "T",
    "[",
    "]",
    "1024",
];

fn check(src: &str) {
    match compile(src) {
        Ok(p) => {
            assert!(p.len() <= MAX_OPS, "{src:?}: {} steps", p.len());
            let _ = frame::<128>(&p, 1234);
            let mut under = vec![0u8; W * H * 3];
            picture(&mut under);
            let _ = frame_over::<128>(&p, 1234, Some(&under));
            let with = frame_ink::<64>(&p, 1234, Some(&under));
            // The column and still runs change no byte: against the formula
            // laid out without them, at the other lane width (where the
            // column run is worked out on every lane).
            let plain = opendmd_psi::compile_plain(src).expect("compiles plain too");
            assert!(
                frame_ink::<128>(&plain, 1234, Some(&under)) == with,
                "{src:?}: the runs changed the frame"
            );
            let mut rows = vec![0u8; W * H * 3];
            render_rows::<128>(
                &p,
                &mut Scratch::new(),
                &inputs(1234),
                &OWN,
                Some(&under),
                0x0f0f_0f0f,
                &mut rows,
            );
            if !p.draws() {
                // Some rows alone (the column run on the first drawn one).
                for r in (0..H).filter(|r| 0x0f0f_0f0fu32 & (1 << r) != 0) {
                    let span = r * W * 3..(r + 1) * W * 3;
                    assert!(
                        rows[span.clone()] == with[span],
                        "{src:?}: row {r} alone differs"
                    );
                }
            }
            if p.still_lanes() > 0 {
                still_same::<128>(src, &p, &[1234, 99], Some(&under));
            }
        }
        Err(e) => {
            assert!(
                e.start <= e.end && e.end <= src.len().max(e.start),
                "{src:?}: span {e:?}"
            );
            assert!(
                src.is_char_boundary(e.start.min(src.len())),
                "{src:?}: span {e:?}"
            );
            let _ = e.render(src);
        }
    }
}

/// Random formulas built from named values the cache would keep (`len`,
/// `atan2`, `sqrt`, `1/…` of the position) shared with moving ones and
/// with each other: every layout draws the plain layout's frames, without a
/// cache and with one.
#[test]
fn random_still_layouts_are_the_plain_frames() {
    let mut r = Rng(0x57111_c4c4e);
    let still = [
        "len(px - 63.5, (15.5 - py)*2)",
        "atan2(15.5 - py, px - 63.5)",
        "sqrt(px*py + 1)",
        "1/(len(cx, cy) + 0.1)",
        "abs(cx*cy)",
        "x*y",
    ];
    let moving = ["t/4", "a", "sin(t)", "x + t"];
    let ops = ["+", "-", "*"];
    let funs = ["abs", "sqrt", "frac", "sin", "floor"];
    for case in 0..400 {
        let mut src = String::from("loop 4\n");
        let n = 3 + r.below(5);
        for k in 0..n {
            let pick = |r: &mut Rng, k: usize| -> String {
                match r.below(4) {
                    0 if k > 0 => format!("v{}", r.below(k)),
                    1 => moving[r.below(moving.len())].to_string(),
                    2 => format!("{}.{}", r.below(4), r.below(10)),
                    _ => still[r.below(still.len())].to_string(),
                }
            };
            let (x, y) = (pick(&mut r, k), pick(&mut r, k));
            let e = if r.below(2) == 0 {
                format!(
                    "{}({} {} {})",
                    funs[r.below(funs.len())],
                    x,
                    ops[r.below(3)],
                    y
                )
            } else {
                format!("({}) {} ({})", x, ops[r.below(3)], y)
            };
            src += &format!("v{k} = {e}\n");
        }
        let used: Vec<String> = (0..n)
            .filter(|_| r.below(3) > 0)
            .map(|k| format!("frac(v{k} + t/4)"))
            .collect();
        src += &format!(
            "gray({} * 0.5)",
            if used.is_empty() {
                "v0".to_string()
            } else {
                used.join(" + ")
            }
        );
        let Ok(p) = compile(&src) else { continue };
        let plain = opendmd_psi::compile_plain(&src).expect("plain compiles");
        for ms in [0, 1300] {
            assert!(
                frame_ink::<128>(&p, ms, None) == frame_ink::<128>(&plain, ms, None),
                "case {case}: {src:?} at {ms} ms\n{}",
                p.listing()
            );
        }
        if p.still_lanes() > 0 {
            still_same::<128>(&src, &p, &[0, 1300], None);
        }
    }
}

#[test]
fn garbage_never_panics() {
    let mut r = Rng(0x5eed_1234_abcd_ef01);
    for _ in 0..3000 {
        // Token soup.
        let n = 1 + r.below(40);
        let pieces: Vec<&str> = (0..n).map(|_| PIECES[r.below(PIECES.len())]).collect();
        check(&pieces.join(" "));
        // And cut off after every piece: a look ahead past the end of the
        // input (`draw { for` at the end) must be an error, never a panic.
        for k in 1..pieces.len() {
            check(&pieces[..k].join(" "));
        }
        // Raw bytes (as UTF-8, lossily).
        let raw: Vec<u8> = (0..r.below(64)).map(|_| r.next() as u8).collect();
        check(&String::from_utf8_lossy(&raw));
    }
}

/// A random well-formed expression of depth ≤ `d`.
fn expr(r: &mut Rng, d: u32) -> String {
    const LEAVES: &[&str] = &[
        "x", "y", "px", "py", "i", "t", "hour", "a", "0.5", "3", "-2.25", "100", "w", "ur", "ul",
    ];
    if d == 0 || r.below(4) == 0 {
        return LEAVES[r.below(LEAVES.len())].to_string();
    }
    let e = |r: &mut Rng| expr(r, d - 1);
    match r.below(10) {
        0..=3 => {
            let op = ["+", "-", "*", "/", "%", "<", ">=", "==", "and", "or"][r.below(10)];
            format!("({} {op} {})", e(r), e(r))
        }
        4 => format!("-({})", e(r)),
        5 => format!("({} ? {} : {})", e(r), e(r), e(r)),
        6 | 7 => {
            let f = [
                "sin", "cos", "tri", "saw", "sqr", "abs", "floor", "frac", "sqrt",
            ][r.below(9)];
            format!("{f}({})", e(r))
        }
        8 => {
            let f = ["min", "max", "step", "hash", "len", "atan2"][r.below(6)];
            format!("{f}({}, {})", e(r), e(r))
        }
        _ => {
            let f = ["clamp", "mix", "smoothstep", "noise"][r.below(4)];
            format!("{f}({}, {}, {})", e(r), e(r), e(r))
        }
    }
}

#[test]
fn random_formulas_render_or_refuse() {
    let mut r = Rng(0xfeed_beef_0bad_cafe);
    let mut ok = 0;
    let mut under = vec![0u8; W * H * 3];
    picture(&mut under);
    for k in 0..600 {
        let src = if k % 2 == 0 {
            format!(
                "loop {}\nv = {}\nu = {}\nhsv({}, 1, {})",
                1 + k % 9,
                expr(&mut r, 4),
                expr(&mut r, 4),
                expr(&mut r, 3),
                expr(&mut r, 3)
            )
        } else {
            // Layers: a shared value, two or three layers of their own.
            let blends = ["", "add", "multiply", "screen", "max", "min", "over"];
            let mut s = format!("loop {}\nv = {}\n", 1 + k % 9, expr(&mut r, 3));
            for _ in 0..2 + r.below(2) {
                let cover = if r.below(2) == 0 {
                    format!(", {}", expr(&mut r, 2))
                } else {
                    String::new()
                };
                s += &format!(
                    "layer {} {{\nu = {}\nrgb(u, v, {}{cover})\n}}\n",
                    blends[r.below(blends.len())],
                    expr(&mut r, 3),
                    expr(&mut r, 2)
                );
            }
            s
        };
        match compile(&src) {
            Ok(p) => {
                ok += 1;
                assert!(p.len() <= MAX_OPS);
                // Both lane widths, bit for bit, alone and over a picture.
                assert!(frame::<128>(&p, 777) == frame::<64>(&p, 777), "{src}");
                assert!(
                    frame_over::<128>(&p, 777, Some(&under))
                        == frame_over::<64>(&p, 777, Some(&under)),
                    "{src}"
                );
            }
            Err(e) => {
                // Only the size limits may refuse a well-formed formula.
                assert!(
                    e.msg.contains("too long")
                        || e.msg.contains("too many")
                        || e.msg.contains("nested"),
                    "{src}\n{}",
                    e.render(&src)
                );
            }
        }
    }
    assert!(ok > 500, "only {ok} of 600 compiled");
}

/// The same for pick, the values about the middle and colours: random
/// well-formed uses render the same at both lane widths, alone and over
/// a picture, or are refused only for their size.
#[test]
fn random_picks_and_colours_render_or_refuse() {
    let mut r = Rng(0x0dd_c010_75ee_d5ed);
    let mut under = vec![0u8; W * H * 3];
    picture(&mut under);
    let leaf = |r: &mut Rng| {
        [
            "cx",
            "cy",
            "r",
            "angle",
            "ty",
            "px/16",
            "y*3",
            "second % 5",
            "v",
        ][r.below(9)]
        .to_string()
    };
    let hex = |r: &mut Rng| format!("#{:06x}", r.next() & 0xff_ffff);
    let mut ok = 0;
    for k in 0..400 {
        let n = 1 + r.below(5);
        let opts: Vec<String> = (0..n)
            .map(|_| {
                if r.below(3) == 0 {
                    leaf(&mut r)
                } else {
                    expr(&mut r, 2)
                }
            })
            .collect();
        let pick = format!("pick({}, {})", leaf(&mut r), opts.join(", "));
        let colour = match k % 4 {
            0 => hex(&mut r),
            1 => format!("mix({}, {}, {pick})", hex(&mut r), hex(&mut r)),
            2 => format!(
                "rgb(mix(rgb({pick}, {}, ur), {}, {}), {})",
                leaf(&mut r),
                hex(&mut r),
                leaf(&mut r),
                expr(&mut r, 2)
            ),
            _ => format!(
                "layer multiply {{ mix(#fff, mix({}, rgb(v, {pick}, 0.5), cy*2 + 1), r) }}",
                hex(&mut r)
            ),
        };
        let src = format!("loop 4\nv = {}\nw2 = {pick}\n{colour}", expr(&mut r, 3));
        match compile(&src) {
            Ok(p) => {
                ok += 1;
                assert!(frame::<128>(&p, 777) == frame::<64>(&p, 777), "{src}");
                assert!(
                    frame_over::<128>(&p, 777, Some(&under))
                        == frame_over::<64>(&p, 777, Some(&under)),
                    "{src}"
                );
            }
            Err(e) => assert!(
                e.msg.contains("too long")
                    || e.msg.contains("too many")
                    || e.msg.contains("nested"),
                "{src}\n{}",
                e.render(&src)
            ),
        }
    }
    assert!(ok > 350, "only {ok} of 400 compiled");
}

/// Random draw blocks: loops (one or two deep), tables, values from the
/// loop counters and the frame, lines and dots anywhere (far off the panel
/// too): they render the same at both lane widths, alone and over a
/// picture, within the caps, or are refused only for their size.
#[test]
fn random_draw_blocks_render_or_refuse() {
    let mut r = Rng(0x0d2a_35ee_d0f1_a7e5);
    let mut under = vec![0u8; W * H * 3];
    picture(&mut under);
    // A value inside a loop: the counters, the tables, the time.
    fn val(r: &mut Rng, ks: &[&str], d: u32) -> String {
        let leaf = |r: &mut Rng| -> String {
            match r.below(6) {
                0 | 1 if !ks.is_empty() => ks[r.below(ks.len())].to_string(),
                2 => format!("T[{}]", ks.first().copied().unwrap_or("t")),
                3 => ["t", "a", "second", "f"][r.below(4)].to_string(),
                _ => format!("{}", r.below(2000) as i32 - 900),
            }
        };
        if d == 0 || r.below(3) == 0 {
            return leaf(r);
        }
        match r.below(6) {
            0 => format!("({} + {})", val(r, ks, d - 1), val(r, ks, d - 1)),
            1 => format!("({} * {})", val(r, ks, d - 1), val(r, ks, d - 1)),
            2 => format!("({} / {})", val(r, ks, d - 1), val(r, ks, d - 1)),
            3 => format!("sin({})", val(r, ks, d - 1)),
            4 => format!("hash({}, 3)*{}", val(r, ks, d - 1), r.below(200)),
            _ => format!("({} % 37)", val(r, ks, d - 1)),
        }
    }
    let prim = |r: &mut Rng, ks: &[&str]| -> String {
        if r.below(3) == 0 {
            format!(
                "dot({}, {}, {})",
                val(r, ks, 2),
                val(r, ks, 2),
                val(r, ks, 1)
            )
        } else {
            format!(
                "line({}, {}, {}, {}, {}, {})",
                val(r, ks, 2),
                val(r, ks, 2),
                val(r, ks, 2),
                val(r, ks, 2),
                val(r, ks, 1),
                val(r, ks, 1)
            )
        }
    };
    let mut ok = 0;
    for k in 0..300 {
        let n1 = [1, 3, 12, 100, 1024][r.below(5)];
        let mut body = format!(
            "v = {}\n{}\n",
            val(&mut r, &["k"], 3),
            prim(&mut r, &["k", "v"])
        );
        if r.below(2) == 0 {
            let n2 = 1 + r.below(8);
            body += &format!("for j < {n2} {{\n{}\n}}\n", prim(&mut r, &["k", "j", "v"]));
        }
        let colour = [
            "gray(ink)",
            "pal(fire, ink)",
            "#40c0ff * ink",
            "layer screen { rgb(ink, ur, 0.5) }",
            "rgb(ink, T[px % 4], x)",
        ][k % 5];
        let src = format!(
            "loop {}\ntable T = 0.5, -2, 30, 100\nframe {{ f = t*{} }}\ndraw {{\n{}\nfor k < {n1} {{\n{body}}}\n}}\n{colour}",
            1 + k % 7,
            r.below(50),
            prim(&mut r, &[]),
        );
        match compile(&src) {
            Ok(p) => {
                ok += 1;
                assert!(p.draws(), "{src}");
                assert!(p.len() <= MAX_OPS);
                for ms in [0, 777] {
                    assert!(
                        frame_ink::<128>(&p, ms, None) == frame_ink::<64>(&p, ms, None),
                        "{src}"
                    );
                    assert!(
                        frame_ink::<128>(&p, ms, Some(&under))
                            == frame_ink::<64>(&p, ms, Some(&under)),
                        "{src}"
                    );
                }
            }
            Err(e) => assert!(
                e.msg.contains("too long")
                    || e.msg.contains("too many")
                    || e.msg.contains("too much"),
                "{src}\n{}",
                e.render(&src)
            ),
        }
    }
    assert!(ok > 250, "only {ok} of 300 compiled");
}

#[test]
fn scalar_cap_is_a_friendly_error() {
    let body: Vec<String> = (1..=120).map(|k| format!("x*{k}.{k}")).collect();
    let src = format!("gray(frac({}))", body.join(" + "));
    let e = compile(&src).unwrap_err();
    assert!(e.msg.contains("too many numbers"), "{}", e.msg);
}

#[test]
fn op_cap_is_a_friendly_error() {
    // 2 steps a line, 150 lines.
    let mut src = String::from("v0 = x\n");
    for k in 1..=150 {
        src += &format!("v{k} = sin(v{}) + x\n", k - 1);
    }
    src += "gray(v150)";
    let e = compile(&src).unwrap_err();
    assert!(e.msg.contains("too long for the panel"), "{}", e.msg);
}

/// Division by a constant (2026-10-26): once a frame or a row it is a true
/// division, so `floor(minute/10)` is 5 at 50..59 and `floor(hour/10)` 2 at
/// 20..23 (it used to be a multiply by 1/10 rounded down: 20/10 came out
/// 1.99998); per pixel it is a multiply by 1/k rounded up, exact under
/// `floor` for whole numbers n with n·k < 65,536 (px, py, and i for k ≤ 16).
#[test]
fn division_by_a_constant_floors_whole_numbers_right() {
    use opendmd_psi::Inputs;
    let decode = |f: &[u8], at: usize| f[at * 3] as u32 + 256 * f[at * 3 + 1] as u32;
    for k in [10u32, 60, 3, 7, 12] {
        // Once a frame: n = hour·3600 + minute·60 + second, 0..=3600.
        let src = format!(
            "frame {{ n = hour*3600 + minute*60 + second; q = floor(n/{k}) }}\nrgb((q % 256)/255, floor(q/256)/255, n/{k} == q)"
        );
        let p = compile(&src).unwrap_or_else(|e| panic!("{}", e.render(&src)));
        let mut s = Scratch::<128>::new();
        let mut out = vec![0u8; W * H * 3];
        for n in 0..=3600u32 {
            let inp = Inputs {
                t: 0,
                hour: (n / 3600) as u8,
                minute: (n % 3600 / 60) as u8,
                second: (n % 60) as u8,
                a: 0,
                b: 0,
                c: 0,
            };
            render_with::<128>(&p, &mut s, &inp, &OWN, &mut out);
            assert_eq!(decode(&out, 0), n / k, "floor({n}/{k}) once a frame");
            assert_eq!(out[2] == 255, n % k == 0, "{n}/{k} exact once a frame");
        }
        // Per pixel: n = i, 0..4095 (k = 60: px, 0..127).
        let (n_src, top) = if k == 60 { ("px", 127) } else { ("i", 4095) };
        let src = format!("q = floor({n_src}/{k})\nrgb((q % 256)/255, floor(q/256)/255, 0)");
        let p = compile(&src).unwrap_or_else(|e| panic!("{}", e.render(&src)));
        render_with::<128>(&p, &mut s, &inputs(0), &OWN, &mut out);
        for at in 0..W * H {
            let n = if k == 60 { (at % W) as u32 } else { at as u32 };
            assert!(n <= top);
            assert_eq!(
                decode(&out, at),
                n / k,
                "floor({n}/{k}) per pixel (rgb {:?})",
                &out[at * 3..at * 3 + 3]
            );
        }
    }
}

/// A frame of `p` at `inputs` with the draw stage's canvas and a picture
/// under it.
fn frame_at(p: &Program, inputs: &opendmd_psi::Inputs, under: &[u8]) -> Vec<u8> {
    let mut s = Scratch::<128>::new();
    let mut ink = Ink::boxed();
    let mut out = vec![0x5a; W * H * 3];
    render_rows_ink::<128>(
        p,
        &mut s,
        Some(&mut ink),
        inputs,
        &OWN,
        Some(under),
        u32::MAX,
        &mut out,
    );
    out
}

/// The narrowed steps (`add_w`, `mul_n`, `shr`, …: the ranges say they
/// never saturate) draw what the checked ones draw: random formulas with
/// values near the ends of the range, divisions by values whose range
/// holds 0, values that are constant only for inputs in range, at times
/// across the loop and past it, with the sliders and the clock in range
/// and out (where the VM runs the checked steps). `PSI_FUZZ_N` sets the
/// number of formulas (1,500 by default).
#[test]
fn narrowed_steps_are_the_checked_frames() {
    let mut r = Rng(0x0a77_0e5d_57e9_5000);
    let mut under = vec![0u8; W * H * 3];
    picture(&mut under);
    const BIG: &[&str] = &[
        "181",
        "30000",
        "t*100",
        "px*py*8",
        "t*t",
        "a*3000",
        "1/8",
        "0.0625",
        "px*256",
        "-px*256",
        "4",
        "x*x",
        "len(px, py)",
        "atan2(py, px)",
        "floor(t*7)",
        "frac(t*3)",
        // Divisions by a value whose range holds 0; values constant only
        // while the inputs are in range (a power of two then: a shift).
        "1/(floor(a) - 1)",
        "1/(a - 0.5)",
        "1/(t - 2)",
        "1/(floor(b) - 1)",
        "min(a + 1, 0.5)",
        "max(0.25, a - 3)",
        "floor(b) - 1",
        "min(5 - t, 0.5)",
        "min(c + 1, 0.25)",
        // Small products (one 32-bit multiply).
        "x * 0.3",
        "y * 0.45",
        "sin(x) * 0.6",
    ];
    // Slider values: in range (0, 1, ½, ⅓) and out (-1, 3).
    const SLIDE: [i32; 6] = [0, 1 << 16, 1 << 15, 21845, -(1 << 16), 3 << 16];
    let n: usize = std::env::var("PSI_FUZZ_N")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1500);
    let (mut narrowed, mut shr, mut mul_n, mut frames) = (0, 0, 0, 0);
    for k in 0..n {
        let leaf = |r: &mut Rng| -> String {
            if r.below(2) == 0 {
                BIG[r.below(BIG.len())].to_string()
            } else {
                expr(r, 1)
            }
        };
        let e = |r: &mut Rng| -> String {
            let (a, b, c) = (leaf(r), leaf(r), leaf(r));
            match r.below(7) {
                0 => format!("{a} * {b} + {c}"),
                1 => format!("({a} - {b}) * {c}"),
                2 => format!("{a} * {b} * {c}"),
                3 => format!("({a} + {b}) / 8 - {c}"),
                4 => format!("min({a}, 10) * min({b}, 10) * {c}"),
                5 => format!("x * {a} + {b}"),
                _ => format!("{a} + {b} - {c} * 0.25"),
            }
        };
        let lp = 1 + k % 6;
        let src = format!(
            "loop {lp}\nv = {}\nu = {}\n{}",
            e(&mut r),
            e(&mut r),
            if k % 3 == 0 {
                "pal(fire, frac(v)) * (floor(u % 9)/8)".to_string()
            } else {
                "hsv(v, 1, frac(u))".to_string()
            }
        );
        let (mut p, mut q) = (Program::default(), Program::default());
        if opendmd_psi::compile_with(&src, &mut p, true, true).is_err() {
            continue;
        }
        opendmd_psi::compile_with(&src, &mut q, true, false).expect("compiles unnarrowed too");
        if p != q {
            narrowed += 1;
        }
        let listing = p.listing();
        shr += listing.matches("shr(").count();
        mul_n += listing.matches("mul_n(").count();
        let end = lp as u32 * 1000;
        let times = [0, end / 3, end / 2, end - 1, end, end + 5000];
        for j in 0..8 {
            let ms = times[r.below(times.len())];
            let (a, b, c) = (
                SLIDE[r.below(SLIDE.len())],
                SLIDE[r.below(SLIDE.len())],
                SLIDE[r.below(SLIDE.len())],
            );
            let (hour, second) = if j % 2 == 0 { (23, 59) } else { (10, 30) };
            let inputs = opendmd_psi::Inputs {
                a,
                b,
                c,
                hour,
                second,
                ..inputs(ms)
            };
            frames += 1;
            assert!(
                frame_at(&p, &inputs, &under) == frame_at(&q, &inputs, &under),
                "case {k}: {src:?} at {ms} ms, a {a} b {b} c {c}\n{listing}"
            );
        }
    }
    // The generator must keep reaching the rewrites.
    assert!(narrowed * 3 > n * 2, "only {narrowed} of {n} narrowed");
    assert!(shr * 10 > n, "only {shr} shr in {n} formulas");
    assert!(mul_n * 10 > n, "only {mul_n} mul_n in {n} formulas");
    eprintln!(
        "{n} formulas, {narrowed} narrowed, {shr} shr, {mul_n} mul_n, {frames} frames compared"
    );
}

/// The review's two cases (psi-speed): a division whose divisor's range
/// holds 0 (`1/(floor(a) - 1)` at a = 1: a division by 0 is the
/// numerator's end, the top one here), and a shift by a value that is a
/// power of two only for inputs in range (`min(a + 1, 0.5)` at a = -1, and
/// `min(5 - t, 0.5)` past the loop).
#[test]
fn the_reviews_narrowing_cases() {
    let mut under = vec![0u8; W * H * 3];
    picture(&mut under);
    for (src, ms, a) in [
        (
            "loop 4\nk = 1/(floor(a) - 1)\nrgb(x + k + 1, 0, 0)",
            1000,
            1 << 16,
        ),
        (
            "loop 4\nk = min(a + 1, 0.5)\nrgb(x * k, 0, 0)",
            1000,
            -(1 << 16),
        ),
        (
            "loop 4\nk = min(5 - t, 0.5)\nrgb(x * k, 0, 0)",
            9000,
            1 << 15,
        ),
    ] {
        let (mut p, mut q) = (Program::default(), Program::default());
        opendmd_psi::compile_with(src, &mut p, true, true).unwrap();
        opendmd_psi::compile_with(src, &mut q, true, false).unwrap();
        let inputs = opendmd_psi::Inputs { a, ..inputs(ms) };
        assert!(
            frame_at(&p, &inputs, &under) == frame_at(&q, &inputs, &under),
            "{src:?} at {ms} ms, a {a}\n{}",
            p.listing()
        );
    }
}

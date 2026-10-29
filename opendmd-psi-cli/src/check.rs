//! `psi check`: does a candidate Formula compile — and if not, its error's
//! code, the line, the column and carets —, how heavy it is for the panel
//! (on the VM and compiled by the panel's JIT), what it takes to compile
//! on the panel, how long it is as a path, what it reads, and a PNG
//! contact sheet of its frames. The feedback loop `docs/psi-for-models.md`
//! describes (it was opendmd-psi's `psi-check` example).
//!
//!     psi check my.psi [--frames 8 --out my.png --a 0.5]
//!     make psi-check F=my.psi [ARGS="--frames 8 --out my.png --a 0.5"]
//!     echo 'gray(x)' | psi check -
//!
//! Options: `--out FILE.png` (default: the formula's name with .png, or
//! `formula.png` from stdin; `--out -` for none), `--frames N` (8, spread
//! over one loop; a still formula gets 1), `--ms 0,250,…` (exact times
//! instead), `--scale K` (4), `--a`/`--b`/`--c V` (the sliders; default a
//! `control` line's default, 0.5 for one read without), `--clock HH:MM:SS`
//! (10:09:30), `--over` (draw over the sample picture even if the formula
//! doesn't read it; it is used anyway whenever it does), `--hash` (each
//! frame's FNV-1a, as the golden tests print them), `--listing` (the
//! compiled steps), `--seed V` (a `loop forever` formula's `seed`, 0 to 1;
//! default 0). A formula that never repeats (`loop forever`) gets its
//! frames over its first minute. Exit status 1 when it doesn't compile, 2
//! for a missing file or a bad option.

use opendmd_psi::golden::fnv;
use opendmd_psi::{
    compile, q16, render_rows_ink, render_rows_still, Ink, Inputs, Scratch, Still, H, MAX_OPS,
    MAX_SCALARS, OWN, REGS, W,
};
use std::io::Read;

const USAGE: &str = "psi check FILE.psi|- [--out FILE.png|-] [--frames N] [--ms 0,250,…] [--scale K] [--a V] [--b V] [--c V] [--clock HH:MM:SS] [--over] [--hash] [--listing] [--seed V]";

pub fn run(args: Vec<String>) -> i32 {
    let mut args = args.into_iter();
    let (mut path, mut out, mut frames, mut scale) =
        (None::<String>, None::<String>, 8usize, 4usize);
    let (mut ms_list, mut over, mut hash, mut listing) = (None::<Vec<u32>>, false, false, false);
    let mut sl: [Option<f64>; 3] = [None; 3];
    let mut seed = 0f64;
    let mut clock = (10u8, 9u8, 30u8);
    let num = |v: Option<String>, what: &str| -> f64 {
        v.and_then(|s| s.parse().ok())
            .unwrap_or_else(|| die(&format!("{what} needs a number")))
    };
    while let Some(a) = args.next() {
        match a.as_str() {
            "--out" => out = args.next(),
            "--frames" => frames = num(args.next(), "--frames").max(1.0) as usize,
            "--scale" => scale = num(args.next(), "--scale").clamp(1.0, 16.0) as usize,
            "--a" => sl[0] = Some(num(args.next(), "--a")),
            "--b" => sl[1] = Some(num(args.next(), "--b")),
            "--c" => sl[2] = Some(num(args.next(), "--c")),
            "--seed" => seed = num(args.next(), "--seed"),
            "--ms" => {
                let v = args.next().unwrap_or_default();
                ms_list = Some(
                    v.split(',')
                        .map(|s| {
                            s.trim()
                                .parse()
                                .unwrap_or_else(|_| die("--ms takes 0,250,…"))
                        })
                        .collect(),
                );
            }
            "--clock" => {
                let v = args.next().unwrap_or_default();
                let p: Vec<u8> = v.split(':').filter_map(|s| s.parse().ok()).collect();
                if p.len() != 3 || p[0] > 23 || p[1] > 59 || p[2] > 59 {
                    die("--clock takes HH:MM:SS");
                }
                clock = (p[0], p[1], p[2]);
            }
            "--over" => over = true,
            "--hash" => hash = true,
            "--listing" => listing = true,
            "-h" | "--help" => {
                println!("{USAGE}");
                return 0;
            }
            _ if path.is_none() => path = Some(a),
            _ => die(&format!("unknown option {a}")),
        }
    }
    let path = path.unwrap_or_else(|| die("which formula? (a .psi file, or - for stdin)"));
    let src = read(&path);

    let prog = match compile(&src) {
        Ok(p) => p,
        Err(e) => {
            print!("{}", crate::render_error(&e, &src));
            return 1;
        }
    };

    // What it is.
    let (f, r, x) = prog.counts();
    let inputs = prog.inputs();
    let plain_us = prog.esp32_us();
    // As the player draws it: with its still cache when it has values for one.
    let us = prog.esp32_us_still();
    let rating = match us {
        0..=5000 => "light",
        5001..=8000 => "medium",
        _ => "heavy",
    };
    let reads: Vec<&str> = [
        (1, "a"),
        (2, "b"),
        (4, "c"),
        (8, "t"),
        (16, "the clock"),
        (32, "the picture under it"),
        (64, "seed"),
    ]
    .iter()
    .filter(|e| inputs & e.0 != 0)
    .map(|e| e.1)
    .collect();
    println!(
        "ok: {} of {MAX_OPS} steps (per frame {f}, per row {r}, per pixel {x}), {} layer{}, {} of {REGS} registers, {} of {MAX_SCALARS} values",
        prog.steps(),
        prog.layers(),
        if prog.layers() == 1 { "" } else { "s" },
        prog.regs(),
        prog.values()
    );
    // What it declares (`name`, `control`, `look`).
    let mut p2 = Box::<opendmd_psi::Program>::default();
    if let Ok(d) = opendmd_psi::compile_declared(&src, &mut p2) {
        let text = |r: (u32, u32)| &src[r.0 as usize..r.1 as usize];
        let mut parts = Vec::new();
        if let Some(n) = d.name {
            parts.push(format!("name: {}", text(n)));
        }
        let ctl: Vec<String> = (0..3)
            .filter_map(|k| {
                let c = d.controls[k]?;
                let label = c.label.map(|l| format!(" \"{}\"", text(l))).unwrap_or_default();
                Some(match prog.meta().control(k)? {
                    opendmd_psi::Control::Choice { .. } => format!(
                        "{} ({}){label}",
                        text(c.name),
                        text(c.words?).split(',').map(str::trim).collect::<Vec<_>>().join("|")
                    ),
                    opendmd_psi::Control::Slider { lo, hi, default } => format!(
                        "{} {}..{} = {}{label}",
                        text(c.name),
                        f64::from(lo) / 65536.0,
                        f64::from(hi) / 65536.0,
                        f64::from(default) / 65536.0
                    ),
                })
            })
            .collect();
        if !ctl.is_empty() {
            parts.push(format!("controls: {}", ctl.join(", ")));
        }
        if let Some((b, k)) = prog.meta().look() {
            parts.push(format!("look: {b} {k}"));
        }
        if !parts.is_empty() {
            println!("{}", parts.join(" · "));
        }
    }
    if prog.draws() {
        println!(
            "draws: {} steps in draw {{ }}, {} lines and dots a frame",
            prog.draw_steps(),
            prog.primitives()
        );
    }
    let grid = prog.frame_ms().unwrap_or(20);
    let lp = if prog.forever() {
        "never repeats (loop forever)".to_string()
    } else {
        format!("loop {} s", prog.loop_ms() as f64 / 1000.0)
    };
    println!(
        "{lp} at {} fps; reads {}",
        1000 / grid,
        if reads.is_empty() {
            "nothing (a still picture)".into()
        } else {
            reads.join(", ")
        }
    );
    println!(
        "panel: ~{:.1} ms a frame: {rating} (light ≤ 5 ms, medium ≤ 8 ms; GIF looks take ≤ 12 ms)",
        us as f64 / 1000.0
    );
    // The panel's JIT (pixel and row stages compiled; never slower: the
    // player keeps whichever draws faster). The limits stay the VM's.
    let jit = prog.esp32_us_jit_still();
    if jit < us {
        println!(
            "compiled: ~{:.1} ms a frame with the panel's JIT (the VM's ~{:.1} ms is what the limits count)",
            jit as f64 / 1000.0,
            us as f64 / 1000.0
        );
    } else {
        println!("compiled: no faster with the panel's JIT (it draws on the VM)");
    }
    // What compiling it takes on the panel: the compiler's heap at its peak
    // (by tokens), and what the panel asks to have free first.
    let heap = opendmd_psi::compile_heap_estimate(&src);
    let asks = PANEL_COMPILE_STACK + 512 + heap + PANEL_COMPILE_RESERVE;
    let tokens = opendmd_psi::syntax::tokens(&src).map_or(0, |t| t.len());
    println!(
        "compile: {} tokens, ~{:.1} KB of heap; the panel asks for ~{:.1} KB free first ({}about {} KB is free between items)",
        n(tokens),
        heap as f64 / 1024.0,
        asks as f64 / 1024.0,
        if asks > PANEL_FREE { "more than it has: " } else { "" },
        PANEL_FREE / 1024
    );
    // How long it is, each way it travels.
    let kept = kept_on_panel(&src);
    let p = path_len(&opendmd_psi::compact(&src));
    println!(
        "length: {} B as written ({} at most), {} B as the panel keeps it saved ({} at most), {} B as a path (@formula?src=…: {} to show it, {} for an inline look){}",
        n(src.len()),
        n(opendmd_psi::MAX_SOURCE),
        n(kept),
        n(MAX_FORMULA_SOURCE),
        n(p),
        n(MAX_PATH),
        n(MAX_LOOK_PATH),
        if src.len() > GALLERY_MAX { format!("; over the gallery's {} B", n(GALLERY_MAX)) } else { String::new() }
    );
    if prog.still_lanes() > 0 {
        println!(
            "still cache: {} value{} worked out once ({} KB of the player's heap); ~{:.1} ms a frame without it",
            prog.still_lanes(),
            if prog.still_lanes() == 1 { "" } else { "s" },
            Still::bytes_for(&prog) / 1024,
            plain_us as f64 / 1000.0
        );
    }
    if grid == 10 && us > 5000 {
        println!("fps 100: over its 5 ms budget, so the panel plays it at 50 fps or fewer");
    }
    if listing {
        print!("{}", prog.listing());
    }

    // The frames: over one loop; a clock-only formula over a day; a still one once.
    let moves = inputs & 8 != 0;
    let clocked = inputs & 16 != 0 && !moves;
    let times: Vec<(u32, (u8, u8, u8))> = match ms_list {
        Some(v) => v.into_iter().map(|ms| (ms, clock)).collect(),
        None if moves => (0..frames)
            .map(|k| {
                // Whole frames of its grid, as the panel plays them (one
                // that never repeats: over its first minute).
                let lp = if prog.forever() {
                    60_000
                } else {
                    prog.loop_ms()
                };
                let ms = (lp as u64 * k as u64 / frames as u64) as u32 / grid * grid;
                (ms, clock)
            })
            .collect(),
        None if clocked => (0..frames)
            .map(|k| {
                // Across the day, the minutes moving too.
                let m = (k * 24 * 60 / frames + k * 13) as u32 % (24 * 60);
                (0, ((m / 60) as u8, (m % 60) as u8, ((k * 17) % 60) as u8))
            })
            .collect(),
        None => vec![(0, clock)],
    };
    let under = (over || prog.over()).then(sample);
    let mut scratch = Scratch::<128>::new();
    let mut ink = Ink::boxed();
    let mut still = Still::new();
    still.take(&prog);
    let mut shots = Vec::new();
    // A slider not given: its control's default, else ½.
    let slider = |k: usize| match sl[k] {
        Some(v) => q(v),
        None if prog.meta().declared() & (1 << k) != 0 => prog.meta().default_value(k),
        None => q(0.5),
    };
    for &(ms, (hh, mm, ss)) in &times {
        let mut inp = Inputs {
            hour: hh,
            minute: mm,
            second: ss,
            a: slider(0),
            b: slider(1),
            c: slider(2),
            seed: q(seed),
            ..Inputs::default()
        };
        inp.set_ms(&prog, u64::from(ms));
        let mut frame = vec![0u8; W * H * 3];
        render_rows_ink(
            &prog,
            &mut scratch,
            Some(&mut ink),
            &inp,
            &OWN,
            under.as_deref(),
            u32::MAX,
            &mut frame,
        );
        if still.is_held() {
            // The still cache keeps its values on the first frame and draws
            // from them after: the same bytes either way.
            let mut cached = vec![0u8; W * H * 3];
            for _ in 0..2 {
                render_rows_still(
                    &prog,
                    &mut scratch,
                    Some(&mut ink),
                    Some(&mut still),
                    &inp,
                    &OWN,
                    under.as_deref(),
                    u32::MAX,
                    &mut cached,
                );
                if cached != frame {
                    die("the still cache drew different bytes (a bug in Psi)");
                }
            }
        }
        shots.push(frame);
    }
    let label: Vec<String> = times
        .iter()
        .map(|&(ms, (hh, mm, ss))| {
            if clocked {
                format!("{hh:02}:{mm:02}:{ss:02}")
            } else {
                format!("{ms} ms")
            }
        })
        .collect();
    if hash {
        for (l, s) in label.iter().zip(&shots) {
            println!("frame {l}: {:#x}", fnv(s));
        }
    }
    let out = out.unwrap_or_else(|| {
        if path == "-" {
            "formula.png".into()
        } else {
            std::path::Path::new(&path)
                .with_extension("png")
                .to_string_lossy()
                .into_owned()
        }
    });
    if out != "-" {
        let png = sheet(&shots, scale);
        std::fs::write(&out, png).unwrap_or_else(|e| die(&format!("{out}: {e}")));
        println!(
            "frames: {} → {out} ({} a row, left to right, {scale}x{})",
            label.join(", "),
            COLS.min(shots.len()),
            if under.is_some() {
                ", over a sample picture"
            } else {
                ""
            }
        );
    }
    0
}

fn die(msg: &str) -> ! {
    eprintln!("psi check: {msg}");
    std::process::exit(2)
}

/// `16384` as `16,384`.
fn n(v: usize) -> String {
    let s = v.to_string();
    let mut o = String::new();
    for (k, c) in s.chars().enumerate() {
        if k > 0 && (s.len() - k).is_multiple_of(3) {
            o.push(',');
        }
        o.push(c);
    }
    o
}

/// The file, or stdin for `-`.
pub fn read(path: &str) -> String {
    if path == "-" {
        let mut s = String::new();
        std::io::stdin()
            .read_to_string(&mut s)
            .unwrap_or_else(|e| die(&e.to_string()));
        s
    } else {
        std::fs::read_to_string(path).unwrap_or_else(|e| die(&format!("{path}: {e}")))
    }
}

// The panel's numbers (firmware/src/config.rs, opendmd-core/src/synth.rs,
// js/psikit.js, opendmd-gallery): what `psi check` measures a Formula
// against. `PANEL_FREE` is the heap measured free between items on the
// DevKit (CLAUDE.md, "The compiler's heap").
const PANEL_COMPILE_STACK: usize = 12 * 1024;
const PANEL_COMPILE_RESERVE: usize = 6 * 1024;
const PANEL_FREE: usize = 63 * 1024;
const MAX_FORMULA_SOURCE: usize = 8192;
const MAX_LOOK_PATH: usize = 2048;
const MAX_PATH: usize = 6144;
const GALLERY_MAX: usize = 4096;

/// What the panel keeps of a saved Formula (opendmd-core `fx_psi::read_text`):
/// its lines trimmed, blank and comment-only ones left out.
fn kept_on_panel(src: &str) -> usize {
    src.lines()
        .map(str::trim)
        .filter(|l| !(l.is_empty() || l.starts_with("--") || l.starts_with("//")))
        .map(|l| l.len() + 1)
        .sum()
}

/// `@formula?src=` and the compacted text as js/psikit.js `enc` writes it:
/// a space `+`; `% & + # ?`, controls and non-ASCII as %XX.
fn path_len(compact: &str) -> usize {
    "@formula?src=".len()
        + compact
            .bytes()
            .map(|b| if b == b' ' || (b > 32 && b < 127 && !b"%&+#?".contains(&b)) { 1 } else { 3 })
            .sum::<usize>()
}

fn q(v: f64) -> i32 {
    q16((v.clamp(-32767.0, 32767.0) * 10000.0).round() as i32, 10000)
}

/// The web app's sample picture (js/psikit.js: a sky, a sun and hills),
/// what the editor draws an over-the-GIF formula over: 128 × 32 RGB, top
/// row first.
fn sample() -> Vec<u8> {
    let mut p = vec![0u8; W * H * 3];
    for i in 0..W * H {
        let (x, y) = ((i % W) as f64, (i / W) as f64);
        let (xi, yi) = (i % W, i / W);
        let c = if y > 23.0 - 4.0 * (x / 13.0).sin() - 2.0 * (x / 5.0).sin() {
            [30, (110 + (xi * 7 + yi * 13) % 50) as u8, 40]
        } else if (x - 98.0).powi(2) + 1.5 * (y - 10.0).powi(2) < 50.0 {
            [255, 215, 80]
        } else {
            [
                (40 + yi * 6) as u8,
                (60 + yi * 4) as u8,
                (170 - yi * 3) as u8,
            ]
        };
        p[i * 3..i * 3 + 3].copy_from_slice(&c);
    }
    p
}

const COLS: usize = 4;
const GAP: usize = 4;

/// The frames in rows of [`COLS`], each scaled `s` times, as a PNG.
fn sheet(shots: &[Vec<u8>], s: usize) -> Vec<u8> {
    let cols = COLS.min(shots.len());
    let rows = shots.len().div_ceil(COLS);
    let (fw, fh) = (W * s, H * s);
    let (iw, ih) = (GAP + cols * (fw + GAP), GAP + rows * (fh + GAP));
    let mut img = vec![48u8; iw * ih * 3];
    for (k, f) in shots.iter().enumerate() {
        let (ox, oy) = (GAP + (k % COLS) * (fw + GAP), GAP + (k / COLS) * (fh + GAP));
        for y in 0..fh {
            for x in 0..fw {
                let src = ((y / s) * W + x / s) * 3;
                let dst = ((oy + y) * iw + ox + x) * 3;
                img[dst..dst + 3].copy_from_slice(&f[src..src + 3]);
            }
        }
    }
    png(iw, ih, &img)
}

/// An RGB PNG, uncompressed (stored deflate blocks): no dependencies.
fn png(w: usize, h: usize, rgb: &[u8]) -> Vec<u8> {
    let mut raw = Vec::with_capacity(h * (w * 3 + 1));
    for row in rgb.chunks(w * 3) {
        raw.push(0);
        raw.extend_from_slice(row);
    }
    let mut z = vec![0x78, 0x01];
    let chunks: Vec<&[u8]> = raw.chunks(65535).collect();
    for (k, c) in chunks.iter().enumerate() {
        z.push((k + 1 == chunks.len()) as u8);
        z.extend_from_slice(&(c.len() as u16).to_le_bytes());
        z.extend_from_slice(&(!(c.len() as u16)).to_le_bytes());
        z.extend_from_slice(c);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &x in &raw {
        a = (a + x as u32) % 65521;
        b = (b + a) % 65521;
    }
    z.extend_from_slice(&((b << 16) | a).to_be_bytes());
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&(w as u32).to_be_bytes());
    ihdr.extend_from_slice(&(h as u32).to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    for (ty, data) in [(b"IHDR", &ihdr[..]), (b"IDAT", &z[..]), (b"IEND", &[][..])] {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let mut c = ty.to_vec();
        c.extend_from_slice(data);
        out.extend_from_slice(&c);
        out.extend_from_slice(&crc32(&c).to_be_bytes());
    }
    out
}

fn crc32(d: &[u8]) -> u32 {
    let mut c = !0u32;
    for &b in d {
        c ^= b as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 {
                0xedb8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
        }
    }
    !c
}

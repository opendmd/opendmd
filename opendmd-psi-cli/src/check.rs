//! `psi check`: does a candidate Formula compile — and if not, its error's
//! code, the line, the column and carets —, how heavy it is for the panel
//! (on the VM and compiled by the panel's JIT), what it takes to compile
//! on the panel, how long it is compiled, what it reads, and a PNG
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
//! frames over its first minute. `--levels`: every value the formula sets,
//! how often it is worked out and why (the chain of values to the built-in
//! that makes it per pixel). `--same-as OTHER.psi [--seeds N]`: whether the
//! two are the same program, and the same frames (N seeds, 3 by default, ×
//! `--frames` frames, 120 by default, over a loop or a `loop forever`
//! formula's first 5 minutes, the clock moving; and 8 each with the sliders
//! at 0 and at 1): exit 1 when a frame differs. The compiler's notes (a
//! `pick` worked out per pixel, a sum that brackets better, what the still
//! cache and the column run took, draw loops run as lanes) and whether the
//! panel's JIT takes it (and why not) are always printed. Exit status 1
//! when it doesn't compile, 2 for a missing file or a bad option.

use opendmd_psi::golden::fnv;
use opendmd_psi::syntax::{compile_observed, Defined, Level, Sink};
use opendmd_psi::{
    compile, q16, render_rows_ink, render_rows_still, Ink, Inputs, Scratch, Still, H, MAX_OPS,
    MAX_SCALARS, OWN, REGS, W,
};
use std::io::Read;

const USAGE: &str = "psi check FILE.psi|- [--out FILE.png|-] [--frames N] [--ms 0,250,…] [--scale K] [--a V] [--b V] [--c V] [--clock HH:MM:SS] [--over] [--hash] [--listing] [--seed V] [--levels] [--same-as OTHER.psi [--seeds N]]";

pub fn run(args: Vec<String>) -> i32 {
    let mut args = args.into_iter();
    let (mut path, mut out, mut frames, mut scale) =
        (None::<String>, None::<String>, None::<usize>, 4usize);
    let (mut levels, mut same_as, mut seeds) = (false, None::<String>, 3usize);
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
            "--frames" => frames = Some(num(args.next(), "--frames").max(1.0) as usize),
            "--levels" => levels = true,
            "--same-as" => {
                same_as = Some(args.next().unwrap_or_else(|| die("--same-as needs a file")))
            }
            "--seeds" => seeds = num(args.next(), "--seeds").clamp(1.0, 64.0) as usize,
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
        "ok: {} of {MAX_OPS} steps (per frame {f}, draw {}, per row {r}, per pixel {x}), {} layer{}, {} of {REGS} registers, {} of {MAX_SCALARS} values",
        prog.steps(),
        prog.draw_steps(),
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
    match opendmd_psi::jit::refusal(&prog) {
        Some(why) => println!(
            "JIT: no, {why}{}: the panel draws it on the VM",
            if why.contains("registers") {
                format!(" ({} values at once)", prog.regs())
            } else {
                String::new()
            }
        ),
        None if jit < us => println!(
            "JIT: yes, ~{:.1} ms a frame compiled (the VM's ~{:.1} ms is what the limits count)",
            jit as f64 / 1000.0,
            us as f64 / 1000.0
        ),
        None => println!("JIT: yes, but no faster by the estimate (the panel keeps whichever draws faster)"),
    }
    // How long it is, as written and compiled (the player never compiles:
    // it plays the compiled Formula, a card's `.psb`, or `@formula?psb=…`).
    let psb = opendmd_psi::build(&src).map_or(0, |b| b.len());
    let p = "@formula?psb=".len() + psb.div_ceil(3) * 4;
    println!(
        "length: {} B as written ({} at most), {} B compiled ({} B as a path, @formula?psb=…: {} to show it, {} for an inline look){}",
        n(src.len()),
        n(opendmd_psi::MAX_SOURCE),
        n(psb),
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
    // What the compiler saw: its notes, and with --levels every value's level.
    let mut seen = Seen::default();
    let _ = compile_observed(&src, &mut Box::<opendmd_psi::Program>::default(), &mut seen);
    let line = |at: usize| src[..at.min(src.len())].bytes().filter(|&b| b == b'\n').count() + 1;
    for (at, text) in &seen.notes {
        println!("note: line {}: {text}", line(at.0));
    }
    // Past light: where the time goes (the steps a frame, by line).
    if us > 5000 {
        print!("heaviest lines (their share of the work a frame; make these lighter first):\n{}", seen.heaviest(&src));
    }
    if levels {
        print!("{}", seen.levels(&src));
    }
    let mut status = 0;
    if let Some(other) = &same_as {
        status = same(&prog, &path, other, seeds, frames.unwrap_or(120), &sl, seed);
    }
    let frames = frames.unwrap_or(8);

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
    status
}

/// What `compile_observed` told: each value (where, its level, why) and
/// the notes.
#[derive(Default)]
struct Seen {
    defs: Vec<Def>,
    notes: Vec<((usize, usize), String)>,
    /// Each step the compiler made: where, and its cost a frame (ns: a
    /// pixel's ×4,096, a row's ×32), for the heaviest lines.
    ops: Vec<(usize, u64)>,
}

struct Def {
    name: String,
    at: (usize, usize),
    level: Level,
    via: Option<(usize, usize)>,
    input: String,
}

impl Sink for Seen {
    fn defined(&mut self, name: &str, at: (usize, usize), what: Defined, _: u16, level: Level) {
        if matches!(what, Defined::Value | Defined::Counter) {
            self.defs.push(Def { name: name.into(), at, level, via: None, input: String::new() });
        }
    }
    fn why(&mut self, at: (usize, usize), via: Option<(usize, usize)>, input: &str) {
        if let Some(d) = self.defs.iter_mut().rev().find(|d| d.at == at) {
            d.via = via;
            d.input = input.into();
        }
    }
    fn note(&mut self, at: (usize, usize), text: &str) {
        self.notes.push((at, text.into()));
    }
    fn op(&mut self, at: (usize, usize), level: Level, _: u8, cost_ns: u32) {
        let n = match level {
            Level::Pixel => 4096,
            Level::Row => 32,
            _ => 1,
        };
        self.ops.push((at.0, u64::from(cost_ns) * n));
    }
}

impl Seen {
    /// The lines whose steps cost the most a frame (pixel steps 4,096
    /// times, row steps 32), with their share: what to make lighter first.
    fn heaviest(&self, src: &str) -> String {
        let line = |at: usize| src[..at.min(src.len())].bytes().filter(|&b| b == b'\n').count() + 1;
        let mut per: std::collections::BTreeMap<usize, u64> = Default::default();
        for &(at, ns) in &self.ops {
            *per.entry(line(at)).or_default() += ns;
        }
        let total: u64 = per.values().sum::<u64>().max(1);
        let mut v: Vec<(usize, u64)> = per.into_iter().collect();
        v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        let lines: Vec<&str> = src.lines().collect();
        let mut out = String::new();
        for (l, ns) in v.into_iter().take(3) {
            let text = lines.get(l - 1).map_or("", |t| t.trim());
            let text: String = if text.chars().count() > 60 {
                text.chars().take(57).chain("…".chars()).collect()
            } else {
                text.into()
            };
            out += &format!("  line {l} ({}%): {text}\n", ns * 100 / total);
        }
        out
    }
}

impl Seen {
    /// `--levels`: a line a value, in source order: its line, its name, how
    /// often it is worked out and the chain that makes it so.
    fn levels(&self, src: &str) -> String {
        let line = |at: usize| src[..at.min(src.len())].bytes().filter(|&b| b == b'\n').count() + 1;
        let word = |l: Level| match l {
            Level::Constant => "constant",
            Level::Frame => "once a frame",
            Level::Draw => "each turn of a draw loop",
            Level::Row => "per row",
            Level::Pixel => "per pixel",
        };
        let mut defs: Vec<&Def> = self.defs.iter().collect();
        defs.sort_by_key(|d| d.at);
        let w = defs.iter().map(|d| d.name.len()).max().unwrap_or(0);
        let mut out = String::from("levels:\n");
        for d in defs {
            let mut chain = Vec::new();
            let mut at = d.via;
            let mut root = d.input.clone();
            while let Some(v) = at.filter(|_| chain.len() < 12) {
                let Some(e) = self.defs.iter().find(|e| e.at == v) else { break };
                chain.push(format!("`{}` (line {})", e.name, line(e.at.0)));
                root = e.input.clone();
                at = e.via;
            }
            if !root.is_empty() {
                chain.push(format!("`{root}`"));
            }
            let why = if chain.is_empty() || d.level == Level::Constant {
                String::new()
            } else {
                format!(": reads {}", chain.join(" ← "))
            };
            out.push_str(&format!(
                "  line {:>3}  {:<w$}  {}{why}\n",
                line(d.at.0),
                d.name,
                word(d.level)
            ));
        }
        out
    }
}

/// `--same-as`: the same program as `other`, and the same frames.
fn same(
    prog: &opendmd_psi::Program,
    path: &str,
    other: &str,
    seeds: usize,
    frames: usize,
    sl: &[Option<f64>; 3],
    seed0: f64,
) -> i32 {
    let osrc = read(other);
    let oprog = match compile(&osrc) {
        Ok(p) => p,
        Err(e) => {
            println!("same as {other}: no, it doesn't compile:");
            print!("{}", crate::render_error(&e, &osrc));
            return 1;
        }
    };
    let program = if *prog == oprog {
        format!("the same program ({} steps)", prog.steps())
    } else {
        format!("a different program ({} steps against {})", prog.steps(), oprog.steps())
    };
    let span = if prog.forever() || oprog.forever() {
        300_000
    } else {
        prog.loop_ms().max(oprog.loop_ms())
    };
    let under = (prog.over() || oprog.over()).then(sample);
    let slider = |p: &opendmd_psi::Program, k: usize, set: Option<f64>| match set.or(sl[k]) {
        Some(v) => q(v),
        None if p.meta().declared() & (1 << k) != 0 => p.meta().default_value(k),
        None => q(0.5),
    };
    // (seed, ms, clock, sliders: None the defaults)
    type Case = (f64, u32, (u8, u8, u8), Option<f64>);
    let mut cases: Vec<Case> = Vec::new();
    for s in 0..seeds {
        let sd = if s == 0 { seed0 } else { (s as f64 * 0.618_034).fract() };
        for k in 0..frames {
            let ms = (span as u64 * k as u64 / frames as u64) as u32;
            let n = s * frames + k;
            cases.push((sd, ms, ((n * 7 % 24) as u8, (n * 13 % 60) as u8, (n * 17 % 60) as u8), None));
        }
    }
    for v in [0.0, 1.0] {
        for k in 0..8u32 {
            cases.push((seed0, span / 8 * k, (10, 9, 30), Some(v)));
        }
    }
    let mut sa = Scratch::<128>::new();
    let mut sb = Scratch::<128>::new();
    let (mut ia, mut ib) = (Ink::boxed(), Ink::boxed());
    for (n, &(sd, ms, (hh, mm, ss), set)) in cases.iter().enumerate() {
        let mut fa = vec![0u8; W * H * 3];
        let mut fb = vec![0u8; W * H * 3];
        for (p, sc, ink, f) in [(prog, &mut sa, &mut ia, &mut fa), (&oprog, &mut sb, &mut ib, &mut fb)] {
            let mut inp = Inputs {
                hour: hh,
                minute: mm,
                second: ss,
                a: slider(p, 0, set),
                b: slider(p, 1, set),
                c: slider(p, 2, set),
                seed: q(sd),
                ..Inputs::default()
            };
            inp.set_ms(p, u64::from(ms));
            render_rows_ink(p, sc, Some(ink), &inp, &OWN, under.as_deref(), u32::MAX, f);
        }
        if fa != fb {
            println!(
                "same as {other}: no: {program}, and frame {} of {} differs (seed {sd}, {ms} ms, clock {hh:02}:{mm:02}:{ss:02}, sliders {})",
                n + 1,
                cases.len(),
                set.map_or("as set".to_string(), |v| format!("at {v}"))
            );
            return 1;
        }
    }
    let _ = path;
    println!(
        "same as {other}: yes: {program}, and the same frames ({seeds} seed{} × {frames} over {} s, and 16 with the sliders at 0 and 1)",
        if seeds == 1 { "" } else { "s" },
        span / 1000
    );
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

// The panel's numbers (opendmd-core/src/synth.rs, js/psikit.js,
// opendmd-gallery): what `psi check` measures a Formula against.
const MAX_LOOK_PATH: usize = 2048;
const MAX_PATH: usize = 6144;
const GALLERY_MAX: usize = 4096;

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

//! psi-refresh: the board rows of opendmd-psi/tests/measured.rs that
//! `make psi-stale` lists, measured again with the Formulas as they are
//! now, in one boot, and nothing else (no play lines after).
//!
//! Each row is measured the way its estimate test compiles it
//! (opendmd-psi/tests/estimate.rs), so a number here can go into its row
//! as it is:
//!
//! - `STEPS`: [`psi::compile_measured`] with no runs, the checked steps and
//!   unfused compares; the VM's frame directly, no picture, 20 ms apart
//!   within 8 s (as the step table was timed).
//! - `LOOKS`: [`psi::look::set`] at its blend and amount, then
//!   [`psi::compile_look_with`] with no runs, checked steps; the VM's frame
//!   directly over the golden picture.
//! - `SPEED2`: `compile_measured` with the runs and the narrowed steps,
//!   unfused compares; the VM's frame directly with the still cache where
//!   it has values (kept on an untimed first frame), as speed2's rows.
//! - `EFFECTS`: the real path (`Fx::parse` + `Fx::frame`, slider a = 0.5),
//!   the JIT off: the VM's effect frame.
//! - `JIT_EFFECTS`, `CONSOLEB`: the real path with the JIT on (whichever
//!   the trial kept), [`jit2::effect`].
//! - `JIT_LOOKS`: the look over a changing picture, the JIT on,
//!   [`jit2::look`].
//! - `PAINT_FORMULAS`: the real path, JIT on and off (`jit`, `vm`).
//!
//! Each result is one line, `psi-probe: refresh|TABLE|name|look|file|src|
//! values #check`: `look` the blend and amount ("-" for none), `file` the
//! Formula's file (opendmd-web/formulas/), `src` the FNV-1a 64 and length
//! of the exact text measured (the file's bytes, `include_str!`), `values`
//! `key=µs` pairs, the row's number first (`us=`), and `#check` the FNV-1a
//! 32 of everything before it (a line cut or garbled on a console is
//! dropped, never misread). A row that could not be measured says
//! `psi-probe: refresh-failed|TABLE|name|look|why`. At the end the done
//! line, then the result lines logged again, 50 ms apart (the console
//! only).

use super::*;

/// A row's way of measuring (see the module's doc).
#[derive(Clone, Copy)]
enum How {
    /// The VM directly, compiled `compile_measured(runs, narrow, cmp)`;
    /// with the still cache (`still`), over the picture (`under`), `t`
    /// within `loop_ms` (0: the program's loop).
    Vm { runs: bool, narrow: bool, cmp: bool, still: bool, under: bool, loop_ms: u32 },
    /// A look at `blend` `k` %, compiled as `compile_look_with(.., false,
    /// false)`, the VM directly over the picture.
    LookVm,
    /// The real path as an effect with these options: the JIT on and off.
    Effect(&'static str),
    /// The real path as a look over a changing picture: JIT on and off.
    LookPath,
}

/// One row: its table, its name in the table, the look's blend and
/// amount (`""`, 0 for none), the Formula's file stem and text.
struct Row {
    table: &'static str,
    name: &'static str,
    blend: &'static str,
    k: u8,
    stem: &'static str,
    src: &'static str,
    how: How,
}

macro_rules! formula {
    ($stem:literal) => {
        ($stem, include_str!(concat!("../../../opendmd-web/formulas/", $stem, ".psi")))
    };
}

const BIG_CLOCK: (&str, &str) = formula!("big-clock");
const STARFIELD: (&str, &str) = formula!("starfield");
const CUBE: (&str, &str) = formula!("wireframe-cube");
const TUNNEL: (&str, &str) = formula!("tunnel");
const ROTOZOOM: (&str, &str) = formula!("rotozoom");
const FLOOR: (&str, &str) = formula!("rotozoom-floor");
const ROAD: (&str, &str) = formula!("endless-road");
const PLASMA: (&str, &str) = formula!("plasma");
const FIRE: (&str, &str) = formula!("fire");
const PLASMA_LETTERS: (&str, &str) = formula!("plasma-letters");
const TIME_GATE: (&str, &str) = formula!("time-gate");
const OLD_TV: (&str, &str) = formula!("old-tv");
const STRIPES: (&str, &str) = formula!("stripes");
const COPPER_BARS: (&str, &str) = formula!("copper-bars");
const OLD_PHOTO: (&str, &str) = formula!("old-photo");
const CYCLED: (&str, &str) = formula!("cycled-plasma");
const WATERFALL: (&str, &str) = formula!("waterfall");
const COPPER_SKY: (&str, &str) = formula!("copper-sky");
const STORM: (&str, &str) = formula!("storm-front");
const NEON: (&str, &str) = formula!("neon-city");

// Calibration (`CAL_*`, not rows of measured.rs: tools/psi-refresh.py
// leaves them out): rows whose sources have not changed, measured again as
// they were, so the stale rows' new numbers can be told apart from the
// board and the firmware having changed under them (the VM's step rows,
// a look, two JIT rows). Their old numbers: 678, 1346, 4540, 6614, 1429, 818.
const CAL_BASE: (&str, &str) = ("-", "gray(x)");
const CAL_SIN: (&str, &str) = ("-", "gray(sin(x + t))");
const CAL_NOISE: (&str, &str) = ("-", "gray(noise(x * 8, y, t))");
const CAL_RINGS: (&str, &str) = ("-", "loop 4\n-- r: the distance from the middle\npal(rainbow, frac(r*1.5 - t/2))");
const CAL_P0_BIG: (&str, &str) = ("-", include_str!("../../../opendmd-psi/tests/device/p0-big.psi"));
const CAL_P0_TWEEN: (&str, &str) = ("-", include_str!("../../../opendmd-psi/tests/device/p0-tween.psi"));

const STEP: How = How::Vm { runs: false, narrow: false, cmp: false, still: false, under: false, loop_ms: 8000 };
const SPEED: How = How::Vm { runs: true, narrow: true, cmp: false, still: true, under: false, loop_ms: 0 };
const REAL: How = How::Effect("");

const fn row(table: &'static str, name: &'static str, f: (&'static str, &'static str), how: How) -> Row {
    Row { table, name, blend: "", k: 0, stem: f.0, src: f.1, how }
}

const fn look(table: &'static str, name: &'static str, f: (&'static str, &'static str), blend: &'static str, k: u8, how: How) -> Row {
    Row { table, name, blend, k, stem: f.0, src: f.1, how }
}

/// The rows, the real path's first, the biggest Formulas first (the heap is
/// least cut up then).
const ROWS: &[Row] = &[
    row("PAINT_FORMULAS", "Neon city", NEON, REAL),
    row("JIT_EFFECTS", "Endless road", ROAD, REAL),
    row("JIT_EFFECTS", "Time gate", TIME_GATE, REAL),
    row("JIT_EFFECTS", "Wireframe cube", CUBE, REAL),
    row("JIT_EFFECTS", "Plasma letters", PLASMA_LETTERS, REAL),
    row("JIT_EFFECTS", "Rotozoom floor", FLOOR, REAL),
    row("JIT_EFFECTS", "Tunnel", TUNNEL, REAL),
    row("JIT_EFFECTS", "Fire", FIRE, REAL),
    row("JIT_EFFECTS", "Plasma", PLASMA, REAL),
    look("JIT_LOOKS", "Old TV", OLD_TV, "over", 100, How::LookPath),
    look("JIT_LOOKS", "Plasma letters", PLASMA_LETTERS, "over", 100, How::LookPath),
    look("JIT_LOOKS", "Plasma", PLASMA, "screen", 40, How::LookPath),
    row("CAL_JIT_EFFECTS", "p0 big", CAL_P0_BIG, REAL),
    row("CAL_JIT_EFFECTS", "p0 tween", CAL_P0_TWEEN, REAL),
    row("CONSOLEB", "Cycled plasma", CYCLED, REAL),
    row("CONSOLEB", "Waterfall", WATERFALL, REAL),
    row("CONSOLEB", "Copper sky", COPPER_SKY, REAL),
    // (Not a stale row: the one bundled `layer rate` Formula, measured in
    // case it takes the place of the retired "Big clock rate" row.)
    row("CONSOLEB", "Storm front", STORM, REAL),
    row("EFFECTS", "Starfield", STARFIELD, How::Effect("a=0.5")),
    row("EFFECTS", "Wireframe cube", CUBE, How::Effect("a=0.5")),
    // (The VM's rows after the real path's: the canvas let go first, the
    // picture under the looks taken only for them; the still cache's 32 KB
    // for Tunnel.)
    row("SPEED2", "Endless road", ROAD, SPEED),
    row("SPEED2", "Starfield", STARFIELD, SPEED),
    row("SPEED2", "Wireframe cube", CUBE, SPEED),
    row("SPEED2", "Tunnel", TUNNEL, SPEED),
    row("SPEED2", "Rotozoom", ROTOZOOM, SPEED),
    row("SPEED2", "Rotozoom floor", FLOOR, SPEED),
    row("SPEED2", "Plasma", PLASMA, SPEED),
    row("SPEED2", "Fire", FIRE, SPEED),
    row("STEPS", "big clock", BIG_CLOCK, STEP),
    look("LOOKS", "Plasma", PLASMA, "screen", 40, How::LookVm),
    look("LOOKS", "Plasma", PLASMA, "over", 30, How::LookVm),
    look("LOOKS", "Plasma", PLASMA, "add", 40, How::LookVm),
    look("LOOKS", "Plasma", PLASMA, "multiply", 100, How::LookVm),
    look("LOOKS", "Plasma", PLASMA, "max", 50, How::LookVm),
    look("LOOKS", "Fire", FIRE, "screen", 40, How::LookVm),
    look("LOOKS", "Fire", FIRE, "over", 30, How::LookVm),
    look("LOOKS", "Stripes", STRIPES, "screen", 40, How::LookVm),
    look("LOOKS", "Copper bars", COPPER_BARS, "screen", 40, How::LookVm),
    look("LOOKS", "Old photo", OLD_PHOTO, "screen", 50, How::LookVm),
    look("LOOKS", "Old TV", OLD_TV, "over", 100, How::LookVm),
    row("CAL_STEPS", "base gray(x)", CAL_BASE, STEP),
    row("CAL_STEPS", "sin (+add)", CAL_SIN, STEP),
    row("CAL_STEPS", "noise (+mul)", CAL_NOISE, STEP),
    look("CAL_LOOKS", "Rings", CAL_RINGS, "screen", 40, How::LookVm),
];

fn fnv64(b: &[u8]) -> u64 {
    b.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &c| (h ^ u64::from(c)).wrapping_mul(0x0000_0100_0000_01b3))
}

fn fnv32(b: &[u8]) -> u32 {
    b.iter().fold(0x811c_9dc5u32, |h, &c| (h ^ u32::from(c)).wrapping_mul(0x0100_0193))
}

/// `line` with its check (`#` and the FNV-1a 32 of the line, 8 hex digits).
fn checked(line: String) -> String {
    let c = fnv32(line.as_bytes());
    format!("{line} #{c:08x}")
}

fn look_of(r: &Row) -> String {
    if r.blend.is_empty() {
        "-".into()
    } else {
        format!("{} {}", r.blend, r.k)
    }
}

/// The VM's frame directly, as [`How::Vm`] or [`How::LookVm`] says:
/// `key=value` pairs, or why not.
fn vm_row(r: &Row, under: &[u8], out: &mut [u8]) -> Result<String, String> {
    let mut prog = zeroed::<psi::Program>();
    let composed;
    let compiled = match r.how {
        How::Vm { runs, narrow, cmp, .. } => {
            crate::psi_stack::run(|| psi::compile_measured(r.src, &mut prog, runs, narrow, cmp).map(|_| ()))
        }
        How::LookVm => {
            composed = psi::look::set(r.src, r.blend, r.k).ok_or_else(|| String::from("does not compose"))?;
            crate::psi_stack::run(|| psi::compile_look_with(&composed, &mut prog, false, false).map(|_| ()))
        }
        _ => return Err("not a VM row".into()),
    };
    match compiled {
        Some(Ok(())) => {}
        Some(Err(e)) => return Err(format!("does not compile: {}", e.msg)),
        None => return Err("no formula stack".into()),
    }
    let (still, over, loop_ms) = match r.how {
        How::Vm { still, under, loop_ms, .. } => (still, under, if loop_ms == 0 { prog.loop_ms() } else { loop_ms }),
        _ => (false, true, prog.loop_ms()),
    };
    let u = over.then_some(under);
    let (p50, p90, max, cached) = if still {
        // (speed2's `time_way`: the still cache kept on an untimed first
        // frame, then drawn from. Its lanes, 16 KB each, taken first, before
        // the scratch and the canvas; a row whose cache the heap hasn't is
        // failed, never timed without it: its estimate counts on it.)
        let mut st = psi::Still::new();
        let cached = prog.still_lanes() > 0 && st.take(&prog);
        if prog.still_lanes() > 0 && !cached {
            return Err(format!(
                "no heap for its still cache ({} B; {} B free, largest block {} B)",
                psi::Still::bytes_for(&prog), heap(), crate::state::largest_free_block()
            ));
        }
        let mut s = zeroed::<psi::Scratch<128>>();
        s.reset();
        let mut ink = psi::Ink::boxed();
        draw_way(&prog, Way::Fused, &mut s, &mut ink, cached.then_some(&mut st), 0, out);
        let (a, b, c) = timed(|k| draw_way(&prog, Way::Fused, &mut s, &mut ink, cached.then_some(&mut st), k * 20, out));
        (a, b, c, cached)
    } else {
        let (a, b, c) = lanes_over::<128>(&prog, loop_ms, u, out);
        (a, b, c, false)
    };
    Ok(format!(
        "us={p50} p90={p90} max={max} still={} est={} est_still={} est_jit={} est_jit_still={}",
        u8::from(cached),
        prog.esp32_us(),
        prog.esp32_us_still(),
        prog.esp32_us_jit(),
        prog.esp32_us_jit_still()
    ))
}

/// The real path (effect or look), JIT on and off: the row's number is
/// the JIT's (`EFFECTS`: the VM's), then both.
fn real_row(r: &Row, canvas: &mut Canvas) -> Result<String, String> {
    for c in &fx_psi::RATE_FRAMES {
        c.store(0, Ordering::Relaxed);
    }
    let got = match r.how {
        How::Effect(opts) => jit2::effect(r.name, r.src, opts, canvas),
        How::LookPath => {
            // (As jit2::timings builds its looks: a look's own file for
            // `over 100`, the blend set on it otherwise.)
            let src = if r.blend == "over" && r.k == 100 {
                r.src.to_string()
            } else {
                psi::look::set(r.src, r.blend, r.k).ok_or_else(|| String::from("does not compose"))?
            };
            jit2::look(r.name, &fx_psi::path(&src, ""), canvas)
        }
        _ => return Err("not a real-path row".into()),
    };
    let Some((jit, vm, kernel)) = got else {
        return Err(format!("did not open or ran short ({} B free, largest block {} B)", heap(), crate::state::largest_free_block()));
    };
    let us = if r.table == "EFFECTS" { vm } else { jit };
    Ok(format!("us={us} jit={jit} vm={vm} kernel={}", u8::from(kernel)))
}

/// The golden picture the looks are drawn over, made the first time.
fn picture(under: &mut Option<Vec<u8>>) -> &[u8] {
    under.get_or_insert_with(|| {
        let mut u = vec![0u8; psi::W * psi::H * 3];
        psi::golden::picture(&mut u);
        u
    })
}

pub(super) fn probe(start: Instant, stack0: u32) {
    say!(info,
        "psi-probe: refresh: start: main stack {stack0} B unused, heap {} B free (largest {} B), {} rows",
        heap(), crate::state::largest_free_block(), ROWS.len()
    );
    psi::stats::CLOCK.store(ccount as fn() -> u32 as usize, Ordering::Relaxed);
    // The goldens first (the VM's and through kernels, as JIT2_ONLY): the
    // emulator lane's check that what is timed below draws what the host
    // draws.
    {
        let mut out = vec![0u8; psi::W * psi::H * 3];
        let (vo, vb) = vm_goldens(&mut out, false);
        let (ko, kb) = jit2::goldens(&mut out);
        say!(info,
            "psi-probe: golden frames: {} match the host/wasm hashes, {} differ{}",
            vo + ko, vb + kb, if vb + kb == 0 { " (bit-identical)" } else { "" }
        );
    }
    fx_psi::set_still_gate(Some(crate::still_gate));
    fx_psi::set_jit_gate(Some(crate::jit_gate));
    let mut canvas = Some(Canvas::new());
    let mut under: Option<Vec<u8>> = None;
    let mut out = vec![0u8; psi::W * psi::H * 3];
    let mut results = Vec::with_capacity(ROWS.len());
    let mut failed = 0u32;
    for r in ROWS {
        let t0 = Instant::now();
        let got = match r.how {
            How::Vm { under: over, .. } => {
                canvas = None;
                vm_row(r, if over { picture(&mut under) } else { &[] }, &mut out)
            }
            How::LookVm => {
                canvas = None;
                vm_row(r, picture(&mut under), &mut out)
            }
            How::Effect(_) | How::LookPath => real_row(r, canvas.get_or_insert_with(Canvas::new)),
        };
        let line = match got {
            Ok(values) => {
                let l = checked(format!(
                    "psi-probe: refresh|{}|{}|{}|{}|{:016x}/{}|{values} ms={}",
                    r.table,
                    r.name,
                    look_of(r),
                    r.stem,
                    fnv64(r.src.as_bytes()),
                    r.src.len(),
                    t0.elapsed().as_millis()
                ));
                results.push(l.clone());
                l
            }
            Err(why) => {
                failed += 1;
                checked(format!("psi-probe: refresh-failed|{}|{}|{}|{why}", r.table, r.name, look_of(r)))
            }
        };
        say!(info, "{line}");
        say!(info, "psi-probe: refresh: heap {} B free (largest {} B) after {} {}", heap(), crate::state::largest_free_block(), r.table, r.name);
    }
    say!(info, "psi-probe: refresh rows: {} measured, {failed} failed, of {}", results.len(), ROWS.len());
    say!(info,
        "{DONE}: refresh, main stack {} B unused (was {stack0}), heap {} B free (largest {} B), {} ms in all",
        stack_unused(), heap(), crate::state::largest_free_block(), start.elapsed().as_millis()
    );
    *REPORT.lock().unwrap() = None;
    // (Again, on the console only: a second chance at each line there,
    // spaced out, as the log ring drops lines said in a burst.)
    for l in &results {
        std::thread::sleep(Duration::from_millis(50));
        log::info!("{l}");
    }
}

//! The Psi spike's on-device measurement (`--features psi-probe`, which
//! implies `psi`): run once at boot on the main task — CPU1, the core and
//! the stack the effects render on, with the panel's DMA interrupt live —
//! right after the panel's own bench.
//!
//! For each of the report's three sketches: the compile time and the free
//! heap around the effect's one allocation; the frame time through the
//! real effect path (`Fx::parse` + `Fx::frame` into a `Canvas`, as the
//! engine calls it) and through the VM directly at lane widths 64 and 128,
//! each as p50 / p90 / max over [`FRAMES`] frames; then the golden frames
//! (the same FNV-1a hashes as the host and wasm tests: bit-identical
//! output), a per-step cost table (single-step formulas against a
//! baseline), and the main task's stack high-water mark before and after.
//!
//! Every line starts `psi-probe:`. Between frames it sleeps 1 ms (outside
//! the timed region) so CPU1's idle task feeds the watchdog. Takes ~25 s.
//!
//! **The report on the card.** The probe runs before the card is mounted
//! (nothing else is running yet: the cleanest measurement), so its lines
//! are also kept in memory and written, once the boot task hands the card
//! over and before the player starts, to [`REPORT_NAME`] at the card's root
//! (overwritten every boot) — for a board with no serial console. The name
//! starts `filelist` so the existing read-only `GET /filelist*` route
//! serves it (`http://<board>/filelist-psi-probe.txt`), but not
//! `filelist.`, so nothing takes it for a playlist (the firmware's and the
//! web app's manifest checks are all `filelist.`-prefixed).

use opendmd_core::canvas::Canvas;
use opendmd_core::fx::Fx;
use opendmd_core::fx_psi;
use opendmd_core::psi;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// The report's file name at the card's root.
pub const REPORT_NAME: &str = "filelist-psi-probe.txt";

/// The lines so far, for [`save`].
static REPORT: Mutex<String> = Mutex::new(String::new());

/// Log a line and keep it for the report.
macro_rules! say {
    ($level:ident, $($arg:tt)*) => {{
        let line = format!($($arg)*);
        log::$level!("{line}");
        if let Ok(mut r) = REPORT.lock() {
            r.push_str(&line);
            r.push('\n');
        }
    }};
}

/// Write the report to the card (mounted at `root`), replacing last boot's.
/// Called by the main task once the card is its own.
pub fn save(root: &str) {
    let text = match REPORT.lock() {
        Ok(mut r) => std::mem::take(&mut *r),
        Err(_) => return,
    };
    if text.is_empty() {
        return;
    }
    let path = format!("{root}/{REPORT_NAME}");
    let header = format!(
        "# OpenDMD {} psi-probe report, written {} ms after boot\n",
        crate::state::firmware_version(),
        // SAFETY: a clock read.
        unsafe { esp_idf_sys::esp_timer_get_time() } / 1000
    );
    match std::fs::write(&path, header + &text) {
        Ok(()) => log::info!(
            "psi-probe: report written to /{REPORT_NAME} ({} B): GET /{REPORT_NAME}",
            text.len()
        ),
        Err(e) => log::error!("psi-probe: writing {path} failed: {e}"),
    }
}

/// Frames timed per measurement.
const FRAMES: u32 = 300;

fn stack_unused() -> u32 {
    crate::frametime::stack_unused()
}

fn heap() -> u32 {
    crate::state::free_heap()
}

/// p50 / p90 / max of `us` (sorted in place).
fn pct(us: &mut [u32]) -> (u32, u32, u32) {
    us.sort_unstable();
    let n = us.len();
    (us[n / 2], us[n * 9 / 10], us[n - 1])
}

fn timed(mut f: impl FnMut(u32)) -> (u32, u32, u32) {
    let mut us = Vec::with_capacity(FRAMES as usize);
    for k in 0..FRAMES {
        let t = Instant::now();
        f(k);
        us.push(t.elapsed().as_micros() as u32);
        std::thread::sleep(Duration::from_millis(1));
    }
    pct(&mut us)
}

/// A zeroed `Box<T>` for plain-integer `T` (never on the stack).
fn zeroed<T>() -> Box<T> {
    // SAFETY: only used for psi's `Program` and `Scratch`, which are
    // integers and arrays of integers: all-zero is a valid value.
    unsafe { Box::<T>::new_zeroed().assume_init() }
}

fn lanes<const N: usize>(prog: &psi::Program, loop_ms: u32, out: &mut [u8]) -> (u32, u32, u32) {
    lanes_over::<N>(prog, loop_ms, None, out)
}

/// As [`lanes`], over `under` (the picture a formula look is drawn over).
fn lanes_over<const N: usize>(
    prog: &psi::Program,
    loop_ms: u32,
    under: Option<&[u8]>,
    out: &mut [u8],
) -> (u32, u32, u32) {
    let mut s = zeroed::<psi::Scratch<N>>();
    s.reset();
    let t = fx_psi::tables();
    timed(|k| {
        let inputs = psi::golden::inputs(k * 20 % loop_ms);
        psi::render_rows::<N>(prog, &mut s, &inputs, &t, under, u32::MAX, out);
    })
}

/// The compiler's own stack on the worst inputs (`psi::worst`, the deepest
/// last): each compiled on a thread of its own with a [`WORST_STACK`] stack
/// (an overflow on the main task at boot would be a boot loop), reporting
/// the bytes it used; then, only if that plus what the main task already
/// uses leaves a 2 KB margin, the deepest again on the main task itself.
const WORST_STACK: usize = 40 * 1024;

fn worst_stack() {
    let mut deepest = 0u32;
    // The sources first, on a thread of their own: making some of them
    // compiles deeper and deeper candidates.
    let srcs: Vec<(&str, String)> = std::thread::Builder::new()
        .stack_size(WORST_STACK)
        .spawn(|| psi::worst::ALL.iter().map(|(n, f)| (*n, f())).collect())
        .ok()
        .and_then(|h| h.join().ok())
        .unwrap_or_default();
    for (name, src) in srcs.iter().cloned() {
        let r = std::thread::Builder::new()
            .stack_size(WORST_STACK)
            .spawn(move || {
                let mut prog = zeroed::<psi::Program>();
                let base = stack_unused();
                let r = psi::compile_into(&src, &mut prog);
                (
                    src.len(),
                    base.saturating_sub(stack_unused()),
                    r.err().map(|e| e.msg),
                )
            })
            .map(|h| h.join());
        match r {
            Ok(Ok((len, used, err))) => {
                deepest = deepest.max(used);
                say!(info,
                    "psi-probe: worst {name}: {len} B source, compile used {used} B of a thread's stack ({})",
                    err.as_deref().unwrap_or("ok")
                );
            }
            _ => say!(
                error,
                "psi-probe: worst {name}: no thread (heap {} B free)",
                heap()
            ),
        }
    }
    // The deepest through the real path (`Fx::parse`, as the engine and the
    // checks before saving open a formula): compiled on its own stack
    // (`psi_stack`), so the main task's mark should not move.
    for (name, src) in &srcs {
        let fixed = src.replace("tme", "t");
        let before = stack_unused();
        let bad = Fx::parse(&fx_psi::path(src, "")).is_some();
        let good = Fx::parse(&fx_psi::path(&fixed, "")).is_some();
        say!(info,
            "psi-probe: worst {name} as an effect on the main task: misspelt opened {bad}, fixed opened {good}, main stack {before} -> {} B unused",
            stack_unused()
        );
    }
    let unused = stack_unused();
    if deepest > 0 && deepest + 2048 < unused {
        let mut prog = zeroed::<psi::Program>();
        for (_, src) in &srcs {
            let _ = psi::compile_into(src, &mut prog);
        }
        say!(info,
            "psi-probe: worst on the main task: {unused} B unused before, {} B after (deepest thread figure {deepest} B)",
            stack_unused()
        );
    } else {
        say!(warn,
            "psi-probe: worst on the main task: skipped, {unused} B unused and the deepest compile takes {deepest} B"
        );
    }
}

pub fn run() {
    let start = Instant::now();
    let stack0 = stack_unused();
    say!(info,
        "psi-probe: start: main stack {stack0} B unused, heap {} B free, Program {} B, Scratch<128> {} B, Scratch<64> {} B",
        heap(),
        std::mem::size_of::<psi::Program>(),
        std::mem::size_of::<psi::Scratch<128>>(),
        std::mem::size_of::<psi::Scratch<64>>()
    );
    let mut out = vec![0u8; psi::W * psi::H * 3];
    let mut canvas = Canvas::new();

    for (name, src) in psi::sketches::ALL {
        // Compile (into a zeroed program, as the effect does).
        let mut prog = zeroed::<psi::Program>();
        let t0 = Instant::now();
        let r = psi::compile_into(src, &mut prog);
        let compile_us = t0.elapsed().as_micros();
        if let Err(e) = r {
            say!(error, "psi-probe: {name}: {}", e.msg);
            continue;
        }
        let (f, row, px) = prog.counts();
        // The effect, as the engine opens and draws it.
        let path = fx_psi::path(src, "");
        let h0 = heap();
        let t0 = Instant::now();
        let fx = Fx::parse(&path);
        let open_us = t0.elapsed().as_micros();
        let h1 = heap();
        let Some(mut fx) = fx else {
            say!(error, "psi-probe: {name}: the path did not parse");
            continue;
        };
        let delay = fx.delay();
        let n = fx.loop_frames();
        let (e50, e90, emax) = timed(|k| {
            fx.frame(delay * (k % n), &mut canvas);
        });
        drop(fx);
        let (a50, a90, amax) = lanes::<128>(&prog, prog.loop_ms(), &mut out);
        let (b50, b90, bmax) = lanes::<64>(&prog, prog.loop_ms(), &mut out);
        say!(info,
            "psi-probe: {name}: steps {} ({f} frame / {row} row / {px} pixel), regs {}, compile {compile_us} us, open {open_us} us, heap {h0} -> {h1} B ({} B for the effect), path {} B",
            prog.len(),
            prog.regs(),
            h0.saturating_sub(h1),
            path.len()
        );
        say!(info,
            "psi-probe: {name}: effect frame p50 {e50} p90 {e90} max {emax} us | vm N=128 p50 {a50} p90 {a90} max {amax} us | vm N=64 p50 {b50} p90 {b90} max {bmax} us"
        );
    }

    // Golden frames: the host's and wasm's hashes, here.
    let t = fx_psi::tables();
    let mut s = zeroed::<psi::Scratch<128>>();
    s.reset();
    let mut prog = zeroed::<psi::Program>();
    let (mut ok, mut bad) = (0, 0);
    for &(name, src, ms, want) in psi::golden::FRAMES {
        if psi::compile_into(src, &mut prog).is_err() {
            bad += 1;
            say!(error, "psi-probe: golden {name}: does not compile");
            continue;
        }
        psi::render_with::<128>(&prog, &mut s, &psi::golden::inputs(ms), &t, &mut out);
        let got = psi::golden::fnv(&out);
        if got == want {
            ok += 1;
        } else {
            bad += 1;
            say!(
                error,
                "psi-probe: golden {name} at {ms} ms: {got:#018x}, host/wasm {want:#018x}"
            );
        }
    }
    // Layers, and formulas over a picture (the formula look's path).
    let mut under = vec![0u8; psi::W * psi::H * 3];
    psi::golden::picture(&mut under);
    for &(name, src, ms, over, want) in psi::golden::OVER {
        if psi::compile_into(src, &mut prog).is_err() {
            bad += 1;
            say!(error, "psi-probe: golden {name}: does not compile");
            continue;
        }
        let u = over.then_some(under.as_slice());
        psi::render_rows::<128>(&prog, &mut s, &psi::golden::inputs(ms), &t, u, u32::MAX, &mut out);
        let got = psi::golden::fnv(&out);
        if got == want {
            ok += 1;
        } else {
            bad += 1;
            say!(
                error,
                "psi-probe: golden {name} (over) at {ms} ms: {got:#018x}, host/wasm {want:#018x}"
            );
        }
    }
    say!(
        info,
        "psi-probe: golden frames: {ok} match the host/wasm hashes, {bad} differ{}",
        if bad == 0 { " (bit-identical)" } else { "" }
    );

    // What one step costs: each formula is the baseline plus one or two
    // per-pixel steps (t is a frame value, so nothing folds away).
    const STEPS: [(&str, &str); 14] = [
        ("base gray(x)", "gray(x)"),
        ("add", "gray(x + t)"),
        ("mul", "gray(x * t)"),
        ("div (+add)", "gray(x / (x + t))"),
        ("mod 0.3", "gray(x % 0.3)"),
        ("sin (+add)", "gray(sin(x + t))"),
        ("sqrt (+add)", "gray(sqrt(x + t))"),
        ("len (+add)", "gray(len(x, x + t))"),
        ("atan2 (+sub)", "gray(atan2(y - 0.5, x - t))"),
        ("smoothstep (+add)", "gray(smoothstep(0.2, 0.8, x + t))"),
        ("noise (+mul)", "gray(noise(x * 8, y, t))"),
        ("select (+lt)", "gray(x < t ? x : 0.5)"),
        ("out hsv", "hsv(x, 1, 1)"),
        ("out pal", "pal(fire, x)"),
    ];
    // Layers and what is under (drawn over the golden picture): the
    // formula look's costs, for `Program::esp32_us`.
    const LAYER_STEPS: [(&str, &str); 10] = [
        ("ur", "gray(ur)"),
        ("ul", "gray(ul)"),
        ("shows through", "gray(x, 0.5)"),
        ("+layer over 0.5", "layer { gray(x) }\nlayer { gray(x + t, 0.5) }"),
        ("+layer add", "layer { gray(x) }\nlayer add { gray(x + t) }"),
        ("+layer multiply", "layer { gray(x) }\nlayer multiply { gray(x + t) }"),
        ("+layer screen", "layer { gray(x) }\nlayer screen { gray(x + t) }"),
        ("+layer max", "layer { gray(x) }\nlayer max { gray(x + t) }"),
        ("look scanlines", "layer multiply { gray(py % 2 ? 1 : 0.55) }"),
        ("look 4 layers", "layer multiply { gray(y) }\nlayer add { gray(x) }\nlayer screen { gray(x + t) }\nlayer max { gray(py / 32) }"),
    ];
    let steps = STEPS
        .iter()
        .map(|&(w, s)| (w, s, false))
        .chain(LAYER_STEPS.iter().map(|&(w, s)| (w, s, true)));
    for (what, src, over) in steps {
        if psi::compile_into(src, &mut prog).is_err() {
            say!(error, "psi-probe: step {what}: does not compile");
            continue;
        }
        let u = over.then_some(under.as_slice());
        let (p50, p90, _) = lanes_over::<128>(&prog, 8000, u, &mut out);
        let (_, row, px) = prog.counts();
        say!(info,
            "psi-probe: step {what:<18} pixel steps {px} (+{row} row): p50 {p50} p90 {p90} us a frame = {} ns a pixel",
            p50 as u64 * 1000 / 4096
        );
    }

    // The formula look as the main loop runs it (`FormulaLook::apply` over
    // a picture, every row: a frame of its own), timed, with the heap its
    // buffers take and the stack after (the high-water mark).
    const LOOKS: [(&str, &str); 3] = [
        ("Scanlines", "layer multiply { gray(py % 2 ? 1 : 0.3 + a*0.6) }"),
        ("Old TV", "loop 4\ndx = (x - 0.5)*1.6; dy = (y - 0.5)*1.2\nlines = py % 2 ? 1 : 0.6\nroll = 0.85 + 0.15*max(0, tri(y/2 + t/4))\ncorners = clamp(1.5 - (dx*dx + dy*dy)*1.6, 0, 1)\nlayer multiply { gray(lines * roll * corners) }"),
        ("Corner clock", "frame { hr = 104 + (hour % 12 + minute/60)/12*22; mn = 104 + minute/60*22 }\ntop = py > 3 and py < 6; bot = py > 0 and py < 3\nbar = (top or bot) * (px >= 104) * (px < (top ? hr : mn))\nlayer multiply { gray(1 - (px >= 101) * (py < 8) * 0.8) }\nlayer { rgb(1, 0.75, 0.3, bar) }"),

    ];
    let mut pic = Canvas::new();
    pic.as_mut_bytes().copy_from_slice(&under);
    for (name, src) in LOOKS {
        let Ok(mut look) = fx_psi::FormulaLook::from_path(&fx_psi::path(src, "a=0.4")) else {
            say!(error, "psi-probe: look {name}: refused");
            continue;
        };
        let h0 = heap();
        let took = look.take();
        let h1 = heap();
        let mut k = 0u32;
        let (p50, p90, max) = timed(|_| {
            look.redraw();
            pic.dirty_mut().mark_all();
            k += 1;
            let out = look.apply(Duration::from_millis(u64::from(k) * 20), &mut pic, 0);
            out.dirty_mut().clear();
        });
        say!(info,
            "psi-probe: look {name}: est {} us, apply every row p50 {p50} p90 {p90} max {max} us, buffers {} ({h0} -> {h1} B free), main stack {} B unused",
            look.program().esp32_us(),
            if took { "taken" } else { "REFUSED" },
            stack_unused()
        );
    }

    worst_stack();
    let stack1 = stack_unused();
    say!(info,
        "psi-probe: done: main stack {stack1} B unused (was {stack0} before the probe; the high-water mark is the deepest so far), heap {} B free, {} ms in all",
        heap(),
        start.elapsed().as_millis()
    );
}

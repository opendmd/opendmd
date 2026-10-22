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
    let mut s = zeroed::<psi::Scratch<N>>();
    s.reset();
    let t = fx_psi::tables();
    timed(|k| {
        let inputs = psi::golden::inputs(k * 20 % loop_ms);
        psi::render_with::<N>(prog, &mut s, &inputs, &t, out);
    })
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
    for (what, src) in STEPS {
        if psi::compile_into(src, &mut prog).is_err() {
            say!(error, "psi-probe: step {what}: does not compile");
            continue;
        }
        let (p50, p90, _) = lanes::<128>(&prog, 8000, &mut out);
        let (_, row, px) = prog.counts();
        say!(info,
            "psi-probe: step {what:<18} pixel steps {px} (+{row} row): p50 {p50} p90 {p90} us a frame = {} ns a pixel",
            p50 as u64 * 1000 / 4096
        );
    }

    let stack1 = stack_unused();
    say!(info,
        "psi-probe: done: main stack {stack1} B unused (was {stack0} before the probe; the high-water mark is the deepest so far), heap {} B free, {} ms in all",
        heap(),
        start.elapsed().as_millis()
    );
}

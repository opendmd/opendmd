//! The Psi spike's on-device measurement (`--features psi-probe`, which
//! implies `psi`): run once a boot on the main task — CPU1, the core and
//! the stack the effects render on, with the panel's DMA interrupt live —
//! when the boot task hands the card over: the httpd and the network are
//! up by then, so a board running it stays reachable (and can be updated
//! back over WiFi) whatever a step does.
//!
//! **Crash-safe** (2026-10-26, after a probe build that never came up on
//! the rig: it ran before WiFi, so nothing could reach it): every line is
//! appended to [`REPORT_NAME`] on the card as it is said, the first line
//! written before any measurement; a report there without its `done` line
//! is a probe that did not finish (a panic, an overflow, out of memory),
//! and the next boot does not run it again — the player starts and the
//! partial report says how far it got. Delete the file to probe again. The
//! compiler's worst cases come first (what the probe is for); each is
//! compiled on a thread of [`WORST_STACK`] and only then, if what it used
//! leaves [`PSI_MARGIN`] of `config::PSI_COMPILE_STACK`, on the player's
//! own compile stack (`psi_stack`) and through `Fx::parse`.
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
//! **The report on the card**, [`REPORT_NAME`] at its root, for a board
//! with no serial console. The name
//! starts `filelist` so the existing read-only `GET /filelist*` route
//! serves it (`http://<board>/filelist-psi-probe.txt`), but not
//! `filelist.`, so nothing takes it for a playlist (the firmware's and the
//! web app's manifest checks are all `filelist.`-prefixed).

use opendmd_core::canvas::Canvas;
use opendmd_core::fx::Fx;
use opendmd_core::fx_psi;
use opendmd_core::psi;
use std::sync::atomic::Ordering;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Every bundled Formula (`-- bundle`), by file stem, comments left out (the
/// play path compacts them anyway): one copy of each in the image, which the
/// rows below look up rather than `include_str!` again. (Time-gate and
/// Endless road are the roadprobe's pair; Endless road is also the
/// psi-lang case for `loop forever`, a sprite and slot reuse.)
use crate::formulas::BUNDLED_SOURCES as BUNDLED;

/// The examples that are not bundled (the drawing and plain ones the speed2
/// and leak rows compile), likewise comment-free, from build.rs.
const EXAMPLES: &[(&str, &str)] = include!(concat!(env!("OUT_DIR"), "/probe_examples.rs"));

fn bundled_src(stem: &str) -> &'static str {
    BUNDLED.iter().chain(EXAMPLES).find(|(n, _)| *n == stem).map_or("", |(_, s)| s)
}

/// Endless road as bundled (since road-life written tight: the matches
/// below follow its spelling), without its
/// row noises, and without those, the draw stage, the palette and pick
/// (the rows' floor). roadprobe-a..c also timed the sprite car's version.
fn road_variants() -> Vec<(&'static str, String)> {
    // (As build.rs keeps it: `opendmd_psi::squeeze`d.)
    let src = bundled_src("endless-road");
    let rows = src.find("below=py<19").unwrap_or(0);
    let draw = src.find("draw{").unwrap_or(rows);
    let no_noise = |s: &str| s.replace("noise(rowDriven,5,0)", "height").replace("noise(rowDriven,0,0)", "heading");
    let no_draw = |s: &str| format!("{}{}", &s[..draw], s[rows..].replace("ink(16)", "0"));
    // (The pick's first value is written in place: up to the road's arm,
    // `,\nroadInk-stripe`.)
    const PICK: &str = "pal(road,pick(";
    let gray = |s: &str| match (s.find(PICK), s.find(",\nroadInk-stripe")) {
        (Some(i), Some(j)) if i + PICK.len() <= j => format!("{}gray(({})/6)\n", &s[..i], &s[i + PICK.len()..j]),
        _ => s.to_string(),
    };
    vec![
        ("Endless road (bundled)", src.to_string()),
        ("road: no row noise", no_noise(src)),
        ("road: floor (no noise, draw, pal or pick)", gray(&no_noise(&no_draw(src)))),
    ]
}

/// The player's own frames of a Formula, as the main loop times them
/// (roadprobe): where a 10 ms frame's time goes on the panel, which the
/// boot-time rows cannot see (the pacer's waits, the loop's own work, the
/// wake). The main loop calls [`play::note`] for each frame of a Formula on
/// its schedule; a CPU0 task appends a line a [`play::EVERY`] to the report.
pub mod play {
    use std::sync::atomic::{AtomicU32, Ordering::Relaxed};
    use std::time::Duration;

    /// The report line's period.
    pub const EVERY: Duration = Duration::from_secs(10);
    /// How long the lines go on after the probe (then the task ends).
    const FOR: Duration = Duration::from_secs(40 * 60);

    /// Phases: wake (the sleep ending late), other (before the tick), start
    /// (the tick's start past the frame's due time), tick (the engine: the
    /// render), blit (less its wait), wait (the pacer), post (the blit's end
    /// to the judgment); and px, the pixels the blit pushed (consolea).
    pub const PHASES: [&str; 8] = ["wake", "other", "start", "tick", "blit", "wait", "post", "px"];
    /// Work (start + tick + blit + wait + post) at or over these µs.
    const OVER: [u32; 5] = [8_000, 9_000, 10_000, 11_000, 12_000];

    static N: AtomicU32 = AtomicU32::new(0);
    static SUM: [AtomicU32; 8] = [const { AtomicU32::new(0) }; 8];
    static MAX: [AtomicU32; 8] = [const { AtomicU32::new(0) }; 8];
    static MISSES: AtomicU32 = AtomicU32::new(0);
    static OVERS: [AtomicU32; 5] = [const { AtomicU32::new(0) }; 5];

    /// One Formula frame (main task only), or a frame under a formula look.
    pub fn note(p: [u32; 8], missed: bool) {
        N.fetch_add(1, Relaxed);
        for (i, v) in p.iter().enumerate() {
            SUM[i].fetch_add(*v, Relaxed);
            MAX[i].fetch_max(*v, Relaxed);
        }
        MISSES.fetch_add(u32::from(missed), Relaxed);
        let work = p[2] + p[3] + p[4] + p[5] + p[6];
        for (c, o) in OVERS.iter().zip(OVER) {
            c.fetch_add(u32::from(work >= o), Relaxed);
        }
    }

    fn line() -> Option<String> {
        let n = N.swap(0, Relaxed);
        let sums: [u32; 8] = core::array::from_fn(|i| SUM[i].swap(0, Relaxed));
        let maxes: [u32; 8] = core::array::from_fn(|i| MAX[i].swap(0, Relaxed));
        let missed = MISSES.swap(0, Relaxed);
        let o: [u32; 5] = core::array::from_fn(|i| OVERS[i].swap(0, Relaxed));
        if n == 0 {
            return None;
        }
        let (us, ms, asked, _) = opendmd_core::fx_psi::stats();
        // (consolea: the window's frames ran as the switch was through
        // it; it flips after each line.)
        let all = opendmd_core::fx_psi::OLD.load(Relaxed);
        let (jit, why, _) = opendmd_core::fx_psi::jit_status();
        let jit = if jit { "kernel".to_string() } else { format!("vm: {why}") };
        let mut s = format!(
            "psi-probe: play {} s [{}]: {n} formula frames, misses {missed}, governor {us} us mean -> {ms} ms frames (asked {asked}), {jit}; mean/max us:",
            // SAFETY: a clock read.
            unsafe { esp_idf_sys::esp_timer_get_time() } / 1_000_000,
            if all { "as before" } else { "phase A" },
        );
        for (i, name) in PHASES.iter().enumerate() {
            s.push_str(&format!(" {name} {}/{}", sums[i] / n, maxes[i]));
        }
        s.push_str(&format!(
            "; work >=8 ms {} >=9 {} >=10 {} >=11 {} >=12 {}",
            o[0], o[1], o[2], o[3], o[4]
        ));
        // Racing the beam (console-c): the raced frame time and the
        // counters since boot (`scan.beam`).
        s.push_str(&format!("; beam {} us {}", opendmd_core::fx_psi::beam_us(), crate::panel::beam_json()));
        Some(s)
    }

    /// Flip `fx_psi::OLD` after each line (consolea).
    pub static AB: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

    /// Append a line every [`EVERY`] for [`FOR`] (a CPU0 task).
    pub fn start(path: String) {
        let r = crate::task::spawn_core0(
            b"psi_play\0",
            6 * 1024,
            1,
            Box::new(move || {
                let t0 = std::time::Instant::now();
                while t0.elapsed() < FOR {
                    std::thread::sleep(EVERY);
                    let Some(l) = line() else { continue };
                    // consolea's A/B: every other window runs a Formula's
                    // frames as before Phase A (`fx_psi::OLD`).
                    if AB.load(Relaxed) {
                        let all = &opendmd_core::fx_psi::OLD;
                        all.store(!all.load(Relaxed), Relaxed);
                    }
                    log::info!("{l}");
                    if let Ok(mut f) = std::fs::OpenOptions::new().append(true).open(&path) {
                        use std::io::Write;
                        let _ = writeln!(f, "{l}");
                    }
                }
            }),
        );
        if let Err(e) = r {
            log::error!("psi-probe: play lines: no task ({e})");
        }
    }
}

/// The report's file name at the card's root.
pub const REPORT_NAME: &str = "filelist-psi-probe.txt";

/// The report, open for appending while the probe runs.
static REPORT: Mutex<Option<std::fs::File>> = Mutex::new(None);

/// Log a line and append it to the report on the card at once (synced:
/// a crash after it leaves it there).
macro_rules! say {
    (error, $($arg:tt)*) => {
        $crate::psi_probe::report_line(log::Level::Error, format!($($arg)*))
    };
    (warn, $($arg:tt)*) => {
        $crate::psi_probe::report_line(log::Level::Warn, format!($($arg)*))
    };
    ($level:ident, $($arg:tt)*) => {
        $crate::psi_probe::report_line(log::Level::Info, format!($($arg)*))
    };
}

/// A report line: logged, and appended to the report on the card at once
/// (synced). Out of line: `say!` at every row inlined all of this (the
/// full probe no longer fitted the 4mb partition).
#[inline(never)]
pub(crate) fn report_line(level: log::Level, line: String) {
    log::log!(level, "{line}");
    if let Ok(mut r) = REPORT.lock() {
        if let Some(f) = r.as_mut() {
            use std::io::Write;
            let _ = writeln!(f, "{line}");
            let _ = f.sync_data();
        }
    }
}

/// Only the road's rows, then the play lines (roadprobe-b/c: a's full
/// probe ran out of heap before them). `false`: the full probe, which
/// starts the play lines when it is done.
const ROAD_ONLY: bool = false;

/// Only the formula-over-formula sections (`swap`, `swap_card`).
const SWAP_ONLY: bool = false;

/// Only the JIT spike's rows (jitspike-b: a's full probe ran out of heap
/// in the looks, 225 s in, and the spike's own rows were done long before).
const JIT_ONLY: bool = false;

/// Only the JIT's rows (`jit2::probe`: the goldens through kernels, the
/// examples as effects and the looks JIT against VM), then the play lines. The full probe runs out of heap; these run first and alone.
const JIT2_ONLY: bool = false;

/// Endless road and its variants one at a time (each source made, run and
/// dropped before the next), splits, then Starfield and Time gate as
/// references; the play lines start first.
fn road_probe(root: &str, start: Instant, stack0: u32) {
    play::start(format!("{root}/{REPORT_NAME}"));
    say!(info, "psi-probe: roadprobe: heap {} B free, largest block {} B, main stack {stack0} B unused",
        heap(), crate::state::largest_free_block());
    psi::stats::CLOCK.store(ccount as fn() -> u32 as usize, Ordering::Relaxed);
    fx_psi::set_still_gate(Some(crate::still_gate));
    let mut canvas = Canvas::new();
    let n = road_variants().len();
    for k in 0..n {
        let (name, src) = road_variants().swap_remove(k);
        effects(&[(name, src.as_str(), "")], "", &mut canvas);
        fx_psi::set_still_gate(Some(crate::still_gate));
    }
    split("Endless road (bundled)", bundled_src("endless-road"), true, &mut canvas);
    effects(&[("Starfield", bundled_src("starfield"), "a=0.5"), ("Time gate (bundled)", bundled_src("time-gate"), "")], "", &mut canvas);
    fx_psi::set_still_gate(Some(crate::still_gate));
    split("Time gate (bundled)", bundled_src("time-gate"), true, &mut canvas);
    fx_psi::set_still_gate(Some(crate::still_gate));
    say!(info,
        "{DONE}: roadprobe, main stack {} B unused (was {stack0}), heap {} B free (largest {} B), {} ms in all",
        stack_unused(), heap(), crate::state::largest_free_block(), start.elapsed().as_millis()
    );
    *REPORT.lock().unwrap() = None;
}

/// fast-div (fastdiv-a before, -b after): only its rows, first, one
/// source at a time (the full probe has run out of heap twice), then the
/// play lines. `false`: the full probe (or the others above).
const FASTDIV_ONLY: bool = false;

const TUNNEL_FILE: &str = include_str!("../../opendmd-web/formulas/tunnel.psi");

// The definitions of `div`, `recip`, `sqrt`, `len` and `atan2` as they were
// before fast-div (64-bit divisions and shifts, `leading_zeros`), for the
// device's own check of psi's (the same code on fastdiv-a).

fn sat64(v: i64) -> i32 {
    v.clamp(i32::MIN as i64, i32::MAX as i64) as i32
}

fn div_ref(a: i32, b: i32) -> i32 {
    if b == 0 {
        return match a {
            0 => 0,
            a if a > 0 => i32::MAX,
            _ => i32::MIN,
        };
    }
    sat64(((a as i64) << 16) / b as i64)
}

fn sqrt_q32_ref(v: u64) -> i32 {
    if v == 0 {
        return 0;
    }
    let s = v.leading_zeros() as i32 & !1;
    let w = ((v << s) >> 34) as u32;
    let i = (w >> 22) as usize - 64;
    let f = (w >> 6) & 0xffff;
    let (a, b) = (psi::SQRT[i] as u32, psi::SQRT[(i + 1).min(192)] as u32);
    let r = (a + (((b - a) * f) >> 16)) as u64;
    let sh = 17 - s / 2;
    sat64((if sh >= 0 { r << sh } else { r >> -sh }) as i64)
}

fn sqrt_ref(a: i32) -> i32 {
    if a <= 0 {
        0
    } else {
        sqrt_q32_ref((a as u64) << 16)
    }
}

fn len_ref(a: i32, b: i32) -> i32 {
    let (a, b) = (a.unsigned_abs() as u64, b.unsigned_abs() as u64);
    sqrt_q32_ref((a * a).saturating_add(b * b))
}

fn atan2_ref(y: i32, x: i32) -> i32 {
    let (ax, ay) = (x.unsigned_abs(), y.unsigned_abs());
    let (mut lo, mut hi) = if ay > ax { (ax, ay) } else { (ay, ax) };
    if hi == 0 {
        return 0;
    }
    let bits = 32 - hi.leading_zeros();
    if bits > 15 {
        lo >>= bits - 15;
        hi >>= bits - 15;
        if hi == 0 {
            return 0;
        }
    }
    let r = (lo << 16) / hi;
    let (i, f) = ((r >> 8) as usize, (r & 255) as i32);
    let (t0, t1) = (psi::ATAN[i] as i32, psi::ATAN[(i + 1).min(256)] as i32);
    let mut ang = t0 + (((t1 - t0) * f) >> 8);
    if ay > ax {
        ang = (1 << 14) - ang;
    }
    if x < 0 {
        ang = (1 << 15) - ang;
    }
    if y < 0 {
        -ang
    } else {
        ang
    }
}

/// A value of any magnitude and either sign (xorshift32), or every few a
/// pixel-sized one (0..±200 in Q16.16).
fn fastdiv_any(seed: &mut u32) -> i32 {
    let mut next = || {
        *seed ^= *seed << 13;
        *seed ^= *seed >> 17;
        *seed ^= *seed << 5;
        *seed
    };
    let (v, k) = (next(), next());
    let m = match k % 4 {
        0 => (v % (200 << 16)) as i32,
        _ => (v >> ((k >> 8) % 32)) as i32,
    };
    if k & 0x8000 == 0 {
        m
    } else {
        m.wrapping_neg()
    }
}

/// psi's `div`, `recip`, `sqrt`, `len`, `atan2` against the definitions on
/// the device itself (the ESP32's own `quou`, `nsau` and ROM 64-bit
/// division): every power of two and its neighbours, then random inputs.
fn fastdiv_check() {
    let mut edges = vec![0i32, i32::MIN, i32::MIN + 1, i32::MAX, i32::MAX - 1];
    for k in 0..31 {
        for d in -2i32..=2 {
            let v = (1i32 << k).wrapping_add(d);
            edges.extend([v, v.wrapping_neg()]);
        }
    }
    let (mut n, mut bad) = (0u64, 0u64);
    let mut check = |a: i32, b: i32| {
        let pairs = [
            ("div", psi::fixed::div(a, b), div_ref(a, b)),
            ("recip", psi::fixed::recip(a), div_ref(1 << 16, a)),
            ("sqrt", psi::fixed::sqrt(a), sqrt_ref(a)),
            ("len", psi::fixed::len(a, b), len_ref(a, b)),
            ("atan2", psi::fixed::atan2(a, b), atan2_ref(a, b)),
        ];
        for (what, got, want) in pairs {
            n += 1;
            if got != want {
                bad += 1;
                if bad <= 20 {
                    say!(error, "psi-probe: fastdiv check {what}({a}, {b}): {got}, the definition {want}");
                }
            }
        }
    };
    for &a in &edges {
        for &b in &edges {
            check(a, b);
        }
    }
    let t0 = Instant::now();
    let mut seed = 0x2545_f491u32;
    for k in 0..400_000u32 {
        let (a, b) = (fastdiv_any(&mut seed), fastdiv_any(&mut seed));
        check(a, b);
        if k % 20_000 == 0 {
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    say!(info,
        "psi-probe: fastdiv check: {n} results of div/recip/sqrt/len/atan2 against the definitions, {bad} differ{} ({} ms)",
        if bad == 0 { " (bit-identical)" } else { "" },
        t0.elapsed().as_millis()
    );
}

/// Cycles a call over 4,096 inputs (best of 5 passes), psi's function
/// against the definition's copy here (both in flash, inlined in the same
/// loop shape): what each costs alone, outside the VM's lanes.
fn fastdiv_micro() {
    let mut seed = 0x9e37_79b9u32;
    let n = 4096usize;
    // Pixel-like inputs: values 0..±200, and the panel's distances.
    let xs: Vec<i32> = (0..n).map(|_| fastdiv_any(&mut seed)).collect();
    let ys: Vec<i32> = (0..n)
        .map(|k| (((k % 128) as i32 - 64) << 16) + (fastdiv_any(&mut seed) & 0xffff))
        .collect();
    let zs: Vec<i32> = (0..n).map(|k| ((k / 128) as i32 - 16) << 16 | 0x8000).collect();
    // Distances from the panel's middle (Tunnel's `len`), never 0.
    let ds: Vec<i32> = (0..n).map(|k| len_ref(ys[k], zs[k])).collect();
    let mut out = vec![0i32; n];
    let mut time = |f: &dyn Fn(i32, i32) -> i32, a: &[i32], b: &[i32]| -> f64 {
        let mut best = u32::MAX;
        for _ in 0..5 {
            let t0 = ccount();
            for k in 0..n {
                out[k] = f(core::hint::black_box(a[k]), b[k]);
            }
            best = best.min(ccount().wrapping_sub(t0));
            core::hint::black_box(&out);
            std::thread::sleep(Duration::from_millis(1));
        }
        f64::from(best) / n as f64
    };
    type F<'a> = &'a dyn Fn(i32, i32) -> i32;
    type Row<'a> = (&'a str, F<'a>, F<'a>, &'a [i32], &'a [i32]);
    let rows: [Row; 8] = [
        ("div x/y (any)", &psi::fixed::div, &div_ref, &xs, &zs),
        ("div 128/len", &|_, d| psi::fixed::div(128 << 16, d), &|_, d| div_ref(128 << 16, d), &xs, &ds),
        ("div 1/v", &|a, _| psi::fixed::div(1 << 16, a), &|a, _| div_ref(1 << 16, a), &ys, &zs),
        ("recip", &|a, _| psi::fixed::recip(a), &|a, _| div_ref(1 << 16, a), &ys, &zs),
        ("sqrt", &|a, _| psi::fixed::sqrt(a), &|a, _| sqrt_ref(a), &ds, &zs),
        ("len (panel)", &psi::fixed::len, &len_ref, &ys, &zs),
        ("atan2 (panel)", &|a, b| psi::fixed::atan2(b, a), &|a, b| atan2_ref(b, a), &ys, &zs),
        ("mod (control)", &psi::fixed::modulo, &|a, b| psi::fixed::modulo(a, b), &xs, &zs),
    ];
    for (what, new, old, a, b) in rows {
        let c_new = time(new, a, b);
        let c_old = time(old, a, b);
        say!(info,
            "psi-probe: fastdiv micro {what:<14}: psi {c_new:.1} cycles a call ({:.0} ns), the definition {c_old:.1} ({:.0} ns)",
            c_new / 0.24,
            c_old / 0.24
        );
    }
}

/// fast-div's rows: the device's exactness check, the functions alone,
/// single-step formulas (each over a baseline row; fastdiv-a against -b is
/// the step's saving), Tunnel and Time gate as effects with the still
/// cache and with the gate refusing it, and the golden frames.
fn fastdiv_probe(start: Instant, stack0: u32) {
    say!(info, "psi-probe: fastdiv: heap {} B free, largest block {} B, main stack {stack0} B unused",
        heap(), crate::state::largest_free_block());
    psi::stats::CLOCK.store(ccount as fn() -> u32 as usize, Ordering::Relaxed);
    // The effects first, while the heap is at its fullest (the still cache
    // wants a 16-32 KB block).
    let mut canvas = Canvas::new();
    for (name, src) in [("Tunnel (bundled)", TUNNEL_FILE), ("Time gate (bundled)", bundled_src("time-gate"))] {
        effects(&[(name, src, "")], "", &mut canvas);
        fx_psi::set_still_gate(Some(crate::still_gate));
    }
    drop(canvas);
    fastdiv_check();
    fastdiv_micro();
    // Single steps, laid out without the column and still runs and checked
    // (as the step table was measured).
    const ROWS: [(&str, &str); 13] = [
        ("base gray(x)", "gray(x)"),
        ("add", "gray(x + t)"),
        ("mul", "gray(x * t)"),
        ("mod 0.3", "gray(x % 0.3)"),
        ("div (+add)", "gray(x / (x + t))"),
        ("div 1/v (+add)", "gray(1 / (px + t))"),
        ("div 128/v (+add)", "gray(128 / (px + t))"),
        ("sqrt (+add)", "gray(sqrt(x + t))"),
        ("sqrt px (+add)", "gray(sqrt(px + t) / 12)"),
        ("len (+add)", "gray(len(x, x + t))"),
        ("len px (+2)", "gray(len(px - 63.5, py + t) / 70)"),
        ("atan2 (+sub)", "gray(atan2(y - 0.5, x - t))"),
        ("atan2 px (+2)", "gray(atan2(py - 15.5, px - 63.5 + t))"),
    ];
    let mut out = vec![0u8; psi::W * psi::H * 3];
    let mut prog = zeroed::<psi::Program>();
    for (what, src) in ROWS {
        if compile(src, &mut prog, false, false).is_err() {
            say!(error, "psi-probe: fastdiv step {what}: does not compile");
            continue;
        }
        let (p50, p90, _) = lanes_over::<128>(&prog, 8000, None, &mut out);
        let (_, row, px) = prog.counts();
        say!(info,
            "psi-probe: fastdiv step {what:<18} pixel steps {px} (+{row} row): p50 {p50} p90 {p90} us a frame = {} ns a pixel, est {} us",
            p50 as u64 * 1000 / 4096,
            prog.esp32_us()
        );
    }
    // The golden frames: the host's and wasm's hashes, here.
    let t = fx_psi::tables();
    let mut s = zeroed::<psi::Scratch<128>>();
    s.reset();
    let (mut ok, mut bad) = (0, 0);
    for &(name, src, ms, want) in psi::golden::FRAMES {
        if compile(src, &mut prog, true, true).is_err() {
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
            say!(error, "psi-probe: golden {name} at {ms} ms: {got:#018x}, host/wasm {want:#018x}");
        }
    }
    say!(info,
        "psi-probe: golden frames: {ok} match the host/wasm hashes, {bad} differ{}",
        if bad == 0 { " (bit-identical)" } else { "" }
    );
    say!(info,
        "{DONE}: fastdiv, main stack {} B unused (was {stack0}), heap {} B free (largest {} B), {} ms in all",
        stack_unused(), heap(), crate::state::largest_free_block(), start.elapsed().as_millis()
    );
    *REPORT.lock().unwrap() = None;
}

/// console-lessons Phase A (consolea): only its rows, first, one source at
/// a time (the full probe has run out of heap), then the play lines.
/// `false`: the full probe (or `ROAD_ONLY`).
const CONSOLE_ONLY: bool = false;

const STARFIELD_FILE: &str = include_str!("../../opendmd-web/formulas/starfield.psi");
const BIG_CLOCK_FILE: &str = include_str!("../../opendmd-web/formulas/big-clock.psi");
const GLITTER_FILE: &str = include_str!("../../opendmd-web/formulas/glitter.psi");
const ROTOZOOM_FLOOR_FILE: &str = include_str!("../../opendmd-web/formulas/rotozoom-floor.psi");
const SCANLINES_FILE: &str = include_str!("../../opendmd-web/formulas/scanlines.psi");

/// One Formula as an effect on the real path, a way at a time (each made,
/// run and dropped before the next): Phase A as shipped, then as before it
/// (`fx_psi::OLD`: no compare, every pixel marked); each line the frame's
/// p50/p90/max, the pixels marked for the blit (of 4,096), the still cache
/// and the heap. A Formula with still values says what the firmware's
/// still gate says first, and why.
fn console_row(name: &str, src: &str, opts: &str, canvas: &mut Canvas) {
    let mut prog = zeroed::<psi::Program>();
    let (est, still) = match compile(src, &mut prog, true, true) {
        Ok(_) => (prog.esp32_us_still(), psi::Still::bytes_for(&prog)),
        Err(e) => {
            say!(error, "psi-probe: consolea {name}: does not compile: {e:?}");
            return;
        }
    };
    drop(prog);
    if still > 0 {
        say!(info,
            "psi-probe: consolea {name}: still gate for {still} B says {}: heap {} B free (needs {}), largest block {} B, look source {}, card builder {}",
            crate::still_gate(still),
            heap(),
            still + crate::config::STILL_KEEP_HEAP,
            crate::state::largest_free_block(),
            crate::formula_look::SOURCE.load(Ordering::Relaxed),
            if crate::cache_builder::holding() { "mid-file" } else { "idle" }
        );
    }
    for (way, old) in [("phase A", false), ("as before", true)] {
        fx_psi::OLD.store(old, Ordering::Relaxed);
        let h0 = heap();
        let Some(mut fx) = Fx::parse(&fx_psi::path(src, opts)) else {
            say!(error, "psi-probe: consolea {name}: did not open");
            continue;
        };
        let delay = fx.delay();
        let n = fx.loop_frames();
        fx.frame(Duration::ZERO, canvas);
        let held = fx_psi::still_held();
        let mut marked = 0u64;
        let (p50, p90, max) = timed(|k| {
            canvas.dirty_mut().clear();
            fx.frame(delay * (1 + k % n), canvas);
            marked += u64::from(canvas.dirty().count());
        });
        drop(fx);
        say!(info,
            "psi-probe: consolea {name} ({way}): p50 {p50} p90 {p90} max {max} us a frame (timed with the count), est {est} us, marked {} px a frame of 4096, still cache {held} B, heap {h0} B before, main stack {} B unused",
            marked / u64::from(FRAMES),
            stack_unused()
        );
    }
    fx_psi::OLD.store(false, Ordering::Relaxed);
}

/// The VM alone on a frame: without and with the change compare (the
/// compare's own cost, ~1-2 µs a row by the note).
fn console_compare(name: &str, src: &str) {
    let mut prog = zeroed::<psi::Program>();
    if compile(src, &mut prog, true, true).is_err() {
        return;
    }
    let mut s = zeroed::<psi::Scratch<128>>();
    s.reset();
    let mut ink = psi::Ink::boxed();
    let mut out = vec![0u8; psi::W * psi::H * 3];
    let lp = prog.loop_ms().max(1);
    let mut ch = [0u32; psi::H];
    let t = fx_psi::tables();
    let mut run = |with: bool| {
        timed(|k| {
            let i = inputs_at(&prog, k * 10 % lp);
            if with {
                psi::render_rows_changed::<128>(&prog, &mut s, Some(&mut ink), None, &i, &t, None, u32::MAX, &mut out, &mut ch);
            } else {
                psi::render_rows_still::<128>(&prog, &mut s, Some(&mut ink), None, &i, &t, None, u32::MAX, &mut out);
            }
        })
    };
    let (a50, a90, _) = run(false);
    let (b50, b90, _) = run(true);
    say!(info, "psi-probe: consolea compare {name}: VM frame p50/p90 {a50}/{a90} us without, {b50}/{b90} with the change compare");
}

/// Scanlines as a look over a GIF-like picture: a few rows of it change a
/// frame (~17 % of the pixels, the card's mean); the look redraws those
/// rows. What it marks now (the pixels that changed) against the whole
/// rows it marked before Phase A, and its time.
fn console_look(canvas: &mut Canvas) {
    let Ok(mut look) = fx_psi::FormulaLook::from_path(&fx_psi::path(SCANLINES_FILE, "")) else {
        say!(error, "psi-probe: consolea look: did not open");
        return;
    };
    if !look.take() {
        say!(error, "psi-probe: consolea look: no room for its buffers ({} B free)", heap());
        return;
    }
    canvas.clear();
    let _ = look.apply(Duration::ZERO, canvas, 0);
    let (mut marked, mut rows_px, mut seed) = (0u64, 0u64, 0x2545_f491u32);
    let mut us = Vec::with_capacity(FRAMES as usize);
    for k in 0..FRAMES {
        // ~700 pixels in 6 rows, as a GIF frame changes them.
        for _ in 0..700 {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            let y = (seed as usize >> 8) % 6 + (k as usize * 5) % 26;
            let x = (seed as usize >> 16) % psi::W;
            canvas.set(x, y, opendmd_core::canvas::Rgb::new(seed as u8, (seed >> 8) as u8, k as u8));
        }
        rows_px += u64::from(canvas.dirty().row_mask().count_ones()) * psi::W as u64;
        let t = Instant::now();
        let out = look.apply(Duration::from_millis(u64::from(k) * 40 + 40), canvas, 0);
        us.push(t.elapsed().as_micros() as u32);
        marked += u64::from(out.dirty().count());
        out.dirty_mut().clear();
    }
    let (p50, p90, max) = pct(&mut us);
    say!(info,
        "psi-probe: consolea look Scanlines over a changing picture: p50 {p50} p90 {p90} max {max} us a frame, marked {} px a frame (the rows it redrew: {} px)",
        marked / u64::from(FRAMES),
        rows_px / u64::from(FRAMES)
    );
}

/// consolea: Phase A's rows, then the play lines (with the marking A/B).
fn console_probe(root: &str, start: Instant, stack0: u32) {
    play::AB.store(true, Ordering::Relaxed);
    play::start(format!("{root}/{REPORT_NAME}"));
    say!(info, "psi-probe: consolea: heap {} B free, largest block {} B, main stack {stack0} B unused, Scratch<128> {} B",
        heap(), crate::state::largest_free_block(), std::mem::size_of::<psi::Scratch<128>>());
    fx_psi::set_still_gate(Some(crate::still_gate));
    let mut canvas = Canvas::new();
    let rows: [(&str, &str, &str); 6] = [
        ("Endless road", bundled_src("endless-road"), ""),
        ("Starfield", STARFIELD_FILE, ""),
        ("Big clock", BIG_CLOCK_FILE, ""),
        ("Glitter", GLITTER_FILE, ""),
        ("Rotozoom floor", ROTOZOOM_FLOOR_FILE, ""),
        ("Time gate", bundled_src("time-gate"), ""),
    ];
    for (name, src, opts) in rows {
        console_row(name, src, opts, &mut canvas);
        fx_psi::set_still_gate(Some(crate::still_gate));
    }
    console_look(&mut canvas);
    console_compare("Endless road", bundled_src("endless-road"));
    console_compare("Starfield", STARFIELD_FILE);
    say!(info,
        "{DONE}: consolea, main stack {} B unused (was {stack0}), heap {} B free (largest {} B), {} ms in all",
        stack_unused(), heap(), crate::state::largest_free_block(), start.elapsed().as_millis()
    );
    *REPORT.lock().unwrap() = None;
}

/// console-lessons Phase B (consoleb): only its rows. `false`: the full
/// probe (or another `*_ONLY` mode).
const CONSOLEB_ONLY: bool = false;

/// consoleb: `cycle` and `layer rate N`: the goldens (VM and kernels),
/// the new Formulas and Big clock's rewrites as effects (JIT on, then off),
/// then the VM's step rows for the estimate (the turn, the held copy).
#[cfg(feature = "jit")]
fn consoleb_probe(start: Instant, stack0: u32) {
    say!(info, "psi-probe: consoleb: heap {} B free, largest block {} B, main stack {stack0} B unused, Held {} B",
        heap(), crate::state::largest_free_block(), psi::Held::BYTES);
    let mut out = vec![0u8; psi::W * psi::H * 3];
    let (vo, vb) = vm_goldens(&mut out, false);
    let (ko, kb) = jit2::goldens(&mut out);
    say!(info,
        "psi-probe: golden frames: {} match the host/wasm hashes, {} differ{}",
        vo + ko, vb + kb, if vb + kb == 0 { " (bit-identical)" } else { "" }
    );
    let mut canvas = Canvas::new();
    let list: [(&str, &str); 8] = [
        ("Big clock", BIG_CLOCK_FILE),
        ("Big clock rate", include_str!("../../opendmd-web/experiments/console-b/big-clock-rate.psi")),
        ("Big clock rate (bar in the mask)", include_str!("../../opendmd-web/experiments/console-b/big-clock-rate-mask.psi")),
        ("Cycled plasma", include_str!("../../opendmd-web/formulas/cycled-plasma.psi")),
        ("Plasma", include_str!("../../opendmd-web/formulas/plasma.psi")),
        ("Waterfall", include_str!("../../opendmd-web/formulas/waterfall.psi")),
        ("Copper sky", include_str!("../../opendmd-web/formulas/copper-sky.psi")),
        ("Storm front", include_str!("../../opendmd-web/formulas/storm-front.psi")),
    ];
    for (name, src) in list {
        let mut prog = zeroed::<psi::Program>();
        if compile(src, &mut prog, true, true).is_ok() {
            say!(info, "psi-probe: consoleb {name}: est VM {} us (still {}), JIT {} us (still {})",
                prog.esp32_us(), prog.esp32_us_still(), prog.esp32_us_jit(), prog.esp32_us_jit_still());
        }
        drop(prog);
        for c in &fx_psi::RATE_FRAMES {
            c.store(0, Ordering::Relaxed);
        }
        jit2::effect(name, src, "", &mut canvas);
        consoleb_split(name, src);
    }
    // The compiler's time on the longest bundled Formula (some of the
    // compiler is optimised for size since consoleb).
    {
        let road = bundled_src("endless-road");
        let mut prog = zeroed::<psi::Program>();
        let (p50, p90, _) = timed(|_| {
            let _ = compile(road, &mut prog, true, true);
        });
        say!(info, "psi-probe: consoleb compile Endless road ({} B): p50/p90 {p50}/{p90} us", road.len());
    }
    // The VM's frames directly (no still cache, no held picture unless
    // said): the turn's cost against `pal`, the held copy's.
    let rows: [(&str, &str, bool); 9] = [
        ("gray(x)", "gray(x)", false),
        ("pal(fire, x)", "pal(fire, x)", false),
        ("pal(fire, frac(x))", "pal(fire, frac(x*3))", false),
        ("cycle frame", "cycle(fire, x*3, t)", false),
        ("cycle row", "cycle(fire, x*3, t + py/64)", false),
        ("cycle x3 rows", "layer { cycle(fire, x*3, t + py/64) }\nlayer add { cycle(ice, y, t/2 + py/30, 0.5) }\nlayer max { cycle(chrono, x, py/20 - t, 0.3) }", false),
        ("layers, every frame", "layer { gray(x) }\nlayer max { gray(y) }", false),
        ("rate 50, held (still)", "layer rate 50 { gray(x) }\nlayer max { gray(y) }", true),
        ("rate 4, held (t)", "layer rate 4 { hsv(x + t, 1, 1) }\nlayer max { gray(y) }", true),
    ];
    let t = fx_psi::tables();
    for (name, src, held) in rows {
        let mut prog = zeroed::<psi::Program>();
        if let Err(e) = compile(src, &mut prog, true, true) {
            say!(error, "psi-probe: consoleb row {name}: {e:?}");
            continue;
        }
        let mut s = zeroed::<psi::Scratch<128>>();
        s.reset();
        let mut h = psi::Held::try_boxed();
        let lp = prog.loop_ms().max(1);
        let (p50, p90, _) = timed(|k| {
            let i = inputs_at(&prog, k * 20 % lp);
            match h.as_deref_mut().filter(|_| held) {
                Some(h) => psi::render_held::<128>(&prog, &mut s, None, None, h, &i, &t, &mut out, None),
                None => psi::render_rows_still::<128>(&prog, &mut s, None, None, &i, &t, None, u32::MAX, &mut out),
            }
        });
        say!(info, "psi-probe: consoleb row {name}: VM frame p50/p90 {p50}/{p90} us, est {} us", prog.esp32_us());
    }
    say!(info,
        "{DONE}: consoleb, main stack {} B unused (was {stack0}), heap {} B free (largest {} B), {} ms in all",
        stack_unused(), heap(), crate::state::largest_free_block(), start.elapsed().as_millis()
    );
    *REPORT.lock().unwrap() = None;
}

/// A `rate` formula drawn directly, frame by frame (each 20 ms on): the
/// held picture brought up to date, then its fast part by the VM and by
/// the kernel over it, µs and pixels changed; and the frame drawn from
/// nothing. Nothing for a formula without `rate`.
#[cfg(feature = "jit")]
fn consoleb_split(name: &str, src: &str) {
    let mut prog = zeroed::<psi::Program>();
    if compile(src, &mut prog, true, true).is_err() || prog.rate().is_none() {
        return;
    }
    let t = fx_psi::tables();
    let mut s = zeroed::<psi::Scratch<128>>();
    s.reset();
    let Some(mut held) = psi::Held::try_boxed() else {
        say!(error, "psi-probe: consoleb split {name}: no heap for the held picture ({} B free, largest {} B)", heap(), crate::state::largest_free_block());
        return;
    };
    let mut kernel = psi::jit::emit(&prog);
    let mut a = vec![0u8; psi::W * psi::H * 3];
    let mut b = vec![0u8; psi::W * psi::H * 3];
    let px = |ch: &[u32; psi::H]| ch.iter().map(|m| m.count_ones() * 4).sum::<u32>();
    let mut sums = [0u64; 6];
    let n = 40u32;
    for k in 0..n {
        let mut i = psi::golden::inputs(0);
        i.set_ms(&prog, u64::from(k) * 20);
        let t0 = Instant::now();
        let rows = held.update::<128>(&prog, &mut s, &i, &t);
        let t1 = Instant::now();
        let mut ch = [0u32; psi::H];
        psi::render_fast::<128>(&prog, &mut s, None, None, &i, &t, Some(held.picture()), u32::MAX, &mut a, Some(&mut ch));
        let t2 = Instant::now();
        let vpx = px(&ch);
        let (mut kus, mut kpx) = (0u64, 0u64);
        if let Ok(kn) = kernel.as_mut() {
            let mut ch = [0u32; psi::H];
            let t3 = Instant::now();
            let drew = kn.render(&prog, &mut s, None, None, &i, &t, Some(held.picture()), u32::MAX, &mut b, Some(&mut ch));
            kus = t3.elapsed().as_micros() as u64;
            kpx = u64::from(px(&ch));
            if !drew {
                say!(info, "psi-probe: consoleb split {name}: frame {k}: the kernel's inputs out of range (the VM drew it)");
            }
            if a != b {
                say!(error, "psi-probe: consoleb split {name}: frame {k}: the kernel's bytes differ from the VM's");
            }
        }
        for (j, v) in [u64::from(rows), (t1 - t0).as_micros() as u64, (t2 - t1).as_micros() as u64, u64::from(vpx), kus, kpx].into_iter().enumerate() {
            sums[j] += v;
        }
    }
    let m = |j: usize| sums[j] / u64::from(n);
    say!(info,
        "psi-probe: consoleb split {name}: a frame on average: {} slow rows drawn in {} us; the fast part VM {} us ({} px changed), kernel {} us ({} px changed){}",
        m(0), m(1), m(2), m(3), m(4), m(5),
        match &kernel { Ok(_) => String::new(), Err(w) => format!(" (no kernel: {w})") }
    );
    let (p50, _, _) = timed(|k| {
        let mut i = psi::golden::inputs(0);
        i.set_ms(&prog, u64::from(k) * 20);
        psi::render_rows_still::<128>(&prog, &mut s, None, None, &i, &t, None, u32::MAX, &mut a);
    });
    say!(info, "psi-probe: consoleb split {name}: from nothing (no held picture) p50 {p50} us");
}

/// consoleb-fix: Endless road's frames. Only these rows, then the play
/// lines (play `@formula:endless-road` after it to see its misses).
/// `false`: the full probe (or another `*_ONLY` mode).
const ROADFIX_ONLY: bool = false;

/// consoleb-fix: Endless road as bundled (played by name), directly
/// (its kernel's frame, the VM's) and as an effect (JIT on, then off); what
/// the per-frame `rate` questions cost (`Program::rate`, 40 a frame).
#[cfg(feature = "jit")]
fn roadfix_probe(root: &str, start: Instant, stack0: u32) {
    play::start(format!("{root}/{REPORT_NAME}"));
    say!(info, "psi-probe: roadfix: heap {} B free, largest block {} B, main stack {stack0} B unused",
        heap(), crate::state::largest_free_block());
    fx_psi::set_still_gate(Some(crate::still_gate));
    let road = bundled_src("endless-road");
    let mut prog = zeroed::<psi::Program>();
    if compile(road, &mut prog, true, true).is_ok() {
        let mut n = 0u32;
        let (q50, _, _) = timed(|_| {
            for _ in 0..40 {
                n += u32::from(core::hint::black_box(&*prog).rate().is_some());
            }
        });
        say!(info, "psi-probe: roadfix: 40 rate() questions a frame: p50 {q50} us ({n})");
        let t = fx_psi::tables();
        let mut s = zeroed::<psi::Scratch<128>>();
        s.reset();
        let mut ink = psi::Ink::boxed();
        let mut out = vec![0u8; psi::W * psi::H * 3];
        let (v50, v90, _) = timed(|k| {
            let i = inputs_at(&prog, k * 10);
            psi::render_rows_ink::<128>(&prog, &mut s, Some(&mut ink), &i, &t, None, u32::MAX, &mut out);
        });
        let kernel = psi::jit::emit(&prog);
        let (k50, k90) = match kernel {
            Ok(mut kn) => {
                let (a, b, _) = timed(|k| {
                    let i = inputs_at(&prog, k * 10);
                    kn.render(&prog, &mut s, Some(&mut ink), None, &i, &t, None, u32::MAX, &mut out, None);
                });
                (a, b)
            }
            Err(_) => (0, 0),
        };
        say!(info, "psi-probe: roadfix: Endless road direct: VM p50/p90 {v50}/{v90} us, kernel {k50}/{k90} us");
    }
    drop(prog);
    let mut canvas = Canvas::new();
    jit2::effect("Endless road", road, "", &mut canvas);
    fx_psi::set_still_gate(Some(crate::still_gate));
    say!(info,
        "{DONE}: roadfix, main stack {} B unused (was {stack0}), heap {} B free (largest {} B), {} ms in all; play @formula:endless-road now for the play lines",
        stack_unused(), heap(), crate::state::largest_free_block(), start.elapsed().as_millis()
    );
    *REPORT.lock().unwrap() = None;
}

/// psi-ergo-1: only its rows. `false`: the full probe (or another
/// `*_ONLY` mode).
const ERGO_ONLY: bool = false;

/// psi-ergo-1: the goldens (VM and kernels: `ink-levels` among them),
/// Endless road as bundled (respelled: `ink(16)`, named palette entries,
/// lines hidden by ink 0) directly (VM, kernel) and as an effect (JIT on,
/// then off), and the VM's step rows for `ink(16)` against `ink*16`.
#[cfg(feature = "jit")]
fn ergo_probe(root: &str, start: Instant, stack0: u32) {
    play::start(format!("{root}/{REPORT_NAME}"));
    say!(info, "psi-probe: ergo: heap {} B free, largest block {} B, main stack {stack0} B unused",
        heap(), crate::state::largest_free_block());
    let mut out = vec![0u8; psi::W * psi::H * 3];
    let (vo, vb) = vm_goldens(&mut out, false);
    let (ko, kb) = jit2::goldens(&mut out);
    say!(info,
        "psi-probe: golden frames: {} match the host/wasm hashes, {} differ{}",
        vo + ko, vb + kb, if vb + kb == 0 { " (bit-identical)" } else { "" }
    );
    fx_psi::set_still_gate(Some(crate::still_gate));
    let t = fx_psi::tables();
    let road = bundled_src("endless-road");
    let mut prog = zeroed::<psi::Program>();
    if compile(road, &mut prog, true, true).is_ok() {
        let mut s = zeroed::<psi::Scratch<128>>();
        s.reset();
        let mut ink = psi::Ink::boxed();
        let (v50, v90, _) = timed(|k| {
            let i = inputs_at(&prog, k * 10);
            psi::render_rows_ink::<128>(&prog, &mut s, Some(&mut ink), &i, &t, None, u32::MAX, &mut out);
        });
        let (k50, k90) = match psi::jit::emit(&prog) {
            Ok(mut kn) => {
                let (a, b, _) = timed(|k| {
                    let i = inputs_at(&prog, k * 10);
                    kn.render(&prog, &mut s, Some(&mut ink), None, &i, &t, None, u32::MAX, &mut out, None);
                });
                (a, b)
            }
            Err(_) => (0, 0),
        };
        say!(info, "psi-probe: ergo: Endless road direct: VM p50/p90 {v50}/{v90} us, kernel {k50}/{k90} us, est VM {} JIT {} us",
            prog.esp32_us(), prog.esp32_us_jit());
    }
    drop(prog);
    let mut canvas = Canvas::new();
    jit2::effect("Endless road", road, "", &mut canvas);
    fx_psi::set_still_gate(Some(crate::still_gate));
    // The VM's frames of a canvas read three ways (16 lines of ink k/16).
    const DRAW: &str = "draw { for k < 16 { line(k*8, 0, k*8 + 7, 31, k/16) } }\n";
    let rows: [(&str, &str); 3] = [
        ("gray(ink)", "gray(ink)"),
        ("ink*16 picked", "pal(fire, pick(max(px > 200, ink*16), 0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1, 0.9, 0.8, 0.7, 0.6, 0.5))"),
        ("ink(16) picked", "pal(fire, pick(max(px > 200, ink(16)), 0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1, 0.9, 0.8, 0.7, 0.6, 0.5))"),
    ];
    for (name, colour) in rows {
        let src = format!("{DRAW}{colour}");
        let mut prog = zeroed::<psi::Program>();
        if let Err(e) = compile(&src, &mut prog, true, true) {
            say!(error, "psi-probe: ergo row {name}: {e:?}");
            continue;
        }
        let mut s = zeroed::<psi::Scratch<128>>();
        s.reset();
        let mut ink = psi::Ink::boxed();
        let (p50, p90, _) = timed(|k| {
            let i = inputs_at(&prog, k * 20);
            psi::render_rows_ink::<128>(&prog, &mut s, Some(&mut ink), &i, &t, None, u32::MAX, &mut out);
        });
        say!(info, "psi-probe: ergo row {name}: VM frame p50/p90 {p50}/{p90} us, est {} us", prog.esp32_us());
    }
    say!(info,
        "{DONE}: ergo, main stack {} B unused (was {stack0}), heap {} B free (largest {} B), {} ms in all; play @formula:endless-road now for the play lines",
        stack_unused(), heap(), crate::state::largest_free_block(), start.elapsed().as_millis()
    );
    *REPORT.lock().unwrap() = None;
}

/// The line a finished probe ends its report with.
const DONE: &str = "psi-probe: done";



/// Open the report at `root` for this boot's probe: `false` (and nothing
/// to run) when last boot's report is there without its [`DONE`] line.
fn open_report(root: &str) -> bool {
    let path = format!("{root}/{REPORT_NAME}");
    if let Ok(old) = std::fs::read_to_string(&path) {
        if !old.contains(DONE) {
            log::warn!(
                "psi-probe: /{REPORT_NAME} has no `{DONE}` line: last boot's probe did not finish; not running it again (delete the file to)"
            );
            return false;
        }
    }
    match std::fs::File::create(&path) {
        Ok(mut f) => {
            use std::io::Write;
            let _ = writeln!(
                f,
                "# OpenDMD {} psi-probe report, started {} ms after boot",
                crate::state::firmware_version(),
                // SAFETY: a clock read.
                unsafe { esp_idf_sys::esp_timer_get_time() } / 1000
            );
            let _ = f.sync_data();
            *REPORT.lock().unwrap() = Some(f);
            log::info!("psi-probe: writing /{REPORT_NAME} as it goes: GET /{REPORT_NAME}");
            true
        }
        Err(e) => {
            log::error!("psi-probe: cannot write {path} ({e}); not running");
            false
        }
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


/// As [`lanes`], over `under` (the picture a formula look is drawn over).
fn lanes_over<const N: usize>(
    prog: &psi::Program,
    loop_ms: u32,
    under: Option<&[u8]>,
    out: &mut [u8],
) -> (u32, u32, u32) {
    let mut s = zeroed::<psi::Scratch<N>>();
    s.reset();
    let mut ink = psi::Ink::boxed();
    let t = fx_psi::tables();
    timed(|k| {
        let inputs = psi::golden::inputs(k * 20 % loop_ms.max(1));
        psi::render_rows_ink::<N>(prog, &mut s, Some(&mut ink), &inputs, &t, under, u32::MAX, out);
    })
}

/// A golden frame again laid out without the column and still runs, and
/// (when it has values for one) drawn with a still cache, kept on one frame
/// and drawn from on the next: (matches, differences). `prog` is left
/// compiled as usual.
/// The golden frames through the VM against the host's and wasm's hashes:
/// plain (with `variants_too`, also without the runs and from the still
/// cache, `variants`), over the picture, drawn (with a canvas). (matched,
/// differing).
#[inline(never)]
fn vm_goldens(out: &mut [u8], variants_too: bool) -> (u32, u32) {
    let t = fx_psi::tables();
    let mut s = zeroed::<psi::Scratch<128>>();
    s.reset();
    let mut prog = zeroed::<psi::Program>();
    let mut under = vec![0u8; psi::W * psi::H * 3];
    psi::golden::picture(&mut under);
    let mut ink = psi::Ink::boxed();
    let (mut ok, mut bad) = (0u32, 0u32);
    let plain = psi::golden::FRAMES.iter().map(|&(n, src, ms, want)| (n, src, ms, false, want, true));
    let over = psi::golden::OVER.iter().map(|&(n, src, ms, o, want)| (n, src, ms, o, want, false));
    let drawn = psi::golden::DRAWN.iter().map(|&(n, src, ms, o, want)| (n, src, ms, o, want, false));
    for (name, src, ms, o, want, first) in plain.chain(over).chain(drawn) {
        if heap_low("golden frames") {
            break;
        }
        if compile(src, &mut prog, true, true).is_err() {
            bad += 1;
            say!(error, "psi-probe: golden {name}: does not compile");
            continue;
        }
        let u = o.then_some(under.as_slice());
        psi::render_rows_ink::<128>(&prog, &mut s, Some(&mut ink), &psi::golden::inputs(ms), &t, u, u32::MAX, out);
        let got = psi::golden::fnv(out);
        if got == want {
            ok += 1;
        } else {
            bad += 1;
            say!(error, "psi-probe: golden {name} at {ms} ms: {got:#018x}, host/wasm {want:#018x}");
        }
        if variants_too && first {
            let (o, b) = variants(name, src, ms, &mut s, &mut prog, out, want);
            ok += o;
            bad += b;
        }
    }
    (ok, bad)
}

#[inline(never)]
fn variants(
    name: &str,
    src: &str,
    ms: u32,
    s: &mut psi::Scratch<128>,
    prog: &mut psi::Program,
    out: &mut [u8],
    want: u64,
) -> (u32, u32) {
    let t = fx_psi::tables();
    let (mut ok, mut bad) = (0, 0);
    let mut plain = zeroed::<psi::Program>();
    if compile(src, &mut plain, false, true).is_ok() {
        psi::render_with::<128>(&plain, s, &psi::golden::inputs(ms), &t, out);
        let got = psi::golden::fnv(out);
        if got == want {
            ok += 1;
        } else {
            bad += 1;
            say!(error, "psi-probe: golden {name} (plain) at {ms} ms: {got:#018x}, host/wasm {want:#018x}");
        }
    }
    let _ = compile(src, prog, true, true);
    if prog.still_lanes() > 0 {
        let mut st = psi::Still::new();
        if !st.take(prog) {
            say!(error, "psi-probe: golden {name} (still): no heap for the cache ({} B free)", heap());
            return (ok, bad + 1);
        }
        for pass in ["kept", "cached"] {
            psi::render_rows_still::<128>(prog, s, None, Some(&mut st), &psi::golden::inputs(ms), &t, None, u32::MAX, out);
            let got = psi::golden::fnv(out);
            if got == want {
                ok += 1;
            } else {
                bad += 1;
                say!(error, "psi-probe: golden {name} (still, {pass}) at {ms} ms: {got:#018x}, host/wasm {want:#018x}");
            }
        }
    }
    (ok, bad)
}


/// The examples as effects on the real path (`Fx::parse`, the canvas taken
/// at open, `Fx::frame`, the still cache taken on the first frame): with
/// the cache, then with the gate refusing it. `again` names a second run.
#[inline(never)]
fn effects(list: &[(&str, &str, &str)], again: &str, canvas: &mut Canvas) {
    for &(name, src, opts) in list {
        // (Into a zeroed box: a `Program` by value is 2.8 KB of the main
        // task's stack.)
        let mut prog = zeroed::<psi::Program>();
        let (est, est_still) = if compile(src, &mut prog, true, true).is_ok() {
            (prog.esp32_us(), prog.esp32_us_still())
        } else {
            (0, 0)
        };
        // ("cache": the firmware's gate bypassed, so the cache is taken
        // whatever the heap; the player's gate keeps 32 KB free after it.)
        for gate in ["cache", "no cache"] {
            fx_psi::set_still_gate(if gate == "cache" { None } else { Some(|_| false) });
            let h0 = heap();
            let Some(mut fx) = Fx::parse(&fx_psi::path(src, opts)) else {
                say!(error, "psi-probe: example {name}: did not open");
                continue;
            };
            let h1 = heap();
            let delay = fx.delay();
            let n = fx.loop_frames();
            // The first frame (it takes and keeps the cache), untimed.
            fx.frame(Duration::ZERO, canvas);
            let h2 = heap();
            let held = fx_psi::still_held();
            let (n0, c0) = (psi::stats::NARROWED.load(Ordering::Relaxed), psi::stats::CHECKED.load(Ordering::Relaxed));
            let (e50, e90, emax) = timed(|k| {
                fx.frame(delay * (k % n), canvas);
            });
            drop(fx);
            let (n1, c1) = (psi::stats::NARROWED.load(Ordering::Relaxed), psi::stats::CHECKED.load(Ordering::Relaxed));
            say!(info,
                "psi-probe: example {name}{again} ({gate}): frames narrowed {} checked {}, scratch {:#x} picture {:#x} still lane {:#x}",
                n1 - n0,
                c1 - c0,
                psi::stats::SCRATCH.load(Ordering::Relaxed),
                psi::stats::OUT.load(Ordering::Relaxed),
                psi::stats::STILL.load(Ordering::Relaxed)
            );
            say!(info,
                "psi-probe: example {name}{again} as an effect ({gate}): p50 {e50} p90 {e90} max {emax} us a frame, est {est} us ({est_still} with the cache), heap {h0} -> {h1} B at open ({} B for the effect and its canvas), {h2} B after its first frame (still cache {held} B), {} B back after",
                h0.saturating_sub(h1),
                heap()
            );
        }
    }
}



/// The CPU's cycle counter (240 per µs), for psi's frame stamps.
fn ccount() -> u32 {
    // SAFETY: reads the CCOUNT special register.
    unsafe { esp_idf_sys::xthal_get_ccount() }
}

/// p50 / p90 of cycles, in µs (one decimal).
fn us50_90(v: &mut [u32]) -> String {
    v.sort_unstable();
    let n = v.len().max(1);
    let f = |c: u32| format!("{:.1}", f64::from(c) / 240.0);
    format!("{}/{}", f(v[n / 2]), f(v[(n * 9 / 10).min(n - 1)]))
}

/// One effect's frames split by the stamps (CCOUNT): before the VM (the
/// player's frame: the gate, the still cache's step, the time), the VM's
/// setup (frame steps, the cache's choice), its rows, after it (the
/// governor, the status), and outside the player's frame (the engine's
/// call), p50/p90 µs each; with the firmware's gate (`gate`) or none.
#[inline(never)]
fn split(name: &str, src: &str, gate: bool, canvas: &mut Canvas) {
    fx_psi::set_still_gate(if gate { Some(crate::still_gate) } else { None });
    let Some(mut fx) = Fx::parse(&fx_psi::path(src, "")) else {
        say!(error, "psi-probe: split {name}: did not open");
        return;
    };
    let delay = fx.delay();
    let n = fx.loop_frames();
    fx.frame(Duration::ZERO, canvas);
    let held = fx_psi::still_held();
    let mut parts: [Vec<u32>; 6] = Default::default();
    if !reserve(&mut parts, &format!("split {name}")) {
        return;
    }
    for k in 0..FRAMES {
        let t0 = ccount();
        fx.frame(delay * (k % n), canvas);
        let t1 = ccount();
        let st: [u32; 5] = core::array::from_fn(|i| psi::stats::STAMPS[i].load(Ordering::Relaxed));
        let inside = st[4].wrapping_sub(st[0]);
        parts[0].push(st[1].wrapping_sub(st[0]));
        parts[1].push(st[2].wrapping_sub(st[1]));
        parts[2].push(st[3].wrapping_sub(st[2]));
        parts[3].push(st[4].wrapping_sub(st[3]));
        parts[4].push(t1.wrapping_sub(t0).wrapping_sub(inside));
        parts[5].push(t1.wrapping_sub(t0));
        std::thread::sleep(Duration::from_millis(1));
    }
    drop(fx);
    let [a, b, c, d, e, t] = &mut parts;
    say!(info,
        "psi-probe: split {name} (gate {}, still cache {held} B), p50/p90 us: before the VM {} | VM setup {} | VM rows {} | after the VM {} | outside {} | in all {} ({})",
        if gate { "on" } else { "off" },
        us50_90(a), us50_90(b), us50_90(c), us50_90(d), us50_90(e), us50_90(t),
        addrs()
    );
}

/// The last frame's memory: scratch, picture, still lanes.
fn addrs() -> String {
    let l = |a: &std::sync::atomic::AtomicUsize| a.load(Ordering::Relaxed);
    format!(
        "scratch {:#x} picture {:#x} still lanes {:#x} {:#x}",
        l(&psi::stats::SCRATCH),
        l(&psi::stats::OUT),
        l(&psi::stats::STILL),
        l(&psi::stats::STILL1)
    )
}



/// A probe row's compile on the formula stack, as the player compiles
/// (`psi_stack`): the probe's own rows never deepen the main task's stack.
fn compile(src: &str, prog: &mut psi::Program, runs: bool, narrow: bool) -> Result<psi::Info, psi::Error> {
    crate::psi_stack::run(|| psi::compile_with(src, prog, runs, narrow)).unwrap_or_else(|| {
        Err(psi::Error { start: 0, end: 0, msg: String::from("no formula stack") })
    })
}

/// Whether the heap is too low to go on with `section` (under 40 KB):
/// said, and the caller stops it, rather than the probe failing an
/// allocation.
/// Room for `FRAMES` timings in each of `parts`, taken fallibly (lang-probe-a
/// panicked growing these with 5 KB free): false, said, when the heap hasn't it.
fn reserve(parts: &mut [Vec<u32>], section: &str) -> bool {
    for p in parts.iter_mut() {
        if p.try_reserve_exact(FRAMES as usize).is_err() {
            say!(error, "psi-probe: {section}: stopped, no heap for its timings ({} B free)", heap());
            return false;
        }
    }
    true
}

fn heap_low(section: &str) -> bool {
    let h = heap();
    if h < 40 * 1024 {
        say!(error, "psi-probe: {section}: stopped, heap {h} B free (under 40 KB), largest block {} B", crate::state::largest_free_block());
        true
    } else {
        false
    }
}


/// What compiling leaves behind: one formula compiled on the player's
/// formula stack (`psi_stack::compile`, a FreeRTOS task since speed-d) 200
/// times at a builder's pace (20 ms apart), then on the old std thread 50
/// times, the heap before and after each.
#[inline(never)]
fn compile_leak() {
    let mut prog = zeroed::<psi::Program>();
    let src = bundled_src("tunnel");
    for (how, n) in [("formula stack (task)", 200u32), ("old std thread", 50)] {
        std::thread::sleep(Duration::from_millis(50));
        let (h0, b0) = (heap(), crate::state::largest_free_block());
        let mut done = 0;
        for k in 0..n {
            if heap_low(&format!("compile leak, {how}")) {
                break;
            }
            let ok = if k == 0 || how.starts_with("formula") {
                crate::psi_stack::compile(src, &mut prog).is_ok()
            } else {
                crate::psi_stack::run_std_thread(|| psi::compile_into(src, &mut prog))
                    .is_some_and(|r| r.is_ok())
            };
            if !ok {
                say!(error, "psi-probe: compile leak, {how}: compile {k} failed");
                break;
            }
            done += 1;
            std::thread::sleep(Duration::from_millis(20));
        }
        std::thread::sleep(Duration::from_millis(50));
        let (h1, b1) = (heap(), crate::state::largest_free_block());
        say!(info,
            "psi-probe: compile leak, {how}: {done} compiles, heap {h0} -> {h1} B ({} B a compile), largest block {b0} -> {b1} B",
            (i64::from(h0) - i64::from(h1)) / i64::from(done.max(1))
        );
    }
}



/// How a psi-speed2 row draws: the program as compiled (fused runs, draw
/// loops as lanes), compiled without the fused runs' markers, or that
/// with every draw loop a turn at a time (before psi-speed2).
#[derive(Clone, Copy, PartialEq)]
enum Way {
    Fused,
    Steps,
    Turns,
}

/// A frame's inputs at `ms` into the row (a `loop forever` formula's
/// `tw` from `ms` as it is, the others' `t` within the loop).
fn inputs_at(prog: &psi::Program, ms: u32) -> psi::Inputs {
    let lp = prog.loop_ms();
    psi::golden::inputs(if lp == 0 { ms } else { ms % lp })
}

/// One frame of `prog` drawn `way` (with `st`, a still cache, if given).
fn draw_way(
    prog: &psi::Program,
    way: Way,
    s: &mut psi::Scratch<128>,
    ink: &mut psi::Ink,
    st: Option<&mut psi::Still>,
    ms: u32,
    out: &mut [u8],
) {
    let t = fx_psi::tables();
    let i = inputs_at(prog, ms);
    if way == Way::Turns {
        psi::render_rows_ink_turns::<128>(prog, s, Some(ink), &i, &t, None, u32::MAX, out);
    } else {
        psi::render_rows_still::<128>(prog, s, Some(ink), st, &i, &t, None, u32::MAX, out);
    }
}


/// psi-speed2's before and after, in one boot: each example's VM frame as
/// compiled (fused runs; draw loops of 32 turns and more as lanes), without
/// the fused runs (`compile_unfused`), and that with every draw loop a turn
/// at a time (`render_rows_ink_turns`, for the examples that have a lane
/// loop); every frame of each the same bytes all three ways.
#[inline(never)]
fn speed2(out: &mut [u8]) {
    let (mut same, mut differ) = (0u32, 0u32);
    for (name, src) in [
        ("Starfield", bundled_src("starfield")),
        ("Wireframe cube", bundled_src("wireframe-cube")),
        ("Tunnel", bundled_src("tunnel")),
        ("Rotozoom", bundled_src("rotozoom")),
        ("Rotozoom floor", bundled_src("rotozoom-floor")),
        ("Endless road", bundled_src("endless-road")),
        ("Plasma", bundled_src("plasma")),
        ("Fire", bundled_src("fire")),
    ] {
        if heap_low("speed2") {
            break;
        }
        let mut fused = zeroed::<psi::Program>();
        let mut steps = zeroed::<psi::Program>();
        let ok = crate::psi_stack::run(|| psi::compile_into(src, &mut fused)).is_some_and(|r| r.is_ok())
            && crate::psi_stack::run(|| psi::compile_unfused(src, &mut steps)).is_some_and(|r| r.is_ok());
        if !ok {
            say!(error, "psi-probe: speed2 {name}: does not compile");
            continue;
        }
        {
            let mut s = zeroed::<psi::Scratch<128>>();
            s.reset();
            let mut ink = psi::Ink::boxed();
            let span = if fused.loop_ms() == 0 { 60_000 } else { fused.loop_ms() };
            for ms in [0, span / 3, span * 2 / 3, span.saturating_sub(20)] {
                let mut h = [0u64; 3];
                for (j, (p, way)) in [(&*fused, Way::Fused), (&*steps, Way::Steps), (&*steps, Way::Turns)]
                    .into_iter()
                    .enumerate()
                {
                    draw_way(p, way, &mut s, &mut ink, None, ms, out);
                    h[j] = psi::golden::fnv(out);
                }
                if h[0] == h[1] && h[1] == h[2] {
                    same += 1;
                } else {
                    differ += 1;
                    say!(error,
                        "psi-probe: speed2 {name} at {ms} ms: fused {:#018x}, steps {:#018x}, turns {:#018x}",
                        h[0], h[1], h[2]
                    );
                }
            }
        }
    }
    say!(info,
        "psi-probe: speed2 frames: {same} the same fused, as steps and a turn at a time, {differ} differ{}",
        if differ == 0 { " (bit-identical)" } else { "" }
    );
}

/// Only [`bundled`] (and `done`): a probe image small enough for the 4mb
/// layout while the full probe is not (beta.11: 2,418,496 B, over the
/// factory partition); `make emu-probe` then fails on the missing golden
/// line, so read the bundled line from its log. `false`: the full probe.
const BUNDLED_ONLY: bool = false;

/// The bundled Formulas (`-- bundle` in opendmd-web/formulas/; the list
/// is firmware/build.rs's), each opened through the real path
/// (`Fx::parse`: the Formula's box, its compiled program loaded, the
/// canvas) and drawn for a few frames; then a heap made short on purpose:
/// Endless road refused ([`psi::NO_MEMORY`] or [`fx_psi::NO_CANVAS`]) with
/// the heap far short and just short of what its open took — never the
/// abort it was on 2026-10-29. `make emu-probe` requires the summary line.
#[inline(never)]
fn bundled(canvas: &mut Canvas) {
    let (mut opened, mut failed) = (0, 0);
    let mut road = None;
    for &(name, src) in BUNDLED {
        // Through the path playlists and looks use: `@formula:<name>`.
        let path = format!("{}{name}", opendmd_core::synth::FORMULA_REF);
        let h0 = heap();
        let Some(mut fx) = Fx::parse(&path) else {
            failed += 1;
            let why = fx_psi::open_error(&path).map(|e| e.to_string()).unwrap_or_default();
            say!(error, "psi-probe: bundled {name}: did not open: {why} (heap {h0} B free, largest block {} B)", crate::state::largest_free_block());
            continue;
        };
        let h1 = heap();
        let delay = fx.delay();
        for k in 0..3 {
            fx.frame(delay * k, canvas);
        }
        drop(fx);
        opened += 1;
        say!(info, "psi-probe: bundled {name}: opened and drawn, {} B of source, heap {h0} -> {h1} B at open, {} B after",
            psi::compact(src).len(), heap());
        if name == "endless-road" {
            // (What its open took: the Formula's box and its canvas.)
            road = Some(h0.saturating_sub(h1) as usize);
        }
    }
    let Some(open_use) = road else {
        say!(error, "psi-probe: bundled formulas: {opened} opened, {failed} failed; short heap: no Endless road to try");
        return;
    };
    // Ballast down to `left` B free: 4 KB blocks, then 256 B ones.
    let ballast = |left: usize| -> Vec<Vec<u8>> {
        let mut b: Vec<Vec<u8>> = Vec::new();
        let _ = b.try_reserve(512);
        for size in [4096usize, 256] {
            while heap() as usize > left + size && b.len() < b.capacity() {
                let mut v = Vec::new();
                if v.try_reserve_exact(size).is_err() {
                    break;
                }
                v.resize(size, 1u8);
                b.push(v);
            }
        }
        b
    };
    // The player loads a compiled Formula (no compile, no compile check):
    // what a short heap must do is refuse the open, never abort.
    // 1. Far too short: the Formula's box (~12 KB) does not fit; 8 KB left
    // for everything else on the heap meanwhile.
    let road_path = format!("{}endless-road", opendmd_core::synth::FORMULA_REF);
    let b = ballast(8 * 1024);
    let h = heap();
    let boxed = fx_psi::open_error(&road_path).map(|e| e.to_string()).unwrap_or_else(|| "opened".into());
    drop(b);
    say!(info, "psi-probe: short heap, {h} B free: Endless road: {boxed}");
    // 2. Just short of what its open took ({open_use} B: the box and the
    // canvas): the box may fit, the canvas not, or neither.
    let b = ballast(open_use.saturating_sub(5 * 1024).max(8 * 1024));
    let h = heap();
    let canvas_r = fx_psi::open_error(&road_path).map(|e| e.to_string()).unwrap_or_else(|| "opened".into());
    drop(b);
    say!(info, "psi-probe: short heap, {h} B free (its open took {open_use}): Endless road: {canvas_r}");
    let ok = |m: &str| m == psi::NO_MEMORY || m == fx_psi::NO_CANVAS;
    say!(info,
        "psi-probe: bundled formulas: {opened} opened, {failed} failed; short heap: {} far short, {} just short; heap {} B free after",
        if ok(&boxed) { "refused" } else { "NOT refused" },
        if ok(&canvas_r) { "refused" } else { "NOT refused" },
        heap()
    );
}

/// The card's Formula the probe plays (tools/ci-card.py writes it).
const CARD_FORMULA: &str = "/formulas/ci-test.psi";

/// Formulas by reference, opened as the engine opens them (`Engine::open_reader`):
/// a bundled one by name through its virtual stream, and the card's `.psi`
/// file read through the card (compacted as it is read) into one, each
/// drawn for a few frames. `make emu-probe` requires the summary line.
#[inline(never)]
fn refs(root: &str, canvas: &mut Canvas) {
    use opendmd_core::FrameSource;
    let draw = |r: std::io::Result<std::io::Cursor<Vec<u8>>>, canvas: &mut Canvas| -> String {
        let mut src = match r.and_then(FrameSource::open) {
            Ok(s) => s,
            Err(e) => return format!("did not open: {e}"),
        };
        for _ in 0..3 {
            if let Err(e) = src.next_frame(canvas) {
                return format!("did not draw: {e}");
            }
        }
        match src.formula_name() {
            Some(n) => format!("opened and drawn (\"{n}\")"),
            None => "opened and drawn".to_string(),
        }
    };
    let h0 = heap();
    let named = draw(
        opendmd_core::synth::open("@formula:endless-road?a=0.5")
            .ok_or_else(|| std::io::Error::other("not virtual")),
        canvas,
    );
    say!(info, "psi-probe: @formula:endless-road: {named} (heap {h0} B free at open)");
    let h0 = heap();
    // (The player plays a card Formula from its `.psb`; the probe card
    // carries the source alone, so it is built here, as the web app would.)
    let card = std::fs::read_to_string(format!("{root}{CARD_FORMULA}")).and_then(|text| {
        // (On a thread of its own, as below: the compiler's recursion.)
        let psb = std::thread::Builder::new()
            .stack_size(32 * 1024)
            .spawn(move || psi::build(&text).map_err(|e| e.msg))
            .map_err(std::io::Error::other)?
            .join()
            .map_err(|_| std::io::Error::other("the build panicked"))?
            .map_err(std::io::Error::other)?;
        Ok(opendmd_core::synth::open_formula_bytes("a=1", &psb))
    });
    let card = draw(card, canvas);
    say!(info, "psi-probe: {CARD_FORMULA}: {card} (heap {h0} B free at open)");
    let ok = |m: &str| m.starts_with("opened");
    say!(info,
        "psi-probe: formulas by reference: by name {}, card file {}",
        if ok(&named) { "played" } else { "FAILED" },
        if ok(&card) { "played" } else { "FAILED" }
    );
}

/// What opening Endless road takes, with 2 KB to spare: its box and canvas
/// (15,828 B in the emulator's bundled step).
const ROAD_OPEN_NEED: usize = 18 * 1024;

/// A formula sent over a formula, as `/api/play` sends it (`Engine::retune`
/// through the real engine and `SdSource`, the compiled Formula loaded):
/// Time gate on screen, the heap made short so that Endless road fits only
/// once Time gate is let go (the owner's panel, 558416c: refused until a
/// GIF or a lighter formula had played). Then a heap short even without
/// Time gate, when that can be made: refused, nothing on screen, the
/// playlist's GIF next. First, Time gate and a GIF in turn, the heap after
/// each GIF (a leak would fall). `make emu-probe` requires the summary line.
#[inline(never)]
fn swap(root: &str, canvas: &mut Canvas) {
    use opendmd_core::{DwellPolicy, Engine, FileCatalog, Filter, Mode, Scheduler};
    const GATE: &str = "@formula:time-gate";
    const ROAD: &str = "@formula:endless-road";
    let gif = std::fs::read_to_string(format!("{root}/filelist.txt"))
        .ok()
        .and_then(|l| l.lines().map(str::trim).find(|p| p.ends_with(".gif")).map(str::to_string));
    let Some(gif) = gif else {
        say!(error, "psi-probe: formula over formula: no GIF in filelist.txt");
        return;
    };
    let Ok(cat) = FileCatalog::build(std::io::Cursor::new(format!("{gif}\n").into_bytes()), &Filter::new()) else {
        say!(error, "psi-probe: formula over formula: no catalog");
        return;
    };
    let mut e = Engine::new(Box::new(cat), Scheduler::new(1, Mode::Sequential, 0), DwellPolicy::default(), Box::new(crate::sd::SdSource));
    // The engine's own clock: ticks 20 ms apart, past any transition.
    let mut t = Duration::ZERO;
    let mut run = |e: &mut Engine, canvas: &mut Canvas, ms: u64| {
        let end = t + Duration::from_millis(ms);
        while t < end {
            t += Duration::from_millis(20);
            if let Err(err) = e.tick(t, canvas) {
                say!(error, "psi-probe: formula over formula: tick: {err}");
            }
        }
        t
    };
    let keep = opendmd_core::FormulaKeep::Hold.dwell();
    let h0 = heap();
    let mut after_gif = Vec::new();
    for _ in 0..4 {
        let now = run(&mut e, canvas, 0);
        if let Err(err) = e.play_now_with(GATE, now, keep) {
            say!(error, "psi-probe: formula over formula: Time gate: {err}");
            return;
        }
        run(&mut e, canvas, 1_000);
        let now = run(&mut e, canvas, 0);
        if let Err(err) = e.play_now(&gif, now) {
            say!(error, "psi-probe: formula over formula: {gif}: {err}");
            return;
        }
        run(&mut e, canvas, 600);
        after_gif.push(heap());
    }
    say!(info, "psi-probe: formula over formula: Time gate and a GIF in turn: heap {h0} B, after each GIF {after_gif:?} B");

    // Time gate on screen; what it holds.
    let h1 = heap();
    let now = run(&mut e, canvas, 0);
    let _ = e.play_now_with(GATE, now, keep);
    run(&mut e, canvas, 1_000);
    let held = h1.saturating_sub(heap()) as usize;
    // What opening Endless road takes (its box and canvas: 15,828 B in the
    // emulator's bundled step) and 2 KB more: the player loads a compiled
    // Formula, it no longer compiles one.
    let need = ROAD_OPEN_NEED;
    // The heap down to `left` B free, most of it one block (the compile
    // needs a 16 KB one): that block held while the rest is filled with
    // 4 KB, then 256 B pieces (4 KB left over for the system), then let go.
    let short = |left: usize| -> Option<Vec<Vec<u8>>> {
        let mut hold: Vec<u8> = Vec::new();
        hold.try_reserve_exact(left.checked_sub(4096)?).ok()?;
        hold.push(1);
        let hold = core::hint::black_box(hold);
        let mut b: Vec<Vec<u8>> = Vec::new();
        b.try_reserve(1024).ok()?;
        for size in [4096usize, 256] {
            while heap() as usize > 4096 + size && b.len() < b.capacity() {
                let mut v = Vec::new();
                if v.try_reserve_exact(size).is_err() {
                    break;
                }
                v.resize(size, 1u8);
                b.push(v);
            }
        }
        drop(hold);
        Some(b)
    };
    let gate_on = |e: &Engine| e.now_playing().is_some_and(|p| p.path == GATE);
    let road_on = |e: &Engine| e.now_playing().is_some_and(|p| p.path == ROAD);
    // 1. Short beside Time gate, room once it goes: played.
    let mut first = "NOT tried".to_string();
    if gate_on(&e) && held > 4096 + 2048 {
        // (Under the check's figure already beside Time gate, as in the
        // emulator: nothing more to take.)
        let left = need - 2048;
        let ballast = if (heap() as usize) < left { Some(Vec::new()) } else { short(left) };
        match ballast {
            None => first = format!("NOT tried (no {left} B to keep)"),
            Some(b) => {
                let h = heap();
                let now = run(&mut e, canvas, 0);
                let r = e.retune(ROAD, now, keep);
                let r = match r {
                    None => "NOT a retune".to_string(),
                    Some(Ok(())) if road_on(&e) => "played".to_string(),
                    Some(Ok(())) => "NOT on screen".to_string(),
                    Some(Err(err)) => format!("NOT played ({err})"),
                };
                run(&mut e, canvas, 200);
                drop(b);
                first = r;
                say!(info, "psi-probe: formula over formula: Time gate holding {held} B, {h} B free (its open needs {need}): Endless road: {first}");
            }
        }
    }
    // 2. Short even without Time gate: refused, the playlist goes on —
    // when that can be made: Time gate holds more than Endless road's open
    // takes, so with a compiled Formula letting it go always makes room.
    let now = run(&mut e, canvas, 0);
    let _ = e.play_now_with(GATE, now, keep);
    run(&mut e, canvas, 1_000);
    let mut second = "NOT tried".to_string();
    let held_now = h1.saturating_sub(heap()) as usize;
    if gate_on(&e) && held_now + 4096 >= need {
        second = format!("not constructible (Time gate holds {held_now} B, Endless road needs {need})");
    } else if gate_on(&e) {
        let held = held_now;
        let left = need.saturating_sub(held + 4096).max(8192);
        match short(left) {
            None => second = format!("NOT tried (no {left} B to keep)"),
            Some(b) => {
                let h = heap();
                let now = run(&mut e, canvas, 0);
                let r = e.retune(ROAD, now, keep);
                let gone = e.now_playing().is_none();
                drop(b);
                run(&mut e, canvas, 600);
                let next = e.now_playing().is_some_and(|p| p.path == gif);
                second = match r {
                    Some(Err(err)) if fx_psi::is_no_memory(&err) && gone && next => "refused, the playlist on".to_string(),
                    Some(Err(err)) => format!("NOT as asked ({err}; nothing on screen {gone}, the GIF next {next})"),
                    _ => "NOT refused".to_string(),
                };
                say!(info, "psi-probe: formula over formula: {h} B free, Time gate holding {held} B (its open needs {need}): Endless road: {second}");
            }
        }
    }
    drop(e);
    say!(info,
        "psi-probe: formula over formula: short beside Time gate: {first}; short without it: {second}; heap {} B free after (was {h0})",
        heap()
    );
}

/// A card Formula sent over a card Formula, as the rig did it (owner,
/// build 664d0ed): Neon city playing from `/formulas/neon-city.psi`, the
/// file written anew (`PUT /api/files/…`), then `POST /api/play` of the
/// same path; the same within 200 ms of a skip (what the `PUT` did until
/// swap-mem2: the GIF's transition held its 24 KB through the compile,
/// refused at 48 KB free); and Wormhole run and Neon city over each other. Each play goes the way the
/// main loop sends it (`Engine::retune`, else `play_now_with`), with the
/// heap made like the rig's (free with a GIF on screen as the owner saw
/// it). `make emu-probe` requires the summary line.
#[inline(never)]
fn swap_card(root: &str, canvas: &mut Canvas) {
    use opendmd_core::{DwellPolicy, Engine, FileCatalog, Filter, Mode, Scheduler};
    const NEON: &str = "/formulas/neon-city.psi";
    const WORM: &str = "/formulas/wormhole-run.psi";
    const NEON_SRC: &str = include_str!("../../opendmd-web/formulas/neon-city.psi");
    const WORM_SRC: &str = include_str!("../../opendmd-web/formulas/wormhole-run.psi");
    // The owner's panel with a GIF on screen (build 664d0ed).
    const RIG_FREE_WITH_GIF: u32 = 95_060;
    // A card Formula is its `.psi` and the compiled `.psb` beside it (the
    // player plays the `.psb`; the web app writes both, as here).
    let write = |file: &str, src: &str| {
        std::fs::write(format!("{root}{file}"), src)?;
        // (Built on a thread of its own: the compiler's recursion is more
        // than the main task's stack.)
        let owned = src.to_string();
        let psb = std::thread::Builder::new()
            .stack_size(32 * 1024)
            .spawn(move || psi::build(&owned).map_err(|e| e.msg))
            .map_err(std::io::Error::other)?
            .join()
            .map_err(|_| std::io::Error::other("the build panicked"))?
            .map_err(std::io::Error::other)?;
        std::fs::write(format!("{root}{}", opendmd_core::synth::compiled_path(file)), psb)
    };
    if write(NEON, NEON_SRC).and_then(|_| write(WORM, WORM_SRC)).is_err() {
        say!(error, "psi-probe: card formula over card formula: could not write the files");
        return;
    }
    let gif = std::fs::read_to_string(format!("{root}/filelist.txt"))
        .ok()
        .and_then(|l| l.lines().map(str::trim).find(|p| p.ends_with(".gif")).map(str::to_string));
    let Some(gif) = gif else {
        say!(error, "psi-probe: card formula over card formula: no GIF in filelist.txt");
        return;
    };
    let Ok(cat) = FileCatalog::build(std::io::Cursor::new(format!("{gif}\n").into_bytes()), &Filter::new()) else {
        return;
    };
    let mut e = Engine::new(Box::new(cat), Scheduler::new(1, Mode::Sequential, 0), DwellPolicy::default(), Box::new(crate::sd::SdSource));
    e.set_transition_gate(Some(crate::transition_gate));
    let mut t = Duration::ZERO;
    let mut run = |e: &mut Engine, canvas: &mut Canvas, ms: u64| {
        let end = t + Duration::from_millis(ms);
        while t < end {
            t += Duration::from_millis(20);
            if let Err(err) = e.tick(t, canvas) {
                say!(error, "psi-probe: card formula over card formula: tick: {err}");
            }
        }
        t
    };
    let keep = opendmd_core::FormulaKeep::Hold.dwell();
    // As main.rs's `PlayNow` sends it.
    let play = |e: &mut Engine, canvas: &mut Canvas, path: &str, now: Duration| -> std::io::Result<()> {
        crate::room_for_formula(Some(&mut *e), canvas);
        match e.retune(path, now, keep) {
            Some(r) => r,
            None => e.play_now_with(path, now, keep),
        }
    };
    let on = |e: &Engine, p: &str| e.now_playing().is_some_and(|n| n.path == p);
    let blk = crate::state::largest_free_block;
    // The rig's heap: a GIF on screen, then ballast in 4 KB pieces down to
    // what the owner saw.
    let now = run(&mut e, canvas, 0);
    let _ = e.play_now(&gif, now);
    run(&mut e, canvas, 800);
    let mut ballast: Vec<Vec<u8>> = Vec::new();
    let _ = ballast.try_reserve(64);
    while heap() > RIG_FREE_WITH_GIF + 4096 && ballast.len() < ballast.capacity() {
        let mut v = Vec::new();
        if v.try_reserve_exact(4096 - 16).is_err() {
            break;
        }
        v.resize(4096 - 16, 1u8);
        ballast.push(v);
    }
    say!(info, "psi-probe: card formula over card formula: a GIF on screen, {} B free (largest {} B; the rig {RIG_FREE_WITH_GIF})", heap(), blk());
    let mut rows: Vec<String> = Vec::new();
    let mut step = |e: &mut Engine, canvas: &mut Canvas, what: &str, path: &str, skip_first: bool| {
        let now = run(e, canvas, 0);
        if skip_first {
            // A skip 200 ms before (what `PUT` of the file on screen did
            // until swap-mem2): the next file's transition still running.
            e.skip();
            run(e, canvas, 200);
        }
        let (h, b) = (heap(), blk());
        let now = if skip_first { run(e, canvas, 0) } else { now };
        let r = play(e, canvas, path, now);
        run(e, canvas, 1_500);
        let r = match r {
            Ok(()) if on(e, path) => "played".to_string(),
            Ok(()) => "NOT on screen".to_string(),
            Err(err) => format!("NOT played ({err})"),
        };
        say!(info, "psi-probe: card formula over card formula: {what}: from {h} B free (largest {b} B): {r}; {} B free while it plays", heap());
        rows.push(r);
    };
    step(&mut e, canvas, "Neon city over the GIF", NEON, false);
    let _ = write(NEON, &format!("{NEON_SRC}-- v2\n"));
    step(&mut e, canvas, "Neon city over itself, its file written anew", NEON, false);
    let _ = write(NEON, NEON_SRC);
    step(&mut e, canvas, "Neon city over the next file's transition (a skip 200 ms before)", NEON, true);
    step(&mut e, canvas, "Wormhole run over Neon city", WORM, false);
    step(&mut e, canvas, "Neon city over Wormhole run", NEON, false);
    drop(ballast);
    drop(e);
    let all = rows.iter().all(|r| r == "played");
    say!(info,
        "psi-probe: card formula over card formula: {}; heap {} B free after",
        if all { "all played".to_string() } else { format!("NOT all played: {rows:?}") },
        heap()
    );
}


/// The probe, when the card at `root` is the main task's (see the module
/// docs).
// --- The JIT spike (docs/research/psi-jit-esp32.md, phase 0) ---------------
//
// Three hand-assembled kernels (`src/jitspike/*.s`; `assemble.sh` makes the
// bytes), copied into executable heap and called: does `MALLOC_CAP_EXEC`
// give memory and where, does it run from each region, does a windowed call
// out of it reach a Rust helper, and what does a register-resident pixel
// stage (Rotozoom floor's) cost against the VM's lanes, drawing the same
// bytes.
mod jitk {
    pub static TINY: &[u8] = include_bytes!("jitspike/tiny.bin");
    pub static CALL: &[u8] = include_bytes!("jitspike/call.bin");
    pub static FLOOR: &[u8] = include_bytes!("jitspike/floor.bin");
}

type TinyFn = extern "C" fn() -> i32;
type HelperFn = extern "C" fn(i32) -> i32;
type CallFn = extern "C" fn(HelperFn, i32) -> i32;
type FloorFn = extern "C" fn(*const i32, *const i32, *mut u8, *const i32);

/// A block of executable heap.
struct Exec {
    p: *mut u32,
    bytes: usize,
}

impl Exec {
    fn take(bytes: usize) -> Option<Exec> {
        // SAFETY: a plain allocation, freed by `Drop`.
        let p = unsafe { esp_idf_sys::heap_caps_malloc(bytes, esp_idf_sys::MALLOC_CAP_EXEC) };
        if p.is_null() {
            None
        } else {
            Some(Exec { p: p as *mut u32, bytes })
        }
    }

    fn addr(&self) -> usize {
        self.p as usize
    }

    fn region(&self) -> &'static str {
        match self.addr() {
            0x4007_0000..=0x4007_FFFF => "cache IRAM",
            0x4008_0000..=0x4009_FFFF => "IRAM",
            0x400A_0000..=0x400B_FFFF => "D/IRAM",
            _ => "elsewhere",
        }
    }

    /// `code` into the block as 32-bit words (IRAM takes no byte stores):
    /// the address to call, or None when it does not fit.
    fn load(&self, code: &[u8]) -> Option<usize> {
        if code.len() > self.bytes {
            return None;
        }
        for (w, chunk) in code.chunks(4).enumerate() {
            let mut b = [0u8; 4];
            b[..chunk.len()].copy_from_slice(chunk);
            // SAFETY: within the block (checked above), word-aligned (the heap's).
            unsafe { self.p.add(w).write_volatile(u32::from_le_bytes(b)) };
        }
        // A data barrier (`memw`). No `isync`: internal SRAM has no
        // instruction cache, and the block is nowhere near fetched code.
        std::sync::atomic::fence(Ordering::SeqCst);
        Some(self.addr())
    }
}

impl Drop for Exec {
    fn drop(&mut self) {
        // SAFETY: from `heap_caps_malloc`, freed once.
        unsafe { esp_idf_sys::heap_caps_free(self.p as *mut _) }
    }
}

extern "C" fn sin_helper(a: i32) -> i32 {
    psi::fixed::sin(&fx_psi::tables(), a)
}

fn exec_heap() -> (usize, usize) {
    // SAFETY: plain queries.
    unsafe {
        (
            esp_idf_sys::heap_caps_get_free_size(esp_idf_sys::MALLOC_CAP_EXEC),
            esp_idf_sys::heap_caps_get_largest_free_block(esp_idf_sys::MALLOC_CAP_EXEC),
        )
    }
}

/// The floor's scalar slots as this build's compiler laid them out:
/// `L1 = mul_w(s_a, L0); L3 = L1 + s_b; L1 = mul_w(s_c, L0); L4 = L1 + L2;
/// L1 = L4 + s_d; … pal(L1, s_k)`.
#[derive(Clone, Copy, Debug)]
struct FloorSlots {
    a: usize,
    b: usize,
    c: usize,
    d: usize,
    k: usize,
}

/// The number between `prefix` and `suffix` in the `nth` line of `listing`
/// that has them (the slot a step reads).
fn slot_in(listing: &str, prefix: &str, suffix: &str, nth: usize) -> Option<usize> {
    listing
        .lines()
        .filter_map(|l| {
            let rest = &l[l.find(prefix)? + prefix.len()..];
            let end = rest.find(suffix)?;
            rest[..end].parse::<usize>().ok()
        })
        .nth(nth)
}

/// The floor's rows through the VM, one at a time, keeping what the kernel
/// reads: the four row scalars and the row's two colours (`params`), and
/// the column run's lanes `L0` (dx) and `L2` (col).
#[allow(clippy::too_many_arguments)]
fn floor_capture(
    prog: &psi::Program,
    t: &psi::Tables,
    s: &mut psi::Scratch<128>,
    ms: u32,
    slots: FloorSlots,
    params: &mut [[i32; 6]; psi::H],
    dx: &mut [i32; 128],
    col: &mut [i32; 128],
    out: &mut [u8],
) {
    let inputs = inputs_at(prog, ms);
    let fire = t.palettes[0];
    for (row, p) in params.iter_mut().enumerate() {
        psi::render_rows::<128>(prog, s, &inputs, t, None, 1 << row, out);
        let k = s.scalar_slot(slots.k);
        let ca = psi::fixed::pal_lit(fire, 51118, k);
        let cb = psi::fixed::pal_lit(fire, 18350, k);
        *p = [
            s.scalar_slot(slots.a),
            s.scalar_slot(slots.b),
            s.scalar_slot(slots.c),
            s.scalar_slot(slots.d),
            i32::from_le_bytes([ca[0], ca[1], ca[2], cb[0]]),
            i32::from_le_bytes([cb[1], cb[2], 0, 0]),
        ];
    }
    dx.copy_from_slice(s.lane_reg(0));
    col.copy_from_slice(s.lane_reg(2));
}

fn jit_spike(out: &mut [u8]) {
    let (free0, big0) = exec_heap();
    say!(info, "psi-probe: jit: exec heap {free0} B free, largest block {big0} B; data heap {} B free", heap());
    // 4 KB blocks until one is D/IRAM (the IRAM leftover goes first).
    let mut blocks: Vec<Exec> = Vec::new();
    let mut iram: Option<usize> = None;
    let mut diram: Option<usize> = None;
    for k in 0..12 {
        let Some(b) = Exec::take(4096) else {
            say!(error, "psi-probe: jit: block {k}: no executable memory");
            break;
        };
        say!(info,
            "psi-probe: jit: block {k}: {:#x} ({}); exec heap {} B free, data heap {} B free",
            b.addr(), b.region(), exec_heap().0, heap()
        );
        let last = b.region() == "D/IRAM";
        if b.region() == "IRAM" && iram.is_none() {
            iram = Some(blocks.len());
        }
        if last && diram.is_none() {
            diram = Some(blocks.len());
        }
        blocks.push(b);
        if last {
            break;
        }
    }
    let regions = [("IRAM", iram), ("D/IRAM", diram)];
    for (what, at) in regions {
        let Some(i) = at else {
            say!(error, "psi-probe: jit: no {what} block to run from");
            continue;
        };
        let Some(a) = blocks[i].load(jitk::TINY) else { continue };
        // SAFETY: the block holds `tiny_kernel`, a windowed-ABI function of no arguments.
        let f: TinyFn = unsafe { std::mem::transmute::<usize, TinyFn>(a) };
        let r = f();
        say!(info, "psi-probe: jit: tiny kernel at {a:#x} ({what}) returned {r} (want 42)");
    }
    if let Some(i) = iram.or(diram) {
        if let Some(a) = blocks[i].load(jitk::CALL) {
            // SAFETY: the block holds `call_kernel`.
            let f: CallFn = unsafe { std::mem::transmute::<usize, CallFn>(a) };
            let want = sin_helper(1 << 14);
            let got = f(sin_helper, 1 << 14);
            say!(info,
                "psi-probe: jit: call kernel at {a:#x}: sin(1/4) through callx8 = {got}, direct {want} ({})",
                if got == want { "the same" } else { "DIFFER" }
            );
        }
    }

    // Rotozoom floor's pixel stage by hand, against the VM.
    let mut prog = zeroed::<psi::Program>();
    if let Err(e) = compile(bundled_src("rotozoom-floor"), &mut prog, true, true) {
        say!(error, "psi-probe: jit: the floor does not compile: {}", e.msg);
        return;
    }
    // The kernel's layout (registers fixed, scalar slots read from the
    // listing: they move with the source's header, jitspike-a found).
    let listing = prog.listing();
    let want = [
        "pixel L0 = sub_w(px, 64)",
        "pixel L2 = mul_w(L1, s",
        "pixel L4 = add_w(L1, L2)",
        "pixel L4 = floor(L3)",
        "pixel L3 = floor(L1)",
        "pixel L1 = add_w(L4, L3)",
        "pixel L4 = mask(L1, 1.9999847412109375)",
        "pixel L1 = select(L4, 0.779998779296875, 0.279998779296875)",
    ];
    if let Some(miss) = want.iter().find(|w| !listing.contains(*w)) {
        say!(error, "psi-probe: jit: the floor's listing has no `{miss}`: the kernel was written for another layout; skipping");
        return;
    }
    let slots = [
        slot_in(&listing, "pixel L1 = mul_w(s", ", L0)", 0),
        slot_in(&listing, "pixel L3 = add_w(L1, s", ")", 0),
        slot_in(&listing, "pixel L1 = mul_w(s", ", L0)", 1),
        slot_in(&listing, "pixel L1 = add_w(L4, s", ")", 0),
        slot_in(&listing, "pixel out = pal(L1, s", ")", 0),
    ];
    let [Some(s_a), Some(s_b), Some(s_c), Some(s_d), Some(s_k)] = slots else {
        say!(error, "psi-probe: jit: the floor's scalar slots were not found in its listing ({slots:?}); skipping");
        return;
    };
    let slots = FloorSlots { a: s_a, b: s_b, c: s_c, d: s_d, k: s_k };
    say!(info, "psi-probe: jit: floor layout: mul_w(s{s_a}, L0) + s{s_b}; mul_w(s{s_c}, L0) + L2 + s{s_d}; pal * s{s_k}");
    let t = fx_psi::tables();
    let mut s = zeroed::<psi::Scratch<128>>();
    s.reset();
    let lp = prog.loop_ms().max(1);
    let mut jit = vec![0u8; out.len()];
    let mut params = [[0i32; 6]; psi::H];
    let mut dx = [0i32; 128];
    let mut col = [0i32; 128];
    let Some(i) = iram.or(diram) else { return };
    let Some(a) = blocks[i].load(jitk::FLOOR) else { return };
    // SAFETY: the block holds `floor_kernel`; its arguments are a 128-lane,
    // a 128-lane, a 384 B row and the 6-word params, all live below.
    let f: FloorFn = unsafe { std::mem::transmute::<usize, FloorFn>(a) };
    let row_of = |buf: &mut [u8], row: usize| buf[row * psi::W * 3..(row + 1) * psi::W * 3].as_mut_ptr();
    // The same bytes? Four times in the loop.
    let (mut differ, mut checked) = (0usize, 0usize);
    for ms in [0, lp / 4, lp / 2, lp * 3 / 4] {
        let inputs = inputs_at(&prog, ms);
        psi::render_rows::<128>(&prog, &mut s, &inputs, &t, None, u32::MAX, out);
        floor_capture(&prog, &t, &mut s, ms, slots, &mut params, &mut dx, &mut col, &mut jit);
        for (row, p) in params.iter().enumerate().take(psi::H) {
            f(dx.as_ptr(), col.as_ptr(), row_of(&mut jit, row), p.as_ptr());
        }
        differ += out.iter().zip(jit.iter()).filter(|(a, b)| a != b).count();
        checked += out.len();
    }
    say!(info, "psi-probe: jit: floor kernel against the VM at four times: {differ} of {checked} bytes differ{}", if differ == 0 { " (the same bytes)" } else { "" });
    // Time: the VM's whole frame (frame, row and column steps included)
    // against the kernel over the 32 rows (the pixel stage alone), from
    // each region.
    let (v50, v90, _) = timed(|k| {
        let inputs = inputs_at(&prog, k * 20 % lp);
        psi::render_rows::<128>(&prog, &mut s, &inputs, &t, None, u32::MAX, out);
    });
    say!(info, "psi-probe: jit: floor VM frame p50 {v50} p90 {v90} us = {} ns a pixel (rows and all)", v50 * 1000 / 4096);
    floor_capture(&prog, &t, &mut s, lp / 3, slots, &mut params, &mut dx, &mut col, &mut jit);
    for (what, at) in regions {
        let Some(i) = at else { continue };
        let Some(a) = blocks[i].load(jitk::FLOOR) else { continue };
        // SAFETY: as above.
        let f: FloorFn = unsafe { std::mem::transmute::<usize, FloorFn>(a) };
        let (j50, j90, _) = timed(|_| {
            for (row, p) in params.iter().enumerate().take(psi::H) {
                f(dx.as_ptr(), col.as_ptr(), row_of(&mut jit, row), p.as_ptr());
            }
        });
        say!(info,
            "psi-probe: jit: floor kernel from {what} at {a:#x}: p50 {j50} p90 {j90} us a frame = {} ns a pixel ({}.{}x the VM frame)",
            j50 * 1000 / 4096, v50 / j50.max(1), (v50 * 10 / j50.max(1)) % 10
        );
    }
    drop(blocks);
    say!(info, "psi-probe: jit: done, exec heap {} B free, data heap {} B free", exec_heap().0, heap());
}

/// Racing the beam (console-c): the raced goldens and the split, then the
/// play lines (each with the beam's counters), for the integrator's
/// Formulas played by name. Built with `OPENDMD_PROBE_BEAM` set in the
/// environment (`make fw-release-variant FW_RELEASE_FEATURES=psi-probe`);
/// `make emu-probe` builds the full probe.
const BEAM_ONLY: bool = option_env!("OPENDMD_PROBE_BEAM").is_some();

pub fn run(root: &str) {
    if !open_report(root) {
        return;
    }
    let start = Instant::now();
    let stack0 = stack_unused();
    if BEAM_ONLY {
        psi::stats::CLOCK.store(ccount as fn() -> u32 as usize, Ordering::Relaxed);
        let mut out = vec![0u8; psi::W * psi::H * 3];
        beam::goldens(&mut out);
        beam::split(&mut out);
        play::start(format!("{root}/{REPORT_NAME}"));
        say!(info, "{DONE}: beam (play lines from now), main stack {} B unused (was {stack0}), heap {} B free, {} ms in all",
            stack_unused(), heap(), start.elapsed().as_millis());
        return;
    }
    if BUNDLED_ONLY {
        let mut canvas = Canvas::new();
        bundled(&mut canvas);
        say!(info, "{DONE}: bundled, main stack {} B unused (was {stack0}), heap {} B free, {} ms in all",
            stack_unused(), heap(), start.elapsed().as_millis());
        return;
    }
    if FASTDIV_ONLY {
        fastdiv_probe(start, stack0);
        return;
    }
    if CONSOLE_ONLY {
        console_probe(root, start, stack0);
        return;
    }
    #[cfg(feature = "jit")]
    if ERGO_ONLY {
        ergo_probe(root, start, stack0);
        return;
    }
    #[cfg(feature = "jit")]
    if ROADFIX_ONLY {
        roadfix_probe(root, start, stack0);
        return;
    }
    #[cfg(feature = "jit")]
    if CONSOLEB_ONLY {
        consoleb_probe(start, stack0);
        return;
    }
    if SWAP_ONLY {
        let mut canvas = Canvas::new();
        swap(root, &mut canvas);
        swap_card(root, &mut canvas);
        say!(info, "{DONE}: swap only, heap {} B free (largest {} B), lowest {} B, {} ms in all", heap(), crate::state::largest_free_block(), unsafe { esp_idf_sys::esp_get_minimum_free_heap_size() }, start.elapsed().as_millis());
        return;
    }
    if ROAD_ONLY {
        road_probe(root, start, stack0);
        return;
    }
    #[cfg(feature = "jit")]
    if JIT2_ONLY {
        jit2::probe(root, start, stack0);
        return;
    }
    if JIT_ONLY {
        say!(info, "psi-probe: start: main stack {stack0} B unused, heap {} B free (the JIT spike only)", heap());
        let mut out = vec![0u8; psi::W * psi::H * 3];
        jit_spike(&mut out);
        say!(info,
            "{DONE}: jitspike, main stack {} B unused (was {stack0}), heap {} B free (largest {} B), {} ms in all",
            stack_unused(), heap(), crate::state::largest_free_block(), start.elapsed().as_millis()
        );
        return;
    }
    say!(info,
        "psi-probe: start: main stack {stack0} B unused, heap {} B free, Program {} B, Scratch<128> {} B, Scratch<64> {} B, Ink {} B",
        heap(),
        std::mem::size_of::<psi::Program>(),
        std::mem::size_of::<psi::Scratch<128>>(),
        std::mem::size_of::<psi::Scratch<64>>(),
        std::mem::size_of::<psi::Ink>()
    );
    // What the probe is for first: the compiler's stack.
    // (The compile-stack worst cases are gone: the player never compiles;
    // the browser and `psi build` do, with the stacks they have.)
    // (Held through the sections below as before: `black_box`, or the
    // allocation would be elided until its first use.)
    let out = core::hint::black_box(vec![0u8; psi::W * psi::H * 3]);
    let mut canvas = Canvas::new();
    // Then its heap: every bundled Formula opens, a short heap refuses one.
    bundled(&mut canvas);
    // Formulas by reference, as the engine opens them.
    refs(root, &mut canvas);
    // A formula over a formula with the heap short, through the engine.
    swap(root, &mut canvas);
    // A card Formula over a card Formula: Neon city's check asks 59.6 KB
    // with nothing else on screen, so without the probe's 12 KB frame
    // buffer (the rig has 95 KB free with a GIF on screen; here 73 KB with
    // that buffer held). Taken again after, in the same place as a rule.
    drop(out);
    swap_card(root, &mut canvas);
    let mut out = vec![0u8; psi::W * psi::H * 3];

    // (The examples as effects, with and without the still cache, timed, are
    // not in the full probe: it is the emulator lane's correctness run, and
    // `bundled` already opens and draws every bundled Formula through the same
    // `Fx` path. For timing work use the `*_ONLY` modes: `ROAD_ONLY`,
    // `JIT2_ONLY` (the examples JIT against VM, with frame splits), `JIT_ONLY`,
    // `CONSOLE_ONLY`, `FASTDIV_ONLY`.)
    psi::stats::CLOCK.store(ccount as fn() -> u32 as usize, Ordering::Relaxed);
    fx_psi::set_still_gate(Some(crate::still_gate));

    speed2(&mut out);
    say!(info, "psi-probe: main stack {} B unused after the speed2 rows", stack_unused());
    compile_leak();
    // Golden frames: the host's and wasm's hashes, here (plain with the
    // still cache's variants, over the picture, drawn), then the same
    // through the JIT's kernels (the emulator lane's check of the
    // generated code), counted together.
    say!(info, "psi-probe: golden frames from {} B free", heap());
    let (mut ok, mut bad) = vm_goldens(&mut out, true);
    #[cfg(feature = "jit")]
    {
        let (o, b) = jit2::goldens(&mut out);
        ok += o;
        bad += b;
    }
    let mut under = vec![0u8; psi::W * psi::H * 3];
    psi::golden::picture(&mut under);
    say!(
        info,
        "psi-probe: golden frames: {ok} match the host/wasm hashes, {bad} differ{}",
        if bad == 0 { " (bit-identical)" } else { "" }
    );
    beam::goldens(&mut out);

    // The formula look as the main loop runs it (`FormulaLook::apply` over
    // a picture, every row: a frame of its own), with the heap its
    // buffers take and the stack after (the high-water mark).
    const LOOKS: [(&str, &str); 3] = [("Scanlines", "scanlines"), ("Old TV", "old-tv"), ("Corner clock", "corner-clock")];
    let mut pic = Canvas::new();
    pic.as_mut_bytes().copy_from_slice(&under);
    for (name, src) in LOOKS {
        let Ok(mut look) = fx_psi::FormulaLook::from_path(&fx_psi::path(bundled_src(src), "a=0.4")) else {
            say!(error, "psi-probe: look {name}: refused");
            continue;
        };
        let h0 = heap();
        let took = look.take();
        let h1 = heap();
        // (One apply over every row, a frame of its own; the timed run of
        // them is the `*_ONLY` modes' job, `JIT2_ONLY` for the looks.)
        look.redraw();
        pic.dirty_mut().mark_all();
        look.apply(Duration::from_millis(20), &mut pic, 0).dirty_mut().clear();
        say!(info,
            "psi-probe: look {name}: est {} us, buffers {} ({h0} -> {h1} B free), main stack {} B unused",
            look.program().esp32_us(),
            if took { "taken" } else { "REFUSED" },
            stack_unused()
        );
    }

    let stack1 = stack_unused();
    say!(info,
        "{DONE}: main stack {stack1} B unused (was {stack0} before the probe; the high-water mark is the deepest so far), heap {} B free, {} ms in all",
        heap(),
        start.elapsed().as_millis()
    );
    // (No `play::start` here: the play lines are timing on the panel, started
    // by the `*_ONLY` modes -- `ROAD_ONLY`, `JIT2_ONLY` -- for the rig.)
    *REPORT.lock().unwrap() = None;
    let _ = root;
}

/// The JIT's rows (docs/research/psi-jit-esp32.md, "Phase 2 as built"):
/// the goldens through kernels (also in the full probe, for the emulator
/// lane), then one source at a time, each made, run and dropped before the
/// next: the examples as effects JIT against VM with their frame splits,
/// the looks over a changing picture JIT against VM.
#[cfg(feature = "jit")]
pub(crate) mod jit2 {
    use super::*;

    const TUNNEL: &str = include_str!("../../opendmd-web/formulas/tunnel.psi");
    const FLOOR: &str = include_str!("../../opendmd-web/formulas/rotozoom-floor.psi");
    const ROTOZOOM: &str = include_str!("../../opendmd-web/formulas/rotozoom.psi");
    const STARFIELD: &str = include_str!("../../opendmd-web/formulas/starfield.psi");
    const CUBE: &str = include_str!("../../opendmd-web/formulas/wireframe-cube.psi");
    const PLASMA_LETTERS: &str = include_str!("../../opendmd-web/formulas/plasma-letters.psi");
    const OLD_TV: &str = include_str!("../../opendmd-web/formulas/old-tv.psi");
    const PLASMA: &str = include_str!("../../opendmd-web/formulas/plasma.psi");
    const FIRE: &str = include_str!("../../opendmd-web/formulas/fire.psi");
    // The road as bundled before road-full (the bundled one is Full since).
    const ROAD_BOTH: &str = include_str!("../../opendmd-web/experiments/road-life/road-both.psi");

    fn exec_line(what: &str) {
        let (free, big) = exec_heap();
        say!(info, "psi-probe: jit2: {what}: exec heap {free} B free, largest block {big} B; data heap {} B free", heap());
    }

    /// The golden frames through kernels against the host's hashes: plain
    /// (and with the still cache kept, then drawn from), over the picture,
    /// drawn (with a canvas); each kernel also self-checked against the
    /// VM's frame. (matched, differing).
    pub(crate) fn goldens(out: &mut [u8]) -> (u32, u32) {
        let t = fx_psi::tables();
        let mut s = zeroed::<psi::Scratch<128>>();
        s.reset();
        let mut prog = zeroed::<psi::Program>();
        let mut under = vec![0u8; psi::W * psi::H * 3];
        psi::golden::picture(&mut under);
        let mut ink = psi::Ink::boxed();
        let (mut ok, mut bad, mut vm) = (0u32, 0u32, 0u32);
        let mut why: Vec<(&str, u32)> = Vec::new();
        let mut code = 0usize;
        let plain = psi::golden::FRAMES.iter().map(|&(n, src, ms, want)| (n, src, ms, false, want, false));
        let over = psi::golden::OVER.iter().map(|&(n, src, ms, o, want)| (n, src, ms, o, want, false));
        let drawn = psi::golden::DRAWN.iter().map(|&(n, src, ms, o, want)| (n, src, ms, o, want, true));
        for (name, src, ms, o, want, draws) in plain.chain(over).chain(drawn) {
            if compile(src, &mut prog, true, true).is_err() {
                say!(error, "psi-probe: jit2: golden {name}: does not compile");
                bad += 1;
                continue;
            }
            let mut k = match psi::jit::emit(&prog) {
                Ok(k) => k,
                Err(w) => {
                    vm += 1;
                    match why.iter_mut().find(|(x, _)| *x == w) {
                        Some((_, n)) => *n += 1,
                        None => why.push((w, 1)),
                    }
                    continue;
                }
            };
            code += k.code_bytes();
            let inputs = psi::golden::inputs(ms);
            let u = o.then_some(under.as_slice());
            let mut passes: Vec<(&str, u64, bool)> = Vec::new();
            let drew = k.render(&prog, &mut s, draws.then_some(&mut *ink), None, &inputs, &t, u, u32::MAX, out, None);
            passes.push(("kernel", psi::golden::fnv(out), drew));
            if prog.still_lanes() > 0 {
                let mut st = psi::Still::new();
                if st.take(&prog) {
                    for pass in ["kernel, still kept", "kernel, still cached"] {
                        let drew = k.render(&prog, &mut s, None, Some(&mut st), &inputs, &t, u, u32::MAX, out, None);
                        passes.push((pass, psi::golden::fnv(out), drew));
                    }
                }
            }
            for (pass, got, drew) in passes {
                if got == want {
                    ok += 1;
                } else {
                    bad += 1;
                    say!(error, "psi-probe: jit2: golden {name} ({pass}{}) at {ms} ms: {got:#018x}, host/wasm {want:#018x}",
                        if drew { "" } else { ", the VM drew it: a checked frame" });
                }
            }
            // The player's self-check against the VM's frame.
            psi::render_rows_ink::<128>(&prog, &mut s, draws.then_some(&mut *ink), &inputs, &t, u, u32::MAX, out);
            if let Err((row, x)) = k.check(&prog, &mut s, draws.then_some(&mut *ink), &inputs, &t, u, out) {
                bad += 1;
                say!(error, "psi-probe: jit2: golden {name}: the self-check found row {row} differing at x = {x}");
            }
        }
        say!(info,
            "psi-probe: jit2: golden frames through kernels: {ok} match the host/wasm hashes, {bad} differ{}; {vm} on the VM {why:?}; {code} B of code in all",
            if bad == 0 { " (bit-identical)" } else { "" }
        );
        (ok, bad)
    }

    /// One example as an effect on the real path (`Fx::parse`; the first
    /// 12 frames untimed: the still cache, the kernel made, checked and
    /// trialed), the JIT on and then off: frame p50/p90/max, the pixels
    /// marked for the blit.
    pub(crate) fn effect(name: &str, src: &str, opts: &str, canvas: &mut Canvas) {
        let mut row = Vec::new();
        for jit in [true, false] {
            fx_psi::set_jit(jit);
            fx_psi::set_still_gate(None);
            let h0 = heap();
            let Some(mut fx) = Fx::parse(&fx_psi::path(src, opts)) else {
                say!(error, "psi-probe: jit2: example {name}: did not open");
                break;
            };
            let delay = fx.delay();
            let n = fx.loop_frames();
            for k in 0..12 {
                fx.frame(delay * (k % n), canvas);
                canvas.dirty_mut().clear();
            }
            let (on, why, bytes) = fx_psi::jit_status();
            let held = fx_psi::still_held();
            let h1 = heap();
            let mut marked = 0u64;
            let (e50, e90, emax) = timed(|k| {
                fx.frame(delay * ((k + 12) % n), canvas);
                marked += u64::from(canvas.dirty().count());
                canvas.dirty_mut().clear();
            });
            drop(fx);
            let what = if !jit { "vm".to_string() } else if on { format!("kernel {bytes} B") } else { format!("vm: {why}") };
            say!(info,
                "psi-probe: jit2: example {name} as an effect ({what}): p50 {e50} p90 {e90} max {emax} us a frame, {} px marked a frame, still {held} B, heap {h0} -> {h1} B -> {} B after",
                marked / u64::from(FRAMES), heap()
            );
            row.push(e50);
            // (A `rate` formula's frames this run: held, by the kernel,
            // from nothing, past the asked grid.)
            let r: Vec<u32> = fx_psi::RATE_FRAMES.iter().map(|c| c.swap(0, Ordering::Relaxed)).collect();
            if r.iter().any(|&n| n > 0) {
                say!(info, "psi-probe: jit2: example {name} ({what}): rate frames {} held, {} by the kernel, {} from nothing, {} past the asked grid", r[0], r[1], r[2], r[3]);
            }
        }
        if let [j, v] = row[..] {
            say!(info, "psi-probe: jit2: example {name}: JIT {j} us against VM {v} us = {}.{:02}x", v / j.max(1), (v * 100 / j.max(1)) % 100);
        }
        fx_psi::set_jit(true);
        fx_psi::set_still_gate(Some(crate::still_gate));
    }

    /// Where a kernel's frame goes: the example drawn directly through its
    /// kernel (its canvas if it draws; the still cache when it has values
    /// for one), split by `psi::jit::stats` (µs a frame), against the VM's
    /// direct frame; and the estimates.
    fn split(name: &str, src: &str) {
        let mut prog = zeroed::<psi::Program>();
        if compile(src, &mut prog, true, true).is_err() {
            return;
        }
        let Ok(mut k) = psi::jit::emit(&prog) else { return };
        let t = fx_psi::tables();
        let mut s = zeroed::<psi::Scratch<128>>();
        s.reset();
        let mut out = vec![0u8; psi::W * psi::H * 3];
        let mut ink = prog.draws().then(psi::Ink::boxed);
        let mut st = psi::Still::new();
        let still = prog.still_lanes() > 0 && st.take(&prog);
        let lp = prog.loop_ms().max(1);
        let inputs = |n: u32| {
            let mut i = psi::golden::inputs(0);
            i.set_ms(&prog, u64::from(n * 20 % lp));
            i
        };
        k.render(&prog, &mut s, ink.as_deref_mut(), if still { Some(&mut st) } else { None }, &inputs(0), &t, None, u32::MAX, &mut out, None);
        let _ = psi::jit::stats::take();
        let (k50, _, _) = timed(|n| {
            k.render(&prog, &mut s, ink.as_deref_mut(), if still { Some(&mut st) } else { None }, &inputs(n), &t, None, u32::MAX, &mut out, None);
        });
        let (cyc, frames) = psi::jit::stats::take();
        let (v50, _, _) = timed(|n| {
            psi::render_rows_still::<128>(&prog, &mut s, ink.as_deref_mut(), if still { Some(&mut st) } else { None }, &inputs(n), &t, None, u32::MAX, &mut out);
        });
        let parts: Vec<String> = psi::jit::stats::NAMES
            .iter()
            .zip(cyc)
            .map(|(n, c)| format!("{n} {}", c / 240 / frames.max(1)))
            .collect();
        say!(info,
            "psi-probe: jit2: split {name} (direct, still {still}): kernel frame p50 {k50} us = {}; VM frame p50 {v50} us; est vm {} jit {} (still: vm {} jit {}); kernel {} B at {:#x}",
            parts.join(", "), prog.esp32_us(), prog.esp32_us_jit(), prog.esp32_us_still(), prog.esp32_us_jit_still(), k.code_bytes(), k.addr()
        );
    }

    /// A look over a changing picture (~700 pixels in 6 rows a frame, as a
    /// GIF frame changes them; its own frames every 40 ms when it moves),
    /// the JIT on and then off: apply p50/p90/max (the first 12 untimed:
    /// the kernel made, checked and trialed), pixels marked.
    fn look(name: &str, path: &str, canvas: &mut Canvas) {
        let mut row = Vec::new();
        for jit in [true, false] {
            fx_psi::set_jit(jit);
            let Ok(mut look) = fx_psi::FormulaLook::from_path(path) else {
                say!(error, "psi-probe: jit2: look {name}: did not open");
                return;
            };
            if !look.take() {
                say!(error, "psi-probe: jit2: look {name}: no room for its buffers ({} B free)", heap());
                return;
            }
            let mut u = vec![0u8; psi::W * psi::H * 3];
            psi::golden::picture(&mut u);
            for y in 0..psi::H {
                for x in 0..psi::W {
                    let p = (y * psi::W + x) * 3;
                    canvas.set(x, y, opendmd_core::canvas::Rgb::new(u[p], u[p + 1], u[p + 2]));
                }
            }
            let mut seed = 0x2545_f491u32;
            let mut frame = |k: u32, look: &mut fx_psi::FormulaLook, canvas: &mut Canvas| -> (u32, u32) {
                for _ in 0..700 {
                    seed ^= seed << 13;
                    seed ^= seed >> 17;
                    seed ^= seed << 5;
                    let y = (seed as usize >> 8) % 6 + (k as usize * 5) % 26;
                    let x = (seed as usize >> 16) % psi::W;
                    canvas.set(x, y, opendmd_core::canvas::Rgb::new(seed as u8, (seed >> 8) as u8, k as u8));
                }
                let t = Instant::now();
                let out = look.apply(Duration::from_millis(u64::from(k) * 40), canvas, 0);
                let us = t.elapsed().as_micros() as u32;
                let m = out.dirty().count();
                out.dirty_mut().clear();
                (us, m)
            };
            for k in 0..12 {
                frame(k, &mut look, canvas);
            }
            let (on, why, bytes) = fx_psi::look_jit_status();
            let mut us = Vec::with_capacity(FRAMES as usize);
            let mut marked = 0u64;
            for k in 12..12 + FRAMES {
                let (t, m) = frame(k, &mut look, canvas);
                us.push(t);
                marked += u64::from(m);
            }
            let (p50, p90, max) = pct(&mut us);
            let what = if !jit { "vm".to_string() } else if on { format!("kernel {bytes} B") } else { format!("vm: {why}") };
            say!(info,
                "psi-probe: jit2: look {name} over a changing picture ({what}): p50 {p50} p90 {p90} max {max} us a frame, {} px marked a frame, est vm {} jit {}",
                marked / u64::from(FRAMES), look.program().esp32_us(), look.program().esp32_us_jit()
            );
            row.push(p50);
        }
        if let [j, v] = row[..] {
            say!(info, "psi-probe: jit2: look {name}: JIT {j} us against VM {v} us = {}.{:02}x", v / j.max(1), (v * 100 / j.max(1)) % 100);
        }
        fx_psi::set_jit(true);
    }

    pub fn probe(root: &str, start: Instant, stack0: u32) {
        say!(info, "psi-probe: start: main stack {stack0} B unused, heap {} B free (the JIT's phase-2 rows only)", heap());
        psi::stats::CLOCK.store(ccount as fn() -> u32 as usize, Ordering::Relaxed);
        exec_line("start");
        let mut out = vec![0u8; psi::W * psi::H * 3];
        let (vo, vb) = vm_goldens(&mut out, false);
        let (ko, kb) = goldens(&mut out);
        // (The emulator lane's line: the VM's goldens and the kernels'.)
        say!(info,
            "psi-probe: golden frames: {} match the host/wasm hashes, {} differ{}",
            vo + ko, vb + kb, if vb + kb == 0 { " (bit-identical)" } else { "" }
        );
        drop(out);
        exec_line("after the goldens");
        let mut canvas = Canvas::new();
        let list: [(&str, &str, &str); 11] = [
            ("Endless road", bundled_src("endless-road"), ""),
            ("Endless road Both", ROAD_BOTH, ""),
            ("Starfield", STARFIELD, ""),
            ("Wireframe cube", CUBE, ""),
            ("Rotozoom", ROTOZOOM, ""),
            ("Plasma letters", PLASMA_LETTERS, ""),
            ("Rotozoom floor", FLOOR, ""),
            ("Tunnel", TUNNEL, ""),
            ("Time gate", bundled_src("time-gate"), ""),
            ("Fire", FIRE, ""),
            ("Plasma", PLASMA, ""),
        ];
        for (name, src, opts) in list {
            effect(name, src, opts, &mut canvas);
            split(name, src);
        }
        say!(info, "psi-probe: jit2: main stack {} B unused after the examples", stack_unused());
        exec_line("after the examples");
        let looks = [
            ("Old TV", fx_psi::path(OLD_TV, "")),
            ("Plasma letters", fx_psi::path(PLASMA_LETTERS, "")),
            ("Plasma (screen 40)", fx_psi::path(&psi::look::set(PLASMA, "screen", 40).unwrap_or_default(), "")),
        ];
        for (name, path) in &looks {
            look(name, path, &mut canvas);
        }
        exec_line("after the looks");
        say!(info,
            "{DONE}: jit2, main stack {} B unused (was {stack0}), heap {} B free (largest {} B), {} ms in all",
            stack_unused(), heap(), crate::state::largest_free_block(), start.elapsed().as_millis()
        );
        *REPORT.lock().unwrap() = None;
        play::start(format!("{root}/{REPORT_NAME}"));
    }
}

/// Racing the beam (console-c): the golden frames drawn in the panel's pair
/// order (VM, and through kernels), each pair told once and in order, the
/// same hashes as drawn at once; and the raced frame's split for the
/// bundled Formulas (a frame at once against raced with a sink that only
/// counts: the pair order's own cost; the writes are the play lines').
pub(crate) mod beam {
    use super::*;

    /// A sink that checks the pairs come once and in order.
    struct Order {
        next: usize,
        bad: bool,
        started: bool,
    }

    impl Order {
        fn tell(&mut self, p: usize) {
            // (The start, once, before the first pair.)
            if p == psi::BEAM_START {
                self.bad |= self.started || self.next != 0;
                self.started = true;
                return;
            }
            self.bad |= p != self.next || !self.started;
            self.next += 1;
        }
        fn done(&self) -> bool {
            !self.bad && self.next == psi::H / 2
        }
    }

    #[inline(never)]
    pub(crate) fn goldens(out: &mut [u8]) -> (u32, u32) {
        let t = fx_psi::tables();
        let mut s = zeroed::<psi::Scratch<128>>();
        s.reset();
        let mut prog = zeroed::<psi::Program>();
        let mut ink = psi::Ink::boxed();
        let (mut ok, mut bad, mut kernels) = (0u32, 0u32, 0u32);
        // (Raced frames have nothing under them: the goldens over a
        // picture are not the player's.)
        let plain = psi::golden::FRAMES.iter().map(|&(n, src, ms, want)| (n, src, ms, want, false));
        let drawn = psi::golden::DRAWN.iter().filter(|g| !g.3).map(|&(n, src, ms, _, want)| (n, src, ms, want, true));
        for (name, src, ms, want, draws) in plain.chain(drawn) {
            if heap_low("raced golden frames") {
                break;
            }
            if compile(src, &mut prog, true, true).is_err() {
                bad += 1;
                say!(error, "psi-probe: beam: golden {name}: does not compile");
                continue;
            }
            let inputs = psi::golden::inputs(ms);
            let mut order = Order { next: 0, bad: false, started: false };
            psi::render_rows_beam::<128>(&prog, &mut s, draws.then_some(&mut *ink), None, &inputs, &t, None, out, None,
                &mut |p, _, _, _| order.tell(p));
            let mut check = |pass: &str, got: u64, told: bool| {
                if got == want && told {
                    ok += 1;
                } else {
                    bad += 1;
                    say!(error, "psi-probe: beam: golden {name} ({pass}) at {ms} ms: {got:#018x}, host/wasm {want:#018x}, pairs {}",
                        if told { "in order" } else { "out of order" });
                }
            };
            check("VM", psi::golden::fnv(out), order.done());
            #[cfg(feature = "jit")]
            if let Ok(mut k) = psi::jit::emit(&prog) {
                let mut order = Order { next: 0, bad: false, started: false };
                k.render_beam(&prog, &mut s, draws.then_some(&mut *ink), None, &inputs, &t, None, out, None,
                    &mut |p, _, _, _| order.tell(p));
                check("kernel", psi::golden::fnv(out), order.done());
                kernels += 1;
            }
        }
        say!(info,
            "psi-probe: beam: raced golden frames: {ok} match the host/wasm hashes, {bad} differ{} ({kernels} through kernels)",
            if bad == 0 { " (bit-identical)" } else { "" }
        );
        (ok, bad)
    }

    /// p50 (µs) of `n` runs of `f`.
    fn p50(n: usize, mut f: impl FnMut()) -> u32 {
        let mut v: Vec<u32> = (0..n)
            .map(|_| {
                let t = Instant::now();
                f();
                t.elapsed().as_micros() as u32
            })
            .collect();
        v.sort_unstable();
        v[n / 2]
    }

    /// The bundled Formulas' frames at once against raced (a sink that
    /// only counts): VM, and kernel where one is made; p50 µs of 15 frames
    /// at moving times, each over the frame before (the compare runs).
    #[inline(never)]
    pub(crate) fn split(out: &mut [u8]) {
        const NAMES: [&str; 6] = ["endless-road", "big-clock", "time-gate", "starfield", "matrix", "glitter"];
        let t = fx_psi::tables();
        let mut s = zeroed::<psi::Scratch<128>>();
        s.reset();
        let mut prog = zeroed::<psi::Program>();
        let mut ink = psi::Ink::boxed();
        for name in NAMES {
            if compile(bundled_src(name), &mut prog, true, true).is_err() {
                say!(error, "psi-probe: beam: {name}: does not compile");
                continue;
            }
            let draws = prog.draws();
            let mut ms = 0u32;
            let mut ch = [0u32; psi::H];
            let mut at_once = |s: &mut psi::Scratch<128>, out: &mut [u8]| {
                ms += 10;
                psi::render_rows_changed::<128>(&prog, s, draws.then_some(&mut *ink), None, &psi::golden::inputs(ms), &t, None, u32::MAX, out, &mut ch);
            };
            let whole = p50(15, || at_once(&mut s, out));
            let mut ms = 0u32;
            let mut ch = [0u32; psi::H];
            let mut told = 0u32;
            let raced = p50(15, || {
                ms += 10;
                psi::render_rows_beam::<128>(&prog, &mut s, draws.then_some(&mut *ink), None, &psi::golden::inputs(ms), &t, None, out,
                    Some(&mut ch), &mut |p, _, _, _| told += u32::from(p != psi::BEAM_START));
            });
            // Before the rows (frame steps, draw stage) and the rows, over
            // 120 frames 8 ms apart: how much the start moves frame to frame
            // (beam-c: Starfield's late frames).
            let (mut pre, mut rows): (Vec<u32>, Vec<u32>) = (Vec::new(), Vec::new());
            for k in 0..120u32 {
                let t0 = Instant::now();
                let mut at = t0;
                psi::render_rows_beam::<128>(&prog, &mut s, draws.then_some(&mut *ink), None, &psi::golden::inputs(k * 8), &t, None, out,
                    Some(&mut ch), &mut |p, _, _, _| {
                        if p == psi::BEAM_START {
                            at = Instant::now();
                        }
                    });
                pre.push(at.duration_since(t0).as_micros() as u32);
                rows.push(at.elapsed().as_micros() as u32);
            }
            let spread = |v: &mut Vec<u32>| {
                v.sort_unstable();
                let n = v.len();
                format!("{}/{}/{}/{}", v[n / 10], v[n / 2], v[n * 9 / 10], v[n - 1])
            };
            #[allow(unused_mut)]
            let mut line = format!(
                "psi-probe: beam: {name}: VM at once {whole} us, raced {raced} us; raced p10/p50/p90/max us: before the rows {}, the rows {}",
                spread(&mut pre),
                spread(&mut rows)
            );
            #[cfg(feature = "jit")]
            if let Ok(mut k) = psi::jit::emit(&prog) {
                let mut ms = 0u32;
                let kw = p50(15, || {
                    ms += 10;
                    k.render(&prog, &mut s, draws.then_some(&mut *ink), None, &psi::golden::inputs(ms), &t, None, u32::MAX, out, Some(&mut ch));
                });
                let mut ms = 0u32;
                let kr = p50(15, || {
                    ms += 10;
                    k.render_beam(&prog, &mut s, draws.then_some(&mut *ink), None, &psi::golden::inputs(ms), &t, None, out,
                        Some(&mut ch), &mut |p, _, _, _| told += u32::from(p != psi::BEAM_START));
                });
                line.push_str(&format!("; kernel at once {kw} us, raced {kr} us"));
            }
            say!(info, "{line} ({told} pairs told)");
        }
    }
}

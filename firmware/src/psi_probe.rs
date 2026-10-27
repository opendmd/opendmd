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
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Examples (formulas.json) timed as looks.
const PLASMA_EX: &str = "loop 8\n-- three waves added together\nwave = sin(x*2 + t/8) + sin(y*1.5 - t/4) + sin(x*1.5 + y + t/8)\nhsv(wave/6 + t/8, 0.9, 1)";
const FIRE_EX: &str = "loop 4\n-- curl bends the flames, more and more as they rise\ncurl = sin(y*1.5 - t)*0.25 + sin(y*3 - t*2 + x*4)*0.15\nflames = sin(x*6 + curl)*0.5 + sin(x*11 - curl)*0.5\nheat = (1 - y)*(1.6 + flames*0.7) - 0.5\npal(fire, clamp(heat, 0, 0.95))";
const RINGS_EX: &str =
    "loop 4\n-- r: the distance from the middle\npal(rainbow, frac(r*1.5 - t/2))";
const STRIPES_EX: &str = "loop 4\n-- a: stripe width\nsize = 8 + a*24\nband = sin((px + py)/size - t/2)\ns = clamp(band*1.5 + 0.5, 0, 1)\nrgb(1, s, s)";
const COPPER_EX: &str = "loop 4\n-- each bar is brightest in its middle, in steps like the old chips\nred = floor(max(0, 1 - abs(cy - 0.35*sin(t/4))*5)*6)/6\ngold = floor(max(0, 1 - abs(cy - 0.35*sin(t/4 - 0.12))*5)*6)/6\nblue = floor(max(0, 1 - abs(cy - 0.35*sin(t/4 - 0.24))*5)*6)/6\nrgb(max(red, gold), max(gold*0.8, blue*0.6), blue)";
const OLD_PHOTO_EX: &str = "-- a: how much (0: the GIF as it is)\nmix(rgb(ur, ug, ub), rgb(ul*1.1 + 0.08, ul*0.92 + 0.04, ul*0.65), a)";
const OLD_TV_EX: &str = "loop 4\ndx = (x - 0.5)*1.6; dy = (y - 0.5)*1.2\nlines = py % 2 ? 1 : 0.6\nroll = 0.85 + 0.15*max(0, tri(y/2 + t/4))\ncorners = clamp(1.5 - (dx*dx + dy*dy)*1.6, 0, 1)\nlayer multiply { gray(lines * roll * corners) }";

/// The drawing examples (formulas.json), timed as effects.
const STARFIELD_EX: &str = "loop 2.4\nfps 100\n-- Starfield: 192 stars flying at you, a pass through the field a loop,\n-- each a streak from where it was a moment ago\n-- a: streak length\nframe { back = 1 + 1/64 + (144 + a*240)/4096 }\ndraw {\n  for k < 192 {\n    sx = hash(k, 7)*32 - 16; sy = hash(k, 3)*8 - 4  -- where it is, across and up\n    near = frac(t/2.4 - hash(k, 11))          -- 0 far, 1 at you\n    s = 1/(65/64 - near); st = 1/(back - near)  -- perspective, now and a moment ago\n    hx = sx*s; hy = sy*s                      -- the head, from the middle\n    -- a head off the panel: the line moved far below it, where it is skipped\n    row = abs(hx) > 64 or abs(hy) > 16 ? -999 : 16\n    lit = near*near                           -- squared: far stars stay dark longer\n    line(64 + sx*st, row + sy*st, 64 + hx, row + hy, lit*0.2, lit)\n  }\n}\npal(ice, ink)";
const TUNNEL_EX: &str = "loop 4.8\n-- Tunnel: the time warp. Each loop the walls turn once, travel two\n-- checker lengths, and the palette goes round once.\n-- From the vanishing point, the tunnel twice as tall as wide:\ndx = px - 63.5; dy = (15.5 - py)*2\n-- how far each pixel is from the black middle: 0 there (where the\n-- tunnel is deeper than 16), ~15 at the edges\n-- (this and `round` depend on the pixel alone: worked out once, kept)\nnear = 16 - min(128/len(dx, dy), 16)\n-- round the walls: 16 checker cells a turn, twisting with depth\ntwist = 1                              -- try 0 to 3\nround = atan2(dy, dx)*16 + (16 - near)*twist/8\n-- the checker, turning and travelling\nu = round + t/0.3; v = 16 + t/0.6 - near\ncell = (floor(u) + floor(v)) % 2\n-- lit by depth (full near, fading as a square, black in the middle) and\n-- by a light going round with the walls, in eight steps\nfade = min(near, 10)                    -- 10 near, to 0 where it turns black\nside = 0.0635 + 0.0159*cos(u/16)\nlight = floor(fade*fade*side + (near > 0))/8\npal(chrono, cell ? frac(0.711 + t/4.8) : frac(0.336 + t/4.8)) * light";
const ROTOZOOM_EX: &str = "loop 9.6\n-- Rotozoom: Mode 7. A checker turning once a loop, its zoom breathing\n-- twice, gliding two checkers across and one down; under it a noise\n-- texture turning the other way twice as fast; rows and columns wobble.\n-- a: wobble = gentle, strong, off\nframe {\n  sn = sin(t/9.6); cs = cos(t/9.6)                 -- the turn\n  s2 = -sin(t/4.8); c2 = cos(t/4.8)                -- the noise's, back twice\n  zoom = 2                                         -- try 1 to 4\n  sc = 1/((1 + sin(t/4.8)/2)*zoom*8)               -- checker cells a pixel\n  sc2 = sc*0.75\n  cu = t/0.6; cv = t/1.2                           -- the glide, in cells\n  amp = pick(floor(a*2 + 0.5), 3, 4.5, 0)/8        -- the wobble, in texels\n}\ndx = px - 64; dy = 15 - py\n-- the wobble: rows slide sideways, columns up and down, on travelling waves\nwob = sin((31 - py)/32 + t/3.2)*amp\ncol = sin(px*0.0234375 - t/4.8)*amp\n-- where each pixel is on the checker: its column's part plus its row's\nu = (cu + cs*sc*dx) + (wob - sn*sc*dy)\nv = (cv + sn*sc*dx + col) + cs*sc*dy\ncell = (floor(u) + floor(v)) % 2\nu2 = (cv + c2*sc2*dx) + (wob - s2*sc2*dy)\nv2 = (cu + s2*sc2*dx + col) + c2*sc2*dy\npal(fire, (cell ? 0.44 : 0.19) + noise(u2, v2, 0)*0.6)";
const ROTOZOOM_FLOOR_EX: &str = "loop 9.6\nfps 100\n-- Rotozoom floor: Mode 7 as a floor. A checker running away to a horizon\n-- at the top, turning once a loop and gliding, each row scaled by its\n-- distance and the far rows fogged; rows and columns wobble.\n-- a: wobble = gentle, strong, off\nframe {\n  sn = sin(t/9.6); cs = cos(t/9.6)                 -- the turn\n  zoom = 2                                         -- try 1 to 4\n  cu = t/0.6; cv = t/1.2                           -- the glide, in cells\n  amp = pick(floor(a*2 + 0.5), 3, 4.5, 0)/8        -- the wobble, in texels\n}\ndx = px - 64; ty = 31 - py\n-- a row's distance: checker cells a pixel, smaller further down\nrs = 1/((ty + 2)*zoom)\nwob = sin(ty/32 + t/3.2)*amp\ncol = sin(px*0.0234375 - t/4.8)*amp\n-- ahead by the row's distance, across by the pixel's\nu = cs*rs*dx + (cu + wob + sn*rs*64)\nv = sn*rs*dx + col + (cv - cs*rs*64)\ncell = (floor(u) + floor(v)) % 2\n-- the far rows fade into the fog (black at the horizon)\nfog = ty < 3 ? 0 : ty < 6 ? 0.375 : ty < 10 ? 0.625 : 1\npal(fire, cell ? 0.78 : 0.28) * fog";
const CUBE_EX: &str = "loop 4.8\nfps 100\n-- Wireframe cube: turning about two axes and swinging across; the near\n-- edges brighter. The colour is the last line's #40c0ff\n-- a: size\ntable V = -1,-1,-1, 1,-1,-1, 1,1,-1, -1,1,-1, -1,-1,1, 1,-1,1, 1,1,1, -1,1,1\ntable E = 0,1, 1,2, 2,3, 3,0, 4,5, 5,6, 6,7, 7,4, 0,4, 1,5, 2,6, 3,7\nframe {\n  sa = sin(t/4.8); ca = cos(t/4.8); sb = sin(t/4.8 + 1/6); cb = cos(t/4.8 + 1/6)\n  size = 34 + a*28; mid = 64 + sa*40\n}\ndraw {\n  for e < 12 {\n    m = E[e*2]*3; n = E[e*2 + 1]*3            -- the edge's two corners\n    x0 = V[m]*size; y0 = V[m + 1]*size; z0 = V[m + 2]*size\n    x1 = V[n]*size; y1 = V[n + 1]*size; z1 = V[n + 2]*size\n    xa = x0*ca + z0*sa; za = z0*ca - x0*sa; ya = y0*cb - za*sb; zb = y0*sb + za*cb  -- turned\n    xc = x1*ca + z1*sa; zc = z1*ca - x1*sa; yc = y1*cb - zc*sb; zd = y1*sb + zc*cb\n    line(mid + xa*36/(zb + 128), 16 + ya*36/(zb + 128), mid + xc*36/(zd + 128), 16 + yc*36/(zd + 128),\n         clamp(0.594 - zb/148, 0.156, 1), clamp(0.594 - zd/148, 0.156, 1))\n  }\n}\n#40c0ff * ink";

/// The report's file name at the card's root.
pub const REPORT_NAME: &str = "filelist-psi-probe.txt";

/// The report, open for appending while the probe runs.
static REPORT: Mutex<Option<std::fs::File>> = Mutex::new(None);

/// Log a line and append it to the report on the card at once (synced:
/// a crash after it leaves it there).
macro_rules! say {
    ($level:ident, $($arg:tt)*) => {{
        let line = format!($($arg)*);
        log::$level!("{line}");
        if let Ok(mut r) = REPORT.lock() {
            if let Some(f) = r.as_mut() {
                use std::io::Write;
                let _ = writeln!(f, "{line}");
                let _ = f.sync_data();
            }
        }
    }};
}

/// The line a finished probe ends its report with.
const DONE: &str = "psi-probe: done";

/// The compile threads' stack for the worst cases: the host's high-water
/// for the deepest is ~16 KB (bigger frames than the ESP32's ~10 KB).
const WORST_STACK: usize = 24 * 1024;

/// What a worst case must leave of the player's compile stack before the
/// probe compiles it there (an overflow there is a crash).
const PSI_MARGIN: u32 = 1024;

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
    let mut ink = psi::Ink::boxed();
    let t = fx_psi::tables();
    timed(|k| {
        let inputs = psi::golden::inputs(k * 20 % loop_ms);
        psi::render_rows_ink::<N>(prog, &mut s, Some(&mut ink), &inputs, &t, under, u32::MAX, out);
    })
}

/// A golden frame again laid out without the column and still runs, and
/// (when it has values for one) drawn with a still cache, kept on one frame
/// and drawn from on the next: (matches, differences). `prog` is left
/// compiled as usual.
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
    if psi::compile_layout(src, &mut plain, false).is_ok() {
        psi::render_with::<128>(&plain, s, &psi::golden::inputs(ms), &t, out);
        let got = psi::golden::fnv(out);
        if got == want {
            ok += 1;
        } else {
            bad += 1;
            say!(error, "psi-probe: golden {name} (plain) at {ms} ms: {got:#018x}, host/wasm {want:#018x}");
        }
    }
    let _ = psi::compile_into(src, prog);
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

/// Frames of `prog` with a still cache (kept on the first, untimed), as
/// [`lanes`].
fn lanes_still(prog: &psi::Program, loop_ms: u32, out: &mut [u8]) -> Option<(u32, u32, u32)> {
    let mut s = zeroed::<psi::Scratch<128>>();
    s.reset();
    let mut st = psi::Still::new();
    if !st.take(prog) {
        return None;
    }
    let t = fx_psi::tables();
    let inputs = psi::golden::inputs(0);
    psi::render_rows_still::<128>(prog, &mut s, None, Some(&mut st), &inputs, &t, None, u32::MAX, out);
    Some(timed(|k| {
        let inputs = psi::golden::inputs(k * 20 % loop_ms);
        psi::render_rows_still::<128>(prog, &mut s, None, Some(&mut st), &inputs, &t, None, u32::MAX, out);
    }))
}

/// The compiler's own stack on the worst inputs (`psi::worst`, the deepest
/// last): each compiled on a thread of its own with a [`WORST_STACK`] stack,
/// reporting the bytes it used; then on the player's compile stack and
/// through `Fx::parse` only those that fit it with [`PSI_MARGIN`] to spare;
/// then, only if the deepest plus what the main task already uses leaves a
/// 2 KB margin, the deepest again on the main task itself.
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
    let mut used_by: Vec<u32> = Vec::new();
    for (name, src) in &srcs {
        let src = src.clone();
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
                used_by.push(used);
                say!(info,
                    "psi-probe: worst {name}: {len} B source, compile used {used} B of a thread's stack ({})",
                    err.as_deref().unwrap_or("ok")
                );
            }
            _ => {
                used_by.push(u32::MAX);
                say!(
                    error,
                    "psi-probe: worst {name}: no thread (heap {} B free)",
                    heap()
                )
            }
        }
    }
    let fits = |k: usize| used_by[k] as usize + PSI_MARGIN as usize <= crate::config::PSI_COMPILE_STACK;
    // Through `psi_stack` itself, on its own thread (the size the player
    // uses, pinned and prioritised as it does): the compile time (what a
    // frame deadline sees when a formula opens) and the bytes of that stack
    // never touched — the real headroom, the thread's entry frames and any
    // interrupt frames pushed on it included.
    for (k, (name, src)) in srcs.iter().enumerate() {
        if !fits(k) {
            say!(warn,
                "psi-probe: worst {name} on psi_stack ({} B): NOT RUN, it needs {} B (+{PSI_MARGIN} B margin)",
                crate::config::PSI_COMPILE_STACK,
                used_by[k]
            );
            continue;
        }
        let mut prog = zeroed::<psi::Program>();
        let r = crate::psi_stack::run(|| {
            let t0 = Instant::now();
            let ok = psi::compile_into(src, &mut prog).is_ok();
            (t0.elapsed().as_micros(), ok, stack_unused())
        });
        match r {
            Some((us, ok, free)) => say!(info,
                "psi-probe: worst {name} on psi_stack ({} B): compile {us} us ({}), {free} B of its stack never touched",
                crate::config::PSI_COMPILE_STACK,
                if ok { "ok" } else { "an error" }
            ),
            None => say!(error, "psi-probe: worst {name} on psi_stack: no thread (heap {} B free)", heap()),
        }
    }
    // The deepest through the real path (`Fx::parse`, as the engine and the
    // checks before saving open a formula): compiled on its own stack
    // (`psi_stack`), so the main task's mark should not move.
    for (k, (name, src)) in srcs.iter().enumerate() {
        if !fits(k) {
            continue;
        }
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
    if deepest > 0 && deepest != u32::MAX && deepest + 2048 < unused {
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

/// The probe, when the card at `root` is the main task's (see the module
/// docs).
pub fn run(root: &str) {
    if !open_report(root) {
        return;
    }
    let start = Instant::now();
    let stack0 = stack_unused();
    say!(info,
        "psi-probe: start: main stack {stack0} B unused, heap {} B free, Program {} B, Scratch<128> {} B, Scratch<64> {} B, Ink {} B",
        heap(),
        std::mem::size_of::<psi::Program>(),
        std::mem::size_of::<psi::Scratch<128>>(),
        std::mem::size_of::<psi::Scratch<64>>(),
        std::mem::size_of::<psi::Ink>()
    );
    // What the probe is for first: the compiler's stack.
    worst_stack();
    let mut out = vec![0u8; psi::W * psi::H * 3];
    let mut canvas = Canvas::new();

    // The examples as effects, first while the heap is at its fullest (the
    // still cache is 16-32 KB; p2probe-a ran these last, ~38 KB free, and no
    // 16 KB block was left for it). The real path: `Fx::parse`, the canvas taken
    // at open, `Fx::frame`, the still cache taken on the first frame): with
    // the cache, then with the gate refusing it (every step every frame).
    for (name, src, opts) in [
        ("Starfield", STARFIELD_EX, "a=0.5"),
        ("Wireframe cube", CUBE_EX, "a=0.5"),
        ("Tunnel", TUNNEL_EX, ""),
        ("Rotozoom", ROTOZOOM_EX, ""),
        ("Rotozoom floor", ROTOZOOM_FLOOR_EX, ""),
    ] {
        // (Into a zeroed box: a `Program` by value is 2.8 KB of the main
        // task's stack.)
        let mut prog = zeroed::<psi::Program>();
        let (est, est_still) = if psi::compile_into(src, &mut prog).is_ok() {
            (prog.esp32_us(), prog.esp32_us_still())
        } else {
            (0, 0)
        };
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
            fx.frame(Duration::ZERO, &mut canvas);
            let h2 = heap();
            let held = fx_psi::still_held();
            let (e50, e90, emax) = timed(|k| {
                fx.frame(delay * (k % n), &mut canvas);
            });
            drop(fx);
            say!(info,
                "psi-probe: example {name} as an effect ({gate}): p50 {e50} p90 {e90} max {emax} us a frame, est {est} us ({est_still} with the cache), heap {h0} -> {h1} B at open ({} B for the effect and its canvas), {h2} B after its first frame (still cache {held} B), {} B back after",
                h0.saturating_sub(h1),
                heap()
            );
        }
    }
    // The firmware's own gate again.
    fx_psi::set_still_gate(Some(crate::still_gate));


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
        // No N=64 run: its VM instantiation no longer fits a 4mb probe image.
        let (b50, b90, bmax) = (0, 0, 0);
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
        // The same frame without the column and still runs, and with the
        // still cache kept and then drawn from (the second frame).
        let (o, b) = variants(name, src, ms, &mut s, &mut prog, &mut out, want);
        ok += o;
        bad += b;
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
    // Formulas that draw (the draw stage's canvas, lines, loops, tables).
    let mut ink = psi::Ink::boxed();
    for &(name, src, ms, over, want) in psi::golden::DRAWN {
        if psi::compile_into(src, &mut prog).is_err() {
            bad += 1;
            say!(error, "psi-probe: golden {name}: does not compile");
            continue;
        }
        let u = over.then_some(under.as_slice());
        psi::render_rows_ink::<128>(&prog, &mut s, Some(&mut ink), &psi::golden::inputs(ms), &t, u, u32::MAX, &mut out);
        let got = psi::golden::fnv(&out);
        if got == want {
            ok += 1;
        } else {
            bad += 1;
            say!(
                error,
                "psi-probe: golden {name} (drawn) at {ms} ms: {got:#018x}, host/wasm {want:#018x}"
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
    const STEPS: [(&str, &str); 26] = [
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
        ("digit (+2 mask)", "gray(digit(px % 8, px % 4, y * 6))"),
        ("bit", "gray(bit(px, t))"),
        ("pick (+mul)", "gray(pick(x * 4, t, 0.5, 0.75, 1))"),
        ("pick 8 (+mul)", "gray(pick(x * 8, t, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7))"),
        ("chain 4 (+mul)", "k = x * 4\ngray(k < 0 ? 0 : k < 1 ? t : k < 2 ? 0.5 : k < 3 ? 0.75 : k < 4 ? 1 : 0)"),
        ("big clock", "loop 60\n-- Big clock: the time in four big digits, and a seconds bar along the bottom\nframe {\n  h1 = floor(hour/10); h2 = hour % 10       -- the hours' two digits\n  m1 = floor(minute/10); m2 = minute % 10   -- the minutes' two digits\n  end = (second + 1)*128/60                 -- where the seconds bar ends\n}\n-- a digit: the bold font's 3 x 5 pixels, each 8 px wide, 6 tall (24 x 30)\nq = px >= 64 ? px - 8 : px     -- the minutes sit 8 px further right\nk = floor(q/32)                -- which digit: one every 32 px (0 1, then 2 3)\nn = pick(k, h1, h2, m1, m2)    -- and its value\ncol = floor(q % 32 / 8)        -- the font's column, 0 to 2 (then the gap)\nrow = floor(ty/6)              -- its row from the top, 0 to 4 (5: below)\nbar = py < 1                   -- the bottom row is the seconds bar\nlit = bar ? px < end : digit(n, col, row, bold)\n-- a rainbow drifting through the digits, tilted by row; the bar is white\nhue = py/64 + hour/24 + t/10\nhsv(x + hue, bar ? 0 : 1, lit)"),
        ("big clock (beta.10)", "loop 60\n-- Big clock: the time in four big digits, and a seconds bar along the bottom\nframe {\n  h1 = floor(hour/10); h2 = hour % 10       -- the hours' two digits\n  m1 = floor(minute/10); m2 = minute % 10   -- the minutes' two digits\n  end = (second + 1)*128/60                 -- where the seconds bar ends\n}\n-- a digit: the bold font's 3 x 5 pixels, each 8 px wide, 6 tall (24 x 30)\nq = px >= 64 ? px - 8 : px     -- the minutes sit 8 px further right\nk = floor(q/32)                -- which digit: one every 32 px (0 1, then 2 3)\nn = k < 1 ? h1 : k < 2 ? h2 : k < 3 ? m1 : m2   -- and its value\ncol = floor(q % 32 / 8)        -- the font's column, 0 to 2 (then the gap)\nrow = floor((31 - py)/6)       -- its row from the top, 0 to 4 (5: below)\nbar = py < 1                   -- the bottom row is the seconds bar\nlit = bar ? px < end : digit(n, col, row, bold)\n-- a rainbow drifting through the digits, tilted by row; the bar is white\nhue = py/64 + hour/24 + t/10\nhsv(x + hue, bar ? 0 : 1, lit)"),
        ("tab (+mul +add)", "table T = 0.1, 0.5, 0.9, 0.3\ngray(T[x * 4] + t)"),
        ("ink (+1 dot)", "draw { dot(-1, -1, t) }\ngray(ink)"),
        ("out hsv", "hsv(x, 1, 1)"),
        ("out pal", "pal(fire, x)"),
        // Phase 2: noise at a whole z (four corners), a scaled palette.
        ("noise flat (+mul)", "gray(noise(x * 8, y, 3))"),
        ("out pal * k (+mul)", "pal(fire, x) * (x * 0.9)"),
        ("out pal * k=1", "pal(fire, x) * (t * 0 + 1)"),
    ];
    // Layers and what is under (drawn over the golden picture): the
    // formula look's costs, for `Program::esp32_us`.
    const LAYER_STEPS: [(&str, &str); 20] = [
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
        // Covered blends (a look's amount) and a row colour (`fill`).
        ("+layer add 0.4", "layer { gray(x) }\nlayer add { gray(x + t, 0.4) }"),
        ("+layer multiply 0.4", "layer { gray(x) }\nlayer multiply { gray(x + t, 0.4) }"),
        ("+layer screen 0.4", "layer { gray(x) }\nlayer screen { gray(x + t, 0.4) }"),
        ("+layer max 0.4", "layer { gray(x) }\nlayer max { gray(x + t, 0.4) }"),
        ("+layer min 0.4", "layer { gray(x) }\nlayer min { gray(x + t, 0.4) }"),
        ("+layer screen row", "layer { gray(x) }\nlayer screen { gray(y + t) }"),
        ("+layer screen hsv", "layer { gray(x) }\nlayer screen { hsv(x + t, 1, 1) }"),
        ("screen 0.4 on picture", "layer screen { gray(x + t, 0.4) }"),
        ("hsv 0.3 on picture", "hsv(x + t, 1, 1, 0.3)"),
        ("screen hsv 0.4 on picture", "layer screen { hsv(x + t, 1, 1, 0.4) }"),
    ];
    let steps = STEPS
        .iter()
        .map(|&(w, s)| (w, s, false))
        .chain(LAYER_STEPS.iter().map(|&(w, s)| (w, s, true)));
    for (what, src, over) in steps {
        // (Laid out without the column and still runs: a step's cost on
        // every pixel, as the table was measured.)
        if psi::compile_layout(src, &mut prog, false).is_err() {
            say!(error, "psi-probe: step {what}: does not compile");
            continue;
        }
        let u = over.then_some(under.as_slice());
        let (p50, p90, _) = lanes_over::<128>(&prog, 8000, u, &mut out);
        let (_, row, px) = prog.counts();
        say!(info,
            "psi-probe: step {what:<18} pixel steps {px} (+{row} row): p50 {p50} p90 {p90} us a frame = {} ns a pixel, est {} us",
            p50 as u64 * 1000 / 4096,
            prog.esp32_us()
        );
    }

    // The draw stage: frames against the one-dot baseline, per turn (or per
    // line), for `Program::draw_us`'s constants (calibrated from these on
    // 2026-10-27: a step run 420 ns, a dot 400, a line 1,100 + 320 a pixel;
    // tests/measured.rs `DRAW`).
    const DRAW_STEPS: [(&str, &str, u32); 7] = [
        ("draw base (1 dot off)", "draw { dot(-1, -1, t) }\ngray(ink)", 1),
        ("draw loop step: 1024 dots off", "draw { for k < 1024 { dot(-1, -1, t) } }\ngray(ink)", 1024),
        ("draw loop step: +add", "draw { for k < 1024 { dot(-1, -1, k + t) } }\ngray(ink)", 1024),
        ("draw loop step: +hash", "draw { for k < 1024 { dot(-1, -1, hash(k, t)) } }\ngray(ink)", 1024),
        ("draw dot on panel x1024", "draw { for k < 1024 { dot(k % 128, 5 + t) } }\ngray(ink)", 1024),
        ("draw line 12 x256", "draw { for k < 256 { line(10, 5, 21, 5 + t) } }\ngray(ink)", 256),
        ("draw line 100 x64", "draw { for k < 64 { line(10, 5, 109, 5 + t) } }\ngray(ink)", 64),
    ];
    let mut base = 0u32;
    for (what, src, n) in DRAW_STEPS {
        if psi::compile_layout(src, &mut prog, false).is_err() {
            say!(error, "psi-probe: step {what}: does not compile");
            continue;
        }
        let (p50, p90, _) = lanes_over::<128>(&prog, 8000, None, &mut out);
        if n == 1 {
            base = p50;
        }
        say!(info,
            "psi-probe: step {what:<30} p50 {p50} p90 {p90} us a frame = {} ns a turn over the base, est {} us (draw {} us)",
            u64::from(p50.saturating_sub(base)) * 1000 / u64::from(n),
            prog.esp32_us(),
            prog.draw_us()
        );
    }
    // Phase 2's runs: the column run (once a frame a column) and the still
    // cache (kept values copied in), against the same formula laid out
    // plain, for `Program::esp32_us`'s column and STILL_COPY_NS figures.
    const RUNS: [(&str, &str); 4] = [
        ("column sin (+3)", "gray(sin(x * 3 + t) * 0.5 + 0.5)"),
        ("column 8 steps", "v = sin(x * 3 + t) + cos(x * 5 - t) + tri(x + t) + saw(x * 2)\ngray(v * 0.25 + 0.5)"),
        ("still 1: 1/len", "gray(frac(1 / len(px - 63.5, (15.5 - py) * 2) * 8 + t))"),
        ("still 2: 1/len, atan2", "d = 1 / len(px - 63.5, (15.5 - py) * 2)\ngray(frac(d * 8 + t) * frac(atan2(15.5 - py, px - 63.5) * 4 + t))"),
    ];
    for (what, src) in RUNS {
        let mut plain = zeroed::<psi::Program>();
        if psi::compile_layout(src, &mut plain, false).is_err() || psi::compile_into(src, &mut prog).is_err() {
            say!(error, "psi-probe: run {what}: does not compile");
            continue;
        }
        let (q50, _, _) = lanes::<128>(&plain, 8000, &mut out);
        let (r50, r90, _) = lanes::<128>(&prog, 8000, &mut out);
        let h0 = heap();
        let still = lanes_still(&prog, 8000, &mut out);
        let line = match still {
            Some((s50, s90, _)) => format!(
                "with the still cache ({} B, {h0} B free before) p50 {s50} p90 {s90} us, est {} us",
                psi::Still::bytes_for(&prog),
                prog.esp32_us_still()
            ),
            None if prog.still_lanes() > 0 => format!("still cache refused ({h0} B free)"),
            None => String::from("no still values"),
        };
        say!(info,
            "psi-probe: run {what:<22} plain p50 {q50} us (est {}) | runs p50 {r50} p90 {r90} us (est {}, {} pixel steps) | {line}",
            plain.esp32_us(),
            prog.esp32_us(),
            prog.counts().2
        );
    }


    // Formulas as looks (`psi::look`, as the web app composes them): the
    // VM's frame over a picture against the estimate the looks' limit reads.
    const AS_LOOKS: [(&str, &str, &str, u8); 12] = [
        ("Plasma", PLASMA_EX, "screen", 40),
        ("Plasma", PLASMA_EX, "over", 30),
        ("Plasma", PLASMA_EX, "add", 40),
        ("Plasma", PLASMA_EX, "multiply", 100),
        ("Plasma", PLASMA_EX, "max", 50),
        ("Fire", FIRE_EX, "screen", 40),
        ("Fire", FIRE_EX, "over", 30),
        ("Rings", RINGS_EX, "screen", 40),
        ("Stripes", STRIPES_EX, "screen", 40),
        ("Copper bars", COPPER_EX, "screen", 40),
        ("Old photo", OLD_PHOTO_EX, "screen", 50),
        ("Old TV", OLD_TV_EX, "over", 100),
    ];
    for (name, src, blend, k) in AS_LOOKS {
        let Ok(look) = psi::look::compose(src, blend, k) else {
            say!(
                error,
                "psi-probe: as look {name} {blend} {k}: does not compose"
            );
            continue;
        };
        if psi::compile_into(&look, &mut prog).is_err() {
            say!(
                error,
                "psi-probe: as look {name} {blend} {k}: does not compile"
            );
            continue;
        }
        let (p50, p90, max) =
            lanes_over::<128>(&prog, prog.loop_ms(), Some(under.as_slice()), &mut out);
        say!(info,
            "psi-probe: as look {name} {blend} {k}%: p50 {p50} p90 {p90} max {max} us a frame, est {} us",
            prog.esp32_us()
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

    let stack1 = stack_unused();
    say!(info,
        "{DONE}: main stack {stack1} B unused (was {stack0} before the probe; the high-water mark is the deepest so far), heap {} B free, {} ms in all",
        heap(),
        start.elapsed().as_millis()
    );
    *REPORT.lock().unwrap() = None;
}

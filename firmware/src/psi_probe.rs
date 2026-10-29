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

/// Examples (opendmd-web/formulas/, as they were) timed as looks.
const PLASMA_EX: &str = "loop 8\n-- three waves added together\nwave = sin(x*2 + t/8) + sin(y*1.5 - t/4) + sin(x*1.5 + y + t/8)\nhsv(wave/6 + t/8, 0.9, 1)";
const FIRE_EX: &str = "loop 4\n-- curl bends the flames, more and more as they rise\ncurl = sin(y*1.5 - t)*0.25 + sin(y*3 - t*2 + x*4)*0.15\nflames = sin(x*6 + curl)*0.5 + sin(x*11 - curl)*0.5\nheat = (1 - y)*(1.6 + flames*0.7) - 0.5\npal(fire, clamp(heat, 0, 0.95))";
const RINGS_EX: &str =
    "loop 4\n-- r: the distance from the middle\npal(rainbow, frac(r*1.5 - t/2))";
const STRIPES_EX: &str = "loop 4\n-- a: stripe width\nsize = 8 + a*24\nband = sin((px + py)/size - t/2)\ns = clamp(band*1.5 + 0.5, 0, 1)\nrgb(1, s, s)";
const COPPER_EX: &str = "loop 4\n-- each bar is brightest in its middle, in steps like the old chips\nred = floor(max(0, 1 - abs(cy - 0.35*sin(t/4))*5)*6)/6\ngold = floor(max(0, 1 - abs(cy - 0.35*sin(t/4 - 0.12))*5)*6)/6\nblue = floor(max(0, 1 - abs(cy - 0.35*sin(t/4 - 0.24))*5)*6)/6\nrgb(max(red, gold), max(gold*0.8, blue*0.6), blue)";
const OLD_PHOTO_EX: &str = "-- a: how much (0: the GIF as it is)\nmix(rgb(ur, ug, ub), rgb(ul*1.1 + 0.08, ul*0.92 + 0.04, ul*0.65), a)";
const OLD_TV_EX: &str = "loop 4\ndx = (x - 0.5)*1.6; dy = (y - 0.5)*1.2\nlines = py % 2 ? 1 : 0.6\nroll = 0.85 + 0.15*max(0, tri(y/2 + t/4))\ncorners = clamp(1.5 - (dx*dx + dy*dy)*1.6, 0, 1)\nlayer multiply { gray(lines * roll * corners) }";

/// The drawing examples (opendmd-web/formulas/, as they were), timed as effects.
const STARFIELD_EX: &str = "loop 2.4\nfps 100\n-- Starfield: 192 stars flying at you, a pass through the field a loop,\n-- each a streak from where it was a moment ago\n-- a: streak length\nframe { back = 1 + 1/64 + (144 + a*240)/4096 }\ndraw {\n  for k < 192 {\n    sx = hash(k, 7)*32 - 16; sy = hash(k, 3)*8 - 4  -- where it is, across and up\n    near = frac(t/2.4 - hash(k, 11))          -- 0 far, 1 at you\n    s = 1/(65/64 - near); st = 1/(back - near)  -- perspective, now and a moment ago\n    hx = sx*s; hy = sy*s                      -- the head, from the middle\n    -- a head off the panel: the line moved far below it, where it is skipped\n    row = abs(hx) > 64 or abs(hy) > 16 ? -999 : 16\n    lit = near*near                           -- squared: far stars stay dark longer\n    line(64 + sx*st, row + sy*st, 64 + hx, row + hy, lit*0.2, lit)\n  }\n}\npal(ice, ink)";
const TUNNEL_EX: &str = "loop 4.8\nfps 100\n-- Tunnel: the time warp. Each loop the walls turn once, travel two\n-- checker lengths, and the palette goes round once.\n-- From the vanishing point, the tunnel twice as tall as wide:\ndx = px - 63.5; dy = (15.5 - py)*2\n-- how far each pixel is from the black middle: 0 there (where the\n-- tunnel is deeper than 16), ~15 at the edges\n-- (this and `round` depend on the pixel alone: worked out once, kept)\nnear = 16 - min(128/len(dx, dy), 16)\n-- round the walls: 16 checker cells a turn, twisting with depth\ntwist = 1                              -- try 0 to 3\nround = atan2(dy, dx)*16 + (16 - near)*twist/8\n-- the checker, turning and travelling\nu = round + t/0.3; v = 16 + t/0.6 - near\ncell = (floor(u) + floor(v)) % 2\n-- lit by depth (full near, fading as a square, black in the middle) and\n-- by a light going round with the walls, in eight steps\nfade = min(near, 10)                    -- 10 near, to 0 where it turns black\nside = 0.0635 + 0.0159*cos(u/16)\nlight = floor(fade*fade*side + (near > 0))/8\npal(chrono, cell ? frac(0.711 + t/4.8) : frac(0.336 + t/4.8)) * light";
const ROTOZOOM_EX: &str = "loop 9.6\n-- Rotozoom: Mode 7. A checker turning once a loop, its zoom breathing\n-- twice, gliding two checkers across and one down; under it a noise\n-- texture turning the other way twice as fast; rows and columns wobble.\n-- a: wobble = gentle, strong, off\nframe {\n  sn = sin(t/9.6); cs = cos(t/9.6)                 -- the turn\n  s2 = -sin(t/4.8); c2 = cos(t/4.8)                -- the noise's, back twice\n  zoom = 2                                         -- try 1 to 4\n  sc = 1/((1 + sin(t/4.8)/2)*zoom*8)               -- checker cells a pixel\n  sc2 = sc*0.75\n  cu = t/0.6; cv = t/1.2                           -- the glide, in cells\n  amp = pick(floor(a*2 + 0.5), 3, 4.5, 0)/8        -- the wobble, in texels\n}\ndx = px - 64; dy = 15 - py\n-- the wobble: rows slide sideways, columns up and down, on travelling waves\nwob = sin((31 - py)/32 + t/3.2)*amp\ncol = sin(px*0.0234375 - t/4.8)*amp\n-- where each pixel is on the checker: its column's part plus its row's\nu = (cu + cs*sc*dx) + (wob - sn*sc*dy)\nv = (cv + sn*sc*dx + col) + cs*sc*dy\ncell = (floor(u) + floor(v)) % 2\nu2 = (cv + c2*sc2*dx) + (wob - s2*sc2*dy)\nv2 = (cu + s2*sc2*dx + col) + c2*sc2*dy\npal(fire, (cell ? 0.44 : 0.19) + noise(u2, v2, 0)*0.6)";
const ROTOZOOM_FLOOR_EX: &str = "loop 9.6\nfps 100\n-- Rotozoom floor: Mode 7 as a floor. A checker running away to a horizon\n-- at the top, turning once a loop and gliding, each row scaled by its\n-- distance and the far rows fogged; rows and columns wobble.\n-- a: wobble = gentle, strong, off\nframe {\n  sn = sin(t/9.6); cs = cos(t/9.6)                 -- the turn\n  zoom = 2                                         -- try 1 to 4\n  cu = t/0.6; cv = t/1.2                           -- the glide, in cells\n  amp = pick(floor(a*2 + 0.5), 3, 4.5, 0)/8        -- the wobble, in texels\n}\ndx = px - 64; ty = 31 - py\n-- a row's distance: checker cells a pixel, smaller further down\nrs = 1/((ty + 2)*zoom)\nwob = sin(ty/32 + t/3.2)*amp\ncol = sin(px*0.0234375 - t/4.8)*amp\n-- ahead by the row's distance, across by the pixel's\nu = cs*rs*dx + (cu + wob + sn*rs*64)\nv = sn*rs*dx + col + (cv - cs*rs*64)\ncell = (floor(u) + floor(v)) % 2\n-- the far rows fade into the fog (black at the horizon)\nfog = ty < 3 ? 0 : ty < 6 ? 0.375 : ty < 10 ? 0.625 : 1\npal(fire, cell ? 0.78 : 0.28) * fog";
// psi-lang: `loop forever`, a sprite, slot reuse (opendmd-web/formulas/endless-road.psi).
const ROAD_EX: &str = "loop forever\nfps 100\n-- Endless road: an OutRun drive into a synthwave sunset. Bends and hills\n-- come from noise along the distance driven, which grows without end, so\n-- the track doesn't repeat; each time it plays it starts somewhere else.\n-- The colours, 0 to 18: the mountains and the car's glass and tyres\n-- (0), the road's stripes (1, 2), the kerbs' (3, 4), the verge's (5, 6),\n-- the sun from the top down (7 to 9), the car's body and tail lights\n-- (10, 11), then the sky from the top down to the horizon (12 to 18)\npalette road = #000, #400032, #800064, #e4cf00, #f00, #3f0000, #7f0000,\n  #e4cf00, #ff7800, #ff2800, #bf0000, #ff5000,\n  #1478ff, #2850ff, #3c28ff, #5000ff, #7b00f2, #a700e4, #d300d6\n-- The car, from behind: body (4), glass and tyres (5), tail lights (6)\nsprite car {\n  key . 0, R 4, D 5, L 6\n  ........RRRRRRRRRR........\n  .......RDDDDDDDDDDR.......\n  ......RDDDDDDDDDDDDR......\n  ...RRRRRRRRRRRRRRRRRRRR...\n  .RLLLLRDDDDDDDDDDDDRLLLLR.\n  .RLLLLRDDDDDDDDDDDDRLLLLR.\n  .RRRRRRRRRRRRRRRRRRRRRRRR.\n  .RRRRRRRRRRRRRRRRRRRRRRRR.\n  DDDDDDDDDDDDDDDDDDDDDDDDDD\n}\n-- The mountains: 19 spans (row, from, to)\ntable MY = 19,19,19,20,20,20,21,21,21,22,22,22,23,23,23,24,24,25,26\ntable MA = -1,57,118,0,59,120,2,61,122,4,62,123,5,64,125,7,66,67,69\ntable MB = 17,83,134,16,81,132,14,79,130,12,78,129,11,76,127,9,74,73,71\n-- Road rows py 0 to 18: depth/2 in bends (ZH), depth in stripes (ZS),\n-- half width in pixels (HW); the sun's rows and half widths\ntable ZH = .0058,.0061,.0064,.0068,.0072,.0077,.0083,.009,.0098,.011,.012,.013,.015,.017,.02,.025,.032,.045,.075\ntable ZS = .77,.81,.86,.91,.97,1.03,1.11,1.2,1.3,1.43,1.58,1.76,2,2.31,2.73,3.33,4.29,6,10\ntable HW = 50.7,48.1,45.5,42.9,40.3,37.7,35.1,32.5,29.9,27.3,24.7,22.1,19.5,16.9,14.3,11.7,9.1,6.5,3.9\ntable UY = 20,21,23,24,25,26,27,28,29,30,31\ntable UC = 7.2,7.6,8,8,7.9,7.6,7.2,6.7,6,5,3.5\nframe {\n  u = t*0.15 + seed*1000                    -- distance driven, in bends\n  h0 = noise(u, 0, 0); h1 = noise(u + 0.03, 0, 0)    -- heading here, just ahead\n  v0 = noise(u, 5, 0)                       -- height here, and its slope\n  dv = (noise(u + 0.01, 5, 0) - v0)*100\n  cx = 50.5 + (h1 - h0)*80                  -- the car's left edge: it slides out in bends\n  mx = (0.5 - h0)*60; sunx = 64 + mx        -- the sky turns with the road\n}\ndraw {\n  for k < 19 {\n    my = MY[k]\n    line(MA[k] + mx, my, MB[k] + mx, my, .35)\n  }\n  for k < 11 {\n    uy = UY[k]; uc = UC[k]\n    line(sunx - uc, uy, sunx + uc, uy, .2)\n  }\n}\n-- each row looks ahead: how far the road bends there, and how far a hill\n-- lifts or hides it (a crest moves rows to where the road is out of sight)\nzp = ZH[py]\nun = u + zp\nq = py - clamp((noise(un, 5, 0) - v0 - zp*dv)*ZS[py]*200, -6, 2)\nhw = HW[q]\nalt = frac(t*10 + ZS[q]) < 0.5             -- light or dark stripe: 10 a second\nadx = abs(px - (64 + (noise(un, 0, 0) - h0)*2000))\n-- 0 road, 1 kerb, 2 verge or sky; drawn: 3 the sun, 5 the mountains,\n-- 4 to 6 the car\nm = max(max((adx >= hw) + (adx >= hw*1.2), ink*16), car[px - cx, ty - 23])\n-- its colour, i/18 for colour i: the stripes', the sky's and the sun's by row\npal(road, pick(m, (1 + alt)/18, (3 + alt)/18,\n                  (py < 19 ? 5 + alt : floor(28.44 - py*.515))/18,\n                  (py < 23 ? 9 : py < 27 ? 8 : 7)/18, 10/18, 0, 11/18))";
const CUBE_EX: &str = "loop 4.8\nfps 100\n-- Wireframe cube: turning about two axes and swinging across; the near\n-- edges brighter. The colour is the last line's #40c0ff\n-- a: size\ntable V = -1,-1,-1, 1,-1,-1, 1,1,-1, -1,1,-1, -1,-1,1, 1,-1,1, 1,1,1, -1,1,1\ntable E = 0,1, 1,2, 2,3, 3,0, 4,5, 5,6, 6,7, 7,4, 0,4, 1,5, 2,6, 3,7\nframe {\n  sa = sin(t/4.8); ca = cos(t/4.8); sb = sin(t/4.8 + 1/6); cb = cos(t/4.8 + 1/6)\n  size = 34 + a*28; mid = 64 + sa*40\n}\ndraw {\n  for e < 12 {\n    m = E[e*2]*3; n = E[e*2 + 1]*3            -- the edge's two corners\n    x0 = V[m]*size; y0 = V[m + 1]*size; z0 = V[m + 2]*size\n    x1 = V[n]*size; y1 = V[n + 1]*size; z1 = V[n + 2]*size\n    xa = x0*ca + z0*sa; za = z0*ca - x0*sa; ya = y0*cb - za*sb; zb = y0*sb + za*cb  -- turned\n    xc = x1*ca + z1*sa; zc = z1*ca - x1*sa; yc = y1*cb - zc*sb; zd = y1*sb + zc*cb\n    line(mid + xa*36/(zb + 128), 16 + ya*36/(zb + 128), mid + xc*36/(zd + 128), 16 + yc*36/(zd + 128),\n         clamp(0.594 - zb/148, 0.156, 1), clamp(0.594 - zd/148, 0.156, 1))\n  }\n}\n#40c0ff * ink";

/// The bundled files as they ship (roadprobe: Endless road's cost on the
/// panel, against Time gate's).
const ROAD_FILE: &str = include_str!("../../opendmd-web/formulas/endless-road.psi");
const TIME_GATE_FILE: &str = include_str!("../../opendmd-web/formulas/time-gate.psi");

/// Endless road as bundled (the car drawn since roadprobe-c), without its
/// row noises, and without those, the draw stage, the palette and pick
/// (the rows' floor). roadprobe-a..c also timed the sprite car's version.
fn road_variants() -> Vec<(&'static str, String)> {
    let src = ROAD_FILE;
    let rows = src.find("-- each row looks").unwrap_or(0);
    let draw = src.find("draw {").unwrap_or(rows);
    let no_noise = |s: &str| s.replace("noise(un, 5, 0)", "v0").replace("noise(un, 0, 0)", "h0");
    let no_draw = |s: &str| format!("{}{}", &s[..draw], s[rows..].replace("ink*16", "0"));
    let gray = |s: &str| match s.find("pal(road, pick(m,") {
        Some(i) => format!("{}gray(m/6)\n", &s[..i]),
        None => s.to_string(),
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
    /// to the judgment).
    pub const PHASES: [&str; 7] = ["wake", "other", "start", "tick", "blit", "wait", "post"];
    /// Work (start + tick + blit + wait + post) at or over these µs.
    const OVER: [u32; 5] = [8_000, 9_000, 10_000, 11_000, 12_000];

    static N: AtomicU32 = AtomicU32::new(0);
    static SUM: [AtomicU32; 7] = [const { AtomicU32::new(0) }; 7];
    static MAX: [AtomicU32; 7] = [const { AtomicU32::new(0) }; 7];
    static COUNTS: [AtomicU32; 3] = [const { AtomicU32::new(0) }; 3];
    static OVERS: [AtomicU32; 5] = [const { AtomicU32::new(0) }; 5];

    /// One Formula frame (main task only).
    pub fn note(p: [u32; 7], missed: bool, excused: bool, stretched: bool) {
        N.fetch_add(1, Relaxed);
        for (i, v) in p.iter().enumerate() {
            SUM[i].fetch_add(*v, Relaxed);
            MAX[i].fetch_max(*v, Relaxed);
        }
        for (c, b) in COUNTS.iter().zip([missed, excused, stretched]) {
            c.fetch_add(u32::from(b), Relaxed);
        }
        let work = p[2] + p[3] + p[4] + p[5] + p[6];
        for (c, o) in OVERS.iter().zip(OVER) {
            c.fetch_add(u32::from(work >= o), Relaxed);
        }
    }

    fn line() -> Option<String> {
        let n = N.swap(0, Relaxed);
        let sums: [u32; 7] = core::array::from_fn(|i| SUM[i].swap(0, Relaxed));
        let maxes: [u32; 7] = core::array::from_fn(|i| MAX[i].swap(0, Relaxed));
        let c: [u32; 3] = core::array::from_fn(|i| COUNTS[i].swap(0, Relaxed));
        let o: [u32; 5] = core::array::from_fn(|i| OVERS[i].swap(0, Relaxed));
        if n == 0 {
            return None;
        }
        let (us, ms, asked, _) = opendmd_core::fx_psi::stats();
        let mut s = format!(
            "psi-probe: play {} s: {n} formula frames, misses {}, late excused {}, stretched {}, governor {us} us mean -> {ms} ms frames (asked {asked}); mean/max us:",
            // SAFETY: a clock read.
            unsafe { esp_idf_sys::esp_timer_get_time() } / 1_000_000,
            c[0],
            c[1],
            c[2]
        );
        for (i, name) in PHASES.iter().enumerate() {
            s.push_str(&format!(" {name} {}/{}", sums[i] / n, maxes[i]));
        }
        s.push_str(&format!(
            "; work >=8 ms {} >=9 {} >=10 {} >=11 {} >=12 {}",
            o[0], o[1], o[2], o[3], o[4]
        ));
        Some(s)
    }

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

/// Only the road's rows, then the play lines (roadprobe-b/c: a's full
/// probe ran out of heap before them). `false`: the full probe, which
/// starts the play lines when it is done.
const ROAD_ONLY: bool = false;

/// Only the JIT spike's rows (jitspike-b: a's full probe ran out of heap
/// in the looks, 225 s in, and the spike's own rows were done long before).
const JIT_ONLY: bool = false;

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
    split("Endless road (bundled)", ROAD_FILE, true, &mut canvas);
    effects(&[("Starfield", STARFIELD_EX, "a=0.5"), ("Time gate (bundled)", TIME_GATE_FILE, "")], "", &mut canvas);
    fx_psi::set_still_gate(Some(crate::still_gate));
    split("Time gate (bundled)", TIME_GATE_FILE, true, &mut canvas);
    fx_psi::set_still_gate(Some(crate::still_gate));
    say!(info,
        "{DONE}: roadprobe, main stack {} B unused (was {stack0}), heap {} B free (largest {} B), {} ms in all",
        stack_unused(), heap(), crate::state::largest_free_block(), start.elapsed().as_millis()
    );
    *REPORT.lock().unwrap() = None;
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
        let inputs = psi::golden::inputs(k * 20 % loop_ms.max(1));
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
        let inputs = psi::golden::inputs(k * 20 % loop_ms.max(1));
        psi::render_rows_still::<128>(prog, &mut s, None, Some(&mut st), &inputs, &t, None, u32::MAX, out);
    }))
}

/// The examples as effects on the real path (`Fx::parse`, the canvas taken
/// at open, `Fx::frame`, the still cache taken on the first frame): with
/// the cache, then with the gate refusing it. `again` names a second run.
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

/// Cycles a 32-bit load (and a load plus a store) over `w`, best of three
/// passes: the memory's own latency where `w` lies.
fn latency(w: &mut [u32]) -> (f64, f64) {
    let n = w.len() as f64;
    let (mut ld, mut ls) = (u32::MAX, u32::MAX);
    for _ in 0..3 {
        let t0 = ccount();
        let mut acc = 0u32;
        for v in w.iter() {
            // SAFETY: a read of our own buffer.
            acc = acc.wrapping_add(unsafe { core::ptr::read_volatile(v) });
        }
        let t1 = ccount();
        core::hint::black_box(acc);
        for v in w.iter_mut() {
            // SAFETY: our own buffer.
            unsafe { core::ptr::write_volatile(v, core::ptr::read_volatile(v).wrapping_add(1)) };
        }
        let t2 = ccount();
        ld = ld.min(t1.wrapping_sub(t0));
        ls = ls.min(t2.wrapping_sub(t1));
    }
    (f64::from(ld) / n, f64::from(ls) / n)
}

/// The data memory's latency by address: as many 2 KB blocks as the heap
/// gives (down to ~24 KB free), each timed, bucketed by 8 KB; then the
/// same with CPU0 kept busy loading and storing over a buffer of its own.
fn latency_map() {
    struct Spin {
        buf: Vec<u32>,
        stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    }
    let mut blocks: Vec<Vec<u32>> = Vec::new();
    while heap() > 24 * 1024 + 2048 && blocks.len() < 64 {
        let mut v: Vec<u32> = Vec::new();
        if v.try_reserve_exact(512).is_err() {
            break;
        }
        v.resize(512, 0);
        blocks.push(v);
    }
    // (Unstable: the stable sort keeps a 4 KB buffer on the stack, and
    // this runs on the main task, whose high-water mark the probe reports.)
    blocks.sort_unstable_by_key(|b| b.as_ptr() as usize);
    let map = |blocks: &mut Vec<Vec<u32>>, what: &str| {
        let mut line = String::new();
        let mut bucket = usize::MAX;
        let mut acc: Vec<(f64, f64)> = Vec::new();
        let flush = |line: &mut String, bucket: usize, acc: &mut Vec<(f64, f64)>| {
            if !acc.is_empty() {
                let n = acc.len() as f64;
                let (l, s) = acc.iter().fold((0.0, 0.0), |a, b| (a.0 + b.0, a.1 + b.1));
                line.push_str(&format!(" {:#x}:{:.2}/{:.2}", bucket << 13, l / n, s / n));
                acc.clear();
            }
        };
        for b in blocks.iter_mut() {
            let k = b.as_ptr() as usize >> 13;
            if k != bucket {
                flush(&mut line, bucket, &mut acc);
                bucket = k;
            }
            acc.push(latency(b));
        }
        flush(&mut line, bucket, &mut acc);
        say!(info, "psi-probe: memory {what}, cycles a load/a load+store by 8 KB from:{line}");
    };
    say!(info, "psi-probe: memory: {} blocks of 2 KB from {:#x} to {:#x}, heap {} B free",
        blocks.len(),
        blocks.first().map_or(0, |b| b.as_ptr() as usize),
        blocks.last().map_or(0, |b| b.as_ptr() as usize),
        heap());
    map(&mut blocks, "(CPU0 as it is)");
    // CPU0 busy over a 2 KB buffer of its own (the network's side).
    if let Some(spare) = blocks.pop() {
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mut spin = Spin { buf: spare, stop: stop.clone() };
        let at = spin.buf.as_ptr() as usize;
        let cfg = esp_idf_hal::task::thread::ThreadSpawnConfiguration {
            name: Some(b"psi-spin\0"),
            stack_size: 3072,
            priority: 1,
            pin_to_core: Some(esp_idf_hal::cpu::Core::Core0),
            ..Default::default()
        };
        let _ = cfg.set();
        let h = std::thread::Builder::new().stack_size(3072).spawn(move || {
            let mut i = 0usize;
            while !spin.stop.load(Ordering::Relaxed) {
                let n = spin.buf.len();
                let v = unsafe { core::ptr::read_volatile(&spin.buf[i % n]) };
                unsafe { core::ptr::write_volatile(&mut spin.buf[(i + 7) % n], v.wrapping_add(1)) };
                i = i.wrapping_add(1);
                if i.is_multiple_of(100_000) {
                    std::thread::yield_now();
                }
            }
        });
        let _ = esp_idf_hal::task::thread::ThreadSpawnConfiguration::default().set();
        std::thread::sleep(Duration::from_millis(20));
        map(&mut blocks, &format!("(CPU0 spinning over {at:#x})"));
        stop.store(true, Ordering::Relaxed);
        if let Ok(h) = h {
            let _ = h.join();
        }
    }
    drop(blocks);
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

/// Tunnel's VM rows with its still lanes at one placement after another,
/// in one boot: the panel's DMA ranges first, then every ordered pair of a
/// pool of 16 KB lanes taken from wherever the heap gives them (as many as
/// leave ~40 KB free), then a second scratch. p50/p90 of the rows' cycles.
fn placement(out: &mut [u8]) {
    let (mut lo, mut hi, mut bytes, mut n, mut dlo, mut dhi) = (0, 0, 0, 0, 0, 0);
    // SAFETY: plain out-parameters; the panel is up.
    unsafe { esp_idf_sys::hub75::hub75_dma_span(&mut lo, &mut hi, &mut bytes, &mut n, &mut dlo, &mut dhi) };
    say!(info, "psi-probe: placement: the panel's DMA reads {n} buffers, {bytes} B in {lo:#x}..{hi:#x}; descriptors {dlo:#x}..{dhi:#x}");
    let mut prog = zeroed::<psi::Program>();
    if compile(TUNNEL_EX, &mut prog, true, true).is_err() {
        return;
    }
    let t = fx_psi::tables();
    let lp = prog.loop_ms();
    let mut st = psi::Still::new();
    if !st.take(&prog) {
        say!(error, "psi-probe: placement: no heap for the cache");
        return;
    }
    let mut pool: Vec<Box<psi::StillLane>> = Vec::new();
    while pool.len() < 4 && heap() > 40 * 1024 + 16 * 1024 {
        match psi::StillLane::try_new() {
            Some(l) => pool.push(l),
            None => break,
        }
    }
    let mut cands = pool;
    let own = [st.lane_addr(0), st.lane_addr(1)];
    say!(info, "psi-probe: placement: lanes at {:#x} {:#x} (the cache's own), others {:?}, heap {} B free",
        own[0], own[1], cands.iter().map(|l| format!("{:#x}", &**l as *const psi::StillLane as usize)).collect::<Vec<_>>(), heap());
    let mut s = zeroed::<psi::Scratch<128>>();
    s.reset();
    let rows = |st: &mut psi::Still, s: &mut psi::Scratch<128>, out: &mut [u8]| -> String {
        // The first frame keeps the values, the rest are drawn from them.
        psi::render_rows_still::<128>(&prog, s, None, Some(&mut *st), &psi::golden::inputs(0), &t, None, u32::MAX, out);
        let mut v = Vec::with_capacity(60);
        for k in 0..60u32 {
            psi::render_rows_still::<128>(&prog, s, None, Some(&mut *st), &psi::golden::inputs(k * 10 % lp), &t, None, u32::MAX, out);
            let a: [u32; 4] = core::array::from_fn(|i| psi::stats::STAMPS[i].load(Ordering::Relaxed));
            v.push(a[3].wrapping_sub(a[2]));
        }
        us50_90(&mut v)
    };
    let sa = &*s as *const psi::Scratch<128> as usize;
    let r = rows(&mut st, &mut s, out);
    say!(info, "psi-probe: placement: lanes {:#x} {:#x} (own), scratch {sa:#x}: rows {r}", own[0], own[1]);
    // Each candidate as lane 0, then as lane 1, the other lane the cache's own.
    for i in 0..cands.len() {
        for k in 0..2 {
            let c = cands.remove(i);
            let back = st.swap_lane(k, c).expect("a lane");
            let r = rows(&mut st, &mut s, out);
            say!(info, "psi-probe: placement: lanes {:#x} {:#x}, scratch {sa:#x}: rows {r}", st.lane_addr(0), st.lane_addr(1));
            let c = st.swap_lane(k, back).expect("a lane");
            cands.insert(i, c);
        }
    }
    // A second scratch, wherever the heap puts it now, with the own lanes.
    drop(cands);
    let mut s2 = zeroed::<psi::Scratch<128>>();
    s2.reset();
    let sa2 = &*s2 as *const psi::Scratch<128> as usize;
    let r = rows(&mut st, &mut s2, out);
    say!(info, "psi-probe: placement: lanes {:#x} {:#x} (own), scratch {sa2:#x}: rows {r}", own[0], own[1]);
}

/// Tunnel VM-direct with the same stamps (setup, rows): against `split`.
fn split_direct(out: &mut [u8]) {
    if heap_low("split Tunnel VM-direct") {
        return;
    }
    let mut prog = zeroed::<psi::Program>();
    if compile(TUNNEL_EX, &mut prog, true, true).is_err() {
        return;
    }
    let mut s = zeroed::<psi::Scratch<128>>();
    s.reset();
    let mut st = psi::Still::new();
    if !st.take(&prog) {
        say!(error, "psi-probe: split Tunnel direct: no heap for the cache");
        return;
    }
    let t = fx_psi::tables();
    let lp = prog.loop_ms();
    psi::render_rows_still::<128>(&prog, &mut s, None, Some(&mut st), &psi::golden::inputs(0), &t, None, u32::MAX, out);
    let mut parts: [Vec<u32>; 3] = Default::default();
    if !reserve(&mut parts, "split Tunnel VM-direct") {
        return;
    }
    for k in 0..FRAMES {
        let t0 = ccount();
        psi::render_rows_still::<128>(&prog, &mut s, None, Some(&mut st), &psi::golden::inputs(k * 10 % lp), &t, None, u32::MAX, out);
        let t1 = ccount();
        let a: [u32; 4] = core::array::from_fn(|i| psi::stats::STAMPS[i].load(Ordering::Relaxed));
        parts[0].push(a[2].wrapping_sub(a[1]));
        parts[1].push(a[3].wrapping_sub(a[2]));
        parts[2].push(t1.wrapping_sub(t0));
        std::thread::sleep(Duration::from_millis(1));
    }
    let [b, c, t] = &mut parts;
    say!(info,
        "psi-probe: split Tunnel VM-direct, p50/p90 us: VM setup {} | VM rows {} | in all {} ({})",
        us50_90(b), us50_90(c), us50_90(t),
        addrs()
    );
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

/// The heap every `n` steps of a section: its slope.
fn heap_every(section: &str, k: u32, n: u32) {
    if k.is_multiple_of(n) {
        say!(info, "psi-probe: {section} {k}: heap {} B free, largest block {} B", heap(), crate::state::largest_free_block());
    }
}

/// What compiling leaves behind: one formula compiled on the player's
/// formula stack (`psi_stack::compile`, a FreeRTOS task since speed-d) 200
/// times at a builder's pace (20 ms apart), then on the old std thread 50
/// times, the heap before and after each.
fn compile_leak() {
    let mut prog = zeroed::<psi::Program>();
    let src = TUNNEL_EX;
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

/// A VM frame's p50 / p90 for `prog`: with a still cache when it has
/// values for one (and the heap a block for it), else every step (with the
/// draw stage's canvas).
fn vm_frame(prog: &psi::Program, out: &mut [u8]) -> (u32, u32, &'static str) {
    let lp = prog.loop_ms();
    if prog.still_lanes() > 0 {
        if let Some((a, b, _)) = lanes_still(prog, lp, out) {
            return (a, b, "cached");
        }
    }
    let (a, b, _) = lanes::<128>(prog, lp, out);
    (a, b, "plain")
}

/// psi-speed's before and after, in one boot: the five examples' VM frames
/// with the narrowed steps (`add_w`, `mul_n`, `shr`, …) and without, and
/// with the cheap loops in IRAM (the release) and in flash; and every
/// frame of each the same bytes either way.
fn speed(out: &mut [u8]) {
    let t = fx_psi::tables();
    let (mut same, mut differ) = (0u32, 0u32);
    for (name, src) in [
        ("Starfield", STARFIELD_EX),
        ("Wireframe cube", CUBE_EX),
        ("Tunnel", TUNNEL_EX),
        ("Rotozoom", ROTOZOOM_EX),
        ("Rotozoom floor", ROTOZOOM_FLOOR_EX),
        ("Plasma", PLASMA_EX),
        ("Fire", FIRE_EX),
    ] {
        let mut fast = zeroed::<psi::Program>();
        let mut checked = zeroed::<psi::Program>();
        if compile(src, &mut fast, true, true).is_err() || compile(src, &mut checked, true, false).is_err() {
            say!(error, "psi-probe: speed {name}: does not compile");
            continue;
        }
        let lp = fast.loop_ms();
        // (Its own scratch and canvas, given back before the timed frames:
        // the still cache wants a 16 KB block.)
        let mut s = zeroed::<psi::Scratch<128>>();
        s.reset();
        let mut ink = psi::Ink::boxed();
        let mut frame_hash = |p: &psi::Program, ms: u32, out: &mut [u8]| {
            psi::render_rows_ink::<128>(p, &mut s, Some(&mut ink), &psi::golden::inputs(ms), &t, None, u32::MAX, out);
            psi::golden::fnv(out)
        };
        for ms in [0, lp / 3, lp * 2 / 3, lp.saturating_sub(20)] {
            let a = frame_hash(&fast, ms, out);
            let b = frame_hash(&checked, ms, out);
            if a == b {
                same += 1;
            } else {
                differ += 1;
                say!(error, "psi-probe: speed {name} at {ms} ms: {b:#018x} checked against {a:#018x} narrowed");
            }
        }
        drop(s);
        drop(ink);
        let (f50, f90, how) = vm_frame(&fast, out);
        let (sc, still) = (psi::stats::SCRATCH.load(Ordering::Relaxed), psi::stats::STILL.load(Ordering::Relaxed));
        let (c50, c90, _) = vm_frame(&checked, out);
        say!(info,
            "psi-probe: speed {name:<15} ({how}): narrowed p50 {f50} p90 {f90} | checked p50 {c50} p90 {c90} us a frame; est {} us ({} checked); scratch {sc:#x} picture {:#x} still lane {still:#x}",
            fast.esp32_us_still(),
            checked.esp32_us_still(),
            out.as_ptr() as usize
        );
    }
    say!(info,
        "psi-probe: speed frames: {same} the same narrowed or checked, {differ} differ{}",
        if differ == 0 { " (bit-identical)" } else { "" }
    );
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

/// [`FRAMES`] VM frames of `prog` drawn `way`, 20 ms apart, with a still
/// cache when it has values for one (kept on an untimed first frame).
fn time_way(prog: &psi::Program, way: Way, out: &mut [u8]) -> (u32, u32, &'static str) {
    let mut s = zeroed::<psi::Scratch<128>>();
    s.reset();
    let mut ink = psi::Ink::boxed();
    let mut st = psi::Still::new();
    let cached = way != Way::Turns && prog.still_lanes() > 0 && st.take(prog);
    draw_way(prog, way, &mut s, &mut ink, cached.then_some(&mut st), 0, out);
    let (a, b, _) = timed(|k| draw_way(prog, way, &mut s, &mut ink, cached.then_some(&mut st), k * 20, out));
    (a, b, if cached { "cached" } else { "plain" })
}

/// psi-speed2's before and after, in one boot: each example's VM frame as
/// compiled (fused runs; draw loops of 32 turns and more as lanes), without
/// the fused runs (`compile_unfused`), and that with every draw loop a turn
/// at a time (`render_rows_ink_turns`, for the examples that have a lane
/// loop); every frame of each the same bytes all three ways.
fn speed2(out: &mut [u8]) {
    let (mut same, mut differ) = (0u32, 0u32);
    for (name, src) in [
        ("Starfield", STARFIELD_EX),
        ("Wireframe cube", CUBE_EX),
        ("Tunnel", TUNNEL_EX),
        ("Rotozoom", ROTOZOOM_EX),
        ("Rotozoom floor", ROTOZOOM_FLOOR_EX),
        ("Endless road", ROAD_EX),
        ("Plasma", PLASMA_EX),
        ("Fire", FIRE_EX),
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
        let lanes = fused.lane_loops();
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
        let (f50, f90, how) = time_way(&fused, Way::Fused, out);
        let (s50, s90, _) = time_way(&steps, Way::Steps, out);
        let (t50, t90) = if lanes > 0 {
            let (a, b, _) = time_way(&steps, Way::Turns, out);
            (a, b)
        } else {
            (0, 0)
        };
        say!(info,
            "psi-probe: speed2 {name:<15} ({how}): fused p50 {f50} p90 {f90} | steps p50 {s50} p90 {s90} | turns p50 {t50} p90 {t90} us a frame ({lanes} lane loops); est {} us ({} steps, {} turns)",
            fused.esp32_us_still(),
            steps.esp32_us_still(),
            steps.esp32_us_turns()
        );
    }
    say!(info,
        "psi-probe: speed2 frames: {same} the same fused, as steps and a turn at a time, {differ} differ{}",
        if differ == 0 { " (bit-identical)" } else { "" }
    );
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
    if let Err(e) = compile(ROTOZOOM_FLOOR_EX, &mut prog, true, true) {
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
        for row in 0..psi::H {
            f(dx.as_ptr(), col.as_ptr(), row_of(&mut jit, row), params[row].as_ptr());
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
            for row in 0..psi::H {
                f(dx.as_ptr(), col.as_ptr(), row_of(&mut jit, row), params[row].as_ptr());
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

pub fn run(root: &str) {
    if !open_report(root) {
        return;
    }
    let start = Instant::now();
    let stack0 = stack_unused();
    if ROAD_ONLY {
        road_probe(root, start, stack0);
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
    worst_stack();
    let mut out = vec![0u8; psi::W * psi::H * 3];
    let mut canvas = Canvas::new();
    jit_spike(&mut out);

    // The examples as effects, first while the heap is at its fullest (the
    // still cache is 16-32 KB; p2probe-a ran these last, ~38 KB free, and no
    // 16 KB block was left for it). The real path: `Fx::parse`, the canvas taken
    // at open, `Fx::frame`, the still cache taken on the first frame): with
    // the cache, then with the gate refusing it (every step every frame).
    // Where the effect path's time goes (speed-d: Tunnel 4,932 us as an
    // effect against 3,651 VM-direct; the others ~200-250 us over).
    psi::stats::CLOCK.store(ccount as fn() -> u32 as usize, Ordering::Relaxed);
    latency_map();
    for gate in [true, false] {
        split("Tunnel", TUNNEL_EX, gate, &mut canvas);
        split("Rotozoom floor", ROTOZOOM_FLOOR_EX, gate, &mut canvas);
    }
    split_direct(&mut out);
    placement(&mut out);
    fx_psi::set_still_gate(Some(crate::still_gate));
    effects(
        &[
            ("Starfield", STARFIELD_EX, "a=0.5"),
            ("Wireframe cube", CUBE_EX, "a=0.5"),
            ("Tunnel", TUNNEL_EX, ""),
            ("Rotozoom", ROTOZOOM_EX, ""),
            ("Rotozoom floor", ROTOZOOM_FLOOR_EX, ""),
            ("Endless road", ROAD_EX, ""),
        ],
        "",
        &mut canvas,
    );
    say!(info, "psi-probe: main stack {} B unused after the examples as effects (the play path; compiles on the formula stack)", stack_unused());
    // The firmware's own gate again.
    fx_psi::set_still_gate(Some(crate::still_gate));

    speed(&mut out);
    say!(info, "psi-probe: main stack {} B unused after the speed rows", stack_unused());
    speed2(&mut out);
    say!(info, "psi-probe: main stack {} B unused after the speed2 rows", stack_unused());
    // Tunnel's effect again, now the heap has moved on: placement against
    // code (speed-a: 4,860 us as an effect, 3,709 VM-direct).
    effects(&[("Tunnel", TUNNEL_EX, "")], " again", &mut canvas);
    fx_psi::set_still_gate(Some(crate::still_gate));


    for (name, src) in psi::sketches::ALL {
        // Compile (into a zeroed program, as the effect does).
        let mut prog = zeroed::<psi::Program>();
        let t0 = Instant::now();
        let r = compile(src, &mut prog, true, true);
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

    compile_leak();
    // Golden frames: the host's and wasm's hashes, here.
    say!(info, "psi-probe: golden frames from {} B free", heap());
    let t = fx_psi::tables();
    let mut s = zeroed::<psi::Scratch<128>>();
    s.reset();
    let mut prog = zeroed::<psi::Program>();
    let (mut ok, mut bad) = (0, 0);
    for (gk, &(name, src, ms, want)) in psi::golden::FRAMES.iter().enumerate() {
        if heap_low("golden frames") {
            break;
        }
        heap_every("golden frames", gk as u32, 10);
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
        if heap_low("golden over") {
            break;
        }
        if compile(src, &mut prog, true, true).is_err() {
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
        if heap_low("golden drawn") {
            break;
        }
        if compile(src, &mut prog, true, true).is_err() {
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
    const STEPS: [(&str, &str); 29] = [
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
        // psi-lang: a noise of a growing value per pixel (`NOISE_O`, its
        // offsets set once a frame), against `noise (+mul)`'s row.
        ("noise wide (+mul +add)", "loop forever\ngray(noise(x * 8 + t, y, t))"),
        // A sprite's pixel (`SPR`), against `mul`'s row.
        // A palette of the formula's own (unpacked from its tables), against
        // `out pal`'s row.
        ("out own pal", "palette p = #102030, #f08000, #ffffff\npal(p, x)"),
        ("sprite (+mul)", "sprite s {\nkey . 0, X 1, Y 0.5\nXY.XY.XY.XY.XY.XY.\n.XYXY.XYXY.XYXY.XY\n}\ngray(s[x * 18, y + t])"),
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
    // psi-speed's narrowed steps (`range`: the ranges say they never
    // saturate), each over a baseline row above: add_w over `add`'s row,
    // and so on.
    const NARROW_STEPS: [(&str, &str); 8] = [
        ("add_w", "gray(x + t)"),
        ("sub_w", "gray(x - t)"),
        ("mul_w", "gray(px * t)"),
        ("mul_n", "gray(x * (t * 0.01))"),
        ("shr (+add_w)", "gray((x + t) / 4)"),
        ("select (+lt) n", "gray(x < t ? x : 0.5)"),
        ("pal * k few (+3)", "pal(fire, floor(x * 4) / 4) * (floor(x * 8 + t) / 8)"),
        ("pal * k=x (+mul_n)", "pal(fire, x) * (x * 0.9)"),
    ];
    let steps = STEPS
        .iter()
        .map(|&(w, s)| (w, s, false, false))
        .chain(LAYER_STEPS.iter().map(|&(w, s)| (w, s, true, false)))
        .chain(NARROW_STEPS.iter().map(|&(w, s)| (w, s, false, true)));
    for (what, src, over, narrow) in steps {
        // (Laid out without the column and still runs: a step's cost on
        // every pixel, as the table was measured; the checked steps as
        // then, but for psi-speed's rows.)
        if compile(src, &mut prog, false, narrow).is_err() {
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
    // psi-speed3: the last two, steps laned since (`%`, the comparisons,
    // `sin`) and steps still worked out turn by turn (`noise`, `digit`,
    // `sqrt`, `step`: `vm::lane_draw` -> `value_of` -> `eval`, the deepest
    // draw path), for `LANE_EVAL_NS`.
    const DRAW_STEPS: [(&str, &str, u32); 9] = [
        ("draw base (1 dot off)", "draw { dot(-1, -1, t) }\ngray(ink)", 1),
        ("draw loop step: 1024 dots off", "draw { for k < 1024 { dot(-1, -1, t) } }\ngray(ink)", 1024),
        ("draw loop step: +add", "draw { for k < 1024 { dot(-1, -1, k + t) } }\ngray(ink)", 1024),
        ("draw loop step: +hash", "draw { for k < 1024 { dot(-1, -1, hash(k, t)) } }\ngray(ink)", 1024),
        ("draw dot on panel x1024", "draw { for k < 1024 { dot(k % 128, 5 + t) } }\ngray(ink)", 1024),
        ("draw line 12 x256", "draw { for k < 256 { line(10, 5, 21, 5 + t) } }\ngray(ink)", 256),
        ("draw line 100 x64", "draw { for k < 64 { line(10, 5, 109, 5 + t) } }\ngray(ink)", 64),
        ("draw laned: mod cmp sin", "draw { for k < 1024 { dot(-1, -1, sin(k*0.01 + t) + (k % 7 >= 3) + (k % 5 != 1)) } }\ngray(ink)", 1024),
        ("draw eval: noise digit sqrt step", "draw { for k < 1024 { dot(-1, -1, noise(k*0.1, t, 0.5) + digit(k % 10, 1, 2) + sqrt(k + t) + step(0.5, frac(k*0.3))) } }\ngray(ink)", 1024),
    ];
    say!(info, "psi-probe: main stack {} B unused before the draw rows", stack_unused());
    let mut base = 0u32;
    for (what, src, n) in DRAW_STEPS {
        if compile(src, &mut prog, false, true).is_err() {
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
        // psi-speed2: a loop of 32 turns or more runs as lanes; the same
        // row a turn at a time (as these were measured before).
        if prog.lane_loops() > 0 {
            let (q50, q90, _) = time_way(&prog, Way::Turns, &mut out);
            say!(info,
                "psi-probe: step {what:<30} (turns) p50 {q50} p90 {q90} us a frame = {} ns a turn over the base, est {} us",
                u64::from(q50.saturating_sub(base)) * 1000 / u64::from(n),
                prog.esp32_us_turns()
            );
        }
    }
    say!(info, "psi-probe: main stack {} B unused after the draw rows (lane steps through value_of: noise, glyph)", stack_unused());
    // psi-speed2's fused runs: each pattern alone, as compiled (fused) and
    // without the markers (its steps), for `fuse::saved`. (psi-speed3: the
    // dispatch in flash, a `mul_n` multiply-add run as `mul_w`.)
    const FUSE_STEPS: [(&str, &str); 12] = [
        ("mul_add (lane, value)", "v = px*py*0.002\ngray(frac(v*0.7 + t))"),
        ("mul_add (mul_n)", "gray(frac((x + y)*0.2 + t))"),
        ("mul_add (lane, lane)", "v = px*py*0.002\nww = sin(px*0.1 + py*0.3)\ngray(frac(v*0.7 + ww))"),
        ("checker", "u = px*py*0.01 + t\nv = py*0.7 - px*0.3 + t/2\ngray((floor(u) + floor(v)) % 2)"),
        ("checker_3", "gray((floor(px*0.37 + py*0.2 + t) + floor(py*0.5 - px*0.13)) % 2)"),
        ("pal_select (row scale)", "cc = (floor(px*py*0.01 + t) + floor(py*0.3 - px*0.2)) % 2\npal(fire, cc ? 0.3 : 0.7) * (py/31)"),
        ("pal_select_k (lane)", "cc = (floor(px*py*0.01 + t) + floor(py*0.3 - px*0.2)) % 2\nlit = floor(px*py*0.002 + t) % 8 / 8\npal(chrono, cc ? frac(0.711 + t/4.8) : frac(0.336 + t/4.8)) * lit"),
        ("pal_pick", "pal(rainbow, pick(floor(px*0.07 + py*0.3) % 9, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, t/8, 0.8, 0.9))"),
        ("ink_1 pal, lines", "draw { for k < 8 { line(k*16, 0, k*16 + 8, 31) } }\npal(ice, ink)"),
        ("ink_1 gray, blank", "draw { dot(-1, -1, t) }\ngray(ink)"),
        ("ink rgb, lines", "draw { for k < 8 { line(k*16, 0, k*16 + 8, 31) } }\n#40c0ff * ink"),
        ("ink rgb, streaks", "draw { for k < 64 { line(k*2, 0, k*2 + 30, 31, 0, 1) } }\n#40c0ff * ink"),
    ];
    for (what, src) in FUSE_STEPS {
        let mut steps = zeroed::<psi::Program>();
        let ok = crate::psi_stack::run(|| psi::compile_into(src, &mut prog)).is_some_and(|r| r.is_ok())
            && crate::psi_stack::run(|| psi::compile_unfused(src, &mut steps)).is_some_and(|r| r.is_ok());
        if !ok {
            say!(error, "psi-probe: fuse {what}: does not compile");
            continue;
        }
        let fused = prog.steps() < prog.len();
        let (f50, f90, _) = lanes::<128>(&prog, 8000, &mut out);
        let (s50, s90, _) = lanes::<128>(&steps, 8000, &mut out);
        say!(info,
            "psi-probe: fuse {what:<22} fused {fused}: p50 {f50} p90 {f90} | steps p50 {s50} p90 {s90} us a frame = {} ns a pixel saved, est {} us ({} steps)",
            (i64::from(s50) - i64::from(f50)) * 1000 / 4096,
            prog.esp32_us(),
            steps.esp32_us()
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
        if compile(src, &mut plain, false, true).is_err() || compile(src, &mut prog, true, true).is_err() {
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
        if compile(&look, &mut prog, true, true).is_err() {
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
    play::start(format!("{root}/{REPORT_NAME}"));
}

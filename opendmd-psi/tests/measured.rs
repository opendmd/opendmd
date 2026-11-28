// Measured on a test board by psi-probe (2026-10-26, 1.0.0-beta.10-anylook-a): VM p50 µs a frame.
// "big clock": the bundled Formula as it is now, by psi-refresh (1.1.0-beta.4.probe1, 2026-11-28, the spare test board, 80 MHz flash; firmware/src/psi_probe/refresh.rs):
// compiled as below (no runs, the checked steps, unfused compares), the
// VM directly, t within 8 s. (Its calibration rows the same run, sources
// unchanged: base 704, sin 1394, noise 4462, within 4 % of these.)
pub const STEPS: &[(&str, &str, u32)] = &[
    ("base gray(x)", "gray(x)", 678),
    ("add", "gray(x + t)", 897),
    ("mul", "gray(x * t)", 1041),
    ("div (+add)", "gray(x / (x + t))", 2072),
    // fast-div (1.0.0-beta.11-fastdiv-b, 2026-10-29): these four rows again,
    // and a division taking the Knuth digit.
    ("div 128/v (+add)", "gray(128 / (px + t))", 2644),
    ("mod 0.3", "gray(x % 0.3)", 1152),
    ("sin (+add)", "gray(sin(x + t))", 1346),
    ("sqrt (+add)", "gray(sqrt(x + t))", 1789),
    ("len (+add)", "gray(len(x, x + t))", 2799),
    ("atan2 (+sub)", "gray(atan2(y - 0.5, x - t))", 2452),
    ("smoothstep (+add)", "gray(smoothstep(0.2, 0.8, x + t))", 1782),
    ("noise (+mul)", "gray(noise(x * 8, y, t))", 4540),
    ("select (+lt)", "gray(x < t ? x : 0.5)", 1064),
    ("digit (+2 mask)", "gray(digit(px % 8, px % 4, y * 6))", 2066),
    ("bit", "gray(bit(px, t))", 915),
    ("pick (+mul)", "gray(pick(x * 4, t, 0.5, 0.75, 1))", 1452),
    ("pick 8 (+mul)", "gray(pick(x * 8, t, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7))", 1452),
    ("chain 4 (+mul)", "k = x * 4\ngray(k < 0 ? 0 : k < 1 ? t : k < 2 ? 0.5 : k < 3 ? 0.75 : k < 4 ? 1 : 0)", 3015),
    ("big clock", include_str!("../../opendmd-web/formulas/big-clock.psi"), 8528),
    ("out hsv", "hsv(x, 1, 1)", 1511),
    ("out pal", "pal(fire, x)", 610),
    ("ur", "gray(ur)", 962),
    ("ul", "gray(ul)", 1256),
    ("shows through", "gray(x, 0.5)", 2135),
    ("+layer over 0.5", "layer { gray(x) }\nlayer { gray(x + t, 0.5) }", 2949),
    ("+layer add", "layer { gray(x) }\nlayer add { gray(x + t) }", 2128),
    ("+layer multiply", "layer { gray(x) }\nlayer multiply { gray(x + t) }", 2349),
    ("+layer screen", "layer { gray(x) }\nlayer screen { gray(x + t) }", 2575),
    ("+layer max", "layer { gray(x) }\nlayer max { gray(x + t) }", 2077),
    ("look scanlines", "layer multiply { gray(py % 2 ? 1 : 0.55) }", 1098),
    ("look 4 layers", "layer multiply { gray(y) }\nlayer add { gray(x) }\nlayer screen { gray(x + t) }\nlayer max { gray(py / 32) }", 4957),
    ("+layer add 0.4", "layer { gray(x) }\nlayer add { gray(x + t, 0.4) }", 3707),
    ("+layer multiply 0.4", "layer { gray(x) }\nlayer multiply { gray(x + t, 0.4) }", 3929),
    ("+layer screen 0.4", "layer { gray(x) }\nlayer screen { gray(x + t, 0.4) }", 3789),
    ("+layer max 0.4", "layer { gray(x) }\nlayer max { gray(x + t, 0.4) }", 3174),
    ("+layer min 0.4", "layer { gray(x) }\nlayer min { gray(x + t, 0.4) }", 3191),
    ("+layer screen row", "layer { gray(x) }\nlayer screen { gray(y + t) }", 1918),
    ("+layer screen hsv", "layer { gray(x) }\nlayer screen { hsv(x + t, 1, 1) }", 3407),
    ("screen 0.4 on picture", "layer screen { gray(x + t, 0.4) }", 3193),
    ("hsv 0.3 on picture", "hsv(x + t, 1, 1, 0.3)", 3186),
    ("screen hsv 0.4 on picture", "layer screen { hsv(x + t, 1, 1, 0.4) }", 4405),
];
// Formulas as looks: Rings by anylook-a (2026-10-26; again by psi-refresh,
// its source unchanged: 6,032); the rest the bundled Formulas as they are
// now, by psi-refresh (1.1.0-beta.4.probe1, 2026-11-28, the spare test board, 80 MHz flash; firmware/src/psi_probe/refresh.rs): each set as a
// look at its blend and amount, compiled without runs (as `compose`
// below), the VM directly over the golden picture.
pub const LOOKS: &[(&str, &str, &str, u8, u32)] = &[
    ("Plasma", include_str!("../../opendmd-web/formulas/plasma.psi"), "screen", 40, 8071),
    ("Plasma", include_str!("../../opendmd-web/formulas/plasma.psi"), "over", 30, 7226),
    ("Plasma", include_str!("../../opendmd-web/formulas/plasma.psi"), "add", 40, 7602),
    ("Plasma", include_str!("../../opendmd-web/formulas/plasma.psi"), "multiply", 100, 6606),
    ("Plasma", include_str!("../../opendmd-web/formulas/plasma.psi"), "max", 50, 7449),
    ("Fire", include_str!("../../opendmd-web/formulas/fire.psi"), "screen", 40, 8089),
    ("Fire", include_str!("../../opendmd-web/formulas/fire.psi"), "over", 30, 7247),
    ("Rings", "loop 4\n-- r: the distance from the middle\npal(rainbow, frac(r*1.5 - t/2))", "screen", 40, 6614),
    ("Stripes", include_str!("../../opendmd-web/formulas/stripes.psi"), "screen", 40, 6457),
    ("Copper bars", include_str!("../../opendmd-web/formulas/copper-bars.psi"), "screen", 40, 3085),
    ("Old photo", include_str!("../../opendmd-web/formulas/old-photo.psi"), "screen", 50, 8163),
    ("Old TV", include_str!("../../opendmd-web/formulas/old-tv.psi"), "over", 100, 3598),
];
// The draw stage (psi-probe 2026-10-27, 1.0.0-beta.11-drawprobe-a): VM p50
// µs a frame, as STEPS.
pub const DRAW: &[(&str, &str, u32)] = &[
    (
        "draw base (1 dot off)",
        "draw { dot(-1, -1, t) }\ngray(ink)",
        931,
    ),
    (
        "draw loop step: 1024 dots off",
        "draw { for k < 1024 { dot(-1, -1, t) } }\ngray(ink)",
        1694,
    ),
    (
        "draw loop step: +add",
        "draw { for k < 1024 { dot(-1, -1, k + t) } }\ngray(ink)",
        2124,
    ),
    (
        "draw loop step: +hash",
        "draw { for k < 1024 { dot(-1, -1, hash(k, t)) } }\ngray(ink)",
        2235,
    ),
    (
        "draw dot on panel x1024",
        "draw { for k < 1024 { dot(k % 128, 5 + t) } }\ngray(ink)",
        2295,
    ),
    (
        "draw line 12 x256",
        "draw { for k < 256 { line(10, 5, 21, 5 + t) } }\ngray(ink)",
        2257,
    ),
    (
        "draw line 100 x64",
        "draw { for k < 64 { line(10, 5, 109, 5 + t) } }\ngray(ink)",
        3041,
    ),
];
// Formulas that draw played as effects (the real path, `Fx::frame`, the
// JIT off: the VM's frame), p50 µs a frame, compared with the estimate as
// the player compiles them (it includes the real paths' ×1.15). The
// bundled Formulas as they are now, by psi-refresh (1.1.0-beta.4.probe1, 2026-11-28, the spare test board, 80 MHz flash; firmware/src/psi_probe/refresh.rs)
// (drawprobe-a's of 2026-10-27 measured the examples before the eye test).
pub const EFFECTS: &[(&str, &str, u32)] = &[
    ("Starfield", include_str!("../../opendmd-web/formulas/starfield.psi"), 2164),
    ("Wireframe cube", include_str!("../../opendmd-web/formulas/wireframe-cube.psi"), 2588),
];

// psi-speed3 (psi-probe on a test board, 2026-10-28, 1.0.0-beta.11-s3probe-a;
// s2probe-b's before it): the draw rows with their loops run as lanes, VM
// p50 µs a frame, compared over the one-dot base (`DRAW_LANES_BASE`); the
// last two rows: steps laned since psi-speed3, and steps still worked out
// turn by turn (`LANE_EVAL_NS`).
pub const DRAW_LANES_BASE: u32 = 633;
pub const DRAW_LANES: &[(&str, &str, u32)] = &[
    (
        "lanes: 1024 dots off",
        "draw { for k < 1024 { dot(-1, -1, t) } }\ngray(ink)",
        1371,
    ),
    (
        "lanes: +add",
        "draw { for k < 1024 { dot(-1, -1, k + t) } }\ngray(ink)",
        1516,
    ),
    (
        "lanes: +hash",
        "draw { for k < 1024 { dot(-1, -1, hash(k, t)) } }\ngray(ink)",
        1597,
    ),
    (
        "lanes: dot on panel x1024",
        "draw { for k < 1024 { dot(k % 128, 5 + t) } }\ngray(ink)",
        2044,
    ),
    (
        "lanes: line 12 x256",
        "draw { for k < 256 { line(10, 5, 21, 5 + t) } }\ngray(ink)",
        1882,
    ),
    (
        "lanes: line 100 x64",
        "draw { for k < 64 { line(10, 5, 109, 5 + t) } }\ngray(ink)",
        2395,
    ),
    (
        "lanes: laned mod cmp sin",
        "draw { for k < 1024 { dot(-1, -1, sin(k*0.01 + t) + (k % 7 >= 3) + (k % 5 != 1)) } }\ngray(ink)",
        2236,
    ),
    (
        "lanes: eval noise digit sqrt step",
        "draw { for k < 1024 { dot(-1, -1, noise(k*0.1, t, 0.5) + digit(k % 10, 1, 2) + sqrt(k + t) + step(0.5, frac(k*0.3))) } }\ngray(ink)",
        5903,
    ),
];
// The examples' VM frames (`speed2` rows, as compiled with the runs and
// the narrowed steps, unfused compares; with the still cache where it
// applies; slider a = 0.5), the VM directly: the bundled Formulas as they
// are now, by psi-refresh (1.1.0-beta.4.probe1, 2026-11-28, the spare test board, 80 MHz flash; firmware/src/psi_probe/refresh.rs)
// (s2probe-a/b's of 2026-10-28 before them; road-cost's sprite-car and
// spans rows retired with that road).
pub const SPEED2: &[(&str, &str, u32)] = &[
    ("Starfield", include_str!("../../opendmd-web/formulas/starfield.psi"), 1762),
    ("Wireframe cube", include_str!("../../opendmd-web/formulas/wireframe-cube.psi"), 2207),
    ("Tunnel", include_str!("../../opendmd-web/formulas/tunnel.psi"), 3574),
    ("Rotozoom", include_str!("../../opendmd-web/formulas/rotozoom.psi"), 4332),
    ("Rotozoom floor", include_str!("../../opendmd-web/formulas/rotozoom-floor.psi"), 1667),
    ("Endless road", include_str!("../../opendmd-web/formulas/endless-road.psi"), 4778),
    ("Plasma", include_str!("../../opendmd-web/formulas/plasma.psi"), 3328),
    ("Fire", include_str!("../../opendmd-web/formulas/fire.psi"), 3844),
];

// The JIT on a test board (psi-probe JIT2_ONLY, 1.0.0-beta.11-jit2-c,
// 2026-10-29; the bundled rows again by psi-refresh (1.1.0-beta.4.probe1, 2026-11-28, the spare test board, 80 MHz flash; firmware/src/psi_probe/refresh.rs),
// as JIT2_ONLY's: the road-life rows retired for the bundled road, and
// the p0 rows again, their sources unchanged): each bundled formula as an effect on the player's path with
// the JIT on (its kernel, after the self-check and the trial; the still
// cache taken where it has values), p50 µs a frame; and the looks over a
// changing picture. For `Program::esp32_us_jit_still` / `esp32_us_jit`.
pub const JIT_EFFECTS: &[(&str, &str, u32)] = &[
    (
        "Endless road",
        include_str!("../../opendmd-web/formulas/endless-road.psi"),
        3724,
    ),
    (
        "Wireframe cube",
        include_str!("../../opendmd-web/formulas/wireframe-cube.psi"),
        1854,
    ),
    (
        "Plasma letters",
        include_str!("../../opendmd-web/formulas/plasma-letters.psi"),
        5056,
    ),
    (
        "Rotozoom floor",
        include_str!("../../opendmd-web/formulas/rotozoom-floor.psi"),
        1626,
    ),
    (
        "Tunnel",
        include_str!("../../opendmd-web/formulas/tunnel.psi"),
        2802,
    ),
    (
        "Time gate",
        include_str!("../../opendmd-web/formulas/time-gate.psi"),
        2953,
    ),
    (
        "Fire",
        include_str!("../../opendmd-web/formulas/fire.psi"),
        2591,
    ),
    (
        "Plasma",
        include_str!("../../opendmd-web/formulas/plasma.psi"),
        2890,
    ),
    // psi-p0 on a test board (paint80-713cd, 2026-11-04, a paint probe
    // capture, not committed): the effect path
    // with the JIT on, as the other rows (the tween's VM frame is 757: the
    // player keeps the faster). p0 big and p0 tween again by psi-refresh
    // (2026-11-28, the same build as the rows above; tween VM 771): p0 big
    // 1,429 -> 1,729 (VM 2,676) with its program byte for byte the same
    // (the compiler of 2026-11-04 makes the same listing and estimates);
    // the cause is on the device's path, not found yet. Megacity not
    // measured again.
    (
        "p0 Megacity sketch",
        include_str!("device/p0-road-v2-city.psi"),
        6245,
    ),
    (
        "p0 big",
        include_str!("device/p0-big.psi"),
        1729,
    ),
    (
        "p0 tween",
        include_str!("device/p0-tween.psi"),
        822,
    ),
];
pub const JIT_LOOKS: &[(&str, &str, &str, u8, u32)] = &[
    (
        "Old TV",
        include_str!("../../opendmd-web/formulas/old-tv.psi"),
        "over",
        100,
        1862,
    ),
    (
        "Plasma letters",
        include_str!("../../opendmd-web/formulas/plasma-letters.psi"),
        "over",
        100,
        5155,
    ),
    (
        "Plasma",
        include_str!("../../opendmd-web/formulas/plasma.psi"),
        "screen",
        40,
        4277,
    ),
];

// console-b on a test board (psi-probe CONSOLEB_ONLY, 1.0.0-beta.11-consoleb-a,
// 2026-10-29; again by psi-refresh (1.1.0-beta.4.probe1, 2026-11-28, the spare test board, 80 MHz flash; firmware/src/psi_probe/refresh.rs),
// Big clock's rate experiment retired for Storm front, the one bundled
// `layer rate` Formula): the cycled and rate Formulas as effects on the player's path,
// p50 µs a frame, whichever the trial kept (JIT or VM; the still cache
// taken, the held picture held). For `Program::esp32_us_jit_still`.
pub const CONSOLEB: &[(&str, &str, u32)] = &[
    (
        "Cycled plasma",
        include_str!("../../opendmd-web/formulas/cycled-plasma.psi"),
        974,
    ),
    (
        "Waterfall",
        include_str!("../../opendmd-web/formulas/waterfall.psi"),
        1073,
    ),
    (
        "Copper sky",
        include_str!("../../opendmd-web/formulas/copper-sky.psi"),
        2206,
    ),
    (
        "Storm front",
        include_str!("../../opendmd-web/formulas/storm-front.psi"),
        5778,
    ),
];

// The paint primitives (paint-probe on a test board, 2026-11-04,
// 1.0.0-beta.12-paint80-15d18, 80 MHz flash): each `for k < 16 { … }` over
// `gray(x)` with a sprite `ship` (5 × 4), the frame steps and the draw stage
// alone (the VM's stamps), p50 µs a frame: the fit set for `paint_ns`.
pub const PAINT_SPRITE: &str = "sprite ship {\n  key . 0, W #fff\n  ..W..\n  .WWW.\n  WWWWW\n  W.W.W\n}\n";
pub const PAINT: &[(&str, &str, u32)] = &[
    ("base", "pset(-9, -9, #f00)", 43),
    ("pset", "pset(64, 16, #f00)", 45),
    ("line 1", "line(10, 16, 10, 16, #f00)", 58),
    ("line 32", "line(0, 16, 31, 16, #f00)", 213),
    ("line 128", "line(0, 0, 127, 31, #f00)", 695),
    ("rect off", "rect(-30, -30, -20, -20, #f00)", 56),
    ("rect 64", "rect(60, 12, 67, 19, #f00)", 224),
    ("rect 2048", "rect(32, 0, 95, 31, #f00)", 4347),
    ("rectfill off", "rectfill(-30, -30, -20, -20, #f00)", 56),
    ("rectfill 64", "rectfill(60, 12, 67, 19, #f00)", 237),
    ("rectfill 2048", "rectfill(32, 0, 95, 31, #f00)", 4975),
    ("circ off", "circ(-40, 16, 6, #f00)", 99),
    ("circ 100", "circ(64, 16, 3, #f00)", 1121),
    ("circ 1024", "circ(64, 16, 14, #f00)", 14081),
    ("circfill off", "circfill(-40, 16, 6, #f00)", 99),
    ("circfill 100", "circfill(64, 16, 3, #f00)", 763),
    ("circfill 1024", "circfill(64, 16, 14, #f00)", 7056),
    ("trifill off", "trifill(-30, -30, -20, -30, -25, -20, #f00)", 68),
    ("trifill 110", "trifill(60, 12, 68, 12, 64, 20, #f00)", 1782),
    ("trifill 4096", "trifill(0, 31, 64, 0, 127, 31, #f00)", 54830),
    ("spr off", "spr(ship, -20, -20)", 155),
    ("spr 20", "spr(ship, 60, 4)", 166),
    ("print 5", "print(\"SCORE\", 2, 1, #ff0)", 453),
    ("print num 4", "print(1234, 30, 1, #ff0, small, 4)", 329),
    ("cls", "cls(#f00)", 130),
];
// The paint plane over a frame (the same run): the VM's rows, µs a frame,
// with no plane, an empty one and a full one (`cls`); the overlay alone 197
// and 401.
pub const OVERLAY_ROWS: [u32; 3] = [700, 896, 1101];
// `under(x, y)` looks over the golden picture (the same run): the VM's frame
// directly, p50 µs (the real path adds ~5 %).
pub const UNDER: &[(&str, &str, u32)] = &[
    ("uat 3", "look\nunder(px + a, py)\n", 5068),
    ("uat 6", "look\nunder(px + a, py)*0.5 + under(px - a, py)*0.5\n", 10314),
    (
        "uat 12",
        "look\nunder(px + a, py)*0.25 + under(px - a, py)*0.25 + under(px, py + a)*0.25 + under(px, py - a)*0.25\n",
        19786,
    ),
];
// Formulas that paint or read the picture elsewhere, on the real path (the
// same run; palm and paint-heavy as looks: they paint without a colour
// line), VM and JIT p50 µs a frame.
pub const PAINT_FORMULAS: &[(&str, &str, u32, u32)] = &[
    ("functions", include_str!("device/functions.psi"), 7330, 6799),
    ("paint-heavy", include_str!("device/paint-heavy.psi"), 6407, 5569),
    ("palm", include_str!("device/palm.psi"), 3233, 2385),
    // (The JIT's under(x, y) template, the second run, paint80-f6aa6.)
    ("pixelate", include_str!("device/pixelate.psi"), 5318, 2645),
    ("under-shake", include_str!("device/under-shake.psi"), 5312, 2563),
    // (Neon city: the gallery's, its storm, bolt, palette tween and rain
    // included, by psi-refresh, 2026-11-28, 1.1.0-beta.4.probe1: the real
    // path, JIT off and on.)
    ("Neon city", include_str!("../../opendmd-web/formulas/neon-city.psi"), 5430, 4224),
];

/// P0b-4 and the guards a row on a test board (paint80-01e06, 2026-11-04,
/// a paint probe capture (not committed), the paint
/// probe's `cmp` lines): direct frames from `start` ms (seed
/// `golden::SEED`), µs, VM and kernel, each against its frame's own
/// estimate. `fused`: compiled with the fused compares. The sources are
/// firmware/build.rs's (`u` reads `t`: no still cache).
pub const P0B4_U: &str = "loop 8\nu = frac(x + t/8)\n";
pub const P0B4: &[(&str, &str, bool, u32, u32, u32)] = &[
    ("sel4 fused", "gray(frac((u < y)*u + (y < frac(u*3))*y + (u*2 < y)*0.5 + (y <= u*0.7)*0.25))", true, 0, 2781, 1134),
    ("sel4 unfused", "gray(frac((u < y)*u + (y < frac(u*3))*y + (u*2 < y)*0.5 + (y <= u*0.7)*0.25))", false, 0, 2645, 1308),
    ("cnt2 fused", "gray(frac(((u >= y) + (u >= y*0.5))*0.2 + ((y >= u*0.5) + (y >= u*0.3))*0.2))", true, 0, 2232, 1034),
    ("cnt2 unfused", "gray(frac(((u >= y) + (u >= y*0.5))*0.2 + ((y >= u*0.5) + (y >= u*0.3))*0.2))", false, 0, 2286, 1248),
    ("guards none", "", true, 0, 6358, 3859),
    ("guards on", "t >= 0", true, 0, 6634, 3944),
    ("guards off", "t < 0", true, 0, 2027, 1212),
    ("guards rows", "py < 16", true, 0, 4345, 2589),
    ("guards base", "gray(frac(u*0.1))", true, 0, 729, 519),
    ("road-v2 exit fused", "road", true, 2_056_049, 12984, 7103),
    ("road-v2 exit unfused", "road", false, 2_056_049, 12884, 8609),
    ("road-v2 mouth fused", "road", true, 733_049, 12946, 7091),
    ("road-v2 mouth unfused", "road", false, 733_049, 12760, 8636),
    ("road-v2 forest-desert fused", "road", true, 544_049, 10353, 6514),
    ("road-v2 forest-desert unfused", "road", false, 544_049, 10407, 7269),
    ("road-v2 desert fused", "road", true, 2_177_549, 8866, 5607),
    ("road-v2 desert unfused", "road", false, 2_177_549, 8902, 6127),
];

/// The `guards` cases' source: three scenes `when` (none for "").
pub fn p0b4_guards(when: &str) -> String {
    let mut f = String::from(P0B4_U);
    for k in 1..=3 {
        match when {
            "" => f += &format!("b{k}a = sin(u*{k}.7 + y)*0.3\nb{k}b = sin(y*{k}.3 + u)*0.3\n"),
            w => f += &format!(
                "scene s{k} when {w} {{\n  b{k}a = sin(u*{k}.7 + y)*0.3\n  b{k}b = sin(y*{k}.3 + u)*0.3\n}}\n"
            ),
        }
    }
    f + "gray(frac(u*0.1 + b1a + b1b + b2a + b2b + b3a + b3b))\n"
}

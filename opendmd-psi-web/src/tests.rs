//! Natively (`cargo test`) and in wasm under node (`wasm-pack test --node`):
//! the module draws the golden frames — the hashes the host, wasm32 and the
//! ESP32 agree on — and reports errors where JS will look for them. One
//! test: the formula is a static.

use super::*;
use opendmd_psi::golden::{fnv, inputs, picture, DRAWN, FRAMES, OVER};

#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test as test;

fn drawn(ms: u32) -> &'static [u8] {
    frame(ms, 0, 0, 0, 0, 0, 0);
    // SAFETY: the static frame, read before the next call.
    unsafe { core::slice::from_raw_parts(pixels(), W * H * 3) }
}

#[test]
fn formulas() {
    // Black before anything compiled.
    assert!(info().is_empty());
    assert!(drawn(0).iter().all(|&b| b == 0));

    // The golden frames (`seed` theirs).
    set_seed(opendmd_psi::golden::SEED);
    for &(name, src, ms, want) in FRAMES {
        assert!(compile(src), "{name}: {}", error());
        let i = inputs(ms);
        frame(ms, i.hour, i.minute, i.second, i.a, i.b, i.c);
        let got = fnv(unsafe { core::slice::from_raw_parts(pixels(), W * H * 3) });
        assert_eq!(got, want, "{name} at {ms} ms");
    }

    // What it is.
    assert!(compile("loop 4\nrgb(a, b * x, 0)"));
    let i = info();
    assert_eq!(i[4], 4000);
    assert_eq!(i[5], 0b011);
    assert!(i[0] >= 2 && i[0] == i[1] + i[2] + i[3]);
    assert!(i[6] > 0);
    assert_eq!(i.len(), 15);
    assert!(i[14] > 0 && i[14] <= i[6], "the still estimate: never more than the VM's");
    assert_eq!(i[10], 0, "no `fps` line: the player's 20 ms");
    assert!(compile("fps 100\nloop 4\ngray(x)"));
    assert_eq!(
        (info()[4], info()[10]),
        (4000, 10),
        "`fps 100`: a 10 ms grid"
    );

    // An error keeps the last formula drawn; the span is in JS's units
    // ("é" is 2 bytes in UTF-8, one unit in JS).
    assert!(compile("gray(0.5)"));
    let before = drawn(0).to_vec();
    let src = "-- é\nv = x\ngray(tme)";
    assert!(!compile(src));
    assert_eq!(error(), "unknown name `tme` — did you mean `t`?");
    assert_eq!(error_at(), [16, 19]);
    assert_eq!(src.chars().skip(16).take(3).collect::<String>(), "tme");
    assert_eq!(
        drawn(0),
        &before[..],
        "the last good formula is still drawn"
    );
    assert!(compile("gray(x)"));
    assert!(error().is_empty());

    // Over a picture: the golden frames over the golden picture.
    // SAFETY: the static buffer, written before the next call.
    picture(unsafe { core::slice::from_raw_parts_mut(under(), W * H * 3) });
    for &(name, src, ms, over, want) in OVER {
        assert!(compile(src), "{name}: {}", error());
        let i = inputs(ms);
        if over {
            frame_over(ms, i.hour, i.minute, i.second, i.a, i.b, i.c);
        } else {
            frame(ms, i.hour, i.minute, i.second, i.a, i.b, i.c);
        }
        let got = fnv(unsafe { core::slice::from_raw_parts(pixels(), W * H * 3) });
        assert_eq!(got, want, "{name} at {ms} ms");
    }
    // Formulas that draw, with the module's canvas.
    for &(name, src, ms, over, want) in DRAWN {
        assert!(compile(src), "{name}: {}", error());
        assert!(info()[8] > 0 && info()[9] > 0, "{name}: draws");
        let i = inputs(ms);
        if over {
            frame_over(ms, i.hour, i.minute, i.second, i.a, i.b, i.c);
        } else {
            frame(ms, i.hour, i.minute, i.second, i.a, i.b, i.c);
        }
        let got = fnv(unsafe { core::slice::from_raw_parts(pixels(), W * H * 3) });
        assert_eq!(got, want, "{name} (drawn) at {ms} ms");
    }
    assert!(compile(
        "layer multiply { gray(ul) }\nlayer add { gray(x, 0.5) }"
    ));
    let i = info();
    assert_eq!(
        (i[5] & 32, i[7]),
        (32, 2),
        "draws over what is under, two layers"
    );

    // A look is the formula's own `look` line: set, changed and taken out
    // as text, then drawn as the panel draws it (asLook).
    let s = look("hsv(x, 1, 1)", "screen", 40);
    assert_eq!(s, "look screen 40\nhsv(x, 1, 1)");
    assert_eq!(
        as_look(&s),
        "part look {\n  hsv(x, 1, 1)\n}\nlayer screen { look(x, y, 0.4) }"
    );
    assert_eq!((info()[5] & 32, info()[7]), (32, 1));
    assert_eq!(look("gray(ul)", "over", 100), "look\ngray(ul)");
    assert_eq!(as_look("look\ngray(ul)"), "look\ngray(ul)");
    assert_eq!(as_look("gray(ul)"), "gray(ul)");
    assert_eq!(look("look multiply 20\nhsv(x, 1, 1)", "over", 30), "look over 30\nhsv(x, 1, 1)");
    assert_eq!(look("look multiply 20\nhsv(x, 1, 1)", "", 0), "hsv(x, 1, 1)");
    assert_eq!(look("gray(x)", "lighten", 40), "");
    // The other lines, in their places among the declarations.
    let d = declare("-- hi\nloop 4\ngray(x)", "name", 0, "name \"Grey\"");
    assert_eq!(d, "name \"Grey\"\n-- hi\nloop 4\ngray(x)");
    let d = declare(&look(&d, "add", 50), "control", 0, "control k = 0.5");
    assert_eq!(d, "name \"Grey\"\ncontrol k = 0.5\nlook add 50\n-- hi\nloop 4\ngray(x)");
    let d = declare(&d, "about", 0, "about \"A grey ramp\"");
    assert_eq!(declare(&d, "control", 1, "control m = a, b"), "name \"Grey\"\nabout \"A grey ramp\"\ncontrol k = 0.5\ncontrol m = a, b\nlook add 50\n-- hi\nloop 4\ngray(x)");
    assert_eq!(declare(&d, "control", 0, ""), "name \"Grey\"\nabout \"A grey ramp\"\nlook add 50\n-- hi\nloop 4\ngray(x)");
    assert_eq!(declare(&d, "name", 0, "name \"G\""), d.replace("Grey\"", "G\""));
    // `psi 1` stays first; `name = …` is a value, not a declaration.
    assert_eq!(declare("psi 1\ngray(x)", "name", 0, "name \"P\""), "psi 1\nname \"P\"\ngray(x)");
    assert_eq!(declare("name = 3\ngray(name)", "name", 0, ""), "name = 3\ngray(name)");
    assert_eq!(declare("look", "about", 0, "about \"x\""), "about \"x\"\nlook");
    // A path's text, its statements `; `-joined (a look read from a path).
    let p = "name \"Go; ne\"; look add 25; loop 4; gray(x)";
    assert_eq!(declare(p, "name", 0, "name \"G\""), "name \"G\"; look add 25; loop 4; gray(x)");
    assert_eq!(declare(p, "name", 0, ""), "look add 25; loop 4; gray(x)");
    assert_eq!(declare(p, "about", 0, "about \"x\""), "name \"Go; ne\"; about \"x\"; look add 25; loop 4; gray(x)");
    assert_eq!(look(p, "screen", 40), "name \"Go; ne\"; look screen 40; loop 4; gray(x)");
    assert_eq!(look(p, "", 0), "name \"Go; ne\"; loop 4; gray(x)");
    assert_eq!(look("name \"x\"; gray(x)", "over", 100), "name \"x\"; look; gray(x)");
    // A comment between declarations, one after a declaration.
    assert_eq!(declare("name \"a\" -- n; o\n-- c\nlook\ngray(x)", "control", 0, "control k"), "name \"a\" -- n; o\ncontrol k\n-- c\nlook\ngray(x)");

    // What a formula declares, for the page's controls.
    assert_eq!(
        controls("name \"Zoom é\"\ncontrol zoom 1..4 = 2.5 \"Zoom\"\ncontrol wob = gentle, strong, off\nlook screen 40\ngray(zoom/4 + wob*0.1 + c)"),
        r#"{"name":"Zoom é","about":null,"look":["screen",40],"own":2,"controls":[{"slot":"a","name":"zoom","label":"Zoom","kind":"slider","lo":1,"hi":4,"default":2.5,"declared":true},{"slot":"b","name":"wob","label":"wob","kind":"choice","words":["gentle","strong","off"],"default":0,"declared":true},{"slot":"c","name":"c","label":"c","kind":"slider","lo":0,"hi":1,"default":0,"declared":false}]}"#
    );
    assert_eq!(
        controls("control k -0.5..0.3 = -0.25\ngray(x)"),
        r#"{"name":null,"about":null,"look":null,"own":1,"controls":[{"slot":"a","name":"k","label":"k","kind":"slider","lo":-0.5,"hi":0.3,"default":-0.25,"declared":true}]}"#
    );
    assert_eq!(controls("gray(x)"), r#"{"name":null,"about":null,"look":null,"own":0,"controls":[]}"#);
    assert_eq!(controls("gray(nope)"), "");
    // (The formula drawn is unchanged by it.)
    assert!(compile("control k 0..1 = 0.25\ngray(k)"));
    let _ = controls("gray(1)");
    // An unset slider plays at the declared default.
    frame(0, 0, 0, 0, UNSET, UNSET, UNSET);
    let unset = unsafe { core::slice::from_raw_parts(pixels(), W * H * 3) }.to_vec();
    frame(0, 0, 0, 0, 1 << 14, 0, 0);
    assert_eq!(unsafe { core::slice::from_raw_parts(pixels(), W * H * 3) }, &unset[..]);
    assert_eq!(unset[0], 64);
    // (The module's state is a static: one test.)
    #[cfg(not(target_arch = "wasm32"))]
    every_example_as_a_look();
}

/// The examples gallery: every one compiles, is short, and is light or
/// medium for the panel (the ESP32 estimate within 10 ms: since the
/// estimate was calibrated on the device, 2026-10-26, the heaviest five read
/// 8.3-9.7 ms, which is how they always ran: at 25 frames a second).
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn the_examples_are_light_enough_for_the_panel() {
    let all = crate::examples();
    assert!(all.len() >= 33, "{} examples", all.len());
    // The ten the web app (and so the panel) ships: the owner's (formula-apps
    // §3.1, then Endless road and Time gate in for Clock and Corner clock):
    // five overlays, Big clock, four screensavers. The rest are the site
    // gallery's.
    let bundled: Vec<&str> = all.iter().filter(|e| e.bundled).map(|e| e.name.as_str()).collect();
    assert_eq!(
        bundled,
        [
            "Big clock", "Endless road", "Glitter", "Matrix", "Old photo", "Old TV",
            "Scanlines", "Starfield", "Time gate", "Vignette"
        ]
    );
    for e in &all {
        let p = opendmd_psi::compile(&e.src)
            .unwrap_or_else(|err| panic!("{}: {}", e.name, err.render(&e.src)));
        // Each says what it is: its name (within the gallery's 40) and a line
        // about it, in its own declarations; the file is named after it.
        assert!(!e.name.is_empty() && e.name.chars().count() <= 40 && !e.about.is_empty(), "{e:?}");
        let slug: String = e
            .name
            .to_lowercase()
            .replace('é', "e")
            .split(|c: char| !c.is_ascii_alphanumeric())
            .filter(|w| !w.is_empty())
            .collect::<Vec<_>>()
            .join("-");
        assert_eq!(e.slug, slug, "{}: the file's name", e.name);
        // One that draws over the GIF says it can be the look (Settings ›
        // Look lists only those that declare it).
        assert!(
            !p.over() || p.meta().look().is_some(),
            "{}: draws over the GIF without a `look` line",
            e.name
        );
        assert!(
            p.meta().look().is_none() || p.over(),
            "{}: a `look` line on a formula that covers the GIF",
            e.name
        );
        // Short: seven lines, not counting comments or the parts pasted in.
        let mut depth = 0;
        let own = e
            .src
            .lines()
            .filter(|l| {
                let inside = depth > 0 || l.trim_start().starts_with("part ");
                depth += l.matches('{').count() as i32 - l.matches('}').count() as i32;
                // (Nor the declarations: what it is, not what it draws.)
                let decl = ["name ", "about ", "control ", "look", "psi "]
                    .iter()
                    .any(|w| l.starts_with(w));
                !inside && !decl && !l.trim_start().starts_with("--")
            })
            .count();
        // The showcase, Big clock, reads as a commented step a line
        // (owner, 2026-10-26: legible over short); the rest stay short.
        // So do the built-in effects' Formulas (Tunnel, Rotozoom): a native
        // effect's knobs, written out.
        // Time gate is a showcase too: the per-row tables are a line a
        // quantity (19 lines of code, 6.1 ms with its still cache).
        // Endless road is a showcase too: its tables (the car, the peaks,
        // the palm) and five draw loops, written tight for the path; and
        // so are its two alternatives, Neon city and Wormhole run.
        let most = if ["Endless road", "Neon city", "Wormhole run"].contains(&e.name.as_str()) {
            80
        } else if e.name == "Time gate" {
            // 20 since `psi fmt` (2026-10-29): one statement a line (24
            // since the `frame { }` block's lines count).
            30
        } else if ["Big clock", "Tunnel", "Rotozoom", "Rotozoom floor", "Matrix"].contains(&e.name.as_str()) {
            30
        } else {
            // (30 since the `frame { }` blocks went: their lines count.)
            30
        };
        assert!(own <= most, "{}: too long for an example", e.name);
        // (As the player draws it: with its still cache, when it has one.)
        assert!(
            p.esp32_us_still() <= 10_000,
            "{}: ~{} µs a frame on the panel",
            e.name,
            p.esp32_us_still()
        );
        // A slider it reads is declared (a `control` line: its name,
        // label and default), not named in a comment.
        for k in 0..3 {
            let used = p.sliders() & (1 << k) != 0;
            assert!(
                !used || p.meta().declared() & (1 << k) != 0,
                "{}: slider {} without a control line",
                e.name,
                ['a', 'b', 'c'][k]
            );
        }
        assert!(
            !e.src.lines().any(|l| l.trim_start().starts_with("-- a:")),
            "{}: the `-- a:` comment is retired",
            e.name
        );
    }
}

/// Every example compacted (what an inline path and a Formula read from its
/// file on the card carry: js/psikit.js `path`, `fx_psi::read_text`, the
/// firmware's bundled list) compiles to the same program. (No size test:
/// playlists and looks name Formulas, `@formula:<stem>` or their file, so
/// the inline cap no longer shapes them.)
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn every_example_compacts_to_the_same_program() {
    for e in crate::examples() {
        let s = opendmd_psi::compact(&e.src);
        assert_eq!(
            opendmd_psi::compile(&s).map(|p| p.len()).ok(),
            opendmd_psi::compile(&e.src).map(|p| p.len()).ok(),
            "{}",
            e.name
        );
    }
}

/// Every example pasted into another formula as a part (as the editor's
/// `use … as p` does: indented, under a comment) and drawn whole: the same
/// frames, over the golden picture too.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn every_example_pasted_in_as_a_part_draws_the_same() {
    use opendmd_psi::{golden, render_rows_ink, Scratch};
    let mut under = vec![0u8; W * H * 3];
    golden::picture(&mut under);
    let draw = |src: &str, ms: u32| {
        let p = opendmd_psi::compile(src).unwrap_or_else(|e| panic!("{}", e.render(src)));
        let mut out = vec![0u8; W * H * 3];
        render_rows_ink::<128>(
            &p,
            &mut Scratch::new(),
            Some(&mut opendmd_psi::Ink::boxed()),
            &golden::inputs(ms),
            &OWN,
            Some(&under),
            u32::MAX,
            &mut out,
        );
        (out, p.loop_ms())
    };
    for e in crate::examples() {
        // (`rate` is left out inside a part: a slow layer that moves draws
        // every frame there, not as the formula does.)
        if opendmd_psi::compile(&e.src).is_ok_and(|p| p.rate().is_some()) {
            continue;
        }
        let inner: String = e.src.lines().map(|l| format!("  {l}\n")).collect();
        for ms in [0, 1730, 6420] {
            let (want, lp) = draw(&e.src, ms);
            let lp = if lp == 0 {
                "forever".to_string()
            } else {
                format!("{}", lp as f64 / 1000.0)
            };
            let src = format!(
                "loop {lp}\npart p {{\n  -- {}\n{inner}}}\nlayer {{ p }}",
                e.name
            );
            assert!(draw(&src, ms).0 == want, "{} at {ms} ms", e.name);
        }
    }
}

/// Every example as a look, at the choice Settings starts it at (js/looks.js
/// `choice`): one that draws over the GIF as it is (over, 100 %); one that
/// covers it, screened at 40 %, or over at 30 % when that is too heavy (or
/// it has layers of its own, which go over only) —
/// within the looks' 12 ms (`fx_psi::LOOK_MAX_EST_US`), except the ones
/// listed: too heavy to draw over every GIF at all.
#[cfg(not(target_arch = "wasm32"))]
fn every_example_as_a_look() {
    // (Tunnel: ~4.6 ms as an effect with its still cache, ~14.4 without
    // one, and a look never has one. Time gate: ~6.1 ms and ~15.6. Storm
    // front: its slow clouds are drawn every frame in a look, two noises a
    // pixel. Wormhole run: ~4.7 ms with its still cache (the sky's haze
    // and stars, the wormhole's distance), ~12.9 without.)
    const TOO_HEAVY: &[&str] = &["Storm front", "Time gate", "Tunnel", "Wormhole run"];
    let mut heavy = Vec::new();
    let mut chosen = Vec::new();
    for e in crate::examples() {
        assert!(compile(&e.src), "{}: {}", e.name, error());
        let over = info()[5] & 32 != 0;
        let tries: &[(&str, u8)] = if over {
            &[("over", 100)]
        } else {
            &[("screen", 40), ("over", 30)]
        };
        let fit = tries.iter().find(|&&(b, k)| {
            // The line set, then the look the panel draws for it.
            let s = as_look(&look(&e.src, b, k));
            // A formula with layers of its own goes over only.
            if s.is_empty() && b == "screen" && error().contains("goes over the GIF as a look") {
                return false;
            }
            assert!(!s.is_empty(), "{} {b} {k}: {}", e.name, error());
            let i = info();
            assert!(
                i[5] & 32 != 0,
                "{} {b} {k}: lets the GIF show through",
                e.name
            );
            if over {
                assert_eq!(s, look(&e.src, b, k), "{}: the formula itself", e.name);
            } else {
                assert_eq!(
                    s,
                    opendmd_psi::look::compose(&e.src, b, k).unwrap(),
                    "{}: the line is the composition",
                    e.name
                );
            }
            i[6] <= 12_000
        });
        match fit {
            Some((b, k)) => chosen.push(format!("{} {b} {k}", e.name)),
            None => heavy.push(e.name.clone()),
        }
    }
    assert_eq!(heavy, TOO_HEAVY, "chosen: {chosen:?}");
    // Another blend for a formula with layers is the compiler's error.
    assert!(as_look(&look("layer multiply { gray(0.5) }", "screen", 40)).is_empty());
    assert!(error().contains("goes over the GIF as a look"), "{}", error());
}

/// Every example's frames at its defaults, as they were before the
/// examples' `-- a:` comments became `control` lines (formula-apps F1,
/// 2026-10-28: the JSON's `a` then, the declared default now, which the
/// player and the page use when a slot is unset), over the golden picture.
#[cfg(not(target_arch = "wasm32"))]
const EXAMPLE_FRAMES: &[(&str, u32, u64)] = &[
    ("Big clock", 0, 0x5671a92327d0e76d),
    ("Big clock", 1730, 0x70ef55ab888f0011),
    ("Big clock", 6420, 0x2e757d511ef7ca23),
    ("Starfield", 0, 0xd4e070cecf44fe93),
    ("Starfield", 1730, 0xaf7d5b6be57277aa),
    ("Starfield", 6420, 0x90e3758ef743ba71),
    ("Wireframe cube", 0, 0x3c481b55b889f211),
    ("Wireframe cube", 1730, 0x1db6636a007350c1),
    ("Wireframe cube", 6420, 0xf31ba094a7ba7337),
    ("Tunnel", 0, 0xe94602d1652e1129),
    ("Tunnel", 1730, 0x5b78122dff79a963),
    ("Tunnel", 6420, 0xea814a0f353fa2cf),
    ("Rotozoom", 0, 0x6ad2db7bf82cddb6),
    ("Rotozoom", 1730, 0xc16b9f2e65b5fdae),
    ("Rotozoom", 6420, 0xe57a5bd42332c352),
    ("Rotozoom floor", 0, 0xab6b6c0c1c05d139),
    ("Rotozoom floor", 1730, 0x66b37879e7a17a09),
    ("Rotozoom floor", 6420, 0x5af9feaa3a8b41f5),
    // Re-pinned for road-full (2026-10-29): Full bundled, the frames of
    // experiments/road-life/road-full.psi (psi-check --hash, 3 seeds × 142
    // frames over a day, all the same).
    ("Endless road", 0, 0x0688870f3d2fa467),
    ("Endless road", 1730, 0xdbabb755250ccc83),
    ("Endless road", 6420, 0xe6076a6dca5a376d),
    ("Time gate", 0, 0x24bd9942453ef2e8),
    ("Time gate", 1730, 0x7787a7d7cc3f723e),
    ("Time gate", 6420, 0x2a17edb7a1ade755),
    ("Plasma", 0, 0x240bb68b2b00acf3),
    ("Plasma", 1730, 0x4c5a8909ed18b7ae),
    ("Plasma", 6420, 0x134d518c3dbbaf84),
    ("Fire", 0, 0xc3bd2977c66368c0),
    ("Fire", 1730, 0xaea0b3dc586e6f2c),
    ("Fire", 6420, 0x2f3083cf9d7fa2a0),
    ("Rain", 0, 0x6f4f326c2dee6b52),
    ("Rain", 1730, 0x75468f97617c2d11),
    ("Rain", 6420, 0x8f94da3025fe1fec),
    ("Matrix", 0, 0xfaae4ed4783bbda8),
    ("Matrix", 9730, 0xec157dd618adb5e3),
    ("Matrix", 21420, 0x859a43082b013930),
    ("Lava lamp", 0, 0x147fb5afe15bdce1),
    ("Lava lamp", 1730, 0xdf3f08c182e57534),
    ("Lava lamp", 6420, 0x23a9774c63cba823),
    ("Rings", 0, 0xf2cca724804083b5),
    ("Rings", 1730, 0x4e985fe095de8d29),
    ("Rings", 6420, 0x75de8c7a3137a917),
    ("Moiré", 0, 0x2bcfae197227fd8e),
    ("Moiré", 1730, 0xafa70d932caf84a0),
    ("Moiré", 6420, 0x9d9d7bcaa44b6fd8),
    ("XOR pattern", 0, 0x3d89f2b7439e8625),
    ("XOR pattern", 1730, 0xbd877aa5f66e5925),
    ("XOR pattern", 6420, 0x49a1218e8c889d65),
    ("Stripes", 0, 0x0a53a451bf3de0ed),
    ("Stripes", 1730, 0x05a99347bbcaae41),
    ("Stripes", 6420, 0xd4ba0313857c3a7f),
    ("Clock", 0, 0x1c859c9432c78b42),
    ("Clock", 1730, 0x1c859c9432c78b42),
    ("Clock", 6420, 0x1c859c9432c78b42),
    ("CRT", 0, 0xaa8baed99c3bc884),
    ("CRT", 1730, 0xd2139818ebfe2b2d),
    ("CRT", 6420, 0x757aac8dcffdce1e),
    ("Rainbow wave", 0, 0x452a5ceff4a49295),
    ("Rainbow wave", 1730, 0x7e1aa7ac7a23dded),
    ("Rainbow wave", 6420, 0xd2645bea0a67abe2),
    ("Spotlight", 0, 0xcf55f10e666af368),
    ("Spotlight", 1730, 0x877bf5fbc38a6066),
    ("Spotlight", 6420, 0x85322417781d8056),
    ("Copper bars", 0, 0xd65a063a3b24a925),
    ("Copper bars", 1730, 0xf229f95cbec2ed25),
    ("Copper bars", 6420, 0xbd475933b7c45625),
    ("Bars to stars", 0, 0x862d5be567aebd54),
    ("Bars to stars", 1730, 0x2e1e0904e4748c3d),
    ("Bars to stars", 6420, 0xe0a25bef73e17289),
    ("Rings through stripes", 0, 0x0cce44831f18a402),
    ("Rings through stripes", 1730, 0xbd9030d90c96e5c8),
    ("Rings through stripes", 6420, 0x87e61aad5e38e649),
    ("Kaleidoscope", 0, 0xc414c7245ecf81a0),
    ("Kaleidoscope", 1730, 0x385590dddc6f72fe),
    ("Kaleidoscope", 6420, 0x0ac44860ea5f2fa5),
    ("Scanlines", 0, 0x2e1c992bbc8c9f5a),
    ("Scanlines", 1730, 0x2e1c992bbc8c9f5a),
    ("Scanlines", 6420, 0x2e1c992bbc8c9f5a),
    ("Glitter", 0, 0x0ef799246d6d4252),
    ("Glitter", 1730, 0x3cc2113373285791),
    ("Glitter", 6420, 0x4325e334249f1715),
    ("Old photo", 0, 0x8a96329730876e8a),
    ("Old photo", 1730, 0x8a96329730876e8a),
    ("Old photo", 6420, 0x8a96329730876e8a),
    ("Vignette", 0, 0x453b285a00fb5235),
    ("Vignette", 1730, 0x453b285a00fb5235),
    ("Vignette", 6420, 0x453b285a00fb5235),
    ("Corner clock", 0, 0xf9ac5e7955b4a76c),
    ("Corner clock", 1730, 0xf9ac5e7955b4a76c),
    ("Corner clock", 6420, 0xf9ac5e7955b4a76c),
    ("Old TV", 0, 0x2a5a1b307957cfe2),
    ("Old TV", 1730, 0x8b53ce2d8fe16652),
    ("Old TV", 6420, 0x596b9ec7904d3353),
    ("Plasma letters", 0, 0x937ede84e51c5360),
    ("Plasma letters", 1730, 0xb316afe62da32778),
    ("Plasma letters", 6420, 0xd2757fe31150dfda),
    ("Searchlight", 0, 0xd0c55470814a1ec4),
    ("Searchlight", 1730, 0x27134696e6b29b96),
    ("Searchlight", 6420, 0xcb61313dcf7f82d9),
    // console-b (2026-10-29): `cycle` and `layer rate N`.
    ("Cycled plasma", 0, 0x14b6efec60d6a3a2),
    ("Cycled plasma", 1730, 0x7b793a80ab8ff5c3),
    ("Cycled plasma", 6420, 0x3928057f5f1044f8),
    ("Waterfall", 0, 0x48cec5829abeb314),
    ("Waterfall", 1730, 0x2883fa1fa89f947c),
    ("Waterfall", 6420, 0xc139dc8e6cbb20cd),
    ("Copper sky", 0, 0x37d19a046136c611),
    ("Copper sky", 1730, 0x30938cc2cfb8aa77),
    ("Copper sky", 6420, 0x37d16e5a38a4527b),
    ("Storm front", 0, 0x6d0e0e7d3130b115),
    ("Storm front", 1730, 0x66c4db3c49640df1),
    ("Storm front", 6420, 0x14020362b551311d),
    // road-alts (2026-10-30): Endless road's alternatives.
    ("Neon city", 0, 0x13e664bf85322932),
    ("Neon city", 1730, 0xfeb6ace546e7770b),
    ("Neon city", 6420, 0xde41e747ec3eff17),
    ("Wormhole run", 0, 0x72b7572f97630448),
    ("Wormhole run", 1730, 0xe9ce3bd5322c9207),
    ("Wormhole run", 6420, 0xbea23f200e4f7996),
];

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn every_example_draws_as_before_at_its_defaults() {
    use opendmd_psi::{golden, render_rows_ink, Scratch};
    let mut under = vec![0u8; W * H * 3];
    golden::picture(&mut under);
    let all = crate::examples();
    // (Pinned by name: the files are read in file-name order, the table is
    // in formulas.json's old order.)
    assert_eq!(all.len() * 3, EXAMPLE_FRAMES.len());
    let mut bad = 0;
    for e in &all {
        let p = opendmd_psi::compile(&e.src).unwrap();
        let want: Vec<_> = EXAMPLE_FRAMES.iter().filter(|w| w.0 == e.name).collect();
        assert_eq!(want.len(), 3, "{}: not pinned", e.name);
        for &&(name, ms, hash) in &want {
            let mut i = golden::inputs(ms);
            [i.a, i.b, i.c] = p.meta().values([None; 3]);
            i.set_ms(&p, u64::from(ms));
            let mut out = vec![0u8; W * H * 3];
            render_rows_ink::<128>(
                &p,
                &mut Scratch::new(),
                Some(&mut opendmd_psi::Ink::boxed()),
                &i,
                &OWN,
                Some(&under),
                u32::MAX,
                &mut out,
            );
            if golden::fnv(&out) != hash {
                std::println!("BLESS (\"{name}\", {ms}, {:#018x}),", golden::fnv(&out));
                bad += 1;
            }
        }
    }
    assert_eq!(bad, 0, "pinned example frames differ (BLESS lines above)");
}

/// (Pinned frames: `BLESS` lines printed for a deliberate change.)
#[allow(dead_code)]
const _BLESS: () = ();

/// The web app's path for a Formula (`js/psikit.js` `path`, which Show on
/// panel sends) is `fx_psi::path_of`: `@formula?psb=`, the compiled
/// Formula in base64url (unpadded), then the values. Run under node, with
/// the compiler stood in for by the bytes.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn the_web_apps_path_is_the_players() {
    let kit = concat!(env!("CARGO_MANIFEST_DIR"), "/../opendmd-web/www/js/psikit.js");
    let progs: Vec<Vec<u8>> = crate::examples()
        .into_iter()
        .map(|e| opendmd_psi::build(&e.src).unwrap())
        .chain((0..40).map(|n| (0..n).map(|k| (k * 37 + n) as u8).collect()))
        .collect();
    let input: String = format!(
        "[{}]",
        progs
            .iter()
            .map(|p| format!("[{}]", p.iter().map(|b| b.to_string()).collect::<Vec<_>>().join(",")))
            .collect::<Vec<_>>()
            .join(",")
    );
    let script = format!(
        "import {{ path, compiler }} from 'file://{kit}';\nlet s = ''; process.stdin.on('data', (d) => s += d); process.stdin.on('end', () => {{ for (const b of JSON.parse(s)) {{ compiler({{ program: () => new Uint8Array(b) }}); console.log(path({{ src: 'x', a: 0.5 }})); }} }});"
    );
    let mut child = std::process::Command::new("node")
        .args(["--input-type=module", "-e", &script])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("node (the web app's tests need it)");
    use std::io::Write;
    child.stdin.take().unwrap().write_all(input.as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success());
    let got: Vec<String> = String::from_utf8(out.stdout).unwrap().lines().map(String::from).collect();
    assert_eq!(got.len(), progs.len());
    for (b, g) in progs.iter().zip(&got) {
        let want = format!("@formula?psb={}&a=0.5", opendmd_psi::psb::to_base64(b));
        assert_eq!(g, &want);
        assert_eq!(opendmd_psi::psb::from_base64(&want.as_bytes()[13..want.len() - 6]).as_ref(), Some(b));
    }
}

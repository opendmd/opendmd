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
    let json = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../opendmd-web/www/formulas.json"
    ))
    .unwrap();
    let all = crate::examples(&json);
    assert!(all.len() >= 15, "{} examples", all.len());
    for e in &all {
        let p = opendmd_psi::compile(&e.src)
            .unwrap_or_else(|err| panic!("{}: {}", e.name, err.render(&e.src)));
        assert!(!e.name.is_empty() && !e.desc.is_empty(), "{e:?}");
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
        // Endless road is a showcase too, and most of its lines are data:
        // 13 `table` lines (the car, the mountains and the sun as spans,
        // the road's rows) and three draw loops.
        let most = if e.name == "Endless road" {
            52
        } else if e.name == "Time gate" {
            19
        } else if ["Big clock", "Tunnel", "Rotozoom", "Rotozoom floor"].contains(&e.name.as_str()) {
            16
        } else {
            7
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

/// The path the web app sends for an example (js/psishow.js `fpath`:
/// comments out, spaces collapsed, lines joined with `; `, lightly encoded)
/// with the `&name=` Add to playlist puts on it, and the sliders at their
/// most: within the panel's 2,048 B (`synth::MAX_FORMULA_PATH`).
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn every_example_fits_the_panels_path_as_sent() {
    let json = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../opendmd-web/www/formulas.json"
    ))
    .unwrap();
    for e in crate::examples(&json) {
        let mut s = String::new();
        for l in e.src.lines() {
            let l = match (l.find("--"), l.find("//")) {
                (Some(a), Some(b)) => &l[..a.min(b)],
                (Some(a), None) | (None, Some(a)) => &l[..a],
                (None, None) => l,
            };
            let c = l.split_whitespace().collect::<Vec<_>>().join(" ");
            if !c.is_empty() {
                if !s.is_empty() {
                    s.push_str("; ");
                }
                s.push_str(&c);
            }
        }
        let mut p = String::from("@fx:psi?src=");
        for b in s.bytes() {
            match b {
                b' ' => p.push('+'),
                b'%' | b'&' | b'+' | b'#' | b'?' => p.push_str(&format!("%{b:02X}")),
                0x21..=0x7e => p.push(b as char),
                _ => p.push_str(&format!("%{b:02X}")),
            }
        }
        // The compacted text still compiles to the same program.
        assert_eq!(
            opendmd_psi::compile(&s).map(|p| p.len()).ok(),
            opendmd_psi::compile(&e.src).map(|p| p.len()).ok(),
            "{}",
            e.name
        );
        p.push_str("&a=0.3333&b=0.3333&c=0.3333&name=");
        p.push_str(&e.name.replace(' ', "+"));
        assert!(p.len() <= 2048, "{}: {} B as sent", e.name, p.len());
    }
}

/// Every example pasted into another formula as a part (as the editor's
/// `use … as p` does: indented, under a comment) and drawn whole: the same
/// frames, over the golden picture too.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn every_example_pasted_in_as_a_part_draws_the_same() {
    use opendmd_psi::{golden, render_rows_ink, Scratch};
    let json = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../opendmd-web/www/formulas.json"
    ))
    .unwrap();
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
    for e in crate::examples(&json) {
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
    // one, and a look never has one. Time gate: ~6.1 ms and ~15.6.)
    const TOO_HEAVY: &[&str] = &["Tunnel", "Time gate"];
    let json = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../opendmd-web/www/formulas.json"
    ))
    .unwrap();
    let mut heavy = Vec::new();
    let mut chosen = Vec::new();
    for e in crate::examples(&json) {
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
    ("Endless road", 0, 0x150f6df9763d2806),
    ("Endless road", 1730, 0xfbe6b29e2e5c9bd1),
    ("Endless road", 6420, 0x5ff2083213399702),
    ("Time gate", 0, 0x24bd9942453ef2e8),
    ("Time gate", 1730, 0x7787a7d7cc3f723e),
    ("Time gate", 6420, 0x2a17edb7a1ade755),
    ("Plasma", 0, 0xfb5c1cb21173b111),
    ("Plasma", 1730, 0x9546a903ee9acb61),
    ("Plasma", 6420, 0xbf90cf3c83bac96b),
    ("Fire", 0, 0x960e59aeab7951c8),
    ("Fire", 1730, 0xbea6b26c5d43c15d),
    ("Fire", 6420, 0x847898880d31ee05),
    ("Rain", 0, 0xbda039267e727a4e),
    ("Rain", 1730, 0x8e61dd4784375452),
    ("Rain", 6420, 0x77570f4a476294a7),
    ("Matrix", 0, 0x7bdb4ff5caa79867),
    ("Matrix", 1730, 0x498bebeaf2536936),
    ("Matrix", 6420, 0x93001b452ba06964),
    ("Lava lamp", 0, 0x1c70828a94fa5c82),
    ("Lava lamp", 1730, 0x3912ecb2c5c51b43),
    ("Lava lamp", 6420, 0xe2da3e4b13b05630),
    ("Rings", 0, 0xf2cca724804083b5),
    ("Rings", 1730, 0x4e985fe095de8d29),
    ("Rings", 6420, 0xdcba9def1c26c04f),
    ("Moiré", 0, 0x4157bdfdb00c81c3),
    ("Moiré", 1730, 0xafa70d932caf84a0),
    ("Moiré", 6420, 0x8681c1171c7953c7),
    ("XOR pattern", 0, 0x3d89f2b7439e8625),
    ("XOR pattern", 1730, 0xbd877aa5f66e5925),
    ("XOR pattern", 6420, 0x49a1218e8c889d65),
    ("Stripes", 0, 0x0a53a451bf3de0ed),
    ("Stripes", 1730, 0x05a99347bbcaae41),
    ("Stripes", 6420, 0xd4ba0313857c3a7f),
    ("Clock", 0, 0x1c859c9432c78b42),
    ("Clock", 1730, 0x1c859c9432c78b42),
    ("Clock", 6420, 0x1c859c9432c78b42),
    ("CRT", 0, 0xa370b6d938e6a211),
    ("CRT", 1730, 0x624e33f3904fa52c),
    ("CRT", 6420, 0x5754c9112c907b97),
    ("Rainbow wave", 0, 0x6d6cd15d6293619f),
    ("Rainbow wave", 1730, 0x7e1aa7ac7a23dded),
    ("Rainbow wave", 6420, 0xd2645bea0a67abe2),
    ("Spotlight", 0, 0x5a646f5a21333771),
    ("Spotlight", 1730, 0x6ca3a96555f17be5),
    ("Spotlight", 6420, 0xeecb16dc3e03b206),
    ("Copper bars", 0, 0xd65a063a3b24a925),
    ("Copper bars", 1730, 0xf229f95cbec2ed25),
    ("Copper bars", 6420, 0xbd475933b7c45625),
    ("Bars to stars", 0, 0xe121beaf81716ded),
    ("Bars to stars", 1730, 0x4269f88db1e25dc1),
    ("Bars to stars", 6420, 0x9806d62838473829),
    ("Rings through stripes", 0, 0x0cce44831f18a402),
    ("Rings through stripes", 1730, 0xbd9030d90c96e5c8),
    ("Rings through stripes", 6420, 0x7bc9d15841cb6754),
    ("Kaleidoscope", 0, 0x8acaa53c934ec265),
    ("Kaleidoscope", 1730, 0xf7ac89f4ae0e1d0f),
    ("Kaleidoscope", 6420, 0xfac1d28e1c0ad8a7),
    ("Scanlines", 0, 0x2e1c992bbc8c9f5a),
    ("Scanlines", 1730, 0x2e1c992bbc8c9f5a),
    ("Scanlines", 6420, 0x2e1c992bbc8c9f5a),
    ("Glitter", 0, 0x0ef799246d6d4252),
    ("Glitter", 1730, 0x3cc2113373285791),
    ("Glitter", 6420, 0x4325e334249f1715),
    ("Old photo", 0, 0x8a96329730876e8a),
    ("Old photo", 1730, 0x8a96329730876e8a),
    ("Old photo", 6420, 0x8a96329730876e8a),
    ("Vignette", 0, 0x33bf77d214dc3c9d),
    ("Vignette", 1730, 0x33bf77d214dc3c9d),
    ("Vignette", 6420, 0x33bf77d214dc3c9d),
    ("Corner clock", 0, 0xf9ac5e7955b4a76c),
    ("Corner clock", 1730, 0xf9ac5e7955b4a76c),
    ("Corner clock", 6420, 0xf9ac5e7955b4a76c),
    ("Old TV", 0, 0x858b99abf175d63d),
    ("Old TV", 1730, 0x5b634bb1f03cce37),
    ("Old TV", 6420, 0xf530faae1d47c446),
    ("Plasma letters", 0, 0x25c29254edf5deb8),
    ("Plasma letters", 1730, 0xc116670365b1c4b9),
    ("Plasma letters", 6420, 0x129224eed97a6537),
    ("Searchlight", 0, 0x6688521f0639056f),
    ("Searchlight", 1730, 0x27134696e6b29b96),
    ("Searchlight", 6420, 0xf3755c40b9cf4ce2),
];

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn every_example_draws_as_before_at_its_defaults() {
    use opendmd_psi::{golden, render_rows_ink, Scratch};
    let json = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../opendmd-web/www/formulas.json"
    ))
    .unwrap();
    let mut under = vec![0u8; W * H * 3];
    golden::picture(&mut under);
    let all = crate::examples(&json);
    assert_eq!(all.len() * 3, EXAMPLE_FRAMES.len());
    for (e, want) in all.iter().zip(EXAMPLE_FRAMES.chunks(3)) {
        let p = opendmd_psi::compile(&e.src).unwrap();
        for &(name, ms, hash) in want {
            assert_eq!(name, e.name);
            let mut i = golden::inputs(ms);
            let given = |v: i32| (v != 0).then_some(v);
            [i.a, i.b, i.c] = p.meta().values([given(e.a), given(e.b), given(e.c)]);
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
            assert_eq!(golden::fnv(&out), hash, "{name} at {ms} ms");
        }
    }
}

/// The web app's path text (`js/psikit.js` `compact`, which `fpath` sends)
/// is the compiler's `compact()`, for every example and the edge cases:
/// text in quotes on declaration lines only, `'"'` a character, comments,
/// runs of spaces, `;`-joined lines. Run under node.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn the_web_apps_compact_is_the_compilers() {
    let json = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../opendmd-web/www/formulas.json"
    ))
    .unwrap();
    let mut srcs: Vec<String> = crate::examples(&json).into_iter().map(|e| e.src).collect();
    for s in [
        "name = '\"' -- a quote\ngray(name/100)",
        "name \"a  b\"\ngray(x)",
        "control k 0..1 = 0.5 \"a;b -- c\"\ngray(k)",
        "name \"x\"  -- \"q\"\ngray(x)",
        "name \"a\" ; control k = 0.5 ; gray(k)",
        "psi 1\nlook screen 40 -- c\nloop 4\ngray(x)",
        "psi 1; name \"a -- b; c\"; gray(x)",
        "-- name \"x\"\n  v = 1   // c \"a  b\" \ngray( v )",
        "k = ' ' + ';' + '-'-'/' -- c\ngray(glyph(k, px % 5, py))",
        "about \"Moiré:\tx\" \r\nname \"unclosed -- x\ngray(x)",
        "control  k   = 0  \"Ünï  cödé\"  // c\r\ngray(k)",
        "v = x % 2 + 1 & 3 # é  // c",
        "names \"x\" -- c\ngray(x)",
        "sprite s {\n  key \" 1, . 0\n  .\"\".\n}\ngray(s[px, py])",
    ] {
        srcs.push(s.to_string());
    }
    // Over the panel's 2,048 B with its `about` line, under it without:
    // the path leaves the line out (`;` and `--` in another's quotes).
    let body = format!("gray({}1)", "x*".repeat(970));
    let about = format!("about \"{}\"", "A".repeat(110));
    srcs.push(format!("name \"Long; -- one\"\n{about}\ncontrol k = 0.5 \"x; y\"\n{body}"));
    srcs.push(format!("{about}\n{body}"));
    // Over it either way: still without.
    srcs.push(format!("{about}\n{body}{}", "+1".repeat(100)));
    let input: String = {
        let mut j = String::from("[");
        for (k, s) in srcs.iter().enumerate() {
            if k > 0 {
                j.push(',');
            }
            j.push('"');
            for c in s.chars() {
                match c {
                    '"' => j.push_str("\\\""),
                    '\\' => j.push_str("\\\\"),
                    '\n' => j.push_str("\\n"),
                    '\r' => j.push_str("\\r"),
                    '\t' => j.push_str("\\t"),
                    c => j.push(c),
                }
            }
            j.push('"');
        }
        j.push(']');
        j
    };
    let kit = concat!(env!("CARGO_MANIFEST_DIR"), "/../opendmd-web/www/js/psikit.js");
    // Each formula's compact text, then its path (psikit `path`, what
    // psishow's fpath sends).
    let script = format!(
        "import {{ compact, path }} from 'file://{kit}';\nlet s = ''; process.stdin.on('data', (d) => s += d); process.stdin.on('end', () => {{ for (const x of JSON.parse(s)) console.log(JSON.stringify(compact(x)) + '\\n' + path({{ src: x }})); }});"
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
    assert_eq!(got.len(), srcs.len() * 2);
    // fx_psi's path: the compact text, lightly encoded; over 2,048 B
    // without its `about` line.
    let enc = |c: &str| {
        let mut p = String::from("@fx:psi?src=");
        for b in c.bytes() {
            match b {
                b' ' => p.push('+'),
                b'%' | b'&' | b'+' | b'#' | b'?' => p.push_str(&format!("%{b:02X}")),
                0x21..=0x7e => p.push(b as char),
                _ => p.push_str(&format!("%{b:02X}")),
            }
        }
        p
    };
    let mut dropped = 0;
    for (src, g) in srcs.iter().zip(got.chunks(2)) {
        let full = enc(&opendmd_psi::compact(src));
        let want = if full.len() > 2048 {
            dropped += 1;
            enc(&opendmd_psi::compact(&opendmd_psi::look::declare(src, "about", 0, "")))
        } else {
            full
        };
        assert_eq!(g[1], want, "path of {src:?}");
        assert!(want.len() <= 2048 || !src.starts_with("name"), "{}", want.len());
    }
    assert_eq!(dropped, 3);
    for (src, g) in srcs.iter().zip(got.into_iter().step_by(2)) {
        let want = opendmd_psi::compact(src);
        let mut w = String::from("\"");
        for c in want.chars() {
            match c {
                '"' => w.push_str("\\\""),
                '\\' => w.push_str("\\\\"),
                '\t' => w.push_str("\\t"),
                c => w.push(c),
            }
        }
        w.push('"');
        assert_eq!(g, w, "compact of {src:?}");
    }
}

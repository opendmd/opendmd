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

    // The golden frames.
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

    // Any formula as a look: the text, and what it is.
    let s = look("hsv(x, 1, 1)", "screen", 40);
    assert_eq!(
        s,
        "part look {\n  hsv(x, 1, 1)\n}\nlayer screen { look(x, y, 0.4) }"
    );
    assert_eq!((info()[5] & 32, info()[7]), (32, 1));
    assert_eq!(look("gray(ul)", "over", 100), "gray(ul)");
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
                !inside && !l.trim_start().starts_with("--")
            })
            .count();
        // The showcase, Big clock, reads as a commented step a line
        // (owner, 2026-10-26: legible over short); the rest stay short.
        // So do the built-in effects' Formulas (Tunnel, Rotozoom): a native
        // effect's knobs, written out.
        // Endless road is a showcase too, and most of its lines are data:
        // 13 `table` lines (the car, the mountains and the sun as spans,
        // the road's rows) and three draw loops.
        let most = if e.name == "Endless road" {
            52
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
        // A slider it reads has a starting value or a name.
        for (k, v) in [('a', e.a), ('b', e.b), ('c', e.c)] {
            let used = p.sliders() & (1 << (k as u8 - b'a')) != 0;
            assert!(
                !used || v != 0 || e.src.contains(&format!("-- {k}:")),
                "{}: slider {k}",
                e.name
            );
        }
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
            let src = format!(
                "loop {}\npart p {{\n  -- {}\n{inner}}}\nlayer {{ p }}",
                lp as f64 / 1000.0,
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
    // (Tunnel: ~7.5 ms as an effect with its still cache, ~17.6 without
    // one, and a look never has one.)
    const TOO_HEAVY: &[&str] = &["Tunnel"];
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
            let s = look(&e.src, b, k);
            // A formula with layers of its own goes over only.
            if s.is_empty() && b == "screen" && error().contains("can only go over") {
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
                assert_eq!(s, e.src, "{}: today's look, unchanged", e.name);
            } else {
                assert!(
                    s.starts_with("part look {") || s.starts_with("loop "),
                    "{}",
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
    assert!(look("layer multiply { gray(0.5) }", "screen", 40).is_empty());
    assert!(error().contains("can only go over"), "{}", error());
    assert!(look("gray(x)", "lighten", 40).is_empty());
    assert!(
        error().starts_with("a look's blend is one of over"),
        "{}",
        error()
    );
}

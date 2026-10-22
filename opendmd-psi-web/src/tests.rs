//! Natively (`cargo test`) and in wasm under node (`wasm-pack test --node`):
//! the module draws the golden frames — the hashes the host, wasm32 and the
//! ESP32 agree on — and reports errors where JS will look for them. One
//! test: the formula is a static.

use super::*;
use opendmd_psi::golden::{fnv, inputs, FRAMES};

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
}

/// The examples gallery: every one compiles, is short, and is light or
/// medium for the panel (the ESP32 estimate within the effects' 8 ms).
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
        assert!(
            e.src
                .lines()
                .filter(|l| !l.trim_start().starts_with("--"))
                .count()
                <= 7,
            "{}: too long for an example",
            e.name
        );
        assert!(
            p.esp32_us() <= 8000,
            "{}: ~{} µs a frame on the panel",
            e.name,
            p.esp32_us()
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

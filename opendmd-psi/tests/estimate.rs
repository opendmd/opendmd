//! `Program::esp32_us` against what the device measured (`psi-probe` on
//! the spare, VM frame p50 over a picture: tests/measured.rs, × 1.15 for
//! what the effect and look paths add, as the estimate counts it).
//! Formulas that draw a whole panel through a layer — every look — are held
//! to ±15 %; the rest is printed.

mod measured;
use opendmd_psi::compile;
use opendmd_psi::look::compose;

#[test]
fn the_estimate_is_within_15_percent_for_layers_and_looks() {
    let mut bad = Vec::new();
    let mut rows = Vec::new();
    for &(name, src, us) in measured::STEPS {
        let p = compile(src).unwrap();
        let est = p.esp32_us();
        let r = est as f64 / (us as f64 * 1.15);
        rows.push(format!(
            "{name:<26} measured {us:>5} est {est:>5} ({:+.0} %)",
            (r - 1.0) * 100.0
        ));
        if p.layers() > 0 && !(0.85..=1.15).contains(&r) {
            bad.push(name.to_string());
        }
    }
    for &(name, src, blend, k, us) in measured::LOOKS {
        let p = compile(&compose(src, blend, k).unwrap()).unwrap();
        let est = p.esp32_us();
        let r = est as f64 / (us as f64 * 1.15);
        rows.push(format!(
            "look {name} {blend} {k:<3} measured {us:>5} est {est:>5} ({:+.0} %)",
            (r - 1.0) * 100.0
        ));
        if !(0.85..=1.15).contains(&r) {
            bad.push(format!("{name} {blend} {k}"));
        }
    }
    println!("{}", rows.join("\n"));
    assert!(
        bad.is_empty(),
        "outside ±15 %: {bad:?}\n{}",
        rows.join("\n")
    );
}

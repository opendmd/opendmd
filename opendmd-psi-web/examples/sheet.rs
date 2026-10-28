//! The examples' contact sheet, for tuning them: every formula in
//! `opendmd-web/formulas/` (or DIR) at four times in its loop (controls at
//! their defaults, 10:09:30), each 128x32 frame scaled 3x, one PPM per
//! example, plus its steps and the ESP32 estimate on stdout.
//!
//!     cargo run --release --example sheet -- ../opendmd-web/formulas OUT_DIR

// (Host only: `wasm-pack test` builds the examples for wasm32 too.)
#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    use opendmd_psi::{compile, render_rows_ink, Ink, Inputs, Scratch, H, OWN, W};
    use opendmd_psi_web::examples_in;
    use std::io::Write;

    let mut args = std::env::args().skip(1);
    let dir = std::path::PathBuf::from(args.next().expect("the examples' folder"));
    let out = std::path::PathBuf::from(args.next().expect("output directory"));
    std::fs::create_dir_all(&out).unwrap();
    const S: usize = 3;
    for (k, ex) in examples_in(&dir).iter().enumerate() {
        let p = compile(&ex.src).unwrap_or_else(|e| panic!("{}: {}", ex.name, e.render(&ex.src)));
        let (f, r, x) = p.counts();
        println!(
            "{:<16} {:>3} steps ({f}/{r}/{x})  ~{:>5.1} ms  loop {} s  sliders {:03b}",
            ex.name,
            p.len(),
            p.esp32_us() as f64 / 1000.0,
            p.loop_ms() / 1000,
            p.sliders()
        );
        let mut s = Scratch::<128>::new();
        let mut frame = vec![0u8; W * H * 3];
        let (cols, rows) = (2, 2);
        let (iw, ih) = (W * S * cols + 8, (H * S + 8) * rows);
        let mut img = vec![40u8; iw * ih * 3];
        for q in 0..4 {
            // (One that never repeats: over its first minute.)
            let lp = if p.forever() { 60_000 } else { p.loop_ms() };
            let ms = lp * q as u32 / 4;
            let mut i = Inputs {
                hour: 10,
                minute: 9,
                second: 30,
                ..Inputs::default()
            };
            [i.a, i.b, i.c] = p.meta().values([None; 3]);
            i.set_ms(&p, u64::from(ms));
            render_rows_ink(
                &p,
                &mut s,
                Some(&mut Ink::boxed()),
                &i,
                &OWN,
                None,
                u32::MAX,
                &mut frame,
            );
            let (ox, oy) = ((q % cols) * (W * S + 8), (q / cols) * (H * S + 8));
            for y in 0..H * S {
                for x in 0..W * S {
                    let src = ((y / S) * W + x / S) * 3;
                    let dst = ((oy + y) * iw + ox + x) * 3;
                    img[dst..dst + 3].copy_from_slice(&frame[src..src + 3]);
                }
            }
        }
        let mut f = std::fs::File::create(out.join(format!("{k:02}.ppm"))).unwrap();
        writeln!(f, "P6 {iw} {ih} 255").unwrap();
        f.write_all(&img).unwrap();
    }
}

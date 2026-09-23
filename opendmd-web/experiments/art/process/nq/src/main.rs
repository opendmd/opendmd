use std::io::{Read, Write};
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let colors: usize = a.get(1).map_or(16, |s| s.parse().unwrap());
    let fac: i32 = a.get(2).map_or(10, |s| s.parse().unwrap());
    let mut rgba = Vec::new();
    std::io::stdin().read_to_end(&mut rgba).unwrap();
    let nq = color_quant::NeuQuant::new(fac, colors, &rgba);
    let mut out = nq.color_map_rgb();
    for px in rgba.chunks_exact(4) {
        out.push(nq.index_of(px) as u8);
    }
    std::io::stdout().write_all(&out).unwrap();
}

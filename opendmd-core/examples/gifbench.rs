//! Decode CPU per decoder, for the gifslim experiment.
//!
//! Usage: `gifbench <base>...` where `<base>` is a GIF path without `.gif`;
//! beside it the tool looks for `<base>.odm` and the recoded copies
//! `<base>.cap<N>[d].gif` (`opendmd-pack recode`). Every file is decoded
//! from memory, all frames, repeated to at least ~40 ms of work; five such
//! runs, median reported. Time is this thread's CPU time
//! (`CLOCK_THREAD_CPUTIME_ID`), so a loaded machine inflates it less than
//! wall time would.
use opendmd_core::canvas::Canvas;
use opendmd_core::gifslim::GifSlim;
use opendmd_core::odm::OdmSource;
use opendmd_core::player::GifSource;
use std::io::{self, Cursor};

#[repr(C)]
struct Timespec {
    tv_sec: i64,
    tv_nsec: i64,
}
extern "C" {
    fn clock_gettime(clk: i32, tp: *mut Timespec) -> i32;
}
fn cpu_ns() -> u64 {
    let mut t = Timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // CLOCK_THREAD_CPUTIME_ID = 3 on Linux.
    unsafe { clock_gettime(3, &mut t) };
    t.tv_sec as u64 * 1_000_000_000 + t.tv_nsec as u64
}

/// Decode every frame once; returns (frames, pixels per frame).
fn once(kind: &str, bytes: &[u8], canvas: &mut Canvas) -> io::Result<(u64, usize)> {
    macro_rules! drain {
        ($s:expr) => {{
            let mut s = $s;
            let (w, h) = s.logical_size();
            let mut n = 0u64;
            while let Some(d) = s.next_frame(canvas)? {
                std::hint::black_box(d);
                canvas.dirty_mut().clear();
                n += 1;
            }
            Ok((n, w * h))
        }};
    }
    match kind {
        "gif" => drain!(GifSource::new(Cursor::new(bytes))?),
        "odm" => drain!(OdmSource::new(Cursor::new(bytes))?),
        k => {
            let cap: u8 = k.trim_start_matches("slim").parse().unwrap();
            drain!(GifSlim::with_cap(Cursor::new(bytes), cap)?)
        }
    }
}

fn bench(kind: &str, bytes: &[u8]) -> io::Result<(f64, f64, u64)> {
    let mut canvas = Canvas::new();
    let (frames, px) = once(kind, bytes, &mut canvas)?;
    // Repeat to ~40 ms per run.
    let t0 = cpu_ns();
    once(kind, bytes, &mut canvas)?;
    let one = (cpu_ns() - t0).max(1);
    let reps = (40_000_000 / one).clamp(1, 1000);
    let mut runs = Vec::new();
    for _ in 0..5 {
        let t0 = cpu_ns();
        for _ in 0..reps {
            once(kind, bytes, &mut canvas)?;
        }
        runs.push((cpu_ns() - t0) as f64 / reps as f64);
    }
    runs.sort_by(f64::total_cmp);
    let per_file = runs[2];
    let ns_px = per_file / (frames as f64 * px as f64);
    let us_frame = per_file / frames as f64 / 1000.0;
    Ok((ns_px, us_frame, frames))
}

fn main() {
    let bases: Vec<String> = std::env::args().skip(1).collect();
    let mut cols: Vec<(String, String)> = vec![
        ("gif".into(), ".gif".into()),
        ("odm".into(), ".odm".into()),
        ("slim12".into(), ".gif".into()),
    ];
    for cap in [9, 10, 11, 12] {
        cols.push((format!("slim{cap}"), format!(".cap{cap}d.gif")));
    }
    for cap in [9, 10, 11, 12] {
        cols.push((format!("slim{cap}"), format!(".cap{cap}.gif")));
    }
    cols.push(("gif".into(), ".cap10d.gif".into()));
    cols.push(("gif".into(), ".cap12.gif".into()));
    println!("decoder\tfile\tname\tframes\tns_per_px\tus_per_frame");
    let mut sums: Vec<(f64, f64, u32)> = vec![(0.0, 0.0, 0); cols.len()];
    for base in &bases {
        let name = base.rsplit('/').next().unwrap();
        for (i, (kind, ext)) in cols.iter().enumerate() {
            let path = format!("{base}{ext}");
            let Ok(bytes) = std::fs::read(&path) else {
                continue;
            };
            match bench(kind, &bytes) {
                Ok((ns, us, frames)) => {
                    println!("{kind}\t{ext}\t{name}\t{frames}\t{ns:.2}\t{us:.1}");
                    sums[i].0 += ns;
                    sums[i].1 += us;
                    sums[i].2 += 1;
                }
                Err(e) => println!("{kind}\t{ext}\t{name}\t-\terror: {e}"),
            }
        }
    }
    println!(
        "\nmean over files: decoder, file, ns/px, us/frame at 128x32, device ms/frame at 50x..100x"
    );
    for (i, (kind, ext)) in cols.iter().enumerate() {
        let (ns, us, n) = sums[i];
        if n == 0 {
            continue;
        }
        let (ns, us) = (ns / n as f64, us / n as f64);
        let (lo, hi) = (us * 50.0 / 1000.0, us * 100.0 / 1000.0);
        println!(
            "{kind:<7} {ext:<12} {ns:>6.2} ns/px {us:>7.1} us/frame  device {lo:.2}-{hi:.2} ms{}  ({n} files)",
            if hi > 5.0 { "  > 5 ms!" } else { "" }
        );
    }
}

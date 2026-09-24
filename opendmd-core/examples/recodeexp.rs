//! Recoder variants over a sample of files, in memory: total bytes per
//! variant, and encode CPU time. `recodeexp [--every K] <dir>`.
use opendmd_core::gifslim::{recode_gif, RecodeOptions};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering::Relaxed};
use std::time::Instant;

fn walk(p: &Path, out: &mut Vec<PathBuf>) {
    if p.is_dir() {
        let mut v: Vec<_> = std::fs::read_dir(p)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .collect();
        v.sort();
        for e in v {
            walk(&e, out);
        }
    } else {
        let n = p
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_ascii_lowercase();
        if n.ends_with(".gif") && !n.contains(".cap") {
            out.push(p.to_path_buf());
        }
    }
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut every = 1;
    if args.first().map(|a| a == "--every").unwrap_or(false) {
        every = args[1].parse().unwrap();
        args.drain(..2);
    }
    let mut files = Vec::new();
    walk(Path::new(&args[0]), &mut files);
    let files: Vec<Vec<u8>> = files
        .iter()
        .step_by(every)
        .map(|p| std::fs::read(p).unwrap())
        .collect();
    let gif_total: u64 = files.iter().map(|f| f.len() as u64).sum();
    println!("{} files, {:.1} MB", files.len(), gif_total as f64 / 1e6);
    let variants: Vec<(String, RecodeOptions)> = [9u8, 10, 11, 12]
        .iter()
        .flat_map(|&cap| {
            [(false, false), (true, false), (true, true)].map(|(delta, keep_runs)| {
                (
                    format!("cap{cap} delta={delta} keep_runs={keep_runs}"),
                    RecodeOptions {
                        cap,
                        delta,
                        hint: true,
                        keep_runs,
                    },
                )
            })
        })
        .collect();
    let only: Option<String> = std::env::var("ONLY").ok();
    for (name, opts) in variants {
        if only.as_ref().is_some_and(|o| !name.starts_with(o.as_str())) {
            continue;
        }
        let next = AtomicUsize::new(0);
        let bytes = AtomicU64::new(0);
        let t0 = Instant::now();
        std::thread::scope(|s| {
            for _ in 0..16 {
                s.spawn(|| loop {
                    let i = next.fetch_add(1, Relaxed);
                    let Some(f) = files.get(i) else { break };
                    let mut out = Vec::new();
                    if let Ok(st) = recode_gif(&f[..], &mut out, &opts) {
                        bytes.fetch_add(st.out_bytes, Relaxed);
                    }
                });
            }
        });
        let b = bytes.load(Relaxed);
        println!(
            "{name:<34} {:>8.1} MB  ratio {:.4}  {:.1}s",
            b as f64 / 1e6,
            b as f64 / gif_total as f64,
            t0.elapsed().as_secs_f64()
        );
    }
}

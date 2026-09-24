//! The device's streaming recoder against the desktop/browser one, over real
//! files: byte-identity, the files it refuses, and its heap.
//!
//! Usage: `encodeprobe [--heap] [--out <card-root>] <gif-or-dir>...`
//! (directories are walked for `*.gif`). Every file goes through
//! `gifslim::recode_gif` (in memory, what `opendmd-pack` and the browser
//! run) and `gifslim::recode_gif_streaming` over a bare `File` (what the
//! cache builder runs), both in the prepared format; the output must be
//! identical unless the in-memory one had to quantise, in which case the
//! streaming one must fail with `TooManyColours`. Runs on every core.
//!
//! `--out <card-root>` also writes each streaming copy where the card keeps
//! it (`<card-root>/cache/<Category>/<xx>/<name>.gif`, the GIF's path taken
//! from its `gifs/` component on), so the tree can be compared with
//! `opendmd-pack convert`'s file for file.
//!
//! `--heap` runs one file at a time instead and reports the streaming
//! recoder's peak live heap (a counting global allocator; the reader is a
//! bare `File` and the writer a 2 KB `BufWriter`, as on the device), so its
//! figure is the whole encoder: decoder, canvas, index buffers, colour
//! table, LZW dictionary and write buffer. Files gifslim cannot decode are
//! measured with the `gif` crate as the fallback, and listed apart.
use opendmd_core::gifslim::{self, PREPARED};
use std::alloc::{GlobalAlloc, Layout, System};
use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
use std::sync::Mutex;

struct Counting;
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let n = LIVE.fetch_add(l.size(), SeqCst) + l.size();
        PEAK.fetch_max(n, SeqCst);
        System.alloc(l)
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        LIVE.fetch_sub(l.size(), SeqCst);
        System.dealloc(p, l)
    }
}
#[global_allocator]
static A: Counting = Counting;

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
    } else if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("gif")) {
        out.push(p.to_path_buf());
    }
}

/// The GIF's path relative to its `gifs/` directory, `/`-separated.
fn gif_rel(p: &Path) -> Option<String> {
    let s = p.to_string_lossy().replace(std::path::MAIN_SEPARATOR, "/");
    let i = s.rfind("/gifs/")?;
    Some(s[i + "/gifs/".len()..].to_string())
}

#[derive(Default)]
struct Tally {
    files: usize,
    identical: usize,
    full_decoder: Vec<String>,
    too_many: Vec<String>,
    both_fail: Vec<String>,
    mismatch: Vec<String>,
    written: usize,
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let heap = args.first().is_some_and(|a| a == "--heap");
    if heap {
        args.remove(0);
    }
    let out_root = if args.first().is_some_and(|a| a == "--out") {
        args.remove(0);
        Some(PathBuf::from(args.remove(0)))
    } else {
        None
    };
    let mut files = Vec::new();
    for a in &args {
        walk(Path::new(a), &mut files);
    }
    if heap {
        heap_probe(&files);
        return;
    }

    let next = AtomicUsize::new(0);
    let tally = Mutex::new(Tally::default());
    let jobs = std::thread::available_parallelism().map_or(1, |n| n.get());
    let t0 = std::time::Instant::now();
    std::thread::scope(|s| {
        for _ in 0..jobs {
            s.spawn(|| loop {
                let i = next.fetch_add(1, SeqCst);
                let Some(path) = files.get(i) else { break };
                let name = path.display().to_string();
                let bytes = std::fs::read(path).unwrap();
                let mut want = Vec::new();
                let desk = gifslim::recode_gif(&bytes[..], &mut want, &PREPARED);
                let mut got = Vec::new();
                let streamed = File::open(path).and_then(|f| {
                    gifslim::recode_gif_streaming(f, &mut got, &PREPARED, || true, || Ok(()))
                });
                let mut t = tally.lock().unwrap();
                t.files += 1;
                match (&desk, &streamed) {
                    (Ok(_), Ok(gs)) if want == got => {
                        t.identical += 1;
                        if gs.full_decoder {
                            t.full_decoder.push(name.clone());
                        }
                        if let (Some(root), Some(rel)) = (&out_root, gif_rel(path)) {
                            let dst = root.join(opendmd_core::layout::cache_rel(&rel).unwrap());
                            std::fs::create_dir_all(dst.parent().unwrap()).unwrap();
                            std::fs::write(&dst, &got).unwrap();
                            t.written += 1;
                        }
                    }
                    (Ok(ds), Err(e)) if ds.quantised && gifslim::is_too_many_colours(e) => {
                        t.too_many.push(name)
                    }
                    (Err(_), Err(_)) => t.both_fail.push(name),
                    _ => t.mismatch.push(format!(
                        "{name}: desk {:?} stream {:?} ({} vs {} B)",
                        desk.as_ref()
                            .map(|s| s.out_bytes)
                            .map_err(|e| e.to_string()),
                        streamed
                            .as_ref()
                            .map(|s| s.out_bytes)
                            .map_err(|e| e.to_string()),
                        want.len(),
                        got.len()
                    )),
                }
                if t.files.is_multiple_of(1000) {
                    eprintln!("  {} files, {:.0}s", t.files, t0.elapsed().as_secs_f64());
                }
            });
        }
    });
    let t = tally.into_inner().unwrap();
    println!(
        "{} files in {:.1}s: {} byte-identical ({} of them through the gif crate), {} too many colours \
         (quantised in memory, refused streaming), {} undecodable by both, {} MISMATCHES{}",
        t.files,
        t0.elapsed().as_secs_f64(),
        t.identical,
        t.full_decoder.len(),
        t.too_many.len(),
        t.both_fail.len(),
        t.mismatch.len(),
        if out_root.is_some() {
            format!("; {} streaming copies written", t.written)
        } else {
            String::new()
        }
    );
    for f in &t.full_decoder {
        println!("  gif crate: {f}");
    }
    for m in t.mismatch.iter().take(30) {
        println!("  MISMATCH {m}");
    }
    if !t.mismatch.is_empty() {
        std::process::exit(1);
    }
}

fn heap_probe(files: &[PathBuf]) {
    let mut peaks: Vec<(usize, String)> = Vec::new();
    let mut full: Vec<(usize, String)> = Vec::new();
    let (mut refused, mut failed) = (0, 0);
    for path in files {
        let Ok(f) = File::open(path) else { continue };
        let base = LIVE.load(SeqCst);
        PEAK.store(base, SeqCst);
        let out = BufWriter::with_capacity(2048, std::io::sink());
        let r = gifslim::recode_gif_streaming(f, out, &PREPARED, || true, || Ok(()));
        let peak = PEAK.load(SeqCst) - base;
        let name = path.display().to_string();
        match r {
            Ok(s) if s.full_decoder => full.push((peak, name)),
            Ok(_) => peaks.push((peak, name)),
            Err(e) if gifslim::is_too_many_colours(&e) => refused += 1,
            Err(_) => failed += 1,
        }
    }
    peaks.sort();
    let n = peaks.len().max(1);
    let mean = peaks.iter().map(|p| p.0).sum::<usize>() / n;
    let pct = |q: f64| {
        peaks
            .get(((peaks.len() as f64 - 1.0) * q) as usize)
            .map_or(0, |p| p.0)
    };
    println!(
        "streaming recoder peak heap over {} files (gifslim): mean {mean} B, min {} B, median {} B, \
         p99 {} B, max {} B ({})",
        peaks.len(),
        pct(0.0),
        pct(0.5),
        pct(0.99),
        pct(1.0),
        peaks.last().map_or("", |p| p.1.as_str())
    );
    for (p, name) in &full {
        println!("  with the gif crate: {p} B  {name}");
    }
    println!("  refused (more than 256 colours): {refused}; undecodable: {failed}");
}

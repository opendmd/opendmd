//! The device's streaming ODM encoder against the desktop one, over real
//! files: byte-identity, the files it refuses, and its heap.
//!
//! Usage: `encodeprobe [--heap] <gif-or-dir>...` (directories are walked for
//! `*.gif`). Every file goes through `odm::encode_gif` and
//! `odm::encode_gif_streaming`; the output must be identical unless the
//! desktop encoder had to quantise, in which case the streaming one must
//! fail with `TooManyColours`. Runs on every core.
//!
//! `--heap` runs one file at a time instead and reports the streaming
//! encoder's peak live heap (a counting global allocator; the reader is a
//! bare `File`, as on the device), so its figure is the whole encoder:
//! decoder, canvas, index buffers, colour table and write buffer.
use opendmd_core::odm;
use std::alloc::{GlobalAlloc, Layout, System};
use std::fs::File;
use std::io::Cursor;
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

#[derive(Default)]
struct Tally {
    files: usize,
    identical: usize,
    too_many: Vec<String>,
    both_fail: Vec<String>,
    mismatch: Vec<String>,
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let heap = args.first().is_some_and(|a| a == "--heap");
    if heap {
        args.remove(0);
    }
    let mut files = Vec::new();
    for a in &args {
        walk(Path::new(a), &mut files);
    }

    if heap {
        let (mut max, mut sum, mut n, mut worst) = (0usize, 0usize, 0usize, String::new());
        for f in &files {
            let base = LIVE.load(SeqCst);
            PEAK.store(base, SeqCst);
            let file = File::open(f).unwrap();
            let mut out = Cursor::new(Vec::new());
            // The output lives in a Vec here, not on a card: its growth is
            // not the encoder's. Measure with it pre-sized and subtract it.
            let cap = std::fs::metadata(f)
                .map(|m| m.len() as usize * 4 + 4096)
                .unwrap();
            out.get_mut().reserve(cap);
            let pre = LIVE.load(SeqCst);
            PEAK.store(pre, SeqCst);
            let r = odm::encode_gif_streaming(file, &mut out, || Ok(()));
            let peak = PEAK.load(SeqCst) - pre;
            let grew = out.get_ref().capacity() > cap;
            drop(out);
            let _ = base;
            if r.is_ok() && !grew {
                n += 1;
                sum += peak;
                if peak > max {
                    max = peak;
                    worst = f.display().to_string();
                }
            }
        }
        println!(
            "streaming encoder peak heap over {n} files: mean {} B, max {max} B ({worst})",
            sum / n.max(1)
        );
        return;
    }

    let tally = Mutex::new(Tally::default());
    let next = AtomicUsize::new(0);
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| loop {
                let i = next.fetch_add(1, SeqCst);
                let Some(f) = files.get(i) else { break };
                let bytes = std::fs::read(f).unwrap();
                let name = f.display().to_string();
                let mut want = Vec::new();
                let desk = odm::encode_gif(&bytes[..], &mut want);
                let mut out = Cursor::new(Vec::new());
                let got = odm::encode_gif_streaming(&bytes[..], &mut out, || Ok(()));
                let mut t = tally.lock().unwrap();
                t.files += 1;
                match (desk, got) {
                    (Ok(ds), Ok(gs)) => {
                        let mut v = out.into_inner();
                        v.truncate(gs.odm_bytes as usize);
                        if !ds.quantised && v == want {
                            t.identical += 1;
                        } else {
                            t.mismatch.push(name);
                        }
                    }
                    (Ok(ds), Err(e)) if ds.quantised && odm::is_too_many_colours(&e) => {
                        t.too_many.push(name)
                    }
                    (Err(d), Err(g)) => t
                        .both_fail
                        .push(format!("{name}: desktop '{d}', streaming '{g}'")),
                    (d, g) => t.mismatch.push(format!(
                        "{name}: desktop {:?} / streaming {:?}",
                        d.map(|s| s.quantised),
                        g.map(|s| s.frames)
                    )),
                }
            });
        }
    });
    let t = tally.into_inner().unwrap();
    println!(
        "{} files: {} byte-identical, {} too many colours, {} fail in both, {} MISMATCH",
        t.files,
        t.identical,
        t.too_many.len(),
        t.both_fail.len(),
        t.mismatch.len()
    );
    for m in &t.both_fail {
        println!("  both fail: {m}");
    }
    for m in &t.too_many {
        println!("  too many colours: {m}");
    }
    for m in t.mismatch.iter().take(20) {
        println!("  MISMATCH: {m}");
    }
    if !t.mismatch.is_empty() {
        std::process::exit(1);
    }
}

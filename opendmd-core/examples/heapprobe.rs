//! Live and peak heap while decoding GIFs through `GifSource`, via a counting
//! global allocator.
//!
//! Usage: `heapprobe [-v] <gif>...`
//!
//! `steady` is the live heap right after the first frame — every buffer that
//! stays alive for the rest of the playback. `peak` is the high-water mark
//! over the whole file; the two differ only when the `gif` crate throws its
//! LZW decoder away and builds a new one, which it does whenever a frame's
//! LZW minimum code size differs from the previous frame's.
//!
//! `-v` also prints where the steady bytes went, by taking a reading at each
//! construction step.
use opendmd_core::canvas::{Canvas, HEIGHT, WIDTH};
use opendmd_core::player::GifSource;
use std::alloc::{GlobalAlloc, Layout, System};
use std::fs::File;
use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};

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

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let verbose = args.first().map(|a| a == "-v").unwrap_or(false);
    if verbose {
        args.remove(0);
    }

    for path in args {
        let base = LIVE.load(SeqCst);
        PEAK.store(base, SeqCst);

        let mut canvas = Canvas::new();
        let at_canvas = LIVE.load(SeqCst) - base;

        // Handed over unbuffered, like `SdSource` on the firmware: the `gif`
        // crate wraps it in an 8 KiB `BufReader` of its own anyway.
        let file = match File::open(&path) {
            Ok(f) => f,
            Err(e) => {
                println!("{path}: {e}");
                continue;
            }
        };
        let reader: Box<dyn std::io::Read> = Box::new(file);
        let at_reader = LIVE.load(SeqCst) - base;

        let mut src = match GifSource::new(reader) {
            Ok(s) => s,
            Err(e) => {
                println!("{path}: {e}");
                continue;
            }
        };
        let at_source = LIVE.load(SeqCst) - base;

        let mut frames = 0;
        let mut after_first = 0;
        let mut error = String::new();
        loop {
            match src.next_frame(&mut canvas) {
                Ok(Some(_)) => {
                    frames += 1;
                    if frames == 1 {
                        after_first = LIVE.load(SeqCst) - base;
                    }
                }
                Ok(None) => break,
                Err(e) => {
                    error = format!("  ERROR {e}");
                    break;
                }
            }
        }
        let peak = PEAK.load(SeqCst) - base;
        let (w, h) = src.logical_size();
        drop(src);

        println!(
            "{:<45} {:>3}x{:<3} frames {:>5}  steady {:>6} B  peak {:>6} B{}",
            path.rsplit('/').next().unwrap(),
            w,
            h,
            frames,
            after_first,
            peak,
            error
        );
        if verbose {
            // The RGB screen is the caller's `Canvas` when the logical
            // screen is exactly the panel; only otherwise is it a buffer of
            // GifSource's own.
            let own_screen = if w == WIDTH && h == HEIGHT {
                0
            } else {
                w * h * 3
            };
            println!(
                "    canvas {} B | + source reader {} B | + GifSource {} B \
                 (screen {} + indexed {} + gif internals {}) | \
                 + first frame {} B (LZW) | transient {} B",
                at_canvas,
                at_reader - at_canvas,
                at_source - at_reader,
                own_screen,
                w * h,
                (at_source - at_reader).saturating_sub(own_screen + w * h),
                after_first - at_source,
                peak - after_first,
            );
        }
    }
}

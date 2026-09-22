//! Desktop simulator for OpenDMD.
//!
//! Glue only: the playlist, filtering, scheduling and GIF decoding all live in
//! `opendmd-core`. This binary owns the window, the keyboard and the clock —
//! exactly the responsibilities the firmware's main loop will own.

use anyhow::{bail, Context, Result};
use clap::Parser;
use minifb::{Key, KeyRepeat, Window, WindowOptions};
use opendmd_core::canvas::{Canvas, Rgb, HEIGHT, WIDTH};
use opendmd_core::engine::{Catalog, DwellPolicy, Engine, FileCatalog, Source, TickOutcome};
use opendmd_core::filter::Filter;
use opendmd_core::manifest::Entry;
use opendmd_core::scheduler::{Mode, Scheduler};
use std::collections::BTreeMap;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Colour of the gap between simulated LEDs.
const GUTTER: u32 = 0x0008_0808;
/// How often the main loop wakes; the engine decides what actually changes.
const POLL: Duration = Duration::from_millis(5);

#[derive(Parser, Debug)]
#[command(
    name = "opendmd-sim",
    about = "Play a DMDoS card's GIFs in a 128x32 desktop window"
)]
struct Args {
    /// Card root: the directory holding gifs/ and filelist.txt.
    #[arg(long)]
    root: PathBuf,

    /// Manifest to play [default: <root>/filelist.txt].
    #[arg(long)]
    manifest: Option<PathBuf>,

    /// Comma-separated platform prefixes, e.g. MD,SNES.
    #[arg(long, value_delimiter = ',')]
    platforms: Vec<String>,

    /// Title substring (normalized); repeatable, OR-ed.
    #[arg(long)]
    title: Vec<String>,

    /// Drop entries carrying this flag, e.g. INVERT; repeatable.
    #[arg(long = "exclude-flag")]
    exclude_flag: Vec<String>,

    /// Keep only these categories (directories under gifs/); repeatable.
    #[arg(long = "category")]
    category: Vec<String>,

    /// Drop these categories; repeatable.
    #[arg(long = "exclude-category")]
    exclude_category: Vec<String>,

    /// Shuffle instead of playing in manifest order.
    #[arg(long)]
    shuffle: bool,

    /// Seed for the shuffle, so a run can be reproduced.
    #[arg(long, default_value_t = 0x4F50_454E_444D_4400)]
    seed: u64,

    /// Pixels per LED.
    #[arg(long, default_value_t = 8)]
    scale: u32,

    /// Minimum seconds to hold one GIF.
    #[arg(long = "min-dwell", default_value_t = 3.0)]
    min_dwell: f64,

    /// Maximum seconds to hold one GIF.
    #[arg(long = "max-dwell", default_value_t = 30.0)]
    max_dwell: f64,

    /// Don't replay any of the last N entries when shuffling.
    #[arg(long = "no-repeat", default_value_t = 32)]
    no_repeat: usize,
}

/// Opens card-absolute manifest paths (`/gifs/Consoles/X.gif`) under a root
/// directory. The firmware's equivalent joins onto the SD mount point.
struct DirSource {
    root: PathBuf,
}

impl Source for DirSource {
    fn open(&self, path: &str) -> io::Result<Box<dyn Read>> {
        let rel = Path::new(path.trim_start_matches('/'));
        Ok(Box::new(std::fs::File::open(self.root.join(rel))?))
    }
}

fn build_filter(args: &Args) -> Filter {
    let mut filter = Filter::new();
    if !args.platforms.is_empty() {
        filter = filter.platforms(args.platforms.iter().map(|s| s.trim()));
    }
    if !args.category.is_empty() {
        filter = filter.include_categories(&args.category);
    }
    if !args.exclude_category.is_empty() {
        filter = filter.exclude_categories(&args.exclude_category);
    }
    if !args.exclude_flag.is_empty() {
        filter = filter.exclude_flags(&args.exclude_flag);
    }
    for t in &args.title {
        filter = filter.title(t);
    }
    filter
}

/// Startup summary, accumulated *during* the single indexing pass rather than
/// from a `Vec<Entry>` — same numbers, one read of the card, no entries held.
#[derive(Default)]
struct ManifestStats {
    total: usize,
    selected: usize,
    by_platform: BTreeMap<String, usize>,
}

impl ManifestStats {
    fn observe(&mut self, entry: &Entry, matched: bool) {
        self.total += 1;
        if matched {
            self.selected += 1;
            let platform = entry.platform.as_deref().unwrap_or("(none)");
            *self.by_platform.entry(platform.to_string()).or_default() += 1;
        }
    }

    fn print(&self) {
        println!(
            "entries: {} total -> {} after filter",
            self.total, self.selected
        );
        let mut rows: Vec<_> = self.by_platform.iter().collect();
        rows.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
        for (platform, count) in rows {
            println!("  {count:>6}  {platform}");
        }
        println!();
    }
}

/// Nearest-neighbour upscale. At scale >= 4 the last row/column of each cell is
/// left dark, which reads like the gaps between LEDs on a P4 panel.
fn paint(canvas: &Canvas, scale: usize, buffer: &mut [u32]) {
    let row = WIDTH * scale;
    let lit = if scale >= 4 { scale - 1 } else { scale };
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let color = canvas.get(x, y).to_u32();
            for dy in 0..scale {
                let base = (y * scale + dy) * row + x * scale;
                for dx in 0..scale {
                    buffer[base + dx] = if dx < lit && dy < lit { color } else { GUTTER };
                }
            }
        }
    }
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let args = Args::parse();

    if args.scale == 0 || args.scale > 32 {
        bail!("--scale must be between 1 and 32");
    }
    if args.max_dwell < args.min_dwell {
        bail!("--max-dwell must be at least --min-dwell");
    }
    if !args.root.join("gifs").is_dir() {
        log::warn!("{} has no gifs/ directory", args.root.display());
    }

    let manifest_path = args
        .manifest
        .clone()
        .unwrap_or_else(|| args.root.join("filelist.txt"));
    let file = std::fs::File::open(&manifest_path)
        .with_context(|| format!("reading {}", manifest_path.display()))?;

    // One streaming pass builds the offset index and the summary together;
    // the manifest is never materialised as entries. Same code path as the
    // firmware, which is the point of the simulator.
    let filter = build_filter(&args);
    let mut stats = ManifestStats::default();
    let catalog = FileCatalog::build_with(file, &filter, |entry, matched| {
        stats.observe(entry, matched)
    })
    .with_context(|| format!("indexing {}", manifest_path.display()))?;

    if stats.total == 0 {
        bail!("{} has no entries", manifest_path.display());
    }
    stats.print();
    if catalog.is_empty() {
        bail!("the filter matched nothing");
    }
    let total = catalog.len();
    println!("playlist index: {} B\n", catalog.offsets().len() * 4);

    let mode = if args.shuffle {
        Mode::Shuffle
    } else {
        Mode::Sequential
    };
    let scheduler = Scheduler::new(total, mode, args.seed).with_no_repeat(args.no_repeat);
    let dwell = DwellPolicy {
        min: Duration::from_secs_f64(args.min_dwell),
        max: Duration::from_secs_f64(args.max_dwell),
        min_loops: 1,
    };
    let mut engine = Engine::new(
        Box::new(catalog),
        scheduler,
        dwell,
        Box::new(DirSource {
            root: args.root.clone(),
        }),
    );

    let scale = args.scale as usize;
    let (win_w, win_h) = (WIDTH * scale, HEIGHT * scale);
    let mut window = Window::new("OpenDMD", win_w, win_h, WindowOptions::default())
        .context("opening the simulator window")?;
    window.set_target_fps(0); // we pace ourselves off the GIF delays

    let mut buffer = vec![0u32; win_w * win_h];
    let mut canvas = Canvas::new();
    canvas.fill(Rgb::BLACK);
    println!("Space: next   P: pause/resume   Esc: quit\n");

    while window.is_open() && !window.is_key_down(Key::Escape) {
        let now = Instant::now();

        if window.is_key_pressed(Key::Space, KeyRepeat::No) {
            engine.skip();
        }
        if window.is_key_pressed(Key::P, KeyRepeat::No) {
            engine.toggle_pause(now);
            let state = if engine.is_paused() {
                "paused"
            } else {
                "resumed"
            };
            println!("-- {state}");
        }

        match engine.tick(now, &mut canvas)? {
            TickOutcome::Frame {
                playing, started, ..
            } => {
                if started {
                    println!("[{}/{}] {}", playing.ordinal, total, playing.path);
                    window.set_title(&format!("OpenDMD - {}", playing.path));
                }
                paint(&canvas, scale, &mut buffer);
                window.update_with_buffer(&buffer, win_w, win_h)?;
            }
            TickOutcome::Idle => {
                // Keep the event loop responsive without redrawing.
                window.update();
                std::thread::sleep(POLL);
            }
            TickOutcome::Exhausted => bail!("nothing playable under {}", args.root.display()),
        }
    }

    Ok(())
}

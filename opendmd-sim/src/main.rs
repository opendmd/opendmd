//! Desktop simulator for OpenDMD.
//!
//! Glue only: the playlist, filtering, scheduling and GIF decoding all live in
//! `opendmd-core`. This binary owns the window, the keyboard and the clock —
//! exactly the responsibilities the firmware's main loop will own.

use anyhow::{bail, Context, Result};
use clap::Parser;
use minifb::{Key, KeyRepeat, Window, WindowOptions};
use opendmd_core::canvas::{Canvas, Rgb, HEIGHT, WIDTH};
use opendmd_core::compositor::Transition;
use opendmd_core::engine::{
    Catalog, DwellPolicy, Engine, FileCatalog, ReadSeek, Source, TickOutcome,
};
use opendmd_core::filter::Filter;
use opendmd_core::look::{GlowLevel, Look};
use opendmd_core::manifest::Entry;
use opendmd_core::scheduler::{Mode, Scheduler};
use opendmd_core::synth::{self, WallTime};
use opendmd_render::{LedRenderer, PanelModel, MIN_LED_SCALE};
use std::collections::BTreeMap;
use std::io;
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
    #[arg(long, default_value_t = 10)]
    scale: u32,

    /// Draw flat squares with a gutter instead of simulated LEDs.
    #[arg(long)]
    flat: bool,

    /// Gamma used to decode canvas bytes to linear light and re-encode after.
    #[arg(long, default_value_t = 2.2)]
    gamma: f32,

    /// Scales LED intensity, to match the real panel's brightness by eye.
    #[arg(long = "led-gain", default_value_t = 1.0)]
    led_gain: f32,

    /// Halo strength around each lit LED (0 = none). The default was fitted
    /// to the panel photo by eye, so this is the one to tune.
    #[arg(long, default_value_t = opendmd_render::BLOOM_STRENGTH)]
    bloom: f32,

    /// Render the first frame of the first scheduled GIF to a PNG and exit,
    /// without opening a window. Headless-friendly.
    #[arg(long)]
    screenshot: Option<PathBuf>,

    /// Write that same frame as the raw 128x32 canvas, for comparison.
    #[arg(long = "screenshot-canvas")]
    screenshot_canvas: Option<PathBuf>,

    /// Minimum seconds to hold one GIF.
    #[arg(long = "min-dwell", default_value_t = 3.0)]
    min_dwell: f64,

    /// Maximum seconds to hold one GIF.
    #[arg(long = "max-dwell", default_value_t = 30.0)]
    max_dwell: f64,

    /// Don't replay any of the last N entries when shuffling.
    #[arg(long = "no-repeat", default_value_t = 32)]
    no_repeat: usize,

    /// How files come in: dissolve, wipe_left, wipe_right, wipe_down,
    /// wipe_up, blinds, push, fade or none, with an optional duration in ms
    /// (e.g. `wipe_left:250`; 20..2000, default 400).
    #[arg(long, default_value = "dissolve:400", value_parser = parse_transition)]
    transition: Transition,

    /// The panel's tint, as the device's `look` setting: colour, amber,
    /// red, green, blue or white.
    #[arg(long, default_value = "colour", value_parser = parse_look)]
    look: Look,

    /// Phosphor glow, as the device's `glow` setting: off, short or long.
    #[arg(long, default_value = "off", value_parser = parse_glow)]
    glow: GlowLevel,

    /// Play just this path instead of the manifest: a card path, or a
    /// virtual file (`@clock?style=small&date=1`, `@text?msg=Hello`).
    #[arg(long)]
    play: Option<String>,
}

fn parse_look(s: &str) -> Result<Look, String> {
    Look::from_name(s).ok_or_else(|| format!("'{s}': colour, amber, red, green, blue or white"))
}

fn parse_glow(s: &str) -> Result<GlowLevel, String> {
    GlowLevel::from_name(s).ok_or_else(|| format!("'{s}': off, short or long"))
}

/// The host's offset from UTC in seconds, from `date +%z` (`+0100`); 0 if
/// that is not available. Read once: the live clock's local time.
fn utc_offset() -> i64 {
    let out = std::process::Command::new("date").arg("+%z").output().ok();
    let s = out
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .unwrap_or_default();
    let s = s.trim();
    let (sign, digits) = match s.as_bytes().first() {
        Some(b'-') => (-1, &s[1..]),
        Some(b'+') => (1, &s[1..]),
        _ => return 0,
    };
    match (
        digits.get(0..2).and_then(|h| h.parse::<i64>().ok()),
        digits.get(2..4).and_then(|m| m.parse::<i64>().ok()),
    ) {
        (Some(h), Some(m)) => sign * (h * 3600 + m * 60),
        _ => 0,
    }
}

static UTC_OFFSET: std::sync::OnceLock<i64> = std::sync::OnceLock::new();

/// The live clock's source: the host's local time.
fn host_time() -> Option<WallTime> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?;
    let offset = *UTC_OFFSET.get_or_init(utc_offset);
    Some(WallTime::from_unix(
        now.as_secs() as i64 + offset,
        now.subsec_millis() as u16,
    ))
}

fn parse_transition(s: &str) -> Result<Transition, String> {
    Transition::parse(s).ok_or_else(|| {
        format!(
            "'{s}': expected kind[:ms], kind one of {}",
            opendmd_core::compositor::Kind::ALL
                .iter()
                .map(|k| k.name())
                .collect::<Vec<_>>()
                .join(", ")
        )
    })
}

/// Opens card-absolute manifest paths (`/gifs/Consoles/X.gif`) under a root
/// directory. The firmware's equivalent joins onto the SD mount point.
struct DirSource {
    root: PathBuf,
}

impl Source for DirSource {
    fn open(&self, path: &str) -> io::Result<Box<dyn ReadSeek>> {
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

/// Nearest-neighbour upscale: flat squares separated by a one-pixel gutter.
/// The `--flat` renderer, and the only one below [`MIN_LED_SCALE`], where
/// there are too few window pixels per LED for a dot and a bloom to survive.
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

/// How the canvas is drawn. `L` toggles it at runtime.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum RenderMode {
    Led,
    Flat,
}

impl RenderMode {
    fn label(self) -> &'static str {
        match self {
            RenderMode::Led => "LED",
            RenderMode::Flat => "flat",
        }
    }
}

/// Draw one frame into `buffer` through whichever renderer is selected.
fn draw(
    mode: RenderMode,
    canvas: &Canvas,
    scale: usize,
    renderer: &mut Option<LedRenderer>,
    buffer: &mut [u32],
) {
    match (mode, renderer.as_mut()) {
        (RenderMode::Led, Some(r)) => r.render(canvas, buffer),
        _ => paint(canvas, scale, buffer),
    }
}

/// Write a `0x00RRGGBB` framebuffer out as an 8-bit RGB PNG.
fn write_png(path: &Path, width: usize, height: usize, buffer: &[u32]) -> Result<()> {
    let file =
        std::fs::File::create(path).with_context(|| format!("creating {}", path.display()))?;
    let mut encoder = png::Encoder::new(io::BufWriter::new(file), width as u32, height as u32);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header()?;
    let mut rgb = Vec::with_capacity(width * height * 3);
    for px in buffer {
        rgb.push((px >> 16) as u8);
        rgb.push((px >> 8) as u8);
        rgb.push(*px as u8);
    }
    writer.write_image_data(&rgb)?;
    Ok(())
}

/// Advance the engine until it produces the first frame of the first
/// scheduled GIF. No clock is consulted beyond one `Instant::now` epoch, and
/// the engine emits a frame on its first tick, so this is deterministic for a
/// given seed.
fn first_frame(engine: &mut Engine, canvas: &mut Canvas) -> Result<String> {
    let t0 = Instant::now();
    for _ in 0..1024 {
        match engine.tick(t0.elapsed(), canvas)? {
            TickOutcome::Frame { playing, .. } => return Ok(playing.path),
            TickOutcome::Idle => continue,
            TickOutcome::Exhausted => bail!("nothing playable"),
        }
    }
    bail!("the engine produced no frame")
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
    if !(0.5..=4.0).contains(&args.gamma) {
        bail!("--gamma must be between 0.5 and 4.0");
    }
    if !(0.0..=1.0).contains(&args.bloom) {
        bail!("--bloom must be between 0 and 1");
    }
    if !(0.0..=8.0).contains(&args.led_gain) {
        bail!("--led-gain must be between 0.0 and 8.0");
    }
    if !args.root.join("gifs").is_dir() {
        log::warn!("{} has no gifs/ directory", args.root.display());
    }

    synth::set_wall_clock(Some(host_time));
    let manifest_path = args
        .manifest
        .clone()
        .unwrap_or_else(|| args.root.join("filelist.txt"));
    // `--play`: a one-line playlist in memory.
    let file: Box<dyn ReadSeek> = match &args.play {
        Some(p) => Box::new(io::Cursor::new(format!("{p}\n").into_bytes())),
        None => Box::new(
            std::fs::File::open(&manifest_path)
                .with_context(|| format!("reading {}", manifest_path.display()))?,
        ),
    };

    // One streaming pass builds the offset index and the summary together;
    // the manifest is never materialised as entries. Same code path as the
    // firmware, which is the point of the simulator.
    let filter = if args.play.is_some() {
        Filter::new()
    } else {
        build_filter(&args)
    };
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
    engine.set_transition(args.transition);

    let scale = args.scale as usize;
    let (win_w, win_h) = (WIDTH * scale, HEIGHT * scale);

    // The LED renderer needs enough window pixels per LED to put a dot, a
    // bloom and a mask inside one cell; below that the flat renderer is a
    // better picture of the panel than a smeared one.
    let mut mode = if args.flat {
        RenderMode::Flat
    } else if args.scale < MIN_LED_SCALE {
        log::info!(
            "--scale {} is below {MIN_LED_SCALE}: using the flat renderer (LED simulation needs \
             more pixels per LED)",
            args.scale
        );
        RenderMode::Flat
    } else {
        RenderMode::Led
    };
    let mut renderer = (args.scale >= MIN_LED_SCALE)
        .then(|| LedRenderer::new(scale, args.gamma, args.led_gain, args.bloom));

    let mut buffer = vec![0u32; win_w * win_h];
    let mut canvas = Canvas::new();
    canvas.fill(Rgb::BLACK);
    // The panel's tint and glow, as the firmware applies them.
    let mut model = PanelModel::new(args.look, args.glow);

    // Headless: render the first frame of the first scheduled GIF and exit
    // without ever touching the window system. This is the only check that
    // runs on a box with no display.
    if args.screenshot.is_some() || args.screenshot_canvas.is_some() {
        let path = first_frame(&mut engine, &mut canvas)?;
        println!("screenshot of {path}");
        let shown = model.present(&canvas, Duration::ZERO);
        if let Some(out) = &args.screenshot {
            draw(mode, shown, scale, &mut renderer, &mut buffer);
            write_png(out, win_w, win_h, &buffer)?;
            println!(
                "wrote {} ({win_w}x{win_h}, {})",
                out.display(),
                mode.label()
            );
        }
        if let Some(out) = &args.screenshot_canvas {
            // The canvas as the panel shows it (after the tint).
            let raw: Vec<u32> = shown.as_slice().iter().map(|p| p.to_u32()).collect();
            write_png(out, WIDTH, HEIGHT, &raw)?;
            println!("wrote {} ({WIDTH}x{HEIGHT}, raw canvas)", out.display());
        }
        return Ok(());
    }

    let mut window = Window::new(
        &format!("OpenDMD [{}]", mode.label()),
        win_w,
        win_h,
        WindowOptions::default(),
    )
    .context("opening the simulator window")?;
    window.set_target_fps(0); // we pace ourselves off the GIF delays

    let mut playing_path = String::new();
    println!("Space: next   P: pause/resume   L: LED/flat   Esc: quit\n");

    // The engine's clock is a Duration since an epoch we pick, not an
    // `Instant` — see `opendmd_core::engine`. Boot is as good an epoch as any.
    let t0 = Instant::now();

    while window.is_open() && !window.is_key_down(Key::Escape) {
        let now = t0.elapsed();

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
        if window.is_key_pressed(Key::L, KeyRepeat::No) {
            mode = match mode {
                RenderMode::Led => RenderMode::Flat,
                RenderMode::Flat if renderer.is_some() => RenderMode::Led,
                RenderMode::Flat => {
                    log::info!("the LED renderer needs --scale {MIN_LED_SCALE} or more");
                    RenderMode::Flat
                }
            };
            println!("-- renderer: {}", mode.label());
            window.set_title(&format!("OpenDMD [{}] - {playing_path}", mode.label()));
            // Redraw now so the toggle is visible even while paused.
            draw(
                mode,
                model.present(engine.display(&canvas), now),
                scale,
                &mut renderer,
                &mut buffer,
            );
            window.update_with_buffer(&buffer, win_w, win_h)?;
        }

        match engine.tick(now, &mut canvas)? {
            TickOutcome::Frame {
                playing, started, ..
            } => {
                if started {
                    println!("[{}/{}] {}", playing.ordinal, total, playing.path);
                    playing_path = playing.path;
                    window.set_title(&format!("OpenDMD [{}] - {playing_path}", mode.label()));
                }
                // The compositor's output while a transition runs.
                draw(
                    mode,
                    model.present(engine.display(&canvas), now),
                    scale,
                    &mut renderer,
                    &mut buffer,
                );
                window.update_with_buffer(&buffer, win_w, win_h)?;
            }
            TickOutcome::Idle if model.is_animating() => {
                // A dot still fading (phosphor glow).
                draw(
                    mode,
                    model.present(engine.display(&canvas), now),
                    scale,
                    &mut renderer,
                    &mut buffer,
                );
                window.update_with_buffer(&buffer, win_w, win_h)?;
                std::thread::sleep(POLL);
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

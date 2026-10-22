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
    version,
    about = "Play a DMDoS card's GIFs in a 128x32 desktop window",
    after_help = "With no card, a folder picker opens; cancel it (or pass --demo) for the built-in demo \
                  of effects and live items."
)]
struct Args {
    /// A card (a folder holding filelist.txt), any folder of GIFs, or one
    /// GIF. A folder dropped onto the program arrives here.
    #[arg(value_name = "CARD", conflicts_with = "root")]
    card: Option<PathBuf>,

    /// Card root: the directory holding gifs/ and filelist.txt.
    #[arg(long)]
    root: Option<PathBuf>,

    /// Play the built-in demo: the panel's effects, the clock and scrolling
    /// text, no card needed (also what plays when the folder picker is
    /// cancelled).
    #[arg(long, conflicts_with_all = ["card", "root", "manifest", "play"])]
    demo: bool,

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

    /// Take the screenshot this far into the file (ms of its own timeline)
    /// instead of at its first frame: a live effect mid-loop.
    #[arg(long = "screenshot-at", default_value_t = 0)]
    screenshot_at: u64,

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
    /// virtual file (`@clock?style=small&date=1`, `@text?msg=Hello`,
    /// `@fx:tunnel?palette=chrono&cycle=1`).
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

/// The built-in demo: live items only, drawn by the engine, so nothing is
/// read from disk and nothing third-party is shown. The web demo's "All
/// files" (tools/demo-card.py `LIVE`) without its GIFs.
const DEMO: &[&str] = &[
    "@fx:tunnel?palette=fire&texture=rings&twist=2",
    "@text?msg=OpenDMD%20-%20open%20firmware%20for%20LED%20dot-matrix%20displays&speed=30&color=ffaa00",
    "@fx:rotozoom?floor=1&palette=chrono&cycle=1",
    "@fx:wire?shape=tesseract&color=ff66cc",
    "@clock?style=big&date=1",
    "@fx:starfield?palette=ice&density=3",
    "@fx:tunnel?palette=chrono&cycle=1&drift=1",
    "@text?msg=Open%20a%20folder%20of%20GIFs%20to%20play%20your%20own&speed=30&color=00ccff",
    "@fx:rotozoom?palette=rainbow&texture=rings&wobble=2",
    "@fx:wire?shape=morph",
];

/// Most GIFs a folder without a filelist.txt is scanned for.
const MAX_SCAN: usize = 100_000;

/// What to play: where card paths are opened from, and the playlist.
struct Card {
    root: PathBuf,
    playlist: Playlist,
    /// For messages: the folder, the GIF, or "the built-in demo".
    label: String,
}

enum Playlist {
    /// A manifest on disk: `filelist.txt` or `--manifest`.
    File(PathBuf),
    /// Lines made in memory: the demo, a folder's GIFs, one GIF, `--play`.
    Lines(String),
}

impl Card {
    fn demo() -> Card {
        Card {
            root: PathBuf::from("."),
            playlist: Playlist::Lines(DEMO.join("\n") + "\n"),
            label: "the built-in demo".into(),
        }
    }

    /// `--root`, or a card folder: its filelist.txt unless `--manifest` or
    /// `--play` says otherwise. Exactly the simulator's original behaviour.
    fn at_root(args: &Args, root: &Path) -> Card {
        if !root.join("gifs").is_dir() {
            log::warn!("{} has no gifs/ directory", root.display());
        }
        let playlist = match &args.play {
            Some(p) => Playlist::Lines(format!("{p}\n")),
            None => Playlist::File(
                args.manifest
                    .clone()
                    .unwrap_or_else(|| root.join("filelist.txt")),
            ),
        };
        Card {
            root: root.to_path_buf(),
            playlist,
            label: root.display().to_string(),
        }
    }

    /// The positional argument (or the picked folder): a card, any folder
    /// of GIFs, or one GIF.
    fn open(args: &Args, path: &Path) -> Result<Card> {
        if path.is_dir() {
            if args.play.is_some() || args.manifest.is_some() || path.join("filelist.txt").is_file()
            {
                return Ok(Card::at_root(args, path));
            }
            let lines = scan_gifs(path)?;
            if lines.is_empty() {
                bail!(
                    "{} has no filelist.txt and no GIFs: choose a card, or a folder with GIFs in it",
                    path.display()
                );
            }
            println!(
                "{}: no filelist.txt, playing the {} GIFs in it",
                path.display(),
                lines.len()
            );
            return Ok(Card {
                root: path.to_path_buf(),
                playlist: Playlist::Lines(lines.join("\n") + "\n"),
                label: path.display().to_string(),
            });
        }
        if path.is_file() && is_gif(path) {
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .with_context(|| format!("{}: the name is not UTF-8", path.display()))?;
            let root = match path.parent() {
                Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
                _ => PathBuf::from("."),
            };
            return Ok(Card {
                root,
                playlist: Playlist::Lines(format!("/{name}\n")),
                label: path.display().to_string(),
            });
        }
        if !path.exists() {
            bail!("{} does not exist", path.display());
        }
        bail!("{} is not a folder or a GIF", path.display())
    }

    fn describe(&self) -> String {
        match &self.playlist {
            Playlist::File(p) => p.display().to_string(),
            Playlist::Lines(_) => self.label.clone(),
        }
    }

    fn reader(&self) -> Result<Box<dyn ReadSeek>> {
        Ok(match &self.playlist {
            Playlist::File(p) => Box::new(
                std::fs::File::open(p).with_context(|| format!("reading {}", p.display()))?,
            ),
            Playlist::Lines(text) => Box::new(io::Cursor::new(text.clone().into_bytes())),
        })
    }
}

fn is_gif(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("gif"))
}

/// Every GIF under `root` as a card path (`/Sonic/SNES_Sonic_01.gif`),
/// sorted. Hidden entries and a card's `cache/` (prepared copies of the
/// same GIFs) are skipped; symlinked folders are not followed.
fn scan_gifs(root: &Path) -> Result<Vec<String>> {
    let mut out = Vec::new();
    let mut stack = vec![(root.to_path_buf(), String::new())];
    while let Some((dir, prefix)) = stack.pop() {
        let entries =
            std::fs::read_dir(&dir).with_context(|| format!("reading {}", dir.display()))?;
        for entry in entries.flatten() {
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if name.starts_with('.') || name.contains(['\n', '\r']) {
                continue;
            }
            let path = entry.path();
            let card_path = format!("{prefix}/{name}");
            match entry.file_type() {
                Ok(t) if t.is_dir() => {
                    if !(prefix.is_empty() && name.eq_ignore_ascii_case("cache")) {
                        stack.push((path, card_path));
                    }
                }
                _ if path.is_file() && is_gif(&path) => {
                    out.push(card_path);
                    if out.len() >= MAX_SCAN {
                        log::warn!("more than {MAX_SCAN} GIFs: playing the first {MAX_SCAN} found");
                        out.sort();
                        return Ok(out);
                    }
                }
                _ => {}
            }
        }
    }
    out.sort();
    Ok(out)
}

/// The native folder picker, or `None` if it was cancelled or there is no
/// desktop to show it on. On Linux it is the XDG desktop portal (libdbus,
/// loaded at run time), then zenity; neither present means `None`.
fn pick_folder() -> Option<PathBuf> {
    if !has_desktop() {
        return None;
    }
    println!("Choose a card or a folder of GIFs (cancel for the built-in demo)");
    rfd::FileDialog::new()
        .set_title("OpenDMD: choose a card or a folder of GIFs")
        .pick_folder()
}

/// Whether there is a desktop to show a dialog on: on Linux and the BSDs, an
/// X11 or Wayland display (over SSH or on a server there is none).
fn has_desktop() -> bool {
    cfg!(any(windows, target_os = "macos"))
        || std::env::var_os("DISPLAY").is_some()
        || std::env::var_os("WAYLAND_DISPLAY").is_some()
}

/// What to play, from the arguments; with none, the folder picker, and the
/// demo if that is cancelled.
fn choose_card(args: &Args) -> Result<Card> {
    if args.demo {
        return Ok(Card::demo());
    }
    if let Some(root) = &args.root {
        return Ok(Card::at_root(args, root));
    }
    if let Some(path) = &args.card {
        return Card::open(args, path);
    }
    if args.manifest.is_some() {
        bail!("--manifest needs --root (or a card folder)");
    }
    if let Some(p) = &args.play {
        // A live item needs no card; a card path is opened from here.
        return Ok(Card {
            root: PathBuf::from("."),
            playlist: Playlist::Lines(format!("{p}\n")),
            label: p.clone(),
        });
    }
    if args.screenshot.is_some() || args.screenshot_canvas.is_some() {
        return Ok(Card::demo());
    }
    match pick_folder() {
        Some(dir) => Card::open(args, &dir),
        None => {
            println!("No folder chosen: playing the built-in demo (effects and live items)\n");
            Ok(Card::demo())
        }
    }
}

/// Started the way a desktop starts it: double-clicked (no arguments), with
/// a folder dropped on it, or `--demo`. Then an error is also shown in a
/// dialog, because the console it was printed to closes with the program
/// (Windows) or was never there (a Linux file manager).
fn launched_from_desktop(args: &Args) -> bool {
    std::env::args_os().len() <= 2
        && args.root.is_none()
        && args.play.is_none()
        && args.screenshot.is_none()
        && args.screenshot_canvas.is_none()
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info,rfd=off"))
        .init();
    let args = Args::parse();
    let desktop = launched_from_desktop(&args);
    let result = run(args);
    if let (Err(e), true) = (&result, desktop && has_desktop()) {
        rfd::MessageDialog::new()
            .set_level(rfd::MessageLevel::Error)
            .set_title("OpenDMD simulator")
            .set_description(format!("{e:#}"))
            .show();
    }
    result
}

fn run(args: Args) -> Result<()> {
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
    let card = choose_card(&args)?;

    synth::set_wall_clock(Some(host_time));
    let file = card.reader()?;

    // One streaming pass builds the offset index and the summary together;
    // the manifest is never materialised as entries. Same code path as the
    // firmware, which is the point of the simulator.
    let filter = if args.play.is_some() || args.demo {
        Filter::new()
    } else {
        build_filter(&args)
    };
    let mut stats = ManifestStats::default();
    let catalog = FileCatalog::build_with(file, &filter, |entry, matched| {
        stats.observe(entry, matched)
    })
    .with_context(|| format!("indexing {}", card.describe()))?;

    if stats.total == 0 {
        bail!("{} has no entries", card.describe());
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
            root: card.root.clone(),
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
    // Effects hold to 50 fps under the glow, as on the device.
    opendmd_core::fx::set_frame_floor(opendmd_core::fx::glow_floor(args.glow));

    // Headless: render the first frame of the first scheduled GIF and exit
    // without ever touching the window system. This is the only check that
    // runs on a box with no display.
    if args.screenshot.is_some() || args.screenshot_canvas.is_some() {
        let path = first_frame(&mut engine, &mut canvas)?;
        if args.screenshot_at > 0 {
            engine.seek_elapsed(Duration::from_millis(args.screenshot_at), &mut canvas)?;
        }
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
            TickOutcome::Exhausted => bail!("nothing playable in {}", card.label),
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(argv: &[&str]) -> Args {
        Args::try_parse_from(std::iter::once("opendmd-sim").chain(argv.iter().copied())).unwrap()
    }

    #[test]
    fn a_folder_without_a_filelist_plays_its_gifs() {
        let dir = std::env::temp_dir().join(format!("opendmd-sim-scan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for p in [
            "b/MD_B_01.GIF",
            "a.gif",
            "cache/a/x.gif",
            ".hidden/y.gif",
            "notes.txt",
        ] {
            let p = dir.join(p);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, b"GIF89a").unwrap();
        }
        assert_eq!(scan_gifs(&dir).unwrap(), ["/a.gif", "/b/MD_B_01.GIF"]);
        let card = Card::open(&args(&[]), &dir).unwrap();
        assert!(matches!(&card.playlist, Playlist::Lines(t) if t == "/a.gif\n/b/MD_B_01.GIF\n"));
        // One GIF: its folder is the root.
        let card = Card::open(&args(&[]), &dir.join("b/MD_B_01.GIF")).unwrap();
        assert_eq!(card.root, dir.join("b"));
        assert!(matches!(&card.playlist, Playlist::Lines(t) if t == "/MD_B_01.GIF\n"));
        // With a filelist.txt it is a card.
        std::fs::write(dir.join("filelist.txt"), "/a.gif\n").unwrap();
        let card = Card::open(&args(&[]), &dir).unwrap();
        assert!(matches!(&card.playlist, Playlist::File(p) if p == &dir.join("filelist.txt")));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_scripts_arguments_still_parse() {
        // tools/demo-card.py, site-art.py and web-smoke.py
        let a = args(&[
            "--root",
            "card",
            "--play",
            "@clock",
            "--screenshot-canvas",
            "f.png",
            "--screenshot-at",
            "40",
        ]);
        let card = choose_card(&a).unwrap();
        assert_eq!(card.root, Path::new("card"));
        assert!(matches!(&card.playlist, Playlist::Lines(t) if t == "@clock\n"));
        // Headless with no card: the demo, never a dialog.
        let card = choose_card(&args(&["--screenshot", "x.png"])).unwrap();
        assert!(matches!(&card.playlist, Playlist::Lines(t) if t.lines().count() == DEMO.len()));
        assert!(Args::try_parse_from(["opendmd-sim", "card", "--root", "card"]).is_err());
    }
}

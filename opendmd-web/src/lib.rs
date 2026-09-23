//! OpenDMD in the browser: the same `opendmd-core` engine and the same
//! `opendmd-render` LED renderer, compiled to `wasm32-unknown-unknown`.
//!
//! This is the spike behind the plan to make the whole UI — simulator,
//! curation, preprocessing — a web app that the ESP32 will eventually serve
//! off the SD card. Everything heavy runs in the browser: JS reads the files
//! the user picked and hands the bytes over, and from there the playlist,
//! filtering, scheduling, GIF decoding and LED rendering are all Rust.
//!
//! The split inside this file is deliberate:
//!
//! - [`Player`] is plain Rust with no `web-sys` in it, so it can be tested
//!   headlessly (see the tests at the bottom, run by `wasm-pack test --node`).
//! - [`DmdApp`] is the thin `wasm-bindgen` shell that owns the DOM handles.
//!
//! The card is not on a filesystem here, so [`Source`] is backed by the byte
//! arrays JS handed over and [`Catalog`] by the list of reconstructed paths.
//! `Cursor<Vec<u8>>` clones a GIF's bytes on every open (a loop rewinds the
//! cursor instead, so that is once per GIF started) — fine for a spike on a
//! desktop, and the thing to revisit before this becomes the real UI.

use opendmd_core::canvas::{Canvas, HEIGHT, WIDTH};
use opendmd_core::engine::{Catalog, DwellPolicy, Engine, ReadSeek, Source, TickOutcome};
use opendmd_core::filter::Filter;
use opendmd_core::manifest::Entry;
use opendmd_core::scheduler::{Mode, Scheduler};
use opendmd_render::{LedRenderer, BLOOM_STRENGTH, MIN_LED_SCALE};
use std::collections::HashMap;
use std::io::{self, Cursor};
use std::rc::Rc;
use std::time::Duration;
use wasm_bindgen::prelude::*;
use wasm_bindgen::Clamped;

/// The GIFs the user picked, keyed by the card-absolute path we rebuilt for
/// them. Shared (immutably) between the [`Source`] and the catalog builder.
type Files = Rc<HashMap<String, Vec<u8>>>;

/// Rebuild a card-absolute manifest path (`/gifs/<Category>/<file>.gif`) from
/// what the browser gives us.
///
/// A directory pick yields `webkitRelativePath` — `dmdcard/gifs/Consoles/X.gif`
/// or, if the user picked `gifs/` itself, `gifs/Consoles/X.gif`. A plain file
/// pick yields only the basename. The category matters because `Filter` and
/// `Entry` both key off it, so:
///
/// 1. a `gifs` segment with at least two segments after it wins;
/// 2. otherwise the file's own parent directory is the category;
/// 3. otherwise `Unknown`.
pub fn card_path(relative: &str) -> String {
    let segments: Vec<&str> = relative
        .split('/')
        .filter(|s| !s.is_empty() && *s != ".")
        .collect();
    let Some((name, dirs)) = segments.split_last() else {
        return "/gifs/Unknown/".to_string();
    };
    let category = dirs
        .iter()
        .position(|s| s.eq_ignore_ascii_case("gifs"))
        .and_then(|i| dirs.get(i + 1).copied())
        .or_else(|| dirs.last().copied())
        .filter(|s| !s.eq_ignore_ascii_case("gifs"))
        .unwrap_or("Unknown");
    format!("/gifs/{category}/{name}")
}

/// [`Source`] over the in-memory files. `open` clones the GIF's bytes, which
/// a real implementation would not do — see the module docs.
struct MemSource(Files);

impl Source for MemSource {
    fn open(&self, path: &str) -> io::Result<Box<dyn ReadSeek>> {
        match self.0.get(path) {
            Some(bytes) => Ok(Box::new(Cursor::new(bytes.clone()))),
            None => Err(io::Error::new(io::ErrorKind::NotFound, path.to_string())),
        }
    }
}

/// [`Catalog`] over the filtered path list. Like `FileCatalog` it holds no
/// entries: it re-parses one path with [`Entry::from_path`] when the engine is
/// about to open it.
struct MemCatalog(Vec<String>);

impl Catalog for MemCatalog {
    fn len(&self) -> usize {
        self.0.len()
    }

    fn entry(&mut self, i: usize) -> io::Result<Entry> {
        self.0.get(i).map(|p| Entry::from_path(p)).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("catalog index {i} out of range"),
            )
        })
    }
}

/// How the canvas is drawn into the framebuffer.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum RenderMode {
    Led,
    Flat,
}

/// Nearest-neighbour upscale of the raw canvas — the "flat" checkbox, and the
/// only option below [`MIN_LED_SCALE`], where there is no room in a cell for a
/// dot, a bloom and a mask.
fn paint_flat(canvas: &Canvas, scale: usize, out: &mut [u32]) {
    let row = WIDTH * scale;
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let color = canvas.get(x, y).to_u32();
            for dy in 0..scale {
                let base = (y * scale + dy) * row + x * scale;
                out[base..base + scale].fill(color);
            }
        }
    }
}

/// The three sliders and the flat checkbox — everything that changes how the
/// panel is drawn without touching the playlist. Carried as one value so the
/// renderer is rebuilt in exactly one place.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Look {
    pub gamma: f32,
    pub gain: f32,
    pub bloom: f32,
    /// Blit the raw canvas nearest-neighbour instead of simulating LEDs.
    pub flat: bool,
}

/// What one [`Player::tick`] produced, for the status line.
pub struct Update {
    /// 1-based count of GIFs started, the playlist length and the path.
    pub ordinal: u64,
    pub total: usize,
    pub path: String,
    /// True on the first frame of a newly started GIF.
    pub started: bool,
}

/// The whole player, with no browser in it: engine, canvas, renderer and the
/// `0x00RRGGBB` framebuffer, plus the RGBA copy the browser wants.
pub struct Player {
    engine: Engine,
    canvas: Canvas,
    total: usize,
    scale: usize,
    mode: RenderMode,
    renderer: Option<LedRenderer>,
    look: Look,
    buffer: Vec<u32>,
    rgba: Vec<u8>,
}

impl Player {
    /// Build a playlist out of `files` and the filter controls on the page.
    ///
    /// This is `opendmd-sim`'s startup, minus the filesystem: a [`Filter`]
    /// from the platform box, a [`Scheduler`] from the shuffle checkbox and
    /// seed, and an [`Engine`] over both.
    ///
    /// `files` is keyed by whatever the browser called each file; [`card_path`]
    /// turns that into the `/gifs/<Category>/<file>` shape the rest of the
    /// core expects.
    pub fn new(
        files: Vec<(String, Vec<u8>)>,
        platforms: &str,
        shuffle: bool,
        seed: u64,
        scale: usize,
        look: Look,
    ) -> Result<Player, String> {
        let mut filter = Filter::new();
        let platforms: Vec<&str> = platforms
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();
        if !platforms.is_empty() {
            filter = filter.platforms(platforms);
        }

        let map: HashMap<String, Vec<u8>> = files
            .into_iter()
            .map(|(name, bytes)| (card_path(&name), bytes))
            .collect();
        if map.is_empty() {
            return Err("no GIFs were selected".to_string());
        }
        // Paths in insertion-independent but stable order: the scheduler's
        // seed is meaningless otherwise.
        let mut paths: Vec<String> = map.keys().cloned().collect();
        paths.sort();
        paths.retain(|p| filter.matches(&Entry::from_path(p)));
        if paths.is_empty() {
            return Err("the filter matched none of the selected files".to_string());
        }

        let total = paths.len();
        let mode = if shuffle {
            Mode::Shuffle
        } else {
            Mode::Sequential
        };
        let scheduler = Scheduler::new(total, mode, seed).with_no_repeat(32);
        let engine = Engine::new(
            Box::new(MemCatalog(paths)),
            scheduler,
            DwellPolicy::default(),
            Box::new(MemSource(Rc::new(map))),
        );

        let mut player = Player {
            engine,
            canvas: Canvas::new(),
            total,
            scale,
            mode: RenderMode::Flat,
            renderer: None,
            look,
            buffer: vec![0; WIDTH * scale * HEIGHT * scale],
            rgba: vec![255; WIDTH * scale * HEIGHT * scale * 4],
        };
        player.set_look(look);
        Ok(player)
    }

    pub fn total(&self) -> usize {
        self.total
    }

    /// Framebuffer dimensions in browser pixels.
    pub fn size(&self) -> (u32, u32) {
        ((WIDTH * self.scale) as u32, (HEIGHT * self.scale) as u32)
    }

    /// Rebuild the renderer for new slider values. Everything scale-dependent
    /// is precomputed in `LedRenderer::new`, so changing a knob means a new
    /// renderer rather than a per-frame branch.
    pub fn set_look(&mut self, look: Look) {
        let led_possible = self.scale >= MIN_LED_SCALE as usize;
        let changed = look != self.look;
        self.look = look;
        self.mode = if look.flat || !led_possible {
            RenderMode::Flat
        } else {
            RenderMode::Led
        };
        if self.mode == RenderMode::Led && (changed || self.renderer.is_none()) {
            self.renderer = Some(LedRenderer::new(
                self.scale, look.gamma, look.gain, look.bloom,
            ));
        }
    }

    pub fn skip(&mut self) {
        self.engine.skip();
    }

    pub fn toggle_pause(&mut self, now: Duration) -> bool {
        self.engine.toggle_pause(now);
        self.engine.is_paused()
    }

    pub fn is_paused(&self) -> bool {
        self.engine.is_paused()
    }

    /// One engine tick. On a frame the canvas is rendered into [`Player::rgba`]
    /// and an [`Update`] comes back; `Ok(None)` means nothing changed.
    pub fn tick(&mut self, now: Duration) -> Result<Option<Update>, String> {
        match self.engine.tick(now, &mut self.canvas) {
            Ok(TickOutcome::Frame {
                playing, started, ..
            }) => {
                self.draw();
                Ok(Some(Update {
                    ordinal: playing.ordinal,
                    total: self.total,
                    path: playing.path,
                    started,
                }))
            }
            Ok(TickOutcome::Idle) => Ok(None),
            Ok(TickOutcome::Exhausted) => Err("nothing playable in the selected files".to_string()),
            Err(e) => Err(e.to_string()),
        }
    }

    /// Render the current canvas again without advancing the engine — what a
    /// slider move needs so the change is visible while paused.
    pub fn draw(&mut self) {
        match (self.mode, self.renderer.as_mut()) {
            (RenderMode::Led, Some(r)) => r.render(&self.canvas, &mut self.buffer),
            _ => paint_flat(&self.canvas, self.scale, &mut self.buffer),
        }
        // `0x00RRGGBB` -> RGBA8888. Alpha was filled with 255 at construction
        // and is never written again.
        let (chunks, _) = self.rgba.as_chunks_mut::<4>();
        for (px, out) in self.buffer.iter().zip(chunks) {
            out[0] = (px >> 16) as u8;
            out[1] = (px >> 8) as u8;
            out[2] = *px as u8;
        }
    }

    /// The RGBA bytes of the last [`Player::draw`], ready for `ImageData`.
    pub fn rgba(&self) -> &[u8] {
        &self.rgba
    }
}

/// The browser-facing object. Owns the DOM handles and does nothing else: all
/// the logic is in [`Player`].
#[wasm_bindgen]
pub struct DmdApp {
    player: Player,
    ctx: web_sys::CanvasRenderingContext2d,
    status: Option<web_sys::Element>,
    /// Exponential moving average of the render+blit cost, for the status line.
    frame_ms: f64,
    last_path: String,
}

#[wasm_bindgen]
impl DmdApp {
    /// Set up panic hook and logging once, before anything else.
    #[wasm_bindgen(js_name = init)]
    pub fn init() {
        console_error_panic_hook::set_once();
    }

    /// Milliseconds since the page's time origin, i.e. the engine's clock.
    #[wasm_bindgen(js_name = now)]
    pub fn now() -> f64 {
        window().performance().map(|p| p.now()).unwrap_or(0.0)
    }

    /// Build a player over the files JS read for us and size the canvas
    /// element to the panel. `names` and `blobs` are parallel arrays: the
    /// name is `webkitRelativePath` when the browser gave us one, else the
    /// file name. The LED scale comes from the canvas element's own width, so
    /// the page's CSS decides it.
    #[wasm_bindgen(constructor)]
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        canvas_id: &str,
        status_id: &str,
        names: Vec<String>,
        blobs: Vec<js_sys::Uint8Array>,
        platforms: &str,
        shuffle: bool,
        seed: f64,
        gamma: f32,
        gain: f32,
        bloom: f32,
        flat: bool,
    ) -> Result<DmdApp, JsValue> {
        let document = window().document().ok_or_else(|| err("no document"))?;
        let element = document
            .get_element_by_id(canvas_id)
            .ok_or_else(|| err(&format!("no #{canvas_id} element")))?;
        let canvas: web_sys::HtmlCanvasElement = element.dyn_into()?;
        let scale = ((canvas.width() as usize) / WIDTH).clamp(1, 32);

        let files: Vec<(String, Vec<u8>)> = names
            .iter()
            .zip(blobs.iter())
            .map(|(name, blob)| (name.clone(), blob.to_vec()))
            .collect();

        let player = Player::new(
            files,
            platforms,
            shuffle,
            seed as u64,
            scale,
            Look {
                gamma,
                gain,
                bloom,
                flat,
            },
        )
        .map_err(|e| err(&e))?;

        let (w, h) = player.size();
        canvas.set_width(w);
        canvas.set_height(h);
        let ctx: web_sys::CanvasRenderingContext2d = canvas
            .get_context("2d")?
            .ok_or_else(|| err("no 2d context"))?
            .dyn_into()?;

        Ok(DmdApp {
            player,
            ctx,
            status: document.get_element_by_id(status_id),
            frame_ms: 0.0,
            last_path: String::new(),
        })
    }

    /// Playlist length after filtering.
    #[wasm_bindgen(getter)]
    pub fn total(&self) -> usize {
        self.player.total()
    }

    /// One animation-frame step: tick, and on a new frame blit and update the
    /// status line. Returns false once nothing is playable, so the JS loop can
    /// stop.
    pub fn frame(&mut self) -> bool {
        let t0 = Self::now();
        match self.player.tick(Duration::from_secs_f64(t0 / 1000.0)) {
            Ok(Some(update)) => {
                if let Err(e) = self.blit() {
                    self.set_status(&format!("blit failed: {e:?}"));
                    return false;
                }
                let cost = Self::now() - t0;
                // Smoothed, because a single frame's cost is mostly noise.
                self.frame_ms = if self.frame_ms == 0.0 {
                    cost
                } else {
                    self.frame_ms * 0.9 + cost * 0.1
                };
                if update.started {
                    self.last_path = update.path;
                }
                let line = format!(
                    "[{}/{}] {}  —  {:.1} ms/frame",
                    update.ordinal, update.total, self.last_path, self.frame_ms
                );
                self.set_status(&line);
                true
            }
            Ok(None) => true,
            Err(e) => {
                self.set_status(&e);
                false
            }
        }
    }

    pub fn skip(&mut self) {
        self.player.skip();
    }

    /// Returns the new paused state.
    #[wasm_bindgen(js_name = togglePause)]
    pub fn toggle_pause(&mut self) -> bool {
        self.player
            .toggle_pause(Duration::from_secs_f64(Self::now() / 1000.0))
    }

    /// Slider or checkbox moved: rebuild the renderer and redraw at once, so
    /// the change is visible even while paused or between frames.
    #[wasm_bindgen(js_name = setLook)]
    pub fn set_look(&mut self, gamma: f32, gain: f32, bloom: f32, flat: bool) {
        self.player.set_look(Look {
            gamma,
            gain,
            bloom,
            flat,
        });
        self.player.draw();
        let _ = self.blit();
    }

    /// The renderer's default bloom, so the page's slider starts where the
    /// simulator's `--bloom` does rather than duplicating the constant in JS.
    #[wasm_bindgen(js_name = defaultBloom)]
    pub fn default_bloom() -> f32 {
        BLOOM_STRENGTH
    }

    fn blit(&self) -> Result<(), JsValue> {
        let (w, h) = self.player.size();
        let image = web_sys::ImageData::new_with_u8_clamped_array_and_sh(
            Clamped(self.player.rgba()),
            w,
            h,
        )?;
        self.ctx.put_image_data(&image, 0.0, 0.0)
    }

    fn set_status(&self, text: &str) {
        if let Some(el) = &self.status {
            el.set_text_content(Some(text));
        }
    }
}

fn window() -> web_sys::Window {
    web_sys::window().expect("no window: this build only runs in a browser")
}

fn err(message: &str) -> JsValue {
    JsValue::from_str(message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gif::{DisposalMethod, Encoder, Frame, Repeat};
    use opendmd_core::canvas::Rgb;
    use std::borrow::Cow;

    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    /// The page's defaults.
    const LOOK: Look = Look {
        gamma: 2.2,
        gain: 1.0,
        bloom: BLOOM_STRENGTH,
        flat: false,
    };

    /// A one-frame 128x32 GIF with a single white pixel at (4, 2).
    fn white_pixel_gif() -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut enc = Encoder::new(&mut out, WIDTH as u16, HEIGHT as u16, &[]).unwrap();
            enc.set_repeat(Repeat::Infinite).unwrap();
            let mut f = Frame {
                left: 4,
                top: 2,
                width: 1,
                height: 1,
                buffer: Cow::Owned(vec![1u8]),
                dispose: DisposalMethod::Keep,
                delay: 5,
                ..Default::default()
            };
            f.palette = Some(vec![0, 0, 0, 255, 255, 255]);
            enc.write_frame(&f).unwrap();
        }
        out
    }

    #[test]
    fn card_paths_are_reconstructed_from_what_the_browser_gives_us() {
        assert_eq!(
            card_path("dmdcard/gifs/Consoles/SNES_Plok_08.gif"),
            "/gifs/Consoles/SNES_Plok_08.gif"
        );
        assert_eq!(
            card_path("gifs/NEW 2021/MD_Sonic3.gif"),
            "/gifs/NEW 2021/MD_Sonic3.gif"
        );
        // A directory pick aimed straight at a category.
        assert_eq!(
            card_path("Arcade/ARCADE_Pac-Man_01.gif"),
            "/gifs/Arcade/ARCADE_Pac-Man_01.gif"
        );
        // A plain multi-file pick: no directory at all.
        assert_eq!(
            card_path("NES_Gradius.gif"),
            "/gifs/Unknown/NES_Gradius.gif"
        );
        // Files sitting directly in gifs/ have no category to take.
        assert_eq!(
            card_path("gifs/NES_Gradius.gif"),
            "/gifs/Unknown/NES_Gradius.gif"
        );
    }

    /// The page's whole path, headless: bytes in, a lit pixel in the rendered
    /// RGBA buffer out. Same `Source`, `Engine` and `LedRenderer` the browser
    /// drives; only `ImageData` is missing.
    #[test]
    fn a_gif_plays_through_the_engine_and_lights_a_pixel() {
        let scale = 10;
        let mut player = Player::new(
            vec![(
                "dmdcard/gifs/Consoles/SNES_Plok_08.gif".to_string(),
                white_pixel_gif(),
            )],
            "SNES",
            false,
            7,
            scale,
            LOOK,
        )
        .expect("player");
        assert_eq!(player.total(), 1);
        assert_eq!(
            player.size(),
            ((WIDTH * scale) as u32, (HEIGHT * scale) as u32)
        );

        let update = player
            .tick(Duration::ZERO)
            .expect("tick")
            .expect("a frame on the first tick");
        assert!(update.started);
        assert_eq!(update.path, "/gifs/Consoles/SNES_Plok_08.gif");

        // The LED renderer puts the emitter's peak at the centre of the cell.
        let w = WIDTH * scale;
        let centre = ((2 * scale + scale / 2) * w + 4 * scale + scale / 2) * 4;
        let rgba = player.rgba();
        assert_eq!(
            &rgba[centre..centre + 4],
            &[255, 255, 255, 255],
            "the lit LED did not reach the framebuffer"
        );
        // Its neighbour a pitch and a half away is still dark.
        let far = ((2 * scale + scale / 2) * w + 4 * scale + 2 * scale) * 4;
        assert!(rgba[far] < 20, "bloom reaches too far: {}", rgba[far]);
    }

    #[test]
    fn the_platform_filter_rejects_a_selection_that_matches_nothing() {
        let e = Player::new(
            vec![("SNES_Plok_08.gif".to_string(), white_pixel_gif())],
            "MD",
            false,
            0,
            10,
            LOOK,
        )
        .err()
        .expect("a platform filter matching nothing must not build a player");
        assert!(e.contains("matched none"), "{e}");
    }

    #[test]
    fn the_flat_renderer_blits_the_raw_canvas() {
        let scale = 10;
        let mut player = Player::new(
            vec![("SNES_Plok_08.gif".to_string(), white_pixel_gif())],
            "",
            false,
            0,
            scale,
            Look { flat: true, ..LOOK },
        )
        .expect("player");
        player.tick(Duration::ZERO).expect("tick").expect("a frame");
        let w = WIDTH * scale;
        // Every pixel of the lit LED's cell is the canvas colour, corner
        // included — no dot, no bloom, no mask.
        let corner = ((2 * scale) * w + 4 * scale) * 4;
        assert_eq!(&player.rgba()[corner..corner + 3], &[255, 255, 255]);
        // And the raw canvas itself agrees.
        assert_eq!(player.canvas.get(4, 2), Rgb::new(255, 255, 255));
    }
}

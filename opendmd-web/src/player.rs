//! The preview player: one file at a time, no browser types, testable
//! headlessly.
//!
//! The page plays exactly one file at once — the file on the device's panel
//! (Now), the entry under the cursor (Library), or the next pick of the local
//! simulator — so a [`Player`] owns the renderer and the framebuffers for its
//! whole life and swaps only the file underneath them. [`Player::load`] builds
//! a fresh [`Engine`] over a one-entry catalog (cheap: no entries, no index)
//! and keeps the [`LedRenderer`], whose kernel and backdrop are the expensive
//! part.
//!
//! The file's bytes are held once, as an `Rc<[u8]>`: every open (and a dwell
//! that ends re-opens the same entry) hands out a `Cursor` over another handle
//! to the same allocation, never a copy.

use opendmd_core::canvas::{Canvas, HEIGHT, WIDTH};
use opendmd_core::engine::{Catalog, DwellPolicy, Engine, ReadSeek, Source, TickOutcome};
use opendmd_core::manifest::Entry;
use opendmd_core::scheduler::{Mode, Scheduler};
use opendmd_render::{LedRenderer, MIN_LED_SCALE};
use std::io::{self, Cursor};
use std::rc::Rc;
use std::time::Duration;

/// The bytes of the one loaded file, shared between the source and every
/// cursor it has handed out.
#[derive(Clone)]
struct Bytes(Rc<[u8]>);

impl AsRef<[u8]> for Bytes {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

/// [`Source`] over the one loaded file. The engine only ever asks for the
/// path the catalog gave it.
struct OneSource(Bytes);

impl Source for OneSource {
    fn open(&self, _path: &str) -> io::Result<Box<dyn ReadSeek>> {
        Ok(Box::new(Cursor::new(self.0.clone())))
    }
}

/// A one-entry [`Catalog`]: the path parsed on demand, like `FileCatalog`.
struct OneCatalog(String);

impl Catalog for OneCatalog {
    fn len(&self) -> usize {
        1
    }

    fn entry(&mut self, _i: usize) -> io::Result<Entry> {
        Ok(Entry::from_path(&self.0))
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

/// The LED-look sliders and the flat checkbox — everything that changes how
/// the panel is drawn without touching what plays. Carried as one value so
/// the renderer is rebuilt in exactly one place.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Look {
    pub gamma: f32,
    pub gain: f32,
    pub bloom: f32,
    /// Blit the raw canvas nearest-neighbour instead of simulating LEDs.
    pub flat: bool,
}

/// What one [`Player::tick`] produced.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tick {
    /// Nothing to draw: paused, nothing loaded, or the frame is not due.
    Idle,
    /// A frame was rendered into [`Player::rgba`].
    Frame,
    /// A frame was rendered, and it is the first frame after the file's dwell
    /// ran out — the engine re-opened its only entry. A page that plays a
    /// playlist (the local simulator) loads the next file on this; a preview
    /// that loops forever never sees it.
    DwellEnded,
}

/// Play forever: never "done" on loops, never capped.
const LOOP_FOREVER: DwellPolicy = DwellPolicy {
    min: Duration::MAX,
    max: Duration::MAX,
    min_loops: u32::MAX,
};

/// One file, the engine over it, the canvas, the renderer and the
/// `0x00RRGGBB` framebuffer plus the RGBA copy the browser wants.
pub struct Player {
    engine: Option<Engine>,
    path: String,
    dwell: DwellPolicy,
    paused: bool,
    canvas: Canvas,
    scale: usize,
    mode: RenderMode,
    renderer: Option<LedRenderer>,
    look: Look,
    buffer: Vec<u32>,
    rgba: Vec<u8>,
}

impl Player {
    pub fn new(scale: usize, look: Look) -> Player {
        let scale = scale.clamp(1, 32);
        let mut player = Player {
            engine: None,
            path: String::new(),
            dwell: LOOP_FOREVER,
            paused: false,
            canvas: Canvas::new(),
            scale,
            mode: RenderMode::Flat,
            renderer: None,
            look,
            buffer: vec![0; WIDTH * scale * HEIGHT * scale],
            rgba: vec![255; WIDTH * scale * HEIGHT * scale * 4],
        };
        player.set_look(look);
        player.draw();
        player
    }

    /// Framebuffer dimensions in browser pixels.
    pub fn size(&self) -> (u32, u32) {
        ((WIDTH * self.scale) as u32, (HEIGHT * self.scale) as u32)
    }

    /// The loaded file's card-absolute path, or empty.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Swap the file. `path` is only used for its tags (and echoed back);
    /// the bytes may be a GIF or an `.odm` — the engine sniffs them. The
    /// canvas is blanked so a slow next file does not show the old one.
    pub fn load(&mut self, path: &str, bytes: Vec<u8>, now: Duration) {
        let mut engine = Engine::new(
            Box::new(OneCatalog(path.to_string())),
            Scheduler::new(1, Mode::Sequential, 0),
            self.dwell,
            Box::new(OneSource(Bytes(bytes.into()))),
        );
        if self.paused {
            engine.pause(now);
        }
        self.engine = Some(engine);
        self.path = path.to_string();
        self.canvas = Canvas::new();
    }

    /// Drop the file and blank the panel.
    pub fn unload(&mut self) {
        self.engine = None;
        self.path.clear();
        self.canvas = Canvas::new();
        self.draw();
    }

    /// `None` loops the file forever (the preview); `Some((min, max))` gives
    /// it the firmware's dwell (at least one loop and `min`, at most `max`),
    /// after which [`Tick::DwellEnded`] is reported.
    pub fn set_dwell(&mut self, dwell: Option<(Duration, Duration)>) {
        self.dwell = match dwell {
            Some((min, max)) => DwellPolicy {
                min,
                max: max.max(min),
                min_loops: 1,
            },
            None => LOOP_FOREVER,
        };
        if let Some(e) = self.engine.as_mut() {
            e.set_dwell(self.dwell);
        }
    }

    pub fn is_paused(&self) -> bool {
        self.paused
    }

    pub fn set_paused(&mut self, paused: bool, now: Duration) {
        self.paused = paused;
        if let Some(e) = self.engine.as_mut() {
            if paused {
                e.pause(now);
            } else {
                e.resume(now);
            }
        }
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

    /// One engine tick; on a frame the canvas is rendered into
    /// [`Player::rgba`].
    pub fn tick(&mut self, now: Duration) -> Result<Tick, String> {
        let Some(engine) = self.engine.as_mut() else {
            return Ok(Tick::Idle);
        };
        match engine.tick(now, &mut self.canvas) {
            Ok(TickOutcome::Frame {
                playing, started, ..
            }) => {
                self.draw();
                Ok(if started && playing.ordinal > 1 {
                    Tick::DwellEnded
                } else {
                    Tick::Frame
                })
            }
            Ok(TickOutcome::Idle) => Ok(Tick::Idle),
            Ok(TickOutcome::Exhausted) => {
                let path = std::mem::take(&mut self.path);
                self.engine = None;
                Err(format!("{path}: not a playable GIF or .odm"))
            }
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

    #[cfg(test)]
    pub(crate) fn canvas(&self) -> &Canvas {
        &self.canvas
    }
}

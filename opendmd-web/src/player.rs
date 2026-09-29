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
//!
//! **Following the panel.** The Now view's preview plays the file the device
//! is playing, and [`Player::sync_to`] puts it in step: the device reports
//! where the file is in its own timeline (`elapsed_ms`, content time — see
//! `NowPlaying::elapsed`) and the page says when that was on its own clock.
//! From then on the player keeps no frame clock of its own: every tick seeks
//! the engine to where the panel is by now ([`Engine::seek_elapsed`]), so it
//! shows the frame the panel shows, and a new report only moves the anchor.
//! The panel's position advances a little slower than wall time (it sleeps
//! in 10 ms FreeRTOS ticks, and each frame is timed from when the last was
//! ticked), so the page also passes the rate it measured from the device's
//! own reports. Clocks are synced, never pixels: the device sends nothing but
//! two numbers.
//!
//! **Transitions** are the engine's own ([`Engine::set_transition`]): the
//! canvas is kept across [`Player::load`], and the new file's engine is told
//! it carries over what is on it, so the preview goes from one file to the
//! next the way the panel does. What is drawn is always
//! [`Engine::display`] — the compositor's output while one runs.

use opendmd_core::canvas::{Canvas, HEIGHT, WIDTH};
use opendmd_core::compositor::Transition;
use opendmd_core::engine::{Catalog, DwellPolicy, Engine, ReadSeek, Source, TickOutcome};

use opendmd_core::look::{GlowLevel, Look as Tint};
use opendmd_core::manifest::Entry;
use opendmd_core::scheduler::{Mode, Scheduler};
use opendmd_render::{LedRenderer, PanelModel, MIN_LED_SCALE};
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

/// A preview in step with another player (the panel): the file's position
/// is `pos_ms + rate * (now - at_ms)` on the page's clock, or `held_ms`
/// while paused.
#[derive(Clone, Copy, Debug)]
struct Sync {
    at_ms: f64,
    pos_ms: f64,
    rate: f64,
    held_ms: Option<f64>,
    /// The preview got far ahead: start the file again and seek from zero.
    restart: bool,
}

impl Sync {
    fn position(&self, at_ms: f64) -> f64 {
        self.held_ms
            .unwrap_or(self.pos_ms + self.rate * (at_ms - self.at_ms))
    }
}

/// A report within this of where the preview is (or within the frame's
/// delay, when that is shorter) changes nothing but the rate: request-latency
/// noise is well under it with the fastest-sample offset, and a shift this
/// small is below one 60 Hz refresh, so correcting it could only add jitter.
/// A whole frame's delay was tried first: on the rig it let the error run up
/// to a frame before correcting (median 58 ms against 27 ms without it).
pub const DEADBAND_MS: f64 = 20.0;

/// Ahead of the panel by less than this, the preview holds its frame until
/// the panel catches up (what the panel itself does when it is late); by
/// more — the panel started the file again, say — it starts over and seeks.
pub const RESTART_AHEAD_MS: f64 = 1000.0;

fn ms_of(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn engine_for(path: &str, bytes: &Bytes, dwell: DwellPolicy, transition: Transition) -> Engine {
    let mut engine = Engine::new(
        Box::new(OneCatalog(path.to_string())),
        Scheduler::new(1, Mode::Sequential, 0),
        dwell,
        Box::new(OneSource(bytes.clone())),
    );
    engine.set_transition(transition);
    engine
}

/// One file, the engine over it, the canvas, the renderer and the
/// `0x00RRGGBB` framebuffer plus the RGBA copy the browser wants.
pub struct Player {
    engine: Option<Engine>,
    bytes: Option<Bytes>,
    sync: Option<Sync>,
    /// The frame on the canvas: its position in the file, its delay and
    /// when (page clock, ms) it was drawn — what the drift is measured from
    /// before the first sync.
    frame_at_ms: f64,
    frame_delay_ms: f64,
    shown_ms: f64,
    path: String,
    dwell: DwellPolicy,
    transition: Transition,
    /// A frame has been drawn since the canvas was last blanked: the next
    /// file transitions from it.
    shown: bool,
    paused: bool,
    canvas: Canvas,
    scale: usize,
    mode: RenderMode,
    renderer: Option<LedRenderer>,
    look: Look,
    /// The panel's tint and glow (the device's `look` and `glow`), applied
    /// to the picture before it is rendered.
    model: PanelModel,
    /// The clock of the last tick: what the glow steps by.
    now: Duration,
    buffer: Vec<u32>,
    rgba: Vec<u8>,
}

impl Player {
    pub fn new(scale: usize, look: Look) -> Player {
        let scale = scale.clamp(1, 32);
        let mut player = Player {
            engine: None,
            bytes: None,
            sync: None,
            frame_at_ms: 0.0,
            frame_delay_ms: 0.0,
            shown_ms: 0.0,
            path: String::new(),
            dwell: LOOP_FOREVER,
            transition: Transition::DEFAULT,
            shown: false,
            paused: false,
            canvas: Canvas::new(),
            scale,
            mode: RenderMode::Flat,
            renderer: None,
            look,
            model: PanelModel::new(Tint::Colour, GlowLevel::Off),
            now: Duration::ZERO,
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
    /// the bytes may be a GIF or its prepared copy — the engine sniffs them. The
    /// old file's last frame stays up and the new one transitions in from
    /// it (a running transition is finished first, so it starts from what
    /// is really shown). A new file is not in step with anything until
    /// [`Player::sync_to`].
    pub fn load(&mut self, path: &str, bytes: Vec<u8>, now: Duration) {
        let bytes = Bytes(bytes.into());
        if let Some(old) = self.engine.as_mut() {
            old.finish_transition(&mut self.canvas);
        }
        let mut engine = engine_for(path, &bytes, self.dwell, self.transition);
        if self.shown {
            engine.carry_over();
        }
        if self.paused {
            engine.pause(now);
        }
        self.engine = Some(engine);
        self.bytes = Some(bytes);
        self.sync = None;
        self.frame_at_ms = 0.0;
        self.frame_delay_ms = 0.0;
        self.path = path.to_string();
    }

    /// Start the loaded file again from its first frame, transitioning in
    /// from what is shown — the Settings page's "Preview" of a transition.
    /// Not in step with the panel afterwards (it is a demonstration).
    pub fn replay(&mut self, now: Duration) {
        let Some(bytes) = self.bytes.clone() else {
            return;
        };
        let path = self.path.clone();
        self.load(&path, bytes.0.to_vec(), now);
    }

    /// How files come in from now on (see [`Engine::set_transition`]).
    pub fn set_transition(&mut self, transition: Transition) {
        self.transition = transition;
        if let Some(e) = self.engine.as_mut() {
            e.set_transition(transition);
        }
    }

    /// A transition is on screen.
    pub fn is_transitioning(&self) -> bool {
        self.engine.as_ref().is_some_and(Engine::is_transitioning)
    }

    /// Drop the file and blank the panel.
    pub fn unload(&mut self) {
        self.engine = None;
        self.bytes = None;
        self.sync = None;
        self.path.clear();
        self.canvas = Canvas::new();
        self.shown = false;
        self.model.reset();
        self.draw();
    }

    /// The panel's tint and phosphor glow, as the device's config has them
    /// (see `opendmd_core::look`); redrawn at once.
    pub fn set_display(&mut self, look: Tint, glow: GlowLevel) {
        self.model.set(look, glow);
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
        if let Some(sync) = self.sync.as_mut() {
            let now = ms_of(now);
            match (paused, sync.held_ms) {
                (true, None) => sync.held_ms = Some(sync.position(now).max(0.0)),
                (false, Some(held)) => {
                    (sync.at_ms, sync.pos_ms) = (now, held);
                    sync.held_ms = None;
                }
                _ => {}
            }
        }
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

    /// Whether [`Player::sync_to`] has put this file in step.
    pub fn is_synced(&self) -> bool {
        self.sync.is_some()
    }

    /// Where the preview is in the file at `at_ms` (page clock).
    fn position_ms(&self, at_ms: f64) -> f64 {
        match self.sync {
            Some(s) => s.position(at_ms),
            // Free-running: the frame's start plus the time it has been up,
            // never past its delay.
            None if self.paused => self.frame_at_ms,
            None => self.frame_at_ms + (at_ms - self.shown_ms).max(0.0).min(self.frame_delay_ms),
        }
    }

    /// The panel was at `elapsed` in this file at `at_ms` (page clock, the
    /// device's `t_ms` mapped over), and its position advances `rate` ms per
    /// ms (1.0 until the page has measured it); `now_ms` is the page clock.
    ///
    /// Returns the drift before any correction — how far ahead (positive)
    /// or behind the preview was — or `None` with nothing loaded. Within
    /// [`DEADBAND_MS`] (or one frame delay, if shorter) only the rate is
    /// taken; beyond it the anchor moves to the panel's report, and the next
    /// tick seeks forward — or, ahead by more than [`RESTART_AHEAD_MS`],
    /// starts the file again and seeks from zero; ahead by less, it holds its
    /// frame until the panel catches up. The first sync of a file always
    /// takes. While paused the position is held where the panel's is.
    pub fn sync_to(
        &mut self,
        elapsed: Duration,
        at_ms: f64,
        rate: f64,
        now_ms: f64,
    ) -> Option<f64> {
        self.engine.as_ref()?;
        let elapsed_ms = ms_of(elapsed);
        let drift = self.position_ms(at_ms) - elapsed_ms;
        let rate = if rate.is_finite() && rate > 0.0 {
            rate
        } else {
            1.0
        };
        // Not `f64::clamp`: its panic message formats floats, which pulls
        // ~20 KB of float formatting into the wasm.
        #[allow(clippy::manual_clamp)]
        let deadband = self.frame_delay_ms.max(1.0).min(DEADBAND_MS);
        if let Some(sync) = self.sync.as_mut() {
            if drift.abs() < deadband {
                // Re-anchor where the preview already is, at the new rate.
                if sync.held_ms.is_none() {
                    sync.pos_ms = sync.position(now_ms);
                    sync.at_ms = now_ms;
                }
                sync.rate = rate;
                return Some(drift);
            }
        }
        let mut sync = Sync {
            at_ms,
            pos_ms: elapsed_ms,
            rate,
            held_ms: self.paused.then_some(elapsed_ms),
            restart: false,
        };
        sync.restart = self.frame_delay_ms > 0.0
            && self.frame_at_ms - sync.position(now_ms) > RESTART_AHEAD_MS;
        self.sync = Some(sync);
        Some(drift)
    }

    /// Nothing new from the engine, but a dot is still fading: draw the
    /// glow's next step.
    fn fade(&mut self) -> Tick {
        if self.model.is_animating() {
            self.draw();
            Tick::Frame
        } else {
            Tick::Idle
        }
    }

    /// Synced: seek to where the panel is now. Draws when a frame changed.
    fn follow(&mut self, now: Duration) -> Result<Tick, String> {
        let (Some(sync), Some(bytes)) = (self.sync.as_mut(), self.bytes.as_ref()) else {
            return Ok(Tick::Idle);
        };
        if sync.restart {
            sync.restart = false;
            let mut engine = engine_for(&self.path, bytes, self.dwell, self.transition);
            if self.paused {
                engine.pause(now);
            }
            self.engine = Some(engine);
            self.canvas = Canvas::new();
        }
        let target = sync.position(ms_of(now)).max(0.0);
        let Some(engine) = self.engine.as_mut() else {
            return Ok(Tick::Idle);
        };
        match engine.seek_elapsed(Duration::from_secs_f64(target / 1000.0), &mut self.canvas) {
            Ok(seek) => {
                // A transition runs on the page's clock, the file under it
                // on the panel's.
                let stepped = engine.step_transition(now, &mut self.canvas);
                if seek.frames > 0 {
                    self.shown = true;
                    self.frame_at_ms = ms_of(seek.elapsed);
                    self.frame_delay_ms = ms_of(seek.delay);
                    self.shown_ms = ms_of(now);
                } else if !stepped {
                    return Ok(self.fade());
                }
                self.draw();
                Ok(Tick::Frame)
            }
            Err(e) => {
                let path = std::mem::take(&mut self.path);
                self.engine = None;
                self.sync = None;
                Err(format!("{path}: {e}"))
            }
        }
    }

    /// One engine tick; on a frame the canvas is rendered into
    /// [`Player::rgba`]. In step with the panel ([`Player::sync_to`]) this
    /// is a seek instead.
    pub fn tick(&mut self, now: Duration) -> Result<Tick, String> {
        self.now = now;
        if self.sync.is_some() {
            return self.follow(now);
        }
        let Some(engine) = self.engine.as_mut() else {
            return Ok(self.fade());
        };
        match engine.tick(now, &mut self.canvas) {
            Ok(TickOutcome::Frame {
                playing, started, ..
            }) => {
                // The file's own frame timing, not a transition step's.
                let due = engine.frame_due().unwrap_or(now);
                self.frame_at_ms = ms_of(playing.elapsed);
                self.frame_delay_ms = ms_of(due.saturating_sub(now));
                self.shown_ms = ms_of(now);
                self.shown = true;
                self.draw();
                Ok(if started && playing.ordinal > 1 {
                    Tick::DwellEnded
                } else {
                    Tick::Frame
                })
            }
            Ok(TickOutcome::Idle) => Ok(self.fade()),
            Ok(TickOutcome::Exhausted) => {
                let path = std::mem::take(&mut self.path);
                self.engine = None;
                Err(format!("{path}: not a playable GIF"))
            }
            Err(e) => Err(e.to_string()),
        }
    }

    /// Render the current canvas again without advancing the engine — what a
    /// slider move needs so the change is visible while paused.
    pub fn draw(&mut self) {
        let shown = match self.engine.as_ref() {
            Some(e) => e.display(&self.canvas),
            None => &self.canvas,
        };
        let shown = self.model.present(shown, self.now);
        match (self.mode, self.renderer.as_mut()) {
            (RenderMode::Led, Some(r)) => r.render(shown, &mut self.buffer),
            _ => paint_flat(shown, self.scale, &mut self.buffer),
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

    /// What is drawn: the transition's output while one runs.
    #[cfg(test)]
    pub(crate) fn shown(&self) -> &Canvas {
        match self.engine.as_ref() {
            Some(e) => e.display(&self.canvas),
            None => &self.canvas,
        }
    }
}

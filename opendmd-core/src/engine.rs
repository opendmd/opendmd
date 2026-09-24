//! The playlist engine: picks entries, opens them, advances frames, decides
//! when a GIF has had its turn.
//!
//! The engine never sleeps and never reads the clock. The caller (simulator or
//! firmware main loop) passes `now` into [`Engine::tick`] and owns the timing,
//! so the same code drives a 60 Hz desktop loop and a HUB75 refresh loop.
//!
//! `now` is a [`Duration`] since an epoch the *caller* chooses, not an
//! [`std::time::Instant`]: `Instant` has no clock on `wasm32-unknown-unknown`
//! and panics if constructed. The desktop simulator and the firmware pass
//! `boot.elapsed()`, the web app passes `performance.now()`. Only the
//! differences matter, so any fixed epoch works.
//!
//! **Transitions.** When a new file starts and something is already on
//! display, the engine's [`Compositor`] blends from that picture to the new
//! file's frames ([`Engine::set_transition`]; Dissolve over 400 ms by
//! default). The decoder keeps decoding into the caller's canvas exactly as
//! before — it is the file's state and is never drawn on — and while a
//! transition runs the picture to show is the compositor's output:
//! [`Engine::display`] / [`Engine::display_mut`] hand out whichever is
//! current, and [`TickOutcome::Frame::transitioning`] says which. Loops of
//! the same file never transition.
//!
//! **Virtual files and holding.** A path starting with `@` is drawn, not
//! read ([`crate::synth`]: the power-on effect, title cards); the engine
//! opens it itself, before asking its [`Source`]. And a caller that sequences
//! files one after another (the firmware's "Follow RePlayOS": power-on, logo,
//! title card) sets [`Engine::set_hold`]: a file whose dwell is over then
//! stays on its last frame ([`Engine::is_waiting`]) instead of the scheduler
//! moving on, until the caller plays the next one or releases the hold.

use crate::canvas::Canvas;
use crate::compositor::{Compositor, Step, Transition, STEP};
use crate::filter::Filter;
use crate::manifest::{self, Entry};
use crate::overrides::{Dwell, Override, Overrides};
use crate::player::FrameSource;
use crate::scheduler::Scheduler;
use crate::synth;
use std::io::{self, BufReader, Read, Seek, SeekFrom};
use std::time::Duration;

/// A playlist the engine can look entries up in by index.
///
/// The engine only ever asks for an entry when it is about to open the next
/// GIF, so an implementation is free to hold nothing but an index and go back
/// to the card for the rest — which is exactly what [`FileCatalog`] and
/// [`IndexedCatalog`](crate::index::IndexedCatalog) do.
pub trait Catalog {
    fn len(&self) -> usize;

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The `i`-th entry, `i` in `0..len()`.
    fn entry(&mut self, i: usize) -> io::Result<Entry>;
}

/// A [`Catalog`] backed by a seekable `filelist.*`: a `u32` byte offset per
/// matching line, and the file handle to read them back through.
///
/// The index is 4 bytes per entry (~40 KB for the whole card) against the
/// ~590 bytes per entry a `Vec<Entry>` costs, and the only other live
/// allocation is one [`Entry`] at a time inside [`Engine::tick`]. On the ESP32
/// even that 40 KB (and the 64 KB block the `Vec` passes through while it
/// doubles) is too much, so the firmware keeps the same offsets in a file on
/// the card instead — [`IndexedCatalog`](crate::index::IndexedCatalog) — and
/// uses this only as a fallback when the card cannot be written.
///
/// `R` is `std::fs::File` in the simulator (and the firmware's fallback) and
/// `std::io::Cursor<Vec<u8>>` in tests.
pub struct FileCatalog<R: Read + Seek> {
    reader: BufReader<R>,
    offsets: Vec<u32>,
}

impl<R: Read + Seek> FileCatalog<R> {
    /// One streaming pass over the manifest, keeping the offsets of the lines
    /// that pass `filter` and the reader they came from.
    pub fn build(reader: R, filter: &Filter) -> io::Result<Self> {
        Self::build_with(reader, filter, |_, _| {})
    }

    /// [`build`](Self::build), reporting every entry it sees and whether the
    /// filter kept it, so a caller can compute a summary in the *same* pass
    /// rather than reading the card twice.
    pub fn build_with(
        reader: R,
        filter: &Filter,
        mut visit: impl FnMut(&Entry, bool),
    ) -> io::Result<Self> {
        // Small buffer on purpose: this runs on a part with ~120 KB of heap
        // once the HUB75 DMA buffers are allocated.
        let mut reader = BufReader::with_capacity(1024, reader);
        let mut offsets = Vec::new();
        manifest::scan(&mut reader, |offset, entry| {
            let matched = filter.matches(entry);
            if matched {
                offsets.push(offset);
            }
            visit(entry, matched);
        })?;
        offsets.shrink_to_fit();
        Ok(FileCatalog { reader, offsets })
    }

    /// Byte offsets of the matching lines, in manifest order. `4 *
    /// offsets().len()` is the playlist's entire memory cost.
    pub fn offsets(&self) -> &[u32] {
        &self.offsets
    }
}

impl<R: Read + Seek> Catalog for FileCatalog<R> {
    fn len(&self) -> usize {
        self.offsets.len()
    }

    fn entry(&mut self, i: usize) -> io::Result<Entry> {
        let offset = *self.offsets.get(i).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("catalog index {i} out of range"),
            )
        })?;
        manifest::entry_at(&mut self.reader, offset)
    }
}

/// A readable, seekable byte stream: what a [`Source`] hands out. `Seek` is
/// there so a finished file can be looped by rewinding it rather than opening
/// it again — on the ESP32 an open is a linear FAT directory scan, ~330 ms in
/// a 3,781-entry directory, while a seek to 0 costs nothing.
pub trait ReadSeek: Read + Seek {}

impl<T: Read + Seek + ?Sized> ReadSeek for T {}

/// Where GIF (or ODM) bytes come from. The firmware implements this over
/// `/sdcard`, the simulator over a directory. What `open` returns is sniffed
/// by [`FrameSource`], not judged by its extension. Manifest paths are
/// card-absolute (`/gifs/Consoles/X.gif`); joining them onto a root is the
/// source's job.
pub trait Source {
    fn open(&self, path: &str) -> io::Result<Box<dyn ReadSeek>>;
}

/// What the engine plays: a decoder over whatever the source opened.
type Stream = FrameSource<Box<dyn ReadSeek>>;

/// What `gif`'s decoder will still allocate before its first frame is on
/// the canvas (a transition's gate has to leave room for it).
fn first_frame_heap(gif: &Stream) -> usize {
    // (A virtual file allocates nothing.)
    if gif.kind() == "gif" && gif.frames_decoded() == 0 {
        crate::compositor::GIF_FIRST_FRAME_BYTES
    } else {
        0
    }
}

/// How long one GIF holds the display.
///
/// A GIF ends its turn once it has looped `min_loops` times *and* `min` has
/// elapsed, or unconditionally at `max`. Short loops therefore repeat rather
/// than flickering past, and a 10-minute GIF still yields.
#[derive(Clone, Copy, Debug)]
pub struct DwellPolicy {
    pub min: Duration,
    pub max: Duration,
    pub min_loops: u32,
}

/// How long [`DwellPolicy::with_override`] lets a `loops` override run
/// before the hard cap still cuts it off, if the configured `max` is shorter.
pub const LOOPS_MAX: Duration = Duration::from_secs(600);

impl DwellPolicy {
    /// This policy as one entry's override bends it:
    ///
    /// - `dwell=<s>`: the entry holds the panel for `s` seconds — `min` is
    ///   `s`, and `max` is raised to `s` if it was shorter.
    /// - `dwell=<k>x`: `min` and `max` are both scaled by `k`.
    /// - `loops=<n>`: exactly `n` loops — `min_loops` is `n` and, unless a
    ///   `dwell` is also set, `min` is 0 so it moves on after the `n`th; `max`
    ///   is raised to [`LOOPS_MAX`] so a slow loop is not cut short.
    ///
    /// Weights and `never` are the scheduler's business and change nothing
    /// here.
    pub fn with_override(self, ov: &Override) -> DwellPolicy {
        let mut d = self;
        let scale = |t: Duration, k: f32| {
            Duration::try_from_secs_f64(t.as_secs_f64() * f64::from(k)).unwrap_or(Duration::MAX)
        };
        match ov.dwell {
            Some(Dwell::Secs(s)) => {
                let s = Duration::from_secs(u64::from(s));
                d.min = s;
                d.max = d.max.max(s);
            }
            Some(Dwell::Times(k)) => {
                d.min = scale(d.min, k);
                d.max = scale(d.max, k);
            }
            None => {}
        }
        if let Some(n) = ov.loops {
            d.min_loops = u32::from(n);
            if ov.dwell.is_none() {
                d.min = Duration::ZERO;
            }
            d.max = d.max.max(LOOPS_MAX);
        }
        d
    }
}

impl Default for DwellPolicy {
    fn default() -> Self {
        DwellPolicy {
            min: Duration::from_secs(3),
            max: Duration::from_secs(30),
            min_loops: 1,
        }
    }
}

/// What is on screen right now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NowPlaying {
    /// Index into the engine's catalog, or [`OFF_PLAYLIST`] for a file
    /// started with [`Engine::play_now`].
    pub index: usize,
    pub path: String,
    pub title: String,
    /// 1-based count of GIFs started since the engine was created.
    pub ordinal: u64,
    /// Where the frame on screen starts in the file's own timeline: the sum
    /// of the delays of every frame decoded before it since the file
    /// started, loops included (a 3-frame, 300 ms loop on its 5th frame is at
    /// 400 ms). This is *content* time, not `now - started_at`: the caller
    /// shows each frame for at least its delay and usually a little more
    /// (sleep granularity, decode and blit), so wall time since the start
    /// runs ahead of what is on screen. Two players showing the same file are
    /// in step when their positions agree, which is what the web app's
    /// preview syncs on ([`Engine::seek_elapsed`]).
    pub elapsed: Duration,
    /// Frames decoded in this file since it started, loops included; the one
    /// on screen is number `frame` (1-based).
    pub frame: u64,
}

/// [`NowPlaying::index`] of a file started by [`Engine::play_now`]: it did not
/// come from the catalog.
pub const OFF_PLAYLIST: usize = usize::MAX;

/// Result of one [`Engine::tick`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TickOutcome {
    /// Nothing to do: paused, or the current frame's delay hasn't elapsed.
    /// The canvas was not touched.
    Idle,
    /// The picture changed: show [`Engine::display`] (the caller's canvas,
    /// or the compositor's output while a transition runs).
    Frame {
        playing: NowPlaying,
        /// True on the first frame of a newly started GIF.
        started: bool,
        /// When the caller should tick again: the file's next frame, or
        /// sooner while a transition runs — its next step,
        /// [`STEP`](crate::compositor::STEP) after the last, unless the
        /// file's next frame comes within half a step of it (that frame
        /// recomposites anyway).
        next_frame_at: Duration,
        /// A transition is running and [`Engine::display`] is its output,
        /// not the caller's canvas. False on the frame that ends it.
        transitioning: bool,
    },
    /// Nothing is playable — empty playlist, or every candidate failed to open.
    Exhausted,
}

/// How far [`Engine::seek_elapsed`] got.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SeekTo {
    /// Frames decoded by this call (0: already there, nothing drawn).
    pub frames: u32,
    /// The on-screen frame's position afterwards ([`NowPlaying::elapsed`]).
    pub elapsed: Duration,
    /// Its delay: the frame covers `elapsed..elapsed + delay`.
    pub delay: Duration,
    /// False if the bound stopped it short of the target; call again (the
    /// next animation frame) to carry on.
    pub reached: bool,
}

/// [`Engine::seek_elapsed`] decodes at most this many frames per call...
pub const SEEK_MAX_FRAMES: u32 = 2_000;
/// ...or this much content, whichever comes first, so a seek into a long
/// file costs one bounded stall rather than an unbounded one.
pub const SEEK_MAX_CONTENT: Duration = Duration::from_secs(60);

struct Playing {
    index: usize,
    /// This entry's override, if it bends the dwell (`dwell`/`loops`).
    timed: Option<Override>,
    path: String,
    title: String,
    ordinal: u64,
    gif: Stream,
    started_at: Duration,
    next_frame_at: Duration,
    loops: u32,
    first_frame: bool,
    /// Its dwell is over and the engine is holding ([`Engine::set_hold`]):
    /// nothing more is decoded, the last frame stays up.
    done: bool,
    /// Frames decoded in this file, across loops (the decoder's own count
    /// restarts with every loop).
    frames: u64,
    /// Content position of the last decoded frame, and its delay.
    frame_at: Duration,
    frame_delay: Duration,
}

impl Playing {
    /// A frame of `delay` was just decoded: move the file's position past
    /// the previous one.
    fn advance(&mut self, delay: Duration) {
        if self.frames > 0 {
            self.frame_at += self.frame_delay;
        }
        self.frame_delay = delay;
        self.frames += 1;
    }

    fn now_playing(&self) -> NowPlaying {
        NowPlaying {
            index: self.index,
            path: self.path.clone(),
            title: self.title.clone(),
            ordinal: self.ordinal,
            elapsed: self.frame_at,
            frame: self.frames,
        }
    }

    fn new(
        index: usize,
        path: String,
        title: String,
        ordinal: u64,
        gif: Stream,
        at: Duration,
    ) -> Self {
        Playing {
            index,
            timed: None,
            path,
            title,
            ordinal,
            gif,
            started_at: at,
            next_frame_at: at,
            loops: 0,
            first_frame: true,
            done: false,
            frames: 0,
            frame_at: Duration::ZERO,
            frame_delay: Duration::ZERO,
        }
    }
}

/// Drives a scheduled [`Catalog`] over a [`Source`].
///
/// The engine holds no entries: it asks the catalog for one only when it is
/// about to open the next GIF, and keeps just that GIF's path and title in
/// [`Playing`]. Playlist memory is therefore the catalog's index, nothing more.
pub struct Engine {
    catalog: Box<dyn Catalog>,
    scheduler: Scheduler,
    dwell: DwellPolicy,
    /// Per-entry dwell overrides, `(index, override)` sorted by index. Only
    /// the entries with a `dwell` or `loops`; weights live in the scheduler.
    timed: Vec<(u32, Override)>,
    source: Box<dyn Source>,
    current: Option<Playing>,
    paused_since: Option<Duration>,
    started_count: u64,
    skip_requested: bool,
    compositor: Compositor,
    /// Something is on display: a new file transitions from it rather than
    /// cutting in. False until the first frame (nothing to transition from
    /// at boot), unless [`Engine::carry_over`] says the caller's canvas
    /// already shows a picture.
    shown: bool,
    /// A file whose dwell is over waits on its last frame instead of the
    /// scheduler moving on ([`Engine::set_hold`]).
    hold: bool,
}

impl Engine {
    /// `scheduler` should be built over `catalog.len()` — filtering happened
    /// when the catalog was built, and is never re-applied per tick.
    pub fn new(
        catalog: Box<dyn Catalog>,
        scheduler: Scheduler,
        dwell: DwellPolicy,
        source: Box<dyn Source>,
    ) -> Self {
        Engine {
            catalog,
            scheduler,
            dwell,
            timed: Vec::new(),
            source,
            current: None,
            paused_since: None,
            started_count: 0,
            skip_requested: false,
            compositor: Compositor::default(),
            shown: false,
            hold: false,
        }
    }

    /// Hold (or stop holding) at the end of each file: with `hold` set, a
    /// file whose dwell is over stays on its last frame — nothing more is
    /// decoded, [`is_waiting`](Self::is_waiting) turns true — until the
    /// caller [`play_now`](Self::play_now)s another or clears the hold (the
    /// scheduler then moves on at the next tick). For a caller that plays a
    /// sequence of its own and must not let the playlist cut in between.
    pub fn set_hold(&mut self, hold: bool) {
        self.hold = hold;
    }

    /// The file on screen has finished its dwell and is being held.
    pub fn is_waiting(&self) -> bool {
        self.current.as_ref().is_some_and(|p| p.done)
    }

    /// Open `path` for playing: a virtual file ([`synth`]) in memory, else
    /// through the source.
    fn open_stream(&self, path: &str) -> io::Result<Stream> {
        match synth::open(path) {
            Some(bytes) => Stream::open(Box::new(bytes) as Box<dyn ReadSeek>),
            None => self.source.open(path).and_then(Stream::open),
        }
    }

    /// How new files come in (default [`Transition::DEFAULT`], Dissolve over
    /// 400 ms). Takes effect from the next file; a transition already running
    /// finishes as it began.
    pub fn set_transition(&mut self, transition: Transition) {
        self.compositor.set_transition(transition);
    }

    pub fn transition(&self) -> Transition {
        self.compositor.transition()
    }

    /// Asked before each transition takes its 24 KB (see
    /// [`Gate`](crate::compositor::Gate)); refused, the file cuts in.
    pub fn set_transition_gate(&mut self, gate: Option<crate::compositor::Gate>) {
        self.compositor.set_gate(gate);
    }

    /// The caller's canvas already shows a picture — the last frame of an
    /// engine this one replaces (a playlist change, the web preview's next
    /// file) — so the first file transitions from it instead of cutting in.
    pub fn carry_over(&mut self) {
        self.shown = true;
    }

    /// A transition is running: [`display`](Self::display) is its output.
    pub fn is_transitioning(&self) -> bool {
        self.compositor.is_active()
    }

    /// What to show after a tick: the compositor's output while a
    /// transition runs, else `canvas` (the decoder's, as passed to
    /// [`tick`](Self::tick)). The firmware blits `display_mut`, which also
    /// lets the blit clear the dirty map of whichever it is.
    pub fn display<'a>(&'a self, canvas: &'a Canvas) -> &'a Canvas {
        self.compositor.output().unwrap_or(canvas)
    }

    /// [`display`](Self::display), mutably.
    pub fn display_mut<'a>(&'a mut self, canvas: &'a mut Canvas) -> &'a mut Canvas {
        match self.compositor.output_mut() {
            Some(out) => out,
            None => canvas,
        }
    }

    /// End a running transition at once (the new file's frame goes on
    /// display, `canvas`'s dirty map set to what the panel needs to get
    /// there). For a caller about to drop this engine while `canvas` lives
    /// on — the next engine's [`carry_over`](Self::carry_over) then starts
    /// from a picture that is really on the panel.
    pub fn finish_transition(&mut self, canvas: &mut Canvas) {
        self.compositor.finish(canvas);
    }

    /// Step a running transition at `now` if a step is due or the file
    /// underneath drew since the last one; true if the picture to show
    /// changed (a step, or the end). [`tick`](Self::tick) does this itself;
    /// a caller that moves the file with [`seek_elapsed`](Self::seek_elapsed)
    /// instead calls it after each seek. Nothing while paused.
    pub fn step_transition(&mut self, now: Duration, canvas: &mut Canvas) -> bool {
        if self.is_paused() {
            return false;
        }
        self.compositor.step(now, canvas) != Step::Idle
    }

    /// How many entries survived the filter, i.e. the playlist length.
    pub fn catalog_len(&self) -> usize {
        self.catalog.len()
    }

    /// Change the dwell policy of a running engine.
    ///
    /// It takes effect on the GIF already playing — `min`/`max` are measured
    /// against its `started_at`, which is untouched — so a caller can apply a
    /// dwell change live instead of paying for a rebuild of the catalog just to
    /// swap two `Duration`s. The firmware's `PUT /api/config` is why this
    /// exists.
    pub fn set_dwell(&mut self, dwell: DwellPolicy) {
        self.dwell = dwell;
    }

    /// Apply a playlist's resolved overrides: `never` and `w` become the
    /// scheduler's sparse weights (0 for `never`), `dwell` and `loops` are
    /// kept here and bend the [`DwellPolicy`] of that entry when it plays
    /// ([`DwellPolicy::with_override`]). Takes effect from the next entry
    /// opened. The indices are the catalog's, as the index build resolved
    /// them.
    pub fn set_overrides(&mut self, overrides: &Overrides) {
        self.timed.clear();
        for (index, ov) in overrides.iter() {
            if let Some(w) = ov.scheduler_weight() {
                self.scheduler.set_weight(*index as usize, w);
            }
            if ov.affects_dwell() {
                self.timed.push((*index, *ov));
            }
        }
        self.timed.shrink_to_fit();
    }

    /// The override that bends entry `index`'s dwell, if any.
    fn timed_for(&self, index: usize) -> Option<Override> {
        let index = u32::try_from(index).ok()?;
        self.timed
            .binary_search_by_key(&index, |(i, _)| *i)
            .ok()
            .map(|k| self.timed[k].1)
    }

    /// The dwell policy the file on screen plays under: the configured one,
    /// bent by its override.
    pub fn current_dwell(&self) -> DwellPolicy {
        match self.current.as_ref().and_then(|p| p.timed.as_ref()) {
            Some(ov) => self.dwell.with_override(ov),
            None => self.dwell,
        }
    }

    /// Reseed and rewind the scheduler, dropping whatever is on screen.
    ///
    /// The catalog is untouched, so recovering from [`TickOutcome::Exhausted`]
    /// costs no re-read of the card.
    pub fn reset(&mut self, seed: u64) {
        self.scheduler.reset(seed);
        self.current = None;
        self.skip_requested = false;
    }

    /// Drop the file on screen now — its decoder, buffers and handle — and
    /// leave the scheduler where it is: the next [`tick`](Self::tick) opens
    /// the next entry (after a pause, once resumed). For a caller that wants
    /// the heap back while nothing is shown (the firmware's night mode, which
    /// lends it to the cache builder). A running transition is ended first,
    /// so `canvas` holds what the panel was last sent.
    pub fn drop_current(&mut self, canvas: &mut Canvas) {
        self.compositor.finish(canvas);
        self.current = None;
        self.skip_requested = false;
    }

    pub fn now_playing(&self) -> Option<NowPlaying> {
        self.current.as_ref().map(Playing::now_playing)
    }

    /// Play `path` next, now, ahead of the schedule.
    ///
    /// The file is opened straight away through the [`Source`] (so an open
    /// failure is reported to the caller and what is on screen keeps
    /// playing), then the current GIF is dropped *before* the new decoder is
    /// built — never two decoders at once — and the file takes the screen at
    /// the next [`tick`](Engine::tick) with a fresh dwell: it plays under the
    /// same [`DwellPolicy`] as any entry, after which the scheduler resumes
    /// where it was. The scheduler is not advanced and the file need not be in
    /// the catalog; it is reported with [`OFF_PLAYLIST`] as its index. A
    /// pending [`skip`](Engine::skip) is cancelled, and a paused engine stays
    /// paused (the file starts on resume). Its tags come from
    /// [`Entry::from_path`], exactly as if a manifest had named it.
    ///
    /// Errors: the open failed (the current GIF is untouched), or the file
    /// opened but is neither a GIF nor an ODM file (the current GIF is gone
    /// and the scheduler picks the next entry at the next tick).
    pub fn play_now(&mut self, path: &str, now: Duration) -> io::Result<()> {
        self.play_now_with(path, now, None)
    }

    /// [`play_now`](Self::play_now), with `ov` bending this one play's
    /// dwell (`loops=1`: exactly one pass, however short — how the firmware
    /// plays the power-on effect and a title card). The title reported for
    /// a virtual file is what it shows.
    pub fn play_now_with(
        &mut self,
        path: &str,
        now: Duration,
        ov: Option<Override>,
    ) -> io::Result<()> {
        let reader: Box<dyn ReadSeek> = match synth::open(path) {
            Some(bytes) => Box::new(bytes),
            None => self.source.open(path)?,
        };
        self.current = None;
        self.skip_requested = false;
        let gif = Stream::open(reader)?;
        self.started_count += 1;
        // A paused engine has not run its clock; `resume` shifts these by the
        // pause, so start them at the pause instant rather than `now`.
        let at = self.paused_since.unwrap_or(now);
        let title = synth::title(path).unwrap_or_else(|| Entry::from_path(path).title);
        let mut playing = Playing::new(
            OFF_PLAYLIST,
            path.to_string(),
            title,
            self.started_count,
            gif,
            at,
        );
        playing.timed = ov.filter(Override::affects_dwell);
        self.current = Some(playing);
        Ok(())
    }

    /// Finish the current GIF at the next tick.
    pub fn skip(&mut self) {
        self.skip_requested = true;
    }

    pub fn is_paused(&self) -> bool {
        self.paused_since.is_some()
    }

    pub fn pause(&mut self, now: Duration) {
        if self.paused_since.is_none() {
            self.paused_since = Some(now);
        }
    }

    /// Resuming shifts the current GIF's clocks forward by the paused duration,
    /// so a pause doesn't eat its dwell time.
    pub fn resume(&mut self, now: Duration) {
        if let Some(since) = self.paused_since.take() {
            let paused_for = now.saturating_sub(since);
            if let Some(p) = &mut self.current {
                p.started_at += paused_for;
                p.next_frame_at += paused_for;
            }
            self.compositor.shift(paused_for);
        }
    }

    pub fn toggle_pause(&mut self, now: Duration) {
        if self.is_paused() {
            self.resume(now);
        } else {
            self.pause(now);
        }
    }

    /// Advance playback: decode the next frame into `canvas` when it is
    /// due, and step a running transition. Returns whether the picture to
    /// show ([`display`](Self::display)) changed, and what is playing.
    pub fn tick(&mut self, now: Duration, canvas: &mut Canvas) -> io::Result<TickOutcome> {
        if self.is_paused() {
            return Ok(TickOutcome::Idle);
        }
        let decoded = self.advance(now, canvas)?;
        let stepped = self.compositor.step(now, canvas) != Step::Idle;
        let (playing, started) = match decoded {
            TickOutcome::Frame {
                playing, started, ..
            } => (playing, started),
            TickOutcome::Idle if stepped => match &self.current {
                Some(p) => (p.now_playing(), false),
                None => return Ok(TickOutcome::Idle),
            },
            other => return Ok(other),
        };
        self.shown = true;
        let due = self.current.as_ref().map_or(now, |p| p.next_frame_at);
        // A step of its own only when the file's next frame is not about to
        // do it anyway: two ticks a few ms apart would make the first one
        // late by the second's work, for no visible gain.
        let next_frame_at = match self.compositor.step_at() {
            Some(step) if step + STEP / 2 < due => step,
            _ => due,
        };
        Ok(TickOutcome::Frame {
            playing,
            started,
            next_frame_at,
            transitioning: self.compositor.is_active(),
        })
    }

    /// When the file on screen's next frame is due (its own timing, which a
    /// transition's steps do not change).
    pub fn frame_due(&self) -> Option<Duration> {
        self.current.as_ref().map(|p| p.next_frame_at)
    }

    /// The decoding half of [`tick`](Self::tick): open, loop and move on as
    /// the dwell says, decode the next frame when it is due, and start a
    /// transition in front of a new file's first frame.
    fn advance(&mut self, now: Duration, canvas: &mut Canvas) -> io::Result<TickOutcome> {
        if self.skip_requested {
            self.skip_requested = false;
            self.current = None;
        }

        if self.current.is_none() && !self.open_next(now) {
            return Ok(TickOutcome::Exhausted);
        }

        loop {
            let Some(playing) = &mut self.current else {
                return Ok(TickOutcome::Exhausted);
            };
            if playing.done {
                if self.hold {
                    return Ok(TickOutcome::Idle);
                }
                // Released: move on as the dwell said to.
                self.current = None;
                if !self.open_next(now) {
                    return Ok(TickOutcome::Exhausted);
                }
                continue;
            }
            if now < playing.next_frame_at {
                return Ok(TickOutcome::Idle);
            }

            let dwell = match &playing.timed {
                Some(ov) => self.dwell.with_override(ov),
                None => self.dwell,
            };
            // Hard cap: never let one GIF hold the panel past `max`.
            if now.saturating_sub(playing.started_at) >= dwell.max {
                if self.hold {
                    playing.done = true;
                    return Ok(TickOutcome::Idle);
                }
                self.current = None;
                if !self.open_next(now) {
                    return Ok(TickOutcome::Exhausted);
                }
                continue;
            }

            if playing.first_frame && self.shown {
                // The canvas still holds the old picture: transition from it.
                self.compositor
                    .begin(canvas, first_frame_heap(&playing.gif));
            }
            let frame = match playing.gif.next_frame(canvas) {
                Ok(f) => f,
                Err(e) => {
                    log::warn!("decode failed for {}: {e}", playing.path);
                    self.current = None;
                    if !self.open_next(now) {
                        return Ok(TickOutcome::Exhausted);
                    }
                    continue;
                }
            };

            match frame {
                Some(delay) => {
                    let started = playing.first_frame;
                    playing.first_frame = false;
                    playing.next_frame_at = now + delay;
                    playing.advance(delay);
                    self.compositor.frame_decoded();
                    return Ok(TickOutcome::Frame {
                        playing: playing.now_playing(),
                        started,
                        next_frame_at: playing.next_frame_at,
                        transitioning: false,
                    });
                }
                None => {
                    // End of stream with nothing drawn: a header-only or
                    // truncated file. Looping it would spin until `min`.
                    if playing.gif.frames_decoded() == 0 {
                        log::warn!("no frames in {}, skipping", playing.path);
                        self.current = None;
                        if !self.open_next(now) {
                            return Ok(TickOutcome::Exhausted);
                        }
                        continue;
                    }
                    // End of stream: loop the same file or move on.
                    playing.loops += 1;
                    let elapsed = now.saturating_sub(playing.started_at);
                    let done = playing.loops >= dwell.min_loops && elapsed >= dwell.min;
                    if done && self.hold {
                        // Hold the last frame; the reader stays open (it
                        // would be rewound if the file were played again).
                        playing.done = true;
                        return Ok(TickOutcome::Idle);
                    }
                    if done || !self.restart_current() {
                        self.current = None;
                        if !self.open_next(now) {
                            return Ok(TickOutcome::Exhausted);
                        }
                    }
                    continue;
                }
            }
        }
    }

    /// Fast-forward the current file until the frame on the canvas is the
    /// one that covers `target` in the file's own timeline (see
    /// [`NowPlaying::elapsed`]): decode frames into `canvas` — the decoders
    /// are deltas, so every frame is decoded, but the caller renders only the
    /// last — rewinding at the end of the file exactly as a loop does.
    ///
    /// This is how a second player follows the first: the web app's preview
    /// is told where the panel is and seeks there instead of keeping its own
    /// clock. Nothing here reads `now`; the dwell policy is not applied (the
    /// caller that follows another player learns about file changes from it),
    /// and a pause does not stop it. Only forward: a `target` at or before
    /// the frame on screen decodes nothing — to go back, start the file again
    /// and seek from zero.
    ///
    /// Opens the next entry first if nothing is playing (its first frame is
    /// decoded even for a zero `target`). Bounded by [`SEEK_MAX_FRAMES`] and
    /// [`SEEK_MAX_CONTENT`] per call; [`SeekTo::reached`] says whether it got
    /// there. An error means the file is gone (unreadable, or no frames).
    pub fn seek_elapsed(&mut self, target: Duration, canvas: &mut Canvas) -> io::Result<SeekTo> {
        if self.current.is_none() && !self.open_next(Duration::ZERO) {
            return Err(io::Error::new(io::ErrorKind::NotFound, "nothing to play"));
        }
        let mut frames = 0u32;
        let mut content = Duration::ZERO;
        loop {
            let Some(playing) = self.current.as_mut() else {
                return Err(io::Error::new(io::ErrorKind::NotFound, "file gone"));
            };
            let due = playing.frames == 0 || playing.frame_at + playing.frame_delay <= target;
            if !due || frames >= SEEK_MAX_FRAMES || content >= SEEK_MAX_CONTENT {
                return Ok(SeekTo {
                    frames,
                    elapsed: playing.frame_at,
                    delay: playing.frame_delay,
                    reached: !due,
                });
            }
            if playing.first_frame && self.shown {
                self.compositor
                    .begin(canvas, first_frame_heap(&playing.gif));
            }
            match playing.gif.next_frame(canvas)? {
                Some(delay) => {
                    playing.first_frame = false;
                    playing.advance(delay);
                    self.compositor.frame_decoded();
                    self.shown = true;
                    frames += 1;
                    content += delay;
                }
                None if playing.gif.frames_decoded() == 0 => {
                    let path = std::mem::take(&mut playing.path);
                    self.current = None;
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("no frames in {path}"),
                    ));
                }
                None => {
                    playing.loops += 1;
                    if !self.restart_current() {
                        return Err(io::Error::other("rewind failed"));
                    }
                }
            }
        }
    }

    /// Rewind the current file and start a fresh decoder over the same
    /// handle — no second open, so no second directory scan on a FAT card.
    /// False if the rewind or the header fails, in which case `current` is
    /// left empty and the caller moves on.
    fn restart_current(&mut self) -> bool {
        // The finished decoder is dropped (by `into_inner`) *before* the new
        // one is built. A GifSource owns the LZW state plus its buffers;
        // holding two at once is the difference between a ~17 KB and a
        // ~47 KB free-heap floor on the ESP32.
        let Some(playing) = self.current.take() else {
            return false;
        };
        let Playing {
            index,
            timed,
            path,
            title,
            ordinal,
            gif,
            started_at,
            next_frame_at,
            loops,
            first_frame,
            done,
            frames,
            frame_at,
            frame_delay,
        } = playing;
        let mut reader = gif.into_inner();
        match reader
            .seek(SeekFrom::Start(0))
            .and_then(|_| Stream::open(reader))
        {
            Ok(gif) => {
                self.current = Some(Playing {
                    index,
                    timed,
                    path,
                    title,
                    ordinal,
                    gif,
                    started_at,
                    next_frame_at,
                    loops,
                    first_frame,
                    done,
                    frames,
                    frame_at,
                    frame_delay,
                });
                true
            }
            Err(e) => {
                log::warn!("rewind failed for {path}: {e}");
                false
            }
        }
    }

    /// Pull entries from the scheduler until one opens. A GIF that won't open
    /// or won't decode its header is logged and skipped, never fatal.
    fn open_next(&mut self, now: Duration) -> bool {
        let budget = self.scheduler.len().saturating_mul(2).max(32);
        for _ in 0..budget {
            let Some(index) = self.scheduler.next_index() else {
                return false;
            };
            let entry = match self.catalog.entry(index) {
                Ok(entry) => entry,
                Err(e) => {
                    log::warn!("catalog entry {index} unreadable: {e}");
                    continue;
                }
            };
            let (path, title) = (entry.path, entry.title);
            let title = synth::title(&path).unwrap_or(title);
            match self.open_stream(&path) {
                Ok(gif) => {
                    self.started_count += 1;
                    let mut playing =
                        Playing::new(index, path, title, self.started_count, gif, now);
                    playing.timed = self.timed_for(index);
                    self.current = Some(playing);
                    return true;
                }
                Err(e) => log::warn!("skipping {path}: {e}"),
            }
        }
        log::error!("no playable entries after {budget} attempts");
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::Rgb;
    use crate::scheduler::Mode;
    use gif::{DisposalMethod, Encoder, Frame, Repeat};
    use std::borrow::Cow;
    use std::collections::HashMap;
    use std::io::Cursor;

    /// A one-frame 128x32 GIF with a single pixel of the given palette colour.
    fn one_pixel_gif(r: u8, g: u8, b: u8) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut enc = Encoder::new(&mut out, 128, 32, &[]).unwrap();
            enc.set_repeat(Repeat::Infinite).unwrap();
            let mut f = Frame {
                left: 0,
                top: 0,
                width: 1,
                height: 1,
                buffer: Cow::Owned(vec![1u8]),
                dispose: DisposalMethod::Keep,
                delay: 5,
                ..Default::default()
            };
            f.palette = Some(vec![0, 0, 0, r, g, b]);
            enc.write_frame(&f).unwrap();
        }
        out
    }

    struct MemSource(HashMap<String, Vec<u8>>);

    impl Source for MemSource {
        fn open(&self, path: &str) -> io::Result<Box<dyn ReadSeek>> {
            match self.0.get(path) {
                Some(bytes) => Ok(Box::new(Cursor::new(bytes.clone()))),
                None => Err(io::Error::new(io::ErrorKind::NotFound, path.to_string())),
            }
        }
    }

    fn catalog(manifest: &str, filter: &Filter) -> FileCatalog<Cursor<Vec<u8>>> {
        FileCatalog::build(Cursor::new(manifest.as_bytes().to_vec()), filter).unwrap()
    }

    fn engine_with(files: &[(&str, Vec<u8>)], manifest: &str, dwell: DwellPolicy) -> Engine {
        let cat = catalog(manifest, &Filter::new());
        let sched = Scheduler::new(cat.len(), Mode::Sequential, 0);
        let src = MemSource(
            files
                .iter()
                .map(|(p, b)| ((*p).to_string(), b.clone()))
                .collect(),
        );
        Engine::new(Box::new(cat), sched, dwell, Box::new(src))
    }

    #[test]
    fn set_dwell_applies_to_the_gif_already_playing() {
        // A 30 s cap, then tightened to 0 s while the first GIF is on screen:
        // the next tick past the new cap must move on, without the catalog
        // being rebuilt.
        let mut e = engine_with(
            &[
                ("/gifs/Consoles/NES_A_01.gif", one_pixel_gif(255, 0, 0)),
                ("/gifs/Consoles/NES_B_01.gif", one_pixel_gif(0, 255, 0)),
            ],
            "/gifs/Consoles/NES_A_01.gif\n/gifs/Consoles/NES_B_01.gif",
            DwellPolicy {
                min: Duration::from_secs(3),
                max: Duration::from_secs(30),
                min_loops: 1,
            },
        );
        let mut canvas = Canvas::new();
        let first = e.tick(Duration::ZERO, &mut canvas).unwrap();
        let TickOutcome::Frame { playing, .. } = first else {
            panic!("expected a frame, got {first:?}");
        };
        assert_eq!(playing.path, "/gifs/Consoles/NES_A_01.gif");

        e.set_dwell(DwellPolicy {
            min: Duration::ZERO,
            max: Duration::from_millis(1),
            min_loops: 1,
        });
        let next = e.tick(Duration::from_millis(500), &mut canvas).unwrap();
        let TickOutcome::Frame { playing, .. } = next else {
            panic!("expected a frame, got {next:?}");
        };
        assert_eq!(playing.path, "/gifs/Consoles/NES_B_01.gif");
        assert_eq!(e.catalog_len(), 2);
    }

    #[test]
    fn play_now_jumps_the_queue_then_the_schedule_resumes() {
        let dwell = DwellPolicy {
            min: Duration::ZERO,
            max: Duration::from_secs(30),
            min_loops: 1,
        };
        let mut e = engine_with(
            &[
                ("/gifs/Consoles/NES_A_01.gif", one_pixel_gif(255, 0, 0)),
                ("/gifs/Consoles/NES_B_01.gif", one_pixel_gif(0, 255, 0)),
                ("/gifs/Other/SNES_Preview_01.gif", one_pixel_gif(0, 0, 255)),
            ],
            "/gifs/Consoles/NES_A_01.gif\n/gifs/Consoles/NES_B_01.gif",
            dwell,
        );
        let mut canvas = Canvas::new();
        let TickOutcome::Frame { playing, .. } = e.tick(Duration::ZERO, &mut canvas).unwrap()
        else {
            panic!("expected A");
        };
        assert_eq!(playing.title, "A");

        // A missing file is an error and leaves A on screen.
        assert!(e
            .play_now("/gifs/nope.gif", Duration::from_millis(10))
            .is_err());
        assert_eq!(e.now_playing().unwrap().title, "A");

        // Mid-frame: the preview takes the screen at the very next tick, not
        // after A's delay.
        e.play_now("/gifs/Other/SNES_Preview_01.gif", Duration::from_millis(10))
            .unwrap();
        let t = Duration::from_millis(10);
        match e.tick(t, &mut canvas).unwrap() {
            TickOutcome::Frame {
                playing, started, ..
            } => {
                assert!(started);
                assert_eq!(playing.index, OFF_PLAYLIST);
                assert_eq!(playing.path, "/gifs/Other/SNES_Preview_01.gif");
                assert_eq!(playing.title, "Preview");
                assert_eq!(playing.ordinal, 2);
            }
            other => panic!("expected the preview, got {other:?}"),
        }
        assert_eq!(canvas.get(0, 0), Rgb::new(0, 0, 255));

        // Once its dwell is spent the schedule carries on with B — A's turn
        // was used, the scheduler was not advanced by the preview.
        match e.tick(t + Duration::from_millis(100), &mut canvas).unwrap() {
            TickOutcome::Frame { playing, .. } => assert_eq!(playing.title, "B"),
            other => panic!("expected B, got {other:?}"),
        }
    }

    #[test]
    fn plays_and_advances_through_the_playlist() {
        let dwell = DwellPolicy {
            min: Duration::from_millis(0),
            max: Duration::from_secs(30),
            min_loops: 1,
        };
        let mut e = engine_with(
            &[
                ("/gifs/Consoles/NES_A_01.gif", one_pixel_gif(255, 0, 0)),
                ("/gifs/Consoles/NES_B_01.gif", one_pixel_gif(0, 255, 0)),
            ],
            "/gifs/Consoles/NES_A_01.gif\n/gifs/Consoles/NES_B_01.gif",
            dwell,
        );
        assert_eq!(e.catalog_len(), 2);

        let t0 = Duration::ZERO;
        let mut canvas = Canvas::new();
        let out = e.tick(t0, &mut canvas).unwrap();
        match out {
            TickOutcome::Frame {
                playing, started, ..
            } => {
                assert!(started);
                assert_eq!(playing.title, "A");
                assert_eq!(playing.ordinal, 1);
            }
            other => panic!("expected a frame, got {other:?}"),
        }
        assert_eq!(canvas.get(0, 0), Rgb::new(255, 0, 0));

        // Before the frame delay elapses, nothing happens.
        assert_eq!(e.tick(t0, &mut canvas).unwrap(), TickOutcome::Idle);

        // After it, the single-frame GIF hits EOF, min_loops is satisfied and
        // min dwell is zero, so we move to the next entry.
        let t1 = t0 + Duration::from_millis(100);
        match e.tick(t1, &mut canvas).unwrap() {
            TickOutcome::Frame { playing, .. } => assert_eq!(playing.title, "B"),
            other => panic!("expected B, got {other:?}"),
        }
        assert_eq!(canvas.get(0, 0), Rgb::new(0, 255, 0));
    }

    #[test]
    fn odm_entries_play_through_the_same_engine() {
        let mut odm = Vec::new();
        crate::odm::encode_gif(&one_pixel_gif(0, 0, 255)[..], &mut odm).unwrap();
        let dwell = DwellPolicy {
            min: Duration::ZERO,
            max: Duration::from_secs(30),
            min_loops: 1,
        };
        let mut e = engine_with(
            &[
                ("/gifs/Consoles/NES_A_01.odm", odm),
                ("/gifs/Consoles/NES_B_01.gif", one_pixel_gif(0, 255, 0)),
            ],
            "/gifs/Consoles/NES_A_01.odm\n/gifs/Consoles/NES_B_01.gif",
            dwell,
        );
        let mut canvas = Canvas::new();
        match e.tick(Duration::ZERO, &mut canvas).unwrap() {
            TickOutcome::Frame {
                playing,
                next_frame_at,
                ..
            } => {
                assert_eq!(playing.title, "A");
                assert_eq!(next_frame_at, Duration::from_millis(50));
            }
            other => panic!("expected A, got {other:?}"),
        }
        assert_eq!(canvas.get(0, 0), Rgb::new(0, 0, 255));
        // It loops and hands over exactly as the GIF path does.
        match e.tick(Duration::from_millis(100), &mut canvas).unwrap() {
            TickOutcome::Frame { playing, .. } => assert_eq!(playing.title, "B"),
            other => panic!("expected B, got {other:?}"),
        }
        assert_eq!(canvas.get(0, 0), Rgb::new(0, 255, 0));
    }

    #[test]
    fn short_gifs_loop_until_min_dwell() {
        let dwell = DwellPolicy {
            min: Duration::from_secs(3),
            max: Duration::from_secs(30),
            min_loops: 1,
        };
        let mut e = engine_with(
            &[
                ("/gifs/Consoles/NES_A_01.gif", one_pixel_gif(255, 0, 0)),
                ("/gifs/Consoles/NES_B_01.gif", one_pixel_gif(0, 255, 0)),
            ],
            "/gifs/Consoles/NES_A_01.gif\n/gifs/Consoles/NES_B_01.gif",
            dwell,
        );
        let t0 = Duration::ZERO;
        let mut canvas = Canvas::new();
        e.tick(t0, &mut canvas).unwrap();
        // 1 s in: looped, but min dwell not reached -> still A.
        match e.tick(t0 + Duration::from_secs(1), &mut canvas).unwrap() {
            TickOutcome::Frame {
                playing, started, ..
            } => {
                assert_eq!(playing.title, "A");
                assert!(!started);
            }
            other => panic!("expected A again, got {other:?}"),
        }
        // Past min dwell -> B.
        match e.tick(t0 + Duration::from_secs(4), &mut canvas).unwrap() {
            TickOutcome::Frame { playing, .. } => assert_eq!(playing.title, "B"),
            other => panic!("expected B, got {other:?}"),
        }
    }

    /// A two-frame GIF: a red pixel at (0,0), then a green one at (1,0).
    fn two_frame_gif() -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut enc = Encoder::new(&mut out, 128, 32, &[]).unwrap();
            for (x, colour) in [(0u16, [255, 0, 0]), (1, [0, 255, 0])] {
                let mut f = Frame {
                    left: x,
                    width: 1,
                    height: 1,
                    buffer: Cow::Owned(vec![1u8]),
                    dispose: DisposalMethod::Keep,
                    delay: 5,
                    ..Default::default()
                };
                f.palette = Some([&[0, 0, 0][..], &colour[..]].concat());
                enc.write_frame(&f).unwrap();
            }
        }
        out
    }

    /// Counts opens, so a loop can be shown not to open the file again.
    struct CountingSource(MemSource, std::rc::Rc<std::cell::Cell<u32>>);

    impl Source for CountingSource {
        fn open(&self, path: &str) -> io::Result<Box<dyn ReadSeek>> {
            self.1.set(self.1.get() + 1);
            self.0.open(path)
        }
    }

    #[test]
    fn looping_rewinds_instead_of_reopening() {
        let mut odm = Vec::new();
        crate::odm::encode_gif(&two_frame_gif()[..], &mut odm).unwrap();
        for (path, bytes) in [
            ("/gifs/Consoles/NES_A_01.gif", two_frame_gif()),
            ("/gifs/Consoles/NES_A_01.odm", odm),
        ] {
            let opens = std::rc::Rc::new(std::cell::Cell::new(0));
            let cat = catalog(path, &Filter::new());
            let sched = Scheduler::new(cat.len(), Mode::Sequential, 0);
            let src = CountingSource(
                MemSource([(path.to_string(), bytes)].into_iter().collect()),
                opens.clone(),
            );
            let dwell = DwellPolicy {
                min: Duration::from_secs(10),
                max: Duration::from_secs(30),
                min_loops: 1,
            };
            let mut e = Engine::new(Box::new(cat), sched, dwell, Box::new(src));
            let mut canvas = Canvas::new();
            // Five loops of two frames: every loop starts from a cleared
            // canvas (red only) and ends with red and green, and the frames
            // of the first loop repeat exactly.
            for i in 0..10u64 {
                let t = Duration::from_millis(50 * i);
                let TickOutcome::Frame { started, .. } = e.tick(t, &mut canvas).unwrap() else {
                    panic!("{path}: expected a frame at {t:?}");
                };
                assert_eq!(started, i == 0);
                assert_eq!(canvas.get(0, 0), Rgb::new(255, 0, 0), "{path} tick {i}");
                let second = if i % 2 == 0 {
                    Rgb::BLACK
                } else {
                    Rgb::new(0, 255, 0)
                };
                assert_eq!(canvas.get(1, 0), second, "{path} tick {i}");
            }
            assert_eq!(opens.get(), 1, "{path}: looping opened the file again");
            assert_eq!(e.now_playing().unwrap().ordinal, 1);
        }
    }

    /// Three frames of 100, 200 and 300 ms: a red, a green and a blue pixel
    /// at (0,0), so the canvas says which frame is up. One loop is 600 ms.
    fn three_frame_gif() -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut enc = Encoder::new(&mut out, 128, 32, &[]).unwrap();
            enc.set_repeat(Repeat::Infinite).unwrap();
            for (delay, colour) in [(10u16, [255, 0, 0]), (20, [0, 255, 0]), (30, [0, 0, 255])] {
                let mut f = Frame {
                    width: 1,
                    height: 1,
                    buffer: Cow::Owned(vec![1u8]),
                    dispose: DisposalMethod::Keep,
                    delay,
                    ..Default::default()
                };
                f.palette = Some([&[0, 0, 0][..], &colour[..]].concat());
                enc.write_frame(&f).unwrap();
            }
        }
        out
    }

    #[test]
    fn seek_elapsed_lands_on_the_frame_and_loop_that_cover_the_target() {
        const RED: Rgb = Rgb::new(255, 0, 0);
        const GREEN: Rgb = Rgb::new(0, 255, 0);
        const BLUE: Rgb = Rgb::new(0, 0, 255);
        let file = "/gifs/Consoles/NES_Three_01.gif";
        let mut odm = Vec::new();
        crate::odm::encode_gif(&three_frame_gif()[..], &mut odm).unwrap();
        for (path, bytes) in [
            (file, three_frame_gif()),
            ("/gifs/Consoles/NES_Three_01.odm", odm),
        ] {
            let mut e = engine_with(&[(path, bytes)], path, DwellPolicy::default());
            let mut canvas = Canvas::new();
            let seek = |e: &mut Engine, canvas: &mut Canvas, ms| {
                e.seek_elapsed(Duration::from_millis(ms), canvas).unwrap()
            };

            // Nothing open yet: the first frame is decoded even for zero.
            let s = seek(&mut e, &mut canvas, 0);
            assert_eq!(
                (s.frames, s.elapsed, s.delay, s.reached),
                (1, Duration::ZERO, Duration::from_millis(100), true),
                "{path}"
            );
            assert_eq!(canvas.get(0, 0), RED);
            // Inside frame 2 (100..300).
            let s = seek(&mut e, &mut canvas, 150);
            assert_eq!(
                (s.frames, s.elapsed),
                (1, Duration::from_millis(100)),
                "{path}"
            );
            assert_eq!(canvas.get(0, 0), GREEN);
            // Third loop (1200..1800), frame 2 of it (1300..1500): the 8th frame.
            let s = seek(&mut e, &mut canvas, 1350);
            assert_eq!(
                (s.frames, s.elapsed, s.reached),
                (6, Duration::from_millis(1300), true),
                "{path}"
            );
            assert_eq!(canvas.get(0, 0), GREEN);
            let np = e.now_playing().unwrap();
            assert_eq!(
                (np.frame, np.elapsed, np.ordinal),
                (8, Duration::from_millis(1300), 1)
            );
            // Exactly on a boundary is the new frame; backwards decodes nothing.
            assert_eq!(
                seek(&mut e, &mut canvas, 1500).elapsed,
                Duration::from_millis(1500)
            );
            assert_eq!(canvas.get(0, 0), BLUE);
            assert_eq!(seek(&mut e, &mut canvas, 200).frames, 0);
            assert_eq!(canvas.get(0, 0), BLUE);

            // Ten minutes in is past the per-call bound: it stops after 60 s
            // of content and says so; calling again carries on from there.
            let s = seek(&mut e, &mut canvas, 600_000);
            assert!(!s.reached && s.elapsed >= Duration::from_secs(60), "{s:?}");
            let mut calls = 1;
            while !seek(&mut e, &mut canvas, 600_000).reached {
                calls += 1;
            }
            // ~598.5 s of content at <= 60 s a call: ten calls in all.
            assert_eq!(calls + 1, 10);
            // 600 s is the start of the 1001st loop: frame 3001, red.
            let np = e.now_playing().unwrap();
            assert_eq!((np.frame, np.elapsed), (3001, Duration::from_secs(600)));
            assert_eq!(canvas.get(0, 0), RED);
        }
    }

    #[test]
    fn tick_reports_the_content_position_not_the_wall_clock() {
        let path = "/gifs/Consoles/NES_Three_01.gif";
        let mut e = engine_with(&[(path, three_frame_gif())], path, DwellPolicy::default());
        let mut canvas = Canvas::new();
        // Every frame is ticked 7 ms late: wall time runs ahead, the
        // position does not.
        let mut now = Duration::ZERO;
        let mut seen = Vec::new();
        for _ in 0..5 {
            let TickOutcome::Frame {
                playing,
                next_frame_at,
                ..
            } = e.tick(now, &mut canvas).unwrap()
            else {
                panic!("expected a frame at {now:?}");
            };
            seen.push((playing.frame, playing.elapsed.as_millis()));
            now = next_frame_at + Duration::from_millis(7);
        }
        assert_eq!(seen, [(1, 0), (2, 100), (3, 300), (4, 600), (5, 700)]);
    }

    #[test]
    fn max_dwell_cuts_a_long_gif_off() {
        let dwell = DwellPolicy {
            min: Duration::from_secs(3),
            max: Duration::from_secs(5),
            min_loops: 100,
        };
        let mut e = engine_with(
            &[
                ("/gifs/Consoles/NES_A_01.gif", one_pixel_gif(255, 0, 0)),
                ("/gifs/Consoles/NES_B_01.gif", one_pixel_gif(0, 255, 0)),
            ],
            "/gifs/Consoles/NES_A_01.gif\n/gifs/Consoles/NES_B_01.gif",
            dwell,
        );
        let t0 = Duration::ZERO;
        let mut canvas = Canvas::new();
        e.tick(t0, &mut canvas).unwrap();
        match e.tick(t0 + Duration::from_secs(6), &mut canvas).unwrap() {
            TickOutcome::Frame { playing, .. } => assert_eq!(playing.title, "B"),
            other => panic!("expected B, got {other:?}"),
        }
    }

    fn three_entries(dwell: DwellPolicy) -> Engine {
        engine_with(
            &[
                ("/gifs/Consoles/NES_A_01.gif", one_pixel_gif(255, 0, 0)),
                ("/gifs/Consoles/NES_B_01.gif", one_pixel_gif(0, 255, 0)),
                ("/gifs/Consoles/NES_C_01.gif", one_pixel_gif(0, 0, 255)),
            ],
            "/gifs/Consoles/NES_A_01.gif\n/gifs/Consoles/NES_B_01.gif\n/gifs/Consoles/NES_C_01.gif",
            dwell,
        )
    }

    fn title_at(e: &mut Engine, t: Duration, canvas: &mut Canvas) -> String {
        match e.tick(t, canvas).unwrap() {
            TickOutcome::Frame { playing, .. } => playing.title,
            other => panic!("expected a frame at {t:?}, got {other:?}"),
        }
    }

    #[test]
    fn a_hidden_entry_is_never_opened() {
        let dwell = DwellPolicy {
            min: Duration::ZERO,
            max: Duration::from_secs(30),
            min_loops: 1,
        };
        let mut e = three_entries(dwell);
        let mut ov = Overrides::new();
        ov.push(
            1,
            Override {
                never: true,
                ..Default::default()
            },
        );
        e.set_overrides(&ov);
        let mut canvas = Canvas::new();
        let titles: Vec<String> = (0..6)
            .map(|k| title_at(&mut e, Duration::from_millis(100 * k), &mut canvas))
            .collect();
        assert_eq!(titles, ["A", "C", "A", "C", "A", "C"]);
    }

    #[test]
    fn dwell_and_loops_overrides_bend_that_entry_only() {
        let dwell = DwellPolicy {
            min: Duration::from_secs(3),
            max: Duration::from_secs(30),
            min_loops: 1,
        };
        let mut e = three_entries(dwell);
        let mut ov = Overrides::new();
        // A: exactly 2 loops (its loop is 50 ms), not 3 s.
        ov.push(
            0,
            Override {
                loops: Some(2),
                ..Default::default()
            },
        );
        // B: 10 s instead of 3.
        ov.push(
            1,
            Override {
                dwell: Some(crate::overrides::Dwell::Secs(10)),
                ..Default::default()
            },
        );
        e.set_overrides(&ov);
        let mut canvas = Canvas::new();
        let ms = Duration::from_millis;
        assert_eq!(title_at(&mut e, ms(0), &mut canvas), "A");
        assert_eq!(e.current_dwell().min_loops, 2);
        assert_eq!(
            title_at(&mut e, ms(50), &mut canvas),
            "A",
            "first loop done, one to go"
        );
        assert_eq!(
            title_at(&mut e, ms(100), &mut canvas),
            "B",
            "two loops, then on"
        );
        assert_eq!(e.current_dwell().min, Duration::from_secs(10));
        assert_eq!(
            title_at(&mut e, ms(5_100), &mut canvas),
            "B",
            "past 3 s, not past 10"
        );
        assert_eq!(title_at(&mut e, ms(10_200), &mut canvas), "C");
        assert_eq!(
            e.current_dwell().min,
            Duration::from_secs(3),
            "C is as configured"
        );
    }

    #[test]
    fn with_override_scales_raises_and_caps() {
        let base = DwellPolicy {
            min: Duration::from_secs(3),
            max: Duration::from_secs(30),
            min_loops: 1,
        };
        let twice = base.with_override(&Override {
            dwell: Some(crate::overrides::Dwell::Times(2.0)),
            ..Default::default()
        });
        assert_eq!(
            (twice.min, twice.max),
            (Duration::from_secs(6), Duration::from_secs(60))
        );
        let long = base.with_override(&Override {
            dwell: Some(crate::overrides::Dwell::Secs(45)),
            ..Default::default()
        });
        assert_eq!(
            (long.min, long.max),
            (Duration::from_secs(45), Duration::from_secs(45))
        );
        let looped = base.with_override(&Override {
            loops: Some(3),
            ..Default::default()
        });
        assert_eq!(
            (looped.min, looped.min_loops, looped.max),
            (Duration::ZERO, 3, LOOPS_MAX)
        );
        // The web preview's "forever" does not overflow.
        let forever = DwellPolicy {
            min: Duration::MAX,
            max: Duration::MAX,
            min_loops: u32::MAX,
        };
        assert_eq!(forever.with_override(&twice_ov()).max, Duration::MAX);
        // Weights change nothing here.
        let fav = Override {
            weight: Some(3.0),
            ..Default::default()
        };
        assert_eq!(base.with_override(&fav).min, base.min);
    }

    fn twice_ov() -> Override {
        Override {
            dwell: Some(crate::overrides::Dwell::Times(2.0)),
            ..Default::default()
        }
    }

    #[test]
    fn favourites_are_drawn_more_often() {
        let dwell = DwellPolicy {
            min: Duration::ZERO,
            max: Duration::from_secs(30),
            min_loops: 1,
        };
        let cat = catalog(
            "/gifs/Consoles/NES_A_01.gif\n/gifs/Consoles/NES_B_01.gif\n/gifs/Consoles/NES_C_01.gif",
            &Filter::new(),
        );
        let sched = Scheduler::new(cat.len(), Mode::Shuffle, 11);
        let src = MemSource(
            [("A", (255, 0, 0)), ("B", (0, 255, 0)), ("C", (0, 0, 255))]
                .iter()
                .map(|(t, (r, g, b))| {
                    (
                        format!("/gifs/Consoles/NES_{t}_01.gif"),
                        one_pixel_gif(*r, *g, *b),
                    )
                })
                .collect(),
        );
        let mut e = Engine::new(Box::new(cat), sched, dwell, Box::new(src));
        let mut ov = Overrides::new();
        ov.push(
            2,
            Override {
                weight: Some(3.0),
                ..Default::default()
            },
        );
        e.set_overrides(&ov);
        let mut canvas = Canvas::new();
        let mut c = 0;
        for k in 0..600 {
            if title_at(&mut e, Duration::from_millis(100 * k), &mut canvas) == "C" {
                c += 1;
            }
        }
        // Expected share 3/5 = 360 of 600.
        assert!((300..420).contains(&c), "favourite drawn {c}/600");
    }

    /// Valid header and trailer, zero image frames.
    fn zero_frame_gif() -> Vec<u8> {
        let mut out = Vec::new();
        Encoder::new(&mut out, 128, 32, &[]).unwrap();
        out
    }

    #[test]
    fn a_zero_frame_gif_is_skipped_not_spun_on() {
        let mut e = engine_with(
            &[
                ("/gifs/Consoles/NES_Empty_01.gif", zero_frame_gif()),
                ("/gifs/Consoles/NES_Good_01.gif", one_pixel_gif(0, 255, 0)),
            ],
            "/gifs/Consoles/NES_Empty_01.gif\n/gifs/Consoles/NES_Good_01.gif",
            DwellPolicy::default(),
        );
        let mut canvas = Canvas::new();
        // The engine clock is a caller-chosen Duration; "did it spin?" is a
        // question about wall time, so that one assertion uses a real clock.
        let wall = std::time::Instant::now();
        match e.tick(Duration::ZERO, &mut canvas).unwrap() {
            TickOutcome::Frame { playing, .. } => assert_eq!(playing.title, "Good"),
            other => panic!("expected Good, got {other:?}"),
        }
        assert!(
            wall.elapsed() < Duration::from_secs(1),
            "engine spun on the empty GIF"
        );
    }

    #[test]
    fn a_broken_gif_is_skipped_not_fatal() {
        let mut e = engine_with(
            &[
                ("/gifs/Consoles/NES_Bad_01.gif", b"junk".to_vec()),
                ("/gifs/Consoles/NES_Good_01.gif", one_pixel_gif(0, 0, 255)),
            ],
            "/gifs/Consoles/NES_Bad_01.gif\n\
             /gifs/Consoles/NES_Missing_01.gif\n\
             /gifs/Consoles/NES_Good_01.gif",
            DwellPolicy::default(),
        );
        let mut canvas = Canvas::new();
        match e.tick(Duration::ZERO, &mut canvas).unwrap() {
            TickOutcome::Frame { playing, .. } => assert_eq!(playing.title, "Good"),
            other => panic!("expected Good, got {other:?}"),
        }
    }

    #[test]
    fn nothing_playable_is_reported_not_looped_forever() {
        let mut e = engine_with(
            &[],
            "/gifs/Consoles/NES_Missing_01.gif",
            DwellPolicy::default(),
        );
        let mut canvas = Canvas::new();
        assert_eq!(
            e.tick(Duration::ZERO, &mut canvas).unwrap(),
            TickOutcome::Exhausted
        );
    }

    #[test]
    fn skip_pause_and_resume() {
        let mut e = engine_with(
            &[
                ("/gifs/Consoles/NES_A_01.gif", one_pixel_gif(255, 0, 0)),
                ("/gifs/Consoles/NES_B_01.gif", one_pixel_gif(0, 255, 0)),
            ],
            "/gifs/Consoles/NES_A_01.gif\n/gifs/Consoles/NES_B_01.gif",
            DwellPolicy::default(),
        );
        let t0 = Duration::ZERO;
        let mut canvas = Canvas::new();
        e.tick(t0, &mut canvas).unwrap();
        assert_eq!(e.now_playing().unwrap().title, "A");

        e.skip();
        e.tick(t0, &mut canvas).unwrap();
        assert_eq!(e.now_playing().unwrap().title, "B");

        e.pause(t0);
        assert!(e.is_paused());
        assert_eq!(
            e.tick(t0 + Duration::from_secs(60), &mut canvas).unwrap(),
            TickOutcome::Idle
        );
        // The paused minute did not count against the dwell.
        e.resume(t0 + Duration::from_secs(60));
        assert!(!e.is_paused());
        assert_eq!(e.now_playing().unwrap().title, "B");
    }

    #[test]
    fn drop_current_frees_the_file_and_the_next_tick_opens_the_next() {
        let mut e = engine_with(
            &[
                ("/gifs/Consoles/NES_A_01.gif", one_pixel_gif(255, 0, 0)),
                ("/gifs/Consoles/NES_B_01.gif", one_pixel_gif(0, 255, 0)),
            ],
            "/gifs/Consoles/NES_A_01.gif\n/gifs/Consoles/NES_B_01.gif",
            DwellPolicy::default(),
        );
        let t0 = Duration::ZERO;
        let mut canvas = Canvas::new();
        e.tick(t0, &mut canvas).unwrap();
        assert_eq!(e.now_playing().unwrap().title, "A");
        // Night mode: paused, then the file let go of.
        e.pause(t0);
        e.drop_current(&mut canvas);
        assert!(e.now_playing().is_none());
        assert!(e.is_paused());
        assert_eq!(e.tick(t0, &mut canvas).unwrap(), TickOutcome::Idle);
        e.resume(t0 + Duration::from_secs(60));
        e.tick(t0 + Duration::from_secs(60), &mut canvas).unwrap();
        let now = e.now_playing().unwrap();
        assert_eq!((now.title.as_str(), now.ordinal), ("B", 2));
    }

    /// A full-screen one-frame GIF of one colour, so a transition changes
    /// every pixel.
    fn solid_gif(r: u8, g: u8, b: u8) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut enc = Encoder::new(&mut out, 128, 32, &[]).unwrap();
            enc.set_repeat(Repeat::Infinite).unwrap();
            let mut f = Frame {
                width: 128,
                height: 32,
                buffer: Cow::Owned(vec![1u8; 128 * 32]),
                dispose: DisposalMethod::Keep,
                delay: 5,
                ..Default::default()
            };
            f.palette = Some(vec![0, 0, 0, r, g, b]);
            enc.write_frame(&f).unwrap();
        }
        out
    }

    fn two_solids(transition: Transition) -> Engine {
        let mut e = engine_with(
            &[
                ("/gifs/Consoles/NES_A_01.gif", solid_gif(255, 0, 0)),
                ("/gifs/Consoles/NES_B_01.gif", solid_gif(0, 0, 255)),
            ],
            "/gifs/Consoles/NES_A_01.gif\n/gifs/Consoles/NES_B_01.gif",
            DwellPolicy {
                min: Duration::from_secs(1),
                max: Duration::from_secs(30),
                min_loops: 1,
            },
        );
        e.set_transition(transition);
        e
    }

    #[test]
    fn a_new_file_transitions_in_and_ends_on_its_own_frame() {
        use crate::compositor::{Kind, STEP};
        const RED: Rgb = Rgb::new(255, 0, 0);
        const BLUE: Rgb = Rgb::new(0, 0, 255);
        let ms = Duration::from_millis;
        let mut e = two_solids(Transition::new(Kind::WipeRight, ms(400)));
        let mut canvas = Canvas::new();
        // The first file after boot cuts in: nothing to transition from.
        let TickOutcome::Frame { transitioning, .. } = e.tick(ms(0), &mut canvas).unwrap() else {
            panic!("A");
        };
        assert!(!transitioning && !e.is_transitioning());
        assert_eq!(e.display(&canvas).get(5, 5), RED);
        // A loops (a rewind, not a new file): no transition.
        let TickOutcome::Frame { transitioning, .. } = e.tick(ms(50), &mut canvas).unwrap() else {
            panic!("A again");
        };
        assert!(!transitioning);
        // Past A's dwell: B starts, and the picture wipes from red to blue.
        let t0 = ms(1000);
        let mut t = t0;
        let mut steps = 0;
        loop {
            let TickOutcome::Frame {
                playing,
                started,
                next_frame_at,
                transitioning,
            } = e.tick(t, &mut canvas).unwrap()
            else {
                panic!("expected a frame at {t:?}");
            };
            assert_eq!(playing.title, "B");
            assert_eq!(started, steps == 0);
            // The decoder's canvas is B's frame throughout; never drawn on.
            assert!(canvas.as_slice().iter().all(|p| *p == BLUE));
            let shown = e.display(&canvas);
            if !transitioning {
                assert_eq!(t - t0, ms(400), "ends exactly at the duration");
                assert!(shown.as_slice().iter().all(|p| *p == BLUE));
                assert!(!e.is_transitioning());
                break;
            }
            steps += 1;
            let blue = shown.as_slice().iter().filter(|p| **p == BLUE).count();
            let red = shown.as_slice().iter().filter(|p| **p == RED).count();
            // The wipe's front moves right, never back.
            assert!(
                red + blue >= 4096 - 2 * 32,
                "{red} red {blue} blue at {:?}",
                t - t0
            );
            assert_eq!(shown.get(0, 3) == BLUE, t > t0, "{:?}", t - t0);
            // A step 20 ms on (the last at the end), or B's own next frame
            // (50 ms) when that comes within half a step of it.
            let step = (t + STEP).min(t0 + ms(400));
            let due = e.frame_due().unwrap();
            let want = if step + STEP / 2 < due { step } else { due };
            assert_eq!(next_frame_at, want, "at {:?}", t - t0);
            assert!(next_frame_at - t <= ms(30));
            t = next_frame_at;
        }
        // Every 20-30 ms: at 0, 20, 50, 70, 100, ... 370 (B's frames at 50k).
        assert_eq!(steps, 16);
        // Afterwards B's own timing again (50 ms frames, looping).
        let TickOutcome::Frame { next_frame_at, .. } = e.tick(ms(1450), &mut canvas).unwrap()
        else {
            panic!("B loops");
        };
        assert_eq!(next_frame_at, ms(1500));
    }

    #[test]
    fn carry_over_skip_mid_transition_pause_and_none() {
        use crate::compositor::Kind;
        let ms = Duration::from_millis;
        // A replacement engine whose canvas already shows something.
        let mut e = two_solids(Transition::new(Kind::Dissolve, ms(200)));
        let mut canvas = Canvas::new();
        canvas.fill(Rgb::new(9, 9, 9));
        e.carry_over();
        let TickOutcome::Frame { transitioning, .. } = e.tick(ms(0), &mut canvas).unwrap() else {
            panic!("A");
        };
        assert!(
            transitioning,
            "the first file transitions from the carried-over picture"
        );
        assert_eq!(e.display(&canvas).get(0, 0), Rgb::new(9, 9, 9));
        // Paused halfway for a minute: nothing moves, and it resumes where it was.
        e.tick(ms(100), &mut canvas).unwrap();
        let half = e.display(&canvas).clone();
        e.pause(ms(100));
        assert_eq!(e.tick(ms(30_000), &mut canvas).unwrap(), TickOutcome::Idle);
        e.resume(ms(60_100));
        let TickOutcome::Frame { transitioning, .. } = e.tick(ms(60_150), &mut canvas).unwrap()
        else {
            panic!("resumed");
        };
        assert!(transitioning);
        let later = e.display(&canvas).clone();
        let moved = later
            .as_slice()
            .iter()
            .zip(half.as_slice())
            .filter(|(a, b)| a != b)
            .count();
        assert!(
            (600..2000).contains(&moved),
            "{moved} px moved in 50 ms of a 200 ms dissolve"
        );
        // Skip halfway: B starts from the half-dissolved picture.
        e.skip();
        e.tick(ms(60_160), &mut canvas).unwrap();
        assert!(e.is_transitioning());
        assert_eq!(e.display(&canvas), &later);
        // finish_transition hands the canvas back with what the panel owes.
        canvas.dirty_mut().clear();
        e.finish_transition(&mut canvas);
        assert!(!e.is_transitioning());
        assert!(canvas.dirty().count() > 2000);
        assert_eq!(e.display(&canvas), &canvas);

        // None: straight cuts, no extra frames.
        let mut e = two_solids(Transition::NONE);
        let mut canvas = Canvas::new();
        e.tick(ms(0), &mut canvas).unwrap();
        let TickOutcome::Frame {
            playing,
            transitioning,
            next_frame_at,
            ..
        } = e.tick(ms(1000), &mut canvas).unwrap()
        else {
            panic!("B");
        };
        assert_eq!((playing.title.as_str(), transitioning), ("B", false));
        assert_eq!(next_frame_at, ms(1050));
        assert_eq!(e.display(&canvas).get(0, 0), Rgb::new(0, 0, 255));
    }

    #[test]
    fn a_seek_starts_the_transition_and_step_transition_runs_it() {
        use crate::compositor::Kind;
        let ms = Duration::from_millis;
        let path = "/gifs/Consoles/NES_Three_01.gif";
        let mut e = engine_with(&[(path, three_frame_gif())], path, DwellPolicy::default());
        e.set_transition(Transition::new(Kind::Fade, ms(100)));
        e.carry_over();
        let mut canvas = Canvas::new();
        canvas.fill(Rgb::new(200, 200, 200));
        e.seek_elapsed(ms(150), &mut canvas).unwrap();
        assert!(e.is_transitioning());
        // Not shown yet: its clock starts at the first step.
        assert!(e.step_transition(ms(5000), &mut canvas));
        assert_eq!(e.display(&canvas).get(9, 9), Rgb::new(200, 200, 200));
        assert!(!e.step_transition(ms(5010), &mut canvas), "not due");
        assert!(e.step_transition(ms(5025), &mut canvas));
        assert!(e.step_transition(ms(5050), &mut canvas));
        assert_eq!(e.display(&canvas).get(9, 9), Rgb::BLACK, "the dip");
        assert!(e.step_transition(ms(5090), &mut canvas));
        assert!(e.is_transitioning());
        assert!(e.step_transition(ms(5100), &mut canvas));
        assert!(!e.is_transitioning());
        assert_eq!(e.display(&canvas).get(0, 0), Rgb::new(0, 255, 0));
    }

    const MANIFEST: &str = "/gifs/Consoles/SNES_Plok_08.gif\n\
                            \n\
                            /gifs/Arcade/ARCADE_Pac-Man_01.gif\r\n\
                            /gifs/Consoles/MD_Sonic3_RattenJager.gif\n";

    #[test]
    fn file_catalog_indexes_and_reads_back() {
        let mut cat = catalog(MANIFEST, &Filter::new());
        assert_eq!(cat.len(), 3);
        assert_eq!(cat.offsets().len(), 3);
        assert_eq!(cat.entry(0).unwrap().title, "Plok");
        assert_eq!(cat.entry(2).unwrap().title, "Sonic3");
        // Out of order and repeated: the reader is seeked, not consumed.
        assert_eq!(cat.entry(1).unwrap().platform.as_deref(), Some("ARCADE"));
        assert_eq!(
            cat.entry(0).unwrap().path,
            "/gifs/Consoles/SNES_Plok_08.gif"
        );
        assert!(cat.entry(3).is_err());
    }

    #[test]
    fn file_catalog_applies_the_filter_and_reports_the_pass() {
        let filter = Filter::new().platforms(["SNES", "MD"]);
        let (mut total, mut kept) = (0usize, 0usize);
        let mut cat = FileCatalog::build_with(
            Cursor::new(MANIFEST.as_bytes().to_vec()),
            &filter,
            |_, matched| {
                total += 1;
                kept += usize::from(matched);
            },
        )
        .unwrap();
        assert_eq!((total, kept), (3, 2));
        assert_eq!(cat.len(), 2);
        assert_eq!(cat.entry(0).unwrap().title, "Plok");
        assert_eq!(cat.entry(1).unwrap().title, "Sonic3");
    }

    #[test]
    fn reset_reseeds_without_touching_the_catalog() {
        let mut e = engine_with(
            &[
                ("/gifs/Consoles/NES_A_01.gif", one_pixel_gif(255, 0, 0)),
                ("/gifs/Consoles/NES_B_01.gif", one_pixel_gif(0, 255, 0)),
            ],
            "/gifs/Consoles/NES_A_01.gif\n/gifs/Consoles/NES_B_01.gif",
            DwellPolicy::default(),
        );
        let t0 = Duration::ZERO;
        let mut canvas = Canvas::new();
        e.tick(t0, &mut canvas).unwrap();
        e.skip();
        e.tick(t0, &mut canvas).unwrap();
        assert_eq!(e.now_playing().unwrap().title, "B");

        e.reset(0);
        assert!(e.now_playing().is_none());
        assert_eq!(e.catalog_len(), 2);
        // Sequential + rewound cursor: back to the top of the playlist.
        e.tick(t0, &mut canvas).unwrap();
        assert_eq!(e.now_playing().unwrap().title, "A");
    }

    #[test]
    fn virtual_files_play_without_the_source() {
        let dwell = DwellPolicy {
            min: Duration::from_secs(3),
            max: Duration::from_secs(30),
            min_loops: 1,
        };
        let mut e = engine_with(
            &[("/gifs/Consoles/NES_A_01.gif", one_pixel_gif(255, 0, 0))],
            "/gifs/Consoles/NES_A_01.gif",
            dwell,
        );
        let mut canvas = Canvas::new();
        e.tick(Duration::ZERO, &mut canvas).unwrap();
        let once = Override {
            loops: Some(1),
            ..Override::default()
        };
        let card = synth::card_path("Sonic the Hedgehog 3", "Mega Drive");
        e.play_now_with(&card, Duration::from_millis(10), Some(once))
            .unwrap();
        let TickOutcome::Frame { playing, .. } =
            e.tick(Duration::from_millis(10), &mut canvas).unwrap()
        else {
            panic!("expected the card");
        };
        assert_eq!(playing.title, "Sonic the Hedgehog 3");
        assert_eq!(playing.path, card);
        // `loops=1`: one pass of its 3 s frame, then the playlist (not the
        // configured 3 s minimum *after* it).
        let t = Duration::from_millis(10) + synth::CARD_DWELL;
        match e.tick(t, &mut canvas).unwrap() {
            TickOutcome::Frame { playing, .. } => assert_eq!(playing.title, "A"),
            other => panic!("expected A, got {other:?}"),
        }
        // The power-on effect: 35 frames, then the playlist.
        e.play_now_with(synth::POWER_ON, t, Some(once)).unwrap();
        let mut frames = 0;
        let mut now = t;
        loop {
            match e.tick(now, &mut canvas).unwrap() {
                TickOutcome::Frame { playing, .. } if playing.path == synth::POWER_ON => {
                    frames += 1
                }
                TickOutcome::Frame { .. } => break,
                _ => {}
            }
            now += Duration::from_millis(5);
        }
        assert_eq!(frames, synth::POWER_ON_FRAMES);
        assert!(now - t >= Duration::from_millis(700) && now - t < Duration::from_millis(760));
    }

    #[test]
    fn hold_keeps_the_last_frame_until_released_or_replaced() {
        let dwell = DwellPolicy {
            min: Duration::ZERO,
            max: Duration::from_secs(30),
            min_loops: 1,
        };
        let mut e = engine_with(
            &[
                ("/gifs/Consoles/NES_A_01.gif", one_pixel_gif(255, 0, 0)),
                ("/gifs/Consoles/NES_B_01.gif", one_pixel_gif(0, 255, 0)),
            ],
            "/gifs/Consoles/NES_A_01.gif\n/gifs/Consoles/NES_B_01.gif",
            dwell,
        );
        let mut canvas = Canvas::new();
        e.set_hold(true);
        e.tick(Duration::ZERO, &mut canvas).unwrap();
        assert_eq!(e.now_playing().unwrap().title, "A");
        assert!(!e.is_waiting());
        // A's 50 ms loop is its whole dwell: held, not B.
        for ms in [50, 60, 500, 5_000] {
            assert_eq!(
                e.tick(Duration::from_millis(ms), &mut canvas).unwrap(),
                TickOutcome::Idle
            );
            assert!(e.is_waiting());
            assert_eq!(e.now_playing().unwrap().title, "A");
        }
        assert_eq!(canvas.get(0, 0), Rgb::new(255, 0, 0));
        // Replaced: the next file plays, and is held in its turn.
        let once = Override {
            loops: Some(1),
            ..Override::default()
        };
        e.play_now_with(synth::POWER_ON, Duration::from_millis(5_000), Some(once))
            .unwrap();
        assert!(!e.is_waiting());
        let mut now = Duration::from_millis(5_000);
        while !e.is_waiting() {
            e.tick(now, &mut canvas).unwrap();
            now += Duration::from_millis(20);
            assert!(now < Duration::from_secs(10));
        }
        assert_eq!(e.now_playing().unwrap().path, synth::POWER_ON);
        // Released: the scheduler moves on (to B: A's turn was used).
        e.set_hold(false);
        match e.tick(now, &mut canvas).unwrap() {
            TickOutcome::Frame { playing, .. } => assert_eq!(playing.title, "B"),
            other => panic!("expected B, got {other:?}"),
        }
        // The hard cap holds too.
        e.set_hold(true);
        let mut e2 = engine_with(
            &[("/gifs/Consoles/NES_A_01.gif", one_pixel_gif(255, 0, 0))],
            "/gifs/Consoles/NES_A_01.gif",
            DwellPolicy {
                min: Duration::from_secs(60),
                max: Duration::from_secs(1),
                min_loops: 1,
            },
        );
        e2.set_hold(true);
        e2.tick(Duration::ZERO, &mut canvas).unwrap();
        e2.tick(Duration::from_millis(1_100), &mut canvas).unwrap();
        assert!(e2.is_waiting());
    }
}

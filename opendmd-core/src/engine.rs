//! The playlist engine: picks entries, opens them, advances frames, decides
//! when a GIF has had its turn.
//!
//! The engine never sleeps and never reads the clock. The caller (simulator or
//! firmware main loop) passes `now` into [`Engine::tick`] and owns the timing,
//! so the same code drives a 60 Hz desktop loop and a HUB75 refresh loop.

use crate::canvas::Canvas;
use crate::filter::Filter;
use crate::manifest::{self, Entry};
use crate::player::GifSource;
use crate::scheduler::Scheduler;
use std::io::{self, BufReader, Read, Seek};
use std::time::{Duration, Instant};

/// A playlist the engine can look entries up in by index.
///
/// The engine only ever asks for an entry when it is about to open the next
/// GIF, so an implementation is free to hold nothing but an index and go back
/// to the card for the rest — which is exactly what [`FileCatalog`] does.
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
/// This is what makes a 10,133-entry card playable on an ESP32. The index is
/// 4 bytes per entry (~40 KB for the whole card) against the ~590 bytes per
/// entry a `Vec<Entry>` costs, and the only other live allocation is one
/// [`Entry`] at a time inside [`Engine::tick`].
///
/// `R` is `std::fs::File` on both the firmware (`/sdcard/filelist.txt`, one
/// handle kept open for the life of the playlist) and the simulator, and
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

/// Where GIF bytes come from. The firmware implements this over `/sdcard`, the
/// simulator over a directory. Manifest paths are card-absolute
/// (`/gifs/Consoles/X.gif`); joining them onto a root is the source's job.
pub trait Source {
    fn open(&self, path: &str) -> io::Result<Box<dyn Read>>;
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
    /// Index into the engine's entry slice.
    pub index: usize,
    pub path: String,
    pub title: String,
    /// 1-based count of GIFs started since the engine was created.
    pub ordinal: u64,
}

/// Result of one [`Engine::tick`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TickOutcome {
    /// Nothing to do: paused, or the current frame's delay hasn't elapsed.
    /// The canvas was not touched.
    Idle,
    /// The canvas was updated.
    Frame {
        playing: NowPlaying,
        /// True on the first frame of a newly started GIF.
        started: bool,
        /// When the caller should tick again for the next frame.
        next_frame_at: Instant,
    },
    /// Nothing is playable — empty playlist, or every candidate failed to open.
    Exhausted,
}

struct Playing {
    index: usize,
    path: String,
    title: String,
    ordinal: u64,
    gif: GifSource<Box<dyn Read>>,
    started_at: Instant,
    next_frame_at: Instant,
    loops: u32,
    first_frame: bool,
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
    source: Box<dyn Source>,
    current: Option<Playing>,
    paused_since: Option<Instant>,
    started_count: u64,
    skip_requested: bool,
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
            source,
            current: None,
            paused_since: None,
            started_count: 0,
            skip_requested: false,
        }
    }

    /// How many entries survived the filter, i.e. the playlist length.
    pub fn catalog_len(&self) -> usize {
        self.catalog.len()
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

    pub fn now_playing(&self) -> Option<NowPlaying> {
        self.current.as_ref().map(|p| NowPlaying {
            index: p.index,
            path: p.path.clone(),
            title: p.title.clone(),
            ordinal: p.ordinal,
        })
    }

    /// Finish the current GIF at the next tick.
    pub fn skip(&mut self) {
        self.skip_requested = true;
    }

    pub fn is_paused(&self) -> bool {
        self.paused_since.is_some()
    }

    pub fn pause(&mut self, now: Instant) {
        if self.paused_since.is_none() {
            self.paused_since = Some(now);
        }
    }

    /// Resuming shifts the current GIF's clocks forward by the paused duration,
    /// so a pause doesn't eat its dwell time.
    pub fn resume(&mut self, now: Instant) {
        if let Some(since) = self.paused_since.take() {
            let paused_for = now.saturating_duration_since(since);
            if let Some(p) = &mut self.current {
                p.started_at += paused_for;
                p.next_frame_at += paused_for;
            }
        }
    }

    pub fn toggle_pause(&mut self, now: Instant) {
        if self.is_paused() {
            self.resume(now);
        } else {
            self.pause(now);
        }
    }

    /// Advance playback. Returns whether the canvas changed and what is on it.
    pub fn tick(&mut self, now: Instant, canvas: &mut Canvas) -> io::Result<TickOutcome> {
        if self.is_paused() {
            return Ok(TickOutcome::Idle);
        }

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
            if now < playing.next_frame_at {
                return Ok(TickOutcome::Idle);
            }

            // Hard cap: never let one GIF hold the panel past `max`.
            if now.saturating_duration_since(playing.started_at) >= self.dwell.max {
                self.current = None;
                if !self.open_next(now) {
                    return Ok(TickOutcome::Exhausted);
                }
                continue;
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
                    return Ok(TickOutcome::Frame {
                        playing: NowPlaying {
                            index: playing.index,
                            path: playing.path.clone(),
                            title: playing.title.clone(),
                            ordinal: playing.ordinal,
                        },
                        started,
                        next_frame_at: playing.next_frame_at,
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
                    let elapsed = now.saturating_duration_since(playing.started_at);
                    let done = playing.loops >= self.dwell.min_loops && elapsed >= self.dwell.min;
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

    /// Re-open the current file from the start (no rewind on a streaming
    /// reader). False if it can't be reopened, in which case we move on.
    fn restart_current(&mut self) -> bool {
        let Some(playing) = &mut self.current else {
            return false;
        };
        match self
            .source
            .open(&playing.path)
            .and_then(GifSource::<Box<dyn Read>>::new)
        {
            Ok(gif) => {
                playing.gif = gif;
                true
            }
            Err(e) => {
                log::warn!("reopen failed for {}: {e}", playing.path);
                false
            }
        }
    }

    /// Pull entries from the scheduler until one opens. A GIF that won't open
    /// or won't decode its header is logged and skipped, never fatal.
    fn open_next(&mut self, now: Instant) -> bool {
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
            match self
                .source
                .open(&path)
                .and_then(GifSource::<Box<dyn Read>>::new)
            {
                Ok(gif) => {
                    self.started_count += 1;
                    self.current = Some(Playing {
                        index,
                        path,
                        title,
                        ordinal: self.started_count,
                        gif,
                        started_at: now,
                        next_frame_at: now,
                        loops: 0,
                        first_frame: true,
                    });
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
        fn open(&self, path: &str) -> io::Result<Box<dyn Read>> {
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

        let t0 = Instant::now();
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
        let t0 = Instant::now();
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
        let t0 = Instant::now();
        let mut canvas = Canvas::new();
        e.tick(t0, &mut canvas).unwrap();
        match e.tick(t0 + Duration::from_secs(6), &mut canvas).unwrap() {
            TickOutcome::Frame { playing, .. } => assert_eq!(playing.title, "B"),
            other => panic!("expected B, got {other:?}"),
        }
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
        let t0 = Instant::now();
        match e.tick(t0, &mut canvas).unwrap() {
            TickOutcome::Frame { playing, .. } => assert_eq!(playing.title, "Good"),
            other => panic!("expected Good, got {other:?}"),
        }
        assert!(
            t0.elapsed() < Duration::from_secs(1),
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
        match e.tick(Instant::now(), &mut canvas).unwrap() {
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
            e.tick(Instant::now(), &mut canvas).unwrap(),
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
        let t0 = Instant::now();
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
        let t0 = Instant::now();
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
}

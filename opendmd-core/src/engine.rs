//! The playlist engine: picks entries, opens them, advances frames, decides
//! when a GIF has had its turn.
//!
//! The engine never sleeps and never reads the clock. The caller (simulator or
//! firmware main loop) passes `now` into [`Engine::tick`] and owns the timing,
//! so the same code drives a 60 Hz desktop loop and a HUB75 refresh loop.

use crate::canvas::Canvas;
use crate::filter::Filter;
use crate::manifest::Entry;
use crate::player::GifSource;
use crate::scheduler::Scheduler;
use std::io::{self, Read};
use std::time::{Duration, Instant};

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

/// Drives a filtered, scheduled playlist over a [`Source`].
pub struct Engine {
    entries: Vec<Entry>,
    playable: Vec<usize>,
    scheduler: Scheduler,
    dwell: DwellPolicy,
    source: Box<dyn Source>,
    current: Option<Playing>,
    paused_since: Option<Instant>,
    started_count: u64,
    skip_requested: bool,
}

impl Engine {
    /// `scheduler` should already be built over `filter.apply(&entries)`; the
    /// filter is kept for reporting (`playable()`), not re-applied per tick.
    pub fn new(
        entries: Vec<Entry>,
        filter: &Filter,
        scheduler: Scheduler,
        dwell: DwellPolicy,
        source: Box<dyn Source>,
    ) -> Self {
        let playable = filter.apply(&entries);
        Engine {
            entries,
            playable,
            scheduler,
            dwell,
            source,
            current: None,
            paused_since: None,
            started_count: 0,
            skip_requested: false,
        }
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// Indices that survived the filter, in manifest order.
    pub fn playable(&self) -> &[usize] {
        &self.playable
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
            let Some(entry) = self.entries.get(index) else {
                continue;
            };
            let path = entry.path.clone();
            let title = entry.title.clone();
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
    use crate::manifest::parse_manifest;
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

    fn engine_with(files: &[(&str, Vec<u8>)], manifest: &str, dwell: DwellPolicy) -> Engine {
        let entries = parse_manifest(manifest);
        let filter = Filter::new();
        let idx = filter.apply(&entries);
        let sched = Scheduler::new(&idx, Mode::Sequential, 0);
        let src = MemSource(
            files
                .iter()
                .map(|(p, b)| ((*p).to_string(), b.clone()))
                .collect(),
        );
        Engine::new(entries, &filter, sched, dwell, Box::new(src))
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
        assert_eq!(e.playable().len(), 2);

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
}

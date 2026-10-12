//! Where a deadline miss came from, always on: a few clock reads per frame.
//!
//! The display loop splits every iteration into phases and, when a frame is
//! late, counts the miss against the largest one (`Cause`); it also keeps
//! a histogram of how late (`LATE_BUCKETS`), and the headline: misses of
//! 5 ms or more, steady or at a GIF change (`opendmd_core::frametime`, which
//! holds the counters). All of it goes in the heartbeat and in
//! `/api/status` (`misses: {steady, switch, …, by_cause, late_ms}`). The
//! phases:
//!
//! - `open`: [`crate::sd::SdSource::open`] (the copy lookup, the file's own
//!   open and its magic check, the builder's release wait) and the
//!   playlist lookup before it ([`CountedCatalog`]); also an open done by a
//!   command (`POST /api/play`) at the top of the loop;
//! - `read`: card reads and seeks of the playing file ([`Counted`]);
//! - `decode`: the engine's tick minus the two above (`transition` instead
//!   while a transition runs: the composite and the new file's frames);
//! - `blit`: pushing the picture to the panel's DMA buffer;
//! - `log`: the heartbeat and the loop's own log lines;
//! - `other`: the rest of the time between the frame's sleep and the tick
//!   (commands, the schedule, Follow RePlayOS's runner, status);
//! - `wake`: the frame's sleep ending later than asked, plus what the loop
//!   did between judging a frame and going to sleep (preemption on CPU1).
//!
//! The per-iteration I/O counters are statics touched only by the main task
//! (the player's file and catalog are only ever read there), so `Relaxed`
//! is plenty.
//!
//! Also here: [`Floor`], the sampled heap floor with a hint of who was
//! running when it was reached.

use opendmd_core::engine::Catalog;
use opendmd_core::manifest::Entry;
use std::io::{self, Read, Seek, SeekFrom};
use std::sync::atomic::{AtomicU32, Ordering::Relaxed};
use std::time::Instant;

use crate::state::{free_heap, largest_free_block};

static OPEN_US: AtomicU32 = AtomicU32::new(0);
static READ_US: AtomicU32 = AtomicU32::new(0);

fn us_since(t: Instant) -> u32 {
    t.elapsed().as_micros().min(u32::MAX as u128) as u32
}

/// Card time since the last [`take`]: `(open µs, read µs)`, and reset.
pub fn take() -> (u32, u32) {
    (OPEN_US.swap(0, Relaxed), READ_US.swap(0, Relaxed))
}

/// Run `f` as an open: all of its time is open time, the reads it makes
/// (a copy's magic check) included.
pub fn open<T>(f: impl FnOnce() -> T) -> T {
    let t = Instant::now();
    let reads = READ_US.load(Relaxed);
    let r = f();
    READ_US.store(reads, Relaxed);
    OPEN_US.fetch_add(us_since(t), Relaxed);
    r
}

/// The player's file, its reads and seeks timed into the iteration's `read`.
///
/// Also the player's half of card-failure counting ([`crate::sdhealth`]):
/// a card-class error is recorded, *proven* (the card, not the file) when
/// this handle had already read (`.1`). The flag fits the `Box`'s minimum
/// heap block, so it costs nothing; so does `.2`, the copy's key when it
/// was opened ahead for the next file ([`crate::state::AHEAD_KEY`], cleared
/// when this handle closes; 0 otherwise).
pub struct Counted<R>(pub R, pub bool, pub u32);

impl<R> Drop for Counted<R> {
    fn drop(&mut self) {
        if self.2 != 0 {
            let _ = crate::state::AHEAD_KEY.compare_exchange(
                self.2,
                0,
                std::sync::atomic::Ordering::Relaxed,
                std::sync::atomic::Ordering::Relaxed,
            );
        }
    }
}

impl<R: Read> Read for Counted<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let t = Instant::now();
        // The I/O test hook fails only reads of a file that has read fine,
        // as a card failing under a playing GIF does.
        let sim = if self.1 {
            crate::sdhealth::take_sim_read()
        } else {
            None
        };
        let r = match sim {
            Some(e) => Err(e),
            None => self.0.read(buf),
        };
        READ_US.fetch_add(us_since(t), Relaxed);
        match &r {
            Ok(n) => self.1 |= *n > 0,
            Err(e) => crate::sdhealth::read_failed(
                crate::sdhealth::Kind::Read,
                self.1,
                "the playing file",
                e,
            ),
        }
        r
    }
}

impl<R: Seek> Seek for Counted<R> {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let t = Instant::now();
        let r = self.0.seek(pos);
        READ_US.fetch_add(us_since(t), Relaxed);
        if let Err(e) = &r {
            crate::sdhealth::read_failed(
                crate::sdhealth::Kind::Read,
                self.1,
                "the playing file (seek)",
                e,
            );
        }
        r
    }
}

/// The playlist lookup (an index seek and a manifest line) timed as `open`:
/// it is the card work of choosing the next file. Its card errors count as
/// reads, proven once a lookup has succeeded (`.1`).
pub struct CountedCatalog(pub Box<dyn Catalog>, pub bool);

impl Catalog for CountedCatalog {
    fn len(&self) -> usize {
        self.0.len()
    }

    fn entry(&mut self, i: usize) -> io::Result<Entry> {
        let r = open(|| self.0.entry(i));
        match &r {
            Ok(_) => self.1 = true,
            Err(e) => {
                crate::sdhealth::read_failed(crate::sdhealth::Kind::Read, self.1, "the playlist", e)
            }
        }
        r
    }
}

// The counters and the steady/switch rule are host-tested in the core.
pub use opendmd_core::frametime::{when, Misses, Phases};

/// A FreeRTOS task name (≤ 15 bytes), copied.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct TaskName([u8; 16]);

impl TaskName {
    /// The task running on `core` right now.
    fn running_on(core: i32) -> TaskName {
        let mut out = [0u8; 16];
        // SAFETY: queries of the scheduler's current-task pointer and the
        // TCB's name, a NUL-terminated array inside the TCB; copied at once
        // (a task deleted meanwhile leaves a stale but readable array).
        unsafe {
            let h = esp_idf_sys::xTaskGetCurrentTaskHandleForCore(core);
            if !h.is_null() {
                let p = esp_idf_sys::pcTaskGetName(h);
                if !p.is_null() {
                    for (i, b) in out.iter_mut().take(15).enumerate() {
                        let c = *p.add(i) as u8;
                        if c == 0 {
                            break;
                        }
                        *b = c;
                    }
                }
            }
        }
        TaskName(out)
    }

    pub fn as_str(&self) -> &str {
        let n = self.0.iter().position(|&b| b == 0).unwrap_or(16);
        std::str::from_utf8(&self.0[..n]).unwrap_or("?")
    }
}

/// Who was running on each core, and the largest free block, the moment
/// the sampled floor was reached. A hint, not proof: the task that took the
/// heap down may have been preempted by then, and the main task (the one
/// sampling) is always one of the two.
#[derive(Clone, Copy, Default, Debug)]
pub struct FloorAt {
    pub task0: TaskName,
    pub task1: TaskName,
    pub largest: u32,
}

impl FloorAt {
    /// `{"task0":…,"task1":…,"largest":N}` (`/api/status` `heap_floor_at`).
    pub fn json(&self) -> String {
        format!(
            r#"{{"task0":{},"task1":{},"largest":{}}}"#,
            crate::httpd::json_string(self.task0.as_str()),
            crate::httpd::json_string(self.task1.as_str()),
            self.largest
        )
    }
}

/// The display loop's sampled heap floor: the lowest `free_heap()` seen at
/// its phase boundaries (see main.rs for why not `esp_get_minimum_free_heap_size`).
pub struct Floor {
    pub min: u32,
    pub at: Option<FloorAt>,
}

impl Floor {
    pub fn new() -> Floor {
        Floor {
            min: free_heap(),
            at: None,
        }
    }

    /// Sample now.
    pub fn sample(&mut self) {
        self.note(free_heap());
    }

    /// A free-heap reading taken anyway: a new low records who was running
    /// (two scheduler queries) and the largest block (a heap walk — only
    /// on a new low, which after the first minutes is rare).
    pub fn note(&mut self, free: u32) {
        if free < self.min {
            self.min = free;
            self.at = Some(FloorAt {
                task0: TaskName::running_on(0),
                task1: TaskName::running_on(1),
                largest: largest_free_block() as u32,
            });
        }
    }

    /// `at cpu0 <task>/cpu1 <task>, largest N B` for the heartbeat.
    pub fn heartbeat(&self) -> String {
        match &self.at {
            Some(a) => format!(
                " at cpu0 {} / cpu1 {}, largest {} B",
                a.task0.as_str(),
                a.task1.as_str(),
                a.largest
            ),
            None => String::new(),
        }
    }
}

/// The calling task's stack high-water mark: bytes never used.
pub fn stack_unused() -> u32 {
    // SAFETY: a query about the calling task.
    unsafe { esp_idf_sys::uxTaskGetStackHighWaterMark(std::ptr::null_mut()) }
}

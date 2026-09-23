//! The playback cache: prepared copies (`.odm`) of the card's GIFs, and how
//! many of the active playlist's entries have one.
//!
//! Users see GIFs and playlists; a prepared copy is a cache. `SdSource` plays
//! one when it exists (see [`crate::sd::SdSource`]), and this module answers
//! `GET /api/cache` (`{"total","prepared","state"}`) and streams
//! `GET /api/cache/list` (every copy's path, for a browser deciding what is
//! left to prepare).
//!
//! # The count
//!
//! `total` is the playlist length the engine already knows. `prepared` needs
//! the copies on the card intersected with the playlist, which is done without
//! ever holding either in memory:
//!
//! 1. **Discover** the directories under `odm/` that hold bucket directories
//!    (one per category; a listing of `odm/` and of each `odm/<Category>`,
//!    ≤256 short 8.3 entries each) and which of the 256 buckets exist in each.
//! 2. **Walk the buckets in order**, `00` to `ff`, reading each existing
//!    bucket directory (~15 files) and keeping a `u32`
//!    [`layout::prepared_key`] per copy, until
//!    [`config::CACHE_KEYS_PER_PASS`] keys are held.
//! 3. **One pass over the manifest** ([`layout::count_prepared`]) for that
//!    slice of buckets, then carry on with the next slice.
//!
//! One slice (and one manifest pass) covers a card with up to ~3,000 copies;
//! a fully prepared 11k-file card takes four. It runs on the network worker
//! (CPU0), never the display task, in FAT-lock-sized pieces with a yield
//! between directories, and restarts if the playlist or the copies change
//! underneath it. Nothing counts until someone asks: the first
//! `GET /api/cache` starts it, and after that every change (an upload to
//! `odm/`, a forget, a reindex or set change) recounts, debounced by
//! [`config::CACHE_DEBOUNCE`] so a browser uploading copies one after another
//! causes one count at the end.

use anyhow::Result;
use opendmd_core::filter::Filter;
use opendmd_core::layout::{self, CacheCount};
use std::fs;
use std::io::{self, BufReader, Read};
use std::sync::atomic::{AtomicU32, Ordering::Relaxed};
use std::sync::{mpsc, Mutex, OnceLock};
use std::thread;
use std::time::Instant;

use crate::config;
use crate::http::NetCommand;
use crate::httpd::{json_string, Req};
use crate::state::free_heap;

/// Bumped by every change that makes a count stale; a count in flight that
/// sees it move gives up, and the result of one that started under an older
/// value is not published.
static GEN: AtomicU32 = AtomicU32::new(0);
static STATE: Mutex<State> = Mutex::new(State::new());
/// Wakes the network worker. Set once in `main`.
static WAKE: OnceLock<Mutex<mpsc::Sender<NetCommand>>> = OnceLock::new();

/// What `/api/cache` reports, plus what the next count needs.
#[derive(Clone, Debug)]
pub struct State {
    /// The manifest the count is for (filesystem path) and the platform
    /// filter, as the engine was last built with.
    pub manifest: String,
    pub platforms: String,
    /// The last completed count, if it is for this manifest and filter.
    pub prepared: Option<usize>,
    /// When `prepared` was counted, the generation it was counted at.
    done_gen: Option<u32>,
    pub counting: bool,
    /// How long the last completed count took, end to end.
    pub count_ms: u64,
    /// Whether anyone has asked (`GET /api/cache`). Until then nothing counts.
    requested: bool,
}

impl State {
    const fn new() -> Self {
        State {
            manifest: String::new(),
            platforms: String::new(),
            prepared: None,
            done_gen: None,
            counting: false,
            count_ms: 0,
            requested: false,
        }
    }
}

/// Called once from `main`, with the network worker's inbox.
pub fn init(wake: mpsc::Sender<NetCommand>) {
    let _ = WAKE.set(Mutex::new(wake));
}

fn wake() {
    if let Some(tx) = WAKE.get() {
        let _ = tx.lock().unwrap().send(NetCommand::CountCache);
    }
}

/// The playlist the engine now plays (main task, after every rebuild). A
/// different manifest or filter forgets the old count.
pub fn set_playlist(manifest: &str, platforms: &str) {
    {
        let mut s = STATE.lock().unwrap();
        if s.manifest != manifest || s.platforms != platforms {
            s.manifest = manifest.to_string();
            s.platforms = platforms.to_string();
            s.prepared = None;
        }
    }
    // A rebuild of the same playlist (a reindex) recounts too: the caller
    // may know the card changed underneath it.
    invalidate();
}

/// The copies (or the playlist) changed: the count is stale. Recount now if
/// anyone has asked for one before; otherwise wait until someone does.
pub fn invalidate() {
    GEN.fetch_add(1, Relaxed);
    let start = {
        let mut s = STATE.lock().unwrap();
        if s.requested {
            s.counting = true;
        }
        s.requested
    };
    if start {
        wake();
    }
}

/// `GET /api/cache`: the current state, starting a count if it is stale and
/// none is running.
pub fn snapshot() -> State {
    let start = {
        let mut s = STATE.lock().unwrap();
        s.requested = true;
        let stale = s.done_gen != Some(GEN.load(Relaxed));
        let start = stale && !s.counting;
        if start {
            s.counting = true;
        }
        start
    };
    if start {
        wake();
    }
    STATE.lock().unwrap().clone()
}

/// The network worker's side of [`NetCommand::CountCache`]: wait for the
/// requests to go quiet, count, publish. Returns early if the count is
/// already current (a queued duplicate).
pub fn run_count() {
    loop {
        // Debounce: start only once nothing has invalidated for a while.
        let gen = loop {
            let g = GEN.load(Relaxed);
            thread::sleep(config::CACHE_DEBOUNCE);
            if GEN.load(Relaxed) == g {
                break g;
            }
        };
        let (manifest, platforms, current) = {
            let s = STATE.lock().unwrap();
            (
                s.manifest.clone(),
                s.platforms.clone(),
                s.done_gen == Some(gen),
            )
        };
        if current {
            STATE.lock().unwrap().counting = false;
            return;
        }
        if manifest.is_empty() {
            // No playlist yet (boot, or a card with none). Nothing to count.
            let mut s = STATE.lock().unwrap();
            s.counting = false;
            return;
        }
        let started = Instant::now();
        match count(&manifest, &platforms, gen) {
            Ok(Some((c, report))) => {
                let ms = started.elapsed().as_millis() as u64;
                let mut s = STATE.lock().unwrap();
                if GEN.load(Relaxed) != gen || s.manifest != manifest {
                    drop(s);
                    continue;
                }
                s.prepared = Some(c.prepared);
                s.done_gen = Some(gen);
                s.count_ms = ms;
                s.counting = false;
                drop(s);
                log::info!(
                    "cache: {} of {} entries prepared in {manifest}, counted in {ms} ms ({report}), heap {} B free",
                    c.prepared,
                    c.total,
                    free_heap()
                );
                return;
            }
            // Superseded mid-count: go round again.
            Ok(None) => continue,
            Err(e) => {
                log::warn!("cache: counting {manifest} failed: {e:#}");
                let mut s = STATE.lock().unwrap();
                s.counting = false;
                s.done_gen = Some(gen);
                return;
            }
        }
    }
}

fn build_filter(platforms: &str) -> Filter {
    let list: Vec<&str> = platforms
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    if list.is_empty() {
        Filter::new()
    } else {
        Filter::new().platforms(list)
    }
}

/// A directory under `odm/` that holds bucket directories.
struct Parent {
    /// Filesystem path, e.g. `/sdcard/odm/Consoles`.
    fs: String,
    /// Card-absolute path, e.g. `/odm/Consoles`.
    card: String,
    /// Which of the 256 buckets exist under it.
    buckets: [u32; 8],
}

impl Parent {
    fn has(&self, b: u8) -> bool {
        self.buckets[usize::from(b / 32)] & (1 << (b % 32)) != 0
    }
}

/// `"9f"` -> `Some(0x9f)`: exactly two lowercase hex digits, the only names
/// `opendmd_core::layout::bucket` produces.
fn bucket_name(name: &str) -> Option<u8> {
    let b = name.as_bytes();
    let ok = |c: u8| c.is_ascii_digit() || (b'a'..=b'f').contains(&c);
    (b.len() == 2 && ok(b[0]) && ok(b[1]))
        .then(|| u8::from_str_radix(name, 16).ok())
        .flatten()
}

/// Step 1: every directory under `odm/` (depth ≤ 3, for nested categories)
/// that has bucket directories in it. A card with no `odm/` has none.
fn discover(fs_dir: &str, card_dir: &str, depth: u32, out: &mut Vec<Parent>) -> io::Result<u32> {
    let mut dirs = 1;
    let mut buckets = [0u32; 8];
    let mut subdirs = Vec::new();
    for (i, entry) in fs::read_dir(fs_dir)?.enumerate() {
        let Ok(entry) = entry else { continue };
        // `d_type`, not a `stat`: a stat re-scans the directory (see files.rs).
        if !entry.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        match bucket_name(&name) {
            Some(b) => buckets[usize::from(b / 32)] |= 1 << (b % 32),
            None if depth < 3 => subdirs.push(name),
            None => {}
        }
        if i % 32 == 31 {
            thread::yield_now();
        }
    }
    if buckets != [0; 8] {
        out.push(Parent {
            fs: fs_dir.to_string(),
            card: card_dir.to_string(),
            buckets,
        });
    }
    for name in subdirs {
        dirs += discover(
            &format!("{fs_dir}/{name}"),
            &format!("{card_dir}/{name}"),
            depth + 1,
            out,
        )?;
    }
    Ok(dirs)
}

fn discover_all() -> io::Result<(Vec<Parent>, u32)> {
    let mut parents = Vec::new();
    match discover(config::ODM_ROOT, "/odm", 0, &mut parents) {
        Ok(dirs) => Ok((parents, dirs)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok((parents, 0)),
        Err(e) => Err(e),
    }
}

/// Step 2 for one bucket: call `visit` with the card path of every `.odm` in
/// bucket `b` of every parent. Returns the number of directories read.
fn walk_bucket(
    parents: &[Parent],
    b: u8,
    mut visit: impl FnMut(&str) -> Result<()>,
) -> Result<u32> {
    let mut dirs = 0;
    for p in parents.iter().filter(|p| p.has(b)) {
        let fs_dir = format!("{}/{b:02x}", p.fs);
        let Ok(rd) = fs::read_dir(&fs_dir) else {
            continue;
        };
        dirs += 1;
        for entry in rd {
            let Ok(entry) = entry else { continue };
            let name = entry.file_name();
            let name = name.to_string_lossy();
            let is_odm = name.len() > 4 && name[name.len() - 4..].eq_ignore_ascii_case(".odm");
            if is_odm && !entry.file_type().is_ok_and(|t| t.is_dir()) {
                visit(&format!("{}/{b:02x}/{name}", p.card))?;
            }
        }
        thread::yield_now();
    }
    Ok(dirs)
}

/// Fails the manifest pass as soon as the count has been superseded.
///
/// Not with `ErrorKind::Interrupted`: `BufRead::read_line` *retries*
/// Interrupted reads, so a superseded pass used to spin on CPU0 forever —
/// allocating an error per turn, starving the idle task (task watchdog) and,
/// with it, the httpd and WiFi (found on the rig when a config change landed
/// mid-count).
struct Abortable<R> {
    inner: R,
    gen: u32,
}

impl<R: Read> Read for Abortable<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if GEN.load(Relaxed) != self.gen {
            return Err(io::Error::other("superseded"));
        }
        let n = self.inner.read(buf)?;
        thread::yield_now();
        Ok(n)
    }
}

/// Steps 1-3. `Ok(None)` if superseded (the generation moved) part way.
fn count(manifest: &str, platforms: &str, gen: u32) -> Result<Option<(CacheCount, String)>> {
    let filter = build_filter(platforms);
    let t = Instant::now();
    let (parents, mut dirs) = discover_all()?;
    let discover_ms = t.elapsed().as_millis();
    // What this heap can spare right now, within the configured cap.
    let spare =
        (free_heap() as usize).saturating_sub(config::HEAP_RESERVE as usize + 24 * 1024) / 4;
    let cap = config::CACHE_KEYS_PER_PASS.min(spare).max(256);
    let mut keys: Vec<u32> = Vec::with_capacity(cap);
    let mut total = CacheCount::default();
    let (mut walk_ms, mut pass_ms, mut passes, mut copies) = (0u128, 0u128, 0u32, 0usize);
    let mut lo: u16 = 0;
    while lo <= 255 {
        let tw = Instant::now();
        keys.clear();
        let mut hi = lo;
        loop {
            if GEN.load(Relaxed) != gen {
                return Ok(None);
            }
            dirs += walk_bucket(&parents, hi as u8, |path| {
                keys.push(layout::prepared_key(path));
                Ok(())
            })?;
            if hi == 255 || keys.len() >= cap {
                break;
            }
            hi += 1;
        }
        walk_ms += tw.elapsed().as_millis();
        copies += keys.len();
        keys.sort_unstable();
        // A slice with no copies in it needs no pass, except the first, which
        // is where `total` and the direct `/odm/` entries are counted.
        if keys.is_empty() && passes > 0 {
            lo = hi + 1;
            continue;
        }
        let tp = Instant::now();
        let file = fs::File::open(manifest)?;
        let reader = BufReader::with_capacity(1024, Abortable { inner: file, gen });
        let c =
            match layout::count_prepared(reader, &filter, (lo as u8, hi as u8), &keys, passes == 0)
            {
                Ok(c) => c,
                Err(_) if GEN.load(Relaxed) != gen => return Ok(None),
                Err(e) => return Err(e.into()),
            };
        pass_ms += tp.elapsed().as_millis();
        if passes == 0 {
            total.total = c.total;
        }
        total.prepared += c.prepared;
        passes += 1;
        lo = hi + 1;
    }
    let report = format!(
        "{copies} copies in {} categories; discover {discover_ms} ms, walk {walk_ms} ms over {dirs} dirs, \
         {passes} manifest pass(es) {pass_ms} ms, {cap} keys/pass",
        parents.len()
    );
    Ok(Some((total, report)))
}

/// `GET /api/cache/list`: every prepared copy on the card, streamed as
/// `{"paths":[...],"count":N}` in bucket order. Nothing is held but the
/// category list and a 2 KB output buffer; the walk is the same one the count
/// makes, on the httpd task.
pub fn stream_list(req: &mut Req) -> Result<()> {
    let (parents, _) = discover_all()?;
    req.header("Cache-Control", "no-cache")?;
    req.begin_chunked("200 OK", "application/json")?;
    let mut out = String::with_capacity(2304);
    out.push_str(r#"{"paths":["#);
    let mut n = 0usize;
    for b in 0..=255u8 {
        walk_bucket(&parents, b, |path| {
            if n > 0 {
                out.push(',');
            }
            n += 1;
            out.push_str(&json_string(path));
            if out.len() >= 2048 {
                req.chunk(out.as_bytes())?;
                out.clear();
            }
            Ok(())
        })?;
    }
    out.push_str(&format!(r#"],"count":{n}}}"#));
    req.chunk(out.as_bytes())?;
    req.finish()
}

/// `POST /api/cache/forget` and `DELETE /api/files/gifs/…`: remove the
/// prepared copy of a GIF, if there is one. `Ok(Some(path))` if one was
/// removed, `Ok(None)` if there was none (or `gif` has no prepared form).
pub fn forget(gif: &str) -> io::Result<Option<String>> {
    let Some(copy) = layout::bucketed_path(gif) else {
        return Ok(None);
    };
    match fs::remove_file(format!("{}{copy}", config::SD_MOUNT)) {
        Ok(()) => {
            log::info!("cache: forgot {copy} (for {gif})");
            invalidate();
            Ok(Some(copy))
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

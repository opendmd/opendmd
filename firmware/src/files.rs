//! Serving and receiving files on the shared card.
//!
//! The card is shared between the player (main task, CPU1) and the HTTP server
//! (httpd task, CPU0). FATFS itself is reentrant, so that is allowed, but the
//! rules that keep the display honest live here:
//!
//! - Everything moves in [`config::IO_CHUNK`] pieces with a `yield_now()`
//!   between them, so the FAT lock is never held across a long transfer and the
//!   player's next `open`/`read` is at most one chunk behind.
//! - An upload is streamed to `<dest>.part` and renamed into place, so a file
//!   the player might open is never half-written. FAT has no atomic replace, so
//!   the old file is removed first — which is exactly why a manifest swap is
//!   not done here but handed to the main task (see [`crate::state::Command`]).
//! - `..` is rejected rather than normalised, and paths are percent-decoded
//!   before that check, not after.

use anyhow::{bail, Result};
use std::fs;
use std::io::{Read, Write};
use std::path::Path;
use std::thread;
use std::time::Duration;

use crate::config;
use crate::httpd::{json_string, BodyError, Req, Respond, Upload, UploadAborted};
use crate::probe;
use crate::state::free_heap;

/// Refuse to start a transfer that the player cannot afford.
///
/// Returns `Ok(false)` having already sent a 503 (`Retry-After: 2`, before
/// any body) if the heap is below [`config::STREAM_START_HEAP`] — the
/// reserve plus one transfer's worth. Checked when a request arrives and
/// again when the stream task starts it: a response that begins has room to
/// finish, and one that would not is refused whole rather than cut short.
pub fn affordable(req: &mut Req) -> Result<bool> {
    let free = free_heap();
    if free >= config::STREAM_START_HEAP {
        return Ok(true);
    }
    log::warn!(
        "refusing {}: {free} B free, a response needs {} B to start",
        req.path(),
        config::STREAM_START_HEAP
    );
    req.header("Retry-After", "2")?;
    req.error(
        503,
        "503 Service Unavailable",
        "the player needs the heap right now; retry shortly",
    )?;
    Ok(false)
}

/// A zeroed [`config::IO_CHUNK`] buffer, or `None` if the heap has no block
/// that size. `vec![0; n]` would abort the board instead: seen with a GIF
/// just opened (13 KB free, the largest block 1.6 KB) as an upload began.
pub fn chunk_buffer() -> Option<Vec<u8>> {
    let mut buf = Vec::new();
    buf.try_reserve_exact(config::IO_CHUNK).ok()?;
    buf.resize(config::IO_CHUNK, 0);
    Some(buf)
}

/// Wait between chunks while the heap is below the player's reserve.
///
/// Gives up after [`config::HEAP_WAIT`] so a wedged transfer cannot hold a
/// socket forever. The error cuts the response short; the request wrapper
/// logs that with the path, the bytes sent of the total and the free heap,
/// and counts it (`httpd::streams_aborted`). Every streamed responder calls
/// this between chunks, and uploads between writes.
pub fn wait_for_heap() -> Result<()> {
    let deadline = std::time::Instant::now() + config::HEAP_WAIT;
    while free_heap() < config::HEAP_RESERVE {
        if std::time::Instant::now() >= deadline {
            bail!(
                "the heap stayed under the {} B reserve for {} s",
                config::HEAP_RESERVE,
                config::HEAP_WAIT.as_secs()
            );
        }
        thread::sleep(Duration::from_millis(20));
    }
    Ok(())
}

/// Map a card-absolute request path onto the filesystem, refusing anything
/// that tries to leave `root`.
///
/// `rel` is the already-percent-decoded path fragment.
pub fn card_path(root: &str, rel: &str) -> Option<String> {
    let rel = rel.trim_start_matches('/');
    if rel.contains('\0') || rel.contains('\\') {
        return None;
    }
    for component in rel.split('/') {
        if component == ".." {
            return None;
        }
    }
    if rel.is_empty() {
        Some(root.to_string())
    } else {
        Some(format!("{root}/{rel}"))
    }
}

/// Content type by extension. Everything the web app ships (served from the
/// www pack, `www.rs`, by its logical name), plus the card's own files;
/// anything unknown is an octet stream rather than a guess.
pub fn mime_for(path: &str) -> &'static str {
    // Root manifests have whatever extension their owner chose
    // (`filelist.hall`, `filelist.xmas`); they are all text.
    if is_root_manifest_name(path) {
        return "text/plain; charset=utf-8";
    }
    let ext = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "html" | "htm" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "wasm" => "application/wasm",
        "css" => "text/css; charset=utf-8",
        "json" | "map" => "application/json",
        "png" => "image/png",
        "gif" => "image/gif",
        "jpg" | "jpeg" => "image/jpeg",
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        "txt" | "meta" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

/// Cache policy by path.
///
/// `wasm-pack` puts no hash in its filenames, so `pkg/` cannot be immutable
/// however much we would like it to be: an hour is the compromise between a
/// fast reload and a stale wasm module after `make www-push`. The HTML and JS
/// that reference it are revalidated every time.
pub fn cache_for(path: &str) -> &'static str {
    // A manifest is replaced in place by the install path; never let a
    // browser keep an old copy for an hour.
    if is_root_manifest_name(path) {
        return "no-cache";
    }
    if path.contains("/pkg/") {
        "max-age=3600"
    } else {
        match Path::new(path)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
        {
            "html" | "htm" | "js" | "mjs" | "json" | "meta" | "txt" => "no-cache",
            _ => "max-age=3600",
        }
    }
}

/// Send a file from the card, [`config::IO_CHUNK`] at a time, with a
/// `Content-Length` (its size from `fstat`): a body cut short is visible to
/// the client as such, and always logged here (see [`wait_for_heap`]).
///
/// Returns false if no file is there, leaving the response untouched so the
/// caller can 404 or fall through. (The web app is not on the card: it is
/// served from flash by `www.rs`.)
///
/// **Every path lookup is a linear scan of the FAT directory** — ~0.65 ms per
/// 512 B directory sector, 330-550 ms in the 3,781-entry `gifs/Consoles` —
/// and it holds the FATFS lock the player needs for its own reads. So the
/// file is opened straight away (one scan) and sized with `fstat` on the open
/// handle (no scan), never `stat`ed first: one scan per request, where it
/// used to be three.
pub fn serve(req: &mut Req, path: &str) -> Result<bool> {
    let t_all = probe::http_stamp();
    let t = probe::http_stamp();
    let (mut file, actual) = match fs::File::open(path) {
        Ok(f) => (f, path),
        Err(_) => {
            probe::http_log("open-miss", t_all, path);
            return Ok(false);
        }
    };
    let open_us = probe::elapsed_us(t);
    // `fstat` on the handle: FATFS answers it from the open file object.
    let Some(total) = file
        .metadata()
        .ok()
        .filter(|m| m.is_file())
        .map(|m| m.len())
    else {
        return Ok(false);
    };
    if probe::HTTP_LOG {
        probe::http_log("open", t_all, &format!("(open={open_us}) {actual}"));
    }
    // The queue may have held this a while: is there still room to start?
    if !affordable(req)? {
        return Ok(true);
    }
    let Some(mut buf) = chunk_buffer() else {
        log::warn!("refusing {path}: no {} B block free", config::IO_CHUNK);
        req.header("Retry-After", "2")?;
        req.error(
            503,
            "503 Service Unavailable",
            "the player needs the heap right now; retry shortly",
        )?;
        return Ok(true);
    };
    let (mut reads, mut rd_tot, mut rd_max) = (0u32, 0u32, 0u32);
    req.header("Cache-Control", cache_for(path))?;
    req.begin_sized("200 OK", mime_for(path), total)?;

    let mut left = total;
    while left > 0 {
        let t = probe::http_stamp();
        let want = (left as usize).min(buf.len());
        let n = file.read(&mut buf[..want])?;
        if probe::HTTP_LOG {
            let us = probe::elapsed_us(t);
            reads += 1;
            rd_tot += us;
            rd_max = rd_max.max(us);
        }
        if n == 0 {
            bail!("the file ended early (it was {total} B when opened)");
        }
        req.chunk(&buf[..n])?;
        left -= n as u64;
        if left == 0 {
            break;
        }
        // Hand the scheduler a turn between chunks: the player is on the other
        // core but it shares the FAT lock and the SPI bus with us. And if the
        // heap has fallen into the player's reserve, stop until it recovers.
        thread::yield_now();
        wait_for_heap()?;
    }
    if probe::HTTP_LOG {
        probe::http_log(
            "served",
            t_all,
            &format!("(reads={reads} rd_tot={rd_tot} rd_max={rd_max}) {actual}"),
        );
    }
    Ok(true)
}

/// Stream a directory listing as JSON.
///
/// Streamed rather than built, because `/gifs/Consoles` on the stock card has
/// 3,781 entries and the assembled string would be ~200 KB against a heap with
/// under 20 KB free while playing. The buffer is flushed every few KB and the
/// task yields as it goes.
///
/// **Sizes are opt-in (`?sizes=1`) and this is not a stylistic choice.** The
/// directory flag comes free from `readdir`'s `d_type`, but a size means
/// `stat`, and FAT's `f_stat` re-walks the directory from its first entry every
/// time: listing that same directory with sizes is quadratic and took **108 s**
/// on the rig, against well under a second without. Worse, it is 3,781
/// directory scans over the same SPI bus the player is reading GIFs through —
/// measured, it starved the display loop into an out-of-memory abort. Without
/// sizes, `"size"` is `null`.
pub fn stream_dir_json(req: &mut Req, card_dir: &str, fs_dir: &str, sizes: bool) -> Result<()> {
    let t_all = probe::http_stamp();
    let opened = fs::read_dir(fs_dir);
    probe::http_log("opendir", t_all, fs_dir);
    let mut rd_max = 0u32;
    let entries = match opened {
        Ok(e) => e,
        Err(e) => {
            req.not_found(&format!("{card_dir}: {e}"))?;
            return Ok(());
        }
    };

    req.header("Cache-Control", "no-cache")?;
    req.begin_chunked("200 OK", "application/json")?;

    let mut out = String::with_capacity(4096);
    out.push_str(&format!(r#"{{"dir":{},"entries":["#, json_string(card_dir)));
    let mut first = true;
    let mut count: usize = 0;

    let mut entries = entries;
    loop {
        let t = probe::http_stamp();
        let next = entries.next();
        if probe::HTTP_LOG {
            rd_max = rd_max.max(probe::elapsed_us(t));
        }
        let Some(next) = next else { break };
        let Ok(entry) = next else { continue };
        let name = entry.file_name().to_string_lossy().into_owned();
        // `file_type` reads `d_type` straight out of the dirent; `metadata`
        // would be a fresh `stat` per entry. See the note above.
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        let size = if sizes {
            entry
                .metadata()
                .map(|m| m.len().to_string())
                .unwrap_or_else(|_| "null".into())
        } else {
            "null".to_string()
        };
        if !first {
            out.push(',');
        }
        first = false;
        out.push_str(&format!(
            r#"{{"name":{},"size":{size},"dir":{is_dir}}}"#,
            json_string(&name),
        ));

        count += 1;
        if out.len() >= 2048 {
            req.chunk(out.as_bytes())?;
            out.clear();
            thread::yield_now();
            wait_for_heap()?;
        }
        if count.is_multiple_of(64) {
            thread::yield_now();
        }
    }

    out.push_str(&format!(r#"],"count":{count},"sizes":{sizes}}}"#));
    req.chunk(out.as_bytes())?;
    req.finish()?;
    if probe::HTTP_LOG {
        probe::http_log(
            "listing",
            t_all,
            &format!("(entries={count} readdir_max={rd_max} sizes={sizes}) {fs_dir}"),
        );
    }
    Ok(())
}

/// Stream a request body to `dest`, via a temp file in the same directory.
///
/// Returns the byte count. Parent directories are created. The caller is
/// responsible for making sure the player is not reading `dest`.
pub fn receive_upload(up: &mut Upload, dest: &str) -> Result<u64> {
    let temp = format!("{dest}.part");
    stream_body_to(up, &temp)?;
    replace(&temp, dest)?;
    Ok(up.received())
}

/// The on-card playlist index that belongs to a manifest: `<manifest>.idx`.
/// Derived data, rebuilt by the main task whenever it is missing or stale.
pub fn index_path(manifest: &str) -> String {
    format!("{manifest}.idx")
}

/// Is `path` (filesystem, `/sdcard/...`) something the player could use as a
/// manifest — the card's `filelist.*` or anything in `sets/`? Only these have
/// an `.idx` sidecar worth looking for; checking every upload would cost a
/// directory scan per GIF, and FAT directories here run to 3,781 entries.
pub fn is_manifest_path(path: &str) -> bool {
    if path.ends_with(".idx")
        || path.ends_with(".tmp")
        || path.ends_with(".part")
        || path.ends_with(".meta")
    {
        return false;
    }
    let p = Path::new(path);
    let in_root = p.parent().is_some_and(|d| d == Path::new(config::SD_MOUNT));
    let in_sets = p.parent().is_some_and(|d| d == Path::new(config::SETS_DIR));
    let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
    (in_root && name.starts_with("filelist.")) || in_sets
}

/// Is `path` (filesystem) a playlist's overrides file — `/filelist*.meta`
/// at the root or `/sets/<name>.meta`? Returns the manifest it belongs to
/// (`opendmd_core::overrides::manifest_for_meta`).
pub fn meta_manifest(path: &str) -> Option<String> {
    let p = Path::new(path);
    let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let in_root = p.parent().is_some_and(|d| d == Path::new(config::SD_MOUNT));
    let in_sets = p.parent().is_some_and(|d| d == Path::new(config::SETS_DIR));
    if !name.ends_with(".meta") || !(in_sets || (in_root && name.starts_with("filelist"))) {
        return None;
    }
    opendmd_core::overrides::manifest_for_meta(path)
}

/// Remove the directory `dir` and everything under it, deepest first, for
/// at most `budget`: `(entries removed, done)`. One directory's names are
/// read before any of them is removed (FATFS must not be iterating a
/// directory while its entries go), and the task yields between entries so
/// the player's reads get the FAT lock in between. `NotFound` if `dir` is
/// not there.
pub fn remove_tree(dir: &str, budget: std::time::Duration) -> std::io::Result<(u32, bool)> {
    fn go(
        dir: &str,
        until: std::time::Instant,
        removed: &mut u32,
        depth: u32,
    ) -> std::io::Result<bool> {
        let mut names = Vec::new();
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let is_dir = entry.file_type().is_ok_and(|t| t.is_dir());
            names.push((entry.file_name().to_string_lossy().into_owned(), is_dir));
        }
        for (name, is_dir) in names {
            if std::time::Instant::now() >= until {
                return Ok(false);
            }
            let p = format!("{dir}/{name}");
            if is_dir {
                if depth >= 8 || !go(&p, until, removed, depth + 1)? {
                    return Ok(false);
                }
            } else {
                std::fs::remove_file(&p)?;
                *removed += 1;
            }
            std::thread::yield_now();
        }
        std::fs::remove_dir(dir)?;
        *removed += 1;
        Ok(true)
    }
    let mut removed = 0;
    let done = go(dir, std::time::Instant::now() + budget, &mut removed, 0)?;
    Ok((removed, done))
}

/// Is filesystem path `path` inside directory `dir` (case-insensitively, as
/// FAT compares)?
pub fn under(path: &str, dir: &str) -> bool {
    path.len() > dir.len()
        && path.as_bytes()[dir.len()] == b'/'
        && path
            .get(..dir.len())
            .is_some_and(|p| p.eq_ignore_ascii_case(dir))
}

/// Do two filesystem paths name the same file on the card? FAT compares
/// names case-insensitively, so `/sets/A.TXT` *is* `/sets/a.txt`: a guard
/// that compares with `==` can be walked around by changing the case.
pub fn same_card_path(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

/// Does this path's file name look like a root manifest (`filelist.*`, but not
/// its `.idx` or a temp file)? Name only; where it lives is the caller's check.
fn is_root_manifest_name(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.starts_with("filelist.")
        && !name.ends_with(".idx")
        && !name.ends_with(".tmp")
        && !name.ends_with(".part")
}

/// Delete the `.idx` beside a manifest that is about to be replaced or removed.
///
/// The index header's staleness check (manifest length + filter fingerprint)
/// would catch most replacements on its own, but not a new manifest of exactly
/// the same length, and not a crash between two writes. So the order is fixed:
/// **delete the index first, then write the manifest.** A crash in between
/// leaves an old manifest with no index, which just costs a rebuild; the other
/// order could leave a new manifest beside an index of the old one. A missing
/// index is not an error.
pub fn remove_index_beside(manifest: &str) -> Result<()> {
    let idx = index_path(manifest);
    match fs::remove_file(&idx) {
        Ok(()) => {
            log::info!("removed stale {idx}");
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => bail!("removing {idx}: {e}"),
    }
}

/// Stream a request body to `temp`, creating parent directories.
///
/// If it cannot all be received and written — no data for
/// [`config::UPLOAD_IDLE_TIMEOUT`], the [`config::UPLOAD_MAX_TIME`] cap, the
/// client gone, the heap that did not come back, a card write that failed —
/// the temp file is removed, one warn line says how far it got (`PUT <path>:
/// upload aborted at <received> of <Content-Length> B after <s> s: <why>`),
/// and the error is an [`UploadAborted`] carrying what to answer.
pub fn stream_body_to(up: &mut Upload, temp: &str) -> Result<()> {
    if let Some(parent) = Path::new(temp).parent() {
        fs::create_dir_all(parent)?;
    }
    let Some(mut buf) = chunk_buffer() else {
        log::warn!(
            "{} {}: refused, no {} B block free",
            up.method(),
            up.path(),
            config::IO_CHUNK
        );
        up.header("Retry-After", "2");
        return Err(UploadAborted {
            status: Some((503, "503 Service Unavailable")),
        }
        .into());
    };
    let mut file = fs::File::create(temp)?;
    let outcome = loop {
        let n = match up.read(&mut buf) {
            Ok(0) => {
                break file
                    .flush()
                    .map_err(|e| (e.into(), Some((500, "500 Internal Server Error"))))
            }
            Ok(n) => n,
            Err(e) => {
                let status = e.downcast_ref::<BodyError>().and_then(|b| b.status);
                break Err((e, status));
            }
        };
        if let Err(e) = wait_for_heap() {
            break Err((e, Some((503, "503 Service Unavailable"))));
        }
        if let Err(e) = file.write_all(&buf[..n]) {
            break Err((
                anyhow::Error::from(e).context("writing to the card"),
                Some((500, "500 Internal Server Error")),
            ));
        }
        thread::yield_now();
    };
    drop(file);
    let Err((e, status)) = outcome else {
        return Ok(());
    };
    let _ = fs::remove_file(temp);
    log::warn!(
        "{} {}: upload aborted at {} of {} B after {:.1} s: {e:#}",
        up.method(),
        up.path(),
        up.received(),
        up.content_len(),
        up.elapsed().as_secs_f32()
    );
    Err(UploadAborted { status }.into())
}

/// Move `temp` over `dest`. FAT's `rename` refuses an existing destination, so
/// the old file goes first; there is no atomic replace to be had here.
pub fn replace(temp: &str, dest: &str) -> Result<()> {
    if fs::metadata(dest).is_ok() {
        fs::remove_file(dest)?;
    }
    if let Err(e) = fs::rename(temp, dest) {
        let _ = fs::remove_file(temp);
        bail!("rename {temp} -> {dest}: {e}");
    }
    Ok(())
}

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
use crate::httpd::{json_string, Req};
use crate::state::free_heap;

/// Refuse to start a transfer that the player cannot afford.
///
/// Returns `Ok(false)` having already sent a 503 if the heap is below
/// [`config::HEAP_RESERVE`]. See that constant for why this exists.
pub fn affordable(req: &mut Req) -> Result<bool> {
    let free = free_heap();
    if free >= config::HEAP_RESERVE {
        return Ok(true);
    }
    log::warn!(
        "refusing {}: {free} B free, reserve is {} B",
        req.path(),
        config::HEAP_RESERVE
    );
    req.header("Retry-After", "1")?;
    req.error(
        503,
        "503 Service Unavailable",
        "the player needs the heap right now; retry shortly",
    )?;
    Ok(false)
}

/// Wait between chunks while the heap is below the player's reserve.
///
/// Gives up after [`config::HEAP_WAIT`] so a wedged transfer cannot hold a
/// socket forever.
fn wait_for_heap() -> Result<()> {
    let deadline = std::time::Instant::now() + config::HEAP_WAIT;
    while free_heap() < config::HEAP_RESERVE {
        if std::time::Instant::now() >= deadline {
            bail!(
                "gave up waiting for heap ({} B free, reserve {} B)",
                free_heap(),
                config::HEAP_RESERVE
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

/// Content type by extension. Everything the web app ships, plus the card's
/// own files; anything unknown is an octet stream rather than a guess.
pub fn mime_for(path: &str) -> &'static str {
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
        // ODM v1, the pre-rendered frame format (docs/odm-format.md).
        "odm" => "application/octet-stream",
        "jpg" | "jpeg" => "image/jpeg",
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

/// Cache policy by path.
///
/// `wasm-pack` puts no hash in its filenames, so `pkg/` cannot be immutable
/// however much we would like it to be: an hour is the compromise between a
/// fast reload and a stale wasm module after `make card-sync`. The HTML and JS
/// that reference it are revalidated every time.
pub fn cache_for(path: &str) -> &'static str {
    if path.contains("/pkg/") {
        "max-age=3600"
    } else {
        match Path::new(path)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
        {
            "html" | "htm" | "js" | "mjs" | "json" => "no-cache",
            _ => "max-age=3600",
        }
    }
}

/// Send a file from the card, 4 KB at a time.
///
/// If `<path>.gz` exists it is sent instead with `Content-Encoding: gzip` —
/// the content type still comes from the *logical* name, so `app.js.gz` is
/// served as JavaScript. Returns false if neither file exists, leaving the
/// response untouched so the caller can 404 or fall through.
pub fn serve(req: &mut Req, path: &str) -> Result<bool> {
    let gz = format!("{path}.gz");
    let (actual, gzipped) = if fs::metadata(&gz).is_ok_and(|m| m.is_file()) {
        (gz, true)
    } else {
        (path.to_string(), false)
    };
    let Ok(meta) = fs::metadata(&actual) else {
        return Ok(false);
    };
    if !meta.is_file() {
        return Ok(false);
    }

    let mut file = fs::File::open(&actual)?;
    if gzipped {
        req.header("Content-Encoding", "gzip")?;
    }
    req.header("Cache-Control", cache_for(path))?;
    req.begin_chunked("200 OK", mime_for(path))?;

    let mut buf = vec![0u8; config::IO_CHUNK];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        req.chunk(&buf[..n])?;
        // Hand the scheduler a turn between chunks: the player is on the other
        // core but it shares the FAT lock and the SPI bus with us. And if the
        // heap has fallen into the player's reserve, stop until it recovers.
        thread::yield_now();
        wait_for_heap()?;
    }
    req.finish()?;
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
    let entries = match fs::read_dir(fs_dir) {
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

    for entry in entries.flatten() {
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
    Ok(())
}

/// Stream a request body to `dest`, via a temp file in the same directory.
///
/// Returns the byte count. Parent directories are created. The caller is
/// responsible for making sure the player is not reading `dest`.
pub fn receive_upload(req: &mut Req, dest: &str) -> Result<u64> {
    let temp = format!("{dest}.part");
    stream_body_to(req, &temp)?;
    let written = fs::metadata(&temp).map(|m| m.len()).unwrap_or(0);
    replace(&temp, dest)?;
    Ok(written)
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
    if path.ends_with(".idx") || path.ends_with(".tmp") || path.ends_with(".part") {
        return false;
    }
    let p = Path::new(path);
    let in_root = p.parent().is_some_and(|d| d == Path::new(config::SD_MOUNT));
    let in_sets = p.parent().is_some_and(|d| d == Path::new(config::SETS_DIR));
    let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
    (in_root && name.starts_with("filelist.")) || in_sets
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
pub fn stream_body_to(req: &mut Req, temp: &str) -> Result<()> {
    if let Some(parent) = Path::new(temp).parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = fs::File::create(temp)?;
    let mut buf = vec![0u8; config::IO_CHUNK];
    loop {
        let n = match req.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) => {
                drop(file);
                let _ = fs::remove_file(temp);
                return Err(e);
            }
        };
        wait_for_heap()?;
        if let Err(e) = file.write_all(&buf[..n]) {
            drop(file);
            let _ = fs::remove_file(temp);
            return Err(e.into());
        }
        thread::yield_now();
    }
    file.flush()?;
    Ok(())
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

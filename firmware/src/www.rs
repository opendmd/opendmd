//! The web app, served out of flash.
//!
//! `make web` packs the built app into a www pack (`opendmd_core::www_pack`,
//! `docs/www-pack.md`), which lives in the `www` data partition
//! (`partitions.csv`: data/`esphttpd`, 0x2C0000, 1,280 KB). At boot [`init`]
//! reads its 64-byte header, memory-maps exactly the pack's length
//! (`esp_partition_mmap`, `ESP_PARTITION_MMAP_DATA`: MMU pages, no heap
//! beyond IDF's small bookkeeping entry), checks every entry's bounds and the
//! CRC over the mapped bytes in one pass (no buffer), and keeps the mapping.
//! [`serve`] then sends a file as slices of the mapping, read through the
//! flash cache on CPU0 into a 1 KB stack buffer (never on CPU1, whose cache
//! the display loop runs from): no card read, nothing on the heap per file
//! beyond what lwip copies into its own buffers. The card is not involved at
//! all, so the app (and the setup flow) work with no card inserted.
//!
//! **Updating it** is `PUT /api/www` with a whole pack ([`receive`]), on the
//! stream task like every other upload. The header is read and checked before
//! anything is erased; then the old mapping is dropped (unmapped) and the
//! body is written as it arrives, a 4 KB sector erased ahead of each write;
//! then the pack is parsed, CRC-checked and mapped again. A bad or cut-short
//! upload leaves the API running and the UI answering 503 with the reason
//! until a good pack is written. Flash erases stall both CPUs' caches, so the
//! display loop misses frames while a pack is written; a pause after every
//! sector erase ([`config::WWW_ERASE_BREATHER`]) keeps that to short stalls
//! (see CLAUDE.md).
//!
//! **Why a response can never read an unmapped pack.** The mapping sits in
//! [`MAP`], and a response holds that lock from the lookup to its last byte;
//! the update takes the same lock before it unmaps and keeps it until the new
//! pack is mapped. Both run on the stream task (downloads are `hand_off`ed
//! there, uploads `receive`d), one job at a time, so in practice the lock is
//! never contended — a request that arrives during an update waits in the
//! stream queue and is served from the new pack (a third, with two already
//! waiting, gets `503` + `Retry-After: 1` after 2 s, as behind any upload). The lock is what keeps it correct if that ever changes (the
//! stream task failing to start runs both inline on the httpd task, which is
//! also one at a time). What `/api/status` and `GET /api/www` read is a
//! separate small [`Info`], never the mapping's lock, so neither waits for a
//! response or an update.

use anyhow::Result;
use esp_idf_sys as sys;
use opendmd_core::www_pack::{Header, Pack, PackError, HEADER_LEN};
use std::sync::Mutex;
use std::time::Instant;

use crate::config;
use crate::files;
use crate::httpd::{json_string, BodyError, Req, Respond, Upload};

/// The partition's name and data subtype in `partitions.csv`.
const LABEL: &std::ffi::CStr = c"www";
const SUBTYPE: sys::esp_partition_subtype_t =
    sys::esp_partition_subtype_t_ESP_PARTITION_SUBTYPE_DATA_ESPHTTPD;
/// Flash erase granularity.
const SECTOR: usize = 4096;
/// A response's slices are copied out of the mapping into a buffer this size
/// on the serving task's stack (see [`serve`]): no heap, and the flash is only
/// ever read on CPU0.
const STAGING: usize = 1024;

/// The live mapping of the pack. Held by a response for its whole length
/// and by an update from before the unmap to after the remap.
struct Mapping {
    ptr: *const u8,
    len: usize,
    handle: sys::esp_partition_mmap_handle_t,
}

// SAFETY: the pointer is into memory-mapped flash, valid until the handle is
// unmapped, which only `Drop` does; `MAP`'s lock serialises every use.
unsafe impl Send for Mapping {}

impl Mapping {
    fn bytes(&self) -> &[u8] {
        // SAFETY: `ptr..ptr+len` was mapped by `esp_partition_mmap` and stays
        // mapped for as long as `self` exists.
        unsafe { std::slice::from_raw_parts(self.ptr, self.len) }
    }
}

impl Drop for Mapping {
    fn drop(&mut self) {
        // SAFETY: the handle came from a successful `esp_partition_mmap` and
        // is unmapped exactly once.
        unsafe { sys::esp_partition_munmap(self.handle) };
    }
}

static MAP: Mutex<Option<Mapping>> = Mutex::new(None);

/// The pack as `/api/status` and `GET /api/www` report it.
#[derive(Clone)]
pub struct Info {
    /// A verified pack is mapped and being served.
    pub ok: bool,
    /// Its header's version string (`git describe` of the app).
    pub version: Option<String>,
    pub bytes: u32,
    pub entries: u16,
    /// Why there is no web app, when `ok` is false.
    pub error: Option<String>,
}

static INFO: Mutex<Info> = Mutex::new(Info {
    ok: false,
    version: None,
    bytes: 0,
    entries: 0,
    error: None,
});

pub fn info() -> Info {
    INFO.lock().unwrap_or_else(|p| p.into_inner()).clone()
}

fn set_info(i: Info) {
    *INFO.lock().unwrap_or_else(|p| p.into_inner()) = i;
}

fn broken(error: String) -> Info {
    Info {
        ok: false,
        version: None,
        bytes: 0,
        entries: 0,
        error: Some(error),
    }
}

/// `{"version","bytes","entries","ok","error"}` — `GET /api/www`, and the
/// `www` object in `/api/status`.
pub fn info_json() -> String {
    let i = info();
    format!(
        r#"{{"version":{},"bytes":{},"entries":{},"ok":{},"error":{}}}"#,
        i.version
            .as_deref()
            .map_or_else(|| "null".into(), json_string),
        i.bytes,
        i.entries,
        i.ok,
        i.error
            .as_deref()
            .map_or_else(|| "null".into(), json_string),
    )
}

fn partition() -> Option<&'static sys::esp_partition_t> {
    // SAFETY: a read-only query of the partition table; the returned entry
    // lives for the life of the program.
    unsafe {
        sys::esp_partition_find_first(
            sys::esp_partition_type_t_ESP_PARTITION_TYPE_DATA,
            SUBTYPE,
            LABEL.as_ptr(),
        )
        .as_ref()
    }
}

/// What to tell a person about a pack that failed to load.
fn describe(e: &PackError) -> String {
    match e {
        PackError::BadMagic => {
            "the www partition holds no web app (erased, or never written): flash the full image, or `make www-push`".into()
        }
        e => format!("the web app in flash is damaged ({e}): `make www-push` a good one"),
    }
}

/// Read the header, map the pack, check it. Touches neither lock.
fn load() -> std::result::Result<(Mapping, Info), String> {
    let part = partition().ok_or_else(|| {
        "this flash has no www partition (an old partition table): flash the full image".to_string()
    })?;
    let mut head = [0u8; HEADER_LEN];
    // SAFETY: reading HEADER_LEN bytes into a live stack buffer of that size.
    let err = unsafe { sys::esp_partition_read(part, 0, head.as_mut_ptr().cast(), HEADER_LEN) };
    if err != sys::ESP_OK {
        return Err(format!("reading the www partition failed ({err})"));
    }
    let header = Header::parse(&head).map_err(|e| describe(&e))?;
    let len = header.total_len as usize;
    if len > part.size as usize {
        return Err(format!(
            "the pack claims {len} B, the partition is {} B",
            part.size
        ));
    }
    let mut ptr: *const std::ffi::c_void = std::ptr::null();
    let mut handle: sys::esp_partition_mmap_handle_t = 0;
    // SAFETY: `part` is a live partition entry and `len` is within it; the
    // out-pointers are live locals.
    let err = unsafe {
        sys::esp_partition_mmap(
            part,
            0,
            len,
            sys::esp_partition_mmap_memory_t_ESP_PARTITION_MMAP_DATA,
            &mut ptr,
            &mut handle,
        )
    };
    if err != sys::ESP_OK {
        return Err(format!("mapping the www partition failed ({err})"));
    }
    let mapping = Mapping {
        ptr: ptr.cast(),
        len,
        handle,
    };
    let pack = Pack::parse(mapping.bytes()).map_err(|e| describe(&e))?;
    // One pass over the mapped bytes through the flash cache: no buffer.
    pack.verify().map_err(|e| describe(&e))?;
    let info = Info {
        ok: true,
        version: Some(pack.version().to_string()),
        bytes: len as u32,
        entries: pack.count() as u16,
        error: None,
    };
    Ok((mapping, info))
}

/// Map and verify the pack at boot. Never fails the boot: a missing or bad
/// pack is logged, reported in `/api/status`, and the UI answers 503.
pub fn init() {
    let t = Instant::now();
    let mut map = MAP.lock().unwrap_or_else(|p| p.into_inner());
    match load() {
        Ok((mapping, info)) => {
            log::info!(
                "www: pack '{}' verified, {} entries, {} B, mapped at {:p}, CRC pass {} ms",
                info.version.as_deref().unwrap_or(""),
                info.entries,
                info.bytes,
                mapping.ptr,
                t.elapsed().as_millis()
            );
            *map = Some(mapping);
            set_info(info);
        }
        Err(e) => {
            log::warn!("www: no web app: {e}");
            *map = None;
            set_info(broken(e));
        }
    }
}

/// What [`serve`] did.
pub enum Served {
    Sent,
    /// A good pack, but no such file in it.
    NotFound,
    /// No good pack: the reason, for a 503.
    Unavailable(String),
}

/// Send `rel` (no leading slash: `index.html`, `js/now.js`) from the pack.
pub fn serve(req: &mut Req, rel: &str) -> Result<Served> {
    let map = MAP.lock().unwrap_or_else(|p| p.into_inner());
    let Some(mapping) = map.as_ref() else {
        return Ok(Served::Unavailable(
            info()
                .error
                .unwrap_or_else(|| "the web app is not loaded".into()),
        ));
    };
    // Parsed again per request (header + ~40 entries' bounds: microseconds)
    // rather than keeping a `Pack<'static>` beside the mapping.
    let pack = match Pack::parse(mapping.bytes()) {
        Ok(p) => p,
        Err(e) => return Ok(Served::Unavailable(describe(&e))),
    };
    let Some(entry) = pack.get(rel) else {
        return Ok(Served::NotFound);
    };
    // The queue may have held this a while: is there still room to start?
    if !files::affordable(req)? {
        return Ok(Served::Sent);
    }
    let logical = format!("/{rel}");
    if entry.gzip {
        req.header("Content-Encoding", "gzip")?;
    }
    req.header("Cache-Control", files::cache_for(&logical))?;
    req.begin_sized("200 OK", files::mime_for(&logical), entry.data.len() as u64)?;
    // **Every read of the mapping happens here, on CPU0.** lwip copies a
    // send buffer into its pbufs on the tcpip task (priority 18): before
    // that task was pinned to CPU0 (`CONFIG_LWIP_TCPIP_TASK_AFFINITY_CPU0`
    // in sdkconfig.defaults), handed the mapping itself the copy often ran
    // on CPU1 and pulled the pack through CPU1's flash cache, evicting the
    // display loop's code — measured, a steady UI download at ~450 KB/s
    // cost +66 and +80 deadline misses per 90 s (idle: +5; pinned and
    // staged: +8). Each slice is still staged on this task's stack first
    // (the stream and httpd tasks are pinned to CPU0 too) so lwip only ever
    // copies RAM, whatever core a future lwip path copies on.
    let mut staging = [0u8; STAGING];
    let mut pieces = entry.data.chunks(STAGING).peekable();
    while let Some(piece) = pieces.next() {
        let staged = &mut staging[..piece.len()];
        staged.copy_from_slice(piece);
        req.chunk(staged)?;
        if pieces.peek().is_some() {
            std::thread::yield_now();
            files::wait_for_heap()?;
        }
    }
    Ok(Served::Sent)
}

/// Answer "no web app" in plain text, with the reason.
pub fn unavailable(req: &mut Req, why: &str) -> Result<()> {
    req.header("Cache-Control", "no-store")?;
    let body = format!(
        "OpenDMD: the web app is unavailable.\n\n{why}\n\nThe API is up; the setup page is at /setup.\n"
    );
    req.send_status(
        503,
        "503 Service Unavailable",
        "text/plain; charset=utf-8",
        body.as_bytes(),
    )
}

/// `PUT /api/www`, on the stream task: write a whole pack into the partition
/// and serve it. See the module docs for the order of things.
pub fn receive(up: &mut Upload) -> Result<()> {
    let Some(part) = partition() else {
        return up.error(
            500,
            "500 Internal Server Error",
            "this flash has no www partition (an old partition table): flash the full image",
        );
    };
    let len = up.content_len() as usize;
    if len == 0 {
        return up.error(411, "411 Length Required", "Content-Length required");
    }
    if len > part.size as usize {
        return up.error(
            413,
            "413 Payload Too Large",
            &format!("the pack is {len} B, the www partition is {} B", part.size),
        );
    }
    let Some(mut buf) = files::chunk_buffer() else {
        up.header("Retry-After", "2");
        return up.error(
            503,
            "503 Service Unavailable",
            "the player needs the heap right now; retry shortly",
        );
    };

    // The header first, before anything is erased: a wrong file leaves the
    // web app that is there untouched.
    let mut have = 0;
    while have < HEADER_LEN {
        match up.read(&mut buf[have..HEADER_LEN]) {
            Ok(0) => break,
            Ok(n) => have += n,
            Err(e) => return aborted_before_erase(up, &e),
        }
    }
    let header = match Header::parse(&buf[..have]) {
        Ok(h) => h,
        Err(e) => return up.bad_request(&format!("not a www pack: {e}")),
    };
    if header.total_len as usize != len {
        return up.bad_request(&format!(
            "the pack's header says {} B, the body is {len} B",
            header.total_len
        ));
    }

    let started = Instant::now();
    let mut map = MAP.lock().unwrap_or_else(|p| p.into_inner());
    // Unmap before the first erase: nothing may read the old pack from here.
    *map = None;
    set_info(Info {
        error: Some("the web app is being updated; reload in a moment".into()),
        ..broken(String::new())
    });
    log::info!(
        "www: receiving pack '{}', {len} B, {} entries",
        header.version(),
        header.count
    );

    let mut off = 0usize;
    let mut erased = 0usize;
    let mut pending = have;
    let failure: Option<(String, Option<(u16, &'static str)>)> = loop {
        if pending > 0 {
            let end = off + pending;
            let mut flash_err = None;
            while erased < end {
                // SAFETY: a sector-aligned range inside the partition (`len`
                // <= its size, which is a whole number of sectors).
                let err = unsafe { sys::esp_partition_erase_range(part, erased, SECTOR) };
                if err != sys::ESP_OK {
                    flash_err = Some(format!("erase at {erased} failed ({err})"));
                    break;
                }
                erased += SECTOR;
                // A sector erase keeps both CPUs' flash caches off for its
                // ~15-20 ms, so the display loop (CPU1) stalls meanwhile;
                // back to back they starved it (a frame 1.46 s late, +20
                // misses in a 3.3 s push). A pause after each gives it room.
                std::thread::sleep(config::WWW_ERASE_BREATHER);
            }
            if flash_err.is_none() {
                // SAFETY: `buf[..pending]` is live and `off + pending` is
                // within the partition.
                let err =
                    unsafe { sys::esp_partition_write(part, off, buf.as_ptr().cast(), pending) };
                if err != sys::ESP_OK {
                    flash_err = Some(format!("write at {off} failed ({err})"));
                }
            }
            if let Some(e) = flash_err {
                break Some((e, Some((500, "500 Internal Server Error"))));
            }
            off = end;
        }
        match up.read(&mut buf) {
            Ok(0) => break None,
            Ok(n) => pending = n,
            Err(e) => {
                let status = e.downcast_ref::<BodyError>().and_then(|b| b.status);
                break Some((format!("{e:#}"), status));
            }
        }
        std::thread::yield_now();
    };
    drop(buf);

    if let Some((why, status)) = failure {
        log::warn!(
            "PUT /api/www: upload aborted at {} of {len} B after {:.1} s: {why}",
            up.received(),
            started.elapsed().as_secs_f32()
        );
        set_info(broken(format!(
            "the last web app upload was cut short at {} of {len} B ({why}): `make www-push` again",
            up.received()
        )));
        return match status {
            Some((code, line)) => up.error(
                code,
                line,
                &format!("upload aborted at {} of {len} B: {why}", up.received()),
            ),
            None => Ok(()),
        };
    }

    let written_s = started.elapsed().as_secs_f32();
    let t = Instant::now();
    match load() {
        Ok((mapping, info)) => {
            log::info!(
                "www: pack '{}' written ({len} B in {written_s:.1} s), verified in {} ms, {} entries",
                info.version.as_deref().unwrap_or(""),
                t.elapsed().as_millis(),
                info.entries
            );
            *map = Some(mapping);
            drop(map);
            set_info(info);
            up.json(&info_json())
        }
        Err(e) => {
            log::warn!("PUT /api/www: the written pack does not verify: {e}");
            drop(map);
            set_info(broken(e.clone()));
            up.error(422, "422 Unprocessable Entity", &e)
        }
    }
}

/// The body failed before anything was erased: say so, keep the app.
fn aborted_before_erase(up: &mut Upload, e: &anyhow::Error) -> Result<()> {
    log::warn!(
        "PUT /api/www: upload aborted at {} of {} B, before anything was erased: {e:#}",
        up.received(),
        up.content_len()
    );
    match e.downcast_ref::<BodyError>().and_then(|b| b.status) {
        Some((code, line)) => up.error(code, line, &format!("{e:#}")),
        None => Ok(()),
    }
}

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
//! stream task like every other upload. The header is read and checked
//! before anything is erased. With a card in, the rest of the body is staged
//! on the card and CRC-checked there while the old pack stays mapped, so a
//! failed upload leaves the web app as it was; without one it is written as
//! it arrives. Either way [`write_pack`] writes the header last: the body
//! from offset 64 (a 4 KB sector erased ahead of each write), its CRC read
//! back through a fresh mapping, then the 64-byte header into the still
//! erased bytes, so a header in flash always means a whole, verified pack.
//! Then the pack is parsed, CRC-checked and mapped again. Flash erases stall
//! both CPUs' caches, so the display loop misses frames while a pack is
//! written; a pause after every sector erase ([`config::WWW_ERASE_BREATHER`])
//! keeps that to short stalls (see CLAUDE.md).
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
use std::sync::atomic::{AtomicU8, Ordering::Relaxed};
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

/// Whether this boot's last `PUT /api/www` was staged on the card:
/// 0 none yet, 1 no (written to flash from the upload), 2 yes.
static STAGED_LAST: AtomicU8 = AtomicU8::new(0);

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

/// `{"version","bytes","entries","ok","error","staged"}` — `GET /api/www`,
/// `PUT /api/www`'s reply, and the `www` object in `/api/status`. `staged`:
/// whether this boot's last update was staged on the card first (null:
/// none since boot).
pub fn info_json() -> String {
    let i = info();
    format!(
        r#"{{"version":{},"bytes":{},"entries":{},"ok":{},"error":{},"staged":{}}}"#,
        i.version
            .as_deref()
            .map_or_else(|| "null".into(), json_string),
        i.bytes,
        i.entries,
        i.ok,
        i.error
            .as_deref()
            .map_or_else(|| "null".into(), json_string),
        match STAGED_LAST.load(Relaxed) {
            1 => "false",
            2 => "true",
            _ => "null",
        },
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

/// Where `PUT /api/www` stages a pack on the card before it touches flash.
const STAGED: &str = "/sdcard/update/www.pack.part";

/// Why writing a pack failed: the message, and the status to answer (none
/// when the client has gone).
type Failure = (String, Option<(u16, &'static str)>);

/// Where the body after the header comes from: the connection itself, or
/// the copy staged on the card.
enum Body<'a> {
    Upload(&'a mut Upload),
    Card(std::fs::File),
}

impl Body<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::result::Result<usize, Failure> {
        match self {
            Body::Upload(up) => up.read(buf).map_err(|e| {
                let status = e.downcast_ref::<BodyError>().and_then(|b| b.status);
                (format!("{e:#}"), status)
            }),
            Body::Card(f) => std::io::Read::read(f, buf).map_err(|e| {
                (
                    format!("reading the staged pack from the card: {e}"),
                    Some((500, "500 Internal Server Error")),
                )
            }),
        }
    }
}

/// CRC-32 of the `len - HEADER_LEN` bytes after the header, from `read`.
fn crc_of(
    mut read: impl FnMut(usize, &mut [u8]) -> std::result::Result<(), Failure>,
    len: usize,
    buf: &mut [u8],
) -> std::result::Result<u32, Failure> {
    let mut crc = opendmd_core::www_pack::Crc32::new();
    let mut off = HEADER_LEN;
    while off < len {
        let n = (len - off).min(buf.len());
        read(off, &mut buf[..n])?;
        crc.update(&buf[..n]);
        off += n;
        std::thread::yield_now();
    }
    Ok(crc.finish())
}

/// Write the pack into the partition with **the header last**. Sector 0 is
/// erased first and the body (everything after the 64-byte header) written
/// from offset 64 on, a sector erased ahead of each write; the header's 64
/// bytes stay erased (0xFF: "no web app") while the body goes in. Then the
/// body's CRC is computed again *from flash* and only if it matches the
/// header's is the header written into those erased bytes: a pack header in
/// flash always means a whole pack whose CRC verified. A failure at any
/// point leaves the header erased.
fn write_pack(
    part: &sys::esp_partition_t,
    head: &[u8; HEADER_LEN],
    header: &Header,
    len: usize,
    body: &mut Body,
    buf: &mut [u8],
) -> std::result::Result<(), Failure> {
    let flash_fail = |what: String| (what, Some((500u16, "500 Internal Server Error")));
    let mut off = HEADER_LEN;
    let mut erased = 0usize;
    while off < len {
        let want = (len - off).min(buf.len());
        let n = body.read(&mut buf[..want])?;
        if n == 0 {
            return Err((
                format!("the body ended at {off} of {len} B"),
                Some((408, "408 Request Timeout")),
            ));
        }
        while erased < off + n {
            // SAFETY: a sector-aligned range inside the partition (`len` <=
            // its size, a whole number of sectors).
            let err = unsafe { sys::esp_partition_erase_range(part, erased, SECTOR) };
            if err != sys::ESP_OK {
                return Err(flash_fail(format!("erase at {erased} failed ({err})")));
            }
            erased += SECTOR;
            // A sector erase keeps both CPUs' flash caches off for its
            // ~15-20 ms, so the display loop (CPU1) stalls meanwhile; back
            // to back they starved it (a frame 1.46 s late, +20 misses in a
            // 3.3 s push). A pause after each gives it room.
            std::thread::sleep(config::WWW_ERASE_BREATHER);
        }
        // SAFETY: `buf[..n]` is live and `off + n` is within the partition.
        let err = unsafe { sys::esp_partition_write(part, off, buf.as_ptr().cast(), n) };
        if err != sys::ESP_OK {
            return Err(flash_fail(format!("write at {off} failed ({err})")));
        }
        off += n;
        std::thread::yield_now();
    }
    if erased == 0 {
        // A pack that is all header (no entries): sector 0 still needs it.
        // SAFETY: as above.
        let err = unsafe { sys::esp_partition_erase_range(part, 0, SECTOR) };
        if err != sys::ESP_OK {
            return Err(flash_fail(format!("erase at 0 failed ({err})")));
        }
    }
    // Read back through a fresh mapping (the flash cache, on this task's
    // core, CPU0), as `load` does at boot: `esp_partition_read` would take
    // the SPI1 path, which stops both CPUs' caches for every piece.
    let from_flash = {
        let mut ptr: *const std::ffi::c_void = std::ptr::null();
        let mut handle: sys::esp_partition_mmap_handle_t = 0;
        // SAFETY: `len` is within the partition; the out-pointers are live.
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
            return Err(flash_fail(format!(
                "mapping the written pack failed ({err})"
            )));
        }
        let mapping = Mapping {
            ptr: ptr.cast(),
            len,
            handle,
        };
        let mut crc = opendmd_core::www_pack::Crc32::new();
        for piece in mapping.bytes()[HEADER_LEN..].chunks(16 * 1024) {
            crc.update(piece);
            std::thread::yield_now();
        }
        crc.finish()
        // `mapping` is unmapped here, before the header is written.
    };
    if from_flash != header.crc32 {
        return Err((
            format!(
                "CRC mismatch in flash: the header says {:08x}, the written pack is {from_flash:08x}",
                header.crc32
            ),
            Some((422, "422 Unprocessable Entity")),
        ));
    }
    // The commit: 64 bytes into erased flash.
    // SAFETY: `head` is live; bytes 0..64 of the partition are erased.
    let err = unsafe { sys::esp_partition_write(part, 0, head.as_ptr().cast(), HEADER_LEN) };
    if err != sys::ESP_OK {
        return Err(flash_fail(format!("writing the header failed ({err})")));
    }
    Ok(())
}

/// Receive the body after the header onto the card, and check its CRC
/// there. `Ok(Err(why))`: the staging file could not be created (write
/// straight to flash instead). `Ok(Ok(file))`: staged and good, rewound.
/// `Err`: answered (or the client went away); the web app in flash is
/// untouched.
#[allow(clippy::type_complexity)]
fn stage_on_card(
    up: &mut Upload,
    header: &Header,
    len: usize,
    buf: &mut [u8],
) -> std::result::Result<std::result::Result<std::fs::File, String>, Result<()>> {
    use std::io::{Read, Seek};
    let dir = format!("{}/update", config::SD_MOUNT);
    if let Err(e) = std::fs::create_dir_all(&dir) {
        return Ok(Err(format!("{dir}: {e}")));
    }
    if let Err(e) = std::fs::File::create(STAGED) {
        return Ok(Err(format!("{STAGED}: {e}")));
    }
    log::info!(
        "www: staging on the card ({STAGED}), the web app in flash stays until the pack checks out"
    );
    if let Err(e) = files::stream_body_to(up, STAGED) {
        // Logged and removed by stream_body_to; the caller of `receive`
        // answers an `UploadAborted` with its status.
        return Err(Err(e));
    }
    let checked = (|| -> std::result::Result<std::fs::File, String> {
        let mut f = std::fs::File::open(STAGED).map_err(|e| e.to_string())?;
        let got = f.metadata().map_err(|e| e.to_string())?.len() as usize + HEADER_LEN;
        if got != len {
            return Err(format!("{got} B staged, the pack is {len} B"));
        }
        let crc = crc_of(
            |_, out| f.read_exact(out).map_err(|e| (e.to_string(), None)),
            len,
            buf,
        )
        .map_err(|(e, _)| e)?;
        if crc != header.crc32 {
            return Err(format!(
                "CRC mismatch: the header says {:08x}, the pack is {crc:08x}",
                header.crc32
            ));
        }
        f.rewind().map_err(|e| e.to_string())?;
        Ok(f)
    })();
    match checked {
        Ok(f) => Ok(Ok(f)),
        Err(why) => {
            let _ = std::fs::remove_file(STAGED);
            log::warn!("PUT /api/www: refused, the web app in flash is unchanged: {why}");
            Err(up.error(
                422,
                "422 Unprocessable Entity",
                &format!("{why}; the web app in flash is unchanged"),
            ))
        }
    }
}

/// `PUT /api/www`, on the stream task: write a whole pack into the partition
/// and serve it.
///
/// **With a card** (`card_mounted`: mounted and readable), the pack is
/// received onto the card first (`/update/www.pack.part`) and its CRC
/// checked there, while the old pack stays mapped: an upload cut short, or a file that is not a good
/// pack, is answered (408, 422, …) with the web app in flash untouched.
/// Only then is the old pack unmapped and the partition rewritten from the
/// card, ~3 s, header last ([`write_pack`]). **Without a card**, the body is
/// written to flash as it arrives (header last as well), so a failure in
/// the middle leaves no web app until a good pack is written; so does a
/// card on which the staging file cannot be created (logged with the
/// reason). The reply's `staged` says which it was. Either way the header
/// is checked before anything happens (400).
pub fn receive(up: &mut Upload, card_mounted: bool) -> Result<()> {
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
    let mut head = [0u8; HEADER_LEN];
    let mut have = 0;
    while have < HEADER_LEN {
        match up.read(&mut head[have..]) {
            Ok(0) => break,
            Ok(n) => have += n,
            Err(e) => return aborted_before_erase(up, &e),
        }
    }
    let header = match Header::parse(&head[..have]) {
        Ok(h) => h,
        Err(e) => return up.bad_request(&format!("not a www pack: {e}")),
    };
    if header.total_len as usize != len {
        return up.bad_request(&format!(
            "the pack's header says {} B, the body is {len} B",
            header.total_len
        ));
    }
    log::info!(
        "www: receiving pack '{}', {len} B, {} entries",
        header.version(),
        header.count,
    );

    let started = Instant::now();
    let staged = if card_mounted {
        match stage_on_card(up, &header, len, &mut buf) {
            Ok(f) => f,
            Err(answered) => {
                STAGED_LAST.store(2, Relaxed);
                return answered;
            }
        }
    } else {
        Err("no card".to_string())
    };
    let staged = match staged {
        Ok(f) => Some(f),
        Err(why) => {
            let line = format!("www: no card staging ({why}): writing to flash from the upload, the old web app is replaced during the transfer");
            if card_mounted {
                log::warn!("{line}");
            } else {
                log::info!("{line}");
            }
            None
        }
    };
    let from_card = staged.is_some();
    STAGED_LAST.store(if from_card { 2 } else { 1 }, Relaxed);
    let mut body = match staged {
        Some(f) => Body::Card(f),
        None => Body::Upload(&mut *up),
    };

    let mut map = MAP.lock().unwrap_or_else(|p| p.into_inner());
    // Unmap before the first erase: nothing may read the old pack from here.
    *map = None;
    set_info(Info {
        error: Some("the web app is being updated; reload in a moment".into()),
        ..broken(String::new())
    });
    let written = write_pack(part, &head, &header, len, &mut body, &mut buf);
    drop(body);
    drop(buf);
    if from_card {
        let _ = std::fs::remove_file(STAGED);
    }

    if let Err((why, status)) = written {
        log::warn!(
            "PUT /api/www: writing the pack failed after {:.1} s ({}): {why}",
            started.elapsed().as_secs_f32(),
            if from_card {
                "from the card"
            } else {
                "from the upload"
            }
        );
        set_info(broken(format!(
            "the last web app update failed ({why}): `make www-push` again"
        )));
        return match status {
            Some((code, line)) => up.error(code, line, &why),
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

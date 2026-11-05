//! The web app, served out of flash.
//!
//! `make web` packs the built app into a www pack (`opendmd_core::www_pack`,
//! `docs/www-pack.md`), which lives in the `www` data partition
//! (`partitions.csv`: data/`esphttpd`, 0x2C0000, 1,280 KB; 0x380000, 2,048 KB
//! in `partitions-8mb.csv`: found by name, so either). At boot [`init`]
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
//! before anything is erased. With a card in, the whole pack is staged on
//! the card and read back whole (size and CRC, every card read retried a
//! few times) while the old pack stays mapped, so a failed upload, or a
//! card that will not read it back, leaves the web app as it was; without
//! one it is written as it arrives. If the write from the card still fails
//! once the partition is erased, the checked copy stays on the card and the
//! next boot writes it ([`restore_from_card`]). Either way [`write_pack`]
//! writes the header last: the body
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
//!
//! **Caching.** Every file goes out `Cache-Control: no-cache` with an
//! `ETag`, the mapped pack's CRC (`"1a2b3c4d"`: one value for every file of
//! a pack, new with any change to any file), and a request whose
//! `If-None-Match` names it is answered `304 Not Modified` by
//! [`not_modified`] on the httpd task, before any hand-off: a reload costs a
//! few dozen header-only answers, and a browser never runs an earlier
//! pack's file with a later one's. (Releases up to 1.0.0-beta.13 sent the
//! wasm `max-age=3600`, and wasm-pack's names carry no hash: an updated app
//! ran the old compiler. The web app now also asks for every wasm module at
//! an address with its build's id, `?v=`, js/wasm.js, which is what gets a
//! browser holding such an hour-long copy past it.)

use anyhow::Result;
use esp_idf_sys as sys;
use opendmd_core::ota::www::{PackHasher, Signed};
use opendmd_core::www_pack::{Header, Pack, PackError, HEADER_LEN};
use std::sync::atomic::{AtomicU32, AtomicU8, Ordering::Relaxed};
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

/// The mapped pack's CRC, its files' ETag ([`not_modified`]); 0 while none
/// is mapped (and for the one pack in 2^32 whose CRC is 0: no 304s then).
/// Read by the httpd task without [`MAP`]'s lock, which a response holds
/// for its whole length.
static ETAG_CRC: AtomicU32 = AtomicU32::new(0);

/// `"<crc as 8 hex digits>"`, on the stack.
fn etag_text(crc: u32, out: &mut [u8; 10]) -> &str {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    out[0] = b'"';
    out[9] = b'"';
    for i in 0..8 {
        out[1 + i] = HEX[(crc >> (28 - 4 * i)) as usize & 0xF];
    }
    std::str::from_utf8(out).unwrap_or("\"\"")
}

/// Put `mapping` (or nothing) in [`MAP`] and publish its ETag: cleared
/// before an unmap, set once the new pack is mapped.
fn set_mapping(map: &mut Option<Mapping>, mapping: Option<Mapping>) {
    let crc = mapping
        .as_ref()
        .and_then(|m| Header::parse(m.bytes()).ok())
        .map_or(0, |h| h.crc32);
    if crc == 0 {
        ETAG_CRC.store(0, Relaxed);
    }
    *map = mapping;
    ETAG_CRC.store(crc, Relaxed);
}

/// Answer `304 Not Modified` (no body) if the request's `If-None-Match`
/// names the mapped pack's ETag: the browser's copy is this pack's file.
/// On the httpd task, before the hand-off to the stream task (a 304 needs
/// no place in its queue): an atomic load, a 64 B header read, no lock.
/// True when it answered.
pub fn not_modified(req: &mut Req) -> Result<bool> {
    let crc = ETAG_CRC.load(Relaxed);
    if crc == 0 {
        return Ok(false);
    }
    let mut buf = [0u8; 10];
    let tag = etag_text(crc, &mut buf);
    if !req.header_names(c"If-None-Match", tag) {
        return Ok(false);
    }
    req.header("ETag", tag)?;
    req.header("Cache-Control", "no-cache")?;
    req.send_status(304, "304 Not Modified", "text/plain", b"")?;
    Ok(true)
}

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
/// 0 none yet (or the last one was refused before it got that far: at its
/// header), 1 no (written to flash from the upload), 2 yes.
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
/// whether this boot's last update attempt was staged on the card first
/// (null: none since boot, or the last one was refused at its header).
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
            set_mapping(&mut map, Some(mapping));
            set_info(info);
        }
        Err(e) => {
            log::warn!("www: no web app: {e}");
            set_mapping(&mut map, None);
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
    // no-cache + the pack's ETag: see "Caching" above.
    let mut tag = [0u8; 10];
    req.header("ETag", etag_text(pack.header().crc32, &mut tag))?;
    req.header("Cache-Control", "no-cache")?;
    req.app_headers(opendmd_core::access::APP_CSP)?;
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

/// Where `PUT /api/www` stages a pack on the card before it touches flash:
/// the whole pack, header included, so a copy kept after a failed write can
/// be checked and written again at the next boot ([`restore_from_card`]).
const STAGED: &str = "/sdcard/update/www.pack.part";

/// One update at a time: `PUT /api/www` and the boot-time restore both
/// read and write [`STAGED`].
static UPDATE: Mutex<()> = Mutex::new(());

/// Why writing a pack failed: the message, and the status to answer (none
/// when the client has gone).
type Failure = (String, Option<(u16, &'static str)>);

const INTERNAL: Option<(u16, &str)> = Some((500, "500 Internal Server Error"));

/// How a pack's signature is checked (docs/www-pack.md "Signature"): on
/// the network worker for `PUT /api/www`, against the digest that check
/// left in RTC memory at boot. The reason in words when it fails; `None`
/// is "could not ask".
#[derive(Clone, Copy)]
enum Verify<'a> {
    /// [`verify_on_worker`].
    Worker(&'a crate::chan::Sender<crate::http::NetCommand>),
    /// [`verify_remembered`].
    Remembered,
}

/// Check `signed` with `verify`, as a [`Failure`] (422 for a bad one).
#[optimize(size)]
fn check_signature(verify: Verify, signed: Signed) -> std::result::Result<(), Failure> {
    let r = match verify {
        Verify::Worker(net) => verify_on_worker(signed, net),
        Verify::Remembered => verify_remembered(signed),
    };
    r.map_err(|e| match e {
        Some(why) => (why.to_string(), Some((422, "422 Unprocessable Entity"))),
        None => (
            "the signature could not be checked just now; retry shortly".into(),
            Some((503, "503 Service Unavailable")),
        ),
    })
}

/// The signed digest of the last pack whose signature verified this power
/// cycle (`PUT /api/www`), in RTC memory: a software restart keeps it, a
/// power cut clears it. The boot task's [`Verify`] for a pack left on the
/// card by a write that failed ([`restore_from_card`]) takes that pack
/// alone: the network worker, which has the stack for Ed25519, is not
/// running yet, and a file on the card is anyone's to write.
#[link_section = ".rtc_noinit"]
static mut VERIFIED: [u32; 9] = [0; 9];
const VERIFIED_MAGIC: u32 = 0x5157_7357;

fn verified_word(i: usize) -> *mut u32 {
    // SAFETY: an element of the static; only reached through volatile ops.
    unsafe { (&raw mut VERIFIED).cast::<u32>().add(i) }
}

/// Remember `signed`'s digest (it verified).
fn remember(signed: &Signed) {
    for (i, w) in signed.message[9..].chunks(4).enumerate() {
        // SAFETY: see [`verified_word`]; the stream and boot tasks never run
        // an update at once (`UPDATE`).
        unsafe { verified_word(i + 1).write_volatile(u32::from_le_bytes([w[0], w[1], w[2], w[3]])) };
    }
    // SAFETY: as above.
    unsafe { verified_word(0).write_volatile(VERIFIED_MAGIC) };
}

/// `PUT /api/www`'s [`Verify`]: the network worker checks, and a pack that
/// verified is [`remember`]ed.
#[optimize(size)]
#[inline(never)]
fn verify_on_worker(
    signed: Signed,
    net: &crate::chan::Sender<crate::http::NetCommand>,
) -> std::result::Result<(), Option<&'static str>> {
    crate::ota::verify_www_on_worker(signed, net).map_err(|e| {
        e.map(|e| match e {
            opendmd_core::ota::OtaError::UnknownKey(_) => "signed with a key this player does not know",
            _ => "bad signature: not signed by this firmware's key, or the pack was changed",
        })
    })?;
    remember(&signed);
    Ok(())
}

/// [`Verify`] at boot: the digest is the one [`remember`]ed.
#[optimize(size)]
#[inline(never)]
fn verify_remembered(signed: Signed) -> std::result::Result<(), Option<&'static str>> {
    // SAFETY: see [`verified_word`].
    let same = unsafe { verified_word(0).read_volatile() } == VERIFIED_MAGIC
        && signed.message[9..].chunks(4).enumerate().all(|(i, w)| {
            // SAFETY: as above.
            let have = unsafe { verified_word(i + 1).read_volatile() };
            have == u32::from_le_bytes([w[0], w[1], w[2], w[3]])
        });
    if same {
        Ok(())
    } else {
        Err(Some("not the pack this panel checked before the restart"))
    }
}

/// The staged pack on the card, read by offset. **A read the card fails is
/// tried again** ([`config::WWW_CARD_READ_TRIES`] in all, a pause between):
/// a 64 GB card at 40 MHz failed one 4 KB read with a data CRC error in a
/// pass that read the rest fine. FATFS remembers a disk error in the open
/// file (`FIL.err`) and fails every later call on it, so each retry opens
/// the file again and seeks. One open file and the caller's buffer: no heap
/// of its own beyond FATFS's per-file state.
struct Staged {
    file: Option<std::fs::File>,
    /// Where the open file's position is.
    pos: u64,
    /// Reads that needed more than one attempt.
    retried: u32,
}

impl Staged {
    fn new() -> Staged {
        Staged {
            file: None,
            pos: 0,
            retried: 0,
        }
    }

    fn try_read(&mut self, off: u64, out: &mut [u8]) -> std::io::Result<()> {
        use std::io::{Read, Seek, SeekFrom};
        let f = match &mut self.file {
            Some(f) => f,
            None => {
                self.pos = 0;
                self.file.insert(std::fs::File::open(STAGED)?)
            }
        };
        if self.pos != off {
            f.seek(SeekFrom::Start(off))?;
            self.pos = off;
        }
        f.read_exact(out)?;
        self.pos += out.len() as u64;
        Ok(())
    }

    /// Fill `out` from byte `off` of the staged pack.
    fn read_at(&mut self, off: usize, out: &mut [u8]) -> std::result::Result<(), String> {
        let tries = config::WWW_CARD_READ_TRIES;
        let mut attempt = 1;
        loop {
            let err = match self.try_read(off as u64, out) {
                Ok(()) => {
                    if attempt > 1 {
                        self.retried += 1;
                        log::info!("www: the staged pack at byte {off} read on attempt {attempt}");
                    }
                    return Ok(());
                }
                Err(e) => e,
            };
            // The handle is poisoned (see above): open it again next time.
            self.file = None;
            if attempt >= tries {
                // One failure for the card's health count, however many
                // attempts it took: retries are not more evidence.
                crate::sdhealth::read_failed(crate::sdhealth::Kind::Serve, true, STAGED, &err);
                return Err(format!(
                    "reading the staged pack from the card at byte {off} failed {tries} times: {err}"
                ));
            }
            log::warn!(
                "www: reading the staged pack at byte {off} failed (attempt {attempt} of {tries}): {err}; trying again"
            );
            std::thread::sleep(config::WWW_CARD_RETRY_PAUSE * attempt);
            attempt += 1;
        }
    }

    /// The whole staged pack read back and checked against `header`: its
    /// size, the CRC of everything after the header, and its signature
    /// (`verify`; the bytes hashed as they are read for the CRC).
    #[optimize(size)]
    fn check(
        &mut self,
        head: &[u8; HEADER_LEN],
        header: &Header,
        buf: &mut [u8],
        verify: Verify,
    ) -> std::result::Result<(), Unfit> {
        let len = header.total_len as usize;
        let size = std::fs::metadata(STAGED)
            .map_err(|e| Unfit::Unreadable(format!("{STAGED}: {e}")))?
            .len();
        if size != len as u64 {
            return Err(Unfit::Bad(format!("{size} B staged, the pack is {len} B")));
        }
        let mut sig = PackHasher::new(len);
        sig.update(head);
        let crc = crc_of(|off, out| self.read_at(off, out), len, buf, &mut sig)
            .map_err(Unfit::Unreadable)?;
        if crc != header.crc32 {
            return Err(Unfit::Bad(format!(
                "CRC mismatch: the header says {:08x}, the staged pack is {crc:08x}",
                header.crc32
            )));
        }
        let signed = sig.finish().map_err(|e| Unfit::Bad(e.to_string()))?;
        check_signature(verify, signed).map_err(|(why, status)| match status {
            Some((422, _)) => Unfit::Bad(why),
            _ => Unfit::Busy(why),
        })
    }
}

/// Why a staged pack cannot be written.
enum Unfit {
    /// It is not the pack its header describes, or not signed (the
    /// client's fault: 422).
    Bad(String),
    /// The card would not read it back (the device's: 500).
    Unreadable(String),
    /// The signature could not be checked just now (503).
    Busy(String),
}

/// CRC-32 of the `len - HEADER_LEN` bytes after the header, from `read`,
/// each piece through `sig` too (the signature's hash).
fn crc_of<E>(
    mut read: impl FnMut(usize, &mut [u8]) -> std::result::Result<(), E>,
    len: usize,
    buf: &mut [u8],
    sig: &mut PackHasher,
) -> std::result::Result<u32, E> {
    let mut crc = opendmd_core::www_pack::Crc32::new();
    let mut off = HEADER_LEN;
    while off < len {
        let n = (len - off).min(buf.len());
        read(off, &mut buf[..n])?;
        crc.update(&buf[..n]);
        sig.update(&buf[..n]);
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
/// flash always means a whole pack whose CRC verified (and whose signature
/// verified on the card, [`Staged::check`]). A failure at any
/// point leaves the header erased.
#[optimize(size)] // (CPU0 / boot / config: no frame runs it — console-c)
#[inline(never)]
fn write_pack(
    part: &sys::esp_partition_t,
    head: &[u8; HEADER_LEN],
    header: &Header,
    len: usize,
    body: &mut Staged,
    buf: &mut [u8],
) -> std::result::Result<(), Failure> {
    let flash_fail = |what: String| (what, INTERNAL);
    let mut off = HEADER_LEN;
    let mut erased = 0usize;
    while off < len {
        let n = (len - off).min(buf.len());
        body.read_at(off, &mut buf[..n]).map_err(|e| (e, INTERNAL))?;
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

/// Receive the whole pack onto the card ([`STAGED`], the header first) and
/// read it back, checking its size and CRC, retrying what the card fails
/// to read ([`Staged::read_at`]). `Ok(Err(why))`: the staging file could
/// not be created (write straight to flash instead). `Ok(Ok(staged))`:
/// staged and good. `Err`: answered (or the client went away); nothing in
/// flash has been touched.
#[allow(clippy::type_complexity)]
#[optimize(size)] // (CPU0 / boot / config: no frame runs it — console-c)
#[inline(never)]
fn stage_on_card(
    up: &mut Upload,
    head: &[u8; HEADER_LEN],
    header: &Header,
    buf: &mut [u8],
    verify: Verify,
) -> std::result::Result<std::result::Result<Staged, String>, Result<()>> {
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
    if let Err(e) = files::stream_body_after(up, STAGED, head) {
        // Logged and removed by stream_body_after; the caller of `receive`
        // answers an `UploadAborted` with its status.
        return Err(Err(e));
    }
    let t = Instant::now();
    let mut staged = Staged::new();
    match staged.check(head, header, buf, verify) {
        Ok(()) => {
            log::info!(
                "www: the staged pack checks out (read back in {} ms{})",
                t.elapsed().as_millis(),
                match staged.retried {
                    0 => String::new(),
                    1 => ", 1 read needed another attempt".to_string(),
                    n => format!(", {n} reads needed another attempt"),
                }
            );
            Ok(Ok(staged))
        }
        Err(unfit) => {
            drop(staged);
            let _ = std::fs::remove_file(STAGED);
            let (code, line, why) = match unfit {
                Unfit::Bad(why) => (422, "422 Unprocessable Entity", why),
                Unfit::Unreadable(why) => (500, "500 Internal Server Error", why),
                Unfit::Busy(why) => (503, "503 Service Unavailable", why),
            };
            log::warn!("PUT /api/www: refused, the web app in flash is unchanged: {why}");
            Err(up.error(
                code,
                line,
                &format!("{why}; the web app in flash is unchanged"),
            ))
        }
    }
}

/// `PUT /api/www`, on the stream task: write a whole pack into the partition
/// and serve it.
///
/// The pack is received onto the card first (`/update/www.pack.part`) and
/// read back whole, its size, CRC and signature checked, while the old pack
/// stays mapped: an upload cut short, a file that is not a good signed pack,
/// or a card that will not read it back (after retries) is answered (408,
/// 422, 500, …) with the web app in flash untouched. Only then is the old
/// pack unmapped and the partition rewritten from the card, ~3 s, header
/// last ([`write_pack`]), each card read retried as in the check. If that
/// still fails, the whole write is tried once more from the start; if that
/// fails too, the verified copy stays on the card and the next boot writes
/// it ([`restore_from_card`]). **Without a card** (`no_card`: none, or one
/// that cannot be read), or one on which the staging file cannot be made,
/// it is refused (503, 500) and the app in flash stays: the signature is
/// checked on the staged copy (until 1.0.0-beta.15 the body then went to
/// flash as it arrived). The header is checked before anything happens
/// (400; 422 unsigned).
pub fn receive(
    up: &mut Upload,
    no_card: Option<&'static str>,
    net: &crate::chan::Sender<crate::http::NetCommand>,
) -> Result<()> {
    let _one = UPDATE.lock().unwrap_or_else(|p| p.into_inner());
    // `staged` describes this attempt from here on: null until it gets as
    // far as choosing.
    STAGED_LAST.store(0, Relaxed);
    // Staged on the card or not at all: the signature is checked on the
    // staged copy before flash is touched (docs/www-pack.md "Signature").
    if no_card.is_some() {
        up.header("Retry-After", "10");
        return up.error(
            503,
            "503 Service Unavailable",
            "no card (or one that cannot be read): a web app update is staged on the card, insert one",
        );
    }
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
    // Only a signed pack, refused here before anything is touched (the
    // signature itself is checked once the whole pack is in).
    if !header.signed() {
        log::warn!("PUT /api/www: refused, pack '{}' is not signed", header.version());
        return up.error(
            422,
            "422 Unprocessable Entity",
            "not signed: this player takes only a release's web app pack (signed); the web app in flash is unchanged",
        );
    }
    let verify = Verify::Worker(net);
    log::info!(
        "www: receiving pack '{}', {len} B, {} entries",
        header.version(),
        header.count,
    );

    let started = Instant::now();
    STAGED_LAST.store(2, Relaxed);
    let mut body = match stage_on_card(up, &head, &header, &mut buf, verify) {
        Ok(Ok(s)) => s,
        Ok(Err(why)) => {
            log::warn!("PUT /api/www: cannot stage on the card ({why}); the web app in flash is unchanged");
            return up.error(500, "500 Internal Server Error", &format!("cannot write to the card: {why}"));
        }
        Err(answered) => return answered,
    };

    let mut map = MAP.lock().unwrap_or_else(|p| p.into_inner());
    // Unmap before the first erase: nothing may read the old pack from here.
    set_mapping(&mut map, None);
    set_info(Info {
        error: Some("the web app is being updated; reload in a moment".into()),
        ..broken(String::new())
    });
    let mut written = write_pack(part, &head, &header, len, &mut body, &mut buf);
    drop(body);
    {
        if let Err((why, _)) = &written {
            // The partition is erased by now. The copy on the card checked
            // out a moment ago, so try the whole write once more before
            // giving up on it.
            log::warn!(
                "PUT /api/www: writing the pack from the card failed after {:.1} s ({why}); writing it again from the start",
                started.elapsed().as_secs_f32()
            );
            std::thread::sleep(config::WWW_CARD_RETRY_PAUSE * config::WWW_CARD_READ_TRIES);
            written = write_pack(
                part,
                &head,
                &header,
                len,
                &mut Staged::new(),
                &mut buf,
            );
        }
    }
    drop(buf);

    if let Err((why, status)) = written {
        log::warn!(
            "PUT /api/www: writing the pack failed after {:.1} s ({}): {why}",
            started.elapsed().as_secs_f32(),
            "from the card; the checked copy stays there and is written at the next boot"
        );
        let error = format!(
            "the last web app update failed while writing it to flash ({why}); the checked pack stays on the card and is written at the next restart, or `make www-push` again"
        );
        set_info(broken(error.clone()));
        return match status {
            Some((code, line)) => up.error(code, line, &error),
            None => Ok(()),
        };
    }
    let _ = std::fs::remove_file(STAGED);

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
            set_mapping(&mut map, Some(mapping));
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

/// At boot, once the card is mounted and [`init`] has looked at the flash:
/// a pack staged on the card is left there only when a `PUT /api/www`
/// failed after it had erased the partition (see [`receive`]). With no good
/// pack in flash it is checked (size, CRC) and written, as `PUT` would;
/// otherwise it is stale and removed. On the boot task (CPU0); the erase
/// pauses are the same. Never fails the boot.
pub fn restore_from_card() {
    if std::fs::metadata(STAGED).is_err() {
        return;
    }
    if info().ok {
        let _ = std::fs::remove_file(STAGED);
        log::info!("www: removed a staged pack left on the card (the web app in flash is good)");
        return;
    }
    let _one = UPDATE.lock().unwrap_or_else(|p| p.into_inner());
    let t = Instant::now();
    // `Unfit::Bad`: the file goes; anything else, it stays for next time.
    let restored = (|| -> std::result::Result<String, Unfit> {
        let part = partition().ok_or(Unfit::Unreadable("no www partition".into()))?;
        let mut buf =
            files::chunk_buffer().ok_or(Unfit::Unreadable("no heap for a buffer".into()))?;
        let mut staged = Staged::new();
        let mut head = [0u8; HEADER_LEN];
        staged.read_at(0, &mut head).map_err(Unfit::Unreadable)?;
        let header =
            Header::parse(&head).map_err(|e| Unfit::Bad(format!("not a www pack: {e}")))?;
        let len = header.total_len as usize;
        if len > part.size as usize {
            return Err(Unfit::Bad(format!(
                "{len} B, the partition is {} B",
                part.size
            )));
        }
        log::warn!(
            "www: no web app in flash, and pack '{}' ({len} B) staged on the card by an update that failed: checking it",
            header.version()
        );
        if !header.signed() {
            return Err(Unfit::Bad("not signed".into()));
        }
        staged.check(&head, &header, &mut buf, Verify::Remembered)?;
        let mut map = MAP.lock().unwrap_or_else(|p| p.into_inner());
        set_mapping(&mut map, None);
        write_pack(part, &head, &header, len, &mut staged, &mut buf)
            .map_err(|(why, _)| Unfit::Unreadable(why))?;
        let (mapping, info) = load().map_err(Unfit::Unreadable)?;
        set_mapping(&mut map, Some(mapping));
        drop(map);
        let v = info.version.clone().unwrap_or_default();
        set_info(info);
        Ok(v)
    })();
    match restored {
        Ok(v) => {
            let _ = std::fs::remove_file(STAGED);
            log::info!(
                "www: pack '{v}' written from the card in {:.1} s; the web app is back",
                t.elapsed().as_secs_f32()
            );
        }
        Err(Unfit::Bad(why)) => {
            let _ = std::fs::remove_file(STAGED);
            log::warn!("www: the pack staged on the card is not usable ({why}); removed it");
        }
        Err(Unfit::Unreadable(why) | Unfit::Busy(why)) => {
            log::warn!(
                "www: the pack staged on the card could not be written ({why}); it stays for the next boot"
            );
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

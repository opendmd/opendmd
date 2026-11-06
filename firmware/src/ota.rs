//! Firmware updates over the LAN (`docs/ota.md`, steps 1, 2 and 5).
//!
//! The running firmware cannot overwrite the partition it runs from, and two
//! app slots do not fit in 4 MB next to the web app, so an update is staged
//! on the card: `PUT /api/firmware` ([`receive`], on the stream task like
//! every upload) writes the signed bundle to `/update/opendmd.bin.part`,
//! hashing the image as it goes and keeping the 140-byte header in memory,
//! then checks it (`opendmd_core::ota`: the header, that it is for this
//! image's flash layout ([`crate::state::FLASH_VARIANT`] and its table,
//! [`crate::state::flash_layout`]: a 4mb player refuses an 8mb bundle and
//! the reverse, a table v3 player a v2 bundle; a v2 player, beta.15 and
//! earlier, does not know the v3 magic at all), that the image fits the factory
//! partition, the ESP image magic 0xE9, the SHA-256, the Ed25519 signature
//! against [`PUBLIC_KEY`], and that the image's app descriptor is a build
//! for the same layout). The layout and the signature are checked as soon
//! as the header is in, before anything is written to the card. Any failure is
//! a 422 and the card is as it was. Success renames the file to
//! `/update/opendmd.bin`, writes `/update/pending.txt` (the version), points
//! otadata at `ota_0` (the updater, `ota/updater`) and restarts once the
//! reply is out. The updater copies the image into `factory`, sets factory
//! to boot, renames the file to `done-<version>.bin` and restarts into the
//! new firmware.
//!
//! [`after_boot`] (the boot task, once the card is mounted) tidies up: a
//! `done-*.bin` is logged as the version change and deleted with
//! `pending.txt`; a staged `opendmd.bin` that is still there means the
//! updater did not install it (it logs why in `/update/log.txt`), and it is
//! deleted too, since a firmware never boots between the reply and the
//! updater otherwise.
//!
//! The panel: from the accepted header to the restart it shows a still
//! message ([`crate::show::Updating`]: "Updating", then "Installing" once
//! staged), the file on screen let go; any failure takes it down and
//! playback resumes. The updater's copy runs with the panel dark (it has no
//! panel driver), and the new firmware's first boot says "Updated to v…".
//!
//! Heap: one [`config::IO_CHUNK`] buffer and one open file, the GIF upload
//! path's; the SHA-256 state is ~100 B, the header 140 B, on the stream
//! task's stack. No buffer holds the image.

use anyhow::Result;
use esp_idf_sys as sys;
use opendmd_core::ota::{
    check_app_desc, check_floor, check_key_id, verify_header_any, Header, ImageHasher, KeyTail, Meta,
    OtaError, HEADER_LEN,
};
use std::fs;
use std::io::{Read, Seek, SeekFrom, Write};

use crate::chan as mpsc;
use crate::config;
use crate::files;
use crate::http::NetCommand;
use crate::httpd::{json_string, BodyError, Respond, Upload};

/// The Ed25519 public key bundles must be signed with
/// (`tools/fw-sign.py keygen`; the private half never enters the repo).
pub const PUBLIC_KEY: &[u8; 32] = include_bytes!("../keys/opendmd.pub");

/// Every key this firmware trusts, by id (docs/ota.md "Keys"): a bundle's
/// key trailer and a www pack's trailer name one. A rotated key is added
/// here with the next id, in a release signed with the old one.
pub const KEYS: &[(u8, &[u8; 32])] = &[(0, PUBLIC_KEY)];

/// The rollback floor (docs/ota.md): built with `OPENDMD_OTA_FLOOR=<version>`
/// (in the environment of the firmware build; no Makefile variable sets it), a bundle older than it is refused.
/// Off by default: any validly signed bundle goes, older ones too.
const OTA_FLOOR: Option<&str> = option_env!("OPENDMD_OTA_FLOOR");

const BUNDLE: &str = "/sdcard/update/opendmd.bin";
const BUNDLE_PART: &str = "/sdcard/update/opendmd.bin.part";
const PENDING: &str = "/sdcard/update/pending.txt";
const LOG: &str = "/sdcard/update/log.txt";
/// The longest `last` line `GET /api/firmware` reports.
const LAST_MAX: usize = 200;
/// The most of the log's end `GET /api/firmware` reports as `log`.
const LOG_TAIL: usize = 1024;

fn app_partition(subtype: sys::esp_partition_subtype_t) -> Option<&'static sys::esp_partition_t> {
    // SAFETY: a read-only query of the partition table; the entry lives for
    // the life of the program.
    unsafe {
        sys::esp_partition_find_first(
            sys::esp_partition_type_t_ESP_PARTITION_TYPE_APP,
            subtype,
            std::ptr::null(),
        )
        .as_ref()
    }
}

fn factory() -> Option<&'static sys::esp_partition_t> {
    app_partition(sys::esp_partition_subtype_t_ESP_PARTITION_SUBTYPE_APP_FACTORY)
}

fn updater() -> Option<&'static sys::esp_partition_t> {
    app_partition(sys::esp_partition_subtype_t_ESP_PARTITION_SUBTYPE_APP_OTA_0)
}

/// Append one line to `/update/log.txt` (best effort).
fn log_line(line: &str) {
    let r = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(LOG)
        .and_then(|mut f| writeln!(f, "firmware {}: {line}", crate::state::running_version()));
    if let Err(e) = r {
        log::warn!("ota: {LOG}: {e}");
    }
}

/// How long the upload waits for the network worker to check a signature
/// (it may be testing Follow RePlayOS, or in a FAT index slice: seconds at
/// most; a cache count, which was tens of seconds on the rig, stops for it,
/// see [`verify_waiting`]).
const VERIFY_WAIT: std::time::Duration = std::time::Duration::from_secs(30);

/// A signature check is queued for the network worker. Its long jobs (the
/// cache count: a bucket walk and manifest passes, ~29 s on the rig's card)
/// look at this and give way: they stop, queue themselves again behind the
/// check and start over once it is done. The check itself is ~50 ms. Before
/// this, a firmware update sent while the web app's Settings had just asked
/// for a count waited behind it and got a 503 ("the signature could not be
/// checked just now") until a retry (the rig, 2026-11-03).
static VERIFY_WAITING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// See [`VERIFY_WAITING`].
pub fn verify_waiting() -> bool {
    VERIFY_WAITING.load(std::sync::atomic::Ordering::Relaxed)
}

/// The network worker's side of `NetCommand::VerifyBundle`.
pub fn serve_verify(head: &[u8; HEADER_LEN]) -> Result<(Meta, u8), OtaError> {
    VERIFY_WAITING.store(false, std::sync::atomic::Ordering::Relaxed);
    verify_signature(head)
}

/// The network worker's side of `NetCommand::VerifyWww`: a web app pack's
/// signature (`www::receive`), for the same reason as a bundle's.
/// (Its reply goes on a bundle's kind of channel, `Ok` with an empty
/// `Meta`: one channel type less in the image.)
pub fn serve_verify_www(signed: &opendmd_core::ota::www::Signed) -> Result<(Meta, u8), OtaError> {
    VERIFY_WAITING.store(false, std::sync::atomic::Ordering::Relaxed);
    opendmd_core::ota::www::verify(signed, KEYS)?;
    Ok((
        Meta {
            variant: "",
            version: String::new(),
            length: 0,
            sha256: [0; 32],
        },
        signed.key_id,
    ))
}

/// Have the network worker check a www pack's signature (the stream task's
/// stack cannot: see [`verify_signature`]). `Err(None)`: it did not answer.
pub fn verify_www_on_worker(
    signed: opendmd_core::ota::www::Signed,
    net: &mpsc::Sender<NetCommand>,
) -> Result<(), Option<OtaError>> {
    let (tx, rx) = mpsc::channel();
    VERIFY_WAITING.store(true, std::sync::atomic::Ordering::Relaxed);
    let sent = net.send(NetCommand::VerifyWww {
        signed: Box::new(signed),
        reply: tx,
    });
    let got = sent.ok().and_then(|()| rx.recv_timeout(VERIFY_WAIT).ok());
    VERIFY_WAITING.store(false, std::sync::atomic::Ordering::Relaxed);
    got.ok_or(None)?.map(|_| ()).map_err(Some)
}

/// Parse a bundle header and check its signature against [`PUBLIC_KEY`].
/// **On the network worker** (`NetCommand::VerifyBundle`): Ed25519's
/// variable-base multiplication keeps ~7-8.5 KB of frames on the stack
/// (curve25519-dalek's NAF lookup table, 2.1 KB, and its caller, 2.6 KB,
/// among them; measured from the ELF's `entry` sizes), more than the stream
/// task's 7 KB, and the worker's 11 KB has it (`config::NET_WORKER_STACK`). ~50 ms of CPU0.
#[inline(never)]
pub fn verify_signature(head: &[u8; HEADER_LEN]) -> Result<(Meta, u8), OtaError> {
    let header = Header::parse(head)?;
    verify_header_any(&header, KEYS)
}

/// The header checks that need nothing but the header: shape, the flash
/// layout, signature (on the network worker), that the image fits, that the
/// body is exactly header + image, the rollback floor if built with one.
/// The trusted header and the id of the key that verified it. `Err(None)`:
/// the worker did not answer.
fn check_header(
    head: &[u8],
    body_len: u64,
    factory_size: u32,
    net: &mpsc::Sender<NetCommand>,
) -> Result<(Meta, u8), Option<OtaError>> {
    let mut header = Box::new([0u8; HEADER_LEN]);
    if head.len() < HEADER_LEN {
        return Err(Some(OtaError::Truncated { have: head.len() }));
    }
    header.copy_from_slice(&head[..HEADER_LEN]);
    // The cheap checks first, here: no round trip for a file that is not a
    // bundle at all, or one for another flash layout or partition table.
    Header::parse(&header[..])?.check_layout(crate::state::FLASH_VARIANT, crate::state::flash_layout())?;
    let (tx, rx) = mpsc::channel();
    VERIFY_WAITING.store(true, std::sync::atomic::Ordering::Relaxed);
    let sent = net.send(NetCommand::VerifyBundle { header, reply: tx });
    let got = sent.ok().and_then(|()| rx.recv_timeout(VERIFY_WAIT).ok());
    VERIFY_WAITING.store(false, std::sync::atomic::Ordering::Relaxed);
    let (meta, key_id) = got.ok_or(None)??;
    meta.check_fits(factory_size)?;
    // A const: with no floor (the default) none of this is in the image.
    if let Some(floor) = OTA_FLOOR {
        check_floor(&meta.version, Some(floor))?;
    }
    let image = body_len.saturating_sub(HEADER_LEN as u64);
    if u64::from(meta.length) != image {
        return Err(Some(OtaError::LengthMismatch {
            header: meta.length,
            actual: image,
        }));
    }
    Ok((meta, key_id))
}

/// Refuse the bundle: 422 at once, however much of the body is still to
/// come (a wrong key is known at the 140-byte header). The [`Upload`] reads
/// off what the client still sends while it stops, then closes (see there).
fn refuse(up: &mut Upload, why: &str) -> Result<()> {
    log::warn!(
        "PUT /api/firmware: refused at {} of {} B after {:.1} s: {why}",
        up.received(),
        up.content_len(),
        up.elapsed().as_secs_f32()
    );
    up.error(422, "422 Unprocessable Entity", why)
}

/// The body failed (cut short, timed out): nothing staged.
fn aborted(up: &mut Upload, e: &anyhow::Error) -> Result<()> {
    let _ = fs::remove_file(BUNDLE_PART);
    log::warn!(
        "PUT /api/firmware: upload aborted at {} of {} B after {:.1} s: {e:#}",
        up.received(),
        up.content_len(),
        up.elapsed().as_secs_f32()
    );
    match e.downcast_ref::<BodyError>().and_then(|b| b.status) {
        Some((code, line)) => up.error(
            code,
            line,
            &format!(
                "upload aborted at {} of {} B: {e:#}",
                up.received(),
                up.content_len()
            ),
        ),
        None => Ok(()),
    }
}

/// `PUT /api/firmware`, on the stream task. Restarts on success (never
/// returns then).
pub fn receive(up: &mut Upload, card_mounted: bool, net: &mpsc::Sender<NetCommand>) -> Result<()> {
    let (Some(factory), Some(_)) = (factory(), updater()) else {
        return up.error(
            500,
            "500 Internal Server Error",
            "this flash has no updater partition (layout v1): install a current release over USB once (opendmd.dev/install)",
        );
    };
    let len = up.content_len();
    if len == 0 {
        return up.error(411, "411 Length Required", "Content-Length required");
    }
    let max = HEADER_LEN as u64 + u64::from(factory.size);
    if len > max {
        return up.error(
            413,
            "413 Payload Too Large",
            &format!("the bundle is {len} B, at most {max} B fits the firmware partition"),
        );
    }
    if !card_mounted {
        up.header("Retry-After", "10");
        return up.error(
            503,
            "503 Service Unavailable",
            "no card (or one that cannot be read): a firmware update is staged on the card, insert one",
        );
    }

    // The header, and its signature, before anything touches the card.
    let mut head = [0u8; HEADER_LEN];
    let mut have = 0;
    while have < HEADER_LEN {
        match up.read(&mut head[have..]) {
            Ok(0) => break,
            Ok(n) => have += n,
            Err(e) => return aborted(up, &e),
        }
    }
    let (meta, key_id) = match check_header(&head[..have], len, factory.size, net) {
        Ok(m) => m,
        Err(Some(e)) => return refuse(up, &e.to_string()),
        Err(None) => {
            log::warn!("PUT /api/firmware: the network worker did not check the signature in time");
            up.header("Retry-After", "10");
            return up.error(
                503,
                "503 Service Unavailable",
                "busy: the signature could not be checked just now; retry shortly",
            );
        }
    };
    log::info!(
        "PUT /api/firmware: bundle {} ({} B image), signature ok (key {key_id}); staging it on the card",
        meta.version,
        meta.length
    );

    let Some(mut buf) = files::chunk_buffer() else {
        up.header("Retry-After", "2");
        return up.error(
            503,
            "503 Service Unavailable",
            "the player needs the heap right now; retry shortly",
        );
    };
    // From here the panel says an update is coming in, until this returns
    // (refused, cut short, a card error: playback resumes) or the restart.
    let panel = crate::show::Updating::begin(&meta.version);
    let staged = (|| -> std::io::Result<fs::File> {
        fs::create_dir_all(format!("{}/update", config::SD_MOUNT))?;
        // A previous staging that never ran is superseded.
        let _ = fs::remove_file(PENDING);
        let _ = fs::remove_file(BUNDLE);
        let mut f = fs::File::create(BUNDLE_PART)?;
        f.write_all(&head)?;
        Ok(f)
    })();
    let mut file = match staged {
        Ok(f) => f,
        Err(e) => {
            log::warn!("PUT /api/firmware: {BUNDLE_PART}: {e}");
            return up.error(
                500,
                "500 Internal Server Error",
                &format!("cannot write to the card: {e}"),
            );
        }
    };
    let mut hasher = ImageHasher::new();
    let mut tail = KeyTail::default();
    let outcome: std::result::Result<(), anyhow::Error> = loop {
        let n = match up.read(&mut buf) {
            Ok(0) => break file.flush().map_err(Into::into),
            Ok(n) => n,
            Err(e) => break Err(e),
        };
        if let Err(e) = files::wait_for_heap() {
            break Err(e);
        }
        hasher.update(&buf[..n]);
        tail.update(&buf[..n]);
        if let Err(e) = file.write_all(&buf[..n]) {
            break Err(anyhow::Error::from(e).context("writing to the card"));
        }
        std::thread::yield_now();
    };
    drop(file);
    drop(buf);
    if let Err(e) = outcome {
        if e.downcast_ref::<BodyError>().is_some() {
            return aborted(up, &e);
        }
        let _ = fs::remove_file(BUNDLE_PART);
        log::warn!("PUT /api/firmware: {e:#}");
        return up.error(500, "500 Internal Server Error", &format!("{e:#}"));
    }

    let (got, first, prefix) = (hasher.len(), hasher.first(), *hasher.prefix());
    if let Err(e) = meta
        .check_image(got, first, &hasher.finish())
        .and_then(|()| check_app_desc(&prefix, meta.variant))
        .and_then(|()| check_key_id(key_id, tail.key_id()))
    {
        let _ = fs::remove_file(BUNDLE_PART);
        return refuse(up, &e.to_string());
    }
    if let Err(e) = files::replace(BUNDLE_PART, BUNDLE)
        .and_then(|()| fs::write(PENDING, format!("{}\n", meta.version)).map_err(Into::into))
    {
        let _ = fs::remove_file(BUNDLE);
        let _ = fs::remove_file(PENDING);
        log::warn!("PUT /api/firmware: {e:#}");
        return up.error(500, "500 Internal Server Error", &format!("{e:#}"));
    }

    // SAFETY: a partition entry from the table; IDF verifies the updater's
    // image before it writes otadata, and refuses one that does not verify.
    let err = unsafe { sys::esp_ota_set_boot_partition(updater().unwrap_or(factory)) };
    if err != sys::ESP_OK {
        let _ = fs::remove_file(BUNDLE);
        let _ = fs::remove_file(PENDING);
        log::warn!("PUT /api/firmware: esp_ota_set_boot_partition(ota_0) failed ({err})");
        return up.error(
            500,
            "500 Internal Server Error",
            &format!(
                "the updater in flash cannot be started (error {err}): install a v2 release over USB"
            ),
        );
    }
    panel.installing();
    log::info!(
        "PUT /api/firmware: {} staged and verified in {:.1} s; restarting into the updater",
        meta.version,
        up.elapsed().as_secs_f32()
    );
    log_line(&format!(
        "staged {} ({} B), restarting into the updater",
        meta.version, meta.length
    ));
    // Restart even if the reply cannot be sent (the client gone): otadata
    // already points at the updater, and staying up would leave the panel's
    // message to come down over an update the next restart installs anyway.
    if let Err(e) = up.json(&format!(
        r#"{{"ok":true,"version":{},"rebooting":true}}"#,
        json_string(&meta.version)
    )) {
        log::warn!("PUT /api/firmware: the reply did not go out ({e:#}); restarting into the updater anyway");
    }
    crate::http::reboot_after_response(opendmd_core::restarts::Reason::Ota)
}

/// The end of `/update/log.txt`: at most [`LOG_TAIL`] bytes, from the
/// first whole line in them. One small read on the card.
fn log_tail() -> Option<String> {
    let mut f = fs::File::open(LOG).ok()?;
    let size = f.seek(SeekFrom::End(0)).ok()?;
    let from = size.saturating_sub(LOG_TAIL as u64);
    f.seek(SeekFrom::Start(from)).ok()?;
    let mut tail = Vec::with_capacity((size - from) as usize);
    f.take(LOG_TAIL as u64).read_to_end(&mut tail).ok()?;
    let text = String::from_utf8_lossy(&tail);
    // A cut first line is dropped, unless it is all there is.
    let text = match (from > 0, text.find('\n')) {
        (true, Some(i)) if i + 1 < text.len() => &text[i + 1..],
        _ => &text[..],
    };
    let text = text.trim_end();
    (!text.is_empty()).then(|| text.to_string())
}

/// The last line of a [`log_tail`], at most [`LAST_MAX`] characters.
fn last_line(tail: &str) -> Option<String> {
    tail.lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .map(|l| l.chars().take(LAST_MAX).collect())
}

fn pending_version() -> Option<String> {
    let s = fs::read_to_string(PENDING).ok()?;
    let v = s.lines().next()?.trim();
    (!v.is_empty() && v.len() <= 32).then(|| v.to_string())
}

/// The flash chip's real size in bytes (`esp_flash_get_physical_size`: its
/// JEDEC ID), 0 if it cannot be read. Asked once (a flash command, a few
/// tens of µs with the cache off), then kept. A 4mb image on an 8 MB board
/// runs (with the smaller layout); this is how the web app can tell.
fn flash_size() -> u32 {
    static SIZE: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *SIZE.get_or_init(|| {
        let mut size = 0u32;
        // SAFETY: the default chip (null), an out-parameter on our stack.
        let err = unsafe { sys::esp_flash_get_physical_size(std::ptr::null_mut(), &mut size) };
        if err == sys::ESP_OK {
            size
        } else {
            0
        }
    })
}

/// `GET /api/firmware`: `{"version","build","variant","layout","flash_size","pending","last","log"}`.
/// Reads two small files on the card when it is mounted (at most
/// [`LOG_TAIL`] bytes of the log).
pub fn info_json(card_mounted: bool) -> String {
    let (pending, log) = if card_mounted {
        (pending_version(), log_tail())
    } else {
        (None, None)
    };
    let last = log.as_deref().and_then(last_line);
    let opt = |v: Option<String>| v.as_deref().map_or_else(|| "null".into(), json_string);
    let (mhz, mode) = flash_clock();
    format!(
        r#"{{"version":{},"build":{},"variant":{},"layout":{},"features":{},"flash_size":{},"flash":{{"mhz":{mhz},"mode":"{mode}"}},"pending":{},"last":{},"log":{}}}"#,
        json_string(crate::state::firmware_version()),
        json_string(&crate::state::running_version()),
        json_string(crate::state::FLASH_VARIANT),
        crate::state::flash_layout(),
        crate::state::features_json(),
        flash_size(),
        opt(pending),
        opt(last),
        opt(log),
    )
}

/// The clock and read mode the flash runs at now, read back from SPI0
/// (the cache's port): MHz (80 when its clock equals the 80 MHz APB, else
/// 80 / ((pre + 1) × (n + 1))) and `qio`, `qout`, `dio`, `dout`, `fast` or
/// `slow`. The app sets the clock at start-up from its own image header
/// (`esp_system/port/cpu_start.c`), the read mode is the bootloader's.
pub fn flash_clock() -> (u32, &'static str) {
    const SPI0: usize = 0x3ff4_3000; // DR_REG_SPI0_BASE
    const CTRL: usize = SPI0 + 0x08; // SPI_CTRL_REG(0)
    const CLOCK: usize = SPI0 + 0x18; // SPI_CLOCK_REG(0)
    // SAFETY: two aligned reads of SPI0's registers (memory-mapped, always
    // readable; nothing is written).
    let (ctrl, clock) = unsafe {
        (
            core::ptr::read_volatile(CTRL as *const u32),
            core::ptr::read_volatile(CLOCK as *const u32),
        )
    };
    let mhz = if clock & (1 << 31) != 0 {
        80
    } else {
        let pre = (clock >> 18) & 0x1fff;
        let n = (clock >> 12) & 0x3f;
        80 / ((pre + 1) * (n + 1))
    };
    let mode = if ctrl & (1 << 13) == 0 {
        "slow"
    } else if ctrl & (1 << 24) != 0 {
        "qio"
    } else if ctrl & (1 << 23) != 0 {
        "dio"
    } else if ctrl & (1 << 20) != 0 {
        "qout"
    } else if ctrl & (1 << 14) != 0 {
        "dout"
    } else {
        "fast"
    };
    (mhz, mode)
}

/// At boot, once the card is mounted: account for an update that ran (or
/// did not). Logs only; never fails the boot.
pub fn after_boot() {
    let dir = format!("{}/update", config::SD_MOUNT);
    let Ok(entries) = fs::read_dir(&dir) else {
        return;
    };
    let mut done = Vec::new();
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if let Some(v) = name
            .strip_prefix("done-")
            .and_then(|r| r.strip_suffix(".bin"))
        {
            done.push((name.clone(), v.to_string()));
        }
    }
    let running = crate::state::firmware_version();
    for (name, v) in &done {
        if v == running {
            log::info!("firmware updated over the LAN to {v} (the updater installed it)");
        } else {
            log::warn!(
                "the updater installed {v}, but this firmware is {running} (the bundle's version is not the one its image was built with: FW_VERSION?)"
            );
        }
        log_line(&format!("booted after the updater installed {v}"));
        if let Err(e) = fs::remove_file(format!("{dir}/{name}")) {
            log::warn!("ota: removing {dir}/{name}: {e}");
        }
    }
    if !done.is_empty() {
        crate::restarts::booted_after_update();
        let _ = fs::remove_file(PENDING);
    }
    if fs::metadata(BUNDLE).is_ok() {
        let v = pending_version().unwrap_or_else(|| "?".into());
        log::warn!(
            "a staged firmware update ({v}) was not installed (see /update/log.txt on the card); removing it"
        );
        log_line(&format!(
            "the staged update to {v} was not installed; removed it"
        ));
        let _ = fs::remove_file(BUNDLE);
        let _ = fs::remove_file(PENDING);
    }
    let _ = fs::remove_file(BUNDLE_PART);
    // A web app pack staged on the card is `www::restore_from_card`'s.
}

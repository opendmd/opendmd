//! Firmware updates over the LAN (`docs/ota.md`, steps 1, 2 and 5).
//!
//! The running firmware cannot overwrite the partition it runs from, and two
//! app slots do not fit in 4 MB next to the web app, so an update is staged
//! on the card: `PUT /api/firmware` ([`receive`], on the stream task like
//! every upload) writes the signed bundle to `/update/opendmd.bin.part`,
//! hashing the image as it goes and keeping the 140-byte header in memory,
//! then checks it (`opendmd_core::ota`: the header, that the image fits the
//! factory partition, the ESP image magic 0xE9, the SHA-256 and the Ed25519
//! signature against [`PUBLIC_KEY`]). The signature is checked as soon as
//! the header is in, before anything is written to the card. Any failure is
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
//! Heap: one [`config::IO_CHUNK`] buffer and one open file, the GIF upload
//! path's; the SHA-256 state is ~100 B, the header 140 B, on the stream
//! task's stack. No buffer holds the image.

use anyhow::Result;
use esp_idf_sys as sys;
use opendmd_core::ota::{verify_header, Header, ImageHasher, Meta, OtaError, HEADER_LEN};
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

const BUNDLE: &str = "/sdcard/update/opendmd.bin";
const BUNDLE_PART: &str = "/sdcard/update/opendmd.bin.part";
const PENDING: &str = "/sdcard/update/pending.txt";
const LOG: &str = "/sdcard/update/log.txt";
/// The longest `last` line `GET /api/firmware` reports.
const LAST_MAX: usize = 200;

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
/// (it may be counting the cache or testing Follow RePlayOS).
const VERIFY_WAIT: std::time::Duration = std::time::Duration::from_secs(30);

/// Parse a bundle header and check its signature against [`PUBLIC_KEY`].
/// **On the network worker** (`NetCommand::VerifyBundle`): Ed25519's
/// variable-base multiplication keeps ~7-8.5 KB of frames on the stack
/// (curve25519-dalek's NAF lookup table, 2.1 KB, and its caller, 2.6 KB,
/// among them; measured from the ELF's `entry` sizes), more than the stream
/// task's 6 KB, and the worker's 12 KB has it. ~50 ms of CPU0.
#[inline(never)]
pub fn verify_signature(head: &[u8; HEADER_LEN]) -> Result<Meta, OtaError> {
    let header = Header::parse(head)?;
    verify_header(&header, PUBLIC_KEY)
}

/// The header checks that need nothing but the header: shape, signature
/// (on the network worker), that the image fits, that the body is exactly
/// header + image. `Err(None)`: the worker did not answer.
fn check_header(
    head: &[u8],
    body_len: u64,
    factory_size: u32,
    net: &mpsc::Sender<NetCommand>,
) -> Result<Meta, Option<OtaError>> {
    let mut header = Box::new([0u8; HEADER_LEN]);
    if head.len() < HEADER_LEN {
        return Err(Some(OtaError::Truncated { have: head.len() }));
    }
    header.copy_from_slice(&head[..HEADER_LEN]);
    // The cheap checks first, here: no round trip for a file that is not a
    // bundle at all.
    Header::parse(&header[..])?;
    let (tx, rx) = mpsc::channel();
    net.send(NetCommand::VerifyBundle { header, reply: tx })
        .map_err(|_| None)?;
    let meta = rx.recv_timeout(VERIFY_WAIT).map_err(|_| None)??;
    meta.check_fits(factory_size)?;
    let image = body_len.saturating_sub(HEADER_LEN as u64);
    if u64::from(meta.length) != image {
        return Err(Some(OtaError::LengthMismatch {
            header: meta.length,
            actual: image,
        }));
    }
    Ok(meta)
}

/// Refuse the bundle: the rest of the body is read and dropped first, so the
/// client (a browser's XHR in particular) gets the 422 rather than a reset.
fn refuse(up: &mut Upload, why: &str) -> Result<()> {
    log::warn!("PUT /api/firmware: refused: {why}");
    let mut sink = [0u8; 512];
    while let Ok(n) = up.read(&mut sink) {
        if n == 0 {
            break;
        }
    }
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
            "this flash has no updater partition (layout v1): install a v2 release over USB once (docs/deploying.md)",
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
    let meta = match check_header(&head[..have], len, factory.size, net) {
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
        "PUT /api/firmware: bundle {} ({} B image), signature ok; staging it on the card",
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

    let (got, first) = (hasher.len(), hasher.first());
    if let Err(e) = meta.check_image(got, first, &hasher.finish()) {
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
    log::info!(
        "PUT /api/firmware: {} staged and verified in {:.1} s; restarting into the updater",
        meta.version,
        up.elapsed().as_secs_f32()
    );
    log_line(&format!(
        "staged {} ({} B), restarting into the updater",
        meta.version, meta.length
    ));
    up.json(&format!(
        r#"{{"ok":true,"version":{},"rebooting":true}}"#,
        json_string(&meta.version)
    ))?;
    crate::http::reboot_after_response()
}

/// The last line of `/update/log.txt`, at most [`LAST_MAX`] bytes.
fn last_log_line() -> Option<String> {
    let mut f = fs::File::open(LOG).ok()?;
    let size = f.seek(SeekFrom::End(0)).ok()?;
    let from = size.saturating_sub(LAST_MAX as u64 + 2);
    f.seek(SeekFrom::Start(from)).ok()?;
    let mut tail = Vec::with_capacity((size - from) as usize);
    f.take(LAST_MAX as u64 + 2).read_to_end(&mut tail).ok()?;
    let text = String::from_utf8_lossy(&tail);
    text.lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .map(|l| l.chars().take(LAST_MAX).collect())
}

fn pending_version() -> Option<String> {
    let s = fs::read_to_string(PENDING).ok()?;
    let v = s.lines().next()?.trim();
    (!v.is_empty() && v.len() <= 32).then(|| v.to_string())
}

/// `GET /api/firmware`: `{"version","build","variant","pending","last"}`.
/// Reads two small files on the card when it is mounted.
pub fn info_json(card_mounted: bool) -> String {
    let (pending, last) = if card_mounted {
        (pending_version(), last_log_line())
    } else {
        (None, None)
    };
    let opt = |v: Option<String>| v.as_deref().map_or_else(|| "null".into(), json_string);
    format!(
        r#"{{"version":{},"build":{},"variant":{},"pending":{},"last":{}}}"#,
        json_string(env!("CARGO_PKG_VERSION")),
        json_string(&crate::state::running_version()),
        json_string(crate::state::FLASH_VARIANT),
        opt(pending),
        opt(last),
    )
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
    let running = env!("CARGO_PKG_VERSION");
    for (name, v) in &done {
        if v == running {
            log::info!("firmware updated over the LAN to {v} (the updater installed it)");
        } else {
            log::warn!(
                "the updater installed {v}, but this firmware is {running} (a version string that does not match Cargo.toml?)"
            );
        }
        log_line(&format!("booted after the updater installed {v}"));
        if let Err(e) = fs::remove_file(format!("{dir}/{name}")) {
            log::warn!("ota: removing {dir}/{name}: {e}");
        }
    }
    if !done.is_empty() {
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
    // A web app upload staged on the card and never written (`www.rs`).
    let _ = fs::remove_file(format!("{dir}/www.pack.part"));
}

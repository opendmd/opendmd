//! `POST /api/ota` — push a new firmware image over the air.
//!
//! Lifted from `~/projects/stripe-dashboard/src/web_portal.rs`
//! (`handle_ota_upload` + `extract_firmware_variant`), adapted to this
//! firmware's version string and to the raw httpd wrapper in [`crate::httpd`].
//! The awkward parts are all theirs:
//!
//! - `esp_ota_mark_app_valid_cancel_rollback()` first, or `esp_ota_begin`
//!   returns `ESP_ERR_OTA_ROLLBACK_INVALID_STATE` on a rollback-enabled build.
//! - The **first chunk is read before any flash is erased**, so a wrong-variant
//!   image is rejected with the running firmware still intact.
//! - Every failure path calls `esp_ota_abort`.
//! - The success response is flushed and then slept on for a second, because
//!   `esp_restart()` on a socket that has not drained leaves the browser with a
//!   connection reset instead of a result.
//!
//! This board has 4 MB of flash and two 1.94 MB OTA slots, and
//! `CONFIG_BOOTLOADER_APP_ROLLBACK_ENABLE` is off (see `partitions.csv`): a bad
//! image means a USB reflash, which is why the variant check earns its keep.

use anyhow::Result;
use esp_idf_sys as sys;
use log::{error, info, warn};

use crate::config;
use crate::httpd::Req;

/// Which flash layout this build is for, compiled in by `build.rs`.
pub const FLASH_VARIANT: &str = env!("OPENDMD_FLASH_VARIANT");

/// The running firmware's version, as the bootloader records it.
pub fn running_version() -> String {
    // SAFETY: `esp_app_get_description` returns a pointer into the app image
    // header, which is mapped for the life of the program.
    unsafe {
        let desc = sys::esp_app_get_description();
        if desc.is_null() {
            return "unknown".to_string();
        }
        std::ffi::CStr::from_ptr((*desc).version.as_ptr())
            .to_string_lossy()
            .into_owned()
    }
}

/// Read the flash variant out of an uploaded image's `esp_app_desc_t`.
///
/// The struct starts at binary offset 0x20 (image header + first segment
/// header) and its `version` field is 0x10 into it, so 0x30 in the file. A
/// release build ends that string with the variant word ("0.1.0 4mb").
/// `None` means "no variant marker", which older and dev builds legitimately
/// have — those are allowed through.
fn extract_firmware_variant(data: &[u8]) -> Option<&str> {
    const VERSION_OFFSET: usize = 0x30;
    const VERSION_LEN: usize = 32;

    if data.len() < VERSION_OFFSET + VERSION_LEN {
        return None;
    }
    let bytes = &data[VERSION_OFFSET..VERSION_OFFSET + VERSION_LEN];
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(VERSION_LEN);
    let version = std::str::from_utf8(&bytes[..end]).ok()?;
    match version.split_whitespace().last()? {
        v @ ("4mb" | "8mb") => Some(v),
        _ => None,
    }
}

/// Receive a raw firmware image and reboot into it.
pub fn handle_upload(req: &mut Req) -> Result<()> {
    // SAFETY: an argument-free IDF call; a no-op when rollback is disabled.
    unsafe { sys::esp_ota_mark_app_valid_cancel_rollback() };

    let content_len = req.content_len();
    if content_len == 0 {
        return req.bad_request("Content-Length required");
    }

    // SAFETY: a read-only query of the partition table.
    let partition = unsafe { sys::esp_ota_get_next_update_partition(std::ptr::null()) };
    if partition.is_null() {
        return req.error(
            500,
            "500 Internal Server Error",
            "no OTA partition available",
        );
    }
    // SAFETY: non-null, and the partition table is immutable at runtime.
    let max_size = unsafe { (*partition).size } as usize;
    if content_len > max_size {
        return req.error(
            413,
            "413 Payload Too Large",
            &format!("firmware is {content_len} B, the slot is {max_size} B"),
        );
    }

    info!("OTA: receiving {content_len} B");

    // Read the first chunk before erasing anything, so the variant check can
    // reject the upload with the running firmware untouched.
    let mut first = vec![0u8; config::IO_CHUNK];
    let first_len = match req.read(&mut first) {
        Ok(0) => return req.bad_request("no data received"),
        Ok(n) => n,
        Err(e) => {
            return req.error(
                500,
                "500 Internal Server Error",
                &format!("read error: {e}"),
            )
        }
    };
    match extract_firmware_variant(&first[..first_len]) {
        Some(v) if v != FLASH_VARIANT => {
            warn!("OTA: variant mismatch, image is '{v}', device is '{FLASH_VARIANT}'");
            return req.bad_request(&format!(
                "firmware variant mismatch: this is a {FLASH_VARIANT} device, the image is {v}"
            ));
        }
        Some(v) => info!("OTA: variant check passed ({v})"),
        None => info!("OTA: no variant marker in the image, skipping the check"),
    }

    let mut handle: sys::esp_ota_handle_t = 0;
    // SAFETY: `partition` is non-null and `handle` is a live local.
    let err = unsafe {
        sys::esp_ota_begin(
            partition,
            sys::OTA_WITH_SEQUENTIAL_WRITES as usize,
            &mut handle,
        )
    };
    if err != sys::ESP_OK {
        error!("esp_ota_begin failed: {err}");
        return req.error(500, "500 Internal Server Error", "OTA begin failed");
    }

    let mut written = 0usize;
    let mut failure: Option<String> = None;

    // SAFETY: each write hands OTA a live slice of exactly `n` bytes.
    let write = |buf: &[u8]| -> bool {
        let err = unsafe { sys::esp_ota_write(handle, buf.as_ptr() as *const _, buf.len()) };
        err == sys::ESP_OK
    };

    if !write(&first[..first_len]) {
        failure = Some("write failed on the first chunk".into());
    } else {
        written = first_len;
        let mut buf = vec![0u8; config::IO_CHUNK];
        loop {
            match req.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if !write(&buf[..n]) {
                        failure = Some(format!("write failed at offset {written}"));
                        break;
                    }
                    written += n;
                }
                Err(e) => {
                    failure = Some(format!("read error at offset {written}: {e}"));
                    break;
                }
            }
        }
    }

    if failure.is_none() && written != content_len {
        failure = Some(format!("upload incomplete ({written}/{content_len})"));
    }
    if let Some(reason) = failure {
        error!("OTA: {reason}");
        // SAFETY: `handle` came from a successful esp_ota_begin.
        unsafe { sys::esp_ota_abort(handle) };
        return req.error(500, "500 Internal Server Error", &reason);
    }

    // SAFETY: as above; `esp_ota_end` validates the image and consumes the
    // handle either way.
    let err = unsafe { sys::esp_ota_end(handle) };
    if err != sys::ESP_OK {
        error!("esp_ota_end failed: {err}");
        return req.bad_request("firmware validation failed");
    }
    // SAFETY: a partition-table pointer, checked non-null above.
    let err = unsafe { sys::esp_ota_set_boot_partition(partition) };
    if err != sys::ESP_OK {
        error!("esp_ota_set_boot_partition failed: {err}");
        return req.error(
            500,
            "500 Internal Server Error",
            "could not set boot partition",
        );
    }

    info!("OTA: {written} B written, rebooting");
    req.json(r#"{"ok":true,"rebooting":true}"#)?;
    reboot_after_response()
}

/// Let the response drain out of the TCP stack, then restart.
pub fn reboot_after_response() -> ! {
    std::thread::sleep(std::time::Duration::from_secs(1));
    // SAFETY: an argument-free IDF call, declared `-> !` by the bindings.
    unsafe { sys::esp_restart() }
}

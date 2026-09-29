//! The brightness ceiling: `max_brightness` limits `brightness`.
//!
//! The panels and the ESP32 share one 5 V supply; at high brightness on a
//! white screen with WiFi busy the supply sags and the chip resets. The
//! setting `max_brightness` (16..=255) caps what the panel is ever driven at.
//! Pure rules only, so the firmware's HTTP handler, its boot path and the
//! tests share one definition.

/// Lowest `max_brightness` accepted: below this the panel is barely visible.
pub const MIN_MAX: u8 = 16;

/// `min(brightness, max)`: what is actually applied to the panel. Every path
/// that sets panel brightness (play, the schedule's level, boot) goes through
/// this, directly or via the panel's own cap.
pub fn effective(brightness: u8, max: u8) -> u8 {
    brightness.min(max)
}

/// Check a requested `max_brightness` (as sent, so a value beyond `u8` gets
/// the range message, not a JSON error).
pub fn validate_max(v: u32) -> Result<u8, String> {
    match u8::try_from(v) {
        Ok(m) if m >= MIN_MAX => Ok(m),
        _ => Err(format!(
            "max_brightness must be {MIN_MAX}..=255, not {v}"
        )),
    }
}

/// Resolve a `PUT /api/config` patch against the saved pair, returning the
/// `(brightness, max_brightness)` to save. A `brightness` above the maximum
/// in force (the request's own `max_brightness`, else the saved one) is an
/// error; a maximum lowered below the saved brightness lowers it in the same
/// save, so what is saved is always consistent.
pub fn resolve(
    brightness: u8,
    max: u8,
    new_brightness: Option<u8>,
    new_max: Option<u32>,
) -> Result<(u8, u8), String> {
    let max = match new_max {
        Some(v) => validate_max(v)?,
        None => max,
    };
    let brightness = match new_brightness {
        Some(b) if b > max => {
            return Err(format!(
                "brightness {b} is above the maximum {max}; raise Maximum brightness first"
            ))
        }
        Some(b) => b,
        None => brightness,
    };
    Ok((effective(brightness, max), max))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effective_is_the_minimum() {
        assert_eq!(effective(96, 160), 96);
        assert_eq!(effective(200, 160), 160);
        assert_eq!(effective(160, 160), 160);
        assert_eq!(effective(0, 16), 0);
        assert_eq!(effective(255, 255), 255);
    }

    #[test]
    fn max_range_is_16_to_255() {
        assert_eq!(validate_max(16), Ok(16));
        assert_eq!(validate_max(255), Ok(255));
        assert!(validate_max(15).is_err());
        assert!(validate_max(0).is_err());
        assert!(validate_max(256).is_err());
        assert!(validate_max(100_000).is_err());
    }

    #[test]
    fn brightness_above_saved_max_is_refused() {
        let e = resolve(96, 160, Some(200), None).unwrap_err();
        assert_eq!(
            e,
            "brightness 200 is above the maximum 160; raise Maximum brightness first"
        );
        assert_eq!(resolve(96, 160, Some(160), None), Ok((160, 160)));
    }

    #[test]
    fn brightness_is_judged_against_the_same_requests_max() {
        assert_eq!(resolve(96, 160, Some(200), Some(220)), Ok((200, 220)));
        let e = resolve(96, 220, Some(200), Some(180)).unwrap_err();
        assert!(e.contains("above the maximum 180"), "{e}");
    }

    #[test]
    fn lowering_max_lowers_brightness() {
        assert_eq!(resolve(150, 200, None, Some(100)), Ok((100, 100)));
        assert_eq!(resolve(50, 200, None, Some(100)), Ok((50, 100)));
        // Lowering the max and the brightness together is fine.
        assert_eq!(resolve(150, 200, Some(60), Some(100)), Ok((60, 100)));
    }

    #[test]
    fn bad_max_is_refused_before_anything_else() {
        assert!(resolve(96, 160, None, Some(8)).is_err());
        assert!(resolve(96, 160, Some(10), Some(300)).is_err());
    }

    #[test]
    fn no_patch_keeps_the_pair() {
        assert_eq!(resolve(96, 160, None, None), Ok((96, 160)));
        // A saved pair that was inconsistent is repaired.
        assert_eq!(resolve(200, 160, None, None), Ok((160, 160)));
    }
}

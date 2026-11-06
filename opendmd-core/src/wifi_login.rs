//! WPA-Enterprise (802.1X) WiFi logins: the checks `PUT /api/config` (and
//! the mock device) make on a username, an outer identity and the inner
//! method, and the words they are stored and shown as.
//!
//! A network with a username saved is joined with 802.1X: the firmware's
//! `wifi.rs` hands the username, the outer identity and the WiFi password
//! to ESP-IDF's EAP client (`esp_eap_client_*`), which answers PEAP or
//! TTLS, whichever the network's server asks for. PEAP's inner method is
//! always MSCHAPv2; TTLS's is [`Phase2`]. EAP-TLS (a client certificate)
//! is not offered. No username: WPA2/WPA3 Personal or an open network, as
//! before. See docs/api.md "Network".

/// The longest username or outer identity, in bytes: ESP-IDF's
/// (`USERNAME_LEN_MAX`, `ANONYMOUS_ID_LEN_MAX` in `esp_eap_client.c`).
pub const MAX_NAME: usize = 128;
/// The longest password with a username: the EAP client copies any length,
/// but nothing real is longer, and NVS keeps it like the WiFi password.
pub const MAX_PASS: usize = 128;

/// TTLS's inner method (ESP-IDF's `esp_eap_ttls_phase2_types`, the same
/// order): MSCHAPv2 unless the network's instructions say otherwise.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Phase2 {
    #[default]
    Mschapv2,
    Mschap,
    Pap,
    Chap,
}

impl Phase2 {
    pub const ALL: [Phase2; 4] = [Phase2::Mschapv2, Phase2::Mschap, Phase2::Pap, Phase2::Chap];

    /// The API's and NVS's word for it.
    pub fn as_str(self) -> &'static str {
        match self {
            Phase2::Mschapv2 => "mschapv2",
            Phase2::Mschap => "mschap",
            Phase2::Pap => "pap",
            Phase2::Chap => "chap",
        }
    }

    /// From [`as_str`](Self::as_str)'s word.
    pub fn parse(s: &str) -> Option<Phase2> {
        Some(match s {
            "mschapv2" => Phase2::Mschapv2,
            "mschap" => Phase2::Mschap,
            "pap" => Phase2::Pap,
            "chap" => Phase2::Chap,
            _ => return None,
        })
    }

    /// ESP-IDF's number for it (`ESP_EAP_TTLS_PHASE2_*`: EAP is 0).
    pub fn esp_type(self) -> u32 {
        match self {
            Phase2::Mschapv2 => 1,
            Phase2::Mschap => 2,
            Phase2::Pap => 3,
            Phase2::Chap => 4,
        }
    }
}

/// Is `v` a username or an outer identity the EAP client takes? `""` is
/// (none); else at most [`MAX_NAME`] bytes, one line, no control
/// characters. The error is `PUT /api/config`'s 400 sentence.
pub fn check_name(field: &str, v: &str) -> Result<(), String> {
    if v.len() > MAX_NAME {
        return Err(format!("{field} is at most {MAX_NAME} bytes"));
    }
    if v.chars().any(char::is_control) {
        return Err(format!("{field} must be one line, without control characters"));
    }
    Ok(())
}

/// The outer identity that goes out in the clear: the one given (often
/// `anonymous@example.edu`), else the username itself.
pub fn outer<'a>(identity: &'a str, user: &'a str) -> &'a str {
    if identity.is_empty() {
        user
    } else {
        identity
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phase2_words_round_trip() {
        for p in Phase2::ALL {
            assert_eq!(Phase2::parse(p.as_str()), Some(p));
        }
        assert_eq!(Phase2::parse("MSCHAPv2"), None);
        assert_eq!(Phase2::parse("eap"), None);
        assert_eq!(Phase2::parse(""), None);
        assert_eq!(Phase2::default(), Phase2::Mschapv2);
        // ESP-IDF's enum order: EAP, MSCHAPV2, MSCHAP, PAP, CHAP.
        assert_eq!(Phase2::ALL.map(Phase2::esp_type), [1, 2, 3, 4]);
    }

    #[test]
    fn names_are_checked() {
        assert!(check_name("wifi_eap_user", "").is_ok());
        assert!(check_name("wifi_eap_user", "ab12@example.edu").is_ok());
        assert!(check_name("wifi_eap_user", &"a".repeat(MAX_NAME)).is_ok());
        let long = check_name("wifi_eap_user", &"a".repeat(MAX_NAME + 1)).unwrap_err();
        assert!(long.contains("128"), "{long}");
        assert!(check_name("wifi_eap_identity", "a\nb").is_err());
        assert!(check_name("wifi_eap_identity", "a\0b").is_err());
        // Bytes, not characters: 43 three-byte characters are 129 bytes.
        assert!(check_name("wifi_eap_user", &"€".repeat(43)).is_err());
    }

    #[test]
    fn outer_identity_falls_back_to_the_username() {
        assert_eq!(outer("", "ab12@example.edu"), "ab12@example.edu");
        assert_eq!(outer("anonymous@example.edu", "ab12@example.edu"), "anonymous@example.edu");
    }
}

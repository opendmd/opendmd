//! The display's name (the `hostname` setting): `http://<name>.local/`, the
//! DHCP client name and the mDNS service instance. Pure rules, so the
//! firmware's `PUT /api/config` handler, its network worker and the tests
//! share one definition. A saved change is applied live by the network
//! worker (`NetCommand::Rename`): mDNS re-announces, DHCP sends the new name
//! from the next lease renewal.

/// The longest name: esp-netif's limit for the DHCP client name
/// (`esp_netif_set_hostname` refuses more), well inside mDNS's 63 even
/// after a conflict's `-2`.
pub const MAX: usize = 32;

/// Is `name` a name the device may store? 1..=[`MAX`] of `a-z`, `0-9` and
/// `-`, not starting or ending with `-`. The setup page and the web app
/// check the same rule.
pub fn valid(name: &str) -> bool {
    (1..=MAX).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !name.starts_with('-')
        && !name.ends_with('-')
}

/// A `PUT /api/config` `hostname` value: trimmed and lowercased; `Ok(None)`
/// when blank (the saved name stays), `Err` with the reply's message when
/// it is not [`valid`].
pub fn parse(v: &str) -> Result<Option<String>, String> {
    let v = v.trim().to_ascii_lowercase();
    if v.is_empty() {
        Ok(None)
    } else if valid(&v) {
        Ok(Some(v))
    } else {
        Err(format!(
            "hostname must be 1-{MAX} lowercase letters, digits and hyphens, not starting or ending with a hyphen"
        ))
    }
}

/// The mDNS service instance: per display, so two on one network are told
/// apart in a browser's service list.
pub fn instance_name(name: &str) -> String {
    format!("OpenDMD ({name})")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_names() {
        for ok in ["opendmd", "a", "rig-2", "0", "a-b-c", &"a".repeat(MAX)] {
            assert!(valid(ok), "{ok:?}");
        }
        for bad in [
            "",
            "-rig",
            "rig-",
            "Rig",
            "rig.two",
            "rig two",
            "rig_two",
            "rïg",
            &"a".repeat(MAX + 1),
        ] {
            assert!(!valid(bad), "{bad:?}");
        }
    }

    #[test]
    fn parse_trims_lowercases_and_keeps_blank() {
        assert_eq!(parse(" Rig-Two "), Ok(Some("rig-two".into())));
        assert_eq!(parse(""), Ok(None));
        assert_eq!(parse("   "), Ok(None));
        let e = parse("bad name").unwrap_err();
        assert!(e.starts_with("hostname must be 1-32 "), "{e}");
        assert!(parse("-x").is_err());
    }

    #[test]
    fn instance_names_the_display() {
        assert_eq!(instance_name("kitchen"), "OpenDMD (kitchen)");
    }
}

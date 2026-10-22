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

/// The start of `PUT /api/config`'s 409 message when another device
/// already answers to the new name (`<name>.local`): clients match on it,
/// as on `FILTER_EMPTY`. [`taken_message`] builds the whole message.
pub const NAME_TAKEN: &str = "name taken";

/// The 409 message: `name taken: <name>.local already answers from <ip>`.
pub fn taken_message(name: &str, by: &str) -> String {
    format!("{NAME_TAKEN}: {name}.local already answers from {by}")
}

/// What asking the network for `<name>.local` found
/// (`GET /api/name-check`, and `PUT /api/config` before a new name is
/// saved). mDNS's own tie-break would otherwise hand the name to the
/// display renamed last and push the one that had it to `<name>-2`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Check {
    /// No other device answered.
    Free,
    /// Another device answered, from this IPv4 address.
    Taken(String),
    /// Could not ask: no mDNS (the setup network), the query failed, or the
    /// network worker was busy. A save goes ahead.
    Unknown,
}

impl Check {
    /// The other device's address, from the IPv4 addresses that answered
    /// (`answers`) and ours (`own`, if known): an answer from our own
    /// address is not a clash.
    pub fn from_answers(answers: &[[u8; 4]], own: Option<[u8; 4]>) -> Check {
        match answers.iter().find(|a| Some(**a) != own && **a != [0; 4]) {
            Some([a, b, c, d]) => Check::Taken(format!("{a}.{b}.{c}.{d}")),
            None => Check::Free,
        }
    }

    /// `"taken"`'s JSON: `true`, `false`, or `null` when it could not ask.
    pub fn taken_json(&self) -> &'static str {
        match self {
            Check::Free => "false",
            Check::Taken(_) => "true",
            Check::Unknown => "null",
        }
    }

    /// Who has it, if anyone.
    pub fn by(&self) -> Option<&str> {
        match self {
            Check::Taken(ip) => Some(ip),
            _ => None,
        }
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
    fn answers_from_elsewhere_are_a_clash() {
        let own = Some([192, 168, 1, 20]);
        assert_eq!(Check::from_answers(&[], own), Check::Free);
        assert_eq!(Check::from_answers(&[[192, 168, 1, 20]], own), Check::Free);
        assert_eq!(Check::from_answers(&[[0, 0, 0, 0]], own), Check::Free);
        assert_eq!(
            Check::from_answers(&[[192, 168, 1, 20], [192, 168, 1, 7]], own),
            Check::Taken("192.168.1.7".into())
        );
        assert_eq!(
            Check::from_answers(&[[10, 0, 0, 2]], None),
            Check::Taken("10.0.0.2".into())
        );
    }

    #[test]
    fn check_json_and_message() {
        assert_eq!(Check::Free.taken_json(), "false");
        assert_eq!(Check::Unknown.taken_json(), "null");
        let t = Check::Taken("10.0.0.2".into());
        assert_eq!((t.taken_json(), t.by()), ("true", Some("10.0.0.2")));
        assert_eq!(Check::Free.by(), None);
        let m = taken_message("kitchen", "10.0.0.2");
        assert_eq!(m, "name taken: kitchen.local already answers from 10.0.0.2");
        assert!(m.starts_with(NAME_TAKEN));
    }

    #[test]
    fn instance_names_the_display() {
        assert_eq!(instance_name("kitchen"), "OpenDMD (kitchen)");
    }
}

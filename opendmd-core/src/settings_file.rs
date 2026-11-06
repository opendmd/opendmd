//! The settings file on the card, `/opendmd.json`: the settings that travel
//! with the card (`firmware/src/settings_card.rs` says which), kept there
//! instead of NVS so a change made while a Formula plays is durable at once:
//! a card write does not stall the flash cache, an NVS write does (it waits
//! for no Formula to play).
//!
//! **Format.** A JSON object of the settings by their `/api/config` names,
//! sealed by its writer with a last member `"crc":"<8 hex>"`: the CRC-32 of
//! the object's text without that member ([`seal`], [`check`]). A file
//! edited on a PC (no seal, or a stale one) is taken as it is if it parses.
//!
//! **Atomicity.** FAT renames refuse an existing name, so a save is
//! ([`save`]): write `opendmd.json.new` whole and sync it; move
//! `opendmd.json` to `opendmd.json.bak` (an older `.bak` removed first);
//! rename `.new` to `opendmd.json`. A load ([`choose`]) takes
//! `opendmd.json` if it parses (it is only ever a completed `.new`, or the
//! user's edit), else `.new` if its seal verifies (a save cut after the
//! move), else `.bak` if it parses. A power cut at any point leaves the new
//! settings or the previous ones, never a mixture, never nothing
//! (`tests`: a save cut after every one of its steps).

/// The file and its siblings, at the card's root.
pub const NAME: &str = "opendmd.json";
pub const NEW: &str = "opendmd.json.new";
pub const BAK: &str = "opendmd.json.bak";

/// CRC-32 (IEEE, as zlib's), bitwise: a few hundred bytes a save.
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn crc32(bytes: &[u8]) -> u32 {
    let mut c = !0u32;
    for &b in bytes {
        c ^= b as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { (c >> 1) ^ 0xEDB8_8320 } else { c >> 1 };
        }
    }
    !c
}

/// The file's text for the JSON object `body` (`{...}`, as its writer
/// serialised it): `body` with `"crc"` appended as its last member, and a
/// line end.
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn seal(body: &str) -> String {
    let body = body.trim();
    let crc = crc32(body.as_bytes());
    let inner = body.strip_suffix('}').unwrap_or(body).trim_end();
    let sep = if inner.trim_end().ends_with('{') { "" } else { "," };
    format!("{inner}{sep}\"crc\":\"{crc:08x}\"}}\n")
}

/// What [`check`] found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Seal {
    /// Sealed, and the seal matches: the body is exactly what was written.
    Verified,
    /// No seal, or one that does not match (an edit on a PC, or a file cut
    /// short): its text is all there is to go on.
    Unverified,
}

/// Whether `text` carries a matching seal.
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn check(text: &str) -> Seal {
    let t = text.trim_end();
    // `…,"crc":"xxxxxxxx"}` or `{"crc":"xxxxxxxx"}`.
    const TAIL: usize = r#""crc":"xxxxxxxx"}"#.len();
    if t.len() < TAIL + 1 || !t.ends_with("\"}") {
        return Seal::Unverified;
    }
    let (head, tail) = t.split_at(t.len() - TAIL);
    let Some(hex) = tail.strip_prefix("\"crc\":\"").and_then(|h| h.strip_suffix("\"}")) else {
        return Seal::Unverified;
    };
    let Ok(want) = u32::from_str_radix(hex, 16) else {
        return Seal::Unverified;
    };
    let head = head.trim_end();
    let body = match head.strip_suffix(',') {
        Some(h) => format!("{}}}", h.trim_end()),
        None if head.ends_with('{') => format!("{head}}}"),
        None => return Seal::Unverified,
    };
    if crc32(body.as_bytes()) == want {
        Seal::Verified
    } else {
        Seal::Unverified
    }
}

/// Which file a load takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    Name,
    New,
    Bak,
    /// None usable: the settings stay what they were (NVS's), and the next
    /// save writes the file afresh.
    None,
}

/// Choose among the three files' texts (`None`: not there or unreadable).
/// `parses` says whether a text is a settings object the firmware reads.
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn choose(name: Option<&str>, new: Option<&str>, bak: Option<&str>, parses: impl Fn(&str) -> bool) -> Choice {
    if name.is_some_and(&parses) {
        return Choice::Name;
    }
    if new.is_some_and(|t| check(t) == Seal::Verified && parses(t)) {
        return Choice::New;
    }
    if bak.is_some_and(&parses) {
        return Choice::Bak;
    }
    Choice::None
}

/// The card, as a save and a load see it: files at the root by name.
pub trait Store {
    /// The file's text, `None` if it is not there (or not readable).
    fn read(&mut self, name: &str) -> Option<String>;
    /// Create or truncate `name`, write `text` and sync it to the card.
    fn write_synced(&mut self, name: &str, text: &str) -> std::io::Result<()>;
    /// Rename `from` to `to` (`to` must not exist).
    fn rename(&mut self, from: &str, to: &str) -> std::io::Result<()>;
    /// Remove `name`; one that is not there is not an error.
    fn remove(&mut self, name: &str) -> std::io::Result<()>;
    fn exists(&mut self, name: &str) -> bool;
}

/// A save's steps, for a test (or a debug hook) to stop after one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Step {
    WroteNew,
    RemovedOldBak,
    MovedToBak,
    Renamed,
}

/// Save the sealed `body` (see the module docs), stopping after `stop` if
/// given (the power-cut hook: what is on the card then is what a cut there
/// leaves).
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn save(store: &mut dyn Store, body: &str, stop: Option<Step>) -> std::io::Result<()> {
    store.write_synced(NEW, &seal(body))?;
    if stop == Some(Step::WroteNew) {
        return Ok(());
    }
    if store.exists(NAME) {
        store.remove(BAK)?;
        if stop == Some(Step::RemovedOldBak) {
            return Ok(());
        }
        store.rename(NAME, BAK)?;
    }
    if stop == Some(Step::MovedToBak) {
        return Ok(());
    }
    store.rename(NEW, NAME)?;
    Ok(())
}

/// Load: the chosen file's text, after putting the card back in order (a
/// `.new` that won is renamed into place; a stale `.new` is removed).
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn load(store: &mut dyn Store, parses: impl Fn(&str) -> bool) -> Option<String> {
    let name = store.read(NAME);
    let new = store.read(NEW);
    let bak = store.read(BAK);
    match choose(name.as_deref(), new.as_deref(), bak.as_deref(), parses) {
        Choice::Name => {
            if new.is_some() {
                let _ = store.remove(NEW);
            }
            name
        }
        Choice::New => {
            if name.is_some() {
                // (Unreadable as settings: kept aside as the backup.)
                let _ = store.remove(BAK);
                let _ = store.rename(NAME, BAK);
            }
            let _ = store.rename(NEW, NAME);
            new
        }
        Choice::Bak => bak,
        Choice::None => None,
    }
}

/// [`Store`] on a directory of the filesystem: the card's root on the
/// device (`/sdcard`), a temporary directory in the tests.
pub struct DirStore(pub std::path::PathBuf);

impl Store for DirStore {
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    fn read(&mut self, name: &str) -> Option<String> {
        std::fs::read_to_string(self.0.join(name)).ok()
    }
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    fn write_synced(&mut self, name: &str, text: &str) -> std::io::Result<()> {
        use std::io::Write;
        let mut f = std::fs::File::create(self.0.join(name))?;
        f.write_all(text.as_bytes())?;
        f.sync_all()
    }
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    fn rename(&mut self, from: &str, to: &str) -> std::io::Result<()> {
        std::fs::rename(self.0.join(from), self.0.join(to))
    }
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    fn remove(&mut self, name: &str) -> std::io::Result<()> {
        match std::fs::remove_file(self.0.join(name)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    fn exists(&mut self, name: &str) -> bool {
        self.0.join(name).exists()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[derive(Default, Clone)]
    struct Mem(HashMap<String, String>);

    impl Store for Mem {
        fn read(&mut self, name: &str) -> Option<String> {
            self.0.get(name).cloned()
        }
        fn write_synced(&mut self, name: &str, text: &str) -> std::io::Result<()> {
            self.0.insert(name.into(), text.into());
            Ok(())
        }
        fn rename(&mut self, from: &str, to: &str) -> std::io::Result<()> {
            if self.0.contains_key(to) {
                return Err(std::io::Error::other("exists"));
            }
            let v = self.0.remove(from).ok_or_else(|| std::io::Error::other("no such file"))?;
            self.0.insert(to.into(), v);
            Ok(())
        }
        fn remove(&mut self, name: &str) -> std::io::Result<()> {
            self.0.remove(name);
            Ok(())
        }
        fn exists(&mut self, name: &str) -> bool {
            self.0.contains_key(name)
        }
    }

    /// Stands for the firmware's parse: an object with a `"b"` member.
    fn parses(t: &str) -> bool {
        let t = t.trim();
        t.starts_with('{') && t.ends_with('}') && t.contains("\"b\":")
    }

    fn value(t: &str) -> Option<u32> {
        let i = t.find("\"b\":")? + 4;
        t[i..].split([',', '}']).next()?.trim().parse().ok()
    }

    #[test]
    fn a_sealed_file_verifies_and_any_change_unseals_it() {
        let s = seal(r#"{"b":12,"look":"amber"}"#);
        assert!(s.ends_with("\"}\n"));
        assert_eq!(check(&s), Seal::Verified);
        assert_eq!(check(&s.replace("12", "13")), Seal::Unverified);
        assert_eq!(check(r#"{"b":12}"#), Seal::Unverified);
        assert_eq!(check(&s[..s.len() - 10]), Seal::Unverified, "cut short");
        assert_eq!(check(&seal("{}")), Seal::Verified);
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926, "the IEEE check value");
    }

    /// A save cut after each of its steps (and a whole one), from a card
    /// that had settings and from one that had none: the load gets the old
    /// value or the new one, never nothing once a file existed, and leaves
    /// the card loadable by the next save.
    #[test]
    fn a_save_cut_at_any_step_loads_the_old_settings_or_the_new() {
        for had in [None, Some(1u32)] {
            for stop in [Some(Step::WroteNew), Some(Step::RemovedOldBak), Some(Step::MovedToBak), Some(Step::Renamed), None] {
                let mut m = Mem::default();
                if let Some(v) = had {
                    save(&mut m, &format!(r#"{{"b":{v}}}"#), None).unwrap();
                    // And a backup from before that.
                    save(&mut m, &format!(r#"{{"b":{v}}}"#), None).unwrap();
                }
                save(&mut m, r#"{"b":2}"#, stop).unwrap();
                let got = load(&mut m, parses).and_then(|t| value(&t));
                match (had, stop) {
                    (_, None | Some(Step::Renamed)) => assert_eq!(got, Some(2), "{had:?} {stop:?}"),
                    (None, Some(Step::MovedToBak)) => assert_eq!(got, Some(2), "{had:?} {stop:?}"),
                    (None, _) => assert!(got.is_none() || got == Some(2), "{had:?} {stop:?}: {got:?}"),
                    (Some(_), Some(Step::MovedToBak)) => assert_eq!(got, Some(2), "the sealed .new: {stop:?}"),
                    (Some(v), _) => assert_eq!(got, Some(v), "{stop:?}"),
                }
                // Whatever was left, the next save completes and loads.
                save(&mut m, r#"{"b":3}"#, None).unwrap();
                assert_eq!(load(&mut m, parses).and_then(|t| value(&t)), Some(3), "{had:?} {stop:?}");
                assert!(!m.0.contains_key(NEW), "no stale .new after a full save and load");
            }
        }
    }

    /// The same, through real files.
    #[test]
    fn saves_and_loads_through_a_directory() {
        let dir = std::env::temp_dir().join(format!("opendmd-settings-{}-{}", std::process::id(), line!()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut d = DirStore(dir.clone());
        assert!(load(&mut d, parses).is_none());
        save(&mut d, r#"{"b":1}"#, None).unwrap();
        save(&mut d, r#"{"b":2}"#, Some(Step::MovedToBak)).unwrap();
        assert!(!d.exists(NAME), "cut between the renames");
        assert_eq!(load(&mut d, parses).and_then(|t| value(&t)), Some(2));
        assert!(d.exists(NAME) && !d.exists(NEW), "put back in order");
        save(&mut d, r#"{"b":3}"#, None).unwrap();
        assert_eq!(value(&d.read(BAK).unwrap()), Some(2));
        assert_eq!(load(&mut d, parses).and_then(|t| value(&t)), Some(3));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_new_file_cut_short_never_wins() {
        let mut m = Mem::default();
        save(&mut m, r#"{"b":1}"#, None).unwrap();
        // A `.new` cut mid-write: no valid seal.
        let half = seal(r#"{"b":2,"look":"amber"}"#);
        m.0.insert(NEW.into(), half[..half.len() / 2].into());
        m.0.remove(NAME);
        m.0.insert(BAK.into(), seal(r#"{"b":1}"#));
        assert_eq!(load(&mut m, parses).and_then(|t| value(&t)), Some(1));
    }

    #[test]
    fn an_edit_on_a_pc_is_taken_as_it_is() {
        let mut m = Mem::default();
        save(&mut m, r#"{"b":1}"#, None).unwrap();
        save(&mut m, r#"{"b":2}"#, None).unwrap();
        // Edited: its seal no longer matches, and the backup's does.
        let edited = m.0[NAME].replace("2", "7");
        m.0.insert(NAME.into(), edited);
        assert_eq!(load(&mut m, parses).and_then(|t| value(&t)), Some(7));
        // Written by hand, no seal at all.
        m.0.insert(NAME.into(), "{\n  \"b\": 9\n}\n".into());
        assert_eq!(load(&mut m, parses).and_then(|t| value(&t)), Some(9));
        // Not settings at all: the backup.
        m.0.insert(NAME.into(), "garbage".into());
        assert_eq!(load(&mut m, parses).and_then(|t| value(&t)), Some(1), "the .bak: the save before");
    }
}

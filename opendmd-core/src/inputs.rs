//! The input table (`docs/research/psi-inputs.md` §2.1): the panel's own
//! values that Formulas read as built-in inputs (`daylight`, `night`,
//! `replay.game` …), each with its previous value, when it last changed and
//! where it came from. The firmware's sources write it (the sky once a
//! minute, night mode and the brightness from the display loop, RePlayOS
//! from the poller); the playing Formula reads it once a frame
//! ([`snapshot`]) into the VM's frame slots, where `since(KEY)` is the
//! time since the change and `was(KEY)` the value before it.
//!
//! Keys are the built-in inputs' numbers (`opendmd_psi::world::id`, 1 up;
//! the same numbers here so this module needs no Psi). A key never written
//! has no value: the Formula reads its default. Writing the value a key
//! already has is no change; [`Table::clear`] (a source that lost its
//! value: RePlayOS gone, the location cleared) is one when it had a value,
//! as the design says (the Formula fades to the default through `since`
//! and `was` like any change).
//!
//! One mutex, held for microseconds: 16 entries of 20 bytes, static.

use std::sync::Mutex;

/// Room for this many keys (numbers 1 to `KEYS − 1`): the built-ins and
/// room to grow (`opendmd_psi::world::MAX_KEYS`).
pub const KEYS: usize = 16;

/// The key numbers (`opendmd_psi::world::id`'s).
pub mod id {
    pub const DAYLIGHT: u8 = 1;
    pub const SUN_ALT: u8 = 2;
    pub const MOON_PHASE: u8 = 3;
    pub const SEASON: u8 = 4;
    pub const YEARDAY: u8 = 5;
    pub const NIGHT: u8 = 6;
    pub const BRIGHT: u8 = 7;
    pub const REPLAY_PLAYING: u8 = 8;
    pub const REPLAY_PLATFORM: u8 = 9;
    pub const REPLAY_GAME: u8 = 10;
}

/// Where a key's value came from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum Source {
    #[default]
    None = 0,
    /// The clock and the location (the sky).
    Sky = 1,
    /// The display loop (night mode, the brightness).
    Panel = 2,
    /// The RePlayOS poller.
    Replay = 3,
}

/// One key: its value and the one before (Q16.16; `None`: none, the
/// default), when it last changed (ms on the table's clock; 0 never),
/// whether it has a value now, and its source.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Entry {
    pub value: i32,
    pub was: Option<i32>,
    pub changed_ms: u64,
    pub live: bool,
    pub source: Source,
}

/// `since` of a key that never changed, and its cap: 32,767 s, Q16.16
/// (`opendmd_psi::world::SINCE_NEVER`).
pub const SINCE_NEVER: i32 = 32767 << 16;

/// The table.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Table {
    pub entries: [Entry; KEYS],
}

/// What a frame reads of a key: `None` when it has no value (the
/// Formula's default then), else (value, was, since Q16.16 s). A key that
/// changed and then lost its value reads the default as its value but
/// keeps its `since` and `was`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Read {
    pub value: Option<i32>,
    pub was: Option<i32>,
    pub since: i32,
}

impl Table {
    pub const fn new() -> Table {
        Table {
            entries: [Entry {
                value: 0,
                was: None,
                changed_ms: 0,
                live: false,
                source: Source::None,
            }; KEYS],
        }
    }

    /// Key `k` takes `v` at `now_ms` from `source`: a change (its `was` the
    /// value it had, or none) when it had no value or another one.
    pub fn set(&mut self, k: u8, v: i32, now_ms: u64, source: Source) {
        let Some(e) = self.entries.get_mut(k as usize) else {
            return;
        };
        if e.live && e.value == v {
            e.source = source;
            return;
        }
        // (From no value: `was` is the default, none.)
        e.was = e.live.then_some(e.value);
        e.value = v;
        e.live = true;
        e.changed_ms = now_ms.max(1);
        e.source = source;
    }

    /// Key `k` has no value any more (its source lost it): a change, to the
    /// default, if it had one.
    pub fn clear(&mut self, k: u8, now_ms: u64) {
        let Some(e) = self.entries.get_mut(k as usize) else {
            return;
        };
        if e.live {
            e.was = Some(e.value);
            e.live = false;
            e.changed_ms = now_ms.max(1);
        }
    }

    /// Key `k` as a frame at `now_ms` reads it.
    pub fn read(&self, k: u8, now_ms: u64) -> Read {
        let Some(e) = self.entries.get(k as usize) else {
            return Read {
                since: SINCE_NEVER,
                ..Read::default()
            };
        };
        let since = if e.changed_ms == 0 {
            SINCE_NEVER
        } else {
            let ms = now_ms.saturating_sub(e.changed_ms);
            ((ms * 65_536 / 1000).min(SINCE_NEVER as u64)) as i32
        };
        Read {
            value: e.live.then_some(e.value),
            was: e.was,
            since,
        }
    }
}

/// The panel's table.
static TABLE: Mutex<Table> = Mutex::new(Table::new());

/// The table's clock (ms since boot; the firmware's `esp_timer`), for the
/// global [`set`], [`clear`] and [`snapshot`]. None installed: every
/// change at 1 ms and every read at 1 ms (`since` 0), as the browser and
/// the tests never use these.
static CLOCK: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Install the table's clock.
pub fn set_clock(f: fn() -> u64) {
    CLOCK.store(f as usize, std::sync::atomic::Ordering::Relaxed);
}

fn now_ms() -> u64 {
    match CLOCK.load(std::sync::atomic::Ordering::Relaxed) {
        0 => 1,
        a => {
            // SAFETY: stored by `set_clock` from a `fn() -> u64`; fn
            // pointers are never null, so 0 is none.
            let f = unsafe { std::mem::transmute::<usize, fn() -> u64>(a) };
            f()
        }
    }
}

/// [`Table::set`] on the panel's table, now.
pub fn set(k: u8, v: i32, source: Source) {
    let now = now_ms();
    if let Ok(mut t) = TABLE.lock() {
        t.set(k, v, now, source);
    }
}

/// [`Table::clear`] on the panel's table, now.
pub fn clear(k: u8) {
    let now = now_ms();
    if let Ok(mut t) = TABLE.lock() {
        t.clear(k, now);
    }
}

/// The panel's table and the time now, for a frame (one lock).
pub fn snapshot() -> (Table, u64) {
    let now = now_ms();
    let t = TABLE.lock().map(|t| *t).unwrap_or_default();
    (t, now)
}

/// The sky's keys for `unix` (UTC seconds), the local `utc_offset`
/// (seconds) and the location if one is set, into the panel's table:
/// `moon.phase` and `yearday` from the clock alone, the sun's keys and
/// `season` only with a location (cleared without one).
pub fn set_sky(unix: i64, utc_offset: i64, place: Option<crate::sky::Place>) {
    let s = crate::sky::inputs(unix, utc_offset, place);
    let now = now_ms();
    let Ok(mut t) = TABLE.lock() else {
        return;
    };
    t.set(id::MOON_PHASE, s.moon_phase, now, Source::Sky);
    t.set(id::YEARDAY, s.yearday, now, Source::Sky);
    for (k, v) in [
        (id::DAYLIGHT, s.daylight),
        (id::SUN_ALT, s.sun_alt),
        (id::SEASON, s.season),
    ] {
        match v {
            Some(v) => t.set(k, v, now, Source::Sky),
            None => t.clear(k, now),
        }
    }
}

/// The sky's keys cleared (the clock lost, say): the Formulas read their
/// defaults.
pub fn clear_sky() {
    let now = now_ms();
    if let Ok(mut t) = TABLE.lock() {
        for k in [id::DAYLIGHT, id::SUN_ALT, id::SEASON, id::MOON_PHASE, id::YEARDAY] {
            t.clear(k, now);
        }
    }
}

/// `replay.game`'s number for a game: a hash of its name (FNV-1a, folded
/// to 16 bits), Q16.16 in 0..1, never 0 (0 is "no game").
pub fn game_hash(name: &str) -> i32 {
    let mut h: u32 = 0x811c_9dc5;
    for b in name.bytes() {
        h = (h ^ u32::from(b)).wrapping_mul(0x0100_0193);
    }
    (((h >> 16) ^ (h & 0xffff)) as i32).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_change_keeps_the_value_before_and_when() {
        let mut t = Table::new();
        let r = t.read(id::NIGHT, 5_000);
        assert_eq!(r, Read { value: None, was: None, since: SINCE_NEVER });
        t.set(id::NIGHT, 65_536, 1_000, Source::Panel);
        let r = t.read(id::NIGHT, 3_000);
        assert_eq!(r.value, Some(65_536));
        assert_eq!(r.was, None, "the first value: was is the default");
        assert_eq!(r.since, 2 << 16);
        // The same value again: no change.
        t.set(id::NIGHT, 65_536, 2_500, Source::Panel);
        assert_eq!(t.read(id::NIGHT, 3_000).since, 2 << 16);
        t.set(id::NIGHT, 0, 4_000, Source::Panel);
        let r = t.read(id::NIGHT, 4_500);
        assert_eq!((r.value, r.was, r.since), (Some(0), Some(65_536), 1 << 15));
        // Lost: the default, a change.
        t.clear(id::NIGHT, 6_000);
        let r = t.read(id::NIGHT, 6_000);
        assert_eq!((r.value, r.was, r.since), (None, Some(0), 0));
        // Held at 32,767 s.
        assert_eq!(t.read(id::NIGHT, 6_000 + 40_000_000).since, SINCE_NEVER);
        // Keys out of the table are ignored.
        t.set(200, 1, 1, Source::Sky);
        assert_eq!(t.read(200, 1).value, None);
    }

    #[test]
    fn a_game_hash_is_never_none() {
        assert_ne!(game_hash(""), 0);
        assert_ne!(game_hash("Sonic the Hedgehog 3 (USA)"), game_hash("Sonic the Hedgehog 2 (USA)"));
        assert!((1..65_536).contains(&game_hash("Gunstar Heroes")));
    }
}

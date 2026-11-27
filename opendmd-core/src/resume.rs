//! Where an in-order playlist left off, so a rebuild of the engine (an
//! upload, a reindex, a settings change, a schedule rule switching
//! playlists and back, a software restart) continues it instead of starting
//! again at the first entry.
//!
//! **What is kept.** A [`Mark`] per playlist: the playlist's key
//! ([`list_key`]: its manifest path, or a Follow RePlayOS context's index
//! path, with the platform filter and the card's identity, everything that
//! decides which entries its catalog holds), the catalog index of the last
//! entry that started in order and a hash of that entry's path. Only
//! in-order starts are marked: a spell of shuffle leaves the in-order place
//! where it was. The firmware keeps the last [`SLOTS`] marks (most recent
//! first, [`Marks`]) and a count of crash boots in a row in RTC slow memory
//! ([`Store`], [`Tracker`]), rewritten at every start, so a software
//! restart finds them; a power-on finds garbage, which the check words
//! drop. Shuffle ignores them.
//!
//! **Finding the place again** ([`locate`]): the catalog is card-backed (an
//! `.idx` of offsets into the manifest; an entry costs a 4-byte read of the
//! index and a line of the manifest) and may hold 10,000 entries, so it is
//! never searched. The mark's own index is looked at first, then up to
//! [`WINDOW`] either side, nearest first: an upload or a delete usually
//! moves the position by a few entries, an append (what adding to a
//! playlist does) not at all. Found, playback continues [`Step`] entries on
//! from it (the next one, as a rule); not found within the window (the
//! entry itself was removed, or moved further), or at the first entry that
//! will not read, it continues at the same numeric position, or at the top
//! when the list no longer reaches it. At most `2 * WINDOW + 1` lookups,
//! once per engine built.
//!
//! **Which step** ([`Tracker`]): a rebuild cuts the file on screen, so the
//! next one plays ([`Step::Next`]), unless that file had been up for under
//! [`BRIEF_MS`] (a rebuild right after another: it plays again,
//! [`Step::Replay`], so a run of rebuilds does not flash through the list).
//! A file that crashes the board before its first frame is never marked:
//! the boot after the crash would go straight back to it, so at the
//! second crash boot in a row the place steps one further on
//! ([`Step::Over`]), and at the third the marks are dropped and playlists
//! start at the top. Two playlist starts clear the count.

use crate::engine::Catalog;

/// Playlists remembered: the playing one and the few it switched from.
pub const SLOTS: usize = 4;
/// Words in an encoded [`Mark`]: key, index, entry hash, check.
pub const WORDS: usize = 4;
/// Entries looked at either side of a mark's index ([`locate`]).
pub const WINDOW: usize = 8;
/// A file up for less than this when a rebuild cut it plays again
/// ([`Step::Replay`]): under 3 s is a glimpse, not a turn.
pub const BRIEF_MS: u64 = 3_000;
/// Words a [`Store`] holds: the marks' slots, then the crash count.
pub const STORE_WORDS: usize = SLOTS * WORDS + 1;
const CRASH_WORD: usize = SLOTS * WORDS;

/// The layout's magic, folded into the check word: bump it when the layout
/// changes, so an older layout's slots read as empty.
const MAGIC: u32 = 0x5e9_4e50;

/// FNV-1a over `path`, ASCII case folded (FAT names files case-blind).
/// Never 0, so an all-zero slot is never a mark.
pub fn path_hash(path: &str) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for b in path.bytes() {
        h ^= u32::from(b.to_ascii_lowercase());
        h = h.wrapping_mul(0x0100_0193);
    }
    h.max(1)
}

/// A playlist's key: [`path_hash`] of its path (a manifest, or a Follow
/// RePlayOS context's index), folded with its platform filter and the
/// card's identity (`card`: [`path_hash`] of `opendmd_core::card::identity`,
/// 0 for none), so a filter change or another card is another playlist
/// rather than a mark looked for in a different list.
pub fn list_key(path: &str, platforms: &str, card: u32) -> u32 {
    let h = fmix(path_hash(path));
    let h = fmix(h ^ path_hash(platforms.trim()).rotate_left(7)).wrapping_add(0x9e37_79b9);
    fmix(h ^ card.rotate_left(13)).max(1)
}

/// The last entry of one playlist that started.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mark {
    /// The playlist's key ([`path_hash`] of its manifest path).
    pub list: u32,
    /// The entry's catalog index.
    pub index: u32,
    /// [`path_hash`] of the entry's path.
    pub entry: u32,
}

/// murmur3's finalizer: every input bit moves every output bit.
fn fmix(mut h: u32) -> u32 {
    h ^= h >> 16;
    h = h.wrapping_mul(0x85eb_ca6b);
    h ^= h >> 13;
    h = h.wrapping_mul(0xc2b2_ae35);
    h ^ (h >> 16)
}

fn check(list: u32, index: u32, entry: u32) -> u32 {
    fmix(MAGIC ^ fmix(list ^ fmix(index ^ fmix(entry))))
}

impl Mark {
    pub fn encode(&self) -> [u32; WORDS] {
        [
            self.list,
            self.index,
            self.entry,
            check(self.list, self.index, self.entry),
        ]
    }

    /// `None` unless the check word matches (an empty slot, a power-on's
    /// garbage, a write cut short by a reset, an older layout).
    pub fn decode(w: &[u32; WORDS]) -> Option<Mark> {
        let [list, index, entry, c] = *w;
        (list != 0 && entry != 0 && c == check(list, index, entry)).then_some(Mark { list, index, entry })
    }
}

/// The remembered marks, most recent first, one per playlist.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Marks([Option<Mark>; SLOTS]);

impl Marks {
    /// Read back from their words: invalid slots dropped, and a playlist's
    /// second slot (the first one wins), so garbage never picks a position.
    pub fn decode(words: &[[u32; WORDS]; SLOTS]) -> Marks {
        let mut out = Marks::default();
        let mut n = 0;
        for w in words {
            if let Some(m) = Mark::decode(w) {
                if out.get(m.list).is_none() {
                    out.0[n] = Some(m);
                    n += 1;
                }
            }
        }
        out
    }

    /// An empty slot is all zeros (never a valid mark).
    pub fn encode(&self) -> [[u32; WORDS]; SLOTS] {
        let mut out = [[0; WORDS]; SLOTS];
        for (o, m) in out.iter_mut().zip(&self.0) {
            if let Some(m) = m {
                *o = m.encode();
            }
        }
        out
    }

    /// The mark of the playlist keyed `list`.
    pub fn get(&self, list: u32) -> Option<Mark> {
        self.0.iter().flatten().find(|m| m.list == list).copied()
    }

    /// `m` becomes its playlist's mark and the most recent; the least
    /// recent playlist goes when all [`SLOTS`] are taken.
    pub fn note(&mut self, m: Mark) {
        let at = self
            .0
            .iter()
            .position(|s| s.is_none_or(|s| s.list == m.list))
            .unwrap_or(SLOTS - 1);
        self.0.copy_within(0..at, 1);
        self.0[0] = Some(m);
    }

    pub fn is_empty(&self) -> bool {
        self.0.iter().all(Option::is_none)
    }
}

/// How far on from the mark's entry playback continues ([`locate`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// The entry itself again: it was up only briefly ([`BRIEF_MS`]).
    Replay,
    /// The one after it: the rule.
    Next,
    /// The one after that: the next one crashed the board twice in a row.
    Over,
}

impl Step {
    fn on(self) -> usize {
        match self {
            Step::Replay => 0,
            Step::Next => 1,
            Step::Over => 2,
        }
    }
}

/// Where a mark puts playback in a catalog ([`locate`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Located {
    /// The catalog index to play next.
    pub next: usize,
    /// Where the mark's entry was found, if it was.
    pub found: Option<usize>,
    /// Entries read to find it.
    pub probes: usize,
    /// The look stopped at an entry that would not read.
    pub failed: bool,
}

/// The catalog index to continue `mark` at in `catalog`, `step` on (see
/// the module docs): from the mark's entry if it is within `window` of its
/// old index; else the mark's old position (the slot its successor moved
/// into, one further for [`Step::Over`]), or the top when the catalog no
/// longer reaches it. The look stops at the first entry that will not read
/// (each failure is a card error counted, and time on the first file's
/// open). `None` for an empty catalog.
pub fn locate(catalog: &mut dyn Catalog, mark: &Mark, window: usize, step: Step) -> Option<Located> {
    let len = catalog.len();
    if len == 0 {
        return None;
    }
    let at = mark.index as usize;
    let mut probes = 0;
    let mut failed = false;
    'look: for d in 0..=window {
        let ahead = at.checked_add(d).filter(|k| *k < len);
        let behind = at.checked_sub(d).filter(|k| d > 0 && *k < len);
        for k in [ahead, behind].into_iter().flatten() {
            probes += 1;
            match catalog.entry(k) {
                Ok(e) if path_hash(&e.path) == mark.entry => {
                    return Some(Located {
                        next: (k + step.on()) % len,
                        found: Some(k),
                        probes,
                        failed,
                    });
                }
                Ok(_) => {}
                Err(_) => {
                    failed = true;
                    break 'look;
                }
            }
        }
        // Past both ends of the catalog: nothing more to look at.
        if at.saturating_sub(d) == 0 && at.saturating_add(d) >= len - 1 {
            break;
        }
    }
    let place = at + step.on().max(1) - 1;
    Some(Located {
        next: if place < len { place } else { 0 },
        found: None,
        probes,
        failed,
    })
}

/// The firmware's RTC words ([`STORE_WORDS`]: plain word loads and stores).
pub trait Store {
    fn get(&self, i: usize) -> u32;
    fn put(&mut self, i: usize, v: u32);
}

/// The marks in a store.
pub fn load(s: &impl Store) -> Marks {
    let mut w = [[0; WORDS]; SLOTS];
    for (i, x) in w.iter_mut().flatten().enumerate() {
        *x = s.get(i);
    }
    Marks::decode(&w)
}

fn save(s: &mut impl Store, m: &Marks) {
    for (i, x) in m.encode().iter().flatten().enumerate() {
        s.put(i, *x);
    }
}

/// The crash count's word: the count and its complement (anything else
/// reads as 0).
fn count_word(n: u32) -> u32 {
    (n & 0xFFFF) | ((!n & 0xFFFF) << 16)
}

fn count_of(w: u32) -> u32 {
    if w >> 16 == !w & 0xFFFF {
        w & 0xFFFF
    } else {
        0
    }
}

/// What [`Tracker::boot`] found, for its log line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Booted {
    /// A power-on, or nothing kept.
    Fresh,
    /// Marks kept; `crashes` crash boots in a row (2: the next place steps
    /// one further on).
    Kept { marks: usize, crashes: u32 },
    /// The third crash boot in a row (or more): the marks dropped.
    Dropped { crashes: u32 },
}

/// The firmware's resume state over its [`Store`]: which playlist the
/// engine plays, whether a new engine still has to look for its place, the
/// last start (for [`Step::Replay`]) and the crash count. The main task's
/// alone.
#[derive(Clone, Debug, Default)]
pub struct Tracker {
    /// The engine's playlist ([`list_key`]; 0: none yet).
    list: u32,
    /// An engine was built and has not looked for its place yet.
    pending: Option<Step>,
    /// The last playlist start this boot: its playlist and when (ms).
    last: Option<(u32, u64)>,
    /// Playlist starts this boot, counted to 2.
    starts: u8,
    /// Crash boots in a row, as this boot found them (0 once two
    /// playlist starts cleared it).
    crashes: u32,
}

impl Tracker {
    pub const fn new() -> Self {
        Tracker {
            list: 0,
            pending: None,
            last: None,
            starts: 0,
            crashes: 0,
        }
    }

    /// First thing at boot: a power-on starts with none; a crash counts
    /// one more crash boot in a row (any other reset none); at three the
    /// marks go. Writes the store back clean.
    pub fn boot(&mut self, s: &mut impl Store, power_on: bool, crashed: bool) -> Booted {
        let n = match (power_on, crashed) {
            (true, _) | (false, false) => 0,
            (false, true) => count_of(s.get(CRASH_WORD)) + 1,
        };
        let mut marks = if power_on { Marks::default() } else { load(s) };
        if n >= 3 {
            marks = Marks::default();
        }
        save(s, &marks);
        s.put(CRASH_WORD, count_word(n));
        *self = Tracker {
            crashes: n,
            ..Tracker::new()
        };
        match (n, marks.0.iter().flatten().count()) {
            (3.., _) => Booted::Dropped { crashes: n },
            (_, 0) => Booted::Fresh,
            (crashes, marks) => Booted::Kept { marks, crashes },
        }
    }

    /// An engine is being built over the playlist keyed `list` at `now_ms`:
    /// the starts from here on are its, and [`pending`](Self::pending)
    /// says where it continues.
    pub fn building(&mut self, list: u32, now_ms: u64) {
        let step = if self.crashes >= 2 && self.starts == 0 {
            Step::Over
        } else if self.last.is_some_and(|(l, t)| l == list && now_ms.saturating_sub(t) < BRIEF_MS) {
            Step::Replay
        } else {
            Step::Next
        };
        self.list = list;
        self.pending = Some(step);
    }

    /// Once after [`building`](Self::building): the mark of the new
    /// engine's playlist and the step to take from it, if it has one.
    pub fn pending(&mut self, s: &impl Store) -> Option<(Mark, Step)> {
        let step = self.pending.take()?;
        load(s).get(self.list).map(|m| (m, step))
    }

    /// A file started at `now_ms`: a playlist entry (`index`; never
    /// [`OFF_PLAYLIST`](crate::engine::OFF_PLAYLIST), a file played on
    /// request, "previous" included). Marked only `in_order`. The second
    /// playlist start of a boot clears the crash count.
    pub fn started(&mut self, s: &mut impl Store, index: usize, path: &str, in_order: bool, now_ms: u64) {
        if index == crate::engine::OFF_PLAYLIST || self.list == 0 {
            return;
        }
        if self.starts < 2 {
            self.starts += 1;
            if self.starts == 2 && self.crashes > 0 {
                self.crashes = 0;
                s.put(CRASH_WORD, count_word(0));
            }
        }
        self.last = Some((self.list, now_ms));
        if !in_order {
            return;
        }
        let mut m = load(s);
        m.note(Mark {
            list: self.list,
            index: index as u32,
            entry: path_hash(path),
        });
        save(s, &m);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::OFF_PLAYLIST;
    use crate::manifest::Entry;
    use std::io;

    /// Paths, the reads made, and an index that will not read.
    struct Paths(Vec<String>, usize, Option<usize>);

    impl Catalog for Paths {
        fn len(&self) -> usize {
            self.0.len()
        }
        fn entry(&mut self, i: usize) -> io::Result<Entry> {
            self.1 += 1;
            if self.2 == Some(i) {
                return Err(io::Error::other("card error"));
            }
            Ok(Entry::from_path(&self.0[i]))
        }
    }

    fn list(n: usize) -> Paths {
        Paths((0..n).map(|i| format!("/gifs/T/g{i}.gif")).collect(), 0, None)
    }

    fn mark_at(c: &Paths, i: usize) -> Mark {
        Mark {
            list: path_hash("/filelist.txt"),
            index: i as u32,
            entry: path_hash(&c.0[i]),
        }
    }

    fn next(c: &mut Paths, m: &Mark) -> Located {
        locate(c, m, WINDOW, Step::Next).unwrap()
    }

    #[test]
    fn same_list_continues_after_the_entry() {
        let mut c = list(10_000);
        let m = mark_at(&c, 4321);
        let l = next(&mut c, &m);
        assert_eq!((l.next, l.found, l.probes, l.failed), (4322, Some(4321), 1, false));
        // The last entry: the one after it is the first.
        let m = mark_at(&c, 9999);
        assert_eq!(next(&mut c, &m).next, 0);
        // Replayed, or stepped over.
        let m = mark_at(&c, 4321);
        assert_eq!(locate(&mut c, &m, WINDOW, Step::Replay).unwrap().next, 4321);
        assert_eq!(locate(&mut c, &m, WINDOW, Step::Over).unwrap().next, 4323);
        let m = mark_at(&c, 9999);
        assert_eq!(locate(&mut c, &m, WINDOW, Step::Over).unwrap().next, 1);
    }

    #[test]
    fn entries_inserted_or_removed_before_it_are_followed() {
        let c0 = list(100);
        let m = mark_at(&c0, 50);
        // Two inserted before: g50 is now at 52.
        let mut c = list(100);
        c.0.insert(10, "/gifs/T/new1.gif".into());
        c.0.insert(10, "/gifs/T/new2.gif".into());
        let l = next(&mut c, &m);
        assert_eq!((l.next, l.found), (53, Some(52)));
        assert_eq!(c.0[l.next], "/gifs/T/g51.gif");
        // Three removed before: at 47.
        let mut c = list(100);
        c.0.drain(0..3);
        let l = next(&mut c, &m);
        assert_eq!(c.0[l.next], "/gifs/T/g51.gif");
        // Further than the window: the same numeric position.
        let mut c = list(100);
        c.0.drain(0..WINDOW + 1);
        let l = next(&mut c, &m);
        assert_eq!((l.next, l.found, l.probes), (50, None, 2 * WINDOW + 1));
    }

    #[test]
    fn its_own_entry_removed_continues_at_its_position() {
        let c0 = list(100);
        let m = mark_at(&c0, 50);
        let mut c = list(100);
        c.0.remove(50);
        let l = next(&mut c, &m);
        assert_eq!((l.next, l.found), (50, None));
        // Which is its successor; a replay cannot replay it; one over is one on.
        assert_eq!(c.0[50], "/gifs/T/g51.gif");
        assert_eq!(locate(&mut c, &m, WINDOW, Step::Replay).unwrap().next, 50);
        assert_eq!(locate(&mut c, &m, WINDOW, Step::Over).unwrap().next, 51);
        // Case-blind, as FAT is.
        let mut c = list(100);
        c.0[50] = "/GIFS/t/G50.GIF".into();
        assert_eq!(next(&mut c, &m).found, Some(50));
    }

    #[test]
    fn a_list_shrunk_below_it_starts_at_the_top() {
        let c0 = list(1000);
        let m = mark_at(&c0, 900);
        let mut c = list(40);
        let l = next(&mut c, &m);
        assert_eq!((l.next, l.found), (0, None), "not the last entry again");
        assert_eq!(c.1, 0, "the window is all past the end: no read");
        let mut c = list(895);
        let l = next(&mut c, &m);
        assert_eq!((l.next, l.found), (0, None));
        assert_eq!(l.probes, WINDOW - 5);
        // Its position still in the list: there.
        let mut c = list(901);
        c.0[900] = "/gifs/T/other.gif".into();
        assert_eq!(next(&mut c, &m).next, 900);
        // One entry, and none.
        let mut c = list(1);
        assert_eq!(next(&mut c, &m).next, 0);
        let mut c = list(1);
        let m0 = mark_at(&c, 0);
        assert_eq!(next(&mut c, &m0), Located { next: 0, found: Some(0), probes: 1, failed: false });
        let mut c = list(0);
        assert_eq!(locate(&mut c, &m, WINDOW, Step::Next), None);
    }

    #[test]
    fn the_lookup_is_bounded() {
        let mut c = list(10_000);
        let m = Mark {
            list: 1,
            index: 5000,
            entry: path_hash("/gifs/T/gone.gif"),
        };
        let l = next(&mut c, &m);
        assert_eq!((l.next, l.found), (5000, None));
        assert_eq!(c.1, 2 * WINDOW + 1, "at most 2 * WINDOW + 1 entries read");
    }

    #[test]
    fn the_lookup_stops_at_the_first_read_error() {
        let c0 = list(100);
        let m = mark_at(&c0, 50);
        // The entry moved two on, and the one at 51 will not read: no more reads.
        let mut c = list(100);
        c.0.insert(0, "/gifs/T/a.gif".into());
        c.0.insert(0, "/gifs/T/b.gif".into());
        c.2 = Some(51);
        let l = next(&mut c, &m);
        assert_eq!((l.next, l.found, l.failed, l.probes, c.1), (50, None, true, 2, 2));
        // At its own index: the clamp's fallback, the top past the end.
        let mut c = list(30);
        let m = Mark { index: 29, ..m };
        c.2 = Some(29);
        let l = next(&mut c, &m);
        assert_eq!((l.next, l.failed, c.1), (29, true, 1));
    }

    #[test]
    fn list_keys_name_the_filter_and_the_card() {
        let (a, b) = (path_hash("card-a"), path_hash("card-b"));
        let k = list_key("/filelist.txt", "", a);
        assert_eq!(k, list_key("/FILELIST.TXT", " ", a));
        assert_ne!(k, list_key("/filelist.txt", "NES", a));
        assert_ne!(k, list_key("/filelist.txt", "", b));
        assert_ne!(k, list_key("/filelist.txt", "", 0));
        assert_ne!(k, list_key("/sets/Night.txt", "", a));
        assert_ne!(list_key("/filelist.txt", "NES", 0), list_key("/filelist.txt", "", path_hash("NES")));
    }

    #[test]
    fn marks_round_trip_and_drop_garbage() {
        let mut ms = Marks::default();
        assert!(ms.is_empty());
        let a = Mark { list: 11, index: 3, entry: 7 };
        let b = Mark { list: 22, index: 9000, entry: 8 };
        ms.note(a);
        ms.note(b);
        let words = ms.encode();
        assert_eq!(Marks::decode(&words), ms);
        assert_eq!(Marks::decode(&words).get(11), Some(a));
        // All zeros (a cleared slot) and a power-on's garbage are not marks.
        assert!(Marks::decode(&[[0; WORDS]; SLOTS]).is_empty());
        let mut seed = 0x1234_5678u32;
        let mut junk = [[0; WORDS]; SLOTS];
        for w in junk.iter_mut().flatten() {
            seed = fmix(seed.wrapping_add(0x9e37_79b9));
            *w = seed;
        }
        assert!(Marks::decode(&junk).is_empty());
        // One word changed: that slot goes, the other stays.
        for k in 0..WORDS {
            let mut w = words;
            w[0][k] ^= 1 << (k * 7 % 32);
            let d = Marks::decode(&w);
            assert_eq!((d.get(22), d.get(11)), (None, Some(a)), "word {k}");
        }
        // Two slots for one playlist (a torn rewrite): the first wins.
        let mut w = words;
        w[1] = Mark { list: 22, index: 1, entry: 8 }.encode();
        assert_eq!(Marks::decode(&w).get(22), Some(b));
    }

    #[test]
    fn marks_keep_the_most_recent_playlists() {
        let mut ms = Marks::default();
        for l in 1..=SLOTS as u32 {
            ms.note(Mark { list: l, index: l, entry: 1 });
        }
        // Playing list 1 again moves it to the front, keeps the rest.
        ms.note(Mark { list: 1, index: 50, entry: 2 });
        assert_eq!(ms.0[0], Some(Mark { list: 1, index: 50, entry: 2 }));
        assert!((1..=SLOTS as u32).all(|l| ms.get(l).is_some()));
        // A new one pushes out the least recent (list 2).
        ms.note(Mark { list: 99, index: 0, entry: 1 });
        assert_eq!(ms.get(2), None);
        assert!(ms.get(1).is_some() && ms.get(99).is_some());
        assert_eq!(ms.0.iter().flatten().count(), SLOTS);
    }

    // -- the tracker (the firmware's RTC words) --------------------------------

    struct Rtc([u32; STORE_WORDS]);

    impl Store for Rtc {
        fn get(&self, i: usize) -> u32 {
            self.0[i]
        }
        fn put(&mut self, i: usize, v: u32) {
            self.0[i] = v;
        }
    }

    const A: &str = "/gifs/T/a.gif";

    /// Garbage, as RTC memory holds at a power-on.
    fn garbage() -> Rtc {
        let mut seed = 77u32;
        Rtc(core::array::from_fn(|_| {
            seed = fmix(seed.wrapping_add(1));
            seed
        }))
    }

    #[test]
    fn the_tracker_marks_playlist_starts_in_order_only() {
        let mut rtc = garbage();
        let mut t = Tracker::new();
        assert_eq!(t.boot(&mut rtc, true, false), Booted::Fresh);
        // No engine yet: nothing marked.
        t.started(&mut rtc, 3, A, true, 0);
        assert!(load(&rtc).is_empty());
        t.building(10, 0);
        assert_eq!(t.pending(&rtc), None, "nothing kept for it");
        assert_eq!(t.pending(&rtc), None);
        // A file played on request is not the playlist's place; a shuffle's start neither.
        t.started(&mut rtc, OFF_PLAYLIST, A, true, 100);
        t.started(&mut rtc, 5, A, false, 200);
        assert!(load(&rtc).is_empty());
        t.started(&mut rtc, 7, A, true, 300);
        assert_eq!(load(&rtc).get(10), Some(Mark { list: 10, index: 7, entry: path_hash(A) }));
        // A spell of shuffle leaves it.
        t.started(&mut rtc, 2, "/gifs/T/b.gif", false, 9_000);
        assert_eq!(load(&rtc).get(10).unwrap().index, 7);
        // Once per engine built.
        t.building(10, 20_000);
        assert_eq!(t.pending(&rtc), Some((load(&rtc).get(10).unwrap(), Step::Next)));
        assert_eq!(t.pending(&rtc), None);
        // Another playlist, and back: each its own.
        t.building(20, 21_000);
        t.started(&mut rtc, 1, A, true, 21_000);
        t.building(10, 40_000);
        assert_eq!(t.pending(&rtc).unwrap().0.index, 7);
        assert_eq!(load(&rtc).get(20).unwrap().index, 1);
    }

    #[test]
    fn a_rebuild_right_after_a_start_replays_it() {
        let mut rtc = garbage();
        let mut t = Tracker::new();
        t.boot(&mut rtc, true, false);
        t.building(10, 0);
        t.started(&mut rtc, 4, A, true, 10_000);
        // 2.9 s on: again; 3 s on: the next.
        t.building(10, 12_900);
        assert_eq!(t.pending(&rtc).unwrap().1, Step::Replay);
        t.building(10, 13_000);
        assert_eq!(t.pending(&rtc).unwrap().1, Step::Next);
        // Briefly up in another playlist: no replay for this one.
        t.building(20, 13_000);
        t.started(&mut rtc, 1, A, true, 13_000);
        t.building(10, 13_100);
        assert_eq!(t.pending(&rtc).unwrap().1, Step::Next);
        // Nor after a restart (the last start was the run before's).
        t.boot(&mut rtc, false, false);
        t.building(10, 0);
        assert_eq!(t.pending(&rtc).unwrap().1, Step::Next);
    }

    #[test]
    fn crash_boots_step_over_then_drop_the_marks() {
        let mut rtc = garbage();
        let mut t = Tracker::new();
        t.boot(&mut rtc, true, false);
        t.building(10, 0);
        t.started(&mut rtc, 4, A, true, 0);
        t.started(&mut rtc, 5, A, true, 9_000);
        // Entry 6 crashes the board before its first frame, every time.
        assert_eq!(t.boot(&mut rtc, false, true), Booted::Kept { marks: 1, crashes: 1 });
        t.building(10, 0);
        assert_eq!(t.pending(&rtc).unwrap(), (load(&rtc).get(10).unwrap(), Step::Next));
        assert_eq!(t.boot(&mut rtc, false, true), Booted::Kept { marks: 1, crashes: 2 });
        t.building(10, 0);
        assert_eq!(t.pending(&rtc).unwrap().1, Step::Over, "the second crash boot steps over it");
        // A rebuild before any start still steps over.
        t.building(10, 100);
        assert_eq!(t.pending(&rtc).unwrap().1, Step::Over);
        // That crashed too: the marks go.
        assert_eq!(t.boot(&mut rtc, false, true), Booted::Dropped { crashes: 3 });
        assert!(load(&rtc).is_empty());
        t.building(10, 0);
        assert_eq!(t.pending(&rtc), None, "from the top");
        // Two playlist starts clear the count; the next crash is the first again.
        t.started(&mut rtc, 0, A, true, 0);
        t.started(&mut rtc, 1, A, true, 9_000);
        assert_eq!(t.boot(&mut rtc, false, true), Booted::Kept { marks: 1, crashes: 1 });
        // One start does not.
        t.building(10, 0);
        t.started(&mut rtc, 2, A, true, 0);
        assert_eq!(t.boot(&mut rtc, false, true), Booted::Kept { marks: 1, crashes: 2 });
        // A planned restart, or a power-on, is no crash.
        assert_eq!(t.boot(&mut rtc, false, false), Booted::Kept { marks: 1, crashes: 0 });
        assert_eq!(t.boot(&mut rtc, true, true), Booted::Fresh);
        assert!(load(&rtc).is_empty());
        // Garbage in the count word reads as none.
        let mut rtc = garbage();
        assert!(matches!(t.boot(&mut rtc, false, true), Booted::Fresh | Booted::Kept { crashes: 1, .. }));
    }
}

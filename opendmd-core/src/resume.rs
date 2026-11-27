//! Where an in-order playlist left off, so a rebuild of the engine (an
//! upload, a reindex, a settings change, a schedule rule switching
//! playlists and back, a software restart) continues it instead of starting
//! again at the first entry.
//!
//! **What is kept.** A [`Mark`] per playlist: the playlist's key (a hash of
//! its manifest path, or of a Follow RePlayOS context's index path), the
//! catalog index of the last playlist entry that started and a hash of that
//! entry's path. The firmware keeps the last [`SLOTS`] of them (most recent
//! first, [`Marks`]) in RTC slow memory, rewritten at every start, so a
//! software restart finds them; a power-on finds garbage, which the check
//! word drops. Shuffle ignores them.
//!
//! **Finding the place again** ([`locate`]): the catalog is card-backed (an
//! `.idx` of offsets into the manifest; an entry costs a 4-byte read of the
//! index and a line of the manifest) and may hold 10,000 entries, so it is
//! never searched. The mark's own index is looked at first, then up to
//! [`WINDOW`] either side, nearest first: an upload or a delete usually
//! moves the position by a few entries, an append (what adding to a
//! playlist does) not at all. Found, playback continues with the entry after
//! it; not found within the window (the entry itself was removed, or moved
//! further), it continues at the same numeric position, clamped to the new
//! length. At most `2 * WINDOW + 1` lookups, only when an engine is built.
//!
//! Nothing here allocates beyond the catalog's own entry reads.

use crate::engine::Catalog;

/// Playlists remembered: the playing one and the few it switched from.
pub const SLOTS: usize = 4;
/// Words in an encoded [`Mark`]: key, index, entry hash, check.
pub const WORDS: usize = 4;
/// Entries looked at either side of a mark's index ([`locate`]).
pub const WINDOW: usize = 8;

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

/// Where a mark puts playback in a catalog ([`locate`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Located {
    /// The catalog index to play next.
    pub next: usize,
    /// Where the mark's entry was found, if it was.
    pub found: Option<usize>,
    /// Entries read to find it.
    pub probes: usize,
}

/// The catalog index to continue `mark` at in `catalog` (see the module
/// docs): after the mark's entry if it is within `window` of its old index,
/// else the old index clamped to the catalog. `None` for an empty catalog.
/// An entry that will not read counts as not it.
pub fn locate(catalog: &mut dyn Catalog, mark: &Mark, window: usize) -> Option<Located> {
    let len = catalog.len();
    if len == 0 {
        return None;
    }
    let at = mark.index as usize;
    let mut probes = 0;
    let mut is_it = |k: usize, probes: &mut usize| {
        *probes += 1;
        catalog.entry(k).is_ok_and(|e| path_hash(&e.path) == mark.entry)
    };
    for d in 0..=window {
        let ahead = at.checked_add(d).filter(|k| *k < len);
        let behind = at.checked_sub(d).filter(|k| d > 0 && *k < len);
        for k in [ahead, behind].into_iter().flatten() {
            if is_it(k, &mut probes) {
                return Some(Located {
                    next: (k + 1) % len,
                    found: Some(k),
                    probes,
                });
            }
        }
        // Past both ends of the catalog: nothing more to look at.
        if at.saturating_sub(d) == 0 && at.saturating_add(d) >= len - 1 {
            break;
        }
    }
    Some(Located {
        next: at.min(len - 1),
        found: None,
        probes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::Entry;
    use std::io;

    struct Paths(Vec<String>, usize);

    impl Catalog for Paths {
        fn len(&self) -> usize {
            self.0.len()
        }
        fn entry(&mut self, i: usize) -> io::Result<Entry> {
            self.1 += 1;
            Ok(Entry::from_path(&self.0[i]))
        }
    }

    fn list(n: usize) -> Paths {
        Paths((0..n).map(|i| format!("/gifs/T/g{i}.gif")).collect(), 0)
    }

    fn mark_at(c: &Paths, i: usize) -> Mark {
        Mark {
            list: path_hash("/filelist.txt"),
            index: i as u32,
            entry: path_hash(&c.0[i]),
        }
    }

    #[test]
    fn same_list_continues_after_the_entry() {
        let mut c = list(10_000);
        let m = mark_at(&c, 4321);
        let l = locate(&mut c, &m, WINDOW).unwrap();
        assert_eq!((l.next, l.found, l.probes), (4322, Some(4321), 1));
        // The last entry: the one after it is the first.
        let m = mark_at(&c, 9999);
        assert_eq!(locate(&mut c, &m, WINDOW).unwrap().next, 0);
    }

    #[test]
    fn entries_inserted_or_removed_before_it_are_followed() {
        let c0 = list(100);
        let m = mark_at(&c0, 50);
        // Two inserted before: g50 is now at 52.
        let mut c = list(100);
        c.0.insert(10, "/gifs/T/new1.gif".into());
        c.0.insert(10, "/gifs/T/new2.gif".into());
        let l = locate(&mut c, &m, WINDOW).unwrap();
        assert_eq!((l.next, l.found), (53, Some(52)));
        assert_eq!(c.0[l.next], "/gifs/T/g51.gif");
        // Three removed before: at 47.
        let mut c = list(100);
        c.0.drain(0..3);
        let l = locate(&mut c, &m, WINDOW).unwrap();
        assert_eq!(c.0[l.next], "/gifs/T/g51.gif");
        // Further than the window: the same numeric position.
        let mut c = list(100);
        c.0.drain(0..WINDOW + 1);
        let l = locate(&mut c, &m, WINDOW).unwrap();
        assert_eq!((l.next, l.found, l.probes), (50, None, 2 * WINDOW + 1));
    }

    #[test]
    fn its_own_entry_removed_continues_at_its_position() {
        let c0 = list(100);
        let m = mark_at(&c0, 50);
        let mut c = list(100);
        c.0.remove(50);
        let l = locate(&mut c, &m, WINDOW).unwrap();
        assert_eq!((l.next, l.found), (50, None));
        // Which is its successor.
        assert_eq!(c.0[50], "/gifs/T/g51.gif");
        // Case-blind, as FAT is.
        let mut c = list(100);
        c.0[50] = "/GIFS/t/G50.GIF".into();
        assert_eq!(locate(&mut c, &m, WINDOW).unwrap().found, Some(50));
    }

    #[test]
    fn a_list_shrunk_below_it_is_clamped() {
        let c0 = list(1000);
        let m = mark_at(&c0, 900);
        let mut c = list(40);
        let l = locate(&mut c, &m, WINDOW).unwrap();
        assert_eq!((l.next, l.found), (39, None));
        // Nothing in reach was read twice, and nothing out of range.
        assert_eq!(c.1, 0, "the window is all past the end: no read");
        let mut c = list(895);
        let l = locate(&mut c, &m, WINDOW).unwrap();
        assert_eq!((l.next, l.found), (894, None));
        assert_eq!(l.probes, WINDOW - 5);
        // One entry, and none.
        let mut c = list(1);
        assert_eq!(locate(&mut c, &m, WINDOW).unwrap().next, 0);
        let mut c = list(1);
        let m0 = mark_at(&c, 0);
        assert_eq!(locate(&mut c, &m0, WINDOW).unwrap(), Located { next: 0, found: Some(0), probes: 1 });
        let mut c = list(0);
        assert_eq!(locate(&mut c, &m, WINDOW), None);
    }

    #[test]
    fn the_lookup_is_bounded() {
        let mut c = list(10_000);
        let m = Mark {
            list: 1,
            index: 5000,
            entry: path_hash("/gifs/T/gone.gif"),
        };
        let l = locate(&mut c, &m, WINDOW).unwrap();
        assert_eq!((l.next, l.found), (5000, None));
        assert_eq!(c.1, 2 * WINDOW + 1, "at most 2 * WINDOW + 1 entries read");
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
}

//! What the panel played recently: the device's **Recently played** list
//! (`/api/status?recent=1` `recent`) and its **Previous** (`POST
//! /api/skip?back=1`).
//!
//! A most-recently-played list of distinct paths, newest first, at most
//! [`LEN`] of them in at most [`BYTES`] of text: one `String`, allocated
//! once at that capacity on the first start and never grown, entries oldest
//! first and each ended by `\n`. A GIF's `/gifs/<rest>.gif` is kept as
//! `<rest>` (a stored entry starting with `/` or `@` is a whole path), so a
//! typical entry is ~35 B and sixteen fit.
//!
//! **Previous walks back, it does not ping-pong.** [`History::back`] names
//! the entry one older than the one on screen (the *cursor*, counted back
//! from the newest) and remembers it as pending; when that file starts,
//! [`History::started`] moves the cursor there instead of recording it, so
//! pressing Previous again goes one further. Any other start (the schedule,
//! Next, a tap on an entry) first brings the entry the cursor was on to the
//! front — it was the last thing on screen — then records the new one, and
//! the cursor is back at the newest. A path already listed moves to the
//! front rather than appearing twice.

/// At most this many entries.
pub const LEN: usize = 16;
/// At most this many bytes of stored text (entries and their newlines).
pub const BYTES: usize = 640;
/// A path whose stored form is longer than this is not recorded (a long
/// scrolling text), so one entry cannot push out half the list.
pub const MAX_ENTRY: usize = 200;

const NONE: u8 = u8::MAX;

/// See the module docs.
#[derive(Clone, Debug)]
pub struct History {
    buf: String,
    cursor: u8,
    pending: u8,
}

impl Default for History {
    fn default() -> Self {
        Self::new()
    }
}

/// The stored form of `path`, or `None` if it is not recorded: Follow
/// RePlayOS's power-on effect and title cards (`@power-on`, `@card:…`) are
/// not things a user plays, and a path must be card-absolute or a live item.
fn compact(path: &str) -> Option<&str> {
    if path.contains('\n') || path == "@power-on" || path.starts_with("@card:") {
        return None;
    }
    let s = match path
        .strip_prefix("/gifs/")
        .and_then(|r| r.strip_suffix(".gif"))
    {
        Some(r) if !r.is_empty() && !r.starts_with(['/', '@']) => r,
        _ if path.starts_with(['/', '@']) => path,
        _ => return None,
    };
    (s.len() <= MAX_ENTRY).then_some(s)
}

/// The path a stored entry stands for.
fn expand(s: &str) -> String {
    if s.starts_with(['/', '@']) {
        s.to_string()
    } else {
        format!("/gifs/{s}.gif")
    }
}

impl History {
    pub const fn new() -> Self {
        History {
            buf: String::new(),
            cursor: 0,
            pending: NONE,
        }
    }

    /// Stored entries, newest first.
    fn stored(&self) -> impl Iterator<Item = &str> {
        self.buf.lines().rev()
    }

    /// How many entries there are.
    pub fn len(&self) -> usize {
        self.buf.bytes().filter(|&b| b == b'\n').count()
    }

    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    /// The entry on screen after [`back`](Self::back)s, counted back from the
    /// newest (0 = the newest).
    pub fn cursor(&self) -> usize {
        self.cursor as usize
    }

    /// The paths, newest first.
    pub fn paths(&self) -> impl Iterator<Item = String> + '_ {
        self.stored().map(expand)
    }

    /// The byte range of the `i`th entry back from the newest, its newline
    /// included.
    fn range(&self, i: usize) -> Option<std::ops::Range<usize>> {
        let mut end = self.buf.len();
        for (k, line) in self.stored().enumerate() {
            let start = end - line.len() - 1;
            if k == i {
                return Some(start..end);
            }
            end = start;
        }
        None
    }

    /// Take the `i`th entry out and put it at the front (the newest).
    fn bring_forward(&mut self, i: usize) {
        if i == 0 {
            return;
        }
        if let Some(r) = self.range(i) {
            // In place, no allocation (this runs on the display loop).
            // SAFETY: the rotation moves one whole entry, newline included,
            // past whole entries, so the bytes stay valid UTF-8.
            unsafe { self.buf.as_mut_vec()[r.start..].rotate_left(r.len()) };
        }
    }

    /// The panel started showing `path`.
    pub fn started(&mut self, path: &str) {
        let Some(s) = compact(path) else {
            return;
        };
        let pending = std::mem::replace(&mut self.pending, NONE);
        if pending != NONE && self.stored().nth(pending as usize) == Some(s) {
            self.cursor = pending;
            return;
        }
        let cursor = std::mem::take(&mut self.cursor) as usize;
        self.bring_forward(cursor);
        let listed = self.stored().position(|e| e == s);
        if let Some(i) = listed {
            self.bring_forward(i);
            return;
        }
        if self.buf.capacity() == 0 {
            self.buf.reserve_exact(BYTES);
        }
        // Make room (the oldest go), so the buffer never grows.
        while self.len() >= LEN || self.buf.len() + s.len() + 1 > BYTES {
            let Some(nl) = self.buf.find('\n') else { break };
            self.buf.replace_range(..=nl, "");
        }
        self.buf.push_str(s);
        self.buf.push('\n');
    }

    /// Previous: the path one older than the one on screen (or than the one
    /// a previous `back` already asked for and that has not started yet),
    /// remembered so its start moves the cursor rather than being recorded.
    /// `None` at the oldest.
    pub fn back(&mut self) -> Option<String> {
        let from = if self.pending != NONE {
            self.pending
        } else {
            self.cursor
        };
        let target = from as usize + 1;
        let path = self.stored().nth(target).map(expand)?;
        self.pending = target as u8;
        Some(path)
    }

    /// The file [`back`](Self::back) named did not open: nothing moves.
    pub fn back_failed(&mut self) {
        self.pending = NONE;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn g(name: &str) -> String {
        format!("/gifs/Consoles/{name}.gif")
    }

    fn list(h: &History) -> Vec<String> {
        h.paths().collect()
    }

    #[test]
    fn records_newest_first_compactly() {
        let mut h = History::new();
        assert!(h.is_empty());
        h.started(&g("A"));
        h.started("@clock?seconds=1");
        h.started("/cache/X/ab/B.gif");
        h.started("/gifs/C.GIF");
        assert_eq!(
            list(&h),
            [
                "/gifs/C.GIF",
                "/cache/X/ab/B.gif",
                "@clock?seconds=1",
                &g("A")
            ]
        );
        assert_eq!(
            h.buf,
            "Consoles/A\n@clock?seconds=1\n/cache/X/ab/B.gif\n/gifs/C.GIF\n"
        );
        assert_eq!(h.len(), 4);
        assert_eq!(h.cursor(), 0);
    }

    #[test]
    fn skips_what_a_user_does_not_play() {
        let mut h = History::new();
        h.started("@power-on");
        h.started("@card:Sonic|Mega Drive");
        h.started("relative.gif");
        h.started("/gifs/a\nb.gif");
        h.started(&format!("@text?msg={}", "x".repeat(MAX_ENTRY)));
        assert!(h.is_empty());
        assert_eq!(
            h.buf.capacity(),
            0,
            "nothing allocated until a start is recorded"
        );
    }

    #[test]
    fn a_repeat_moves_to_the_front() {
        let mut h = History::new();
        for n in ["A", "B", "C", "B"] {
            h.started(&g(n));
        }
        assert_eq!(list(&h), [g("B"), g("C"), g("A")]);
        h.started(&g("B"));
        assert_eq!(list(&h), [g("B"), g("C"), g("A")]);
    }

    #[test]
    fn bounded_by_count_and_bytes_without_growing() {
        let mut h = History::new();
        for i in 0..40 {
            h.started(&g(&format!("F{i:02}")));
        }
        assert_eq!(h.len(), LEN);
        assert_eq!(h.paths().next().unwrap(), g("F39"));
        assert_eq!(h.paths().last().unwrap(), g("F24"));
        assert_eq!(h.buf.capacity(), BYTES);
        let long = "y".repeat(MAX_ENTRY - 20);
        for i in 0..10 {
            h.started(&format!("@text?msg={long}{i}"));
            assert!(h.buf.len() <= BYTES);
        }
        assert_eq!(h.len(), 3, "three 192 B entries fit in 640 B");
        assert_eq!(h.buf.capacity(), BYTES);
    }

    #[test]
    fn previous_walks_back_and_does_not_ping_pong() {
        let mut h = History::new();
        for n in ["A", "B", "C"] {
            h.started(&g(n));
        }
        let p = h.back().unwrap();
        assert_eq!(p, g("B"));
        h.started(&p);
        assert_eq!(h.cursor(), 1);
        let p = h.back().unwrap();
        assert_eq!(p, g("A"));
        h.started(&p);
        assert_eq!(h.cursor(), 2);
        assert_eq!(
            list(&h),
            [g("C"), g("B"), g("A")],
            "walking back reorders nothing"
        );
        assert_eq!(h.back(), None, "nothing older");
        assert_eq!(h.cursor(), 2);
        // The schedule moves on: what was on screen (A) comes to the front,
        // then the new file, and Previous from it is A.
        h.started(&g("X"));
        assert_eq!(h.cursor(), 0);
        assert_eq!(list(&h), [g("X"), g("A"), g("C"), g("B")]);
        assert_eq!(h.back().unwrap(), g("A"));
    }

    #[test]
    fn two_presses_before_the_first_starts_go_two_back() {
        let mut h = History::new();
        for n in ["A", "B", "C"] {
            h.started(&g(n));
        }
        assert_eq!(h.back().unwrap(), g("B"));
        assert_eq!(h.back().unwrap(), g("A"));
        h.started(&g("A"));
        assert_eq!(h.cursor(), 2);
    }

    #[test]
    fn a_failed_or_overtaken_back_moves_nothing() {
        let mut h = History::new();
        for n in ["A", "B", "C"] {
            h.started(&g(n));
        }
        h.back().unwrap();
        h.back_failed();
        assert_eq!(h.back().unwrap(), g("B"), "still one back from the newest");
        // Something else started before B did: recorded as usual.
        h.started("@clock");
        assert_eq!(h.cursor(), 0);
        assert_eq!(list(&h)[0], "@clock");
        assert_eq!(h.back().unwrap(), g("C"));
    }

    #[test]
    fn replaying_the_entry_on_screen_keeps_one_copy() {
        let mut h = History::new();
        for n in ["A", "B", "C"] {
            h.started(&g(n));
        }
        let p = h.back().unwrap();
        h.started(&p);
        // Play again (POST /api/play of B): B to the front, once.
        h.started(&g("B"));
        assert_eq!(list(&h), [g("B"), g("C"), g("A")]);
        assert_eq!(h.cursor(), 0);
    }
}

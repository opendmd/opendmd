//! What was played by hand and is kept on the panel until something else
//! plays ([`crate::engine::hand_picked`]): which start it is, and what each
//! event does to it. The firmware holds one in its status and asks here.
//!
//! The rule: a kept GIF or Formula ends when the user changes what plays
//! (another play, Skip or Next, Previous, choosing another playlist) or the
//! playlist changes under it (a schedule's playlist, Follow RePlayOS), and
//! when the panel turns off for the night (in the morning the playlist
//! plays). It survives a pause and a rebuild of the same playlist (an edit
//! of it, a reindex, a platform filter, an override or a favourite): it is
//! played again, kept, on the rebuilt engine.

/// A start kept on the panel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Kept {
    /// The engine's ordinal of the start.
    pub ordinal: u64,
    /// What it plays, to play it again after a rebuild.
    pub path: String,
}

/// What happens to the panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// Paused or resumed.
    Pause,
    /// The engine is rebuilt over a playlist: the same one (`true`) or
    /// another.
    Rebuild { same_playlist: bool },
    /// The schedule turned the panel off for the night.
    NightOff,
    /// Another start: `ordinal` (a play, Skip, Previous, the playlist).
    Started { ordinal: u64 },
}

/// What becomes of a kept start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fate {
    /// It stays as it is.
    Stays,
    /// Played again, kept, on the rebuilt engine (a new start there).
    PlayAgain,
    /// It ends: nothing is kept.
    Ends,
}

impl Kept {
    /// Kept, and the start on screen (`ordinal`).
    pub fn on_screen(&self, ordinal: u64) -> bool {
        self.ordinal == ordinal
    }

    /// What `event` does to it.
    pub fn fate(&self, event: Event) -> Fate {
        match event {
            Event::Pause => Fate::Stays,
            Event::Rebuild { same_playlist: true } => Fate::PlayAgain,
            Event::Rebuild { same_playlist: false } | Event::NightOff => Fate::Ends,
            Event::Started { ordinal } if ordinal == self.ordinal => Fate::Stays,
            Event::Started { .. } => Fate::Ends,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k() -> Kept {
        Kept { ordinal: 7, path: "/gifs/A/a.gif".into() }
    }

    #[test]
    fn the_rule_table() {
        let k = k();
        assert!(k.on_screen(7) && !k.on_screen(8));
        assert_eq!(k.fate(Event::Pause), Fate::Stays, "a pause and its resume keep it");
        assert_eq!(k.fate(Event::Rebuild { same_playlist: true }), Fate::PlayAgain, "an edit, a reindex, a filter, a favourite");
        assert_eq!(k.fate(Event::Rebuild { same_playlist: false }), Fate::Ends, "another playlist, a schedule's, Follow's");
        assert_eq!(k.fate(Event::NightOff), Fate::Ends, "in the morning the playlist plays");
        assert_eq!(k.fate(Event::Started { ordinal: 7 }), Fate::Stays);
        assert_eq!(k.fate(Event::Started { ordinal: 8 }), Fate::Ends, "Skip, Previous, another play");
    }
}

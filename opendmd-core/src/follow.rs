//! "Follow RePlayOS": the pure half — which platform a RePlayOS system is,
//! how well a GIF's title matches the game being played, and the state
//! machine that turns a stream of `get_status` polls into what the panel
//! should do. Everything with a socket, a file or a clock lives in the
//! firmware (`firmware/src/follow.rs`); this is host-tested.
//!
//! # The state machine
//!
//! | state | panel | left by |
//! |---|---|---|
//! | `Off` | normal (playlist, schedule, night mode) | enabling |
//! | `Unreachable` | normal | a successful poll |
//! | `Attract` | the attract playlist | `view == game_play` |
//! | `Game` | the sequence, then the context playlist | leaving the game (-> `Cooldown`), another game (-> `Game`) |
//! | `Cooldown` | still the game's playlist | [`GRACE`] with no game (-> `Attract`), the game again (-> `Game`, no restart) |
//!
//! `paused: true` changes nothing, whatever the view (RePlayOS's pause menu
//! is the `game_options` view), and neither do the in-game overlays
//! (`game_options`, `achievements`, `leaderboards`) while a game is loaded.
//! `halted` counts as out of the game. A reachable Pi in a menu is
//! `Attract`. Polls that fail move a following panel back to normal only
//! after [`FAILURES_TO_DROP`] in a row (one lost reply must not end a game);
//! a panel that never reached the Pi is `Unreachable` from the first. The
//! firmware shows `Game` as `starting` while its sequence runs
//! ([`state_name`]).

use crate::manifest::Entry;
use crate::overrides::{Override, Overrides};
use crate::text::fold_char;
use std::time::Duration;

/// Seconds with no game before a following panel returns to attract.
pub const GRACE: Duration = Duration::from_secs(20);
/// Polling interval while the Pi answers...
pub const POLL: Duration = Duration::from_secs(2);
/// ...and while it does not.
pub const BACKOFF: Duration = Duration::from_secs(15);
/// Failed polls in a row before a following panel gives up on the Pi.
pub const FAILURES_TO_DROP: u32 = 3;

/// Score at or above which an entry's title counts as this game
/// ([`Match::Strong`]).
pub const STRONG: f32 = 0.6;
/// Score at or above which it is related (another game of the series, say:
/// [`Match::Related`]).
pub const WEAK: f32 = 0.35;

/// Share of a context playlist's draws, in percent, the game's matches
/// (strong and related together) get. Strong takes as much of it as
/// [`MAX_WEIGHT`] and the no-repeat window let it; related gets the rest.
pub const MATCH_PCT: u32 = 60;
/// No entry of a context playlist weighs more than this. The scheduler's
/// draw is rejection sampling against the largest weight, so this also
/// bounds how many tries a draw takes (~1-2% of tries accept at 255 on a
/// 600-1,000 entry platform: a failed draw — 1,024 rejections — is ~1e-8).
pub const MAX_WEIGHT: u32 = 255;

/// The no-repeat window of a context playlist with `n_matches` strong +
/// related entries: `clamp(n_matches - 1, 4, 32)`, so a small match set can
/// cycle (the global window is 32, which would let one strong GIF play at
/// most once every 33 draws).
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn context_window(n_matches: usize) -> usize {
    n_matches.saturating_sub(1).clamp(4, 32)
}

const STRONG_MARK: f32 = 2.0;
const RELATED_MARK: f32 = 1.5;

/// How an entry's title relates to the game being played.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Match {
    /// The game's own art ([`STRONG`] or better).
    Strong,
    /// Related ([`WEAK`] or better): another game of the series, say.
    Related,
}

impl Match {
    /// The mark a context index's pass leaves on a match, before
    /// [`weigh_context`] knows the counts and replaces it with the weight.
    /// (Never seen outside the pass; positive so that an index written
    /// without the second step still plays every entry.)
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn mark(self) -> Override {
        Override {
            weight: Some(match self {
                Match::Strong => STRONG_MARK,
                Match::Related => RELATED_MARK,
            }),
            ..Override::default()
        }
    }

    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    fn of_mark(ov: &Override) -> Option<Match> {
        match ov.weight {
            Some(STRONG_MARK) => Some(Match::Strong),
            Some(RELATED_MARK) => Some(Match::Related),
            _ => None,
        }
    }
}

/// Which class a [`title_score`] puts an entry in (`None`: the rest of the
/// platform, weight 1).
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn classify(score: f32) -> Option<Match> {
    if score >= STRONG {
        Some(Match::Strong)
    } else if score >= WEAK {
        Some(Match::Related)
    } else {
        None
    }
}

/// The per-entry weights of a context playlist, from how many entries it
/// has of each kind and its no-repeat window ([`context_window`]). The rest
/// of the platform weighs 1; the matches together should get [`MATCH_PCT`]
/// of the draws — strong first, as far as [`MAX_WEIGHT`] and the window
/// allow, then related (at 1 if strong alone gets there). Weights are whole
/// numbers, and all of this is integer arithmetic (see "Gotchas" in
/// CLAUDE.md: the Xtensa backend and float constants).
///
/// The model behind it (`shares_ppm`), for weights `w` over `k` entries of
/// a class and a window `W`: at steady state `W * s` of a class's entries
/// sit in the window (`s` its share of draws), so with `E` the weight of
/// every entry *not* in the window, `s = w k / (E + w W)` — which tends to
/// `k / W` however large `w` gets. `E` is whatever makes the shares sum to
/// 1. The unit tests check it against the scheduler itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ContextWeights {
    pub strong: u32,
    pub related: u32,
    /// The matches can't reach [`MATCH_PCT`] even at [`MAX_WEIGHT`].
    pub capped: bool,
    /// The no-repeat window the playlist plays with.
    pub window: u32,
    pub n_strong: u32,
    pub n_related: u32,
    /// Entries in the playlist, matches included.
    pub total: u32,
}

/// Fixed-point scale of `E` in [`ContextWeights::shares_ppm`].
const E_ONE: u64 = 256;

impl ContextWeights {
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn for_counts(total: u32, n_strong: u32, n_related: u32) -> Self {
        let window = context_window((n_strong + n_related) as usize) as u32;
        let mut w = ContextWeights {
            strong: 1,
            related: 1,
            capped: false,
            window,
            n_strong,
            n_related,
            total,
        };
        let target = MATCH_PCT * 10_000;
        let matched = |w: &ContextWeights| {
            let [s, r, _] = w.shares_ppm();
            s + r
        };
        if n_strong + n_related == 0 || matched(&w) >= target {
            return w;
        }
        // The smallest weight in 1..=MAX_WEIGHT at which the matches reach
        // the target (they only grow with it), or None.
        let search = |w: &mut ContextWeights, strong: bool| -> bool {
            let set = |w: &mut ContextWeights, v: u32| {
                if strong {
                    w.strong = v
                } else {
                    w.related = v
                }
            };
            set(w, MAX_WEIGHT);
            if matched(w) < target {
                return false;
            }
            let (mut lo, mut hi) = (1u32, MAX_WEIGHT);
            while lo < hi {
                let mid = (lo + hi) / 2;
                set(w, mid);
                if matched(w) >= target {
                    hi = mid;
                } else {
                    lo = mid + 1;
                }
            }
            set(w, lo);
            true
        };
        if n_strong > 0 && search(&mut w, true) {
            return w;
        }
        if n_related > 0 && search(&mut w, false) {
            return w;
        }
        w.capped = true;
        w
    }

    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn weight(&self, m: Match) -> u32 {
        match m {
            Match::Strong => self.strong,
            Match::Related => self.related,
        }
    }

    /// The expected share of draws of `(strong, related, rest)`, in parts
    /// per million, with these weights and the window (see the type's doc).
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn shares_ppm(&self) -> [u32; 3] {
        let n_rest = self.total.saturating_sub(self.n_strong + self.n_related);
        // The scheduler never lets the window exclude every entry.
        let win = u64::from(self.window.min(self.total.saturating_sub(1)));
        let classes = [
            (u64::from(self.strong), u64::from(self.n_strong)),
            (u64::from(self.related), u64::from(self.n_related)),
            (1, u64::from(n_rest)),
        ];
        let share = |e: u64, (w, k): (u64, u64)| {
            if k == 0 {
                0
            } else {
                w * k * E_ONE * 1_000_000 / (e + w * win * E_ONE).max(1)
            }
        };
        let sum = |e: u64| classes.iter().map(|c| share(e, *c)).sum::<u64>();
        // The shares fall as E grows: the largest E whose shares still sum
        // to at least 1.
        let (mut lo, mut hi) = (
            0u64,
            classes.iter().map(|(w, k)| w * k).sum::<u64>() * E_ONE,
        );
        if hi == 0 {
            return [0; 3];
        }
        while lo < hi {
            let mid = (lo + hi).div_ceil(2);
            if sum(mid) >= 1_000_000 {
                lo = mid;
            } else {
                hi = mid - 1;
            }
        }
        classes.map(|c| share(lo, c).min(1_000_000) as u32)
    }

    /// [`Self::shares_ppm`] as fractions.
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn shares(&self) -> (f32, f32, f32) {
        let [s, r, o] = self.shares_ppm();
        (s as f32 / 1e6, r as f32 / 1e6, o as f32 / 1e6)
    }
}

/// The second half of a context index's pass (the `finish` of
/// [`crate::index::write_index_resolving_then`]): count the [`Match::mark`]s
/// among `count` entries and replace each with its class's weight.
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn weigh_context(count: u32, resolved: &mut Overrides) -> ContextWeights {
    let (mut s, mut r) = (0u32, 0u32);
    for (_, ov) in resolved.iter() {
        match Match::of_mark(ov) {
            Some(Match::Strong) => s += 1,
            Some(Match::Related) => r += 1,
            None => {}
        }
    }
    let w = ContextWeights::for_counts(count, s, r);
    for ov in resolved.values_mut() {
        if let Some(m) = Match::of_mark(ov) {
            // Through `black_box`: an integer choice converted to a float
            // is folded into a table of float constants, which the Xtensa
            // backend cannot compile (see CLAUDE.md "Gotchas").
            ov.weight = Some(std::hint::black_box(w.weight(m)) as f32);
        }
    }
    w
}

/// RePlayOS system ids (`<company>_<system>`) to the tag grammar's
/// platforms. Any `arcade_*` is `ARCADE`.
pub const SYSTEMS: &[(&str, &str)] = &[
    ("sega_smd", "MD"),
    ("nintendo_snes", "SNES"),
    ("nintendo_gb", "GB"),
    ("nintendo_gbc", "GBC"),
    ("nintendo_gba", "GBA"),
    ("sega_st", "SATURN"),
    ("nintendo_ds", "NDS"),
    ("nintendo_nes", "NES"),
    ("sega_sms", "SMS"),
    ("nec_pce", "PCE"),
    ("sony_psx", "PS1"),
    ("nintendo_n64", "N64"),
    ("sega_gg", "GAMEGEAR"),
    ("snk_ngp", "NGP"),
];

/// The platform tag for a RePlayOS system id, or `None` if the pack has no
/// platform for it (the panel then follows with the console's name only).
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn platform_for_system(system: &str) -> Option<&'static str> {
    let system = system.trim();
    if system
        .get(..7)
        .is_some_and(|p| p.eq_ignore_ascii_case("arcade_"))
    {
        return Some("ARCADE");
    }
    SYSTEMS
        .iter()
        .find(|(id, _)| id.eq_ignore_ascii_case(system))
        .map(|(_, p)| *p)
}

/// What a title card calls a platform.
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn console_name(platform: &str) -> &'static str {
    match platform {
        "MD" => "Mega Drive",
        "SNES" => "Super Nintendo",
        "GB" => "Game Boy",
        "GBC" => "Game Boy Color",
        "GBA" => "Game Boy Advance",
        "SATURN" => "Saturn",
        "NDS" => "Nintendo DS",
        "NES" => "NES",
        "SMS" => "Master System",
        "PCE" => "PC Engine",
        "PS1" => "PlayStation",
        "N64" => "Nintendo 64",
        "GAMEGEAR" => "Game Gear",
        "NGP" => "Neo Geo Pocket",
        "ARCADE" => "Arcade",
        // Not RePlayOS systems: for the web app's Platform menus.
        "NGPC" => "Neo Geo Pocket Color",
        "NEOGEO" => "Neo Geo",
        "GAMECUBE" => "GameCube",
        "WII" => "Wii",
        "SWITCH" => "Switch",
        "3DS" => "Nintendo 3DS",
        "VIRTUAL_BOY" => "Virtual Boy",
        "SG1000" => "SG-1000",
        "32X" => "32X",
        "SEGA_MEGACD" => "Mega-CD",
        "DREAMCAST" => "Dreamcast",
        "PS" | "PSX" => "PlayStation",
        "PS2" => "PlayStation 2",
        "PS3" => "PlayStation 3",
        "PS4" => "PlayStation 4",
        "VITA" => "PS Vita",
        "LYNX" => "Lynx",
        "JAGUAR" => "Jaguar",
        "WONDERSWAN" => "WonderSwan",
        "COLECOVISION" => "ColecoVision",
        "INTELLIVISION" => "Intellivision",
        "VECTREX" => "Vectrex",
        "AMIGA" => "Amiga",
        "C64" => "Commodore 64",
        "ATARIST" => "Atari ST",
        "ATARI800" => "Atari 800",
        "AMSTRAD" => "Amstrad CPC",
        "SPECTRUM" | "ZX" => "ZX Spectrum",
        "X68000" => "Sharp X68000",
        _ => "",
    }
}

/// The console name for a system id RePlayOS reported, falling back to the
/// id itself (`sega_32x` -> `sega 32x`) for systems without a platform.
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn console_name_for_system(system: &str) -> String {
    match platform_for_system(system).map(console_name) {
        Some(n) if !n.is_empty() => n.to_string(),
        _ => system.replace('_', " "),
    }
}

/// A game name as a card shows it: bracketed tags (`(USA)`, `[!]`, `{…}`)
/// dropped, No-Intro's trailing article moved to the front (`Legend of
/// Zelda, The - A Link…` -> `The Legend of Zelda - A Link…`), whitespace
/// collapsed.
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn clean_game_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut depth = 0u32;
    for c in name.chars() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    let words: Vec<&str> = out.split_whitespace().collect();
    let mut s = words.join(" ");
    // "X, The" or "X, The - Y": the article belongs in front.
    for art in ["The", "A", "An"] {
        let pat = format!(", {art}");
        if let Some(i) = s.find(&pat) {
            let after = &s[i + pat.len()..];
            if after.is_empty() || after.starts_with(" -") || after.starts_with(':') {
                s = format!("{art} {}{}", &s[..i], after);
                break;
            }
        }
    }
    s
}

/// Words that say nothing about which game it is.
const STOPWORDS: &[&str] = &["the", "a", "an", "and", "of"];

/// Roman numerals and number words, as digits.
#[cfg_attr(target_arch = "xtensa", optimize(size))]
fn numeral(word: &str) -> Option<&'static str> {
    Some(match word {
        "one" => "1",
        "two" | "ii" => "2",
        "three" | "iii" => "3",
        "four" | "iv" => "4",
        "five" => "5",
        "six" | "vi" => "6",
        "seven" | "vii" => "7",
        "eight" | "viii" => "8",
        "nine" | "ix" => "9",
        "ten" => "10",
        _ => return None,
    })
}

/// A title as matching tokens: bracketed tags dropped, split at anything
/// that is not a letter or digit *and* at CamelCase and letter/digit
/// boundaries (`Sonic3` -> `sonic 3`, `StreetsOfRage` -> `streets of
/// rage`), lowercased, accents folded, numerals as digits (`II`, `two` ->
/// `2`; leading zeros dropped), stopwords dropped, duplicates removed.
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn tokens(title: &str) -> Vec<String> {
    let mut words: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut prev: Option<char> = None;
    let mut depth = 0u32;
    let chars: Vec<char> = title.chars().map(fold_char).collect();
    for (i, &c) in chars.iter().enumerate() {
        match c {
            '(' | '[' | '{' => {
                depth += 1;
                flush(&mut cur, &mut words);
                prev = None;
                continue;
            }
            ')' | ']' | '}' => {
                depth = depth.saturating_sub(1);
                prev = None;
                continue;
            }
            _ if depth > 0 => continue,
            _ => {}
        }
        if !c.is_ascii_alphanumeric() {
            flush(&mut cur, &mut words);
            prev = None;
            continue;
        }
        if let Some(p) = prev {
            let next_lower = chars.get(i + 1).is_some_and(|n| n.is_ascii_lowercase());
            let boundary = (p.is_ascii_lowercase() && c.is_ascii_uppercase())
                || (p.is_ascii_digit() != c.is_ascii_digit())
                || (p.is_ascii_uppercase() && c.is_ascii_uppercase() && next_lower);
            if boundary {
                flush(&mut cur, &mut words);
            }
        }
        cur.push(c.to_ascii_lowercase());
        prev = Some(c);
    }
    flush(&mut cur, &mut words);
    let mut out: Vec<String> = Vec::with_capacity(words.len());
    for w in words {
        if STOPWORDS.contains(&w.as_str()) {
            continue;
        }
        let w = match numeral(&w) {
            Some(d) => d.to_string(),
            None if w.bytes().all(|b| b.is_ascii_digit()) => {
                let t = w.trim_start_matches('0');
                if t.is_empty() { "0" } else { t }.to_string()
            }
            None => w,
        };
        if !out.contains(&w) {
            out.push(w);
        }
    }
    out
}

#[cfg_attr(target_arch = "xtensa", optimize(size))]
fn flush(cur: &mut String, words: &mut Vec<String>) {
    if !cur.is_empty() {
        words.push(std::mem::take(cur));
    }
}

#[cfg_attr(target_arch = "xtensa", optimize(size))]
fn is_number(t: &str) -> bool {
    t.bytes().all(|b| b.is_ascii_digit())
}

/// How well an entry's title matches the game being played, 0..=1: the
/// Dice coefficient of their [`tokens`] (1 as well when the tokens run
/// together are the same string: `Megaman` / `Mega Man`), scaled down
///
/// - when the numbers disagree: a different sequel number ×0.5, the game
///   has a number and the entry none ×0.7, the other way round ×0.8;
/// - when *both* sides have words the other lacks (`Super Mario Kart`
///   against `Super Mario World`: another game of the series) ×0.75.
///
/// "Sonic3" against "Sonic the Hedgehog 3 (USA)" is 0.8 (strong), "Sonic
/// The Hedgehog" 0.56 (related), "Sonic The Hedgehog 2" 0.33 (neither);
/// "Super Mario Kart" against "Super Mario World" 0.5 (related).
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn title_score(entry_title: &str, game_name: &str) -> f32 {
    score_tokens(&tokens(entry_title), &tokens(game_name))
}

/// [`title_score`] on tokens already computed (the game's once per build).
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn score_tokens(entry: &[String], game: &[String]) -> f32 {
    if entry.is_empty() || game.is_empty() {
        return 0.0;
    }
    let common = game.iter().filter(|t| entry.contains(t)).count();
    let mut dice = 2.0 * common as f32 / (entry.len() + game.len()) as f32;
    if entry.concat() == game.concat() {
        dice = 1.0;
    }
    let gn: Vec<&String> = game.iter().filter(|t| is_number(t)).collect();
    let en: Vec<&String> = entry.iter().filter(|t| is_number(t)).collect();
    let mut factor = match (gn.is_empty(), en.is_empty()) {
        (false, false) if !gn.iter().any(|n| en.contains(n)) => 0.5,
        (false, true) => 0.7,
        (true, false) => 0.8,
        _ => 1.0,
    };
    let game_extra = game.iter().any(|t| !is_number(t) && !entry.contains(t));
    let entry_extra = entry.iter().any(|t| !is_number(t) && !game.contains(t));
    if game_extra && entry_extra && dice < 1.0 {
        factor *= 0.75;
    }
    dice * factor
}

/// Is this entry a logo? (`MD_LOGO_Sega31`: the `LOGO` flag, or `logo` in
/// the title.)
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn is_logo(entry: &Entry) -> bool {
    entry.has_flag("LOGO") || entry.title.to_ascii_lowercase().contains("logo")
}

/// Words that make a logo a console maker's rather than a game's.
const CONSOLE_WORDS: &[&str] = &[
    "sega",
    "nintendo",
    "sony",
    "playstation",
    "snk",
    "neogeo",
    "nec",
    "hudson",
    "atari",
    "bandai",
    "genesis",
    "megadrive",
    "snes",
    "famicom",
    "saturn",
    "console",
];

/// Which of a platform's logo entries (`(title, path)`, manifest order) to
/// show before a game: one whose title matches this game (its own logo —
/// `SNES_LOGO_SuperMarioWorld` for Super Mario World), else the first that
/// is the console's rather than another game's (a console maker's name in
/// it, or no words at all: `MD_LOGO_Sega31`, `N64_LOGO_01`), else none —
/// the sequence then shows a card with the console's name.
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn choose_logo<'a>(candidates: &'a [(String, String)], game_name: &str) -> Option<&'a str> {
    let game = tokens(game_name);
    if let Some((_, path)) = candidates
        .iter()
        .find(|(title, _)| score_tokens(&tokens(title), &game) >= WEAK)
    {
        return Some(path);
    }
    candidates
        .iter()
        .find(|(title, _)| {
            let t = tokens(title);
            t.iter().all(|w| is_number(w) || w == "logo")
                || t.iter().any(|w| CONSOLE_WORDS.contains(&w.as_str()))
        })
        .map(|(_, path)| path.as_str())
}

/// What a context index's header records in place of a `.meta`
/// fingerprint: the platform and the game's tokens (FNV-1a 64, with a
/// version, so a change to the scoring rebuilds every cached index). Never
/// 0 ([`crate::overrides::NO_META`]).
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn context_fingerprint(platform: &str, game_name: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |b: &[u8]| {
        for &x in b {
            h ^= u64::from(x);
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
        h ^= 0xff;
        h = h.wrapping_mul(0x0100_0000_01b3);
    };
    eat(b"follow-context-v3");
    eat(platform.as_bytes());
    for t in tokens(game_name) {
        eat(t.as_bytes());
    }
    h.max(1)
}

/// What RePlayOS said, reduced to what the state machine needs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Observed {
    /// `view`: `system_list`, `game_play`, …
    pub view: String,
    pub paused: bool,
    pub halted: bool,
    pub system: String,
    pub game_name: String,
    pub game_file: String,
}

impl Observed {
    /// A game is loaded: `game_play`, or paused / an in-game overlay
    /// (`game_options`, `achievements`, `leaderboards`) over a game file.
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    fn loaded(&self) -> bool {
        if self.halted {
            return false;
        }
        let overlay = matches!(
            self.view.as_str(),
            "game_options" | "achievements" | "leaderboards"
        );
        self.view == "game_play" || ((self.paused || overlay) && !self.game_file.is_empty())
    }
}

/// `get_status`'s reply read as it arrives, keeping only what [`Observed`]
/// needs: the top-level `system`, `game_file`, `game_name`, `view`
/// (strings, each cut at [`StatusScan::MAX_FIELD`] bytes) and `paused`,
/// `halted` (booleans). Everything else — other fields, nested objects and
/// arrays, however long — is skipped a byte at a time, so a reply of any
/// size is read in the same few hundred bytes (the firmware's poll used to
/// buffer the body, capped at 2 KB, and parse it with serde).
///
/// Lenient about what it skips; strict only in that the reply must be one
/// complete top-level object ([`StatusScan::finish`]).
#[derive(Debug, Default)]
pub struct StatusScan {
    depth: u32,
    seen_object: bool,
    closed: bool,
    in_string: bool,
    /// 0: none; 1: after a backslash; 2..=5: reading `\u` hex digits.
    escape: u8,
    hex: u32,
    /// At depth 1: the next string is a key (after `{` or `,`).
    expect_key: bool,
    /// The string being read is a top-level key.
    reading_key: bool,
    key: Vec<u8>,
    /// Which wanted field the current top-level value is for.
    field: Option<Field>,
    value: Vec<u8>,
    /// The first byte of a bare top-level value (`t`, `f`, a digit, `n`).
    bare: Option<u8>,
    out: Observed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Field {
    System,
    GameFile,
    GameName,
    View,
    Paused,
    Halted,
}

impl StatusScan {
    /// Longest string field kept, in bytes (the rest is dropped).
    pub const MAX_FIELD: usize = 256;
    const MAX_KEY: usize = 16;

    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn new() -> Self {
        StatusScan::default()
    }

    /// The next piece of the reply, any size.
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn feed(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.byte(b);
        }
    }

    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    fn byte(&mut self, b: u8) {
        if self.closed {
            return;
        }
        if self.in_string {
            self.string_byte(b);
            return;
        }
        match b {
            b'"' => {
                self.in_string = true;
                self.reading_key = self.depth == 1 && self.expect_key;
                self.key.clear();
                self.value.clear();
            }
            b'{' | b'[' => {
                if self.depth == 0 {
                    if b != b'{' {
                        self.closed = true;
                        return;
                    }
                    self.seen_object = true;
                    self.expect_key = true;
                }
                self.depth += 1;
            }
            b'}' | b']' => {
                if self.depth == 1 {
                    self.end_bare();
                }
                self.depth = self.depth.saturating_sub(1);
                if self.depth == 0 {
                    self.closed = true;
                }
            }
            b':' if self.depth == 1 => self.expect_key = false,
            b',' if self.depth == 1 => {
                self.end_bare();
                self.expect_key = true;
                self.field = None;
            }
            b' ' | b'\t' | b'\r' | b'\n' => {}
            _ if self.depth == 1 && !self.expect_key && self.bare.is_none() => {
                self.bare = Some(b);
            }
            _ => {}
        }
    }

    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    fn string_byte(&mut self, b: u8) {
        match self.escape {
            0 => match b {
                b'\\' => self.escape = 1,
                b'"' => {
                    self.in_string = false;
                    self.end_string();
                }
                _ => self.keep(&[b]),
            },
            1 => {
                self.escape = 0;
                let c = match b {
                    b'n' => b'\n',
                    b't' => b'\t',
                    b'r' => b'\r',
                    b'b' => 0x08,
                    b'f' => 0x0c,
                    b'u' => {
                        self.escape = 2;
                        self.hex = 0;
                        return;
                    }
                    other => other,
                };
                self.keep(&[c]);
            }
            _ => {
                let d = (b as char).to_digit(16).unwrap_or(0);
                self.hex = self.hex * 16 + d;
                if self.escape == 5 {
                    self.escape = 0;
                    // A lone surrogate half (a pair's two escapes are not
                    // joined) becomes U+FFFD, like any other bad escape.
                    let c = char::from_u32(self.hex).unwrap_or('\u{fffd}');
                    let mut buf = [0u8; 4];
                    self.keep(c.encode_utf8(&mut buf).as_bytes());
                } else {
                    self.escape += 1;
                }
            }
        }
    }

    /// Keep string bytes if they are a top-level key or a wanted value.
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    fn keep(&mut self, bytes: &[u8]) {
        if self.reading_key {
            if self.key.len() + bytes.len() <= Self::MAX_KEY {
                self.key.extend_from_slice(bytes);
            } else {
                // Longer than any wanted key: poison it.
                self.key.clear();
                self.key.push(0);
            }
        } else if self.depth == 1
            && matches!(
                self.field,
                Some(Field::System | Field::GameFile | Field::GameName | Field::View)
            )
            && self.value.len() + bytes.len() <= Self::MAX_FIELD
        {
            self.value.extend_from_slice(bytes);
        }
    }

    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    fn end_string(&mut self) {
        if self.reading_key {
            self.reading_key = false;
            self.field = match self.key.as_slice() {
                b"system" => Some(Field::System),
                b"game_file" => Some(Field::GameFile),
                b"game_name" => Some(Field::GameName),
                b"view" => Some(Field::View),
                b"paused" => Some(Field::Paused),
                b"halted" => Some(Field::Halted),
                _ => None,
            };
            return;
        }
        if self.depth != 1 {
            return;
        }
        let text = || String::from_utf8_lossy(&self.value).into_owned();
        match self.field {
            Some(Field::System) => self.out.system = text(),
            Some(Field::GameFile) => self.out.game_file = text(),
            Some(Field::GameName) => self.out.game_name = text(),
            Some(Field::View) => self.out.view = text(),
            _ => {}
        }
        self.field = None;
    }

    /// A bare top-level value (`true`, `false`, a number, `null`) ended.
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    fn end_bare(&mut self) {
        if let Some(first) = self.bare.take() {
            match self.field {
                Some(Field::Paused) => self.out.paused = first == b't',
                Some(Field::Halted) => self.out.halted = first == b't',
                _ => {}
            }
        }
    }

    /// The fields, if the reply was one complete JSON object.
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn finish(self) -> Result<Observed, &'static str> {
        match (self.seen_object, self.closed && self.depth == 0) {
            (true, true) => Ok(self.out),
            (true, false) => Err("truncated"),
            _ => Err("not a JSON object"),
        }
    }
}

/// A game being followed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Game {
    pub system: String,
    /// The platform tag, if the pack has one for this system.
    pub platform: Option<&'static str>,
    /// As RePlayOS named it (`Sonic the Hedgehog 3 (USA)`).
    pub name: String,
    pub file: String,
}

impl Game {
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    fn from(o: &Observed) -> Game {
        Game {
            system: o.system.clone(),
            platform: platform_for_system(&o.system),
            name: o.game_name.clone(),
            file: o.game_file.clone(),
        }
    }

    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    fn same(&self, o: &Observed) -> bool {
        self.system == o.system
            && if self.file.is_empty() && o.game_file.is_empty() {
                self.name == o.game_name
            } else {
                self.file == o.game_file
            }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Off,
    Unreachable,
    Attract,
    Game,
    Cooldown,
}

/// The name status reports: `Game` is `starting` while the firmware's
/// sequence (power-on, logo, title card) runs.
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn state_name(state: State, sequence_running: bool) -> &'static str {
    match state {
        State::Off => "off",
        State::Unreachable => "unreachable",
        State::Attract => "attract",
        State::Game if sequence_running => "starting",
        State::Game => "game",
        State::Cooldown => "cooldown",
    }
}

/// What the panel should do now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Back to the normal playlist (the Pi is gone, or following was turned
    /// off).
    Normal,
    /// The attract playlist.
    Attract,
    /// A game started: the sequence, then its context playlist.
    Game(Game),
}

/// The state machine. Feed it every poll's outcome ([`observe`]) and let it
/// [`tick`] between polls (the grace period ends on its own); each returns
/// the action to take on a change, `None` otherwise.
///
/// [`observe`]: Follow::observe
/// [`tick`]: Follow::tick
#[derive(Clone, Debug)]
pub struct Follow {
    state: State,
    game: Option<Game>,
    /// When the state was entered.
    since: Duration,
    /// When the game was left (`Cooldown`).
    left_at: Duration,
    failures: u32,
}

impl Default for Follow {
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    fn default() -> Self {
        Follow {
            state: State::Off,
            game: None,
            since: Duration::ZERO,
            left_at: Duration::ZERO,
            failures: 0,
        }
    }
}

impl Follow {
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn new() -> Self {
        Self::default()
    }

    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn state(&self) -> State {
        self.state
    }

    /// The game being followed (`Game` and `Cooldown`).
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn game(&self) -> Option<&Game> {
        self.game.as_ref()
    }

    /// When the current state was entered.
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn since(&self) -> Duration {
        self.since
    }

    /// Failed polls in a row.
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn failures(&self) -> u32 {
        self.failures
    }

    /// How long until the next poll should go out.
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn poll_interval(&self) -> Duration {
        match self.state {
            State::Unreachable => BACKOFF,
            _ => POLL,
        }
    }

    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    fn enter(&mut self, state: State, now: Duration) {
        if self.state != state {
            self.state = state;
            self.since = now;
        }
    }

    /// Following turned on (`true`) or off. Turning it on changes nothing on
    /// the panel until the Pi answers; turning it off returns to normal.
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn set_enabled(&mut self, on: bool, now: Duration) -> Option<Action> {
        match (on, self.state) {
            (true, State::Off) => {
                self.failures = 0;
                self.enter(State::Unreachable, now);
                None
            }
            (false, State::Off) | (true, _) => None,
            (false, prev) => {
                self.game = None;
                self.failures = 0;
                self.enter(State::Off, now);
                (prev != State::Unreachable).then_some(Action::Normal)
            }
        }
    }

    /// A poll came back (`Some`) or failed (`None`: unreachable, timed out,
    /// refused, 401 — the firmware says which).
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn observe(&mut self, now: Duration, poll: Option<&Observed>) -> Option<Action> {
        if self.state == State::Off {
            return None;
        }
        let Some(o) = poll else {
            self.failures = self.failures.saturating_add(1);
            let give_up = self.state == State::Unreachable || self.failures >= FAILURES_TO_DROP;
            if give_up && self.state != State::Unreachable {
                self.game = None;
                self.enter(State::Unreachable, now);
                return Some(Action::Normal);
            }
            return None;
        };
        self.failures = 0;
        if o.paused && matches!(self.state, State::Game | State::Cooldown) {
            return None;
        }
        if o.loaded() {
            match (&self.game, self.state) {
                (Some(g), State::Game | State::Cooldown) if g.same(o) => {
                    self.enter(State::Game, now);
                    None
                }
                _ => {
                    let game = Game::from(o);
                    self.game = Some(game.clone());
                    // A new game restarts the clock even from `Game`.
                    self.state = State::Game;
                    self.since = now;
                    Some(Action::Game(game))
                }
            }
        } else {
            match self.state {
                State::Game => {
                    self.left_at = now;
                    self.enter(State::Cooldown, now);
                    None
                }
                State::Cooldown => self.tick(now),
                State::Attract => None,
                State::Unreachable | State::Off => {
                    self.game = None;
                    self.enter(State::Attract, now);
                    Some(Action::Attract)
                }
            }
        }
    }

    /// Time passed: the grace period may be over.
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn tick(&mut self, now: Duration) -> Option<Action> {
        if self.state == State::Cooldown && now.saturating_sub(self.left_at) >= GRACE {
            self.game = None;
            self.enter(State::Attract, now);
            return Some(Action::Attract);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scan(chunks: &[&[u8]]) -> Result<Observed, &'static str> {
        let mut s = StatusScan::new();
        for c in chunks {
            s.feed(c);
        }
        s.finish()
    }

    #[test]
    fn status_scan_reads_the_real_reply() {
        let reply = br#"{"system":"sega_smd","game_file":"/roms/sega_smd/Sonic 3.md","game_name":"Sonic the Hedgehog 3 (USA)","paused":false,"halted":false,"view":"game_play","view_id":2,"core_file":"genesis_plus_gx_libretro.so","core_info":"Genesis Plus GX v1.7.4"}"#;
        let o = scan(&[reply]).unwrap();
        assert_eq!(o.system, "sega_smd");
        assert_eq!(o.game_file, "/roms/sega_smd/Sonic 3.md");
        assert_eq!(o.game_name, "Sonic the Hedgehog 3 (USA)");
        assert_eq!(o.view, "game_play");
        assert!(!o.paused && !o.halted);
        // Byte by byte, the same.
        let bytes: Vec<&[u8]> = reply.chunks(1).collect();
        assert_eq!(scan(&bytes).unwrap(), o);
    }

    #[test]
    fn status_scan_skips_what_it_does_not_want_however_big() {
        let pad = "x".repeat(100_000);
        let reply = format!(
            r#"{{ "core_info" : "{pad}", "nested": {{"system":"no","list":[1,{{"view":"no"}},"a\"b"]}},
               "system" : "nintendo_snes" , "paused" : true, "halted":null,
               "game_name":"Café \"Mario\" \\ World\n", "view":"game_options", "extra":[{pad:?}] }}"#
        );
        let chunks: Vec<&[u8]> = reply.as_bytes().chunks(97).collect();
        let o = scan(&chunks).unwrap();
        assert_eq!(o.system, "nintendo_snes");
        assert!(o.paused && !o.halted);
        assert_eq!(o.game_name, "Café \"Mario\" \\ World\n");
        assert_eq!(o.view, "game_options");
        assert_eq!(o.game_file, "");
    }

    #[test]
    fn status_scan_cuts_long_fields_and_refuses_broken_replies() {
        let long = "a".repeat(1000);
        let o = scan(&[format!(r#"{{"game_name":"{long}"}}"#).as_bytes()]).unwrap();
        assert_eq!(o.game_name.len(), StatusScan::MAX_FIELD);
        assert_eq!(scan(&[br#"{"system":"sega_smd","view":"game"#]), Err("truncated"));
        assert_eq!(scan(&[b"[1,2]"]), Err("not a JSON object"));
        assert_eq!(scan(&[b""]), Err("not a JSON object"));
        assert_eq!(scan(&[br#"{"error":"unauthorized"}"#]).unwrap(), Observed::default());
        // Trailing bytes after the object are ignored.
        assert_eq!(scan(&[br#"{"view":"system_list"} junk"#]).unwrap().view, "system_list");
    }

    fn s(secs: u64) -> Duration {
        Duration::from_secs(secs)
    }

    fn menu() -> Observed {
        Observed {
            view: "system_list".into(),
            ..Observed::default()
        }
    }

    fn playing(system: &str, name: &str) -> Observed {
        Observed {
            view: "game_play".into(),
            system: system.into(),
            game_name: name.into(),
            game_file: format!("/media/nvme/roms/{system}/{name}.bin"),
            ..Observed::default()
        }
    }

    #[test]
    fn system_map() {
        assert_eq!(platform_for_system("sega_smd"), Some("MD"));
        assert_eq!(platform_for_system("nintendo_snes"), Some("SNES"));
        assert_eq!(platform_for_system("sega_st"), Some("SATURN"));
        assert_eq!(platform_for_system("nintendo_ds"), Some("NDS"));
        assert_eq!(platform_for_system("sony_psx"), Some("PS1"));
        assert_eq!(platform_for_system("sega_gg"), Some("GAMEGEAR"));
        assert_eq!(platform_for_system("snk_ngp"), Some("NGP"));
        assert_eq!(platform_for_system("arcade_fbneo"), Some("ARCADE"));
        assert_eq!(platform_for_system("arcade_mame_2k3p"), Some("ARCADE"));
        assert_eq!(platform_for_system("SEGA_SMD"), Some("MD"));
        assert_eq!(platform_for_system("sega_32x"), None);
        assert_eq!(platform_for_system(""), None);
        assert_eq!(platform_for_system("arcade"), None);
        // Every platform in the table is one the tag grammar knows, and has
        // a name.
        for (_, p) in SYSTEMS {
            assert!(crate::manifest::PLATFORMS.contains(p), "{p}");
            assert!(!console_name(p).is_empty(), "{p}");
        }
        assert_eq!(console_name_for_system("sega_smd"), "Mega Drive");
        assert_eq!(console_name_for_system("sega_32x"), "sega 32x");
    }

    #[test]
    fn game_names_clean_up() {
        assert_eq!(
            clean_game_name("Sonic the Hedgehog 3 (USA)"),
            "Sonic the Hedgehog 3"
        );
        assert_eq!(
            clean_game_name("Legend of Zelda, The - A Link to the Past (USA) [!]"),
            "The Legend of Zelda - A Link to the Past"
        );
        assert_eq!(clean_game_name("Castlevania, The"), "The Castlevania");
        assert_eq!(
            clean_game_name("  Super  Mario World  "),
            "Super Mario World"
        );
        assert_eq!(clean_game_name("Mario, Luigi"), "Mario, Luigi");
    }

    #[test]
    fn tokens_split_and_normalise() {
        assert_eq!(
            tokens("Sonic the Hedgehog 3 (USA)"),
            ["sonic", "hedgehog", "3"]
        );
        assert_eq!(tokens("Sonic3"), ["sonic", "3"]);
        assert_eq!(tokens("StreetsOfRage2"), ["streets", "rage", "2"]);
        assert_eq!(tokens("Street Fighter II'"), ["street", "fighter", "2"]);
        assert_eq!(tokens("StreetFighterII"), ["street", "fighter", "2"]);
        assert_eq!(tokens("SNESMario"), ["snes", "mario"]);
        assert_eq!(tokens("Pokémon Two"), ["pokemon", "2"]);
        assert_eq!(tokens("Mega Man X"), ["mega", "man", "x"]);
        assert_eq!(tokens("Level 007"), ["level", "7"]);
        assert_eq!(tokens("The"), Vec::<String>::new());
        assert_eq!(tokens("Sonic & Knuckles & Sonic"), ["sonic", "knuckles"]);
    }

    #[test]
    fn title_scores() {
        let game = "Sonic the Hedgehog 3 (USA)";
        let score = |t: &str| title_score(t, game);
        assert!((score("Sonic3") - 0.8).abs() < 1e-6);
        assert!((score("Sonic The Hedgehog 3") - 1.0).abs() < 1e-6);
        assert!((score("Sonic The Hedgehog") - 0.56).abs() < 1e-6);
        assert!((score("Sonic The Hedgehog 2") - 1.0 / 3.0).abs() < 1e-6);
        // Another Sonic GIF with no number: 0.4 x 0.7 x 0.75.
        assert!((score("SonicVitre") - 0.21).abs() < 1e-6);
        assert!(score("Streets of Rage") < WEAK);
        assert_eq!(score(""), 0.0);
        assert_eq!(title_score("Anything", ""), 0.0);
        // Run together: Megaman is Mega Man.
        assert!(title_score("Megaman", "Mega Man (USA)") >= STRONG);
        assert!(
            title_score(
                "StreetFighter2",
                "Street Fighter II' - Special Champion Edition"
            ) >= STRONG
        );
        // A game with no number against an entry with one.
        assert!((title_score("Sonic3", "Sonic the Hedgehog") - 0.4).abs() < 1e-6);
        // Another game of the series.
        assert!((title_score("Super Mario Kart", "Super Mario World (USA)") - 0.5).abs() < 1e-6);
        assert!(title_score("SuperMarioWorld", "Super Mario World (USA)") >= STRONG);
        assert!(title_score("StreetsOfRage2Skate Joge", "Streets of Rage 2 (USA)") >= STRONG);
        // Classes.
        assert_eq!(classify(score("Sonic3")), Some(Match::Strong));
        assert_eq!(classify(score("Sonic The Hedgehog")), Some(Match::Related));
        assert_eq!(classify(score("SonicVitre")), None);
        assert_eq!(classify(score("Sonic The Hedgehog 2")), None);
        assert_eq!(classify(0.0), None);
    }

    /// Draw `n` from a context index the way the engine does (the weights
    /// read back from the index into the scheduler) and count each class.
    fn draw_shares(
        cat: &mut crate::index::IndexedCatalog<std::io::Cursor<Vec<u8>>, std::io::Cursor<Vec<u8>>>,
        game: &str,
        window: usize,
        n: usize,
    ) -> (f32, f32, f32) {
        use crate::engine::Catalog;
        use crate::scheduler::{Mode, Scheduler};
        let mut sched = Scheduler::new(cat.len(), Mode::Shuffle, 0x5eed).with_no_repeat(window);
        for (i, ov) in cat.overrides().unwrap().iter() {
            sched.set_weight(*i as usize, ov.scheduler_weight().unwrap());
        }
        let class: Vec<Option<Match>> = (0..cat.len())
            .map(|i| classify(title_score(&cat.entry(i).unwrap().title, game)))
            .collect();
        let mut c = [0usize; 3];
        for _ in 0..n {
            match class[sched.next_index().unwrap()] {
                Some(Match::Strong) => c[0] += 1,
                Some(Match::Related) => c[1] += 1,
                None => c[2] += 1,
            }
        }
        let f = |k: usize| c[k] as f32 / n as f32;
        (f(0), f(1), f(2))
    }

    fn near(got: f32, want: f32, what: &str) {
        assert!(
            (got - want).abs() <= 0.03,
            "{what}: {got:.3}, want {want:.3} ± 0.03"
        );
    }

    /// A synthetic manifest per game with the stock card's counts (Sonic 3:
    /// 1 strong, 27 related of 635 Mega Drive entries; Super Mario World: 26
    /// strong, 91 related of 1,052 SNES), through the same one-pass index
    /// build as the firmware (`write_index_resolving_then` +
    /// `weigh_context`), read back, and drawn from with the context window:
    /// strong + related get 60% of the draws, by the model and by 60,000
    /// draws of the scheduler. (The model is an average: with only two or
    /// three matches at weight 255 and the window at its floor it overstates
    /// them — 2 strong + 1 related of 63 is 60% by the model, 53% drawn.)
    #[test]
    fn context_weights_hit_their_shares() {
        use crate::engine::Catalog;
        use crate::filter::Filter;
        use crate::index::{write_index_resolving_then, IndexedCatalog};
        use std::io::Cursor;
        let others = ["StreetsOfRage", "Plok", "Gunstar Heroes", "Ecco", "Contra"];
        let target = MATCH_PCT as f32 / 100.0;
        for (platform, game, strong, related, n) in [
            (
                "MD",
                "Sonic the Hedgehog 3 (USA)",
                "Sonic3",
                "SonicTheHedgehog",
                (1, 27, 607),
            ),
            (
                "SNES",
                "Super Mario World (USA)",
                "SuperMarioWorld",
                "SuperMarioKart",
                (26, 91, 935),
            ),
        ] {
            let mut m = String::new();
            let mut line = |title: &str, i: usize| {
                m.push_str(&format!("/gifs/Consoles/{platform}_{title}_{i:02}.gif\n"));
                // Another platform's entry between each: filtered out.
                m.push_str(&format!("/gifs/Consoles/NES_{title}_{i:02}.gif\n"));
            };
            for i in 0..n.0 {
                line(strong, i);
            }
            for i in 0..n.1 {
                line(related, i);
            }
            for i in 0..n.2 {
                line(others[i % others.len()], i);
            }
            let filter = Filter::new().platforms([platform]);
            let game_tokens = tokens(game);
            let mut weights = None;
            let mut out = Cursor::new(Vec::new());
            let (header, _) = write_index_resolving_then(
                Cursor::new(m.as_bytes()),
                m.len() as u32,
                &filter,
                context_fingerprint(platform, game),
                |e| classify(score_tokens(&tokens(&e.title), &game_tokens)).map(Match::mark),
                |count, ov| weights = Some(weigh_context(count, ov)),
                &mut out,
            )
            .unwrap();
            let w = weights.unwrap();
            assert_eq!(header.count as usize, n.0 + n.1 + n.2);
            assert_eq!(
                (w.n_strong, w.n_related),
                (n.0 as u32, n.1 as u32),
                "{game}"
            );
            assert_eq!(w.total, header.count);
            assert_eq!(w.window as usize, context_window(n.0 + n.1));
            assert!(!w.capped, "{game}: {w:?}");
            let (s, r, _) = w.shares();
            near(
                s + r,
                target,
                &format!("{game} {n:?}: matches by the model ({w:?})"),
            );

            let mut cat =
                IndexedCatalog::open(Cursor::new(m.into_bytes()), Cursor::new(out.into_inner()))
                    .unwrap();
            assert_eq!(cat.len(), header.count as usize);
            let (ds, dr, _) = draw_shares(&mut cat, game, w.window as usize, 60_000);
            println!(
                "{game} {n:?}: window {}, weights {} / {}, model {s:.3} / {r:.3}, drawn {ds:.3} / {dr:.3}",
                w.window, w.strong, w.related
            );
            near(ds + dr, target, &format!("{game} {n:?}: drawn matches"));
            near(ds, s, &format!("{game} {n:?}: drawn strong vs the model"));
            near(dr, r, &format!("{game} {n:?}: drawn related vs the model"));
        }
    }

    #[test]
    fn context_weights_and_windows() {
        assert_eq!(context_window(0), 4);
        assert_eq!(context_window(3), 4);
        assert_eq!(context_window(10), 9);
        assert_eq!(context_window(28), 27);
        assert_eq!(context_window(117), 32);
        // Sonic 3: one strong GIF can't reach 60% (at most 1 draw in 28),
        // so it goes to 255 and the related carry the rest.
        let w = ContextWeights::for_counts(635, 1, 27);
        assert_eq!(
            (w.strong, w.window, w.capped),
            (MAX_WEIGHT, 27, false),
            "{w:?}"
        );
        assert!(w.related > 1);
        // Strong alone gets there: related stay at 1.
        let w = ContextWeights::for_counts(600, 30, 40);
        assert!(w.strong < MAX_WEIGHT && w.related == 1, "{w:?}");
        let (s, r, _) = w.shares();
        near(s + r, 0.60, "strong enough");
        // Matches already the majority: everything 1.
        let w = ContextWeights::for_counts(100, 70, 10);
        assert_eq!((w.strong, w.related, w.capped), (1, 1, false));
        // Too few to get there even at 255: capped.
        let w = ContextWeights::for_counts(100_000, 1, 0);
        assert!(w.capped && w.strong == MAX_WEIGHT, "{w:?}");
        assert_eq!(
            ContextWeights::for_counts(50, 0, 0).shares_ppm(),
            [0, 0, 1_000_000]
        );
        assert_eq!(ContextWeights::for_counts(0, 0, 0).shares_ppm(), [0; 3]);
        // The model without a window reduces to plain proportions.
        let w = ContextWeights {
            strong: 10,
            related: 2,
            capped: false,
            window: 0,
            n_strong: 5,
            n_related: 10,
            total: 115,
        };
        let [s, r, o] = w.shares_ppm();
        assert!(s.abs_diff(294_117) < 50 && r.abs_diff(117_647) < 50 && o.abs_diff(588_235) < 50);
    }

    #[test]
    fn logos_and_fingerprints() {
        assert!(is_logo(&Entry::from_path(
            "/gifs/Consoles/MD_LOGO_Sega31.gif"
        )));
        assert!(is_logo(&Entry::from_path(
            "/gifs/Other/OTHER_SEGA_SonicLogo.gif"
        )));
        assert!(!is_logo(&Entry::from_path(
            "/gifs/Consoles/MD_Sonic3_spritefox11.gif"
        )));
        let logos = |v: &[&str]| -> Vec<(String, String)> {
            v.iter()
                .map(|p| {
                    let e = Entry::from_path(p);
                    (e.title, e.path)
                })
                .collect()
        };
        let snes = logos(&["/gifs/Consoles/SNES_LOGO_SuperMarioWorld_spritefox11.gif"]);
        assert_eq!(
            choose_logo(&snes, "Super Mario World (USA)"),
            Some("/gifs/Consoles/SNES_LOGO_SuperMarioWorld_spritefox11.gif")
        );
        assert_eq!(choose_logo(&snes, "Donkey Kong Country (USA)"), None);
        let md = logos(&[
            "/gifs/Consoles/MD_LOGO_Road_Rash_Joge.gif",
            "/gifs/Consoles/MD_LOGO_Sega32.gif",
            "/gifs/Consoles/MD_LOGO_Sega29_Sonic_3D_Blast.gif",
        ]);
        assert_eq!(
            choose_logo(&md, "Sonic the Hedgehog 3 (USA)"),
            Some("/gifs/Consoles/MD_LOGO_Sega32.gif")
        );
        assert_eq!(
            choose_logo(&md, "Road Rash (USA)"),
            Some("/gifs/Consoles/MD_LOGO_Road_Rash_Joge.gif")
        );
        let n64 = logos(&["/gifs/Consoles/N64_LOGO_01.gif"]);
        assert_eq!(
            choose_logo(&n64, "Mario Kart 64"),
            Some("/gifs/Consoles/N64_LOGO_01.gif")
        );
        assert_eq!(choose_logo(&[], "Anything"), None);
        let a = context_fingerprint("MD", "Sonic the Hedgehog 3 (USA)");
        assert_eq!(
            a,
            context_fingerprint("MD", "Sonic the Hedgehog 3 (Europe)")
        );
        assert_ne!(a, context_fingerprint("SNES", "Sonic the Hedgehog 3 (USA)"));
        assert_ne!(a, context_fingerprint("MD", "Sonic the Hedgehog 2"));
        assert_ne!(a, crate::overrides::NO_META);
    }

    #[test]
    fn off_until_enabled_then_unreachable_until_it_answers() {
        let mut f = Follow::new();
        assert_eq!(f.observe(s(0), Some(&menu())), None);
        assert_eq!(f.state(), State::Off);
        assert_eq!(f.set_enabled(true, s(1)), None);
        assert_eq!(f.state(), State::Unreachable);
        assert_eq!(f.poll_interval(), BACKOFF);
        assert_eq!(f.observe(s(2), None), None);
        assert_eq!(f.state(), State::Unreachable);
        assert_eq!(f.observe(s(3), Some(&menu())), Some(Action::Attract));
        assert_eq!(f.state(), State::Attract);
        assert_eq!(f.since(), s(3));
        assert_eq!(f.poll_interval(), POLL);
        assert_eq!(f.observe(s(5), Some(&menu())), None);
        // Disabled: back to normal.
        assert_eq!(f.set_enabled(false, s(6)), Some(Action::Normal));
        assert_eq!(f.state(), State::Off);
        assert_eq!(f.set_enabled(false, s(7)), None);
        // Disabled while unreachable: nothing to undo.
        f.set_enabled(true, s(8));
        assert_eq!(f.set_enabled(false, s(9)), None);
    }

    #[test]
    fn a_game_then_the_grace_period_then_attract() {
        let mut f = Follow::new();
        f.set_enabled(true, s(0));
        f.observe(s(1), Some(&menu()));
        let sonic = playing("sega_smd", "Sonic the Hedgehog 3 (USA)");
        let Some(Action::Game(g)) = f.observe(s(3), Some(&sonic)) else {
            panic!("expected a game");
        };
        assert_eq!(g.platform, Some("MD"));
        assert_eq!(g.name, "Sonic the Hedgehog 3 (USA)");
        assert_eq!(f.state(), State::Game);
        assert_eq!(state_name(f.state(), true), "starting");
        assert_eq!(state_name(f.state(), false), "game");
        // Still playing: nothing.
        assert_eq!(f.observe(s(5), Some(&sonic)), None);
        // Paused (the pause menu is game_options): nothing, even for long.
        let mut paused = sonic.clone();
        paused.paused = true;
        paused.view = "game_options".into();
        for t in 7..60 {
            assert_eq!(f.observe(s(t), Some(&paused)), None);
            assert_eq!(f.tick(s(t)), None);
        }
        assert_eq!(f.state(), State::Game);
        // The in-game options unpaused are still the game.
        paused.paused = false;
        assert_eq!(f.observe(s(61), Some(&paused)), None);
        assert_eq!(f.state(), State::Game);
        // Back to the menu: cooldown, then attract after the grace period.
        assert_eq!(f.observe(s(70), Some(&menu())), None);
        assert_eq!(f.state(), State::Cooldown);
        assert_eq!(state_name(f.state(), false), "cooldown");
        assert!(f.game().is_some());
        assert_eq!(f.observe(s(80), Some(&menu())), None);
        assert_eq!(f.tick(s(89)), None);
        assert_eq!(f.tick(s(90)), Some(Action::Attract));
        assert_eq!(f.state(), State::Attract);
        assert_eq!(f.game(), None);
    }

    #[test]
    fn the_same_game_within_the_grace_period_carries_on() {
        let mut f = Follow::new();
        f.set_enabled(true, s(0));
        f.observe(s(1), Some(&menu()));
        let sonic = playing("sega_smd", "Sonic the Hedgehog 3 (USA)");
        assert!(matches!(
            f.observe(s(2), Some(&sonic)),
            Some(Action::Game(_))
        ));
        f.observe(s(10), Some(&menu()));
        assert_eq!(f.observe(s(15), Some(&sonic)), None);
        assert_eq!(f.state(), State::Game);
        // Another game within the grace period starts over.
        f.observe(s(20), Some(&menu()));
        let smw = playing("nintendo_snes", "Super Mario World (USA)");
        let Some(Action::Game(g)) = f.observe(s(25), Some(&smw)) else {
            panic!("expected the new game");
        };
        assert_eq!(g.platform, Some("SNES"));
        // A game switch without a menu in between too.
        assert!(matches!(
            f.observe(s(30), Some(&sonic)),
            Some(Action::Game(_))
        ));
        assert_eq!(f.since(), s(30));
        // Halted is out of the game.
        let mut halted = sonic.clone();
        halted.halted = true;
        f.observe(s(40), Some(&halted));
        assert_eq!(f.state(), State::Cooldown);
    }

    #[test]
    fn lost_polls_drop_a_game_only_after_three() {
        let mut f = Follow::new();
        f.set_enabled(true, s(0));
        let sonic = playing("sega_smd", "Sonic the Hedgehog 3 (USA)");
        // Straight into a game from unreachable.
        assert!(matches!(
            f.observe(s(1), Some(&sonic)),
            Some(Action::Game(_))
        ));
        assert_eq!(f.observe(s(3), None), None);
        assert_eq!(f.observe(s(5), None), None);
        assert_eq!(f.failures(), 2);
        // An answer in between resets the count.
        assert_eq!(f.observe(s(7), Some(&sonic)), None);
        assert_eq!(f.failures(), 0);
        f.observe(s(9), None);
        f.observe(s(11), None);
        assert_eq!(f.observe(s(13), None), Some(Action::Normal));
        assert_eq!(f.state(), State::Unreachable);
        assert_eq!(f.game(), None);
        // Back: a new game, from the top.
        assert!(matches!(
            f.observe(s(30), Some(&sonic)),
            Some(Action::Game(_))
        ));
        // Paused while unreachable is still a first contact.
        let mut g = Follow::new();
        g.set_enabled(true, s(0));
        let mut p = sonic.clone();
        p.paused = true;
        assert!(matches!(g.observe(s(1), Some(&p)), Some(Action::Game(_))));
    }
}

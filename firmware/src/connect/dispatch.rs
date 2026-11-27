//! Signed commands, once checked (`proto::Session::check`: a known key, a
//! good signature, this panel and this session, a fresh counter, the
//! rates, the allow-list and its arguments), carried out through the same
//! paths the LAN's handlers use: `Command::PlayNow`, `Skip`, `Back`,
//! `TogglePause`, `ApplyConfig` (brightness, playlist) with the LAN's own
//! play and config validation run as well (`http::check_play_path`,
//! `http::apply_patch`) and the same 0.8 s "busy" rule. Each answers one
//! `ack`.
//!
//! Nothing here reaches anything the allow-list does not name: there is no
//! command for an update, a delete, WiFi, a reboot or a setting other than
//! brightness and the playlist.

use super::client::Client;
use super::{change, Store};
use crate::chan as mpsc;
use crate::http;
use crate::nvs_config::Settings;
use crate::state::{Applied, Command, PlayError, Status};
use opendmd_core::connect::codec::hex;
use opendmd_core::connect::keys::{key_id, Entry};
use opendmd_core::connect::proto::{self, Accepted, Command as Remote, Refusal};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// What the client reaches the player through.
pub struct Link {
    pub cmd: mpsc::Sender<Command>,
    pub status: Arc<Mutex<Status>>,
    pub settings: Arc<Mutex<Settings>>,
}

/// A configuration change the player took and has not answered: its
/// answer, the settings to share once it is applied, and how long it is
/// waited for.
pub struct Pending {
    pub rx: mpsc::Receiver<Result<Applied, String>>,
    /// The one field asked (a playlist: brightness is answered at once).
    pub brightness: Option<u8>,
    pub set: Option<String>,
    pub until: Instant,
}

/// An `ack`'s result for a command taken and still under way.
const PENDING: &str = r#"{"pending":true}"#;
/// How long a configuration's answer (a playlist rebuilt) is waited for
/// in the background.
const CONFIG_WAIT: Duration = Duration::from_secs(60);
/// The most of `/sets` a `playlists` answer lists.
const SETS_MAX: usize = 200;

type Done = Result<Option<String>, Refusal>;

impl Client {
    /// Carry out an accepted command; its `ack`.
    #[optimize(size)]
    #[inline(never)]
    pub(super) fn run(&mut self, acc: Accepted) -> String {
        let Accepted { k, n, cmd } = acc;
        let r = match cmd {
            Remote::Play { path } => self.play(path),
            Remote::Next => self.link.cmd.send(Command::Skip).map(|_| None).map_err(|_| Refusal::Busy),
            Remote::Prev => self.back(),
            Remote::Pause => self.pause(),
            Remote::Brightness { v } => self.config(k, Some(v), None),
            Remote::Playlist { set } => self.config(k, None, Some(set)),
            Remote::Playlists => self.playlists(),
            Remote::Formulas { after } => self.formulas(after),
            Remote::Controllers => Ok(Some(self.keys.controllers_result(|k| {
                self.last_use.iter().find(|(x, _)| x == k).map(|(_, at)| at.elapsed().as_secs())
            }))),
            Remote::AddController { public, label } => self.add(k, public, label),
            Remote::RemoveController { k: gone } => self.remove(gone),
            Remote::Name { name } => self.rename(name),
            // Queued for the top level by `frame` (`xfer`), never here.
            Remote::Send(_) | Remote::SendCancel { .. } => Err(Refusal::Busy),
        };
        let ack = match r {
            Ok(r) => proto::ack_ok(&k, n, r.as_deref()),
            Err(e) => proto::ack_err(&k, Some(n), e),
        };
        log::info!("connect: command n={n} from {}: {}", hex(&k[..4]), if ack.contains("\"ok\":true") { "ok" } else { "refused" });
        ack
    }

    #[optimize(size)]
    #[inline(never)]
    pub(super) fn play(&mut self, path: String) -> Done {
        // The remote rule (`proto::remote_play_path_ok`) has run; the LAN's
        // own validation runs too.
        if http::check_play_path(&path, false, false).is_err() {
            return Err(Refusal::BadArg);
        }
        if path.starts_with("/gifs/") && !self.link.status.lock().unwrap().sd_usable() {
            return Err(Refusal::NoCard);
        }
        let (tx, rx) = mpsc::channel();
        self.link
            .cmd
            .send(Command::PlayNow { path, live: false, over: false, psb: None, reply: tx })
            .map_err(|_| Refusal::Busy)?;
        crate::wake::kick();
        played(rx.answer_within(http::TAKE_WAIT, http::PLAY_ANSWER))
    }

    #[optimize(size)]
    #[inline(never)]
    fn back(&mut self) -> Done {
        let (tx, rx) = mpsc::channel();
        self.link.cmd.send(Command::Back(tx)).map_err(|_| Refusal::Busy)?;
        crate::wake::kick();
        played(rx.answer_within(http::TAKE_WAIT, http::PLAY_ANSWER))
    }

    #[optimize(size)]
    #[inline(never)]
    fn pause(&mut self) -> Done {
        let (tx, rx) = mpsc::channel();
        self.link.cmd.send(Command::TogglePause(tx)).map_err(|_| Refusal::Busy)?;
        crate::wake::kick();
        match rx.answer_within(http::TAKE_WAIT, http::PLAY_ANSWER) {
            mpsc::Handoff::Answered(paused) => Ok(Some(format!(r#"{{"paused":{paused}}}"#))),
            mpsc::Handoff::Taken => Ok(Some(PENDING.into())),
            mpsc::Handoff::Withdrawn => Err(Refusal::Busy),
        }
    }

    /// Brightness or the playlist, as `PUT /api/config` makes them: the
    /// patch checked by the same function, the same `ApplyConfig`, the
    /// settings shared once the player has applied them. One at a time.
    #[optimize(size)]
    #[inline(never)]
    fn config(&mut self, k: [u8; 8], brightness: Option<u8>, set: Option<String>) -> Done {
        if self.pending.is_some() {
            return Err(Refusal::Busy);
        }
        // One a second from each controller (a drag of the slider sends 4:
        // the last one wins a moment later; the player and the card are
        // spared the rest), as the core keeps its 4 a second per key.
        if !self.config_rate_take(k) {
            return Err(Refusal::Rate);
        }
        // The LAN's own rule for a playlist name, beside the remote one.
        if set.as_deref().is_some_and(|s| !crate::nvs_config::valid_set_name(s)) {
            return Err(Refusal::BadArg);
        }
        let (tx, rx) = mpsc::channel();
        let cmd = match (brightness, &set) {
            (Some(v), _) => Command::RemoteConfig { brightness: v, reply: tx },
            // Only the playlist: the main task takes the rest from its own
            // settings when it gets to it (`only_set`).
            (None, s) => Command::ApplyConfig {
                settings: Box::new(Settings { set: s.clone().unwrap_or_default(), ..Settings::default() }),
                panel: None,
                look_named: false,
                boot_named: false,
                only_set: true,
                reply: tx,
            },
        };
        self.link.cmd.send(cmd).map_err(|_| Refusal::Busy)?;
        crate::wake::kick();
        let p = Pending { rx, brightness, set, until: Instant::now() + CONFIG_WAIT };
        match p.rx.answer_within(http::TAKE_WAIT, http::PLAY_ANSWER) {
            mpsc::Handoff::Answered(answer) => {
                let refused = answer.as_ref().err().map(|e| refusal_of(e));
                if let Some(after) = settle(&self.link, p, answer) {
                    self.save_card_after(after);
                }
                refused.map_or(Ok(None), Err)
            }
            mpsc::Handoff::Taken => {
                self.pending = Some(p);
                Ok(Some(PENDING.into()))
            }
            mpsc::Handoff::Withdrawn => Err(Refusal::Busy),
        }
    }

    /// `formulas`: its own rate (`proto::FormulasRate`: a listing's first
    /// page one in 5 s per panel, its next pages free), then a page.
    #[optimize(size)]
    #[inline(never)]
    fn formulas(&mut self, after: Option<String>) -> Done {
        let now = super::client::core_now();
        if !self.formulas_rate.take(after.as_deref(), now) {
            return Err(Refusal::Rate);
        }
        // (The status lock let go before the card is read.)
        let card = self.link.status.lock().unwrap().sd_usable();
        let (r, next) = formulas_page(card, after.as_deref());
        self.formulas_rate.answered(next.as_deref(), super::client::core_now());
        Ok(Some(r))
    }

    /// The playlists in `/sets` (`""`, the main list, is always there),
    /// as many as fit the panel's text limit.
    #[optimize(size)]
    #[inline(never)]
    fn playlists(&mut self) -> Done {
        if !self.link.status.lock().unwrap().sd_usable() {
            return Err(Refusal::NoCard);
        }
        let Ok(dir) = std::fs::read_dir(crate::config::SETS_DIR) else {
            return Ok(Some(r#"{"sets":[],"more":false}"#.into()));
        };
        // In the directory's order: a controller sorts them. One past the
        // most says there are more.
        let mut names: Vec<String> = dir
            .filter_map(|e| e.ok())
            .filter_map(|e| e.file_name().to_str().and_then(|n| n.strip_suffix(".txt")).map(str::to_string))
            .filter(|n| proto::remote_set_ok(n) && !n.is_empty())
            .take(SETS_MAX + 1)
            .collect();
        let cut = names.len() > SETS_MAX;
        names.truncate(SETS_MAX);
        Ok(Some(sets_result(&names, cut)))
    }

    #[optimize(size)]
    #[inline(never)]
    fn add(&mut self, by: [u8; 8], public: [u8; 32], label: String) -> Done {
        let added = crate::clock::unix_valid().and_then(|t| u32::try_from(t).ok()).unwrap_or(0);
        let k = key_id(&public);
        change(
            &self.link.cmd,
            move |s| {
                let mut keys = s.keys.clone();
                keys.add(Entry { public, flags: 0, label, added, added_by: Some(by) }).map_err(|e| format!("{e:?}"))?;
                Ok((Store::Keys(keys.encode()), keys))
            },
            |s, keys| {
                s.keys = keys;
                s.keys_changed = true;
            },
        )
        .map_err(|e| stored_refusal(&e))?;
        Ok(Some(format!(r#"{{"k":"{}"}}"#, hex(&k))))
    }

    #[optimize(size)]
    #[inline(never)]
    fn remove(&mut self, k: [u8; 8]) -> Done {
        change(
            &self.link.cmd,
            move |s| {
                let mut keys = s.keys.clone();
                keys.remove(&k).map_err(|e| format!("{e:?}"))?;
                Ok((Store::Keys(keys.encode()), keys))
            },
            |s, keys| {
                s.keys = keys;
                s.keys_changed = true;
            },
        )
        .map_err(|e| stored_refusal(&e))?;
        Ok(None)
    }

    #[optimize(size)]
    #[inline(never)]
    fn rename(&mut self, name: String) -> Done {
        change(&self.link.cmd, move |_| Ok((Store::Name(name.clone()), name)), |s, n| {
            s.name = n;
            s.name_changed = true;
        })
        .map_err(|e| stored_refusal(&e))?;
        Ok(None)
    }
}

/// A store's failure as an `ack`'s error: the list changed under it (a
/// LAN removal at the same moment) is a `bad-arg`, anything else `busy`.
#[optimize(size)]
#[inline(never)]
fn stored_refusal(e: &str) -> Refusal {
    match e {
        "Unknown" | "Duplicate" | "BadKey" | "BadLabel" => Refusal::BadArg,
        "Full" => Refusal::Quota,
        _ => Refusal::Busy,
    }
}

/// A play's or Previous's answer from the main task, as an `ack` result.
#[optimize(size)]
#[inline(never)]
fn played(h: mpsc::Handoff<Result<(), PlayError>>) -> Done {
    match h {
        mpsc::Handoff::Answered(Ok(())) => Ok(None),
        mpsc::Handoff::Answered(Err(PlayError::NoPlaylist)) => Err(Refusal::Busy),
        mpsc::Handoff::Answered(Err(PlayError::BadFormula(e))) if http::short_of_memory(&e) => Err(Refusal::Busy),
        mpsc::Handoff::Answered(Err(_)) => Err(Refusal::BadArg),
        mpsc::Handoff::Taken => Ok(Some(PENDING.into())),
        mpsc::Handoff::Withdrawn => Err(Refusal::Busy),
    }
}

/// An `ApplyConfig` refusal as an `ack` error: a playlist that is not
/// there, or one that would be empty, is the caller's `bad-arg`.
#[optimize(size)]
#[inline(never)]
fn refusal_of(e: &str) -> Refusal {
    if e == crate::state::SET_NOT_FOUND || e.starts_with(crate::state::FILTER_EMPTY) {
        Refusal::BadArg
    } else {
        Refusal::Busy
    }
}

/// The player answered a configuration change: once applied, the shared
/// settings follow it (as `PUT /api/config`'s handler does), on CPU0.
#[optimize(size)]
#[inline(never)]
/// Only the field asked changes in the shared settings (a LAN change made
/// meanwhile stays); `Some(after)`: changed, the card's copy is due that
/// long from now. A brightness once commands stop; a playlist at once (the
/// card's copy wins at boot, and a playlist change is rare).
pub fn settle(link: &Link, p: Pending, answer: Result<Applied, String>) -> Option<Duration> {
    crate::state::drop_spent();
    match answer {
        Ok(_) => {
            let mut s = link.settings.lock().unwrap();
            let mut after = crate::config::REMOTE_SAVE_AFTER;
            if let Some(v) = p.brightness {
                s.brightness = v;
            }
            if let Some(set) = p.set {
                s.set = set;
                after = Duration::ZERO;
            }
            Some(after)
        }
        Err(e) => {
            log::info!("connect: a remote setting refused: {e}");
            None
        }
    }
}

/// What this panel can play by reference (`formulas`, protocol 5.6): on
/// the first page the built-in Formulas (`@formula:<stem>`, named from
/// their compiled header), then, with a card it can read, a page of the
/// card's (`/formulas/<stem>.psi`, `card_formulas::remote_list`), bounded
/// to the panel's text limit (`proto::FormulaList`). A build without
/// Formulas plays none: two empty lists.
#[optimize(size)]
#[inline(never)]
fn formulas_page(card: bool, after: Option<&str>) -> (String, Option<String>) {
    let mut out = proto::FormulaList::new(card);
    #[cfg(feature = "psi")]
    if after.is_none() {
        use opendmd_core::synth;
        for stem in synth::bundled_names().filter(|s| proto::formula_stem_ok(s)) {
            let name = synth::bundled(stem).and_then(synth::compiled_name).unwrap_or_else(|| synth::stem_title(stem));
            out.builtin(stem, &name);
        }
    }
    if !card {
        return (out.finish(false, None), None);
    }
    #[cfg(feature = "psi")]
    let (cut, next) = crate::card_formulas::remote_list(&mut out, after);
    #[cfg(not(feature = "psi"))]
    let (cut, next) = {
        let _ = after;
        (false, None)
    };
    let r = out.finish(cut, next.as_deref());
    (r, if cut { next } else { None })
}

/// `{"sets":[…],"more":bool}`, bounded so the `ack` that carries it (its
/// key id, `n` and the rest: under 100 B) stays inside the panel's text
/// limit.
#[optimize(size)]
#[inline(never)]
fn sets_result(names: &[String], cut: bool) -> String {
    const ROOM: usize = proto::RESULT_ROOM;
    let mut s = String::from(r#"{"sets":["#);
    let mut more = cut;
    for (i, n) in names.iter().enumerate() {
        let q = opendmd_core::connect::json::quote(n);
        if s.len() + q.len() + 1 + r#"],"more":true}"#.len() > ROOM {
            more = true;
            break;
        }
        if i > 0 {
            s.push(',');
        }
        s.push_str(&q);
    }
    s.push_str(if more { r#"],"more":true}"# } else { r#"],"more":false}"# });
    s
}

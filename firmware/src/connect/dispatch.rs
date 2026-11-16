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
use crate::http::{self, ConfigPatch};
use crate::nvs_config::Settings;
use crate::state::{Applied, Command, PlayError, Status};
use opendmd_core::connect::codec::hex;
use opendmd_core::connect::keys::{key_id, Entry};
use opendmd_core::connect::proto::{self, Accepted, Command as Remote, Refusal, MAX_TEXT_OUT};
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
    pub shared: Box<Settings>,
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
            Remote::Brightness { v } => self.config(ConfigPatch { brightness: Some(v), ..Default::default() }),
            Remote::Playlist { set } => self.config(ConfigPatch { set: Some(set), ..Default::default() }),
            Remote::Playlists => self.playlists(),
            Remote::Controllers => Ok(Some(self.keys.controllers_result(|k| {
                self.last_use.iter().find(|(x, _)| x == k).map(|(_, at)| at.elapsed().as_secs())
            }))),
            Remote::AddController { public, label } => self.add(k, public, label),
            Remote::RemoveController { k: gone } => self.remove(gone),
            Remote::Name { name } => self.rename(name),
            // GIF sends come with P1's F4; `hello` says `send: false`.
            Remote::Send(_) | Remote::SendCancel { .. } => Err(Refusal::NotAllowed),
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
    fn play(&mut self, path: String) -> Done {
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
    fn config(&mut self, patch: ConfigPatch) -> Done {
        if self.pending.is_some() {
            return Err(Refusal::Busy);
        }
        let mut next = Box::new(self.link.settings.lock().unwrap().clone());
        if http::apply_patch(&mut next, &patch).is_err() {
            return Err(Refusal::BadArg);
        }
        let shared = next.clone();
        let (tx, rx) = mpsc::channel();
        self.link
            .cmd
            .send(Command::ApplyConfig { settings: next, panel: None, look_named: false, boot_named: false, reply: tx })
            .map_err(|_| Refusal::Busy)?;
        crate::wake::kick();
        match rx.answer_within(http::TAKE_WAIT, http::PLAY_ANSWER) {
            mpsc::Handoff::Answered(answer) => {
                let refused = answer.as_ref().err().map(|e| refusal_of(e));
                settle(&self.link, Pending { rx, shared, until: Instant::now() }, answer);
                refused.map_or(Ok(None), Err)
            }
            mpsc::Handoff::Taken => {
                self.pending = Some(Pending { rx, shared, until: Instant::now() + CONFIG_WAIT });
                Ok(Some(PENDING.into()))
            }
            mpsc::Handoff::Withdrawn => Err(Refusal::Busy),
        }
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
        // In the directory's order: a controller sorts them.
        let names: Vec<String> = dir
            .filter_map(|e| e.ok())
            .take(SETS_MAX)
            .filter_map(|e| e.file_name().to_str().and_then(|n| n.strip_suffix(".txt")).map(str::to_string))
            .filter(|n| proto::remote_set_ok(n) && !n.is_empty())
            .collect();
        Ok(Some(sets_result(&names)))
    }

    #[optimize(size)]
    #[inline(never)]
    fn add(&mut self, by: [u8; 8], public: [u8; 32], label: String) -> Done {
        let added = crate::clock::unix_valid().and_then(|t| u32::try_from(t).ok()).unwrap_or(0);
        let k = key_id(&public);
        change(
            &self.link.cmd,
            |s| {
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
            |s| {
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
        change(&self.link.cmd, |_| Ok((Store::Name(name.clone()), name)), |s, n| {
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
pub fn settle(link: &Link, p: Pending, answer: Result<Applied, String>) {
    crate::state::drop_spent();
    match answer {
        Ok(_) => http::commit_config(&link.settings, p.shared, false, false),
        Err(e) => log::info!("connect: a remote setting refused: {e}"),
    }
}

/// `{"sets":[…],"more":bool}`, bounded so the `ack` that carries it (its
/// key id, `n` and the rest: under 100 B) stays inside the panel's text
/// limit.
#[optimize(size)]
#[inline(never)]
fn sets_result(names: &[String]) -> String {
    const ROOM: usize = MAX_TEXT_OUT - 100;
    let mut s = String::from(r#"{"sets":["#);
    let mut more = false;
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

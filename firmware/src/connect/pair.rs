//! Pairing on the panel (`docs/connect-protocol.md` 6.1; P1-PLAN 2.6),
//! run by [`Client::pairing`] once a turn from `Client::tick`, never from
//! inside the session's read path (`service` → `act` → `frame`), whose
//! stack is the worker's tightest: the MACs hash here, the key is stored
//! here (waiting on the main task), the QR code is made here.
//!
//! - **The window** is the LAN route's (`api`): `connect::Pairing` in the
//!   shared state. A dead one (used, expired, three wrong MACs) is dropped
//!   here; one closed or replaced on the LAN is already gone.
//! - **The server** hears of each window once (`pair-open`, its time left)
//!   and of its end at once (`pair-close`), in the same turn it dies, also
//!   after three wrong MACs; after a reconnect what is open is said again
//!   and an announced window that died meanwhile closed.
//! - **A guest's `pair`**: the window checks the MAC over the label as
//!   sent (one use, three tries); only then is the label cleaned (the
//!   controller cleans and bounds it before its MAC, so a correct one is
//!   never spoilt here). The key is stored with `can_add`; `keys` goes,
//!   then `paired {k, mac2}`, then
//!   `pair-close`. A store that fails answers `busy`; the window is used
//!   up either way.
//! - **What can still be lost**: a session that ends between the store
//!   and the answer (a read error, a close) drops `paired`: the key stays
//!   stored (the device is listed in Settings, where it can be removed),
//!   the controller sees no answer and asks for a new link. A store
//!   refused `busy` twice answers `busy`.
//! - **The screens**: the QR code while a window opened with `qr=1` is
//!   open, then "Remote control added" for 10 s after a pairing, through
//!   `Command::ConnectScreen` (the main task draws them).

use super::api::LINK;
use super::client::{core_now, Client, PairReq, Shown, PAIR_QUEUE};
use super::tls::Phase;
use super::{change, error, shared, Screen, Store, ADDED_FOR};
use crate::state::Command;
use opendmd_core::connect::codec::hex;
use opendmd_core::connect::keys::{clean_label, key_id, usable_public_key, Entry, FLAG_CAN_ADD};
use opendmd_core::connect::pair::{pair_id, PairError};
use opendmd_core::connect::{proto, qr};
use std::time::{Duration, Instant};

/// A pairing whose MAC checked (the window spent), its key to store.
pub(super) struct PairStore {
    g: u8,
    public: [u8; 32],
    label: String,
    mac2: [u8; 32],
    /// Already tried once and refused `busy`.
    retried: bool,
}

impl Client {
    /// The turn's pairing work (see the module's notes).
    #[optimize(size)]
    #[inline(never)]
    pub(super) fn pairing(&mut self) {
        let now = core_now();
        // A store refused `busy` last turn: once more, then answered.
        if let Some(p) = self.pair_store.take() {
            self.store_paired(p);
        }
        for r in std::mem::take(&mut self.pair_reqs) {
            self.pair_one(r, now);
        }
        // The window as it stands: a dead one dropped; a new one's secret
        // taken out once (to hash and draw), never again.
        let known = self.window.as_ref().map(|w| w.0);
        let cur = shared(|s| {
            if s.pairing.as_ref().is_some_and(|p| !p.win.is_open(now)) {
                s.pairing = None;
            }
            s.pairing.as_ref().map(|p| {
                let left = p.win.left(now).unwrap_or_default();
                (p.seq, p.qr, left, (known != Some(p.seq)).then(|| p.win.secret().to_string()))
            })
        })
        .flatten();
        match &cur {
            Some((seq, qr_wanted, _, Some(secret))) => {
                let code = qr_wanted.then(|| qr::encode(format!("{LINK}{secret}").as_bytes()).map(Box::new)).flatten();
                self.window = Some((*seq, pair_id(secret), code));
            }
            Some(_) => {}
            None => self.window = None,
        }
        let want = self.window.as_ref().map(|w| (w.1, cur.as_ref().map_or(0, |c| c.2.as_secs().max(1) as u32)));
        if let Err(e) = self.announce(want) {
            return self.end(e);
        }
        self.screens(Instant::now());
    }

    /// Tell the server what is open, and what is not any more.
    #[optimize(size)]
    #[inline(never)]
    pub(super) fn announce(&mut self, want: Option<([u8; 10], u32)>) -> Result<(), super::End> {
        if !self.conn.as_ref().is_some_and(|c| c.phase == Phase::Open) || !self.paired_due.is_empty() || self.pair_store.is_some() {
            // (Offline: said at the next session. A pairing's answer first.)
            return Ok(());
        }
        let want_id = want.map(|w| w.0);
        if let Some(old) = self.announced.filter(|a| Some(*a) != want_id) {
            self.conn.as_mut().expect("held").text(&proto::pair_close(&old))?;
            self.announced = None;
            log::info!("connect: pair-close sent");
        }
        if let Some((id, ttl)) = want {
            if self.announced != Some(id) || self.reannounce {
                if !self.meta_ok() {
                    // Within the second.
                    return Ok(());
                }
                self.conn.as_mut().expect("held").text(&proto::pair_open(&id, ttl))?;
                self.announced = Some(id);
                log::info!("connect: pair-open sent ({ttl} s)");
            }
        }
        self.reannounce = false;
        Ok(())
    }

    /// One guest's `pair`: the MAC, then the key stored, then the answer.
    #[optimize(size)]
    #[inline(never)]
    fn pair_one(&mut self, r: PairReq, now: Duration) {
        let PairReq { seq, g, public, label, mac } = r;
        let Some((dev, _)) = self.identity else { return };
        // Checked against the window it came for: one cancelled or replaced
        // since is closed to it.
        let checked = shared(|s| match s.pairing.as_mut() {
            Some(p) if Some(p.seq) == seq => p.win.check(&dev.0, &public, label.as_bytes(), &mac, now),
            _ => Err(PairError::Closed),
        })
        .unwrap_or(Err(PairError::Closed));
        let answer = match checked {
            Err(e) => {
                let left = match e {
                    PairError::BadMac { left } => format!(" ({left} tries left)"),
                    _ => String::new(),
                };
                log::info!("connect: a pairing try refused: {}{left}", e.code());
                proto::paired_err(g, e.code())
            }
            // The window is used up. The label is cleaned only now.
            Ok(mac2) => match (clean_label(&label), usable_public_key(&public)) {
                (Some(clean), true) => {
                    self.store_paired(PairStore { g, public, label: clean, mac2, retried: false });
                    return;
                }
                _ => {
                    log::info!("connect: a pairing's MAC was right but its label or key is not usable");
                    proto::paired_err(g, "bad-arg")
                }
            },
        };
        if let Some(conn) = self.conn.as_mut().filter(|c| c.phase == Phase::Open) {
            if let Err(e) = conn.text(&answer) {
                self.end(e);
            }
        }
    }

    /// Store the paired key, then answer: `keys` and `paired` in the same
    /// turn when the budget has a token (one back a second, up to 4), else
    /// `paired` waits for `keys` (`paired_due`, `sync_out`). A store
    /// refused `busy` (another store under way, or the main task slow to
    /// answer) is tried once more next turn before `busy` is answered; the
    /// window is spent either way.
    #[optimize(size)]
    #[inline(never)]
    fn store_paired(&mut self, p: PairStore) {
        let PairStore { g, public, label, mac2, retried } = p;
        let added = crate::clock::unix_valid().and_then(|t| u32::try_from(t).ok()).unwrap_or(0);
        let entry = Entry { public, flags: FLAG_CAN_ADD, label: label.clone(), added, added_by: None };
        let stored = change(
            &self.link.cmd,
            move |s| {
                let mut keys = s.keys.clone();
                keys.add(entry).map_err(|e| format!("{e:?}"))?;
                Ok((Store::Keys(keys.encode()), keys))
            },
            |s, keys| {
                s.keys = keys;
                s.keys_changed = true;
            },
        );
        let answer = match stored {
            Err(e) if !retried && (e == "busy" || e == super::NOT_IN_TIME) => {
                log::info!("connect: a pairing's key store: {e}; once more at the next turn");
                self.pair_store = Some(PairStore { g, public, label, mac2, retried: true });
                return;
            }
            Err(e) => {
                error(format!("a pairing's key was not stored: {e}"));
                proto::paired_err(
                    g,
                    match e.as_str() {
                        "Full" => "quota",
                        "Duplicate" | "BadKey" | "BadLabel" => "bad-arg",
                        _ => "busy",
                    },
                )
            }
            Ok(()) => {
                let k = key_id(&public);
                log::info!("connect: a device paired on the panel (key {})", hex(&k[..4]));
                self.added = Some((label, Instant::now() + ADDED_FOR));
                let _ = self.sync_shared();
                if self.paired_due.len() < PAIR_QUEUE {
                    self.paired_due.push(proto::paired(g, &k, &mac2));
                }
                if let Err(e) = self.sync_out() {
                    self.end(e);
                }
                return;
            }
        };
        if let Some(conn) = self.conn.as_mut().filter(|c| c.phase == Phase::Open) {
            if let Err(e) = conn.text(&answer) {
                self.end(e);
            }
        }
    }

    /// What the panel shows: the QR code of an open `qr=1` window, or
    /// "added" for its 10 s, or nothing.
    #[optimize(size)]
    #[inline(never)]
    fn screens(&mut self, now: Instant) {
        if self.added.as_ref().is_some_and(|(_, until)| now >= *until) {
            self.added = None;
        }
        let want = if self.added.is_some() {
            Shown::Added
        } else {
            match &self.window {
                Some((seq, _, Some(_))) => Shown::Qr(*seq),
                _ => Shown::None,
            }
        };
        if want == self.shown {
            return;
        }
        let screen = match want {
            Shown::None => None,
            Shown::Added => self.added.as_ref().map(|(l, _)| Box::new(Screen::Added(l.clone()))),
            Shown::Qr(_) => self.window.as_ref().and_then(|w| w.2.as_ref()).map(|q| Box::new(Screen::Pair(q.clone()))),
        };
        if self.link.cmd.send(Command::ConnectScreen(screen)).is_ok() {
            crate::wake::kick();
            self.shown = want;
        }
    }
}

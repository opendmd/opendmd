//! Remote control's LAN routes (`docs/api.md` "Remote control"). Every one
//! is under `/api/`, so `httpd::gate` asks for the API key where one is set
//! (`access::needs_key`) before any of these runs, the pairing route
//! included: it is no weaker than the rest of the LAN API.
//!
//! - `GET /api/connect`: whether it is on, its state, the controllers
//!   (labels and dates stay on the panel; never the panel's secret), an
//!   open pairing window's time left (never its secret).
//! - `PUT /api/connect` `{"enabled":bool}` and/or `{"name":str}`: the
//!   switch (on starts at once; off closes any pairing window), the
//!   panel's name (`""`: its hostname).
//! - `POST /api/connect/pair[?qr=1]`: a new pairing window (any open one
//!   goes); the answer's `link` (`https://opendmd.dev/c#<S>`) carries the
//!   secret, for this caller only.
//!   With `qr=1` the panel shows the QR code until the window dies.
//! - `DELETE /api/connect/pair`: the window goes (and the QR code).
//! - `POST /api/connect/forget`: a new identity; every controller goes.
//! - `DELETE /api/connect/controllers/<k>`: one controller goes.
//!
//! The routes only make the secret (random bytes and base32) and change
//! the shared state: hashing it (`pair_id`), the QR code and the MAC checks
//! run on the network worker, at its turn's top level.

use super::client::core_now;
use super::{change, shared, Pairing, Store, Window, PAIR_TTL};
use crate::http::Ctx;
use crate::httpd::{json_string, query_param, Method, Respond, Server};
use opendmd_core::connect::codec::unhex;
use opendmd_core::connect::json::Value;
use opendmd_core::connect::keys::{clean_text, KeyList};
use opendmd_core::connect::pair::secret_from_random;
use opendmd_core::connect::proto::MAX_NAME_CHARS;
use std::sync::Arc;

/// The link a pairing secret goes in, for the LAN caller and in the QR
/// code alike (`docs/connect-protocol.md` 6.1; `/c` hands the fragment to
/// `/connect/`): 42 bytes, version 3-L.
pub const LINK: &str = "https://opendmd.dev/c#";
const _: () = assert!(LINK.len() + opendmd_core::connect::pair::SECRET_CHARS <= opendmd_core::connect::qr::MAX_DATA);

/// `GET /api/connect`'s body.
#[optimize(size)]
#[inline(never)]
fn connect_json() -> String {
    let now = core_now();
    let (on, registered, controllers, name, pairing) = shared(|s| {
        let pairing = s.pairing.as_ref().and_then(|p| p.win.left(now).map(|l| format!(r#"{{"expires_s":{},"qr":{}}}"#, l.as_secs(), p.qr)));
        (s.on, s.identity.is_some(), s.keys.controllers_result(|_| None), s.name.clone(), pairing)
    })
    .unwrap_or((false, false, "[]".into(), String::new(), None));
    format!(
        r#"{{"enabled":{on},"registered":{registered},"name":{},"controllers":{controllers},"pairing":{},{}}}"#,
        json_string(&name),
        pairing.as_deref().unwrap_or("null"),
        super::lite_fields()
    )
}

/// What a store's failure answers.
#[optimize(size)]
#[inline(never)]
fn refused(req: &mut crate::httpd::Req, e: &str) -> anyhow::Result<()> {
    if e == "busy" {
        req.header("Retry-After", "1")?;
        return req.error(503, "503 Service Unavailable", "busy; retry shortly");
    }
    req.error(500, "500 Internal Server Error", e)
}

/// `PUT /api/connect`'s body: `enabled` and/or `name`, nothing else.
#[optimize(size)]
#[inline(never)]
fn put_body(body: &[u8]) -> Option<(Option<bool>, Option<String>)> {
    let v = opendmd_core::connect::json::parse(body).ok()?;
    let Value::Object(ref m) = v else { return None };
    // An emulator image's lane: `test_no_card` stands in for a card taken
    // out, for sends only (`connect-test` builds; anywhere else a 400).
    #[cfg(feature = "connect-test")]
    {
        let mut tests = 0;
        for (k, flag) in [("test_no_card", &super::TEST_NO_CARD), ("test_fail_write", &super::TEST_FAIL_WRITE)] {
            if let Some(t) = v.get(k) {
                flag.store(t.as_bool()?, std::sync::atomic::Ordering::Relaxed);
                tests += 1;
            }
        }
        if tests > 0 && m.len() == tests {
            return Some((None, None));
        }
    }
    let test = |k: &str| cfg!(feature = "connect-test") && (k == "test_no_card" || k == "test_fail_write");
    if m.is_empty() || m.iter().any(|(k, _)| k != "enabled" && k != "name" && !test(k)) {
        return None;
    }
    let on = match v.get("enabled") {
        None => None,
        Some(e) => Some(e.as_bool()?),
    };
    let name = match v.get("name") {
        None => None,
        // "" puts the hostname back; anything else is cleaned as a
        // controller's `name` command is.
        Some(n) if n.as_str()?.trim().is_empty() => Some(String::new()),
        Some(n) => Some(clean_text(n.as_str()?, 4 * MAX_NAME_CHARS, MAX_NAME_CHARS)?),
    };
    Some((on, name))
}

#[optimize(size)]
#[inline(never)]
pub fn routes(server: &mut Server, ctx: Arc<Ctx>) -> anyhow::Result<()> {
    server.route("/api/connect", Method::Get, #[optimize(size)] |req| req.json(&connect_json()))?;

    let c = ctx.clone();
    server.route_json("/api/connect", Method::Put, #[optimize(size)] move |req| {
        let Some(body) = crate::http::json_body(req)? else {
            return Ok(());
        };
        let Some((on, name)) = put_body(&body) else {
            return req.bad_request(r#"the body is {"enabled": true|false} and/or {"name": "..."} (up to 32 characters; "" for the hostname)"#);
        };
        if let Some(name) = name {
            if let Err(e) = change(&c.cmd, move |_| Ok((Store::Name(name.clone()), name)), |s, n| {
                s.name = n;
                s.name_changed = true;
            }) {
                return refused(req, &e);
            }
            log::info!("connect: the panel renamed on the LAN");
        }
        if let Some(on) = on {
            if let Err(e) = change(&c.cmd, move |_| Ok((Store::On(on), on)), |s, on| {
                s.on = on;
                // On from Settings: at once (no hold, no backoff; after two
                // crashes too). Off: no window stays open.
                s.start_now = on;
                if !on {
                    s.pairing = None;
                }
            }) {
                return refused(req, &e);
            }
            log::info!("connect: Remote control switched {}", if on { "on" } else { "off" });
        }
        req.json(&connect_json())
    })?;

    let c = ctx.clone();
    server.route("/api/connect/pair", Method::Post, #[optimize(size)] move |req| {
        // Night mode's dark panel shows no code: the link only.
        let dark = c.status.lock().unwrap().mode == "off";
        let qr = query_param(req.query(), "qr").as_deref() == Some("1") && !dark;
        let (online, refused, too_old) = super::lite(|l| (l.state == "open", l.refused, l.too_old));
        // 100 random bits; nothing here hashes it (the worker does).
        let secret = secret_from_random(&super::tls::random::<13>());
        let link = format!("{LINK}{secret}");
        let now = core_now();
        let opened = shared(move |s| {
            // A window nobody can reach is not opened: off, not registered
            // yet, or refused by the server (or too old for it).
            if !s.on {
                return Err("Remote control is off: switch it on first");
            }
            if s.identity.is_none() {
                return Err("Remote control is not set up yet: try again in a minute");
            }
            if refused || too_old {
                return Err(if refused { "opendmd.dev does not know this panel: Forget all, then link again" } else { "this panel needs a firmware update for Remote control" });
            }
            s.pair_seq = s.pair_seq.wrapping_add(1);
            s.pairing = Some(Pairing { win: Window::open_for(secret, now, PAIR_TTL), qr, seq: s.pair_seq });
            s.gen = s.gen.wrapping_add(1);
            Ok(())
        })
        .unwrap_or(Err("Remote control is not running"));
        req.header("Cache-Control", "no-store")?;
        if let Err(why) = opened {
            return req.error(409, "409 Conflict", why);
        }
        crate::wake::kick();
        log::info!("connect: a pairing window opened on the LAN{}", if qr { ", its QR code on the panel" } else { "" });
        req.json(&format!(r#"{{"link":{},"expires_s":{},"qr":{qr},"dark":{dark},"online":{online}}}"#, json_string(&link), PAIR_TTL.as_secs()))
    })?;

    server.route("/api/connect/pair", Method::Delete, #[optimize(size)] move |req| {
        if shared(|s| s.pairing.take().is_some()).unwrap_or(false) {
            log::info!("connect: the pairing window closed on the LAN");
        }
        req.json(&connect_json())
    })?;

    let c = ctx.clone();
    server.route("/api/connect/forget", Method::Post, #[optimize(size)] move |req| {
        match change(&c.cmd, |_| Ok((Store::Forget, ())), |s, ()| {
            s.identity = None;
            s.keys = KeyList::new();
            s.keys_changed = true;
            s.forget = true;
            s.pairing = None;
        }) {
            Ok(()) => {
                log::info!("connect: identity forgotten, every controller removed");
                req.json(&connect_json())
            }
            Err(e) => refused(req, &e),
        }
    })?;

    let c = ctx;
    server.route("/api/connect/controllers/*", Method::Delete, #[optimize(size)] move |req| {
        let Some(k) = unhex::<8>(req.path().trim_start_matches("/api/connect/controllers/")) else {
            return req.not_found("no such controller");
        };
        if shared(|s| s.keys.find(&k).is_none()).unwrap_or(true) {
            return req.not_found("no such controller");
        }
        match change(
            &c.cmd,
            move |s| {
                let mut keys = s.keys.clone();
                keys.remove(&k).map_err(|e| format!("{e:?}"))?;
                Ok((Store::Keys(keys.encode()), keys))
            },
            |s, keys| {
                s.keys = keys;
                s.keys_changed = true;
            },
        ) {
            Ok(()) => {
                log::info!("connect: a controller removed on the LAN");
                req.json(&connect_json())
            }
            Err(e) if e == "Unknown" => req.not_found("no such controller"),
            Err(e) => refused(req, &e),
        }
    })?;
    Ok(())
}

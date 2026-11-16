//! Remote control's LAN routes (`docs/api.md` "Remote control"; the API
//! key applies to them as to every `/api/` route):
//!
//! - `GET /api/connect`: whether it is on, its state, the controllers
//!   (labels and dates stay on the panel; never the panel's secret).
//! - `PUT /api/connect` `{"enabled":bool}`: the switch (on starts at once).
//! - `POST /api/connect/forget`: a new identity; every controller goes.
//! - `DELETE /api/connect/controllers/<k>`: one controller goes.
//!
//! Pairing (`POST /api/connect/pair`, `DELETE /api/connect/pair`) comes
//! with P1's F3.

use super::{change, shared, Store};
use crate::http::Ctx;
use crate::httpd::{json_string, Method, Respond, Server};
use opendmd_core::connect::codec::unhex;
use opendmd_core::connect::keys::KeyList;
use std::sync::Arc;


/// `GET /api/connect`'s body.
#[optimize(size)]
#[inline(never)]
fn connect_json() -> String {
    let (on, registered, controllers, name) = shared(|s| (s.on, s.identity.is_some(), s.keys.controllers_result(|_| None), s.name.clone())).unwrap_or((false, false, "[]".into(), String::new()));
    format!(r#"{{"enabled":{on},"registered":{registered},"name":{},"controllers":{controllers},"pairing":null,{}}}"#, json_string(&name), super::lite_fields())
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

#[optimize(size)]
#[inline(never)]
pub fn routes(server: &mut Server, ctx: Arc<Ctx>) -> anyhow::Result<()> {
    server.route("/api/connect", Method::Get, #[optimize(size)] |req| req.json(&connect_json()))?;

    let c = ctx.clone();
    server.route_json("/api/connect", Method::Put, #[optimize(size)] move |req| {
        let Some(body) = crate::http::json_body(req)? else {
            return Ok(());
        };
        // `{"enabled": bool}` and nothing else, read by the protocol's strict
        // JSON reader (no serde derive for one field).
        let v = opendmd_core::connect::json::parse(&body).ok();
        let on = match v.as_ref().and_then(|v| v.get("enabled")).and_then(|e| e.as_bool()) {
            Some(on) if matches!(v, Some(opendmd_core::connect::json::Value::Object(ref m)) if m.len() == 1) => on,
            _ => return req.bad_request(r#"the body is {"enabled": true|false}"#),
        };
        match change(&c.cmd, move |_| Ok((Store::On(on), on)), |s, on| {
            s.on = on;
            // On from Settings: at once (no hold, no backoff; after two
            // crashes too).
            s.start_now = on;
        }) {
            Ok(()) => {
                log::info!("connect: Remote control switched {}", if on { "on" } else { "off" });
                req.json(&connect_json())
            }
            Err(e) => refused(req, &e),
        }
    })?;

    let c = ctx.clone();
    server.route("/api/connect/forget", Method::Post, #[optimize(size)] move |req| {
        match change(&c.cmd, |_| Ok((Store::Forget, ())), |s, ()| {
            s.identity = None;
            s.keys = KeyList::new();
            s.keys_changed = true;
            s.forget = true;
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

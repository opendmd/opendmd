//! Who may use the player's HTTP API: the pure part of the firmware's
//! request gate (`firmware/src/httpd.rs`), here so it is tested on the host.
//! `docs/api.md`, "Security", is the user-facing description.
//!
//! Four rules, none of which is a defence against someone on the same WiFi
//! (the device speaks plain HTTP):
//!
//! - **Host.** Every request, `GET` included, must name the panel in its
//!   `Host` header: `opendmd.local`, `<hostname>.local` (the configured
//!   name, and the one mDNS answers to), the bare names, the panel's
//!   current address, or `localhost` (the emulator's port forward). Anything
//!   else is a 421: a page whose DNS name was re-pointed at the panel (DNS
//!   rebinding) sends its own name, which matches its `Origin`, so the
//!   Origin rule alone cannot catch it. No `Host` at all (HTTP/1.0 scripts)
//!   is fine: a browser always sends one.
//! - **Origin.** A mutating request (`POST`, `PUT`, `DELETE`) whose `Origin`
//!   header names another host than its `Host` header is refused (403): a
//!   page from elsewhere cannot drive the player through a browser on the
//!   LAN. No `Origin` (curl, scripts) is fine.
//! - **Content type.** A JSON body must say `application/json` (415), so a
//!   cross-site `<form>` (which can only send `text/plain`, urlencoded or
//!   multipart without a preflight) cannot reach a JSON endpoint even from a
//!   browser that sends no `Origin`.
//! - **An optional API key.** With one set, the API and the card's files
//!   need `Authorization: Bearer <key>` or `?key=<key>` (401).

/// The shortest and longest key `PUT /api/config` accepts.
pub const KEY_MIN: usize = 8;
pub const KEY_MAX: usize = 64;

/// Is `k` a key that may be stored? 8..=64 of `A-Z a-z 0-9 . _ ~ -` (URL-safe,
/// so `?key=` needs no encoding). The empty string (no key) is handled by
/// the caller.
pub fn valid_key(k: &str) -> bool {
    (KEY_MIN..=KEY_MAX).contains(&k.len())
        && k.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'~' | b'-'))
}

/// Does a request for `path` (no query) need the key when one is set? The
/// API and the card's files; not the web app's own files or `/setup`.
pub fn needs_key(path: &str) -> bool {
    path.starts_with("/api/") || is_card_path(path)
}

/// Is `path` (no query) one of the card's files served as they are
/// (`/gifs/`, `/cache/`, `/sets/`, `/formulas/`, `/filelist*`)? They go out
/// as inert bytes (`card_media_type`), never as a page of the panel's.
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn is_card_path(path: &str) -> bool {
    path.starts_with("/gifs/")
        || path.starts_with("/cache/")
        || path.starts_with("/sets/")
        || path.starts_with("/formulas/")
        || path.starts_with("/filelist")
}

/// The `Content-Type` a card file goes out with: an image's own media type
/// (the web app shows GIFs and thumbnails with `<img>`), `None` for
/// anything else, which is sent as `application/octet-stream` with
/// `Content-Disposition: attachment`. Never `text/html`, never SVG: a card
/// file written by anyone on the LAN must not run as a page on the panel's
/// origin, where the API key lives.
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn card_media_type(path: &str) -> Option<&'static str> {
    let p = path.as_bytes();
    let ends = |e: &[u8]| p.len() >= e.len() && p[p.len() - e.len()..].eq_ignore_ascii_case(e);
    [
        (&b".gif"[..], "image/gif"),
        (b".png", "image/png"),
        (b".jpg", "image/jpeg"),
        (b".jpeg", "image/jpeg"),
        (b".webp", "image/webp"),
    ]
    .iter()
    .find(|(e, _)| ends(e))
    .map(|&(_, t)| t)
}

/// The web app's `Content-Security-Policy` (the panel's `www::serve`, and
/// `tools/mock-device.py`, which copies it). Its own scripts and wasm;
/// inline styles (`h()` sets `style`); pictures and fetches from anywhere
/// (Create Art's sources, opendmd.dev's gallery, relay and release, a
/// development build's `?updates=` feed on localhost); never in a frame.
/// What it is for is script: only the app's own runs.
pub const APP_CSP: &str = "default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; \
style-src 'self' 'unsafe-inline'; img-src * data: blob:; media-src 'self' blob:; connect-src *; \
object-src 'none'; base-uri 'self'; frame-ancestors 'none'";

/// The setup page's: one self-contained page, its script and styles inline
/// (so no script-src to speak of); never in a frame.
pub const SETUP_CSP: &str = "frame-ancestors 'none'; object-src 'none'; base-uri 'none'; form-action 'none'";

/// `host[:port]` (a `Host` header) without its port, lowercased, a trailing
/// dot dropped (`[::1]:8080` is `[::1]`).
#[cfg_attr(target_arch = "xtensa", optimize(size))]
fn host_name(s: &str) -> String {
    let s = s.trim().to_ascii_lowercase();
    let name = match s.rsplit_once(':') {
        Some((h, p)) if !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()) => h,
        _ => &s,
    };
    name.strip_suffix('.').unwrap_or(name).to_string()
}

/// The names the panel answers to, beside `opendmd.local` and `localhost`.
#[derive(Debug, Clone, Copy, Default)]
pub struct Names<'a> {
    /// The configured name (`hostname`), "" if none.
    pub hostname: &'a str,
    /// The name mDNS answers to (`<hostname>-2` after a clash), "" if none.
    pub mdns: &'a str,
    /// The panel's current address (the station's, or the setup network's).
    pub ip: &'a str,
    /// The name it had before its last rename this boot: a page still open
    /// at the old address must be able to follow it to the new one.
    pub was: &'a str,
}

/// Is `host` (a request's `Host` header) one of the panel's own names? Any
/// port: the name is what DNS rebinding controls, not the port.
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn host_allowed(host: &str, names: &Names) -> bool {
    let h = host_name(host);
    if h.is_empty() {
        return false;
    }
    if matches!(
        h.as_str(),
        "localhost" | "127.0.0.1" | "[::1]" | "opendmd.local" | "opendmd"
    ) {
        return true;
    }
    // (`h` is lowercase; a stored name is too: `hostname::valid`.)
    let bare = h.strip_suffix(".local").unwrap_or(&h);
    [names.hostname, names.mdns, names.was]
        .iter()
        .any(|n| !n.is_empty() && bare.eq_ignore_ascii_case(n))
        || (!names.ip.is_empty() && h == names.ip)
}

/// The 421's words.
pub const MISDIRECTED: &str =
    "this panel answers only to its own names: opendmd.local, <its name>.local or its IP address";

/// What the gate does with a request while the panel is the open setup
/// network (`SETUP_MODE`): the WiFi setup, and nothing else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Setup {
    /// One of the routes the setup page needs.
    Allow,
    /// A page or probe from anywhere: sent to the setup page (the captive
    /// portal).
    Portal,
    /// Not on the setup network (403).
    Refuse,
}

/// The phones' and laptops' captive-portal probes (`http.rs` registers
/// each).
pub const PORTAL_PROBES: [&str; 6] = [
    "/hotspot-detect.html", // Apple
    "/generate_204",        // Android
    "/gen_204",             // Android, older
    "/ncsi.txt",            // Windows
    "/connecttest.txt",     // Windows 10+
    "/canonical.html",      // Firefox
];

/// While in setup mode: the setup page, its status, the WiFi scan, the
/// settings (`PUT /api/config` limited to [`SETUP_CONFIG_KEYS`] by its
/// handler) and the restart that joins the network. Every other API route
/// and every card file is refused; any other page is the setup page.
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn setup_route(method: &str, path: &str) -> Setup {
    let get = method == "GET";
    match path {
        "/" | "/setup" if get => Setup::Allow,
        "/api/status" | "/api/config" | "/api/wifi/scan" if get => Setup::Allow,
        "/api/config" if method == "PUT" => Setup::Allow,
        "/api/reboot" if method == "POST" => Setup::Allow,
        p if get && PORTAL_PROBES.contains(&p) => Setup::Allow,
        p if get && !p.starts_with("/api/") && !is_card_path(p) => Setup::Portal,
        _ => Setup::Refuse,
    }
}

/// The settings `PUT /api/config` may change on the setup network: the
/// WiFi (a work or school network's login included: `wifi_login`), the
/// display's name and the API key (set, or `""` to clear a lost one).
pub const SETUP_CONFIG_KEYS: [&str; 7] = [
    "wifi_ssid",
    "wifi_pass",
    "wifi_eap_user",
    "wifi_eap_identity",
    "wifi_eap_ttls",
    "hostname",
    "api_key",
];

fn host_part(s: &str) -> String {
    let s = s.trim().to_ascii_lowercase();
    let s = s.strip_suffix('/').unwrap_or(&s);
    let s = s.strip_suffix(":80").unwrap_or(s);
    s.to_string()
}

/// Is `origin` (an `Origin` header: `http://host[:port]`, or `null`) the
/// same host as `host` (the `Host` header)? Port 80 is the same as none.
pub fn same_origin(origin: &str, host: &str) -> bool {
    let o = origin.trim();
    let Some(rest) = o
        .strip_prefix("http://")
        .or_else(|| o.strip_prefix("https://"))
    else {
        return false;
    };
    !rest.is_empty() && host_part(rest) == host_part(host)
}

/// Is this `Content-Type` JSON (`application/json`, any parameters)?
pub fn is_json(content_type: &str) -> bool {
    let t = content_type.trim().to_ascii_lowercase();
    t == "application/json" || t.starts_with("application/json;")
}

/// Compare without stopping at the first difference.
fn same_secret(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Does the request carry `key`, as `Authorization: Bearer <key>` or as
/// the `key` query parameter (already percent-decoded)?
pub fn key_ok(key: &str, authorization: Option<&str>, query_key: Option<&str>) -> bool {
    if let Some(auth) = authorization {
        let auth = auth.trim();
        if auth.len() > 7
            && auth[..7].eq_ignore_ascii_case("bearer ")
            && same_secret(auth[7..].trim().as_bytes(), key.as_bytes())
        {
            return true;
        }
    }
    query_key.is_some_and(|q| same_secret(q.as_bytes(), key.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys() {
        assert!(valid_key("abcdEFGH1234-_.~"));
        assert!(!valid_key("short"));
        assert!(!valid_key(&"a".repeat(65)));
        assert!(valid_key(&"a".repeat(64)));
        assert!(!valid_key("has space in it"));
        assert!(!valid_key("quote\"inside"));
    }

    #[test]
    fn paths() {
        for p in [
            "/api/status",
            "/api/firmware",
            "/gifs/A/b.gif",
            "/cache/A/0f/b.gif",
            "/sets/x.txt",
            "/filelist.txt",
            "/filelist.hall",
            "/formulas/plasma.psi",
        ] {
            assert!(needs_key(p), "{p}");
        }
        for p in [
            "/",
            "/index.html",
            "/js/app.js",
            "/www/index.html",
            "/setup",
            "/licenses.txt",
            "/pkg/x.wasm",
            "/generate_204",
        ] {
            assert!(!needs_key(p), "{p}");
        }
    }

    #[test]
    fn origins() {
        assert!(same_origin("http://opendmd.local", "opendmd.local"));
        assert!(same_origin("http://OpenDMD.local", "opendmd.local:80"));
        assert!(same_origin("http://192.168.1.69:80", "192.168.1.69"));
        assert!(same_origin("http://localhost:8080", "localhost:8080"));
        assert!(!same_origin("http://localhost:8081", "localhost:8080"));
        assert!(!same_origin("http://evil.example", "opendmd.local"));
        assert!(!same_origin("null", "opendmd.local"));
        assert!(!same_origin("", "opendmd.local"));
        assert!(!same_origin("http://", ""));
        assert!(!same_origin("file://", "opendmd.local"));
    }

    #[test]
    fn content_types() {
        assert!(is_json("application/json"));
        assert!(is_json("Application/JSON; charset=utf-8"));
        assert!(!is_json("text/plain"));
        assert!(!is_json("application/x-www-form-urlencoded"));
        assert!(!is_json("application/jsonx"));
    }

    #[test]
    fn key_checks() {
        let k = "s3cret-key-123456";
        assert!(key_ok(k, Some("Bearer s3cret-key-123456"), None));
        assert!(key_ok(k, Some("bearer  s3cret-key-123456 "), None));
        assert!(key_ok(k, None, Some(k)));
        assert!(key_ok(k, Some("Bearer wrong"), Some(k)));
        assert!(!key_ok(k, Some("Bearer wrong"), None));
        assert!(!key_ok(k, Some("Basic czNjcmV0"), None));
        assert!(!key_ok(k, None, None));
        assert!(!key_ok(k, None, Some("s3cret-key-12345")));
    }

    #[test]
    fn hosts() {
        let n = Names {
            hostname: "Den",
            mdns: "den-2",
            ip: "192.168.1.69",
            was: "",
        };
        for h in [
            "opendmd.local",
            "OpenDMD.local:80",
            "opendmd.local.",
            "opendmd",
            "den.local",
            "DEN.local:80",
            "den",
            "den-2.local",
            "192.168.1.69",
            "192.168.1.69:80",
            "localhost:8080",
            "127.0.0.1:8080",
            "[::1]:8080",
        ] {
            assert!(host_allowed(h, &n), "{h}");
        }
        for h in [
            "evil.example",
            "evil.example:80",
            "den.local.evil.example",
            "opendmd.local.evil.example",
            "den.evil",
            "den-3.local",
            "192.168.1.70",
            "192.168.1.6",
            "10.0.2.15",
            "",
            " ",
            "[::2]:80",
            "localhost.evil.example",
        ] {
            assert!(!host_allowed(h, &n), "{h}");
        }
        // No name, no mDNS yet, no address: only the fixed names.
        let none = Names::default();
        assert!(host_allowed("opendmd.local", &none));
        assert!(host_allowed("127.0.0.1:8080", &none));
        assert!(!host_allowed(".local", &none));
        assert!(!host_allowed("192.168.4.1", &none));
        let ap = Names {
            ip: "192.168.4.1",
            ..none
        };
        assert!(host_allowed("192.168.4.1", &ap));
        // Renamed: the old name follows until the restart.
        let renamed = Names {
            hostname: "kitchen",
            was: "den",
            ..none
        };
        assert!(host_allowed("den.local", &renamed));
        assert!(host_allowed("kitchen.local", &renamed));
        assert!(!host_allowed("hall.local", &renamed));
    }

    #[test]
    fn card_types() {
        assert_eq!(card_media_type("/gifs/A/b.GIF"), Some("image/gif"));
        assert_eq!(card_media_type("/cache/A/0f/b.gif"), Some("image/gif"));
        assert_eq!(card_media_type("/gifs/x.png"), Some("image/png"));
        for p in [
            "/gifs/x.html",
            "/gifs/x.htm",
            "/gifs/x.svg",
            "/gifs/x.js",
            "/sets/a.txt",
            "/filelist.txt",
            "/filelist.hall",
            "/formulas/a.psi",
            "/gifs/no-extension",
            "/gifs/dir.gif/x",
        ] {
            assert_eq!(card_media_type(p), None, "{p}");
        }
        assert!(is_card_path("/formulas/a.psi"));
        assert!(!is_card_path("/api/files"));
        assert!(!is_card_path("/index.html"));
    }

    #[test]
    fn setup_mode() {
        use Setup::*;
        for (m, p) in [
            ("GET", "/"),
            ("GET", "/setup"),
            ("GET", "/api/status"),
            ("GET", "/api/config"),
            ("PUT", "/api/config"),
            ("GET", "/api/wifi/scan"),
            ("POST", "/api/reboot"),
            ("GET", "/generate_204"),
            ("GET", "/hotspot-detect.html"),
        ] {
            assert_eq!(setup_route(m, p), Allow, "{m} {p}");
        }
        for (m, p) in [
            ("GET", "/index.html"),
            ("GET", "/js/app.js"),
            ("GET", "/www/index.html"),
            ("GET", "/favicon.ico"),
        ] {
            assert_eq!(setup_route(m, p), Portal, "{m} {p}");
        }
        for (m, p) in [
            ("PUT", "/api/www"),
            ("PUT", "/api/firmware"),
            ("PUT", "/api/files/gifs/x.gif"),
            ("DELETE", "/api/files/gifs"),
            ("GET", "/api/files"),
            ("GET", "/gifs/A/b.gif"),
            ("GET", "/formulas/a.psi"),
            ("GET", "/filelist.txt"),
            ("POST", "/api/skip"),
            ("PUT", "/api/manifest"),
            ("POST", "/api/follow/test"),
            ("GET", "/api/www"),
            ("POST", "/"),
            ("POST", "/api/card/format"),
        ] {
            assert_eq!(setup_route(m, p), Refuse, "{m} {p}");
        }
    }
}

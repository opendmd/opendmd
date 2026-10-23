//! Who may use the player's HTTP API: the pure part of the firmware's
//! request gate (`firmware/src/httpd.rs`), here so it is tested on the host.
//! `docs/api.md`, "Security", is the user-facing description.
//!
//! Three rules, none of which is a defence against someone on the same WiFi
//! (the device speaks plain HTTP):
//!
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
    path.starts_with("/api/")
        || path.starts_with("/gifs/")
        || path.starts_with("/cache/")
        || path.starts_with("/sets/")
        || path.starts_with("/filelist")
}

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
}

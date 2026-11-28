//! OpenDMD Connect protocol v1 (P1-PLAN 2; the reference is
//! `docs/connect-protocol.md`): versions, the panel's Connect id, the
//! signed command envelope, the panel's per-connection session (replay
//! counters and rates), the remote allow-list, and the text messages the
//! panel sends and reads.
//!
//! Both ends are here: the panel's checks ([`Session::check`]) and the
//! controller's encoding ([`command_body`], [`sign_command`],
//! [`attach_signature`]), so the tests, the shared vectors and later the
//! iPhone app's Rust core use one implementation. The core never reads the
//! clock (rates take `now`) and never draws random numbers (the caller
//! passes them in).

use super::codec::{ct_eq, hex, hmac_sha256, sha256, unbase32, unhex};
use super::json::{self, push_str, Value};
use super::keys::{clean_label, clean_text, key_id, usable_public_key, KeyId, KeyList};
use super::xfer::MAX_FILE;
use core::time::Duration;
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};

// --- Versions ----------------------------------------------------------------

/// The protocol version this build speaks.
pub const VERSION: u32 = 1;
/// The oldest version still accepted: the current one and the one before
/// it (P1-PLAN 2.2). With only version 1 in existence, 1.
pub const MIN_VERSION: u32 = if VERSION > 1 { VERSION - 1 } else { 1 };

/// Close codes (WebSocket application range).
pub const CLOSE_TOO_OLD: u16 = 4426;
/// A version newer than any this end knows: the reason is the newest it
/// takes. (Not in P1-PLAN 2.2, which names only "too old"; a server older
/// than a panel answers this.)
pub const CLOSE_TOO_NEW: u16 = 4406;
/// "Stay away": the reason is the minutes to wait.
pub const CLOSE_STAY_AWAY: u16 = 4429;
/// Refused identity (unknown panel, bad secret, unknown or bad controller).
pub const CLOSE_REFUSED: u16 = 4403;
/// The panel's answer to a frame that breaks the protocol (P1-PLAN 2.1's
/// record cap is enforced below this, by TLS).
pub const CLOSE_PROTOCOL: u16 = 1002;

/// `odc<N>`.
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn subprotocol(v: u32) -> String {
    format!("odc{v}")
}

/// The version an `odc<N>` subprotocol names: decimal, no sign, no leading
/// zero, 1 to 999.
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn parse_subprotocol(s: &str) -> Option<u32> {
    let d = s.strip_prefix("odc")?;
    if d.is_empty() || d.len() > 3 || d.starts_with('0') || !d.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    d.parse().ok()
}

/// The server's choice among the subprotocols a client offered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Negotiated {
    /// Answer `odc<N>` and speak version N.
    Accept(u32),
    /// Every version offered is below [`MIN_VERSION`]: close
    /// [`CLOSE_TOO_OLD`] with the reason `min`.
    TooOld { min: u32 },
    /// Every version offered is above [`VERSION`]: close [`CLOSE_TOO_NEW`]
    /// with the reason `max`.
    TooNew { max: u32 },
    /// No `odc<N>` at all: refuse the upgrade.
    NoVersion,
}

/// Pick the newest accepted version among `offered` (the
/// `Sec-WebSocket-Protocol` values, in any order).
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn negotiate<'a>(offered: impl IntoIterator<Item = &'a str>) -> Negotiated {
    let (mut best, mut low, mut high) = (None::<u32>, false, false);
    for v in offered.into_iter().filter_map(|s| parse_subprotocol(s.trim())) {
        if v < MIN_VERSION {
            low = true;
        } else if v > VERSION {
            high = true;
        } else if best.is_none_or(|b| v > b) {
            best = Some(v);
        }
    }
    match best {
        Some(v) => Negotiated::Accept(v),
        None if low => Negotiated::TooOld { min: MIN_VERSION },
        None if high => Negotiated::TooNew { max: VERSION },
        None => Negotiated::NoVersion,
    }
}

// --- The panel's Connect id ------------------------------------------------------

const DEV_TAG_DOMAIN: &[u8] = b"opendmd-connect-v1 dev\0";

/// A panel's Connect id: 10 random bytes and a 6-byte tag under the
/// server's `ID_KEY`, so the server refuses a made-up id before any object
/// wakes. Written as 26 base32 characters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Dev(pub [u8; 16]);

impl Dev {
    /// The server's mint: `random` from its RNG.
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn mint(random: &[u8; 10], id_key: &[u8]) -> Dev {
        let tag = hmac_sha256(id_key, &[DEV_TAG_DOMAIN, random]);
        let mut d = [0u8; 16];
        d[..10].copy_from_slice(random);
        d[10..].copy_from_slice(&tag[..6]);
        Dev(d)
    }
    /// The server's check of an id it is shown.
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn tag_ok(&self, id_key: &[u8]) -> bool {
        let tag = hmac_sha256(id_key, &[DEV_TAG_DOMAIN, &self.0[..10]]);
        ct_eq(&tag[..6], &self.0[10..])
    }
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn text(&self) -> String {
        super::codec::base32(&self.0)
    }
    /// Canonical lowercase base32 only.
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn parse(s: &str) -> Option<Dev> {
        unbase32::<16>(s).map(Dev)
    }
    /// The `d.<dev>` subprotocol that carries it (never a URL).
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn subprotocol(&self) -> String {
        format!("d.{}", self.text())
    }
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn from_subprotocol(s: &str) -> Option<Dev> {
        Dev::parse(s.strip_prefix("d.")?)
    }
}

// --- The signed command envelope -----------------------------------------------

/// A binary signed-command frame: `0x01` ‖ key id (8) ‖ signature (64) ‖
/// body.
pub const TYPE_COMMAND: u8 = 0x01;
pub const COMMAND_HEAD: usize = 1 + 8 + 64;
/// A command body's longest, in bytes.
pub const MAX_BODY: usize = 700;
/// The longest text frame anyone reads that does not come from the panel
/// (server to panel, controller to server): the panel's receive buffer.
pub const MAX_TEXT_IN: usize = 1024;
/// The longest text frame the panel sends (an `ack` with a full
/// `controllers` result, `status`): what the server and controllers must
/// take from it.
pub const MAX_TEXT_OUT: usize = 2048;
/// What a command signature covers before the body.
pub const CMD_DOMAIN: &[u8] = b"opendmd-connect-v1 cmd\0";
/// What an attach signature covers before `dev` and the nonce.
pub const ATTACH_DOMAIN: &[u8] = b"opendmd-connect-v1 attach\0";
/// The largest `n`: JavaScript's exact integers.
pub const MAX_N: u64 = json::MAX_SAFE as u64;

/// The parts of a command frame, unchecked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Frame<'a> {
    pub k: KeyId,
    pub sig: [u8; 64],
    pub body: &'a [u8],
}

/// Split a command frame. `Err(None)`: not a command frame at all (no key
/// id to answer); `Err(Some(k))`: one whose body is over [`MAX_BODY`] (an empty body
/// goes on, to fail its signature or its parse).
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn split_command(frame: &[u8]) -> Result<Frame<'_>, Option<KeyId>> {
    if frame.len() < COMMAND_HEAD || frame[0] != TYPE_COMMAND {
        return Err(None);
    }
    let mut k = [0u8; 8];
    k.copy_from_slice(&frame[1..9]);
    let body = &frame[COMMAND_HEAD..];
    if body.len() > MAX_BODY {
        return Err(Some(k));
    }
    let mut sig = [0u8; 64];
    sig.copy_from_slice(&frame[9..COMMAND_HEAD]);
    Ok(Frame { k, sig, body })
}

/// Does `sig` verify over the domain and `body` for `public`? Strict
/// verification (no small-order keys, canonical `S`).
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn verify_body(public: &[u8; 32], body: &[u8], sig: &[u8; 64]) -> bool {
    let Ok(key) = VerifyingKey::from_bytes(public) else {
        return false;
    };
    let mut msg = Vec::with_capacity(CMD_DOMAIN.len() + body.len());
    msg.extend_from_slice(CMD_DOMAIN);
    msg.extend_from_slice(body);
    key.verify_strict(&msg, &Signature::from_bytes(sig)).is_ok()
}

/// `formula-add`: its argument rules, then the gallery's signature
/// (`fadd::admit_where_it_runs`: on the firmware, a task of its own, after
/// the command's own check has returned). In a frame of its own, so the
/// check every command goes through (`Session::check`) is no deeper for it.
#[cfg(feature = "connect-formula-add")]
#[inline(never)]
#[cfg_attr(target_arch = "xtensa", optimize(size))]
fn formula_add_command(a: &Value) -> Result<Command, Refusal> {
    let (args, claim) = super::fadd::add_args(a)?;
    let signed = super::fadd::admit_where_it_runs(&args, &claim, &crate::gallery_sig::Trust::panel(crate::psi::psb::ABI))?;
    Ok(Command::FormulaAdd(Box::new(super::fadd::Admitted { args, signed })))
}

/// [`formula_add_command`] with the panel's adds ([`Session::check_adds`]):
/// their verify rates, and the held add's witness taken again when its own
/// key sends it again.
#[cfg(feature = "connect-formula-add")]
#[inline(never)]
#[cfg_attr(target_arch = "xtensa", optimize(size))]
fn formula_add_with(k: KeyId, a: &Value, now: Duration, adds: &mut super::fadd::Adds) -> Result<Command, Refusal> {
    let (args, claim) = super::fadd::add_args(a)?;
    let signed = adds.admit(k, &args, &claim, now, &crate::gallery_sig::Trust::panel(crate::psi::psb::ABI))?;
    Ok(Command::FormulaAdd(Box::new(super::fadd::Admitted { args, signed })))
}

/// What [`Session::check`]'s inner half takes for the panel's adds.
#[cfg(feature = "connect-formula-add")]
type AddsArg<'a> = Option<&'a mut super::fadd::Adds>;
#[cfg(not(feature = "connect-formula-add"))]
type AddsArg<'a> = core::marker::PhantomData<&'a ()>;

/// The server's pre-check (P1-PLAN 2.5): a known key and a good signature,
/// before the frame goes to the panel. `keys`: (id, public key) pairs.
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn precheck(keys: &[(KeyId, [u8; 32])], frame: &[u8]) -> Result<KeyId, Refusal> {
    let f = match split_command(frame) {
        Ok(f) => f,
        Err(None) => return Err(Refusal::BadFrame),
        Err(Some(_)) => return Err(Refusal::TooBig),
    };
    let public = keys.iter().find(|(k, _)| *k == f.k).map(|(_, p)| p).ok_or(Refusal::UnknownKey)?;
    if !verify_body(public, f.body, &f.sig) {
        return Err(Refusal::BadSig);
    }
    Ok(f.k)
}

/// A controller's command body: `{"v","dev","sess","n","c","a"}`. `args`:
/// the members of `a` already as JSON (`""` for none), e.g.
/// `"path":"/gifs/x.gif"`.
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn command_body(dev: &Dev, sess: &[u8; 12], n: u64, c: &str, args: &str) -> String {
    let mut s = format!(r#"{{"v":{VERSION},"dev":"{}","sess":"{}","n":{n},"c":"#, dev.text(), hex(sess));
    push_str(&mut s, c);
    s.push_str(r#","a":{"#);
    s.push_str(args);
    s.push_str("}}");
    s
}

/// A controller's signed command frame for `body` (its bytes as sent).
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn sign_command(key: &SigningKey, body: &[u8]) -> Vec<u8> {
    let mut msg = Vec::with_capacity(CMD_DOMAIN.len() + body.len());
    msg.extend_from_slice(CMD_DOMAIN);
    msg.extend_from_slice(body);
    let sig = key.sign(&msg).to_bytes();
    let mut f = Vec::with_capacity(COMMAND_HEAD + body.len());
    f.push(TYPE_COMMAND);
    f.extend_from_slice(&key_id(&key.verifying_key().to_bytes()));
    f.extend_from_slice(&sig);
    f.extend_from_slice(body);
    f
}

#[cfg_attr(target_arch = "xtensa", optimize(size))]
fn attach_message(dev: &Dev, nonce: &[u8; 16]) -> Vec<u8> {
    [ATTACH_DOMAIN, &dev.0, nonce].concat()
}

/// A controller's answer to the server's `challenge`: the signature over
/// the domain ‖ `dev` ‖ nonce.
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn attach_signature(key: &SigningKey, dev: &Dev, nonce: &[u8; 16]) -> [u8; 64] {
    key.sign(&attach_message(dev, nonce)).to_bytes()
}

/// The server's check of an `attach`.
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn verify_attach(public: &[u8; 32], dev: &Dev, nonce: &[u8; 16], sig: &[u8; 64]) -> bool {
    VerifyingKey::from_bytes(public).is_ok_and(|k| k.verify_strict(&attach_message(dev, nonce), &Signature::from_bytes(sig)).is_ok())
}

// --- Refusals ------------------------------------------------------------------

/// Why a command was refused: the `err` of its `ack` (P1-PLAN 2.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    BadSig,
    UnknownKey,
    StaleSession,
    Replay,
    NotAllowed,
    BadArg,
    Busy,
    TooBig,
    Quota,
    NoCard,
    Rate,
    /// Not a command frame at all: dropped, no `ack` (there is no key id
    /// to answer).
    BadFrame,
}

impl Refusal {
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn code(self) -> &'static str {
        match self {
            Refusal::BadSig => "bad-sig",
            Refusal::UnknownKey => "unknown-key",
            Refusal::StaleSession => "stale-session",
            Refusal::Replay => "replay",
            Refusal::NotAllowed => "not-allowed",
            Refusal::BadArg => "bad-arg",
            Refusal::Busy => "busy",
            Refusal::TooBig => "too-big",
            Refusal::Quota => "quota",
            Refusal::NoCard => "no-card",
            Refusal::Rate => "rate",
            Refusal::BadFrame => "bad-frame",
        }
    }
}

// --- The allow-list ------------------------------------------------------------

/// Every `c` a remote controller may send (P1-PLAN 2.5). Anything else is
/// `not-allowed`; there is no [`Command`] for anything else either.
pub const ALLOWED: &[&str] = &[
    "play",
    "next",
    "prev",
    "pause",
    "brightness",
    "playlist",
    "playlists",
    "formulas",
    #[cfg(feature = "connect-formula-add")]
    "formula-add",
    #[cfg(feature = "connect-formula-add")]
    "formula-part",
    "send",
    "send-cancel",
    "controllers",
    "add-controller",
    "remove-controller",
    "name",
];

/// Words no remote command may ever contain: the never-remote column of
/// CONNECT.md 4.3 (update, delete, WiFi, reboot and their kin). Checked
/// against [`ALLOWED`] at compile time.
pub const NEVER: &[&str] = &[
    "update", "firmware", "ota", "www", "delete", "format", "rename", "wifi", "ssid", "hostname", "key-",
    "api", "token", "replay", "reboot", "restart", "reset", "timing", "network", "config", "file", "read", "upload",
];

const fn contains(hay: &[u8], needle: &[u8]) -> bool {
    if needle.len() > hay.len() {
        return false;
    }
    let mut i = 0;
    while i + needle.len() <= hay.len() {
        let mut j = 0;
        while j < needle.len() && hay[i + j] == needle[j] {
            j += 1;
        }
        if j == needle.len() {
            return true;
        }
        i += 1;
    }
    false
}

const fn allow_list_is_clean() -> bool {
    let mut a = 0;
    while a < ALLOWED.len() {
        let mut n = 0;
        while n < NEVER.len() {
            if contains(ALLOWED[a].as_bytes(), NEVER[n].as_bytes()) {
                return false;
            }
            n += 1;
        }
        a += 1;
    }
    true
}

const _: () = assert!(allow_list_is_clean(), "a remote command names something that must never be remote");

/// `send`'s arguments, validated (P1-PLAN 2.8).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendArgs {
    /// The transfer id (non-zero), as in the chunk frames.
    pub id: u32,
    /// A file name only, for `/gifs/Sent/`.
    pub name: String,
    pub size: u32,
    pub sha256: [u8; 32],
    /// 0: no prepared copy.
    pub copy_size: u32,
    pub copy_sha256: Option<[u8; 32]>,
    pub play: bool,
}

/// A remote command, validated. The firmware maps each to what the LAN
/// handlers send (F2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Play { path: String },
    Next,
    Prev,
    Pause,
    Brightness { v: u8 },
    Playlist { set: String },
    Playlists,
    /// `formulas`; `after`: the page after this stem (5.6), `None` the first.
    Formulas { after: Option<String> },
    /// `formula-add` (5.7): a Formula's source and compiled file to come,
    /// and the gallery's signature over them (the witness only
    /// `gallery_sig::check` makes).
    #[cfg(feature = "connect-formula-add")]
    FormulaAdd(Box<super::fadd::Admitted>),
    /// `formula-part`: `data` of the add `id` at stream offset `off`.
    #[cfg(feature = "connect-formula-add")]
    FormulaPart { id: u32, off: u32, data: Vec<u8> },
    Send(SendArgs),
    SendCancel { id: u32 },
    Controllers,
    AddController { public: [u8; 32], label: String },
    RemoveController { k: KeyId },
    Name { name: String },
}

/// What the panel knows that the checks need.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// The brightness ceiling (`max_brightness`).
    pub max_brightness: u8,
}

/// A panel name's longest: 32 characters (and 128 UTF-8 bytes).
pub const MAX_NAME_CHARS: usize = 32;

/// A Formula's stem in a remote play: `[a-z0-9_-]`, 1 to 64, starting with
/// a letter or digit (so never `~sent`).
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn formula_stem_ok(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty()
        && b.len() <= 64
        && b[0].is_ascii_alphanumeric()
        && b.iter().all(|&c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-' || c == b'_')
}

/// A path segment a remote path may hold: not empty, not `.`/`..` or any
/// hidden name, none ending in `.` or a space (FAT drops those, so two
/// spellings would name one file), no control or path-meaningful character.
#[cfg_attr(target_arch = "xtensa", optimize(size))]
fn segment_ok(s: &str) -> bool {
    !s.is_empty() && !s.starts_with('.') && !s.ends_with(['.', ' ']) && !s.chars().any(|c| c.is_control() || matches!(c, '\\' | '?' | '#' | ':' | '*' | '<' | '>' | '|' | '"'))
}

/// The longest remote play path, in bytes.
pub const MAX_PATH: usize = 240;

/// A path a remote `play` may name: a GIF under `/gifs/` (never `/cache/`,
/// never `..`), `@formula:<stem>` or `/formulas/<stem>.psi` (never an
/// inline Formula, never `~sent`). The firmware's own play validation runs
/// as well (F2).
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn remote_play_path_ok(path: &str) -> bool {
    if let Some(stem) = path.strip_prefix("@formula:") {
        return formula_stem_ok(stem);
    }
    if let Some(stem) = path.strip_prefix("/formulas/").and_then(|p| p.strip_suffix(".psi")) {
        return formula_stem_ok(stem);
    }
    let Some(rest) = path.strip_prefix("/gifs/") else {
        return false;
    };
    path.len() <= MAX_PATH
        && rest.len() > 4
        && rest.get(rest.len() - 4..).is_some_and(|e| e.eq_ignore_ascii_case(".gif"))
        && rest.split('/').all(segment_ok)
}

/// A playlist a remote `playlist` may choose: `""` (the main list) or one
/// name, as the LAN's rule (`valid_set_name`) and stricter: ≤ 64 bytes, no
/// control character, not hidden.
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn remote_set_ok(set: &str) -> bool {
    set.is_empty() || (set.len() <= 64 && segment_ok(set) && !set.contains(['/', '\0']) && !set.contains(".."))
}

/// A sent name's longest, in bytes: `/gifs/Sent/<name>` fits a remote
/// play's [`MAX_PATH`] (240).
pub const MAX_SENT_NAME_BYTES: usize = MAX_PATH - "/gifs/Sent/".len();

/// A sent GIF's file name (the web app's rule): no slash, no control
/// characters, `.gif`, ≤ 120 characters; and not hidden.
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn sent_name_ok(name: &str) -> bool {
    // ≤ 229 bytes, and room in FAT's 255 UTF-16 units for the panel's
    // `.<id>.copy.part` (19 more).
    name.len() <= MAX_SENT_NAME_BYTES
        && name.encode_utf16().count() + 19 <= 255
        && name.chars().count() <= 120
        && name.len() > 4
        && name.get(name.len() - 4..).is_some_and(|e| e.eq_ignore_ascii_case(".gif"))
        && segment_ok(name)
        && !name.contains('/')
}

#[cfg_attr(target_arch = "xtensa", optimize(size))]
fn arg<'a>(a: &'a Value, key: &str) -> Option<&'a Value> {
    a.get(key)
}

#[cfg_attr(target_arch = "xtensa", optimize(size))]
fn u32_arg(a: &Value, key: &str) -> Result<u32, Refusal> {
    arg(a, key).and_then(Value::as_int).and_then(|n| u32::try_from(n).ok()).ok_or(Refusal::BadArg)
}

#[cfg_attr(target_arch = "xtensa", optimize(size))]
fn str_arg<'a>(a: &'a Value, key: &str) -> Result<&'a str, Refusal> {
    arg(a, key).and_then(Value::as_str).ok_or(Refusal::BadArg)
}

#[cfg_attr(target_arch = "xtensa", optimize(size))]
fn send_args(a: &Value) -> Result<SendArgs, Refusal> {
    let id = u32_arg(a, "id")?;
    let name = str_arg(a, "name")?;
    let size = arg(a, "size").and_then(Value::as_int).ok_or(Refusal::BadArg)?;
    let copy_size = match arg(a, "copy_size") {
        None => 0,
        Some(v) => v.as_int().ok_or(Refusal::BadArg)?,
    };
    if size <= 0 || copy_size < 0 {
        return Err(Refusal::BadArg);
    }
    if size > i64::from(MAX_FILE) || copy_size > i64::from(MAX_FILE) {
        return Err(Refusal::TooBig);
    }
    if id == 0 || !sent_name_ok(name) {
        return Err(Refusal::BadArg);
    }
    let sha256 = unhex::<32>(str_arg(a, "sha256")?).ok_or(Refusal::BadArg)?;
    let copy_sha256 = match (copy_size, arg(a, "copy_sha256")) {
        (0, None) => None,
        (0, Some(_)) => return Err(Refusal::BadArg),
        (_, Some(v)) => Some(v.as_str().and_then(unhex::<32>).ok_or(Refusal::BadArg)?),
        (_, None) => return Err(Refusal::BadArg),
    };
    let play = match arg(a, "play") {
        None => false,
        Some(v) => v.as_bool().ok_or(Refusal::BadArg)?,
    };
    Ok(SendArgs { id, name: name.to_string(), size: size as u32, sha256, copy_size: copy_size as u32, copy_sha256, play })
}

/// Look `c` up in the allow-list and validate its arguments `a`. `can_add`:
/// the sender's key has the flag. `keys`: the panel's list (a removal must
/// name a key on it).
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn parse_command(c: &str, a: &Value, limits: &Limits, can_add: bool, keys: &KeyList) -> Result<Command, Refusal> {
    Ok(match c {
        "play" => {
            let path = str_arg(a, "path")?;
            if !remote_play_path_ok(path) {
                return Err(Refusal::BadArg);
            }
            Command::Play { path: path.to_string() }
        }
        "next" => Command::Next,
        "prev" => Command::Prev,
        "pause" => Command::Pause,
        "brightness" => {
            let v = arg(a, "v").and_then(Value::as_int).ok_or(Refusal::BadArg)?;
            if !(0..=i64::from(limits.max_brightness)).contains(&v) {
                return Err(Refusal::BadArg);
            }
            Command::Brightness { v: v as u8 }
        }
        "playlist" => {
            let set = str_arg(a, "set")?;
            if !remote_set_ok(set) {
                return Err(Refusal::BadArg);
            }
            Command::Playlist { set: set.to_string() }
        }
        "playlists" => Command::Playlists,
        "formulas" => {
            // Strict, so a later page can tell a panel without a feature:
            // only `after`, a stem.
            let Value::Object(m) = a else { return Err(Refusal::BadArg) };
            if m.iter().any(|(k, _)| k != "after") {
                return Err(Refusal::BadArg);
            }
            let after = match a.get("after") {
                None => None,
                Some(v) => Some(v.as_str().filter(|s| formula_stem_ok(s)).ok_or(Refusal::BadArg)?.to_string()),
            };
            Command::Formulas { after }
        }
        // An add needs the gallery's signature, checked here, right after
        // its argument rules and before anything else (the rate, the day's
        // count, the card): `fadd::admit` (`gallery_sig::check` over the
        // argument stem, sizes and hashes), whose witness
        // `Command::FormulaAdd` takes. Any refusal is `not-allowed`. (This
        // runs after the command's own signature check has returned, never
        // inside it. A part with no add held is the firmware's
        // `not-allowed`, before the card.)
        #[cfg(feature = "connect-formula-add")]
        "formula-add" => formula_add_command(a)?,
        #[cfg(feature = "connect-formula-add")]
        "formula-part" => {
            let (id, off, data) = super::fadd::part_args(a)?;
            Command::FormulaPart { id, off, data }
        }
        "send" => Command::Send(send_args(a)?),
        "send-cancel" => Command::SendCancel { id: u32_arg(a, "id")? },
        "controllers" => Command::Controllers,
        "add-controller" => {
            if !can_add {
                return Err(Refusal::NotAllowed);
            }
            let public = unhex::<32>(str_arg(a, "pub")?).ok_or(Refusal::BadArg)?;
            if !usable_public_key(&public) || keys.find(&key_id(&public)).is_some() {
                return Err(Refusal::BadArg);
            }
            let label = clean_label(str_arg(a, "label")?).ok_or(Refusal::BadArg)?;
            if keys.len() >= super::keys::MAX_KEYS {
                return Err(Refusal::Quota);
            }
            Command::AddController { public, label }
        }
        "remove-controller" => {
            let k = unhex::<8>(str_arg(a, "k")?).ok_or(Refusal::BadArg)?;
            if keys.find(&k).is_none() {
                return Err(Refusal::BadArg);
            }
            Command::RemoveController { k }
        }
        "name" => Command::Name { name: clean_text(str_arg(a, "name")?, 4 * MAX_NAME_CHARS, MAX_NAME_CHARS).ok_or(Refusal::BadArg)? },
        _ => return Err(Refusal::NotAllowed),
    })
}

// --- Rates ---------------------------------------------------------------------

/// A token bucket: `per_s` a second, a burst of the same.
#[derive(Debug, Clone, Copy)]
pub struct Bucket {
    per_s: u32,
    /// Tokens in millionths.
    micro: u64,
    last: Duration,
}

impl Bucket {
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn new(per_s: u32, now: Duration) -> Bucket {
        Bucket { per_s, micro: u64::from(per_s) * 1_000_000, last: now }
    }
    /// Take one token if there is one.
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn take(&mut self, now: Duration) -> bool {
        let cap = u64::from(self.per_s) * 1_000_000;
        let dt = now.saturating_sub(self.last).as_micros().min(u128::from(u64::MAX)) as u64;
        self.micro = self.micro.saturating_add(dt.saturating_mul(u64::from(self.per_s))).min(cap);
        self.last = self.last.max(now);
        if self.micro >= 1_000_000 {
            self.micro -= 1_000_000;
            true
        } else {
            false
        }
    }
}

/// Commands a second from one controller, and to one panel.
pub const RATE_PER_KEY: u32 = 4;
pub const RATE_PER_PANEL: u32 = 8;

/// Keys a session keeps counters for (removed ones included, so a key
/// removed and added again in one session cannot have its old commands
/// replayed). Past this the session answers `busy` and the caller
/// reconnects, which voids everything.
pub const MAX_COUNTERS: usize = 16;

// --- The panel's session -----------------------------------------------------------

/// A command the panel accepts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Accepted {
    pub k: KeyId,
    pub n: u64,
    pub cmd: Command,
}

/// A command the panel refuses; `k`/`n` for the `ack` when known.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Refused {
    pub k: Option<KeyId>,
    pub n: Option<u64>,
    pub err: Refusal,
}

impl Refused {
    /// The `ack` to send, or `None` (no key id: drop the frame).
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn ack(&self) -> Option<String> {
        self.k.map(|k| ack_err(&k, self.n, self.err))
    }
}

/// One connection's state on the panel: its `sess` nonce (new at every
/// connect), the last `n` of each key, the rates. Nothing of it outlives
/// the connection.
#[derive(Debug, Clone)]
pub struct Session {
    pub dev: Dev,
    pub sess: [u8; 12],
    pub version: u32,
    counters: Vec<(KeyId, u64, Bucket)>,
    panel_rate: Bucket,
}

impl Session {
    /// A new connection; `sess`: 12 fresh random bytes.
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn new(dev: Dev, sess: [u8; 12], version: u32, now: Duration) -> Session {
        Session { dev, sess, version, counters: Vec::new(), panel_rate: Bucket::new(RATE_PER_PANEL, now) }
    }

    /// The last `n` accepted from `k` this session.
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn last_n(&self, k: &KeyId) -> Option<u64> {
        self.counters.iter().find(|(x, _, _)| x == k).map(|&(_, n, _)| n)
    }

    /// Check one binary command frame, in this order: the frame, a known
    /// key, the panel's rate (before the 30 ms signature check), the
    /// signature over the body's bytes, the body, `v`, `dev`, `sess`, `n`
    /// above the key's last (which then advances), the key's rate, the
    /// allow-list and the arguments.
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn check(&mut self, keys: &KeyList, limits: &Limits, frame: &[u8], now: Duration) -> Result<Accepted, Refused> {
        #[cfg(feature = "connect-formula-add")]
        let adds = None;
        #[cfg(not(feature = "connect-formula-add"))]
        let adds = core::marker::PhantomData;
        self.check_in(keys, limits, frame, now, adds)
    }

    /// [`check`](Self::check), with the panel's adds for a `formula-add`
    /// (the firmware's: its verify rates, the held add).
    #[cfg(feature = "connect-formula-add")]
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn check_adds(&mut self, keys: &KeyList, limits: &Limits, frame: &[u8], now: Duration, adds: &mut super::fadd::Adds) -> Result<Accepted, Refused> {
        self.check_in(keys, limits, frame, now, Some(adds))
    }

    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    fn check_in(&mut self, keys: &KeyList, limits: &Limits, frame: &[u8], now: Duration, adds: AddsArg) -> Result<Accepted, Refused> {
        let no = |k: Option<KeyId>, n: Option<u64>, err| Refused { k, n, err };
        let f = match split_command(frame) {
            Ok(f) => f,
            Err(None) => return Err(no(None, None, Refusal::BadFrame)),
            Err(Some(k)) => return Err(no(Some(k), None, Refusal::TooBig)),
        };
        let k = f.k;
        let Some(entry) = keys.find(&k) else {
            return Err(no(Some(k), None, Refusal::UnknownKey));
        };
        if !self.panel_rate.take(now) {
            return Err(no(Some(k), None, Refusal::Rate));
        }
        if !verify_body(&entry.public, f.body, &f.sig) {
            return Err(no(Some(k), None, Refusal::BadSig));
        }
        let body = json::parse(f.body).map_err(|_| no(Some(k), None, Refusal::BadArg))?;
        let n = body.get("n").and_then(Value::as_int).filter(|&n| n >= 1).map(|n| n as u64);
        let bad = |err| no(Some(k), n, err);
        let n = n.ok_or(bad(Refusal::BadArg))?;
        if body.get("v").and_then(Value::as_int) != Some(i64::from(self.version)) {
            return Err(bad(Refusal::BadArg));
        }
        if body.get("dev").and_then(Value::as_str).and_then(Dev::parse) != Some(self.dev) {
            return Err(bad(Refusal::StaleSession));
        }
        if body.get("sess").and_then(Value::as_str).and_then(unhex::<12>) != Some(self.sess) {
            return Err(bad(Refusal::StaleSession));
        }
        let c = body.get("c").and_then(Value::as_str).ok_or(bad(Refusal::BadArg))?;
        let empty = Value::Object(Vec::new());
        let a = match body.get("a") {
            None => &empty,
            Some(v) if v.is_object() => v,
            Some(_) => return Err(bad(Refusal::BadArg)),
        };
        // The counter, then the key's rate.
        let slot = match self.counters.iter().position(|(x, _, _)| *x == k) {
            Some(i) => i,
            None if self.counters.len() >= MAX_COUNTERS => return Err(bad(Refusal::Busy)),
            None => {
                self.counters.push((k, 0, Bucket::new(RATE_PER_KEY, now)));
                self.counters.len() - 1
            }
        };
        let (_, last, rate) = &mut self.counters[slot];
        if n <= *last {
            return Err(bad(Refusal::Replay));
        }
        *last = n;
        if !rate.take(now) {
            return Err(bad(Refusal::Rate));
        }
        #[cfg(feature = "connect-formula-add")]
        if let (Some(adds), "formula-add") = (adds, c) {
            let cmd = formula_add_with(k, a, now, adds).map_err(bad)?;
            return Ok(Accepted { k, n, cmd });
        }
        #[cfg(not(feature = "connect-formula-add"))]
        let _ = adds;
        let cmd = parse_command(c, a, limits, entry.can_add(), keys).map_err(bad)?;
        Ok(Accepted { k, n, cmd })
    }
}

// --- Text messages -------------------------------------------------------------

/// What an `ack` adds around its result `r` (`t`, a key id, the largest
/// `n`, `ok`): under this, so a result of [`RESULT_ROOM`] bytes keeps the
/// `ack` inside [`MAX_TEXT_OUT`].
pub const ACK_OVERHEAD: usize = 100;
/// The most a bounded result (`playlists`, `formulas`) may take.
pub const RESULT_ROOM: usize = MAX_TEXT_OUT - ACK_OVERHEAD;

/// `formulas`'s result, built a Formula at a time so the caller can stop
/// reading the card once it is full (protocol 5.6):
/// `{"builtin":[[stem,name],…],"card":[[stem,name],…]|null,"more":bool}`,
/// and `"next":"<stem>"` with `more` when the caller knows where the next
/// page starts (its `after`). Built-in ones first (the first page only),
/// then the card's (`null`: no card the panel can read), each a
/// `[stem, display name]` pair, while the whole stays within
/// [`RESULT_ROOM`] whatever the closing turns out to be; the first that
/// does not fit, and every one after it, is left out and `more` is true
/// (as it is when the caller stopped early: [`FormulaList::finish`]).
#[derive(Debug)]
pub struct FormulaList {
    s: String,
    /// A card list follows (else `"card":null`).
    has_card: bool,
    /// Its `[` is written.
    card_open: bool,
    more: bool,
}

/// What may still close a list, at most: with a card, `],"card":[` if not
/// yet open, then `]` and `,"more":true,"next":"<a 64-byte stem>"}`;
/// without one, `],"card":null,"more":false}`.
const CARD_OPEN: &str = r#"],"card":["#;
const CLOSE_CARD_MAX: usize = "]".len() + r#","more":true,"next":""}"#.len() + 64;
const CLOSE_NULL: &str = r#"],"card":null,"more":false}"#;

impl FormulaList {
    /// `card`: the panel has a card it can read (a card list follows).
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn new(card: bool) -> FormulaList {
        FormulaList { s: String::from(r#"{"builtin":["#), has_card: card, card_open: false, more: false }
    }

    /// [`new`](Self::new), saying first what an add needs (5.7): the
    /// compiled format this player takes (`abi`) and the gallery keys it
    /// trusts (`gk`, their ids): a controller sends a signature by one of
    /// them, for that format.
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn with_add(card: bool, abi: u8, keys: &[u8]) -> FormulaList {
        let ids: Vec<String> = keys.iter().map(u8::to_string).collect();
        FormulaList { s: format!(r#"{{"abi":{abi},"gk":[{}],"builtin":["#, ids.join(",")), has_card: card, card_open: false, more: false }
    }

    /// Anything left out yet?
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn full(&self) -> bool {
        self.more
    }

    /// The most the closing may add now.
    fn closing(&self) -> usize {
        match (self.has_card, self.card_open) {
            (false, _) => CLOSE_NULL.len(),
            (true, false) => CARD_OPEN.len() + CLOSE_CARD_MAX,
            (true, true) => CLOSE_CARD_MAX,
        }
    }

    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    fn push_into(&mut self, card: bool, stem: &str, name: &str) -> bool {
        if self.more || (card && !self.has_card) {
            self.more |= card && !self.has_card;
            return false;
        }
        let opening = if card && !self.card_open { CARD_OPEN } else { "" };
        let mut e = String::with_capacity(opening.len() + stem.len() + name.len() + 8);
        e.push_str(opening);
        if !self.s.ends_with('[') && opening.is_empty() {
            e.push(',');
        }
        e.push('[');
        push_str(&mut e, stem);
        e.push(',');
        push_str(&mut e, name);
        e.push(']');
        let after = if card || self.card_open { CLOSE_CARD_MAX } else { self.closing() };
        if self.s.len() + e.len() + after > RESULT_ROOM {
            self.more = true;
            return false;
        }
        self.s.push_str(&e);
        self.card_open |= card;
        true
    }

    /// A built-in one; false (and `more`) if it did not fit, or an earlier
    /// one did not: the list never skips one to fit a later, shorter one.
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn builtin(&mut self, stem: &str, name: &str) -> bool {
        if self.card_open {
            return false;
        }
        self.push_into(false, stem, name)
    }

    /// One of the card's, as [`builtin`](Self::builtin).
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn card(&mut self, stem: &str, name: &str) -> bool {
        self.push_into(true, stem, name)
    }

    /// The result. `cut`: the caller stopped before the card's end (its
    /// read cap, the card busy): `more` then too. `next`: with `more`, the
    /// `after` for the next page (a stem: nothing to escape; one that is not
    /// is left out).
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn finish(mut self, cut: bool, next: Option<&str>) -> String {
        let more = self.has_card && (self.more || cut);
        if !self.has_card {
            self.s.push_str(CLOSE_NULL);
            return self.s;
        }
        if !self.card_open {
            self.s.push_str(CARD_OPEN);
        }
        self.s.push(']');
        match next.filter(|n| more && formula_stem_ok(n)) {
            Some(n) => {
                self.s.push_str(r#","more":true,"next":""#);
                self.s.push_str(n);
                self.s.push_str("\"}");
            }
            None if more => self.s.push_str(r#","more":true}"#),
            None => self.s.push_str(r#","more":false}"#),
        }
        self.s
    }
}

/// `formulas`'s own rate, per panel (5.6): a listing's first page (no
/// `after`, or an `after` that is not the last page's `next`) at most one
/// every [`FORMULAS_EVERY`], answered `rate` otherwise; the pages that
/// follow it (an `after` equal to the `next` the panel last gave, within
/// [`FORMULAS_PAGE_WAIT`]) free, up to [`FORMULAS_PAGES`] in all.
#[derive(Debug, Clone, Default)]
pub struct FormulasRate {
    last_first: Option<Duration>,
    /// The listing under way: the `next` it gave, when, and its pages.
    listing: Option<(String, Duration, u8)>,
}

pub const FORMULAS_EVERY: Duration = Duration::from_secs(5);
pub const FORMULAS_PAGE_WAIT: Duration = Duration::from_secs(60);
pub const FORMULAS_PAGES: u8 = 8;

impl FormulasRate {
    /// May a `formulas` with `after` go now?
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn take(&mut self, after: Option<&str>, now: Duration) -> bool {
        if let (Some(a), Some((next, at, pages))) = (after, self.listing.as_mut()) {
            if a == next && now.saturating_sub(*at) <= FORMULAS_PAGE_WAIT && *pages < FORMULAS_PAGES {
                *pages += 1;
                return true;
            }
        }
        if self.last_first.is_some_and(|t| now.saturating_sub(t) < FORMULAS_EVERY) {
            return false;
        }
        self.last_first = Some(now);
        self.listing = Some((String::new(), now, 1));
        true
    }

    /// The page just answered said `next` (none: the listing is done).
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn answered(&mut self, next: Option<&str>, now: Duration) {
        match (next, self.listing.as_mut()) {
            (Some(n), Some(l)) => {
                l.0 = n.to_string();
                l.1 = now;
            }
            _ => self.listing = None,
        }
    }
}

/// `ack` for an accepted command; `r`: its result as JSON, if any.
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn ack_ok(k: &KeyId, n: u64, r: Option<&str>) -> String {
    match r {
        Some(r) => format!(r#"{{"t":"ack","k":"{}","n":{n},"ok":true,"r":{r}}}"#, hex(k)),
        None => format!(r#"{{"t":"ack","k":"{}","n":{n},"ok":true}}"#, hex(k)),
    }
}

/// `ack` for a refused one (`n` left out when it could not be read).
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn ack_err(k: &KeyId, n: Option<u64>, err: Refusal) -> String {
    match n {
        Some(n) => format!(r#"{{"t":"ack","k":"{}","n":{n},"err":"{}"}}"#, hex(k), err.code()),
        None => format!(r#"{{"t":"ack","k":"{}","err":"{}"}}"#, hex(k), err.code()),
    }
}

/// The body of `POST /v1/register`: the version and SHA-256 of the panel
/// secret.
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn register_body(secret: &[u8; 32]) -> String {
    format!(r#"{{"v":{VERSION},"secret_hash":"{}"}}"#, hex(&sha256(&[secret])))
}

/// The server's answer to `POST /v1/register` (`{"dev":"<base32>"}`): the
/// panel's new Connect id.
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn parse_register_answer(body: &[u8]) -> Option<Dev> {
    if body.len() > MAX_TEXT_IN {
        return None;
    }
    json::parse(body).ok()?.get("dev")?.as_str().and_then(Dev::parse)
}

/// The reason of a "stay away" close ([`CLOSE_STAY_AWAY`]) as minutes: its
/// text a decimal of at most 5 digits, else none.
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn stay_away_minutes(reason: &[u8]) -> Option<u32> {
    if reason.is_empty() || reason.len() > 5 || !reason.iter().all(u8::is_ascii_digit) {
        return None;
    }
    core::str::from_utf8(reason).ok()?.parse().ok()
}

/// What the panel can do, for `hello`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Caps {
    /// Takes `send`.
    pub send: bool,
    /// Chunks of credit at a time.
    pub window: u32,
}

/// The panel's first message on a connection.
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn hello(secret: &[u8; 32], fw: &str, sess: &[u8; 12], caps: Caps) -> String {
    let mut s = format!(r#"{{"t":"hello","v":{VERSION},"secret":"{}","fw":"#, hex(secret));
    push_str(&mut s, fw);
    s.push_str(&format!(r#","sess":"{}","caps":{{"send":{},"win":{}}}}}"#, hex(sess), caps.send, caps.window));
    s
}

/// `info`: the panel's name (after `hello` and after a rename).
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn info(name: &str) -> String {
    let mut s = String::from(r#"{"t":"info","name":"#);
    push_str(&mut s, name);
    s.push('}');
    s
}

#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn pair_open(pair_id: &[u8; 10], ttl_s: u32) -> String {
    format!(r#"{{"t":"pair-open","pair_id":"{}","ttl":{ttl_s}}}"#, hex(pair_id))
}

#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn pair_close(pair_id: &[u8; 10]) -> String {
    format!(r#"{{"t":"pair-close","pair_id":"{}"}}"#, hex(pair_id))
}

/// The panel's answer to guest `g`'s `pair`, once the key is stored.
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn paired(g: u32, k: &KeyId, mac2: &[u8; 32]) -> String {
    format!(r#"{{"t":"paired","g":{g},"k":"{}","mac2":"{}"}}"#, hex(k), hex(mac2))
}

#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn paired_err(g: u32, err: &str) -> String {
    format!(r#"{{"t":"paired","g":{g},"err":"{err}"}}"#)
}

/// The largest guest slot `g` a `pair` may carry (and `paired` echoes):
/// 2^31 - 1. Relays assign 1 to 255 (protocol 4.1).
pub const MAX_GUEST_SLOT: u32 = (1 << 31) - 1;

/// The panel's last message under an identity it is dropping.
pub const FORGET: &str = r#"{"t":"forget"}"#;

/// A message the panel reads from the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerMsg {
    /// `watch {on}`: whether a controller is looking.
    Watch(bool),
    /// A guest's `pair`, relayed; `g`: the guest's slot, echoed in `paired`
    /// (0 to [`MAX_GUEST_SLOT`]: a relay assigns 1 to 255, which panels up
    /// to 1.1.0-beta.4 read as one byte; this one takes any up to 2^31 - 1).
    /// `label` is as sent: its bytes go into the MAC check
    /// ([`Window::check`](super::pair::Window::check)) and it is cleaned
    /// only after that.
    Pair { g: u32, public: [u8; 32], label: String, mac: [u8; 32] },
    /// A type this version does not know: ignored.
    Unknown,
}

/// Why a text message was not read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextError {
    TooBig,
    /// Not JSON, no `t`, or a known type with missing or bad fields.
    Malformed,
}

/// Read a text frame from the server.
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn parse_server_text(text: &[u8]) -> Result<ServerMsg, TextError> {
    if text.len() > MAX_TEXT_IN {
        return Err(TextError::TooBig);
    }
    let v = json::parse(text).map_err(|_| TextError::Malformed)?;
    let bad = TextError::Malformed;
    match v.get("t").and_then(Value::as_str).ok_or(bad)? {
        "watch" => Ok(ServerMsg::Watch(v.get("on").and_then(Value::as_bool).ok_or(bad)?)),
        "pair" => {
            let g = v.get("g").and_then(Value::as_int).filter(|g| (0..=i64::from(MAX_GUEST_SLOT)).contains(g)).ok_or(bad)? as u32;
            let public = v.get("pub").and_then(Value::as_str).and_then(unhex::<32>).ok_or(bad)?;
            let mac = v.get("mac").and_then(Value::as_str).and_then(unhex::<32>).ok_or(bad)?;
            let label = v.get("label").and_then(Value::as_str).ok_or(bad)?.to_string();
            Ok(ServerMsg::Pair { g, public, label, mac })
        }
        _ => Ok(ServerMsg::Unknown),
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subprotocols() {
        assert_eq!(subprotocol(1), "odc1");
        assert_eq!(parse_subprotocol("odc1"), Some(1));
        for bad in ["odc", "odc01", "odc0", "odc-1", "odc1x", "ODC1", "odc1000", "odc+1"] {
            assert_eq!(parse_subprotocol(bad), None, "{bad}");
        }
    }

    #[test]
    fn negotiation_takes_current_and_previous() {
        assert_eq!(negotiate(["odc1"]), Negotiated::Accept(1));
        assert_eq!(negotiate(["d.x", "odc1", "odc9"]), Negotiated::Accept(VERSION));
        assert_eq!(negotiate(["odc9"]), Negotiated::TooNew { max: VERSION });
        assert_eq!(negotiate(["chat"]), Negotiated::NoVersion);
        assert_eq!(negotiate([]), Negotiated::NoVersion);
        const { assert!(MIN_VERSION == VERSION || MIN_VERSION == VERSION - 1) };
    }

    #[test]
    fn dev_ids() {
        let key = b"test id key, never a real one";
        let d = Dev::mint(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10], key);
        assert!(d.tag_ok(key));
        assert!(!d.tag_ok(b"another key"));
        let mut forged = d;
        forged.0[0] ^= 1;
        assert!(!forged.tag_ok(key));
        assert_eq!(d.text().len(), 26);
        assert_eq!(Dev::parse(&d.text()), Some(d));
        assert_eq!(Dev::parse(&d.text().to_ascii_uppercase()), None);
        assert_eq!(Dev::from_subprotocol(&d.subprotocol()), Some(d));
    }

    #[test]
    fn the_allow_list_is_what_parse_command_knows() {
        let keys = KeyList::new();
        let limits = Limits { max_brightness: 128 };
        let empty = Value::Object(Vec::new());
        for c in ALLOWED {
            assert_ne!(parse_command(c, &empty, &limits, true, &keys), Err(Refusal::NotAllowed), "{c}");
        }
        for c in ["update", "firmware", "delete", "wifi", "reboot", "restart", "config", "forget", "", "PLAY", "next "] {
            assert_eq!(parse_command(c, &empty, &limits, true, &keys), Err(Refusal::NotAllowed), "{c}");
        }
        assert!(allow_list_is_clean());
        assert!(contains(b"remove-controller", b"controller") && !contains(b"ab", b"abc"));
    }

    #[test]
    fn paths() {
        for ok in ["/gifs/Arcade/pac man.gif", "/gifs/a/b/c.GIF", "@formula:fire", "/formulas/big-clock.psi", "/gifs/Sent/x.gif"] {
            assert!(remote_play_path_ok(ok), "{ok}");
        }
        for bad in [
            "/gifs/../wifi.txt", "/gifs/a/../../x.gif", "/cache/a/xx/b.gif", "/gifs/.hidden/x.gif", "/gifs//x.gif",
            "/gifs/x.txt", "gifs/x.gif", "/gifs/.gif", "/gifs/a\\..\\b.gif", "@formula:~sent", "@formula:~sent?psb=AA",
            "@formula?psb=AAAA", "@formula:Fire", "@formula:", "/formulas/../x.psi", "/formulas/~sent.psi",
            "/gifs/Arcade./x.gif", "/gifs/Arcade /x.gif", "/gifs/a/x.gif.", "@card:boot", "/gifs/x.gif?x=1", "/gifs/a\u{0}.gif", "/filelist.txt", "",
        ] {
            assert!(!remote_play_path_ok(bad), "{bad}");
        }
        assert!(!remote_play_path_ok(&format!("/gifs/{}.gif", "a".repeat(240))));
        assert!(remote_set_ok("") && remote_set_ok("consoles") && remote_set_ok("filelist.alt"));
        for bad in ["../x", "a/b", ".tags", "a\\b", "x\n", "consoles.", "consoles "] {
            assert!(!remote_set_ok(bad), "{bad}");
        }
        assert!(sent_name_ok("cat.gif") && !sent_name_ok("a/cat.gif") && !sent_name_ok(".gif") && !sent_name_ok("cat.png"));
        assert!(!sent_name_ok(&format!("{}.gif", "a".repeat(117))));
        assert!(sent_name_ok(&format!("{}.gif", "a".repeat(116))));
        // 116 three-byte characters: 120 characters but 352 bytes.
        assert!(!sent_name_ok(&format!("{}.gif", "\u{20ac}".repeat(116))));
        assert!(sent_name_ok(&format!("{}.gif", "\u{20ac}".repeat(75))));
        assert!(!sent_name_ok(&format!("{}.gif", "\u{20ac}".repeat(76))));
        // Characters outside the BMP: 56 fit the 229 bytes, 57 do not.
        assert!(sent_name_ok(&format!("{}.gif", "\u{1F600}".repeat(56))));
        assert!(!sent_name_ok(&format!("{}.gif", "\u{1F600}".repeat(57))));
        assert_eq!(MAX_SENT_NAME_BYTES, 229);
    }

    #[test]
    fn buckets() {
        let t = |ms: u64| Duration::from_millis(ms);
        let mut b = Bucket::new(4, t(0));
        assert!((0..4).all(|_| b.take(t(0))));
        assert!(!b.take(t(0)));
        assert!(!b.take(t(200)));
        assert!(b.take(t(250)));
        // Time going backwards gives nothing.
        assert!(!b.take(t(100)));
    }

    #[test]
    fn server_text() {
        assert_eq!(parse_server_text(br#"{"t":"watch","on":true}"#), Ok(ServerMsg::Watch(true)));
        assert_eq!(parse_server_text(br#"{"t":"watch","on":true,"extra":1}"#), Ok(ServerMsg::Watch(true)));
        assert_eq!(parse_server_text(br#"{"t":"later","x":1}"#), Ok(ServerMsg::Unknown));
        assert_eq!(parse_server_text(br#"{"t":"watch"}"#), Err(TextError::Malformed));
        assert_eq!(parse_server_text(br#"{"on":true}"#), Err(TextError::Malformed));
        assert_eq!(parse_server_text(b"ping"), Err(TextError::Malformed));
        let big = format!(r#"{{"t":"watch","on":true,"p":"{}"}}"#, "x".repeat(MAX_TEXT_IN));
        assert_eq!(parse_server_text(big.as_bytes()), Err(TextError::TooBig));
    }

    /// A guest's slot `g`: any integer 0 to 2^31 - 1, echoed whole in
    /// `paired` (the relay once gave 31-bit slots that a one-byte read
    /// dropped as malformed, so pairing never answered).
    #[test]
    fn pair_slots() {
        let pair = |g: &str| format!(r#"{{"t":"pair","g":{g},"pub":"{}","label":"phone","mac":"{}"}}"#, "ab".repeat(32), "cd".repeat(32));
        for (text, want) in [("0", 0), ("1", 1), ("7", 7), ("255", 255), ("256", 256), ("65536", 65536), ("2147483647", MAX_GUEST_SLOT)] {
            match parse_server_text(pair(text).as_bytes()) {
                Ok(ServerMsg::Pair { g, public, label, mac }) => {
                    assert_eq!(g, want, "{text}");
                    assert_eq!((public, label.as_str(), mac), ([0xab; 32], "phone", [0xcd; 32]));
                    let back = json::parse(paired(g, &[1; 8], &[2; 32]).as_bytes()).unwrap();
                    assert_eq!(back.get("g").and_then(Value::as_int), Some(i64::from(want)), "paired {text}");
                    let back = json::parse(paired_err(g, "bad-mac").as_bytes()).unwrap();
                    assert_eq!(back.get("g").and_then(Value::as_int), Some(i64::from(want)), "paired_err {text}");
                }
                other => panic!("{text}: {other:?}"),
            }
        }
        for bad in ["2147483648", "4294967296", "9007199254740991", "-1", "-2147483648", "1.5", "1e2", "-0", "\"1\"", "true", "null", "[1]", "{}"] {
            assert_eq!(parse_server_text(pair(bad).as_bytes()), Err(TextError::Malformed), "{bad}");
        }
        let no_g = format!(r#"{{"t":"pair","pub":"{}","label":"phone","mac":"{}"}}"#, "ab".repeat(32), "cd".repeat(32));
        assert_eq!(parse_server_text(no_g.as_bytes()), Err(TextError::Malformed));
    }

    #[test]
    fn formula_lists() {
        let parse = |s: &str| json::parse(s.as_bytes()).unwrap();
        let mut l = FormulaList::new(true);
        assert!(l.builtin("fire", "Fire") && l.builtin("big-clock", "Big \"clock\""));
        let r = l.finish(false, None);
        assert_eq!(r, r#"{"builtin":[["fire","Fire"],["big-clock","Big \"clock\""]],"card":[],"more":false}"#);
        let mut l = FormulaList::new(false);
        l.builtin("fire", "Fire");
        assert!(!l.card("x", "X"), "no card: nothing of the card's");
        let r = l.finish(true, Some("x"));
        assert_eq!(r, r#"{"builtin":[["fire","Fire"]],"card":null,"more":false}"#);
        assert!(parse(&r).get("card").is_some_and(|c| matches!(c, Value::Null)));
        let mut l = FormulaList::new(true);
        assert!(l.card("a", "A") && !l.builtin("b", "B"), "built-in ones before the card's only");
        assert_eq!(l.finish(true, Some("a")), r#"{"builtin":[],"card":[["a","A"]],"more":true,"next":"a"}"#);
        let l = FormulaList::new(true);
        assert_eq!(l.finish(true, None), r#"{"builtin":[],"card":[],"more":true}"#);
        let l = FormulaList::new(true);
        assert_eq!(l.finish(true, Some("Bad/stem")), r#"{"builtin":[],"card":[],"more":true}"#);
    }

    /// Fill a list until one does not fit; whatever the closing, the result
    /// is at most RESULT_ROOM (and so the ack within MAX_TEXT_OUT), and the
    /// room is used to within an entry and the closing's reserve.
    fn fill(card: bool, name: &str) -> (String, usize) {
        let mut l = FormulaList::new(card);
        let mut n = 0;
        loop {
            let stem = format!("{}{n:03}", "s".repeat(61));
            let ok = if card { l.card(&stem, name) } else { l.builtin(&stem, name) };
            if !ok {
                break;
            }
            n += 1;
        }
        assert!(n > 0 && l.full());
        assert!(!l.card("x", "X") && !l.builtin("x", "X"));
        (l.finish(false, Some(&"z".repeat(64))), n)
    }

    #[test]
    fn formula_lists_stay_in_room() {
        for name in ["\u{2028}".repeat(60), "\u{7f}".repeat(60), "\"".repeat(60), "x".to_string()] {
            for card in [true, false] {
                let (r, n) = fill(card, &name);
                assert!(r.len() <= RESULT_ROOM, "{} > {RESULT_ROOM} (card {card}, {n} entries)", r.len());
                let ack = ack_ok(&[0xff; 8], MAX_N, Some(&r));
                assert!(ack.len() <= MAX_TEXT_OUT, "{}", ack.len());
                let v = json::parse(ack.as_bytes()).unwrap();
                let r = v.get("r").unwrap();
                assert_eq!(r.get("more").and_then(Value::as_bool), Some(card), "{name:?}");
            }
        }
    }

    /// The exact boundary: entries made to land the result at RESULT_ROOM
    /// to the byte, with no card (its closing is 27 bytes) and with one (at
    /// most 88), and one byte past it refused.
    #[test]
    fn formula_lists_at_the_boundary() {
        for card in [false, true] {
            // One entry, its name sized so that it just fits.
            let head = r#"{"builtin":["#.len();
            let close = if card { CARD_OPEN.len() + CLOSE_CARD_MAX } else { CLOSE_NULL.len() };
            let fixed = head + r#"["s",""]"#.len() + close;
            let fits = "n".repeat(RESULT_ROOM - fixed);
            let mut l = FormulaList::new(card);
            assert!(l.builtin("s", &fits), "card {card}: exactly at the room");
            let r = l.finish(false, None);
            assert!(r.len() <= RESULT_ROOM);
            let mut l = FormulaList::new(card);
            assert!(!l.builtin("s", &format!("{fits}n")), "card {card}: one byte past it");
            assert!(l.full());
        }
        // Without a card, the most a result can be is exactly RESULT_ROOM.
        let fits = "n".repeat(RESULT_ROOM - r#"{"builtin":["#.len() - r#"["s",""]"#.len() - CLOSE_NULL.len());
        let mut l = FormulaList::new(false);
        l.builtin("s", &fits);
        assert_eq!(l.finish(false, None).len(), RESULT_ROOM);
    }

    /// With `connect-formula-add` (test images only): a published gallery
    /// Formula's own signature (the committed `signatures.json`) is taken,
    /// over its argument stem, sizes and hashes; any change to what it
    /// covers, another key id, format or a revision under the floor is
    /// `not-allowed`; malformed fields are `bad-arg`. A part parses (the
    /// firmware refuses it with no add held).
    #[cfg(feature = "connect-formula-add")]
    #[test]
    fn formula_add_takes_only_a_published_signature() {
        use super::super::fadd::b64url;
        use super::super::json;
        let keys = KeyList::new();
        let limits = Limits { max_brightness: 128 };
        let sigs: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../opendmd-web/formulas/signatures.json")).unwrap()).unwrap();
        let e = &sigs["formulas"]["plasma"][0];
        let sig = b64url(&super::super::codec::unhex::<64>(e["sig"].as_str().unwrap()).unwrap());
        let body = |stem: &str, kid: u64, abi: u64, rev: u64, src: u64, sig: &str| {
            format!(
                r#"{{"id":3,"stem":"{stem}","kid":{kid},"abi":{abi},"rev":{rev},"sig":"{sig}","src_size":{src},"src_sha256":"{}","psb_size":{},"psb_sha256":"{}"}}"#,
                e["src_sha256"].as_str().unwrap(),
                e["psb_len"],
                e["psb_sha256"].as_str().unwrap()
            )
        };
        let (src, kid, abi, rev) = (e["src_len"].as_u64().unwrap(), e["kid"].as_u64().unwrap(), e["abi"].as_u64().unwrap(), e["rev"].as_u64().unwrap());
        let run = |b: String| parse_command("formula-add", &json::parse(b.as_bytes()).unwrap(), &limits, true, &keys);
        match run(body("plasma", kid, abi, rev, src, &sig)) {
            Ok(Command::FormulaAdd(x)) => assert!(x.args.signed_by(&x.signed) && x.signed.stem() == "plasma"),
            other => panic!("the published signature: {other:?}"),
        }
        let mut flipped = super::super::codec::unhex::<64>(e["sig"].as_str().unwrap()).unwrap();
        flipped[10] ^= 1;
        for (what, b) in [
            ("another stem (a landing stem)", body("plasma-2", kid, abi, rev, src, &sig)),
            ("another source's size", body("plasma", kid, abi, rev, src + 1, &sig)),
            ("another format", body("plasma", kid, abi + 1, rev, src, &sig)),
            ("a revision under the floor", body("plasma", kid, abi, 0, src, &sig)),
            ("an unknown key id", body("plasma", 9, abi, rev, src, &sig)),
            ("a flipped bit", body("plasma", kid, abi, rev, src, &b64url(&flipped))),
            ("no signature (zeros)", body("plasma", kid, abi, rev, src, &b64url(&[0; 64]))),
        ] {
            assert_eq!(run(b).map(drop), Err(Refusal::NotAllowed), "{what}");
        }
        assert_eq!(run(body("plasma", 256, abi, rev, src, &sig)).map(drop), Err(Refusal::BadArg), "kid past 255");
        assert_eq!(run(body("plasma", kid, abi, rev, src, &sig[..80])).map(drop), Err(Refusal::BadArg), "a short sig");
        let part = r#"{"id":3,"off":0,"data":"bmFtZSAiUmlmdCI"}"#;
        let v = json::parse(part.as_bytes()).unwrap();
        assert!(matches!(parse_command("formula-part", &v, &limits, true, &keys), Ok(Command::FormulaPart { id: 3, off: 0, .. })));
        // The largest add is a command (a 40-character stem, every field at its widest).
        let dev = Dev([0; 16]);
        let wide = format!(
            r#""id":{},"stem":"{}","kid":255,"abi":255,"rev":{},"sig":"{sig}","src_size":16384,"src_sha256":"{}","psb_size":8192,"psb_sha256":"{}""#,
            u32::MAX,
            "r".repeat(40),
            u32::MAX,
            "f".repeat(64),
            "f".repeat(64)
        );
        let b = command_body(&dev, &[0; 12], MAX_N, "formula-add", &wide);
        assert!(b.len() <= MAX_BODY, "{}", b.len());
    }

    /// Without `connect-formula-add` (every release and beta) the two
    /// commands are not on the allow-list.
    #[test]
    fn formula_add_is_absent_without_its_feature() {
        let keys = KeyList::new();
        let limits = Limits { max_brightness: 128 };
        let empty = Value::Object(Vec::new());
        let on = cfg!(feature = "connect-formula-add");
        for c in ["formula-add", "formula-part"] {
            assert_eq!(ALLOWED.contains(&c), on, "{c}");
            if !on {
                assert_eq!(parse_command(c, &empty, &limits, true, &keys), Err(Refusal::NotAllowed), "{c}");
            }
        }
    }

    #[test]
    fn formulas_rate() {
        let t = |ms: u64| Duration::from_millis(ms);
        let mut r = FormulasRate::default();
        assert!(r.take(None, t(0)));
        assert!(!r.take(None, t(1000)), "a second first page within 5 s");
        r.answered(Some("b"), t(1000));
        // The listing's next pages are free, up to 8 in all, but only with
        // the `next` it gave.
        assert!(!r.take(Some("zz"), t(1100)), "another after is a first page");
        for i in 2..=FORMULAS_PAGES {
            assert!(r.take(Some("b"), t(1200)), "page {i}");
            r.answered(Some("b"), t(1200));
        }
        assert!(!r.take(Some("b"), t(1300)), "past 8 pages");
        // Done: the next listing after 5 s.
        r.answered(None, t(1300));
        assert!(!r.take(Some("b"), t(2000)));
        assert!(r.take(None, t(5000)));
        r.answered(Some("c"), t(5000));
        // A stale listing's page (past 60 s) is a first page: it takes the
        // rate, and a second at once is refused.
        assert!(r.take(Some("c"), t(66_000)), "a stale page, 5 s after the last first page");
        assert!(!r.take(Some("d"), t(66_100)));
    }

    #[test]
    fn formulas_arguments() {
        let keys = KeyList::new();
        let limits = Limits { max_brightness: 128 };
        let p = |a: &str| parse_command("formulas", &json::parse(a.as_bytes()).unwrap(), &limits, false, &keys);
        assert_eq!(p("{}"), Ok(Command::Formulas { after: None }));
        assert_eq!(p(r#"{"after":"my-road"}"#), Ok(Command::Formulas { after: Some("my-road".into()) }));
        for bad in [r#"{"after":""}"#, r#"{"after":"My-road"}"#, r#"{"after":"../x"}"#, r#"{"after":1}"#, r#"{"after":null}"#, r#"{"page":2}"#, r#"{"after":"a","x":1}"#] {
            assert_eq!(p(bad), Err(Refusal::BadArg), "{bad}");
        }
    }

    #[test]
    fn register_answers() {
        let d = Dev::mint(&[7; 10], b"k");
        assert_eq!(parse_register_answer(format!(r#"{{"dev":"{}"}}"#, d.text()).as_bytes()), Some(d));
        assert_eq!(parse_register_answer(format!(r#"{{"dev":"{}","x":1}}"#, d.text()).as_bytes()), Some(d));
        for bad in [&b"{}"[..], b"{\"dev\":1}", b"{\"dev\":\"ABC\"}", b"[]", b"", b"{\"dev\":\"x\",\"dev\":\"y\"}"] {
            assert_eq!(parse_register_answer(bad), None, "{bad:?}");
        }
        assert_eq!(stay_away_minutes(b"15"), Some(15));
        for bad in [&b""[..], b"-1", b"1.5", b"123456", b"x"] {
            assert_eq!(stay_away_minutes(bad), None);
        }
    }

    #[test]
    fn messages_parse_back() {
        let sess = [3u8; 12];
        let h = hello(&[1; 32], "1.1.0-beta.2", &sess, Caps { send: false, window: 2 });
        let v = json::parse(h.as_bytes()).unwrap();
        assert_eq!(v.get("t").and_then(Value::as_str), Some("hello"));
        assert_eq!(v.get("sess").and_then(Value::as_str), Some(hex(&sess).as_str()));
        assert!(h.len() < MAX_TEXT_IN);
        for m in [info("Den \"panel\""), pair_open(&[1; 10], 300), pair_close(&[1; 10]), paired(1, &[2; 8], &[3; 32]), paired_err(1, "bad-mac"), FORGET.into(), register_body(&[4; 32]), ack_ok(&[1; 8], 3, Some("[1,2]")), ack_err(&[1; 8], None, Refusal::BadSig)] {
            assert!(json::parse(m.as_bytes()).is_ok(), "{m}");
        }
    }
}

#[cfg(test)]
mod size_tests {
    use super::*;
    use crate::connect::keys::{Entry, KeyList};

    /// Eight controllers with the worst labels JSON can make of 24 bytes
    /// (every byte a `"`, written `\"`), the largest numbers JSON carries, in an `ack`:
    /// inside what the panel may send, and over what it may receive (so
    /// the two limits really differ).
    #[test]
    fn a_full_controllers_ack_fits_the_panels_text_limit() {
        let mut keys = KeyList::new();
        for s in 1..=8u8 {
            let public = SigningKey::from_bytes(&[s; 32]).verifying_key().to_bytes();
            keys.add(Entry { public, flags: 1, label: "\"".repeat(24), added: u32::MAX, added_by: Some([0xff; 8]) }).unwrap();
        }
        let r = keys.controllers_result(|_| Some(MAX_N));
        let ack = ack_ok(&[0xff; 8], MAX_N, Some(&r));
        assert!(json::parse(ack.as_bytes()).is_ok());
        assert!(ack.len() <= MAX_TEXT_OUT, "{} > {}", ack.len(), MAX_TEXT_OUT);
        assert!(ack.len() > MAX_TEXT_IN, "{}", ack.len());
    }
}

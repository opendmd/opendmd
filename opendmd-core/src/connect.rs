//! OpenDMD Connect, spike S0: the host-testable half of the firmware's
//! measurement probe (`firmware/src/connect_probe.rs`, feature
//! `connect-probe`). No protocol and no pairing: what a bare WebSocket
//! client over TLS needs, written so the host can test it.
//!
//! - [`Decoder`], [`encode_frame`]: RFC 6455 framing, client side (our
//!   frames masked, the server's not), streaming: a binary message is
//!   handed on in pieces as it arrives and never held whole.
//! - [`upgrade_request`], [`check_upgrade`]: the HTTP/1.1 upgrade and its
//!   `Sec-WebSocket-Accept` check ([`sha1`], [`base64`]).
//! - [`Backoff`]: 5 s doubling to 15 minutes with jitter; 0-30 s before the
//!   first retry after a session that ran (CONNECT.md 5.3, 9.4).
//! - [`Hist`]: handshake times in fixed millisecond buckets, p50/p95.
//! - [`Payload`], [`Fnv`]: M10's test stream, the same bytes the echo
//!   Worker (`connect/s0-echo/src/payload.js`) makes from a seed.
//! - [`cert_subject`]: a certificate's subject Name (its DER, tag and
//!   length included), how the probe finds a pinned root without parsing
//!   any it does not need.
//! - [`civil_to_unix`]: an X.509 time against the build date (the clock
//!   floor before SNTP, CONNECT.md 9.4).
//! - [`ed25519_vector`] (with `ota`): M9's fixed signature check.
//!
//! Nothing here allocates per frame beyond what a caller hands it.

use std::time::Duration;

// --- SHA-1 and base64 (the upgrade's accept key only) -------------------

/// SHA-1 of `data` (RFC 3174). Only for `Sec-WebSocket-Accept`, where
/// RFC 6455 fixes it; never for anything that needs collision resistance.
pub fn sha1(data: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [0x6745_2301, 0xEFCD_AB89, 0x98BA_DCFE, 0x1032_5476, 0xC3D2_E1F0];
    let bit_len = (data.len() as u64).wrapping_mul(8);
    let (blocks, rest) = data.as_chunks::<64>();
    for block in blocks {
        sha1_block(&mut h, block);
    }
    let mut block = [0u8; 64];
    block[..rest.len()].copy_from_slice(rest);
    block[rest.len()] = 0x80;
    if rest.len() >= 56 {
        sha1_block(&mut h, &block);
        block = [0u8; 64];
    }
    block[56..].copy_from_slice(&bit_len.to_be_bytes());
    sha1_block(&mut h, &block);
    let mut out = [0u8; 20];
    for (k, w) in h.iter().enumerate() {
        out[k * 4..k * 4 + 4].copy_from_slice(&w.to_be_bytes());
    }
    out
}

fn sha1_block(h: &mut [u32; 5], block: &[u8; 64]) {
    let mut w = [0u32; 80];
    for t in 0..16 {
        w[t] = u32::from_be_bytes([block[t * 4], block[t * 4 + 1], block[t * 4 + 2], block[t * 4 + 3]]);
    }
    for t in 16..80 {
        w[t] = (w[t - 3] ^ w[t - 8] ^ w[t - 14] ^ w[t - 16]).rotate_left(1);
    }
    let [mut a, mut b, mut c, mut d, mut e] = *h;
    for (t, wt) in w.iter().enumerate() {
        let (f, k) = match t {
            0..=19 => ((b & c) | (!b & d), 0x5A82_7999),
            20..=39 => (b ^ c ^ d, 0x6ED9_EBA1),
            40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1B_BCDC),
            _ => (b ^ c ^ d, 0xCA62_C1D6),
        };
        let temp = a.rotate_left(5).wrapping_add(f).wrapping_add(e).wrapping_add(k).wrapping_add(*wt);
        e = d;
        d = c;
        c = b.rotate_left(30);
        b = a;
        a = temp;
    }
    for (x, y) in h.iter_mut().zip([a, b, c, d, e]) {
        *x = x.wrapping_add(y);
    }
}

/// Standard base64 with padding.
pub fn base64(data: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (u32::from(c[0]) << 16) | (u32::from(*c.get(1).unwrap_or(&0)) << 8) | u32::from(*c.get(2).unwrap_or(&0));
        out.push(A[(n >> 18) as usize & 63] as char);
        out.push(A[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 { A[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if c.len() > 2 { A[n as usize & 63] as char } else { '=' });
    }
    out
}

/// What the server must answer in `Sec-WebSocket-Accept` for `key`.
pub fn ws_accept(key: &str) -> String {
    let mut s = Vec::with_capacity(key.len() + 36);
    s.extend_from_slice(key.as_bytes());
    s.extend_from_slice(b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11");
    base64(&sha1(&s))
}

// --- The upgrade ---------------------------------------------------------

/// The upgrade request for `path` (with its query) on `host`; `key` is the
/// base64 of 16 random bytes.
pub fn upgrade_request(host: &str, path: &str, key: &str) -> String {
    format!(
        "GET {path} HTTP/1.1\r\nHost: {host}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\nUser-Agent: opendmd-connect-s0\r\n\r\n"
    )
}

/// The most of a response's head the probe reads.
pub const MAX_UPGRADE_HEAD: usize = 1024;

/// Where the head of an HTTP response ends in `buf` (the index just past
/// its blank line), if it does yet.
pub fn head_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n").map(|k| k + 4)
}

/// Check a response head: `101`, `Upgrade: websocket`, and the accept key
/// for `key`. `Err` says what was wrong, with the status line for anything
/// but 101.
pub fn check_upgrade(head: &[u8], key: &str) -> Result<(), String> {
    let text = std::str::from_utf8(head).map_err(|_| "upgrade answer is not text".to_string())?;
    let mut lines = text.split("\r\n");
    let status = lines.next().unwrap_or("");
    let code = status.split(' ').nth(1).unwrap_or("");
    if code != "101" {
        return Err(format!("upgrade refused: {}", status.chars().take(48).collect::<String>()));
    }
    let (mut upgrade, mut accept) = (false, None);
    for l in lines {
        let Some((name, value)) = l.split_once(':') else { continue };
        let value = value.trim();
        if name.eq_ignore_ascii_case("upgrade") && value.eq_ignore_ascii_case("websocket") {
            upgrade = true;
        } else if name.eq_ignore_ascii_case("sec-websocket-accept") {
            accept = Some(value);
        }
    }
    if !upgrade {
        return Err("upgrade answer has no Upgrade: websocket".into());
    }
    match accept {
        Some(a) if a == ws_accept(key) => Ok(()),
        Some(_) => Err("Sec-WebSocket-Accept does not match".into()),
        None => Err("upgrade answer has no Sec-WebSocket-Accept".into()),
    }
}

// --- Frames ----------------------------------------------------------------

pub const OP_CONT: u8 = 0x0;
pub const OP_TEXT: u8 = 0x1;
pub const OP_BINARY: u8 = 0x2;
pub const OP_CLOSE: u8 = 0x8;
pub const OP_PING: u8 = 0x9;
pub const OP_PONG: u8 = 0xA;

/// Append one masked client frame (FIN set) to `out`.
pub fn encode_frame(opcode: u8, payload: &[u8], mask: [u8; 4], out: &mut Vec<u8>) {
    out.push(0x80 | (opcode & 0x0F));
    let n = payload.len();
    if n < 126 {
        out.push(0x80 | n as u8);
    } else if n <= 0xFFFF {
        out.push(0x80 | 126);
        out.extend_from_slice(&(n as u16).to_be_bytes());
    } else {
        out.push(0x80 | 127);
        out.extend_from_slice(&(n as u64).to_be_bytes());
    }
    out.extend_from_slice(&mask);
    out.extend(payload.iter().enumerate().map(|(k, b)| b ^ mask[k & 3]));
}

/// A close frame's payload: the code, then the reason.
pub fn close_payload(code: u16, reason: &str) -> Vec<u8> {
    let mut p = code.to_be_bytes().to_vec();
    p.extend_from_slice(&reason.as_bytes()[..reason.len().min(123)]);
    p
}

/// The longest text message the probe takes (the echo Worker's are tens
/// of bytes).
pub const MAX_TEXT: usize = 256;

/// What [`Decoder::feed`] hands on.
#[derive(Debug, PartialEq, Eq)]
pub enum Event<'a> {
    /// A whole text message (at most [`MAX_TEXT`]).
    Text(&'a [u8]),
    /// A binary frame begins: its payload length.
    BinaryStart(u64),
    /// Some of its payload, as it arrived.
    Binary(&'a [u8]),
    /// The frame's payload is all here; `fin`: the message too.
    BinaryEnd { fin: bool },
    Ping(&'a [u8]),
    Pong(&'a [u8]),
    /// The server closed: its code (none if it gave none) and reason.
    Close { code: Option<u16>, reason: &'a [u8] },
}

/// A protocol error from the server's frames.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum WsError {
    /// A server frame with the mask bit (RFC 6455 5.1 forbids it).
    Masked,
    /// A control frame over 125 bytes, or fragmented.
    BadControl,
    /// A text message over [`MAX_TEXT`], or a fragmented one.
    TextTooLong,
    /// An opcode RFC 6455 does not define, or a continuation with no
    /// message to continue, or a new message inside a fragmented one.
    BadOpcode(u8),
    /// RSV bits without an extension.
    Reserved,
}

impl WsError {
    pub fn as_str(self) -> &'static str {
        match self {
            WsError::Masked => "server frame masked",
            WsError::BadControl => "bad control frame",
            WsError::TextTooLong => "text message too long",
            WsError::BadOpcode(_) => "bad opcode",
            WsError::Reserved => "reserved bits set",
        }
    }
}

/// The server's frames, fed in pieces of any size.
pub struct Decoder {
    head: [u8; 10],
    have: usize,
    /// Payload bytes of the current frame still to come; `None` between
    /// frames (reading a header).
    left: Option<u64>,
    opcode: u8,
    fin: bool,
    /// The message a continuation frame continues (`OP_BINARY`), if any.
    message: Option<u8>,
    small: [u8; MAX_TEXT],
    small_len: usize,
}

impl Default for Decoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Decoder {
    pub const fn new() -> Decoder {
        Decoder {
            head: [0; 10],
            have: 0,
            left: None,
            opcode: 0,
            fin: false,
            message: None,
            small: [0; MAX_TEXT],
            small_len: 0,
        }
    }

    /// The header's length once its first two bytes are in.
    fn head_len(&self) -> usize {
        match self.head[1] & 0x7F {
            126 => 4,
            127 => 10,
            _ => 2,
        }
    }

    /// Feed bytes; `f` is called for each event, in order.
    pub fn feed<F: FnMut(Event<'_>)>(&mut self, mut input: &[u8], f: &mut F) -> Result<(), WsError> {
        loop {
            match self.left {
                None => {
                    if input.is_empty() {
                        return Ok(());
                    }
                    let want = if self.have < 2 { 2 } else { self.head_len() };
                    let take = (want - self.have).min(input.len());
                    self.head[self.have..self.have + take].copy_from_slice(&input[..take]);
                    self.have += take;
                    input = &input[take..];
                    if self.have < 2 || self.have < self.head_len() {
                        continue;
                    }
                    self.start_frame(f)?;
                }
                Some(0) => self.end_frame(f),
                Some(left) => {
                    if input.is_empty() {
                        return Ok(());
                    }
                    let take = (left.min(input.len() as u64)) as usize;
                    let (piece, rest) = input.split_at(take);
                    input = rest;
                    self.left = Some(left - take as u64);
                    if self.opcode == OP_BINARY {
                        f(Event::Binary(piece));
                    } else {
                        self.small[self.small_len..self.small_len + take].copy_from_slice(piece);
                        self.small_len += take;
                    }
                }
            }
        }
    }

    fn start_frame<F: FnMut(Event<'_>)>(&mut self, f: &mut F) -> Result<(), WsError> {
        let (b0, b1) = (self.head[0], self.head[1]);
        if b0 & 0x70 != 0 {
            return Err(WsError::Reserved);
        }
        if b1 & 0x80 != 0 {
            return Err(WsError::Masked);
        }
        let len = match b1 & 0x7F {
            126 => u64::from(u16::from_be_bytes([self.head[2], self.head[3]])),
            127 => u64::from_be_bytes(self.head[2..10].try_into().unwrap_or([0; 8])),
            n => u64::from(n),
        };
        let fin = b0 & 0x80 != 0;
        let mut op = b0 & 0x0F;
        self.have = 0;
        match op {
            OP_CLOSE | OP_PING | OP_PONG => {
                if len > 125 || !fin {
                    return Err(WsError::BadControl);
                }
            }
            OP_TEXT => {
                if self.message.is_some() {
                    return Err(WsError::BadOpcode(op));
                }
                if len as usize > MAX_TEXT || !fin {
                    return Err(WsError::TextTooLong);
                }
            }
            OP_BINARY | OP_CONT => {
                if (op == OP_BINARY) == self.message.is_some() {
                    return Err(WsError::BadOpcode(op));
                }
                op = OP_BINARY;
                self.message = if fin { None } else { Some(OP_BINARY) };
                f(Event::BinaryStart(len));
            }
            _ => return Err(WsError::BadOpcode(op)),
        }
        self.opcode = op;
        self.fin = fin;
        self.small_len = 0;
        self.left = Some(len);
        Ok(())
    }

    fn end_frame<F: FnMut(Event<'_>)>(&mut self, f: &mut F) {
        self.left = None;
        let p = &self.small[..self.small_len];
        match self.opcode {
            OP_BINARY => f(Event::BinaryEnd { fin: self.fin }),
            OP_TEXT => f(Event::Text(p)),
            OP_PING => f(Event::Ping(p)),
            OP_PONG => f(Event::Pong(p)),
            _ => {
                let code = (p.len() >= 2).then(|| u16::from_be_bytes([p[0], p[1]]));
                f(Event::Close { code, reason: p.get(2..).unwrap_or(&[]) })
            }
        }
    }
}

// --- Backoff -----------------------------------------------------------------

/// Reconnect delays (CONNECT.md 5.3, 9.4): 5 s doubling to 15 minutes, each
/// drawn between half and all of that (jitter); the first retry after a
/// session that ran waits 0-30 s, so a whole fleet dropped at once by an
/// edge restart does not come back in the same second.
#[derive(Default, Debug, Clone, Copy)]
pub struct Backoff {
    failures: u32,
}

pub const BACKOFF_FIRST: Duration = Duration::from_secs(5);
pub const BACKOFF_CAP: Duration = Duration::from_secs(15 * 60);
pub const SPREAD_AFTER_DROP: Duration = Duration::from_secs(30);

impl Backoff {
    /// Failures in a row so far.
    pub fn failures(&self) -> u32 {
        self.failures
    }

    /// A connection attempt failed: the wait before the next. `rand`: any
    /// random `u32`.
    pub fn failed(&mut self, rand: u32) -> Duration {
        self.failures = self.failures.saturating_add(1);
        let full = BACKOFF_FIRST
            .saturating_mul(1u32 << (self.failures - 1).min(20))
            .min(BACKOFF_CAP);
        let half = full / 2;
        half + Duration::from_millis(u64::from(rand) % (half.as_millis() as u64 + 1))
    }

    /// A session ran and ended: the failures are forgotten, and the wait
    /// before the first retry is 0-30 s.
    pub fn dropped(&mut self, rand: u32) -> Duration {
        self.failures = 0;
        Duration::from_millis(u64::from(rand) % (SPREAD_AFTER_DROP.as_millis() as u64 + 1))
    }

    pub fn reset(&mut self) {
        self.failures = 0;
    }
}

// --- Handshake times -------------------------------------------------------

/// The buckets' upper bounds, ms (the last is everything above).
pub const HIST_MS: [u32; 14] = [50, 100, 200, 300, 500, 750, 1000, 1500, 2000, 3000, 4000, 6000, 8000, u32::MAX];

/// Times in [`HIST_MS`] buckets, with the count, sum and most.
#[derive(Debug, Clone, Default)]
pub struct Hist {
    pub counts: [u32; HIST_MS.len()],
    pub n: u32,
    pub sum_ms: u64,
    pub max_ms: u32,
    pub last_ms: u32,
}

impl Hist {
    pub fn add(&mut self, ms: u32) {
        let k = HIST_MS.iter().position(|&b| ms <= b).unwrap_or(HIST_MS.len() - 1);
        self.counts[k] += 1;
        self.n += 1;
        self.sum_ms += u64::from(ms);
        self.max_ms = self.max_ms.max(ms);
        self.last_ms = ms;
    }

    /// The `p`th percentile (0-100), as its bucket's upper bound (the most
    /// seen, for the top bucket or when that is lower); `None` when empty.
    pub fn percentile(&self, p: u32) -> Option<u32> {
        if self.n == 0 {
            return None;
        }
        let rank = (u64::from(self.n) * u64::from(p.min(100))).div_ceil(100).max(1);
        let mut seen = 0u64;
        for (k, &c) in self.counts.iter().enumerate() {
            seen += u64::from(c);
            if seen >= rank {
                return Some(HIST_MS[k].min(self.max_ms));
            }
        }
        Some(self.max_ms)
    }

    pub fn mean_ms(&self) -> Option<u32> {
        (self.n > 0).then(|| (self.sum_ms / u64::from(self.n)) as u32)
    }

    /// `{"n","last","max","mean","p50","p95","buckets":[…]}`.
    pub fn json(&self) -> String {
        let o = |v: Option<u32>| v.map_or_else(|| "null".to_string(), |v| v.to_string());
        let buckets: Vec<String> = self.counts.iter().map(|c| c.to_string()).collect();
        format!(
            r#"{{"n":{},"last":{},"max":{},"mean":{},"p50":{},"p95":{},"buckets":[{}]}}"#,
            self.n,
            o((self.n > 0).then_some(self.last_ms)),
            o((self.n > 0).then_some(self.max_ms)),
            o(self.mean_ms()),
            o(self.percentile(50)),
            o(self.percentile(95)),
            buckets.join(",")
        )
    }
}

// --- M10's stream ------------------------------------------------------------

/// The test stream: xorshift32 from `seed` (0 is taken as 0x9E3779B9), each
/// step's word sent little-endian. `connect/s0-echo/src/payload.js` makes
/// the same bytes.
#[derive(Debug, Clone, Copy)]
pub struct Payload {
    x: u32,
    k: u8,
}

impl Payload {
    pub fn new(seed: u32) -> Payload {
        Payload { x: if seed == 0 { 0x9E37_79B9 } else { seed }, k: 0 }
    }

    pub fn next_byte(&mut self) -> u8 {
        if self.k == 0 {
            let mut x = self.x;
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            self.x = x;
        }
        let b = (self.x >> (8 * u32::from(self.k))) as u8;
        self.k = (self.k + 1) & 3;
        b
    }

    /// How many of `got` differ from the stream's next bytes.
    pub fn check(&mut self, got: &[u8]) -> u32 {
        got.iter().filter(|&&b| b != self.next_byte()).count() as u32
    }
}

/// FNV-1a, 32 bits.
#[derive(Debug, Clone, Copy)]
pub struct Fnv(pub u32);

impl Default for Fnv {
    fn default() -> Self {
        Fnv(0x811C_9DC5)
    }
}

impl Fnv {
    pub fn update(&mut self, data: &[u8]) {
        for &b in data {
            self.0 = (self.0 ^ u32::from(b)).wrapping_mul(0x0100_0193);
        }
    }
}

/// The echo Worker's `end <bytes> <fnv hex>`: (bytes, fnv).
pub fn parse_end(text: &[u8]) -> Option<(u64, u32)> {
    let t = std::str::from_utf8(text).ok()?;
    let mut w = t.split(' ');
    if w.next()? != "end" {
        return None;
    }
    let bytes = w.next()?.parse().ok()?;
    let fnv = u32::from_str_radix(w.next()?, 16).ok()?;
    Some((bytes, fnv))
}

// --- Certificates -----------------------------------------------------------

/// One DER TLV at the start of `d`: (tag, the whole TLV's length, the
/// value's offset).
fn der_tlv(d: &[u8]) -> Option<(u8, usize, usize)> {
    let tag = *d.first()?;
    let b = *d.get(1)?;
    let (len, off) = if b < 0x80 {
        (usize::from(b), 2)
    } else {
        let n = usize::from(b & 0x7F);
        if n == 0 || n > 3 {
            return None;
        }
        let mut len = 0usize;
        for k in 0..n {
            len = (len << 8) | usize::from(*d.get(2 + k)?);
        }
        (len, 2 + n)
    };
    let total = off.checked_add(len)?;
    (total <= d.len()).then_some((tag, total, off))
}

/// The subject Name of an X.509 certificate in DER, tag and length
/// included: what mbedTLS keeps as `subject_raw`, and compares a child's
/// `issuer_raw` with. `None` if `der` is not a certificate's shape.
pub fn cert_subject(der: &[u8]) -> Option<&[u8]> {
    let (tag, total, off) = der_tlv(der)?;
    if tag != 0x30 {
        return None;
    }
    let cert = &der[off..total];
    let (tag, total, off) = der_tlv(cert)?;
    if tag != 0x30 {
        return None;
    }
    let mut tbs = &cert[off..total];
    // [0] version (optional), serial, signature algorithm, issuer,
    // validity: then the subject.
    if tbs.first() == Some(&0xA0) {
        let (_, n, _) = der_tlv(tbs)?;
        tbs = &tbs[n..];
    }
    for want in [0x02, 0x30, 0x30, 0x30] {
        let (tag, n, _) = der_tlv(tbs)?;
        if tag != want {
            return None;
        }
        tbs = &tbs[n..];
    }
    let (tag, n, _) = der_tlv(tbs)?;
    (tag == 0x30).then(|| &tbs[..n])
}

/// Seconds since 1970 of a UTC civil time (proleptic Gregorian; month and
/// day from 1), for X.509 validity against the clock floor.
pub fn civil_to_unix(year: i64, mon: u32, day: u32, hour: u32, min: u32, sec: u32) -> i64 {
    // Howard Hinnant's days_from_civil.
    let y = if mon <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let m = i64::from(mon);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    days * 86_400 + i64::from(hour) * 3600 + i64::from(min) * 60 + i64::from(sec)
}

// --- M9 ----------------------------------------------------------------------

/// M9's message: 300 bytes of [`Payload`] from this seed (a command's size,
/// CONNECT.md 4.3).
pub const ED25519_MSG_SEED: u32 = 0x5EED_0009;
pub const ED25519_MSG_LEN: usize = 300;
/// The vector's public key and signature (made on the host from a fixed
/// test seed: `tests::make_ed25519_vector`).
pub const ED25519_PUBLIC: [u8; 32] = [
    0x21, 0x52, 0xf8, 0xd1, 0x9b, 0x79, 0x1d, 0x24, 0x45, 0x32, 0x42, 0xe1, 0x5f, 0x2e, 0xab, 0x6c,
    0xb7, 0xcf, 0xfa, 0x7b, 0x6a, 0x5e, 0xd3, 0x00, 0x97, 0x96, 0x0e, 0x06, 0x98, 0x81, 0xdb, 0x12,
];
pub const ED25519_SIG: [u8; 64] = [
    0x38, 0xdb, 0x3d, 0xe6, 0x70, 0x18, 0x2f, 0x19, 0x79, 0xd3, 0x3c, 0xf6, 0x1b, 0x52, 0x91, 0x56,
    0xbe, 0xad, 0xb4, 0x51, 0x3a, 0xf9, 0x6b, 0x61, 0xed, 0x0c, 0x96, 0xe3, 0x7a, 0x93, 0xb0, 0x8a,
    0x93, 0x68, 0x4c, 0xae, 0x1b, 0x35, 0xa7, 0x83, 0xc8, 0x06, 0x89, 0x74, 0x47, 0x9f, 0x9d, 0xcf,
    0xc1, 0x5a, 0x6a, 0x88, 0x85, 0xec, 0x23, 0xe1, 0x62, 0xef, 0xee, 0xa0, 0x83, 0x0a, 0x88, 0x0e,
];

/// M9's message.
pub fn ed25519_msg() -> [u8; ED25519_MSG_LEN] {
    let mut p = Payload::new(ED25519_MSG_SEED);
    std::array::from_fn(|_| p.next_byte())
}

/// Verify M9's fixed vector the way an update bundle is checked
/// (`verify_strict`). `msg` from [`ed25519_msg`] (made once, outside the
/// timing).
#[cfg(feature = "ota")]
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn ed25519_vector(msg: &[u8]) -> bool {
    use ed25519_dalek::{Signature, VerifyingKey};
    let Ok(key) = VerifyingKey::from_bytes(&ED25519_PUBLIC) else {
        return false;
    };
    key.verify_strict(msg, &Signature::from_bytes(&ED25519_SIG)).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha1_vectors() {
        let hex = |d: [u8; 20]| d.iter().map(|b| format!("{b:02x}")).collect::<String>();
        assert_eq!(hex(sha1(b"")), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
        assert_eq!(hex(sha1(b"abc")), "a9993e364706816aba3e25717850c26c9cd0d89d");
        assert_eq!(
            hex(sha1(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq")),
            "84983e441c3bd26ebaae4aa1f95129e5e54670f1"
        );
        // 55, 56 and 64 bytes: the padding's edges.
        let a = [b'a'; 64];
        assert_eq!(hex(sha1(&a[..55])), "c1c8bbdc22796e28c0e15163d20899b65621d65a");
        assert_eq!(hex(sha1(&a[..56])), "c2db330f6083854c99d4b5bfb6e8f29f201be699");
        assert_eq!(hex(sha1(&a)), "0098ba824b5c16427bd7a1122a5a442a25ec644d");
    }

    #[test]
    fn base64_and_accept() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        // RFC 6455 1.3's example.
        assert_eq!(ws_accept("dGhlIHNhbXBsZSBub25jZQ=="), "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=");
    }

    #[test]
    fn upgrade_answers() {
        let key = "dGhlIHNhbXBsZSBub25jZQ==";
        let good = b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: s3pPLMBiTxaQ9kYGzzhZRbK+xOo=\r\n\r\n";
        assert_eq!(head_end(good), Some(good.len()));
        assert_eq!(check_upgrade(good, key), Ok(()));
        let lower = b"HTTP/1.1 101 x\r\nupgrade: WebSocket\r\nsec-websocket-accept: s3pPLMBiTxaQ9kYGzzhZRbK+xOo=\r\n\r\n";
        assert_eq!(check_upgrade(lower, key), Ok(()));
        let wrong = b"HTTP/1.1 101 x\r\nUpgrade: websocket\r\nSec-WebSocket-Accept: AAAA\r\n\r\n";
        assert!(check_upgrade(wrong, key).unwrap_err().contains("does not match"));
        let refused = b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n";
        assert!(check_upgrade(refused, key).unwrap_err().contains("403 Forbidden"));
        let no_up = b"HTTP/1.1 101 x\r\nSec-WebSocket-Accept: s3pPLMBiTxaQ9kYGzzhZRbK+xOo=\r\n\r\n";
        assert!(check_upgrade(no_up, key).is_err());
        assert_eq!(head_end(b"HTTP/1.1 101 x\r\n"), None);
        let req = upgrade_request("h.example", "/ws?k=s", key);
        assert!(req.starts_with("GET /ws?k=s HTTP/1.1\r\nHost: h.example\r\n"));
        assert!(req.ends_with("\r\n\r\n"));
    }

    /// The server's frame (unmasked).
    fn server_frame(fin: bool, op: u8, payload: &[u8]) -> Vec<u8> {
        let mut v = vec![(if fin { 0x80 } else { 0 }) | op];
        let n = payload.len();
        if n < 126 {
            v.push(n as u8);
        } else if n <= 0xFFFF {
            v.push(126);
            v.extend_from_slice(&(n as u16).to_be_bytes());
        } else {
            v.push(127);
            v.extend_from_slice(&(n as u64).to_be_bytes());
        }
        v.extend_from_slice(payload);
        v
    }

    #[derive(Debug, PartialEq)]
    enum Ev {
        Text(Vec<u8>),
        Start(u64),
        Bin(Vec<u8>),
        End(bool),
        Ping(Vec<u8>),
        Pong(Vec<u8>),
        Close(Option<u16>, Vec<u8>),
    }

    /// Decode `bytes` fed `step` bytes at a time; binary pieces merged.
    fn decode(bytes: &[u8], step: usize) -> Result<Vec<Ev>, WsError> {
        let mut d = Decoder::new();
        let mut out = Vec::new();
        for c in bytes.chunks(step.max(1)) {
            d.feed(c, &mut |e| {
                let e = match e {
                    Event::Text(t) => Ev::Text(t.to_vec()),
                    Event::BinaryStart(n) => Ev::Start(n),
                    Event::Binary(b) => {
                        if let Some(Ev::Bin(v)) = out.last_mut() {
                            v.extend_from_slice(b);
                            return;
                        }
                        Ev::Bin(b.to_vec())
                    }
                    Event::BinaryEnd { fin } => Ev::End(fin),
                    Event::Ping(p) => Ev::Ping(p.to_vec()),
                    Event::Pong(p) => Ev::Pong(p.to_vec()),
                    Event::Close { code, reason } => Ev::Close(code, reason.to_vec()),
                };
                out.push(e);
            })?;
        }
        Ok(out)
    }

    #[test]
    fn frames_in_any_pieces() {
        let big: Vec<u8> = (0..70_000u32).map(|k| k as u8).collect();
        let mut s = Vec::new();
        s.extend(server_frame(true, OP_TEXT, b"pong"));
        s.extend(server_frame(true, OP_PONG, b"7"));
        s.extend(server_frame(true, OP_BINARY, &big[..1000]));
        s.extend(server_frame(true, OP_BINARY, &big));
        s.extend(server_frame(true, OP_BINARY, &big[..300]));
        s.extend(server_frame(true, OP_PING, b""));
        s.extend(server_frame(true, OP_BINARY, b""));
        s.extend(server_frame(true, OP_CLOSE, &close_payload(1001, "bye")));
        let want = vec![
            Ev::Text(b"pong".to_vec()),
            Ev::Pong(b"7".to_vec()),
            Ev::Start(1000),
            Ev::Bin(big[..1000].to_vec()),
            Ev::End(true),
            Ev::Start(70_000),
            Ev::Bin(big.clone()),
            Ev::End(true),
            Ev::Start(300),
            Ev::Bin(big[..300].to_vec()),
            Ev::End(true),
            Ev::Ping(vec![]),
            Ev::Start(0),
            Ev::End(true),
            Ev::Close(Some(1001), b"bye".to_vec()),
        ];
        for step in [1, 2, 3, 7, 125, 126, 1000, 4096, s.len()] {
            assert_eq!(decode(&s, step).unwrap(), want, "step {step}");
        }
    }

    #[test]
    fn fragmented_binary_and_hostile_frames() {
        let mut s = server_frame(false, OP_BINARY, b"ab");
        s.extend(server_frame(true, OP_PING, b"x")); // control between fragments: allowed
        s.extend(server_frame(true, OP_CONT, b"cd"));
        let got = decode(&s, 1).unwrap();
        assert_eq!(
            got,
            vec![
                Ev::Start(2),
                Ev::Bin(b"ab".to_vec()),
                Ev::End(false),
                Ev::Ping(b"x".to_vec()),
                Ev::Start(2),
                Ev::Bin(b"cd".to_vec()),
                Ev::End(true)
            ]
        );
        // Masked server frame.
        let mut m = Vec::new();
        encode_frame(OP_TEXT, b"hi", [1, 2, 3, 4], &mut m);
        assert_eq!(decode(&m, 1), Err(WsError::Masked));
        // Oversized control, fragmented control, long text, stray continuation.
        assert_eq!(decode(&server_frame(true, OP_PING, &[0; 126]), 4), Err(WsError::BadControl));
        assert_eq!(decode(&server_frame(false, OP_PING, b""), 4), Err(WsError::BadControl));
        assert_eq!(decode(&server_frame(true, OP_TEXT, &[b'a'; MAX_TEXT + 1]), 9), Err(WsError::TextTooLong));
        assert_eq!(decode(&server_frame(true, OP_CONT, b"x"), 9), Err(WsError::BadOpcode(OP_CONT)));
        assert_eq!(decode(&server_frame(true, 0x3, b""), 9), Err(WsError::BadOpcode(3)));
        assert_eq!(decode(&[0xC1, 0x00], 9), Err(WsError::Reserved));
        // A new binary message inside a fragmented one.
        let mut n = server_frame(false, OP_BINARY, b"a");
        n.extend(server_frame(true, OP_BINARY, b"b"));
        assert_eq!(decode(&n, 9), Err(WsError::BadOpcode(OP_BINARY)));
        // Close with no code.
        assert_eq!(decode(&server_frame(true, OP_CLOSE, b""), 1).unwrap(), vec![Ev::Close(None, vec![])]);
    }

    #[test]
    fn client_frames_are_masked() {
        let payload: Vec<u8> = (0..300u32).map(|k| k as u8).collect();
        for n in [0usize, 5, 125, 126, 300] {
            let mut f = Vec::new();
            let mask = [0x11, 0x22, 0x33, 0x44];
            encode_frame(OP_BINARY, &payload[..n], mask, &mut f);
            assert_eq!(f[0], 0x82);
            assert_ne!(f[1] & 0x80, 0);
            let (len, off) = match f[1] & 0x7F {
                126 => (usize::from(u16::from_be_bytes([f[2], f[3]])), 4),
                l => (usize::from(l), 2),
            };
            assert_eq!(len, n);
            assert_eq!(&f[off..off + 4], &mask);
            let un: Vec<u8> = f[off + 4..].iter().enumerate().map(|(k, b)| b ^ mask[k & 3]).collect();
            assert_eq!(un, &payload[..n]);
        }
    }

    #[test]
    fn backoff_doubles_with_jitter_and_spreads_after_a_drop() {
        let mut b = Backoff::default();
        let mut maxes = Vec::new();
        for _ in 0..12 {
            let lo = b.clone().failed(0);
            let hi = b.clone().failed(u32::MAX);
            let d = b.failed(12345);
            assert!(lo <= d && d <= hi.max(lo) + Duration::from_millis(1));
            maxes.push(hi);
        }
        assert!(maxes[0] <= BACKOFF_FIRST && maxes[0] >= BACKOFF_FIRST / 2);
        assert!(maxes.iter().all(|&d| d <= BACKOFF_CAP));
        assert!(maxes[11] >= BACKOFF_CAP / 2);
        assert_eq!(b.failures(), 12);
        let d = b.dropped(u32::MAX);
        assert!(d <= SPREAD_AFTER_DROP);
        assert_eq!(b.failures(), 0);
        // 96 attempts a day at most at the cap (CONNECT.md 9.4).
        assert!(Duration::from_secs(86_400).as_secs() / (BACKOFF_CAP / 2).as_secs() <= 192);
    }

    #[test]
    fn hist_percentiles() {
        let mut h = Hist::default();
        assert_eq!(h.percentile(50), None);
        for ms in [40, 90, 150, 160, 170, 180, 190, 210, 1200, 3500] {
            h.add(ms);
        }
        assert_eq!(h.n, 10);
        assert_eq!(h.max_ms, 3500);
        assert_eq!(h.last_ms, 3500);
        assert_eq!(h.percentile(50), Some(200));
        assert_eq!(h.percentile(95), Some(3500));
        assert_eq!(h.percentile(10), Some(50));
        let mut one = Hist::default();
        one.add(20_000);
        assert_eq!(one.percentile(50), Some(20_000));
        assert!(h.json().contains(r#""p50":200"#));
    }

    #[test]
    fn payload_matches_the_worker() {
        // The same numbers are asserted by connect/s0-echo/test/echo.test.mjs.
        let mut p = Payload::new(1);
        let first: Vec<u8> = (0..8).map(|_| p.next_byte()).collect();
        assert_eq!(first, [0x21, 0x20, 0x04, 0x00, 0x01, 0x06, 0x08, 0x04]);
        let mut p = Payload::new(1);
        let mut f = Fnv::default();
        let mut buf = vec![0u8; 524_288];
        for b in buf.iter_mut() {
            *b = p.next_byte();
        }
        f.update(&buf);
        assert_eq!(f.0, 0x2c13_55c2, "fnv {:08x}", f.0);
        let mut q = Payload::new(1);
        assert_eq!(q.check(&buf[..1000]), 0);
        let mut bad = buf[1000..2000].to_vec();
        bad[5] ^= 1;
        assert_eq!(q.check(&bad), 1);
        assert_eq!(parse_end(b"end 524288 2c1355c2"), Some((524_288, 0x2c13_55c2)));
        assert_eq!(parse_end(b"nope"), None);
        // Seed 0 is not a fixed point.
        let mut z = Payload::new(0);
        assert_ne!((0..4).map(|_| z.next_byte()).collect::<Vec<_>>(), [0, 0, 0, 0]);
    }

    #[test]
    fn subjects_of_shapes() {
        // A minimal certificate shape: SEQ{ SEQ{ [0]{INT 2}, INT, SEQ, SEQ(issuer), SEQ(validity), SEQ(subject), ... }, ... }.
        let subject = [0x30, 0x03, 0x31, 0x01, 0x00];
        let mut tbs = vec![0xA0, 0x03, 0x02, 0x01, 0x02, 0x02, 0x01, 0x07, 0x30, 0x00, 0x30, 0x02, 0x05, 0x00, 0x30, 0x00];
        tbs.extend_from_slice(&subject);
        tbs.extend_from_slice(&[0x30, 0x00]);
        let mut inner = vec![0x30, tbs.len() as u8];
        inner.extend(&tbs);
        let mut cert = vec![0x30, 0x81, inner.len() as u8];
        cert.extend(&inner);
        assert_eq!(cert_subject(&cert), Some(&subject[..]));
        assert_eq!(cert_subject(&cert[..cert.len() - 3]), None);
        assert_eq!(cert_subject(b"\x02\x01\x00"), None);
        assert_eq!(cert_subject(&[]), None);
    }

    #[test]
    fn civil_dates() {
        assert_eq!(civil_to_unix(1970, 1, 1, 0, 0, 0), 0);
        assert_eq!(civil_to_unix(2000, 3, 1, 0, 0, 0), 951_868_800);
        assert_eq!(civil_to_unix(2026, 11, 12, 12, 30, 15), 1_794_486_615);
        assert_eq!(civil_to_unix(2024, 2, 29, 23, 59, 59), 1_709_251_199);
    }

    /// The pinned roots the firmware embeds (`firmware/connect-roots/`)
    /// all parse to a subject.
    #[test]
    fn pinned_roots_have_subjects() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../firmware/connect-roots");
        let mut n = 0;
        for e in std::fs::read_dir(&dir).unwrap() {
            let p = e.unwrap().path();
            if p.extension().and_then(|x| x.to_str()) != Some("der") {
                continue;
            }
            let der = std::fs::read(&p).unwrap();
            let s = cert_subject(&der).unwrap_or_else(|| panic!("{}: no subject", p.display()));
            assert_eq!(s[0], 0x30);
            n += 1;
        }
        assert!(n >= 3, "{n} roots in {}", dir.display());
    }

    #[cfg(feature = "ota")]
    #[test]
    fn ed25519_vector_verifies_and_rejects() {
        let msg = ed25519_msg();
        assert!(ed25519_vector(&msg));
        let mut bad = msg;
        bad[0] ^= 1;
        assert!(!ed25519_vector(&bad));
    }

    /// How [`ED25519_PUBLIC`] and [`ED25519_SIG`] were made (a fixed test
    /// seed, not a secret): `cargo test --features ota,connect make_ed25519 -- --ignored --nocapture`.
    #[cfg(feature = "ota")]
    #[test]
    #[ignore]
    fn make_ed25519_vector() {
        use ed25519_dalek::{Signer, SigningKey};
        let key = SigningKey::from_bytes(&[0x42; 32]);
        let sig = key.sign(&ed25519_msg());
        let hex = |b: &[u8]| b.iter().map(|x| format!("0x{x:02x}, ")).collect::<String>();
        println!("public: {}", hex(&key.verifying_key().to_bytes()));
        println!("sig: {}", hex(&sig.to_bytes()));
    }
}

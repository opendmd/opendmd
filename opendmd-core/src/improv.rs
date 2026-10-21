//! Improv Serial (<https://www.improv-wifi.com/serial/>): the framing and
//! the RPCs a browser installer (ESP Web Tools) speaks over the USB serial
//! port to find out what a board runs and to give it WiFi details. The
//! pure part, tested here; the firmware's `improv.rs` reads the UART and
//! answers.
//!
//! A frame, either way:
//!
//! ```text
//! I M P R O V | version (1) | type | length | data (length bytes) | checksum
//! ```
//!
//! The checksum is the low byte of the sum of every byte before it, header
//! included. The host ends its frames with `\n`; so do we.
//!
//! Types: 1 current state (device → host, one byte), 2 error state (device
//! → host, one byte), 3 RPC (host → device: command, length, payload), 4
//! RPC result (device → host: command, length, then length-prefixed
//! strings).
//!
//! **Two things about the host's reader** (`improv-wifi-serial-sdk` 2.8,
//! what ESP Web Tools 10.4 uses), checked against its source:
//!
//! - It only looks for `IMPROV` at the start of a line: after a `\n`, or
//!   after the previous frame. A frame that starts mid-line is lost, so
//!   frames are only ever sent whole, between log lines (the firmware puts
//!   them in its log ring as records of their own).
//! - While it reads the nine header bytes, a byte 10 (`\n`) restarts the
//!   line, and the ninth byte is the data length: **a frame whose data is
//!   exactly 10 bytes long is dropped**. [`encode_rpc_result`] pads such a
//!   result with one byte past the strings (the reader stops at the
//!   strings' total, so it never sees it). The other frames we send are
//!   one byte of data.
//!
//!   **This deviates from the spec**, which has an RPC result's data be
//!   exactly its strings: the padded frame's length byte (and checksum)
//!   covers one byte more than the strings do. The SDK reads the strings
//!   by their own lengths and ignores the rest; a host that insisted the
//!   lengths match exactly would reject these frames (rare: a scan entry
//!   or URL whose strings total 8 bytes). We know of no such host.

/// The frame's first six bytes.
pub const HEADER: &[u8; 6] = b"IMPROV";
/// The protocol version we speak (and the only one we accept).
pub const VERSION: u8 = 1;

/// Frame types.
pub const TYPE_CURRENT_STATE: u8 = 0x01;
pub const TYPE_ERROR_STATE: u8 = 0x02;
pub const TYPE_RPC: u8 = 0x03;
pub const TYPE_RPC_RESULT: u8 = 0x04;

/// RPC commands.
pub const CMD_WIFI_SETTINGS: u8 = 0x01;
pub const CMD_CURRENT_STATE: u8 = 0x02;
pub const CMD_DEVICE_INFO: u8 = 0x03;
pub const CMD_SCAN: u8 = 0x04;

/// The longest data a frame can carry (its length is one byte).
pub const MAX_DATA: usize = 255;
/// Header, version, type, length, the most data, checksum: a whole frame
/// as received (the host's trailing `\n` is not part of it).
pub const MAX_FRAME: usize = HEADER.len() + 3 + MAX_DATA + 1;
/// The longest SSID (802.11) and WPA passphrase we take.
pub const MAX_SSID: usize = 32;
pub const MAX_PASSWORD: usize = 64;

/// The device's state, as the current-state frame reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum State {
    /// Ready to take WiFi details ("authorized": no button to press here).
    Ready = 0x02,
    /// Trying the WiFi details it was just sent.
    Provisioning = 0x03,
    /// On a network; the RPC result that follows carries the device's URL.
    Provisioned = 0x04,
}

/// The error-state frame's codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Error {
    None = 0x00,
    /// A frame or RPC that does not parse (bad checksum, lengths that do
    /// not add up, an SSID that is too long, …).
    InvalidRpc = 0x01,
    UnknownRpc = 0x02,
    /// The WiFi details did not get the device onto the network.
    UnableToConnect = 0x03,
    Unknown = 0xFF,
}

/// The low byte of the sum of `bytes`.
pub fn checksum(bytes: &[u8]) -> u8 {
    bytes.iter().fold(0u8, |s, &b| s.wrapping_add(b))
}

/// What [`Parser::push`] found.
#[derive(Debug, PartialEq, Eq)]
pub enum Event<'a> {
    /// A whole frame with a good checksum: its type and data.
    Frame { kind: u8, data: &'a [u8] },
    /// A whole frame whose checksum is wrong (answer [`Error::InvalidRpc`]).
    BadChecksum,
}

/// Finds frames in a byte stream that also carries anything else (a
/// terminal's keystrokes, line noise): bytes that are not part of a frame
/// are skipped, and `IMPROV` is looked for anywhere. No allocation; the
/// firmware keeps one in RTC slow memory.
pub struct Parser {
    buf: [u8; MAX_FRAME],
    len: usize,
    /// The last push completed a frame: start over on the next byte.
    done: bool,
}

impl Default for Parser {
    fn default() -> Self {
        Self::new()
    }
}

impl Parser {
    pub const fn new() -> Self {
        Parser {
            buf: [0; MAX_FRAME],
            len: 0,
            done: false,
        }
    }

    /// Forget any partial frame.
    pub fn reset(&mut self) {
        self.len = 0;
        self.done = false;
    }

    /// Take one byte; `Some` when it completes a frame.
    pub fn push(&mut self, byte: u8) -> Option<Event<'_>> {
        if self.done {
            self.reset();
        }
        let at = self.len;
        if at < HEADER.len() {
            if byte == HEADER[at] {
                self.buf[at] = byte;
                self.len += 1;
            } else if byte == HEADER[0] {
                // `IMPRIMPROV`: the second I starts over.
                self.buf[0] = byte;
                self.len = 1;
            } else {
                self.len = 0;
            }
            return None;
        }
        if at == HEADER.len() && byte != VERSION {
            // Another protocol version, or `IMPROV` in ordinary text.
            self.len = usize::from(byte == HEADER[0]);
            return None;
        }
        self.buf[at] = byte;
        self.len += 1;
        // Header, version, type, length are in: the data length is known.
        let data_len = if self.len > 8 {
            usize::from(self.buf[8])
        } else {
            return None;
        };
        let total = 9 + data_len + 1;
        if self.len < total {
            return None;
        }
        self.done = true;
        let sum = checksum(&self.buf[..total - 1]);
        if sum != self.buf[total - 1] {
            return Some(Event::BadChecksum);
        }
        Some(Event::Frame {
            kind: self.buf[7],
            data: &self.buf[9..9 + data_len],
        })
    }
}

/// A parsed RPC (a type-3 frame's data).
#[derive(Debug, PartialEq, Eq)]
pub enum Rpc<'a> {
    WifiSettings {
        ssid: &'a str,
        password: &'a str,
    },
    CurrentState,
    DeviceInfo,
    Scan,
    /// A command we do not implement (hostname, device name, network
    /// state, …): answer [`Error::UnknownRpc`].
    Other(u8),
}

/// Parse an RPC frame's data: `command, length, payload` with the length
/// matching the payload exactly. Anything malformed is
/// [`Error::InvalidRpc`].
pub fn parse_rpc(data: &[u8]) -> Result<Rpc<'_>, Error> {
    let [cmd, len, payload @ ..] = data else {
        return Err(Error::InvalidRpc);
    };
    if usize::from(*len) != payload.len() {
        return Err(Error::InvalidRpc);
    }
    Ok(match *cmd {
        CMD_WIFI_SETTINGS => {
            let (ssid, rest) = take_string(payload)?;
            let (password, rest) = take_string(rest)?;
            if !rest.is_empty()
                || ssid.is_empty()
                || ssid.len() > MAX_SSID
                || password.len() > MAX_PASSWORD
            {
                return Err(Error::InvalidRpc);
            }
            Rpc::WifiSettings { ssid, password }
        }
        CMD_CURRENT_STATE => Rpc::CurrentState,
        CMD_DEVICE_INFO => Rpc::DeviceInfo,
        CMD_SCAN => Rpc::Scan,
        other => Rpc::Other(other),
    })
}

/// One length-prefixed UTF-8 string off the front of `b`.
fn take_string(b: &[u8]) -> Result<(&str, &[u8]), Error> {
    let (&n, rest) = b.split_first().ok_or(Error::InvalidRpc)?;
    let n = usize::from(n);
    if rest.len() < n {
        return Err(Error::InvalidRpc);
    }
    let s = core::str::from_utf8(&rest[..n]).map_err(|_| Error::InvalidRpc)?;
    Ok((s, &rest[n..]))
}

/// Bytes a frame with `data_len` bytes of data takes on the wire, its
/// trailing `\n` included.
pub const fn frame_len(data_len: usize) -> usize {
    HEADER.len() + 3 + data_len + 1 + 1
}

/// Write a frame of `kind` around `data` (≤ [`MAX_DATA`] bytes) into `out`
/// (≥ [`frame_len`]`(data.len())` bytes); returns the length written.
pub fn encode(kind: u8, data: &[u8], out: &mut [u8]) -> usize {
    debug_assert!(data.len() <= MAX_DATA);
    let n = frame_len(data.len());
    out[..6].copy_from_slice(HEADER);
    out[6] = VERSION;
    out[7] = kind;
    out[8] = data.len() as u8;
    out[9..9 + data.len()].copy_from_slice(data);
    out[n - 2] = checksum(&out[..n - 2]);
    out[n - 1] = b'\n';
    n
}

/// The current-state frame's length ([`frame_len`] of one byte).
pub const STATE_FRAME: usize = frame_len(1);

/// A current-state frame.
pub fn encode_state(state: State, out: &mut [u8]) -> usize {
    encode(TYPE_CURRENT_STATE, &[state as u8], out)
}

/// An error-state frame.
pub fn encode_error(error: Error, out: &mut [u8]) -> usize {
    encode(TYPE_ERROR_STATE, &[error as u8], out)
}

/// The strings an RPC result carries, each cut to fit: a string is at most
/// 255 bytes and all of them, with their length bytes, at most 253 (the
/// frame's data also holds the command and the total). A string is cut at
/// a character boundary.
fn fitted<'a>(strings: &'a [&'a str]) -> impl Iterator<Item = &'a [u8]> + 'a {
    let mut room = MAX_DATA - 2 - 1; // command, total; one byte of padding
    strings.iter().map(move |s| {
        let mut n = s.len().min(room.saturating_sub(1));
        while !s.is_char_boundary(n) {
            n -= 1;
        }
        room = room.saturating_sub(n + 1);
        &s.as_bytes()[..n]
    })
}

/// The data length of an RPC result with these strings, before padding.
fn rpc_result_data(strings: &[&str]) -> usize {
    2 + fitted(strings).map(|s| 1 + s.len()).sum::<usize>()
}

/// Bytes [`encode_rpc_result`] writes for these strings.
pub fn rpc_result_len(strings: &[&str]) -> usize {
    let data = rpc_result_data(strings);
    frame_len(if data == 10 { 11 } else { data })
}

/// An RPC result frame: `command, total length, (length, bytes)*`, padded
/// past the strings when its data would be exactly 10 bytes (see the
/// module notes). `out` must hold [`rpc_result_len`] bytes. A result with
/// no strings ends a scan's list.
pub fn encode_rpc_result(cmd: u8, strings: &[&str], out: &mut [u8]) -> usize {
    let data = rpc_result_data(strings);
    let padded = if data == 10 { 11 } else { data };
    let n = frame_len(padded);
    out[..6].copy_from_slice(HEADER);
    out[6] = VERSION;
    out[7] = TYPE_RPC_RESULT;
    out[8] = padded as u8;
    out[9] = cmd;
    out[10] = (data - 2) as u8;
    let mut at = 11;
    for s in fitted(strings) {
        out[at] = s.len() as u8;
        out[at + 1..at + 1 + s.len()].copy_from_slice(s);
        at += 1 + s.len();
    }
    if padded != data {
        out[at] = 0;
    }
    out[n - 2] = checksum(&out[..n - 2]);
    out[n - 1] = b'\n';
    n
}

/// `http://a.b.c.d/` for an IPv4 address (octets in network order), into
/// `out` (≥ 23 bytes); returns the length.
pub fn url_for_ip(ip: [u8; 4], out: &mut [u8]) -> usize {
    let mut at = 0;
    let mut put = |b: &[u8]| {
        out[at..at + b.len()].copy_from_slice(b);
        at += b.len();
    };
    put(b"http://");
    for (i, o) in ip.iter().enumerate() {
        if i > 0 {
            put(b".");
        }
        let digits = [o / 100, o / 10 % 10, o % 10];
        let skip = if *o >= 100 {
            0
        } else if *o >= 10 {
            1
        } else {
            2
        };
        for d in &digits[skip..] {
            put(&[b'0' + d]);
        }
    }
    put(b"/");
    at
}

/// Decides when a console running faster than 115200 baud is being talked
/// to at 115200 (ESP Web Tools opens the port at 115200 and nothing else):
/// bytes sent at an eighth of the receiver's rate arrive as framing errors
/// and breaks (the zero bits are eight bit-times long) plus a burst of
/// junk bytes. One glitch (a cable plugged in, the USB chip powering up)
/// is one error and a byte or two, so the rule wants both: an error and at
/// least [`Self::MIN_BYTES`] bytes over the last [`Self::WINDOW`] polls. A
/// terminal at the right rate produces no errors at all, so typing into it
/// never trips this.
#[derive(Debug, Default)]
pub struct BaudSniffer {
    bytes: [u16; Self::WINDOW],
    errors: [bool; Self::WINDOW],
    at: usize,
}

impl BaudSniffer {
    /// Polls remembered (the firmware polls every 5 ms: 20 ms).
    pub const WINDOW: usize = 4;
    /// Junk bytes wanted alongside an error. A request-current-state frame
    /// (11 bytes at 115200) arrives at 921600 as ~11-20 bytes.
    pub const MIN_BYTES: usize = 4;

    pub const fn new() -> Self {
        BaudSniffer {
            bytes: [0; Self::WINDOW],
            errors: [false; Self::WINDOW],
            at: 0,
        }
    }

    /// One poll: `bytes` received since the last one, and whether a framing
    /// error, parity error or break was flagged. `true` = switch to 115200.
    pub fn poll(&mut self, bytes: usize, error: bool) -> bool {
        self.bytes[self.at] = bytes.min(u16::MAX as usize) as u16;
        self.errors[self.at] = error;
        self.at = (self.at + 1) % Self::WINDOW;
        let total: usize = self.bytes.iter().map(|&b| usize::from(b)).sum();
        self.errors.iter().any(|&e| e) && total >= Self::MIN_BYTES
    }
}

/// What [`BaudSwitch::poll`] wants done with the console's rate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BaudStep {
    /// Leave it.
    Stay,
    /// Switch to 115200: a host seems to be talking at that rate.
    ToImprov,
    /// Back to the console's own rate: no Improv frame came in time.
    Revert,
}

/// The console's rate over time, from [`BaudSniffer`]'s guess to a
/// confirmed host. A switch to 115200 is provisional: unless a valid
/// Improv frame ([`Self::frame`]) arrives within [`Self::TRIAL_POLLS`],
/// the console goes back to its own rate (something else made the junk:
/// a terminal at the wrong rate, line noise). Once a frame has arrived it
/// stays at 115200 until restart, so the installer's "Logs & console"
/// keeps working.
#[derive(Debug, Default)]
pub struct BaudSwitch {
    sniffer: BaudSniffer,
    /// 0: at the console's rate; `u16::MAX`: confirmed; else the polls
    /// left in the trial.
    trial: u16,
}

impl BaudSwitch {
    /// The trial (5 ms polls: 3 s). The host repeats its first request
    /// every second, so a real installer is heard well within it.
    pub const TRIAL_POLLS: u16 = 600;
    const CONFIRMED: u16 = u16::MAX;

    pub const fn new() -> Self {
        BaudSwitch {
            sniffer: BaudSniffer::new(),
            trial: 0,
        }
    }

    /// One poll, as [`BaudSniffer::poll`] takes it. Report the frames in
    /// this poll's bytes ([`Self::frame`]) first: a frame in the trial's
    /// last poll then confirms it rather than arriving after the revert.
    pub fn poll(&mut self, bytes: usize, error: bool) -> BaudStep {
        match self.trial {
            Self::CONFIRMED => BaudStep::Stay,
            0 => {
                if self.sniffer.poll(bytes, error) {
                    self.trial = Self::TRIAL_POLLS;
                    BaudStep::ToImprov
                } else {
                    BaudStep::Stay
                }
            }
            1 => {
                self.trial = 0;
                self.sniffer = BaudSniffer::new();
                BaudStep::Revert
            }
            _ => {
                self.trial -= 1;
                BaudStep::Stay
            }
        }
    }

    /// A valid Improv frame arrived. `true` if that confirmed a trial.
    pub fn frame(&mut self) -> bool {
        let confirms = self.trial != 0 && self.trial != Self::CONFIRMED;
        if confirms {
            self.trial = Self::CONFIRMED;
        }
        confirms
    }

    /// The console is at 115200 (on trial or confirmed).
    pub fn slow(&self) -> bool {
        self.trial != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The SDK's `writePacketToStream` (improv-wifi-serial-sdk 2.8.1,
    /// `dist/serial.js`), byte for byte: header, version, type, length,
    /// data, checksum over all of that, `\n`.
    fn sdk_write(kind: u8, data: &[u8]) -> Vec<u8> {
        let mut p: Vec<u8> = b"IMPROV".to_vec();
        p.push(1);
        p.push(kind);
        p.push(data.len() as u8);
        p.extend_from_slice(data);
        p.push(0);
        p.push(0);
        let n = p.len();
        p[n - 2] = (p.iter().map(|&b| b as u32).sum::<u32>() & 0xff) as u8;
        p[n - 1] = 10;
        p
    }

    /// The SDK's `_sendRPC(command, data)`.
    fn sdk_rpc(cmd: u8, data: &[u8]) -> Vec<u8> {
        let mut d = vec![cmd, data.len() as u8];
        d.extend_from_slice(data);
        sdk_write(TYPE_RPC, &d)
    }

    /// What the SDK's reader (`_processInput` + `_handleIncomingPacket`)
    /// makes of a byte stream: each packet it accepts, as `(type, data)`,
    /// with RPC results decoded into their strings. Ported line for line,
    /// quirks included.
    #[derive(Debug, PartialEq)]
    enum Got {
        State(u8),
        Error(u8),
        Result(u8, Vec<String>),
    }

    fn sdk_read(stream: &[u8]) -> Vec<Got> {
        let mut out = Vec::new();
        let mut line: Vec<u8> = Vec::new();
        let mut is_improv: Option<bool> = None;
        let mut improv_len = 0usize;
        for &byte in stream {
            if is_improv == Some(false) {
                if byte == 10 {
                    is_improv = None;
                }
                continue;
            }
            if is_improv == Some(true) {
                line.push(byte);
                if line.len() == improv_len {
                    if let Some(g) = sdk_handle(&line) {
                        out.push(g);
                    }
                    is_improv = None;
                    line.clear();
                }
                continue;
            }
            if byte == 10 {
                line.clear();
                continue;
            }
            line.push(byte);
            if line.len() != 9 {
                continue;
            }
            is_improv = Some(&line[..6] == b"IMPROV");
            if is_improv != Some(true) {
                line.clear();
                continue;
            }
            improv_len = 9 + line[8] as usize + 1;
        }
        out
    }

    fn sdk_handle(line: &[u8]) -> Option<Got> {
        let payload = &line[6..];
        let (version, kind, len) = (payload[0], payload[1], payload[2] as usize);
        let data = &payload[3..3 + len];
        if version != 1 {
            return None;
        }
        let sum = (line[..line.len() - 1]
            .iter()
            .map(|&b| b as u32)
            .sum::<u32>()
            & 0xff) as u8;
        if sum != payload[3 + len] {
            return None;
        }
        match kind {
            1 => Some(Got::State(data[0])),
            2 => Some(Got::Error(data[0])),
            4 => {
                let total = data[1] as usize;
                let mut idx = 2;
                let mut strings = Vec::new();
                while idx < 2 + total {
                    let n = data[idx] as usize;
                    strings.push(String::from_utf8(data[idx + 1..idx + 1 + n].to_vec()).unwrap());
                    idx += n + 1;
                }
                Some(Got::Result(data[0], strings))
            }
            _ => None,
        }
    }

    fn frames(stream: &[u8]) -> Vec<(u8, Vec<u8>)> {
        let mut p = Parser::new();
        let mut out = Vec::new();
        for &b in stream {
            match p.push(b) {
                Some(Event::Frame { kind, data }) => out.push((kind, data.to_vec())),
                Some(Event::BadChecksum) => out.push((0, vec![])),
                None => {}
            }
        }
        out
    }

    fn rpcs(stream: &[u8]) -> Vec<Result<String, Error>> {
        frames(stream)
            .into_iter()
            .map(|(kind, data)| {
                if kind == 0 {
                    return Err(Error::InvalidRpc);
                }
                assert_eq!(kind, TYPE_RPC);
                parse_rpc(&data).map(|r| format!("{r:?}"))
            })
            .collect()
    }

    #[test]
    fn checksum_wraps() {
        assert_eq!(checksum(b""), 0);
        assert_eq!(checksum(&[0xff, 0x02]), 0x01);
        // The SDK's request-current-state frame, worked by hand:
        // I M P R O V = 0x49+0x4d+0x50+0x52+0x4f+0x56 = 0x1dd; +1+3+2+2+0 = 0x1e5.
        let f = sdk_rpc(CMD_CURRENT_STATE, &[]);
        assert_eq!(f, b"IMPROV\x01\x03\x02\x02\x00\xe5\n");
    }

    #[test]
    fn parses_what_the_sdk_sends() {
        let mut s = Vec::new();
        s.extend(sdk_rpc(CMD_CURRENT_STATE, &[]));
        s.extend(sdk_rpc(CMD_DEVICE_INFO, &[]));
        s.extend(sdk_rpc(CMD_SCAN, &[]));
        // provision("Home WiFi", "pässword"): lengths are UTF-8 bytes.
        let (ssid, pw) = ("Home WiFi", "pässword");
        let mut d = vec![ssid.len() as u8];
        d.extend_from_slice(ssid.as_bytes());
        d.push(pw.len() as u8);
        d.extend_from_slice(pw.as_bytes());
        s.extend(sdk_rpc(CMD_WIFI_SETTINGS, &d));
        s.extend(sdk_rpc(0x05, b"newname")); // set hostname: not ours
        assert_eq!(
            rpcs(&s),
            vec![
                Ok("CurrentState".into()),
                Ok("DeviceInfo".into()),
                Ok("Scan".into()),
                Ok(r#"WifiSettings { ssid: "Home WiFi", password: "pässword" }"#.into()),
                Ok("Other(5)".into()),
            ]
        );
    }

    #[test]
    fn finds_frames_among_noise() {
        let mut s = b"garbage\x00\xffIMPR".to_vec(); // a false start
        s.extend(b"IMPRIMPROV\x02"); // wrong version, then a restart below
        s.extend(sdk_rpc(CMD_DEVICE_INFO, &[]));
        s.extend(b"\r\nIMPROVE yourself\n");
        s.extend(sdk_rpc(CMD_SCAN, &[]));
        assert_eq!(rpcs(&s), vec![Ok("DeviceInfo".into()), Ok("Scan".into())]);
        // A frame split anywhere arrives the same.
        let f = sdk_rpc(CMD_CURRENT_STATE, &[]);
        let mut p = Parser::new();
        let mut got = 0;
        for b in f.iter().chain(f.iter()) {
            if let Some(Event::Frame { kind: TYPE_RPC, .. }) = p.push(*b) {
                got += 1;
            }
        }
        assert_eq!(got, 2);
    }

    #[test]
    fn malformed_input() {
        // Bad checksum.
        let mut f = sdk_rpc(CMD_CURRENT_STATE, &[]);
        let n = f.len();
        f[n - 2] ^= 1;
        assert_eq!(rpcs(&f), vec![Err(Error::InvalidRpc)]);
        // The parser recovers for the next frame.
        f.extend(sdk_rpc(CMD_SCAN, &[]));
        assert_eq!(rpcs(&f)[1], Ok("Scan".into()));
        // RPC lengths that do not add up.
        assert_eq!(parse_rpc(&[]), Err(Error::InvalidRpc));
        assert_eq!(parse_rpc(&[CMD_SCAN]), Err(Error::InvalidRpc));
        assert_eq!(parse_rpc(&[CMD_SCAN, 1]), Err(Error::InvalidRpc));
        assert_eq!(parse_rpc(&[CMD_SCAN, 0, 9]), Err(Error::InvalidRpc));
        // WiFi settings: truncated, trailing bytes, empty or long SSID,
        // long password, not UTF-8.
        let wifi = |p: &[u8]| {
            let mut d = vec![CMD_WIFI_SETTINGS, p.len() as u8];
            d.extend_from_slice(p);
            parse_rpc(&d).map(|r| format!("{r:?}"))
        };
        assert_eq!(wifi(&[3, b'a', b'b']), Err(Error::InvalidRpc));
        assert_eq!(wifi(&[1, b'a']), Err(Error::InvalidRpc));
        assert_eq!(wifi(&[1, b'a', 0, 7]), Err(Error::InvalidRpc));
        assert_eq!(wifi(&[0, 0]), Err(Error::InvalidRpc));
        let mut long = vec![33];
        long.extend([b'a'; 33]);
        long.push(0);
        assert_eq!(wifi(&long), Err(Error::InvalidRpc));
        let mut pw = vec![1, b'a', 65];
        pw.extend([b'p'; 65]);
        assert_eq!(wifi(&pw), Err(Error::InvalidRpc));
        assert_eq!(wifi(&[1, 0xff, 0]), Err(Error::InvalidRpc));
        // An open network: an empty password is fine.
        assert_eq!(
            wifi(&[2, b'h', b'i', 0]),
            Ok(r#"WifiSettings { ssid: "hi", password: "" }"#.into())
        );
        // The longest legal ones.
        let mut max = vec![32];
        max.extend([b's'; 32]);
        max.push(64);
        max.extend([b'p'; 64]);
        assert!(wifi(&max).is_ok_and(|r| r.starts_with("WifiSettings")));
    }

    #[test]
    fn a_data_length_of_255_is_a_whole_frame() {
        let data = [7u8; 255];
        let mut out = [0u8; frame_len(255)];
        let n = encode(TYPE_RPC, &data, &mut out);
        assert_eq!(n, MAX_FRAME + 1);
        let got = frames(&out[..n]);
        assert_eq!(got, vec![(TYPE_RPC, data.to_vec())]);
    }

    fn result(cmd: u8, strings: &[&str]) -> Vec<u8> {
        let mut out = vec![0u8; rpc_result_len(strings)];
        let n = encode_rpc_result(cmd, strings, &mut out);
        assert_eq!(n, out.len());
        out
    }

    #[test]
    fn the_sdk_reads_what_we_send() {
        let mut s = b"I (1234) opendmd_firmware: a log line\n".to_vec();
        let mut buf = [0u8; STATE_FRAME];
        let n = encode_state(State::Provisioned, &mut buf);
        s.extend_from_slice(&buf[..n]);
        s.extend(result(CMD_CURRENT_STATE, &["http://192.168.1.69/"]));
        s.extend(b"W (1240) wifi: another line\n");
        s.extend(result(
            CMD_DEVICE_INFO,
            &["OpenDMD", "0.9.0", "ESP32", "opendmd"],
        ));
        s.extend(result(CMD_SCAN, &["Home", "-52", "YES"]));
        s.extend(result(CMD_SCAN, &["Café", "-80", "NO"]));
        s.extend(result(CMD_SCAN, &[]));
        let n = encode_error(Error::UnableToConnect, &mut buf);
        s.extend_from_slice(&buf[..n]);
        let n = encode_state(State::Ready, &mut buf);
        s.extend_from_slice(&buf[..n]);
        let st = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            sdk_read(&s),
            vec![
                Got::State(4),
                Got::Result(2, st(&["http://192.168.1.69/"])),
                Got::Result(3, st(&["OpenDMD", "0.9.0", "ESP32", "opendmd"])),
                Got::Result(4, st(&["Home", "-52", "YES"])),
                Got::Result(4, st(&["Café", "-80", "NO"])),
                Got::Result(4, vec![]),
                Got::Error(3),
                Got::State(2),
            ]
        );
    }

    #[test]
    fn ten_bytes_of_data_are_padded() {
        // A result whose data would be 10 bytes: cmd, total, len + 7 bytes.
        // Unpadded, the SDK's reader takes the length byte (10) for a
        // newline and drops the frame.
        let mut d = vec![CMD_SCAN, 8, 7];
        d.extend(b"seven!!");
        let unpadded = sdk_write(TYPE_RPC_RESULT, &d);
        assert!(sdk_read(&unpadded).is_empty(), "the quirk is real");
        let f = result(CMD_SCAN, &["seven!!"]);
        assert_eq!(f[8], 11);
        assert_eq!(sdk_read(&f), vec![Got::Result(4, vec!["seven!!".into()])]);
        // Every short result length goes through.
        for n in 0..40 {
            let s = "x".repeat(n);
            assert_eq!(
                sdk_read(&result(CMD_SCAN, &[&s, "-1"])),
                vec![Got::Result(4, vec![s.clone(), "-1".into()])],
                "string of {n}"
            );
        }
    }

    #[test]
    fn long_strings_are_cut_to_fit() {
        let long = "é".repeat(200); // 400 bytes
        let f = result(CMD_DEVICE_INFO, &["OpenDMD", &long, "ESP32"]);
        assert!(f.len() <= frame_len(MAX_DATA));
        let got = sdk_read(&f);
        let Got::Result(3, s) = &got[0] else {
            panic!("{got:?}")
        };
        assert_eq!(s[0], "OpenDMD");
        assert!(s[1].chars().all(|c| c == 'é') && !s[1].is_empty());
        assert_eq!(s.len(), 3);
    }

    #[test]
    fn urls() {
        let mut b = [0u8; 23];
        let n = url_for_ip([192, 168, 1, 69], &mut b);
        assert_eq!(&b[..n], b"http://192.168.1.69/");
        let n = url_for_ip([255, 255, 255, 255], &mut b);
        assert_eq!(&b[..n], b"http://255.255.255.255/");
        let n = url_for_ip([10, 0, 7, 100], &mut b);
        assert_eq!(&b[..n], b"http://10.0.7.100/");
    }

    #[test]
    fn baud_sniffer() {
        let mut s = BaudSniffer::new();
        // A glitch: one error, one byte.
        assert!(!s.poll(1, true));
        for _ in 0..BaudSniffer::WINDOW {
            assert!(!s.poll(0, false));
        }
        // Typing at the right rate: bytes, never an error.
        for _ in 0..20 {
            assert!(!s.poll(5, false));
        }
        // An 11-byte frame at an eighth of the rate: errors and junk.
        let mut s = BaudSniffer::new();
        assert!(!s.poll(2, true));
        assert!(s.poll(9, true));
        // Spread over polls inside the window.
        let mut s = BaudSniffer::new();
        assert!(!s.poll(0, true));
        assert!(!s.poll(2, false));
        assert!(s.poll(2, false));
        // ...but not outside it.
        let mut s = BaudSniffer::new();
        assert!(!s.poll(0, true));
        for _ in 0..BaudSniffer::WINDOW {
            assert!(!s.poll(1, false));
        }
    }

    #[test]
    fn baud_switch_is_provisional() {
        let trip = |b: &mut BaudSwitch| {
            assert_eq!(b.poll(2, true), BaudStep::Stay);
            assert_eq!(b.poll(9, true), BaudStep::ToImprov);
        };
        // No frame within the trial: back, on the last poll of it.
        let mut b = BaudSwitch::new();
        assert!(!b.frame(), "a frame at the console's rate confirms nothing");
        trip(&mut b);
        assert!(b.slow());
        for _ in 1..BaudSwitch::TRIAL_POLLS {
            assert_eq!(b.poll(3, true), BaudStep::Stay, "junk during the trial");
        }
        assert_eq!(b.poll(0, false), BaudStep::Revert);
        assert!(!b.slow());
        // The sniffer starts afresh: the trial's junk does not count.
        assert_eq!(b.poll(0, false), BaudStep::Stay);
        // A second trip, confirmed by a frame: sticky for good.
        trip(&mut b);
        for _ in 0..100 {
            assert_eq!(b.poll(0, false), BaudStep::Stay);
        }
        assert!(b.frame());
        assert!(!b.frame(), "confirmed once");
        for _ in 0..3 * BaudSwitch::TRIAL_POLLS {
            assert_eq!(b.poll(5, true), BaudStep::Stay);
        }
        assert!(b.slow());
        // A frame in the bytes of the trial's very last poll still counts:
        // the firmware parses a poll's bytes (frame) before poll().
        let mut b = BaudSwitch::new();
        trip(&mut b);
        for _ in 1..BaudSwitch::TRIAL_POLLS {
            assert_eq!(b.poll(0, false), BaudStep::Stay);
        }
        assert!(b.frame());
        assert_eq!(b.poll(12, false), BaudStep::Stay);
        assert!(b.slow());
        // The same poll without a frame is the revert.
        let mut b = BaudSwitch::new();
        trip(&mut b);
        for _ in 1..BaudSwitch::TRIAL_POLLS {
            b.poll(0, false);
        }
        assert_eq!(b.poll(12, false), BaudStep::Revert);
        assert!(!b.frame(), "too late: back at the console's rate");
    }
}

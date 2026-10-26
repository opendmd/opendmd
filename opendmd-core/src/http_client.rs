//! A minimal HTTP/1.1 client for one plain-HTTP `GET` (Follow RePlayOS's
//! poll, `firmware/src/follow.rs`), over any `Read + Write` stream: the
//! firmware hands it a `std::net::TcpStream`, the tests a scripted one.
//!
//! It exists so the firmware links no `esp_http_client` (and with it no
//! esp-tls and no TLS client; `docs/research/size-budget.md` 1.5). What it
//! does: writes the request (`Connection: close`), reads the status line
//! and the headers a line at a time (only `Content-Length` and
//! `Transfer-Encoding: chunked` are kept), then hands the body to a
//! callback as it arrives: by `Content-Length`, chunked, or up to the
//! server's close. Nothing is buffered beyond one header line (cut at
//! [`MAX_LINE`] bytes) and the caller's read buffer, so a reply of any size
//! is read in the same few hundred bytes. No redirects, no keep-alive, no
//! TLS, no compression.

use std::io::{self, Read, Write};
use std::time::Instant;

/// A header line longer than this is cut (only its start, the name and a
/// short value, is ever looked at).
pub const MAX_LINE: usize = 256;
/// The status line and headers together may not be longer than this, nor
/// may a chunked body's trailers.
pub const MAX_HEAD: usize = 8 * 1024;

/// Why a `GET` did not produce a status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// A read or write timed out (`WouldBlock`/`TimedOut`: a socket timeout).
    Timeout,
    /// The connection closed before a complete status line and headers.
    NoReply,
    /// The connection closed in the middle of a body whose length was known
    /// (`Content-Length`, or chunked without its last chunk).
    Dropped,
    /// Not an HTTP/1.x reply, or a framing error (what was wrong).
    Malformed(&'static str),
    /// Any other I/O error.
    Io(io::ErrorKind),
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        match e.kind() {
            io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut => Error::Timeout,
            io::ErrorKind::UnexpectedEof
            | io::ErrorKind::ConnectionReset
            | io::ErrorKind::ConnectionAborted
            | io::ErrorKind::BrokenPipe => Error::Dropped,
            k => Error::Io(k),
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Timeout => f.write_str("timed out"),
            Error::NoReply => f.write_str("no reply"),
            Error::Dropped => f.write_str("connection dropped"),
            Error::Malformed(what) => write!(f, "bad reply ({what})"),
            Error::Io(k) => write!(f, "error {k}"),
        }
    }
}

/// The request bytes: `GET <path> HTTP/1.1` with `Host`, `Connection:
/// close` and `headers`. CR, LF and other control characters are dropped
/// from every value, so a saved code cannot add a header of its own.
pub fn request(host: &str, path: &str, headers: &[(&str, &str)]) -> String {
    fn clean(s: &str) -> impl Iterator<Item = char> + '_ {
        s.chars().filter(|c| !c.is_control())
    }
    let mut r = String::with_capacity(96 + path.len() + host.len());
    r.push_str("GET ");
    r.extend(clean(path));
    r.push_str(" HTTP/1.1\r\nHost: ");
    r.extend(clean(host));
    r.push_str("\r\n");
    for (name, value) in headers {
        r.extend(clean(name));
        r.push_str(": ");
        r.extend(clean(value));
        r.push_str("\r\n");
    }
    r.push_str("Connection: close\r\n\r\n");
    r
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// Reading the status line.
    Status,
    /// Reading header lines.
    Headers,
    /// `Content-Length` body: this many bytes to go.
    Fixed(u64),
    /// Read to the close.
    ToClose,
    /// Chunked: reading a chunk-size line.
    ChunkSize,
    /// Chunked: inside a chunk, this many bytes to go.
    Chunk(u64),
    /// Chunked: the CRLF after a chunk's data.
    ChunkEnd,
    /// Chunked: trailer lines after the last chunk.
    Trailers,
    /// The whole reply is in.
    Done,
}

/// The reply, read incrementally ([`ResponseReader::feed`]); no I/O of its
/// own.
#[derive(Debug)]
pub struct ResponseReader {
    state: State,
    line: Vec<u8>,
    /// Bytes of status line and headers so far (capped at [`MAX_HEAD`]).
    head_bytes: usize,
    /// Bytes of chunked trailers so far (capped at [`MAX_HEAD`]).
    trailer_bytes: usize,
    status: u16,
    content_length: Option<u64>,
    chunked: bool,
}

impl Default for ResponseReader {
    fn default() -> Self {
        ResponseReader::new()
    }
}

impl ResponseReader {
    pub fn new() -> Self {
        ResponseReader {
            state: State::Status,
            line: Vec::new(),
            head_bytes: 0,
            trailer_bytes: 0,
            status: 0,
            content_length: None,
            chunked: false,
        }
    }

    /// The status, once the status line has been read.
    pub fn status(&self) -> Option<u16> {
        (self.state != State::Status).then_some(self.status)
    }

    /// The status line and every header have been read.
    pub fn head_done(&self) -> bool {
        !matches!(self.state, State::Status | State::Headers)
    }

    /// The body is complete by its framing (nothing more to read).
    pub fn is_complete(&self) -> bool {
        self.state == State::Done
    }

    /// The next bytes of the reply; the body's bytes go to `body`. Bytes
    /// after the end of the reply are ignored.
    pub fn feed(&mut self, mut input: &[u8], body: &mut dyn FnMut(&[u8])) -> Result<(), Error> {
        while !input.is_empty() {
            match self.state {
                State::Done => return Ok(()),
                State::Fixed(left) | State::Chunk(left) => {
                    let n = (left.min(input.len() as u64)) as usize;
                    body(&input[..n]);
                    input = &input[n..];
                    let left = left - n as u64;
                    self.state = match (self.state, left) {
                        (State::Fixed(_), 0) => State::Done,
                        (State::Fixed(_), l) => State::Fixed(l),
                        (_, 0) => State::ChunkEnd,
                        (_, l) => State::Chunk(l),
                    };
                }
                State::ToClose => {
                    body(input);
                    return Ok(());
                }
                _ => {
                    // A line-oriented state: up to the next LF.
                    let (part, rest, ended) = match input.iter().position(|&b| b == b'\n') {
                        Some(i) => (&input[..i], &input[i + 1..], true),
                        None => (input, &[][..], false),
                    };
                    input = rest;
                    let len = part.len() + usize::from(ended);
                    match self.state {
                        State::Status | State::Headers => {
                            self.head_bytes += len;
                            if self.head_bytes > MAX_HEAD {
                                return Err(Error::Malformed("headers too long"));
                            }
                        }
                        State::Trailers => {
                            self.trailer_bytes += len;
                            if self.trailer_bytes > MAX_HEAD {
                                return Err(Error::Malformed("trailers too long"));
                            }
                        }
                        // A chunk-size line (or a chunk's CRLF) is short:
                        // one that runs on is not one.
                        _ if self.line.len() + part.len() > MAX_LINE => {
                            return Err(Error::Malformed("chunk line too long"));
                        }
                        _ => {}
                    }
                    // A longer line keeps its first MAX_LINE bytes.
                    let room = MAX_LINE.saturating_sub(self.line.len());
                    self.line.extend_from_slice(&part[..part.len().min(room)]);
                    if ended {
                        let mut line = std::mem::take(&mut self.line);
                        let r = self.end_line(trim_cr(&line));
                        line.clear();
                        self.line = line;
                        r?;
                    }
                }
            }
        }
        Ok(())
    }

    fn end_line(&mut self, line: &[u8]) -> Result<(), Error> {
        match self.state {
            State::Status => {
                self.status = parse_status_line(line)?;
                self.state = State::Headers;
            }
            State::Headers if line.is_empty() => {
                // An interim reply (100 Continue, ...): its headers are
                // done, the real status line comes next.
                if (100..200).contains(&self.status) {
                    self.content_length = None;
                    self.chunked = false;
                    self.state = State::Status;
                    return Ok(());
                }
                self.state = if matches!(self.status, 204 | 304) {
                    State::Done
                } else if self.chunked {
                    State::ChunkSize
                } else {
                    match self.content_length {
                        Some(0) => State::Done,
                        Some(n) => State::Fixed(n),
                        None => State::ToClose,
                    }
                };
            }
            State::Headers => self.header(line)?,
            State::ChunkSize => {
                // Hex digits, then optional `;extensions`.
                let digits = line.split(|&b| b == b';').next().unwrap_or_default();
                let size = parse_hex(trim(digits)).ok_or(Error::Malformed("chunk size"))?;
                self.state = if size == 0 {
                    State::Trailers
                } else {
                    State::Chunk(size)
                };
            }
            State::ChunkEnd => {
                if !line.is_empty() {
                    return Err(Error::Malformed("chunk end"));
                }
                self.state = State::ChunkSize;
            }
            State::Trailers => {
                if line.is_empty() {
                    self.state = State::Done;
                }
            }
            State::Fixed(_) | State::Chunk(_) | State::ToClose | State::Done => {}
        }
        Ok(())
    }

    fn header(&mut self, line: &[u8]) -> Result<(), Error> {
        let Some(colon) = line.iter().position(|&b| b == b':') else {
            // Not a header (an obsolete folded line, or noise): ignored.
            return Ok(());
        };
        let name = trim(&line[..colon]);
        let value = trim(&line[colon + 1..]);
        if name.eq_ignore_ascii_case(b"content-length") {
            // Digits only: `u64::from_str` would take a leading `+`.
            let n = Some(value)
                .filter(|v| !v.is_empty() && v.iter().all(u8::is_ascii_digit))
                .and_then(|v| std::str::from_utf8(v).ok())
                .and_then(|v| v.parse::<u64>().ok())
                .ok_or(Error::Malformed("content-length"))?;
            if self.content_length.is_some_and(|c| c != n) {
                return Err(Error::Malformed("content-length"));
            }
            self.content_length = Some(n);
        } else if name.eq_ignore_ascii_case(b"transfer-encoding") {
            // The last coding is the one that frames the body.
            let last = value
                .rsplit(|&b| b == b',')
                .next()
                .map(trim)
                .unwrap_or_default();
            if last.eq_ignore_ascii_case(b"chunked") {
                self.chunked = true;
            } else if !last.eq_ignore_ascii_case(b"identity") {
                return Err(Error::Malformed("transfer-encoding"));
            }
        }
        Ok(())
    }

    /// The connection closed: the status if the reply was whole (by its
    /// framing, or a read-to-close body), else why not.
    pub fn finish(&self) -> Result<u16, Error> {
        match self.state {
            State::Done | State::ToClose => Ok(self.status),
            State::Status | State::Headers => Err(Error::NoReply),
            _ => Err(Error::Dropped),
        }
    }
}

fn trim_cr(line: &[u8]) -> &[u8] {
    line.strip_suffix(b"\r").unwrap_or(line)
}

fn trim(s: &[u8]) -> &[u8] {
    let start = s
        .iter()
        .position(|b| !b.is_ascii_whitespace())
        .unwrap_or(s.len());
    let end = s
        .iter()
        .rposition(|b| !b.is_ascii_whitespace())
        .map_or(start, |e| e + 1);
    &s[start..end]
}

fn parse_hex(s: &[u8]) -> Option<u64> {
    if s.is_empty() || s.len() > 15 {
        return None;
    }
    s.iter().try_fold(0u64, |n, &b| {
        let d = (b as char).to_digit(16)?;
        Some(n * 16 + u64::from(d))
    })
}

/// `HTTP/1.x NNN reason`.
fn parse_status_line(line: &[u8]) -> Result<u16, Error> {
    let rest = line
        .strip_prefix(b"HTTP/1.")
        .ok_or(Error::Malformed("status line"))?;
    let code = rest
        .get(2..5)
        .filter(|_| rest.get(1) == Some(&b' '))
        .ok_or(Error::Malformed("status line"))?;
    if rest.get(5).is_some_and(|&b| b != b' ') {
        return Err(Error::Malformed("status line"));
    }
    std::str::from_utf8(code)
        .ok()
        .and_then(|c| c.parse::<u16>().ok())
        .filter(|c| (100..600).contains(c))
        .ok_or(Error::Malformed("status line"))
}

/// What to ask for: `GET <path>` from `host` (the `Host` header:
/// `name[:port]`) with `headers`; at most `max_body` bytes of the body are
/// read (a reply that long is not the one wanted). The whole exchange must
/// be over by `deadline` ([`Error::Timeout`] after it), and the reply, all
/// bytes counted, may not exceed [`Get::max_total`], so a server that
/// trickles, or sends framing without end, cannot hold the caller.
#[derive(Debug, Clone, Copy)]
pub struct Get<'a> {
    pub host: &'a str,
    pub path: &'a str,
    pub headers: &'a [(&'a str, &'a str)],
    pub max_body: usize,
    pub deadline: Option<Instant>,
}

impl Get<'_> {
    /// The most bytes read in all: headers and trailers at their caps,
    /// the body, and room for chunk framing as dense as one 8-byte chunk
    /// line per 8 bytes of data.
    pub fn max_total(&self) -> usize {
        2 * MAX_HEAD + 2 * self.max_body
    }
}

/// `host` as a name and a port: `name` alone (the port is `None`) or
/// `name:port` with a port of 1-65535 in digits. `None` when it is neither
/// (an empty name, or a port that is not a number).
pub fn split_host_port(host: &str) -> Option<(&str, Option<u16>)> {
    let (name, port) = match host.rsplit_once(':') {
        None => (host, None),
        Some((name, port)) => {
            let ok =
                !port.is_empty() && port.len() <= 5 && port.bytes().all(|b| b.is_ascii_digit());
            let port = port.parse::<u16>().ok().filter(|&p| ok && p != 0)?;
            (name, Some(port))
        }
    };
    (!name.is_empty()).then_some((name, port))
}

/// One `GET` over `stream`: writes the request, reads the reply and hands
/// the body to `body` as it arrives, `buf`'s size at a time. Stops (with
/// the status) once the body is whole, the server closes a read-to-close
/// body, or `max_body` bytes have been handed over (the rest is not read).
/// `after_io` runs after the write and after every read (the firmware
/// samples its heap there).
pub fn get<S: Read + Write>(
    stream: &mut S,
    req: &Get,
    buf: &mut [u8],
    body: &mut dyn FnMut(&[u8]),
    after_io: &mut dyn FnMut(),
) -> Result<u16, Error> {
    stream.write_all(request(req.host, req.path, req.headers).as_bytes())?;
    stream.flush()?;
    after_io();
    let mut reader = ResponseReader::new();
    let mut read = 0usize;
    let mut total = 0usize;
    let late = || req.deadline.is_some_and(|d| Instant::now() >= d);
    loop {
        if late() {
            return Err(Error::Timeout);
        }
        let n = match stream.read(buf) {
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e.into()),
        };
        after_io();
        if n == 0 {
            return reader.finish();
        }
        total += n;
        if total > req.max_total() {
            return Err(Error::Malformed("reply too long"));
        }
        reader.feed(&buf[..n], &mut |b| {
            read += b.len();
            body(b);
        })?;
        if reader.is_complete() || (reader.head_done() && read >= req.max_body) {
            return Ok(reader.status);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    /// A stream that answers with scripted reads: each a chunk of bytes, or
    /// an error; then EOF.
    struct Script {
        reads: VecDeque<Result<Vec<u8>, io::ErrorKind>>,
        written: Vec<u8>,
    }

    impl Script {
        fn new(reads: Vec<Result<&[u8], io::ErrorKind>>) -> Self {
            Script {
                reads: reads.into_iter().map(|r| r.map(<[u8]>::to_vec)).collect(),
                written: Vec::new(),
            }
        }
        fn chunks(reply: &[u8], size: usize) -> Self {
            Script::new(reply.chunks(size).map(Ok).collect())
        }
    }

    impl Read for Script {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            match self.reads.pop_front() {
                None => Ok(0),
                Some(Err(k)) => Err(k.into()),
                Some(Ok(mut v)) => {
                    let n = v.len().min(buf.len());
                    buf[..n].copy_from_slice(&v[..n]);
                    if n < v.len() {
                        self.reads.push_front(Ok(v.split_off(n)));
                    }
                    Ok(n)
                }
            }
        }
    }

    impl Write for Script {
        fn write(&mut self, b: &[u8]) -> io::Result<usize> {
            self.written.extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn run(s: &mut Script, buf_size: usize) -> (Result<u16, Error>, Vec<u8>, usize) {
        let mut body = Vec::new();
        let mut ios = 0;
        let mut buf = vec![0u8; buf_size];
        let req = Get {
            host: "192.168.1.112:55356",
            path: "/api/v1/get_status",
            headers: &[("X-RePlay-Token", "1234")],
            max_body: 16 * 1024,
            deadline: None,
        };
        let r = get(
            s,
            &req,
            &mut buf,
            &mut |b| body.extend_from_slice(b),
            &mut || ios += 1,
        );
        (r, body, ios)
    }

    // What RePlayOS v1.8.0 sends (captured with curl -i, 2026-10-26).
    const REAL_401: &[u8] = b"HTTP/1.1 401 Unauthorized\r\nContent-Type: application/json\r\nCache-Control: no-store\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET, OPTIONS\r\nAccess-Control-Allow-Headers: X-RePlay-Token\r\nConnection: close\r\nContent-Length: 25\r\n\r\n{\"error\":\"Unauthorized\"}\n";
    const REAL_VERSION: &[u8] = b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nCache-Control: no-store\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET, OPTIONS\r\nAccess-Control-Allow-Headers: X-RePlay-Token\r\nConnection: close\r\nContent-Length: 30\r\n\r\n{\"version\":\"RePlayOS v1.8.0\"}\n";

    #[test]
    fn writes_the_request() {
        let mut s = Script::chunks(REAL_VERSION, 1000);
        run(&mut s, 256).0.unwrap();
        assert_eq!(
            String::from_utf8(s.written).unwrap(),
            "GET /api/v1/get_status HTTP/1.1\r\nHost: 192.168.1.112:55356\r\nX-RePlay-Token: 1234\r\nConnection: close\r\n\r\n"
        );
    }

    #[test]
    fn request_values_cannot_add_headers() {
        let r = request("h", "/p", &[("X-RePlay-Token", "12\r\nEvil: 1")]);
        assert_eq!(
            r,
            "GET /p HTTP/1.1\r\nHost: h\r\nX-RePlay-Token: 12Evil: 1\r\nConnection: close\r\n\r\n"
        );
    }

    #[test]
    fn reads_the_real_replies_whole_and_byte_by_byte() {
        for size in [1, 2, 7, 64, 4096] {
            let (r, body, _) = run(&mut Script::chunks(REAL_VERSION, size), 256);
            assert_eq!(r, Ok(200), "chunk size {size}");
            assert_eq!(body, b"{\"version\":\"RePlayOS v1.8.0\"}\n");
            let (r, body, _) = run(&mut Script::chunks(REAL_401, size), 256);
            assert_eq!(r, Ok(401));
            assert_eq!(body, b"{\"error\":\"Unauthorized\"}\n");
        }
        // A tiny read buffer too.
        let (r, body, _) = run(&mut Script::chunks(REAL_VERSION, 4096), 3);
        assert_eq!((r, body.len()), (Ok(200), 30));
    }

    #[test]
    fn stops_at_content_length_without_waiting_for_the_close() {
        // The reply, then a read that would block forever: never reached.
        let mut s = Script::new(vec![Ok(REAL_VERSION), Err(io::ErrorKind::WouldBlock)]);
        let (r, _, ios) = run(&mut s, 4096);
        assert_eq!(r, Ok(200));
        assert_eq!(ios, 2, "the write and one read");
        // Bytes past the length are not body.
        let reply = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nokEXTRA";
        let (r, body, _) = run(&mut Script::chunks(reply, 100), 256);
        assert_eq!((r, body), (Ok(200), b"ok".to_vec()));
    }

    #[test]
    fn chunked() {
        let reply = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4\r\n{\"a\"\r\nA;ext=1\r\n:\"bcdefghi\r\n1\r\n}\r\n0\r\nX-Trailer: y\r\n\r\n";
        for size in [1, 3, 5, 1000] {
            let (r, body, _) = run(&mut Script::chunks(reply, size), 256);
            assert_eq!(r, Ok(200), "chunk size {size}");
            assert_eq!(body, b"{\"a\":\"bcdefghi}");
        }
        // Upper-case hex, any case of the header, bare LF line ends.
        let reply = b"HTTP/1.1 200 OK\ntransfer-encoding: Chunked\n\n1F\nabcdefghijklmnopqrstuvwxyz01234\n0\n\n";
        let (r, body, _) = run(&mut Script::chunks(reply, 4), 256);
        assert_eq!((r, body.len()), (Ok(200), 31));
        // A coding other than chunked/identity last cannot be framed.
        let reply = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: gzip\r\n\r\n";
        assert_eq!(
            run(&mut Script::chunks(reply, 100), 256).0,
            Err(Error::Malformed("transfer-encoding"))
        );
    }

    #[test]
    fn chunked_errors() {
        let bad = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\nzz\r\n";
        assert_eq!(
            run(&mut Script::chunks(bad, 100), 256).0,
            Err(Error::Malformed("chunk size"))
        );
        let bad = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n2\r\nabc\r\n";
        assert_eq!(
            run(&mut Script::chunks(bad, 100), 256).0,
            Err(Error::Malformed("chunk end"))
        );
        // Closed before the last chunk.
        let cut = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nab";
        assert_eq!(
            run(&mut Script::chunks(cut, 100), 256).0,
            Err(Error::Dropped)
        );
    }

    #[test]
    fn read_to_close_without_a_length() {
        let reply = b"HTTP/1.0 200 OK\r\nServer: x\r\n\r\n{\"system\":\"\"}";
        let (r, body, _) = run(&mut Script::chunks(reply, 5), 256);
        assert_eq!((r, body), (Ok(200), b"{\"system\":\"\"}".to_vec()));
    }

    #[test]
    fn short_reads_and_closes() {
        // Nothing at all: the server accepted and closed.
        assert_eq!(run(&mut Script::new(vec![]), 256).0, Err(Error::NoReply));
        // Closed inside the status line, inside the headers.
        assert_eq!(
            run(&mut Script::new(vec![Ok(b"HTTP/1.1 20")]), 256).0,
            Err(Error::NoReply)
        );
        assert_eq!(
            run(
                &mut Script::new(vec![Ok(b"HTTP/1.1 200 OK\r\nContent-Le")]),
                256
            )
            .0,
            Err(Error::NoReply)
        );
        // Closed inside a Content-Length body: the part read went to the body.
        let (r, body, _) = run(
            &mut Script::new(vec![
                Ok(b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\n"),
                Ok(b"abc"),
            ]),
            256,
        );
        assert_eq!((r, body), (Err(Error::Dropped), b"abc".to_vec()));
    }

    #[test]
    fn timeouts() {
        // Before any byte (the socket's read timeout).
        let mut s = Script::new(vec![Err(io::ErrorKind::WouldBlock)]);
        assert_eq!(run(&mut s, 256).0, Err(Error::Timeout));
        let mut s = Script::new(vec![Err(io::ErrorKind::TimedOut)]);
        assert_eq!(run(&mut s, 256).0, Err(Error::Timeout));
        // Mid-headers and mid-body.
        let mut s = Script::new(vec![
            Ok(b"HTTP/1.1 200 OK\r\n"),
            Err(io::ErrorKind::WouldBlock),
        ]);
        assert_eq!(run(&mut s, 256).0, Err(Error::Timeout));
        let mut s = Script::new(vec![
            Ok(b"HTTP/1.1 200 OK\r\nContent-Length: 9\r\n\r\nabc"),
            Err(io::ErrorKind::WouldBlock),
        ]);
        assert_eq!(run(&mut s, 256).0, Err(Error::Timeout));
        // An interrupted read is retried.
        let mut s = Script::new(vec![Err(io::ErrorKind::Interrupted), Ok(REAL_VERSION)]);
        assert_eq!(run(&mut s, 256).0, Ok(200));
        // Other errors are named.
        let mut s = Script::new(vec![Err(io::ErrorKind::PermissionDenied)]);
        assert_eq!(
            run(&mut s, 256).0,
            Err(Error::Io(io::ErrorKind::PermissionDenied))
        );
    }

    #[test]
    fn not_http() {
        for reply in [
            &b"SSH-2.0-OpenSSH\r\n"[..],
            b"HTTP/2 200\r\n\r\n",
            b"HTTP/1.1 2000 OK\r\n",
            b"HTTP/1.1 abc\r\n",
            b"HTTP/1.1 099 x\r\n",
        ] {
            assert_eq!(
                run(&mut Script::chunks(reply, 100), 256).0,
                Err(Error::Malformed("status line")),
                "{:?}",
                String::from_utf8_lossy(reply)
            );
        }
        // A reason phrase is optional.
        assert_eq!(
            run(&mut Script::chunks(b"HTTP/1.1 204\r\n\r\n", 100), 256).0,
            Ok(204)
        );
        for bad in [
            &b"HTTP/1.1 200 OK\r\nContent-Length: +5\r\n\r\nabcde"[..],
            b"HTTP/1.1 200 OK\r\nContent-Length: -1\r\n\r\n",
            b"HTTP/1.1 200 OK\r\nContent-Length:\r\n\r\n",
        ] {
            assert_eq!(
                run(&mut Script::chunks(bad, 100), 256).0,
                Err(Error::Malformed("content-length"))
            );
        }
        let bad = b"HTTP/1.1 200 OK\r\nContent-Length: x\r\n\r\n";
        assert_eq!(
            run(&mut Script::chunks(bad, 100), 256).0,
            Err(Error::Malformed("content-length"))
        );
        let bad = b"HTTP/1.1 200 OK\r\nContent-Length: 1\r\nContent-Length: 2\r\n\r\n";
        assert_eq!(
            run(&mut Script::chunks(bad, 100), 256).0,
            Err(Error::Malformed("content-length"))
        );
    }

    #[test]
    fn long_headers_are_cut_and_capped() {
        // A long header line is cut, and the ones after it still count.
        let reply = format!(
            "HTTP/1.1 200 OK\r\nX-Long: {}\r\nContent-Length: 2\r\n\r\nok",
            "v".repeat(1000)
        );
        let (r, body, _) = run(&mut Script::chunks(reply.as_bytes(), 50), 256);
        assert_eq!((r, body), (Ok(200), b"ok".to_vec()));
        // But not without end.
        let reply = format!("HTTP/1.1 200 OK\r\nX-Long: {}", "v".repeat(MAX_HEAD + 1));
        assert_eq!(
            run(&mut Script::chunks(reply.as_bytes(), 512), 256).0,
            Err(Error::Malformed("headers too long"))
        );
    }

    #[test]
    fn interim_and_bodyless_replies() {
        let reply = b"HTTP/1.1 100 Continue\r\n\r\nHTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok";
        let (r, body, _) = run(&mut Script::chunks(reply, 7), 256);
        assert_eq!((r, body), (Ok(200), b"ok".to_vec()));
        // 304 / 204 / Content-Length: 0 are complete at the blank line.
        for reply in [
            &b"HTTP/1.1 304 Not Modified\r\nContent-Length: 50\r\n\r\n"[..],
            b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n",
        ] {
            let mut s = Script::new(vec![Ok(reply), Err(io::ErrorKind::WouldBlock)]);
            assert!(run(&mut s, 256).0.is_ok());
        }
    }

    #[test]
    fn a_long_body_stops_at_the_cap() {
        let mut reply = b"HTTP/1.1 200 OK\r\n\r\n".to_vec();
        reply.extend(std::iter::repeat_n(b'x', 100_000));
        let mut s = Script::chunks(&reply, 256);
        let (r, body, _) = run(&mut s, 256);
        assert_eq!(r, Ok(200));
        assert!(
            body.len() >= 16 * 1024 && body.len() < 16 * 1024 + 256,
            "{}",
            body.len()
        );
        assert!(!s.reads.is_empty(), "the rest was not read");
    }

    /// A stream whose reads each take `delay` and return `bytes` forever.
    struct Trickle {
        delay: std::time::Duration,
        bytes: &'static [u8],
        first: Option<&'static [u8]>,
        reads: usize,
    }

    impl Read for Trickle {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            std::thread::sleep(self.delay);
            self.reads += 1;
            let b = self.first.take().unwrap_or(self.bytes);
            let n = b.len().min(buf.len());
            buf[..n].copy_from_slice(&b[..n]);
            Ok(n)
        }
    }

    impl Write for Trickle {
        fn write(&mut self, b: &[u8]) -> io::Result<usize> {
            Ok(b.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn get_from(s: &mut impl ReadWrite, deadline: Option<Instant>) -> Result<u16, Error> {
        let req = Get {
            host: "h",
            path: "/",
            headers: &[],
            max_body: 16 * 1024,
            deadline,
        };
        get(s, &req, &mut [0u8; 256], &mut |_| {}, &mut || {})
    }

    trait ReadWrite: Read + Write {}
    impl<T: Read + Write> ReadWrite for T {}

    #[test]
    fn a_trickling_server_meets_the_deadline() {
        use std::time::Duration;
        // One byte of header every 5 ms, never ending: each read is quick
        // (no socket timeout would fire), the deadline ends it.
        let mut s = Trickle {
            delay: Duration::from_millis(5),
            bytes: b"x",
            first: Some(b"HTTP/1.1 200 OK\r\nX: "),
            reads: 0,
        };
        let t = Instant::now();
        let r = get_from(&mut s, Some(Instant::now() + Duration::from_millis(60)));
        assert_eq!(r, Err(Error::Timeout));
        assert!(
            t.elapsed() < Duration::from_millis(200),
            "{:?}",
            t.elapsed()
        );
        assert!(s.reads > 3);
        // A deadline already past: no read at all.
        let mut s = Trickle {
            delay: Duration::ZERO,
            bytes: b"x",
            first: None,
            reads: 0,
        };
        assert_eq!(get_from(&mut s, Some(Instant::now())), Err(Error::Timeout));
        assert_eq!(s.reads, 0);
    }

    #[test]
    fn endless_framing_is_cut_off() {
        use std::time::Duration;
        let z = Duration::ZERO;
        // A chunk-size line without end.
        let mut s = Trickle {
            delay: z,
            bytes: b"0000000000000000",
            first: Some(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n"),
            reads: 0,
        };
        assert_eq!(
            get_from(&mut s, None),
            Err(Error::Malformed("chunk line too long"))
        );
        // Trailer lines without end (each one short).
        let mut s = Trickle {
            delay: z,
            bytes: b"X-T: y\r\n",
            first: Some(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n"),
            reads: 0,
        };
        assert_eq!(
            get_from(&mut s, None),
            Err(Error::Malformed("trailers too long"))
        );
        // Empty chunks never count as body: the total cap ends them.
        let mut s = Trickle {
            delay: z,
            bytes: b"1\r\nx\r\n",
            first: Some(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n"),
            reads: 0,
        };
        // (1-byte chunks: 6 bytes read per body byte, past 2 x the body cap
        // before the body cap.)
        assert_eq!(
            get_from(&mut s, None),
            Err(Error::Malformed("reply too long"))
        );
        // Headers without end.
        let mut s = Trickle {
            delay: z,
            bytes: b"X: y\r\n",
            first: Some(b"HTTP/1.1 200 OK\r\n"),
            reads: 0,
        };
        assert_eq!(
            get_from(&mut s, None),
            Err(Error::Malformed("headers too long"))
        );
    }

    #[test]
    fn resets_read_as_dropped() {
        for k in [
            io::ErrorKind::ConnectionReset,
            io::ErrorKind::ConnectionAborted,
            io::ErrorKind::BrokenPipe,
        ] {
            let mut s = Script::new(vec![Ok(b"HTTP/1.1 200 OK\r\n"), Err(k)]);
            assert_eq!(run(&mut s, 256).0, Err(Error::Dropped), "{k:?}");
        }
    }

    #[test]
    fn host_and_port() {
        assert_eq!(
            split_host_port("192.168.1.112"),
            Some(("192.168.1.112", None))
        );
        assert_eq!(
            split_host_port("replay.local:8080"),
            Some(("replay.local", Some(8080)))
        );
        for bad in [
            "", ":55356", "h:", "h:x", "h:+80", "h:0", "h:65536", "h:123456", "h: 80",
        ] {
            assert_eq!(split_host_port(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn errors_read_as_before() {
        // The words the poll reported with esp_http_client.
        assert_eq!(Error::Timeout.to_string(), "timed out");
        assert_eq!(Error::NoReply.to_string(), "no reply");
        assert_eq!(Error::Dropped.to_string(), "connection dropped");
        assert_eq!(
            Error::Malformed("chunk size").to_string(),
            "bad reply (chunk size)"
        );
    }
}

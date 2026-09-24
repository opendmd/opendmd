//! gifslim: a small streaming GIF decoder with a **capped LZW table**, and
//! the recoder that writes GIFs it can play.
//!
//! An experiment in whether plain GIF files can replace ODM as the prepared
//! format (`docs/gifslim-experiment.md`). The `gif` crate's decoder costs
//! ~37 KB of heap on top of the canvas, most of it `weezl`'s tables, which are
//! sized for the full 12-bit code space whatever the file needs. A GIF whose
//! encoder emits a CLEAR before the dictionary would outgrow `2^CAP` entries
//! never needs a code wider than `CAP` bits, so its decoder needs only
//! `prefix: u16[2^CAP]` and `suffix: u8[2^CAP]`.
//!
//! [`GifSlim`] plays the subset such a recoder writes, and most ordinary
//! GIFs: header, global and local palettes, graphic control (delay,
//! transparency, disposal `Any`/`Keep`/`Background`), frame sub-rectangles,
//! NETSCAPE/comment/plain-text/application extensions (skipped). Interlaced
//! frames and `Previous` disposal are [`Unsupported`]; a code wider than the
//! cap is [`NeedsFullDecoder`] — the caller falls back to the `gif` crate.
//! Output is identical to [`GifSource`](crate::player::GifSource)'s, canvas
//! and delay, frame for frame (the card-wide check is `opendmd-pack
//! recode --verify` and `gifscan`).
//!
//! **No frame buffer, no output stack on the heap.** Each LZW string is
//! walked once into a 96-byte buffer on the call stack and copied into a
//! one-row index buffer, which is composited onto the screen as each row
//! completes. A longer string — impossible in a frame of up to 4,656 pixels
//! (see `SHORT_STRING`) — is walked a second time and painted backwards from
//! its end position. So the heap is the 2 KB read buffer, the two tables,
//! the row and the global palette: for `CAP = 10`, 5,248 B + palette.
//!
//! [`recode_gif`] is the encoder: frames through `GifSource` (so disposal and
//! transparency are what playback shows), one global palette, deltas as the
//! minimal changed rectangle with unchanged pixels transparent and disposal
//! `Keep`, LZW via [`lzw_encode_capped`]. The output is a standard GIF89a.

use crate::canvas::{Canvas, DirtyMap, HEIGHT, WIDTH};
use crate::player::{
    blit, clear_rect, frame_delay, invalid, mark_changes, GifSource, Rect, Screen,
    MAX_LOGICAL_PIXELS, MAX_PALETTE,
};
use gif::DisposalMethod;
use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::time::Duration;

/// The decoder's read buffer (as `OdmSource`'s).
pub const READ_BUF: usize = 2048;
/// GIF's own limit on a code's width.
pub const MAX_CAP: u8 = 12;
/// Narrowest useful cap: minimum code size 2 starts at 3 bits.
pub const MIN_CAP: u8 = 3;
/// Cap used by [`GifSlim::new`] when the file carries no hint: the full
/// 12 bits, so an ordinary GIF plays (26.8 KB with the canvas, against the
/// `gif` crate's ~54 KB) unless it is interlaced or uses `Previous`.
pub const DEFAULT_CAP: u8 = 12;
/// Application extension identifier (8 bytes) + authentication code (3) of
/// the hint block the recoder writes: the data sub-block that follows is
/// `[version = 1, cap, flags]` (flags bit 0: delta frames).
pub const HINT_APP_ID: &[u8; 11] = b"OPENDMD1LZW";
/// Longest LZW string composited from the on-stack buffer; longer ones take
/// the two-walk path. A string of length `L` needs entries of every length
/// below it, defined by codes of lengths `1..L` since the last CLEAR, so a
/// frame of `P` pixels holds strings of at most `L` with `L(L+1)/2 <= P`:
/// 90 for a 128x32 frame (measured: 90, a one-colour frame). 96 covers any
/// frame up to 4,656 pixels without the second walk.
const SHORT_STRING: usize = 96;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// The file needs a wider LZW code than this decoder's tables hold: play it
/// with the `gif` crate. Carried in an `io::Error` of kind `Unsupported`;
/// test with [`is_needs_full_decoder`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NeedsFullDecoder {
    pub cap: u8,
}

impl std::fmt::Display for NeedsFullDecoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "LZW code wider than {} bits (needs the full decoder)",
            self.cap
        )
    }
}
impl std::error::Error for NeedsFullDecoder {}

/// A GIF feature outside gifslim's subset (interlacing, `Previous`
/// disposal). Carried in an `io::Error` of kind `Unsupported`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Unsupported(pub &'static str);

impl std::fmt::Display for Unsupported {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "unsupported by gifslim: {}", self.0)
    }
}
impl std::error::Error for Unsupported {}

pub fn is_needs_full_decoder(e: &io::Error) -> bool {
    e.get_ref().is_some_and(|i| i.is::<NeedsFullDecoder>())
}

pub fn is_unsupported(e: &io::Error) -> bool {
    e.get_ref().is_some_and(|i| i.is::<Unsupported>())
}

fn needs_full(cap: u8) -> io::Error {
    io::Error::new(io::ErrorKind::Unsupported, NeedsFullDecoder { cap })
}

fn unsupported(what: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::Unsupported, Unsupported(what))
}

// ---------------------------------------------------------------------------
// Decoder
// ---------------------------------------------------------------------------

/// What the recoder's application-extension hint said.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hint {
    pub cap: u8,
    pub delta: bool,
}

/// Counters over what has been decoded so far (for the experiment's tools).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SlimStats {
    /// Widest code actually read before a frame's pixels were complete.
    pub max_width: u8,
    /// Longest LZW string.
    pub max_string: usize,
    /// Strings longer than the on-stack buffer (the two-walk path).
    pub long_strings: u64,
    pub codes: u64,
}

/// A GIF being played by the small decoder. Same contract as
/// [`OdmSource`](crate::odm::OdmSource): one per playback, no rewind,
/// `Ok(None)` at the end with the canvas untouched, and at the panel size
/// the caller's canvas is the logical screen (do not write it between frames).
pub struct GifSlim<R: Read> {
    reader: BufReader<R>,
    width: usize,
    height: usize,
    cap: u8,
    /// `2^cap` entries each; allocated once in `new`.
    prefix: Vec<u16>,
    suffix: Vec<u8>,
    /// One row of indices on their way to the screen (`width` bytes).
    row: Vec<u8>,
    /// Packed RGB, as declared (a power of two entries); empty if none.
    global: Vec<u8>,
    screen: Screen,
    pending: Option<(DisposalMethod, Rect)>,
    hint: Option<Hint>,
    ended: bool,
    frames_decoded: u64,
    stats: SlimStats,
}

/// Next byte from a `BufReader`; end of file is `UnexpectedEof`, as in the
/// `gif` crate.
#[inline]
fn byte<R: Read>(r: &mut BufReader<R>) -> io::Result<u8> {
    if let Some(&b) = r.buffer().first() {
        r.consume(1);
        return Ok(b);
    }
    let buf = r.fill_buf()?;
    let Some(&b) = buf.first() else {
        return Err(io::ErrorKind::UnexpectedEof.into());
    };
    r.consume(1);
    Ok(b)
}

fn read_exact<R: Read>(r: &mut BufReader<R>, buf: &mut [u8]) -> io::Result<()> {
    r.read_exact(buf)
}

fn skip<R: Read>(r: &mut BufReader<R>, mut n: usize) -> io::Result<()> {
    while n > 0 {
        let avail = r.fill_buf()?.len();
        if avail == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        let k = avail.min(n);
        r.consume(k);
        n -= k;
    }
    Ok(())
}

/// Skip data sub-blocks up to and including the terminator.
fn skip_sub_blocks<R: Read>(r: &mut BufReader<R>) -> io::Result<()> {
    loop {
        let len = byte(r)? as usize;
        if len == 0 {
            return Ok(());
        }
        skip(r, len)?;
    }
}

/// Parse an application extension (the `0x21 0xFF` already consumed),
/// returning our hint if it is one.
fn read_app_ext<R: Read>(r: &mut BufReader<R>) -> io::Result<Option<Hint>> {
    let mut id = [0u8; 11];
    let len = byte(r)? as usize;
    if len == 0 {
        return Ok(None);
    }
    if len != 11 {
        skip(r, len)?;
        skip_sub_blocks(r)?;
        return Ok(None);
    }
    read_exact(r, &mut id)?;
    // ImageMagick keeps unknown application extensions through a rewrite
    // but lowercases the identifier (and re-encodes the LZW, so a hint is
    // a promise to check, never a guarantee: `NeedsFullDecoder` stands).
    if !id.eq_ignore_ascii_case(HINT_APP_ID) {
        skip_sub_blocks(r)?;
        return Ok(None);
    }
    let len = byte(r)? as usize;
    let mut hint = None;
    if len >= 2 {
        let mut d = [0u8; 3];
        let k = len.min(3);
        read_exact(r, &mut d[..k])?;
        skip(r, len - k)?;
        if d[0] == 1 && (MIN_CAP..=MAX_CAP).contains(&d[1]) {
            hint = Some(Hint {
                cap: d[1],
                delta: d[2] & 1 != 0,
            });
        }
    } else {
        skip(r, len)?;
    }
    if len != 0 {
        skip_sub_blocks(r)?;
    }
    Ok(hint)
}

/// Read the header, the logical screen descriptor and the global palette;
/// then any application/comment extensions that come before the first
/// frame (where the hint lives), stopping at anything else without
/// consuming it.
#[allow(clippy::type_complexity)]
fn read_header<R: Read>(r: &mut BufReader<R>) -> io::Result<(usize, usize, Vec<u8>, Option<Hint>)> {
    let mut h = [0u8; 13];
    read_exact(r, &mut h).map_err(|e| {
        if e.kind() == io::ErrorKind::UnexpectedEof {
            invalid("GIF header is truncated")
        } else {
            e
        }
    })?;
    if &h[..6] != b"GIF87a" && &h[..6] != b"GIF89a" {
        return Err(invalid("malformed GIF header"));
    }
    let width = u16::from_le_bytes([h[6], h[7]]) as usize;
    let height = u16::from_le_bytes([h[8], h[9]]) as usize;
    if width == 0 || height == 0 {
        return Err(invalid("GIF logical screen has a zero dimension"));
    }
    if width * height > MAX_LOGICAL_PIXELS {
        return Err(invalid(format!(
            "GIF logical screen {width}x{height} is implausibly large"
        )));
    }
    let flags = h[10];
    let mut global = Vec::new();
    if flags & 0x80 != 0 {
        global = vec![0u8; 3 << ((flags & 7) + 1)];
        read_exact(r, &mut global)?;
    }
    let mut hint = None;
    loop {
        let buf = r.fill_buf()?;
        if buf.len() < 2 || buf[0] != 0x21 || !(buf[1] == 0xFF || buf[1] == 0xFE) {
            break;
        }
        let label = buf[1];
        r.consume(2);
        if label == 0xFF {
            if let Some(h) = read_app_ext(r)? {
                hint = Some(h);
            }
        } else {
            skip_sub_blocks(r)?;
        }
    }
    Ok((width, height, global, hint))
}

/// Reads LZW codes, LSB first, out of a frame's data sub-blocks.
struct Codes<'a, R: Read> {
    r: &'a mut BufReader<R>,
    /// Bytes left in the current sub-block.
    left: usize,
    /// The zero-length terminator has been read.
    done: bool,
    acc: u64,
    nbits: u32,
}

impl<R: Read> Codes<'_, R> {
    /// Move at least one byte of code data into the accumulator — as many
    /// as the sub-block, the read buffer and the accumulator allow. `false`
    /// when the data has ended.
    #[inline]
    fn refill(&mut self) -> io::Result<bool> {
        if self.left == 0 {
            if self.done {
                return Ok(false);
            }
            let len = byte(self.r)?;
            if len == 0 {
                self.done = true;
                return Ok(false);
            }
            self.left = len as usize;
        }
        if self.r.buffer().is_empty() && self.r.fill_buf()?.is_empty() {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        let buf = self.r.buffer();
        let k = self
            .left
            .min(buf.len())
            .min(((64 - self.nbits) / 8) as usize);
        for &b in &buf[..k] {
            self.acc |= (b as u64) << self.nbits;
            self.nbits += 8;
        }
        self.r.consume(k);
        self.left -= k;
        Ok(true)
    }

    /// The next `width`-bit code, or `None` when the data ran out.
    #[inline]
    fn read(&mut self, width: u32) -> io::Result<Option<u16>> {
        while self.nbits < width {
            if !self.refill()? {
                return Ok(None);
            }
        }
        let code = (self.acc & ((1 << width) - 1)) as u16;
        self.acc >>= width;
        self.nbits -= width;
        Ok(Some(code))
    }

    /// Discard whatever is left of the frame's data, terminator included.
    fn finish(&mut self) -> io::Result<()> {
        if self.done {
            return Ok(());
        }
        skip(self.r, self.left)?;
        self.left = 0;
        skip_sub_blocks(self.r)?;
        self.done = true;
        Ok(())
    }
}

/// Paints decoded indices onto the logical screen: the frame rectangle,
/// clipped to the screen, transparent and out-of-palette indices skipped, a
/// pixel written (and marked, on the canvas) only if it changes — exactly
/// what `player::Compositor::row` does, for a segment of a row at a time.
struct Painter<'a> {
    screen: &'a mut [u8],
    dirty: Option<&'a mut DirtyMap>,
    screen_w: usize,
    left: usize,
    top: usize,
    /// Visible columns / rows of the frame rectangle.
    vis_w: usize,
    vis_h: usize,
    /// The frame's palette as 256 entries, and which indices leave the
    /// screen alone (the transparent one, and any past the palette's end).
    rgb: &'a [[u8; 3]; 256],
    skip: &'a [bool; 256],
}

impl Painter<'_> {
    /// Indices for columns `x0..` of frame row `y`.
    #[inline]
    fn segment(&mut self, y: usize, x0: usize, idx: &[u8]) {
        if y >= self.vis_h || x0 >= self.vis_w {
            return;
        }
        let n = idx.len().min(self.vis_w - x0);
        let (sx, sy) = (self.left + x0, self.top + y);
        let base = (sy * self.screen_w + sx) * 3;
        let px = self.screen[base..base + n * 3].as_chunks_mut::<3>().0;
        let mut run: Option<usize> = None;
        for (i, (p, &k)) in px.iter_mut().zip(&idx[..n]).enumerate() {
            let mut changed = false;
            if !self.skip[k as usize] {
                let c = self.rgb[k as usize];
                if *p != c {
                    *p = c;
                    changed = true;
                }
            }
            match (changed, run) {
                (true, None) => run = Some(i),
                (false, Some(start)) => {
                    if let Some(d) = self.dirty.as_deref_mut() {
                        d.mark_span(sy, sx + start, sx + i);
                    }
                    run = None;
                }
                _ => {}
            }
        }
        if let (Some(start), Some(d)) = (run, self.dirty.as_deref_mut()) {
            d.mark_span(sy, sx + start, sx + n);
        }
    }

    /// One pixel (the rare long-string path).
    #[inline]
    fn put(&mut self, x: usize, y: usize, k: u8) {
        self.segment(y, x, &[k]);
    }
}

/// One frame's LZW decode, compositing as it goes.
struct Lzw<'a> {
    prefix: &'a mut [u16],
    suffix: &'a mut [u8],
    /// A row of indices waiting to be composited (the visible columns).
    row: &'a mut [u8],
    cap: u8,
    min: u8,
    frame_w: usize,
    frame_h: usize,
    stats: &'a mut SlimStats,
}

impl Lzw<'_> {
    fn run<R: Read>(&mut self, codes: &mut Codes<'_, R>, p: &mut Painter<'_>) -> io::Result<()> {
        let (fw, fh) = (self.frame_w, self.frame_h);
        if fw == 0 || fh == 0 {
            return Ok(());
        }
        let vis = p.vis_w.min(self.row.len());
        let clear = 1u16 << self.min;
        let eoi = clear + 1;
        let first = clear + 2;
        let mut next = first;
        let mut width = self.min as u32 + 1;
        const NONE: u16 = u16::MAX;
        let mut prev = NONE;
        // Next pixel (x, y) of the frame; row[seg..x] are decoded but not
        // yet composited.
        let (mut x, mut y, mut seg) = (0usize, 0usize, 0usize);
        let cap = self.cap as u32;
        let mut buf = [0u8; SHORT_STRING];
        let mut n_walked = 0usize;
        while y < fh {
            if width > cap {
                return Err(needs_full(self.cap));
            }
            if width as u8 > self.stats.max_width {
                self.stats.max_width = width as u8;
            }
            let Some(code) = codes.read(width)? else {
                return Err(invalid("image truncated"));
            };
            self.stats.codes += 1;
            if code == clear {
                next = first;
                width = self.min as u32 + 1;
                prev = NONE;
                continue;
            }
            if code == eoi {
                return Err(invalid("image truncated"));
            }
            // The string to walk: the code's own, or for KwKwK (the code
            // being defined right now) the previous one, plus its first
            // character.
            let (head, kwkwk) = if code < next {
                (code, false)
            } else if code == next && prev != NONE {
                (prev, true)
            } else {
                return Err(invalid("invalid LZW code"));
            };
            // First walk: the string's characters into the end of `buf`,
            // last first, so `buf[SHORT_STRING - n..]` reads forwards — while
            // they fit — and its length.
            let mut c = head;
            let mut pos = SHORT_STRING - kwkwk as usize;
            while c >= first {
                if pos > 0 {
                    pos -= 1;
                    buf[pos] = self.suffix[c as usize];
                }
                c = self.prefix[c as usize];
                n_walked += 1;
            }
            let fc = c as u8;
            let fits = n_walked < SHORT_STRING - kwkwk as usize;
            if fits {
                pos -= 1;
                buf[pos] = fc;
            }
            if kwkwk {
                buf[SHORT_STRING - 1] = fc;
            }
            let n = n_walked + 1 + kwkwk as usize;
            n_walked = 0;
            if n > self.stats.max_string {
                self.stats.max_string = n;
            }

            if prev != NONE && next < 4096 {
                self.prefix[next as usize] = prev;
                self.suffix[next as usize] = fc;
                next += 1;
                width = (16 - next.leading_zeros()).min(12);
            }
            prev = code;

            if fits && x + n < fw && x + n <= vis {
                // The common case: the string lands inside the current row.
                for (d, &c) in self.row[x..x + n].iter_mut().zip(&buf[pos..]) {
                    *d = c;
                }
                x += n;
            } else if fits {
                // Forward into the row buffer, compositing each row as it
                // completes.
                let mut s = &buf[pos..];
                loop {
                    let take = s.len().min(fw - x);
                    if x < vis {
                        let k = take.min(vis - x);
                        self.row[x..x + k].copy_from_slice(&s[..k]);
                    }
                    x += take;
                    s = &s[take..];
                    if x == fw {
                        p.segment(y, seg, &self.row[seg..vis.max(seg)]);
                        x = 0;
                        seg = 0;
                        y += 1;
                        if y == fh {
                            break;
                        }
                    }
                    if s.is_empty() {
                        break;
                    }
                }
            } else {
                // Composite what the row buffer holds, then paint this
                // string straight onto the screen, backwards from its end:
                // a second walk instead of an output stack.
                self.stats.long_strings += 1;
                if seg < x.min(vis) {
                    p.segment(y, seg, &self.row[seg..x.min(vis)]);
                }
                // In u64: a frame rectangle may be up to 65535x65535, and
                // `usize` is 32 bits on the ESP32.
                let fw64 = fw as u64;
                let start = y as u64 * fw64 + x as u64;
                let total = fw64 * fh as u64;
                let mut q = start + n as u64;
                if kwkwk {
                    q -= 1;
                    if q < total {
                        p.put((q % fw64) as usize, (q / fw64) as usize, fc);
                    }
                }
                let mut c = head;
                loop {
                    q -= 1;
                    let ch = if c >= first {
                        self.suffix[c as usize]
                    } else {
                        c as u8
                    };
                    if q < total {
                        p.put((q % fw64) as usize, (q / fw64) as usize, ch);
                    }
                    if c < first {
                        break;
                    }
                    c = self.prefix[c as usize];
                }
                let end = start + n as u64;
                x = (end % fw64) as usize;
                y = (end / fw64).min(fh as u64) as usize;
                seg = x;
            }
        }
        Ok(())
    }
}

impl<R: Read> GifSlim<R> {
    /// Open with the cap the file's hint names, else [`DEFAULT_CAP`].
    pub fn new(reader: R) -> io::Result<Self> {
        Self::open(reader, None)
    }

    /// Open with tables for codes of at most `cap` bits (3..=12), whatever
    /// the hint says.
    pub fn with_cap(reader: R, cap: u8) -> io::Result<Self> {
        Self::open(reader, Some(cap))
    }

    fn open(reader: R, cap: Option<u8>) -> io::Result<Self> {
        let mut reader = BufReader::with_capacity(READ_BUF, reader);
        let (width, height, global, hint) = read_header(&mut reader)?;
        let cap = cap.or(hint.map(|h| h.cap)).unwrap_or(DEFAULT_CAP);
        if !(MIN_CAP..=MAX_CAP).contains(&cap) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("LZW cap {cap} is not in {MIN_CAP}..={MAX_CAP}"),
            ));
        }
        Ok(GifSlim {
            reader,
            width,
            height,
            cap,
            prefix: vec![0u16; 1 << cap],
            suffix: vec![0u8; 1 << cap],
            row: vec![0u8; width],
            global,
            screen: if width == WIDTH && height == HEIGHT {
                Screen::Direct { cleared: false }
            } else {
                Screen::Owned(vec![0u8; width * height * 3])
            },
            pending: None,
            hint,
            ended: false,
            frames_decoded: 0,
            stats: SlimStats::default(),
        })
    }

    pub fn logical_size(&self) -> (usize, usize) {
        (self.width, self.height)
    }

    pub fn frames_decoded(&self) -> u64 {
        self.frames_decoded
    }

    /// The table size this instance was opened with.
    pub fn cap(&self) -> u8 {
        self.cap
    }

    /// The recoder's hint, if the file carries one.
    pub fn hint(&self) -> Option<Hint> {
        self.hint
    }

    pub fn stats(&self) -> SlimStats {
        self.stats
    }

    /// The reader, with the read buffer (and whatever it held) dropped.
    pub fn into_inner(self) -> R {
        self.reader.into_inner()
    }

    /// Heap this instance owns: the read buffer, the LZW tables, the global
    /// palette and, for a stray size, its own screen.
    pub fn heap_bytes(&self) -> usize {
        let screen = match &self.screen {
            Screen::Direct { .. } => 0,
            Screen::Owned(v) => v.len(),
        };
        READ_BUF
            + self.prefix.len() * 2
            + self.suffix.len()
            + self.row.len()
            + self.global.len()
            + screen
    }

    /// Decode the next frame onto `canvas` and return how long to show it.
    /// `Ok(None)` at the end of the stream leaves the canvas as it was. A
    /// code wider than the cap is [`NeedsFullDecoder`]: the canvas then
    /// holds part of this frame (and nothing else is wrong with it).
    pub fn next_frame(&mut self, canvas: &mut Canvas) -> io::Result<Option<Duration>> {
        if self.ended {
            return Ok(None);
        }
        // Blocks up to the next image descriptor.
        let mut delay = 0u16;
        let mut dispose = DisposalMethod::Any;
        let mut transparent = None;
        loop {
            match byte(&mut self.reader)? {
                0x21 => {
                    let label = byte(&mut self.reader)?;
                    match label {
                        0xF9 => {
                            let len = byte(&mut self.reader)? as usize;
                            if len != 4 {
                                return Err(invalid("control extension has wrong length"));
                            }
                            let mut g = [0u8; 4];
                            read_exact(&mut self.reader, &mut g)?;
                            if byte(&mut self.reader)? != 0 {
                                return Err(invalid("control extension has wrong length"));
                            }
                            dispose = DisposalMethod::from_u8((g[0] >> 2) & 7)
                                .unwrap_or(DisposalMethod::Any);
                            delay = u16::from_le_bytes([g[1], g[2]]);
                            transparent = (g[0] & 1 != 0).then_some(g[3]);
                        }
                        0xFF => {
                            if let Some(h) = read_app_ext(&mut self.reader)? {
                                self.hint = Some(h);
                            }
                        }
                        0x01 | 0xFE => skip_sub_blocks(&mut self.reader)?,
                        _ => return Err(invalid("unknown extension block encountered")),
                    }
                }
                0x2C => break,
                0x3B => {
                    self.ended = true;
                    return Ok(None);
                }
                _ => return Err(invalid("unknown block type encountered")),
            }
        }

        let mut d = [0u8; 9];
        read_exact(&mut self.reader, &mut d)?;
        let (width, height) = (self.width, self.height);
        let f_left = u16::from_le_bytes([d[0], d[1]]) as usize;
        let f_top = u16::from_le_bytes([d[2], d[3]]) as usize;
        let fw = u16::from_le_bytes([d[4], d[5]]) as usize;
        let fh = u16::from_le_bytes([d[6], d[7]]) as usize;
        let flags = d[8];
        let mut local = [0u8; MAX_PALETTE];
        let local_len = if flags & 0x80 != 0 {
            let n = 3 << ((flags & 7) + 1);
            read_exact(&mut self.reader, &mut local[..n])?;
            n
        } else {
            0
        };
        let palette: &[u8] = if local_len > 0 {
            &local[..local_len]
        } else if !self.global.is_empty() {
            &self.global
        } else {
            return Err(invalid("no color table available for current frame"));
        };
        let min = byte(&mut self.reader)?;
        if !(1..=11).contains(&min) {
            return Err(invalid("invalid minimal code size"));
        }
        if flags & 0x40 != 0 {
            return Err(unsupported("interlaced frame"));
        }
        if dispose == DisposalMethod::Previous {
            return Err(unsupported("Previous disposal"));
        }
        if min as u32 + 1 > self.cap as u32 {
            return Err(needs_full(self.cap));
        }
        let rect = Rect {
            left: f_left.min(width),
            top: f_top.min(height),
            width: fw,
            height: fh,
        };

        // The frame's palette as a 256-entry table, and the indices that
        // leave the screen alone: the transparent one and any past the end.
        let mut rgb = [[0u8; 3]; 256];
        let mut skip = [true; 256];
        for (i, c) in palette.as_chunks::<3>().0.iter().take(256).enumerate() {
            rgb[i] = *c;
            skip[i] = false;
        }
        if let Some(t) = transparent {
            skip[t as usize] = true;
        }

        let GifSlim {
            reader,
            prefix,
            suffix,
            row,
            screen,
            pending,
            stats,
            cap,
            ..
        } = self;
        {
            let (buf, mut dirty): (&mut [u8], Option<&mut DirtyMap>) = match screen {
                Screen::Direct { cleared } => {
                    if !*cleared {
                        canvas.clear();
                        *cleared = true;
                    }
                    let (b, d) = canvas.bytes_and_dirty_mut();
                    (b, Some(d))
                }
                Screen::Owned(v) => (v.as_mut_slice(), None),
            };
            if let Some((method, prev)) = pending.take() {
                if method == DisposalMethod::Background {
                    mark_changes(buf, None, dirty.as_deref_mut(), prev, width, height);
                    clear_rect(buf, width, prev);
                }
            }
            let mut painter = Painter {
                screen: buf,
                dirty,
                screen_w: width,
                left: rect.left,
                top: rect.top,
                vis_w: fw.min(width - rect.left),
                vis_h: fh.min(height - rect.top),
                rgb: &rgb,
                skip: &skip,
            };
            let mut codes = Codes {
                r: reader,
                left: 0,
                done: false,
                acc: 0,
                nbits: 0,
            };
            let mut lzw = Lzw {
                prefix,
                suffix,
                row,
                cap: *cap,
                min,
                frame_w: fw,
                frame_h: fh,
                stats,
            };
            lzw.run(&mut codes, &mut painter)?;
            codes.finish()?;
        }
        *pending = Some((dispose, rect));
        self.frames_decoded += 1;
        if let Screen::Owned(v) = &self.screen {
            blit(v, width, height, canvas);
        }
        Ok(Some(frame_delay(delay)))
    }
}

// ---------------------------------------------------------------------------
// Capped LZW encoder
// ---------------------------------------------------------------------------

/// What one [`lzw_encode_capped`] call emitted.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LzwStats {
    pub codes: u64,
    /// CLEARs after the first (the cap at work).
    pub clears: u64,
    /// Widest code written.
    pub max_width: u8,
    /// Longest string one code stood for.
    pub max_string: usize,
}

impl LzwStats {
    fn add(&mut self, o: LzwStats) {
        self.codes += o.codes;
        self.clears += o.clears;
        self.max_width = self.max_width.max(o.max_width);
        self.max_string = self.max_string.max(o.max_string);
    }
}

/// LSB-first bit packer.
struct Bits<'a> {
    out: &'a mut Vec<u8>,
    acc: u64,
    n: u32,
}

impl Bits<'_> {
    #[inline]
    fn put(&mut self, code: u16, width: u32) {
        self.acc |= (code as u64) << self.n;
        self.n += width;
        while self.n >= 8 {
            self.out.push(self.acc as u8);
            self.acc >>= 8;
            self.n -= 8;
        }
    }

    fn flush(&mut self) {
        if self.n > 0 {
            self.out.push(self.acc as u8);
            self.acc = 0;
            self.n = 0;
        }
    }
}

/// GIF-flavoured LZW of `data` (palette indices, each `< 2^min_code_size`)
/// appended to `out` — the raw code stream, not yet cut into sub-blocks —
/// such that **no code is wider than `cap` bits**: a CLEAR is emitted as the
/// code the decoder reads when its next free entry is `2^cap - 1`, the last
/// point at which a code (the CLEAR itself) still fits in `cap` bits. So a
/// decoder with `2^cap`-entry tables plays it; any standard decoder does too.
///
/// Code widths follow the GIF rule every decoder uses (giflib, `weezl`): a
/// code is read with as many bits as the decoder's next free entry needs.
/// `cap = 12` is an ordinary GIF encoder that clears one entry early.
pub fn lzw_encode_capped(data: &[u8], min_code_size: u8, cap: u8, out: &mut Vec<u8>) -> LzwStats {
    assert!(
        (2..=11).contains(&min_code_size),
        "GIF minimum code size is 2..=11"
    );
    assert!(
        cap > min_code_size && cap <= MAX_CAP,
        "cap {cap} cannot hold the literals of code size {min_code_size}"
    );
    let min = min_code_size as u32;
    let clear = 1u16 << min;
    let eoi = clear + 1;
    let first = clear + 2;
    // The decoder's next free entry once it has read everything emitted so
    // far, and whether the next code it reads is the first after a CLEAR
    // (which defines nothing).
    let limit = (1u16 << cap) - 1;
    debug_assert!(data.iter().all(|&b| (b as u16) < clear));
    let width_of = |next: u16| (16 - next.leading_zeros()).min(12);

    // (prefix << 8 | byte) + 1 -> code, open addressing over 2^(cap+1)
    // slots; emptied on every CLEAR.
    let slots = 1usize << (cap + 1);
    let mut keys = vec![0u32; slots];
    let mut vals = vec![0u16; slots];
    let mask = slots - 1;
    let hash = |k: u32| (k.wrapping_mul(0x9E37_79B1) >> 7) as usize;

    let mut bits = Bits { out, acc: 0, n: 0 };
    let mut st = LzwStats::default();
    let emit = |bits: &mut Bits<'_>, st: &mut LzwStats, code: u16, next: u16| {
        let w = width_of(next);
        debug_assert!(w <= cap as u32);
        bits.put(code, w);
        st.codes += 1;
        st.max_width = st.max_width.max(w as u8);
    };

    emit(&mut bits, &mut st, clear, first);
    let Some((&head, rest)) = data.split_first() else {
        emit(&mut bits, &mut st, eoi, first);
        bits.flush();
        return st;
    };
    let mut next = first; // decoder's next free entry
    let mut fresh = true; // next code read is the first after a CLEAR
    let mut w = head as u16;
    let mut wlen = 1usize;
    for &c in rest {
        let key = ((w as u32) << 8 | c as u32) + 1;
        let mut s = hash(key) & mask;
        loop {
            let k = keys[s];
            if k == key {
                break;
            }
            if k == 0 {
                break;
            }
            s = (s + 1) & mask;
        }
        if keys[s] == key {
            w = vals[s];
            wlen += 1;
            continue;
        }
        // Emit w. The decoder, reading it, defines entry `next` (unless it
        // is the first code after a CLEAR); the encoder defines the same
        // entry, w + c, now.
        emit(&mut bits, &mut st, w, next);
        st.max_string = st.max_string.max(wlen);
        let defined = if fresh { None } else { Some(next) };
        fresh = false;
        if let Some(n) = defined {
            next = n + 1;
        }
        // The encoder's dictionary runs one entry ahead of the decoder's:
        // w + c becomes entry `next`, which the decoder defines on reading
        // the code after this one.
        if next >= limit {
            // The next code would be read at `width_of(2^cap - 1)` = cap
            // bits; after one more entry it would need cap + 1. Clear now.
            emit(&mut bits, &mut st, clear, next);
            st.clears += 1;
            keys.fill(0);
            next = first;
            fresh = true;
        } else {
            keys[s] = key;
            vals[s] = next;
        }
        w = c as u16;
        wlen = 1;
    }
    emit(&mut bits, &mut st, w, next);
    st.max_string = st.max_string.max(wlen);
    if !fresh {
        next += 1;
    }
    emit(&mut bits, &mut st, eoi, next);
    bits.flush();
    st
}

// ---------------------------------------------------------------------------
// Recoder
// ---------------------------------------------------------------------------

/// How [`recode_gif`] writes its GIF.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecodeOptions {
    /// Widest LZW code, 3..=12.
    pub cap: u8,
    /// Frames as the minimal changed rectangle, unchanged pixels transparent,
    /// disposal Keep; otherwise every frame whole.
    pub delta: bool,
    /// Write the `OPENDMD1LZW` application extension naming the cap.
    pub hint: bool,
    /// Inside a delta rectangle, an unchanged pixel repeats the previous
    /// pixel's index when that is its own colour, instead of always being
    /// transparent: longer runs for the LZW (the output is the same).
    pub keep_runs: bool,
}

/// What [`recode_gif`] produced.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RecodeStats {
    pub width: usize,
    pub height: usize,
    pub frames: u32,
    /// Distinct colours in the palette written (not counting a transparent
    /// slot).
    pub colours: usize,
    /// More than 256 colours (or 256 with delta frames): `NeuQuant`, lossy.
    pub quantised: bool,
    /// Exactly 256 colours with delta frames: no slot to spare, so each
    /// frame's transparent index is one its changed pixels do not use.
    pub opaque_deltas: bool,
    /// Frames among those whose changed pixels use all 256 colours: sent as
    /// the changed rectangle with no transparency.
    pub opaque_frames: u64,
    pub gif_bytes: u64,
    pub out_bytes: u64,
    /// Pixels written in frame rectangles, over all frames.
    pub rect_px: u64,
    pub lzw: LzwStats,
}

const NEUQUANT_SAMPLEFAC: i32 = 10;

struct CountingWriter<W> {
    inner: W,
    bytes: u64,
}

impl<W: Write> Write for CountingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = self.inner.write(buf)?;
        self.bytes += n as u64;
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

fn enc_err(e: gif::EncodingError) -> io::Error {
    match e {
        gif::EncodingError::Io(e) => e,
        e => io::Error::new(io::ErrorKind::InvalidInput, e),
    }
}

/// Every frame of `gif` through [`GifSource`]: the logical screen (packed
/// RGB, before any centre-crop) and the frame's index.
fn each_frame(
    gif: &[u8],
    mut visit: impl FnMut(usize, &[u8]) -> io::Result<()>,
) -> io::Result<(usize, usize, usize)> {
    let mut src = GifSource::new(gif)?;
    let mut canvas = Canvas::new();
    let mut n = 0;
    while src.next_frame(&mut canvas)?.is_some() {
        visit(n, src.screen(&canvas))?;
        n += 1;
    }
    let (w, h) = src.logical_size();
    Ok((w, h, n))
}

/// Re-encode a GIF as a standard GIF whose LZW never needs more than
/// `opts.cap` bits, so [`GifSlim`] at that cap plays it.
///
/// Frames come from [`GifSource`], so what is encoded is what playback
/// shows. The raw delays (hundredths) and the loop count are copied from the
/// source, so every viewer times it as before. Palette: the animation's
/// distinct colours in first-seen order, plus a transparent slot with delta
/// frames; past 255 colours with deltas (256 without) `NeuQuant` builds
/// 255 (256) — lossy, flagged in the stats. Exactly 256 colours with deltas
/// keep all 256 and send changed rectangles without transparency.
pub fn recode_gif(
    mut reader: impl Read,
    out: impl Write,
    opts: &RecodeOptions,
) -> io::Result<RecodeStats> {
    if !(MIN_CAP..=MAX_CAP).contains(&opts.cap) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("cap {} is not in {MIN_CAP}..={MAX_CAP}", opts.cap),
        ));
    }
    let mut gif = Vec::new();
    reader.read_to_end(&mut gif)?;

    // Raw delays and the loop count, from the crate's own metadata.
    let mut delays = Vec::new();
    let repeat = {
        let mut o = gif::DecodeOptions::new();
        o.set_color_output(gif::ColorOutput::Indexed);
        let mut d = o.read_info(&gif[..]).map_err(crate::player::decode_err)?;
        while let Some(f) = d.next_frame_info().map_err(crate::player::decode_err)? {
            delays.push(f.delay);
        }
        d.repeat()
    };

    // Colours, first seen.
    let mut lookup: HashMap<[u8; 3], u8> = HashMap::new();
    let mut palette: Vec<[u8; 3]> = Vec::new();
    let mut too_many = false;
    let (width, height, frames) = each_frame(&gif, |_, screen| {
        if !too_many {
            for &c in screen.as_chunks::<3>().0 {
                if let std::collections::hash_map::Entry::Vacant(slot) = lookup.entry(c) {
                    if palette.len() == 256 {
                        too_many = true;
                        break;
                    }
                    slot.insert(palette.len() as u8);
                    palette.push(c);
                }
            }
        }
        Ok(())
    })?;
    if frames == 0 {
        return Err(invalid("GIF has no frames"));
    }
    if frames != delays.len() {
        return Err(invalid("frame count differs between passes"));
    }
    let max_colours = if opts.delta { 255 } else { 256 };
    let quantise = too_many;
    let opaque_deltas = opts.delta && !too_many && palette.len() == 256;
    let quant = if quantise {
        let mut rgba = Vec::with_capacity(frames * width * height * 4);
        each_frame(&gif, |_, screen| {
            for px in screen.as_chunks::<3>().0 {
                rgba.extend_from_slice(&[px[0], px[1], px[2], 255]);
            }
            Ok(())
        })?;
        let nq = color_quant::NeuQuant::new(NEUQUANT_SAMPLEFAC, max_colours, &rgba);
        drop(rgba);
        palette = nq.color_map_rgb().as_chunks::<3>().0.to_vec();
        lookup.clear();
        Some(nq)
    } else {
        None
    };
    let transparent = (opts.delta && !opaque_deltas).then_some(palette.len() as u8);
    let entries = palette.len() + transparent.is_some() as usize;
    let table = entries.max(4).next_power_of_two();
    let min_code_size = table.trailing_zeros() as u8;
    if opts.cap <= min_code_size {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "cap {} cannot hold a {entries}-entry palette's literals",
                opts.cap
            ),
        ));
    }
    let mut pal_bytes = vec![0u8; table * 3];
    for (i, c) in palette.iter().enumerate() {
        pal_bytes[i * 3..i * 3 + 3].copy_from_slice(c);
    }

    let mut stats = RecodeStats {
        width,
        height,
        frames: frames as u32,
        colours: palette.len(),
        quantised: quant.is_some(),
        opaque_deltas,
        gif_bytes: gif.len() as u64,
        ..Default::default()
    };
    let mut enc = gif::Encoder::new(
        CountingWriter {
            inner: out,
            bytes: 0,
        },
        width as u16,
        height as u16,
        &pal_bytes,
    )
    .map_err(enc_err)?;
    if opts.hint {
        enc.write_raw_extension(
            gif::AnyExtension(0xFF),
            &[HINT_APP_ID, &[1, opts.cap, opts.delta as u8]],
        )?;
    }
    if repeat != gif::Repeat::Finite(0) {
        enc.set_repeat(repeat).map_err(enc_err)?;
    }

    let n = width * height;
    let mut idx = vec![0u8; n];
    let mut prev = vec![0u8; n * 3]; // what a decoder shows: starts black
    let mut changed = vec![false; n];
    let mut rect_buf = Vec::with_capacity(n);
    let mut lzw = Vec::new();
    each_frame(&gif, |k, screen| {
        let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
        for (i, &c) in screen.as_chunks::<3>().0.iter().enumerate() {
            let q = match (&quant, lookup.get(&c)) {
                (_, Some(&q)) => q,
                (Some(nq), None) => {
                    let q = nq.index_of(&[c[0], c[1], c[2], 255]) as u8;
                    lookup.insert(c, q);
                    q
                }
                (None, None) => return Err(invalid("GIF decoded differently on the second pass")),
            };
            idx[i] = q;
            let shown = &palette[q as usize];
            let d = &mut prev[i * 3..i * 3 + 3];
            // Frame 0 is sent whole and opaque: our players start from
            // black, but a browser shows what a first frame leaves
            // uncovered (or transparent) as the page behind it.
            changed[i] = k == 0 || d != shown;
            if changed[i] {
                d.copy_from_slice(shown);
                let (x, y) = (i % width, i / width);
                x0 = x0.min(x);
                x1 = x1.max(x + 1);
                y0 = y0.min(y);
                y1 = y1.max(y + 1);
            }
        }
        rect_buf.clear();
        // With all 256 palette slots taken there is no spare transparent
        // index, but a frame's changed pixels rarely use every colour: any
        // index they do not use can be this frame's transparent one.
        let frame_t = if opaque_deltas && k > 0 && x0 != usize::MAX {
            let mut used = [false; 256];
            for y in y0..y1 {
                for x in x0..x1 {
                    let i = y * width + x;
                    if changed[i] {
                        used[idx[i] as usize] = true;
                    }
                }
            }
            let t = used.iter().position(|u| !u).map(|t| t as u8);
            if t.is_none() {
                stats.opaque_frames += 1;
            }
            t
        } else {
            transparent
        };
        let (left, top, w, h) = if !opts.delta {
            rect_buf.extend_from_slice(&idx);
            (0, 0, width, height)
        } else if x0 == usize::MAX {
            // Nothing changed: one pixel that leaves the screen alone (or,
            // with no transparent index, repaints itself).
            rect_buf.push(frame_t.unwrap_or(idx[0]));
            (0, 0, 1, 1)
        } else {
            let mut last = None;
            for y in y0..y1 {
                for x in x0..x1 {
                    let i = y * width + x;
                    let v = match frame_t {
                        Some(_) if !changed[i] && opts.keep_runs && last == Some(idx[i]) => idx[i],
                        Some(t) if !changed[i] => t,
                        _ => idx[i],
                    };
                    rect_buf.push(v);
                    last = Some(v);
                }
            }
            (x0, y0, x1 - x0, y1 - y0)
        };
        stats.rect_px += (w * h) as u64;
        lzw.clear();
        lzw.push(min_code_size);
        let ls = lzw_encode_capped(&rect_buf, min_code_size, opts.cap, &mut lzw);
        stats.lzw.add(ls);
        let frame = gif::Frame {
            delay: delays[k],
            dispose: if opts.delta {
                DisposalMethod::Keep
            } else {
                DisposalMethod::Any
            },
            transparent: frame_t,
            left: left as u16,
            top: top as u16,
            width: w as u16,
            height: h as u16,
            buffer: std::borrow::Cow::Borrowed(&lzw),
            ..Default::default()
        };
        enc.write_lzw_pre_encoded_frame(&frame).map_err(enc_err)
    })?;
    let mut w = enc.into_inner()?;
    w.flush()?;
    stats.out_bytes = w.bytes;
    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gif::{Encoder, Frame, Repeat};
    use std::borrow::Cow;
    use std::io::Cursor;

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn below(&mut self, n: usize) -> usize {
            (self.next() % n as u64) as usize
        }
    }

    type Played = (Vec<(Vec<u8>, Duration)>, Option<String>);

    fn play_gif(bytes: &[u8]) -> Played {
        let mut out = Vec::new();
        let mut c = Canvas::new();
        let mut s = match GifSource::new(bytes) {
            Ok(s) => s,
            Err(e) => return (out, Some(e.to_string())),
        };
        loop {
            match s.next_frame(&mut c) {
                Ok(Some(d)) => out.push((c.as_bytes().to_vec(), d)),
                Ok(None) => return (out, None),
                Err(e) => return (out, Some(e.to_string())),
            }
        }
    }

    fn play_slim(bytes: &[u8], cap: u8) -> io::Result<Vec<(Vec<u8>, Duration)>> {
        let mut out = Vec::new();
        let mut c = Canvas::new();
        let mut s = GifSlim::with_cap(bytes, cap)?;
        while let Some(d) = s.next_frame(&mut c)? {
            out.push((c.as_bytes().to_vec(), d));
        }
        assert!(s.stats().max_width <= cap);
        Ok(out)
    }

    fn assert_same(bytes: &[u8], cap: u8) {
        let (want, err) = play_gif(bytes);
        assert_eq!(err, None, "gif crate failed");
        let got = play_slim(bytes, cap).expect("gifslim failed");
        assert_eq!(want.len(), got.len(), "frame count");
        for (i, (a, b)) in want.iter().zip(&got).enumerate() {
            assert_eq!(a.1, b.1, "delay of frame {i}");
            assert!(a.0 == b.0, "canvas of frame {i} differs");
        }
    }

    fn palette(n: usize) -> Vec<u8> {
        (0..n)
            .flat_map(|i| [(i * 37) as u8, (i * 91 + 5) as u8, (i * 13 + 200) as u8])
            .collect()
    }

    /// One frame of `pixels` LZW'd by our encoder at `cap`, in a GIF.
    fn capped_gif(w: u16, h: u16, colours: usize, pixels: &[u8], cap: u8) -> (Vec<u8>, LzwStats) {
        let min = colours.max(4).next_power_of_two().trailing_zeros() as u8;
        let mut lzw = vec![min];
        let st = lzw_encode_capped(pixels, min, cap, &mut lzw);
        let mut out = Vec::new();
        {
            let mut e = Encoder::new(&mut out, w, h, &palette(colours)).unwrap();
            let f = Frame {
                width: w,
                height: h,
                delay: 7,
                buffer: Cow::Borrowed(&lzw),
                ..Default::default()
            };
            e.write_lzw_pre_encoded_frame(&f).unwrap();
        }
        (out, st)
    }

    fn noisy(rng: &mut Rng, n: usize, colours: usize, runs: usize) -> Vec<u8> {
        let mut v = Vec::with_capacity(n);
        while v.len() < n {
            let c = rng.below(colours) as u8;
            let len = 1 + rng.below(runs);
            for _ in 0..len.min(n - v.len()) {
                v.push(c);
            }
        }
        v
    }

    #[test]
    fn capped_lzw_round_trips_at_every_cap() {
        let mut rng = Rng(0x1234_5678_9abc_def1);
        for colours in [2usize, 4, 16, 100, 256] {
            let min = colours.max(4).next_power_of_two().trailing_zeros() as u8;
            for cap in (min + 1)..=MAX_CAP {
                for runs in [1usize, 3, 40, 5000] {
                    let px = noisy(&mut rng, 128 * 32, colours, runs);
                    let (gif, st) = capped_gif(128, 32, colours, &px, cap);
                    assert!(
                        st.max_width <= cap,
                        "encoder wrote a {}-bit code",
                        st.max_width
                    );
                    assert_same(&gif, cap);
                    // Every wider decoder plays it too.
                    if cap < MAX_CAP {
                        assert_same(&gif, cap + 1);
                    }
                }
            }
        }
    }

    #[test]
    fn long_strings_take_the_two_walk_path() {
        // One colour: strings grow by one per code. In a 128x32 frame they
        // stop at 90; in 256x64 they pass the on-stack buffer and span rows.
        let (gif, st) = capped_gif(128, 32, 4, &[1u8; 128 * 32], 12);
        assert_eq!(st.max_string, 90);
        assert_same(&gif, 12);
        let px = vec![1u8; 256 * 64];
        for cap in [9u8, 10, 12] {
            let (gif, st) = capped_gif(256, 64, 4, &px, cap);
            assert!(st.max_string > SHORT_STRING);
            assert_same(&gif, cap);
            let mut s = GifSlim::with_cap(&gif[..], cap).unwrap();
            s.next_frame(&mut Canvas::new()).unwrap();
            assert!(s.stats().long_strings > 0);
        }
    }

    #[test]
    fn wide_codes_are_needs_full_decoder() {
        // The gif crate's own encoder (weezl) lets the table grow to 12 bits.
        let mut rng = Rng(99);
        let px: Vec<u8> = (0..128 * 32).map(|_| rng.below(256) as u8).collect();
        let mut out = Vec::new();
        {
            let mut e = Encoder::new(&mut out, 128, 32, &palette(256)).unwrap();
            e.write_frame(&Frame {
                width: 128,
                height: 32,
                buffer: Cow::Borrowed(&px),
                ..Default::default()
            })
            .unwrap();
        }
        for cap in [9u8, 10, 11] {
            let e = play_slim(&out, cap).unwrap_err();
            assert!(is_needs_full_decoder(&e), "{e}");
            assert!(!is_unsupported(&e));
        }
        assert_same(&out, 12);
        // A 256-colour palette's literals alone need 9 bits.
        let e = play_slim(&out, 8).unwrap_err();
        assert!(is_needs_full_decoder(&e));
    }

    /// A GIF from frame specs through the gif crate's encoder: `(left, top,
    /// w, h, pixels, dispose, delay, local palette, transparent)`.
    #[allow(clippy::type_complexity)]
    fn build(
        w: u16,
        h: u16,
        global: Option<Vec<u8>>,
        frames: Vec<(
            u16,
            u16,
            u16,
            u16,
            Vec<u8>,
            DisposalMethod,
            u16,
            Option<Vec<u8>>,
            Option<u8>,
        )>,
        interlaced: bool,
    ) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let g = global.unwrap_or_default();
            let mut e = Encoder::new(&mut out, w, h, &g).unwrap();
            e.set_repeat(Repeat::Infinite).unwrap();
            for (left, top, fw, fh, px, dispose, delay, pal, transparent) in frames {
                e.write_frame(&Frame {
                    left,
                    top,
                    width: fw,
                    height: fh,
                    buffer: Cow::Owned(px),
                    dispose,
                    delay,
                    palette: pal,
                    transparent,
                    interlaced,
                    ..Default::default()
                })
                .unwrap();
            }
        }
        out
    }

    /// A little animation that uses sub-rectangles, transparency, Keep and
    /// Background disposal, and a local palette.
    fn animation(w: u16, h: u16, seed: u64) -> Vec<u8> {
        let mut rng = Rng(seed);
        let mut frames = Vec::new();
        for k in 0..12u16 {
            let fw = 1 + rng.below(w as usize) as u16;
            let fh = 1 + rng.below(h as usize) as u16;
            let left = rng.below((w - fw + 1) as usize) as u16;
            let top = rng.below((h - fh + 1) as usize) as u16;
            let px = noisy(&mut rng, fw as usize * fh as usize, 8, 6);
            let dispose = [
                DisposalMethod::Keep,
                DisposalMethod::Background,
                DisposalMethod::Any,
            ][rng.below(3)];
            let local = (k % 4 == 3).then(|| palette(8).iter().map(|b| b ^ 0x55).collect());
            frames.push((left, top, fw, fh, px, dispose, k % 5, local, Some(0)));
        }
        build(w, h, Some(palette(8)), frames, false)
    }

    #[test]
    fn delta_frame_with_transparency_background_disposal_and_local_palette() {
        for seed in 1..20 {
            let gif = animation(128, 32, seed);
            assert_same(&gif, 12);
            // weezl's codes grow freely: at a small cap it is either the
            // same output or a clean NeedsFullDecoder.
            match play_slim(&gif, 5) {
                Ok(f) => assert_eq!(f, play_gif(&gif).0),
                Err(e) => assert!(is_needs_full_decoder(&e), "{e}"),
            }
        }
    }

    #[test]
    fn background_disposal_clears_the_rectangle() {
        let red = vec![1u8; 16];
        let gif = build(
            128,
            32,
            Some(palette(4)),
            vec![
                (
                    0,
                    0,
                    128,
                    32,
                    vec![2u8; 4096],
                    DisposalMethod::Keep,
                    5,
                    None,
                    None,
                ),
                (10, 10, 4, 4, red, DisposalMethod::Background, 5, None, None),
                (0, 0, 1, 1, vec![0], DisposalMethod::Keep, 5, None, Some(0)),
            ],
            false,
        );
        assert_same(&gif, 9);
        let frames = play_slim(&gif, 9).unwrap();
        // The cleared rectangle is black on the last frame.
        let px = |x: usize, y: usize| &frames[2].0[(y * 128 + x) * 3..(y * 128 + x) * 3 + 3];
        assert_eq!(px(11, 11), &[0, 0, 0]);
        assert_eq!(px(0, 0), &palette(4)[6..9]);
    }

    #[test]
    fn local_palette_wins_and_the_global_one_returns() {
        let local: Vec<u8> = vec![9, 9, 9, 200, 100, 50];
        let gif = build(
            128,
            32,
            Some(palette(4)),
            vec![
                (
                    0,
                    0,
                    128,
                    32,
                    vec![1u8; 4096],
                    DisposalMethod::Keep,
                    5,
                    Some(local),
                    None,
                ),
                (
                    0,
                    0,
                    64,
                    32,
                    vec![1u8; 2048],
                    DisposalMethod::Keep,
                    5,
                    None,
                    None,
                ),
            ],
            false,
        );
        assert_same(&gif, 9);
        let f = play_slim(&gif, 9).unwrap();
        assert_eq!(&f[0].0[..3], &[200, 100, 50]);
        assert_eq!(&f[1].0[..3], &palette(4)[3..6]);
        assert_eq!(&f[1].0[127 * 3..128 * 3], &[200, 100, 50]);
    }

    #[test]
    fn interlace_and_previous_are_unsupported() {
        let gif = build(
            16,
            16,
            Some(palette(4)),
            vec![(
                0,
                0,
                16,
                16,
                vec![1u8; 256],
                DisposalMethod::Keep,
                5,
                None,
                None,
            )],
            true,
        );
        let e = play_slim(&gif, 12).unwrap_err();
        assert!(is_unsupported(&e), "{e}");
        let gif = build(
            16,
            16,
            Some(palette(4)),
            vec![(
                0,
                0,
                16,
                16,
                vec![1u8; 256],
                DisposalMethod::Previous,
                5,
                None,
                None,
            )],
            false,
        );
        assert!(is_unsupported(&play_slim(&gif, 12).unwrap_err()));
    }

    #[test]
    fn strays_are_centre_cropped_like_the_player() {
        for (w, h) in [(135u16, 32u16), (10, 5), (132, 40)] {
            assert_same(&animation(w, h, 7), 12);
        }
    }

    fn recode(gif: &[u8], cap: u8, delta: bool) -> (Vec<u8>, RecodeStats) {
        let mut out = Vec::new();
        let st = recode_gif(
            gif,
            &mut out,
            &RecodeOptions {
                cap,
                delta,
                hint: true,
                keep_runs: delta,
            },
        )
        .unwrap();
        (out, st)
    }

    #[test]
    fn recoded_gifs_play_like_the_original_at_their_cap() {
        for seed in 1..8 {
            let orig = animation(128, 32, seed);
            let (want, err) = play_gif(&orig);
            assert!(err.is_none());
            for cap in [9u8, 10, 11, 12] {
                for delta in [false, true] {
                    let (re, st) = recode(&orig, cap, delta);
                    assert!(st.lzw.max_width <= cap);
                    assert!(!st.quantised);
                    // A standard GIF: the gif crate plays it identically.
                    assert_eq!(play_gif(&re), (want.clone(), None));
                    let got = play_slim(&re, cap).unwrap();
                    assert!(got == want, "gifslim cap {cap} delta {delta}");
                    // The hint names the cap, so `new` picks it.
                    let s = GifSlim::new(&re[..]).unwrap();
                    assert_eq!(s.hint(), Some(Hint { cap, delta }));
                    assert_eq!(s.cap(), cap);
                }
            }
        }
    }

    #[test]
    fn recode_keeps_raw_delays_and_loop_count() {
        let orig = build(
            8,
            8,
            Some(palette(4)),
            vec![
                (
                    0,
                    0,
                    8,
                    8,
                    vec![1u8; 64],
                    DisposalMethod::Keep,
                    0,
                    None,
                    None,
                ),
                (
                    0,
                    0,
                    8,
                    8,
                    vec![2u8; 64],
                    DisposalMethod::Keep,
                    1,
                    None,
                    None,
                ),
                (
                    0,
                    0,
                    8,
                    8,
                    vec![2u8; 64],
                    DisposalMethod::Keep,
                    7,
                    None,
                    None,
                ),
            ],
            false,
        );
        let (re, st) = recode(&orig, 9, true);
        assert_eq!(st.frames, 3);
        let mut d = gif::DecodeOptions::new().read_info(&re[..]).unwrap();
        let mut delays = Vec::new();
        while let Some(f) = d.next_frame_info().unwrap() {
            delays.push(f.delay);
        }
        assert_eq!(delays, vec![0, 1, 7]);
        assert_eq!(d.repeat(), Repeat::Infinite);
        // The unchanged third frame is a single transparent pixel.
        assert_eq!(st.rect_px, 64 + 64 + 1);
    }

    #[test]
    fn many_colours_are_quantised_and_256_are_kept() {
        // 300 colours: NeuQuant, lossy; gif crate and gifslim still agree.
        let mut frames = Vec::new();
        for k in 0..3u16 {
            let pal: Vec<u8> = (0..100)
                .flat_map(|i: u16| [(i * 2) as u8, (k * 80) as u8, (255 - i) as u8])
                .collect();
            let px: Vec<u8> = (0..4096).map(|i| (i % 100) as u8).collect();
            frames.push((0, 0, 128, 32, px, DisposalMethod::Keep, 5, Some(pal), None));
        }
        let orig = build(128, 32, None, frames, false);
        let (re, st) = recode(&orig, 10, true);
        assert!(st.quantised);
        assert_eq!(st.colours, 255);
        let (want, _) = play_gif(&re);
        assert_eq!(play_slim(&re, 10).unwrap(), want);
        assert_eq!(want.len(), 3);

        // Exactly 256: kept exact, deltas without transparency.
        let pal = palette(256);
        let px: Vec<u8> = (0..4096).map(|i| (i % 256) as u8).collect();
        let px2: Vec<u8> = (0..4096).map(|i| ((i + 1) % 256) as u8).collect();
        let orig = build(
            128,
            32,
            Some(pal),
            vec![
                (0, 0, 128, 32, px, DisposalMethod::Keep, 5, None, None),
                (0, 0, 128, 32, px2, DisposalMethod::Keep, 5, None, None),
            ],
            false,
        );
        let (re, st) = recode(&orig, 9, true);
        assert!(!st.quantised && st.opaque_deltas);
        // The second frame changes every pixel through all 256 colours.
        assert_eq!(st.opaque_frames, 1);
        let (want, _) = play_gif(&orig);
        assert_eq!(play_gif(&re).0, want);
        assert_eq!(play_slim(&re, 9).unwrap(), want);

        // A small change borrows an index the changed pixels do not use as
        // that frame's transparent one.
        let px: Vec<u8> = (0..4096).map(|i| (i % 256) as u8).collect();
        let mut px3 = px.clone();
        for p in &mut px3[130..140] {
            *p = 7;
        }
        let orig = build(
            128,
            32,
            Some(palette(256)),
            vec![
                (0, 0, 128, 32, px, DisposalMethod::Keep, 5, None, None),
                (0, 0, 128, 32, px3, DisposalMethod::Keep, 5, None, None),
            ],
            false,
        );
        let (re, st) = recode(&orig, 9, true);
        assert!(st.opaque_deltas);
        assert_eq!(st.opaque_frames, 0);
        let mut d = gif::DecodeOptions::new().read_info(&re[..]).unwrap();
        d.next_frame_info().unwrap();
        let f = d.next_frame_info().unwrap().unwrap();
        assert_eq!(f.transparent, Some(0));
        assert!(f.width < 128 || f.height < 32);
        let (want, _) = play_gif(&orig);
        assert_eq!(play_gif(&re).0, want);
        assert_eq!(play_slim(&re, 9).unwrap(), want);
    }

    #[test]
    fn truncated_or_corrupt_input_errors_and_never_panics() {
        let (re, _) = recode(&animation(128, 32, 3), 10, true);
        for cut in 0..re.len() {
            let mut c = Canvas::new();
            if let Ok(mut s) = GifSlim::with_cap(&re[..cut], 10) {
                while let Ok(Some(_)) = s.next_frame(&mut c) {}
            }
        }
        let mut rng = Rng(5);
        for _ in 0..300 {
            let mut bad = re.clone();
            for _ in 0..4 {
                let i = rng.below(bad.len());
                bad[i] = rng.next() as u8;
            }
            let mut c = Canvas::new();
            if let Ok(mut s) = GifSlim::with_cap(Cursor::new(bad), 9) {
                for _ in 0..100 {
                    if !matches!(s.next_frame(&mut c), Ok(Some(_))) {
                        break;
                    }
                }
            }
        }
    }

    #[test]
    fn end_of_stream_leaves_the_canvas_alone() {
        let (re, _) = recode(&animation(128, 32, 4), 10, true);
        let mut s = GifSlim::with_cap(&re[..], 10).unwrap();
        let mut c = Canvas::new();
        while s.next_frame(&mut c).unwrap().is_some() {}
        let before = c.as_bytes().to_vec();
        c.dirty_mut().clear();
        assert_eq!(s.next_frame(&mut c).unwrap(), None);
        assert_eq!(c.as_bytes(), &before[..]);
        assert!(c.dirty().is_clean());
    }

    #[test]
    fn dirty_map_matches_the_players() {
        // Every changed pixel is marked, and the over-marking (a pixel a
        // Background disposal clears and the next frame repaints) is
        // exactly GifSource's.
        for seed in 1..6 {
            let gif = animation(128, 32, seed);
            for g in [gif.clone(), recode(&gif, 10, true).0] {
                let mut s = GifSlim::with_cap(&g[..], 12).unwrap();
                let mut p = GifSource::new(&g[..]).unwrap();
                let (mut c, mut cp) = (Canvas::new(), Canvas::new());
                loop {
                    let before = c.as_bytes().to_vec();
                    c.dirty_mut().clear();
                    cp.dirty_mut().clear();
                    let d = s.next_frame(&mut c).unwrap();
                    assert_eq!(d, p.next_frame(&mut cp).unwrap());
                    if d.is_none() {
                        break;
                    }
                    for y in 0..HEIGHT {
                        for x in 0..WIDTH {
                            let o = (y * WIDTH + x) * 3;
                            let changed = before[o..o + 3] != c.as_bytes()[o..o + 3];
                            assert!(!changed || c.dirty().is_dirty(x, y), "({x},{y})");
                            assert_eq!(c.dirty().is_dirty(x, y), cp.dirty().is_dirty(x, y));
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn first_frame_is_whole_and_opaque() {
        // What a browser shows of frame 0 must not depend on starting black.
        let (re, _) = recode(&animation(128, 32, 9), 10, true);
        let mut o = gif::DecodeOptions::new();
        o.set_color_output(gif::ColorOutput::Indexed);
        let mut d = o.read_info(&re[..]).unwrap();
        let f = d.read_next_frame().unwrap().unwrap();
        assert_eq!((f.left, f.top, f.width, f.height), (0, 0, 128, 32));
        let t = f.transparent.unwrap();
        assert!(f.buffer.iter().all(|&i| i != t));
    }
}

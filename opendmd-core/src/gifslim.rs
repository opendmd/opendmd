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
    MAX_LOGICAL_PIXELS,
};
use gif::DisposalMethod;
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::time::Duration;

/// The decoder's read buffer (as `OdmSource`'s).
pub const READ_BUF: usize = 2048;

/// What an open refused for the heap says (`ErrorKind::OutOfMemory`).
pub const NO_MEMORY: &str = "no memory to open the GIF just now";

/// `vec![x; n]` taken fallibly: [`NO_MEMORY`] when the heap has no block
/// that size.
fn try_filled<T: Clone>(x: T, n: usize) -> io::Result<Vec<T>> {
    let mut v = Vec::new();
    v.try_reserve_exact(n)
        .map_err(|_| io::Error::new(io::ErrorKind::OutOfMemory, NO_MEMORY))?;
    v.resize(n, x);
    Ok(v)
}
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
    /// The NETSCAPE loop count, as the `gif` crate reports it (the last one
    /// before the first frame; `Finite(0)` if none).
    repeat: gif::Repeat,
    /// The last frame's delay as written (hundredths).
    raw_delay: u16,
    ended: bool,
    frames_decoded: u64,
    stats: SlimStats,
}

/// Next byte from a `BufReader`; end of file is `UnexpectedEof`, as in the
/// `gif` crate. Not inlined: it is on no per-pixel path (sub-block lengths,
/// headers), and inlined at every call it cost the image and the browser's
/// wasm a few KB.
#[inline(never)]
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

#[inline(never)]
fn read_exact<R: Read>(r: &mut BufReader<R>, buf: &mut [u8]) -> io::Result<()> {
    r.read_exact(buf)
}

#[inline(never)]
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

/// Parse an application extension (the `0x21 0xFF` already consumed):
/// our hint, or a NETSCAPE loop count (read as the `gif` crate reads it:
/// the sub-blocks after the identifier concatenated, `1` then a `u16`).
fn read_app_ext<R: Read>(r: &mut BufReader<R>) -> io::Result<(Option<Hint>, Option<gif::Repeat>)> {
    let mut id = [0u8; 11];
    let len = byte(r)? as usize;
    if len == 0 {
        return Ok((None, None));
    }
    if len != 11 {
        skip(r, len)?;
        skip_sub_blocks(r)?;
        return Ok((None, None));
    }
    read_exact(r, &mut id)?;
    if &id == b"NETSCAPE2.0" {
        let (mut d, mut got) = ([0u8; 3], 0usize);
        loop {
            let n = byte(r)? as usize;
            if n == 0 {
                break;
            }
            for _ in 0..n {
                let b = byte(r)?;
                if got < 3 {
                    d[got] = b;
                }
                got += 1;
            }
        }
        let repeat = (got >= 3 && d[0] == 1).then(|| match u16::from_le_bytes([d[1], d[2]]) {
            0 => gif::Repeat::Infinite,
            n => gif::Repeat::Finite(n),
        });
        return Ok((None, repeat));
    }
    // ImageMagick keeps unknown application extensions through a rewrite
    // but lowercases the identifier (and re-encodes the LZW, so a hint is
    // a promise to check, never a guarantee: `NeedsFullDecoder` stands).
    if !id.eq_ignore_ascii_case(HINT_APP_ID) {
        skip_sub_blocks(r)?;
        return Ok((None, None));
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
    Ok((hint, None))
}

/// Read the header, the logical screen descriptor and the global palette;
/// then any application/comment extensions that come before the first
/// frame (where the hint lives), stopping at anything else without
/// consuming it.
#[allow(clippy::type_complexity)]
fn read_header<R: Read>(
    r: &mut BufReader<R>,
) -> io::Result<(usize, usize, Vec<u8>, Option<Hint>, gif::Repeat)> {
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
        global = try_filled(0u8, 3 << ((flags & 7) + 1))?;
        read_exact(r, &mut global)?;
    }
    let mut hint = None;
    let mut repeat = gif::Repeat::Finite(0);
    loop {
        let buf = r.fill_buf()?;
        if buf.len() < 2 || buf[0] != 0x21 || !(buf[1] == 0xFF || buf[1] == 0xFE) {
            break;
        }
        let label = buf[1];
        r.consume(2);
        if label == 0xFF {
            let (h, rep) = read_app_ext(r)?;
            hint = h.or(hint);
            repeat = rep.unwrap_or(repeat);
        } else {
            skip_sub_blocks(r)?;
        }
    }
    Ok((width, height, global, hint, repeat))
}

/// What a GIF's first bytes say, read without a decoder ([`probe`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Probe {
    pub width: usize,
    pub height: usize,
    pub hint: Option<Hint>,
}

/// The logical screen and the hint [`GifSlim::new`] would open `r` with,
/// read as [`read_header`] reads them but straight off `r`, a few bytes at a
/// time, seeking past the palette and the other extensions — no
/// `BufReader`, no palette, no tables — and `r` rewound to 0 after. For a
/// caller deciding whether a file may be opened at all before paying for
/// its decoder (the engine's file held ahead, at most [`PREPARED`]'s cap):
/// on the device each read is a copy out of FATFS's sector buffer for the
/// file. An error is the reader's, or what [`GifSlim::new`] would return
/// for the same header (a file that ends inside its palette reads as one
/// with no hint; the decoder refuses it when it opens).
pub fn probe<R: Read + Seek>(r: &mut R) -> io::Result<Probe> {
    fn truncated(e: io::Error) -> io::Error {
        if e.kind() == io::ErrorKind::UnexpectedEof {
            invalid("GIF header is truncated")
        } else {
            e
        }
    }
    fn skip_blocks<R: Read + Seek>(r: &mut R) -> io::Result<()> {
        let mut len = [0u8; 1];
        loop {
            r.read_exact(&mut len).map_err(truncated)?;
            if len[0] == 0 {
                return Ok(());
            }
            r.seek(SeekFrom::Current(i64::from(len[0])))?;
        }
    }
    let mut h = [0u8; 13];
    r.read_exact(&mut h).map_err(truncated)?;
    if &h[..6] != b"GIF87a" && &h[..6] != b"GIF89a" {
        return Err(invalid("malformed GIF header"));
    }
    let width = u16::from_le_bytes([h[6], h[7]]) as usize;
    let height = u16::from_le_bytes([h[8], h[9]]) as usize;
    if width == 0 || height == 0 || width * height > MAX_LOGICAL_PIXELS {
        return Err(invalid("GIF logical screen is zero or implausibly large"));
    }
    if h[10] & 0x80 != 0 {
        r.seek(SeekFrom::Current(3 << ((h[10] & 7) + 1)))?;
    }
    // The extensions before the first frame, as `read_header` takes them:
    // the last hint wins, and anything but an application or comment
    // extension (or the end of the file) ends the look.
    let mut hint = None;
    loop {
        let mut intro = [0u8; 2];
        match r.read_exact(&mut intro) {
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(e),
            Ok(()) if intro[0] != 0x21 || !(intro[1] == 0xFF || intro[1] == 0xFE) => break,
            Ok(()) if intro[1] == 0xFE => {
                skip_blocks(r)?;
                continue;
            }
            Ok(()) => {}
        }
        let mut id = [0u8; 12];
        r.read_exact(&mut id[..1]).map_err(truncated)?;
        match id[0] {
            0 => continue,
            11 => r.read_exact(&mut id[1..]).map_err(truncated)?,
            n => {
                r.seek(SeekFrom::Current(i64::from(n)))?;
                skip_blocks(r)?;
                continue;
            }
        }
        if !id[1..].eq_ignore_ascii_case(HINT_APP_ID) {
            skip_blocks(r)?;
            continue;
        }
        // `[len, version, cap, flags]`, as `read_app_ext`.
        let mut d = [0u8; 4];
        r.read_exact(&mut d[..1]).map_err(truncated)?;
        let len = d[0] as usize;
        let k = if len >= 2 { len.min(3) } else { 0 };
        r.read_exact(&mut d[1..=k]).map_err(truncated)?;
        r.seek(SeekFrom::Current((len - k) as i64))?;
        if k > 0 && d[1] == 1 && (MIN_CAP..=MAX_CAP).contains(&d[2]) {
            hint = Some(Hint {
                cap: d[2],
                delta: d[3] & 1 != 0,
            });
        }
        if len != 0 {
            skip_blocks(r)?;
        }
    }
    r.seek(SeekFrom::Start(0))?;
    Ok(Probe {
        width,
        height,
        hint,
    })
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
    /// Indices for columns `x0..` of frame row `y`. (In IRAM in a
    /// firmware without Formulas, with [`Lzw::run`], in the room their code
    /// leaves: the two are ~70 % of the instructions a GIF frame fetches
    /// from flash.)
    #[inline]
    #[cfg_attr(all(target_os = "espidf", not(feature = "psi")), link_section = ".iram1.gif")]
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
    #[cfg_attr(all(target_os = "espidf", not(feature = "psi")), link_section = ".iram1.gif")]
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
        let (width, height, global, hint, repeat) = read_header(&mut reader)?;
        let cap = cap.or(hint.map(|h| h.cap)).unwrap_or(DEFAULT_CAP);
        if !(MIN_CAP..=MAX_CAP).contains(&cap) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("LZW cap {cap} is not in {MIN_CAP}..={MAX_CAP}"),
            ));
        }
        // Its tables and screen fallibly (8 KB + 4 KB at cap 12, a whole
        // RGB screen for a GIF not 128x32): a fragmented heap is an open
        // that fails (the player skips the GIF) rather than an abort.
        let prefix = try_filled(0u16, 1 << cap)?;
        let suffix = try_filled(0u8, 1 << cap)?;
        let row = try_filled(0u8, width)?;
        let screen = if width == WIDTH && height == HEIGHT {
            Screen::Direct { cleared: false }
        } else {
            Screen::Owned(try_filled(0u8, width * height * 3)?)
        };
        Ok(GifSlim {
            reader,
            width,
            height,
            cap,
            prefix,
            suffix,
            row,
            global,
            screen,
            pending: None,
            hint,
            repeat,
            raw_delay: 0,
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

    /// The NETSCAPE loop count as the `gif` crate's `Decoder::repeat` gives
    /// it (complete once the first frame has been read).
    pub fn repeat(&self) -> gif::Repeat {
        self.repeat
    }

    /// The last frame's delay as written in the file, in hundredths (the
    /// `Duration` [`next_frame`](Self::next_frame) returns has the player's
    /// 0 -> 100 ms and 20 ms-floor rules applied).
    pub fn raw_delay(&self) -> u16 {
        self.raw_delay
    }

    /// The RGB logical screen as of the last frame (`width * height * 3`
    /// bytes), before any centre-crop: the caller's `canvas` itself at the
    /// panel size, so it must be the one passed to `next_frame`.
    pub fn screen<'a>(&'a self, canvas: &'a Canvas) -> &'a [u8] {
        match &self.screen {
            Screen::Direct { .. } => canvas.as_bytes(),
            Screen::Owned(v) => v,
        }
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
                            let (h, rep) = read_app_ext(&mut self.reader)?;
                            if let Some(h) = h {
                                self.hint = Some(h);
                            }
                            // The crate only takes loop counts from before
                            // the first image.
                            if let (Some(rep), 0) = (rep, self.frames_decoded) {
                                self.repeat = rep;
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
        // The frame's palette as a 256-entry table — a local one read
        // straight into it (no second 768-byte buffer on the stack: the
        // device's cache builder runs this on an 8.5 KB task) — and the
        // indices that leave the screen alone: the transparent one and any
        // past the palette's end.
        let mut rgb = [[0u8; 3]; 256];
        let mut skip = [true; 256];
        let entries = if flags & 0x80 != 0 {
            let n = 2usize << (flags & 7);
            read_exact(&mut self.reader, rgb[..n].as_flattened_mut())?;
            n
        } else if !self.global.is_empty() {
            let n = self.global.len() / 3;
            rgb[..n].as_flattened_mut().copy_from_slice(&self.global);
            n
        } else {
            return Err(invalid("no color table available for current frame"));
        };
        skip[..entries].fill(false);
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
        self.raw_delay = delay;
        if let Screen::Owned(v) = &self.screen {
            blit(v, width, height, canvas);
        }
        Ok(Some(frame_delay(delay)))
    }
}

// ---------------------------------------------------------------------------
// Capped LZW encoder
// ---------------------------------------------------------------------------

/// What an [`LzwEncoder`] emitted, summed over the frames it encoded.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LzwStats {
    pub codes: u64,
    /// CLEARs after the first of each frame (the cap at work).
    pub clears: u64,
    /// Widest code written.
    pub max_width: u8,
    /// Longest string one code stood for.
    pub max_string: usize,
}

/// Where an [`LzwEncoder`]'s bytes go: the raw code stream into a `Vec`
/// ([`lzw_encode_capped`]), or cut into a GIF's data sub-blocks straight
/// into a writer ([`SubBlocks`]).
pub trait LzwSink {
    fn byte(&mut self, b: u8) -> io::Result<()>;
}

impl LzwSink for Vec<u8> {
    #[inline]
    fn byte(&mut self, b: u8) -> io::Result<()> {
        self.push(b);
        Ok(())
    }
}

/// GIF data sub-blocks (`len` + up to 255 bytes) written as they fill.
/// [`finish`](Self::finish) writes the last one and the zero terminator.
pub struct SubBlocks<'a, W: Write> {
    out: &'a mut W,
    buf: [u8; 256],
}

impl<'a, W: Write> SubBlocks<'a, W> {
    pub fn new(out: &'a mut W) -> Self {
        SubBlocks { out, buf: [0; 256] }
    }

    pub fn finish(self) -> io::Result<()> {
        let n = self.buf[0] as usize;
        if n > 0 {
            self.out.write_all(&self.buf[..=n])?;
        }
        self.out.write_all(&[0])
    }
}

impl<W: Write> LzwSink for SubBlocks<'_, W> {
    #[inline]
    fn byte(&mut self, b: u8) -> io::Result<()> {
        let n = self.buf[0] as usize + 1;
        self.buf[n] = b;
        self.buf[0] = n as u8;
        if n == 255 {
            self.out.write_all(&self.buf)?;
            self.buf[0] = 0;
        }
        Ok(())
    }
}

/// GIF-flavoured LZW, fed one palette index at a time, such that **no code
/// is wider than `cap` bits**: a CLEAR is emitted as the code the decoder
/// reads when its next free entry is `2^cap - 1`, the last point at which a
/// code (the CLEAR itself) still fits in `cap` bits. So a decoder with
/// `2^cap`-entry tables plays it; any standard decoder does too.
///
/// Code widths follow the GIF rule every decoder uses (giflib, `weezl`): a
/// code is read with as many bits as the decoder's next free entry needs.
/// `cap = 12` is an ordinary GIF encoder that clears one entry early.
///
/// The dictionary is an open-addressing hash of `2^(cap+1)` slots (`u32`
/// key + `u16` code: 6 KB at cap 9), allocated once in [`new`](Self::new)
/// and emptied at every CLEAR; one encoder serves every frame of a file
/// ([`begin`](Self::begin) … [`push`](Self::push) … [`end`](Self::end)).
pub struct LzwEncoder {
    cap: u8,
    keys: Vec<u32>,
    vals: Vec<u16>,
    min: u32,
    clear: u16,
    first: u16,
    /// The decoder's next free entry once it has read everything emitted.
    next: u16,
    /// The next code the decoder reads is the first after a CLEAR (it
    /// defines nothing).
    fresh: bool,
    /// The string being extended (a code), if any.
    w: Option<u16>,
    wlen: usize,
    acc: u64,
    nbits: u32,
    stats: LzwStats,
}

#[inline]
fn width_of(next: u16) -> u32 {
    (16 - next.leading_zeros()).min(12)
}

impl LzwStats {
    /// Sum of two runs (counts add, maxima take the larger).
    pub fn add(&mut self, o: LzwStats) {
        self.codes += o.codes;
        self.clears += o.clears;
        self.max_width = self.max_width.max(o.max_width);
        self.max_string = self.max_string.max(o.max_string);
    }
}

impl LzwEncoder {
    pub fn new(cap: u8) -> Self {
        assert!(
            (MIN_CAP..=MAX_CAP).contains(&cap),
            "cap {cap} is not in {MIN_CAP}..={MAX_CAP}"
        );
        let slots = 1usize << (cap + 1);
        LzwEncoder {
            cap,
            keys: vec![0; slots],
            vals: vec![0; slots],
            min: 2,
            clear: 4,
            first: 6,
            next: 6,
            fresh: true,
            w: None,
            wlen: 0,
            acc: 0,
            nbits: 0,
            stats: LzwStats::default(),
        }
    }

    pub fn stats(&self) -> LzwStats {
        self.stats
    }

    /// Heap this encoder owns (the dictionary).
    pub fn heap_bytes(&self) -> usize {
        self.keys.len() * 4 + self.vals.len() * 2
    }

    #[inline]
    fn emit(&mut self, code: u16, out: &mut impl LzwSink) -> io::Result<()> {
        let w = width_of(self.next);
        debug_assert!(w <= self.cap as u32);
        self.acc |= (code as u64) << self.nbits;
        self.nbits += w;
        while self.nbits >= 8 {
            out.byte(self.acc as u8)?;
            self.acc >>= 8;
            self.nbits -= 8;
        }
        self.stats.codes += 1;
        self.stats.max_width = self.stats.max_width.max(w as u8);
        Ok(())
    }

    fn reset(&mut self) {
        self.keys.fill(0);
        self.next = self.first;
        self.fresh = true;
    }

    /// Start a frame's code stream (indices `< 2^min_code_size`): the
    /// initial CLEAR.
    pub fn begin(&mut self, min_code_size: u8, out: &mut impl LzwSink) -> io::Result<()> {
        assert!(
            (2..=11).contains(&min_code_size),
            "GIF minimum code size is 2..=11"
        );
        assert!(
            self.cap > min_code_size,
            "cap {} cannot hold the literals of code size {min_code_size}",
            self.cap
        );
        self.min = min_code_size as u32;
        self.clear = 1 << self.min;
        self.first = self.clear + 2;
        self.reset();
        self.w = None;
        self.wlen = 0;
        self.acc = 0;
        self.nbits = 0;
        self.emit(self.clear, out)
    }

    /// One more index.
    #[inline]
    pub fn push(&mut self, c: u8, out: &mut impl LzwSink) -> io::Result<()> {
        debug_assert!((c as u16) < self.clear);
        let Some(w) = self.w else {
            self.w = Some(c as u16);
            self.wlen = 1;
            return Ok(());
        };
        let key = ((w as u32) << 8 | c as u32) + 1;
        let mask = self.keys.len() - 1;
        let mut s = (key.wrapping_mul(0x9E37_79B1) >> 7) as usize & mask;
        loop {
            let k = self.keys[s];
            if k == key {
                self.w = Some(self.vals[s]);
                self.wlen += 1;
                return Ok(());
            }
            if k == 0 {
                break;
            }
            s = (s + 1) & mask;
        }
        // Emit w. The decoder, reading it, defines entry `next` (unless it
        // is the first code after a CLEAR); the encoder defines the same
        // entry, w + c, now.
        self.emit(w, out)?;
        self.stats.max_string = self.stats.max_string.max(self.wlen);
        if !self.fresh {
            self.next += 1;
        }
        self.fresh = false;
        // The encoder's dictionary runs one entry ahead of the decoder's:
        // w + c becomes entry `next`, which the decoder defines on reading
        // the code after this one.
        if self.next >= (1u16 << self.cap) - 1 {
            // The next code would be read at `width_of(2^cap - 1)` = cap
            // bits; after one more entry it would need cap + 1. Clear now.
            self.emit(self.clear, out)?;
            self.stats.clears += 1;
            self.reset();
        } else {
            self.keys[s] = key;
            self.vals[s] = self.next;
        }
        self.w = Some(c as u16);
        self.wlen = 1;
        Ok(())
    }

    /// The last string, END, and the bits still in hand.
    pub fn end(&mut self, out: &mut impl LzwSink) -> io::Result<()> {
        if let Some(w) = self.w.take() {
            self.emit(w, out)?;
            self.stats.max_string = self.stats.max_string.max(self.wlen);
            if !self.fresh {
                self.next += 1;
            }
        }
        self.emit(self.clear + 1, out)?;
        if self.nbits > 0 {
            out.byte(self.acc as u8)?;
            self.acc = 0;
            self.nbits = 0;
        }
        Ok(())
    }
}

/// [`LzwEncoder`] over a whole slice, appending the raw code stream (not cut
/// into sub-blocks) to `out`.
pub fn lzw_encode_capped(data: &[u8], min_code_size: u8, cap: u8, out: &mut Vec<u8>) -> LzwStats {
    let mut enc = LzwEncoder::new(cap);
    // A `Vec` sink cannot fail.
    let run = |enc: &mut LzwEncoder, out: &mut Vec<u8>| -> io::Result<()> {
        enc.begin(min_code_size, out)?;
        for &c in data {
            enc.push(c, out)?;
        }
        enc.end(out)
    };
    run(&mut enc, out).expect("writing to a Vec");
    enc.stats()
}

// ---------------------------------------------------------------------------
// Recoder: the prepared copy
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

/// **The prepared copy's format** (`docs/cache-format.md`): LZW capped at 9
/// bits, delta frames with `keep_runs`, the hint. What `opendmd-pack
/// convert`, the browser and the device's cache builder all write.
pub const PREPARED: RecodeOptions = RecodeOptions {
    cap: 9,
    delta: true,
    hint: true,
    keep_runs: true,
};

/// What [`recode_gif`] produced.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RecodeStats {
    pub width: usize,
    pub height: usize,
    pub frames: u32,
    /// Distinct colours in the palette written (not counting a transparent
    /// slot).
    pub colours: usize,
    /// Exactly 256 colours with delta frames: no slot to spare, so each
    /// frame's transparent index is one its changed pixels do not use.
    pub opaque_deltas: bool,
    /// Frames among those whose changed pixels use all 256 colours: sent as
    /// the changed rectangle with no transparency.
    pub opaque_frames: u64,
    /// The source's length.
    pub gif_bytes: u64,
    pub out_bytes: u64,
    /// Pixels written in frame rectangles, over all frames.
    pub rect_px: u64,
    /// The source needed the `gif` crate (interlaced frames or `Previous`
    /// disposal, which gifslim does not decode).
    pub full_decoder: bool,
    pub lzw: LzwStats,
}

/// The recoder met a 257th distinct colour: an exact copy is impossible, so
/// **there is no copy** and the player plays the GIF itself (gifslim at cap
/// 12 plays it exactly; a quantised copy would only make it worse —
/// `opendmd-notes docs/research/cache-value.md`). Every caller — the device's builder,
/// `opendmd-pack`, the browser — records it with [`ORIGINAL_REASON`] and
/// counts it as done. Carried inside an `io::Error` of kind `InvalidData`;
/// test with [`is_too_many_colours`]. Nothing has been written when it is
/// returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TooManyColours;

impl std::fmt::Display for TooManyColours {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(ORIGINAL_REASON)
    }
}
impl std::error::Error for TooManyColours {}

pub fn is_too_many_colours(e: &io::Error) -> bool {
    e.get_ref().is_some_and(|i| i.is::<TooManyColours>())
}

/// The reason recorded in `/cache/.failed` for a GIF that deliberately has
/// no copy because it has more than 256 colours. [`plays_original`]
/// recognises it, and the bare `"more than 256 colours"` earlier firmware
/// wrote.
pub const ORIGINAL_REASON: &str = "more than 256 colours: plays as it is";

/// Does a `/cache/.failed` reason mean "no copy on purpose: the GIF plays
/// as it is"? Such a GIF counts as done (`/api/cache`'s `prepared`), not as
/// failed, and is never tried again.
pub fn plays_original(reason: &str) -> bool {
    let r = reason.trim_start().as_bytes();
    let p = b"more than 256 colo";
    r.len() >= p.len() && r[..p.len()].eq_ignore_ascii_case(p)
}

/// Could the copy whose first bytes are `head` be a quantised one (made
/// before copies were exact-only)? A quantised copy always has a 256-entry
/// global table (255 `NeuQuant` colours and the transparent slot), so a
/// copy with a smaller table is exact; with a 256-entry table only the
/// GIF's colour count ([`fits_palette`]) can tell.
pub fn copy_may_be_quantised(head: &[u8]) -> bool {
    head.len() >= 13 && head.starts_with(b"GIF8") && head[10] & 0x87 == 0x87
}

/// Does the GIF have at most 256 distinct colours (on its composited
/// frames), i.e. can its prepared copy be exact? The recoder's first pass
/// alone; it stops at the 257th colour.
pub fn fits_palette<R: Read + Seek>(reader: R) -> io::Result<bool> {
    let mut colours = Colours::new();
    let mut fits = true;
    scan(
        reader,
        false,
        &mut || true,
        &mut || Ok(()),
        &mut |k, screen| {
            if k == 0 {
                // (A pass gifslim gave up part way is started again.)
                colours = Colours::new();
            }
            fits = screen.as_chunks::<3>().0.iter().all(|&c| colours.see(c));
            Ok(fits)
        },
    )?;
    Ok(fits)
}

/// The source GIF's decoder: gifslim with the full 12-bit table (≈15 KB),
/// or — for what gifslim does not decode (interlacing, `Previous`) — the
/// `gif` crate (≈41 KB). Both give the same frames (`gifscan`: 0
/// mismatches over the card).
// One per encode, so the variants' sizes (~400 vs ~600 B inline) do not
// matter; boxing would be one more allocation.
#[allow(clippy::large_enum_variant)]
enum SourceGif<R: Read> {
    Slim(GifSlim<R>),
    Full(GifSource<R>),
}

impl<R: Read> SourceGif<R> {
    fn open(reader: R, full: bool) -> io::Result<Self> {
        Ok(if full {
            SourceGif::Full(GifSource::new(reader)?)
        } else {
            SourceGif::Slim(GifSlim::with_cap(reader, MAX_CAP)?)
        })
    }

    /// The next frame's raw delay (hundredths), or `None` at the end.
    fn next(&mut self, canvas: &mut Canvas) -> io::Result<Option<u16>> {
        Ok(match self {
            SourceGif::Slim(s) => s.next_frame(canvas)?.map(|_| s.raw_delay()),
            SourceGif::Full(s) => s.next_frame(canvas)?.map(|_| s.raw_delay()),
        })
    }

    fn screen<'a>(&'a self, canvas: &'a Canvas) -> &'a [u8] {
        match self {
            SourceGif::Slim(s) => s.screen(canvas),
            SourceGif::Full(s) => s.screen(canvas),
        }
    }

    fn logical_size(&self) -> (usize, usize) {
        match self {
            SourceGif::Slim(s) => s.logical_size(),
            SourceGif::Full(s) => s.logical_size(),
        }
    }

    fn repeat(&self) -> gif::Repeat {
        match self {
            SourceGif::Slim(s) => s.repeat(),
            SourceGif::Full(s) => s.repeat(),
        }
    }

    fn into_inner(self) -> R {
        match self {
            SourceGif::Slim(s) => s.into_inner(),
            SourceGif::Full(s) => s.into_inner(),
        }
    }
}

/// Should the recoder give up gifslim for the `gif` crate on this error?
fn wants_full_decoder(e: &io::Error) -> bool {
    is_unsupported(e) || is_needs_full_decoder(e)
}

/// Slots in the exact colour table: a power of two, twice 256.
const COLOUR_SLOTS: usize = 512;
const NO_COLOUR: u32 = u32::MAX;

/// The source's colours in first-seen order (scanning each composited
/// frame in raster order) and an RGB -> index lookup: 512 `u32` keys and
/// 512 `u8` indices by open addressing, plus the palette (~2.8 KB).
struct Colours {
    keys: Vec<u32>,
    index: Vec<u8>,
    palette: Vec<[u8; 3]>,
    /// The last colour looked up and its index (runs are common).
    last: (u32, u8),
}

impl Colours {
    fn new() -> Self {
        Colours {
            keys: vec![NO_COLOUR; COLOUR_SLOTS],
            index: vec![0; COLOUR_SLOTS],
            palette: Vec::with_capacity(256),
            last: (NO_COLOUR, 0),
        }
    }

    #[inline]
    fn slot(&self, rgb: u32) -> usize {
        let mut s = (rgb.wrapping_mul(0x9E37_79B1) >> 23) as usize & (COLOUR_SLOTS - 1);
        while self.keys[s] != rgb && self.keys[s] != NO_COLOUR {
            s = (s + 1) & (COLOUR_SLOTS - 1);
        }
        s
    }

    /// Record a colour seen; `false` for a 257th.
    #[inline]
    fn see(&mut self, c: [u8; 3]) -> bool {
        let rgb = u32::from_be_bytes([0, c[0], c[1], c[2]]);
        if rgb == self.last.0 {
            return true;
        }
        let s = self.slot(rgb);
        if self.keys[s] == NO_COLOUR {
            if self.palette.len() == 256 {
                return false;
            }
            self.keys[s] = rgb;
            self.index[s] = self.palette.len() as u8;
            self.palette.push(c);
        }
        self.last = (rgb, self.index[s]);
        true
    }
}

/// Palette index of each composited pixel, for the second pass.
trait Indexer {
    fn index_of(&mut self, c: [u8; 3]) -> io::Result<u8>;
}

impl Indexer for Colours {
    #[inline]
    fn index_of(&mut self, c: [u8; 3]) -> io::Result<u8> {
        let rgb = u32::from_be_bytes([0, c[0], c[1], c[2]]);
        if rgb == self.last.0 {
            return Ok(self.last.1);
        }
        let s = self.slot(rgb);
        if self.keys[s] == NO_COLOUR {
            return Err(invalid("GIF decoded differently on the second pass"));
        }
        self.last = (rgb, self.index[s]);
        Ok(self.index[s])
    }
}

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

/// What the first pass learnt.
struct Scan {
    width: usize,
    height: usize,
    frames: u32,
    repeat: gif::Repeat,
    full: bool,
}

/// Decode every frame of `reader` (from the start) with `visit` on each
/// logical screen, falling back to the `gif` crate if gifslim cannot
/// decode the file and `full_ok()` allows it. The reader is handed back
/// rewound.
fn scan<R: Read + Seek>(
    mut reader: R,
    mut full: bool,
    full_ok: &mut dyn FnMut() -> bool,
    between: &mut dyn FnMut() -> io::Result<()>,
    visit: &mut dyn FnMut(u32, &[u8]) -> io::Result<bool>,
) -> io::Result<(R, Scan)> {
    let mut canvas = Canvas::new();
    loop {
        reader.seek(SeekFrom::Start(0))?;
        let mut src = SourceGif::open(reader, full)?;
        let mut frames = 0u32;
        let res = loop {
            between()?;
            match src.next(&mut canvas) {
                Ok(Some(_)) => {
                    let k = frames;
                    frames = frames
                        .checked_add(1)
                        .ok_or_else(|| invalid("too many frames"))?;
                    if !visit(k, src.screen(&canvas))? {
                        break Ok(false);
                    }
                }
                Ok(None) => break Ok(true),
                Err(e) => break Err(e),
            }
        };
        match res {
            Err(e) if !full && wants_full_decoder(&e) => {
                if !full_ok() {
                    return Err(e);
                }
                reader = src.into_inner();
                full = true;
                continue;
            }
            Err(e) => return Err(e),
            Ok(_) => {}
        }
        let (width, height) = src.logical_size();
        let repeat = src.repeat();
        let mut reader = src.into_inner();
        reader.seek(SeekFrom::Start(0))?;
        return Ok((
            reader,
            Scan {
                width,
                height,
                frames,
                repeat,
                full,
            },
        ));
    }
}

/// Re-encode a GIF as the prepared copy (or any [`RecodeOptions`]) with
/// **bounded memory**: the encoder the device's cache builder runs, and —
/// through [`recode_gif`] — the one `opendmd-pack` and the browser run, so
/// all three write the same bytes.
///
/// Two passes over `reader` (rewound for each): the first collects the
/// colours (first seen, scanning each composited frame in raster order),
/// the frame count, the size and the loop count; the second decodes again
/// and writes the GIF, a frame at a time, the LZW streamed into `out` as it
/// is made. Nothing is written before the second pass, so an error from the
/// first ([`TooManyColours`] past 256 colours, a GIF that will not decode)
/// leaves `out` untouched.
///
/// Frames are decoded with [`GifSlim`] at cap 12; a file it does not decode
/// (interlaced, `Previous` disposal: 5 of the card's 11,047) is decoded with
/// the `gif` crate instead if `full_ok()` says so (the device asks whether
/// its ~41 KB are free), else that `Unsupported` error is returned.
/// `between` runs before every frame of both passes; an error from it
/// abandons the encode (the device's pause).
///
/// Heap for a 128x32 GIF at the prepared format's cap 9: the decoder
/// (~15 KB: 2 KB read buffer, 12 KB of tables, a row, the source's
/// palette), a canvas (12 KB), the previous frame's indices (`w*h`), the
/// changed bits (`w*h/8`), the colour table (~2.8 KB), the LZW dictionary
/// (6 KB) — ~42 KB, plus whatever buffering `out` has.
pub fn recode_gif_streaming<R: Read + Seek, W: Write>(
    reader: R,
    mut out: W,
    opts: &RecodeOptions,
    mut full_ok: impl FnMut() -> bool,
    mut between: impl FnMut() -> io::Result<()>,
) -> io::Result<RecodeStats> {
    check_opts(opts)?;
    let mut colours = Colours::new();
    let mut reader = reader;
    let gif_bytes = reader.seek(SeekFrom::End(0))?;
    let mut too_many = false;
    let (reader, sc) = scan(
        reader,
        false,
        &mut full_ok,
        &mut between,
        &mut |_, screen| {
            for &c in screen.as_chunks::<3>().0 {
                if !colours.see(c) {
                    too_many = true;
                    return Ok(false);
                }
            }
            Ok(true)
        },
    )?;
    if too_many {
        return Err(io::Error::new(io::ErrorKind::InvalidData, TooManyColours));
    }
    let palette = std::mem::take(&mut colours.palette);
    write_gif(
        reader,
        &mut out,
        opts,
        &sc,
        palette,
        &mut colours,
        gif_bytes,
        &mut between,
    )
}

/// [`recode_gif_streaming`] with its types fixed — a boxed reader, a `dyn`
/// writer and `dyn` hooks — instantiated here, in this crate: what the
/// firmware calls, so the decoders the recoder runs are the very
/// instantiations the player already links (`FrameSource` over a
/// `Box<dyn ReadSeek>`), not a second copy of each (~10 KB of image).
pub fn recode_gif_streaming_dyn(
    reader: Box<dyn crate::engine::ReadSeek + '_>,
    out: &mut dyn Write,
    opts: &RecodeOptions,
    full_ok: &mut dyn FnMut() -> bool,
    between: &mut dyn FnMut() -> io::Result<()>,
) -> io::Result<RecodeStats> {
    recode_gif_streaming(reader, out, opts, full_ok, between)
}

fn check_opts(opts: &RecodeOptions) -> io::Result<()> {
    if !(MIN_CAP..=MAX_CAP).contains(&opts.cap) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("cap {} is not in {MIN_CAP}..={MAX_CAP}", opts.cap),
        ));
    }
    Ok(())
}

/// Re-encode a GIF held in memory (desktop, browser): exactly
/// [`recode_gif_streaming`]'s bytes, through the same boxed entry point
/// the device calls. Past 256 colours it is [`TooManyColours`] here too:
/// such a GIF gets no copy and plays as it is (copies are exact or absent).
pub fn recode_gif(
    mut reader: impl Read,
    mut out: impl Write,
    opts: &RecodeOptions,
) -> io::Result<RecodeStats> {
    check_opts(opts)?;
    let mut gif = Vec::new();
    reader.read_to_end(&mut gif)?;
    // Through the boxed entry point, like the device: one instantiation of
    // the decoders, shared with the player (the browser's wasm budget).
    recode_gif_streaming_dyn(
        Box::new(io::Cursor::new(&gif[..])),
        &mut out,
        opts,
        &mut || true,
        &mut || Ok(()),
    )
}

/// The `gif` crate's flag for a colour table of `n` entries.
fn table_flag(n: usize) -> u8 {
    (n.max(2).next_power_of_two().trailing_zeros() - 1) as u8
}

/// The second pass: the GIF itself. Byte for byte what the `gif` crate's
/// encoder writes for the same frames (header, global palette, the hint,
/// NETSCAPE, a graphic control extension before every frame, 255-byte
/// sub-blocks, the trailer) — a unit test holds it to that.
#[allow(clippy::too_many_arguments, clippy::needless_range_loop)]
fn write_gif<R: Read + Seek>(
    reader: R,
    out: &mut dyn Write,
    opts: &RecodeOptions,
    sc: &Scan,
    palette: Vec<[u8; 3]>,
    indexer: &mut dyn Indexer,
    gif_bytes: u64,
    between: &mut dyn FnMut() -> io::Result<()>,
) -> io::Result<RecodeStats> {
    let (width, height) = (sc.width, sc.height);
    if sc.frames == 0 {
        return Err(invalid("GIF has no frames"));
    }
    if width > u16::MAX as usize || height > u16::MAX as usize {
        return Err(invalid("GIF screen too large"));
    }
    let opaque_deltas = opts.delta && palette.len() == 256;
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
    let mut stats = RecodeStats {
        width,
        height,
        frames: sc.frames,
        colours: palette.len(),
        opaque_deltas,
        gif_bytes,
        full_decoder: sc.full,
        ..Default::default()
    };

    let mut w = CountingWriter {
        inner: out,
        bytes: 0,
    };
    // Header, logical screen, global palette (padded to its table size).
    let flag = table_flag(table);
    let mut head = [0u8; 13];
    head[..6].copy_from_slice(b"GIF89a");
    head[6..8].copy_from_slice(&(width as u16).to_le_bytes());
    head[8..10].copy_from_slice(&(height as u16).to_le_bytes());
    head[10] = 0x80 | flag | flag << 4;
    w.write_all(&head)?;
    for c in &palette {
        w.write_all(c)?;
    }
    for _ in palette.len()..table {
        w.write_all(&[0, 0, 0])?;
    }
    if opts.hint {
        w.write_all(&[0x21, 0xFF, 11])?;
        w.write_all(HINT_APP_ID)?;
        w.write_all(&[3, 1, opts.cap, opts.delta as u8, 0])?;
    }
    if sc.repeat != gif::Repeat::Finite(0) {
        let n = match sc.repeat {
            gif::Repeat::Finite(n) => n,
            gif::Repeat::Infinite => 0,
        };
        let n = n.to_le_bytes();
        w.write_all(&[0x21, 0xFF, 11])?;
        w.write_all(b"NETSCAPE2.0")?;
        w.write_all(&[3, 1, n[0], n[1], 0])?;
    }

    let n = width * height;
    let mut idx = vec![0u8; n];
    let mut changed = vec![0u8; n.div_ceil(8)];
    let is_changed = |changed: &[u8], i: usize| changed[i / 8] & (1 << (i % 8)) != 0;
    let mut lzw = LzwEncoder::new(opts.cap);
    let mut canvas = Canvas::new();
    let mut src = SourceGif::open(reader, sc.full)?;
    let mut k = 0u32;
    loop {
        between()?;
        let Some(delay) = src.next(&mut canvas)? else {
            break;
        };
        if k == sc.frames {
            return Err(invalid("frame count differs between passes"));
        }
        let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
        changed.fill(0);
        for (i, &c) in src.screen(&canvas).as_chunks::<3>().0.iter().enumerate() {
            let q = indexer.index_of(c)?;
            // Frame 0 is sent whole and opaque: our players start from
            // black, but a browser shows what a first frame leaves
            // uncovered (or transparent) as the page behind it. After it,
            // equal indices are equal colours as shown.
            if k == 0 || idx[i] != q {
                idx[i] = q;
                changed[i / 8] |= 1 << (i % 8);
                let (x, y) = (i % width, i / width);
                x0 = x0.min(x);
                x1 = x1.max(x + 1);
                y0 = y0.min(y);
                y1 = y1.max(y + 1);
            }
        }
        // With all 256 palette slots taken there is no spare transparent
        // index, but a frame's changed pixels rarely use every colour: any
        // index they do not use can be this frame's transparent one.
        let frame_t = if opaque_deltas && k > 0 && x0 != usize::MAX {
            let mut used = [false; 256];
            for y in y0..y1 {
                for i in y * width + x0..y * width + x1 {
                    if is_changed(&changed, i) {
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
        let (left, top, fw, fh) = if !opts.delta {
            (0, 0, width, height)
        } else if x0 == usize::MAX {
            (0, 0, 1, 1)
        } else {
            (x0, y0, x1 - x0, y1 - y0)
        };
        stats.rect_px += (fw * fh) as u64;

        // Graphic control, image descriptor, the data.
        let dispose = if opts.delta { 1u8 } else { 0 };
        let d = delay.to_le_bytes();
        w.write_all(&[
            0x21,
            0xF9,
            4,
            dispose << 2 | frame_t.is_some() as u8,
            d[0],
            d[1],
            frame_t.unwrap_or(0),
            0,
        ])?;
        let mut desc = [0u8; 11];
        desc[0] = 0x2C;
        desc[1..3].copy_from_slice(&(left as u16).to_le_bytes());
        desc[3..5].copy_from_slice(&(top as u16).to_le_bytes());
        desc[5..7].copy_from_slice(&(fw as u16).to_le_bytes());
        desc[7..9].copy_from_slice(&(fh as u16).to_le_bytes());
        desc[10] = min_code_size;
        w.write_all(&desc)?;
        let mut blocks = SubBlocks::new(&mut w);
        lzw.begin(min_code_size, &mut blocks)?;
        if !opts.delta {
            for &q in &idx {
                lzw.push(q, &mut blocks)?;
            }
        } else if x0 == usize::MAX {
            // Nothing changed: one pixel that leaves the screen alone (or,
            // with no transparent index, repaints itself).
            lzw.push(frame_t.unwrap_or(idx[0]), &mut blocks)?;
        } else {
            let mut last = None;
            for y in y0..y1 {
                for i in y * width + x0..y * width + x1 {
                    let v = match frame_t {
                        Some(_)
                            if !is_changed(&changed, i)
                                && opts.keep_runs
                                && last == Some(idx[i]) =>
                        {
                            idx[i]
                        }
                        Some(t) if !is_changed(&changed, i) => t,
                        _ => idx[i],
                    };
                    lzw.push(v, &mut blocks)?;
                    last = Some(v);
                }
            }
        }
        lzw.end(&mut blocks)?;
        blocks.finish()?;
        k += 1;
    }
    if k != sc.frames {
        return Err(invalid("frame count differs between passes"));
    }
    w.write_all(&[0x3B])?;
    w.flush()?;
    stats.out_bytes = w.bytes;
    stats.lzw = lzw.stats();
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
    fn many_colours_get_no_copy_and_256_are_kept() {
        // 300 colours: no copy at all (never a lossy one), nothing written;
        // the GIF plays as it is, exactly, through gifslim at cap 12.
        let mut frames = Vec::new();
        for k in 0..3u16 {
            let pal: Vec<u8> = (0..100)
                .flat_map(|i: u16| [(i * 2) as u8, (k * 80) as u8, (255 - i) as u8])
                .collect();
            let px: Vec<u8> = (0..4096).map(|i| (i % 100) as u8).collect();
            frames.push((0, 0, 128, 32, px, DisposalMethod::Keep, 5, Some(pal), None));
        }
        let orig = build(128, 32, None, frames, false);
        for delta in [false, true] {
            let opts = RecodeOptions { delta, ..PREPARED };
            let mut out = Vec::new();
            let e = recode_gif(&orig[..], &mut out, &opts).unwrap_err();
            assert!(is_too_many_colours(&e));
            assert!(out.is_empty());
            assert!(plays_original(&e.to_string()));
        }
        assert!(!fits_palette(Cursor::new(&orig[..])).unwrap());
        let (want, _) = play_gif(&orig);
        assert_eq!(play_slim(&orig, 12).unwrap(), want);

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
        assert!(fits_palette(Cursor::new(&orig[..])).unwrap());
        let (re, st) = recode(&orig, 9, true);
        assert!(st.opaque_deltas && st.colours == 256);
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

    /// The experiment's in-memory recoder, on the `gif` crate's encoder:
    /// the reference the hand-written container is held to.
    mod reference {
        use super::super::*;
        use std::collections::HashMap;

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
        /// frames; past 256 colours [`TooManyColours`]. Exactly 256 colours with deltas
        /// keep all 256 and send changed rectangles without transparency.
        pub fn reference_recode(
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
            if too_many {
                return Err(io::Error::new(io::ErrorKind::InvalidData, TooManyColours));
            }
            let opaque_deltas = opts.delta && palette.len() == 256;
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
                    let Some(&q) = lookup.get(&c) else {
                        return Err(invalid("GIF decoded differently on the second pass"));
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
                                Some(_)
                                    if !changed[i] && opts.keep_runs && last == Some(idx[i]) =>
                                {
                                    idx[i]
                                }
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
    }

    /// A reader that hands out at most 7 bytes a call, and can seek.
    struct Trickle(Cursor<Vec<u8>>);
    impl Read for Trickle {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let n = buf.len().min(7);
            self.0.read(&mut buf[..n])
        }
    }
    impl Seek for Trickle {
        fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
            self.0.seek(pos)
        }
    }

    fn streaming(
        gif: &[u8],
        opts: &RecodeOptions,
        full_ok: bool,
    ) -> io::Result<(Vec<u8>, RecodeStats)> {
        let mut out = Vec::new();
        let st = recode_gif_streaming(
            Trickle(Cursor::new(gif.to_vec())),
            &mut out,
            opts,
            || full_ok,
            || Ok(()),
        )?;
        Ok((out, st))
    }

    #[test]
    fn the_container_is_the_gif_crates_byte_for_byte() {
        let mut files: Vec<Vec<u8>> = (1..10).map(|s| animation(128, 32, s)).collect();
        files.push(animation(135, 32, 3));
        files.push(animation(10, 5, 4));
        for g in &files {
            for cap in [9u8, 10, 12] {
                for (delta, keep_runs, hint) in [
                    (true, true, true),
                    (true, false, true),
                    (false, false, false),
                ] {
                    let opts = RecodeOptions {
                        cap,
                        delta,
                        hint,
                        keep_runs,
                    };
                    let mut want = Vec::new();
                    let ws = reference::reference_recode(&g[..], &mut want, &opts).unwrap();
                    let (got, gs) = streaming(g, &opts, true).unwrap();
                    assert!(got == want, "cap {cap} delta {delta} keep_runs {keep_runs}");
                    assert_eq!(gs.out_bytes, ws.out_bytes);
                    assert_eq!(
                        (gs.frames, gs.colours, gs.rect_px, gs.lzw),
                        (ws.frames, ws.colours, ws.rect_px, ws.lzw)
                    );
                }
            }
        }
    }

    #[test]
    fn in_memory_and_streaming_write_the_same_bytes() {
        let mut rng = Rng(11);
        for seed in 1..40 {
            let (w, h) = match seed % 5 {
                0 => (135, 32),
                1 => (16, 8),
                _ => (128, 32),
            };
            let g = animation(w, h, seed * 7 + rng.below(1000) as u64);
            let mut mem = Vec::new();
            let ms = recode_gif(&g[..], &mut mem, &PREPARED).unwrap();
            let (st, ss) = streaming(&g, &PREPARED, false).unwrap();
            assert!(mem == st, "seed {seed}");
            assert_eq!(ms, ss);
            assert_eq!(ss.out_bytes, st.len() as u64);
            assert_eq!(ss.gif_bytes, g.len() as u64);
        }
    }

    #[test]
    fn a_257th_colour_is_too_many_for_the_stream_and_writes_nothing() {
        let mut frames = Vec::new();
        for k in 0..3u16 {
            let pal: Vec<u8> = (0..100)
                .flat_map(|i: u16| [(i * 2) as u8, (k * 80) as u8, (255 - i) as u8])
                .collect();
            let px: Vec<u8> = (0..4096).map(|i| (i % 100) as u8).collect();
            frames.push((0, 0, 128, 32, px, DisposalMethod::Keep, 5, Some(pal), None));
        }
        let g = build(128, 32, None, frames, false);
        let mut out = Vec::new();
        let e = recode_gif_streaming(Cursor::new(&g[..]), &mut out, &PREPARED, || true, || Ok(()))
            .unwrap_err();
        assert!(is_too_many_colours(&e));
        assert!(out.is_empty());
        // The in-memory path refuses it the same way: no lossy copy.
        let mut out = Vec::new();
        assert!(is_too_many_colours(
            &recode_gif(&g[..], &mut out, &PREPARED).unwrap_err()
        ));
        assert!(out.is_empty());
        assert!(!fits_palette(Cursor::new(&g[..])).unwrap());
    }

    #[test]
    fn the_original_reason_and_the_quantised_copy_test() {
        // What .failed lines say: this build's reason, the bare one earlier
        // firmware wrote, and nothing else.
        assert!(plays_original(ORIGINAL_REASON));
        assert!(plays_original("more than 256 colours"));
        assert!(plays_original(" More than 256 colors"));
        assert!(!plays_original("invalid LZW"));
        assert!(!plays_original("132x32 is too large to prepare here"));
        assert!(!plays_original(""));
        // A copy with a small table cannot be a quantised one; a 256-entry
        // table might be (and needs the GIF's colour count).
        let small = recode(&animation(128, 32, 3), 9, true).0;
        assert!(!copy_may_be_quantised(&small));
        let pal = palette(256);
        let px: Vec<u8> = (0..4096).map(|i| (i % 256) as u8).collect();
        let full = build(
            128,
            32,
            Some(pal),
            vec![(0, 0, 128, 32, px, DisposalMethod::Keep, 5, None, None)],
            false,
        );
        let big = recode(&full, 9, true).0;
        assert!(copy_may_be_quantised(&big));
        assert!(!copy_may_be_quantised(b"GIF8"));
        assert!(!copy_may_be_quantised(&[0u8; 32]));
    }

    #[test]
    fn what_gifslim_cannot_decode_takes_the_gif_crate_when_allowed() {
        let interlaced = build(
            8,
            8,
            Some(palette(4)),
            vec![
                (
                    0,
                    0,
                    8,
                    8,
                    (0..64).map(|i| (i % 3) as u8).collect(),
                    DisposalMethod::Keep,
                    5,
                    None,
                    None,
                ),
                (
                    0,
                    0,
                    8,
                    8,
                    (0..64).map(|i| (i % 4) as u8).collect(),
                    DisposalMethod::Keep,
                    5,
                    None,
                    None,
                ),
            ],
            true,
        );
        let e = streaming(&interlaced, &PREPARED, false).unwrap_err();
        assert!(is_unsupported(&e));
        let (a, sa) = streaming(&interlaced, &PREPARED, true).unwrap();
        assert!(sa.full_decoder);
        let mut b = Vec::new();
        recode_gif(&interlaced[..], &mut b, &PREPARED).unwrap();
        assert_eq!(a, b);
        assert_eq!(play_slim(&a, 9).unwrap(), play_gif(&interlaced).0);
    }

    #[test]
    fn the_loop_count_is_read_like_the_gif_crate() {
        for repeat in [Repeat::Infinite, Repeat::Finite(3), Repeat::Finite(0)] {
            let mut out = Vec::new();
            {
                let mut enc = Encoder::new(&mut out, 4, 4, &palette(4)).unwrap();
                enc.set_repeat(repeat).unwrap();
                let f = Frame {
                    width: 4,
                    height: 4,
                    buffer: Cow::Owned(vec![1u8; 16]),
                    ..Default::default()
                };
                enc.write_frame(&f).unwrap();
            }
            let mut s = GifSlim::new(&out[..]).unwrap();
            s.next_frame(&mut Canvas::new()).unwrap();
            let d = gif::DecodeOptions::new().read_info(&out[..]).unwrap();
            assert_eq!(s.repeat(), d.repeat());
            assert_eq!(s.repeat(), repeat);
            let (re, _) = recode(&out, 9, true);
            assert_eq!(
                gif::DecodeOptions::new()
                    .read_info(&re[..])
                    .unwrap()
                    .repeat(),
                repeat
            );
        }
    }

    #[test]
    fn the_prepared_format_is_hinted_at_cap_9() {
        let (re, _) = streaming(&animation(128, 32, 5), &PREPARED, false).unwrap();
        let s = GifSlim::new(&re[..]).unwrap();
        assert_eq!(
            s.hint(),
            Some(Hint {
                cap: 9,
                delta: true
            })
        );
        assert_eq!(s.cap(), 9);
        // ~16 KB with the canvas: tables for 9-bit codes only.
        assert_eq!(
            s.heap_bytes(),
            READ_BUF + 512 * 2 + 512 + 128 + s.global.len()
        );
    }

    #[test]
    fn probe_reads_what_new_would_open_with_and_rewinds() {
        let plain = animation(128, 32, 3);
        let (hinted, _) = streaming(&plain, &PREPARED, false).unwrap();
        let (wide, _) = streaming(&animation(132, 32, 2), &PREPARED, false).unwrap();
        for bytes in [&plain, &hinted, &wide] {
            let mut c = Cursor::new(&bytes[..]);
            let p = probe(&mut c).unwrap();
            assert_eq!(c.position(), 0);
            let s = GifSlim::new(&bytes[..]).unwrap();
            assert_eq!((p.width, p.height), s.logical_size());
            assert_eq!(p.hint, s.hint());
        }
        assert_eq!(
            probe(&mut Cursor::new(&hinted[..]))
                .unwrap()
                .hint
                .map(|h| h.cap),
            Some(9)
        );
        assert_eq!(probe(&mut Cursor::new(&plain[..])).unwrap().hint, None);
        // A comment and a foreign application extension before the hint are
        // stepped over, as `read_header` steps over them.
        let at = 13 + 3 * (2 << (hinted[10] & 7));
        let mut extra = hinted[..at].to_vec();
        extra.extend_from_slice(&[0x21, 0xFE, 2, b'h', b'i', 0]);
        extra.extend_from_slice(&[0x21, 0xFF, 11]);
        extra.extend_from_slice(b"XMP DataXMP");
        extra.extend_from_slice(&[1, 0, 0]);
        extra.extend_from_slice(&hinted[at..]);
        let p = probe(&mut Cursor::new(&extra[..])).unwrap();
        assert_eq!(p.hint, GifSlim::new(&extra[..]).unwrap().hint());
        assert!(p.hint.is_some());
        // A bad header is the decoder's error.
        assert!(probe(&mut Cursor::new(&b"GIF8zz\0\0"[..])).is_err());
    }

    #[test]
    fn a_stop_from_between_abandons_the_encode() {
        let g = animation(128, 32, 2);
        let mut n = 0;
        let e = recode_gif_streaming(
            Cursor::new(&g[..]),
            Vec::new(),
            &PREPARED,
            || true,
            || {
                n += 1;
                if n > 5 {
                    Err(io::Error::other("stopped"))
                } else {
                    Ok(())
                }
            },
        )
        .unwrap_err();
        assert_eq!(e.to_string(), "stopped");
    }
}

//! ODM v1: pre-rendered, palette-indexed, delta-run-length frames.
//!
//! A GIF costs ~54 KB of heap to play on the ESP32, ~37 KB of it the LZW
//! decoder and the `gif` crate's own read buffer. An `.odm` file carries the
//! same animation already composited — disposal and transparency applied by
//! the encoder — as runs over the previous frame, so playing one needs only
//! the palette (at most 768 B), a 2 KB read buffer and the canvas the caller
//! already owns. The byte-level spec is `docs/odm-format.md`; the Python
//! prototype `tools/odm-proto.py` is the compatibility reference, and files it
//! wrote decode here unchanged.
//!
//! Layout, little-endian:
//!
//! ```text
//! header   32 B  "ODM1", version u8 = 1, width u16, height u16,
//!                frame_count u32, palette_len u16 (1..=256), 17 reserved
//! palette  palette_len x RGB888
//! frames   frame_count x { delay_ms u16, data_len u32, data_len bytes of ops }
//! ```
//!
//! Ops cover the frame's `width * height` pixels in raster order. An op byte
//! is `tag << 6 | len6`; `len6 == 63` means the real length is the `u16` that
//! follows. `tag` 0 = KEEP (pixels unchanged from the previous frame), 1 =
//! FILL (one palette index follows), 2 = LIT (`len` indices follow). Frame
//! 0's "previous frame" is all black RGB — not palette index 0, which need
//! not be black. `delay_ms` is final: the encoder already applied the GIF
//! player's 0 -> 100 ms and 20 ms-floor rules, so the decoder applies none.
//!
//! [`OdmSource`] is the decoder (the firmware's path), [`encode_gif`] the
//! encoder (desktop and wasm: it can quantise), and [`encode_gif_streaming`]
//! the bounded one-pass encoder the ESP32 prepares its own cache with (no
//! quantiser: a file past 256 colours is left for the other one).

use crate::canvas::{Canvas, DirtyMap, HEIGHT, WIDTH};
use crate::player::{
    blit, decode_err, frame_delay, invalid, GifSource, Rect, RowOrder, Screen, MAX_LOGICAL_PIXELS,
    MAX_PALETTE,
};
use std::collections::hash_map::{Entry as MapEntry, HashMap};
use std::io::{self, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::time::Duration;

/// The first four bytes of every ODM file.
pub const MAGIC: [u8; 4] = *b"ODM1";
/// The only version there is.
pub const VERSION: u8 = 1;
/// Fixed header size; the 17 bytes after `palette_len` are reserved (zero).
pub const HEADER_LEN: usize = 32;
/// Largest palette an ODM file may carry.
pub const MAX_PALETTE_LEN: usize = 256;

/// Safety floor on a frame delay read from a file (matches the GIF player's floor).
const MIN_DELAY_MS: u64 = 20;

const TAG_KEEP: u8 = 0;
const TAG_FILL: u8 = 1;
const TAG_LIT: u8 = 2;
/// `len6` value meaning "a `u16` length follows".
const LEN_ESCAPE: u8 = 63;
/// Longest run one op can describe.
const MAX_RUN: usize = u16::MAX as usize;
/// The greedy encoder's threshold: a KEEP or FILL run shorter than this is
/// folded into a LIT instead (what `tools/odm-proto.py` does).
const MIN_RUN: u32 = 4;

/// The decoder's read buffer. Unlike `gif`, nothing underneath buffers, and
/// the firmware's `SdSource` hands out a bare `File`, so this is the only one.
const READ_BUF: usize = 2048;
/// LIT runs are read through a stack buffer of this many indices at a time,
/// never a whole-frame one.
const LIT_CHUNK: usize = 64;

/// `UnexpectedEof` from a `read_exact` means the file stopped mid-structure:
/// report it as malformed data, like every other structural error here.
fn read_exact<R: Read>(r: &mut R, buf: &mut [u8]) -> io::Result<()> {
    r.read_exact(buf).map_err(|e| {
        if e.kind() == io::ErrorKind::UnexpectedEof {
            invalid("ODM stream is truncated")
        } else {
            e
        }
    })
}

fn read_u8<R: Read>(r: &mut R) -> io::Result<u8> {
    let mut b = [0u8; 1];
    read_exact(r, &mut b)?;
    Ok(b[0])
}

fn read_u16<R: Read>(r: &mut R) -> io::Result<u16> {
    let mut b = [0u8; 2];
    read_exact(r, &mut b)?;
    Ok(u16::from_le_bytes(b))
}

fn read_u32<R: Read>(r: &mut R) -> io::Result<u32> {
    let mut b = [0u8; 4];
    read_exact(r, &mut b)?;
    Ok(u32::from_le_bytes(b))
}

/// The fixed 32-byte header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    pub width: u16,
    pub height: u16,
    pub frame_count: u32,
    pub palette_len: u16,
}

impl Header {
    /// Parse and validate: magic, version, non-zero and plausible dimensions,
    /// and a palette of 1..=256 entries. Reserved bytes are not checked, so a
    /// later writer may use them without breaking this reader.
    pub fn parse(b: &[u8; HEADER_LEN]) -> io::Result<Header> {
        if b[0..4] != MAGIC {
            return Err(invalid("not an ODM file (bad magic)"));
        }
        if b[4] != VERSION {
            return Err(invalid(format!("unsupported ODM version {}", b[4])));
        }
        let h = Header {
            width: u16::from_le_bytes([b[5], b[6]]),
            height: u16::from_le_bytes([b[7], b[8]]),
            frame_count: u32::from_le_bytes([b[9], b[10], b[11], b[12]]),
            palette_len: u16::from_le_bytes([b[13], b[14]]),
        };
        let (w, hh) = (h.width as usize, h.height as usize);
        if w == 0 || hh == 0 {
            return Err(invalid("ODM screen has a zero dimension"));
        }
        if w * hh > MAX_LOGICAL_PIXELS {
            return Err(invalid(format!("ODM screen {w}x{hh} is implausibly large")));
        }
        if h.palette_len == 0 || h.palette_len as usize > MAX_PALETTE_LEN {
            return Err(invalid(format!(
                "ODM palette length {} is not in 1..=256",
                h.palette_len
            )));
        }
        Ok(h)
    }

    pub fn to_bytes(self) -> [u8; HEADER_LEN] {
        let mut b = [0u8; HEADER_LEN];
        b[0..4].copy_from_slice(&MAGIC);
        b[4] = VERSION;
        b[5..7].copy_from_slice(&self.width.to_le_bytes());
        b[7..9].copy_from_slice(&self.height.to_le_bytes());
        b[9..13].copy_from_slice(&self.frame_count.to_le_bytes());
        b[13..15].copy_from_slice(&self.palette_len.to_le_bytes());
        b
    }
}

// ---------------------------------------------------------------------------
// Decoder
// ---------------------------------------------------------------------------

/// An ODM file being played. Same contract as
/// [`GifSource`](crate::player::GifSource): one per playback, no rewind,
/// `Ok(None)` at the end with the canvas untouched.
///
/// Heap: the 2 KB `BufReader`, the palette (`3 * palette_len` bytes) and, only
/// for a file that is not exactly 128x32, an owned RGB screen that is
/// centre-cropped onto the canvas. At the panel size the caller's [`Canvas`]
/// *is* the screen, exactly as in the GIF path's `Direct` mode, and the caller
/// must not write it between frames: KEEP runs read it back. Nothing is
/// allocated per frame.
pub struct OdmSource<R: Read> {
    reader: BufReader<R>,
    width: usize,
    height: usize,
    frame_count: u32,
    /// Packed RGB, `3 * palette_len` bytes.
    palette: Vec<u8>,
    screen: Screen,
    frames_decoded: u64,
}

impl<R: Read> OdmSource<R> {
    /// Read and validate the header and the palette.
    pub fn new(reader: R) -> io::Result<Self> {
        let mut reader = BufReader::with_capacity(READ_BUF, reader);
        let mut hb = [0u8; HEADER_LEN];
        read_exact(&mut reader, &mut hb)?;
        let header = Header::parse(&hb)?;
        let mut palette = vec![0u8; header.palette_len as usize * 3];
        read_exact(&mut reader, &mut palette)?;
        let (width, height) = (header.width as usize, header.height as usize);
        Ok(OdmSource {
            reader,
            width,
            height,
            frame_count: header.frame_count,
            palette,
            screen: if width == WIDTH && height == HEIGHT {
                Screen::Direct { cleared: false }
            } else {
                // Zeroed: frame 0's "previous frame" is all black.
                Screen::Owned(vec![0u8; width * height * 3])
            },
            frames_decoded: 0,
        })
    }

    /// The encoded screen size, which may not be 128x32.
    pub fn logical_size(&self) -> (usize, usize) {
        (self.width, self.height)
    }

    /// Frames handed out so far.
    pub fn frames_decoded(&self) -> u64 {
        self.frames_decoded
    }

    /// The reader, with the decoder's buffer (and whatever it held) dropped.
    pub fn into_inner(self) -> R {
        self.reader.into_inner()
    }

    /// Frames in the file, from the header.
    pub fn frame_count(&self) -> u32 {
        self.frame_count
    }

    /// Palette entries, from the header.
    pub fn palette_len(&self) -> usize {
        self.palette.len() / 3
    }

    /// Decode the next frame onto `canvas` and return how long to show it.
    /// A truncated or malformed frame is `InvalidData`; it never panics.
    pub fn next_frame(&mut self, canvas: &mut Canvas) -> io::Result<Option<Duration>> {
        if self.frames_decoded >= u64::from(self.frame_count) {
            return Ok(None);
        }
        let OdmSource {
            reader,
            width,
            height,
            palette,
            screen,
            frames_decoded,
            ..
        } = self;
        let (width, height) = (*width, *height);

        // The frame header is read before the screen is touched, so a file
        // that ends here leaves the canvas as it was.
        let delay_ms = read_u16(reader)?;
        let data_len = read_u32(reader)? as u64;

        {
            // `Direct`: FILL and LIT runs are marked in the canvas's dirty
            // map as they are painted, KEEP runs are not — that is the whole
            // point of the map. `Owned` is re-blitted (all marked) below.
            let (buf, dirty): (&mut [u8], Option<&mut DirtyMap>) = match screen {
                Screen::Direct { cleared } => {
                    if !*cleared {
                        // Frame 0 is a delta against all-black. Marks all.
                        canvas.clear();
                        *cleared = true;
                    }
                    let (b, d) = canvas.bytes_and_dirty_mut();
                    (b, Some(d))
                }
                Screen::Owned(v) => (v.as_mut_slice(), None),
            };
            let consumed = apply_ops(reader, palette, buf, dirty, width * height, data_len)?;
            if consumed != data_len {
                return Err(invalid(format!(
                    "ODM frame {} declares {data_len} bytes of ops but uses {consumed}",
                    *frames_decoded
                )));
            }
        }

        *frames_decoded += 1;
        if let Screen::Owned(v) = screen {
            blit(v, width, height, canvas);
        }
        // The encoder writes final delays, but a file from another writer may
        // carry 0 ms; floor it so a bad file can never spin the player.
        Ok(Some(Duration::from_millis(
            u64::from(delay_ms).max(MIN_DELAY_MS),
        )))
    }
}

/// Writes pixels and, when there is a dirty map, marks the ones whose value
/// changed, a run at a time. FILL and LIT are what the encoder emits for
/// changed pixels, but a short unchanged stretch is folded into a LIT (see
/// `MIN_RUN`) and a stray's crop can repeat colours, so the value decides.
struct Runs<'a> {
    dirty: Option<&'a mut DirtyMap>,
    start: Option<usize>,
    end: usize,
}

impl<'a> Runs<'a> {
    fn new(dirty: Option<&'a mut DirtyMap>) -> Self {
        Runs {
            dirty,
            start: None,
            end: 0,
        }
    }

    #[inline]
    fn paint(&mut self, i: usize, px: &mut [u8; 3], rgb: &[u8]) {
        if px[..] == *rgb {
            return;
        }
        px.copy_from_slice(rgb);
        if self.dirty.is_some() {
            if self.start.is_some() && self.end != i {
                self.finish();
            }
            self.start.get_or_insert(i);
            self.end = i + 1;
        }
    }

    fn finish(&mut self) {
        if let (Some(start), Some(d)) = (self.start.take(), self.dirty.as_deref_mut()) {
            d.mark_run(start, self.end);
        }
    }
}

/// Apply one frame's ops to `screen` (packed RGB, `pixels * 3` bytes).
/// Returns the op bytes consumed; never reads past `limit` of them. Pixels a
/// FILL or LIT run changes are marked in `dirty` when there is one (the
/// screen is then the canvas, so run indices are canvas indices); KEEP runs
/// are never marked.
fn apply_ops<R: Read>(
    reader: &mut R,
    palette: &[u8],
    screen: &mut [u8],
    mut dirty: Option<&mut DirtyMap>,
    pixels: usize,
    limit: u64,
) -> io::Result<u64> {
    let colour = |idx: u8| -> io::Result<&[u8]> {
        let o = idx as usize * 3;
        palette
            .get(o..o + 3)
            .ok_or_else(|| invalid(format!("ODM palette index {idx} out of range")))
    };
    let mut consumed: u64 = 0;
    let mut take = |n: u64| -> io::Result<()> {
        consumed += n;
        if consumed > limit {
            Err(invalid("ODM ops overrun the frame's data_len"))
        } else {
            Ok(())
        }
    };

    let mut p = 0usize;
    let mut lit = [0u8; LIT_CHUNK];
    while p < pixels {
        take(1)?;
        let op = read_u8(reader)?;
        let tag = op >> 6;
        let mut len = (op & 0x3F) as usize;
        if len == LEN_ESCAPE as usize {
            take(2)?;
            len = read_u16(reader)? as usize;
        }
        if len > pixels - p {
            return Err(invalid("ODM run overruns the frame"));
        }
        let span = &mut screen[p * 3..(p + len) * 3];
        // Pixels that really change, as runs of raster indices; only
        // tracked when there is a map to put them in.
        let mut runs = Runs::new(dirty.as_deref_mut());
        match tag {
            TAG_KEEP => {}
            TAG_FILL => {
                take(1)?;
                let rgb = colour(read_u8(reader)?)?;
                for (i, px) in span.as_chunks_mut::<3>().0.iter_mut().enumerate() {
                    runs.paint(p + i, px, rgb);
                }
            }
            TAG_LIT => {
                take(len as u64)?;
                let mut i = p;
                for dst in span.chunks_mut(LIT_CHUNK * 3) {
                    let n = dst.len() / 3;
                    read_exact(reader, &mut lit[..n])?;
                    for (px, &idx) in dst.as_chunks_mut::<3>().0.iter_mut().zip(&lit[..n]) {
                        runs.paint(i, px, colour(idx)?);
                        i += 1;
                    }
                }
            }
            _ => return Err(invalid(format!("ODM op has reserved tag {tag}"))),
        }
        runs.finish();
        p += len;
    }
    Ok(consumed)
}

// ---------------------------------------------------------------------------
// Encoder
// ---------------------------------------------------------------------------

/// What [`encode_gif`] produced.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EncodeStats {
    pub width: usize,
    pub height: usize,
    pub frames: u32,
    pub palette_len: usize,
    /// More than 256 distinct colours, so the palette was built with
    /// `NeuQuant` and the output is lossy for this file.
    pub quantised: bool,
    pub gif_bytes: u64,
    pub odm_bytes: u64,
    pub keep_px: u64,
    pub fill_px: u64,
    pub lit_px: u64,
}

impl EncodeStats {
    pub fn total_px(&self) -> u64 {
        self.keep_px + self.fill_px + self.lit_px
    }
}

/// `NeuQuant`'s sampling factor: 1 is best and slowest, 30 fastest. 10 is
/// what the `gif` crate itself uses.
const NEUQUANT_SAMPLEFAC: i32 = 10;

/// Counts what passes through, so the stats know the output size without the
/// writer having to be seekable.
struct Counting<W> {
    inner: W,
    bytes: u64,
}

impl<W: Write> Write for Counting<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = self.inner.write(buf)?;
        self.bytes += n as u64;
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// Decode `gif` through [`GifSource`] and hand each frame's logical screen
/// (packed RGB, before any centre-crop) and delay to `visit`.
fn for_each_frame(
    gif: &[u8],
    mut visit: impl FnMut(&[u8], Duration) -> io::Result<()>,
) -> io::Result<(usize, usize)> {
    let mut src = GifSource::new(gif)?;
    let mut canvas = Canvas::new();
    while let Some(delay) = src.next_frame(&mut canvas)? {
        visit(src.screen(&canvas), delay)?;
    }
    Ok(src.logical_size())
}

/// Convert a GIF to ODM v1.
///
/// Frames come from our own [`GifSource`], so disposal, transparency and the
/// delay rules are exactly what playback does, and the decoder's output is
/// identical to the GIF path's canvas frame for frame. The palette is the
/// animation's distinct colours in first-seen order when there are at most
/// 256; otherwise `NeuQuant` builds a 256-entry one over every pixel of every
/// frame and [`EncodeStats::quantised`] says so. Ops follow the prototype's
/// greedy rule (see [`encode_frame`]), so the output matches
/// `tools/odm-proto.py`'s op for op wherever the frames and palette agree.
///
/// The GIF is read into memory and decoded two or three times rather than
/// keeping every frame: memory is the file plus a few `w*h` buffers.
pub fn encode_gif(mut reader: impl Read, out: impl Write) -> io::Result<EncodeStats> {
    let mut gif = Vec::new();
    reader.read_to_end(&mut gif)?;

    // Pass 1: frame count and the colour set, in first-seen order, until
    // there are too many to index exactly.
    let mut frames: u32 = 0;
    let mut lookup: HashMap<[u8; 3], u8> = HashMap::new();
    let mut palette: Vec<[u8; 3]> = Vec::new();
    let mut too_many = false;
    let (width, height) = for_each_frame(&gif, |screen, _| {
        frames = frames
            .checked_add(1)
            .ok_or_else(|| invalid("more frames than ODM can count"))?;
        if !too_many {
            for &c in screen.as_chunks::<3>().0 {
                if let MapEntry::Vacant(slot) = lookup.entry(c) {
                    if palette.len() == MAX_PALETTE_LEN {
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
    if width > u16::MAX as usize || height > u16::MAX as usize {
        return Err(invalid("GIF screen too large for ODM"));
    }

    // Pass 1b, only past 256 colours: quantise over the whole animation.
    let quant = if too_many {
        let mut rgba = Vec::with_capacity(frames as usize * width * height * 4);
        for_each_frame(&gif, |screen, _| {
            for px in screen.as_chunks::<3>().0 {
                rgba.extend_from_slice(&[px[0], px[1], px[2], 255]);
            }
            Ok(())
        })?;
        let nq = color_quant::NeuQuant::new(NEUQUANT_SAMPLEFAC, MAX_PALETTE_LEN, &rgba);
        drop(rgba);
        palette = nq.color_map_rgb().as_chunks::<3>().0.to_vec();
        lookup.clear();
        Some(nq)
    } else {
        None
    };

    let mut out = Counting {
        inner: out,
        bytes: 0,
    };
    let header = Header {
        width: width as u16,
        height: height as u16,
        frame_count: frames,
        palette_len: palette.len() as u16,
    };
    out.write_all(&header.to_bytes())?;
    for c in &palette {
        out.write_all(c)?;
    }

    // Pass 2: index each frame, diff it against the previous one (in the
    // palette's RGB, so frame 0 is diffed against black) and emit ops.
    let n = width * height;
    let mut enc = FrameEncoder::new(n);
    let mut idx = vec![0u8; n];
    let mut unchanged = vec![false; n];
    let mut prev = vec![0u8; n * 3];
    let mut ops = Vec::new();
    let mut stats = EncodeStats {
        width,
        height,
        frames,
        palette_len: palette.len(),
        quantised: quant.is_some(),
        gif_bytes: gif.len() as u64,
        ..Default::default()
    };
    for_each_frame(&gif, |screen, delay| {
        for (i, &c) in screen.as_chunks::<3>().0.iter().enumerate() {
            let k = match (&quant, lookup.get(&c)) {
                (_, Some(&k)) => k,
                (Some(nq), None) => {
                    let k = nq.index_of(&[c[0], c[1], c[2], 255]) as u8;
                    lookup.insert(c, k);
                    k
                }
                (None, None) => return Err(invalid("GIF decoded differently on the second pass")),
            };
            idx[i] = k;
            let q = palette[k as usize];
            let d = &mut prev[i * 3..i * 3 + 3];
            unchanged[i] = d == q;
            d.copy_from_slice(&q);
        }
        ops.clear();
        let (k, f, l) = enc.encode(&idx, &unchanged, &mut ops);
        stats.keep_px += k;
        stats.fill_px += f;
        stats.lit_px += l;
        let delay_ms = delay.as_millis().min(u16::MAX as u128) as u16;
        out.write_all(&delay_ms.to_le_bytes())?;
        out.write_all(&(ops.len() as u32).to_le_bytes())?;
        out.write_all(&ops)
    })?;
    out.flush()?;
    stats.odm_bytes = out.bytes;
    Ok(stats)
}

fn push_op(out: &mut Vec<u8>, tag: u8, len: usize) {
    debug_assert!(len > 0 && len <= MAX_RUN);
    if len < LEN_ESCAPE as usize {
        out.push(tag << 6 | len as u8);
    } else {
        out.push(tag << 6 | LEN_ESCAPE);
        out.extend_from_slice(&(len as u16).to_le_bytes());
    }
}

/// The greedy op encoder, with its two run-length tables reused across
/// frames.
struct FrameEncoder {
    /// `fill[i]`: how far a run of equal indices starting at `i` reaches.
    fill: Vec<u32>,
    /// `keep[i]`: how far a run of unchanged pixels starting at `i` reaches,
    /// 0 where pixel `i` changed.
    keep: Vec<u32>,
}

impl FrameEncoder {
    fn new(n: usize) -> Self {
        FrameEncoder {
            fill: vec![0; n],
            keep: vec![0; n],
        }
    }

    /// Encode one frame; returns the KEEP/FILL/LIT pixel counts.
    ///
    /// At each position take the longest KEEP or FILL run available (KEEP on
    /// a tie) if it is at least 4 long; otherwise emit literals up to the
    /// next position where such a run opens, or the end of the frame. That
    /// is `encode_frame` in `tools/odm-proto.py`, and it is deterministic, so
    /// two encoders given the same frames and palette write the same bytes.
    /// Runs longer than a `u16` are split, which the prototype never needed.
    fn encode(&mut self, idx: &[u8], unchanged: &[bool], out: &mut Vec<u8>) -> (u64, u64, u64) {
        let n = idx.len();
        let (fill, keep) = (&mut self.fill, &mut self.keep);
        for i in (0..n).rev() {
            let last = i + 1 == n;
            fill[i] = if !last && idx[i] == idx[i + 1] {
                fill[i + 1] + 1
            } else {
                1
            };
            keep[i] = if !unchanged[i] {
                0
            } else if !last && unchanged[i + 1] {
                keep[i + 1] + 1
            } else {
                1
            };
        }
        let good = |i: usize| fill[i] >= MIN_RUN || keep[i] >= MIN_RUN;

        let (mut kp, mut fp, mut lp) = (0u64, 0u64, 0u64);
        let mut p = 0;
        while p < n {
            if good(p) {
                let (k, f) = (keep[p] as usize, fill[p] as usize);
                if k >= f {
                    let len = k.min(MAX_RUN);
                    push_op(out, TAG_KEEP, len);
                    kp += len as u64;
                    p += len;
                } else {
                    let len = f.min(MAX_RUN);
                    push_op(out, TAG_FILL, len);
                    out.push(idx[p]);
                    fp += len as u64;
                    p += len;
                }
            } else {
                let mut q = p + 1;
                while q < n && !good(q) && q - p < MAX_RUN {
                    q += 1;
                }
                push_op(out, TAG_LIT, q - p);
                out.extend_from_slice(&idx[p..q]);
                lp += (q - p) as u64;
                p = q;
            }
        }
        (kp, fp, lp)
    }
}

// ---------------------------------------------------------------------------
// The streaming encoder (the device's)
// ---------------------------------------------------------------------------

/// [`encode_gif_streaming`] met a 257th distinct colour: exact indexing is
/// impossible and the quantising encoder ([`encode_gif`], i.e. the browser or
/// `opendmd-pack`) has to make this file. Carried inside an
/// `io::Error` of kind `InvalidData`; test for it with
/// [`is_too_many_colours`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TooManyColours;

impl std::fmt::Display for TooManyColours {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("more than 256 colours (needs the quantising encoder)")
    }
}

impl std::error::Error for TooManyColours {}

/// Is `e` the streaming encoder's [`TooManyColours`]?
pub fn is_too_many_colours(e: &io::Error) -> bool {
    e.get_ref()
        .is_some_and(|inner| inner.is::<TooManyColours>())
}

/// The streaming encoder's output buffer, and its chunk for the palette shift.
const STREAM_BUF: usize = 2048;
/// Bytes reserved after the header for a palette of unknown length.
const PALETTE_AREA: usize = MAX_PALETTE_LEN * 3;
/// Slots in the streaming encoder's colour table: a power of two, at least
/// twice [`MAX_PALETTE_LEN`], so open addressing never gets crowded.
const TABLE_SLOTS: usize = 512;
/// An empty slot. No RGB888 colour packs to it (they fit in 24 bits).
const EMPTY: u32 = u32::MAX;

/// The slot the screen starts in: black (until [`Colours::recycle_black`]).
const BLACK_SLOT: u8 = 0;
/// A key position whose colour gave its slot up; lookups probe past it.
const GONE: u32 = u32::MAX - 1;
/// "No palette index yet" in [`Colours::pal_of`].
const UNSEEN: u16 = u16::MAX;

/// The streaming encoder's colours, in two numberings:
///
/// - a **slot** per distinct colour, in the order colours are first
///   *painted* (black, the empty screen, is slot 0) — what the screen holds,
///   one byte a pixel, by open addressing over 512 `u32` keys;
/// - a **palette index** per slot, in the order colours are first *seen*
///   scanning each composited frame in raster order — [`encode_gif`]'s
///   palette, and what the ops carry.
///
/// Painting order is not raster order (interlaced rows, disposal to black),
/// hence both. ~3.8 KB in all.
///
/// Black holds slot 0 from the start, for the empty screen, though it may
/// never be *seen*: a file of 256 other colours whose frames always cover it
/// would need a 257th slot. So when the table is full and black has never
/// been seen and is on no pixel, its slot goes to the new colour
/// ([`recycle_black`](Self::recycle_black)); disposal to black after that is
/// [`TooManyColours`]. Over the whole card this matches [`encode_gif`]'s
/// indexing exactly (the encodeprobe sweep).
struct Colours {
    keys: Vec<u32>,
    slot_of: Vec<u8>,
    slot_rgb: Vec<[u8; 3]>,
    pal_of: Vec<u16>,
    /// Packed RGB, palette order.
    palette: Vec<u8>,
    /// The last colour painted and its slot (runs of one colour are common).
    last: (u32, u8),
}

impl Colours {
    fn new() -> Self {
        let mut c = Colours {
            keys: vec![EMPTY; TABLE_SLOTS],
            slot_of: vec![0; TABLE_SLOTS],
            slot_rgb: Vec::with_capacity(MAX_PALETTE_LEN),
            pal_of: vec![UNSEEN; MAX_PALETTE_LEN],
            palette: Vec::with_capacity(PALETTE_AREA),
            last: (EMPTY, 0),
        };
        // Cannot fail: the table is empty.
        let _ = c.slot([0, 0, 0]);
        c
    }

    /// The slot of a painted colour, making one on first sight. A 257th
    /// colour is [`TooManyColours`].
    #[inline]
    fn slot(&mut self, c: [u8; 3]) -> io::Result<u8> {
        let rgb = u32::from_be_bytes([0, c[0], c[1], c[2]]);
        if rgb == self.last.0 {
            return Ok(self.last.1);
        }
        let mut s = (rgb.wrapping_mul(0x9E37_79B1) >> 23) as usize & (TABLE_SLOTS - 1);
        let slot = loop {
            match self.keys[s] {
                key if key == rgb => break self.slot_of[s],
                EMPTY => {
                    if self.slot_rgb.len() == MAX_PALETTE_LEN {
                        return Err(io::Error::new(io::ErrorKind::InvalidData, TooManyColours));
                    }
                    let slot = self.slot_rgb.len() as u8;
                    self.keys[s] = rgb;
                    self.slot_of[s] = slot;
                    self.slot_rgb.push(c);
                    break slot;
                }
                _ => s = (s + 1) & (TABLE_SLOTS - 1),
            }
        };
        self.last = (rgb, slot);
        Ok(slot)
    }

    /// Black's slot for `c` (see the type's docs), if black may give it up:
    /// the table is full, black was never seen, and `in_use` (the screen and
    /// the `Previous` snapshot) holds no black pixel.
    fn recycle_black(&mut self, c: [u8; 3], in_use: &[&[u8]]) -> Option<u8> {
        if self.slot_rgb.len() < MAX_PALETTE_LEN
            || self.slot_rgb[BLACK_SLOT as usize] != [0, 0, 0]
            || self.pal_of[BLACK_SLOT as usize] != UNSEEN
            || in_use.iter().any(|px| px.contains(&BLACK_SLOT))
        {
            return None;
        }
        let pos = |keys: &[u32], rgb: u32| {
            let mut s = (rgb.wrapping_mul(0x9E37_79B1) >> 23) as usize & (TABLE_SLOTS - 1);
            while keys[s] != rgb && keys[s] != EMPTY {
                s = (s + 1) & (TABLE_SLOTS - 1);
            }
            s
        };
        let black = pos(&self.keys, 0);
        self.keys[black] = GONE;
        let rgb = u32::from_be_bytes([0, c[0], c[1], c[2]]);
        let s = pos(&self.keys, rgb);
        self.keys[s] = rgb;
        self.slot_of[s] = BLACK_SLOT;
        self.slot_rgb[BLACK_SLOT as usize] = c;
        self.last = (rgb, BLACK_SLOT);
        Some(BLACK_SLOT)
    }

    /// Is this slot's colour black?
    fn is_black(&self, slot: u8) -> bool {
        self.slot_rgb[slot as usize] == [0, 0, 0]
    }

    /// The palette index of a slot, assigning the next one on first sight.
    #[inline]
    fn index(&mut self, slot: u8) -> u8 {
        let p = &mut self.pal_of[slot as usize];
        if *p == UNSEEN {
            // At most one index per slot, and at most 256 slots.
            *p = (self.palette.len() / 3) as u16;
            self.palette
                .extend_from_slice(&self.slot_rgb[slot as usize]);
        }
        *p as u8
    }
}

/// A GIF decoded for [`encode_gif_streaming`]: exactly [`GifSource`]'s
/// compositing (disposal, transparency, local palettes, crop of frames
/// that overhang), but onto a screen of colour *slots* (1 B a pixel, not an
/// RGB canvas at 3), with the frame decoded a row at a time into a row
/// buffer rather than a `w*h` one. Same decoder, so the same errors.
struct SlotGif<R: Read> {
    decoder: gif::Decoder<R>,
    width: usize,
    height: usize,
    screen: Vec<u8>,
    /// Snapshot for `DisposalMethod::Previous`, allocated on first use.
    saved: Vec<u8>,
    pending: Option<(gif::DisposalMethod, Rect)>,
    /// The frame's indices: `w*h`, as `GifSource` has. Not a row buffer:
    /// the `gif` decoder can end a frame early when filled a row at a time
    /// that it decodes whole into one buffer (found on the card), so the
    /// buffer shape is `GifSource`'s exactly.
    indexed: Vec<u8>,
}

impl<R: Read> SlotGif<R> {
    fn new(reader: R) -> io::Result<Self> {
        let mut opts = gif::DecodeOptions::new();
        opts.set_color_output(gif::ColorOutput::Indexed);
        let decoder = opts.read_info(reader).map_err(decode_err)?;
        let (width, height) = (decoder.width() as usize, decoder.height() as usize);
        if width == 0 || height == 0 {
            return Err(invalid("GIF logical screen has a zero dimension"));
        }
        if width.saturating_mul(height) > MAX_LOGICAL_PIXELS {
            return Err(invalid(format!(
                "GIF logical screen {width}x{height} is implausibly large"
            )));
        }
        Ok(SlotGif {
            decoder,
            width,
            height,
            screen: vec![BLACK_SLOT; width * height],
            saved: Vec::new(),
            pending: None,
            indexed: vec![0; width * height],
        })
    }

    /// Composite the next frame onto `screen`; its delay, or `None` at the
    /// end. Mirrors [`GifSource::next_frame`] step for step.
    fn next_frame(&mut self, colours: &mut Colours) -> io::Result<Option<Duration>> {
        let SlotGif {
            decoder,
            width,
            height,
            screen,
            saved,
            pending,
            indexed,
        } = self;
        let (width, height) = (*width, *height);
        let (rect, dispose, delay, transparent, interlaced, frame_w, frame_h) =
            match decoder.next_frame_info().map_err(decode_err)? {
                Some(f) => (
                    Rect {
                        left: (f.left as usize).min(width),
                        top: (f.top as usize).min(height),
                        width: f.width as usize,
                        height: f.height as usize,
                    },
                    f.dispose,
                    f.delay,
                    f.transparent,
                    f.interlaced,
                    f.width as usize,
                    f.height as usize,
                ),
                None => return Ok(None),
            };
        let mut pal = [0u8; MAX_PALETTE];
        let pal_len = {
            let p = decoder.palette().map_err(decode_err)?;
            let n = p.len().min(MAX_PALETTE);
            pal[..n].copy_from_slice(&p[..n]);
            n
        };
        let pal = &pal[..pal_len];

        if let Some((method, prev)) = pending.take() {
            match method {
                gif::DisposalMethod::Background => {
                    let black = colours.slot([0, 0, 0])?;
                    let x1 = prev.left.saturating_add(prev.width).min(width);
                    for y in prev.top..prev.top.saturating_add(prev.height).min(height) {
                        screen[y * width + prev.left..y * width + x1].fill(black);
                    }
                }
                gif::DisposalMethod::Previous if saved.len() == screen.len() => {
                    screen.copy_from_slice(saved);
                }
                _ => {}
            }
        }
        if dispose == gif::DisposalMethod::Previous {
            saved.resize(screen.len(), 0);
            saved.copy_from_slice(screen);
        }

        let mut paint = |src_y: usize, pixels: &[u8]| -> io::Result<()> {
            let dy = rect.top + src_y;
            if dy >= height {
                return Ok(());
            }
            for (x, &idx) in pixels.iter().enumerate() {
                let dx = rect.left + x;
                if dx >= width {
                    break;
                }
                if transparent != Some(idx) {
                    let o = idx as usize * 3;
                    if let Some(c) = pal.get(o..o + 3) {
                        let c = [c[0], c[1], c[2]];
                        let slot = match colours.slot(c) {
                            Ok(s) => s,
                            Err(e) => colours
                                .recycle_black(c, &[&screen[..], &saved[..]])
                                .ok_or(e)?,
                        };
                        screen[dy * width + dx] = slot;
                    }
                }
            }
            Ok(())
        };
        let frame_pixels = frame_w.saturating_mul(frame_h);
        if frame_pixels <= indexed.len() {
            decoder
                .read_into_buffer(&mut indexed[..frame_pixels])
                .map_err(decode_err)?;
            for y in 0..frame_h {
                paint(y, &indexed[y * frame_w..(y + 1) * frame_w])?;
            }
        } else {
            if frame_w > indexed.len() {
                return Err(invalid(format!(
                    "GIF frame is {frame_w} px wide, wider than the {width}x{height} logical screen holds"
                )));
            }
            let mut rows = RowOrder::new(frame_h, interlaced);
            while let Some(src_y) = rows.next() {
                if !decoder
                    .fill_buffer(&mut indexed[..frame_w])
                    .map_err(decode_err)?
                {
                    return Err(invalid("GIF frame data ended early"));
                }
                paint(src_y, &indexed[..frame_w])?;
            }
        }
        *pending = Some((dispose, rect));
        Ok(Some(frame_delay(delay)))
    }

    fn into_reader(self) -> R {
        self.decoder.into_inner().into_inner()
    }
}

/// One op of the greedy rule, as [`greedy_ops`] hands them out.
#[derive(Clone, Copy)]
enum Op {
    Keep(usize),
    Fill(usize, u8),
    /// Literal indices `start..end`.
    Lit(usize, usize),
}

impl Op {
    fn encoded_len(self) -> u64 {
        let head = |len: usize| if len < LEN_ESCAPE as usize { 1 } else { 3 };
        match self {
            Op::Keep(len) => head(len),
            Op::Fill(len, _) => head(len) + 1,
            Op::Lit(s, e) => head(e - s) + (e - s) as u64,
        }
    }
}

/// [`FrameEncoder::encode`]'s greedy rule without its two `u32`-per-pixel
/// tables (32 KB for 128x32): a KEEP or FILL run's length is found by
/// scanning forward from where it starts, and "does a run of at least 4 open
/// here?" by looking four pixels ahead. Exactly the same op stream; see the
/// byte-identity tests. `unchanged` is a bitmap, one bit per pixel.
fn greedy_ops(
    idx: &[u8],
    unchanged: &[u8],
    mut emit: impl FnMut(Op) -> io::Result<()>,
) -> io::Result<()> {
    let n = idx.len();
    let same = |i: usize| unchanged[i / 8] & (1 << (i % 8)) != 0;
    let fill_run = |p: usize| idx[p..].iter().take_while(|&&k| k == idx[p]).count();
    let keep_run = |p: usize| (p..n).take_while(|&i| same(i)).count();
    let good = |q: usize| {
        q + MIN_RUN as usize <= n
            && ((q + 1..q + MIN_RUN as usize).all(|i| idx[i] == idx[q])
                || (q..q + MIN_RUN as usize).all(same))
    };
    let mut p = 0;
    while p < n {
        if good(p) {
            let (k, f) = (keep_run(p), fill_run(p));
            if k >= f {
                let len = k.min(MAX_RUN);
                emit(Op::Keep(len))?;
                p += len;
            } else {
                let len = f.min(MAX_RUN);
                emit(Op::Fill(len, idx[p]))?;
                p += len;
            }
        } else {
            let mut q = p + 1;
            while q < n && !good(q) && q - p < MAX_RUN {
                q += 1;
            }
            emit(Op::Lit(p, q))?;
            p = q;
        }
    }
    Ok(())
}

fn write_op(out: &mut impl Write, idx: &[u8], op: Op) -> io::Result<()> {
    let head = |out: &mut dyn Write, tag: u8, len: usize| {
        if len < LEN_ESCAPE as usize {
            out.write_all(&[tag << 6 | len as u8])
        } else {
            let l = (len as u16).to_le_bytes();
            out.write_all(&[tag << 6 | LEN_ESCAPE, l[0], l[1]])
        }
    };
    match op {
        Op::Keep(len) => head(out, TAG_KEEP, len),
        Op::Fill(len, k) => {
            head(out, TAG_FILL, len)?;
            out.write_all(&[k])
        }
        Op::Lit(s, e) => {
            head(out, TAG_LIT, e - s)?;
            out.write_all(&idx[s..e])
        }
    }
}

/// Counts what is read through it, for [`EncodeStats::gif_bytes`].
struct CountingReader<R> {
    inner: R,
    bytes: u64,
}

impl<R: Read> Read for CountingReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.bytes += n as u64;
        Ok(n)
    }
}

/// Convert a GIF to ODM v1 in one pass with bounded memory — the encoder
/// the ESP32 runs to prepare its own cache.
///
/// Decodes through [`GifSource`] exactly like [`encode_gif`], but assigns
/// palette indices in first-seen order *while* it encodes each frame, keeps
/// only the previous frame's indices (`w*h` bytes) and a one-bit-per-pixel
/// "unchanged" map, and never holds the GIF or more than one frame. The
/// palette's length is only known at the end, so the frames are written
/// after a [`PALETTE_AREA`] (768 B) gap; at the end the header and palette
/// go in at offset 0 and, when the palette is shorter than 256 entries, the
/// frames are moved down over the rest of the gap, 2 KB at a time. That is
/// why `out` must be readable and seekable. **The file is then
/// [`EncodeStats::odm_bytes`] long, and anything past that is stale: the
/// caller truncates it** (`File::set_len`, `Vec::truncate`).
///
/// For a GIF of at most 256 colours the output is byte-identical to
/// [`encode_gif`]'s (the palette order and the greedy ops are the same
/// rule). A 257th colour is `Err` with [`TooManyColours`] inside
/// ([`is_too_many_colours`]); frames have been written by then, so write to
/// a temporary file and remove it. A truncated or corrupt GIF is
/// `InvalidData`, never a panic. `between` is called before each frame and
/// each 2 KB of the final move; an error from it abandons the encode (the
/// firmware's pause).
///
/// Heap beyond the decoder's own (see "GIF heap budget"): the caller's-size
/// canvas it decodes into (12 KB), `w*h` bytes of indices, `w*h/8` of
/// unchanged bits, the 2.5 KB colour table and a 2 KB write buffer.
pub fn encode_gif_streaming<R: Read, W: Read + Write + Seek>(
    reader: R,
    out: &mut W,
    mut between: impl FnMut() -> io::Result<()>,
) -> io::Result<EncodeStats> {
    let mut src = SlotGif::new(CountingReader {
        inner: reader,
        bytes: 0,
    })?;
    let (width, height) = (src.width, src.height);
    if width > u16::MAX as usize || height > u16::MAX as usize {
        return Err(invalid("GIF screen too large for ODM"));
    }
    let n = width * height;
    let mut colours = Colours::new();
    let mut idx = vec![0u8; n];
    let mut unchanged = vec![0u8; n.div_ceil(8)];
    let mut stats = EncodeStats {
        width,
        height,
        ..Default::default()
    };

    out.seek(SeekFrom::Start(0))?;
    let mut w = BufWriter::with_capacity(STREAM_BUF, &mut *out);
    // Placeholder header and palette area, both rewritten at the end.
    w.write_all(&[0u8; HEADER_LEN + PALETTE_AREA])?;
    let mut frames_len: u64 = 0;
    let mut first = true;
    loop {
        between()?;
        let Some(delay) = src.next_frame(&mut colours)? else {
            break;
        };
        stats.frames = stats
            .frames
            .checked_add(1)
            .ok_or_else(|| invalid("more frames than ODM can count"))?;
        unchanged.fill(0);
        for (i, &slot) in src.screen.iter().enumerate() {
            let k = colours.index(slot);
            // Frame 0 is diffed against black (not index 0); after that,
            // equal indices are equal colours, since the palette is exact.
            let same = if first {
                colours.is_black(slot)
            } else {
                idx[i] == k
            };
            if same {
                unchanged[i / 8] |= 1 << (i % 8);
            }
            idx[i] = k;
        }
        first = false;
        // Once to size it (data_len comes first), once to write it.
        let mut data_len: u64 = 0;
        greedy_ops(&idx, &unchanged, |op| {
            data_len += op.encoded_len();
            match op {
                Op::Keep(l) => stats.keep_px += l as u64,
                Op::Fill(l, _) => stats.fill_px += l as u64,
                Op::Lit(s, e) => stats.lit_px += (e - s) as u64,
            }
            Ok(())
        })?;
        let delay_ms = delay.as_millis().min(u16::MAX as u128) as u16;
        w.write_all(&delay_ms.to_le_bytes())?;
        w.write_all(&(data_len as u32).to_le_bytes())?;
        greedy_ops(&idx, &unchanged, |op| write_op(&mut w, &idx, op))?;
        frames_len += 6 + data_len;
    }
    if stats.frames == 0 {
        return Err(invalid("GIF has no frames"));
    }
    w.flush()?;
    drop(w);
    stats.gif_bytes = src.into_reader().bytes;
    drop((idx, unchanged));

    let palette = colours.palette;
    stats.palette_len = palette.len() / 3;
    let header = Header {
        width: width as u16,
        height: height as u16,
        frame_count: stats.frames,
        palette_len: stats.palette_len as u16,
    };
    out.seek(SeekFrom::Start(0))?;
    out.write_all(&header.to_bytes())?;
    out.write_all(&palette)?;
    // Move the frames down over the unused part of the palette area.
    let from = (HEADER_LEN + PALETTE_AREA) as u64;
    let to = (HEADER_LEN + palette.len()) as u64;
    if to < from {
        let mut buf = vec![0u8; STREAM_BUF];
        let mut done = 0u64;
        while done < frames_len {
            between()?;
            let m = (frames_len - done).min(STREAM_BUF as u64) as usize;
            out.seek(SeekFrom::Start(from + done))?;
            out.read_exact(&mut buf[..m])?;
            out.seek(SeekFrom::Start(to + done))?;
            out.write_all(&buf[..m])?;
            done += m as u64;
        }
    }
    out.flush()?;
    stats.odm_bytes = to + frames_len;
    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::Rgb;
    use crate::player::FrameSource;
    use gif::{DisposalMethod, Encoder, Frame, Repeat};
    use std::borrow::Cow;
    use std::io::Cursor;

    struct Spec {
        rect: (u16, u16, u16, u16),
        pixels: Vec<u8>,
        palette: Vec<u8>,
        transparent: Option<u8>,
        dispose: DisposalMethod,
        delay: u16,
        interlaced: bool,
    }

    fn gif(w: u16, h: u16, frames: Vec<Spec>) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut enc = Encoder::new(&mut out, w, h, &[]).unwrap();
            enc.set_repeat(Repeat::Infinite).unwrap();
            for s in frames {
                let f = Frame {
                    left: s.rect.0,
                    top: s.rect.1,
                    width: s.rect.2,
                    height: s.rect.3,
                    buffer: Cow::Owned(s.pixels),
                    palette: Some(s.palette),
                    transparent: s.transparent,
                    dispose: s.dispose,
                    delay: s.delay,
                    interlaced: s.interlaced,
                    ..Default::default()
                };
                enc.write_frame(&f).unwrap();
            }
        }
        out
    }

    /// Black, red, green, blue, white; index 0 transparent where used.
    fn pal() -> Vec<u8> {
        vec![0, 0, 0, 255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255]
    }

    fn full(w: u16, h: u16, f: impl Fn(usize, usize) -> u8, dispose: DisposalMethod) -> Spec {
        let mut pixels = Vec::with_capacity(w as usize * h as usize);
        for y in 0..h as usize {
            for x in 0..w as usize {
                pixels.push(f(x, y));
            }
        }
        Spec {
            rect: (0, 0, w, h),
            pixels,
            palette: pal(),
            transparent: Some(0),
            dispose,
            delay: 7,
            interlaced: false,
        }
    }

    /// Every canvas and delay `GifSource` produces, and the same through the
    /// encoder and `OdmSource`, must be identical (and so must end-of-stream).
    fn assert_round_trip(gif_bytes: &[u8]) -> EncodeStats {
        let mut odm = Vec::new();
        let stats = encode_gif(gif_bytes, &mut odm).unwrap();
        assert_eq!(stats.odm_bytes, odm.len() as u64);
        assert_eq!(stats.gif_bytes, gif_bytes.len() as u64);

        let mut g = GifSource::new(gif_bytes).unwrap();
        let mut o = OdmSource::new(&odm[..]).unwrap();
        assert_eq!(g.logical_size(), o.logical_size());
        let (mut cg, mut co) = (Canvas::new(), Canvas::new());
        // A dirty canvas must not leak into the first frame of either.
        cg.fill(Rgb::new(9, 9, 9));
        co.fill(Rgb::new(9, 9, 9));
        let mut n = 0;
        loop {
            let (before_g, before_o) = (cg.clone(), co.clone());
            cg.dirty_mut().clear();
            co.dirty_mut().clear();
            let (dg, d_o) = (
                g.next_frame(&mut cg).unwrap(),
                o.next_frame(&mut co).unwrap(),
            );
            assert_eq!(dg, d_o, "delay of frame {n}");
            if dg.is_none() {
                assert!(cg.dirty().is_clean() && co.dirty().is_clean());
                break;
            }
            assert!(cg == co, "canvas of frame {n} differs");
            // Whatever changed was marked, by both decoders.
            assert_dirty_covers(&before_g, &cg, n);
            assert_dirty_covers(&before_o, &co, n);
            n += 1;
        }
        assert_eq!(n, stats.frames as usize);
        assert_eq!(o.frames_decoded(), n as u64);
        assert_eq!(
            stats.total_px(),
            n as u64 * (stats.width * stats.height) as u64
        );
        stats
    }

    /// Every pixel that differs between `before` and `after` is dirty in
    /// `after` — the one guarantee the panel's partial blit relies on.
    fn assert_dirty_covers(before: &Canvas, after: &Canvas, frame: usize) {
        for y in 0..HEIGHT {
            for x in 0..WIDTH {
                if before.get(x, y) != after.get(x, y) {
                    assert!(
                        after.dirty().is_dirty(x, y),
                        "frame {frame}: ({x},{y}) changed but was not marked"
                    );
                }
            }
        }
    }

    #[test]
    fn dirty_map_is_what_changed() {
        let bytes = pattern_gif(128, 32);
        let mut odm = Vec::new();
        encode_gif(&bytes[..], &mut odm).unwrap();
        let mut g = GifSource::new(&bytes[..]).unwrap();
        let mut o = OdmSource::new(&odm[..]).unwrap();
        let (mut cg, mut co) = (Canvas::new(), Canvas::new());
        for frame in 0..4 {
            let before = cg.clone();
            cg.dirty_mut().clear();
            co.dirty_mut().clear();
            g.next_frame(&mut cg).unwrap().unwrap();
            o.next_frame(&mut co).unwrap().unwrap();
            let changed = (0..HEIGHT)
                .flat_map(|y| (0..WIDTH).map(move |x| (x, y)))
                .filter(|&(x, y)| before.get(x, y) != cg.get(x, y))
                .count() as u32;
            let (dg, d_o) = (cg.dirty().count(), co.dirty().count());
            if frame == 0 {
                // The first frame clears the canvas: everything is marked.
                assert_eq!((dg, d_o), (4096, 4096));
            } else if frame == 1 {
                // No disposal owed yet (frame 0 is Keep): exactly the pixels
                // that changed, from both decoders — the 6x4 rectangle's
                // transparent and same-colour pixels are not in it.
                assert_eq!((dg, d_o), (changed, changed));
                assert!(changed > 0 && changed < 24, "{changed}");
            } else {
                // A disposal may mark a pixel the next frame paints back to
                // what it was; never fewer than changed, never the screen.
                assert!(dg >= changed && d_o >= changed, "frame {frame}");
                assert!(dg < 4096 && d_o < 4096, "frame {frame}: {dg} {d_o}");
            }
        }
    }

    fn pattern_gif(w: u16, h: u16) -> Vec<u8> {
        gif(
            w,
            h,
            vec![
                // Stripes plus a noisy patch: FILL, LIT and KEEP all occur.
                full(w, h, |x, y| ((x / 7 + y) % 5) as u8, DisposalMethod::Keep),
                Spec {
                    rect: (10, 3, 6, 4),
                    pixels: (0..24).map(|i| (i * 7 % 5) as u8).collect(),
                    palette: pal(),
                    transparent: Some(0),
                    dispose: DisposalMethod::Background,
                    delay: 0,
                    interlaced: false,
                },
                Spec {
                    rect: (30, 10, 20, 2),
                    pixels: vec![4; 40],
                    palette: pal(),
                    transparent: Some(0),
                    dispose: DisposalMethod::Previous,
                    delay: 1,
                    interlaced: false,
                },
                full(w, h, |x, _| (x % 2) as u8 * 2, DisposalMethod::Keep),
            ],
        )
    }

    #[test]
    fn round_trips_transparency_and_every_disposal() {
        let s = assert_round_trip(&pattern_gif(128, 32));
        assert!(!s.quantised);
        assert!(s.keep_px > 0 && s.fill_px > 0 && s.lit_px > 0, "{s:?}");
        assert_eq!(s.frames, 4);
    }

    #[test]
    fn round_trips_a_135x32_stray_through_the_crop() {
        let s = assert_round_trip(&pattern_gif(135, 32));
        assert_eq!((s.width, s.height), (135, 32));
        let s = assert_round_trip(&pattern_gif(10, 5));
        assert_eq!((s.width, s.height), (10, 5));
    }

    #[test]
    fn palette_is_first_seen_order_and_frame_zero_diffs_against_black() {
        // White first, then black: palette [white, black]. Frame 0's black
        // pixels are KEEP (unchanged from all-black), not FILL of index 1.
        let g = gif(
            8,
            1,
            vec![Spec {
                rect: (0, 0, 8, 1),
                pixels: vec![4, 0, 0, 0, 0, 0, 0, 0],
                palette: pal(),
                transparent: None,
                dispose: DisposalMethod::Keep,
                delay: 5,
                interlaced: false,
            }],
        );
        let mut odm = Vec::new();
        let s = encode_gif(&g[..], &mut odm).unwrap();
        assert_eq!(s.palette_len, 2);
        assert_eq!(&odm[HEADER_LEN..HEADER_LEN + 6], &[255, 255, 255, 0, 0, 0]);
        // delay 50 ms, 5 bytes of ops: LIT 1 [0], KEEP 7.
        let frame = &odm[HEADER_LEN + 6..];
        assert_eq!(&frame[..6], &[50, 0, 3, 0, 0, 0]);
        assert_eq!(&frame[6..], &[0x81, 0, 7]);
        assert_eq!((s.keep_px, s.fill_px, s.lit_px), (7, 0, 1));
        assert_round_trip(&g);
    }

    #[test]
    fn more_than_256_colours_is_quantised() {
        // Two frames, 300 distinct colours between them, no transparency.
        let mut frames = Vec::new();
        for base in [0u16, 150] {
            let mut palette = Vec::new();
            for i in 0..150u16 {
                let v = base + i;
                palette.extend_from_slice(&[(v % 256) as u8, (v * 3 % 256) as u8, (v / 2) as u8]);
            }
            palette.resize(256 * 3, 0);
            frames.push(Spec {
                rect: (0, 0, 128, 32),
                pixels: (0..128 * 32).map(|i| (i % 150) as u8).collect(),
                palette,
                transparent: None,
                dispose: DisposalMethod::Keep,
                delay: 4,
                interlaced: false,
            });
        }
        let g = gif(128, 32, frames);
        let mut odm = Vec::new();
        let s = encode_gif(&g[..], &mut odm).unwrap();
        assert!(s.quantised);
        assert_eq!(s.palette_len, 256);

        // Lossy by design: same frames and delays, colours close.
        let mut gs = GifSource::new(&g[..]).unwrap();
        let mut os = OdmSource::new(&odm[..]).unwrap();
        let (mut cg, mut co) = (Canvas::new(), Canvas::new());
        let mut frames = 0;
        while let Some(d) = gs.next_frame(&mut cg).unwrap() {
            assert_eq!(os.next_frame(&mut co).unwrap(), Some(d));
            let worst = cg
                .as_bytes()
                .iter()
                .zip(co.as_bytes())
                .map(|(a, b)| a.abs_diff(*b))
                .max()
                .unwrap();
            assert!(worst < 64, "quantisation error {worst}");
            frames += 1;
        }
        assert_eq!(frames, 2);
        assert!(os.next_frame(&mut co).unwrap().is_none());
    }

    #[test]
    fn long_runs_use_the_u16_escape() {
        // A solid frame then an identical one: FILL 4096 then KEEP 4096.
        let g = gif(
            128,
            32,
            vec![
                full(128, 32, |_, _| 1, DisposalMethod::Keep),
                full(128, 32, |_, _| 1, DisposalMethod::Keep),
            ],
        );
        let mut odm = Vec::new();
        encode_gif(&g[..], &mut odm).unwrap();
        let f0 = HEADER_LEN + 3 + 6;
        assert_eq!(&odm[f0..f0 + 4], &[0x40 | 63, 0x00, 0x10, 0]);
        let f1 = f0 + 4 + 6;
        assert_eq!(&odm[f1..], &[63, 0x00, 0x10]);
        assert_round_trip(&g);
    }

    #[test]
    fn frame_source_sniffs_both_formats() {
        let g = pattern_gif(128, 32);
        let mut odm = Vec::new();
        encode_gif(&g[..], &mut odm).unwrap();
        let mut a = FrameSource::open(Cursor::new(g)).unwrap();
        let mut b = FrameSource::open(Cursor::new(odm)).unwrap();
        assert_eq!((a.kind(), b.kind()), ("gif", "odm"));
        let (mut ca, mut cb) = (Canvas::new(), Canvas::new());
        while let Some(d) = a.next_frame(&mut ca).unwrap() {
            assert_eq!(b.next_frame(&mut cb).unwrap(), Some(d));
            assert!(ca == cb);
        }
        assert!(b.next_frame(&mut cb).unwrap().is_none());
        assert_eq!(a.frames_decoded(), b.frames_decoded());
        assert!(FrameSource::open(Cursor::new(b"GIF".to_vec())).is_err());
        assert!(FrameSource::open(Cursor::new(b"PNG\x89....".to_vec())).is_err());
    }

    /// The streaming encoder's whole output (truncated as the caller must).
    fn stream(gif_bytes: &[u8]) -> io::Result<(Vec<u8>, EncodeStats)> {
        let mut out = Cursor::new(Vec::new());
        let stats = encode_gif_streaming(gif_bytes, &mut out, || Ok(()))?;
        let mut v = out.into_inner();
        assert!(v.len() as u64 >= stats.odm_bytes);
        v.truncate(stats.odm_bytes as usize);
        Ok((v, stats))
    }

    /// Byte-identical to `encode_gif`, same stats (bar the bytes read).
    fn assert_stream_matches(g: &[u8]) {
        let mut want = Vec::new();
        let ws = encode_gif(g, &mut want).unwrap();
        let (got, gs) = stream(g).unwrap();
        assert_eq!(got.len(), want.len(), "length");
        assert!(got == want, "bytes differ");
        assert_eq!(
            EncodeStats { gif_bytes: 0, ..gs },
            EncodeStats { gif_bytes: 0, ..ws }
        );
    }

    /// A random animation over at most `colours` distinct colours: frames
    /// of random rectangles with random disposal, transparency and local
    /// palettes, runs and noise mixed.
    fn random_gif(seed: u32, w: u16, h: u16, colours: usize) -> Vec<u8> {
        let mut s = seed;
        let mut rnd = |m: u32| {
            s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (s >> 8) % m.max(1)
        };
        let pool: Vec<[u8; 3]> = (0..colours)
            // Black first: transparent and disposed pixels are black too.
            .map(|i| [(i * 37) as u8, (i * 101) as u8, (i * 13 / 3) as u8])
            .collect();
        let mut frames = Vec::new();
        for f in 0..(2 + rnd(6)) {
            let (fw, fh) = if f == 0 {
                (w, h)
            } else {
                (1 + rnd(w as u32) as u16, 1 + rnd(h as u32) as u16)
            };
            let (l, t) = (
                rnd((w - fw + 1) as u32) as u16,
                rnd((h - fh + 1) as u32) as u16,
            );
            let npal = 2 + rnd(15) as usize;
            let mut palette = Vec::new();
            for _ in 0..npal {
                palette.extend_from_slice(&pool[rnd(pool.len() as u32) as usize]);
            }
            let noisy = rnd(2) == 0;
            let mut last = 0u8;
            let pixels = (0..fw as usize * fh as usize)
                .map(|_| {
                    if noisy || rnd(8) == 0 {
                        last = rnd(npal as u32) as u8;
                    }
                    last
                })
                .collect();
            frames.push(Spec {
                rect: (l, t, fw, fh),
                pixels,
                palette,
                transparent: (rnd(2) == 0).then_some(0),
                dispose: [
                    DisposalMethod::Keep,
                    DisposalMethod::Background,
                    DisposalMethod::Previous,
                    DisposalMethod::Any,
                ][rnd(4) as usize],
                delay: rnd(12) as u16,
                interlaced: rnd(3) == 0,
            });
        }
        gif(w, h, frames)
    }

    #[test]
    fn streaming_is_byte_identical_to_the_desktop_encoder() {
        assert_stream_matches(&pattern_gif(128, 32));
        assert_stream_matches(&pattern_gif(135, 32));
        assert_stream_matches(&pattern_gif(10, 5));
        assert_stream_matches(&gif(
            8,
            1,
            vec![Spec {
                rect: (0, 0, 8, 1),
                pixels: vec![4, 0, 0, 0, 0, 0, 0, 0],
                palette: pal(),
                transparent: None,
                dispose: DisposalMethod::Keep,
                delay: 5,
                interlaced: false,
            }],
        ));
        // Solid frames: FILL 4096 then KEEP 4096, through the u16 escape.
        assert_stream_matches(&gif(
            128,
            32,
            vec![
                full(128, 32, |_, _| 1, DisposalMethod::Keep),
                full(128, 32, |_, _| 1, DisposalMethod::Keep),
            ],
        ));
        // A 256-entry palette: the palette area is exactly full, nothing moves.
        for (seed, colours) in [(1u32, 256usize), (2, 200), (3, 17), (4, 2), (5, 256)] {
            let g = random_gif(seed, 128, 32, colours);
            let (_, s) = stream(&g).unwrap();
            assert!(s.palette_len <= colours);
            assert_stream_matches(&g);
        }
        for seed in 10..60 {
            assert_stream_matches(&random_gif(seed, 128, 32, 1 + (seed as usize * 7) % 256));
        }
        for seed in 60..70 {
            assert_stream_matches(&random_gif(seed, 131 + seed as u16 % 5, 30, 40));
        }
    }

    #[test]
    fn streaming_indexes_256_colours_that_never_include_black() {
        // Frame 0 covers the screen in 200 colours, frame 1 adds 56 more:
        // 256 in all, none black. Black's slot (the empty screen's) must go
        // to the 256th, as the desktop encoder never sees black at all.
        let colour = |i: usize| [1 + (i % 250) as u8, (i * 7 / 3) as u8, (i / 250) as u8 + 1];
        let spec = |base: usize, count: usize| {
            let mut palette = Vec::new();
            for i in 0..count {
                palette.extend_from_slice(&colour(base + i));
            }
            palette.resize(256 * 3, 1);
            Spec {
                rect: (0, 0, 128, 32),
                pixels: (0..128 * 32).map(|i| (i % count) as u8).collect(),
                palette,
                transparent: None,
                dispose: DisposalMethod::Keep,
                delay: 4,
                interlaced: false,
            }
        };
        let g = gif(128, 32, vec![spec(0, 200), spec(200, 56)]);
        let (_, s) = stream(&g).unwrap();
        assert_eq!(s.palette_len, 256);
        assert_stream_matches(&g);
        // Disposal to black once all 256 are taken is past what it can do.
        let mut last = spec(0, 1);
        last.rect = (0, 0, 4, 4);
        last.pixels = vec![0; 16];
        let mut first = spec(200, 56);
        first.dispose = DisposalMethod::Background;
        let g = gif(128, 32, vec![spec(0, 200), first, last]);
        assert!(is_too_many_colours(&stream(&g).unwrap_err()));
    }

    #[test]
    fn streaming_a_257th_colour_is_too_many_colours() {
        // The 300-colour fixture the desktop encoder quantises.
        let mut frames = Vec::new();
        for base in [0u16, 150] {
            let mut palette = Vec::new();
            for i in 0..150u16 {
                let v = base + i;
                palette.extend_from_slice(&[(v % 256) as u8, (v * 3 % 256) as u8, (v / 2) as u8]);
            }
            palette.resize(256 * 3, 0);
            frames.push(Spec {
                rect: (0, 0, 128, 32),
                pixels: (0..128 * 32).map(|i| (i % 150) as u8).collect(),
                palette,
                transparent: None,
                dispose: DisposalMethod::Keep,
                delay: 4,
                interlaced: false,
            });
        }
        let err = stream(&gif(128, 32, frames)).unwrap_err();
        assert!(is_too_many_colours(&err), "{err}");
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        // Anything else is not.
        assert!(!is_too_many_colours(&invalid("x")));
    }

    #[test]
    fn streaming_a_truncated_or_corrupt_gif_errors_never_panics() {
        let g = random_gif(7, 128, 32, 30);
        // Every cut in the header and the first frame's descriptor, then
        // every 7th byte (all of them take ~10 s unoptimised).
        for n in (0..g.len()).filter(|&n| n < 900 || n % 7 == 0) {
            match stream(&g[..n]) {
                Err(_) => {}
                // A cut on a frame boundary is a shorter animation, and a
                // valid file.
                Ok((odm, s)) => {
                    assert!(s.frames > 0);
                    decode_all(&odm).unwrap();
                }
            }
        }
        // Cut in the middle of the first frame's image data: an error.
        assert!(stream(&g[..g.len() / 3]).is_err());
        assert!(stream(b"GIF89a").is_err());
        assert!(stream(b"").is_err());
        let mut seed = 99u32;
        for _ in 0..200 {
            let mut b = g.clone();
            for _ in 0..8 {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let i = (seed >> 8) as usize % b.len();
                b[i] ^= (seed >> 3) as u8 | 1;
            }
            if let Ok((odm, _)) = stream(&b) {
                decode_all(&odm).unwrap();
            }
        }
    }

    #[test]
    fn streaming_stops_when_told_to() {
        let g = random_gif(8, 128, 32, 30);
        let mut calls = 0;
        let mut out = Cursor::new(Vec::new());
        let err = encode_gif_streaming(&g[..], &mut out, || {
            calls += 1;
            if calls == 2 {
                Err(io::Error::other("paused"))
            } else {
                Ok(())
            }
        })
        .unwrap_err();
        assert_eq!(err.to_string(), "paused");
    }

    fn minimal_odm() -> Vec<u8> {
        // 2x1, one frame, palette [red]: FILL 2 of index 0.
        let mut v = Header {
            width: 2,
            height: 1,
            frame_count: 1,
            palette_len: 1,
        }
        .to_bytes()
        .to_vec();
        v.extend_from_slice(&[255, 0, 0]);
        v.extend_from_slice(&[10, 0, 2, 0, 0, 0, 0x42, 0]);
        v
    }

    #[test]
    fn a_hand_written_file_decodes() {
        let mut o = OdmSource::new(Cursor::new(minimal_odm())).unwrap();
        let mut c = Canvas::new();
        // The file says 10 ms; the decoder floors file delays at MIN_DELAY_MS.
        assert_eq!(
            o.next_frame(&mut c).unwrap(),
            Some(Duration::from_millis(MIN_DELAY_MS))
        );
        // 2x1 centred: (128-2)/2 = 63, (32-1)/2 = 15.
        assert_eq!(c.get(63, 15), Rgb::new(255, 0, 0));
        assert_eq!(c.get(64, 15), Rgb::new(255, 0, 0));
        assert_eq!(c.get(62, 15), Rgb::BLACK);
        assert!(o.next_frame(&mut c).unwrap().is_none());
    }

    fn decode_all(bytes: &[u8]) -> io::Result<()> {
        let mut o = OdmSource::new(bytes)?;
        let mut c = Canvas::new();
        while o.next_frame(&mut c)?.is_some() {}
        Ok(())
    }

    #[test]
    fn malformed_input_is_invalid_data_never_a_panic() {
        let good = minimal_odm();
        let kind = |b: &[u8]| decode_all(b).unwrap_err().kind();
        // Every truncation.
        for n in 0..good.len() {
            assert_eq!(kind(&good[..n]), io::ErrorKind::InvalidData, "cut at {n}");
        }
        let with = |i: usize, v: u8| {
            let mut b = good.clone();
            b[i] = v;
            b
        };
        assert_eq!(kind(&with(0, b'X')), io::ErrorKind::InvalidData, "magic");
        assert_eq!(kind(&with(4, 2)), io::ErrorKind::InvalidData, "version");
        assert_eq!(kind(&with(5, 0)), io::ErrorKind::InvalidData, "width 0");
        assert_eq!(kind(&with(13, 0)), io::ErrorKind::InvalidData, "palette 0");
        let ops = HEADER_LEN + 3 + 6;
        assert_eq!(
            kind(&with(ops + 1, 1)),
            io::ErrorKind::InvalidData,
            "palette index"
        );
        assert_eq!(
            kind(&with(ops, 0x43)),
            io::ErrorKind::InvalidData,
            "run overruns frame"
        );
        assert_eq!(kind(&with(ops, 0xC2)), io::ErrorKind::InvalidData, "tag 3");
        assert_eq!(
            kind(&with(ops - 4, 3)),
            io::ErrorKind::InvalidData,
            "data_len too long"
        );
        assert_eq!(
            kind(&with(ops - 4, 1)),
            io::ErrorKind::InvalidData,
            "data_len too short"
        );
        // Random garbage after a valid header must not panic either.
        let mut seed = 0x1234_5678u32;
        for _ in 0..500 {
            let mut b = good[..HEADER_LEN + 3].to_vec();
            b[9] = 3; // three frames
            for _ in 0..40 {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                b.push((seed >> 24) as u8);
            }
            let _ = decode_all(&b);
        }
    }
}

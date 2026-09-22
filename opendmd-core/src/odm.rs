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
//! encoder (desktop and wasm; the ESP32 never runs it).

use crate::canvas::{Canvas, HEIGHT, WIDTH};
use crate::player::{blit, invalid, GifSource, Screen, MAX_LOGICAL_PIXELS};
use std::collections::hash_map::{Entry as MapEntry, HashMap};
use std::io::{self, BufReader, Read, Write};
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
            let buf: &mut [u8] = match screen {
                Screen::Direct { cleared } => {
                    if !*cleared {
                        // Frame 0 is a delta against all-black.
                        canvas.clear();
                        *cleared = true;
                    }
                    canvas.as_mut_bytes()
                }
                Screen::Owned(v) => v.as_mut_slice(),
            };
            let consumed = apply_ops(reader, palette, buf, width * height, data_len)?;
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

/// Apply one frame's ops to `screen` (packed RGB, `pixels * 3` bytes).
/// Returns the op bytes consumed; never reads past `limit` of them.
fn apply_ops<R: Read>(
    reader: &mut R,
    palette: &[u8],
    screen: &mut [u8],
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
        match tag {
            TAG_KEEP => {}
            TAG_FILL => {
                take(1)?;
                let rgb = colour(read_u8(reader)?)?;
                for px in span.as_chunks_mut::<3>().0 {
                    px.copy_from_slice(rgb);
                }
            }
            TAG_LIT => {
                take(len as u64)?;
                for dst in span.chunks_mut(LIT_CHUNK * 3) {
                    let n = dst.len() / 3;
                    read_exact(reader, &mut lit[..n])?;
                    for (px, &idx) in dst.as_chunks_mut::<3>().0.iter_mut().zip(&lit[..n]) {
                        px.copy_from_slice(colour(idx)?);
                    }
                }
            }
            _ => return Err(invalid(format!("ODM op has reserved tag {tag}"))),
        }
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
            let (dg, d_o) = (
                g.next_frame(&mut cg).unwrap(),
                o.next_frame(&mut co).unwrap(),
            );
            assert_eq!(dg, d_o, "delay of frame {n}");
            if dg.is_none() {
                break;
            }
            assert!(cg == co, "canvas of frame {n} differs");
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
                },
                Spec {
                    rect: (30, 10, 20, 2),
                    pixels: vec![4; 40],
                    palette: pal(),
                    transparent: Some(0),
                    dispose: DisposalMethod::Previous,
                    delay: 1,
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

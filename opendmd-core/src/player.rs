//! Streaming GIF playback onto the 128x32 canvas.
//!
//! One frame is decoded at a time, **indexed**, into a single reusable
//! `width * height` byte buffer allocated once in [`GifSource::new`] and never
//! grown. The frame's own palette (local if it has one, else the global table)
//! is then applied while compositing into an **RGB** logical screen
//! (`width * height * 3`). A `Previous` disposal snapshot is allocated lazily
//! and is RGB too. A 4 MB GIF therefore costs ~`w*h*4` bytes, not 4 MB.
//!
//! When the GIF's logical screen is exactly `WIDTH x HEIGHT` — 11,044 of the
//! 11,047 files on the real card — there is no separate screen buffer at all:
//! the [`Canvas`] the caller passes in *is* the logical screen, composited
//! into directly through [`Canvas::as_mut_bytes`], and `blit` is skipped
//! because centre-and-crop at that size is the identity. That is another
//! 12 KB off the ESP32 heap. The canvas is cleared on the first frame, which
//! is what the old full-canvas `blit` from a zeroed screen did anyway, and it
//! must not be written by the caller between frames — it is the GIF's
//! accumulating state, which `Keep` and `Previous` disposal both depend on.
//!
//! Going through `ColorOutput::Indexed` plus the caller-owned-buffer path
//! (`next_frame_info` + `read_into_buffer`/`fill_buffer`) matters on the
//! ESP32: `ColorOutput::RGBA` makes the `gif` crate keep *two* extra buffers
//! alive per frame — a `w*h*4` RGBA `Frame::buffer` and a `w*h` indexed
//! scratch inside its `PixelConverter` — 20 KB for 128x32 that we do not need,
//! because the screen we composite into is the only thing we ever read back.
//!
//! There is no alpha channel on the screen: an untouched or background-cleared
//! pixel is RGB `(0,0,0)`, which is exactly what a transparent screen pixel
//! rendered as before (there is no "behind" on a HUB75 panel).
//!
//! Mismatched sizes are centred and cropped, never resampled — the archive is
//! pixel art and every scaling filter ruins it. With an odd difference the
//! extra pixel goes to the right/bottom.

use crate::canvas::{Canvas, DirtyMap, Rgb, HEIGHT, WIDTH};
use gif::{ColorOutput, DecodeOptions, DisposalMethod};
use std::io::{self, Read};
use std::time::Duration;

/// GIFs with a delay of 0 are "as fast as possible"; browsers and the stock
/// firmware both settle on 100 ms.
const ZERO_DELAY_MS: u64 = 100;
/// Floor for everything else — 50 fps is well past what a HUB75 refresh can do.
const MIN_DELAY_MS: u64 = 20;
/// Sanity cap on the declared logical screen, to bound the screen buffer.
pub(crate) const MAX_LOGICAL_PIXELS: usize = 2048 * 2048;
/// Bytes in a maximal GIF colour table: 256 entries of RGB.
pub(crate) const MAX_PALETTE: usize = 256 * 3;

pub(crate) fn decode_err(e: gif::DecodingError) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, e)
}

pub(crate) fn invalid(msg: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg)
}

/// A GIF being played. Construct one per playback; there is no rewind inside
/// the decoder, so looping means building a new one — over the same reader,
/// rewound, when it can seek ([`FrameSource::into_inner`]).
pub struct GifSource<R: Read> {
    decoder: gif::Decoder<R>,
    width: usize,
    height: usize,
    /// Indexed pixels of the frame being decoded. Exactly `width * height`
    /// bytes, sized once in `new` and never reallocated; a frame rectangle
    /// bigger than the logical screen goes down the row-chunked path instead.
    indexed: Vec<u8>,
    /// Logical screen, RGB, `width * height * 3` — borrowed from the caller's
    /// `Canvas` at the panel size, owned otherwise.
    screen: Screen,
    /// Snapshot for `DisposalMethod::Previous`; allocated on first use.
    saved: Vec<u8>,
    /// Disposal owed by the frame already on screen, with its rectangle.
    pending: Option<(DisposalMethod, Rect)>,
    frames_decoded: u64,
    /// The last frame's delay as written (hundredths).
    raw_delay: u16,
}

/// Where the RGB logical screen lives.
pub(crate) enum Screen {
    /// The logical screen is exactly the panel, so the caller's `Canvas` is
    /// the screen. `cleared` is false until the first frame zeroes it.
    Direct { cleared: bool },
    /// Any other size: an owned buffer that `blit` centre-crops onto the
    /// canvas each frame.
    Owned(Vec<u8>),
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Rect {
    pub(crate) left: usize,
    pub(crate) top: usize,
    pub(crate) width: usize,
    pub(crate) height: usize,
}

/// A frame's delay as playback uses it: GIF hundredths to a `Duration`,
/// 0 meaning [`ZERO_DELAY_MS`], floored at [`MIN_DELAY_MS`].
pub(crate) fn frame_delay(delay: u16) -> Duration {
    let delay_ms = if delay == 0 {
        ZERO_DELAY_MS
    } else {
        (delay as u64) * 10
    };
    Duration::from_millis(delay_ms.max(MIN_DELAY_MS))
}

impl<R: Read> GifSource<R> {
    /// Read the GIF header. Fails on anything the `gif` crate rejects, and on
    /// an absurd declared logical screen.
    pub fn new(reader: R) -> io::Result<Self> {
        let mut opts = DecodeOptions::new();
        opts.set_color_output(ColorOutput::Indexed);
        let decoder = opts.read_info(reader).map_err(decode_err)?;

        let width = decoder.width() as usize;
        let height = decoder.height() as usize;
        if width == 0 || height == 0 {
            return Err(invalid("GIF logical screen has a zero dimension"));
        }
        if width.saturating_mul(height) > MAX_LOGICAL_PIXELS {
            return Err(invalid(format!(
                "GIF logical screen {width}x{height} is implausibly large"
            )));
        }

        Ok(GifSource {
            decoder,
            width,
            height,
            indexed: vec![0u8; width * height],
            screen: if width == WIDTH && height == HEIGHT {
                Screen::Direct { cleared: false }
            } else {
                Screen::Owned(vec![0u8; width * height * 3])
            },
            saved: Vec::new(),
            pending: None,
            frames_decoded: 0,
            raw_delay: 0,
        })
    }

    /// The last frame's delay as written in the file, in hundredths (before
    /// the player's 0 -> 100 ms and 20 ms-floor rules).
    pub fn raw_delay(&self) -> u16 {
        self.raw_delay
    }

    /// The NETSCAPE loop count (the crate's `Decoder::repeat`).
    pub fn repeat(&self) -> gif::Repeat {
        self.decoder.repeat()
    }

    /// The reader, with the decoder (and its buffer) dropped.
    pub fn into_inner(self) -> R {
        self.decoder.into_inner().into_inner()
    }

    /// The GIF's declared logical screen size, which may not be 128x32.
    pub fn logical_size(&self) -> (usize, usize) {
        (self.width, self.height)
    }

    /// Frames handed out so far.
    pub fn frames_decoded(&self) -> u64 {
        self.frames_decoded
    }

    /// The RGB logical screen (`width * height * 3` bytes, row-major) as of
    /// the last frame handed out, before `blit` centre-crops it onto the
    /// canvas. At the panel size that is the caller's `canvas` itself, so it
    /// must be the one passed to [`next_frame`](Self::next_frame). The
    /// recoder reads frames through this so a stray keeps its own size.
    pub fn screen<'a>(&'a self, canvas: &'a Canvas) -> &'a [u8] {
        match &self.screen {
            Screen::Direct { .. } => canvas.as_bytes(),
            Screen::Owned(v) => v,
        }
    }

    /// Decode the next frame onto `canvas` and return how long to show it.
    /// `Ok(None)` means end of stream; `canvas` is then untouched.
    pub fn next_frame(&mut self, canvas: &mut Canvas) -> io::Result<Option<Duration>> {
        let GifSource {
            decoder,
            width,
            height,
            indexed,
            screen,
            saved,
            pending,
            frames_decoded,
            raw_delay,
        } = self;
        let (width, height) = (*width, *height);

        // Metadata first, before anything touches the screen: `Ok(None)` must
        // leave the canvas exactly as it was, and in `Direct` mode the canvas
        // *is* the screen. Reading the frame header does not disturb it.
        //
        // `read_next_frame` is deliberately not used: it allocates a frame
        // buffer per frame, which is the whole thing we are avoiding. Copy the
        // fields out so the decoder borrow ends before we ask it to decode
        // into a buffer we own.
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

        // The palette that applies to *this* frame: its local table if it has
        // one, else the global table. Copied onto the stack (768 B max) so
        // that decoding, which needs the decoder mutably, can be interleaved
        // with compositing, which needs the palette.
        let mut pal = [0u8; MAX_PALETTE];
        let pal_len = {
            let p = decoder.palette().map_err(decode_err)?;
            let n = p.len().min(MAX_PALETTE);
            pal[..n].copy_from_slice(&p[..n]);
            n
        };
        let pal = &pal[..pal_len];

        // The screen borrow — of the canvas or of our own buffer — lives only
        // inside this block, so `blit` can take the canvas again afterwards.
        {
            // In `Direct` mode the canvas's dirty map comes along: what is
            // painted here is marked here. `Owned` needs none, because `blit`
            // rewrites (and marks) the whole canvas every frame.
            let (buf, mut dirty): (&mut [u8], Option<&mut DirtyMap>) = match screen {
                Screen::Direct { cleared } => {
                    if !*cleared {
                        // What the old full-canvas blit from a zeroed screen
                        // did on the first frame. Marks everything.
                        canvas.clear();
                        *cleared = true;
                    }
                    let (b, d) = canvas.bytes_and_dirty_mut();
                    (b, Some(d))
                }
                Screen::Owned(v) => (v.as_mut_slice(), None),
            };

            // Whatever the last frame asked for, before drawing the next one.
            if let Some((method, prev)) = pending.take() {
                match method {
                    DisposalMethod::Background => {
                        mark_changes(buf, None, dirty.as_deref_mut(), prev, width, height);
                        clear_rect(buf, width, prev);
                    }
                    DisposalMethod::Previous if saved.len() == buf.len() => {
                        // Only the last frame's rectangle was painted since
                        // the snapshot, so only it can differ.
                        mark_changes(buf, Some(saved), dirty.as_deref_mut(), prev, width, height);
                        buf.copy_from_slice(saved);
                    }
                    // Any / Keep: leave the canvas alone.
                    _ => {}
                }
            }

            if dispose == DisposalMethod::Previous {
                if saved.len() != buf.len() {
                    saved.resize(buf.len(), 0);
                }
                saved.copy_from_slice(buf);
            }

            let mut comp = Compositor {
                screen: buf,
                dirty,
                width,
                height,
                rect,
                palette: pal,
                transparent,
            };

            let frame_pixels = frame_w.saturating_mul(frame_h);
            if frame_pixels <= indexed.len() {
                // Common case: the whole frame fits the buffer we already own,
                // so let the crate deinterlace it for us.
                decoder
                    .read_into_buffer(&mut indexed[..frame_pixels])
                    .map_err(decode_err)?;
                for y in 0..frame_h {
                    comp.row(y, &indexed[y * frame_w..(y + 1) * frame_w]);
                }
            } else {
                // Frame rectangle larger than the logical screen. Decode it
                // one stored row at a time and throw away what falls outside;
                // the rows must all be consumed or the LZW stream
                // desynchronises.
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
                    comp.row(src_y, &indexed[..frame_w]);
                }
            }
        }

        *pending = Some((dispose, rect));
        *frames_decoded += 1;
        *raw_delay = delay;

        // `Direct` composited into the canvas already; at the panel size
        // centre-and-crop is the identity, so there is nothing to blit.
        if let Screen::Owned(v) = screen {
            blit(v, width, height, canvas);
        }
        Ok(Some(frame_delay(delay)))
    }
}

/// One frame's worth of compositing state: everything `row` needs that does
/// not change between the rows of a frame.
struct Compositor<'a> {
    screen: &'a mut [u8],
    /// The canvas's dirty map when the screen is the canvas (`Direct`).
    dirty: Option<&'a mut DirtyMap>,
    width: usize,
    height: usize,
    rect: Rect,
    palette: &'a [u8],
    transparent: Option<u8>,
}

impl Compositor<'_> {
    /// Paint one decoded row of the frame into the RGB logical screen. `row`
    /// is the frame's full row (`rect.width` indices); pixels that fall
    /// outside the screen, carry the transparent index, or name a palette
    /// entry that does not exist are skipped, leaving the screen as it was.
    ///
    /// In `Direct` mode only pixels whose value actually changes are written
    /// and marked dirty. Most GIFs on the card repaint the full 128x32
    /// rectangle every frame, relying on transparency or on repeating the
    /// same colours; marking the rectangle would make every frame a full
    /// panel update, while the comparison is a few instructions a pixel.
    fn row(&mut self, src_y: usize, row: &[u8]) {
        let dy = self.rect.top + src_y;
        if dy >= self.height {
            return;
        }
        // A run of changed pixels, `run..x`, marked when it ends.
        let mut run: Option<usize> = None;
        let mut end = self.rect.left;
        for (x, &idx) in row.iter().enumerate() {
            let dx = self.rect.left + x;
            if dx >= self.width {
                break;
            }
            end = dx + 1;
            let mut changed = false;
            if self.transparent != Some(idx) {
                let o = idx as usize * 3;
                if let Some(rgb) = self.palette.get(o..o + 3) {
                    let d = (dy * self.width + dx) * 3;
                    let px = &mut self.screen[d..d + 3];
                    if px != rgb {
                        px.copy_from_slice(rgb);
                        changed = true;
                    }
                }
            }
            match (changed, run) {
                (true, None) => run = Some(dx),
                (false, Some(start)) => {
                    if let Some(dirty) = self.dirty.as_deref_mut() {
                        dirty.mark_span(dy, start, dx);
                    }
                    run = None;
                }
                _ => {}
            }
        }
        if let (Some(start), Some(dirty)) = (run, self.dirty.as_deref_mut()) {
            dirty.mark_span(dy, start, end);
        }
    }
}

/// Before a disposal rewrites `rect` of a `Direct` screen (where screen and
/// canvas coordinates coincide), mark the pixels it will change: those that
/// differ from `to`, or from black when `to` is `None`.
pub(crate) fn mark_changes(
    screen: &[u8],
    to: Option<&[u8]>,
    dirty: Option<&mut DirtyMap>,
    rect: Rect,
    width: usize,
    height: usize,
) {
    let Some(dirty) = dirty else { return };
    let x1 = rect.left.saturating_add(rect.width).min(width);
    let y1 = rect.top.saturating_add(rect.height).min(height);
    for y in rect.top..y1 {
        for x in rect.left..x1 {
            let o = (y * width + x) * 3;
            let now = &screen[o..o + 3];
            let after = to.map_or(&[0u8; 3][..], |t| &t[o..o + 3]);
            if now != after {
                dirty.mark(x, y);
            }
        }
    }
}

pub(crate) fn clear_rect(screen: &mut [u8], screen_width: usize, rect: Rect) {
    let height = screen.len() / (screen_width * 3);
    for y in 0..rect.height {
        let dy = rect.top + y;
        if dy >= height {
            break;
        }
        let start = (dy * screen_width + rect.left) * 3;
        let span = rect.width.min(screen_width.saturating_sub(rect.left)) * 3;
        screen[start..start + span].fill(0);
    }
}

/// Centre-and-crop the logical screen onto the 128x32 canvas. Areas the GIF
/// doesn't cover, and its transparent pixels, are black. Every canvas pixel is
/// written, so everything is marked dirty.
pub(crate) fn blit(screen: &[u8], width: usize, height: usize, canvas: &mut Canvas) {
    let off_x = (WIDTH as i32 - width as i32) / 2;
    let off_y = (HEIGHT as i32 - height as i32) / 2;
    for cy in 0..HEIGHT {
        let sy = cy as i32 - off_y;
        for cx in 0..WIDTH {
            let sx = cx as i32 - off_x;
            let px = if sx >= 0 && sy >= 0 && (sx as usize) < width && (sy as usize) < height {
                let o = ((sy as usize) * width + sx as usize) * 3;
                Rgb::new(screen[o], screen[o + 1], screen[o + 2])
            } else {
                Rgb::BLACK
            };
            canvas.set(cx, cy, px);
        }
    }
}

/// The order `fill_buffer` hands rows back in: stored order, which for an
/// interlaced frame is the GIF interlace order. Not an `Iterator` impl so the
/// oversize path never needs a `Box<dyn Iterator>` or a `Vec` of row numbers.
pub(crate) enum RowOrder {
    Sequential(std::ops::Range<usize>),
    Interlaced(InterlaceRows),
}

impl RowOrder {
    pub(crate) fn new(height: usize, interlaced: bool) -> Self {
        if interlaced {
            RowOrder::Interlaced(InterlaceRows::new(height))
        } else {
            RowOrder::Sequential(0..height)
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub(crate) fn next(&mut self) -> Option<usize> {
        match self {
            RowOrder::Sequential(r) => r.next(),
            RowOrder::Interlaced(i) => i.next(),
        }
    }
}

/// The GIF interlace row order (passes of 8, 8, 4, 2 starting at 0, 4, 2, 1).
/// `read_into_buffer` does this for us; the row-chunked oversize path has to
/// do it itself, because `fill_buffer` hands back rows in stored order.
pub(crate) struct InterlaceRows {
    len: usize,
    next: usize,
    pass: usize,
}

impl InterlaceRows {
    fn new(len: usize) -> Self {
        InterlaceRows {
            len,
            next: 0,
            pass: 0,
        }
    }
}

impl Iterator for InterlaceRows {
    type Item = usize;

    fn next(&mut self) -> Option<usize> {
        if self.len == 0 {
            return None;
        }
        let mut next = self.next + *[8usize, 8, 4, 2].get(self.pass)?;
        while next >= self.len {
            next = *[4usize, 2, 1, 0].get(self.pass)?;
            self.pass += 1;
        }
        std::mem::swap(&mut next, &mut self.next);
        Some(next)
    }
}

/// A playable file, chosen by its first four bytes: `GIF8` is a GIF, played
/// by [`GifSlim`](crate::gifslim::GifSlim) — at the cap its `OPENDMD1LZW`
/// hint names (a prepared copy: 9, ~16 KB of heap with the canvas), else
/// at 12 (~27 KB) — and `SYNV` a virtual file drawn by
/// [`SynthSource`](crate::synth::SynthSource) (only ever produced in memory
/// by [`crate::synth::open`]).
///
/// **The fallback.** A GIF gifslim does not decode — interlaced frames,
/// `Previous` disposal (`Unsupported`), or a code wider than the cap
/// (`NeedsFullDecoder`: a hinted file some tool re-encoded) — is played by
/// the `gif` crate's [`GifSource`] (~54 KB) instead: the reader is rewound,
/// gifslim is dropped *before* the crate's decoder is built (never two at
/// once), and the frames already shown are decoded again onto the canvas,
/// so the file carries on from the frame that failed. Why is kept for the
/// caller ([`take_fallback`](Self::take_fallback): the engine logs it once
/// per file) and a rewound loop opens the file with the crate directly
/// ([`open_full`](Self::open_full)).
///
/// No allocation per opened file beyond the decoder's own, and the sniff is
/// what decides, not the extension: the first four bytes are read and the
/// reader is rewound, so every decoder sees the whole file (and the device
/// has one instantiation of each decoder, shared with its cache builder).
pub struct FrameSource<R: Read> {
    dec: Dec<R>,
    fallback: Option<String>,
}

// The GIF variants are ~520 B inline against the others' ~150 B, but
// boxing them would add a heap block per open: `large_enum_variant` is the
// wrong trade here.
#[allow(clippy::large_enum_variant)]
enum Dec<R: Read> {
    Slim(crate::gifslim::GifSlim<R>),
    Gif(GifSource<R>),
    /// A virtual file ([`crate::synth`]): a title card or the power-on
    /// effect, drawn rather than decoded.
    Synth(crate::synth::SynthSource<R>),
    /// A fallback that failed half-way: the reader is gone.
    Gone,
}

impl<R: Read + io::Seek> FrameSource<R> {
    /// Read the first four bytes, rewind, and open the matching decoder over
    /// the whole stream. Anything else — including a file shorter than four bytes — is
    /// `InvalidData`.
    pub fn open(reader: R) -> io::Result<Self> {
        Self::open_with(reader, false)
    }

    /// [`open`](Self::open), but a GIF goes straight to the `gif` crate: a
    /// file that fell back once, looped.
    pub fn open_full(reader: R) -> io::Result<Self> {
        Self::open_with(reader, true)
    }

    fn open_with(mut reader: R, full: bool) -> io::Result<Self> {
        let mut magic = [0u8; 4];
        reader.read_exact(&mut magic).map_err(|e| {
            if e.kind() == io::ErrorKind::UnexpectedEof {
                invalid("file is too short to be a GIF")
            } else {
                e
            }
        })?;
        reader.seek(io::SeekFrom::Start(0))?;
        let stream = reader;
        let dec = match &magic {
            b"GIF8" if full => Dec::Gif(GifSource::new(stream)?),
            b"GIF8" => Dec::Slim(crate::gifslim::GifSlim::new(stream)?),
            m if *m == crate::synth::MAGIC => Dec::Synth(crate::synth::SynthSource::new(stream)?),
            _ => {
                return Err(invalid(format!(
                    "not a GIF file (starts with {magic:02x?})"
                )))
            }
        };
        Ok(FrameSource {
            dec,
            fallback: None,
        })
    }

    /// Give the underlying reader back, dropping the decoder (and with it the
    /// LZW state and buffers) first. Whatever the decoders had buffered is
    /// discarded, so the reader's position is somewhere past the last byte
    /// decoded: seek it before reusing it. This is how the engine loops a
    /// file without opening it again. `None` only after a fallback that
    /// failed (the reader went with it).
    pub fn into_inner(self) -> Option<R> {
        Some(match self.dec {
            Dec::Slim(s) => s.into_inner(),
            Dec::Gif(s) => s.into_inner(),
            Dec::Synth(s) => s.into_inner(),
            Dec::Gone => return None,
        })
    }

    /// The virtual file being drawn, if this is one (a live edit of a
    /// formula on screen).
    #[cfg(feature = "psi")]
    pub fn synth_mut(&mut self) -> Option<&mut crate::synth::SynthSource<R>> {
        match &mut self.dec {
            Dec::Synth(s) => Some(s),
            _ => None,
        }
    }

    /// Loop a virtual file in place ([`SynthSource::rewind`](crate::synth::SynthSource::rewind)):
    /// true if this is one (nothing reopened, nothing allocated), false for
    /// a GIF, which the caller rewinds and reopens.
    pub fn rewind_virtual(&mut self) -> bool {
        match &mut self.dec {
            Dec::Synth(s) => {
                s.rewind();
                true
            }
            _ => false,
        }
    }

    /// `"slim"` (gifslim), `"gif"` (the `gif` crate) or `"synth"`, for logs
    /// and tools.
    pub fn kind(&self) -> &'static str {
        match self.dec {
            Dec::Slim(_) => "slim",
            Dec::Gif(_) => "gif",
            Dec::Synth(_) => "synth",
            Dec::Gone => "gone",
        }
    }

    /// The cap gifslim plays this file at (`None` for the other decoders).
    pub fn cap(&self) -> Option<u8> {
        match &self.dec {
            Dec::Slim(s) => Some(s.cap()),
            _ => None,
        }
    }

    /// Why the last [`next_frame`](Self::next_frame) gave gifslim up for
    /// the `gif` crate, once.
    pub fn take_fallback(&mut self) -> Option<String> {
        self.fallback.take()
    }

    pub fn logical_size(&self) -> (usize, usize) {
        match &self.dec {
            Dec::Slim(s) => s.logical_size(),
            Dec::Gif(s) => s.logical_size(),
            Dec::Synth(s) => s.logical_size(),
            Dec::Gone => (0, 0),
        }
    }

    pub fn frames_decoded(&self) -> u64 {
        match &self.dec {
            Dec::Slim(s) => s.frames_decoded(),
            Dec::Gif(s) => s.frames_decoded(),
            Dec::Synth(s) => s.frames_decoded(),
            Dec::Gone => 0,
        }
    }
}

/// What the `gif` crate's [`GifSource`] allocates for a 128x32 file, all of
/// it before its first frame: its 8 KB `BufReader`, `weezl`'s LZW state
/// (28,784 B, in pieces up to 16 KB), the 4 KB indexed frame buffer (see
/// "GIF heap budget" in CLAUDE.md) and, since `Previous` disposal is one of
/// the reasons a file falls back, the 12 KB RGB snapshot that disposal
/// keeps. What a fallback asks its [`Gate`](crate::compositor::Gate) for,
/// after gifslim's tables are gone. (42 KB, without the snapshot, let
/// `NES_Excitebike_02` take the rig's heap to 1,124 B on 2026-09-24.)
pub const GIF_CRATE_BYTES: usize = 54 * 1024;

/// Whether a [`FrameSource::try_frame`] error is gifslim giving the file up
/// for the `gif` crate: interlaced frames or `Previous` disposal
/// (`Unsupported`), or a code wider than the cap (`NeedsFullDecoder`).
pub fn wants_fallback(e: &io::Error) -> bool {
    crate::gifslim::is_unsupported(e) || crate::gifslim::is_needs_full_decoder(e)
}

impl<R: Read + io::Seek> FrameSource<R> {
    /// See [`GifSource::next_frame`]; the contract is the same for every
    /// decoder. A GIF gifslim cannot finish falls back to the `gif` crate
    /// here (see the type's docs) and this frame comes from it, with no
    /// question asked about heap: [`next_frame_gated`](Self::next_frame_gated)
    /// is what the engine uses.
    pub fn next_frame(&mut self, canvas: &mut Canvas) -> io::Result<Option<Duration>> {
        self.next_frame_gated(canvas, None)
    }

    /// [`next_frame`](Self::next_frame), but a fallback first asks `gate`
    /// whether [`GIF_CRATE_BYTES`] may be allocated (gifslim's tables are
    /// already freed when it asks). Refused, the frame is an error naming
    /// both reasons and the file is gone (`kind()` is `"gone"`,
    /// `into_inner` is `None`): the engine skips it, as it would any file
    /// that fails to decode, instead of the allocation aborting the device.
    /// The firmware's gate is the compositor's, which keeps its heap reserve.
    pub fn next_frame_gated(
        &mut self,
        canvas: &mut Canvas,
        gate: Option<crate::compositor::Gate>,
    ) -> io::Result<Option<Duration>> {
        match self.try_frame(canvas) {
            Err(e) if wants_fallback(&e) => self.fall_back(canvas, e, gate),
            r => r,
        }
    }

    /// One frame from the current decoder, with no fallback: an error for
    /// which [`wants_fallback`] holds is gifslim giving the file up, and
    /// [`fall_back`](Self::fall_back) carries on with the `gif` crate. The
    /// engine takes this route so it can give a running transition's
    /// buffers back before the crate's decoder is built.
    pub fn try_frame(&mut self, canvas: &mut Canvas) -> io::Result<Option<Duration>> {
        match &mut self.dec {
            Dec::Slim(s) => s.next_frame(canvas),
            Dec::Gif(s) => s.next_frame(canvas),
            Dec::Synth(s) => s.next_frame(canvas),
            Dec::Gone => Err(invalid("the file could not be reopened")),
        }
    }

    /// Switch to the `gif` crate after [`try_frame`](Self::try_frame)
    /// returned `why` (a [`wants_fallback`] error) and produce that frame;
    /// see [`next_frame_gated`](Self::next_frame_gated) for `gate`. Anything
    /// but a gifslim decoder here is a caller's mistake and `why` comes back.
    pub fn fall_back(
        &mut self,
        canvas: &mut Canvas,
        why: io::Error,
        gate: Option<crate::compositor::Gate>,
    ) -> io::Result<Option<Duration>> {
        if !matches!(self.dec, Dec::Slim(_)) {
            return Err(why);
        }
        self.fall_back_inner(canvas, why, gate)
    }

    fn fall_back_inner(
        &mut self,
        canvas: &mut Canvas,
        why: io::Error,
        gate: Option<crate::compositor::Gate>,
    ) -> io::Result<Option<Duration>> {
        let Dec::Slim(s) = std::mem::replace(&mut self.dec, Dec::Gone) else {
            unreachable!("checked by fall_back");
        };
        let (done, cap) = (s.frames_decoded(), s.cap());
        // gifslim's tables go before the crate's decoder is built.
        let mut reader = s.into_inner();
        if gate.is_some_and(|allow| !allow(GIF_CRATE_BYTES)) {
            let why = why
                .get_ref()
                .map_or_else(|| why.to_string(), |e| e.to_string());
            return Err(invalid(format!(
                "{why} at frame {done} (gifslim cap {cap}), and the {} KB the gif crate needs is not free",
                GIF_CRATE_BYTES / 1024
            )));
        }
        reader.seek(io::SeekFrom::Start(0))?;
        let mut g = GifSource::new(reader)?;
        for _ in 0..done {
            if g.next_frame(canvas)?.is_none() {
                return Err(invalid("the gif decoder found fewer frames"));
            }
        }
        let delay = g.next_frame(canvas);
        self.dec = Dec::Gif(g);
        let why = why
            .get_ref()
            .map_or_else(|| why.to_string(), |e| e.to_string());
        self.fallback = Some(format!("{why} at frame {done} (gifslim cap {cap})"));
        delay
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gif::{Encoder, Frame, Repeat};
    use std::borrow::Cow;

    const BLACK_I: u8 = 0;
    const RED_I: u8 = 1;
    const GREEN_I: u8 = 2;
    const RED: Rgb = Rgb { r: 255, g: 0, b: 0 };
    const GREEN: Rgb = Rgb { r: 0, g: 255, b: 0 };
    const BLUE: Rgb = Rgb { r: 0, g: 0, b: 255 };

    /// Palette index 0 is the transparent index in every fixture below, so a
    /// frame's "background" really is transparent rather than black.
    fn palette() -> Vec<u8> {
        vec![0, 0, 0, 255, 0, 0, 0, 255, 0]
    }

    struct FrameSpec {
        left: u16,
        top: u16,
        width: u16,
        height: u16,
        pixels: Vec<u8>,
        dispose: DisposalMethod,
        delay: u16,
    }

    fn encode(screen_w: u16, screen_h: u16, specs: Vec<FrameSpec>) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut enc = Encoder::new(&mut out, screen_w, screen_h, &[]).unwrap();
            enc.set_repeat(Repeat::Infinite).unwrap();
            for spec in specs {
                let mut f = Frame {
                    left: spec.left,
                    top: spec.top,
                    width: spec.width,
                    height: spec.height,
                    buffer: Cow::Owned(spec.pixels),
                    dispose: spec.dispose,
                    delay: spec.delay,
                    ..Default::default()
                };
                f.palette = Some(palette());
                f.transparent = Some(BLACK_I);
                enc.write_frame(&f).unwrap();
            }
        }
        out
    }

    /// One frame covering the whole logical screen, `on` pixels in `color`.
    fn full_frame(
        w: u16,
        h: u16,
        color: u8,
        on: &[(u16, u16)],
        dispose: DisposalMethod,
        delay: u16,
    ) -> FrameSpec {
        let mut pixels = vec![BLACK_I; w as usize * h as usize];
        for (x, y) in on {
            pixels[*y as usize * w as usize + *x as usize] = color;
        }
        FrameSpec {
            left: 0,
            top: 0,
            width: w,
            height: h,
            pixels,
            dispose,
            delay,
        }
    }

    fn source(bytes: Vec<u8>) -> GifSource<std::io::Cursor<Vec<u8>>> {
        GifSource::new(std::io::Cursor::new(bytes)).unwrap()
    }

    /// A frame spec that carries its own (optional) local palette and
    /// transparent index, for the global-vs-local tests below. Kept separate
    /// from `FrameSpec` so the fixtures above stay as they were.
    struct PalFrameSpec {
        left: u16,
        top: u16,
        width: u16,
        height: u16,
        pixels: Vec<u8>,
        dispose: DisposalMethod,
        palette: Option<Vec<u8>>,
        transparent: Option<u8>,
    }

    fn encode_with_global(
        screen_w: u16,
        screen_h: u16,
        global: &[u8],
        specs: Vec<PalFrameSpec>,
    ) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut enc = Encoder::new(&mut out, screen_w, screen_h, global).unwrap();
            for spec in specs {
                let f = Frame {
                    left: spec.left,
                    top: spec.top,
                    width: spec.width,
                    height: spec.height,
                    buffer: Cow::Owned(spec.pixels),
                    dispose: spec.dispose,
                    delay: 10,
                    palette: spec.palette,
                    transparent: spec.transparent,
                    ..Default::default()
                };
                enc.write_frame(&f).unwrap();
            }
        }
        out
    }

    #[test]
    fn background_disposal_and_transparency() {
        // Frame 0: red at (1,1), full-screen rect, disposed to background.
        // Frame 1: a 2x2 rect at (10,10) with one green pixel and one
        //          transparent pixel.
        let bytes = encode(
            128,
            32,
            vec![
                full_frame(128, 32, RED_I, &[(1, 1)], DisposalMethod::Background, 10),
                FrameSpec {
                    left: 10,
                    top: 10,
                    width: 2,
                    height: 1,
                    pixels: vec![GREEN_I, BLACK_I],
                    dispose: DisposalMethod::Keep,
                    delay: 10,
                },
            ],
        );
        let mut gif = source(bytes);
        assert_eq!(gif.logical_size(), (128, 32));
        let mut canvas = Canvas::new();

        let d = gif.next_frame(&mut canvas).unwrap().unwrap();
        assert_eq!(d, Duration::from_millis(100));
        assert_eq!(canvas.get(1, 1), RED);
        assert_eq!(canvas.get(0, 0), Rgb::BLACK);
        assert_eq!(canvas.get(10, 10), Rgb::BLACK);

        gif.next_frame(&mut canvas).unwrap().unwrap();
        // Frame 0's whole rect was disposed to background, so the red is gone.
        assert_eq!(canvas.get(1, 1), Rgb::BLACK);
        assert_eq!(canvas.get(10, 10), GREEN);
        // The transparent pixel of frame 1 left the (cleared) screen showing.
        assert_eq!(canvas.get(11, 10), Rgb::BLACK);

        assert!(gif.next_frame(&mut canvas).unwrap().is_none());
        assert_eq!(gif.frames_decoded(), 2);
    }

    #[test]
    fn keep_disposal_accumulates() {
        let bytes = encode(
            128,
            32,
            vec![
                full_frame(128, 32, RED_I, &[(1, 1)], DisposalMethod::Keep, 10),
                FrameSpec {
                    left: 10,
                    top: 10,
                    width: 1,
                    height: 1,
                    pixels: vec![GREEN_I],
                    dispose: DisposalMethod::Keep,
                    delay: 10,
                },
            ],
        );
        let mut gif = source(bytes);
        let mut canvas = Canvas::new();
        gif.next_frame(&mut canvas).unwrap();
        gif.next_frame(&mut canvas).unwrap();
        assert_eq!(canvas.get(1, 1), RED);
        assert_eq!(canvas.get(10, 10), GREEN);
    }

    #[test]
    fn previous_disposal_restores_the_snapshot() {
        let bytes = encode(
            128,
            32,
            vec![
                full_frame(128, 32, RED_I, &[(1, 1)], DisposalMethod::Keep, 10),
                // Drawn, then undone.
                FrameSpec {
                    left: 10,
                    top: 10,
                    width: 1,
                    height: 1,
                    pixels: vec![GREEN_I],
                    dispose: DisposalMethod::Previous,
                    delay: 10,
                },
                FrameSpec {
                    left: 20,
                    top: 20,
                    width: 1,
                    height: 1,
                    pixels: vec![GREEN_I],
                    dispose: DisposalMethod::Keep,
                    delay: 10,
                },
            ],
        );
        let mut gif = source(bytes);
        let mut canvas = Canvas::new();
        gif.next_frame(&mut canvas).unwrap();
        gif.next_frame(&mut canvas).unwrap();
        assert_eq!(canvas.get(10, 10), GREEN);
        gif.next_frame(&mut canvas).unwrap();
        assert_eq!(canvas.get(10, 10), Rgb::BLACK, "frame 1 should be undone");
        assert_eq!(canvas.get(1, 1), RED, "frame 0 should survive");
        assert_eq!(canvas.get(20, 20), GREEN);
    }

    #[test]
    fn small_gif_is_centred() {
        // 10x5 -> offset (128-10)/2 = 59, (32-5)/2 = 13.
        let bytes = encode(
            10,
            5,
            vec![full_frame(
                10,
                5,
                RED_I,
                &[(0, 0), (9, 4)],
                DisposalMethod::Keep,
                10,
            )],
        );
        let mut gif = source(bytes);
        assert_eq!(gif.logical_size(), (10, 5));
        let mut canvas = Canvas::new();
        gif.next_frame(&mut canvas).unwrap();
        assert_eq!(canvas.get(59, 13), RED);
        assert_eq!(canvas.get(68, 17), RED);
        assert_eq!(canvas.get(58, 13), Rgb::BLACK);
        assert_eq!(canvas.get(0, 0), Rgb::BLACK);
    }

    #[test]
    fn oversized_gif_is_cropped_not_scaled() {
        // 135x32 -> offset (128-135)/2 = -3, so source x=3 lands at canvas x=0.
        let on: Vec<(u16, u16)> = vec![(0, 0), (3, 0), (4, 1), (130, 2), (134, 3)];
        let bytes = encode(
            135,
            32,
            vec![full_frame(135, 32, RED_I, &on, DisposalMethod::Keep, 10)],
        );
        let mut gif = source(bytes);
        assert_eq!(gif.logical_size(), (135, 32));
        let mut canvas = Canvas::new();
        gif.next_frame(&mut canvas).unwrap();
        assert_eq!(canvas.get(0, 0), RED, "source x=3 -> canvas x=0");
        assert_eq!(canvas.get(1, 1), RED, "source x=4 -> canvas x=1");
        assert_eq!(canvas.get(127, 2), RED, "source x=130 -> canvas x=127");
        // Source x=0 was cropped off the left, x=134 off the right.
        assert_eq!(canvas.get(3, 3), Rgb::BLACK);
        assert!((0..128).all(|x| canvas.get(x, 3) == Rgb::BLACK));
    }

    #[test]
    fn frame_delays_are_normalized() {
        let bytes = encode(
            128,
            32,
            vec![
                full_frame(128, 32, RED_I, &[(0, 0)], DisposalMethod::Keep, 0),
                full_frame(128, 32, RED_I, &[(0, 0)], DisposalMethod::Keep, 1),
                full_frame(128, 32, RED_I, &[(0, 0)], DisposalMethod::Keep, 7),
            ],
        );
        let mut gif = source(bytes);
        let mut canvas = Canvas::new();
        // 0 -> 100 ms, 1 (=10 ms) -> clamped to 20 ms, 7 -> 70 ms.
        assert_eq!(
            gif.next_frame(&mut canvas).unwrap(),
            Some(Duration::from_millis(100))
        );
        assert_eq!(
            gif.next_frame(&mut canvas).unwrap(),
            Some(Duration::from_millis(20))
        );
        assert_eq!(
            gif.next_frame(&mut canvas).unwrap(),
            Some(Duration::from_millis(70))
        );
    }

    #[test]
    fn garbage_is_an_error_not_a_panic() {
        assert!(GifSource::new(std::io::Cursor::new(b"not a gif at all".to_vec())).is_err());
    }

    #[test]
    fn panel_sized_gif_composites_into_a_dirty_canvas() {
        // At exactly WIDTH x HEIGHT the canvas *is* the logical screen, so
        // whatever the caller left on it must not survive the first frame.
        let bytes = encode(
            128,
            32,
            vec![full_frame(
                128,
                32,
                RED_I,
                &[(1, 1)],
                DisposalMethod::Keep,
                10,
            )],
        );
        let mut gif = source(bytes);
        let mut canvas = Canvas::new();
        canvas.fill(GREEN);
        gif.next_frame(&mut canvas).unwrap().unwrap();
        assert_eq!(canvas.get(1, 1), RED);
        assert_eq!(canvas.get(0, 0), Rgb::BLACK, "stale canvas must be cleared");
        assert_eq!(canvas.get(127, 31), Rgb::BLACK);
    }

    #[test]
    fn end_of_stream_leaves_the_canvas_alone() {
        // Including in the direct-to-canvas case, where the screen and the
        // canvas are the same bytes and a pending disposal is still owed.
        let bytes = encode(
            128,
            32,
            vec![full_frame(
                128,
                32,
                RED_I,
                &[(1, 1)],
                DisposalMethod::Background,
                10,
            )],
        );
        let mut gif = source(bytes);
        let mut canvas = Canvas::new();
        gif.next_frame(&mut canvas).unwrap().unwrap();
        assert_eq!(canvas.get(1, 1), RED);
        assert!(gif.next_frame(&mut canvas).unwrap().is_none());
        assert_eq!(
            canvas.get(1, 1),
            RED,
            "the owed Background disposal must not be applied at end of stream"
        );
    }

    #[test]
    fn local_palette_overrides_the_global_one() {
        // Global: 0 black, 1 red, 2 green. Local on frame 1: 1 is blue.
        let global = vec![0, 0, 0, 255, 0, 0, 0, 255, 0];
        let local = vec![0, 0, 0, 0, 0, 255, 0, 255, 0];
        let bytes = encode_with_global(
            4,
            2,
            &global,
            vec![
                PalFrameSpec {
                    left: 0,
                    top: 0,
                    width: 4,
                    height: 2,
                    pixels: vec![
                        RED_I, GREEN_I, BLACK_I, BLACK_I, BLACK_I, BLACK_I, BLACK_I, BLACK_I,
                    ],
                    dispose: DisposalMethod::Keep,
                    palette: None,
                    transparent: None,
                },
                PalFrameSpec {
                    left: 0,
                    top: 1,
                    width: 1,
                    height: 1,
                    pixels: vec![RED_I],
                    dispose: DisposalMethod::Keep,
                    palette: Some(local),
                    transparent: None,
                },
            ],
        );
        let mut gif = source(bytes);
        assert_eq!(gif.logical_size(), (4, 2));
        let mut canvas = Canvas::new();
        // 4x2 -> offsets (128-4)/2 = 62, (32-2)/2 = 15.
        gif.next_frame(&mut canvas).unwrap().unwrap();
        assert_eq!(canvas.get(62, 15), RED, "global palette index 1 is red");
        assert_eq!(canvas.get(63, 15), GREEN);
        gif.next_frame(&mut canvas).unwrap().unwrap();
        assert_eq!(
            canvas.get(62, 16),
            BLUE,
            "the local palette must win over the global one"
        );
        // The first frame, drawn from the global palette, is untouched.
        assert_eq!(canvas.get(62, 15), RED);
    }

    #[test]
    fn transparent_index_in_a_local_palette_is_honoured() {
        // Global 1 = red. Frame 1 has a local palette whose index 1 is green
        // and is *also* the transparent index, so it must paint nothing.
        let global = vec![0, 0, 0, 255, 0, 0];
        let local = vec![0, 0, 255, 0, 255, 0];
        let bytes = encode_with_global(
            2,
            1,
            &global,
            vec![
                PalFrameSpec {
                    left: 0,
                    top: 0,
                    width: 2,
                    height: 1,
                    pixels: vec![RED_I, RED_I],
                    dispose: DisposalMethod::Keep,
                    palette: None,
                    transparent: None,
                },
                PalFrameSpec {
                    left: 0,
                    top: 0,
                    width: 2,
                    height: 1,
                    pixels: vec![RED_I, BLACK_I],
                    dispose: DisposalMethod::Keep,
                    palette: Some(local),
                    transparent: Some(RED_I),
                },
            ],
        );
        let mut gif = source(bytes);
        let mut canvas = Canvas::new();
        // 2x1 -> offsets (128-2)/2 = 63, (32-1)/2 = 15.
        gif.next_frame(&mut canvas).unwrap().unwrap();
        assert_eq!(canvas.get(63, 15), RED);
        assert_eq!(canvas.get(64, 15), RED);
        gif.next_frame(&mut canvas).unwrap().unwrap();
        assert_eq!(canvas.get(63, 15), RED, "transparent index paints nothing");
        assert_eq!(canvas.get(64, 15), BLUE, "local index 0 is blue");
    }

    #[test]
    fn frame_rect_larger_than_the_logical_screen_is_clipped() {
        // 4x4 logical screen, 6x6 frame rectangle: 36 indices is more than the
        // 16-byte buffer holds, so this goes down the row-chunked path.
        let global = vec![0, 0, 0, 255, 0, 0, 0, 255, 0];
        let mut pixels = vec![BLACK_I; 36];
        pixels[0] = RED_I; //  (0,0) -> visible
        pixels[3] = GREEN_I; // (3,0) -> visible, last column
        pixels[4] = RED_I; //  (4,0) -> off the right edge
        pixels[6 * 3 + 1] = GREEN_I; // (1,3) -> visible, last row
        pixels[6 * 4 + 1] = RED_I; // (1,4) -> off the bottom
        let bytes = encode_with_global(
            4,
            4,
            &global,
            vec![PalFrameSpec {
                left: 0,
                top: 0,
                width: 6,
                height: 6,
                pixels,
                dispose: DisposalMethod::Keep,
                palette: None,
                transparent: Some(BLACK_I),
            }],
        );
        let mut gif = source(bytes);
        assert_eq!(gif.logical_size(), (4, 4));
        let mut canvas = Canvas::new();
        // 4x4 -> offsets (128-4)/2 = 62, (32-4)/2 = 14.
        assert!(gif.next_frame(&mut canvas).unwrap().is_some());
        assert_eq!(canvas.get(62, 14), RED);
        assert_eq!(canvas.get(65, 14), GREEN, "last in-screen column");
        assert_eq!(canvas.get(63, 17), GREEN, "last in-screen row");
        assert_eq!(canvas.get(66, 14), Rgb::BLACK, "clipped off the right");
        assert_eq!(canvas.get(63, 18), Rgb::BLACK, "clipped off the bottom");
        // The stream stayed in sync: the trailer is reached cleanly.
        assert!(gif.next_frame(&mut canvas).unwrap().is_none());
        assert_eq!(gif.frames_decoded(), 1);
    }

    #[test]
    fn interlace_row_order_matches_the_spec() {
        assert_eq!(
            InterlaceRows::new(0).collect::<Vec<_>>(),
            Vec::<usize>::new()
        );
        assert_eq!(InterlaceRows::new(1).collect::<Vec<_>>(), vec![0]);
        assert_eq!(
            InterlaceRows::new(5).collect::<Vec<_>>(),
            vec![0, 4, 2, 1, 3]
        );
        assert_eq!(
            InterlaceRows::new(9).collect::<Vec<_>>(),
            vec![0, 8, 4, 2, 6, 1, 3, 5, 7]
        );
        assert_eq!(
            InterlaceRows::new(17).collect::<Vec<_>>(),
            vec![0, 8, 16, 4, 12, 2, 6, 10, 14, 1, 3, 5, 7, 9, 11, 13, 15]
        );
    }

    /// Frames of a noisy 128x32 animation, whole-screen, a local palette of
    /// 64 colours each, the given disposal and interlacing per frame.
    fn noisy_gif(frames: &[(DisposalMethod, bool)]) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut enc = Encoder::new(&mut out, 128, 32, &[]).unwrap();
            enc.set_repeat(Repeat::Infinite).unwrap();
            let mut x = 0x9E37_79B9u32;
            for (k, &(dispose, interlaced)) in frames.iter().enumerate() {
                let pixels: Vec<u8> = (0..4096)
                    .map(|_| {
                        x ^= x << 13;
                        x ^= x >> 17;
                        x ^= x << 5;
                        (x % 64) as u8
                    })
                    .collect();
                let pal: Vec<u8> = (0..64u32)
                    .flat_map(|i| {
                        [
                            (i * 4) as u8,
                            (k as u32 * 40 + i) as u8,
                            (255 - i * 3) as u8,
                        ]
                    })
                    .collect();
                let f = Frame {
                    width: 128,
                    height: 32,
                    buffer: Cow::Owned(pixels),
                    palette: Some(pal),
                    transparent: Some(0),
                    dispose,
                    interlaced,
                    delay: 4,
                    ..Default::default()
                };
                enc.write_frame(&f).unwrap();
            }
        }
        out
    }

    #[test]
    fn a_refused_fallback_gives_the_file_up_instead_of_allocating() {
        // An interlaced first frame: gifslim gives up before any pixel.
        let bytes = noisy_gif(&[(DisposalMethod::Keep, true)]);
        let mut f = FrameSource::open(std::io::Cursor::new(bytes)).unwrap();
        let mut canvas = Canvas::new();
        let err = f
            .next_frame_gated(&mut canvas, Some(|_| false))
            .unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
        assert!(
            err.to_string().contains("gif crate needs is not free"),
            "{err}"
        );
        assert_eq!(f.kind(), "gone");
        assert!(f.take_fallback().is_none());
        assert!(f.into_inner().is_none());

        // The same file with a gate that agrees plays as the crate does.
        let bytes = noisy_gif(&[(DisposalMethod::Keep, true)]);
        let mut f = FrameSource::open(std::io::Cursor::new(bytes.clone())).unwrap();
        let mut g = GifSource::new(&bytes[..]).unwrap();
        let (mut cf, mut cg) = (Canvas::new(), Canvas::new());
        let allow: crate::compositor::Gate = |need| need == GIF_CRATE_BYTES;
        assert_eq!(
            f.next_frame_gated(&mut cf, Some(allow)).unwrap(),
            g.next_frame(&mut cg).unwrap()
        );
        assert_eq!(f.kind(), "gif");
        assert_eq!(cf.as_bytes(), cg.as_bytes());
    }

    /// Play `bytes` through FrameSource and GifSource side by side: every
    /// canvas and delay equal, every changed pixel marked. Returns the
    /// FrameSource's kind at the end and its fallback note.
    fn plays_like_the_crate(bytes: &[u8]) -> (&'static str, Option<String>) {
        let mut f = FrameSource::open(std::io::Cursor::new(bytes.to_vec())).unwrap();
        assert_eq!(f.kind(), "slim");
        let mut g = GifSource::new(bytes).unwrap();
        let (mut cf, mut cg) = (Canvas::new(), Canvas::new());
        let mut note = None;
        loop {
            let before = cf.as_bytes().to_vec();
            cf.dirty_mut().clear();
            let df = f.next_frame(&mut cf).unwrap();
            note = note.or(f.take_fallback());
            assert_eq!(df, g.next_frame(&mut cg).unwrap());
            if df.is_none() {
                break;
            }
            assert_eq!(cf.as_bytes(), cg.as_bytes());
            for y in 0..HEIGHT {
                for x in 0..WIDTH {
                    let o = (y * WIDTH + x) * 3;
                    if before[o..o + 3] != cf.as_bytes()[o..o + 3] {
                        assert!(cf.dirty().is_dirty(x, y));
                    }
                }
            }
        }
        assert!(f.take_fallback().is_none(), "the note is handed out once");
        (f.kind(), note)
    }

    #[test]
    fn a_plain_gif_plays_through_gifslim() {
        let g = noisy_gif(&[(DisposalMethod::Keep, false); 4]);
        assert_eq!(plays_like_the_crate(&g), ("slim", None));
    }

    #[test]
    fn interlacing_and_previous_fall_back_to_the_gif_crate_mid_file() {
        let g = noisy_gif(&[
            (DisposalMethod::Keep, false),
            (DisposalMethod::Keep, false),
            (DisposalMethod::Keep, true),
            (DisposalMethod::Keep, false),
        ]);
        let (kind, note) = plays_like_the_crate(&g);
        assert_eq!(kind, "gif");
        assert!(note.unwrap().contains("at frame 2"));
        let g = noisy_gif(&[
            (DisposalMethod::Keep, false),
            (DisposalMethod::Previous, false),
            (DisposalMethod::Keep, false),
        ]);
        let (kind, note) = plays_like_the_crate(&g);
        assert_eq!(kind, "gif");
        assert!(note.unwrap().contains("Previous"));
    }

    #[test]
    fn a_hint_that_lies_falls_back_on_the_first_wide_code() {
        // Recoded at cap 12 (codes up to 12 bits), then the hint rewritten
        // to claim 9 — what a tool that keeps the block but re-encodes does.
        let orig = noisy_gif(&[(DisposalMethod::Keep, false); 3]);
        let mut re = Vec::new();
        let opts = crate::gifslim::RecodeOptions {
            cap: 12,
            ..crate::gifslim::PREPARED
        };
        crate::gifslim::recode_gif(&orig[..], &mut re, &opts).unwrap();
        let at = re
            .windows(11)
            .position(|w| w == crate::gifslim::HINT_APP_ID)
            .unwrap();
        assert_eq!(re[at + 12..at + 14], [1, 12]);
        re[at + 13] = 9;
        assert_eq!(
            FrameSource::open(std::io::Cursor::new(re.clone()))
                .unwrap()
                .cap(),
            Some(9)
        );
        let (kind, note) = plays_like_the_crate(&re);
        assert_eq!(kind, "gif");
        assert!(note.unwrap().contains("needs the full decoder"));
    }
}

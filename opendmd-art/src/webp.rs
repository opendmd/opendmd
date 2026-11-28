//! An animated WebP, frame by frame, composited as a browser shows it
//! (feature `webp`: the site's WebP reader links it; the web app's own
//! wasm does not).
//!
//! The container is read here (RIFF, `VP8X`, `ANIM`, `ANMF`); each frame's
//! bitstream (`VP8L`, `VP8 `, or `ALPH` + `VP8 `) is decoded on its own by
//! the pure-Rust `image-webp` and composited here, as the WebP container
//! specification says and as Chrome, Firefox and Safari show it: the
//! canvas starts transparent (the `ANIM` background colour is a hint every
//! browser ignores), a frame marked "dispose" has its rectangle cleared to
//! transparent before the next frame is drawn, and a frame is either
//! alpha-blended over the canvas (non-premultiplied "over") or copied onto
//! it, see-through pixels and all. (`image-webp`'s own animation reader
//! disposes to nothing unless told a colour, and mis-clears behind an
//! opaque frame smaller than the canvas, hence the compositing here.)
//!
//! [`Webp::rgb`] is the canvas as the panel would show it: on black.
//!
//! Memory: the file's bytes, the canvas (4 bytes a pixel), one decoded
//! frame at a time. The canvas is at most [`MAX_PIXELS`] (the player's
//! GIF limit, 2048 x 2048), so a hostile header cannot ask for gigabytes.

use std::io::Cursor;

/// The largest canvas read, in pixels (the core's `player::MAX_LOGICAL_PIXELS`).
pub const MAX_PIXELS: usize = 2048 * 2048;

/// What went wrong, in words a page can show after "That WebP could not be read: ".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(pub String);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

fn err<T>(why: &str) -> Result<T, Error> {
    Err(Error(why.into()))
}

/// Is this the start of an animated WebP (RIFF, WEBP, a `VP8X` chunk with
/// its animation flag)? 21 bytes are enough.
pub fn is_animated(head: &[u8]) -> bool {
    head.len() >= 21
        && &head[..4] == b"RIFF"
        && &head[8..12] == b"WEBP"
        && &head[12..16] == b"VP8X"
        && head[20] & 0x02 != 0
}

struct Frame {
    x: usize,
    y: usize,
    w: usize,
    h: usize,
    ms: u32,
    blend: bool,
    dispose: bool,
    /// The frame's bitstream chunks, a byte range of the file.
    data: (usize, usize),
    /// Whether they start with an `ALPH` chunk.
    alph: bool,
}

/// An animated WebP being read: [`next_frame`](Self::next_frame) composites the next
/// frame onto the canvas.
pub struct Webp {
    bytes: Vec<u8>,
    pub width: usize,
    pub height: usize,
    /// From the `ANIM` chunk: 0 = forever, else how many times it plays.
    pub plays: u32,
    frames: Vec<Frame>,
    next: usize,
    canvas: Vec<u8>,
    /// The last frame's rectangle, when it is to be cleared before the next.
    clear: Option<(usize, usize, usize, usize)>,
}

fn le(b: &[u8], i: usize, n: usize) -> usize {
    (0..n).rev().fold(0, |v, j| v << 8 | b[i + j] as usize)
}

/// The chunks in `b[from..to]`: (fourcc, payload start, payload length).
fn chunks(b: &[u8], from: usize, to: usize) -> Result<Vec<([u8; 4], usize, usize)>, Error> {
    let mut out = Vec::new();
    let mut i = from;
    while i + 8 <= to {
        let n = le(b, i + 4, 4);
        let tag = [b[i], b[i + 1], b[i + 2], b[i + 3]];
        if n > to - i - 8 {
            // (A truncated last chunk: what came before it is kept.)
            if tag == *b"ANMF"
                && out
                    .iter()
                    .any(|c: &([u8; 4], usize, usize)| c.0 == *b"ANMF")
            {
                break;
            }
            return err("the file is cut short");
        }
        out.push((tag, i + 8, n));
        i += 8 + n + (n & 1);
    }
    Ok(out)
}

impl Webp {
    /// Read the container: its size, plays and frames (none decoded yet).
    pub fn open(bytes: Vec<u8>) -> Result<Webp, Error> {
        if bytes.len() < 30 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WEBP" {
            return err("it isn’t a WebP");
        }
        let end = (le(&bytes, 4, 4) + 8).min(bytes.len());
        let all = chunks(&bytes, 12, end)?;
        let Some(&(_, x0, _)) = all.first().filter(|c| c.0 == *b"VP8X" && c.2 >= 10) else {
            return err("it isn’t an animated WebP");
        };
        if bytes[x0] & 0x02 == 0 {
            return err("it isn’t an animated WebP");
        }
        let (width, height) = (1 + le(&bytes, x0 + 4, 3), 1 + le(&bytes, x0 + 7, 3));
        if width * height > MAX_PIXELS {
            return Err(Error(format!(
                "it is {width} × {height}, more than the 2048 × 2048 pixels’ worth the Studio reads"
            )));
        }
        let plays = all
            .iter()
            .find(|c| c.0 == *b"ANIM" && c.2 >= 6)
            .map_or(0, |&(_, i, _)| le(&bytes, i + 4, 2) as u32);
        let mut frames = Vec::new();
        for &(tag, i, n) in &all {
            if tag != *b"ANMF" {
                continue;
            }
            if n < 16 {
                return err("a frame is damaged");
            }
            let f = Frame {
                x: 2 * le(&bytes, i, 3),
                y: 2 * le(&bytes, i + 3, 3),
                w: 1 + le(&bytes, i + 6, 3),
                h: 1 + le(&bytes, i + 9, 3),
                ms: le(&bytes, i + 12, 3) as u32,
                blend: bytes[i + 15] & 0x02 == 0,
                dispose: bytes[i + 15] & 0x01 != 0,
                data: (i + 16, i + n),
                alph: false,
            };
            if f.x + f.w > width || f.y + f.h > height {
                return err("a frame lies outside the picture");
            }
            let subs = chunks(&bytes, f.data.0, f.data.1)?;
            let alph = subs.first().is_some_and(|c| c.0 == *b"ALPH");
            frames.push(Frame { alph, ..f });
        }
        if frames.is_empty() {
            return err("it has no frames");
        }
        Ok(Webp {
            bytes,
            width,
            height,
            plays,
            frames,
            next: 0,
            canvas: vec![0; width * height * 4],
            clear: None,
        })
    }

    /// Frames in the file.
    pub fn len(&self) -> usize {
        self.frames.len()
    }

    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    /// Frames composited so far.
    pub fn done(&self) -> usize {
        self.next
    }

    /// Composite the next frame; its duration in ms as the file says it
    /// (0 included: see [`shown_ms`]). `None` after the last.
    pub fn next_frame(&mut self) -> Option<Result<u32, Error>> {
        let f = self.frames.get(self.next)?;
        let px = match decode(&self.bytes, f) {
            Ok(px) => px,
            Err(e) => return Some(Err(e)),
        };
        let (cw, f) = (self.width, &self.frames[self.next]);
        if let Some((x0, y0, w, h)) = self.clear.take() {
            for y in y0..y0 + h {
                self.canvas[(y * cw + x0) * 4..(y * cw + x0 + w) * 4].fill(0);
            }
        }
        for y in 0..f.h {
            let row = &px[y * f.w * 4..(y + 1) * f.w * 4];
            let to = &mut self.canvas[((f.y + y) * cw + f.x) * 4..((f.y + y) * cw + f.x + f.w) * 4];
            if f.blend {
                for (s, d) in row.as_chunks::<4>().0.iter().zip(to.as_chunks_mut::<4>().0) {
                    over(s, d);
                }
            } else {
                to.copy_from_slice(row);
            }
        }
        if f.dispose {
            self.clear = Some((f.x, f.y, f.w, f.h));
        }
        self.next += 1;
        Some(Ok(f.ms))
    }

    /// The canvas, RGBA (not premultiplied).
    pub fn rgba(&self) -> &[u8] {
        &self.canvas
    }

    /// The canvas on black, as the panel shows it: RGB.
    pub fn rgb(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.width * self.height * 3);
        for p in self.canvas.as_chunks::<4>().0 {
            let a = p[3] as u32;
            out.extend((0..3).map(|i| ((p[i] as u32 * a + 127) / 255) as u8));
        }
        out
    }
}

/// How long a frame shows, as browsers and the player show it: 10 ms or
/// less is 100 ms (as for a GIF), anything else at least the player's 20.
pub fn shown_ms(ms: u32) -> u32 {
    if ms <= 10 {
        100
    } else {
        ms.max(crate::pipeline::MIN_DELAY_MS)
    }
}

/// Non-premultiplied "over": `s` onto `d`, in place.
fn over(s: &[u8; 4], d: &mut [u8; 4]) {
    let sa = s[3] as u32;
    if sa == 255 {
        d.copy_from_slice(s);
        return;
    }
    if sa == 0 {
        return;
    }
    // In 255ths: a = sa + da (1 - sa); c = (s sa + d da (1 - sa)) / a.
    let dw = d[3] as u32 * (255 - sa); // (d's weight, x 255)
    let a = sa * 255 + dw; // (x 255)
    for i in 0..3 {
        d[i] = ((s[i] as u32 * sa * 255 + d[i] as u32 * dw + a / 2) / a) as u8;
    }
    d[3] = ((a + 127) / 255) as u8;
}

/// One frame's pixels, RGBA: its bitstream made a WebP of its own and
/// decoded by `image-webp`.
fn decode(b: &[u8], f: &Frame) -> Result<Vec<u8>, Error> {
    let body = &b[f.data.0..f.data.1];
    let mut file = Vec::with_capacity(body.len() + 30);
    let chunk = |file: &mut Vec<u8>, tag: &[u8], data: &[u8]| {
        file.extend_from_slice(tag);
        file.extend_from_slice(&(data.len() as u32).to_le_bytes());
        file.extend_from_slice(data);
    };
    file.extend_from_slice(b"RIFF\0\0\0\0WEBP");
    if f.alph {
        // An ALPH chunk needs a VP8X saying "alpha" before it.
        let mut x = vec![0x10, 0, 0, 0];
        x.extend_from_slice(&((f.w - 1) as u32).to_le_bytes()[..3]);
        x.extend_from_slice(&((f.h - 1) as u32).to_le_bytes()[..3]);
        chunk(&mut file, b"VP8X", &x);
    }
    // Only the bitstream chunks (an unknown one in a frame is skipped, as the spec says).
    for (tag, i, n) in chunks(b, f.data.0, f.data.1)? {
        if matches!(&tag, b"ALPH" | b"VP8 " | b"VP8L") {
            chunk(&mut file, &tag, &b[i..i + n]);
            if n & 1 == 1 {
                file.push(0);
            }
        }
    }
    let size = (file.len() - 8) as u32;
    file[4..8].copy_from_slice(&size.to_le_bytes());
    let bad = |_| Error("a frame is damaged".into());
    let mut d = image_webp::WebPDecoder::new(Cursor::new(file)).map_err(bad)?;
    if d.dimensions() != (f.w as u32, f.h as u32) {
        return err("a frame is damaged");
    }
    let n = f.w * f.h;
    if d.has_alpha() {
        let mut px = vec![0; n * 4];
        d.read_image(&mut px).map_err(bad)?;
        Ok(px)
    } else {
        let mut rgb = vec![0; n * 3];
        d.read_image(&mut rgb).map_err(bad)?;
        Ok(rgb
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect())
    }
}

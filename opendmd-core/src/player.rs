//! Streaming GIF playback onto the 128x32 canvas.
//!
//! One frame is decoded at a time; the only persistent buffers are the logical
//! screen (RGBA) and, when a `Previous` disposal actually occurs, one snapshot
//! of it. A 4 MB GIF therefore costs ~`w*h*4` bytes, not 4 MB.
//!
//! Mismatched sizes are centred and cropped, never resampled — the archive is
//! pixel art and every scaling filter ruins it. With an odd difference the
//! extra pixel goes to the right/bottom.

use crate::canvas::{Canvas, Rgb, HEIGHT, WIDTH};
use gif::{ColorOutput, DecodeOptions, DisposalMethod};
use std::io::{self, Read};
use std::time::Duration;

/// GIFs with a delay of 0 are "as fast as possible"; browsers and the stock
/// firmware both settle on 100 ms.
const ZERO_DELAY_MS: u64 = 100;
/// Floor for everything else — 50 fps is well past what a HUB75 refresh can do.
const MIN_DELAY_MS: u64 = 20;
/// Sanity cap on the declared logical screen, to bound the RGBA buffer.
const MAX_LOGICAL_PIXELS: usize = 2048 * 2048;

fn decode_err(e: gif::DecodingError) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, e)
}

/// A GIF being played. Construct one per playback; there is no rewind (the
/// reader is consumed), so looping means re-opening the file.
pub struct GifSource<R: Read> {
    decoder: gif::Decoder<R>,
    width: usize,
    height: usize,
    /// Logical screen, RGBA, `width * height * 4`.
    screen: Vec<u8>,
    /// Snapshot for `DisposalMethod::Previous`; allocated on first use.
    saved: Vec<u8>,
    /// Disposal owed by the frame already on screen, with its rectangle.
    pending: Option<(DisposalMethod, Rect)>,
    frames_decoded: u64,
}

#[derive(Clone, Copy, Debug)]
struct Rect {
    left: usize,
    top: usize,
    width: usize,
    height: usize,
}

impl<R: Read> GifSource<R> {
    /// Read the GIF header. Fails on anything the `gif` crate rejects, and on
    /// an absurd declared logical screen.
    pub fn new(reader: R) -> io::Result<Self> {
        let mut opts = DecodeOptions::new();
        opts.set_color_output(ColorOutput::RGBA);
        let decoder = opts.read_info(reader).map_err(decode_err)?;

        let width = decoder.width() as usize;
        let height = decoder.height() as usize;
        if width == 0 || height == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "GIF logical screen has a zero dimension",
            ));
        }
        if width.saturating_mul(height) > MAX_LOGICAL_PIXELS {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("GIF logical screen {width}x{height} is implausibly large"),
            ));
        }

        Ok(GifSource {
            decoder,
            width,
            height,
            screen: vec![0u8; width * height * 4],
            saved: Vec::new(),
            pending: None,
            frames_decoded: 0,
        })
    }

    /// The GIF's declared logical screen size, which may not be 128x32.
    pub fn logical_size(&self) -> (usize, usize) {
        (self.width, self.height)
    }

    /// Frames handed out so far.
    pub fn frames_decoded(&self) -> u64 {
        self.frames_decoded
    }

    /// Decode the next frame onto `canvas` and return how long to show it.
    /// `Ok(None)` means end of stream; `canvas` is then untouched.
    pub fn next_frame(&mut self, canvas: &mut Canvas) -> io::Result<Option<Duration>> {
        let GifSource {
            decoder,
            width,
            height,
            screen,
            saved,
            pending,
            frames_decoded,
        } = self;
        let (width, height) = (*width, *height);

        // Whatever the last frame asked for, before drawing the next one.
        if let Some((method, rect)) = pending.take() {
            match method {
                DisposalMethod::Background => clear_rect(screen, width, rect),
                DisposalMethod::Previous if saved.len() == screen.len() => {
                    screen.copy_from_slice(saved);
                }
                // Any / Keep: leave the canvas alone.
                _ => {}
            }
        }

        let frame = match decoder.read_next_frame().map_err(decode_err)? {
            Some(f) => f,
            None => return Ok(None),
        };

        let rect = Rect {
            left: (frame.left as usize).min(width),
            top: (frame.top as usize).min(height),
            width: frame.width as usize,
            height: frame.height as usize,
        };

        if frame.dispose == DisposalMethod::Previous {
            if saved.len() != screen.len() {
                saved.resize(screen.len(), 0);
            }
            saved.copy_from_slice(screen);
        }

        // Composite: alpha is 0 exactly for the transparent palette index, and
        // transparent source pixels leave the screen as it was.
        for y in 0..rect.height {
            let dy = rect.top + y;
            if dy >= height {
                break;
            }
            for x in 0..rect.width {
                let dx = rect.left + x;
                if dx >= width {
                    break;
                }
                let src = (y * rect.width + x) * 4;
                if frame.buffer[src + 3] == 0 {
                    continue;
                }
                let dst = (dy * width + dx) * 4;
                screen[dst..dst + 4].copy_from_slice(&frame.buffer[src..src + 4]);
            }
        }

        let delay_ms = if frame.delay == 0 {
            ZERO_DELAY_MS
        } else {
            (frame.delay as u64) * 10
        };
        *pending = Some((frame.dispose, rect));
        *frames_decoded += 1;

        blit(screen, width, height, canvas);
        Ok(Some(Duration::from_millis(delay_ms.max(MIN_DELAY_MS))))
    }
}

fn clear_rect(screen: &mut [u8], screen_width: usize, rect: Rect) {
    let height = screen.len() / (screen_width * 4);
    for y in 0..rect.height {
        let dy = rect.top + y;
        if dy >= height {
            break;
        }
        let start = (dy * screen_width + rect.left) * 4;
        let span = rect.width.min(screen_width.saturating_sub(rect.left)) * 4;
        screen[start..start + span].fill(0);
    }
}

/// Centre-and-crop the logical screen onto the 128x32 canvas. Areas the GIF
/// doesn't cover, and its transparent pixels, are black.
fn blit(screen: &[u8], width: usize, height: usize, canvas: &mut Canvas) {
    let off_x = (WIDTH as i32 - width as i32) / 2;
    let off_y = (HEIGHT as i32 - height as i32) / 2;
    for cy in 0..HEIGHT {
        let sy = cy as i32 - off_y;
        for cx in 0..WIDTH {
            let sx = cx as i32 - off_x;
            let px = if sx >= 0 && sy >= 0 && (sx as usize) < width && (sy as usize) < height {
                let o = ((sy as usize) * width + sx as usize) * 4;
                if screen[o + 3] == 0 {
                    Rgb::BLACK
                } else {
                    Rgb::new(screen[o], screen[o + 1], screen[o + 2])
                }
            } else {
                Rgb::BLACK
            };
            canvas.set(cx, cy, px);
        }
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
}

//! An animated GIF as the Studio's source: every frame decoded (the core's
//! [`GifSource`], so disposal and transparency are composited exactly as
//! the player does: each frame a full picture), kept at the source's native
//! pixels, and rendered frame by frame through the same Frame and Look
//! steps as a still ([`pipeline::stage_frame`]), lazily: a change of a
//! control drops the rendered frames and each is made again when it is
//! shown, or when the GIF is saved. No browser types (tested natively and
//! under node).
//!
//! Memory: the frames are kept as RGB, 3 bytes a pixel. A nearest-neighbour
//! upscale found on the first frame ([`pipeline::native_scale`]) is undone
//! on the way in, losing nothing. If the frames would still take more than
//! [`BUDGET`], each side is divided by the smallest whole number that fits
//! (area-averaged, [`pipeline::resize_area`]) and [`Anim::reduce`] says by
//! what. At most [`MAX_FRAMES`] frames are kept. A rendered frame is its
//! own palette (≤ 256) and 4 KB of indices.

use crate::encode;
use crate::pipeline::{self, Colours, Edit, Fit, Image, Mapper, Overlay, Scale, H, W};
use opendmd_core::canvas::Canvas;
use opendmd_core::player::GifSource;
use std::collections::HashMap;
use std::io::{self, Cursor};

/// The most the source frames may take, in bytes (RGB).
pub const BUDGET: usize = 64 << 20;
/// Frames past this are not kept.
pub const MAX_FRAMES: usize = 5_000;
/// A preview frame's delay: long, so the one-frame GIF never loops by itself.
const HOLD_MS: u32 = 60_000;
/// Image descriptors in a GIF, by walking its blocks without decoding any
/// (cheap: the import sizes its memory by it before decoding). A truncated
/// frame is not counted.
pub fn scan_frames(b: &[u8]) -> usize {
    if b.len() < 13 || &b[..3] != b"GIF" {
        return 0;
    }
    let table = |flags: u8| {
        if flags & 0x80 != 0 {
            3usize << ((flags & 7) + 1)
        } else {
            0
        }
    };
    let skip = |mut i: usize| -> Option<usize> {
        loop {
            let l = *b.get(i)? as usize;
            i += 1;
            if l == 0 {
                return Some(i);
            }
            i += l;
        }
    };
    let (mut i, mut n) = (13 + table(b[10]), 0);
    loop {
        match b.get(i) {
            Some(0x21) => match skip(i + 2) {
                Some(j) => i = j,
                None => return n,
            },
            Some(0x2C) => {
                let Some(&f) = b.get(i + 9) else { return n };
                match skip(i + 10 + table(f) + 1) {
                    Some(j) if j <= b.len() => i = j,
                    _ => return n,
                }
                n += 1;
            }
            _ => return n,
        }
    }
}

/// One rendered 128x32 frame: its own palette and indices.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rendered {
    pub palette: Vec<u32>,
    pub idx: Vec<u8>,
}

impl Rendered {
    pub fn rgba(&self) -> Vec<u8> {
        self.idx
            .iter()
            .flat_map(|&k| {
                let c = self.palette[k as usize];
                [(c >> 16) as u8, (c >> 8) as u8, c as u8, 255]
            })
            .collect()
    }
}

/// The GIF being read, one per animation.
enum Decoding {
    Gif {
        src: GifSource<Cursor<Vec<u8>>>,
        canvas: Canvas,
    },
}

/// Everything the Frame and Look steps decide (compared, so a call that
/// changes nothing keeps the rendered frames).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Look {
    fit: Fit,
    lift: bool,
    x: usize,
    y: usize,
    key: bool,
    gain: u32,
    colours: Colours,
}

struct Saving {
    seq: Vec<(usize, u32)>,
    loops: u32,
    uniq: Vec<usize>,
    next: usize,
    enc: Option<gif::Encoder<Vec<u8>>>,
    remap: HashMap<usize, Vec<u8>>,
    written: usize,
}

/// The animation being edited.
pub struct Anim {
    dec: Option<Decoding>,
    /// The file's logical screen.
    pub orig: (usize, usize),
    /// Image descriptors in the file ([`scan_frames`]).
    pub in_file: usize,
    /// The upscale undone on import (k = 1: none).
    pub scale: Scale,
    /// The further reduction to fit [`BUDGET`] (1: none).
    pub reduce: usize,
    /// The working size of every frame.
    pub w: usize,
    pub h: usize,
    frames: Vec<Vec<u8>>,
    /// Per frame, as the player shows it ([`pipeline::player_delay`]).
    pub delays: Vec<u32>,
    /// From the file: 0 = forever, else how many times it plays.
    pub loops: u32,
    /// Decoding stopped at a frame that would not decode.
    pub damaged: bool,
    /// A Formula's frames cut short of a whole loop ([`frames`](Self::frames)).
    pub cut: bool,
    look: Look,
    look_set: bool,
    flip: bool,
    key_rgb: Option<[u8; 3]>,
    text: Option<(Overlay, Vec<bool>)>,
    cache: Vec<Option<Rendered>>,
    saving: Option<Saving>,
    /// Colours of the last save.
    pub colours: usize,
}

impl Anim {
    /// Read the header and the first frame; the rest come from
    /// [`decode`](Self::decode).
    pub fn open(bytes: Vec<u8>) -> io::Result<Anim> {
        let in_file = scan_frames(&bytes);
        let mut src = GifSource::new(Cursor::new(bytes))?;
        let mut canvas = Canvas::new();
        let (ow, oh) = src.logical_size();
        src.next_frame(&mut canvas)?
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "no frames"))?;
        let first = rgb_image(src.screen(&canvas), ow, oh);
        let scale = pipeline::native_scale(&first);
        let (nw, nh) = if scale.k > 1 {
            ((ow - scale.x0) / scale.k, (oh - scale.y0) / scale.k)
        } else {
            (ow, oh)
        };
        let keep = in_file.clamp(1, MAX_FRAMES);
        let mut reduce = 1;
        // u64: on wasm32 5,000 frames of 2048x2048 overflow a usize.
        let bytes = |f: usize| keep as u64 * (nw / f).max(1) as u64 * (nh / f).max(1) as u64 * 3;
        while reduce < nw.max(nh) && bytes(reduce) > BUDGET as u64 {
            reduce += 1;
        }
        let (w, h) = ((nw / reduce).max(1), (nh / reduce).max(1));
        let mut a = Anim {
            dec: None,
            orig: (ow, oh),
            in_file,
            scale,
            reduce,
            w,
            h,
            frames: Vec::with_capacity(keep),
            delays: Vec::with_capacity(keep),
            loops: match src.repeat() {
                gif::Repeat::Finite(n) if n > 0 => n as u32 + 1,
                _ => 0,
            },
            damaged: false,
            cut: false,
            look: Look {
                fit: Fit::Pixels,
                lift: false,
                x: 0,
                y: 0,
                key: false,
                gain: 100,
                colours: Colours::Auto,
            },
            look_set: false,
            flip: false,
            key_rgb: None,
            text: None,
            cache: Vec::new(),
            saving: None,
            colours: 0,
        };
        a.keep(&first, pipeline::player_delay(src.raw_delay()));
        a.dec = Some(Decoding::Gif { src, canvas });
        Ok(a)
    }

    /// Frames the page draws (a formula, by the Psi module): `n` panel
    /// frames of 128x32 RGB888 at `delay_ms` each, `first` now and the rest
    /// by [`push`](Self::push); `cut`: not a whole loop.
    pub fn frames(first: &[u8], n: usize, delay_ms: u32, cut: bool) -> Option<Anim> {
        if first.len() != W * H * 3 {
            return None;
        }
        let mut a = Anim::panel(n.clamp(1, MAX_FRAMES), cut);
        a.keep(&rgb_image(first, W, H), delay_ms);
        Some(a)
    }

    /// The next of [`frames`](Self::frames)' frames, at the first's delay;
    /// true once all of them are in (more are ignored).
    pub fn push(&mut self, rgb: &[u8]) -> bool {
        if self.frames.len() < self.in_file && rgb.len() == W * H * 3 {
            let d = self.delays[0];
            self.keep(&rgb_image(rgb, W, H), d);
        }
        self.frames.len() >= self.in_file
    }

    /// An empty animation of `n` frames of the panel's size.
    fn panel(n: usize, cut: bool) -> Anim {
        Anim {
            dec: None,
            orig: (W, H),
            in_file: n,
            scale: Scale { k: 1, x0: 0, y0: 0 },
            reduce: 1,
            w: W,
            h: H,
            frames: Vec::with_capacity(n),
            delays: Vec::with_capacity(n),
            loops: 0,
            damaged: false,
            cut,
            look: Look {
                fit: Fit::Pixels,
                lift: false,
                x: 0,
                y: 0,
                key: false,
                gain: 100,
                colours: Colours::Auto,
            },
            look_set: false,
            flip: false,
            key_rgb: None,
            text: None,
            cache: Vec::new(),
            saving: None,
            colours: 0,
        }
    }

    fn keep(&mut self, img: &Image, delay_ms: u32) {
        let mut img = if self.scale.k > 1 {
            pipeline::native(img, self.scale)
        } else {
            img.clone()
        };
        if self.reduce > 1 {
            img = pipeline::resize_area(&img, self.w, self.h);
        }
        self.frames
            .push(img.px.iter().flat_map(|p| [p[0], p[1], p[2]]).collect());
        self.delays.push(delay_ms);
        self.cache.push(None);
    }

    /// Decode up to `n` more frames; true once every frame is in (or the
    /// rest would not decode: [`damaged`](Self::damaged)).
    pub fn decode(&mut self, n: usize) -> bool {
        for _ in 0..n {
            let Some(d) = self.dec.as_mut() else {
                return true;
            };
            if self.frames.len() >= MAX_FRAMES {
                self.dec = None;
                return true;
            }
            let Decoding::Gif { src, canvas } = d;
            match src.next_frame(canvas) {
                Ok(Some(_)) => {
                    let (w, h) = src.logical_size();
                    let img = rgb_image(src.screen(canvas), w, h);
                    let delay = pipeline::player_delay(src.raw_delay());
                    self.keep(&img, delay);
                }
                Ok(None) => self.dec = None,
                Err(_) => {
                    self.damaged = true;
                    self.dec = None;
                }
            }
        }
        self.dec.is_none()
    }

    pub fn len(&self) -> usize {
        self.frames.len()
    }

    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    /// Frames not kept past [`MAX_FRAMES`].
    pub fn capped(&self) -> bool {
        self.in_file > MAX_FRAMES
    }

    /// Source frame `i` at its working size, opaque.
    pub fn image(&self, i: usize) -> Image {
        rgb_image(&self.frames[i], self.w, self.h)
    }

    /// Frame `i`'s stage (the picture the window is cut from).
    pub fn stage(&self, i: usize) -> Image {
        pipeline::stage_frame(&self.image(i), self.look.fit, self.flip)
    }

    /// The Frame and Look steps. Keeps the rendered frames when nothing
    /// changed; else they are made again as they are asked for. The
    /// background key's colour and the dark-stroke decision come from the
    /// first frame, once.
    #[allow(clippy::too_many_arguments)]
    pub fn configure(
        &mut self,
        fit: Fit,
        lift: bool,
        x: usize,
        y: usize,
        key: bool,
        gain: u32,
        colours: Colours,
    ) {
        let look = Look {
            fit,
            lift,
            x,
            y,
            key,
            gain,
            colours,
        };
        if self.look_set && look == self.look {
            return;
        }
        self.look = look;
        self.look_set = true;
        self.flip = lift && pipeline::should_lift(&self.image(0));
        self.key_rgb = key
            .then(|| {
                let st = self.stage(0);
                let (x, y) = self.window(&st);
                pipeline::commonest(&st.crop(x, y, W.min(st.w), H.min(st.h)))
            })
            .flatten();
        self.cache.iter_mut().for_each(|c| *c = None);
    }

    fn window(&self, st: &Image) -> (usize, usize) {
        (
            self.look.x.min(st.w - W.min(st.w)),
            self.look.y.min(st.h - H.min(st.h)),
        )
    }

    /// The text over the frames (`None`: none). Only the frames it covered
    /// or covers are made again.
    pub fn set_text(&mut self, t: Option<Overlay>) {
        if self.text.as_ref().map(|x| &x.0) == t.as_ref() {
            return;
        }
        let range = |o: &Option<(Overlay, Vec<bool>)>| o.as_ref().map(|(t, _)| (t.from, t.to));
        let old = range(&self.text);
        self.text = t.map(|t| {
            let m = t.mask();
            (t, m)
        });
        for (a, b) in [old, range(&self.text)].into_iter().flatten() {
            for c in self.cache.iter_mut().take(b.saturating_add(1)).skip(a) {
                *c = None;
            }
        }
    }

    /// Frame `i`, rendered (from the cache when nothing changed since).
    pub fn render(&mut self, i: usize) -> &Rendered {
        if self.cache[i].is_none() {
            let r = self.make(i);
            self.cache[i] = Some(r);
        }
        self.cache[i].as_ref().expect("just made")
    }

    pub fn is_rendered(&self, i: usize) -> bool {
        self.cache.get(i).is_some_and(|c| c.is_some())
    }

    fn make(&self, i: usize) -> Rendered {
        let st = self.stage(i);
        let (x, y) = self.window(&st);
        let mut win = st.crop(x, y, W.min(st.w), H.min(st.h));
        if let Some(bg) = self.key_rgb {
            pipeline::key_colour(&mut win, bg, 12);
        }
        pipeline::gain(&mut win, self.look.gain);
        if let Some((t, m)) = &self.text {
            if t.covers(i) {
                t.draw(m, &mut win);
            }
        }
        let counts = pipeline::colour_counts(&win);
        let palette = pipeline::spread_pick(&counts, self.look.colours.max());
        let mut map = Mapper::new(&palette);
        let idx = win
            .px
            .iter()
            .map(|&p| map.index(pipeline::rgb_key(p)))
            .collect();
        Rendered { palette, idx }
    }

    /// Frame `i` as a one-frame GIF for the preview (held for a minute).
    pub fn frame_gif(&mut self, i: usize) -> Vec<u8> {
        let r = self.render(i).clone();
        encode::encode(&pipeline::Animation {
            palette: r.palette,
            frames: vec![r.idx],
            delays_ms: vec![HOLD_MS],
        })
    }

    /// The edited animation's order and delays ([`pipeline::sequence`]).
    pub fn sequence(&self, e: &Edit) -> Vec<(usize, u32)> {
        pipeline::sequence(&self.delays, e)
    }

    /// Start writing the GIF of `seq` (from [`sequence`](Self::sequence)),
    /// played `loops` times (0: forever).
    pub fn save_begin(&mut self, seq: Vec<(usize, u32)>, loops: u32) {
        let mut uniq: Vec<usize> = seq.iter().map(|s| s.0).collect();
        uniq.sort_unstable();
        uniq.dedup();
        self.saving = Some(Saving {
            seq,
            loops,
            uniq,
            next: 0,
            enc: None,
            remap: HashMap::new(),
            written: 0,
        });
    }

    /// Up to `n` frames of work (rendering, then writing); true when the
    /// GIF is written ([`saved`](Self::saved)).
    pub fn save_step(&mut self, n: usize) -> bool {
        let Some(mut s) = self.saving.take() else {
            return true;
        };
        let mut budget = n.max(1);
        // 1. Every frame used, rendered.
        while s.next < s.uniq.len() && budget > 0 {
            self.render(s.uniq[s.next]);
            s.next += 1;
            budget -= 1;
        }
        if s.next < s.uniq.len() {
            self.saving = Some(s);
            return false;
        }
        // 2. One palette for the whole animation: the frames' own colours
        // when they fit together (the preview is then exact), else picked
        // for spread over all of them.
        if s.enc.is_none() {
            let mut counts: HashMap<u32, u32> = HashMap::new();
            for &i in &s.uniq {
                let r = self.cache[i].as_ref().expect("rendered");
                let mut hist = vec![0u32; r.palette.len()];
                for &k in &r.idx {
                    hist[k as usize] += 1;
                }
                for (c, n) in r.palette.iter().zip(hist) {
                    *counts.entry(*c).or_default() += n;
                }
            }
            let mut v: Vec<(u32, u32)> = counts.into_iter().collect();
            v.sort_unstable_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
            let palette = pipeline::spread_pick(&v, self.look.colours.max());
            let mut map = Mapper::new(&palette);
            for &i in &s.uniq {
                let r = self.cache[i].as_ref().expect("rendered");
                s.remap
                    .insert(i, r.palette.iter().map(|&c| map.index(c)).collect());
            }
            self.colours = palette.len();
            s.enc = Some(encode::begin(&palette, s.loops));
        }
        // 3. The frames.
        while s.written < s.seq.len() && budget > 0 {
            let (i, d) = s.seq[s.written];
            let r = self.cache[i].as_ref().expect("rendered");
            let m = &s.remap[&i];
            let idx: Vec<u8> = r.idx.iter().map(|&k| m[k as usize]).collect();
            encode::frame(s.enc.as_mut().expect("begun"), &idx, d);
            s.written += 1;
            budget -= 1;
        }
        let done = s.written == s.seq.len();
        self.saving = Some(s);
        done
    }

    /// Frames rendered or written so far, of how many (for a progress line).
    pub fn save_progress(&self) -> (usize, usize) {
        self.saving
            .as_ref()
            .map_or((0, 0), |s| (s.next + s.written, s.uniq.len() + s.seq.len()))
    }

    /// The GIF, once [`save_step`](Self::save_step) said done.
    pub fn saved(&mut self) -> Vec<u8> {
        self.saving
            .take()
            .and_then(|s| s.enc)
            .map(encode::finish)
            .unwrap_or_default()
    }
}

/// RGB bytes (`w * h * 3`) as an opaque picture.
fn rgb_image(rgb: &[u8], w: usize, h: usize) -> Image {
    Image {
        w,
        h,
        px: rgb
            .as_chunks::<3>()
            .0
            .iter()
            .map(|p| [p[0], p[1], p[2], 255])
            .collect(),
    }
}

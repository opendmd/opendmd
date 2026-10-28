//! The Studio ("Make art" in Library): a picture in, a 128x32 GIF out.
//!
//! A second wasm module, separate from `opendmd-web` and loaded only when
//! the Studio opens (`www/pkg-art/`), so the app everyone loads does not
//! grow. The split is the web crate's: [`pipeline`] and [`encode`] hold no
//! browser types and are tested natively and under node; this file only
//! converts. Parsing the saved name is the main module's `parsePath`.
//!
//! It also carries [`prepare_gif`] (`prepareGif`), the prepared copy of any
//! GIF — the core's recoder, byte-identical to `opendmd-pack` and the
//! device — for the prepare job, Add GIFs and the Studio's save: ~10 KB gz
//! the main module has no room for, loaded only when something is prepared.
//! And Smooth motion ([`Smoother`], `smoothMarker`, `prepareSmoothGif`):
//! the core's `tween` — the analysis a Library preview shows, the tweened
//! frames its Preview plays and the smoothed copy — in steps the page
//! yields between, byte-identical to `opendmd-pack`'s.

pub mod anim;
pub mod encode;
pub mod pipeline;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_anim;

use anim::Anim;
use pipeline::{Colours, Edit, Fit, Image, Motion, Overlay, Plan, Scale};
use wasm_bindgen::prelude::*;

/// One picture being made into art. `new Studio(rgba, width, height)` from
/// `ImageData`; then `prepare(fit, lift)` builds the stage (the picture the
/// window moves over, `width` x `height`, `pixels()`), `autoMotion()` and
/// `autoWindow(motion)` suggest, and `render(...)` makes the GIF.
///
/// Or an animated GIF: `Studio.fromGif(bytes)`, then `decode(n)` until true
/// (the frames, `n` a call, so the page can yield), `prepare` as above (the
/// stage is the first frame's), `look(...)` for the Frame and Look steps,
/// `setEdit(...)` for the timeline, `frameGif(i)` / `thumb(i)` per source
/// frame (rendered when first asked for), and `saveBegin(plays)`,
/// `saveStep(n)` until true, `saved()` for the GIF ([`anim`]).
#[wasm_bindgen]
pub struct Studio {
    raw: Image,
    scale: Scale,
    stage: Image,
    frames: usize,
    colours: usize,
    duration_ms: f64,
    anim: Option<Anim>,
    fit: Fit,
    lift: bool,
    text: Option<Overlay>,
    seq: Vec<(usize, u32)>,
}

#[wasm_bindgen]
impl Studio {
    #[wasm_bindgen(constructor)]
    pub fn new(rgba: &[u8], width: usize, height: usize) -> Result<Studio, JsError> {
        let raw = Image::from_rgba(rgba, width, height)
            .ok_or_else(|| JsError::new("that picture could not be read"))?;
        let scale = pipeline::native_scale(&raw);
        let stage = pipeline::stage(&raw, scale, Fit::Pixels, false);
        Ok(Studio {
            raw,
            scale,
            stage,
            frames: 0,
            colours: 0,
            duration_ms: 0.0,
            anim: None,
            fit: Fit::Pixels,
            lift: false,
            text: None,
            seq: Vec::new(),
        })
    }

    /// An animated GIF: its header and first frame read (throws if it does
    /// not decode); `decode` reads the rest.
    #[wasm_bindgen(js_name = fromGif)]
    pub fn from_gif(bytes: Vec<u8>) -> Result<Studio, JsError> {
        let a = Anim::open(bytes).map_err(|e| JsError::new(&e.to_string()))?;
        let raw = a.image(0);
        let stage = pipeline::stage_frame(&raw, Fit::Pixels, false);
        Ok(Studio {
            raw,
            scale: Scale { k: 1, x0: 0, y0: 0 },
            stage,
            frames: 0,
            colours: 0,
            duration_ms: 0.0,
            anim: Some(a),
            fit: Fit::Pixels,
            lift: false,
            text: None,
            seq: Vec::new(),
        })
    }

    /// Frames drawn by the page — a formula's, by the Psi module: `n`
    /// frames of 128x32 RGB888 at `delayMs` each, `first` now and the rest
    /// by `push` (`cut`: they are not a whole loop). Throws on a frame of
    /// the wrong size.
    #[wasm_bindgen(js_name = fromFrames)]
    pub fn from_frames(first: &[u8], n: u32, delay_ms: u32, cut: bool) -> Result<Studio, JsError> {
        let a = Anim::frames(first, n as usize, delay_ms, cut)
            .ok_or_else(|| JsError::new("not a 128x32 frame"))?;
        let raw = a.image(0);
        let stage = pipeline::stage_frame(&raw, Fit::Pixels, false);
        Ok(Studio {
            raw,
            scale: Scale { k: 1, x0: 0, y0: 0 },
            stage,
            frames: 0,
            colours: 0,
            duration_ms: 0.0,
            anim: Some(a),
            fit: Fit::Pixels,
            lift: false,
            text: None,
            seq: Vec::new(),
        })
    }

    /// The next of `fromFrames`' frames; true once all are in.
    pub fn push(&mut self, rgb: &[u8]) -> bool {
        self.anim.as_mut().is_none_or(|a| a.push(rgb))
    }

    /// Decode up to `n` more frames; true once all are in.
    pub fn decode(&mut self, n: u32) -> bool {
        self.anim.as_mut().is_none_or(|a| a.decode(n as usize))
    }

    #[wasm_bindgen(getter)]
    pub fn animated(&self) -> bool {
        self.anim.is_some()
    }

    /// Source frames decoded (all of them once `decode` is done).
    #[wasm_bindgen(getter, js_name = sourceFrames)]
    pub fn source_frames(&self) -> usize {
        self.anim.as_ref().map_or(1, |a| a.len())
    }

    /// Frames the file has (read before decoding them).
    #[wasm_bindgen(getter, js_name = inFile)]
    pub fn in_file(&self) -> usize {
        self.anim.as_ref().map_or(1, |a| a.in_file)
    }

    /// `[width, height, workWidth, workHeight, reduce, damaged, capped, plays,
    /// cut]` of an animation (plays: 0 forever; cut: a Formula longer than
    /// the frames it was given); empty for a still.
    pub fn info(&self) -> Vec<u32> {
        self.anim.as_ref().map_or(Vec::new(), |a| {
            vec![
                a.orig.0 as u32,
                a.orig.1 as u32,
                a.w as u32,
                a.h as u32,
                a.reduce as u32,
                a.damaged as u32,
                a.capped() as u32,
                a.loops,
                a.cut as u32,
            ]
        })
    }

    /// Source frame delays as the player shows them, ms.
    pub fn delays(&self) -> Vec<u32> {
        self.anim.as_ref().map_or(Vec::new(), |a| a.delays.clone())
    }

    /// The upscale undone by the pixel fits (1: none found).
    #[wasm_bindgen(getter)]
    pub fn scale(&self) -> usize {
        self.anim.as_ref().map_or(self.scale.k, |a| a.scale.k)
    }

    /// `"pixels"`, `"half"` or `"fit"`: what to start with.
    #[wasm_bindgen(js_name = autoFit)]
    pub fn auto_fit(&self) -> String {
        match &self.anim {
            // Frames are kept at their native pixels already; a big picture
            // with no pixel grid is a video still or a photo: Fit.
            Some(a) if a.scale.k == 1 && a.orig.0.max(a.orig.1) > 640 => Fit::Smooth,
            _ => pipeline::auto_fit(&self.raw, self.scale),
        }
        .name()
        .into()
    }

    /// Build the stage for a fit (`"pixels"`, `"half"`, `"fit"`), lifting
    /// dark strokes first when asked.
    pub fn prepare(&mut self, fit: &str, lift: bool) {
        self.fit = Fit::parse(fit);
        self.lift = lift;
        self.stage = match &self.anim {
            Some(_) => pipeline::stage_frame(
                &self.raw,
                self.fit,
                lift && pipeline::should_lift(&self.raw),
            ),
            None => pipeline::stage(&self.raw, self.scale, self.fit, lift),
        };
    }

    /// An animation's Frame and Look steps (the window at `x`, `y`; the
    /// rest as `render`). Rendered frames are kept if nothing changed.
    pub fn look(&mut self, x: u32, y: u32, key: bool, gain: u32, colours: u32) {
        let (fit, lift) = (self.fit, self.lift);
        if let Some(a) = self.anim.as_mut() {
            a.configure(
                fit,
                lift,
                x as usize,
                y as usize,
                key,
                gain.clamp(10, 200),
                Colours::from_count(colours),
            );
        }
    }

    /// Text over the frames: `msg` in the small (4x6) or normal (5x7)
    /// font, colour `0xRRGGBB`, anchor 0..=8 (top-left, row by row), nudged
    /// by `dx`, `dy`, over source frames `from..=to` (an animation's). An
    /// empty message is no text.
    #[allow(clippy::too_many_arguments)]
    #[wasm_bindgen(js_name = setText)]
    pub fn set_text(
        &mut self,
        msg: &str,
        small: bool,
        colour: u32,
        anchor: u8,
        dx: i32,
        dy: i32,
        from: u32,
        to: u32,
    ) {
        let t = (!msg.trim().is_empty()).then(|| Overlay {
            msg: msg.into(),
            small,
            colour,
            anchor: anchor.min(8),
            dx: dx.clamp(-128, 128),
            dy: dy.clamp(-32, 32),
            from: from as usize,
            to: to as usize,
        });
        if let Some(a) = self.anim.as_mut() {
            a.set_text(t.clone());
        }
        self.text = t;
    }

    /// The timeline: source frames `from..=to` less `deleted`, at `speed`
    /// percent, reversed, ping-pong ([`pipeline::sequence`]). Returns the
    /// play order flattened, `[frame, delay ms, frame, delay ms, ...]`.
    #[allow(clippy::too_many_arguments)]
    #[wasm_bindgen(js_name = setEdit)]
    pub fn set_edit(
        &mut self,
        from: u32,
        to: u32,
        speed: u32,
        reverse: bool,
        pingpong: bool,
        deleted: &[u32],
    ) -> Vec<u32> {
        let e = Edit {
            from: from as usize,
            to: to as usize,
            speed,
            reverse,
            pingpong,
            deleted: deleted.iter().map(|&d| d as usize).collect(),
        };
        self.seq = self.anim.as_ref().map_or(Vec::new(), |a| a.sequence(&e));
        self.frames = self.seq.len();
        self.duration_ms = self.seq.iter().map(|s| s.1 as f64).sum();
        self.seq.iter().flat_map(|&(i, d)| [i as u32, d]).collect()
    }

    /// Source frame `i`, edited, as a one-frame GIF for the preview.
    #[wasm_bindgen(js_name = frameGif)]
    pub fn frame_gif(&mut self, i: usize) -> Vec<u8> {
        self.anim
            .as_mut()
            .map_or(Vec::new(), |a| a.frame_gif(i.min(a.len() - 1)))
    }

    /// Source frame `i`, edited, as 128x32 RGBA (a thumbnail).
    pub fn thumb(&mut self, i: usize) -> Vec<u8> {
        self.anim
            .as_mut()
            .map_or(Vec::new(), |a| a.render(i.min(a.len() - 1)).rgba())
    }

    /// Whether `thumb(i)` is ready without work.
    #[wasm_bindgen(js_name = isRendered)]
    pub fn is_rendered(&self, i: usize) -> bool {
        self.anim.as_ref().is_some_and(|a| a.is_rendered(i))
    }

    /// Start writing the edited animation (the last `setEdit`), played
    /// `plays` times (0: forever).
    #[wasm_bindgen(js_name = saveBegin)]
    pub fn save_begin(&mut self, plays: u32) {
        let seq = self.seq.clone();
        if let Some(a) = self.anim.as_mut() {
            a.save_begin(seq, plays);
        }
    }

    /// `n` frames' worth of the save; true once written.
    #[wasm_bindgen(js_name = saveStep)]
    pub fn save_step(&mut self, n: u32) -> bool {
        self.anim.as_mut().is_none_or(|a| a.save_step(n as usize))
    }

    /// `[done, of]`: the save's progress, in frames of work.
    #[wasm_bindgen(js_name = saveProgress)]
    pub fn save_progress(&self) -> Vec<u32> {
        let (d, t) = self.anim.as_ref().map_or((0, 0), |a| a.save_progress());
        vec![d as u32, t as u32]
    }

    /// The saved GIF (then `colours` is its palette's size).
    pub fn saved(&mut self) -> Vec<u8> {
        match self.anim.as_mut() {
            Some(a) => {
                let b = a.saved();
                self.colours = a.colours;
                b
            }
            None => Vec::new(),
        }
    }

    #[wasm_bindgen(getter)]
    pub fn width(&self) -> usize {
        self.stage.w
    }

    #[wasm_bindgen(getter)]
    pub fn height(&self) -> usize {
        self.stage.h
    }

    /// The stage as RGBA bytes, for drawing.
    pub fn pixels(&self) -> Vec<u8> {
        self.stage.to_rgba()
    }

    /// `[x, y]`: where the window starts on the stage for a motion
    /// (`"still"`: the busiest 128x32; `"pan"`: the busiest band;
    /// `"scroll"`: the busiest column).
    #[wasm_bindgen(js_name = autoWindow)]
    pub fn auto_window(&self, motion: &str) -> Vec<u32> {
        let (x, y) = pipeline::auto_window(&self.stage, Motion::parse(motion));
        vec![x as u32, y as u32]
    }

    /// `"still"`, `"pan"` or `"scroll"`, from the stage's shape.
    #[wasm_bindgen(js_name = autoMotion)]
    pub fn auto_motion(&self) -> String {
        pipeline::auto_motion(self.stage.w, self.stage.h)
            .name()
            .into()
    }

    /// The GIF: `motion` `"still"`/`"pan"`/`"scroll"` from window (`x`, `y`),
    /// `step_ms` a pixel, `hold_ms` on the end frames, background to black,
    /// brightness `gain` percent, at most `colours` colours (0: auto).
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        motion: &str,
        x: u32,
        y: u32,
        step_ms: u32,
        hold_ms: u32,
        key: bool,
        gain: u32,
        colours: u32,
    ) -> Vec<u8> {
        let plan = Plan {
            motion: Motion::parse(motion),
            x: x as usize,
            y: y as usize,
            step_ms,
            hold_ms,
            key,
            gain: gain.clamp(10, 200),
            colours: Colours::from_count(colours),
        };
        let anim = pipeline::render_with(&self.stage, &plan, self.text.as_ref());
        self.frames = anim.frames.len();
        self.colours = anim.palette.len();
        self.duration_ms = anim.duration_ms() as f64;
        encode::encode(&anim)
    }

    /// Of the last `render`.
    #[wasm_bindgen(getter)]
    pub fn frames(&self) -> usize {
        self.frames
    }
    #[wasm_bindgen(getter)]
    pub fn colours(&self) -> usize {
        self.colours
    }
    #[wasm_bindgen(getter, js_name = durationMs)]
    pub fn duration_ms(&self) -> f64 {
        self.duration_ms
    }
}

/// A No-Intro name as a title: `(USA)`-style tags dropped, a trailing
/// article moved to the front (`opendmd_core::follow::clean_game_name`).
#[wasm_bindgen(js_name = cleanGameName)]
pub fn clean_game_name(name: &str) -> String {
    opendmd_core::follow::clean_game_name(name)
}

/// What people call a platform tag (`MD` -> "Mega Drive"); empty if unknown.
#[wasm_bindgen(js_name = consoleName)]
pub fn console_name(platform: &str) -> String {
    opendmd_core::follow::console_name(platform).into()
}

/// GIF bytes -> its prepared copy with the core's recoder in the prepared
/// format (the one `opendmd-pack` and the device run, so the output is
/// byte-identical). Returns `{copy: Uint8Array, stats: {width, height,
/// frames, colours, gif_bytes, out_bytes}}`; throws on a GIF the player
/// could not play either, and — with `gifslim::ORIGINAL_REASON`, "more
/// than 256 colours: plays as it is" — on one that gets no copy because
/// only a lossy one could be made (the panel plays the GIF itself).
#[wasm_bindgen(js_name = prepareGif)]
pub fn prepare_gif(gif: &[u8]) -> Result<JsValue, JsError> {
    use js_sys::{Object, Reflect, Uint8Array};
    use opendmd_core::gifslim::{recode_gif, PREPARED};
    let mut out = Vec::with_capacity(gif.len());
    let s = recode_gif(gif, &mut out, &PREPARED).map_err(|e| JsError::new(&e.to_string()))?;
    let set = |o: &Object, k: &str, v: JsValue| {
        let _ = Reflect::set(o, &JsValue::from_str(k), &v);
    };
    let stats = Object::new();
    set(&stats, "width", (s.width as u32).into());
    set(&stats, "height", (s.height as u32).into());
    set(&stats, "frames", s.frames.into());
    set(&stats, "colours", (s.colours as u32).into());
    set(&stats, "gif_bytes", (s.gif_bytes as f64).into());
    set(&stats, "out_bytes", (s.out_bytes as f64).into());
    let o = Object::new();
    set(&o, "copy", Uint8Array::from(&out[..]).into());
    set(&o, "stats", stats.into());
    Ok(o.into())
}

/// Smooth motion for one GIF (`opendmd_core::tween::Smooth`), in steps so
/// the page can yield between them: `new Smoother(gifBytes)`, then
/// `analyse(n)` until it returns true (`n` frames a call; `decoded` counts
/// them), then the analysis (`frames`, `pairs`, `tweened`, `added`,
/// `percent`, `offered`, `hint`); `tween(n)` until true, then `gif()` — the tweened
/// frames as a plain GIF, what the preview plays — and `prepare()`, the
/// smoothed prepared copy (marked). Throws on a GIF that does not decode.
#[wasm_bindgen]
pub struct Smoother {
    s: opendmd_core::tween::Smooth,
}

fn js_err(e: std::io::Error) -> JsError {
    JsError::new(&e.to_string())
}

#[wasm_bindgen]
impl Smoother {
    #[wasm_bindgen(constructor)]
    pub fn new(gif: &[u8]) -> Result<Smoother, JsError> {
        Ok(Smoother {
            s: opendmd_core::tween::Smooth::new(gif).map_err(js_err)?,
        })
    }

    /// Decode and classify up to `frames` more frames; true once done.
    pub fn analyse(&mut self, frames: u32) -> Result<bool, JsError> {
        self.s.analyse(frames).map_err(js_err)
    }

    /// Write up to `frames` more source frames of the tweened GIF; true
    /// once done.
    pub fn tween(&mut self, frames: u32) -> Result<bool, JsError> {
        self.s.tween(frames).map_err(js_err)
    }

    /// Frames decoded so far (the total once analysed).
    #[wasm_bindgen(getter)]
    pub fn decoded(&self) -> u32 {
        self.s.summary().frames
    }
    #[wasm_bindgen(getter)]
    pub fn frames(&self) -> u32 {
        self.s.summary().frames
    }
    /// Frame pairs, the loop's last -> first included.
    #[wasm_bindgen(getter)]
    pub fn pairs(&self) -> u32 {
        self.s.summary().pairs
    }
    /// Pairs that move in steps of 2 px or more the tweener takes.
    #[wasm_bindgen(getter)]
    pub fn tweened(&self) -> u32 {
        self.s.summary().tweened
    }
    /// Frames Smooth motion adds.
    #[wasm_bindgen(getter)]
    pub fn added(&self) -> u32 {
        self.s.summary().added
    }
    /// `tweened` as a whole percentage of `pairs`, rounded down.
    #[wasm_bindgen(getter)]
    pub fn percent(&self) -> u32 {
        self.s.summary().percent()
    }
    /// Worth offering (≥ 10% of the pairs).
    #[wasm_bindgen(getter)]
    pub fn offered(&self) -> bool {
        self.s.summary().offered()
    }

    /// Why there is little or nothing to smooth, for the wording:
    /// `"offered"`, `"steady"` (it moves 1 px at a time: already as smooth
    /// as whole pixels allow), `"fast"` (big steps in frames too short to
    /// split) or `"nothing"` (nothing slides).
    #[wasm_bindgen(getter)]
    pub fn hint(&self) -> String {
        self.s.summary().hint().name().into()
    }

    /// At most 256 colours, so a smoothed copy can be made at all (past
    /// 256 the GIF plays as it is, never smoothed).
    #[wasm_bindgen(getter)]
    pub fn fits(&self) -> bool {
        self.s.fits_palette()
    }

    /// The tweened frames as a plain GIF (empty until `tween` is done).
    pub fn gif(&self) -> Vec<u8> {
        self.s.tweened_gif().to_vec()
    }

    /// The smoothed prepared copy: the tweened GIF through the recoder in
    /// the prepared format, marked `OPENDMD1TWN`.
    pub fn prepare(&self) -> Result<Vec<u8>, JsError> {
        self.s
            .prepare(&opendmd_core::gifslim::PREPARED)
            .map(|(copy, _)| copy)
            .map_err(js_err)
    }
}

/// A copy's Smooth motion marker from its first bytes (~1 KB is plenty):
/// `{pairs, added}`, or `null` for a plain copy (or too few bytes).
#[wasm_bindgen(js_name = smoothMarker)]
pub fn smooth_marker(head: &[u8]) -> JsValue {
    use js_sys::{Object, Reflect};
    match opendmd_core::tween::marker(head) {
        Some(m) => {
            let o = Object::new();
            let _ = Reflect::set(&o, &"pairs".into(), &m.pairs.into());
            let _ = Reflect::set(&o, &"added".into(), &m.added.into());
            o.into()
        }
        None => JsValue::NULL,
    }
}

/// The smoothed prepared copy in one call (`opendmd_core::tween::smooth_gif`
/// in the prepared format): what `Smoother` gives step by step.
#[wasm_bindgen(js_name = prepareSmoothGif)]
pub fn prepare_smooth_gif(gif: &[u8]) -> Result<Vec<u8>, JsError> {
    let mut out = Vec::with_capacity(gif.len() * 2);
    opendmd_core::tween::smooth_gif(gif, &mut out, &opendmd_core::gifslim::PREPARED)
        .map_err(js_err)?;
    Ok(out)
}

// --- GIF names ------------------------------------------------------------
// A GIF's metadata is its file name (`opendmd_core::manifest::NameFields`):
// "Add GIFs" and "Edit details" write names through these. Here, not in
// the main module, which has no room for them (~3 KB gz); the dialogs load
// this module anyway to prepare the GIFs they add.

fn js_set(o: &js_sys::Object, k: &str, v: JsValue) {
    let _ = js_sys::Reflect::set(o, &JsValue::from_str(k), &v);
}

fn js_str(v: &Option<String>) -> JsValue {
    v.as_deref().map_or(JsValue::NULL, JsValue::from_str)
}

/// A GIF's metadata as its path carries it (`NameFields::from_path`):
/// `{title, platform, category, ordinal, author, flags}` — absent = `null`,
/// `category` `""` for a GIF directly in `gifs/`.
#[wasm_bindgen(js_name = nameFields)]
pub fn name_fields(path: &str) -> JsValue {
    let f = opendmd_core::manifest::NameFields::from_path(path);
    let o = js_sys::Object::new();
    js_set(&o, "title", f.title.as_str().into());
    js_set(&o, "platform", js_str(&f.platform));
    js_set(&o, "category", f.category.as_str().into());
    js_set(
        &o,
        "ordinal",
        f.ordinal.map_or(JsValue::NULL, JsValue::from),
    );
    js_set(&o, "author", js_str(&f.author));
    js_set(
        &o,
        "flags",
        f.flags
            .iter()
            .map(|s| JsValue::from_str(s))
            .collect::<js_sys::Array>()
            .into(),
    );
    o.into()
}

/// The card path for an object of [`name_fields`]' shape (its inverse,
/// `NameFields::path`); throws the reason's code (`title`, `title-char`,
/// `platform`, `category`, `author`, `needs-platform`, `too-long`,
/// `reads-back`) when the fields cannot be written as a name that reads
/// back as the same fields.
#[wasm_bindgen(js_name = buildName)]
pub fn build_name(fields: &JsValue) -> Result<String, JsValue> {
    let get =
        |k: &str| js_sys::Reflect::get(fields, &JsValue::from_str(k)).unwrap_or(JsValue::UNDEFINED);
    let text = |k: &str| get(k).as_string().filter(|s| !s.trim().is_empty());
    let flags = get("flags");
    let flags = if js_sys::Array::is_array(&flags) {
        js_sys::Array::from(&flags)
            .iter()
            .filter_map(|v| v.as_string())
            .collect()
    } else {
        Vec::new()
    };
    opendmd_core::manifest::NameFields {
        title: text("title").unwrap_or_default(),
        platform: text("platform"),
        category: get("category").as_string().unwrap_or_default(),
        ordinal: get("ordinal")
            .as_f64()
            .filter(|n| *n >= 0.0 && *n < 1e6)
            .map(|n| n as u32),
        author: text("author"),
        flags,
    }
    .path()
    .map_err(|e| JsValue::from_str(e.code()))
}

/// The platform tags the parser knows, written as a name carries them
/// (`NDS`, not `NINTENDO_DS`).
#[wasm_bindgen]
pub fn platforms() -> js_sys::Array {
    use opendmd_core::manifest::{MULTI_PLATFORMS, PLATFORMS};
    PLATFORMS
        .iter()
        .copied()
        .chain(
            MULTI_PLATFORMS
                .iter()
                .map(|(_, n)| *n)
                .filter(|n| !PLATFORMS.contains(n)),
        )
        .map(JsValue::from_str)
        .collect()
}

/// How many frames a GIF has, from its block structure (no decoding): the
/// Studio edits a GIF of two or more as an animation.
#[wasm_bindgen(js_name = gifFrames)]
pub fn gif_frames(bytes: &[u8]) -> usize {
    anim::scan_frames(bytes)
}

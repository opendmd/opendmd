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

pub mod encode;
pub mod pipeline;

#[cfg(test)]
mod tests;

use pipeline::{Colours, Fit, Image, Motion, Plan, Scale};
use wasm_bindgen::prelude::*;

/// One picture being made into art. `new Studio(rgba, width, height)` from
/// `ImageData`; then `prepare(fit, lift)` builds the stage (the picture the
/// window moves over, `width` x `height`, `pixels()`), `autoMotion()` and
/// `autoWindow(motion)` suggest, and `render(...)` makes the GIF.
#[wasm_bindgen]
pub struct Studio {
    raw: Image,
    scale: Scale,
    stage: Image,
    frames: usize,
    colours: usize,
    duration_ms: f64,
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
        })
    }

    /// The upscale undone by the pixel fits (1: none found).
    #[wasm_bindgen(getter)]
    pub fn scale(&self) -> usize {
        self.scale.k
    }

    /// `"pixels"`, `"half"` or `"fit"`: what to start with.
    #[wasm_bindgen(js_name = autoFit)]
    pub fn auto_fit(&self) -> String {
        pipeline::auto_fit(&self.raw, self.scale).name().into()
    }

    /// Build the stage for a fit (`"pixels"`, `"half"`, `"fit"`), lifting
    /// dark strokes first when asked.
    pub fn prepare(&mut self, fit: &str, lift: bool) {
        self.stage = pipeline::stage(&self.raw, self.scale, Fit::parse(fit), lift);
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
        let anim = pipeline::render(&self.stage, &plan);
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
/// frames, colours, quantised, gif_bytes, out_bytes}}`; throws on a GIF the
/// player could not play either.
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
    set(&stats, "quantised", s.quantised.into());
    set(&stats, "gif_bytes", (s.gif_bytes as f64).into());
    set(&stats, "out_bytes", (s.out_bytes as f64).into());
    let o = Object::new();
    set(&o, "copy", Uint8Array::from(&out[..]).into());
    set(&o, "stats", stats.into());
    Ok(o.into())
}

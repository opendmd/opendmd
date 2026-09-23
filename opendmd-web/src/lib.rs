//! OpenDMD's management UI in the browser: the same `opendmd-core` engine,
//! tag parser, filter and ODM encoder, and the same `opendmd-render` LED
//! renderer, compiled to `wasm32-unknown-unknown`.
//!
//! The ESP32 serves this module off the card and is otherwise a file server
//! with a JSON API; everything heavy — parsing and filtering manifests,
//! previewing files with the panel look, converting GIFs to `.odm` — runs
//! here. The split inside the crate is deliberate:
//!
//! - [`player`] and [`curate`] are plain Rust with no `web-sys` in them, so
//!   their tests run natively (`cargo test`) and under node (`wasm-pack test
//!   --node`).
//! - This file is the `wasm-bindgen` surface: [`Preview`] (the canvas-owning
//!   player) and free functions that convert between JS values and the core
//!   types. It holds no logic of its own.

pub mod curate;
pub mod player;

use curate::FilterSpec;
use js_sys::{Array, Object, Reflect, Uint8Array};
use opendmd_core::layout;
use opendmd_core::manifest::{normalize_title as core_normalize, Entry};
use opendmd_core::odm::{self, EncodeStats};
use opendmd_core::overrides::{parse_meta, render_meta, Dwell, MetaLine, Override};
use opendmd_render::BLOOM_STRENGTH;
use player::{Look, Player, Tick};
use std::collections::HashSet;
use std::time::Duration;
use wasm_bindgen::prelude::*;
use wasm_bindgen::Clamped;

/// Rebuild a card-absolute manifest path (`/gifs/<Category>/<file>.gif`) for
/// a file picked on its own, from whatever the browser gives us.
///
/// A directory pick yields `webkitRelativePath` — `dmdcard/gifs/Consoles/X.gif`
/// or, if the user picked `gifs/` itself, `gifs/Consoles/X.gif`. A plain file
/// pick yields only the basename. The category matters because `Filter` and
/// `Entry` both key off it, so:
///
/// 1. a `gifs` segment with at least two segments after it wins;
/// 2. otherwise the file's own parent directory is the category;
/// 3. otherwise `Unknown`.
#[wasm_bindgen(js_name = cardPath)]
pub fn card_path(relative: &str) -> String {
    let segments: Vec<&str> = relative
        .split('/')
        .filter(|s| !s.is_empty() && *s != ".")
        .collect();
    let Some((name, dirs)) = segments.split_last() else {
        return "/gifs/Unknown/".to_string();
    };
    let category = dirs
        .iter()
        .position(|s| s.eq_ignore_ascii_case("gifs"))
        .and_then(|i| dirs.get(i + 1).copied())
        .or_else(|| dirs.last().copied())
        .filter(|s| !s.eq_ignore_ascii_case("gifs"))
        .unwrap_or("Unknown");
    format!("/gifs/{category}/{name}")
}

// --- Small JS helpers ---------------------------------------------------

fn set(obj: &Object, key: &str, value: impl Into<JsValue>) {
    // Setting a property on a plain object we just created cannot fail.
    let _ = Reflect::set(obj, &JsValue::from_str(key), &value.into());
}

fn opt_str(v: &Option<String>) -> JsValue {
    v.as_deref().map_or(JsValue::NULL, JsValue::from_str)
}

fn get(obj: &JsValue, key: &str) -> JsValue {
    Reflect::get(obj, &JsValue::from_str(key)).unwrap_or(JsValue::UNDEFINED)
}

/// A JS string array, or a comma-separated string, as `Vec<String>`.
fn strings(v: &JsValue) -> Vec<String> {
    if let Some(s) = v.as_string() {
        return s
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();
    }
    if Array::is_array(v) {
        return Array::from(v)
            .iter()
            .filter_map(|x| x.as_string())
            .filter(|s| !s.trim().is_empty())
            .collect();
    }
    Vec::new()
}

fn entry_object(e: &Entry) -> Object {
    let o = Object::new();
    set(&o, "path", e.path.as_str());
    set(&o, "category", opt_str(&e.category));
    set(&o, "platform", opt_str(&e.platform));
    set(&o, "title", e.title.as_str());
    set(&o, "variant", e.variant.map_or(JsValue::NULL, |v| v.into()));
    let flags = Array::new();
    for f in &e.flags {
        flags.push(&JsValue::from_str(f));
    }
    set(&o, "flags", flags);
    o
}

// --- Tags, titles, filters ----------------------------------------------

/// `Entry::from_path` as a plain object:
/// `{path, category, platform, title, variant, flags}`; absent fields are
/// `null`.
#[wasm_bindgen(js_name = parsePath)]
pub fn parse_path(path: &str) -> JsValue {
    entry_object(&Entry::from_path(path)).into()
}

/// [`parse_path`] over every non-blank line of a manifest, in one call.
#[wasm_bindgen(js_name = parseManifest)]
pub fn parse_manifest(text: &str) -> Array {
    curate::manifest_lines(text)
        .map(|p| JsValue::from(entry_object(&Entry::from_path(p))))
        .collect()
}

/// Lowercase alphanumerics — how the core matches titles.
#[wasm_bindgen(js_name = normalizeTitle)]
pub fn normalize_title(s: &str) -> String {
    core_normalize(s)
}

fn filter_spec(json: &str) -> Result<FilterSpec, JsValue> {
    let v = if json.trim().is_empty() {
        Object::new().into()
    } else {
        js_sys::JSON::parse(json)?
    };
    let seed = get(&v, "seed").as_f64().unwrap_or(0.0);
    let limit = get(&v, "limit").as_f64().filter(|n| *n >= 0.0);
    let mut titles = strings(&get(&v, "titles"));
    if let Some(t) = get(&v, "title").as_string() {
        if !t.trim().is_empty() {
            titles.push(t);
        }
    }
    Ok(FilterSpec {
        platforms: strings(&get(&v, "platforms")),
        categories: strings(&get(&v, "categories")),
        exclude_categories: strings(&get(&v, "exclude_categories")),
        titles,
        exclude_flags: strings(&get(&v, "exclude_flags")),
        shuffle: get(&v, "shuffle").as_bool().unwrap_or(false),
        seed: seed.max(0.0) as u64,
        limit: limit.map(|n| n as usize),
    })
}

/// The manifest paths passing a filter, as a JS string array. `filter_json`
/// is `{"platforms":[..], "categories":[..], "exclude_categories":[..],
/// "titles":[..] | "title":"..", "exclude_flags":[..], "shuffle":bool,
/// "seed":n, "limit":n}`; every key is optional, lists may also be
/// comma-separated strings, and an empty list is no constraint.
#[wasm_bindgen(js_name = filterManifest)]
pub fn filter_manifest(manifest_text: &str, filter_json: &str) -> Result<Array, JsValue> {
    let spec = filter_spec(filter_json)?;
    Ok(curate::filter_paths(manifest_text, &spec)
        .iter()
        .map(|p| JsValue::from_str(p))
        .collect())
}

// --- Layout and conversion ----------------------------------------------

/// The bucket directory (`"00"`..`"ff"`) for an `.odm` file name — the same
/// FNV-1a rule `opendmd-pack --layout bucketed` uses.
#[wasm_bindgen(js_name = bucketFor)]
pub fn bucket_for(odm_name: &str) -> String {
    layout::bucket(odm_name)
}

/// `/gifs/Consoles/X.gif` -> `/odm/Consoles/<bb>/X.odm`, or `undefined` for
/// anything that is not a `.gif` under `/gifs/`.
#[wasm_bindgen(js_name = odmPathFor)]
pub fn odm_path_for(gif_path: &str) -> Option<String> {
    layout::bucketed_path(gif_path)
}

/// `filelist.odm.txt` from `filelist.txt`: every line whose bucketed `.odm`
/// is in `present` (a JS string array) is switched to it. Returns
/// `{text, switched}`.
#[wasm_bindgen(js_name = odmManifest)]
pub fn odm_manifest(manifest_text: &str, present: Array) -> JsValue {
    let present: HashSet<String> = present.iter().filter_map(|v| v.as_string()).collect();
    let (text, switched) = curate::odm_manifest(manifest_text, &present);
    let o = Object::new();
    set(&o, "text", text);
    set(&o, "switched", switched as u32);
    o.into()
}

// --- Per-entry overrides (the playlist's `.meta`) --------------------------

/// One override as the device's `GET /api/overrides` sends it:
/// `{path, never?, w?, dwell?, dwell_x?, loops?}` — absent fields are left
/// out, `dwell` is seconds and `dwell_x` a multiple of the configured dwell.
fn override_object(path: &str, ov: &Override) -> Object {
    let o = Object::new();
    set(&o, "path", path);
    if ov.never {
        set(&o, "never", true);
    }
    if let Some(w) = ov.weight {
        set(&o, "w", w);
    }
    match ov.dwell {
        Some(Dwell::Secs(s)) => set(&o, "dwell", s),
        Some(Dwell::Times(k)) => set(&o, "dwell_x", k),
        None => {}
    }
    if let Some(n) = ov.loops {
        set(&o, "loops", u32::from(n));
    }
    o
}

/// The inverse of [`override_object`], range-checked as the card parser
/// would check a line; `None` if it overrides nothing valid.
fn override_from(v: &JsValue) -> Option<MetaLine> {
    let path = get(v, "path").as_string().filter(|p| !p.is_empty())?;
    let num = |k: &str| get(v, k).as_f64();
    let ov = Override {
        never: get(v, "never").as_bool() == Some(true),
        weight: num("w").map(|w| w as f32),
        dwell: match (num("dwell"), num("dwell_x")) {
            (Some(s), _) if s >= 0.0 && s <= u32::MAX as f64 => Some(Dwell::Secs(s as u32)),
            (_, Some(k)) => Some(Dwell::Times(k as f32)),
            _ => None,
        },
        loops: num("loops")
            .filter(|n| *n >= 0.0 && *n <= u16::MAX as f64)
            .map(|n| n as u16),
    };
    (ov.is_valid() && !ov.is_empty()).then_some(MetaLine { path, ov })
}

/// A playlist's `.meta` text -> `[{path, never?, w?, dwell?, dwell_x?,
/// loops?}]`, tolerant like the device.
#[wasm_bindgen(js_name = parseOverrides)]
pub fn parse_overrides(text: &str) -> Array {
    parse_meta(text)
        .iter()
        .map(|l| JsValue::from(override_object(&l.path, &l.ov)))
        .collect()
}

/// `[{path, ...}]` -> the canonical `.meta` text. Entries that override
/// nothing (or nothing valid) are dropped.
#[wasm_bindgen(js_name = renderOverrides)]
pub fn render_overrides(list: Array) -> String {
    let lines: Vec<MetaLine> = list.iter().filter_map(|v| override_from(&v)).collect();
    render_meta(&lines)
}

fn stats_object(s: &EncodeStats) -> Object {
    let o = Object::new();
    set(&o, "width", s.width as u32);
    set(&o, "height", s.height as u32);
    set(&o, "frames", s.frames);
    set(&o, "palette_len", s.palette_len as u32);
    set(&o, "quantised", s.quantised);
    set(&o, "gif_bytes", s.gif_bytes as f64);
    set(&o, "odm_bytes", s.odm_bytes as f64);
    set(&o, "keep_px", s.keep_px as f64);
    set(&o, "fill_px", s.fill_px as f64);
    set(&o, "lit_px", s.lit_px as f64);
    o
}

/// GIF bytes -> ODM v1 with the core's encoder (the one `opendmd-pack` runs,
/// so the output is byte-identical). Returns `{odm: Uint8Array, stats}`;
/// throws on a GIF the player could not play either.
#[wasm_bindgen(js_name = encodeGif)]
pub fn encode_gif(gif: &[u8]) -> Result<JsValue, JsValue> {
    let mut out = Vec::with_capacity(gif.len());
    let stats = odm::encode_gif(gif, &mut out).map_err(|e| JsValue::from_str(&e.to_string()))?;
    let o = Object::new();
    set(&o, "odm", Uint8Array::from(&out[..]));
    set(&o, "stats", stats_object(&stats));
    Ok(o.into())
}

/// The renderer's default bloom, so the page's slider starts where the
/// simulator's `--bloom` does rather than duplicating the constant in JS.
#[wasm_bindgen(js_name = defaultBloom)]
pub fn default_bloom() -> f32 {
    BLOOM_STRENGTH
}

// --- The preview player -------------------------------------------------

/// A panel-look player bound to one `<canvas>`. The LED scale comes from the
/// canvas element's `width` at construction (1280 wide is scale 10); the
/// renderer lives as long as the object, and [`Preview::load`] swaps the file
/// underneath it.
#[wasm_bindgen]
pub struct Preview {
    player: Player,
    ctx: web_sys::CanvasRenderingContext2d,
    /// Exponential moving average of the render+blit cost.
    frame_ms: f64,
    error: String,
}

fn now_ms() -> f64 {
    web_sys::window()
        .and_then(|w| w.performance())
        .map_or(0.0, |p| p.now())
}

fn at(ms: f64) -> Duration {
    Duration::from_secs_f64(ms.max(0.0) / 1000.0)
}

#[wasm_bindgen]
impl Preview {
    #[wasm_bindgen(constructor)]
    pub fn new(
        canvas: web_sys::HtmlCanvasElement,
        gamma: f32,
        gain: f32,
        bloom: f32,
        flat: bool,
    ) -> Result<Preview, JsValue> {
        console_error_panic_hook::set_once();
        let scale = (canvas.width() as usize / opendmd_core::canvas::WIDTH).clamp(1, 32);
        let player = Player::new(
            scale,
            Look {
                gamma,
                gain,
                bloom,
                flat,
            },
        );
        let (w, h) = player.size();
        canvas.set_width(w);
        canvas.set_height(h);
        let ctx: web_sys::CanvasRenderingContext2d = canvas
            .get_context("2d")?
            .ok_or_else(|| JsValue::from_str("no 2d context"))?
            .dyn_into()?;
        let p = Preview {
            player,
            ctx,
            frame_ms: 0.0,
            error: String::new(),
        };
        p.blit()?;
        Ok(p)
    }

    /// Play `bytes` (a GIF or an `.odm`) as `path`. The bytes are copied
    /// into wasm memory once and the previous file is dropped.
    pub fn load(&mut self, path: &str, bytes: &[u8]) {
        self.error.clear();
        self.player.load(path, bytes.to_vec(), at(now_ms()));
    }

    /// Stop and blank the panel.
    pub fn clear(&mut self) {
        self.error.clear();
        self.player.unload();
        let _ = self.blit();
    }

    /// One animation-frame step: 0 nothing drawn, 1 a frame, 2 a frame after
    /// the dwell ran out (see [`Preview::set_dwell`]), -1 an error (read
    /// [`Preview::error`]; the file is dropped).
    pub fn frame(&mut self) -> i32 {
        let t0 = now_ms();
        match self.player.tick(at(t0)) {
            Ok(Tick::Idle) => 0,
            Ok(tick) => {
                if self.blit().is_err() {
                    return -1;
                }
                let cost = now_ms() - t0;
                self.frame_ms = if self.frame_ms == 0.0 {
                    cost
                } else {
                    self.frame_ms * 0.9 + cost * 0.1
                };
                if tick == Tick::DwellEnded {
                    2
                } else {
                    1
                }
            }
            Err(e) => {
                self.error = e;
                -1
            }
        }
    }

    /// Give files the firmware's dwell (seconds): at least one loop and
    /// `min`, at most `max`; then [`Preview::frame`] returns 2. `min <= 0`
    /// loops the file forever, which is the default.
    #[wasm_bindgen(js_name = setDwell)]
    pub fn set_dwell(&mut self, min: f64, max: f64) {
        self.player.set_dwell((min > 0.0).then(|| {
            (
                Duration::from_secs_f64(min),
                Duration::from_secs_f64(max.max(min)),
            )
        }));
    }

    /// The panel was `elapsed_ms` into this file at `at_ms` on this page's
    /// `performance.now()` clock, advancing `rate` ms per ms: follow it from
    /// now on (see `Player::sync_to`). Returns the drift before correction,
    /// positive when the preview was ahead, or NaN with nothing loaded.
    #[wasm_bindgen(js_name = syncTo)]
    pub fn sync_to(&mut self, elapsed_ms: f64, at_ms: f64, rate: f64) -> f64 {
        self.player
            .sync_to(at(elapsed_ms), at_ms, rate, now_ms())
            .unwrap_or(f64::NAN)
    }

    #[wasm_bindgen(getter)]
    pub fn synced(&self) -> bool {
        self.player.is_synced()
    }

    #[wasm_bindgen(js_name = setPaused)]
    pub fn set_paused(&mut self, paused: bool) {
        self.player.set_paused(paused, at(now_ms()));
    }

    #[wasm_bindgen(getter)]
    pub fn paused(&self) -> bool {
        self.player.is_paused()
    }

    /// Slider or checkbox moved: rebuild the renderer and redraw at once, so
    /// the change is visible even while paused or between frames.
    #[wasm_bindgen(js_name = setLook)]
    pub fn set_look(&mut self, gamma: f32, gain: f32, bloom: f32, flat: bool) {
        self.player.set_look(Look {
            gamma,
            gain,
            bloom,
            flat,
        });
        self.player.draw();
        let _ = self.blit();
    }

    #[wasm_bindgen(getter)]
    pub fn path(&self) -> String {
        self.player.path().to_string()
    }

    #[wasm_bindgen(getter)]
    pub fn error(&self) -> String {
        self.error.clone()
    }

    /// Smoothed milliseconds per rendered frame (render + `putImageData`).
    #[wasm_bindgen(getter, js_name = frameMs)]
    pub fn frame_ms(&self) -> f64 {
        self.frame_ms
    }

    fn blit(&self) -> Result<(), JsValue> {
        let (w, h) = self.player.size();
        let image = web_sys::ImageData::new_with_u8_clamped_array_and_sh(
            Clamped(self.player.rgba()),
            w,
            h,
        )?;
        self.ctx.put_image_data(&image, 0.0, 0.0)
    }
}

#[cfg(test)]
mod tests;

//! The user's tags and stars (`/sets/.tags`, `opendmd_core::tags`), the
//! search (`opendmd_core::query`: the Library's words, a smart playlist's)
//! and the web app's path-keyed lists (`/sets/.smart`,
//! `opendmd_core::sidecar`), for the page. Here rather than in the main
//! module, which is at its size budget; loaded only by a Library whose card
//! has tags, a search with words like `tag:` or `rating>=4`, and the
//! playlist editor's Add by filter.

use js_sys::{Array, Object, Reflect};
use opendmd_core::query::{parse_query, Query};
use opendmd_core::sidecar;
use opendmd_core::tags::TagSet;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

fn set(o: &Object, k: &str, v: impl Into<JsValue>) {
    let _ = Reflect::set(o, &JsValue::from_str(k), &v.into());
}

fn get(o: &JsValue, k: &str) -> JsValue {
    Reflect::get(o, &JsValue::from_str(k)).unwrap_or(JsValue::UNDEFINED)
}

/// The card's `/sets/.tags`, held parsed: what the user said about each
/// GIF. The page keeps one and writes `render()` back.
#[wasm_bindgen]
pub struct TagBook {
    set: TagSet,
}

#[wasm_bindgen]
impl TagBook {
    #[wasm_bindgen(constructor)]
    pub fn new(text: &str) -> TagBook {
        TagBook {
            set: TagSet::from_text(text),
        }
    }

    /// `{r, t}` for a GIF (`r` 0 when unrated, `t` its tags), or
    /// `undefined` when nothing is said about it.
    pub fn get(&self, path: &str) -> JsValue {
        let Some(l) = self.set.get(path) else {
            return JsValue::UNDEFINED;
        };
        let o = Object::new();
        set(&o, "r", u32::from(l.rating.unwrap_or(0)));
        set(
            &o,
            "t",
            l.tags
                .iter()
                .map(|t| JsValue::from_str(t))
                .collect::<Array>(),
        );
        o.into()
    }

    /// Replace what is said about `path`: `rating` 1..=5 (0: unrated) and
    /// its tags (cleaned; ones that are not tags dropped). Nothing left
    /// removes the line.
    pub fn set(&mut self, path: &str, rating: u8, tags: Array) {
        let tags: Vec<String> = tags.iter().filter_map(|t| t.as_string()).collect();
        self.set.set(path, (rating > 0).then_some(rating), &tags);
    }

    /// A GIF renamed or moved: its line follows.
    pub fn rename(&mut self, from: &str, to: &str) -> bool {
        self.set.rename(from, to)
    }

    /// A GIF deleted: its line goes.
    pub fn forget(&mut self, path: &str) -> bool {
        self.set.drop_path(path)
    }

    /// The canonical file.
    pub fn render(&self) -> String {
        self.set.render()
    }

    #[wasm_bindgen(getter)]
    pub fn size(&self) -> usize {
        self.set.len()
    }

    /// Every tag in use, `[[tag, count]]`, most first.
    pub fn tags(&self) -> Array {
        self.set
            .tag_counts()
            .into_iter()
            .map(|(t, n)| {
                JsValue::from(Array::of2(&JsValue::from_str(&t), &JsValue::from(n as u32)))
            })
            .collect()
    }

    /// Is any GIF rated?
    #[wasm_bindgen(getter)]
    pub fn rated(&self) -> bool {
        self.set.iter().any(|l| l.rating.is_some())
    }

    /// Every path a line names.
    pub fn paths(&self) -> Array {
        self.set
            .iter()
            .map(|l| JsValue::from_str(&l.path))
            .collect()
    }
}

/// A tag as it is kept (lowercased, spaces tidied), or `undefined` when the
/// text is not one.
#[wasm_bindgen(js_name = cleanTag)]
pub fn clean_tag(s: &str) -> Option<String> {
    opendmd_core::tags::clean_tag(s)
}

/// A search as typed. The constructor throws `{message, at}` for one it
/// cannot read.
#[wasm_bindgen]
pub struct Search {
    q: Query,
}

#[wasm_bindgen]
impl Search {
    #[wasm_bindgen(constructor)]
    pub fn new(text: &str) -> Result<Search, JsValue> {
        parse_query(text).map(|q| Search { q }).map_err(|e| {
            let o = Object::new();
            set(&o, "message", e.message.as_str());
            set(&o, "at", e.at as u32);
            o.into()
        })
    }

    /// It tests more than free words.
    #[wasm_bindgen(getter)]
    pub fn keyed(&self) -> bool {
        self.q.is_keyed()
    }

    /// It reads tags or stars.
    #[wasm_bindgen(getter, js_name = usesTags)]
    pub fn uses_tags(&self) -> bool {
        self.q.uses_tags()
    }

    /// Does the GIF at `path` pass, with `book` the card's tags (an empty
    /// one when there are none)?
    pub fn test(&self, path: &str, book: &TagBook) -> bool {
        self.q.matches_path(path, Some(&book.set))
    }

    /// The non-blank lines of a playlist's text (or every GIF's path, one
    /// a line) that pass, shuffled and cut as the search says.
    pub fn run(&self, text: &str, book: &TagBook) -> Array {
        let lines = text
            .strip_prefix('\u{feff}')
            .unwrap_or(text)
            .split('\n')
            .map(|l| l.trim_end_matches(['\r', ' ', '\t']))
            .filter(|l| !l.is_empty());
        self.q
            .run(lines, Some(&book.set))
            .iter()
            .map(|p| JsValue::from_str(p))
            .collect()
    }

    /// The canonical text.
    pub fn render(&self) -> String {
        self.q.render()
    }

    /// What it finds, in words ("Mega Drive · ★ 4+ · tagged boss").
    pub fn describe(&self) -> String {
        self.q.describe()
    }
}

/// A path-keyed list (`/sets/.smart`) -> `[{path, f: {key: value}}]`.
#[wasm_bindgen(js_name = parseSidecar)]
pub fn parse_sidecar(text: &str) -> Array {
    sidecar::parse(text)
        .into_iter()
        .map(|l| {
            let o = Object::new();
            let f = Object::new();
            for (k, v) in &l.fields {
                set(&f, k, v.as_str());
            }
            set(&o, "path", l.path.as_str());
            set(&o, "f", f);
            JsValue::from(o)
        })
        .collect()
}

/// `[{path, f: {key: value}}]` -> the canonical text (sorted by path; empty
/// values and lines dropped).
#[wasm_bindgen(js_name = renderSidecar)]
pub fn render_sidecar(list: Array) -> String {
    let lines: Vec<sidecar::Line> = list
        .iter()
        .filter_map(|v| {
            let path = get(&v, "path").as_string()?;
            let fields = Object::entries(get(&v, "f").dyn_ref::<Object>()?)
                .iter()
                .filter_map(|kv| {
                    let kv = Array::from(&kv);
                    let v = kv.get(1).as_string().filter(|v| !v.is_empty())?;
                    Some((kv.get(0).as_string()?, v))
                })
                .collect();
            Some(sidecar::Line { path, fields })
        })
        .collect();
    sidecar::render(&lines)
}

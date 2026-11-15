//! The bindings' own tests: the prepared copy, Smooth motion and the name
//! builder as the page calls them. The pipeline's are opendmd-art's. Run
//! natively by `cargo test` and compiled to wasm under node by
//! `wasm-pack test --node`. Every picture is made here, in code.

use crate::encode::encode;
use crate::pipeline::*;

#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test as test;

/// A deterministic generator (splitmix64), so fixtures are the same everywhere.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

const PAL: [Px; 8] = [
    [20, 24, 46, 255],
    [255, 255, 255, 255],
    [230, 60, 40, 255],
    [60, 200, 80, 255],
    [250, 210, 30, 255],
    [40, 120, 230, 255],
    [140, 90, 50, 255],
    [180, 180, 200, 255],
];

/// Native pixel art: a flat sky, then blobs and single pixels of a small
/// palette — 1 px detail everywhere, as in a real screenshot.
fn pixel_art(w: usize, h: usize, seed: u64) -> Image {
    let mut r = Rng(seed);
    let mut img = Image::new(w, h);
    img.px.fill(PAL[7]);
    for _ in 0..(w * h / 40) {
        let (x, y) = (r.below(w), r.below(h));
        let c = PAL[r.below(PAL.len())];
        let (bw, bh) = (1 + r.below(4), 1 + r.below(3));
        for yy in y..(y + bh).min(h) {
            for xx in x..(x + bw).min(w) {
                img.px[yy * w + xx] = c;
            }
        }
    }
    img
}

/// `prepareGif` builds JS values, so this only exists on wasm.
#[cfg(target_arch = "wasm32")]
#[test]
fn prepare_gif_matches_the_core_recoder_byte_for_byte() {
    use js_sys::{Reflect, Uint8Array};
    use wasm_bindgen::JsValue;
    let get = |v: &JsValue, k: &str| Reflect::get(v, &JsValue::from_str(k)).unwrap();
    let scroll = render(
        &flatten_pad(&pixel_art(128, 60, 8)),
        &Plan {
            motion: Motion::Scroll,
            ..Plan::default()
        },
    );
    for gif in [
        encode(&scroll),
        encode(&render(&Image::new(W, H), &Plan::default())),
    ] {
        let mut want = Vec::new();
        let stats = opendmd_core::gifslim::recode_gif(
            gif.as_slice(),
            &mut want,
            &opendmd_core::gifslim::PREPARED,
        )
        .unwrap();
        let got = crate::prepare_gif(&gif).map_err(|_| ()).unwrap();
        assert_eq!(Uint8Array::from(get(&got, "copy")).to_vec(), want);
        let st = get(&got, "stats");
        assert_eq!(get(&st, "frames").as_f64(), Some(stats.frames as f64));
        assert_eq!(get(&st, "out_bytes").as_f64(), Some(want.len() as f64));
    }
    assert!(crate::prepare_gif(b"GIF89a but not really").is_err());
    // Past 256 colours: no copy (never a lossy one); the error says the GIF
    // plays as it is, which is what js/prepare.js recognises and records.
    let mut many = Vec::new();
    {
        let mut e = gif::Encoder::new(&mut many, 128, 32, &[]).unwrap();
        for k in 0..3u16 {
            let pal: Vec<u8> = (0..100)
                .flat_map(|i: u16| [(i * 2) as u8, (k * 80) as u8, (255 - i) as u8])
                .collect();
            e.write_frame(&gif::Frame {
                width: 128,
                height: 32,
                palette: Some(pal),
                buffer: std::borrow::Cow::Owned((0..4096).map(|i| (i % 100) as u8).collect()),
                ..Default::default()
            })
            .unwrap();
        }
    }
    let e = crate::prepare_gif(&many).map(|_| ()).unwrap_err();
    let msg: String = js_sys::Error::from(JsValue::from(e)).message().into();
    assert!(opendmd_core::gifslim::plays_original(&msg), "{msg}");
}

/// FNV-1a 64: a fingerprint of bytes, the same on every platform.
fn fnv64(b: &[u8]) -> u64 {
    b.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &x| {
        (h ^ x as u64).wrapping_mul(0x100_0000_01b3)
    })
}

/// A 2 px-per-frame scroll of generated pixel art, 50 ms a frame.
fn two_px_scroll() -> Vec<u8> {
    let stage = flatten_pad(&pixel_art(128, 80, 3));
    let mut anim = render(
        &stage,
        &Plan {
            motion: Motion::Scroll,
            ..Plan::default()
        },
    );
    // Every other frame: 2 px a step.
    anim.frames = anim.frames.into_iter().step_by(2).collect();
    anim.delays_ms = vec![50; anim.frames.len()];
    encode(&anim)
}

/// Smooth motion: stepwise == one call, marked, and the same bytes natively
/// and in wasm (the fingerprint is pinned: a change here is a change in
/// every smoothed copy on every card).
#[test]
fn smoothing_is_deterministic_and_stepwise_equals_one_call() {
    let gif = two_px_scroll();
    let mut s = crate::Smoother::new(&gif).map_err(|_| ()).unwrap();
    while !s.analyse(3).map_err(|_| ()).unwrap() {}
    assert!(s.offered());
    assert!(s.tweened() > 0 && s.added() == s.tweened());
    while !s.tween(4).map_err(|_| ()).unwrap() {}
    let copy = s.prepare().map_err(|_| ()).unwrap();
    let once = crate::prepare_smooth_gif(&gif).map_err(|_| ()).unwrap();
    assert_eq!(copy, once);
    let m = opendmd_core::tween::marker(&copy).unwrap();
    assert_eq!((m.pairs as u32, m.added as u32), (s.tweened(), s.added()));
    let mut native = Vec::new();
    opendmd_core::tween::smooth_gif(&gif, &mut native, &opendmd_core::gifslim::PREPARED).unwrap();
    assert_eq!(native, copy);
    assert_eq!(fnv64(&copy), SMOOTH_FINGERPRINT, "{:#x}", fnv64(&copy));
}

const SMOOTH_FINGERPRINT: u64 = 0x232e_36c6_5c2b_7b7d;

/// The wording's hint: the same scroll at 1 px a frame (50 ms) has nothing
/// to tween and says it is already smooth ("steady"); at 2 px it is offered.
#[test]
fn a_one_px_scroll_is_steady_and_a_two_px_one_offered() {
    let stage = flatten_pad(&pixel_art(128, 80, 3));
    let mut anim = render(
        &stage,
        &Plan {
            motion: Motion::Scroll,
            ..Plan::default()
        },
    );
    anim.delays_ms = vec![50; anim.frames.len()];
    let analysed = |gif: &[u8]| {
        let mut s = crate::Smoother::new(gif).map_err(|_| ()).unwrap();
        while !s.analyse(5).map_err(|_| ()).unwrap() {}
        s
    };
    let s = analysed(&encode(&anim));
    assert_eq!((s.tweened(), s.added(), s.offered()), (0, 0, false));
    assert_eq!(s.hint(), "steady");
    assert_eq!(analysed(&two_px_scroll()).hint(), "offered");
}

/// The browser's `nameFields` / `buildName` are the core's
/// `NameFields::from_path` / `path`: over real card names (and a few
/// refusals), the JS objects go through and come back as the core's paths
/// and error codes. JS values exist only in wasm, so this runs under node
/// (`make test-wasm`); the core's own tests cover the grammar natively.
#[cfg(target_arch = "wasm32")]
#[test]
fn build_name_in_the_browser_is_the_cores() {
    use opendmd_core::manifest::NameFields;
    use wasm_bindgen::JsValue;
    for path in [
        "/gifs/Consoles/SNES_Donkey_Kong_Country_05.gif",
        "/gifs/Consoles/MD_Sonic3_spritefox11.gif",
        "/gifs/Consoles/SNES_Asterix01.gif",
        "/gifs/NEW 2021/NINTENDO_DS_Mario_Kart_DS_01..gif",
        "/gifs/Consoles/NES_Unknow_01_Invert.gif",
        "/gifs/Halloween/HALLOWEEN_ARCADE_GoldenAxe16.gif",
        "/gifs/Consoles/MD_LOGO_Sega31.gif",
        "/gifs/Consoles/PSP_Parasite_Eve_01__pixelkid42.gif",
        "/gifs/Spectrum_Slide/Exolon2.gif",
        "/gifs/Computers/PC_Leilanis Island_42.gif",
        "/gifs/Consoles/GAMEGEAR_Columns_01_bd.azerty.p.gif",
        "/gifs/Consoles/VIRTUAL_BOY_Mario_Clash_01.gif",
        "/gifs/Other/OTHER_037.gif",
    ] {
        let want = NameFields::from_path(path).path().unwrap();
        let got = crate::build_name(&crate::name_fields(path)).unwrap();
        assert_eq!(got, want, "{path}");
    }
    let obj = |title: &str, platform: Option<&str>, ordinal: Option<u32>, author: Option<&str>| {
        let o = js_sys::Object::new();
        crate::js_set(&o, "title", title.into());
        crate::js_set(
            &o,
            "platform",
            platform.map_or(JsValue::NULL, JsValue::from),
        );
        crate::js_set(&o, "category", "Consoles".into());
        crate::js_set(&o, "ordinal", ordinal.map_or(JsValue::NULL, JsValue::from));
        crate::js_set(&o, "author", author.map_or(JsValue::NULL, JsValue::from));
        JsValue::from(o)
    };
    assert_eq!(
        crate::build_name(&obj("Super Metroid", Some("SNES"), Some(1), None)).unwrap(),
        "/gifs/Consoles/SNES_Super_Metroid_01.gif"
    );
    for (o, code) in [
        (obj("", Some("SNES"), Some(1), None), "title"),
        (obj("Toy Story", Some("SNES"), Some(1), None), "reads-back"),
        (obj("Art", None, Some(1), None), "needs-platform"),
        (obj("Art", Some("SNES"), Some(1), Some("ab")), "author"),
    ] {
        assert_eq!(
            crate::build_name(&o).unwrap_err().as_string().as_deref(),
            Some(code)
        );
    }
    assert!(crate::platforms().includes(&"VIRTUAL_BOY".into(), 0));
    assert!(!crate::platforms().includes(&"NINTENDO_DS".into(), 0));
}

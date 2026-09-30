//! Run natively by `cargo test` and compiled to wasm under node by
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

fn upscale(img: &Image, k: usize) -> Image {
    let mut out = Image::new(img.w * k, img.h * k);
    for y in 0..out.h {
        for x in 0..out.w {
            out.px[y * out.w + x] = img.at(x / k, y / k);
        }
    }
    out
}

fn colours(img: &Image) -> Vec<u32> {
    let mut v: Vec<u32> = img.px.iter().map(|p| u32::from_be_bytes(*p)).collect();
    v.sort_unstable();
    v.dedup();
    v
}

#[test]
fn native_scale_undoes_2x_and_3x() {
    let art = pixel_art(120, 70, 1);
    assert_eq!(native_scale(&art).k, 1, "native pixels are not an upscale");
    for k in [2, 3] {
        let big = upscale(&art, k);
        let s = native_scale(&big);
        assert_eq!(s, Scale { k, x0: 0, y0: 0 }, "{k}x");
        assert_eq!(native(&big, s), art, "{k}x recovered exactly");
    }
    // A 3x copy cropped off the grid (2 px off the left, 1 off the top):
    // the phase is found and the partial blocks dropped.
    let big = upscale(&art, 3);
    let off = big.crop(2, 1, big.w - 2, big.h - 1);
    let s = native_scale(&off);
    assert_eq!(s, Scale { k: 3, x0: 1, y0: 2 });
    assert_eq!(native(&off, s), art.crop(1, 1, art.w - 1, art.h - 1));
}

#[test]
fn native_scale_refuses_smooth_and_flat_pictures() {
    let art = pixel_art(80, 48, 2);
    // A 2.5x area-resampled copy has no integer grid.
    let smooth = resize_area(&art, 200, 120);
    assert_eq!(native_scale(&smooth).k, 1);
    assert_eq!(native_scale(&Image::new(64, 64)).k, 1);
    // A 2x copy of big flat tiles (edges only every 16 px) is 2x, not 8x.
    let mut tiles = Image::new(64, 40);
    for y in 0..40 {
        for x in 0..64 {
            let t = (x / 8 + y / 8) % PAL.len();
            tiles.px[y * 64 + x] = if (x + y) % 7 == 0 { PAL[1] } else { PAL[t] };
        }
    }
    let s = native_scale(&upscale(&tiles, 2));
    assert_eq!(s.k, 2);
    assert_eq!(native(&upscale(&tiles, 2), s), tiles);
}

#[test]
fn best_window_finds_the_busy_part() {
    let mut img = Image::new(400, 200);
    img.px.fill([90, 150, 230, 255]);
    let patch = pixel_art(W, H, 3);
    for y in 0..H {
        for x in 0..W {
            img.px[(y + 120) * 400 + x + 200] = patch.at(x, y);
        }
    }
    assert_eq!(best_window(&img, W, H, 0.15), (200, 120));
    // Smaller than the window: the origin.
    assert_eq!(best_window(&Image::new(50, 20), W, H, 0.15), (0, 0));
    // A flat picture: the centre.
    assert_eq!(best_window(&Image::new(256, 64), W, H, 0.15), (64, 16));
    // A pan starts from the left on the busiest band; a scroll from the
    // top on the busiest column.
    assert_eq!(auto_window(&img, Motion::Pan), (0, 120));
    assert_eq!(auto_window(&img, Motion::Scroll), (200, 0));
    assert_eq!(auto_window(&img, Motion::Still), (200, 120));
}

#[test]
fn down_mode_keeps_source_colours() {
    let art = pixel_art(64, 40, 4);
    // A 2x copy comes back exactly.
    assert_eq!(down_mode(&upscale(&art, 2), 2), art);
    // No invented colours, ever (area averaging would invent blends).
    let half = down_mode(&art, 2);
    assert_eq!((half.w, half.h), (32, 20));
    let src = colours(&art);
    assert!(colours(&half).iter().all(|c| src.contains(c)));
    // The commonest colour of the block wins.
    let block = |p: [Px; 4]| Image {
        w: 2,
        h: 2,
        px: p.to_vec(),
    };
    let (red, black, white, grey) = (
        [200, 0, 0, 255],
        [0, 0, 0, 255],
        [255, 255, 255, 255],
        [120, 120, 120, 255],
    );
    assert_eq!(down_mode(&block([red, black, red, white]), 2).px, vec![red]);
    // Four different colours: the one furthest from the block's mean (an
    // outline against its fills), not the first.
    assert_eq!(
        down_mode(&block([grey, black, red, [110, 100, 100, 255]]), 2).px,
        vec![black]
    );
}

/// A textbook median cut that splits the most populous box, as count-driven
/// quantisers do — the reference that loses accents.
fn median_cut(px: &[[i32; 3]], n: usize) -> Vec<[i32; 3]> {
    let mut boxes: Vec<Vec<[i32; 3]>> = vec![px.to_vec()];
    while boxes.len() < n {
        let (i, _) = boxes
            .iter()
            .enumerate()
            .max_by_key(|(_, b)| b.len())
            .unwrap();
        let mut b = boxes.swap_remove(i);
        let range = |c: usize| {
            b.iter().map(|p| p[c]).max().unwrap() - b.iter().map(|p| p[c]).min().unwrap()
        };
        let axis = (0..3).max_by_key(|&c| range(c)).unwrap();
        b.sort_by_key(|p| p[axis]);
        let hi = b.split_off(b.len() / 2);
        boxes.push(b);
        boxes.push(hi);
    }
    boxes
        .iter()
        .map(|b| {
            let s = b
                .iter()
                .fold([0; 3], |a, p| [a[0] + p[0], a[1] + p[1], a[2] + p[2]]);
            let n = b.len() as i32;
            [s[0] / n, s[1] / n, s[2] / n]
        })
        .collect()
}

#[test]
fn spread_pick_keeps_a_one_pixel_accent_median_cut_drops() {
    // 60 shades of foliage over 64x64, and one yellow coin pixel.
    let mut img = Image::new(64, 64);
    let mut r = Rng(5);
    for p in &mut img.px {
        let s = r.below(60) as u8;
        *p = [20 + s / 2, 80 + 2 * s, 30 + s / 3, 255];
    }
    let coin = [255u8, 220, 0, 255];
    img.px[64 * 30 + 33] = coin;
    let coin_key = 0xFFDC00;
    let picked = spread_pick(&colour_counts(&img), 16);
    assert_eq!(picked.len(), 16);
    assert!(picked.contains(&coin_key), "spread pick keeps the coin");
    let px: Vec<[i32; 3]> = img
        .px
        .iter()
        .map(|p| [p[0] as i32, p[1] as i32, p[2] as i32])
        .collect();
    let cut = median_cut(&px, 16);
    let nearest = cut
        .iter()
        .map(|c| (c[0] - 255).pow(2) + (c[1] - 220).pow(2) + (c[2]).pow(2))
        .min()
        .unwrap();
    assert!(
        nearest > 80 * 80,
        "median cut has nothing near the coin (d² {nearest})"
    );
    // And through render: the coin pixel is still yellow in the GIF's frame.
    let mut stage = Image::new(W, H);
    stage.px = stage_px(&img);
    let anim = render(
        &stage,
        &Plan {
            colours: Colours::Max(16),
            ..Plan::default()
        },
    );
    assert_eq!(anim.palette.len(), 16);
    assert!(anim.palette.contains(&coin_key));
}

/// `img`'s left 64 columns, rows 16..48, twice side by side: 128x32 with the coin in it.
fn stage_px(img: &Image) -> Vec<Px> {
    let half = img.crop(0, 16, W / 2, H);
    let mut px = Vec::with_capacity(W * H);
    for y in 0..H {
        px.extend_from_slice(&half.px[y * 64..y * 64 + 64]);
        px.extend_from_slice(&half.px[y * 64..y * 64 + 64]);
    }
    px
}

#[test]
fn spread_pick_is_exact_when_it_fits() {
    let art = pixel_art(64, 32, 6);
    let counts = colour_counts(&art);
    assert_eq!(spread_pick(&counts, 256).len(), counts.len());
    assert_eq!(spread_pick(&counts, 3).len(), 3);
    // The most used colour is always first.
    assert_eq!(spread_pick(&counts, 3)[0], counts[0].0);
}

#[test]
fn lift_dark_flips_strokes_but_not_a_backing_plate() {
    // Dark grey lettering on transparency: lifted.
    let mut logo = Image::new(60, 20);
    logo.px.fill([0, 0, 0, 0]);
    for y in 4..16 {
        for x in (6..54).step_by(4) {
            logo.px[y * 60 + x] = [40, 40, 44, 255];
        }
    }
    logo.px[10 * 60 + 30] = [230, 40, 40, 255];
    let up = lift_dark(&logo);
    assert_eq!(up.at(6, 4), [215, 215, 211, 255]);
    assert_eq!(up.at(30, 10), [230, 40, 40, 255], "colour is left alone");
    assert_eq!(up.at(0, 0), [0, 0, 0, 0], "transparency is left alone");
    // A black plate (most of the box) with a red and a green letter: kept.
    let mut plate = Image::new(40, 40);
    plate.px.fill([10, 10, 10, 255]);
    for i in 0..8 {
        plate.px[20 * 40 + 5 + i] = [220, 30, 30, 255];
        plate.px[25 * 40 + 5 + i] = [40, 200, 60, 255];
    }
    assert_eq!(lift_dark(&plate), plate);
}

#[test]
fn smooth_fit_sizes_logos() {
    assert_eq!(smooth_size(400, 100), (120, 30));
    assert_eq!(smooth_size(100, 100), (30, 30));
    // Long: full height, to pan.
    assert_eq!(smooth_size(1000, 100), (320, 32));
    // A logo with a transparent border: trimmed, fitted and centred on black.
    let mut logo = Image::new(300, 200);
    logo.px.fill([0, 0, 0, 0]);
    for y in 50..150 {
        for x in 50..250 {
            logo.px[y * 300 + x] = [255, 255, 255, 255];
        }
    }
    let st = stage(&logo, native_scale(&logo), Fit::Smooth, false);
    assert_eq!((st.w, st.h), (W, H));
    // 200x100 -> 60x30, centred: columns 34..94, rows 1..31.
    assert_eq!(st.at(34, 1), [255, 255, 255, 255]);
    assert_eq!(st.at(93, 30), [255, 255, 255, 255]);
    assert_eq!(st.at(33, 1), [0, 0, 0, 255]);
    assert_eq!(st.at(34, 0), [0, 0, 0, 255]);
}

#[test]
fn resize_area_averages_premultiplied() {
    let img = Image {
        w: 4,
        h: 1,
        px: vec![
            [200, 0, 0, 255],
            [200, 0, 0, 255],
            [255, 255, 255, 0],
            [0, 0, 255, 255],
        ],
    };
    let out = resize_area(&img, 2, 1);
    assert_eq!(out.px[0], [200, 0, 0, 255]);
    // Half transparent white + blue: blue at half alpha (no white fringe).
    assert_eq!(out.px[1], [0, 0, 255, 128]);
}

#[test]
fn key_background_blacks_out_the_commonest_colour() {
    let mut img = Image::new(10, 10);
    img.px.fill([100, 180, 240, 255]);
    img.px[3] = [104, 176, 250, 255]; // within 12: sky too
    img.px[4] = [120, 180, 240, 255]; // not
    img.px[5] = [250, 200, 20, 255];
    assert_eq!(key_background(&mut img, 12), Some([100, 180, 240]));
    assert_eq!(img.px[0], [0, 0, 0, 255]);
    assert_eq!(img.px[3], [0, 0, 0, 255]);
    assert_eq!(img.px[4], [120, 180, 240, 255]);
    assert_eq!(img.px[5], [250, 200, 20, 255]);
}

#[test]
fn pan_and_scroll_frames_and_delays() {
    // Pan: y is kept, x runs 0..=w-128 one pixel a frame.
    let (rect, wins) = windows(200, 40, Motion::Pan, 17, 5);
    assert_eq!(rect, (0, 5, 200, H));
    assert_eq!(wins.len(), 73);
    assert!(wins.iter().enumerate().all(|(i, &w)| w == (i, 5)));
    let d = delays(wins.len(), Motion::Pan, 40, 1500);
    assert_eq!((d[0], d[1], d[71], d[72]), (1500, 40, 40, 1500));
    assert_eq!(d.iter().sum::<u32>(), 3000 + 71 * 40);
    // Scroll: x is kept (clamped), y runs 0..=h-32.
    let (rect, wins) = windows(128, 100, Motion::Scroll, 99, 50);
    assert_eq!(rect, (0, 0, W, 100));
    assert_eq!(wins, (0..=68).map(|y| (0, y)).collect::<Vec<_>>());
    // Holds never shorter than a step; no holds -> all steps.
    assert_eq!(delays(3, Motion::Scroll, 30, 0), vec![30, 30, 30]);
    // A still, and a move with nowhere to go, are one 5 s frame.
    let (rect, wins) = windows(300, 50, Motion::Still, 250, 40);
    assert_eq!((rect, wins.clone()), ((172, 18, W, H), vec![(172, 18)]));
    assert_eq!(delays(1, Motion::Still, 40, 1500), vec![STILL_MS]);
    assert_eq!(delays(1, Motion::Pan, 40, 1500), vec![STILL_MS]);
}

#[test]
fn render_crops_the_stage_frame_by_frame() {
    let art = pixel_art(180, 32, 7);
    let st = flatten_pad(&art);
    let plan = Plan {
        motion: Motion::Pan,
        step_ms: 30,
        ..Plan::default()
    };
    let anim = render(&st, &plan);
    assert_eq!(anim.frames.len(), 53);
    assert_eq!(
        anim.palette.len(),
        colour_counts(&st).len(),
        "exact palette"
    );
    for i in [0, 1, 26, 52] {
        let want: Vec<u8> = st
            .crop(i, 0, W, H)
            .px
            .iter()
            .flat_map(|p| [p[0], p[1], p[2]])
            .collect();
        assert_eq!(anim.rgb(i), want, "frame {i}");
    }
    assert_eq!(anim.duration_ms(), 3000 + 51 * 30);
    // Brightness halves every channel; the background key takes the sky.
    let dim = render(
        &st,
        &Plan {
            gain: 50,
            key: true,
            ..Plan::default()
        },
    );
    assert!(dim.palette.contains(&0));
    let sky = u32::from_be_bytes([0, PAL[7][0], PAL[7][1], PAL[7][2]]);
    assert!(!dim.palette.contains(&sky));
    assert!(dim.palette.iter().all(|&c| (c >> 16) as u8 <= 128));
}

#[test]
fn gif_round_trips_and_prepares_without_requantising() {
    let art = pixel_art(128, 60, 8);
    let st = flatten_pad(&art);
    let anim = render(
        &st,
        &Plan {
            motion: Motion::Scroll,
            ..Plan::default()
        },
    );
    assert_eq!(anim.frames.len(), 29);
    let gif = encode(&anim);
    let mut opts = gif::DecodeOptions::new();
    opts.set_color_output(gif::ColorOutput::RGBA);
    let mut dec = opts.read_info(gif.as_slice()).unwrap();
    let mut i = 0;
    while let Some(f) = dec.read_next_frame().unwrap() {
        assert_eq!((f.width, f.height), (W as u16, H as u16));
        let rgb: Vec<u8> = f
            .buffer
            .chunks(4)
            .flat_map(|p| [p[0], p[1], p[2]])
            .collect();
        assert_eq!(rgb, anim.rgb(i), "frame {i}");
        assert_eq!(f.delay as u32 * 10, anim.delays_ms[i]);
        i += 1;
    }
    assert_eq!(i, 29);
    // The prepared copy keeps these colours exactly.
    let prepared = &opendmd_core::gifslim::PREPARED;
    let stats = opendmd_core::gifslim::recode_gif(gif.as_slice(), Vec::new(), prepared).unwrap();
    assert!(!stats.quantised);
    assert_eq!(stats.frames, 29);
    assert_eq!(stats.colours, anim.palette.len());
    // One colour still makes a valid file.
    let one = render(&Image::new(W, H), &Plan::default());
    assert_eq!(one.palette, vec![0]);
    assert!(
        opendmd_core::gifslim::recode_gif(encode(&one).as_slice(), Vec::new(), prepared).is_ok()
    );
}

#[test]
fn auto_fit_and_motion() {
    let art = pixel_art(160, 144, 9);
    assert_eq!(
        auto_fit(&upscale(&art, 2), native_scale(&upscale(&art, 2))),
        Fit::Pixels
    );
    let mut logo = Image::new(300, 100);
    logo.px.fill([0, 0, 0, 0]);
    for y in 30..70 {
        for x in 20..280 {
            logo.px[y * 300 + x] = [250, 250, 250, 255];
        }
    }
    assert_eq!(auto_fit(&logo, native_scale(&logo)), Fit::Smooth);
    assert_eq!(auto_motion(400, 32), Motion::Pan);
    assert_eq!(auto_motion(128, 430), Motion::Scroll);
    assert_eq!(auto_motion(256, 224), Motion::Still);
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
        assert_eq!(get(&st, "quantised").as_bool(), Some(false));
    }
    assert!(crate::prepare_gif(b"GIF89a but not really").is_err());
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
        "/gifs/Consoles/MD_Sonic3_RattenJager.gif",
        "/gifs/Consoles/SNES_Asterix01.gif",
        "/gifs/NEW 2021/NINTENDO_DS_Mario_Kart_DS_01..gif",
        "/gifs/Consoles/NES_Unknow_01_Invert.gif",
        "/gifs/Halloween/HALLOWEEN_ARCADE_GoldenAxe16.gif",
        "/gifs/Consoles/MD_LOGO_Sega31.gif",
        "/gifs/Consoles/PSP_Parasite_Eve_01__sephirot68.gif",
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

//! Run natively by `cargo test` and compiled to wasm under node by
//! `wasm-pack test --node`. The tests of the `#[wasm_bindgen]` exports that
//! build JS values only exist on wasm (`js-sys` has no host implementation).

use super::*;
use gif::{DisposalMethod, Encoder, Frame, Repeat};
use opendmd_core::canvas::{Rgb, HEIGHT, WIDTH};
use std::borrow::Cow;

#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test as test;

/// The page's defaults.
const LOOK: Look = Look {
    gamma: 2.2,
    gain: 1.0,
    bloom: BLOOM_STRENGTH,
    flat: false,
};

/// A 128x32 GIF of `frames` frames, each a single white pixel at (4, 2)
/// shown for 50 ms.
fn white_pixel_gif(frames: usize) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut enc = Encoder::new(&mut out, WIDTH as u16, HEIGHT as u16, &[]).unwrap();
        enc.set_repeat(Repeat::Infinite).unwrap();
        for _ in 0..frames {
            let mut f = Frame {
                left: 4,
                top: 2,
                width: 1,
                height: 1,
                buffer: Cow::Owned(vec![1u8]),
                dispose: DisposalMethod::Keep,
                delay: 5,
                ..Default::default()
            };
            f.palette = Some(vec![0, 0, 0, 255, 255, 255]);
            enc.write_frame(&f).unwrap();
        }
    }
    out
}

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

#[test]
fn card_paths_are_reconstructed_from_what_the_browser_gives_us() {
    assert_eq!(
        card_path("dmdcard/gifs/Consoles/SNES_Plok_08.gif"),
        "/gifs/Consoles/SNES_Plok_08.gif"
    );
    assert_eq!(
        card_path("gifs/NEW 2021/MD_Sonic3.gif"),
        "/gifs/NEW 2021/MD_Sonic3.gif"
    );
    assert_eq!(
        card_path("Arcade/ARCADE_Pac-Man_01.gif"),
        "/gifs/Arcade/ARCADE_Pac-Man_01.gif"
    );
    assert_eq!(
        card_path("NES_Gradius.gif"),
        "/gifs/Unknown/NES_Gradius.gif"
    );
    assert_eq!(
        card_path("gifs/NES_Gradius.gif"),
        "/gifs/Unknown/NES_Gradius.gif"
    );
}

/// The page's whole path, headless: bytes in, a lit pixel in the rendered
/// RGBA buffer out. Same `Engine` and `LedRenderer` the browser drives; only
/// `ImageData` is missing.
#[test]
fn a_gif_plays_through_the_engine_and_lights_a_pixel() {
    let scale = 10;
    let mut player = Player::new(scale, LOOK);
    assert_eq!(
        player.size(),
        ((WIDTH * scale) as u32, (HEIGHT * scale) as u32)
    );
    assert_eq!(player.tick(ms(0)), Ok(Tick::Idle), "nothing loaded yet");
    player.load("/gifs/Consoles/SNES_Plok_08.gif", white_pixel_gif(1), ms(0));
    assert_eq!(player.path(), "/gifs/Consoles/SNES_Plok_08.gif");
    assert_eq!(player.tick(ms(0)), Ok(Tick::Frame));

    // The LED renderer puts the emitter's peak at the centre of the cell.
    let w = WIDTH * scale;
    let centre = ((2 * scale + scale / 2) * w + 4 * scale + scale / 2) * 4;
    let rgba = player.rgba();
    assert_eq!(
        &rgba[centre..centre + 4],
        &[255, 255, 255, 255],
        "the lit LED did not reach the framebuffer"
    );
    // Its neighbour a pitch and a half away is still dark.
    let far = ((2 * scale + scale / 2) * w + 4 * scale + 2 * scale) * 4;
    assert!(rgba[far] < 20, "bloom reaches too far: {}", rgba[far]);
}

/// Four 50 ms frames, frame `i` lighting (i, 0) on top of the ones before:
/// the lit pixels on row 0 are the frame's number within the loop.
fn counting_gif() -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut enc = Encoder::new(&mut out, WIDTH as u16, HEIGHT as u16, &[]).unwrap();
        enc.set_repeat(Repeat::Infinite).unwrap();
        for x in 0..4u16 {
            let mut f = Frame {
                left: x,
                width: 1,
                height: 1,
                buffer: Cow::Owned(vec![1u8]),
                dispose: DisposalMethod::Keep,
                delay: 5,
                ..Default::default()
            };
            f.palette = Some(vec![0, 0, 0, 255, 255, 255]);
            enc.write_frame(&f).unwrap();
        }
    }
    out
}

fn lit_in_row0(p: &Player) -> usize {
    (0..WIDTH)
        .filter(|&x| p.canvas().get(x, 0) != Rgb::BLACK)
        .count()
}

/// The Now view's preview: told where the panel is, it shows the panel's
/// frame, ignores sub-frame noise, holds when a little ahead, starts over
/// when far ahead, follows the panel's rate, and holds the panel's frame
/// while paused.
#[test]
fn a_synced_preview_shows_the_panels_frame() {
    let mut p = Player::new(1, Look { flat: true, ..LOOK });
    p.load("/gifs/X/A.gif", counting_gif(), ms(0));
    assert!(!p.is_synced());
    // The panel is 520 ms in (third loop, 400..600: frame 3 of it is
    // 500..550) as of page time 1000; nothing was shown here yet.
    assert_eq!(p.sync_to(ms(520), 1000.0, 1.0, 1000.0), Some(-520.0));
    assert!(p.is_synced());
    assert_eq!(p.tick(ms(1000)), Ok(Tick::Frame));
    assert_eq!(lit_in_row0(&p), 3);
    assert_eq!(p.tick(ms(1010)), Ok(Tick::Idle), "same frame until 550");
    assert_eq!(p.tick(ms(1030)), Ok(Tick::Frame));
    assert_eq!(lit_in_row0(&p), 4);

    // 3 ms of request noise: under a frame, nothing moves.
    assert_eq!(p.sync_to(ms(557), 1040.0, 1.0, 1040.0), Some(3.0));
    // 170 ms ahead (the panel ran late): hold the frame until it catches up
    // rather than start over.
    assert_eq!(p.sync_to(ms(400), 1050.0, 1.0, 1050.0), Some(170.0));
    assert_eq!(p.tick(ms(1050)), Ok(Tick::Idle));
    assert_eq!(p.tick(ms(1240)), Ok(Tick::Idle), "the panel is at 590");
    assert_eq!(lit_in_row0(&p), 4);
    assert_eq!(p.tick(ms(1250)), Ok(Tick::Frame), "600: the next loop");
    assert_eq!(lit_in_row0(&p), 1);
    // Far ahead (the panel started the file again): start over and seek.
    assert_eq!(
        p.tick(ms(3000)),
        Ok(Tick::Frame),
        "2350: 4th frame of a loop"
    );
    assert_eq!(lit_in_row0(&p), 4);
    assert_eq!(p.sync_to(ms(100), 3000.0, 1.0, 3000.0), Some(2250.0));
    assert_eq!(p.tick(ms(3000)), Ok(Tick::Frame));
    assert_eq!(lit_in_row0(&p), 3);

    // Paused: the frame is held however long, and a report while paused
    // moves it to the panel's.
    p.set_paused(true, ms(3010));
    assert_eq!(p.tick(ms(5000)), Ok(Tick::Idle));
    assert_eq!(lit_in_row0(&p), 3);
    assert_eq!(p.sync_to(ms(180), 5000.0, 1.0, 5000.0), Some(110.0 - 180.0));
    assert_eq!(p.tick(ms(5001)), Ok(Tick::Frame));
    assert_eq!(lit_in_row0(&p), 4);
    // Resumed, it carries on from the held position.
    p.set_paused(false, ms(6000));
    assert_eq!(p.tick(ms(6020)), Ok(Tick::Frame));
    assert_eq!(lit_in_row0(&p), 1);

    // A panel whose position runs at 0.9x wall time: at 1000 ms of page
    // time past a report of 0 it is at 900, the fourth loop's third frame.
    p.load("/gifs/X/B.gif", counting_gif(), ms(7000));
    assert!(
        !p.is_synced(),
        "a new file is free-running until it is synced"
    );
    p.sync_to(ms(0), 7000.0, 0.9, 7000.0);
    assert_eq!(p.tick(ms(8000)), Ok(Tick::Frame));
    assert_eq!(lit_in_row0(&p), 3);
}

#[test]
fn a_preview_loops_forever_and_a_dwell_reports_its_end() {
    let mut player = Player::new(2, LOOK);
    player.load("/gifs/X/A.gif", white_pixel_gif(2), ms(0));
    // Loop forever: a minute of 50 ms frames, never a dwell end.
    let mut t = 0;
    while t < 60_000 {
        assert_ne!(player.tick(ms(t)), Ok(Tick::DwellEnded), "at {t} ms");
        t += 50;
    }
    // With a 1 s dwell the engine re-opens its only entry after ~1 s.
    player.set_dwell(Some((ms(1000), ms(5000))));
    player.load("/gifs/X/A.gif", white_pixel_gif(2), ms(t));
    let start = t;
    loop {
        if player.tick(ms(t)) == Ok(Tick::DwellEnded) {
            break;
        }
        t += 50;
        assert!(t - start < 3000, "dwell never ended");
    }
    assert!(t - start >= 1000);
}

#[test]
fn swapping_files_keeps_the_player_and_blanks_the_canvas() {
    let mut player = Player::new(2, Look { flat: true, ..LOOK });
    player.load("/gifs/X/A.gif", white_pixel_gif(1), ms(0));
    player.tick(ms(0)).unwrap();
    assert_eq!(player.canvas().get(4, 2), Rgb::new(255, 255, 255));
    player.load("/gifs/X/B.gif", b"not a gif".to_vec(), ms(10));
    assert_eq!(player.canvas().get(4, 2), Rgb::new(0, 0, 0));
    let err = player.tick(ms(10)).unwrap_err();
    assert!(err.contains("/gifs/X/B.gif"), "{err}");
    assert_eq!(player.path(), "", "a bad file is dropped");
    assert_eq!(player.tick(ms(20)), Ok(Tick::Idle));
}

#[test]
fn pause_holds_the_frame_across_a_load() {
    let mut player = Player::new(2, LOOK);
    player.set_paused(true, ms(0));
    player.load("/gifs/X/A.gif", white_pixel_gif(1), ms(0));
    assert_eq!(player.tick(ms(100)), Ok(Tick::Idle));
    player.set_paused(false, ms(200));
    assert_eq!(player.tick(ms(200)), Ok(Tick::Frame));
}

#[test]
fn an_odm_plays_like_its_gif() {
    let gif = white_pixel_gif(3);
    let mut odm_bytes = Vec::new();
    odm::encode_gif(&gif[..], &mut odm_bytes).unwrap();
    let mut a = Player::new(1, Look { flat: true, ..LOOK });
    let mut b = Player::new(1, Look { flat: true, ..LOOK });
    a.load("/gifs/X/A.gif", gif, ms(0));
    b.load("/odm/X/00/A.odm", odm_bytes, ms(0));
    for t in (0..500).step_by(50) {
        assert_eq!(a.tick(ms(t)), b.tick(ms(t)));
        assert_eq!(a.rgba(), b.rgba(), "at {t} ms");
    }
}

#[test]
fn the_flat_renderer_blits_the_raw_canvas() {
    let scale = 10;
    let mut player = Player::new(scale, Look { flat: true, ..LOOK });
    player.load("/gifs/X/SNES_Plok_08.gif", white_pixel_gif(1), ms(0));
    player.tick(ms(0)).unwrap();
    let w = WIDTH * scale;
    let corner = ((2 * scale) * w + 4 * scale) * 4;
    assert_eq!(&player.rgba()[corner..corner + 3], &[255, 255, 255]);
    assert_eq!(player.canvas().get(4, 2), Rgb::new(255, 255, 255));
}

#[test]
fn bucket_for_is_the_pack_rule() {
    assert_eq!(bucket_for("MD_Sonic3_RattenJager.odm"), "90");
    assert_eq!(bucket_for("SNES_Plok_08.odm"), "c0");
    assert_eq!(
        odm_path_for("/gifs/Spectrum_Slide/Exolon2.gif").as_deref(),
        Some("/odm/Spectrum_Slide/20/Exolon2.odm")
    );
    assert_eq!(odm_path_for("/odm/X/00/A.odm"), None);
}

#[test]
fn normalize_title_is_the_cores() {
    assert_eq!(
        normalize_title("Donkey Kong-Country 2!"),
        "donkeykongcountry2"
    );
}

// --- The JS-value exports: wasm only --------------------------------------

#[cfg(target_arch = "wasm32")]
mod js {
    use super::*;
    use wasm_bindgen_test::wasm_bindgen_test as test;

    fn s(v: &JsValue, k: &str) -> JsValue {
        get(v, k)
    }

    #[test]
    fn parse_path_returns_the_entry_as_an_object() {
        let e = parse_path("/gifs/Consoles/HALLOWEEN_ARCADE_GoldenAxe_16.gif");
        assert_eq!(s(&e, "category").as_string().as_deref(), Some("Consoles"));
        assert_eq!(s(&e, "platform").as_string().as_deref(), Some("ARCADE"));
        assert_eq!(s(&e, "title").as_string().as_deref(), Some("GoldenAxe"));
        assert_eq!(s(&e, "variant").as_f64(), Some(16.0));
        let flags = Array::from(&s(&e, "flags"));
        assert_eq!(flags.get(0).as_string().as_deref(), Some("HALLOWEEN"));
        // Absent fields are null, not missing.
        let e = parse_path("/gifs/Other/Nes_Gradius.gif");
        assert!(s(&e, "platform").is_null());
        assert!(s(&e, "variant").is_null());
        // A bucketed .odm has its GIF's tags, bucket skipped.
        let e = parse_path("/odm/NEW 2021/3a/SNES_Plok_08.odm");
        assert_eq!(s(&e, "category").as_string().as_deref(), Some("NEW 2021"));
        assert_eq!(s(&e, "platform").as_string().as_deref(), Some("SNES"));
        assert_eq!(
            parse_manifest("/gifs/A/MD_X.gif\n\n/gifs/B/SNES_Y.gif\n").length(),
            2
        );
    }

    #[test]
    fn filter_manifest_takes_json() {
        let text = "/gifs/Consoles/SNES_Plok_08.gif\n/gifs/Consoles/MD_Sonic3_RattenJager.gif\n/gifs/Arcade/ARCADE_Pac-Man_01.gif\n";
        let got = filter_manifest(text, r#"{"platforms":["MD","SNES"]}"#).unwrap();
        assert_eq!(got.length(), 2);
        let got = filter_manifest(text, r#"{"platforms":"arcade","title":"pac man"}"#).unwrap();
        assert_eq!(got.length(), 1);
        assert_eq!(
            got.get(0).as_string().as_deref(),
            Some("/gifs/Arcade/ARCADE_Pac-Man_01.gif")
        );
        let all = filter_manifest(text, "").unwrap();
        assert_eq!(all.length(), 3);
        let shuffled = filter_manifest(text, r#"{"shuffle":true,"seed":3,"limit":2}"#).unwrap();
        assert_eq!(shuffled.length(), 2);
        assert!(filter_manifest(text, "{nope").is_err());
    }

    #[test]
    fn encode_gif_matches_the_core_encoder_byte_for_byte() {
        let gif = white_pixel_gif(3);
        let mut want = Vec::new();
        let stats = odm::encode_gif(&gif[..], &mut want).unwrap();
        let got = encode_gif(&gif).unwrap();
        let bytes = Uint8Array::from(s(&got, "odm")).to_vec();
        assert_eq!(bytes, want);
        let st = s(&got, "stats");
        assert_eq!(s(&st, "frames").as_f64(), Some(stats.frames as f64));
        assert_eq!(s(&st, "odm_bytes").as_f64(), Some(want.len() as f64));
        assert!(encode_gif(b"GIF89a but not really").is_err());
    }

    #[test]
    fn overrides_round_trip_through_js_objects() {
        let text = "/gifs/Consoles/SNES_Plok_08.gif\tw=3\n\
                    /gifs/NEW 2021/X.gif\tnever\n\
                    /gifs/A/Y.gif\tdwell=45\tloops=3\n\
                    /gifs/A/Z.gif\tw=0.5\tdwell=2x\n";
        let list = parse_overrides(text);
        assert_eq!(list.length(), 4);
        let fav = list.get(0);
        assert_eq!(s(&fav, "w").as_f64(), Some(3.0));
        assert!(s(&fav, "never").is_undefined());
        assert_eq!(s(&list.get(1), "never").as_bool(), Some(true));
        assert_eq!(s(&list.get(2), "dwell").as_f64(), Some(45.0));
        assert_eq!(s(&list.get(2), "loops").as_f64(), Some(3.0));
        assert_eq!(s(&list.get(3), "dwell_x").as_f64(), Some(2.0));
        assert_eq!(render_overrides(list.clone()), text);
        // Nothing valid to say: dropped. Out of range: dropped.
        let o = Object::new();
        set(&o, "path", "/gifs/A/W.gif");
        list.push(&o);
        let bad = Object::new();
        set(&bad, "path", "/gifs/A/V.gif");
        set(&bad, "loops", 0);
        list.push(&bad);
        assert_eq!(render_overrides(list), text);
    }

    #[test]
    fn odm_manifest_switches_present_lines() {
        let present = Array::new();
        present.push(&JsValue::from_str("/odm/Spectrum_Slide/20/Exolon2.odm"));
        let r = odm_manifest(
            "/gifs/Spectrum_Slide/Exolon2.gif\n/gifs/Spectrum_Slide/Abadia_Crimen.gif\n",
            present,
        );
        assert_eq!(
            s(&r, "text").as_string().as_deref(),
            Some("/odm/Spectrum_Slide/20/Exolon2.odm\n/gifs/Spectrum_Slide/Abadia_Crimen.gif\n")
        );
        assert_eq!(s(&r, "switched").as_f64(), Some(1.0));
    }
}

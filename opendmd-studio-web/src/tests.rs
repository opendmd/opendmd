//! Run natively by `cargo test` and compiled to wasm under node by
//! `wasm-pack test --node`. Every picture is made here, in code.

use crate::encode::*;

#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test as test;

#[test]
fn indexed_frames_keep_their_palette_exactly() {
    let pal = [0x000000, 0x0f380f, 0x306230, 0x8bac0f, 0x9bbc0f];
    let mut frames = vec![0u8; 2 * W * H];
    frames[5 * W + 7] = 4;
    frames[W * H + 31 * W + 127] = 2;
    let gif = indexed(&pal, &frames, &[120, 240], 0).unwrap();
    let mut opts = gif::DecodeOptions::new();
    opts.set_color_output(gif::ColorOutput::Indexed);
    let mut dec = opts.read_info(gif.as_slice()).unwrap();
    assert_eq!((dec.width(), dec.height()), (W as u16, H as u16));
    let table: Vec<u32> = dec
        .global_palette()
        .unwrap()
        .chunks(3)
        .map(|c| (c[0] as u32) << 16 | (c[1] as u32) << 8 | c[2] as u32)
        .collect();
    // (The table is padded to a power of two with black.)
    assert_eq!(table[..pal.len()], pal);
    let mut got = Vec::new();
    while let Some(f) = dec.read_next_frame().unwrap() {
        got.push((f.buffer.to_vec(), f.delay));
    }
    assert_eq!(got.len(), 2);
    assert_eq!(got[0], (frames[..W * H].to_vec(), 12));
    assert_eq!(got[1], (frames[W * H..].to_vec(), 24));
    // One colour still makes a valid file; bad calls are refused.
    assert!(indexed(&[0], &vec![0; W * H], &[100], 1).is_ok());
    assert!(indexed(&[], &vec![0; W * H], &[100], 0).is_err());
    assert!(indexed(&pal, &vec![0; W * H - 1], &[100], 0).is_err());
    assert!(indexed(&pal, &vec![5; W * H], &[100], 0).is_err());
    assert!(indexed(&[0; 257], &vec![0; W * H], &[100], 0).is_err());
}

/// A still of `n` colours (n <= 4096), each used about as often, and its GIF through the player's decoder:
/// the screen after each GIF frame and the delays.
fn wide_play(n: usize, ms: u32) -> (Vec<u32>, Vec<Vec<u32>>, Vec<std::time::Duration>) {
    // Colour c used (c % 7) + 1 times more, so the most used are not simply the first.
    let mut px = Vec::with_capacity(W * H);
    let mut c = 0;
    while px.len() < W * H {
        for _ in 0..=(c % 7) {
            px.push(((c % n) as u32) * 0x3fff % 0xff_ffff);
        }
        c += 1;
    }
    px.truncate(W * H);
    let gif = wide(&px, &[ms], 0).unwrap();
    let (shots, delays) = play(&gif);
    (px, shots, delays)
}

/// A GIF through the player's decoder: the screen after each GIF frame, and the delays.
fn play(gif: &[u8]) -> (Vec<Vec<u32>>, Vec<std::time::Duration>) {
    use opendmd_core::canvas::Canvas;
    use opendmd_core::gifslim::GifSlim;
    let mut dec = GifSlim::new(std::io::Cursor::new(gif)).unwrap();
    let mut canvas = Canvas::new();
    let (mut shots, mut delays) = (Vec::new(), Vec::new());
    while let Some(d) = dec.next_frame(&mut canvas).unwrap() {
        let rgb = dec.screen(&canvas);
        shots.push(
            rgb.chunks(3)
                .map(|c| (c[0] as u32) << 16 | (c[1] as u32) << 8 | c[2] as u32)
                .collect(),
        );
        delays.push(d);
    }
    (shots, delays)
}

#[test]
fn wide_frames_paint_the_whole_picture_first_then_correct_it_exactly() {
    use opendmd_core::gifslim::{recode_gif, ORIGINAL_REASON, PREPARED};
    use std::time::Duration;
    let (px, shots, delays) = wide_play(1024, 5000);
    let mut distinct = px.clone();
    distinct.sort_unstable();
    distinct.dedup();
    assert_eq!(distinct.len(), 1024);
    // GIF frame 0 is the whole picture over its 256 most used colours (nothing left black).
    let (table, idx) = fit(&px);
    assert_eq!(table.len(), 256);
    let base: Vec<u32> = idx.iter().map(|&i| table[i as usize]).collect();
    assert_eq!(shots[0], base);
    // Then the corrections, 255 colours each; at the end every pixel exact.
    assert_eq!(shots.len(), 1 + (1024 - 256_usize).div_ceil(255));
    assert_eq!(shots.last().unwrap(), &px);
    // 20 ms each but the last, which holds the rest: 5000 ms in all.
    assert!(delays[..delays.len() - 1]
        .iter()
        .all(|&d| d == Duration::from_millis(20)));
    assert_eq!(delays.iter().sum::<Duration>(), Duration::from_millis(5000));
    // No prepared copy: only a lossy one could be made, so the GIF plays as it is.
    let gif = wide(&px, &[5000], 0).unwrap();
    let err = recode_gif(gif.as_slice(), &mut Vec::new(), &PREPARED).unwrap_err();
    assert!(err.to_string().contains(ORIGINAL_REASON), "{err}");
    // Bad calls are refused.
    assert!(wide(&px[1..], &[100], 0).is_err());
    assert!(wide(&vec![0x0100_0000; W * H], &[100], 0).is_err());
    assert!(wide(&px, &[], 0).is_err());
}

#[test]
fn wide_frames_at_the_edges() {
    use std::time::Duration;
    // 256 colours: one GIF frame, exact. 257: one correction of one colour.
    let (px, shots, delays) = wide_play(256, 5000);
    assert_eq!((shots.len(), &shots[0]), (1, &px));
    assert_eq!(delays, [Duration::from_millis(5000)]);
    let (px, shots, _) = wide_play(257, 5000);
    assert_eq!(shots.len(), 2);
    assert_ne!(shots[0], px);
    assert_eq!(shots[1], px);
    let (px, shots, _) = wide_play(256 + 255, 5000);
    assert_eq!((shots.len(), shots.last().unwrap()), (2, &px));
    let (_, shots, _) = wide_play(256 + 256, 5000);
    assert_eq!(shots.len(), 3);
    // A frame shorter than 20 ms a GIF frame lasts 20 ms a GIF frame.
    let (px, shots, delays) = wide_play(700, 30);
    assert_eq!((shots.len(), shots.last().unwrap()), (3, &px));
    assert_eq!(delays.iter().sum::<Duration>(), Duration::from_millis(60));
}

#[test]
fn fitted_frames_keep_the_most_used_colours_and_the_nearest_for_the_rest() {
    let (px, _, _) = wide_play(300, 5000);
    let gif = fitted(&px, &[5000], 0).unwrap();
    let mut opts = gif::DecodeOptions::new();
    opts.set_color_output(gif::ColorOutput::RGBA);
    let mut dec = opts.read_info(gif.as_slice()).unwrap();
    let f = dec.read_next_frame().unwrap().unwrap().buffer.to_vec();
    let got: Vec<u32> = f
        .chunks(4)
        .map(|c| (c[0] as u32) << 16 | (c[1] as u32) << 8 | c[2] as u32)
        .collect();
    let (table, idx) = fit(&px);
    assert_eq!(
        got,
        idx.iter().map(|&i| table[i as usize]).collect::<Vec<_>>()
    );
    assert!(dec.read_next_frame().unwrap().is_none());
    // It is a GIF the player can prepare.
    assert!(opendmd_core::gifslim::recode_gif(
        gif.as_slice(),
        &mut Vec::new(),
        &opendmd_core::gifslim::PREPARED
    )
    .is_ok());
}

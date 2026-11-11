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
    // GIF frame 0 is the whole picture over 256 of its colours (nothing left black).
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
fn fitted_frames_keep_256_colours_and_the_nearest_for_the_rest() {
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

#[test]
fn fitted_frames_up_to_256_colours_are_exact() {
    for n in [1, 2, 255, 256] {
        let (px, _, _) = wide_play(n, 100);
        let gif = fitted(&px, &[100], 0).unwrap();
        let mut opts = gif::DecodeOptions::new();
        opts.set_color_output(gif::ColorOutput::RGBA);
        let mut dec = opts.read_info(gif.as_slice()).unwrap();
        let f = dec.read_next_frame().unwrap().unwrap().buffer.to_vec();
        let got: Vec<u32> = f
            .chunks(4)
            .map(|c| (c[0] as u32) << 16 | (c[1] as u32) << 8 | c[2] as u32)
            .collect();
        assert_eq!(got, px, "{n} colours");
    }
}

/// The squared distance from `c` to the nearest of `table`.
fn gap(table: &[u32], c: u32) -> i32 {
    let ch = |a: u32, s: u32| ((a >> s) & 255) as i32;
    table
        .iter()
        .map(|&t| (0..3).map(|k| (ch(c, 8 * k) - ch(t, 8 * k)).pow(2)).sum())
        .min()
        .unwrap()
}

#[test]
fn the_256_are_picked_for_spread_not_by_use() {
    // 4,096 colours, each used once (a flat histogram): the 16 x 16 x 16
    // grid of 0, 17, ... 255, in a scrambled order.
    let grid = |i: usize| {
        let s = |v: usize| (v as u32 & 15) * 17;
        s(i >> 8) << 16 | s(i >> 4) << 8 | s(i)
    };
    let px: Vec<u32> = (0..W * H).map(|i| grid(i * 1031 % 4096)).collect();
    let counts = by_use(&px);
    assert_eq!(counts.len(), 4096);
    assert!(counts.iter().all(|&(_, n)| n == 1));
    let (table, idx) = fit(&px);
    let mut t = table.clone();
    t.sort_unstable();
    t.dedup();
    assert_eq!(t.len(), 256);
    assert!(table.iter().all(|c| counts.iter().any(|d| d.0 == *c)));
    // Not the first (lowest) 256, nor the most used (ties: the lowest
    // again): those are the red = 0 face of the cube, every one of them.
    let first: Vec<u32> = counts[..256].iter().map(|c| c.0).collect();
    assert!(first.iter().all(|&c| c >> 16 == 0));
    assert!(table.iter().filter(|&&c| c >> 16 == 0).count() < 64);
    // Every eighth of the cube gets about its share (32).
    let mut octants = [0; 8];
    for &c in &table {
        octants[((c >> 23 & 1) << 2 | (c >> 15 & 1) << 1 | (c >> 7 & 1)) as usize] += 1;
    }
    assert!(
        octants.iter().all(|&n| (16..=48).contains(&n)),
        "{octants:?}"
    );
    // And no colour is far from one picked: the worst is within 48 (the best 256 could do is ~35); the
    // first 256 leave white 255 away.
    let worst = |tab: &[u32]| (0..4096).map(|i| gap(tab, grid(i))).max().unwrap();
    let (spread, by_first) = (worst(&table), worst(&first));
    assert!(spread <= 48 * 48, "{spread}");
    assert!(by_first >= 255 * 255);
    // Each pixel is drawn as its nearest in the table.
    for (&c, &i) in px.iter().zip(&idx) {
        assert_eq!(gap(&[table[i as usize]], c), gap(&table, c));
    }
}

#[test]
fn a_rare_colour_far_from_the_rest_is_kept() {
    // 4,095 pixels of 300 dark near-twins, the most used by far, and one
    // yellow pixel: by use, yellow would be the first dropped.
    let mut px: Vec<u32> = (0..W * H - 1)
        .map(|i| ((i % 300) as u32 / 20) << 16 | (i % 20) as u32)
        .collect();
    px.push(0xffd700);
    let (table, idx) = fit(&px);
    assert_eq!(table.len(), 256);
    assert_eq!(table[idx[W * H - 1] as usize], 0xffd700);
}

#[test]
fn wide_frames_of_4096_colours_then_fewer_end_exact() {
    use std::time::Duration;
    let grid: Vec<u32> = (0..W * H)
        .map(|i| {
            let s = |v: usize| (v as u32 & 15) * 17;
            let j = i * 1031 % 4096;
            s(j >> 8) << 16 | s(j >> 4) << 8 | s(j)
        })
        .collect();
    let (few, _, _) = wide_play(300, 100);
    let gif = wide(&[grid.clone(), few.clone()].concat(), &[1000, 100], 0).unwrap();
    let (shots, delays) = play(&gif);
    // 4,096: a base and 15 corrections of 255 and one of 15; then 300: a base and one.
    assert_eq!(shots.len(), 17 + 2);
    assert_eq!(shots[16], grid);
    assert_eq!(shots[18], few);
    assert_eq!(
        delays[..17].iter().sum::<Duration>(),
        Duration::from_millis(1000)
    );
    assert_eq!(
        delays[17..].iter().sum::<Duration>(),
        Duration::from_millis(100)
    );
    // The base of the first is already close: every pixel within 48 of its own colour.
    assert!(grid
        .iter()
        .zip(&shots[0])
        .all(|(&c, &b)| gap(&[b], c) <= 48 * 48));
}

#[test]
fn a_much_used_colour_beats_a_lone_one_slightly_farther() {
    // After black (the most used), a red used 50 times 100 away and a red
    // used once 110 away: the weight, log(1 + uses), takes the first;
    // distance alone would take the second.
    let counts = [(0x000000, 100), (0x640000, 50), (0x6e0000, 1)];
    assert_eq!(spread_pick(&counts, 2), [0x000000, 0x640000]);
    // With room for all three, all are kept.
    assert_eq!(spread_pick(&counts, 3).len(), 3);
}

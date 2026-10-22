//! The animated source (`anim`) and the timeline (`pipeline::sequence`):
//! natively and under node. Every GIF is made here, in code, with an
//! explicit palette (index 0 black) so the composites are exact.

use crate::anim::{scan_frames, Anim, BUDGET};
use crate::pipeline::*;
use gif::{DisposalMethod, Encoder, Frame, Repeat};
use std::borrow::Cow;

#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test as test;

const PALETTE: [[u8; 3]; 6] = [
    [0, 0, 0],
    [200, 0, 0],
    [0, 200, 0],
    [0, 0, 200],
    [255, 255, 255],
    [90, 90, 90],
];

/// One frame to write: rectangle, indices, delay (hundredths), disposal.
struct F {
    rect: (u16, u16, u16, u16),
    px: Vec<u8>,
    delay: u16,
    dispose: DisposalMethod,
}

fn full(w: u16, h: u16, k: u8, delay: u16) -> F {
    F {
        rect: (0, 0, w, h),
        px: vec![k; w as usize * h as usize],
        delay,
        dispose: DisposalMethod::Keep,
    }
}

fn gif(w: u16, h: u16, frames: &[F], repeat: Option<Repeat>) -> Vec<u8> {
    let pal: Vec<u8> = PALETTE.iter().flatten().copied().collect();
    let mut out = Vec::new();
    {
        let mut e = Encoder::new(&mut out, w, h, &pal).unwrap();
        if let Some(r) = repeat {
            e.set_repeat(r).unwrap();
        }
        for f in frames {
            e.write_frame(&Frame {
                left: f.rect.0,
                top: f.rect.1,
                width: f.rect.2,
                height: f.rect.3,
                buffer: Cow::Borrowed(&f.px),
                delay: f.delay,
                dispose: f.dispose,
                ..Frame::default()
            })
            .unwrap();
        }
    }
    out
}

fn open_all(bytes: Vec<u8>) -> Anim {
    let mut a = Anim::open(bytes).unwrap();
    while !a.decode(3) {}
    a
}

fn px(a: &Anim, i: usize, x: usize, y: usize) -> [u8; 3] {
    let p = a.image(i).at(x, y);
    [p[0], p[1], p[2]]
}

fn edit(n: usize) -> Edit {
    Edit::all(n)
}

#[test]
fn sequence_trim_delete_reverse_pingpong() {
    let d = [100, 40, 40, 30, 20, 60];
    let order = |e: &Edit| sequence(&d, e).iter().map(|s| s.0).collect::<Vec<_>>();
    assert_eq!(order(&edit(6)), [0, 1, 2, 3, 4, 5]);
    let trim = Edit {
        from: 1,
        to: 4,
        ..edit(6)
    };
    assert_eq!(order(&trim), [1, 2, 3, 4]);
    let del = Edit {
        deleted: vec![2, 9],
        ..trim.clone()
    };
    assert_eq!(order(&del), [1, 3, 4]);
    let rev = Edit {
        reverse: true,
        ..del.clone()
    };
    assert_eq!(order(&rev), [4, 3, 1]);
    let pp = Edit {
        pingpong: true,
        ..trim.clone()
    };
    assert_eq!(
        order(&pp),
        [1, 2, 3, 4, 3, 2],
        "there and back, ends not repeated"
    );
    let both = Edit {
        pingpong: true,
        reverse: true,
        ..trim.clone()
    };
    assert_eq!(order(&both), [4, 3, 2, 1, 2, 3]);
    // Two frames: ping-pong changes nothing; a backwards trim is the same range.
    assert_eq!(
        order(&Edit {
            from: 3,
            to: 2,
            pingpong: true,
            ..edit(6)
        }),
        [2, 3]
    );
    // Everything deleted: nothing left.
    assert!(sequence(
        &d,
        &Edit {
            deleted: (0..6).collect(),
            ..edit(6)
        }
    )
    .is_empty());
}

#[test]
fn sequence_speed_rounds_to_10ms_and_floors_at_20() {
    let d = [100, 40, 30, 20, 70];
    let delays = |speed| {
        sequence(&d, &Edit { speed, ..edit(5) })
            .iter()
            .map(|s| s.1)
            .collect::<Vec<_>>()
    };
    assert_eq!(delays(100), [100, 40, 30, 20, 70]);
    assert_eq!(
        delays(200),
        [50, 20, 20, 20, 40],
        "70/2 = 35 -> 40 (to the nearest 10), 15 -> 20 floor"
    );
    assert_eq!(
        delays(400),
        [30, 20, 20, 20, 20],
        "100/4 = 25 -> 30 (halves round up)"
    );
    assert_eq!(delays(50), [200, 80, 60, 40, 140]);
    assert_eq!(delays(25), [400, 160, 120, 80, 280]);
    assert_eq!(
        delays(150),
        [70, 30, 20, 20, 50],
        "100/1.5 = 66.7 -> 70; 26.7 -> 30; 46.7 -> 50"
    );
    let total = |speed| {
        sequence(
            &d,
            &Edit {
                speed,
                pingpong: true,
                ..edit(5)
            },
        )
        .iter()
        .map(|s| s.1)
        .sum::<u32>()
    };
    assert_eq!(total(100), 100 + 40 + 30 + 20 + 70 + 20 + 30 + 40);
    assert_eq!(total(200), 50 + 20 + 20 + 20 + 40 + 20 + 20 + 20);
    // The player's rules on the way in: 0 -> 100 ms, never under 20.
    assert_eq!(
        (
            player_delay(0),
            player_delay(1),
            player_delay(2),
            player_delay(7)
        ),
        (100, 20, 20, 70)
    );
}

#[test]
fn import_composites_background_and_previous_disposal() {
    // An 8x4 screen: red; a green 2x2 at (1,1) disposed to background; a
    // blue 2x2 at (4,0) disposed to previous; a white pixel at (0,0).
    let frames = [
        full(8, 4, 1, 10),
        F {
            rect: (1, 1, 2, 2),
            px: vec![2; 4],
            delay: 0,
            dispose: DisposalMethod::Background,
        },
        F {
            rect: (4, 0, 2, 2),
            px: vec![3; 4],
            delay: 1,
            dispose: DisposalMethod::Previous,
        },
        F {
            rect: (0, 0, 1, 1),
            px: vec![4],
            delay: 5,
            dispose: DisposalMethod::Keep,
        },
    ];
    let bytes = gif(8, 4, &frames, Some(Repeat::Infinite));
    assert_eq!(scan_frames(&bytes), 4);
    let a = open_all(bytes);
    assert_eq!(
        (a.len(), a.w, a.h, a.scale.k, a.reduce, a.loops, a.damaged),
        (4, 8, 4, 1, 1, 0, false)
    );
    assert_eq!(a.delays, [100, 100, 20, 50], "0 -> 100 ms, 10 ms -> 20");
    let (r, g, b, w, k) = (
        [200, 0, 0],
        [0, 200, 0],
        [0, 0, 200],
        [255, 255, 255],
        [0, 0, 0],
    );
    assert!((0..8).all(|x| (0..4).all(|y| px(&a, 0, x, y) == r)));
    assert_eq!(
        (
            px(&a, 1, 1, 1),
            px(&a, 1, 2, 2),
            px(&a, 1, 0, 0),
            px(&a, 1, 3, 1)
        ),
        (g, g, r, r)
    );
    // Frame 1's rectangle went back to the background (black) before frame 2.
    assert_eq!(
        (
            px(&a, 2, 1, 1),
            px(&a, 2, 2, 2),
            px(&a, 2, 4, 0),
            px(&a, 2, 5, 1),
            px(&a, 2, 6, 0)
        ),
        (k, k, b, b, r)
    );
    // Frame 2 was put back to what was there before it (the hole stays black).
    assert_eq!(
        (
            px(&a, 3, 4, 0),
            px(&a, 3, 5, 1),
            px(&a, 3, 1, 1),
            px(&a, 3, 0, 0),
            px(&a, 3, 7, 3)
        ),
        (r, r, k, w, r)
    );
}

#[test]
fn import_undoes_an_upscale_and_fits_the_memory_budget() {
    // A 2x copy of 64x32 pixel art, three frames: kept at 64x32.
    let mut small = Vec::new();
    for y in 0..32u16 {
        for x in 0..64u16 {
            small.push(((x / 3 + y / 5 + x * y % 7) % 6) as u8);
        }
    }
    let big: Vec<u8> = (0..64 * 128)
        .map(|i| small[(i / 128 / 2) * 64 + (i % 128) / 2])
        .collect();
    let fr = |d| F {
        rect: (0, 0, 128, 64),
        px: big.clone(),
        delay: d,
        dispose: DisposalMethod::Keep,
    };
    let a = open_all(gif(
        128,
        64,
        &[fr(4), fr(4), fr(4)],
        Some(Repeat::Finite(2)),
    ));
    assert_eq!((a.scale.k, a.w, a.h, a.reduce, a.len()), (2, 64, 32, 1, 3));
    assert_eq!(a.loops, 3, "a loop count of 2 plays three times");
    assert_eq!(px(&a, 2, 10, 10), PALETTE[small[10 * 64 + 10] as usize]);
    // 700x700 (no pixel grid) x 50 frames is 73.5 MB as RGB: halved.
    let (w, h) = (700u16, 700u16);
    let noise: Vec<u8> = (0..w as usize * h as usize)
        .map(|i| ((i as u32).wrapping_mul(2_654_435_761) >> 7) as u8 % 6)
        .collect();
    let mut frames = vec![F {
        rect: (0, 0, w, h),
        px: noise,
        delay: 4,
        dispose: DisposalMethod::Keep,
    }];
    frames.extend((1..50).map(|i| F {
        rect: (i, i, 1, 1),
        px: vec![4],
        delay: 4,
        dispose: DisposalMethod::Keep,
    }));
    let a = open_all(gif(w, h, &frames, None));
    assert_eq!(
        (a.scale.k, a.reduce, a.w, a.h, a.len(), a.loops),
        (1, 2, 350, 350, 50, 0)
    );
    assert!(a.len() * a.w * a.h * 3 <= BUDGET);
}

/// Four black frames at the panel's size.
fn black4() -> Anim {
    let fr = |d| full(W as u16, H as u16, 0, d);
    open_all(gif(
        W as u16,
        H as u16,
        &[fr(10), fr(10), fr(10), fr(10)],
        None,
    ))
}

fn lit(a: &mut Anim, i: usize, rgb: u32) -> usize {
    let r = a.render(i).clone();
    r.idx
        .iter()
        .filter(|&&k| r.palette[k as usize] == rgb)
        .count()
}

#[test]
fn text_overlay_covers_its_frame_range_only() {
    let mut a = black4();
    a.configure(Fit::Pixels, false, 0, 0, false, 100, Colours::Auto);
    let t = Overlay {
        msg: "HI".into(),
        small: false,
        colour: 0xFFD000,
        anchor: 4,
        dx: 0,
        dy: 0,
        from: 1,
        to: 2,
    };
    let mask = t.mask();
    let n = mask.iter().filter(|&&b| b).count();
    assert!(n > 10);
    // Centred: "HI" in 5x7 is 9 px wide, 7 high -> x 59, y 12.
    assert_eq!(t.origin(), (59, 12));
    a.set_text(Some(t.clone()));
    assert_eq!([0, 1, 2, 3].map(|i| lit(&mut a, i, 0xFFD000)), [0, n, n, 0]);
    // Moving it re-renders only what it covered and covers.
    for i in 0..4 {
        a.render(i);
    }
    a.set_text(Some(Overlay {
        from: 2,
        to: 3,
        anchor: 0,
        small: true,
        ..t
    }));
    assert_eq!(
        [0, 1, 2, 3].map(|i| a.is_rendered(i)),
        [true, false, false, false]
    );
    assert_eq!(lit(&mut a, 1, 0xFFD000), 0);
    assert!(lit(&mut a, 3, 0xFFD000) > 0);
    // A Look change drops every rendered frame; the same one keeps them.
    a.configure(Fit::Pixels, false, 0, 0, false, 100, Colours::Auto);
    assert!(a.is_rendered(3));
    a.configure(Fit::Pixels, false, 0, 0, false, 90, Colours::Auto);
    assert!(!(0..4).any(|i| a.is_rendered(i)));
}

#[test]
fn saving_writes_the_edited_frames_and_delays() {
    // Five frames of different colours at the panel's size, 40 ms each but
    // the last 70.
    let frames: Vec<F> = (0..5)
        .map(|i| full(W as u16, H as u16, i as u8 + 1, if i == 4 { 7 } else { 4 }))
        .collect();
    let mut a = open_all(gif(W as u16, H as u16, &frames, Some(Repeat::Infinite)));
    a.configure(Fit::Pixels, false, 0, 0, false, 100, Colours::Auto);
    let e = Edit {
        from: 0,
        to: 4,
        speed: 200,
        reverse: false,
        pingpong: true,
        deleted: vec![1],
    };
    let seq = a.sequence(&e);
    assert_eq!(seq, [(0, 20), (2, 20), (3, 20), (4, 40), (3, 20), (2, 20)]);
    for plays in [0, 1, 3] {
        a.save_begin(seq.clone(), plays);
        while !a.save_step(2) {}
        let bytes = a.saved();
        let mut o = gif::DecodeOptions::new();
        o.set_color_output(gif::ColorOutput::RGBA);
        let mut d = o.read_info(bytes.as_slice()).unwrap();
        let mut got = Vec::new();
        while let Some(f) = d.read_next_frame().unwrap() {
            got.push((f.buffer[0..3].to_vec(), f.delay as u32 * 10));
        }
        let want: Vec<(Vec<u8>, u32)> = seq
            .iter()
            .map(|&(i, ms)| (PALETTE[i + 1].to_vec(), ms))
            .collect();
        assert_eq!(got, want, "{plays} plays");
        let rep = match plays {
            0 => Repeat::Infinite,
            1 => Repeat::Finite(0),
            n => Repeat::Finite(n as u16 - 1),
        };
        assert_eq!(d.repeat(), rep);
        assert_eq!(a.colours, 4, "the frames' own colours, exactly");
        // What the player plays: the same count and delays.
        let s = opendmd_core::gifslim::recode_gif(
            bytes.as_slice(),
            Vec::new(),
            &opendmd_core::gifslim::PREPARED,
        )
        .unwrap();
        assert_eq!((s.frames, s.quantised), (6, false));
    }
}

// --- A live effect as the source ("Save as GIF" in the Effects tab) --------

fn effect_all(path: &str) -> Anim {
    let mut a = Anim::effect(path).unwrap();
    while !a.decode(7) {}
    a
}

#[test]
fn an_effect_becomes_one_loop_at_the_gif_floor() {
    use crate::anim::effect_plan;
    use opendmd_core::fx::Fx;
    let plan = |p: &str| effect_plan(&mut Fx::parse(p).unwrap());
    // 100 fps effects give every other frame at 20 ms; a loop too long for
    // FX_MAX_FRAMES at 20 ms goes to 40; one too long even then is cut.
    assert_eq!(plan("@fx:tunnel"), (240, 20, true));
    assert_eq!(plan("@fx:tunnel?fps=50"), (240, 20, true));
    assert_eq!(plan("@fx:tunnel?fps=25&speed=1"), (240, 40, true));
    assert_eq!(plan("@fx:tunnel?speed=4"), (120, 20, true));
    assert_eq!(plan("@fx:rotozoom"), (240, 40, true));
    assert_eq!(plan("@fx:rotozoom?speed=1"), (240, 40, false));
    assert_eq!(plan("@fx:starfield"), (120, 20, true));
    assert_eq!(plan("@fx:wire?shape=cube"), (240, 20, true));
    assert!(Anim::effect("@fx:nothing").is_none());
    assert!(Anim::effect("@clock").is_none());

    let a = Anim::effect("@fx:rotozoom?speed=1").unwrap();
    assert_eq!((a.in_file, a.len(), a.cut), (240, 1, true));
    let a = effect_all("@fx:starfield?palette=fire");
    assert_eq!(
        (a.len(), a.orig, a.w, a.h, a.loops, a.cut),
        (120, (W, H), W, H, 0, false)
    );
    assert!(a.delays.iter().all(|&d| d == 20));
}

#[test]
fn an_effects_frames_are_the_panels() {
    use opendmd_core::canvas::Canvas;
    use opendmd_core::fx::Fx;
    use std::time::Duration;
    // Every frame equals the effect drawn on its own at that time — the
    // sparse ones too, though every other frame is skipped.
    for path in [
        "@fx:tunnel?palette=fire&texture=rings",
        "@fx:wire?shape=torus",
        "@fx:starfield?density=3",
    ] {
        let a = effect_all(path);
        for i in [0, 1, 2, 57, a.len() - 1] {
            let mut c = Canvas::new();
            Fx::parse(path)
                .unwrap()
                .frame(Duration::from_millis(i as u64 * 20), &mut c);
            let want = c.as_bytes();
            let got: Vec<u8> = a
                .image(i)
                .px
                .iter()
                .flat_map(|p| [p[0], p[1], p[2]])
                .collect();
            assert_eq!(got, want, "{path} frame {i}");
        }
        assert_ne!(px(&a, 0, 64, 16), [1, 2, 3]); // (just touching every frame's picture)
    }
}

#[test]
fn an_effect_saves_as_a_gif_the_player_plays() {
    let mut a = effect_all("@fx:starfield?palette=amber&warp=0");
    a.configure(Fit::Pixels, false, 0, 0, false, 100, Colours::Auto);
    let seq = a.sequence(&edit(a.len()));
    assert_eq!(seq.len(), 120);
    a.save_begin(seq, 0);
    while !a.save_step(16) {}
    let bytes = a.saved();
    let mut d = gif::DecodeOptions::new()
        .read_info(bytes.as_slice())
        .unwrap();
    let mut n = 0;
    while let Some(f) = d.read_next_frame().unwrap() {
        assert_eq!((f.width, f.height, f.delay), (W as u16, H as u16, 2));
        n += 1;
    }
    assert_eq!((n, d.repeat()), (120, Repeat::Infinite));
    let s = opendmd_core::gifslim::recode_gif(
        bytes.as_slice(),
        Vec::new(),
        &opendmd_core::gifslim::PREPARED,
    )
    .unwrap();
    assert_eq!(s.frames, 120);
}

// --- Frames drawn by the page (a formula's "Save as GIF") ----------------

#[test]
fn pushed_frames_become_an_animation() {
    // Four 128x32 frames, each a different grey: kept as given, at the
    // first one's delay; extra frames are ignored; a wrong size is refused.
    let f = |v: u8| vec![v; W * H * 3];
    assert!(Anim::frames(&[0; 12], 4, 40, false).is_none());
    let mut a = Anim::frames(&f(10), 4, 40, true).unwrap();
    assert_eq!((a.in_file, a.len(), a.cut, a.orig), (4, 1, true, (W, H)));
    assert!(!a.push(&f(20)));
    assert!(!a.push(&[1, 2, 3]), "a wrong size is not a frame");
    assert!(!a.push(&f(30)));
    assert!(a.push(&f(40)));
    assert!(a.push(&f(50)), "more are ignored");
    assert_eq!(a.len(), 4);
    assert!(a.delays.iter().all(|&d| d == 40));
    for (i, v) in [10, 20, 30, 40].into_iter().enumerate() {
        assert_eq!(px(&a, i, 64, 16), [v, v, v]);
    }
}

/// What an effect's GIF and its prepared copy weigh:
/// `cargo test --release -- --ignored effect_gif_sizes --nocapture`.
#[test]
#[ignore]
fn effect_gif_sizes() {
    for path in [
        "@fx:tunnel",
        "@fx:tunnel?palette=fire&texture=rings&twist=2",
        "@fx:tunnel?texture=noise&drift=1",
        "@fx:rotozoom",
        "@fx:rotozoom?palette=rainbow&texture=rings&wobble=3",
        "@fx:rotozoom?floor=1",
        "@fx:starfield",
        "@fx:starfield?density=3&speed=4",
        "@fx:wire",
        "@fx:wire?shape=tesseract",
    ] {
        let mut a = effect_all(path);
        a.configure(Fit::Pixels, false, 0, 0, false, 100, Colours::Auto);
        let seq = a.sequence(&edit(a.len()));
        a.save_begin(seq, 0);
        while !a.save_step(16) {}
        let bytes = a.saved();
        let mut copy = Vec::new();
        opendmd_core::gifslim::recode_gif(
            bytes.as_slice(),
            &mut copy,
            &opendmd_core::gifslim::PREPARED,
        )
        .unwrap();
        println!(
            "{path}: {} frames x {} ms, {} colours, GIF {} B ({} B a frame), copy {} B",
            a.len(),
            a.delays[0],
            a.colours,
            bytes.len(),
            bytes.len() / a.len(),
            copy.len()
        );
    }
}

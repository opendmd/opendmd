//! The WebP reader (`webp`, feature `webp`) against the fixtures made by
//! testdata/webp/make.py: what a browser shows after each frame, worked out
//! there from the container spec (exact blending, rounded once) and
//! libwebp's own decoding of each frame. Natively and under node (wasm32:
//! where a usize is 32 bits, the size checks below are the ones that matter).

use crate::anim::{Anim, MAX_FRAMES};
use crate::webp::{is_animated, shown_ms, Webp, MAX_PIXELS};

#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test as test;

const ANIM: &[u8] = include_bytes!("../testdata/webp/anim.webp");
const EXPECT: &[u8] = include_bytes!("../testdata/webp/anim.rgba");
const META: &str = include_str!("../testdata/webp/anim.json");

/// anim.json's numbers, read without a JSON crate: `"key": [a, b]` or `"key": n`.
fn meta(key: &str) -> Vec<u32> {
    let at = META.find(&format!("\"{key}\":")).unwrap() + key.len() + 3;
    let rest = META[at..].trim_start();
    let end = if rest.starts_with('[') {
        rest.find(']').unwrap() + 1
    } else {
        rest.find([',', '}']).unwrap()
    };
    rest[..end]
        .trim_matches(['[', ']'])
        .split(',')
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.trim().parse().unwrap())
        .collect()
}

#[test]
fn reads_the_container() {
    let w = Webp::open(ANIM.to_vec()).unwrap();
    assert_eq!(
        (w.width as u32, w.height as u32),
        (meta("width")[0], meta("height")[0])
    );
    assert_eq!(w.plays, meta("plays")[0]);
    assert_eq!(w.len(), meta("ms").len());
    assert!(is_animated(&ANIM[..21]));
}

#[test]
fn every_frame_as_a_browser_shows_it() {
    let mut w = Webp::open(ANIM.to_vec()).unwrap();
    let n = w.width * w.height * 4;
    let (ms, lossy) = (meta("ms"), meta("lossy"));
    for (i, want_ms) in ms.iter().enumerate() {
        assert_eq!(
            w.next_frame().unwrap().unwrap(),
            *want_ms,
            "frame {i}'s duration"
        );
        let (got, want) = (w.rgba(), &EXPECT[i * n..(i + 1) * n]);
        // Lossless frames: exact, RGBA (the spec's arithmetic, see-through over
        // see-through included). A lossy frame: its alpha exact, its colour on
        // black within what two VP8 decoders differ by.
        let exact = !lossy.contains(&(i as u32));
        let tol = if exact { 0 } else { 6 };
        let rgb = w.rgb();
        for p in 0..w.width * w.height {
            let (g, e) = (&got[p * 4..p * 4 + 4], &want[p * 4..p * 4 + 4]);
            assert_eq!(g[3], e[3], "frame {i} pixel {p}: alpha");
            if exact && e[3] > 0 {
                assert_eq!(g, e, "frame {i} pixel ({}, {})", p % w.width, p / w.width);
            }
            for c in 0..3 {
                let on_black = (e[c] as u32 * e[3] as u32 + 127) / 255;
                assert!(
                    rgb[p * 3 + c].abs_diff(on_black as u8) <= tol,
                    "frame {i} pixel ({}, {}) channel {c}: {} for {on_black}",
                    p % w.width,
                    p / w.width,
                    rgb[p * 3 + c]
                );
            }
        }
    }
    assert!(w.next_frame().is_none(), "six frames, then none");
}

#[test]
fn disposal_and_holes() {
    // Spot checks the reference also covers, in words: frame 1's rectangle
    // (6, 4) 12x8 is cleared before frame 2, so after frame 2 a pixel there that
    // frame 0 left see-through is see-through again; frame 2's see-through
    // pixels punched holes (no blending): (20, 10) is one of them.
    let mut w = Webp::open(ANIM.to_vec()).unwrap();
    let px = |w: &Webp, x: usize, y: usize| w.rgba()[(y * w.width + x) * 4..][..4].to_vec();
    w.next_frame().unwrap().unwrap();
    assert_eq!(px(&w, 7, 5), [42, 50, 200, 255], "frame 0: the gradient");
    w.next_frame().unwrap().unwrap();
    assert_eq!(
        px(&w, 7, 5),
        [0, 255, 0, 255],
        "frame 1: opaque green over it"
    );
    w.next_frame().unwrap().unwrap();
    assert_eq!(
        px(&w, 7, 5)[3],
        0,
        "frame 1 disposed: cleared to transparent, not to the ANIM colour"
    );
    assert_eq!(px(&w, 20, 10)[3], 0, "frame 2's hole");
    assert_eq!(px(&w, 21, 10), [250, 200, 0, 255]);
}

#[test]
fn refuses_in_words() {
    let why = |b: Vec<u8>| Webp::open(b).err().map(|e| e.0).unwrap_or_default();
    assert_eq!(why(b"GIF89a".repeat(10)), "it isn’t a WebP");
    // A still (VP8X without the animation flag).
    let mut still = ANIM.to_vec();
    still[20] &= !0x02;
    assert_eq!(why(still.clone()), "it isn’t an animated WebP");
    assert!(!is_animated(&still));
    // A canvas past the limit.
    let mut big = ANIM.to_vec();
    big[24..30].copy_from_slice(&[0xff, 0x0f, 0, 0xff, 0x0f, 0]); // 4096 x 4096
    const { assert!(4096 * 4096 > MAX_PIXELS) };
    assert!(why(big).starts_with("it is 4096 × 4096"));
    // Cut short in its first frame: refused; in a later one: the frames before it are kept.
    let first_anmf = ANIM.windows(4).position(|x| x == b"ANMF").unwrap();
    assert_eq!(
        why(ANIM[..first_anmf + 30].to_vec()),
        "the file is cut short"
    );
    let w = Webp::open(ANIM[..ANIM.len() - 10].to_vec()).unwrap();
    assert_eq!(w.len(), 5);
}

#[test]
fn a_damaged_frame_says_so() {
    let mut b = ANIM.to_vec();
    // Frame 2's bitstream (the third ANMF's VP8L) garbled.
    let at = b
        .windows(4)
        .enumerate()
        .filter(|(_, x)| *x == b"VP8L")
        .nth(2)
        .unwrap()
        .0;
    for v in &mut b[at + 9..at + 20] {
        *v = 0xff;
    }
    let mut w = Webp::open(b).unwrap();
    assert!(w.next_frame().unwrap().is_ok());
    assert!(w.next_frame().unwrap().is_ok());
    assert_eq!(w.next_frame().unwrap().unwrap_err().0, "a frame is damaged");
}

#[test]
fn durations_as_shown() {
    assert_eq!(
        [0, 10, 11, 15, 40, 1000].map(shown_ms),
        [100, 100, 20, 20, 40, 1000]
    );
}

/// Pixel art (a grey ground, small blocks of a few colours) `k` times upscaled, RGB.
fn upscaled(w: usize, h: usize, k: usize, seed: u32) -> Vec<u8> {
    const PAL: [[u8; 3]; 5] = [
        [230, 40, 40],
        [40, 200, 60],
        [50, 80, 230],
        [250, 220, 0],
        [255, 255, 255],
    ];
    let mut px = vec![[90u8, 90, 90]; w * h];
    let mut r = seed.wrapping_mul(2654435761) | 1;
    let mut rnd = |n: usize| {
        r ^= r << 13;
        r ^= r >> 17;
        r ^= r << 5;
        r as usize % n
    };
    for _ in 0..w * h / 12 {
        let (x, y, c, bw, bh) = (rnd(w), rnd(h), PAL[rnd(5)], 1 + rnd(4), 1 + rnd(3));
        for yy in y..(y + bh).min(h) {
            for xx in x..(x + bw).min(w) {
                px[yy * w + xx] = c;
            }
        }
    }
    (0..h * k)
        .flat_map(|y| (0..w * k).map(move |x| (x / k, y / k)))
        .flat_map(|(x, y)| px[y * w + x])
        .collect()
}

#[test]
fn pictures_are_treated_as_a_gif_is() {
    // A 3x upscale of 40 x 16 found on the first frame and undone; the rest
    // come at the source's size; plays kept.
    let (w, h, k) = (40, 16, 3);
    let mut a = Anim::pictures(&upscaled(w, h, k, 1), w * k, h * k, 3, 120, 2).unwrap();
    assert_eq!((a.scale.k, a.w, a.h, a.orig), (3, w, h, (w * k, h * k)));
    assert!(!a.video);
    assert!(!a.push_delay(&upscaled(w, h, k, 3), 60));
    assert!(
        !a.push_delay(&[0; 9], 60),
        "a frame of the wrong size is ignored"
    );
    assert!(a.push_delay(&upscaled(w, h, k, 5), 30));
    assert_eq!(
        (a.len(), a.delays.clone(), a.loops),
        (3, vec![120, 60, 30], 2)
    );
    let f = a.image(2);
    assert_eq!((f.w, f.h), (w, h));
    let want = upscaled(w, h, 1, 5);
    assert!(
        f.px.iter().zip(want.chunks(3)).all(|(p, q)| p[..3] == *q),
        "the source's own pixels"
    );
}

#[test]
fn pictures_fit_the_budget_and_the_cap() {
    // 100 frames of 200 x 100 in a 1 MB budget: reduced by a whole number.
    let first = vec![9u8; 200 * 100 * 3];
    let a = Anim::pictures_with_budget(&first, 200, 100, 100, 50, 0, 1 << 20).unwrap();
    assert!(
        a.reduce > 1 && 100 * a.w * a.h * 3 <= 1 << 20,
        "reduce {} to {} x {}",
        a.reduce,
        a.w,
        a.h
    );
    // More frames in the file than are kept: capped, and pushing stops there.
    let mut a = Anim::pictures(&[1, 2, 3], 1, 1, MAX_FRAMES + 5, 50, 0).unwrap();
    assert!(a.capped());
    let mut done = false;
    for _ in 1..MAX_FRAMES {
        done = a.push_delay(&[1, 2, 3], 50);
    }
    assert!(done && a.len() == MAX_FRAMES);
    assert!(
        Anim::pictures(&[1, 2], 1, 1, 1, 50, 0).is_none(),
        "a first frame of the wrong size"
    );
}

/// The fixture with a byte range replaced.
fn patched(at: usize, with: &[u8]) -> Vec<u8> {
    let mut b = ANIM.to_vec();
    b[at..at + with.len()].copy_from_slice(with);
    b
}

/// Where the `k`th chunk named `tag` starts (its fourcc).
fn nth(tag: &[u8], k: usize) -> usize {
    ANIM.windows(4)
        .enumerate()
        .filter(|(_, x)| *x == tag)
        .nth(k)
        .unwrap()
        .0
}

#[test]
fn sizes_that_wrap_on_wasm32_are_refused() {
    let why = |b: Vec<u8>| Webp::open(b).err().map(|e| e.0).unwrap_or_default();
    // 65536 x 65536 is 2^32: 0 in a 32-bit usize. 65536 x 65552 wraps to 2^20, under the limit.
    for (w, h) in [(65536usize, 65536usize), (65536, 65552)] {
        let mut b = ANIM.to_vec();
        b[24..27].copy_from_slice(&((w - 1) as u32).to_le_bytes()[..3]);
        b[27..30].copy_from_slice(&((h - 1) as u32).to_le_bytes()[..3]);
        assert!(
            why(b).starts_with(&format!("it is {w} × {h}, more than")),
            "{w} x {h}"
        );
    }
    // A RIFF size of 0xffffffff (+ 8 wraps): read as "to the end of the file".
    let w = Webp::open(patched(4, &[0xff; 4])).unwrap();
    assert_eq!(w.len(), 6);
    // The pictures path: a page's w * h * 3 that wraps is refused, not trusted.
    let rgb = vec![0u8; 3 << 20];
    assert!(Anim::pictures(&rgb, 65536, 65552, 2, 50, 0).is_none());
    assert!(Anim::video(&rgb, 65536, 65552, 65536, 65552, 50).is_none());
}

#[test]
fn a_frame_whose_bitstream_says_another_size_is_damaged() {
    // Frame 3 is ALPH + VP8 (16 x 12): its VP8 header made to say 16383 x 8000,
    // refused before decoding (a small frame must not carry a huge picture).
    let vp8 = nth(b"VP8 ", 0) + 8; // (the payload)
    let mut w = Webp::open(patched(vp8 + 6, &[0xff, 0x3f, 0x40, 0x1f])).unwrap();
    for _ in 0..3 {
        w.next_frame().unwrap().unwrap();
    }
    // (On wasm32 the memory it took is seen: a decoder told 16383 x 8000
    // sets out ~200 MB of buffers before it finds the data short.)
    #[cfg(target_arch = "wasm32")]
    let before = core::arch::wasm32::memory_size(0);
    assert_eq!(w.next_frame().unwrap().unwrap_err().0, "a frame is damaged");
    #[cfg(target_arch = "wasm32")]
    {
        let grew_mb = (core::arch::wasm32::memory_size(0) - before) * 64 / 1024;
        assert!(
            grew_mb < 16,
            "decoding was started: the memory grew {grew_mb} MB"
        );
    }
    // A VP8L frame (frame 1, 12 x 8) whose header says 13 x 8.
    let vp8l = nth(b"VP8L", 1) + 8;
    let mut b = ANIM.to_vec();
    b[vp8l + 1] = b[vp8l + 1].wrapping_add(1);
    let mut w = Webp::open(b).unwrap();
    w.next_frame().unwrap().unwrap();
    assert_eq!(w.next_frame().unwrap().unwrap_err().0, "a frame is damaged");
}

#[test]
fn lossless_alpha_is_read_whatever_its_hint_says() {
    // Frame 1's VP8L with its "alpha is used" hint cleared: libwebp still
    // decodes its alpha, and so does this reader: the canvas is as before.
    let vp8l = nth(b"VP8L", 1) + 8;
    let mut b = ANIM.to_vec();
    b[vp8l + 4] &= !0x10;
    let (mut a, mut z) = (Webp::open(ANIM.to_vec()).unwrap(), Webp::open(b).unwrap());
    for _ in 0..2 {
        a.next_frame().unwrap().unwrap();
        z.next_frame().unwrap().unwrap();
    }
    assert_eq!(a.rgba(), z.rgba());
}

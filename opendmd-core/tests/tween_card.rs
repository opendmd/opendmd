//! Smooth motion against the tweening experiment (`docs/tweening-experiment.md`):
//! the Rust tweener on the experiment's 30 files (20 candidates, 10
//! controls), compared with the Python reference's output.
//!
//! Ignored by default — it needs the (third-party) GIF pack and the
//! experiment's outputs, which are not in the repo:
//!
//! ```sh
//! DMD_CARD_ROOT=~/scratch/dmdos-card cargo test --release --features tween \
//!     --test tween_card -- --ignored --nocapture
//! ```
//!
//! Per file: the **decisions** — which pairs get in-betweens, their class,
//! shift, count and delay — against `out/candidates.json` (`tween_pairs`,
//! what `tween.py` did), and the **pixels** — every composited canvas and
//! delay of the Rust tweened GIF against `out/gifs/<name>.tween.gif`, both
//! played through `GifSource`. Pillow's writer merges consecutive identical
//! frames (adding their durations), so both sides are compared as
//! timelines: runs of identical canvases with their summed durations.
//! `TWEEN_OUT` overrides the outputs' directory.

#![cfg(feature = "tween")]

use opendmd_core::canvas::Canvas;
use opendmd_core::player::GifSource;
use opendmd_core::tween::{Class, Smooth};
use std::path::PathBuf;

/// One `tween_pairs` row: (pair, class, (dx, dy), k, delay ms).
type Row = (u32, String, (i32, i32), u32, u32);

struct Candidate {
    role: String,
    path: String,
    name: String,
    pairs: Vec<Row>,
}

/// JSON strings and integers in order — all `tween_pairs` holds.
fn tokens(s: &str) -> Vec<Result<i64, String>> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'"' => {
                let j = i + 1 + s[i + 1..].find('"').unwrap();
                out.push(Err(s[i + 1..j].to_string()));
                i = j + 1;
            }
            b'-' | b'0'..=b'9' => {
                let j = i
                    + 1
                    + s[i + 1..]
                        .find(|c: char| !c.is_ascii_digit())
                        .unwrap_or(s.len() - i - 1);
                out.push(Ok(s[i..j].parse().unwrap()));
                i = j;
            }
            _ => i += 1,
        }
    }
    out
}

fn string_field(chunk: &str, key: &str) -> String {
    let k = format!("\"{key}\": \"");
    let at = chunk.find(&k).unwrap_or_else(|| panic!("no {key}")) + k.len();
    chunk[at..at + chunk[at..].find('"').unwrap()].to_string()
}

/// `candidates.json` (indent=1, written by `tween.py`), read without a JSON
/// crate: each entry's role, path, name and `tween_pairs`.
fn candidates(text: &str) -> Vec<Candidate> {
    text.split("\"role\": ")
        .skip(1)
        .map(|chunk| {
            let role = chunk[1..chunk[1..].find('"').unwrap() + 1].to_string();
            let at = chunk.find("\"tween_pairs\": [").unwrap() + "\"tween_pairs\": ".len();
            let mut depth = 0;
            let mut end = at;
            for (i, c) in chunk[at..].char_indices() {
                match c {
                    '[' => depth += 1,
                    ']' => {
                        depth -= 1;
                        if depth == 0 {
                            end = at + i + 1;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            let t = tokens(&chunk[at..end]);
            let pairs = t
                .chunks(6)
                .map(|r| {
                    let n = |k: usize| *r[k].as_ref().unwrap();
                    (
                        n(0) as u32,
                        r[1].clone().unwrap_err(),
                        (n(2) as i32, n(3) as i32),
                        n(4) as u32,
                        n(5) as u32,
                    )
                })
                .collect();
            Candidate {
                role,
                path: string_field(chunk, "path"),
                name: string_field(chunk, "name"),
                pairs,
            }
        })
        .collect()
}

/// A GIF's frames as the player shows them, as a timeline: runs of
/// identical canvases with their summed duration (ms).
fn timeline(gif: &[u8]) -> Vec<(Vec<u8>, u64)> {
    let mut g = GifSource::new(gif).unwrap();
    let mut c = Canvas::new();
    let mut out: Vec<(Vec<u8>, u64)> = Vec::new();
    while let Some(d) = g.next_frame(&mut c).unwrap() {
        let ms = d.as_millis() as u64;
        match out.last_mut() {
            Some(last) if last.0 == c.as_bytes() => last.1 += ms,
            _ => out.push((c.as_bytes().to_vec(), ms)),
        }
    }
    out
}

#[test]
#[ignore = "needs DMD_CARD_ROOT and the tweening experiment's outputs"]
fn smooth_motion_matches_the_experiment() {
    let Ok(root) = std::env::var("DMD_CARD_ROOT") else {
        eprintln!("DMD_CARD_ROOT not set, nothing to do");
        return;
    };
    let out = std::env::var("TWEEN_OUT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../opendmd-web/experiments/tween/out")
        });
    let text = std::fs::read_to_string(out.join("candidates.json"))
        .expect("candidates.json (run the experiment's tween.py)");
    let cands = candidates(&text);
    assert_eq!(cands.len(), 30);
    println!(
        "{:<9} {:<48} {:>6} {:>6} {:>6} {:>9} {:>8} {:>10} {:>9}",
        "role", "file", "frames", "rust", "python", "decisions", "timeline", "frames≠", "pixels≠"
    );
    let mut failures = Vec::new();
    for c in &cands {
        let rel = c.path.split("/gifs/").nth(1).unwrap();
        let gif = std::fs::read(PathBuf::from(&root).join("gifs").join(rel)).unwrap();
        let mut s = Smooth::new(gif).unwrap();
        while !s.analyse(u32::MAX).unwrap() {}
        while !s.tween(u32::MAX).unwrap() {}
        let sum = s.summary();

        // Decisions, pair by pair.
        let ours: Vec<Row> = s
            .pairs()
            .iter()
            .enumerate()
            .filter(|(_, p)| p.k > 0)
            .map(|(i, p)| {
                (
                    i as u32,
                    p.class.name().to_string(),
                    p.shift,
                    p.k,
                    p.delay_ms,
                )
            })
            .collect();
        let decisions_equal = ours == c.pairs;
        if !decisions_equal {
            let theirs: std::collections::BTreeMap<u32, &Row> =
                c.pairs.iter().map(|r| (r.0, r)).collect();
            let mine: std::collections::BTreeMap<u32, &Row> =
                ours.iter().map(|r| (r.0, r)).collect();
            for i in theirs
                .keys()
                .chain(mine.keys())
                .collect::<std::collections::BTreeSet<_>>()
            {
                if theirs.get(i) != mine.get(i) {
                    println!(
                        "    pair {i}: python {:?} rust {:?} (rust saw {:?})",
                        theirs.get(i),
                        mine.get(i),
                        s.pairs()[*i as usize]
                    );
                }
            }
            failures.push(format!("{}: decisions differ", c.name));
        }
        assert!(ours
            .iter()
            .all(|r| r.1 != Class::Cut.name() && r.1 != Class::Other.name()));

        // Pixels and delays, as timelines.
        let python = std::fs::read(out.join("gifs").join(format!("{}.tween.gif", c.name))).unwrap();
        let (a, b) = (timeline(s.tweened_gif()), timeline(&python));
        let dur_a: u64 = a.iter().map(|x| x.1).sum();
        let dur_b: u64 = b.iter().map(|x| x.1).sum();
        assert_eq!(dur_a, sum.duration_ms, "{}: total duration", c.name);
        let (mut frames_diff, mut px_diff) = (0usize, 0usize);
        for (x, y) in a.iter().zip(&b) {
            if x != y {
                frames_diff += 1;
                px_diff +=
                    x.0.chunks(3)
                        .zip(y.0.chunks(3))
                        .filter(|(p, q)| p != q)
                        .count();
            }
        }
        let timeline_equal = a == b;
        if !timeline_equal {
            failures.push(format!(
                "{}: timeline {} vs {} runs ({} vs {} ms), {frames_diff} runs differ, {px_diff} px",
                c.name,
                a.len(),
                b.len(),
                dur_a,
                dur_b
            ));
        }
        println!(
            "{:<9} {:<48} {:>6} {:>6} {:>6} {:>9} {:>8} {:>10} {:>9}",
            c.role,
            rel.trim_end_matches(".gif"),
            sum.frames,
            s.tweened_frames(),
            sum.frames + c.pairs.iter().map(|r| r.3).sum::<u32>(),
            if decisions_equal { "equal" } else { "DIFFER" },
            if timeline_equal { "equal" } else { "DIFFER" },
            frames_diff,
            px_diff
        );
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

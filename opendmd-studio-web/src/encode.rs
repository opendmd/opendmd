//! The Studio's GIF writers: a piece of 256 colours or fewer over exactly
//! its colours, one global table ([`indexed`]); past 256, each frame as a
//! whole picture over 256 of its colours and then corrections with tables
//! of their own, every colour kept ([`wide`]); or fitted to 256 over one
//! table ([`fitted`]). The 256 are picked for spread ([`spread_pick`], the
//! converter's rule), not by use. Delays are ms in, hundredths of a second
//! in the file.

use gif::{Encoder, Frame, Repeat};
use std::borrow::Cow;

/// The panel.
pub const W: usize = 128;
pub const H: usize = 32;

/// `table` (`0xRRGGBB`) as a GIF colour table: a one-colour picture still
/// needs a two-entry table.
fn flat(table: &[u32]) -> Vec<u8> {
    let mut pal: Vec<u8> = table
        .iter()
        .flat_map(|&c| [(c >> 16) as u8, (c >> 8) as u8, c as u8])
        .collect();
    pal.resize(pal.len().max(6), 0);
    pal
}

/// A GIF's header over `palette` (`0xRRGGBB`), played `plays` times — 0
/// forever (a NETSCAPE loop count of 0), 1 once (no loop block: what
/// players do without one), N: a loop count of N - 1 (N plays in all).
fn begin(palette: &[u32], plays: u32) -> Encoder<Vec<u8>> {
    // Writing into a Vec cannot fail, and the frames are well formed
    // (W x H indices into a palette of <= 256), so errors are bugs.
    let mut enc = Encoder::new(Vec::new(), W as u16, H as u16, &flat(palette)).expect("gif header");
    match plays {
        0 => enc.set_repeat(Repeat::Infinite).expect("gif loop"),
        1 => {}
        n => enc
            .set_repeat(Repeat::Finite((n - 1).min(u16::MAX as u32) as u16))
            .expect("gif loop"),
    }
    enc
}

/// A frame's delay in hundredths, at least 2 (20 ms, the shortest players keep).
fn hundredths(ms: u32) -> u32 {
    (ms.saturating_add(5) / 10).clamp(2, u16::MAX as u32)
}

/// Frames drawn as palette indices: a 128x32 GIF over exactly `palette`
/// (`0xRRGGBB`, 1..=256 colours, never re-quantised), `delays_ms.len()`
/// frames of `W * H` indices each, back to back in `frames`, played
/// `plays` times ([`begin`]). Refuses (with why) a palette out of range,
/// frames of the wrong length or an index past the palette, so a bad call
/// never writes a broken file.
pub fn indexed(
    palette: &[u32],
    frames: &[u8],
    delays_ms: &[u32],
    plays: u32,
) -> Result<Vec<u8>, &'static str> {
    if palette.is_empty() || palette.len() > 256 {
        return Err("a palette of 1 to 256 colours");
    }
    if delays_ms.is_empty() || frames.len() != delays_ms.len() * W * H {
        return Err("frames of 128 x 32 indices, one delay each");
    }
    if frames.iter().any(|&i| i as usize >= palette.len()) {
        return Err("an index past the palette");
    }
    let mut enc = begin(palette, plays);
    for (f, &d) in frames.as_chunks::<{ W * H }>().0.iter().zip(delays_ms) {
        let frame = Frame {
            width: W as u16,
            height: H as u16,
            buffer: Cow::Borrowed(f),
            delay: hundredths(d) as u16,
            ..Frame::default()
        };
        enc.write_frame(&frame).expect("gif frame");
    }
    Ok(enc.into_inner().expect("gif trailer"))
}

/// The colours of `px` with their counts, `(0xRRGGBB, uses)`, most used
/// first (ties: the lower colour). One sort of `u64`s throughout (count and
/// colour packed): the wasm stays small.
pub fn by_use(px: &[u32]) -> Vec<(u32, u32)> {
    let mut v: Vec<u64> = px.iter().map(|&c| c as u64).collect();
    v.sort_unstable();
    let mut runs: Vec<u64> = Vec::new();
    let mut i = 0;
    while i < v.len() {
        let j = i + v[i..].partition_point(|&x| x == v[i]);
        runs.push(!((j - i) as u64) << 32 | v[i]);
        i = j;
    }
    runs.sort_unstable();
    runs.into_iter()
        .map(|r| (r as u32, !(r >> 32) as u32))
        .collect()
}

fn split(c: u32) -> [i32; 3] {
    [
        (c >> 16) as i32 & 255,
        (c >> 8) as i32 & 255,
        c as i32 & 255,
    ]
}

fn dist2(a: [i32; 3], b: [i32; 3]) -> i32 {
    (0..3).map(|i| (a[i] - b[i]) * (a[i] - b[i])).sum()
}

/// At most `max` of the picture's own colours: all of them when they fit;
/// else the most used first, then again and again whichever is furthest
/// from everything picked so far, weighted by log(1 + uses), so a rare
/// colour far from the rest is kept and a crowd of near-twins is not (a
/// copy of the converter's, opendmd-art-web `pipeline::spread_pick`).
/// `counts` as [`by_use`] gives them.
pub fn spread_pick(counts: &[(u32, u32)], max: usize) -> Vec<u32> {
    if counts.len() <= max {
        return counts.iter().map(|c| c.0).collect();
    }
    let rgb: Vec<[i32; 3]> = counts.iter().map(|c| split(c.0)).collect();
    let wt: Vec<f64> = counts.iter().map(|c| (c.1 as f64).ln_1p()).collect();
    let mut picked = vec![0usize];
    let mut dmin: Vec<i32> = rgb.iter().map(|&p| dist2(p, rgb[0])).collect();
    while picked.len() < max {
        let (mut bi, mut bv) = (0, -1.0);
        for (i, &d) in dmin.iter().enumerate() {
            let v = (d as f64).sqrt() * wt[i];
            if v > bv {
                bv = v;
                bi = i;
            }
        }
        if bv <= 0.0 {
            break;
        }
        picked.push(bi);
        for (i, d) in dmin.iter_mut().enumerate() {
            *d = (*d).min(dist2(rgb[i], rgb[bi]));
        }
    }
    picked.iter().map(|&i| counts[i].0).collect()
}

/// Colour -> index pairs as a lookup, `colour << 8 | index`, sorted.
fn lookup(pairs: impl Iterator<Item = (u32, usize)>) -> Vec<u64> {
    let mut v: Vec<u64> = pairs.map(|(c, i)| (c as u64) << 8 | i as u64).collect();
    v.sort_unstable();
    v
}
fn find(look: &[u64], c: u32) -> Option<u8> {
    let i = look.partition_point(|&e| (e >> 8) < c as u64);
    look.get(i)
        .filter(|&&e| (e >> 8) == c as u64)
        .map(|&e| e as u8)
}

/// The nearest of `table` to `c` (the converter's distance; ties: the first).
fn nearest(table: &[u32], c: u32) -> usize {
    let p = split(c);
    let mut best = (0, i32::MAX);
    for (i, &t) in table.iter().enumerate() {
        let d = dist2(p, split(t));
        if d < best.1 {
            best = (i, d);
        }
    }
    best.0
}

/// `px` over 256 of its colours ([`spread_pick`]; all of them, exactly,
/// when they fit): its colours by use, the table and each pixel's index,
/// every other colour drawn as its nearest there (found once a colour).
fn fit_counts(px: &[u32]) -> (Vec<(u32, u32)>, Vec<u32>, Vec<u8>) {
    let counts = by_use(px);
    let table = spread_pick(&counts, 256);
    let own = lookup(table.iter().enumerate().map(|(i, &c)| (c, i)));
    let map = lookup(counts.iter().map(|&(c, _)| {
        (
            c,
            find(&own, c).map_or_else(|| nearest(&table, c), |i| i as usize),
        )
    }));
    let idx = px
        .iter()
        .map(|&c| find(&map, c).expect("every colour mapped"))
        .collect();
    (counts, table, idx)
}

/// `px` over 256 of its colours: the table and each pixel's index.
pub fn fit(px: &[u32]) -> (Vec<u32>, Vec<u8>) {
    let (_, table, idx) = fit_counts(px);
    (table, idx)
}

fn check_rgb(frames: &[u32], delays_ms: &[u32]) -> Result<(), &'static str> {
    if delays_ms.is_empty() || frames.len() != delays_ms.len() * W * H {
        return Err("frames of 128 x 32 colours, one delay each");
    }
    if frames.iter().any(|&c| c > 0xff_ffff) {
        return Err("colours as 0xRRGGBB");
    }
    Ok(())
}

/// Frames of 128x32 colours (`0xRRGGBB`, back to back) fitted to 256
/// colours ([`fit`]: over the whole animation) and written over one global
/// table ([`indexed`]): lossy past 256, but a GIF the player can prepare
/// and smooth. 256 colours or fewer, it is exact.
pub fn fitted(frames: &[u32], delays_ms: &[u32], plays: u32) -> Result<Vec<u8>, &'static str> {
    check_rgb(frames, delays_ms)?;
    let (table, idx) = fit(frames);
    indexed(&table, &idx, delays_ms, plays)
}

/// Frames past 256 colours, exactly: a 128x32 GIF of `delays_ms.len()`
/// frames of `W * H` colours (`0xRRGGBB`, back to back in `frames`), played
/// `plays` times ([`begin`]). A GIF frame holds 256 colours at most, so
/// each frame is written as a base and its corrections: first the whole
/// picture over 256 of its colours picked for spread (every other colour as
/// its nearest there, [`fit`]), so no pixel is ever left unpainted (a
/// player clears to black when it starts a GIF, and at every loop); then,
/// 20 ms each (the shortest delay players keep), only the pixels not yet
/// exact, up to 255 more colours a frame (by use), the rest of the
/// rectangle transparent. The last of them holds the rest of the frame's
/// time, so the frame lasts what it says whenever that is at least 20 ms a
/// GIF frame; shorter, it lasts 20 ms a GIF frame. Nothing is lost: the
/// panel plays such a GIF exactly, as it is (`docs/cache-format.md`, "Exact
/// or absent"). A frame of 256 colours or fewer is one GIF frame.
pub fn wide(frames: &[u32], delays_ms: &[u32], plays: u32) -> Result<Vec<u8>, &'static str> {
    check_rgb(frames, delays_ms)?;
    // No global table (the encoder writes a 2-entry black one): every frame has its own.
    let mut enc = begin(&[], plays);
    for (f, &d) in frames.as_chunks::<{ W * H }>().0.iter().zip(delays_ms) {
        let (counts, table, idx) = fit_counts(f);
        let own = lookup(table.iter().enumerate().map(|(i, &c)| (c, i)));
        let order: Vec<u32> = counts
            .iter()
            .map(|c| c.0)
            .filter(|&c| find(&own, c).is_none())
            .collect();
        let rest: Vec<&[u32]> = order.chunks(255).collect();
        let total = hundredths(d);
        let n = 1 + rest.len() as u32;
        let last = total.saturating_sub(2 * (n - 1)).max(2) as u16;
        let mut put =
            |top: usize, h: usize, buffer: Vec<u8>, pal: &[u32], trns: Option<u8>, delay: u16| {
                let frame = Frame {
                    top: top as u16,
                    width: W as u16,
                    height: h as u16,
                    buffer: Cow::Owned(buffer),
                    palette: Some(flat(pal)),
                    transparent: trns,
                    delay,
                    dispose: gif::DisposalMethod::Keep,
                    ..Frame::default()
                };
                enc.write_frame(&frame).expect("gif frame");
            };
        put(
            0,
            H,
            idx,
            &table,
            None,
            if rest.is_empty() { last } else { 2 },
        );
        for (k, part) in rest.iter().enumerate() {
            let look = lookup(part.iter().enumerate().map(|(i, &c)| (c, i + 1)));
            let at = |c: u32| find(&look, c);
            let rows: Vec<usize> = (0..H)
                .filter(|&y| f[y * W..(y + 1) * W].iter().any(|&c| at(c).is_some()))
                .collect();
            let (y0, y1) = (rows[0], rows[rows.len() - 1] + 1);
            let buf = f[y0 * W..y1 * W]
                .iter()
                .map(|&c| at(c).unwrap_or(0))
                .collect();
            let mut pal = vec![0];
            pal.extend_from_slice(part);
            put(
                y0,
                y1 - y0,
                buf,
                &pal,
                Some(0),
                if k + 1 == rest.len() { last } else { 2 },
            );
        }
    }
    Ok(enc.into_inner().expect("gif trailer"))
}

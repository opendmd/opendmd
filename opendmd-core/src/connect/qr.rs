//! The pairing QR code (P1-PLAN 2.6): version 3, error correction level L,
//! byte mode only, which holds the 42 bytes of `https://opendmd.dev/c#<S>`
//! (up to [`MAX_DATA`]). 29 x 29 modules: on the 128 x 32 panel one LED a
//! module, the full height with one row above and two below (the panel's
//! screen, `firmware/src/connect/screen`, lights those as the quiet zone).
//!
//! Small on purpose: no other version, level or mode, no tables (the
//! Reed-Solomon arithmetic is done bit by bit), the matrix as 29 words. The
//! mask is the one ISO/IEC 18004's penalty rules score lowest (ties: the
//! lowest number), as any encoder would pick; the decoding is checked by an
//! independent decoder (`tools/connect/qr_check.py`, under `uv`), which also
//! checks the matrix module for module against another encoder's for the
//! same mask.

/// Modules a side.
pub const SIZE: usize = 29;
/// Data codewords of version 3-L.
const DATA_CODEWORDS: usize = 55;
/// Error correction codewords of version 3-L (one block).
const EC_CODEWORDS: usize = 15;
/// The most bytes byte mode holds in version 3-L (4 + 8 bits of header).
pub const MAX_DATA: usize = 53;
/// The alignment pattern's centre (version 3 has one).
const ALIGN: usize = 22;

/// A QR code's modules: bit `x` of `rows[y]` set for a dark module.
#[derive(Clone, PartialEq, Eq)]
pub struct Qr {
    rows: [u32; SIZE],
    mask: u8,
}

impl core::fmt::Debug for Qr {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "Qr(mask {})", self.mask)
    }
}

impl Qr {
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn dark(&self, x: usize, y: usize) -> bool {
        x < SIZE && y < SIZE && self.rows[y] >> x & 1 == 1
    }
    /// The mask pattern used (0-7).
    pub fn mask(&self) -> u8 {
        self.mask
    }
    /// The rows (bit `x` of row `y`: dark).
    pub fn rows(&self) -> &[u32; SIZE] {
        &self.rows
    }
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    fn set(&mut self, x: usize, y: usize, dark: bool) {
        if dark {
            self.rows[y] |= 1 << x;
        } else {
            self.rows[y] &= !(1 << x);
        }
    }
}

/// `data` as a version 3-L QR code with the best mask; `None` if longer
/// than [`MAX_DATA`].
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn encode(data: &[u8]) -> Option<Qr> {
    let codewords = codewords(data)?;
    let mut best: Option<(u32, Qr)> = None;
    for mask in 0..8u8 {
        let q = build(&codewords, mask);
        let p = penalty(&q);
        if best.as_ref().is_none_or(|(b, _)| p < *b) {
            best = Some((p, q));
        }
    }
    best.map(|(_, q)| q)
}

/// `data` with mask `mask` (0-7), whatever its score.
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn encode_with_mask(data: &[u8], mask: u8) -> Option<Qr> {
    (mask < 8).then_some(())?;
    Some(build(&codewords(data)?, mask))
}

// --- The codewords -------------------------------------------------------------

/// Byte mode's header, the data, the terminator, the padding, then the
/// error correction: 70 codewords.
#[cfg_attr(target_arch = "xtensa", optimize(size))]
fn codewords(data: &[u8]) -> Option<[u8; DATA_CODEWORDS + EC_CODEWORDS]> {
    if data.len() > MAX_DATA {
        return None;
    }
    let mut out = [0u8; DATA_CODEWORDS + EC_CODEWORDS];
    let mut bit = 0usize;
    let mut put = |out: &mut [u8], v: u32, n: usize| {
        for i in (0..n).rev() {
            if v >> i & 1 == 1 {
                out[bit >> 3] |= 0x80 >> (bit & 7);
            }
            bit += 1;
        }
    };
    put(&mut out, 0b0100, 4);
    put(&mut out, data.len() as u32, 8);
    for &b in data {
        put(&mut out, u32::from(b), 8);
    }
    // The terminator (up to 4 zero bits) and the rest of the byte are
    // already zero; then the pad bytes.
    let used = (bit + 4).min(DATA_CODEWORDS * 8).div_ceil(8);
    for (i, b) in out[used..DATA_CODEWORDS].iter_mut().enumerate() {
        *b = if i % 2 == 0 { 0xEC } else { 0x11 };
    }
    let ec = reed_solomon(&out[..DATA_CODEWORDS]);
    out[DATA_CODEWORDS..].copy_from_slice(&ec);
    Some(out)
}

/// GF(256) multiplication, modulo x^8 + x^4 + x^3 + x^2 + 1.
#[cfg_attr(target_arch = "xtensa", optimize(size))]
fn gf_mul(x: u8, y: u8) -> u8 {
    let mut z: u32 = 0;
    for i in (0..8).rev() {
        z = (z << 1) ^ ((z >> 7) * 0x11D);
        z ^= ((u32::from(y) >> i) & 1) * u32::from(x);
    }
    z as u8
}

/// The remainder of `data` times x^15 by the generator with roots
/// α^0..α^14.
#[cfg_attr(target_arch = "xtensa", optimize(size))]
fn reed_solomon(data: &[u8]) -> [u8; EC_CODEWORDS] {
    // The generator's coefficients, highest power first (its leading 1
    // left out).
    let mut g = [0u8; EC_CODEWORDS];
    g[EC_CODEWORDS - 1] = 1;
    let mut root = 1u8;
    for _ in 0..EC_CODEWORDS {
        for j in 0..EC_CODEWORDS {
            g[j] = gf_mul(g[j], root);
            if j + 1 < EC_CODEWORDS {
                g[j] ^= g[j + 1];
            }
        }
        root = gf_mul(root, 0x02);
    }
    let mut r = [0u8; EC_CODEWORDS];
    for &b in data {
        let factor = b ^ r[0];
        r.copy_within(1.., 0);
        r[EC_CODEWORDS - 1] = 0;
        for (x, &c) in r.iter_mut().zip(g.iter()) {
            *x ^= gf_mul(c, factor);
        }
    }
    r
}

// --- The matrix ------------------------------------------------------------------

/// A module of a function pattern (the finders and their separators, the
/// format areas, the timing lines, the alignment pattern, the dark module).
#[cfg_attr(target_arch = "xtensa", optimize(size))]
fn function(x: usize, y: usize) -> bool {
    (x < 9 && y < 9)
        || (x >= SIZE - 8 && y < 9)
        || (x < 9 && y >= SIZE - 8)
        || x == 6
        || y == 6
        || (x.abs_diff(ALIGN) <= 2 && y.abs_diff(ALIGN) <= 2)
}

// The eight mask conditions as ISO/IEC 18004 writes them (`% n == 0`), so
// they read against the standard line for line.
#[allow(clippy::manual_is_multiple_of)]
#[cfg_attr(target_arch = "xtensa", optimize(size))]
fn masked(mask: u8, x: usize, y: usize) -> bool {
    match mask {
        0 => (x + y) % 2 == 0,
        1 => y % 2 == 0,
        2 => x % 3 == 0,
        3 => (x + y) % 3 == 0,
        4 => (x / 3 + y / 2) % 2 == 0,
        5 => x * y % 2 + x * y % 3 == 0,
        6 => (x * y % 2 + x * y % 3) % 2 == 0,
        _ => ((x + y) % 2 + x * y % 3) % 2 == 0,
    }
}

#[cfg_attr(target_arch = "xtensa", optimize(size))]
fn build(codewords: &[u8; DATA_CODEWORDS + EC_CODEWORDS], mask: u8) -> Qr {
    let mut q = Qr { rows: [0; SIZE], mask };
    // Timing lines (the finders draw over their ends).
    for i in 0..SIZE {
        q.set(6, i, i % 2 == 0);
        q.set(i, 6, i % 2 == 0);
    }
    // Finders, with their light separators.
    for (cx, cy) in [(3, 3), (SIZE - 4, 3), (3, SIZE - 4)] {
        for dy in -4i32..=4 {
            for dx in -4i32..=4 {
                let (x, y) = (cx as i32 + dx, cy as i32 + dy);
                if (0..SIZE as i32).contains(&x) && (0..SIZE as i32).contains(&y) {
                    let d = dx.abs().max(dy.abs());
                    q.set(x as usize, y as usize, d != 2 && d != 4);
                }
            }
        }
    }
    // The alignment pattern.
    for dy in -2i32..=2 {
        for dx in -2i32..=2 {
            q.set((ALIGN as i32 + dx) as usize, (ALIGN as i32 + dy) as usize, dx.abs().max(dy.abs()) != 1);
        }
    }
    // The data, in two-column strips from the bottom right, up then down,
    // past the timing column; the 7 remainder bits stay light.
    let mut i = 0usize;
    let mut right = SIZE - 1;
    loop {
        if right == 6 {
            right = 5;
        }
        let upward = (right + 1) & 2 == 0;
        for vert in 0..SIZE {
            let y = if upward { SIZE - 1 - vert } else { vert };
            for x in [right, right - 1] {
                if function(x, y) {
                    continue;
                }
                let dark = i < codewords.len() * 8 && codewords[i >> 3] >> (7 - (i & 7)) & 1 == 1;
                i += 1;
                q.set(x, y, dark ^ masked(mask, x, y));
            }
        }
        if right < 2 {
            break;
        }
        right -= 2;
    }
    // Format information: level L (01) and the mask, BCH(15, 5), masked.
    let data = (0b01u32 << 3) | u32::from(mask);
    let mut rem = data;
    for _ in 0..10 {
        rem = (rem << 1) ^ ((rem >> 9) * 0x537);
    }
    let bits = ((data << 10) | rem) ^ 0x5412;
    let b = |i: usize| bits >> i & 1 == 1;
    for i in 0..6 {
        q.set(8, i, b(i));
    }
    q.set(8, 7, b(6));
    q.set(8, 8, b(7));
    q.set(7, 8, b(8));
    for i in 9..15 {
        q.set(14 - i, 8, b(i));
    }
    for i in 0..8 {
        q.set(SIZE - 1 - i, 8, b(i));
    }
    for i in 8..15 {
        q.set(8, SIZE - 15 + i, b(i));
    }
    // The dark module.
    q.set(8, SIZE - 8, true);
    q
}

// --- Choosing the mask ----------------------------------------------------------

/// ISO/IEC 18004's penalty score (rules 1-4; rule 3 over the matrix
/// alone, no quiet zone).
#[cfg_attr(target_arch = "xtensa", optimize(size))]
fn penalty(q: &Qr) -> u32 {
    let mut p = 0u32;
    // Rules 1 and 3, along rows (`t` false) and columns.
    for t in [false, true] {
        for a in 0..SIZE {
            let at = |b: usize| if t { q.dark(a, b) } else { q.dark(b, a) };
            let mut run = 1u32;
            for b in 1..SIZE {
                if at(b) == at(b - 1) {
                    run += 1;
                    if run == 5 {
                        p += 3;
                    } else if run > 5 {
                        p += 1;
                    }
                } else {
                    run = 1;
                }
            }
            // 1011101 with four light modules on either side.
            const FINDER: [bool; 7] = [true, false, true, true, true, false, true];
            for b in 0..=SIZE - 11 {
                let window = |o: usize| (0..7).all(|k| at(b + o + k) == FINDER[k]);
                let light = |o: usize| (0..4).all(|k| !at(b + o + k));
                if (window(0) && light(7)) || (light(0) && window(4)) {
                    p += 40;
                }
            }
        }
    }
    // Rule 2: 2 x 2 blocks of one colour.
    for y in 0..SIZE - 1 {
        for x in 0..SIZE - 1 {
            let c = q.dark(x, y);
            if c == q.dark(x + 1, y) && c == q.dark(x, y + 1) && c == q.dark(x + 1, y + 1) {
                p += 3;
            }
        }
    }
    // Rule 4: how far the dark share is from half, in steps of 5%.
    let dark: u32 = q.rows.iter().map(|r| r.count_ones()).sum();
    let total = (SIZE * SIZE) as u32;
    let k = (dark * 20).abs_diff(total * 10).div_ceil(total).saturating_sub(1);
    p + k * 10
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINK: &[u8] = b"https://opendmd.dev/c#z5bltqg6537uvagizsvc";

    #[test]
    fn the_link_fits_and_longer_does_not() {
        assert_eq!(LINK.len(), 42);
        assert!(encode(LINK).is_some());
        assert!(encode(&[b'a'; MAX_DATA]).is_some());
        assert!(encode(&[b'a'; MAX_DATA + 1]).is_none());
        assert!(encode_with_mask(LINK, 8).is_none());
    }

    #[test]
    fn function_patterns_are_where_they_belong() {
        let q = encode(LINK).unwrap();
        // The finders' corners and centres, their separators light.
        for (x, y) in [(0, 0), (6, 6), (3, 3), (22, 0), (28, 6), (25, 3), (0, 22), (6, 28), (3, 25)] {
            assert!(q.dark(x, y), "({x},{y})");
        }
        for (x, y) in [(7, 0), (7, 7), (0, 7), (21, 0), (21, 7), (0, 21), (7, 21), (1, 1), (5, 5)] {
            assert!(!q.dark(x, y), "({x},{y})");
        }
        // Timing between the finders: alternating.
        for i in 8..21 {
            assert_eq!(q.dark(i, 6), i % 2 == 0);
            assert_eq!(q.dark(6, i), i % 2 == 0);
        }
        // The alignment pattern at (22, 22) and the dark module.
        assert!(q.dark(22, 22) && !q.dark(21, 22) && q.dark(20, 20) && q.dark(24, 24));
        assert!(q.dark(8, SIZE - 8));
    }

    #[test]
    fn format_bits_are_written_twice_and_name_level_l() {
        for mask in 0..8 {
            let q = encode_with_mask(LINK, mask).unwrap();
            let mut a = 0u32;
            let mut b = 0u32;
            let first: [(usize, usize); 15] = [(8, 0), (8, 1), (8, 2), (8, 3), (8, 4), (8, 5), (8, 7), (8, 8), (7, 8), (5, 8), (4, 8), (3, 8), (2, 8), (1, 8), (0, 8)];
            for (i, &(x, y)) in first.iter().enumerate() {
                a |= u32::from(q.dark(x, y)) << i;
            }
            for i in 0..8 {
                b |= u32::from(q.dark(SIZE - 1 - i, 8)) << i;
            }
            for i in 8..15 {
                b |= u32::from(q.dark(8, SIZE - 15 + i)) << i;
            }
            assert_eq!(a, b);
            let f = (a ^ 0x5412) >> 10;
            assert_eq!(f, (0b01 << 3) | u32::from(mask));
        }
    }

    #[test]
    fn reed_solomon_matches_a_known_codeword() {
        // ISO/IEC 18004 Annex I's "01234567" at 1-M is another size;
        // here: the remainder of a codeword with its own EC is zero, i.e.
        // the whole 70 bytes evaluate to 0 at each root.
        let c = codewords(LINK).unwrap();
        let mut root = 1u8;
        for _ in 0..EC_CODEWORDS {
            let mut acc = 0u8;
            for &b in c.iter() {
                acc = gf_mul(acc, root) ^ b;
            }
            assert_eq!(acc, 0);
            root = gf_mul(root, 2);
        }
        // The header: byte mode, 42.
        assert_eq!(c[0], 0x42);
        assert_eq!(c[1] >> 4, 0x0a);
    }

    #[test]
    fn the_best_mask_scores_lowest() {
        let q = encode(LINK).unwrap();
        let p = penalty(&q);
        for m in 0..8 {
            assert!(penalty(&encode_with_mask(LINK, m).unwrap()) >= p);
        }
    }
}

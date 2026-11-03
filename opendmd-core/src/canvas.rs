//! The fixed 128x32 RGB framebuffer.
//!
//! The physical display is two chained 64x32 HUB75 panels, addressed as one
//! 128x32 surface. The size is a compile-time constant on purpose: everything
//! downstream (centring, cropping, the simulator window) depends on it.

/// Display width in pixels.
pub const WIDTH: usize = 128;
/// Display height in pixels.
pub const HEIGHT: usize = 32;
/// Total pixel count.
pub const PIXELS: usize = WIDTH * HEIGHT;

/// An 8-bit-per-channel RGB pixel.
///
/// `#[repr(C)]` is load-bearing, not decoration: it pins the layout to exactly
/// three bytes in `r, g, b` order with no padding, which is what lets
/// [`Canvas::as_bytes`] hand out the pixel buffer as a packed RGB byte slice.
/// `player::GifSource` composites straight into that slice when the GIF's
/// logical screen is exactly `WIDTH x HEIGHT`, saving a second 12 KB buffer on
/// the ESP32. Without `repr(C)` the compiler is free to reorder or pad the
/// fields and that cast would be unsound.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Hash)]
#[repr(C)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

// The guarantee `as_bytes`/`as_mut_bytes` rely on, checked at compile time.
const _: () = assert!(core::mem::size_of::<Rgb>() == 3);
const _: () = assert!(core::mem::align_of::<Rgb>() == 1);

impl Rgb {
    pub const BLACK: Rgb = Rgb { r: 0, g: 0, b: 0 };

    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Rgb { r, g, b }
    }

    /// Pack into `0x00RRGGBB`, the layout `minifb` wants.
    pub const fn to_u32(self) -> u32 {
        ((self.r as u32) << 16) | ((self.g as u32) << 8) | (self.b as u32)
    }
}

/// Which pixels changed since the panel last saw the canvas: one bit per
/// pixel (4,096 bits, 512 B) plus a `u32` summary with bit `y` set when row
/// `y` has any dirty pixel, so a consumer can skip clean rows without looking
/// at their words.
///
/// It exists for the firmware's `Panel::blit`, which pays ~3.3 µs per pixel
/// pushed into the HUB75 DMA buffer through the driver's per-pixel entry
/// point; most GIF and ODM frames repaint a small part of the screen. The
/// decoders mark exactly what they paint (see [`Canvas`] for which methods
/// mark by themselves), the consumer walks the dirty runs and then calls
/// [`clear`](Self::clear). Over-marking is always safe — it only costs blit
/// time — and under-marking leaves stale pixels on the panel, so anything
/// that is unsure marks everything.
#[derive(Clone, Debug)]
pub struct DirtyMap {
    /// Row `y` is words `y * WORDS_PER_ROW ..`, bit `x % 64` of word `x / 64`.
    bits: [u64; PIXELS / 64],
    rows: u32,
}

const WORDS_PER_ROW: usize = WIDTH / 64;
// The row summary is a `u32`, and a row is a whole number of words.
const _: () = assert!(HEIGHT <= 32 && WIDTH.is_multiple_of(64));

impl Default for DirtyMap {
    fn default() -> Self {
        DirtyMap {
            bits: [0; PIXELS / 64],
            rows: 0,
        }
    }
}

impl DirtyMap {
    /// Mark one pixel. Out-of-range coordinates are ignored.
    pub fn mark(&mut self, x: usize, y: usize) {
        if x < WIDTH && y < HEIGHT {
            let i = y * WIDTH + x;
            self.bits[i / 64] |= 1 << (i % 64);
            self.rows |= 1 << y;
        }
    }

    /// Mark `x0..x1` of row `y`, clipped to the canvas.
    pub fn mark_span(&mut self, y: usize, x0: usize, x1: usize) {
        if y >= HEIGHT {
            return;
        }
        let x1 = x1.min(WIDTH);
        if x0 >= x1 {
            return;
        }
        self.mark_run(y * WIDTH + x0, y * WIDTH + x1);
    }

    /// Mark the raster-order pixel indices `start..end` (`y * WIDTH + x`),
    /// which may wrap across rows — the shape of an ODM run. Clipped to the
    /// canvas.
    #[cfg_attr(all(target_os = "espidf", feature = "sram1-iram"), link_section = ".iram1.s1")]
    pub fn mark_run(&mut self, start: usize, end: usize) {
        let end = end.min(PIXELS);
        if start >= end {
            return;
        }
        let (first, last) = (start / 64, (end - 1) / 64);
        let lo = u64::MAX << (start % 64);
        let hi = u64::MAX >> (63 - (end - 1) % 64);
        if first == last {
            self.bits[first] |= lo & hi;
        } else {
            self.bits[first] |= lo;
            for w in &mut self.bits[first + 1..last] {
                *w = u64::MAX;
            }
            self.bits[last] |= hi;
        }
        let (y0, y1) = (start / WIDTH, (end - 1) / WIDTH);
        // Bits y0..=y1; y1 < 32, so the shift below never overflows.
        self.rows |= (u32::MAX >> (31 - y1)) & (u32::MAX << y0);
    }

    /// Mark every pixel.
    pub fn mark_all(&mut self) {
        self.bits = [u64::MAX; PIXELS / 64];
        self.rows = u32::MAX >> (32 - HEIGHT);
    }

    /// Forget row `y` (written to the panel on its own: racing the beam).
    pub fn clear_row(&mut self, y: usize) {
        if y < HEIGHT {
            self.bits[y * WORDS_PER_ROW..(y + 1) * WORDS_PER_ROW].fill(0);
            self.rows &= !(1 << y);
        }
    }

    /// Forget everything; what the consumer calls once it has caught up.
    pub fn clear(&mut self) {
        self.bits = [0; PIXELS / 64];
        self.rows = 0;
    }

    pub fn is_dirty(&self, x: usize, y: usize) -> bool {
        if x >= WIDTH || y >= HEIGHT {
            return false;
        }
        let i = y * WIDTH + x;
        self.bits[i / 64] & (1 << (i % 64)) != 0
    }

    /// Bit `y` set when row `y` has at least one dirty pixel.
    pub fn row_mask(&self) -> u32 {
        self.rows
    }

    /// Row `y` is dirty end to end (a whole row to write).
    #[inline]
    pub fn row_full(&self, y: usize) -> bool {
        y < HEIGHT && self.bits[y * WORDS_PER_ROW..(y + 1) * WORDS_PER_ROW].iter().all(|&w| w == u64::MAX)
    }

    /// Nothing is dirty.
    pub fn is_clean(&self) -> bool {
        self.rows == 0
    }

    /// Dirty pixels in total (a probe statistic, not a hot path).
    pub fn count(&self) -> u32 {
        self.bits.iter().map(|w| w.count_ones()).sum()
    }

    /// The dirty runs of row `y` as half-open `(x0, x1)` spans, left to right.
    #[inline]
    pub fn row_spans(&self, y: usize) -> RowSpans {
        let mut words = [0u64; WORDS_PER_ROW];
        if y < HEIGHT {
            words.copy_from_slice(&self.bits[y * WORDS_PER_ROW..(y + 1) * WORDS_PER_ROW]);
        }
        RowSpans { words, x: 0 }
    }
}

/// Iterator over one row's dirty spans; see [`DirtyMap::row_spans`].
pub struct RowSpans {
    words: [u64; WORDS_PER_ROW],
    x: usize,
}

impl RowSpans {
    fn bit(&self, x: usize) -> bool {
        self.words[x / 64] & (1 << (x % 64)) != 0
    }
}

impl Iterator for RowSpans {
    type Item = (usize, usize);

    fn next(&mut self) -> Option<(usize, usize)> {
        // Skip clean pixels a word at a time, then walk the run.
        while self.x < WIDTH {
            let w = self.words[self.x / 64] >> (self.x % 64);
            if w == 0 {
                self.x = (self.x / 64 + 1) * 64;
                continue;
            }
            self.x += w.trailing_zeros() as usize;
            let start = self.x;
            while self.x < WIDTH {
                let w = !(self.words[self.x / 64]) >> (self.x % 64);
                if w == 0 {
                    self.x = (self.x / 64 + 1) * 64;
                    continue;
                }
                self.x += w.trailing_zeros() as usize;
                break;
            }
            let end = self.x.min(WIDTH);
            debug_assert!(self.bit(start) && (end == WIDTH || !self.bit(end)));
            return Some((start, end));
        }
        None
    }
}

/// A `WIDTH * HEIGHT` framebuffer, row-major from the top-left, plus the
/// [`DirtyMap`] of what changed since the consumer last cleared it.
///
/// [`set`](Self::set), [`fill`](Self::fill) and [`clear`](Self::clear) mark
/// what they write. The raw views ([`as_mut_slice`](Self::as_mut_slice),
/// [`as_mut_bytes`](Self::as_mut_bytes), [`bytes_and_dirty_mut`](Self::bytes_and_dirty_mut))
/// do not: whoever writes through them marks what it wrote. Equality compares
/// pixels only — two canvases showing the same picture are equal whatever
/// their dirty maps say.
#[derive(Clone, Debug)]
pub struct Canvas {
    pixels: Vec<Rgb>,
    dirty: DirtyMap,
}

impl PartialEq for Canvas {
    fn eq(&self, other: &Self) -> bool {
        self.pixels == other.pixels
    }
}

impl Eq for Canvas {}

impl Default for Canvas {
    fn default() -> Self {
        Self::new()
    }
}

impl Canvas {
    pub fn new() -> Self {
        Canvas {
            pixels: vec![Rgb::BLACK; PIXELS],
            dirty: DirtyMap::default(),
        }
    }

    /// A canvas of these pixels (exactly [`PIXELS`] of them) and this dirty
    /// map; how the compositor builds its output from a fallibly allocated
    /// buffer.
    pub fn from_parts(pixels: Vec<Rgb>, dirty: DirtyMap) -> Self {
        assert_eq!(pixels.len(), PIXELS);
        Canvas { pixels, dirty }
    }

    /// Set every pixel to black.
    pub fn clear(&mut self) {
        self.fill(Rgb::BLACK);
    }

    /// Set every pixel to `color`. Marks everything.
    pub fn fill(&mut self, color: Rgb) {
        self.pixels.fill(color);
        self.dirty.mark_all();
    }

    /// Out-of-range coordinates read as black rather than panicking — frame
    /// compositing walks rectangles that may hang off the edge.
    pub fn get(&self, x: usize, y: usize) -> Rgb {
        if x >= WIDTH || y >= HEIGHT {
            return Rgb::BLACK;
        }
        self.pixels[y * WIDTH + x]
    }

    /// Out-of-range writes are dropped. Marks the pixel.
    pub fn set(&mut self, x: usize, y: usize, color: Rgb) {
        if x >= WIDTH || y >= HEIGHT {
            return;
        }
        self.pixels[y * WIDTH + x] = color;
        self.dirty.mark(x, y);
    }

    /// What changed since the last [`DirtyMap::clear`].
    pub fn dirty(&self) -> &DirtyMap {
        &self.dirty
    }

    pub fn dirty_mut(&mut self) -> &mut DirtyMap {
        &mut self.dirty
    }

    /// The packed RGB bytes (see [`as_mut_bytes`](Self::as_mut_bytes)) and
    /// the dirty map at once, so a decoder can paint and mark in one pass.
    pub fn bytes_and_dirty_mut(&mut self) -> (&mut [u8], &mut DirtyMap) {
        let len = self.pixels.len() * 3;
        // SAFETY: as `as_mut_bytes`.
        let bytes =
            unsafe { core::slice::from_raw_parts_mut(self.pixels.as_mut_ptr().cast::<u8>(), len) };
        (bytes, &mut self.dirty)
    }

    /// The pixels and the dirty map at once, for a writer that marks what
    /// it changes.
    pub fn pixels_and_dirty_mut(&mut self) -> (&mut [Rgb], &mut DirtyMap) {
        (&mut self.pixels, &mut self.dirty)
    }

    pub fn as_slice(&self) -> &[Rgb] {
        &self.pixels
    }

    pub fn as_mut_slice(&mut self) -> &mut [Rgb] {
        &mut self.pixels
    }

    /// The pixel buffer as packed RGB bytes, `PIXELS * 3` long.
    pub fn as_bytes(&self) -> &[u8] {
        // SAFETY: `Rgb` is `#[repr(C)]` with three `u8` fields, so it is
        // exactly 3 bytes with alignment 1 (asserted above). A `[Rgb]` is
        // therefore a `[u8]` of three times the length, and `u8` has no
        // invalid bit patterns.
        unsafe {
            core::slice::from_raw_parts(self.pixels.as_ptr().cast::<u8>(), self.pixels.len() * 3)
        }
    }

    /// The pixel buffer as packed RGB bytes, `PIXELS * 3` long. Writing any
    /// byte pattern through this is fine: every `u8` triple is a valid `Rgb`.
    /// Not marked: the writer marks what it wrote.
    pub fn as_mut_bytes(&mut self) -> &mut [u8] {
        // SAFETY: as `as_bytes`, plus every bit pattern is a valid `Rgb`.
        let len = self.pixels.len() * 3;
        unsafe { core::slice::from_raw_parts_mut(self.pixels.as_mut_ptr().cast::<u8>(), len) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canvas_is_exactly_the_panel_size() {
        let c = Canvas::new();
        assert_eq!(c.as_slice().len(), 128 * 32);
        assert!(c.as_slice().iter().all(|p| *p == Rgb::BLACK));
    }

    /// A whole row is dirty end to end only when every pixel of it is:
    /// what the raced pair writer tests instead of walking its spans.
    #[test]
    fn row_full_is_a_whole_dirty_row() {
        let mut d = DirtyMap::default();
        assert!(!d.row_full(3));
        d.mark_span(3, 0, WIDTH - 1);
        assert!(!d.row_full(3));
        d.mark(WIDTH - 1, 3);
        assert!(d.row_full(3) && !d.row_full(4) && !d.row_full(HEIGHT));
        assert_eq!(d.row_full(3), d.row_spans(3).eq([(0, WIDTH)]));
        d.mark_all();
        assert!((0..HEIGHT).all(|y| d.row_full(y)));
    }

    #[test]
    fn set_get_roundtrip_and_bounds() {
        let mut c = Canvas::new();
        c.set(127, 31, Rgb::new(1, 2, 3));
        assert_eq!(c.get(127, 31), Rgb::new(1, 2, 3));
        c.set(128, 0, Rgb::new(9, 9, 9)); // dropped
        assert_eq!(c.get(128, 0), Rgb::BLACK);
        c.clear();
        assert_eq!(c.get(127, 31), Rgb::BLACK);
    }

    #[test]
    fn byte_view_aliases_the_pixels() {
        let mut c = Canvas::new();
        assert_eq!(c.as_bytes().len(), PIXELS * 3);
        c.set(2, 0, Rgb::new(1, 2, 3));
        assert_eq!(&c.as_bytes()[6..9], &[1, 2, 3]);
        c.as_mut_bytes()[3..6].copy_from_slice(&[9, 8, 7]);
        assert_eq!(c.get(1, 0), Rgb::new(9, 8, 7));
        // Last pixel, to pin down the length as well as the order.
        c.set(WIDTH - 1, HEIGHT - 1, Rgb::new(4, 5, 6));
        assert_eq!(&c.as_bytes()[PIXELS * 3 - 3..], &[4, 5, 6]);
    }

    #[test]
    fn set_fill_and_clear_mark_and_equality_ignores_the_map() {
        let mut c = Canvas::new();
        assert!(c.dirty().is_clean());
        c.set(5, 3, Rgb::new(1, 1, 1));
        assert!(c.dirty().is_dirty(5, 3));
        assert!(!c.dirty().is_dirty(6, 3));
        assert_eq!(c.dirty().row_mask(), 1 << 3);
        let mut d = c.clone();
        d.dirty_mut().clear();
        assert_eq!(c, d);
        c.dirty_mut().clear();
        c.clear();
        assert_eq!(c.dirty().count(), PIXELS as u32);
        assert_eq!(c.dirty().row_mask(), u32::MAX);
    }

    #[test]
    fn dirty_spans_and_runs() {
        let mut m = DirtyMap::default();
        m.mark_span(2, 3, 7);
        m.mark_span(2, 60, 70); // across a word boundary
        m.mark_span(2, 126, 500); // clipped
        m.mark(0, 2);
        assert_eq!(
            m.row_spans(2).collect::<Vec<_>>(),
            vec![(0, 1), (3, 7), (60, 70), (126, 128)]
        );
        assert_eq!(m.row_mask(), 1 << 2);
        assert_eq!(m.count(), 1 + 4 + 10 + 2);
        assert_eq!(m.row_spans(3).next(), None);

        // A run wrapping rows 4 -> 6, ODM-style.
        let mut m = DirtyMap::default();
        m.mark_run(4 * WIDTH + 100, 6 * WIDTH + 2);
        assert_eq!(m.row_mask(), 0b111 << 4);
        assert_eq!(m.row_spans(4).collect::<Vec<_>>(), vec![(100, 128)]);
        assert_eq!(m.row_spans(5).collect::<Vec<_>>(), vec![(0, 128)]);
        assert_eq!(m.row_spans(6).collect::<Vec<_>>(), vec![(0, 2)]);
        m.mark_run(PIXELS - 1, PIXELS + 10);
        assert!(m.is_dirty(WIDTH - 1, HEIGHT - 1));
        assert_eq!(m.row_mask() >> 31, 1);

        // Exhaustive against a naive model for every short run.
        for start in 0..300 {
            for len in 0..140 {
                let mut m = DirtyMap::default();
                m.mark_run(start, start + len);
                let mut n = 0;
                for i in 0..PIXELS {
                    let want = i >= start && i < start + len;
                    assert_eq!(
                        m.is_dirty(i % WIDTH, i / WIDTH),
                        want,
                        "{start}+{len} at {i}"
                    );
                    n += want as u32;
                }
                assert_eq!(m.count(), n);
                let rows: u32 = (0..HEIGHT)
                    .filter(|y| (0..WIDTH).any(|x| m.is_dirty(x, *y)))
                    .map(|y| 1 << y)
                    .sum();
                assert_eq!(m.row_mask(), rows);
            }
        }

        let mut m = DirtyMap::default();
        m.mark_all();
        assert_eq!(m.row_spans(31).collect::<Vec<_>>(), vec![(0, 128)]);
        m.clear();
        assert!(m.is_clean() && m.count() == 0);
    }

    #[test]
    fn packs_to_minifb_layout() {
        assert_eq!(Rgb::new(0x12, 0x34, 0x56).to_u32(), 0x0012_3456);
    }
}

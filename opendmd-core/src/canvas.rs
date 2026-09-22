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

/// A `WIDTH * HEIGHT` framebuffer, row-major from the top-left.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Canvas {
    pixels: Vec<Rgb>,
}

impl Default for Canvas {
    fn default() -> Self {
        Self::new()
    }
}

impl Canvas {
    pub fn new() -> Self {
        Canvas {
            pixels: vec![Rgb::BLACK; PIXELS],
        }
    }

    /// Set every pixel to black.
    pub fn clear(&mut self) {
        self.fill(Rgb::BLACK);
    }

    /// Set every pixel to `color`.
    pub fn fill(&mut self, color: Rgb) {
        self.pixels.fill(color);
    }

    /// Out-of-range coordinates read as black rather than panicking — frame
    /// compositing walks rectangles that may hang off the edge.
    pub fn get(&self, x: usize, y: usize) -> Rgb {
        if x >= WIDTH || y >= HEIGHT {
            return Rgb::BLACK;
        }
        self.pixels[y * WIDTH + x]
    }

    /// Out-of-range writes are dropped.
    pub fn set(&mut self, x: usize, y: usize, color: Rgb) {
        if x >= WIDTH || y >= HEIGHT {
            return;
        }
        self.pixels[y * WIDTH + x] = color;
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
    fn packs_to_minifb_layout() {
        assert_eq!(Rgb::new(0x12, 0x34, 0x56).to_u32(), 0x0012_3456);
    }
}

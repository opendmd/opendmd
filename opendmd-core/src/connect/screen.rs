//! The panel's two pairing screens (P1-PLAN 2.6, 4.3), drawn by the
//! firmware's main task from what the network worker computed:
//!
//! - [`pair`]: the QR code of `https://opendmd.dev/c#<S>` at one LED a
//!   module, on a lit quiet zone (the panel's full height: one row above,
//!   two below; four columns either side), with a few words beside it.
//!   Light modules are lit and dark ones off, so a camera sees the usual
//!   dark-on-light code.
//! - [`added`]: "Remote control added" and the new device's name, for 10 s.

use super::qr::{Qr, SIZE};
use crate::canvas::{Canvas, Rgb, HEIGHT, WIDTH};
use crate::text::{self, Font};

/// The quiet zone and the light modules: a plain white (the panel's
/// brightness scales it).
pub const LIGHT: Rgb = Rgb::new(255, 255, 255);
/// The panel's brightness while the QR code is up, whatever the setting
/// (under `max_brightness`): mid, so the lit quiet zone neither blooms
/// into the dark modules nor fades on a camera (P1-PLAN 10.3; the rig's
/// scan, D6, settles it).
pub const PAIR_BRIGHTNESS: u8 = 96;
/// The code's top-left module.
pub const QR_X: usize = 4;
pub const QR_Y: usize = 1;
/// The lit area's width: the code and four columns either side.
pub const ZONE_W: usize = QR_X + SIZE + 4;
const TEXT_X: i32 = ZONE_W as i32 + 4;
const _: () = assert!(QR_Y + SIZE + 2 == HEIGHT && ZONE_W < WIDTH);

/// The words beside the code (the small font: 22 characters fit).
pub const PAIR_LINES: [&str; 3] = ["Scan to control", "this panel from", "your phone"];
pub const PAIR_TITLE: &str = "Remote";
pub const ADDED_TITLE: &str = "Remote control added";

/// The QR screen.
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn pair(canvas: &mut Canvas, qr: &Qr) {
    canvas.clear();
    for y in 0..HEIGHT {
        for x in 0..ZONE_W {
            let (mx, my) = (x.wrapping_sub(QR_X), y.wrapping_sub(QR_Y));
            if !qr.dark(mx, my) {
                canvas.set(x, y, LIGHT);
            }
        }
    }
    text::draw_text_with(canvas, TEXT_X, 2, PAIR_TITLE, text::TITLE_COLOUR, Font::Normal, 1);
    for (i, l) in PAIR_LINES.iter().enumerate() {
        text::draw_text_with(canvas, TEXT_X, 13 + 6 * i as i32, l, text::SUBTITLE_COLOUR, Font::Small, 1);
    }
    canvas.dirty_mut().mark_all();
}

/// "Remote control added" over the device's name (cut to fit).
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn added(canvas: &mut Canvas, label: &str) {
    canvas.clear();
    text::draw_centered(canvas, 7, ADDED_TITLE, text::TITLE_COLOUR, Font::Normal, 1);
    let l = text::fit(Font::Small, label, WIDTH - 8, 1);
    text::draw_centered(canvas, 19, &l, text::SUBTITLE_COLOUR, Font::Small, 1);
    canvas.dirty_mut().mark_all();
}

/// Which LEDs are lit, a row as 128 bits (big-endian hex, x = 0 first): the
/// vectors' copy of a screen, for the decoder check.
pub fn lit_rows(canvas: &Canvas) -> [u128; HEIGHT] {
    let mut rows = [0u128; HEIGHT];
    for (y, r) in rows.iter_mut().enumerate() {
        for x in 0..WIDTH {
            if canvas.get(x, y) != Rgb::new(0, 0, 0) {
                *r |= 1 << (WIDTH - 1 - x);
            }
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connect::qr;

    #[test]
    fn the_code_sits_on_its_quiet_zone_and_the_words_fit() {
        let q = qr::encode(b"https://opendmd.dev/c#z5bltqg6537uvagizsvc").unwrap();
        let mut c = Canvas::new();
        pair(&mut c, &q);
        // The quiet zone: lit all round.
        for x in 0..ZONE_W {
            assert_ne!(c.get(x, 0), Rgb::new(0, 0, 0));
            assert_ne!(c.get(x, HEIGHT - 1), Rgb::new(0, 0, 0));
        }
        for y in 0..HEIGHT {
            for x in (0..QR_X).chain(QR_X + SIZE..ZONE_W) {
                assert_ne!(c.get(x, y), Rgb::new(0, 0, 0));
            }
        }
        // A module for an LED.
        for y in 0..SIZE {
            for x in 0..SIZE {
                assert_eq!(c.get(QR_X + x, QR_Y + y) == Rgb::new(0, 0, 0), q.dark(x, y));
            }
        }
        for l in PAIR_LINES {
            assert!(TEXT_X as usize + text::text_width(Font::Small, l, 1) <= WIDTH, "{l}");
        }
        assert!(TEXT_X as usize + text::text_width(Font::Normal, PAIR_TITLE, 1) <= WIDTH);
        assert!(text::text_width(Font::Normal, ADDED_TITLE, 1) <= WIDTH);
        let rows = lit_rows(&c);
        assert_eq!(rows[0] >> (WIDTH - ZONE_W), (1u128 << ZONE_W) - 1);
    }

    #[test]
    fn added_shows_the_name_cut_to_fit() {
        let mut c = Canvas::new();
        added(&mut c, "A very long name for a phone that goes on");
        assert!(lit_rows(&c).iter().any(|r| *r != 0));
    }
}

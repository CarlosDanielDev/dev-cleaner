//! The logo the header carries: a trash can with a cyan mark inside it, speed
//! lines to its left and a sparkle at its upper right.
//!
//! Drawn from [`PIXELS`], six pixel rows of nine, two pixel rows to a text row.
//! The grid is the owner's pixel art cropped to its content, area-downsampled,
//! and quantized to the two inks it has (`M` magenta, `C` cyan, `.` nothing);
//! the ground is never part of it, so whatever the terminal paints shows
//! through. To regenerate it, run the art through the same crop, a 9×6
//! downsample that keeps a pixel when a tenth of its area is ink, and one ink
//! per text cell (cyan wins a cell when it covers a third as much as magenta).

use ratatui::buffer::Buffer;

use super::palette::{Mode, Theme};

/// Columns the logo takes.
pub const WIDTH: u16 = 9;

/// Text rows the logo takes: two pixel rows each.
pub const HEIGHT: u16 = 3;

/// Pixel rows, `.` for nothing, `M` for magenta and `C` for cyan. Both pixels
/// of a text cell share their ink, so a cell never needs two colours.
const PIXELS: [&str; 6] = [
    "...MM.CC.",
    "..MMMMCCC",
    ".CMCCMM..",
    "CCMCCMM..",
    ".CMCCMM..",
    "..MCCM...",
];

/// Paint the logo with its top-left corner at `x`, `y`.
///
/// Only the cells that hold ink are touched, so the ground under the rest is
/// whatever the frame already painted. The inks are the theme's own: the head
/// colour for the can and the accent for the mark, speed lines and sparkle.
///
/// With no colour there is nothing to tell the two inks apart, so the mark
/// inside the can is drawn in a lighter glyph where it fills a whole cell.
pub fn draw(theme: &Theme, buf: &mut Buffer, x: u16, y: u16) {
    let mono = theme.mode() == Mode::Mono;
    for row in 0..HEIGHT {
        let (top, bottom) = (PIXELS[2 * row as usize], PIXELS[2 * row as usize + 1]);
        for (col, (t, b)) in top.chars().zip(bottom.chars()).enumerate() {
            let ink = if t == '.' { b } else { t };
            let glyph = match (t != '.', b != '.') {
                (false, false) => continue,
                (true, false) => '▀',
                (false, true) => '▄',
                (true, true) if mono && ink == 'C' => '▒',
                (true, true) => '█',
            };
            let style = if ink == 'M' { theme.head } else { theme.accent };
            if let Some(cell) = buf.cell_mut((x + col as u16, y + row)) {
                cell.set_char(glyph);
                if let Some(fg) = style.fg {
                    cell.set_fg(fg);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_grid_is_a_rectangle_of_two_inks() {
        assert_eq!(PIXELS.len(), 2 * HEIGHT as usize);
        for row in PIXELS {
            assert_eq!(row.chars().count(), WIDTH as usize, "{row}");
            assert!(row.chars().all(|c| matches!(c, '.' | 'M' | 'C')), "{row}");
        }
    }

    #[test]
    fn the_two_pixels_of_a_cell_share_their_ink() {
        for pair in PIXELS.chunks(2) {
            for (t, b) in pair[0].chars().zip(pair[1].chars()) {
                assert!(t == b || t == '.' || b == '.', "{} over {}", t, b);
            }
        }
    }

    #[test]
    fn the_source_is_small() {
        let source = include_str!("logo.rs");
        let code = source.split("#[cfg(test)]").next().unwrap_or(source);
        assert!(code.len() < 4096, "{} bytes", code.len());
    }
}

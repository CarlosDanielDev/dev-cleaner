//! The logo: a trash can with a lid and a handle, a cyan `</>` on its body,
//! speed lines to its left and two sparkles at its upper right.
//!
//! Two drawings of the same art, both as rows of `.` (nothing), `M` (magenta,
//! the can) and `C` (cyan, the rest); the ground is never part of them, so
//! whatever the terminal paints shows through. Half blocks give two pixel rows
//! to a text row and two inks to a cell: `▀` with the upper pixel as its
//! foreground and the lower one as its background.
//!
//! - [`MASTER`] is the whole art, drawn by the scan while it runs.
//! - [`HEADER`] is the smallest size at which the lid, the handle, the three
//!   strokes of `</>`, the speed lines and the large sparkle each still read.
//!   At 10 pixels the can turned into a face; at 14 it is the logo.

use ratatui::buffer::Buffer;

use super::palette::{Mode, Theme};

/// Columns the header's logo takes.
pub const WIDTH: u16 = 24;

/// Text rows the header's logo takes: two pixel rows each.
pub const HEIGHT: u16 = 7;

/// The smallest terminal whose header grows to [`HEIGHT`] rows.
///
/// Under it the header is the two rows of text it was before the logo, and no
/// logo is drawn: one that cannot read is worse than none. Seven rows of
/// header leave the body the 25 rows it had at 90x30, which is more than the
/// 20 every screen is laid out for.
pub const MIN_COLS: u16 = 100;
pub const MIN_ROWS: u16 = 34;

/// Columns and text rows the master takes.
pub const MASTER_WIDTH: u16 = 50;
pub const MASTER_HEIGHT: u16 = 17;

/// The header's logo: the master at 14 pixels tall, 24 wide, drawn by hand.
///
/// A reduction of the master by block mode kept the can and lost the lid, the
/// handle and every thin stroke, so each feature was placed pixel by pixel:
/// the handle ring, the lid with its two hooks, the body with its rim, the
/// three strokes of `</>` kept apart by a column each, four speed lines and
/// two sparkles.
pub const HEADER: [&str; 14] = [
    ".....................C..",
    ".........MMMMM......CCC.",
    ".........M...M.......C..",
    "....MMMMMMMMMMMMMMM.....",
    "....M.............M...C.",
    ".....................CCC",
    ".CCC.MMMMMMMMMMMMM....C.",
    ".....M......C....M......",
    "C.CC.M..C...C.C..M......",
    ".....M.C...C...C.M......",
    ".CCC.M..C.C...C..M......",
    "......M...C.....M.......",
    "..CC...M.......M........",
    "........MMMMMMM.........",
];

/// The master: the owner's art on a grid of 50 by 34 pixels, one ink to a
/// pixel. `.` is nothing, `M` the can and `C` the cyan parts. The art was
/// drawn on a grid that is not quite regular, so the rows were read off it
/// feature by feature and every stroke snapped to whole pixels, thin ones kept.
pub const MASTER: [&str; 34] = [
    "........................................CC........",
    "........................................CC........",
    "......................................CCCCCC......",
    "....................................CCCCCCCCCC....",
    "...................MMMMMMMMMMM......CCCCCCCCCC....",
    "..................MM.........MM.......CCCCCC......",
    "..................MM.........MM.........CC........",
    "........................................CC........",
    "...............................................C..",
    ".............MMMMMMMMMMMMMMMMMMMMMMM.........CCCCC",
    "...........MMMMMMMMMMMMMMMMMMMMMMMMMMM.........C..",
    "...........MM.......................MM............",
    "..................................................",
    ".............MMMMMMMMMMMMMMMMMMMMMMM..............",
    ".............MM...................MM..............",
    "..............MM.................MM...............",
    "......CCCCC...MM.................MM...............",
    "..............MM.................MM...............",
    "..............MM.................MM...............",
    "..............MM.........CC......MM...............",
    "C..CCCCCCCCC..MM....C....CC.C....MM...............",
    "..............MM...C....CC...C...MM...............",
    "..............MM..C.....CC....C..MM...............",
    "..............MM.CC....CC.....CC.MM...............",
    "......CCCCCC..MM..C....CC.....C..MM...............",
    "..............MM...C..CC.....C...MM...............",
    "..............MM....C.CC....C....MM...............",
    "...............MM.....CC........MM................",
    "........CCC....MM...............MM................",
    "...............MM...............MM................",
    "...............MM...............MM................",
    "................MM.............MM.................",
    ".................MMMMMMMMMMMMMMM..................",
    "..................MMMMMMMMMMMMM...................",
];

/// Paint the header's logo with its top-left corner at `x`, `y`.
pub fn draw(theme: &Theme, buf: &mut Buffer, x: u16, y: u16) {
    paint(theme, buf, x, y, &HEADER);
}

/// Paint the whole art with its top-left corner at `x`, `y`.
pub fn draw_master(theme: &Theme, buf: &mut Buffer, x: u16, y: u16) {
    paint(theme, buf, x, y, &MASTER);
}

/// Paint `art`, touching only the cells that hold ink. The inks are the
/// theme's own: the head colour for the can, the accent for the rest.
///
/// With no colour there is nothing to tell the inks apart, so the can is the
/// full block and half blocks and the rest a lighter glyph, which still draws
/// a can with something in it. A cell that holds both is a lighter one.
fn paint(theme: &Theme, buf: &mut Buffer, x: u16, y: u16, art: &[&str]) {
    let mono = theme.mode() == Mode::Mono;
    let ink = |c: u8| match c {
        b'M' => theme.head.fg,
        _ => theme.accent.fg,
    };
    for (row, pair) in art.chunks(2).enumerate() {
        let bottom = pair.get(1).map_or(&[][..], |r| r.as_bytes());
        for (col, &t) in pair[0].as_bytes().iter().enumerate() {
            let b = bottom.get(col).copied().unwrap_or(b'.');
            let glyph = match (t != b'.', b != b'.') {
                (false, false) => continue,
                _ if mono && (t == b'C' || b == b'C') => '▒',
                (true, false) => '▀',
                (false, true) => '▄',
                (true, true) if t == b || mono => '█',
                (true, true) => '▀',
            };
            let Some(cell) = buf.cell_mut((x + col as u16, y + row as u16)) else {
                continue;
            };
            cell.set_char(glyph);
            let (upper, lower) = if t == b'.' { (b, b) } else { (t, b) };
            if let Some(fg) = ink(upper) {
                cell.set_fg(fg);
            }
            // Only a cell that holds two different inks needs a background.
            if lower != b'.'
                && lower != upper
                && let Some(bg) = ink(lower)
            {
                cell.set_bg(bg);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rectangle(art: &[&str], width: u16, rows: u16) {
        assert_eq!(art.len(), rows as usize);
        for row in art {
            assert_eq!(row.chars().count(), width as usize, "{row}");
            assert!(row.chars().all(|c| matches!(c, '.' | 'M' | 'C')), "{row}");
        }
    }

    #[test]
    fn the_master_is_the_art_on_its_own_grid() {
        rectangle(&MASTER, MASTER_WIDTH, 2 * MASTER_HEIGHT);
        let ink = |c: char| {
            MASTER
                .iter()
                .flat_map(|r| r.chars())
                .filter(|&x| x == c)
                .count()
        };
        assert!(
            ink('M') > 100 && ink('C') > 100,
            "{} {}",
            ink('M'),
            ink('C')
        );
    }

    #[test]
    fn the_mark_is_cyan_inside_the_cans_bounds() {
        // The body: the rows under the lid, from the rim down.
        let body = &MASTER[13..];
        let can: Vec<usize> = body
            .iter()
            .flat_map(|r| r.chars().enumerate().filter(|(_, c)| *c == 'M'))
            .map(|(i, _)| i)
            .collect();
        let (left, right) = (*can.iter().min().unwrap(), *can.iter().max().unwrap());
        let inside = body
            .iter()
            .flat_map(|r| r.chars().enumerate())
            .filter(|&(i, c)| c == 'C' && i > left && i < right)
            .count();
        assert!(inside >= 20, "{inside} cyan pixels inside the can");
    }

    #[test]
    fn the_header_art_is_a_rectangle_of_two_inks() {
        rectangle(&HEADER, WIDTH, 2 * HEIGHT);
    }

    #[test]
    fn the_header_keeps_every_feature_of_the_logo() {
        let has =
            |rows: std::ops::Range<usize>, c: char| HEADER[rows].iter().any(|r| r.contains(c));
        // The handle is a ring above the lid, the lid wider than the body.
        assert!(HEADER[1].contains("MMMMM") && HEADER[2].matches('M').count() == 2);
        assert!(HEADER[3].matches('M').count() > HEADER[6].matches('M').count());
        // Three strokes of `</>` with a clear column between each.
        for row in &HEADER[8..11] {
            assert!(row.matches('C').count() >= 2, "{row}");
        }
        assert!(has(7..12, 'C'));
        // Four speed lines left of the can, one of them a lone square.
        let lines = (0..14).filter(|&y| HEADER[y][..4].contains('C')).count();
        assert_eq!(lines, 4);
        // The larger sparkle is the widest cyan mark on the right.
        assert!(HEADER[1].ends_with("CCC."));
    }

    #[test]
    fn the_source_is_small() {
        let source = include_str!("logo.rs");
        let code = source.split("#[cfg(test)]").next().unwrap_or(source);
        assert!(code.len() < 8192, "{} bytes", code.len());
    }
}

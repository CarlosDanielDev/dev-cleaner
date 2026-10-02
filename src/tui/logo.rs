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
//! - [`ICON`] is the mark beside the name in the header: 28 by 20 pixels drawn
//!   by hand, as braille, two dots across and four down in a cell, so the five
//!   rows the header gives it hold the lid, the handle, the `</>`, the speed
//!   lines and both sparkles. A cell has one ink, so the art keeps the two
//!   inks in cells of their own.
//! - [`FALLBACK`] is the same mark for a terminal whose font has no braille
//!   (`TERM=linux|dumb`): 10 by 10 pixels in half blocks, the same five rows.
//!
//! The name is [`wordmark`]: the same two inks, run through the letters.

use ratatui::buffer::Buffer;
use ratatui::style::{Modifier, Style};

use super::palette::{Mode, Theme};

/// Columns the braille icon takes: two dot columns to a cell.
pub const WIDTH: u16 = 14;

/// Columns the half-block fallback takes. A text cell is two pixel rows high,
/// so a square icon is as many columns wide as it is pixels tall.
pub const FALLBACK_WIDTH: u16 = 10;

/// Text rows either icon takes, and so the header's height when it has one.
pub const HEIGHT: u16 = 5;

/// Rows above the body when the header has its icon: the icon's rows and one
/// blank row (on the screens with a danger band, the band) before the first
/// section.
pub const TOP: u16 = HEIGHT + 1;

/// Columns between the icon and the text beside it.
pub const GAP: u16 = 2;

/// The smallest terminal whose header grows to [`TOP`] rows.
///
/// Under it the header is the two rows of text it was before the logo, and no
/// icon is drawn: one that is cropped or squashed is worse than none. Six
/// rows above the body leave it the 20 rows it has at 90x28, which is what
/// every screen is laid out for.
pub const MIN_COLS: u16 = 90;
pub const MIN_ROWS: u16 = 28;

/// Columns and text rows the master takes.
pub const MASTER_WIDTH: u16 = 50;
pub const MASTER_HEIGHT: u16 = 17;

/// The name in the header, as drawn by [`wordmark`].
pub const NAME: &str = "dev-cleaner";

/// The icon: 28 by 20 pixels, drawn by hand on the grid braille gives, which is
/// square: a cell is half as wide as it is tall, and holds two dots by four.
/// Every cell holds one ink, so no stroke of one ink shares a cell with the
/// other.
pub const ICON: [&str; 20] = [
    ".......................C....",
    ".......................C....",
    "...........MMMMMM....CCCCC..",
    "..........M......M.....C....",
    "..........M......M.....C....",
    "......MMMMMMMMMMMMMMMM......",
    "......MMMMMMMMMMMMMMMM....C.",
    "......M..............M...CCC",
    "..........................C.",
    ".......MMMMMMMMMMMMMM.......",
    "...CCC..M..........M........",
    "........M..........M........",
    "C.CCCC..M..C..C.C..M........",
    "........M.C...C..C.M........",
    "...CCCC.M..C.C..C..M........",
    "........M....C.....M........",
    "....CCC.M..........M........",
    ".........M........M.........",
    "..........M......M..........",
    "..........MMMMMMMM..........",
];

/// The icon for a font with no braille: 10 by 10 pixels, drawn by hand. Eight
/// pixels turned the can into a bottle and twelve cost two more rows for
/// nothing the ten did not say.
pub const FALLBACK: [&str; 10] = [
    "........C.",
    "...MMM.CCC",
    "...M.M..C.",
    "MMMMMMMMM.",
    ".MMMMMMM..",
    ".M...C.M..",
    ".M..C..M..",
    ".M.C...M..",
    "..M...M...",
    "..MMMMM...",
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

/// Columns the icon takes in `theme`'s look: [`WIDTH`] in braille,
/// [`FALLBACK_WIDTH`] in half blocks.
pub fn width(theme: &Theme) -> u16 {
    if theme.braille() {
        WIDTH
    } else {
        FALLBACK_WIDTH
    }
}

/// Paint the icon with its top-left corner at `x`, `y`: in braille, or in half
/// blocks where the look has no braille.
pub fn draw(theme: &Theme, buf: &mut Buffer, x: u16, y: u16) {
    if theme.braille() {
        braille(theme, buf, x, y);
    } else {
        paint(theme, buf, x, y, &FALLBACK);
    }
}

/// Whether text `widest` columns long, drawn from `x` on, still ends a column
/// short of `width`: the text beside the icon wins over the icon.
pub fn fits(width: u16, x: u16, widest: usize) -> bool {
    x as usize + widest < width as usize
}

/// [`NAME`] a letter at a time, each in its place of the gradient: the icon's
/// cyan, through violet, to its magenta. The hyphen is no letter and no step of
/// it: it is the quiet one.
pub fn wordmark(theme: &Theme) -> Vec<(String, Style)> {
    let letters = NAME.chars().filter(|&c| c != '-').count();
    let mut at = 0;
    NAME.chars()
        .map(|c| {
            if c == '-' {
                return (c.to_string(), theme.muted);
            }
            at += 1;
            (
                c.to_string(),
                theme.brand((at - 1) as f32 / (letters - 1) as f32),
            )
        })
        .collect()
}

/// Paint the whole art with its top-left corner at `x`, `y`.
pub fn draw_master(theme: &Theme, buf: &mut Buffer, x: u16, y: u16) {
    paint(theme, buf, x, y, &MASTER);
}

/// The icon as braille: a cell is the dots of two columns by four rows of
/// [`ICON`], in the ink most of them are. Only a cell that holds a dot is
/// touched, as the ground shows through everywhere else. With no colour there
/// is only the shape.
fn braille(theme: &Theme, buf: &mut Buffer, x: u16, y: u16) {
    // The bit of the dot at column `dx` and row `dy` of a braille cell.
    const DOT: [[u32; 4]; 2] = [[0x01, 0x02, 0x04, 0x40], [0x08, 0x10, 0x20, 0x80]];
    for (row, quad) in ICON.chunks(4).enumerate() {
        for col in 0..WIDTH as usize {
            let (mut bits, mut magenta, mut cyan) = (0, 0, 0);
            for (dy, line) in quad.iter().enumerate() {
                for (dx, dots) in DOT.iter().enumerate() {
                    match line.as_bytes()[2 * col + dx] {
                        b'.' => continue,
                        b'M' => magenta += 1,
                        _ => cyan += 1,
                    }
                    bits |= dots[dy];
                }
            }
            let Some(glyph) = char::from_u32(0x2800 + bits).filter(|_| bits != 0) else {
                continue;
            };
            let Some(cell) = buf.cell_mut((x + col as u16, y + row as u16)) else {
                continue;
            };
            cell.set_char(glyph);
            // Full strength: the ink the theme gives its heads, in bold where the
            // terminal has it.
            cell.modifier.insert(Modifier::BOLD);
            if let Some(fg) = if magenta >= cyan {
                theme.head
            } else {
                theme.accent
            }
            .fg
            {
                cell.set_fg(fg);
            }
        }
    }
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

    fn ink(art: &[&str], c: char) -> usize {
        art.iter()
            .flat_map(|r| r.chars())
            .filter(|&x| x == c)
            .count()
    }

    #[test]
    fn the_icon_is_square_pixels_in_two_inks() {
        // Braille: two dots across a cell and four down, and a cell is half as
        // wide as it is tall, so a dot is square.
        rectangle(&ICON, 2 * WIDTH, 4 * HEIGHT);
        assert!(ink(&ICON, 'M') > 50 && ink(&ICON, 'C') > 30);
        // The half-block fallback: two pixels down a row, one across a column.
        rectangle(&FALLBACK, FALLBACK_WIDTH, 2 * HEIGHT);
        assert_eq!(FALLBACK_WIDTH as usize, FALLBACK.len());
        assert!(ink(&FALLBACK, 'M') > 20 && ink(&FALLBACK, 'C') > 5);
    }

    #[test]
    fn a_cell_of_the_icon_holds_one_ink() {
        // Braille gives a cell one colour: a cell that held both would lose the
        // lesser, and the art is drawn so that none does.
        for (row, quad) in ICON.chunks(4).enumerate() {
            for col in 0..WIDTH as usize {
                let inks: std::collections::HashSet<u8> = quad
                    .iter()
                    .flat_map(|l| l.as_bytes()[2 * col..2 * col + 2].iter().copied())
                    .filter(|&b| b != b'.')
                    .collect();
                assert!(inks.len() <= 1, "cell {col},{row} holds {inks:?}");
            }
        }
    }

    #[test]
    fn the_icon_keeps_the_lid_the_handle_the_stroke_the_lines_and_both_sparkles() {
        // The lid is the widest run of the can, and the handle a ring above it.
        let widest = ICON.iter().map(|r| r.matches('M').count()).max();
        assert_eq!(widest, Some(ICON[5].matches('M').count()));
        assert!(ICON[2].contains("MMMMMM") && ICON[3].matches('M').count() == 2);
        // The `</>` is cyan between the can's walls, on four rows.
        let walls = |r: &str| (r.find('M'), r.rfind('M'));
        for row in &ICON[12..15] {
            let (l, r) = walls(row);
            let inside = row
                .char_indices()
                .filter(|&(i, c)| c == 'C' && Some(i) > l && Some(i) < r);
            assert!(inside.count() >= 3, "{row}");
        }
        // Speed lines to its left: cyan, left of the can's wall.
        assert!(
            ICON[10..17]
                .iter()
                .filter(|r| r.find('C') < r.find('M'))
                .count()
                >= 4
        );
        // Two sparkles at the upper right, both plus signs.
        assert!(ICON[2].ends_with("CCCCC..") && ICON[7].ends_with("CCC"));
    }

    #[test]
    fn the_text_wins_over_the_icon_by_a_column() {
        assert!(fits(100, 13, 86) && !fits(100, 13, 87));
        assert!(!fits(90, 13, 77) && fits(90, 13, 76));
    }

    #[test]
    fn the_wordmark_spells_the_name_and_keeps_the_hyphen_quiet() {
        let theme = Theme::neon();
        let parts = wordmark(&theme);
        let name: String = parts.iter().map(|(c, _)| c.as_str()).collect();
        assert_eq!(name, NAME);
        assert_eq!(parts[3].1, theme.muted);
        assert!(parts.iter().filter(|(_, s)| *s != theme.muted).count() == 10);
    }

    #[test]
    fn the_master_is_what_the_scan_has_always_drawn() {
        // 50 by 34 pixels, and every row of it: the scan splash does not move.
        let mut hash: u64 = 0xcbf29ce484222325;
        for byte in MASTER.iter().flat_map(|r| r.bytes().chain(*b"\n")) {
            hash = (hash ^ byte as u64).wrapping_mul(0x100000001b3);
        }
        assert_eq!((MASTER_WIDTH, MASTER_HEIGHT), (50, 17));
        assert_eq!(hash, 0x83dc32731058a55a, "the master changed");
    }

    #[test]
    fn the_source_is_small() {
        let source = include_str!("logo.rs");
        let code = source.split("#[cfg(test)]").next().unwrap_or(source);
        assert!(code.len() < 12288, "{} bytes", code.len());
    }
}

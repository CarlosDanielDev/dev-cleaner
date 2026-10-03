//! A dialog: a bordered box over the body that asks one question.
//!
//! The notice row's two-press pattern is for cheap questions that fit a line.
//! This is for a question that needs facts, and it is modal: whoever draws it
//! also owns the key bar while it is up and draws the same keys there, so the
//! two cannot disagree about what a key does.

use super::super::palette::Theme;
use super::super::result::wrap;
use super::super::row::put;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

/// The widest the box is drawn: a longer line is read from one end to the other.
const WIDTH: u16 = 64;

/// Columns the border and its padding take from each side of the text.
const MARGIN: u16 = 2;

/// The corners and sides of the box, in the theme's own line weight.
fn frame(theme: &Theme) -> (char, char, [char; 4]) {
    if theme.rule() == '═' {
        ('═', '║', ['╔', '╗', '╚', '╝'])
    } else {
        ('─', '│', ['┌', '┐', '└', '┘'])
    }
}

/// Draw the dialog centred in `area`: `title` in the top edge, each of
/// `paragraphs` wrapped to the box with a blank row between, and the `keys` as
/// caps on the last row. What does not fit is left out from the end of the
/// text, never from the keys.
pub(in crate::tui) fn dialog(
    buf: &mut Buffer,
    theme: &Theme,
    area: Rect,
    title: &str,
    paragraphs: &[String],
    keys: &[(&str, &str)],
) {
    if area.width < 2 * MARGIN + 8 || area.height < 5 {
        return;
    }
    let width = area.width.min(WIDTH + 2 * MARGIN);
    let text_width = (width - 2 * MARGIN) as usize;
    let mut text: Vec<String> = Vec::new();
    for paragraph in paragraphs {
        if !text.is_empty() {
            text.push(String::new());
        }
        text.extend(wrap(paragraph, text_width));
    }
    // Top edge, bottom edge, a blank row and the keys.
    let room = (area.height as usize).saturating_sub(4);
    text.truncate(room);
    let height = text.len() as u16 + 4;
    let rect = Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    );

    // The body underneath must not show through the gaps between words.
    for y in rect.y..rect.bottom() {
        for x in rect.x..rect.right() {
            buf[(x, y)].reset();
        }
    }
    buf.set_style(rect, theme.ground);

    let (across, down, [tl, tr, bl, br]) = frame(theme);
    let inner = (width - 2) as usize;
    let top = format!(" {title} ");
    buf.set_string(
        rect.x,
        rect.y,
        format!("{tl}{}{tr}", across.to_string().repeat(inner)),
        theme.violet,
    );
    buf.set_string(rect.x + 2, rect.y, &top, theme.head);
    buf.set_string(
        rect.x,
        rect.bottom() - 1,
        format!("{bl}{}{br}", across.to_string().repeat(inner)),
        theme.violet,
    );
    for y in rect.y + 1..rect.bottom() - 1 {
        buf.set_string(rect.x, y, down.to_string(), theme.violet);
        buf.set_string(rect.right() - 1, y, down.to_string(), theme.violet);
    }
    for (i, line) in text.iter().enumerate() {
        buf.set_string(rect.x + MARGIN, rect.y + 1 + i as u16, line, theme.text);
    }
    let (open, close) = theme.caps();
    let mut x = rect.x + MARGIN;
    let y = rect.bottom() - 2;
    for (i, (key, label)) in keys.iter().enumerate() {
        if i > 0 {
            x = put(buf, x, y, &[("   ", theme.text)]);
        }
        x = put(
            buf,
            x,
            y,
            &[
                (open, theme.muted),
                (key, theme.key),
                (close, theme.muted),
                (" ", theme.text),
                (label, theme.muted),
            ],
        );
    }
}

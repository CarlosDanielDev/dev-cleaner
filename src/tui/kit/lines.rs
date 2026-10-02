//! The lines around the rows of a table screen.

use super::super::palette::Theme;
use super::super::row::{elide_path, elide_tail, heading};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;

/// Where a line of keys begins in an empty body, so it is drawn quieter than
/// the facts above it.
pub(in crate::tui) const KEYS_LEAD: &str = "Keys: ";

/// A section's head: a title in the head role, what it counts in the text role,
/// anything `extra` adds after it, and a rule out to the edge, so the sections of a screen separate where the
/// eye is already moving. Returns the row after it.
pub(in crate::tui) fn section(
    buf: &mut Buffer,
    theme: &Theme,
    (x, y): (u16, u16),
    width: usize,
    title: &str,
    counts: &str,
    extra: Vec<(String, Style)>,
) -> u16 {
    let mut parts: Vec<(String, Style)> = vec![(title.to_string(), theme.head)];
    if !counts.is_empty() {
        parts.push((format!("  {counts}"), theme.text));
    }
    parts.extend(extra);
    heading(buf, theme, x, y, width, &parts)
}

/// The view bar: part of the screen, so it never times out. Lit when the view
/// is anything but the one the screen opens on.
pub(in crate::tui) fn view_bar(
    buf: &mut Buffer,
    theme: &Theme,
    (x, y): (u16, u16),
    line: &str,
    lit: bool,
) {
    buf.set_string(x, y, line, if lit { theme.head } else { theme.text });
}

/// The cursor band, across the whole row, gaps included: highlighted cell by
/// cell it reads as separate blocks rather than as one line under a cursor.
pub(in crate::tui) fn band(buf: &mut Buffer, theme: &Theme, area: Rect, y: u16) {
    buf.set_style(Rect::new(area.x, y, area.width, 1), theme.selected);
}

/// `label value` pairs, one after the other, for the selected row. A pair with
/// no label is just its value.
pub(in crate::tui) fn facts(pairs: &[(&str, String)]) -> String {
    pairs
        .iter()
        .filter(|(_, v)| !v.is_empty())
        .map(|(l, v)| {
            if l.is_empty() {
                v.clone()
            } else {
                format!("{l} {v}")
            }
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

/// The selected row in full: `facts`, then the whole path, cut from the left
/// only when even that cannot fit. Telling rows apart never depends on how wide
/// a column was.
pub(in crate::tui) fn detail(facts: &str, path: &str, width: usize) -> String {
    let room = width.saturating_sub(facts.chars().count() + 3);
    format!("{facts} · {}", elide_path(path, room.max(8)))
}

/// [`detail`] for a line that may have more facts than room: they are cut with
/// a mark before the path is, because the path is what the facts are about and
/// the facts are what the rest of the row already said.
pub(in crate::tui) fn detail_line(facts: &str, path: &str, width: usize) -> String {
    let most = width
        .saturating_sub(3 + path.chars().count())
        .max((width / 2).saturating_sub(3));
    detail(&elide_tail(facts, most), path, width)
}

/// Draw the detail line on the second to last row of `area`.
pub(in crate::tui) fn put_detail(buf: &mut Buffer, theme: &Theme, area: Rect, line: &str) {
    buf.set_string(area.x + 1, area.bottom() - 2, line, theme.text);
}

/// Draw the position line on the last row of `area`, cut with a mark.
pub(in crate::tui) fn position(buf: &mut Buffer, theme: &Theme, area: Rect, line: &str) {
    buf.set_string(
        area.x + 1,
        area.bottom() - 1,
        elide_tail(line, area.width.saturating_sub(1) as usize),
        theme.text,
    );
}

/// What an empty view says in place of its rows: what is true, where something
/// is, and the keys that get there. One row each from `y`, no more than `room`,
/// each cut with a mark to `width`.
pub(in crate::tui) fn empty_body(
    buf: &mut Buffer,
    theme: &Theme,
    (x, y): (u16, u16),
    (width, room): (usize, usize),
    lines: &[String],
) {
    for (i, line) in lines.iter().take(room).enumerate() {
        let style = if line.starts_with(KEYS_LEAD) {
            theme.muted
        } else {
            theme.text
        };
        buf.set_string(x, y + i as u16, elide_tail(line, width), style);
    }
}

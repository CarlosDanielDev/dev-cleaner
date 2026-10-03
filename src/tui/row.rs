//! Laying out a row of facts about a path.
//!
//! Shared by the candidates screen and the review screen, which draw the same
//! shape: a size, a path that will not fit, and a sentence about how the path
//! comes back. One definition, because a second copy is how the two screens
//! come to disagree about what a row says.

use super::palette::Theme;
use crate::bytes::human;
use crate::safety::{Candidate, Safety};
use ratatui::buffer::Buffer;
use ratatui::style::Style;

/// Columns between the path and the column after it.
const GAP: usize = 2;

/// Width for the path column, where the right-hand column starts, and the
/// width it has.
///
/// The right column is sized to its own longest entry so the fact it carries
/// arrives whole; the path takes what is left, no more than its longest entry
/// needs, so the right column sits beside the paths it belongs to and not out
/// at the edge of the screen. A path can be shortened and still identify its
/// entry while a half-sentence cannot. The column has a ceiling all the same,
/// and an entry past it is cut: the width comes back so the caller can say so
/// rather than let the buffer edge cut it in silence.
pub(super) fn columns(
    left: u16,
    width: usize,
    prefix: usize,
    longest_path: usize,
    entries: &[String],
) -> (usize, u16, usize) {
    let longest = entries.iter().map(|e| e.chars().count()).max().unwrap_or(0);
    let desc_w = longest.clamp(0, width * 3 / 5);
    let path_w = longest_path.min(width.saturating_sub(prefix + desc_w + GAP));
    // What is left after the path, not `desc_w`: on a screen too narrow for
    // the prefix and the column together the path has already given up its
    // width, and the column gets the remainder, not its ceiling.
    let desc_w = width.saturating_sub(prefix + path_w + GAP);
    (path_w, left + (prefix + path_w + GAP) as u16, desc_w)
}

/// The width of the longest of `paths`, as the screen shows them.
pub(super) fn widest<'a>(paths: impl IntoIterator<Item = &'a std::path::Path>) -> usize {
    paths
        .into_iter()
        .map(|p| p.display().to_string().chars().count())
        .max()
        .unwrap_or(0)
}

/// Fit a path into `width`, keeping the end and marking what was dropped.
///
/// The end is what identifies a path: which project, which directory. Cutting
/// the tail leaves rows that cannot be told apart, and cutting either end
/// without the mark reads as a shorter path that exists.
pub(super) fn elide_path(text: &str, width: usize) -> String {
    let count = text.chars().count();
    if count <= width {
        return text.to_string();
    }
    if width <= 1 {
        return "…".repeat(width);
    }
    let tail: String = text.chars().skip(count - (width - 1)).collect();
    format!("…{tail}")
}

/// Fit a command or a reason into `width`, keeping the start and marking what
/// was dropped.
///
/// The other way round from a path: the start is what identifies a command
/// (`cargo build`, `npm install`) and states a reason's rule. Cutting the
/// start leaves a fragment, and cutting the end without the mark reads as a
/// sentence that ended there.
pub(super) fn elide_tail(text: &str, width: usize) -> String {
    let count = text.chars().count();
    if count <= width {
        return text.to_string();
    }
    if width <= 1 {
        return "…".repeat(width);
    }
    let head: String = text.chars().take(width - 1).collect();
    format!("{head}…")
}

/// What this tier means, in the user's words.
pub(super) fn describe(safety: &Safety) -> String {
    match safety {
        Safety::Cache { refills_on } => format!("refills on {refills_on}"),
        Safety::Regenerable { regen } => regen.to_string(),
        Safety::Unproven { reason } => reason.clone(),
        Safety::Protected { reason } => reason.explain().to_string(),
    }
}

/// Draw plan entries one per row from `y`, stopping short of `bottom`, and
/// return the row after the last one drawn.
///
/// Shared by the review and confirm screens, so the last look before a purge
/// says of each entry exactly what the plan said of it.
pub(super) fn plan_rows(
    theme: &Theme,
    items: &[&Candidate],
    left: u16,
    width: usize,
    mut y: u16,
    bottom: u16,
    buf: &mut Buffer,
) -> u16 {
    // Every row carries the command that brings it back. Nothing without one
    // can be in a plan at all — `Plan::<Draft>::add` refuses the tiers that
    // have none — so a blank here is a bug rather than a row to draw.
    let commands: Vec<String> = items.iter().map(|c| describe(&c.safety)).collect();
    let longest = widest(items.iter().map(|c| c.path.as_path()));
    let (path_w, command_x, command_w) = columns(left, width, 14, longest, &commands);
    for (c, command) in items.iter().zip(&commands) {
        if y >= bottom {
            break;
        }
        put(
            buf,
            left,
            y,
            &[
                (c.safety.symbol().to_string(), theme.violet),
                (format!(" {:>10}", human(c.bytes)), theme.size(c.bytes)),
            ],
        );
        buf.set_string(
            left + 14,
            y,
            elide_path(&c.path.display().to_string(), path_w),
            theme.text,
        );
        buf.set_string(command_x, y, elide_tail(command, command_w), theme.safe);
        y += 1;
    }
    y
}

/// Draw `parts` one after the other from `x`, each in its own style, and return
/// the column after the last.
///
/// A line made of several roles is several runs; this is how a row says a size
/// in the size colour and the path beside it in text without either knowing
/// where the other ends.
pub(super) fn put<S: AsRef<str>>(buf: &mut Buffer, x: u16, y: u16, parts: &[(S, Style)]) -> u16 {
    let mut x = x;
    for (text, style) in parts {
        let text = text.as_ref();
        buf.set_string(x, y, text, *style);
        x = x.saturating_add(text.chars().count() as u16);
    }
    x
}

/// `parts` cut to `width` columns the way [`super::projects::truncate`] cuts a
/// string: what fits is kept, and a `…` says the rest was dropped.
pub(super) fn clip(parts: Vec<(String, Style)>, width: usize) -> Vec<(String, Style)> {
    let total: usize = parts.iter().map(|(t, _)| t.chars().count()).sum();
    if total <= width {
        return parts;
    }
    let mut room = width.saturating_sub(1);
    let mut kept = Vec::new();
    let mut last = Style::new();
    for (text, style) in parts {
        let take = text.chars().count().min(room);
        room -= take;
        last = style;
        kept.push((text.chars().take(take).collect(), style));
        if room == 0 {
            break;
        }
    }
    kept.push(("…".to_string(), last));
    kept
}

/// A heading with a rule after it, out to `width` columns from `x`, so the
/// sections of a screen separate where the eye is already moving. Returns the
/// row after it.
pub(super) fn section(
    buf: &mut Buffer,
    theme: &Theme,
    x: u16,
    y: u16,
    width: usize,
    title: &str,
) -> u16 {
    heading(buf, theme, x, y, width, &[(title, theme.head)])
}

/// [`section`], for a heading made of several runs: a title and the size it
/// totals, each in its own role.
pub(super) fn heading<S: AsRef<str>>(
    buf: &mut Buffer,
    theme: &Theme,
    x: u16,
    y: u16,
    width: usize,
    parts: &[(S, Style)],
) -> u16 {
    let end = put(buf, x, y, parts);
    let used = (end - x) as usize;
    if width > used + 1 {
        buf.set_string(
            end + 1,
            y,
            theme.rule().to_string().repeat(width - used - 1),
            theme.violet,
        );
    }
    y + 1
}

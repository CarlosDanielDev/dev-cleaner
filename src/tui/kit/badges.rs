//! A concept, its glyph and its role, in one place.
//!
//! No glyph carries a meaning alone: each is drawn beside a word somewhere on
//! the screen, so a font without it costs decoration only.

use super::super::icons::Icon;
use super::super::palette::Theme;
use crate::classify::artifact_for;
use crate::safety::Safety;
use ratatui::style::Style;
use std::path::Path;

/// The kind of directory a path is, with the icon of its toolchain: `⬢ node_modules`.
/// What is not a registered kind has the plain entry icon and its own name.
pub(in crate::tui) fn kind_badge(theme: &Theme, path: &Path) -> (String, Style) {
    let name = path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    let icon = artifact_for(&name).map_or(Icon::Entries, |k| Icon::of(k.ecosystem));
    (format!("{} {name}", theme.icon(icon)), theme.violet)
}

/// How a tier is told apart without colour: its glyph.
pub(in crate::tui) fn tier_badge(theme: &Theme, safety: &Safety) -> (char, Style) {
    (safety.symbol(), theme.violet)
}

/// What a held-back entry leads with.
pub(in crate::tui) fn held_badge(theme: &Theme) -> (char, Style) {
    (theme.icon(Icon::HeldBack), theme.blocked)
}

/// A mark box: the glyph says whether it is marked and the colour only second.
pub(in crate::tui) fn checkbox(theme: &Theme, marked: bool) -> (&'static str, Style) {
    if marked {
        ("[x]", theme.head)
    } else {
        ("[ ]", theme.text)
    }
}

/// The glyph for what a project has marked of what it offers, with the ink it
/// is drawn in. Nothing offerable is blank: there is nothing there to be marked
/// or not.
pub(in crate::tui) fn mark_glyph(
    theme: &Theme,
    offered: usize,
    marked: usize,
) -> (&'static str, Style) {
    match (offered, marked) {
        (0, _) => (" ", theme.text),
        (_, 0) => ("·", theme.text),
        (all, some) if some == all => ("●", theme.safe),
        _ => ("◐", theme.head),
    }
}

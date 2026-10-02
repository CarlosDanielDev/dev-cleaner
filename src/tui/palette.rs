//! Every style the interface draws with, named for what it means.
//!
//! The background is the user's terminal profile, not ours, so nothing here
//! picks a shade against it. Colours are the named ANSI ones, which the profile
//! itself defines — a light theme and a dark theme each map them to something
//! that reads on their own background. Bright black is left out: it is the one
//! ANSI colour many profiles paint within a shade of the background. Hierarchy
//! comes from weight instead, dim and bold over the profile's own foreground.
//!
//! No colour is the only carrier of a meaning. Every style below is paired, on
//! the screen that uses it, with a glyph, a word or a weight that says the same.
//!
//! ponytail: fixed to the ANSI set. A user theme file is its own feature.

use ratatui::style::{Color, Modifier, Style};

/// Ordinary text: the profile's own foreground.
pub const DEFAULT: Style = Style::new();

/// Headings and the sorted column.
pub const HEAD: Style = Style::new().add_modifier(Modifier::BOLD);

/// Labels and hints that can be skipped. Never a fact.
///
/// Dim over the profile's foreground rather than a grey of our choosing, so it
/// stays the profile's contrast, halved, on any background.
pub const MUTED: Style = Style::new().add_modifier(Modifier::DIM);

/// Gauges that only measure.
pub const ACCENT: Style = Style::new().fg(Color::Cyan);

/// How a path comes back: the promise that makes it safe to remove.
pub const SAFE: Style = Style::new().fg(Color::Green);

/// Held back: a guard refused it, or a hold stopped short.
pub const BLOCKED: Style = Style::new().fg(Color::Yellow);

/// The gauge on the one screen that removes anything.
///
/// A shape, never text: ANSI red is dark enough in common dark profiles that
/// words drawn in it stop reading, which is the failure this palette exists
/// to avoid.
pub const DANGER: Style = Style::new().fg(Color::Red);

/// The title band of the one screen that removes anything.
///
/// Inverted in the profile's own colours, so it keeps the profile's contrast
/// and reads the same on a terminal with no colour at all.
pub const WARNING_BAND: Style = Style::new().add_modifier(Modifier::BOLD.union(Modifier::REVERSED));

/// The row under the cursor.
pub const SELECTED: Style = Style::new().add_modifier(Modifier::REVERSED);

/// The verdict of a run that moved everything: bold, in the colour that means
/// the way back is there. The word and the glyph beside it say the same.
pub const VERDICT_SAFE: Style = SAFE.add_modifier(Modifier::BOLD);

/// The verdict of a run that failed or stopped: bold, in the colour of a hold.
pub const VERDICT_BLOCKED: Style = BLOCKED.add_modifier(Modifier::BOLD);

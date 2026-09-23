//! Laying out a row of facts about a path.
//!
//! Shared by the candidates screen and the review screen, which draw the same
//! shape: a size, a path that will not fit, and a sentence about how the path
//! comes back. One definition, because a second copy is how the two screens
//! come to disagree about what a row says.

use crate::safety::Safety;

/// Width for the path column, and where the right-hand column starts.
///
/// The right column is sized to its own longest entry so the fact it carries
/// arrives whole; the path takes the remainder, since a path can be shortened
/// and still identify its entry while a half-sentence cannot.
pub(super) fn columns(left: u16, width: usize, prefix: usize, entries: &[String]) -> (usize, u16) {
    let longest = entries.iter().map(|e| e.chars().count()).max().unwrap_or(0);
    let desc_w = longest.clamp(0, width * 3 / 5);
    let path_w = width.saturating_sub(prefix + desc_w + 1);
    (path_w, left + (prefix + path_w + 1) as u16)
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

/// What this tier means, in the user's words.
pub(super) fn describe(safety: &Safety) -> String {
    match safety {
        Safety::Cache { refills_on } => format!("refills on {refills_on}"),
        Safety::Regenerable { regen } => regen.to_string(),
        Safety::Unproven { reason } => reason.clone(),
        Safety::Protected { reason } => reason.explain().to_string(),
    }
}

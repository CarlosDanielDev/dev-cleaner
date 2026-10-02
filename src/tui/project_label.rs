//! What a project is called on screen, so that no two rows read alike.

use std::collections::{BTreeMap, HashMap};
use std::path::{Component, Path, PathBuf};

use crate::classify::{Checkout, Kind};

/// The glyph for a kind of checkout; the words beside it say what it means.
pub(super) fn glyph(kind: Kind) -> &'static str {
    match kind {
        Kind::Main => "◆",
        Kind::Worktree => "⎇",
        Kind::Orphan => "⌀",
        Kind::Plain => " ",
    }
}

/// The shortest path suffix that no other project shares.
///
/// Directory names repeat: git worktrees multiply `app/ios` once per worktree,
/// and the directory above is the same in every one. So the suffix grows one
/// segment at a time, per project, until it stops matching any other project's
/// suffix of the same length. Paths are unique, so the full path is the worst
/// case and always terminates.
pub(super) fn unique_suffixes<'a>(
    paths: impl Iterator<Item = &'a Path> + Clone,
) -> BTreeMap<PathBuf, String> {
    let segments = |p: &Path| -> Vec<String> {
        p.components()
            .filter_map(|c| match c {
                Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
                _ => None,
            })
            .collect()
    };
    let all: Vec<(&Path, Vec<String>)> = paths.map(|p| (p, segments(p))).collect();
    let deepest = all.iter().map(|(_, s)| s.len()).max().unwrap_or(0);

    // How many projects end in each suffix, by suffix length.
    let mut seen: Vec<HashMap<String, usize>> = vec![HashMap::new(); deepest + 1];
    for (_, segs) in &all {
        for (k, counts) in seen.iter_mut().enumerate().skip(1).take(segs.len()) {
            *counts.entry(tail(segs, k)).or_default() += 1;
        }
    }

    all.iter()
        .map(|(path, segs)| {
            let label = (1..=segs.len())
                .map(|k| tail(segs, k))
                .find(|s| seen[s.matches('/').count() + 1].get(s) == Some(&1))
                .unwrap_or_else(|| path.display().to_string());
            (path.to_path_buf(), label)
        })
        .collect()
}

fn tail(segs: &[String], k: usize) -> String {
    segs[segs.len() - k..].join("/")
}

/// What a checkout adds to its name, where it adds anything.
///
/// A main checkout says so only where worktrees hang off it: alone, it is just
/// a repository, and the word would be noise on most rows.
pub(super) fn annotation(checkout: &Checkout) -> Option<String> {
    let kind = checkout.kind;
    match kind {
        Kind::Worktree => {
            let what = checkout.branch.as_ref().or(checkout.worktree.as_ref())?;
            Some(format!("{} {what}", glyph(kind)))
        }
        Kind::Orphan => Some(format!("{} orphan", glyph(kind))),
        Kind::Main if checkout.linked > 0 => Some(format!("{} main", glyph(kind))),
        _ => None,
    }
}

/// Fit `text` in `width` by cutting out its middle, so that both ends survive:
/// the first segment is what tells projects apart and the last is what they are.
pub(super) fn squeeze(text: &str, width: usize) -> String {
    let count = text.chars().count();
    if count <= width {
        return text.to_string();
    }
    let parts: Vec<&str> = text.split('/').collect();
    if let [first, .., last] = parts.as_slice()
        && parts.len() >= 3
    {
        let short = format!("{first}/…/{last}");
        if short.chars().count() <= width {
            return short;
        }
    }
    if width <= 1 {
        return "…".repeat(width);
    }
    let tail = (width - 1) / 2;
    let head = width - 1 - tail;
    let start: String = text.chars().take(head).collect();
    let end: String = text.chars().skip(count - tail).collect();
    format!("{start}…{end}")
}

/// The least room a note is worth drawing in: a branch cut shorter than this
/// says nothing, and the selected row's detail line says it whole.
const MIN_NOTE: usize = 6;

/// The label and its note, together no wider than `width`.
///
/// The label is what identifies the project, so it is drawn first and whole
/// where it fits; the note takes what is left and goes where that is too little.
pub(super) fn fit_name(label: &str, note: Option<&str>, width: usize) -> (String, String) {
    let used = label.chars().count();
    let left = width.saturating_sub(used + 2);
    match note {
        Some(note) if used <= width && left >= MIN_NOTE => {
            (label.to_string(), format!("  {}", squeeze(note, left)))
        }
        _ => (squeeze(label, width), String::new()),
    }
}

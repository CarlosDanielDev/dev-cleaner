//! What the purge actually did.
//!
//! The screen reports the same facts the written record does, from the same
//! sentences, and keeps a prediction and a measurement in separate rows. The
//! numbers all come from the manifest; nothing here recomputes one, because a
//! second arithmetic is a second answer. The runs before this one come from the
//! store's `purge` table, and never from the manifests on disk.
//!
//! Opens on the verdict, glyph and word first and colour second, with the bar
//! that says how much of the plan moved. Everything under it scrolls; the
//! verdict does not.
//!
//! Named `Report` rather than `Result`, which is taken.

use std::cell::Cell;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;

use super::confirm::{EMPTY, FILLED};
use super::keymap::Motion;
use super::palette::{ACCENT, BLOCKED, DEFAULT, HEAD, VERDICT_BLOCKED, VERDICT_SAFE};
use super::showing;

use crate::bytes::human;
use crate::purge::{
    Manifest, Outcome, not_attempted_note, restore_steps, shortfall_note, took, trash_note,
};
use crate::store::RunSummary;

/// The result screen.
#[derive(Debug, Default)]
pub struct Report {
    /// First row of the scrolling part that is on screen.
    top: usize,
    /// The runs before this one, as the store read them, or why it could not.
    /// `None` until something has looked: the section is left out, not empty.
    history: Option<Result<RunSummary, String>>,
    /// Rows of the scrolling part and rows of room for them, as the last frame
    /// laid them out. A key has to move by what was drawn, and the layout
    /// depends on a width this type is not told until it draws.
    shape: Cell<(usize, usize)>,
}

impl Report {
    pub fn new() -> Self {
        Self::default()
    }

    /// Say what the store holds, or why it could not be read.
    pub fn set_history(&mut self, history: Result<RunSummary, String>) {
        self.history = Some(history);
    }

    /// Scroll the part of the screen under the verdict.
    ///
    /// Moves by the last frame's rows, so a key that arrives before anything
    /// has been drawn finds nothing to move.
    pub fn scroll(&mut self, motion: Motion) {
        let (len, window) = self.shape.get();
        let last = len.saturating_sub(window);
        // Clamped first, so a window that grew since the last scroll does not
        // leave `Up` spending presses on rows that are already in view.
        self.top = self.top.min(last);
        self.top = match motion {
            Motion::Up => self.top.saturating_sub(1),
            Motion::Down => (self.top + 1).min(last),
            Motion::Top => 0,
            Motion::Bottom => last,
            Motion::PageUp => self.top.saturating_sub(window.max(1)),
            Motion::PageDown => (self.top + window.max(1)).min(last),
        };
    }

    /// Draw the run into `area`.
    ///
    /// `record` is where the manifest was written, or `None` when writing it
    /// failed. A path is only shown when there is a file at the end of it.
    pub fn render(&self, manifest: &Manifest, record: Option<&Path>, area: Rect, buf: &mut Buffer) {
        let width = area.width.saturating_sub(2) as usize;
        let (verdict, body) = self.layout(manifest, record, width);
        let left = area.x + 1;
        let height = area.height as usize;
        let mut draw = |row: &[Segment], y: usize| {
            if y < height {
                for (x, text, style) in row {
                    buf.set_string(left + x, area.y + y as u16, text, *style);
                }
            }
        };

        for (y, row) in verdict.iter().enumerate() {
            draw(row, y);
        }
        // The row between the two: blank when everything fits, and the place
        // where a list that does not fit says which part of it is showing.
        let gap = verdict.len();
        let window = height.saturating_sub(gap + 1);
        self.shape.set((body.len(), window.min(body.len())));
        let start = if gap + 1 + body.len() <= height {
            0
        } else {
            let start = self.top.min(body.len().saturating_sub(window));
            let shown = window.min(body.len() - start);
            draw(&[(0, showing(start, shown, body.len()), DEFAULT)], gap);
            start
        };
        for (i, row) in body.iter().skip(start).enumerate() {
            draw(row, gap + 1 + i);
        }
    }

    /// The verdict, which stays put, and everything under it, which scrolls.
    fn layout(&self, manifest: &Manifest, record: Option<&Path>, width: usize) -> (Rows, Rows) {
        let mut verdict = Page::new(width);
        let moved = manifest.removed().count();
        let planned = manifest.planned;
        let failed = manifest.failed().count();
        let skipped = manifest.skipped().count();

        // Glyph and word before colour: the style says the same thing a second
        // time, to the people who can see it.
        let (glyph, word, style) = if manifest.is_complete() {
            ('✓', "SAFE", VERDICT_SAFE)
        } else {
            ('!', "BLOCKED", VERDICT_BLOCKED)
        };
        let mut parts = vec![
            format!("Purged {moved} of {planned} items"),
            human(manifest.bytes_moved()),
            format!("in {}", took(manifest.elapsed)),
        ];
        // A stop is counted, not blamed: a failure is never added to it.
        if failed > 0 {
            parts.push(format!("{failed} failed"));
        }
        if skipped > 0 {
            parts.push(format!("{skipped} not attempted"));
        }
        verdict.wrapped(0, &format!("{glyph} {word} {}", parts.join(" · ")), style);

        // Items, never bytes: the bytes are a prediction until the Trash is
        // emptied, and the rows below keep saying so.
        let percent = (moved * 100).checked_div(planned).unwrap_or(0);
        let label = format!("{percent}% of the plan");
        let bar = width.saturating_sub(label.chars().count() + 2).min(40);
        let filled = (bar * moved).checked_div(planned).unwrap_or(0);
        let mut row = Vec::new();
        if bar > 0 {
            let gauge: String = std::iter::repeat_n(FILLED, filled)
                .chain(std::iter::repeat_n(EMPTY, bar - filled))
                .collect();
            row.push((0, gauge, ACCENT));
        }
        row.push((if bar > 0 { bar as u16 + 2 } else { 0 }, label, DEFAULT));
        verdict.rows.push(row);

        let mut page = Page::new(width);
        page.line("Space", HEAD);
        // Planned and moved are both stated, always. They differ whenever
        // anything failed, and a screen showing one number has to pick which —
        // picking the plan is how a prediction becomes a claim about the disk.
        page.field("Planned", &human(manifest.bytes_expected), DEFAULT);
        page.field("Moved", &human(manifest.bytes_moved()), DEFAULT);

        if manifest.freed_immediately {
            match manifest.bytes_actual {
                Some(actual) => {
                    page.field("Reclaimed on disk", &human(actual), DEFAULT);
                    if let Some(gap) = manifest.shortfall() {
                        page.wrapped(INDENT, &shortfall_note(gap), BLOCKED);
                    }
                }
                None => page.field("Reclaimed on disk", "not measured", DEFAULT),
            }
        } else {
            // `shortfall` answers `None` here by construction, so this branch
            // cannot describe a normal trashed run as a deficit.
            page.field(
                "Waiting in the Trash",
                &human(manifest.pending_in_trash()),
                DEFAULT,
            );
            page.wrapped(INDENT, trash_note(), DEFAULT);
        }

        match &self.history {
            None => {}
            Some(Ok(summary)) => {
                page.gap();
                page.line("All runs", HEAD);
                for line in all_runs(summary) {
                    page.wrapped(INDENT, &line, DEFAULT);
                }
            }
            // Stated, and in the words that name what is missing: a section
            // that vanished would read as there being no history at all.
            Some(Err(why)) => {
                page.gap();
                page.line("All runs", HEAD);
                page.wrapped(
                    INDENT,
                    &format!("The history could not be read: {why}"),
                    BLOCKED,
                );
            }
        }

        // One line per item, carrying that item's own error. A count tells the
        // user something went wrong and not which path to go and look at.
        if failed > 0 {
            page.gap();
            page.line("Not moved", HEAD);
            for item in manifest.failed() {
                let Outcome::Failed { error } = &item.result else {
                    unreachable!("filtered to failed")
                };
                // The path on its own line, its reason under it. Running the
                // three together wraps one item's error into the next item's
                // path, and the list stops being readable as a list.
                page.wrapped(INDENT, &item.path.display().to_string(), DEFAULT);
                page.wrapped(
                    INDENT + 2,
                    &format!("{}  {error}", human(item.bytes)),
                    BLOCKED,
                );
            }
            page.wrapped(INDENT, "These are untouched and still on disk.", DEFAULT);
        }

        // Not a failure, so nothing here is drawn in `BLOCKED`: the run was
        // stopped, and these were left exactly as they were.
        if skipped > 0 {
            page.gap();
            page.line("Not attempted", HEAD);
            for item in manifest.skipped() {
                page.wrapped(INDENT, &item.path.display().to_string(), DEFAULT);
                page.wrapped(INDENT + 2, &human(item.bytes), DEFAULT);
            }
            page.wrapped(INDENT, not_attempted_note(), DEFAULT);
        }

        page.gap();
        page.line("Record", HEAD);
        match record {
            // Wrapped, never elided: a path with its middle replaced by a mark
            // reads as a path and cannot be opened, copied or pasted.
            Some(path) => page.wrapped(INDENT, &path.display().to_string(), DEFAULT),
            None => page.wrapped(
                INDENT,
                "The record could not be written, so what follows is the only account of \
                 this run.",
                BLOCKED,
            ),
        }

        page.gap();
        page.line("Restore", HEAD);
        for step in restore_steps(manifest.freed_immediately) {
            page.wrapped(INDENT, step, DEFAULT);
        }
        (verdict.rows, page.rows)
    }
}

/// What the store says of every run so far, one sentence to a line.
///
/// ponytail: the date is UTC. A local date needs the time zone database, and
/// the day a run happened is context here, not a deadline.
fn all_runs(summary: &RunSummary) -> Vec<String> {
    let s = if summary.runs == 1 { "" } else { "s" };
    let mut lines = vec![format!(
        "{} run{s} since {} · {} moved to the Trash",
        summary.runs,
        date(summary.since),
        human(summary.bytes_moved)
    )];
    // One run has nothing to be fastest or largest of.
    if summary.runs == 1 && summary.this_rank == Some(1) {
        lines.push("this is the first run on record".to_string());
        return lines;
    }
    let mut parts = Vec::new();
    if let Some(fastest) = summary.fastest {
        parts.push(format!("fastest {}", took(fastest)));
    }
    parts.push(format!("largest {}", human(summary.largest)));
    match summary.this_rank {
        Some(1) => parts.push("this run is the largest".to_string()),
        Some(n) => parts.push(format!("this run is the {} largest", ordinal(n))),
        None => {}
    }
    lines.push(parts.join(" · "));
    lines
}

fn ordinal(n: usize) -> String {
    let suffix = match (n % 100, n % 10) {
        (11..=13, _) => "th",
        (_, 1) => "st",
        (_, 2) => "nd",
        (_, 3) => "rd",
        _ => "th",
    };
    format!("{n}{suffix}")
}

/// `YYYY-MM-DD`, in UTC.
fn date(t: SystemTime) -> String {
    let days = t
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() / 86_400) as i64;
    // Days since the epoch to a civil date, by Hinnant's algorithm.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

/// A piece of a row: where it starts, what it says, how it is drawn.
type Segment = (u16, String, Style);

/// Rows laid out, ready to be drawn from whichever one is on top.
type Rows = Vec<Vec<Segment>>;

/// Lines laid out to a width, before anything is drawn.
///
/// Laying out first is what lets a screen taller than the terminal know how
/// tall it is, which is what a scroll position is measured against.
struct Page {
    rows: Rows,
    width: usize,
}

/// Indent for everything under a heading.
const INDENT: u16 = 2;

/// Width of the label column in a `field` row.
const LABEL: usize = 22;

impl Page {
    fn new(width: usize) -> Self {
        Self {
            rows: Vec::new(),
            width,
        }
    }

    fn line(&mut self, text: &str, style: Style) {
        self.rows.push(vec![(0, text.to_string(), style)]);
    }

    fn gap(&mut self) {
        self.rows.push(Vec::new());
    }

    /// A label and its value, in two columns under the heading above.
    ///
    /// Not wrapped: these are the numbers the screen exists to keep apart, and
    /// they only read as a pair when they line up. A terminal too narrow for
    /// the pair puts the value on the next line rather than over the label.
    fn field(&mut self, label: &str, value: &str, style: Style) {
        let column = INDENT as usize + LABEL;
        if column + value.chars().count() <= self.width {
            self.rows.push(vec![
                (INDENT, label.to_string(), style),
                (column as u16, value.to_string(), style),
            ]);
        } else {
            self.wrapped(INDENT, label, style);
            self.wrapped(INDENT + 2, value, style);
        }
    }

    /// A sentence, or a path, broken to fit rather than shortened to fit.
    fn wrapped(&mut self, indent: u16, text: &str, style: Style) {
        let room = self.width.saturating_sub(indent as usize);
        for line in wrap(text, room) {
            self.rows.push(vec![(indent, line, style)]);
        }
    }
}

/// Break `text` into lines of at most `width` characters.
///
/// A word longer than the line is split across lines rather than cut short.
/// Everything on this screen that cannot be shortened is a path, and a path
/// that has been shortened is no longer a path.
pub(super) fn wrap(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return Vec::new();
    }
    let mut lines: Vec<String> = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        let mut word = word;
        while word.chars().count() > width {
            if !line.is_empty() {
                lines.push(std::mem::take(&mut line));
            }
            let cut = byte_at(word, width);
            lines.push(word[..cut].to_string());
            word = &word[cut..];
        }
        if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > width {
            lines.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

/// The byte offset `chars` characters in, or the end.
fn byte_at(s: &str, chars: usize) -> usize {
    s.char_indices().nth(chars).map_or(s.len(), |(i, _)| i)
}

//! What the purge actually did.
//!
//! The screen reports the same facts the written record does, from the same
//! sentences, and keeps a prediction and a measurement in separate rows. The
//! numbers all come from the manifest; nothing here recomputes one, because a
//! second arithmetic is a second answer.
//!
//! Named `Report` rather than `Result`, which is taken.

use std::path::Path;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};

use crate::bytes::human;
use crate::purge::{Manifest, Outcome, restore_steps, shortfall_note, trash_note};

/// The result screen.
///
/// ponytail: holds nothing, so there is nothing to scroll. A run that fails on
/// more items than fit the terminal loses the tail of the list on screen — the
/// written record still has all of them. Give it a `top` like [`super::Review`]
/// if that stops being rare.
#[derive(Debug, Default)]
pub struct Report;

impl Report {
    pub fn new() -> Self {
        Self
    }

    /// Draw the run into `area`.
    ///
    /// `record` is where the manifest was written, or `None` when writing it
    /// failed. A path is only shown when there is a file at the end of it.
    pub fn render(&self, manifest: &Manifest, record: Option<&Path>, area: Rect, buf: &mut Buffer) {
        let head = Style::new().add_modifier(Modifier::BOLD);
        let dim = Style::new().fg(Color::DarkGray);
        let plain = Style::new();
        let mut page = Page::new(area, buf);

        let moved = manifest.removed().count();
        let failed = manifest.failed().count();
        page.line(
            &if manifest.is_complete() {
                format!("Purged  ({moved} items, {})", human(manifest.bytes_moved()))
            } else {
                format!("Purged  ({moved} moved, {failed} failed)")
            },
            head,
        );

        page.gap();
        page.line("Space", head);
        // Planned and moved are both stated, always. They differ whenever
        // anything failed, and a screen showing one number has to pick which —
        // picking the plan is how a prediction becomes a claim about the disk.
        page.field("Planned", &human(manifest.bytes_expected), plain);
        page.field("Moved", &human(manifest.bytes_moved()), plain);

        if manifest.freed_immediately {
            match manifest.bytes_actual {
                Some(actual) => {
                    page.field("Reclaimed on disk", &human(actual), plain);
                    if let Some(gap) = manifest.shortfall() {
                        page.wrapped(INDENT, &shortfall_note(gap), dim);
                    }
                }
                None => page.field("Reclaimed on disk", "not measured", plain),
            }
        } else {
            // `shortfall` answers `None` here by construction, so this branch
            // cannot describe a normal trashed run as a deficit.
            page.field(
                "Waiting in the Trash",
                &human(manifest.pending_in_trash()),
                plain,
            );
            page.wrapped(INDENT, trash_note(), dim);
        }

        // One line per item, carrying that item's own error. A count tells the
        // user something went wrong and not which path to go and look at.
        if failed > 0 {
            page.gap();
            page.line("Not moved", head);
            for item in manifest.failed() {
                let Outcome::Failed { error } = &item.result else {
                    unreachable!("filtered to failed")
                };
                // The path on its own line, its reason under it. Running the
                // three together wraps one item's error into the next item's
                // path, and the list stops being readable as a list.
                page.wrapped(INDENT, &item.path.display().to_string(), plain);
                page.wrapped(INDENT + 2, &format!("{}  {error}", human(item.bytes)), dim);
            }
            page.wrapped(INDENT, "These are untouched and still on disk.", dim);
        }

        page.gap();
        page.line("Record", head);
        match record {
            // Wrapped, never elided: a path with its middle replaced by a mark
            // reads as a path and cannot be opened, copied or pasted.
            Some(path) => page.wrapped(INDENT, &path.display().to_string(), plain),
            None => page.wrapped(
                INDENT,
                "The record could not be written, so what follows is the only account of \
                 this run.",
                dim,
            ),
        }

        page.gap();
        page.line("Restore", head);
        for step in restore_steps(manifest.freed_immediately) {
            page.wrapped(INDENT, step, plain);
        }
    }
}

/// A cursor down the area, which stops at the bottom instead of drawing past it.
struct Page<'a> {
    buf: &'a mut Buffer,
    left: u16,
    y: u16,
    bottom: u16,
    width: usize,
}

/// Indent for everything under a heading.
const INDENT: u16 = 2;

/// Width of the label column in a `field` row.
const LABEL: usize = 22;

impl<'a> Page<'a> {
    fn new(area: Rect, buf: &'a mut Buffer) -> Self {
        Self {
            left: area.x + 1,
            y: area.y,
            bottom: area.bottom(),
            width: area.width.saturating_sub(2) as usize,
            buf,
        }
    }

    fn line(&mut self, text: &str, style: Style) {
        if self.y < self.bottom {
            self.buf.set_string(self.left, self.y, text, style);
        }
        self.y = self.y.saturating_add(1);
    }

    fn gap(&mut self) {
        self.y = self.y.saturating_add(1);
    }

    /// A label and its value, in two columns under the heading above.
    ///
    /// Not wrapped: these are the numbers the screen exists to keep apart, and
    /// they only read as a pair when they line up. A terminal too narrow for
    /// the pair puts the value on the next line rather than over the label.
    fn field(&mut self, label: &str, value: &str, style: Style) {
        let column = INDENT as usize + LABEL;
        if column + value.chars().count() <= self.width {
            if self.y < self.bottom {
                self.buf
                    .set_string(self.left + INDENT, self.y, label, style);
                self.buf
                    .set_string(self.left + column as u16, self.y, value, style);
            }
            self.y = self.y.saturating_add(1);
        } else {
            self.wrapped(INDENT, label, style);
            self.wrapped(INDENT + 2, value, style);
        }
    }

    /// A sentence, or a path, broken to fit rather than shortened to fit.
    fn wrapped(&mut self, indent: u16, text: &str, style: Style) {
        let room = self.width.saturating_sub(indent as usize);
        self.at(indent, &wrap(text, room), style);
    }

    fn at(&mut self, indent: u16, lines: &[String], style: Style) {
        for line in lines {
            if self.y < self.bottom {
                self.buf.set_string(self.left + indent, self.y, line, style);
            }
            self.y = self.y.saturating_add(1);
        }
    }
}

/// Break `text` into lines of at most `width` characters.
///
/// A word longer than the line is split across lines rather than cut short.
/// Everything on this screen that cannot be shortened is a path, and a path
/// that has been shortened is no longer a path.
fn wrap(text: &str, width: usize) -> Vec<String> {
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

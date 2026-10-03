//! What the interface shows while a scan runs behind it.
//!
//! The scan is on a thread of its own and this holds nothing of it but the
//! counters it shares: it reads them on the loop's tick, hands them to the
//! [`Gauge`], and draws what comes back. It never waits for the walk, and it
//! never shows a number the walk has not earned: a percentage appears only
//! against an earlier scan or a known total, and is marked as a guess when it
//! is one.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use super::bar;
use super::icons::Icon;
use super::logo::{self, MASTER_HEIGHT, MASTER_WIDTH};
use super::palette::{Ramp, Theme};
use super::row::{put, section};
use crate::bytes::human;
use crate::scan::{Bar, Baseline, Display, Gauge, Phase, Progress, Reading, Unreadable};

/// How long the logo stands in for the progress block on a first, empty look.
pub const LOGO_FOR: Duration = Duration::from_secs(1);

/// The widest the bar is drawn: past it a bar is read cell by cell.
const BAR_COLUMNS: usize = 48;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Stage {
    Running,
    /// Told to stop, and not yet stopped.
    Stopping,
    /// Stopped: what it found is gone, and nothing was written.
    Cancelled {
        after: Duration,
        entries: u64,
    },
}

#[derive(Debug)]
pub(super) struct ScanView {
    pub(super) progress: Arc<Progress>,
    started: Instant,
    /// Whether a result is being replaced, so the old numbers are known false.
    pub(super) again: bool,
    pub(super) stage: Stage,
    gauge: Gauge,
    elapsed: Duration,
    reading: Reading,
    shown: Display,
    baseline: Option<Arc<Baseline>>,
    current: Option<PathBuf>,
    unreadable: Unreadable,
}

impl ScanView {
    pub(super) fn new(progress: Arc<Progress>, now: Instant, again: bool) -> Self {
        let mut view = Self {
            reading: progress.read(),
            shown: Display {
                phase: Phase::Discovering,
                bar: Bar::Indeterminate,
                past_last: None,
                eta: None,
                rate: 0,
                stalled: None,
                folders: (0, 0),
            },
            progress,
            started: now,
            again,
            stage: Stage::Running,
            gauge: Gauge::new(),
            elapsed: Duration::ZERO,
            baseline: None,
            current: None,
            unreadable: Unreadable::default(),
        };
        view.sample(now);
        view
    }

    pub(super) fn running(&self) -> bool {
        !matches!(self.stage, Stage::Cancelled { .. })
    }

    /// Ask the walk to stop.
    pub(super) fn stop(&mut self) {
        self.progress.cancel();
        self.stage = Stage::Stopping;
    }

    /// The walk has stopped. What it counted is kept for the sentence, not for use.
    pub(super) fn cancelled(&mut self) {
        self.stage = Stage::Cancelled {
            after: self.elapsed,
            entries: self.reading.entries,
        };
    }

    pub(super) fn elapsed(&self) -> Duration {
        self.elapsed
    }

    pub(super) fn unreadable(&self) -> u64 {
        self.unreadable.total()
    }

    /// Read the counters, once, and decide what to say about them.
    ///
    /// Never waits: the folder table is skipped if the walk holds it, and the
    /// last one read is used instead.
    pub(super) fn sample(&mut self, now: Instant) {
        if !self.running() {
            return;
        }
        self.elapsed = now.saturating_duration_since(self.started);
        self.reading = self.progress.read();
        if self.reading.current.is_some() {
            self.current.clone_from(&self.reading.current);
        }
        self.baseline = self.progress.baseline();
        self.unreadable = self.progress.unreadable();
        self.shown = self
            .gauge
            .display(&self.reading, self.baseline.as_deref(), self.elapsed);
    }

    /// The sentence a `q` says it would throw away.
    pub(super) fn dropping(&self) -> String {
        format!("{} entries so far", grouped(self.reading.entries))
    }

    /// The key bar while this is up: the keys that do anything, and no others.
    pub(super) fn keys(&self) -> Vec<(&'static str, &'static str)> {
        match self.stage {
            Stage::Running => vec![("Esc", "cancel the scan"), ("q", "quit")],
            Stage::Stopping => vec![("q", "quit")],
            Stage::Cancelled { .. } => vec![("R", "scan again"), ("q", "quit")],
        }
    }

    /// What the way row says.
    pub(super) fn way(&self) -> &'static str {
        match self.stage {
            Stage::Running => "Esc cancels the scan   ·   the screens open when it finishes",
            Stage::Stopping => "stopping   ·   nothing is written",
            Stage::Cancelled { .. } => "the scan was cancelled   ·   nothing was written",
        }
    }

    /// The fact for the title line.
    pub(super) fn chip(&self) -> String {
        match self.stage {
            Stage::Running => format!("scanning {}", span(self.elapsed)),
            Stage::Stopping => "stopping".to_string(),
            Stage::Cancelled { .. } => "scan cancelled".to_string(),
        }
    }

    pub(super) fn render(&self, theme: &Theme, body: Rect, buf: &mut Buffer) {
        let left = body.x + 2;
        let width = body.width.saturating_sub(4) as usize;
        if let Stage::Cancelled { after, entries } = self.stage {
            let mut y = section(buf, theme, left, body.y, width, "Scan cancelled");
            let said = format!(
                "Scan cancelled after {} · {} entries · nothing was written",
                span(after),
                grouped(entries)
            );
            if y < body.bottom() {
                put(buf, left, y, &[(said, theme.text)]);
                y += 2;
            }
            if y < body.bottom() {
                put(
                    buf,
                    left,
                    y,
                    &[
                        ("R", theme.key),
                        (" scans again   ", theme.muted),
                        ("q", theme.key),
                        (" quits", theme.muted),
                    ],
                );
            }
            return;
        }
        if self.logo_wanted(body) {
            self.render_logo(theme, body, buf);
            return;
        }
        self.render_block(theme, body, buf, left, width);
    }

    /// The master logo, only on a first look at nothing, and only for a moment.
    fn logo_wanted(&self, body: Rect) -> bool {
        !self.again
            && self.elapsed < LOGO_FOR
            && self.reading.projects == 0
            && body.width >= MASTER_WIDTH
            && body.height >= MASTER_HEIGHT + 3
    }

    fn render_logo(&self, theme: &Theme, body: Rect, buf: &mut Buffer) {
        let top = body.y + (body.height - (MASTER_HEIGHT + 2)) / 2;
        let left = body.x + (body.width - MASTER_WIDTH) / 2;
        logo::draw_master(theme, buf, left, top);
        let line = format!(
            "Scanning · {} · {} entries · {}",
            self.shown.phase.name(),
            grouped(self.reading.entries),
            human(self.reading.bytes)
        );
        let line: String = line.chars().take(body.width as usize).collect();
        let at = body.x + body.width.saturating_sub(line.chars().count() as u16) / 2;
        buf.set_string(at, top + MASTER_HEIGHT + 1, line, theme.text);
    }

    fn render_block(&self, theme: &Theme, body: Rect, buf: &mut Buffer, left: u16, width: usize) {
        let shown = &self.shown;
        let mut y = section(
            buf,
            theme,
            left,
            body.y,
            width,
            &format!("Scanning · {}", shown.phase.name()),
        );
        let room = |y: u16| y < body.bottom();

        // The bar, and what it is measured against.
        if room(y) {
            let columns = width.min(BAR_COLUMNS);
            let parts = match shown.bar {
                Bar::Indeterminate => bar::sweep(theme, columns, self.elapsed.as_millis() as u64),
                Bar::Estimated(f) => bar::estimated(theme, Ramp::Measure, f, columns),
                Bar::Exact(f) => {
                    bar::line(theme, Ramp::Measure, (f * 1000.0) as u64, 1000, columns)
                }
            };
            let end = put(buf, left, y, &parts);
            let note = self.bar_note();
            let room_left = (left as usize + width).saturating_sub(end as usize + 2);
            let note: String = note.chars().take(room_left).collect();
            put(buf, end + 2, y, &[(note, theme.muted)]);
            y += 1;
        }

        // How fast, how long, and how much longer, as a range or not at all.
        if room(y) {
            let mut line = format!("{} entries/s · {}", grouped(shown.rate), span(self.elapsed));
            if let Some((low, high)) = shown.eta {
                line.push_str(&format!(" · about {} to {} left", span(low), span(high)));
            }
            put(buf, left, y, &[(line, theme.text)]);
            y += 1;
        }

        // What has been found so far. Always the walk's totals: they stay on
        // screen when the walk is over and the rest of the work begins.
        if room(y) {
            let mut parts = vec![
                (theme.icon(Icon::Entries).to_string(), theme.accent),
                (
                    format!(" {} entries   ", grouped(self.reading.entries)),
                    theme.text,
                ),
                (theme.icon(Icon::Projects).to_string(), theme.accent),
                (
                    format!(
                        " {} {}   ",
                        grouped(self.reading.projects),
                        plural(self.reading.projects, "project")
                    ),
                    theme.text,
                ),
                (theme.icon(Icon::Disk).to_string(), theme.accent),
                (format!(" {}", human(self.reading.bytes)), theme.text),
            ];
            let unreadable = self.unreadable.total();
            if unreadable > 0 {
                parts.push((theme.icon(Icon::HeldBack).to_string(), theme.blocked));
                parts.push((
                    format!(
                        "   {} unreadable {}",
                        grouped(unreadable),
                        plural(unreadable, "folder")
                    ),
                    theme.text,
                ));
            }
            put(buf, left, y, &parts);
            y += 1;
        }

        // Where the walk is, in the folders the roots hold.
        if room(y) && shown.phase == Phase::Discovering {
            let (done, all) = shown.folders;
            let mut line = String::new();
            if let Some(name) = self.current.as_ref().and_then(|p| p.file_name()) {
                line.push_str(&format!("in {}", name.to_string_lossy()));
            }
            if all > 0 {
                if !line.is_empty() {
                    line.push_str(" · ");
                }
                line.push_str(&format!("top-level folders {done} of {all} done"));
            }
            if !line.is_empty() {
                let line: String = line.chars().take(width).collect();
                put(buf, left, y, &[(line, theme.muted)]);
                y += 1;
            }
        }

        if room(y) && self.unreadable.total() > 0 {
            let u = self.unreadable;
            let parts: Vec<String> = [
                (u.permission, "permission denied"),
                (u.vanished, "vanished"),
                (u.looped, "loops"),
                (u.other, "other"),
            ]
            .iter()
            .filter(|(n, _)| *n > 0)
            .map(|(n, why)| format!("{n} {why}"))
            .collect();
            put(
                buf,
                left,
                y,
                &[(format!("unreadable: {}", parts.join(" · ")), theme.muted)],
            );
            y += 1;
        }

        // A counter that has stopped is said out loud, in the ink for a warning.
        if room(y)
            && let Some(quiet) = shown.stalled
        {
            put(
                buf,
                left,
                y,
                &[(
                    format!("no progress for {} ({})", span(quiet), self.waiting_on()),
                    theme.blocked,
                )],
            );
            y += 1;
        }

        y += 1;
        if room(y) && self.again {
            put(
                buf,
                left,
                y,
                &[(
                    "The purge changed the disk, so the old numbers are being measured again.",
                    theme.muted,
                )],
            );
            y += 1;
        }
        if room(y) {
            let hint = match self.stage {
                Stage::Stopping => "Stopping: it ends at the next entry. Nothing is written.",
                _ => "Nothing is written until the scan finishes. Esc cancels it.",
            };
            put(buf, left, y, &[(hint, theme.muted)]);
        }
    }

    /// What the bar is measured against, in words.
    fn bar_note(&self) -> String {
        let shown = &self.shown;
        if let Some(entries) = shown.past_last {
            return format!("past last scan ({} entries)", grouped(entries));
        }
        match shown.bar {
            Bar::Estimated(_) => match &self.baseline {
                Some(base) => format!(
                    "of last scan ({} entries, {})",
                    grouped(base.entries),
                    span(base.wall)
                ),
                None => String::new(),
            },
            Bar::Exact(_) => {
                let (unit, done, total) = (
                    match shown.phase {
                        Phase::Classifying => "artifact folders",
                        Phase::Measuring => "files",
                        _ => "steps",
                    },
                    self.reading.phase_done.min(self.reading.phase_total),
                    self.reading.phase_total,
                );
                format!("{} of {} {unit}", grouped(done), grouped(total))
            }
            Bar::Indeterminate => match shown.phase {
                Phase::Discovering => {
                    "no earlier scan of these roots, so no percentage".to_string()
                }
                Phase::Indexing => "working out which folders are projects".to_string(),
                Phase::Saving => "writing the scan down".to_string(),
                _ => String::new(),
            },
        }
    }

    fn waiting_on(&self) -> &'static str {
        match self.shown.phase {
            Phase::Discovering => "reading a large folder, or waiting on the disk",
            _ => "still working",
        }
    }
}

/// `204107` as `204,107`, so a counter that moves fast stays readable.
pub(super) fn grouped(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn plural(n: u64, noun: &str) -> String {
    if n == 1 {
        noun.to_string()
    } else {
        format!("{noun}s")
    }
}

/// A span of time as a person says it: seconds, then minutes, then hours.
pub(super) fn span(d: Duration) -> String {
    let secs = d.as_secs();
    match secs {
        0..60 => format!("{secs} s"),
        60..3600 => format!("{} min {} s", secs / 60, secs % 60),
        _ => format!("{} h {:02} min", secs / 3600, secs % 3600 / 60),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_are_grouped_in_threes() {
        assert_eq!(grouped(0), "0");
        assert_eq!(grouped(999), "999");
        assert_eq!(grouped(204_107), "204,107");
        assert_eq!(grouped(1_491_788), "1,491,788");
    }

    #[test]
    fn spans_are_said_the_way_people_say_them() {
        assert_eq!(span(Duration::from_secs(38)), "38 s");
        assert_eq!(span(Duration::from_secs(72)), "1 min 12 s");
        assert_eq!(span(Duration::from_secs(3_900)), "1 h 05 min");
    }
}

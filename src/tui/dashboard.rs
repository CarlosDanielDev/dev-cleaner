use super::Screen;
use super::bar::{self, Part};
use super::keymap::{Action, bindings_for};
use super::palette::{Ramp, Theme};
use super::row::{clip, elide_path, put, section};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;

use crate::bytes::human;
use crate::store::{Change, TrendRow};
use crate::volume::Volume;

/// One thing taking up room, measured both ways.
#[derive(Debug, Clone)]
pub struct Consumer {
    pub label: String,
    pub bytes: u64,
    /// File count. The largest directory is often not the one with the most
    /// files in it, and it is the file count that makes every backup and every
    /// Spotlight pass slow.
    pub inodes: u64,
}

/// What can be said about change since the last scan.
///
/// A first run has no comparison, which is ordinary rather than missing. Saying
/// so once here keeps it out of every field that would otherwise be optional.
#[derive(Debug, Clone)]
pub enum Trend {
    FirstScan,
    Since(Vec<TrendRow>),
    /// The history could not be read, so nothing can be said about change.
    ///
    /// Deliberately not folded into `FirstScan`. Announcing "recorded as the
    /// first scan of these roots" because the database would not open is a
    /// claim about the disk made out of a failure to reach a file, and the next
    /// run would contradict it.
    Unavailable(String),
}

/// What one step forward would let the user do.
///
/// Counted from the objects the candidates screen and the projects table are
/// built from, never from the scan again, so the opening screen cannot
/// disagree with the screens it points at.
#[derive(Debug, Clone, Default)]
pub struct Now {
    /// Entries every guard cleared.
    pub offerable: usize,
    /// What they add up to, summed as the plan sums them: the figure the
    /// confirm screen would ask the user to approve if every one were marked.
    /// The gauge above measures the union instead and can read lower where
    /// directories hardlink into each other, as the plan already does (#45).
    pub offerable_bytes: u64,
    /// Why entries were held back, in the words the candidates screen uses,
    /// each with how many it held. In no particular order; the screen ranks.
    pub blocked: Vec<(String, usize)>,
    /// Projects the table calls dead.
    pub dead: usize,
    /// The build output measured inside them, per project as the table
    /// measures it. Not the projects themselves: the tool does not offer those.
    pub dead_reclaimable: u64,
}

/// The opening screen: the state of the disk, what moved since last time, and
/// what can be done about it now.
#[derive(Debug, Clone)]
pub struct Dashboard {
    /// `None` when the volume could not be measured. The rest of the scan's
    /// answers are still worth showing.
    pub volume: Option<Volume>,
    pub reclaimable: u64,
    pub trend: Trend,
    pub consumers: Vec<Consumer>,
    pub now: Now,
    /// What each recent scan of these roots found reclaimable, oldest first.
    /// `None` is a scan recorded before the total was kept. Empty when the
    /// history could not be read.
    pub history: Vec<Option<u64>>,
}

/// Sparkline steps, lowest first, and what a scan with no value draws.
const RAMP: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
const NO_VALUE: char = '·';

impl Dashboard {
    pub fn top_by_bytes(&self, n: usize) -> Vec<&Consumer> {
        let mut all: Vec<&Consumer> = self.consumers.iter().collect();
        all.sort_by_key(|c| std::cmp::Reverse(c.bytes));
        all.into_iter().take(n).collect()
    }

    pub fn top_by_inodes(&self, n: usize) -> Vec<&Consumer> {
        let mut all: Vec<&Consumer> = self.consumers.iter().collect();
        all.sort_by_key(|c| std::cmp::Reverse(c.inodes));
        all.into_iter().take(n).collect()
    }

    pub fn render(&self, theme: &Theme, area: Rect, buf: &mut Buffer) {
        let mut y = area.y;
        let left = area.x + 2;
        let width = area.width.saturating_sub(4) as usize;

        y = section(buf, theme, left, y, width, "Disk");
        y = self.render_volume(theme, left, y, width, area.bottom(), buf);
        if y < area.bottom()
            && let Some(parts) = self.sparkline(theme, width)
        {
            put(buf, left, y, &parts);
            y += 1;
        }

        // One blank row between sections, here and everywhere below.
        y += 1;
        if y < area.bottom() {
            y = section(buf, theme, left, y, width, "Top consumers");
            y = self.render_consumers(theme, left, y, width, area.bottom(), buf);
        }

        y += 1;
        if y < area.bottom() {
            y = self.render_now(theme, left, y, width, area, buf);
        }

        // A cell below the body is a panic, not a blank. The sections above
        // are fixed in height; this one is where a short terminal runs out.
        y += 1;
        if y < area.bottom() {
            self.render_trend(theme, left, y, width, area, buf);
        }
    }

    /// The disk gauge and the legend under it. Returns the row after them.
    fn render_volume(
        &self,
        theme: &Theme,
        left: u16,
        mut y: u16,
        width: usize,
        bottom: u16,
        buf: &mut Buffer,
    ) -> u16 {
        let Some(volume) = self.volume else {
            buf.set_string(left, y, "free space unavailable on this path", theme.text);
            return y + 1;
        };

        // Reclaimable is drawn inside the used portion, never beside the free
        // one: it is space the user does not have yet, and showing it as free
        // would overstate the disk by exactly the amount this tool is for.
        const USED_LABEL: usize = 10;
        let cells = bar::stacked_cells(theme, width.saturating_sub(USED_LABEL)).max(10);
        let scale = |bytes: u64| -> usize {
            if bytes == 0 || volume.total == 0 {
                return 0;
            }
            // Anything present gets at least one cell. Reclaimable space is a
            // small fraction of a large disk in exactly the ordinary case — 5 GB
            // of 460 GB rounds to nine tenths of a cell — and a segment that
            // rounds away to nothing reports "none" for the one quantity this
            // screen exists to show.
            ((bytes as u128 * cells as u128 / volume.total as u128) as usize).max(1)
        };
        let reclaimable = scale(self.reclaimable.min(volume.used()));
        let in_use = scale(volume.used()).saturating_sub(reclaimable);
        let free = cells.saturating_sub(reclaimable + in_use);
        let used_percent = (volume.used() as u128 * 100)
            .checked_div(volume.total as u128)
            .unwrap_or(0);

        let mut gauge = bar::stacked(theme, [reclaimable, in_use, free]);
        gauge.push((format!(" {used_percent:>3}% used"), theme.text));
        put(buf, left, y, &gauge);
        y += 1;
        if y >= bottom {
            return y;
        }

        // Each swatch is the glyph and the colour of the cells it names, from
        // the same place the cells are drawn from.
        // The figure is a fact, so it is never muted: free space is drawn in
        // muted cells and its figure in text.
        let entry = |kind: Part, bytes: u64, label: &str| {
            let (glyph, style) = bar::part(theme, kind);
            let figure = if kind == Part::Free {
                theme.text
            } else {
                style
            };
            [
                (format!("{glyph} "), style),
                (human(bytes), figure),
                (format!(" {label}"), theme.muted),
            ]
        };
        let mut legend: Vec<(String, Style)> = Vec::new();
        let gap = || ("  ".to_string(), theme.text);
        legend.extend(entry(Part::Reclaimable, self.reclaimable, "reclaimable"));
        legend.push(gap());
        legend.extend(entry(
            Part::InUse,
            volume.used().saturating_sub(self.reclaimable),
            "in use",
        ));
        legend.push(gap());
        legend.extend(entry(Part::Free, volume.free, "free"));
        legend.push(gap());
        legend.push((format!("of {}", human(volume.total)), theme.text));
        put(buf, left, y, &clip(legend, width));
        y + 1
    }

    /// The reclaimable total over the last scans, newest at the right, or
    /// `None` when fewer than two scans have a value to draw.
    ///
    /// Scaled to the highest value in the window, not to the disk: the line is
    /// about the shape of the change, and against 460 GB every point would be
    /// the bottom step. At most `cells` wide, keeping the newest scans; the
    /// words after the glyphs are dropped, longest first, before the glyphs are.
    fn sparkline(&self, theme: &Theme, cells: usize) -> Option<Vec<(String, Style)>> {
        let window = &self.history[self.history.len().saturating_sub(cells)..];
        let values: Vec<u64> = window.iter().flatten().copied().collect();
        if values.len() < 2 {
            return None;
        }
        let (low, high) = (*values.iter().min()?, *values.iter().max()?);
        let glyphs: Vec<char> = window
            .iter()
            .map(|point| match point {
                None => NO_VALUE,
                Some(_) if high == 0 => RAMP[0],
                // Rounded up, so a value above zero never draws as nothing.
                Some(v) => RAMP[((*v as u128 * 8).div_ceil(high as u128) as usize).clamp(1, 8) - 1],
            })
            .collect();
        let now = window
            .last()
            .copied()
            .flatten()
            .map(|v| format!(" · now {}", human(v)))
            .unwrap_or_default();
        let figures = format!("low {} · high {}{now}", human(low), human(high));
        let label = format!("reclaimable over the last {} scans", window.len());
        // The widest tail that fits, and the figures are dropped before the
        // label is: the range is what the line is for, and the label can be
        // read off the figures.
        let tail = [
            Some((label.clone(), figures.clone())),
            Some((String::new(), figures)),
            None,
        ]
        .into_iter()
        .find(|tail| {
            let extra = tail.as_ref().map_or(0, |(l, f)| {
                2 + l.chars().count() + if l.is_empty() { 0 } else { 3 } + f.chars().count()
            });
            glyphs.len() + extra <= cells
        })
        .flatten();

        // The newest scan is the right-hand cell; the colour runs with the
        // cells, so a rising line also warms.
        let mut parts: Vec<(String, Style)> = glyphs
            .iter()
            .enumerate()
            .map(|(i, g)| {
                let style = if *g == NO_VALUE {
                    theme.muted
                } else {
                    theme.ramp(Ramp::Measure, i, glyphs.len())
                };
                (g.to_string(), style)
            })
            .collect();
        if let Some((label, figures)) = tail {
            parts.push(("  ".to_string(), theme.text));
            if !label.is_empty() {
                parts.push((label, theme.muted));
                parts.push((" · ".to_string(), theme.violet));
            }
            parts.push((figures, theme.text));
        }
        Some(parts)
    }

    fn render_consumers(
        &self,
        theme: &Theme,
        left: u16,
        mut y: u16,
        width: usize,
        bottom: u16,
        buf: &mut Buffer,
    ) -> u16 {
        // Two columns of the same shape, each a right-aligned figure and the
        // name it belongs to, so every figure ends where the one above it does
        // and every name starts where the one above it does.
        const FIGURE: usize = 10;
        let half = width.saturating_sub(2 + 3) / 2;
        let name_w = half.saturating_sub(FIGURE + 2).max(1);
        let (x_bytes, x_inodes) = (left + 2, left + 2 + half as u16 + 3);

        buf.set_string(x_bytes, y, format!("{:>FIGURE$}", "by size"), theme.muted);
        buf.set_string(
            x_inodes,
            y,
            format!("{:>FIGURE$}", "by inodes"),
            theme.muted,
        );
        y += 1;

        let by_bytes = self.top_by_bytes(5);
        let by_inodes = self.top_by_inodes(5);
        for row in 0..by_bytes.len().max(by_inodes.len()) {
            if y >= bottom {
                break;
            }
            if let Some(c) = by_bytes.get(row) {
                put(
                    buf,
                    x_bytes,
                    y,
                    &[
                        (format!("{:>FIGURE$}", human(c.bytes)), theme.size(c.bytes)),
                        (format!("  {}", elide_path(&c.label, name_w)), theme.text),
                    ],
                );
            }
            if let Some(c) = by_inodes.get(row) {
                put(
                    buf,
                    x_inodes,
                    y,
                    &[
                        (format!("{:>FIGURE$}", c.inodes), theme.accent),
                        (format!("  {}", elide_path(&c.label, name_w)), theme.text),
                    ],
                );
            }
            y += 1;
        }
        y
    }

    fn render_now(
        &self,
        theme: &Theme,
        left: u16,
        mut y: u16,
        width: usize,
        area: Rect,
        buf: &mut Buffer,
    ) -> u16 {
        y = section(buf, theme, left, y, width, "Now");
        let room = area.bottom().saturating_sub(y) as usize;
        for line in self.now_lines(theme).iter().take(room) {
            put(buf, left + 2, y, line);
            y += 1;
        }
        y
    }

    /// The Now section, one row per line, each in the colours of what it says.
    /// Every row is a fact, so none is muted.
    fn now_lines(&self, theme: &Theme) -> Vec<Vec<(String, Style)>> {
        let now = &self.now;
        let plain = |text: String| vec![(text, theme.text)];
        let mut lines = Vec::new();

        if now.offerable == 0 {
            lines.push(plain("Nothing can be rebuilt on these roots".to_string()));
        } else {
            let mut line = vec![
                (
                    format!(
                        "{:<32}",
                        format!(
                            "{} can be rebuilt",
                            count(now.offerable, "directory", "directories")
                        )
                    ),
                    theme.safe,
                ),
                ("  ".to_string(), theme.text),
                (
                    format!("{:>10}", human(now.offerable_bytes)),
                    theme.size(now.offerable_bytes),
                ),
                ("     ".to_string(), theme.text),
            ];
            line.extend(way_to_candidates(theme));
            lines.push(line);
        }

        let held: usize = now.blocked.iter().map(|(_, n)| n).sum();
        if held > 0 {
            lines.push(vec![(
                format!("{held} held back by a guard"),
                theme.blocked,
            )]);
            // The three reasons that held the most, then what they leave out,
            // so the rows under the total add up to it.
            let mut reasons: Vec<&(String, usize)> = now.blocked.iter().collect();
            reasons.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
            for (reason, n) in reasons.iter().take(3) {
                lines.push(vec![
                    (format!("{n:>10}"), theme.blocked),
                    (format!("  {reason}"), theme.text),
                ]);
            }
            let rest: usize = reasons.iter().skip(3).map(|(_, n)| n).sum();
            if rest > 0 {
                lines.push(plain(format!("{:>10}  and {rest} more", "")));
            }
        }

        if now.dead > 0 {
            lines.push(vec![
                (
                    format!("{:<32}", count(now.dead, "dead project", "dead projects")),
                    theme.violet,
                ),
                ("  ".to_string(), theme.text),
                (
                    format!("{:>10}", human(now.dead_reclaimable)),
                    theme.size(now.dead_reclaimable),
                ),
                (
                    format!(
                        " of build output inside {}",
                        if now.dead == 1 { "it" } else { "them" }
                    ),
                    theme.text,
                ),
            ]);
        }
        lines
    }

    fn render_trend(
        &self,
        theme: &Theme,
        left: u16,
        mut y: u16,
        width: usize,
        area: Rect,
        buf: &mut Buffer,
    ) {
        match &self.trend {
            Trend::Unavailable(why) => {
                buf.set_string(
                    left,
                    y,
                    format!("History unavailable, so nothing can be compared: {why}"),
                    theme.blocked,
                );
            }
            Trend::FirstScan => {
                buf.set_string(
                    left,
                    y,
                    "Recorded as the first scan of these roots. Run again later to see what changed.",
                    theme.text,
                );
            }
            Trend::Since(rows) => {
                y = section(buf, theme, left, y, width, "Since the previous scan");
                // Most paths in a scan are unchanged. Listing them buries the
                // few that are not, which are the whole reason for the section.
                let moved: Vec<&TrendRow> = rows
                    .iter()
                    .filter(|r| r.change != Change::Unchanged)
                    .collect();
                if moved.is_empty() {
                    buf.set_string(left + 2, y, "nothing changed", theme.text);
                    return;
                }
                let room = area.bottom().saturating_sub(y) as usize;
                for row in moved.iter().take(room) {
                    // Grown is attention, shrunk is calm, and the sign or the
                    // word says so with no colour at all.
                    let style = match row.change {
                        Change::Grew { .. } => theme.blocked,
                        Change::Shrank { .. } => theme.safe,
                        Change::New => theme.accent,
                        Change::Removed => theme.violet,
                        Change::Unchanged => theme.text,
                    };
                    put(
                        buf,
                        left + 2,
                        y,
                        &[
                            (format!("{:>12}", row.change.describe()), style),
                            ("  ".to_string(), theme.text),
                            (row.path.display().to_string(), theme.text),
                        ],
                    );
                    y += 1;
                }
            }
        }
    }
}

/// `n` with the noun agreeing with it.
fn count(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// The presses that lead from the dashboard to the candidates.
///
/// Walked over `Screen::next` and read from the table, so the line cannot
/// name a key that does not work or a screen that is not on the way.
fn way_to_candidates(theme: &Theme) -> Vec<(String, Style)> {
    let mut keys: Vec<String> = Vec::new();
    let mut screen = Screen::Dashboard;
    while screen != Screen::Candidates {
        let Some(next) = screen.next() else {
            break;
        };
        keys.extend(
            bindings_for(screen)
                .iter()
                .find(|b| b.action == Action::Forward)
                .map(|b| b.key.to_string()),
        );
        screen = next;
    }
    let presses = match keys.as_slice() {
        [key, rest @ ..] if rest.iter().all(|k| k == key) => match keys.len() {
            1 => key.clone(),
            2 => format!("{key} twice"),
            n => format!("{key} {n} times"),
        },
        _ => keys.join(", then "),
    };
    vec![
        (presses, theme.key),
        (" → ".to_string(), theme.violet),
        (Screen::Candidates.name().to_string(), theme.accent),
    ]
}

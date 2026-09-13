use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};

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
}

/// The opening screen: the state of the disk, and what moved since last time.
#[derive(Debug, Clone)]
pub struct Dashboard {
    /// `None` when the volume could not be measured. The rest of the scan's
    /// answers are still worth showing.
    pub volume: Option<Volume>,
    pub reclaimable: u64,
    pub trend: Trend,
    pub consumers: Vec<Consumer>,
}

/// Gauge segments. Each is told apart by its symbol, so the bar reads the same
/// to someone who cannot distinguish the colours.
const RECLAIMABLE: char = '█';
const IN_USE: char = '▒';
const FREE: char = '·';

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

    pub fn render(&self, area: Rect, buf: &mut Buffer) {
        let mut y = area.y;
        let left = area.x + 2;
        let dim = Style::new().fg(Color::DarkGray);
        let head = Style::new().add_modifier(Modifier::BOLD);

        buf.set_string(left, y, "Disk", head);
        y += 2;
        y = self.render_volume(left, y, area, buf, dim);

        y += 1;
        buf.set_string(left, y, "Top consumers", head);
        y += 1;
        y = self.render_consumers(left, y, buf, dim);

        y += 1;
        self.render_trend(left, y, area, buf, dim);
    }

    fn render_volume(
        &self,
        left: u16,
        mut y: u16,
        area: Rect,
        buf: &mut Buffer,
        dim: Style,
    ) -> u16 {
        let Some(volume) = self.volume else {
            buf.set_string(left, y, "free space unavailable on this path", dim);
            return y + 1;
        };

        // Reclaimable is drawn inside the used portion, never beside the free
        // one: it is space the user does not have yet, and showing it as free
        // would overstate the disk by exactly the amount this tool is for.
        let width = area.width.saturating_sub(6).max(10) as u64;
        let scale = |bytes: u64| -> usize {
            if bytes == 0 || volume.total == 0 {
                return 0;
            }
            // Anything present gets at least one cell. Reclaimable space is a
            // small fraction of a large disk in exactly the ordinary case — 5 GB
            // of 460 GB rounds to nine tenths of a cell — and a segment that
            // rounds away to nothing reports "none" for the one quantity this
            // screen exists to show.
            ((bytes as u128 * width as u128 / volume.total as u128) as usize).max(1)
        };
        let reclaimable = scale(self.reclaimable.min(volume.used()));
        let in_use = scale(volume.used()).saturating_sub(reclaimable);
        let free = (width as usize).saturating_sub(reclaimable + in_use);

        let bar: String = std::iter::repeat_n(RECLAIMABLE, reclaimable)
            .chain(std::iter::repeat_n(IN_USE, in_use))
            .chain(std::iter::repeat_n(FREE, free))
            .collect();
        buf.set_string(left, y, bar, Style::new().fg(Color::Cyan));
        y += 1;

        buf.set_string(
            left,
            y,
            format!(
                "{RECLAIMABLE} {} reclaimable   {IN_USE} {} in use   {FREE} {} free   of {}",
                human(self.reclaimable),
                human(volume.used().saturating_sub(self.reclaimable)),
                human(volume.free),
                human(volume.total),
            ),
            dim,
        );
        y + 1
    }

    fn render_consumers(&self, left: u16, mut y: u16, buf: &mut Buffer, dim: Style) -> u16 {
        buf.set_string(left + 2, y, "by size", dim);
        buf.set_string(left + 36, y, "by inodes", dim);
        y += 1;

        let by_bytes = self.top_by_bytes(5);
        let by_inodes = self.top_by_inodes(5);
        for row in 0..by_bytes.len().max(by_inodes.len()) {
            if let Some(c) = by_bytes.get(row) {
                buf.set_string(
                    left + 2,
                    y,
                    format!("{:>10}  {}", human(c.bytes), c.label),
                    Style::new(),
                );
            }
            if let Some(c) = by_inodes.get(row) {
                buf.set_string(
                    left + 36,
                    y,
                    format!("{:>10}  {}", c.inodes, c.label),
                    Style::new(),
                );
            }
            y += 1;
        }
        y
    }

    fn render_trend(&self, left: u16, mut y: u16, area: Rect, buf: &mut Buffer, dim: Style) {
        match &self.trend {
            Trend::FirstScan => {
                buf.set_string(
                    left,
                    y,
                    "Recorded as the first scan of these roots. Run again later to see what changed.",
                    dim,
                );
            }
            Trend::Since(rows) => {
                buf.set_string(
                    left,
                    y,
                    "Since the previous scan",
                    Style::new().add_modifier(Modifier::BOLD),
                );
                y += 1;
                // Most paths in a scan are unchanged. Listing them buries the
                // few that are not, which are the whole reason for the section.
                let moved: Vec<&TrendRow> = rows
                    .iter()
                    .filter(|r| r.change != Change::Unchanged)
                    .collect();
                if moved.is_empty() {
                    buf.set_string(left + 2, y, "nothing changed", dim);
                    return;
                }
                let room = area.bottom().saturating_sub(y) as usize;
                for row in moved.iter().take(room) {
                    buf.set_string(
                        left + 2,
                        y,
                        format!(
                            "{:>12}  {:<10}  {}",
                            row.change.describe(),
                            "",
                            row.path.display()
                        ),
                        Style::new(),
                    );
                    y += 1;
                }
            }
        }
    }
}

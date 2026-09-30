use super::Screen;
use super::keymap::{Action, bindings_for};
use super::palette::{ACCENT, DEFAULT, HEAD, MUTED};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

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

        buf.set_string(left, y, "Disk", HEAD);
        y += 2;
        y = self.render_volume(left, y, area, buf);

        y += 1;
        buf.set_string(left, y, "Top consumers", HEAD);
        y += 1;
        y = self.render_consumers(left, y, buf);

        y += 1;
        y = self.render_now(left, y, area, buf);

        // A cell below the body is a panic, not a blank. The sections above
        // are fixed in height; this one is where a short terminal runs out.
        y += 1;
        if y < area.bottom() {
            self.render_trend(left, y, area, buf);
        }
    }

    fn render_volume(&self, left: u16, mut y: u16, area: Rect, buf: &mut Buffer) -> u16 {
        let Some(volume) = self.volume else {
            buf.set_string(left, y, "free space unavailable on this path", DEFAULT);
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
        buf.set_string(left, y, bar, ACCENT);
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
            DEFAULT,
        );
        y + 1
    }

    fn render_consumers(&self, left: u16, mut y: u16, buf: &mut Buffer) -> u16 {
        buf.set_string(left + 2, y, "by size", MUTED);
        buf.set_string(left + 36, y, "by inodes", MUTED);
        y += 1;

        let by_bytes = self.top_by_bytes(5);
        let by_inodes = self.top_by_inodes(5);
        for row in 0..by_bytes.len().max(by_inodes.len()) {
            if let Some(c) = by_bytes.get(row) {
                buf.set_string(
                    left + 2,
                    y,
                    format!("{:>10}  {}", human(c.bytes), c.label),
                    DEFAULT,
                );
            }
            if let Some(c) = by_inodes.get(row) {
                buf.set_string(
                    left + 36,
                    y,
                    format!("{:>10}  {}", c.inodes, c.label),
                    DEFAULT,
                );
            }
            y += 1;
        }
        y
    }

    fn render_now(&self, left: u16, mut y: u16, area: Rect, buf: &mut Buffer) -> u16 {
        buf.set_string(left, y, "Now", HEAD);
        y += 1;
        let room = area.bottom().saturating_sub(y) as usize;
        for line in self.now_lines().iter().take(room) {
            buf.set_string(left + 2, y, line, DEFAULT);
            y += 1;
        }
        y
    }

    /// The Now section, one row per line. Every row is a fact, so none is
    /// muted.
    fn now_lines(&self) -> Vec<String> {
        let now = &self.now;
        let mut lines = Vec::new();

        if now.offerable == 0 {
            lines.push("Nothing can be rebuilt on these roots".to_string());
        } else {
            lines.push(format!(
                "{:<32}  {:>10}     {}",
                format!(
                    "{} can be rebuilt",
                    count(now.offerable, "directory", "directories")
                ),
                human(now.offerable_bytes),
                way_to_candidates(),
            ));
        }

        let held: usize = now.blocked.iter().map(|(_, n)| n).sum();
        if held > 0 {
            lines.push(format!("{held} held back by a guard"));
            // The three reasons that held the most, then what they leave out,
            // so the rows under the total add up to it.
            let mut reasons: Vec<&(String, usize)> = now.blocked.iter().collect();
            reasons.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
            for (reason, n) in reasons.iter().take(3) {
                lines.push(format!("{n:>10}  {reason}"));
            }
            let rest: usize = reasons.iter().skip(3).map(|(_, n)| n).sum();
            if rest > 0 {
                lines.push(format!("{:>10}  and {rest} more", ""));
            }
        }

        if now.dead > 0 {
            lines.push(format!(
                "{:<32}  {:>10} of build output inside {}",
                count(now.dead, "dead project", "dead projects"),
                human(now.dead_reclaimable),
                if now.dead == 1 { "it" } else { "them" },
            ));
        }
        lines
    }

    fn render_trend(&self, left: u16, mut y: u16, area: Rect, buf: &mut Buffer) {
        match &self.trend {
            Trend::Unavailable(why) => {
                buf.set_string(
                    left,
                    y,
                    format!("History unavailable, so nothing can be compared: {why}"),
                    DEFAULT,
                );
            }
            Trend::FirstScan => {
                buf.set_string(
                    left,
                    y,
                    "Recorded as the first scan of these roots. Run again later to see what changed.",
                    DEFAULT,
                );
            }
            Trend::Since(rows) => {
                buf.set_string(left, y, "Since the previous scan", HEAD);
                y += 1;
                // Most paths in a scan are unchanged. Listing them buries the
                // few that are not, which are the whole reason for the section.
                let moved: Vec<&TrendRow> = rows
                    .iter()
                    .filter(|r| r.change != Change::Unchanged)
                    .collect();
                if moved.is_empty() {
                    buf.set_string(left + 2, y, "nothing changed", DEFAULT);
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
                        DEFAULT,
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
fn way_to_candidates() -> String {
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
    format!("{presses} → {}", Screen::Candidates.name())
}

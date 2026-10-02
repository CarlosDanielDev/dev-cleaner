//! Drawing the dashboard: four blocks, laid out to what the terminal has room for.
//!
//! Every block is built as lines of styled runs first and drawn after, so how
//! tall a block is can be known before anything is placed. That is what lets a
//! short terminal drop a whole block instead of cutting one in half: the layout
//! tries the fullest arrangement, then each poorer one, and draws the first
//! whose height fits.

use super::Screen;
use super::bar::{self, Part};
use super::dashboard::{Dashboard, Group, Subject, Target, Trend};
use super::icons::Icon;
use super::keymap::{Action, bindings_for};
use super::palette::{Ramp, Theme};
use super::row::{clip, elide_path, elide_tail, heading, put};
use crate::bytes::human;
use crate::store::{Change, TrendRow};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use std::time::Duration;

type Line = Vec<(String, Style)>;

/// Sparkline steps, lowest first, and what a scan with no value draws.
const RAMP: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
const NO_VALUE: char = '·';

/// From this width, Disk and Analysed share a row.
const WIDE: u16 = 110;
/// Columns of inner width from which the breakdown runs in two columns.
const TWO_COLUMNS: usize = 120;
/// Columns between side-by-side blocks.
const GAP: usize = 3;
/// The most rows the breakdown ever shows before `+N more`.
const MAX_ROWS: usize = 6;
/// The most insights ever shown.
const MAX_INSIGHTS: usize = 4;
/// Columns the headline of an insight is padded to: the longest is 15.
const HEADLINE: usize = 15;

/// What is kept and what is dropped at one height, richest first.
#[derive(Clone, Copy)]
struct Variant {
    roots: bool,
    rows: usize,
    insights: usize,
    analysed: bool,
}

/// Lowest priority goes first: the roots line, then the breakdown's rows, then
/// insights past the first, then the Analysed block. The disk and the best
/// insight are the last things standing.
const VARIANTS: [Variant; 8] = [
    Variant {
        roots: true,
        rows: MAX_ROWS,
        insights: MAX_INSIGHTS,
        analysed: true,
    },
    Variant {
        roots: false,
        rows: MAX_ROWS,
        insights: MAX_INSIGHTS,
        analysed: true,
    },
    Variant {
        roots: false,
        rows: 4,
        insights: MAX_INSIGHTS,
        analysed: true,
    },
    Variant {
        roots: false,
        rows: 3,
        insights: 3,
        analysed: true,
    },
    Variant {
        roots: false,
        rows: 2,
        insights: 2,
        analysed: true,
    },
    Variant {
        roots: false,
        rows: 0,
        insights: 1,
        analysed: true,
    },
    Variant {
        roots: false,
        rows: 0,
        insights: 1,
        analysed: false,
    },
    Variant {
        roots: false,
        rows: 0,
        insights: 0,
        analysed: false,
    },
];

/// A heading and what is under it.
struct Block {
    title: Line,
    lines: Vec<Line>,
}

impl Block {
    fn height(&self) -> usize {
        1 + self.lines.len()
    }
}

/// A block and where it goes, relative to the top left of the body.
struct Placed {
    x: usize,
    y: usize,
    width: usize,
    block: Block,
}

impl Dashboard {
    pub fn render(&self, theme: &Theme, area: Rect, buf: &mut Buffer) {
        if area.width < 8 || area.height == 0 {
            return;
        }
        let left = area.x + 2;
        let width = area.width.saturating_sub(4) as usize;
        let wide = area.width >= WIDE;

        let mut chosen = None;
        for variant in VARIANTS {
            let plan = self.plan(theme, width, wide, variant);
            let height = plan.last().map_or(0, |p| p.y + p.block.height());
            let fits = height <= area.height as usize;
            chosen = Some(plan);
            if fits {
                break;
            }
        }

        // Past the last variant the body is too short for even the disk; what
        // does not fit is cut at the bottom edge, a row at a time, never past it.
        for placed in chosen.unwrap_or_default() {
            let x = left + placed.x as u16;
            let y = area.y + placed.y as u16;
            if y >= area.bottom() {
                continue;
            }
            heading(buf, theme, x, y, placed.width, &placed.block.title);
            for (i, line) in placed.block.lines.into_iter().enumerate() {
                let row = y + 1 + i as u16;
                if row >= area.bottom() {
                    break;
                }
                put(buf, x, row, &clip(line, placed.width));
            }
        }
    }

    fn plan(&self, theme: &Theme, width: usize, wide: bool, v: Variant) -> Vec<Placed> {
        let mut placed = Vec::new();
        let mut y = 0;

        if wide && v.analysed {
            let disk_w = (width - GAP) * 9 / 20;
            let analysed_w = width - GAP - disk_w;
            let disk = self.disk_block(theme, disk_w);
            let analysed = self.analysed_block(theme, analysed_w, v.roots);
            let tall = disk.height().max(analysed.height());
            placed.push(Placed {
                x: 0,
                y,
                width: disk_w,
                block: disk,
            });
            placed.push(Placed {
                x: disk_w + GAP,
                y,
                width: analysed_w,
                block: analysed,
            });
            y += tall;
        } else {
            let disk = self.disk_block(theme, width);
            y += disk.height();
            placed.push(Placed {
                x: 0,
                y: 0,
                width,
                block: disk,
            });
            if v.analysed {
                let analysed = self.analysed_block(theme, width, v.roots);
                placed.push(Placed {
                    x: 0,
                    y: y + 1,
                    width,
                    block: analysed,
                });
                y += 1 + placed[1].block.height();
            }
        }

        if v.rows > 0 {
            let block = self.where_block(theme, width, v.rows);
            placed.push(Placed {
                x: 0,
                y: y + 1,
                width,
                block,
            });
            y += 1 + placed.last().map_or(0, |p| p.block.height());
        }
        if v.insights > 0 {
            let block = self.insights_block(theme, width, v.insights);
            placed.push(Placed {
                x: 0,
                y: y + 1,
                width,
                block,
            });
        }
        placed
    }

    /// The volume in three parts: what can be rebuilt, what else is in use, and
    /// what is free.
    fn disk_block(&self, theme: &Theme, width: usize) -> Block {
        let title = vec![("Disk".to_string(), theme.head)];
        let Some(volume) = self.volume else {
            let mut lines = vec![vec![(
                "free space unavailable on this path".to_string(),
                theme.blocked,
            )]];
            if self.reclaimable > 0 {
                lines.push(vec![
                    (human(self.reclaimable), theme.size(self.reclaimable)),
                    (" rebuildable".to_string(), theme.safe),
                ]);
            }
            return Block { title, lines };
        };

        let mut lines = Vec::new();
        let place = match self.analysed.roots.first() {
            Some(root) => format!(
                " {} volume  {}",
                human(volume.total),
                elide_path(&root.display().to_string(), width.saturating_sub(18))
            ),
            None => format!(" {} volume", human(volume.total)),
        };
        lines.push(vec![
            (theme.icon(Icon::Disk).to_string(), theme.accent),
            (place, theme.text),
        ]);

        // Reclaimable is drawn inside the used portion, never beside the free
        // one: it is space the user does not have yet, and showing it as free
        // would overstate the disk by exactly the amount this tool is for.
        const USED_LABEL: usize = 10;
        let cells = bar::stacked_cells(theme, width.saturating_sub(USED_LABEL)).max(10);
        let reclaimable = self.reclaimable.min(volume.used());
        let other = volume.used() - reclaimable;
        let counts = bar::split(cells, [reclaimable, other, volume.free]);
        let used_percent = (volume.used() as u128 * 100)
            .checked_div(volume.total as u128)
            .unwrap_or(0);
        let mut gauge = bar::stacked(theme, counts);
        gauge.push((format!(" {used_percent:>3}% used"), theme.text));
        lines.push(gauge);

        // Each swatch is the glyph and the colour of the cells it names, and a
        // part that drew no cells is not in the legend. The figure is a fact,
        // so it is never muted: free space is drawn in muted cells and its
        // figure in text.
        let entry = |kind: Part, bytes: u64, label: &str| -> Line {
            let (glyph, style) = bar::part(theme, kind);
            let figure = if kind == Part::Free {
                theme.text
            } else {
                style
            };
            vec![
                (format!("{glyph} "), style),
                (human(bytes), figure),
                (format!(" {label}"), theme.muted),
            ]
        };
        let legend: Vec<Line> = [
            (Part::Reclaimable, reclaimable, "rebuildable", counts[0]),
            (Part::InUse, other, "other", counts[1]),
            (Part::Free, volume.free, "free", counts[2]),
        ]
        .into_iter()
        .filter(|(_, _, _, cells)| *cells > 0)
        .map(|(kind, bytes, label, _)| entry(kind, bytes, label))
        .collect();
        lines.extend(pack(legend, width, theme));

        let share = if volume.used() == 0 || reclaimable == 0 {
            "nothing in it can be rebuilt".to_string()
        } else {
            let tenths = reclaimable as u128 * 1000 / volume.used() as u128;
            // Under a tenth is not none: it is the case this tool lives in.
            let figure = match tenths {
                0 => "<0.1".to_string(),
                t => format!("{}.{}", t / 10, t % 10),
            };
            format!("{figure}% of what is used can be rebuilt")
        };
        lines.push(vec![(share, theme.text)]);
        Block { title, lines }
    }

    /// What the scan looked at. Context, not a call to action: quiet colours.
    fn analysed_block(&self, theme: &Theme, width: usize, roots: bool) -> Block {
        let a = &self.analysed;
        let title = vec![("Analysed".to_string(), theme.head)];
        let fact = |icon: Icon, figure: String, rest: &str| -> Line {
            vec![
                (format!("{} ", theme.icon(icon)), theme.violet),
                (figure, theme.text),
                (format!(" {rest}"), theme.muted),
            ]
        };
        let mut lines = pack(
            vec![
                fact(Icon::Projects, count(a.projects, "project", "projects"), ""),
                fact(
                    Icon::Rebuild,
                    a.with_rebuild.to_string(),
                    "with something to rebuild",
                ),
            ],
            width,
            theme,
        );
        lines.extend(pack(
            vec![
                fact(Icon::Entries, thousands(a.entries), "entries walked"),
                fact(
                    Icon::Measured,
                    a.measured.to_string(),
                    "directories measured",
                ),
                fact(Icon::Time, duration(a.elapsed), ""),
            ],
            width,
            theme,
        ));
        match &self.trend {
            Trend::FirstScan => lines.push(vec![(
                "First scan of these roots. Run again later to see what changed.".to_string(),
                theme.text,
            )]),
            Trend::Unavailable(why) => lines.push(vec![(
                format!("History unavailable, so nothing can be compared: {why}"),
                theme.blocked,
            )]),
            Trend::Since(_) => {}
        }
        if roots && !a.roots.is_empty() {
            let names: Vec<String> = a.roots.iter().map(|r| r.display().to_string()).collect();
            lines.push(vec![
                ("roots ".to_string(), theme.muted),
                (names.join("  "), theme.text),
            ]);
        }
        Block { title, lines }
    }

    /// Where the rebuildable bytes are, by kind, heaviest first.
    fn where_block(&self, theme: &Theme, width: usize, rows: usize) -> Block {
        let total: u64 = self.groups.iter().map(|g| g.bytes).sum();
        let ranked = self.ranked_groups();
        let mut title = vec![("Where it is".to_string(), theme.head)];
        if ranked.is_empty() {
            return Block {
                title,
                lines: vec![vec![(
                    "No build output found on these roots.".to_string(),
                    theme.text,
                )]],
            };
        }

        title.extend([
            ("  ".to_string(), theme.text),
            (human(total), theme.size(total)),
            (" rebuildable".to_string(), theme.muted),
        ]);
        let shown = rows.min(MAX_ROWS).min(ranked.len());
        // A row hidden is a row counted: the remainder is one line with its
        // own bytes, so the rows and that line add up to the heading.
        let hidden = &ranked[shown..];
        let columns = if width >= TWO_COLUMNS { 2 } else { 1 };
        let column_w = (width - GAP * (columns - 1)) / columns;
        let largest = ranked[0].bytes;

        let cells: Vec<Line> = ranked[..shown]
            .iter()
            .map(|g| group_cell(theme, g, largest, column_w))
            .collect();
        let mut lines: Vec<Line> = cells
            .chunks(columns)
            .map(|pair| {
                let mut line = Vec::new();
                for (i, cell) in pair.iter().enumerate() {
                    if i > 0 {
                        line.push((" ".repeat(GAP), theme.text));
                    }
                    line.extend(pad(cell.clone(), column_w, theme));
                }
                line
            })
            .collect();

        if !hidden.is_empty() {
            let rest: u64 = hidden.iter().map(|g| g.bytes).sum();
            lines.push(vec![
                (
                    format!(
                        "+{} more {}  ",
                        hidden.len(),
                        if hidden.len() == 1 { "kind" } else { "kinds" }
                    ),
                    theme.text,
                ),
                (human(rest), theme.size(rest)),
            ]);
        }
        if let Some(c) = self.top_by_inodes(1).first() {
            lines.push(vec![
                (format!("{} ", theme.icon(Icon::MostFiles)), theme.violet),
                ("most files  ".to_string(), theme.muted),
                (elide_path(&c.label, width.saturating_sub(34)), theme.text),
                ("  ".to_string(), theme.text),
                (thousands(c.inodes), theme.accent),
                (" inodes".to_string(), theme.muted),
            ]);
        }
        Block { title, lines }
    }

    /// At most `limit` ranked sentences, or one calm line when there are none.
    fn insights_block(&self, theme: &Theme, width: usize, limit: usize) -> Block {
        let title = vec![("Insights".to_string(), theme.head)];
        let all = self.insights(theme);
        if all.is_empty() {
            let mut line = vec![("Nothing to rebuild on these roots.".to_string(), theme.text)];
            if let Some(v) = self.volume {
                let yours = v.used().saturating_sub(self.reclaimable);
                line.push((" ".to_string(), theme.text));
                line.push((human(yours), theme.size(yours)));
                line.push((" is yours.".to_string(), theme.text));
            }
            return Block {
                title,
                lines: vec![line],
            };
        }

        let lines = all
            .into_iter()
            .take(limit.min(MAX_INSIGHTS))
            .map(|insight| {
                let way = insight.target.map(|t| way_to(theme, t.screen));
                let way_w: usize = way.as_ref().map_or(0, |w| chars(w) + 2);
                let prefix = 2 + HEADLINE + 1;
                // The way is the first thing to give: the sentence is the
                // insight, and a way squeezed beside a cut sentence helps
                // nobody.
                let mut text = insight.text;
                let way = way.filter(|_| prefix + chars(&text) + way_w <= width);
                let way_w = if way.is_some() { way_w } else { 0 };
                let text_w = width.saturating_sub(prefix + way_w);

                if insight.spark {
                    let room = text_w.saturating_sub(chars(&text) + 2);
                    if let Some(parts) = self.sparkline(theme, room) {
                        text.push(("  ".to_string(), theme.text));
                        text.extend(parts);
                    }
                }
                let mut line = vec![
                    (theme.icon(insight.icon).to_string(), insight.tone),
                    (format!(" {:<HEADLINE$} ", insight.headline), insight.tone),
                ];
                let text = clip(text, text_w);
                let used = chars(&text);
                line.extend(text);
                if let Some(way) = way {
                    line.push((" ".repeat(text_w.saturating_sub(used) + 2), theme.text));
                    line.extend(way);
                }
                line
            })
            .collect();
        Block { title, lines }
    }

    /// Everything worth saying, in the order it ranks: the biggest win, what has
    /// gone quiet, what was held back, what changed. One that has nothing to
    /// say is not in the list.
    fn insights(&self, theme: &Theme) -> Vec<Insight> {
        let now = &self.now;
        let mut out = Vec::new();
        // Enter follows one insight, so one insight may say where it goes.
        let lead = self.lead();
        let target = |subject| lead.clone().filter(|t| t.subject == subject);

        if let Some(g) = self.biggest_win() {
            out.push(Insight {
                icon: Icon::BiggestWin,
                headline: "Biggest win",
                tone: theme.safe,
                text: vec![
                    (human(g.offerable_bytes), theme.size(g.offerable_bytes)),
                    (format!(" of {} in ", g.label), theme.text),
                    (
                        count(g.offerable_dirs, "directory", "directories"),
                        theme.text,
                    ),
                    (", all rebuildable".to_string(), theme.text),
                ],
                target: target(Subject::BiggestWin),
                spark: false,
            });
        }

        if now.dead > 0 {
            out.push(Insight {
                icon: Icon::GoneQuiet,
                headline: "Gone quiet",
                tone: theme.violet,
                text: vec![
                    (
                        format!(
                            "{} {} ",
                            count(now.dead, "dead project", "dead projects"),
                            if now.dead == 1 { "holds" } else { "hold" }
                        ),
                        theme.text,
                    ),
                    (
                        human(now.dead_reclaimable),
                        theme.size(now.dead_reclaimable),
                    ),
                    (" of build output".to_string(), theme.text),
                ],
                target: target(Subject::GoneQuiet),
                spark: false,
            });
        }

        let held: usize = now.blocked.iter().map(|(_, n)| n).sum();
        if held > 0 {
            let top = now.blocked.iter().max_by_key(|(_, n)| *n);
            // The count of reasons comes before the reason, so a narrow line
            // cuts the sentence and not the fact that there are more.
            let mut text = vec![(
                format!("{} kept", count(held, "entry", "entries")),
                theme.text,
            )];
            if now.blocked.len() > 1 {
                text.push((format!(", {} reasons", now.blocked.len()), theme.text));
            }
            if let Some((reason, _)) = top {
                text.push((format!(": {}", reason.trim_end_matches('.')), theme.text));
            }
            text.push((
                " — dev-cleaner will not offer these".to_string(),
                theme.text,
            ));
            out.push(Insight {
                icon: Icon::HeldBack,
                headline: "Held back",
                tone: theme.blocked,
                text,
                target: target(Subject::HeldBack),
                spark: false,
            });
        }

        if let Some(text) = self.since(theme) {
            out.push(Insight {
                icon: Icon::Trend,
                headline: "Since last scan",
                tone: theme.accent,
                text,
                target: None,
                spark: true,
            });
        }
        out
    }

    /// Which way reclaimable moved against the scan before this one, and how
    /// many paths changed.
    fn since(&self, theme: &Theme) -> Option<Line> {
        let known: Vec<u64> = self.history.iter().flatten().copied().collect();
        let mut text: Line = Vec::new();
        if let [.., before, now] = known[..]
            && now != before
        {
            // Growing is attention, shrinking is calm, and the word says so
            // with no colour at all. No change is not an insight: it is the
            // absence of one.
            let (word, by, style) = if now > before {
                ("up", now - before, theme.blocked)
            } else {
                ("down", before - now, theme.safe)
            };
            text.push(("reclaimable is ".to_string(), theme.text));
            text.push((format!("{word} {}", human(by)), style));
        }
        if let Trend::Since(rows) = &self.trend {
            let moved = rows
                .iter()
                .filter(|r: &&TrendRow| r.change != Change::Unchanged)
                .count();
            if moved > 0 {
                if !text.is_empty() {
                    text.push((" · ".to_string(), theme.violet));
                }
                text.push((
                    format!("{} changed", count(moved, "path", "paths")),
                    theme.text,
                ));
            }
        }
        (!text.is_empty()).then_some(text)
    }

    /// The reclaimable total over the last scans, newest at the right, or
    /// `None` when fewer than two scans have a value to draw, or there is no
    /// room for it.
    ///
    /// Scaled to the highest value in the window, not to the disk: the line is
    /// about the shape of the change, and against 460 GB every point would be
    /// the bottom step. At most `cells` wide, keeping the newest scans; the
    /// words after the glyphs are dropped, longest first, before the glyphs are.
    fn sparkline(&self, theme: &Theme, cells: usize) -> Option<Line> {
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
        let label = format!("over the last {} scans", window.len());
        // The widest tail that fits, and the label is dropped before the
        // figures: the range is what the line is for.
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
        let mut parts: Line = glyphs
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
}

/// One ranked sentence, before it is laid out.
struct Insight {
    icon: Icon,
    headline: &'static str,
    tone: Style,
    text: Line,
    /// Where Enter leads from here, when this is the insight it follows.
    target: Option<Target>,
    /// Whether the sparkline follows the sentence.
    spark: bool,
}

/// One row of the breakdown, built in the order of what can be spared when the
/// column is narrow: the name and the bytes always, then the bar, then the
/// count, then the command.
fn group_cell(theme: &Theme, g: &Group, largest: u64, width: usize) -> Line {
    const NAME: usize = 13;
    const BAR: usize = 10;
    const BYTES: usize = 10;
    const DIRS: usize = 8;
    const MIN_COMMAND: usize = 8;

    let mut cell = vec![
        (
            format!("{} ", theme.icon(Icon::of(g.ecosystem))),
            theme.accent,
        ),
        (format!("{:<NAME$}", elide_tail(&g.label, NAME)), theme.text),
    ];
    let mut room = width.saturating_sub(2 + NAME);
    if room > BYTES + 1 + BAR {
        cell.push((" ".to_string(), theme.text));
        cell.extend(bar::share(theme, Ramp::Measure, g.bytes, largest, BAR));
        room -= BAR + 1;
    }
    cell.push((format!("{:>BYTES$}", human(g.bytes)), theme.size(g.bytes)));
    room = room.saturating_sub(BYTES);
    if room > DIRS + 2 {
        cell.push((
            format!("  {:>DIRS$}", count(g.dirs, "dir", "dirs")),
            theme.text,
        ));
        room -= DIRS + 2;
        if room >= MIN_COMMAND + 2 {
            cell.push(("  ".to_string(), theme.text));
            cell.push((elide_tail(&g.regen, room - 2), theme.safe));
        }
    }
    cell
}

/// `entries` laid out left to right in rows no wider than `width`, two spaces
/// apart, a new row when the next one would not fit.
fn pack(entries: Vec<Line>, width: usize, theme: &Theme) -> Vec<Line> {
    let mut rows: Vec<Line> = Vec::new();
    let mut used = 0;
    for entry in entries {
        let w = chars(&entry);
        match rows.last_mut() {
            Some(row) if used + 2 + w <= width => {
                row.push(("  ".to_string(), theme.text));
                row.extend(entry);
                used += 2 + w;
            }
            _ => {
                rows.push(entry);
                used = w;
            }
        }
    }
    rows
}

/// `parts` filled out with spaces to exactly `width` columns, or cut to it.
fn pad(parts: Line, width: usize, theme: &Theme) -> Line {
    let mut parts = clip(parts, width);
    let short = width.saturating_sub(chars(&parts));
    parts.push((" ".repeat(short), theme.text));
    parts
}

fn chars(parts: &[(String, Style)]) -> usize {
    parts.iter().map(|(t, _)| t.chars().count()).sum()
}

/// `n` with the noun agreeing with it.
fn count(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// `1234567` as `1,234,567`.
fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// A scan time: tenths of a second, then minutes once it is long.
fn duration(d: Duration) -> String {
    let secs = d.as_secs_f64();
    if secs < 60.0 {
        format!("{secs:.1} s")
    } else {
        format!("{} min {} s", d.as_secs() / 60, d.as_secs() % 60)
    }
}

/// The presses that lead from the dashboard to `target`.
///
/// Walked over `Screen::next` and read from the table, so the line cannot name
/// a key that does not work or a screen that is not on the way.
fn way_to(theme: &Theme, target: Screen) -> Line {
    let mut keys: Vec<String> = Vec::new();
    let mut screen = Screen::Dashboard;
    while screen != target {
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
        (target.name().to_string(), theme.accent),
    ]
}

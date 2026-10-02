//! Reading the plan before it can be carried out.
//!
//! The screen owns no candidates of its own: it is a window onto the plan the
//! router is holding, and every draw reads it again. A copy taken when the
//! screen was built would keep showing a set the plan no longer holds, which is
//! the one thing review must never do.
//!
//! The plan is drawn in the candidates table's look, grouped by the project each
//! entry is in: a head per project with its subtotal, the entries under it, and
//! a total at the foot. Grouping is how the plan is *shown*; what it holds, and
//! in what order, is the plan's.

use std::path::Path;

use super::bar;
use super::keymap::Motion;
use super::kit::{
    Align, Col, GAP, KEYS_LEAD, Locator, Table, Where, empty_body, kind_badge, section, tier_badge,
};
use super::palette::{Ramp, Theme};
use super::row::{elide_tail, put};
use super::showing;
use crate::bytes::human;
use crate::safety::{Candidate, Plan, Reviewed};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;

/// Where the list has been scrolled to.
///
/// Taking `Plan<Reviewed>` on every call rather than holding one is what makes
/// the type do the work: a draft cannot be reviewed, because `Plan<Draft>` will
/// not go through this door.
#[derive(Debug, Default)]
pub struct Review {
    /// The first line shown: a line is an entry or a project's head.
    top: usize,
}

/// Rows of the screen that are not the list: the section head, the header row,
/// the total and the sentence under it.
pub(super) const CHROME: usize = 4;

/// Cells of a project's share of the plan, beside its subtotal.
const SHARE_CELLS: usize = 8;

/// The narrowest the project and path column is drawn.
const PATH_MIN: u16 = 40;

/// One line of the list.
#[derive(Debug, Clone, Copy)]
enum Line<'a> {
    /// A project, with how many entries of the plan it holds and their bytes.
    Head(Group<'a>),
    Item(&'a Candidate),
}

#[derive(Debug, Clone, Copy)]
struct Group<'a> {
    /// The project's root, or `None` for entries in no project.
    root: Option<&'a Path>,
    entries: usize,
    bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Field {
    Tier,
    Size,
    Kind,
    Path,
    Command,
}

/// The plan, a project at a time: the biggest subtotal first, and each
/// project's entries in the order the plan holds them.
fn lines<'a>(plan: &'a Plan<Reviewed>, locator: &'a Locator) -> Vec<Line<'a>> {
    let mut groups: Vec<(Option<&Path>, Vec<&Candidate>)> = Vec::new();
    for c in plan.items() {
        let root = locator.site(&c.path).map(|s| s.project.root.as_path());
        match groups.iter_mut().find(|(r, _)| *r == root) {
            Some((_, members)) => members.push(c),
            None => groups.push((root, vec![c])),
        }
    }
    let bytes = |members: &[&Candidate]| members.iter().map(|c| c.bytes).sum::<u64>();
    groups.sort_by_key(|(root, members)| (std::cmp::Reverse(bytes(members)), *root));
    groups
        .into_iter()
        .flat_map(|(root, members)| {
            let head = Line::Head(Group {
                root,
                entries: members.len(),
                bytes: bytes(&members),
            });
            std::iter::once(head).chain(members.into_iter().map(Line::Item))
        })
        .collect()
}

impl Review {
    pub fn new() -> Self {
        Self::default()
    }

    /// The entries on screen, always a full window where there are enough lines.
    pub fn visible<'a>(
        &self,
        plan: &'a Plan<Reviewed>,
        locator: &'a Locator,
        height: usize,
    ) -> Vec<&'a Candidate> {
        let lines = lines(plan, locator);
        let start = self.start(lines.len(), height);
        lines[start..(start + height).min(lines.len())]
            .iter()
            .filter_map(|l| match l {
                Line::Item(c) => Some(*c),
                Line::Head(_) => None,
            })
            .collect()
    }

    /// How many lines the list is: an entry each, and a head for each project.
    pub fn lines(&self, plan: &Plan<Reviewed>, locator: &Locator) -> usize {
        lines(plan, locator).len()
    }

    /// First line on screen, for a list of `len` lines in a window of `height`.
    pub(super) fn offset(&self, len: usize, height: usize) -> usize {
        self.start(len, height)
    }

    fn start(&self, len: usize, height: usize) -> usize {
        self.top.min(len.saturating_sub(height))
    }

    /// Scroll the list, which is `height` rows tall.
    ///
    /// There is no cursor: nothing here acts on a single row, and a cursor that
    /// selects nothing is a promise the screen does not keep. What moves is the
    /// window, and the foot says where it is.
    pub fn scroll(
        &mut self,
        motion: Motion,
        plan: &Plan<Reviewed>,
        locator: &Locator,
        height: usize,
    ) {
        let last = self.lines(plan, locator).saturating_sub(height);
        // Clamped first, so a window that grew since the last scroll does not
        // leave `Up` spending presses on rows that are already in view.
        self.top = self.top.min(last);
        self.top = match motion {
            Motion::Up => self.top.saturating_sub(1),
            Motion::Down => (self.top + 1).min(last),
            Motion::Top => 0,
            Motion::Bottom => last,
            Motion::PageUp => self.top.saturating_sub(height),
            Motion::PageDown => (self.top + height).min(last),
        };
    }

    pub fn render(
        &self,
        theme: &Theme,
        plan: &Plan<Reviewed>,
        locator: &Locator,
        area: Rect,
        buf: &mut Buffer,
    ) {
        if area.height < CHROME as u16 {
            return;
        }
        let left = area.x + 1;
        let width = area.width.saturating_sub(2) as usize;
        let all = lines(plan, locator);
        let rows = area.height as usize - CHROME;
        let start = self.start(all.len(), rows);
        let window = &all[start..(start + rows).min(all.len())];
        let items = plan.items();
        let projects = all.iter().filter(|l| matches!(l, Line::Head(_))).count();

        // The phrase the confirm screen repeats: what is confirmed is what was
        // reviewed, count for count and total for total.
        let counts = format!("({} items, {})", items.len(), human(plan.total_bytes()));
        section(
            buf,
            theme,
            (left, area.y),
            width,
            "The plan",
            &counts,
            vec![],
        );

        if items.is_empty() {
            let lines = vec![
                "The plan holds nothing.".to_string(),
                format!("{KEYS_LEAD}Esc candidates"),
            ];
            empty_body(buf, theme, (left, area.y + 2), (width, rows), &lines);
            return;
        }

        let kinds: Vec<String> = items.iter().map(|c| kind_badge(theme, &c.path).0).collect();
        let commands: Vec<String> = items
            .iter()
            .map(|c| super::row::describe(&c.safety))
            .collect();
        let widest = |texts: &[String], least: &str| {
            texts
                .iter()
                .map(|t| t.chars().count())
                .chain([least.chars().count()])
                .max()
                .unwrap_or(0) as u16
        };
        let table = Table::new(
            vec![
                Col::fixed(Field::Tier, 1 + GAP, Align::Left),
                Col::fixed(Field::Size, 10 + GAP, Align::Right),
                Col::fixed(Field::Kind, widest(&kinds, "kind") + GAP, Align::Left),
                Col::flex(Field::Path, PATH_MIN),
                Col::squeezable(
                    Field::Command,
                    widest(&commands, "comes back as").min((width * 2 / 5) as u16) + GAP,
                    13 + GAP,
                    Align::Left,
                ),
            ],
            vec![
                Field::Path,
                Field::Size,
                Field::Tier,
                Field::Command,
                Field::Kind,
            ],
        );
        // Under a project's head an entry says only the path inside it; the
        // project is said again where the window starts below its head.
        let inside = |c: &Candidate| Where::of(locator, &c.path);
        let wanted = items.iter().map(|c| inside(c).width()).max().unwrap_or(0) as u16 + GAP;
        let fit = table.fit(width as u16, wanted);
        let at = |field| left + fit.x_of(field).unwrap_or(0);

        for (field, x) in &fit.drawn {
            let text = match field {
                Field::Tier => continue,
                Field::Size => "size",
                Field::Kind => "kind",
                Field::Path => "project / path",
                Field::Command => "comes back as",
            };
            table.put(buf, (left + x, area.y + 1), *field, text, theme.muted);
        }

        let index = |c: &Candidate| items.iter().position(|i| std::ptr::eq(i, c)).unwrap_or(0);
        // Entries whose head is above the window carry their project themselves.
        let mut headed = matches!(window.first(), Some(Line::Head(_)));
        for (y, line) in (area.y + 2..).zip(window) {
            match line {
                Line::Head(group) => {
                    headed = true;
                    let site = group.root.and_then(|r| locator.site(r));
                    let (title, note) = site
                        .map_or(("Outside any project".to_string(), None), |s| {
                            (s.project.label.clone(), s.project.note.clone())
                        });
                    let title = match note {
                        Some(note) => format!("{title}  {note}"),
                        None => title,
                    };
                    let counts = format!("{} · {}", entries(group.entries), human(group.bytes));
                    let share = bar::share(
                        theme,
                        Ramp::Measure,
                        group.bytes,
                        plan.total_bytes(),
                        SHARE_CELLS,
                    );
                    let mut extra = vec![("  ".to_string(), theme.text)];
                    extra.extend(share);
                    section(buf, theme, (left, y), width, &title, &counts, extra);
                }
                Line::Item(c) => {
                    let (tier, tier_style) = tier_badge(theme, &c.safety);
                    buf.set_string(at(Field::Tier), y, tier.to_string(), tier_style);
                    table.put(
                        buf,
                        (at(Field::Size), y),
                        Field::Size,
                        &human(c.bytes),
                        theme.size(c.bytes),
                    );
                    let i = index(c);
                    if fit.has(Field::Kind) {
                        let style = kind_badge(theme, &c.path).1;
                        table.put(buf, (at(Field::Kind), y), Field::Kind, &kinds[i], style);
                    }
                    let place = inside(c);
                    let runs = if headed {
                        place.inside_runs(theme, fit.flex_w as usize - 2)
                    } else {
                        place.runs(theme, fit.flex_w as usize - 2)
                    };
                    put(buf, at(Field::Path), y, &runs);
                    if let Some(x) = fit.x_of(Field::Command) {
                        let room = fit.room(Field::Command);
                        buf.set_string(left + x, y, elide_tail(&commands[i], room), theme.safe);
                    }
                }
            }
        }

        // The total, and where the window is in it; then what the lines mean,
        // said once at the bottom, where the eye lands after the list: the
        // right-hand column above is a promise, and this is what it means.
        let seen_before = all[..start]
            .iter()
            .filter(|l| matches!(l, Line::Item(_)))
            .count();
        let seen = window.iter().filter(|l| matches!(l, Line::Item(_))).count();
        let total = format!(
            "{} · {} · {}  {}",
            entries(items.len()),
            if projects == 1 {
                "1 project".to_string()
            } else {
                format!("{projects} projects")
            },
            human(plan.total_bytes()),
            showing(seen_before, seen, items.len()),
        );
        let style: Style = theme.text;
        buf.set_string(left, area.bottom() - 2, elide_tail(&total, width), style);
        buf.set_string(
            left,
            area.bottom() - 1,
            "Each line names the command that rebuilds it. Esc to change the plan.",
            theme.muted,
        );
    }
}

/// `1 entry`, `7 entries`.
fn entries(n: usize) -> String {
    format!("{n} {}", if n == 1 { "entry" } else { "entries" })
}

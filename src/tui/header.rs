//! The header: a status bar in the idiom of btop, lazygit and k9s.
//!
//! Where the terminal is tall enough, the icon fills the band's five rows and
//! beside it, on one left edge, sit three lines: the title (the wordmark, a
//! `▸`, the screen's name, and the context of the scan at the right), the
//! stepper (where this screen is in the six-step flow) and the hints (the keys
//! that lead out of it). A rule from column 0 closes the band on the row under
//! the icon, or on the screens that remove things, a bar in the danger red.
//!
//! Under that size the same elements share one status line over the rule.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};

use super::Screen;
use super::logo;
use super::palette::Theme;
use super::projects::truncate;
use super::row::put;
use super::run::WAY_SEPARATOR;

/// What the header says: the screen's name, the keys out of it, the facts of the
/// scan, and whether the screen is one that removes things, which carries its
/// danger in a red bar.
pub(super) struct Header<'a> {
    pub name: &'a str,
    /// Where in the flow this is. The running screen is the confirm screen still.
    pub stage: Screen,
    pub way: Option<&'a str>,
    pub danger: bool,
    /// Where the scan was run, cut in the middle to the room the facts leave.
    pub place: Option<&'a str>,
    /// What else is known about the scan, in the order it is read. When the
    /// title line has no room for all of it the first goes first.
    pub context: Vec<String>,
    /// The setting the screen is drawn in (`theme: neon`), said quietly after
    /// the facts. It is the first thing to go: it never takes room from a fact
    /// or squeezes the place out.
    pub setting: Option<String>,
    /// Whether the prompt's block cursor is lit: it blinks on the tick, in the
    /// themes whose title is a prompt, and takes its column either way.
    pub cursor: bool,
}

/// The six steps of the flow, as the stepper names them.
const STEPS: [(Screen, &str); 6] = [
    (Screen::Dashboard, "Dashboard"),
    (Screen::Projects, "Projects"),
    (Screen::Candidates, "Candidates"),
    (Screen::Review, "Plan"),
    (Screen::Confirm, "Confirm"),
    (Screen::Result, "Result"),
];

const CONTEXT_JOIN: &str = "  ·  ";
/// The narrowest a place is worth drawing, and the widest it is given.
const PLACE_MIN: usize = 14;
const PLACE_MAX: usize = 40;
const TITLE_ARROW: &str = " ▸ ";

/// Paint the header into the top of `area`.
///
/// Text wins over the icon: a frame that cannot fit both draws the compact
/// header, in a header that keeps its height.
pub(super) fn render(theme: &Theme, buf: &mut Buffer, area: Rect, tall: bool, header: &Header) {
    let beside = area.x + 1 + logo::width(theme) + logo::GAP;
    let name = capital(header.name);
    // The keys out of a screen are what the hints are for: the facts between
    // them go before the icon does.
    let way = header
        .way
        .map(|w| fit_way(w, area.right().saturating_sub(beside + 1) as usize));
    let title_width: usize = title(theme, &name, header.cursor)
        .iter()
        .map(|(t, _)| t.chars().count())
        .sum();
    let widest = title_width.max(way.as_ref().map_or(0, |w| w.chars().count()));
    let icon = tall && logo::fits(area.right(), beside, widest);
    let band = |buf: &mut Buffer, y: u16, title: &str| {
        buf.set_string(
            area.x,
            y,
            " ".repeat(area.width as usize),
            theme.warning_band,
        );
        buf.set_string(area.x + 1, y, title, theme.warning_band);
    };
    let rule = |buf: &mut Buffer, y: u16| {
        buf.set_string(
            area.x,
            y,
            theme.rule_line(area.width as usize),
            theme.violet,
        );
    };
    if icon {
        let avail = area.right().saturating_sub(beside + 1) as usize;
        logo::draw(theme, buf, area.x + 1, area.y);
        let end = put(buf, beside, area.y + 1, &title(theme, &name, header.cursor));
        let _ = context(theme, buf, area, area.y + 1, end, 0, header);
        put(
            buf,
            beside,
            area.y + 2,
            &stepper(theme, header.stage, avail),
        );
        if let Some(way) = &way {
            put(
                buf,
                beside,
                area.y + 3,
                &way_parts(theme, &truncate(way, avail)),
            );
        }
        let closing = area.y + logo::HEIGHT;
        if header.danger {
            band(buf, closing, &format!("{name}  ·  files go to the Trash"));
        } else {
            rule(buf, closing);
        }
        return;
    }
    // The compact header. The danger band is the whole of the first row, in
    // the band's own bold ink: the brand's gradient is no colour to put on red.
    // It keeps the hints under it, as the way to the hold is not one to lose.
    let left = area.x + 1;
    if header.danger {
        let title = if theme.prompt() {
            format!(
                "C:\\{}\\{}>",
                logo::NAME.to_uppercase(),
                name.to_uppercase()
            )
        } else {
            format!("{}  ·  {name}", logo::NAME)
        };
        band(buf, area.y, &title);
        if let Some(way) = header.way {
            let room = area.width.saturating_sub(2) as usize;
            put(
                buf,
                left,
                area.y + 1,
                &way_parts(theme, &truncate(way, room)),
            );
        }
        return;
    }
    let end = put(buf, left, area.y, &title(theme, &name, header.cursor));
    // What the scan says is a fact, and the stepper's glyphs are not: they take
    // only the room it leaves.
    let dots = dots(theme, header.stage);
    let wide: usize = dots.iter().map(|(t, _)| t.chars().count()).sum();
    let facts = context(theme, buf, area, area.y, end, wide + 2, header).unwrap_or(area.right());
    if end as usize + 2 + wide + 2 <= facts as usize {
        put(buf, end + 2, area.y, &dots);
    }
    if area.height > 1 {
        rule(buf, area.y + 1);
    }
}

/// The wordmark, a quiet `▸`, and the screen's name: the primary fact, in bold.
/// Where the theme has a prompt for a title it is `C:\DEV-CLEANER\SCREEN>` and
/// a block cursor, lit or dark, which keeps its column when it is dark.
fn title(theme: &Theme, name: &str, cursor: bool) -> Vec<(String, Style)> {
    if theme.prompt() {
        let mut parts = vec![("C:\\".to_string(), theme.muted)];
        parts.extend(
            logo::wordmark(theme)
                .into_iter()
                .map(|(letter, style)| (letter.to_uppercase(), style)),
        );
        parts.push(("\\".to_string(), theme.muted));
        parts.push((name.to_uppercase(), theme.head));
        parts.push((">".to_string(), theme.text));
        parts.push((if cursor { "█" } else { " " }.to_string(), theme.head));
        return parts;
    }
    let mut parts = logo::wordmark(theme);
    parts.push((TITLE_ARROW.to_string(), theme.violet));
    parts.push((name.to_string(), theme.text.add_modifier(Modifier::BOLD)));
    parts
}

/// Draw the facts of the scan ending two columns from the right edge, as many
/// as leave three columns clear of what the line already holds.
///
/// The place takes what the facts leave, down to [`PLACE_MIN`] columns, or goes.
/// `reserve` columns are kept clear of them. Returns the column the facts start
/// at, when there are any.
fn context(
    theme: &Theme,
    buf: &mut Buffer,
    area: Rect,
    y: u16,
    end: u16,
    reserve: usize,
    header: &Header,
) -> Option<u16> {
    let room = (area.right().saturating_sub(end + 3 + 2) as usize).saturating_sub(reserve);
    let join = CONTEXT_JOIN.chars().count();
    let width = |facts: &[String]| {
        facts.iter().map(|f| f.chars().count()).sum::<usize>()
            + join * facts.len().saturating_sub(1)
    };
    let fit = |shown: &[String]| {
        let left = room.saturating_sub(width(shown) + if shown.is_empty() { 0 } else { join });
        (left, header.place.filter(|_| left >= PLACE_MIN))
    };
    // The setting is kept only when every fact and the place still fit with it.
    let setting = header.setting.as_ref().filter(|s| {
        let all = [header.context.as_slice(), std::slice::from_ref(*s)].concat();
        width(&all) <= room && (header.place.is_none() || fit(&all).1.is_some())
    });
    let mut shown = header.context.as_slice();
    while !shown.is_empty() && width(shown) > room {
        shown = &shown[1..];
    }
    let (left, place) = match setting {
        Some(s) => fit(&[header.context.as_slice(), std::slice::from_ref(s)].concat()),
        None => fit(shown),
    };
    let place = place.map(|p| middle(p, left.min(PLACE_MAX)));
    let mut parts: Vec<(&str, Style)> = Vec::new();
    if let Some(place) = &place {
        parts.push((place, theme.text));
    }
    for fact in shown {
        if !parts.is_empty() {
            parts.push((CONTEXT_JOIN, theme.violet));
        }
        parts.push((fact, theme.text));
    }
    if let Some(setting) = setting {
        if !parts.is_empty() {
            parts.push((CONTEXT_JOIN, theme.violet));
        }
        parts.push((setting, theme.muted));
    }
    let total: usize = parts.iter().map(|(t, _)| t.chars().count()).sum();
    if total == 0 {
        return None;
    }
    let at = area.right().saturating_sub(2 + total as u16);
    put(buf, at, y, &parts);
    Some(at)
}

/// `text` cut in the middle to `width` columns, so both the start of a path and
/// the directory it ends in stay.
fn middle(text: &str, width: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= width {
        return text.to_string();
    }
    let keep = width.saturating_sub(1);
    let head = keep.div_ceil(3);
    let tail = keep - head;
    let start: String = chars[..head].iter().collect();
    let end: String = chars[chars.len() - tail..].iter().collect();
    format!("{start}…{end}")
}

/// `name` with its first letter in capitals.
fn capital(name: &str) -> String {
    let mut chars = name.chars();
    chars
        .next()
        .map(|c| c.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

/// One step: its marker, and its word. The glyph and the word carry the state;
/// the colour only agrees.
fn step(theme: &Theme, at: usize, current: usize) -> [(String, Style); 2] {
    let [done, here, to_come] = theme.steps();
    let (marker, label) = match at.cmp(&current) {
        std::cmp::Ordering::Less => (done, theme.text),
        std::cmp::Ordering::Equal => (here, theme.head),
        std::cmp::Ordering::Greater => (to_come, theme.muted),
    };
    let marker_style = if at < current { theme.safe } else { label };
    [
        (marker.to_string(), marker_style),
        (STEPS[at].1.to_string(), label),
    ]
}

fn current(stage: Screen) -> usize {
    STEPS.iter().position(|(s, _)| *s == stage).unwrap_or(0)
}

/// The flow as one line, in the most it can say in `width` columns: all six
/// steps; the one before, this one and the one after, between `‹` and `›`; or
/// this one alone with its place in the six. Whole steps only, never cut.
fn stepper(theme: &Theme, stage: Screen, width: usize) -> Vec<(String, Style)> {
    let now = current(stage);
    let join = format!(" {0}{0} ", theme.rule());
    let line = |from: usize, to: usize, edges: bool| {
        let mut parts = Vec::new();
        if edges && from > 0 {
            parts.push(("‹ ".to_string(), theme.muted));
        }
        for at in from..=to {
            if at > from {
                parts.push((join.clone(), theme.muted));
            }
            parts.extend(step(theme, at, now));
        }
        if edges && to + 1 < STEPS.len() {
            parts.push((" ›".to_string(), theme.muted));
        }
        parts
    };
    let wide =
        |parts: &[(String, Style)]| -> usize { parts.iter().map(|(t, _)| t.chars().count()).sum() };
    let all = line(0, STEPS.len() - 1, false);
    if wide(&all) <= width {
        return all;
    }
    let near = line(now.saturating_sub(1), (now + 1).min(STEPS.len() - 1), true);
    if wide(&near) <= width {
        return near;
    }
    let mut alone = line(now, now, false);
    alone.push((format!(" {}/{}", now + 1, STEPS.len()), theme.muted));
    alone
}

/// The compact header's stepper: a glyph a step, and the place in the six.
fn dots(theme: &Theme, stage: Screen) -> Vec<(String, Style)> {
    let now = current(stage);
    let mut parts: Vec<(String, Style)> = (0..STEPS.len())
        .map(|at| {
            let [(marker, style), _] = step(theme, at, now);
            // Bracketed markers are read one at a time, so they keep a space
            // between them; the bare glyphs run together, as they always did.
            let last = at + 1 == STEPS.len();
            let marker = if theme.prompt() && !last {
                marker
            } else {
                marker.trim_end().to_string()
            };
            (marker, style)
        })
        .collect();
    parts.push((format!(" {}/{}", now + 1, STEPS.len()), theme.muted));
    parts
}

/// `way` in `width` columns if it can be, by dropping the parts that are no key
/// (the last first) where there are keys to keep; whole as it was, for the
/// caller to find too long, if not.
fn fit_way(way: &str, width: usize) -> String {
    if !way.contains(['←', '→']) {
        return way.to_string();
    }
    let mut parts: Vec<&str> = way.split(WAY_SEPARATOR).collect();
    while parts.join(WAY_SEPARATOR).chars().count() > width {
        match parts.iter().rposition(|p| !p.contains(['←', '→'])) {
            Some(at) if parts.len() > 1 => parts.remove(at),
            _ => return way.to_string(),
        };
    }
    parts.join(WAY_SEPARATOR)
}

/// The hints row: key caps with their labels quiet, and every fact in text.
///
/// `Esc ← dashboard`, `Enter → candidates`, `hold P → result`: the key as a cap,
/// the arrow quiet, the screen it leads to in the text colour. What the screen
/// is, rather than what to press, is quiet too.
fn way_parts<'a>(theme: &Theme, line: &'a str) -> Vec<(&'a str, Style)> {
    let mut parts = Vec::new();
    for (i, part) in line.split(WAY_SEPARATOR).enumerate() {
        if i > 0 {
            parts.push((WAY_SEPARATOR, theme.violet));
        }
        // `Esc ← Back`, `Enter → Next`, `hold P → Next`: the key, then the arrow.
        let (key, rest) = match part.split_once(' ') {
            Some((key @ ("Esc" | "Enter"), rest)) => (Some(key), rest),
            Some(("hold", rest)) => {
                parts.push(("hold ", theme.text));
                let (k, rest) = rest.split_once(' ').unwrap_or((rest, ""));
                (Some(k), rest)
            }
            _ => (None, part),
        };
        if let Some(key) = key {
            parts.push((key, theme.key));
            parts.push((" ", theme.text));
        }
        let Some(at) = rest.find(['←', '→']) else {
            let own = matches!(rest, "the run is over");
            parts.push((rest, if own { theme.muted } else { theme.text }));
            continue;
        };
        let arrow = at + rest[at..].chars().next().map_or(0, char::len_utf8);
        parts.push((&rest[..at], theme.text));
        parts.push((&rest[at..arrow], theme.muted));
        let after = &rest[arrow..];
        let name = after.trim_start();
        let end = name.find(',').unwrap_or(name.len());
        parts.push((&after[..after.len() - name.len()], theme.text));
        parts.push((&name[..end], theme.text));
        parts.push((&name[end..], theme.text));
    }
    parts
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(parts: &[(String, Style)]) -> String {
        parts.iter().map(|(t, _)| t.as_str()).collect()
    }

    #[test]
    fn the_stepper_falls_back_to_the_current_step_alone() {
        let theme = Theme::mono();
        assert_eq!(
            text(&stepper(&theme, Screen::Candidates, 10)),
            "● Candidates 3/6"
        );
        assert_eq!(
            text(&stepper(&theme, Screen::Dashboard, 30)),
            "● Dashboard ── ○ Projects ›"
        );
        assert_eq!(
            text(&stepper(&theme, Screen::Result, 30)),
            "‹ ✓ Confirm ── ● Result"
        );
    }
}

//! The one progress bar.
//!
//! A row of discrete squared cells, the filled ones running through a colour
//! ramp from the theme and the unfilled ones muted, with the percentage as a
//! number right after them. The result screen, the confirm hold, the running
//! screen and the dashboard's disk gauge all draw through here, so there is one
//! look and one rule for how much of it is filled.
//!
//! No screen builds a fill of its own. The glyphs and the colours are the
//! theme's, so a terminal that cannot draw the cells gets `[###---]` from the
//! same switch that picks the colours.

use ratatui::style::Style;

use super::palette::{Ramp, Theme};

/// The most cells a bar is ever drawn with. Past this a bar stops being
/// glanced at and starts being read cell by cell.
pub const MAX_CELLS: usize = 40;

/// The fewest cells that still read as a bar. Narrower than this the cells are
/// left out and the percentage stands alone, rather than a stump.
pub const MIN_CELLS: usize = 4;

/// Columns the percentage takes, always: a space, three digits and the sign.
/// Fixed, so the bar does not shift as the figure goes from 9 to 10 to 100.
const LABEL: usize = 5;

/// How many cells are filled out of how many, and the percentage they say.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bar {
    pub cells: usize,
    pub filled: usize,
    pub percent: u8,
}

impl Bar {
    /// `done` out of `total`, in `cells` cells.
    ///
    /// The percentage is rounded down, as a share of work should be, and the
    /// cells follow it: anything above nothing shows a cell, and anything
    /// short of everything leaves one empty, so a full bar always says 100.
    pub fn of(done: u64, total: u64, cells: usize) -> Self {
        let done = done.min(total);
        let percent = (done as u128 * 100).checked_div(total as u128).unwrap_or(0) as u8;
        let filled = match percent {
            0 => 0,
            100 => cells,
            p => (cells * p as usize)
                .div_ceil(100)
                .clamp(1, cells.saturating_sub(1).max(1)),
        };
        Self {
            cells,
            filled,
            percent,
        }
    }
}

/// A bar of `done` out of `total` that fits in `width` columns, with its
/// percentage, as runs of text and the style each is drawn in.
///
/// Wider than [`MAX_CELLS`] allows, the bar stops growing; narrower than
/// [`MIN_CELLS`] allows, only the percentage is drawn.
pub fn line(
    theme: &Theme,
    ramp: Ramp,
    done: u64,
    total: u64,
    width: usize,
) -> Vec<(String, Style)> {
    let glyphs = theme.cells();
    let frame = if glyphs.bracketed { 2 } else { 0 };
    let cells = width.saturating_sub(LABEL + frame).min(MAX_CELLS);
    if cells < MIN_CELLS {
        let percent = Bar::of(done, total, 1).percent;
        return vec![(format!(" {percent:>3}%"), theme.text)];
    }
    let bar = Bar::of(done, total, cells);
    let mut parts = Vec::with_capacity(cells + 3);
    if glyphs.bracketed {
        parts.push(("[".to_string(), theme.muted));
    }
    for i in 0..bar.filled {
        parts.push((glyphs.full.to_string(), theme.ramp(ramp, i, cells)));
    }
    parts.push((
        glyphs.empty.to_string().repeat(cells - bar.filled),
        theme.muted,
    ));
    if glyphs.bracketed {
        parts.push(("]".to_string(), theme.muted));
    }
    parts.push((format!(" {:>3}%", bar.percent), theme.text));
    parts
}

/// [`line`] for a share that is a guess: the percentage carries a `~`, so the
/// bar says it is an estimate in the same place it says how much.
pub fn estimated(theme: &Theme, ramp: Ramp, fraction: f64, width: usize) -> Vec<(String, Style)> {
    let mut parts = line(theme, ramp, (fraction * 1000.0) as u64, 1000, width);
    if let Some((label, _)) = parts.last_mut() {
        *label = format!("{:>5}", format!("~{}", label.trim()));
    }
    parts
}

/// A bar that moves without measuring: a short run of cells travelling back and
/// forth, no percentage, the label's columns left empty so the line below it
/// does not shift when the scan switches to one that has a number.
///
/// A pure function of `ms`, the time since the scan began, so a frame is the
/// same whenever it is drawn and a test can ask for any of them.
pub fn sweep(theme: &Theme, width: usize, ms: u64) -> Vec<(String, Style)> {
    let glyphs = theme.cells();
    let frame = if glyphs.bracketed { 2 } else { 0 };
    let cells = width.saturating_sub(LABEL + frame).min(MAX_CELLS);
    if cells < MIN_CELLS {
        return vec![(" ...".to_string(), theme.muted)];
    }
    let run = (cells / 4).clamp(2, 6);
    let travel = cells - run;
    // Out and back: 0, 1, .. travel, travel - 1, .. 1.
    let step = (ms / 100) as usize % (2 * travel).max(1);
    let at = if step <= travel {
        step
    } else {
        2 * travel - step
    };

    let mut parts = Vec::with_capacity(run + 4);
    if glyphs.bracketed {
        parts.push(("[".to_string(), theme.muted));
    }
    parts.push((glyphs.empty.to_string().repeat(at), theme.muted));
    for i in 0..run {
        parts.push((glyphs.full.to_string(), theme.ramp(Ramp::Measure, i, run)));
    }
    parts.push((
        glyphs.empty.to_string().repeat(cells - at - run),
        theme.muted,
    ));
    if glyphs.bracketed {
        parts.push(("]".to_string(), theme.muted));
    }
    parts.push(("     ".to_string(), theme.text));
    parts
}

/// The columns [`line`] draws in `width`: what a caller lays the next thing out
/// after.
pub fn drawn_width(theme: &Theme, width: usize) -> usize {
    let frame = if theme.cells().bracketed { 2 } else { 0 };
    let cells = width.saturating_sub(LABEL + frame).min(MAX_CELLS);
    if cells < MIN_CELLS {
        LABEL
    } else {
        cells + frame + LABEL
    }
}

/// The three parts of the disk gauge: what can be given back, what else is in
/// use, and what is free. Reclaimable sits inside the used portion, never
/// beside the free one, so the order is the order the parts are meant to be
/// read in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Part {
    Reclaimable,
    InUse,
    Free,
}

/// The glyph and the style of one part of the disk gauge. The cells and the
/// legend swatch both come from here, so a legend colour is the colour of the
/// cells it names.
pub fn part(theme: &Theme, part: Part) -> (char, Style) {
    let glyphs = theme.cells();
    match part {
        Part::Reclaimable => (glyphs.mark, theme.safe),
        Part::InUse => (glyphs.full, theme.accent),
        Part::Free => (glyphs.empty, theme.muted),
    }
}

/// The disk gauge: `reclaimable`, `in_use` and `free` cells, in that order.
pub fn stacked(theme: &Theme, counts: [usize; 3]) -> Vec<(String, Style)> {
    let mut parts = Vec::new();
    if theme.cells().bracketed {
        parts.push(("[".to_string(), theme.muted));
    }
    for (kind, n) in [Part::Reclaimable, Part::InUse, Part::Free]
        .into_iter()
        .zip(counts)
    {
        let (glyph, style) = part(theme, kind);
        if n > 0 {
            parts.push((glyph.to_string().repeat(n), style));
        }
    }
    if theme.cells().bracketed {
        parts.push(("]".to_string(), theme.muted));
    }
    parts
}

/// How many cells the disk gauge has in `width` columns.
pub fn stacked_cells(theme: &Theme, width: usize) -> usize {
    let frame = if theme.cells().bracketed { 2 } else { 0 };
    width.saturating_sub(frame).min(MAX_DISK_CELLS)
}

/// The disk gauge is wider than a progress bar: it is the first thing on the
/// opening screen, and its cells are a share of a disk, not a count of steps.
const MAX_DISK_CELLS: usize = 64;

/// Share `bytes` out between `cells` cells: `[reclaimable, other, free]` in,
/// the cells each part gets out.
///
/// The cells always add up to `cells`, and a part with bytes in it never gets
/// none: 5 GB of a 460 GB disk is nine tenths of a cell, and a part that rounds
/// away to nothing reports "none" for the one quantity the gauge exists to
/// show. What the minimum takes, and what rounding leaves over, comes off and
/// goes onto the largest part, where one cell is least noticed.
pub fn split(cells: usize, bytes: [u64; 3]) -> [usize; 3] {
    let total: u128 = bytes.iter().map(|b| *b as u128).sum();
    if total == 0 || cells == 0 {
        return [0; 3];
    }
    let mut parts = bytes.map(|b| match b {
        0 => 0,
        b => ((b as u128 * cells as u128 / total) as usize).max(1),
    });
    let largest = (0..3).max_by_key(|&i| bytes[i]).unwrap_or(0);
    let drawn: usize = parts.iter().sum();
    if drawn > cells {
        parts[largest] = parts[largest].saturating_sub(drawn - cells).max(1);
    } else {
        parts[largest] += cells - drawn;
    }
    parts
}

/// One quantity against the largest of its kind, as cells with no label: the
/// mini bar of a list row. Filled cells run through `ramp`, the rest are muted,
/// and anything above nothing shows at least one cell.
pub fn share(
    theme: &Theme,
    ramp: Ramp,
    part: u64,
    whole: u64,
    cells: usize,
) -> Vec<(String, Style)> {
    let glyphs = theme.cells();
    if cells == 0 {
        return Vec::new();
    }
    let filled = if part == 0 || whole == 0 {
        0
    } else {
        ((part as u128 * cells as u128).div_ceil(whole as u128) as usize).clamp(1, cells)
    };
    let mut parts: Vec<(String, Style)> = (0..filled)
        .map(|i| (glyphs.full.to_string(), theme.ramp(ramp, i, cells)))
        .collect();
    parts.push((glyphs.empty.to_string().repeat(cells - filled), theme.muted));
    parts
}

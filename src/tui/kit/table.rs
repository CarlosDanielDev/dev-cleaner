//! Columns declared once.

use ratatui::buffer::Buffer;
use ratatui::style::Style;

/// Columns between one cell and the next.
pub(in crate::tui) const GAP: u16 = 2;

/// Which edge a cell's text keeps to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::tui) enum Align {
    /// Text starts where the one above it does.
    Left,
    /// A figure ends where the one above it does, so the digits line up and a
    /// longer number is visibly a bigger one.
    Right,
}

/// A column: what it is, how wide it wants to be (gap included) and which edge
/// its text keeps to.
#[derive(Debug, Clone, Copy)]
pub(in crate::tui) struct Col<K> {
    pub key: K,
    pub width: u16,
    /// How narrow it may be squeezed before it is dropped. Its width, for a
    /// column whose cells cannot be cut.
    pub min: u16,
    pub align: Align,
    /// Whether the column takes the room the others leave. At most one: it is
    /// the column whose cut loses what tells two rows apart.
    pub flex: bool,
}

impl<K> Col<K> {
    pub(in crate::tui) fn fixed(key: K, width: u16, align: Align) -> Self {
        Self {
            key,
            width,
            min: width,
            align,
            flex: false,
        }
    }

    /// A column that gives up room, down to `min`, before it is dropped: the
    /// cells of a command can be cut with a mark and still say what it is.
    pub(in crate::tui) fn squeezable(key: K, width: u16, min: u16, align: Align) -> Self {
        Self {
            key,
            width,
            min: min.min(width),
            align,
            flex: false,
        }
    }

    pub(in crate::tui) fn flex(key: K, min: u16) -> Self {
        Self {
            key,
            width: min,
            min,
            align: Align::Left,
            flex: true,
        }
    }
}

/// Which columns fit, and where they start.
#[derive(Debug)]
pub(in crate::tui) struct Fit<K> {
    /// Each column drawn, with the x it starts at.
    pub drawn: Vec<(K, u16)>,
    /// The ones that did not fit, most important first.
    pub hidden: Vec<K>,
    /// How wide the flexible column ended up, gap included.
    pub flex_w: u16,
    /// How wide each drawn column ended up, gap included.
    widths: Vec<(K, u16)>,
}

impl<K: Copy + PartialEq> Fit<K> {
    pub(in crate::tui) fn has(&self, key: K) -> bool {
        self.drawn.iter().any(|(k, _)| *k == key)
    }

    pub(in crate::tui) fn x_of(&self, key: K) -> Option<u16> {
        self.drawn.iter().find(|(k, _)| *k == key).map(|(_, x)| *x)
    }

    /// The room for text in column `key`: its width as drawn, less the gap.
    pub(in crate::tui) fn room(&self, key: K) -> usize {
        self.widths
            .iter()
            .find(|(k, _)| *k == key)
            .map_or(0, |(_, w)| w.saturating_sub(GAP) as usize)
    }
}

/// A table's columns, in the order they are drawn, and the order they are kept
/// in when the area is narrower than all of them.
#[derive(Debug)]
pub(in crate::tui) struct Table<K> {
    cols: Vec<Col<K>>,
    /// Most important first.
    priority: Vec<K>,
}

impl<K: Copy + PartialEq> Table<K> {
    pub(in crate::tui) fn new(cols: Vec<Col<K>>, priority: Vec<K>) -> Self {
        Self { cols, priority }
    }

    fn col(&self, key: K) -> Option<&Col<K>> {
        self.cols.iter().find(|c| c.key == key)
    }

    /// The columns that fit in `width`, each with the x it starts at, and the
    /// ones that did not.
    ///
    /// Columns are dropped whole, least important first, the way the key bar
    /// drops entries: a header cut mid-word reads as a column that was never
    /// there, and a cell run into its neighbour reads as a number nobody
    /// measured. What is left over goes to the flexible column, up to `wanted`.
    pub(in crate::tui) fn fit(&self, width: u16, wanted: u16) -> Fit<K> {
        let mut kept: Vec<Col<K>> = self.cols.clone();
        // Squeezed as far as they go before any column is dropped.
        let narrowest = |kept: &[Col<K>]| kept.iter().map(|c| c.min).sum::<u16>();
        for key in self.priority.iter().rev() {
            if narrowest(&kept) <= width {
                break;
            }
            kept.retain(|c| c.key != *key);
        }
        // Then shrunk, the least important first, only as far as it takes.
        let mut over = kept
            .iter()
            .map(|c| c.width)
            .sum::<u16>()
            .saturating_sub(width);
        for key in self.priority.iter().rev() {
            if let Some(c) = kept.iter_mut().find(|c| c.key == *key && !c.flex) {
                let give = over.min(c.width - c.min);
                c.width -= give;
                over -= give;
            }
        }
        let used: u16 = kept.iter().map(|c| c.width).sum();
        let min = self.cols.iter().find(|c| c.flex).map_or(0, |c| c.width);
        let flex_w = min.max(wanted.min(min + width.saturating_sub(used)));
        let hidden = self
            .priority
            .iter()
            .copied()
            .filter(|k| self.col(*k).is_some() && !kept.iter().any(|c| c.key == *k))
            .collect();
        let mut x = 0;
        let mut widths = Vec::new();
        let drawn = kept
            .into_iter()
            .map(|c| {
                let at = x;
                let w = if c.flex { flex_w } else { c.width };
                x += w;
                widths.push((c.key, w));
                (c.key, at)
            })
            .collect();
        Fit {
            drawn,
            hidden,
            flex_w,
            widths,
        }
    }

    /// `text` laid in its column, for the column's alignment. Two gap columns
    /// follow each cell.
    pub(in crate::tui) fn aligned(&self, key: K, text: &str) -> String {
        let Some(col) = self.col(key) else {
            return text.to_string();
        };
        let width = col.width.saturating_sub(GAP) as usize;
        match col.align {
            Align::Left => text.to_string(),
            Align::Right => format!("{text:>width$}"),
        }
    }

    /// Draw `text` in column `key` from `(x, y)`.
    pub(in crate::tui) fn put(
        &self,
        buf: &mut Buffer,
        (x, y): (u16, u16),
        key: K,
        text: &str,
        style: Style,
    ) {
        buf.set_string(x, y, self.aligned(key, text), style);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum K {
        A,
        B,
        C,
        D,
    }

    /// `A` and `D` cannot be cut, `B` can go down to 8, `C` takes what is left.
    fn table() -> Table<K> {
        Table::new(
            vec![
                Col::fixed(K::A, 10, Align::Right),
                Col::squeezable(K::B, 20, 8, Align::Left),
                Col::flex(K::C, 10),
                Col::fixed(K::D, 6, Align::Left),
            ],
            vec![K::C, K::A, K::B, K::D],
        )
    }

    #[test]
    fn every_column_is_drawn_where_there_is_room_and_the_flexible_one_takes_the_rest() {
        let fit = table().fit(100, 30);
        assert_eq!(fit.hidden, vec![]);
        assert_eq!(fit.flex_w, 30);
        assert_eq!(
            fit.drawn,
            vec![(K::A, 0), (K::B, 10), (K::C, 30), (K::D, 60)]
        );
    }

    #[test]
    fn a_squeezable_column_gives_room_before_any_column_is_dropped() {
        let fit = table().fit(40, 30);
        assert_eq!(fit.hidden, vec![], "6 short: B gives 6 of its 12 spare");
        assert_eq!(fit.room(K::B), 12, "14 wide less its gap");
        assert_eq!(fit.room(K::A), 8, "what cannot be cut is not");
    }

    #[test]
    fn a_column_goes_whole_and_the_least_important_goes_first() {
        let fit = table().fit(30, 30);
        assert_eq!(fit.hidden, vec![K::D]);
        assert!(!fit.has(K::D));
        let fit = table().fit(20, 30);
        assert_eq!(fit.hidden, vec![K::B, K::D]);
    }

    #[test]
    fn a_figure_ends_where_the_one_above_it_does() {
        let t = table();
        assert_eq!(t.aligned(K::A, "1.00 GB"), " 1.00 GB");
        assert_eq!(t.aligned(K::A, "512.00 MB"), "512.00 MB");
    }

    #[test]
    fn text_starts_where_the_one_above_it_does() {
        assert_eq!(table().aligned(K::D, "word"), "word");
    }
}

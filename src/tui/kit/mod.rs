//! The design system of the table screens.
//!
//! Projects, Candidates and the plan are the same kind of screen: a view bar, a
//! header row, aligned columns, a mark where something can be marked, a cursor
//! band, the selected row in full and a position line. They drifted when each
//! owned its own column arithmetic, and every screen the owner reviewed needed
//! its own round of fixes. The pieces live here once, and the screens say only
//! which columns they have and what each cell holds:
//!
//! - [`Table`]: columns declared once, dropped whole under their minimum width,
//!   numbers right-aligned and text left-aligned.
//! - [`section`], [`view_bar`], [`detail`], [`position`], [`empty_body`], [`band`]:
//!   the lines around the rows.
//! - [`Where`] and [`Locator`]: what a path is called, by its project, so the
//!   absolute path is only ever drawn where a row is selected.
//! - [`kind_badge`], [`tier_badge`], [`checkbox`], [`mark_glyph`]: one place that
//!   maps a concept to a glyph and a role.

mod badges;
mod lines;
mod locate;
mod table;

pub(super) use badges::{checkbox, held_badge, kind_badge, mark_glyph, tier_badge};
pub(super) use lines::{
    KEYS_LEAD, band, detail, detail_line, empty_body, facts, position, put_detail, section,
    view_bar,
};
pub(super) use locate::Where;
pub use locate::{Located, Locator};
pub(super) use table::{Align, Col, GAP, Table};

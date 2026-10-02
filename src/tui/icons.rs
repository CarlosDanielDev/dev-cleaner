//! One glyph per concept, in one table.
//!
//! A screen asks for `Icon::Node`, never for a character. Three sets exist and
//! one is chosen at startup, with the colours: single-width Unicode shapes by
//! default, a Nerd Font set when the user opts in with `DEV_CLEANER_ICONS=nerd`,
//! and ASCII where the terminal cannot be trusted with either (`TERM=linux` or
//! `dumb`). `NO_COLOR` is not a reason to change the set: an icon is a shape,
//! not a colour.
//!
//! No emoji, ever: they are two columns wide in most terminals and break every
//! column calculation on the screen. And no icon stands alone; the screens
//! always put a word beside it, so a missing glyph costs decoration only.

use crate::classify::Ecosystem;

/// Which glyphs the icons are drawn in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum IconSet {
    #[default]
    Unicode,
    Nerd,
    Ascii,
}

impl IconSet {
    /// The set for what the user asked for (`DEV_CLEANER_ICONS`) on `term`.
    ///
    /// A console that cannot draw the squared cells cannot draw a Nerd Font
    /// either, so the terminal's fallback wins over the request.
    pub fn choose(requested: Option<&str>, term: Option<&str>) -> Self {
        if matches!(term, Some("linux" | "dumb")) {
            return Self::Ascii;
        }
        match requested {
            Some("nerd") => Self::Nerd,
            Some("ascii") => Self::Ascii,
            _ => Self::Unicode,
        }
    }
}

/// A concept the interface draws an icon for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Disk,
    Projects,
    Rebuild,
    Entries,
    Measured,
    Time,
    Node,
    Rust,
    Python,
    Swift,
    Ruby,
    Go,
    Java,
    Php,
    Embedded,
    BiggestWin,
    GoneQuiet,
    HeldBack,
    Trend,
    MostFiles,
}

impl Icon {
    /// Every icon, so a test can walk the table.
    pub const ALL: [Icon; 20] = [
        Icon::Disk,
        Icon::Projects,
        Icon::Rebuild,
        Icon::Entries,
        Icon::Measured,
        Icon::Time,
        Icon::Node,
        Icon::Rust,
        Icon::Python,
        Icon::Swift,
        Icon::Ruby,
        Icon::Go,
        Icon::Java,
        Icon::Php,
        Icon::Embedded,
        Icon::BiggestWin,
        Icon::GoneQuiet,
        Icon::HeldBack,
        Icon::Trend,
        Icon::MostFiles,
    ];

    /// The icon of a toolchain.
    pub fn of(ecosystem: Ecosystem) -> Self {
        match ecosystem {
            Ecosystem::Node => Icon::Node,
            Ecosystem::Rust => Icon::Rust,
            Ecosystem::Python => Icon::Python,
            Ecosystem::Swift => Icon::Swift,
            Ecosystem::Ruby => Icon::Ruby,
            Ecosystem::Go => Icon::Go,
            Ecosystem::Java => Icon::Java,
            Ecosystem::Php => Icon::Php,
            Ecosystem::Embedded => Icon::Embedded,
        }
    }

    /// The glyph of this icon in `set`: unicode, nerd, ascii.
    pub fn glyph(self, set: IconSet) -> char {
        let (unicode, nerd, ascii) = match self {
            Icon::Disk => ('◉', '\u{f0a0}', '@'),
            Icon::Projects => ('▣', '\u{f07b}', '#'),
            Icon::Rebuild => ('⬡', '\u{f021}', '+'),
            Icon::Entries => ('◇', '\u{f15b}', '.'),
            Icon::Measured => ('◫', '\u{f1b2}', '%'),
            Icon::Time => ('◷', '\u{f017}', '~'),
            Icon::Node => ('⬢', '\u{e718}', 'n'),
            Icon::Rust => ('◆', '\u{e7a8}', 'r'),
            Icon::Python => ('◈', '\u{e73c}', 'p'),
            Icon::Swift => ('◭', '\u{e755}', 's'),
            Icon::Ruby => ('◊', '\u{e791}', 'b'),
            Icon::Go => ('◐', '\u{e626}', 'g'),
            Icon::Java => ('◒', '\u{e738}', 'j'),
            Icon::Php => ('◓', '\u{e73d}', '$'),
            Icon::Embedded => ('▦', '\u{f2db}', 'e'),
            Icon::BiggestWin => ('●', '\u{f005}', '*'),
            Icon::GoneQuiet => ('◌', '\u{f186}', 'z'),
            Icon::HeldBack => ('⊘', '\u{f023}', '!'),
            Icon::Trend => ('↕', '\u{f201}', '^'),
            Icon::MostFiles => ('▤', '\u{f0c5}', '='),
        };
        match set {
            IconSet::Unicode => unicode,
            IconSet::Nerd => nerd,
            IconSet::Ascii => ascii,
        }
    }
}

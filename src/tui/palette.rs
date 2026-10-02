//! Every style the interface draws with, named for what it means.
//!
//! Screens never name a colour, only a role: a size is always drawn in a size
//! style, a key in the key style, and what a role looks like is this module's
//! business. Three looks exist, chosen once at startup by [`Theme::detect`] and
//! handed down to every draw, so a test picks a look by constructing one:
//!
//! - [`Theme::neon`], on a truecolor terminal (`COLORTERM=truecolor|24bit`):
//!   a near-black indigo ground that we paint ourselves, and neon on it. Neon
//!   only reads against a dark ground, and only a truecolor terminal can be
//!   trusted to show the exact shade, so this is the one look that owns its
//!   background.
//! - [`Theme::ansi`], on anything else that has colour: the named ANSI colours,
//!   which the user's own profile defines, over the profile's own background.
//!   A light profile and a dark one each map them to something that reads, and
//!   bright black is left out because many profiles paint it within a shade of
//!   the background. This is the look the interface had before the theme.
//! - [`Theme::mono`], when `NO_COLOR` is set: no colour escape at all. Every
//!   meaning is carried by a glyph, a word or a weight (bold, dim, reverse).
//!
//! No colour is the only carrier of a meaning. Every role below is paired, on
//! the screen that uses it, with a glyph, a word or a weight that says the same
//! thing, and red and green are never the only difference between two states.
//!
//! ponytail: fixed to these three looks. A user theme file is its own feature.

use super::icons::{Icon, IconSet};
use ratatui::style::{Color, Modifier, Style};

/// Bytes at which a size stops being small and starts to be worth a look.
pub const SIZE_WARM: u64 = 100 * 1024 * 1024;

/// Bytes at which a size is the loud thing on the screen.
pub const SIZE_HOT: u64 = 1024 * 1024 * 1024;

/// The ground the neon look paints, as RGB, so a test can measure against it.
pub const GROUND_RGB: (u8, u8, u8) = (0x0b, 0x0e, 0x1a);

const GROUND: Color = Color::Rgb(0x0b, 0x0e, 0x1a);
const NEON_CYAN: Color = Color::Rgb(0x00, 0xe5, 0xff);
const NEON_MAGENTA: Color = Color::Rgb(0xff, 0x2e, 0x97);
const NEON_PINK: Color = Color::Rgb(0xff, 0x4f, 0xd8);
const NEON_GREEN: Color = Color::Rgb(0x39, 0xff, 0x14);
const NEON_AMBER: Color = Color::Rgb(0xff, 0xb0, 0x00);
const NEON_ORANGE: Color = Color::Rgb(0xff, 0x8c, 0x42);
const NEON_RED: Color = Color::Rgb(0xff, 0x38, 0x60);
const NEON_VIOLET: Color = Color::Rgb(0xb4, 0x8c, 0xff);
const TEXT: Color = Color::Rgb(0xc8, 0xd3, 0xf5);
const MUTED_INK: Color = Color::Rgb(0x8a, 0x98, 0xc4);

/// Where the wordmark changes ink under ANSI colours: after `dev`, the third of
/// the ten letters, and before `cleaner`.
const BRAND_SEAM: f32 = 0.3;

/// The glyphs a bar is drawn in, chosen once with the colours.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cells {
    /// A cell that is filled: progress made, or space that is taken.
    pub full: char,
    /// A cell of the part a gauge is about: what can be reclaimed.
    pub mark: char,
    /// A cell still to go, or space that is free.
    pub empty: char,
    /// Whether the bar is closed in brackets, as `[###---]` is.
    pub bracketed: bool,
}

impl Cells {
    /// Squared, same height, same width: `▰▰▰▱▱▱`. The marked cell is the
    /// narrow upright one, so the part of a gauge the screen is about stands
    /// out of the rest with no colour at all.
    const BLOCKS: Self = Self {
        full: '▰',
        mark: '▮',
        empty: '▱',
        bracketed: false,
    };
    /// For a console whose font has no geometric shapes: `[###---]`.
    const ASCII: Self = Self {
        full: '#',
        mark: '*',
        empty: '-',
        bracketed: true,
    };
}

/// Which colours the filled cells of a bar run through, left to right.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ramp {
    /// Cyan to magenta: something being measured, or counted up.
    Measure,
    /// Amber to red: the one bar that is a step towards removing something.
    Danger,
}

/// How much colour the terminal was asked to carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// 24-bit colour: the interface paints its own ground.
    Truecolor,
    /// The named ANSI colours over the profile's own background.
    Ansi,
    /// No colour at all.
    Mono,
}

/// Every role the interface draws in.
#[derive(Debug, Clone, Copy)]
pub struct Theme {
    mode: Mode,
    /// Painted under the whole frame before anything is drawn. Empty unless
    /// the look owns its background.
    pub ground: Style,
    /// Ordinary text, and every fact: paths, counts, sentences.
    pub text: Style,
    /// Labels, hints and glue words that can be skipped. Never a fact.
    pub muted: Style,
    /// Titles, section headings and the sorted column.
    pub head: Style,
    /// Gauges that measure, the name a way forward leads to.
    pub accent: Style,
    /// A key cap: every key in the key bar, the key list and the way row.
    pub key: Style,
    /// How a path comes back: the promise that makes it safe to remove.
    pub safe: Style,
    /// Held back: a guard refused it, or a hold stopped short.
    pub blocked: Style,
    /// The gauge and the band of the one screen that removes anything.
    pub danger: Style,
    /// Secondary structure: separators, arrows, tier names.
    pub violet: Style,
    /// The row under the cursor, whatever its text was drawn in.
    pub selected: Style,
    /// The title band of the one screen that removes anything.
    pub warning_band: Style,
    /// The verdict of a run that moved everything.
    pub verdict_safe: Style,
    /// The verdict of a run that failed or stopped.
    pub verdict_blocked: Style,
    sizes: [Style; 3],
    cells: Cells,
    icons: IconSet,
}

/// Ground ink on a neon fill, bold: a key cap, the cursor, the warning band.
const fn on_ground(bg: Color) -> Style {
    Style::new().fg(GROUND).bg(bg).add_modifier(Modifier::BOLD)
}

impl Theme {
    /// The neon look, for a truecolor terminal.
    pub const fn neon() -> Self {
        Self {
            mode: Mode::Truecolor,
            ground: Style::new().fg(TEXT).bg(GROUND),
            text: Style::new().fg(TEXT),
            muted: Style::new().fg(MUTED_INK),
            head: Style::new().fg(NEON_MAGENTA).add_modifier(Modifier::BOLD),
            accent: Style::new().fg(NEON_CYAN),
            key: on_ground(NEON_MAGENTA),
            safe: Style::new().fg(NEON_GREEN),
            blocked: Style::new().fg(NEON_AMBER),
            danger: Style::new().fg(NEON_RED).add_modifier(Modifier::BOLD),
            violet: Style::new().fg(NEON_VIOLET),
            selected: on_ground(NEON_CYAN),
            warning_band: on_ground(NEON_RED),
            verdict_safe: Style::new().fg(NEON_GREEN).add_modifier(Modifier::BOLD),
            verdict_blocked: Style::new().fg(NEON_AMBER).add_modifier(Modifier::BOLD),
            sizes: [
                Style::new().fg(NEON_CYAN),
                Style::new().fg(NEON_ORANGE),
                Style::new().fg(NEON_PINK).add_modifier(Modifier::BOLD),
            ],
            cells: Cells::BLOCKS,
            icons: IconSet::Unicode,
        }
    }

    /// The named ANSI colours, over the profile's own background.
    pub const fn ansi() -> Self {
        let bold = Modifier::BOLD;
        Self {
            mode: Mode::Ansi,
            ground: Style::new(),
            text: Style::new(),
            // Dim over the profile's foreground rather than a grey of our
            // choosing, so it stays the profile's contrast, halved.
            muted: Style::new().add_modifier(Modifier::DIM),
            head: Style::new().fg(Color::Magenta).add_modifier(bold),
            accent: Style::new().fg(Color::Cyan),
            key: Style::new().fg(Color::Magenta).add_modifier(bold),
            safe: Style::new().fg(Color::Green),
            blocked: Style::new().fg(Color::Yellow),
            // A shape and a weight, never a sentence: ANSI red is dark enough
            // in common dark profiles that words drawn in it stop reading.
            danger: Style::new().fg(Color::Red).add_modifier(bold),
            violet: Style::new().fg(Color::Magenta),
            selected: Style::new().add_modifier(Modifier::REVERSED),
            warning_band: Style::new().add_modifier(bold.union(Modifier::REVERSED)),
            verdict_safe: Style::new().fg(Color::Green).add_modifier(bold),
            verdict_blocked: Style::new().fg(Color::Yellow).add_modifier(bold),
            sizes: [
                Style::new().fg(Color::Cyan),
                Style::new().add_modifier(bold),
                Style::new().fg(Color::Magenta).add_modifier(bold),
            ],
            cells: Cells::BLOCKS,
            icons: IconSet::Unicode,
        }
    }

    /// No colour at all: weight, glyphs and words only.
    pub const fn mono() -> Self {
        let bold = Modifier::BOLD;
        Self {
            mode: Mode::Mono,
            ground: Style::new(),
            text: Style::new(),
            muted: Style::new().add_modifier(Modifier::DIM),
            head: Style::new().add_modifier(bold),
            accent: Style::new(),
            key: Style::new().add_modifier(bold),
            safe: Style::new(),
            blocked: Style::new(),
            danger: Style::new().add_modifier(bold),
            violet: Style::new(),
            selected: Style::new().add_modifier(Modifier::REVERSED),
            warning_band: Style::new().add_modifier(bold.union(Modifier::REVERSED)),
            verdict_safe: Style::new().add_modifier(bold),
            verdict_blocked: Style::new().add_modifier(bold),
            sizes: [Style::new(), Style::new(), Style::new().add_modifier(bold)],
            cells: Cells::BLOCKS,
            icons: IconSet::Unicode,
        }
    }

    /// The look for a terminal that said `no_color` and `colorterm`.
    ///
    /// `NO_COLOR` wins over `COLORTERM`, and counts only when it is not empty,
    /// as no-color.org has it. 256 and 16 colours both take the ANSI look: the
    /// named colours are the nearest either has to the neon ones.
    pub fn choose(no_color: Option<&str>, colorterm: Option<&str>) -> Self {
        if no_color.is_some_and(|v| !v.is_empty()) {
            return Self::mono();
        }
        match colorterm {
            Some("truecolor" | "24bit") => Self::neon(),
            _ => Self::ansi(),
        }
    }

    /// [`Theme::choose`] on this process's environment. Read once, at startup.
    pub fn detect() -> Self {
        Self::choose(
            std::env::var("NO_COLOR").ok().as_deref(),
            std::env::var("COLORTERM").ok().as_deref(),
        )
        .for_term(std::env::var("TERM").ok().as_deref())
        .icons_from(
            std::env::var("DEV_CLEANER_ICONS").ok().as_deref(),
            std::env::var("TERM").ok().as_deref(),
        )
    }

    /// The icon set the user asked for, unless the terminal rules it out.
    pub fn icons_from(self, requested: Option<&str>, term: Option<&str>) -> Self {
        self.with_icons(IconSet::choose(requested, term))
    }

    /// The same look, with icons drawn from `set`.
    pub const fn with_icons(mut self, set: IconSet) -> Self {
        self.icons = set;
        self
    }

    /// The glyph for `icon` in the chosen set.
    pub fn icon(&self, icon: Icon) -> char {
        icon.glyph(self.icons)
    }

    /// Draw bars in ASCII where the terminal is one whose font cannot be
    /// trusted with the squared cells: the Linux console and a dumb terminal.
    pub fn for_term(self, term: Option<&str>) -> Self {
        match term {
            Some("linux" | "dumb") => self.ascii(),
            _ => self,
        }
    }

    /// The same look, with bars drawn as `[###---]`.
    pub const fn ascii(mut self) -> Self {
        self.cells = Cells::ASCII;
        self.icons = IconSet::Ascii;
        self
    }

    /// The glyphs bars are drawn in.
    pub fn cells(&self) -> Cells {
        self.cells
    }

    /// The style of filled cell number `at` of `of`, so the colour of a cell
    /// belongs to its place in the bar and not to how much of the bar is full.
    pub fn ramp(&self, ramp: Ramp, at: usize, of: usize) -> Style {
        let t = if of <= 1 {
            0.0
        } else {
            at as f32 / (of - 1) as f32
        };
        match self.mode {
            Mode::Truecolor => {
                let (from, to) = match ramp {
                    Ramp::Measure => ((0x00, 0xe5, 0xff), (0xff, 0x2e, 0x97)),
                    Ramp::Danger => ((0xff, 0xb0, 0x00), (0xff, 0x38, 0x60)),
                };
                let mix = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
                Style::new().fg(Color::Rgb(
                    mix(from.0, to.0),
                    mix(from.1, to.1),
                    mix(from.2, to.2),
                ))
            }
            Mode::Ansi => {
                let (from, to) = match ramp {
                    Ramp::Measure => (Color::Cyan, Color::Magenta),
                    Ramp::Danger => (Color::Yellow, Color::Red),
                };
                Style::new().fg(if t < 0.5 { from } else { to })
            }
            Mode::Mono => Style::new().add_modifier(Modifier::BOLD),
        }
    }

    /// The style of a letter of the wordmark `t` of the way from its first
    /// letter (0.0) to its last (1.0): the logo's cyan, through violet, to its
    /// magenta under truecolor; cyan, then magenta from [`BRAND_SEAM`] on, under
    /// ANSI colours; and bold alone under none. Always bold: it is the brand.
    pub fn brand(&self, t: f32) -> Style {
        let bold = Modifier::BOLD;
        match self.mode {
            Mode::Truecolor => {
                let (from, to, t) = if t < 0.5 {
                    ((0x00, 0xe5, 0xff), (0xb4, 0x8c, 0xff), t * 2.0)
                } else {
                    ((0xb4, 0x8c, 0xff), (0xff, 0x2e, 0x97), t * 2.0 - 1.0)
                };
                let mix = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
                Style::new()
                    .fg(Color::Rgb(
                        mix(from.0, to.0),
                        mix(from.1, to.1),
                        mix(from.2, to.2),
                    ))
                    .add_modifier(bold)
            }
            Mode::Ansi => Style::new()
                .fg(if t < BRAND_SEAM {
                    Color::Cyan
                } else {
                    Color::Magenta
                })
                .add_modifier(bold),
            Mode::Mono => Style::new().add_modifier(bold),
        }
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// A size, drawn so the big thing is the loud thing.
    pub fn size(&self, bytes: u64) -> Style {
        match bytes {
            b if b >= SIZE_HOT => self.sizes[2],
            b if b >= SIZE_WARM => self.sizes[1],
            _ => self.sizes[0],
        }
    }

    /// The scan's progress line, wrapped in the escape that colours it.
    ///
    /// The line is a plain string on stdout rather than a frame, so it gets
    /// the one accent and a reset, with the `·` between its figures in the
    /// structure colour so the figures read as separate things, and nothing at
    /// all under `NO_COLOR`.
    pub fn progress_line(&self, line: &str) -> String {
        let (text, rule) = match self.mode {
            Mode::Truecolor => ("\x1b[38;2;0;229;255m", "\x1b[38;2;180;140;255m"),
            Mode::Ansi => ("\x1b[36m", "\x1b[35m"),
            Mode::Mono => return line.to_string(),
        };
        let line = line.replace('·', &format!("{rule}·{text}"));
        format!("{text}{line}\x1b[0m")
    }
}

impl Default for Theme {
    /// The look every test and every embedder gets unless it asks for another:
    /// the profile's own colours, as before the theme existed.
    fn default() -> Self {
        Self::ansi()
    }
}

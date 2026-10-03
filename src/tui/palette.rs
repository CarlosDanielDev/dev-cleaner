//! Every style the interface draws with, named for what it means.
//!
//! Screens never name a colour, only a role: a size is always drawn in a size
//! style, a key in the key style, and what a role looks like is this module's
//! business. A theme is a [`ThemeName`] (the registry below: one entry per
//! theme) in a [`Mode`] (how much colour the terminal carries), chosen at
//! startup by [`Theme::detect_named`], switched live by [`Theme::switched`], and
//! handed down to every draw, so a test picks a look by constructing one with
//! [`Theme::named`]. The neon theme comes in three looks:
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
//! The `matrix` theme (MS-DOS meets the movie: phosphor green on true black,
//! double-line rules, shaded block bars, bracketed key caps, a prompt for a
//! title) has the same three modes. Under `NO_COLOR` it keeps its glyphs and
//! loses its colours, so it still looks like itself and still carries every
//! meaning by shape.
//!
//! No colour is the only carrier of a meaning. Every role below is paired, on
//! the screen that uses it, with a glyph, a word or a weight that says the same
//! thing, and red and green are never the only difference between two states.
//!
//! ponytail: the registry is a const table. A user theme file is its own feature.

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
/// The danger band's fill and the ink on it: white on a deep red reads at 6.8:1,
/// where the ground ink on the neon red was a dark word on a loud colour.
const BAND_FILL: Color = Color::Rgb(0xb3, 0x12, 0x3a);
const BAND_INK: Color = Color::Rgb(0xff, 0xff, 0xff);
const TEXT: Color = Color::Rgb(0xc8, 0xd3, 0xf5);
const MUTED_INK: Color = Color::Rgb(0x8a, 0x98, 0xc4);

/// Where the wordmark changes ink under ANSI colours: after `dev`, the third of
/// the ten letters, and before `cleaner`.
const BRAND_SEAM: f32 = 0.3;

/// One theme of the registry: the name the code uses, the one the user types,
/// and a line that says what it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThemeEntry {
    pub name: ThemeName,
    pub id: &'static str,
    pub about: &'static str,
}

/// The themes there are.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ThemeName {
    /// Neon on a near-black indigo ground: the look the interface was born in.
    Neon,
    /// Phosphor green on true black, drawn like a 1990s DOS program that has
    /// seen the Matrix.
    Matrix,
}

impl ThemeName {
    /// Every theme, in the order `T` cycles through them. A third theme is one
    /// more entry here and one more arm in [`Theme::named`].
    pub const ALL: [ThemeEntry; 2] = [
        ThemeEntry {
            name: ThemeName::Neon,
            id: "neon",
            about: "neon on indigo: the look dev-cleaner was born in",
        },
        ThemeEntry {
            name: ThemeName::Matrix,
            id: "matrix",
            about: "MS-DOS meets the Matrix: phosphor green on black, double rules, digital rain",
        },
    ];

    /// What the user types for this theme.
    pub fn id(self) -> &'static str {
        Self::ALL
            .iter()
            .find(|e| e.name == self)
            .map_or("neon", |e| e.id)
    }

    /// The theme the user typed, if there is one. Exact: no case folding, so a
    /// typo is a typo everywhere.
    pub fn parse(id: &str) -> Option<Self> {
        Self::ALL.iter().find(|e| e.id == id).map(|e| e.name)
    }

    /// The theme after this one, round to the first.
    pub fn next(self) -> Self {
        let at = Self::ALL.iter().position(|e| e.name == self).unwrap_or(0);
        Self::ALL[(at + 1) % Self::ALL.len()].name
    }

    /// `neon, matrix`: what an error names when it says what was valid.
    pub fn ids() -> String {
        Self::ALL.map(|e| e.id).join(", ")
    }
}

// The matrix theme: a green phosphor and the colours a CRT had besides it.
const MX_GROUND: Color = Color::Rgb(0x00, 0x00, 0x00);
const MX_TEXT: Color = Color::Rgb(0x33, 0xff, 0x66);
const MX_QUIET: Color = Color::Rgb(0x1f, 0x9a, 0x3f);
const MX_MID: Color = Color::Rgb(0x23, 0xc8, 0x4f);
const MX_HEAD: Color = Color::Rgb(0xcc, 0xff, 0xdd);
const MX_ACCENT: Color = Color::Rgb(0x00, 0xff, 0x41);
/// Classic CRT amber: held back, which is never a shade of ok.
const MX_AMBER: Color = Color::Rgb(0xff, 0xb0, 0x00);
/// Hot red, only where danger is.
const MX_RED: Color = Color::Rgb(0xff, 0x40, 0x40);

/// Black on a fill, bold: the DOS highlight bar, a key cap, the danger band.
const fn on_black(bg: Color) -> Style {
    Style::new()
        .fg(MX_GROUND)
        .bg(bg)
        .add_modifier(Modifier::BOLD)
}

/// The shapes a theme draws with, which are the theme's as much as its colours.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Glyphs {
    /// The line a heading is drawn out with.
    rule: char,
    /// The ends of the rule that closes the header band, when it has any.
    rule_ends: Option<(char, char)>,
    /// What a key cap is wrapped in.
    caps: (&'static str, &'static str),
    /// A step done, the step here, a step to come; and the same on a console.
    steps: [&'static str; 3],
    steps_ascii: [&'static str; 3],
    /// Whether the screen's title is a DOS prompt with a cursor.
    prompt: bool,
    /// Whether digital rain falls beside a scan.
    rain: bool,
}

impl Glyphs {
    const NEON: Self = Self {
        rule: '─',
        rule_ends: None,
        caps: ("", ""),
        steps: ["✓ ", "● ", "○ "],
        steps_ascii: ["✓ ", "● ", "○ "],
        prompt: false,
        rain: false,
    };
    const MATRIX: Self = Self {
        rule: '═',
        rule_ends: Some(('╞', '╡')),
        caps: ("[", "]"),
        steps: ["[✓] ", "[●] ", "[ ] "],
        steps_ascii: ["[x] ", "[*] ", "[ ] "],
        prompt: true,
        rain: true,
    };
}

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
    /// Shaded blocks, as a DOS program drew a progress bar: `███▒░░░`.
    const SHADE: Self = Self {
        full: '█',
        mark: '▒',
        empty: '░',
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
    name: ThemeName,
    mode: Mode,
    /// Whether things may move: off when the user asked for reduced motion.
    motion: bool,
    glyphs: Glyphs,
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
    /// The can of the logo, and the mark and sparkles on it.
    pub logo_can: Style,
    pub logo_mark: Style,
    /// The leading glyph of a falling stream, and the glyphs behind it.
    pub rain_head: Style,
    pub rain_trail: Style,
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
            name: ThemeName::Neon,
            mode: Mode::Truecolor,
            motion: true,
            glyphs: Glyphs::NEON,
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
            warning_band: Style::new()
                .fg(BAND_INK)
                .bg(BAND_FILL)
                .add_modifier(Modifier::BOLD),
            verdict_safe: Style::new().fg(NEON_GREEN).add_modifier(Modifier::BOLD),
            verdict_blocked: Style::new().fg(NEON_AMBER).add_modifier(Modifier::BOLD),
            logo_can: Style::new().fg(NEON_MAGENTA).add_modifier(Modifier::BOLD),
            logo_mark: Style::new().fg(NEON_CYAN),
            rain_head: Style::new(),
            rain_trail: Style::new(),
            sizes: [
                Style::new().fg(NEON_CYAN),
                Style::new().fg(NEON_ORANGE),
                Style::new().fg(NEON_PINK).add_modifier(Modifier::BOLD),
            ],
            cells: Cells::BLOCKS,
            icons: IconSet::Unicode,
        }
    }

    /// The matrix look, for a truecolor terminal: phosphor on true black.
    const fn matrix_truecolor() -> Self {
        let bold = Modifier::BOLD;
        Self {
            name: ThemeName::Matrix,
            mode: Mode::Truecolor,
            motion: true,
            glyphs: Glyphs::MATRIX,
            ground: Style::new().fg(MX_TEXT).bg(MX_GROUND),
            text: Style::new().fg(MX_TEXT),
            muted: Style::new().fg(MX_QUIET),
            head: Style::new().fg(MX_HEAD).add_modifier(bold),
            accent: Style::new().fg(MX_ACCENT),
            key: Style::new().fg(MX_ACCENT).add_modifier(bold),
            safe: Style::new().fg(MX_ACCENT),
            blocked: Style::new().fg(MX_AMBER),
            danger: Style::new().fg(MX_RED).add_modifier(bold),
            violet: Style::new().fg(MX_QUIET),
            selected: on_black(MX_ACCENT),
            warning_band: on_black(MX_RED),
            verdict_safe: Style::new().fg(MX_ACCENT).add_modifier(bold),
            verdict_blocked: Style::new().fg(MX_AMBER).add_modifier(bold),
            logo_can: Style::new().fg(MX_ACCENT).add_modifier(bold),
            logo_mark: Style::new().fg(MX_HEAD),
            rain_head: Style::new().fg(MX_HEAD).add_modifier(bold),
            rain_trail: Style::new().fg(MX_QUIET),
            sizes: [
                Style::new().fg(MX_MID),
                Style::new().fg(MX_TEXT),
                Style::new().fg(MX_HEAD).add_modifier(bold),
            ],
            cells: Cells::SHADE,
            icons: IconSet::Unicode,
        }
    }

    /// The matrix look on the profile's own background: ANSI green, bright
    /// green, yellow and red, which a green phosphor look has natively.
    const fn matrix_ansi() -> Self {
        let bold = Modifier::BOLD;
        Self {
            name: ThemeName::Matrix,
            mode: Mode::Ansi,
            motion: true,
            glyphs: Glyphs::MATRIX,
            ground: Style::new(),
            text: Style::new().fg(Color::Green),
            muted: Style::new().fg(Color::Green).add_modifier(Modifier::DIM),
            head: Style::new().fg(Color::LightGreen).add_modifier(bold),
            accent: Style::new().fg(Color::LightGreen),
            key: Style::new().fg(Color::LightGreen).add_modifier(bold),
            safe: Style::new().fg(Color::LightGreen),
            blocked: Style::new().fg(Color::Yellow),
            danger: Style::new().fg(Color::Red).add_modifier(bold),
            violet: Style::new().fg(Color::Green).add_modifier(Modifier::DIM),
            selected: Style::new()
                .fg(Color::Green)
                .add_modifier(Modifier::REVERSED),
            warning_band: Style::new()
                .fg(Color::Red)
                .add_modifier(bold.union(Modifier::REVERSED)),
            verdict_safe: Style::new().fg(Color::LightGreen).add_modifier(bold),
            verdict_blocked: Style::new().fg(Color::Yellow).add_modifier(bold),
            logo_can: Style::new().fg(Color::Green).add_modifier(bold),
            logo_mark: Style::new().fg(Color::LightGreen),
            rain_head: Style::new().fg(Color::LightGreen).add_modifier(bold),
            rain_trail: Style::new().fg(Color::Green).add_modifier(Modifier::DIM),
            sizes: [
                Style::new().fg(Color::Green),
                Style::new().fg(Color::LightGreen),
                Style::new().fg(Color::LightGreen).add_modifier(bold),
            ],
            cells: Cells::SHADE,
            icons: IconSet::Unicode,
        }
    }

    /// The matrix look with no colour: its glyphs, and weight for the rest.
    const fn matrix_mono() -> Self {
        Self {
            name: ThemeName::Matrix,
            glyphs: Glyphs::MATRIX,
            cells: Cells::SHADE,
            ..Self::mono()
        }
    }

    /// The `name` theme, drawn for a terminal that carries `mode`.
    pub const fn named(name: ThemeName, mode: Mode) -> Self {
        match (name, mode) {
            (ThemeName::Neon, Mode::Truecolor) => Self::neon(),
            (ThemeName::Neon, Mode::Ansi) => Self::ansi(),
            (ThemeName::Neon, Mode::Mono) => Self::mono(),
            (ThemeName::Matrix, Mode::Truecolor) => Self::matrix_truecolor(),
            (ThemeName::Matrix, Mode::Ansi) => Self::matrix_ansi(),
            (ThemeName::Matrix, Mode::Mono) => Self::matrix_mono(),
        }
    }

    /// The same terminal, drawn as `name`: the colour mode, the icon set, the
    /// console's ASCII bars and the motion setting all stay what they were.
    pub fn switched(self, name: ThemeName) -> Self {
        let mut theme = Self::named(name, self.mode)
            .with_icons(self.icons)
            .with_motion(self.motion);
        if self.cells == Cells::ASCII {
            theme = theme.ascii();
        }
        theme
    }

    pub fn name(&self) -> ThemeName {
        self.name
    }

    /// Whether the interface may animate beyond a cursor.
    pub const fn with_motion(mut self, motion: bool) -> Self {
        self.motion = motion;
        self
    }

    /// Whether the interface may animate at all.
    pub fn motion(&self) -> bool {
        self.motion
    }

    /// `DEV_CLEANER_REDUCED_MOTION`: `1` or `true` asks for nothing to move.
    pub fn motion_from(self, reduced: Option<&str>) -> Self {
        self.with_motion(!matches!(reduced, Some("1" | "true")))
    }

    /// Whether digital rain falls beside a running scan: only in a theme that
    /// has it, only where there is colour for it, and never when the user asked
    /// for reduced motion.
    pub fn rain(&self) -> bool {
        self.glyphs.rain && self.mode != Mode::Mono && self.motion
    }

    /// The line a heading is drawn out with.
    pub fn rule(&self) -> char {
        self.glyphs.rule
    }

    /// A rule `width` columns wide, with the ends the theme closes a band with.
    pub fn rule_line(&self, width: usize) -> String {
        match self.glyphs.rule_ends {
            Some((left, right)) if width >= 2 => {
                format!("{left}{}{right}", self.rule().to_string().repeat(width - 2))
            }
            _ => self.rule().to_string().repeat(width),
        }
    }

    /// What a key cap is wrapped in: nothing, or `[` and `]`.
    pub fn caps(&self) -> (&'static str, &'static str) {
        self.glyphs.caps
    }

    /// The stepper's markers: a step done, the step here, a step to come.
    pub fn steps(&self) -> [&'static str; 3] {
        if self.icons == IconSet::Ascii {
            self.glyphs.steps_ascii
        } else {
            self.glyphs.steps
        }
    }

    /// Whether the screen's title is a DOS prompt with a blinking cursor.
    pub fn prompt(&self) -> bool {
        self.glyphs.prompt
    }

    /// The named ANSI colours, over the profile's own background.
    pub const fn ansi() -> Self {
        let bold = Modifier::BOLD;
        Self {
            name: ThemeName::Neon,
            mode: Mode::Ansi,
            motion: true,
            glyphs: Glyphs::NEON,
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
            logo_can: Style::new().fg(Color::Magenta).add_modifier(bold),
            logo_mark: Style::new().fg(Color::Cyan),
            rain_head: Style::new(),
            rain_trail: Style::new(),
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
            name: ThemeName::Neon,
            mode: Mode::Mono,
            motion: true,
            glyphs: Glyphs::NEON,
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
            logo_can: Style::new(),
            logo_mark: Style::new(),
            rain_head: Style::new(),
            rain_trail: Style::new(),
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
        Self::choose_named(ThemeName::Neon, no_color, colorterm)
    }

    /// [`Theme::choose`] for the theme `name`.
    pub fn choose_named(name: ThemeName, no_color: Option<&str>, colorterm: Option<&str>) -> Self {
        let mode = if no_color.is_some_and(|v| !v.is_empty()) {
            Mode::Mono
        } else {
            match colorterm {
                Some("truecolor" | "24bit") => Mode::Truecolor,
                _ => Mode::Ansi,
            }
        };
        Self::named(name, mode)
    }

    /// [`Theme::detect_named`] for the neon theme.
    pub fn detect() -> Self {
        Self::detect_named(ThemeName::Neon)
    }

    /// The `name` theme for this process's environment. Read once, at startup.
    pub fn detect_named(name: ThemeName) -> Self {
        Self::choose_named(
            name,
            std::env::var("NO_COLOR").ok().as_deref(),
            std::env::var("COLORTERM").ok().as_deref(),
        )
        .for_term(std::env::var("TERM").ok().as_deref())
        .icons_from(
            std::env::var("DEV_CLEANER_ICONS").ok().as_deref(),
            std::env::var("TERM").ok().as_deref(),
        )
        .motion_from(std::env::var("DEV_CLEANER_REDUCED_MOTION").ok().as_deref())
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

    /// Whether the logo may be drawn in braille: not where the icon set is
    /// ASCII, which is where the font cannot be trusted with anything but it.
    pub fn braille(&self) -> bool {
        self.icons != IconSet::Ascii
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
        let matrix = self.name == ThemeName::Matrix;
        match self.mode {
            Mode::Truecolor if matrix => Style::new().fg(match ramp {
                // Dim green, phosphor, white-green: the largest is the brightest.
                Ramp::Measure => three(MX_STOPS, t),
                Ramp::Danger => three(
                    [(0xff, 0xb0, 0x00), (0xff, 0x80, 0x20), (0xff, 0x40, 0x40)],
                    t,
                ),
            }),
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
                let (from, to) = match (ramp, matrix) {
                    (Ramp::Measure, false) => (Color::Cyan, Color::Magenta),
                    (Ramp::Measure, true) => (Color::Green, Color::LightGreen),
                    (Ramp::Danger, _) => (Color::Yellow, Color::Red),
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
        let matrix = self.name == ThemeName::Matrix;
        match self.mode {
            Mode::Truecolor if matrix => Style::new().fg(three(MX_STOPS, t)).add_modifier(bold),
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
                .fg(match (matrix, t < BRAND_SEAM) {
                    (false, true) => Color::Cyan,
                    (false, false) => Color::Magenta,
                    (true, true) => Color::Green,
                    (true, false) => Color::LightGreen,
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
        if self.mode == Mode::Mono {
            return line.to_string();
        }
        let (text, rule) = (sgr(self.accent), sgr(self.violet));
        let line = line.replace('·', &format!("{rule}·{text}"));
        format!("{text}{line}\x1b[0m")
    }

    /// `text` in `style`, wrapped in the escapes that draw it, for the plain
    /// text the command line prints. Nothing is added under `NO_COLOR`.
    pub fn paint(&self, style: Style, text: &str) -> String {
        match (self.mode, sgr(style)) {
            (Mode::Mono, _) => text.to_string(),
            (_, open) if open.is_empty() => text.to_string(),
            (_, open) => format!("{open}{text}\x1b[0m"),
        }
    }
}

/// The three stops of the matrix gradient: dim green, phosphor, white-green.
const MX_STOPS: [(u8, u8, u8); 3] = [(0x1f, 0x9a, 0x3f), (0x33, 0xff, 0x66), (0xcc, 0xff, 0xdd)];

/// `t` of the way along the line through three stops, as a colour.
fn three(stops: [(u8, u8, u8); 3], t: f32) -> Color {
    let (from, to, t) = if t < 0.5 {
        (stops[0], stops[1], t * 2.0)
    } else {
        (stops[1], stops[2], t * 2.0 - 1.0)
    };
    let mix = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
    Color::Rgb(mix(from.0, to.0), mix(from.1, to.1), mix(from.2, to.2))
}

/// The escape that opens `style`: weight first, then the colours. Empty for a
/// style that asks for nothing.
fn sgr(style: Style) -> String {
    let mut params: Vec<String> = Vec::new();
    for (modifier, code) in [
        (Modifier::BOLD, "1"),
        (Modifier::DIM, "2"),
        (Modifier::REVERSED, "7"),
    ] {
        if style.add_modifier.contains(modifier) {
            params.push(code.to_string());
        }
    }
    for (colour, base) in [(style.fg, 30), (style.bg, 40)] {
        let Some(colour) = colour else { continue };
        let named = |n: u8| (base + n).to_string();
        let bright = |n: u8| (base + 60 + n).to_string();
        params.push(match colour {
            Color::Rgb(r, g, b) => format!("{};2;{r};{g};{b}", base + 8),
            Color::Indexed(i) => format!("{};5;{i}", base + 8),
            Color::Black => named(0),
            Color::Red => named(1),
            Color::Green => named(2),
            Color::Yellow => named(3),
            Color::Blue => named(4),
            Color::Magenta => named(5),
            Color::Cyan => named(6),
            Color::Gray => named(7),
            Color::DarkGray => bright(0),
            Color::LightRed => bright(1),
            Color::LightGreen => bright(2),
            Color::LightYellow => bright(3),
            Color::LightBlue => bright(4),
            Color::LightMagenta => bright(5),
            Color::LightCyan => bright(6),
            Color::White => bright(7),
            Color::Reset => continue,
        });
    }
    if params.is_empty() {
        String::new()
    } else {
        format!("\x1b[{}m", params.join(";"))
    }
}

impl Default for Theme {
    /// The look every test and every embedder gets unless it asks for another:
    /// the profile's own colours, as before the theme existed.
    fn default() -> Self {
        Self::ansi()
    }
}

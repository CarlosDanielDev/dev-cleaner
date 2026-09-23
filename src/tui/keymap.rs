//! Every key the interface answers to, in one place.
//!
//! The promise the TUI makes is about all of its keys at once — that none of
//! the ones a hand finds by accident deletes anything, and that the one which
//! does is nowhere near the ones pressed all day. A promise about every key is
//! only testable against a table of every key, so the table is the keymap
//! rather than a description of it: screens read their bindings from here, and
//! `tests/keymap.rs` asserts over the same rows.

use super::Screen;
use super::candidates::Key;
use super::projects::Column;

/// A key as a terminal reports it.
///
/// Physical keys, not the actions they stand for. The invariants are about the
/// keyboard — what sits under which finger — so the table has to name the
/// thing that is actually pressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyPress {
    Char(char),
    Enter,
    Esc,
    Tab,
    Space,
    Backspace,
    Delete,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
}

impl std::fmt::Display for KeyPress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            KeyPress::Char(c) => write!(f, "{c}"),
            KeyPress::Enter => f.write_str("Enter"),
            KeyPress::Esc => f.write_str("Esc"),
            KeyPress::Tab => f.write_str("Tab"),
            KeyPress::Space => f.write_str("Space"),
            KeyPress::Backspace => f.write_str("Backspace"),
            KeyPress::Delete => f.write_str("Delete"),
            KeyPress::Up => f.write_str("Up"),
            KeyPress::Down => f.write_str("Down"),
            KeyPress::Left => f.write_str("Left"),
            KeyPress::Right => f.write_str("Right"),
            KeyPress::Home => f.write_str("Home"),
            KeyPress::End => f.write_str("End"),
            KeyPress::PageUp => f.write_str("PageUp"),
            KeyPress::PageDown => f.write_str("PageDown"),
        }
    }
}

/// Moving through a list, in the vocabulary the read-only screens share.
///
/// The candidates screen keeps its own [`Key`] instead: it is driven
/// exhaustively by a test that predates this table, and rewriting a proven
/// keymap to share an enum would risk the guarantee for the sake of tidiness.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Motion {
    Up,
    Down,
    Top,
    Bottom,
    PageUp,
    PageDown,
}

/// What pressing a key does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Quit,
    Help,
    Back,
    Forward,
    /// Move within a list that only shows things.
    Move(Motion),
    /// Order the projects table by a column, or reverse it if it is already
    /// the one in use. Reading, not marking: nothing about the plan changes.
    Sort(Column),
    /// Handled by `Candidates::press`, which owns the selection rules.
    Candidate(Key),
    /// Hold to carry the plan out. The only action in the table that deletes.
    Purge,
}

/// What a key costs to press.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    /// Changes where you are or what you are looking at, and nothing else.
    Navigate,
    /// Changes which candidates the plan will hold. Reversible, and nothing
    /// leaves the disk.
    Mark,
    /// Sends files to the Trash.
    Destructive,
}

impl Action {
    /// Derived from the action rather than declared beside it.
    ///
    /// A table row that carried its own severity could say `Navigate` next to a
    /// purge, and every invariant would pass while the guarantee was gone. Here
    /// a new destructive action has to be classified in one place that the
    /// compiler points at.
    pub fn effect(self) -> Effect {
        match self {
            Action::Purge => Effect::Destructive,
            Action::Candidate(Key::Toggle | Key::MarkAll | Key::ClearMarks) => Effect::Mark,
            Action::Quit
            | Action::Help
            | Action::Back
            | Action::Forward
            | Action::Move(_)
            | Action::Sort(_)
            | Action::Candidate(_) => Effect::Navigate,
        }
    }
}

/// One key, where it works, and what it does.
#[derive(Debug)]
pub struct Binding {
    pub key: KeyPress,
    /// The screen this key works on, or `None` for one that works everywhere.
    pub screen: Option<Screen>,
    pub action: Action,
    /// What to call it in a footer.
    pub label: &'static str,
}

impl Binding {
    pub fn effect(&self) -> Effect {
        self.action.effect()
    }
}

/// The key held down to purge.
///
/// Named here and used both by the table and by the confirm screen's own
/// drawing, so the key the user is told to press is the key the invariant test
/// governs. `x` was chosen for its neighbours: `z`, `s`, `d` and `c`, none of
/// which moves a cursor. The obvious mnemonic keys are all worse — `p` sits
/// beside `l`, and the whole `hjkl` neighbourhood is motion.
pub const PURGE: KeyPress = KeyPress::Char('x');

const fn global(key: KeyPress, action: Action, label: &'static str) -> Binding {
    Binding {
        key,
        screen: None,
        action,
        label,
    }
}

const fn on(screen: Screen, key: KeyPress, action: Action, label: &'static str) -> Binding {
    Binding {
        key,
        screen: Some(screen),
        action,
        label,
    }
}

/// Every binding in the interface.
pub fn bindings() -> &'static [Binding] {
    use Action::*;
    use KeyPress::*;
    const TABLE: &[Binding] = &[
        // Everywhere. Enter advances, which on the confirmation screen is a
        // deliberate no-op: the step out of it is not a move at all.
        global(Esc, Back, "back"),
        global(Enter, Forward, "next"),
        global(KeyPress::Char('q'), Quit, "quit"),
        global(KeyPress::Char('?'), Help, "keys"),
        // The projects table.
        on(Screen::Projects, Up, Move(Motion::Up), "up a row"),
        on(
            Screen::Projects,
            KeyPress::Char('k'),
            Move(Motion::Up),
            "up a row",
        ),
        on(Screen::Projects, Down, Move(Motion::Down), "down a row"),
        on(
            Screen::Projects,
            KeyPress::Char('j'),
            Move(Motion::Down),
            "down a row",
        ),
        // Ordering, on the digits, in the order the columns are drawn. Letters
        // were the obvious choice and are the wrong one: the mnemonic for
        // "size" is `s`, which sits next to the key that purges, and a table
        // is sorted far more often than a plan is confirmed.
        on(
            Screen::Projects,
            KeyPress::Char('1'),
            Sort(Column::Name),
            "by name",
        ),
        on(
            Screen::Projects,
            KeyPress::Char('2'),
            Sort(Column::Unique),
            "by unique",
        ),
        on(
            Screen::Projects,
            KeyPress::Char('3'),
            Sort(Column::Apparent),
            "by apparent",
        ),
        on(
            Screen::Projects,
            KeyPress::Char('4'),
            Sort(Column::Inodes),
            "by inodes",
        ),
        on(
            Screen::Projects,
            KeyPress::Char('5'),
            Sort(Column::Reclaimable),
            "by reclaimable",
        ),
        on(
            Screen::Projects,
            KeyPress::Char('6'),
            Sort(Column::Activity),
            "by activity",
        ),
        // The candidates screen, whose own enum this mirrors exactly.
        on(Screen::Candidates, Up, Candidate(Key::Up), "up"),
        on(
            Screen::Candidates,
            KeyPress::Char('k'),
            Candidate(Key::Up),
            "up",
        ),
        on(Screen::Candidates, Down, Candidate(Key::Down), "down"),
        on(
            Screen::Candidates,
            KeyPress::Char('j'),
            Candidate(Key::Down),
            "down",
        ),
        on(
            Screen::Candidates,
            KeyPress::Char('g'),
            Candidate(Key::Top),
            "first",
        ),
        on(
            Screen::Candidates,
            KeyPress::Char('G'),
            Candidate(Key::Bottom),
            "last",
        ),
        on(
            Screen::Candidates,
            PageUp,
            Candidate(Key::PageUp),
            "a page up",
        ),
        on(
            Screen::Candidates,
            PageDown,
            Candidate(Key::PageDown),
            "a page down",
        ),
        on(Screen::Candidates, Space, Candidate(Key::Toggle), "mark"),
        on(
            Screen::Candidates,
            KeyPress::Char('a'),
            Candidate(Key::MarkAll),
            "mark all",
        ),
        on(
            Screen::Candidates,
            KeyPress::Char('c'),
            Candidate(Key::ClearMarks),
            "clear marks",
        ),
        // Reading the plan.
        on(Screen::Review, Up, Move(Motion::Up), "up"),
        on(Screen::Review, KeyPress::Char('k'), Move(Motion::Up), "up"),
        on(Screen::Review, Down, Move(Motion::Down), "down"),
        on(
            Screen::Review,
            KeyPress::Char('j'),
            Move(Motion::Down),
            "down",
        ),
        on(Screen::Review, PageUp, Move(Motion::PageUp), "a page up"),
        on(
            Screen::Review,
            PageDown,
            Move(Motion::PageDown),
            "a page down",
        ),
        on(
            Screen::Review,
            KeyPress::Char('g'),
            Move(Motion::Top),
            "first",
        ),
        on(
            Screen::Review,
            KeyPress::Char('G'),
            Move(Motion::Bottom),
            "last",
        ),
        // The one binding that deletes, on the one screen that may.
        on(Screen::Confirm, PURGE, Purge, "hold to purge"),
    ];
    TABLE
}

/// The bindings a screen answers to, its own and the global ones.
pub fn bindings_for(screen: Screen) -> Vec<&'static Binding> {
    bindings()
        .iter()
        .filter(|b| b.screen.is_none() || b.screen == Some(screen))
        .collect()
}

/// The staggered rows of an ANSI keyboard, in quarter key widths.
///
/// Row offsets are the real ones: Tab is 1.5 keys wide, Caps 1.75, Shift 2.25,
/// which is why `q` sits between `1` and `2` rather than above `1`. A grid that
/// ignored the stagger would call `q` and `1` neighbours and `q` and `a`
/// strangers, which is backwards.
const ROWS: [(&str, i32); 4] = [
    ("`1234567890-=", 0),
    ("qwertyuiop[]", 6),
    ("asdfghjkl;'", 7),
    ("zxcvbnm,./", 9),
];

/// Where a key sits: its row, and the left edge of its cap in quarter widths.
///
/// ponytail: only the main block has coordinates. The arrow cluster, Enter,
/// Space and the editing keys sit in blocks of their own, far enough away that
/// no key in this table is beside them — so they are simply never adjacent to
/// anything, which is the honest answer for a letter key. Give them positions
/// if a binding ever needs to keep clear of an arrow.
fn position(key: KeyPress) -> Option<(usize, i32)> {
    let KeyPress::Char(c) = key else {
        return None;
    };
    // Shift does not move a key. `G` is where `g` is.
    let c = c.to_ascii_lowercase();
    ROWS.iter().enumerate().find_map(|(row, (keys, offset))| {
        keys.chars()
            .position(|k| k == c)
            .map(|i| (row, offset + 4 * i as i32))
    })
}

/// Whether two keys are close enough that a finger aiming for one hits the
/// other.
///
/// Adjacent means the caps touch: the same row one over, or the row above or
/// below with any horizontal overlap. A key is never adjacent to itself, and
/// `G` is not adjacent to `g` — they are the same key.
pub fn adjacent(a: KeyPress, b: KeyPress) -> bool {
    let (Some((row_a, x_a)), Some((row_b, x_b))) = (position(a), position(b)) else {
        return false;
    };
    let apart = (x_a - x_b).abs();
    let same_key = row_a == row_b && apart == 0;
    !same_key && row_a.abs_diff(row_b) <= 1 && apart <= 4
}

//! The candidates screen, where the central promise is kept or broken.

use dev_cleaner::safety::{BlockReason, Candidate, RegenCommand, Rejected, Safety};
use dev_cleaner::tui::{Candidates, Key};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use std::path::PathBuf;

const MB: u64 = 1024 * 1024;

fn regenerable(name: &str) -> Candidate {
    Candidate {
        path: PathBuf::from(name),
        bytes: 10 * MB,
        safety: Safety::Regenerable {
            regen: RegenCommand::new("npm install").expect("valid"),
        },
    }
}

fn cache(name: &str) -> Candidate {
    Candidate {
        path: PathBuf::from(name),
        bytes: 5 * MB,
        safety: Safety::Cache {
            refills_on: "the next build",
        },
    }
}

fn unproven(name: &str) -> Candidate {
    Candidate {
        path: PathBuf::from(name),
        bytes: MB,
        safety: Safety::for_unknown("nothing here says how it comes back"),
    }
}

fn protected(name: &str) -> Candidate {
    Candidate {
        path: PathBuf::from(name),
        bytes: MB,
        safety: Safety::Protected {
            reason: BlockReason::DockerVolume,
        },
    }
}

fn rejected(name: &str, reason: BlockReason) -> Rejected {
    Rejected {
        path: PathBuf::from(name),
        because: reason.explain().to_string(),
    }
}

/// A screen holding every kind of entry, with the unsafe ones interleaved among
/// the safe ones rather than tidily at one end.
fn mixed() -> Candidates {
    Candidates::new(
        vec![
            regenerable("/p/a/node_modules"),
            unproven("/p/b/mystery"),
            cache("/p/c/.gradle"),
            protected("/p/d/volume"),
            regenerable("/p/e/target"),
        ],
        vec![
            rejected("/p/f/vendor", BlockReason::DirtyWorktree),
            rejected("/p/g/.venv", BlockReason::StashEntries),
        ],
    )
}

fn text(c: &Candidates) -> String {
    text_at(c, 110)
}

/// The screen drawn into a terminal `width` columns wide, one line per row.
fn text_at(c: &Candidates, width: u16) -> String {
    let area = Rect::new(0, 0, width, 30);
    let mut buf = Buffer::empty(area);
    c.render(area, &mut buf);
    (0..area.height)
        .map(|y| {
            (0..area.width)
                .map(|x| buf[(x, y)].symbol())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn no_sequence_of_keys_can_put_the_cursor_on_a_blocked_entry() {
    // Every key in the keymap, in every order, to a depth of three. The claim
    // is not that the cursor skips blocked entries but that it cannot address
    // them, so no combination of keys is expected to find a way in.
    let keys = Key::all();
    let blocked: Vec<PathBuf> = mixed().blocked().iter().map(|b| b.path.clone()).collect();
    assert!(
        !blocked.is_empty(),
        "the fixture must contain blocked entries"
    );

    let mut checked = 0;
    for a in keys {
        for b in keys {
            for c in keys {
                let mut screen = mixed();
                for key in [a, b, c] {
                    screen.press(*key);
                    if let Some(sel) = screen.selected() {
                        assert!(
                            !blocked.contains(&sel.path),
                            "{:?} then {:?} then {:?} selected a blocked entry: {}",
                            a,
                            b,
                            c,
                            sel.path.display()
                        );
                        assert!(
                            sel.safety.is_selectable(),
                            "the cursor landed on an unselectable tier: {:?}",
                            sel.safety
                        );
                    }
                    checked += 1;
                }
            }
        }
    }
    assert!(checked >= 3 * keys.len().pow(3), "the walk did not run");
}

#[test]
fn an_unproven_candidate_is_moved_to_the_side_the_cursor_cannot_reach() {
    // Handed in among the candidates, not among the rejections. The screen
    // decides from the tier rather than trusting where the caller put it.
    let screen = Candidates::new(vec![unproven("/p/mystery")], vec![]);

    assert!(
        screen.selectable().is_empty(),
        "an unproven entry must not be offered"
    );
    assert_eq!(screen.blocked().len(), 1);
    assert!(screen.selected().is_none());
}

#[test]
fn a_protected_candidate_is_moved_to_the_side_the_cursor_cannot_reach() {
    let screen = Candidates::new(vec![protected("/p/volume")], vec![]);

    assert!(screen.selectable().is_empty());
    assert_eq!(screen.blocked().len(), 1);
}

#[test]
fn marking_can_only_ever_reach_selectable_entries() {
    let mut screen = mixed();
    screen.press(Key::MarkAll);

    let marked = screen.marked();
    assert_eq!(marked.len(), 3, "the three safe entries, and only those");
    for c in marked {
        assert!(c.safety.is_selectable(), "{:?} was marked", c.safety);
    }
}

#[test]
fn every_blocked_entry_says_why_in_plain_language() {
    let out = text(&mixed());

    for expected in [
        BlockReason::DirtyWorktree.explain(),
        BlockReason::StashEntries.explain(),
        BlockReason::DockerVolume.explain(),
    ] {
        assert!(
            out.contains(expected),
            "missing reason {expected:?}:\n{out}"
        );
    }
    assert!(
        out.contains("nothing here says how it comes back"),
        "an unproven entry must say what is unproven about it:\n{out}"
    );
}

#[test]
fn every_selectable_entry_says_how_it_comes_back() {
    let out = text(&mixed());

    assert!(
        out.contains("npm install"),
        "a regeneration command is missing:\n{out}"
    );
    assert!(
        out.contains("the next build"),
        "a cache must say what refills it:\n{out}"
    );
}

#[test]
fn a_screen_with_nothing_safe_still_explains_itself() {
    let screen = Candidates::new(
        vec![protected("/p/volume")],
        vec![rejected("/p/dirty/vendor", BlockReason::DirtyWorktree)],
    );

    assert!(screen.selected().is_none());
    let out = text(&screen);
    assert!(
        out.contains("dirty/vendor") && out.contains("volume"),
        "both blocked entries should still be shown:\n{out}"
    );
}

#[test]
fn toggling_marks_the_entry_under_the_cursor_and_toggling_again_clears_it() {
    let mut screen = mixed();
    let first = screen.selected().expect("a safe entry").path.clone();

    screen.press(Key::Toggle);
    assert_eq!(screen.marked().len(), 1);
    assert_eq!(screen.marked()[0].path, first);

    screen.press(Key::Toggle);
    assert!(screen.marked().is_empty(), "a second press clears the mark");
}

#[test]
fn clearing_marks_leaves_nothing_selected_for_purging() {
    let mut screen = mixed();
    screen.press(Key::MarkAll);
    assert_eq!(screen.marked().len(), 3);

    screen.press(Key::ClearMarks);
    assert!(screen.marked().is_empty());
}

#[test]
fn a_long_path_does_not_collide_with_the_column_beside_it() {
    // Real paths are far longer than any column a fixture suggests. Writing the
    // description at a fixed offset overwrote the middle of the path, leaving
    // "/Users/carlos/projects/.worktrees/akasregenerated on next importpycache__"
    // on screen: neither fact readable, and the entry impossible to identify.
    let long =
        "/Users/carlos/projects/.worktrees/akasha-limpar-guardrails/src/akasha/domain/__pycache__";
    let screen = Candidates::new(vec![regenerable(long)], vec![]);
    let out = text(&screen);

    assert_path_is_honest(&out, long, "npm install");
}

#[test]
fn a_long_blocked_path_does_not_collide_with_its_reason_either() {
    let long = "/Users/carlos/projects/alternatives/puravida/ios/Pods/SomeVeryLongPodName/vendor";
    let screen = Candidates::new(vec![], vec![rejected(long, BlockReason::DirtyWorktree)]);
    let out = text(&screen);

    assert_path_is_honest(&out, long, BlockReason::DirtyWorktree.explain());
}

/// Assert the row tells the truth about a path too long to fit.
///
/// Two ways it can lie. Text written past the end of one field leaves the tail
/// of another after it, which reads as a path that does not exist. And a path
/// cut short without saying so reads as a complete path somewhere else - which
/// checking that the row "ends correctly" does not catch, because a path with
/// text through its middle still ends in the right characters.
fn assert_path_is_honest(rendered: &str, path: &str, description: &str) {
    let row = rendered
        .lines()
        .find(|l| l.contains(description))
        .unwrap_or_else(|| panic!("no row showed {description:?}:\n{rendered}"));

    let after = row.split(description).nth(1).unwrap_or("").trim();
    assert!(
        after.is_empty(),
        "{after:?} follows the last column, so one field was drawn over another:\n{row}"
    );
    assert!(
        row.contains(path) || row.contains('…'),
        "the path did not fit and was cut without saying so, which reads as a \
         different path that exists:\n{row}"
    );
}

/// A reason no column could hold: the shape a rule that lists what it found
/// produces.
fn long_reason() -> String {
    let reason = "the worktree has changes that were never committed, in files a user would miss; "
        .repeat(3);
    assert!(
        reason.chars().count() >= 200,
        "the fixture must be longer than any column"
    );
    reason
}

/// The row of `rendered` that shows `head`, where `head` is the start of a
/// reason or a command. What a column keeps of a cut string is its head, and
/// the head is what a test knows to look for.
fn row_showing<'a>(rendered: &'a str, head: &str) -> &'a str {
    rendered
        .lines()
        .find(|l| l.contains(head))
        .unwrap_or_else(|| panic!("no row showed {head:?}:\n{rendered}"))
}

#[test]
fn a_reason_longer_than_its_column_ends_with_a_mark() {
    // Written whole at its column, a long reason ran to the buffer edge and
    // ratatui clipped it there without a sign: a sentence that read as if it
    // ended where the screen did.
    let reason = long_reason();
    let screen = Candidates::new(
        vec![],
        vec![Rejected {
            path: PathBuf::from("/p/f/vendor"),
            because: reason,
        }],
    );
    let out = text_at(&screen, 80);

    let row = row_showing(&out, "the worktree has changes");
    assert!(
        row.chars().count() < 80,
        "the reason ran through the margin to the edge of the screen:\n{row}"
    );
    assert!(
        row.ends_with('…'),
        "the reason was cut without saying so:\n{row}"
    );
}

#[test]
fn a_command_longer_than_its_column_ends_with_a_mark() {
    let command = "pip install -r requirements.txt -r requirements-dev.txt \
                   -r requirements-test.txt --no-cache-dir --upgrade";
    let screen = Candidates::new(
        vec![Candidate {
            path: PathBuf::from("/p/x"),
            bytes: MB,
            safety: Safety::Regenerable {
                regen: RegenCommand::new(command).expect("valid"),
            },
        }],
        vec![],
    );
    let out = text_at(&screen, 80);

    let row = row_showing(&out, "pip install");
    assert!(
        row.chars().count() < 80,
        "the command ran through the margin to the edge of the screen:\n{row}"
    );
    assert!(
        row.ends_with('…'),
        "the command was cut without saying so:\n{row}"
    );
}

#[test]
fn a_command_that_fits_is_drawn_whole() {
    // The column is sized to its longest entry, so the only entry fills it
    // exactly: the boundary at which a mark would be one cell too many.
    let screen = Candidates::new(vec![regenerable("/p/e/target")], vec![]);
    let out = text_at(&screen, 80);

    let row = row_showing(&out, "/p/e/target");
    assert!(
        row.contains("npm install"),
        "the command is missing:\n{row}"
    );
    assert!(
        !row.contains('…'),
        "nothing was cut, so nothing should say it was:\n{row}"
    );
}

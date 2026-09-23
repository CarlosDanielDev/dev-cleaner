//! The binding table: what every key does, on which screen, and at what cost.
//!
//! The guarantee the TUI exists to make is that no slip deletes anything. That
//! is a claim about every binding at once, so it is asserted over the whole
//! table rather than screen by screen.

use dev_cleaner::tui::{Action, Effect, Key, KeyPress, Screen, adjacent, bindings, bindings_for};

/// Every binding that can send something to the Trash.
fn destructive() -> Vec<&'static dev_cleaner::tui::Binding> {
    bindings()
        .iter()
        .filter(|b| b.effect() == Effect::Destructive)
        .collect()
}

#[test]
fn the_table_has_something_destructive_in_it_to_assert_about() {
    // Every invariant below is a statement about destructive bindings. With
    // none in the table they all pass while proving nothing, which is the shape
    // of a test that survives the feature being deleted.
    assert_eq!(
        destructive().len(),
        1,
        "exactly one action deletes anything: the hold on the confirm screen"
    );
    assert!(
        bindings().iter().any(|b| b.screen.is_none()),
        "the table must contain global bindings, or the rule that destructive \
         ones are never global asserts nothing"
    );
}

#[test]
fn no_destructive_binding_is_enter_delete_or_backspace() {
    // The three keys a hand reaches for without looking. Enter in particular is
    // bound globally to "next screen", so it is pressed constantly.
    for slip in [KeyPress::Enter, KeyPress::Delete, KeyPress::Backspace] {
        for binding in destructive() {
            assert_ne!(
                binding.key, slip,
                "{:?} is bound to {:?}, which deletes",
                slip, binding.action
            );
        }
    }
    assert!(
        bindings().iter().any(|b| b.key == KeyPress::Enter),
        "Enter must be in the table, or the rule above has nothing to exclude"
    );
}

#[test]
fn nothing_destructive_is_global_or_reachable_outside_the_confirm_screen() {
    for binding in destructive() {
        assert_eq!(
            binding.screen,
            Some(Screen::Confirm),
            "{:?} deletes and is reachable from {:?}",
            binding.action,
            binding.screen
        );
    }

    // Stated the other way round as well: walking every screen must find the
    // purge on exactly one of them.
    let screens_that_delete: Vec<Screen> = Screen::all()
        .into_iter()
        .filter(|s| {
            bindings_for(*s)
                .iter()
                .any(|b| b.effect() == Effect::Destructive)
        })
        .collect();
    assert_eq!(screens_that_delete, vec![Screen::Confirm]);
}

#[test]
fn the_confirm_key_is_not_adjacent_to_any_navigation_key() {
    let navigation: Vec<KeyPress> = bindings()
        .iter()
        .filter(|b| b.effect() == Effect::Navigate)
        .map(|b| b.key)
        .collect();
    assert!(
        !navigation.is_empty(),
        "no navigation keys to keep clear of"
    );

    for binding in destructive() {
        for key in &navigation {
            assert!(
                !adjacent(binding.key, *key),
                "{:?} deletes and sits beside {:?} on the keyboard",
                binding.key,
                key
            );
        }
    }
}

#[test]
fn adjacency_is_a_real_relation_and_not_an_empty_one() {
    // The invariant above is only worth as much as this function. A definition
    // that answered "never adjacent" would pass it against any keymap at all.
    assert!(
        adjacent(KeyPress::Char('j'), KeyPress::Char('k')),
        "j next to k"
    );
    assert!(
        adjacent(KeyPress::Char('q'), KeyPress::Char('a')),
        "q above a"
    );
    assert!(
        adjacent(KeyPress::Char('m'), KeyPress::Char('j')),
        "m below j"
    );
    assert!(
        !adjacent(KeyPress::Char('q'), KeyPress::Char('p')),
        "far apart"
    );
    assert!(
        !adjacent(KeyPress::Char('a'), KeyPress::Char('a')),
        "not itself"
    );

    for binding in destructive() {
        let neighbours = "abcdefghijklmnopqrstuvwxyz"
            .chars()
            .filter(|c| adjacent(binding.key, KeyPress::Char(*c)))
            .count();
        assert!(
            neighbours >= 3,
            "{:?} has {neighbours} neighbours, which means the geometry does \
             not know where it is",
            binding.key
        );
    }
}

#[test]
fn every_screen_is_covered_by_the_table() {
    for screen in Screen::all() {
        let found = bindings_for(screen);
        assert!(
            !found.is_empty(),
            "{screen:?} answers to no key at all, not even a global one"
        );
        assert!(
            found
                .iter()
                .any(|b| b.action == Action::Back || b.action == Action::Quit),
            "{screen:?} offers no way out"
        );
    }
}

#[test]
fn every_key_the_candidates_screen_answers_to_is_declared() {
    // The candidates screen keeps its own enum, driven exhaustively by
    // tests/candidates_screen.rs. If the table does not carry all of it, the
    // table is a description of the keymap rather than the keymap.
    for key in Key::all() {
        assert!(
            bindings_for(Screen::Candidates)
                .iter()
                .any(|b| b.action == Action::Candidate(*key)),
            "{key:?} is handled by the screen but missing from the table"
        );
    }
    for binding in bindings_for(Screen::Candidates) {
        if let Action::Candidate(key) = binding.action {
            assert!(
                Key::all().contains(&key),
                "{key:?} is bound but the screen does not answer to it"
            );
        }
    }
}

#[test]
fn no_key_means_two_things_on_one_screen() {
    // A key bound twice on the same screen has no defined behaviour, and the
    // one that loses is whichever the dispatch happens to reach second.
    for screen in Screen::all() {
        let found = bindings_for(screen);
        for (i, a) in found.iter().enumerate() {
            for b in &found[i + 1..] {
                assert!(
                    a.key != b.key,
                    "{:?} is bound to both {:?} and {:?} on {screen:?}",
                    a.key,
                    a.action,
                    b.action
                );
            }
        }
    }
}

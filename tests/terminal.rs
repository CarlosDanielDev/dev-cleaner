//! The panic path: a panic inside the loop still hands the terminal back.
//!
//! The hook is the only part of the driver that can be exercised without a
//! terminal attached, and it is the part that matters most: a panic that skips
//! restoration leaves the user with no cursor, no echo, and a shell they cannot
//! see themselves typing into.
//!
//! This is the whole binary. The panic hook is process-global, so a suite that
//! replaced it alongside other tests would be changing what happens when any of
//! them fails.

use std::panic;
use std::sync::{Mutex, OnceLock};

use dev_cleaner::tui::install_panic_hook;

/// What ran, in the order it ran.
fn order() -> &'static Mutex<Vec<&'static str>> {
    static ORDER: OnceLock<Mutex<Vec<&'static str>>> = OnceLock::new();
    ORDER.get_or_init(Mutex::default)
}

/// Stands in for handing the terminal back, which cannot be observed in a test
/// with no terminal to hand back.
fn restoring() {
    order().lock().expect("lock").push("restore");
}

#[test]
fn a_panic_restores_the_terminal_before_the_message_is_printed() {
    // The hook that prints is the one being chained to, so recording both in a
    // single list is what makes "before" an assertion rather than a hope. A
    // hook that restored afterwards would still restore, and the user would
    // still read the panic through a terminal in raw mode.
    panic::set_hook(Box::new(|_| order().lock().expect("lock").push("print")));
    install_panic_hook(restoring);

    let panicked = panic::catch_unwind(|| panic!("inside the loop")).is_err();

    // Copied out before asserting, and the lock released. An assertion that
    // failed while still holding it would panic into the hook under test, which
    // locks the same mutex, and the suite would hang instead of reporting.
    // Found by mutating the hook: the first version of this test deadlocked
    // rather than failing, which is a test that cannot report what it proves.
    let happened = order().lock().expect("lock").clone();
    // And the recording hook put back, so an assertion that fails below is
    // printed rather than swallowed by the hook this test installed.
    let _ = panic::take_hook();

    assert!(
        panicked,
        "the panic must still happen; the hook only cleans up before it"
    );
    assert_eq!(
        happened,
        vec!["restore", "print"],
        "the terminal has to be given back before anything is printed into it"
    );
}

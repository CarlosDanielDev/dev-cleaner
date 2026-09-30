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

use dev_cleaner::tui::{PURGE_THREAD, install_panic_hook};

/// What ran, in the order it ran.
fn order() -> &'static Mutex<Vec<&'static str>> {
    static ORDER: OnceLock<Mutex<Vec<&'static str>>> = OnceLock::new();
    ORDER.get_or_init(Mutex::default)
}

/// The hook is process-global, so the tests that replace it take turns.
fn one_at_a_time() -> std::sync::MutexGuard<'static, ()> {
    static TURN: Mutex<()> = Mutex::new(());
    TURN.lock().unwrap_or_else(|e| e.into_inner())
}

/// Stands in for handing the terminal back, which cannot be observed in a test
/// with no terminal to hand back.
fn restoring() {
    order().lock().expect("lock").push("restore");
}

#[test]
fn a_panic_restores_the_terminal_before_the_message_is_printed() {
    let _turn = one_at_a_time();
    order().lock().expect("lock").clear();
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

#[test]
fn a_panic_on_the_purge_thread_leaves_the_terminal_and_the_screen_alone() {
    // The loop is still drawing while the purge thread runs. A restore from the
    // thread's panic would take the alternate screen out from under it, and the
    // message would be printed across the frame; the loop reports the panic on
    // the result screen instead.
    let _turn = one_at_a_time();
    order().lock().expect("lock").clear();
    panic::set_hook(Box::new(|_| order().lock().expect("lock").push("print")));
    install_panic_hook(restoring);

    let panicked = std::thread::Builder::new()
        .name(PURGE_THREAD.to_string())
        .spawn(|| panic!("inside the purge"))
        .expect("spawn")
        .join()
        .is_err();

    let happened = order().lock().expect("lock").clone();
    let _ = panic::take_hook();

    assert!(panicked, "the thread must still panic");
    assert!(
        happened.is_empty(),
        "the purge thread's panic touched the terminal: {happened:?}"
    );
}

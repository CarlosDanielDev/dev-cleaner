//! Signals that end the run, noticed by the loop rather than acted on.
//!
//! A handler does nothing but remember that one arrived. The loop looks every
//! tick, cancels the scan, gives the terminal back and exits; a handler that
//! did any of that would be doing it from inside whatever the thread was in the
//! middle of, which for a terminal write is the one place it must not.

use std::sync::atomic::{AtomicI32, Ordering};

static ARRIVED: AtomicI32 = AtomicI32::new(0);

extern "C" fn note(signal: libc::c_int) {
    ARRIVED.store(signal, Ordering::Relaxed);
}

/// Have SIGINT, SIGTERM and SIGHUP noted rather than kill the process outright.
pub fn install() {
    for signal in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP] {
        // SAFETY: `note` is an async-signal-safe handler: one atomic store.
        unsafe {
            libc::signal(signal, note as *const () as libc::sighandler_t);
        }
    }
}

/// The signal that has arrived, if one has.
pub fn pending() -> Option<i32> {
    match ARRIVED.load(Ordering::Relaxed) {
        0 => None,
        signal => Some(signal),
    }
}

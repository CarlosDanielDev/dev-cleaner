//! Raw mode and the alternate screen, given back on every way out.
//!
//! There are three ways out of a terminal interface: the user quits, the loop
//! returns an error, or something panics. The first two are ordinary control
//! flow; the third is not, and it is the one that costs the user a working
//! shell. A panic that unwinds past the restore leaves no cursor, no echo, and
//! a screen that is not the one the shell is writing to.
//!
//! `ratatui::try_init` would do the setup below in one call, and it installs a
//! restoring panic hook of its own. The hook is the reason this is written out:
//! a hook installed inside the dependency is a hook this repository cannot put
//! a test on, and `docs/SESSION-HANDOFF.md` is explicit that an untested guard
//! is an intention. Everything else here is the dependency's own work —
//! restoration is `ratatui::restore`, not a second implementation of it.

use std::io;
use std::panic;

use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{EnterAlternateScreen, enable_raw_mode};
use ratatui::init::DefaultTerminal;

/// Put the terminal into the state the interface draws in.
///
/// Nothing here unwraps. Each step is reported so the caller can print the
/// reason on the terminal the user still has, rather than panicking through a
/// half-configured one.
pub fn enter() -> io::Result<DefaultTerminal> {
    // Installed before the first mode change, so a panic anywhere after this
    // line is cleaned up — including one inside the setup below.
    install_panic_hook(leave);
    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen)?;
    Terminal::new(CrosstermBackend::new(io::stdout()))
}

/// Hand the terminal back: raw mode off, alternate screen left.
///
/// A failure is printed rather than returned. There is nothing useful left to
/// do about it at this point, and swallowing it would hide the one message that
/// explains why the shell is behaving oddly.
pub fn leave() {
    ratatui::restore();
}

/// Restore the terminal before anything is printed into it.
///
/// The previous hook is kept and called afterwards, so the panic still reports
/// itself exactly as it would have: this adds a step, it does not replace one.
/// Taking the restore as an argument is what makes the order testable without a
/// terminal to observe — see `tests/terminal.rs`.
pub fn install_panic_hook(restore: fn()) {
    let previous = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        restore();
        previous(info);
    }));
}

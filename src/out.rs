//! Writing the report without letting a vanished reader end the run.
//!
//! `println!` panics when the write fails, and piping a long report into `head`
//! or `less` makes it fail as a matter of course: the reader exits, the pipe
//! closes, and the next line returns `EPIPE`. The panic then kills the process
//! partway through, before the scan is recorded, while the shell reports the
//! reader's exit status and the report on screen looks complete.
//!
//! These write the same lines and classify the failure instead. A reader that
//! went away is not an error — the work still has to finish. Anything else is
//! a real failure and is reported rather than swallowed.

use std::fmt::Arguments;
use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};

/// What became of one line.
#[derive(Debug)]
pub enum Wrote {
    Line,
    /// The pipe closed because whatever was reading exited. Expected.
    ReaderGone,
    /// A genuine failure: a full disk, a revoked terminal, a bad descriptor.
    Failed(io::Error),
}

/// Write one line, classifying failure rather than panicking on it.
///
/// Split from the streams so both cases can be tested against a writer that
/// fails on demand; a real broken pipe is awkward to arrange in a unit test and
/// a full disk more so.
pub fn write_line<W: Write>(w: &mut W, args: Arguments) -> Wrote {
    match writeln!(w, "{args}") {
        Ok(()) => Wrote::Line,
        Err(e) if e.kind() == io::ErrorKind::BrokenPipe => Wrote::ReaderGone,
        Err(e) => Wrote::Failed(e),
    }
}

/// Set once a stream has stopped accepting writes, so the remaining lines are
/// skipped instead of failing one syscall at a time.
static STDOUT_CLOSED: AtomicBool = AtomicBool::new(false);
static STDERR_CLOSED: AtomicBool = AtomicBool::new(false);

/// Write a report line to stdout. Never panics, never aborts the run.
pub fn line(args: Arguments) {
    if STDOUT_CLOSED.load(Ordering::Relaxed) {
        return;
    }
    let stdout = io::stdout();
    match write_line(&mut stdout.lock(), args) {
        Wrote::Line => {}
        Wrote::ReaderGone => STDOUT_CLOSED.store(true, Ordering::Relaxed),
        Wrote::Failed(err) => {
            STDOUT_CLOSED.store(true, Ordering::Relaxed);
            warn(format_args!(
                "the rest of the report could not be written: {err}"
            ));
        }
    }
}

/// Write a warning to stderr, under the same rules.
///
/// `2>&1 | head` makes stderr the broken pipe instead, and a warning is no more
/// worth dying for than a report line. When stderr is gone too there is nowhere
/// left to report, which is the one case that is genuinely silent.
pub fn warn(args: Arguments) {
    if STDERR_CLOSED.load(Ordering::Relaxed) {
        return;
    }
    let stderr = io::stderr();
    if !matches!(write_line(&mut stderr.lock(), args), Wrote::Line) {
        STDERR_CLOSED.store(true, Ordering::Relaxed);
    }
}

/// `println!` that survives a reader walking away.
macro_rules! outln {
    () => { crate::out::line(format_args!("")) };
    ($($arg:tt)*) => { crate::out::line(format_args!($($arg)*)) };
}

/// `eprintln!` under the same rule.
macro_rules! warnln {
    () => { crate::out::warn(format_args!("")) };
    ($($arg:tt)*) => { crate::out::warn(format_args!($($arg)*)) };
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A writer that fails every write with a chosen kind.
    struct Broken(io::ErrorKind);

    impl Write for Broken {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::Error::from(self.0))
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn a_working_stream_gets_the_line() {
        let mut sink = Vec::new();
        let wrote = write_line(&mut sink, format_args!("scanned {} roots", 3));

        assert!(matches!(wrote, Wrote::Line));
        assert_eq!(String::from_utf8(sink).expect("utf8"), "scanned 3 roots\n");
    }

    #[test]
    fn a_reader_that_went_away_is_not_an_error() {
        let mut gone = Broken(io::ErrorKind::BrokenPipe);
        let wrote = write_line(&mut gone, format_args!("anything"));

        assert!(
            matches!(wrote, Wrote::ReaderGone),
            "a closed pipe is the ordinary result of piping into head, not a failure"
        );
    }

    #[test]
    fn a_genuine_write_failure_is_not_swallowed() {
        // Tolerating a broken pipe must not turn into tolerating a full disk or
        // a descriptor that was taken away.
        for kind in [
            io::ErrorKind::PermissionDenied,
            io::ErrorKind::StorageFull,
            io::ErrorKind::InvalidInput,
        ] {
            let mut failing = Broken(kind);
            match write_line(&mut failing, format_args!("x")) {
                Wrote::Failed(err) => assert_eq!(err.kind(), kind),
                other => panic!("{kind:?} should surface as a failure, got {other:?}"),
            }
        }
    }
}

//! The line that says a walk is still moving.
//!
//! The walk runs on a thread and the caller's thread does the talking, so a
//! slow disk and a hung mount stop looking alike: one line keeps counting and
//! the other does not.

use std::io::IsTerminal;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use dev_cleaner::bytes::human;
use dev_cleaner::scan::Progress;
use dev_cleaner::tui::palette::Theme;
use dev_cleaner::tui::splash::{Splash, Tick};
use ratatui::backend::Backend;

use crate::out;

/// How often the line is redrawn.
pub const TICK: Duration = Duration::from_millis(200);

/// How often the interface's scan is redrawn: often enough that the logo is on
/// screen about 300 ms in, where [`TICK`] would make it 400.
pub const SPLASH_TICK: Duration = Duration::from_millis(100);

/// The line shown while the walk runs.
pub fn live(roots: usize, entries: u64, bytes: u64, elapsed: Duration) -> String {
    format!(
        "scanning {roots} {} · {} entries · {} · {:.1} s",
        plural(roots, "root"),
        grouped(entries),
        human(bytes),
        elapsed.as_secs_f64()
    )
}

/// The line that replaces it once the walk is done.
pub fn finished(projects: usize, entries: u64, elapsed: Duration) -> String {
    format!(
        "scanned {projects} {}, {} entries in {:.2} s",
        plural(projects, "project"),
        grouped(entries),
        elapsed.as_secs_f64()
    )
}

fn plural(n: usize, noun: &str) -> String {
    if n == 1 {
        noun.into()
    } else {
        format!("{noun}s")
    }
}

/// `204107` as `204,107`, so a counter that moves fast stays readable.
fn grouped(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// [`watch`] as the commands use it: every [`TICK`], on stdout, only when
/// stdout is a terminal. `scan` and `tui` both come through here, so they show
/// the same line.
pub fn show<T: Send>(progress: &Arc<Progress>, roots: usize, work: impl FnOnce() -> T + Send) -> T {
    let live = std::io::stdout().is_terminal();
    // Read once, before the walk starts: the line is drawn from another thread's
    // clock, and none of it should be looking at the environment.
    let theme = Theme::detect();
    watch(progress, roots, TICK, live, |s| plain(&theme, s), work)
}

/// [`show`] for the interface: the same line, and once the scan has outlasted
/// a short wait on a terminal big enough, the whole logo with that line under
/// it, in the alternate screen the interface is about to use anyway.
pub fn show_with_logo<T: Send>(
    progress: &Arc<Progress>,
    roots: usize,
    work: impl FnOnce() -> T + Send,
) -> T {
    let live = std::io::stdout().is_terminal();
    let theme = Theme::detect();
    let mut splash = Splash::on_terminal(theme);
    watch_with_logo(
        progress,
        roots,
        SPLASH_TICK,
        live,
        &mut splash,
        |s| plain(&theme, s),
        interrupted,
        work,
    )
}

/// What a Ctrl-C does while the splash holds raw mode, where it is a key: the
/// same as it does without, which is to end the run.
fn interrupted() {
    std::process::exit(130);
}

/// One frame of the plain line, coloured; the escape that wipes it is not text.
fn plain(theme: &Theme, s: &str) {
    match s.strip_prefix('\r') {
        Some(line) if !line.starts_with('\x1b') => {
            out::redraw(format_args!("\r{}", theme.progress_line(line)));
        }
        _ => out::redraw(format_args!("{s}")),
    }
}

/// [`watch`] with a splash: each frame is the splash's if it takes it, and
/// `text`'s if not. The line the splash covered is wiped once it is taken down.
#[allow(clippy::too_many_arguments)]
fn watch_with_logo<B: Backend, T: Send>(
    progress: &Arc<Progress>,
    roots: usize,
    tick: Duration,
    live: bool,
    splash: &mut Splash<B>,
    mut text: impl FnMut(&str),
    interrupt: fn(),
    work: impl FnOnce() -> T + Send,
) -> T {
    let value = watch(
        progress,
        roots,
        tick,
        live,
        |s| match s.strip_prefix('\r') {
            Some(line) if !line.starts_with('\x1b') => match splash.tick(line) {
                Tick::Quiet => text(s),
                Tick::Shown => {}
                Tick::Interrupted => {
                    splash.finish();
                    interrupt();
                }
            },
            // The wipe at the end: the splash is under the alternate screen
            // and takes no wipe, the plain line does.
            _ if splash.shown() => {}
            _ => text(s),
        },
        work,
    );
    if splash.finish() {
        text("\r\x1b[2K");
    }
    value
}

/// Run `work` on a thread and call `draw` with the live line every `tick`.
///
/// `draw` gets raw text: a leading `\r` to redraw in place, and a last frame
/// that wipes the line, so the caller's own final line starts on a clean one.
/// Nothing is drawn when `live` is false (stdout is not a terminal), or when
/// the walk ends before the first tick. Ctrl-C needs no handling here: this
/// thread only waits, so the default signal action still ends the process.
pub fn watch<T: Send>(
    progress: &Arc<Progress>,
    roots: usize,
    tick: Duration,
    live_line: bool,
    mut draw: impl FnMut(&str),
    work: impl FnOnce() -> T + Send,
) -> T {
    let started = Instant::now();
    let mut drawn = false;
    thread::scope(|scope| {
        let (done, result) = mpsc::channel();
        let worker = scope.spawn(move || {
            let _ = done.send(work());
        });
        loop {
            match result.recv_timeout(tick) {
                Ok(value) => {
                    if drawn {
                        draw("\r\x1b[2K");
                    }
                    return value;
                }
                Err(RecvTimeoutError::Timeout) => {
                    if live_line {
                        let line = live(
                            roots,
                            progress.entries.load(Ordering::Relaxed),
                            progress.bytes.load(Ordering::Relaxed),
                            started.elapsed(),
                        );
                        draw(&format!("\r{line}"));
                        drawn = true;
                    }
                }
                // The walk ended without a result: it panicked. Hand the panic on.
                Err(RecvTimeoutError::Disconnected) => match worker.join() {
                    Err(panic) => std::panic::resume_unwind(panic),
                    Ok(()) => unreachable!("the walk finished without sending its result"),
                },
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_live_line_carries_roots_entries_bytes_and_time() {
        let line = live(2, 118_402, 10_523_000_000, Duration::from_millis(3_100));
        assert_eq!(line, "scanning 2 roots · 118,402 entries · 9.80 GB · 3.1 s");
    }

    #[test]
    fn one_root_is_not_plural() {
        assert!(live(1, 0, 0, Duration::ZERO).starts_with("scanning 1 root ·"));
    }

    #[test]
    fn the_final_line_names_projects_and_entries() {
        let line = finished(258, 204_107, Duration::from_millis(1_610));
        assert_eq!(line, "scanned 258 projects, 204,107 entries in 1.61 s");
        assert_eq!(
            finished(1, 5, Duration::ZERO),
            "scanned 1 project, 5 entries in 0.00 s"
        );
    }

    /// A walk that does not finish until a frame has been drawn, so the tests
    /// depend on ordering and not on how fast the machine is.
    fn walk_until_drawn(progress: &Arc<Progress>, drawn: mpsc::Receiver<()>) -> u32 {
        progress.entries.store(7, Ordering::Relaxed);
        drawn
            .recv_timeout(Duration::from_secs(30))
            .expect("no frame was ever drawn");
        42
    }

    #[test]
    fn a_terminal_sees_the_line_redrawn_in_place_and_then_cleared() {
        let progress = Arc::new(Progress::default());
        let mut frames = Vec::new();
        let (drew, drawn) = mpsc::channel();

        let got = watch(
            &progress,
            1,
            Duration::from_millis(5),
            true,
            |s| {
                frames.push(s.to_string());
                let _ = drew.send(());
            },
            || walk_until_drawn(&progress, drawn),
        );

        assert_eq!(got, 42, "the walk's result is handed back");
        assert!(frames.len() >= 2, "a redraw and the wipe, got {frames:?}");
        assert!(frames.iter().all(|f| f.starts_with('\r')));
        assert!(frames[0].contains("7 entries"), "{frames:?}");
        assert_eq!(
            frames.last().expect("frames"),
            "\r\x1b[2K",
            "the last frame wipes the line so the final one does not inherit its tail"
        );
    }

    #[test]
    fn a_pipe_sees_nothing_in_between() {
        let progress = Arc::new(Progress::default());
        let mut frames = Vec::new();

        watch(
            &progress,
            1,
            Duration::from_millis(5),
            false,
            |s| frames.push(s.to_string()),
            || std::thread::sleep(Duration::from_millis(60)),
        );

        assert!(
            frames.is_empty(),
            "a log must not fill with redraws: {frames:?}"
        );
    }

    #[test]
    fn a_walk_faster_than_a_tick_draws_nothing() {
        let progress = Arc::new(Progress::default());
        let mut frames = Vec::new();

        watch(
            &progress,
            1,
            Duration::from_secs(5),
            true,
            |s| frames.push(s.to_string()),
            || (),
        );

        assert!(
            frames.is_empty(),
            "nothing to wipe, nothing drawn: {frames:?}"
        );
    }

    #[test]
    #[should_panic(expected = "walk blew up")]
    fn a_panic_in_the_walk_is_not_swallowed() {
        let progress = Arc::new(Progress::default());
        watch(
            &progress,
            1,
            Duration::from_millis(5),
            true,
            |_| {},
            || -> () { panic!("walk blew up") },
        );
    }

    /// What a splash over a test backend did, in order.
    static EVENTS: std::sync::Mutex<Vec<&'static str>> = std::sync::Mutex::new(Vec::new());

    fn note(event: &'static str) {
        EVENTS.lock().unwrap().push(event);
    }

    fn entered() -> std::io::Result<ratatui::Terminal<ratatui::backend::TestBackend>> {
        note("enter");
        Ok(ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 24)).unwrap())
    }

    /// A scan that takes `scan`, under a splash that waits `after`; what the
    /// plain line and the splash did, in order.
    fn scan_taking(scan: Duration, after: Duration, size: fn() -> (u16, u16)) -> Vec<&'static str> {
        EVENTS.lock().unwrap().clear();
        let progress = Arc::new(Progress::default());
        let mut splash = Splash::with(Theme::neon(), size, entered, || note("leave")).after(after);
        let value = watch_with_logo(
            &progress,
            1,
            Duration::from_millis(5),
            true,
            &mut splash,
            |s| {
                note(if s.contains("\x1b[2K") {
                    "wipe"
                } else {
                    "line"
                })
            },
            || note("interrupted"),
            || {
                thread::sleep(scan);
                7
            },
        );
        assert_eq!(value, 7, "the scan's result was lost");
        EVENTS.lock().unwrap().clone()
    }

    /// One test owns the log: it is process-wide.
    #[test]
    fn the_logo_is_drawn_into_a_scan_that_outlasts_the_wait_and_never_into_a_quick_one() {
        let big: fn() -> (u16, u16) = || (80, 24);
        let slow = scan_taking(Duration::from_millis(150), Duration::from_millis(30), big);
        let enter = slow.iter().position(|e| *e == "enter").expect("no splash");
        assert!(slow[..enter].iter().all(|e| *e == "line"), "{slow:?}");
        assert!(
            slow[enter..].iter().all(|e| *e != "line"),
            "the plain line was drawn over the logo: {slow:?}"
        );
        // Taken down exactly once, and the covered line wiped after it.
        assert_eq!(&slow[slow.len() - 2..], ["leave", "wipe"], "{slow:?}");
        assert_eq!(slow.iter().filter(|e| **e == "enter").count(), 1);
        assert_eq!(slow.iter().filter(|e| **e == "wipe").count(), 1, "{slow:?}");

        // Ctrl-C under the logo takes it down and ends the run.
        EVENTS.lock().unwrap().clear();
        let mut splash = Splash::with(Theme::neon(), big, entered, || note("leave"))
            .after(Duration::ZERO)
            .keys(|| true);
        let progress = Arc::new(Progress::default());
        watch_with_logo(
            &progress,
            1,
            Duration::from_millis(5),
            true,
            &mut splash,
            |_| {},
            || note("interrupted"),
            || thread::sleep(Duration::from_millis(40)),
        );
        let stopped = EVENTS.lock().unwrap().clone();
        let ended = stopped
            .iter()
            .position(|e| *e == "interrupted")
            .expect("{stopped:?}");
        assert_eq!(stopped[ended - 1], "leave", "{stopped:?}");

        // A scan quicker than the wait draws no logo, with nothing to take down.
        let quick = scan_taking(Duration::ZERO, Duration::from_secs(5), big);
        assert!(
            !quick.contains(&"enter") && !quick.contains(&"leave"),
            "{quick:?}"
        );

        // A terminal too small for it keeps the line however long the scan runs.
        let small = scan_taking(Duration::from_millis(80), Duration::ZERO, || (79, 24));
        assert!(!small.contains(&"enter"), "{small:?}");
        assert!(small.contains(&"line"), "{small:?}");
    }
}

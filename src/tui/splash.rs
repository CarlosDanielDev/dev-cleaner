//! The logo, whole, while the first scan runs.
//!
//! The scan is a wait the user already sits through, so the art is drawn into
//! it: centred, with the scan's own progress line under it, and gone the moment
//! the dashboard replaces it. It is a screen of no key, no state and no delay:
//! it is drawn between the ticks of the work, never instead of it.
//!
//! It appears only once the scan has outlasted [`AFTER`], so a quick scan never
//! flashes it, and only where the art fits. Anywhere else the scan shows the
//! progress line it always did, on the screen the user still has.

use std::io;
use std::time::{Duration, Instant};

use ratatui::Terminal;
use ratatui::backend::{Backend, CrosstermBackend};
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::crossterm::terminal::size;
use ratatui::layout::Rect;

use super::logo::{self, MASTER_HEIGHT, MASTER_WIDTH};
use super::palette::Theme;
use super::terminal;

/// How long a scan runs before the logo is drawn.
pub const AFTER: Duration = Duration::from_millis(300);

/// The smallest terminal the art is drawn on: the one every screen is laid out
/// for. It needs about 50 by 19 cells; the rest is margin.
pub const MIN_COLS: u16 = 80;
pub const MIN_ROWS: u16 = 24;

/// Whether the logo should be on screen: the scan has run long enough and the
/// terminal is big enough for the art and its line.
pub fn wanted(elapsed: Duration, size: (u16, u16)) -> bool {
    elapsed >= AFTER && fits(size)
}

/// Whether the art and its line fit a terminal of `cols` by `rows`.
pub fn fits((cols, rows): (u16, u16)) -> bool {
    cols >= MIN_COLS && rows >= MIN_ROWS
}

/// Paint the splash into `buf`: the ground, the art centred, the line under it.
pub fn render(theme: &Theme, line: &str, area: Rect, buf: &mut Buffer) {
    buf.set_style(area, theme.ground);
    // The art, a blank row and the line, centred as one block.
    let top = area.y + area.height.saturating_sub(MASTER_HEIGHT + 2) / 2;
    let left = area.x + area.width.saturating_sub(MASTER_WIDTH) / 2;
    logo::draw_master(theme, buf, left, top);
    let line: String = line.chars().take(area.width as usize).collect();
    let at = area.x + area.width.saturating_sub(line.chars().count() as u16) / 2;
    buf.set_string(at, top + MASTER_HEIGHT + 1, line, theme.text);
}

/// What a [`Splash::tick`] did.
#[derive(Debug, PartialEq, Eq)]
pub enum Tick {
    /// Nothing was drawn: the caller shows its own line.
    Quiet,
    /// The splash is up and drew this line.
    Shown,
    /// Ctrl-C arrived while the terminal was in raw mode, where it is a key and
    /// not a signal.
    Interrupted,
}

/// The splash, entered the first time a tick finds it wanted.
pub struct Splash<B: Backend> {
    theme: Theme,
    started: Instant,
    after: Duration,
    size: fn() -> (u16, u16),
    enter: fn() -> io::Result<Terminal<B>>,
    leave: fn(),
    /// Whether Ctrl-C is waiting; the terminal's own queue unless a test says.
    keys: fn() -> bool,
    terminal: Option<Terminal<B>>,
    /// Set once entering failed, so a terminal that cannot do it is asked once.
    off: bool,
}

impl Splash<CrosstermBackend<io::Stdout>> {
    /// The splash on the real terminal, which takes raw mode and the alternate
    /// screen the same way the interface does and gives them back the same way.
    pub fn on_terminal(theme: Theme) -> Self {
        Self::with(
            theme,
            || size().unwrap_or((0, 0)),
            terminal::enter,
            terminal::leave,
        )
    }
}

impl<B: Backend> Splash<B> {
    /// A splash over any backend, so a test can look at what was drawn.
    pub fn with(
        theme: Theme,
        size: fn() -> (u16, u16),
        enter: fn() -> io::Result<Terminal<B>>,
        leave: fn(),
    ) -> Self {
        Self {
            theme,
            started: Instant::now(),
            after: AFTER,
            size,
            enter,
            leave,
            keys: interrupted,
            terminal: None,
            off: false,
        }
    }

    /// Wait `after` instead of [`AFTER`].
    pub fn after(mut self, after: Duration) -> Self {
        self.after = after;
        self
    }

    /// Ask `keys` whether Ctrl-C is waiting, instead of the terminal.
    pub fn keys(mut self, keys: fn() -> bool) -> Self {
        self.keys = keys;
        self
    }

    /// Whether the splash is on screen.
    pub fn shown(&self) -> bool {
        self.terminal.is_some()
    }

    /// Draw `line` under the logo, entering the splash first if it is time.
    pub fn tick(&mut self, line: &str) -> Tick {
        if self.off {
            return Tick::Quiet;
        }
        if self.terminal.is_none() {
            let elapsed = self.started.elapsed();
            if elapsed < self.after || !fits((self.size)()) {
                return Tick::Quiet;
            }
            match (self.enter)() {
                Ok(terminal) => self.terminal = Some(terminal),
                Err(_) => {
                    // Whatever of the mode change took is undone, and the
                    // line goes on being drawn in plain text.
                    (self.leave)();
                    self.off = true;
                    return Tick::Quiet;
                }
            }
        }
        if (self.keys)() {
            return Tick::Interrupted;
        }
        let theme = self.theme;
        let drew = self.terminal.as_mut().is_some_and(|t| {
            t.draw(|f| render(&theme, line, f.area(), f.buffer_mut()))
                .is_ok()
        });
        if !drew {
            self.finish();
            self.off = true;
            return Tick::Quiet;
        }
        Tick::Shown
    }

    /// Give the terminal back. Says whether there was a splash to take down.
    pub fn finish(&mut self) -> bool {
        let was = self.terminal.take().is_some();
        if was {
            (self.leave)();
        }
        was
    }
}

/// Whether Ctrl-C is waiting. Anything else pressed during the scan is read
/// and dropped: the interface that follows should not start on stray keys.
fn interrupted() -> bool {
    while event::poll(Duration::ZERO).unwrap_or(false) {
        if let Ok(Event::Key(key)) = event::read()
            && key.kind != KeyEventKind::Release
            && key.code == KeyCode::Char('c')
            && key.modifiers.contains(KeyModifiers::CONTROL)
        {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn text(buf: &Buffer, y: u16) -> String {
        (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect()
    }

    fn drawn(buf: &Buffer) -> Vec<(u16, u16)> {
        let area = buf.area;
        let mut cells = Vec::new();
        for y in 0..area.height {
            for x in 0..area.width {
                if "▀▄█▒".contains(buf[(x, y)].symbol()) {
                    cells.push((x, y));
                }
            }
        }
        cells
    }

    #[test]
    fn the_logo_waits_for_the_scan_to_outlast_the_threshold() {
        let big = (80, 24);
        assert!(!wanted(AFTER - Duration::from_millis(1), big));
        assert!(wanted(AFTER, big));
        assert!(!wanted(AFTER, (79, 24)));
        assert!(!wanted(AFTER, (80, 23)));
    }

    #[test]
    fn the_art_is_centred_with_the_line_under_it() {
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        render(
            &Theme::neon(),
            "scanning 1 root · 12 entries",
            area,
            &mut buf,
        );
        let cells = drawn(&buf);
        let (left, right) = (
            cells.iter().map(|c| c.0).min().unwrap(),
            cells.iter().map(|c| c.0).max().unwrap(),
        );
        assert_eq!((left, right), (15, 64), "centred columns");
        let bottom = cells.iter().map(|c| c.1).max().unwrap();
        assert!(text(&buf, bottom + 2).contains("scanning 1 root · 12 entries"));
        assert!(cells.iter().map(|c| c.1).min().unwrap() >= 2);
    }

    static ENTERED: AtomicUsize = AtomicUsize::new(0);
    static LEFT: AtomicUsize = AtomicUsize::new(0);

    fn entered() -> io::Result<Terminal<TestBackend>> {
        ENTERED.fetch_add(1, Ordering::SeqCst);
        Ok(Terminal::new(TestBackend::new(80, 24)).unwrap())
    }

    fn left() {
        LEFT.fetch_add(1, Ordering::SeqCst);
    }

    /// One test owns the counters: they are process-wide.
    #[test]
    fn a_scan_that_outlasts_the_threshold_gets_the_logo_and_a_quick_one_never_does() {
        let quick = Splash::with(Theme::neon(), || (80, 24), entered, left);
        let mut quick = quick;
        assert_eq!(quick.tick("scanning"), Tick::Quiet);
        assert!(!quick.finish());
        assert_eq!(ENTERED.load(Ordering::SeqCst), 0, "a quick scan was drawn");

        let mut slow =
            Splash::with(Theme::neon(), || (80, 24), entered, left).after(Duration::from_millis(5));
        assert_eq!(slow.tick("scanning"), Tick::Quiet, "before the threshold");
        std::thread::sleep(Duration::from_millis(10));
        assert_eq!(slow.tick("scanning · 5 entries"), Tick::Shown);
        assert!(slow.shown());
        let buf = slow.terminal.as_ref().unwrap().backend().buffer().clone();
        assert!(!drawn(&buf).is_empty() && text(&buf, 20).contains("5 entries"));
        // Entered once, however many ticks follow.
        assert_eq!(slow.tick("scanning · 6 entries"), Tick::Shown);
        assert_eq!(ENTERED.load(Ordering::SeqCst), 1);
        // The dashboard replaces it: the terminal is given back, once.
        assert!(slow.finish());
        assert!(!slow.shown() && !slow.finish());
        assert_eq!(LEFT.load(Ordering::SeqCst), 1);

        // Ctrl-C is a key in raw mode: the tick says so and leaves the
        // terminal to the caller, who is about to end the run.
        let mut stopped = Splash::with(Theme::neon(), || (80, 24), entered, left)
            .after(Duration::ZERO)
            .keys(|| true);
        assert_eq!(stopped.tick("scanning"), Tick::Interrupted);
        assert!(stopped.shown());
        assert!(stopped.finish());
        ENTERED.store(1, Ordering::SeqCst);

        // Too small: the scan stays a line of text, however long it runs.
        let mut small =
            Splash::with(Theme::neon(), || (79, 24), entered, left).after(Duration::ZERO);
        assert_eq!(small.tick("scanning"), Tick::Quiet);
        assert_eq!(ENTERED.load(Ordering::SeqCst), 1);
    }
}

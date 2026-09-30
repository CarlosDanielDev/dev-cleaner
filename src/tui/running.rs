//! The run, while it is under way: one row per item, moving as each one does.
//!
//! The purge runs on a thread of its own and reports each item over a channel,
//! so the loop keeps drawing and reading keys for the whole of the run. Nothing
//! here decides what is removed: the thread was handed a `Plan<Confirmed>` and
//! only `execute_with` can consume one. What this module keeps is the picture
//! of the run so far, and it is redrawn from that on every frame.
//!
//! Not a router stage. The router knows a plan and a finished record and
//! nothing between them; the run is a fact about the loop.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;

use super::palette::{BLOCKED, DEFAULT, HEAD, SAFE};
use super::projects::truncate;
use super::row::{columns, describe, elide_path, elide_tail};
use super::window_start;
use crate::bytes::human;
use crate::purge::{Manifest, Outcome, PurgeItem, Remover, execute_with, write_manifest};
use crate::safety::{Confirmed, Plan};

/// What the thread is called, so a panic hook can tell its panics from the
/// loop's own.
pub const PURGE_THREAD: &str = "dev-cleaner-purge";

/// One frame of the spinner per tick.
///
/// Decoration only: the count and the elapsed time carry the same fact in
/// words, and a terminal that draws none of these still reads correctly.
const SPINNER: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

/// What the thread tells the loop about each item: the run's stamp, and the
/// item as the record holds it.
type Progress = (SystemTime, PurgeItem);

/// One entry of the plan, as it was when the run began.
#[derive(Debug)]
struct Planned {
    path: PathBuf,
    bytes: u64,
    command: String,
}

/// The run in progress.
#[derive(Debug)]
pub(super) struct Running {
    started: Instant,
    now: Instant,
    phase: usize,
    planned: Vec<Planned>,
    /// Items whose outcome has arrived, in plan order.
    done: Vec<PurgeItem>,
    /// The stamp the record carries, learnt from the first item to arrive.
    executed_at: Option<SystemTime>,
    stream: Receiver<Progress>,
    closed: bool,
    worker: JoinHandle<Manifest>,
    freed_immediately: bool,
    /// Free space before anything moved, and where it was measured.
    pub before: Option<u64>,
    pub measure_at: PathBuf,
}

impl Running {
    /// Begin the run on its own thread.
    ///
    /// The record is written from the thread, after every item and before the
    /// item is announced, so it is on disk whatever the loop is doing and by the
    /// time the loop hears of an item.
    ///
    /// ponytail: a thread that cannot be started panics, before anything has
    /// moved. Report it as a run that ended early if that ever happens.
    pub fn spawn(
        plan: Plan<Confirmed>,
        remover: Box<dyn Remover + Send>,
        dir: PathBuf,
        before: Option<u64>,
        measure_at: PathBuf,
        now: Instant,
    ) -> Self {
        let planned = plan
            .items()
            .iter()
            .map(|c| Planned {
                path: c.path.clone(),
                bytes: c.bytes,
                command: describe(&c.safety),
            })
            .collect();
        let freed_immediately = remover.frees_space_immediately();
        let (tx, stream) = mpsc::channel();
        let worker = thread::Builder::new()
            .name(PURGE_THREAD.to_string())
            .spawn(move || {
                execute_with(plan, &*remover, &mut |record| {
                    let _ = write_manifest(record, &dir);
                    if let Some(item) = record.items.last() {
                        // The loop is gone only if the interface is, and then
                        // there is nobody to tell.
                        let _ = tx.send((record.executed_at, item.clone()));
                    }
                })
            })
            .expect("the purge thread starts");
        Self {
            started: now,
            now,
            phase: 0,
            planned,
            done: Vec::new(),
            executed_at: None,
            stream,
            closed: false,
            worker,
            freed_immediately,
            before,
            measure_at,
        }
    }

    /// Take in whatever the thread has reported, and turn the spinner.
    pub fn advance(&mut self, now: Instant) {
        self.now = now;
        self.phase = self.phase.wrapping_add(1);
        loop {
            match self.stream.try_recv() {
                Ok((executed_at, item)) => {
                    self.executed_at = Some(executed_at);
                    self.done.push(item);
                }
                Err(TryRecvError::Empty) => break,
                // Reported only after every queued item has been read.
                Err(TryRecvError::Disconnected) => {
                    self.closed = true;
                    break;
                }
            }
        }
    }

    /// Whether the thread has stopped, by finishing or by panicking.
    pub fn is_over(&self) -> bool {
        self.closed
    }

    /// The record, and why the run ended early if it did.
    ///
    /// A worker that panicked leaves no manifest of its own, so the record is
    /// rebuilt from the items that were reported. The item in flight when it
    /// stopped is not among them, and the note says so.
    pub fn finish(self) -> (Manifest, Option<String>) {
        let total = self.planned.len();
        match self.worker.join() {
            Ok(manifest) => (manifest, None),
            Err(payload) => {
                let why = payload
                    .downcast_ref::<&str>()
                    .map(ToString::to_string)
                    .or_else(|| payload.downcast_ref::<String>().cloned())
                    .unwrap_or_else(|| "no reason given".to_string());
                let note = format!(
                    "The run ended early: it stopped after {} of {total} items ({why}). \
                     The rest were not attempted. Whatever was in flight when it stopped is not \
                     in the record; look for it in the Trash.",
                    self.done.len()
                );
                let manifest = Manifest {
                    executed_at: self.executed_at.unwrap_or_else(SystemTime::now),
                    bytes_expected: self.planned.iter().map(|p| p.bytes).sum(),
                    items: self.done,
                    bytes_actual: None,
                    freed_immediately: self.freed_immediately,
                };
                (manifest, Some(note))
            }
        }
    }

    /// Draw the run into `area`: the running line, then one row per item.
    ///
    /// The list follows the item in flight the way the candidates window
    /// follows the cursor, so the row being worked on is always in view.
    pub fn render(&self, area: Rect, buf: &mut Buffer) {
        if area.is_empty() {
            return;
        }
        let left = area.x + 1;
        let width = area.width.saturating_sub(2) as usize;

        let moved = self
            .done
            .iter()
            .filter(|i| matches!(i.result, Outcome::Removed { .. }));
        let moved_bytes: u64 = moved.map(|i| i.bytes).sum();
        let failed = self
            .done
            .iter()
            .filter(|i| matches!(i.result, Outcome::Failed { .. }))
            .count();
        let elapsed = self.now.saturating_duration_since(self.started);
        let in_flight = self
            .planned
            .get(self.done.len())
            .map(|p| elide_path(&p.path.display().to_string(), 32))
            .unwrap_or_default();
        let line = format!(
            "Purging  {} of {}{}   {} moved   {}   {} {in_flight}",
            self.done.len(),
            self.planned.len(),
            if failed > 0 {
                format!(" ({failed} failed)")
            } else {
                String::new()
            },
            human(moved_bytes),
            seconds(elapsed),
            SPINNER[self.phase % SPINNER.len()],
        );
        buf.set_string(left, area.y, truncate(&line, width), HEAD);

        let top = area.y.saturating_add(2);
        let height = area.bottom().saturating_sub(top) as usize;
        let len = self.planned.len();
        let cursor = self.done.len().min(len.saturating_sub(1));
        let start = window_start(cursor, len, height);
        let shown = &self.planned[start..(start + height).min(len)];

        let right: Vec<String> = shown
            .iter()
            .enumerate()
            .map(|(i, p)| match self.done.get(start + i) {
                Some(PurgeItem {
                    result: Outcome::Failed { error },
                    ..
                }) => error.clone(),
                _ => p.command.clone(),
            })
            .collect();
        let (path_w, right_x, right_w) = columns(left, width, 14, &right);
        for (i, (p, right)) in shown.iter().zip(&right).enumerate() {
            let y = top + i as u16;
            let (glyph, style, size) = match self.done.get(start + i) {
                None => ('·', DEFAULT, String::new()),
                Some(item) => match item.result {
                    Outcome::Removed { .. } => ('+', SAFE, human(item.bytes)),
                    Outcome::Failed { .. } => ('!', BLOCKED, human(item.bytes)),
                },
            };
            buf.set_string(left, y, glyph.to_string(), style);
            buf.set_string(left + 2, y, format!("{size:>10}"), DEFAULT);
            buf.set_string(
                left + 14,
                y,
                elide_path(&p.path.display().to_string(), path_w),
                DEFAULT,
            );
            let right_style: Style = if matches!(glyph, '!') {
                BLOCKED
            } else {
                DEFAULT
            };
            buf.set_string(right_x, y, elide_tail(right, right_w), right_style);
        }
    }
}

/// Elapsed time to a tenth of a second, which is as fine as a tick can tell.
fn seconds(elapsed: Duration) -> String {
    format!("{:.1} s", elapsed.as_secs_f32())
}

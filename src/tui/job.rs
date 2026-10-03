//! The scan, on a thread of its own.
//!
//! The interface never calls into the walk and never waits on it: it holds the
//! counters the scan shares and a channel the scan answers on once, when it is
//! over. Everything between is the counters, which cost the walk a relaxed
//! add apiece and cost the interface a read per frame, however many entries
//! there are.

use std::any::Any;
use std::io;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;

use super::data::Screens;
use crate::scan::Progress;

/// What the scan thread was called, for the panic message that names it.
pub const SCAN_THREAD: &str = "dev-cleaner-scan";

/// How a scan ended.
pub enum Finished {
    /// It ran to its end and built the screens.
    Done(Box<Screens>),
    /// It was told to stop, and built nothing.
    Cancelled,
    /// It panicked. The caller gives the terminal back and hands the panic on.
    Panicked(Box<dyn Any + Send>),
}

/// A scan in flight.
pub struct ScanJob {
    progress: Arc<Progress>,
    answer: Receiver<thread::Result<Option<Screens>>>,
}

impl ScanJob {
    /// Run `work` on its own thread, counting into the progress it is handed.
    pub fn spawn(
        work: impl FnOnce(&Arc<Progress>) -> Option<Screens> + Send + 'static,
    ) -> io::Result<Self> {
        let progress = Arc::new(Progress::default());
        let (send, answer) = mpsc::channel();
        let theirs = Arc::clone(&progress);
        thread::Builder::new()
            .name(SCAN_THREAD.to_string())
            .spawn(move || {
                // Caught so the caller hears of a panic from the channel, in
                // order, rather than from a thread that is simply gone.
                let outcome = catch_unwind(AssertUnwindSafe(|| work(&theirs)));
                let _ = send.send(outcome);
            })?;
        Ok(Self { progress, answer })
    }

    /// The counters the scan is bumping.
    pub fn progress(&self) -> &Arc<Progress> {
        &self.progress
    }

    /// Ask the scan to stop.
    pub fn cancel(&self) {
        self.progress.cancel();
    }

    /// How it ended, if it has. Never waits.
    pub fn poll(&self) -> Option<Finished> {
        match self.answer.try_recv() {
            Ok(Ok(Some(screens))) => Some(Finished::Done(Box::new(screens))),
            Ok(Ok(None)) => Some(Finished::Cancelled),
            Ok(Err(panic)) => Some(Finished::Panicked(panic)),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Finished::Panicked(Box::new(
                "the scan thread ended without an answer",
            ))),
        }
    }
}

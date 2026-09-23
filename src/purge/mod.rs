//! Carrying out a confirmed plan, reversibly, and recording what happened.

mod execute;
mod manifest;

pub use execute::{Manifest, Outcome, PurgeItem, Remover, TrashRemover, execute, free_bytes};
pub use manifest::{manifest_dir, restore_steps, shortfall_note, trash_note, write_manifest};

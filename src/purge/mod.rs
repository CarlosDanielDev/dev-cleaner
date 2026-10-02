//! Carrying out a confirmed plan, reversibly, and recording what happened.

mod execute;
mod manifest;

pub use execute::{
    Manifest, Outcome, PurgeItem, Remover, TrashRemover, execute, execute_with, free_bytes,
};
pub use manifest::{
    execute_and_record, manifest_dir, not_attempted_note, restore_steps, shortfall_note, took,
    trash_note, write_manifest,
};

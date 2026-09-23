//! dev-cleaner: map developer project folders and recover disk space safely.

pub mod bytes;
pub mod candidates;
pub mod classify;
pub mod cli;
pub mod config;
pub mod duplicates;
pub mod purge;
pub mod safety;
pub mod scan;
pub mod shared_store;
pub mod store;
pub mod tui;
pub mod volume;

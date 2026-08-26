//! Recording a scan and reporting what moved since the last one.
//!
//! Part of the binary rather than the library: it decides where the database
//! lives and what gets printed, which are the command's concerns, not the
//! store's.

use dev_cleaner::bytes::human;
use dev_cleaner::store::{Change, Snapshot, Store, db_path};

/// Record the scan and show what moved since the last one.
///
/// A store that will not open is reported and stepped over. The scan's answer
/// is already on screen and is correct; losing the history is worth a warning,
/// not the loss of a walk the user just waited for.
pub fn remember(snap: &Snapshot) {
    let mut store = match Store::open(&db_path()) {
        Ok(store) => store,
        Err(err) => return eprintln!("\nhistory unavailable, this scan was not recorded: {err}"),
    };
    // The baseline has to be a scan of the same roots. Against a scan of a
    // wider root set, every path outside this one reads as removed, which is
    // the tool reporting deletions that never happened.
    let previous = store.latest_scan_for(&snap.roots).unwrap_or(None);
    let current = match store.write_snapshot(snap) {
        Ok(id) => id,
        Err(err) => return eprintln!("\nthis scan could not be recorded: {err}"),
    };

    let Some(previous) = previous else {
        println!(
            "\nRecorded as the first scan of these roots. Run again later to see what changed."
        );
        return;
    };
    let rows = match store.trend(previous, current) {
        Ok(rows) => rows,
        Err(err) => return eprintln!("\ncould not compare against the previous scan: {err}"),
    };
    let moved: Vec<_> = rows
        .iter()
        .filter(|r| r.change != Change::Unchanged)
        .collect();

    println!("\nsince the previous scan");
    if moved.is_empty() {
        println!("  nothing changed");
        return;
    }
    for row in moved.iter().take(10) {
        println!(
            "  {:>10}  {:<12} {}",
            human(row.bytes),
            row.change.describe(),
            row.path.display()
        );
    }
    if moved.len() > 10 {
        println!("  ... and {} more", moved.len() - 10);
    }
}

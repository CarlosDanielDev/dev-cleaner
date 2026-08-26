use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use super::{Manifest, Outcome};

impl Manifest {
    /// The record, as the file that gets written.
    ///
    /// Self-contained by design. Someone opening this months from now will not
    /// have the terminal session that produced it, so the restore instructions
    /// live in the document rather than in the UI that created it.
    pub fn render(&self) -> String {
        let stamp = self
            .executed_at
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let mut out = String::new();
        out.push_str("# dev-cleaner purge record\n\n");
        out.push_str(&format!("Unix time: {stamp}\n"));
        out.push_str(&format!(
            "Outcome: {}\n\n",
            if self.is_complete() {
                "every item moved".to_string()
            } else {
                format!(
                    "{} moved, {} failed",
                    self.removed().count(),
                    self.failed().count()
                )
            }
        ));

        out.push_str("## Moved to Trash\n\n");
        out.push_str("| path | size | regenerate with | moved to |\n");
        out.push_str("|---|---:|---|---|\n");
        for item in self.removed() {
            let dest = match &item.result {
                Outcome::Removed { trashed_to } => trashed_to.display().to_string(),
                Outcome::Failed { .. } => unreachable!("filtered to removed"),
            };
            out.push_str(&format!(
                "| {} | {} | {} | {} |\n",
                item.path.display(),
                human(item.bytes),
                item.regen,
                dest
            ));
        }

        if !self.is_complete() {
            out.push_str("\n## Not moved\n\n");
            out.push_str("| path | size | reason |\n|---|---:|---|\n");
            for item in self.failed() {
                let why = match &item.result {
                    Outcome::Failed { error } => error.as_str(),
                    Outcome::Removed { .. } => unreachable!("filtered to failed"),
                };
                out.push_str(&format!(
                    "| {} | {} | {} |\n",
                    item.path.display(),
                    human(item.bytes),
                    why
                ));
            }
            out.push_str("\nThese are untouched and still on disk.\n");
        }

        out.push_str("\n## Space\n\n");
        out.push_str(&format!("- Planned: {}\n", human(self.bytes_expected)));
        out.push_str(&format!("- Moved: {}\n", human(self.bytes_moved())));

        if self.freed_immediately {
            match self.bytes_actual {
                Some(actual) => {
                    out.push_str(&format!("- Reclaimed on disk: {}\n", human(actual)));
                    if let Some(gap) = self.shortfall() {
                        out.push_str(&format!(
                            "\nThe disk returned {:.0}% less than was moved. That usually means \
                             hardlinked content whose inodes are still referenced elsewhere, or a \
                             sparse file whose host has not released its blocks yet.\n",
                            gap * 100.0
                        ));
                    }
                }
                None => out.push_str("- Reclaimed on disk: not measured\n"),
            }
        } else {
            out.push_str(&format!(
                "- Waiting in the Trash: {}\n",
                human(self.pending_in_trash())
            ));
            out.push_str(
                "\nFree space has not changed yet, and that is expected. The Trash is on the \
                 same disk, so nothing is reclaimed until you empty it.\n",
            );
        }

        out.push_str("\n## Restore\n\n");
        out.push_str(
            "Everything listed above was moved to the Trash, not deleted. To restore an \
             entry, open the Trash in Finder, right-click it and choose \"Put Back\".\n\n\
             The space is not actually reclaimed until you empty the Trash. Until then \
             these files still occupy the disk, and every one of them remains recoverable.\n\n\
             Once you empty the Trash, anything listed here can still be rebuilt with the \
             command shown in its row.\n",
        );
        out
    }
}

/// Where records are kept.
///
/// Deliberately outside every scanned root and every registered cache: a record
/// the tool could later clean up is not a record. A test asserts this against
/// the cache registry so a newly registered cache cannot start shadowing it.
pub fn manifest_dir() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/".into()))
        .join(".local/state/dev-cleaner/manifests")
}

/// Write the record, returning where it landed.
///
/// Called even when the run failed part way. A partial run is exactly when the
/// user most needs to know what happened.
pub fn write_manifest(manifest: &Manifest, dir: &Path) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let stamp = manifest
        .executed_at
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let path = dir.join(format!("purge-{stamp}.md"));
    std::fs::write(&path, manifest.render())?;
    Ok(path)
}

fn human(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.2} {}", UNITS[unit])
    }
}

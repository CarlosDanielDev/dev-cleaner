use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::{Duration, UNIX_EPOCH};

use super::{Manifest, Outcome, PurgeItem, Remover, execute_with};
use crate::bytes::human;
use crate::safety::{Confirmed, Plan};

/// How long a run took, to the precision it was measured at.
///
/// Milliseconds under a second, because a quick run shown as `0.0 s` reads as
/// a clock that was never read; tenths of a second above it.
pub fn took(elapsed: Duration) -> String {
    if elapsed < Duration::from_secs(1) {
        format!("{} ms", elapsed.as_millis())
    } else {
        format!("{:.1} s", elapsed.as_secs_f32())
    }
}

/// Why a trashed run shows no free space.
///
/// Shared with the result screen rather than written out on both. The first
/// end-to-end run of this tool reported the Trash still holding the bytes as a
/// 97% shortfall; the sentence that corrects it is worth exactly one copy.
pub fn trash_note() -> &'static str {
    "Free space has not changed yet, and that is expected. The Trash is on the \
     same disk, so nothing is reclaimed until you empty it."
}

/// Why the disk returned materially less than was removed.
///
/// Only ever reached through [`Manifest::shortfall`], which answers `None` for
/// a run through the Trash. Nothing else should be phrasing this.
pub fn shortfall_note(gap: f64) -> String {
    format!(
        "The disk returned {:.0}% less than was moved. That usually means \
         hardlinked content whose inodes are still referenced elsewhere, or a \
         sparse file whose host has not released its blocks yet.",
        gap * 100.0
    )
}

/// What a stopped run left alone.
///
/// Shared with the result screen for the same reason as [`trash_note`]. Nothing
/// went wrong, so it says what is true of these entries and what happens to
/// them next, not why they failed.
pub fn not_attempted_note() -> &'static str {
    "These are untouched and still on disk. They will be offered again by the next scan."
}

/// How to put everything back, one paragraph per step.
///
/// A list rather than a block so the record can join them with blank lines and
/// a terminal can wrap them to its own width, without either one owning the
/// words. Emptying the Trash is a step here because it is the point at which
/// this stops being reversible.
///
/// `freed_immediately` because a sanctioned cleanup command deletes rather than
/// trashes: telling someone to recover from the Trash something that never went
/// there is the same error as reporting a prediction as a result, and the
/// instruction would fail in front of them.
pub fn restore_steps(freed_immediately: bool) -> &'static [&'static str] {
    if freed_immediately {
        return &[
            "These were removed outright by the cleanup command that owns them rather than \
             moved to the Trash, so the space is already back and there is nothing to recover.",
            "Every entry above rebuilds with the command shown in its row.",
        ];
    }
    &[
        "Everything listed above was moved to the Trash, not deleted. To restore an \
         entry, open the Trash in Finder, right-click it and choose \"Put Back\".",
        "The space is not actually reclaimed until you empty the Trash. Until then \
         these files still occupy the disk, and every one of them remains recoverable.",
        "Once you empty the Trash, anything listed here can still be rebuilt with the \
         command shown in its row.",
    ]
}

impl Manifest {
    /// How the run came out, in the words the screen and the record share.
    ///
    /// Failures and skipped items appear only when there are some: a stop is
    /// not a failure, and a failure count of zero beside it would suggest the
    /// two were being added together.
    pub fn tally(&self) -> String {
        let mut parts = vec![format!("{} moved", self.removed().count())];
        let failed = self.failed().count();
        if failed > 0 || self.skipped().next().is_none() {
            parts.push(format!("{failed} failed"));
        }
        let skipped = self.skipped().count();
        if skipped > 0 {
            parts.push(format!("{skipped} not attempted"));
        }
        parts.join(", ")
    }

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
            "Outcome: {}\n",
            if self.is_complete() {
                "every item moved".to_string()
            } else {
                self.tally()
            }
        ));
        out.push_str(&format!("Elapsed: {}\n\n", took(self.elapsed)));

        out.push_str("## Moved to Trash\n\n");
        out.push_str("| path | size | regenerate with | moved to |\n");
        out.push_str("|---|---:|---|---|\n");
        for item in self.removed() {
            let dest = match &item.result {
                Outcome::Removed { trashed_to } => trashed_to.display().to_string(),
                Outcome::Failed { .. } | Outcome::Skipped => unreachable!("filtered to removed"),
            };
            out.push_str(&format!(
                "| {} | {} | {} | {} |\n",
                item.path.display(),
                human(item.bytes),
                item.regen,
                dest
            ));
        }

        if self.failed().next().is_some() {
            out.push_str("\n## Not moved\n\n");
            out.push_str("| path | size | reason |\n|---|---:|---|\n");
            for item in self.failed() {
                let why = match &item.result {
                    Outcome::Failed { error } => error.as_str(),
                    Outcome::Removed { .. } | Outcome::Skipped => {
                        unreachable!("filtered to failed")
                    }
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

        if self.skipped().next().is_some() {
            out.push_str("\n## Not attempted\n\n");
            out.push_str("| path | size |\n|---|---:|\n");
            for item in self.skipped() {
                out.push_str(&format!(
                    "| {} | {} |\n",
                    item.path.display(),
                    human(item.bytes)
                ));
            }
            out.push_str(&format!("\n{}\n", not_attempted_note()));
        }

        out.push_str("\n## Space\n\n");
        out.push_str(&format!("- Planned: {}\n", human(self.bytes_expected)));
        out.push_str(&format!("- Moved: {}\n", human(self.bytes_moved())));

        if self.freed_immediately {
            match self.bytes_actual {
                Some(actual) => {
                    out.push_str(&format!("- Reclaimed on disk: {}\n", human(actual)));
                    if let Some(gap) = self.shortfall() {
                        out.push_str(&format!("\n{}\n", shortfall_note(gap)));
                    }
                }
                None => out.push_str("- Reclaimed on disk: not measured\n"),
            }
        } else {
            out.push_str(&format!(
                "- Waiting in the Trash: {}\n",
                human(self.pending_in_trash())
            ));
            out.push_str(&format!("\n{}\n", trash_note()));
        }

        out.push_str("\n## Restore\n\n");
        out.push_str(&restore_steps(self.freed_immediately).join("\n\n"));
        out.push('\n');
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

/// Carry out a plan the way the command line does: one line per item as it
/// moves, and the record rewritten after every one.
///
/// The record is on disk from the first item on, so closing the terminal
/// halfway through leaves a file naming what had already gone to the Trash.
/// Each write here is best effort; the caller's final write, after measuring
/// the disk, is the one that reports a failure to write.
pub fn execute_and_record(
    plan: Plan<Confirmed>,
    remover: &dyn Remover,
    dir: &Path,
    say: &mut dyn FnMut(&str),
) -> Manifest {
    execute_with(plan, remover, &AtomicBool::new(false), &mut |record| {
        let _ = write_manifest(record, dir);
        if let Some(item) = record.items.last() {
            say(&item_line(item));
        }
    })
}

/// What happened to one item, as the command line prints it.
fn item_line(item: &PurgeItem) -> String {
    let size = human(item.bytes);
    match &item.result {
        Outcome::Removed { .. } => format!("  moved   {size:>10}  {}", item.path.display()),
        Outcome::Failed { error } => {
            format!("  failed  {size:>10}  {}: {error}", item.path.display())
        }
        Outcome::Skipped => format!("  skipped {size:>10}  {}", item.path.display()),
    }
}

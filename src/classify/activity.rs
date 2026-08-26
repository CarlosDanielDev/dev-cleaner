use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const ACTIVE_WITHIN: Duration = Duration::from_secs(30 * 86_400);
const DORMANT_WITHIN: Duration = Duration::from_secs(180 * 86_400);

/// How recently a project was worked on, and whether losing it would lose work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Activity {
    /// Touched within the last 30 days. Only build artifacts may be offered.
    Active,
    /// Idle between 30 and 180 days. Artifacts may be offered, flagged.
    Dormant,
    /// Idle beyond 180 days, with every commit present on a remote. The whole
    /// project may be offered, because `git clone` provably restores it.
    Dead,
}

impl Activity {
    /// Classify `root`, given the newest source-file modification time the
    /// scanner observed there.
    pub fn of(root: &Path, newest_source: Option<SystemTime>, now: SystemTime) -> Self {
        let git = GitFacts::read(root);

        // Take the most recent evidence of work from either source. Overstating
        // recency is the safe direction: it can only move a project away from
        // Dead, never towards it.
        let last = [git.last_activity, newest_source]
            .into_iter()
            .flatten()
            .max();

        let idle = last
            .and_then(|t| now.duration_since(t).ok())
            .unwrap_or(DORMANT_WITHIN);

        if idle < ACTIVE_WITHIN {
            return Activity::Active;
        }
        // Dead requires proof the work survives elsewhere. Without a remote
        // that already contains HEAD, deleting the project destroys it.
        if idle >= DORMANT_WITHIN && git.head_pushed {
            return Activity::Dead;
        }
        Activity::Dormant
    }
}

/// When the repository at `root` was last touched, from its reflog.
///
/// `None` where there is no repository, or none the reflog can date. Shared
/// with [`Activity`] rather than read separately, so the stored history and the
/// activity classification can never disagree about the same repository.
pub fn last_activity(root: &Path) -> Option<SystemTime> {
    GitFacts::read(root).last_activity
}

/// The facts we need from a repository, read directly rather than through a
/// git library or a subprocess.
#[derive(Debug, Default)]
struct GitFacts {
    last_activity: Option<SystemTime>,
    head_pushed: bool,
}

impl GitFacts {
    fn read(root: &Path) -> Self {
        let git = root.join(".git");
        if !git.exists() {
            return Self::default();
        }
        Self {
            last_activity: reflog_mtime(&git),
            head_pushed: head_is_on_a_remote(&git),
        }
    }
}

/// Timestamp of the last entry in `.git/logs/HEAD`.
///
/// The reflog is plain text, so this avoids inflating commit objects and
/// walking packfiles for a single date.
///
/// ponytail: reflogs can be pruned by `git gc`, and a repository created with
/// `core.logAllRefUpdates=false` has none. Both cases fall through to the
/// scanner's source mtime, which reads newer than reality and therefore only
/// ever moves a project away from Dead. Parse commit objects if a sharper date
/// is ever needed.
fn reflog_mtime(git: &Path) -> Option<SystemTime> {
    let text = std::fs::read_to_string(git.join("logs/HEAD")).ok()?;
    let last = text.lines().rfind(|l| !l.trim().is_empty())?;

    // "<old> <new> <name> <email> <unix-ts> <tz>\t<message>"
    let before_tab = last.split('\t').next()?;
    let secs: u64 = before_tab.split_whitespace().rev().nth(1)?.parse().ok()?;
    Some(UNIX_EPOCH + Duration::from_secs(secs))
}

/// Whether HEAD's commit is already present under `refs/remotes`.
///
/// ponytail: compares HEAD against remote-tracking refs by exact object id
/// rather than testing ancestry. A HEAD that is an *ancestor* of a remote ref
/// therefore reads as not-pushed. That errs towards keeping the project, which
/// is the direction a deletion tool should err. Walk the commit graph if the
/// false negatives ever matter.
fn head_is_on_a_remote(git: &Path) -> bool {
    let Some(head) = resolve_head(git) else {
        return false;
    };
    remote_ref_ids(git).any(|id| id == head)
}

fn resolve_head(git: &Path) -> Option<String> {
    let head = std::fs::read_to_string(git.join("HEAD")).ok()?;
    let head = head.trim();
    match head.strip_prefix("ref: ") {
        // A symbolic HEAD: follow it to the branch it names.
        Some(r) => read_ref(git, r),
        // A detached HEAD already holds the object id.
        None => Some(head.to_string()),
    }
}

fn read_ref(git: &Path, name: &str) -> Option<String> {
    if let Ok(s) = std::fs::read_to_string(git.join(name)) {
        return Some(s.trim().to_string());
    }
    // Refs are packed away once `git pack-refs` has run.
    let packed = std::fs::read_to_string(git.join("packed-refs")).ok()?;
    packed.lines().find_map(|line| {
        let (id, r) = line.split_once(' ')?;
        (r.trim() == name).then(|| id.trim().to_string())
    })
}

fn remote_ref_ids(git: &Path) -> impl Iterator<Item = String> {
    let mut ids = Vec::new();

    let loose = git.join("refs/remotes");
    if loose.exists() {
        let walker = jwalk::WalkDir::new(&loose)
            .skip_hidden(false)
            .follow_links(false);
        for entry in walker.into_iter().flatten() {
            if entry.file_type().is_file()
                && let Ok(s) = std::fs::read_to_string(entry.path())
            {
                ids.push(s.trim().to_string());
            }
        }
    }

    if let Ok(packed) = std::fs::read_to_string(git.join("packed-refs")) {
        for line in packed.lines() {
            if let Some((id, r)) = line.split_once(' ')
                && r.trim().starts_with("refs/remotes/")
            {
                ids.push(id.trim().to_string());
            }
        }
    }
    ids.into_iter()
}

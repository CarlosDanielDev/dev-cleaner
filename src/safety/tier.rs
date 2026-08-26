/// A command that regenerates a deleted directory.
///
/// The inner string is private and the only constructor rejects empty input,
/// so a `Regenerable` candidate cannot exist without a real command behind it.
/// That property is what the selectable tiers rest on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegenCommand(String);

impl RegenCommand {
    pub fn new(cmd: impl Into<String>) -> Option<Self> {
        let cmd = cmd.into();
        (!cmd.trim().is_empty()).then_some(Self(cmd))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for RegenCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Why a candidate can never be offered, whatever else concludes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockReason {
    DirtyWorktree,
    UntrackedSource,
    StashEntries,
    OutsideRoots,
    SymlinkEscape,
    Denylisted,
    DockerVolume,
}

impl BlockReason {
    pub fn all() -> [BlockReason; 7] {
        use BlockReason::*;
        [
            DirtyWorktree,
            UntrackedSource,
            StashEntries,
            OutsideRoots,
            SymlinkEscape,
            Denylisted,
            DockerVolume,
        ]
    }

    /// Plain language, shown next to the blocked entry. The user should never
    /// have to guess why something is unavailable.
    pub fn explain(&self) -> &'static str {
        use BlockReason::*;
        match self {
            DirtyWorktree => "Uncommitted changes are present in this repository.",
            UntrackedSource => "Untracked source files here exist nowhere else.",
            StashEntries => "Stashed work is present and would be lost.",
            OutsideRoots => "This path lies outside every configured root.",
            SymlinkEscape => "This is a symlink leading outside the configured roots.",
            Denylisted => "This path is on the denylist in your configuration.",
            DockerVolume => "Docker volumes hold data no command can rebuild.",
        }
    }
}

/// How much the tool can prove about recovering a candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Safety {
    /// Refills by itself the next time the toolchain runs.
    Cache { refills_on: &'static str },
    /// Rebuildable by a known command.
    Regenerable { regen: RegenCommand },
    /// Looks disposable, but nothing proves it. Inspect-only.
    Unproven { reason: String },
    /// Never offered under any circumstances.
    Protected { reason: BlockReason },
}

impl Safety {
    /// The fallback for anything the registries do not recognise.
    ///
    /// Defaulting to a selectable tier would mean guessing on the user's
    /// behalf, so the default is the tier that offers nothing.
    pub fn for_unknown(reason: impl Into<String>) -> Self {
        Safety::Unproven {
            reason: reason.into(),
        }
    }

    /// Whether the selection cursor may land on this candidate at all.
    ///
    /// Unproven and Protected entries are not disabled-but-focusable; the UI
    /// skips them entirely, so the wrong path cannot be reached by any sequence
    /// of keystrokes.
    pub fn is_selectable(&self) -> bool {
        matches!(self, Safety::Cache { .. } | Safety::Regenerable { .. })
    }

    /// A glyph distinguishing the tier without relying on colour.
    pub fn symbol(&self) -> char {
        match self {
            Safety::Cache { .. } => '~',
            Safety::Regenerable { .. } => '+',
            Safety::Unproven { .. } => '?',
            Safety::Protected { .. } => '!',
        }
    }
}

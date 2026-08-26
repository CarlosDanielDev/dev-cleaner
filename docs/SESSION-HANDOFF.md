# Session handoff

Last updated: 2026-08-26, after M2 merged.

## Where the project stands

| Milestone | State |
| --- | --- |
| M1 — Scan and see | 16 of 16 closed, merged in #43 |
| M2 — Prove and purge | 11 of 11 closed, merged in #44 |
| M3 — Remember and report | 15 open, not started |

`main` is at the M2 merge. No open PRs, no working branch.

89 tests. `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
`cargo test` and `cargo deny check` are all clean, locally and in CI.

## What works today

```
dev-cleaner scan [roots...]     # walk, classify, report. Always read-only.
dev-cleaner purge               # dry run: plan, blocked list, confirmation phrase
dev-cleaner purge --execute --confirm "<phrase>"
```

On the reference machine: 238 projects, 5.08 GB reclaimable across 88 artifact
directories, 128 candidates blocked with reasons shown.

## The rules this codebase holds to

These are not style preferences. Each one exists because breaking it caused a
real problem during M1 or M2.

**Nothing is deletable unless the tool can name the command that brings it
back.** `RegenCommand` keeps its string private and rejects empty input, and
registry constructors assert at compile time, so a kind without a regeneration
command fails the build rather than reaching a user.

**The walk never consults `.gitignore`.** The reclaimable bytes are exactly what
`.gitignore` hides. An `fd`-based scan during design reported zero `node_modules`
across 55 JavaScript projects for this reason.

**Sizes come from `st_blocks`, never `st_size`, and each inode counts once.**
`Docker.raw` measured 60 GB apparent against 34 GB actual. Hardlinked package
stores otherwise inflate every total.

**Guards ask about the path being deleted, not the whole repository.** For a
whole-project candidate the path is the repository root, so the check stays
repo-wide. Getting this wrong made the tool offer 0.64 GB where 5.08 GB was
available, with no gain in safety.

**Never report a prediction as a result.** The first end-to-end run claimed a
97% shortfall and blamed hardlinks for space the Trash was simply still holding.
Trashed bytes are reported as waiting in the Trash; free space is only presented
as reclaimed for removers that free it immediately.

**Deletion is reversible.** Everything routes through the `Remover` trait to the
macOS Trash. There is no `remove_dir_all` in `src/`.

## How the work has been done

Test first, always. Watch it fail for the right reason before implementing.

**Mutation-check every guard.** Remove the protection, confirm the test fails,
restore it. This caught a false-confidence test in M1: `unpushed_commits_are_never_dead`
passed while the pushed-check was mutated to always return true, because the
fixture's unpushed commit wrote a fresh reflog entry and the repo read as
recently active. It asserted the right thing for the wrong reason.

**Cross-check against reality, not just against tests.** Sizing was validated
against `du` (both 7.98 GB), free space against `df` (both 110.31 GB), and
project detection against an independent `find` (both 240, which corrected the
issue's stated criterion of 103 and surfaced two false positives under
`.pio/libdeps`).

**Run it for real before believing it.** Both of M2's significant corrections
came from executing the binary, not from the test suite.

One issue per unit of work, one PR per milestone, `Closes #N` in the commit.
Deviations from an issue's stated criteria get a comment on that issue
explaining what changed and why.

## Deviations already recorded

- **gix dropped** (#16): its current release depends on an unpublished crate.
  Last-activity comes from `.git/logs/HEAD`, pushed-state from `.git/refs` and
  `packed-refs`. Two file reads, no subprocess.
- **trybuild dropped** (#20): compares exact compiler stderr, which drifts
  between Rust versions. Built-in `compile_fail` doctests give the same
  guarantee, each paired with a companion that must compile so a typo cannot
  masquerade as a pass.
- **Empty module placeholders skipped** (#2): `store/` and `tui/` do not exist
  yet. Their absence is what proves the tool cannot do those things.
- **`git status` delegated to git** (#21): reimplementing it means the binary
  index plus gitignore semantics plus submodules, where a subtle bug reports
  clean and costs someone their uncommitted work. Any failure to get an answer
  blocks.

## M3, in dependency order

**Persistence first (#28)** — #29 schema and migrations, #30 snapshot round
trip, #31 trend diff. The TUI dashboard needs trends, so this comes before it.

**Then the TUI (#32)** — #33 state machine, #34 dashboard, #35 projects table,
#36 candidates screen, #37 plan review and hold-to-confirm, #38 result screen.
#36 and #37 carry the poka-yoke requirements: unsafe tiers must be unreachable
by the cursor rather than disabled, and no destructive action may bind to Enter,
Delete, Backspace, or to any global key.

**Duplicates last (#39)** — #40 lockfile parsing, #41 cross-project duplicate
report, #42 shared-store migration estimate. Independent of the other two.

The TUI screen flow must mirror the type-state plan rather than duplicate its
logic: back-navigation from review discards confirmation.

# Session handoff

Last updated: 2026-08-26, after M3 persistence.

## Where the project stands

| Milestone | State |
| --- | --- |
| M1 — Scan and see | 16 of 16 closed, merged in #43 |
| M2 — Prove and purge | 11 of 11 closed, merged in #44 |
| M3 — Remember and report | persistence done (#29, #30, #31); TUI and duplicates open |

Persistence is on `feat/m3-persistence`, not merged.

118 tests. `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
`cargo test` and `cargo deny check` are all clean locally.

## What works today

```
dev-cleaner scan [roots...]     # walk, classify, report, record. Always read-only.
dev-cleaner purge               # dry run: plan, blocked list, confirmation phrase
dev-cleaner purge --execute --confirm "<phrase>"
```

On the reference machine: 238 projects, 5.08 GB reclaimable across 88 artifact
directories, 128 candidates blocked with reasons shown.

Every scan is now written to `~/.local/state/dev-cleaner/history.sqlite3` and
compared against the last scan of the same roots. A second run prints only what
moved:

```
since the previous scan
     1.41 GB  -780.00 KB   /Users/carlos/projects/dev-cleaner/target
```

A store that will not open costs a warning, not the scan.

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

## Open findings

**#45: artifact totals overstate reclaimable space.** `report_artifacts` and
`candidates::from_scan` sum `bytes_actual` per file without deduplicating
inodes, so cargo's hardlinked build output is counted twice. The store, which
does deduplicate, agrees with `du -sk` exactly on every directory checked; the
report does not. `target` reads as 3.21 GB against 1.78 GB actual. This means
`purge` states a plan total the disk cannot return. Found by cross-checking the
new store against `du`, not by any test.

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
- **Empty module placeholders skipped** (#2): `tui/` does not exist yet. Its
  absence is what proves the tool cannot do that. `store/` was created when
  there was something to put in it.
- **`git status` delegated to git** (#21): reimplementing it means the binary
  index plus gitignore semantics plus submodules, where a subtle bug reports
  clean and costs someone their uncommitted work. Any failure to get an answer
  blocks.

## What persistence looks like

`Store::open` runs `MIGRATIONS` by index against `PRAGMA user_version`, so a
migration is never renumbered and never re-run. Three tables: `scan`, `project`
and `entry`, with `project` carrying a `scan_id` so a stored snapshot reproduces
what was true at the time rather than what is true now.

`store::snapshot(...)` turns a finished walk into a `Snapshot`. It measures each
artifact directory with `Usage::of`, which is why it disagrees with the report
(#45) and agrees with `du`.

`Store::trend(before, after)` diffs two scans by path into `New`, `Grew`,
`Shrank`, `Unchanged` and `Removed`. `Removed` is deliberately not a shrink to
zero: "0 B" reads as a directory that is still there and now empty.

The baseline is `latest_scan_for(roots)`, not `latest_scan()`. That came from
running the binary: scanning a narrower root set reported every directory
outside it as removed, when nothing had been deleted.

Two things the tests could not prove, recorded rather than papered over. The
concurrency test proves contention is handled — dropping the busy timeout fails
it every time — but no reachable interleaving exercises scan-id attribution,
which rests on `last_insert_rowid` being per-connection. And the query plan is
asserted with `EXPLAIN QUERY PLAN` rather than a stopwatch, because a wall-clock
bound passes on a small fixture and rots on a real machine.

## M3, what is left

**The TUI (#32)** — #33 state machine, #34 dashboard, #35 projects table,
#36 candidates screen, #37 plan review and hold-to-confirm, #38 result screen.
#36 and #37 carry the poka-yoke requirements: unsafe tiers must be unreachable
by the cursor rather than disabled, and no destructive action may bind to Enter,
Delete, Backspace, or to any global key.

**Duplicates last (#39)** — #40 lockfile parsing, #41 cross-project duplicate
report, #42 shared-store migration estimate. Independent of the other two.

The TUI screen flow must mirror the type-state plan rather than duplicate its
logic: back-navigation from review discards confirmation.

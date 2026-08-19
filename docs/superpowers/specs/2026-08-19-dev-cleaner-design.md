# dev-cleaner — Design

Date: 2026-08-19
Status: Approved
Author: Carlos Daniel

## Problem

A developer machine accumulates regenerable bytes faster than any human tracks
them. Build artifacts, dependency trees, toolchain caches and device symbol
bundles all reappear on every build, and none of them announce themselves. The
result is a disk that fills without an obvious culprit, a machine that lags, and
an owner who cannot tell which directories are safe to delete.

Existing tools each solve one slice. `dust` and `ncdu` rank directories by size
but understand nothing about what a directory *is*. `kondo` recognises build
artifacts but keeps no history and cannot tell an abandoned project from an
active one. None of them answer the question that actually blocks the user:
*which of these can I delete without losing work?*

## Evidence

Measured on the author's machine, 2026-08-19, before any cleanup:

| Metric | Value |
| --- | --- |
| Data volume | 349 GiB used, 24 GiB free, 94% capacity |
| Projects under `~/projects` | 103 (81 git repositories) |
| Build/dependency directories | 972 |
| Reclaimable inside projects | 3.04 GB |
| `Docker.raw` | 60 GB apparent, 34 GB actual |
| Xcode iOS DeviceSupport | 11.1 GB across two near-identical iOS versions |
| Xcode SwiftUI preview cache | 5.3 GB |
| Go module cache | 2.9 GB |
| Ecosystems present | JS 55, Rust 9, Ruby 9, Gradle 6, Python 4, Swift 2, Go 2, PHP 1 |

A manual Tier-1 clean executed against this design's rules recovered **41.8 GB**
with no data loss, taking free space from 27.4 GB to 69.2 GB. The full record is
in `docs/evidence/purge-manifest-2026-08-19.md`. That run is the acceptance
baseline: the tool must find at least what the manual pass found.

## Goals

1. Map every registered project root and known developer cache.
2. Classify each directory by what it is and whether it can be regenerated.
3. Report *unique* reclaimable bytes — never a number the user will not get back.
4. Make destructive mistakes structurally impossible, not merely discouraged.
5. Persist snapshots so regrowth and staleness are visible over time.

## Non-goals

Background daemons, filesystem event watching, scanning non-developer regions of
`$HOME`, content-hash deduplication, and automatic package-manager migration are
all out of scope for v1. The tool reports migration opportunities; it does not
perform them.

## Design pillar: mistake-proofing

The governing rule: **nothing is deletable unless the tool can name the exact
command that brings it back.** This inverts the usual burden. Rather than the
user guessing what is safe, the tool must prove safety or decline to offer the
action at all.

### Compile-time enforcement

Rust's type system carries the guarantee, so no future edit can bypass it:

```rust
struct Plan<S> { items: Vec<Candidate>, _state: PhantomData<S> }
struct Draft; struct Reviewed; struct Confirmed;

impl Plan<Draft>     { fn review(self)               -> Plan<Reviewed>          }
impl Plan<Reviewed>  { fn confirm(self, typed: &str) -> Result<Plan<Confirmed>> }
impl Plan<Confirmed> { fn execute(self)              -> Manifest                }
```

`execute()` exists only on `Plan<Confirmed>`. Code that deletes without passing
review and explicit confirmation does not compile. A `trybuild` compile-fail test
locks this in permanently.

### Safety tiers

```rust
enum Safety {
    Cache       { refills_on: &'static str },  // selectable
    Regenerable { regen_cmd: String },         // selectable
    Unproven    { reason: String },            // NOT selectable, inspect-only
    Protected   { reason: BlockReason },       // NOT selectable, hard block
}
```

`Unproven` and `Protected` entries are unreachable by the selection cursor. They
are not disabled-but-clickable; the wrong path does not exist in the UI.

Hard blocks, non-overridable:

- dirty git worktree
- untracked source files present
- stash entries present
- path outside every registered root
- symlink resolving outside registered roots
- denylist match

The Docker case validated this rule during the manual baseline: `docker system
df` advertised 1.38 GB of "reclaimable" volumes, but those volumes held
`astral-system_postgres_data` and `astral-system_qdrant_storage`. Volumes are
`Protected`. The tool never passes `--volumes`.

### Interaction guards

| Risk | Guard |
| --- | --- |
| Irreversibility | Move to macOS Trash via `NSFileManager.trashItem`, never `rm` |
| Blind deletion | Missing `regen_cmd` demotes an entry to `Unproven` |
| Key slip | No single keystroke destroys; mark, review, then hold-to-confirm |
| Wrong default | Dry-run is the default; execution is opt-in per run |
| Losing work | Dirty repositories are hard-blocked, not warned about |
| Scope escape | Symlinks are never followed out of registered roots |
| Silent damage | Every purge writes a dated manifest with restore instructions |
| Colour-blind users | Safety tier encoded as symbol *and* colour, never colour alone |

No destructive action binds to `Enter`, `Delete`, or `Backspace`, and none binds
globally. Execution exists only on the confirmation screen.

## Scanner

The walker is deliberately **gitignore-blind**. During evidence gathering, a
`fd`-based scan reported zero `node_modules` because `fd` honours `.gitignore` —
and the junk is precisely what `.gitignore` hides. Any gitignore-aware walker
silently reports nothing.

Two measurement rules prevent the tool from overstating what it can recover:

1. **Hardlinks.** pnpm and `uv` hardlink into a shared store. Naive traversal
   counts those bytes once per project, so deleting a hardlinked tree frees far
   less than reported. The scanner keeps a `(st_dev, st_ino)` seen-set and
   reports unique bytes.
2. **Sparse files and APFS clones.** `st_size` is not disk usage. `Docker.raw`
   measured 60 GB apparent against 34 GB actual. Sizing reads `st_blocks`.

Inode count is a first-class metric alongside bytes. The 972 artifact
directories hold on the order of a million small files, and Spotlight, Time
Machine and iCloud each walk all of them. Inode pressure explains the lag that
byte counts alone do not.

## Classification

Project detection keys off ecosystem markers: `package.json`, `Cargo.toml`,
`go.mod`, `pyproject.toml`, `Package.swift`, `build.gradle`, `Gemfile`,
`composer.json`.

Activity derives from git history and source mtime:

| Class | Rule | What is offered |
| --- | --- | --- |
| `active` | commit or source mtime under 30 days | artifacts only |
| `dormant` | 30–180 days | artifacts, flagged |
| `dead` | over 180 days, clean tree, pushed remote | artifacts **and the whole project** |

The `dead` row is the largest single opportunity in the measured corpus: 81 git
repositories, most dormant, each provably restorable because `regen_cmd` is
`git clone <remote>`. A project without a pushed remote is never `dead`.

## Duplicate reporting

Content-hashing a million files is expensive and mostly surfaces noise. The
duplicate that actually costs space is the same package at the same version
installed across many projects, which lockfiles already record. Reading them is
close to free:

```
react@18.2.0        x14 projects    ~118 MB duplicated
typescript@5.4.2    x22 projects    ~340 MB duplicated
  -> migrate to pnpm store: est. 1.1 GB recovered
```

v1 reports the opportunity. It does not rewrite projects.

## Persistence

```sql
scan(id, started_at, root_set, total_bytes, total_inodes)
project(id, path, kind, vcs_remote, last_commit_at, dirty)
entry(scan_id, project_id, path, kind, bytes_unique, inodes, safety, regen_cmd)
purge(id, scan_id, executed_at, bytes_expected, bytes_actual)
purge_item(purge_id, path, bytes, trashed_to)
```

Trends come from joining `entry` across scans by path, which is what turns a
one-shot cleaner into something that answers "what regrew since last month".

`bytes_expected` and `bytes_actual` are stored separately and reported
separately. Expected is never presented as actual.

## TUI

Screens, in order:

1. **Dashboard** — free-space gauge, total reclaimable, trend since last scan
2. **Projects** — sortable table: name, size, unique size, inodes, activity
3. **Candidates** — grouped by safety tier; only safe tiers are reachable
4. **Plan review** — every item with its regeneration command visible
5. **Confirm** — hold-to-arm on a key not adjacent to navigation
6. **Result** — actual versus expected freed, manifest path, restore steps

## Stack

| Concern | Choice |
| --- | --- |
| TUI | `ratatui` + `crossterm` |
| Directory walk | `jwalk` (parallel, rayon-backed) |
| Storage | `rusqlite`, bundled SQLite |
| Deletion | `trash` crate, routing to macOS Trash |
| Git state | `gix`, no subprocess |
| CLI | `clap` |

Single static binary, no runtime dependency. Target scan time for `~/projects`
is 2–4 seconds.

## Error handling

Partial failures do not abort a purge. Each item records its own outcome, and
the manifest reflects what actually moved. After execution the tool re-stats
every path to confirm removal and reports the true delta, including the case
where a sparse-file host does not return blocks immediately.

## Testing

- Fixture trees built in a tempdir, asserting unique-byte arithmetic against
  hardlinked and sparse inputs
- `trybuild` compile-fail test proving `Plan<Draft>::execute()` does not compile
- Guard tests: dirty repository blocked, symlink escape blocked, denylist honoured
- Golden test against the 2026-08-19 evidence baseline

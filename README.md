# dev-cleaner

A terminal UI that maps developer project folders, classifies what each
directory is, measures what can actually be recovered, and makes deleting the
wrong thing structurally impossible.

> Status: design approved, implementation not started. See
> [the design spec](docs/superpowers/specs/2026-08-19-dev-cleaner-design.md).

## Why

Build artifacts, dependency trees and toolchain caches regenerate on every
build and never announce themselves. The disk fills, the machine lags, and
there is no obvious culprit. Size-ranking tools show *what is big*. None of
them answer the question that actually blocks you: **which of these can I
delete without losing work?**

## The rule

Nothing is deletable unless the tool can name the exact command that brings it
back. Safety is proven, not assumed — and the proof is enforced by the
compiler:

```rust
impl Plan<Draft>     { fn review(self)               -> Plan<Reviewed>          }
impl Plan<Reviewed>  { fn confirm(self, typed: &str) -> Result<Plan<Confirmed>> }
impl Plan<Confirmed> { fn execute(self)              -> Manifest                }
```

`execute()` exists only on `Plan<Confirmed>`. Deleting without review and
explicit confirmation does not compile.

## What it does

- Walks registered project roots and known developer caches — **ignoring
  `.gitignore`**, because the reclaimable bytes are exactly what `.gitignore`
  hides
- Reports **unique** bytes, accounting for hardlinked package stores and sparse
  files, so the number shown is the number you get back
- Classifies projects as active, dormant, or dead from git history, and offers
  to remove a dead project entirely when `git clone` provably restores it
- Tracks inode counts alongside bytes, because a million small files cost more
  in daily lag than their size suggests
- Persists dated snapshots, so regrowth and staleness become visible
- Moves everything to Trash and writes a restore manifest

## Baseline

Measured on a real machine, 2026-08-19: 103 projects, 972 artifact directories,
data volume at 94% capacity. A manual pass following these rules recovered
**41.8 GB** with no data loss — free space went from 27.4 GB to 69.2 GB. That
run is the acceptance baseline; the tool must find at least as much.

Full record: [`docs/evidence/purge-manifest-2026-08-19.md`](docs/evidence/purge-manifest-2026-08-19.md)

## Stack

`ratatui` · `jwalk` · `rusqlite` · `trash` · `gix` · `clap`

Single static binary. No runtime dependency.

## License

MIT

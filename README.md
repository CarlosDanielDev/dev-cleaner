# dev-cleaner

A terminal UI that maps developer project folders, classifies what each
directory is, measures what can actually be recovered, and makes deleting the
wrong thing structurally impossible.

> Status: `scan` and `purge` work end to end, and every scan is recorded so the
> next one can say what changed. The terminal interface is not built yet. See
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
  files, so the number shown is the number you get back — every directory
  measured on its own, verified against `du`
- Classifies projects as active, dormant, or dead from git history, and offers
  to remove a dead project entirely when `git clone` provably restores it
- Tracks inode counts alongside bytes, because a million small files cost more
  in daily lag than their size suggests
- Persists dated snapshots, so regrowth and staleness become visible
- Moves everything to Trash and writes a restore manifest

## Using it

```
dev-cleaner scan [roots...]         # walk, classify, report. Always read-only.
dev-cleaner duplicates [roots...]   # the same package installed in many projects
dev-cleaner shared-store [roots...] # estimate what a shared store would recover
dev-cleaner purge                   # the plan, what is blocked, and the phrase
dev-cleaner purge --execute --confirm "<phrase>"
```

Every scan is recorded, and each one is compared against the last scan of the
same roots, so a narrower scan never reports the directories outside it as
deleted.

`duplicates` is a report and not a plan. What it names is already inside the
artifact directories `scan` counts, so those bytes are not additional space, and
the number it gives is what collapsing every copy into one would free: the size
times the copies that could go, never the sum of all of them. A package pnpm has
already hardlinked into a shared store reads as duplicating nothing, because it
does.

`shared-store` is an estimate and says so wherever it prints. It is the only
figure in the tool that was not measured after the fact, so it is kept off every
screen that shows a reclaimable total: a number in gigabytes reads as a
measurement unless it is labelled otherwise. It excludes projects that already
install through a store, detected by `pnpm-lock.yaml` or `uv.lock` rather than
by a directory name, and it names the projects it could not decide about instead
of picking for them. It prints the migration command and never runs it.

## Where things live

| What | Path |
| --- | --- |
| Configuration | `~/.config/dev-cleaner/config.toml` |
| Scan history | `~/.local/state/dev-cleaner/history.sqlite3` |
| Purge records | `~/.local/state/dev-cleaner/manifests/` |

Deliberately outside every scanned root and every registered cache. A record
the tool could later offer to delete is not a record, and a test pins that
against both registries so a newly registered cache cannot start shadowing it.

## Baseline

Measured on a real machine, 2026-08-19: 103 projects, 972 artifact directories,
data volume at 94% capacity. A manual pass following these rules recovered
**41.8 GB** with no data loss — free space went from 27.4 GB to 69.2 GB. That
run is the acceptance baseline; the tool must find at least as much.

Full record: [`docs/evidence/purge-manifest-2026-08-19.md`](docs/evidence/purge-manifest-2026-08-19.md)

## Roadmap

Work is tracked as 8 epics with 34 sub-issues across three milestones.

| Milestone | Focus | Issues |
| --- | --- | --- |
| [M1 — Scan and see](../../milestone/1) | Walk, measure honestly, classify. Read-only; no deletion path exists yet. | 16 |
| [M2 — Prove and purge](../../milestone/2) | Safety tiers, the compile-time purge gate, Trash-based execution. | 11 |
| [M3 — Remember and report](../../milestone/3) | Snapshots, trends, duplicate reporting, full TUI. | 15 |

Epics: [#1 Foundation](../../issues/1) · [#6 Scanner](../../issues/6) ·
[#12 Classification](../../issues/12) · [#17 Safety core](../../issues/17) ·
[#23 Purge execution](../../issues/23) · [#28 Persistence](../../issues/28) ·
[#32 TUI](../../issues/32) · [#39 Duplicates](../../issues/39)

## Stack

`ratatui` · `jwalk` · `rusqlite` · `trash` · `clap`

Git state is read straight from `.git`, two files at a time, rather than
through a library or a subprocess.

Single static binary. No runtime dependency.

## License

MIT

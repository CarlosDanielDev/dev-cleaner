# dev-cleaner

[![CI](https://github.com/CarlosDanielDev/dev-cleaner/actions/workflows/ci.yml/badge.svg)](https://github.com/CarlosDanielDev/dev-cleaner/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
![Platform: macOS](https://img.shields.io/badge/platform-macOS-lightgrey.svg)

A terminal UI that maps developer project folders, classifies what each
directory is, measures what can actually be recovered, and makes deleting the
wrong thing structurally impossible.

```
scanned 1 root(s) in 1.61s
  projects       258
  entries        204107
  inodes         151125
  actual/unique  11.59 GB

build artifacts
  target            6.54 GB    86034 files   cargo build
  .venv             0.39 GB     9854 files   python -m venv .venv && pip install -r requirements.txt
  node_modules      0.32 GB    14245 files   npm install
  total             7.29 GB  reclaimable
```

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

Three more rules follow from that one:

- **Deletion routes to the Trash.** There is no `remove_dir_all` in `src/`, and
  every removal writes a restore manifest naming what moved and where.
- **A prediction is never reported as a result.** The one estimated figure the
  tool produces is labelled as an estimate and kept off every screen that shows
  a measured reclaimable total.
- **Measured bytes are unique bytes.** Every directory is sized with each inode
  counted once, so hardlinked package stores and sparse files cannot inflate
  the number you are promised.

## What it does

- Walks registered project roots and known developer caches — **ignoring
  `.gitignore`**, because the reclaimable bytes are exactly what `.gitignore`
  hides
- Reports **unique** bytes, accounting for hardlinked package stores and sparse
  files, so the number shown is the number you get back — every directory
  measured on its own, verified against `du`
- Classifies projects as active, dormant, or dead from git history, and offers
  to remove a dead project entirely when `git clone` provably restores it
- Refuses anything holding work that exists nowhere else: untracked files,
  unpushed commits, stashes, dirty worktrees
- Tracks inode counts alongside bytes, because a million small files cost more
  in daily lag than their size suggests
- Persists dated snapshots, so regrowth and staleness become visible
- Reports the same package installed across many projects, and estimates what a
  shared package store would recover

## Requirements

| | |
| --- | --- |
| Platform | macOS. The tool reads `st_blocks`, routes deletions to the macOS Trash, and classifies Xcode and CocoaPods artifacts. |
| Rust | 1.85 or newer (edition 2024), to build from source. |
| Runtime dependencies | None. SQLite is compiled in; git state is read straight from `.git`. |

## Install

No binaries are published yet — see [#61](../../issues/61). Until then, build
from source:

```sh
git clone https://github.com/CarlosDanielDev/dev-cleaner.git
cd dev-cleaner
cargo install --path .
```

That puts `dev-cleaner` in `~/.cargo/bin`. To try it without installing, use
`cargo run --release -- <command>` from the clone.

## Using it

```
dev-cleaner scan [roots...]         # walk, classify, report. Always read-only.
dev-cleaner tui [roots...]          # browse the scan full-screen
dev-cleaner duplicates [roots...]   # the same package installed in many projects
dev-cleaner shared-store [roots...] # estimate what a shared store would recover
dev-cleaner purge                   # the plan, what is blocked, and the phrase
dev-cleaner purge --execute --confirm "<phrase>"
```

Every command except `purge --execute` is read-only. Omitting `roots` falls
back to the configured roots.

### `scan`

Walks, classifies and measures. Every scan is recorded and compared against the
last scan of the same roots, so a narrower scan never reports the directories
outside it as deleted.

```
activity
  active         52
  dormant        202
  dead           4  (idle >180d, every commit on a remote)
      1 of 4 clear every guard
      3 blocked: Stashed work is present and would be lost.

global caches
  go                        0.59 GB   go clean -modcache
  npm                       1.21 GB   npm cache clean --force
  cargo                     0.62 GB   re-downloaded on next build
  pnpm                      4.14 GB   pnpm store prune
  total                     6.56 GB   reclaimable

since the previous scan
     1.27 GB  new          /Users/carlos/projects/dev-cleaner-42/target
     1.21 GB  +12.54 MB    /Users/carlos/.npm/_cacache
   780.76 MB  removed      /Users/carlos/projects/dev-cleaner-38/target
```

### `purge`

A dry run by default — the flag is not something you have to remember. It
prints the plan, everything it refused and why, and the confirmation phrase.

```
Plan: 43 item(s), 7.26 GB
      1.53 GB  /Users/carlos/projects/dev-cleaner/target
      0.35 GB  /Users/carlos/projects/drinith/backend/.venv
      0.09 GB  /Users/carlos/projects/akasha-bot/web/node_modules
  ... and 28 more

Blocked, not in the plan (128):
  /Users/carlos/projects/block-zero/.next: Untracked source files here exist nowhere else.

This was a dry run. Nothing has been touched.
To carry it out:
  dev-cleaner purge --execute --confirm "purge 43 items 7796432896 bytes"
```

`--execute` alone is refused. The phrase describes that exact plan, so it
cannot be known without having read the plan — a flag can be recalled from
shell history, a phrase cannot.

### `duplicates`

A report, not a plan. What it names is already inside the artifact directories
`scan` counts, so those bytes are not additional space. The number it gives is
what collapsing every copy into one would free: the size times the copies that
could go, never the sum of all of them. A package pnpm has already hardlinked
into a shared store reads as duplicating nothing, because it does.

### `shared-store`

Estimates what a shared, content-addressed store would recover across the
projects that still copy packages into themselves.

```
-> migrate to pnpm store: est. 108.39 MB recovered across 63 projects
```

It is an estimate and says so wherever it prints. It is the only figure in the
tool that was not measured after the fact, so it is kept off every screen that
shows a reclaimable total: a number in gigabytes reads as a measurement unless
it is labelled otherwise. It excludes projects that already install through a
store, detected by `pnpm-lock.yaml` or `uv.lock` rather than by a directory
name, and it names the projects it could not decide about instead of picking
for them. It prints the migration command and never runs it.

## The terminal interface

`dev-cleaner tui` scans first, then opens six screens. Routing lives apart from
drawing, so what the interface refuses is a property of the state machine
rather than of how a screen happens to be painted — and is tested with no
terminal attached.

| Screen | What it is |
| --- | --- |
| Dashboard | The disk as it stands, and what moved since the last scan |
| Projects | Every project, sortable by every column |
| Candidates | What is offered, what is blocked, and why |
| Review | The plan as it will be carried out |
| Confirm | The one screen a deletion can start from |
| Result | What moved, where it went, and what failed |

| Key | Action |
| --- | --- |
| `Enter` / `Esc` | forward · back |
| `j` `k` / `↑` `↓` | move |
| `g` / `G` | first · last |
| `PageUp` / `PageDown` | a page at a time |
| `1`–`6` | sort the projects table by column |
| `Space` | mark a candidate |
| `a` / `c` | mark all · clear marks |
| `x` | hold to purge — **only on the confirmation screen** |
| `?` | keys |
| `q` | quit |

Sorting is on the digits rather than on letters deliberately: the mnemonic for
"size" is `s`, which sits next to the key that purges, and a table is sorted far
more often than a plan is confirmed.

## Configuration

`~/.config/dev-cleaner/config.toml`. A missing file yields working defaults —
a first run should need no setup.

```toml
# Directories to scan for projects.
roots = ["~/projects"]

# Ecosystem cache registries to include, by name.
caches = ["npm", "cargo", "go", "xcode", "gradle", "cocoapods", "pnpm"]

# Paths that must never be offered, whatever else concludes.
denylist = ["~/projects/client-work"]
```

The denylist is the outermost safety boundary: both sides are canonicalised
before comparison, so `a/../denied/x` is recognised as the denied location it
actually resolves to.

## Where things live

| What | Path |
| --- | --- |
| Configuration | `~/.config/dev-cleaner/config.toml` |
| Scan history | `~/.local/state/dev-cleaner/history.sqlite3` |
| Purge records | `~/.local/state/dev-cleaner/manifests/` |

Deliberately outside every scanned root and every registered cache. A record
the tool could later offer to delete is not a record, and a test pins that
against both registries so a newly registered cache cannot start shadowing it.

## Development

```sh
cargo build                       # debug build
cargo run -- scan ~/projects      # run a command against a real tree
cargo build --release             # optimised, LTO, stripped
cargo test                        # 249 tests: 21 integration suites, unit, doc
```

### The gate

These four are the whole of CI. Run them before pushing; `RUSTFLAGS: -D warnings`
is set in the workflow, so a warning that is tolerable locally fails the build.

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo deny check                  # cargo install cargo-deny
```

### Running less than everything

```sh
cargo test --test safety                      # one integration suite
cargo test --test shared_store -- --nocapture # with printed output
cargo test hardlink                           # every test whose name matches
```

### How the work is done

**Test first, always.** Watch it fail for the right reason before implementing.

**Mutation-check every guard.** Remove the protection, confirm the test fails,
restore it. This caught a false-confidence test that asserted the right thing
for the wrong reason: the fixture's unpushed commit wrote a fresh reflog entry,
so the repository read as recently active and the test passed while the check
it was meant to pin was disabled.

**Cross-check against reality, not just against tests.** Sizing is validated
against `du` on a real corpus. Watch the units when you do: BSD `du -c` reports
512-byte blocks and `du -k` reports kilobytes, which once made a correct total
look like exact double-counting.

**Fixtures over mocks.** `tests/common/mod.rs` builds real trees in a
`TempDir` — sparse files, hardlinks, git repositories with dated commits and
rewritten reflogs. The purge path uses a recording `Remover` so no suite ever
puts anything in the real Trash.

### Layout

| Path | What lives there |
| --- | --- |
| `src/scan/` | Walking and measuring. `Usage` is the only place bytes are totalled. |
| `src/classify/` | What a directory is, which ecosystem owns it, whether its project is alive. |
| `src/safety/` | Tiers, guards, and the typestate `Plan`. |
| `src/purge/` | Trash-based execution and the restore manifest. |
| `src/store/` | Snapshots and trends, in SQLite. |
| `src/tui/` | Screens, routing and the keymap. |
| `src/duplicates.rs`, `src/shared_store.rs` | Cross-project package reporting. |
| `docs/SESSION-HANDOFF.md` | The decisions worth knowing before changing anything. |
| [`docs/superpowers/specs/`](docs/superpowers/specs/2026-08-19-dev-cleaner-design.md) | The original design spec. |

## Baseline

Measured on a real machine, 2026-08-19: 103 projects, 972 artifact directories,
data volume at 94% capacity. A manual pass following these rules recovered
**41.8 GB** with no data loss — free space went from 27.4 GB to 69.2 GB. That
run is the acceptance baseline; the tool must find at least as much.

Full record: [`docs/evidence/purge-manifest-2026-08-19.md`](docs/evidence/purge-manifest-2026-08-19.md)

## Roadmap

All three milestones are closed: 44 issues across 8 epics.

| Milestone | Focus | Issues |
| --- | --- | --- |
| [M1 — Scan and see](../../milestone/1) | Walk, measure honestly, classify. Read-only; no deletion path exists yet. | 16 |
| [M2 — Prove and purge](../../milestone/2) | Safety tiers, the compile-time purge gate, Trash-based execution. | 11 |
| [M3 — Remember and report](../../milestone/3) | Snapshots, trends, duplicate reporting, full TUI. | 17 |

Epics: [#1 Foundation](../../issues/1) · [#6 Scanner](../../issues/6) ·
[#12 Classification](../../issues/12) · [#17 Safety core](../../issues/17) ·
[#23 Purge execution](../../issues/23) · [#28 Persistence](../../issues/28) ·
[#32 TUI](../../issues/32) · [#39 Duplicates](../../issues/39)

Next: [#61 release binaries](../../issues/61) and
[#62 CI hardening](../../issues/62).

## Stack

`ratatui` · `jwalk` · `rusqlite` · `trash` · `clap`

Git state is read straight from `.git`, two files at a time, rather than
through a library or a subprocess.

Single static binary. No runtime dependency.

## License

MIT

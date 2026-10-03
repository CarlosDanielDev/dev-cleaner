The first tagged version, and a summary of the project from its first commit on
2026-08-19. It is built from source: there are no published binaries or
packages yet. macOS only.

### Added

- Gitignore-blind scanner that sizes every directory in unique bytes, counting each inode once, so hardlinked package stores and sparse files cannot inflate a total (#43)
- Classification of artifacts, projects, global caches and activity (active, dormant, dead), and a TOML configuration with a denylist (#43)
- The safety core: tiers that say what brings each path back, hard guards, and a typestate `Plan` that cannot be executed without review and confirmation (#44)
- `purge`, a dry run by default, which carries a plan out only with `--execute` and a `--confirm` phrase describing that exact plan; removals go to the Trash and write a restore manifest (#44)
- Persistence: dated snapshots, trends and a scan history a purge cannot erase (#46)
- The terminal interface: the screen router with the plan inside it (#50), the dashboard (#51), the projects table sortable by every column (#52), the candidates screen where a blocked entry has no index (#53), plan review and hold-to-confirm over one table of every binding (#55), the result report read from the manifest (#56), and a terminal driver that restores the terminal on every way out (#57)
- `duplicates`, which reports the copies of a package that could go rather than the copies that exist (#59), reading what each project installed from its lockfile (#58)
- `shared-store`, an estimate of what a shared package store would recover, labelled as an estimate and kept off every screen that shows a measured total (#60)
- A semantic palette and a neon theme in which every role is one colour, with truecolor, ANSI and `NO_COLOR` looks (#72, #134)
- Orientation on every screen: the selected row across its width and a stated position (#73), a row naming each screen's neighbours (#74), a key bar that drops entries whole by importance (#75), one notice row (#109), a view bar with a removable filter and one key to reset it (#154), and one table design for Projects, Candidates and the plan (#157)
- Tables that fit the terminal: paging by the rows shown (#107, #123), columns dropped whole from the right with a note saying so (#111), and cuts at a column that keep the head (#110)
- Candidates that open largest first and order on the digits, with marks keyed by path so a reorder cannot change the plan (#112), a cursor that lands on the project `Enter` was pressed on (#128), scoping to one project with marks shown on the projects table (#146), and `c` restoring marks it just cleared (#130)
- A purge that runs on a worker thread and draws one row per item as it moves, with a record written while it runs (#113, #125), and `Esc` stopping it between items (#129)
- A result screen that opens on a SAFE or BLOCKED verdict with its time and compares the run with every run before it (#132), and a way back to a fresh dashboard (#136)
- A dashboard that is an overview of the disk, the scan, the rebuildable bytes and ranked insights (#139), with hints that name what one step forward allows (#116) and land on the project they name (#150), and a sparkline of the reclaimable total over the last scans (#127, #114)
- The logo and wordmark in the header, a status bar with a six-step stepper and key-cap hints (#140, #142, #145, #148, #158)
- A line that counts entries, bytes and time while `scan` and the interface walk (#117, #131), and a scan that runs behind a live interface with a cancel that works (#163)
- Linked git worktrees told apart from main checkouts on the projects table (#153)
- A message for a key that does nothing on the screen, naming the two that would (#122), and a statement of what marking, clearing and sorting changed (#124)
- Documentation of the tool that exists, and how to build and check it locally (#63)

### Changed

- The post-walk phase of `scan` and the interface load is linear in the number of files: a plain `scan` of a 959,245-inode tree went from 89.3 s to 13.8-17.4 s, with identical output (#161)
- Each artifact directory is measured as a whole instead of adding its files up (#47)
- The confirm screen shows the plan's largest entries, and the way back carries the weight of the way forward (#76)
- `q` asks once before it drops marks or a plan, and says what it would drop (#126)

### Fixed

- A scan is recorded before it reports, and finishes its run when the process reading its output goes away, as `| head` does (#49)
- The purge hold is measured on the clock, so its gauge fills when it says it will (#71)

### Security

- Deleting is one held key on the confirm screen, bound to nothing global and to nothing a hand reaches for by accident, asserted over the whole keymap (#55)
- The hold's gauge is emptied on every screen change, so a hold cannot outlive the screen it began on (#108)
- Below 80x24 the interface says what it needs and refuses a hold the screen cannot show the plan for (#119)

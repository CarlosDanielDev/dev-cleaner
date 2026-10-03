# Contributing to dev-cleaner

Thanks for looking. dev-cleaner removes files from people's disks, so the way
it is built is stricter than the size of the project suggests. This is how the
work is actually done here.

## Before you start

- **One issue, one branch, one pull request.** Open or pick an issue first and
  say what you intend to change. Name the branch for it, for example
  `feat/123-short-name` or `fix/124-short-name`.
- Build from source (macOS, Rust 1.85 or newer):

  ```sh
  git clone https://github.com/CarlosDanielDev/dev-cleaner.git
  cd dev-cleaner
  cargo build
  ```

- Read [`docs/SESSION-HANDOFF.md`](docs/SESSION-HANDOFF.md) for the decisions
  worth knowing before changing anything.

## How the work is done

**Test first.** Write the test, watch it fail for the right reason, then make
it pass. A test that passed on the first run proved nothing.

**Mutation-check every new guard.** Remove the protection, confirm the test
fails, restore it. Paste both outputs in the pull request. A guard whose test
survives its own removal is a guard nobody can trust.

**Fixtures, not mocks.** `tests/common/mod.rs` builds real trees in a
`TempDir`: sparse files, hardlinks, git repositories with dated commits. The
purge path uses a recording `Remover`, so no test ever puts anything in the
real Trash. Tests never touch your real folders.

**Cross-check against reality.** Sizing is validated against `du`. Watch the
units: BSD `du -c` counts 512-byte blocks and `du -k` kilobytes.

## The gate

These four are the whole of CI. Run them all before you push:

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo deny check                  # cargo install cargo-deny
```

CI differs from your machine in two ways. It runs a **newer clippy** than a
months-old local toolchain may have, and it tests your pull request **merged
with `main`**, not your branch alone. Merge `origin/main` into your branch and
run the gate again before you ask for review. `RUSTFLAGS: -D warnings` is set in
the workflow, so a warning that is tolerable locally fails the build.

To run less than everything:

```sh
cargo test --test safety                      # one integration suite
cargo test hardlink                           # every test whose name matches
```

## The protected path: `src/safety`

`src/safety` holds the guards and the typestate `Plan` that make an unreviewed
purge impossible to compile. **A change there needs an explicit reviewer and a
reason in the pull request**, and a mutation check for any guard it touches.
The pull request template asks whether you touched it. Do not weaken a guard to
make a test pass; change the test, and say why.

Anything that changes **what can be removed** gets the same scrutiny, wherever
it lives. See [SECURITY.md](SECURITY.md) for what counts as a vulnerability.

## The interface

The look is a design system, and a screen that breaks it is a bug:

- **One colour per role**, through `src/tui/palette.rs`. Screens name a role
  (`theme.accent`, `theme.blocked`), never a colour.
- **Colour is never the only carrier.** Every meaning also has a glyph, a word
  or a weight, and red versus green is never the only difference.
- **Three looks work**: truecolor, 256/16-colour ANSI, and `NO_COLOR`. Test the
  one you changed and the one it falls back to.
- **Tables go through the shared kit** in `src/tui/kit`, so Projects,
  Candidates and the plan keep one look.
- **Keys live in `src/tui/keymap.rs`**, in one table, and `tests/keymap.rs`
  asserts over all of it. Only the held key on the confirm screen may delete.
- The README's keys table and command list are checked against the keymap and
  the CLI by `tests/readme_claims.rs`; change one, change the other.

### Showing a frame in a pull request

Capture frames with a pty and [pyte](https://pypi.org/project/pyte/), against a
**synthetic tree and an isolated `HOME`**. Never scan your real folders, and
never press the purge key in a capture. [`docs/tools/screenshots.py`](docs/tools/screenshots.py)
does exactly this and writes the SVGs the README uses:

```sh
python3 -m venv .venv && .venv/bin/pip install pyte
.venv/bin/python docs/tools/screenshots.py target/debug/dev-cleaner /path/to/scratch
```

[`docs/tools/logo_svg.py`](docs/tools/logo_svg.py) regenerates the logo and the
banner from the grid in `src/tui/logo.rs`. Commit SVG and Markdown only: no
binary images.

## Commits and pull requests

- Imperative mood, in English, one line that says what changes and why it is
  worth it: `Make the scan's post-walk phase linear`. Look at `git log
  --oneline` and match it.
- Put `Closes #N` in the body, not issue numbers in the subject.
- Keep the diff to what the issue asks. No drive-by refactors in the same pull
  request.
- Use the pull request template: the gate as a checklist, and the safety
  questions.
- Add a line to the `Unreleased` section of [CHANGELOG.md](CHANGELOG.md) for
  anything a user would notice.

## Where to ask

Open an issue. For a question that is not a bug or a feature, say so in the
title. Vulnerabilities do not go there: see [SECURITY.md](SECURITY.md).

By contributing you agree that your work is released under the
[MIT licence](LICENSE).

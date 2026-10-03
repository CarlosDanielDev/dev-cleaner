<!-- One issue, one branch, one pull request. -->

Closes #

## What changes, and why


## Safety

- [ ] This does **not** change what can be removed.
- [ ] I did **not** touch `src/safety`.

If either box is unchecked, explain: what changes, who reviews `src/safety`, and
the mutation check (guard removed, test fails, guard restored; paste both).


## The gate

Run on this branch **after merging `origin/main`**. CI tests the merge and runs
a newer clippy.

- [ ] `cargo fmt --check`
- [ ] `cargo clippy --all-targets -- -D warnings`
- [ ] `cargo test`
- [ ] `cargo deny check`

<details><summary>Output</summary>

```
```

</details>

## Interface changes

- [ ] Roles through `src/tui/palette.rs`; colour is never the only carrier
- [ ] Checked under truecolor, 256/16-colour and `NO_COLOR`
- [ ] README keys table and commands still match (`tests/readme_claims.rs`)
- [ ] Frames captured from a synthetic tree in an isolated `HOME`, nothing purged

## Changelog

- [ ] `CHANGELOG.md` has a line under `Unreleased`, or this changes nothing a user would notice

# Security policy

dev-cleaner deletes files. Its whole purpose is to make deleting the wrong one
impossible, so a way around that is the most serious kind of bug it can have,
and it is handled as one.

## Supported versions

Only the latest minor release is supported. While the project is at `0.x` that
means the most recent `0.<minor>.*`; fixes are not backported.

| Version | Supported |
| --- | --- |
| latest `0.x` minor | yes |
| anything older | no |

## Reporting a vulnerability

**Please do not open a public issue for a vulnerability.** Report it privately
through GitHub:

1. Go to the repository's **Security** tab.
2. Choose **Report a vulnerability**
   ([direct link](https://github.com/CarlosDanielDev/dev-cleaner/security/advisories/new)).
3. Describe what you found, the version or commit, and the smallest steps that
   reproduce it. A synthetic directory tree is better than a description of
   your own; never send anyone your real folders.

Private reporting is the only channel. No e-mail address is published for it.

### What to expect

These are intentions from a one-person project, not guarantees:

- an acknowledgement within 7 days;
- an assessment, and whether it is accepted as a vulnerability, within 14 days;
- a fix, or a plan for one, within 30 days of acceptance, released as a new
  version with the advisory published once users can update.

You will be credited in the advisory unless you ask not to be.

## What is a vulnerability

Anything that lets the tool act outside what the user confirmed:

- removing, moving or altering something **outside the guarded path**: outside
  every configured root, or inside the denylist;
- **bypassing the guards in [`src/safety`](src/safety)**: a path offered or
  purged that a guard should have blocked (dirty worktree, untracked files,
  stashed work, symlink escape, Docker volume);
- **following a symlink out of a root**, including through `..` components or a
  link swapped in between the scan and the purge;
- **purging without the typed or held confirmation**: any route to the
  executing code that does not pass through a reviewed and confirmed plan, in
  the CLI (`purge --execute --confirm`) or the interface (the held key on the
  confirm screen);
- **writing outside its own state directory** (`~/.local/state/dev-cleaner`)
  or its configuration file;
- a **restore manifest that names the wrong path**, or a dependency with a
  known advisory that `cargo deny check` would not catch.

## What is not

These are bugs, not vulnerabilities. Open a normal issue:

- a wrong size, count or estimate (`shared-store` is an estimate on purpose);
- a cosmetic or layout problem, a wrong colour, a truncated path;
- a project classified as dormant that you think is active, or the reverse,
  when no guard was bypassed;
- anything that needs you to edit your own configuration to name a path you
  then confirm for removal.

## For maintainers

`src/safety` is the protected path of this repository: a change there needs an
explicit reviewer and a stated reason in the pull request. See
[CONTRIBUTING.md](CONTRIBUTING.md).

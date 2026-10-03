//! The command line honours the theme too (#166): a flag that names no theme is
//! an error that lists the valid ones, and what is printed to a pipe or a file
//! is plain text in every theme.

pub mod common;

use std::path::Path;
use std::process::{Command, Output};

use common::Fixture;
use dev_cleaner::tui::palette::ThemeName;

/// Run the binary with an isolated HOME and no theme in the environment.
fn run(home: &Path, args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_dev-cleaner"));
    cmd.args(args)
        .env_clear()
        .env("HOME", home)
        .env("PATH", "/usr/bin:/bin");
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.output().expect("run the binary")
}

fn tree() -> (Fixture, Fixture) {
    let home = Fixture::new();
    let work = Fixture::new();
    work.file("app/package.json", b"{}");
    work.file("app/src/index.js", b"console.log(1)");
    work.file("app/node_modules/dep/blob.bin", &[7u8; 4096]);
    (home, work)
}

#[test]
fn a_flag_that_names_no_theme_is_an_error_that_lists_the_valid_ones() {
    let (home, work) = tree();
    let root = work.root().to_str().unwrap();
    for command in ["scan", "tui"] {
        let out = run(home.root(), &[command, "--theme", "bogus", root], &[]);
        assert!(!out.status.success(), "{command} --theme bogus succeeded");
        let said = String::from_utf8_lossy(&out.stderr);
        assert!(said.contains("\"bogus\""), "{command}: {said}");
        for entry in ThemeName::ALL {
            assert!(
                said.contains(entry.id),
                "{command} did not list {}: {said}",
                entry.id
            );
        }
        assert!(out.stdout.is_empty(), "{command} printed before refusing");
    }
}

#[test]
fn nothing_is_written_to_the_state_directory_by_a_refused_flag_or_a_scan() {
    let (home, work) = tree();
    let root = work.root().to_str().unwrap();
    run(home.root(), &["scan", "--theme", "bogus", root], &[]);
    assert!(
        !home.root().join(".local/state/dev-cleaner/theme").exists(),
        "a theme was saved without the interface asking"
    );
}

#[test]
fn scan_prints_the_same_plain_text_in_every_theme_when_it_is_not_a_terminal() {
    let (home, work) = tree();
    let root = work.root().to_str().unwrap();
    let first = run(home.root(), &["scan", "--theme", "neon", root], &[]);
    assert!(first.status.success());
    assert!(!first.stdout.contains(&0x1b), "an escape reached a pipe");
    for entry in ThemeName::ALL {
        let out = run(home.root(), &["scan", "--theme", entry.id, root], &[]);
        assert!(out.status.success(), "{}", entry.id);
        assert!(
            !out.stdout.contains(&0x1b),
            "{}: an escape reached a pipe",
            entry.id
        );
        // The lines that name a time differ from run to run; the facts do not.
        let facts = |o: &Output| -> Vec<String> {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .filter(|l| l.starts_with("  projects") || l.starts_with("  entries"))
                .map(str::to_string)
                .collect()
        };
        assert_eq!(facts(&out), facts(&first), "{}", entry.id);
    }
}

#[test]
fn the_environment_and_the_saved_choice_reach_the_command_line_too() {
    let (home, work) = tree();
    let root = work.root().to_str().unwrap();
    // A bad value in the environment is a warning, not a refusal: the run goes on.
    let out = run(
        home.root(),
        &["scan", root],
        &[("DEV_CLEANER_THEME", "bogus")],
    );
    assert!(out.status.success());
    let said = String::from_utf8_lossy(&out.stderr);
    assert!(
        said.contains("DEV_CLEANER_THEME") && said.contains("unknown"),
        "{said}"
    );
    // A bad flag beats a good environment value into an error.
    let out = run(
        home.root(),
        &["scan", "--theme", "bogus", root],
        &[("DEV_CLEANER_THEME", "matrix")],
    );
    assert!(!out.status.success());
}

#[test]
fn a_dry_run_of_purge_is_plain_on_a_pipe_in_every_theme_and_removes_nothing() {
    let (home, work) = tree();
    let cfg = home.file(
        ".config/dev-cleaner/config.toml",
        format!("roots = [\"{}\"]\ncaches = []\n", work.root().display()).as_bytes(),
    );
    assert!(cfg.exists());
    let blob = work.root().join("app/node_modules/dep/blob.bin");
    for theme in ["neon", "matrix"] {
        let out = run(home.root(), &["purge"], &[("DEV_CLEANER_THEME", theme)]);
        assert!(out.status.success(), "{theme}");
        assert!(
            !out.stdout.contains(&0x1b),
            "{theme}: an escape reached a pipe"
        );
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(
            text.contains("This was a dry run. Nothing has been touched."),
            "{text}"
        );
        assert!(blob.exists(), "a dry run removed something");
    }
}

//! The repository's documents make claims: commands that exist, keys that are
//! bound, a licence that matches, a logo that is the one the interface draws.
//! A claim nothing checks drifts, so each one is read back against the code.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use dev_cleaner::tui::{Screen, bindings, logo};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(path: &str) -> String {
    fs::read_to_string(root().join(path)).unwrap_or_else(|e| panic!("{path}: {e}"))
}

fn help(args: &[&str]) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_dev-cleaner"))
        .args(args)
        .arg("--help")
        .output()
        .expect("run the binary");
    assert!(out.status.success(), "--help failed for {args:?}");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// The text of every code span and fenced block: where a README shows what to
/// type. Prose that merely mentions the name is not a command.
fn code(readme: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut fenced = false;
    for line in readme.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
        } else if fenced {
            out.push(line.to_string());
        } else {
            out.extend(line.split('`').skip(1).step_by(2).map(String::from));
        }
    }
    out
}

fn subcommands() -> BTreeSet<String> {
    help(&[])
        .split("Commands:")
        .nth(1)
        .expect("a Commands: section")
        .lines()
        .skip(1)
        .take_while(|l| l.starts_with("  "))
        .filter_map(|l| l.split_whitespace().next())
        .filter(|c| *c != "help")
        .map(String::from)
        .collect()
}

#[test]
fn every_command_the_readme_shows_exists() {
    let known = subcommands();
    assert!(known.contains("scan") && known.contains("purge"));
    let mut seen = BTreeSet::new();
    for line in code(&read("README.md")) {
        for part in line.split("dev-cleaner ").skip(1) {
            let mut words = part.split_whitespace();
            let Some(first) = words.next() else { continue };
            if first.starts_with('-') || first.starts_with('<') {
                continue; // a flag or a placeholder, not a command
            }
            let sub = first.trim_end_matches(|c: char| !c.is_alphanumeric() && c != '-');
            assert!(
                known.contains(sub),
                "README shows `dev-cleaner {sub}`, which is not a command ({known:?})"
            );
            let flags = help(&[sub]);
            for word in words.take_while(|w| *w != "#") {
                if let Some(flag) = word.strip_prefix("--") {
                    let flag = flag.trim_matches(|c: char| !c.is_alphanumeric() && c != '-');
                    assert!(
                        flags.contains(&format!("--{flag}")),
                        "README shows `{sub} --{flag}`, which `{sub} --help` does not list"
                    );
                }
            }
            seen.insert(sub.to_string());
        }
    }
    assert_eq!(seen, known, "the README should show every command");
}

/// How a key is written in the README, from how the keymap displays it.
fn screens(names: &str) -> Vec<Option<Screen>> {
    names
        .split(',')
        .map(|n| match n.trim() {
            "Everywhere" => None,
            "Dashboard" => Some(Screen::Dashboard),
            "Projects" => Some(Screen::Projects),
            "Candidates" => Some(Screen::Candidates),
            "Plan" => Some(Screen::Review),
            "Confirm" => Some(Screen::Confirm),
            "Result" => Some(Screen::Result),
            other => panic!("README names a screen that does not exist: {other}"),
        })
        .collect()
}

#[test]
fn every_key_in_the_keys_table_is_bound_where_it_says() {
    let readme = read("README.md");
    let table = readme
        .split("| Keys | Where | What they do |")
        .nth(1)
        .expect("the keys table");
    let mut checked = 0;
    for row in table.lines().skip(2).take_while(|l| l.starts_with('|')) {
        let cells: Vec<&str> = row.split('|').map(str::trim).collect();
        let keys: Vec<&str> = cells[1].split('`').skip(1).step_by(2).collect();
        assert!(!keys.is_empty(), "no key in row: {row}");
        for key in keys {
            for screen in screens(cells[2]) {
                let bound = bindings()
                    .iter()
                    .any(|b| b.key.to_string() == key && b.screen == screen);
                assert!(
                    bound,
                    "README binds `{key}` on {screen:?}; the keymap does not"
                );
                checked += 1;
            }
        }
    }
    assert!(
        checked > 40,
        "the table was not read: {checked} keys checked"
    );
}

#[test]
fn install_is_from_source_and_packaging_is_roadmap() {
    let readme = read("README.md");
    assert!(readme.contains("cargo install --path ."));
    let lower = readme.to_lowercase();
    for claim in [
        "brew install",
        "cargo install dev-cleaner",
        "releases/download",
        "curl ",
        "crates.io/crates",
    ] {
        assert!(
            !lower.contains(claim),
            "README claims `{claim}`, which does not exist yet"
        );
    }
    let roadmap = readme
        .split("## Roadmap")
        .nth(1)
        .expect("a Roadmap section");
    assert!(roadmap.contains("Homebrew") && roadmap.contains("Packaging"));
}

#[test]
fn every_relative_link_and_image_points_at_a_file() {
    for doc in [
        "README.md",
        "SECURITY.md",
        "CONTRIBUTING.md",
        "CHANGELOG.md",
    ] {
        let text = read(doc);
        for part in text.split("](").skip(1).chain(text.split("src=\"").skip(1)) {
            let target = part.split([')', '"']).next().unwrap();
            let target = target.split('#').next().unwrap();
            if target.is_empty() || target.contains("://") || target.starts_with("../") {
                continue;
            }
            assert!(
                root().join(target).exists(),
                "{doc} links to {target}, which is missing"
            );
        }
    }
}

#[test]
fn the_licence_agrees_everywhere() {
    let licence = read("LICENSE");
    assert!(licence.starts_with("MIT License\n\nCopyright (c) 2026 Carlos Daniel\n"));
    assert!(licence.contains("Permission is hereby granted, free of charge"));
    assert!(read("Cargo.toml").contains("license = \"MIT\""));
    assert!(read("deny.toml").contains("\"MIT\""));
    assert!(read("README.md").contains("license-MIT"));
    assert!(read("README.md").contains("[MIT](LICENSE)"));
}

/// The (x, y, colour) of every cell a `<rect>` of the SVG covers.
fn cells_of(svg: &str) -> BTreeSet<(usize, usize, String)> {
    let attr = |tag: &str, name: &str| -> String {
        let key = format!("{name}=\"");
        tag.split(&key)
            .nth(1)
            .unwrap()
            .split('"')
            .next()
            .unwrap()
            .to_string()
    };
    let mut cells = BTreeSet::new();
    for tag in svg.split("<rect ").skip(1) {
        let tag = tag.split("/>").next().unwrap();
        let (x, y): (usize, usize) = (
            attr(tag, "x").parse().unwrap(),
            attr(tag, "y").parse().unwrap(),
        );
        let (w, h): (usize, usize) = (
            attr(tag, "width").parse().unwrap(),
            attr(tag, "height").parse().unwrap(),
        );
        for dy in 0..h {
            for dx in 0..w {
                assert!(
                    cells.insert((x + dx, y + dy, attr(tag, "fill"))),
                    "overlapping rects"
                );
            }
        }
    }
    cells
}

#[test]
fn the_logo_svg_is_the_master_grid_cell_for_cell() {
    let mut want = BTreeSet::new();
    for (y, row) in logo::MASTER.iter().enumerate() {
        for (x, ink) in row.chars().enumerate() {
            match ink {
                'M' => want.insert((x, y, "#ff2e97".to_string())),
                'C' => want.insert((x, y, "#00e5ff".to_string())),
                _ => false,
            };
        }
    }
    let got = cells_of(&read("docs/img/logo.svg"));
    assert_eq!(got.len(), want.len(), "a different number of cells");
    assert_eq!(
        got, want,
        "docs/img/logo.svg is not logo::MASTER: run docs/tools/logo_svg.py"
    );
}

fn files_under(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            files_under(&path, out);
        } else {
            out.push(path);
        }
    }
}

#[test]
fn no_binary_image_is_committed_and_every_asset_is_small() {
    let mut files = Vec::new();
    for dir in ["docs", ".github"] {
        files_under(&root().join(dir), &mut files);
    }
    for file in &files {
        let ext = file.extension().and_then(|e| e.to_str()).unwrap_or("");
        assert!(
            !["png", "jpg", "jpeg", "gif", "webp", "ico", "pdf", "bmp"].contains(&ext),
            "{file:?} is a binary; assets are SVG and Markdown only"
        );
    }
    let svgs: Vec<_> = files
        .iter()
        .filter(|f| f.extension().is_some_and(|e| e == "svg"))
        .collect();
    assert!(svgs.len() >= 8, "the README's images are missing: {svgs:?}");
    for svg in svgs {
        let size = fs::metadata(svg).unwrap().len();
        assert!(size < 150 * 1024, "{svg:?} is {size} bytes");
        let text = fs::read_to_string(svg).unwrap();
        assert!(
            !text.contains("<script") && !text.contains("<image"),
            "{svg:?} is not plain vector"
        );
    }
}

#[test]
fn the_project_documents_name_no_personal_address() {
    let docs = [
        "README.md",
        "SECURITY.md",
        "CONTRIBUTING.md",
        "CHANGELOG.md",
        "LICENSE",
        "docs/release-notes-0.1.0.md",
        ".github/PULL_REQUEST_TEMPLATE.md",
        ".github/ISSUE_TEMPLATE/bug_report.yml",
        ".github/ISSUE_TEMPLATE/feature_request.yml",
        ".github/ISSUE_TEMPLATE/config.yml",
    ];
    for doc in docs {
        let text = read(doc);
        for (at, _) in text.match_indices('@') {
            let around = &text[at.saturating_sub(1)..(at + 2).min(text.len())];
            let mailish = around.chars().next().is_some_and(char::is_alphanumeric)
                && around.chars().last().is_some_and(char::is_alphanumeric);
            assert!(
                !mailish,
                "{doc} contains something that looks like an e-mail address"
            );
        }
        assert!(
            !text.to_lowercase().contains("mailto:"),
            "{doc} has a mailto: link"
        );
    }
}

/// The body of the `## [0.1.0]` section, without its heading.
fn section_0_1_0(changelog: &str) -> String {
    let body = changelog
        .split("## [0.1.0]")
        .nth(1)
        .expect("a 0.1.0 section");
    let body = body.split_once('\n').unwrap().1;
    let end = body
        .find("\n## [")
        .or_else(|| body.find("\n[Unreleased]:"))
        .unwrap_or(body.len());
    body[..end].trim().to_string()
}

#[test]
fn the_release_notes_are_the_changelog_section_and_each_entry_names_its_pr() {
    let section = section_0_1_0(&read("CHANGELOG.md"));
    assert!(
        read("docs/release-notes-0.1.0.md").trim() == section,
        "release notes differ"
    );
    let bullets: Vec<_> = section.lines().filter(|l| l.starts_with("- ")).collect();
    assert!(
        bullets.len() >= 10,
        "a 0.1.0 with {} entries is not the history",
        bullets.len()
    );
    for bullet in bullets {
        let numbered = bullet
            .split('#')
            .skip(1)
            .any(|r| r.starts_with(|c: char| c.is_ascii_digit()));
        assert!(numbered, "entry without a PR or issue number: {bullet}");
    }
    assert!(read("CHANGELOG.md").contains("## [0.1.0] - 2026-10-03"));
}

#[test]
fn the_gate_in_the_documents_is_the_gate_ci_runs() {
    let ci = read(".github/workflows/ci.yml");
    let gate = [
        "cargo fmt --check",
        "cargo clippy --all-targets -- -D warnings",
        "cargo test",
    ];
    for doc in ["CONTRIBUTING.md", ".github/PULL_REQUEST_TEMPLATE.md"] {
        let text = read(doc);
        for step in gate.iter().chain(&["cargo deny check"]) {
            assert!(text.contains(step), "{doc} is missing `{step}`");
        }
    }
    for step in gate {
        assert!(
            ci.contains(step),
            "CI no longer runs `{step}`; update the documents"
        );
    }
    assert!(
        ci.contains("cargo-deny-action"),
        "CI no longer runs the deny check"
    );
    for doc in [
        "CONTRIBUTING.md",
        "SECURITY.md",
        ".github/PULL_REQUEST_TEMPLATE.md",
    ] {
        assert!(
            read(doc).contains("src/safety"),
            "{doc} does not name the protected path"
        );
    }
    assert!(Path::new(&root().join("src/safety")).is_dir());
}

#[test]
fn the_themes_section_names_every_theme_the_flag_the_key_and_the_variables() {
    let readme = read("README.md");
    let themes = readme
        .split("\n## Themes")
        .nth(1)
        .expect("a Themes section")
        .split("\n## ")
        .next()
        .unwrap();
    for entry in dev_cleaner::tui::palette::ThemeName::ALL {
        assert!(
            themes.contains(&format!("| `{}` |", entry.id)),
            "the Themes table does not list `{}`",
            entry.id
        );
        assert!(
            themes.contains(entry.about),
            "the Themes table describes `{}` in other words than the registry",
            entry.id
        );
    }
    for claim in [
        "dev-cleaner tui --theme matrix",
        "DEV_CLEANER_THEME",
        "DEV_CLEANER_REDUCED_MOTION",
        "NO_COLOR",
        "~/.local/state/dev-cleaner/theme",
        "`T`",
    ] {
        assert!(themes.contains(claim), "the Themes section lacks `{claim}`");
    }
    // The flag is on the commands the README says it is on.
    for sub in ["tui", "scan"] {
        assert!(
            help(&[sub]).contains("--theme"),
            "`{sub} --help` lacks --theme"
        );
    }
    // The key is in the keys table, which the test above checks against the keymap.
    assert!(
        readme.contains("| `T` | Everywhere |"),
        "the keys table lacks the theme key"
    );
    // The environment variables are read by the code, not only written down.
    let source = [read("src/tui/palette.rs"), read("src/main.rs")].join("\n");
    for var in ["DEV_CLEANER_THEME", "DEV_CLEANER_REDUCED_MOTION"] {
        assert!(source.contains(var), "{var} is documented and not read");
    }
}

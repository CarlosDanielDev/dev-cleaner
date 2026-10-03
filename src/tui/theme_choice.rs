//! Which theme the interface opens in, and where the choice is remembered.
//!
//! Highest first: `--theme`, `DEV_CLEANER_THEME`, the choice the TUI saved, the
//! config's own `theme = "..."`, then neon. The flag is a request made now, so a
//! bad one is an error; every other source is something left lying around, so a
//! bad one is noticed once and the next source answers.
//!
//! The choice is saved by the TUI in its own state directory, next to the
//! records of purges, as a file of one word written whole (temp file, then
//! rename). The owner's hand-edited config is never rewritten: that would drop
//! its comments and its order, and the key in it stays the owner's own default.

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use super::palette::ThemeName;
use crate::config::theme_setting;

/// Where each source says the theme should come from.
#[derive(Debug, Default, Clone, Copy)]
pub struct Sources<'a> {
    /// `--theme`.
    pub flag: Option<&'a str>,
    /// `DEV_CLEANER_THEME`.
    pub env: Option<&'a str>,
    /// What the TUI saved.
    pub saved: Option<&'a str>,
    /// `theme = "..."` in the config file.
    pub config: Option<&'a str>,
}

/// The theme to open in, and anything the user should be told about the way it
/// was reached.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    pub name: ThemeName,
    /// One line each; shown once, when the interface opens.
    pub notices: Vec<String>,
}

/// `--theme` named a theme there is not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownTheme(pub String);

impl fmt::Display for UnknownTheme {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "unknown theme \"{}\"; valid themes: {}",
            self.0,
            ThemeName::ids()
        )
    }
}

impl std::error::Error for UnknownTheme {}

/// A value cut to a length a notice can carry.
fn shown(value: &str) -> String {
    let mut out: String = value.chars().take(40).collect();
    if value.chars().count() > 40 {
        out.push('…');
    }
    out
}

/// Pick the theme by the order above.
pub fn resolve(sources: &Sources) -> Result<Resolved, UnknownTheme> {
    if let Some(flag) = sources.flag.filter(|v| !v.trim().is_empty()) {
        return ThemeName::parse(flag.trim())
            .map(|name| Resolved {
                name,
                notices: Vec::new(),
            })
            .ok_or_else(|| UnknownTheme(shown(flag)));
    }
    let mut notices = Vec::new();
    let lower = [
        (sources.env, "DEV_CLEANER_THEME"),
        (sources.saved, "Saved theme"),
        (sources.config, "The config's theme"),
    ];
    // What was said, source by source, until one of them is a theme. The notice
    // names the theme that ended up answering, which is known only after.
    let mut unknown: Vec<(&str, String)> = Vec::new();
    let mut found = ThemeName::Neon;
    for (value, source) in lower {
        let Some(value) = value.map(str::trim).filter(|v| !v.is_empty()) else {
            continue;
        };
        match ThemeName::parse(value) {
            Some(name) => {
                found = name;
                break;
            }
            None => unknown.push((source, shown(value))),
        }
    }
    for (source, value) in unknown {
        notices.push(match source {
            "Saved theme" => format!("Saved theme \"{value}\" is unknown; using {}", found.id()),
            "DEV_CLEANER_THEME" => {
                format!(
                    "DEV_CLEANER_THEME \"{value}\" is unknown; using {}",
                    found.id()
                )
            }
            _ => format!(
                "The theme \"{value}\" in the config is unknown; using {}",
                found.id()
            ),
        });
    }
    Ok(Resolved {
        name: found,
        notices,
    })
}

/// The app's own state directory under `home`: where the records of purges and
/// the saved theme live.
pub fn state_dir_in(home: &Path) -> PathBuf {
    home.join(".local/state/dev-cleaner")
}

/// [`state_dir_in`] for the user running this.
pub fn state_dir() -> PathBuf {
    state_dir_in(Path::new(
        &std::env::var("HOME").unwrap_or_else(|_| "/".into()),
    ))
}

/// The file the saved theme is in.
pub fn theme_file() -> PathBuf {
    state_dir().join("theme")
}

/// The word saved in `path`: nothing if there is no such file or it cannot be
/// read, which is not a choice and not worth a notice.
pub fn read_saved(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    let text = String::from_utf8_lossy(&bytes);
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

/// Remember `name` in `path`, whole or not at all.
///
/// Written beside the file under a name of its own and renamed over it, so a
/// second instance saving at the same moment, or a power cut, leaves one whole
/// value: the last rename wins, and nobody ever reads half a word.
pub fn save(path: &Path, name: ThemeName) -> io::Result<()> {
    static SAVES: AtomicU64 = AtomicU64::new(0);
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(dir)?;
    let temp = dir.join(format!(
        ".theme.{}.{}.tmp",
        std::process::id(),
        SAVES.fetch_add(1, Ordering::Relaxed)
    ));
    let written = std::fs::write(&temp, format!("{}\n", name.id()))
        .and_then(|()| std::fs::rename(&temp, path));
    if written.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    written
}

/// Every source read: the flag and the environment variable as given, the
/// saved file at `saved`, and the key of the config at `config`.
pub fn load(
    flag: Option<&str>,
    env: Option<&str>,
    saved: &Path,
    config: &Path,
) -> Result<Resolved, UnknownTheme> {
    let saved = read_saved(saved);
    let config = theme_setting(config);
    resolve(&Sources {
        flag,
        env,
        saved: saved.as_deref(),
        config: config.as_deref(),
    })
}

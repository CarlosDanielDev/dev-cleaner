//! Which theme the interface opens in, and where the choice is kept (#166):
//! `--theme`, then `DEV_CLEANER_THEME`, then what the TUI saved, then the
//! config's `theme = "..."`, then neon.

pub mod common;

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use common::Fixture;
use dev_cleaner::config::theme_setting;
use dev_cleaner::purge::manifest_dir;
use dev_cleaner::tui::palette::ThemeName;
use dev_cleaner::tui::theme_choice::{Sources, load, read_saved, resolve, save, state_dir_in};

const NEON: Option<&str> = Some("neon");
const MATRIX: Option<&str> = Some("matrix");

fn name(sources: Sources) -> ThemeName {
    resolve(&sources).expect("every source is valid").name
}

#[test]
fn with_nothing_said_the_theme_is_neon() {
    let got = resolve(&Sources::default()).unwrap();
    assert_eq!(got.name, ThemeName::Neon);
    assert!(got.notices.is_empty());
}

#[test]
fn each_source_beats_every_source_below_it() {
    // Highest first. Each source says matrix and the ones under it say neon, so
    // matrix is the answer exactly when the source is the one that decides.
    type Set = fn(&mut Sources<'static>, Option<&'static str>);
    let order: [(&str, Set); 4] = [
        ("flag", |s, v| s.flag = v),
        ("env", |s, v| s.env = v),
        ("saved", |s, v| s.saved = v),
        ("config", |s, v| s.config = v),
    ];
    for (hi, (high, set_high)) in order.iter().enumerate() {
        for (low, set_low) in &order[hi + 1..] {
            let mut s = Sources::default();
            set_high(&mut s, MATRIX);
            set_low(&mut s, NEON);
            assert_eq!(name(s), ThemeName::Matrix, "{high} over {low}");
            let mut s = Sources::default();
            set_high(&mut s, NEON);
            set_low(&mut s, MATRIX);
            assert_eq!(name(s), ThemeName::Neon, "{high} over {low}, the other way");
        }
    }
    // And the last one beats the default.
    let s = Sources {
        config: MATRIX,
        ..Sources::default()
    };
    assert_eq!(name(s), ThemeName::Matrix);
}

#[test]
fn a_bad_flag_is_an_error_that_lists_the_names() {
    let err = resolve(&Sources {
        flag: Some("bogus"),
        env: MATRIX,
        ..Sources::default()
    })
    .expect_err("--theme bogus must not be guessed at");
    let said = err.to_string();
    assert!(said.contains("\"bogus\""), "{said}");
    assert!(said.contains("neon, matrix"), "{said}");
}

#[test]
fn a_bad_environment_value_is_noticed_and_the_next_source_answers() {
    let got = resolve(&Sources {
        env: Some("bogus"),
        saved: MATRIX,
        ..Sources::default()
    })
    .unwrap();
    assert_eq!(got.name, ThemeName::Matrix);
    assert_eq!(got.notices.len(), 1);
    assert!(
        got.notices[0].contains("DEV_CLEANER_THEME")
            && got.notices[0].contains("\"bogus\"")
            && got.notices[0].contains("using matrix"),
        "{:?}",
        got.notices
    );
}

#[test]
fn a_bad_saved_value_says_so_once_and_neon_answers() {
    let got = resolve(&Sources {
        saved: Some("x"),
        ..Sources::default()
    })
    .unwrap();
    assert_eq!(got.name, ThemeName::Neon);
    assert_eq!(got.notices, ["Saved theme \"x\" is unknown; using neon"]);
}

#[test]
fn a_bad_config_value_is_noticed_and_neon_answers() {
    let got = resolve(&Sources {
        config: Some("bogus"),
        ..Sources::default()
    })
    .unwrap();
    assert_eq!(got.name, ThemeName::Neon);
    assert_eq!(got.notices.len(), 1);
    assert!(got.notices[0].contains("config"), "{:?}", got.notices);
}

#[test]
fn an_empty_value_is_not_a_choice() {
    let got = resolve(&Sources {
        env: Some(""),
        saved: Some("  "),
        config: MATRIX,
        ..Sources::default()
    })
    .unwrap();
    assert_eq!(got.name, ThemeName::Matrix);
    assert!(got.notices.is_empty(), "{:?}", got.notices);
}

// --- the saved file -----------------------------------------------------

fn state(fx: &Fixture) -> std::path::PathBuf {
    fx.root().join("state/theme")
}

#[test]
fn a_saved_theme_survives_a_restart() {
    let fx = Fixture::new();
    let file = state(&fx);
    assert_eq!(read_saved(&file), None, "nothing saved yet");
    save(&file, ThemeName::Matrix).expect("the state directory is made");
    assert_eq!(read_saved(&file).as_deref(), Some("matrix"));
    save(&file, ThemeName::Neon).unwrap();
    assert_eq!(read_saved(&file).as_deref(), Some("neon"));
    // A "restart" is a fresh resolution from the same files.
    let again = load(None, None, &file, &fx.root().join("no-config.toml")).unwrap();
    assert_eq!(again.name, ThemeName::Neon);
}

#[test]
fn a_missing_or_unreadable_saved_file_is_no_choice_and_no_notice() {
    let fx = Fixture::new();
    let file = state(&fx);
    let got = load(None, None, &file, &fx.root().join("c.toml")).unwrap();
    assert_eq!((got.name, got.notices.len()), (ThemeName::Neon, 0));
    // A directory where the file should be reads as nothing.
    fs::create_dir_all(&file).unwrap();
    assert_eq!(read_saved(&file), None);
    let got = load(None, None, &file, &fx.root().join("c.toml")).unwrap();
    assert_eq!((got.name, got.notices.len()), (ThemeName::Neon, 0));
}

#[test]
fn a_saved_file_with_nonsense_in_it_is_noticed_not_fatal() {
    let fx = Fixture::new();
    let file = state(&fx);
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(&file, b"\xff\xfe not a theme \n").unwrap();
    let got = load(None, None, &file, &fx.root().join("c.toml")).unwrap();
    assert_eq!(got.name, ThemeName::Neon);
    assert_eq!(got.notices.len(), 1, "{:?}", got.notices);
}

#[test]
fn the_file_is_written_whole_or_not_at_all_and_leaves_nothing_behind() {
    let fx = Fixture::new();
    let file = state(&fx);
    save(&file, ThemeName::Matrix).unwrap();
    let left: Vec<_> = fs::read_dir(file.parent().unwrap())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(left, ["theme"], "a temp file was left behind");
    assert_eq!(fs::read_to_string(&file).unwrap(), "matrix\n");
}

#[test]
fn two_instances_saving_at_once_leave_one_whole_value_and_no_torn_read() {
    let fx = Fixture::new();
    let file = Arc::new(state(&fx));
    save(&file, ThemeName::Neon).unwrap();
    let done = Arc::new(AtomicBool::new(false));
    let reader = {
        let (file, done) = (Arc::clone(&file), Arc::clone(&done));
        std::thread::spawn(move || {
            let mut reads = 0u32;
            while !done.load(Ordering::Relaxed) {
                // `read_saved` is None for an unreadable file: a torn or missing
                // file would show here as None or as text that is no theme.
                let text = fs::read_to_string(&*file).expect("the file is always there");
                assert!(
                    text == "neon\n" || text == "matrix\n",
                    "a torn read: {text:?}"
                );
                reads += 1;
            }
            reads
        })
    };
    let writers: Vec<_> = [ThemeName::Neon, ThemeName::Matrix]
        .into_iter()
        .map(|which| {
            let file = Arc::clone(&file);
            std::thread::spawn(move || {
                for _ in 0..200 {
                    save(&file, which).expect("a concurrent save");
                }
            })
        })
        .collect();
    for w in writers {
        w.join().unwrap();
    }
    done.store(true, Ordering::Relaxed);
    assert!(reader.join().unwrap() > 0);
    let last = read_saved(&file).expect("a value");
    assert!(last == "neon" || last == "matrix", "{last}");
    let left = fs::read_dir(file.parent().unwrap()).unwrap().count();
    assert_eq!(left, 1, "temp files were left behind");
}

#[test]
fn a_read_only_state_directory_is_an_error_to_say_and_not_a_crash() {
    use std::os::unix::fs::PermissionsExt;
    let fx = Fixture::new();
    let dir = fx.root().join("ro");
    fs::create_dir_all(&dir).unwrap();
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o555)).unwrap();
    // root writes anywhere, which proves nothing about this.
    // SAFETY: geteuid has no preconditions.
    if unsafe { libc::geteuid() } == 0 {
        return;
    }
    let err = save(&dir.join("theme"), ThemeName::Matrix).expect_err("nothing can be written");
    assert!(!err.to_string().is_empty());
    assert_eq!(read_saved(&dir.join("theme")), None);
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
}

#[test]
fn the_choice_is_kept_beside_the_records_in_the_apps_own_state_directory() {
    let home = Path::new("/home/someone");
    let dir = state_dir_in(home);
    assert_eq!(dir, home.join(".local/state/dev-cleaner"));
    // The same directory the manifests live in, which `purge` already owns.
    assert_eq!(
        manifest_dir().parent().map(Path::to_path_buf),
        Some(dev_cleaner::tui::theme_choice::state_dir())
    );
}

// --- the config key ------------------------------------------------------

#[test]
fn the_config_theme_is_read_without_touching_the_rest_of_the_config() {
    let fx = Fixture::new();
    let path = fx.file("config.toml", b"roots = [\"/x\"]\ntheme = \"matrix\"\n");
    assert_eq!(theme_setting(&path).as_deref(), Some("matrix"));
    let none = fx.file("none.toml", b"roots = [\"/x\"]\n");
    assert_eq!(theme_setting(&none), None);
    let wrong = fx.file("wrong.toml", b"theme = 7\n");
    assert_eq!(theme_setting(&wrong), None);
    assert_eq!(theme_setting(&fx.root().join("missing.toml")), None);
    // The config still loads with the key in it, and its fields are unchanged.
    let cfg = dev_cleaner::config::Config::load(&path).unwrap();
    assert_eq!(cfg.roots, [Path::new("/x")]);
}

#[test]
fn every_source_end_to_end_from_files() {
    let fx = Fixture::new();
    let saved = state(&fx);
    let config = fx.file("config.toml", b"theme = \"matrix\"\n");
    // config only
    assert_eq!(
        load(None, None, &saved, &config).unwrap().name,
        ThemeName::Matrix
    );
    // saved beats config
    save(&saved, ThemeName::Neon).unwrap();
    assert_eq!(
        load(None, None, &saved, &config).unwrap().name,
        ThemeName::Neon
    );
    // env beats saved
    assert_eq!(
        load(None, MATRIX, &saved, &config).unwrap().name,
        ThemeName::Matrix
    );
    // flag beats env
    assert_eq!(
        load(NEON, MATRIX, &saved, &config).unwrap().name,
        ThemeName::Neon
    );
    // a bad flag is an error whatever the rest says
    assert!(load(Some("bogus"), MATRIX, &saved, &config).is_err());
}

pub mod common;

use dev_cleaner::classify::{Ecosystem, artifact_for, artifact_kinds};

#[test]
fn recognises_the_artifact_directories_found_on_a_real_machine() {
    // Every name here was observed in the reference corpus of 103 projects.
    for (dir, eco) in [
        ("node_modules", Ecosystem::Node),
        ("target", Ecosystem::Rust),
        (".venv", Ecosystem::Python),
        ("__pycache__", Ecosystem::Python),
        ("Pods", Ecosystem::Swift),
        ("DerivedData", Ecosystem::Swift),
        (".next", Ecosystem::Node),
        (".gradle", Ecosystem::Java),
    ] {
        let kind = artifact_for(dir).unwrap_or_else(|| panic!("{dir} should be recognised"));
        assert_eq!(
            kind.ecosystem, eco,
            "{dir} attributed to the wrong ecosystem"
        );
    }
}

#[test]
fn every_registered_kind_declares_how_it_comes_back() {
    // The governing rule: nothing is deletable unless the tool can name the
    // command that regenerates it. A kind without one would leak into the
    // selectable tiers.
    for kind in artifact_kinds() {
        assert!(
            !kind.regen.trim().is_empty(),
            "{} has no regeneration command",
            kind.dir_name
        );
    }
    assert!(artifact_kinds().len() >= 8, "registry looks unpopulated");
}

#[test]
fn unknown_directories_are_not_claimed() {
    // An unrecognised directory must fall through to the Unproven tier rather
    // than being guessed at.
    for dir in ["src", "docs", "my_node_modules_backup", ""] {
        assert!(
            artifact_for(dir).is_none(),
            "{dir} must not be claimed by the registry"
        );
    }
}

mod projects {
    use super::common::Fixture;
    use dev_cleaner::classify::{Ecosystem, ProjectIndex};
    use dev_cleaner::scan::Walker;

    fn index(fx: &Fixture) -> ProjectIndex {
        ProjectIndex::from_files(&Walker::new([fx.root()]).walk().files)
    }

    #[test]
    fn a_marker_file_defines_a_project() {
        let fx = Fixture::new();
        fx.file("app/package.json", b"{}");
        fx.file("app/src/index.js", b"");

        let idx = index(&fx);

        assert_eq!(idx.len(), 1);
        let owner = idx
            .owner_of(&fx.root().join("app/src/index.js"))
            .expect("file should belong to the project");
        assert_eq!(owner.root, fx.root().join("app"));
    }

    #[test]
    fn markers_inside_artifact_directories_do_not_create_projects() {
        // Every package under node_modules ships its own package.json. Treating
        // those as projects would turn a 103-project corpus into thousands and
        // make every downstream count meaningless.
        let fx = Fixture::new();
        fx.file("app/package.json", b"{}");
        fx.file("app/node_modules/react/package.json", b"{}");
        fx.file("app/node_modules/lodash/package.json", b"{}");
        fx.file("app/vendor/some/composer.json", b"{}");

        let idx = index(&fx);

        assert_eq!(
            idx.len(),
            1,
            "only the real project counts; found roots: {:?}",
            idx.projects().map(|p| &p.root).collect::<Vec<_>>()
        );
    }

    #[test]
    fn platformio_dependency_trees_do_not_create_projects() {
        // .pio/libdeps is PlatformIO's dependency tree, and vendored libraries
        // there ship their own example projects complete with platformio.ini.
        // Found on the reference corpus by cross-checking detection against an
        // independent method.
        let fx = Fixture::new();
        fx.file("firmware/platformio.ini", b"");
        fx.file(
            "firmware/.pio/libdeps/m5stick/M5GFX/examples/PlatformIO_SDL/platformio.ini",
            b"",
        );
        fx.file("firmware/.pio/build/m5stick/firmware.bin", b"");

        let idx = index(&fx);

        assert_eq!(
            idx.len(),
            1,
            "only the firmware project counts; found roots: {:?}",
            idx.projects().map(|p| &p.root).collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_project_records_every_ecosystem_it_uses() {
        let fx = Fixture::new();
        fx.file("app/package.json", b"{}");
        fx.file("app/Cargo.toml", b"");
        fx.file("app/Gemfile", b"");

        let idx = index(&fx);
        let p = idx.owner_of(&fx.root().join("app/package.json")).unwrap();

        assert!(p.ecosystems.contains(&Ecosystem::Node));
        assert!(p.ecosystems.contains(&Ecosystem::Rust));
        assert!(p.ecosystems.contains(&Ecosystem::Ruby));
    }

    #[test]
    fn nested_projects_resolve_to_the_innermost_owner() {
        let fx = Fixture::new();
        fx.file("mono/package.json", b"{}");
        fx.file("mono/packages/ui/package.json", b"{}");
        let deep = fx.file("mono/packages/ui/src/button.tsx", b"");

        let idx = index(&fx);

        assert_eq!(idx.len(), 2);
        assert_eq!(
            idx.owner_of(&deep).expect("owner").root,
            fx.root().join("mono/packages/ui"),
            "the innermost project owns the file"
        );
    }

    #[test]
    fn files_outside_any_project_have_no_owner() {
        let fx = Fixture::new();
        fx.file("app/package.json", b"{}");
        let loose = fx.file("notes.txt", b"");

        assert!(index(&fx).owner_of(&loose).is_none());
    }
}

mod caches {
    use super::common::Fixture;
    use dev_cleaner::classify::{cache_kinds, probe_caches};

    #[test]
    fn every_cache_declares_where_it_lives_and_how_it_comes_back() {
        for c in cache_kinds() {
            assert!(!c.name.trim().is_empty(), "cache has no name");
            assert!(!c.rel_path.trim().is_empty(), "{} has no location", c.name);
            assert!(
                !c.regen.trim().is_empty(),
                "{} does not say how it regenerates",
                c.name
            );
        }
        assert!(cache_kinds().len() >= 8, "registry looks unpopulated");
    }

    #[test]
    fn absent_caches_are_skipped_rather_than_reported_as_errors() {
        // A machine without Xcode or Go should not produce a wall of failures.
        let fake_home = Fixture::new();
        fake_home.file(".npm/_cacache/index/x", b"cached");

        let found = probe_caches(fake_home.root(), &["npm".into(), "go".into()]);

        assert_eq!(found.len(), 1, "only the cache that exists is returned");
        assert_eq!(found[0].kind.name, "npm");
        assert!(found[0].path.starts_with(fake_home.root()));
    }

    #[test]
    fn only_enabled_caches_are_probed() {
        let fake_home = Fixture::new();
        fake_home.file(".npm/_cacache/x", b"a");
        fake_home.file("go/pkg/mod/x", b"b");

        let only_go = probe_caches(fake_home.root(), &["go".into()]);

        assert_eq!(only_go.len(), 1);
        assert_eq!(only_go[0].kind.name, "go");
    }

    #[test]
    fn a_probed_cache_can_report_its_size() {
        let fake_home = Fixture::new();
        fake_home.file(".npm/_cacache/blob", &vec![b'x'; 40_000]);

        let found = probe_caches(fake_home.root(), &["npm".into()]);

        assert!(
            found[0].usage().bytes_unique >= 40_000,
            "cache should measure its own contents"
        );
    }
}

mod activity {
    use super::common::Fixture;
    use dev_cleaner::classify::Activity;
    use std::time::{Duration, SystemTime};

    fn days(n: u64) -> Duration {
        Duration::from_secs(n * 86_400)
    }
    fn now() -> SystemTime {
        SystemTime::now()
    }

    #[test]
    fn a_recent_commit_means_active() {
        let fx = Fixture::new();
        let repo = fx.git_repo("app", 3);
        assert_eq!(Activity::of(&repo, None, now()), Activity::Active);
    }

    #[test]
    fn a_few_months_idle_means_dormant() {
        let fx = Fixture::new();
        let repo = fx.git_repo("app", 60);
        fx.mark_pushed("app");
        assert_eq!(Activity::of(&repo, None, now()), Activity::Dormant);
    }

    #[test]
    fn long_idle_and_fully_pushed_means_dead() {
        let fx = Fixture::new();
        let repo = fx.git_repo("app", 200);
        fx.mark_pushed("app");
        assert_eq!(
            Activity::of(&repo, None, now()),
            Activity::Dead,
            "a pushed, long-idle repo is restorable by clone"
        );
    }

    #[test]
    fn unpushed_commits_are_never_dead() {
        // The whole project is restorable by clone only if the remote actually
        // has the work. An unpushed commit exists nowhere else.
        let fx = Fixture::new();
        let repo = fx.git_repo("app", 200);
        fx.mark_pushed("app");
        fx.commit_unpushed("app", 190);

        assert_ne!(
            Activity::of(&repo, None, now()),
            Activity::Dead,
            "a repo with unpushed work must never be classified dead"
        );
    }

    #[test]
    fn a_repo_without_a_remote_is_never_dead() {
        let fx = Fixture::new();
        let repo = fx.git_repo("app", 200);
        assert_ne!(
            Activity::of(&repo, None, now()),
            Activity::Dead,
            "with no remote there is nothing to clone from"
        );
    }

    #[test]
    fn recent_source_edits_keep_an_old_repo_active() {
        // Committed long ago but edited yesterday: still in use.
        let fx = Fixture::new();
        let repo = fx.git_repo("app", 200);
        fx.mark_pushed("app");
        let yesterday = now() - days(1);

        assert_eq!(
            Activity::of(&repo, Some(yesterday), now()),
            Activity::Active
        );
    }

    #[test]
    fn a_directory_without_git_falls_back_to_file_times() {
        let fx = Fixture::new();
        let plain = fx.file("loose/main.py", b"print(1)");
        let dir = plain.parent().unwrap();

        assert_eq!(
            Activity::of(dir, Some(now() - days(2)), now()),
            Activity::Active
        );
        assert_eq!(
            Activity::of(dir, Some(now() - days(300)), now()),
            Activity::Dormant,
            "without a pushed remote it can never be dead, however old"
        );
    }
}

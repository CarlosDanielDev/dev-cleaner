pub mod common;

use dev_cleaner::safety::{BlockReason, RegenCommand, Safety};

#[test]
fn a_regeneration_command_cannot_be_empty() {
    // Regenerable is the tier that makes a candidate selectable. If it could
    // hold an empty command, "we know how to bring this back" would become a
    // claim with nothing behind it.
    assert!(RegenCommand::new("cargo build").is_some());
    assert!(RegenCommand::new("").is_none());
    assert!(RegenCommand::new("   ").is_none());
    assert!(RegenCommand::new("\t\n").is_none());
}

#[test]
fn only_proven_tiers_are_selectable() {
    let regen = RegenCommand::new("npm install").expect("valid");

    assert!(
        Safety::Cache {
            refills_on: "next install"
        }
        .is_selectable()
    );
    assert!(
        Safety::Regenerable {
            regen: regen.clone()
        }
        .is_selectable()
    );

    assert!(
        !Safety::Unproven {
            reason: "unrecognised directory".into()
        }
        .is_selectable(),
        "an unproven candidate must never be selectable"
    );
    assert!(
        !Safety::Protected {
            reason: BlockReason::DirtyWorktree
        }
        .is_selectable(),
        "a protected candidate must never be selectable"
    );
}

#[test]
fn an_unknown_directory_defaults_to_unproven() {
    // Anything the registries do not recognise falls to the inspect-only tier
    // rather than being guessed at.
    let s = Safety::for_unknown("no registry entry");
    assert!(matches!(s, Safety::Unproven { .. }));
    assert!(!s.is_selectable());
}

#[test]
fn each_tier_carries_a_symbol_so_colour_is_never_the_only_signal() {
    let regen = RegenCommand::new("pod install").expect("valid");
    let tiers = [
        Safety::Cache { refills_on: "x" },
        Safety::Regenerable { regen },
        Safety::Unproven { reason: "x".into() },
        Safety::Protected {
            reason: BlockReason::Denylisted,
        },
    ];

    let symbols: Vec<char> = tiers.iter().map(|t| t.symbol()).collect();
    let mut unique = symbols.clone();
    unique.sort_unstable();
    unique.dedup();

    assert_eq!(
        unique.len(),
        symbols.len(),
        "tiers must be distinguishable without colour: {symbols:?}"
    );
}

#[test]
fn a_block_reason_explains_itself_in_plain_language() {
    for reason in BlockReason::all() {
        let text = reason.explain();
        assert!(!text.trim().is_empty(), "{reason:?} has no explanation");
        assert!(
            text.chars().next().is_some_and(|c| c.is_uppercase()),
            "{reason:?} explanation should read as a sentence: {text:?}"
        );
    }
}

mod plan {
    use dev_cleaner::safety::{BlockReason, Candidate, Plan, RegenCommand, Safety};
    use std::path::PathBuf;

    fn regenerable(name: &str, bytes: u64) -> Candidate {
        Candidate {
            path: PathBuf::from(name),
            bytes,
            safety: Safety::Regenerable {
                regen: RegenCommand::new("npm install").expect("valid"),
            },
        }
    }

    fn protected(name: &str) -> Candidate {
        Candidate {
            path: PathBuf::from(name),
            bytes: 1,
            safety: Safety::Protected {
                reason: BlockReason::DirtyWorktree,
            },
        }
    }

    fn unproven(name: &str) -> Candidate {
        Candidate {
            path: PathBuf::from(name),
            bytes: 1,
            safety: Safety::for_unknown("unrecognised"),
        }
    }

    #[test]
    fn a_draft_accepts_only_candidates_that_proved_themselves() {
        let mut draft = Plan::draft();

        assert!(draft.add(regenerable("a/node_modules", 100)).is_ok());
        assert!(
            draft.add(protected("b/.git")).is_err(),
            "a protected candidate must be refused entry to the plan"
        );
        assert!(
            draft.add(unproven("c/mystery")).is_err(),
            "an unproven candidate must be refused entry to the plan"
        );
        assert_eq!(draft.len(), 1, "only the proven candidate is in the plan");
    }

    #[test]
    fn confirmation_requires_the_exact_phrase() {
        let mut draft = Plan::draft();
        draft.add(regenerable("a", 1024)).unwrap();
        let reviewed = draft.review();

        let phrase = reviewed.confirmation_phrase();
        let reviewed = reviewed
            .confirm("yes")
            .expect_err("a guess must not confirm the plan");
        let reviewed = reviewed
            .confirm(&phrase.to_uppercase())
            .expect_err("near-misses must not confirm either");

        assert!(
            reviewed.confirm(&phrase).is_ok(),
            "the exact phrase confirms"
        );
    }

    #[test]
    fn the_phrase_changes_with_the_plan_so_it_cannot_be_typed_from_memory() {
        let mut a = Plan::draft();
        a.add(regenerable("a", 1024)).unwrap();

        let mut b = Plan::draft();
        b.add(regenerable("a", 1024)).unwrap();
        b.add(regenerable("b", 2048)).unwrap();

        assert_ne!(
            a.review().confirmation_phrase(),
            b.review().confirmation_phrase(),
            "a bigger plan must demand a different phrase"
        );
    }

    #[test]
    fn amending_a_reviewed_plan_returns_it_to_draft() {
        let mut draft = Plan::draft();
        draft.add(regenerable("a", 1024)).unwrap();
        let reviewed = draft.review();
        let phrase = reviewed.confirmation_phrase();

        let mut back = reviewed.amend();
        back.add(regenerable("b", 4096)).unwrap();
        let reviewed_again = back.review();

        assert_ne!(
            reviewed_again.confirmation_phrase(),
            phrase,
            "an amended plan must not accept the phrase shown before the change"
        );
        assert!(reviewed_again.confirm(&phrase).is_err());
    }

    #[test]
    fn a_confirmed_plan_carries_exactly_what_was_reviewed() {
        let mut draft = Plan::draft();
        draft.add(regenerable("a", 1000)).unwrap();
        draft.add(regenerable("b", 2000)).unwrap();
        let reviewed = draft.review();
        let phrase = reviewed.confirmation_phrase();

        assert_eq!(reviewed.total_bytes(), 3000);
        let confirmed = reviewed.confirm(&phrase).expect("confirms");

        assert_eq!(confirmed.items().len(), 2);
        assert_eq!(confirmed.total_bytes(), 3000);
    }
}

mod guards {
    use super::common::Fixture;
    use dev_cleaner::safety::{BlockReason, Guards};

    fn guards(fx: &Fixture) -> Guards {
        Guards::new(vec![fx.root().to_path_buf()], vec![])
    }

    #[test]
    fn a_clean_pushed_repository_passes_every_guard() {
        let fx = Fixture::new();
        let repo = fx.git_repo("app", 200);
        fx.mark_pushed("app");

        assert_eq!(guards(&fx).check(&repo), Ok(()));
    }

    #[test]
    fn uncommitted_changes_block_the_candidate() {
        let fx = Fixture::new();
        let repo = fx.git_repo("app", 200);
        fx.file("app/README.md", b"edited after the commit");

        assert_eq!(
            guards(&fx).check(&repo),
            Err(BlockReason::DirtyWorktree),
            "a modified tracked file must block, not warn"
        );
    }

    #[test]
    fn untracked_files_block_the_candidate() {
        let fx = Fixture::new();
        let repo = fx.git_repo("app", 200);
        fx.file("app/scratch.rs", b"fn main() {}");

        assert_eq!(
            guards(&fx).check(&repo),
            Err(BlockReason::UntrackedSource),
            "untracked work exists nowhere else"
        );
    }

    #[test]
    fn stashed_work_blocks_the_candidate() {
        let fx = Fixture::new();
        let repo = fx.git_repo("app", 200);
        fx.file("app/README.md", b"work in progress");
        fx.git("app", &["stash", "push", "-q", "-m", "wip"]);

        assert_eq!(guards(&fx).check(&repo), Err(BlockReason::StashEntries));
    }

    #[test]
    fn a_gitignored_artifact_survives_dirt_elsewhere_in_the_repository() {
        // node_modules is gitignored, so deleting it cannot lose uncommitted
        // source. Blocking it because src/ is dirty would make the tool useless
        // on any machine where work is in progress, without making it safer.
        let fx = Fixture::new();
        fx.git_repo("app", 10);
        fx.file("app/.gitignore", b"node_modules\n");
        fx.git("app", &["add", "-A"]);
        fx.git("app", &["commit", "-q", "-m", "ignore deps"]);
        fx.file("app/src.rs", b"uncommitted edit");
        fx.file("app/node_modules/react/index.js", b"dep");

        let artifact = fx.root().join("app/node_modules");
        assert_eq!(
            guards(&fx).check(&artifact),
            Ok(()),
            "a gitignored artifact directory is unaffected by dirt elsewhere"
        );
    }

    #[test]
    fn tracked_changes_inside_the_artifact_itself_still_block_it() {
        // A patched dependency committed into the repo is real work living in
        // a directory that otherwise looks disposable.
        let fx = Fixture::new();
        fx.git_repo("app", 10);
        fx.file("app/vendor/lib/patch.php", b"original");
        fx.git("app", &["add", "-A"]);
        fx.git("app", &["commit", "-q", "-m", "vendored"]);
        fx.file(
            "app/vendor/lib/patch.php",
            b"locally patched, never committed",
        );

        assert_eq!(
            guards(&fx).check(&fx.root().join("app/vendor")),
            Err(BlockReason::DirtyWorktree),
            "modified tracked content inside the candidate must still block"
        );
    }

    #[test]
    fn a_stash_does_not_block_a_subdirectory_because_stashes_live_in_git() {
        // Deleting a working-tree subdirectory cannot destroy a stash: stashes
        // are objects inside .git. Only removing the repository itself can.
        let fx = Fixture::new();
        fx.git_repo("app", 10);
        fx.file("app/.gitignore", b"node_modules\n");
        fx.git("app", &["add", "-A"]);
        fx.git("app", &["commit", "-q", "-m", "ignore deps"]);
        fx.file("app/README.md", b"work in progress");
        fx.git("app", &["stash", "push", "-q", "-m", "wip"]);
        fx.file("app/node_modules/react/index.js", b"dep");

        assert_eq!(
            guards(&fx).check(&fx.root().join("app/node_modules")),
            Ok(()),
            "the stash is safe in .git; the artifact directory is not part of it"
        );
    }

    #[test]
    fn paths_outside_every_root_are_blocked() {
        let inside = Fixture::new();
        let outside = Fixture::new();
        let stray = outside.file("elsewhere/file.txt", b"x");

        assert_eq!(
            guards(&inside).check(&stray),
            Err(BlockReason::OutsideRoots)
        );
    }

    #[test]
    fn traversal_cannot_smuggle_a_path_past_the_root_check() {
        let inside = Fixture::new();
        let outside = Fixture::new();
        outside.file("secret/data.txt", b"x");
        inside.file("real/keep.txt", b"y");

        // Textually rooted, but resolves outside.
        let sneaky = inside
            .root()
            .join("real/../..")
            .join(outside.root().file_name().unwrap())
            .join("secret/data.txt");

        assert_eq!(
            guards(&inside).check(&sneaky),
            Err(BlockReason::OutsideRoots),
            "guards must run after canonicalisation"
        );
    }

    #[test]
    fn a_symlink_leading_out_of_the_roots_is_blocked() {
        let inside = Fixture::new();
        let outside = Fixture::new();
        outside.file("vault/keys.txt", b"secret");
        let link = inside.symlink_to("escape", outside.root());

        assert_eq!(
            guards(&inside).check(&link.join("vault")),
            Err(BlockReason::SymlinkEscape)
        );
    }

    #[test]
    fn denylisted_paths_are_blocked_even_inside_a_root() {
        let fx = Fixture::new();
        let protected = fx.file("kyte/payroll.txt", b"work");
        let g = Guards::new(vec![fx.root().to_path_buf()], vec![fx.root().join("kyte")]);

        assert_eq!(g.check(&protected), Err(BlockReason::Denylisted));
    }

    #[test]
    fn a_repository_whose_state_cannot_be_read_is_blocked() {
        // Failing closed: if git cannot answer, the tool must not assume clean.
        let fx = Fixture::new();
        fx.file("app/.git/HEAD", b"this is not a repository");
        let candidate = fx.root().join("app");

        assert_eq!(
            guards(&fx).check(&candidate),
            Err(BlockReason::DirtyWorktree),
            "an unreadable repository must block rather than pass"
        );
    }
}

mod docker {
    use dev_cleaner::safety::{BlockReason, DockerTarget, Safety};

    #[test]
    fn volumes_are_protected_and_offer_no_command() {
        // docker system df advertises volumes as reclaimable. On the reference
        // machine those volumes were astral-system_postgres_data and
        // astral-system_qdrant_storage: a live database and a vector store.
        // Nothing regenerates them.
        let v = DockerTarget::Volumes;

        assert_eq!(
            v.safety(),
            Safety::Protected {
                reason: BlockReason::DockerVolume
            }
        );
        assert!(!v.safety().is_selectable());
        assert!(
            v.prune_args().is_none(),
            "there must be no command that prunes volumes"
        );
    }

    #[test]
    fn no_prune_command_can_ever_touch_volumes() {
        // The guarantee stated as a property over every target, so a new one
        // added later cannot quietly introduce the flag.
        for target in DockerTarget::all() {
            let Some(args) = target.prune_args() else {
                continue;
            };
            assert!(
                !args.iter().any(|a| *a == "--volumes" || *a == "-v"),
                "{target:?} would pass a volume-pruning flag: {args:?}"
            );
        }
    }

    #[test]
    fn images_build_cache_and_stopped_containers_remain_offerable() {
        for target in [
            DockerTarget::Images,
            DockerTarget::BuildCache,
            DockerTarget::StoppedContainers,
        ] {
            assert!(
                target.safety().is_selectable(),
                "{target:?} should be offerable"
            );
            assert!(
                target.prune_args().is_some(),
                "{target:?} should carry a command"
            );
        }
    }
}

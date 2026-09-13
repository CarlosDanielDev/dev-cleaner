//! The screen router: where the interface can go, and what that does to the plan.

use dev_cleaner::safety::{Candidate, Plan, RegenCommand, Safety};
use dev_cleaner::tui::{App, Screen};
use std::path::PathBuf;

fn candidate(name: &str, bytes: u64) -> Candidate {
    Candidate {
        path: PathBuf::from(name),
        bytes,
        safety: Safety::Regenerable {
            regen: RegenCommand::new("npm install").expect("valid"),
        },
    }
}

fn app_with(items: &[(&str, u64)]) -> App {
    let mut draft = Plan::draft();
    for (name, bytes) in items {
        draft.add(candidate(name, *bytes)).expect("selectable");
    }
    App::new(draft)
}

#[test]
fn the_router_runs_with_no_terminal_attached() {
    // The whole point of separating routing from drawing: this test drives the
    // entire flow in CI, where there is no tty, no crossterm and no raw mode.
    let mut app = app_with(&[("a/node_modules", 1024)]);
    assert_eq!(app.screen(), Screen::Dashboard);

    for expected in [
        Screen::Projects,
        Screen::Candidates,
        Screen::Review,
        Screen::Confirm,
    ] {
        app = app.forward();
        assert_eq!(app.screen(), expected);
    }
}

#[test]
fn the_first_screen_is_the_dashboard_and_back_stays_put() {
    let app = app_with(&[("a", 1)]);
    assert_eq!(app.screen(), Screen::Dashboard);

    // Nowhere to go back to. Leaving is quitting, which is not the router's job.
    assert_eq!(app.back().screen(), Screen::Dashboard);
}

#[test]
fn reaching_review_reviews_the_plan_itself() {
    // The screen does not merely say "review"; the plan is in its reviewed
    // state, which is what makes a confirmation phrase exist at all.
    let app = app_with(&[("a", 1000), ("b", 2000)])
        .forward()
        .forward()
        .forward();

    assert_eq!(app.screen(), Screen::Review);
    assert_eq!(
        app.phrase().expect("a reviewed plan has a phrase"),
        "purge 2 items 3000 bytes"
    );
}

#[test]
fn a_browsing_screen_has_no_confirmation_phrase_to_show() {
    for app in [
        app_with(&[("a", 1)]),
        app_with(&[("a", 1)]).forward(),
        app_with(&[("a", 1)]).forward().forward(),
    ] {
        assert!(
            app.phrase().is_none(),
            "{:?} is still assembling the plan; there is nothing to confirm yet",
            app.screen()
        );
    }
}

#[test]
fn going_back_from_review_returns_the_plan_to_a_draft() {
    let app = app_with(&[("a", 1)]).forward().forward().forward();
    assert_eq!(app.screen(), Screen::Review);

    let back = app.back();

    assert_eq!(back.screen(), Screen::Candidates);
    assert!(
        back.phrase().is_none(),
        "a plan being edited again must not still offer a phrase"
    );
}

#[test]
fn back_from_review_discards_confirmation_it_never_preserves_it() {
    // The phrase describes one exact plan. Editing the plan and returning must
    // not let the earlier phrase through, or a user could approve one set and
    // delete another.
    let app = app_with(&[("a", 1000)]).forward().forward().forward();
    let stale = app.phrase().expect("reviewed");

    let mut draft = app.back();
    draft.add(candidate("b", 2000)).expect("selectable");
    let reviewed = draft.forward();

    assert_ne!(
        reviewed.phrase().expect("reviewed again"),
        stale,
        "an amended plan must demand a different phrase"
    );
    assert!(
        reviewed.confirm(&stale).is_err(),
        "the phrase from before the edit must be refused"
    );
}

#[test]
fn confirmation_is_only_possible_on_the_confirm_screen() {
    let phrase = "purge 1 items 1000 bytes";

    // Every screen before Confirm refuses, including Review itself: seeing the
    // plan and approving it are deliberately two separate steps.
    for app in [
        app_with(&[("a", 1000)]),
        app_with(&[("a", 1000)]).forward(),
        app_with(&[("a", 1000)]).forward().forward(),
        app_with(&[("a", 1000)]).forward().forward().forward(),
    ] {
        let screen = app.screen();
        assert!(
            app.confirm(phrase).is_err(),
            "{screen:?} must not be able to confirm"
        );
    }

    let confirm = app_with(&[("a", 1000)])
        .forward()
        .forward()
        .forward()
        .forward();
    assert_eq!(confirm.screen(), Screen::Confirm);
    let plan = confirm
        .confirm(phrase)
        .expect("the right phrase on the right screen");
    assert_eq!(plan.total_bytes(), 1000);
}

#[test]
fn a_wrong_phrase_hands_the_app_back_instead_of_losing_the_plan() {
    let app = app_with(&[("a", 1000)])
        .forward()
        .forward()
        .forward()
        .forward();

    let Err(returned) = app.confirm("purge 9 items 9 bytes") else {
        panic!("a wrong phrase must not confirm");
    };

    assert_eq!(
        returned.screen(),
        Screen::Confirm,
        "the user stays where they were, with the plan intact"
    );
    assert_eq!(
        returned.phrase().as_deref(),
        Some("purge 1 items 1000 bytes")
    );
}

#[test]
fn the_result_screen_is_the_end_of_the_road() {
    let app = app_with(&[("a", 1)])
        .forward()
        .forward()
        .forward()
        .forward();
    let done = app.finished();

    assert_eq!(done.screen(), Screen::Result);
    assert_eq!(
        done.forward().screen(),
        Screen::Result,
        "nothing follows a result"
    );

    let done = app_with(&[("a", 1)])
        .forward()
        .forward()
        .forward()
        .forward()
        .finished();
    assert_eq!(
        done.back().screen(),
        Screen::Result,
        "a purge that happened cannot be navigated away from into a stale plan"
    );
}

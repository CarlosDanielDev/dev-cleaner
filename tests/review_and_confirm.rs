//! The last two screens: reading the plan, and holding a key to carry it out.

use dev_cleaner::safety::{Candidate, Plan, RegenCommand, Reviewed, Safety};
use dev_cleaner::tui::{App, Confirm, Motion, Review, Screen};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use std::path::PathBuf;
use std::time::Duration;

const MB: u64 = 1024 * 1024;

fn regenerable(path: &str, bytes: u64, regen: &str) -> Candidate {
    Candidate {
        path: PathBuf::from(path),
        bytes,
        safety: Safety::Regenerable {
            regen: RegenCommand::new(regen).expect("valid"),
        },
    }
}

fn cache(path: &str, bytes: u64, refills_on: &'static str) -> Candidate {
    Candidate {
        path: PathBuf::from(path),
        bytes,
        safety: Safety::Cache { refills_on },
    }
}

fn plan_of(items: Vec<Candidate>) -> Plan<Reviewed> {
    let mut draft = Plan::draft();
    for item in items {
        draft
            .add(item)
            .expect("the fixture holds only selectable tiers");
    }
    draft.review()
}

fn plan() -> Plan<Reviewed> {
    plan_of(vec![
        regenerable("/p/a/node_modules", 120 * MB, "npm install"),
        cache("/p/c/.gradle", 5 * MB, "the next build"),
        regenerable("/p/e/target", 1500 * MB, "cargo build"),
    ])
}

fn text(draw: impl FnOnce(Rect, &mut Buffer)) -> String {
    let area = Rect::new(0, 0, 110, 30);
    let mut buf = Buffer::empty(area);
    draw(area, &mut buf);
    (0..area.height)
        .map(|y| {
            (0..area.width)
                .map(|x| buf[(x, y)].symbol())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn reviewed(review: &Review, plan: &Plan<Reviewed>) -> String {
    text(|area, buf| review.render(plan, area, buf))
}

#[test]
fn every_item_shows_the_command_that_brings_it_back() {
    // The rule the whole tool rests on, at the last moment it can still be
    // read: nothing is offered for deletion that the tool cannot name a way
    // back from.
    let plan = plan();
    let out = reviewed(&Review::new(), &plan);

    for expected in ["npm install", "the next build", "cargo build"] {
        assert!(
            out.contains(expected),
            "no way back shown for {expected:?}:\n{out}"
        );
    }
}

#[test]
fn every_item_shows_its_path_and_its_size() {
    let plan = plan();
    let out = reviewed(&Review::new(), &plan);

    for path in ["/p/a/node_modules", "/p/c/.gradle", "/p/e/target"] {
        assert!(out.contains(path), "{path} is missing:\n{out}");
    }
    assert!(out.contains("120.00 MB"), "a byte count is missing:\n{out}");
    assert!(out.contains("1.46 GB"), "a byte count is missing:\n{out}");
}

#[test]
fn the_total_shown_is_the_plans_own_total() {
    let plan = plan();
    let out = reviewed(&Review::new(), &plan);

    assert!(
        out.contains(&dev_cleaner::bytes::human(plan.total_bytes())),
        "the screen must show what the whole plan costs:\n{out}"
    );
    assert!(
        out.contains("3 items"),
        "the screen must show how many items:\n{out}"
    );
}

#[test]
fn a_long_path_does_not_collide_with_its_command() {
    let long =
        "/Users/carlos/projects/.worktrees/akasha-limpar-guardrails/src/akasha/domain/__pycache__";
    let plan = plan_of(vec![regenerable(long, MB, "uv sync")]);
    let out = reviewed(&Review::new(), &plan);

    let row = out
        .lines()
        .find(|l| l.contains("uv sync"))
        .unwrap_or_else(|| panic!("no row showed the command:\n{out}"));
    let after = row.split("uv sync").nth(1).unwrap_or("").trim();
    assert!(
        after.is_empty(),
        "{after:?} follows the last column, so one field was drawn over another:\n{row}"
    );
    assert!(
        row.contains(long) || row.contains('…'),
        "a path cut short without saying so reads as a different path:\n{row}"
    );
}

#[test]
fn every_item_of_a_long_plan_can_be_brought_into_view() {
    // A plan longer than the screen must still be readable in full. Approving
    // what cannot be read is the thing review exists to prevent.
    let items: Vec<Candidate> = (0..40)
        .map(|i| regenerable(&format!("/p/{i}/node_modules"), MB, "npm install"))
        .collect();
    let plan = plan_of(items);
    let height = 10;
    let mut review = Review::new();

    let mut seen = 0;
    for _ in 0..plan.items().len() * 2 {
        seen += review.visible(&plan, height).len().min(1);
        review.scroll(Motion::Down, &plan, height);
    }
    assert!(seen > 0);

    let last = review.visible(&plan, height).last().expect("rows visible");
    assert_eq!(
        last.path,
        PathBuf::from("/p/39/node_modules"),
        "scrolling to the end must reach the last item"
    );

    review.scroll(Motion::Top, &plan, height);
    assert_eq!(
        review.visible(&plan, height)[0].path,
        PathBuf::from("/p/0/node_modules")
    );
}

#[test]
fn scrolling_never_runs_off_either_end() {
    let plan = plan();
    let mut review = Review::new();
    for motion in [Motion::Up, Motion::PageUp, Motion::Top, Motion::Up] {
        review.scroll(motion, &plan, 2);
        assert_eq!(review.visible(&plan, 2).len(), 2);
    }
    for motion in [Motion::Down, Motion::PageDown, Motion::Bottom, Motion::Down] {
        review.scroll(motion, &plan, 2);
        assert_eq!(review.visible(&plan, 2).len(), 2, "the window stays full");
    }
}

/// An app parked on the confirmation screen, with a plan worth 1000 bytes.
fn on_confirm() -> App {
    let mut draft = Plan::draft();
    draft
        .add(regenerable("/p/a/node_modules", 1000, "npm install"))
        .expect("selectable");
    App::new(draft).forward().forward().forward().forward()
}

/// Hold for `total`, in the small steps a key repeat actually delivers.
fn hold_for(confirm: &mut Confirm, total: Duration) -> bool {
    let step = Duration::from_millis(16);
    let mut spent = Duration::ZERO;
    let mut armed = false;
    while spent < total {
        armed |= confirm.hold(step);
        spent += step;
    }
    armed
}

#[test]
fn a_hold_shorter_than_the_threshold_never_arms() {
    let mut confirm = Confirm::new();
    let armed = hold_for(&mut confirm, Confirm::HOLD / 2);

    assert!(!armed, "half a hold must not arm the purge");
    assert!(!confirm.is_armed());
    assert!(
        confirm.progress() > 0.0,
        "the bar must show it was being held"
    );
    assert!(confirm.progress() < 1.0);
}

#[test]
fn releasing_early_cancels_without_the_plan_ever_being_confirmed() {
    // Cancelling is not an undo. The screen simply never calls App::confirm,
    // so there is nothing to unwind: the app is still sitting on the same plan.
    let app = on_confirm();
    let mut confirm = Confirm::new();
    hold_for(&mut confirm, Confirm::HOLD / 2);
    confirm.release();

    assert!(!confirm.is_armed(), "a released hold arms nothing");
    assert_eq!(confirm.progress(), 0.0, "the bar returns to empty");
    assert_eq!(app.screen(), Screen::Confirm);
    assert_eq!(app.phrase().as_deref(), Some("purge 1 items 1000 bytes"));

    // And holding again starts from nothing rather than from where it stopped.
    let armed = hold_for(&mut confirm, Confirm::HOLD / 2);
    assert!(!armed, "a second half-hold must not complete the first");
}

#[test]
fn a_full_hold_is_what_lets_the_plan_be_confirmed() {
    let app = on_confirm();
    let phrase = app.phrase().expect("a reviewed plan has a phrase");
    let mut confirm = Confirm::new();

    assert!(hold_for(&mut confirm, Confirm::HOLD), "a full hold arms");
    assert!(confirm.is_armed());
    assert_eq!(confirm.progress(), 1.0);

    let plan = app.confirm(&phrase).expect("armed, on the confirm screen");
    assert_eq!(plan.total_bytes(), 1000);
}

#[test]
fn one_event_carrying_a_long_delta_cannot_arm_the_purge() {
    // A stalled frame, a debugger, a laptop waking from sleep: any of them can
    // hand the screen a delta longer than the whole threshold. Arming on it
    // would turn a single keypress into a purge, which is the exact slip
    // hold-to-confirm exists to prevent.
    let mut confirm = Confirm::new();

    assert!(!confirm.hold(Confirm::HOLD * 10), "one event is one press");
    assert!(!confirm.is_armed());
    assert!(confirm.progress() < 1.0);
}

#[test]
fn the_hold_arms_once_and_not_again_while_it_is_held() {
    let mut confirm = Confirm::new();
    assert!(hold_for(&mut confirm, Confirm::HOLD));

    let again = hold_for(&mut confirm, Confirm::HOLD);
    assert!(
        !again,
        "an armed hold that keeps being held must not fire a second purge"
    );
}

#[test]
fn the_confirm_screen_says_what_will_happen_and_how_to_stop_it() {
    let plan = plan();
    let mut confirm = Confirm::new();
    hold_for(&mut confirm, Confirm::HOLD / 3);
    let out = text(|area, buf| confirm.render(&plan, area, buf));

    assert!(
        out.contains("1.59 GB"),
        "the size at stake is missing:\n{out}"
    );
    assert!(
        out.contains("3 items"),
        "the number of items is missing:\n{out}"
    );
    assert!(
        out.to_lowercase().contains("release"),
        "the way out must be on screen:\n{out}"
    );
    assert!(
        out.to_lowercase().contains("trash"),
        "where the files go must be on screen:\n{out}"
    );
}

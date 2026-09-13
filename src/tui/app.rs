use crate::safety::{Candidate, Confirmed, Draft, Plan, Rejected, Reviewed};

/// Which screen is on show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Dashboard,
    Projects,
    Candidates,
    Review,
    Confirm,
    Result,
}

/// Where the interface is, holding the plan at whatever state it has reached.
///
/// The plan is carried *inside* the stage rather than beside it. A separate
/// screen field and plan field could disagree — the interface showing a review
/// of a plan still being edited, or offering a confirmation for a set that has
/// since changed — and reconciling them would mean restating the rules the
/// type-state plan already enforces. Carried this way, the screen is a fact
/// about the plan's type rather than a claim about it.
#[derive(Debug)]
enum Stage {
    Dashboard(Plan<Draft>),
    Projects(Plan<Draft>),
    Candidates(Plan<Draft>),
    Review(Plan<Reviewed>),
    Confirm(Plan<Reviewed>),
    /// A purge has happened. There is no plan any more, and no way back to one.
    ///
    /// ponytail: carries nothing yet. The result screen needs the manifest to
    /// draw, which arrives with that screen rather than ahead of it.
    Finished,
}

/// The screen router.
///
/// Navigation consumes the app and returns the next one, because the plan
/// transitions it performs — `review`, `amend` — consume the plan. Advancing
/// and going back are therefore the same kind of move as the plan's own, and
/// cannot be applied to a plan that has already moved on.
#[derive(Debug)]
pub struct App {
    stage: Stage,
}

impl App {
    /// Start at the dashboard with a plan still being assembled.
    pub fn new(plan: Plan<Draft>) -> Self {
        Self {
            stage: Stage::Dashboard(plan),
        }
    }

    pub fn screen(&self) -> Screen {
        match self.stage {
            Stage::Dashboard(_) => Screen::Dashboard,
            Stage::Projects(_) => Screen::Projects,
            Stage::Candidates(_) => Screen::Candidates,
            Stage::Review(_) => Screen::Review,
            Stage::Confirm(_) => Screen::Confirm,
            Stage::Finished => Screen::Result,
        }
    }

    /// Advance one screen.
    ///
    /// Forward never deletes anything. The step out of `Confirm` is not a move
    /// at all: it requires [`App::confirm`], so no sequence of the key that
    /// means "next" can reach a purge.
    pub fn forward(self) -> Self {
        let stage = match self.stage {
            Stage::Dashboard(plan) => Stage::Projects(plan),
            Stage::Projects(plan) => Stage::Candidates(plan),
            // The plan itself moves to reviewed here, which is what gives it a
            // confirmation phrase.
            Stage::Candidates(plan) => Stage::Review(plan.review()),
            Stage::Review(plan) => Stage::Confirm(plan),
            Stage::Confirm(plan) => Stage::Confirm(plan),
            Stage::Finished => Stage::Finished,
        };
        Self { stage }
    }

    /// Go back one screen.
    ///
    /// Going back from `Review` calls `amend`, returning the plan to a draft.
    /// Any confirmation the user had given is discarded with it: the phrase is
    /// derived from the plan's contents, so one taken before an edit cannot
    /// approve what comes after it.
    pub fn back(self) -> Self {
        let stage = match self.stage {
            // Nothing precedes the dashboard. Leaving is quitting, which is the
            // caller's decision rather than a move within the flow.
            Stage::Dashboard(plan) => Stage::Dashboard(plan),
            Stage::Projects(plan) => Stage::Dashboard(plan),
            Stage::Candidates(plan) => Stage::Projects(plan),
            Stage::Review(plan) => Stage::Candidates(plan.amend()),
            Stage::Confirm(plan) => Stage::Review(plan),
            // A purge that happened cannot be navigated back into a plan that
            // described the disk as it was beforehand.
            Stage::Finished => Stage::Finished,
        };
        Self { stage }
    }

    /// The phrase this exact plan requires, once there is a plan to confirm.
    ///
    /// `None` while the plan is still being assembled: there is nothing to
    /// approve until the user has been shown what they are approving.
    pub fn phrase(&self) -> Option<String> {
        match &self.stage {
            Stage::Review(plan) | Stage::Confirm(plan) => Some(plan.confirmation_phrase()),
            _ => None,
        }
    }

    /// Add to the plan while it is still a draft.
    ///
    /// Refusals come from the plan, so a candidate the guards blocked cannot be
    /// added through the interface any more than through the command line.
    pub fn add(&mut self, candidate: Candidate) -> Result<(), Rejected> {
        match &mut self.stage {
            Stage::Dashboard(plan) | Stage::Projects(plan) | Stage::Candidates(plan) => {
                plan.add(candidate)
            }
            _ => Err(Rejected {
                path: candidate.path,
                because: "this plan is being confirmed; go back to change it".to_string(),
            }),
        }
    }

    /// Confirm the plan, yielding one that may be executed.
    ///
    /// Only the confirm screen accepts this. Review shows the plan and confirm
    /// approves it, deliberately as two steps, so that reaching the screen
    /// where a plan can be approved is itself something the user chose.
    ///
    /// A refusal hands the app back rather than dropping it, so a mistyped
    /// phrase costs a keystroke instead of the plan.
    pub fn confirm(self, typed: &str) -> Result<Plan<Confirmed>, Self> {
        let Stage::Confirm(plan) = self.stage else {
            return Err(self);
        };
        plan.confirm(typed).map_err(|plan| Self {
            stage: Stage::Confirm(plan),
        })
    }

    /// Move to the result screen once a purge has run.
    pub fn finished(self) -> Self {
        Self {
            stage: Stage::Finished,
        }
    }
}

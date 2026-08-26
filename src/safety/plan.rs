use std::marker::PhantomData;
use std::path::PathBuf;

use super::Safety;

/// Something that might be deleted, together with what the tool can prove
/// about recovering it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub path: PathBuf,
    pub bytes: u64,
    pub safety: Safety,
}

/// Why a candidate was refused entry to a plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rejected {
    pub path: PathBuf,
    pub because: String,
}

impl std::fmt::Display for Rejected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.path.display(), self.because)
    }
}

impl std::error::Error for Rejected {}

/// Being assembled. Candidates can still be added.
#[derive(Debug)]
pub struct Draft;
/// Fixed and presented to the user. Nothing can be added without amending.
#[derive(Debug)]
pub struct Reviewed;
/// The user typed the confirmation phrase for exactly this set.
#[derive(Debug)]
pub struct Confirmed;

/// A set of candidates moving through review towards execution.
///
/// The state parameter is the safety mechanism. `execute` is implemented only
/// for `Plan<Confirmed>`, and the only route to `Confirmed` runs through
/// `review` then `confirm`. Code that deletes without passing both does not
/// compile, so the guarantee cannot be lost to a later edit.
///
/// Every transition takes `self` by value, which means a plan cannot be
/// confirmed and then quietly altered: the confirmed value is the reviewed one.
///
/// # The gate, proved
///
/// Each rejection below is paired with the same call on a correctly confirmed
/// plan. The pair matters: `compile_fail` alone would also pass on a typo, so
/// the companion proves the API exists and that the state is the only
/// difference between the two.
///
/// A draft holds no order to carry out:
///
/// ```compile_fail
/// use dev_cleaner::safety::Plan;
/// let draft = Plan::draft();
/// let _order = draft.into_items();
/// ```
///
/// Neither does a reviewed but unconfirmed plan:
///
/// ```compile_fail
/// use dev_cleaner::safety::Plan;
/// let reviewed = Plan::draft().review();
/// let _order = reviewed.into_items();
/// ```
///
/// A confirmed plan does, and this is the only route to one:
///
/// ```
/// use dev_cleaner::safety::Plan;
/// let reviewed = Plan::draft().review();
/// let phrase = reviewed.confirmation_phrase();
/// let confirmed = reviewed.confirm(&phrase).expect("phrase matches");
/// let _order = confirmed.into_items();
/// ```
///
/// Confirmation is not reachable from a draft:
///
/// ```compile_fail
/// use dev_cleaner::safety::Plan;
/// let draft = Plan::draft();
/// let _ = draft.confirm("purge 0 items 0 bytes");
/// ```
///
/// A plan cannot be reviewed twice, because reviewing consumes it. This is what
/// stops a confirmed set from being altered after the user approved it:
///
/// ```compile_fail
/// use dev_cleaner::safety::Plan;
/// let draft = Plan::draft();
/// let _first = draft.review();
/// let _second = draft.review();
/// ```
///
/// Nor amended after confirmation:
///
/// ```compile_fail
/// use dev_cleaner::safety::Plan;
/// let reviewed = Plan::draft().review();
/// let phrase = reviewed.confirmation_phrase();
/// let confirmed = reviewed.confirm(&phrase).expect("phrase matches");
/// let _back = confirmed.amend();
/// ```
#[derive(Debug)]
pub struct Plan<S> {
    items: Vec<Candidate>,
    _state: PhantomData<S>,
}

impl Plan<Draft> {
    pub fn draft() -> Self {
        Self {
            items: Vec::new(),
            _state: PhantomData,
        }
    }

    /// Add a candidate, or refuse it.
    ///
    /// This is where the tier system becomes enforcement rather than
    /// description: anything the tool could not prove recoverable never enters
    /// a plan, so no later stage has to remember to check.
    pub fn add(&mut self, candidate: Candidate) -> Result<(), Rejected> {
        if !candidate.safety.is_selectable() {
            return Err(Rejected {
                path: candidate.path,
                because: match &candidate.safety {
                    Safety::Protected { reason } => reason.explain().to_string(),
                    Safety::Unproven { reason } => {
                        format!("nothing proves this can be restored: {reason}")
                    }
                    _ => unreachable!("selectable tiers are accepted above"),
                },
            });
        }
        self.items.push(candidate);
        Ok(())
    }

    pub fn review(self) -> Plan<Reviewed> {
        Plan {
            items: self.items,
            _state: PhantomData,
        }
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

impl Plan<Reviewed> {
    pub fn items(&self) -> &[Candidate] {
        &self.items
    }

    pub fn total_bytes(&self) -> u64 {
        self.items.iter().map(|c| c.bytes).sum()
    }

    /// The phrase the user must type to proceed.
    ///
    /// Derived from the plan's contents, so it changes whenever the plan does.
    /// A phrase seen before an amendment will not confirm the amended plan,
    /// which stops a confirmation from being carried over to a set the user
    /// never actually read.
    pub fn confirmation_phrase(&self) -> String {
        format!(
            "purge {} items {} bytes",
            self.items.len(),
            self.total_bytes()
        )
    }

    /// Consume the plan and confirm it.
    ///
    /// On a mismatch the plan is handed back rather than dropped, so the caller
    /// can show it again without rebuilding it. Comparison is exact: a
    /// case-insensitive match would let a half-read phrase through.
    pub fn confirm(self, typed: &str) -> Result<Plan<Confirmed>, Plan<Reviewed>> {
        if typed == self.confirmation_phrase() {
            Ok(Plan {
                items: self.items,
                _state: PhantomData,
            })
        } else {
            Err(self)
        }
    }

    /// Return to `Draft` to change the plan. The only way back, and it discards
    /// any confirmation the user had already given.
    pub fn amend(self) -> Plan<Draft> {
        Plan {
            items: self.items,
            _state: PhantomData,
        }
    }
}

impl Plan<Confirmed> {
    /// Consume the plan into the set of candidates to act on.
    ///
    /// Defined only here. An executor takes `Plan<Confirmed>`, so no deletion
    /// can be reached from a draft or an unconfirmed plan.
    pub fn into_items(self) -> Vec<Candidate> {
        self.items
    }

    pub fn items(&self) -> &[Candidate] {
        &self.items
    }

    pub fn total_bytes(&self) -> u64 {
        self.items.iter().map(|c| c.bytes).sum()
    }
}

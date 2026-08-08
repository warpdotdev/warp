//! The nag engine: when a blocked agent is announced, how often it is
//! announced again, and what finally shuts it up.
//!
//! Spec §6 (`specs/samithaj/rail-triage/plan.md`). The one-shot desktop
//! notification Warp already sends says "an agent is blocked" exactly once, to
//! a user who may not be at the keyboard. The whole point of this layer is that
//! a *waiting* agent keeps costing time until someone answers it, so the
//! announcement repeats until the wait actually ends — scaled by the project's
//! priority rank, because a rank-1 fire deserves interrupting for and a scratch
//! project does not.
//!
//! Everything here is a pure function of (observed blocked tasks, `now`), with
//! the clock injected — the same split [`rail_triage`](super::rail_triage) uses
//! — so the cadence, the debounce, the acknowledgement grace and the coalesced
//! banner text are all unit-testable without a timer, a window or a
//! notification centre. The `Workspace` owns the timer and the delivery; this
//! module owns the rules.

use std::collections::HashMap;
use std::time::Duration;

use instant::Instant;
use warpui::EntityId;

/// How long an *unranked* project's agent must stay blocked before it is
/// announced at all.
///
/// The debounce is the feature's survival (spec §8, nag fatigue): a permission
/// prompt the user answers in ten seconds should never have interrupted
/// anything. Ranked projects deliberately skip it — being interrupted
/// immediately is precisely what rank buys.
pub const UNRANKED_DEBOUNCE: Duration = Duration::from_secs(60);

/// How often a *ranked* project's blocked agent is re-announced.
pub const RANKED_REPEAT_INTERVAL: Duration = Duration::from_secs(3 * 60);

/// How often an *unranked* project's blocked agent is re-announced. Five times
/// slower than a ranked one: still unmissable over an afternoon, cheap enough
/// that a forgotten scratch tab is not a reason to turn notifications off.
pub const UNRANKED_REPEAT_INTERVAL: Duration = Duration::from_secs(15 * 60);

/// How long after the user stops looking at a blocked task the nag resumes.
///
/// Looking at the task silences it (see [`BlockedTask::in_view`]), but only
/// *answering* it stops the nag for good. Without this the easiest way to
/// mute a blocked agent forever would be to glance at its tab and walk away —
/// which is the exact failure the repeat exists to prevent.
pub const ACKNOWLEDGE_GRACE: Duration = Duration::from_secs(2 * 60);

/// The longest the engine will go between polls while anything is waiting.
///
/// A user looking at a blocked task is not an event this module can subscribe
/// to — it is a fact read off the workspace at poll time — so the poll interval
/// is also the worst-case lateness of both silencing and re-arming. 30s matches
/// the rail's own wait-age refresh (`RAIL_WAIT_AGE_REFRESH_INTERVAL`), which
/// already runs under exactly this condition, so this adds no new class of
/// background work.
pub const FOCUS_POLL_INTERVAL: Duration = Duration::from_secs(30);

/// How many distinct projects the coalesced banner names before it falls back
/// to "+k". Two fits a macOS banner's body line without truncation, which is
/// the constraint that picked the number.
const MAX_NAMED_PROJECTS: usize = 2;

/// One blocked agent as the engine sees it on a single poll.
///
/// Everything here is re-read every poll rather than remembered, so a project
/// that gains a rank, or a task the user switches to, takes effect at the next
/// poll without any invalidation step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockedTask {
    /// The terminal view behind the blocked pane — the same identity the rail
    /// and [`CLIAgentSessionsModel`](crate::terminal::cli_agent_sessions::CLIAgentSessionsModel)
    /// key sessions on.
    pub id: EntityId,
    /// Whether this task's project is in the user's priority list. Decides
    /// both the debounce and the repeat cadence.
    pub ranked: bool,
    /// The project's rail label, used to name it in a coalesced banner.
    pub project: String,
    /// Whether the user is looking at this task right now.
    ///
    /// The rail triages per *tab*, so this does too: the task's tab is the
    /// active tab of a frontmost window. A blocked agent the user is already
    /// staring at needs no banner and no sound.
    pub in_view: bool,
}

impl BlockedTask {
    /// This task's repeat cadence.
    fn repeat_interval(&self) -> Duration {
        if self.ranked {
            RANKED_REPEAT_INTERVAL
        } else {
            UNRANKED_REPEAT_INTERVAL
        }
    }
}

/// Where one tracked task is in its nag cycle.
///
/// Every variant carries an *absolute deadline* rather than an elapsed time, so
/// the engine never subtracts two instants — which is also what makes
/// [`NagEngine::next_poll_delay`] a plain `min` over the tracked set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NagPhase {
    /// Blocked, but not announced yet: the unranked debounce is still running.
    /// A task that unblocks from here was never announced at all.
    Debouncing { due: Instant },
    /// Announced at least once; announced again at `due`.
    Armed { due: Instant },
    /// Silenced because the user looked at it. `rearm_at` is `None` while the
    /// task is still in view, and the end of the grace period once it is not.
    Acknowledged { rearm_at: Option<Instant> },
}

impl NagPhase {
    /// The next moment worth waking up for, or `None` when the phase is
    /// waiting on something only a poll can observe (the user looking away).
    fn deadline(self) -> Option<Instant> {
        match self {
            NagPhase::Debouncing { due } | NagPhase::Armed { due } => Some(due),
            NagPhase::Acknowledged { rearm_at } => rearm_at,
        }
    }
}

/// The coalesced banner for several agents waiting at once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NagSummary {
    pub title: String,
    pub body: String,
}

/// What one poll decided.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NagOutcome {
    /// The tasks this poll announces, in the order they were observed. Empty
    /// when nothing fired.
    ///
    /// This is every task currently past its debounce and unacknowledged — not
    /// only the ones whose own cadence came due. One agent falling due is the
    /// trigger, but the announcement then speaks for all of them, which is what
    /// makes the banner honest about how many are waiting and is why the
    /// coalescing can never degrade into one banner per session.
    pub announced: Vec<EntityId>,
    /// The coalesced banner text, present only when more than one task is
    /// announced. With a single waiter the caller has something better to say:
    /// the agent's own prompt.
    pub summary: Option<NagSummary>,
    /// How long until the engine wants to be polled again, or `None` when
    /// nothing is being tracked and the timer should be torn down.
    pub next_poll: Option<Duration>,
}

/// Per-task nag state for one workspace.
///
/// Tracks only what it has been told is blocked: [`Self::poll`] takes the full
/// set every time and drops everything else, so a session that unblocks, whose
/// pane closes, or whose window goes away stops nagging by construction rather
/// than by anybody remembering to cancel it (spec §8 — a dead session must
/// never nag forever).
#[derive(Debug, Default)]
pub struct NagEngine {
    tracked: HashMap<EntityId, NagPhase>,
}

impl NagEngine {
    /// Forgets every tracked task.
    ///
    /// Used when the feature is switched off underneath the engine (the
    /// notification setting, or the project layout): re-enabling it should
    /// start a fresh cycle rather than resume mid-cadence with deadlines
    /// computed for a state of the world nobody was watching.
    pub fn reset(&mut self) {
        self.tracked.clear();
    }

    /// Advances every tracked task to `now` and decides what to announce.
    ///
    /// `blocked` is the complete set of currently-blocked tasks in this
    /// workspace; anything absent from it has left `Blocked` and is dropped.
    pub fn poll(&mut self, blocked: &[BlockedTask], now: Instant) -> NagOutcome {
        // Leaving `Blocked` is the *only* permanent stop (spec §6): the user
        // actually answered, the agent died, or the pane is gone. Retaining
        // against the observed set covers all three without a separate
        // cancellation path that could be forgotten at one call site.
        self.tracked
            .retain(|id, _| blocked.iter().any(|task| task.id == *id));

        for task in blocked {
            let phase = self
                .tracked
                .entry(task.id)
                .or_insert_with(|| Self::initial_phase(task, now));
            *phase = Self::advance(*phase, task, now);
        }

        let announced = self.announcement_set(blocked, now);
        let summary = (announced.len() > 1).then(|| Self::summarize(&announced));
        for task in &announced {
            // Everything named in this announcement has just been announced,
            // so everything named restarts its cadence — that is what makes a
            // re-notify refresh *the* banner instead of stacking a second one.
            self.tracked.insert(
                task.id,
                NagPhase::Armed {
                    due: now + task.repeat_interval(),
                },
            );
        }

        NagOutcome {
            announced: announced.iter().map(|task| task.id).collect(),
            summary,
            next_poll: self.next_poll_delay(now),
        }
    }

    /// The phase a task starts in the first time the engine sees it blocked.
    fn initial_phase(task: &BlockedTask, now: Instant) -> NagPhase {
        if task.in_view {
            NagPhase::Acknowledged { rearm_at: None }
        } else if task.ranked {
            NagPhase::Armed { due: now }
        } else {
            NagPhase::Debouncing {
                due: now + UNRANKED_DEBOUNCE,
            }
        }
    }

    /// Moves one task's phase forward to `now`.
    fn advance(phase: NagPhase, task: &BlockedTask, now: Instant) -> NagPhase {
        if task.in_view {
            // Looking at it acknowledges it from any phase, including a
            // debounce that had not fired yet — there is nothing left to warn
            // about once the user is on the tab.
            return NagPhase::Acknowledged { rearm_at: None };
        }
        match phase {
            // The task was in view at the previous poll and is not now, so the
            // grace starts here rather than at some earlier unobserved moment.
            NagPhase::Acknowledged { rearm_at: None } => NagPhase::Acknowledged {
                rearm_at: Some(now + ACKNOWLEDGE_GRACE),
            },
            NagPhase::Acknowledged {
                rearm_at: Some(rearm_at),
            } => {
                if now >= rearm_at {
                    // The grace expiring *is* the resumption signal, so the
                    // re-armed task is due immediately rather than after a
                    // further cadence — the user has now been away from a
                    // still-blocked agent for the whole grace.
                    NagPhase::Armed { due: now }
                } else {
                    NagPhase::Acknowledged {
                        rearm_at: Some(rearm_at),
                    }
                }
            }
            NagPhase::Debouncing { due } => {
                if now >= due {
                    NagPhase::Armed { due: now }
                } else {
                    NagPhase::Debouncing { due }
                }
            }
            NagPhase::Armed { due } => NagPhase::Armed { due },
        }
    }

    /// The tasks this poll speaks for, or empty when none has come due.
    fn announcement_set<'a>(
        &self,
        blocked: &'a [BlockedTask],
        now: Instant,
    ) -> Vec<&'a BlockedTask> {
        let armed: Vec<&BlockedTask> = blocked
            .iter()
            .filter(|task| matches!(self.tracked.get(&task.id), Some(NagPhase::Armed { .. })))
            .collect();
        let any_due = armed.iter().any(|task| {
            matches!(
                self.tracked.get(&task.id),
                Some(NagPhase::Armed { due }) if *due <= now,
            )
        });
        if any_due { armed } else { Vec::new() }
    }

    /// The one banner that speaks for several waiting agents.
    ///
    /// Counts *agents* but names *projects*, deduplicated: two blocked agents
    /// in one repo is one place to go look, and repeating the project name
    /// would waste the banner's only line.
    fn summarize(tasks: &[&BlockedTask]) -> NagSummary {
        let mut projects: Vec<&str> = Vec::new();
        for task in tasks {
            let project = task.project.as_str();
            if !projects.contains(&project) {
                projects.push(project);
            }
        }
        let named = projects
            .iter()
            .take(MAX_NAMED_PROJECTS)
            .copied()
            .collect::<Vec<_>>()
            .join(", ");
        let unnamed = projects.len().saturating_sub(MAX_NAMED_PROJECTS);
        let body = if unnamed > 0 {
            format!("{named} +{unnamed}")
        } else {
            named
        };
        NagSummary {
            title: format!("{} agents waiting", tasks.len()),
            body,
        }
    }

    /// How long until the next poll, or `None` when nothing is tracked.
    ///
    /// Capped at [`FOCUS_POLL_INTERVAL`] whenever anything is tracked, because
    /// "the user is looking at this task" is read at poll time and not
    /// delivered as an event: a task armed three minutes out still has to be
    /// silenced promptly if the user walks over to it.
    fn next_poll_delay(&self, now: Instant) -> Option<Duration> {
        if self.tracked.is_empty() {
            return None;
        }
        let soonest = self
            .tracked
            .values()
            .filter_map(|phase| phase.deadline())
            .min()
            .map_or(FOCUS_POLL_INTERVAL, |deadline| {
                deadline
                    .checked_duration_since(now)
                    .unwrap_or_default()
                    .min(FOCUS_POLL_INTERVAL)
            });
        Some(soonest)
    }
}

#[cfg(test)]
#[path = "nag_engine_tests.rs"]
mod tests;

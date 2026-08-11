//! The project rail's triage layer: which color a task row wears, what a
//! project header inherits from its tasks, and where the header chips jump.
//!
//! Everything here is a pure function of plain data so the rules can be
//! unit-tested without a renderer — the same split [`rail_project_rows`]
//! (super::project_priorities) uses for the rail's banding.
//!
//! The design is spec §5 (`specs/samithaj/rail-triage/plan.md`): rank decides
//! *where* a project sits and never changes on agent events, so color is the
//! only thing left to signal urgency, and it must be readable at a glance
//! without opening anything.

use std::time::Duration;

use super::project_priorities::RailProjectRow;
use crate::ai::agent::conversation::ConversationStatus;

/// How long an agent may wait on the user before its row escalates from
/// orange to red.
///
/// Five minutes, decided with the spec (§5). One gradient rather than two
/// independent states: the point is that the *oldest* fire reads at a glance.
/// Long enough that stepping away to read a diff does not paint the rail red,
/// short enough that a forgotten permission prompt is unmistakable.
pub const BLOCKED_ESCALATION_THRESHOLD: Duration = Duration::from_secs(5 * 60);

/// Below this the wait age is not worth rendering: the tint already says
/// "just now", and the rail's refresh is only 30s coarse (see
/// `Workspace::sync_rail_wait_age_refresh`), so a seconds counter would be
/// visibly stale about as often as it was right.
const MIN_RENDERED_WAIT_AGE: Duration = Duration::from_secs(60);

/// A rail row's triage state, least urgent first.
///
/// The derived `Ord` **is** the precedence rule "red > orange > green >
/// running-crescent": a project header inherits `max()` over its tasks, so a
/// future state is added by placing it in this list, never by editing a
/// comparison somewhere else.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RailUrgency {
    /// Working; needs nothing from the user.
    Running,
    /// Finished, and the user has not looked at the result yet.
    Unseen,
    /// Waiting on the user (a permission prompt or a question).
    Waiting,
    /// Still waiting past [`BLOCKED_ESCALATION_THRESHOLD`].
    Overdue,
}

/// Everything one task row contributes to triage.
///
/// `status` is the row's aggregate agent status (the same value the tab and
/// the header badge show, so a row and its project can never disagree); the
/// other two come from the CLI agent session behind the row, and stay at
/// their defaults for rows with no such session.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TaskTriage {
    pub status: Option<ConversationStatus>,
    /// How long the agent has been waiting on the user. `None` for a blocked
    /// row whose session predates the stamp, which degrades to plain orange
    /// rather than to a fake age.
    pub blocked_for: Option<Duration>,
    /// Whether the row's agent has finished with a result the user has not
    /// seen. Only a CLI agent session carries the acknowledgement bit, so a
    /// conversation-backed `Success` is never green — we have no honest way
    /// to know whether it was read.
    pub has_unseen_success: bool,
    /// The user's manual "mark as unread". Wins over everything, including a
    /// missing status: it is how dormant rows (no live session, no status)
    /// can be pinned green.
    pub marked_unread: bool,
}

impl TaskTriage {
    /// The row's color state, or `None` when it should render neutral.
    pub fn urgency(&self) -> Option<RailUrgency> {
        if self.marked_unread {
            return Some(RailUrgency::Unseen);
        }
        match self.status.as_ref()? {
            ConversationStatus::Blocked { .. } => Some(match self.blocked_for {
                Some(waited) if waited >= BLOCKED_ESCALATION_THRESHOLD => RailUrgency::Overdue,
                Some(_) | None => RailUrgency::Waiting,
            }),
            // A result nobody has looked at is the whole point of the green
            // state; once acknowledged the row goes back to neutral rather
            // than staying lit forever.
            ConversationStatus::Success => self.has_unseen_success.then_some(RailUrgency::Unseen),
            ConversationStatus::InProgress => Some(RailUrgency::Running),
            // The agent yielded and is listening for something from outside.
            // It is parked, not working, and it is not asking the user
            // anything either — but it shares the "needs attention before it
            // moves" band, which is where the pre-existing project-header
            // aggregate already put it.
            ConversationStatus::WaitingForEvents => Some(RailUrgency::Waiting),
            // Errors are not triage: nothing about them is time-sensitive, and
            // painting the rail red for a turn that failed an hour ago would
            // drown out the agent that is actually waiting right now.
            ConversationStatus::Error
            | ConversationStatus::TransientError
            | ConversationStatus::Cancelled => None,
        }
    }

    /// The wait-age suffix for this row ("3m"), or `None` when there is no
    /// wait worth naming.
    pub fn wait_age(&self) -> Option<String> {
        match self.urgency()? {
            RailUrgency::Waiting | RailUrgency::Overdue => format_wait_age(self.blocked_for?),
            RailUrgency::Running | RailUrgency::Unseen => None,
        }
    }
}

/// Formats a wait as the rail renders it: whole minutes ("3m") up to an hour,
/// then whole hours ("2h"). `None` below [`MIN_RENDERED_WAIT_AGE`].
pub fn format_wait_age(waited: Duration) -> Option<String> {
    if waited < MIN_RENDERED_WAIT_AGE {
        return None;
    }
    let minutes = waited.as_secs() / 60;
    match minutes / 60 {
        0 => Some(format!("{minutes}m")),
        hours => Some(format!("{hours}h")),
    }
}

/// A project header's inherited triage.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProjectTriage {
    /// The status the header badge renders. Taken from the *winning* child so
    /// the badge and the tint are the same task's, never two different ones'.
    pub status: Option<ConversationStatus>,
    /// The most urgent child's state, or `None` when no child is triageable.
    pub urgency: Option<RailUrgency>,
    /// The longest wait among the project's waiting children — the age shown
    /// next to the header badge. The worst one, not the winner's: with two
    /// blocked tasks the header must report the older fire.
    pub worst_blocked_for: Option<Duration>,
}

impl ProjectTriage {
    /// The wait-age suffix for the header ("7m"), or `None`.
    pub fn wait_age(&self) -> Option<String> {
        match self.urgency? {
            RailUrgency::Waiting | RailUrgency::Overdue => format_wait_age(self.worst_blocked_for?),
            RailUrgency::Running | RailUrgency::Unseen => None,
        }
    }
}

/// Aggregates a project's task rows into the one triage its header shows.
///
/// A task that needs the user wins over one that is merely working — with many
/// projects open, "which project is waiting on me?" is the question the rail
/// exists to answer. Ties go to the first task in rail order, so the header
/// stays put while equally urgent siblings come and go.
pub fn project_triage<'a>(tasks: impl IntoIterator<Item = &'a TaskTriage>) -> ProjectTriage {
    let mut aggregate = ProjectTriage::default();
    for task in tasks {
        let Some(urgency) = task.urgency() else {
            continue;
        };
        if aggregate.urgency.is_none_or(|best| urgency > best) {
            aggregate.urgency = Some(urgency);
            aggregate.status = task.status.clone();
        }
        match urgency {
            RailUrgency::Waiting | RailUrgency::Overdue => {
                aggregate.worst_blocked_for = aggregate.worst_blocked_for.max(task.blocked_for);
            }
            RailUrgency::Running | RailUrgency::Unseen => {}
        }
    }
    aggregate
}

/// Which header chip a jump belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RailChip {
    /// "⏳ N" — agents waiting on the user, anywhere in the rail.
    Blocked,
    /// "✅ M" — agents finished with results nobody has looked at.
    Unseen,
}

impl RailChip {
    /// Whether a row in this state is one of the chip's jump targets.
    pub fn matches(self, urgency: RailUrgency) -> bool {
        match (self, urgency) {
            // Orange and red are one gradient over the same condition, so the
            // waiting chip counts both — a task must not fall out of the count
            // by having waited *longer*.
            (RailChip::Blocked, RailUrgency::Waiting | RailUrgency::Overdue) => true,
            (RailChip::Blocked, RailUrgency::Running | RailUrgency::Unseen) => false,
            (RailChip::Unseen, RailUrgency::Unseen) => true,
            (
                RailChip::Unseen,
                RailUrgency::Running | RailUrgency::Waiting | RailUrgency::Overdue,
            ) => false,
        }
    }
}

/// One task row in rail order: what a chip would activate, and how urgent it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RailTask {
    /// Index into the workspace's tabs — the row's identity for jumping.
    pub tab_index: usize,
    pub urgency: Option<RailUrgency>,
}

/// Flattens the rail's projects into one task list in *render* order: the
/// ranked band in rank order, then the unranked band in first-seen order.
///
/// `tasks_by_project` is indexed by the same project index the rows carry.
/// Chip cycling walks this list, so "the next blocked task" means the next one
/// going down the rail, and a rank-1 fire is always reached before a rank-8
/// one — which is the whole reason rank exists.
pub fn rail_task_order(
    rows: &[RailProjectRow],
    tasks_by_project: &[Vec<RailTask>],
) -> Vec<RailTask> {
    let mut order = Vec::new();
    for row in rows {
        match row {
            RailProjectRow::UnrankedDivider => {}
            // A project index with no entry cannot happen for rows built from
            // the same slice, but skipping beats panicking on a render path.
            RailProjectRow::Project { index, rank: _ } => {
                if let Some(tasks) = tasks_by_project.get(*index) {
                    order.extend_from_slice(tasks);
                }
            }
        }
    }
    order
}

/// How many tasks each header chip would jump to. A chip whose count is zero
/// is not rendered at all.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ChipCounts {
    pub blocked: usize,
    pub unseen: usize,
}

impl ChipCounts {
    /// The count for one chip.
    pub fn get(&self, chip: RailChip) -> usize {
        match chip {
            RailChip::Blocked => self.blocked,
            RailChip::Unseen => self.unseen,
        }
    }
}

/// Counts both chips in one pass over the rail's tasks.
pub fn chip_counts(tasks: &[RailTask]) -> ChipCounts {
    let mut counts = ChipCounts::default();
    for urgency in tasks.iter().filter_map(|task| task.urgency) {
        if RailChip::Blocked.matches(urgency) {
            counts.blocked += 1;
        }
        if RailChip::Unseen.matches(urgency) {
            counts.unseen += 1;
        }
    }
    counts
}

/// The tab a chip click should activate: the next matching task after
/// `active_tab` in rail order, wrapping at the end.
///
/// Cycling from the active tab (rather than from a stored cursor) is what
/// makes repeated clicks walk the whole set: each jump makes its target
/// active, so the next click resumes from there. An `active_tab` that is not
/// itself a rail task simply starts the walk at the top.
pub fn next_chip_target(
    tasks: &[RailTask],
    chip: RailChip,
    active_tab: Option<usize>,
) -> Option<usize> {
    let start = active_tab
        .and_then(|active| tasks.iter().position(|task| task.tab_index == active))
        .map_or(0, |position| position + 1);
    tasks
        .iter()
        .cycle()
        .skip(start)
        .take(tasks.len())
        .find(|task| task.urgency.is_some_and(|urgency| chip.matches(urgency)))
        .map(|task| task.tab_index)
}

/// Which controls the rail header renders beside its chips.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RailHeaderControls {
    /// The magnifier that opens the session-search popup.
    pub session_search: bool,
    /// The button that closes the shells with no agent on them.
    pub clear_shells: bool,
}

/// Decides the header's two controls from the only two inputs they depend on.
///
/// They deliberately disagree about `show_tasks`, and the asymmetry is the
/// point: clearing shells destroys exactly the task rows listed under each
/// project, so with those rows hidden the button would ask the user to approve
/// closing tabs they cannot see, and it disappears with them — while session
/// search reaches sessions that have **no row at all** (the rail caps its
/// dormant rows, and a project with no open tab may not be listed), so hiding
/// rows makes it more useful, not less.
pub fn rail_header_controls(session_search_enabled: bool, show_tasks: bool) -> RailHeaderControls {
    RailHeaderControls {
        session_search: session_search_enabled,
        clear_shells: show_tasks,
    }
}

#[cfg(test)]
#[path = "rail_triage_tests.rs"]
mod tests;

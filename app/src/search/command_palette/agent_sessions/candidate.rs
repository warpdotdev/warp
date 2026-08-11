use chrono::{DateTime, Utc};

use crate::terminal::CLIAgent;

/// Where a candidate came from, and therefore how much is known about it.
///
/// Ordering is the resolution order when the same session is reachable from
/// more than one source: an open tab knows the most (it is on screen right
/// now), a stored handle knows its pane and cwd, and the on-disk scan knows
/// only that a transcript exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CandidateOrigin {
    /// Found by scanning the agent's own transcripts. Warp never saw it run.
    Scanned,
    /// A durable session handle: Warp ran this session at some point.
    Handle,
    /// Currently running in an open tab.
    Live,
}

/// One CLI-agent session the popup can offer to resume.
///
/// Flat and owned on purpose: the candidate list is assembled once when the
/// popup opens and then queried on every keystroke, so nothing here may need a
/// workspace, a view handle or the disk to render.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentSessionCandidate {
    pub agent: CLIAgent,
    pub session_id: String,
    /// The rail's name for the project this session's directory belongs to.
    pub project_name: String,
    /// The session's own name — never a path. Matching a path is what `cwd` is
    /// for; a task named after its directory is the problem the rail exists to
    /// fix.
    pub task_name: String,
    pub cwd: String,
    pub origin: CandidateOrigin,
    /// When this session was last active, if its source knows: `last_seen_at`
    /// for a handle, the transcript mtime for a scanned session. `None` for a
    /// live session, which has no such clock and does not need one — it sorts
    /// above everything by origin.
    pub last_active: Option<DateTime<Utc>>,
}

impl AgentSessionCandidate {
    /// The key a session is deduped on. Two sources describing the same
    /// session must collapse to one row, or picking either would resume the
    /// same conversation twice.
    pub fn key(&self) -> (CLIAgent, &str) {
        (self.agent, self.session_id.as_str())
    }
}

/// Merges the three sources into one candidate list, newest-and-most-known
/// first.
///
/// Deduped by `(agent, session_id)` with `Live > Handle > Scanned`: the more
/// authoritative source wins the row wholesale, because a live session's tab
/// and a scanned transcript disagree about almost everything except identity.
///
/// Deliberately **not** derived from `ProjectLayout::compute_with_handles`'s
/// dormant list, which drops any handle whose session is currently live —
/// exactly the sessions this popup most wants to offer, since activating the
/// tab a session is already running in is half the feature.
///
/// The result is sorted by authority and then, within each source, newest
/// first — so callers can turn position into a ranking without re-sorting.
/// The two clocks are deliberately not interleaved: a handle's timestamp says
/// when Warp last saw the session, a scanned one says when the agent last
/// wrote its transcript, and mixing them would order rows by neither. The rail
/// keeps the same two bands for the same reason.
pub fn merge(
    live: Vec<AgentSessionCandidate>,
    handles: Vec<AgentSessionCandidate>,
    scanned: Vec<AgentSessionCandidate>,
) -> Vec<AgentSessionCandidate> {
    let mut merged: Vec<AgentSessionCandidate> = Vec::new();
    // Highest-authority source first, so the first candidate seen for a key is
    // the one that wins and later duplicates are simply dropped.
    for candidate in live.into_iter().chain(handles).chain(scanned) {
        if merged
            .iter()
            .any(|existing| existing.key() == candidate.key())
        {
            continue;
        }
        merged.push(candidate);
    }
    // Authority first, recency within it. A missing timestamp sorts last,
    // which only ever affects rows that already share an origin — a live
    // session has none and does not need one.
    merged.sort_by(|left, right| {
        right
            .origin
            .cmp(&left.origin)
            .then_with(|| right.last_active.cmp(&left.last_active))
    });
    merged
}

#[cfg(test)]
#[path = "candidate_tests.rs"]
mod tests;

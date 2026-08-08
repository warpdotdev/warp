//! In-memory read model over the durable `agent_session_handles` table.
//!
//! The rail (and anything else on the UI thread) reads this mirror; sqlite is
//! only touched at startup hydration and on the writer thread. The mirror
//! tracks **identified** handles only — an in-flight row (no session id yet)
//! always belongs to a live pane, which the rail already shows as a live task.
//!
//! Kept consistent by applying every [`AgentSessionHandleOp`] that
//! [`CLIAgentSessionsModel`](super::CLIAgentSessionsModel) enqueues for the
//! writer to this mirror as well. The table stays a rebuildable index; this is
//! a cache of a cache and must never be the source of truth.

use chrono::{NaiveDateTime, Utc};
use persistence::model::AgentSessionHandleRecord;
use warpui::{Entity, ModelContext, SingletonEntity};

use crate::persistence::AgentSessionHandleOp;
use crate::terminal::CLIAgent;

/// One resumable (identified) CLI-agent session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentSessionHandle {
    pub agent: CLIAgent,
    pub session_id: String,
    /// Canonical working directory; the project bucket derives from this.
    pub cwd: String,
    /// `terminal_panes.uuid` of the pane that last ran this session. Stable
    /// across restarts, so it is how a restored tab is matched back to the
    /// session it ran — both to name the tab and to suppress a duplicate
    /// dormant row for it.
    pub pane_uuid: Vec<u8>,
    /// Cached display label (naming-cascade tier 0). `None` until the
    /// transcript resolver has produced one.
    pub title: Option<String>,
    pub last_seen_at: NaiveDateTime,
}

pub struct AgentSessionHandlesChanged;

/// Singleton mirror of the identified rows in `agent_session_handles`,
/// most-recently-seen first.
#[derive(Default)]
pub struct AgentSessionHandlesModel {
    handles: Vec<AgentSessionHandle>,
}

impl Entity for AgentSessionHandlesModel {
    type Event = AgentSessionHandlesChanged;
}

impl SingletonEntity for AgentSessionHandlesModel {}

impl AgentSessionHandlesModel {
    /// Builds the mirror from the rows loaded at startup. Un-identified rows
    /// and rows whose agent no longer deserializes are dropped; ordering
    /// follows the query (`last_seen_at DESC`).
    pub fn from_records(records: &[AgentSessionHandleRecord]) -> Self {
        let handles = records
            .iter()
            .filter_map(|record| {
                let session_id = record.session_id.clone()?;
                let agent = CLIAgent::from_serialized_name(&record.agent);
                if matches!(agent, CLIAgent::Unknown) {
                    return None;
                }
                Some(AgentSessionHandle {
                    agent,
                    session_id,
                    cwd: record.cwd.clone(),
                    pane_uuid: record.pane_uuid.clone(),
                    title: record.title.clone(),
                    last_seen_at: record.last_seen_at,
                })
            })
            .collect();
        Self { handles }
    }

    /// Identified handles, most recently seen first.
    pub fn handles(&self) -> &[AgentSessionHandle] {
        &self.handles
    }

    pub fn get(&self, agent: CLIAgent, session_id: &str) -> Option<&AgentSessionHandle> {
        self.handles
            .iter()
            .find(|handle| handle.agent == agent && handle.session_id == session_id)
    }

    /// The most recently seen handle that a tab still *hosts*: its pane
    /// satisfies `pane_matches` **and** the pane is still in the session's own
    /// directory (`pane_cwd`).
    ///
    /// Both conditions are required. The pane uuid alone is not enough — a
    /// shell can `cd` away, and a restored pane reopens at its own startup
    /// directory, so the pane that once ran a session may now be somewhere
    /// else entirely. Resuming there fails ("No conversation found with
    /// session ID"), because an agent's resume lookup is scoped to the working
    /// directory. A drifted pane is just a shell; the session stays dormant.
    pub fn find_by_pane_and_cwd(
        &self,
        pane_cwd: &str,
        mut pane_matches: impl FnMut(&[u8]) -> bool,
    ) -> Option<&AgentSessionHandle> {
        self.handles
            .iter()
            .find(|handle| handle.cwd == pane_cwd && pane_matches(&handle.pane_uuid))
    }

    /// Applies the same op that was enqueued for the sqlite writer, so the
    /// mirror and the table cannot drift within a run.
    pub fn apply(&mut self, op: &AgentSessionHandleOp, ctx: &mut ModelContext<Self>) {
        if self.apply_op(op) {
            ctx.emit(AgentSessionHandlesChanged);
        }
    }

    /// Pure core of [`Self::apply`]; returns whether anything changed.
    fn apply_op(&mut self, op: &AgentSessionHandleOp) -> bool {
        match op {
            // In-flight rows are not mirrored (see module docs).
            AgentSessionHandleOp::StartInflight { .. } => return false,
            AgentSessionHandleOp::Identify {
                agent,
                cwd,
                session_id,
                pane_uuid,
            } => {
                let agent = CLIAgent::from_serialized_name(agent);
                self.handles
                    .retain(|handle| !(handle.agent == agent && handle.session_id == *session_id));
                self.handles.insert(
                    0,
                    AgentSessionHandle {
                        agent,
                        session_id: session_id.clone(),
                        cwd: cwd.clone(),
                        pane_uuid: pane_uuid.clone(),
                        title: None,
                        last_seen_at: Utc::now().naive_utc(),
                    },
                );
            }
            AgentSessionHandleOp::Touch { agent, session_id } => {
                let agent = CLIAgent::from_serialized_name(agent);
                if let Some(position) = self
                    .handles
                    .iter()
                    .position(|handle| handle.agent == agent && handle.session_id == *session_id)
                {
                    let mut handle = self.handles.remove(position);
                    handle.last_seen_at = Utc::now().naive_utc();
                    self.handles.insert(0, handle);
                }
            }
            AgentSessionHandleOp::SetTitle {
                agent,
                session_id,
                title,
            } => {
                let agent = CLIAgent::from_serialized_name(agent);
                if let Some(handle) = self
                    .handles
                    .iter_mut()
                    .find(|handle| handle.agent == agent && handle.session_id == *session_id)
                {
                    handle.title = Some(title.clone());
                }
            }
            AgentSessionHandleOp::Forget { agent, session_id } => {
                let agent = CLIAgent::from_serialized_name(agent);
                self.handles
                    .retain(|handle| !(handle.agent == agent && handle.session_id == *session_id));
            }
        }
        true
    }
}

#[cfg(test)]
#[path = "handle_store_tests.rs"]
mod tests;

pub mod event;
pub mod handle_store;
pub mod listener;
#[cfg(not(target_family = "wasm"))]
pub(crate) mod plugin_manager;
/// Discovery of sessions Warp never witnessed, from Claude's own on-disk
/// state. Unconditional (unlike `transcript_naming`) so the rail's projection
/// needs no `cfg`; the filesystem work inside it is what is gated.
pub mod session_scan;
/// Literal substring search inside the transcripts `session_scan` discovers.
/// Unconditional for the same reason as `session_scan`: the palette's data
/// source and the model's state exist everywhere, only the reading is gated.
pub mod transcript_digest;
#[cfg(not(target_family = "wasm"))]
pub(crate) mod transcript_naming;

use std::collections::{HashMap, HashSet};

use event::{CLIAgentEvent, CLIAgentEventSource, CLIAgentEventType};
use warpui::{Entity, EntityId, ModelContext, ModelHandle, SingletonEntity};

use self::listener::CLIAgentSessionListener;
use super::CLIAgent;
use crate::GlobalResourceHandlesProvider;
use crate::ai::blocklist::InputConfig;
use crate::features::FeatureFlag;
use crate::persistence::{AgentSessionHandleOp, ModelEvent};
use crate::terminal::cli_agent_resume::is_valid_session_id;

/// Status of a tracked CLI agent session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CLIAgentSessionStatus {
    InProgress,
    Success,
    Failed {
        error_type: Option<String>,
        message: Option<String>,
    },
    Blocked {
        message: Option<String>,
    },
}

impl CLIAgentSessionStatus {
    pub fn to_conversation_status(&self) -> crate::ai::agent::conversation::ConversationStatus {
        use crate::ai::agent::conversation::ConversationStatus;
        match self {
            CLIAgentSessionStatus::InProgress => ConversationStatus::InProgress,
            CLIAgentSessionStatus::Success => ConversationStatus::Success,
            CLIAgentSessionStatus::Failed { .. } => ConversationStatus::Error,
            CLIAgentSessionStatus::Blocked { message } => ConversationStatus::Blocked {
                blocked_action: message.clone().unwrap_or_default(),
            },
        }
    }
}

/// Rich context accumulated from CLI agent session events.
#[derive(Debug, Clone, Default)]
pub struct CLIAgentSessionContext {
    pub cwd: Option<String>,
    pub project: Option<String>,
    pub session_id: Option<String>,
    pub tool_name: Option<String>,
    pub tool_input_preview: Option<String>,
    pub summary: Option<String>,
    pub query: Option<String>,
    pub response: Option<String>,
}

/// State of the rich input editor for composing a prompt to send to a CLI agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CLIAgentInputState {
    /// The rich input editor is not open.
    Closed,
    /// The rich input editor is open.
    Open {
        /// How this session was opened (for telemetry).
        entrypoint: CLIAgentInputEntrypoint,
        /// The input config that was active before opening rich input.
        previous_input_config: InputConfig,
        /// Whether the previous lock state was established while the input buffer was empty.
        previous_was_lock_set_with_empty_buffer: bool,
    },
}

/// Why the CLI agent rich input was closed (for telemetry).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum CLIAgentRichInputCloseReason {
    /// User explicitly closed (Escape, Ctrl-G, footer button).
    Manual,
    /// Auto-closed due to agent status change (e.g. Blocked).
    AutoToggle,
    /// Auto-dismissed after submitting a prompt.
    Submit,
    /// Closed for another reason (chip removed, session ended, shared session sync).
    Other,
}

/// How a [`CLIAgentInputState`] was opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum CLIAgentInputEntrypoint {
    /// User pressed Ctrl-G while a CLI agent was active.
    CtrlG,
    /// User clicked the rich input button in the CLI agent footer.
    FooterButton,
    /// Automatically opened when the CLI agent resumed work (left a blocked state)
    /// and the auto-show setting is enabled.
    AutoShow,
    /// Rich input was opened to mirror a shared-session participant's state.
    SharedSessionSync,
}

impl CLIAgentSessionContext {
    pub(crate) fn display_title(&self) -> Option<String> {
        self.latest_user_prompt().or_else(|| self.title_like_text())
    }

    pub(crate) fn latest_user_prompt(&self) -> Option<String> {
        self.query
            .as_deref()
            .map(str::trim)
            .filter(|query| !query.is_empty())
            .map(str::to_owned)
    }

    /// Returns summary text suitable as a fallback title when no user prompt is available.
    pub(crate) fn title_like_text(&self) -> Option<String> {
        self.summary
            .as_deref()
            .map(str::trim)
            .filter(|summary| !summary.is_empty())
            .map(str::to_owned)
    }
}

/// A tracked CLI agent session.
#[derive(Debug, Clone)]
pub struct CLIAgentSession {
    pub agent: CLIAgent,
    pub status: CLIAgentSessionStatus,
    pub session_context: CLIAgentSessionContext,
    /// Rich input editor state.
    pub input_state: CLIAgentInputState,
    /// Whether status-driven auto-toggle is enabled for this session.
    pub should_auto_toggle_input: bool,
    /// Event listener for plugin-backed sessions or Codex OSC9 fallback.
    /// `None` for non-Codex sessions created by command detection alone.
    /// Dropping this handle cleans up the listener's PTY event subscription.
    pub listener: Option<ModelHandle<CLIAgentSessionListener>>,
    /// The plugin version reported by structured plugin events.
    /// `None` if the plugin predates version reporting or Codex is using OSC9 fallback.
    pub plugin_version: Option<String>,
    /// `None` when the session is local.
    /// `Some("user@hostname")` when running over SSH (warpified or legacy).
    /// Used as a key for per-host plugin install failure tracking.
    pub remote_host: Option<String>,
    /// Draft text saved from the rich input composer when it was closed.
    /// Restored into the editor when the composer is reopened.
    pub draft_text: Option<String>,
    /// When the session was detected via a custom toolbar command pattern,
    /// the first word of the command (the binary/alias the user typed).
    /// Used to customize plugin instructions and force manual install mode.
    pub custom_command_prefix: Option<String>,
    /// Set once the session has received any structured OSC 777 (rich)
    /// notification. Codex's OSC 9 fallback never sets it, so this is the
    /// single source of truth for whether the session is plugin-backed.
    pub received_rich_notification: bool,
    /// When the session *entered* its current `Blocked` state, or `None` when
    /// it is not blocked. Deliberately a session field rather than a payload on
    /// `CLIAgentSessionStatus::Blocked`: the status derives `PartialEq`, and a
    /// timestamp inside the variant would make two otherwise identical blocked
    /// states unequal and leak a clock into `to_conversation_status`.
    ///
    /// `instant::Instant` (not `std::time::Instant`) because the app also
    /// builds for wasm; see the workspace's `disallowed-types` lint.
    pub blocked_since: Option<instant::Instant>,
    /// Whether the user has looked at this session's finished result.
    ///
    /// A session field for the same reason as [`Self::blocked_since`]: the
    /// status derives `PartialEq`, and an acknowledgement bit inside
    /// `Success` would make a seen and an unseen success unequal.
    ///
    /// Only meaningful while the status is `Success` — it is reset on every
    /// entry into `Success` so a *new* result is always unseen, and the rail
    /// reads it through [`Self::has_unseen_success`], which checks the status
    /// too. See [`CLIAgentSessionsModel::mark_success_seen`] for who sets it.
    pub success_seen: bool,
}

impl CLIAgentSession {
    pub fn is_remote(&self) -> bool {
        self.remote_host.is_some()
    }

    /// Whether the session surfaces trustworthy fine-grained status
    /// (in-progress / blocked / success). True only after receiving a rich OSC
    /// 777 notification. Codex's OSC 9 fallback emits only opaque `Stop`
    /// notifications and never sets `received_rich_notification`, so it does
    /// not qualify. Synthetic listener registration also does not qualify until
    /// an actual rich notification arrives.
    pub fn supports_rich_status(&self) -> bool {
        self.received_rich_notification
    }

    /// How long the session has been waiting on the user, or `None` when it is
    /// not blocked. Measured from when the session *first* entered `Blocked`,
    /// so a second permission prompt arriving while the first is unanswered
    /// does not restart the clock — the rail's escalation and the nag engine
    /// both want the age of the wait, not the age of the last event.
    pub fn blocked_duration(&self) -> Option<std::time::Duration> {
        self.blocked_since.map(|since| since.elapsed())
    }

    /// Whether the agent has finished and the user has not looked at the
    /// result yet — the rail's green row state.
    ///
    /// Gated on the status as well as the bit, so a session that left
    /// `Success` can never read as an unseen result no matter what the bit
    /// happens to hold.
    pub fn has_unseen_success(&self) -> bool {
        matches!(self.status, CLIAgentSessionStatus::Success) && !self.success_seen
    }

    /// Stamps or clears [`Self::blocked_since`] for a transition from the
    /// current status to `new_status`. Must be called *before* `self.status` is
    /// overwritten, since the decision depends on the state being left.
    fn track_blocked_since(&mut self, new_status: &CLIAgentSessionStatus) {
        let was_blocked = matches!(self.status, CLIAgentSessionStatus::Blocked { .. });
        match new_status {
            // Blocked → Blocked keeps the original stamp: repeated prompts are
            // the same unanswered wait from the user's point of view.
            CLIAgentSessionStatus::Blocked { .. } => {
                if !was_blocked {
                    self.blocked_since = Some(instant::Instant::now());
                }
            }
            // Any exit from Blocked ends the wait; clearing unconditionally
            // also keeps a never-blocked session's stamp at `None`.
            CLIAgentSessionStatus::InProgress
            | CLIAgentSessionStatus::Success
            | CLIAgentSessionStatus::Failed { .. } => self.blocked_since = None,
        }
    }

    /// Resets [`Self::success_seen`] for a transition to `new_status`. Like
    /// [`Self::track_blocked_since`] this must run *before* `self.status` is
    /// overwritten, since the decision depends on the state being left.
    fn track_success_seen(&mut self, new_status: &CLIAgentSessionStatus) {
        let was_success = matches!(self.status, CLIAgentSessionStatus::Success);
        match new_status {
            // A fresh finish is by definition unseen. Success → Success keeps
            // the acknowledgement: a second `Stop` without an intervening
            // prompt is the same result being re-announced, and re-arming the
            // green would make a chatty agent's row impossible to clear.
            CLIAgentSessionStatus::Success => {
                if !was_success {
                    self.success_seen = false;
                }
            }
            // Leaving `Success` ends the result's life; clearing
            // unconditionally also keeps a never-finished session at `false`,
            // which `has_unseen_success` reads as "nothing to see" because it
            // checks the status too.
            CLIAgentSessionStatus::InProgress
            | CLIAgentSessionStatus::Blocked { .. }
            | CLIAgentSessionStatus::Failed { .. } => self.success_seen = false,
        }
    }

    /// Clears state populated by `PermissionRequest`. Called whenever the
    /// session leaves the permission flow (the user replied, a blocking tool
    /// completed, a new prompt is submitted, or the session ends successfully)
    /// so the permission summary doesn't leak into later UI surfaces — most
    /// visibly the tab title, which can fall back to `summary` when `query`
    /// is unset.
    fn clear_permission_scoped_state(&mut self) {
        self.session_context.summary = None;
        self.session_context.tool_name = None;
        self.session_context.tool_input_preview = None;
    }

    /// Applies an event to this session, updating context and status.
    /// Returns the new status if it changed, or `None` if the event was irrelevant.
    fn apply_event(&mut self, event: &CLIAgentEvent) -> Option<CLIAgentSessionStatus> {
        self.session_context.cwd = event.cwd.clone().or(self.session_context.cwd.take());
        self.session_context.project = event
            .project
            .clone()
            .or(self.session_context.project.take());
        self.session_context.session_id = event
            .session_id
            .clone()
            .or(self.session_context.session_id.take());

        let new_status = match &event.event {
            CLIAgentEventType::PromptSubmit => {
                self.session_context.query = event.payload.query.clone();
                self.session_context.response = None;
                self.clear_permission_scoped_state();
                CLIAgentSessionStatus::InProgress
            }
            CLIAgentEventType::ToolComplete => {
                if !matches!(self.status, CLIAgentSessionStatus::Blocked { .. }) {
                    return None;
                }
                self.clear_permission_scoped_state();
                CLIAgentSessionStatus::InProgress
            }
            CLIAgentEventType::Stop => {
                self.session_context.query = event.payload.query.clone();
                self.session_context.response = event.payload.response.clone();
                self.clear_permission_scoped_state();
                CLIAgentSessionStatus::Success
            }
            CLIAgentEventType::StopFailure => {
                self.session_context.query = event.payload.query.clone();
                self.session_context.response = event.payload.response.clone();
                self.clear_permission_scoped_state();
                CLIAgentSessionStatus::Failed {
                    error_type: event.payload.error_type.clone(),
                    message: event.payload.response.clone(),
                }
            }
            CLIAgentEventType::PermissionRequest => {
                self.session_context.summary = event.payload.summary.clone();
                self.session_context.tool_name = event.payload.tool_name.clone();
                self.session_context.tool_input_preview = event.payload.tool_input_preview.clone();
                CLIAgentSessionStatus::Blocked {
                    message: event.payload.summary.clone(),
                }
            }
            CLIAgentEventType::QuestionAsked => CLIAgentSessionStatus::Blocked {
                message: event
                    .payload
                    .summary
                    .clone()
                    .or_else(|| Some("Waiting for your answer".to_owned())),
            },
            CLIAgentEventType::PermissionReplied => {
                if !matches!(self.status, CLIAgentSessionStatus::Blocked { .. }) {
                    return None;
                }
                self.clear_permission_scoped_state();
                CLIAgentSessionStatus::InProgress
            }
            // IdlePrompt means the agent is sitting at its prompt waiting for input.
            // This should not affect status — otherwise it would override Success after a Stop event.
            CLIAgentEventType::IdlePrompt => return None,
            CLIAgentEventType::SessionStart => {
                self.plugin_version = event.payload.plugin_version.clone();
                return None;
            }
            CLIAgentEventType::Unknown(_) => return None,
        };

        self.track_blocked_since(&new_status);
        self.track_success_seen(&new_status);
        self.status = new_status.clone();
        Some(new_status)
    }
}

/// Events emitted by `CLIAgentSessionsModel` for subscribers (e.g., `AgentNotificationsModel`).
#[allow(dead_code)] // `agent` fields on Started/InputSessionChanged/Ended are used for logging and future subscribers.
#[derive(Debug, Clone)]
pub enum CLIAgentSessionsModelEvent {
    Started {
        terminal_view_id: EntityId,
        agent: CLIAgent,
    },
    StatusChanged {
        terminal_view_id: EntityId,
        agent: CLIAgent,
        status: CLIAgentSessionStatus,
        session_context: Box<CLIAgentSessionContext>,
    },
    InputSessionChanged {
        terminal_view_id: EntityId,
        agent: CLIAgent,
        /// The input state BEFORE this change. When transitioning from
        /// `Open` → `Closed`, contains the saved input config to restore.
        previous_input_state: CLIAgentInputState,
        /// The input state AFTER this change.
        new_input_state: CLIAgentInputState,
    },
    Ended {
        terminal_view_id: EntityId,
        agent: CLIAgent,
    },
    /// The agent session has been updated. Subscribers may use this as a trigger for best-effort
    /// saving of state derived from the agent's session.
    SessionUpdated {
        terminal_view_id: EntityId,
        agent: CLIAgent,
    },
}

impl CLIAgentSessionsModelEvent {
    pub fn terminal_view_id(&self) -> EntityId {
        match self {
            CLIAgentSessionsModelEvent::Started {
                terminal_view_id, ..
            }
            | CLIAgentSessionsModelEvent::StatusChanged {
                terminal_view_id, ..
            }
            | CLIAgentSessionsModelEvent::InputSessionChanged {
                terminal_view_id, ..
            }
            | CLIAgentSessionsModelEvent::Ended {
                terminal_view_id, ..
            }
            | CLIAgentSessionsModelEvent::SessionUpdated {
                terminal_view_id, ..
            } => *terminal_view_id,
        }
    }
}

/// Singleton model that tracks pane-scoped CLI agent state and plugin-enriched session context.
pub struct CLIAgentSessionsModel {
    sessions: HashMap<EntityId, CLIAgentSession>,
    /// Tracks (agent, remote_host) pairs where an auto plugin operation (install or update) has failed.
    /// Shared across all views so failure in one tab is reflected everywhere.
    plugin_auto_failures: HashSet<(CLIAgent, Option<String>)>,
    /// `terminal_panes.uuid` per terminal view, registered by the owning
    /// [`TerminalPane`]. The durable session-handle store keys its in-flight
    /// rows on this uuid (stable across restarts), never on the `EntityId`.
    pane_uuids: HashMap<EntityId, Vec<u8>>,
}

impl Entity for CLIAgentSessionsModel {
    type Event = CLIAgentSessionsModelEvent;
}

impl SingletonEntity for CLIAgentSessionsModel {}

impl CLIAgentSessionsModel {
    pub fn new() -> Self {
        Self {
            sessions: HashMap::new(),
            plugin_auto_failures: HashSet::new(),
            pane_uuids: HashMap::new(),
        }
    }

    /// Registers the durable pane uuid for a terminal view. Called by the
    /// owning `TerminalPane`; without it, sessions in that pane are tracked
    /// live-only and never written to the handle store.
    pub fn register_pane_uuid(&mut self, terminal_view_id: EntityId, pane_uuid: Vec<u8>) {
        self.pane_uuids.insert(terminal_view_id, pane_uuid);
    }

    /// Drops the pane-uuid mapping when a pane is closed for good (not moved).
    /// Deliberately does NOT touch the handle store: closed panes are exactly
    /// what dormant rows represent.
    pub fn unregister_pane(&mut self, terminal_view_id: EntityId) {
        self.pane_uuids.remove(&terminal_view_id);
    }

    pub fn session(&self, terminal_view_id: EntityId) -> Option<&CLIAgentSession> {
        self.sessions.get(&terminal_view_id)
    }

    /// The `(agent, session_id)` of every live tracked session. The rail uses
    /// this to suppress a dormant handle whose session is currently running —
    /// the live row wins.
    pub fn live_session_ids(&self) -> HashSet<(CLIAgent, String)> {
        self.sessions
            .values()
            .filter_map(|session| {
                let session_id = session.session_context.session_id.clone()?;
                Some((session.agent, session_id))
            })
            .collect()
    }

    /// Acknowledges a finished session because the user is now looking at it,
    /// clearing the rail's green "results unseen" tint for that row.
    ///
    /// Called from `TerminalPane::focus` — the one funnel every "this terminal
    /// pane is now the focused pane" passes through (`PaneGroup::focus` →
    /// `focused_pane_content().focus()`), and the only focus signal that
    /// reaches this model at all. Pane-group *construction* also calls it (see
    /// the note on `TerminalPane::focus`), which is harmless: a pane being
    /// built has no session yet, so this is a no-op.
    ///
    /// Only a `Success` session is acknowledged. Focusing a pane whose agent is
    /// still working or blocked must not pre-acknowledge the result it has not
    /// produced yet — that would let a row finish silently, never going green.
    pub fn mark_success_seen(&mut self, terminal_view_id: EntityId, ctx: &mut ModelContext<Self>) {
        let Some(session) = self.sessions.get_mut(&terminal_view_id) else {
            return;
        };
        if !matches!(session.status, CLIAgentSessionStatus::Success) || session.success_seen {
            return;
        }
        session.success_seen = true;
        let agent = session.agent;
        // The rail renders from this model, so the row only loses its tint if
        // subscribers hear about it.
        ctx.emit(CLIAgentSessionsModelEvent::SessionUpdated {
            terminal_view_id,
            agent,
        });
        ctx.notify();
    }

    /// Whether any tracked session is currently waiting on the user.
    ///
    /// Drives the lifecycle of the rail's wait-age refresh timer: with nothing
    /// blocked there is no age on screen to keep current, so no timer runs.
    pub fn any_blocked(&self) -> bool {
        self.sessions
            .values()
            .any(|session| matches!(session.status, CLIAgentSessionStatus::Blocked { .. }))
    }

    /// Returns `true` if the rich input editor is currently open for this terminal.
    pub fn is_input_open(&self, terminal_view_id: EntityId) -> bool {
        self.sessions
            .get(&terminal_view_id)
            .is_some_and(|s| matches!(s.input_state, CLIAgentInputState::Open { .. }))
    }

    /// Registers a plugin-backed listener on the session for this terminal.
    ///
    /// If a session for the same agent already exists (e.g. created earlier by
    /// command detection), it is upgraded with the listener and plugin context.
    /// Otherwise a new session is created.
    ///
    /// The optional `cwd` / `project` / `session_id` fields supply initial
    /// context when available (e.g. from a `SessionStart` event). Passing
    /// `None` for all three is fine — happens when the plugin is installed
    /// mid-session and there is no start event to extract context from.
    #[allow(clippy::too_many_arguments)]
    pub fn register_listener(
        &mut self,
        terminal_view_id: EntityId,
        agent: CLIAgent,
        cwd: Option<String>,
        project: Option<String>,
        session_id: Option<String>,
        plugin_version: Option<String>,
        remote_host: Option<String>,
        should_auto_toggle_input: bool,
        listener: ModelHandle<CLIAgentSessionListener>,
        ctx: &mut ModelContext<Self>,
    ) {
        if let Some(session) = self
            .sessions
            .get_mut(&terminal_view_id)
            .filter(|s| s.agent == agent)
        {
            let had_valid_id_before = session
                .session_context
                .session_id
                .as_deref()
                .is_some_and(is_valid_session_id);
            // Upgrade existing session with plugin context.
            session.track_blocked_since(&CLIAgentSessionStatus::InProgress);
            session.track_success_seen(&CLIAgentSessionStatus::InProgress);
            session.status = CLIAgentSessionStatus::InProgress;
            session.listener = Some(listener);
            session.plugin_version = plugin_version;
            session.remote_host = remote_host;
            session.should_auto_toggle_input = should_auto_toggle_input;
            session.session_context.cwd = cwd.or(session.session_context.cwd.take());
            session.session_context.project = project.or(session.session_context.project.take());
            session.session_context.session_id =
                session_id.or(session.session_context.session_id.take());
            self.sync_session_to_handle_store(terminal_view_id, had_valid_id_before, ctx);
            return;
        }

        self.set_session(
            terminal_view_id,
            CLIAgentSession {
                agent,
                status: CLIAgentSessionStatus::InProgress,
                session_context: CLIAgentSessionContext {
                    cwd,
                    project,
                    session_id,
                    ..Default::default()
                },
                input_state: CLIAgentInputState::Closed,
                should_auto_toggle_input,
                listener: Some(listener),
                plugin_version,
                remote_host,
                draft_text: None,
                custom_command_prefix: None,
                received_rich_notification: false,
                blocked_since: None,
                success_seen: false,
            },
            ctx,
        );
        self.sync_session_to_handle_store(terminal_view_id, false, ctx);
    }

    pub fn remove_session(&mut self, terminal_view_id: EntityId, ctx: &mut ModelContext<Self>) {
        if let Some(session) = self.sessions.remove(&terminal_view_id) {
            // Final `last_seen_at` stamp so the now-dormant handle sorts to the
            // top of its project's dormant rows. Invariant: exiting a session
            // must never delete its handle — the handle IS the dormant row the
            // project rail resumes from.
            if let Some(op) = Self::identified_session_op(&session, |agent, session_id| {
                AgentSessionHandleOp::Touch { agent, session_id }
            }) {
                self.send_handle_op(op, ctx);
            }
            ctx.emit(CLIAgentSessionsModelEvent::Ended {
                terminal_view_id,
                agent: session.agent,
            });
        }
    }

    /// Builds `op` from a session's agent + validated session id, or `None`
    /// for remote sessions, missing ids, and ids that fail validation
    /// (untrusted input never reaches the store or a command line).
    fn identified_session_op(
        session: &CLIAgentSession,
        op: impl FnOnce(String, String) -> AgentSessionHandleOp,
    ) -> Option<AgentSessionHandleOp> {
        if session.remote_host.is_some() {
            return None;
        }
        let session_id = session.session_context.session_id.as_deref()?;
        if !is_valid_session_id(session_id) {
            return None;
        }
        Some(op(
            session.agent.to_serialized_name(),
            session_id.to_owned(),
        ))
    }

    /// Records the current lifecycle state of a local session into the durable
    /// handle store: `Identify` once a valid session id is known, otherwise an
    /// in-flight row keyed on the pane uuid. No-op when the pane uuid is
    /// unregistered, the session is remote, or the cwd is unknown (a handle
    /// without a cwd could never be resumed in the right directory).
    fn sync_session_to_handle_store(
        &self,
        terminal_view_id: EntityId,
        had_valid_id_before: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(session) = self.sessions.get(&terminal_view_id) else {
            return;
        };
        if session.remote_host.is_some() {
            return;
        }
        let Some(pane_uuid) = self.pane_uuids.get(&terminal_view_id) else {
            return;
        };
        let Some(cwd) = session.session_context.cwd.clone() else {
            return;
        };
        let agent = session.agent.to_serialized_name();

        let op = match session.session_context.session_id.as_deref() {
            Some(id) if is_valid_session_id(id) => {
                if had_valid_id_before {
                    // Already identified; activity stamps go through `Touch`
                    // at the call sites that own event cadence.
                    return;
                }
                AgentSessionHandleOp::Identify {
                    agent,
                    pane_uuid: pane_uuid.clone(),
                    cwd,
                    session_id: id.to_owned(),
                }
            }
            // Invalid ids are dropped at ingest; the row stays in-flight and
            // is never offered as resumable.
            Some(_) => return,
            None => AgentSessionHandleOp::StartInflight {
                agent,
                pane_uuid: pane_uuid.clone(),
                cwd,
            },
        };
        self.send_handle_op(op, ctx);
    }

    /// Resolves the session's display label from its transcript off-thread and
    /// caches it onto the durable handle (`SetTitle`). Never touches the disk
    /// on the calling thread; failure to resolve is a normal outcome and
    /// leaves the previous cache in place.
    ///
    /// Applies the same uniqueness rule the rail's disk scan applies, using the
    /// scan's claims map: a transcript's newest `aiTitle` is only this
    /// session's name if no other session claims it, because `/rename`
    /// broadcasts into concurrently-live sessions' transcripts. This path
    /// resolves one session at a time and so has no such view of its own.
    fn refresh_cached_title(&self, terminal_view_id: EntityId, ctx: &mut ModelContext<Self>) {
        #[cfg(not(target_family = "wasm"))]
        {
            if !FeatureFlag::ResumeProjectTasks.is_enabled() {
                return;
            }
            let Some(session) = self.sessions.get(&terminal_view_id) else {
                return;
            };
            // Transcript layout and record format are Claude Code's; other
            // agents keep their tier-0 cache until they get their own reader.
            if session.agent != CLIAgent::Claude || session.remote_host.is_some() {
                return;
            }
            let Some(session_id) = session
                .session_context
                .session_id
                .clone()
                .filter(|id| is_valid_session_id(id))
            else {
                return;
            };
            let Some(cwd) = session.session_context.cwd.clone() else {
                return;
            };
            let agent = session.agent.to_serialized_name();
            // Snapshotted on this thread — the scan model is a singleton the
            // spawned task cannot reach. Absent in test harnesses that never
            // register it, which is the same "cannot tell" case as a scan that
            // has not run yet.
            let claims = ctx
                .has_singleton_model::<session_scan::ClaudeSessionScanModel>()
                .then(|| {
                    session_scan::ClaudeSessionScanModel::as_ref(ctx)
                        .title_claims()
                        .clone()
                });

            let _ = ctx.spawn(
                async move {
                    let cwd_path = std::path::PathBuf::from(&cwd);
                    let names = transcript_naming::claude_transcript_path(&cwd_path, &session_id)
                        .map(|path| transcript_naming::read_transcript_names(&path, &cwd_path))
                        .unwrap_or_default();
                    let title = match &claims {
                        // The scan has read this session's directory, so it has
                        // also read the siblings a broadcast would have
                        // contaminated: uniqueness is answerable.
                        Some(claims) if claims.knows_session(&session_id) => {
                            names.resolve(|title| claims.is_only_claimant(title, &session_id))
                        }
                        // No scan yet, or not this directory. Degrade to the
                        // session's own first title rather than trusting the
                        // last: measured, the last `aiTitle` was the corrupt
                        // field in 13 of 13 contaminated transcripts and the
                        // first was correct in all 13.
                        Some(_) | None => names.resolve_without_uniqueness(),
                    };
                    (agent, session_id, title)
                },
                |me: &mut Self, (agent, session_id, title), ctx| {
                    if let Some(title) = title {
                        me.send_handle_op(
                            AgentSessionHandleOp::SetTitle {
                                agent,
                                session_id,
                                title,
                            },
                            ctx,
                        );
                    }
                },
            );
        }
    }

    /// Ships a handle-store op to the persistence writer and mirrors it into
    /// the in-memory read model so the rail updates without a DB round-trip.
    /// Feature-gated here so no call site can forget the flag.
    fn send_handle_op(&self, op: AgentSessionHandleOp, ctx: &mut ModelContext<Self>) {
        if !FeatureFlag::ResumeProjectTasks.is_enabled() {
            return;
        }
        handle_store::AgentSessionHandlesModel::handle(ctx).update(ctx, |mirror, ctx| {
            mirror.apply(&op, ctx);
        });
        let Some(sender) = GlobalResourceHandlesProvider::as_ref(ctx)
            .get()
            .model_event_sender
            .clone()
        else {
            return;
        };
        // `try_send` rather than `send`: the writer queue blocking must never
        // stall the UI thread for a best-effort index write.
        if let Err(err) = sender.try_send(ModelEvent::AgentSessionHandle(op)) {
            log::warn!("Failed to enqueue agent session handle op: {err}");
        }
    }

    /// Updates the session's status and context from a parsed CLI agent event.
    /// Rich plugin events latch `received_rich_notification` so rich-status
    /// surfaces stay consistent even if the first event was not SessionStart.
    pub fn update_from_event(
        &mut self,
        terminal_view_id: EntityId,
        event: &CLIAgentEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(session) = self.sessions.get_mut(&terminal_view_id) else {
            return;
        };

        if event.source == CLIAgentEventSource::RichPlugin {
            session.received_rich_notification = true;
        }

        let had_valid_id_before = session
            .session_context
            .session_id
            .as_deref()
            .is_some_and(is_valid_session_id);

        let event_type = event.event.clone();
        if let Some(new_status) = session.apply_event(event) {
            let agent = session.agent;
            ctx.emit(CLIAgentSessionsModelEvent::StatusChanged {
                terminal_view_id,
                agent,
                status: new_status,
                session_context: Box::new(session.session_context.clone()),
            });
        }

        if matches!(
            event_type,
            CLIAgentEventType::SessionStart
                | CLIAgentEventType::PromptSubmit
                | CLIAgentEventType::ToolComplete
        ) {
            ctx.emit(CLIAgentSessionsModelEvent::SessionUpdated {
                terminal_view_id,
                agent: session.agent,
            });
        }

        // Durable-handle bookkeeping. Only `SessionStart` may create an
        // in-flight row; every other event syncs solely on the id *becoming*
        // valid (late-id agents like Codex), so id-less fallback agents cannot
        // generate a write per event. Already-identified sessions get a
        // `Touch` stamp on the low-frequency turn boundaries so dormant
        // ordering tracks real activity without a write per tool call.
        let has_valid_id_now = self
            .sessions
            .get(&terminal_view_id)
            .and_then(|session| session.session_context.session_id.as_deref())
            .is_some_and(is_valid_session_id);
        let id_just_arrived = !had_valid_id_before && has_valid_id_now;

        match event_type {
            CLIAgentEventType::SessionStart => {
                self.sync_session_to_handle_store(terminal_view_id, had_valid_id_before, ctx);
            }
            CLIAgentEventType::PromptSubmit
            | CLIAgentEventType::Stop
            | CLIAgentEventType::StopFailure => {
                if id_just_arrived {
                    self.sync_session_to_handle_store(terminal_view_id, had_valid_id_before, ctx);
                } else if had_valid_id_before
                    && let Some(session) = self.sessions.get(&terminal_view_id)
                    && let Some(op) = Self::identified_session_op(session, |agent, session_id| {
                        AgentSessionHandleOp::Touch { agent, session_id }
                    })
                {
                    self.send_handle_op(op, ctx);
                }
                // A stop is a turn boundary: the transcript now holds whatever
                // `ai-title` Claude generated for this turn. Refresh the cached
                // label so the row is well-named the moment it goes dormant.
                if matches!(
                    event_type,
                    CLIAgentEventType::Stop | CLIAgentEventType::StopFailure
                ) {
                    self.refresh_cached_title(terminal_view_id, ctx);
                }
            }
            CLIAgentEventType::ToolComplete
            | CLIAgentEventType::PermissionRequest
            | CLIAgentEventType::PermissionReplied
            | CLIAgentEventType::QuestionAsked
            | CLIAgentEventType::IdlePrompt
            | CLIAgentEventType::Unknown(_) => {
                if id_just_arrived {
                    self.sync_session_to_handle_store(terminal_view_id, had_valid_id_before, ctx);
                }
            }
        }
    }

    pub fn open_input(
        &mut self,
        terminal_view_id: EntityId,
        entrypoint: CLIAgentInputEntrypoint,
        previous_input_config: InputConfig,
        previous_was_lock_set_with_empty_buffer: bool,
        should_auto_toggle_input: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(session) = self.sessions.get_mut(&terminal_view_id) else {
            return;
        };

        let previous_input_state = session.input_state;
        session.input_state = CLIAgentInputState::Open {
            entrypoint,
            previous_input_config,
            previous_was_lock_set_with_empty_buffer,
        };
        session.should_auto_toggle_input = should_auto_toggle_input;

        ctx.emit(CLIAgentSessionsModelEvent::InputSessionChanged {
            terminal_view_id,
            agent: session.agent,
            previous_input_state,
            new_input_state: session.input_state,
        });
    }

    pub fn close_input(
        &mut self,
        terminal_view_id: EntityId,
        should_auto_toggle_input: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(session) = self.sessions.get_mut(&terminal_view_id) else {
            return;
        };
        if session.input_state == CLIAgentInputState::Closed {
            return;
        }

        let previous_input_state = session.input_state;
        session.input_state = CLIAgentInputState::Closed;
        session.should_auto_toggle_input = should_auto_toggle_input;
        ctx.emit(CLIAgentSessionsModelEvent::InputSessionChanged {
            terminal_view_id,
            agent: session.agent,
            previous_input_state,
            new_input_state: CLIAgentInputState::Closed,
        });
    }

    pub fn set_session(
        &mut self,
        terminal_view_id: EntityId,
        session: CLIAgentSession,
        ctx: &mut ModelContext<Self>,
    ) {
        let agent = session.agent;
        // Close any open rich input before replacing, so subscribers can
        // restore input config before the session ends.
        self.close_input(terminal_view_id, false, ctx);
        if let Some(old) = self.sessions.insert(terminal_view_id, session) {
            ctx.emit(CLIAgentSessionsModelEvent::Ended {
                terminal_view_id,
                agent: old.agent,
            });
        }

        ctx.emit(CLIAgentSessionsModelEvent::Started {
            terminal_view_id,
            agent,
        });
    }

    /// Records that an auto plugin operation (install or update) failed for the given agent/host.
    /// `remote_host` is `None` for local sessions, `Some("user@hostname")` for remote.
    #[cfg(not(target_family = "wasm"))]
    pub fn record_plugin_auto_failure(&mut self, agent: CLIAgent, remote_host: Option<String>) {
        self.plugin_auto_failures.insert((agent, remote_host));
    }

    /// Saves draft text from the rich input composer for the given terminal.
    /// Stores `None` for empty or whitespace-only text.
    pub fn set_draft(&mut self, terminal_view_id: EntityId, text: String) {
        if let Some(session) = self.sessions.get_mut(&terminal_view_id) {
            session.draft_text = if text.trim().is_empty() {
                None
            } else {
                Some(text)
            };
        }
    }

    /// Clears any saved draft text for the given terminal.
    pub fn clear_draft(&mut self, terminal_view_id: EntityId) {
        if let Some(session) = self.sessions.get_mut(&terminal_view_id) {
            session.draft_text = None;
        }
    }

    /// Returns and clears the draft text for the given terminal, if any.
    pub fn take_draft(&mut self, terminal_view_id: EntityId) -> Option<String> {
        self.sessions
            .get_mut(&terminal_view_id)
            .and_then(|s| s.draft_text.take())
    }

    /// Whether an auto plugin operation has previously failed for this agent on this host.
    pub fn has_plugin_auto_failed(&self, agent: CLIAgent, remote_host: &Option<String>) -> bool {
        self.plugin_auto_failures
            .contains(&(agent, remote_host.clone()))
    }
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;

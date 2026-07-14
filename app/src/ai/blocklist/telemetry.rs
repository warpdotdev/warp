use serde::Serialize;
use serde_json::{json, Value};
use strum_macros::{EnumDiscriminants, EnumIter};
use warp_core::telemetry::{EnablementState, TelemetryEvent, TelemetryEventDesc};

use crate::ai::agent::conversation::AIConversationId;

#[derive(Debug, EnumDiscriminants)]
#[strum_discriminants(derive(EnumIter))]
pub(crate) enum BlocklistOrchestrationTelemetryEvent {
    TeamAgentCommunicationFailed(TeamAgentCommunicationFailedEvent),
    PlanConfigApprovalToggled(PlanConfigApprovalToggledEvent),
    RunAgentsCardDecision(RunAgentsCardDecisionEvent),
    PillBarInteraction(PillBarInteractionEvent),
    OrchestrationEntered(OrchestrationEnteredEvent),
    AgentProposedConfig(AgentProposedConfigEvent),
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
#[allow(dead_code)]
pub(crate) enum TeamAgentCommunicationKind {
    Message,
    LifecycleEvent,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
#[allow(dead_code)]
pub(crate) enum TeamAgentCommunicationTransport {
    Local,
    ServerApi,
}
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
#[allow(dead_code)]
pub(crate) enum TeamAgentOrchestrationVersion {
    V1,
    V2,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
#[allow(dead_code)]
pub(crate) enum TeamAgentCommunicationFailureReason {
    InvalidLifecycleEventType,
    MissingSourceConversation,
    MissingSourceIdentifier,
    UnknownAgent,
    NoTargets,
    RequestFailed,
}

#[derive(Debug, Serialize)]
pub(crate) struct TeamAgentCommunicationFailedEvent {
    pub communication_kind: TeamAgentCommunicationKind,
    pub transport: TeamAgentCommunicationTransport,
    pub orchestration_version: TeamAgentOrchestrationVersion,
    pub failure_reason: TeamAgentCommunicationFailureReason,
    pub source_conversation_id: AIConversationId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_run_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lifecycle_event_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

/// Coarse approval transition for the plan card's
/// `Use orchestration` toggle.
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum OrchestrationApprovalStatus {
    Approved,
    Disapproved,
}

/// Run-wide execution mode reported on telemetry payloads. A flat
/// enum so the payload stays metadata-only and never carries an
/// environment id or worker host.
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum OrchestrationExecutionModeKind {
    Local,
    Remote,
}

impl OrchestrationExecutionModeKind {
    pub(crate) fn from_run_agents(mode: &ai::agent::action::RunAgentsExecutionMode) -> Self {
        if mode.is_remote() {
            Self::Remote
        } else {
            Self::Local
        }
    }
}

/// Closed-set bucket for the run-wide harness selection. Anything
/// unrecognized collapses to `Unknown` to keep the analytics column
/// low-cardinality.
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum OrchestrationHarnessKind {
    Oz,
    ClaudeCode,
    Codex,
    OpenCode,
    Gemini,
    Unknown,
}

impl OrchestrationHarnessKind {
    pub(crate) fn from_str(harness_type: &str) -> Self {
        match harness_type {
            "oz" | "" => Self::Oz,
            "claude" | "claude-code" | "claude_code" => Self::ClaudeCode,
            "codex" => Self::Codex,
            "opencode" | "open-code" | "open_code" => Self::OpenCode,
            "gemini" => Self::Gemini,
            _ => Self::Unknown,
        }
    }
}

/// Stable names for run-wide config fields that can diverge between
/// the dispatched orchestration request and either the original tool
/// call or an active approved config. Match the server's equivalent
/// field-name constants so the two telemetry streams can be joined.
pub(crate) mod orchestration_modified_field {
    pub const MODEL_ID: &str = "model_id";
    pub const HARNESS: &str = "harness";
    pub const EXECUTION_MODE: &str = "execution_mode";
    pub const ENVIRONMENT_ID: &str = "environment_id";
    pub const WORKER_HOST: &str = "worker_host";
    pub const AUTH_SECRET: &str = "auth_secret";
}

#[derive(Debug, Serialize)]
pub(crate) struct PlanConfigApprovalToggledEvent {
    pub conversation_id: AIConversationId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan_id: Option<String>,
    /// State after the toggle. The pre-toggle state is the opposite
    /// since this event only fires on a binary flip.
    pub status: OrchestrationApprovalStatus,
    pub execution_mode: OrchestrationExecutionModeKind,
    pub harness: OrchestrationHarnessKind,
    pub has_model: bool,
    pub has_environment: bool,
    pub has_worker_host: bool,
    pub has_auth_secret: bool,
}

/// Decision a user took on the run_agents confirmation card.
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RunAgentsCardDecision {
    Accept,
    AcceptWithoutOrchestration,
    Reject,
}

#[derive(Debug, Serialize)]
pub(crate) struct RunAgentsCardDecisionEvent {
    pub conversation_id: AIConversationId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan_id: Option<String>,
    pub decision: RunAgentsCardDecision,
    pub agent_count: usize,
    pub harness: OrchestrationHarnessKind,
    pub execution_mode: OrchestrationExecutionModeKind,
    pub model_id: String,
    /// Field names from [`orchestration_modified_field`] that diverged
    /// between the dispatched request and the LLM's original
    /// `RunAgentsRequest`. Empty when the user accepted without edits.
    pub modified_fields_from_tool_call: Vec<&'static str>,
    /// Same shape, but compared against the approved orchestration
    /// config snapshot. Empty when no approved config exists or the
    /// dispatched request matches it.
    pub modified_fields_from_active_config: Vec<&'static str>,
    pub had_active_config: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_config_status: Option<OrchestrationApprovalStatus>,
}

/// Surface that first introduced orchestration into a conversation.
///
/// Plan-card surfacing is intentionally NOT a variant here — that signal
/// is covered by [`AgentProposedConfigEvent`] (fires once per plan card
/// instance when an agent-authored snapshot first becomes visible) plus
/// [`PlanConfigApprovalToggledEvent`] (the user's approval toggle).
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum OrchestrationEntrySource {
    /// `/orchestrate` slash-command mode on a user query.
    SlashCommandOrchestrate,
    /// `run_agents` confirmation card was shown (not auto-launched).
    RunAgentsCardShown,
}

#[derive(Debug, Serialize)]
pub(crate) struct OrchestrationEnteredEvent {
    pub conversation_id: AIConversationId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan_id: Option<String>,
    pub entry_source: OrchestrationEntrySource,
}

/// Fires when an agent-authored orchestration config snapshot first
/// becomes visible to the user on a plan card. One emission per
/// `OrchestrationConfigBlockView` instance.
#[derive(Debug, Serialize)]
pub(crate) struct AgentProposedConfigEvent {
    pub conversation_id: AIConversationId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan_id: Option<String>,
    pub harness: OrchestrationHarnessKind,
    pub execution_mode: OrchestrationExecutionModeKind,
    pub has_model: bool,
    pub has_environment: bool,
    pub has_worker_host: bool,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PillBarPillKind {
    Orchestrator,
    Child,
}

/// Concrete user actions against an orchestration pill bar entry.
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PillBarActionKind {
    /// User clicked the pill body. See `switch_outcome` for what
    /// happened next.
    Switch,
    OpenInNewPane,
    OpenInNewTab,
    /// User picked "Focus pane" from a pill's 3-dot menu. Distinct
    /// from a pill-body click that resolves to the same outcome
    /// (those are `Switch` with `switch_outcome = focused_existing_pane`).
    FocusOpenedConversation,
    Stop,
    Kill,
    TogglePinOn,
    TogglePinOff,
    ViewInOz,
    OpenMenu,
}

/// Outcome of a pill-body click. Closed enum so future navigation
/// outcomes can be added without splitting `Switch` into multiple
/// action variants again.
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PillSwitchOutcome {
    /// Pill click navigated within the current pane.
    SwitchedInPlace,
    /// Target conversation was already owned by another visible
    /// terminal view; focus moved there instead of switching in place.
    FocusedExistingPane,
}

#[derive(Debug, Serialize)]
pub(crate) struct PillBarInteractionEvent {
    pub action: PillBarActionKind,
    pub pill_kind: PillBarPillKind,
    pub total_pills: usize,
    pub total_pinned: usize,
    /// The orchestrator that hosts the pill bar.
    pub source_conversation_id: AIConversationId,
    /// The pill the action targets.
    pub target_conversation_id: AIConversationId,
    /// Present only when `action == Switch`. Distinguishes whether the
    /// pill-body click navigated within the current pane or moved
    /// focus to an existing pane already owning the conversation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub switch_outcome: Option<PillSwitchOutcome>,
}

impl TelemetryEvent for BlocklistOrchestrationTelemetryEvent {
    fn name(&self) -> &'static str {
        BlocklistOrchestrationTelemetryEventDiscriminants::from(self).name()
    }

    fn payload(&self) -> Option<Value> {
        match self {
            Self::TeamAgentCommunicationFailed(event) => Some(json!(event)),
            Self::PlanConfigApprovalToggled(event) => Some(json!(event)),
            Self::RunAgentsCardDecision(event) => Some(json!(event)),
            Self::PillBarInteraction(event) => Some(json!(event)),
            Self::OrchestrationEntered(event) => Some(json!(event)),
            Self::AgentProposedConfig(event) => Some(json!(event)),
        }
    }

    fn description(&self) -> &'static str {
        BlocklistOrchestrationTelemetryEventDiscriminants::from(self).description()
    }

    fn enablement_state(&self) -> EnablementState {
        BlocklistOrchestrationTelemetryEventDiscriminants::from(self).enablement_state()
    }

    fn contains_ugc(&self) -> bool {
        false
    }

    fn event_descs() -> impl Iterator<Item = Box<dyn TelemetryEventDesc>> {
        warp_core::telemetry::enum_events::<Self>()
    }
}

impl TelemetryEventDesc for BlocklistOrchestrationTelemetryEventDiscriminants {
    fn name(&self) -> &'static str {
        match self {
            Self::TeamAgentCommunicationFailed => {
                "AgentMode.Orchestration.TeamAgentCommunicationFailed"
            }
            Self::PlanConfigApprovalToggled => "AgentMode.Orchestration.PlanConfigApprovalToggled",
            Self::RunAgentsCardDecision => "AgentMode.Orchestration.RunAgentsCardDecision",
            Self::PillBarInteraction => "AgentMode.Orchestration.PillBarInteraction",
            Self::OrchestrationEntered => "AgentMode.Orchestration.Entered",
            Self::AgentProposedConfig => "AgentMode.Orchestration.AgentProposedConfig",
        }
    }

    fn description(&self) -> &'static str {
        match self {
            Self::TeamAgentCommunicationFailed => {
                "Failed to send an orchestration message or lifecycle event for a TeamAgent"
            }
            Self::PlanConfigApprovalToggled => {
                "User toggled the Use orchestration switch on a plan card"
            }
            Self::RunAgentsCardDecision => {
                "User accepted, accepted-without-orchestration, or rejected a run_agents confirmation card. Reports which config fields diverged from the original tool call and/or the active approved config."
            }
            Self::PillBarInteraction => {
                "User interacted with the orchestration pill bar (switch, pin, open in pane/tab, stop, kill, etc.)"
            }
            Self::OrchestrationEntered => {
                "Orchestration was activated in a conversation via /orchestrate or a run_agents confirmation card surfacing. Plan-card entries are tracked separately via AgentProposedConfig + PlanConfigApprovalToggled."
            }
            Self::AgentProposedConfig => {
                "An agent-authored orchestration config snapshot first became visible to the user on a plan card"
            }
        }
    }

    fn enablement_state(&self) -> EnablementState {
        EnablementState::Always
    }
}

warp_core::register_telemetry_event!(BlocklistOrchestrationTelemetryEvent);

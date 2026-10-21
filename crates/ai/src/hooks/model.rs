use std::collections::HashSet;
use std::fmt::Display;
use std::path::PathBuf;
use std::time::Duration;

use ai_types::{AIAgentActionId, AIConversationId};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::LLMId;
use crate::agent::action::{AIAgentActionType, FileEdit};
use crate::agent::action_result::AIAgentActionResultType;

/// Version of the JSON contract hooks see on stdin and write on stdout.
pub const HOOK_PROTOCOL_VERSION: u32 = 1;

/// Version of the [`HookEnvelope`] shape. Bumped when the common fields change.
pub const ENVELOPE_SCHEMA_VERSION: u32 = 1;

/// Default deadline for hooks that run before an operation and can block it.
pub const PRE_EXECUTION_TIMEOUT: Duration = Duration::from_secs(30);

/// Default deadline for hooks that run after an operation or at a lifecycle moment.
pub const POST_AND_OTHER_TIMEOUT: Duration = Duration::from_secs(60);

/// Default deadline for `session.end` hooks, which run while a surface is closing.
pub const SESSION_END_TIMEOUT: Duration = Duration::from_secs(5);

/// Longest per-definition `timeout` a source may declare. A definition above it is rejected when
/// the source is activated, not run with a shorter deadline.
pub const HOOK_TIMEOUT_CEILING: Duration = Duration::from_secs(600);

/// A moment in the agent loop at which hooks can run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum HookBoundary {
    SessionStart,
    SessionEnd,
    PromptSubmit,
    ToolPre,
    ToolPermissionRequest,
    ToolResult,
    ToolFailure,
    FileExposure,
    SubagentStart,
    SubagentStop,
    TurnStop,
    TurnInterrupt,
    CompactionPre,
    CompactionPost,
    ResponseComplete,
    ReasoningComplete,
}

impl HookBoundary {
    /// Whether Warp fires this boundary in Phase 1. The others are parsed and validated but never
    /// fired.
    pub fn fires_in_phase_1(&self) -> bool {
        match self {
            Self::SessionStart
            | Self::PromptSubmit
            | Self::ToolPre
            | Self::ToolResult
            | Self::ToolFailure
            | Self::TurnStop
            | Self::ResponseComplete
            | Self::ReasoningComplete => true,
            Self::SessionEnd
            | Self::ToolPermissionRequest
            | Self::FileExposure
            | Self::SubagentStart
            | Self::SubagentStop
            | Self::TurnInterrupt
            | Self::CompactionPre
            | Self::CompactionPost => false,
        }
    }

    /// Whether a verdict from a hook on this boundary can change what the agent does.
    pub fn honors_verdict(&self) -> bool {
        match self {
            Self::PromptSubmit
            | Self::ToolPre
            | Self::ToolPermissionRequest
            | Self::FileExposure
            | Self::SubagentStart
            | Self::TurnStop => true,
            Self::SessionStart
            | Self::SessionEnd
            | Self::ToolResult
            | Self::ToolFailure
            | Self::SubagentStop
            | Self::TurnInterrupt
            | Self::CompactionPre
            | Self::CompactionPost
            | Self::ResponseComplete
            | Self::ReasoningComplete => false,
        }
    }

    pub fn default_timeout(&self) -> Duration {
        match self {
            Self::SessionEnd => SESSION_END_TIMEOUT,
            Self::PromptSubmit
            | Self::ToolPre
            | Self::ToolPermissionRequest
            | Self::FileExposure
            | Self::SubagentStart => PRE_EXECUTION_TIMEOUT,
            Self::SessionStart
            | Self::ToolResult
            | Self::ToolFailure
            | Self::SubagentStop
            | Self::TurnStop
            | Self::TurnInterrupt
            | Self::CompactionPre
            | Self::CompactionPost
            | Self::ResponseComplete
            | Self::ReasoningComplete => POST_AND_OTHER_TIMEOUT,
        }
    }

    /// The vendor-neutral event name, for logs and diagnostics.
    pub fn canonical_name(&self) -> &'static str {
        match self {
            Self::SessionStart => "session.start",
            Self::SessionEnd => "session.end",
            Self::PromptSubmit => "prompt.submit",
            Self::ToolPre => "tool.pre",
            Self::ToolPermissionRequest => "tool.permission_request",
            Self::ToolResult => "tool.result",
            Self::ToolFailure => "tool.failure",
            Self::FileExposure => "file.exposure",
            Self::SubagentStart => "subagent.start",
            Self::SubagentStop => "subagent.stop",
            Self::TurnStop => "turn.stop",
            Self::TurnInterrupt => "turn.interrupt",
            Self::CompactionPre => "compaction.pre",
            Self::CompactionPost => "compaction.post",
            Self::ResponseComplete => "response.complete",
            Self::ReasoningComplete => "reasoning.complete",
        }
    }
}

impl Display for HookBoundary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.canonical_name())
    }
}

/// Where a hook configuration came from. The user file is not a Phase 1 source and is left out
/// until a later PR needs it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum HookSourceKind {
    ProjectCursorFile,
    RunDelivered,
    FactoryAgent,
    FactoryWide,
}

impl HookSourceKind {
    /// Merge priority. Lower values win scalar conflicts and sort first.
    pub fn priority(&self) -> u8 {
        match self {
            Self::FactoryAgent => 0,
            Self::FactoryWide => 1,
            Self::RunDelivered => 2,
            Self::ProjectCursorFile => 3,
        }
    }
}

/// How a source is identified once resolved: a canonical file path for discovered files, or the
/// name an explicit source was delivered under.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum HookSourceLocation {
    CanonicalPath(PathBuf),
    Named(String),
}

/// A resolved source: what kind it is and where it was found.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct HookSourceId {
    pub kind: HookSourceKind,
    pub location: HookSourceLocation,
}

impl Display for HookSourceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.location {
            HookSourceLocation::CanonicalPath(path) => write!(f, "{}", path.display()),
            HookSourceLocation::Named(name) => f.write_str(name),
        }
    }
}

/// One hook definition inside a source: the vendor event it is declared under and its position in
/// that event's array.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct HookBindingId {
    pub source: HookSourceId,
    pub event: String,
    pub index: usize,
}

impl Display for HookBindingId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}#{}[{}]", self.source, self.event, self.index)
    }
}

/// A supported configuration format. Each format has one adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum HookFormat {
    Cursor,
}

/// The runtime that is firing the boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum HookSurface {
    OzRun,
    CliRun,
    LocalChild,
    Gui,
    Tui,
}

/// Where the operation the hook describes is executing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum HookExecutionHost {
    Local,
}

/// Identity of the run this conversation belongs to, when it is part of one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunMetadata {
    pub run_id: String,
    pub parent_run_id: Option<String>,
    pub task_id: Option<String>,
}

/// Identifies one user turn across every exchange it causes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct HookTurnId(Uuid);

impl HookTurnId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn as_uuid(&self) -> Uuid {
        self.0
    }
}

impl Default for HookTurnId {
    fn default() -> Self {
        Self::new()
    }
}

/// What Warp's own permission check knows at the time the boundary fires.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionFacts {
    pub warp_would_ask: Option<bool>,
    pub autonomy: Option<String>,
}

/// Everything a boundary knows, in Warp's own types. Adapters turn this into a vendor's stdin
/// shape; scripts never see it directly.
#[derive(Debug, Clone)]
pub struct HookEnvelope {
    /// Version of this envelope's shape; see [`ENVELOPE_SCHEMA_VERSION`]. Adapters read it;
    /// scripts never see it.
    pub schema_version: u32,
    /// Identifies one firing of a boundary, shared by every hook that runs for it.
    pub occurrence_id: Uuid,
    /// Identifies one hook process run. The dispatcher stamps a fresh value per binding.
    pub invocation_id: Uuid,
    pub conversation_id: AIConversationId,
    pub turn_id: Option<HookTurnId>,
    pub exchange_id: Option<Uuid>,
    pub action_id: Option<AIAgentActionId>,
    pub run: Option<RunMetadata>,
    pub surface: HookSurface,
    pub execution_host: HookExecutionHost,
    pub cwd: Option<PathBuf>,
    pub roots: Vec<PathBuf>,
    pub source_revision: u64,
    pub model_id: Option<LLMId>,
    pub user_email: Option<String>,
    pub warp_version: String,
    /// Version of the stdin and stdout JSON contract scripts see; see [`HOOK_PROTOCOL_VERSION`].
    pub protocol_version: u32,
    pub permission_facts: PermissionFacts,
    pub payload: HookPayload,
}

impl HookEnvelope {
    pub fn boundary(&self) -> HookBoundary {
        self.payload.boundary()
    }
}

/// Why a turn ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TurnStopStatus {
    Completed,
    Cancelled,
    Error,
}

/// The boundary-specific part of an envelope.
#[derive(Debug, Clone)]
pub enum HookPayload {
    SessionStart {
        is_background_agent: bool,
    },
    PromptSubmit {
        text: String,
        attachments_summary: Vec<String>,
    },
    ToolPre {
        action: AIAgentActionType,
        tool_name: CanonicalToolName,
    },
    ToolResult {
        action: AIAgentActionType,
        tool_name: CanonicalToolName,
        result: AIAgentActionResultType,
        /// How long the tool ran, when the caller measured it.
        duration: Option<Duration>,
    },
    ToolFailure {
        action: AIAgentActionType,
        tool_name: CanonicalToolName,
        result: AIAgentActionResultType,
        /// Present when the failure is a cancellation rather than an error result.
        cancellation_reason: Option<String>,
        /// How long the tool ran before failing, when the caller measured it.
        duration: Option<Duration>,
    },
    TurnStop {
        status: TurnStopStatus,
        continuation_count: u32,
    },
    ResponseComplete {
        message_id: String,
        text: String,
    },
    ReasoningComplete {
        message_id: String,
        text: String,
        duration: Option<Duration>,
    },
}

impl HookPayload {
    pub fn boundary(&self) -> HookBoundary {
        match self {
            Self::SessionStart { .. } => HookBoundary::SessionStart,
            Self::PromptSubmit { .. } => HookBoundary::PromptSubmit,
            Self::ToolPre { .. } => HookBoundary::ToolPre,
            Self::ToolResult { .. } => HookBoundary::ToolResult,
            Self::ToolFailure { .. } => HookBoundary::ToolFailure,
            Self::TurnStop { .. } => HookBoundary::TurnStop,
            Self::ResponseComplete { .. } => HookBoundary::ResponseComplete,
            Self::ReasoningComplete { .. } => HookBoundary::ReasoningComplete,
        }
    }

    /// The canonical tool name for tool boundaries.
    pub fn tool_name(&self) -> Option<&CanonicalToolName> {
        match self {
            Self::ToolPre { tool_name, .. }
            | Self::ToolResult { tool_name, .. }
            | Self::ToolFailure { tool_name, .. } => Some(tool_name),
            Self::SessionStart { .. }
            | Self::PromptSubmit { .. }
            | Self::TurnStop { .. }
            | Self::ResponseComplete { .. }
            | Self::ReasoningComplete { .. } => None,
        }
    }
}

/// The tool name hooks match on and see in their input. Cursor's names are used where a Warp
/// action maps onto one of Cursor's tools; Warp-only tools get Warp-specific names.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CanonicalToolName(String);

impl CanonicalToolName {
    pub fn for_action(action: &AIAgentActionType) -> Self {
        let name = match action {
            AIAgentActionType::RequestCommandOutput { .. } => "Shell".to_owned(),
            AIAgentActionType::ReadFiles(_) => "Read".to_owned(),
            AIAgentActionType::RequestFileEdits { file_edits, .. } => {
                let all_deletes = !file_edits.is_empty()
                    && file_edits
                        .iter()
                        .all(|edit| matches!(edit, FileEdit::Delete { .. }));
                if all_deletes {
                    "Delete".to_owned()
                } else {
                    "Write".to_owned()
                }
            }
            AIAgentActionType::Grep { .. } => "Grep".to_owned(),
            AIAgentActionType::RunAgents(_) => "Task".to_owned(),
            AIAgentActionType::CallMCPTool { name, .. } => format!("MCP:{name}"),
            AIAgentActionType::FileGlob { .. } | AIAgentActionType::FileGlobV2 { .. } => {
                "Glob".to_owned()
            }
            AIAgentActionType::SearchCodebase(_) => "SearchCodebase".to_owned(),
            AIAgentActionType::ReadMCPResource { name, .. } => format!("MCPResource:{name}"),
            AIAgentActionType::WriteToLongRunningShellCommand { .. } => "ShellWrite".to_owned(),
            AIAgentActionType::ReadShellCommandOutput { .. } => "ShellRead".to_owned(),
            AIAgentActionType::TransferShellCommandControlToUser { .. } => {
                "ShellHandoff".to_owned()
            }
            AIAgentActionType::ReadSkill(_) => "ReadSkill".to_owned(),
            AIAgentActionType::UseComputer(_) => "UseComputer".to_owned(),
            AIAgentActionType::RequestComputerUse(_) => "RequestComputerUse".to_owned(),
            AIAgentActionType::StartRecording { .. } => "StartRecording".to_owned(),
            AIAgentActionType::StopRecording { .. } => "StopRecording".to_owned(),
            AIAgentActionType::ReadDocuments(_) => "ReadDocuments".to_owned(),
            AIAgentActionType::EditDocuments(_) => "EditDocuments".to_owned(),
            AIAgentActionType::CreateDocuments(_) => "CreateDocuments".to_owned(),
            AIAgentActionType::AskUserQuestion { .. } => "AskUserQuestion".to_owned(),
            AIAgentActionType::SendMessageToAgent { .. } => "SendMessageToAgent".to_owned(),
            AIAgentActionType::FetchConversation { .. } => "FetchConversation".to_owned(),
            AIAgentActionType::WaitForEvents { .. } => "WaitForEvents".to_owned(),
            AIAgentActionType::UploadArtifact(_) => "UploadArtifact".to_owned(),
            AIAgentActionType::SuggestNewConversation { .. } => "SuggestNewConversation".to_owned(),
            AIAgentActionType::SuggestPrompt(_) => "SuggestPrompt".to_owned(),
            AIAgentActionType::InitProject => "InitProject".to_owned(),
            AIAgentActionType::OpenCodeReview => "OpenCodeReview".to_owned(),
            AIAgentActionType::InsertCodeReviewComments { .. } => {
                "InsertCodeReviewComments".to_owned()
            }
        };
        Self(name)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Display for CanonicalToolName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// What a hook said about the operation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum HookVerdict {
    #[default]
    NoOpinion,
    Allow,
    Deny,
    Ask,
    /// Represented because Claude Code defines it; Warp does not support resumable deferral, so
    /// the dispatcher reports it as unsupported instead of merging it.
    Defer,
}

impl HookVerdict {
    /// Rank used when merging verdicts: the highest wins. `None` for verdicts that are not merged.
    pub fn severity(&self) -> Option<u8> {
        match self {
            Self::NoOpinion => Some(0),
            Self::Allow => Some(1),
            Self::Ask => Some(2),
            Self::Deny => Some(3),
            Self::Defer => None,
        }
    }
}

/// Text explaining a verdict. `binding` is absent when Warp itself wrote the reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookReason {
    pub text: String,
    pub binding: Option<HookBindingId>,
}

/// A request from a stop hook to continue the turn with more input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContinuationRequest {
    pub text: String,
    pub loop_limit: Option<u32>,
}

/// Everything one hook asked for, decoded from its output.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HookEffects {
    pub verdict: HookVerdict,
    pub reasons: Vec<HookReason>,
    pub replacement_input: Option<serde_json::Value>,
    pub replacement_result: Option<serde_json::Value>,
    pub model_context: Vec<String>,
    pub user_notices: Vec<String>,
    pub session_env: Vec<(String, String)>,
    pub halt_turn: bool,
    pub continuation: Option<ContinuationRequest>,
    /// Effects the hook asked for that its event cannot produce, so the adapter did not apply
    /// them. They are reported, not acted on.
    pub unsupported: Vec<UnsupportedEffect>,
}

/// One channel of [`HookEffects`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum EffectKind {
    Verdict,
    ReplacementInput,
    ReplacementResult,
    ModelContext,
    UserNotice,
    SessionEnv,
    HaltTurn,
    Continuation,
}

/// An effect a hook produced that the current phase or surface cannot honor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnsupportedEffect {
    pub binding: HookBindingId,
    pub effect: EffectKind,
    pub reason: String,
}

/// The effects of every hook that ran for one boundary firing, merged in priority order.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MergedEffects {
    pub verdict: HookVerdict,
    /// The binding whose verdict won the merge.
    pub verdict_binding: Option<HookBindingId>,
    pub reasons: Vec<HookReason>,
    pub replacement_input: Option<serde_json::Value>,
    pub replacement_result: Option<serde_json::Value>,
    pub model_context: Vec<String>,
    pub user_notices: Vec<String>,
    pub session_env: Vec<(String, String)>,
    pub halt_turn: bool,
    pub continuation: Option<ContinuationRequest>,
    pub conflicts: Vec<String>,
    pub unsupported: Vec<UnsupportedEffect>,
}

impl MergedEffects {
    pub fn no_opinion() -> Self {
        Self::default()
    }
}

/// Merges per-binding effects. `ordered` must already be sorted by source priority and then by
/// array index; this function does not sort. Verdicts merge by severity, halting is any-wins,
/// list channels concatenate in order, and scalar channels keep the first value and record later
/// ones as conflicts.
pub fn reduce_effects(ordered: Vec<(HookBindingId, HookEffects)>) -> MergedEffects {
    let mut merged = MergedEffects::no_opinion();
    let mut winning_severity: Option<u8> = None;
    let mut replacement_input_binding: Option<HookBindingId> = None;
    let mut replacement_result_binding: Option<HookBindingId> = None;
    let mut continuation_binding: Option<HookBindingId> = None;

    for (binding, effects) in ordered {
        match effects.verdict.severity() {
            Some(severity) => {
                if winning_severity.is_none_or(|winning| severity > winning) {
                    winning_severity = Some(severity);
                    merged.verdict = effects.verdict;
                    merged.verdict_binding = Some(binding.clone());
                }
            }
            None => merged.unsupported.push(UnsupportedEffect {
                binding: binding.clone(),
                effect: EffectKind::Verdict,
                reason: "defer is not supported; the verdict was ignored".to_owned(),
            }),
        }

        merged
            .reasons
            .extend(effects.reasons.into_iter().map(|reason| HookReason {
                text: reason.text,
                binding: reason.binding.or_else(|| Some(binding.clone())),
            }));
        merged.model_context.extend(effects.model_context);
        merged.user_notices.extend(effects.user_notices);
        merged.session_env.extend(effects.session_env);
        merged.unsupported.extend(effects.unsupported);
        merged.halt_turn |= effects.halt_turn;

        merge_scalar(
            effects.replacement_input,
            &mut merged.replacement_input,
            &mut replacement_input_binding,
            &binding,
            "replacement input",
            &mut merged.conflicts,
        );
        merge_scalar(
            effects.replacement_result,
            &mut merged.replacement_result,
            &mut replacement_result_binding,
            &binding,
            "replacement result",
            &mut merged.conflicts,
        );
        merge_scalar(
            effects.continuation,
            &mut merged.continuation,
            &mut continuation_binding,
            &binding,
            "continuation",
            &mut merged.conflicts,
        );
    }

    merged
}

fn merge_scalar<T>(
    candidate: Option<T>,
    slot: &mut Option<T>,
    slot_binding: &mut Option<HookBindingId>,
    binding: &HookBindingId,
    channel: &str,
    conflicts: &mut Vec<String>,
) {
    let Some(candidate) = candidate else {
        return;
    };
    match slot_binding {
        Some(first) => conflicts.push(format!(
            "{binding} also set a {channel}; the value from {first} was kept"
        )),
        None => {
            *slot = Some(candidate);
            *slot_binding = Some(binding.clone());
        }
    }
}

/// What happens when a hook exits with an unexpected code, crashes, or times out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FailurePolicy {
    FailOpen,
    FailClosed,
}

/// What happens when a hook's output is not valid for its event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MalformedOutputPolicy {
    Deny,
    FollowFailurePolicy,
}

/// What happens when a hook exits successfully with no output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EmptyOutputPolicy {
    FollowFailurePolicy,
    NoOpinion,
}

/// The value a binding's matcher is evaluated against.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MatcherTarget {
    ToolName,
    CommandText,
    SubagentType,
    Literal(String),
    None,
}

/// How the binding's handler runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HandlerKind {
    Command,
    Prompt,
}

/// What one binding is allowed to do and how it fails, as declared by its adapter for the vendor
/// event it was parsed from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindingCapabilities {
    pub may_produce: HashSet<EffectKind>,
    pub failure: FailurePolicy,
    pub malformed_output: MalformedOutputPolicy,
    pub empty_output: EmptyOutputPolicy,
    pub matcher_target: MatcherTarget,
    pub handler: HandlerKind,
}

/// A declared capability the boundary cannot honor in the current phase. The binding still
/// activates; the effect is rejected at runtime if the hook emits it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityWarning {
    pub boundary: HookBoundary,
    pub effect: EffectKind,
    pub message: String,
}

/// Compares declared capabilities against what `boundary` can honor today.
pub fn validate_capabilities(
    boundary: HookBoundary,
    capabilities: &BindingCapabilities,
) -> Vec<CapabilityWarning> {
    let mut warnings = Vec::new();
    for effect in &capabilities.may_produce {
        let message = match effect {
            EffectKind::ReplacementInput => Some(
                "input rewrites are not supported yet; a rewrite, if emitted, will be rejected",
            ),
            EffectKind::ReplacementResult => Some(
                "result replacement is not supported yet; a replacement, if emitted, will be \
                 ignored and reported",
            ),
            EffectKind::ModelContext => Some(
                "model context is not supported yet; context, if emitted, will be ignored and \
                 reported",
            ),
            EffectKind::Verdict => (!boundary.honors_verdict()).then_some(
                "this boundary cannot be blocked; a verdict, if emitted, will be ignored",
            ),
            EffectKind::UserNotice
            | EffectKind::SessionEnv
            | EffectKind::HaltTurn
            | EffectKind::Continuation => None,
        };
        if let Some(message) = message {
            warnings.push(CapabilityWarning {
                boundary,
                effect: *effect,
                message: message.to_owned(),
            });
        }
    }
    // `may_produce` is a set, so sort for a stable order in diagnostics and tests.
    warnings.sort_by_key(|warning| warning.effect);
    warnings
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DiagnosticSeverity {
    Info,
    Warning,
    Error,
}

/// A message about a hook source, produced while loading it or running its hooks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookDiagnostic {
    pub source: HookSourceId,
    pub severity: DiagnosticSeverity,
    pub message: String,
}

#[cfg(test)]
#[path = "model_tests.rs"]
mod tests;

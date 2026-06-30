pub(crate) mod conversation;
pub(crate) mod conversation_yaml;
pub(crate) mod todos;

pub(crate) mod api;
pub(crate) mod comment;
pub(crate) mod icons;
pub(crate) mod linearization;
pub(crate) mod redaction;
pub(crate) mod task;
mod task_store;
pub(super) mod telemetry;
pub(super) mod util;

use std::collections::{HashMap, HashSet};
use std::fmt::Display;
use std::ops::{AddAssign, Deref, DerefMut, Range};
use std::sync::Arc;
use std::time::Duration;

// Re-export types that were moved to the ai crate.
pub use ai::agent::action::*;
pub use ai::agent::action_result::*;
use ai::agent::orchestration_config::{OrchestrationConfig, OrchestrationConfigStatus};
pub use ai::agent::{AIAgentCitation, FileLocations};
use ai::skills::ParsedSkill;
use chrono::{DateTime, Local, TimeDelta};
use comment::ReviewComment;
use derivative::Derivative;
use markdown_parser::{parse_markdown, FormattedTable, FormattedText, FormattedTextInline};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use session_sharing_protocol::common::ParticipantId;
use task::TaskId;
pub use telemetry::AIIdentifiers;
use uuid::Uuid;
use warp_core::channel::ChannelState;
use warp_core::features::FeatureFlag;
use warp_editor::render::model::LineCount;
use warp_multi_agent_api::{diff_hunk as diff_hunk_api, AgentEvent, AgentType};

pub use self::api::{MaybeAIAgentOutputMessage, MessageToAIAgentOutputMessageError};
use super::llms::LLMId;
use crate::ai::block_context::BlockContext;
use crate::ai::blocklist::block::view_impl::output::are_all_text_sections_empty;
use crate::ai::skills::SkillDescriptor;
use crate::ai_assistant::execution_context::WarpAiExecutionContext;
use crate::code::editor_management::CodeSource;
use crate::code_review::comments::{
    AttachedReviewComment as CodeReviewComment, ReviewCommentBatch,
};
use crate::search::slash_command_menu::static_commands::commands;
use crate::server::server_api::{AIApiError, DeserializationError};
use crate::terminal::model::block::BlockId;
use crate::terminal::shell::ShellType;
use crate::TelemetryEvent;

/// A server supplied ID for a specific AI generated output.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash)]
pub struct ServerOutputId(String);

impl std::fmt::Display for ServerOutputId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Display only the inner UUID string without the wrapper
        write!(f, "{}", self.0)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct InvokeSkillUserQuery {
    pub query: String,
    pub referenced_attachments: HashMap<String, AIAgentAttachment>,
}

impl ServerOutputId {
    pub fn new(value: String) -> Self {
        ServerOutputId(value)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CancellationReason {
    /// The user explicitly cancelled without providing a follow-up.
    ManuallyCancelled,
    /// Warp automatically cancelled the local run so it could continue in Cloud Mode.
    AutomaticCloudHandoff,

    /// The user submitted a follow-up query during streaming which implicitly cancelled the current one.
    FollowUpSubmitted {
        is_for_same_conversation: bool,
    },

    /// The user executed a shell command in the middle of the response stream.
    UserCommandExecuted,

    /// The user reverted the conversation to a previous state, deleting exchanges.
    Reverted,

    // The user deleted the conversation while it was in progress.
    Deleted,

    /// The long-running command completed while the agent was still streaming.
    /// This should be treated as a successful completion, not a cancellation.
    OptimisticCLISubagentCompletion,

    /// The user manually took control of a long-running command away from the agent.
    /// The agent conversation is still in progress — it will resume after the command
    /// finishes or once the user hands control back. The stream is cancelled only to
    /// stop the CLI subagent monitoring loop, not to end the conversation.
    CLISubagentUserTakeover,
}

impl Display for CancellationReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CancellationReason::ManuallyCancelled => write!(f, "manual cancellation"),
            CancellationReason::AutomaticCloudHandoff => write!(f, "automatic cloud handoff"),
            CancellationReason::FollowUpSubmitted { .. } => write!(f, "follow-up submission"),
            CancellationReason::UserCommandExecuted => write!(f, "user command execution"),
            CancellationReason::Reverted => write!(f, "revert"),
            CancellationReason::Deleted => write!(f, "deleted"),
            CancellationReason::OptimisticCLISubagentCompletion => {
                write!(f, "LRC command completed")
            }
            CancellationReason::CLISubagentUserTakeover => {
                write!(f, "CLI subagent user takeover")
            }
        }
    }
}

impl CancellationReason {
    pub fn is_follow_up_for_same_conversation(&self) -> bool {
        matches!(
            self,
            CancellationReason::FollowUpSubmitted {
                is_for_same_conversation: true
            }
        )
    }
}

impl CancellationReason {
    pub fn is_manually_cancelled(&self) -> bool {
        matches!(self, CancellationReason::ManuallyCancelled)
    }

    pub fn is_reverted(&self) -> bool {
        matches!(self, CancellationReason::Reverted)
    }

    pub fn is_lrc_command_completed(&self) -> bool {
        matches!(self, CancellationReason::OptimisticCLISubagentCompletion)
    }

    /// Returns true when the stream was cancelled because the user took manual
    /// control of the long-running command. The conversation remains in progress
    /// and the ambient agent task should not be reported as cancelled.
    pub fn is_cli_subagent_user_takeover(&self) -> bool {
        matches!(self, CancellationReason::CLISubagentUserTakeover)
    }

    /// Returns true when the stream cancellation should NOT transition the
    /// conversation status away from InProgress.
    pub fn should_preserve_in_progress_status(&self) -> bool {
        self.is_follow_up_for_same_conversation() || self.is_cli_subagent_user_takeover()
    }
}

#[derive(Clone, Debug)]
pub enum FinishedAIAgentOutput {
    /// The user manually cancelled output streaming.
    Cancelled {
        // The output received up til the point of cancellation, if any.
        output: Option<Shared<AIAgentOutput>>,
        /// Why the stream was cancelled.
        reason: CancellationReason,
    },
    /// Output streaming failed.
    Error {
        // The output received up til the error was encountered, if any.
        output: Option<Shared<AIAgentOutput>>,
        error: RenderableAIError,
    },
    /// Output streaming completed successfully.
    Success { output: Shared<AIAgentOutput> },
}

impl Display for FinishedAIAgentOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FinishedAIAgentOutput::Cancelled { .. } => write!(f, "Cancelled"),
            FinishedAIAgentOutput::Error { error, .. } => write!(f, "Error: {error}"),
            FinishedAIAgentOutput::Success { output } => write!(f, "\n{output}"),
        }
    }
}

impl FinishedAIAgentOutput {
    pub fn server_output_id(&self) -> Option<ServerOutputId> {
        self.output()
            .and_then(|output| output.get().server_output_id.clone())
    }

    pub fn model_id(&self) -> Option<LLMId> {
        self.output()
            .and_then(|output| output.get().model_info.as_ref().map(|m| m.model_id.clone()))
    }

    pub fn output(&self) -> Option<&Shared<AIAgentOutput>> {
        match self {
            Self::Cancelled { output, .. } => output.as_ref(),
            Self::Error { .. } => None,
            Self::Success { output } => Some(output),
        }
    }
}

#[derive(Debug)]
pub struct Shared<T> {
    value: Arc<RwLock<T>>,
}

impl<T> Clone for Shared<T>
where
    T: Clone,
{
    fn clone(&self) -> Self {
        Self {
            value: Arc::new(RwLock::new(self.value.read().clone())),
        }
    }
}

impl<T: Clone + std::fmt::Debug> Shared<T> {
    pub fn new(value: T) -> Self {
        Self {
            value: Arc::new(RwLock::new(value)),
        }
    }

    pub fn get(&self) -> impl Deref<Target = T> + '_ {
        self.value.read()
    }

    /// Returns an owned `Shared` pointing to the same underlying `T`.
    ///
    /// While `Clone` performs a deep copy on the other value, this ultimately points to the same
    /// value `T`.
    pub fn get_owned(&self) -> Shared<T> {
        Self {
            value: self.value.clone(),
        }
    }

    fn get_mut(&self) -> impl DerefMut<Target = T> + '_ {
        self.value.write()
    }
}

impl<T: Display> Display for Shared<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.value.read().fmt(f)
    }
}

/// Status of output streaming from the AI API.
#[derive(Clone, Debug)]
pub enum AIAgentOutputStatus {
    Streaming {
        output: Option<Shared<AIAgentOutput>>,
    },
    Finished {
        finished_output: FinishedAIAgentOutput,
    },
}

impl Display for AIAgentOutputStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AIAgentOutputStatus::Streaming { .. } => write!(f, "Streaming..."),
            AIAgentOutputStatus::Finished { finished_output } => write!(f, "{finished_output}"),
        }
    }
}

impl AIAgentOutputStatus {
    pub fn server_output_id(&self) -> Option<ServerOutputId> {
        self.output()
            .and_then(|output| output.get().server_output_id.clone())
    }

    pub fn cancel_reason(&self) -> Option<&CancellationReason> {
        match self {
            Self::Finished {
                finished_output: FinishedAIAgentOutput::Cancelled { reason, .. },
            } => Some(reason),
            _ => None,
        }
    }

    pub fn model_id(&self) -> Option<LLMId> {
        self.output()
            .and_then(|output| output.get().model_info.as_ref().map(|m| m.model_id.clone()))
    }

    pub fn output(&self) -> Option<&Shared<AIAgentOutput>> {
        match self {
            Self::Streaming { output, .. } => output.as_ref(),
            Self::Finished {
                finished_output, ..
            } => finished_output.output(),
        }
    }

    pub fn is_cancelled(&self) -> bool {
        matches!(
            self,
            Self::Finished {
                finished_output: FinishedAIAgentOutput::Cancelled { .. },
                ..
            }
        )
    }

    pub fn is_finished(&self) -> bool {
        match self {
            Self::Streaming { .. } => false,
            Self::Finished { .. } => true,
        }
    }

    pub fn is_finished_and_successful(&self) -> bool {
        matches!(
            self,
            Self::Finished {
                finished_output: FinishedAIAgentOutput::Success { .. }
            }
        )
    }

    pub fn is_streaming(&self) -> bool {
        matches!(self, Self::Streaming { .. })
    }
}

// This value is the cost of a single request.
// It is returned as part of the final response chunk from the agent.
#[derive(Clone, Copy, Default, Deserialize, Serialize, PartialOrd, Derivative)]
#[derivative(Debug, PartialEq, Eq)]
pub struct RequestCost(f64);

impl RequestCost {
    pub fn new(value: f64) -> Self {
        Self(value)
    }

    pub fn value(&self) -> f64 {
        self.0
    }

    pub fn zero() -> Self {
        Self(0.0)
    }

    pub fn one() -> Self {
        Self(1.0)
    }
}

impl AddAssign for RequestCost {
    fn add_assign(&mut self, rhs: Self) {
        self.0 += rhs.0;
    }
}

impl std::iter::Sum for RequestCost {
    fn sum<I: Iterator<Item = Self>>(iter: I) -> Self {
        Self(iter.map(|cost| cost.0).sum())
    }
}

impl std::fmt::Display for RequestCost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// The AI output received in response to a user prompt/query.
#[derive(Clone, Default, Derivative)]
#[derivative(Debug, PartialEq, Eq)]
pub struct AIAgentOutput {
    pub messages: Vec<AIAgentOutputMessage>,

    /// The set of documents that were referenced in the LLM's response.
    pub citations: Vec<AIAgentCitation>,

    /// Unique ID generated by `warp-server`. Used to join client and server telemetry and logs.
    pub server_output_id: Option<ServerOutputId>,

    /// Optional metadata that may be attached by the `AIAgentApi` when emitting this output.
    ///
    /// This is guaranteed to be stored and passed back with this output if/when it is passed back
    /// to `AIAgentApi` as conversation history.
    pub api_metadata_bytes: Option<Vec<u8>>,

    /// Suggested objects to apply to the output.
    pub suggestions: Option<Suggestions>,

    /// Information about the model that generated this output.
    pub model_info: Option<OutputModelInfo>,

    /// Telemetry events related to the AI Agent Output that we want to send after completion.
    #[derivative(Debug = "ignore")]
    #[derivative(PartialEq = "ignore")]
    pub telemetry_events: Vec<TelemetryEvent>,

    /// The number of requests that the request cost.
    pub request_cost: Option<RequestCost>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutputModelInfo {
    pub model_id: LLMId,
    pub display_name: String,
    pub is_fallback: bool,
    /// When the provider-side prompt cache for this request is expected to
    /// expire. `None` means unknown / no cache-expiry info.
    pub prompt_cache_expires_at: Option<DateTime<Local>>,
}

impl Display for AIAgentOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (i, message) in self.messages.iter().enumerate() {
            if i > 0 {
                writeln!(f)?;
            }
            write!(f, "Message {}: {}", i + 1, message)?;
        }
        Ok(())
    }
}

impl AIAgentOutput {
    /// Returns only the text from agent output messages in the output.
    pub fn text_from_agent_output(&self) -> impl Iterator<Item = &AIAgentText> {
        self.messages
            .iter()
            .filter_map(|message| match &message.message {
                AIAgentOutputMessageType::Text(text) => Some(text),
                _ => None,
            })
    }

    /// Returns only the text from reasoning messages in the output.
    pub fn text_from_agent_reasoning(&self) -> impl Iterator<Item = &AIAgentText> {
        self.messages
            .iter()
            .filter_map(|message| match &message.message {
                AIAgentOutputMessageType::Reasoning { text, .. } => Some(text),
                _ => None,
            })
    }

    /// Returns all of the text contained in the output, including agent output, reasoning,
    /// and conversation summaries.
    ///
    /// IMPORTANT: This must stay in sync with the rendering code in `output.rs` — every
    /// message type whose sections increment `text_section_index` during rendering must
    /// also be yielded here, otherwise link detection indices will be offset.
    pub fn all_text(&self) -> impl Iterator<Item = &AIAgentText> {
        self.messages
            .iter()
            .filter_map(|message| match &message.message {
                AIAgentOutputMessageType::Text(text) => Some(text),
                AIAgentOutputMessageType::Reasoning { text, .. } => Some(text),
                AIAgentOutputMessageType::Summarization {
                    text,
                    summarization_type: SummarizationType::ConversationSummary,
                    ..
                } => Some(text),
                _ => None,
            })
            // It's important to filter these out, because we filter these out when rendering the output
            // and the text_section_index must match for detected links to work.
            .filter(|text| !are_all_text_sections_empty(&text.sections))
    }

    /// Returns all of the text contained in the output with their message IDs, including agent output,
    /// reasoning, and conversation summaries.
    ///
    /// IMPORTANT: This must stay in sync with the rendering code in `output.rs` — see [`all_text`].
    pub fn all_text_with_message_id(&self) -> impl Iterator<Item = (&MessageId, &AIAgentText)> {
        self.messages
            .iter()
            .filter_map(|message| match &message.message {
                AIAgentOutputMessageType::Text(text) => Some((&message.id, text)),
                AIAgentOutputMessageType::Reasoning { text, .. } => Some((&message.id, text)),
                AIAgentOutputMessageType::Summarization {
                    text,
                    summarization_type: SummarizationType::ConversationSummary,
                    ..
                } => Some((&message.id, text)),
                _ => None,
            })
            // It's important to filter these out, because we filter these out when rendering the output
            // and the text_section_index must match for detected links to work.
            .filter(|(_, text)| !are_all_text_sections_empty(&text.sections))
    }

    pub fn actions(&self) -> impl Iterator<Item = &AIAgentAction> {
        self.messages
            .iter()
            .filter_map(|message| match &message.message {
                AIAgentOutputMessageType::Action(action) => Some(action),
                _ => None,
            })
    }

    pub fn todo_operations(&self) -> impl Iterator<Item = &TodoOperation> {
        self.messages
            .iter()
            .filter_map(|message| match &message.message {
                AIAgentOutputMessageType::TodoOperation(operation) => Some(operation),
                _ => None,
            })
    }

    /// Format this output for copying to clipboard.
    /// This extracts all content (text, code, and action results) with proper formatting.
    pub fn format_for_copy(
        &self,
        action_model: Option<&crate::ai::blocklist::BlocklistAIActionModel>,
    ) -> String {
        let mut result = Vec::new();
        let mut last_was_action = false;

        // Process all messages in order, collecting all content
        for message in &self.messages {
            match &message.message {
                AIAgentOutputMessageType::Text(text) => {
                    // If the last message was an action and this is text, add some separation
                    if last_was_action {
                        result.push(String::new()); // Add blank line for readability
                    }

                    // Collect all text and code sections from this text message
                    for section in &text.sections {
                        match section {
                            AIAgentTextSection::PlainText { text } => {
                                result.push(text.text().to_string());
                            }
                            AIAgentTextSection::Code { .. }
                            | AIAgentTextSection::Table { .. }
                            | AIAgentTextSection::Image { .. }
                            | AIAgentTextSection::MermaidDiagram { .. } => {
                                result.push(format!("{}", MarkdownTextSection(section)));
                            }
                        }
                    }
                    last_was_action = false;
                }
                AIAgentOutputMessageType::Action(action) => {
                    // Include action results from the action model if available
                    if let Some(action_model) = action_model {
                        if let Some(action_result) = action_model.get_action_result(&action.id) {
                            result.push(format!("{}", MarkdownActionResult(&action_result.result)));
                            // Add an extra newline after tool call results for readability
                            result.push(String::new());
                            last_was_action = true;
                        }
                    }
                }
                AIAgentOutputMessageType::TodoOperation(operation) => {
                    result.push(format!("{operation}"));
                    last_was_action = false;
                }
                AIAgentOutputMessageType::Subagent(subagent) => {
                    result.push(format!("{subagent}"));
                    last_was_action = false;
                }
                AIAgentOutputMessageType::CommentsAddressed {
                    comments: comment_ids,
                } => {
                    result.push(format!("Addressed {} comments", comment_ids.len()));
                    last_was_action = false;
                }
                AIAgentOutputMessageType::Reasoning { .. } => continue,
                AIAgentOutputMessageType::Summarization { .. } => continue,
                AIAgentOutputMessageType::WebSearch(_) => continue,
                AIAgentOutputMessageType::WebFetch(_) => continue,
                AIAgentOutputMessageType::DebugOutput { text } => {
                    result.push(format!("[DEBUG] {text}"));
                    last_was_action = false;
                }
                AIAgentOutputMessageType::ArtifactCreated(_) => continue,
                AIAgentOutputMessageType::SkillInvoked(_) => continue,
                AIAgentOutputMessageType::MessagesReceivedFromAgents { messages } => {
                    result.push(format!("Received {} messages", messages.len()));
                    last_was_action = false;
                }
                AIAgentOutputMessageType::EventsFromAgents { event_ids } => {
                    result.push(format!("Received {} agent events", event_ids.len()));
                    last_was_action = false;
                }
            }
        }

        // Remove trailing empty lines
        while result.last() == Some(&String::new()) {
            result.pop();
        }

        result.join("\n")
    }

    pub fn extend_citations(&mut self, citations: Vec<AIAgentCitation>) {
        let new_citations: Vec<_> = citations
            .into_iter()
            .filter(|c| !self.citations.contains(c))
            .collect();
        self.citations.extend(new_citations);
    }

    /// Calculate the action index for a given action_id by counting preceding actions in the output.
    /// Returns the 0-based index of the action, or None if the action is not found.
    pub fn calculate_action_index(&self, target_action_id: &AIAgentActionId) -> Option<usize> {
        let mut action_index = 0;
        for output_message in &self.messages {
            if let AIAgentOutputMessageType::Action(AIAgentAction { id, .. }) =
                &output_message.message
            {
                if id == target_action_id {
                    return Some(action_index);
                }
                action_index += 1;
            }
        }
        None // Fallback if action_id not found
    }
}

/// Represents user visible errors.
#[derive(Clone, Debug)]
pub enum RenderableAIError {
    QuotaLimit {
        user_display_message: Option<String>,
    },
    ServerOverloaded,
    InternalWarpError,
    ContextWindowExceeded(String),
    InvalidApiKey {
        provider: String,
        model_name: String,
    },
    AwsBedrockCredentialsExpiredOrInvalid {
        model_name: String,
    },
    /// A transient network failure (lost connection or truncated response stream). Carries its
    /// own complete user-facing copy; `kind` preserves the structured cause (including the raw
    /// API error) so user reports can disambiguate the different causes behind the shared message.
    TransientNetworkError {
        kind: TransientNetworkErrorKind,
        will_attempt_resume: bool,
        /// When `will_attempt_resume` is true, this indicates whether we're waiting for network
        /// connectivity before attempting the resume.
        waiting_for_network: bool,
    },
    Other {
        error_message: String,
        will_attempt_resume: bool,
        /// When `will_attempt_resume` is true, this indicates whether we're waiting for network
        /// connectivity before attempting the resume.
        waiting_for_network: bool,
        /// True when the error originates from a user-side issue (e.g., model not allowed,
        /// blocked due to fraud, plan restriction). Maps the task to FAILED state instead of ERROR.
        is_user_error: bool,
    },
}

impl RenderableAIError {
    const TRANSIENT_NETWORK_ERROR_MESSAGE: &'static str =
        "Warp lost connection while receiving the agent response. This is usually temporary.";
    /// Creates a transient network error. `kind` is the structured cause (including the raw API
    /// error where one exists), preserved so user reports can disambiguate the different causes
    /// behind the shared user-facing copy.
    pub fn transient_network_error(
        will_attempt_resume: bool,
        waiting_for_network: bool,
        kind: TransientNetworkErrorKind,
    ) -> Self {
        Self::TransientNetworkError {
            kind,
            will_attempt_resume,
            waiting_for_network,
        }
    }

    fn is_transient_network_transport_error(error: &reqwest::Error) -> bool {
        // If reqwest has an HTTP status, the server responded. Preserve the existing generic
        // rendering for those failures rather than calling them lost connections.
        error.status().is_none()
    }

    pub fn is_invalid_api_key(&self) -> bool {
        matches!(self, Self::InvalidApiKey { .. })
    }

    pub fn is_aws_bedrock_credentials_error(&self) -> bool {
        matches!(self, Self::AwsBedrockCredentialsExpiredOrInvalid { .. })
    }

    /// Returns true if an automatic resume will be attempted for this error.
    pub fn will_attempt_resume(&self) -> bool {
        matches!(
            self,
            Self::Other {
                will_attempt_resume: true,
                ..
            } | Self::TransientNetworkError {
                will_attempt_resume: true,
                ..
            }
        )
    }

    /// Whether the failed-output UI should be suppressed while an automatic resume is in
    /// flight. Release builds stay quiet so transient blips that recover on their own
    /// don't surface an alarming error; dogfood builds (Local/Dev) keep the old, more
    /// aggressive behavior so developers still see every transport failure.
    pub fn should_suppress_during_recovery(&self) -> bool {
        self.will_attempt_resume() && !ChannelState::channel().is_dogfood()
    }
}

/// The cause behind a [`RenderableAIError::TransientNetworkError`]. Kept structured (rather than
/// collapsed to a free-form string) so user reports preserve the raw error; rendered to text only
/// at display time.
#[derive(Clone, Debug, thiserror::Error)]
pub enum TransientNetworkErrorKind {
    /// A lost connection or truncated response stream — the raw underlying API error. Rendered via
    /// `Debug` so reports preserve the full structured error rather than its terse `Display`.
    #[error("{0:?}")]
    Api(Arc<AIApiError>),
    /// The response stream completed with an unfinished exchange and no error event.
    #[error("stream completed with an unfinished exchange and no error event")]
    UnfinishedExchange,
    /// The conversation was left in a transient-error state but the last exchange carried no
    /// structured error to surface.
    #[error("no structured error on the last exchange")]
    MissingExchangeError,
}

impl From<&Arc<AIApiError>> for RenderableAIError {
    fn from(value: &Arc<AIApiError>) -> Self {
        // Non-retryable 4xx errors (403 fraud block, 400 model/plan restriction, etc.)
        // are user-originating — map them to a user error so the task reaches FAILED
        // state rather than ERROR state.
        let is_user_error = !value.is_recoverable();
        match value.as_ref() {
            AIApiError::QuotaLimit {
                user_display_message,
            } => Self::QuotaLimit {
                user_display_message: user_display_message.clone(),
            },
            AIApiError::ServerOverloaded => Self::ServerOverloaded,
            AIApiError::Transport(error)
            | AIApiError::Deserialization(DeserializationError::Transport(error)) => {
                // A transport error with no HTTP status is a lost-connection failure; one that
                // carries a status means the server responded, so it gets generic rendering.
                if Self::is_transient_network_transport_error(error) {
                    Self::transient_network_error(
                        false,
                        false,
                        TransientNetworkErrorKind::Api(value.clone()),
                    )
                } else {
                    Self::Other {
                        error_message: format!("Request failed with error: {value:?}"),
                        will_attempt_resume: false,
                        waiting_for_network: false,
                        is_user_error,
                    }
                }
            }
            AIApiError::UnexpectedEof => Self::transient_network_error(
                false,
                false,
                TransientNetworkErrorKind::Api(value.clone()),
            ),
            AIApiError::Deserialization(DeserializationError::Json(_))
            | AIApiError::NoContextFound
            | AIApiError::ErrorStatus(_, _)
            | AIApiError::Other(_)
            | AIApiError::Stream { .. } => Self::Other {
                error_message: format!("Request failed with error: {value:?}"),
                will_attempt_resume: false,
                waiting_for_network: false,
                is_user_error,
            },
        }
    }
}

impl Display for RenderableAIError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::QuotaLimit {
                user_display_message,
            } => {
                if let Some(message) = user_display_message {
                    write!(f, "{message}")
                } else {
                    write!(f, "Quota limit reached.")
                }
            }
            Self::ServerOverloaded => {
                write!(f, "Warp is currently overloaded. Please try again later.")
            }
            Self::InternalWarpError => write!(f, "Internal Warp error."),
            Self::ContextWindowExceeded(message) => {
                write!(f, "Context window exceeded: {message}")
            }
            Self::InvalidApiKey { provider, .. } => {
                write!(f, "Invalid API key for {provider}")
            }
            Self::AwsBedrockCredentialsExpiredOrInvalid { model_name } => {
                write!(
                    f,
                    "AWS Bedrock credentials expired or invalid for {model_name}"
                )
            }
            Self::TransientNetworkError { kind, .. } => {
                write!(
                    f,
                    "{}\n\nDebug info: {kind}",
                    Self::TRANSIENT_NETWORK_ERROR_MESSAGE
                )
            }
            Self::Other { error_message, .. } => write!(f, "{error_message}"),
        }
    }
}

#[allow(unused)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProgrammingLanguage {
    Shell(ShellType),
    Other(String),
}

impl ProgrammingLanguage {
    pub fn display_name(&self) -> String {
        match self {
            Self::Shell(shell_type) => shell_type.name().to_owned(),
            Self::Other(language) => language.to_lowercase(),
        }
    }

    /// Returns the file extension for the given programming language.
    // TODO(INT-605): Refactor so we don't have to edit this function and the `languages` crate.
    #[cfg_attr(target_family = "wasm", allow(unused))]
    pub fn to_extension(&self) -> Option<&str> {
        match self {
            // The arms below cover both canonical language names emitted by the agent (e.g.
            // "rust", "kotlin") and common markdown code-fence aliases (e.g. "rs", "kt") to keep
            // syntax highlighting working when the model uses either. The set of recognized
            // languages here is kept in sync with `SUPPORTED_LANGUAGES` in the `languages` crate.
            Self::Other(language) => match language.to_lowercase().as_str() {
                "rust" | "rs" => Some("rs"),
                "go" | "golang" => Some("go"),
                "python" | "py" => Some("py"),
                "javascript" | "js" => Some("js"),
                "typescript" | "ts" => Some("ts"),
                "jsx" => Some("jsx"),
                "tsx" => Some("tsx"),
                "yaml" | "yml" => Some("yaml"),
                "cpp" | "c++" => Some("cpp"),
                "java" => Some("java"),
                "groovy" => Some("java"),
                "shell" => Some("sh"),
                "c#" | "csharp" => Some("cs"),
                "html" => Some("html"),
                "css" => Some("css"),
                "c" => Some("c"),
                "json" => Some("json"),
                "jq" => Some("jq"),
                "hcl" | "terraform" | "tf" => Some("hcl"),
                "lua" => Some("lua"),
                "ruby" | "rb" => Some("rb"),
                "php" => Some("php"),
                "toml" => Some("toml"),
                "swift" => Some("swift"),
                "kotlin" | "kt" => Some("kt"),
                "powershell" => Some("ps1"),
                "elixir" => Some("exs"),
                "scala" => Some("scala"),
                "sql" => Some("sql"),
                "objective-c" | "objc" => Some("m"),
                "starlark" => Some("bzl"),
                "xml" => Some("xml"),
                "vue" => Some("vue"),
                "dockerfile" | "docker" | "containerfile" => Some("dockerfile"),
                _ => None,
            },
            Self::Shell(ShellType::PowerShell) => Some("ps1"),
            _ => None,
        }
    }

    /// Return whether this language is a shell language.
    /// This is used to determine whether to show the "execute in terminal" button.
    pub fn is_shell(&self) -> bool {
        matches!(self, Self::Shell(_))
    }
}

impl Display for ProgrammingLanguage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProgrammingLanguage::Shell(shell_type) => write!(f, "{}", shell_type.name()),
            ProgrammingLanguage::Other(language) => write!(f, "{}", language.to_lowercase()),
        }
    }
}

impl From<String> for ProgrammingLanguage {
    // Returns a programming language for a markdown language specifier
    fn from(value: String) -> Self {
        if let Some(shell_type) = ShellType::from_markdown_language_spec(value.as_str()) {
            ProgrammingLanguage::Shell(shell_type)
        } else {
            ProgrammingLanguage::Other(value)
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum AgentOutputImageLayout {
    Block,
    Inline,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct SuggestedAgentModeWorkflow {
    pub name: String,
    pub prompt: String,
    pub logging_id: SuggestedLoggingId,
}

/// A ID for an AI action generated as part of an [`AIAgentOutput`].
///
/// The internal ID itself should be opaque to all callers. This ID may be relayed back to the AI with
/// the `AIAgentActionResult` from the action.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AIAgentActionId(String);

impl From<String> for AIAgentActionId {
    fn from(value: String) -> Self {
        AIAgentActionId(value)
    }
}

impl From<AIAgentActionId> for String {
    fn from(value: AIAgentActionId) -> Self {
        value.0
    }
}

impl Display for AIAgentActionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl From<crate::persistence::model::AIAgentActionId> for AIAgentActionId {
    fn from(value: crate::persistence::model::AIAgentActionId) -> Self {
        Self(value.0)
    }
}

impl From<AIAgentActionId> for crate::persistence::model::AIAgentActionId {
    fn from(value: AIAgentActionId) -> Self {
        crate::persistence::model::AIAgentActionId(value.0)
    }
}

/// An "action" included in an AI output.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct AIAgentAction {
    /// Unique ID for the action.
    pub id: AIAgentActionId,

    /// The ID of the task to which this action belongs.
    pub task_id: TaskId,

    /// The action itself.
    pub action: AIAgentActionType,

    /// `true` if this action requires a corresponding `AIAgentActionResult` to be sent back to the
    /// AI API.
    ///
    /// If this is `true`, a corresponding result _must_ be included in the next query to the AI.
    pub requires_result: bool,
}

impl Display for AIAgentAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.action)
    }
}

impl AIAgentAction {
    pub fn is_request_file_edit(&self) -> bool {
        matches!(self.action, AIAgentActionType::RequestFileEdits { .. })
    }

    pub fn is_request_command_output(&self) -> bool {
        self.action.is_request_command_output()
    }

    pub fn is_agent_monitored_request_command_output(&self) -> bool {
        matches!(
            self.action,
            AIAgentActionType::RequestCommandOutput {
                wait_until_completion: false,
                ..
            }
        )
    }

    pub fn is_get_specific_files(&self) -> bool {
        self.action.is_read_files()
    }

    pub fn is_get_relevant_files(&self) -> bool {
        self.action.is_search_codebase()
    }

    pub fn is_grep(&self) -> bool {
        self.action.is_grep()
    }

    pub fn is_file_glob(&self) -> bool {
        self.action.is_file_glob()
    }

    pub fn executable_command(&self) -> Option<String> {
        match &self.action {
            AIAgentActionType::RequestCommandOutput { command, .. } => Some(command.clone()),
            _ => None,
        }
    }

    pub fn is_write_to_shell_command(&self) -> bool {
        self.action.is_write_to_shell_command()
    }

    pub fn matches_command(&self, command: &String) -> bool {
        Some(command) == self.executable_command().as_ref()
    }
}

pub struct MarkdownTextSection<'a>(pub &'a AIAgentTextSection);

impl<'a> std::fmt::Display for MarkdownTextSection<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0 {
            AIAgentTextSection::PlainText { text } => {
                write!(f, "{}", text.text())
            }
            AIAgentTextSection::Code {
                code,
                language,
                source,
            } => {
                write!(f, "```")?;
                if let Some(lang) = language {
                    write!(f, "{}", lang.display_name())?;
                }
                if let Some(CodeSource::Link {
                    path,
                    range_start,
                    range_end,
                }) = source
                {
                    write!(f, " path={path:?}")?;
                    if let (Some(range_start), Some(range_end)) = (range_start, range_end) {
                        write!(
                            f,
                            " start={} end={}",
                            range_start.line_num, range_end.line_num
                        )?;
                    }
                }
                writeln!(f)?;
                writeln!(f, "{code}")?;
                write!(f, "```")
            }
            AIAgentTextSection::Table { table } => {
                write!(f, "{}", table.markdown_source)
            }
            AIAgentTextSection::Image { image } => {
                write!(f, "{}", image.markdown_source)
            }
            AIAgentTextSection::MermaidDiagram { diagram } => {
                write!(f, "{}", diagram.markdown_source)
            }
        }
    }
}

pub struct MarkdownActionResult<'a>(pub &'a AIAgentActionResultType);

impl<'a> std::fmt::Display for MarkdownActionResult<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0 {
            AIAgentActionResultType::RequestCommandOutput(result) => match result {
                RequestCommandOutputResult::Completed {
                    command,
                    output,
                    exit_code: _,
                    ..
                } => {
                    write!(
                        f,
                        "\n**Command Executed:**\n```bash\n{command}\n```\n\n**Output:**\n```\n{output}\n```"
                    )
                }
                RequestCommandOutputResult::LongRunningCommandSnapshot {
                    command,
                    grid_contents,
                    ..
                } => {
                    write!(
                        f,
                        "\n```bash\n{command}\n```\n\n**Current Output:**\n```\n{grid_contents}\n```"
                    )
                }
                RequestCommandOutputResult::CancelledBeforeExecution => {
                    write!(f, "\n_Command cancelled_")
                }
                RequestCommandOutputResult::Denylisted { command } => {
                    write!(
                        f,
                        "\nCommand ({command}) was on denylist and so was not allowed to run"
                    )
                }
            },
            AIAgentActionResultType::WriteToLongRunningShellCommand(result) => match result {
                WriteToLongRunningShellCommandResult::CommandFinished { output, .. } => {
                    write!(f, "\n```\n{output}\n```")
                }
                WriteToLongRunningShellCommandResult::Snapshot { grid_contents, .. } => {
                    write!(f, "\n```\n{grid_contents}\n```")
                }
                WriteToLongRunningShellCommandResult::Cancelled => {
                    write!(f, "\n_Command cancelled_")
                }
                WriteToLongRunningShellCommandResult::Error(e) => {
                    write!(f, "\n_Write to command failed: {e:?}")
                }
            },
            AIAgentActionResultType::RequestFileEdits(result) => match result {
                RequestFileEditsResult::Success { diff, .. } => {
                    write!(f, "\n\n**Diff:**\n```diff\n{diff}\n```\n\n")
                }
                RequestFileEditsResult::Cancelled => write!(f, "\n_File edits cancelled_"),
                RequestFileEditsResult::DiffApplicationFailed { error } => {
                    write!(f, "\n_File edits failed: {error} _")
                }
            },
            AIAgentActionResultType::ReadFiles(result) => match result {
                ReadFilesResult::Success { files } => {
                    write!(f, "\n\n**Files Read:**\n\n")?;
                    for file in files {
                        writeln!(f, "**{}**", file.file_name)?;
                        let content = &file.content;
                        if let AnyFileContent::StringContent(text) = content {
                            if !text.trim().is_empty() {
                                writeln!(f, "```\n{text}\n```\n")?;
                            }
                        }
                    }
                    Ok(())
                }
                ReadFilesResult::Error(error) => write!(f, "\n_Read files error: {error} _"),
                ReadFilesResult::Cancelled => write!(f, "\n_Read files cancelled_"),
            },
            AIAgentActionResultType::UploadArtifact(result) => match result {
                UploadArtifactResult::Success {
                    artifact_uid,
                    filepath,
                    mime_type,
                    description,
                    size_bytes,
                } => {
                    write!(f, "\n**Artifact Uploaded:** `{artifact_uid}`")?;
                    if let Some(filepath) = filepath {
                        write!(f, "\n\n**File:** `{filepath}`")?;
                    }
                    write!(
                        f,
                        "\n\n**MIME Type:** `{mime_type}`\n\n**Size:** `{size_bytes}` bytes"
                    )?;
                    if let Some(description) = description {
                        write!(f, "\n\n**Description:** {description}")?;
                    }
                    Ok(())
                }
                UploadArtifactResult::Error(error) => {
                    write!(f, "\n_Upload artifact error: {error} _")
                }
                UploadArtifactResult::Cancelled => write!(f, "\n_Upload artifact cancelled_"),
            },
            AIAgentActionResultType::SearchCodebase(result) => match result {
                SearchCodebaseResult::Success { files } => {
                    write!(f, "\n\n**Codebase Search Results:**\n\n")?;
                    for file in files {
                        writeln!(f, "- **{}**", file.file_name)?;
                        let content = &file.content;
                        if let AnyFileContent::StringContent(text) = content {
                            if !text.trim().is_empty() {
                                writeln!(f, "```\n{text}\n```\n")?;
                            }
                        }
                    }
                    Ok(())
                }
                SearchCodebaseResult::Failed { message, .. } => {
                    write!(f, "\n_Codebase search failed: {message} _")
                }
                SearchCodebaseResult::Cancelled => write!(f, "\n_Codebase search cancelled_"),
            },
            AIAgentActionResultType::FileGlobV2(result) => match result {
                FileGlobV2Result::Success { matched_files, .. } => {
                    write!(f, "\n\n**File Glob Results:**\n\n")?;
                    for file in matched_files {
                        writeln!(f, "- **{}**", file.file_path)?;
                    }
                    Ok(())
                }
                FileGlobV2Result::Error(message) => {
                    write!(f, "\n_File glob error: {message} _")
                }
                FileGlobV2Result::Cancelled => write!(f, "\n_File glob cancelled_"),
            },
            AIAgentActionResultType::Grep(result) => match result {
                GrepResult::Success { matched_files } => {
                    write!(f, "\n\n**Grep Results:**\n\n")?;
                    for file in matched_files {
                        writeln!(f, "- **{}**", file.file_path)?;
                    }
                    Ok(())
                }
                GrepResult::Error(message) => {
                    write!(f, "\n_Grep error: {message} _")
                }
                GrepResult::Cancelled => write!(f, "\n_Grep cancelled_"),
            },
            AIAgentActionResultType::ReadDocuments(result) => match result {
                ReadDocumentsResult::Success { documents } => {
                    write!(f, "\n\n**Documents Read:**\n\n")?;
                    for document in documents {
                        writeln!(f, "**Document {}**", document.document_id)?;
                        if !document.content.trim().is_empty() {
                            writeln!(f, "```\n{}\n```\n", document.content)?;
                        }
                    }
                    Ok(())
                }
                ReadDocumentsResult::Error(error) => {
                    write!(f, "\n_Read documents error: {error} _")
                }
                ReadDocumentsResult::Cancelled => write!(f, "\n_Read documents cancelled_"),
            },
            AIAgentActionResultType::EditDocuments(result) => match result {
                EditDocumentsResult::Success { updated_documents } => {
                    write!(f, "\n\n**Documents Edited:**\n\n")?;
                    for document in updated_documents {
                        writeln!(f, "**Document {}**", document.document_id)?;
                        if !document.content.trim().is_empty() {
                            writeln!(f, "```\n{}\n```\n", document.content)?;
                        }
                    }
                    Ok(())
                }
                EditDocumentsResult::Error(error) => {
                    write!(f, "\n_Edit documents error: {error} _")
                }
                EditDocumentsResult::Cancelled => write!(f, "\n_Edit documents cancelled_"),
            },
            AIAgentActionResultType::CreateDocuments(result) => match result {
                CreateDocumentsResult::Success { created_documents } => {
                    write!(f, "\n\n**Documents Created:**\n\n")?;
                    for document in created_documents {
                        writeln!(f, "**Document {}**", document.document_id)?;
                        if !document.content.trim().is_empty() {
                            writeln!(f, "```\n{}\n```\n", document.content)?;
                        }
                    }
                    Ok(())
                }
                CreateDocumentsResult::Error(error) => {
                    write!(f, "\n_Create documents error: {error} _")
                }
                CreateDocumentsResult::Cancelled => write!(f, "\n_Create documents cancelled_"),
            },
            AIAgentActionResultType::ReadShellCommandOutput(result) => match result {
                ReadShellCommandOutputResult::CommandFinished { output, .. } => {
                    write!(f, "\n```\n{output}\n```")
                }
                ReadShellCommandOutputResult::LongRunningCommandSnapshot {
                    command,
                    grid_contents,
                    ..
                } => {
                    write!(
                        f,
                        "\n```bash\n{command}\n```\n\n**Current Output:**\n```\n{grid_contents}\n```"
                    )
                }
                ReadShellCommandOutputResult::Cancelled => {
                    write!(f, "\n_Command cancelled_")
                }
                ReadShellCommandOutputResult::Error(e) => {
                    write!(f, "\n_Read shell command output failed: {e:?}_")
                }
            },
            other => {
                write!(f, "{other}")
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AIAgentActionResult {
    pub id: AIAgentActionId,
    pub task_id: TaskId,
    pub result: AIAgentActionResultType,
}

impl Display for AIAgentActionResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.result)
    }
}

impl AIAgentActionResult {
    /// Returns `true` if this action was explicitly rejected by the user.
    pub fn is_rejected(&self) -> bool {
        matches!(
            self.result,
            AIAgentActionResultType::RequestFileEdits(RequestFileEditsResult::Cancelled)
                | AIAgentActionResultType::RequestCommandOutput(
                    RequestCommandOutputResult::CancelledBeforeExecution
                )
                | AIAgentActionResultType::ReadFiles(ReadFilesResult::Cancelled)
                | AIAgentActionResultType::UploadArtifact(UploadArtifactResult::Cancelled)
                | AIAgentActionResultType::SearchCodebase(SearchCodebaseResult::Cancelled)
                | AIAgentActionResultType::Grep(GrepResult::Cancelled)
                | AIAgentActionResultType::FileGlob(FileGlobResult::Cancelled)
                | AIAgentActionResultType::ReadMCPResource(ReadMCPResourceResult::Cancelled)
                | AIAgentActionResultType::CallMCPTool(CallMCPToolResult::Cancelled)
                | AIAgentActionResultType::SuggestNewConversation(
                    SuggestNewConversationResult::Cancelled,
                )
                | AIAgentActionResultType::SuggestPrompt(SuggestPromptResult::Cancelled),
        )
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) struct FormattedTextLineWrapper {
    /// The raw text with the Markdown formatting syntax stripped.
    /// This is needed for find & link/secret detection.
    stripped_text: String,
    /// Pre-extracted URL hyperlinks from this line.
    /// The AI formatted text wrapper only supports URL hyperlinks (since it's constructed via markdown).
    hyperlinks: Vec<(Range<usize>, String)>,
}

impl FormattedTextLineWrapper {
    /// Returns the raw text with the Markdown formatting syntax stripped.
    pub fn raw_text(&self) -> &str {
        &self.stripped_text
    }

    pub fn hyperlinks(&self) -> Vec<(Range<usize>, String)> {
        self.hyperlinks.clone()
    }
}

#[derive(Debug, Clone)]
pub(crate) struct FormattedTextWrapper {
    /// Private to prevent direct mutation that would desync the cached `formatted_text` Arc.
    lines: Vec<FormattedTextLineWrapper>,
    formatted_text: Arc<FormattedText>,
}

impl PartialEq for FormattedTextWrapper {
    fn eq(&self, other: &Self) -> bool {
        self.lines == other.lines
    }
}

impl Eq for FormattedTextWrapper {}

impl FormattedTextWrapper {
    pub fn lines(&self) -> &[FormattedTextLineWrapper] {
        &self.lines
    }

    /// Returns a cheap clone of the cached [`FormattedText`], avoiding a per-call deep copy.
    pub fn formatted_text_arc(&self) -> Arc<FormattedText> {
        Arc::clone(&self.formatted_text)
    }
}

impl From<FormattedText> for FormattedTextWrapper {
    fn from(value: FormattedText) -> Self {
        let formatted_text = Arc::new(value);
        let lines = formatted_text
            .lines
            .iter()
            .map(|line| FormattedTextLineWrapper {
                stripped_text: line.raw_text(),
                hyperlinks: line
                    .hyperlinks(true)
                    .into_iter()
                    .filter_map(|(r, u)| Some((r, u.url()?)))
                    .collect(),
            })
            .collect();
        Self {
            lines,
            formatted_text,
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct AgentOutputText {
    pub(crate) formatted_lines: Option<FormattedTextWrapper>,
    /// The raw text with the Markdown formatting syntax. This is needed for restoring the
    /// Markdown formatting when reopening warp.
    markdown_text: String,
}

impl AgentOutputText {
    /// Returns the original responded text with the Markdown format syntax.
    pub fn text(&self) -> &str {
        self.markdown_text.as_str()
    }

    /// Note that mutating the returned string will not automatically reparse the text and update `formatted_lines`.
    pub fn mut_text(&mut self) -> &mut String {
        &mut self.markdown_text
    }

    pub fn reparse_markdown(&mut self) {
        let parsed_result = parse_markdown(self.markdown_text.as_str());
        self.formatted_lines = parsed_result.map(|formatted| formatted.into()).ok();
    }
}

impl From<String> for AgentOutputText {
    fn from(value: String) -> Self {
        let parsed_result = parse_markdown(value.as_str());
        Self {
            formatted_lines: parsed_result.map(|formatted| formatted.into()).ok(),
            markdown_text: value,
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum AgentOutputTableRendering {
    Legacy { content: String },
    Structured { table: FormattedTable },
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct AgentOutputTable {
    pub markdown_source: String,
    pub rendering: AgentOutputTableRendering,
}

impl AgentOutputTable {
    pub fn legacy(content: String) -> Self {
        Self {
            markdown_source: content.clone(),
            rendering: AgentOutputTableRendering::Legacy { content },
        }
    }

    pub fn structured(markdown_source: String, table: FormattedTable) -> Self {
        Self {
            markdown_source,
            rendering: AgentOutputTableRendering::Structured { table },
        }
    }

    fn plain_text_for_cell(cell: &FormattedTextInline) -> String {
        cell.iter().map(|fragment| fragment.text.as_str()).collect()
    }

    fn plain_text_for_row(cells: &[FormattedTextInline]) -> String {
        cells
            .iter()
            .map(Self::plain_text_for_cell)
            .collect::<Vec<_>>()
            .join("\t")
    }

    pub fn rendered_lines(&self) -> Vec<String> {
        match &self.rendering {
            AgentOutputTableRendering::Legacy { content } => {
                content.lines().map(str::to_owned).collect()
            }
            AgentOutputTableRendering::Structured { table } => {
                let mut lines = Vec::with_capacity(1 + table.rows.len());
                lines.push(Self::plain_text_for_row(&table.headers));
                lines.extend(table.rows.iter().map(|row| Self::plain_text_for_row(row)));
                lines
            }
        }
    }

    pub fn structured_table(&self) -> Option<&FormattedTable> {
        match &self.rendering {
            AgentOutputTableRendering::Legacy { .. } => None,
            AgentOutputTableRendering::Structured { table } => Some(table),
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct AgentOutputImage {
    pub alt_text: String,
    pub source: String,
    /// Optional CommonMark image title preserved from `![alt](src "title")`.
    /// Empty titles are normalized to `None` by the shared markdown parser.
    pub title: Option<String>,
    pub markdown_source: String,
    pub layout: AgentOutputImageLayout,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct AgentOutputMermaidDiagram {
    pub source: String,
    pub markdown_source: String,
}
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum AIAgentTextSection {
    /// Plain textual output from the AI.
    PlainText { text: AgentOutputText },
    /// A snippet of code included as part of the AI's output.
    Code {
        code: String,
        language: Option<ProgrammingLanguage>,
        source: Option<CodeSource>,
    },
    /// A formatted markdown table rendered in a text block.
    Table { table: AgentOutputTable },
    /// A markdown image rendered as a visual block.
    Image { image: AgentOutputImage },
    /// A Mermaid diagram rendered as a visual block.
    MermaidDiagram { diagram: AgentOutputMermaidDiagram },
}

impl AIAgentTextSection {
    pub fn is_empty(&self) -> bool {
        match self {
            AIAgentTextSection::PlainText { text } => text.text().is_empty(),
            AIAgentTextSection::Code { code, .. } => code.is_empty(),
            AIAgentTextSection::Table { table } => table.markdown_source.is_empty(),
            AIAgentTextSection::Image { image } => image.markdown_source.is_empty(),
            AIAgentTextSection::MermaidDiagram { diagram } => diagram.markdown_source.is_empty(),
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct AIAgentText {
    pub sections: Vec<AIAgentTextSection>,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct AIAgentTodoId(String);

impl AsRef<str> for AIAgentTodoId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl From<String> for AIAgentTodoId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<AIAgentTodoId> for String {
    fn from(value: AIAgentTodoId) -> Self {
        value.0
    }
}

impl Display for AIAgentTodoId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct AIAgentTodo {
    pub id: AIAgentTodoId,
    pub title: String,
    pub description: String,
}

impl AIAgentTodo {
    pub fn new(id: AIAgentTodoId, title: String, description: String) -> Self {
        Self {
            id,
            title,
            description,
        }
    }
}

impl Display for AIAgentTodo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.id, self.title)?;
        Ok(())
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum TodoOperation {
    UpdateTodos { todos: Vec<AIAgentTodo> },
    MarkAsCompleted { completed_todos: Vec<AIAgentTodo> },
}

impl Display for TodoOperation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TodoOperation::UpdateTodos { todos } => {
                write!(f, "UpdateTodos: {} items", todos.len())
            }
            TodoOperation::MarkAsCompleted { completed_todos } => {
                write!(f, "MarkAsCompleted: {} items", completed_todos.len())
            }
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum SubagentType {
    Cli,
    Research,
    Advice,
    ComputerUse,
    Summarization,
    ConversationSearch {
        query: Option<String>,
        /// Search targets are mutually exclusive; at most one of `conversation_id` or
        /// `agent_run_id` should be populated for a single conversation search subagent.
        /// The ID of the conversation being searched.
        conversation_id: Option<String>,
        /// The ID of the agent run being searched.
        agent_run_id: Option<String>,
    },
    WarpDocumentationSearch,
    Unknown,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct SubagentCall {
    pub task_id: String,
    pub subagent_type: SubagentType,
}

impl Display for SubagentCall {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Subagent: {}", self.task_id)
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct InvokedSkill {
    pub name: String,
}

/// Data for a single received message, used for rendering in the UI.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ReceivedMessageDisplay {
    pub message_id: String,
    pub sender_agent_id: String,
    pub addresses: Vec<String>,
    pub subject: String,
    pub message_body: String,
}

impl Display for InvokedSkill {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "InvokedSkill: {}", self.name)
    }
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum AIAgentOutputMessageType {
    Text(AIAgentText),
    Reasoning {
        text: AIAgentText,
        /// How long the Agent reasoned for.
        /// Only populated when the Agent is done reasoning.
        finished_duration: Option<Duration>,
    },
    Summarization {
        /// The summarization text sections.
        text: AIAgentText,
        /// How long the Agent spent summarizing.
        /// Only populated when the summarization is done.
        finished_duration: Option<Duration>,
        summarization_type: SummarizationType,
        /// Number of tokens in the summarization.
        /// Only populated for ConversationSummary during/after summarization.
        token_count: Option<u32>,
    },
    Subagent(SubagentCall),
    Action(AIAgentAction),
    TodoOperation(TodoOperation),
    WebSearch(WebSearchStatus),
    WebFetch(WebFetchStatus),
    CommentsAddressed {
        comments: Vec<ReviewComment>,
    },
    /// Debug-only output message for staging/dev builds.
    DebugOutput {
        text: String,
    },
    /// Notification that an artifact was created (e.g. a PR).
    ArtifactCreated(ArtifactCreatedData),
    SkillInvoked(InvokedSkill),
    /// Messages received from other agent conversations.
    MessagesReceivedFromAgents {
        messages: Vec<ReceivedMessageDisplay>,
    },
    /// Lifecycle events received from other agent conversations.
    EventsFromAgents {
        event_ids: Vec<String>,
    },
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum ArtifactCreatedData {
    PullRequest {
        url: String,
        branch: String,
    },
    Screenshot {
        artifact_uid: String,
        mime_type: String,
        description: Option<String>,
    },
    File {
        artifact_uid: String,
        filepath: String,
        filename: String,
        mime_type: String,
        description: Option<String>,
        size_bytes: i64,
    },
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum SummarizationType {
    ConversationSummary,
    ToolCallResultSummary,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum WebSearchStatus {
    Searching {
        query: Option<String>,
    },
    Success {
        query: String,
        pages: Vec<(String, String)>,
    },
    Error {
        query: String,
    },
}

/// Status of a web fetch operation (fetching content from specific URLs).
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum WebFetchStatus {
    /// Currently fetching content from URLs.
    Fetching {
        /// The URLs being fetched.
        urls: Vec<String>,
    },
    /// Successfully fetched content from URLs.
    Success {
        /// The fetched pages: (url, title, success).
        pages: Vec<(String, String, bool)>,
    },
    /// Failed to fetch content.
    Error,
}

#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct MessageId(String);

impl MessageId {
    pub fn new(id: String) -> Self {
        Self(id)
    }
}

impl Deref for MessageId {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// A single output message received in an AI's response to some [`AIAgentInput`].
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct AIAgentOutputMessage {
    pub id: MessageId,
    pub message: AIAgentOutputMessageType,
    pub citations: Vec<AIAgentCitation>,
}

impl Display for AIAgentOutputMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.message {
            AIAgentOutputMessageType::Text(text)
            | AIAgentOutputMessageType::Reasoning { text, .. }
            | AIAgentOutputMessageType::Summarization { text, .. } => {
                if matches!(self.message, AIAgentOutputMessageType::Reasoning { .. }) {
                    write!(f, "LLM Reasoning: ")?;
                } else if matches!(self.message, AIAgentOutputMessageType::Summarization { .. }) {
                    write!(f, "Conversation Summary: ")?;
                }
                for (i, section) in text.sections.iter().enumerate() {
                    if i > 0 {
                        writeln!(f)?;
                    }
                    match section {
                        AIAgentTextSection::PlainText { text } => write!(f, "{}", text.text())?,
                        AIAgentTextSection::Code {
                            code,
                            language,
                            source,
                        } => {
                            write!(f, "```")?;
                            if let Some(lang) = language {
                                write!(f, "{lang}")?;
                            }
                            if let Some(CodeSource::Link {
                                path,
                                range_start,
                                range_end,
                            }) = source
                            {
                                write!(f, " path={path:?}")?;
                                if let (Some(range_start), Some(range_end)) =
                                    (range_start, range_end)
                                {
                                    write!(
                                        f,
                                        " start={} end={}",
                                        range_start.line_num, range_end.line_num
                                    )?;
                                }
                            }
                            writeln!(f)?;
                            writeln!(f, "{code}")?;
                            write!(f, "```")
                        }?,
                        AIAgentTextSection::Table { table } => {
                            { write!(f, "{}", table.markdown_source) }?
                        }
                        AIAgentTextSection::Image { image } => {
                            write!(f, "{}", image.markdown_source)?
                        }
                        AIAgentTextSection::MermaidDiagram { diagram } => {
                            write!(f, "{}", diagram.markdown_source)?
                        }
                    }
                }
            }
            AIAgentOutputMessageType::Action(action) => write!(f, "Action: {action}")?,
            AIAgentOutputMessageType::TodoOperation(todo) => write!(f, "Todo: {todo}")?,
            AIAgentOutputMessageType::Subagent(subagent) => write!(f, "Subagent: {subagent}")?,
            AIAgentOutputMessageType::WebSearch(status) => match status {
                WebSearchStatus::Searching { query } => match query {
                    Some(q) => write!(f, "Searching web for: {q}")?,
                    None => write!(f, "Searching web")?,
                },
                WebSearchStatus::Success { query, pages } => {
                    write!(f, "Searched web for: {query} ({} results)", pages.len())?
                }
                WebSearchStatus::Error { query } => write!(f, "Web search failed for: {query}")?,
            },
            AIAgentOutputMessageType::WebFetch(status) => match status {
                WebFetchStatus::Fetching { urls } => {
                    write!(f, "Fetching {} web pages...", urls.len())?
                }
                WebFetchStatus::Success { pages } => {
                    write!(f, "Fetched {} web pages", pages.len())?
                }
                WebFetchStatus::Error => write!(f, "Web fetch failed")?,
            },
            AIAgentOutputMessageType::CommentsAddressed {
                comments: comment_ids,
            } => write!(f, "Addressed {} comments", comment_ids.len())?,
            AIAgentOutputMessageType::DebugOutput { text } => write!(f, "[DEBUG] {text}")?,
            AIAgentOutputMessageType::ArtifactCreated(data) => match data {
                ArtifactCreatedData::PullRequest { url, branch } => {
                    write!(f, "Created PR: {url} (branch: {branch})")?
                }
                ArtifactCreatedData::Screenshot { artifact_uid, .. } => {
                    write!(f, "Screenshot captured (artifact: {artifact_uid})")?
                }
                ArtifactCreatedData::File {
                    artifact_uid,
                    filepath,
                    ..
                } => write!(
                    f,
                    "File artifact uploaded: {filepath} (artifact: {artifact_uid})"
                )?,
            },
            AIAgentOutputMessageType::SkillInvoked(invoked_skill) => {
                write!(f, "Skill Invoked: {}", invoked_skill.name)?
            }
            AIAgentOutputMessageType::MessagesReceivedFromAgents { messages } => {
                write!(f, "Received {} messages", messages.len())?
            }
            AIAgentOutputMessageType::EventsFromAgents { event_ids } => {
                write!(f, "Received {} agent events", event_ids.len())?
            }
        }

        if !self.citations.is_empty() {
            writeln!(f)?;
            writeln!(f, "Citations:")?;
            for citation in &self.citations {
                writeln!(f, "  - {citation}")?
            }
        }
        Ok(())
    }
}

impl AIAgentOutputMessage {
    pub fn action(id: MessageId, action: AIAgentAction) -> Self {
        Self {
            id,
            message: AIAgentOutputMessageType::Action(action),
            citations: vec![],
        }
    }

    pub fn text(id: MessageId, text: AIAgentText) -> Self {
        Self {
            id,
            message: AIAgentOutputMessageType::Text(text),
            citations: vec![],
        }
    }

    pub fn subagent(id: MessageId, subagent: SubagentCall) -> Self {
        Self {
            id,
            message: AIAgentOutputMessageType::Subagent(subagent),
            citations: vec![],
        }
    }

    pub fn reasoning(id: MessageId, text: AIAgentText, duration: Option<Duration>) -> Self {
        Self {
            id,
            message: AIAgentOutputMessageType::Reasoning {
                text,
                finished_duration: duration,
            },
            citations: vec![],
        }
    }

    pub fn todo_operation(id: MessageId, operation: TodoOperation) -> Self {
        Self {
            id,
            message: AIAgentOutputMessageType::TodoOperation(operation),
            citations: vec![],
        }
    }

    pub fn comments_addressed(id: MessageId, comments: Vec<ReviewComment>) -> Self {
        Self {
            id,
            message: AIAgentOutputMessageType::CommentsAddressed { comments },
            citations: vec![],
        }
    }

    pub fn debug_output(id: MessageId, text: String) -> Self {
        Self {
            id,
            message: AIAgentOutputMessageType::DebugOutput { text },
            citations: vec![],
        }
    }

    pub fn summarization(
        id: MessageId,
        text: AIAgentText,
        duration: Option<Duration>,
        summarization_type: SummarizationType,
        token_count: Option<u32>,
    ) -> Self {
        Self {
            id,
            message: AIAgentOutputMessageType::Summarization {
                text,
                finished_duration: duration,
                summarization_type,
                token_count,
            },
            citations: vec![],
        }
    }

    pub fn web_search(id: MessageId, status: WebSearchStatus) -> Self {
        Self {
            id,
            message: AIAgentOutputMessageType::WebSearch(status),
            citations: vec![],
        }
    }

    pub fn web_fetch(id: MessageId, status: WebFetchStatus) -> Self {
        Self {
            id,
            message: AIAgentOutputMessageType::WebFetch(status),
            citations: vec![],
        }
    }

    pub fn artifact_created(id: MessageId, data: ArtifactCreatedData) -> Self {
        Self {
            id,
            message: AIAgentOutputMessageType::ArtifactCreated(data),
            citations: vec![],
        }
    }

    pub fn with_citations(self, citations: Vec<AIAgentCitation>) -> Self {
        Self { citations, ..self }
    }

    pub fn skill_invoked(id: MessageId, invoked_skill: InvokedSkill) -> Self {
        Self {
            id,
            message: AIAgentOutputMessageType::SkillInvoked(invoked_skill),
            citations: vec![],
        }
    }

    pub fn messages_received_from_agents(
        id: MessageId,
        messages: Vec<ReceivedMessageDisplay>,
    ) -> Self {
        Self {
            id,
            message: AIAgentOutputMessageType::MessagesReceivedFromAgents { messages },
            citations: vec![],
        }
    }

    pub fn events_from_agents(id: MessageId, event_ids: Vec<String>) -> Self {
        Self {
            id,
            message: AIAgentOutputMessageType::EventsFromAgents { event_ids },
            citations: vec![],
        }
    }
}

// Information about what MCP capabilities the client has, to
// be provided as context for Agent Mode requests.
#[derive(Debug, Clone)]
#[cfg_attr(target_family = "wasm", allow(dead_code))]
pub struct MCPContext {
    // Old flat structure (deprecated but kept for backward compatibility)
    #[deprecated]
    pub resources: Vec<rmcp::model::Resource>,
    #[deprecated]
    pub tools: Vec<rmcp::model::Tool>,
    // New grouped structure
    pub servers: Vec<MCPServer>,
}

#[derive(Debug, Clone)]
#[cfg_attr(target_family = "wasm", allow(dead_code))]
pub struct MCPServer {
    pub id: String,
    pub name: String,
    pub description: String,
    pub resources: Vec<rmcp::model::Resource>,
    pub tools: Vec<rmcp::model::Tool>,
}

/// Contains context that may be attached to a user query.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AIAgentContext {
    Directory {
        pwd: Option<String>,
        home_dir: Option<String>,
        are_file_symbols_indexed: bool,
    },

    /// Text selected via the cursor within the block list.
    SelectedText(String),

    /// Information about the execution environment (OS, shell type and version) is included in the
    /// query.
    ExecutionEnvironment(WarpAiExecutionContext),

    /// The current date and time.
    CurrentTime {
        current_time: DateTime<Local>,
    },

    /// An image attached to the query.
    Image(ImageContext),

    /// Indexed codebase possibly relevant to the query.
    Codebase {
        /// Absolute path to the indexed codebase.
        path: String,
        /// Repository name.
        name: String,
    },

    ProjectRules {
        root_path: String,
        active_rules: Vec<FileContext>,
        additional_rule_paths: Vec<String>,
    },

    File(FileContext),

    Git {
        head: String,
        branch: Option<String>,
    },

    /// Information about the git repository in the current working directory.
    Repository {
        /// The repository name (e.g. "warp-internal").
        name: String,
        /// The repository owner/organization (e.g. "warpdotdev"), if determinable from the remote URL.
        owner: Option<String>,
    },

    /// Information about the GitHub pull request associated with the current branch.
    PullRequest {
        /// The pull request number.
        #[serde(default, deserialize_with = "deserialize_pull_request_number")]
        number: i32,
        /// The pull request state (for example, `OPEN`, `MERGED`, or `CLOSED`).
        #[serde(default)]
        state: String,
        /// Whether the pull request is marked as draft.
        #[serde(default)]
        draft: bool,
        /// The pull request's base branch.
        #[serde(default)]
        base_branch: String,
    },

    /// List of available skills is provided to the agent during initialization
    /// or when updated.
    Skills {
        skills: Vec<SkillDescriptor>,
    },

    #[serde(untagged)]
    Block(Box<BlockContext>),
}

fn deserialize_pull_request_number<'de, D>(deserializer: D) -> Result<i32, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    match value {
        serde_json::Value::Null => Ok(0),
        serde_json::Value::String(s) => {
            if !s.chars().all(|c| c.is_ascii_digit()) {
                return Ok(0);
            }
            Ok(s.parse()
                .ok()
                .filter(|number| *number > 0)
                .unwrap_or_default())
        }
        serde_json::Value::Number(n) => {
            let Some(number) = n.as_i64() else {
                return Ok(0);
            };
            Ok(i32::try_from(number)
                .ok()
                .filter(|number| *number > 0)
                .unwrap_or_default())
        }
        value => Err(serde::de::Error::custom(format!(
            "expected string or number for pull request number, got {value}"
        ))),
    }
}

#[derive(Clone, Serialize, Deserialize, Eq, PartialEq)]
pub struct ImageContext {
    /// Base64-encoded image data.
    pub data: String,

    /// MIME type of the media content (e.g., "image/jpeg", "image/png")
    pub mime_type: String,

    pub file_name: String,

    /// Whether this image was exported from Figma, detected via
    /// the `Software: Figma` PNG metadata field.
    #[serde(default)]
    pub is_figma: bool,
}

impl std::fmt::Debug for ImageContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // We log dispatching typed actions (with `ImageContext` as an argument) and we don't want
        // to log any UGC in prod.
        f.debug_struct("ImageContext")
            .field("data", &"REDACTED_B64_IMAGE_DATA_UGC")
            .field("mime_type", &self.mime_type)
            .field("file_name", &"REDACTED_FILE_NAME_UGC")
            .finish()
    }
}

/// Source of a document content attachment.
/// Used to identify user-attached plans to track in the UI.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DocumentContentAttachmentSource {
    UserAttached,
    PlanEdited,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AIAgentAttachment {
    PlainText(String),
    DocumentContent {
        document_id: String,
        content: String,
        source: DocumentContentAttachmentSource,
        line_range: Option<Range<LineCount>>,
    },
    DriveObject {
        /// The UID of the drive object.
        uid: String,
        /// The payload of the drive object (e.g., workflow content).
        payload: Option<DriveObjectPayload>,
    },
    DiffHunk {
        file_path: String,
        line_range: Range<LineCount>,
        diff_content: String,
        lines_added: u32,
        lines_removed: u32,
        current: Option<CurrentHead>,
        base: DiffBase,
    },
    DiffSet {
        /// Map from file path to list of diff hunks for that file
        file_diffs: HashMap<String, Vec<DiffSetHunk>>,
        /// Git branch information for the diff
        current: Option<CurrentHead>,
        base: DiffBase,
    },
    /// Reference to a file on the VM filesystem (e.g., downloaded attachments from cloud mode).
    /// The server uses this to provide the file as an inline reference to the LLM.
    FilePathReference {
        /// The UUID of the attachment (from warp-server's presigned URL flow).
        file_id: String,
        /// The original filename.
        file_name: String,
        /// The full resolved path on disk where the file was downloaded.
        file_path: String,
    },
    #[serde(untagged)]
    Block(BlockContext),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CurrentHead {
    BranchName(String),
    HeadlessCommitSha(String),
}

impl CurrentHead {
    pub fn title(&self) -> String {
        match self {
            CurrentHead::BranchName(name) => name.clone(),
            CurrentHead::HeadlessCommitSha(sha) => {
                let short = sha.chars().take(7).collect::<String>();
                format!("Commit {short}")
            }
        }
    }
}

impl From<CurrentHead> for warp_multi_agent_api::CurrentRef {
    fn from(value: CurrentHead) -> Self {
        Self {
            r#ref: Some(match value {
                CurrentHead::BranchName(name) => {
                    warp_multi_agent_api::current_ref::Ref::BranchName(name)
                }
                CurrentHead::HeadlessCommitSha(sha) => {
                    warp_multi_agent_api::current_ref::Ref::HeadlessCommitSha(sha)
                }
            }),
        }
    }
}

impl From<CurrentHead> for diff_hunk_api::Current {
    fn from(value: CurrentHead) -> Self {
        match value {
            CurrentHead::BranchName(name) => diff_hunk_api::Current::CurrentBranchName(name),
            CurrentHead::HeadlessCommitSha(sha) => {
                diff_hunk_api::Current::CurrentHeadlessCommitSha(sha)
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiffBase {
    BranchName(String),
    HeadlessCommitSha(String),
    UncommittedChanges,
}

impl From<DiffBase> for warp_multi_agent_api::BaseRef {
    fn from(value: DiffBase) -> Self {
        Self {
            r#ref: Some(match value {
                DiffBase::BranchName(name) => warp_multi_agent_api::base_ref::Ref::BranchName(name),
                DiffBase::HeadlessCommitSha(sha) => {
                    warp_multi_agent_api::base_ref::Ref::HeadlessCommitSha(sha)
                }
                DiffBase::UncommittedChanges => {
                    warp_multi_agent_api::base_ref::Ref::UncommittedChanges(())
                }
            }),
        }
    }
}

impl From<DiffBase> for diff_hunk_api::Base {
    fn from(value: DiffBase) -> Self {
        match value {
            DiffBase::BranchName(branch_name) => diff_hunk_api::Base::BaseBranchName(branch_name),
            DiffBase::HeadlessCommitSha(sha) => diff_hunk_api::Base::BaseHeadlessCommitSha(sha),
            DiffBase::UncommittedChanges =>
            {
                #[warn(clippy::unit_arg)]
                diff_hunk_api::Base::UncommittedChanges(())
            }
        }
    }
}

/// A simplified diff hunk for use in DiffSet attachments
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffSetHunk {
    pub line_range: Range<LineCount>,
    pub diff_content: String,
    pub lines_added: u32,
    pub lines_removed: u32,
}

impl DiffSetHunk {
    pub fn convert_to_api(self, file_path: String) -> warp_multi_agent_api::diff_set::DiffHunk {
        warp_multi_agent_api::diff_set::DiffHunk {
            file_path,
            line_range: Some(warp_multi_agent_api::FileContentLineRange {
                start: self.line_range.start.as_usize() as u32,
                end: self.line_range.end.as_usize() as u32,
            }),
            diff_content: self.diff_content,
            lines_added: self.lines_added,
            lines_removed: self.lines_removed,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DriveObjectPayload {
    Workflow {
        name: String,
        description: String,
        command: String,
    },
    Notebook {
        title: String,
        content: String,
    },
    GenericStringObject {
        payload: String,
        object_type: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StaticQueryType {
    Install,
    Code,
    Deploy,
    SomethingElse,
    EvaluationSuite,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[allow(clippy::enum_variant_names)]
pub enum EntrypointType {
    PromptSuggestion {
        is_static: bool,
        is_coding: bool,
    },
    ZeroStateAgentModePromptSuggestion,
    InitProjectRules,
    TriggerPassiveSuggestion {
        trigger: Option<PassiveSuggestionTriggerType>,
    },
    UserInitiated,
    AgentInitiated,
    SharedSession,
    CloneRepository,
    ResumeConversation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[allow(clippy::enum_variant_names)]
pub enum PassiveSuggestionTriggerType {
    /// Used for unit test generation.
    FilesChanged,
    /// Used for unit test generation.
    CommandRun,

    ShellCommandCompleted,
    AgentResponseCompleted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ShellCommandCompletedTrigger {
    // We heap-allocate this because it's large and bloats the size of the
    // `ShellCommandCompleted` enum variant relative to other variants.
    pub executed_shell_command: Box<BlockContext>,
    pub relevant_files: Vec<FileContext>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[allow(clippy::enum_variant_names)]
pub enum PassiveSuggestionTrigger {
    FilesChanged,
    CommandRun,
    ShellCommandCompleted(ShellCommandCompletedTrigger),
    AgentResponseCompleted { exchange_id: AIAgentExchangeId },
}

impl From<&PassiveSuggestionTrigger> for PassiveSuggestionTriggerType {
    fn from(value: &PassiveSuggestionTrigger) -> Self {
        match value {
            PassiveSuggestionTrigger::FilesChanged => PassiveSuggestionTriggerType::FilesChanged,
            PassiveSuggestionTrigger::CommandRun => PassiveSuggestionTriggerType::CommandRun,
            PassiveSuggestionTrigger::ShellCommandCompleted(_) => {
                PassiveSuggestionTriggerType::ShellCommandCompleted
            }
            PassiveSuggestionTrigger::AgentResponseCompleted { .. } => {
                PassiveSuggestionTriggerType::AgentResponseCompleted
            }
        }
    }
}

impl PassiveSuggestionTrigger {
    /// Returns the block ID that triggered this passive suggestion
    /// iff the trigger type was [Self::ShellCommandCompleted].
    pub fn block_id(&self) -> Option<BlockId> {
        match self {
            Self::ShellCommandCompleted(c) => Some(c.executed_shell_command.id.clone()),
            _ => None,
        }
    }

    /// Returns the exchange ID that triggered this passive suggestion
    /// iff the trigger type was [Self::AgentResponseCompleted].
    pub fn exchange_id(&self) -> Option<AIAgentExchangeId> {
        match self {
            Self::AgentResponseCompleted { exchange_id } => Some(*exchange_id),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum UserQueryMode {
    #[default]
    Normal,
    Plan,
    Orchestrate,
}

pub fn extract_user_query_mode(query: String) -> (String, UserQueryMode) {
    if let Some(query) = commands::strip_command_prefix(&query, commands::PLAN_NAME) {
        (query, UserQueryMode::Plan)
    } else if let Some(query) = commands::strip_command_prefix(&query, commands::ORCHESTRATE_NAME) {
        (query, UserQueryMode::Orchestrate)
    } else {
        (query, UserQueryMode::Normal)
    }
}

/// Reconstructs the display form of a user query that has been stripped via
/// [`extract_user_query_mode`], by re-prepending the slash-command prefix
/// associated with [`UserQueryMode`].
///
/// This is the inverse of [`extract_user_query_mode`] and the canonical way
/// for UI to render a stored `(mode, query)` pair so the displayed prompt
/// always matches what the user originally submitted.
pub fn display_user_query_with_mode(mode: UserQueryMode, query: &str) -> String {
    match mode {
        UserQueryMode::Normal => query.to_owned(),
        UserQueryMode::Plan => format!("{} {query}", commands::PLAN.name),
        UserQueryMode::Orchestrate => format!("{} {query}", commands::ORCHESTRATE.name),
    }
}

// TODO(zachbai): Refactor this to consolidate with `LongRunningCommandSnapshot` and `Snapshot`
// variants of `ReadShellCommandOutputResult` and `WriteToLongRunningShellCommandResult`.
#[derive(Clone, Debug, PartialEq)]
pub struct RunningCommand {
    pub command: String,
    pub block_id: BlockId,
    pub grid_contents: String,
    pub cursor: String,
    pub requested_command_id: Option<AIAgentActionId>,
    pub is_alt_screen_active: bool,
}

/// A single search/replace diff entry for a passive code suggestion.
#[derive(Clone, Debug, PartialEq)]
pub struct PassiveCodeDiffEntry {
    pub file_path: String,
    pub search: String,
    pub replace: String,
}

/// The outcome of a passive suggestion that the user interacted with.
#[derive(Clone, Debug, PartialEq)]
pub enum PassiveSuggestionResultType {
    Prompt {
        prompt: String,
    },
    CodeDiff {
        diffs: Vec<PassiveCodeDiffEntry>,
        summary: String,
        accepted: bool,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum AIAgentInput {
    /// A user's query to the AI.
    UserQuery {
        query: String,
        context: Arc<[AIAgentContext]>,
        static_query_type: Option<StaticQueryType>,
        referenced_attachments: HashMap<String, AIAgentAttachment>,
        user_query_mode: UserQueryMode,
        running_command: Option<RunningCommand>,
        intended_agent: Option<AgentType>,
    },

    AutoCodeDiffQuery {
        query: String,
        context: Arc<[AIAgentContext]>,
    },

    ResumeConversation {
        context: Arc<[AIAgentContext]>,
    },

    InitProjectRules {
        context: Arc<[AIAgentContext]>,
        display_query: Option<String>,
    },

    CreateEnvironment {
        context: Arc<[AIAgentContext]>,
        display_query: Option<String>,
        repo_paths: Vec<String>,
    },

    TriggerPassiveSuggestion {
        context: Arc<[AIAgentContext]>,
        attachments: Vec<AIAgentAttachment>,
        trigger: PassiveSuggestionTrigger,
    },

    CreateNewProject {
        query: String,
        context: Arc<[AIAgentContext]>,
    },

    CloneRepository {
        clone_repo_url: CloneRepositoryURL,
        context: Arc<[AIAgentContext]>,
    },

    /// A batch of inline code review comments for the agent to address.
    CodeReview {
        context: Arc<[AIAgentContext]>,
        review_comments: AgentReviewCommentBatch,
    },

    FetchReviewComments {
        repo_path: String,
        context: Arc<[AIAgentContext]>,
    },

    SummarizeConversation {
        prompt: Option<String>,
        context: Arc<[AIAgentContext]>,
    },

    /// Invoke a skill. The skill content is passed as instructions to the agent.
    InvokeSkill {
        context: Arc<[AIAgentContext]>,
        skill: ParsedSkill,
        user_query: Option<InvokeSkillUserQuery>,
    },

    /// Start a conversation using the prompt stored for an ambient agent run.
    /// The server resolves the prompt from the run's latest known prompt.
    /// If runtime_skill is provided, the server will create an InvokeSkill message. The skill
    /// instructions are sent to the LLM but not displayed in the UI query bubble.
    StartFromAmbientRunPrompt {
        ambient_run_id: String,
        context: Arc<[AIAgentContext]>,
        /// Optional skill to use as base context (content hidden from user in UI).
        runtime_skill: Option<ai::skills::ParsedSkill>,
        /// Optional directory path where the client downloaded task attachments.
        /// Passed to the server so it can construct correct file paths for the LLM.
        attachments_dir: Option<String>,
    },

    /// The result of an `AIAgentAction`, relayed back to the LLM for it to continue answering a
    /// user query.
    ActionResult {
        result: AIAgentActionResult,
        context: Arc<[AIAgentContext]>,
    },

    /// Messages received from other agent conversations via the message bus.
    MessagesReceivedFromAgents {
        messages: Vec<ReceivedMessageInput>,
    },
    /// Events received from other agent conversations.
    EventsFromAgents {
        events: Vec<AgentEvent>,
    },

    /// The result of a passive suggestion that should be
    /// handled in the active conversation.
    PassiveSuggestionResult {
        trigger: Option<PassiveSuggestionTrigger>,
        suggestion: PassiveSuggestionResultType,
        context: Arc<[AIAgentContext]>,
    },

    /// Piggybacked orchestration config update from the plan card.
    /// Sent on the next outbound request after the user edits the
    /// config block or toggles approval.
    OrchestrationConfigUpdate {
        plan_id: String,
        config: OrchestrationConfig,
        status: OrchestrationConfigStatus,
    },
}

/// Data for a single message received by an agent from another agent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReceivedMessageInput {
    pub message_id: String,
    pub sender_agent_id: String,
    pub addresses: Vec<String>,
    pub subject: String,
    pub message_body: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AgentReviewCommentBatch {
    /// The review comments in this batch. Uses `code_review::comments::ReviewComment`
    /// because it contains full target information needed for API conversion and UI rendering.
    pub comments: Vec<CodeReviewComment>,
    /// All diff hunks that have comments in this batch attached to them, grouped by file name.
    pub diff_set: HashMap<String, Vec<DiffSetHunk>>,
}

impl AgentReviewCommentBatch {
    pub fn review_comments(&self) -> ReviewCommentBatch {
        ReviewCommentBatch::from_comments(self.comments.clone())
    }
}

/// A simple struct that holds a URL to be used for the CloneRepository input.
///
/// Needed because we want to display a query that's more than just the URL to the user
/// and the code is setup such that the query string must be preallocated.
#[derive(Clone, Debug, PartialEq)]
pub struct CloneRepositoryURL {
    /// The query displayed to the user when a user clones a repository.
    query: String,

    /// The URL of the repository to clone.
    url: String,
}

impl CloneRepositoryURL {
    pub fn new(url: String) -> Self {
        Self {
            query: format!("Clone {url}"),
            url,
        }
    }

    pub fn into_url(self) -> String {
        self.url
    }
}

impl Display for AIAgentInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UserQuery { .. } => {
                write!(f, "UserQuery: {}", self.display_query().unwrap_or_default())
            }
            Self::AutoCodeDiffQuery { query, .. } => {
                write!(f, "AutoCodeDiffQuery: {query}")
            }
            Self::ActionResult { result, .. } => write!(f, "ActionResult: {result}"),
            Self::ResumeConversation { .. } => write!(f, "ResumeConversation"),
            Self::InitProjectRules { .. } => write!(f, "InitProjectRules"),
            Self::CreateEnvironment { .. } => write!(f, "CreateEnvironment"),
            Self::TriggerPassiveSuggestion { .. } => write!(f, "TriggerSuggestPrompt"),
            Self::CreateNewProject { .. } => write!(f, "CreateNewProject"),
            Self::CloneRepository { .. } => write!(f, "CloneRepository"),
            Self::CodeReview { .. } => write!(f, "CodeReview"),
            Self::FetchReviewComments { .. } => write!(f, "FetchReviewComments"),
            Self::SummarizeConversation { .. } => write!(f, "SummarizeConversation"),
            Self::InvokeSkill {
                skill, user_query, ..
            } => {
                if let Some(user_query) = user_query {
                    if user_query.query.is_empty() {
                        write!(f, "InvokeSkill: {}", skill.name)
                    } else {
                        write!(f, "InvokeSkill: {} {}", skill.name, user_query.query)
                    }
                } else {
                    write!(f, "InvokeSkill: {}", skill.name)
                }
            }
            Self::StartFromAmbientRunPrompt { .. } => write!(f, "StartFromAmbientRunPrompt"),
            Self::MessagesReceivedFromAgents { messages } => {
                write!(f, "MessagesReceivedFromAgents({} messages)", messages.len())
            }
            Self::EventsFromAgents { events } => {
                write!(f, "EventsFromAgents({} events)", events.len())
            }
            Self::PassiveSuggestionResult { .. } => write!(f, "PassiveSuggestionResult"),
            Self::OrchestrationConfigUpdate { .. } => write!(f, "OrchestrationConfigUpdate"),
        }
    }
}

impl AIAgentInput {
    /// Display text for any input that surfaces a prompt-like query in the UI
    /// (typed queries, slash commands, skill invocations, etc.). Unlike
    /// [`Self::is_user_query`], which strictly matches the `UserQuery` variant,
    /// this returns `Some` for several input variants.
    pub fn display_query(&self) -> Option<String> {
        match self {
            Self::UserQuery {
                query,
                user_query_mode,
                ..
            } => Some(display_user_query_with_mode(*user_query_mode, query)),
            Self::CreateNewProject { query, .. } => Some(query.clone()),
            Self::CloneRepository {
                clone_repo_url: url,
                ..
            } => Some(url.query.clone()),
            Self::InitProjectRules { display_query, .. }
            | Self::CreateEnvironment { display_query, .. } => display_query.clone(),
            Self::CodeReview { .. } => Some("Address these comments".to_string()),
            Self::FetchReviewComments { .. } => Some(commands::PR_COMMENTS.name.to_string()),
            Self::InvokeSkill {
                skill, user_query, ..
            } => {
                if let Some(user_query) = user_query {
                    if user_query.query.is_empty() {
                        Some(format!("/{}", skill.name))
                    } else {
                        Some(format!("/{} {}", skill.name, user_query.query))
                    }
                } else {
                    Some(format!("/{}", skill.name))
                }
            }
            Self::ActionResult {
                result:
                    AIAgentActionResult {
                        result:
                            AIAgentActionResultType::SuggestPrompt(SuggestPromptResult::Accepted {
                                query,
                            }),
                        ..
                    },
                ..
            } => Some(query.clone()),
            Self::PassiveSuggestionResult {
                suggestion: PassiveSuggestionResultType::Prompt { prompt },
                ..
            } => Some(prompt.clone()),
            Self::AutoCodeDiffQuery { .. }
            | Self::ActionResult { .. }
            | Self::TriggerPassiveSuggestion { .. }
            | Self::ResumeConversation { .. }
            | Self::SummarizeConversation { .. }
            | Self::StartFromAmbientRunPrompt { .. }
            | Self::MessagesReceivedFromAgents { .. }
            | Self::EventsFromAgents { .. }
            | Self::PassiveSuggestionResult { .. }
            | Self::OrchestrationConfigUpdate { .. } => None,
        }
    }

    /// Returns the user query text as it should be displayed in the UI.
    /// This includes the "/agent" prefix for the initial conversation query.
    pub fn display_user_query(
        &self,
        initial_conversation_query: Option<&String>,
    ) -> Option<String> {
        let mut query = self.display_query()?;
        if self
            .user_query_mode()
            .is_none_or(|mode| matches!(mode, UserQueryMode::Normal))
            && Some(&query) == initial_conversation_query
            && !self.has_custom_display_query()
        {
            query = format!("/agent {query}");
        }
        Some(query)
    }

    pub fn user_query_mode(&self) -> Option<UserQueryMode> {
        match self {
            AIAgentInput::UserQuery {
                user_query_mode, ..
            } => Some(*user_query_mode),
            _ => None,
        }
    }

    pub fn action_result(&self) -> Option<&AIAgentActionResult> {
        match self {
            Self::ActionResult { result, .. } => Some(result),
            _ => None,
        }
    }

    pub fn auto_code_diff_query(&self) -> Option<&str> {
        let Self::AutoCodeDiffQuery { query, .. } = self else {
            return None;
        };
        Some(query.as_str())
    }

    pub fn passive_suggestion_trigger(&self) -> Option<&PassiveSuggestionTrigger> {
        match self {
            AIAgentInput::TriggerPassiveSuggestion { trigger, .. } => Some(trigger),
            _ => None,
        }
    }

    pub fn is_passive_suggestion_trigger(&self) -> bool {
        matches!(self, AIAgentInput::TriggerPassiveSuggestion { .. })
    }

    pub fn is_user_query(&self) -> bool {
        matches!(self, AIAgentInput::UserQuery { .. })
    }

    pub fn prompt_suggestion_result(&self) -> Option<&String> {
        if let Some(AIAgentActionResult {
            result: AIAgentActionResultType::SuggestPrompt(SuggestPromptResult::Accepted { query }),
            ..
        }) = self.action_result()
        {
            Some(query)
        } else {
            None
        }
    }

    pub fn is_passive_request(&self) -> bool {
        matches!(
            self,
            AIAgentInput::AutoCodeDiffQuery { .. } | AIAgentInput::TriggerPassiveSuggestion { .. }
        )
    }

    pub fn context(&self) -> Option<&[AIAgentContext]> {
        match self {
            Self::UserQuery { context, .. }
            | Self::ActionResult { context, .. }
            | Self::AutoCodeDiffQuery { context, .. }
            | Self::ResumeConversation { context, .. }
            | Self::InitProjectRules { context, .. }
            | Self::CreateEnvironment { context, .. }
            | Self::TriggerPassiveSuggestion { context, .. }
            | Self::CreateNewProject { context, .. }
            | Self::CloneRepository { context, .. }
            | Self::CodeReview { context, .. }
            | Self::FetchReviewComments { context, .. }
            | Self::InvokeSkill { context, .. }
            | Self::StartFromAmbientRunPrompt { context, .. }
            | Self::PassiveSuggestionResult { context, .. } => Some(context),
            Self::SummarizeConversation { context, .. } => Some(context),
            Self::MessagesReceivedFromAgents { .. }
            | Self::EventsFromAgents { .. }
            | Self::OrchestrationConfigUpdate { .. } => None,
        }
    }

    /// Returns all of the attachments for the given input,
    /// converting any blocks blocks attached in the context into the correct type of attachment.
    pub fn attachments(&self) -> Option<Vec<AIAgentAttachment>> {
        match self {
            Self::UserQuery {
                referenced_attachments,
                ..
            } => {
                let res: Vec<AIAgentAttachment> =
                    referenced_attachments.values().cloned().collect();
                Some(res)
            }
            Self::TriggerPassiveSuggestion { attachments, .. } => Some(attachments.clone()),
            Self::ActionResult { .. }
            | Self::AutoCodeDiffQuery { .. }
            | Self::ResumeConversation { .. }
            | Self::InitProjectRules { .. }
            | Self::CreateEnvironment { .. }
            | Self::CreateNewProject { .. }
            | Self::CloneRepository { .. }
            | Self::CodeReview { .. }
            | Self::FetchReviewComments { .. }
            | Self::SummarizeConversation { .. }
            | Self::InvokeSkill { .. }
            | Self::StartFromAmbientRunPrompt { .. }
            | Self::MessagesReceivedFromAgents { .. }
            | Self::EventsFromAgents { .. }
            | Self::PassiveSuggestionResult { .. }
            | Self::OrchestrationConfigUpdate { .. } => None,
        }
    }

    pub fn is_auto_code_diff_query(&self) -> bool {
        matches!(self, AIAgentInput::AutoCodeDiffQuery { .. })
    }

    /// Returns true if this input type provides its own display query that should be preserved
    /// without prepending "/agent".
    pub fn has_custom_display_query(&self) -> bool {
        matches!(
            self,
            AIAgentInput::InitProjectRules { .. }
                | AIAgentInput::CreateEnvironment { .. }
                | AIAgentInput::FetchReviewComments { .. }
                | AIAgentInput::InvokeSkill { .. }
        )
    }
}

/// A globally unique ID for an `AIAgentExchange`.
#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct AIAgentExchangeId(Uuid);

impl Display for AIAgentExchangeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl AIAgentExchangeId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for AIAgentExchangeId {
    fn default() -> Self {
        Self::new()
    }
}

impl TryFrom<String> for AIAgentExchangeId {
    type Error = anyhow::Error;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Ok(Self(Uuid::try_parse(&value)?))
    }
}

/// Represents a single user input/AI output pair. Each exchange corresponds to a request to an AI
/// backend model and its response.
#[derive(Debug, Clone)]
pub struct AIAgentExchange {
    /// Unique ID for the exchange.
    pub id: AIAgentExchangeId,

    /// The input originating from the user.
    pub input: Vec<AIAgentInput>,

    /// The status of the output stream. Updated during the course of the exchange.
    pub output_status: AIAgentOutputStatus,

    /// The ids for all messages added to the task in this exchange.
    pub added_message_ids: HashSet<MessageId>,

    /// The time the input was sent.
    pub start_time: DateTime<Local>,

    /// The time the exchange's output finished streaming, if known.
    pub finish_time: Option<DateTime<Local>>,

    /// Time to first token for this exchange in milliseconds.
    pub time_to_first_token_ms: Option<i64>,

    // TODO(CORE-3546): add shell launch data when the input was submitted.
    /// The current working directory when the input was submitted.
    pub working_directory: Option<String>,

    /// The model to which the request was sent.
    pub model_id: LLMId,

    // The request count for the exchange.
    pub request_cost: Option<RequestCost>,

    /// The coding model to which the request was sent.
    pub coding_model_id: LLMId,

    /// The CLI agent model to which the request was sent.
    pub cli_agent_model_id: LLMId,

    /// The computer use model to which the request was sent.
    pub computer_use_model_id: LLMId,

    /// The participant who initiated this exchange (for shared sessions)
    /// For non-shared sessions, we just leave this as None.
    pub response_initiator: Option<ParticipantId>,
}

impl AIAgentExchange {
    /// Format the user input part of this exchange for copying to clipboard.
    /// We don't copy tool call results.
    pub fn format_input_for_copy(&self) -> String {
        let user_queries: Vec<String> = self
            .input
            .iter()
            .filter_map(|input| input.display_query())
            .collect();
        user_queries.join("\n")
    }

    /// Format the output part of this exchange for copying to clipboard.
    pub fn format_output_for_copy(
        &self,
        action_model: Option<&crate::ai::blocklist::BlocklistAIActionModel>,
    ) -> String {
        match self.output_status.output() {
            Some(output) => output.get().format_for_copy(action_model),
            None => String::new(),
        }
    }

    /// Format the entire exchange (both input and output) for copying to clipboard.
    /// Always adds USER: and AGENT: labels.
    /// If `skip_agent_label` is true, skips the AGENT: label (for consecutive agent outputs).
    pub fn format_for_copy(
        &self,
        action_model: Option<&crate::ai::blocklist::BlocklistAIActionModel>,
    ) -> String {
        let input_text = self.format_input_for_copy();
        let output_text = self.format_output_for_copy(action_model);
        let has_user_input = !input_text.is_empty();
        let has_agent_output = !output_text.is_empty();

        if !has_user_input && !has_agent_output {
            return String::new();
        }

        let mut parts = Vec::new();

        if has_user_input {
            parts.push(format!("USER:\n{input_text}"));
        }

        if has_agent_output {
            if has_user_input {
                parts.push(format!("AGENT:\n{output_text}"));
            } else {
                parts.push(output_text);
            }
        }

        parts.join("\n\n")
    }

    pub fn has_user_query(&self) -> bool {
        self.input
            .iter()
            .any(|input| input.display_query().is_some())
    }

    /// True when this exchange finished in an error that will not be automatically resumed.
    ///
    /// These are terminal, non-recoverable failures (e.g. a 403 fraud block or other
    /// user-originating 4xx). Their user query never reaches durable server history, so it must
    /// be replayed on the next request to preserve conversation context. Recoverable errors are
    /// excluded because the existing auto-resume path already restores their context.
    pub fn errored_without_resume(&self) -> bool {
        matches!(
            &self.output_status,
            AIAgentOutputStatus::Finished {
                finished_output: FinishedAIAgentOutput::Error { error, .. },
            } if !error.will_attempt_resume()
        )
    }

    pub fn has_accepted_file_edit(&self) -> bool {
        self.input.iter().any(|input| {
            matches!(
                input.action_result(),
                Some(AIAgentActionResult {
                    result: AIAgentActionResultType::RequestFileEdits(
                        RequestFileEditsResult::Success { .. }
                    ),
                    ..
                })
            )
        })
    }

    pub fn has_passive_request(&self) -> bool {
        self.input.iter().any(|input| input.is_passive_request())
    }

    pub fn has_passive_code_diff(&self) -> bool {
        self.input
            .iter()
            .any(|input| input.auto_code_diff_query().is_some())
            || (FeatureFlag::PromptSuggestionsViaMAA.is_enabled()
                && self.has_passive_request()
                && self.output_status.output().is_some_and(|output| {
                    output.get().actions().any(|action| {
                        matches!(action.action, AIAgentActionType::RequestFileEdits { .. })
                    })
                }))
    }

    pub fn passive_suggestion_trigger(&self) -> Option<&PassiveSuggestionTrigger> {
        self.input
            .iter()
            .find_map(|input| input.passive_suggestion_trigger())
    }

    pub fn duration(&self) -> Option<TimeDelta> {
        self.finish_time
            .map(|finish_time| finish_time.signed_duration_since(self.start_time))
    }
}

/// Request-level metadata propagated to the `AIAgentApi` that may be used for logging.
#[derive(Clone, Debug, Serialize)]
pub struct RequestMetadata {
    /// `true` if the user query was autodetected as AI input.
    ///
    /// This only applies to `AIAgentInput::UserQuery`.
    pub is_autodetected_user_query: bool,

    /// The entrypoint (onboarding, prompt suggestion, etc.) of the AI conversation.
    pub entrypoint: EntrypointType,

    /// Whether this request is an automatic resume triggered by a previous error.
    pub is_auto_resume_after_error: bool,
}

pub use cloud_object_models::SuggestedLoggingId;

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct SuggestedRule {
    pub name: String,
    pub content: String,
    pub logging_id: SuggestedLoggingId,
}

#[derive(Debug, Default, Clone, Eq, PartialEq)]
pub struct Suggestions {
    pub rules: Vec<SuggestedRule>,
    pub agent_mode_workflows: Vec<SuggestedAgentModeWorkflow>,
}

impl Suggestions {
    /// Extend the suggestions, ensuring that we don't add duplicates by checking the logging_id
    pub fn extend(&mut self, other: &Suggestions) {
        let existing_logging_ids: Vec<_> = self
            .rules
            .iter()
            .map(|rule| rule.logging_id.clone())
            .collect();
        let new_rules = other
            .rules
            .iter()
            .filter(|rule| !existing_logging_ids.contains(&rule.logging_id))
            .cloned();
        self.rules.extend(new_rules);

        // Add new agent mode workflows, ensuring no duplicates by logging_id
        let existing_workflow_ids: Vec<_> = self
            .agent_mode_workflows
            .iter()
            .map(|workflow| workflow.logging_id.clone())
            .collect();
        let new_workflows = other
            .agent_mode_workflows
            .iter()
            .filter(|workflow| !existing_workflow_ids.contains(&workflow.logging_id))
            .cloned();
        self.agent_mode_workflows.extend(new_workflows);
    }
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;

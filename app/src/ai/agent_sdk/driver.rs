use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::future::Future;
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use ai::skills::{ParsedSkill, SKILL_PROVIDER_DEFINITIONS};
use anyhow::{anyhow, Context as _};
use futures::channel::oneshot;
use futures::future::{self, join_all, Either};
use futures::FutureExt as _;
use itertools::Itertools as _;
use oneshot::{Canceled, Receiver};
use repo_metadata::local_model::IndexedRepoState;
use repo_metadata::{RepoMetadataModel, RepositoryIdentifier};
use session_sharing_protocol::sharer::SessionRetentionReason;
use uuid::Uuid;
use warp_cli::agent::{Harness, OutputFormat};
use warp_cli::mcp::MCPSpec;
use warp_cli::share::ShareRequest;
use warp_cli::skill::SkillSpec;
use warp_core::features::FeatureFlag;
use warp_core::{report_error, report_if_error, safe_debug, safe_error, safe_info};
use warp_graphql::ai::AgentTaskState;
use warp_managed_secrets::ManagedSecretValue;
use warp_util::local_or_remote_path::LocalOrRemotePath;
use warpui::r#async::{FutureExt, TimeoutError};
use warpui::{AppContext, Entity, ModelContext, ModelHandle, ModelSpawner, SingletonEntity};

use crate::ai::agent::conversation::AIConversationId;
use crate::ai::agent::{
    AIAgentActionResultType, AIAgentExchange, AIAgentInput, AIAgentOutput, CancellationReason,
    RenderableAIError, RequestFileEditsResult,
};
use crate::ai::agent_sdk::driver::harness::{
    harness_model_env_vars, task_env_vars, HarnessCleanupDisposition, HarnessKind, HarnessRunner,
    ResumePayload, SavePoint, ThirdPartyHarness, ThirdPartyHarnessTelemetryEvent,
};
use crate::ai::agent_sdk::setup_observability::{SetupClientEventReporter, SetupStep};
use crate::ai::ambient_agents::task::HarnessModelConfig;
use crate::ai::ambient_agents::{
    conversation_output_status_from_conversation, AmbientAgentTaskId, AmbientConversationStatus,
};
use crate::ai::blocklist::agent_view::AgentViewEntryOrigin;
use crate::ai::blocklist::local_agent_task_sync_model::LocalAgentTaskSyncModel;
use crate::ai::blocklist::orchestration_event_streamer::{
    register_agent_event_consumer, unregister_agent_event_consumer,
};
use crate::ai::blocklist::{
    BlocklistAIHistoryEvent, BlocklistAIHistoryModel, BlocklistAIPermissions,
};
use crate::ai::cloud_environments::{
    AmbientAgentEnvironment, CloudAmbientAgentEnvironment, GithubRepo,
};
use crate::ai::document::ai_document_model::{AIDocumentModel, AIDocumentModelEvent};
use crate::ai::execution_profiles::profiles::AIExecutionProfilesModel;
use crate::ai::llms::{LLMId, LLMPreferences};
use crate::ai::mcp::file_based_manager::{FileBasedMCPManager, FileBasedMCPManagerEvent};
use crate::ai::mcp::parsing::{normalize_mcp_json, resolve_json, ParsedTemplatableMCPServerResult};
use crate::ai::mcp::templatable_manager::TemplatableMCPServerManagerEvent;
use crate::ai::mcp::{
    JSONMCPServer, MCPServerState, TemplatableMCPServerInstallation, TemplatableMCPServerManager,
};
use crate::ai::skills::{
    filter_skills_by_spec, read_skills_from_directories, resolve_skill_repos, SkillManager,
    SkillWatcher,
};
use crate::auth::AuthStateProvider;
use crate::cloud_object::{CloudObject, CloudObjectLookup as _};
use crate::send_telemetry_from_app_ctx;
use crate::server::ids::{ServerId, SyncId};
use crate::server::server_api::ai::{AIClient, TaskStatusUpdate};
use crate::server::server_api::harness_support::{
    HarnessSupportClient, ResolvePromptAttachedSkill, ResolvePromptRequest,
};
use crate::server::server_api::ServerApiProvider;
use crate::terminal::cli_agent_sessions::plugin_manager::{
    plugin_manager_for, CliAgentPluginManager,
};
use crate::terminal::cli_agent_sessions::{
    CLIAgentSessionStatus, CLIAgentSessionsModel, CLIAgentSessionsModelEvent,
};
use crate::terminal::model::BlockId;
use crate::terminal::view::ConversationRestorationInNewPaneType;

pub(crate) mod attachments;
pub(crate) mod cloud_provider;
pub(crate) mod environment;
mod error_classification;
pub(crate) mod git_credentials;
pub(crate) mod harness;
mod harness_output_monitor;
pub(super) mod output;
mod snapshot;
pub(crate) mod terminal;

use environment::PrepareEnvironmentError;
pub(crate) use snapshot::upload_snapshot_for_handoff;
use terminal::TerminalDriverEvent;

const MCP_SERVER_STARTUP_TIMEOUT: Duration = Duration::from_secs(20);
const HARNESS_SAVE_INTERVAL: Duration = Duration::from_secs(30);
/// Timeout for individual harness auth preflight commands.
const PREFLIGHT_CHECK_TIMEOUT: Duration = Duration::from_secs(30);
pub(crate) const WARP_DRIVE_SYNC_TIMEOUT: Duration = Duration::from_secs(60);
const SETUP_FAILED_IDLE_TIMEOUT: Duration = Duration::from_secs(120);
/// Maximum time to wait for an automatic error resume before propagating the error.
/// If no follow-up status arrives within this window, the driver terminates with the
/// original error so the CLI does not hang indefinitely.
const AUTO_RESUME_TIMEOUT: Duration = Duration::from_secs(120);
/// Signals to Claude child-harness hooks that Warp already owns the background
/// message-listener lifecycle, so the plugin should reuse the shared state
/// files instead of spawning and cleaning up its own listener.
///
/// When this variable is absent, the Claude plugin falls back to its legacy
/// self-managed listener path so older Warp builds and standalone plugin
/// invocations keep working.
pub(crate) const OZ_MESSAGE_LISTENER_MANAGED_EXTERNALLY_ENV: &str =
    "OZ_MESSAGE_LISTENER_MANAGED_EXTERNALLY";
/// Optional root directory for the per-session Claude message-listener state
/// that Warp and the Claude hook scripts share.
pub(crate) const OZ_MESSAGE_LISTENER_STATE_ROOT_ENV: &str = "OZ_MESSAGE_LISTENER_STATE_ROOT";
// Keep exporting the legacy `OZ_PARENT_*` names to child hooks until the
// external Claude plugin has fully migrated to the canonical
// `OZ_MESSAGE_LISTENER_*` names.
const LEGACY_OZ_PARENT_LISTENER_MANAGED_EXTERNALLY_ENV: &str =
    "OZ_PARENT_LISTENER_MANAGED_EXTERNALLY";
const LEGACY_OZ_PARENT_STATE_ROOT_ENV: &str = "OZ_PARENT_STATE_ROOT";

/// IdleTimeoutSender is wrapper around a sender that signals when a run is done after
/// an idle timeout. Used for both Oz runs and third-party harnesses.
///
/// We use a generation-based approach to cancel timers instead of storing timer handles:
///
/// - `tx_cell` holds the completion sender; taking it ensures we only complete once.
/// - `timer_generation` starts at 0 and is incremented each time we want to cancel
///   existing timers and potentially start a new one. When a timer fires, it checks
///   if its generation still matches the current generation. If not, the timer was
///   "cancelled" by a newer timer and should not complete the conversation.
///
/// This approach avoids the complexity of storing and cancelling timer handles,
/// while allowing multiple events to safely race without double-completion.
struct IdleTimeoutSender<T: Send + 'static> {
    tx_cell: Arc<Mutex<Option<oneshot::Sender<T>>>>,
    generation: Arc<AtomicUsize>,
}

impl<T: Send + 'static> IdleTimeoutSender<T> {
    fn new(tx: oneshot::Sender<T>) -> Self {
        Self {
            tx_cell: Arc::new(Mutex::new(Some(tx))),
            generation: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// End the run by sending `value` immediately.
    fn end_run_now(&self, value: T) {
        if let Ok(mut guard) = self.tx_cell.lock() {
            if let Some(sender) = guard.take() {
                let _ = sender.send(value);
            }
        }
    }

    /// End the run after `timeout` by sending `value`, unless cancelled before then.
    fn end_run_after(&self, timeout: Duration, value: T) {
        // Increment the generation counter to invalidate any existing timers,
        // then capture the new generation for our timer to check against.
        let current_gen = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let tx_cell = Arc::clone(&self.tx_cell);
        let generation = Arc::clone(&self.generation);

        // Spawn a background thread that will complete the oneshot after the idle timeout,
        // unless a follow-up query resets the timer (by bumping the generation counter).
        thread::spawn(move || {
            thread::sleep(timeout);

            // Check if our timer generation is still current. If not, a follow-up
            // query or other activity has "cancelled" this timer by bumping the generation.
            if generation.load(Ordering::SeqCst) != current_gen {
                return;
            }
            if let Ok(mut guard) = tx_cell.lock() {
                if let Some(sender) = guard.take() {
                    // Send the value after the idle timeout expires.
                    let _ = sender.send(value);
                }
            }
        });
    }

    /// Cancel any pending idle timers.
    fn cancel_idle_timeout(&self) {
        if self.generation.load(Ordering::SeqCst) > 0 {
            self.generation.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// End the run with `value`, deferring by `idle_on_complete` when set so the
    /// driver stays alive long enough to accept a follow-up. Use for "graceful"
    /// terminal statuses (Success / Blocked / Cancelled). Falls back to
    /// immediate completion when `idle_on_complete` is `None`.
    fn complete_with_optional_idle(&self, idle_on_complete: Option<Duration>, value: T) {
        if let Some(idle_timeout) = idle_on_complete {
            self.end_run_after(idle_timeout, value);
        } else {
            self.end_run_now(value);
        }
    }
}

/// How to resume an existing conversation when starting an agent run.
///
/// The Oz harness restores the full conversation transcript into the terminal pane and treats
/// any new prompt as a follow-up; third-party harnesses round-trip a harness-specific payload
/// (see [`ResumePayload`]) instead.
pub enum ResumeOptions {
    Oz(Box<ConversationRestorationInNewPaneType>),
    ThirdParty(Box<ResumePayload>),
}

/// Options for initializing the agent driver.
pub struct AgentDriverOptions {
    /// Initial working directory for the agent's terminal session.
    pub working_dir: PathBuf,
    /// Secrets to inject into the agent's terminal session.
    pub secrets: HashMap<String, ManagedSecretValue>,
    /// ID of the task being executed.
    pub task_id: Option<AmbientAgentTaskId>,
    /// Parent run ID for child orchestration flows, if this task was spawned by another run.
    pub parent_run_id: Option<String>,
    /// Whether the agent run should share its session.
    pub should_share: bool,
    /// How long to keep the session alive after the agent run completes, if at all.
    pub idle_on_complete: Option<Duration>,
    /// If set, resume an existing conversation instead of starting fresh. The variant
    /// determines which harness-specific path is taken (Oz transcript restore vs.
    /// third-party-harness payload rehydration).
    pub resume: Option<ResumeOptions>,
    /// Cloud providers to configure within the agent's session.
    pub cloud_providers: Vec<Box<dyn cloud_provider::CloudProvider>>,
    /// Resolved environment configuration, if any.
    pub environment: Option<AmbientAgentEnvironment>,
    /// Selected execution harness for this run.
    pub selected_harness: Harness,
    /// Model config for the selected harness. Only used for non-Oz harnesses.
    pub third_party_harness_model_config: Option<HarnessModelConfig>,
    /// Whether to skip end-of-run snapshot upload.
    pub snapshot_disabled: Option<bool>,
    /// End-of-run snapshot upload timeout override.
    pub snapshot_upload_timeout: Option<Duration>,
    /// Declarations script timeout override.
    pub snapshot_script_timeout: Option<Duration>,
    /// Skip the initial `StartFromAmbientRunPrompt` so the agent waits for a
    /// follow-up instead of hallucinating an empty turn. Sourced from the
    /// `--skip-initial-turn` CLI flag, which the worker emits when the
    /// execution input has neither a prompt nor a snapshot token.
    pub skip_initial_turn: bool,
    /// Fail the run when MCP servers fail to start, instead of continuing
    /// without the unavailable servers.
    pub strict_mcp_startup: bool,
    /// MCP server startup timeout override.
    pub mcp_startup_timeout: Option<Duration>,
}

/// `AgentDriver` is a model for driving an ambient Warp agent to completion.
///
/// Its primary responsibility is to configure a headless terminal pane and execute an AI query within it.
pub struct AgentDriver {
    terminal_driver: ModelHandle<terminal::TerminalDriver>,
    working_dir: PathBuf,

    /// Secrets available to the running agent.
    /// - Secrets are injected as environment variables when the terminal session is created.
    /// - Secrets are passed to MCP servers during spawning.
    secrets: Arc<HashMap<String, ManagedSecretValue>>,

    /// Env vars passed to the terminal session, including resolved secrets, cloud
    /// provider vars, task vars, and sandbox flags. Passed to
    /// `build_runner` so harnesses can look up resolved secret values
    /// without re-deriving precedence.
    resolved_env_vars: Arc<HashMap<OsString, OsString>>,

    output_format: OutputFormat,

    // The associated task ID for this agent run, if any.
    task_id: Option<AmbientAgentTaskId>,

    /// Harness adapter for the running agent. This is only set if:
    /// - The harness has started successfully.
    /// - We're using a third-party harness.
    /// In the future, we _may_ use the harness abstraction for the Oz agent as well.
    harness: Option<Arc<dyn HarnessRunner>>,

    // Optional idle timeout after completion. If set, the process will stay alive for follow-ups
    // and exit after this period of inactivity.
    idle_on_complete: Option<Duration>,

    // The conversation ID to continue (if provided).
    restored_conversation_id: Option<AIConversationId>,

    /// If set, a third-party-harness conversation to resume. Consumed when
    /// preparing the harness runner and cleared afterward.
    resume_payload: Option<ResumePayload>,

    /// Cloud providers set up within this driver session.
    cloud_providers: Vec<Box<dyn cloud_provider::CloudProvider>>,

    /// Resolved environment configuration.
    environment: Option<AmbientAgentEnvironment>,

    // End-of-run snapshot upload controls.
    snapshot_disabled: bool,
    snapshot_upload_timeout: Duration,
    snapshot_script_timeout: Duration,

    /// Conversation ID this driver is running. Set at construction for
    /// resumed runs and on `ConversationServerTokenAssigned` for fresh
    /// runs; consumed by `unregister_streamer_consumer` at end of run.
    run_conversation_id: Option<AIConversationId>,

    /// Parent agent run's `run_id` from the server task metadata, when
    /// this driver run was spawned by another agent. Stamped onto the
    /// conversation's `parent_agent_id` field at register time so the
    /// streamer recognizes the child role in driver-hosted processes.
    parent_run_id: Option<String>,
    third_party_harness_model_config: Option<HarnessModelConfig>,

    /// Async writer that records `file` declarations for paths the agent creates or edits
    /// via `RequestFileEdits`. `Some` only when `FeatureFlag::OzHandoff` is enabled, the run
    /// has a cloud task id, and `--no-snapshot` was not set; `None` keeps the observer a
    /// pure no-op for local and disabled runs.
    snapshot_file_writer: Option<snapshot::DeclarationsWriterHandle>,

    /// Whether the driver should skip dispatching the initial
    /// `StartFromAmbientRunPrompt`. Mirror of `AgentDriverOptions::skip_initial_turn`,
    /// sourced from the `--skip-initial-turn` CLI flag. Read by `execute_run`
    /// to gate the empty-prompt short-circuit path.
    skip_initial_turn: bool,

    /// Whether MCP server startup failures are fatal for the run.
    strict_mcp_startup: bool,
    /// How long to wait for MCP servers to start before degrading (or failing,
    /// in strict mode).
    mcp_startup_timeout: Duration,
}

pub(crate) enum SDKConversationOutputStatus {
    Success,
    Error { error: RenderableAIError },
    Cancelled { reason: CancellationReason },
    Blocked { blocked_action: String },
}

impl SDKConversationOutputStatus {
    pub fn into_result(self) -> Result<(), AgentDriverError> {
        match self {
            SDKConversationOutputStatus::Success => Ok(()),
            SDKConversationOutputStatus::Error { error } => {
                Err(AgentDriverError::ConversationError { error })
            }
            // NOTE: this doesn't happen in the SDK (yet) because CTRL+C kills the whole program.
            SDKConversationOutputStatus::Cancelled { reason } => {
                Err(AgentDriverError::ConversationCancelled { reason })
            }
            SDKConversationOutputStatus::Blocked { blocked_action } => {
                Err(AgentDriverError::ConversationBlocked { blocked_action })
            }
        }
    }
}

/// Task configuration for running an agent.
#[derive(Debug)]
pub struct Task {
    /// The prompt for the agent.
    pub prompt: AgentRunPrompt,
    pub model: Option<LLMId>,
    /// ID of the profile to run as (SyncId string). If None, use the default profile.
    pub profile: Option<String>,
    /// MCP server specifications to start prior to execution.
    pub mcp_specs: Vec<MCPSpec>,
    /// Which harness to use for executing the agent run.
    pub harness: HarnessKind,
}

struct GlobalSkillResolution {
    specs: Vec<SkillSpec>,
    repos: Vec<GithubRepo>,
}

/// Prompt that we initialize an agent driver with. Can represent either a local prompt or
/// a prompt that we resolve server-side.
#[derive(Debug, Clone)]
pub enum AgentRunPrompt {
    /// Prompt is provided locally (already resolved to a plain string).
    Local(String),
    /// Server resolves prompt from the task's stored prompt.
    /// Used when task_id is provided without an explicit prompt.
    ServerSide {
        /// Optional skill whose instructions are sent to the agent.
        skill: Option<ParsedSkill>,
        /// Directory where task attachments were downloaded.
        attachments_dir: Option<String>,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum AgentDriverError {
    #[error("Terminal session is not available.")]
    TerminalUnavailable,
    #[error("Invalid runtime state - please file a bug report.")]
    InvalidRuntimeState,
    #[error("Requested MCP server not found: {0}")]
    MCPServerNotFound(uuid::Uuid),
    #[error("Failed to start MCP servers: {}", .details.join("; "))]
    MCPStartupFailed {
        /// One line per unavailable server (e.g. "'datadog' failed to start:
        /// connection refused").
        details: Vec<String>,
    },
    #[error("Failed to parse MCP server JSON: {0}")]
    MCPJsonParseError(String),
    #[error("MCP server configuration is missing required variables")]
    MCPMissingVariables,
    #[error("Agent profile \"{0}\" not found")]
    ProfileError(String),
    #[error(
        "Failed to authenticate with server - please log in via 'oz login', provide an API key via '--api-key <key>', or set the WARP_API_KEY environment variable"
    )]
    NotLoggedIn,
    #[error("Saved prompt not found for id {0}")]
    AIWorkflowNotFound(String),
    #[error("Terminal bootstrap failed")]
    BootstrapFailed,
    #[error("Unable to share agent session")]
    ShareSessionFailed {
        #[source]
        error: terminal::ShareSessionError,
    },
    #[error("Error syncing Warp Drive")]
    WarpDriveSyncFailed,
    #[error("Requested environment not found: {0}")]
    EnvironmentNotFound(String),
    #[error("Environment setup failed: {0}")]
    EnvironmentSetupFailed(String),
    #[error("Cloud provider setup failed")]
    CloudProviderSetupFailed(#[from] cloud_provider::CloudProviderSetupError),
    #[error("Could not resolve working directory {}", path.display())]
    InvalidWorkingDirectory {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("{error}")]
    ConversationError { error: RenderableAIError },
    #[error("Conversation was canceled: {reason}")]
    ConversationCancelled { reason: CancellationReason },
    #[error("The agent got stuck waiting for user confirmation on the action: {blocked_action}")]
    ConversationBlocked { blocked_action: String },
    #[error("Timed out refreshing team metadata")]
    TeamMetadataRefreshTimeout,
    #[error("{0}")]
    SkillResolutionFailed(String),
    #[error("Failed to build agent configuration")]
    ConfigBuildFailed(#[source] anyhow::Error),
    #[error("Failed to resolve server-side prompt")]
    PromptResolutionFailed(#[source] anyhow::Error),
    #[error("Failed to fetch task secrets")]
    SecretsFetchFailed(#[source] anyhow::Error),
    #[error("Failed to load conversation: {0}")]
    ConversationLoadFailed(String),
    #[error("Failed to initialize AWS Bedrock credentials: {0}")]
    AwsBedrockCredentialsFailed(String),
    #[error(
        "Conversation {conversation_id} was produced by the {expected} harness, but --harness {got} was requested. \
         Re-run with --harness {expected} (or omit --harness to match) to continue this conversation."
    )]
    ConversationHarnessMismatch {
        conversation_id: String,
        expected: String,
        got: String,
    },
    #[error(
        "Task {task_id} was created with the {expected} harness, but --harness {got} was requested. \
         Re-run with --harness {expected} (or omit --harness to match) to continue this task."
    )]
    TaskHarnessMismatch {
        task_id: String,
        expected: String,
        got: String,
    },
    #[error(
        "Conversation {conversation_id} has no stored transcript for the {harness} harness. \
         The prior run may have crashed before saving any state."
    )]
    ConversationResumeStateMissing {
        harness: String,
        conversation_id: String,
    },
    #[error("Harness command exited with code {exit_code}")]
    HarnessCommandFailed { exit_code: i32 },
    #[error("Harness '{harness}' setup failed: {reason}")]
    HarnessSetupFailed { harness: String, reason: String },
    #[error("Harness '{harness}' config setup failed")]
    HarnessConfigSetupFailed {
        harness: String,
        #[source]
        error: anyhow::Error,
    },
    #[error("Harness '{harness}' auth preflight failed")]
    HarnessAuthCheckFailed {
        harness: String,
        /// Stderr/stdout captured from the failing command, for logs.
        detail: String,
    },
    #[error("Harness '{harness}' reported a runtime failure matching '{pattern}'")]
    HarnessRuntimeFailureDetected {
        harness: String,
        /// The originating needle from `runtime_error_patterns` that hit.
        pattern: String,
        /// Matching row(s) from the harness block, trimmed and capped.
        excerpt: String,
    },
}

impl From<warpui::ModelDropped> for AgentDriverError {
    fn from(_: warpui::ModelDropped) -> Self {
        AgentDriverError::InvalidRuntimeState
    }
}

impl From<PrepareEnvironmentError> for AgentDriverError {
    fn from(error: PrepareEnvironmentError) -> Self {
        match error {
            PrepareEnvironmentError::InvalidRuntimeState => AgentDriverError::InvalidRuntimeState,
            PrepareEnvironmentError::TerminalDriver { source } => source,
            error => AgentDriverError::EnvironmentSetupFailed(error.to_string()),
        }
    }
}

impl AgentDriver {
    #[tracing::instrument(name = "AgentDriver::new", skip_all, err, fields(
        tags.cloud_agent = true,
        task_id = ?options.task_id,
        parent_run_id = ?options.parent_run_id,
        is_sandbox = tracing::field::Empty,
    ))]
    pub fn new(
        options: AgentDriverOptions,
        ctx: &mut ModelContext<Self>,
    ) -> Result<Self, AgentDriverError> {
        let AgentDriverOptions {
            working_dir,
            task_id,
            parent_run_id,
            should_share,
            idle_on_complete,
            secrets,
            resume,
            cloud_providers,
            environment,
            selected_harness,
            third_party_harness_model_config,
            snapshot_disabled,
            snapshot_upload_timeout,
            snapshot_script_timeout,
            skip_initial_turn,
            strict_mcp_startup,
            mcp_startup_timeout,
        } = options;

        // Split the unified resume option into the two internal slots that the rest of
        // the driver consumes: terminal-driven Oz transcript restoration vs. third-party
        // harness payload rehydration.
        let (conversation_restoration, resume_payload) = match resume {
            Some(ResumeOptions::Oz(restoration)) => (Some(*restoration), None),
            Some(ResumeOptions::ThirdParty(payload)) => (None, Some(*payload)),
            None => (None, None),
        };

        safe_info!(
            safe: ("Initializing agent driver: share={should_share}, idle_on_complete={idle_on_complete:?}"),
            full: (
                "Initializing agent driver: share={should_share}, idle_on_complete={idle_on_complete:?}, working_dir={}",
                working_dir.display()
            )
        );

        // If we're not logged in, the root view will go to an auth screen, and all subsequent steps will fail.
        // This should be impossible, since we enforce login before reaching this point.
        if !AuthStateProvider::as_ref(ctx).get().is_logged_in() {
            return Err(AgentDriverError::NotLoggedIn);
        }

        // Extract the conversation ID if we're restoring a conversation.
        // This will be used when submitting the initial query to continue the conversation.
        let restored_conversation_id =
            conversation_restoration
                .as_ref()
                .and_then(|restoration| match restoration {
                    ConversationRestorationInNewPaneType::Historical { conversation, .. } => {
                        Some(conversation.id())
                    }
                    _ => None,
                });

        let mut env_vars = build_secret_env_vars(&secrets);

        // Inject cloud provider env vars.
        cloud_provider::collect_env_vars(&cloud_providers, &mut env_vars)?;
        // Clone before consuming for env vars; the field on `Self` is
        // also needed at register time.
        let parent_run_id_for_self = parent_run_id.clone();
        env_vars.extend(task_env_vars(
            task_id.as_ref(),
            parent_run_id.as_deref(),
            selected_harness,
        ));
        env_vars.extend(harness_model_env_vars(
            selected_harness,
            third_party_harness_model_config.as_ref(),
        ));

        // Signal to third-party harnesses (e.g. Claude Code) that we're in a sandbox
        // so they allow root execution with permissive flags.
        if warp_isolation_platform::detect().is_some() {
            env_vars.insert(OsString::from("IS_SANDBOX"), OsString::from("1"));
            tracing::Span::current().record("is_sandbox", true);
        }

        let resolved_env_vars = Arc::new(env_vars);

        let terminal_driver = terminal::TerminalDriver::create(
            terminal::TerminalDriverOptions {
                working_dir: working_dir.clone(),
                env_vars: HashMap::clone(&resolved_env_vars),
                should_share,
                task_id,
                conversation_restoration,
            },
            ctx,
        )?;

        // Subscribe to TerminalDriver events for task-specific handling.
        ctx.subscribe_to_model(&terminal_driver, |me, event, ctx| {
            me.handle_terminal_driver_event(event, ctx);
        });

        let mut run_conversation_id: Option<AIConversationId> = None;

        // For a resumed conversation the ID is known up front; register
        // immediately so the streamer can satisfy the parent gate as soon
        // as the first child is registered.
        if let Some(conv_id) = restored_conversation_id {
            stamp_parent_agent_id_if_some(conv_id, parent_run_id_for_self.as_deref(), ctx);
            register_agent_event_consumer(conv_id, ctx.model_id(), ctx);
            run_conversation_id = Some(conv_id);
        }

        // Spawn the async declarations writer only when the snapshot pipeline will actually
        // read what it produces: feature enabled, cloud task run, and --no-snapshot not set.
        let snapshot_disabled_value = snapshot_disabled.unwrap_or(false);
        let snapshot_file_writer = match task_id {
            Some(id) if FeatureFlag::OzHandoff.is_enabled() && !snapshot_disabled_value => {
                let background = ctx.background_executor();
                Some(snapshot::DeclarationsWriterHandle::new(
                    id,
                    working_dir.clone(),
                    &background,
                ))
            }
            _ => None,
        };

        Ok(Self {
            terminal_driver,
            working_dir,
            secrets: Arc::new(secrets),
            resolved_env_vars,
            output_format: OutputFormat::default(),
            task_id,
            harness: None,
            idle_on_complete,
            restored_conversation_id,
            resume_payload,
            cloud_providers,
            environment,
            snapshot_disabled: snapshot_disabled_value,
            snapshot_upload_timeout: snapshot_upload_timeout
                .unwrap_or(snapshot::DEFAULT_SNAPSHOT_UPLOAD_TIMEOUT),
            snapshot_script_timeout: snapshot_script_timeout
                .unwrap_or(snapshot::DEFAULT_DECLARATIONS_SCRIPT_TIMEOUT),
            run_conversation_id,
            parent_run_id: parent_run_id_for_self,
            third_party_harness_model_config,
            snapshot_file_writer,
            skip_initial_turn,
            strict_mcp_startup,
            mcp_startup_timeout: mcp_startup_timeout.unwrap_or(MCP_SERVER_STARTUP_TIMEOUT),
        })
    }

    /// Minimal constructor for unit tests that need a live `AgentDriver` model to call
    /// methods on (e.g. `load_environment_skills`, `load_global_skills`) without
    /// bootstrapping a full agent run.
    ///
    /// The caller is responsible for creating the `TerminalDriver` handle beforehand
    /// (e.g. via `TerminalDriver::create_from_existing_view`) and for registering all
    /// required singleton models before constructing the driver.
    #[cfg(test)]
    pub(crate) fn new_for_test(
        working_dir: PathBuf,
        terminal_driver: ModelHandle<terminal::TerminalDriver>,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        ctx.subscribe_to_model(&terminal_driver, |me, event, ctx| {
            me.handle_terminal_driver_event(event, ctx);
        });
        Self {
            terminal_driver,
            working_dir,
            secrets: Arc::new(HashMap::new()),
            resolved_env_vars: Arc::new(HashMap::new()),
            output_format: OutputFormat::default(),
            task_id: None,
            harness: None,
            idle_on_complete: None,
            restored_conversation_id: None,
            resume_payload: None,
            cloud_providers: Vec::new(),
            environment: None,
            snapshot_disabled: false,
            snapshot_upload_timeout: snapshot::DEFAULT_SNAPSHOT_UPLOAD_TIMEOUT,
            snapshot_script_timeout: snapshot::DEFAULT_DECLARATIONS_SCRIPT_TIMEOUT,
            run_conversation_id: None,
            parent_run_id: None,
            third_party_harness_model_config: None,
            snapshot_file_writer: None,
            skip_initial_turn: false,
            strict_mcp_startup: false,
            mcp_startup_timeout: MCP_SERVER_STARTUP_TIMEOUT,
        }
    }

    /// Pair to the registration in `new` / `execute_run`. No-op when
    /// nothing was registered.
    fn unregister_streamer_consumer(&self, ctx: &mut ModelContext<Self>) {
        let Some(conversation_id) = self.run_conversation_id else {
            return;
        };
        unregister_agent_event_consumer(conversation_id, ctx.model_id(), ctx);
    }

    pub fn set_output_format(&mut self, output_format: OutputFormat) {
        self.output_format = output_format;
    }

    pub fn add_share_requests(
        &self,
        share_requests: impl IntoIterator<Item = ShareRequest>,
        ctx: &mut ModelContext<Self>,
    ) {
        self.terminal_driver.update(ctx, |td, ctx| {
            td.add_share_requests(share_requests, ctx);
        });
    }
    fn extend_shared_session_retention(
        &mut self,
        reason: SessionRetentionReason,
        ctx: &mut ModelContext<Self>,
    ) {
        self.terminal_driver.update(ctx, |driver, ctx| {
            driver.extend_shared_session_retention(reason, ctx);
        });
    }

    pub fn run(
        &mut self,
        task: Task,
        ctx: &mut ModelContext<Self>,
    ) -> impl Future<Output = Result<(), AgentDriverError>> {
        let (tx, rx) = oneshot::channel();
        let foreground = ctx.spawner();
        let foreground_for_error = foreground.clone();
        let server_api = ServerApiProvider::as_ref(ctx).get_ai_client();
        let task_id = self.task_id;
        let idle_on_complete = self.idle_on_complete;

        ctx.spawn(
            async move {
                // Mark the task as IN_PROGRESS before starting work. This covers
                // the gap during environment setup, MCP startup, etc. — before any
                // conversation exists and LocalAgentTaskSyncModel can fire.
                if let Some(task_id) = task_id {
                    if let Err(e) = server_api
                        .update_agent_task(
                            task_id,
                            Some(AgentTaskState::InProgress),
                            None,
                            None,
                            None,
                        )
                        .await
                    {
                        log::error!("Failed to update agent task state to InProgress: {e}");
                    }
                }
                let result = Self::run_internal(task, foreground.clone()).await;

                // Unregister the driver consumer now that the run is done.
                // The streamer will tear down the SSE if no other consumer
                // remains and the conversation isn't a child.
                let _ = foreground
                    .spawn(|me, ctx| me.unregister_streamer_consumer(ctx))
                    .await;

                // Run the snapshot upload before signaling the caller. The caller resumes and
                // triggers process termination as soon as it receives `result`; the snapshot
                // upload depends on the event loop that termination tears down, so anything
                // async it awaits (presigned URL fetch, uploads, timers) would get abandoned
                // mid-flight. Provider cleanup is just local temp-file teardown, so it's safe
                // to run after the send.
                Self::run_snapshot_upload(&foreground).await;

                if tx.send(result).is_err() {
                    log::error!("Caller did not wait for agent driver to finish");
                }

                Self::cleanup(foreground).await;
            },
            |_, _, _| {},
        );

        let server_api_for_error = ServerApiProvider::as_ref(ctx).get_ai_client();

        async move {
            if let Some(ref task_id) = task_id {
                log::info!("Executing task {task_id}");
            }

            let result = match rx.await {
                Ok(result) => result,
                Err(Canceled) => {
                    log::error!("Agent driver exited abruptly");
                    Err(AgentDriverError::InvalidRuntimeState)
                }
            };

            // Report driver-level errors directly to the server. These errors
            // occur before or outside a conversation (e.g. bootstrap, MCP startup,
            // environment setup) so LocalAgentTaskSyncModel never fires for them.
            // Success/blocked/cancelled are handled by LocalAgentTaskSyncModel.
            if let (Some(task_id), Err(err)) = (task_id, &result) {
                report_driver_error(task_id, err, &server_api_for_error).await;
                if matches!(err, AgentDriverError::EnvironmentSetupFailed(_)) {
                    let _ = foreground_for_error
                        .spawn(|me, ctx| {
                            me.extend_shared_session_retention(
                                SessionRetentionReason::SetupFailed,
                                ctx,
                            );
                        })
                        .await;

                    // Keep the session alive after environment setup failures so
                    // the viewer can connect, receive scrollback, and see the error.
                    if let Some(idle_timeout) = idle_on_complete {
                        let timeout = idle_timeout.min(SETUP_FAILED_IDLE_TIMEOUT);
                        log::info!(
                            "Environment setup failed; keeping session alive for {timeout:?}"
                        );
                        warpui::r#async::Timer::after(timeout).await;
                    }
                }
            }

            result
        }
    }

    /// Log all valid environment IDs for the user.
    pub(super) fn log_valid_environments(app: &AppContext) {
        let environments = CloudAmbientAgentEnvironment::get_all(app);
        if environments.is_empty() {
            log::error!("No environments available for this user.");
        } else {
            log::error!("Valid environment IDs:");
            for env in environments {
                log::error!("  - {} ({})", env.sync_id(), env.model().string_model.name);
            }
        }
    }

    /// Check that the working directory exists. Since it's user-specified, we don't automatically
    /// create the directory (in case they made a typo).
    fn check_working_dir(&self) -> impl Future<Output = Result<(), AgentDriverError>> {
        let working_dir = self.working_dir.clone();
        async move {
            match async_fs::metadata(&working_dir).await {
                Ok(metadata) => {
                    if metadata.is_dir() {
                        Ok(())
                    } else {
                        Err(AgentDriverError::InvalidWorkingDirectory {
                            path: working_dir.to_owned(),
                            source: io::ErrorKind::NotADirectory.into(),
                        })
                    }
                }
                Err(err) => Err(AgentDriverError::InvalidWorkingDirectory {
                    path: working_dir.to_owned(),
                    source: err,
                }),
            }
        }
    }

    /// Resolve MCP specs into a map of MCP name to `JSONMCPServer` for use in
    /// third-party harnesses. Each spec is fully resolved (secrets applied, templates
    /// rendered) so harnesses can serialize directly into their native config format.
    fn resolve_mcp_specs_to_json(
        specs: &[MCPSpec],
        secrets: &HashMap<String, ManagedSecretValue>,
        ctx: &ModelContext<Self>,
    ) -> Result<HashMap<String, JSONMCPServer>, AgentDriverError> {
        let (existing_uuids, mut ephemeral_installations) = Self::resolve_mcp_specs(specs)?;
        let mut result = HashMap::new();

        // Resolve UUID-referenced servers from the TemplatableMCPServerManager.
        let manager = TemplatableMCPServerManager::as_ref(ctx);
        for uuid in &existing_uuids {
            let installation = manager
                .get_installed_server(uuid)
                .ok_or(AgentDriverError::MCPServerNotFound(*uuid))?
                .clone();
            let mut installation = installation;
            installation.apply_secrets(secrets);
            let resolved = resolve_json(&installation);
            let servers: HashMap<String, JSONMCPServer> = serde_json::from_str(&resolved)
                .map_err(|e| AgentDriverError::MCPJsonParseError(e.to_string()))?;
            result.extend(servers);
        }

        // Resolve ephemeral (inline JSON) servers.
        for installation in ephemeral_installations.iter_mut() {
            installation.apply_secrets(secrets);
            let resolved = resolve_json(installation);
            let servers: HashMap<String, JSONMCPServer> = serde_json::from_str(&resolved)
                .map_err(|e| AgentDriverError::MCPJsonParseError(e.to_string()))?;
            result.extend(servers);
        }

        Ok(result)
    }

    /// Resolve MCP specs into UUIDs for existing servers and ephemeral installations for inline specs.
    ///
    /// Returns (existing_server_uuids, ephemeral_installations)
    fn resolve_mcp_specs(
        specs: &[MCPSpec],
    ) -> Result<(Vec<Uuid>, Vec<TemplatableMCPServerInstallation>), AgentDriverError> {
        let mut existing_uuids = Vec::new();
        let mut ephemeral_installations = Vec::new();

        for spec in specs {
            match spec {
                MCPSpec::Uuid(uuid) => {
                    existing_uuids.push(*uuid);
                }
                MCPSpec::Json(json_str) => {
                    // Normalize the JSON - if it's a single server definition (has command or url
                    // at top level), wrap it with a generated name.
                    let normalized_json = normalize_mcp_json(json_str)
                        .map_err(|e| AgentDriverError::MCPJsonParseError(e.to_string()))?;

                    // Parse as inline MCP server configuration
                    let parsed_results =
                        ParsedTemplatableMCPServerResult::from_user_json(&normalized_json)
                            .map_err(|e| AgentDriverError::MCPJsonParseError(e.to_string()))?;

                    for result in parsed_results {
                        let installation = result
                            .templatable_mcp_server_installation
                            .ok_or(AgentDriverError::MCPMissingVariables)?;
                        ephemeral_installations.push(installation);
                    }
                }
            }
        }

        Ok((existing_uuids, ephemeral_installations))
    }

    /// Start MCP servers from profile allowlist for the terminal.
    fn start_profile_mcp_servers(
        &self,
        ctx: &mut ModelContext<Self>,
    ) -> impl Future<Output = Result<(), AgentDriverError>> {
        let terminal_id = self.terminal_driver.as_ref(ctx).terminal_view().id();
        let permissions = BlocklistAIPermissions::as_ref(ctx);
        let profile_allowlist = permissions.get_mcp_allowlist(ctx, Some(terminal_id));

        if !profile_allowlist.is_empty() {
            log::info!(
                "Starting {} MCP servers allowlisted in profile",
                profile_allowlist.len()
            );
        }
        self.start_mcp_servers(&profile_allowlist, ctx)
    }

    fn get_mcp_servers_to_start(
        &self,
        uuids: &[uuid::Uuid],
        ctx: &mut ModelContext<Self>,
    ) -> Result<HashSet<Uuid>, AgentDriverError> {
        let templatable_mcp_manager = TemplatableMCPServerManager::handle(ctx);

        let mut servers_to_start: HashSet<Uuid> = HashSet::new();

        for uuid in uuids.iter() {
            if templatable_mcp_manager
                .as_ref(ctx)
                .is_server_active_or_pending(*uuid)
            {
                log::debug!("MCP server {uuid} is already active or pending; skipping");
                continue;
            } else if templatable_mcp_manager
                .as_ref(ctx)
                .get_installed_server(uuid)
                .is_some()
            {
                servers_to_start.insert(*uuid);
            } else {
                return Err(AgentDriverError::MCPServerNotFound(*uuid));
            }
        }

        Ok(servers_to_start)
    }

    /// Subscribe to MCP server state changes and wait for every server in
    /// `servers` (keyed by installation UUID, valued by display name) to reach
    /// a terminal state (`Running` or `FailedToStart`), up to the configured
    /// startup timeout.
    ///
    /// Returns [`AgentDriverError::MCPStartupFailed`] naming the servers that
    /// failed to start or were still starting at the deadline. Callers decide
    /// whether that is fatal (see strict MCP startup handling in
    /// `run_internal`).
    ///
    /// Must be called before the servers are spawned so no state changes are
    /// missed, and never concurrently with another MCP wait: the driver keeps
    /// at most one subscription to [`TemplatableMCPServerManager`].
    fn wait_for_mcp_servers_started(
        &self,
        servers: HashMap<Uuid, String>,
        ctx: &mut ModelContext<Self>,
    ) -> impl Future<Output = Result<(), AgentDriverError>> {
        // If no servers to wait for, complete immediately.
        if servers.is_empty() {
            return Either::Right(future::ready(Ok(())));
        }

        // Stall for user-configured timeout, else 20 seconds (configured in [`AgentDriverOptions`]).
        let timeout = self.mcp_startup_timeout;
        let (tx, rx) = oneshot::channel::<()>();
        let mut tx = Some(tx);

        let pending = Arc::new(Mutex::new(servers));
        let failed: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let pending_for_subscription = Arc::clone(&pending);
        let failed_for_subscription = Arc::clone(&failed);

        let templatable_mcp_manager = TemplatableMCPServerManager::handle(ctx);
        let manager_clone = templatable_mcp_manager.clone();

        // Clear any stale subscription left behind by a previous wait that
        // timed out, so it can't tear down this wait's subscription.
        ctx.unsubscribe_from_model(&templatable_mcp_manager);
        ctx.subscribe_to_model(&templatable_mcp_manager, move |_me, event, ctx| {
            let TemplatableMCPServerManagerEvent::StateChanged { uuid, state } = event else {
                return;
            };
            let Ok(mut pending_servers) = pending_for_subscription.lock() else {
                return;
            };
            let Some(name) = pending_servers.get(uuid).cloned() else {
                // If we receive a state change for a server that we're not waiting for, ignore it.
                return;
            };
            match state {
                MCPServerState::Running => {
                    pending_servers.remove(uuid);
                }
                MCPServerState::FailedToStart => {
                    pending_servers.remove(uuid);
                    let error = TemplatableMCPServerManager::as_ref(ctx)
                        .get_server_error_message(*uuid)
                        .map(|message| format!(": {message}"))
                        .unwrap_or_default();
                    let detail = format!("'{name}' failed to start{error}");
                    log::warn!("MCP server {detail}");
                    if let Ok(mut failed_servers) = failed_for_subscription.lock() {
                        failed_servers.push(detail);
                    }
                }
                MCPServerState::NotRunning
                | MCPServerState::Starting
                | MCPServerState::Authenticating
                | MCPServerState::ShuttingDown => return,
            }
            if pending_servers.is_empty() {
                log::info!("All requested MCP servers reached a terminal state");
                if let Some(sender) = tx.take() {
                    let _ = sender.send(());
                }
                ctx.unsubscribe_from_model(&manager_clone);
            }
        });

        let spawner = ctx.spawner();
        Either::Left(async move {
            let wait_result = rx.with_timeout(timeout).await;

            let mut still_starting: Vec<String> = Vec::new();
            match wait_result {
                Ok(Ok(())) => {}
                Ok(Err(Canceled)) => {
                    log::error!("Subscription dropped before MCP servers started");
                    return Err(AgentDriverError::InvalidRuntimeState);
                }
                Err(TimeoutError) => {
                    still_starting = pending
                        .lock()
                        .map(|pending_servers| pending_servers.values().cloned().collect())
                        .unwrap_or_default();
                    still_starting.sort();
                    // The subscription is now stale; remove it so it can't
                    // tear down a later wait's subscription. This completes
                    // before this future resolves, so it cannot race with a
                    // subsequent wait.
                    let _ = spawner
                        .spawn(|_, ctx| {
                            let manager = TemplatableMCPServerManager::handle(ctx);
                            ctx.unsubscribe_from_model(&manager);
                        })
                        .await;
                }
            }

            let mut details = failed
                .lock()
                .map(|failed_servers| failed_servers.clone())
                .unwrap_or_default();
            details.sort();
            details.extend(
                still_starting
                    .iter()
                    .map(|name| format!("'{name}' did not start within {}s", timeout.as_secs())),
            );

            if details.is_empty() {
                Ok(())
            } else {
                Err(AgentDriverError::MCPStartupFailed { details })
            }
        })
    }

    /// Fold an MCP startup result into `degraded`, propagating any error that
    /// is fatal regardless of the strict MCP startup setting.
    fn collect_mcp_degradation(
        result: Result<(), AgentDriverError>,
        degraded: &mut Vec<String>,
    ) -> Result<(), AgentDriverError> {
        match result {
            Ok(()) => Ok(()),
            Err(AgentDriverError::MCPStartupFailed { details }) => {
                degraded.extend(details);
                Ok(())
            }
            Err(other) => Err(other),
        }
    }

    /// Apply strict MCP startup handling to a recorded startup result.
    ///
    /// Degraded startup (`MCPStartupFailed`) is fatal only in strict mode.
    /// Otherwise the run continues without the unavailable servers: the
    /// degradation is logged and reported as a run status message.
    async fn handle_mcp_startup_result(
        result: Result<(), AgentDriverError>,
        foreground: &ModelSpawner<Self>,
    ) -> Result<(), AgentDriverError> {
        let Err(error) = result else {
            return Ok(());
        };
        let AgentDriverError::MCPStartupFailed { details } = &error else {
            return Err(error);
        };
        let details = details.join("; ");

        let strict = foreground.spawn(|me, _| me.strict_mcp_startup).await?;
        if strict {
            return Err(error);
        }

        log::warn!(
            "MCP startup degraded ({details}); continuing without the unavailable MCP servers"
        );

        // Surface the degradation on the run itself. The server currently only
        // persists status messages on terminal state transitions, so this is
        // best-effort until message-only updates are supported.
        let (task_id, ai_client) = foreground
            .spawn(|me, ctx| {
                (
                    me.task_id,
                    ServerApiProvider::as_ref(ctx).get_ai_client().clone(),
                )
            })
            .await?;
        if let Some(task_id) = task_id {
            let message = format!(
                "Warning: some MCP servers were unavailable during startup ({details}); continuing without their tools."
            );
            if let Err(err) = ai_client
                .update_agent_task(
                    task_id,
                    None,
                    None,
                    None,
                    Some(TaskStatusUpdate::message(message)),
                )
                .await
            {
                log::warn!("Failed to report MCP startup warning for task {task_id}: {err:#}");
            }
        }
        Ok(())
    }

    fn spawn_inactive_servers(
        &self,
        servers_to_start: HashSet<Uuid>,
        ctx: &mut ModelContext<Self>,
    ) {
        let templatable_mcp_manager = TemplatableMCPServerManager::handle(ctx);
        templatable_mcp_manager.update(ctx, |manager, ctx| {
            for uuid in servers_to_start {
                manager.spawn_server(uuid, ctx);
            }
        });
    }

    fn start_mcp_servers(
        &self,
        uuids: &[uuid::Uuid],
        ctx: &mut ModelContext<Self>,
    ) -> impl Future<Output = Result<(), AgentDriverError>> {
        let servers_to_start = match self.get_mcp_servers_to_start(uuids, ctx) {
            Ok(val) => val,
            Err(e) => {
                return Either::Right(future::ready(Err(e)));
            }
        };

        // If we don't need to start any servers, complete immediately.
        if servers_to_start.is_empty() {
            return Either::Right(future::ready(Ok(())));
        }

        log::info!("Starting {} MCP servers...", servers_to_start.len());

        let named_servers: HashMap<Uuid, String> = {
            let manager = TemplatableMCPServerManager::as_ref(ctx);
            servers_to_start
                .iter()
                .map(|uuid| {
                    let name = manager
                        .get_installed_server(uuid)
                        .map(|installation| installation.templatable_mcp_server().name.clone())
                        .unwrap_or_else(|| uuid.to_string());
                    (*uuid, name)
                })
                .collect()
        };
        let wait = self.wait_for_mcp_servers_started(named_servers, ctx);

        self.spawn_inactive_servers(servers_to_start, ctx);

        Either::Left(wait)
    }

    /// Start ephemeral MCP servers from inline JSON specifications.
    /// These servers are not persisted and exist only for the duration of the agent run.
    fn start_ephemeral_mcp_servers(
        &self,
        mut installations: Vec<TemplatableMCPServerInstallation>,
        ctx: &mut ModelContext<Self>,
    ) -> impl Future<Output = Result<(), AgentDriverError>> {
        if installations.is_empty() {
            return Either::Right(future::ready(Ok(())));
        }

        // Inject secrets into the ephemeral MCP server installations.
        for installation in installations.iter_mut() {
            installation.apply_secrets(&self.secrets);
        }

        log::info!("Starting {} ephemeral MCP servers...", installations.len());

        let named_servers: HashMap<Uuid, String> = installations
            .iter()
            .map(|installation| {
                (
                    installation.uuid(),
                    installation.templatable_mcp_server().name.clone(),
                )
            })
            .collect();
        let wait = self.wait_for_mcp_servers_started(named_servers, ctx);

        // Spawn the ephemeral servers.
        let templatable_mcp_manager = TemplatableMCPServerManager::handle(ctx);
        templatable_mcp_manager.update(ctx, move |manager, ctx| {
            for installation in installations {
                manager.spawn_cli_ephemeral_server(installation, ctx);
            }
        });

        Either::Left(wait)
    }

    /// Subscribe to [`FileBasedMCPManagerEvent::CloudEnvMcpScanComplete`]
    /// paths and return a receiver that fires with auto-start-requested server UUIDs once every repo
    /// reports in. Must be called **before** `prepare_environment` so no events are missed.
    fn setup_file_based_mcp_discovery(
        &self,
        expected_repos: Vec<PathBuf>,
        ctx: &mut ModelContext<Self>,
    ) -> Receiver<Vec<Uuid>> {
        let (tx, rx) = oneshot::channel::<Vec<Uuid>>();

        if expected_repos.is_empty() {
            let _ = tx.send(vec![]);
            return rx;
        }

        log::info!(
            "Waiting for {} cloud environment repo(s) to report back file-based MCP server UUIDs...",
            expected_repos.len()
        );

        let mut tx = Some(tx);
        let mut pending_repos: HashSet<PathBuf> = HashSet::from_iter(expected_repos);
        let mut collected_wait_uuids = Vec::<Uuid>::new();

        let file_based_mcp_manager = FileBasedMCPManager::handle(ctx);
        let manager_clone = file_based_mcp_manager.clone();

        ctx.subscribe_to_model(&file_based_mcp_manager, move |_me, event, ctx| {
            if let FileBasedMCPManagerEvent::CloudEnvMcpScanComplete {
                repo_path,
                wait_server_uuids,
                ..
            } = event
            {
                if pending_repos.remove(repo_path) {
                    collected_wait_uuids.extend(wait_server_uuids.iter().copied());
                    // If we've received all scan results from all cloud environment repos, send
                    // back the auto-start-requested UUIDs and begin waiting for initialization.
                    if pending_repos.is_empty() {
                        let uuids = collected_wait_uuids.clone();
                        if let Some(sender) = tx.take() {
                            log::info!(
                                "Collected {} auto-started file-based MCP server(s) from cloud environment repos",
                                uuids.len()
                            );
                            let _ = sender.send(uuids);
                        }
                        ctx.unsubscribe_from_model(&manager_clone);
                    }
                }
            }
        });

        rx
    }

    /// Wait for auto-start-requested file-based MCP servers to reach a terminal state
    /// (`Running` or `FailedToStart`). Non-fatal: always completes without returning an error.
    ///
    /// **Sequencing note:** `AgentDriver` supports only one active subscription to
    /// [`TemplatableMCPServerManager`] at a time. This function, [`Self::start_mcp_servers`],
    /// and [`Self::start_ephemeral_mcp_servers`] must therefore run sequentially, never
    /// concurrently.
    fn wait_for_file_based_mcps_running(
        &self,
        uuids: Vec<Uuid>,
        ctx: &mut ModelContext<Self>,
    ) -> impl Future<Output = ()> {
        // Filter out UUIDs that have already reached a terminal state.
        let mut pending_uuids: HashSet<Uuid> = {
            let templatable_manager = TemplatableMCPServerManager::as_ref(ctx);
            uuids
                .into_iter()
                .filter(|uuid| {
                    !matches!(
                        templatable_manager.get_server_state(*uuid),
                        Some(MCPServerState::Running) | Some(MCPServerState::FailedToStart)
                    )
                })
                .collect()
        };

        if pending_uuids.is_empty() {
            log::info!("All file-based MCP servers have reached a terminal state; proceeding");
            return Either::Right(future::ready(()));
        }

        let pending_state_details = {
            let templatable_manager = TemplatableMCPServerManager::as_ref(ctx);
            let file_based_manager = FileBasedMCPManager::as_ref(ctx);
            Arc::new(Mutex::new(
                pending_uuids
                    .iter()
                    .map(|uuid| {
                        let server_name = file_based_manager
                            .get_installation_by_uuid(*uuid)
                            .map(|installation| installation.templatable_mcp_server().name.clone())
                            .unwrap_or_else(|| "<unknown>".to_string());
                        let state = templatable_manager
                            .get_server_state(*uuid)
                            .map(|state| format!("{state:?}"))
                            .unwrap_or_else(|| "no state".to_string());
                        let error = templatable_manager
                            .get_server_error_message(*uuid)
                            .map(|message| format!(", error={message}"))
                            .unwrap_or_default();
                        (*uuid, format!("{server_name} ({uuid}): {state}{error}"))
                    })
                    .collect::<HashMap<_, _>>(),
            ))
        };
        let file_based_mcp_names = {
            let file_based_manager = FileBasedMCPManager::as_ref(ctx);
            pending_uuids
                .iter()
                .map(|uuid| {
                    let server_name = file_based_manager
                        .get_installation_by_uuid(*uuid)
                        .map(|installation| installation.templatable_mcp_server().name.clone())
                        .unwrap_or_else(|| "<unknown>".to_string());
                    (*uuid, server_name)
                })
                .collect::<HashMap<_, _>>()
        };
        log::info!(
            "Waiting for {} file-based MCP server(s) to reach a terminal state",
            pending_uuids.len()
        );

        let (tx, rx) = oneshot::channel::<()>();
        let mut tx = Some(tx);

        let templatable_manager_handle = TemplatableMCPServerManager::handle(ctx);
        let manager_clone = templatable_manager_handle.clone();
        let pending_state_details_for_subscription = Arc::clone(&pending_state_details);

        ctx.subscribe_to_model(&templatable_manager_handle, move |_me, event, ctx| {
            if let TemplatableMCPServerManagerEvent::StateChanged { uuid, state } = event {
                if !pending_uuids.contains(uuid) {
                    return;
                }
                let server_name = file_based_mcp_names
                    .get(uuid)
                    .map(String::as_str)
                    .unwrap_or("<unknown>");
                let error = TemplatableMCPServerManager::as_ref(ctx)
                    .get_server_error_message(*uuid)
                    .map(|message| format!(", error={message}"))
                    .unwrap_or_default();
                if let Ok(mut details) = pending_state_details_for_subscription.lock() {
                    details.insert(*uuid, format!("{server_name} ({uuid}): {state:?}{error}"));
                }
                match state {
                    MCPServerState::Running | MCPServerState::FailedToStart => {
                        pending_uuids.remove(uuid);
                        if let Ok(mut details) = pending_state_details_for_subscription.lock() {
                            details.remove(uuid);
                        }
                    }
                    _ => {
                        return;
                    }
                }
                if pending_uuids.is_empty() {
                    log::info!("All file-based MCP servers reached a terminal state; proceeding");
                    if let Some(sender) = tx.take() {
                        let _ = sender.send(());
                    }
                    ctx.unsubscribe_from_model(&manager_clone);
                }
            }
        });

        Either::Left(async move {
            match rx.with_timeout(MCP_SERVER_STARTUP_TIMEOUT).await {
                Ok(Ok(())) => {}
                Ok(Err(Canceled)) => {
                    log::warn!(
                        "File-based MCP server readiness subscription dropped early; proceeding"
                    );
                }
                Err(TimeoutError) => {
                    let pending_details = pending_state_details
                        .lock()
                        .map(|details| details.values().cloned().join("; "))
                        .unwrap_or_else(|_| "<unable to read pending state>".to_string());
                    log::warn!(
                        "Timed out waiting for file-based MCP servers to reach a terminal state; proceeding without. Still pending: {pending_details}"
                    );
                }
            }
        })
    }

    /// Resolve global skill specs and the GitHub repositories that should be cloned for them.
    async fn resolve_global_skills(
        foreground: &ModelSpawner<Self>,
    ) -> Result<GlobalSkillResolution, AgentDriverError> {
        if !FeatureFlag::OzPlatformSkills.is_enabled() {
            return Ok(GlobalSkillResolution {
                specs: Vec::new(),
                repos: Vec::new(),
            });
        }

        let raw_global_specs = foreground
            .spawn(|_, ctx| AuthStateProvider::as_ref(ctx).get().global_skills())
            .await?;
        let (global_specs, global_repos) = resolve_skill_repos(&raw_global_specs);
        if !global_repos.is_empty() {
            log::info!("Resolving {} global skill repo(s)", global_repos.len());
        }
        Ok(GlobalSkillResolution {
            specs: global_specs,
            repos: global_repos,
        })
    }

    /// Clone all passed-in global skill repositories.
    ///
    /// These repositories are not registered with the `DetectedRepositories`
    /// model, so that we don't automatically detect *all* skills they contain.
    /// This is important when using registry-like skill repos where loading all
    /// skills would bloat the context window.
    async fn clone_global_skill_repos(
        foreground: &ModelSpawner<Self>,
        global_skill_repos: &[GithubRepo],
    ) -> Result<(), AgentDriverError> {
        if global_skill_repos.is_empty() {
            return Ok(());
        }

        let global_skill_repos = global_skill_repos.to_vec();
        let clone_future = foreground
            .spawn(move |me, ctx| {
                let working_dir = me.working_dir.clone();
                me.terminal_driver.update(ctx, |_, ctx| {
                    let spawner = ctx.spawner();
                    async move {
                        environment::clone_repos(&global_skill_repos, &working_dir, &spawner).await
                    }
                })
            })
            .await?;

        if let Err(err) = clone_future.await {
            log::warn!("Failed to clone one or more global-skill repos: {err}");
        }

        Ok(())
    }

    /// Load skills from environment repositories.
    ///
    /// It's assumed that `prepare_environment` registers all cloned repositories
    /// with the `DetectedRepositories` model, so that we can scan for skills
    // here.
    async fn load_environment_skills(foreground: &ModelSpawner<Self>, repos: Vec<GithubRepo>) {
        if repos.is_empty() {
            log::info!("No environment repositories for skill loading");
            return;
        }
        safe_info!(
            safe: ("Loading skills from {} environment repositories", repos.len()),
            full: (
                "Loading environment skills from repositories: {}",
                repos.iter().join(", ")
            )
        );

        // Skill-scanning depends on the in-memory RepoMetadataModel index, so wait for
        // initial indexing of all repos to complete.
        let repo_index_waits = foreground
            .spawn(move |me, ctx| {
                let repo_paths: Vec<PathBuf> = repos
                    .iter()
                    .map(|repo| me.working_dir.join(&repo.repo))
                    .collect();
                let repo_metadata = RepoMetadataModel::handle(ctx);
                let mut repo_index_waits = Vec::new();
                for repo_path in &repo_paths {
                    let Some(id) = RepositoryIdentifier::try_local(repo_path) else {
                        log::warn!(
                            "Cannot wait for repository metadata indexing for non-local path {}",
                            repo_path.display()
                        );
                        continue;
                    };
                    let wait = repo_metadata.update(ctx, |repo_metadata, ctx| {
                        repo_metadata.repository_indexed(&id, ctx)
                    });
                    repo_index_waits.push((repo_path.clone(), id, wait));
                }
                (repo_paths, repo_index_waits)
            })
            .await;

        let (repo_paths, repo_index_waits) = match repo_index_waits {
            Ok(result) => result,
            Err(err) => {
                log::warn!("Failed to prepare environment skill loading: {err}");
                return;
            }
        };

        if !repo_index_waits.is_empty() {
            log::info!(
                "Waiting for repository metadata indexing before loading skills from {} repo(s)",
                repo_index_waits.len()
            );
            let (repo_index_targets, wait_futures): (Vec<_>, Vec<_>) = repo_index_waits
                .into_iter()
                .map(|(repo_path, id, wait)| ((repo_path, id), wait))
                .unzip();
            join_all(wait_futures).await;
            let repo_index_statuses = foreground
                .spawn(move |_, ctx| {
                    let repo_metadata = RepoMetadataModel::handle(ctx);
                    repo_index_targets
                        .into_iter()
                        .filter_map(|(repo_path, id)| {
                            let RepositoryIdentifier::Local(repo_id_path) = &id else {
                                return None;
                            };
                            let error = match repo_metadata.as_ref(ctx).repository_state(&id, ctx) {
                                Some(IndexedRepoState::Indexed(_)) => None,
                                Some(IndexedRepoState::Pending(_)) => Some(format!(
                                    "Repository indexing is still pending: {repo_id_path}"
                                )),
                                Some(IndexedRepoState::Failed(error)) => {
                                    Some(format!("Repository indexing failed: {error}"))
                                }
                                None => Some(format!("Repository not found: {repo_id_path}")),
                            };
                            Some((repo_path, error))
                        })
                        .collect::<Vec<_>>()
                })
                .await;

            let repo_index_statuses = match repo_index_statuses {
                Ok(repo_index_statuses) => repo_index_statuses,
                Err(err) => {
                    log::warn!("Failed to check repository indexing status: {err}");
                    Vec::new()
                }
            };
            for (repo_path, error) in repo_index_statuses {
                if let Some(err) = error {
                    log::warn!(
                        "Repository metadata indexing was not ready for skill loading in {}: {err}",
                        repo_path.display()
                    );
                }
            }
        }

        let load_skills_result = foreground
            .spawn(move |_, ctx| {
                let skills = SkillWatcher::read_local_skills_for_repos(&repo_paths, ctx);
                if !skills.is_empty() {
                    log::info!("Loaded {} environment skill(s)", skills.len());
                } else {
                    log::info!("No environment skills found");
                }
                SkillManager::handle(ctx).update(ctx, |manager, _| {
                    manager.set_cloud_environment(true);
                    manager.handle_skills_added(skills);
                });
            })
            .await;

        if let Err(err) = load_skills_result {
            log::warn!("Failed to load environment skills: {err}");
        }
    }

    /// Load explicitly requested global skills by reading directly from disk.
    async fn load_global_skills(
        foreground: &ModelSpawner<Self>,
        specs: Vec<SkillSpec>,
        repos: Vec<GithubRepo>,
    ) {
        if specs.is_empty() || repos.is_empty() {
            return;
        }
        safe_info!(
            safe: ("Loading {} global skill(s) from {} repo(s)", specs.len(), repos.len()),
            full: (
                "Loading global skills {} from repos: {}",
                specs.iter().map(|s| &s.skill_identifier).join(", "),
                repos.iter().join(", ")
            )
        );

        let load_result = foreground
            .spawn(move |me, _| {
                let mut all_skills = Vec::new();
                for repo in &repos {
                    let repo_path = me.working_dir.join(&repo.repo);
                    // Read skills from all known provider directories on disk,
                    // without depending on RepoMetadataModel.
                    let skill_dirs = SKILL_PROVIDER_DEFINITIONS
                        .iter()
                        .map(|def| repo_path.join(&def.skills_path));
                    let repo_skills = read_skills_from_directories(skill_dirs);
                    let filtered = filter_skills_by_spec(
                        &LocalOrRemotePath::Local(repo_path),
                        repo_skills,
                        &specs,
                    );
                    all_skills.extend(filtered);
                }
                all_skills
            })
            .await;

        let skills = match load_result {
            Ok(skills) => skills,
            Err(err) => {
                log::warn!("Failed to load global skills: {err}");
                return;
            }
        };

        if skills.is_empty() {
            log::info!("No global skills matched the requested specs");
            return;
        }

        log::info!("Loaded {} global skill(s)", skills.len());
        let add_result = foreground
            .spawn(move |_, ctx| {
                SkillManager::handle(ctx).update(ctx, |manager, _| {
                    manager.set_cloud_environment(true);
                    manager.handle_skills_added(skills);
                });
            })
            .await;

        if let Err(err) = add_result {
            log::warn!("Failed to add global skills to SkillManager: {err}");
        }
    }

    /// Runs the agent to completion.
    /// Driving the agent mostly requires main-thread UI framework updates, but using `async` and
    /// a `ModelSpawner` lets us express the high-level process linearly rather than in a
    /// series of callbacks and state machine updates.
    #[tracing::instrument(name = "AgentDriver::run_internal", skip_all, err, fields(tags.cloud_agent = true))]
    async fn run_internal(
        task: Task,
        foreground: ModelSpawner<Self>,
    ) -> Result<(), AgentDriverError> {
        safe_debug!(
            safe: ("Running agent driver"),
            full: ("Running agent driver for query `{:?}`", task.prompt)
        );
        let setup_events = foreground
            .spawn(|me, ctx| {
                let ai_client = ServerApiProvider::as_ref(ctx).get_ai_client().clone();
                match me.task_id {
                    Some(task_id) => {
                        SetupClientEventReporter::new(task_id, ai_client, ctx.background_executor())
                    }
                    None => {
                        log::error!("No task ID found for driver - cannot report client events");
                        SetupClientEventReporter::noop(ai_client, ctx.background_executor())
                    }
                }
            })
            .await?;

        foreground
            .spawn(|me, _| me.check_working_dir())
            .await?
            .await?;

        // IMPORTANT: Wait for the terminal session to bootstrap before starting MCP servers.
        // Some of the initializations are necessary for the MCP servers to start correctly.
        //
        // Why: MCP server startup can happen before we actually execute the agent prompt. For
        // `TransportType::CLIServer` MCPs we currently depend on `AISettings.mcp_execution_path`,
        // which is populated as part of terminal bootstrap. Waiting for the session bootstrap
        // here avoids a subtle race where MCP spawn runs with an unset PATH and then the driver
        // only fails via a timeout.
        setup_events
            .record_result(SetupStep::TerminalBootstrap, async {
                foreground
                    .spawn(|me, ctx| {
                        me.terminal_driver
                            .as_ref(ctx)
                            .wait_for_session_bootstrapped()
                    })
                    .await?
                    .await
            })
            .await?;

        // Once the terminal session is bootstrapped, perform cloud provider setup before spawning MCP servers.
        // MCP servers *may* rely on cloud provider credentials.
        setup_events
            .record_result(
                SetupStep::CloudProviderSetup,
                Self::setup_cloud_providers(&foreground),
            )
            .await?;

        // For the Oz harness only: set up MCP servers, model overrides, and profile information.
        if matches!(&task.harness, HarnessKind::Oz) {
            // Resolve MCP specs into existing server UUIDs and ephemeral installations.
            let mcp_specs = task.mcp_specs.clone();
            let (existing_uuids, ephemeral_installations) = foreground
                .spawn(move |_, _| Self::resolve_mcp_specs(&mcp_specs))
                .await??;

            // Start any requested existing MCP servers first.
            log::info!(
                "Starting {} existing and {} ephemeral MCP servers",
                existing_uuids.len(),
                ephemeral_installations.len()
            );

            let mcp_startup_result = setup_events
                .record_result(SetupStep::McpServerStartup, async {
                    // Run both startup phases even when one degrades, collecting
                    // degradation details so non-strict runs can continue with
                    // whichever servers did start.
                    let mut degraded = Vec::new();
                    if !existing_uuids.is_empty() {
                        let result = foreground
                            .spawn(move |me, ctx| me.start_mcp_servers(&existing_uuids, ctx))
                            .await?
                            .await;
                        Self::collect_mcp_degradation(result, &mut degraded)?;
                    }
                    // Start ephemeral MCP servers from inline JSON specs.
                    if !ephemeral_installations.is_empty() {
                        let result = foreground
                            .spawn(move |me, ctx| {
                                me.start_ephemeral_mcp_servers(ephemeral_installations, ctx)
                            })
                            .await?
                            .await;
                        Self::collect_mcp_degradation(result, &mut degraded)?;
                    }
                    if degraded.is_empty() {
                        Ok(())
                    } else {
                        Err(AgentDriverError::MCPStartupFailed { details: degraded })
                    }
                })
                .await;
            Self::handle_mcp_startup_result(mcp_startup_result, &foreground).await?;
            let profile = task.profile.clone();
            setup_events
                .record_result(SetupStep::AgentProfileConfiguration, async {
                    foreground
                        .spawn(move |me, ctx| me.configure_terminal(profile, ctx))
                        .await??;
                    Ok::<(), AgentDriverError>(())
                })
                .await?;

            if let Some(model_id) = task.model.clone() {
                foreground
                    .spawn(move |me, ctx| me.set_base_model_override(model_id, ctx))
                    .await??;
            }

            let profile_mcp_startup_result = setup_events
                .record_result(SetupStep::ProfileMcpServerStartup, async {
                    foreground
                        .spawn(|me, ctx| me.start_profile_mcp_servers(ctx))
                        .await?
                        .await
                })
                .await;
            Self::handle_mcp_startup_result(profile_mcp_startup_result, &foreground).await?;
        }

        // For all harnesses: wait for the shared session and prepare the environment.
        setup_events
            .record_result(SetupStep::SharedSessionEstablishment, async {
                foreground
                    .spawn(|me, ctx| {
                        me.terminal_driver
                            .update(ctx, |driver, _| driver.wait_for_session_shared())
                    })
                    .await?
                    .await
            })
            .await?;
        let global_skill_resolution = setup_events
            .record_result(
                SetupStep::GlobalSkillResolution,
                Self::resolve_global_skills(&foreground),
            )
            .await?;
        // Clone global skill repos before environment prep can change the
        // terminal's cwd into a single environment repo.
        // We do this for all harnesses, so that the skills *may* be discovered by third-party
        // harnesses if appropriate.
        setup_events
            .record_result(
                SetupStep::GlobalSkillRepoClone,
                Self::clone_global_skill_repos(&foreground, &global_skill_resolution.repos),
            )
            .await?;
        let mut environment_skill_repos = Vec::new();

        let environment_opt = foreground.spawn(|me, _| me.environment.clone()).await?;

        if let Some(environment) = environment_opt {
            log::info!("Loading environment...");
            let environment_github_repos = environment.github_repos.clone();
            environment_skill_repos = environment_github_repos.clone();

            // Subscribe to file-based MCP discovery BEFORE prepare_environment triggers the
            // pipeline so no CloudEnvMcpScanComplete events are missed.
            //
            // File-based MCP discovery is Oz-only.
            // TODO(REMOTE-1345): handle MCP setup for third-party harnesses.
            let file_based_discovery_rx = match &task.harness {
                HarnessKind::Oz => {
                    let github_repos = environment_github_repos.clone();
                    Some(
                        foreground
                            .spawn(move |me, ctx| {
                                let expected_repo_paths: Vec<PathBuf> = github_repos
                                    .iter()
                                    .map(|repo| me.working_dir.join(&repo.repo))
                                    .collect();
                                me.setup_file_based_mcp_discovery(expected_repo_paths, ctx)
                            })
                            .await?,
                    )
                }
                HarnessKind::ThirdParty(_) | HarnessKind::Unsupported(_) => None,
            };

            let harness = task.harness.harness();
            let setup_events_for_environment = setup_events.clone();
            foreground
                .spawn(move |me, ctx| {
                    let working_dir = me.working_dir.clone();
                    me.terminal_driver.update(ctx, |_, ctx| {
                        environment::prepare_environment(
                            environment,
                            working_dir,
                            false, /* is_sandbox */
                            harness,
                            setup_events_for_environment,
                            ctx,
                        )
                    })
                })
                .await?
                .await
                .map_err(AgentDriverError::from)?;

            if let Some(file_based_discovery_rx) = file_based_discovery_rx {
                // Await discovery: collect UUIDs of file-based MCP servers that were auto-started
                // while scanning cloned repos.
                let wait_uuids = setup_events
                    .record_value(SetupStep::FileBasedMcpDiscovery, async {
                        match file_based_discovery_rx
                            .with_timeout(MCP_SERVER_STARTUP_TIMEOUT)
                            .await
                        {
                            Ok(Ok(uuids)) => uuids,
                            Ok(Err(Canceled)) => {
                                log::warn!(
                                    "File-based MCP discovery subscription dropped early; proceeding without"
                                );
                                vec![]
                            }
                            Err(TimeoutError) => {
                                log::warn!(
                                    "Timed out waiting for file-based MCP servers to be parsed; proceeding without"
                                );
                                vec![]
                            }
                        }
                    })
                    .await;

                // Wait for auto-started servers to reach Running (non-fatal: always unblocks).
                if !wait_uuids.is_empty() {
                    log::info!(
                        "Checking readiness for {} auto-started file-based MCP server(s)",
                        wait_uuids.len()
                    );
                    setup_events
                        .record_result(SetupStep::FileBasedMcpReadiness, async {
                            foreground
                                .spawn(move |me, ctx| {
                                    me.wait_for_file_based_mcps_running(wait_uuids, ctx)
                                })
                                .await?
                                .await;
                            Ok::<(), AgentDriverError>(())
                        })
                        .await?;
                }
            }
        }

        // Skill loading is Oz-only; third-party harnesses have their own skill systems.
        if matches!(&task.harness, HarnessKind::Oz) {
            // Load skills from repos synchronously so the initial message includes them.
            // File trees are ready after prepare_environment and global skill repo cloning above.
            let GlobalSkillResolution {
                specs: global_skill_specs,
                repos: global_skill_repos,
            } = global_skill_resolution;
            setup_events
                .record_value(
                    SetupStep::EnvironmentSkillLoading,
                    Self::load_environment_skills(&foreground, environment_skill_repos),
                )
                .await;
            setup_events
                .record_value(
                    SetupStep::GlobalSkillLoading,
                    Self::load_global_skills(&foreground, global_skill_specs, global_skill_repos),
                )
                .await;
        }

        let (task_id_for_refresh, ai_client_for_refresh) = foreground
            .spawn(|me, ctx| {
                let task_id = if FeatureFlag::GitCredentialRefresh.is_enabled() {
                    me.task_id.map(|id| id.to_string())
                } else {
                    None
                };
                let ai_client = ServerApiProvider::as_ref(ctx).get_ai_client().clone();
                (task_id, ai_client)
            })
            .await?;

        // Run the harness with a prompt, racing it against an infinite git-credentials
        // refresh loop. The refresh future never resolves on its own — it is dropped
        // automatically when `select!` resolves on the harness result.
        match task.harness {
            HarnessKind::Oz => {
                let status_rx = foreground
                    .spawn(move |me, ctx| me.execute_run(task.prompt, ctx))
                    .await?;

                let conversation_status = if let Some(task_id) = task_id_for_refresh {
                    let refresh =
                        git_credentials::refresh_loop(task_id, ai_client_for_refresh).fuse();
                    futures::pin_mut!(refresh);
                    futures::select! {
                        result = status_rx.fuse() => result.map_err(|_| {
                            log::error!("Subscription dropped before agent finished");
                            AgentDriverError::InvalidRuntimeState
                        })?,
                        _ = refresh => unreachable!("git credentials refresh loop resolved unexpectedly"),
                    }
                } else {
                    status_rx.await.map_err(|_| {
                        log::error!("Subscription dropped before agent finished");
                        AgentDriverError::InvalidRuntimeState
                    })?
                };

                log::info!(
                    "Ambient agent Oz lifecycle: event=run_exit_received idle_on_complete_elapsed_or_not_configured=true next=terminal_teardown_after_flush"
                );

                // Pause before returning to make sure that all conversation events are transmitted before the session is closed.
                // TODO: This is a bit of a bandaid fix, and it would be better if we explicitly waited for the session to end before terminating.
                // The way we could do that is through having the driver wait for all in-flight streams to be finished before terminating
                // and then call stop_sharing_session when they're done. To know when streams are finished, we would need to modify start_ordered_terminal_events_listener
                // to send a message when the streams are finished, flushed, and the websocket is disconnected. For now, we'll just sleep for a second, as this seems
                // to be enough time for the streams to be finished and the events to be flushed.
                warpui::r#async::Timer::after(Duration::from_secs(1)).await;

                conversation_status.into_result()
            }
            HarnessKind::ThirdParty(harness) => {
                let (harness_exit_rx, runner) = setup_events
                    .record_result(SetupStep::ThirdPartyHarnessPreparation, async {
                        let harness_exit_rx =
                            Self::setup_harness(harness.as_ref(), &foreground).await?;
                        let runner = Self::prepare_harness(
                            &task.prompt,
                            &task.mcp_specs,
                            harness.as_ref(),
                            &foreground,
                        )
                        .await?;

                        Self::run_preflight_checks(harness.as_ref(), &foreground).await?;
                        Ok::<_, AgentDriverError>((harness_exit_rx, runner))
                    })
                    .await?;
                let runtime_error_patterns = harness.runtime_error_patterns();

                if let Some(task_id) = task_id_for_refresh {
                    let harness_fut = Self::run_harness(
                        runner,
                        runtime_error_patterns,
                        &foreground,
                        harness_exit_rx,
                        &setup_events,
                    )
                    .fuse();
                    let refresh =
                        git_credentials::refresh_loop(task_id, ai_client_for_refresh).fuse();
                    futures::pin_mut!(harness_fut, refresh);
                    futures::select! {
                        result = harness_fut => result,
                        _ = refresh => unreachable!("git credentials refresh loop resolved unexpectedly"),
                    }
                } else {
                    Self::run_harness(
                        runner,
                        runtime_error_patterns,
                        &foreground,
                        harness_exit_rx,
                        &setup_events,
                    )
                    .await
                }
            }
            HarnessKind::Unsupported(harness) => Err(AgentDriverError::HarnessSetupFailed {
                harness: harness.to_string(),
                reason: format!(
                    "The {harness} harness is only supported for local child agent launches."
                ),
            }),
        }
    }

    /// Run the authentication preflight check for a third-party harness.
    ///
    /// Uses `execute_command` so the check appears as a collapsible block in
    /// the shared session UI, mirroring how environment setup commands
    /// surface.
    async fn run_preflight_checks(
        harness: &dyn ThirdPartyHarness,
        foreground: &ModelSpawner<Self>,
    ) -> Result<(), AgentDriverError> {
        let harness_name = harness.cli_agent().command_prefix().to_owned();

        if let Some(cmd) = harness.auth_check_command() {
            log::info!("Running auth check for {harness_name}: {cmd}");
            Self::run_single_preflight(&cmd, &harness_name, foreground).await?;
        }

        Ok(())
    }

    /// Run a single preflight check command and return an error if it fails.
    async fn run_single_preflight(
        command: &str,
        harness_name: &str,
        foreground: &ModelSpawner<Self>,
    ) -> Result<(), AgentDriverError> {
        let cmd = command.to_owned();
        let start_future = foreground
            .spawn(move |me, ctx| {
                me.terminal_driver
                    .update(ctx, |driver, ctx| driver.execute_command(&cmd, ctx))
            })
            .await??;

        let command_handle = start_future.await?;
        let block_id = command_handle.block_id().clone();

        let exit_code = match command_handle.with_timeout(PREFLIGHT_CHECK_TIMEOUT).await {
            Err(TimeoutError) => {
                log::error!("Preflight auth check timed out for {harness_name}");
                return Err(AgentDriverError::HarnessAuthCheckFailed {
                    harness: harness_name.to_owned(),
                    detail: "command timed out".to_owned(),
                });
            }
            Ok(result) => result?,
        };

        if !exit_code.was_successful() {
            let output_text = Self::fetch_preflight_block_output(&block_id, foreground).await;
            let detail = if output_text.is_empty() {
                format!("exit code {}", exit_code.value())
            } else {
                format!("exit code {}: {}", exit_code.value(), output_text)
            };
            safe_error!(
                safe: (
                    "Preflight auth check failed for {harness_name} (exit code {})",
                    exit_code.value()
                ),
                full: ("Preflight auth check failed for {harness_name}. {detail}")
            );
            return Err(AgentDriverError::HarnessAuthCheckFailed {
                harness: harness_name.to_owned(),
                detail,
            });
        }

        log::info!("Preflight auth check passed for {harness_name}");
        Ok(())
    }

    async fn fetch_preflight_block_output(
        block_id: &BlockId,
        foreground: &ModelSpawner<Self>,
    ) -> String {
        let block_id = block_id.clone();
        let plaintext = foreground
            .spawn(move |me, ctx| {
                me.terminal_driver
                    .as_ref(ctx)
                    .block_output_plaintext(&block_id, ctx)
            })
            .await;
        match plaintext {
            Ok(Some(text)) => text.trim().to_owned(),
            Ok(None) | Err(_) => String::new(),
        }
    }

    /// Sets up the third-party harness by subscribing to CLI session events and
    /// installing the Warp plugin and platform plugin, if applicable.
    ///
    /// Returns a oneshot receiver that fires when the harness should exit
    /// (either immediately on completion or after the idle-on-complete timeout).
    async fn setup_harness(
        harness: &dyn ThirdPartyHarness,
        foreground: &ModelSpawner<Self>,
    ) -> Result<oneshot::Receiver<()>, AgentDriverError> {
        let (exit_tx, exit_rx) = oneshot::channel();
        let harness_exit = IdleTimeoutSender::new(exit_tx);

        // Subscribe to CLI agent session events so we can update the task
        // state as the harness emits stop/blocked notifications.
        foreground
            .spawn(move |me, ctx| me.subscribe_to_cli_agent_session_events(harness_exit, ctx))
            .await?;

        // Install plugins before running the harness command.
        Self::setup_harness_plugins(harness).await?;

        Ok(exit_rx)
    }

    async fn setup_harness_plugins(
        harness: &dyn ThirdPartyHarness,
    ) -> Result<(), AgentDriverError> {
        let harness_name = harness.cli_agent().command_prefix();
        let requires_platform_plugin = harness.requires_verified_platform_plugin();
        let Some(manager) = plugin_manager_for(harness.cli_agent()) else {
            if requires_platform_plugin {
                return Err(Self::required_platform_plugin_error(
                    harness_name,
                    "Required platform plugin manager is unavailable",
                ));
            }
            return Ok(());
        };

        Self::setup_notification_plugin(manager.as_ref()).await;
        Self::setup_platform_plugin(harness_name, manager.as_ref(), requires_platform_plugin).await
    }

    async fn setup_notification_plugin(manager: &dyn CliAgentPluginManager) {
        if !manager.can_auto_install() {
            return;
        }
        if manager.needs_update() {
            if let Err(e) = manager.update().await {
                log::warn!("Plugin update failed (continuing): {e}");
            }
        } else if !manager.is_installed() {
            if let Err(e) = manager.install().await {
                log::warn!("Plugin installation failed (continuing): {e}");
            }
        }
    }

    async fn setup_platform_plugin(
        harness_name: &str,
        manager: &dyn CliAgentPluginManager,
        required: bool,
    ) -> Result<(), AgentDriverError> {
        if manager.platform_plugin_needs_update() {
            if let Err(e) = manager.update_platform_plugin().await {
                if required {
                    return Err(Self::required_platform_plugin_error(
                        harness_name,
                        format!("Required platform plugin update failed: {e}"),
                    ));
                }
                log::warn!("Platform plugin update failed (continuing): {e}");
            }
        } else if !manager.is_platform_plugin_installed() {
            if let Err(e) = manager.install_platform_plugin().await {
                if required {
                    return Err(Self::required_platform_plugin_error(
                        harness_name,
                        format!("Required platform plugin installation failed: {e}"),
                    ));
                }
                log::warn!("Platform plugin installation failed (continuing): {e}");
            }
        }

        if required {
            Self::verify_required_platform_plugin(harness_name, manager)?;
        }
        Ok(())
    }

    fn verify_required_platform_plugin(
        harness_name: &str,
        manager: &dyn CliAgentPluginManager,
    ) -> Result<(), AgentDriverError> {
        if !manager.is_platform_plugin_installed() {
            return Err(Self::required_platform_plugin_error(
                harness_name,
                "Required platform plugin is not installed",
            ));
        }
        if manager.platform_plugin_needs_update() {
            return Err(Self::required_platform_plugin_error(
                harness_name,
                "Required platform plugin is below the minimum supported version",
            ));
        }
        Ok(())
    }

    fn required_platform_plugin_error(
        harness: &str,
        reason: impl Into<String>,
    ) -> AgentDriverError {
        AgentDriverError::HarnessSetupFailed {
            harness: harness.to_owned(),
            reason: reason.into(),
        }
    }

    /// Configure a third-party harness for execution. This will set `self.harness` and
    /// return a handle to the harness runner.
    async fn prepare_harness(
        prompt: &AgentRunPrompt,
        mcp_specs: &[MCPSpec],
        harness: &dyn ThirdPartyHarness,
        foreground: &ModelSpawner<Self>,
    ) -> Result<Arc<dyn harness::HarnessRunner>, AgentDriverError> {
        let (working_dir, task_id, server_api, terminal_driver) = foreground
            .spawn(|me, ctx| {
                if me.harness.is_some() {
                    log::error!(
                        "Attempted to prepare a third-party harness, but one was already configured"
                    );
                    return Err(AgentDriverError::InvalidRuntimeState);
                }

                Ok((
                    me.working_dir.clone(),
                    me.task_id,
                    ServerApiProvider::as_ref(ctx).get(),
                    me.terminal_driver.clone(),
                ))
            })
            .await
            .map_err(|_| AgentDriverError::InvalidRuntimeState)
            .flatten()?;

        let (prompt_text, system_prompt, resumption_prompt, server_context): (
            Cow<'_, str>,
            Option<String>,
            Option<String>,
            Option<String>,
        ) = match prompt {
            AgentRunPrompt::Local(text) => (Cow::Borrowed(text), None, None, None),
            AgentRunPrompt::ServerSide {
                skill,
                attachments_dir,
            } => {
                let skill = skill
                    .as_ref()
                    .map(|parsed_skill| ResolvePromptAttachedSkill {
                        name: parsed_skill.name.clone(),
                        content: parsed_skill.content.clone(),
                        path: Some(parsed_skill.path.display_path()),
                    });
                let request = ResolvePromptRequest {
                    skill,
                    attachments_dir: attachments_dir.clone(),
                };
                let resolved = server_api
                    .resolve_prompt(request)
                    .await
                    .map_err(AgentDriverError::PromptResolutionFailed)?;
                (
                    Cow::Owned(resolved.prompt),
                    resolved.system_prompt,
                    resolved.resumption_prompt,
                    resolved.context,
                )
            }
        };

        let (secrets, third_party_harness_model_config) = foreground
            .spawn(|me, _| {
                (
                    Arc::clone(&me.secrets),
                    me.third_party_harness_model_config.clone(),
                )
            })
            .await
            .map_err(|_| AgentDriverError::InvalidRuntimeState)?;

        // Clone the raw secrets before the MCP closure consumes the Arc, so the
        // harness can read structured fields (e.g. OpenAI `base_url`) directly.
        let secrets_for_harness = Arc::clone(&secrets);

        // Resolve MCP specs into harness-native JSON format.
        let mcp_specs = mcp_specs.to_vec();
        let resolved_mcp_servers = foreground
            .spawn(move |_, ctx| Self::resolve_mcp_specs_to_json(&mcp_specs, &secrets, ctx))
            .await
            .map_err(|_| AgentDriverError::InvalidRuntimeState)?;
        let resolved_mcp_servers = resolved_mcp_servers?;
        if !resolved_mcp_servers.is_empty() {
            log::info!(
                "Resolved {} MCP server(s) for third-party harness",
                resolved_mcp_servers.len()
            );
        }

        let resolved_env_vars = foreground
            .spawn(|me, _| Arc::clone(&me.resolved_env_vars))
            .await
            .map_err(|_| AgentDriverError::InvalidRuntimeState)?;
        let resume = foreground
            .spawn(|me, _| me.resume_payload.take())
            .await
            .map_err(|_| AgentDriverError::InvalidRuntimeState)?;

        let runner: Arc<dyn HarnessRunner> = harness
            .build_runner(
                prompt_text.as_ref(),
                system_prompt.as_deref(),
                resumption_prompt.as_deref(),
                server_context.as_deref(),
                &working_dir,
                task_id,
                server_api,
                terminal_driver,
                resume,
                &resolved_env_vars,
                &secrets_for_harness,
                &resolved_mcp_servers,
                third_party_harness_model_config.as_ref(),
            )?
            .into();

        let stored_runner = runner.clone();
        foreground
            .spawn(move |me, _| me.harness = Some(stored_runner))
            .await?;

        Ok(runner)
    }

    /// Execute a configured external harness in the terminal.
    ///
    /// The `harness_exit_rx` oneshot fires when the subscription determines it's
    /// time to exit (either immediately on completion or after the idle timeout).
    ///
    /// While the harness runs, a background scanner watches its block for
    /// known runtime failure substrings (e.g. invalid API key, exhausted
    /// credits). If one is detected we send `/exit` to the harness and
    /// synthesize a [`AgentDriverError::HarnessRuntimeFailureDetected`]
    /// failure, which `report_driver_error` reports to the server with the
    /// same `AuthenticationRequired` error code used by the auth preflight.
    async fn run_harness(
        runner: Arc<dyn harness::HarnessRunner>,
        runtime_error_patterns: &'static [&'static str],
        foreground: &ModelSpawner<Self>,
        harness_exit_rx: oneshot::Receiver<()>,
        setup_events: &SetupClientEventReporter,
    ) -> Result<(), AgentDriverError> {
        let harness_name = runner.harness_name().to_owned();

        // Start the third-party harness.
        let command_handle = runner.start(foreground, setup_events).await?;
        let block_id = command_handle.block_id().clone();
        let mut command_handle = command_handle.fuse();
        let mut harness_exit_rx = harness_exit_rx.fuse();

        let scanner_fut = harness_output_monitor::watch_block_for_errors(
            block_id,
            runtime_error_patterns,
            foreground,
        )
        .fuse();
        futures::pin_mut!(scanner_fut);

        // Detected runtime error, if any. Promoted to the final return
        // value below after final-save + cleanup run.
        let mut detected_runtime_failure: Option<harness_output_monitor::DetectedHarnessError> =
            None;

        // Periodically save the conversation while the command is running and handle
        // exiting gracefully once the idle timeout elapses.
        let command_result = loop {
            futures::select! {
                exit_code = command_handle => break exit_code,
                _ = warpui::r#async::Timer::after(HARNESS_SAVE_INTERVAL).fuse() => {
                    log::debug!("Triggering periodic save of harness conversation data");
                    report_if_error!(runner
                        .save_conversation(SavePoint::Periodic, foreground)
                        .await
                        .context("Failed to save harness conversation (periodic)"));
                }
                _ = harness_exit_rx => {
                    log::debug!("Requesting harness exit");
                    report_if_error!(runner
                        .exit(foreground)
                        .await
                        .context("Failed to exit harness"));
                }
                detected = scanner_fut => {
                    if let Some(error) = detected {
                        log::warn!(
                            "Runtime failure detected for {harness_name}: pattern={}, excerpt={}",
                            error.pattern,
                            error.excerpt,
                        );
                        let telemetry_harness = harness_name.clone();
                        let telemetry_pattern = error.pattern.clone();
                        let _ = foreground
                            .spawn(move |_, ctx| {
                                use warp_core::telemetry::TelemetryEvent as _;
                                let event =
                                    ThirdPartyHarnessTelemetryEvent::RuntimeErrorDetected {
                                        harness: telemetry_harness,
                                        pattern: telemetry_pattern,
                                    };
                                send_telemetry_from_app_ctx!(event, ctx);
                            })
                            .await;
                        let session_status = foreground
                            .spawn(|me, ctx| {
                                let view_id =
                                    me.terminal_driver.as_ref(ctx).terminal_view().id();
                                CLIAgentSessionsModel::handle(ctx)
                                    .as_ref(ctx)
                                    .session(view_id)
                                    .map(|session| session.status.clone())
                            })
                            .await
                            .ok()
                            .flatten();
                        if harness_output_monitor::should_suppress_runtime_failure(
                            session_status.as_ref(),
                        ) {
                            log::info!(
                                "Ignoring runtime failure for {harness_name}: \
                                 session already marked Success (pattern={}, excerpt={})",
                                error.pattern,
                                error.excerpt,
                            );
                        } else {
                            report_if_error!(runner
                                .exit(foreground)
                                .await
                                .context(
                                    "Failed to exit harness after runtime failure detection",
                                ));
                            detected_runtime_failure = Some(error);
                        }
                    }
                    // When the schedule exhausts without a hit, the `Fuse`
                    // wrapper makes this branch stay Pending forever, so
                    // we don't busy-loop.
                }
            }
        };

        // Final save after the command finishes.
        log::debug!("Triggering final save of harness conversation data");
        let final_save_succeeded = match runner
            .save_conversation(SavePoint::Final, foreground)
            .await
            .context("Failed to save harness conversation (final)")
        {
            Ok(()) => true,
            Err(err) => {
                report_error!(err);
                false
            }
        };
        let cleanup_disposition = if final_save_succeeded
            && detected_runtime_failure.is_none()
            && matches!(command_result.as_ref(), Ok(exit_code) if exit_code.was_successful())
        {
            HarnessCleanupDisposition::PreserveResumptionStateIfSupported
        } else {
            HarnessCleanupDisposition::DropResumptionState
        };
        if let Err(err) = runner
            .cleanup(cleanup_disposition, foreground)
            .await
            .context("Failed to clean up harness runtime state")
        {
            report_error!(err);
        }

        // A runtime failure detected mid-run takes precedence over the
        // harness's own exit code: surface the actionable detail rather
        // than a generic "exit code N".
        if let Some(error) = detected_runtime_failure {
            return Err(AgentDriverError::HarnessRuntimeFailureDetected {
                harness: harness_name,
                pattern: error.pattern,
                excerpt: error.excerpt,
            });
        }

        let exit_code = command_result?;
        log::debug!("Agent harness exited with status {exit_code}");

        if exit_code.was_successful() {
            Ok(())
        } else {
            Err(AgentDriverError::HarnessCommandFailed {
                exit_code: exit_code.value(),
            })
        }
    }

    /// Configure the active terminal session with the specified profile.
    fn configure_terminal(
        &self,
        profile: Option<String>,
        ctx: &mut ModelContext<Self>,
    ) -> Result<(), AgentDriverError> {
        let terminal_id = self.terminal_driver.as_ref(ctx).terminal_view().id();

        if let Some(profile) = profile {
            let server_id = ServerId::try_from(profile.as_str())
                .map_err(|_| AgentDriverError::ProfileError(profile.clone()))?;
            let sync_id = SyncId::ServerId(server_id);
            AIExecutionProfilesModel::handle(ctx).update(ctx, |model, ctx| {
                if let Some(profile_id) = model.get_profile_id_by_sync_id(&sync_id) {
                    model.set_active_profile(terminal_id, profile_id, ctx);
                } else {
                    return Err(AgentDriverError::ProfileError(profile.clone()));
                }
                Ok(())
            })?;
        }

        Ok(())
    }

    fn set_base_model_override(
        &self,
        model_id: LLMId,
        ctx: &mut ModelContext<Self>,
    ) -> Result<(), AgentDriverError> {
        let terminal_view_id = self.terminal_driver.as_ref(ctx).terminal_view().id();
        log::info!("Selecting base agent model {model_id} (from agent driver)");

        LLMPreferences::handle(ctx).update(ctx, |preferences, ctx| {
            preferences.update_preferred_agent_mode_llm(&model_id, terminal_view_id, ctx);
        });
        Ok(())
    }

    /// Execute an AI run in the terminal session and wait for it to complete.
    ///
    /// Conversation output is streamed as it's available.
    fn execute_run(
        &self,
        task_prompt: AgentRunPrompt,
        ctx: &mut ModelContext<Self>,
    ) -> Receiver<SDKConversationOutputStatus> {
        // Create a oneshot channel to signal task completion.
        let (tx, rx) = oneshot::channel();
        let run_exit = IdleTimeoutSender::new(tx);
        let restored_conversation_id = self.restored_conversation_id;

        // ServerSide prompts enter the agent view and emit
        // `CloudModeSetupPhaseEnded` to tear down the Cloud Mode Setup V2 chip.
        // (Local prompts have no cloud setup phase; they enter the view with
        // the user prompt below.)
        //
        // When `skip_initial_turn` is set, also schedule the deferred `Success`
        // now so the run isn't stuck waiting for a turn that will never arrive.
        // The `AppendedExchange` handler below cancels this timer if a follow-up
        // shows up, keeping the run alive long enough to handle the new turn.
        if matches!(&task_prompt, AgentRunPrompt::ServerSide { .. }) {
            self.terminal_driver.update(ctx, |td, ctx| {
                td.with_terminal_view(ctx, |terminal, ctx| {
                    if FeatureFlag::AgentView.is_enabled() {
                        terminal.enter_agent_view(
                            None,
                            restored_conversation_id,
                            AgentViewEntryOrigin::Cli,
                            ctx,
                        );
                    }
                    terminal
                        .model
                        .lock()
                        .send_cloud_mode_setup_phase_ended_for_shared_session();
                })
            });
            if self.skip_initial_turn {
                run_exit.complete_with_optional_idle(
                    self.idle_on_complete,
                    SDKConversationOutputStatus::Success,
                );
            }
        }

        // Subscribe before the conversation starts.
        let history_model_handle = BlocklistAIHistoryModel::handle(ctx);
        let terminal_id = self.terminal_driver.as_ref(ctx).terminal_view().id();
        let mut written_conversation_id = false;

        // Create shared storage for the conversation ID
        let conversation_id_cell = Arc::new(Mutex::new(Option::<String>::None));
        let conversation_id_cell_for_handler = Arc::clone(&conversation_id_cell);

        // Get the server API from context
        let server_api = ServerApiProvider::as_ref(ctx).get_ai_client();
        let server_api_for_conversation_update = server_api.clone();
        let task_id_for_conversation_update = self.task_id;

        ctx.subscribe_to_model(&history_model_handle, move |me, event, ctx| {
            if event.terminal_view_id().is_some_and(|id| id != terminal_id) {
                return;
            }

            // Fresh runs learn their conversation_id via
            // `ConversationServerTokenAssigned`; resumed runs already
            // registered in `new` (and so skip this branch).
            if me.run_conversation_id.is_none() {
                if let BlocklistAIHistoryEvent::ConversationServerTokenAssigned {
                    conversation_id,
                    ..
                } = event
                {
                    me.run_conversation_id = Some(*conversation_id);
                    stamp_parent_agent_id_if_some(
                        *conversation_id,
                        me.parent_run_id.as_deref(),
                        ctx,
                    );
                    register_agent_event_consumer(*conversation_id, ctx.model_id(), ctx);
                }
            }

            match event {
                BlocklistAIHistoryEvent::UpdatedTodoList { .. } => {
                    // TODO: Log TODO list updates.
                }
                BlocklistAIHistoryEvent::AppendedExchange {
                    exchange_id,
                    conversation_id,
                    ..
                } => {
                    let Some(conversation) = BlocklistAIHistoryModel::as_ref(ctx)
                        .conversation(conversation_id)
                    else {
                        log::warn!("Invalid conversation ID: {conversation_id:?}");
                        return;
                    };

                    let Some(exchange) = conversation.exchange_with_id(*exchange_id) else {
                        log::warn!("Invalid exchange ID: {exchange_id:?}");
                        return;
                    };

                    // When a new exchange is appended, we should already have its inputs available.
                    report_if_error!(me
                        .write_exchange_inputs(exchange)
                        .context("Failed to write exchange inputs"));

                    // Forward any successful file-edit paths from this exchange's inputs to the
                    // snapshot declarations writer so the end-of-run upload covers files written
                    // outside any declared repo.
                    if let Some(writer) = me.snapshot_file_writer.as_ref() {
                        let mut paths = Vec::new();
                        for input in &exchange.input {
                            if let AIAgentInput::ActionResult { result, .. } = input {
                                if let AIAgentActionResultType::RequestFileEdits(
                                    RequestFileEditsResult::Success { updated_files, .. },
                                ) = &result.result
                                {
                                    for updated in updated_files {
                                        paths.push(updated.file_context.file_name.clone());
                                    }
                                }
                            }
                        }
                        writer.append(paths);
                    }

                    // Reset the idle timer only if we've already scheduled one.
                    // This handles the case where a follow-up query creates new exchanges after
                    // the conversation has finished and an idle timer was set.
                    run_exit.cancel_idle_timeout();
                }
                BlocklistAIHistoryEvent::UpdatedStreamingExchange {
                    exchange_id,
                    conversation_id,
                    ..
                } => {
                    // Get conversation data first to avoid borrowing conflicts
                    let history_model = BlocklistAIHistoryModel::handle(ctx);
                    let conversation_data = history_model.as_ref(ctx).conversation(conversation_id)
                        .and_then(|conv| {
                            let token = conv.server_conversation_token().map(|t| t.as_str().to_string());
                            let exchange = conv.exchange_with_id(*exchange_id)?;
                            Some((token, exchange))
                        });
                    let Some((token_opt, exchange)) = conversation_data else {
                        log::warn!("Invalid conversation or exchange ID: {conversation_id:?}, {exchange_id:?}");
                        return;
                    };

                    // Track whether we should spawn an async task to update the server.
                    let mut pending_conversation_update: Option<String> = None;

                    if !written_conversation_id {
                        if let Some(token) = token_opt {
                            report_if_error!(output::with_stdout_buffered(|buf| match me.output_format {
                                OutputFormat::Json | OutputFormat::Ndjson => output::json::conversation_started(&token, buf),
                                OutputFormat::Text | OutputFormat::Pretty => output::text::conversation_started(&token, buf),
                            }).context("Failed to write conversation ID"));
                            written_conversation_id = true;

                            // Store the server conversation token and record that we should update the task
                            if let Ok(mut guard) = conversation_id_cell_for_handler.lock() {
                                *guard = Some(token.clone());

                                if task_id_for_conversation_update.is_some() {
                                    pending_conversation_update = Some(token);
                                }
                            }
                        }
                    }

                    // Once the outputs are fully streamed from the server, write them to stdout.
                    if exchange.output_status.is_finished() {
                        report_if_error!(me
                            .write_exchange_output(exchange)
                            .context("Failed to write exchange output"));
                    }

                    // Perform task update after all immutable borrows end
                    if let (Some(task_id), Some(conversation_id_str)) = (
                        task_id_for_conversation_update,
                        pending_conversation_update,
                    ) {
                        let server_api = server_api_for_conversation_update.clone();
                        ctx.spawn(
                            async move {
                                if let Err(e) = server_api
                                    .update_agent_task(
                                        task_id,
                                        None, // Don't change state, just update conversation ID
                                        None, // Don't update session_id from CLI context
                                        Some(conversation_id_str),
                                        None,
                                    )
                                    .await
                                {
                                    log::error!("Failed to update agent task with conversation ID: {e}");
                                }
                            },
                            |_, _, _| {},
                        );
                    }
                }

                BlocklistAIHistoryEvent::UpdatedConversationStatus { terminal_view_id: conversation_terminal_id, conversation_id, .. } => {
                    if *conversation_terminal_id != terminal_id {
                        return;
                    }
                    let history_model = BlocklistAIHistoryModel::as_ref(ctx);
                    let Some(conversation) = history_model.conversation(conversation_id) else {
                        log::warn!("No active conversation for terminal view {conversation_terminal_id} with id {conversation_id}");
                        return;
                    };

                    if conversation.status().is_in_progress() {
                        // Conversation resumed or a new one started; cancel any
                        // pending idle timeout.
                        log::info!(
                            "Ambient agent idle lifecycle: event=idle_timeout_cancel_requested task_id={:?} terminal_view_id={terminal_id:?} trigger=conversation_in_progress",
                            me.task_id
                        );
                        run_exit.cancel_idle_timeout();
                        return;
                    }

                    // wait_for_events keeps the run alive via the
                    // action_model's running_actions; the executor owns
                    // the watchdog. Don't resolve run_exit.
                    if conversation.status().is_waiting_for_events() {
                        return;
                    }

                    // Conversation is no longer in progress. Handle completion based on the result.
                    if let Some(conversation_status) =
                         conversation_output_status_from_conversation(conversation)
                    {
                        let output_status = match conversation_status {
                            AmbientConversationStatus::Success => {
                                SDKConversationOutputStatus::Success
                            }
                            AmbientConversationStatus::Cancelled { reason } => {
                                SDKConversationOutputStatus::Cancelled { reason }
                            }
                            AmbientConversationStatus::Error { error } => {
                                SDKConversationOutputStatus::Error { error }
                            }
                            AmbientConversationStatus::Blocked { blocked_action } => {
                                SDKConversationOutputStatus::Blocked { blocked_action }
                            }
                        };

                        match output_status {
                            SDKConversationOutputStatus::Success
                            | SDKConversationOutputStatus::Blocked { .. }
                            | SDKConversationOutputStatus::Cancelled { .. } => {
                                // Whether to keep the process alive after completion is controlled by
                                // the `warp agent run --idle-on-complete[=<DURATION>]` flag.
                                if let Some(idle_timeout) = me.idle_on_complete {
                                    log::info!(
                                        "Ambient agent idle lifecycle: event=idle_timeout_scheduled task_id={:?} terminal_view_id={terminal_id:?} timeout={idle_timeout:?} outcome=non_error_completion",
                                        me.task_id
                                    );
                                } else {
                                    log::info!(
                                        "Ambient agent idle lifecycle: event=run_completion_immediate task_id={:?} terminal_view_id={terminal_id:?} outcome=non_error_completion",
                                        me.task_id
                                    );
                                }
                                run_exit.complete_with_optional_idle(
                                    me.idle_on_complete,
                                    output_status,
                                );
                            }
                            // For errors, check if we expect an automatic retry.
                            SDKConversationOutputStatus::Error { ref error } => {
                                // If the error indicates that an automatic resume will be attempted,
                                // don't terminate yet - give the retry a chance to succeed.
                                // However, bound the wait so the CLI doesn't hang indefinitely
                                // if the follow-up never arrives.
                                if error.will_attempt_resume() {
                                    log::info!("Error occurred but automatic resume will be attempted; waiting up to {AUTO_RESUME_TIMEOUT:?} for retry");
                                    log::info!(
                                        "Ambient agent idle lifecycle: event=idle_timeout_scheduled task_id={:?} terminal_view_id={terminal_id:?} timeout={AUTO_RESUME_TIMEOUT:?} outcome=automatic_resume_pending",
                                        me.task_id
                                    );
                                    run_exit.end_run_after(AUTO_RESUME_TIMEOUT, output_status);
                                    return;
                                }

                                log::info!(
                                    "Ambient agent idle lifecycle: event=run_completion_immediate task_id={:?} terminal_view_id={terminal_id:?} outcome=error",
                                    me.task_id
                                );
                                run_exit.end_run_now(output_status);
                            }
                        }
                    }
                }

                BlocklistAIHistoryEvent::SetActiveConversation { .. } => {
                    // Continuing an existing conversation should reset the idle timer.
                    run_exit.cancel_idle_timeout();
                }
                BlocklistAIHistoryEvent::StartedNewConversation { .. }
                | BlocklistAIHistoryEvent::ReassignedExchange { .. }
                | BlocklistAIHistoryEvent::ClearedConversationsInTerminalView { .. }
                | BlocklistAIHistoryEvent::UpdatedAutoexecuteOverride { .. }
                | BlocklistAIHistoryEvent::SplitConversation { .. }
                | BlocklistAIHistoryEvent::RemoveConversation { .. }
                | BlocklistAIHistoryEvent::DeletedConversation { .. }
                | BlocklistAIHistoryEvent::RestoredConversations { .. }
                | BlocklistAIHistoryEvent::CreatedSubtask { .. }
                | BlocklistAIHistoryEvent::UpgradedTask { .. }
                | BlocklistAIHistoryEvent::UpdatedConversationMetadata { .. }
                | BlocklistAIHistoryEvent::ClearedActiveConversation { .. }
                | BlocklistAIHistoryEvent::UpdatedConversationArtifacts { .. }
                | BlocklistAIHistoryEvent::ConversationServerTokenAssigned { .. }
                | BlocklistAIHistoryEvent::ConversationOwnershipTransferred { .. }
                | BlocklistAIHistoryEvent::NewConversationRequestComplete { .. }
                | BlocklistAIHistoryEvent::OrchestrationConfigUpdated { .. }
                | BlocklistAIHistoryEvent::ConversationUsageMetadataUpdated { .. }
                | BlocklistAIHistoryEvent::LocalSharedSessionEstablished { .. } => (),
            }
        });

        // Subscribe to document model events to emit artifact_created when plans sync to Warp Drive.
        ctx.subscribe_to_model(&AIDocumentModel::handle(ctx), move |me, event, ctx| {
            let AIDocumentModelEvent::DocumentSaveStatusUpdated(document_id) = event else {
                return;
            };

            let doc_model = AIDocumentModel::as_ref(ctx);

            // Only emit when the document transitions to "Saved" (has a ServerId)
            if !doc_model.get_document_save_status(document_id).is_saved() {
                return;
            }

            // Get the document to extract the notebook link
            let Some(document) = doc_model.get_current_document(document_id) else {
                return;
            };

            // Get the notebook link from the document model
            let Some(notebook_link) =
                doc_model.get_document_warp_drive_object_link(document_id, ctx)
            else {
                return;
            };

            let document_id_str = document_id.to_string();

            report_if_error!(output::with_stdout_buffered(|buf| {
                match me.output_format {
                    OutputFormat::Json | OutputFormat::Ndjson => {
                        output::json::plan_artifact_created(
                            &document_id_str,
                            &notebook_link,
                            &document.title,
                            buf,
                        )
                    }
                    OutputFormat::Text | OutputFormat::Pretty => {
                        output::text::plan_artifact_created(
                            &document_id_str,
                            &notebook_link,
                            &document.title,
                            buf,
                        )
                    }
                }
            })
            .context("Failed to write artifact_created"));
        });

        // Submit the AI query.
        if !self.skip_initial_turn {
            self.terminal_driver.update(ctx, |td, ctx| {
                td.with_terminal_view(ctx, |terminal, ctx| match task_prompt {
                    AgentRunPrompt::Local(prompt_str) => {
                        if FeatureFlag::AgentView.is_enabled() {
                            terminal.enter_agent_view(
                                Some(prompt_str),
                                restored_conversation_id,
                                AgentViewEntryOrigin::Cli,
                                ctx,
                            );
                        } else {
                            terminal.set_ai_input_mode_with_query(Some(&prompt_str), ctx);
                            terminal
                                .input()
                                .update(ctx, |input, ctx| input.input_enter(ctx));
                        }
                    }
                    AgentRunPrompt::ServerSide {
                        skill,
                        attachments_dir,
                    } => {
                        let Some(task_id) = self.task_id else {
                            log::error!("ServerSide prompt without task_id");
                            return;
                        };
                        let ambient_run_id = task_id.to_string();
                        terminal.ai_controller().update(ctx, |controller, ctx| {
                            controller.send_ai_input_with_context(
                                |context| AIAgentInput::StartFromAmbientRunPrompt {
                                    ambient_run_id: ambient_run_id.clone(),
                                    context,
                                    runtime_skill: skill.clone(),
                                    attachments_dir: attachments_dir.clone(),
                                },
                                ctx,
                            );
                        });
                    }
                })
            });
        }

        rx
    }

    /// Write the inputs to an exchange to stdout.
    fn write_exchange_inputs(&self, exchange: &AIAgentExchange) -> io::Result<()> {
        output::with_stdout_buffered(|buf| {
            for input in &exchange.input {
                self.write_input(buf, input)?;
            }
            Ok(())
        })
    }

    /// Write the outputs of an exchange to stdout.
    fn write_exchange_output(&self, exchange: &AIAgentExchange) -> io::Result<()> {
        let Some(shared) = exchange.output_status.output() else {
            return Ok(());
        };
        let output = shared.get();

        output::with_stdout_buffered(|buf| self.write_output(buf, &output))
    }

    /// Format an agent input for display.
    fn write_input<W: Write>(&self, w: &mut W, input: &AIAgentInput) -> io::Result<()> {
        match self.output_format {
            OutputFormat::Json | OutputFormat::Ndjson => output::json::format_input(input, w),
            OutputFormat::Text | OutputFormat::Pretty => output::text::format_input(input, w),
        }
    }

    /// Format an agent output for display.
    fn write_output<W: Write>(&self, w: &mut W, output: &AIAgentOutput) -> io::Result<()> {
        match self.output_format {
            OutputFormat::Json | OutputFormat::Ndjson => output::json::format_output(output, w),
            OutputFormat::Text | OutputFormat::Pretty => output::text::format_output(output, w),
        }
    }

    /// Subscribe to the singleton `CLIAgentSessionsModel` so that idle-on-complete
    /// timers are driven by CLI agent session status changes.
    ///
    /// Task state reporting is handled centrally by `LocalAgentTaskSyncModel`;
    /// the driver only registers the `terminal_view_id → task_id` mapping
    /// so that the sync model can look up the task for each session.
    fn subscribe_to_cli_agent_session_events(
        &self,
        harness_exit: IdleTimeoutSender<()>,
        ctx: &mut ModelContext<Self>,
    ) {
        let terminal_view_id = self.terminal_driver.as_ref(ctx).terminal_view().id();

        // Register this session with LocalAgentTaskSyncModel so CLI agent
        // status changes are reported to the server.
        if let Some(task_id) = self.task_id {
            LocalAgentTaskSyncModel::handle(ctx).update(ctx, |model, ctx| {
                model.register_cli_session(terminal_view_id, task_id, ctx);
            });
        }

        ctx.subscribe_to_model(
            &CLIAgentSessionsModel::handle(ctx),
            move |me, event, ctx| match event {
                CLIAgentSessionsModelEvent::StatusChanged {
                    terminal_view_id: event_tid,
                    status,
                    ..
                } => {
                    if *event_tid != terminal_view_id {
                        return;
                    }

                    // Drive idle-on-complete timer for the harness exit signal.
                    match status {
                        CLIAgentSessionStatus::Success | CLIAgentSessionStatus::Blocked { .. } => {
                            if let Some(idle_timeout) = me.idle_on_complete {
                                log::info!(
                                    "Ambient agent CLI lifecycle: event=idle_timeout_scheduled task_id={:?} terminal_view_id={terminal_view_id:?} timeout={idle_timeout:?}",
                                    me.task_id
                                );
                            } else {
                                log::info!(
                                    "Ambient agent CLI lifecycle: event=run_completion_immediate task_id={:?} terminal_view_id={terminal_view_id:?}",
                                    me.task_id
                                );
                            }
                            harness_exit.complete_with_optional_idle(me.idle_on_complete, ());
                        }
                        CLIAgentSessionStatus::InProgress => {
                            log::info!(
                                "Ambient agent CLI lifecycle: event=idle_timeout_cancel_requested task_id={:?} terminal_view_id={terminal_view_id:?} trigger=session_in_progress",
                                me.task_id
                            );
                            harness_exit.cancel_idle_timeout();
                        }
                    }
                }
                CLIAgentSessionsModelEvent::SessionUpdated {
                    terminal_view_id: event_tid,
                    ..
                } => {
                    if *event_tid != terminal_view_id {
                        return;
                    }

                    let Some(runner) = me.harness.clone() else {
                        return;
                    };
                    let spawner = ctx.spawner();
                    ctx.spawn(
                        async move {
                            log::debug!(
                                "Triggering post-turn harness session update from CLI agent event"
                            );
                            report_if_error!(runner
                                .handle_session_update(&spawner)
                                .await
                                .context("Failed to update harness state from CLI session event"));
                            log::debug!("Triggering post-turn save of harness conversation data");
                            report_if_error!(runner
                                .save_conversation(SavePoint::PostTurn, &spawner)
                                .await
                                .context("Failed to save harness conversation (post-turn)"));
                        },
                        |_, _, _| {},
                    );
                }
                CLIAgentSessionsModelEvent::Started { .. }
                | CLIAgentSessionsModelEvent::InputSessionChanged { .. }
                | CLIAgentSessionsModelEvent::Ended { .. } => {}
            },
        );
    }

    /// Handle events re-emitted by the `TerminalDriver`.
    fn handle_terminal_driver_event(
        &mut self,
        event: &TerminalDriverEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        match event {
            TerminalDriverEvent::SlowBootstrap => {
                tracing::event!(
                    tracing::Level::WARN,
                    tags.cloud_agent = true,
                    "slow bootstrap"
                );
                eprintln!(
                    "Warning: Terminal session is slow to bootstrap. See https://docs.warp.dev/support-and-community/troubleshooting-and-support/known-issues#shells to troubleshoot."
                );
            }
            TerminalDriverEvent::EstablishedSharedSession {
                session_id,
                join_url,
            } => {
                tracing::event!(
                    tracing::Level::INFO,
                    tags.cloud_agent = true,
                    session_id = %*session_id,
                    "shared session established",
                );
                write_session_joined(join_url, self.output_format);

                // If running as part of a task, store the session-sharing link.
                if let Some(task_id) = self.task_id {
                    let server_api = ServerApiProvider::as_ref(ctx).get_ai_client();
                    let session_id = *session_id;
                    ctx.spawn(
                        async move {
                            report_if_error!(server_api
                                .update_agent_task(task_id, None, Some(session_id), None, None)
                                .await
                                .context("Error setting ambient agent shared session ID"));
                        },
                        |_, _, _| {},
                    );
                }
            }
        }
    }

    /// Set up each cloud provider in sequence.
    async fn setup_cloud_providers(spawner: &ModelSpawner<Self>) -> Result<(), AgentDriverError> {
        let (mut providers, terminal_spawner) = spawner
            .spawn(|me, ctx| {
                let terminal_spawner = me.terminal_driver.update(ctx, |_, ctx| ctx.spawner());
                // Temporarily take all cloud providers so we can move them onto the background thread.
                //
                // Since the Vec of cloud providers is owned by the AgentDriver model, which is
                // itself owned by the UI framework, we can only mutate them in-place on the UI thread.
                // So that `CloudProvider::setup` can be `async` _and_ take `&mut self`, the
                // `setup_cloud_providers` future takes ownership of all the providers, and then moves
                // them back to the UI thread. This is somewhat similar to how views and models are removed
                // from the UI framework temporarily while being mutated.
                let providers = std::mem::take(&mut me.cloud_providers);
                (providers, terminal_spawner)
            })
            .await?;

        let mut result = Ok(());

        for provider in providers.iter_mut() {
            let provider_result = provider.setup(terminal_spawner.clone()).await;
            if provider_result.is_err() {
                result = provider_result;
                break;
            }
        }

        // Restore the cloud providers.
        spawner
            .spawn(move |me, _| {
                me.cloud_providers = providers;
            })
            .await?;

        result?;
        Ok(())
    }

    /// Perform cleanup after the agent has finished running.
    async fn cleanup(spawner: ModelSpawner<Self>) {
        let Ok((providers, task_id)) = spawner
            .spawn(|me, _| (std::mem::take(&mut me.cloud_providers), me.task_id))
            .await
        else {
            log::error!("Unable to retrieve cloud providers for cleanup");
            return;
        };

        log::info!(
            "Ambient agent lifecycle: event=driver_cleanup_started task_id={task_id:?} next=terminal_process_exit"
        );

        for provider in providers {
            if let Err(err) = provider.cleanup().await {
                report_error!(anyhow!(err).context("Unable to clean up cloud provider"));
            }
        }
    }

    /// Invoke the end-of-run snapshot upload pipeline if the feature flag is enabled and this
    /// driver is associated with a cloud task. Errors are logged internally; this helper always
    /// returns so cleanup can proceed.
    #[tracing::instrument(skip_all, fields(tags.cloud_agent = true))]
    async fn run_snapshot_upload(spawner: &ModelSpawner<Self>) {
        if !FeatureFlag::OzHandoff.is_enabled() {
            return;
        }

        // Snapshot upload is only meaningful for cloud task runs, so short-circuit before
        // pulling the rest of the context onto this task.
        let Ok((Some(task_id), snapshot_disabled, upload_timeout, script_timeout)) = spawner
            .spawn(|me, _| {
                (
                    me.task_id,
                    me.snapshot_disabled,
                    me.snapshot_upload_timeout,
                    me.snapshot_script_timeout,
                )
            })
            .await
        else {
            return;
        };
        if snapshot_disabled {
            log::info!("Skipping snapshot upload because --no-snapshot was specified");
            return;
        }

        let Ok((working_dir, client)) = spawner
            .spawn(|me, ctx| {
                let client = ServerApiProvider::as_ref(ctx).get_harness_support_client();
                (me.working_dir.clone(), client)
            })
            .await
        else {
            log::error!("Unable to retrieve snapshot upload context for cleanup (task {task_id})");
            return;
        };

        // Drain any pending declarations writes from the history subscription before the
        // declarations script runs. This guarantees no driver-side `file` append is still in
        // flight when the bash script appends its `repo` entries.
        if let Ok(Some(writer)) = spawner.spawn(|me, _| me.snapshot_file_writer.clone()).await {
            writer.flush().await;
        }

        // Regenerate the declarations file so the upload pipeline sees the latest workspace
        // state. The helper swallows its own errors at ERROR level; we just proceed.
        snapshot::run_declarations_script(&working_dir, &task_id, script_timeout).await;

        // Cap the upload so a pathological slow upload cannot wedge cleanup.
        // On timeout we surface via report_error! so Sentry captures the incident and on-call
        // alerting can fire, then let cloud-provider teardown continue.
        if let Err(TimeoutError) = snapshot::upload_snapshot_from_declarations(client, &task_id)
            .with_timeout(upload_timeout)
            .await
        {
            report_error!(anyhow!(
                "Snapshot upload timed out after {:?}; continuing with cleanup (task {task_id})",
                upload_timeout
            ));
        }
    }
}

/// Build the env-var map for the agent terminal session from managed secrets.
///
/// Invariant: the server resolves at most one typed auth secret per harness, so
/// env-var collisions between typed secrets cannot occur in practice.
///
/// Precedence order:
/// 1. Worker-injected process env (already non-empty in `std::env`). Never overridden.
/// 2. Typed auth secrets (`AnthropicApiKey`, `AnthropicBedrock*`). Inserted atomically:
///    if any one env var for a typed secret is already worker-injected, the entire
///    secret is skipped.
/// 3. Generic `RawValue` secrets. Skipped on collision with either of the above.
fn build_secret_env_vars(
    secrets: &HashMap<String, ManagedSecretValue>,
) -> HashMap<OsString, OsString> {
    let mut env_vars = HashMap::with_capacity(secrets.len() + 1);

    // Phase 1: Record which env-var names are claimed by typed auth secrets.
    let typed_env_names = typed_secret_env_names(secrets);

    // Phase 2: Insert typed auth secrets atomically.
    for (name, secret) in secrets {
        let entries = typed_secret_entries(secret);
        if entries.is_empty() {
            continue;
        }

        if let Some((conflict, _)) = entries
            .iter()
            .find(|(env_name, _)| std::env::var(env_name).is_ok_and(|v| !v.is_empty()))
        {
            log::warn!(
                "Skipping auth secret '{name}' ({:?}): '{conflict}' is already set \
                 in the process environment",
                secret.secret_type(),
            );
            continue;
        }

        for (env_name, env_value) in entries {
            env_vars.insert(OsString::from(env_name), OsString::from(env_value));
        }
    }

    // Phase 3: Insert generic RawValue secrets, skipping any that collide
    // with worker-injected env vars or typed-secret-claimed names.
    for (name, secret) in secrets {
        let ManagedSecretValue::RawValue { value } = secret else {
            continue;
        };
        let env_name = name.as_str();

        if std::env::var(env_name).is_ok_and(|v| !v.is_empty()) {
            log::warn!("Skipping managed secret {env_name}: already set in environment");
            continue;
        }
        if typed_env_names.contains(env_name) {
            log::warn!("Skipping generic secret '{env_name}': overridden by a typed auth secret");
            continue;
        }

        env_vars.insert(OsString::from(env_name), OsString::from(value.as_str()));
    }

    env_vars
}

/// The env-var names that any typed auth secret in `secrets` will populate.
/// Used for phase-3 collision detection and by the suffix resolver.
fn typed_secret_env_names(secrets: &HashMap<String, ManagedSecretValue>) -> HashSet<&'static str> {
    let mut names = HashSet::new();
    for secret in secrets.values() {
        for (env_name, _) in typed_secret_entries(secret) {
            names.insert(env_name);
        }
    }
    names
}

fn typed_secret_entries(secret: &ManagedSecretValue) -> Vec<(&'static str, &str)> {
    match secret {
        ManagedSecretValue::RawValue { .. } => vec![],
        ManagedSecretValue::AnthropicApiKey { api_key } => {
            vec![("ANTHROPIC_API_KEY", api_key.as_str())]
        }
        ManagedSecretValue::AnthropicBedrockApiKey {
            aws_bearer_token_bedrock,
            aws_region,
        } => vec![
            (
                "AWS_BEARER_TOKEN_BEDROCK",
                aws_bearer_token_bedrock.as_str(),
            ),
            ("CLAUDE_CODE_USE_BEDROCK", "1"),
            ("AWS_REGION", aws_region.as_str()),
        ],
        ManagedSecretValue::AnthropicBedrockAccessKey {
            aws_access_key_id,
            aws_secret_access_key,
            aws_session_token,
            aws_region,
        } => {
            let mut entries = vec![
                ("AWS_ACCESS_KEY_ID", aws_access_key_id.as_str()),
                ("AWS_SECRET_ACCESS_KEY", aws_secret_access_key.as_str()),
                ("CLAUDE_CODE_USE_BEDROCK", "1"),
                ("AWS_REGION", aws_region.as_str()),
            ];
            if let Some(token) = aws_session_token.as_deref() {
                entries.push(("AWS_SESSION_TOKEN", token));
            }
            entries
        }
        ManagedSecretValue::OpenaiApiKey { api_key, .. } => {
            vec![("OPENAI_API_KEY", api_key.as_str())]
        }
    }
}

impl Entity for AgentDriver {
    type Event = ();
}

/// The only reason that `AgentDriver` is a singleton entity is to ensure the UI framework
/// doesn't drop it. Generally, we should not assume there's only one running agent.
impl SingletonEntity for AgentDriver {}

/// Write the run ID to stdout using the appropriate output format.
pub(super) fn write_run_started(run_id: &str, output_format: OutputFormat) {
    report_if_error!(output::with_stdout_buffered(|buf| match output_format {
        OutputFormat::Json | OutputFormat::Ndjson => output::json::run_started(run_id, buf),
        OutputFormat::Text | OutputFormat::Pretty => output::text::run_started(run_id, buf),
    })
    .context("Failed to write run ID"));
}

/// Report a driver-level error to the server for the given task.
///
/// Used for errors that occur before or outside a conversation. Errors
/// that occur while the agent is running should be reported through
/// the `LocalAgentTaskSyncModel`.
pub(super) async fn report_driver_error(
    task_id: AmbientAgentTaskId,
    err: &AgentDriverError,
    server_api: &Arc<dyn AIClient>,
) {
    let (state, status_update) = error_classification::classify_driver_error(err);
    if let Err(e) = server_api
        .update_agent_task(task_id, Some(state), None, None, Some(status_update))
        .await
    {
        report_error!(
            anyhow!(e).context(format!("Failed to report driver error for task {task_id}"))
        );
    }
}

/// Stamps `parent_agent_id` (= parent's `run_id` under v2) onto the
/// driver-hosted conversation so the streamer's child-role check
/// succeeds. No-op when `parent_run_id` is `None` (a top-level run).
fn stamp_parent_agent_id_if_some(
    conv_id: AIConversationId,
    parent_run_id: Option<&str>,
    ctx: &mut ModelContext<AgentDriver>,
) {
    let Some(parent_run_id) = parent_run_id else {
        return;
    };
    let parent_run_id = parent_run_id.to_owned();
    BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, _| {
        if let Some(conv) = history.conversation_mut(&conv_id) {
            conv.set_parent_agent_id(parent_run_id);
        }
    });
}

/// Write the session URL to stdout using the appropriate output format
fn write_session_joined(join_url: &str, output_format: OutputFormat) {
    report_if_error!(output::with_stdout_buffered(|buf| match output_format {
        OutputFormat::Json | OutputFormat::Ndjson =>
            output::json::shared_session_established(join_url, buf),
        OutputFormat::Text | OutputFormat::Pretty => {
            output::text::shared_session_established(join_url, buf)
        }
    })
    .context("Failed to write shared session event"));
}

#[cfg(test)]
#[path = "driver_tests.rs"]
mod tests;

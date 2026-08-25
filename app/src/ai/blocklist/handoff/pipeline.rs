//! Shared preparation and execution pipeline for local-to-cloud handoff.
//!
//! Frontends call [`prepare_handoff`] while they still have synchronous access
//! to the source terminal and conversation models. Preparation enforces source
//! guardrails, captures the data needed for a cloud launch or failure
//! restoration, cancels an active local response, and returns an owned
//! [`PendingHandoff`].
//!
//! The frontend may retain that pending value while the user edits its model or
//! environment selection. Passing it to [`execute_handoff`] ends that editing
//! phase. Execution revalidates mutable global state, then returns an owned
//! future that performs the server conversation fork, invokes the frontend's
//! destination-materialization callback, uploads the workspace snapshot, and
//! spawns exactly one cloud run. No app or view context is held across awaits.
//!
//! [`HandoffCommitOutcome`] separates pre-execution rejection from failures
//! after external work begins and from successful run creation. Snapshot upload
//! errors are recorded in the outcome but deliberately degrade to a spawn
//! without workspace state.

use std::collections::HashSet;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use anyhow::Context as _;
use futures::channel::oneshot;
use futures::future::{Either, select};
use warp_core::send_telemetry_from_ctx;
use warp_errors::report_error;
use warp_util::standardized_path::StandardizedPath;
use warpui::{AppContext, EntityId, ModelHandle, SingletonEntity};

use super::snapshot::{HandoffUploadResult, SnapshotUploadTarget, upload_handoff_snapshot};
use super::touched_repos::extract_paths_from_conversation;
use super::{HandoffLaunchAttachments, PendingCloudLaunch};
use crate::ai::agent::conversation::{AIConversation, AIConversationId};
use crate::ai::agent::{CancellationReason, extract_user_query_mode};
use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::ai::ambient_agents::telemetry::{
    CloudAgentTelemetryEvent, HandoffEntryPoint, HandoffInjectionPath, HandoffSurface,
};
use crate::ai::blocklist::orchestration_topology::descendant_conversation_ids_in_spawn_order;
use crate::ai::blocklist::{
    BlocklistAIContextModel, BlocklistAIController, BlocklistAIHistoryModel, PendingAttachment,
};
use crate::ai::cloud_environments::CloudAmbientAgentEnvironment;
use crate::ai::execution_profiles::resolve_cloud_agent_computer_use_state;
use crate::ai::llms::{LLMId, LLMPreferences};
use crate::ai::orchestration::{
    CloudAgentStartupBlocker, CloudAgentStartupFailure, CloudAgentStartupIssue,
    classify_cloud_agent_startup_error, oz_run_url, resolve_default_environment_id,
    resolve_default_host_slug, should_disable_snapshot,
};
use crate::cloud_object::CloudObjectLookup as _;
use crate::server::ids::{ServerId, SyncId};
use crate::server::server_api::ai::{
    AIClient, AgentConfigSnapshot, AttachmentInput, InitialSnapshotToken, SpawnAgentRequest,
};
use crate::settings::AISettings;

const HANDOFF_CONTINUE_PROMPT: &str = "Continue";
const HANDOFF_APPLY_SNAPSHOT_PROMPT: &str = "Apply the workspace changes from my previous session.";

/// Frontend-resolved source state consumed by [`prepare_handoff`].
///
/// This input keeps UI discovery outside the shared pipeline: the caller
/// supplies the relevant terminal/model handles, current working directory,
/// snapshot execution target, launch payload, and policy facts. Preparation
/// consumes these values synchronously and returns a frontend-neutral
/// [`PendingHandoff`].
pub struct HandoffPrepareInput {
    terminal_surface_id: EntityId,
    expected_conversation_id: Option<AIConversationId>,
    source_conversation_id: Option<Option<AIConversationId>>,
    history: ModelHandle<BlocklistAIHistoryModel>,
    controller: ModelHandle<BlocklistAIController>,
    context: ModelHandle<BlocklistAIContextModel>,
    current_working_directory: Option<String>,
    snapshot_target: SnapshotUploadTarget,
    has_long_running_command: bool,
    launch: Option<PendingCloudLaunch>,
    transfer_pending_attachments: bool,
    environment_id: Option<SyncId>,
    environment_required: bool,
    entry_point: HandoffEntryPoint,
    surface: HandoffSurface,
    cancellation_reason: CancellationReason,
    require_in_progress_source: bool,
}

impl HandoffPrepareInput {
    pub fn new(
        terminal_surface_id: EntityId,
        history: ModelHandle<BlocklistAIHistoryModel>,
        controller: ModelHandle<BlocklistAIController>,
        context: ModelHandle<BlocklistAIContextModel>,
        snapshot_target: SnapshotUploadTarget,
        entry_point: HandoffEntryPoint,
        surface: HandoffSurface,
    ) -> Self {
        Self {
            terminal_surface_id,
            expected_conversation_id: None,
            source_conversation_id: None,
            history,
            controller,
            context,
            current_working_directory: None,
            snapshot_target,
            has_long_running_command: false,
            launch: None,
            transfer_pending_attachments: true,
            environment_id: None,
            environment_required: false,
            entry_point,
            surface,
            cancellation_reason: CancellationReason::ManuallyCancelled,
            require_in_progress_source: false,
        }
    }

    pub fn with_expected_conversation_id(
        mut self,
        expected_conversation_id: Option<AIConversationId>,
    ) -> Self {
        self.expected_conversation_id = expected_conversation_id;
        self
    }

    /// Uses the frontend-resolved source instead of the selected conversation.
    ///
    /// Passing `None` explicitly preserves a fresh launch even when another
    /// conversation is selected on the frontend.
    pub fn with_source_conversation_id(
        mut self,
        source_conversation_id: Option<AIConversationId>,
    ) -> Self {
        self.source_conversation_id = Some(source_conversation_id);
        self
    }

    pub fn with_current_working_directory(
        mut self,
        current_working_directory: Option<String>,
    ) -> Self {
        self.current_working_directory = current_working_directory;
        self
    }

    pub fn with_long_running_command(mut self, has_long_running_command: bool) -> Self {
        self.has_long_running_command = has_long_running_command;
        self
    }

    pub fn with_launch(mut self, launch: Option<PendingCloudLaunch>) -> Self {
        self.launch = launch;
        self
    }

    pub fn with_transfer_pending_attachments(mut self, transfer_pending_attachments: bool) -> Self {
        self.transfer_pending_attachments = transfer_pending_attachments;
        self
    }

    pub fn with_environment_id(mut self, environment_id: Option<SyncId>) -> Self {
        self.environment_id = environment_id;
        self
    }

    #[cfg_attr(not(feature = "tui"), allow(dead_code))]
    pub fn with_environment_required(mut self, environment_required: bool) -> Self {
        self.environment_required = environment_required;
        self
    }

    pub fn with_cancellation_reason(mut self, cancellation_reason: CancellationReason) -> Self {
        self.cancellation_reason = cancellation_reason;
        self
    }

    pub fn with_require_in_progress_source(mut self, require_in_progress_source: bool) -> Self {
        self.require_in_progress_source = require_in_progress_source;
        self
    }
}

/// A reason handoff preparation or execution revalidation could not proceed.
///
/// Preparation errors occur before a `PendingHandoff` is returned. Selection
/// and enablement errors can also be returned by [`execute_handoff`] if mutable
/// global state changed while a frontend retained the pending value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HandoffPrepareError {
    /// The explicitly requested source is no longer the selected conversation.
    SourceConversationChanged,
    /// There is no source conversation, prompt, or valid workspace path to launch.
    EmptySourceAndPrompt,
    /// A caller that requires a running source observed a non-running source.
    SourceNotInProgress,
    /// The source terminal is running a command that cannot be transferred.
    LongRunningCommand,
    /// An orchestration descendant is still running or waiting for input.
    ActiveOrBlockedChild,
    /// An existing conversation has not received the token required to fork it.
    MissingServerConversationToken,
    /// Local-to-cloud handoff became disabled before execution.
    HandoffDisabled,
    /// The caller requires an environment, but none is selected.
    MissingRequiredEnvironment,
    /// The selected environment is no longer present in the current catalog.
    InvalidEnvironment,
    /// The selected model cannot run as a cloud Oz model.
    InvalidModel,
}

/// Immutable values a frontend needs while presenting a prepared handoff.
///
/// This deliberately omits execution internals such as the source token,
/// snapshot target, attachments, and restoration state.
#[derive(Clone)]
pub struct HandoffPresentationSnapshot {
    pub source_conversation_id: Option<AIConversationId>,
    pub environment_id: Option<SyncId>,
    pub model_id: String,
    pub forked_existing_conversation: bool,
}

/// Source-input state that can be restored after handoff fails.
///
/// The value is owned by [`PendingHandoff`] until an outcome transfers it to
/// the frontend. It can be taken only once so two failure paths cannot restore
/// duplicate prompt or attachment state.
#[derive(Clone)]
pub struct HandoffRestoration {
    pub prompt: String,
    pub attachments: Vec<PendingAttachment>,
    pub environment_id: Option<SyncId>,
}

/// Data supplied after the server conversation-fork decision is complete.
///
/// The frontend uses this to create its destination pane or card and, for an
/// existing conversation, associate its local fork with the new server token.
pub struct HandoffTargetMaterialization {
    pub source_conversation: Option<AIConversation>,
    pub forked_conversation_id: Option<String>,
    pub title: Option<String>,
    /// Pre-snapshot request used to present the queued prompt immediately.
    pub request: SpawnAgentRequest,
    /// One-shot signal the frontend consumes when the user cancels the handoff.
    pub cancel: oneshot::Sender<()>,
}

/// One-shot frontend callback that materializes the destination handoff UI.
///
/// Execution invokes it after the optional server conversation fork and before
/// snapshot upload or cloud-run creation. Returning an error stops execution
/// before the agent is spawned.
pub type MaterializeHandoffTarget = Box<
    dyn FnOnce(
            HandoffTargetMaterialization,
        ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send>>
        + Send,
>;

/// Fully prepared, editable handoff state between preparation and execution.
///
/// [`prepare_handoff`] owns construction so source policy and captured data
/// cannot diverge between frontends. While a frontend retains this value, it
/// may update the selected environment or model. [`execute_handoff`] then
/// consumes it, revalidates those selections, and runs the async stages.
pub struct PendingHandoff {
    source_conversation: Option<AIConversation>,
    source_conversation_active: bool,
    source_paths: Vec<StandardizedPath>,
    source_token: Option<String>,
    title: Option<String>,
    prompt: String,
    request_attachments: Vec<AttachmentInput>,
    restoration: Option<HandoffRestoration>,
    selected_environment_id: Option<SyncId>,
    environment_required: bool,
    environment_selection_is_explicit: bool,
    valid_environment_ids: HashSet<SyncId>,
    selected_model_id: String,
    model_selection_is_explicit: bool,
    model_is_cloud_runnable: bool,
    config: AgentConfigSnapshot,
    snapshot_target: SnapshotUploadTarget,
    snapshot_disabled: bool,
    orchestration_handoff: Option<bool>,
}

impl PendingHandoff {
    /// Returns the values needed to render or materialize this pending handoff.
    pub fn presentation_snapshot(&self) -> HandoffPresentationSnapshot {
        HandoffPresentationSnapshot {
            source_conversation_id: self.source_conversation.as_ref().map(AIConversation::id),
            environment_id: self.selected_environment_id,
            model_id: self.selected_model_id.clone(),
            forked_existing_conversation: self.source_conversation.is_some(),
        }
    }

    /// Applies an environment selection to the final agent configuration.
    ///
    /// Once a frontend records an explicit user selection, later implicit
    /// defaults cannot replace it.
    #[cfg_attr(not(feature = "tui"), allow(dead_code))]
    pub fn set_environment_id(&mut self, environment_id: Option<SyncId>, is_explicit: bool) {
        if !is_explicit && self.environment_selection_is_explicit {
            return;
        }
        self.selected_environment_id = environment_id;
        self.environment_selection_is_explicit |= is_explicit;
        self.config.environment_id = environment_id.map(|id| id.to_string());
    }

    /// Applies a model selection and records whether it is cloud-runnable.
    ///
    /// Once a frontend records an explicit user selection, later implicit
    /// defaults cannot replace it.
    #[cfg_attr(not(feature = "tui"), allow(dead_code))]
    pub fn set_model_id(&mut self, model_id: String, is_explicit: bool, ctx: &AppContext) {
        if !is_explicit && self.model_selection_is_explicit {
            return;
        }
        let preferences = LLMPreferences::as_ref(ctx);
        let model_id = if is_explicit {
            model_id
        } else {
            preferences.cloud_runnable_oz_model_id_or_fallback(&LLMId::from(model_id.as_str()))
        };

        self.model_is_cloud_runnable =
            preferences.is_cloud_runnable_oz_model_id(&LLMId::from(model_id.as_str()));
        self.selected_model_id = model_id.clone();
        self.model_selection_is_explicit |= is_explicit;
        self.config.model_id = Some(model_id);
    }

    /// Replaces the environment catalog used by [`Self::validate`].
    ///
    /// Frontends call this when their environment projection changes while
    /// retaining an editable pending handoff.
    pub fn set_valid_environment_ids(&mut self, valid_environment_ids: HashSet<SyncId>) {
        self.valid_environment_ids = valid_environment_ids;
    }

    /// Validates the editable selections without starting external work.
    pub fn validate(&self) -> Result<(), HandoffPrepareError> {
        if self.environment_required && self.selected_environment_id.is_none() {
            return Err(HandoffPrepareError::MissingRequiredEnvironment);
        }
        if self
            .selected_environment_id
            .is_some_and(|id| !self.valid_environment_ids.contains(&id))
        {
            return Err(HandoffPrepareError::InvalidEnvironment);
        }
        if !self.model_is_cloud_runnable || self.selected_model_id.trim().is_empty() {
            return Err(HandoffPrepareError::InvalidModel);
        }
        Ok(())
    }

    /// Removes and returns source-input restoration state, at most once.
    pub fn take_restoration(&mut self) -> Option<HandoffRestoration> {
        self.restoration.take()
    }
}

/// Prepares a handoff before a frontend presents or executes it.
///
/// This is the synchronous first half of the lifecycle. It resolves and
/// validates the source conversation, rejects active orchestration children or
/// incompatible terminal state, captures snapshot paths and restoration data,
/// cancels an active source response, verifies that an existing conversation
/// can be forked, transfers pending attachments out of the source input, and
/// resolves initial cloud configuration.
///
/// On success, the caller owns a [`PendingHandoff`] and may edit its selection
/// fields before calling [`execute_handoff`]. On error, no pending value exists;
/// guard failures occur before source cancellation, while a missing server
/// token is detected after cancellation but before attachments are cleared.
pub fn prepare_handoff(
    input: HandoffPrepareInput,
    ctx: &mut AppContext,
) -> Result<PendingHandoff, HandoffPrepareError> {
    let HandoffPrepareInput {
        terminal_surface_id,
        expected_conversation_id,
        source_conversation_id,
        history,
        controller,
        context,
        current_working_directory,
        snapshot_target,
        has_long_running_command,
        launch,
        transfer_pending_attachments,
        environment_id: selected_environment_id,
        environment_required,
        entry_point,
        surface,
        cancellation_reason,
        require_in_progress_source,
    } = input;

    let selected_id = match (expected_conversation_id, source_conversation_id) {
        (Some(expected_conversation_id), _) => Some(expected_conversation_id),
        (None, Some(source_conversation_id)) => source_conversation_id,
        (None, None) => context
            .as_ref(ctx)
            .selected_conversation_id(ctx)
            .or_else(|| {
                history
                    .as_ref(ctx)
                    .active_conversation_id(terminal_surface_id)
            }),
    };
    let source_conversation = selected_id
        .and_then(|id| history.as_ref(ctx).conversation(&id))
        .cloned();
    if expected_conversation_id.is_some()
        && source_conversation.as_ref().map(AIConversation::id) != expected_conversation_id
    {
        return Err(HandoffPrepareError::SourceConversationChanged);
    }

    let source_conversation = source_conversation.filter(|conversation| !conversation.is_empty());
    let prompt = launch
        .as_ref()
        .map(|launch| launch.prompt.trim().to_owned())
        .unwrap_or_default();
    let source_in_progress = source_conversation
        .as_ref()
        .is_some_and(|conversation| conversation.status().is_in_progress());

    let source_conversation_active = source_conversation.as_ref().is_some_and(|conversation| {
        conversation.status().is_in_progress() || conversation.status().is_blocked()
    });
    let has_active_or_blocked_child = source_conversation.as_ref().is_some_and(|source| {
        descendant_conversation_ids_in_spawn_order(history.as_ref(ctx), source.id())
            .into_iter()
            .filter_map(|id| history.as_ref(ctx).conversation(&id))
            .any(|child| child.status().is_in_progress() || child.status().is_blocked())
    });
    if require_in_progress_source && !source_in_progress {
        return Err(HandoffPrepareError::SourceNotInProgress);
    }
    if source_conversation_active && has_long_running_command {
        return Err(HandoffPrepareError::LongRunningCommand);
    }
    if has_active_or_blocked_child {
        return Err(HandoffPrepareError::ActiveOrBlockedChild);
    }

    let title = source_conversation
        .as_ref()
        .and_then(AIConversation::title)
        .map(|title| format!("{title} (Moved to cloud)"));
    let orchestration_handoff = source_conversation.as_ref().and_then(|conversation| {
        (conversation.is_child_agent_conversation()
            || !history
                .as_ref(ctx)
                .child_conversation_ids_of(&conversation.id())
                .is_empty())
        .then_some(true)
    });
    let mut source_paths = source_conversation
        .as_ref()
        .map(|conversation| {
            let mut paths = extract_paths_from_conversation(conversation);
            paths.extend(
                descendant_conversation_ids_in_spawn_order(history.as_ref(ctx), conversation.id())
                    .into_iter()
                    .filter_map(|id| history.as_ref(ctx).conversation(&id))
                    .filter(|conversation| conversation.status().is_done())
                    .flat_map(extract_paths_from_conversation),
            );
            paths
        })
        .unwrap_or_default();
    if let Some(cwd) = current_working_directory
        && let Ok(path) = StandardizedPath::try_new(&cwd)
        && !source_paths.contains(&path)
    {
        source_paths.push(path);
    }
    if source_conversation.is_none() && prompt.is_empty() && source_paths.is_empty() {
        return Err(HandoffPrepareError::EmptySourceAndPrompt);
    }

    let HandoffLaunchAttachments {
        request_attachments,
        display_attachments,
    } = launch
        .as_ref()
        .map(|launch| launch.attachments.clone())
        .unwrap_or_default();
    let restoration = Some(HandoffRestoration {
        prompt: prompt.clone(),
        attachments: display_attachments,
        environment_id: selected_environment_id,
    });

    if source_conversation_active {
        let conversation_id = source_conversation
            .as_ref()
            .expect("active source conversation exists")
            .id();
        controller.update(ctx, |controller, ctx| {
            controller.cancel_conversation_progress(conversation_id, cancellation_reason, ctx);
        });
    }
    let source_token = source_conversation
        .as_ref()
        .map(|conversation| {
            conversation
                .server_conversation_token()
                .map(|token| token.as_str().to_owned())
                .ok_or(HandoffPrepareError::MissingServerConversationToken)
        })
        .transpose()?;
    if transfer_pending_attachments {
        context.update(ctx, |context, ctx| {
            context.clear_pending_attachments(ctx);
        });
    }

    let environment_selection_is_explicit = selected_environment_id.is_some();
    let environment_id = selected_environment_id.or_else(|| {
        resolve_default_environment_id(ctx)
            .and_then(|id| ServerId::try_from(id.as_str()).ok())
            .map(SyncId::ServerId)
    });
    let valid_environment_ids = CloudAmbientAgentEnvironment::get_all(ctx)
        .into_iter()
        .map(|environment| environment.id)
        .collect();
    let preferences = LLMPreferences::as_ref(ctx);
    let active_model_id = &preferences
        .get_active_base_model(ctx, Some(terminal_surface_id))
        .id;
    let model_id = preferences.cloud_runnable_oz_model_id_or_fallback(active_model_id);
    let model_is_cloud_runnable =
        preferences.is_cloud_runnable_oz_model_id(&LLMId::from(model_id.as_str()));
    let scope = controller.as_ref(ctx).team_context(ctx);
    let computer_use_enabled = resolve_cloud_agent_computer_use_state(&scope, ctx).enabled;
    let config = AgentConfigSnapshot {
        environment_id: environment_id.map(|id| id.to_string()),
        model_id: Some(model_id.clone()),
        computer_use_enabled: Some(computer_use_enabled),
        worker_host: resolve_default_host_slug(&scope, ctx),
        ..Default::default()
    };
    let snapshot_disabled = should_disable_snapshot(ctx);
    let empty_prompt = prompt.is_empty();
    let injection_path = if !empty_prompt {
        HandoffInjectionPath::None
    } else if source_conversation_active {
        HandoffInjectionPath::Continue
    } else {
        HandoffInjectionPath::SnapshotRehydration
    };
    send_telemetry_from_ctx!(
        CloudAgentTelemetryEvent::HandoffInitiated {
            entry_point,
            surface,
            forked_existing_conversation: source_conversation.is_some(),
            empty_prompt,
            injection_path,
        },
        ctx
    );

    Ok(PendingHandoff {
        source_conversation,
        source_conversation_active,
        source_paths,
        source_token,
        title,
        prompt,
        request_attachments,
        restoration,
        selected_environment_id: environment_id,
        environment_required,
        environment_selection_is_explicit,
        valid_environment_ids,
        selected_model_id: model_id,
        model_selection_is_explicit: false,
        model_is_cloud_runnable,
        config,
        snapshot_target,
        snapshot_disabled,
        orchestration_handoff,
    })
}

/// Successful cloud-run creation returned by [`execute_handoff`].
///
/// The frontend uses the task/run identity to begin monitoring the already
/// spawned run and the snapshot fields for telemetry and degraded-upload UI.
pub struct HandoffCreated {
    pub task_id: AmbientAgentTaskId,
    pub run_id: String,
    #[cfg_attr(not(feature = "tui"), allow(dead_code))]
    pub url: String,
    pub at_capacity: bool,
    pub request: SpawnAgentRequest,
    pub derived_workspace_had_content: bool,
    pub snapshot_failed: bool,
}

/// Failure after handoff execution has begun external work.
///
/// Depending on the failed stage, this may include a completed spawn request
/// for existing retry UI and source-input restoration state for a frontend
/// that has not yet materialized its destination.
pub struct HandoffCommitFailure {
    pub issue: CloudAgentStartupIssue,
    pub request: Option<SpawnAgentRequest>,
    pub restoration: Option<HandoffRestoration>,
    pub derived_workspace_had_content: Option<bool>,
    pub snapshot_failed: bool,
}

/// Result of consuming a [`PendingHandoff`] through [`execute_handoff`].
pub enum HandoffCommitOutcome {
    /// Mutable configuration became invalid before any async external work.
    Rejected {
        pending: Box<PendingHandoff>,
        error: HandoffPrepareError,
    },
    /// Fork, materialization, or spawn failed after execution began.
    Failed(HandoffCommitFailure),
    /// The frontend cancelled execution before the cloud run was created.
    Cancelled,
    /// The cloud run was created and is ready for frontend monitoring.
    Created(HandoffCreated),
}

pub fn handoff_dispatch_error(issue: &CloudAgentStartupIssue) -> String {
    match issue {
        CloudAgentStartupIssue::Blocked(CloudAgentStartupBlocker::GitHubAuthRequired {
            message,
            ..
        })
        | CloudAgentStartupIssue::Failed(
            CloudAgentStartupFailure::Capacity { message }
            | CloudAgentStartupFailure::OutOfCredits { message }
            | CloudAgentStartupFailure::ServerOverloaded { message }
            | CloudAgentStartupFailure::Other { message },
        ) => message.clone(),
    }
}

/// State after selecting or creating the server-side conversation fork.
struct ForkedHandoff {
    pending: PendingHandoff,
    forked_conversation_id: Option<String>,
}

/// State after snapshot upload has settled and spawn inputs are complete.
struct SnapshotSettledHandoff {
    spawn_ready: SpawnReadyHandoff,
    forked_conversation_id: Option<String>,
    initial_snapshot_token: Option<InitialSnapshotToken>,
    restoration: Option<HandoffRestoration>,
    derived_workspace_had_content: bool,
    snapshot_failed: bool,
}

/// Consumes a prepared handoff and begins its execution lifecycle.
///
/// Mutable feature, environment, model, and snapshot settings are re-read
/// synchronously before the returned future can perform external work. A
/// headless frontend may supply `caller_cancellation`; a materialized frontend
/// instead receives its cancellation sender through
/// [`HandoffTargetMaterialization`]. These cancellation paths are mutually
/// exclusive.
pub fn execute_handoff(
    mut pending: PendingHandoff,
    ai_client: Arc<dyn AIClient>,
    caller_cancellation: Option<oneshot::Receiver<()>>,
    materialize_handoff_target: Option<MaterializeHandoffTarget>,
    ctx: &AppContext,
) -> Pin<Box<dyn Future<Output = HandoffCommitOutcome> + Send>> {
    pending.set_valid_environment_ids(
        CloudAmbientAgentEnvironment::get_all(ctx)
            .into_iter()
            .map(|environment| environment.id)
            .collect(),
    );
    pending.model_is_cloud_runnable = LLMPreferences::as_ref(ctx)
        .is_cloud_runnable_oz_model_id(&LLMId::from(pending.selected_model_id.as_str()));
    pending.snapshot_disabled = should_disable_snapshot(ctx);
    let validation = if AISettings::as_ref(ctx).is_cloud_handoff_enabled(ctx) {
        pending.validate()
    } else {
        Err(HandoffPrepareError::HandoffDisabled)
    };
    if let Err(error) = validation {
        return Box::pin(async move {
            HandoffCommitOutcome::Rejected {
                pending: Box::new(pending),
                error,
            }
        });
    }

    Box::pin(execute_validated_handoff(
        pending,
        ai_client,
        caller_cancellation,
        materialize_handoff_target,
    ))
}

async fn execute_validated_handoff(
    pending: PendingHandoff,
    ai_client: Arc<dyn AIClient>,
    caller_cancellation: Option<oneshot::Receiver<()>>,
    materialize_handoff_target: Option<MaterializeHandoffTarget>,
) -> HandoffCommitOutcome {
    let mut forked = match fork_source_conversation(pending, &ai_client).await {
        Ok(forked) => forked,
        Err(failure) => return HandoffCommitOutcome::Failed(failure),
    };
    let mut cancellation = caller_cancellation;
    if let Some(materialize_handoff_target) = materialize_handoff_target {
        debug_assert!(
            cancellation.is_none(),
            "caller cancellation and target materialization cancellation are mutually exclusive"
        );
        let (cancel, receiver) = oneshot::channel();
        let materialization = HandoffTargetMaterialization {
            source_conversation: forked.pending.source_conversation.clone(),
            forked_conversation_id: forked.forked_conversation_id.clone(),
            title: forked.pending.title.clone(),
            request: build_spawn_request(
                SpawnReadyHandoff {
                    prompt: forked.pending.prompt.clone(),
                    source_conversation_active: forked.pending.source_conversation_active,
                    config: forked.pending.config.clone(),
                    title: forked.pending.title.clone(),
                    attachments: forked.pending.request_attachments.clone(),
                    snapshot_disabled: forked.pending.snapshot_disabled,
                    orchestration_handoff: forked.pending.orchestration_handoff,
                },
                forked.forked_conversation_id.clone(),
                None,
            ),
            cancel,
        };
        if let Err(error) = materialize_handoff_target(materialization)
            .await
            .context("Failed to materialize handoff target")
        {
            return HandoffCommitOutcome::Failed(HandoffCommitFailure {
                issue: classify_cloud_agent_startup_error(&error),
                request: None,
                restoration: forked.pending.take_restoration(),
                derived_workspace_had_content: None,
                snapshot_failed: false,
            });
        }
        cancellation = Some(receiver);
    }

    let (mut settled, mut cancellation) = match cancellation {
        Some(receiver) => {
            let snapshot = Box::pin(prepare_snapshot_for_spawn(forked));
            let cancellation = Box::pin(receiver);
            match select(snapshot, cancellation).await {
                Either::Left((settled, cancellation)) => {
                    (settled, Some(*Pin::into_inner(cancellation)))
                }
                Either::Right((Ok(()), _)) => return HandoffCommitOutcome::Cancelled,
                Either::Right((Err(_), snapshot)) => (snapshot.await, None),
            }
        }
        None => (prepare_snapshot_for_spawn(forked).await, None),
    };

    if cancellation
        .as_mut()
        .is_some_and(handoff_cancellation_requested)
    {
        return HandoffCommitOutcome::Cancelled;
    }

    let request = build_spawn_request(
        settled.spawn_ready,
        settled.forked_conversation_id,
        settled.initial_snapshot_token,
    );
    let response = ai_client.spawn_agent(request.clone()).await;
    if cancellation
        .as_mut()
        .is_some_and(handoff_cancellation_requested)
    {
        if let Ok(response) = &response {
            let task_id = response.task_id;
            if let Err(error) = ai_client.cancel_ambient_agent_task(&task_id).await {
                report_error!(
                    error.context("Failed to cancel ambient agent task"),
                    extra: { "task_id" => %task_id }
                );
            }
        }
        return HandoffCommitOutcome::Cancelled;
    }
    let response = match response {
        Ok(response) => response,
        Err(error) => {
            return HandoffCommitOutcome::Failed(HandoffCommitFailure {
                issue: classify_cloud_agent_startup_error(&error),
                request: Some(request),
                restoration: settled.restoration.take(),
                derived_workspace_had_content: Some(settled.derived_workspace_had_content),
                snapshot_failed: settled.snapshot_failed,
            });
        }
    };

    HandoffCommitOutcome::Created(HandoffCreated {
        task_id: response.task_id,
        run_id: response.run_id.clone(),
        url: oz_run_url(&response.run_id),
        at_capacity: response.at_capacity,
        request,
        derived_workspace_had_content: settled.derived_workspace_had_content,
        snapshot_failed: settled.snapshot_failed,
    })
}

fn handoff_cancellation_requested(cancellation: &mut oneshot::Receiver<()>) -> bool {
    match cancellation.try_recv() {
        Ok(Some(())) => true,
        Ok(None) | Err(_) => false,
    }
}

/// Forks an existing server conversation or records that this is a fresh launch.
///
/// A fork failure returns restoration state before frontend materialization or
/// agent spawn has occurred.
async fn fork_source_conversation(
    mut pending: PendingHandoff,
    ai_client: &Arc<dyn AIClient>,
) -> Result<ForkedHandoff, HandoffCommitFailure> {
    let forked_conversation_id = match pending.source_token.as_ref() {
        Some(source_token) => match ai_client
            .fork_conversation(source_token.clone(), pending.title.clone())
            .await
        {
            Ok(response) => Some(response.forked_conversation_id),
            Err(error) => {
                return Err(HandoffCommitFailure {
                    issue: classify_cloud_agent_startup_error(&error),
                    request: None,
                    restoration: pending.take_restoration(),
                    derived_workspace_had_content: None,
                    snapshot_failed: false,
                });
            }
        },
        None => None,
    };
    Ok(ForkedHandoff {
        pending,
        forked_conversation_id,
    })
}

/// Uploads workspace state and converts the fork stage into spawn-ready data.
///
/// Snapshot upload failure is non-fatal: the returned stage records the
/// failure and omits the token so execution can still create the cloud run.
async fn prepare_snapshot_for_spawn(forked: ForkedHandoff) -> SnapshotSettledHandoff {
    let PendingHandoff {
        source_conversation: _,
        source_conversation_active,
        source_paths,
        source_token: _,
        title,
        prompt,
        request_attachments,
        restoration,
        selected_environment_id: _,
        environment_required: _,
        environment_selection_is_explicit: _,
        valid_environment_ids: _,
        selected_model_id: _,
        model_selection_is_explicit: _,
        model_is_cloud_runnable: _,
        config,
        snapshot_target,
        snapshot_disabled,
        orchestration_handoff,
    } = forked.pending;
    let (workspace, snapshot_result) = upload_handoff_snapshot(source_paths, snapshot_target).await;
    let derived_workspace_had_content =
        !workspace.repos.is_empty() || !workspace.orphan_files.is_empty();
    let (initial_snapshot_token, snapshot_failed) = match snapshot_result {
        Ok(HandoffUploadResult::Uploaded(token)) => (Some(token), false),
        Ok(HandoffUploadResult::EmptyWorkspace) => (None, false),
        Err(error) => {
            let _ = error;
            log::warn!("Handoff snapshot upload failed; continuing without a snapshot");
            (None, true)
        }
    };
    SnapshotSettledHandoff {
        spawn_ready: SpawnReadyHandoff {
            prompt,
            source_conversation_active,
            config,
            title,
            attachments: request_attachments,
            snapshot_disabled,
            orchestration_handoff,
        },
        forked_conversation_id: forked.forked_conversation_id,
        initial_snapshot_token,
        restoration,
        derived_workspace_had_content,
        snapshot_failed,
    }
}

/// Request inputs that no longer depend on source, fork, or snapshot work.
struct SpawnReadyHandoff {
    prompt: String,
    source_conversation_active: bool,
    config: AgentConfigSnapshot,
    title: Option<String>,
    attachments: Vec<AttachmentInput>,
    snapshot_disabled: bool,
    orchestration_handoff: Option<bool>,
}

fn build_spawn_request(
    handoff: SpawnReadyHandoff,
    forked_conversation_id: Option<String>,
    initial_snapshot_token: Option<InitialSnapshotToken>,
) -> SpawnAgentRequest {
    let SpawnReadyHandoff {
        prompt,
        source_conversation_active,
        config,
        title,
        attachments,
        snapshot_disabled,
        orchestration_handoff,
    } = handoff;
    let has_snapshot_content = initial_snapshot_token
        .as_ref()
        .is_some_and(|token| !token.as_str().is_empty());
    let prompt = (!prompt.trim().is_empty()).then_some(prompt);
    let raw_wire_prompt = match (prompt, source_conversation_active, has_snapshot_content) {
        (Some(prompt), _, _) => Some(prompt),
        (None, true, true) => Some(format!(
            "{HANDOFF_CONTINUE_PROMPT}. {HANDOFF_APPLY_SNAPSHOT_PROMPT}"
        )),
        (None, true, false) => Some(HANDOFF_CONTINUE_PROMPT.to_owned()),
        (None, false, true) => Some(HANDOFF_APPLY_SNAPSHOT_PROMPT.to_owned()),
        (None, false, false) => None,
    };
    let (prompt, mode) = match raw_wire_prompt {
        Some(prompt) => {
            let (prompt, mode) = extract_user_query_mode(prompt);
            (Some(prompt), mode)
        }
        None => (None, Default::default()),
    };

    SpawnAgentRequest {
        prompt,
        mode,
        config: Some(config),
        title,
        team: None,
        skill: None,
        attachments,
        interactive: Some(true),
        parent_run_id: None,
        runtime_skills: Vec::new(),
        referenced_attachments: Vec::new(),
        conversation_id: forked_conversation_id,
        initial_snapshot_token,
        agent_identity_uid: None,
        snapshot_disabled: snapshot_disabled.then_some(true),
        orchestration_handoff,
    }
}

#[cfg(test)]
#[path = "pipeline_tests.rs"]
mod tests;

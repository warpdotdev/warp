use std::collections::HashSet;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use anyhow::Context as _;
use warp_core::send_telemetry_from_ctx;
use warp_util::standardized_path::StandardizedPath;
use warpui::{AppContext, EntityId, ModelHandle, SingletonEntity};

use super::snapshot::{HandoffUploadResult, SnapshotUploadTarget, upload_handoff_snapshot};
use super::touched_repos::{descendant_safe_paths, extract_paths_from_conversation};
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
    CloudAgentStartupIssue, classify_cloud_agent_startup_error, oz_run_url,
    resolve_default_environment_id, resolve_default_host_slug, should_disable_snapshot,
};
use crate::cloud_object::CloudObjectLookup as _;
use crate::server::ids::{ServerId, SyncId};
use crate::server::server_api::ai::{
    AIClient, AgentConfigSnapshot, AttachmentInput, InitialSnapshotToken, SpawnAgentRequest,
};
use crate::settings::AISettings;

const HANDOFF_CONTINUE_WITH_SNAPSHOT_PROMPT: &str =
    "Continue. Apply the workspace changes from my previous session.";
const HANDOFF_CONTINUE_PROMPT: &str = "Continue";
const HANDOFF_APPLY_SNAPSHOT_PROMPT: &str = "Apply the workspace changes from my previous session.";

pub struct HandoffPrepareInput {
    pub terminal_surface_id: EntityId,
    pub expected_conversation_id: Option<AIConversationId>,
    pub history: ModelHandle<BlocklistAIHistoryModel>,
    pub controller: ModelHandle<BlocklistAIController>,
    pub context: ModelHandle<BlocklistAIContextModel>,
    pub current_working_directory: Option<String>,
    pub snapshot_target: SnapshotUploadTarget,
    pub has_long_running_command: bool,
    pub launch: Option<PendingCloudLaunch>,
    pub environment_id: Option<SyncId>,
    pub environment_required: bool,
    pub entry_point: HandoffEntryPoint,
    pub surface: HandoffSurface,
    pub cancellation_reason: CancellationReason,
    pub require_in_progress_source: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HandoffPrepareError {
    SourceConversationChanged,
    EmptySourceAndPrompt,
    SourceNotInProgress,
    LongRunningCommand,
    ActiveOrBlockedChild,
    MissingServerConversationToken,
    HandoffDisabled,
    MissingRequiredEnvironment,
    InvalidEnvironment,
    InvalidModel,
}

#[derive(Clone)]
pub struct HandoffPresentationSnapshot {
    pub source_conversation_id: Option<AIConversationId>,
    pub environment_id: Option<SyncId>,
    pub model_id: String,
    pub forked_existing_conversation: bool,
}

#[derive(Clone)]
pub struct HandoffRestoration {
    pub prompt: String,
    pub attachments: Vec<PendingAttachment>,
    pub environment_id: Option<SyncId>,
}

pub struct HandoffTargetMaterialization {
    pub source_conversation: Option<AIConversation>,
    pub forked_conversation_id: Option<String>,
    pub title: Option<String>,
}

pub type MaterializeHandoffTarget = Box<
    dyn FnOnce(
            HandoffTargetMaterialization,
        ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send>>
        + Send,
>;

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

#[cfg_attr(not(feature = "tui"), allow(dead_code))]
impl PendingHandoff {
    pub fn presentation_snapshot(&self) -> HandoffPresentationSnapshot {
        HandoffPresentationSnapshot {
            source_conversation_id: self.source_conversation.as_ref().map(AIConversation::id),
            environment_id: self.selected_environment_id,
            model_id: self.selected_model_id.clone(),
            forked_existing_conversation: self.source_conversation.is_some(),
        }
    }

    pub fn set_environment_id(&mut self, environment_id: Option<SyncId>, is_explicit: bool) {
        if !is_explicit && self.environment_selection_is_explicit {
            return;
        }
        self.selected_environment_id = environment_id;
        self.environment_selection_is_explicit |= is_explicit;
        self.config.environment_id = environment_id.map(|id| id.to_string());
    }

    pub fn set_model_id(&mut self, model_id: String, is_explicit: bool, ctx: &AppContext) {
        if !is_explicit && self.model_selection_is_explicit {
            return;
        }
        self.model_is_cloud_runnable = LLMPreferences::as_ref(ctx)
            .is_cloud_runnable_oz_model_id(&LLMId::from(model_id.as_str()));
        self.selected_model_id = model_id.clone();
        self.model_selection_is_explicit |= is_explicit;
        self.config.model_id = Some(model_id);
    }
    pub fn refresh_valid_environment_ids(&mut self, valid_environment_ids: HashSet<SyncId>) {
        self.valid_environment_ids = valid_environment_ids;
    }

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

    pub fn take_restoration(&mut self) -> Option<HandoffRestoration> {
        self.restoration.take()
    }
}

pub fn prepare_handoff(
    input: HandoffPrepareInput,
    ctx: &mut AppContext,
) -> Result<PendingHandoff, HandoffPrepareError> {
    let HandoffPrepareInput {
        terminal_surface_id,
        expected_conversation_id,
        history,
        controller,
        context,
        current_working_directory,
        snapshot_target,
        has_long_running_command,
        launch,
        environment_id: selected_environment_id,
        environment_required,
        entry_point,
        surface,
        cancellation_reason,
        require_in_progress_source,
    } = input;

    let selected_id = expected_conversation_id
        .or_else(|| context.as_ref(ctx).selected_conversation_id(ctx))
        .or_else(|| {
            history
                .as_ref(ctx)
                .active_conversation_id(terminal_surface_id)
        });
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
    validate_prepare_guard(
        source_conversation.is_some(),
        prompt.is_empty(),
        require_in_progress_source,
        source_in_progress,
        source_conversation_active,
        has_long_running_command,
        has_active_or_blocked_child,
    )?;

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
            paths.extend(descendant_safe_paths(
                history.as_ref(ctx),
                conversation.id(),
            ));
            paths
        })
        .unwrap_or_default();
    if let Some(cwd) = current_working_directory
        && let Ok(path) = StandardizedPath::try_new(&cwd)
        && !source_paths.contains(&path)
    {
        source_paths.push(path);
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
    context.update(ctx, |context, ctx| {
        context.clear_pending_attachments(ctx);
    });

    let environment_selection_is_explicit = selected_environment_id.is_some();
    let environment_id = selected_environment_id.or_else(|| {
        resolve_default_environment_id(ctx)
            .and_then(|id| ServerId::try_from(id.as_str()).ok())
            .map(SyncId::ServerId)
    });
    let valid_environment_ids = current_valid_environment_ids(ctx);
    let model_id = LLMPreferences::as_ref(ctx)
        .get_active_base_model(ctx, Some(terminal_surface_id))
        .id
        .clone();
    let model_is_cloud_runnable =
        LLMPreferences::as_ref(ctx).is_cloud_runnable_oz_model_id(&model_id);
    let config = AgentConfigSnapshot {
        environment_id: environment_id.map(|id| id.to_string()),
        model_id: Some(model_id.to_string()),
        computer_use_enabled: Some(resolve_cloud_agent_computer_use_state(ctx).enabled),
        worker_host: resolve_default_host_slug(ctx),
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
        selected_model_id: model_id.to_string(),
        model_selection_is_explicit: false,
        model_is_cloud_runnable,
        config,
        snapshot_target,
        snapshot_disabled,
        orchestration_handoff,
    })
}
fn current_valid_environment_ids(ctx: &AppContext) -> HashSet<SyncId> {
    CloudAmbientAgentEnvironment::get_all(ctx)
        .into_iter()
        .map(|environment| environment.id)
        .collect()
}

fn validate_prepare_guard(
    has_source: bool,
    prompt_is_empty: bool,
    require_in_progress_source: bool,
    source_in_progress: bool,
    source_active: bool,
    has_long_running_command: bool,
    has_active_or_blocked_child: bool,
) -> Result<(), HandoffPrepareError> {
    if !has_source && prompt_is_empty {
        return Err(HandoffPrepareError::EmptySourceAndPrompt);
    }
    if require_in_progress_source && !source_in_progress {
        return Err(HandoffPrepareError::SourceNotInProgress);
    }
    if source_active && has_long_running_command {
        return Err(HandoffPrepareError::LongRunningCommand);
    }
    if has_active_or_blocked_child {
        return Err(HandoffPrepareError::ActiveOrBlockedChild);
    }
    Ok(())
}

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

pub struct HandoffCommitFailure {
    pub issue: CloudAgentStartupIssue,
    pub request: Option<SpawnAgentRequest>,
    pub restoration: Option<HandoffRestoration>,
    pub derived_workspace_had_content: Option<bool>,
    pub snapshot_failed: bool,
}

pub enum HandoffCommitOutcome {
    Rejected {
        pending: Box<PendingHandoff>,
        error: HandoffPrepareError,
    },
    Failed(HandoffCommitFailure),
    Created(HandoffCreated),
}

struct ForkedHandoff {
    pending: PendingHandoff,
    forked_conversation_id: Option<String>,
}

struct SnapshotSettledHandoff {
    spawn_ready: SpawnReadyHandoff,
    forked_conversation_id: Option<String>,
    initial_snapshot_token: Option<InitialSnapshotToken>,
    restoration: Option<HandoffRestoration>,
    derived_workspace_had_content: bool,
    snapshot_failed: bool,
}

pub fn commit_handoff(
    mut pending: PendingHandoff,
    ai_client: Arc<dyn AIClient>,
    materialize_handoff_target: Option<MaterializeHandoffTarget>,
    ctx: &AppContext,
) -> Pin<Box<dyn Future<Output = HandoffCommitOutcome> + Send>> {
    pending.refresh_valid_environment_ids(current_valid_environment_ids(ctx));
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

    Box::pin(execute_committed_handoff(
        pending,
        ai_client,
        materialize_handoff_target,
    ))
}

async fn execute_committed_handoff(
    pending: PendingHandoff,
    ai_client: Arc<dyn AIClient>,
    materialize_handoff_target: Option<MaterializeHandoffTarget>,
) -> HandoffCommitOutcome {
    let mut forked = match fork_handoff(pending, &ai_client).await {
        Ok(forked) => forked,
        Err(failure) => return HandoffCommitOutcome::Failed(failure),
    };
    if let Some(materialize_handoff_target) = materialize_handoff_target {
        let materialization = HandoffTargetMaterialization {
            source_conversation: forked.pending.source_conversation.clone(),
            forked_conversation_id: forked.forked_conversation_id.clone(),
            title: forked.pending.title.clone(),
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
    }

    let mut settled = settle_snapshot(forked).await;
    let request = build_spawn_request(
        settled.spawn_ready,
        settled.forked_conversation_id,
        settled.initial_snapshot_token,
    );
    let response = match ai_client.spawn_agent(request.clone()).await {
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

async fn fork_handoff(
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

async fn settle_snapshot(forked: ForkedHandoff) -> SnapshotSettledHandoff {
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
        (None, true, true) => Some(HANDOFF_CONTINUE_WITH_SNAPSHOT_PROMPT.to_owned()),
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

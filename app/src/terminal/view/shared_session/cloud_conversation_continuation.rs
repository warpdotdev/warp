use warp_cli::agent::Harness;
use warpui::{AppContext, EntityId, ModelHandle, SingletonEntity};

use crate::ai::agent::api::ServerConversationToken;
use crate::ai::agent::conversation::{
    AIAgentHarness, AIConversation, AIConversationId, ServerAIConversationMetadata,
};
use crate::ai::agent_conversations_model::AgentConversationsModel;
use crate::ai::ambient_agents::{
    AmbientAgentTask, AmbientAgentTaskId, AmbientConversationStatus,
    conversation_output_status_from_conversation,
};
use crate::ai::blocklist::BlocklistAIHistoryModel;
use crate::auth::AuthStateProvider;
use crate::cloud_object::{Owner, ServerGuestSubject};
use crate::drive::sharing::SharingAccessLevel;
use crate::terminal::TerminalModel;
use crate::terminal::view::ambient_agent::AmbientAgentViewModel;
use crate::workspaces::user_workspaces::UserWorkspaces;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TombstoneCta {
    ContinueLocally { conversation_id: AIConversationId },
    ContinueInCloud { task_id: AmbientAgentTaskId },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::terminal::view) enum CloudConversationContinuationUiState {
    FollowupInput,
    Tombstone { cta: Option<TombstoneCta> },
}

/// How a follow-up prompt for this pane should be routed. Single source of truth shared by the
/// submission router (so a remote cloud conversation never continues on the local agent) and the
/// agent input footer live-VM indicator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AIQueryRouting {
    /// Connected as a live shared-session viewer (an ambient cloud run or a shared session); the
    /// follow-up is forwarded to the sharer via the viewer prompt path. `is_executor` is true when
    /// this viewer may submit (read-only viewers are blocked). `ambient_agent_task_id` is set only
    /// for ambient agent shared sessions (it is `None` for a shared local session), so the footer
    /// shows the live-VM indicator only for ambient runs. Also used for a disconnected pane whose
    /// owned run still has an active execution we can't attach to as a viewer, in which case
    /// `is_executor` is false and the submission router surfaces stale-pane guidance.
    LiveRemoteVm {
        is_executor: bool,
        ambient_agent_task_id: Option<AmbientAgentTaskId>,
    },
    /// Disconnected but resumable owned Oz cloud conversation; the follow-up must start a new cloud
    /// VM via cloud-to-cloud handoff.
    NewCloudVm { task_id: AmbientAgentTaskId },
    /// A finished/non-resumable remote cloud conversation (non-owner finished viewer, blocked
    /// source, non-Oz tombstone). The input is non-editable; a follow-up must never run locally.
    UnconnectedReadOnly,
    /// Continues on the local machine. Covers ordinary local agent panes and local ambient sharers
    /// (e.g. `run_agents(local)` orchestration children, `/remote-control` of a local session).
    Local,
}

impl AIQueryRouting {
    pub(crate) fn is_local(&self) -> bool {
        matches!(self, Self::Local)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::terminal::view) enum CloudConversationContinuationError {
    MissingTask,
    ActiveTaskExecution,
    MissingConversationToken,
    MissingServerConversationMetadata,
    UnknownHarness,
    UnknownConversationAccess,
}

impl CloudConversationContinuationError {
    pub(in crate::terminal::view) fn should_fallback_to_tombstone(self) -> bool {
        !matches!(self, Self::ActiveTaskExecution)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConversationAccess {
    Edit,
    ViewOnly,
    Unknown,
}

pub(in crate::terminal::view) fn resolve_cloud_conversation_continuation_ui_state(
    terminal_view_id: EntityId,
    task_id: AmbientAgentTaskId,
    app: &AppContext,
) -> Result<CloudConversationContinuationUiState, CloudConversationContinuationError> {
    let Some(task) = AgentConversationsModel::as_ref(app).get_task_data(&task_id) else {
        return Err(CloudConversationContinuationError::MissingTask);
    };
    if task.blocks_cloud_followups() {
        return Ok(CloudConversationContinuationUiState::Tombstone { cta: None });
    }
    if task.has_active_execution() {
        return Err(CloudConversationContinuationError::ActiveTaskExecution);
    }
    let is_environment_setup_failure = task.state.is_failure_like()
        && task
            .status_message
            .as_ref()
            .is_some_and(|status_message| status_message.is_environment_setup_failure());
    if is_environment_setup_failure && task.conversation_id().is_none() {
        return Ok(CloudConversationContinuationUiState::Tombstone { cta: None });
    }
    let conversation_token = task
        .conversation_id()
        .map(|token| ServerConversationToken::new(token.to_string()));
    let history_model = BlocklistAIHistoryModel::as_ref(app);

    if let Some(conversation_token) = conversation_token.as_ref()
        && let Some(metadata) =
            history_model.get_server_conversation_metadata_by_server_token(conversation_token)
    {
        return continuation_ui_state_for_harness_and_access(
            metadata.harness,
            conversation_access(metadata, app),
            terminal_view_id,
            Some(conversation_token),
            task_id,
            history_model,
        );
    }

    let access = task_creator_access(&task, app);
    if access == ConversationAccess::Edit {
        return continuation_ui_state_for_harness_and_access(
            task_harness(&task),
            access,
            terminal_view_id,
            conversation_token.as_ref(),
            task_id,
            history_model,
        );
    }

    if conversation_token.is_none() {
        Err(CloudConversationContinuationError::MissingConversationToken)
    } else {
        Err(CloudConversationContinuationError::MissingServerConversationMetadata)
    }
}

/// Resolves the [`AIQueryRouting`] for a pane from its terminal model and optional ambient
/// view model. `terminal_model` must already be locked by the caller; this function does not lock
/// it, and reuses [`resolve_cloud_conversation_continuation_ui_state`] for the disconnected case.
pub(crate) fn resolve_ai_query_routing(
    terminal_view_id: EntityId,
    ambient_agent_view_model: Option<&ModelHandle<AmbientAgentViewModel>>,
    terminal_model: &TerminalModel,
    app: &AppContext,
) -> AIQueryRouting {
    let status = terminal_model.shared_session_status();
    let is_ambient = terminal_model.is_shared_ambient_agent_session()
        || ambient_agent_view_model.is_some_and(|model| model.as_ref(app).is_ambient_agent());
    let is_transcript_viewer = terminal_model.is_conversation_transcript_viewer();
    // The ambient task this pane is associated with, if any. `None` for a shared *local* session,
    // which keeps the footer's live-VM indicator hidden for non-ambient shared sessions.
    let pane_ambient_agent_task_id = ambient_agent_view_model
        .and_then(|model| model.as_ref(app).task_id())
        .or_else(|| terminal_model.ambient_agent_task_id());

    // A live shared-session viewer forwards its follow-up to the sharer via the viewer prompt path,
    // whether the shared session is an ambient cloud run or a shared local session. `is_executor`
    // tells the submission router whether this viewer may actually submit; `ambient_agent_task_id`
    // is set only for ambient runs so the footer indicator stays hidden for shared local sessions.
    if status.is_active_viewer() {
        return AIQueryRouting::LiveRemoteVm {
            is_executor: status.is_executor(),
            ambient_agent_task_id: pane_ambient_agent_task_id,
        };
    }

    // A sharer runs on this machine by definition (e.g. a local orchestration child,
    // `/remote-control` of a local session), so it stays local whatever its conversation says.
    if status.is_active_sharer() {
        return AIQueryRouting::Local;
    }

    // The pane's own ambient association is view state that does not survive the cloud run: once
    // the shared session ends the pane can look like an ordinary local one while still holding a
    // cloud conversation. Ask the conversation itself where its continuation belongs.
    let conversation_association = pane_cloud_association(terminal_view_id, app);
    let ambient_agent_task_id =
        pane_ambient_agent_task_id.or_else(|| conversation_association.task_id());

    // Ordinary local pane: not a cloud/ambient or transcript pane, and holding no cloud
    // conversation.
    if !is_ambient && !is_transcript_viewer && !conversation_association.is_cloud() {
        return AIQueryRouting::Local;
    }

    // Disconnected / ended / transcript ambient pane: defer to the resolved continuation state.
    let Some(task_id) = ambient_agent_task_id else {
        return match conversation_association {
            // The conversation belongs to a cloud run this client cannot name right now. Blocking
            // is recoverable; running the prompt on the user's machine is not.
            PaneCloudAssociation::UnresolvedCloud => AIQueryRouting::UnconnectedReadOnly,
            // No ambient task yet (fresh composing cloud pane, replay/loading, or a generic local
            // transcript): defer to existing local handling.
            PaneCloudAssociation::Task(_) | PaneCloudAssociation::None => AIQueryRouting::Local,
        };
    };

    match resolve_cloud_conversation_continuation_ui_state(terminal_view_id, task_id, app) {
        // Editable, resumable owned Oz cloud conversation -> the follow-up starts a new cloud VM.
        Ok(CloudConversationContinuationUiState::FollowupInput)
            if !terminal_model.is_read_only() =>
        {
            AIQueryRouting::NewCloudVm { task_id }
        }
        // A third-party harness run (Claude Code, Gemini, Codex) that ended but is cloud-resumable
        // surfaces a "Continue" tombstone CTA instead of an inline follow-up input. Clicking
        // Continue clears the finished-viewer read-only state and enables the input, so once the
        // pane is editable a follow-up must start a new cloud VM (cloud-to-cloud handoff) rather
        // than be blocked as read-only. While the pane is still read-only (tombstone shown, not yet
        // continued) this falls through to `UnconnectedReadOnly` below.
        Ok(CloudConversationContinuationUiState::Tombstone {
            cta: Some(TombstoneCta::ContinueInCloud { .. }),
        }) if !terminal_model.is_read_only() => AIQueryRouting::NewCloudVm { task_id },
        // The run still has an active execution but this pane is not attached as a viewer; treat it
        // as a live remote VM (never local) so the submission router surfaces stale-pane guidance
        // rather than letting a follow-up fall through to local submission.
        Err(CloudConversationContinuationError::ActiveTaskExecution) => {
            AIQueryRouting::LiveRemoteVm {
                is_executor: false,
                ambient_agent_task_id: Some(task_id),
            }
        }
        // Any other outcome on an existing cloud task is non-resumable here: read-only, never local.
        Ok(_) | Err(_) => AIQueryRouting::UnconnectedReadOnly,
    }
}

/// What a pane's current conversation says about where its continuation belongs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PaneCloudAssociation {
    /// The conversation belongs to a cloud run, and that run is known.
    Task(AmbientAgentTaskId),
    /// The conversation is one this client is only viewing, but its run cannot be named. The
    /// follow-up still must not run locally.
    UnresolvedCloud,
    /// Nothing ties this conversation to a cloud run.
    None,
}

impl PaneCloudAssociation {
    fn task_id(self) -> Option<AmbientAgentTaskId> {
        match self {
            Self::Task(task_id) => Some(task_id),
            Self::UnresolvedCloud | Self::None => None,
        }
    }

    fn is_cloud(self) -> bool {
        !matches!(self, Self::None)
    }
}

/// Where the pane's current conversation says its continuation belongs, read from the
/// conversation itself rather than from pane view state.
///
/// Only the pane's active conversation is consulted, so a conversation deliberately continued
/// locally supersedes the cloud one it came from: `/continue-locally` and both Continue-locally
/// CTAs fork into a fresh conversation that carries no cloud association. Child agent
/// conversations are excluded because a `run_agents(local)` child is tracked as an ambient task
/// while still executing on this machine.
fn pane_cloud_association(terminal_view_id: EntityId, app: &AppContext) -> PaneCloudAssociation {
    let history_model = BlocklistAIHistoryModel::as_ref(app);
    let Some(conversation) = history_model.active_conversation(terminal_view_id) else {
        return PaneCloudAssociation::None;
    };
    if conversation.is_child_agent_conversation() {
        return PaneCloudAssociation::None;
    }
    // The server stamps `ambient_agent_task_id` on every conversation an ambient run owns, which
    // is the same marker that keeps cloud runs out of local conversation navigation. A
    // local-to-cloud handoff records its run before any metadata snapshot for the forked
    // conversation exists, which covers the window the server marker cannot.
    if let Some(task_id) = conversation
        .server_metadata()
        .and_then(|metadata| metadata.ambient_agent_task_id)
        .or_else(|| conversation.cloud_handoff_task_id())
        .or_else(|| synced_metadata_task_id(conversation, history_model))
    {
        return PaneCloudAssociation::Task(task_id);
    }
    // A conversation this client is only viewing executes on whichever machine shared it, never
    // this one, so an unnameable run must block rather than fall through to the local agent.
    if conversation.is_viewing_shared_session()
        && conversation.server_conversation_token().is_some()
    {
        return PaneCloudAssociation::UnresolvedCloud;
    }
    PaneCloudAssociation::None
}

/// The ambient run named by the synced metadata snapshot for this conversation's server token.
///
/// A conversation restored from the local database carries no `server_metadata` of its own:
/// `BlocklistAIHistoryModel::merge_cloud_conversation_metadata` pushes each snapshot onto the
/// matching conversation only if that conversation is already in memory, and a conversation loaded
/// afterwards never receives it. The snapshot is still held in the model's token-keyed metadata,
/// so ask there before concluding the pane holds nothing cloud.
fn synced_metadata_task_id(
    conversation: &AIConversation,
    history_model: &BlocklistAIHistoryModel,
) -> Option<AmbientAgentTaskId> {
    history_model
        .get_server_conversation_metadata_by_server_token(
            conversation.server_conversation_token()?,
        )?
        .ambient_agent_task_id
}

fn continuation_ui_state_for_harness_and_access(
    harness: AIAgentHarness,
    access: ConversationAccess,
    terminal_view_id: EntityId,
    conversation_token: Option<&ServerConversationToken>,
    task_id: AmbientAgentTaskId,
    history_model: &BlocklistAIHistoryModel,
) -> Result<CloudConversationContinuationUiState, CloudConversationContinuationError> {
    match (harness, access) {
        (AIAgentHarness::Oz, ConversationAccess::Edit) => {
            Ok(CloudConversationContinuationUiState::FollowupInput)
        }
        (AIAgentHarness::Oz, ConversationAccess::ViewOnly) => {
            let cta = conversation_token
                .and_then(|conversation_token| {
                    local_conversation_id_for_local_continuation(
                        terminal_view_id,
                        conversation_token,
                        history_model,
                    )
                })
                .map(|conversation_id| TombstoneCta::ContinueLocally { conversation_id });
            Ok(CloudConversationContinuationUiState::Tombstone { cta })
        }
        (
            AIAgentHarness::ClaudeCode | AIAgentHarness::Gemini | AIAgentHarness::Codex,
            ConversationAccess::Edit,
        ) => Ok(CloudConversationContinuationUiState::Tombstone {
            cta: Some(TombstoneCta::ContinueInCloud { task_id }),
        }),
        (
            AIAgentHarness::ClaudeCode | AIAgentHarness::Gemini | AIAgentHarness::Codex,
            ConversationAccess::ViewOnly,
        ) => Ok(CloudConversationContinuationUiState::Tombstone { cta: None }),
        (AIAgentHarness::Unknown, _) => Err(CloudConversationContinuationError::UnknownHarness),
        (_, ConversationAccess::Unknown) => {
            Err(CloudConversationContinuationError::UnknownConversationAccess)
        }
    }
}

fn conversation_access(
    metadata: &ServerAIConversationMetadata,
    app: &AppContext,
) -> ConversationAccess {
    let Some(current_user_uid) = AuthStateProvider::as_ref(app).get().user_id() else {
        return ConversationAccess::Unknown;
    };

    let mut access_level = SharingAccessLevel::View;
    match metadata.permissions.space {
        Owner::User { user_uid } => {
            let is_current_user_owner = user_uid == current_user_uid;
            if is_current_user_owner {
                access_level = access_level.max(SharingAccessLevel::Full);
            }
        }
        Owner::Team { team_uid } => {
            let is_current_team_owner = UserWorkspaces::as_ref(app)
                .team_from_uid_across_all_workspaces(team_uid)
                .is_some();
            if is_current_team_owner {
                access_level = access_level.max(SharingAccessLevel::Full);
            }
        }
    }

    if let Some(link_sharing) = &metadata.permissions.anyone_link_sharing {
        access_level = access_level.max(link_sharing.access_level.into());
    }

    // Direct user and team ACLs can both apply, so use the highest matching grant.
    for guest in &metadata.permissions.guests {
        match &guest.subject {
            ServerGuestSubject::User { firebase_uid } => {
                let matches_current_user = firebase_uid == current_user_uid.as_str();
                if matches_current_user {
                    access_level = access_level.max(guest.access_level.into());
                }
            }
            ServerGuestSubject::Team { team_uid } => {
                let matches_current_team = UserWorkspaces::as_ref(app)
                    .team_from_uid_across_all_workspaces(*team_uid)
                    .is_some();
                if matches_current_team {
                    access_level = access_level.max(guest.access_level.into());
                }
            }
            ServerGuestSubject::PendingUser { .. } => {}
        }
    }
    let is_creator = metadata
        .metadata
        .creator_uid
        .as_ref()
        .is_some_and(|creator_uid| creator_uid == current_user_uid.as_str());
    if is_creator {
        access_level = access_level.max(SharingAccessLevel::Edit);
    }
    if access_level >= SharingAccessLevel::Edit {
        ConversationAccess::Edit
    } else {
        ConversationAccess::ViewOnly
    }
}

fn task_creator_access(task: &AmbientAgentTask, app: &AppContext) -> ConversationAccess {
    let Some(current_user_uid) = AuthStateProvider::as_ref(app).get().user_id() else {
        return ConversationAccess::Unknown;
    };

    if task
        .creator
        .as_ref()
        .is_some_and(|creator| creator.uid == current_user_uid.as_str())
    {
        ConversationAccess::Edit
    } else {
        ConversationAccess::Unknown
    }
}

fn task_harness(task: &AmbientAgentTask) -> AIAgentHarness {
    match task
        .agent_config_snapshot
        .as_ref()
        .and_then(|config| config.harness.as_ref())
        .map(|harness| harness.harness_type)
        .unwrap_or(Harness::Oz)
    {
        Harness::Oz => AIAgentHarness::Oz,
        Harness::Claude => AIAgentHarness::ClaudeCode,
        Harness::Gemini => AIAgentHarness::Gemini,
        Harness::Codex => AIAgentHarness::Codex,
        Harness::OpenCode | Harness::Unknown => AIAgentHarness::Unknown,
    }
}

pub(in crate::terminal::view) fn conversation_failed_before_task_creation(
    terminal_view_id: EntityId,
    history_model: &BlocklistAIHistoryModel,
) -> bool {
    if history_model.is_terminal_surface_conversation_transcript_viewer(terminal_view_id) {
        return false;
    }
    history_model
        .all_live_conversations_for_terminal_surface(terminal_view_id)
        .next()
        .and_then(conversation_output_status_from_conversation)
        .is_some_and(|status| matches!(status, AmbientConversationStatus::Error { .. }))
}

fn local_conversation_id_for_local_continuation(
    terminal_view_id: EntityId,
    conversation_token: &ServerConversationToken,
    history_model: &BlocklistAIHistoryModel,
) -> Option<AIConversationId> {
    history_model
        .all_live_conversations_for_terminal_surface(terminal_view_id)
        .find(|conversation| conversation.server_conversation_token() == Some(conversation_token))
        .map(|conversation| conversation.id())
        .or_else(|| history_model.find_conversation_id_by_server_token(conversation_token))
}

#[cfg(test)]
#[path = "cloud_conversation_continuation_tests.rs"]
mod tests;

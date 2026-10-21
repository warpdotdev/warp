use std::path::{Path, PathBuf};
use std::time::Duration;

use ai::hooks::{
    CanonicalToolName, ENVELOPE_SCHEMA_VERSION, HOOK_PROTOCOL_VERSION, HookEnvelope,
    HookExecutionHost, HookPayload, HookSurface, PermissionFacts, PromptProvenance, TurnStopStatus,
};
use repo_metadata::repositories::DetectedRepositories;
use uuid::Uuid;
use warp_core::channel::ChannelState;
use warp_core::execution_mode::AppExecutionMode;
use warp_util::local_or_remote_path::LocalOrRemotePath;
use warpui::{AppContext, ModelHandle, SingletonEntity};

use super::HookRuntime;
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::agent::{AIAgentAction, AIAgentActionResultType, CancellationReason};
use crate::ai::blocklist::BlocklistAIHistoryModel;
use crate::auth::AuthStateProvider;
use crate::terminal::model::session::active_session::ActiveSession;

/// Where the boundary fired from, as far as the envelope builders need to know.
pub(crate) struct EnvelopeSite<'a> {
    pub conversation_id: AIConversationId,
    /// The session whose working directory the operation runs in, when the caller has one.
    pub active_session: Option<&'a ModelHandle<ActiveSession>>,
    /// Whether the conversation's latest exchange may be read from the history model. False when
    /// the caller is inside a history-model update, where that read would re-borrow the model.
    read_conversation: bool,
}

impl<'a> EnvelopeSite<'a> {
    pub fn new(
        conversation_id: AIConversationId,
        active_session: &'a ModelHandle<ActiveSession>,
    ) -> Self {
        Self {
            conversation_id,
            active_session: Some(active_session),
            read_conversation: true,
        }
    }

    /// A site for a conversation that is still being created: no session is attached yet and the
    /// history model cannot be read.
    pub fn for_new_conversation(conversation_id: AIConversationId) -> Self {
        Self {
            conversation_id,
            active_session: None,
            read_conversation: false,
        }
    }

    pub fn cwd(&self, ctx: &AppContext) -> Option<PathBuf> {
        self.active_session
            .and_then(|session| session.as_ref(ctx).current_working_directory().cloned())
            .map(PathBuf::from)
    }
}

fn build(site: &EnvelopeSite<'_>, payload: HookPayload, ctx: &AppContext) -> HookEnvelope {
    let cwd = site.cwd(ctx);
    let roots = cwd
        .as_deref()
        .and_then(|cwd| repository_root(cwd, ctx))
        .into_iter()
        .collect();
    let (exchange_id, model_id) = if site.read_conversation {
        latest_exchange_facts(site.conversation_id, ctx)
    } else {
        (None, None)
    };
    let user_email = ctx
        .has_singleton_model::<AuthStateProvider>()
        .then(|| AuthStateProvider::as_ref(ctx).get().user_email())
        .flatten()
        .filter(|email| !email.is_empty());
    let runtime = HookRuntime::as_ref(ctx);
    let run = runtime.run_metadata().cloned();
    let surface = if site.read_conversation {
        surface_for_conversation(site.conversation_id, run.is_some(), ctx)
    } else {
        surface(run.is_some(), ctx)
    };

    HookEnvelope {
        schema_version: ENVELOPE_SCHEMA_VERSION,
        occurrence_id: Uuid::new_v4(),
        invocation_id: Uuid::nil(),
        conversation_id: site.conversation_id,
        turn_id: None,
        exchange_id,
        action_id: None,
        surface,
        run,
        execution_host: HookExecutionHost::Local,
        cwd,
        roots,
        source_revision: runtime.snapshot(ctx).revision,
        model_id,
        user_email,
        warp_version: ChannelState::app_version().unwrap_or("unknown").to_owned(),
        protocol_version: HOOK_PROTOCOL_VERSION,
        permission_facts: PermissionFacts::default(),
        session_env: Vec::new(),
        payload,
    }
}

/// The surface this process fires boundaries from. `has_run_metadata` tells an Oz run apart from
/// a local command-line run, which share an execution mode.
pub(super) fn surface(has_run_metadata: bool, ctx: &AppContext) -> HookSurface {
    if !ctx.has_singleton_model::<AppExecutionMode>() {
        return HookSurface::CliRun;
    }
    let mode = AppExecutionMode::as_ref(ctx);
    if mode.is_autonomous() {
        if has_run_metadata {
            HookSurface::OzRun
        } else {
            HookSurface::CliRun
        }
    } else if mode.is_tui() {
        HookSurface::Tui
    } else {
        HookSurface::Gui
    }
}

/// The surface `conversation_id` fires boundaries from. A child agent conversation in a desktop
/// or TUI process is a hidden local child: nobody can see or approve its actions, even though
/// the process itself is interactive. Runs keep their process surface, since a remote child is
/// an Oz run of its own.
pub(super) fn surface_for_conversation(
    conversation_id: AIConversationId,
    has_run_metadata: bool,
    ctx: &AppContext,
) -> HookSurface {
    let process_surface = surface(has_run_metadata, ctx);
    match process_surface {
        HookSurface::Gui | HookSurface::Tui => {
            let is_child = ctx.has_singleton_model::<BlocklistAIHistoryModel>()
                && BlocklistAIHistoryModel::as_ref(ctx)
                    .conversation(&conversation_id)
                    .is_some_and(|conversation| conversation.is_child_agent_conversation());
            if is_child {
                HookSurface::LocalChild
            } else {
                process_surface
            }
        }
        HookSurface::OzRun | HookSurface::CliRun | HookSurface::LocalChild => process_surface,
    }
}

fn repository_root(cwd: &Path, ctx: &AppContext) -> Option<PathBuf> {
    if !ctx.has_singleton_model::<DetectedRepositories>() {
        return None;
    }
    DetectedRepositories::as_ref(ctx)
        .get_root_for_path(&LocalOrRemotePath::Local(cwd.to_path_buf()))
        .and_then(|root| root.to_local_path().map(Path::to_path_buf))
}

fn latest_exchange_facts(
    conversation_id: AIConversationId,
    ctx: &AppContext,
) -> (Option<Uuid>, Option<ai::LLMId>) {
    if !ctx.has_singleton_model::<BlocklistAIHistoryModel>() {
        return (None, None);
    }
    let Some(exchange) = BlocklistAIHistoryModel::as_ref(ctx)
        .conversation(&conversation_id)
        .and_then(|conversation| conversation.latest_exchange())
    else {
        return (None, None);
    };
    let exchange_id = Uuid::parse_str(&exchange.id.to_string()).ok();
    (exchange_id, Some(exchange.model_id.clone()))
}

pub(crate) fn envelope_for_tool_pre(
    action: &AIAgentAction,
    site: &EnvelopeSite<'_>,
    ctx: &AppContext,
) -> HookEnvelope {
    let mut envelope = build(
        site,
        HookPayload::ToolPre {
            action: action.action.clone(),
            tool_name: CanonicalToolName::for_action(&action.action),
        },
        ctx,
    );
    envelope.action_id = Some(action.id.clone());
    envelope
}

pub(crate) fn envelope_for_tool_result(
    action: &AIAgentAction,
    result: &AIAgentActionResultType,
    site: &EnvelopeSite<'_>,
    ctx: &AppContext,
) -> HookEnvelope {
    let mut envelope = build(
        site,
        HookPayload::ToolResult {
            action: action.action.clone(),
            tool_name: CanonicalToolName::for_action(&action.action),
            result: result.clone(),
            duration: None,
        },
        ctx,
    );
    envelope.action_id = Some(action.id.clone());
    envelope
}

pub(crate) fn envelope_for_tool_failure(
    action: &AIAgentAction,
    result: &AIAgentActionResultType,
    cancellation_reason: Option<CancellationReason>,
    site: &EnvelopeSite<'_>,
    ctx: &AppContext,
) -> HookEnvelope {
    let mut envelope = build(
        site,
        HookPayload::ToolFailure {
            action: action.action.clone(),
            tool_name: CanonicalToolName::for_action(&action.action),
            result: result.clone(),
            cancellation_reason: cancellation_reason
                .map(|reason| cancellation_reason_name(reason).to_owned()),
            duration: None,
        },
        ctx,
    );
    envelope.action_id = Some(action.id.clone());
    envelope
}

/// The name hooks see for a cancellation reason. Scripts match on these strings, so they are
/// fixed here rather than derived from the enum's spelling.
pub(crate) fn cancellation_reason_name(reason: CancellationReason) -> &'static str {
    match reason {
        CancellationReason::ManuallyCancelled => "user_cancelled",
        CancellationReason::AutomaticCloudHandoff => "cloud_handoff",
        CancellationReason::FollowUpSubmitted { .. } => "follow_up_submitted",
        CancellationReason::UserCommandExecuted => "user_ran_command",
        CancellationReason::Reverted => "reverted",
        CancellationReason::Deleted => "conversation_deleted",
        CancellationReason::CommandFinishedDuringInlineAgentView => "command_finished",
        CancellationReason::CLISubagentUserTakeover => "user_took_control",
        CancellationReason::AgentExitedShell => "shell_exited",
    }
}

pub(crate) fn envelope_for_prompt_submit(
    text: String,
    attachments_summary: Vec<String>,
    provenance: PromptProvenance,
    site: &EnvelopeSite<'_>,
    ctx: &AppContext,
) -> HookEnvelope {
    build(
        site,
        HookPayload::PromptSubmit {
            text,
            attachments_summary,
            provenance,
        },
        ctx,
    )
}

pub(crate) fn envelope_for_turn_stop(
    status: TurnStopStatus,
    continuation_count: u32,
    site: &EnvelopeSite<'_>,
    ctx: &AppContext,
) -> HookEnvelope {
    build(
        site,
        HookPayload::TurnStop {
            status,
            continuation_count,
        },
        ctx,
    )
}

pub(crate) fn envelope_for_session_start(
    is_background_agent: bool,
    site: &EnvelopeSite<'_>,
    ctx: &AppContext,
) -> HookEnvelope {
    build(
        site,
        HookPayload::SessionStart {
            is_background_agent,
        },
        ctx,
    )
}

pub(crate) fn envelope_for_response_complete(
    message_id: String,
    text: String,
    site: &EnvelopeSite<'_>,
    ctx: &AppContext,
) -> HookEnvelope {
    build(
        site,
        HookPayload::ResponseComplete { message_id, text },
        ctx,
    )
}

pub(crate) fn envelope_for_reasoning_complete(
    message_id: String,
    text: String,
    duration: Option<Duration>,
    site: &EnvelopeSite<'_>,
    ctx: &AppContext,
) -> HookEnvelope {
    build(
        site,
        HookPayload::ReasoningComplete {
            message_id,
            text,
            duration,
        },
        ctx,
    )
}

#[cfg(test)]
#[path = "envelope_tests.rs"]
mod tests;

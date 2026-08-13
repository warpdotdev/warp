//! Builders for a shared-session **viewer** pane.
//!
//! A viewer pane is only interesting when its `TerminalView`, its `Network`, and the
//! `TerminalManager` subscriptions between them are all wired together, because that is the path
//! a submitted prompt actually travels. Constructing that by hand is verbose enough that tests
//! tend to poke internal state instead; these builders exist so they don't have to.

use std::sync::Arc;

use parking_lot::FairMutex;
use session_sharing_protocol::common::AgentPromptRequest;
use session_sharing_protocol::viewer::{DownstreamMessage, FailedToJoinReason, UpstreamMessage};
use warpui::platform::WindowStyle;
use warpui::{App, ModelHandle, SingletonEntity, ViewHandle};

use super::terminal::{add_window_with_terminal, initialize_app_for_terminal_view};
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::agent_conversations_model::AgentConversationsModel;
use crate::ai::ambient_agents::task::{AmbientAgentTask, TaskPrincipalInfo};
use crate::ai::ambient_agents::{AgentSource, AmbientAgentTaskId};
use crate::ai::blocklist::BlocklistAIHistoryModel;
use crate::ai::blocklist::agent_view::AgentViewEntryOrigin;
use crate::auth::user::TEST_USER_UID;
use crate::context_chips::prompt_type::PromptType;
use crate::server::server_api::ai::AmbientAgentTaskState;
use crate::settings::WarpPromptSeparator;
use crate::terminal::event_listener::ChannelEventListener;
use crate::terminal::model::session::SessionId as TerminalSessionId;
use crate::terminal::shared_session::manager::Manager as SharedSessionManager;
use crate::terminal::shared_session::shared_handlers::RemoteUpdateGuard;
use crate::terminal::shared_session::viewer::TerminalManager;
use crate::terminal::shared_session::viewer::network::{Network, Stage};
use crate::terminal::shared_session::{SharedSessionSource, SharedSessionStatus};
use crate::terminal::{TerminalModel, TerminalView};
use crate::workspace::ToastStack;

/// What the viewer is allowed to do in the shared session. Only an executor may submit prompts,
/// so the reader variant exists to assert that ineligible viewers are turned away.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewerRole {
    Executor,
    Reader,
}

impl ViewerRole {
    fn status(self) -> SharedSessionStatus {
        match self {
            ViewerRole::Executor => SharedSessionStatus::executor(),
            ViewerRole::Reader => SharedSessionStatus::reader(),
        }
    }
}

/// A viewer pane with its network attached and the manager subscriptions installed.
pub struct ViewerPane {
    pub view: ViewHandle<TerminalView>,
    pub conversation_id: AIConversationId,
    pub network: ModelHandle<Network>,
    /// The slot the manager reads the live network from. Swapping it models the network
    /// replacement that `attach_execution_session` performs on a fatal disconnect.
    pub current_network: Arc<FairMutex<Option<ModelHandle<Network>>>>,
    pub model: Arc<FairMutex<TerminalModel>>,
}

impl ViewerPane {
    /// Replaces the live network with `network`, as a fatal disconnect followed by a new
    /// execution session would. Events from the previous network must then be ignored.
    pub fn set_current_network(&self, network: Option<ModelHandle<Network>>) {
        *self.current_network.lock() = network;
    }
}

/// Builds an executor viewer pane whose network is in `stage`.
pub fn viewer_pane(app: &mut App, stage: Stage) -> ViewerPane {
    viewer_pane_with_role(app, stage, ViewerRole::Executor)
}

/// Who owns the ambient task the pane is viewing. Ownership is what makes a fatal disconnect
/// eligible to continue as a cloud follow-up, so the non-owner case is the ineligible path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AmbientTaskOwner {
    CurrentUser,
    SomeoneElse,
}

/// An ambient (cloud) viewer pane: a viewer pane that is also watching a cloud agent task, which
/// is the only shape in which a fatal disconnect can hand the queue to a cloud follow-up.
pub struct AmbientViewerPane {
    pub pane: ViewerPane,
    pub task_id: AmbientAgentTaskId,
}

/// Builds an ambient viewer pane whose network is in `stage`, live on `task_id`.
///
/// The pane is constructed in cloud mode so it carries an `AmbientAgentViewModel` up front, the
/// task is seeded into `AgentConversationsModel` with `owner` as its creator, and the model is
/// pointed at the live session. Until that session ends the pane is *not* follow-up eligible,
/// which is the state a fatal disconnect transitions out of.
pub fn ambient_viewer_pane(
    app: &mut App,
    stage: Stage,
    owner: AmbientTaskOwner,
) -> AmbientViewerPane {
    initialize_app_for_terminal_view(app);
    app.add_singleton_model(|_| ToastStack);

    let tips_model = app.add_model(|_| Default::default());
    let (_window_id, view) = app.add_window(WindowStyle::NotStealFocus, move |ctx| {
        TerminalView::new_for_test_with_cloud_mode(
            tips_model, None, /* is_cloud_mode */ true, ctx,
        )
    });

    let creator_uid = match owner {
        AmbientTaskOwner::CurrentUser => TEST_USER_UID,
        AmbientTaskOwner::SomeoneElse => "a-different-user",
    };
    let task = owned_cloud_task(creator_uid);
    let task_id = task.task_id;
    AgentConversationsModel::handle(app).update(app, |model, _| {
        model.insert_task_for_test(task);
    });

    let pane = finish_viewer_pane(app, view, stage, ViewerRole::Executor);
    // Marks the pane as viewing a cloud run, which is what routes a fatal disconnect through the
    // resumable ambient path rather than generic viewer teardown.
    pane.model
        .lock()
        .set_shared_session_source(SharedSessionSource::ambient_agent(Some(
            task_id.to_string(),
        )));
    let session_id = pane.network.read(app, |network, _| network.session_id());
    pane.view.update(app, |view, ctx| {
        view.begin_viewing_ambient_session(task_id, session_id, ctx);
    });
    flush(app);

    AmbientViewerPane { pane, task_id }
}

/// Ends the pane's live execution the way reconnect exhaustion does: the server refuses the
/// rejoin, the network gives up, and the manager routes the ambient pane through its resumable
/// execution-ended path. This is the trigger that can hand the queue to a cloud follow-up.
pub fn exhaust_reconnect(app: &mut App, pane: &ViewerPane) {
    inject_downstream(
        app,
        &pane.network.clone(),
        DownstreamMessage::FailedToJoin {
            reason: FailedToJoinReason::SessionNotFound,
        },
    );
}

/// A finished cloud-mode task created by `creator_uid`.
fn owned_cloud_task(creator_uid: &str) -> AmbientAgentTask {
    let now = chrono::Utc::now();
    AmbientAgentTask {
        task_id: uuid::Uuid::new_v4()
            .to_string()
            .parse()
            .expect("a generated uuid parses as a task id"),
        parent_run_id: None,
        title: "Cloud task".to_string(),
        state: AmbientAgentTaskState::Succeeded,
        prompt: "test".to_string(),
        created_at: now,
        started_at: Some(now),
        updated_at: now,
        run_time: Some("PT1S".parse().expect("a literal duration parses")),
        status_message: None,
        source: Some(AgentSource::CloudMode),
        execution_location: None,
        session_id: None,
        session_link: None,
        creator: Some(TaskPrincipalInfo {
            creator_type: "USER".to_string(),
            uid: creator_uid.to_string(),
            display_name: None,
        }),
        executor: None,
        conversation_id: None,
        request_usage: None,
        is_sandbox_running: false,
        agent_config_snapshot: None,
        artifacts: vec![],
        last_event_sequence: None,
        children: vec![],
    }
}

/// Builds a viewer pane in `stage` with an explicit `role`.
pub fn viewer_pane_with_role(app: &mut App, stage: Stage, role: ViewerRole) -> ViewerPane {
    initialize_app_for_terminal_view(app);
    app.add_singleton_model(|_| ToastStack);

    let view = add_window_with_terminal(app, None);
    finish_viewer_pane(app, view, stage, role)
}

/// Turns an already-created `TerminalView` into a wired viewer pane. Shared by the plain and
/// ambient variants so the two cannot drift in how they attach the network and subscriptions.
fn finish_viewer_pane(
    app: &mut App,
    view: ViewHandle<TerminalView>,
    stage: Stage,
    role: ViewerRole,
) -> ViewerPane {
    // The viewer teardown paths talk to the shared-session manager, so it has to exist before a
    // session can end.
    app.add_singleton_model(SharedSessionManager::new);
    let terminal_view_id = view.id();
    let model = view.read(app, |view, _| view.model.clone());
    {
        let mut model = model.lock();
        model.block_list_mut().set_bootstrapped();
        model
            .block_list_mut()
            .active_block_for_test()
            .set_session_id(TerminalSessionId::from(0));
        model.set_shared_session_status(role.status());
    }

    // Entering agent view is what makes a conversation *selected*, which is how the submission
    // path resolves the queue that owns a fallback row.
    let conversation_id = view.update(app, |view, ctx| {
        view.agent_view_controller().update(ctx, |controller, ctx| {
            controller
                .try_enter_agent_view(
                    None,
                    AgentViewEntryOrigin::Input {
                        was_prompt_autodetected: false,
                    },
                    ctx,
                )
                .expect("the pane can enter agent view")
        })
    });
    BlocklistAIHistoryModel::handle(app).update(app, |history, ctx| {
        history.set_active_conversation_id(conversation_id, terminal_view_id, ctx);
    });

    let network = attach_network(app, &view, stage);
    let current_network = Arc::new(FairMutex::new(Some(network.clone())));
    app.update(|ctx| {
        TerminalManager::handle_view_events(
            current_network.clone(),
            &view,
            model.clone(),
            RemoteUpdateGuard::new(),
            ctx,
        );
    });
    subscribe_network_events(app, &view, &model, &current_network, &network);

    ViewerPane {
        view,
        conversation_id,
        network,
        current_network,
        model,
    }
}

/// Installs the manager's inbound subscription for `network`, so a message injected into it
/// reaches the view the same way a real server message would.
///
/// Called for the pane's initial network, and again by the test for a replacement network so a
/// stale event from the old one can be shown to be ignored.
pub fn subscribe_network_events(
    app: &mut App,
    view: &ViewHandle<TerminalView>,
    model: &Arc<FairMutex<TerminalModel>>,
    current_network: &Arc<FairMutex<Option<ModelHandle<Network>>>>,
    network: &ModelHandle<Network>,
) {
    let prompt_type =
        app.add_model(|_| PromptType::new_static(vec![], false, WarpPromptSeparator::None));
    app.update(|ctx| {
        TerminalManager::handle_network_events(
            network,
            view,
            model.clone(),
            current_network.clone(),
            prompt_type,
            RemoteUpdateGuard::new(),
            Arc::new(FairMutex::new(None)),
            /* enable_orchestration_polling */ false,
            ctx,
        );
    });
}

/// Builds an additional `Network` for `view` without installing it as the live one. Used to model
/// the replacement session created after a fatal disconnect.
pub fn attach_network(
    app: &mut App,
    view: &ViewHandle<TerminalView>,
    stage: Stage,
) -> ModelHandle<Network> {
    let model = view.read(app, |view, _| view.model.clone());
    let channel_event_proxy = ChannelEventListener::new_for_test();
    let (_write_to_pty_tx, write_to_pty_rx) = async_channel::unbounded();
    let network = app.add_model(|ctx| {
        Network::new_for_test(
            channel_event_proxy,
            view.downgrade(),
            model,
            write_to_pty_rx,
            RemoteUpdateGuard::new(),
            ctx,
        )
    });
    network.update(app, |network, _| {
        network.stage = stage;
    });
    network
}

/// A network stage midway through a reconnect: the pane still reports itself an active viewer,
/// but nothing can actually be sent.
pub fn reconnecting_stage() -> Stage {
    let (abort_handle, _registration) = futures_util::stream::AbortHandle::new_pair();
    Stage::Reconnecting { abort_handle }
}

/// Types `prompt` and submits it through the real routing path, then lets the resulting events
/// propagate. The submission crosses `Input` -> `TerminalView` -> `TerminalManager`, and each hop
/// is delivered on an effect flush.
pub fn submit_viewer_prompt(app: &mut App, view: &ViewHandle<TerminalView>, prompt: &str) {
    let input = view.read(app, |view, _| view.input().clone());
    input.update(app, |input, ctx| {
        input.replace_buffer_content(prompt, ctx);
    });
    input.update(app, |input, ctx| {
        input.maybe_route_ai_query_to_remote_target(ctx);
    });
    flush(app);
}

/// Drives a server message through the real inbound path on `network`.
pub fn inject_downstream(
    app: &mut App,
    network: &ModelHandle<Network>,
    message: DownstreamMessage,
) {
    network.update(app, |network, ctx| {
        network.inject_downstream_message_for_test(message, ctx);
    });
    flush(app);
}

/// Runs pending effects so queued emissions are delivered.
pub fn flush(app: &mut App) {
    app.update(|_| ());
    app.update(|_| ());
}

/// Every agent prompt that reached `network`'s outbound channel, draining it. The channel also
/// carries CRDT input updates for the same submission, so prompts have to be picked out rather
/// than assumed to be first.
pub fn drain_agent_prompts(app: &App, network: &ModelHandle<Network>) -> Vec<AgentPromptRequest> {
    let ws_proxy_rx = network.read(app, |network, _| network.ws_proxy_rx.clone());
    let mut requests = Vec::new();
    while let Ok(message) = ws_proxy_rx.try_recv() {
        if let UpstreamMessage::SendAgentPrompt(request) = message {
            requests.push(request);
        }
    }
    requests
}

/// The single agent prompt that reached `network`, asserting there is exactly one.
pub fn sent_agent_prompt(app: &App, network: &ModelHandle<Network>) -> AgentPromptRequest {
    let mut requests = drain_agent_prompts(app, network);
    assert_eq!(
        requests.len(),
        1,
        "expected exactly one agent prompt to reach the network"
    );
    requests.remove(0)
}

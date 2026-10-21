use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use ai::hooks::test_util::{RecordingAdapter, RecordingMessenger, ScriptedRunner};
use ai::hooks::{
    ActiveBinding, BindingScope, DeclaredBinding, HookDiagnostic, HookRegistry, HookRunner,
};
use async_channel::unbounded;
use chrono::Local;
use parking_lot::FairMutex;
use warp_multi_agent_api::client_action::{AddMessagesToTask, CreateTask};
use warp_multi_agent_api::message::{AgentOutput, Message as MessageKind};
use warp_multi_agent_api::response_event::{StreamFinished, StreamInit, stream_finished};
use warp_multi_agent_api::{
    ClientAction, Message, ResponseEvent, Task, client_action, response_event,
};
use warpui::r#async::Timer;
use warpui::{App, AppContext, Entity, EntityId, ModelHandle, SingletonEntity, ViewHandle};

use super::HookRuntime;
use crate::ai::agent::conversation::{AIConversation, AIConversationId, ConversationStatus};
use crate::ai::agent::task::TaskId;
use crate::ai::agent::{AIAgentAction, AIAgentActionId, AIAgentActionType, ReadFilesRequest};
use crate::ai::blocklist::history_model::ConversationStatusUpdate;
use crate::ai::blocklist::{
    BlocklistAIActionModel, BlocklistAIController, BlocklistAIHistoryEvent,
    BlocklistAIHistoryModel, RecordingController, RequestInput, ResponseStream, ResponseStreamId,
};
use crate::ai::get_relevant_files::controller::GetRelevantFilesController;
use crate::ai::llms::LLMId;
use crate::terminal::TerminalView;
use crate::terminal::event::BlockMetadataReceivedEvent;
use crate::terminal::model::block::BlockMetadata;
use crate::terminal::model::session::Sessions;
use crate::terminal::model::session::active_session::ActiveSession;
use crate::terminal::model::terminal_model::{BlockIndex, TerminalModel};
use crate::terminal::model_events::{ModelEvent, ModelEventDispatcher};
use crate::test_util::terminal::{add_window_with_terminal, initialize_app_for_terminal_view};
use crate::workspaces::user_workspaces::UserWorkspaces;

/// A terminal view in a window with the hook doubles installed, for tests that drive the
/// controller the way a person or a response stream would.
pub(crate) struct ControllerHarness {
    pub hooks: TestHooks,
    pub terminal: ViewHandle<TerminalView>,
}

pub(crate) fn controller_harness(app: &mut App) -> ControllerHarness {
    initialize_app_for_terminal_view(app);
    let hooks = app.update(install_test_hooks);
    let terminal = add_window_with_terminal(app, None);
    ControllerHarness { hooks, terminal }
}

impl ControllerHarness {
    pub(crate) fn terminal_id(&self) -> EntityId {
        self.terminal.id()
    }

    pub(crate) fn controller(&self, app: &App) -> ModelHandle<BlocklistAIController> {
        self.terminal
            .read(app, |view, _| view.ai_controller().clone())
    }

    pub(crate) fn update_controller<T>(
        &self,
        app: &mut App,
        update: impl FnOnce(
            &mut BlocklistAIController,
            &mut warpui::ModelContext<BlocklistAIController>,
        ) -> T,
    ) -> T {
        let controller = self.controller(app);
        controller.update(app, update)
    }

    /// A new conversation with one request in flight on a stream the test drives by hand.
    pub(crate) fn mock_stream(
        &self,
        app: &mut App,
    ) -> (AIConversationId, ModelHandle<ResponseStream>) {
        self.terminal.update(app, |view, ctx| {
            let terminal_surface_id = view.id();
            let stream_id = ResponseStreamId::new_for_test();
            let conversation_id =
                BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                    let conversation_id = history.start_new_conversation(
                        terminal_surface_id,
                        false,
                        false,
                        false,
                        ctx,
                    );
                    let task_id = history
                        .conversation(&conversation_id)
                        .unwrap()
                        .get_root_task_id()
                        .clone();
                    history
                        .update_conversation_for_new_request_input_for_test(
                            RequestInput {
                                conversation_id,
                                input_messages: Vec::from([(task_id, vec![])]),
                                working_directory: None,
                                model_id: LLMId::from("test-model"),
                                coding_model_id: LLMId::from("test-coding-model"),
                                cli_agent_model_id: LLMId::from("test-cli-agent-model"),
                                computer_use_model_id: LLMId::from("test-computer-use-model"),
                                shared_session_response_initiator: None,
                                request_start_ts: Local::now(),
                                supported_tools_override: None,
                            },
                            stream_id.clone(),
                            terminal_surface_id,
                            ctx,
                        )
                        .unwrap();
                    history.update_conversation_status(
                        terminal_surface_id,
                        conversation_id,
                        ConversationStatus::InProgress,
                        ctx,
                    );
                    conversation_id
                });
            let stream = ctx.add_model(|_| ResponseStream::new_for_test(stream_id.clone()));
            view.ai_controller().update(ctx, |controller, ctx| {
                controller.register_mock_stream_for_test(
                    stream_id,
                    conversation_id,
                    stream.clone(),
                    ctx,
                );
            });
            (conversation_id, stream)
        })
    }
}

pub(crate) fn root_task_id(app: &App, conversation_id: AIConversationId) -> String {
    BlocklistAIHistoryModel::handle(app).read(app, |history, _| {
        history
            .conversation(&conversation_id)
            .expect("conversation exists")
            .get_root_task_id()
            .to_string()
    })
}

pub(crate) fn conversation_status(
    app: &App,
    conversation_id: AIConversationId,
) -> Option<ConversationStatus> {
    BlocklistAIHistoryModel::handle(app).read(app, |history, _| {
        history
            .conversation(&conversation_id)
            .map(|conversation| conversation.status().clone())
    })
}

pub(crate) fn exchange_count(app: &App, conversation_id: AIConversationId) -> usize {
    BlocklistAIHistoryModel::handle(app).read(app, |history, _| {
        history
            .conversation(&conversation_id)
            .map_or(0, |conversation| conversation.exchange_count())
    })
}

pub(crate) fn stream_init_event() -> ResponseEvent {
    ResponseEvent {
        r#type: Some(response_event::Type::Init(StreamInit {
            request_id: "request".to_owned(),
            conversation_id: "test-server-conversation".to_owned(),
            run_id: String::new(),
        })),
    }
}

pub(crate) fn text_message(task_id: &str, text: &str) -> Message {
    Message {
        id: uuid::Uuid::new_v4().to_string(),
        task_id: task_id.to_owned(),
        request_id: "request".to_owned(),
        timestamp: Some(prost_types::Timestamp::from(SystemTime::now())),
        message: Some(MessageKind::AgentOutput(AgentOutput {
            text: text.to_owned(),
        })),
        ..Default::default()
    }
}

/// A batch that creates the root task (the stream's first action does this in production)
/// and adds `messages` to it.
pub(crate) fn create_task_with_messages(task_id: &str, messages: Vec<Message>) -> ResponseEvent {
    ResponseEvent {
        r#type: Some(response_event::Type::ClientActions(
            response_event::ClientActions {
                actions: vec![
                    ClientAction {
                        action: Some(client_action::Action::CreateTask(CreateTask {
                            task: Some(Task {
                                id: task_id.to_owned(),
                                ..Default::default()
                            }),
                        })),
                    },
                    ClientAction {
                        action: Some(client_action::Action::AddMessagesToTask(
                            AddMessagesToTask {
                                task_id: task_id.to_owned(),
                                messages,
                            },
                        )),
                    },
                ],
            },
        )),
    }
}

pub(crate) fn done_event() -> ResponseEvent {
    ResponseEvent {
        r#type: Some(response_event::Type::Finished(StreamFinished {
            reason: Some(stream_finished::Reason::Done(stream_finished::Done {})),
            ..Default::default()
        })),
    }
}

/// Streams one text message and ends the stream cleanly, the way a turn with no tool calls
/// ends.
pub(crate) fn finish_turn_with_text(
    app: &mut App,
    conversation_id: AIConversationId,
    stream: &ModelHandle<ResponseStream>,
    text: &str,
) {
    let task_id = root_task_id(app, conversation_id);
    stream.update(app, |stream, ctx| {
        stream.emit_response_event_for_test(stream_init_event(), ctx);
        stream.emit_response_event_for_test(
            create_task_with_messages(&task_id, vec![text_message(&task_id, text)]),
            ctx,
        );
        stream.emit_response_event_for_test(done_event(), ctx);
        stream.emit_after_stream_finished_for_test(ctx);
    });
}

/// Every status change the history model reports for one conversation, in order.
pub(crate) struct StatusRecorder {
    conversation_id: AIConversationId,
    statuses: Arc<Mutex<Vec<ConversationStatus>>>,
}

impl Entity for StatusRecorder {
    type Event = ();
}

pub(crate) fn record_statuses(
    app: &mut App,
    conversation_id: AIConversationId,
) -> (
    ModelHandle<StatusRecorder>,
    Arc<Mutex<Vec<ConversationStatus>>>,
) {
    let statuses = Arc::new(Mutex::new(Vec::new()));
    let recorder = app.add_model(|ctx: &mut warpui::ModelContext<StatusRecorder>| {
        ctx.subscribe_to_model(&BlocklistAIHistoryModel::handle(ctx), |me, _, event, _| {
            if let BlocklistAIHistoryEvent::UpdatedConversationStatus {
                conversation_id: id,
                update: ConversationStatusUpdate::Changed { .. },
                new_status,
                ..
            } = event
                && *id == me.conversation_id
            {
                me.statuses.lock().unwrap().push(new_status.clone());
            }
        });
        StatusRecorder {
            statuses: statuses.clone(),
            conversation_id,
        }
    });
    (recorder, statuses)
}

/// A terminal-view-sized app with the hook doubles installed and one native conversation.
pub(crate) struct HookTestApp {
    pub hooks: TestHooks,
    pub active_session: ModelHandle<ActiveSession>,
    pub dispatcher: ModelHandle<ModelEventDispatcher>,
    pub conversation_id: AIConversationId,
    pub terminal_view_id: warpui::EntityId,
}

pub(crate) fn hook_test_app(app: &mut App) -> HookTestApp {
    initialize_app_for_terminal_view(app);
    let hooks = app.update(install_test_hooks);
    let (active_session, dispatcher) = mock_session(app);
    let terminal_view_id = warpui::EntityId::new();
    let conversation = AIConversation::new(false, false);
    let conversation_id = conversation.id();
    app.update(|ctx| {
        BlocklistAIHistoryModel::handle(ctx).update(ctx, |model, ctx| {
            model.restore_conversations(terminal_view_id, vec![conversation], ctx);
        });
    });
    HookTestApp {
        hooks,
        active_session,
        dispatcher,
        conversation_id,
        terminal_view_id,
    }
}

/// A session over a mock terminal, with the dispatcher that feeds it, for envelopes that
/// need a working directory to read.
fn mock_session(
    app: &mut App,
) -> (
    ModelHandle<ActiveSession>,
    ModelHandle<ModelEventDispatcher>,
) {
    let sessions = app.add_model(|_| Sessions::new_for_test());
    let (_tx, rx) = unbounded();
    let dispatcher = app.add_model(|ctx| ModelEventDispatcher::new(rx, sessions.clone(), ctx));
    let active_session = app.add_model(|ctx| ActiveSession::new(sessions, dispatcher.clone(), ctx));
    (active_session, dispatcher)
}

pub(crate) fn mock_active_session(app: &mut App) -> ModelHandle<ActiveSession> {
    mock_session(app).0
}

/// Moves the harness's session to `cwd`, the way a shell's prompt would report it.
pub(crate) fn set_working_directory(app: &mut App, harness: &HookTestApp, cwd: &Path) {
    let cwd = cwd.display().to_string();
    harness.dispatcher.update(app, |_, ctx| {
        ctx.emit(ModelEvent::BlockMetadataReceived(
            BlockMetadataReceivedEvent {
                block_metadata: BlockMetadata::new(None, Some(cwd)),
                block_index: BlockIndex::zero(),
                is_after_in_band_command: false,
                is_done_bootstrapping: true,
            },
        ));
    });
}

/// Adds a second native conversation that a parent conversation spawned, so a test can see
/// how a hidden local child is treated.
pub(crate) fn add_child_conversation(app: &mut App, harness: &HookTestApp) -> AIConversationId {
    let mut child = AIConversation::new(false, false);
    child.set_parent_conversation_id(harness.conversation_id);
    let child_id = child.id();
    let terminal_view_id = harness.terminal_view_id;
    app.update(|ctx| {
        BlocklistAIHistoryModel::handle(ctx).update(ctx, |model, ctx| {
            model.restore_conversations(terminal_view_id, vec![child], ctx);
        });
    });
    child_id
}

pub(crate) fn shell_action(id: &str, command: &str) -> AIAgentAction {
    AIAgentAction {
        id: AIAgentActionId::from(id.to_owned()),
        task_id: TaskId::new("task".to_owned()),
        requires_result: true,
        action: AIAgentActionType::RequestCommandOutput {
            command: command.to_owned(),
            is_read_only: Some(true),
            is_risky: Some(false),
            wait_until_completion: true,
            uses_pager: None,
            rationale: None,
            citations: Vec::new(),
        },
    }
}

pub(crate) fn init_project_action(id: &str) -> AIAgentAction {
    AIAgentAction {
        id: AIAgentActionId::from(id.to_owned()),
        task_id: TaskId::new("task".to_owned()),
        requires_result: true,
        action: AIAgentActionType::InitProject,
    }
}

/// A read of no files: an asynchronous action that completes on its own.
pub(crate) fn read_nothing_action(id: &str) -> AIAgentAction {
    AIAgentAction {
        id: AIAgentActionId::from(id.to_owned()),
        task_id: TaskId::new("task".to_owned()),
        requires_result: true,
        action: AIAgentActionType::ReadFiles(ReadFilesRequest {
            locations: Vec::new(),
        }),
    }
}

/// Builds an action model over a mock terminal, wired to the harness's session.
pub(crate) fn action_model(
    app: &mut App,
    harness: &HookTestApp,
) -> ModelHandle<BlocklistAIActionModel> {
    if !app.read(|ctx| ctx.has_singleton_model::<RecordingController>()) {
        app.add_singleton_model(|_| RecordingController::new());
    }
    let terminal_model = Arc::new(FairMutex::new(TerminalModel::mock(None, None)));
    let get_relevant_files = app.add_model(|_| GetRelevantFilesController::default());
    let active_session = harness.active_session.clone();
    let dispatcher = harness.dispatcher.clone();
    let terminal_view_id = harness.terminal_view_id;
    app.add_model(|ctx| {
        BlocklistAIActionModel::new(
            terminal_model,
            active_session,
            &dispatcher,
            get_relevant_files,
            terminal_view_id,
            UserWorkspaces::teamless_context_resolver_for_test(),
            ctx,
        )
    })
}

/// Polls `done` until it returns true, yielding to background work between polls.
pub(crate) async fn settle(app: &mut App, mut done: impl FnMut(&mut App) -> bool) {
    for _ in 0..400 {
        if done(app) {
            return;
        }
        Timer::after(Duration::from_millis(5)).await;
    }
    panic!("condition was not met in time");
}

/// Yields to background work for a short, fixed time, for asserting that nothing more happens.
pub(crate) async fn let_background_work_settle(app: &mut App) {
    for _ in 0..10 {
        Timer::after(Duration::from_millis(5)).await;
        app.update(|_| {});
    }
}

/// The doubles behind a [`HookRuntime`] installed for a test, and the adapter its bindings use.
pub(crate) struct TestHooks {
    pub adapter: Arc<RecordingAdapter>,
    pub runner: Arc<ScriptedRunner>,
    pub messenger: Arc<RecordingMessenger>,
}

impl TestHooks {
    /// Replaces the active snapshot with `bindings`, all using this adapter and applying
    /// everywhere.
    pub(crate) fn install_bindings(&self, bindings: Vec<DeclaredBinding>, ctx: &mut AppContext) {
        self.install_scoped_bindings(
            bindings
                .into_iter()
                .map(|binding| (binding, BindingScope::Everywhere))
                .collect(),
            ctx,
        );
    }

    /// Replaces the active snapshot with `bindings`, each applying within its own scope. A
    /// root a binding is scoped to is also made a project the registry knows.
    pub(crate) fn install_scoped_bindings(
        &self,
        bindings: Vec<(DeclaredBinding, BindingScope)>,
        ctx: &mut AppContext,
    ) {
        let adapter: Arc<dyn ai::hooks::HookFormatAdapter> = self.adapter.clone();
        let active = bindings
            .into_iter()
            .map(|(binding, scope)| {
                ai::hooks::test_util::active_binding(binding, adapter.clone(), scope)
            })
            .collect::<Vec<ActiveBinding>>();
        HookRegistry::handle(ctx).update(ctx, |registry, _| {
            for binding in &active {
                if let BindingScope::Root(root) = &binding.scope {
                    registry.add_project_root(root);
                }
            }
            registry.install_snapshot_for_test(active, Vec::<HookDiagnostic>::new());
        });
    }
}

/// Registers the hook singletons (or rewires existing ones) with recording doubles.
pub(crate) fn install_test_hooks(ctx: &mut AppContext) -> TestHooks {
    let adapter = Arc::new(RecordingAdapter::new());
    let runner = Arc::new(ScriptedRunner::new());
    let messenger = Arc::new(RecordingMessenger::new());
    if !ctx.has_singleton_model::<HookRegistry>() {
        ctx.add_singleton_model(|_| HookRegistry::new());
    }
    let runner_dyn: Arc<dyn HookRunner> = runner.clone();
    let messenger_dyn: Arc<dyn ai::hooks::HookMessenger> = messenger.clone();
    if ctx.has_singleton_model::<HookRuntime>() {
        HookRuntime::handle(ctx).update(ctx, |runtime, _| {
            runtime.set_runner(runner_dyn.clone());
            runtime.set_messenger(messenger_dyn.clone());
        });
    } else {
        ctx.add_singleton_model(|ctx| HookRuntime::new(runner_dyn, messenger_dyn, ctx));
    }
    TestHooks {
        adapter,
        runner,
        messenger,
    }
}

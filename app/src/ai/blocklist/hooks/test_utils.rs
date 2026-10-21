use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use ai::hooks::test_util::{RecordingAdapter, RecordingMessenger, ScriptedRunner};
use ai::hooks::{
    ActiveBinding, BindingScope, DeclaredBinding, HookDiagnostic, HookRegistry, HookRunner,
};
use async_channel::unbounded;
use parking_lot::FairMutex;
use warpui::r#async::Timer;
use warpui::{App, AppContext, ModelHandle, SingletonEntity};

use super::HookRuntime;
use crate::ai::agent::conversation::{AIConversation, AIConversationId};
use crate::ai::agent::task::TaskId;
use crate::ai::agent::{AIAgentAction, AIAgentActionId, AIAgentActionType, ReadFilesRequest};
use crate::ai::blocklist::{BlocklistAIActionModel, BlocklistAIHistoryModel, RecordingController};
use crate::ai::get_relevant_files::controller::GetRelevantFilesController;
use crate::terminal::event::BlockMetadataReceivedEvent;
use crate::terminal::model::block::BlockMetadata;
use crate::terminal::model::session::Sessions;
use crate::terminal::model::session::active_session::ActiveSession;
use crate::terminal::model::terminal_model::{BlockIndex, TerminalModel};
use crate::terminal::model_events::{ModelEvent, ModelEventDispatcher};
use crate::test_util::terminal::initialize_app_for_terminal_view;
use crate::workspaces::user_workspaces::UserWorkspaces;

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
    let sessions = app.add_model(|_| Sessions::new_for_test());
    let (_tx, rx) = unbounded();
    let dispatcher = app.add_model(|ctx| ModelEventDispatcher::new(rx, sessions.clone(), ctx));
    let active_session = app.add_model(|ctx| ActiveSession::new(sessions, dispatcher.clone(), ctx));
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

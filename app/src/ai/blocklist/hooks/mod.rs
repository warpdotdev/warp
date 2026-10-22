//! Interception points for agent hooks. Each submodule owns one boundary (or a group of related
//! boundaries) and exposes the function the agent loop calls at that moment. Every entry point
//! starts with [`HookRuntime::is_active`], or [`HookRuntime::is_installed`] when it only clears
//! state, and returns early when hooks are off or nothing is configured, so the inactive path
//! adds no work beyond that check.

pub(crate) mod envelope;
pub(crate) mod pre_tool;
pub(crate) mod prompt;
pub(crate) mod session;
pub(crate) mod stream;
pub(crate) mod tool_result;
pub(crate) mod turn;

#[cfg(all(not(target_family = "wasm"), feature = "local_tty"))]
use std::ffi::OsString;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

#[cfg(all(not(target_family = "wasm"), feature = "local_tty"))]
use ai::hooks::runner::command::CommandHookRunner;
use ai::hooks::{
    HookDispatcher, HookEnvelope, HookMessenger, HookRegistry, HookRunner, RegistrySnapshot,
    RunMetadata,
};
#[cfg(all(not(target_family = "wasm"), feature = "local_tty"))]
use parking_lot::Mutex;
use warpui::{AppContext, Entity, ModelContext, ModelHandle, SingletonEntity};

use self::pre_tool::PreToolVerdictCache;
use self::stream::StreamObserverState;
use crate::ai::agent::conversation::{AIConversationId, ConversationDriver};
use crate::ai::blocklist::BlocklistAIHistoryModel;
use crate::features::FeatureFlag;
#[cfg(all(not(target_family = "wasm"), feature = "local_tty"))]
use crate::terminal::local_shell::LocalShellState;

/// Process-wide hook state shared by every conversation and surface.
pub struct HookRuntime {
    registry: ModelHandle<HookRegistry>,
    dispatcher: Arc<HookDispatcher>,
    messenger: Arc<dyn HookMessenger>,
    run_metadata: Option<RunMetadata>,
    pre_tool_verdicts: PreToolVerdictCache,
    stream_observer: StreamObserverState,
    #[cfg(test)]
    observed_terminal_results: Vec<tool_result::ObservedTerminalResult>,
}

impl HookRuntime {
    pub fn new(
        runner: Arc<dyn HookRunner>,
        messenger: Arc<dyn HookMessenger>,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        Self {
            registry: HookRegistry::handle(ctx),
            dispatcher: Arc::new(HookDispatcher::new(runner, messenger.clone())),
            messenger,
            run_metadata: None,
            pre_tool_verdicts: PreToolVerdictCache::default(),
            stream_observer: StreamObserverState::default(),
            #[cfg(test)]
            observed_terminal_results: Vec::new(),
        }
    }

    /// Whether the flag is on and the runtime is registered. True even when no binding is active.
    pub fn is_installed(ctx: &AppContext) -> bool {
        FeatureFlag::AgentHooks.is_enabled() && ctx.has_singleton_model::<HookRuntime>()
    }

    /// Whether any hook can fire right now: the runtime is installed and at least one binding is
    /// active.
    pub fn is_active(ctx: &AppContext) -> bool {
        Self::is_installed(ctx) && !Self::as_ref(ctx).registry.as_ref(ctx).snapshot().is_empty()
    }

    pub fn snapshot(&self, ctx: &AppContext) -> Arc<RegistrySnapshot> {
        self.registry.as_ref(ctx).snapshot()
    }

    pub fn dispatcher(&self) -> Arc<HookDispatcher> {
        self.dispatcher.clone()
    }

    pub fn messenger(&self) -> Arc<dyn HookMessenger> {
        self.messenger.clone()
    }

    pub fn run_metadata(&self) -> Option<&RunMetadata> {
        self.run_metadata.as_ref()
    }

    /// Replaces the runner. Dispatches already in flight keep the runner they started with.
    pub fn set_runner(&mut self, runner: Arc<dyn HookRunner>) {
        self.dispatcher = Arc::new(HookDispatcher::new(runner, self.messenger.clone()));
    }

    /// Installs the runner that spawns hook processes. Hooks get the login shell's PATH once it
    /// has been captured; until then they inherit this process's PATH.
    #[cfg(all(not(target_family = "wasm"), feature = "local_tty"))]
    pub fn install_command_runner(ctx: &mut AppContext) {
        let login_shell_path: Arc<Mutex<Option<OsString>>> = Arc::default();
        let captured = login_shell_path.clone();
        let runner = CommandHookRunner::new(Arc::new(move || captured.lock().clone()));
        Self::handle(ctx).update(ctx, |runtime, ctx| {
            runtime.set_runner(Arc::new(runner));
            Self::capture_login_shell_path(login_shell_path, ctx);
        });
    }

    /// Starts the interactive shell capture, which is only worth its cost when hooks can run.
    #[cfg(all(not(target_family = "wasm"), feature = "local_tty"))]
    fn capture_login_shell_path(
        login_shell_path: Arc<Mutex<Option<OsString>>>,
        ctx: &mut ModelContext<Self>,
    ) {
        if !FeatureFlag::AgentHooks.is_enabled() || !ctx.has_singleton_model::<LocalShellState>() {
            return;
        }
        let capture = LocalShellState::handle(ctx).update(ctx, |shell_state, ctx| {
            shell_state.get_interactive_path_env_var(ctx)
        });
        ctx.spawn(capture, move |_, path, _| {
            *login_shell_path.lock() = path.map(OsString::from);
        });
    }

    // No production caller yet: later work installs a surface-specific messenger and run metadata
    // through these setters.

    /// Records the run this process belongs to. Envelopes built afterwards carry it.
    #[allow(dead_code)]
    pub fn set_run_metadata(&mut self, run_metadata: Option<RunMetadata>) {
        self.run_metadata = run_metadata;
    }

    /// Replaces the messenger for notices and diagnostics, including those the dispatcher routes.
    #[allow(dead_code)]
    pub fn set_messenger(&mut self, messenger: Arc<dyn HookMessenger>) {
        self.dispatcher = Arc::new(HookDispatcher::new(
            self.dispatcher.runner().clone(),
            messenger.clone(),
        ));
        self.messenger = messenger;
    }

    /// Starts a dispatch on the background executor and detaches its future.
    pub(crate) fn dispatch_detached(&self, envelope: HookEnvelope, ctx: &AppContext) {
        let future = self.dispatcher.dispatch(
            envelope,
            self.snapshot(ctx),
            Arc::new(AtomicBool::new(false)),
        );
        ctx.background_executor()
            .spawn(async move {
                future.await;
            })
            .detach();
    }
}

impl Entity for HookRuntime {
    type Event = ();
}

impl SingletonEntity for HookRuntime {}

// Driver checks. Unknown conversations fail every check.

/// Whether this client drives `conversation_id`'s turns.
pub(super) fn owns_turn_lifecycle(conversation_id: AIConversationId, ctx: &AppContext) -> bool {
    driver_check(conversation_id, ctx, |driver| driver.owns_turn_lifecycle())
}

/// Whether this client runs `conversation_id`'s tool calls.
pub(super) fn executes_tool_calls_locally(
    conversation_id: AIConversationId,
    ctx: &AppContext,
) -> bool {
    driver_check(conversation_id, ctx, |driver| {
        driver.executes_tool_calls_locally()
    })
}

fn driver_check(
    conversation_id: AIConversationId,
    ctx: &AppContext,
    check: impl FnOnce(ConversationDriver) -> bool,
) -> bool {
    ctx.has_singleton_model::<BlocklistAIHistoryModel>()
        && BlocklistAIHistoryModel::as_ref(ctx)
            .conversation(&conversation_id)
            .is_some_and(|conversation| check(conversation.driver()))
}

#[cfg(test)]
pub(crate) mod testing {
    use std::sync::Arc;
    use std::time::Duration;

    use ai::hooks::test_util::{RecordingAdapter, RecordingMessenger, ScriptedRunner};
    use ai::hooks::{ActiveBinding, DeclaredBinding, HookDiagnostic, HookRegistry, HookRunner};
    use async_channel::unbounded;
    use parking_lot::FairMutex;
    use warpui::r#async::Timer;
    use warpui::{App, AppContext, ModelHandle, SingletonEntity};

    use super::HookRuntime;
    use crate::ai::agent::conversation::{AIConversation, AIConversationId};
    use crate::ai::agent::task::TaskId;
    use crate::ai::agent::{AIAgentAction, AIAgentActionId, AIAgentActionType, ReadFilesRequest};
    use crate::ai::blocklist::{
        BlocklistAIActionModel, BlocklistAIHistoryModel, RecordingController,
    };
    use crate::ai::get_relevant_files::controller::GetRelevantFilesController;
    use crate::terminal::model::session::Sessions;
    use crate::terminal::model::session::active_session::ActiveSession;
    use crate::terminal::model::terminal_model::TerminalModel;
    use crate::terminal::model_events::ModelEventDispatcher;
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
        let active_session =
            app.add_model(|ctx| ActiveSession::new(sessions, dispatcher.clone(), ctx));
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
        /// Replaces the active snapshot with `bindings`, all using this adapter.
        pub(crate) fn install_bindings(
            &self,
            bindings: Vec<DeclaredBinding>,
            ctx: &mut AppContext,
        ) {
            let adapter: Arc<dyn ai::hooks::HookFormatAdapter> = self.adapter.clone();
            let active = bindings
                .into_iter()
                .map(|binding| {
                    ai::hooks::test_util::active_binding(
                        binding,
                        adapter.clone(),
                        ai::hooks::BindingScope::Everywhere,
                    )
                })
                .collect::<Vec<ActiveBinding>>();
            HookRegistry::handle(ctx).update(ctx, |registry, _| {
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
}

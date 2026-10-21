//! Interception points for agent hooks. Each submodule owns one boundary (or a group of related
//! boundaries) and exposes the function the agent loop calls at that moment. Every entry point
//! starts with [`HookRuntime::is_active`] and returns early when hooks are off or nothing is
//! configured, so the inactive path adds no work beyond that check.

pub(crate) mod envelope;
pub(crate) mod pre_tool;
pub(crate) mod prompt;
pub(crate) mod session;
pub(crate) mod stream;
pub(crate) mod tool_result;
pub(crate) mod turn;

use std::sync::Arc;

use ai::hooks::{
    HookDispatcher, HookEnvelope, HookMessenger, HookRegistry, HookRunner, RegistrySnapshot,
    RunMetadata,
};
use warpui::{AppContext, Entity, ModelContext, ModelHandle, SingletonEntity};

use self::pre_tool::PreToolVerdictCache;
use self::stream::StreamObserverState;
use crate::features::FeatureFlag;

/// Process-wide hook state shared by every conversation and surface.
pub struct HookRuntime {
    registry: ModelHandle<HookRegistry>,
    dispatcher: Arc<HookDispatcher>,
    messenger: Arc<dyn HookMessenger>,
    run_metadata: Option<RunMetadata>,
    pre_tool: PreToolVerdictCache,
    stream: StreamObserverState,
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
            pre_tool: PreToolVerdictCache::default(),
            stream: StreamObserverState::default(),
            #[cfg(test)]
            observed_terminal_results: Vec::new(),
        }
    }

    /// Whether any hook can fire right now: the feature flag is on, the runtime is registered,
    /// and at least one binding is active.
    pub fn is_active(ctx: &AppContext) -> bool {
        FeatureFlag::AgentHooks.is_enabled()
            && ctx.has_singleton_model::<HookRuntime>()
            && !Self::as_ref(ctx).registry.as_ref(ctx).snapshot().is_empty()
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

    /// Records the run this process belongs to. Envelopes built afterwards carry it.
    // Nothing supplies run metadata yet; the setter exists so the run bootstrap can install it.
    #[allow(dead_code)]
    pub fn set_run_metadata(&mut self, run_metadata: Option<RunMetadata>) {
        self.run_metadata = run_metadata;
    }

    /// Replaces the runner. Dispatches already in flight keep the runner they started with.
    // The only runner so far is the placeholder passed at startup; the command runner replaces
    // it through this setter.
    #[allow(dead_code)]
    pub fn set_runner(&mut self, runner: Arc<dyn HookRunner>) {
        self.dispatcher = Arc::new(HookDispatcher::new(runner, self.messenger.clone()));
    }

    /// Replaces the messenger for notices and diagnostics, including those the dispatcher routes.
    // The only messenger so far is the log-only one passed at startup; a surface-specific
    // messenger replaces it through this setter.
    #[allow(dead_code)]
    pub fn set_messenger(&mut self, messenger: Arc<dyn HookMessenger>) {
        self.dispatcher = Arc::new(HookDispatcher::new(
            self.dispatcher.runner().clone(),
            messenger.clone(),
        ));
        self.messenger = messenger;
    }

    /// Starts a dispatch whose outcome nobody waits for. Used by boundaries that only observe.
    pub(crate) fn dispatch_detached(&self, envelope: HookEnvelope, ctx: &AppContext) {
        let future = self.dispatcher.dispatch(
            envelope,
            self.snapshot(ctx),
            Arc::new(std::sync::atomic::AtomicBool::new(false)),
        );
        ctx.background_executor()
            .spawn(async move {
                drop(future.await);
            })
            .detach();
    }
}

impl Entity for HookRuntime {
    type Event = ();
}

impl SingletonEntity for HookRuntime {}

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

    /// The doubles behind a [`HookRuntime`] installed for a test, and the adapter its bindings
    /// use.
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

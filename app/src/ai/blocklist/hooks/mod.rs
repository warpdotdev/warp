//! Interception points for agent hooks. Each submodule owns one boundary (or a group of related
//! boundaries) and exposes the function the agent loop calls at that moment. Every entry point
//! that can run a hook starts with [`HookRuntime::is_active`] and returns early when no binding is
//! active. Entry points that only clear state check [`HookRuntime::is_installed`] and then find
//! nothing to clear, since only an active path records anything.

#[cfg(all(test, feature = "local_fs"))]
mod e2e_tests;
pub(crate) mod envelope;
pub(crate) mod pre_tool;
pub(crate) mod prompt;
pub(crate) mod session;
pub(crate) mod stream;
#[cfg(test)]
pub(crate) mod test_utils;
pub(crate) mod tool_result;
pub(crate) mod turn;

use std::collections::HashMap;
#[cfg(feature = "local_tty")]
use std::ffi::OsString;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

#[cfg(feature = "local_tty")]
use ai::hooks::runner::command::CommandHookRunner;
use ai::hooks::{
    HookDispatcher, HookEnvelope, HookMessenger, HookRegistry, HookRunner, HookSurface, HookTurnId,
    RegistrySnapshot, RunMetadata,
};
#[cfg(feature = "local_tty")]
use futures::FutureExt as _;
#[cfg(feature = "local_tty")]
use futures::future::{BoxFuture, Shared};
#[cfg(feature = "local_tty")]
use parking_lot::Mutex;
use warp_core::execution_mode::AppExecutionMode;
#[cfg(feature = "local_tty")]
use warpui::r#async::TimeoutError;
use warpui::{AppContext, Entity, ModelContext, ModelHandle, SingletonEntity};

use self::pre_tool::PreToolVerdictCache;
use self::prompt::PendingPrompts;
use self::session::SessionStarts;
use self::stream::StreamObserverState;
use self::tool_result::ShellCompletions;
use self::turn::TurnStops;
use crate::ai::agent::AIAgentActionId;
use crate::ai::agent::conversation::{AIConversationId, ConversationDriver};
use crate::ai::blocklist::BlocklistAIHistoryModel;
use crate::ai::blocklist::history_model::BlocklistAIHistoryEvent;
#[cfg(feature = "local_tty")]
use crate::process_environment::InitialProcessEnvironment;
#[cfg(feature = "local_tty")]
use crate::terminal::local_shell::LocalShellState;

/// Which kind of headless run this process is.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum RunSurface {
    /// A run the server dispatched: an Oz cloud run, or a task resumed with `--task-id`.
    #[cfg_attr(
        target_family = "wasm",
        expect(dead_code, reason = "Oz runs are unavailable in WASM builds")
    )]
    Oz,
    /// A `warp agent run` started from a command line.
    #[default]
    CommandLine,
}

impl RunSurface {
    pub fn hook_surface(self) -> HookSurface {
        match self {
            Self::Oz => HookSurface::OzRun,
            Self::CommandLine => HookSurface::CliRun,
        }
    }
}

/// The login shell's PATH for the command runner, and the capture that reads it.
#[cfg(feature = "local_tty")]
struct LoginShellPath {
    /// What the command runner reads for each invocation. It is set once, when the capture is
    /// settled, so every hook in a run sees the same PATH.
    slot: Arc<Mutex<Option<OsString>>>,
    capture: LoginShellPathCapture,
}

#[cfg(feature = "local_tty")]
enum LoginShellPathCapture {
    NotStarted,
    Running(Shared<BoxFuture<'static, Option<String>>>),
    Settled,
}

/// The run this process belongs to and the surface its envelopes report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RunIdentity {
    pub metadata: RunMetadata,
    pub surface: RunSurface,
}

/// Something the hook runtime did that the owner of the run may want to act on.
pub enum HookRuntimeEvent {
    /// A hook asked for confirmation on a surface with no one to ask, and a hook that asked
    /// comes from a source set to end the run when that happens. The action was already denied
    /// with a reason the model can read; the run's owner ends the run.
    #[cfg_attr(
        target_family = "wasm",
        expect(
            dead_code,
            reason = "the Oz driver that consumes this event is unavailable in WASM builds"
        )
    )]
    ApprovalUnavailable {
        conversation_id: AIConversationId,
        action_id: AIAgentActionId,
        reason: String,
    },
}

/// Process-wide hook state shared by every conversation and surface.
pub struct HookRuntime {
    registry: ModelHandle<HookRegistry>,
    dispatcher: Arc<HookDispatcher>,
    messenger: Arc<dyn HookMessenger>,
    run: Option<RunIdentity>,
    pre_tool_verdicts: PreToolVerdictCache,
    stream_observer: StreamObserverState,
    /// The turn each conversation is in, allocated when a prompt starts a turn.
    turn_ids: HashMap<AIConversationId, HookTurnId>,
    session_starts: SessionStarts,
    turn_stops: TurnStops,
    pending_prompts: PendingPrompts,
    shell_completions: ShellCompletions,
    /// Present once the command runner is installed.
    #[cfg(feature = "local_tty")]
    login_shell_path: Option<LoginShellPath>,
    #[cfg(test)]
    observed_terminal_results: Vec<tool_result::ObservedTerminalResult>,
}

impl HookRuntime {
    pub fn new(
        runner: Arc<dyn HookRunner>,
        messenger: Arc<dyn HookMessenger>,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        Self::subscribe_to_conversation_cleanup(ctx);
        Self {
            registry: HookRegistry::handle(ctx),
            dispatcher: Arc::new(HookDispatcher::new(runner, messenger.clone())),
            messenger,
            run: None,
            pre_tool_verdicts: PreToolVerdictCache::default(),
            stream_observer: StreamObserverState::default(),
            turn_ids: HashMap::new(),
            session_starts: SessionStarts::default(),
            turn_stops: TurnStops::default(),
            pending_prompts: PendingPrompts::default(),
            shell_completions: ShellCompletions::default(),
            #[cfg(feature = "local_tty")]
            login_shell_path: None,
            #[cfg(test)]
            observed_terminal_results: Vec::new(),
        }
    }

    fn subscribe_to_conversation_cleanup(ctx: &mut ModelContext<Self>) {
        if !ctx.has_singleton_model::<BlocklistAIHistoryModel>() {
            return;
        }
        ctx.subscribe_to_model(
            &BlocklistAIHistoryModel::handle(ctx),
            |me, _, event, _| match event {
                BlocklistAIHistoryEvent::RemoveConversation {
                    conversation_id, ..
                }
                | BlocklistAIHistoryEvent::DeletedConversation {
                    conversation_id, ..
                } => me.forget_conversation(*conversation_id),
                BlocklistAIHistoryEvent::ClearedConversationsForTerminalSurface {
                    cleared_conversation_ids,
                    ..
                } => {
                    for conversation_id in cleared_conversation_ids {
                        me.forget_cleared_conversation(*conversation_id);
                    }
                }
                BlocklistAIHistoryEvent::AppendedExchange { .. }
                | BlocklistAIHistoryEvent::StartedNewConversation { .. }
                | BlocklistAIHistoryEvent::CreatedSubtask { .. }
                | BlocklistAIHistoryEvent::UpgradedTask { .. }
                | BlocklistAIHistoryEvent::ReassignedExchange { .. }
                | BlocklistAIHistoryEvent::UpdatedStreamingExchange { .. }
                | BlocklistAIHistoryEvent::UpdatedConversationStatus { .. }
                | BlocklistAIHistoryEvent::SetActiveConversation { .. }
                | BlocklistAIHistoryEvent::ClearedActiveConversation { .. }
                | BlocklistAIHistoryEvent::UpdatedTodoList { .. }
                | BlocklistAIHistoryEvent::UpdatedAutoexecuteOverride { .. }
                | BlocklistAIHistoryEvent::SplitConversation { .. }
                | BlocklistAIHistoryEvent::RestoredConversations { .. }
                | BlocklistAIHistoryEvent::UpdatedConversationMetadata { .. }
                | BlocklistAIHistoryEvent::UpdatedConversationTitle { .. }
                | BlocklistAIHistoryEvent::UpdatedConversationArtifacts { .. }
                | BlocklistAIHistoryEvent::ConversationServerTokenAssigned { .. }
                | BlocklistAIHistoryEvent::ConversationTransferredBetweenTerminalSurfaces {
                    ..
                }
                | BlocklistAIHistoryEvent::NewConversationRequestComplete { .. }
                | BlocklistAIHistoryEvent::OrchestrationConfigUpdated { .. }
                | BlocklistAIHistoryEvent::ConversationUsageMetadataUpdated { .. }
                | BlocklistAIHistoryEvent::LocalSharedSessionEstablished { .. } => {}
            },
        );
    }

    /// Drops everything remembered about a conversation that no longer exists. A dispatch still
    /// running for it is told to stop.
    fn forget_conversation(&mut self, conversation_id: AIConversationId) {
        self.forget_cleared_conversation(conversation_id);
        self.turn_stops.forget(conversation_id);
    }

    /// Drops what is remembered about a conversation its surface cleared. The conversation still
    /// exists, so a turn whose success is held keeps its hold: the surface's cancellation settles
    /// it when the conversation was the active one, and otherwise the hooks' answer does.
    fn forget_cleared_conversation(&mut self, conversation_id: AIConversationId) {
        self.turn_ids.remove(&conversation_id);
        self.session_starts.forget(conversation_id);
        self.pending_prompts.forget(conversation_id);
        self.shell_completions.forget(conversation_id);
    }

    /// The turn `conversation_id` is in, once a prompt has started one.
    pub fn turn_id_for(&self, conversation_id: AIConversationId) -> Option<HookTurnId> {
        self.turn_ids.get(&conversation_id).copied()
    }

    /// Starts a new turn for `conversation_id` and returns its id.
    fn start_turn(&mut self, conversation_id: AIConversationId) -> HookTurnId {
        let turn_id = HookTurnId::new();
        self.turn_ids.insert(conversation_id, turn_id);
        turn_id
    }

    /// The variables `conversation_id`'s `session.start` hooks asked to set, once they have run.
    pub fn session_env_for(&self, conversation_id: AIConversationId) -> Vec<(String, String)> {
        self.session_starts.env_for(conversation_id)
    }

    /// The surface of the run this process belongs to, once one was recorded.
    pub(crate) fn run_surface(&self) -> Option<RunSurface> {
        self.run.as_ref().map(|run| run.surface)
    }

    /// Whether this process has no interactive surface where a user can respond.
    pub fn is_headless(&self, ctx: &AppContext) -> bool {
        AppExecutionMode::as_ref(ctx).is_headless()
    }

    /// Whether the runtime is registered. True even when no binding is active.
    pub fn is_installed(ctx: &AppContext) -> bool {
        ctx.has_singleton_model::<HookRuntime>()
    }

    /// Whether any hook can fire right now: the runtime is installed and at least one binding is
    /// active.
    pub fn is_active(ctx: &AppContext) -> bool {
        Self::is_installed(ctx)
            && Self::as_ref(ctx).registry.as_ref(ctx).is_finalized()
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
        self.run.as_ref().map(|run| &run.metadata)
    }

    /// Replaces the runner. Dispatches already in flight keep the runner they started with.
    #[cfg(any(feature = "local_tty", test))]
    pub fn set_runner(&mut self, runner: Arc<dyn HookRunner>) {
        self.dispatcher = Arc::new(HookDispatcher::new(runner, self.messenger.clone()));
    }

    /// Installs the runner that spawns hook processes. Hooks get the login shell's PATH once its
    /// capture is settled with a result, and this process's PATH otherwise. Installing starts
    /// nothing, so a launch that never loads hooks pays nothing.
    #[cfg(feature = "local_tty")]
    pub fn install_command_runner(ctx: &mut AppContext) {
        let slot: Arc<Mutex<Option<OsString>>> = Arc::default();
        let captured = slot.clone();
        let initial_environment = InitialProcessEnvironment::as_ref(ctx).variables().to_vec();
        let runner = CommandHookRunner::new(
            Arc::new(move || captured.lock().clone()),
            initial_environment,
        );
        Self::handle(ctx).update(ctx, |runtime, _| {
            runtime.set_runner(Arc::new(runner));
            runtime.login_shell_path = Some(LoginShellPath {
                slot,
                capture: LoginShellPathCapture::NotStarted,
            });
        });
    }

    /// Starts reading the login shell's PATH for the command runner, at most once per process.
    /// The read starts an interactive shell, so it is only worth its cost when hooks will run. It
    /// is a fresh read rather than the value other features cached earlier in the process, so it
    /// sees what the environment's setup commands added to the shell's startup files.
    #[cfg(feature = "local_tty")]
    pub(crate) fn start_login_shell_path_capture(&mut self, ctx: &mut ModelContext<Self>) {
        let Some(login_shell_path) = self.login_shell_path.as_mut() else {
            return;
        };
        let LoginShellPathCapture::NotStarted = login_shell_path.capture else {
            return;
        };
        if !ctx.has_singleton_model::<LocalShellState>() {
            login_shell_path.capture =
                LoginShellPathCapture::Running(futures::future::ready(None).boxed().shared());
            return;
        }
        let read = LocalShellState::as_ref(ctx).read_interactive_path_env_var_now();
        let (sender, receiver) = futures::channel::oneshot::channel();
        ctx.spawn(read, move |_, path, _| {
            let _ = sender.send(path);
        });
        login_shell_path.capture = LoginShellPathCapture::Running(
            async move { receiver.await.ok().flatten() }
                .boxed()
                .shared(),
        );
    }

    /// The login-shell PATH capture to wait for before the run's first hook, started if it was
    /// not yet. `None` when there is no command runner or the capture is already settled.
    #[cfg(feature = "local_tty")]
    pub(crate) fn login_shell_path_capture(
        &mut self,
        ctx: &mut ModelContext<Self>,
    ) -> Option<Shared<BoxFuture<'static, Option<String>>>> {
        self.start_login_shell_path_capture(ctx);
        match &self.login_shell_path.as_ref()?.capture {
            LoginShellPathCapture::Running(capture) => Some(capture.clone()),
            LoginShellPathCapture::NotStarted | LoginShellPathCapture::Settled => None,
        }
    }

    /// Fixes the PATH every hook in the run gets from the capture's `outcome`. A capture that
    /// failed or did not finish within `waited` leaves this process's PATH in place, and a result
    /// that arrives later is ignored. Returns what to report when the login shell's PATH is not
    /// used.
    #[cfg(feature = "local_tty")]
    pub(crate) fn settle_login_shell_path(
        &mut self,
        outcome: Result<Option<String>, TimeoutError>,
        waited: Duration,
    ) -> Option<String> {
        let login_shell_path = self.login_shell_path.as_mut()?;
        login_shell_path.capture = LoginShellPathCapture::Settled;
        match outcome {
            Ok(Some(path)) => {
                *login_shell_path.slot.lock() = Some(OsString::from(path));
                None
            }
            Ok(None) => Some(
                "hooks run with the worker's PATH because the login shell's PATH could not be read"
                    .to_owned(),
            ),
            Err(TimeoutError) => Some(format!(
                "hooks run with the worker's PATH because the login shell's PATH could not be \
                 read within {} seconds",
                waited.as_secs()
            )),
        }
    }

    /// Whether the command runner is installed and its login-shell PATH capture has not started.
    #[cfg(all(test, feature = "local_tty"))]
    pub(crate) fn login_shell_path_capture_is_pending(&self) -> bool {
        self.login_shell_path
            .as_ref()
            .is_some_and(|login_shell_path| {
                matches!(login_shell_path.capture, LoginShellPathCapture::NotStarted)
            })
    }

    /// Replaces the capture with `capture`, for tests that control when and how it finishes.
    #[cfg(all(test, unix, feature = "local_tty"))]
    pub(crate) fn set_login_shell_path_capture_for_test(
        &mut self,
        capture: BoxFuture<'static, Option<String>>,
    ) {
        if let Some(login_shell_path) = self.login_shell_path.as_mut() {
            login_shell_path.capture = LoginShellPathCapture::Running(capture.shared());
        }
    }

    /// The PATH the command runner gives hooks, when it is not this process's own.
    #[cfg(all(test, unix, feature = "local_tty"))]
    pub(crate) fn login_shell_path_for_test(&self) -> Option<OsString> {
        self.login_shell_path
            .as_ref()
            .and_then(|login_shell_path| login_shell_path.slot.lock().clone())
    }

    /// Records the run this process belongs to. Envelopes built afterwards carry it and report
    /// its surface.
    #[cfg_attr(
        target_family = "wasm",
        expect(dead_code, reason = "Oz runs are unavailable in WASM builds")
    )]
    pub(crate) fn set_run(&mut self, run: Option<RunIdentity>) {
        self.run = run;
    }

    /// Replaces the messenger for notices and diagnostics, including those the dispatcher routes.
    #[cfg_attr(
        target_family = "wasm",
        expect(
            dead_code,
            reason = "the Oz output messenger is unavailable in WASM builds"
        )
    )]
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
            self.is_headless(ctx),
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
    type Event = HookRuntimeEvent;
}

impl SingletonEntity for HookRuntime {}

/// The longest timeout among the bindings in `snapshot` that would run for `envelope`, which is
/// how long a boundary that waits for its hooks waits. `None` when no binding would run.
fn longest_matching_timeout(
    snapshot: &RegistrySnapshot,
    envelope: &HookEnvelope,
) -> Option<Duration> {
    snapshot
        .bindings_for(envelope.boundary(), envelope.execution_host)
        .into_iter()
        .filter(|active| active.adapter.matches(&active.binding, envelope))
        .map(|active| active.binding.timeout)
        .max()
}

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

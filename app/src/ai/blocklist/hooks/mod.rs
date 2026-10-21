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
    HookDispatcher, HookEnvelope, HookMessenger, HookRegistry, HookRunner, HookTurnId,
    RegistrySnapshot, RunMetadata,
};
#[cfg(feature = "local_tty")]
use parking_lot::Mutex;
use warp_core::execution_mode::AppExecutionMode;
use warpui::{AppContext, Entity, ModelContext, ModelHandle, SingletonEntity};

use self::pre_tool::PreToolVerdictCache;
use self::prompt::PendingPrompts;
use self::session::SessionStarts;
use self::stream::StreamObserverState;
use self::tool_result::ShellCompletions;
use self::turn::TurnStops;
use crate::ai::agent::AIAgentActionId;
use crate::ai::agent::conversation::{AIConversationId, ConversationDriver};
use crate::ai::blocklist::history_model::BlocklistAIHistoryEvent;
use crate::ai::blocklist::BlocklistAIHistoryModel;
use crate::features::FeatureFlag;
#[cfg(feature = "local_tty")]
use crate::process_environment::InitialProcessEnvironment;
#[cfg(feature = "local_tty")]
use crate::terminal::local_shell::LocalShellState;

/// Something the hook runtime did that the owner of the run may want to act on.
// No production subscriber yet: the run driver that ends a run on an unanswerable `ask` is
// later work.
#[allow(dead_code)]
pub enum HookRuntimeEvent {
    /// A hook asked for confirmation on a surface with no one to ask, and a hook that asked
    /// comes from a source set to end the run when that happens. The action was already denied
    /// with a reason the model can read; the run's owner ends the run.
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
    run_metadata: Option<RunMetadata>,
    pre_tool_verdicts: PreToolVerdictCache,
    stream_observer: StreamObserverState,
    /// The turn each conversation is in, allocated when a prompt starts a turn.
    turn_ids: HashMap<AIConversationId, HookTurnId>,
    session_starts: SessionStarts,
    turn_stops: TurnStops,
    pending_prompts: PendingPrompts,
    shell_completions: ShellCompletions,
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
            run_metadata: None,
            pre_tool_verdicts: PreToolVerdictCache::default(),
            stream_observer: StreamObserverState::default(),
            turn_ids: HashMap::new(),
            session_starts: SessionStarts::default(),
            turn_stops: TurnStops::default(),
            pending_prompts: PendingPrompts::default(),
            shell_completions: ShellCompletions::default(),
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
                        me.forget_conversation(*conversation_id);
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
        self.turn_ids.remove(&conversation_id);
        self.session_starts.forget(conversation_id);
        self.turn_stops.forget(conversation_id);
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

    /// Whether this process has no interactive surface where a user can respond.
    pub fn is_headless(&self, ctx: &AppContext) -> bool {
        AppExecutionMode::as_ref(ctx).is_headless()
    }

    /// Whether the flag is on and the runtime is registered. True even when no binding is active.
    pub fn is_installed(ctx: &AppContext) -> bool {
        FeatureFlag::AgentHooks.is_enabled() && ctx.has_singleton_model::<HookRuntime>()
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
        self.run_metadata.as_ref()
    }

    /// Replaces the runner. Dispatches already in flight keep the runner they started with.
    #[cfg(any(feature = "local_tty", test))]
    pub fn set_runner(&mut self, runner: Arc<dyn HookRunner>) {
        self.dispatcher = Arc::new(HookDispatcher::new(runner, self.messenger.clone()));
    }

    /// Installs the runner that spawns hook processes. Hooks get the login shell's PATH once it
    /// has been captured; until then they inherit this process's PATH.
    #[cfg(feature = "local_tty")]
    pub fn install_command_runner(ctx: &mut AppContext) {
        let login_shell_path: Arc<Mutex<Option<OsString>>> = Arc::default();
        let captured = login_shell_path.clone();
        let initial_environment = InitialProcessEnvironment::as_ref(ctx).variables().to_vec();
        let runner = CommandHookRunner::new(
            Arc::new(move || captured.lock().clone()),
            initial_environment,
        );
        Self::handle(ctx).update(ctx, |runtime, ctx| {
            runtime.set_runner(Arc::new(runner));
            Self::capture_login_shell_path(login_shell_path, ctx);
        });
    }

    /// Starts the interactive shell capture, which is only worth its cost when hooks can run.
    #[cfg(feature = "local_tty")]
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

    // No production caller yet: later work installs a surface-specific messenger, run metadata,
    // and the run's hook policies through these setters.

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

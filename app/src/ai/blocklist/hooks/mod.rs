//! Interception points and process-wide runtime for agent hooks. Each submodule owns one
//! boundary (or a group of related boundaries) and exposes the entry point the agent loop calls
//! at that moment. Every entry point returns early when hooks are off or nothing is configured,
//! so the inactive path adds no work beyond that check.

pub(crate) mod envelope;
pub(crate) mod pre_tool;
pub(crate) mod prompt;
pub(crate) mod session;
pub(crate) mod stream;
pub(crate) mod strict_policy;
#[cfg(test)]
pub(crate) mod test_utils;
pub(crate) mod tool_result;
pub(crate) mod turn;

use std::collections::HashMap;
#[cfg(all(not(target_family = "wasm"), feature = "local_tty"))]
use std::ffi::OsString;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

#[cfg(all(not(target_family = "wasm"), feature = "local_tty"))]
use ai::hooks::runner::command::CommandHookRunner;
use ai::hooks::{
    CommandExecutionPosture, DispatchPolicy, ExplicitHookSource, HookDiagnostic, HookDispatcher,
    HookEnvelope, HookMessenger, HookRegistry, HookRegistryEvent, HookRunner, HookSourceKind,
    HookSurface, HookTurnId, RegistrySnapshot, RunMetadata,
};
#[cfg(feature = "local_fs")]
use ai::hooks::{HookConfigWatcher, StaticTrustPolicy, watch_matrix};
#[cfg(all(not(target_family = "wasm"), feature = "local_tty"))]
use parking_lot::Mutex;
#[cfg(feature = "local_fs")]
use repo_metadata::Repository;
#[cfg(feature = "local_fs")]
use repo_metadata::repositories::{
    DetectedRepositories, DetectedRepositoriesEvent, RepoDetectionSource,
};
use warp_core::execution_mode::AppExecutionMode;
use warpui::{AppContext, Entity, EntityId, ModelContext, ModelHandle, SingletonEntity};

use self::pre_tool::PreToolVerdictCache;
use self::prompt::PendingPrompts;
use self::session::SessionStarts;
use self::stream::StreamObserverState;
use self::strict_policy::StrictHookPolicy;
use self::tool_result::ShellCompletions;
use self::turn::TurnStops;
use crate::ai::agent::AIAgentActionId;
use crate::ai::agent::conversation::{AIConversationId, ConversationDriver};
use crate::ai::blocklist::history_model::BlocklistAIHistoryEvent;
use crate::ai::blocklist::{BlocklistAIHistoryModel, BlocklistAIPermissions};
use crate::ai::execution_profiles::ActionPermission;
use crate::ai::execution_profiles::profiles::AIExecutionProfilesModel;
use crate::features::FeatureFlag;
#[cfg(all(not(target_family = "wasm"), feature = "local_tty"))]
use crate::terminal::local_shell::LocalShellState;
use crate::workspaces::user_workspaces::{TeamScope, UserWorkspaces};

/// What a hook's `ask` does on a surface with no one to ask. Like [`StrictHookPolicy`], it is
/// run configuration a launcher sets on the runtime; there is no setting for it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum HeadlessAskBehavior {
    /// The action is denied with a reason that tells the model approval was unavailable.
    #[default]
    DenyWithReason,
    /// The action is denied the same way, and the runtime also announces the refusal so the
    /// run's owner can end the run.
    FailRun,
}

/// Which kind of headless run this process is.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum RunSurface {
    /// A run the server dispatched: an Oz cloud run, or a task resumed with `--task-id`.
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

/// The run this process belongs to and the surface its envelopes report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RunIdentity {
    pub metadata: RunMetadata,
    pub surface: RunSurface,
}

/// Something the hook runtime did that the owner of the run may want to act on.
pub enum HookRuntimeEvent {
    /// A hook asked for confirmation on a surface with no one to ask, and the run asked to end
    /// when that happens. The action was already denied with a reason the model can read; the
    /// run's owner decides whether the run goes on.
    ApprovalUnavailable {
        conversation_id: AIConversationId,
        action_id: AIAgentActionId,
        reason: String,
    },
}

/// Process-wide hook state shared by every conversation and surface. It also wires discovery:
/// it hands detected repository roots to the hook file watcher and pins the active set when the
/// run's first request goes out.
pub struct HookRuntime {
    registry: ModelHandle<HookRegistry>,
    dispatcher: Arc<HookDispatcher>,
    messenger: Arc<dyn HookMessenger>,
    run: Option<RunIdentity>,
    /// The explicit sources the run was configured with, whether or not each one loaded.
    delivered_sources: Vec<ExplicitHookSource>,
    /// The diagnostics already known when the run pinned, plus those reported since. `None`
    /// until the run pins.
    diagnostics_seen_since_pin: Option<Vec<HookDiagnostic>>,
    strict_policy: StrictHookPolicy,
    headless_ask_behavior: HeadlessAskBehavior,
    pre_tool_verdicts: PreToolVerdictCache,
    stream_observer: StreamObserverState,
    /// The turn each conversation is in, allocated when a prompt starts a turn.
    turn_ids: HashMap<AIConversationId, HookTurnId>,
    session_starts: SessionStarts,
    turn_stops: TurnStops,
    pending_prompts: PendingPrompts,
    shell_completions: ShellCompletions,
    #[cfg(feature = "local_fs")]
    config_watcher: Option<ModelHandle<HookConfigWatcher>>,
    #[cfg(test)]
    observed_terminal_results: Vec<tool_result::ObservedTerminalResult>,
}

impl HookRuntime {
    pub fn new(
        runner: Arc<dyn HookRunner>,
        messenger: Arc<dyn HookMessenger>,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        Self::subscribe_to_run_start(ctx);
        Self::subscribe_to_registry_changes(ctx);
        #[cfg(feature = "local_fs")]
        Self::subscribe_to_detected_repositories(ctx);
        Self {
            registry: HookRegistry::handle(ctx),
            dispatcher: Arc::new(HookDispatcher::new(runner, messenger.clone())),
            messenger,
            run: None,
            delivered_sources: Vec::new(),
            diagnostics_seen_since_pin: None,
            strict_policy: StrictHookPolicy::default(),
            headless_ask_behavior: HeadlessAskBehavior::default(),
            pre_tool_verdicts: PreToolVerdictCache::default(),
            stream_observer: StreamObserverState::default(),
            turn_ids: HashMap::new(),
            session_starts: SessionStarts::default(),
            turn_stops: TurnStops::default(),
            pending_prompts: PendingPrompts::default(),
            shell_completions: ShellCompletions::default(),
            #[cfg(feature = "local_fs")]
            config_watcher: None,
            #[cfg(test)]
            observed_terminal_results: Vec::new(),
        }
    }

    /// The run starts when its first request goes out, which is when the first exchange is
    /// appended to a conversation. That is the latest moment before any tool can run, and it
    /// comes after a command-line run has applied its profile. The same subscription drops a
    /// conversation's state once the conversation is gone.
    fn subscribe_to_run_start(ctx: &mut ModelContext<Self>) {
        if !ctx.has_singleton_model::<BlocklistAIHistoryModel>() {
            return;
        }
        ctx.subscribe_to_model(
            &BlocklistAIHistoryModel::handle(ctx),
            |me, _, event, ctx| match event {
                BlocklistAIHistoryEvent::AppendedExchange {
                    terminal_surface_id,
                    ..
                } => me.pin_for_run(Some(*terminal_surface_id), ctx),
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
                BlocklistAIHistoryEvent::StartedNewConversation { .. }
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

    /// After the run pins, a repository's hook file that is found, changed, or broken can no
    /// longer take effect; the registry records why, and this reports it through the messenger so
    /// the run's output says so too.
    fn subscribe_to_registry_changes(ctx: &mut ModelContext<Self>) {
        ctx.subscribe_to_model(
            &HookRegistry::handle(ctx),
            |me, _, event, ctx| match event {
                HookRegistryEvent::SnapshotChanged { .. } => me.report_project_diagnostics(ctx),
            },
        );
    }

    fn report_project_diagnostics(&mut self, ctx: &AppContext) {
        let Some(seen) = self.diagnostics_seen_since_pin.as_mut() else {
            return;
        };
        for diagnostic in &self.registry.as_ref(ctx).snapshot().diagnostics {
            let is_project_file = match diagnostic.source.kind {
                HookSourceKind::ProjectCursorFile => true,
                HookSourceKind::RunDelivered
                | HookSourceKind::FactoryAgent
                | HookSourceKind::FactoryWide => false,
            };
            if is_project_file && !seen.contains(diagnostic) {
                self.messenger.diagnostic(diagnostic.clone());
                seen.push(diagnostic.clone());
            }
        }
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

    #[cfg(feature = "local_fs")]
    fn subscribe_to_detected_repositories(ctx: &mut ModelContext<Self>) {
        if !ctx.has_singleton_model::<DetectedRepositories>() {
            return;
        }
        ctx.subscribe_to_model(&DetectedRepositories::handle(ctx), |me, _, event, ctx| {
            let DetectedRepositoriesEvent::DetectedGitRepo { repository, source } = event;
            me.watch_detected_root(repository, *source, ctx);
        });
    }

    /// Fixes the active hook set for the rest of the run. The command execution posture is read
    /// here, once, from the profile active for `terminal_view_id`; project files are trusted
    /// against it and the registry is pinned. Explicit sources must already be loaded, since a
    /// source loaded afterwards cannot activate. Later calls do nothing.
    pub fn pin_for_run(
        &mut self,
        terminal_view_id: Option<EntityId>,
        ctx: &mut ModelContext<Self>,
    ) {
        if !Self::process_runs_agents(ctx) || self.registry.as_ref(ctx).pinned().is_some() {
            return;
        }
        let posture = command_execution_posture(terminal_view_id, ctx);
        let surface = self.surface(ctx);
        #[cfg(feature = "local_fs")]
        if let Some(watcher) = &self.config_watcher {
            watcher.update(ctx, |watcher, ctx| {
                watcher.set_trust_inputs(surface, posture, ctx);
            });
        }
        let revision = HookRegistry::change(&self.registry, ctx, |registry| registry.pin(posture));
        self.diagnostics_seen_since_pin =
            Some(self.registry.as_ref(ctx).snapshot().diagnostics.clone());
        log::info!(
            "hook registry pinned for the run: surface={surface:?} posture={posture:?} \
             revision={revision}"
        );
    }

    /// The surface this process fires boundaries from.
    pub(crate) fn surface(&self, ctx: &AppContext) -> HookSurface {
        envelope::surface(self.run_surface(), ctx)
    }

    /// The surface of the run this process belongs to, once one was recorded.
    pub(crate) fn run_surface(&self) -> Option<RunSurface> {
        self.run.as_ref().map(|run| run.surface)
    }

    /// The command execution posture of the profile active for `terminal_view_id`.
    pub(crate) fn command_execution_posture(
        terminal_view_id: Option<EntityId>,
        ctx: &AppContext,
    ) -> CommandExecutionPosture {
        command_execution_posture(terminal_view_id, ctx)
    }

    /// Whether nobody can answer a confirmation request for `conversation_id`: the process is a
    /// run, or the conversation is a hidden local child.
    pub fn is_headless_for(&self, conversation_id: AIConversationId, ctx: &AppContext) -> bool {
        envelope::surface_for_conversation(conversation_id, self.run_surface(), ctx).is_headless()
    }

    pub fn strict_policy(&self) -> StrictHookPolicy {
        self.strict_policy
    }

    pub fn headless_ask_behavior(&self) -> HeadlessAskBehavior {
        self.headless_ask_behavior
    }

    /// Whether hooks are on and this process is one that runs an agent. The remote server daemon
    /// serves SSH sessions on a remote host; remote hosts are not a hook surface, so it neither
    /// discovers project files nor pins a run.
    fn process_runs_agents(ctx: &AppContext) -> bool {
        FeatureFlag::AgentHooks.is_enabled()
            && !(ctx.has_singleton_model::<AppExecutionMode>()
                && AppExecutionMode::as_ref(ctx).is_remote_server_daemon())
    }

    /// Starts watching a detected repository's hook file when the surface watches roots found
    /// the way this one was. Roots from rule indexing and code review are not where the agent's
    /// session is, so only navigation and cloud preparation count.
    #[cfg(feature = "local_fs")]
    fn watch_detected_root(
        &mut self,
        repository: &ModelHandle<Repository>,
        source: RepoDetectionSource,
        ctx: &mut ModelContext<Self>,
    ) {
        if !Self::process_runs_agents(ctx) {
            return;
        }
        let is_cloud_prepared_root = match source {
            RepoDetectionSource::CloudEnvironmentPrep => true,
            RepoDetectionSource::TerminalNavigation => false,
            RepoDetectionSource::ProjectRulesIndexing
            | RepoDetectionSource::CodeReviewInitialization => return,
        };
        let surface = self.surface(ctx);
        if !watch_matrix(surface, is_cloud_prepared_root) {
            return;
        }
        let Some(root) = repository.as_ref(ctx).root_dir().to_local_path() else {
            return;
        };
        let watcher = self.config_watcher_or_start(surface, ctx);
        watcher.update(ctx, |watcher, ctx| {
            // The watch is requested either way; the future only says when it has attached.
            drop(watcher.add_root(&root, ctx));
        });
    }

    /// The watcher, started on first use so a process that never visits a repository runs none.
    #[cfg(feature = "local_fs")]
    fn config_watcher_or_start(
        &mut self,
        surface: HookSurface,
        ctx: &mut ModelContext<Self>,
    ) -> ModelHandle<HookConfigWatcher> {
        self.config_watcher
            .get_or_insert_with(|| {
                let registry = HookRegistry::handle(ctx);
                ctx.add_model(|ctx| {
                    HookConfigWatcher::new(registry, Arc::new(StaticTrustPolicy), surface, ctx)
                })
            })
            .clone()
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
        self.run.as_ref().map(|run| &run.metadata)
    }

    pub(crate) fn delivered_sources(&self) -> &[ExplicitHookSource] {
        &self.delivered_sources
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

    /// Records the run this process belongs to. Envelopes built afterwards carry it and report
    /// its surface.
    pub(crate) fn set_run(&mut self, run: Option<RunIdentity>) {
        self.run = run;
    }

    pub(crate) fn set_delivered_sources(&mut self, sources: Vec<ExplicitHookSource>) {
        self.delivered_sources = sources;
    }

    /// Turns the strict hook policy on or off for every dispatch from now on.
    pub fn set_strict_policy(&mut self, policy: StrictHookPolicy) {
        self.strict_policy = policy;
    }

    /// Chooses what a hook's `ask` does on a surface with no one to ask.
    pub(crate) fn set_headless_ask_behavior(&mut self, behavior: HeadlessAskBehavior) {
        self.headless_ask_behavior = behavior;
    }

    /// Replaces the messenger for notices and diagnostics, including those the dispatcher routes.
    pub fn set_messenger(&mut self, messenger: Arc<dyn HookMessenger>) {
        self.dispatcher = Arc::new(HookDispatcher::new(
            self.dispatcher.runner().clone(),
            messenger.clone(),
        ));
        self.messenger = messenger;
    }

    /// The run-wide choices every dispatch is made under.
    pub(crate) fn dispatch_policy(&self) -> DispatchPolicy {
        self.strict_policy.dispatch_policy()
    }

    /// Starts a dispatch on the background executor and detaches its future.
    pub(crate) fn dispatch_detached(&self, envelope: HookEnvelope, ctx: &AppContext) {
        let future = self.dispatcher.dispatch(
            envelope,
            self.snapshot(ctx),
            self.dispatch_policy(),
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

/// Whether the run already lets the agent execute commands without asking, read from the
/// execute-commands setting of the profile active for `terminal_view_id` through every open
/// window's team policy, so a workspace rule that tightens the profile is seen. The
/// per-conversation auto-approve toggle does not count: it is one user's choice for one
/// conversation, not the run's configuration. Without a window there is no team policy to read
/// through, so the posture is unknown.
fn command_execution_posture(
    terminal_view_id: Option<EntityId>,
    ctx: &AppContext,
) -> CommandExecutionPosture {
    if !ctx.has_singleton_model::<UserWorkspaces>() {
        return CommandExecutionPosture::Unknown;
    }
    let workspaces = UserWorkspaces::as_ref(ctx);
    let scopes = ctx
        .window_ids()
        .map(|window_id| workspaces.team_context_for_window_operation(window_id))
        .collect::<Vec<_>>();
    posture_for_scopes(terminal_view_id, &scopes, ctx)
}

/// The strictest posture the active profile has under any of `scopes`.
fn posture_for_scopes(
    terminal_view_id: Option<EntityId>,
    scopes: &[impl TeamScope],
    ctx: &AppContext,
) -> CommandExecutionPosture {
    // Everything get_execute_commands_setting reads; a test app may register only some of it.
    if !ctx.has_singleton_model::<BlocklistAIPermissions>()
        || !ctx.has_singleton_model::<AIExecutionProfilesModel>()
        || !ctx.has_singleton_model::<AppExecutionMode>()
    {
        return CommandExecutionPosture::Unknown;
    }
    let permissions = BlocklistAIPermissions::as_ref(ctx);
    CommandExecutionPosture::strictest_of(scopes.iter().map(|scope| {
        posture_for_setting(permissions.get_execute_commands_setting(terminal_view_id, scope, ctx))
    }))
}

fn posture_for_setting(setting: ActionPermission) -> CommandExecutionPosture {
    match setting {
        ActionPermission::AlwaysAllow => CommandExecutionPosture::AutoApproves,
        ActionPermission::AlwaysAsk | ActionPermission::AgentDecides => {
            CommandExecutionPosture::RequiresApproval
        }
        ActionPermission::Unknown => CommandExecutionPosture::Unknown,
    }
}

/// The longest timeout among the bindings in `snapshot` that would run for `envelope`, which is
/// how long a boundary that waits for its hooks waits. `None` when no binding would run.
fn longest_matching_timeout(
    snapshot: &RegistrySnapshot,
    envelope: &HookEnvelope,
) -> Option<Duration> {
    snapshot
        .bindings_for(
            envelope.boundary(),
            envelope.cwd.as_deref(),
            envelope.execution_host,
        )
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

#[cfg(test)]
#[path = "mod_test.rs"]
mod tests;

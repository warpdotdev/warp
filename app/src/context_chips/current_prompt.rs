use std::collections::{HashMap, HashSet};
use std::hash::{Hash as _, Hasher as _};
use std::sync::Arc;
use std::time::Duration;

use futures::{pin_mut, FutureExt as _};
use itertools::Itertools;
use warp_completer::completer::CommandExitStatus;
use warp_core::r#async::debounce;
use warp_core::user_preferences::GetUserPreferences;
use warp_errors::report_error;
use warpui::r#async::{SpawnedFutureHandle, Timer};
use warpui::{
    AppContext, Entity, ModelAsRef, ModelContext, ModelHandle, SingletonEntity, ViewHandle,
    WeakModelHandle,
};

use super::context_chip::{
    ChipAvailability, ChipFingerprintInput, ChipRuntimeCapabilities, ContextChip, Environment,
    ExternalCommandsAvailability, GeneratorContext, PromptGenerator, RefreshConfig,
    ShellCommandGenerator,
};
use super::logging::{ChipCommandLogEntry, PromptChipExecutionPhase, PromptChipLogger};
use super::prompt::Prompt;
use super::{chips_to_string, ChipResult, ChipValue, ContextChipKind};
use crate::code_review::git_repo_model::{GitRepoStatusEvent, GitRepoStatusModel};
use crate::code_review::github_repo_model::{GitHubRepoEvent, GitHubRepoModel};
use crate::context_chips::display_chip::GitLineChanges;
use crate::editor::EditorView;
use crate::features::FeatureFlag;
use crate::menu::{MenuItem, MenuItemFields};
use crate::settings::{InputSettings, WarpPromptSeparator};
use crate::terminal::event::{BlockType, UserBlockCompleted};
use crate::terminal::model::block::{Block, BlockMetadata};
use crate::terminal::model::session::{ExecuteCommandOptions, Session, Sessions, SessionsEvent};
use crate::terminal::model_events::{ModelEvent, ModelEventDispatcher};
use crate::terminal::session_settings::{
    SessionSettings, SessionSettingsChangedEvent, ToolbarChipSelection,
};
use crate::terminal::view::{ContextMenuAction, PromptPart, PromptPosition, TerminalAction};

#[cfg(test)]
#[path = "current_prompt_tests.rs"]
mod tests;

const PROMPT_DEBOUNCE_PERIOD: Duration = Duration::from_millis(50);
const PROMPT_DEBOUNCE_PERIOD_KEY: &str = "PromptDebouncePeriod";
type ChipFingerprint = u64;

/// The lifecycle state of a chip's value computation within a [`CurrentPrompt`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
enum ChipUpdateStatus {
    #[default]
    Idle,
    Loading,
    Ready,
    Cached,
    Disabled,
    TimedOut,
    Error,
}

/// ChipState stores the state and point-in-time information related to a specific chip.
/// For example, it's last computed value or a refresh handle.
#[derive(Clone, Debug, Default)]
pub struct ChipState {
    last_computed_value: Option<ChipValue>,
    last_on_click_values: Option<Vec<String>>,
    last_fingerprint: Option<ChipFingerprint>,
    /// The fingerprint from the last fetch that failed. When the current fingerprint matches,
    /// chips with `suppress_on_failure` skip re-execution.
    last_failure_fingerprint: Option<ChipFingerprint>,
    availability: ChipAvailability,
    update_status: ChipUpdateStatus,
    /// Future handle for periodically-refreshing chips.
    refresh_handle: Option<SpawnedFutureHandle>,
    /// Future handle for asynchronous generators.
    generator_handle: Option<SpawnedFutureHandle>,
    /// Future handle for asynchronous on-click generators.
    on_click_generator_handle: Option<SpawnedFutureHandle>,
    /// Whether the chip should render or not.
    should_render: bool,
    /// Monotonic counter incremented when a user command matching this chip's
    /// `invalidate_on_commands` completes. Hashed via `ChipFingerprintInput::InvalidatingCommandCount`.
    invalidating_command_count: u64,
}

impl Drop for ChipState {
    fn drop(&mut self) {
        if let Some(refresh_handle) = self.refresh_handle.take() {
            refresh_handle.abort();
        }

        if let Some(generator_handle) = self.generator_handle.take() {
            generator_handle.abort();
        }

        if let Some(generator_handle) = self.on_click_generator_handle.take() {
            generator_handle.abort();
        }
    }
}

impl ChipState {
    fn new(kind: &ContextChipKind) -> Self {
        Self {
            last_computed_value: None,
            last_on_click_values: None,
            last_fingerprint: None,
            last_failure_fingerprint: None,
            availability: ChipAvailability::Enabled,
            update_status: ChipUpdateStatus::Idle,
            refresh_handle: None,
            generator_handle: None,
            on_click_generator_handle: None,
            should_render: kind.should_render("", &Default::default()),
            invalidating_command_count: 0,
        }
    }

    fn clear_abort_handlers(&mut self) {
        if let Some(refresh_handle) = self.refresh_handle.take() {
            refresh_handle.abort();
        }
        if let Some(generator_handle) = self.generator_handle.take() {
            generator_handle.abort();
        }
        if let Some(generator_handle) = self.on_click_generator_handle.take() {
            generator_handle.abort();
        }
    }

    fn clear_cache(&mut self) {
        self.last_computed_value = None;
        self.last_on_click_values = None;
        self.last_fingerprint = None;
        self.last_failure_fingerprint = None;
        self.availability = ChipAvailability::Enabled;
        self.update_status = ChipUpdateStatus::Idle;
    }
}

/// CurrentPrompt is a model initialized per session that represents the actual prompt for a given
/// session. It subscribes to the singleton prompt model to get the current settings, and then
/// stores the states for each chip and manages the refreshing logic.
#[derive(Clone)]
pub struct CurrentPrompt {
    states: HashMap<ContextChipKind, ChipState>,
    renderable_chips: HashSet<ContextChipKind>,

    same_line_prompt_enabled: bool,
    /// The separator to use as a trailing character at the end of Warp prompt, if any.
    separator: WarpPromptSeparator,

    latest_context: Option<PromptContext>,
    sessions: ModelHandle<Sessions>,
    prompt_chip_logger: PromptChipLogger,
    update_tx: async_channel::Sender<()>,

    /// When set, branch, branch status, and diff stats are populated from
    /// `GitRepoStatusModel` filesystem events.
    git_repo_status: Option<WeakModelHandle<GitRepoStatusModel>>,

    /// When set, the `GithubPullRequest` chip value is populated from
    /// `GitHubRepoModel` for the current repository.
    github_repo_model: Option<WeakModelHandle<GitHubRepoModel>>,
}

/// Context about the current terminal session, needed to update the prompt.
#[derive(Clone, Debug)]
struct PromptContext {
    active_block_metadata: BlockMetadata,
    environment: Environment,
}

#[derive(Clone)]
struct ShellCommandExecutionContext {
    session: Arc<Session>,
    command: String,
    current_dir_path: Option<String>,
    environment_variables: Option<HashMap<String, String>>,
    shell_type: crate::terminal::shell::ShellType,
}

impl CurrentPrompt {
    pub fn new(sessions: ModelHandle<Sessions>, ctx: &mut ModelContext<Self>) -> Self {
        Self::new_with_model_events(sessions, None, ctx)
    }

    pub fn new_with_model_events(
        sessions: ModelHandle<Sessions>,
        model_events: Option<&ModelHandle<ModelEventDispatcher>>,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        let prompt = Prompt::handle(ctx);
        ctx.subscribe_to_model(&prompt, Self::handle_prompt_changed);
        ctx.subscribe_to_model(
            &SessionSettings::handle(ctx),
            Self::handle_session_settings_changed,
        );
        ctx.subscribe_to_model(&sessions, |me, _, event, ctx| {
            if let SessionsEvent::EnvironmentVariablesUpdated { .. } = event {
                me.update_states_with_new_context(ctx);
            }
        });

        if let Some(model_events) = model_events {
            ctx.subscribe_to_model(model_events, Self::handle_model_event);
        }

        let (update_tx, update_rx) = async_channel::unbounded();
        let debounce_period = ctx
            .private_user_preferences()
            .read_value(PROMPT_DEBOUNCE_PERIOD_KEY)
            .ok()
            .flatten()
            .and_then(|s| s.parse().ok())
            .map(Duration::from_millis)
            .unwrap_or(PROMPT_DEBOUNCE_PERIOD);

        // Debounce rendering updates to the prompt
        ctx.spawn_stream_local(
            debounce(debounce_period, update_rx),
            |_, _, ctx| ctx.notify(),
            |_, _| {},
        );
        Self {
            states: Default::default(),
            renderable_chips: Default::default(),
            sessions,
            latest_context: None,
            prompt_chip_logger: PromptChipLogger::default(),
            update_tx,
            same_line_prompt_enabled: prompt.as_ref(ctx).same_line_prompt_enabled(),
            separator: prompt.as_ref(ctx).separator(),
            git_repo_status: None,
            github_repo_model: None,
        }
    }

    /// This is used to subscribe to an editor view (i.e. in the input) whose buffer
    /// we'd like to use to update chip state.
    pub fn subscribe_to_input_editor(
        &self,
        editor: ViewHandle<EditorView>,
        ctx: &mut ModelContext<Self>,
    ) {
        // A WeakViewHandle is used here to avoid leaking the terminal model
        let weak_editor_handle = editor.downgrade();
        ctx.subscribe_to_view(&editor, move |me, _, _, ctx| {
            // CurrentPrompt exists and this fn is called even if we're not using warp prompt.
            // We don't need to do anything if we're honoring PS1 unless universal developer input
            // or AgentView is enabled (agent view needs chips regardless of PS1 setting).
            if *SessionSettings::as_ref(ctx).honor_ps1
                && !InputSettings::as_ref(ctx).is_universal_developer_input_enabled(ctx)
                && !FeatureFlag::AgentView.is_enabled()
            {
                return;
            }
            let Some(editor) = weak_editor_handle.upgrade(ctx) else {
                return;
            };

            let latest_context = me.latest_context.clone();
            if let Some(context) = latest_context {
                if let Some(session_id) = context.active_block_metadata.session_id() {
                    let session = me
                        .sessions
                        .update(ctx, |sessions, _| sessions.get(session_id));

                    if let Some(session) = session {
                        let buffer_text = editor.as_ref(ctx).buffer_text(ctx);
                        for (kind, state) in me.states.iter_mut() {
                            state.should_render =
                                kind.should_render(&buffer_text, session.aliases());
                        }
                        ctx.notify();
                    }
                }
            }
        });
    }

    pub fn snapshot(&self) -> HashMap<ContextChipKind, Option<ChipValue>> {
        let cur = self
            .states
            .iter()
            .filter_map(|(kind, state)| {
                if state.should_render && !matches!(state.availability, ChipAvailability::Hidden) {
                    Some((kind.clone(), state.last_computed_value.clone()))
                } else {
                    None
                }
            })
            .collect();
        cur
    }

    pub fn on_click_snapshot(&self) -> HashMap<ContextChipKind, Vec<String>> {
        self.states
            .iter()
            .filter_map(|(kind, state)| {
                if matches!(state.availability, ChipAvailability::Hidden) {
                    return None;
                }
                state
                    .last_on_click_values
                    .clone()
                    .map(|values| (kind.clone(), values))
            })
            .collect()
    }

    /// Whether same line prompt is enabled for the Warp prompt.
    pub fn same_line_prompt_enabled(&self) -> bool {
        self.same_line_prompt_enabled
    }

    /// The separator for the current Warp prompt.
    pub fn separator(&self) -> WarpPromptSeparator {
        self.separator
    }

    fn update_chip_value(&mut self, chip_kind: &ContextChipKind, value: Option<ChipValue>) {
        log::debug!("Updating prompt value of {chip_kind:?} to {value:?}");
        if let Some(state) = self.states.get_mut(chip_kind) {
            if state.last_computed_value != value {
                state.last_computed_value = value;
                state.update_status = ChipUpdateStatus::Ready;
                let _ = self.update_tx.try_send(());
            }
        }
    }

    fn update_on_click_value(&mut self, chip_kind: &ContextChipKind, value: Option<Vec<String>>) {
        log::debug!("Updating prompt on_click value of {chip_kind:?} to {value:?}");
        let filter_values = match chip_kind {
            ContextChipKind::ShellGitBranch => self.filter_git_branch_on_click_values(value),
            _ => value,
        };
        if let Some(state) = self.states.get_mut(chip_kind) {
            state.last_on_click_values = filter_values;
            let _ = self.update_tx.try_send(());
        }
    }

    fn set_chip_availability(
        &mut self,
        chip_kind: &ContextChipKind,
        availability: ChipAvailability,
    ) {
        if let Some(state) = self.states.get_mut(chip_kind) {
            if state.availability != availability {
                state.availability = availability;
                let _ = self.update_tx.try_send(());
            }
        }
    }

    fn set_chip_update_status(&mut self, chip_kind: &ContextChipKind, status: ChipUpdateStatus) {
        if let Some(state) = self.states.get_mut(chip_kind) {
            state.update_status = status;
        }
    }

    fn set_chip_fingerprint(
        &mut self,
        chip_kind: &ContextChipKind,
        fingerprint: Option<ChipFingerprint>,
    ) {
        if let Some(state) = self.states.get_mut(chip_kind) {
            state.last_fingerprint = fingerprint;
        }
    }

    fn chip_runtime_capabilities_for_session(
        &self,
        session: Option<&Session>,
        required_executables: &[String],
        include_external_command_count: bool,
    ) -> ChipRuntimeCapabilities {
        session
            .map(|session| {
                ChipRuntimeCapabilities::from_session_with_external_command_queries(
                    session,
                    required_executables.iter().map(String::as_str),
                    include_external_command_count,
                )
            })
            .unwrap_or_default()
    }

    fn build_chip_fingerprint(
        &self,
        chip_kind: &ContextChipKind,
        chip: &ContextChip,
        required_executables: &[String],
        context: &GeneratorContext,
        capabilities: &ChipRuntimeCapabilities,
    ) -> Option<ChipFingerprint> {
        let inputs = chip.runtime_policy().fingerprint_inputs();
        if inputs.is_empty() {
            return None;
        }

        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        for input in inputs {
            input.hash(&mut hasher);
            match input {
                ChipFingerprintInput::SessionId => {
                    context.active_block_metadata.session_id().hash(&mut hasher);
                }
                ChipFingerprintInput::SessionIsLocal => {
                    context
                        .active_session
                        .map(Session::is_local)
                        .hash(&mut hasher);
                }
                ChipFingerprintInput::WorkingDirectory => {
                    context
                        .active_block_metadata
                        .current_working_directory()
                        .hash(&mut hasher);
                }
                ChipFingerprintInput::GitBranch => {
                    context.current_environment.git_branch().hash(&mut hasher);
                }
                ChipFingerprintInput::PythonVirtualenv => {
                    context
                        .current_environment
                        .python_virtualenv()
                        .hash(&mut hasher);
                }
                ChipFingerprintInput::CondaEnvironment => {
                    context
                        .current_environment
                        .conda_environment()
                        .hash(&mut hasher);
                }
                ChipFingerprintInput::NodeVersion => {
                    context.current_environment.node_version().hash(&mut hasher);
                }
                ChipFingerprintInput::SessionUser => {
                    context.active_session.map(Session::user).hash(&mut hasher);
                }
                ChipFingerprintInput::SessionHostname => {
                    context
                        .active_session
                        .map(Session::hostname)
                        .hash(&mut hasher);
                }
                ChipFingerprintInput::ExternalCommandsState => {
                    match &capabilities.external_commands {
                        ExternalCommandsAvailability::Unknown => {
                            0u8.hash(&mut hasher);
                        }
                        ExternalCommandsAvailability::Known { command_count, .. } => {
                            1u8.hash(&mut hasher);
                            command_count.hash(&mut hasher);
                        }
                    }
                }
                ChipFingerprintInput::RequiredExecutablesPresence => {
                    let mut cmds = required_executables
                        .iter()
                        .map(String::as_str)
                        .collect_vec();
                    cmds.sort_unstable();
                    for cmd in cmds {
                        cmd.hash(&mut hasher);
                        capabilities
                            .external_commands
                            .contains(cmd)
                            .hash(&mut hasher);
                    }
                }
                ChipFingerprintInput::InvalidatingCommandCount => {
                    if let Some(state) = self.states.get(chip_kind) {
                        state.invalidating_command_count.hash(&mut hasher);
                    }
                }
            }
        }

        Some(hasher.finish())
    }

    fn maybe_skip_fetch_due_to_matching_fingerprint(
        &mut self,
        chip_kind: &ContextChipKind,
        new_fingerprint: Option<ChipFingerprint>,
        allow_fingerprint_skip: bool,
    ) -> bool {
        if !allow_fingerprint_skip {
            return false;
        }

        let Some(new_fingerprint) = new_fingerprint else {
            return false;
        };

        // A retryable failure (`Error`, `TimedOut`) is not a usable cached
        // result: `last_fingerprint` is recorded before the command runs, and
        // such failures intentionally do not populate `last_failure_fingerprint`.
        // Without this guard, the next periodic tick would treat the failed
        // attempt as a cache hit and never retry. Deterministic failures
        // continue to be suppressed via `last_failure_fingerprint`.
        let should_skip = self
            .states
            .get(chip_kind)
            .filter(|state| {
                !matches!(
                    state.update_status,
                    ChipUpdateStatus::Error | ChipUpdateStatus::TimedOut
                )
            })
            .and_then(|state| state.last_fingerprint.as_ref())
            .is_some_and(|existing| existing == &new_fingerprint);

        if should_skip {
            self.set_chip_update_status(chip_kind, ChipUpdateStatus::Cached);
            return true;
        }

        self.set_chip_fingerprint(chip_kind, Some(new_fingerprint));
        false
    }

    fn with_current_generator_context<R>(
        &self,
        ctx: &AppContext,
        func: impl FnOnce(&GeneratorContext) -> R,
    ) -> Option<R> {
        self.with_generator_context(ctx, |generator_context| Some(func(generator_context)))
    }

    fn prepare_shell_command_context(
        &self,
        cmd: &ShellCommandGenerator,
        ctx: &AppContext,
    ) -> Option<ShellCommandExecutionContext> {
        let latest_context = self.latest_context.as_ref()?;
        let session_id = latest_context.active_block_metadata.session_id()?;

        let (session, mut environment_variables) = self.sessions.read(ctx, |sessions, _| {
            (
                sessions.get(session_id),
                sessions.get_env_vars_for_session(session_id),
            )
        });

        let session = session?;
        let shell_type = session.shell().shell_type();
        let command = cmd.command().for_shell(shell_type).map(str::to_owned)?;

        let current_dir_path = latest_context
            .active_block_metadata
            .current_working_directory()
            .map(ToOwned::to_owned);

        let path_env_var = session.path().as_deref().map(str::to_owned);
        if let (Some(path_var), Some(env_vars)) = (path_env_var, environment_variables.as_mut()) {
            env_vars.insert("PATH".to_string(), path_var);
        }

        Some(ShellCommandExecutionContext {
            session,
            command,
            current_dir_path,
            environment_variables,
            shell_type,
        })
    }

    /// Races command execution against a timeout.
    ///
    /// On timeout we drop the in-flight `execute_command` future, which is the only per-command
    /// cancellation mechanism exposed here today. That drop path triggers actual cancellation for
    /// local and in-band executors (for example `kill_on_drop` / `on_cancel`), but we intentionally
    /// do not call `session.cancel_active_commands()` because it is session-global and would cancel
    /// unrelated generator commands as well.
    async fn execute_session_command_with_timeout(
        session: Arc<Session>,
        command: String,
        current_dir_path: Option<String>,
        environment_variables: Option<HashMap<String, String>>,
        timeout: Option<Duration>,
    ) -> (Option<warp_completer::completer::CommandOutput>, bool) {
        let command_future = session
            .execute_command(
                &command,
                current_dir_path.as_deref(),
                environment_variables,
                ExecuteCommandOptions::default(),
            )
            .fuse();
        let timeout_future = match timeout {
            Some(duration) => Timer::after(duration),
            None => Timer::never(),
        }
        .fuse();
        pin_mut!(command_future);
        pin_mut!(timeout_future);

        futures::select! {
            result = command_future => (result.ok(), false),
            _ = timeout_future => (None, true),
        }
    }

    fn filter_git_branch_on_click_values(
        &self,
        values_opt: Option<Vec<String>>,
    ) -> Option<Vec<String>> {
        super::git_branch_on_click::filter_git_branch_on_click_values(values_opt)
    }

    /// Perform a single update of the given chip.
    ///
    /// If the chip's generator runs asynchronously, this will update its generator future handle.
    fn fetch_chip_value_once(
        &mut self,
        chip_kind: &ContextChipKind,
        generator: &PromptGenerator,
        on_click_generator: Option<PromptGenerator>,
        allow_fingerprint_skip: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(chip) = chip_kind.to_chip() else {
            report_error!("Undefined chip", extra: { "chip_kind" => ?chip_kind });
            return;
        };

        let required_executables = chip.runtime_policy().required_executables();
        let include_external_command_count = chip
            .runtime_policy()
            .fingerprint_inputs()
            .contains(&ChipFingerprintInput::ExternalCommandsState);
        let (availability, fingerprint) = self
            .with_current_generator_context(ctx, |generator_context| {
                let capabilities = self.chip_runtime_capabilities_for_session(
                    generator_context.active_session,
                    required_executables,
                    include_external_command_count,
                );
                (
                    chip.availability(&capabilities),
                    self.build_chip_fingerprint(
                        chip_kind,
                        &chip,
                        required_executables,
                        generator_context,
                        &capabilities,
                    ),
                )
            })
            .unwrap_or((ChipAvailability::Enabled, None));
        self.set_chip_availability(chip_kind, availability.clone());
        if !availability.is_enabled() {
            if let Some(state) = self.states.get_mut(chip_kind) {
                if let Some(handle) = state.generator_handle.take() {
                    handle.abort();
                }
                if let Some(handle) = state.on_click_generator_handle.take() {
                    handle.abort();
                }
            }
            self.update_chip_value(chip_kind, None);
            self.update_on_click_value(chip_kind, None);
            self.set_chip_update_status(chip_kind, ChipUpdateStatus::Disabled);
            return;
        }
        if self.maybe_skip_fetch_due_to_matching_fingerprint(
            chip_kind,
            fingerprint,
            allow_fingerprint_skip,
        ) {
            return;
        }

        if chip.runtime_policy().suppress_on_failure() {
            if let Some(state) = self.states.get(chip_kind) {
                if let Some(current_fp) = &fingerprint {
                    if state.last_failure_fingerprint.as_ref() == Some(current_fp) {
                        self.update_chip_value(chip_kind, None);
                        self.update_on_click_value(chip_kind, None);
                        self.set_chip_update_status(chip_kind, ChipUpdateStatus::Cached);
                        return;
                    }
                }
            }
        }
        match generator {
            PromptGenerator::ShellCommand(cmd) => {
                let Some(exec_ctx) = self.prepare_shell_command_context(cmd, ctx) else {
                    log::warn!("Generator for {chip_kind:?}: could not prepare execution context");
                    self.update_chip_value(chip_kind, None);
                    self.update_on_click_value(chip_kind, None);
                    self.set_chip_update_status(chip_kind, ChipUpdateStatus::Error);
                    return;
                };

                let chip_kind = chip_kind.clone();
                let Some(state) = self.states.get_mut(&chip_kind) else {
                    log::warn!("Tried to run generator for {chip_kind:?}, but state was missing");
                    return;
                };

                if let Some(handle) = state.generator_handle.take() {
                    handle.abort();
                }
                state.update_status = ChipUpdateStatus::Loading;

                let timeout = chip.runtime_policy().shell_command_timeout();
                let suppress_on_failure = chip.runtime_policy().suppress_on_failure();
                let allow_empty_value = chip.allow_empty_value();
                let chip_title = chip.title().to_owned();
                let current_fingerprint = fingerprint;
                let logger = self.prompt_chip_logger.clone();
                let handle = ctx.spawn(
                    async move {
                        let (value, timed_out) = Self::execute_session_command_with_timeout(
                            exec_ctx.session.clone(),
                            exec_ctx.command.clone(),
                            exec_ctx.current_dir_path.clone(),
                            exec_ctx.environment_variables.clone(),
                            timeout,
                        )
                        .await;
                        (value, timed_out, chip_kind, exec_ctx, chip_title)
                    },
                    move |me, (value, timed_out, chip_kind, exec_ctx, chip_title), _| {
                        logger.log_shell_command(&ChipCommandLogEntry {
                            chip_kind: &chip_kind,
                            chip_title: &chip_title,
                            phase: PromptChipExecutionPhase::Value,
                            shell_type: exec_ctx.shell_type,
                            working_directory: exec_ctx.current_dir_path.as_deref(),
                            command: &exec_ctx.command,
                            output: value.as_ref(),
                            timed_out,
                        });
                        // GitDiffStats has two value sources that can race when entering a repo:
                        // this shell fallback (`git diff --shortstat HEAD`, tracked changes only)
                        // and a repo-status watcher that also counts untracked files. If the
                        // watcher attached while this fallback was in flight, drop the fallback's
                        // result
                        if matches!(chip_kind, ContextChipKind::GitDiffStats)
                            && me.is_updated_externally(&chip_kind)
                        {
                            return;
                        }

                        if timed_out {
                            if suppress_on_failure {
                                if let Some(state) = me.states.get_mut(&chip_kind) {
                                    state.last_failure_fingerprint = current_fingerprint;
                                }
                            }
                            me.update_chip_value(&chip_kind, None);
                            me.set_chip_update_status(&chip_kind, ChipUpdateStatus::TimedOut);
                            return;
                        }

                        let (output, status, failed) = match &value {
                            Some(command_output)
                                if command_output.status == CommandExitStatus::Success =>
                            {
                                let output = command_output.to_string().ok().and_then(|mut s| {
                                    s.truncate(s.trim_end().len());
                                    if allow_empty_value || !s.is_empty() {
                                        Some(s)
                                    } else {
                                        None
                                    }
                                });
                                (output, ChipUpdateStatus::Ready, false)
                            }
                            _ => (None, ChipUpdateStatus::Error, true),
                        };

                        if suppress_on_failure && failed {
                            if let Some(state) = me.states.get_mut(&chip_kind) {
                                state.last_failure_fingerprint = current_fingerprint;
                            }
                        } else if suppress_on_failure {
                            if let Some(state) = me.states.get_mut(&chip_kind) {
                                if state.last_failure_fingerprint == current_fingerprint {
                                    state.last_failure_fingerprint = None;
                                }
                            }
                        }
                        let chip_value = output.map(ChipValue::Text);
                        me.update_chip_value(&chip_kind, chip_value);
                        me.set_chip_update_status(&chip_kind, status);
                    },
                );

                state.generator_handle = Some(handle);
            }
            PromptGenerator::Contextual { from_context_fn } => {
                self.set_chip_update_status(chip_kind, ChipUpdateStatus::Loading);
                let value = self.with_generator_context(ctx, from_context_fn);
                self.update_chip_value(chip_kind, value);
                self.set_chip_update_status(chip_kind, ChipUpdateStatus::Ready);
            }
        }

        if let Some(on_click_gen) = on_click_generator {
            self.refresh_on_click_values(chip_kind, on_click_gen, ctx);
        }
    }

    /// Run only the on-click generator for the given chip, updating the
    /// `last_on_click_values` in state when the command completes.
    fn refresh_on_click_values(
        &mut self,
        chip_kind: &ContextChipKind,
        on_click_generator: PromptGenerator,
        ctx: &mut ModelContext<Self>,
    ) {
        let PromptGenerator::ShellCommand(on_click_cmd) = on_click_generator else {
            return;
        };

        if !self
            .states
            .get(chip_kind)
            .is_some_and(|state| state.availability.is_enabled())
        {
            return;
        }

        let chip_kind = chip_kind.clone();
        let Some(exec_ctx) = self.prepare_shell_command_context(&on_click_cmd, ctx) else {
            return;
        };

        let Some(chip) = chip_kind.to_chip() else {
            return;
        };

        let Some(state) = self.states.get_mut(&chip_kind) else {
            log::warn!("Tried to run on-click generator for {chip_kind:?}, but state was missing");
            return;
        };

        if let Some(handle) = state.on_click_generator_handle.take() {
            handle.abort();
        }

        let timeout = chip.runtime_policy().shell_command_timeout();
        let chip_title = chip.title().to_owned();
        let logger = self.prompt_chip_logger.clone();
        let handle = ctx.spawn(
            async move {
                let (value, timed_out) = Self::execute_session_command_with_timeout(
                    exec_ctx.session.clone(),
                    exec_ctx.command.clone(),
                    exec_ctx.current_dir_path.clone(),
                    exec_ctx.environment_variables.clone(),
                    timeout,
                )
                .await;
                (value, timed_out, chip_kind, exec_ctx, chip_title)
            },
            move |me, (on_click_value, timed_out, chip_kind, exec_ctx, chip_title), _ctx| {
                logger.log_shell_command(&ChipCommandLogEntry {
                    chip_kind: &chip_kind,
                    chip_title: &chip_title,
                    phase: PromptChipExecutionPhase::OnClick,
                    shell_type: exec_ctx.shell_type,
                    working_directory: exec_ctx.current_dir_path.as_deref(),
                    command: &exec_ctx.command,
                    output: on_click_value.as_ref(),
                    timed_out,
                });

                if timed_out {
                    me.update_on_click_value(&chip_kind, None);
                    return;
                }

                let on_click_output = match on_click_value {
                    Some(command_output) if command_output.status == CommandExitStatus::Success => {
                        match command_output.to_string() {
                            Ok(string) => string
                                .split('\n')
                                .map(|s| s.trim().to_string())
                                .collect_vec(),
                            Err(_) => Vec::new(),
                        }
                    }
                    _ => Vec::new(),
                };

                me.update_on_click_value(&chip_kind, Some(on_click_output));
            },
        );

        state.on_click_generator_handle = Some(handle);
    }

    fn fetch_chip_value_at_interval(
        &mut self,
        chip_kind: &ContextChipKind,
        initial_value_generator: Option<PromptGenerator>,
        on_click_generator: Option<PromptGenerator>,
        allow_fingerprint_skip: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        // For periodically-updated chips, we have to check if context chips were disabled while
        // waiting on the timer. This protects against race conditions between aborting the
        // previous refresh handle and starting the next one.
        if !self.active(ctx) {
            return;
        }

        let Some(chip) = chip_kind.to_chip() else {
            report_error!("Undefined chip", extra: { "chip_kind" => ?chip_kind });
            return;
        };
        if let RefreshConfig::Periodically { interval } = chip.refresh_config() {
            let initial_value_generator =
                initial_value_generator.as_ref().unwrap_or(chip.generator());
            self.fetch_chip_value_once(
                chip_kind,
                initial_value_generator,
                on_click_generator.clone(),
                allow_fingerprint_skip,
                ctx,
            );
            let interval = *interval;
            let chip_kind_clone = chip_kind.clone();
            let future = ctx.spawn(
                async move {
                    Timer::after(interval).await;
                    chip_kind_clone
                },
                |me, chip_kind, ctx| {
                    me.fetch_chip_value_at_interval(&chip_kind, None, None, true, ctx);
                },
            );

            match self.states.get_mut(chip_kind) {
                Some(state) => state.refresh_handle = Some(future),
                None => log::warn!("Missing state for {chip_kind:?}"),
            }
        }
    }

    fn run_chips(&mut self, chips: Vec<ContextChipKind>, ctx: &mut ModelContext<Self>) {
        if !self.active(ctx) {
            log::debug!("Context chips are not in use, won't run");
            return;
        }

        chips.iter().for_each(|chip_kind| {
            let Some(chip) = chip_kind.to_chip() else {
                report_error!("Undefined chip", extra: { "chip_kind" => ?chip_kind });
                return;
            };
            // Add states of new chips
            if !self.states.contains_key(chip_kind) {
                let state = ChipState::new(chip_kind);
                self.states.insert(chip_kind.clone(), state);
            }
            if self.is_updated_externally(chip_kind) {
                // For chips updated externally (e.g. by the per-repo git status
                // filesystem watcher), avoid running the shell-based fallback
                // generator. Doing so can briefly overwrite the structured
                // watcher value with one that uses different semantics (for
                // example, the `GitDiffStats` shell fallback runs `git diff
                // --shortstat HEAD`, which excludes untracked files, whereas
                // the watcher counts untracked files as changes), causing the
                // chip to flicker between the tracked-only count and the
                // all-files count when untracked files are present.
                //
                // If a chip provides an `initial_value_generator` that sources
                // from the prompt context (rather than running a shell
                // command), use it for a fast initial value until the watcher
                // emits a metadata-changed event.
                if let Some(initial_gen) = chip_kind.initial_value_generator() {
                    self.fetch_chip_value_once(
                        chip_kind,
                        &initial_gen,
                        chip.on_click_generator().cloned(),
                        true,
                        ctx,
                    );
                } else {
                    // Externally-updated chips without an `initial_value_generator`
                    // are left blank after a state rebuild (`states.clear()` in
                    // `handle_prompt_changed`, or `clear_cache()` on a session
                    // change) until their backing model emits a change event.
                    // `GithubPullRequest` only emits when cached PR info actually
                    // changes, so after a rebuild there may be no event to restore
                    // the already-cached value until the next periodic refresh.
                    if matches!(chip_kind, ContextChipKind::GithubPullRequest) {
                        self.sync_pr_chip_from_model(ctx);
                    }
                }
                return;
            }

            match chip.refresh_config() {
                RefreshConfig::OnDemandOnly => {
                    self.fetch_chip_value_once(
                        chip_kind,
                        chip.generator(),
                        chip.on_click_generator().cloned(),
                        true,
                        ctx,
                    );
                }
                RefreshConfig::Periodically { .. } => {
                    self.fetch_chip_value_at_interval(
                        chip_kind,
                        chip_kind.initial_value_generator(),
                        chip.on_click_generator().cloned(),
                        true,
                        ctx,
                    );
                }
                RefreshConfig::OnFileChanges { filepath } => {
                    log::debug!("Unimplemented: would've watched changes to filepath: {filepath}");
                    // fall back to OnDemandOnly behavior instead
                    self.fetch_chip_value_once(
                        chip_kind,
                        chip.generator(),
                        chip.on_click_generator().cloned(),
                        true,
                        ctx,
                    );
                }
            };
        });
    }

    /// Reads the currently-configured chips from the [`Prompt`] model and filters out any that
    /// are missing their definition.
    fn configured_chips(&self, ctx: &AppContext) -> Vec<ContextChipKind> {
        let prompt = Prompt::as_ref(ctx);
        prompt
            .chip_kinds()
            .into_iter()
            .filter(|chip_kind| chip_kind.to_chip().is_some())
            .collect()
    }

    /// Chips whose values we should actively maintain in state.
    ///
    /// When Agent View is enabled, the footer chips should not depend on prompt chip
    /// customization/ordering/visibility, so we keep their backing values up to date even if they
    /// are not present in the prompt configuration.
    fn chips_to_run(&self, ctx: &AppContext) -> Vec<ContextChipKind> {
        let mut chips = self.configured_chips(ctx);

        if FeatureFlag::AgentView.is_enabled() {
            let footer_chips = SessionSettings::as_ref(ctx)
                .agent_footer_chip_selection
                .all_chips();
            for chip_kind in footer_chips {
                if !chips.contains(&chip_kind) {
                    chips.push(chip_kind);
                }
            }

            // Also include chips configured for the CLI agent footer.
            let cli_footer_chips = SessionSettings::as_ref(ctx)
                .cli_agent_footer_chip_selection
                .all_chips();
            for chip_kind in cli_footer_chips {
                if !chips.contains(&chip_kind) {
                    chips.push(chip_kind);
                }
            }
        }

        chips
    }

    /// Resets states (including terminating any in progress spawned operations), and updates the
    /// existing states map with new information.
    /// This is called when the context gets updated (ie. a new block metadata is received).
    fn update_states_with_new_context(&mut self, ctx: &mut ModelContext<Self>) {
        // 1. Terminating existing spawned operations.
        self.clear_chips();

        // 2. Running chips with new context
        self.run_chips(self.chips_to_run(ctx), ctx);
    }

    /// Resets states (including terminating any in progress spawned operations), and updates the
    /// existing states map with new information.
    /// This is called when the context gets updated (ie. a new block metadata is received).
    fn update_states_with_new_context_and_session(&mut self, ctx: &mut ModelContext<Self>) {
        // 1. Terminating existing spawned operations.
        self.clear_chips_and_cache();

        // 2. Running chips with new context
        self.run_chips(self.chips_to_run(ctx), ctx);
    }

    /// Handles prompt updates (ie. configuration changes).
    /// Removes states for chips that are no longer in use, and removes them; and for new chips -
    /// runs them. Note that existing chips don't need to run, because they're already in a good
    /// spot, and changing Prompt configuration most likely doesn't mean updating the context.
    fn handle_prompt_changed(
        &mut self,
        _: ModelHandle<Prompt>,
        _prompt_event: &<Prompt as Entity>::Event,
        ctx: &mut ModelContext<Self>,
    ) {
        self.states.clear();
        self.update_states_with_new_context(ctx);

        let prompt = Prompt::as_ref(ctx);
        self.separator = prompt.separator();

        // Always notify, so that if the prompt layout changed (reordering chips, for example),
        // we'll re-render the prompt, even if no individual chip contents changed.
        ctx.notify();
    }

    fn handle_session_settings_changed(
        &mut self,
        _: ModelHandle<SessionSettings>,
        event: &SessionSettingsChangedEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        if let SessionSettingsChangedEvent::HonorPS1 { .. } = event {
            if self.active(ctx) {
                // If switching from PS1 to context chips, we'll need to restart the chip-updating
                // loops. Any previous async updates will have been cancelled.
                log::debug!("Re-enabling context chips");
                self.update_states_with_new_context(ctx)
            } else {
                // If switching from context chips to PS1, stop any in-flight chip updates.
                log::debug!("Using PS1, disabling context chips");
                self.clear_chips_and_cache();
            }
        }

        if let SessionSettingsChangedEvent::SavedPrompt { .. } = event {
            let session_settings = SessionSettings::as_ref(ctx);

            self.same_line_prompt_enabled =
                session_settings.saved_prompt.same_line_prompt_enabled();
            self.separator = session_settings.saved_prompt.separator();
        }

        if let SessionSettingsChangedEvent::AgentToolbarChipSelectionSetting { .. } = event {
            // Recompute which chips to run when the agent footer config changes.
            self.update_states_with_new_context(ctx);
        }
        if let SessionSettingsChangedEvent::GithubPrChipDefaultValidation { .. } = event {
            // Re-resolve the default prompt's chip list (which gates the
            // PR chip on `is_suppressed()`) and re-run chips with the new
            // suppression state.
            self.update_states_with_new_context(ctx);
        }

        if let SessionSettingsChangedEvent::CLIAgentToolbarChipSelectionSetting { .. } = event {
            self.update_states_with_new_context(ctx);
        }
    }

    fn clear_chips(&mut self) {
        self.states
            .iter_mut()
            .for_each(|(_, state)| state.clear_abort_handlers());
        self.renderable_chips.clear();
    }

    /// Clear all context chip state and stop any in-progress updates.
    fn clear_chips_and_cache(&mut self) {
        self.clear_chips();
        self.states
            .iter_mut()
            .for_each(|(_, state)| state.clear_cache());
    }

    /// Waits for any in-progress asynchronous generators to finish.
    #[cfg(test)]
    pub fn await_generators(
        &self,
        ctx: &mut warpui::AppContext,
    ) -> futures_util::future::BoxFuture<'static, ()> {
        use futures_util::FutureExt;
        use itertools::Itertools;
        // This structure prevents the returned Future from referencing self.
        let chip_futures = self
            .states
            .values()
            .flat_map(|state| {
                [
                    state.generator_handle.as_ref(),
                    state.on_click_generator_handle.as_ref(),
                ]
            })
            .flatten()
            .map(|handle| ctx.await_spawned_future(handle.future_id()))
            .collect_vec();

        async move {
            for future in chip_futures {
                future.await;
            }
        }
        .boxed()
    }

    /// Whether or not any asynchronous generators are currently refreshing.
    #[cfg(test)]
    pub fn are_any_generators_running(&self) -> bool {
        self.states
            .values()
            .flat_map(|state| {
                [
                    state.generator_handle.as_ref(),
                    state.on_click_generator_handle.as_ref(),
                ]
            })
            .flatten()
            .any(|handle| !handle.abort_handle().is_aborted())
    }

    fn handle_model_event(
        &mut self,
        _: ModelHandle<ModelEventDispatcher>,
        event: &ModelEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        if let ModelEvent::AfterBlockCompleted(after_block_completed) = event {
            if let BlockType::User(UserBlockCompleted { command, .. }) =
                &after_block_completed.block_type
            {
                if let Some(cmd) = command.split_whitespace().next() {
                    // Resolve aliases so that e.g. `alias g=git` followed by `g push`
                    // still triggers invalidation for chips watching "git".
                    let resolved = self
                        .latest_context
                        .as_ref()
                        .and_then(|context| context.active_block_metadata.session_id())
                        .and_then(|session_id| self.sessions.as_ref(ctx).get(session_id))
                        .and_then(|session| session.alias_value(cmd).map(String::from));
                    let effective_cmd = resolved.as_deref().unwrap_or(cmd);

                    for (chip_kind, state) in &mut self.states {
                        if let Some(chip) = chip_kind.to_chip() {
                            if chip
                                .runtime_policy()
                                .invalidate_on_commands()
                                .iter()
                                .any(|c| c == effective_cmd)
                            {
                                state.invalidating_command_count += 1;
                            }
                        }
                    }
                }
            }
        }
    }

    /// Update the prompt context to reflect a new active block. This should be called from the
    /// parent terminal whenever a new set of block metadata is received.
    pub fn update_context(&mut self, active_block: &Block, ctx: &mut ModelContext<Self>) {
        let session_has_changed = match &self.latest_context {
            Some(ctx) => ctx.active_block_metadata.session_id() != active_block.session_id(),
            None => true,
        };
        self.latest_context = Some(PromptContext {
            active_block_metadata: active_block.metadata(),
            environment: Environment::from_block(active_block),
        });
        if session_has_changed {
            self.update_states_with_new_context_and_session(ctx);
        } else {
            self.update_states_with_new_context(ctx);
        }
    }

    /// Run a callback with the latest generator context.
    fn with_generator_context<F, C, R>(&self, ctx: &C, func: F) -> Option<R>
    where
        C: ModelAsRef,
        F: FnOnce(&GeneratorContext) -> Option<R>,
    {
        let current_context = self.latest_context.as_ref()?;
        let active_session = current_context
            .active_block_metadata
            .session_id()
            .and_then(|session_id| self.sessions.as_ref(ctx).get(session_id));

        let context = GeneratorContext {
            active_block_metadata: &current_context.active_block_metadata,
            active_session: active_session.as_deref(),
            current_environment: &current_context.environment,
        };
        func(&context)
    }

    /// Builds context menu items for copying individual context chips
    pub fn copy_menu_items(
        &self,
        position: PromptPosition,
        ctx: &AppContext,
    ) -> Vec<MenuItem<TerminalAction>> {
        Prompt::as_ref(ctx)
            .chip_kinds()
            .into_iter()
            .filter_map(|chip_kind| {
                let has_value = self
                    .states
                    .get(&chip_kind)
                    .is_some_and(|state| state.last_computed_value.is_some());
                if has_value && chip_kind.is_copyable() {
                    if let Some(chip) = chip_kind.to_chip() {
                        Some(
                            MenuItemFields::new(format!("Copy {}", chip.title()))
                                .with_on_select_action(TerminalAction::ContextMenu(
                                    ContextMenuAction::CopyPrompt {
                                        position,
                                        part: PromptPart::ContextChip(chip_kind),
                                    },
                                ))
                                .into_item(),
                        )
                    } else {
                        report_error!("Missing definition for chip", extra: { "chip_kind" => ?chip_kind });
                        None
                    }
                } else {
                    None
                }
            })
            .collect()
    }

    /// Gets the latest value of the given chip.
    pub fn latest_chip_value(&self, chip_kind: &ContextChipKind) -> Option<&ChipValue> {
        self.states
            .get(chip_kind)
            .and_then(|state| state.last_computed_value.as_ref())
    }

    /// Gets the latest chip data for the given chip kind, independent of prompt configuration.
    pub fn latest_chip_result(&self, chip_kind: &ContextChipKind) -> Option<ChipResult> {
        let state = self.states.get(chip_kind)?;
        if !state.should_render || matches!(state.availability, ChipAvailability::Hidden) {
            return None;
        }

        Some(ChipResult {
            kind: chip_kind.clone(),
            value: state.last_computed_value.clone(),
            on_click_values: state.last_on_click_values.clone().unwrap_or_default(),
        })
    }

    /// Serializes the current prompt as an unstyled string.
    pub fn prompt_as_string(&self, ctx: &AppContext) -> String {
        chips_to_string(
            Prompt::as_ref(ctx)
                .chip_kinds()
                .into_iter()
                .filter_map(|chip_kind| {
                    let value = &self.states.get(&chip_kind)?.last_computed_value;
                    let on_click_value = self.states.get(&chip_kind)?.last_on_click_values.clone();
                    let chip_result = ChipResult {
                        kind: chip_kind,
                        value: value.clone(),
                        on_click_values: on_click_value.unwrap_or_default(),
                    };
                    Some(chip_result)
                }),
        )
    }

    /// Set the per-repo git status model handle. When `Some`, subscribes to
    /// metadata events so git-backed prompt chips are updated from the
    /// per-repo status model. PR info is handled separately by
    /// [`Self::set_github_repo_model`].
    pub fn set_git_repo_status(
        &mut self,
        handle: Option<WeakModelHandle<GitRepoStatusModel>>,
        ctx: &mut ModelContext<Self>,
    ) {
        // Unsubscribe from the previous model, if any.
        if let Some(old_weak) = self.git_repo_status.take() {
            if let Some(old_strong) = old_weak.upgrade(ctx) {
                ctx.unsubscribe_from_model(&old_strong);
            }
        }

        // Repo detached, clear git chips that require repository metadata.
        if handle.is_none() {
            for chip_kind in [
                ContextChipKind::GitDiffStats,
                ContextChipKind::GitBranchStatus,
            ] {
                if let Some(state) = self.states.get_mut(&chip_kind) {
                    state.clear_abort_handlers();
                    state.clear_cache();
                }
            }
            let _ = self.update_tx.try_send(());
            return;
        }

        if let Some(weak) = handle {
            if let Some(strong) = weak.upgrade(ctx) {
                self.git_repo_status = Some(weak);
                ctx.subscribe_to_model(&strong, |me, _, event, ctx| match event {
                    GitRepoStatusEvent::MetadataChanged => {
                        me.apply_git_repo_metadata(ctx);
                    }
                });

                // Eagerly populate chips if metadata is already available (the
                // initial `refresh_metadata` in `GitRepoStatusModel::new` may
                // have completed before we subscribed). If it hasn't finished
                // yet, the subscription above will catch the `MetadataChanged`
                // event when it does.
                if strong.as_ref(ctx).metadata(ctx).is_some() {
                    self.apply_git_repo_metadata(ctx);
                }
            }
        }
    }

    /// Set the per-repo GitHub-info model handle. When `Some`, subscribes to
    /// its events so the `GithubPullRequest` chip value is updated.
    pub fn set_github_repo_model(
        &mut self,
        handle: Option<WeakModelHandle<GitHubRepoModel>>,
        ctx: &mut ModelContext<Self>,
    ) {
        // Unsubscribe from the previous model, if any.
        if let Some(old_weak) = self.github_repo_model.take() {
            if let Some(old_strong) = old_weak.upgrade(ctx) {
                ctx.unsubscribe_from_model(&old_strong);
            }
        }

        if handle.is_none() {
            // GitHub-info handle detached: clear any stale PR chip state.
            if let Some(state) = self.states.get_mut(&ContextChipKind::GithubPullRequest) {
                state.clear_abort_handlers();
                state.clear_cache();
            }
            let _ = self.update_tx.try_send(());
            return;
        }

        if let Some(weak) = handle {
            if let Some(strong) = weak.upgrade(ctx) {
                self.github_repo_model = Some(weak);
                // Only PR info drives the chip value; repository name/owner
                // changes don't affect it.
                ctx.subscribe_to_model(&strong, |me, _, event, ctx| match event {
                    GitHubRepoEvent::PrInfoChanged => {
                        me.sync_pr_chip_from_model(ctx);
                    }
                    GitHubRepoEvent::RepositoryInfoChanged => {}
                });

                // Eagerly populate the PR chip if PR info has already landed.
                self.sync_pr_chip_from_model(ctx);
            }
        }
    }

    /// Read the current `GitRepoStatusModel` metadata and push it into the
    /// git-backed chip states.
    fn apply_git_repo_metadata(&mut self, ctx: &mut ModelContext<Self>) {
        let metadata = self
            .git_repo_status
            .as_ref()
            .and_then(|w| w.upgrade(ctx))
            .and_then(|h| h.as_ref(ctx).metadata(ctx).cloned());

        let Some(metadata) = metadata else {
            return;
        };

        // Update ShellGitBranch.
        let new_branch = ChipValue::Text(metadata.current_branch_name.clone());
        let current_branch = self
            .latest_chip_value(&ContextChipKind::ShellGitBranch)
            .cloned();
        if current_branch.as_ref() != Some(&new_branch) {
            self.update_chip_value(&ContextChipKind::ShellGitBranch, Some(new_branch));
            // Refresh the branch dropdown so it stays in sync.
            let chip_kind = ContextChipKind::ShellGitBranch;
            if let Some(chip) = chip_kind.to_chip() {
                if let Some(on_click_gen) = chip.on_click_generator().cloned() {
                    self.refresh_on_click_values(&chip_kind, on_click_gen, ctx);
                }
            }
        }

        let new_branch_status = ChipValue::GitBranchStatus(metadata.branch_tracking_status.clone());
        let current_branch_status = self
            .latest_chip_value(&ContextChipKind::GitBranchStatus)
            .cloned();
        if current_branch_status.as_ref() != Some(&new_branch_status) {
            self.update_chip_value(&ContextChipKind::GitBranchStatus, Some(new_branch_status));
        }

        // Update GitDiffStats with structured data directly.
        let new_diff_stats = ChipValue::GitDiffStats(GitLineChanges::from_diff_stats(
            &metadata.stats_against_head,
        ));
        let current_diff_stats = self
            .latest_chip_value(&ContextChipKind::GitDiffStats)
            .cloned();
        if current_diff_stats.as_ref() != Some(&new_diff_stats) {
            self.update_chip_value(&ContextChipKind::GitDiffStats, Some(new_diff_stats));
        }
    }

    /// Reads PR info from the per-repo `GitHubRepoModel` and updates the
    /// `GithubPullRequest` chip value if it differs from the current one.
    fn sync_pr_chip_from_model(&mut self, ctx: &AppContext) {
        let new_pr_value = self
            .github_repo_model
            .as_ref()
            .and_then(|w| w.upgrade(ctx))
            .and_then(|h| {
                h.as_ref(ctx)
                    .pr_info(ctx)
                    .map(|info| ChipValue::Text(info.url.clone()))
            });
        let current_pr = self
            .latest_chip_value(&ContextChipKind::GithubPullRequest)
            .cloned();
        if current_pr != new_pr_value {
            self.update_chip_value(&ContextChipKind::GithubPullRequest, new_pr_value);
        }
    }

    /// Returns `true` when the given chip's value is updated externally
    /// (e.g. by a filesystem watcher) and the periodic timer should be skipped.
    fn is_updated_externally(&self, chip_kind: &ContextChipKind) -> bool {
        match chip_kind {
            ContextChipKind::ShellGitBranch
            | ContextChipKind::GitBranchStatus
            | ContextChipKind::GitDiffStats => self.git_repo_status.is_some(),
            ContextChipKind::GithubPullRequest => self.github_repo_model.is_some(),
            _ => false,
        }
    }

    /// Whether or not context chips are active. If this is false, we can skip running them.
    fn active(&self, ctx: &AppContext) -> bool {
        // Context chips are active when:
        // 1. PS1 is not honored (normal case), OR
        // 2. Universal developer input is enabled (overrides PS1 behavior), OR
        // 3. AgentView feature is enabled (agent view needs chips regardless of PS1)
        !*SessionSettings::as_ref(ctx).honor_ps1
            || InputSettings::as_ref(ctx).is_universal_developer_input_enabled(ctx)
            || FeatureFlag::AgentView.is_enabled()
    }
}

impl Entity for CurrentPrompt {
    type Event = ();
}

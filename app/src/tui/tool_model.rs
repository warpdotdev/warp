use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use ai::diff_validation::{AIRequestedCodeDiff, DiffType};
use anyhow::{anyhow, Result};
use chrono::Local;
use futures::future::BoxFuture;
use futures::FutureExt;
use warp_core::command::ExitCode;
use warp_terminal::model::BlockId;
use warpui::{AppContext, Entity, ModelContext, SingletonEntity};

use crate::ai::agent::conversation::AIConversationId;
use crate::ai::agent::{
    AIAgentAction, AIAgentActionId, AIAgentActionResult, AIAgentActionResultType,
    AIAgentActionType, AnyFileContent, FileContext, FileGlobResult, FileGlobV2Result, GrepResult,
    ReadFilesResult, ReadShellCommandOutputResult, RequestCommandOutputResult,
    RequestFileEditsResult, TransferShellCommandControlToUserResult, UpdatedFileContext,
    WriteToLongRunningShellCommandResult,
};
use crate::ai::blocklist::{
    apply_edits, ActionExecution, AgentToolActionModel, AgentToolExecutionContext,
    AgentToolExecutor, AnyActionExecution, ExecuteActionInput, FileReadResult,
    PreprocessActionInput, SessionContext, SurfaceSpecificToolExecutor,
};
use crate::ai::paths::host_native_absolute_path;
use crate::auth::AuthStateProvider;
use crate::terminal::model::session::{
    BootstrapSessionType, ExecuteCommandOptions, HostInfo, IsSSHWrapperSession,
    LocalCommandExecutor, Session, SessionInfo,
};
use crate::terminal::shell::{Shell, ShellLaunchData, ShellType};
use crate::AuthState;

/// Minimal card data for TUI tool rendering.
#[derive(Clone, Debug)]
pub(crate) struct TuiToolCard {
    pub action_id: AIAgentActionId,
    pub title: String,
    pub lines: Vec<String>,
}

/// Minimal TUI-owned wrapper around shared tool action state and execution.
pub(crate) struct TuiToolActionModel {
    tools: AgentToolActionModel,
    cards_by_conversation: HashMap<AIConversationId, Vec<TuiToolCard>>,
    session: Arc<Session>,
}

pub(crate) enum TuiToolActionEvent {
    Updated { conversation_id: AIConversationId },
    ActionsFinished { conversation_id: AIConversationId },
}

impl TuiToolActionModel {
    pub fn new(_: &mut ModelContext<Self>) -> Self {
        Self {
            tools: AgentToolActionModel::new(),
            cards_by_conversation: HashMap::new(),
            session: Arc::new(tui_local_session()),
        }
    }

    pub fn card_for_action(
        &self,
        conversation_id: AIConversationId,
        action_id: &AIAgentActionId,
    ) -> Option<&TuiToolCard> {
        self.cards_by_conversation
            .get(&conversation_id)
            .and_then(|cards| cards.iter().find(|card| &card.action_id == action_id))
    }

    pub fn drain_finished_results(
        &mut self,
        conversation_id: AIConversationId,
    ) -> Vec<AIAgentActionResult> {
        self.tools.drain_finished_results(conversation_id)
    }

    pub fn queue_actions(
        &mut self,
        actions: Vec<AIAgentAction>,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) {
        if actions.is_empty() {
            return;
        }
        self.tools.record_action_order(conversation_id, &actions);
        for action in &actions {
            self.tools
                .record_serial_running_action(conversation_id, action.id.clone());
        }

        self.cards_by_conversation
            .entry(conversation_id)
            .or_default()
            .extend(actions.iter().map(|action| TuiToolCard {
                action_id: action.id.clone(),
                title: action.action.user_friendly_name(),
                lines: vec!["queued".to_string()],
            }));
        ctx.emit(TuiToolActionEvent::Updated { conversation_id });

        for action in actions {
            self.start_action(action, conversation_id, ctx);
        }
    }

    /// Starts one queued TUI tool action through the shared executor.
    fn start_action(
        &mut self,
        action: AIAgentAction,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) {
        let input = ExecuteActionInput {
            action: &action,
            conversation_id,
        };
        let mut surface = TuiToolExecutor::new(self.session.clone(), ctx);
        let execution = AgentToolExecutor::execute_action(&mut surface, input, ctx);
        let action_id = action.id.clone();
        let task_id = action.task_id.clone();
        let action_type = action.action.clone();

        match execution {
            AnyActionExecution::Async {
                execute_future,
                on_complete,
            } => {
                ctx.spawn(execute_future, move |model, result, ctx| {
                    let result = AIAgentActionResult {
                        id: action_id,
                        task_id,
                        result: on_complete(result, ctx),
                    };
                    model.finish_action(conversation_id, action_type, result, ctx);
                });
            }
            AnyActionExecution::Sync(result) => {
                let result = AIAgentActionResult {
                    id: action_id,
                    task_id,
                    result,
                };
                self.finish_action(conversation_id, action_type, result, ctx);
            }
            AnyActionExecution::NotReady | AnyActionExecution::InvalidAction => {
                let result = AIAgentActionResult {
                    id: action_id,
                    task_id,
                    result: action_type.cancelled_result(),
                };
                self.finish_action(conversation_id, action_type, result, ctx);
            }
        }
    }

    /// Records a finished action result and emits follow-up readiness when the batch is complete.
    fn finish_action(
        &mut self,
        conversation_id: AIConversationId,
        action_type: AIAgentActionType,
        result: AIAgentActionResult,
        ctx: &mut ModelContext<Self>,
    ) {
        let card = card_for_result(result.id.clone(), &action_type, &result.result);
        self.tools
            .finish_running_action(conversation_id, &result.id);
        self.tools
            .push_finished_result(conversation_id, Arc::new(result));
        self.update_card(conversation_id, card);
        let is_done = !self.tools.has_running_actions(conversation_id);
        ctx.emit(TuiToolActionEvent::Updated { conversation_id });
        if is_done {
            ctx.emit(TuiToolActionEvent::ActionsFinished { conversation_id });
        }
    }

    /// Updates or inserts the rendered card for an action.
    fn update_card(&mut self, conversation_id: AIConversationId, card: TuiToolCard) {
        let cards = self
            .cards_by_conversation
            .entry(conversation_id)
            .or_default();
        if let Some(existing) = cards
            .iter_mut()
            .find(|existing| existing.action_id == card.action_id)
        {
            *existing = card;
        } else {
            cards.push(card);
        }
    }
}

impl Entity for TuiToolActionModel {
    type Event = TuiToolActionEvent;
}

impl SingletonEntity for TuiToolActionModel {}

struct TuiToolExecutor {
    session: Arc<Session>,
    current_working_directory: Option<String>,
    session_context: SessionContext,
    shell_type: ShellType,
    shell_launch_data: Option<ShellLaunchData>,
    background_executor: Arc<warpui::r#async::executor::Background>,
    auth_state: Arc<AuthState>,
}

impl TuiToolExecutor {
    /// Creates a surface-specific executor for one TUI action execution pass.
    fn new(session: Arc<Session>, ctx: &mut ModelContext<TuiToolActionModel>) -> Self {
        let current_working_directory = std::env::current_dir()
            .ok()
            .map(|path| path.to_string_lossy().to_string());
        Self {
            session,
            current_working_directory: current_working_directory.clone(),
            session_context: SessionContext::local(current_working_directory),
            shell_type: shell_type_from_env(),
            shell_launch_data: None,
            background_executor: ctx.background_executor(),
            auth_state: AuthStateProvider::as_ref(ctx).get().clone(),
        }
    }
}

impl SurfaceSpecificToolExecutor for TuiToolExecutor {
    type Context<'a> = ModelContext<'a, TuiToolActionModel>;

    fn tool_execution_context(&self, _ctx: &Self::Context<'_>) -> AgentToolExecutionContext {
        self.execution_context()
    }

    fn tool_execution_context_from_app(&self, _ctx: &AppContext) -> AgentToolExecutionContext {
        self.execution_context()
    }

    fn app_context<'a, 'b>(ctx: &'a Self::Context<'b>) -> &'a AppContext {
        ctx
    }

    fn preprocess_shell(
        &mut self,
        _input: PreprocessActionInput<'_>,
        _ctx: &mut Self::Context<'_>,
    ) -> BoxFuture<'static, ()> {
        futures::future::ready(()).boxed()
    }

    fn execute_shell(
        &mut self,
        input: ExecuteActionInput<'_>,
        _ctx: &mut Self::Context<'_>,
    ) -> AnyActionExecution {
        match &input.action.action {
            AIAgentActionType::RequestCommandOutput {
                command,
                wait_until_completion,
                uses_pager,
                ..
            } => {
                let command = command.clone();
                let current_working_directory = self.current_working_directory.clone();
                let shell_type = self.shell_type;
                let session = self.session.clone();
                let wait_until_completion = *wait_until_completion;
                let uses_pager = *uses_pager;
                ActionExecution::new_async(
                    async move {
                        execute_command(
                            command,
                            wait_until_completion,
                            uses_pager,
                            current_working_directory,
                            shell_type,
                            session,
                        )
                        .await
                    },
                    |result, _ctx| {
                        AIAgentActionResultType::RequestCommandOutput(result.unwrap_or_else(
                            |error| RequestCommandOutputResult::Completed {
                                block_id: BlockId::new(),
                                command: "<failed>".to_string(),
                                output: format!("{error:#}"),
                                exit_code: ExitCode::from(1),
                                start_ts: Some(Local::now()),
                                completed_ts: Some(Local::now()),
                            },
                        ))
                    },
                )
                .into()
            }
            AIAgentActionType::ReadShellCommandOutput { .. } => {
                ActionExecution::<()>::Sync(AIAgentActionResultType::ReadShellCommandOutput(
                    ReadShellCommandOutputResult::Error(
                        crate::ai::agent::ShellCommandError::BlockNotFound,
                    ),
                ))
                .into()
            }
            AIAgentActionType::WriteToLongRunningShellCommand { .. } => {
                ActionExecution::<()>::Sync(
                    AIAgentActionResultType::WriteToLongRunningShellCommand(
                        WriteToLongRunningShellCommandResult::Error(
                            crate::ai::agent::ShellCommandError::BlockNotFound,
                        ),
                    ),
                )
                .into()
            }
            AIAgentActionType::TransferShellCommandControlToUser { .. } => {
                ActionExecution::<()>::Sync(
                    AIAgentActionResultType::TransferShellCommandControlToUser(
                        TransferShellCommandControlToUserResult::Error(
                            crate::ai::agent::ShellCommandError::BlockNotFound,
                        ),
                    ),
                )
                .into()
            }
            _ => ActionExecution::<()>::InvalidAction.into(),
        }
    }

    fn should_autoexecute_shell(
        &mut self,
        _input: ExecuteActionInput<'_>,
        _ctx: &mut Self::Context<'_>,
    ) -> bool {
        true
    }

    fn preprocess_file_edits(
        &mut self,
        _input: PreprocessActionInput<'_>,
        _ctx: &mut Self::Context<'_>,
    ) -> BoxFuture<'static, ()> {
        futures::future::ready(()).boxed()
    }

    fn execute_file_edits(
        &mut self,
        input: ExecuteActionInput<'_>,
        _ctx: &mut Self::Context<'_>,
    ) -> AnyActionExecution {
        let AIAgentActionType::RequestFileEdits { file_edits, .. } = &input.action.action else {
            return ActionExecution::<()>::InvalidAction.into();
        };
        let file_edits = file_edits.clone();
        let session_context = self.session_context.clone();
        let background_executor = self.background_executor.clone();
        let auth_state = self.auth_state.clone();
        ActionExecution::new_async(
            async move {
                execute_file_edits(file_edits, session_context, background_executor, auth_state)
                    .await
            },
            |result, _ctx| {
                AIAgentActionResultType::RequestFileEdits(result.unwrap_or_else(|error| {
                    RequestFileEditsResult::DiffApplicationFailed {
                        error: format!("{error:#}"),
                    }
                }))
            },
        )
        .into()
    }

    fn should_autoexecute_file_edits(
        &mut self,
        _input: ExecuteActionInput<'_>,
        _ctx: &mut Self::Context<'_>,
    ) -> bool {
        true
    }
}

impl TuiToolExecutor {
    /// Builds the shared execution context for surface-neutral tools.
    fn execution_context(&self) -> AgentToolExecutionContext {
        AgentToolExecutionContext {
            current_working_directory: self.current_working_directory.clone(),
            shell_launch_data: self.shell_launch_data.clone(),
            session: Some(self.session.clone()),
            terminal_view_id: None,
        }
    }
}

/// Executes a TUI shell command through the local session.
async fn execute_command(
    command: String,
    wait_until_completion: bool,
    uses_pager: Option<bool>,
    current_working_directory: Option<String>,
    shell_type: ShellType,
    session: Arc<Session>,
) -> Result<RequestCommandOutputResult> {
    let command = if uses_pager == Some(true) && wait_until_completion {
        decorate_pager_command(&command, shell_type)
    } else {
        command
    };
    let block_id = BlockId::new();
    let start_ts = Local::now();
    let output = session
        .execute_command(
            &command,
            current_working_directory.as_deref(),
            None,
            ExecuteCommandOptions::default(),
        )
        .await?;
    let completed_ts = Local::now();
    let mut combined = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !stderr.is_empty() {
        if !combined.is_empty() && !combined.ends_with('\n') {
            combined.push('\n');
        }
        combined.push_str(&stderr);
    }
    Ok(RequestCommandOutputResult::Completed {
        block_id,
        command,
        output: combined,
        exit_code: output.exit_code().unwrap_or_else(|| ExitCode::from(1)),
        start_ts: Some(start_ts),
        completed_ts: Some(completed_ts),
    })
}

/// Applies and saves TUI file edits automatically for v0.
async fn execute_file_edits(
    file_edits: Vec<crate::ai::agent::FileEdit>,
    session_context: SessionContext,
    background_executor: Arc<warpui::r#async::executor::Background>,
    auth_state: Arc<AuthState>,
) -> Result<RequestFileEditsResult> {
    let diffs = apply_edits(
        file_edits,
        &session_context,
        &Default::default(),
        background_executor,
        auth_state,
        false,
        |path| async move { FileReadResult::from(std::fs::read_to_string(path)) },
    )
    .await
    .map_err(|errors| {
        anyhow!(errors
            .iter()
            .map(|error| format!("{error:?}"))
            .collect::<Vec<_>>()
            .join("\n"))
    })?;

    apply_requested_diffs(diffs, &session_context).await
}

/// Saves requested code diffs and builds the action result payload.
async fn apply_requested_diffs(
    diffs: Vec<AIRequestedCodeDiff>,
    session_context: &SessionContext,
) -> Result<RequestFileEditsResult> {
    let mut unified = String::new();
    let mut updated_files = Vec::new();
    let mut deleted_files = Vec::new();
    let mut lines_added = 0usize;
    let mut lines_removed = 0usize;

    for diff in diffs {
        let absolute_path = host_native_absolute_path(
            &diff.file_name,
            session_context.shell(),
            session_context.current_working_directory(),
        );
        let path = PathBuf::from(&absolute_path);
        let (new_content, deleted, added, removed) = apply_diff_to_content(&diff)?;
        lines_added += added;
        lines_removed += removed;
        unified.push_str(&format!(
            "--- {}\n+++ {}\n@@\n{}\n",
            diff.file_name, diff.file_name, new_content
        ));
        if deleted {
            if path.exists() {
                async_fs::remove_file(&path).await?;
            }
            deleted_files.push(diff.file_name);
            continue;
        }
        if let Some(parent) = path.parent() {
            async_fs::create_dir_all(parent).await?;
        }
        async_fs::write(&path, new_content.as_bytes()).await?;
        updated_files.push(UpdatedFileContext {
            was_edited_by_user: false,
            file_context: FileContext::new(
                diff.file_name,
                AnyFileContent::StringContent(new_content),
                None,
                None,
            ),
        });
    }

    Ok(RequestFileEditsResult::Success {
        diff: unified,
        updated_files,
        deleted_files,
        lines_added,
        lines_removed,
    })
}

/// Applies one requested diff to its original content.
fn apply_diff_to_content(diff: &AIRequestedCodeDiff) -> Result<(String, bool, usize, usize)> {
    match &diff.diff_type {
        DiffType::Create { delta } => Ok((
            delta.insertion.clone(),
            false,
            delta.insertion.lines().count(),
            0,
        )),
        DiffType::Delete { delta } => {
            Ok((String::new(), true, 0, delta.replacement_line_range.len()))
        }
        DiffType::Update { deltas, .. } => {
            let had_trailing_newline = diff.original_content.ends_with('\n');
            let mut lines = diff
                .original_content
                .lines()
                .map(str::to_string)
                .collect::<Vec<_>>();
            let mut added = 0usize;
            let mut removed = 0usize;
            for delta in deltas.iter().rev() {
                let start = delta
                    .replacement_line_range
                    .start
                    .saturating_sub(1)
                    .min(lines.len());
                let end = delta
                    .replacement_line_range
                    .end
                    .saturating_sub(1)
                    .min(lines.len());
                let replacement = delta
                    .insertion
                    .lines()
                    .map(str::to_string)
                    .collect::<Vec<_>>();
                added += replacement.len();
                removed += end.saturating_sub(start);
                lines.splice(start..end, replacement);
            }
            let mut content = lines.join("\n");
            if had_trailing_newline {
                content.push('\n');
            }
            Ok((content, false, added, removed))
        }
    }
}

/// Builds a concise TUI card from a tool result.
fn card_for_result(
    action_id: AIAgentActionId,
    action: &AIAgentActionType,
    result: &AIAgentActionResultType,
) -> TuiToolCard {
    let mut lines = Vec::new();
    match result {
        AIAgentActionResultType::RequestCommandOutput(RequestCommandOutputResult::Completed {
            command,
            output,
            exit_code,
            ..
        }) => {
            lines.push(command.clone());
            lines.push(format!(
                "exit {} · {} lines captured",
                exit_code.value(),
                output.lines().count()
            ));
        }
        AIAgentActionResultType::RequestFileEdits(RequestFileEditsResult::Success {
            updated_files,
            lines_added,
            lines_removed,
            ..
        }) => {
            lines.push(
                updated_files
                    .iter()
                    .map(|file| file.file_context.file_name.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
            );
            lines.push(format!(
                "+{lines_added} -{lines_removed} · applied automatically"
            ));
        }
        AIAgentActionResultType::ReadFiles(ReadFilesResult::Success { files }) => {
            lines.push(format!("read {} file(s)", files.len()));
        }
        AIAgentActionResultType::ReadFiles(ReadFilesResult::Error(e)) => {
            lines.push(format!("error: {}", e.lines().next().unwrap_or("unknown")));
        }
        AIAgentActionResultType::ReadFiles(ReadFilesResult::Cancelled) => {
            lines.push("cancelled".to_string());
        }
        AIAgentActionResultType::Grep(GrepResult::Success { matched_files }) => {
            lines.push(format!("{} file(s) matched", matched_files.len()));
        }
        AIAgentActionResultType::Grep(GrepResult::Error(e)) => {
            lines.push(format!("error: {}", e.lines().next().unwrap_or("unknown")));
        }
        AIAgentActionResultType::Grep(GrepResult::Cancelled) => {
            lines.push("cancelled".to_string());
        }
        AIAgentActionResultType::FileGlob(FileGlobResult::Success { matched_files }) => {
            let count = if matched_files.trim().is_empty() {
                0
            } else {
                matched_files.lines().count()
            };
            lines.push(format!("{count} file(s) matched"));
        }
        AIAgentActionResultType::FileGlob(FileGlobResult::Error(e)) => {
            lines.push(format!("error: {}", e.lines().next().unwrap_or("unknown")));
        }
        AIAgentActionResultType::FileGlob(FileGlobResult::Cancelled) => {
            lines.push("cancelled".to_string());
        }
        AIAgentActionResultType::FileGlobV2(FileGlobV2Result::Success {
            matched_files, ..
        }) => {
            lines.push(format!("{} file(s) matched", matched_files.len()));
        }
        AIAgentActionResultType::FileGlobV2(FileGlobV2Result::Error(e)) => {
            lines.push(format!("error: {}", e.lines().next().unwrap_or("unknown")));
        }
        AIAgentActionResultType::FileGlobV2(FileGlobV2Result::Cancelled) => {
            lines.push("cancelled".to_string());
        }
        other => {
            lines.push(other.to_string());
        }
    }
    TuiToolCard {
        action_id,
        title: format!("Tool: {}", action.user_friendly_name()),
        lines,
    }
}

/// Decorates pager commands so they do not take over the terminal UI.
fn decorate_pager_command(command: &str, shell_type: ShellType) -> String {
    match shell_type {
        ShellType::Zsh | ShellType::Bash => format!("({command}) | command cat"),
        ShellType::Fish => format!("begin; {command}; end | command cat"),
        ShellType::PowerShell => format!("({command}) | \\Out-Host"),
    }
}

/// Detects the user's shell type from the process environment.
fn shell_type_from_env() -> ShellType {
    std::env::var("SHELL")
        .ok()
        .as_deref()
        .and_then(ShellType::from_name)
        .unwrap_or(ShellType::Zsh)
}

/// Creates the single local session used by the v0 TUI.
fn tui_local_session() -> Session {
    let shell_path = std::env::var("SHELL").ok();
    let shell_type = shell_type_from_env();
    let command_executor = Arc::new(LocalCommandExecutor::new(
        shell_path.as_ref().map(PathBuf::from),
        shell_type,
    ));
    Session::new(
        SessionInfo {
            session_id: 0.into(),
            shell: Shell::new(shell_type, None, None, Default::default(), shell_path),
            launch_data: None,
            histfile: None,
            user: "local:user".to_owned(),
            hostname: "local:host".to_owned(),
            subshell_info: None,
            path: std::env::var("PATH").ok(),
            environment_variable_names: Default::default(),
            aliases: Default::default(),
            abbreviations: Default::default(),
            function_names: Default::default(),
            builtins: Default::default(),
            keywords: Default::default(),
            is_ssh_wrapper_session: IsSSHWrapperSession::No,
            home_dir: dirs::home_dir().map(|path| path.to_string_lossy().to_string()),
            cdpath: None,
            editor: None,
            session_type: BootstrapSessionType::Local,
            host_info: HostInfo {
                os_category: Some(std::env::consts::OS.to_string()),
                linux_distribution: None,
            },
            wsl_name: None,
            spawning_session_id: None,
        },
        command_executor,
    )
}

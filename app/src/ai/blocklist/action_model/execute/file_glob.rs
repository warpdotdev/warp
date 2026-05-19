use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use futures::future::BoxFuture;
use futures::FutureExt;
use itertools::Itertools;
use warpui::r#async::FutureExt as AsyncFutureExt;
use warpui::{AppContext, Entity, EntityId, ModelContext, ModelHandle, SingletonEntity};

use crate::ai::agent::{
    conversation::AIConversationId, AIAgentAction, AIAgentActionType, FileGlobResult,
    FileGlobV2Match, FileGlobV2Result,
};
use crate::ai::blocklist::BlocklistAIPermissions;
use crate::ai::paths::{host_native_absolute_path, join_paths, shell_native_absolute_path};
use crate::terminal::model::session::ExecuteCommandOptions;
use crate::{
    ai::agent::AIAgentActionResultType,
    send_telemetry_from_app_ctx,
    terminal::{
        model::session::active_session::ActiveSession, model::session::Session, shell::ShellType,
        ShellLaunchData,
    },
    TelemetryEvent,
};
use warp_core::features::FeatureFlag;

const FILE_GLOB_TIMEOUT: Duration = Duration::from_secs(10);

use super::{
    get_server_output_id, is_git_repository, ActionExecution, AnyActionExecution,
    ExecuteActionInput, PreprocessActionInput,
};

/// Builds the `git ls-files` command emitted by [`run_git_ls_files_command`].
/// Both `target_path` and `patterns` are agent-controlled; without shell
/// escaping a pattern containing `'` could close the surrounding single-quote
/// pair and append a second command. Each joined path is escaped with
/// `ShellFamily::escape` (no tilde preservation, since these are pathspecs
/// rather than user paths) for the session's shell family — `git ls-files`
/// runs under PowerShell too, so we cannot hard-code POSIX backslash escapes.
pub(crate) fn build_git_ls_files_command(
    patterns: &[String],
    target_path: &str,
    shell_launch_data: Option<&ShellLaunchData>,
    shell_type: ShellType,
) -> String {
    let shell_family = warp_util::path::ShellFamily::from(shell_type);
    let pattern_args = patterns
        .iter()
        .flat_map(|pattern| {
            [
                // Matches on files in the target path.
                join_paths(&[target_path, pattern], shell_launch_data),
                // Matches on files in any subdirectory of the target path.
                join_paths(&[target_path, "*", pattern], shell_launch_data),
            ]
        })
        .map(|joined| shell_family.escape(&joined).into_owned())
        .join(" ");
    format!("git ls-files -c -o --exclude-standard -- {pattern_args}")
}

/// Builds the `find` command emitted by [`run_find_command`]. See
/// [`build_git_ls_files_command`] for the threat model.
pub(crate) fn build_find_command(patterns: &[String], target_path: &str) -> String {
    let shell_family = warp_util::path::ShellFamily::Posix;
    // Build a find command with -name for each pattern. Each pattern is
    // shell-escaped to neutralise embedded `'`, `$(...)`, backticks, etc.
    let pattern_args = patterns
        .iter()
        .map(|pattern| format!(" -name {}", shell_family.escape(pattern)))
        .join(" -o");
    let escaped_target = shell_family.shell_escape(target_path);
    format!("find {escaped_target} -type f {pattern_args}")
}

/// Builds the PowerShell `Get-ChildItem` command emitted by
/// [`run_powershell_get_childitem_command`]. See
/// [`build_git_ls_files_command`] for the threat model.
pub(crate) fn build_powershell_get_childitem_command(
    patterns: &[String],
    target_path: &str,
) -> String {
    let shell_family = warp_util::path::ShellFamily::PowerShell;
    let pattern_args = patterns
        .iter()
        .map(|pattern| shell_family.escape(pattern))
        .collect::<Vec<_>>()
        .join(",");
    let escaped_target = shell_family.shell_escape(target_path);
    format!(
        "Get-ChildItem -File -Recurse -Include {pattern_args} -Path {escaped_target} | ForEach-Object {{ $_.FullName }}"
    )
}

pub struct FileGlobExecutor {
    active_session: ModelHandle<ActiveSession>,
    terminal_view_id: EntityId,
}

fn log_file_glob_error(conversation_id: AIConversationId, ctx: &mut AppContext) {
    let server_output_id = get_server_output_id(conversation_id, ctx);
    send_telemetry_from_app_ctx!(TelemetryEvent::FileGlobToolFailed { server_output_id }, ctx);
}

impl FileGlobExecutor {
    pub fn new(active_session: ModelHandle<ActiveSession>, terminal_view_id: EntityId) -> Self {
        Self {
            active_session,
            terminal_view_id,
        }
    }

    pub(super) fn should_autoexecute(
        &self,
        input: ExecuteActionInput,
        ctx: &mut ModelContext<Self>,
    ) -> bool {
        let ExecuteActionInput {
            action:
                AIAgentAction {
                    action:
                        AIAgentActionType::FileGlob { path, .. }
                        | AIAgentActionType::FileGlobV2 {
                            search_dir: path, ..
                        },
                    ..
                },
            conversation_id,
        } = input
        else {
            return false;
        };

        // If the path is not provided, use the current working directory.
        let path = path.clone().unwrap_or_else(|| ".".to_string());

        let current_working_directory = self
            .active_session
            .as_ref(ctx)
            .current_working_directory()
            .cloned();
        let shell = self.active_session.as_ref(ctx).shell_launch_data(ctx);
        let absolute_path =
            host_native_absolute_path(path.as_str(), &shell, &current_working_directory);

        BlocklistAIPermissions::as_ref(ctx)
            .can_read_files_with_conversation(
                &conversation_id,
                vec![PathBuf::from(absolute_path)],
                Some(self.terminal_view_id),
                ctx,
            )
            .is_allowed()
    }

    pub(super) fn execute(
        &mut self,
        input: ExecuteActionInput,
        ctx: &mut ModelContext<Self>,
    ) -> impl Into<AnyActionExecution> {
        let AIAgentAction {
            action:
                AIAgentActionType::FileGlob { patterns, path }
                | AIAgentActionType::FileGlobV2 {
                    patterns,
                    search_dir: path,
                },
            ..
        } = input.action
        else {
            return ActionExecution::InvalidAction;
        };

        // If the path is not provided, use the current working directory.
        let path = path.clone().unwrap_or_else(|| ".".to_string());

        let shell_launch_data = self.active_session.as_ref(ctx).shell_launch_data(ctx);
        let current_working_directory = self
            .active_session
            .as_ref(ctx)
            .current_working_directory()
            .cloned();
        let absolute_path = shell_native_absolute_path(
            path.as_str(),
            shell_launch_data.as_ref(),
            current_working_directory.as_ref(),
        );

        let session = self.active_session.as_ref(ctx).session(ctx);

        let patterns_clone = patterns.clone();
        let conversation_id_clone = input.conversation_id;
        let is_file_glob_v2 = is_file_glob_v2(&input);
        ActionExecution::new_async(
            async move {
                match run_file_glob(patterns_clone, absolute_path, session, shell_launch_data)
                    .with_timeout(FILE_GLOB_TIMEOUT)
                    .await
                {
                    Ok(result) => result,
                    Err(_) => Err(anyhow::anyhow!("File glob operation timed out")),
                }
            },
            move |result, ctx| match result {
                Ok(file_glob_result) => {
                    match file_glob_result {
                        FileGlobV2Result::Error(ref e) => {
                            log::warn!("Executing file_glob resulted in error: {e:?}");
                            log_file_glob_error(conversation_id_clone, ctx);
                        }
                        FileGlobV2Result::Success { .. } => {
                            send_telemetry_from_app_ctx!(
                                TelemetryEvent::FileGlobToolSucceeded,
                                ctx
                            );
                        }
                        _ => {}
                    }
                    // Convert FileGlobV2Result to FileGlobResult if the request was not V2.
                    if is_file_glob_v2 {
                        AIAgentActionResultType::FileGlobV2(file_glob_result)
                    } else {
                        AIAgentActionResultType::FileGlob(file_glob_result.into())
                    }
                }
                Err(e) => {
                    log::warn!("Failed to execute file_glob: {e:?}");
                    log_file_glob_error(conversation_id_clone, ctx);
                    if is_file_glob_v2 {
                        AIAgentActionResultType::FileGlobV2(FileGlobV2Result::Error(e.to_string()))
                    } else {
                        AIAgentActionResultType::FileGlob(FileGlobResult::Error(e.to_string()))
                    }
                }
            },
        )
    }

    pub(super) fn preprocess_action(
        &mut self,
        _action: PreprocessActionInput,
        _ctx: &mut ModelContext<Self>,
    ) -> BoxFuture<'static, ()> {
        futures::future::ready(()).boxed()
    }

    pub(super) fn can_execute_in_parallel(&self, ctx: &AppContext) -> bool {
        self.active_session
            .as_ref(ctx)
            .session(ctx)
            .is_some_and(|session| session.supports_parallel_command_execution())
    }
}

fn is_file_glob_v2(input: &ExecuteActionInput) -> bool {
    matches!(input.action.action, AIAgentActionType::FileGlobV2 { .. })
}

async fn run_file_glob(
    patterns: Vec<String>,
    absolute_path: String,
    session: Option<Arc<Session>>,
    shell_launch_data: Option<ShellLaunchData>,
) -> anyhow::Result<FileGlobV2Result> {
    if patterns.is_empty() {
        return Err(anyhow::anyhow!("No patterns provided to file_glob"));
    }
    let Some(session) = session else {
        return Err(anyhow::anyhow!("No session provided to file_glob"));
    };

    let is_in_git_repo = is_git_repository(&absolute_path, session.as_ref())
        .await
        .unwrap_or_else(|e| {
            log::error!("Failed to run command to check if in git repository: {e:?}");
            false
        });

    if is_in_git_repo {
        run_git_ls_files_command(
            &patterns,
            &absolute_path,
            session.as_ref(),
            shell_launch_data,
            session.shell().shell_type(),
        )
        .await
    } else if session.shell().shell_type() == ShellType::PowerShell {
        run_powershell_get_childitem_command(&patterns, &absolute_path, session.as_ref()).await
    } else {
        run_find_command(&patterns, &absolute_path, session.as_ref()).await
    }
}

/// Uses git ls-files to list all files in a git repository and filters them by pattern.
async fn run_git_ls_files_command(
    patterns: &[String],
    target_path: &str,
    session: &Session,
    shell_launch_data: Option<ShellLaunchData>,
    shell_type: ShellType,
) -> anyhow::Result<FileGlobV2Result> {
    let command =
        build_git_ls_files_command(patterns, target_path, shell_launch_data.as_ref(), shell_type);

    let command_output = session
        .execute_command(
            command.as_str(),
            Some(target_path),
            None,
            ExecuteCommandOptions::default(),
        )
        .await?;
    let output = String::from_utf8_lossy(command_output.output()).to_string();

    if command_output.success() {
        // git ls-files outputs paths relative to the current directory. For consistency with the
        // `find` and PowerShell implementations, convert to absolute paths.
        let absolute_paths = non_empty_lines(&output)
            .map(|relative_path| {
                join_paths(&[target_path, relative_path], shell_launch_data.as_ref())
            })
            .map(|path| FileGlobV2Match { file_path: path });

        Ok(FileGlobV2Result::Success {
            matched_files: absolute_paths.collect(),
            warnings: None,
        })
    } else {
        Err(anyhow::anyhow!(output))
    }
}

/// Uses the find command for Unix-like environments to find files matching patterns.
async fn run_find_command(
    patterns: &[String],
    target_path: &str,
    session: &Session,
) -> anyhow::Result<FileGlobV2Result> {
    let find_command = build_find_command(patterns, target_path);

    let command_output = session
        .execute_command(
            find_command.as_str(),
            Some(target_path),
            None,
            ExecuteCommandOptions::default(),
        )
        .await?;
    let stdout = String::from_utf8_lossy(&command_output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&command_output.stderr).to_string();

    let has_results = FeatureFlag::FileGlobV2Warnings.is_enabled() && !stdout.trim().is_empty();
    if command_output.success() || has_results {
        let files = non_empty_lines(&stdout).map(|line| FileGlobV2Match {
            file_path: line.to_string(),
        });
        let warnings = if FeatureFlag::FileGlobV2Warnings.is_enabled() && !stderr.trim().is_empty()
        {
            Some(stderr)
        } else {
            None
        };
        Ok(FileGlobV2Result::Success {
            matched_files: files.collect(),
            warnings,
        })
    } else {
        Err(anyhow::anyhow!(stderr))
    }
}

/// Uses PowerShell's Get-ChildItem to find files matching patterns.
async fn run_powershell_get_childitem_command(
    patterns: &[String],
    target_path: &str,
    session: &Session,
) -> anyhow::Result<FileGlobV2Result> {
    let command = build_powershell_get_childitem_command(patterns, target_path);

    let command_output = session
        .execute_command(
            command.as_str(),
            Some(target_path),
            None,
            ExecuteCommandOptions::default(),
        )
        .await?;
    let output = String::from_utf8_lossy(command_output.output()).to_string();

    if command_output.success() {
        let files = non_empty_lines(&output).map(|line| FileGlobV2Match {
            file_path: line.to_string(),
        });
        Ok(FileGlobV2Result::Success {
            matched_files: files.collect(),
            warnings: None,
        })
    } else {
        Err(anyhow::anyhow!(output))
    }
}

fn non_empty_lines(str: &str) -> impl Iterator<Item = &str> {
    str.lines().filter(|line| !line.is_empty())
}

impl Entity for FileGlobExecutor {
    type Event = ();
}

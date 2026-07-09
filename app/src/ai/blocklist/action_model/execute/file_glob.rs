use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use futures::future::BoxFuture;
use futures::FutureExt;
use itertools::Itertools;
use warp_core::features::FeatureFlag;
use warpui::r#async::FutureExt as AsyncFutureExt;
use warpui::{AppContext, Entity, EntityId, ModelContext, ModelHandle, SingletonEntity};

use crate::ai::agent::conversation::AIConversationId;
use crate::ai::agent::{
    AIAgentAction, AIAgentActionResultType, AIAgentActionType, FileGlobResult, FileGlobV2Match,
    FileGlobV2Result,
};
use crate::ai::blocklist::BlocklistAIPermissions;
use crate::ai::paths::{host_native_absolute_path, join_paths, shell_native_absolute_path};
use crate::terminal::model::session::active_session::ActiveSession;
use crate::terminal::model::session::command_executor::shell_quote_arg;
use crate::terminal::model::session::{ExecuteCommandOptions, Session};
use crate::terminal::shell::ShellType;
use crate::terminal::ShellLaunchData;
use crate::{send_telemetry_from_app_ctx, TelemetryEvent};

const FILE_GLOB_TIMEOUT: Duration = Duration::from_secs(10);

use warp_errors::report_error;

use super::{
    get_server_output_id, is_git_repository, ActionExecution, AnyActionExecution,
    ExecuteActionInput, PreprocessActionInput,
};

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
    let shell_type = session.shell().shell_type();

    let is_in_git_repo = is_git_repository(&absolute_path, session.as_ref())
        .await
        .unwrap_or_else(|e| {
            report_error!(e.context("Failed to run command to check if in git repository"));
            false
        });

    if is_in_git_repo {
        run_git_ls_files_command(
            &patterns,
            &absolute_path,
            session.as_ref(),
            shell_launch_data,
            shell_type,
        )
        .await
    } else if shell_type == ShellType::PowerShell {
        run_powershell_get_childitem_command(&patterns, &absolute_path, session.as_ref()).await
    } else {
        run_find_command(&patterns, &absolute_path, session.as_ref(), shell_type).await
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
    let command = build_git_ls_files_command(
        patterns,
        target_path,
        shell_launch_data.as_ref(),
        shell_type,
    );

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
    shell_type: ShellType,
) -> anyhow::Result<FileGlobV2Result> {
    let find_command = build_find_command(patterns, target_path, shell_type);

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

fn build_git_ls_files_command(
    patterns: &[String],
    target_path: &str,
    shell_launch_data: Option<&ShellLaunchData>,
    shell_type: ShellType,
) -> String {
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
        // Patterns are model-controlled action input. Quote after joining with
        // the target path so metacharacters stay inside the git pathspec.
        .map(|pattern| shell_quote_arg(&pattern, shell_type))
        .join(" ");
    format!("git ls-files -c -o --exclude-standard -- {pattern_args}")
}

fn build_find_command(patterns: &[String], target_path: &str, shell_type: ShellType) -> String {
    // Preserve the existing `find` expression while making each model-provided
    // pattern a quoted `-name` argument instead of shell syntax.
    let pattern_args = patterns
        .iter()
        .map(|pattern| format!("-name {}", shell_quote_arg(pattern, shell_type)))
        .join(" -o ");
    format!(
        "find {} -type f {pattern_args}",
        shell_quote_arg(target_path, shell_type)
    )
}

fn build_powershell_get_childitem_command(patterns: &[String], target_path: &str) -> String {
    let pattern_args = patterns
        .iter()
        // PowerShell expands expressions in double-quoted strings. Single quote
        // each pattern so it is passed unchanged to -Include.
        .map(|pattern| shell_quote_arg(pattern, ShellType::PowerShell))
        .join(",");
    format!(
        "Get-ChildItem -File -Recurse -Include {pattern_args} -Path {} | ForEach-Object {{ $_.FullName }}",
        shell_quote_arg(target_path, ShellType::PowerShell)
    )
}

fn non_empty_lines(str: &str) -> impl Iterator<Item = &str> {
    str.lines().filter(|line| !line.is_empty())
}

impl Entity for FileGlobExecutor {
    type Event = ();
}

#[cfg(test)]
#[path = "file_glob_tests.rs"]
mod tests;

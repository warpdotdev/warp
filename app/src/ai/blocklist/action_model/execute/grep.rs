use std::borrow::Cow;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use warp_util::standardized_path::StandardizedPath;

use super::{is_file_path, is_git_repository};
use crate::ai::agent::{GrepFileMatch, GrepLineMatch, GrepResult};
use crate::ai::paths::host_native_absolute_path;
use crate::terminal::model::session::{shell_quote_arg, ExecuteCommandOptions, Session};
use crate::terminal::shell::ShellType;
use crate::terminal::ShellLaunchData;
const NON_ZERO_EXIT_CODE_ERROR: &str = "Grep command exited with non-zero exit code";

/// Information about a grep failure.
pub(crate) struct GrepError {
    command: Option<String>,
    output: Option<String>,
    error: GrepErrorType,
}

enum GrepErrorType {
    NonZeroExitCode,
    Other(String),
}

impl GrepError {
    /// Creates a grep error with a conversation-safe message.
    pub(crate) fn new(error_message: String) -> Self {
        Self {
            command: None,
            output: None,
            error: GrepErrorType::Other(error_message),
        }
    }

    /// Creates a grep error for a non-zero command exit.
    pub(crate) fn new_for_non_zero_exit_code() -> Self {
        Self {
            command: None,
            output: None,
            error: GrepErrorType::NonZeroExitCode,
        }
    }

    /// Attaches the command that failed.
    pub(crate) fn with_command(mut self, command: String) -> Self {
        self.command = Some(command);
        self
    }

    /// Attaches command output from the failed command.
    pub(crate) fn with_output(mut self, output: String) -> Self {
        self.output = Some(output);
        self
    }

    /// Returns the message to include in the agent conversation.
    pub(crate) fn error_for_conversation(&self) -> String {
        match self {
            GrepError {
                error: GrepErrorType::NonZeroExitCode,
                output: Some(output),
                ..
            } => format!("{NON_ZERO_EXIT_CODE_ERROR}, output:\n{output}"),
            GrepError {
                error: GrepErrorType::NonZeroExitCode,
                output: None,
                ..
            } => NON_ZERO_EXIT_CODE_ERROR.to_string(),
            GrepError {
                error: GrepErrorType::Other(error),
                ..
            } => error.clone(),
        }
    }
}

/// Runs a grep-like search to find the files and line numbers that match the queries.
///
/// Depending on the environment, this uses the most optimized tool to perform the search:
/// - if the search is in a git repo, we run `git grep` in the session.
///   `git grep` is the most optimized tool for searching in a git repo since it's already indexed.
/// - otherwise, if the search is against the local file system, we run `ripgrep` via the library.
///   `ripgrep` is a more optimized version of `grep`.
/// - otherwise, we run vanilla `grep` in the session
pub(crate) async fn run_grep(
    queries: Vec<String>,
    absolute_path: String,
    session: Option<Arc<Session>>,
    shell_launch_data: Option<ShellLaunchData>,
) -> Result<GrepResult, GrepError> {
    if queries.is_empty() {
        return Err(GrepError::new("No queries provided to grep".to_string()));
    }
    let Some(session) = session else {
        return Err(GrepError::new("No session provided to grep".to_string()));
    };

    let is_file = is_file_path(&absolute_path, &session).await;
    let execute_directory = if is_file {
        // If path is a file, use its parent directory as the execution directory.
        // Use StandardizedPath instead of std::path::Path to avoid encoding a
        // remote path with the local platform's path separators.
        let Ok(standardized) = StandardizedPath::try_new(&absolute_path) else {
            return Err(GrepError::new(
                "Could not determine parent directory of file when running grep".to_string(),
            ));
        };
        let Some(parent) = standardized.parent() else {
            return Err(GrepError::new(
                "Could not determine parent directory of file when running grep".to_string(),
            ));
        };
        Cow::Owned(parent.as_str().to_owned())
    } else {
        Cow::Borrowed(absolute_path.as_str())
    };

    // TODO(CODE-239): Cache the result of this check.
    let is_grep_in_git_repo = is_git_repository(&execute_directory, &session)
        .await
        .unwrap_or_else(|e| {
            log::error!("Failed to run command to check if in git repository: {e:?}");
            false
        });
    let shell_type = session.shell().shell_type();

    // The most optimized tool to perform the search is `git grep`;
    // whether the session is local or remote, we can run `git grep` in the session.
    // The next best way to search is ripgrep, but we can only run that if the session is local;
    // ripgrep is run using the core lib, not as a command (not everyone will have it installed).
    // And in the worst case, we run vanilla `grep` in the session. Although not optimal, this should always work.
    if is_grep_in_git_repo {
        run_git_grep_command(
            &queries,
            &absolute_path,
            &session,
            shell_launch_data,
            shell_type,
            &execute_directory,
        )
        .await
    } else {
        #[cfg(not(target_family = "wasm"))]
        if session.is_local() {
            return run_ripgrep(&queries, absolute_path).await;
        }
        if shell_type == ShellType::PowerShell {
            run_select_string_command(
                &queries,
                &absolute_path,
                &session,
                shell_launch_data,
                &execute_directory,
            )
            .await
        } else {
            run_grep_command(
                &queries,
                &absolute_path,
                &session,
                shell_launch_data,
                shell_type,
                &execute_directory,
            )
            .await
        }
    }
}

#[cfg(not(target_family = "wasm"))]
async fn run_ripgrep(queries: &[String], absolute_path: String) -> Result<GrepResult, GrepError> {
    let path = PathBuf::from(absolute_path);
    let result = warp_ripgrep::search::search(queries, &[path], false, false).await;

    match result {
        Ok(matches) => {
            let mut files_map: HashMap<PathBuf, Vec<GrepLineMatch>> = HashMap::new();
            for m in matches {
                files_map
                    .entry(m.file_path)
                    .or_default()
                    .push(GrepLineMatch {
                        line_number: m.line_number as usize,
                    });
            }
            let matched_files: Vec<GrepFileMatch> = files_map
                .into_iter()
                .map(|(file_path, matched_lines)| GrepFileMatch {
                    file_path: file_path.to_string_lossy().to_string(),
                    matched_lines,
                })
                .collect();
            Ok(GrepResult::Success { matched_files })
        }
        Err(e) => Err(GrepError::new(format!("Ripgrep search failed: {e}"))),
    }
}

/// Assumes that git is installed in the user's session.
async fn run_git_grep_command(
    queries: &[String],
    target_path: &str,
    session: &Session,
    shell_launch_data: Option<ShellLaunchData>,
    shell_type: ShellType,
    execute_directory: &str,
) -> Result<GrepResult, GrepError> {
    let grep_command = build_git_grep_command(queries, target_path, shell_type);

    let command_output = session
        .execute_command(
            grep_command.as_str(),
            Some(execute_directory),
            None,
            ExecuteCommandOptions::default(),
        )
        .await
        .map_err(|e| GrepError::new(e.to_string()).with_command(grep_command.clone()))?;
    let output = String::from_utf8_lossy(command_output.output());

    if command_output.success() {
        parse_grep_output(
            output.as_ref(),
            shell_launch_data,
            Some(execute_directory.to_string()),
        )
        .map(|matched_files| GrepResult::Success { matched_files })
        .map_err(|e| {
            GrepError::new(e.to_string())
                .with_command(grep_command)
                .with_output(output.into())
        })
    } else if command_output
        .exit_code()
        .is_some_and(|exit_code| exit_code.value() == 1)
    {
        // If the exit code is 1, then grep completed successfully but found no
        // matches.
        Ok(GrepResult::Success {
            matched_files: vec![],
        })
    } else {
        Err(GrepError::new_for_non_zero_exit_code()
            .with_command(grep_command)
            .with_output(output.into()))
    }
}

async fn run_grep_command(
    queries: &[String],
    target_path: &str,
    session: &Session,
    shell_launch_data: Option<ShellLaunchData>,
    shell_type: ShellType,
    execute_directory: &str,
) -> Result<GrepResult, GrepError> {
    let grep_command = build_grep_command(queries, target_path, shell_type);

    let command_output = session
        .execute_command(
            grep_command.as_str(),
            Some(execute_directory),
            None,
            ExecuteCommandOptions::default(),
        )
        .await
        .map_err(|e| GrepError::new(e.to_string()).with_command(grep_command.clone()))?;
    let output = String::from_utf8_lossy(command_output.output());

    if command_output.success() {
        parse_grep_output(
            output.as_ref(),
            shell_launch_data,
            Some(execute_directory.to_string()),
        )
        .map(|matched_files| GrepResult::Success { matched_files })
        .map_err(|e| {
            GrepError::new(e.to_string())
                .with_command(grep_command)
                .with_output(output.into())
        })
    } else if command_output
        .exit_code()
        .is_some_and(|exit_code| exit_code.value() == 1)
    {
        // If the exit code is 1, then grep completed successfully but found no
        // matches.
        Ok(GrepResult::Success {
            matched_files: vec![],
        })
    } else {
        Err(GrepError::new_for_non_zero_exit_code()
            .with_command(grep_command)
            .with_output(output.into()))
    }
}

/// Runs a PowerShell `Select-String` command.
async fn run_select_string_command(
    queries: &[String],
    target_path: &str,
    session: &Session,
    shell_launch_data: Option<ShellLaunchData>,
    execute_directory: &str,
) -> Result<GrepResult, GrepError> {
    let select_string_command = build_select_string_command(queries, target_path);

    let command_output = session
        .execute_command(
            select_string_command.as_str(),
            Some(execute_directory),
            None,
            ExecuteCommandOptions::default(),
        )
        .await
        .map_err(|e| GrepError::new(e.to_string()).with_command(select_string_command.clone()))?;
    let output = String::from_utf8_lossy(command_output.output());

    if command_output.success() {
        parse_grep_output(
            output.as_ref(),
            shell_launch_data,
            Some(execute_directory.to_string()),
        )
        .map(|matched_files| GrepResult::Success { matched_files })
        .map_err(|e| {
            GrepError::new(e.to_string())
                .with_command(select_string_command)
                .with_output(output.into())
        })
    } else {
        Err(GrepError::new_for_non_zero_exit_code()
            .with_command(select_string_command)
            .with_output(output.into()))
    }
}

fn build_git_grep_command(queries: &[String], target_path: &str, shell_type: ShellType) -> String {
    // This command works on all the shells we support (even PowerShell).
    let mut grep_command = "git --no-pager grep --color=never --untracked -nIE".to_string();
    for query in queries {
        // Queries can originate from model output and project instructions. Keep
        // them as grep arguments so shell substitutions like $() are inert.
        grep_command.push_str(format!(" -e {}", shell_quote_arg(query, shell_type)).as_str());
    }
    grep_command.push_str(format!(" {}", shell_quote_arg(target_path, shell_type)).as_str());
    grep_command
}

fn build_grep_command(queries: &[String], target_path: &str, shell_type: ShellType) -> String {
    // Summary of the options we use:
    // * "--color=never" ensures we don't get colorized output which is harder to parse due to escape sequences
    // * "-n" includes line numbers
    // * "-r" performs a recursive search
    // * "-I" ignores binary files
    // * "-H" prints file name headers
    // * "-E" uses extended regex expressions
    let mut grep_command = "grep --color=never -nrIHE --devices=skip".to_string();
    for query in queries {
        // Queries can originate from model output and project instructions. Keep
        // them as grep arguments so shell substitutions like $() are inert.
        grep_command.push_str(format!(" -e {}", shell_quote_arg(query, shell_type)).as_str());
    }
    grep_command.push_str(format!(" {}", shell_quote_arg(target_path, shell_type)).as_str());
    grep_command
}

fn build_select_string_command(queries: &[String], target_path: &str) -> String {
    // We enable the `-CaseSensitive` flag to match the default behavior of grep.
    // TODO(CODE-239): Make this command more efficient when searching a file.
    format!(
        "Get-ChildItem -Path {} -Recurse -File | Select-String -NoEmphasis -CaseSensitive -Pattern {}",
        shell_quote_arg(target_path, ShellType::PowerShell),
        queries
            .iter()
            // PowerShell evaluates command substitutions in double-quoted
            // strings, so patterns must be single-quoted data arguments.
            .map(|q| shell_quote_arg(q, ShellType::PowerShell))
            .collect::<Vec<_>>()
            .join(",")
    )
}

/// Parses the output of grep or a grep-like command into the format that we pass
/// back to the agent.
///
/// Assumes the output is in the format:
/// `{relative_file_path}:{line_number}:{line_contents}`.
fn parse_grep_output(
    output: &str,
    shell_launch_data: Option<ShellLaunchData>,
    current_working_directory: Option<String>,
) -> anyhow::Result<Vec<GrepFileMatch>> {
    let mut matched_files = HashMap::new();

    for line in output.trim().split("\n") {
        let mut parts = line.split(":");
        let file = parts.next();
        let line_number = parts.next();

        let (Some(file), Some(line_number)) = (file, line_number) else {
            return Err(anyhow::anyhow!(
                "Failed to parse Grep output, unexpected format"
            ));
        };
        let line_number = match line_number.parse::<usize>() {
            Ok(line_number) => line_number,
            Err(e) => {
                return Err(anyhow::anyhow!(
                    "Failed to parse line number in Grep output: {:?}",
                    e
                ));
            }
        };

        matched_files
            .entry(file)
            .or_insert_with(Vec::new)
            .push(GrepLineMatch { line_number });
    }

    Ok(matched_files
        .into_iter()
        .map(|(file, matched_lines)| GrepFileMatch {
            file_path: host_native_absolute_path(
                file,
                &shell_launch_data,
                &current_working_directory,
            ),
            matched_lines,
        })
        .collect())
}

#[cfg(test)]
#[path = "grep_tests.rs"]
mod tests;

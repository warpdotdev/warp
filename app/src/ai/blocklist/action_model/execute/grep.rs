use std::borrow::Cow;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use futures::FutureExt;
use futures::future::BoxFuture;
use warp_errors::report_error;
use warp_util::standardized_path::StandardizedPath;
use warpui::r#async::FutureExt as AsyncFutureExt;
use warpui::{AppContext, Entity, EntityId, ModelContext, ModelHandle, SingletonEntity};

use super::{
    ActionExecution, AnyActionExecution, ExecuteActionInput, PreprocessActionInput,
    get_server_output_id, is_file_path, is_git_repository,
};
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::agent::redaction::redact_secrets;
use crate::ai::agent::{
    AIAgentAction, AIAgentActionResultType, AIAgentActionType, GrepFileMatch, GrepLineMatch,
    GrepResult, ServerOutputId,
};
use crate::ai::blocklist::BlocklistAIPermissions;
use crate::ai::blocklist::telemetry_banner::should_collect_ai_ugc_telemetry;
use crate::ai::paths::{host_native_absolute_path, shell_native_absolute_path};
use crate::terminal::ShellLaunchData;
use crate::terminal::model::session::active_session::ActiveSession;
use crate::terminal::model::session::{ExecuteCommandOptions, Session, shell_quote_arg};
use crate::terminal::shell::ShellType;
use crate::{PrivacySettings, TelemetryEvent, send_telemetry_from_app_ctx};

const GREP_TIMEOUT: Duration = Duration::from_secs(10);
const NON_ZERO_EXIT_CODE_ERROR: &str = "Grep command exited with non-zero exit code";

/// Information about the Grep call that resulted in an error, used to send
/// telemetry about the error.
struct GrepError {
    command: Option<String>,
    output: Option<String>,
    /// The error message from the Grep call. This should NOT contain UGC.
    error: GrepErrorType,
}

enum GrepErrorType {
    NonZeroExitCode,
    Other(String),
}

impl GrepError {
    /// Create a new GrepError with the given error message. This should NOT
    /// contain UGC.
    pub fn new(error_message: String) -> Self {
        Self {
            command: None,
            output: None,
            error: GrepErrorType::Other(error_message),
        }
    }

    pub fn new_for_non_zero_exit_code() -> Self {
        Self {
            command: None,
            output: None,
            error: GrepErrorType::NonZeroExitCode,
        }
    }

    pub fn with_command(mut self, command: String) -> Self {
        self.command = Some(command);
        self
    }

    pub fn with_output(mut self, output: String) -> Self {
        self.output = Some(output);
        self
    }

    /// Returns an error message for logging. This should not contain UGC.
    pub fn error_message(&self) -> &str {
        match &self.error {
            GrepErrorType::NonZeroExitCode => NON_ZERO_EXIT_CODE_ERROR,
            GrepErrorType::Other(error) => error,
        }
    }

    /// Returns an error message to be returned as input to the AI conversation.
    /// This may contain UGC.
    pub fn error_for_conversation(&self) -> String {
        match &self {
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

#[allow(clippy::too_many_arguments)]
fn create_redacted_grep_error_event(
    should_collect_ugc: bool,
    server_output_id: Option<ServerOutputId>,
    mut queries: Vec<String>,
    mut path: String,
    shell_type: Option<ShellType>,
    mut working_directory: Option<String>,
    mut absolute_path: String,
    mut error: GrepError,
) -> TelemetryEvent {
    for query in queries.iter_mut() {
        redact_secrets(query);
    }
    redact_secrets(&mut path);
    if let Some(working_directory) = working_directory.as_mut() {
        redact_secrets(working_directory);
    }
    redact_secrets(&mut absolute_path);
    if let Some(command) = error.command.as_mut() {
        redact_secrets(command);
    }
    if let Some(output) = error.output.as_mut() {
        redact_secrets(output);
    }

    TelemetryEvent::GrepToolFailed {
        queries: should_collect_ugc.then_some(queries),
        path: should_collect_ugc.then_some(path),
        shell_type,
        working_directory: should_collect_ugc.then_some(working_directory).flatten(),
        absolute_path: should_collect_ugc.then_some(absolute_path),
        error: error.error_message().to_string(),
        command: should_collect_ugc.then_some(error.command).flatten(),
        output: should_collect_ugc.then_some(error.output).flatten(),
        server_output_id,
    }
}

#[allow(clippy::too_many_arguments)]
fn log_grep_error(
    conversation_id: AIConversationId,
    queries: Vec<String>,
    path: String,
    shell_type: Option<ShellType>,
    working_directory: Option<String>,
    absolute_path: String,
    error: GrepError,
    ctx: &mut AppContext,
) {
    let should_collect_ugc = should_collect_ai_ugc_telemetry(
        ctx,
        PrivacySettings::handle(ctx)
            .as_ref(ctx)
            .is_telemetry_enabled,
    );
    let server_output_id = get_server_output_id(conversation_id, ctx);

    let event = create_redacted_grep_error_event(
        should_collect_ugc,
        server_output_id,
        queries,
        path,
        shell_type,
        working_directory,
        absolute_path,
        error,
    );
    send_telemetry_from_app_ctx!(event, ctx);
}

pub struct GrepExecutor {
    active_session: ModelHandle<ActiveSession>,
    terminal_view_id: EntityId,
}

impl GrepExecutor {
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
                    action: AIAgentActionType::Grep { path, .. },
                    ..
                },
            conversation_id,
        } = input
        else {
            return false;
        };

        let current_working_directory = self
            .active_session
            .as_ref(ctx)
            .current_working_directory()
            .cloned();
        let shell = self.active_session.as_ref(ctx).shell_launch_data(ctx);
        let absolute_path = host_native_absolute_path(path, &shell, &current_working_directory);

        BlocklistAIPermissions::handle(ctx)
            .as_ref(ctx)
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
    ) -> impl Into<AnyActionExecution> + use<> {
        let AIAgentAction {
            action: AIAgentActionType::Grep { queries, path },
            ..
        } = input.action
        else {
            return ActionExecution::InvalidAction;
        };

        let shell_launch_data = self.active_session.as_ref(ctx).shell_launch_data(ctx);
        let shell_type = self.active_session.as_ref(ctx).shell_type(ctx);
        let current_working_directory = self
            .active_session
            .as_ref(ctx)
            .current_working_directory()
            .cloned();
        let absolute_path = shell_native_absolute_path(
            path,
            shell_launch_data.as_ref(),
            current_working_directory.as_ref(),
        );

        let session = self.active_session.as_ref(ctx).session(ctx);

        let path_clone = path.clone();
        let queries_clone = queries.clone();
        let other_queries_clone = queries.clone();
        let absolute_path_clone = absolute_path.clone();
        let working_directory_clone = current_working_directory.clone();
        let conversation_id_clone = input.conversation_id;
        ActionExecution::new_async(
            async move {
                match run_grep(queries_clone, absolute_path, session, shell_launch_data)
                    .with_timeout(GREP_TIMEOUT)
                    .await
                {
                    Ok(result) => result,
                    Err(_) => Err(GrepError::new("Grep operation timed out".to_string())),
                }
            },
            move |result, ctx| match result {
                Ok(grep_result) => {
                    match grep_result {
                        GrepResult::Error(ref e) => {
                            log::warn!("Executing grep resulted in error: {e:?}");
                            log_grep_error(
                                conversation_id_clone,
                                other_queries_clone,
                                path_clone,
                                shell_type,
                                working_directory_clone,
                                absolute_path_clone,
                                GrepError::new(e.to_string()),
                                ctx,
                            );
                        }
                        GrepResult::Success { .. } => {
                            send_telemetry_from_app_ctx!(TelemetryEvent::GrepToolSucceeded, ctx);
                        }
                        _ => {}
                    }
                    AIAgentActionResultType::Grep(grep_result)
                }
                Err(e) => {
                    log::warn!("Failed to execute grep: {:?}", e.error_message());
                    let error_for_conversation = e.error_for_conversation();
                    log_grep_error(
                        conversation_id_clone,
                        other_queries_clone,
                        path_clone,
                        shell_type,
                        working_directory_clone,
                        absolute_path_clone,
                        e,
                        ctx,
                    );
                    AIAgentActionResultType::Grep(GrepResult::Error(error_for_conversation))
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

/// Runs a grep-like search to find the files and line numbers that match the queries.
///
/// Depending on the environment, this uses the most optimized tool to perform the search:
/// - if the search is in a git repo, we run `git grep` in the session.
///   `git grep` is the most optimized tool for searching in a git repo since it's already indexed.
/// - otherwise, if the search is against the local file system, we run `ripgrep` via the library.
///   `ripgrep` is a more optimized version of `grep`.
/// - otherwise, we run vanilla `grep` in the session
async fn run_grep(
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
            report_error!(e.context("Failed to run command to check if in git repository"));
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

/// The outcome of executing a grep-like command that follows the POSIX
/// convention of exiting with status 1 to mean "completed successfully, no
/// matches" (as `git grep` and GNU/BSD `grep` all do).
enum GrepCommandOutcome {
    NoMatches,
    Matches(String),
}

async fn execute_grep_command(
    command: &str,
    session: &Session,
    execute_directory: &str,
) -> Result<GrepCommandOutcome, GrepError> {
    let command_output = session
        .execute_command(
            command,
            Some(execute_directory),
            None,
            ExecuteCommandOptions::default(),
        )
        .await
        .map_err(|e| GrepError::new(e.to_string()).with_command(command.to_string()))?;

    if command_output.success() {
        Ok(GrepCommandOutcome::Matches(
            String::from_utf8_lossy(command_output.output()).into_owned(),
        ))
    } else if command_output
        .exit_code()
        .is_some_and(|exit_code| exit_code.value() == 1)
    {
        Ok(GrepCommandOutcome::NoMatches)
    } else {
        Err(GrepError::new_for_non_zero_exit_code()
            .with_command(command.to_string())
            .with_output(String::from_utf8_lossy(command_output.output()).into_owned()))
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
    // `-z` has been supported by `git grep` since git 1.6 (2008), so we
    // always use it; unlike the plain-`grep` backends below, there's no
    // realistic case where the installed `git` lacks it. See
    // `parse_null_delimited_grep_output` for why this makes the output
    // unambiguous.
    let grep_command = build_git_grep_command(queries, target_path, shell_type, true);

    match execute_grep_command(&grep_command, session, execute_directory).await? {
        GrepCommandOutcome::NoMatches => Ok(GrepResult::Success {
            matched_files: vec![],
        }),
        GrepCommandOutcome::Matches(output) => parse_null_delimited_grep_output(
            &output,
            shell_launch_data,
            Some(execute_directory.to_string()),
        )
        .map(|matched_files| GrepResult::Success { matched_files })
        .map_err(|e| {
            GrepError::new(e.to_string())
                .with_command(grep_command)
                .with_output(output)
        }),
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
    let null_delimited_command = build_grep_command(queries, target_path, shell_type, true);

    match execute_grep_command(&null_delimited_command, session, execute_directory).await {
        Ok(GrepCommandOutcome::NoMatches) => Ok(GrepResult::Success {
            matched_files: vec![],
        }),
        Ok(GrepCommandOutcome::Matches(output)) => parse_null_delimited_grep_output(
            &output,
            shell_launch_data,
            Some(execute_directory.to_string()),
        )
        .map(|matched_files| GrepResult::Success { matched_files })
        .map_err(|e| {
            GrepError::new(e.to_string())
                .with_command(null_delimited_command)
                .with_output(output)
        }),
        Err(null_delimited_error) => {
            // Not every `grep` on a remote session supports `-Z`/`--null`
            // (e.g. BusyBox). Retry once without it rather than failing the
            // tool outright on those hosts, falling back to the colon-based
            // heuristic parser, which is unambiguous for every case we've
            // seen in practice but not for a path engineered to contain a
            // literal `:<digits>:` sequence (see `split_grep_line`).
            let legacy_command = build_grep_command(queries, target_path, shell_type, false);
            match execute_grep_command(&legacy_command, session, execute_directory).await {
                Ok(GrepCommandOutcome::NoMatches) => Ok(GrepResult::Success {
                    matched_files: vec![],
                }),
                Ok(GrepCommandOutcome::Matches(output)) => parse_legacy_grep_output(
                    &output,
                    shell_launch_data,
                    Some(execute_directory.to_string()),
                )
                .map(|matched_files| GrepResult::Success { matched_files })
                .map_err(|e| {
                    GrepError::new(e.to_string())
                        .with_command(legacy_command)
                        .with_output(output)
                }),
                // Neither attempt worked; report the error from the more
                // capable (and therefore more likely to be informative)
                // `-Z` attempt.
                Err(_) => Err(null_delimited_error),
            }
        }
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
        parse_null_delimited_grep_output(
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

fn build_git_grep_command(
    queries: &[String],
    target_path: &str,
    shell_type: ShellType,
    null_delimited: bool,
) -> String {
    // This command works on all the shells we support (even PowerShell).
    let mut grep_command = "git --no-pager grep --color=never --untracked -nIE".to_string();
    if null_delimited {
        // Delimits the file path with a NUL byte instead of `:`. See
        // `parse_null_delimited_grep_output`.
        grep_command.push_str(" -z");
    }
    for query in queries {
        // Queries can originate from model output and project instructions. Keep
        // them as grep arguments so shell substitutions like $() are inert.
        grep_command.push_str(format!(" -e {}", shell_quote_arg(query, shell_type)).as_str());
    }
    grep_command.push_str(format!(" {}", shell_quote_arg(target_path, shell_type)).as_str());
    grep_command
}

fn build_grep_command(
    queries: &[String],
    target_path: &str,
    shell_type: ShellType,
    null_delimited: bool,
) -> String {
    // Summary of the options we use:
    // * "--color=never" ensures we don't get colorized output which is harder to parse due to escape sequences
    // * "-n" includes line numbers
    // * "-r" performs a recursive search
    // * "-I" ignores binary files
    // * "-H" prints file name headers
    // * "-E" uses extended regex expressions
    let mut grep_command = "grep --color=never -nrIHE --devices=skip".to_string();
    if null_delimited {
        // Delimits the file path with a NUL byte instead of `:`, like `git
        // grep`'s `-z` above. Not every `grep` supports this (e.g.
        // BusyBox); `run_grep_command` retries without it when this flag
        // itself causes the command to fail.
        grep_command.push_str(" -Z");
    }
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
    //
    // `Select-String`'s default output separates the path and line number
    // with `:`, which is ambiguous when the path itself contains a colon
    // (e.g. a Windows drive path like `C:\repo\file.rs`). The trailing
    // `ForEach-Object` reformats each match as `{path}\0{line_number}\0`
    // instead, which `parse_null_delimited_grep_output` expects.
    format!(
        "Get-ChildItem -Path {} -Recurse -File | Select-String -NoEmphasis -CaseSensitive -Pattern {} | ForEach-Object {{ \"$($_.Path)`0$($_.LineNumber)`0\" }}",
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

/// Parses NUL-delimited grep output into the format that we pass back to the
/// agent.
///
/// Expects each record to have the shape `{path}\0{line_number}{sep}...\n`,
/// where `sep` is either a second NUL (as emitted by `git grep -z`, and by
/// `build_select_string_command`'s formatter) or a `:` (as emitted by
/// GNU/BSD `grep`'s `-Z`/`--null`, which only replaces the path separator).
/// Everything from `sep` to the next `\n` is the matched line's content and
/// is discarded, since callers only need the file path and line number.
///
/// Because the path ends at a NUL byte -- which can never appear in a file
/// name on any platform this runs on -- this format stays unambiguous even
/// when the path itself contains colons or newlines, unlike the heuristic
/// `parse_legacy_grep_output` below. A record that doesn't match this shape
/// is skipped; `Err` is returned only when every record in a non-empty
/// output was unparseable, since that indicates the output isn't in this
/// format at all rather than containing one unusual record.
fn parse_null_delimited_grep_output(
    output: &str,
    shell_launch_data: Option<ShellLaunchData>,
    current_working_directory: Option<String>,
) -> anyhow::Result<Vec<GrepFileMatch>> {
    let mut matched_files: HashMap<&str, Vec<GrepLineMatch>> = HashMap::new();
    let mut unparseable_record_count = 0usize;
    let mut remaining = output;

    while !remaining.is_empty() {
        match take_null_delimited_record(remaining) {
            Some((file, line_number, rest)) => {
                matched_files
                    .entry(file)
                    .or_default()
                    .push(GrepLineMatch { line_number });
                remaining = rest;
            }
            None => {
                unparseable_record_count += 1;
                // Resync on the next newline so one malformed record doesn't
                // prevent parsing the rest of the output.
                remaining = match remaining.find('\n') {
                    Some(index) => &remaining[index + 1..],
                    None => "",
                };
            }
        }
    }

    if unparseable_record_count > 0 {
        log::warn!(
            "Skipped {unparseable_record_count} unparseable record(s) of NUL-delimited Grep output"
        );
    }
    if matched_files.is_empty() && unparseable_record_count > 0 {
        return Err(anyhow::anyhow!(
            "Failed to parse Grep output, unexpected format"
        ));
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

/// Consumes one `{path}\0{digits}(\0|:){content}\n` record from the front of
/// `input`, returning the path, the line number, and the remainder of
/// `input` after the record. Returns `None` if `input` doesn't start with a
/// well-formed record.
fn take_null_delimited_record(input: &str) -> Option<(&str, usize, &str)> {
    let (path, after_path) = input.split_once('\0')?;
    if path.is_empty() {
        return None;
    }

    let digit_count = after_path
        .bytes()
        .take_while(|b| b.is_ascii_digit())
        .count();
    if digit_count == 0 {
        return None;
    }
    let (digits, after_digits) = after_path.split_at(digit_count);
    let line_number = digits.parse::<usize>().ok()?;

    let after_separator = match after_digits.as_bytes().first() {
        Some(b'\0') | Some(b':') => &after_digits[1..],
        _ => return None,
    };

    let rest = match after_separator.find('\n') {
        Some(index) => &after_separator[index + 1..],
        None => "",
    };
    Some((path, line_number, rest))
}

/// Parses the output of a `grep` that doesn't support NUL-delimited output
/// (see `parse_null_delimited_grep_output`), into the format that we pass
/// back to the agent. Used only as a fallback when a remote `grep` doesn't
/// support `-Z`/`--null`.
///
/// Assumes the output is in the format:
/// `{relative_file_path}:{line_number}:{line_contents}`. The file path itself
/// can contain colons (e.g. an absolute Windows path like `C:\repo\file.rs`,
/// or a Go module path like `vendor/example.com/foo:v1/x.go`), so lines are
/// split by locating the `:<digits>:` boundary rather than by position. This
/// is ambiguous when the path itself contains that exact sequence (unlike
/// `parse_null_delimited_grep_output`), but that's an acceptable tradeoff for
/// a fallback path. A line that doesn't contain such a boundary is skipped
/// instead of failing the whole parse; an error is only returned when every
/// line was unparseable, since that indicates the output isn't grep-formatted
/// at all rather than containing one unusual path.
fn parse_legacy_grep_output(
    output: &str,
    shell_launch_data: Option<ShellLaunchData>,
    current_working_directory: Option<String>,
) -> anyhow::Result<Vec<GrepFileMatch>> {
    let mut matched_files: HashMap<&str, Vec<GrepLineMatch>> = HashMap::new();
    let mut unparseable_line_count = 0usize;

    for line in output.trim().split("\n") {
        if line.is_empty() {
            continue;
        }
        match split_grep_line(line) {
            Some((file, line_number)) => {
                matched_files
                    .entry(file)
                    .or_default()
                    .push(GrepLineMatch { line_number });
            }
            None => {
                unparseable_line_count += 1;
            }
        }
    }

    if unparseable_line_count > 0 {
        log::warn!("Skipped {unparseable_line_count} unparseable line(s) of Grep output");
    }
    if matched_files.is_empty() && unparseable_line_count > 0 {
        return Err(anyhow::anyhow!(
            "Failed to parse Grep output, unexpected format"
        ));
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

/// Splits a single line of grep output (`{file_path}:{line_number}:{line_contents}`)
/// into its file path and line number by locating the `:<digits>:` boundary
/// between them, since the file path may itself contain colons. Returns
/// `None` if the line doesn't contain such a boundary.
fn split_grep_line(line: &str) -> Option<(&str, usize)> {
    let mut search_start = 0;
    while let Some(relative_colon_pos) = line[search_start..].find(':') {
        let colon_pos = search_start + relative_colon_pos;
        search_start = colon_pos + 1;
        if colon_pos == 0 {
            continue;
        }

        let digits_start = colon_pos + 1;
        let digit_count = line[digits_start..]
            .bytes()
            .take_while(|b| b.is_ascii_digit())
            .count();
        let digits_end = digits_start + digit_count;
        if digit_count == 0 || line.as_bytes().get(digits_end) != Some(&b':') {
            continue;
        }

        if let Ok(line_number) = line[digits_start..digits_end].parse::<usize>() {
            return Some((&line[..colon_pos], line_number));
        }
    }
    None
}

impl Entity for GrepExecutor {
    type Event = ();
}

#[cfg(test)]
#[path = "grep_tests.rs"]
mod tests;

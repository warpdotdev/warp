use crate::ai::block_context::BlockContext;
use crate::ai_assistant::execution_context::WarpAiExecutionContext;
use crate::completer::SessionContext;
#[cfg(feature = "local_fs")]
use crate::persistence::{database_file_path_for_scope, establish_ro_connection, PersistenceScope};
use crate::report_error;
use crate::server::server_api::{AIApiError, ServerApi};
use crate::settings::AISettings;
use crate::terminal::event::UserBlockCompleted;
use crate::terminal::input::{CompleterData, IntelligentAutosuggestionResult};
use crate::terminal::model::session::Sessions;
use crate::terminal::{History, HistoryEntry, TerminalModel};
use crate::workspaces::user_workspaces::UserWorkspaces;
use chrono::Utc;
use futures::stream::AbortHandle;
use itertools::Itertools;
#[cfg_attr(not(feature = "local_fs"), allow(unused_imports))]
use parking_lot::{FairMutex, Mutex};
use std::collections::HashMap;
use std::sync::Arc;
#[cfg(feature = "local_fs")]
use std::time::Duration;
use warp_completer::completer::{
    self, expand_command_aliases, AliasExpansionResult, CompleterOptions,
    CompletionsFallbackStrategy, MatchStrategy,
};
use warp_completer::meta::Spanned;
use warp_completer::parsers::hir::{Command, Expression, FlagType};
use warp_completer::parsers::ParsedExpression;
use warp_core::features::FeatureFlag;
#[cfg(feature = "local_fs")]
use warpui::r#async::FutureExt;
use warpui::{AppContext, Entity, ModelContext, ModelHandle, SingletonEntity};

use super::generate_ai_input_suggestions::{
    create_generate_ai_input_suggestions_request, get_context_messages,
    GenerateAIInputSuggestionsRequest, GenerateAIInputSuggestionsResponseV2, NextCommandContext,
};

cfg_if::cfg_if! {
    if #[cfg(feature = "local_fs")] {
        use diesel::SqliteConnection;
        use std::path::PathBuf;
        use warp_completer::parsers::hir::ArgType;
    }
}

#[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
const MAX_NUM_SIMILAR_HISTORY_CONTEXT: usize = 25;

/// The number of additional preceding commands for each HistoryContext
/// included in the LLM request.
#[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
const NUM_ADDITIONAL_PREV_COMMAND_CONTEXT_LLM: usize = 2;

#[cfg(feature = "local_fs")]
const ARG_GENERATOR_VALIDATION_TIMEOUT: Duration = Duration::from_millis(150);

pub fn is_next_command_enabled(app: &warpui::AppContext) -> bool {
    AISettings::as_ref(app).is_intelligent_autosuggestions_enabled(app)
        && UserWorkspaces::as_ref(app).is_next_command_enabled()
}

/// Information about an autosuggestion that would have been made if purely based off history.
/// If there was no history, history_command_prediction would be an empty string.
#[derive(Clone, Default, PartialEq, Debug)]
pub struct HistoryBasedAutosuggestionState {
    pub history_command_prediction: String,
    pub history_command_prediction_likelihood: f64,
    pub total_history_count: usize,
}

#[derive(Clone, Default, PartialEq)]
pub enum NextCommandSuggestionState {
    #[default]
    None,
    Cycling,
    Ready {
        request: Box<GenerateAIInputSuggestionsRequest>,
        response: GenerateAIInputSuggestionsResponseV2,
        /// How long the request took to complete, in milliseconds.
        request_duration_ms: i64,
        /// If true, we made a call to an LLM to generate this.
        /// Otherwise it came from history.
        is_from_ai: bool,
        /// If true, this suggestion came from the user explicitly cycling (is not the initial suggestion).
        is_from_cycle: bool,
        history_based_autosuggestion_state: HistoryBasedAutosuggestionState,
    },
}

impl NextCommandSuggestionState {
    pub fn is_ready(&self) -> bool {
        matches!(self, NextCommandSuggestionState::Ready { .. })
    }

    pub fn is_cycling(&self) -> bool {
        matches!(self, NextCommandSuggestionState::Cycling)
    }

    pub fn command_suggestion(&self) -> Option<&str> {
        match &self {
            // The server only returns one command suggestion as the most likely action.
            NextCommandSuggestionState::Ready { response, .. } => {
                let command = &response.most_likely_action;
                // If AI accidentally returned JSON instead of a plain string for the most likely action, don't use it.
                if command.starts_with('{') {
                    return None;
                }
                Some(command)
            }
            _ => None,
        }
    }
}

/// Struct storing the result of the zero-state next command suggestion,
/// used for telemetry purposes.
#[derive(Clone)]
pub struct ZeroStateSuggestionInfo {
    pub request: Box<GenerateAIInputSuggestionsRequest>,
    pub response: GenerateAIInputSuggestionsResponseV2,
    /// How long the request took to complete, in milliseconds.
    pub request_duration_ms: i64,
    /// If true, we made a call to an LLM to generate this.
    /// Otherwise it came from history.
    pub is_from_ai: bool,
    pub history_based_autosuggestion_state: HistoryBasedAutosuggestionState,
}

pub struct NextCommandModel {
    sessions: ModelHandle<Sessions>,
    model: Arc<FairMutex<TerminalModel>>,
    server_api: Arc<ServerApi>,
    #[cfg(feature = "local_fs")]
    conn: Option<Arc<Mutex<SqliteConnection>>>,

    next_command_state: NextCommandSuggestionState,
    /// Context used to generate the zero-state suggestion.
    /// We reuse this but apply filtering on history_contexts as the user edits the input.
    cached_zerostate_next_command_context: Option<NextCommandContext>,
    zerostate_suggestion_info: Option<ZeroStateSuggestionInfo>,
    next_command_abort_handle: Option<AbortHandle>,
}

impl Entity for NextCommandModel {
    type Event = NextCommandModelEvent;
}

pub enum NextCommandModelEvent {
    NextCommandSuggestionReady,
}

impl NextCommandModel {
    pub fn new(
        sessions: ModelHandle<Sessions>,
        model: Arc<FairMutex<TerminalModel>>,
        server_api: Arc<ServerApi>,
    ) -> Self {
        #[cfg(feature = "local_fs")]
        let conn = database_file_path_for_scope(&PersistenceScope::App)
            .to_str()
            .and_then(|db_url| {
                establish_ro_connection(db_url)
                    .ok()
                    .map(|conn| Arc::new(Mutex::new(conn)))
            });
        Self {
            sessions,
            model,
            server_api,
            #[cfg(feature = "local_fs")]
            conn,
            next_command_state: NextCommandSuggestionState::None,
            cached_zerostate_next_command_context: None,
            zerostate_suggestion_info: None,
            next_command_abort_handle: None,
        }
    }

    /// Returns snippets of command history (HistoryContext) that are similar to the completed_block.
    /// Each HistoryContext contains some sequential commands run in the same session,
    /// where the last element of HistoryContext.previous_commands is the same as completed_block.
    /// Returns None if there was a connection issue, and Some(empty vec)
    /// if there is no similar historical context.
    #[cfg(feature = "local_fs")]
    pub fn get_similar_history_context(
        conn: &mut SqliteConnection,
        completed_block: &UserBlockCompleted,
        num_additional_preceding_commands: usize,
    ) -> Vec<crate::ai::predict::generate_ai_input_suggestions::HistoryContext> {
        // The number of commands from history affects how quickly we "learn" new patterns, the lower the faster.
        let Ok(same_commands_from_history) =
            crate::persistence::commands::get_same_commands_from_history(
                conn,
                completed_block,
                MAX_NUM_SIMILAR_HISTORY_CONTEXT,
            )
        else {
            return vec![];
        };
        // Iterate from oldest to newest
        same_commands_from_history
            .into_iter()
            .rev()
            .filter_map(|command| {
                let next_command =
                    crate::persistence::commands::get_next_command(conn, &command).ok()?;
                if num_additional_preceding_commands == 0 {
                    return Some(
                        crate::ai::predict::generate_ai_input_suggestions::HistoryContext {
                            previous_commands: vec![command],
                            next_command,
                        },
                    );
                }
                // We know next_command comes after command.
                // Get some more commands that came before command so there's additional context before next_command.
                let mut previous_commands = crate::persistence::commands::get_previous_commands(
                    conn,
                    &command,
                    num_additional_preceding_commands,
                )
                .ok()?;
                previous_commands.push(command);
                Some(
                    crate::ai::predict::generate_ai_input_suggestions::HistoryContext {
                        previous_commands,
                        next_command,
                    },
                )
            })
            .collect()
    }

    pub fn get_state(&self) -> &NextCommandSuggestionState {
        &self.next_command_state
    }

    pub fn get_zero_state_suggestion_info(&self) -> Option<&ZeroStateSuggestionInfo> {
        self.zerostate_suggestion_info.as_ref()
    }

    pub fn clear_state(&mut self) {
        self.next_command_state = NextCommandSuggestionState::None;
        self.cached_zerostate_next_command_context = None;
        self.zerostate_suggestion_info = None;
        self.abort_inflight_request();
    }

    pub fn abort_inflight_request(&mut self) {
        if let Some(abort_handle) = self.next_command_abort_handle.take() {
            abort_handle.abort();
        }
    }

    pub fn cycle_next_command_suggestion(&mut self, _ctx: &mut ModelContext<Self>) {
        // TODO(roland): reconsider down arrow UX for next command or remove completely
    }

    #[cfg_attr(not(feature = "local_fs"), allow(unused_variables))]
    fn get_next_command_context(
        terminal_model: Arc<FairMutex<TerminalModel>>,
        #[cfg(feature = "local_fs")] conn: Option<Arc<Mutex<SqliteConnection>>>,
        ai_execution_context: WarpAiExecutionContext,
        block_completed: &UserBlockCompleted,
    ) -> NextCommandContext {
        #[cfg_attr(not(feature = "local_fs"), allow(unused_mut))]
        let mut history_contexts = vec![];
        let context_messages = get_context_messages(terminal_model.clone(), 5, 100, 200);
        #[cfg(feature = "local_fs")]
        if let Some(conn) = conn {
            let mut conn = conn.lock();
            history_contexts = Self::get_similar_history_context(
                &mut conn,
                block_completed,
                NUM_ADDITIONAL_PREV_COMMAND_CONTEXT_LLM,
            );
        }
        NextCommandContext {
            history_contexts,
            ai_execution_context,
            context_messages,
        }
    }

    /// Generates a zero-state next command suggestion immediately after a block completes.
    pub fn generate_next_command_suggestion(
        &mut self,
        block_completed: UserBlockCompleted,
        context: WarpAiExecutionContext,
        completer_data: CompleterData,
        block_context: Option<Box<BlockContext>>,
        previous_result: Option<IntelligentAutosuggestionResult>,
        ctx: &mut ModelContext<Self>,
    ) {
        // Clear the cached next command context so we don't use stale data.
        self.cached_zerostate_next_command_context = None;
        self.zerostate_suggestion_info = None;
        self.generate_next_command_suggestion_with_prefix(
            None,
            block_completed,
            context,
            completer_data,
            block_context,
            previous_result,
            ctx,
        );
    }

    /// Returns the most recent command with a matching prefix run in the user's current working directory.
    /// If no such command exists, returns the most recent command anywhere with a matching prefix.
    pub fn get_reverse_chronological_potential_autosuggestions(
        prefix: &str,
        completer_data: &CompleterData,
        app: &AppContext,
    ) -> Option<Vec<HistoryEntry>> {
        let session_id = completer_data.active_block_session_id()?;
        let history_entries = History::as_ref(app).commands(session_id)?;
        let working_dir = completer_data
            .active_block_metadata
            .as_ref()
            .and_then(|block_metadata| block_metadata.current_working_directory());
        Some(find_potential_autosuggestions_from_history(
            history_entries.into_iter(),
            prefix,
            working_dir,
        ))
    }

    /// Generates a next command suggestion with a prefix in the input.
    #[cfg_attr(not(feature = "local_fs"), allow(unused_variables))]
    #[expect(clippy::too_many_arguments)]
    pub fn generate_next_command_suggestion_with_prefix(
        &mut self,
        prefix: Option<String>,
        block_completed: UserBlockCompleted,
        context: WarpAiExecutionContext,
        completer_data: CompleterData,
        block_context: Option<Box<BlockContext>>,
        previous_result: Option<IntelligentAutosuggestionResult>,
        ctx: &mut ModelContext<Self>,
    ) {
        let server_api = self.server_api.clone();
        let terminal_model = self.model.clone();
        let cached_next_command_context = self.cached_zerostate_next_command_context.clone();

        let completion_context = completer_data.completion_session_context(ctx);
        // This is only needed if we have a prefix.
        let reverse_chronological_potential_autosuggestions = if let Some(prefix) = &prefix {
            Self::get_reverse_chronological_potential_autosuggestions(prefix, &completer_data, ctx)
        } else {
            None
        };

        #[cfg(feature = "local_fs")]
        let conn = self.conn.clone();
        self.next_command_state = NextCommandSuggestionState::None;
        self.abort_inflight_request();
        let session_env_vars = completer_data
            .active_block_session_id()
            .and_then(|session_id| {
                self.sessions.read(ctx, |sessions, _| {
                    sessions.get_env_vars_for_session(session_id)
                })
            });
        self.next_command_abort_handle = Some(
            ctx.spawn(
                async move {
                    let mut history_based_autosuggestion_state =
                        HistoryBasedAutosuggestionState::default();
                    let start_ts_ms = Utc::now().timestamp_millis();

                    let mut next_command_context =
                        if let Some(cached_next_command_context) = cached_next_command_context {
                            cached_next_command_context
                        } else {
                            Self::get_next_command_context(
                                terminal_model,
                                #[cfg(feature = "local_fs")]
                                conn,
                                context,
                                &block_completed,
                            )
                        };
                    // Filter history contexts for only cases where the next command matches our prefix.
                    if let Some(prefix) = &prefix {
                        next_command_context.history_contexts = next_command_context
                            .history_contexts
                            .into_iter()
                            .filter(|context| context.next_command.command.starts_with(prefix))
                            .collect_vec();
                    }
                    // First, use rich history to find commands with a matching prefix that were run
                    // in a similar context, taking into account the most recent block run.
                    if !next_command_context.history_contexts.is_empty() {
                        let mut history_next_command_counts = counter::Counter::new();
                        for history_context in &next_command_context.history_contexts {
                            history_next_command_counts[&history_context.next_command.command] += 1;
                        }

                        let mut total_history_count = history_next_command_counts.total::<usize>();
                        let most_likely_next_commands =
                            history_next_command_counts.k_most_common_ordered(5);
                        for (most_likely_next_command, count) in &most_likely_next_commands {
                            if !is_command_valid(most_likely_next_command, completion_context.as_ref(), session_env_vars.as_ref()).await {
                                log::debug!("Discarding most likely next command from rich history that failed validation: `{most_likely_next_command}`");
                                total_history_count -= *count;
                                continue;
                            }
                            let history_command_prediction_likelihood =
                                *count as f64 / total_history_count as f64;
                            history_based_autosuggestion_state = HistoryBasedAutosuggestionState {
                                history_command_prediction: most_likely_next_command.to_owned(),
                                history_command_prediction_likelihood,
                                total_history_count,
                            };

                            // If one command is very likely based on history, skip the LLM call and return it directly.
                            // Use history-based autosuggestion if there are a min number of similar cases in history
                            // AND the same command was run at least skip_llm_confidence_threshold of the time.
                            // Partial autosuggestions with a prefix have more lenient requirements
                            // because latency matters more as the user is typing.
                            let (min_history_count, skip_llm_confidence_threshold) =
                                if prefix.is_some() {
                                    (1, 0.1)
                                } else {
                                    (2, 0.25)
                                };
                            if total_history_count >= min_history_count
                                && history_command_prediction_likelihood
                                    >= skip_llm_confidence_threshold
                            {
                                // We construct the request even though we're not sending it to the server because
                                // it might be used later for cycling next command suggestions.
                                let request = create_generate_ai_input_suggestions_request(
                                    next_command_context.clone(),
                                    prefix,
                                    block_context,
                                    previous_result,
                                );
                                return (
                                    Ok(GenerateAIInputSuggestionsResponseV2 {
                                        commands: vec![most_likely_next_command.to_owned()],
                                        ai_queries: vec![],
                                        most_likely_action: most_likely_next_command.to_owned(),
                                    }),
                                    request,
                                    false,
                                    start_ts_ms,
                                    history_based_autosuggestion_state,
                                    false,
                                    next_command_context,
                                );
                            }
                        }
                    }
                    let request = create_generate_ai_input_suggestions_request(
                        next_command_context.clone(),
                        prefix.clone(),
                        block_context,
                        previous_result,
                    );

                    // For zero-state next command suggestions, return the result immediately.
                    let Some(prefix) = prefix else {
                        return (
                            server_api.generate_ai_input_suggestions(&request).await,
                            request,
                            true,
                            start_ts_ms,
                            history_based_autosuggestion_state,
                            false,
                            next_command_context,
                        );
                    };

                    // At this point we know we're generating a partial suggestion with a prefix.
                    // First, return the most recent command with a matching prefix run in the same pwd
                    // (if exists, otherwise just most recent command anywhere with matching prefix).
                    for reverse_chronological_command in reverse_chronological_potential_autosuggestions.unwrap_or_default() {
                        if is_command_valid(&reverse_chronological_command.command, completion_context.as_ref(), session_env_vars.as_ref()).await {
                            return (
                                Ok(GenerateAIInputSuggestionsResponseV2 {
                                    commands: vec![reverse_chronological_command.command.clone()],
                                ai_queries: vec![],
                                most_likely_action: reverse_chronological_command.command,
                            }),
                            request,
                            false,
                            start_ts_ms,
                                history_based_autosuggestion_state,
                                false,
                                next_command_context,
                            );
                        }
                    }

                    // If we have no command anywhere in history with a matching prefix, fallback to the first completer result.
                    if let Some(completion_context) = completion_context {
                        let completion_result = completer::suggestions(
                            &prefix,
                            prefix.len(),
                            session_env_vars.as_ref(),
                            CompleterOptions {
                                match_strategy: MatchStrategy::CaseSensitive,
                                fallback_strategy: CompletionsFallbackStrategy::None,
                                suggest_file_path_completions_only: false,
                                parse_quotes_as_literals: false,
                            },
                            &completion_context,
                        )
                        .await;

                        let autosuggestion = completion_result.and_then(|result| {
                            let replacement_span = result.replacement_span;
                            result.suggestions.into_iter().next().map(|s| {
                                // Reproduce the final buffer text with the autosuggestion since the
                                // completer only gives the replacement span of the suggestion.
                                let result = format!(
                                    "{}{}",
                                    &prefix[0..replacement_span.start()],
                                    s.replacement()
                                );
                                result
                            })
                        });

                        if let Some(autosuggestion) = autosuggestion {
                            if is_command_valid(&autosuggestion, Some(&completion_context), session_env_vars.as_ref()).await {
                                return (
                                    Ok(GenerateAIInputSuggestionsResponseV2 {
                                        commands: vec![autosuggestion.clone()],
                                    ai_queries: vec![],
                                    most_likely_action: autosuggestion,
                                }),
                                request,
                                false,
                                    start_ts_ms,
                                    history_based_autosuggestion_state,
                                    false,
                                    next_command_context,
                                );
                            }
                        }
                    };

                    // Only if we have no commands from history and no completions, use the LLM to generate a partial suggestion.
                    let response = server_api.generate_ai_input_suggestions(&request).await;
                    (
                        response,
                        request,
                        true,
                        start_ts_ms,
                        history_based_autosuggestion_state,
                        false,
                        next_command_context,
                    )
                },
                Self::on_next_command_suggestion_result,
            )
            .abort_handle(),
        );
    }

    fn on_next_command_suggestion_result(
        &mut self,
        result: (
            Result<GenerateAIInputSuggestionsResponseV2, AIApiError>,
            GenerateAIInputSuggestionsRequest,
            bool,
            i64,
            HistoryBasedAutosuggestionState,
            bool,
            NextCommandContext,
        ),
        ctx: &mut ModelContext<Self>,
    ) {
        self.next_command_abort_handle = None;
        let (
            result,
            request,
            is_from_ai,
            start_ts_ms,
            history_based_autosuggestion_state,
            is_from_cycle,
            next_command_context,
        ) = result;
        let end_ts_ms = Utc::now().timestamp_millis();
        let request_duration_ms = end_ts_ms - start_ts_ms;
        if request.prefix.is_none() {
            self.cached_zerostate_next_command_context = Some(next_command_context);
        }
        match result {
            Ok(response) => {
                if let Some(prefix) = &request.prefix {
                    if !response.most_likely_action.starts_with(prefix) {
                        // This is not expected to happen because the server applies its own filtering,
                        // but check just in case.
                        log::warn!(
                            "Next command suggestion `{}` does not start with prefix `{}`.",
                            response.most_likely_action,
                            prefix
                        );
                        return;
                    }
                } else {
                    self.zerostate_suggestion_info = Some(ZeroStateSuggestionInfo {
                        request: Box::new(request.clone()),
                        response: response.clone(),
                        request_duration_ms,
                        is_from_ai,
                        history_based_autosuggestion_state: history_based_autosuggestion_state
                            .clone(),
                    });
                }

                self.next_command_state = NextCommandSuggestionState::Ready {
                    request: Box::new(request),
                    response,
                    request_duration_ms,
                    is_from_ai,
                    is_from_cycle,
                    history_based_autosuggestion_state,
                };
                ctx.emit(NextCommandModelEvent::NextCommandSuggestionReady);
            }
            Err(err) => {
                report_error!(
                    anyhow::anyhow!(err).context("Failed to generate Next Command suggestion")
                );
            }
        };
    }
}

impl SingletonEntity for NextCommandModel {}

/// Validates that the arg is valid given its type (e.g. filepath exists if it's a filepath arg).
/// This uses a file system call, so this function should be called only in background threads.
#[cfg_attr(not(feature = "local_fs"), allow(unused_variables))]
async fn is_arg_valid(
    full_command: &str,
    arg: &Spanned<ParsedExpression>,
    ctx: &SessionContext,
    session_env_vars: Option<&HashMap<String, String>>,
) -> bool {
    let Expression::ValidatableArgument(arg_types_to_validate) = arg.expression() else {
        return true;
    };
    // The expression shouldn't be parsed as a `ValidatableArgument` if the arg types are empty,
    // but we check this just in case.
    if arg_types_to_validate.is_empty() {
        return true;
    }
    cfg_if::cfg_if! {
        if #[cfg(feature = "local_fs")] {
            // If we have arg types to validate, the arg must pass validation for at least one of them.
            // If the argument has one or more generators, validate these last because they're more expensive
            // and we can check all generators together using completions suggestions.
            let mut has_generator_arg_type = false;
            for arg_type in arg_types_to_validate {
                match arg_type {
                    ArgType::File => {
                        let mut path_arg = PathBuf::from(arg.value().as_str());
                        if path_arg.is_relative() {
                            if let Ok(working_dir) = PathBuf::try_from(ctx.current_working_directory.clone()) {
                                path_arg = working_dir.join(path_arg);
                            }
                        }
                        if path_arg.is_file() {
                            return true;
                        }
                    }
                    ArgType::Folder => {
                        let mut path_arg = PathBuf::from(arg.value().as_str());
                        if path_arg.is_relative() {
                            if let Ok(working_dir) = PathBuf::try_from(ctx.current_working_directory.clone()) {
                                path_arg = working_dir.join(path_arg);
                            }
                        }
                        if path_arg.is_dir() {
                            return true;
                        }
                    }
                    ArgType::Generator(_) => {
                        has_generator_arg_type = true;
                    }
                };
            }
            if has_generator_arg_type {
                // We don't have completions implemented for feature flags like --features=with_local_server.
                // If arg is the span of `with_local_server`, attempting to complete on --features= to validate it will return no results.
                // We should only use completions to validate the arg if the previous character is whitespace, until completions handles this case.
                let prev_char = full_command.get(..arg.span.start()).and_then(|s| s.chars().next_back());
                if prev_char.is_some_and(|c| !c.is_whitespace()) {
                    return true;
                }
                // Running completions runs all generators, so we only need to do this once.
                // TODO(roland): this also generates completions from sources other than generators, which are unnecessary.
                // If performance becomes a concern, consider validating against generators sequentially and returning early if valid.
                // We use completions suggestions because it's simpler to implement and read.
                let completions_future = completer::suggestions(
                    full_command,
                    arg.span.start(),
                    session_env_vars,
                    CompleterOptions {
                        match_strategy: MatchStrategy::CaseSensitive,
                        fallback_strategy: CompletionsFallbackStrategy::None,
                        suggest_file_path_completions_only: false,
                        parse_quotes_as_literals: false,
                    },
                    ctx,
                );

                // If the completions call times out, assume the arg is valid.
                // This is necessary because some generators can hang (e.g. kubectl commands if the cluster isn't running).
                let Ok(completion_result) = completions_future.with_timeout(ARG_GENERATOR_VALIDATION_TIMEOUT).await else {
                    log::debug!("Generator validation for arg `{}` in command `{}` timed out - assuming it's valid", arg.value().as_str(), full_command);
                    return true;
                };

                let Some(completion_result) = completion_result else {
                    return true;
                };
                for suggestion in completion_result.suggestions {
                    if suggestion.display() == arg.value().as_str() {
                        return true;
                    }
                }
            }
            // If we didn't pass validation for any of the possible arg types, this arg is invalid.
            log::debug!("arg `{}` in command `{}` failed validation", arg.value().as_str(), full_command);
            false
        } else {
            true
        }
    }
}

/// Validates the command is valid.
/// Currently uses completions specs to check if parsing is successful, and validates
/// that any filepaths args actually exist on disk.
/// This uses a file system call, so this function should be called only in background threads.
pub async fn is_command_valid(
    command: &str,
    ctx: Option<&SessionContext>,
    session_env_vars: Option<&HashMap<String, String>>,
) -> bool {
    if !FeatureFlag::ValidateAutosuggestions.is_enabled() {
        return true;
    }
    let Some(ctx) = ctx else {
        return true;
    };
    let AliasExpansionResult {
        expanded_command_line,
        classified_command,
        ..
    } = expand_command_aliases(command, false, ctx).await;

    let Some(classified_command) = classified_command else {
        return true;
    };

    // We assume the command is valid on parse error because
    // 1. Our completion specs are not always comprehensive (unknown args/options cause parse error)
    // 2. Our parsing logic has some bugs that need to be investigated (INT-816)
    if classified_command.error.is_some() {
        log::debug!(
            "Assuming command `{}` is valid because it failed to parse: {:?}",
            expanded_command_line,
            classified_command.error.unwrap()
        );
        return true;
    }
    // If we can't classify the command, it means we don't have completion specs for it.
    // Assume it's valid.
    let Command::Classified(shell_command) = classified_command.command else {
        return true;
    };
    if let Some(positionals) = &shell_command.args.positionals {
        for positional in positionals {
            if !is_arg_valid(&expanded_command_line, positional, ctx, session_env_vars).await {
                return false;
            }
        }
    }
    if let Some(flags) = shell_command.args.flags {
        for flag in flags.iter() {
            if let FlagType::Argument { value } = &flag.flag_type {
                if !is_arg_valid(&expanded_command_line, value, ctx, session_env_vars).await {
                    return false;
                }
            }
        }
    }
    true
}

/// Scans the given history entries in reverse order for commands that start
/// with the buffer text to return as a potential autosuggestion. Prioritizes commands
/// in history that were executed in the user's current working directory,
/// with any command executed in other directories at the end.
fn find_potential_autosuggestions_from_history<'a>(
    history_entries: impl DoubleEndedIterator<Item = &'a HistoryEntry>,
    buffer_text: &str,
    working_dir: Option<&str>,
) -> Vec<HistoryEntry> {
    let mut commands_in_same_dir = vec![];
    let mut commands_in_other_dirs = vec![];
    for entry in history_entries.rev() {
        if !entry.command.starts_with(buffer_text) {
            continue;
        }
        let same_dir = entry
            .pwd
            .as_ref()
            .zip(working_dir)
            .is_some_and(|(pwd, working_dir)| pwd == working_dir);

        if same_dir {
            commands_in_same_dir.push(entry.clone());
        } else {
            commands_in_other_dirs.push(entry.clone());
        }
    }
    commands_in_same_dir.extend(commands_in_other_dirs);
    commands_in_same_dir
}

#[cfg(test)]
#[path = "next_command_model_tests.rs"]
mod tests;

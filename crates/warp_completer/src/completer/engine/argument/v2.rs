//! Contains the v2 implementation of argument suggestion generation that depends on the JS-compatible
//! command signature struct (`crate::signatures::CommandSignature`).
//!
//! Functionality required for parity with the legacy implementation that is yet-to-be implemented
//! is called out throughout with a TODO(completions-v2) comment.
//!
//! Functions in this file closely mirror their legacy implementations in `super::legacy`, though
//! as more functionality is implemented in V2, eventually surpassing the functionality provided by
//! the legacy implementation, the internal structure and logic in this module may begin to
//! (appropriately) diverge.
use std::borrow::Cow;
use std::collections::HashMap;

use itertools::Itertools;
use smol_str::SmolStr;

use super::add_extra_positional;
use crate::completer::GeneratorContext;
use crate::{
    completer::{
        context::call_js_function,
        engine::path::{
            sorted_cd_directories, sorted_directories_relative_to, sorted_paths_relative_to,
        },
        CommandExitStatus, CompleterOptions, CompletionContext, CompletionsFallbackStrategy,
        LocationType, MatchStrategy, MatchedSuggestion, Suggestion, SuggestionType,
    },
    meta::{Span, Spanned},
    parsers::{
        hir::{self, ShellCommand},
        ArgumentError::{MissingMandatoryPositional, MissingValueForName, UnexpectedArgument},
        ClassifiedCommand, ParseError, ParseErrorReason, ParsedToken,
    },
    signatures::{
        self, Argument, ArgumentValue, Command, GeneratorCompletionContext, GeneratorFn,
        GeneratorResults, GeneratorScript, TemplateType,
    },
};

/// Returns completion suggestions for argument values based on the given `input`.
#[allow(clippy::too_many_arguments)]
pub async fn complete(
    input: &str,
    tokens_without_last_editing: &[&str],
    classified_command: ClassifiedCommand,
    found_signature: Option<&Command>,
    location: &Spanned<LocationType>,
    session_env_vars: Option<&HashMap<String, String>>,
    parsed_argument: &ParsedToken,
    options: &CompleterOptions,
    ctx: &dyn CompletionContext,
) -> Vec<MatchedSuggestion> {
    let mut suggestions = Default::default();

    // True if and only if we called the complete function.
    let mut arg_has_spec = false;
    if let Some(found_signature) = found_signature {
        if let hir::Command::Classified(mut shell_command) = classified_command.command {
            suggestions = match classified_command.error {
                Some(error) => {
                    let (results, complete_called) = suggestions_for_parse_error(
                        input,
                        tokens_without_last_editing,
                        &location.span,
                        error,
                        &mut shell_command,
                        found_signature,
                        session_env_vars,
                        options,
                        ctx,
                    )
                    .await;
                    arg_has_spec = complete_called;
                    results
                }
                None => {
                    let (results, complete_called) = suggestions_for_last_argument(
                        tokens_without_last_editing,
                        &mut shell_command,
                        &location.span,
                        found_signature,
                        session_env_vars,
                        options,
                        ctx,
                    )
                    .await;
                    arg_has_spec = complete_called;
                    results
                }
            }
        }
    }

    // If we were unsuccessful in getting completions *and* there was not
    // a completion spec we attempted, fallback to the fallback type (if any).
    if suggestions.is_empty()
        && !arg_has_spec
        && matches!(
            options.fallback_strategy,
            CompletionsFallbackStrategy::FilePaths
        )
    {
        if let Some(path_completion_context) = ctx.path_completion_context() {
            suggestions = sorted_paths_relative_to(
                parsed_argument,
                options.match_strategy,
                path_completion_context,
            )
            .await;
        }
    }

    suggestions
}

/// Generate suggestions for the case where there was a parse error. Given there
/// was a parse error, there could be a few possibilities for options we'd want
/// to suggest based on the error from the parser:
/// 1) MissingMandatoryPositional: The command is missing a mandatory argument
///    at a certain index. In this case, we read the index from the error and
///    execute `complete` on the argument to get the possible suggestions for the
///    argument.
/// 2) MissingValueForName: A flag was supplied without a corresponding value, in
///    this case we suggest the possible arguments for the option that is missing.
/// 3) Unexpected argument: An extra argument was supplied. This extra argument could be the prefix
///    of a subcommand, so we suggest subcommands that start with the value of the extra argument
#[allow(clippy::too_many_arguments)]
async fn suggestions_for_parse_error(
    input: &str,
    tokens_without_last_editing: &[&str],
    cursor: &Span,
    root_err: ParseError,
    shell_command: &mut ShellCommand,
    command_signature: &Command,
    session_env_vars: Option<&HashMap<String, String>>,
    options: &CompleterOptions,
    ctx: &dyn CompletionContext,
) -> (Vec<MatchedSuggestion>, bool) {
    match root_err.reason {
        ParseErrorReason::ArgumentError {
            command: _,
            error:
                MissingValueForName {
                    name,
                    missing_arg_index,
                },
        } => {
            // If there was trailing whitespace in the line, respect the error and try to complete based
            // on the missing argument. If there wasn't any trailing whitespace, the user is trying
            // to complete an argument before the one that's missing (such as `git push ori<tab>`) so we
            // treat this as successful parse so that we can parse out the argument correctly.
            if shell_command.args.ending_whitespace.is_some() {
                let mut arg_has_spec = false;

                let argument = command_signature
                    .options
                    .iter()
                    .find(|opt| opt.has_name(&*name))
                    .and_then(|opt| opt.arguments.get(missing_arg_index));

                let results = match argument {
                    None => Default::default(),
                    Some(arg) => {
                        // Complete on the exact missing argument for the flag
                        // (rather than combining the completions for _all_ of the flags' args)
                        arg_has_spec = true;
                        generate_suggestions_for_argument(
                            arg,
                            &ParsedToken::empty(),
                            tokens_without_last_editing,
                            session_env_vars,
                            options,
                            ctx,
                        )
                        .await
                    }
                };

                return (results, arg_has_spec);
            } else {
                return suggestions_for_last_argument(
                    tokens_without_last_editing,
                    shell_command,
                    cursor,
                    command_signature,
                    session_env_vars,
                    options,
                    ctx,
                )
                .await;
            }
        }
        ParseErrorReason::ArgumentError {
            command: _,
            error:
                MissingMandatoryPositional {
                    name: _name,
                    positional_index,
                },
        } => {
            // If there was ending whitespace in the line respect the error and try to complete based
            // on the missing positional. If there was not an ending whitespace, the user is try trying
            // to complete a positional before the one that's missing such as `git push ori<tab>` so we
            // treat this as successful parse so that we can parse out the positional correctly.
            if shell_command.args.ending_whitespace.is_some() {
                add_extra_positional(shell_command, cursor);
                let argument = command_signature
                    .arguments
                    .get(positional_index)
                    .expect("argument should exist based on error from parser");
                let results = generate_suggestions_for_argument(
                    argument,
                    &ParsedToken::empty(),
                    tokens_without_last_editing,
                    session_env_vars,
                    options,
                    ctx,
                )
                .await;
                return (results, true);
            } else {
                return suggestions_for_last_argument(
                    tokens_without_last_editing,
                    shell_command,
                    cursor,
                    command_signature,
                    session_env_vars,
                    options,
                    ctx,
                )
                .await;
            }
        }
        ParseErrorReason::ArgumentError {
            command: _,
            error: UnexpectedArgument(arg),
        } => {
            if arg.span.end() == input.len() {
                // The unexpected argument could be a prefix for a subcommand.
                let prefix = arg.item.as_str();
                let results = (command_signature.subcommands.iter().filter_map(|subcmd| {
                    options
                        .match_strategy
                        .get_match_type(prefix, subcmd.name.as_str())
                        .map(|match_type| {
                            let suggestion = Suggestion::with_same_display_and_replacement(
                                subcmd.name.clone(),
                                subcmd.description.as_ref().cloned(),
                                SuggestionType::Subcommand,
                                subcmd.priority.into(),
                            );
                            MatchedSuggestion::new(suggestion, match_type)
                        })
                }))
                .sorted_by(MatchedSuggestion::cmp_by_display)
                .collect();
                return (results, false);
            }
        }
        _ => {}
    }

    (Default::default(), false)
}

/// Finds the last positional or named argument (an argument for a flag) in the line and generates
/// suggestions based on the type of the argument.
#[allow(clippy::too_many_arguments)]
async fn suggestions_for_last_argument(
    tokens_without_last_editing: &[&str],
    shell_command: &mut ShellCommand,
    cursor: &Span,
    command_signature: &Command,
    session_env_vars: Option<&HashMap<String, String>>,
    options: &CompleterOptions,
    ctx: &dyn CompletionContext,
) -> (Vec<MatchedSuggestion>, bool) {
    if shell_command.args.ending_whitespace.is_some() {
        add_extra_positional(shell_command, cursor);
    }

    // Find the last positional and named value within the command that the user entered.
    // Whichever ends last is the value we're trying to complete on.
    let last_positional = shell_command.last_positional();
    let last_named_value = shell_command.last_named_argument();

    let results = match (last_positional, last_named_value) {
        (Some(last_positional), Some(last_named_arg)) => {
            if last_positional.span.end() > last_named_arg.span.end() {
                // If there is a positional and a named argument (option), we want to
                // complete on the option's arguments only if the option is variadic.
                // Otherwise, the option's arguments are already satisfied and we
                // should complete the positional.
                // TODO(CORE-646): If the option is variadic, the user could be trying
                // to complete the option's arguments or the positional argument, so
                // we should show suggestions for both.
                let option_with_arguments = command_signature
                    .options
                    .iter()
                    .find(|opt| opt.has_name(last_named_arg.item.name))
                    .filter(|opt| opt.arguments.iter().any(|arg| arg.is_variadic()));
                let arguments = option_with_arguments
                    .map_or(&command_signature.arguments, |opt| &opt.arguments);

                complete_positional(
                    &shell_command,
                    arguments,
                    &command_signature.subcommands,
                    last_positional.item,
                    tokens_without_last_editing,
                    session_env_vars,
                    options,
                    ctx,
                )
                .await
            } else {
                complete_option(
                    command_signature,
                    last_named_arg.item.name,
                    last_named_arg.item.parsed_token,
                    tokens_without_last_editing,
                    session_env_vars,
                    options,
                    ctx,
                )
                .await
            }
        }
        (Some(last_positional), None) => {
            complete_positional(
                &shell_command,
                &command_signature.arguments,
                &command_signature.subcommands,
                last_positional.item,
                tokens_without_last_editing,
                session_env_vars,
                options,
                ctx,
            )
            .await
        }
        (None, Some(last_named_arg)) => {
            complete_option(
                command_signature,
                last_named_arg.item.name,
                last_named_arg.item.parsed_token,
                tokens_without_last_editing,
                session_env_vars,
                options,
                ctx,
            )
            .await
        }
        (None, None) => Default::default(),
    };

    (results, true)
}

#[allow(clippy::too_many_arguments)]
async fn complete_option(
    command_signature: &Command,
    option_name: &str,
    parsed_token: &ParsedToken,
    tokens_without_last_editing: &[&str],
    session_env_vars: Option<&HashMap<String, String>>,
    options: &CompleterOptions,
    ctx: &dyn CompletionContext,
) -> Vec<MatchedSuggestion> {
    let last_argument = command_signature
        .options
        .iter()
        .find(|opt| opt.has_name(option_name))
        .and_then(|opt| opt.arguments.last());

    match last_argument {
        None => Default::default(),
        Some(argument) => {
            generate_suggestions_for_argument(
                argument,
                parsed_token,
                tokens_without_last_editing,
                session_env_vars,
                options,
                ctx,
            )
            .await
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn complete_positional(
    shell_command: &&mut ShellCommand,
    arguments: &[Argument],
    subcommands: &[Command],
    parsed_token: &ParsedToken,
    tokens_without_last_editing: &[&str],
    session_env_vars: Option<&HashMap<String, String>>,
    options: &CompleterOptions,
    ctx: &dyn CompletionContext,
) -> Vec<MatchedSuggestion> {
    let mut suggestions = Default::default();
    // Whether we should suggest subcommands or continue to complete on positionals for the current
    // command.
    let mut should_suggest_subcommands = true;

    if let Some(positionals) = &shell_command.args.positionals.as_ref() {
        if !arguments.is_empty() {
            let positional_index = positionals.len() - 1;

            let arg = match arguments
                .iter()
                .enumerate()
                .find(|(idx, arg)| idx <= &positional_index && arg.is_variadic())
            {
                None => arguments.get(positionals.len() - 1),
                Some((_, arg)) => {
                    // If the argument is optional, it's possible the user is actually trying to
                    // enter a subcommand.
                    should_suggest_subcommands = arg.optional;
                    Some(arg)
                }
            };

            if let Some(arg) = arg {
                suggestions = generate_suggestions_for_argument(
                    arg,
                    parsed_token,
                    tokens_without_last_editing,
                    session_env_vars,
                    options,
                    ctx,
                )
                .await;
            }
        }
    }

    // Append subcommand suggestions to all argument suggestions.
    if should_suggest_subcommands {
        suggestions.extend(
            subcommands
                .iter()
                .filter_map(|subcmd| {
                    options
                        .match_strategy
                        .get_match_type(parsed_token.as_str(), subcmd.name.as_str())
                        .map(|match_type| {
                            let suggestion = Suggestion::with_same_display_and_replacement(
                                subcmd.name.clone(),
                                subcmd.description.as_ref().cloned(),
                                SuggestionType::Subcommand,
                                subcmd.priority.into(),
                            );
                            MatchedSuggestion::new(suggestion, match_type)
                        })
                })
                .sorted_by(MatchedSuggestion::cmp_by_display),
        );
    }
    suggestions
}

async fn generate_suggestions_for_argument(
    argument: &Argument,
    parsed_token: &ParsedToken,
    tokens_without_last_editing: &[&str],
    session_env_vars: Option<&HashMap<String, String>>,
    options: &CompleterOptions,
    ctx: &dyn CompletionContext,
) -> Vec<MatchedSuggestion> {
    if argument.values.is_empty()
        && matches!(
            options.fallback_strategy,
            CompletionsFallbackStrategy::FilePaths
        )
    {
        // If the signature has an argument but no argument type to generate suggestions,
        // fallback to the fallback type if any.
        return match ctx.path_completion_context() {
            Some(path_completion_context) => {
                sorted_paths_relative_to(
                    parsed_token,
                    options.match_strategy,
                    path_completion_context,
                )
                .await
            }
            None => Default::default(),
        };
    }

    // Ideally we could run these in parallel via `join_all`. However, this can cause problems
    // over SSH because a remote box could have a value of `MaxSessions` (the max number of open
    // sessions for a single connection) that is less than the number of arguments that we would
    // be trying to generate in parallel. Functionally, this would result in  `channel: open
    // failed` messages sent back over the PTY in the running sessions.
    // TODO(alokedesai): Consider using `join_all` for local sessions.
    let run_in_parallel = ctx
        .generator_context()
        .is_some_and(GeneratorContext::supports_parallel_execution);

    if run_in_parallel {
        let par_iterator = argument.values.iter().map(|argument_value| {
            generate_suggestions_for_argument_value(
                argument_value,
                parsed_token,
                tokens_without_last_editing,
                session_env_vars,
                options.match_strategy,
                ctx,
            )
        });
        futures::future::join_all(par_iterator)
            .await
            .into_iter()
            .flatten()
            .collect()
    } else {
        // These are processed in the order that argument.argument_types is defined
        // (https://github.com/warpdotdev/command-signatures/blob/5e89fb22995cd5ca9f5609d75193018a2a194c59/completion-metadata/src/fig_types.rs#L288).
        // That's why we can just flat map here without thinking about order.
        // Even if there are multiple generators, they will appear one after the other and we will
        // simply concatenate their results (extracting the non-default priorities).
        let mut completion_results = Vec::new();
        for argument_value in &argument.values {
            let matched_suggestions = generate_suggestions_for_argument_value(
                argument_value,
                parsed_token,
                tokens_without_last_editing,
                session_env_vars,
                options.match_strategy,
                ctx,
            )
            .await;
            completion_results.extend(matched_suggestions)
        }

        completion_results
    }
}

#[allow(clippy::too_many_arguments)]
async fn generate_suggestions_for_argument_value(
    argument_value: &ArgumentValue,
    parsed_token: &ParsedToken,
    tokens_without_last_editing: &[&str],
    session_env_vars: Option<&HashMap<String, String>>,
    matcher: MatchStrategy,
    ctx: &dyn CompletionContext,
) -> impl IntoIterator<Item = MatchedSuggestion> {
    // TODO(completions-v2): Implement generator support.
    match argument_value {
        ArgumentValue::Suggestion(suggestion) => {
            let warp_suggestion: Suggestion = suggestion.clone().into();
            match matcher.get_match_type(parsed_token.as_str(), warp_suggestion.display.as_str()) {
                Some(match_type) => vec![MatchedSuggestion::new(warp_suggestion, match_type)],
                None => vec![],
            }
        }
        // Even if the argument is just files, recommend both files and folders since the
        // user may want to choose a nested file.
        ArgumentValue::Template {
            type_name: TemplateType::FilesAndFolders,
            ..
        }
        | ArgumentValue::Template {
            type_name: TemplateType::Files,
            ..
        } => {
            let path_suggestions = match ctx.path_completion_context() {
                Some(path_completion_context) => {
                    sorted_paths_relative_to(parsed_token, matcher, path_completion_context).await
                }
                None => Vec::new(),
            };

            // TODO(completions-v2): Implement template filter functions.
            path_suggestions
        }
        ArgumentValue::Template {
            type_name: TemplateType::Folders,
            ..
        } => {
            let is_cd = tokens_without_last_editing
                .first()
                .is_some_and(|t| *t == "cd");
            let path_suggestions = match ctx.path_completion_context() {
                Some(path_completion_context) => {
                    if is_cd {
                        sorted_cd_directories(parsed_token, matcher, path_completion_context).await
                    } else {
                        sorted_directories_relative_to(
                            parsed_token,
                            matcher,
                            path_completion_context,
                        )
                        .await
                    }
                }
                None => Vec::new(),
            };

            // TODO(completions-v2): Implement template filter functions.
            path_suggestions
        }
        ArgumentValue::Generator(GeneratorFn::Custom(js_fn)) => {
            let (Some(js_ctx), Some(path_ctx)) = (ctx.js_context(), ctx.path_completion_context())
            else {
                return vec![];
            };
            let input = GeneratorCompletionContext {
                tokens: tokens_without_last_editing
                    .iter()
                    .map(|token| token.to_string())
                    .collect(),
                pwd: path_ctx.pwd().to_string_lossy().to_string(),
            };
            let Ok(output) = call_js_function(&input, js_fn, js_ctx).await else {
                return vec![];
            };
            let internal_suggestions = output
                .suggestions
                .into_iter()
                .map(Suggestion::from)
                .filter_map(|suggestion| {
                    let match_type =
                        matcher.get_match_type(parsed_token.as_str(), suggestion.display.as_str());
                    match_type.map(|match_type| MatchedSuggestion::new(suggestion, match_type))
                });

            if output.is_ordered {
                internal_suggestions.collect()
            } else {
                internal_suggestions
                    .sorted_by(MatchedSuggestion::cmp_by_display)
                    .collect()
            }
        }
        ArgumentValue::Generator(GeneratorFn::ShellCommand {
            script,
            post_process,
        }) => {
            let (Some(js_ctx), Some(generator_ctx)) = (ctx.js_context(), ctx.generator_context())
            else {
                return vec![];
            };
            let script = match script {
                GeneratorScript::Static(script) => Cow::Borrowed(script),
                GeneratorScript::Dynamic(js_fn) => {
                    let input: Vec<String> = tokens_without_last_editing
                        .iter()
                        .map(|token| token.to_string())
                        .collect();
                    let Ok(script) = call_js_function(&input, js_fn, js_ctx).await else {
                        return vec![];
                    };
                    Cow::Owned(script)
                }
            };

            let Ok(output) = generator_ctx
                .execute_command_at_pwd(script.as_str(), session_env_vars.cloned())
                .await
            else {
                return vec![];
            };

            match output.status {
                CommandExitStatus::Success => {
                    let Ok(output_string) = output.to_string() else {
                        return vec![];
                    };

                    let results = match post_process {
                        Some(js_fn) => {
                            let Ok(results) = call_js_function(&output_string, js_fn, js_ctx).await
                            else {
                                return vec![];
                            };
                            results
                        }
                        None => {
                            let suggestions: Vec<signatures::Suggestion> = output_string
                                .lines()
                                .map(|line| signatures::Suggestion {
                                    value: line.to_owned(),
                                    display_value: None,
                                    description: None,
                                    priority: signatures::Priority::default(),
                                })
                                .collect();

                            GeneratorResults {
                                suggestions,
                                is_ordered: true,
                            }
                        }
                    };
                    let internal_suggestions = results
                        .suggestions
                        .into_iter()
                        .map(Into::into)
                        .filter_map(|suggestion: Suggestion| {
                            let match_type = matcher
                                .get_match_type(parsed_token.as_str(), suggestion.display.as_str());
                            match_type
                                .map(|match_type| MatchedSuggestion::new(suggestion, match_type))
                        });

                    if results.is_ordered {
                        internal_suggestions.collect()
                    } else {
                        internal_suggestions
                            .sorted_by(MatchedSuggestion::cmp_by_display)
                            .collect()
                    }
                }
                CommandExitStatus::Failure => {
                    vec![]
                }
            }
        }
        _ => {
            // TODO(completions-v2): Implement suggestion generation for ArgumentValue::RootCommand.
            vec![]
        }
    }
}

impl From<signatures::Suggestion> for Suggestion {
    fn from(suggestion: signatures::Suggestion) -> Self {
        let replacement: SmolStr = suggestion.value.into();
        let display = suggestion
            .display_value
            .map(Into::into)
            .unwrap_or_else(|| replacement.clone());
        Self {
            display,
            replacement,
            description: suggestion.description,
            suggestion_type: SuggestionType::Argument,
            file_type: None,
            is_abbreviation: false,
            priority: suggestion.priority.into(),
            // TODO(completions-v2): Implement these fields for V2 Suggestions.
            override_icon: None,
            is_hidden: false,
        }
    }
}

impl From<Suggestion> for signatures::Suggestion {
    fn from(suggestion: Suggestion) -> signatures::Suggestion {
        signatures::Suggestion {
            value: suggestion.replacement.into(),
            display_value: Some(suggestion.display.into()),
            description: suggestion.description,
            priority: suggestion.priority.into(),
            // TODO(completions-v2): Implement these fields for V2 Suggestions.
            // icon: suggestion.override_icon,
            // is_hidden: suggestion.is_hidden,
        }
    }
}

//! Contains the legacy implementation of argument suggestion generation that depends on the legacy
//! command signature struct (`warp_command_signatures::Signature`).
use std::{borrow::Cow, collections::HashMap};

use itertools::Itertools;
use smol_str::SmolStr;
use warp_command_signatures::{
    Argument, ArgumentType, DynamicCompletionData, Generator, GeneratorProcess, Signature,
    Template, TemplateFilter, TemplateType,
};
use warp_core::features::FeatureFlag;
use warp_util::path::ShellFamily;

use crate::completer::{
    context::CompletionContext,
    engine::{
        self,
        path::{
            sorted_cd_directories, sorted_directories_relative_to, sorted_paths_relative_to,
            EngineFileType,
        },
    },
    matchers::MatchStrategy,
    suggest::{
        CompleterOptions, CompletionsFallbackStrategy, MatchedSuggestion, Suggestion,
        SuggestionType,
    },
    CommandExitStatus, GeneratorContext, LocationType,
};

use crate::meta::{Span, Spanned};
use crate::parsers::{
    ClassifiedCommand, ParseError, ParseErrorReason, ParsedToken, SignatureAtTokenIndex,
};

use crate::parsers::{
    hir::{Command, ShellCommand},
    ArgumentError::{MissingMandatoryPositional, MissingValueForName, UnexpectedArgument},
};

use super::add_extra_positional;

#[allow(clippy::too_many_arguments)]
pub async fn complete(
    line: &str,
    tokens_from_command: &[&str],
    classified_command: ClassifiedCommand,
    found_signature: Option<SignatureAtTokenIndex<'_>>,
    location: &Spanned<LocationType>,
    parsed_argument: &ParsedToken,
    session_env_vars: Option<&HashMap<String, String>>,
    options: &CompleterOptions,
    ctx: &dyn CompletionContext,
) -> Vec<MatchedSuggestion> {
    let mut suggestions = Default::default();

    // True if and only if we called the complete function.
    let mut arg_has_spec = false;
    if let Some(found_signature) = found_signature {
        if let Command::Classified(mut shell_command) = classified_command.command {
            suggestions = match classified_command.error {
                Some(error) => {
                    let (results, complete_called) = suggestions_for_parse_error(
                        error,
                        &mut shell_command,
                        tokens_from_command,
                        &classified_command.env_vars,
                        session_env_vars,
                        &location.span,
                        found_signature.signature,
                        found_signature.dynamic_completion_data,
                        line,
                        options,
                        ctx,
                    )
                    .await;
                    arg_has_spec = complete_called;
                    results
                }
                None => {
                    let (results, complete_called) = suggestions_for_last_argument(
                        &mut shell_command,
                        tokens_from_command,
                        line.ends_with(char::is_whitespace),
                        &classified_command.env_vars,
                        session_env_vars,
                        &location.span,
                        found_signature.signature,
                        found_signature.dynamic_completion_data,
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
            .await
            .into_iter()
            .collect();
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
/// 3) Unexpected argument: An extra argument was supplied. This extra argument
///    could be the prefix of a subcommand, so we suggest subcommands that start
///    with the value of the extra argument.
#[allow(clippy::too_many_arguments)]
async fn suggestions_for_parse_error(
    root_err: ParseError,
    shell_command: &mut ShellCommand,
    tokens_from_command: &[&str],
    command_env_vars: &[String],
    session_env_vars: Option<&HashMap<String, String>>,
    cursor: &Span,
    signature: &Signature,
    dynamic_completion_data: Option<&DynamicCompletionData>,
    line: &str,
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

                let argument = signature
                    .options()
                    .iter()
                    .find(|opt| opt.has_name(name.as_str()))
                    .and_then(|opt| opt.arguments().get(missing_arg_index));

                let results = match argument {
                    None => Default::default(),
                    Some(arg) => {
                        // Complete on the exact missing argument for the flag
                        // (rather than combining the completions for _all_ of the flags' args)
                        arg_has_spec = true;
                        generate_suggestions_for_argument(
                            arg,
                            &ParsedToken::empty(),
                            tokens_from_command,
                            command_env_vars,
                            session_env_vars,
                            line.ends_with(char::is_whitespace),
                            dynamic_completion_data,
                            options,
                            ctx,
                        )
                        .await
                    }
                };

                return (results, arg_has_spec);
            } else {
                return suggestions_for_last_argument(
                    shell_command,
                    tokens_from_command,
                    line.ends_with(char::is_whitespace),
                    command_env_vars,
                    session_env_vars,
                    cursor,
                    signature,
                    dynamic_completion_data,
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
                let argument = signature
                    .arguments()
                    .get(positional_index)
                    .expect("argument should exist based on error from parser");
                let results = generate_suggestions_for_argument(
                    argument,
                    &ParsedToken::empty(),
                    tokens_from_command,
                    command_env_vars,
                    session_env_vars,
                    line.ends_with(char::is_whitespace),
                    dynamic_completion_data,
                    options,
                    ctx,
                )
                .await;
                return (results, true);
            } else {
                return suggestions_for_last_argument(
                    shell_command,
                    tokens_from_command,
                    line.ends_with(char::is_whitespace),
                    command_env_vars,
                    session_env_vars,
                    cursor,
                    signature,
                    dynamic_completion_data,
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
            if arg.span.end() == line.len() {
                // The unexpected argument could be a prefix for a subcommand.
                let prefix = arg.item.as_str();
                let results = (signature.subcommands().iter().filter_map(|subcmd| {
                    options
                        .match_strategy
                        .get_match_type(prefix, subcmd.name())
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
    shell_command: &mut ShellCommand,
    tokens_from_command: &[&str],
    has_trailing_whitespace: bool,
    command_env_vars: &[String],
    session_env_vars: Option<&HashMap<String, String>>,
    cursor: &Span,
    signature: &Signature,
    dynamic_completion_data: Option<&DynamicCompletionData>,
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
                //
                // When the flag's value was specified via '=' (e.g., --strategy=octopus),
                // the flag is fully satisfied by that single token regardless of
                // whether its argument is variadic, so skip the variadic check.
                let flag_name = last_named_arg.item.name;
                let is_eq_delimited = tokens_from_command
                    .iter()
                    .any(|t| t.split_once('=').is_some_and(|(n, _)| n == flag_name));
                let option_with_arguments = if is_eq_delimited {
                    None
                } else {
                    signature
                        .options()
                        .iter()
                        .find(|opt| opt.has_name(flag_name))
                        .filter(|opt| opt.arguments().iter().any(|arg| arg.is_variadic))
                };
                let arguments =
                    option_with_arguments.map_or(signature.arguments(), |opt| opt.arguments());

                complete_positional(
                    &shell_command,
                    tokens_from_command,
                    has_trailing_whitespace,
                    command_env_vars,
                    session_env_vars,
                    arguments,
                    signature.subcommands(),
                    dynamic_completion_data,
                    last_positional.item,
                    options,
                    ctx,
                )
                .await
            } else {
                complete_option(
                    signature,
                    dynamic_completion_data,
                    tokens_from_command,
                    has_trailing_whitespace,
                    command_env_vars,
                    session_env_vars,
                    last_named_arg.item.name,
                    last_named_arg.item.parsed_token,
                    options,
                    ctx,
                )
                .await
            }
        }
        (Some(last_positional), None) => {
            complete_positional(
                &shell_command,
                tokens_from_command,
                has_trailing_whitespace,
                command_env_vars,
                session_env_vars,
                signature.arguments(),
                signature.subcommands(),
                dynamic_completion_data,
                last_positional.item,
                options,
                ctx,
            )
            .await
        }
        (None, Some(last_named_arg)) => {
            complete_option(
                signature,
                dynamic_completion_data,
                tokens_from_command,
                has_trailing_whitespace,
                command_env_vars,
                session_env_vars,
                last_named_arg.item.name,
                last_named_arg.item.parsed_token,
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
    signature: &Signature,
    dynamic_completion_data: Option<&DynamicCompletionData>,
    tokens_from_command: &[&str],
    has_trailing_whitespace: bool,
    command_env_vars: &[String],
    session_env_vars: Option<&HashMap<String, String>>,
    option_name: &str,
    parsed_token: &ParsedToken,
    options: &CompleterOptions,
    ctx: &dyn CompletionContext,
) -> Vec<MatchedSuggestion> {
    let last_argument = signature
        .options()
        .iter()
        .find(|opt| opt.has_name(option_name))
        .and_then(|opt| opt.arguments().last());

    match last_argument {
        None => Default::default(),
        Some(argument) => {
            generate_suggestions_for_argument(
                argument,
                parsed_token,
                tokens_from_command,
                command_env_vars,
                session_env_vars,
                has_trailing_whitespace,
                dynamic_completion_data,
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
    tokens_from_command: &[&str],
    has_trailing_whitespace: bool,
    command_env_vars: &[String],
    session_env_vars: Option<&HashMap<String, String>>,
    arguments: &[Argument],
    subcommands: &[Signature],
    dynamic_completion_data: Option<&DynamicCompletionData>,
    parsed_token: &ParsedToken,
    options: &CompleterOptions,
    ctx: &dyn CompletionContext,
) -> Vec<MatchedSuggestion> {
    let mut suggestions = Default::default();
    // Whether we should suggest subcommands or continue to complete on positionals for the current
    // command.
    let mut suggest_subcommands = true;

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
                    // If the argument is required, we shouldn't continue to suggest subcommands. If
                    // the argument is optional, we will show completions for the current argument
                    // and possible subcommands for the current command.
                    suggest_subcommands = !arg.is_required();
                    Some(arg)
                }
            };

            if let Some(arg) = arg {
                suggestions = generate_suggestions_for_argument(
                    arg,
                    parsed_token,
                    tokens_from_command,
                    command_env_vars,
                    session_env_vars,
                    has_trailing_whitespace,
                    dynamic_completion_data,
                    options,
                    ctx,
                )
                .await;
            }
        }
    }

    // Since all the required positionals have been satisfied, suggest subcommands.
    // Note that since we're extending, subcommands will come after the arguments here,
    // so no need to do a final sort at the end.
    if suggest_subcommands {
        suggestions.extend(
            subcommands
                .iter()
                .filter_map(|subcmd| {
                    options
                        .match_strategy
                        .get_match_type(parsed_token.as_str(), subcmd.name())
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

#[allow(clippy::too_many_arguments)]
async fn generate_suggestions_for_argument(
    argument: &Argument,
    parsed_token: &ParsedToken,
    tokens_from_command: &[&str],
    command_env_vars: &[String],
    session_env_vars: Option<&HashMap<String, String>>,
    has_trailing_whitespace: bool,
    dynamic_completion_data: Option<&DynamicCompletionData>,
    options: &CompleterOptions,
    ctx: &dyn CompletionContext,
) -> Vec<MatchedSuggestion> {
    // If the argument is a top level command (such as `sudo {ARG}`), suggest top-level command
    // suggestions if the command doesn't have any existing argument types.
    if argument.is_command && argument.argument_types.is_empty() {
        return engine::command_suggestions(ctx, options.match_strategy, parsed_token).await;
    }

    if argument.argument_types.is_empty()
        && matches!(
            options.fallback_strategy,
            CompletionsFallbackStrategy::FilePaths
        )
    {
        // If the signature has an argument but no argument type to generate suggestions,
        // fallback to the fallback type if any.
        return match ctx.path_completion_context() {
            Some(path_completion_context) => sorted_paths_relative_to(
                parsed_token,
                options.match_strategy,
                path_completion_context,
            )
            .await
            .into_iter()
            .collect(),
            None => Default::default(),
        };
    }

    // These are processed in the order that argument.argument_types is defined
    // (https://github.com/warpdotdev/command-signatures/blob/5e89fb22995cd5ca9f5609d75193018a2a194c59/completion-metadata/src/fig_types.rs#L288).
    // That's why we can just flat map here without thinking about order.
    // Even if there are multiple generators, they will appear one after the other and we will
    // simply concatenate their results (extracting the non-default priorities).
    let run_in_parallel = ctx
        .generator_context()
        .is_some_and(GeneratorContext::supports_parallel_execution);

    if run_in_parallel {
        let par_iterator = argument.argument_types.iter().map(|argument_type| {
            generate_suggestions_for_argument_type(
                argument,
                argument_type,
                parsed_token,
                tokens_from_command,
                has_trailing_whitespace,
                command_env_vars,
                session_env_vars,
                options.match_strategy,
                dynamic_completion_data,
                ctx,
            )
        });
        futures::future::join_all(par_iterator)
            .await
            .into_iter()
            .flatten()
            .collect()
    } else {
        let mut completion_results = vec![];
        for argument_type in &argument.argument_types {
            let matched_suggestions = generate_suggestions_for_argument_type(
                argument,
                argument_type,
                parsed_token,
                tokens_from_command,
                has_trailing_whitespace,
                command_env_vars,
                session_env_vars,
                options.match_strategy,
                dynamic_completion_data,
                ctx,
            )
            .await;
            completion_results.extend(matched_suggestions);
        }

        completion_results
    }
}

#[allow(clippy::too_many_arguments)]
async fn generate_suggestions_for_argument_type(
    argument: &Argument,
    argument_type: &ArgumentType,
    parsed_token: &ParsedToken,
    tokens_from_command: &[&str],
    has_trailing_whitespace: bool,
    command_env_vars: &[String],
    session_env_vars: Option<&HashMap<String, String>>,
    matcher: MatchStrategy,
    dynamic_completion_data: Option<&DynamicCompletionData>,
    ctx: &dyn CompletionContext,
) -> impl IntoIterator<Item = MatchedSuggestion> {
    match argument_type {
        ArgumentType::Suggestion(suggestion) => {
            let warp_suggestion: Suggestion = suggestion.clone().into();
            match matcher.get_match_type(parsed_token.as_str(), warp_suggestion.display.as_str()) {
                Some(match_type) => vec![MatchedSuggestion::new(warp_suggestion, match_type)],
                None => vec![],
            }
        }
        // Even if the argument is just files, recommend both files and folders since the
        // user may want to choose a nested file.
        ArgumentType::Template(Template {
            type_name: TemplateType::FilesAndFolders,
            filter_name,
        })
        | ArgumentType::Template(Template {
            type_name: TemplateType::Files { .. },
            filter_name,
        }) => {
            let path_suggestions = match ctx.path_completion_context() {
                Some(path_completion_context) => {
                    sorted_paths_relative_to(parsed_token, matcher, path_completion_context).await
                }
                None => Vec::new(),
            };

            match filter_name.as_ref().and_then(|filter_name| {
                argument.filter_template_by_name(
                    dynamic_completion_data.map(DynamicCompletionData::filters),
                    filter_name,
                )
            }) {
                Some(filter) => filter_path_suggestions(filter, path_suggestions.into_iter()),
                None => path_suggestions.into_iter().collect(),
            }
        }
        ArgumentType::Template(Template {
            type_name: TemplateType::Folders { .. },
            filter_name,
        }) => {
            let is_cd = tokens_from_command.first().is_some_and(|t| *t == "cd");
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

            match filter_name.as_ref().and_then(|filter_name| {
                argument.filter_template_by_name(
                    dynamic_completion_data.map(DynamicCompletionData::filters),
                    filter_name,
                )
            }) {
                Some(filter) => filter_path_suggestions(filter, path_suggestions.into_iter()),
                None => path_suggestions.into_iter().collect(),
            }
        }
        ArgumentType::Generator(generator_name) => {
            let generator = match argument.generator_by_name(
                dynamic_completion_data.map(DynamicCompletionData::generators),
                generator_name,
            ) {
                None => return vec![],
                Some(generator) => generator,
            };

            let shell_command = shell_command(
                generator,
                tokens_from_command,
                has_trailing_whitespace,
                ctx.shell_family().unwrap_or(ShellFamily::Posix),
                command_env_vars,
            );

            let Some(generator_context) = ctx.generator_context() else {
                return vec![];
            };

            let Ok(output) = generator_context
                .execute_command_at_pwd(&shell_command, session_env_vars.cloned())
                .await
            else {
                return vec![];
            };

            match output.status {
                CommandExitStatus::Success => {
                    let Ok(output_string) = output.to_string() else {
                        return vec![];
                    };

                    let results = generator.on_complete(&output_string);

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
        _ => vec![],
    }
}

fn shell_command<'a>(
    generator: &'a Generator,
    tokens: &[&str],
    has_trailing_whitespace: bool,
    shell_family: ShellFamily,
    command_env_vars: &[String],
) -> Cow<'a, str> {
    let shell = if cfg!(windows) && FeatureFlag::RunGeneratorsWithCmdExe.is_enabled() {
        warp_command_signatures::Shell::CmdExe
    } else {
        match shell_family {
            ShellFamily::Posix => warp_command_signatures::Shell::Posix,
            ShellFamily::PowerShell => warp_command_signatures::Shell::Powershell,
        }
    };

    match &generator.process {
        GeneratorProcess::ShellCommand(command) => command.build(shell),
        GeneratorProcess::CommandFromTokens(command_generator) => {
            command_generator(tokens, has_trailing_whitespace, command_env_vars)
                .build(shell)
                .to_string()
                .into()
        }
    }
}

fn filter_path_suggestions<'a>(
    filter: &'a TemplateFilter,
    path_suggestions: impl Iterator<Item = MatchedSuggestion> + 'a,
) -> Vec<MatchedSuggestion> {
    path_suggestions
        .filter_map(|path_suggestion| {
            // Note that we need to read out these fields from the Suggestion struct before converting into command signature Suggestion struct
            // because they only belong to the app. We should write these fields back once we finished filtering
            let suggestion_type = path_suggestion.suggestion_type();

            let file_type = path_suggestion
                .suggestion
                .file_type
                .unwrap_or(EngineFileType::File);
            filter
                .filter(path_suggestion.suggestion.into(), file_type.into())
                .map(|filter_suggestion| {
                    let mut suggestion: Suggestion = filter_suggestion.into();
                    suggestion.suggestion_type = suggestion_type;
                    MatchedSuggestion::new(suggestion, path_suggestion.match_type)
                })
        })
        .collect()
}

impl From<warp_command_signatures::Suggestion> for Suggestion {
    /// Convert the `warp_command_signatures::Suggestion`s (which are meant to map
    /// 1:1 to the `completer::Suggestion`s)
    fn from(suggestion: warp_command_signatures::Suggestion) -> Self {
        let exact_string: SmolStr = suggestion.exact_string.into();
        let display = suggestion
            .display_name
            .map(Into::into)
            .unwrap_or_else(|| exact_string.clone());
        Self {
            display,
            replacement: exact_string,
            description: suggestion.description,
            suggestion_type: SuggestionType::Argument,
            priority: suggestion.priority.into(),
            override_icon: suggestion.icon,
            is_hidden: suggestion.is_hidden,
            file_type: None,
            is_abbreviation: false,
        }
    }
}

impl From<Suggestion> for warp_command_signatures::Suggestion {
    fn from(suggestion: Suggestion) -> warp_command_signatures::Suggestion {
        warp_command_signatures::Suggestion {
            exact_string: suggestion.display.as_ref().into(),
            description: suggestion.description,
            priority: suggestion.priority.into(),
            icon: suggestion.override_icon,
            is_hidden: suggestion.is_hidden,
            display_name: Some(suggestion.display.as_ref().into()),
        }
    }
}

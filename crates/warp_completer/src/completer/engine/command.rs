use itertools::Itertools;

use super::path::{sorted_directories_relative_to, sorted_paths_relative_to};
use crate::completer::context::CompletionContext;
use crate::completer::matchers::MatchStrategy;
use crate::completer::suggest::{MatchedSuggestion, Priority, Suggestion, SuggestionType};
use crate::completer::{TopLevelCommandCaseSensitivity, engine, get_path_separators};
use crate::parsers::ParsedToken;

/// Generates top-level completion results based on the fragment of text that is entered into the
/// buffer. We use the following algorithm to generate suggestions, which is also the same as ZSH:
/// 1) If the fragment is clearly a path prefix (./, .., etc), _only_ suggest files.
/// 2) If the fragment starts with a `$`, _only_ suggest environment variables.
/// 3) Otherwise, suggest all top-level executables the user can run. If the shell has
///    "autocd" enabled (cd'ing into a directory without specifying the `cd` command) also suggest
///    filepaths.
pub async fn complete(
    context: &dyn CompletionContext,
    matcher: MatchStrategy,
    parsed_token: &ParsedToken,
) -> Vec<MatchedSuggestion> {
    // If the command trying to be matched contains "/", we're actually in a place where we need
    // to be suggesting paths instead of trying to read the command registry at all.
    if parsed_token
        .as_str()
        .contains(get_path_separators(context).all)
    {
        return match context.path_completion_context() {
            Some(path_completion_context) => {
                sorted_paths_relative_to(parsed_token, matcher, path_completion_context)
                    .await
                    .into_iter()
                    .collect()
            }
            None => Default::default(),
        };
    }

    // The buffer starts with a `$`, suggest environment variables.
    if parsed_token.as_str().starts_with('$') {
        return if let Some(env_vars) = context.environment_variable_names() {
            engine::variable_suggestions(matcher.to_owned(), env_vars, parsed_token)
        } else {
            Default::default()
        };
    }

    let command_suggestions = sorted_top_level_commands(parsed_token.as_str(), matcher, context);

    // If `cd`ing into a directory without entering `cd` is enabled, also suggest directories.
    // Note that the directories will be listed _after_ the top level commands.
    if context.shell_supports_autocd().unwrap_or(false)
        && let Some(path_completion_context) = context.path_completion_context()
    {
        return command_suggestions
            .into_iter()
            .chain(
                sorted_directories_relative_to(parsed_token, matcher, path_completion_context)
                    .await
                    .into_iter(),
            )
            .collect();
    }

    command_suggestions.into_iter().collect()
}

/// Returns a lexicographically ordered `Vec` of suggestions for top-level commands based on the
/// `partial` buffer and the `context`'s top-level commands, aliases, and abbreviations.
fn sorted_top_level_commands(
    partial: &str,
    matcher: MatchStrategy,
    context: &dyn CompletionContext,
) -> Vec<MatchedSuggestion> {
    context
        .top_level_commands()
        .filter_map(|command| {
            matcher.get_match_type(partial, command).map(|match_type| {
                let suggestion = command_suggestion(command, context).unwrap_or_else(|| {
                    Suggestion::with_same_display_and_replacement(
                        command,
                        None,
                        SuggestionType::Command(context.command_case_sensitivity()),
                        Priority::default(),
                    )
                });
                MatchedSuggestion::new(suggestion, match_type)
            })
        })
        .sorted_by(MatchedSuggestion::cmp_by_display)
        .collect()
}

/// Return a top-level command's `Suggestion`
fn command_suggestion(command: &str, context: &dyn CompletionContext) -> Option<Suggestion> {
    cfg_if::cfg_if! {
        if #[cfg(feature = "v2")] {
            let command_suggestion =
                context
                    .command_registry()
                    .get_signature(command)
                    .map(|signature| {
                        Suggestion::with_same_display_and_replacement(
                            signature.command.name.clone(),
                            signature.command.description.as_ref().cloned(),
                            // TODO(CORE-2795) This needs to be overrideable by
                            // signature.parser_directives.always_case_insensitive
                            SuggestionType::Command(context.command_case_sensitivity()),
                            signature.command.priority.into(),
                        )

                    });
        } else {
            let command_suggestion =
                context
                    .command_registry()
                    .signature(command)
                    .map(|signature| {
                    let case_sensitivity = if signature.parser_directives.always_case_insensitive {
                        TopLevelCommandCaseSensitivity::CaseInsensitive
                    } else {
                        context.command_case_sensitivity()
                    };
                        Suggestion::with_same_display_and_replacement(
                            signature.name.clone(),
                            signature.description.as_ref().cloned(),
                            SuggestionType::Command(case_sensitivity),
                            signature.priority.into(),
                        )
                    });
        }
    }

    command_suggestion
        .or_else(|| alias_suggestion(command, context))
        .or_else(|| function_suggestion(command, context))
        .or_else(|| builtin_suggestion(command, context))
        .or_else(|| abbr_suggestion(command, context))
}

fn alias_suggestion(command: &str, context: &dyn CompletionContext) -> Option<Suggestion> {
    context.alias_command(command).map(|value| {
        Suggestion::with_same_display_and_replacement(
            command,
            Some(format!("Alias for \"{value}\"")),
            SuggestionType::Command(context.alias_and_function_case_sensitivity()),
            Priority::default(),
        )
    })
}

fn function_suggestion(command: &str, context: &dyn CompletionContext) -> Option<Suggestion> {
    context.functions()?.get(command).map(|_| {
        Suggestion::with_same_display_and_replacement(
            command,
            Some("Shell function".to_string()),
            SuggestionType::Command(context.alias_and_function_case_sensitivity()),
            Priority::default(),
        )
    })
}

fn builtin_suggestion(command: &str, context: &dyn CompletionContext) -> Option<Suggestion> {
    context.builtins()?.get(command).map(|_| {
        Suggestion::with_same_display_and_replacement(
            command,
            Some("Shell builtin".to_string()),
            SuggestionType::Command(TopLevelCommandCaseSensitivity::CaseSensitive),
            Priority::default(),
        )
    })
}

fn abbr_suggestion(command: &str, context: &dyn CompletionContext) -> Option<Suggestion> {
    context
        .abbreviations()?
        .get(command)
        .map(|value| Suggestion::new_for_abbreviation(command, value, Priority::default()))
}

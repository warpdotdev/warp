use std::collections::HashSet;

use async_recursion::async_recursion;
use itertools::Itertools;

use crate::completer::{CompletionContext, TopLevelCommandCaseSensitivity};
use crate::meta::Span;
use crate::parsers::simple::parse_for_completions;
#[cfg(not(feature = "v2"))]
use crate::parsers::SignatureAtTokenIndex;
use crate::parsers::{classify_command, ClassifiedCommand};
#[cfg(feature = "v2")]
use crate::signatures::Command;

/// This is used to limit how many times we re-run the completer once we
/// evaluate an alias to prevent infinite recursion.
const ALIAS_EXPANSION_MAX_INDIRECTION_LIMIT: usize = 5;

struct NumAliasExpansionsAttempted(usize);

impl NumAliasExpansionsAttempted {
    fn new() -> Self {
        NumAliasExpansionsAttempted(0)
    }

    fn increment(self) -> Self {
        NumAliasExpansionsAttempted(self.0 + 1)
    }

    fn reached_limit(&self) -> bool {
        self.0 >= ALIAS_EXPANSION_MAX_INDIRECTION_LIMIT
    }
}

struct EvaluatedAliases<'a> {
    pub top_level_aliases: HashSet<&'a str>,
    pub command_aliases: HashSet<String>,
}

impl EvaluatedAliases<'_> {
    fn new() -> Self {
        Self {
            top_level_aliases: HashSet::new(),
            command_aliases: HashSet::new(),
        }
    }
}

/// The result of alias expansion.
/// Note that fields other than `expanded_command_line` are specific to completions, which
/// throw away info about earlier commands in cases like "command1 && command2", and
/// generally should not be used outside the completions engine.
pub struct AliasExpansionResult<'a> {
    /// The entire raw expanded command line after alias expansions.
    pub expanded_command_line: String,
    /// The signature we should use for completions after alias expansion.
    #[cfg(feature = "v2")]
    pub(crate) signature_for_completions: Option<&'a Command>,
    #[cfg(not(feature = "v2"))]
    pub(crate) signature_for_completions: Option<SignatureAtTokenIndex<'a>>,
    /// The tokens from the expanded_command_line to be used for completions, without any env vars.
    pub(crate) tokens_from_command: Vec<String>,
    /// The classified command from expanded_command_line to be used for completions.
    /// TODO(roland) This should be pub(crate) once command validation handles "command1 && command2" case
    pub classified_command: Option<ClassifiedCommand>,
}

/// Expands aliases in the `line`, returning a tuple containing `line` with the
/// alias replaced along with the alias itself, but ONLY if there is a space after the alias.
///
/// For example, given alias kgp="kubectl get pod", "kgp" will NOT be expanded, but "kgp " will.
///
/// TODO(INT-830): handle alias expansion with multiple commands. Current alias expansion logic
/// was implemented for completions, where only the last command needs to be expanded.
/// For example, given "kgp && kgp ", we expect it to expand to "kubectl get pod && kubectl get pod ",
/// but currently it will expand to "kgp && kubectl get pod ".
pub async fn expand_command_aliases<'a>(
    line: &str,
    parse_quotes_as_literals: bool,
    ctx: &'a dyn CompletionContext,
) -> AliasExpansionResult<'a> {
    expand_command_aliases_internal(
        line,
        &mut EvaluatedAliases::new(),
        NumAliasExpansionsAttempted::new(),
        parse_quotes_as_literals,
        ctx,
    )
    .await
}

#[async_recursion]
async fn expand_command_aliases_internal<'a>(
    line: &str,
    evaluated_aliases: &mut EvaluatedAliases<'a>,
    num_alias_expansions_attempted: NumAliasExpansionsAttempted,
    parse_quotes_as_literals: bool,
    ctx: &'a dyn CompletionContext,
) -> AliasExpansionResult<'a> {
    // Lite command we are completing upon. Note that this includes the full command including
    // parts like environment variable assignment.
    let command_to_complete =
        parse_for_completions(line, ctx.escape_char(), parse_quotes_as_literals)
            .unwrap_or_default();

    // The vector of tokens in the command. Note that the tokens are modified later to remove
    // any environment variable assignment token for completion generation.
    // TODO(kevin): We are using a mutable vector here so we don't need to allocate
    // multiple times. But this makes the code harder to read. We should think about
    // a better way to represent it.
    let mut tokens_from_command = command_to_complete
        .parts
        .iter()
        .map(|s| s.as_str())
        .collect_vec();

    let classified_command = classify_command(
        command_to_complete.clone(),
        &mut tokens_from_command,
        ctx.command_registry(),
        ctx.command_case_sensitivity(),
    );

    let command_name_span = classified_command
        .as_ref()
        .map(|command| command.command.name_span());

    let alias_expansion_reached_limit = num_alias_expansions_attempted.reached_limit();
    if alias_expansion_reached_limit {
        log::warn!("Alias expansion reached limit!");
    }
    if let Some((command_with_expanded_alias, alias)) = command_name_span
        .and_then(|command_name_span| expand_root_command_alias(line, command_name_span, ctx))
    {
        // If the command has an alias that was evaluated, recursively expand more aliases using the new expanded line.
        if !evaluated_aliases.top_level_aliases.contains(alias) && !alias_expansion_reached_limit {
            // Ensure we don't evaluate the alias again so we don't end up in an infinite
            // loop of alias expansion for the same alias name.
            evaluated_aliases.top_level_aliases.insert(alias);
            return expand_command_aliases_internal(
                command_with_expanded_alias.as_str(),
                evaluated_aliases,
                num_alias_expansions_attempted.increment(),
                parse_quotes_as_literals,
                ctx,
            )
            .await;
        }
    }

    cfg_if::cfg_if! {
        if #[cfg(feature = "v2")] {
            use crate::signatures::{get_matching_signature_for_input, get_matching_signature_for_tokenized_input};

            let found_signature =
                // Short-circuit if this alias has already been expanded, or if we've reached
                // the recursion limit for alias expansion.
                //
                // This is a fork of the logic in the `else` branch, which assumes alias expansion
                // is implemented.
                if evaluated_aliases.command_aliases.contains(line) || alias_expansion_reached_limit {
                    command_name_span.and_then(|command_name_span| {
                        // We get the line based on the classified command name span.
                        // We use this span because it factors in complicated commands like
                        // "git commit && cd".
                        let command_line = &line[command_name_span.start()..];
                        get_matching_signature_for_input(command_line, ctx.command_registry())
                    }).map(|(signature, _)| signature)
                } else {
                    // TODO(completions-v2): Implement command-specific alias expansion.
                    get_matching_signature_for_tokenized_input(
                            &tokens_from_command,
                            command_to_complete.post_whitespace.is_some(),
                            ctx.command_registry()).map(|(signature, _)| signature)
                };
        } else {
            use crate::signatures::registry::SignatureResult;

            let found_signature =
                if evaluated_aliases.command_aliases.contains(line) || alias_expansion_reached_limit {
                    // If we have already expanded the current command alias before or we have reached the
                    // indirection limit, don't consider alias expansion here to avoid infinite looping.
                    command_name_span.and_then(|command_name_span| {
                        // We get the line based on the classified command name span.
                        // We use this span because it factors in complicated commands like
                        // "git commit && cd".
                        let command_line = &line[command_name_span.start()..];
                        ctx.command_registry().signature_from_line(command_line, ctx.command_case_sensitivity())
                    })
                } else {
                    match ctx
                        .command_registry()
                        .signature_with_alias_expansion(
                            &tokens_from_command,
                            command_to_complete.post_whitespace.is_some(),
                            ctx,
                        )
                        .await
                    {
                        SignatureResult::Success(found_signature) => Some(found_signature),
                        SignatureResult::NeedAliasExpansion(mut new_command) => {
                            // Push an additional empty space
                            if command_to_complete.post_whitespace.is_some() {
                                new_command.push(' ');
                            }
                            // We expanded only tokens_from_command, which did not include any earlier commands and env vars in the line.
                            // Add them back in.
                            if let Some(classified_command) = classified_command {
                                let start_of_command = classified_command.command.name_span().start();
                                if start_of_command > 0 {
                                    new_command = format!("{}{}", &line[..start_of_command], new_command);
                                }
                            }
                            evaluated_aliases.command_aliases.insert(line.into());
                            return expand_command_aliases_internal(
                                new_command.as_str(),
                                evaluated_aliases,
                                num_alias_expansions_attempted.increment(),
                                parse_quotes_as_literals,
                                ctx,
                            )
                            .await;
                        }
                        SignatureResult::None => None,
                    }
                };
        }
    }
    AliasExpansionResult {
        expanded_command_line: line.to_string(),
        signature_for_completions: found_signature,
        tokens_from_command: tokens_from_command
            .into_iter()
            .map(|s| s.to_owned())
            .collect_vec(),
        classified_command,
    }
}

/// Expands a root command alias in `input`, returning a tuple containing `input` with the
/// alias replaced along with the alias itself, but ONLY if there is a space after the alias.
///
/// For example, given alias kgp="kubectl get pod", "kgp" will NOT be expanded, but "kgp " will.
///
/// command_name_span contains the span for the root command to check aliases against.
/// No other potential aliases in `input` will be checked.
fn expand_root_command_alias<'a>(
    input: &str,
    command_name_span: Span,
    context: &'a dyn CompletionContext,
) -> Option<(String, &'a str)> {
    // There must be a space after command_name_span for the alias to be expanded.
    if input
        .get(command_name_span.end()..command_name_span.end() + 1)
        .is_none_or(|s| s != " ")
    {
        return None;
    }
    let command_span_str = command_name_span.slice(input);
    context.aliases().find_map(|(alias, value)| {
        // Check if we have an alias for the command_name_span and a space after it.
        let matches = if context.alias_and_function_case_sensitivity()
            == TopLevelCommandCaseSensitivity::CaseInsensitive
        {
            alias.eq_ignore_ascii_case(command_span_str)
        } else {
            alias == command_span_str
        };
        if matches {
            let before_alias = &input[..command_name_span.start()];
            let after_alias = &input[command_name_span.end()..];
            return Some((format!("{before_alias}{value}{after_alias}"), alias));
        }
        None
    })
}

#[cfg(test)]
#[path = "alias_tests.rs"]
mod test;

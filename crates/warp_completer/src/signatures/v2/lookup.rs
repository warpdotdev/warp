//! This module contains functions for looking up a matching command signature from tokenized and
//! untokenized input.
use itertools::Itertools;

use super::{registry::CommandRegistry, Command};

/// Returns the highest-precedence matching `Command` signature object for the given `input`, if
/// any, along with the index of the token in `input` matched to the returned `Command`.
///
/// Subcommands take precedence over parent commands.
///
/// Note that a token in the input must have trailing whitespace (e.g. marking it as "completed")
/// to be eligible to be matched to a command signature. So, for example, if the input does not
/// contain trailing whitespace, the last token is not considered in the matching algorithm.
/// Otherwise, if one subcommand is a prefix of another subcommand, we could mistakenly eagerly
/// return the signature for the shorter subcommand even if the intent was to continue typing to
/// enter the longer subcommand.
///
/// Practically, this means that for input "test_command test_subcommand", even if there is a
/// subcommand signature for "test_subcommand", this returns the signature for "test_command",
/// because it's assumed "test_subcommand" may still be edited.
pub fn get_matching_signature_for_input<'a>(
    input: &str,
    registry: &'a CommandRegistry,
) -> Option<(&'a Command, usize)> {
    let input_tokens = input.split_whitespace().collect_vec();
    get_matching_signature_for_tokenized_input(
        &input_tokens,
        input.ends_with(char::is_whitespace),
        registry,
    )
}

/// Returns the highest-precedence matching `Command` signature object for the given tokenized
/// `input`, if any. This is equivalent to `get_matching_signature_input` above, except input is
/// tokenized (e.g. given as an array of string tokens, which were assumed to be space-delimited in
/// the original input). Because input is tokenized, the caller needs to explicitly specify whether
/// the original input had trailing whitespace to determine if the last token is eligible for use
/// in the matching algorithm.
///
/// See comments on `get_matching_signature_input` for more details.
pub fn get_matching_signature_for_tokenized_input<'a>(
    input_tokens: &[&str],
    has_trailing_whitespace: bool,
    registry: &'a CommandRegistry,
) -> Option<(&'a Command, usize)> {
    let (first_token, remaining_tokens) = input_tokens.split_first()?;

    // Find the top level signature.
    registry.get_signature(first_token).map(|signature| {
        deepest_matching_subcommand_signature(
            remaining_tokens,
            &signature.command,
            0,
            has_trailing_whitespace,
        )
    })
}

/// Given a parent `command_signature`, resolves the most specific (deepest) subcommand that
/// the user has entered in `input_tokens`, skipping over any flags that appear
/// before the subcommand name (e.g. `kubectl -n kube-system get` resolves to `get`).
///
/// Returns the matched `Command` along with the index of its token in `input_tokens`.
/// If no subcommand is found, `command_signature` itself is returned at `current_token_index`.
///
/// The last token is only eligible for a subcommand match when `has_trailing_whitespace` is
/// true, i.e. the user has finished typing it.
fn deepest_matching_subcommand_signature<'a>(
    input_tokens: &[&str],
    command_signature: &'a Command,
    mut current_token_index: usize,
    has_trailing_whitespace: bool,
) -> (&'a Command, usize) {
    if input_tokens.is_empty() {
        return (command_signature, current_token_index);
    }

    // Save the starting index before we begin scanning for subcommands.
    // If we skip past flags but never find a subcommand beyond them, we
    // return this index so that `parse_internal_command` treats the flags
    // as arguments to be parsed rather than swallowing them into the
    // command name.
    let subcommand_search_start_index = current_token_index;

    while current_token_index < input_tokens.len() {
        let is_last_token = current_token_index == input_tokens.len() - 1;
        let token = input_tokens[current_token_index];

        // Try to match the token against a subcommand.
        let subcommand_match = command_signature.subcommands.iter().find(|subcommand| {
            let token_matches_subcommand = token == subcommand.name.as_str();
            if is_last_token {
                // If this is the last token, treat the subcommand signature as a match
                // if there is trailing whitespace, which affirms the user's intent to use
                // that subcommand. If there is no trailing whitespace, the user may still
                // be in the process of editing that subcommand (or specifying a different
                // subcommand of which the current token is a prefix).
                token_matches_subcommand && has_trailing_whitespace
            } else {
                token_matches_subcommand
            }
        });

        if let Some(subcommand) = subcommand_match {
            return deepest_matching_subcommand_signature(
                input_tokens,
                subcommand,
                current_token_index + 1,
                has_trailing_whitespace,
            );
        }

        // If the token is a flag (starts with '-'), try to skip past it and its arguments
        // to continue looking for subcommands. This handles cases like
        // `kubectl -n kube-system get pods` where flags appear before subcommands.
        if token.starts_with('-') {
            if let Some(option) = command_signature
                .options
                .iter()
                .find(|opt| opt.name.iter().any(|name| name == token))
            {
                // Skip the flag's arguments (non-switch options consume the next token(s)).
                // Clamp to the number of argument tokens actually present to avoid
                // advancing past the end of input_tokens (e.g. `kubectl -n ` with no
                // namespace value).
                let num_args = option.arguments.iter().filter(|arg| !arg.optional).count();
                let available = input_tokens.len().saturating_sub(current_token_index + 1);
                current_token_index += num_args.min(available);
            }
            // Advance past the flag token itself.
            current_token_index += 1;
            continue;
        }

        // Token is not a subcommand or a recognized flag; stop searching.
        break;
    }

    // No subcommand was found beyond any skipped flags, so return the
    // start index. This ensures the caller's parser still sees those
    // flag tokens and can process them as flag arguments.
    (command_signature, subcommand_search_start_index)
}

#[cfg(test)]
#[path = "lookup_tests.rs"]
mod test;

//! This module contains functions for looking up a matching command signature from tokenized and
//! untokenized input.
use std::collections::HashSet;

use itertools::Itertools;

use super::Command;
use super::registry::CommandRegistry;

/// The result of resolving the deepest matching `Command` signature for some tokenized input.
#[derive(Debug, Clone)]
pub struct MatchedSignature<'a> {
    /// The most specific (deepest) `Command` signature that matched the input.
    pub command: &'a Command,

    /// The index of `command`'s own token within the tokenized input it was matched against.
    pub token_index: usize,

    /// Sibling keyword `Command`s that remain eligible for suggestion alongside
    /// `command.subcommands`.
    ///
    /// This is only populated when `command` was resolved as one of several repeatable,
    /// order-independent keywords under an ancestor `Command` with `repeatable_keywords` set
    /// (see [`Command::repeatable_keywords`]). It contains the sibling keywords under that
    /// ancestor which have not already been used elsewhere in the input, and is otherwise empty.
    pub eligible_sibling_keywords: Vec<&'a Command>,
}

/// Returns the highest-precedence matching `Command` signature object for the given `input`, if
/// any.
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
) -> Option<MatchedSignature<'a>> {
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
) -> Option<MatchedSignature<'a>> {
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
/// If `command_signature.repeatable_keywords` is set, this delegates to
/// `matching_repeatable_keyword_signature` instead, since `command_signature`'s subcommands are
/// then treated as a set of repeatable, order-independent keywords rather than a set of mutually
/// exclusive subcommands.
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
) -> MatchedSignature<'a> {
    if command_signature.repeatable_keywords {
        return matching_repeatable_keyword_signature(
            input_tokens,
            command_signature,
            current_token_index,
            has_trailing_whitespace,
        );
    }

    if input_tokens.is_empty() {
        return MatchedSignature {
            command: command_signature,
            token_index: current_token_index,
            eligible_sibling_keywords: Vec::new(),
        };
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
    MatchedSignature {
        command: command_signature,
        token_index: subcommand_search_start_index,
        eligible_sibling_keywords: Vec::new(),
    }
}

/// Given a parent `command_signature` whose `repeatable_keywords` flag is set, scans
/// `input_tokens` (starting at `current_token_index`) for a sequence of its repeatable,
/// order-independent keyword subcommands (e.g. `iif`, `from`, `table`, ... under `ip rule add`),
/// skipping over any of `command_signature`'s flags and each matched keyword's own required
/// argument tokens along the way.
///
/// Unlike `deepest_matching_subcommand_signature`, this does not descend into the matched
/// keyword's own subtree to look for further matches. Instead, since keywords are siblings
/// rather than a nested hierarchy, it keeps scanning `command_signature`'s subcommands for
/// further (not-yet-used) keyword matches until the input is exhausted or a token is
/// encountered that isn't a recognized flag or an unused keyword.
///
/// Returns the most recently matched keyword `Command`, along with any sibling keywords under
/// `command_signature` that have not yet been used and so remain eligible for suggestion. If no
/// keyword was matched at all, `command_signature` itself is returned at the entry token index,
/// consistent with `deepest_matching_subcommand_signature`'s behavior when no subcommand match is
/// found.
fn matching_repeatable_keyword_signature<'a>(
    input_tokens: &[&str],
    command_signature: &'a Command,
    mut current_token_index: usize,
    has_trailing_whitespace: bool,
) -> MatchedSignature<'a> {
    let entry_token_index = current_token_index;
    let mut used_keyword_names: HashSet<&str> = HashSet::new();
    let mut last_matched_keyword: Option<(&'a Command, usize)> = None;

    while current_token_index < input_tokens.len() {
        let is_last_token = current_token_index == input_tokens.len() - 1;
        let token = input_tokens[current_token_index];

        // Try to match the token against an unused keyword subcommand.
        let keyword_match = command_signature.subcommands.iter().find(|subcommand| {
            let token_matches_keyword = token == subcommand.name.as_str()
                && !used_keyword_names.contains(subcommand.name.as_str());
            if is_last_token {
                // As in `deepest_matching_subcommand_signature`, only treat the last token as a
                // committed keyword match if there's trailing whitespace; otherwise the user may
                // still be editing it.
                token_matches_keyword && has_trailing_whitespace
            } else {
                token_matches_keyword
            }
        });

        if let Some(keyword) = keyword_match {
            used_keyword_names.insert(keyword.name.as_str());
            last_matched_keyword = Some((keyword, current_token_index + 1));

            // Skip past the keyword's own required argument tokens (e.g. `eth0` after `iif`) so
            // we can keep scanning for its sibling keywords.
            let num_args = keyword.arguments.iter().filter(|arg| !arg.optional).count();
            let available = input_tokens.len().saturating_sub(current_token_index + 1);
            current_token_index += num_args.min(available) + 1;
            continue;
        }

        // If the token is a flag (starts with '-'), skip it and its arguments (if recognized),
        // then keep scanning for keywords, mirroring the flag-skipping behavior in
        // `deepest_matching_subcommand_signature`.
        if token.starts_with('-') {
            if let Some(option) = command_signature
                .options
                .iter()
                .find(|opt| opt.name.iter().any(|name| name == token))
            {
                let num_args = option.arguments.iter().filter(|arg| !arg.optional).count();
                let available = input_tokens.len().saturating_sub(current_token_index + 1);
                current_token_index += num_args.min(available);
            }
            current_token_index += 1;
            continue;
        }

        // Token is not a flag or an unused keyword (e.g. it's still being typed, or it's the
        // last matched keyword's own argument value); stop scanning.
        break;
    }

    match last_matched_keyword {
        Some((keyword, token_index)) => MatchedSignature {
            command: keyword,
            token_index,
            eligible_sibling_keywords: command_signature
                .subcommands
                .iter()
                .filter(|subcommand| !used_keyword_names.contains(subcommand.name.as_str()))
                .collect(),
        },
        None => MatchedSignature {
            command: command_signature,
            token_index: entry_token_index,
            eligible_sibling_keywords: Vec::new(),
        },
    }
}

#[cfg(test)]
#[path = "lookup_tests.rs"]
mod test;

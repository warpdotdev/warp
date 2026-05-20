use std::collections::HashSet;

use lazy_static::lazy_static;
use warp_completer::ParsedTokensSnapshot;

/// The percentage of input tokens that can be described by our completion engine before
/// we consider the input as a shell command. This could be tuned.
const DETECT_AS_COMMAND_THRESHOLD: f32 = 0.5;

/// Threshold for the case when we have a low number of input tokens and require a higher
/// confidence level. This could be tuned.
const DETECT_AS_COMMAND_LOW_TOKEN_THRESHOLD: f32 = 0.7;

lazy_static! {
    /// One-off commands / keywords that should trigger a shell command classification.
    ///
    /// `claude`, `codex`, and `gemini` are not actually _really_ one-off shell command keywords,
    /// but false-positive NL classifications for these inputs (where the user was trying to use
    /// claude code, codex CLI, or gemini CLI) suck, because the user often thinks we're
    /// intentionally trying to push them away from those CLIs into Agent Mode, so we mitigate the
    /// risk by always treating as shell.
    static ref ONE_OFF_SHELL_COMMAND_KEYWORDS: HashSet<&'static str> = HashSet::from(["#", "echo", "man", "sudo", "claude", "codex", "gemini"]);

    static ref ONE_OFF_NATURAL_LANGUAGE_WORDS: HashSet<&'static str> = HashSet::from(["hello", "hi", "hey", "hola", "thanks", "explain", "yes", "no", "what", "nice", "1. "]);

    /// A set of words that should trigger an AI classification if they are the entire input
    /// and the input is a follow-up to an agent response.
    static ref AGENT_FOLLOW_UP_INPUTS: HashSet<&'static str> = HashSet::from(["yes", "continue", "do it"]);
}

pub fn is_agent_follow_up_input(input: &str) -> bool {
    AGENT_FOLLOW_UP_INPUTS.contains(input)
}

pub fn is_one_off_shell_command_keyword(word: &str) -> bool {
    ONE_OFF_SHELL_COMMAND_KEYWORDS.contains(word)
}

/// Returns true if the word is a one-off natural language word or a prefix of a one-off natural language word.
pub fn is_one_off_natural_language_word_or_prefix(word: &str) -> bool {
    is_one_off_natural_language_word(word) || is_prefix_of_natural_language_word(word)
}

// Returns true if the word is a one-off natural language word.
pub fn is_one_off_natural_language_word(word: &str) -> bool {
    ONE_OFF_NATURAL_LANGUAGE_WORDS.contains(word)
}

/// Checks if the input string is a prefix of any word in the ONE_OFF_NATURAL_LANGUAGE_WORDS set.
/// This helps with progressive typing detection to avoid mode flipping.
pub fn is_prefix_of_natural_language_word(input: &str) -> bool {
    // input is already lowercase from caller
    ONE_OFF_NATURAL_LANGUAGE_WORDS
        .iter()
        .any(|word| word.starts_with(input))
}

/// nld_heuristic_v1: current prod, use check_if_token_has_shell_syntax and conditional threshold on input length
/// nld_heuristic_v2: rm check_if_token_has_shell_syntax and pin threshold to be 1 for all input
pub async fn is_likely_shell_command(
    input: &ParsedTokensSnapshot,
    word_tokens_count: usize,
) -> bool {
    const YIELD_BATCH_SIZE: usize = 5;
    let use_nld_heuristic_v2 = cfg!(feature = "nld_heuristic_v2");

    let mut likely_command_token_count = 0;
    let total_token_count = input.parsed_tokens.len();
    let mut is_first_token_command = false;
    log::debug!(
        "is_likely_shell_command start: use_nld_heuristic_v2={use_nld_heuristic_v2}, total_token_count={total_token_count}, word_tokens_count={word_tokens_count}"
    );
    for (idx, token) in input.parsed_tokens.iter().enumerate() {
        // Periodically, yield to the executor so this task can be aborted if
        // requested.
        if idx % YIELD_BATCH_SIZE == 0 {
            futures_lite::future::yield_now().await;
        }
        // Early return if we encounter a one-off command / keyword at the beginning of the line.
        if token.token_index == 0 && ONE_OFF_SHELL_COMMAND_KEYWORDS.contains(&token.token.as_str())
        {
            log::debug!(
                "is_likely_shell_command result=true: first token is one-off shell keyword, use_nld_heuristic_v2={use_nld_heuristic_v2}"
            );
            return true;
        }
        let check_if_token_has_shell_syntax = !use_nld_heuristic_v2
            && natural_language_detection::check_if_token_has_shell_syntax(token.token.as_str());
        log::debug!(
            "is_likely_shell_command token: token_index={}, token_description_is_some={}, check_if_token_has_shell_syntax={check_if_token_has_shell_syntax}, use_nld_heuristic_v2={use_nld_heuristic_v2}",
            token.token_index,
            token.token_description.is_some()
        );
        if token.token_description.is_some() || check_if_token_has_shell_syntax {
            likely_command_token_count += 1;
        }

        if token.token_index == 0 {
            is_first_token_command = token.token_description.is_some();
        }
    }

    // When token count is lower than 2, we should make sure all tokens
    // are matching the target classification category.
    let command_threshold = if use_nld_heuristic_v2 || total_token_count <= 2 {
        1.0
    } else if total_token_count <= 4 {
        DETECT_AS_COMMAND_LOW_TOKEN_THRESHOLD
    } else {
        DETECT_AS_COMMAND_THRESHOLD
    };

    // Classify as shell if:
    // 1) We hit significant threshold of likely shell command tokens.
    // 2) When there are fewer than 3 tokens, the first token is a valid top-level command.
    let is_likely_shell_command = likely_command_token_count
        >= (total_token_count as f32 * command_threshold) as usize
        || (word_tokens_count < 3 && is_first_token_command);
    log::debug!(
        "is_likely_shell_command result={is_likely_shell_command}: use_nld_heuristic_v2={use_nld_heuristic_v2}, likely_command_token_count={likely_command_token_count}, total_token_count={total_token_count}, word_tokens_count={word_tokens_count}, command_threshold={command_threshold}, is_first_token_command={is_first_token_command}"
    );

    is_likely_shell_command
}

/// Returns true if the first token is a command that is installed on the system.
pub fn is_installed_binary(input: &ParsedTokensSnapshot) -> bool {
    input
        .parsed_tokens
        .first()
        .map(|token| token.token_description.is_some())
        .unwrap_or(false)
}

#[cfg(all(test, any(feature = "nld_heuristic_v1", feature = "nld_heuristic_v2")))]
#[path = "util_tests.rs"]
mod tests;

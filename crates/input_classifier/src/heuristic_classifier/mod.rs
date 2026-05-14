use std::borrow::Cow;

use async_trait::async_trait;
use itertools::Itertools as _;
use natural_language_detection::natural_language_words_score;
use warp_completer::ParsedTokensSnapshot;

use crate::{
    ClassificationResult, Context, InputClassificationDecision, InputClassifier,
    InputDecisionSource, InputType,
    parser::parse_query_into_tokens,
    util::{
        is_installed_binary, is_likely_shell_command, is_one_off_natural_language_word_or_prefix,
    },
};

/// Minimum number of tokens users' input should have before kicking off input detection
/// to switch from AI input to command input.
/// This could be tuned.
const MINIMUM_COMMAND_DETECTION_TOKEN_LENGTH: u8 = 2;

/// Minimum number of tokens users' input should have before kicking off input detection
/// to switch from command input to AI input.
/// This could be tuned.
const MINIMUM_NATURAL_LANGUAGE_DETECTION_TOKEN_LENGTH: u8 = 2;

/// The percentage of input tokens that can be recognized as a natural language word before
/// we consider the input as natural language. This could be tuned.
const DETECT_AS_NATURAL_LANGUAGE_THRESHOLD: f32 = 0.6;

/// Threshold for the case when we have a low number of input tokens and require a higher
/// confidence level. This could be tuned.
const DETECT_AS_NATURAL_LANGUAGE_LOW_TOKEN_THRESHOLD: f32 = 0.8;

const END_TOKEN_COMPLETE_KEYS: &[char] = &[' ', '?', '!', '.', '"', ','];

/// A classifier that uses simple heuristics to determine the type of input.
pub struct HeuristicClassifier;

#[cfg_attr(not(target_family = "wasm"), async_trait)]
#[cfg_attr(target_family = "wasm", async_trait(?Send))]
impl InputClassifier for HeuristicClassifier {
    async fn detect_input_decision(
        &self,
        input: ParsedTokensSnapshot,
        context: &Context,
    ) -> InputClassificationDecision {
        let word_tokens = parse_query_into_tokens(input.buffer_text.as_str());
        let total_word_token_count = word_tokens.len();

        if total_word_token_count == 1
            && is_one_off_natural_language_word_or_prefix(&word_tokens[0].to_lowercase())
        {
            return InputClassificationDecision::new(
                InputType::AI,
                InputDecisionSource::OneOffWhitelist,
            );
        }

        if is_likely_shell_command(&input, total_word_token_count).await {
            return InputClassificationDecision::new(
                InputType::Shell,
                InputDecisionSource::ShellHeuristic,
            );
        }

        let input_type = self
            .classify_input(input, context)
            .await
            .map(|result| result.to_input_type())
            .unwrap_or(context.current_input_type);

        InputClassificationDecision::new(
            input_type,
            InputDecisionSource::NldClassifierFallbackHeuristic,
        )
    }

    async fn classify_input(
        &self,
        input: warp_completer::ParsedTokensSnapshot,
        context: &Context,
    ) -> anyhow::Result<super::ClassificationResult> {
        let word_tokens = parse_query_into_tokens(input.buffer_text.as_str());

        // Try autodetecting both including and not including the last token,
        // since we aren't sure if the user is done typing. If either case is
        // detected as AI input, set to AI input.
        let result = natural_language_detection_heuristic(
            input.clone(),
            word_tokens.clone(),
            context.current_input_type,
            false,
        )
        .await;

        if matches!(result.to_input_type(), InputType::AI) {
            return Ok(result);
        }

        Ok(natural_language_detection_heuristic(
            input,
            word_tokens,
            context.current_input_type,
            true,
        )
        .await)
    }
}

/// Given some input text and current input type, return what type of input we think it is
/// using a heuristic.
async fn natural_language_detection_heuristic(
    input: ParsedTokensSnapshot,
    word_tokens: Vec<String>,
    current_input_type: InputType,
    include_last_token: bool,
) -> ClassificationResult {
    let word_tokens_count = word_tokens.len();

    let min_token_length = if matches!(current_input_type, InputType::AI) {
        MINIMUM_COMMAND_DETECTION_TOKEN_LENGTH
    } else {
        MINIMUM_NATURAL_LANGUAGE_DETECTION_TOKEN_LENGTH
    };

    if min_token_length > word_tokens_count as u8 {
        return ClassificationResult::pure_shell();
    }

    let mut word_tokens = word_tokens.into_iter().map(Cow::Owned).collect_vec();

    // If the last token is not complete AND we are configured to not always include the last token, do not consider it
    // in the natural language classifier. When the total word tokens have length less than 3, we also shouldn't pop the last
    // token as this could cause misclassification on any top command we didn't parse.
    let last_token_is_complete = input.buffer_text.ends_with(END_TOKEN_COMPLETE_KEYS);
    if !include_last_token && !last_token_is_complete && word_tokens.len() > 2 {
        word_tokens.pop();
    }

    let updated_word_token_count = word_tokens.len();
    let likely_english_token_count =
        natural_language_words_score(word_tokens, is_installed_binary(&input));

    // When token count is lower than 3, we should make sure all tokens
    // are matching the target classification category.
    let threshold = if updated_word_token_count <= 3 {
        1.0
    } else if updated_word_token_count <= 4 {
        DETECT_AS_NATURAL_LANGUAGE_LOW_TOKEN_THRESHOLD
    } else {
        DETECT_AS_NATURAL_LANGUAGE_THRESHOLD
    };

    if likely_english_token_count >= (updated_word_token_count as f32 * threshold) as usize {
        return ClassificationResult::pure_ai();
    }

    ClassificationResult::pure_shell()
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;

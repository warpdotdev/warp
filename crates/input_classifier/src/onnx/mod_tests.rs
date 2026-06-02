use anyhow::Result;
use futures::executor::block_on;
use warp_completer::meta::SpannedItem;
use warp_completer::{ParsedTokenData, ParsedTokensSnapshot};

use super::*;

struct FailingInferenceRunner;

impl InferenceRunner for FailingInferenceRunner {
    fn run_inference(&self, _input: &ParsedTokensSnapshot) -> Result<ClassificationResult> {
        Err(anyhow::anyhow!("inference failed"))
    }
}

fn parsed_input_without_descriptions(buffer_text: &str) -> ParsedTokensSnapshot {
    let mut next_search_start = 0;
    let parsed_tokens = buffer_text
        .split_whitespace()
        .enumerate()
        .map(|(token_index, token)| {
            let token_start =
                buffer_text[next_search_start..].find(token).unwrap() + next_search_start;
            let token_end = token_start + token.len();
            next_search_start = token_end;

            ParsedTokenData {
                token: token.to_string().spanned((token_start, token_end)),
                token_index,
                token_description: None,
            }
        })
        .collect();

    ParsedTokensSnapshot {
        buffer_text: buffer_text.to_owned(),
        parsed_tokens,
    }
}

#[test]
fn test_inference_error_reports_current_input_fallback_source() {
    block_on(async move {
        let classifier = OnnxClassifier {
            inference_runner: Box::new(FailingInferenceRunner),
            has_panicked: HasPanicked::new(),
        };
        let context = Context {
            current_input_type: InputType::AI,
            is_agent_follow_up: false,
        };
        let input = parsed_input_without_descriptions("help migrate database");

        let decision = classifier.detect_input_type(input, &context).await;

        assert_eq!(
            decision,
            InputClassificationResult::new(
                InputType::AI,
                InputClassifierDecisionSource::InputClassifierFallbackCurrentInput,
            )
        );
    });
}

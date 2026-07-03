use std::io::Write as _;

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use fasttext::FastText;
use rust_embed::RustEmbed;
use tempfile::NamedTempFile;

use crate::{
    ClassificationResult, Context, InputClassifier, InputType,
    parser::parse_query_into_tokens,
    util::{is_likely_shell_command, is_one_off_natural_language_word},
};

#[derive(Clone, Copy, RustEmbed)]
#[folder = "models/fasttext"]
struct Models;

pub struct FasttextClassifier {
    classifier: FastText,
}

impl FasttextClassifier {
    pub fn new() -> Result<Self> {
        Ok(Self {
            classifier: Self::load_classifier()?,
        })
    }

    fn load_classifier() -> Result<FastText> {
        let model_bytes = Models::get("cmd_lang_classifier_v4.bin")
            .ok_or_else(|| anyhow!("Model file not found"))?
            .data;
        let mut temp_file = NamedTempFile::new()?;
        temp_file.write_all(model_bytes.as_ref())?;
        let model_path = temp_file.path();
        let mut classifier = FastText::new();
        classifier
            .load_model(
                model_path
                    .to_str()
                    .ok_or_else(|| anyhow!("Invalid model path"))?,
            )
            .map_err(|_| anyhow!("Failed to load fasttext classifier"))?;
        log::info!("Successfully loaded fasttext classifier");
        Ok(classifier)
    }
}

#[async_trait]
impl InputClassifier for FasttextClassifier {
    async fn detect_input_type(
        &self,
        input: warp_completer::ParsedTokensSnapshot,
        context: &Context,
    ) -> InputType {
        let word_tokens = parse_query_into_tokens(input.buffer_text.as_str());

        let total_word_token_count = word_tokens.len();

        if total_word_token_count == 1 {
            if is_one_off_natural_language_word(&word_tokens[0].to_lowercase()) {
                return InputType::AI;
            }

            // Prevent flickering for short input
            return context.current_input_type;
        }

        if is_likely_shell_command(&input, total_word_token_count).await {
            return InputType::Shell;
        }

        self.classify_input(input, context)
            .await
            .map(|result| result.to_input_type())
            .unwrap_or(context.current_input_type)
    }

    async fn classify_input(
        &self,
        input: warp_completer::ParsedTokensSnapshot,
        context: &Context,
    ) -> anyhow::Result<ClassificationResult> {
        if let Ok(classification_result) =
            classify_input_with_fasttext(&self.classifier, input.buffer_text.as_str())
        {
            return Ok(classification_result);
        }

        super::HeuristicClassifier
            .classify_input(input, context)
            .await
    }
}

/// Classify the current input text with the FastText classifier
fn classify_input_with_fasttext(
    classifier: &FastText,
    input: &str,
) -> anyhow::Result<ClassificationResult> {
    anyhow::ensure!(!input.trim().is_empty(), "cannot classify empty input");

    let predictions = classifier
        .predict(input, 2, 0.0)
        .map_err(|err| anyhow!("Failed to classify input: {err}"))?;

    let mut classification_result = ClassificationResult {
        p_shell: 0.0,
        p_ai: 0.0,
    };

    for prediction in predictions {
        if prediction.label.contains("terminal_command") {
            classification_result.p_shell = prediction.prob;
        } else if prediction.label.contains("natural_language") {
            classification_result.p_ai = prediction.prob;
        }
    }

    Ok(classification_result)
}

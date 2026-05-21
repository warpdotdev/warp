#[cfg(feature = "onnx_candle")]
mod candle;
#[cfg(feature = "onnx_ort")]
mod ort;

use std::borrow::Cow;

use anyhow::Result;
use async_trait::async_trait;
use rust_embed::RustEmbed;
use warp_completer::ParsedTokensSnapshot;

use crate::{
    ClassificationResult, Context, InputClassifier, InputType,
    parser::parse_query_into_tokens,
    util::{
        is_likely_shell_command, is_one_off_natural_language_word, is_one_off_shell_command_keyword,
    },
};

#[derive(Clone, Copy, RustEmbed)]
#[folder = "models/onnx"]
#[include = "bert_tiny_tokenizer.json"]
#[cfg_attr(feature = "nld_classifier_v1", include = "bert_tiny_v1.onnx")]
#[cfg_attr(feature = "nld_classifier_v2", include = "bert_tiny_v2.onnx")]
struct Models;

#[derive(Copy, Clone, Debug)]
pub enum Model {
    BertTinyV1,
    BertTinyV2,
}

impl Model {
    fn bytes(&self) -> Option<Cow<'static, [u8]>> {
        Models::get(self.model_path()).map(|file| file.data)
    }

    fn tokenizer_bytes(&self) -> Option<Cow<'static, [u8]>> {
        Models::get(self.tokenizer_path()).map(|file| file.data)
    }

    fn model_path(&self) -> &'static str {
        match self {
            Model::BertTinyV1 => "bert_tiny_v1.onnx",
            Model::BertTinyV2 => "bert_tiny_v2.onnx",
        }
    }

    fn tokenizer_path(&self) -> &'static str {
        match self {
            Model::BertTinyV1 | Model::BertTinyV2 => "bert_tiny_tokenizer.json",
        }
    }
}

pub struct OnnxClassifier {
    inference_runner: Box<dyn InferenceRunner>,
    has_panicked: HasPanicked,
}

impl OnnxClassifier {
    pub fn new(_model: Model) -> Result<Self> {
        #[cfg(feature = "onnx_candle")]
        match candle::InferenceRunner::new(_model).map(Box::new) {
            Ok(inference_runner) => {
                return Ok(Self {
                    inference_runner,
                    has_panicked: HasPanicked::new(),
                });
            }
            Err(err) => log::warn!("Failed to initialize candle inference runner: {err:#}"),
        }

        #[cfg(feature = "onnx_ort")]
        match ort::InferenceRunner::new(_model).map(Box::new) {
            Ok(inference_runner) => {
                return Ok(Self {
                    inference_runner,
                    has_panicked: HasPanicked::new(),
                });
            }
            Err(err) => log::warn!("Failed to initialize ort inference runner: {err:#}"),
        }

        Err(anyhow::anyhow!("No onnx inference engine enabled"))
    }
}

#[cfg_attr(not(target_family = "wasm"), async_trait)]
#[cfg_attr(target_family = "wasm", async_trait(?Send))]
impl InputClassifier for OnnxClassifier {
    async fn detect_input_type(&self, input: ParsedTokensSnapshot, context: &Context) -> InputType {
        let word_tokens = parse_query_into_tokens(input.buffer_text.as_str());

        let total_word_token_count = word_tokens.len();

        // Start by applying some simple heuristics before running the full classifier.
        if let Some(first_word) = word_tokens.first() {
            let first_word = first_word.to_lowercase();

            // If the input is a single word and the word is one of a specific set of words, classify it as AI
            if word_tokens.len() == 1 && is_one_off_natural_language_word(&first_word) {
                return InputType::AI;
            }

            // If the first token is one of a specific set of shell command keywords (e.g.: echo or sudo),
            // we should classify it as shell.
            if is_one_off_shell_command_keyword(&first_word) {
                return InputType::Shell;
            }
        }

        if is_likely_shell_command(&input, total_word_token_count).await {
            return InputType::Shell;
        }

        // Otherwise, defer all decision-making to the model.
        self.classify_input(input, context)
            .await
            .map(|result| result.to_input_type())
            .unwrap_or(context.current_input_type)
    }

    async fn classify_input(
        &self,
        input: warp_completer::ParsedTokensSnapshot,
        _context: &Context,
    ) -> anyhow::Result<ClassificationResult> {
        // If we ever panicked while running inference, we should fall back to the heuristic classifier.
        if self.has_panicked.has_panicked() {
            return crate::heuristic_classifier::HeuristicClassifier
                .classify_input(input, _context)
                .await;
        }

        // Given that we only can get here if we have never panicked, we don't have to
        // worry about attempting to use an inference runner that is in an invalid state
        // due to recovering after catching a panic unwind.
        let inference_runner = std::panic::AssertUnwindSafe(&self.inference_runner);

        let input_ref = &input;
        match std::panic::catch_unwind(move || {
            let start = instant::Instant::now();
            let result = inference_runner.run_inference(input_ref);
            let duration = start.elapsed();
            let duration_ms = duration.as_secs_f32() * 1000.0;

            match result {
                Ok(result) => {
                    log::debug!(
                        "Inference took {duration_ms:.2} ms; p_shell: {:.5}, p_ai: {:.5}",
                        result.p_shell,
                        result.p_ai
                    );
                    Ok(result)
                }
                Err(e) => {
                    log::error!("Failed to run inference (took {duration_ms:.2} ms): {e:#}");
                    Err(e)
                }
            }
        }) {
            Ok(result) => result,
            Err(_) => {
                log::error!(
                    "Caught panic while running inference; falling back to heuristic classifier."
                );
                self.has_panicked.on_panic();
                crate::heuristic_classifier::HeuristicClassifier
                    .classify_input(input, _context)
                    .await
            }
        }
    }
}

trait InferenceRunner: 'static + Send + Sync {
    fn run_inference(&self, input: &ParsedTokensSnapshot) -> Result<ClassificationResult>;
}

/// A simple structure that we can use to track whether the ONNX classifier has panicked.
struct HasPanicked {
    inner: std::sync::Once,
}

impl HasPanicked {
    fn new() -> Self {
        Self {
            inner: std::sync::Once::new(),
        }
    }

    fn on_panic(&self) {
        // Mark the classifier as having panicked.
        self.inner.call_once(|| {});
    }

    fn has_panicked(&self) -> bool {
        // Return true if the classifier has panicked.
        self.inner.is_completed()
    }
}

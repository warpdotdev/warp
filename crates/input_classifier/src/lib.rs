mod heuristic_classifier;
mod input_type;
#[cfg(feature = "onnx")]
mod onnx;
mod parser;
pub mod test_utils;
pub mod util;

use async_trait::async_trait;
pub use heuristic_classifier::HeuristicClassifier;
pub use input_type::InputType;
#[cfg(feature = "onnx")]
pub use onnx::{Model as OnnxModel, OnnxClassifier};
use serde::{Deserialize, Serialize};

/// Sources produced by the input classifier pipeline.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum InputClassifierDecisionSource {
    // Classification result coming from Onnx classifier
    InputClassifier,
    // Classification result coming from fall back heuristic classifier when onnx classifier panicked
    InputClassifierFallbackHeuristic,
    // Classification result coming from current input type when onnx classifier failed
    InputClassifierFallbackCurrentInput,
    // Input match with ONE_OFF_NATURAL_LANGUAGE_WORDS
    NaturalLanguageOneOffAllowlist,
    // Input match with ONE_OFF_SHELL_COMMAND_KEYWORDS
    ShellCommandAllowList,
    // Classification result coming from is_likely_shell_command
    ShellHeuristic,
}

/// The detected input type along with the decision source that produced it.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputClassificationResult {
    /// The detected input type.
    pub input_type: InputType,
    /// The classifier source that produced this classification.
    pub source: InputClassifierDecisionSource,
}

impl InputClassificationResult {
    pub fn new(input_type: InputType, source: InputClassifierDecisionSource) -> Self {
        Self { input_type, source }
    }
}

/// An input classifier, which can take some parsed user input and determine
/// what type of input it is.
#[cfg_attr(not(target_family = "wasm"), async_trait)]
#[cfg_attr(target_family = "wasm", async_trait(?Send))]
pub trait InputClassifier: 'static + Send + Sync {
    async fn detect_input_type(
        &self,
        input: warp_completer::ParsedTokensSnapshot,
        context: &Context,
    ) -> InputClassificationResult;

    async fn classify_input(
        &self,
        input: warp_completer::ParsedTokensSnapshot,
        context: &Context,
    ) -> anyhow::Result<ClassificationResult>;
}

/// The result of running inference on some user input.
pub struct ClassificationResult {
    /// The probability that the input is a shell command.
    p_shell: f32,
    /// The probability that the input is a natural language query to AI.
    p_ai: f32,
    /// The classifier source that produced this classification.
    pub source: InputClassifierDecisionSource,
}

impl ClassificationResult {
    fn pure_ai(source: InputClassifierDecisionSource) -> Self {
        Self {
            p_shell: 0.0,
            p_ai: 1.0,
            source,
        }
    }

    fn pure_shell(source: InputClassifierDecisionSource) -> Self {
        Self {
            p_shell: 1.0,
            p_ai: 0.0,
            source,
        }
    }

    pub fn p_shell(&self) -> f32 {
        self.p_shell
    }

    pub fn p_ai(&self) -> f32 {
        self.p_ai
    }

    /// Returns the confidence score (0.0 to 1.0) as the maximum of the two probabilities
    pub fn confidence(&self) -> f32 {
        self.p_shell.max(self.p_ai)
    }

    pub fn to_input_type(&self) -> InputType {
        if self.p_shell > self.p_ai {
            InputType::Shell
        } else {
            InputType::AI
        }
    }
}

/// Context for the classifier.
pub struct Context {
    /// The current input type.
    pub current_input_type: InputType,
    /// Whether or not the input is a follow-up to an agent query.
    pub is_agent_follow_up: bool,
}

mod heuristic_classifier;
mod input_type;
#[cfg(feature = "onnx")]
mod onnx;
mod parser;
pub mod test_utils;
pub mod util;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

pub use heuristic_classifier::HeuristicClassifier;
pub use input_type::InputType;
#[cfg(feature = "onnx")]
pub use onnx::{Model as OnnxModel, OnnxClassifier};

/// The source of the final input type decision applied to the user input.
///
/// Variants are grouped by where in the input-type-decision pipeline they
/// originate so analytics can distinguish between app-level overrides that
/// bypass NLD entirely and decisions produced by the NLD pipeline itself.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum InputTypeAutoDetectionSource {
    /// App level settings from user actions
    AppLevelOverride(AppLevelOverride),
    /// Decision Source NLD classifier (heuristic or ML model).
    NldClassifier(NldClassifierSource),
    /// NLD whitelists/blocklists
    NldShortCircuit(NldShortCircuit),
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AppLevelOverride {
    ManualToggle,

    ShellPrefix,

    AttachmentForcedAi,

    SettingDisabled,
}

/// Sources produced by the NLD classifier pipeline (heuristic or ML model).
#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum NldClassifierSource {
    ShellHeuristic,

    NldClassifier,
    /// The NLD model was unavailable or unusable, so the heuristic fallback
    /// made the decision.
    NldClassifierFallbackHeuristic,
    /// The NLD model returned an error, so the current input type was kept.
    NldClassifierFallbackCurrentInput,
    /// The input was a one-off natural-language word. Reserved for symmetry;
    /// production emit paths use [`NldShortCircuit::OneOffWhitelist`].
    OneOffWhitelist,
}

/// Short-circuits inside the NLD pipeline that produce a decision without
/// running the full classifier model.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum NldShortCircuit {
    /// The first token matched the user-configurable autodetection denylist.
    Denylist,
    /// The input closely matched a previous shell command.
    HistoryMatch,
    OneOffWhitelist,
    AgentFollowUp,
}

impl From<AppLevelOverride> for InputTypeAutoDetectionSource {
    fn from(value: AppLevelOverride) -> Self {
        Self::AppLevelOverride(value)
    }
}

impl From<NldClassifierSource> for InputTypeAutoDetectionSource {
    fn from(value: NldClassifierSource) -> Self {
        Self::NldClassifier(value)
    }
}

impl From<NldShortCircuit> for InputTypeAutoDetectionSource {
    fn from(value: NldShortCircuit) -> Self {
        Self::NldShortCircuit(value)
    }
}

/// The result of input type detection, including the source that determined the final decision.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct InputClassificationDecision {
    pub input_type: InputType,
    pub source: InputTypeAutoDetectionSource,
}

impl InputClassificationDecision {
    pub fn new(input_type: InputType, source: InputTypeAutoDetectionSource) -> Self {
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
    ) -> InputClassificationDecision;

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
    /// The source that produced this classification.
    pub source: InputTypeAutoDetectionSource,
}

impl ClassificationResult {
    fn pure_ai(source: InputTypeAutoDetectionSource) -> Self {
        Self {
            p_shell: 0.0,
            p_ai: 1.0,
            source,
        }
    }

    fn pure_shell(source: InputTypeAutoDetectionSource) -> Self {
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

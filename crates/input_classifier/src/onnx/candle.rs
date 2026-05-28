use std::collections::HashMap;
use std::sync::OnceLock;

use anyhow::{Context as _, Result, ensure};
use candle_core::{IndexOp as _, Tensor};
use candle_onnx::onnx::ModelProto;
use prost::Message as _;
use tokenizers::Tokenizer;
use warp_completer::ParsedTokensSnapshot;

use super::{ClassificationResult, Model};
use crate::InputClassifierDecisionSource;

/// Holds the decoded ONNX model and tokenizer. Loaded lazily on first
/// inference to avoid a ~23 MB allocation spike at startup.
struct LoadedModel {
    model: ModelProto,
    tokenizer: Tokenizer,
}

pub struct InferenceRunner {
    model_spec: Model,
    loaded: OnceLock<LoadedModel>,
}

impl InferenceRunner {
    pub fn new(model: Model) -> Result<Self> {
        // Validate that the embedded model files exist without decoding them.
        // The expensive decode + tokenizer parse is deferred to first inference.
        anyhow::ensure!(
            model.bytes().is_some(),
            "Model file not found for {:?}",
            model
        );
        anyhow::ensure!(
            model.tokenizer_bytes().is_some(),
            "Tokenizer file not found for {:?}",
            model
        );
        Ok(Self {
            model_spec: model,
            loaded: OnceLock::new(),
        })
    }

    /// Returns the lazily-loaded model and tokenizer, initializing them on first
    /// call. Since the model bytes are embedded in the binary via `RustEmbed`,
    /// a load failure is deterministic and will be retried on each call.
    fn ensure_loaded(&self) -> Result<&LoadedModel> {
        if let Some(loaded) = self.loaded.get() {
            return Ok(loaded);
        }

        let model = Self::load_model(self.model_spec)?;
        let tokenizer = Self::load_tokenizer(self.model_spec)?;
        // Another thread may have initialized concurrently; use its value if so.
        let _ = self.loaded.set(LoadedModel { model, tokenizer });
        self.loaded
            .get()
            .context("loaded model should be initialized")
    }

    fn load_model(model: Model) -> Result<ModelProto> {
        let model_bytes = model.bytes().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotFound, "Model file not found")
        })?;
        let model = ModelProto::decode(model_bytes.as_ref())?;
        Ok(model)
    }

    fn load_tokenizer(model: Model) -> Result<Tokenizer> {
        let tokenizer_bytes = model.tokenizer_bytes().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotFound, "Tokenizer file not found")
        })?;
        let tokenizer = Tokenizer::from_bytes(tokenizer_bytes).map_err(|e| anyhow::anyhow!(e))?;
        Ok(tokenizer)
    }
}

impl super::InferenceRunner for InferenceRunner {
    fn run_inference(&self, input: &ParsedTokensSnapshot) -> Result<ClassificationResult> {
        let loaded = self.ensure_loaded()?;

        // Encode the input text into tokens.
        let encoding = loaded
            .tokenizer
            .encode_fast(input.buffer_text.as_str(), true)
            .map_err(|e| anyhow::anyhow!(e))?;

        // For now, we'll do all inference on the CPU.
        let device = candle_core::Device::Cpu;

        let input_ids = Tensor::new(
            encoding
                .get_ids()
                .iter()
                .map(|&x| x as i64)
                .collect::<Vec<_>>()
                .as_slice(),
            &device,
        )
        .context("failed to build input ids tensor")?;
        let attention_mask = Tensor::new(
            encoding
                .get_attention_mask()
                .iter()
                .map(|&x| x as i64)
                .collect::<Vec<_>>()
                .as_slice(),
            &device,
        )
        .context("failed to build attention mask tensor")?;

        // Run inference.
        let outputs = candle_onnx::simple_eval(
            &loaded.model,
            HashMap::from([
                ("input_ids".to_string(), input_ids.unsqueeze(0)?),
                ("attention_mask".to_string(), attention_mask.unsqueeze(0)?),
            ]),
        )
        .context("error evaluating the model")?;

        let logits = outputs.get("logits").context("failed to get logits")?;
        let probabilities = candle_nn::ops::softmax_last_dim(logits)
            .context("failed to compute softmax")?
            .i(0)
            .context("failed to get first dimension")?
            .to_vec1::<f32>()
            .context("failed to convert softmax output to vec")?;

        ensure!(probabilities.len() == 2, "expected 2 probabilities");

        Ok(ClassificationResult {
            p_ai: probabilities[0],
            p_shell: probabilities[1],
            source: InputClassifierDecisionSource::InputClassifier,
        })
    }
}

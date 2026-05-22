use std::collections::HashMap;

use anyhow::{Context as _, Result, ensure};
use candle_core::{IndexOp as _, Tensor};
use candle_onnx::onnx::ModelProto;
use prost::Message as _;
use tokenizers::Tokenizer;
use warp_completer::ParsedTokensSnapshot;

use super::{ClassificationResult, Model};
use crate::InputClassifierDecisionSource;

pub struct InferenceRunner {
    model: ModelProto,
    tokenizer: Tokenizer,
}

impl InferenceRunner {
    pub fn new(model: Model) -> Result<Self> {
        Ok(Self {
            model: Self::load_model(model)?,
            tokenizer: Self::load_tokenizer(model)?,
        })
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
        // Encode the input text into tokens.
        let encoding = self
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
            &self.model,
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

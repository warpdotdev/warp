use anyhow::{Result, ensure};
use itertools::Itertools as _;
use ort::execution_providers::CPUExecutionProvider;
use ort::session::Session;
use ort::tensor::ArrayExtensions as _;
use ort::value::Value;
use parking_lot::Mutex;
use tokenizers::Tokenizer;
use warp_completer::ParsedTokensSnapshot;

use super::{ClassificationResult, Model};
use crate::InputClassifierDecisionSource;

pub struct InferenceRunner {
    session: Mutex<Session>,
    tokenizer: Tokenizer,
}

impl InferenceRunner {
    pub fn new(model: Model) -> Result<Self> {
        Ok(Self {
            session: Self::init_session(model)?.into(),
            tokenizer: Self::load_tokenizer(model)?,
        })
    }

    fn init_session(model: Model) -> Result<Session> {
        let model_bytes = model.bytes().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotFound, "Model file not found")
        })?;
        let session = Session::builder()?
            // For now, we'll do all inference on the CPU.
            .with_execution_providers([CPUExecutionProvider::default().build()])?
            .commit_from_memory(model_bytes.as_ref())?;
        Ok(session)
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

        let input_ids = encoding.get_ids();
        let attention_mask = encoding.get_attention_mask();

        let input_ids = Value::from_array((
            [1, input_ids.len()],
            input_ids.iter().map(|&x| x as i64).collect_vec(),
        ))?;
        let attention_mask = Value::from_array((
            [1, attention_mask.len()],
            attention_mask.iter().map(|&x| x as i64).collect_vec(),
        ))?;

        let mut session = self.session.lock();
        let outputs = session.run(ort::inputs![
            "input_ids" => input_ids,
            "attention_mask" => attention_mask,
        ])?;

        let logits = &outputs[0];

        let logits = logits.try_extract_array::<f32>()?;
        let probabilities = logits.softmax(ndarray::Axis(1));

        let probabilities = probabilities.view();
        let probabilities = probabilities
            .as_slice()
            .ok_or_else(|| anyhow::anyhow!("failed to get probabilities"))?;

        ensure!(probabilities.len() == 2, "expected 2 probabilities");

        Ok(ClassificationResult {
            p_ai: probabilities[0],
            p_shell: probabilities[1],
            source: InputClassifierDecisionSource::InputClassifier,
        })
    }
}

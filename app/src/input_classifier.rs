use std::sync::{Arc, OnceLock};

use input_classifier::{HeuristicClassifier, InputClassifier};
#[cfg(any(feature = "nld_classifier_v1", feature = "nld_classifier_v2"))]
use input_classifier::{OnnxClassifier, OnnxModel};
use warpui::{Entity, ModelContext, SingletonEntity};

pub struct InputClassifierModel {
    /// The classifier is lazily initialized on first access to avoid loading
    /// the ONNX model (~17 MB) and tokenizer vocabulary at app startup.
    classifier: OnceLock<Arc<dyn InputClassifier>>,
}

impl InputClassifierModel {
    pub fn new(_ctx: &mut ModelContext<Self>) -> Self {
        Self {
            classifier: OnceLock::new(),
        }
    }

    pub fn classifier(&self) -> Arc<dyn InputClassifier> {
        self.classifier
            .get_or_init(|| Self::init_classifier())
            .clone()
    }

    fn init_classifier() -> Arc<dyn InputClassifier> {
        #[cfg(feature = "nld_classifier_v1")]
        {
            match OnnxClassifier::new(OnnxModel::BertTinyV1) {
                Ok(classifier) => {
                    log::info!("Loaded onnx classifier bert_tiny_v1.onnx");
                    return Arc::new(classifier);
                }
                Err(e) => log::warn!("Failed to load onnx classifier bert_tiny_v1.onnx: {e:#}"),
            }
        }

        #[cfg(feature = "nld_classifier_v2")]
        {
            match OnnxClassifier::new(OnnxModel::BertTinyV2) {
                Ok(classifier) => {
                    log::info!("Loaded onnx classifier bert_tiny_v2.onnx");
                    return Arc::new(classifier);
                }
                Err(e) => log::warn!("Failed to load onnx classifier bert_tiny_v2.onnx: {e:#}"),
            }
        }

        Arc::new(HeuristicClassifier)
    }
}

impl Entity for InputClassifierModel {
    type Event = ();
}

impl SingletonEntity for InputClassifierModel {}

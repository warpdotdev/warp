//! Rust version of `TranscribeRequest` and its fields.
//!
//! These types correspond to the warp-go types defined in
//! warp-server/model/types/transcribe/request.go.
//!
//! Documentation on the types here is directly borrowed from the documentation on the go schema;
//! see the go schema for the source-of-truth.

use serde::{Deserialize, Serialize, Serializer};

#[derive(Debug, Clone, Deserialize, PartialEq, Default)]
pub enum Provider {
    /// Corresponds to "openai" string input.
    OpenAI,

    /// Corresponds to "wispr" string input.
    #[default]
    Wispr,
}

impl Provider {
    fn as_str(&self) -> &'static str {
        match self {
            Provider::OpenAI => "openai",
            Provider::Wispr => "wispr",
        }
    }
}

impl Serialize for Provider {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

/// Top-level request type for the unified transcription endpoint.
/// Corresponds to `TranscribeRequest` in Go.
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq)]
pub struct TranscribeRequest {
    /// Provider specifies which transcription service to use (e.g., "openai", "wispr").
    /// Note we purposefully define this as an enum on the Rust-side (Go doesn't have enums, the REST
    /// API uses strings).
    pub provider: Provider,

    /// Language is the ISO-639-1 code of the audio language (e.g., "en").
    /// Optional.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,

    /// Format is the desired response format (e.g., "json", "text", "srt").
    /// Optional.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,

    /// Prompt is an optional text prompt to guide the transcription style or continue context.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,

    /// Audio is the base64-encoded audio data (primarily used for Wispr).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audio: Option<String>,

    /// Additional configuration fields specific to the OpenAI Whisper API.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub openai_properties: Option<OpenAIProperties>,

    /// Additional configuration fields specific to the Wispr API.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wispr_properties: Option<WisprProperties>,
}

/// Optional configuration fields for the OpenAI Whisper API.
/// Corresponds to `OpenAIProperties` in Go.
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq)]
pub struct OpenAIProperties {
    /// Temperature controls the sampling randomness.
    /// Optional.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,

    /// Whisper output format (e.g., "json", "text", "srt", "verbose_json", "vtt").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_format: Option<String>,

    /// Whisper model to use. Currently, only "whisper-1" is supported.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,

    /// Timestamp resolution for verbose_json format (e.g., ["segment", "word"]).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp_granularities: Option<Vec<String>>,
}

/// Additional optional configuration for Wispr requests.
/// Corresponds to `WisprProperties` in Go.
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq)]
pub struct WisprProperties {
    /// Context for the transcription (e.g., "other").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_type: Option<String>,

    /// Custom dictionary to help improve transcription accuracy.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dictionary: Option<String>,

    /// Context text appearing after the main transcription.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after_text: Option<String>,

    /// Context text appearing before the main transcription.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before_text: Option<String>,

    /// Specific substring that might be highlighted or focused on.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selected_text: Option<String>,
}

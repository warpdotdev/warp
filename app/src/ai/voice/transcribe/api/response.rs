//! Rust version of `TranscribeResponse` and its fields.
//!
//! These types correspond to the warp-go types defined in
//! warp-server/model/types/transcribe/response.go.
//!
//! Documentation on the types here is directly borrowed from the documentation on the go schema;
//! see the go schema for the source-of-truth.

use serde::{Deserialize, Serialize};

/// Top-level response type for the transcription API endpoint.
/// Corresponds to `TranscribeResponse` in Go.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TranscribeResponse {
    /// The transcribed text.
    pub text: String,
}

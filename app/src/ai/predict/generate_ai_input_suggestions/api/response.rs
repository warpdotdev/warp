//! Rust version of `GenerateAIInputSuggestionsResponse` and its fields.
//!
//! These types correspond to the warp-go types defined in
//! warp-server/model/types/generate_ai_input_suggestions/response.go.
//!
//! Documentation on the types here is directly borrowed from the documentation on the go schema;
//! see the go schema for the source-of-truth.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentModeSuggestionV2 {
    pub query: String,
    pub context_block_ids: Vec<String>,
}

/// Top-level response type for the `GenerateAIInputSuggestions` API endpoint.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct GenerateAIInputSuggestionsResponseV2 {
    pub commands: Vec<String>,
    pub ai_queries: Vec<AgentModeSuggestionV2>,
    pub most_likely_action: String,
}

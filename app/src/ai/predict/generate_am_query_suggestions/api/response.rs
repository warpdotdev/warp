//! Rust version of `GenerateAMQuerySuggestionsResponse` and its fields.
//!
//! These types correspond to the warp-go types defined in
//! warp-server/model/types/generate_am_query_suggestions/response.go.
//!
//! Documentation on the types here is directly borrowed from the documentation on the go schema;
//! see the go schema for the source-of-truth.
use serde::{Deserialize, Serialize};

use crate::ai::agent::FileLocations;

/// Top-level response type for the `GenerateAMQuerySuggestions` API endpoint.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct GenerateAMQuerySuggestionsResponse {
    pub id: String,
    pub suggestion: Option<Suggestion>,
}

/// Represents a particular type of suggestion.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Suggestion {
    Simple(SimpleQuery),
    Coding(CodingQuery),
}

/// Simple suggestion structure.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SimpleQuery {
    pub query: String,
    /// If the query is a complex task, we should use Dispatch to create a plan first.
    pub should_plan_task: bool,
}

impl GenerateAMQuerySuggestionsResponse {
    /// Check whether the response contains a valid code delegation, which should include:
    /// 1) the coding_query field is non-empty.
    /// 2) the attached file locations to the coding_query is non-empty.
    pub fn is_valid_code_delegation(&self) -> bool {
        matches!(&self.suggestion, Some(Suggestion::Coding(coding_query)) if !coding_query.files.is_empty())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct CodingQuery {
    pub files: Vec<GeneratedFileLocations>,
    pub query: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct GeneratedFileLocations {
    pub file_name: String,
    pub line_numbers: Option<Vec<usize>>,
}

impl From<GeneratedFileLocations> for FileLocations {
    fn from(value: GeneratedFileLocations) -> Self {
        Self {
            name: value.file_name,
            // We are explicitly discarding the line_numbers right now.
            // TODO(kevin): Convert them into Range<usize>.
            lines: Vec::new(),
        }
    }
}

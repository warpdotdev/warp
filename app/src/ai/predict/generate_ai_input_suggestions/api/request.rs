//! Rust version of `GenerateAIInputSuggestionsRequest` and its fields.
//!
//! These types correspond to the warp-go types defined in
//! warp-server/model/types/generate_ai_input_suggestions/request.go.
//!
//! Documentation on the types here is directly borrowed from the documentation on the go schema;
//! see the go schema for the source-of-truth.

use serde::{Deserialize, Serialize};

use crate::ai::block_context::BlockContext;
use crate::terminal::input::IntelligentAutosuggestionResult;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
/// Top-level request type for the `GenerateAIInputSuggestions` API endpoint.
pub struct GenerateAIInputSuggestionsRequest {
    /// The previous blocks that were run in the session. Each item in the array is expected to correspond to 1 block.
    /// TODO(advait): we've purposely switched over to a free-form string for faster iteration here. We should
    /// switch back to strongly typed fields, once we've figured out the right schema/context/API for this.
    pub context_messages: Vec<String>,

    /// Relevant command history we've found that can inform the next command.
    /// TODO(roland): we've purposely switched over to a free-form string for faster iteration here. We should
    /// switch back to strongly typed fields, once we've figured out the right schema/context/API for this.
    pub history_context: String,

    /// System/platform relevant context for system prompt.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_context: Option<String>,

    /// Earlier suggestions that the user did not accept.
    /// If this is populated, it means the user wanted to generate new suggestions,
    /// and we should not show them the same suggestions again.
    pub rejected_suggestions: Vec<String>,

    /// The prefix that the user has already typed in the terminal input.
    pub prefix: Option<String>,

    /// Context about the just-completed block.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_context: Option<Box<BlockContext>>,

    /// The autosuggestion result from the previous next-command prediction, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_result: Option<IntelligentAutosuggestionResult>,
}

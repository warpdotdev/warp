//! AI Assistant has since been renamed to "Warp AI" in the product.
use std::collections::HashSet;
use std::sync::Arc;

use itertools::Itertools;
use lazy_static::lazy_static;
use pathfinder_color::ColorU;
use serde::{Deserialize, Serialize};
use warp_core::command::ExitCode;
use warp_graphql::ai::{
    RequestLimitInfo as RequestLimitInfoGraphql,
    RequestLimitRefreshDuration as RequestLimitRefreshDurationGraphql,
};
use warp_graphql::mutations::generate_commands::{GenerateCommandsFailureType, GeneratedCommand};

use crate::ai::{RequestLimitInfo, RequestLimitRefreshDuration};
use crate::server::telemetry::OpenedWarpAISource;
use crate::terminal::model::terminal_model::BlockIndex;
use crate::workflows::workflow::{Argument, Workflow};

pub mod execution_context;
pub mod panel;
pub mod requests;
pub mod transcript;
pub mod utils;

#[cfg(test)]
mod test_util;

/// We want to make sure the user doesn't send a prompt too large.s
/// Since a token is ~ 4 chars, the limit we impose here is 250 tokens.
/// This is also roughly the limit at which the editor starts degrading.
pub const PROMPT_CHARACTER_LIMIT: usize = 1000;

pub const AI_ASSISTANT_FEATURE_NAME: &str = "Warp AI";
pub const ASK_AI_ASSISTANT_TEXT: &str = "Ask Warp AI";

pub const AI_ASSISTANT_SVG_PATH: &str = "bundled/svg/ai-assistant.svg";

lazy_static! {
    pub static ref AI_ASSISTANT_LOGO_COLOR: ColorU = ColorU::new(243, 185, 17, 255);
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AskAIType {
    /// Covers all possible origins of text selection, including the block list terminal,
    /// the alt-screen terminal, and the input area. Not all instances will require
    /// `populate_input_box`, which determines whether we should automatically render
    /// something like "Explain the following" within the user's input box.
    FromTextSelection {
        text: Arc<String>,
        populate_input_box: bool,
    },
    /// Data about a block to inform Agent Mode.
    FromBlock {
        input: Arc<String>,
        output: Arc<String>,
        exit_code: ExitCode,
        block_index: BlockIndex,
    },
    /// Which blocks to attach to a block list AI query.
    FromBlocks {
        block_indices: HashSet<BlockIndex>,
    },
    FromAICommandSearch {
        query: Arc<String>,
    },
}

impl From<&AskAIType> for OpenedWarpAISource {
    fn from(value: &AskAIType) -> Self {
        match value {
            AskAIType::FromAICommandSearch { .. } => OpenedWarpAISource::FromAICommandSearch,
            AskAIType::FromBlock { .. } | AskAIType::FromBlocks { .. } => {
                OpenedWarpAISource::HelpWithBlock
            }
            AskAIType::FromTextSelection { .. } => OpenedWarpAISource::HelpWithTextSelection,
        }
    }
}

pub struct AIGeneratedCommand {
    command: String,
    description: String,
    parameters: Vec<AIGeneratedCommandParameter>,
}

pub struct AIGeneratedCommandParameter {
    id: String,
    description: String,
}

impl From<AIGeneratedCommand> for Workflow {
    fn from(ai_command: AIGeneratedCommand) -> Self {
        // Note that we use the AI generated description as the _title_ of the workflow.
        Workflow::new(ai_command.description, ai_command.command).with_arguments(
            ai_command
                .parameters
                .into_iter()
                .map(|p| Argument {
                    name: p.id,
                    description: Some(p.description),
                    default_value: None,
                    arg_type: Default::default(),
                })
                .collect_vec(),
        )
    }
}

impl From<GeneratedCommand> for AIGeneratedCommand {
    fn from(value: GeneratedCommand) -> Self {
        AIGeneratedCommand {
            command: value.command,
            description: value.description,
            parameters: value
                .parameters
                .into_iter()
                .map(|p| AIGeneratedCommandParameter {
                    id: p.id,
                    description: p.description,
                })
                .collect_vec(),
        }
    }
}

#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
pub enum GenerateCommandsFromNaturalLanguageError {
    BadPrompt,
    AiProviderError,
    RateLimited,
    Other,
}

impl From<GenerateCommandsFailureType> for GenerateCommandsFromNaturalLanguageError {
    fn from(value: GenerateCommandsFailureType) -> Self {
        match value {
            GenerateCommandsFailureType::BadPrompt => Self::BadPrompt,
            GenerateCommandsFailureType::AiProviderError => Self::AiProviderError,
            GenerateCommandsFailureType::RateLimited => Self::RateLimited,
            GenerateCommandsFailureType::Other => Self::Other,
        }
    }
}

impl From<RequestLimitRefreshDurationGraphql> for RequestLimitRefreshDuration {
    fn from(value: RequestLimitRefreshDurationGraphql) -> Self {
        match value {
            RequestLimitRefreshDurationGraphql::Monthly => RequestLimitRefreshDuration::Monthly,
            RequestLimitRefreshDurationGraphql::Weekly => RequestLimitRefreshDuration::Weekly,
            RequestLimitRefreshDurationGraphql::EveryTwoWeeks => {
                RequestLimitRefreshDuration::EveryTwoWeeks
            }
        }
    }
}

impl From<RequestLimitInfoGraphql> for RequestLimitInfo {
    fn from(value: RequestLimitInfoGraphql) -> Self {
        RequestLimitInfo {
            is_unlimited: value.is_unlimited,
            limit: value.request_limit as usize,
            num_requests_used_since_refresh: value.requests_used_since_last_refresh as usize,
            next_refresh_time: value.next_refresh_time,
            request_limit_refresh_duration: value.request_limit_refresh_duration.into(),
            is_unlimited_voice: value.is_unlimited_voice,
            voice_request_limit: value.voice_request_limit as usize,
            voice_requests_used_since_last_refresh: value.voice_requests_used_since_last_refresh
                as usize,
            is_unlimited_codebase_indices: value.is_unlimited_codebase_indices,
            max_codebase_indices: value.max_codebase_indices as usize,
            max_files_per_repo: value.max_files_per_repo as usize,
            embedding_generation_batch_size: value.embedding_generation_batch_size as usize,
        }
    }
}

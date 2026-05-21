//! This module contains all auxiliary code for hitting the GenerateAIInputSuggestions endpoint in
//! warp-server that serves Agent Predict.

mod api;

pub use api::*;

use chrono::NaiveDateTime;
use itertools::Itertools;
use parking_lot::FairMutex;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::ai_assistant::execution_context::WarpAiExecutionContext;
use crate::terminal::model::block::BlockState;
use crate::terminal::TerminalModel;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandContext {
    pub pwd: Option<String>,
    pub git_branch: Option<String>,
    pub exit_code: i64,
}
/// Used for AI-powered input suggestions (next action prediction). We
/// pass relevant context per block to AI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextMessageInput {
    pub input: String,
    pub output: String,
    pub context: CommandContext,
}

/// A section of command history that can be used as context for intelligent autosuggestions.
/// The context includes a consecutive sequence of commands in a session (previous_commands) and the next_command,
/// previous_commands can be similar to the user's recently run commands, and next_command can be used
/// to predict the next command.
#[derive(Clone)]
pub struct HistoryContext {
    pub previous_commands: Vec<crate::persistence::model::Command>,
    pub next_command: crate::persistence::model::Command,
}

/// Context needed for a Next Command suggestion that remains constant regardless of what
/// the user has typed in the input. This is reused as the user edits the input and generates new suggestions.
#[derive(Clone)]
pub struct NextCommandContext {
    pub history_contexts: Vec<HistoryContext>,
    pub ai_execution_context: WarpAiExecutionContext,
    pub context_messages: Vec<ContextMessageInput>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CommandWithContext {
    pub command: String,
    pub pwd: Option<String>,
    pub exit_code: Option<i32>,
    pub git_branch: Option<String>,
    pub start_ts: Option<NaiveDateTime>,
}

impl From<&crate::persistence::model::Command> for CommandWithContext {
    fn from(command: &crate::persistence::model::Command) -> Self {
        CommandWithContext {
            command: command.command.clone(),
            pwd: command.pwd.clone(),
            exit_code: command.exit_code,
            git_branch: command.git_branch.clone(),
            start_ts: command.start_ts,
        }
    }
}

impl HistoryContext {
    pub fn to_context_string(&self) -> String {
        self.previous_commands
            .iter()
            .chain(std::iter::once(&self.next_command))
            .map(CommandWithContext::from)
            .filter_map(|command_with_context| serde_json::to_string(&command_with_context).ok())
            .join("\n")
    }

    // Same as to_context_string, but skips any commands up to and including the specified ID.
    // Returns a tuple of the resulting string and a boolean indicating whether any commands were skipped.
    pub fn to_context_string_skip_before_command_id(&self, id: i32) -> (String, bool) {
        let mut command_strings = Vec::with_capacity(self.previous_commands.len());
        let mut skipped = false;
        for command in self
            .previous_commands
            .iter()
            .chain(std::iter::once(&self.next_command))
        {
            // Skip all commands up to the specified ID
            if command.id == id {
                command_strings.clear();
                skipped = true;
                continue;
            }
            let command_with_context = CommandWithContext {
                command: command.command.clone(),
                pwd: command.pwd.clone(),
                exit_code: command.exit_code,
                git_branch: command.git_branch.clone(),
                start_ts: command.start_ts,
            };
            if let Ok(command_string) = serde_json::to_string(&command_with_context) {
                command_strings.push(command_string);
            }
        }
        (command_strings.join("\n"), skipped)
    }
}

/// Gets the last X user-run command blocks to use as context for the AI model. Truncates blocks
/// where relevant to top/bottom lines.
pub fn get_context_messages(
    terminal_model: Arc<FairMutex<TerminalModel>>,
    number_of_blocks: usize,
    number_of_top_lines_per_grid: usize,
    number_of_bottom_lines_per_grid: usize,
) -> Vec<ContextMessageInput> {
    let model = terminal_model.lock();
    let blocks = model.block_list().blocks();
    let terminal_width = model.block_list().size().columns();

    let filtered_blocks: Vec<_> = blocks
        .iter()
        .filter(|block| {
            block.state() == BlockState::DoneWithExecution && !block.is_in_band_command_block()
        })
        .collect();

    let last_blocks = if filtered_blocks.len() > number_of_blocks {
        &filtered_blocks[filtered_blocks.len() - number_of_blocks..]
    } else {
        &filtered_blocks[..]
    };

    last_blocks
        .iter()
        .map(|block| {
            let (processed_input, processed_output) = block.get_block_content_summary(
                terminal_width,
                number_of_top_lines_per_grid,
                number_of_bottom_lines_per_grid,
            );

            ContextMessageInput {
                input: processed_input,
                output: processed_output,
                context: CommandContext {
                    exit_code: block.exit_code().value() as i64,
                    pwd: block.pwd().cloned(),
                    git_branch: block.git_branch().cloned(),
                },
            }
        })
        .collect::<Vec<ContextMessageInput>>()
}

pub fn convert_context_messages_to_strings(
    context_messages: Vec<ContextMessageInput>,
) -> Vec<String> {
    context_messages
        .iter()
        .filter_map(|context_message| serde_json::to_string(context_message).ok())
        .collect_vec()
}

/// Merges history contexts that might be overlapping into a string of commands.
/// History contexts should be sorted from oldest to newest.
/// See generate_ai_input_suggestions_test.rs for an example.
fn merge_history_contexts_to_string(history_contexts: Vec<HistoryContext>) -> String {
    if history_contexts.is_empty() {
        return "".to_string();
    }
    let mut last_command_id = history_contexts[0].next_command.id;
    let mut final_string_lines = Vec::with_capacity(history_contexts.len());
    final_string_lines.push(history_contexts[0].to_context_string());
    for history_context in history_contexts.iter().skip(1) {
        let (lines, is_overlapping) =
            history_context.to_context_string_skip_before_command_id(last_command_id);
        if !is_overlapping {
            final_string_lines.push("...".to_owned());
        }
        final_string_lines.push(lines);
        last_command_id = history_context.next_command.id;
    }
    final_string_lines.join("\n")
}

pub fn create_generate_ai_input_suggestions_request(
    next_command_context: NextCommandContext,
    prefix: Option<String>,
    block_context: Option<Box<crate::ai::block_context::BlockContext>>,
    previous_result: Option<crate::terminal::input::IntelligentAutosuggestionResult>,
) -> GenerateAIInputSuggestionsRequest {
    GenerateAIInputSuggestionsRequest {
        context_messages: convert_context_messages_to_strings(
            next_command_context.context_messages,
        ),
        system_context: next_command_context.ai_execution_context.to_json_string(),
        history_context: merge_history_contexts_to_string(next_command_context.history_contexts),
        rejected_suggestions: vec![],
        prefix,
        block_context,
        previous_result,
    }
}

#[cfg(test)]
#[path = "generate_ai_input_suggestions_tests.rs"]
mod tests;

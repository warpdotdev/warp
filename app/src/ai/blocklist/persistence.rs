//! Manages how we serialize blocklist AI data for persistence.
#![cfg_attr(not(feature = "local_fs"), allow(dead_code))]

use std::{collections::HashMap, sync::Arc};
use uuid::Uuid;

use anyhow::anyhow;
use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};

use crate::{
    ai::{
        agent::{
            conversation::AIConversationId, AIAgentActionType, AIAgentAttachment, AIAgentContext,
            AIAgentExchangeId, AIAgentInput, AIAgentPtyWriteMode, AskUserQuestionItem,
            FileLocations, PassiveSuggestionResultType, ReadFilesRequest,
            RequestComputerUseRequest, SearchCodebaseRequest, UseComputerRequest, UserQueryMode,
        },
        llms::LLMId,
    },
    terminal::model::block::{BlockId, SerializedBlock},
};

use super::AIQueryHistoryOutputStatus;
/// Data we persist for each [`AIAgentExchange`] for use in history. Does not contain output data.
#[derive(Debug, Deserialize, Clone)]
pub struct PersistedAIInput {
    pub(crate) exchange_id: AIAgentExchangeId,
    pub(crate) conversation_id: AIConversationId,
    pub(crate) start_ts: DateTime<Local>,
    pub(crate) inputs: Vec<PersistedAIInputType>,
    pub(crate) output_status: AIQueryHistoryOutputStatus,
    pub(crate) working_directory: Option<String>,
    // TODO(CORE-3546): pub(crate) shell: Option<AvailableShell>,
    pub(crate) model_id: LLMId,
    #[allow(unused)]
    pub(crate) coding_model_id: LLMId,
}

/// Pieces of data we need to persist for each [`AIAgentExchange`]'s input for session restoration.
///
/// Note: Only Query is actually used - it's used for up-arrow history.
/// TODO(roland): consider removing the ai_queries table and getting queries from tasks as well.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub(crate) enum PersistedAIInputType {
    Query {
        text: String,
        #[serde(default)]
        context: Arc<[AIAgentContext]>,
        #[serde(default)]
        referenced_attachments: HashMap<String, AIAgentAttachment>,
    },
}

impl TryFrom<&AIAgentInput> for PersistedAIInputType {
    type Error = anyhow::Error;

    fn try_from(input: &AIAgentInput) -> Result<Self, Self::Error> {
        match input {
            AIAgentInput::UserQuery {
                query,
                context,
                referenced_attachments,
                ..
            } => Ok(Self::Query {
                text: query.clone(),
                context: context.clone(),
                referenced_attachments: referenced_attachments.clone(),
            }),
            AIAgentInput::AutoCodeDiffQuery { query, context } => Ok(Self::Query {
                text: query.clone(),
                context: context.clone(),
                referenced_attachments: Default::default(),
            }),
            AIAgentInput::PassiveSuggestionResult { suggestion: PassiveSuggestionResultType::Prompt { prompt }, context, .. } => Ok(Self::Query {
                text: prompt.clone(),
                context: context.clone(),
                referenced_attachments: Default::default(),
            }),
            AIAgentInput::PassiveSuggestionResult { suggestion: PassiveSuggestionResultType::CodeDiff { .. }, .. } => Err(anyhow!(
                "PassiveSuggestionResult::CodeDiff is not persisted as a query."
            )),
            AIAgentInput::ActionResult { .. }
            | AIAgentInput::ResumeConversation { .. }
            | AIAgentInput::InitProjectRules { .. }
            | AIAgentInput::CreateEnvironment { .. }
            | AIAgentInput::TriggerPassiveSuggestion { .. }
            | AIAgentInput::CreateNewProject { .. }
            | AIAgentInput::CloneRepository { .. }
            | AIAgentInput::CodeReview { .. }
            | AIAgentInput::FetchReviewComments { .. }
            | AIAgentInput::SummarizeConversation { .. }
            | AIAgentInput::InvokeSkill { .. }
            | AIAgentInput::StartFromAmbientRunPrompt { .. }
            | AIAgentInput::MessagesReceivedFromAgents { .. }
            | AIAgentInput::EventsFromAgents { .. }
            | AIAgentInput::OrchestrationConfigUpdate { .. } => Err(anyhow::anyhow!(
                "This input type is not persisted. Only Query inputs are persisted for up-arrow history."
            )),
        }
    }
}

impl TryFrom<PersistedAIInputType> for AIAgentInput {
    type Error = anyhow::Error;

    fn try_from(value: PersistedAIInputType) -> Result<Self, Self::Error> {
        match value {
            PersistedAIInputType::Query {
                text,
                context,
                referenced_attachments,
            } => Ok(Self::UserQuery {
                query: text,
                context,
                referenced_attachments,
                static_query_type: None,
                user_query_mode: UserQueryMode::default(),
                running_command: None,
                intended_agent: None,
            }),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) enum PersistedPtyWriteMode {
    Raw,
    Line,
    Block,
}

impl From<PersistedPtyWriteMode> for AIAgentPtyWriteMode {
    fn from(value: PersistedPtyWriteMode) -> Self {
        match value {
            PersistedPtyWriteMode::Raw => Self::Raw,
            PersistedPtyWriteMode::Block => Self::Block,
            PersistedPtyWriteMode::Line => Self::Line,
        }
    }
}

impl From<AIAgentPtyWriteMode> for PersistedPtyWriteMode {
    fn from(value: AIAgentPtyWriteMode) -> Self {
        match value {
            AIAgentPtyWriteMode::Raw => Self::Raw,
            AIAgentPtyWriteMode::Block => Self::Block,
            AIAgentPtyWriteMode::Line => Self::Line,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) enum PersistedAIAgentActionType {
    RequestCommandOutput {
        command: String,
    },
    WriteToLongRunningShellCommand {
        block_id: BlockId,
        input: bytes::Bytes,
        mode: PersistedPtyWriteMode,
    },
    RequestFileEdits {
        file_names: Vec<String>,
    },
    GetFiles {
        file_names: Vec<String>,
    },
    GetRelevantFiles {
        query: String,
        partial_paths: Option<Vec<String>>,
        codebase_path: Option<String>,
    },
    Grep {
        queries: Vec<String>,
        path: String,
    },
    FileGlob {
        patterns: Vec<String>,
        path: Option<String>,
    },
    FileGlobV2 {
        patterns: Vec<String>,
        search_dir: Option<String>,
    },
    ReadMCPResource {
        server_id: Option<Uuid>,
        name: String,
        uri: Option<String>,
    },
    CallMCPTool {
        server_id: Option<Uuid>,
        name: String,
        input: serde_json::Value,
    },
    SuggestNewConversation {
        message_id: String,
    },
    SuggestPrompt,
    OpenCodeReview,
    InitProject,
    UseComputer {
        action_summary: String,
        actions: Vec<computer_use::Action>,
        screenshot_params: Option<computer_use::ScreenshotParams>,
    },
    RequestComputerUse {
        task_summary: String,
        screenshot_params: Option<computer_use::ScreenshotParams>,
    },
    AskUserQuestion {
        questions: Vec<AskUserQuestionItem>,
    },

    FetchConversation {
        conversation_id: String,
    },

    /// Actions that don't need data persisted (since they're restored from conversation tasks) can be mapped to this.
    NotPersisted,
}

impl From<&AIAgentActionType> for PersistedAIAgentActionType {
    fn from(value: &AIAgentActionType) -> Self {
        match value {
            AIAgentActionType::RequestCommandOutput { command, .. } => Self::RequestCommandOutput {
                command: command.clone(),
            },
            AIAgentActionType::WriteToLongRunningShellCommand {
                block_id,
                input,
                mode,
            } => Self::WriteToLongRunningShellCommand {
                block_id: block_id.clone(),
                input: input.clone(),
                mode: (*mode).into(),
            },
            AIAgentActionType::RequestFileEdits { file_edits, .. } => Self::RequestFileEdits {
                file_names: file_edits
                    .iter()
                    .filter_map(|diff| diff.file())
                    .map(ToOwned::to_owned)
                    .collect(),
            },
            AIAgentActionType::ReadFiles(ReadFilesRequest { locations: files }) => Self::GetFiles {
                file_names: files.iter().map(|f| f.name.clone()).collect(),
            },
            AIAgentActionType::SearchCodebase(SearchCodebaseRequest {
                query,
                partial_paths,
                codebase_path,
            }) => Self::GetRelevantFiles {
                query: query.clone(),
                partial_paths: partial_paths.clone(),
                codebase_path: codebase_path.clone(),
            },
            AIAgentActionType::Grep { queries, path } => Self::Grep {
                queries: queries.clone(),
                path: path.clone(),
            },
            AIAgentActionType::FileGlob { patterns, path } => Self::FileGlob {
                patterns: patterns.clone(),
                path: path.clone(),
            },
            AIAgentActionType::FileGlobV2 {
                patterns,
                search_dir,
            } => Self::FileGlobV2 {
                patterns: patterns.clone(),
                search_dir: search_dir.clone(),
            },
            AIAgentActionType::CallMCPTool {
                server_id,
                name,
                input,
            } => Self::CallMCPTool {
                server_id: *server_id,
                name: name.clone(),
                input: input.clone(),
            },
            AIAgentActionType::ReadMCPResource {
                server_id,
                name,
                uri,
            } => Self::ReadMCPResource {
                server_id: *server_id,
                name: name.clone(),
                uri: uri.clone(),
            },
            AIAgentActionType::SuggestNewConversation { message_id } => {
                Self::SuggestNewConversation {
                    message_id: message_id.clone(),
                }
            }
            AIAgentActionType::SuggestPrompt { .. } => Self::SuggestPrompt,
            AIAgentActionType::OpenCodeReview => Self::OpenCodeReview,
            AIAgentActionType::InsertCodeReviewComments { .. } => Self::NotPersisted,
            AIAgentActionType::InitProject => Self::InitProject,
            AIAgentActionType::ReadDocuments(_)
            | AIAgentActionType::EditDocuments(_)
            | AIAgentActionType::CreateDocuments(_)
            | AIAgentActionType::ReadShellCommandOutput { .. }
            | AIAgentActionType::ReadSkill(_)
            | AIAgentActionType::UploadArtifact(_)
            | AIAgentActionType::TransferShellCommandControlToUser { .. } => Self::NotPersisted,
            AIAgentActionType::UseComputer(req) => Self::UseComputer {
                action_summary: req.action_summary.clone(),
                actions: req.actions.clone(),
                screenshot_params: req.screenshot_params,
            },
            AIAgentActionType::RequestComputerUse(req) => Self::RequestComputerUse {
                task_summary: req.task_summary.clone(),
                screenshot_params: req.screenshot_params,
            },
            AIAgentActionType::AskUserQuestion { questions } => Self::AskUserQuestion {
                questions: questions.clone(),
            },
            AIAgentActionType::FetchConversation { conversation_id } => Self::FetchConversation {
                conversation_id: conversation_id.clone(),
            },
            AIAgentActionType::StartAgent { .. } => Self::NotPersisted,
            AIAgentActionType::SendMessageToAgent { .. } => Self::NotPersisted,
            // Orchestrate is rendered from the in-history tool call message;
            // there is no per-action state we need to persist locally.
            AIAgentActionType::RunAgents(_) => Self::NotPersisted,
        }
    }
}

impl TryFrom<PersistedAIAgentActionType> for AIAgentActionType {
    type Error = anyhow::Error;

    fn try_from(value: PersistedAIAgentActionType) -> Result<Self, Self::Error> {
        match value {
            PersistedAIAgentActionType::RequestCommandOutput { command, .. } => {
                Ok(Self::RequestCommandOutput {
                    command,
                    rationale: None,
                    is_read_only: None,
                    is_risky: None,
                    uses_pager: None,
                    // TODO(zachbai): Support restoring this value from persisted type.
                    wait_until_completion: false,
                    citations: vec![],
                })
            }
            PersistedAIAgentActionType::WriteToLongRunningShellCommand {
                block_id,
                input,
                mode,
            } => Ok(Self::WriteToLongRunningShellCommand {
                block_id: block_id.clone(),
                input: input.clone(),
                mode: mode.into(),
            }),
            PersistedAIAgentActionType::GetRelevantFiles {
                query,
                partial_paths,
                codebase_path,
            } => Ok(Self::SearchCodebase(SearchCodebaseRequest {
                query,
                partial_paths,
                codebase_path,
            })),
            PersistedAIAgentActionType::RequestFileEdits { .. } => {
                // TODO(CODE-301): Implement proper restoration for suggested diffs.
                //
                // The current "implementation" is incomplete and does not actually persist any
                // diff content. For now, we just ignore the suggested diff actions altogether,
                // instead of restoring diffs with no content.
                Err(anyhow!("Restoration for RequestFileEdits is unsupported. "))
            }
            PersistedAIAgentActionType::GetFiles { file_names } => {
                Ok(Self::ReadFiles(ReadFilesRequest {
                    locations: file_names
                        .into_iter()
                        .map(|name| FileLocations {
                            name,
                            lines: Vec::new(),
                        })
                        .collect(),
                }))
            }
            PersistedAIAgentActionType::Grep { queries, path } => Ok(Self::Grep { queries, path }),
            PersistedAIAgentActionType::FileGlob { patterns, path } => {
                Ok(Self::FileGlob { patterns, path })
            }
            PersistedAIAgentActionType::FileGlobV2 {
                patterns,
                search_dir,
            } => Ok(Self::FileGlobV2 {
                patterns,
                search_dir,
            }),
            PersistedAIAgentActionType::CallMCPTool {
                server_id,
                name,
                input,
            } => Ok(Self::CallMCPTool {
                server_id,
                name,
                input,
            }),
            PersistedAIAgentActionType::ReadMCPResource {
                server_id,
                name,
                uri,
            } => Ok(Self::ReadMCPResource {
                server_id,
                name,
                uri,
            }),
            PersistedAIAgentActionType::SuggestNewConversation { message_id } => {
                Ok(Self::SuggestNewConversation {
                    message_id: message_id.clone(),
                })
            }
            PersistedAIAgentActionType::SuggestPrompt => {
                Err(anyhow!("Restoration for suggested prompts is unsupported."))
            }
            PersistedAIAgentActionType::OpenCodeReview => Ok(Self::OpenCodeReview),
            PersistedAIAgentActionType::InitProject => Ok(Self::InitProject),
            PersistedAIAgentActionType::UseComputer {
                action_summary,
                actions,
                screenshot_params,
            } => Ok(Self::UseComputer(UseComputerRequest {
                action_summary,
                actions,
                screenshot_params,
            })),
            PersistedAIAgentActionType::RequestComputerUse {
                task_summary,
                screenshot_params,
            } => Ok(Self::RequestComputerUse(RequestComputerUseRequest {
                task_summary,
                screenshot_params,
            })),
            PersistedAIAgentActionType::AskUserQuestion { questions } => {
                Ok(Self::AskUserQuestion { questions })
            }
            PersistedAIAgentActionType::FetchConversation { conversation_id } => {
                Ok(Self::FetchConversation { conversation_id })
            }
            PersistedAIAgentActionType::NotPersisted => Err(anyhow!(
                "Restoration is handled through conversation tasks, not persisted blocks."
            )),
        }
    }
}

/// The types of "blocks" we can store in our SQLite database for session restoration. Only command
/// blocks are true [`crate::terminal::model::block::Block`]s.
///
/// TODO(roland): now that there is no AI serialized block, consider removing this enum wrapper
#[derive(Debug, Clone, PartialEq)]
pub enum SerializedBlockListItem {
    Command { block: Box<SerializedBlock> },
}

impl SerializedBlockListItem {
    pub(crate) fn start_ts(&self) -> Option<DateTime<Local>> {
        match self {
            Self::Command { block } => block.start_ts,
        }
    }
}

impl From<crate::persistence::model::Block> for SerializedBlockListItem {
    fn from(value: crate::persistence::model::Block) -> Self {
        Self::Command {
            block: Box::new(SerializedBlock::from(value)),
        }
    }
}

impl From<SerializedBlock> for SerializedBlockListItem {
    fn from(value: SerializedBlock) -> Self {
        Self::Command {
            block: Box::new(value),
        }
    }
}

use std::path::PathBuf;
use std::time::Duration;
use uuid::Uuid;
use warp_core::features::FeatureFlag;
use warp_multi_agent_api as api;

use crate::agent::action::{
    AIAgentActionType, AIAgentPtyWriteMode, CommentSide, FileEdit, InsertReviewComment,
    InsertedCommentLine, InsertedCommentLocation, ReadFilesRequest, SearchCodebaseRequest,
    ShellCommandDelay, SuggestPromptRequest, UploadArtifactRequest,
};
use crate::agent::action_result::{AnyFileContent, FileContext};
use crate::agent::convert::ToolToAIAgentActionError;
use crate::agent::FileLocations;
use crate::diff_validation::{ParsedDiff, V4AHunk};
use crate::document::AIDocumentId;

impl From<api::message::tool_call::RunShellCommand> for AIAgentActionType {
    fn from(value: api::message::tool_call::RunShellCommand) -> Self {
        AIAgentActionType::RequestCommandOutput {
            command: value.command,
            is_read_only: Some(value.is_read_only),
            rationale: None,
            uses_pager: Some(value.uses_pager),
            is_risky: Some(value.is_risky),
            wait_until_completion: value.wait_until_complete_value.is_none_or(
                |api::message::tool_call::run_shell_command::WaitUntilCompleteValue::WaitUntilComplete(
                    should_wait,
                )| should_wait,
            ),
            citations: value
                .citations
                .iter()
                .filter_map(|citation| citation.clone().try_into().ok())
                .collect(),
        }
    }
}

impl From<api::message::tool_call::WriteToLongRunningShellCommand> for AIAgentActionType {
    fn from(value: api::message::tool_call::WriteToLongRunningShellCommand) -> Self {
        AIAgentActionType::WriteToLongRunningShellCommand {
            block_id: value.command_id.into(),
            input: value.input.into(),
            mode: value.mode.map(Into::into).unwrap_or_default(),
        }
    }
}

impl From<api::message::tool_call::write_to_long_running_shell_command::Mode>
    for AIAgentPtyWriteMode
{
    fn from(value: api::message::tool_call::write_to_long_running_shell_command::Mode) -> Self {
        match value.mode {
            Some(mode) => {
                use warp_multi_agent_api::message::tool_call::write_to_long_running_shell_command::mode::Mode;
                match mode {
                    Mode::Raw(_) => AIAgentPtyWriteMode::Raw,
                    Mode::Line(_) => AIAgentPtyWriteMode::Line,
                    Mode::Block(_) => AIAgentPtyWriteMode::Block,
                }
            }
            None => AIAgentPtyWriteMode::Raw,
        }
    }
}

impl From<api::message::tool_call::SuggestNewConversation> for AIAgentActionType {
    fn from(value: api::message::tool_call::SuggestNewConversation) -> Self {
        AIAgentActionType::SuggestNewConversation {
            message_id: value.message_id,
        }
    }
}

impl From<api::message::tool_call::ApplyFileDiffs> for AIAgentActionType {
    fn from(value: api::message::tool_call::ApplyFileDiffs) -> Self {
        let diff_edits = value.diffs.into_iter().map(|file_diff| {
            FileEdit::Edit(ParsedDiff::StrReplaceEdit {
                search: file_diff.search.none_if_default(),
                replace: file_diff.replace.none_if_default(),
                file: file_diff.file_path.none_if_default(),
            })
        });
        let v4a_updates = value.v4a_updates.into_iter().map(|v4a_update| {
            FileEdit::Edit(ParsedDiff::V4AEdit {
                file: v4a_update.file_path.none_if_default(),
                move_to: v4a_update.move_to.clone().none_if_default(),
                hunks: v4a_update
                    .hunks
                    .into_iter()
                    .map(|hunk| V4AHunk {
                        change_context: hunk.change_context,
                        pre_context: hunk.pre_context,
                        old: hunk.old,
                        new: hunk.new,
                        post_context: hunk.post_context,
                    })
                    .collect(),
            })
        });
        let file_deletes = value
            .deleted_files
            .into_iter()
            .map(|file_delete| FileEdit::Delete {
                file: file_delete.file_path.none_if_default(),
            });
        let new_file_edits = value
            .new_files
            .into_iter()
            .map(|new_file| FileEdit::Create {
                file: new_file.file_path.none_if_default(),
                content: new_file.content.none_if_default(),
            });

        AIAgentActionType::RequestFileEdits {
            file_edits: diff_edits
                .chain(v4a_updates)
                .chain(new_file_edits)
                .chain(file_deletes)
                .collect(),
            title: Some(value.summary),
        }
    }
}

impl From<api::message::tool_call::ReadFiles> for AIAgentActionType {
    fn from(value: api::message::tool_call::ReadFiles) -> Self {
        AIAgentActionType::ReadFiles(ReadFilesRequest {
            locations: value.files.into_iter().map(Into::into).collect(),
        })
    }
}

impl TryFrom<api::UploadFileArtifact> for AIAgentActionType {
    type Error = ToolToAIAgentActionError;

    fn try_from(value: api::UploadFileArtifact) -> Result<Self, Self::Error> {
        let file = value
            .file
            .filter(|file| !file.file_path.is_empty())
            .ok_or(ToolToAIAgentActionError::MissingUploadArtifactFileReference)?;

        Ok(AIAgentActionType::UploadArtifact(UploadArtifactRequest {
            file_path: file.file_path,
            description: value.description.none_if_default(),
        }))
    }
}

impl From<api::message::tool_call::SearchCodebase> for AIAgentActionType {
    fn from(value: api::message::tool_call::SearchCodebase) -> Self {
        AIAgentActionType::SearchCodebase(SearchCodebaseRequest {
            query: value.query,
            partial_paths: if !value.path_filters.is_empty() {
                Some(value.path_filters)
            } else {
                None
            },
            codebase_path: if !value.codebase_path.is_empty() {
                Some(value.codebase_path)
            } else {
                None
            },
        })
    }
}

impl From<api::message::tool_call::Grep> for AIAgentActionType {
    fn from(value: api::message::tool_call::Grep) -> Self {
        AIAgentActionType::Grep {
            queries: value.queries,
            path: value.path,
        }
    }
}

impl From<api::message::tool_call::FileGlob> for AIAgentActionType {
    fn from(value: api::message::tool_call::FileGlob) -> Self {
        AIAgentActionType::FileGlob {
            patterns: value.patterns,
            path: if value.path.is_empty() {
                None
            } else {
                Some(value.path)
            },
        }
    }
}

impl From<api::message::tool_call::FileGlobV2> for AIAgentActionType {
    fn from(value: api::message::tool_call::FileGlobV2) -> Self {
        AIAgentActionType::FileGlobV2 {
            patterns: value.patterns,
            search_dir: if value.search_dir.is_empty() {
                None
            } else {
                Some(value.search_dir)
            },
        }
    }
}

impl From<api::message::tool_call::read_files::File> for FileLocations {
    fn from(value: api::message::tool_call::read_files::File) -> Self {
        Self {
            name: value.name,
            lines: value
                .line_ranges
                .into_iter()
                .map(|line_range| line_range.start as usize..line_range.end as usize)
                .collect(),
        }
    }
}

impl From<api::message::tool_call::ReadMcpResource> for AIAgentActionType {
    fn from(value: api::message::tool_call::ReadMcpResource) -> Self {
        let server_id = if FeatureFlag::MCPGroupedServerContext.is_enabled() {
            Uuid::parse_str(&value.server_id).ok()
        } else {
            None
        };

        AIAgentActionType::ReadMCPResource {
            server_id,
            uri: Some(value.uri),
            name: Default::default(),
        }
    }
}

impl TryFrom<api::message::tool_call::CallMcpTool> for AIAgentActionType {
    type Error = ToolToAIAgentActionError;

    fn try_from(value: api::message::tool_call::CallMcpTool) -> Result<Self, Self::Error> {
        let Some(args) = value.args else {
            return Err(ToolToAIAgentActionError::CallMCPToolArgsError(
                String::from("missing args"),
            ));
        };

        let input = prost_to_serde_json(prost_types::Value {
            kind: Some(prost_types::value::Kind::StructValue(args)),
        })
        .map_err(ToolToAIAgentActionError::CallMCPToolArgsError)?;

        let server_id = if FeatureFlag::MCPGroupedServerContext.is_enabled() {
            Uuid::parse_str(&value.server_id).ok()
        } else {
            None
        };

        Ok(AIAgentActionType::CallMCPTool {
            server_id,
            name: value.name,
            input,
        })
    }
}

impl TryFrom<api::message::tool_call::SuggestPrompt> for AIAgentActionType {
    type Error = ToolToAIAgentActionError;

    fn try_from(value: api::message::tool_call::SuggestPrompt) -> Result<Self, Self::Error> {
        let request = match value.display_mode {
            Some(api::message::tool_call::suggest_prompt::DisplayMode::InlineQueryBanner(
                inline_query_banner,
            )) => SuggestPromptRequest::UnitTestsSuggestion {
                title: inline_query_banner.title,
                description: inline_query_banner.description,
                query: inline_query_banner.query,
            },
            Some(api::message::tool_call::suggest_prompt::DisplayMode::PromptChip(chip)) => {
                let label = chip.label.none_if_default();
                SuggestPromptRequest::PromptSuggestion {
                    prompt: chip.prompt,
                    label,
                }
            }
            _ => {
                return Err(ToolToAIAgentActionError::SuggestPromptError(String::from(
                    "unsupported display mode",
                )));
            }
        };
        Ok(AIAgentActionType::SuggestPrompt(request))
    }
}

impl From<warp_multi_agent_api::FileContent> for FileContext {
    fn from(content: warp_multi_agent_api::FileContent) -> Self {
        let line_range = content.line_range.map(|r| r.start as usize..r.end as usize);

        FileContext::new(
            content.file_path,
            AnyFileContent::StringContent(content.content),
            line_range,
            None,
        )
    }
}

impl From<warp_multi_agent_api::AnyFileContent> for FileContext {
    fn from(content: warp_multi_agent_api::AnyFileContent) -> Self {
        match content.content {
            Some(api::any_file_content::Content::BinaryContent(binary_content)) => {
                FileContext::new(
                    binary_content.file_path,
                    AnyFileContent::BinaryContent(binary_content.data),
                    None,
                    None,
                )
            }
            Some(api::any_file_content::Content::TextContent(text_content)) => {
                let line_range = text_content
                    .line_range
                    .map(|r| r.start as usize..r.end as usize);

                FileContext::new(
                    text_content.file_path,
                    AnyFileContent::StringContent(text_content.content),
                    line_range,
                    None,
                )
            }
            None => unreachable!("AnyFileContent should always have a content"),
        }
    }
}

impl From<api::message::tool_call::ReadDocuments> for AIAgentActionType {
    fn from(value: api::message::tool_call::ReadDocuments) -> Self {
        use crate::agent::action::ReadDocumentsRequest;
        AIAgentActionType::ReadDocuments(ReadDocumentsRequest {
            document_ids: value
                .documents
                .into_iter()
                .filter_map(|doc| AIDocumentId::try_from(doc.document_id).ok())
                .collect(),
        })
    }
}

impl From<api::message::tool_call::EditDocuments> for AIAgentActionType {
    fn from(value: api::message::tool_call::EditDocuments) -> Self {
        use crate::agent::action::{DocumentDiff, EditDocumentsRequest};
        AIAgentActionType::EditDocuments(EditDocumentsRequest {
            diffs: value
                .diffs
                .into_iter()
                .filter_map(|diff| {
                    AIDocumentId::try_from(diff.document_id)
                        .map(|document_id| DocumentDiff {
                            document_id,
                            search: diff.search,
                            replace: diff.replace,
                        })
                        .ok()
                })
                .collect(),
        })
    }
}

impl From<api::message::tool_call::CreateDocuments> for AIAgentActionType {
    fn from(value: api::message::tool_call::CreateDocuments) -> Self {
        use crate::agent::action::{CreateDocumentsRequest, DocumentToCreate};
        AIAgentActionType::CreateDocuments(CreateDocumentsRequest {
            documents: value
                .new_documents
                .into_iter()
                .map(|doc| DocumentToCreate {
                    content: doc.content,
                    title: if doc.title.is_empty() {
                        // DO NOT SUBMIT
                        // crate::ai::ai_document_view::DEFAULT_PLANNING_DOCUMENT_TITLE.to_string()
                        "".to_string()
                    } else {
                        doc.title
                    },
                })
                .collect(),
        })
    }
}

impl From<api::message::tool_call::ReadShellCommandOutput> for AIAgentActionType {
    fn from(value: api::message::tool_call::ReadShellCommandOutput) -> Self {
        let delay = match value.delay {
            Some(api::message::tool_call::read_shell_command_output::Delay::Duration(duration)) => {
                Some(ShellCommandDelay::Duration(Duration::from_secs(
                    duration.seconds as u64,
                )))
            }
            Some(api::message::tool_call::read_shell_command_output::Delay::OnCompletion(_)) => {
                Some(ShellCommandDelay::OnCompletion)
            }
            None => None,
        };
        AIAgentActionType::ReadShellCommandOutput {
            block_id: value.command_id.into(),
            delay,
        }
    }
}

impl From<api::message::tool_call::TransferShellCommandControlToUser> for AIAgentActionType {
    fn from(value: api::message::tool_call::TransferShellCommandControlToUser) -> Self {
        AIAgentActionType::TransferShellCommandControlToUser {
            reason: value.reason,
        }
    }
}

impl From<api::message::tool_call::FetchConversation> for AIAgentActionType {
    fn from(value: api::message::tool_call::FetchConversation) -> Self {
        AIAgentActionType::FetchConversation {
            conversation_id: value.conversation_id,
        }
    }
}

fn prost_to_serde_json(x: prost_types::Value) -> Result<serde_json::Value, String> {
    use prost_types::value::Kind::*;
    use serde_json::Value::*;

    let Some(kind) = x.kind else {
        return Err("google.protobuf.Value kind was None".to_string());
    };

    Ok(match kind {
        NullValue(_) => Null,
        BoolValue(v) => Bool(v),
        NumberValue(n) => Number(
            serde_json::Number::from_f64(n)
                .ok_or_else(|| format!("float {n} is not valid JSON number"))?,
        ),
        StringValue(s) => String(s),
        ListValue(l) => Array(
            l.values
                .into_iter()
                .map(prost_to_serde_json)
                .collect::<Result<Vec<_>, std::string::String>>()?,
        ),
        StructValue(v) => Object(
            v.fields
                .into_iter()
                .map(|(k, v)| prost_to_serde_json(v).map(|v| (k, v)))
                .collect::<Result<serde_json::Map<_, _>, std::string::String>>()?,
        ),
    })
}

/// Helper trait to easily convert default values to `None`.
/// With `prost`, scalar types are converted to their default values instead
/// of `None` if the type is unset. This trait allows a more natural
/// conversion to better denote if a value should be `Some` (indicating it was set)
/// or `None` (indicating it was unset).
///
///
/// NOTE: Consumers should use this with caution as it only makes sense where the default
/// value of the type isn't a reasonable value and `None` makes more sense instead.
trait NoneIfDefault
where
    Self: Sized + Default,
{
    fn none_if_default(self) -> Option<Self>;
}

impl NoneIfDefault for String {
    fn none_if_default(self) -> Option<Self> {
        if self == Self::default() {
            None
        } else {
            Some(self)
        }
    }
}

impl From<api::message::tool_call::InsertReviewComments> for AIAgentActionType {
    fn from(value: api::message::tool_call::InsertReviewComments) -> Self {
        AIAgentActionType::InsertCodeReviewComments {
            repo_path: PathBuf::from(value.repo_path),
            comments: value.comments.into_iter().map(Into::into).collect(),
            base_branch: value.base_branch.none_if_default(),
        }
    }
}

impl From<api::message::tool_call::insert_review_comments::CommentSide> for CommentSide {
    fn from(value: api::message::tool_call::insert_review_comments::CommentSide) -> Self {
        match value {
            api::message::tool_call::insert_review_comments::CommentSide::New => CommentSide::Right,
            api::message::tool_call::insert_review_comments::CommentSide::Old => CommentSide::Left,
        }
    }
}

impl From<api::message::tool_call::insert_review_comments::Comment> for InsertReviewComment {
    fn from(value: api::message::tool_call::insert_review_comments::Comment) -> Self {
        let location = value.location.map(|loc| InsertedCommentLocation {
            relative_file_path: loc.file_path,
            line: loc.line.and_then(|comment_line_range| {
                let side = comment_line_range.side().into();
                let diff_hunk = comment_line_range.diff_hunk;
                comment_line_range.range.map(|r| InsertedCommentLine {
                    comment_line_range: r.start as usize..r.end as usize,
                    diff_hunk_line_range: r.start as usize..r.end as usize,
                    diff_hunk_text: diff_hunk,
                    side: Some(side),
                })
            }),
        });

        InsertReviewComment {
            comment_id: value.comment_id,
            author: value.author,
            last_modified_timestamp: value.last_modified_timestamp,
            comment_body: value.comment_body,
            parent_comment_id: if value.parent_comment_id.is_empty() {
                None
            } else {
                Some(value.parent_comment_id)
            },
            comment_location: location,
            html_url: value.html_url.none_if_default(),
        }
    }
}

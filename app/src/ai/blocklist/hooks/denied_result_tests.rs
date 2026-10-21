use std::path::PathBuf;

use ai::agent::action::{
    AIAgentPtyWriteMode, AskUserQuestionItem, AskUserQuestionType, CreateDocumentsRequest,
    EditDocumentsRequest, FileEdit, ReadDocumentsRequest, ReadFilesRequest, ReadSkillRequest,
    RequestComputerUseRequest, RunAgentsExecutionMode, RunAgentsRequest, SearchCodebaseRequest,
    SuggestPromptRequest, UploadArtifactRequest, UseComputerRequest,
};
use ai::agent::action_result::{
    AskUserQuestionResult, CallMCPToolResult, CreateDocumentsResult, EditDocumentsResult,
    FetchConversationResult, FileGlobResult, FileGlobV2Result, GrepResult,
    InsertReviewCommentsResult, ReadDocumentsResult, ReadFilesResult, ReadMCPResourceResult,
    ReadShellCommandOutputResult, ReadSkillResult, RequestCommandOutputResult,
    RequestComputerUseResult, RequestFileEditsResult, RunAgentsResult, SearchCodebaseFailureReason,
    SearchCodebaseResult, SendMessageToAgentResult, StartRecordingResult, StopRecordingResult,
    SuggestNewConversationResult, SuggestPromptResult, TransferShellCommandControlToUserResult,
    UploadArtifactResult, UseComputerResult, WaitForEventsResult,
    WriteToLongRunningShellCommandResult,
};
use ai::skills::SkillReference;
use warp_multi_agent_api as api;
use warp_terminal::model::BlockId;

use super::*;
use crate::ai::agent::task::TaskId;
use crate::ai::agent::{AIAgentActionId, AIAgentActionResult};

const REASON: &str = "production branches are off limits";

fn denied(action: AIAgentActionType) -> AIAgentActionResultType {
    denied_result_for(&action, REASON)
}

/// Whether the result's wire form carries `text`. A result the conversion drops carries nothing.
fn wire_carries(result: &AIAgentActionResultType, text: &str) -> bool {
    let action_result = AIAgentActionResult {
        id: AIAgentActionId::from("call_1".to_owned()),
        task_id: TaskId::new("task".to_owned()),
        result: result.clone(),
    };
    api::request::input::user_inputs::user_input::Input::try_from(action_result)
        .is_ok_and(|wire| format!("{wire:?}").contains(text))
}

fn prefixed() -> String {
    format!("{DENIED_BY_HOOK_PREFIX}{REASON}")
}

#[test]
fn a_shell_command_is_denied_with_the_hooks_reason_as_a_denial() {
    let result = denied(AIAgentActionType::RequestCommandOutput {
        command: "git push --force".to_owned(),
        is_read_only: None,
        is_risky: None,
        wait_until_completion: true,
        uses_pager: None,
        rationale: None,
        citations: Vec::new(),
    });
    assert_eq!(
        result,
        AIAgentActionResultType::RequestCommandOutput(RequestCommandOutputResult::HookDenied {
            command: "git push --force".to_owned(),
            reason: REASON.to_owned(),
        })
    );
    assert!(result.is_failed());
    assert!(wire_carries(&result, REASON));
}

/// Every family whose result has free text on the wire carries the prefixed reason there.
#[test]
fn families_with_a_text_slot_carry_the_prefixed_reason() {
    let text = prefixed();
    let cases: Vec<(AIAgentActionType, AIAgentActionResultType)> = vec![
        (
            AIAgentActionType::ReadFiles(ReadFilesRequest { locations: vec![] }),
            AIAgentActionResultType::ReadFiles(ReadFilesResult::Error(text.clone())),
        ),
        (
            AIAgentActionType::UploadArtifact(UploadArtifactRequest {
                file_path: "a.txt".to_owned(),
                description: None,
            }),
            AIAgentActionResultType::UploadArtifact(UploadArtifactResult::Error(text.clone())),
        ),
        (
            AIAgentActionType::SearchCodebase(SearchCodebaseRequest {
                query: "q".to_owned(),
                partial_paths: None,
                codebase_path: None,
            }),
            AIAgentActionResultType::SearchCodebase(SearchCodebaseResult::Failed {
                reason: SearchCodebaseFailureReason::ClientError,
                message: text.clone(),
            }),
        ),
        (
            AIAgentActionType::RequestFileEdits {
                file_edits: vec![FileEdit::Delete {
                    file: Some("a.txt".to_owned()),
                }],
                title: None,
            },
            AIAgentActionResultType::RequestFileEdits(
                RequestFileEditsResult::DiffApplicationFailed {
                    error: text.clone(),
                },
            ),
        ),
        (
            AIAgentActionType::Grep {
                queries: vec![],
                path: ".".to_owned(),
            },
            AIAgentActionResultType::Grep(GrepResult::Error(text.clone())),
        ),
        (
            AIAgentActionType::FileGlob {
                patterns: vec![],
                path: None,
            },
            AIAgentActionResultType::FileGlob(FileGlobResult::Error(text.clone())),
        ),
        (
            AIAgentActionType::FileGlobV2 {
                patterns: vec![],
                search_dir: None,
            },
            AIAgentActionResultType::FileGlobV2(FileGlobV2Result::Error(text.clone())),
        ),
        (
            AIAgentActionType::ReadMCPResource {
                server_id: None,
                name: "docs".to_owned(),
                uri: None,
            },
            AIAgentActionResultType::ReadMCPResource(ReadMCPResourceResult::Error(text.clone())),
        ),
        (
            AIAgentActionType::CallMCPTool {
                server_id: None,
                name: "list_issues".to_owned(),
                input: serde_json::json!({}),
            },
            AIAgentActionResultType::CallMCPTool(CallMCPToolResult::Error(text.clone())),
        ),
        (
            AIAgentActionType::ReadSkill(ReadSkillRequest {
                skill: SkillReference::BundledSkillId("x".to_owned()),
            }),
            AIAgentActionResultType::ReadSkill(ReadSkillResult::Error(text.clone())),
        ),
        (
            AIAgentActionType::ReadDocuments(ReadDocumentsRequest {
                document_ids: vec![],
            }),
            AIAgentActionResultType::ReadDocuments(ReadDocumentsResult::Error(text.clone())),
        ),
        (
            AIAgentActionType::EditDocuments(EditDocumentsRequest { diffs: vec![] }),
            AIAgentActionResultType::EditDocuments(EditDocumentsResult::Error(text.clone())),
        ),
        (
            AIAgentActionType::CreateDocuments(CreateDocumentsRequest { documents: vec![] }),
            AIAgentActionResultType::CreateDocuments(CreateDocumentsResult::Error(text.clone())),
        ),
        (
            AIAgentActionType::UseComputer(UseComputerRequest {
                action_summary: String::new(),
                actions: vec![],
                screenshot_params: None,
            }),
            AIAgentActionResultType::UseComputer(UseComputerResult::Error(text.clone())),
        ),
        (
            AIAgentActionType::RequestComputerUse(RequestComputerUseRequest {
                task_summary: String::new(),
                screenshot_params: None,
            }),
            AIAgentActionResultType::RequestComputerUse(RequestComputerUseResult::Error(
                text.clone(),
            )),
        ),
        (
            AIAgentActionType::StartRecording {
                frame_rate: 0,
                max_duration: None,
                max_size_bytes: None,
                summary: None,
                description: None,
                playback_speed_multiplier: None,
                window: None,
            },
            AIAgentActionResultType::StartRecording(StartRecordingResult::Error(text.clone())),
        ),
        (
            AIAgentActionType::StopRecording {
                recording_id: String::new(),
                should_persist: true,
            },
            AIAgentActionResultType::StopRecording(StopRecordingResult::Error(text.clone())),
        ),
        (
            AIAgentActionType::FetchConversation {
                conversation_id: String::new(),
            },
            AIAgentActionResultType::FetchConversation(FetchConversationResult::Error(
                text.clone(),
            )),
        ),
        (
            AIAgentActionType::SendMessageToAgent {
                addresses: vec![],
                subject: String::new(),
                message: String::new(),
            },
            AIAgentActionResultType::SendMessageToAgent(SendMessageToAgentResult::Error(
                text.clone(),
            )),
        ),
        (
            AIAgentActionType::AskUserQuestion {
                questions: vec![AskUserQuestionItem {
                    question_id: String::new(),
                    question: String::new(),
                    question_type: AskUserQuestionType::MultipleChoice {
                        is_multiselect: false,
                        options: vec![],
                        supports_other: false,
                    },
                }],
            },
            AIAgentActionResultType::AskUserQuestion(AskUserQuestionResult::Error(text.clone())),
        ),
        (
            AIAgentActionType::InsertCodeReviewComments {
                repo_path: PathBuf::from("/repo"),
                comments: vec![],
                base_branch: None,
            },
            AIAgentActionResultType::InsertReviewComments(InsertReviewCommentsResult::Error {
                repo_path: "/repo".to_owned(),
                message: text.clone(),
            }),
        ),
        (
            AIAgentActionType::RunAgents(Box::new(RunAgentsRequest {
                metadata: Default::default(),
                summary: String::new(),
                base_prompt: String::new(),
                skills: vec![],
                model_id: String::new(),
                harness_type: String::new(),
                execution_mode: RunAgentsExecutionMode::Local,
                agent_run_configs: vec![],
                plan_id: String::new(),
                harness_auth_secret_name: None,
            })),
            AIAgentActionResultType::RunAgents(RunAgentsResult::Denied {
                reason: text.clone(),
            }),
        ),
    ];
    for (action, expected) in cases {
        let result = denied(action.clone());
        assert_eq!(result, expected, "{action}");
        assert!(result.is_failed(), "{action}");
        assert!(!result.is_cancelled(), "{action}");
        assert!(
            wire_carries(&result, &text),
            "the reason reaches the wire for {action}"
        );
    }
}

/// Families with no text slot finish with a cancelled or rejected result; the reason reaches the
/// user through the notice only.
#[test]
fn families_without_a_text_slot_finish_cancelled_or_rejected() {
    let cases: Vec<(AIAgentActionType, AIAgentActionResultType)> = vec![
        (
            AIAgentActionType::WriteToLongRunningShellCommand {
                block_id: BlockId::new(),
                input: bytes::Bytes::new(),
                mode: AIAgentPtyWriteMode::Raw,
            },
            AIAgentActionResultType::WriteToLongRunningShellCommand(
                WriteToLongRunningShellCommandResult::Cancelled,
            ),
        ),
        (
            AIAgentActionType::ReadShellCommandOutput {
                block_id: BlockId::new(),
                delay: None,
            },
            AIAgentActionResultType::ReadShellCommandOutput(
                ReadShellCommandOutputResult::Cancelled,
            ),
        ),
        (
            AIAgentActionType::TransferShellCommandControlToUser {
                reason: String::new(),
            },
            AIAgentActionResultType::TransferShellCommandControlToUser(
                TransferShellCommandControlToUserResult::Cancelled,
            ),
        ),
        (
            AIAgentActionType::SuggestNewConversation {
                message_id: String::new(),
            },
            AIAgentActionResultType::SuggestNewConversation(SuggestNewConversationResult::Rejected),
        ),
        (
            AIAgentActionType::SuggestPrompt(SuggestPromptRequest::PromptSuggestion {
                prompt: String::new(),
                label: None,
            }),
            AIAgentActionResultType::SuggestPrompt(SuggestPromptResult::Cancelled),
        ),
        (
            AIAgentActionType::InitProject,
            AIAgentActionResultType::InitProject,
        ),
        (
            AIAgentActionType::OpenCodeReview,
            AIAgentActionResultType::OpenCodeReview,
        ),
        (
            AIAgentActionType::WaitForEvents {
                tool_call_id: String::new(),
                idle_timeout_seconds: 0,
            },
            AIAgentActionResultType::WaitForEvents(WaitForEventsResult::Cancelled),
        ),
    ];
    for (action, expected) in cases {
        let result = denied(action.clone());
        assert_eq!(result, expected, "{action}");
        assert!(
            !result.is_successful()
                || matches!(
                    result,
                    AIAgentActionResultType::InitProject | AIAgentActionResultType::OpenCodeReview
                ),
            "{action}"
        );
        assert!(!wire_carries(&result, REASON), "no text slot for {action}");
    }
}

/// The wire form of `result`, when the conversion keeps it.
fn wire_result(
    result: &AIAgentActionResultType,
) -> Option<api::request::input::tool_call_result::Result> {
    let action_result = AIAgentActionResult {
        id: AIAgentActionId::from("call_1".to_owned()),
        task_id: TaskId::new("task".to_owned()),
        result: result.clone(),
    };
    match api::request::input::user_inputs::user_input::Input::try_from(action_result).ok()? {
        api::request::input::user_inputs::user_input::Input::ToolCallResult(tool_call) => {
            tool_call.result
        }
        _ => None,
    }
}

/// What the model is told when a family without a text slot is denied, pinned so a change is
/// deliberate. A cancelled result sends no follow-up request when it is the last action of the
/// batch, so the turn ends as cancelled and the model never learns the action was denied. The
/// two single-result families reach the wire as their completed marker, so the model believes
/// the action happened.
#[test]
fn a_denied_family_without_a_text_slot_ends_the_turn_or_reports_completion() {
    let cancelled = [
        AIAgentActionType::WriteToLongRunningShellCommand {
            block_id: BlockId::new(),
            input: bytes::Bytes::new(),
            mode: AIAgentPtyWriteMode::Raw,
        },
        AIAgentActionType::ReadShellCommandOutput {
            block_id: BlockId::new(),
            delay: None,
        },
        AIAgentActionType::TransferShellCommandControlToUser {
            reason: String::new(),
        },
        AIAgentActionType::SuggestPrompt(SuggestPromptRequest::PromptSuggestion {
            prompt: String::new(),
            label: None,
        }),
        AIAgentActionType::WaitForEvents {
            tool_call_id: String::new(),
            idle_timeout_seconds: 0,
        },
    ];
    for action in cancelled {
        let result = denied(action.clone());
        assert!(result.is_cancelled(), "{action}");
        assert!(
            !result.should_trigger_request_upon_completion(),
            "a denied {action} on its own ends the turn without a follow-up request"
        );
    }
    let init = denied(AIAgentActionType::InitProject);
    assert!(init.should_trigger_request_upon_completion());
    assert!(
        matches!(
            wire_result(&init),
            Some(api::request::input::tool_call_result::Result::InitProject(
                _
            ))
        ),
        "a denied project initialization is reported to the model as completed"
    );
    let review = denied(AIAgentActionType::OpenCodeReview);
    assert!(review.should_trigger_request_upon_completion());
    assert!(review.is_successful());
    assert!(
        matches!(
            wire_result(&review),
            Some(api::request::input::tool_call_result::Result::OpenCodeReview(_))
        ),
        "a denied code review opening is reported to the model as completed"
    );
}

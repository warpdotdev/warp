use serde_json::json;
use warp_terminal::model::BlockId;

use super::*;
use crate::agent::action::{
    AIAgentPtyWriteMode, AskUserQuestionItem, AskUserQuestionType, ReadFilesRequest,
    RunAgentsExecutionMode, RunAgentsRequest, SearchCodebaseRequest, UploadArtifactRequest,
};
use crate::hooks::adapters::cursor::test_support::shell_action;

#[test]
fn tool_names_follow_cursors_vocabulary() {
    let cases: Vec<(AIAgentActionType, &str)> = vec![
        (shell_action("ls"), "Shell"),
        (
            AIAgentActionType::ReadFiles(ReadFilesRequest { locations: vec![] }),
            "Read",
        ),
        (
            AIAgentActionType::RequestFileEdits {
                file_edits: vec![FileEdit::Create {
                    file: Some("a.txt".to_owned()),
                    content: None,
                    allow_overwrite: false,
                }],
                title: None,
            },
            "Write",
        ),
        (
            AIAgentActionType::RequestFileEdits {
                file_edits: vec![FileEdit::Delete {
                    file: Some("a.txt".to_owned()),
                }],
                title: None,
            },
            "Delete",
        ),
        (
            AIAgentActionType::RequestFileEdits {
                file_edits: vec![
                    FileEdit::Delete {
                        file: Some("a.txt".to_owned()),
                    },
                    FileEdit::Create {
                        file: Some("b.txt".to_owned()),
                        content: None,
                        allow_overwrite: false,
                    },
                ],
                title: None,
            },
            "Write",
        ),
        (
            AIAgentActionType::RequestFileEdits {
                file_edits: vec![],
                title: None,
            },
            "Write",
        ),
        (
            AIAgentActionType::Grep {
                queries: vec![],
                path: ".".to_owned(),
            },
            "Grep",
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
            "Task",
        ),
        (
            AIAgentActionType::CallMCPTool {
                server_id: None,
                name: "list_issues".to_owned(),
                input: json!({}),
            },
            "MCP:list_issues",
        ),
        (
            AIAgentActionType::FileGlob {
                patterns: vec![],
                path: None,
            },
            "Glob",
        ),
        (
            AIAgentActionType::FileGlobV2 {
                patterns: vec![],
                search_dir: None,
            },
            "Glob",
        ),
        (
            AIAgentActionType::SearchCodebase(SearchCodebaseRequest {
                query: String::new(),
                partial_paths: None,
                codebase_path: None,
            }),
            "SearchCodebase",
        ),
        (
            AIAgentActionType::ReadMCPResource {
                server_id: None,
                name: "docs".to_owned(),
                uri: None,
            },
            "MCPResource:docs",
        ),
        (
            AIAgentActionType::WriteToLongRunningShellCommand {
                block_id: BlockId::new(),
                input: bytes::Bytes::new(),
                mode: AIAgentPtyWriteMode::Raw,
            },
            "ShellWrite",
        ),
        (
            AIAgentActionType::ReadShellCommandOutput {
                block_id: BlockId::new(),
                delay: None,
            },
            "ShellRead",
        ),
        (
            AIAgentActionType::TransferShellCommandControlToUser {
                reason: String::new(),
            },
            "ShellHandoff",
        ),
        (
            AIAgentActionType::UploadArtifact(UploadArtifactRequest {
                file_path: String::new(),
                description: None,
            }),
            "UploadArtifact",
        ),
        (
            AIAgentActionType::SuggestNewConversation {
                message_id: String::new(),
            },
            "SuggestNewConversation",
        ),
        (AIAgentActionType::InitProject, "InitProject"),
        (AIAgentActionType::OpenCodeReview, "OpenCodeReview"),
        (
            AIAgentActionType::StopRecording {
                recording_id: String::new(),
                should_persist: true,
            },
            "StopRecording",
        ),
        (
            AIAgentActionType::FetchConversation {
                conversation_id: String::new(),
            },
            "FetchConversation",
        ),
        (
            AIAgentActionType::SendMessageToAgent {
                addresses: vec![],
                subject: String::new(),
                message: String::new(),
            },
            "SendMessageToAgent",
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
            "AskUserQuestion",
        ),
        (
            AIAgentActionType::WaitForEvents {
                tool_call_id: String::new(),
                idle_timeout_seconds: 0,
            },
            "WaitForEvents",
        ),
    ];
    for (action, expected) in cases {
        assert_eq!(tool_name(&action), expected, "{action}");
    }
}

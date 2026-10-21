use std::sync::Arc;

use serde_json::json;
use warp_terminal::model::BlockId;

use super::*;
use crate::agent::action::{
    AIAgentActionType, FileEdit, ReadFilesRequest, RunAgentsExecutionMode, RunAgentsRequest,
    SearchCodebaseRequest,
};
use crate::agent::action_result::{
    AIAgentActionResultType, CallMCPToolResult, ReadFilesResult, RequestCommandOutputResult,
    RequestFileEditsResult,
};
use crate::hooks::adapters::cursor::events::CursorEvent;
use crate::hooks::adapters::cursor::test_support::{
    binding_for, completed_shell, fixture_envelope, mcp_action, shell_action, tool_failure,
    tool_pre, tool_result,
};
use crate::hooks::model::{HookPayload, TurnStopStatus};

fn binding_with_matcher(event: CursorEvent, matcher: Option<&str>) -> DeclaredBinding {
    let mut binding = binding_for(event, false);
    binding.matcher = matcher.map(str::to_owned);
    binding
}

fn matches(event: CursorEvent, matcher: Option<&str>, payload: HookPayload) -> bool {
    let cache = MatcherCache::default();
    binding_matches(
        &cache,
        event.spec(),
        &binding_with_matcher(event, matcher),
        &fixture_envelope(payload),
    )
}

fn shell_pre(command: &str) -> HookPayload {
    tool_pre(shell_action(command))
}

#[test]
fn empty_star_and_missing_matchers_match_everything() {
    for matcher in [None, Some(""), Some("*")] {
        assert!(matches(
            CursorEvent::BeforeShellExecution,
            matcher,
            shell_pre("anything at all")
        ));
        assert!(matches(
            CursorEvent::Stop,
            matcher,
            HookPayload::TurnStop {
                status: TurnStopStatus::Completed,
                continuation_count: 0,
            }
        ));
    }
    assert!(matches_everything(""));
    assert!(matches_everything("*"));
    assert!(!matches_everything(".*"));
}

#[test]
fn a_matcher_is_an_unanchored_case_sensitive_regex() {
    let event = CursorEvent::BeforeShellExecution;
    assert!(matches(
        event,
        Some("rm -rf"),
        shell_pre("sudo rm -rf /tmp/x")
    ));
    assert!(!matches(event, Some("rm -rf"), shell_pre("RM -RF /tmp/x")));
    assert!(matches(
        event,
        Some("^git (push|commit)"),
        shell_pre("git push origin main")
    ));
    assert!(!matches(
        event,
        Some("^git (push|commit)"),
        shell_pre("echo git push")
    ));
    assert!(matches(event, Some("push$"), shell_pre("git push")));
    assert!(!matches(
        event,
        Some("push$"),
        shell_pre("git push --force")
    ));
}

#[test]
fn pre_tool_use_matches_the_canonical_tool_name() {
    let event = CursorEvent::PreToolUse;
    let cases: Vec<(&str, AIAgentActionType)> = vec![
        ("Shell", shell_action("ls")),
        (
            "Read",
            AIAgentActionType::ReadFiles(ReadFilesRequest {
                locations: Vec::new(),
            }),
        ),
        (
            "Write",
            AIAgentActionType::RequestFileEdits {
                file_edits: vec![FileEdit::Create {
                    file: Some("a.rs".to_owned()),
                    content: Some(String::new()),
                    allow_overwrite: false,
                }],
                title: None,
            },
        ),
        (
            "Grep",
            AIAgentActionType::Grep {
                queries: vec!["x".to_owned()],
                path: ".".to_owned(),
            },
        ),
        (
            "Task",
            AIAgentActionType::RunAgents(Box::new(RunAgentsRequest {
                metadata: Default::default(),
                summary: String::new(),
                base_prompt: String::new(),
                skills: Vec::new(),
                model_id: String::new(),
                harness_type: String::new(),
                execution_mode: RunAgentsExecutionMode::Local,
                agent_run_configs: Vec::new(),
                plan_id: String::new(),
                harness_auth_secret_name: None,
            })),
        ),
        ("MCP:read_file", mcp_action("read_file", json!({}))),
        (
            "Glob",
            AIAgentActionType::FileGlob {
                patterns: Vec::new(),
                path: None,
            },
        ),
        (
            "SearchCodebase",
            AIAgentActionType::SearchCodebase(SearchCodebaseRequest {
                query: "q".to_owned(),
                partial_paths: None,
                codebase_path: None,
            }),
        ),
        ("InitProject", AIAgentActionType::InitProject),
    ];
    for (name, action) in cases {
        let exact = format!("^{}$", regex::escape(name));
        assert!(
            matches(event, Some(&exact), tool_pre(action.clone())),
            "{name} should match {exact}"
        );
        assert!(
            !matches(event, Some("^NoSuchTool$"), tool_pre(action.clone())),
            "{name} should not match another name"
        );
        assert!(matches(event, Some("*"), tool_pre(action)));
    }
    assert!(matches(
        event,
        Some("^MCP:"),
        tool_pre(mcp_action("read_file", json!({})))
    ));
    assert!(matches(
        event,
        Some("Shell|Write"),
        tool_pre(shell_action("ls"))
    ));
}

#[test]
fn post_tool_events_match_the_canonical_tool_name_too() {
    let result = tool_result(shell_action("ls"), completed_shell("ls", "", 0), None);
    assert!(matches(
        CursorEvent::PostToolUse,
        Some("^Shell$"),
        result.clone()
    ));
    assert!(!matches(CursorEvent::PostToolUse, Some("^Read$"), result));

    let failure = tool_failure(
        AIAgentActionType::ReadFiles(ReadFilesRequest {
            locations: Vec::new(),
        }),
        AIAgentActionResultType::ReadFiles(ReadFilesResult::Error("boom".to_owned())),
        None,
    );
    assert!(matches(
        CursorEvent::PostToolUseFailure,
        Some("^Read$"),
        failure
    ));
}

#[test]
fn shell_events_match_the_command_text_and_only_shell_commands() {
    let pre = CursorEvent::BeforeShellExecution;
    assert!(matches(pre, Some("^rm "), shell_pre("rm -rf x")));
    assert!(!matches(pre, Some("^rm "), shell_pre("ls")));
    assert!(!matches(
        pre,
        None,
        tool_pre(AIAgentActionType::ReadFiles(ReadFilesRequest {
            locations: Vec::new(),
        }))
    ));
    assert!(!matches(
        pre,
        None,
        tool_pre(mcp_action("read_file", json!({})))
    ));

    let after = CursorEvent::AfterShellExecution;
    let finished = tool_result(
        shell_action("cargo test"),
        completed_shell("cargo test", "ok", 0),
        None,
    );
    assert!(matches(after, Some("^cargo"), finished.clone()));
    assert!(!matches(after, Some("^git"), finished));

    let snapshot = tool_result(
        shell_action("tail -f log"),
        AIAgentActionResultType::RequestCommandOutput(
            RequestCommandOutputResult::LongRunningCommandSnapshot {
                block_id: BlockId::from("b".to_owned()),
                command: "tail -f log".to_owned(),
                grid_contents: String::new(),
                cursor: String::new(),
                is_alt_screen_active: false,
                activity: None,
            },
        ),
        None,
    );
    assert!(
        !matches(after, None, snapshot),
        "a still-running command has not finished"
    );
    let denied = tool_failure(
        shell_action("rm -rf /"),
        AIAgentActionResultType::RequestCommandOutput(RequestCommandOutputResult::HookDenied {
            command: "rm -rf /".to_owned(),
            reason: "no".to_owned(),
        }),
        None,
    );
    assert!(
        !matches(after, None, denied),
        "a command that never ran did not finish"
    );
}

#[test]
fn mcp_events_match_the_bare_tool_name() {
    let pre = CursorEvent::BeforeMcpExecution;
    let call = tool_pre(mcp_action("read_file", json!({ "path": "a" })));
    assert!(matches(pre, Some("^read_file$"), call.clone()));
    assert!(!matches(pre, Some("^MCP:"), call.clone()));
    assert!(!matches(pre, Some("write"), call));
    assert!(!matches(pre, None, shell_pre("ls")));

    let after = CursorEvent::AfterMcpExecution;
    let finished = tool_result(
        mcp_action("read_file", json!({})),
        AIAgentActionResultType::CallMCPTool(CallMCPToolResult::Error("boom".to_owned())),
        None,
    );
    assert!(matches(after, Some("^read_file$"), finished));
    let cancelled = tool_failure(
        mcp_action("read_file", json!({})),
        AIAgentActionResultType::CallMCPTool(CallMCPToolResult::Cancelled),
        None,
    );
    assert!(!matches(after, None, cancelled));
}

#[test]
fn literal_events_match_their_fixed_word() {
    let stop = HookPayload::TurnStop {
        status: TurnStopStatus::Completed,
        continuation_count: 0,
    };
    assert!(matches(CursorEvent::Stop, Some("^Stop$"), stop.clone()));
    assert!(!matches(CursorEvent::Stop, Some("^stop$"), stop.clone()));
    assert!(!matches(CursorEvent::Stop, Some("Shell"), stop));

    let prompt = HookPayload::PromptSubmit {
        text: "hi".to_owned(),
        attachments_summary: Vec::new(),
    };
    assert!(matches(
        CursorEvent::BeforeSubmitPrompt,
        Some("UserPromptSubmit"),
        prompt
    ));
    let response = HookPayload::ResponseComplete {
        message_id: "m".to_owned(),
        text: "t".to_owned(),
    };
    assert!(matches(
        CursorEvent::AfterAgentResponse,
        Some("^AgentResponse$"),
        response
    ));
    let thought = HookPayload::ReasoningComplete {
        message_id: "m".to_owned(),
        text: "t".to_owned(),
        duration: None,
    };
    assert!(matches(
        CursorEvent::AfterAgentThought,
        Some("AgentThought"),
        thought
    ));
}

#[test]
fn events_without_a_matcher_target_match_every_occurrence() {
    let start = HookPayload::SessionStart {
        is_background_agent: false,
    };
    assert!(matches(CursorEvent::SessionStart, None, start.clone()));
    assert!(matches(CursorEvent::SessionStart, Some("*"), start));
    assert_eq!(
        matched_value(
            CursorEvent::SessionStart.spec(),
            &fixture_envelope(HookPayload::SessionStart {
                is_background_agent: true,
            })
        ),
        Some("")
    );
}

#[test]
fn a_binding_never_matches_an_envelope_of_another_boundary() {
    let result = tool_result(shell_action("ls"), completed_shell("ls", "", 0), None);
    assert!(!matches(
        CursorEvent::BeforeShellExecution,
        None,
        result.clone()
    ));
    assert!(!matches(CursorEvent::PreToolUse, None, result));
    assert!(!matches(
        CursorEvent::AfterShellExecution,
        None,
        shell_pre("ls")
    ));
    assert!(!matches(CursorEvent::PostToolUse, None, shell_pre("ls")));
    assert!(!matches(
        CursorEvent::Stop,
        None,
        HookPayload::SessionStart {
            is_background_agent: false,
        }
    ));
}

#[test]
fn events_warp_does_not_fire_yet_never_match_even_on_their_boundary() {
    let edit = tool_result(
        AIAgentActionType::RequestFileEdits {
            file_edits: Vec::new(),
            title: None,
        },
        AIAgentActionResultType::RequestFileEdits(RequestFileEditsResult::Cancelled),
        None,
    );
    assert!(matches(CursorEvent::PostToolUse, None, edit.clone()));
    assert!(
        !matches(CursorEvent::AfterFileEdit, None, edit),
        "afterFileEdit is a Phase 2 facet of tool.result"
    );
}

#[test]
fn compiled_matchers_are_cached_per_pattern() {
    let cache = MatcherCache::default();
    let first = cache.get("^git ").expect("compiles");
    let second = cache.get("^git ").expect("compiles");
    assert!(Arc::ptr_eq(&first, &second));
    assert!(cache.get("(").is_none());
}

#[test]
fn grep_is_a_tool_name_not_a_command() {
    let grep = tool_pre(AIAgentActionType::Grep {
        queries: vec!["rm".to_owned()],
        path: ".".to_owned(),
    });
    assert!(matches(
        CursorEvent::PreToolUse,
        Some("^Grep$"),
        grep.clone()
    ));
    assert!(!matches(
        CursorEvent::BeforeShellExecution,
        Some("rm"),
        grep
    ));
}

use std::ops::Range;
use std::time::Duration;

use chrono::{Local, TimeZone};
use serde_json::{Value, json};
use warp_terminal::event::ObservedExitStatus;
use warp_terminal::model::BlockId;

use super::*;
use crate::agent::FileLocations;
use crate::agent::action::{
    AIAgentActionType, FileEdit, ReadFilesRequest, RunAgentsAgentRunConfig, RunAgentsExecutionMode,
    RunAgentsRequest,
};
use crate::agent::action_result::{
    AIAgentActionResultType, AnyFileContent, CallMCPToolResult, FileContext, ReadFilesFailedFile,
    ReadFilesResult, RequestCommandOutputResult,
};
use crate::hooks::adapters::cursor::test_support::{
    EVENT_FIXTURES, FIXTURE_CONVERSATION_ID, completed_shell, fixture_envelope, json, mcp_action,
    shell_action, tool_failure, tool_pre, tool_result,
};
use crate::hooks::model::{HookTurnId, PromptProvenance, TurnStopStatus};

/// The payload each `stdin.json` fixture was written for.
fn fixture_payload(event: CursorEvent) -> HookPayload {
    match event {
        CursorEvent::SessionStart => HookPayload::SessionStart {
            is_background_agent: false,
        },
        CursorEvent::PreToolUse => tool_pre(AIAgentActionType::RequestCommandOutput {
            command: "rm -rf build".to_owned(),
            is_read_only: Some(false),
            is_risky: Some(true),
            wait_until_completion: true,
            uses_pager: None,
            rationale: Some("Clean the build directory".to_owned()),
            citations: Vec::new(),
        }),
        CursorEvent::PostToolUse => tool_result(
            shell_action("cargo test"),
            completed_shell("cargo test", "test result: ok", 0),
            Some(Duration::from_millis(1500)),
        ),
        CursorEvent::PostToolUseFailure => tool_failure(
            shell_action("rm -rf /"),
            AIAgentActionResultType::RequestCommandOutput(RequestCommandOutputResult::HookDenied {
                command: "rm -rf /".to_owned(),
                reason: "not allowed here".to_owned(),
            }),
            None,
        ),
        CursorEvent::BeforeShellExecution => tool_pre(shell_action("git push --force")),
        CursorEvent::AfterShellExecution => tool_result(
            shell_action("cargo test"),
            completed_shell("cargo test", "test result: ok", 0),
            Some(Duration::from_millis(1500)),
        ),
        CursorEvent::BeforeMcpExecution => {
            tool_pre(mcp_action("write_file", json!({ "path": "notes.txt" })))
        }
        // A finished MCP call is a `tool.result` whether it returned a result or an error; the
        // encoder reads a `tool.failure` payload the same way.
        CursorEvent::AfterMcpExecution => tool_result(
            mcp_action("write_file", json!({ "path": "notes.txt" })),
            AIAgentActionResultType::CallMCPTool(CallMCPToolResult::Error(
                "permission denied".to_owned(),
            )),
            Some(Duration::from_millis(250)),
        ),
        CursorEvent::BeforeSubmitPrompt => HookPayload::PromptSubmit {
            text: "Fix the failing test".to_owned(),
            attachments_summary: vec!["/work/repo/src/lib.rs".to_owned()],
            provenance: PromptProvenance::User,
        },
        CursorEvent::Stop => HookPayload::TurnStop {
            status: TurnStopStatus::Completed,
            continuation_count: 0,
        },
        CursorEvent::AfterAgentResponse => HookPayload::ResponseComplete {
            message_id: "m1".to_owned(),
            text: "Done. The tests pass.".to_owned(),
        },
        CursorEvent::AfterAgentThought => HookPayload::ReasoningComplete {
            message_id: "m0".to_owned(),
            text: "Let me check the tests first.".to_owned(),
            duration: Some(Duration::from_secs(2)),
        },
        CursorEvent::SessionEnd
        | CursorEvent::SubagentStart
        | CursorEvent::SubagentStop
        | CursorEvent::BeforeReadFile
        | CursorEvent::AfterFileEdit
        | CursorEvent::PreCompact
        | CursorEvent::BeforeTabFileRead
        | CursorEvent::AfterTabFileEdit
        | CursorEvent::WorkspaceOpen => panic!("{} has no fixture", event.name()),
    }
}

fn encode(event: CursorEvent, payload: HookPayload) -> Value {
    encode_input(event.spec(), &fixture_envelope(payload))
}

/// Cursor sends some fields as JSON text inside a string. Their key order is a serializer
/// detail, so they are compared as parsed JSON.
fn with_nested_json_parsed(mut value: Value) -> Value {
    if let Some(object) = value.as_object_mut() {
        for key in ["tool_input", "tool_output", "result_json"] {
            let parsed = object
                .get(key)
                .and_then(Value::as_str)
                .and_then(|text| serde_json::from_str::<Value>(text).ok());
            if let Some(parsed) = parsed {
                object.insert(key.to_owned(), parsed);
            }
        }
    }
    value
}

#[test]
fn every_phase_1_event_encodes_exactly_its_fixture() {
    for fixture in &EVENT_FIXTURES {
        let encoded = encode(fixture.event, fixture_payload(fixture.event));
        assert_eq!(
            with_nested_json_parsed(encoded),
            with_nested_json_parsed(json(fixture.stdin)),
            "{} stdin differs from fixtures/{}/stdin.json",
            fixture.event.name(),
            fixture.event.name()
        );
    }
}

#[test]
fn generation_id_is_the_turn_id_once_one_exists() {
    let mut envelope = fixture_envelope(tool_pre(shell_action("ls")));
    let turn = HookTurnId::new();
    envelope.turn_id = Some(turn);
    let input = encode_input(CursorEvent::BeforeShellExecution.spec(), &envelope);
    assert_eq!(input["generation_id"], turn.as_uuid().to_string());
    assert_eq!(input["conversation_id"], FIXTURE_CONVERSATION_ID);
}

#[test]
fn generation_id_falls_back_to_the_conversation_id_before_a_turn_exists() {
    let mut envelope = fixture_envelope(HookPayload::SessionStart {
        is_background_agent: false,
    });
    envelope.turn_id = None;
    for event in [
        CursorEvent::SessionStart,
        CursorEvent::BeforeShellExecution,
        CursorEvent::Stop,
    ] {
        let input = encode_input(event.spec(), &envelope);
        assert_eq!(
            input["generation_id"],
            FIXTURE_CONVERSATION_ID,
            "{}: the key is always present and never a made-up value",
            event.name()
        );
    }
}

#[test]
fn unknown_common_fields_are_absent_or_null_rather_than_invented() {
    let mut envelope = fixture_envelope(tool_pre(shell_action("ls")));
    envelope.model_id = None;
    envelope.user_email = None;
    envelope.action_id = None;
    envelope.roots = Vec::new();
    let input = encode_input(CursorEvent::BeforeShellExecution.spec(), &envelope);
    let object = input.as_object().expect("an object");
    assert!(!object.contains_key("model"));
    assert!(!object.contains_key("model_id"));
    assert!(!object.contains_key("tool_use_id"));
    assert_eq!(input["user_email"], Value::Null);
    assert_eq!(input["transcript_path"], Value::Null);
    assert_eq!(input["model_params"], json!([]));
    assert_eq!(
        input["workspace_roots"],
        json!(["/work/repo/src"]),
        "with no detected root the working directory is the workspace"
    );

    envelope.cwd = None;
    let input = encode_input(CursorEvent::BeforeShellExecution.spec(), &envelope);
    let object = input.as_object().expect("an object");
    assert_eq!(input["workspace_roots"], json!([]));
    assert!(!object.contains_key("cwd"));
    assert_eq!(input["command"], "ls");
}

#[test]
fn cursor_version_and_warp_version_both_carry_warps_version() {
    let input = encode(
        CursorEvent::Stop,
        HookPayload::TurnStop {
            status: TurnStopStatus::Error,
            continuation_count: 2,
        },
    );
    assert_eq!(input["cursor_version"], "v0.2026.01.01");
    assert_eq!(input["warp_version"], "v0.2026.01.01");
    assert_eq!(input["status"], "error");
    assert_eq!(input["loop_count"], 2);
    assert_eq!(
        encode(
            CursorEvent::Stop,
            HookPayload::TurnStop {
                status: TurnStopStatus::Cancelled,
                continuation_count: 0,
            },
        )["status"],
        "aborted"
    );
}

#[test]
fn after_shell_execution_takes_the_duration_from_timestamps_when_nothing_measured_it() {
    let start = Local
        .with_ymd_and_hms(2026, 1, 1, 12, 0, 0)
        .single()
        .expect("a time");
    let end = start + chrono::Duration::milliseconds(2500);
    let result =
        AIAgentActionResultType::RequestCommandOutput(RequestCommandOutputResult::Completed {
            block_id: BlockId::from("b".to_owned()),
            command: "make".to_owned(),
            output: "built".to_owned(),
            exit_code: 1.into(),
            start_ts: Some(start),
            completed_ts: Some(end),
        });
    let input = encode(
        CursorEvent::AfterShellExecution,
        tool_result(shell_action("make"), result, None),
    );
    assert_eq!(input["duration"], 2500);
    assert_eq!(input["exit_code"], 1);
    assert_eq!(input["output"], "built");
}

#[test]
fn a_recovered_shell_has_no_exit_code() {
    let result =
        AIAgentActionResultType::RequestCommandOutput(RequestCommandOutputResult::ShellRecovered {
            block_id: BlockId::from("b".to_owned()),
            command: "exit".to_owned(),
            output: String::new(),
            status: ObservedExitStatus::Code(0),
            restored_working_directory: "/work".to_owned(),
            used_fallback_directory: false,
            start_ts: None,
            completed_ts: None,
        });
    let input = encode(
        CursorEvent::AfterShellExecution,
        tool_result(shell_action("exit"), result, None),
    );
    let object = input.as_object().expect("an object");
    assert_eq!(input["command"], "exit");
    assert!(!object.contains_key("exit_code"));
    assert!(!object.contains_key("duration"));
}

#[test]
fn after_mcp_execution_carries_the_result_as_a_json_string() {
    let result =
        rmcp::model::CallToolResult::success(vec![rmcp::model::ContentBlock::text("hello")]);
    let input = encode(
        CursorEvent::AfterMcpExecution,
        tool_result(
            mcp_action("greet", json!({ "name": "x" })),
            AIAgentActionResultType::CallMCPTool(CallMCPToolResult::Success { result }),
            None,
        ),
    );
    assert_eq!(input["tool_name"], "greet");
    assert_eq!(input["tool_input"], r#"{"name":"x"}"#);
    let result_json: Value =
        serde_json::from_str(input["result_json"].as_str().expect("a string")).expect("JSON");
    assert_eq!(result_json["content"][0]["text"], "hello");
}

#[test]
fn a_cancelled_tool_is_an_interrupt_with_its_reason() {
    let input = encode(
        CursorEvent::PostToolUseFailure,
        tool_failure(
            shell_action("sleep 100"),
            AIAgentActionResultType::RequestCommandOutput(
                RequestCommandOutputResult::CancelledBeforeExecution,
            ),
            Some("user_cancelled"),
        ),
    );
    assert_eq!(input["failure_type"], "error");
    assert_eq!(input["is_interrupt"], true);
    assert_eq!(input["cancellation_reason"], "user_cancelled");
    assert_eq!(input["error_message"], "Command output cancelled");
}

#[test]
fn tool_input_describes_non_shell_actions() {
    let read = tool_pre(AIAgentActionType::ReadFiles(ReadFilesRequest {
        locations: vec![FileLocations {
            name: "src/lib.rs".to_owned(),
            lines: vec![Range { start: 1, end: 20 }],
        }],
    }));
    let input = encode(CursorEvent::PreToolUse, read);
    assert_eq!(input["tool_name"], "Read");
    assert_eq!(
        input["tool_input"],
        json!({ "files": [ { "path": "src/lib.rs", "lines": [[1, 20]] } ] })
    );

    let edit = tool_pre(AIAgentActionType::RequestFileEdits {
        file_edits: vec![
            FileEdit::Create {
                file: Some("new.rs".to_owned()),
                content: Some("fn main() {}".to_owned()),
                allow_overwrite: false,
            },
            FileEdit::Delete {
                file: Some("old.rs".to_owned()),
            },
        ],
        title: Some("Replace old.rs".to_owned()),
    });
    let input = encode(CursorEvent::PreToolUse, edit);
    assert_eq!(input["tool_name"], "Write");
    assert_eq!(
        input["tool_input"],
        json!({
            "title": "Replace old.rs",
            "edits": [
                { "kind": "create", "file": "new.rs", "content": "fn main() {}", "allow_overwrite": false },
                { "kind": "delete", "file": "old.rs" }
            ]
        })
    );

    let mcp = tool_pre(mcp_action("read_file", json!({ "path": "a" })));
    let input = encode(CursorEvent::PreToolUse, mcp);
    assert_eq!(input["tool_name"], "MCP:read_file");
    assert_eq!(
        input["tool_input"],
        json!({
            "server_id": "22222222-2222-2222-2222-222222222222",
            "name": "read_file",
            "input": { "path": "a" }
        })
    );

    let agents = tool_pre(AIAgentActionType::RunAgents(Box::new(RunAgentsRequest {
        metadata: Default::default(),
        summary: "Split the work".to_owned(),
        base_prompt: "Help".to_owned(),
        skills: Vec::new(),
        model_id: "m".to_owned(),
        harness_type: "oz".to_owned(),
        execution_mode: RunAgentsExecutionMode::Local,
        agent_run_configs: vec![RunAgentsAgentRunConfig {
            metadata: Default::default(),
            name: "child".to_owned(),
            prompt: "Do part one".to_owned(),
            title: String::new(),
            agent_identity_uid: String::new(),
            model_id: String::new(),
        }],
        plan_id: String::new(),
        harness_auth_secret_name: Some("secret".to_owned()),
    })));
    let input = encode(CursorEvent::PreToolUse, agents);
    assert_eq!(input["tool_name"], "Task");
    assert_eq!(input["tool_input"]["summary"], "Split the work");
    assert_eq!(input["tool_input"]["execution_mode"], "local");
    assert_eq!(input["tool_input"]["agents"][0]["name"], "child");
    assert!(
        !input["tool_input"].to_string().contains("secret"),
        "the harness secret name is not part of the input"
    );
}

#[test]
fn tool_output_summarizes_binary_content_and_keeps_text() {
    let result = AIAgentActionResultType::ReadFiles(ReadFilesResult::Success {
        files: vec![
            FileContext::new(
                "a.txt".to_owned(),
                AnyFileContent::StringContent("hello\nworld".to_owned()),
                None,
                None,
            ),
            FileContext::new(
                "logo.png".to_owned(),
                AnyFileContent::BinaryContent(vec![0, 1, 2]),
                None,
                None,
            ),
        ],
        failed_files: vec![ReadFilesFailedFile {
            path: "missing.txt".to_owned(),
            message: "not found".to_owned(),
        }],
    });
    let input = encode(
        CursorEvent::PostToolUse,
        tool_result(
            AIAgentActionType::ReadFiles(ReadFilesRequest {
                locations: Vec::new(),
            }),
            result,
            None,
        ),
    );
    let output: Value =
        serde_json::from_str(input["tool_output"].as_str().expect("a string")).expect("JSON");
    assert_eq!(output["kind"], "success");
    assert_eq!(output["files"][0]["content"], "hello\nworld");
    assert_eq!(output["files"][0]["line_count"], 2);
    assert_eq!(output["files"][1]["is_binary"], true);
    assert_eq!(output["files"][1]["size_bytes"], 3);
    assert!(output["files"][1].get("content").is_none());
    assert_eq!(output["failed_files"][0]["path"], "missing.txt");
    let object = input.as_object().expect("an object");
    assert!(!object.contains_key("duration"));
}

#[test]
fn events_warp_does_not_fire_encode_only_the_common_fields() {
    let input = encode(
        CursorEvent::SessionEnd,
        HookPayload::SessionStart {
            is_background_agent: true,
        },
    );
    let mut keys = input
        .as_object()
        .expect("an object")
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    keys.sort();
    assert_eq!(
        keys,
        vec![
            "conversation_id",
            "cursor_version",
            "generation_id",
            "hook_event_name",
            "model",
            "model_id",
            "model_params",
            "transcript_path",
            "user_email",
            "warp_version",
            "workspace_roots",
        ]
    );
    assert_eq!(input["hook_event_name"], "sessionEnd");
}

#[test]
fn an_event_given_a_payload_it_is_not_about_adds_nothing() {
    let input = encode(
        CursorEvent::BeforeShellExecution,
        tool_pre(mcp_action("read_file", json!({}))),
    );
    let object = input.as_object().expect("an object");
    assert!(!object.contains_key("command"));
    assert_eq!(input["hook_event_name"], "beforeShellExecution");
}

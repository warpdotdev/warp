use std::path::PathBuf;
use std::time::Duration;

use ai_types::{AIAgentActionId, AIConversationId};

use super::*;
use crate::LLMId;
use crate::agent::action::ReadFilesRequest;
use crate::hooks::adapters::adapter_for;
use crate::hooks::model::{CanonicalToolName, HookTurnId};
use crate::hooks::test_util::{test_envelope, test_source_id};

fn shell_action(command: &str) -> AIAgentActionType {
    AIAgentActionType::RequestCommandOutput {
        command: command.to_owned(),
        is_read_only: None,
        is_risky: None,
        wait_until_completion: true,
        uses_pager: None,
        rationale: None,
        citations: Vec::new(),
    }
}

fn shell_envelope(command: &str) -> HookEnvelope {
    let action = shell_action(command);
    let tool_name = CanonicalToolName::for_action(&action);
    test_envelope(
        AIConversationId::new(),
        HookPayload::ToolPre { action, tool_name },
    )
}

fn parsed(text: &str) -> ParsedHookConfig {
    CursorAdapter::new()
        .parse(text.as_bytes(), &test_source_id("hooks.json"))
        .expect("parses")
}

#[test]
fn the_cursor_adapter_is_registered() {
    let adapter = adapter_for(HookFormat::Cursor).expect("registered");
    assert_eq!(adapter.format(), HookFormat::Cursor);
}

#[test]
fn bindings_are_declared_per_definition_in_file_order() {
    let adapter = CursorAdapter::new();
    let config = parsed(
        r#"{ "version": 1, "hooks": {
            "afterShellExecution": [ { "command": "./audit.sh" } ],
            "beforeShellExecution": [
                { "command": "./first.sh", "matcher": "rm", "timeout": 3, "failClosed": true },
                { "command": "./second.sh" }
            ]
        } }"#,
    );
    assert_eq!(config.warnings.len(), 1);
    assert!(
        config.warnings[0].message.contains("afterShellExecution"),
        "{}",
        config.warnings[0].message
    );

    let bindings = adapter.bindings(&config);
    assert_eq!(bindings.len(), 2);

    let first = &bindings[0];
    assert_eq!(first.id.source, config.source);
    assert_eq!(first.id.event, "beforeShellExecution");
    assert_eq!(first.id.index, 0);
    assert_eq!(first.boundary, HookBoundary::ToolPre);
    assert_eq!(first.vendor_event, "beforeShellExecution");
    assert_eq!(first.command, "./first.sh");
    assert_eq!(first.matcher.as_deref(), Some("rm"));
    assert_eq!(first.timeout, Duration::from_secs(3));
    assert_eq!(first.capabilities, shell_permission_capabilities(true));

    let second = &bindings[1];
    assert_eq!(second.id.index, 1);
    assert_eq!(second.command, "./second.sh");
    assert_eq!(second.capabilities, shell_permission_capabilities(false));
}

#[test]
fn a_prompt_hook_becomes_a_warning_and_the_command_hook_after_it_keeps_its_index() {
    let adapter = CursorAdapter::new();
    let config = parsed(
        r#"{ "version": 1, "hooks": { "beforeShellExecution": [
            { "type": "prompt", "prompt": "Is this safe?" },
            { "command": "./check.sh" }
        ] } }"#,
    );
    assert_eq!(config.warnings.len(), 1);
    assert_eq!(
        config.warnings[0].message,
        "beforeShellExecution[0]: prompt hooks are not supported yet; it was skipped"
    );
    assert_eq!(config.warnings[0].severity, DiagnosticSeverity::Warning);

    let bindings = adapter.bindings(&config);
    assert_eq!(bindings.len(), 1);
    assert_eq!(
        bindings[0].id.index, 1,
        "the id keeps the position in the file"
    );
    assert_eq!(bindings[0].command, "./check.sh");
}

#[test]
fn matching_tests_the_command_text_with_an_unanchored_case_sensitive_regex() {
    let adapter = CursorAdapter::new();
    let config = parsed(
        r#"{ "version": 1, "hooks": { "beforeShellExecution": [
            { "command": "a", "matcher": "rm -rf" },
            { "command": "b", "matcher": "" },
            { "command": "c", "matcher": "*" },
            { "command": "d" },
            { "command": "e", "matcher": "^git (push|commit)" }
        ] } }"#,
    );
    let bindings = adapter.bindings(&config);
    let envelope = shell_envelope("sudo rm -rf /tmp/x");

    assert!(adapter.matches(&bindings[0], &envelope));
    assert!(adapter.matches(&bindings[1], &envelope));
    assert!(adapter.matches(&bindings[2], &envelope));
    assert!(adapter.matches(&bindings[3], &envelope));
    assert!(!adapter.matches(&bindings[4], &envelope));

    assert!(!adapter.matches(&bindings[0], &shell_envelope("RM -RF /tmp/x")));
    assert!(adapter.matches(&bindings[4], &shell_envelope("git push origin main")));
    assert!(!adapter.matches(&bindings[4], &shell_envelope("echo git push")));
}

#[test]
fn only_shell_commands_match() {
    let adapter = CursorAdapter::new();
    let config =
        parsed(r#"{ "version": 1, "hooks": { "beforeShellExecution": [ { "command": "a" } ] } }"#);
    let binding = &adapter.bindings(&config)[0];

    let read = AIAgentActionType::ReadFiles(ReadFilesRequest {
        locations: Vec::new(),
    });
    let tool_name = CanonicalToolName::for_action(&read);
    let read_envelope = test_envelope(
        AIConversationId::new(),
        HookPayload::ToolPre {
            action: read,
            tool_name,
        },
    );
    assert!(!adapter.matches(binding, &read_envelope));

    let turn_stop = test_envelope(
        AIConversationId::new(),
        HookPayload::TurnStop {
            status: crate::hooks::model::TurnStopStatus::Completed,
            continuation_count: 0,
        },
    );
    assert!(!adapter.matches(binding, &turn_stop));
}

#[test]
fn stdin_is_cursors_before_shell_execution_shape() {
    let adapter = CursorAdapter::new();
    let config =
        parsed(r#"{ "version": 1, "hooks": { "beforeShellExecution": [ { "command": "a" } ] } }"#);
    let binding = &adapter.bindings(&config)[0];

    let mut envelope = shell_envelope("rm -rf /tmp/x");
    envelope.cwd = Some(PathBuf::from("/work/repo/src"));
    envelope.roots = vec![PathBuf::from("/work/repo")];
    envelope.model_id = Some(LLMId::from("claude-sonnet".to_owned()));
    envelope.user_email = Some("dev@example.com".to_owned());
    envelope.action_id = Some(AIAgentActionId::from("call_1".to_owned()));
    envelope.warp_version = "v0.2026.01.01".to_owned();
    let turn = HookTurnId::new();
    envelope.turn_id = Some(turn);

    let input = adapter.encode_input(binding, &envelope);
    assert_eq!(input["hook_event_name"], "beforeShellExecution");
    assert_eq!(
        input["conversation_id"],
        envelope.conversation_id.to_string()
    );
    assert_eq!(input["generation_id"], turn.as_uuid().to_string());
    assert_eq!(input["model"], "claude-sonnet");
    assert_eq!(input["cursor_version"], "v0.2026.01.01");
    assert_eq!(input["workspace_roots"], serde_json::json!(["/work/repo"]));
    assert_eq!(input["user_email"], "dev@example.com");
    assert_eq!(input["transcript_path"], serde_json::Value::Null);
    assert_eq!(input["command"], "rm -rf /tmp/x");
    assert_eq!(input["cwd"], "/work/repo/src");
    assert_eq!(input["tool_use_id"], "call_1");
}

#[test]
fn unknown_common_fields_are_absent_or_null_rather_than_invented() {
    let adapter = CursorAdapter::new();
    let config =
        parsed(r#"{ "version": 1, "hooks": { "beforeShellExecution": [ { "command": "a" } ] } }"#);
    let binding = &adapter.bindings(&config)[0];

    let mut envelope = shell_envelope("ls");
    envelope.cwd = Some(PathBuf::from("/work/elsewhere"));
    let input = adapter.encode_input(binding, &envelope);
    let object = input.as_object().expect("an object");

    assert_eq!(
        input["generation_id"],
        envelope.conversation_id.to_string(),
        "no turn id yet, so the conversation id stands in"
    );
    assert_eq!(input["user_email"], serde_json::Value::Null);
    assert!(!object.contains_key("model"));
    assert!(!object.contains_key("tool_use_id"));
    assert_eq!(
        input["workspace_roots"],
        serde_json::json!(["/work/elsewhere"]),
        "with no detected root the working directory is the workspace"
    );
}

#[test]
fn a_bad_matcher_rejects_the_whole_file() {
    let error = CursorAdapter::new()
        .parse(
            br#"{ "version": 1, "hooks": { "beforeShellExecution": [ { "command": "a", "matcher": "[" } ] } }"#,
            &test_source_id("hooks.json"),
        )
        .expect_err("rejects");
    assert!(matches!(error, HookConfigError::Invalid(_)));
}

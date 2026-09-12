use serde_json::json;

use super::*;
use crate::ai::agent_sdk::hooks::redaction::RedactedValue;

fn context() -> HookPayloadContext {
    HookPayloadContext {
        session_id: "session".into(),
        run_id: "run".into(),
        conversation_id: "conversation".into(),
        cwd: "/workspace/repo".into(),
        model: "model".into(),
        permission_mode: "supervised".into(),
    }
}

fn payload(event: HookEventFields) -> serde_json::Value {
    serde_json::from_slice(
        &HookPayloadTemplate {
            context: context(),
            event,
        }
        .serialize_for_source(HookConfigSource::User)
        .unwrap(),
    )
    .unwrap()
}

#[test]
fn oz_hooks_payload_golden_covers_all_seven_events() {
    let events = [
        (
            HookEventName::SessionStart,
            HookEventFields::SessionStart {
                source: SessionStartSource::Startup,
            },
        ),
        (
            HookEventName::SessionEnd,
            HookEventFields::SessionEnd {
                reason: SessionEndReason::Completed,
            },
        ),
        (
            HookEventName::UserPromptSubmit,
            HookEventFields::user_prompt(RedactedText {
                value: "prompt".into(),
                truncation: None,
            }),
        ),
        (
            HookEventName::Stop,
            HookEventFields::Stop {
                turn_status: TurnStatus::Completed,
            },
        ),
        (
            HookEventName::PreToolUse,
            HookEventFields::PreToolUse {
                tool_name: "run_shell_command".into(),
                tool_use_id: "tool".into(),
                tool_input: RedactedValue::object([("command", "pwd".into())]),
            },
        ),
        (
            HookEventName::PostToolUse,
            HookEventFields::PostToolUse {
                tool_name: "run_shell_command".into(),
                tool_use_id: "tool".into(),
                tool_input: RedactedValue::object([("command", "pwd".into())]),
                tool_response: RedactedValue::object([("status", "succeeded".into())]),
            },
        ),
        (
            HookEventName::PreCompact,
            HookEventFields::PreCompact {
                trigger: CompactTrigger::Auto,
            },
        ),
    ];

    for (event_name, fields) in events {
        let value = payload(fields);
        assert_eq!(value["schema_version"], PAYLOAD_SCHEMA_VERSION);
        assert_eq!(value["hook_event_name"], event_name.as_str());
        assert_eq!(value["session_id"], "session");
        assert_eq!(value["run_id"], "run");
        assert_eq!(value["conversation_id"], "conversation");
        assert_eq!(value["cwd"], "/workspace/repo");
        assert_eq!(value["hook_source"], "user");
        assert_eq!(value["model"], "model");
        assert_eq!(value["permission_mode"], "supervised");
    }
}

#[test]
fn oz_hooks_payload_matches_command_facing_shape() {
    let value = payload(HookEventFields::PreToolUse {
        tool_name: "apply_patch".into(),
        tool_use_id: "tool-1".into(),
        tool_input: RedactedValue::object([("path", "src/lib.rs".into())]),
    });

    assert_eq!(
        value,
        json!({
            "schema_version": "warp.oz_hook.v1",
            "hook_event_name": "PreToolUse",
            "session_id": "session",
            "run_id": "run",
            "conversation_id": "conversation",
            "cwd": "/workspace/repo",
            "hook_source": "user",
            "model": "model",
            "permission_mode": "supervised",
            "tool_name": "apply_patch",
            "tool_use_id": "tool-1",
            "tool_input": {"path": "src/lib.rs"}
        })
    );
}

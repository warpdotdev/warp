use std::collections::BTreeMap;
use std::time::{Duration, SystemTime};

use prost_types::value::Kind;
use prost_types::{Struct, Value};
use warp_multi_agent_api::oz_hook_result::{Outcome, ResolvedAction};
use warp_multi_agent_api::{OzHookEvent as ProtocolEvent, RunOzHook};

use super::*;
use crate::ai::agent_sdk::hooks::runtime::{
    HookFailureCategory, HookInvocationDiagnostic, HookInvocationResult,
};
use crate::ai::agent_sdk::hooks::{HookConfigSource, HookEventName};

fn string(value: &str) -> Value {
    Value {
        kind: Some(Kind::StringValue(value.into())),
    }
}

fn object(fields: impl IntoIterator<Item = (&'static str, Value)>) -> Value {
    Value {
        kind: Some(Kind::StructValue(Struct {
            fields: fields
                .into_iter()
                .map(|(key, value)| (key.into(), value))
                .collect(),
        })),
    }
}

fn common_fields() -> BTreeMap<String, Value> {
    [
        ("schema_version", string(PAYLOAD_SCHEMA_VERSION)),
        ("session_id", string("session")),
        ("run_id", string("run")),
        ("conversation_id", string("conversation")),
        ("cwd", string("/workspace")),
        ("model", string("model")),
        ("permission_mode", string("supervised")),
    ]
    .into_iter()
    .map(|(key, value)| (key.into(), value))
    .collect()
}

fn action(
    event: ProtocolEvent,
    event_fields: impl IntoIterator<Item = (&'static str, Value)>,
) -> RunOzHook {
    let mut fields = common_fields();
    let hook_event_name = match event {
        ProtocolEvent::SessionStart => "SessionStart",
        ProtocolEvent::SessionEnd => "SessionEnd",
        ProtocolEvent::UserPromptSubmit => "UserPromptSubmit",
        ProtocolEvent::Stop => "Stop",
        ProtocolEvent::PreToolUse => "PreToolUse",
        ProtocolEvent::PostToolUse => "PostToolUse",
        ProtocolEvent::PreCompact => "PreCompact",
        ProtocolEvent::Unspecified => "Unspecified",
    };
    fields.insert("hook_event_name".into(), string(hook_event_name));
    fields.extend(
        event_fields
            .into_iter()
            .map(|(key, value)| (key.into(), value)),
    );
    let tool_use_id = matches!(
        event,
        ProtocolEvent::PreToolUse | ProtocolEvent::PostToolUse
    )
    .then(|| "tool-use".into())
    .unwrap_or_default();
    RunOzHook {
        invocation_id: "invocation".into(),
        tool_use_id,
        event: event.into(),
        schema_version: PAYLOAD_SCHEMA_VERSION.into(),
        redacted_payload: Some(Struct { fields }),
    }
}

#[test]
fn oz_hooks_protocol_requires_envelope_fields_and_rejects_non_tool_ids() {
    for field in ["schema_version", "hook_event_name"] {
        let mut missing = action(ProtocolEvent::Stop, [("turn_status", string("idle"))]);
        missing
            .redacted_payload
            .as_mut()
            .unwrap()
            .fields
            .remove(field);
        assert!(matches!(
            event_from_protocol(&missing),
            Err(ProtocolHookError::MissingPayloadField(missing_field))
                if missing_field == field
        ));
    }

    let mut non_tool = action(ProtocolEvent::Stop, [("turn_status", string("idle"))]);
    non_tool.tool_use_id = "unexpected".into();
    assert!(matches!(
        event_from_protocol(&non_tool),
        Err(ProtocolHookError::InvalidToolUseId)
    ));
}

#[test]
fn oz_hooks_protocol_parses_all_seven_protocol_events() {
    let cases = [
        (
            action(ProtocolEvent::SessionStart, [("source", string("startup"))]),
            HookEventName::SessionStart,
        ),
        (
            action(ProtocolEvent::SessionEnd, [("reason", string("completed"))]),
            HookEventName::SessionEnd,
        ),
        (
            action(
                ProtocolEvent::UserPromptSubmit,
                [("prompt", string("hello"))],
            ),
            HookEventName::UserPromptSubmit,
        ),
        (
            action(ProtocolEvent::Stop, [("turn_status", string("completed"))]),
            HookEventName::Stop,
        ),
        (
            action(
                ProtocolEvent::PreToolUse,
                [
                    ("tool_name", string("run_shell_command")),
                    ("tool_use_id", string("tool-use")),
                    ("tool_input", object([("command", string("pwd"))])),
                ],
            ),
            HookEventName::PreToolUse,
        ),
        (
            action(
                ProtocolEvent::PostToolUse,
                [
                    ("tool_name", string("run_shell_command")),
                    ("tool_use_id", string("tool-use")),
                    ("tool_input", object([("command", string("pwd"))])),
                    ("tool_response", object([("exit_code", string("0"))])),
                ],
            ),
            HookEventName::PostToolUse,
        ),
        (
            action(ProtocolEvent::PreCompact, [("trigger", string("auto"))]),
            HookEventName::PreCompact,
        ),
    ];

    for (action, expected) in cases {
        let event = event_from_protocol(&action).expect("event should parse");
        assert_eq!(event.payload.event_name(), expected);
    }
}

#[test]
fn oz_hooks_protocol_rejects_unknown_mismatched_and_source_specific_fields() {
    let mut unknown = action(ProtocolEvent::Stop, [("turn_status", string("idle"))]);
    unknown
        .redacted_payload
        .as_mut()
        .unwrap()
        .fields
        .insert("unknown".into(), string("value"));
    assert!(matches!(
        event_from_protocol(&unknown),
        Err(ProtocolHookError::UnknownPayloadField)
    ));

    let mut mismatched = action(ProtocolEvent::Stop, [("turn_status", string("idle"))]);
    mismatched
        .redacted_payload
        .as_mut()
        .unwrap()
        .fields
        .insert("hook_event_name".into(), string("PreCompact"));
    assert!(matches!(
        event_from_protocol(&mismatched),
        Err(ProtocolHookError::MismatchedEvent)
    ));

    let mut sourced = action(ProtocolEvent::Stop, [("turn_status", string("idle"))]);
    sourced
        .redacted_payload
        .as_mut()
        .unwrap()
        .fields
        .insert("hook_source".into(), string("project"));
    assert!(matches!(
        event_from_protocol(&sourced),
        Err(ProtocolHookError::UnexpectedHookSource)
    ));
}

#[test]
fn oz_hooks_protocol_rejects_unsupported_schema_and_tool_use_mismatch() {
    let mut unsupported = action(ProtocolEvent::Stop, [("turn_status", string("idle"))]);
    unsupported.schema_version = "future".into();
    assert!(matches!(
        event_from_protocol(&unsupported),
        Err(ProtocolHookError::UnsupportedSchema)
    ));

    let mut mismatched = action(
        ProtocolEvent::PreToolUse,
        [
            ("tool_name", string("run_shell_command")),
            ("tool_use_id", string("different")),
            ("tool_input", object([])),
        ],
    );
    mismatched.tool_use_id = "tool-use".into();
    assert!(matches!(
        event_from_protocol(&mismatched),
        Err(ProtocolHookError::MismatchedToolUseId)
    ));
}

fn diagnostic(
    result: HookInvocationResult,
    failure_category: Option<HookFailureCategory>,
) -> HookInvocationDiagnostic {
    HookInvocationDiagnostic {
        event: HookEventName::PreToolUse,
        source: HookConfigSource::Project,
        config_path: ".warp/hooks.json".into(),
        definition_hash: "hash".into(),
        matcher: None,
        started_at: SystemTime::UNIX_EPOCH,
        finished_at: SystemTime::UNIX_EPOCH,
        duration: Duration::ZERO,
        result,
        exit_code: None,
        output_truncated: false,
        failure_category,
    }
}

#[test]
fn oz_hooks_protocol_maps_continue_deny_failed_and_cancelled_results() {
    let action = action(
        ProtocolEvent::PreToolUse,
        [
            ("tool_name", string("run_shell_command")),
            ("tool_use_id", string("tool-use")),
            ("tool_input", object([])),
        ],
    );

    assert!(matches!(
        result_for_observation(&action, &[]).outcome,
        Some(Outcome::Continue(_))
    ));
    assert!(matches!(
        result_for_pre_tool(
            &action,
            OzPreToolUseDecision::Deny {
                reason: "no".into(),
                source: HookConfigSource::Project,
                diagnostics: vec![],
            }
        )
        .outcome,
        Some(Outcome::Deny(_))
    ));
    let failed = result_for_observation(
        &action,
        &[diagnostic(
            HookInvocationResult::Continued,
            Some(HookFailureCategory::Timeout),
        )],
    );
    assert!(matches!(
        failed.outcome,
        Some(Outcome::Failed(ref outcome))
            if outcome.resolved_action == ResolvedAction::Continue as i32
    ));
    assert!(matches!(
        result_for_observation(
            &action,
            &[diagnostic(
                HookInvocationResult::Cancelled,
                Some(HookFailureCategory::Cancelled),
            )],
        )
        .outcome,
        Some(Outcome::Cancelled(_))
    ));
    assert!(matches!(
        result_for_pre_tool(
            &action,
            OzPreToolUseDecision::Deny {
                reason: "explicit".into(),
                source: HookConfigSource::Project,
                diagnostics: vec![
                    diagnostic(
                        HookInvocationResult::Continued,
                        Some(HookFailureCategory::Timeout),
                    ),
                    diagnostic(HookInvocationResult::Denied, None),
                ],
            },
        )
        .outcome,
        Some(Outcome::Deny(ref deny)) if deny.reason == "explicit"
    ));
}

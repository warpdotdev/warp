use std::fs;
use std::sync::Arc;

use serde_json::json;

use super::*;
use crate::ai::agent_sdk::hooks::config::load_hook_config;
use crate::ai::agent_sdk::hooks::payload::{
    HookEventFields, HookPayloadContext, SessionStartSource,
};
use crate::ai::agent_sdk::hooks::trust::DenyProjectHookTrust;

fn runtime_with_hooks(hooks: serde_json::Value) -> OzHookRuntimeService {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("hooks.json");
    fs::write(
        &path,
        serde_json::to_vec(&json!({
            "schema_version": "warp.oz_hooks.config.v1",
            "hooks": hooks
        }))
        .unwrap(),
    )
    .unwrap();
    let snapshot = load_hook_config(Some(&path), None, &DenyProjectHookTrust);
    OzHookRuntimeService::new(snapshot)
}

fn event(invocation_id: &str, fields: HookEventFields) -> OzHookEvent {
    OzHookEvent {
        invocation_id: invocation_id.into(),
        tool_use_id: None,
        payload: HookPayloadTemplate {
            context: HookPayloadContext {
                session_id: "session".into(),
                run_id: "run".into(),
                conversation_id: "conversation".into(),
                cwd: std::env::current_dir()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
                model: "model".into(),
                permission_mode: "supervised".into(),
            },
            event: fields,
        },
    }
}

fn pre_tool_event(invocation_id: &str) -> OzPreToolUseEvent {
    OzPreToolUseEvent::new(event(
        invocation_id,
        HookEventFields::PreToolUse {
            tool_name: "run_shell_command".into(),
            tool_use_id: "tool".into(),
            tool_input: super::super::redaction::RedactedValue::object([] as [(&str, _); 0]),
        },
    ))
    .unwrap()
}

#[tokio::test]
async fn oz_hooks_runtime_runs_matching_handlers_sequentially() {
    let output = tempfile::NamedTempFile::new().unwrap();
    let path = output.path().to_string_lossy();
    let runtime = runtime_with_hooks(json!({
        "SessionStart": [{"hooks": [
            {"type": "command", "command": format!("printf first >> '{path}'")},
            {"type": "command", "command": format!("printf second >> '{path}'")}
        ]}]
    }));

    let observation = runtime
        .observe(event(
            "one",
            HookEventFields::SessionStart {
                source: SessionStartSource::Startup,
            },
        ))
        .await;

    assert_eq!(fs::read_to_string(output.path()).unwrap(), "firstsecond");
    assert_eq!(observation.diagnostics.len(), 2);
}

#[tokio::test]
async fn oz_hooks_runtime_structured_deny_short_circuits_later_handlers() {
    let marker = tempfile::NamedTempFile::new().unwrap();
    let marker_path = marker.path().to_string_lossy();
    let deny = r#"{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny","permissionDecisionReason":"policy"}}"#;
    let runtime = runtime_with_hooks(json!({
        "PreToolUse": [{"hooks": [
            {"type": "command", "command": format!("printf '%s' '{deny}'")},
            {"type": "command", "command": format!("printf ran >> '{marker_path}'")}
        ]}]
    }));

    let decision = runtime.pre_tool_use(pre_tool_event("deny")).await;

    assert!(matches!(
        decision,
        OzPreToolUseDecision::Deny { ref reason, .. } if reason == "policy"
    ));
    assert_eq!(fs::read_to_string(marker.path()).unwrap(), "");
}

#[tokio::test]
async fn oz_hooks_runtime_exit_two_denies_only_pre_tool_use() {
    let runtime = runtime_with_hooks(json!({
        "PreToolUse": [{"hooks": [
            {"type": "command", "command": "printf policy >&2; exit 2"}
        ]}],
        "Stop": [{"hooks": [
            {"type": "command", "command": "printf ignored >&2; exit 2"}
        ]}]
    }));

    assert!(matches!(
        runtime.pre_tool_use(pre_tool_event("pre")).await,
        OzPreToolUseDecision::Deny { ref reason, .. } if reason == "policy"
    ));
    let observation = runtime
        .observe(event(
            "stop",
            HookEventFields::Stop {
                turn_status: super::super::payload::TurnStatus::Completed,
            },
        ))
        .await;
    assert_eq!(
        observation.diagnostics[0].failure_category,
        Some(HookFailureCategory::NonZeroExit)
    );
}

#[tokio::test]
async fn oz_hooks_runtime_invalid_allow_output_fails_open_or_closed() {
    let allow = r#"{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"allow","permissionDecisionReason":"no"}}"#;
    for (mode, denied) in [("continue", false), ("deny", true)] {
        let runtime = runtime_with_hooks(json!({
            "PreToolUse": [{"hooks": [{
                "type": "command",
                "command": format!("printf '%s' '{allow}'"),
                "on_failure": mode
            }]}]
        }));

        let decision = runtime.pre_tool_use(pre_tool_event(mode)).await;

        assert_eq!(
            matches!(decision, OzPreToolUseDecision::Deny { .. }),
            denied
        );
    }
}

#[tokio::test]
async fn oz_hooks_runtime_timeout_kills_and_resolves_failure_mode() {
    let temp = tempfile::tempdir().unwrap();
    let sentinel = temp.path().join("descendant-survived");
    let runtime = runtime_with_hooks(json!({
        "PreToolUse": [{"hooks": [{
            "type": "command",
            "command": format!(
                "(sleep 2; printf survived > '{}') & sleep 30",
                sentinel.display()
            ),
            "timeout": 1,
            "on_failure": "deny"
        }]}]
    }));

    let started = Instant::now();
    let decision = runtime.pre_tool_use(pre_tool_event("timeout")).await;

    assert!(started.elapsed() < Duration::from_secs(5));
    assert!(matches!(decision, OzPreToolUseDecision::Deny { .. }));
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(!sentinel.exists());
}

#[tokio::test]
async fn oz_hooks_runtime_rejects_oversized_output() {
    let runtime = runtime_with_hooks(json!({
        "PreToolUse": [{"hooks": [{
            "type": "command",
            "command": "head -c 70000 /dev/zero",
            "on_failure": "continue"
        }]}]
    }));

    let decision = runtime.pre_tool_use(pre_tool_event("overflow")).await;

    let OzPreToolUseDecision::Continue { diagnostics } = decision else {
        panic!("overflow should fail open");
    };
    assert_eq!(
        diagnostics[0].failure_category,
        Some(HookFailureCategory::OutputOverflow)
    );
}

#[tokio::test]
async fn oz_hooks_runtime_cancellation_removes_pending_event() {
    let runtime = Arc::new(runtime_with_hooks(json!({
        "SessionStart": [{"hooks": [{
            "type": "command",
            "command": "sleep 30"
        }]}]
    })));
    let running = {
        let runtime = Arc::clone(&runtime);
        tokio::spawn(async move {
            runtime
                .observe(event(
                    "cancel",
                    HookEventFields::SessionStart {
                        source: SessionStartSource::Startup,
                    },
                ))
                .await
        })
    };
    tokio::task::yield_now().await;

    runtime.cancel(OzHookCancellationScope::Invocation("cancel".into()));
    let observation = tokio::time::timeout(Duration::from_secs(5), running)
        .await
        .unwrap()
        .unwrap();

    assert_eq!(
        observation.diagnostics[0].result,
        HookInvocationResult::Cancelled
    );
}

#[tokio::test]
async fn oz_hooks_runtime_cancellation_while_queued_is_not_continue() {
    let runtime = Arc::new(runtime_with_hooks(json!({
        "SessionStart": [{"hooks": [{
            "type": "command",
            "command": "sleep 30"
        }]}],
        "PreToolUse": [{"hooks": [{
            "type": "command",
            "command": "exit 0"
        }]}]
    })));
    let blocker = {
        let runtime = Arc::clone(&runtime);
        tokio::spawn(async move {
            runtime
                .observe(event(
                    "blocker",
                    HookEventFields::SessionStart {
                        source: SessionStartSource::Startup,
                    },
                ))
                .await
        })
    };
    tokio::time::sleep(Duration::from_millis(100)).await;
    let queued = {
        let runtime = Arc::clone(&runtime);
        tokio::spawn(async move { runtime.pre_tool_use(pre_tool_event("queued")).await })
    };
    tokio::task::yield_now().await;

    runtime.cancel(OzHookCancellationScope::Invocation("queued".into()));
    let decision = tokio::time::timeout(Duration::from_secs(5), queued)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(decision, OzPreToolUseDecision::Cancelled { .. }));

    runtime.cancel(OzHookCancellationScope::Invocation("blocker".into()));
    blocker.await.unwrap();
}

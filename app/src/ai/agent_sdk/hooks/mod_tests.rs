use std::sync::Arc;

use async_trait::async_trait;

use super::payload::HookPayloadContext;
use super::redaction::HookRedactor;
use super::runtime::{
    OzHookCancellationScope, OzHookEvent, OzHookObservation, OzHookRuntime, OzPreToolUseDecision,
    OzPreToolUseEvent,
};
use super::{HookEventName, OzHookSession};

struct NoopRuntime;

#[async_trait]
impl OzHookRuntime for NoopRuntime {
    async fn observe(&self, _: OzHookEvent) -> OzHookObservation {
        OzHookObservation::default()
    }

    async fn pre_tool_use(&self, _: OzPreToolUseEvent) -> OzPreToolUseDecision {
        OzPreToolUseDecision::Continue {
            diagnostics: Vec::new(),
        }
    }

    fn cancel(&self, _: OzHookCancellationScope) {}
}

fn session() -> OzHookSession {
    OzHookSession::new(
        Arc::new(NoopRuntime),
        warp_multi_agent_api::OzHookContext::default(),
        HookPayloadContext {
            session_id: "session".into(),
            run_id: "run".into(),
            conversation_id: "conversation".into(),
            cwd: "/tmp".into(),
            model: "model".into(),
            permission_mode: "supervised".into(),
        },
        HookRedactor::new([]),
        false,
    )
}

#[test]
fn oz_hooks_config_event_names_are_stable() {
    assert_eq!(
        HookEventName::ALL.map(HookEventName::as_str),
        [
            "SessionStart",
            "SessionEnd",
            "UserPromptSubmit",
            "Stop",
            "PreToolUse",
            "PostToolUse",
            "PreCompact",
        ]
    );
}

#[test]
fn oz_hook_session_claims_stop_once_per_turn_across_clones() {
    let session = session();
    let clone = session.clone();

    assert!(session.claim_stop());
    assert!(!clone.claim_stop());

    clone.begin_turn();

    assert!(session.claim_stop());
    assert!(!clone.claim_stop());
}

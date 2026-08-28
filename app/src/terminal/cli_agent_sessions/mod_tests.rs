use std::time::Duration;

use warpui::r#async::Timer;
use warpui::{App, EntityId};

use super::event::{
    CLIAgentEvent, CLIAgentEventPayload, CLIAgentEventSource, CLIAgentEventType, parse_event,
};
use super::{
    CLIAgentInputEntrypoint, CLIAgentInputState, CLIAgentSession, CLIAgentSessionContext,
    CLIAgentSessionStatus, CLIAgentSessionsModel,
};
use crate::ai::blocklist::{InputConfig, InputType};
use crate::terminal::CLIAgent;

#[test]
fn parse_stop_notification() {
    let body = r#"{"v":1,"agent":"claude","event":"stop","session_id":"abc","cwd":"/tmp/proj","project":"proj","query":"write a haiku","response":"Memory is safe","transcript_path":"/tmp/t.jsonl"}"#;
    let notif = parse_event(Some("warp://cli-agent"), body).unwrap();

    assert_eq!(notif.v, 1);
    assert_eq!(notif.agent, CLIAgent::Claude);
    assert_eq!(notif.event, CLIAgentEventType::Stop);
    assert_eq!(notif.session_id.as_deref(), Some("abc"));
    assert_eq!(notif.cwd.as_deref(), Some("/tmp/proj"));
    assert_eq!(notif.project.as_deref(), Some("proj"));
    assert_eq!(notif.payload.query.as_deref(), Some("write a haiku"));
    assert_eq!(notif.payload.response.as_deref(), Some("Memory is safe"));
    assert_eq!(
        notif.payload.transcript_path.as_deref(),
        Some("/tmp/t.jsonl")
    );
}

#[test]
fn cli_agent_session_context_title_like_text_uses_trimmed_summary() {
    let context = CLIAgentSessionContext {
        summary: Some("  Reviewing changes  ".to_string()),
        query: Some("Latest prompt".to_string()),
        ..Default::default()
    };

    assert_eq!(
        context.title_like_text(),
        Some("Reviewing changes".to_string())
    );
}

#[test]
fn cli_agent_session_context_latest_user_prompt_uses_trimmed_query() {
    let context = CLIAgentSessionContext {
        summary: Some("Reviewing changes".to_string()),
        query: Some("  Latest prompt  ".to_string()),
        ..Default::default()
    };

    assert_eq!(
        context.latest_user_prompt(),
        Some("Latest prompt".to_string())
    );
}

#[test]
fn cli_agent_session_context_title_helpers_ignore_empty_text() {
    let context = CLIAgentSessionContext {
        summary: Some("  ".to_string()),
        query: Some("".to_string()),
        ..Default::default()
    };

    assert_eq!(context.title_like_text(), None);
    assert_eq!(context.latest_user_prompt(), None);
}

#[test]
fn parse_permission_request_notification() {
    let body = r#"{"v":1,"agent":"claude","event":"permission_request","session_id":"abc","cwd":"/tmp/proj","project":"proj","summary":"Wants to run Bash: rm -rf /tmp","tool_name":"Bash","tool_input":{"command":"rm -rf /tmp"}}"#;
    let notif = parse_event(Some("warp://cli-agent"), body).unwrap();

    assert_eq!(notif.event, CLIAgentEventType::PermissionRequest);
    assert_eq!(
        notif.payload.summary.as_deref(),
        Some("Wants to run Bash: rm -rf /tmp")
    );
    assert_eq!(notif.payload.tool_name.as_deref(), Some("Bash"));
    assert_eq!(
        notif.payload.tool_input_preview.as_deref(),
        Some("rm -rf /tmp")
    );
}

#[test]
fn parse_permission_request_with_file_path() {
    let body = r#"{"v":1,"agent":"claude","event":"permission_request","session_id":"abc","cwd":"/tmp","project":"tmp","tool_name":"Write","tool_input":{"file_path":"/tmp/test.py","content":"print('hi')"}}"#;
    let notif = parse_event(Some("warp://cli-agent"), body).unwrap();

    assert_eq!(
        notif.payload.tool_input_preview.as_deref(),
        Some("/tmp/test.py")
    );
}

#[test]
fn parse_idle_prompt_notification() {
    let body = r#"{"v":1,"agent":"claude","event":"idle_prompt","session_id":"abc","cwd":"/tmp","project":"tmp","summary":"Claude is waiting for your input"}"#;
    let notif = parse_event(Some("warp://cli-agent"), body).unwrap();

    assert_eq!(notif.event, CLIAgentEventType::IdlePrompt);
    assert_eq!(
        notif.payload.summary.as_deref(),
        Some("Claude is waiting for your input")
    );
}

#[test]
fn parse_session_start_notification() {
    let body = r#"{"v":1,"agent":"claude","event":"session_start","session_id":"abc","cwd":"/tmp","project":"tmp","plugin_version":"1.1.0"}"#;
    let notif = parse_event(Some("warp://cli-agent"), body).unwrap();

    assert_eq!(notif.event, CLIAgentEventType::SessionStart);
    assert_eq!(notif.payload.plugin_version.as_deref(), Some("1.1.0"));
}

#[test]
fn returns_none_for_wrong_sentinel() {
    let body = r#"{"v":1,"agent":"claude","event":"stop"}"#;
    assert!(parse_event(Some("Claude Code"), body).is_none());
}

#[test]
fn returns_none_for_missing_title() {
    let body = r#"{"v":1,"agent":"claude","event":"stop"}"#;
    assert!(parse_event(None, body).is_none());
}

#[test]
fn returns_none_for_invalid_json() {
    assert!(parse_event(Some("warp://cli-agent"), "not json").is_none());
}

#[test]
fn handles_unknown_event_type() {
    let body = r#"{"v":1,"agent":"claude","event":"some_future_event"}"#;
    let notif = parse_event(Some("warp://cli-agent"), body).unwrap();
    assert_eq!(
        notif.event,
        CLIAgentEventType::Unknown("some_future_event".to_string())
    );
}

#[test]
fn handles_missing_optional_fields() {
    let body = r#"{"event":"stop"}"#;
    let notif = parse_event(Some("warp://cli-agent"), body).unwrap();

    assert_eq!(notif.v, 1);
    assert_eq!(notif.agent, CLIAgent::Unknown);
    assert_eq!(notif.event, CLIAgentEventType::Stop);
    assert!(notif.session_id.is_none());
    assert!(notif.cwd.is_none());
    assert!(notif.project.is_none());
    assert!(notif.payload.query.is_none());
}

#[test]
fn handles_special_characters_in_values() {
    let body = r#"{"v":1,"agent":"claude","event":"stop","query":"what does \"hello\" mean?","response":"It means greeting. Use: printf(\"hello\")"}"#;
    let notif = parse_event(Some("warp://cli-agent"), body).unwrap();

    assert_eq!(
        notif.payload.query.as_deref(),
        Some("what does \"hello\" mean?")
    );
    assert_eq!(
        notif.payload.response.as_deref(),
        Some("It means greeting. Use: printf(\"hello\")")
    );
}

#[test]
fn rejects_unsupported_schema_version() {
    let body = r#"{"v":2,"agent":"claude","event":"stop"}"#;
    assert!(parse_event(Some("warp://cli-agent"), body).is_none());
}

#[test]
fn defaults_to_v1_when_version_missing() {
    let body = r#"{"agent":"claude","event":"stop","query":"hi"}"#;
    let notif = parse_event(Some("warp://cli-agent"), body).unwrap();
    assert_eq!(notif.v, 1);
    assert_eq!(notif.payload.query.as_deref(), Some("hi"));
}

#[test]
fn explicit_v1_parses_correctly() {
    let body = r#"{"v":1,"agent":"claude","event":"stop","query":"test"}"#;
    let notif = parse_event(Some("warp://cli-agent"), body).unwrap();
    assert_eq!(notif.v, 1);
    assert_eq!(notif.payload.query.as_deref(), Some("test"));
}

#[test]
fn parse_prompt_submit_notification() {
    let body = r#"{"v":1,"agent":"claude","event":"prompt_submit","session_id":"abc","cwd":"/tmp/proj","project":"proj","query":"fix the bug"}"#;
    let notif = parse_event(Some("warp://cli-agent"), body).unwrap();

    assert_eq!(notif.event, CLIAgentEventType::PromptSubmit);
    assert_eq!(notif.payload.query.as_deref(), Some("fix the bug"));
}

#[test]
fn parse_tool_complete_notification() {
    let body = r#"{"v":1,"agent":"claude","event":"tool_complete","session_id":"abc","cwd":"/tmp/proj","project":"proj","tool_name":"Bash"}"#;
    let notif = parse_event(Some("warp://cli-agent"), body).unwrap();

    assert_eq!(notif.event, CLIAgentEventType::ToolComplete);
    assert_eq!(notif.payload.tool_name.as_deref(), Some("Bash"));
}

#[test]
fn parse_auggie_stop_notification() {
    // Mirrors what the community auggie-warp plugin emits on the Stop hook.
    let body = r#"{"v":1,"agent":"auggie","event":"stop","session_id":"abc","cwd":"/tmp/proj","project":"proj","query":"write a haiku","response":"Memory is safe"}"#;
    let notif = parse_event(Some("warp://cli-agent"), body).unwrap();

    assert_eq!(notif.agent, CLIAgent::Auggie);
    assert_eq!(notif.event, CLIAgentEventType::Stop);
    assert_eq!(notif.payload.query.as_deref(), Some("write a haiku"));
    assert_eq!(notif.payload.response.as_deref(), Some("Memory is safe"));
}

#[test]
fn parse_pi_stop_notification() {
    // Mirrors what the community pi-mono plugin emits on the Stop hook —
    // matches the Auggie shape and uses `"agent":"pi"`, which `resolve_agent`
    // already maps to `CLIAgent::Pi` via `command_prefix()`.
    let body = r#"{"v":1,"agent":"pi","event":"stop","session_id":"abc","cwd":"/tmp/proj","project":"proj","query":"write a haiku","response":"Memory is safe"}"#;
    let notif = parse_event(Some("warp://cli-agent"), body).unwrap();

    assert_eq!(notif.agent, CLIAgent::Pi);
    assert_eq!(notif.event, CLIAgentEventType::Stop);
    assert_eq!(notif.payload.query.as_deref(), Some("write a haiku"));
    assert_eq!(notif.payload.response.as_deref(), Some("Memory is safe"));
}

#[test]
fn parse_droid_stop_notification() {
    // Droid is already a known CLI agent, so structured OSC 777 events using
    // `"agent":"droid"` should resolve through the existing command prefix
    // parser without any Droid-specific parser logic.
    let body = r#"{"v":1,"agent":"droid","event":"stop","session_id":"abc","cwd":"/tmp/proj","project":"proj","query":"write a haiku","response":"Memory is safe"}"#;
    let notif = parse_event(Some("warp://cli-agent"), body).unwrap();

    assert_eq!(notif.agent, CLIAgent::Droid);
    assert_eq!(notif.event, CLIAgentEventType::Stop);
    assert_eq!(notif.payload.query.as_deref(), Some("write a haiku"));
    assert_eq!(notif.payload.response.as_deref(), Some("Memory is safe"));
}

#[test]
fn apply_event_preserves_input_session() {
    let input_state = CLIAgentInputState::Open {
        entrypoint: CLIAgentInputEntrypoint::CtrlG,
        previous_input_config: InputConfig {
            input_type: InputType::Shell,
            is_locked: false,
        },
        previous_was_lock_set_with_empty_buffer: true,
    };
    let mut session = CLIAgentSession {
        agent: CLIAgent::Claude,
        status: CLIAgentSessionStatus::InProgress,
        session_context: CLIAgentSessionContext::default(),
        input_state,
        should_auto_toggle_input: false,
        listener: None,
        remote_host: None,
        plugin_version: None,
        draft_text: None,
        custom_command_prefix: None,
        received_rich_notification: false,
    };

    let event = CLIAgentEvent {
        source: CLIAgentEventSource::RichPlugin,
        v: 1,
        agent: CLIAgent::Claude,
        event: CLIAgentEventType::PermissionRequest,
        session_id: Some("abc".to_string()),
        cwd: Some("/tmp/proj".to_string()),
        project: Some("proj".to_string()),
        payload: CLIAgentEventPayload {
            summary: Some("Needs approval".to_string()),
            ..Default::default()
        },
    };

    session.apply_event(&event);

    assert_eq!(session.input_state, input_state);
}

#[test]
fn is_remote_returns_true_when_remote_host_is_set() {
    let session = CLIAgentSession {
        agent: CLIAgent::Claude,
        status: CLIAgentSessionStatus::InProgress,
        session_context: CLIAgentSessionContext::default(),
        input_state: CLIAgentInputState::Closed,
        should_auto_toggle_input: false,
        listener: None,
        plugin_version: None,
        draft_text: None,
        remote_host: Some("user@devbox".to_owned()),
        custom_command_prefix: None,
        received_rich_notification: false,
    };
    assert!(session.is_remote());
}

#[test]
fn is_remote_returns_false_when_remote_host_is_none() {
    let session = CLIAgentSession {
        agent: CLIAgent::Claude,
        status: CLIAgentSessionStatus::InProgress,
        session_context: CLIAgentSessionContext::default(),
        input_state: CLIAgentInputState::Closed,
        should_auto_toggle_input: false,
        listener: None,
        remote_host: None,
        plugin_version: None,
        draft_text: None,
        custom_command_prefix: None,
        received_rich_notification: false,
    };
    assert!(!session.is_remote());
}

#[test]
fn local_failure_is_shared_across_local_sessions() {
    let mut model = CLIAgentSessionsModel::new();

    model.record_plugin_auto_failure(CLIAgent::Claude, None);

    assert!(model.has_plugin_auto_failed(CLIAgent::Claude, &None));
}

#[test]
fn local_failure_does_not_affect_remote_host() {
    let mut model = CLIAgentSessionsModel::new();

    model.record_plugin_auto_failure(CLIAgent::Claude, None);

    let remote = Some("user@devbox".to_owned());
    assert!(!model.has_plugin_auto_failed(CLIAgent::Claude, &remote));
}

#[test]
fn remote_failure_does_not_affect_local() {
    let mut model = CLIAgentSessionsModel::new();

    model.record_plugin_auto_failure(CLIAgent::Claude, Some("user@devbox".to_owned()));

    assert!(!model.has_plugin_auto_failed(CLIAgent::Claude, &None));
}

#[test]
fn remote_failures_are_independent_per_host() {
    let mut model = CLIAgentSessionsModel::new();

    let host_a = Some("user@host-a".to_owned());
    let host_b = Some("user@host-b".to_owned());

    model.record_plugin_auto_failure(CLIAgent::Claude, host_a.clone());

    assert!(model.has_plugin_auto_failed(CLIAgent::Claude, &host_a));
    assert!(!model.has_plugin_auto_failed(CLIAgent::Claude, &host_b));
}

#[test]
fn failure_tracking_is_independent_per_agent() {
    let mut model = CLIAgentSessionsModel::new();

    model.record_plugin_auto_failure(CLIAgent::Claude, None);

    assert!(model.has_plugin_auto_failed(CLIAgent::Claude, &None));
    assert!(!model.has_plugin_auto_failed(CLIAgent::Gemini, &None));
}

#[test]
fn session_start_sets_plugin_version() {
    let mut session = CLIAgentSession {
        agent: CLIAgent::Claude,
        status: CLIAgentSessionStatus::InProgress,
        session_context: CLIAgentSessionContext::default(),
        input_state: CLIAgentInputState::Closed,
        should_auto_toggle_input: false,
        listener: None,
        plugin_version: None,
        draft_text: None,
        remote_host: None,
        custom_command_prefix: None,
        received_rich_notification: false,
    };

    let event = CLIAgentEvent {
        source: CLIAgentEventSource::RichPlugin,
        v: 1,
        agent: CLIAgent::Claude,
        event: CLIAgentEventType::SessionStart,
        session_id: Some("abc".to_owned()),
        cwd: Some("/tmp".to_owned()),
        project: Some("proj".to_owned()),
        payload: CLIAgentEventPayload {
            plugin_version: Some("1.5.0".to_owned()),
            ..Default::default()
        },
    };

    session.apply_event(&event);
    assert_eq!(session.plugin_version.as_deref(), Some("1.5.0"));
}

#[test]
fn session_start_without_plugin_version_leaves_none() {
    let mut session = CLIAgentSession {
        agent: CLIAgent::Claude,
        status: CLIAgentSessionStatus::InProgress,
        session_context: CLIAgentSessionContext::default(),
        input_state: CLIAgentInputState::Closed,
        should_auto_toggle_input: false,
        listener: None,
        plugin_version: None,
        draft_text: None,
        remote_host: None,
        custom_command_prefix: None,
        received_rich_notification: false,
    };

    let event = CLIAgentEvent {
        source: CLIAgentEventSource::RichPlugin,
        v: 1,
        agent: CLIAgent::Claude,
        event: CLIAgentEventType::SessionStart,
        session_id: Some("abc".to_owned()),
        cwd: None,
        project: None,
        payload: CLIAgentEventPayload::default(),
    };

    session.apply_event(&event);
    assert_eq!(session.plugin_version, None);
}

#[test]
fn codex_session_not_rich_until_rich_notification() {
    // Codex's OSC 9 fallback never sets `received_rich_notification`, so the
    // session must not claim rich status even when a fallback listener exists.
    let mut session = CLIAgentSession {
        agent: CLIAgent::Codex,
        status: CLIAgentSessionStatus::InProgress,
        session_context: CLIAgentSessionContext::default(),
        input_state: CLIAgentInputState::Closed,
        should_auto_toggle_input: false,
        listener: None,
        plugin_version: None,
        remote_host: None,
        draft_text: None,
        custom_command_prefix: None,
        received_rich_notification: false,
    };
    assert!(!session.supports_rich_status());

    // A structured OSC 777 notification latches the flag -> rich status.
    session.received_rich_notification = true;
    assert!(session.supports_rich_status());
}

#[test]
fn non_codex_session_rich_after_rich_notification() {
    let mut session = CLIAgentSession {
        agent: CLIAgent::Claude,
        status: CLIAgentSessionStatus::InProgress,
        session_context: CLIAgentSessionContext::default(),
        input_state: CLIAgentInputState::Closed,
        should_auto_toggle_input: false,
        listener: None,
        plugin_version: None,
        remote_host: None,
        draft_text: None,
        custom_command_prefix: None,
        received_rich_notification: false,
    };
    // No listener and no rich notification yet.
    assert!(!session.supports_rich_status());

    session.received_rich_notification = true;
    assert!(session.supports_rich_status());
}

/// Constructs a session with permission-scoped state already populated, as if
/// a `PermissionRequest` had just been received and the agent is now Blocked.
/// Used by the GH-9525 regression tests below.
fn blocked_claude_session_with_permission_state() -> CLIAgentSession {
    CLIAgentSession {
        agent: CLIAgent::Claude,
        status: CLIAgentSessionStatus::Blocked {
            message: Some("Wants to run bash: rm -rf /tmp".to_owned()),
        },
        session_context: CLIAgentSessionContext {
            summary: Some("Wants to run bash: rm -rf /tmp".to_owned()),
            tool_name: Some("Bash".to_owned()),
            tool_input_preview: Some("rm -rf /tmp".to_owned()),
            ..Default::default()
        },
        input_state: CLIAgentInputState::Closed,
        should_auto_toggle_input: false,
        listener: None,
        plugin_version: None,
        draft_text: None,
        remote_host: None,
        custom_command_prefix: None,
        received_rich_notification: false,
    }
}

#[test]
fn stop_clears_permission_scoped_state() {
    // GH-9525: after a PermissionRequest sets `summary`, the Stop event must
    // clear it. Otherwise the tab title falls back to the stale permission
    // text instead of reflecting the now-completed session.
    let mut session = blocked_claude_session_with_permission_state();

    let event = CLIAgentEvent {
        source: CLIAgentEventSource::RichPlugin,
        v: 1,
        agent: CLIAgent::Claude,
        event: CLIAgentEventType::Stop,
        session_id: Some("abc".to_owned()),
        cwd: None,
        project: None,
        payload: CLIAgentEventPayload {
            query: Some("write a haiku".to_owned()),
            response: Some("Memory is safe".to_owned()),
            ..Default::default()
        },
    };

    session.apply_event(&event);

    assert_eq!(session.session_context.summary, None);
    assert_eq!(session.session_context.tool_name, None);
    assert_eq!(session.session_context.tool_input_preview, None);
    assert_eq!(
        session.session_context.query.as_deref(),
        Some("write a haiku"),
    );
    assert_eq!(
        session.session_context.response.as_deref(),
        Some("Memory is safe"),
    );
    assert!(matches!(session.status, CLIAgentSessionStatus::Success));
}

#[test]
fn permission_replied_clears_permission_scoped_state() {
    // When the user replies to a permission prompt the agent transitions back
    // to InProgress; the now-stale summary/tool fields must be cleared so they
    // don't leak into UI surfaces during the next turn.
    let mut session = blocked_claude_session_with_permission_state();

    let event = CLIAgentEvent {
        source: CLIAgentEventSource::RichPlugin,
        v: 1,
        agent: CLIAgent::Claude,
        event: CLIAgentEventType::PermissionReplied,
        session_id: Some("abc".to_owned()),
        cwd: None,
        project: None,
        payload: CLIAgentEventPayload::default(),
    };

    session.apply_event(&event);

    assert_eq!(session.session_context.summary, None);
    assert_eq!(session.session_context.tool_name, None);
    assert_eq!(session.session_context.tool_input_preview, None);
    assert!(matches!(session.status, CLIAgentSessionStatus::InProgress));
}

#[test]
fn prompt_submit_clears_permission_scoped_state() {
    // PromptSubmit already clears `response`; clearing the permission-scoped
    // fields keeps the same hygiene if the user manages to start a new turn
    // while permission state is still populated (e.g. an abandoned permission
    // flow that was not closed by an explicit PermissionReplied).
    let mut session = blocked_claude_session_with_permission_state();
    session.session_context.response = Some("stale response".to_owned());

    let event = CLIAgentEvent {
        source: CLIAgentEventSource::RichPlugin,
        v: 1,
        agent: CLIAgent::Claude,
        event: CLIAgentEventType::PromptSubmit,
        session_id: Some("abc".to_owned()),
        cwd: None,
        project: None,
        payload: CLIAgentEventPayload {
            query: Some("next prompt".to_owned()),
            ..Default::default()
        },
    };

    session.apply_event(&event);

    assert_eq!(session.session_context.summary, None);
    assert_eq!(session.session_context.tool_name, None);
    assert_eq!(session.session_context.tool_input_preview, None);
    assert_eq!(session.session_context.response, None);
    assert_eq!(
        session.session_context.query.as_deref(),
        Some("next prompt")
    );
    assert!(matches!(session.status, CLIAgentSessionStatus::InProgress));
}

#[test]
fn tool_complete_clears_permission_scoped_state() {
    // GH-11082: answering an AskUserQuestion emits only ToolComplete (the
    // plugin sends no PermissionReplied for it), so the Blocked -> InProgress
    // transition here must also clear the stale summary. Otherwise the tab
    // title keeps showing "Wants to run AskUserQuestion: ..." until the next
    // prompt or Stop.
    let mut session = blocked_claude_session_with_permission_state();

    let event = CLIAgentEvent {
        source: CLIAgentEventSource::RichPlugin,
        v: 1,
        agent: CLIAgent::Claude,
        event: CLIAgentEventType::ToolComplete,
        session_id: Some("abc".to_owned()),
        cwd: None,
        project: None,
        payload: CLIAgentEventPayload::default(),
    };

    session.apply_event(&event);

    assert_eq!(session.session_context.summary, None);
    assert_eq!(session.session_context.tool_name, None);
    assert_eq!(session.session_context.tool_input_preview, None);
    assert!(matches!(session.status, CLIAgentSessionStatus::InProgress));
}

#[test]
fn permission_request_still_populates_summary_and_tool_fields() {
    // Sanity: clearing permission-scoped state on Stop/Reply/Submit must not
    // also break the PermissionRequest path that initially populates them.
    let mut session = CLIAgentSession {
        agent: CLIAgent::Claude,
        status: CLIAgentSessionStatus::InProgress,
        session_context: CLIAgentSessionContext::default(),
        input_state: CLIAgentInputState::Closed,
        should_auto_toggle_input: false,
        listener: None,
        plugin_version: None,
        draft_text: None,
        remote_host: None,
        custom_command_prefix: None,
        received_rich_notification: false,
    };

    let event = CLIAgentEvent {
        source: CLIAgentEventSource::RichPlugin,
        v: 1,
        agent: CLIAgent::Claude,
        event: CLIAgentEventType::PermissionRequest,
        session_id: Some("abc".to_owned()),
        cwd: None,
        project: None,
        payload: CLIAgentEventPayload {
            summary: Some("Wants to run bash: rm -rf /tmp".to_owned()),
            tool_name: Some("Bash".to_owned()),
            tool_input_preview: Some("rm -rf /tmp".to_owned()),
            ..Default::default()
        },
    };

    session.apply_event(&event);

    assert_eq!(
        session.session_context.summary.as_deref(),
        Some("Wants to run bash: rm -rf /tmp"),
    );
    assert_eq!(session.session_context.tool_name.as_deref(), Some("Bash"));
    assert_eq!(
        session.session_context.tool_input_preview.as_deref(),
        Some("rm -rf /tmp"),
    );
    assert!(matches!(
        session.status,
        CLIAgentSessionStatus::Blocked { .. },
    ));
}

// --- Ctrl-C pending-cancel state machine ---

/// Grace window used by the tests below. Long enough to comfortably
/// distinguish "fired at the original deadline" from "reset" under CI
/// scheduling jitter, short enough to keep the suite fast.
const TEST_WINDOW: Duration = Duration::from_millis(200);
/// Extra margin added when waiting for `TEST_WINDOW` to lapse.
const TEST_WINDOW_BUFFER: Duration = Duration::from_millis(150);

fn cli_agent_session(
    status: CLIAgentSessionStatus,
    received_rich_notification: bool,
) -> CLIAgentSession {
    CLIAgentSession {
        agent: CLIAgent::Claude,
        status,
        session_context: CLIAgentSessionContext::default(),
        input_state: CLIAgentInputState::Closed,
        should_auto_toggle_input: false,
        listener: None,
        plugin_version: None,
        remote_host: None,
        draft_text: None,
        custom_command_prefix: None,
        received_rich_notification,
    }
}

fn plugin_event(source: CLIAgentEventSource, event: CLIAgentEventType) -> CLIAgentEvent {
    CLIAgentEvent {
        source,
        v: 1,
        agent: CLIAgent::Claude,
        event,
        session_id: None,
        cwd: None,
        project: None,
        payload: CLIAgentEventPayload::default(),
    }
}

fn rich_event(event: CLIAgentEventType) -> CLIAgentEvent {
    plugin_event(CLIAgentEventSource::RichPlugin, event)
}

#[test]
fn ctrl_c_does_not_arm_on_optimistic_in_progress_before_prompt_submit() {
    App::test((), |mut app| async move {
        let model = app.add_singleton_model(|_| CLIAgentSessionsModel::new());
        let view_id = EntityId::new();
        model.update(&mut app, |m, ctx| {
            m.set_session(
                view_id,
                cli_agent_session(CLIAgentSessionStatus::InProgress, true),
                ctx,
            );
        });

        model.update(&mut app, |m, ctx| {
            m.observe_ctrl_c_write_with_window(view_id, TEST_WINDOW, ctx);
        });

        assert!(
            !model.read(&app, |m, _| m
                .has_pending_or_resolved_ctrl_c_cancel(view_id)),
            "must not arm on the optimistic InProgress set at registration, before any turn started"
        );
    });
}

#[test]
fn ctrl_c_arms_when_in_progress_rich_and_prompt_submitted() {
    App::test((), |mut app| async move {
        let model = app.add_singleton_model(|_| CLIAgentSessionsModel::new());
        let view_id = EntityId::new();
        model.update(&mut app, |m, ctx| {
            m.set_session(
                view_id,
                cli_agent_session(CLIAgentSessionStatus::InProgress, true),
                ctx,
            );
        });
        model.update(&mut app, |m, ctx| {
            m.update_from_event(view_id, &rich_event(CLIAgentEventType::PromptSubmit), ctx);
        });

        model.update(&mut app, |m, ctx| {
            m.observe_ctrl_c_write_with_window(view_id, TEST_WINDOW, ctx);
        });

        assert!(model.read(&app, |m, _| {
            m.has_pending_or_resolved_ctrl_c_cancel(view_id)
        }));
    });
}

#[test]
fn ctrl_c_arms_when_blocked_rich_and_prompt_submitted() {
    App::test((), |mut app| async move {
        let model = app.add_singleton_model(|_| CLIAgentSessionsModel::new());
        let view_id = EntityId::new();
        model.update(&mut app, |m, ctx| {
            m.set_session(
                view_id,
                cli_agent_session(CLIAgentSessionStatus::InProgress, true),
                ctx,
            );
        });
        model.update(&mut app, |m, ctx| {
            m.update_from_event(view_id, &rich_event(CLIAgentEventType::PromptSubmit), ctx);
        });
        model.update(&mut app, |m, ctx| {
            m.update_from_event(
                view_id,
                &rich_event(CLIAgentEventType::PermissionRequest),
                ctx,
            );
        });

        model.update(&mut app, |m, ctx| {
            m.observe_ctrl_c_write_with_window(view_id, TEST_WINDOW, ctx);
        });

        assert!(model.read(&app, |m, _| {
            m.has_pending_or_resolved_ctrl_c_cancel(view_id)
        }));
    });
}

#[test]
fn ctrl_c_does_not_arm_when_status_is_success() {
    App::test((), |mut app| async move {
        let model = app.add_singleton_model(|_| CLIAgentSessionsModel::new());
        let view_id = EntityId::new();
        model.update(&mut app, |m, ctx| {
            m.set_session(
                view_id,
                cli_agent_session(CLIAgentSessionStatus::InProgress, true),
                ctx,
            );
        });
        model.update(&mut app, |m, ctx| {
            m.update_from_event(view_id, &rich_event(CLIAgentEventType::PromptSubmit), ctx);
        });
        model.update(&mut app, |m, ctx| {
            m.update_from_event(view_id, &rich_event(CLIAgentEventType::Stop), ctx);
        });

        model.update(&mut app, |m, ctx| {
            m.observe_ctrl_c_write_with_window(view_id, TEST_WINDOW, ctx);
        });

        assert!(!model.read(&app, |m, _| {
            m.has_pending_or_resolved_ctrl_c_cancel(view_id)
        }));
    });
}

#[test]
fn ctrl_c_does_not_arm_for_codex_osc9_fallback_session() {
    App::test((), |mut app| async move {
        let model = app.add_singleton_model(|_| CLIAgentSessionsModel::new());
        let view_id = EntityId::new();
        model.update(&mut app, |m, ctx| {
            m.set_session(
                view_id,
                cli_agent_session(CLIAgentSessionStatus::InProgress, false),
                ctx,
            );
        });
        // Codex's OSC 9 fallback never latches `received_rich_notification`,
        // even though `has_seen_prompt_submit` is still tracked.
        model.update(&mut app, |m, ctx| {
            m.update_from_event(
                view_id,
                &plugin_event(
                    CLIAgentEventSource::CodexOsc9Fallback,
                    CLIAgentEventType::PromptSubmit,
                ),
                ctx,
            );
        });

        model.update(&mut app, |m, ctx| {
            m.observe_ctrl_c_write_with_window(view_id, TEST_WINDOW, ctx);
        });

        assert!(
            !model.read(&app, |m, _| m
                .has_pending_or_resolved_ctrl_c_cancel(view_id)),
            "a Codex OSC 9 fallback session must never arm the Ctrl-C cancel window"
        );
    });
}

#[test]
fn window_lapse_transitions_session_to_cancelled() {
    App::test((), |mut app| async move {
        let model = app.add_singleton_model(|_| CLIAgentSessionsModel::new());
        let view_id = EntityId::new();
        model.update(&mut app, |m, ctx| {
            m.set_session(
                view_id,
                cli_agent_session(CLIAgentSessionStatus::InProgress, true),
                ctx,
            );
        });
        model.update(&mut app, |m, ctx| {
            m.update_from_event(view_id, &rich_event(CLIAgentEventType::PromptSubmit), ctx);
        });
        model.update(&mut app, |m, ctx| {
            m.observe_ctrl_c_write_with_window(view_id, TEST_WINDOW, ctx);
        });

        Timer::after(TEST_WINDOW + TEST_WINDOW_BUFFER).await;

        model.read(&app, |m, _| {
            assert_eq!(
                m.session(view_id).map(|s| &s.status),
                Some(&CLIAgentSessionStatus::Cancelled)
            );
            assert!(m.has_pending_or_resolved_ctrl_c_cancel(view_id));
        });
    });
}

/// Arms a window for an InProgress+rich session that has seen `prompt_submit`,
/// then applies `disarming_event` immediately and asserts the window no
/// longer resolves to `Cancelled` once it would have lapsed.
fn assert_event_disarms_pending_cancel(disarming_event: CLIAgentEvent) {
    App::test((), |mut app| async move {
        let model = app.add_singleton_model(|_| CLIAgentSessionsModel::new());
        let view_id = EntityId::new();
        model.update(&mut app, |m, ctx| {
            m.set_session(
                view_id,
                cli_agent_session(CLIAgentSessionStatus::InProgress, true),
                ctx,
            );
        });
        model.update(&mut app, |m, ctx| {
            m.update_from_event(view_id, &rich_event(CLIAgentEventType::PromptSubmit), ctx);
        });
        model.update(&mut app, |m, ctx| {
            m.observe_ctrl_c_write_with_window(view_id, TEST_WINDOW, ctx);
        });
        assert!(model.read(&app, |m, _| {
            m.has_pending_or_resolved_ctrl_c_cancel(view_id)
        }));

        model.update(&mut app, |m, ctx| {
            m.update_from_event(view_id, &disarming_event, ctx);
        });
        assert!(!model.read(&app, |m, _| {
            m.has_pending_or_resolved_ctrl_c_cancel(view_id)
        }));

        Timer::after(TEST_WINDOW + TEST_WINDOW_BUFFER).await;

        model.read(&app, |m, _| {
            assert_ne!(
                m.session(view_id).map(|s| &s.status),
                Some(&CLIAgentSessionStatus::Cancelled),
                "a disarming event must prevent the lapsed window from cancelling the session"
            );
        });
    });
}

#[test]
fn stop_event_disarms_pending_cancel() {
    assert_event_disarms_pending_cancel(rich_event(CLIAgentEventType::Stop));
}

#[test]
fn stop_failure_event_disarms_pending_cancel() {
    assert_event_disarms_pending_cancel(rich_event(CLIAgentEventType::StopFailure));
}

#[test]
fn permission_request_event_disarms_pending_cancel() {
    assert_event_disarms_pending_cancel(rich_event(CLIAgentEventType::PermissionRequest));
}

#[test]
fn question_asked_event_disarms_pending_cancel() {
    assert_event_disarms_pending_cancel(rich_event(CLIAgentEventType::QuestionAsked));
}

#[test]
fn prompt_submit_event_disarms_pending_cancel() {
    assert_event_disarms_pending_cancel(rich_event(CLIAgentEventType::PromptSubmit));
}

#[test]
fn tool_complete_event_disarms_pending_cancel() {
    // ToolComplete only drives a status transition when the session is
    // Blocked, but any plugin traffic must still disarm the window.
    assert_event_disarms_pending_cancel(rich_event(CLIAgentEventType::ToolComplete));
}

#[test]
fn idle_prompt_does_not_disarm_pending_cancel() {
    // Unlike other plugin events, `IdlePrompt` means the CLI is sitting
    // idle at its interactive prompt -- evidence of idleness, not
    // aliveness. If it disarmed the window, an idle notification arriving
    // instead of a genuine `stop`/`stop_failure` after an interrupt would
    // leave the session stuck exactly like the bug this feature exists to
    // fix, so the window must survive it and still resolve to `Cancelled`
    // once it lapses.
    App::test((), |mut app| async move {
        let model = app.add_singleton_model(|_| CLIAgentSessionsModel::new());
        let view_id = EntityId::new();
        model.update(&mut app, |m, ctx| {
            m.set_session(
                view_id,
                cli_agent_session(CLIAgentSessionStatus::InProgress, true),
                ctx,
            );
        });
        model.update(&mut app, |m, ctx| {
            m.update_from_event(view_id, &rich_event(CLIAgentEventType::PromptSubmit), ctx);
        });
        model.update(&mut app, |m, ctx| {
            m.observe_ctrl_c_write_with_window(view_id, TEST_WINDOW, ctx);
        });
        assert!(model.read(&app, |m, _| {
            m.has_pending_or_resolved_ctrl_c_cancel(view_id)
        }));

        model.update(&mut app, |m, ctx| {
            m.update_from_event(view_id, &rich_event(CLIAgentEventType::IdlePrompt), ctx);
        });
        assert!(
            model.read(&app, |m, _| {
                m.has_pending_or_resolved_ctrl_c_cancel(view_id)
            }),
            "an IdlePrompt must not disarm the pending-cancel window"
        );

        Timer::after(TEST_WINDOW + TEST_WINDOW_BUFFER).await;

        model.read(&app, |m, _| {
            assert_eq!(
                m.session(view_id).map(|s| &s.status),
                Some(&CLIAgentSessionStatus::Cancelled),
                "an IdlePrompt must not prevent the lapsed window from cancelling the session"
            );
        });
    });
}

#[test]
fn late_stop_after_cancelled_flips_status_to_success() {
    App::test((), |mut app| async move {
        let model = app.add_singleton_model(|_| CLIAgentSessionsModel::new());
        let view_id = EntityId::new();
        model.update(&mut app, |m, ctx| {
            m.set_session(
                view_id,
                cli_agent_session(CLIAgentSessionStatus::InProgress, true),
                ctx,
            );
        });
        model.update(&mut app, |m, ctx| {
            m.update_from_event(view_id, &rich_event(CLIAgentEventType::PromptSubmit), ctx);
        });
        model.update(&mut app, |m, ctx| {
            m.observe_ctrl_c_write_with_window(view_id, TEST_WINDOW, ctx);
        });
        Timer::after(TEST_WINDOW + TEST_WINDOW_BUFFER).await;
        model.read(&app, |m, _| {
            assert_eq!(
                m.session(view_id).map(|s| &s.status),
                Some(&CLIAgentSessionStatus::Cancelled)
            );
        });

        // A `stop` hook that arrives after the window already marked the
        // session Cancelled must still flow through and flip to Success.
        model.update(&mut app, |m, ctx| {
            m.update_from_event(view_id, &rich_event(CLIAgentEventType::Stop), ctx);
        });

        model.read(&app, |m, _| {
            assert_eq!(
                m.session(view_id).map(|s| &s.status),
                Some(&CLIAgentSessionStatus::Success)
            );
        });
    });
}

#[test]
fn prompt_submit_after_cancelled_returns_session_to_in_progress() {
    App::test((), |mut app| async move {
        let model = app.add_singleton_model(|_| CLIAgentSessionsModel::new());
        let view_id = EntityId::new();
        model.update(&mut app, |m, ctx| {
            m.set_session(
                view_id,
                cli_agent_session(CLIAgentSessionStatus::InProgress, true),
                ctx,
            );
        });
        model.update(&mut app, |m, ctx| {
            m.update_from_event(view_id, &rich_event(CLIAgentEventType::PromptSubmit), ctx);
        });
        model.update(&mut app, |m, ctx| {
            m.observe_ctrl_c_write_with_window(view_id, TEST_WINDOW, ctx);
        });
        Timer::after(TEST_WINDOW + TEST_WINDOW_BUFFER).await;
        model.read(&app, |m, _| {
            assert_eq!(
                m.session(view_id).map(|s| &s.status),
                Some(&CLIAgentSessionStatus::Cancelled)
            );
        });

        model.update(&mut app, |m, ctx| {
            m.update_from_event(view_id, &rich_event(CLIAgentEventType::PromptSubmit), ctx);
        });

        model.read(&app, |m, _| {
            assert_eq!(
                m.session(view_id).map(|s| &s.status),
                Some(&CLIAgentSessionStatus::InProgress)
            );
        });
    });
}

#[test]
fn stale_timer_callback_after_disarming_event_does_not_overwrite_newer_status() {
    // WarpUI's `SpawnedFutureHandle::abort` only takes effect the next time
    // the future is polled: if the timer already completed and queued its
    // resolve callback before a disarming event arrives, that callback can
    // still run after `abort()` was called on it. The token guard in
    // `resolve_pending_cancel` must make the stale callback a no-op instead
    // of overwriting the disarming event's status.
    App::test((), |mut app| async move {
        let model = app.add_singleton_model(|_| CLIAgentSessionsModel::new());
        let view_id = EntityId::new();
        model.update(&mut app, |m, ctx| {
            m.set_session(
                view_id,
                cli_agent_session(CLIAgentSessionStatus::InProgress, true),
                ctx,
            );
        });
        model.update(&mut app, |m, ctx| {
            m.update_from_event(view_id, &rich_event(CLIAgentEventType::PromptSubmit), ctx);
        });
        model.update(&mut app, |m, ctx| {
            m.observe_ctrl_c_write_with_window(view_id, TEST_WINDOW, ctx);
        });
        let armed_token = model
            .read(&app, |m, _| {
                m.ctrl_c_cancel_state
                    .get(&view_id)
                    .and_then(|s| s.armed_token)
            })
            .expect("window should be armed");

        // A disarming event arrives before the queued callback runs.
        model.update(&mut app, |m, ctx| {
            m.update_from_event(view_id, &rich_event(CLIAgentEventType::Stop), ctx);
        });
        model.read(&app, |m, _| {
            assert_eq!(
                m.session(view_id).map(|s| &s.status),
                Some(&CLIAgentSessionStatus::Success)
            );
        });

        // The stale callback finally runs with the pre-disarm token. It must
        // be a no-op: the disarming event's Success status must survive.
        model.update(&mut app, |m, ctx| {
            m.resolve_pending_cancel(view_id, armed_token, ctx);
        });

        model.read(&app, |m, _| {
            assert_eq!(
                m.session(view_id).map(|s| &s.status),
                Some(&CLIAgentSessionStatus::Success),
                "a stale timer callback must not overwrite a newer disarming event's status"
            );
        });
    });
}

#[test]
fn remove_session_clears_pending_cancel_state() {
    // The model is a process-lifetime singleton, so a closed session that
    // saw a `prompt_submit` (and possibly armed a window) must not leave an
    // orphaned `ctrl_c_cancel_state` entry behind.
    App::test((), |mut app| async move {
        let model = app.add_singleton_model(|_| CLIAgentSessionsModel::new());
        let view_id = EntityId::new();
        model.update(&mut app, |m, ctx| {
            m.set_session(
                view_id,
                cli_agent_session(CLIAgentSessionStatus::InProgress, true),
                ctx,
            );
        });
        model.update(&mut app, |m, ctx| {
            m.update_from_event(view_id, &rich_event(CLIAgentEventType::PromptSubmit), ctx);
        });
        model.update(&mut app, |m, ctx| {
            m.observe_ctrl_c_write_with_window(view_id, TEST_WINDOW, ctx);
        });
        assert!(model.read(&app, |m, _| m.ctrl_c_cancel_state.contains_key(&view_id)));

        model.update(&mut app, |m, ctx| {
            m.remove_session(view_id, ctx);
        });

        assert!(
            model.read(&app, |m, _| !m.ctrl_c_cancel_state.contains_key(&view_id)),
            "remove_session must clear the ctrl_c_cancel_state entry for the closed session"
        );
    });
}

#[test]
fn second_ctrl_c_while_armed_reuses_the_existing_window() {
    // Deliberately larger/looser than `TEST_WINDOW` so the checkpoint below
    // has a wide, unambiguous margin on both sides: comfortably after the
    // original deadline, and comfortably before what a reset (rather than
    // reused) window would require.
    let window = Duration::from_millis(300);
    let before_deadline = Duration::from_millis(250);
    let checkpoint_after_second_ctrl_c = Duration::from_millis(100);

    App::test((), |mut app| async move {
        let model = app.add_singleton_model(|_| CLIAgentSessionsModel::new());
        let view_id = EntityId::new();
        model.update(&mut app, |m, ctx| {
            m.set_session(
                view_id,
                cli_agent_session(CLIAgentSessionStatus::InProgress, true),
                ctx,
            );
        });
        model.update(&mut app, |m, ctx| {
            m.update_from_event(view_id, &rich_event(CLIAgentEventType::PromptSubmit), ctx);
        });
        model.update(&mut app, |m, ctx| {
            m.observe_ctrl_c_write_with_window(view_id, window, ctx);
        });

        // Second Ctrl-C at t=250ms, shortly before the original t=300ms
        // deadline. Check at t=350ms: 50ms past the original deadline, but
        // 200ms short of a reset deadline (250+300=550ms) — reuse and reset
        // are unambiguous at this checkpoint.
        Timer::after(before_deadline).await;
        model.update(&mut app, |m, ctx| {
            m.observe_ctrl_c_write_with_window(view_id, window, ctx);
        });
        Timer::after(checkpoint_after_second_ctrl_c).await;

        model.read(&app, |m, _| {
            assert_eq!(
                m.session(view_id).map(|s| &s.status),
                Some(&CLIAgentSessionStatus::Cancelled),
                "a second Ctrl-C while armed must reuse the existing window, not reset it"
            );
        });
    });
}

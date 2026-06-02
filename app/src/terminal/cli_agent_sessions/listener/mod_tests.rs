use super::*;
use crate::terminal::cli_agent_sessions::event::CLIAgentEventType;

#[test]
fn codex_parses_any_text_as_stop() {
    let event = CodexSessionHandler::parse_osc9_text("Agent turn complete").unwrap();
    assert_eq!(event.event, CLIAgentEventType::Stop);
    assert_eq!(event.agent, CLIAgent::Codex);
    assert_eq!(event.payload.query.as_deref(), Some("Agent turn complete"));
}

#[test]
fn codex_body_becomes_query() {
    let event =
        CodexSessionHandler::parse_osc9_text("I've updated the README with the new instructions.")
            .unwrap();
    assert_eq!(event.event, CLIAgentEventType::Stop);
    assert_eq!(
        event.payload.query.as_deref(),
        Some("I've updated the README with the new instructions.")
    );
}

#[test]
fn codex_approval_text_still_becomes_stop() {
    let event =
        CodexSessionHandler::parse_osc9_text("Approval requested: rm -rf /tmp/foo").unwrap();
    assert_eq!(event.event, CLIAgentEventType::Stop);
    assert_eq!(
        event.payload.query.as_deref(),
        Some("Approval requested: rm -rf /tmp/foo")
    );
}

#[test]
fn codex_ignores_empty_body() {
    assert!(CodexSessionHandler::parse_osc9_text("").is_none());
    assert!(CodexSessionHandler::parse_osc9_text("   ").is_none());
}

#[test]
fn codex_try_parse_ignores_titled_notifications() {
    let handler = CodexSessionHandler;
    assert!(handler
        .try_parse(Some("some-title"), "Agent turn complete")
        .is_none());
}

#[test]
fn codex_try_parse_handles_osc9() {
    let handler = CodexSessionHandler;
    let event = handler.try_parse(None, "Agent turn complete").unwrap();
    assert_eq!(event.event, CLIAgentEventType::Stop);
}

#[test]
fn auggie_is_supported() {
    assert!(is_agent_supported(&CLIAgent::Auggie));
}

#[test]
fn auggie_uses_default_handler_with_rich_status() {
    assert!(agent_supports_rich_status(&CLIAgent::Auggie));
}

#[test]
fn auggie_default_handler_skips_session_start() {
    let mut handler = DefaultSessionListener;
    let event = CLIAgentEvent {
        v: 1,
        agent: CLIAgent::Auggie,
        event: CLIAgentEventType::SessionStart,
        session_id: None,
        cwd: None,
        project: None,
        payload: CLIAgentEventPayload::default(),
    };
    assert!(handler.handle_event(event).is_none());
}

#[test]
fn auggie_default_handler_forwards_stop() {
    let mut handler = DefaultSessionListener;
    let event = CLIAgentEvent {
        v: 1,
        agent: CLIAgent::Auggie,
        event: CLIAgentEventType::Stop,
        session_id: None,
        cwd: None,
        project: None,
        payload: CLIAgentEventPayload::default(),
    };
    assert!(handler.handle_event(event).is_some());
}

#[test]
fn pi_is_supported() {
    assert!(is_agent_supported(&CLIAgent::Pi));
}

#[test]
fn pi_uses_default_handler_with_rich_status() {
    assert!(agent_supports_rich_status(&CLIAgent::Pi));
}

#[test]
fn pi_default_handler_skips_session_start() {
    let mut handler = DefaultSessionListener;
    let event = CLIAgentEvent {
        v: 1,
        agent: CLIAgent::Pi,
        event: CLIAgentEventType::SessionStart,
        session_id: None,
        cwd: None,
        project: None,
        payload: CLIAgentEventPayload::default(),
    };
    assert!(handler.handle_event(event).is_none());
}

#[test]
fn pi_default_handler_forwards_stop() {
    let mut handler = DefaultSessionListener;
    let event = CLIAgentEvent {
        v: 1,
        agent: CLIAgent::Pi,
        event: CLIAgentEventType::Stop,
        session_id: None,
        cwd: None,
        project: None,
        payload: CLIAgentEventPayload::default(),
    };
    assert!(handler.handle_event(event).is_some());
}

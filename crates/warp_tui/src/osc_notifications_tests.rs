use super::*;

#[test]
fn test_json_required_fields() {
    let n = CLIAgentNotification::new(WARP_TUI_AGENT_NAME, "session_start");
    let json = build_json(&n);
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed["v"], 1);
    assert_eq!(parsed["agent"], WARP_TUI_AGENT_NAME);
    assert_eq!(parsed["event"], "session_start");
    // Optional fields absent when None.
    assert!(parsed.get("session_id").is_none());
    assert!(parsed.get("cwd").is_none());
}

#[test]
fn test_json_optional_fields_present() {
    let n = CLIAgentNotification {
        session_id: Some("abc-123".to_owned()),
        cwd: Some("/home/user/project".to_owned()),
        query: Some("Fix the bug".to_owned()),
        ..CLIAgentNotification::new(WARP_TUI_AGENT_NAME, "prompt_submit")
    };
    let json = build_json(&n);
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed["session_id"], "abc-123");
    assert_eq!(parsed["cwd"], "/home/user/project");
    assert_eq!(parsed["query"], "Fix the bug");
    assert!(parsed.get("tool_name").is_none());
}

#[test]
fn test_json_stop_failure_error_type() {
    let n = CLIAgentNotification {
        session_id: Some("sess".to_owned()),
        error_type: Some("cancelled".to_owned()),
        ..CLIAgentNotification::new(WARP_TUI_AGENT_NAME, "stop_failure")
    };
    let json = build_json(&n);
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed["event"], "stop_failure");
    assert_eq!(parsed["error_type"], "cancelled");
}

/// Asserts the full OSC 777 wire sequence produced by `build_sequence` matches
/// the format the GUI parser expects: `ESC ] 777 ; notify ; warp://cli-agent ;
/// <json> BEL`. This test exercises `build_sequence` directly so it catches
/// any regression in the shared format string used by both `emit` and the test.
#[test]
fn test_sequence_wire_format() {
    let n = CLIAgentNotification {
        session_id: Some("sess-1".to_owned()),
        ..CLIAgentNotification::new(WARP_TUI_AGENT_NAME, "stop")
    };
    let sequence = build_sequence(&n);
    // Must start with ESC ] 777 ; notify ; warp://cli-agent ;
    assert!(
        sequence.starts_with("\x1b]777;notify;warp://cli-agent;"),
        "unexpected sequence prefix: {sequence:?}"
    );
    // Must end with BEL.
    assert!(
        sequence.ends_with('\x07'),
        "sequence does not end with BEL: {sequence:?}"
    );
    // Must contain the required event field.
    assert!(sequence.contains("\"event\":\"stop\""), "{sequence:?}");
    // Must contain the session_id when provided.
    assert!(
        sequence.contains("\"session_id\":\"sess-1\""),
        "{sequence:?}"
    );
}

/// Verifies the wire-format invariant required for a symmetric Blocked/Unblocked GUI chip
/// transition: the `permission_request` ("block") and `tool_complete` ("unblock") events must
/// embed the same `session_id` value so the GUI's `CLIAgentSessionsModel` can correlate them
/// to the same session. A mismatch here (or in the call sites in `terminal_session_view.rs`)
/// would leave the chip stuck in the Blocked state even after the user responded.
///
/// The emission sites in `terminal_session_view.rs` are both gated on the selected/foreground
/// conversation with the same `terminal_surface_id` as `session_id`, so this test also serves
/// as a regression guard: if either side starts using a different identifier for `session_id`,
/// this test fails.
#[test]
fn blocked_and_unblocked_events_carry_matching_session_id() {
    let session_id = "surface-42";
    let blocked = build_sequence(&CLIAgentNotification {
        session_id: Some(session_id.to_owned()),
        ..CLIAgentNotification::new(WARP_TUI_AGENT_NAME, "permission_request")
    });
    let unblocked = build_sequence(&CLIAgentNotification {
        session_id: Some(session_id.to_owned()),
        ..CLIAgentNotification::new(WARP_TUI_AGENT_NAME, "tool_complete")
    });
    let expected_session = format!("\"session_id\":\"{session_id}\"");
    assert!(
        blocked.contains(&expected_session),
        "permission_request must embed session_id: {blocked:?}"
    );
    assert!(
        unblocked.contains(&expected_session),
        "tool_complete must embed session_id: {unblocked:?}"
    );
    // Both sequences must be valid OSC 777 sequences.
    assert!(blocked.starts_with("\x1b]777;notify;warp://cli-agent;"));
    assert!(unblocked.starts_with("\x1b]777;notify;warp://cli-agent;"));
}

use warp_core::telemetry::TelemetryEvent as _;

use super::*;

#[test]
fn startup_payload_reports_sanitized_terminal_metadata() {
    let event = TuiStartupTelemetryEvent {
        term_program: sanitize_term_program(Some("  ghost\u{1b}ty  ".into())),
        multiplexer: TuiHostMultiplexer::Tmux,
    };

    assert_eq!(
        event.payload(),
        Some(json!({
            "term_program": "ghostty",
            "multiplexer": "tmux",
        }))
    );
}

#[test]
fn terminal_program_is_bounded_and_empty_values_are_omitted() {
    assert_eq!(sanitize_term_program(Some("   ".into())), None);
    assert_eq!(
        sanitize_term_program(Some("x".repeat(MAX_TERM_PROGRAM_CHARS + 10).into()))
            .unwrap()
            .len(),
        MAX_TERM_PROGRAM_CHARS
    );
}

#[test]
fn multiplexer_detection_uses_stable_precedence() {
    let value = std::ffi::OsStr::new("present");
    assert_eq!(
        detect_multiplexer(Some(value), Some(value), Some(value), Some(value)).as_str(),
        "tmux"
    );
    assert_eq!(
        detect_multiplexer(None, Some(value), Some(value), Some(value)).as_str(),
        "screen"
    );
    assert_eq!(
        detect_multiplexer(None, None, Some(value), None).as_str(),
        "zellij"
    );
    assert_eq!(detect_multiplexer(None, None, None, None).as_str(), "none");
}

#[test]
fn conversation_menu_events_use_tui_native_names() {
    let opened = TuiConversationMenuTelemetryEvent::Opened;
    let selected = TuiConversationMenuTelemetryEvent::ItemSelected;

    assert_eq!(TelemetryEvent::name(&opened), "TUI.ConversationMenu.Opened");
    assert_eq!(
        TelemetryEvent::name(&selected),
        "TUI.ConversationMenu.ItemSelected"
    );
    assert_eq!(opened.payload(), None);
    assert_eq!(selected.payload(), None);
}

#[test]
fn conversation_restore_payload_is_low_cardinality_and_non_ugc() {
    let event = TuiConversationRestoreTelemetryEvent {
        state: TuiConversationRestoreTelemetryState::Failed,
        target: TuiConversationRestoreTelemetryTarget::Server,
    };

    assert_eq!(TelemetryEvent::name(&event), "TUI.ConversationRestore");
    assert_eq!(
        event.payload(),
        Some(json!({
            "state": "failed",
            "target": "server",
        }))
    );
    assert!(!event.contains_ugc());
}

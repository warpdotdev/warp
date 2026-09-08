use super::*;

#[test]
fn warp_host_capability_requires_protocol_and_client_versions() {
    let protocol_version = OsStr::new("1");
    let client_version = OsStr::new("local");
    let empty = OsStr::new("");

    assert!(has_warp_host_capability(
        Some(protocol_version),
        Some(client_version)
    ));
    assert!(!has_warp_host_capability(None, Some(client_version)));
    assert!(!has_warp_host_capability(Some(protocol_version), None));
    assert!(!has_warp_host_capability(Some(empty), Some(client_version)));
    assert!(!has_warp_host_capability(
        Some(protocol_version),
        Some(empty)
    ));
}

#[test]
fn status_events_ignore_background_conversations_and_restorations() {
    let selected_conversation_id = AIConversationId::new();

    assert_eq!(
        status_osc_event(
            Some(selected_conversation_id),
            AIConversationId::new(),
            &ConversationStatusUpdate::Changed {
                prev_status: ConversationStatus::InProgress,
            },
            &ConversationStatus::Success,
        ),
        None
    );
    assert_eq!(
        status_osc_event(
            Some(selected_conversation_id),
            selected_conversation_id,
            &ConversationStatusUpdate::Restored,
            &ConversationStatus::Success,
        ),
        None
    );
}

#[test]
fn notification_excerpt_is_trimmed_and_bounded() {
    assert_eq!(
        notification_excerpt("  Finished\n\nupdating   the parser. "),
        Some("Finished updating the parser.".to_owned())
    );

    let long = "x".repeat(MAX_NOTIFICATION_DESCRIPTION_CHARS + 1);
    let excerpt = notification_excerpt(&long).expect("non-empty text should produce an excerpt");
    assert_eq!(
        excerpt.chars().count(),
        MAX_NOTIFICATION_DESCRIPTION_CHARS + 1
    );
    assert!(excerpt.ends_with('…'));
}

#[test]
fn terminal_statuses_map_to_stop_events() {
    let conversation_id = AIConversationId::new();
    let update = ConversationStatusUpdate::Changed {
        prev_status: ConversationStatus::InProgress,
    };

    assert_eq!(
        status_osc_event(
            Some(conversation_id),
            conversation_id,
            &update,
            &ConversationStatus::Success,
        ),
        Some(StatusOscEvent {
            event: "stop",
            error_type: None,
        })
    );
    assert_eq!(
        status_osc_event(
            Some(conversation_id),
            conversation_id,
            &update,
            &ConversationStatus::Error,
        ),
        Some(StatusOscEvent {
            event: "stop_failure",
            error_type: Some("error"),
        })
    );
    assert_eq!(
        status_osc_event(
            Some(conversation_id),
            conversation_id,
            &update,
            &ConversationStatus::Cancelled,
        ),
        Some(StatusOscEvent {
            event: "stop_failure",
            error_type: Some("cancelled"),
        })
    );
}

#[test]
fn leaving_blocked_status_publishes_permission_replied() {
    let conversation_id = AIConversationId::new();

    assert_eq!(
        status_osc_event(
            Some(conversation_id),
            conversation_id,
            &ConversationStatusUpdate::Changed {
                prev_status: ConversationStatus::Blocked {
                    blocked_action: "Approve?".to_owned(),
                },
            },
            &ConversationStatus::InProgress,
        ),
        Some(StatusOscEvent {
            event: "permission_replied",
            error_type: None,
        })
    );
}

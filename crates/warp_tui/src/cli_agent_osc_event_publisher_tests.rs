use super::*;

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

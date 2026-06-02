use warpui::EntityId;

use super::*;
use crate::ai::agent::conversation::AIConversationId;
use crate::terminal::CLIAgent;

fn make_conversation_notification(
    conversation_id: AIConversationId,
    terminal_view_id: EntityId,
) -> NotificationItem {
    NotificationItem::new(
        "test".to_owned(),
        "msg".to_owned(),
        NotificationCategory::Complete,
        NotificationSourceAgent::Oz { is_ambient: false },
        NotificationOrigin::Conversation(conversation_id),
        false,
        terminal_view_id,
        vec![],
        None,
    )
}

fn make_cli_session_notification(terminal_view_id: EntityId) -> NotificationItem {
    NotificationItem::new(
        "cli test".to_owned(),
        "cli msg".to_owned(),
        NotificationCategory::Complete,
        NotificationSourceAgent::CLI {
            agent: CLIAgent::Claude,
            is_ambient: false,
        },
        NotificationOrigin::CLISession(terminal_view_id),
        false,
        terminal_view_id,
        vec![],
        None,
    )
}

#[test]
fn remove_by_origin_cleans_up_conversation_notification() {
    let mut items = NotificationItems::default();
    let conversation_id = AIConversationId::new();
    let terminal_view_id = EntityId::new();

    items.push(make_conversation_notification(
        conversation_id,
        terminal_view_id,
    ));
    assert_eq!(items.filtered_count(NotificationFilter::All), 1);

    let removed = items.remove_by_origin(NotificationOrigin::Conversation(conversation_id));
    assert!(removed);
    assert_eq!(items.filtered_count(NotificationFilter::All), 0);
}

#[test]
fn remove_by_origin_cleans_up_cli_session_notification() {
    let mut items = NotificationItems::default();
    let terminal_view_id = EntityId::new();

    items.push(make_cli_session_notification(terminal_view_id));
    assert_eq!(items.filtered_count(NotificationFilter::All), 1);

    let removed = items.remove_by_origin(NotificationOrigin::CLISession(terminal_view_id));
    assert!(removed);
    assert_eq!(items.filtered_count(NotificationFilter::All), 0);
}

#[test]
fn remove_by_origin_leaves_unrelated_notifications() {
    let mut items = NotificationItems::default();
    let conv_id = AIConversationId::new();
    let terminal_a = EntityId::new();
    let terminal_b = EntityId::new();

    items.push(make_conversation_notification(conv_id, terminal_a));
    items.push(make_cli_session_notification(terminal_b));
    assert_eq!(items.filtered_count(NotificationFilter::All), 2);

    // Remove only the conversation notification; the CLI session notification should remain.
    let removed = items.remove_by_origin(NotificationOrigin::Conversation(conv_id));
    assert!(removed);
    assert_eq!(items.filtered_count(NotificationFilter::All), 1);

    let remaining = items
        .items_filtered(NotificationFilter::All)
        .next()
        .unwrap();
    assert_eq!(remaining.origin, NotificationOrigin::CLISession(terminal_b));
}

#[test]
fn remove_by_origin_returns_false_when_nothing_to_remove() {
    let mut items = NotificationItems::default();
    let terminal_view_id = EntityId::new();

    let removed = items.remove_by_origin(NotificationOrigin::CLISession(terminal_view_id));
    assert!(!removed);
}

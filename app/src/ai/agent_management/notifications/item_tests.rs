use warpui::EntityId;

use super::*;
use crate::ai::agent::conversation::AIConversationId;
use crate::terminal::CLIAgent;

fn make_conversation_notification(
    conversation_id: AIConversationId,
    terminal_view_id: EntityId,
) -> NotificationItem {
    make_conversation_notification_with(
        conversation_id,
        terminal_view_id,
        false,
        NotificationCategory::Complete,
        false,
    )
}

fn make_conversation_notification_with(
    conversation_id: AIConversationId,
    terminal_view_id: EntityId,
    is_ambient: bool,
    category: NotificationCategory,
    is_read: bool,
) -> NotificationItem {
    NotificationItem::new(
        "test".to_owned(),
        "msg".to_owned(),
        category,
        NotificationSourceAgent::Oz { is_ambient },
        NotificationOrigin::Conversation(conversation_id),
        is_read,
        terminal_view_id,
        vec![],
        None,
    )
}

fn make_cli_session_notification(terminal_view_id: EntityId) -> NotificationItem {
    make_cli_session_notification_with(
        terminal_view_id,
        CLIAgent::Claude,
        false,
        NotificationCategory::Complete,
        false,
    )
}

fn make_cli_session_notification_with(
    terminal_view_id: EntityId,
    agent: CLIAgent,
    is_ambient: bool,
    category: NotificationCategory,
    is_read: bool,
) -> NotificationItem {
    NotificationItem::new(
        "cli test".to_owned(),
        "cli msg".to_owned(),
        category,
        NotificationSourceAgent::CLI { agent, is_ambient },
        NotificationOrigin::CLISession(terminal_view_id),
        is_read,
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

#[test]
fn dock_badge_count_counts_requested_unread_local_outcomes() {
    let mut items = NotificationItems::default();

    items.push(make_conversation_notification(
        AIConversationId::new(),
        EntityId::new(),
    ));
    for agent in [
        CLIAgent::Claude,
        CLIAgent::Codex,
        CLIAgent::Auggie,
        CLIAgent::OpenCode,
    ] {
        items.push(make_cli_session_notification_with(
            EntityId::new(),
            agent,
            false,
            NotificationCategory::Complete,
            false,
        ));
    }
    items.push(make_cli_session_notification_with(
        EntityId::new(),
        CLIAgent::Claude,
        false,
        NotificationCategory::Request,
        false,
    ));

    assert_eq!(items.dock_badge_count(), 6);
}

#[test]
fn dock_badge_count_excludes_non_matching_notifications() {
    let mut items = NotificationItems::default();
    let terminal_view_id = EntityId::new();

    items.push(make_conversation_notification_with(
        AIConversationId::new(),
        terminal_view_id,
        true,
        NotificationCategory::Complete,
        false,
    ));
    items.push(make_conversation_notification_with(
        AIConversationId::new(),
        terminal_view_id,
        false,
        NotificationCategory::Complete,
        true,
    ));
    items.push(make_conversation_notification_with(
        AIConversationId::new(),
        terminal_view_id,
        false,
        NotificationCategory::Error,
        false,
    ));
    items.push(make_cli_session_notification_with(
        EntityId::new(),
        CLIAgent::Claude,
        false,
        NotificationCategory::Complete,
        true,
    ));
    items.push(make_cli_session_notification_with(
        EntityId::new(),
        CLIAgent::Claude,
        true,
        NotificationCategory::Complete,
        false,
    ));
    items.push(make_cli_session_notification_with(
        EntityId::new(),
        CLIAgent::Claude,
        false,
        NotificationCategory::Error,
        false,
    ));
    for agent in [CLIAgent::Gemini, CLIAgent::Amp, CLIAgent::Unknown] {
        items.push(make_cli_session_notification_with(
            EntityId::new(),
            agent,
            false,
            NotificationCategory::Complete,
            false,
        ));
    }

    assert_eq!(items.dock_badge_count(), 0);
}

#[test]
fn dock_badge_count_updates_after_read_all_and_remove() {
    let mut items = NotificationItems::default();
    let terminal_a = EntityId::new();
    let terminal_b = EntityId::new();

    let first = make_cli_session_notification_with(
        terminal_a,
        CLIAgent::Claude,
        false,
        NotificationCategory::Complete,
        false,
    );
    let first_id = first.id;
    items.push(first);
    items.push(make_cli_session_notification_with(
        terminal_b,
        CLIAgent::Codex,
        false,
        NotificationCategory::Request,
        false,
    ));
    assert_eq!(items.dock_badge_count(), 2);

    assert!(items.mark_item_read(first_id));
    assert_eq!(items.dock_badge_count(), 1);

    assert!(items.remove_by_origin(NotificationOrigin::CLISession(terminal_b)));
    assert_eq!(items.dock_badge_count(), 0);

    items.push(make_cli_session_notification(EntityId::new()));
    assert_eq!(items.dock_badge_count(), 1);

    assert!(items.mark_all_items_read());
    assert_eq!(items.dock_badge_count(), 0);
}

#[test]
fn dock_badge_count_counts_terminals_not_items() {
    let mut items = NotificationItems::default();
    let terminal_view_id = EntityId::new();

    items.push(make_conversation_notification(
        AIConversationId::new(),
        terminal_view_id,
    ));
    items.push(make_conversation_notification(
        AIConversationId::new(),
        terminal_view_id,
    ));

    assert_eq!(items.filtered_count(NotificationFilter::Unread), 2);
    assert_eq!(items.dock_badge_count(), 1);
}

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

fn make_sleep_prompt_notification(terminal_view_id: EntityId) -> NotificationItem {
    NotificationItem::new(
        "Turn on auto-handoff".to_owned(),
        "Connection lost".to_owned(),
        NotificationCategory::Error,
        NotificationSourceAgent::Oz { is_ambient: false },
        NotificationOrigin::AutoHandoffSleepPrompt,
        false,
        terminal_view_id,
        vec![],
        None,
    )
    .with_actions(vec![
        NotificationAction {
            label: "Enable".to_owned(),
            style: NotificationActionStyle::Primary,
            kind: NotificationActionKind::EnableAutoHandoffOnSleep,
        },
        NotificationAction {
            label: "Dismiss".to_owned(),
            style: NotificationActionStyle::Secondary,
            kind: NotificationActionKind::DismissSleepPrompt,
        },
    ])
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
fn remove_by_id_removes_only_the_matching_item() {
    let mut items = NotificationItems::default();
    let conv_id = AIConversationId::new();
    let terminal_a = EntityId::new();
    let terminal_b = EntityId::new();

    items.push(make_conversation_notification(conv_id, terminal_a));
    items.push(make_cli_session_notification(terminal_b));
    let target_id = items
        .items_filtered(NotificationFilter::All)
        .find(|item| item.origin == NotificationOrigin::CLISession(terminal_b))
        .unwrap()
        .id;

    assert!(items.remove_by_id(target_id));
    assert_eq!(items.filtered_count(NotificationFilter::All), 1);
    assert!(items.get_by_id(target_id).is_none());
}

#[test]
fn remove_by_id_returns_false_when_nothing_to_remove() {
    let mut items = NotificationItems::default();
    let conv_id = AIConversationId::new();
    let terminal_view_id = EntityId::new();
    items.push(make_conversation_notification(conv_id, terminal_view_id));

    // A fresh notification's id won't be present (it was never pushed).
    let unrelated = make_cli_session_notification(EntityId::new());
    assert!(!items.remove_by_id(unrelated.id));
    assert_eq!(items.filtered_count(NotificationFilter::All), 1);
}

#[test]
fn sleep_prompt_dedupes_to_single_item_by_origin() {
    let mut items = NotificationItems::default();

    items.push(make_sleep_prompt_notification(EntityId::new()));
    items.push(make_sleep_prompt_notification(EntityId::new()));

    // The unit `AutoHandoffSleepPrompt` origin de-dupes so only one exists.
    assert_eq!(items.filtered_count(NotificationFilter::All), 1);
}

#[test]
fn sleep_prompt_is_independent_from_conversation_notifications() {
    let mut items = NotificationItems::default();
    let conv_id = AIConversationId::new();
    let terminal = EntityId::new();

    items.push(make_conversation_notification(conv_id, terminal));
    items.push(make_sleep_prompt_notification(terminal));
    assert_eq!(items.filtered_count(NotificationFilter::All), 2);

    // Removing the conversation notification leaves the sleep prompt intact.
    assert!(items.remove_by_origin(NotificationOrigin::Conversation(conv_id)));
    let remaining = items
        .items_filtered(NotificationFilter::All)
        .next()
        .unwrap();
    assert_eq!(remaining.origin, NotificationOrigin::AutoHandoffSleepPrompt);
    assert_eq!(remaining.actions.len(), 2);
}

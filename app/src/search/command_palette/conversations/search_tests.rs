use crate::ai::{
    agent::conversation::AIConversationId, conversation_navigation::ConversationNavigationData,
};
use warpui::{EntityId, WindowId};

#[test]
fn test_conversation_navigation_data_ordering() {
    // Create test data with different active states and timestamps
    let now = chrono::Local::now();
    let one_hour_ago = now - chrono::Duration::hours(1);
    let two_hours_ago = now - chrono::Duration::hours(2);

    let active_recent = ConversationNavigationData {
        id: AIConversationId::new(),
        title: "Active Recent".to_string(),
        initial_query: None,
        last_updated: now,
        terminal_view_id: Some(EntityId::new()),
        window_id: Some(WindowId::new()),
        pane_view_locator: None,
        initial_working_directory: None,
        latest_working_directory: None,
        is_selected: true,
        is_closed: false,
        server_conversation_token: None,
        is_in_active_pane: true,
    };

    let active_old = ConversationNavigationData {
        id: AIConversationId::new(),
        title: "Active Old".to_string(),
        initial_query: None,
        last_updated: two_hours_ago,
        terminal_view_id: Some(EntityId::new()),
        window_id: Some(WindowId::new()),
        pane_view_locator: None,
        initial_working_directory: None,
        latest_working_directory: None,
        is_selected: true,
        is_closed: false,
        server_conversation_token: None,
        is_in_active_pane: true,
    };

    let inactive_recent = ConversationNavigationData {
        id: AIConversationId::new(),
        title: "Inactive Recent".to_string(),
        initial_query: None,
        last_updated: now,
        terminal_view_id: Some(EntityId::new()),
        window_id: Some(WindowId::new()),
        pane_view_locator: None,
        initial_working_directory: None,
        latest_working_directory: None,
        is_selected: false,
        is_closed: false,
        server_conversation_token: None,
        is_in_active_pane: false,
    };

    let inactive_old = ConversationNavigationData {
        id: AIConversationId::new(),
        title: "Inactive Old".to_string(),
        initial_query: None,
        last_updated: one_hour_ago,
        terminal_view_id: Some(EntityId::new()),
        window_id: Some(WindowId::new()),
        pane_view_locator: None,
        initial_working_directory: None,
        latest_working_directory: None,
        is_selected: false,
        is_closed: false,
        server_conversation_token: None,
        is_in_active_pane: false,
    };

    let historical_recent = ConversationNavigationData {
        id: AIConversationId::new(),
        title: "Historical Recent".to_string(),
        initial_query: None,
        last_updated: now,
        terminal_view_id: None,
        window_id: None,
        pane_view_locator: None,
        initial_working_directory: None,
        latest_working_directory: None,
        is_selected: false,
        is_closed: false,
        server_conversation_token: None,
        is_in_active_pane: false,
    };

    let historical_old = ConversationNavigationData {
        id: AIConversationId::new(),
        title: "Historical Old".to_string(),
        initial_query: None,
        last_updated: one_hour_ago,
        terminal_view_id: None,
        window_id: None,
        pane_view_locator: None,
        initial_working_directory: None,
        latest_working_directory: None,
        is_selected: false,
        is_closed: false,
        server_conversation_token: None,
        is_in_active_pane: false,
    };

    // Test sorting a vector
    let mut conversations = [
        inactive_old.clone(),
        active_old.clone(),
        inactive_recent.clone(),
        active_recent.clone(),
        historical_old.clone(),
        historical_recent.clone(),
    ];

    conversations.sort();

    assert_eq!(conversations[0].title, "Historical Old");
    assert_eq!(conversations[1].title, "Historical Recent");
    assert_eq!(conversations[2].title, "Inactive Old");
    assert_eq!(conversations[3].title, "Inactive Recent");
    assert_eq!(conversations[4].title, "Active Old");
    assert_eq!(conversations[5].title, "Active Recent");
}

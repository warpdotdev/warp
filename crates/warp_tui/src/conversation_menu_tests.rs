use warp::tui_export::AIConversationId;

use super::*;

#[test]
fn selection_reconciliation_preserves_id_then_uses_nearest_index() {
    let rows = vec![
        TuiConversationMenuRow {
            id: AgentConversationEntryId::Conversation(AIConversationId::new()),
            title: "First".to_owned(),
        },
        TuiConversationMenuRow {
            id: AgentConversationEntryId::Conversation(AIConversationId::new()),
            title: "Second".to_owned(),
        },
        TuiConversationMenuRow {
            id: AgentConversationEntryId::Conversation(AIConversationId::new()),
            title: "Third".to_owned(),
        },
    ];

    let preserved = reconciled_selection_index(&rows, Some(rows[1].id), Some(0));
    assert_eq!(preserved, Some(1));

    let missing = AgentConversationEntryId::Conversation(AIConversationId::new());
    let nearest = reconciled_selection_index(&rows[..2], Some(missing), Some(2));
    assert_eq!(nearest, Some(1));
}

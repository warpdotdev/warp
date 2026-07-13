use warp::tui_export::AIConversationId;

use super::*;

fn candidate(index: usize, title: impl Into<String>) -> TuiConversationMenuCandidate {
    TuiConversationMenuCandidate {
        id: AgentConversationEntryId::Conversation(AIConversationId::new()),
        title: title.into(),
        last_updated_millis: 1_000 - index as i64,
    }
}

#[test]
fn empty_query_caps_recent_rows_and_places_newest_last() {
    let candidates = (0..55)
        .map(|index| candidate(index, format!("Conversation {index}")))
        .collect();

    let rows = build_rows(candidates, "");

    assert_eq!(rows.len(), DEFAULT_RESULT_COUNT);
    assert_eq!(
        rows.first().map(|row| row.title.as_str()),
        Some("Conversation 49")
    );
    assert_eq!(
        rows.last().map(|row| row.title.as_str()),
        Some("Conversation 0")
    );
    assert!(!rows.iter().any(|row| row.title == "Conversation 50"));
}

#[test]
fn fuzzy_query_filters_titles_and_caps_best_results() {
    let mut candidates = vec![
        candidate(0, "Deploy the API"),
        candidate(1, "Fix unit tests"),
        candidate(2, "Deploy the website"),
    ];
    candidates.extend(
        (0..MAX_SEARCH_RESULTS)
            .map(|index| candidate(index + 3, format!("Deploy service {index}"))),
    );

    let rows = build_rows(candidates, "deploy");

    assert_eq!(rows.len(), MAX_SEARCH_RESULTS);
    assert!(rows.iter().all(|row| row.title.contains("Deploy")));
    assert!(!rows.iter().any(|row| row.title == "Fix unit tests"));
}

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

    let preserved = reconcile_selection(&rows, Some(rows[1].id), Some(0));
    assert_eq!(preserved.selected_index(), Some(1));

    let missing = AgentConversationEntryId::Conversation(AIConversationId::new());
    let nearest = reconcile_selection(&rows[..2], Some(missing), Some(2));
    assert_eq!(nearest.selected_index(), Some(1));
}

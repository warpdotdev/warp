use super::*;

// Pre-order traversal correctness for the descendant walker is exercised in
// `app/src/ai/blocklist/orchestration_topology_tests.rs`. These tests stay
// focused on the pill bar's own dispatch behavior.

#[test]
fn navigation_action_for_child_pill_reveals_existing_child_pane() {
    let conversation_id = AIConversationId::new();

    assert!(matches!(
        navigation_action_for_pill(PillKind::Child, conversation_id),
        TerminalAction::RevealChildAgent {
            conversation_id: actual_id,
        } if actual_id == conversation_id
    ));
}

#[test]
fn navigation_action_for_orchestrator_pill_switches_in_place() {
    let conversation_id = AIConversationId::new();

    assert!(matches!(
        navigation_action_for_pill(PillKind::Orchestrator, conversation_id),
        TerminalAction::SwitchAgentViewToConversation {
            conversation_id: actual_id,
        } if actual_id == conversation_id
    ));
}

#[test]
fn pill_status_sort_key_orders_attention_then_in_progress_then_done() {
    let blocked = ConversationStatus::Blocked {
        blocked_action: String::new(),
    };
    let blocked_key = pill_status_sort_key(Some(&blocked));
    let error_key = pill_status_sort_key(Some(&ConversationStatus::Error));
    let in_progress_key = pill_status_sort_key(Some(&ConversationStatus::InProgress));
    let cancelled_key = pill_status_sort_key(Some(&ConversationStatus::Cancelled));
    let success_key = pill_status_sort_key(Some(&ConversationStatus::Success));

    assert!(blocked_key < error_key);
    assert!(error_key < in_progress_key);
    assert!(in_progress_key < cancelled_key);
    // Cancelled and Success share the done bucket; recency decides within it.
    assert_eq!(cancelled_key, success_key);
}

#[test]
fn pill_status_sort_key_treats_none_as_in_progress() {
    // Safety default for the orchestrator path (never sorted in practice).
    assert_eq!(
        pill_status_sort_key(None),
        pill_status_sort_key(Some(&ConversationStatus::InProgress)),
    );
}

#[test]
fn pill_done_recency_key_puts_most_recent_first_and_unknown_last() {
    let older = pill_done_recency_key(Some(1_000));
    let newer = pill_done_recency_key(Some(2_000));
    let unknown = pill_done_recency_key(None);
    assert!(newer < older);
    assert!(older < unknown);
}

#[test]
fn sort_pills_bubbles_attention_in_progress_keeps_spawn_done_uses_recency() {
    let blocked = ConversationStatus::Blocked {
        blocked_action: String::new(),
    };
    // (status, finish time) per spawn index.
    let inputs: Vec<(ConversationStatus, Option<i64>)> = vec![
        (ConversationStatus::Success, Some(100)),
        (ConversationStatus::InProgress, None),
        (blocked.clone(), None),
        (ConversationStatus::Cancelled, Some(300)),
        (ConversationStatus::InProgress, None),
        (ConversationStatus::Error, None),
        (ConversationStatus::Success, Some(200)),
    ];
    let mut sortable: Vec<(u8, i64, usize)> = inputs
        .iter()
        .enumerate()
        .map(|(idx, (status, ms))| {
            let status_key = pill_status_sort_key(Some(status));
            (status_key, pill_secondary_sort_key(status_key, *ms), idx)
        })
        .collect();
    sortable.sort_by_key(|(k, s, idx)| (*k, *s, *idx));
    let spawn_order: Vec<usize> = sortable.iter().map(|(_, _, idx)| *idx).collect();
    // Blocked, Error, InProgress (spawn order), then done by recency desc.
    assert_eq!(spawn_order, vec![2, 5, 1, 4, 3, 6, 0]);
}

#[test]
fn sort_pills_is_stable_within_in_progress_bucket() {
    let in_progress_key = pill_status_sort_key(Some(&ConversationStatus::InProgress));
    let mut entries: Vec<(u8, i64, usize)> = vec![(in_progress_key, 0, 7), (in_progress_key, 0, 3)];
    entries.sort_by_key(|(k, s, idx)| (*k, *s, *idx));
    let spawn_order: Vec<usize> = entries.iter().map(|(_, _, idx)| *idx).collect();
    assert_eq!(spawn_order, vec![3, 7]);
}

#[test]
fn sort_pills_done_bucket_orders_by_recency_regardless_of_completion_type() {
    // Old Cancelled sinks behind a fresh Success.
    let cancelled_old = pill_secondary_sort_key(DONE_STATUS_KEY, Some(100));
    let success_new = pill_secondary_sort_key(DONE_STATUS_KEY, Some(500));
    let mut entries: Vec<(u8, i64, usize)> = vec![
        (DONE_STATUS_KEY, cancelled_old, 0),
        (DONE_STATUS_KEY, success_new, 1),
    ];
    entries.sort_by_key(|(k, s, idx)| (*k, *s, *idx));
    let spawn_order: Vec<usize> = entries.iter().map(|(_, _, idx)| *idx).collect();
    assert_eq!(spawn_order, vec![1, 0]);
}

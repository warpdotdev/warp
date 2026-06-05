use super::*;

// Pre-order traversal correctness for the descendant walker is exercised in
// `app/src/ai/blocklist/orchestration_topology_tests.rs`. These tests stay
// focused on the pill bar's own dispatch behavior.

/// The data layer that `OrchestrationPillBar::pill_specs` reads must
/// surface restored orchestration children before any pane has been created.
///
/// `pill_specs` (defined privately on `OrchestrationPillBar`) walks
/// `descendant_conversation_ids_in_spawn_order(history, orchestrator_id)` and
/// then `filter_map(|id| history.conversation(&id))`. The
/// `history.conversation(&id)` lookup must return `Some` for restored
/// children even before the parent's hidden pane materializes, or the pill
/// bar renders nothing. This test asserts both layers work after
/// `BlocklistAIHistoryModel::new` runs, before any `restore_conversations` /
/// pane materialization.
#[test]
fn pill_bar_data_layer_finds_restored_children_before_pane_creation() {
    use chrono::Utc;
    use uuid::Uuid;
    use warpui::App;

    use crate::ai::blocklist::orchestration_topology::descendant_conversation_ids_in_spawn_order;
    use crate::ai::blocklist::BlocklistAIHistoryModel;
    use crate::persistence::model::{
        AgentConversation, AgentConversationData, AgentConversationRecord,
    };

    App::test((), |app| async move {
        let parent_id = AIConversationId::new();
        let child_id = AIConversationId::new();
        let parent_run_id = Uuid::new_v4().to_string();
        let child_run_id = Uuid::new_v4().to_string();
        let now = Utc::now().naive_utc();

        let conversations = vec![
            AgentConversation {
                conversation: AgentConversationRecord {
                    id: 1,
                    conversation_id: child_id.to_string(),
                    conversation_data: serde_json::to_string(&AgentConversationData {
                        server_conversation_token: Some("child-token".to_string()),
                        conversation_usage_metadata: None,
                        reverted_action_ids: None,
                        forked_from_server_conversation_token: None,
                        artifacts_json: None,
                        parent_agent_id: Some(parent_run_id.clone()),
                        agent_name: Some("Agent 1".to_string()),
                        orchestration_harness_type: None,
                        parent_conversation_id: Some(parent_id.to_string()),
                        is_remote_child: false,
                        root_task_is_optimistic: None,
                        run_id: Some(child_run_id.clone()),
                        autoexecute_override: None,
                        last_event_sequence: None,
                        pinned: false,
                    })
                    .expect("child conversation data should serialize"),
                    last_modified_at: now,
                },
                tasks: vec![warp_multi_agent_api::Task {
                    id: format!("task-{child_id}"),
                    messages: vec![warp_multi_agent_api::Message {
                        id: "child-msg".to_string(),
                        task_id: format!("task-{child_id}"),
                        server_message_data: String::new(),
                        citations: vec![],
                        message: Some(warp_multi_agent_api::message::Message::UserQuery(
                            warp_multi_agent_api::message::UserQuery {
                                query: "Child query".to_string(),
                                context: None,
                                referenced_attachments: Default::default(),
                                mode: None,
                                intended_agent: Default::default(),
                            },
                        )),
                        request_id: "request-1".to_string(),
                        timestamp: None,
                    }],
                    dependencies: None,
                    description: "Child query".to_string(),
                    summary: String::new(),
                    server_data: String::new(),
                }],
            },
            AgentConversation {
                conversation: AgentConversationRecord {
                    id: 2,
                    conversation_id: parent_id.to_string(),
                    conversation_data: serde_json::to_string(&AgentConversationData {
                        server_conversation_token: Some("parent-token".to_string()),
                        conversation_usage_metadata: None,
                        reverted_action_ids: None,
                        forked_from_server_conversation_token: None,
                        artifacts_json: None,
                        parent_agent_id: None,
                        agent_name: None,
                        orchestration_harness_type: None,
                        parent_conversation_id: None,
                        is_remote_child: false,
                        root_task_is_optimistic: None,
                        run_id: Some(parent_run_id.clone()),
                        autoexecute_override: None,
                        last_event_sequence: None,
                        pinned: false,
                    })
                    .expect("parent conversation data should serialize"),
                    last_modified_at: now - chrono::Duration::seconds(1),
                },
                tasks: vec![warp_multi_agent_api::Task {
                    id: format!("task-{parent_id}"),
                    messages: vec![warp_multi_agent_api::Message {
                        id: "parent-msg".to_string(),
                        task_id: format!("task-{parent_id}"),
                        server_message_data: String::new(),
                        citations: vec![],
                        message: Some(warp_multi_agent_api::message::Message::UserQuery(
                            warp_multi_agent_api::message::UserQuery {
                                query: "Parent query".to_string(),
                                context: None,
                                referenced_attachments: Default::default(),
                                mode: None,
                                intended_agent: Default::default(),
                            },
                        )),
                        request_id: "request-2".to_string(),
                        timestamp: None,
                    }],
                    dependencies: None,
                    description: "Parent query".to_string(),
                    summary: String::new(),
                    server_data: String::new(),
                }],
            },
        ];

        let history_model =
            app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &conversations));

        history_model.read(&app, |model, _| {
            // pill_specs walks `descendant_conversation_ids_in_spawn_order`
            // first. This index must be populated for restored children at
            // app startup, before any pane materializes.
            let descendants = descendant_conversation_ids_in_spawn_order(model, parent_id);
            assert_eq!(
                descendants,
                vec![child_id],
                "orchestration topology must surface restored children before any pane is created",
            );

            // pill_specs then collects pill specs via
            // `descendants.into_iter().filter_map(|id| history.conversation(&id))`.
            // The child must be hydrated eagerly so this lookup succeeds and
            // the pill bar renders; otherwise the filter_map would drop the
            // child (because `conversation(&child_id)` returned `None`) and
            // `pill_specs` would return `None` from the
            // `children.is_empty()` early-exit.
            let resolved_children: Vec<&AIConversation> = descendants
                .iter()
                .filter_map(|id| model.conversation(id))
                .collect();
            assert_eq!(
                resolved_children.len(),
                1,
                "restored child conversation must be available in conversations_by_id so \
                 OrchestrationPillBar::pill_specs renders a child pill",
            );
            assert_eq!(resolved_children[0].id(), child_id);
            assert_eq!(resolved_children[0].agent_name(), Some("Agent 1"));
        });
    });
}

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

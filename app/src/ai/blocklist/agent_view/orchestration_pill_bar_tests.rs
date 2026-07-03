use super::*;

// Traversal and canonical pill-order correctness are exercised in
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
                        fetched_memories: vec![],
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
                        fetched_memories: vec![],
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

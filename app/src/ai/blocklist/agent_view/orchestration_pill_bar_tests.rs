use super::*;

// Traversal and canonical pill-order correctness are exercised in
// `app/src/ai/blocklist/orchestration_topology_tests.rs`. These tests stay
// focused on the pill bar's own dispatch behavior.

#[test]
fn pill_bar_scrollable_finite_under_capped_drag_preview() {
    use pathfinder_geometry::vector::vec2f;
    use warpui::elements::new_scrollable::{NewScrollable, SingleAxisConfig};
    use warpui::elements::{
        ClippedScrollStateHandle, ConstrainedBox, Container, CrossAxisAlignment, Fill, Flex,
        MainAxisSize, ParentElement, Rect,
    };
    use warpui::platform::WindowStyle;
    use warpui::{
        App, Element, Entity, Presenter, TypedActionView, View, ViewContext, WindowInvalidation,
    };

    // Mirror of `PaneView::DRAG_PREVIEW_HEADER_MAX_WIDTH`; kept local so the test
    // documents the finite cap the fix relies on.
    const DRAG_PREVIEW_HEADER_MAX_WIDTH: f32 = 400.;

    struct DragPreviewTestView {
        scroll_state: ClippedScrollStateHandle,
    }

    impl DragPreviewTestView {
        fn new(_ctx: &mut ViewContext<Self>) -> Self {
            Self {
                scroll_state: ClippedScrollStateHandle::new(),
            }
        }
    }

    impl Entity for DragPreviewTestView {
        type Event = ();
    }

    impl View for DragPreviewTestView {
        fn ui_name() -> &'static str {
            "DragPreviewTestView"
        }

        fn render(&self, _app: &warpui::AppContext) -> Box<dyn Element> {
            // Overflowing content so the clipped scrollable has something to clip.
            let content = ConstrainedBox::new(Rect::new().finish())
                .with_width(2000.)
                .with_height(22.)
                .finish();
            let scrollable = NewScrollable::horizontal(
                SingleAxisConfig::Clipped {
                    handle: self.scroll_state.clone(),
                    child: Container::new(content).finish(),
                },
                Fill::None,
                Fill::None,
                Fill::None,
            )
            .finish();
            let bar = Container::new(scrollable).finish();
            // The pane-header content column stretches the bar to the header width.
            let header_column = Flex::column()
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_main_axis_size(MainAxisSize::Min)
                .with_child(bar)
                .finish();
            // The fix: the drag-preview column caps the header to a finite width.
            let drag_preview = Flex::column()
                .with_main_axis_size(MainAxisSize::Min)
                .with_child(
                    ConstrainedBox::new(header_column)
                        .with_max_width(DRAG_PREVIEW_HEADER_MAX_WIDTH)
                        .finish(),
                )
                .finish();
            // Outer row hands the drag-preview column an unbounded max width.
            Flex::row()
                .with_main_axis_size(MainAxisSize::Min)
                .with_child(drag_preview)
                .finish()
        }
    }

    impl TypedActionView for DragPreviewTestView {
        type Action = ();
    }

    App::test((), |mut app| async move {
        let (window_id, _view) =
            app.add_window(WindowStyle::NotStealFocus, DragPreviewTestView::new);
        let root_view_id = app
            .root_view_id(window_id)
            .expect("window should have a root view");

        let mut presenter = Presenter::new(window_id);
        let invalidation = WindowInvalidation {
            updated: [root_view_id].into_iter().collect(),
            ..Default::default()
        };

        app.update(move |ctx| {
            presenter.invalidate(invalidation, ctx);
            // Before the fix this panics in `Scene::validate_rect` while painting
            // the scrollable at an infinite/NaN width.
            let scene = presenter.build_scene(vec2f(400., 300.), 1., None, ctx);

            // Every painted rect must have a finite size. Without the width cap,
            // the clipped scrollable would paint an infinite/NaN-width rect.
            for layer in scene.layers() {
                for rect in &layer.rects {
                    let size = rect.bounds.size();
                    assert!(
                        size.x().is_finite() && size.y().is_finite(),
                        "painted rect should be finite under a capped drag preview, got {size:?}",
                    );
                }
            }
        });
    });
}

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

    use crate::ai::blocklist::BlocklistAIHistoryModel;
    use crate::ai::blocklist::orchestration_topology::descendant_conversation_ids_in_spawn_order;
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
                    summary: None,
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
                    summary: None,
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

        let history_model = app
            .add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], vec![], &conversations));

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

/// Builds a 3-level tree (root → mid → grandchild) and returns the history
/// handle plus the ids.
fn build_three_level_tree(
    app: &mut warpui::App,
) -> (
    ModelHandle<BlocklistAIHistoryModel>,
    AIConversationId,
    AIConversationId,
    AIConversationId,
) {
    use warpui::EntityId;

    use crate::test_util::settings::initialize_history_persistence_for_tests;

    initialize_history_persistence_for_tests(app);
    let terminal_view_id = EntityId::new();
    let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
    let root_id = history_model.update(app, |history, ctx| {
        history.start_new_conversation(terminal_view_id, false, false, false, ctx)
    });
    let mid_id = history_model.update(app, |history, ctx| {
        history.start_new_child_conversation(
            terminal_view_id,
            "mid".to_string(),
            root_id,
            None,
            false,
            ctx,
        )
    });
    let grandchild_id = history_model.update(app, |history, ctx| {
        history.start_new_child_conversation(
            terminal_view_id,
            "grandchild".to_string(),
            mid_id,
            None,
            false,
            ctx,
        )
    });
    (history_model, root_id, mid_id, grandchild_id)
}

#[test]
fn breadcrumb_ids_show_only_root_when_parent_is_root() {
    use warpui::App;

    App::test((), |mut app| async move {
        let (history_model, root_id, mid_id, _grandchild_id) = build_three_level_tree(&mut app);
        history_model.read(&app, |history, _| {
            assert_eq!(breadcrumb_ids(history, mid_id), (Some(root_id), None));
        });
    });
}

#[test]
fn breadcrumb_ids_show_root_and_parent_when_two_levels_deep() {
    use warpui::App;

    App::test((), |mut app| async move {
        let (history_model, root_id, mid_id, grandchild_id) = build_three_level_tree(&mut app);
        history_model.read(&app, |history, _| {
            assert_eq!(
                breadcrumb_ids(history, grandchild_id),
                (Some(root_id), Some(mid_id)),
            );
        });
    });
}

#[test]
fn breadcrumb_ids_are_empty_at_the_root() {
    use warpui::App;

    App::test((), |mut app| async move {
        let (history_model, root_id, _mid_id, _grandchild_id) = build_three_level_tree(&mut app);
        history_model.read(&app, |history, _| {
            assert_eq!(breadcrumb_ids(history, root_id), (None, None));
        });
    });
}

#[test]
fn drill_down_anchor_is_the_parent_level_for_a_leaf() {
    use warpui::App;

    App::test((), |mut app| async move {
        let (_history_model, _root_id, mid_id, grandchild_id) = build_three_level_tree(&mut app);
        app.read(|ctx| {
            let grandchild = BlocklistAIHistoryModel::as_ref(ctx)
                .conversation(&grandchild_id)
                .expect("grandchild conversation exists");
            assert_eq!(
                drill_down_anchor_id(grandchild_id, grandchild, ctx),
                mid_id,
                "a leaf anchors its parent's level so sibling navigation stays symmetric",
            );
        });
    });
}

#[test]
fn drill_down_anchor_is_the_node_itself_when_it_has_children() {
    use warpui::App;

    App::test((), |mut app| async move {
        let (_history_model, root_id, mid_id, _grandchild_id) = build_three_level_tree(&mut app);
        app.read(|ctx| {
            let history = BlocklistAIHistoryModel::as_ref(ctx);
            let root = history
                .conversation(&root_id)
                .expect("root conversation exists");
            let mid = history
                .conversation(&mid_id)
                .expect("mid conversation exists");
            assert_eq!(drill_down_anchor_id(root_id, root, ctx), root_id);
            assert_eq!(
                drill_down_anchor_id(mid_id, mid, ctx),
                mid_id,
                "a node with children anchors its own level",
            );
        });
    });
}

/// At orchestration depth 1 the drill-down anchoring must match the
/// historical root-anchored behavior exactly: both the root and its leaf
/// children anchor the root's level.
#[test]
fn drill_down_anchor_matches_root_anchoring_at_depth_one() {
    use warpui::{App, EntityId};

    use crate::test_util::settings::initialize_history_persistence_for_tests;

    App::test((), |mut app| async move {
        initialize_history_persistence_for_tests(&mut app);
        let terminal_view_id = EntityId::new();
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
        let root_id = history_model.update(&mut app, |history, ctx| {
            history.start_new_conversation(terminal_view_id, false, false, false, ctx)
        });
        let child_id = history_model.update(&mut app, |history, ctx| {
            history.start_new_child_conversation(
                terminal_view_id,
                "child".to_string(),
                root_id,
                None,
                false,
                ctx,
            )
        });

        app.read(|ctx| {
            let history = BlocklistAIHistoryModel::as_ref(ctx);
            let root = history
                .conversation(&root_id)
                .expect("root conversation exists");
            let child = history
                .conversation(&child_id)
                .expect("child conversation exists");
            assert_eq!(drill_down_anchor_id(root_id, root, ctx), root_id);
            assert_eq!(drill_down_anchor_id(child_id, child, ctx), root_id);
        });
    });
}

/// A restored child linked to its parent ONLY via a legacy server
/// conversation token in `parent_agent_id` (no explicit parent conversation
/// id, no run id) must index under the parent at restore and resolve
/// breadcrumbs once the parent is loaded: child indexing, the root walk,
/// and breadcrumb targets all flow through the history model's canonical
/// parent resolution (run-id index with a server-token fallback).
#[test]
fn breadcrumbs_resolve_token_only_parent_linkage_after_restore() {
    use chrono::Utc;
    use warpui::App;

    use crate::ai::blocklist::BlocklistAIHistoryModel;
    use crate::persistence::model::{
        AgentConversation, AgentConversationData, AgentConversationRecord,
    };

    fn user_query_task(
        conversation_id: AIConversationId,
        query: &str,
    ) -> warp_multi_agent_api::Task {
        warp_multi_agent_api::Task {
            id: format!("task-{conversation_id}"),
            messages: vec![warp_multi_agent_api::Message {
                fetched_memories: vec![],
                id: format!("msg-{conversation_id}"),
                task_id: format!("task-{conversation_id}"),
                server_message_data: String::new(),
                citations: vec![],
                message: Some(warp_multi_agent_api::message::Message::UserQuery(
                    warp_multi_agent_api::message::UserQuery {
                        query: query.to_string(),
                        context: None,
                        referenced_attachments: Default::default(),
                        mode: None,
                        intended_agent: Default::default(),
                    },
                )),
                request_id: format!("request-{conversation_id}"),
                timestamp: None,
            }],
            dependencies: None,
            description: query.to_string(),
            summary: String::new(),
            server_data: String::new(),
        }
    }

    App::test((), |mut app| async move {
        let root_id = AIConversationId::new();
        let child_id = AIConversationId::new();
        let now = Utc::now().naive_utc();

        let root_data = AgentConversationData {
            server_conversation_token: Some("root-token".to_string()),
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
            run_id: None,
            autoexecute_override: None,
            last_event_sequence: None,
            pinned: false,
        };
        let child_data = AgentConversationData {
            server_conversation_token: None,
            conversation_usage_metadata: None,
            reverted_action_ids: None,
            forked_from_server_conversation_token: None,
            artifacts_json: None,
            parent_agent_id: Some("root-token".to_string()),
            agent_name: Some("child".to_string()),
            orchestration_harness_type: None,
            parent_conversation_id: None,
            is_remote_child: false,
            root_task_is_optimistic: None,
            run_id: None,
            autoexecute_override: None,
            last_event_sequence: None,
            pinned: false,
        };

        let conversations = vec![
            AgentConversation {
                conversation: AgentConversationRecord {
                    id: 1,
                    conversation_id: child_id.to_string(),
                    conversation_data: serde_json::to_string(&child_data)
                        .expect("child conversation data should serialize"),
                    last_modified_at: now,
                    summary: None,
                },
                tasks: vec![user_query_task(child_id, "Child query")],
            },
            AgentConversation {
                conversation: AgentConversationRecord {
                    id: 2,
                    conversation_id: root_id.to_string(),
                    conversation_data: serde_json::to_string(&root_data)
                        .expect("root conversation data should serialize"),
                    last_modified_at: now - chrono::Duration::seconds(1),
                    summary: None,
                },
                tasks: vec![user_query_task(root_id, "Root query")],
            },
        ];

        let history_model = app
            .add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], vec![], &conversations));

        // Token-linked children index under the parent at restore.
        history_model.read(&app, |history, _| {
            assert_eq!(history.child_conversation_ids_of(&root_id), &[child_id]);
        });

        // Load the root (its pane restore normally does this), then confirm
        // the breadcrumb targets resolve across the token-only linkage.
        history_model.update(&mut app, |history, _ctx| {
            history
                .insert_forked_conversation_from_tasks(
                    root_id,
                    vec![user_query_task(root_id, "Root query")],
                    root_data,
                )
                .expect("root conversation should hydrate");
        });
        history_model.read(&app, |history, _| {
            assert_eq!(breadcrumb_ids(history, child_id), (Some(root_id), None));
        });
    });
}

/// The pill bar's history subscription must treat
/// `ConversationServerTokenAssigned` as a re-render trigger: a remote child's
/// run-id linkage can land after `StartedNewConversation` (via
/// `assign_run_id_for_conversation`), and pill contents keyed on run linkage
/// would otherwise stay stale until an unrelated status event fired.
#[test]
fn conversation_server_token_assignment_rerenders_the_pill_bar() {
    use std::sync::Arc;

    use parking_lot::FairMutex;
    use warpui::App;
    use warpui::r#async::executor::Background;
    use warpui::platform::WindowStyle;

    use crate::ai::blocklist::agent_view::{AgentViewEntryOrigin, EphemeralMessageModel};
    use crate::terminal::TerminalModel;
    use crate::terminal::color::{self, Colors};
    use crate::terminal::event_listener::ChannelEventListener;
    use crate::terminal::model::test_utils::block_size;
    use crate::test_util::settings::initialize_history_persistence_for_tests;

    /// Hosts the pill bar without embedding it in the rendered element tree,
    /// so the scene builds triggered by its notifications stay trivial.
    struct PillBarHost {
        pill_bar: ViewHandle<OrchestrationPillBar>,
    }

    impl Entity for PillBarHost {
        type Event = ();
    }

    impl View for PillBarHost {
        fn ui_name() -> &'static str {
            "PillBarHost"
        }

        fn render(&self, _app: &AppContext) -> Box<dyn Element> {
            Empty::new().finish()
        }
    }

    impl TypedActionView for PillBarHost {
        type Action = ();
    }

    App::test((), |mut app| async move {
        initialize_history_persistence_for_tests(&mut app);
        app.add_singleton_model(|_| Appearance::mock());
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
        app.add_singleton_model(|ctx| OrchestrationPillBarModel::new(HashSet::new(), ctx));

        let terminal_view_id = EntityId::new();
        let terminal_model = Arc::new(FairMutex::new(TerminalModel::new_for_test(
            block_size(),
            color::List::from(&Colors::default()),
            ChannelEventListener::new_for_test(),
            Arc::new(Background::default()),
            false,
            None,
            false,
            false,
            None,
        )));
        let ephemeral_message_model = app.add_model(|_| EphemeralMessageModel::new());
        let agent_view_controller = app.add_model(|_| {
            AgentViewController::new(terminal_model, terminal_view_id, ephemeral_message_model)
        });

        let root_id = history_model.update(&mut app, |history, ctx| {
            history.start_new_conversation(terminal_view_id, false, false, false, ctx)
        });
        let child_id = history_model.update(&mut app, |history, ctx| {
            history.start_new_child_conversation(
                terminal_view_id,
                "child".to_string(),
                root_id,
                None,
                false,
                ctx,
            )
        });

        // Activate the agent view on the root BEFORE the pill bar exists so
        // the entry events cannot populate its mouse-state cache; the only
        // event the bar sees below is the token assignment.
        agent_view_controller.update(&mut app, |controller, ctx| {
            controller
                .try_enter_agent_view(
                    Some(root_id),
                    AgentViewEntryOrigin::ConversationSelector,
                    ctx,
                )
                .expect("agent view entry should succeed");
        });

        let (_window_id, host) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let controller = agent_view_controller.clone();
            let pill_bar =
                ctx.add_typed_action_view(move |ctx| OrchestrationPillBar::new(controller, ctx));
            PillBarHost { pill_bar }
        });
        let pill_bar = host.read(&app, |host, _| host.pill_bar.clone());

        pill_bar.read(&app, |bar, _| {
            assert!(
                bar.mouse_states.borrow().is_empty(),
                "no history event has reached the pill bar yet",
            );
        });

        history_model.update(&mut app, |history, ctx| {
            history.assign_run_id_for_conversation(
                child_id,
                "run-123".to_string(),
                None,
                terminal_view_id,
                ctx,
            );
        });

        pill_bar.read(&app, |bar, _| {
            let mouse_states = bar.mouse_states.borrow();
            assert!(
                mouse_states.contains_key(&root_id) && mouse_states.contains_key(&child_id),
                "ConversationServerTokenAssigned must re-render the pill bar, refreshing \
                 mouse states for the visible pills",
            );
        });
    });
}

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;
use crate::ai::agent::conversation::LocalChildRestorationStatus;
use crate::ai::blocklist::orchestration_event_streamer::LocalChildHistoryInstallation;
use crate::ai::blocklist::orchestration_events::{PendingEvent, PendingEventDetail};
use crate::server::server_api::ai::MockAIClient;

struct LocalChildFixture {
    panes: ViewHandle<PaneGroup>,
    streamer: ModelHandle<OrchestrationEventStreamer>,
    child_id: AIConversationId,
    parent_id: AIConversationId,
    terminal: ViewHandle<TerminalView>,
    target: LocalChildRestorationTarget,
    prepared: Rc<RefCell<Option<(LocalChildHistoryInstallation, AIConversation)>>>,
    reported_errors: Arc<AtomicUsize>,
}

fn prepare_local_child(app: &mut App) -> LocalChildFixture {
    let mut client = MockAIClient::new();
    client
        .expect_get_ai_conversation()
        .times(1)
        .returning(|token| {
            let mut metadata = test_server_conversation_metadata(None);
            metadata.server_conversation_token = token;
            Ok((
                warp_multi_agent_api::ConversationData {
                    tasks: vec![warp_multi_agent_api::Task {
                        id: "existing-root".into(),
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                metadata,
            ))
        });
    let reported_errors = Arc::new(AtomicUsize::new(0));
    client.expect_update_agent_task().returning({
        let reported_errors = reported_errors.clone();
        move |_, state, _, _, status, _, _| {
            assert_eq!(state, Some(warp_graphql::ai::AgentTaskState::Error));
            assert!(status.is_some());
            reported_errors.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    });
    initialize_app_with_streamer(app, Vec::new(), Some(Arc::new(client)));
    let streamer = OrchestrationEventStreamer::handle(app);
    let panes = mock_pane_group(app, Default::default());
    let (child_id, parent_id, terminal, target) = panes.update(app, |panes, ctx| {
        let root_pane = panes.focused_pane_id(ctx);
        let root_terminal = panes.terminal_view_from_pane_id(root_pane, ctx).unwrap();
        let parent_id = start_parent_conversation(panes, root_pane, ctx);
        BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, _| {
            history
                .conversation_mut(&parent_id)
                .unwrap()
                .set_task_id(new_ambient_agent_task_id());
        });
        let mut task = ambient_agent_task_for_current_user(new_ambient_agent_task_id());
        task.conversation_id = Some("existing-child-token".into());
        task.last_event_sequence = Some(23);
        let (child_id, terminal) = panes.prepare_restored_local_child(
            parent_id,
            root_terminal.id(),
            &task,
            HashMap::new(),
            ctx,
        );
        let terminal = terminal.unwrap().unwrap();
        let target = LocalChildRestorationTarget {
            pane_group_id: ctx.view_id(),
            terminal_id: terminal.id(),
        };
        ctx.unsubscribe_to_model(&streamer);
        streamer.update(ctx, |streamer, ctx| {
            streamer.register_restored_local_child(child_id, target, ctx);
            assert!(streamer.restore_local_child(child_id, ctx));
            assert!(streamer.restore_local_child(child_id, ctx));
            streamer.register_restored_local_child(child_id, target, ctx);
        });
        (child_id, parent_id, terminal, target)
    });
    let prepared = Rc::new(RefCell::new(None));
    app.update(|ctx| {
        let prepared = prepared.clone();
        ctx.subscribe_to_model(&streamer, move |_, event, _| {
            if let OrchestrationEventStreamerEvent::LocalChildHistoryPrepared {
                installation,
                conversation,
            } = event
            {
                *prepared.borrow_mut() = Some((*installation, conversation.as_ref().clone()));
            }
        });
    });
    LocalChildFixture {
        panes,
        streamer,
        child_id,
        parent_id,
        terminal,
        target,
        prepared,
        reported_errors,
    }
}

fn assert_restoring(fixture: &LocalChildFixture, app: &App) {
    BlocklistAIHistoryModel::handle(app).read(app, |history, _| {
        let child = history.conversation(&fixture.child_id).unwrap();
        assert_eq!(
            child.local_child_restoration_status(),
            Some(LocalChildRestorationStatus::Restoring)
        );
        assert_eq!(child.last_event_sequence(), Some(23));
        assert!(!child.is_local_child_execution_ready());
    });
    fixture.terminal.read(app, |terminal, ctx| {
        assert!(
            !terminal
                .ai_controller()
                .as_ref(ctx)
                .has_active_stream_for_conversation(fixture.child_id, ctx)
        );
    });
}

#[test]
fn restored_local_child_installation_acknowledgement_and_duplicate_delivery() {
    App::test((), |mut app| async move {
        let fixture = prepare_local_child(&mut app);
        assert_eventually!(
            fixture.prepared.borrow().is_some(),
            "history was not prepared"
        );
        let (installation, prepared) = fixture.prepared.borrow().as_ref().unwrap().clone();
        assert_restoring(&fixture, &app);
        fixture.streamer.update(&mut app, |streamer, ctx| {
            streamer.release_local_subtree_startup(&[fixture.child_id], ctx);
        });
        assert_restoring(&fixture, &app);

        let other_group = mock_pane_group(&mut app, Default::default());
        other_group.update(&mut app, |panes, ctx| {
            panes.install_local_child_history(&fixture.streamer, installation, &prepared, ctx);
        });
        assert_restoring(&fixture, &app);
        let stale = LocalChildHistoryInstallation {
            generation: Uuid::new_v4(),
            ..installation
        };
        fixture.panes.update(&mut app, |panes, ctx| {
            panes.install_local_child_history(&fixture.streamer, stale, &prepared, ctx);
        });
        assert_restoring(&fixture, &app);
        fixture.panes.update(&mut app, |panes, ctx| {
            panes.install_local_child_history(&fixture.streamer, installation, &prepared, ctx);
            let mut duplicate = prepared.clone();
            duplicate.set_last_event_sequence(999);
            panes.install_local_child_history(&fixture.streamer, installation, &duplicate, ctx);
        });
        BlocklistAIHistoryModel::handle(&app).read(&app, |history, _| {
            let child = history.conversation(&fixture.child_id).unwrap();
            assert_eq!(
                child.local_child_restoration_status(),
                Some(LocalChildRestorationStatus::Restored)
            );
            assert!(child.is_local_child_execution_ready());
            assert_eq!(child.last_event_sequence(), Some(23));
            assert_eq!(child.parent_conversation_id(), Some(fixture.parent_id));
            assert_eq!(
                child.server_conversation_token().unwrap().as_str(),
                "existing-child-token"
            );
            assert_eq!(
                history.terminal_surface_id_for_conversation(&fixture.child_id),
                Some(fixture.terminal.id())
            );
        });
        assert_eq!(fixture.reported_errors.load(Ordering::SeqCst), 0);
    });
}

#[test]
fn restored_local_child_subtree_startup_gate() {
    assert!(AIConversation::new(false, false).is_local_child_execution_ready());
    App::test((), |mut app| async move {
        let fixture = prepare_local_child(&mut app);
        assert_eventually!(
            fixture.prepared.borrow().is_some(),
            "history was not prepared"
        );
        let (installation, prepared) = fixture.prepared.borrow().as_ref().unwrap().clone();
        fixture.panes.update(&mut app, |panes, ctx| {
            panes.install_local_child_history(&fixture.streamer, installation, &prepared, ctx);
        });
        BlocklistAIHistoryModel::handle(&app).read(&app, |history, _| {
            let child = history.conversation(&fixture.child_id).unwrap();
            assert_eq!(
                child.local_child_restoration_status(),
                Some(LocalChildRestorationStatus::Restored)
            );
            assert!(!child.is_local_child_execution_ready());
        });
        let (sender, recipient) = BlocklistAIHistoryModel::handle(&app).read(&app, |history, _| {
            let child = history.conversation(&fixture.child_id).unwrap();
            (
                child.parent_agent_id().unwrap().to_owned(),
                child.run_id().unwrap(),
            )
        });
        let _task = ChannelState::mock_server()
            .mock("GET", format!("/api/v1/agent/runs/{recipient}").as_str())
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                serde_json::to_string(&ambient_agent_task_for_current_user(
                    recipient.parse().unwrap(),
                ))
                .unwrap(),
            )
            .create();
        OrchestrationEventService::handle(&app).update(&mut app, |service, ctx| {
            service.enqueue_event_batch(
                fixture.child_id,
                vec![PendingEvent {
                    event_id: "pending-during-startup".into(),
                    source_agent_id: sender,
                    attempt_count: 0,
                    detail: PendingEventDetail::Message {
                        message_id: "pending-during-startup".into(),
                        addresses: vec![recipient],
                        subject: "work".into(),
                        message_body: "continue".into(),
                    },
                }],
                ctx,
            );
        });
        fixture.terminal.read(&app, |terminal, ctx| {
            assert!(
                !terminal
                    .ai_controller()
                    .as_ref(ctx)
                    .has_active_stream_for_conversation(fixture.child_id, ctx)
            );
        });
        fixture.streamer.update(&mut app, |streamer, ctx| {
            streamer.release_local_subtree_startup(&[fixture.child_id], ctx);
        });
        assert_eventually!(
            fixture.terminal.read(&app, |terminal, ctx| terminal
                .ai_controller()
                .as_ref(ctx)
                .has_active_stream_for_conversation(fixture.child_id, ctx)),
            "startup release did not drain the restored child's pending message"
        );
    });
}

#[test]
fn restored_local_child_missing_canonical_pane_fails_closed() {
    App::test((), |mut app| async move {
        let fixture = prepare_local_child(&mut app);
        assert_eventually!(
            fixture.prepared.borrow().is_some(),
            "history was not prepared"
        );
        let (installation, prepared) = fixture.prepared.borrow().as_ref().unwrap().clone();
        fixture.panes.update(&mut app, |panes, ctx| {
            panes.child_agent_panes.remove(&fixture.child_id);
            panes.install_local_child_history(&fixture.streamer, installation, &prepared, ctx);
            panes.install_local_child_history(&fixture.streamer, installation, &prepared, ctx);
        });
        assert_eventually!(
            fixture.reported_errors.load(Ordering::SeqCst) == 1,
            "installation failure was not reported"
        );
        BlocklistAIHistoryModel::handle(&app).read(&app, |history, _| {
            let child = history.conversation(&fixture.child_id).unwrap();
            assert_eq!(
                child.local_child_restoration_status(),
                Some(LocalChildRestorationStatus::Error)
            );
            assert_eq!(child.status(), &ConversationStatus::Error);
            assert!(
                child
                    .status_error_message()
                    .unwrap()
                    .contains("canonical child pane missing")
            );
            assert!(!child.is_local_child_execution_ready());
            assert_eq!(child.last_event_sequence(), Some(23));
        });
    });
}

#[test]
fn restored_local_child_closed_surface_retains_failure_diagnostic_without_reattachment() {
    App::test((), |mut app| async move {
        let fixture = prepare_local_child(&mut app);
        assert_eventually!(
            fixture.prepared.borrow().is_some(),
            "history was not prepared"
        );
        let (installation, prepared) = fixture.prepared.borrow().as_ref().unwrap().clone();
        BlocklistAIHistoryModel::handle(&app).update(&mut app, |history, ctx| {
            history
                .clear_conversations_for_closed_terminal_surface(fixture.target.terminal_id, ctx);
            assert_eq!(
                history.terminal_surface_id_for_conversation(&fixture.child_id),
                None
            );
            assert!(history.conversation(&fixture.child_id).is_some());
        });
        let stale = LocalChildHistoryInstallation {
            generation: Uuid::new_v4(),
            ..installation
        };
        fixture.panes.update(&mut app, |panes, ctx| {
            panes.install_local_child_history(&fixture.streamer, installation, &prepared, ctx);
            panes.install_local_child_history(&fixture.streamer, stale, &prepared, ctx);
            panes.install_local_child_history(&fixture.streamer, installation, &prepared, ctx);
        });
        assert_eventually!(
            fixture.reported_errors.load(Ordering::SeqCst) == 1,
            "closed-surface failure was not reported"
        );
        BlocklistAIHistoryModel::handle(&app).read(&app, |history, _| {
            let child = history.conversation(&fixture.child_id).unwrap();
            assert_eq!(child.status(), &ConversationStatus::Error);
            assert_eq!(
                child.local_child_restoration_status(),
                Some(LocalChildRestorationStatus::Error)
            );
            let diagnostic = child.status_error_message().unwrap();
            assert!(diagnostic.contains(&child.run_id().unwrap()));
            assert!(diagnostic.contains("install transcript"));
            assert!(diagnostic.contains("canonical child pane replaced"));
            assert!(!child.is_local_child_execution_ready());
            assert_eq!(child.last_event_sequence(), Some(23));
            assert_eq!(
                history.terminal_surface_id_for_conversation(&fixture.child_id),
                None
            );
        });
        assert_eq!(fixture.reported_errors.load(Ordering::SeqCst), 1);
    });
}

#[test]
fn restored_local_child_replaced_terminal_rejects_prepared_history() {
    App::test((), |mut app| async move {
        let fixture = prepare_local_child(&mut app);
        assert_eventually!(
            fixture.prepared.borrow().is_some(),
            "history was not prepared"
        );
        let (installation, prepared) = fixture.prepared.borrow().as_ref().unwrap().clone();
        fixture.panes.update(&mut app, |panes, ctx| {
            let current = BlocklistAIHistoryModel::as_ref(ctx)
                .conversation(&fixture.child_id)
                .unwrap()
                .clone();
            let parent_pane = panes.focused_pane_id(ctx);
            panes.discard_child_agent_pane_for_conversation(fixture.child_id, ctx);
            panes.create_hidden_child_agent_pane(current, parent_pane, ctx);
            panes.install_local_child_history(&fixture.streamer, installation, &prepared, ctx);
        });
        assert_eventually!(
            fixture.reported_errors.load(Ordering::SeqCst) == 1,
            "replaced terminal was not rejected"
        );
        BlocklistAIHistoryModel::handle(&app).read(&app, |history, _| {
            let child = history.conversation(&fixture.child_id).unwrap();
            assert_eq!(
                child.local_child_restoration_status(),
                Some(LocalChildRestorationStatus::Error)
            );
            assert!(!child.is_local_child_execution_ready());
        });
    });
}

#[test]
fn restored_local_child_missing_installation_acknowledgement_fails_closed() {
    App::test((), |mut app| async move {
        let fixture = prepare_local_child(&mut app);
        assert_eventually!(
            fixture.prepared.borrow().is_some(),
            "history was not prepared"
        );
        assert_restoring(&fixture, &app);
        assert_eventually!(2000 => fixture.reported_errors.load(Ordering::SeqCst) == 1, "missing installation acknowledgement was not reported");
        BlocklistAIHistoryModel::handle(&app).read(&app, |history, _| {
            let child = history.conversation(&fixture.child_id).unwrap();
            assert_eq!(
                child.local_child_restoration_status(),
                Some(LocalChildRestorationStatus::Error)
            );
            assert!(!child.is_local_child_execution_ready());
        });
    });
}

#[test]
fn restored_local_child_owner_replacement_invalidates_prepared_history() {
    App::test((), |mut app| async move {
        let fixture = prepare_local_child(&mut app);
        assert_eventually!(
            fixture.prepared.borrow().is_some(),
            "history was not prepared"
        );
        let (installation, prepared) = fixture.prepared.borrow().as_ref().unwrap().clone();
        fixture.streamer.update(&mut app, |streamer, ctx| {
            streamer.register_restored_local_child(
                fixture.child_id,
                LocalChildRestorationTarget {
                    terminal_id: EntityId::new(),
                    ..fixture.target
                },
                ctx,
            );
        });
        fixture.panes.update(&mut app, |panes, ctx| {
            panes.install_local_child_history(&fixture.streamer, installation, &prepared, ctx);
        });
        assert_eventually!(
            fixture.reported_errors.load(Ordering::SeqCst) == 1,
            "owner replacement was not reported"
        );
        BlocklistAIHistoryModel::handle(&app).read(&app, |history, _| {
            let child = history.conversation(&fixture.child_id).unwrap();
            assert_eq!(
                child.local_child_restoration_status(),
                Some(LocalChildRestorationStatus::Error)
            );
            assert!(
                child
                    .status_error_message()
                    .unwrap()
                    .contains("owner replaced")
            );
            assert!(!child.is_local_child_execution_ready());
        });
    });
}

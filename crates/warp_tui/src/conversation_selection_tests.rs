use std::sync::Arc;

use parking_lot::FairMutex;
use warp::tui_export::{
    AIConversationId, AgentConversationListEntryState, AgentRunDisplayStatus, AgentViewEntryOrigin,
    BlocklistAIHistoryEvent, BlocklistAIHistoryModel, ConversationSelection,
    ConversationSelectionEvent, ConversationSelectionHandle, Harness, TerminalModel,
    TranscriptScope,
};
use warp_core::execution_mode::{AppExecutionMode, ExecutionMode};
use warpui::{App, EntityId, ModelHandle};

use super::{classify_conversation_list_entry, TuiConversationSelection};

#[test]
fn tui_list_policy_classifies_selected_terminal_and_unavailable_entries() {
    let selected_id = AIConversationId::new();
    assert_eq!(
        classify_conversation_list_entry(
            Some(selected_id),
            Some(selected_id),
            true,
            Some(Harness::Oz),
            &AgentRunDisplayStatus::ConversationSucceeded,
        ),
        AgentConversationListEntryState::Selected
    );
    assert_eq!(
        classify_conversation_list_entry(
            None,
            Some(AIConversationId::new()),
            false,
            Some(Harness::Oz),
            &AgentRunDisplayStatus::ConversationCancelled,
        ),
        AgentConversationListEntryState::Available
    );
    assert_eq!(
        classify_conversation_list_entry(
            None,
            None,
            true,
            Some(Harness::Oz),
            &AgentRunDisplayStatus::TaskInProgress,
        ),
        AgentConversationListEntryState::Unavailable
    );
    assert_eq!(
        classify_conversation_list_entry(
            None,
            None,
            true,
            Some(Harness::Claude),
            &AgentRunDisplayStatus::TaskSucceeded,
        ),
        AgentConversationListEntryState::Unavailable
    );
    assert_eq!(
        classify_conversation_list_entry(
            None,
            None,
            false,
            Some(Harness::Oz),
            &AgentRunDisplayStatus::TaskSucceeded,
        ),
        AgentConversationListEntryState::Unavailable
    );
}

/// Creates a terminal model configured for the TUI's unfiltered transcript.
fn tui_terminal_model() -> Arc<FairMutex<TerminalModel>> {
    let mut terminal_model = TerminalModel::mock(None, None);
    terminal_model
        .block_list_mut()
        .set_transcript_scope(TranscriptScope::Unfiltered);
    Arc::new(FairMutex::new(terminal_model))
}

fn build_tui_selection(
    app: &mut App,
) -> (
    ModelHandle<BlocklistAIHistoryModel>,
    ConversationSelectionHandle,
    EntityId,
    Arc<FairMutex<TerminalModel>>,
) {
    app.add_singleton_model(|ctx| AppExecutionMode::new(ExecutionMode::App, false, ctx));
    let history = app.add_singleton_model(|_| BlocklistAIHistoryModel::default());
    let terminal_surface_id = EntityId::new();
    let terminal_model = tui_terminal_model();
    let terminal_model_for_selection = terminal_model.clone();
    let selection = app.add_model(|ctx| {
        Box::new(TuiConversationSelection::new(
            terminal_surface_id,
            terminal_model_for_selection,
            ctx,
        )) as Box<dyn ConversationSelection>
    });
    (history, selection, terminal_surface_id, terminal_model)
}

#[test]
fn tui_selection_eagerly_owns_session_conversation() {
    App::test((), |mut app| async move {
        let (history, selection, terminal_surface_id, terminal_model) =
            build_tui_selection(&mut app);
        let conversation_id = selection
            .read(&app, |selection, ctx| {
                assert!(selection.is_conversation_active(ctx));
                assert!(selection.is_conversation_fullscreen(ctx));
                selection.selected_conversation_id(ctx)
            })
            .expect("TUI should create its session conversation eagerly");

        history.read(&app, |history, _| {
            assert!(history.conversation(&conversation_id).is_some());
            assert_eq!(
                history
                    .active_conversation(terminal_surface_id)
                    .map(|conversation| conversation.id()),
                Some(conversation_id)
            );
            assert_eq!(
                history
                    .all_live_conversations_for_terminal_surface(terminal_surface_id)
                    .map(|conversation| conversation.id())
                    .collect::<Vec<_>>(),
                vec![conversation_id]
            );
        });
        assert_eq!(
            terminal_model.lock().block_list().active_conversation_id(),
            Some(conversation_id)
        );
        assert_eq!(
            terminal_model.lock().block_list().transcript_scope(),
            &TranscriptScope::Unfiltered
        );

        terminal_model
            .lock()
            .simulate_block("echo before prompt", "before prompt\r\n");
        let restored_conversation_id = terminal_model
            .lock()
            .block_list()
            .blocks()
            .iter()
            .find(|block| block.command_to_string() == "echo before prompt")
            .and_then(|block| block.agent_view_visibility().agent_view_conversation_id());
        assert_eq!(restored_conversation_id, Some(conversation_id));

        selection.update(&mut app, |selection, ctx| {
            selection.select_new_conversation(AgentViewEntryOrigin::Cli, ctx);
        });
        let new_conversation_id = selection
            .read(&app, |selection, ctx| {
                assert!(selection.is_conversation_active(ctx));
                assert!(selection.is_conversation_fullscreen(ctx));
                selection.selected_conversation_id(ctx)
            })
            .expect("TUI /new should eagerly create its replacement conversation");
        assert_ne!(new_conversation_id, conversation_id);
        history.read(&app, |history, _| {
            assert_eq!(
                history
                    .active_conversation(terminal_surface_id)
                    .map(|conversation| conversation.id()),
                Some(new_conversation_id)
            );
        });
        assert_eq!(
            terminal_model.lock().block_list().active_conversation_id(),
            Some(new_conversation_id)
        );
        assert_eq!(
            terminal_model.lock().block_list().transcript_scope(),
            &TranscriptScope::Unfiltered
        );
    });
}

#[test]
fn tui_selection_creates_and_selects_terminal_surface_scoped_conversation() {
    App::test((), |mut app| async move {
        let (history, selection, terminal_surface_id, _) = build_tui_selection(&mut app);
        let initial_conversation_id = selection
            .read(&app, |selection, ctx| {
                selection.selected_conversation_id(ctx)
            })
            .expect("TUI should have an initial conversation");

        let conversation_id = selection
            .update(&mut app, |selection, ctx| {
                selection.try_start_new_conversation(AgentViewEntryOrigin::Cli, ctx)
            })
            .expect("TUI conversation creation should succeed");

        selection.read(&app, |selection, ctx| {
            assert_eq!(
                selection.selected_conversation_id(ctx),
                Some(conversation_id)
            );
        });
        history.read(&app, |history, _| {
            let conversation_ids = history
                .all_live_conversations_for_terminal_surface(terminal_surface_id)
                .map(|conversation| conversation.id())
                .collect::<Vec<_>>();
            assert!(conversation_ids.contains(&initial_conversation_id));
            assert!(conversation_ids.contains(&conversation_id));
        });
    });
}

#[test]
fn tui_selection_reconciles_split_and_removed_selection() {
    App::test((), |mut app| async move {
        let (history, selection, terminal_surface_id, _) = build_tui_selection(&mut app);
        let old_conversation_id = AIConversationId::new();
        let new_conversation_id = AIConversationId::new();

        selection.update(&mut app, |selection, ctx| {
            selection.select_existing_conversation(
                old_conversation_id,
                AgentViewEntryOrigin::Cli,
                ctx,
            );
        });
        history.update(&mut app, |_, ctx| {
            ctx.emit(BlocklistAIHistoryEvent::SplitConversation {
                terminal_surface_id,
                old_conversation_id,
                new_conversation_id,
            });
        });
        selection.read(&app, |selection, ctx| {
            assert_eq!(
                selection.selected_conversation_id(ctx),
                Some(new_conversation_id)
            );
        });

        // Removing the selected conversation clears it and schedules a replacement
        // via `ctx.spawn` (see `defer_replacement_conversation`). `ctx.spawn` runs
        // its future on the background executor and delivers the result back to the
        // main thread, so the replacement is applied on a later tick. Subscribe
        // first so we can await that replacement deterministically via its
        // `Activated` event, instead of polling a fixed number of times (which
        // races the background round-trip and made this test flaky).
        let (replacement_tx, replacement_rx) = futures::channel::oneshot::channel();
        let replacement_tx = std::cell::RefCell::new(Some(replacement_tx));
        app.update(|ctx| {
            ctx.subscribe_to_model(&selection, move |_, event, _| {
                if matches!(event, ConversationSelectionEvent::Activated { .. }) {
                    if let Some(tx) = replacement_tx.borrow_mut().take() {
                        let _ = tx.send(());
                    }
                }
            });
        });

        history.update(&mut app, |_, ctx| {
            ctx.emit(BlocklistAIHistoryEvent::RemoveConversation {
                terminal_surface_id,
                conversation_id: new_conversation_id,
                run_id: None,
            });
        });
        selection.read(&app, |selection, ctx| {
            assert_eq!(selection.selected_conversation_id(ctx), None);
        });

        replacement_rx
            .await
            .expect("removing the selected conversation should select a replacement");

        selection.read(&app, |selection, ctx| {
            assert!(selection.selected_conversation_id(ctx).is_some());
            assert!(selection.is_conversation_active(ctx));
            assert!(selection.is_conversation_fullscreen(ctx));
        });
    });
}

#[test]
fn tui_restoration_wins_over_deferred_replacement() {
    App::test((), |mut app| async move {
        let (history, selection, terminal_surface_id, _) = build_tui_selection(&mut app);
        let provisional_conversation_id = selection
            .read(&app, |selection, ctx| {
                selection.selected_conversation_id(ctx)
            })
            .expect("TUI should have a provisional conversation");
        let restored_conversation_id = AIConversationId::new();
        app.update(|ctx| {
            history.update(ctx, |_, ctx| {
                ctx.emit(
                    BlocklistAIHistoryEvent::ClearedConversationsForTerminalSurface {
                        terminal_surface_id,
                        active_conversation_id: Some(provisional_conversation_id),
                        cleared_conversation_ids: vec![provisional_conversation_id],
                    },
                );
            });
            selection.update(ctx, |selection, ctx| {
                selection.select_existing_conversation(
                    restored_conversation_id,
                    AgentViewEntryOrigin::RestoreExistingConversation,
                    ctx,
                );
            });
        });
        futures_lite::future::yield_now().await;

        selection.read(&app, |selection, ctx| {
            assert_eq!(
                selection.selected_conversation_id(ctx),
                Some(restored_conversation_id)
            );
        });
    });
}

#[test]
fn tui_new_conversation_preserves_pending_autoexecute_override() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|ctx| AppExecutionMode::new(ExecutionMode::App, false, ctx));
        let history = app.add_singleton_model(|_| BlocklistAIHistoryModel::default());
        let terminal_surface_id = EntityId::new();
        let terminal_model = tui_terminal_model();
        let selection = app.add_model(|ctx| {
            Box::new(TuiConversationSelection::new(
                terminal_surface_id,
                terminal_model,
                ctx,
            )) as Box<dyn ConversationSelection>
        });

        let conversation_id = selection
            .update(&mut app, |selection, ctx| {
                selection.try_start_new_conversation(AgentViewEntryOrigin::Cli, ctx)
            })
            .expect("TUI conversation creation should succeed");

        history.read(&app, |history, _| {
            assert!(history
                .conversation(&conversation_id)
                .expect("conversation should exist")
                .autoexecute_any_action());
        });
    });
}

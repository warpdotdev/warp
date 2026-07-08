use warp::tui_export::{
    AIConversationId, AgentViewEntryOrigin, BlocklistAIHistoryEvent, BlocklistAIHistoryModel,
    ConversationSelection, ConversationSelectionHandle,
};
use warp_core::execution_mode::{AppExecutionMode, ExecutionMode};
use warpui::{App, EntityId, ModelHandle};

use super::TuiConversationSelection;

fn build_tui_selection(
    app: &mut App,
) -> (
    ModelHandle<BlocklistAIHistoryModel>,
    ConversationSelectionHandle,
    EntityId,
) {
    app.add_singleton_model(|ctx| AppExecutionMode::new(ExecutionMode::App, false, ctx));
    let history = app.add_singleton_model(|_| BlocklistAIHistoryModel::default());
    let terminal_surface_id = EntityId::new();
    let selection = app.add_model(|ctx| {
        Box::new(TuiConversationSelection::new(terminal_surface_id, ctx))
            as Box<dyn ConversationSelection>
    });
    (history, selection, terminal_surface_id)
}

#[test]
fn tui_selection_owns_next_prompt_selection() {
    App::test((), |mut app| async move {
        let (_, selection, _) = build_tui_selection(&mut app);
        let conversation_id = AIConversationId::new();

        selection.update(&mut app, |selection, ctx| {
            selection.select_existing_conversation(conversation_id, AgentViewEntryOrigin::Cli, ctx);
        });
        selection.read(&app, |selection, ctx| {
            assert_eq!(
                selection.selected_conversation_id(ctx),
                Some(conversation_id)
            );
            assert!(selection.is_conversation_active(ctx));
            assert!(selection.is_conversation_fullscreen(ctx));
        });

        selection.update(&mut app, |selection, ctx| {
            selection.select_new_conversation(AgentViewEntryOrigin::Cli, ctx);
        });
        selection.read(&app, |selection, ctx| {
            assert_eq!(selection.selected_conversation_id(ctx), None);
            assert!(!selection.is_conversation_active(ctx));
            assert!(!selection.is_conversation_fullscreen(ctx));
        });
    });
}

#[test]
fn tui_selection_creates_and_selects_terminal_surface_scoped_conversation() {
    App::test((), |mut app| async move {
        let (history, selection, terminal_surface_id) = build_tui_selection(&mut app);

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
            assert_eq!(
                history
                    .all_live_conversations_for_terminal_surface(terminal_surface_id)
                    .map(|conversation| conversation.id())
                    .collect::<Vec<_>>(),
                vec![conversation_id]
            );
        });
    });
}

#[test]
fn tui_selection_reconciles_split_and_removed_selection() {
    App::test((), |mut app| async move {
        let (history, selection, terminal_surface_id) = build_tui_selection(&mut app);
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
    });
}

#[test]
fn tui_new_conversation_preserves_pending_autoexecute_override() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|ctx| AppExecutionMode::new(ExecutionMode::App, false, ctx));
        let history = app.add_singleton_model(|_| BlocklistAIHistoryModel::default());
        let terminal_surface_id = EntityId::new();
        let selection = app.add_model(|ctx| {
            Box::new(TuiConversationSelection::new(terminal_surface_id, ctx))
                as Box<dyn ConversationSelection>
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

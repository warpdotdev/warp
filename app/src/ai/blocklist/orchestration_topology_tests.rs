use super::*;
use crate::ai::blocklist::BlocklistAIHistoryModel;
use warpui::{App, EntityId};

#[test]
fn descendant_conversation_ids_in_spawn_order_flattens_nested_children_preorder() {
    App::test((), |mut app| async move {
        let terminal_view_id = EntityId::new();
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());

        let orchestrator_id = history_model.update(&mut app, |history_model, ctx| {
            history_model.start_new_conversation(terminal_view_id, false, false, false, ctx)
        });
        let child_a = history_model.update(&mut app, |history_model, ctx| {
            history_model.start_new_child_conversation(
                terminal_view_id,
                "oz-env-check".to_string(),
                orchestrator_id,
                None,
                ctx,
            )
        });
        let child_b = history_model.update(&mut app, |history_model, ctx| {
            history_model.start_new_child_conversation(
                terminal_view_id,
                "sibling-agent".to_string(),
                orchestrator_id,
                None,
                ctx,
            )
        });
        let grandchild_a1 = history_model.update(&mut app, |history_model, ctx| {
            history_model.start_new_child_conversation(
                terminal_view_id,
                "codex-child".to_string(),
                child_a,
                None,
                ctx,
            )
        });
        let grandchild_a2 = history_model.update(&mut app, |history_model, ctx| {
            history_model.start_new_child_conversation(
                terminal_view_id,
                "follow-up-child".to_string(),
                child_a,
                None,
                ctx,
            )
        });
        let grandchild_b1 = history_model.update(&mut app, |history_model, ctx| {
            history_model.start_new_child_conversation(
                terminal_view_id,
                "sibling-grandchild".to_string(),
                child_b,
                None,
                ctx,
            )
        });

        history_model.read(&app, |history_model, _| {
            assert_eq!(
                descendant_conversation_ids_in_spawn_order(history_model, orchestrator_id),
                vec![
                    child_a,
                    grandchild_a1,
                    grandchild_a2,
                    child_b,
                    grandchild_b1
                ],
            );
        });
    });
}

#[test]
fn descendant_conversation_ids_in_spawn_order_returns_empty_without_children() {
    App::test((), |mut app| async move {
        let terminal_view_id = EntityId::new();
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());

        let orchestrator_id = history_model.update(&mut app, |history_model, ctx| {
            history_model.start_new_conversation(terminal_view_id, false, false, false, ctx)
        });

        history_model.read(&app, |history_model, _| {
            assert!(
                descendant_conversation_ids_in_spawn_order(history_model, orchestrator_id)
                    .is_empty()
            );
        });
    });
}

use super::*;
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::blocklist::BlocklistAIHistoryModel;
use warpui::{App, EntityId};

fn set_credits(
    app: &mut App,
    history: &warpui::ModelHandle<BlocklistAIHistoryModel>,
    id: AIConversationId,
    credits: f32,
) {
    history.update(app, |history, _| {
        history
            .conversation_mut(&id)
            .expect("conversation must be loaded")
            .set_credits_spent_for_test(credits);
    });
}

fn spawn_child(
    app: &mut App,
    history: &warpui::ModelHandle<BlocklistAIHistoryModel>,
    name: &str,
    parent_id: AIConversationId,
    terminal_view_id: EntityId,
) -> AIConversationId {
    history.update(app, |history, ctx| {
        history.start_new_child_conversation(
            terminal_view_id,
            name.to_string(),
            parent_id,
            None,
            ctx,
        )
    })
}

#[test]
fn returns_none_when_orchestrator_has_no_descendants() {
    App::test((), |mut app| async move {
        let terminal_view_id = EntityId::new();
        let history = app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());

        let orchestrator_id = history.update(&mut app, |history, ctx| {
            history.start_new_conversation(terminal_view_id, false, false, false, ctx)
        });

        // Even if the orchestrator itself has spent credits, no descendants
        // means no rollup applies.
        set_credits(&mut app, &history, orchestrator_id, 10.0);

        history.read(&app, |history, _| {
            assert!(compute_orchestration_rollup(orchestrator_id, history).is_none());
        });
    });
}

#[test]
fn sums_orchestrator_and_loaded_descendants() {
    App::test((), |mut app| async move {
        let terminal_view_id = EntityId::new();
        let history = app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());

        let orchestrator_id = history.update(&mut app, |history, ctx| {
            history.start_new_conversation(terminal_view_id, false, false, false, ctx)
        });
        let child_id = spawn_child(
            &mut app,
            &history,
            "DesignBot",
            orchestrator_id,
            terminal_view_id,
        );

        set_credits(&mut app, &history, orchestrator_id, 3.0);
        set_credits(&mut app, &history, child_id, 30.0);

        history.read(&app, |history, _| {
            let rollup = compute_orchestration_rollup(orchestrator_id, history)
                .expect("rollup should be Some");
            assert_eq!(rollup.total_credits, 33.0);
            assert_eq!(rollup.per_agent.len(), 2);
            // Child spent more, sorted first.
            assert_eq!(rollup.per_agent[0].conversation_id, child_id);
            assert_eq!(rollup.per_agent[0].credits_spent, 30.0);
            assert_eq!(rollup.per_agent[0].avatar, AgentAvatar::Child);
            assert_eq!(rollup.per_agent[0].display_name, "DesignBot");
            assert_eq!(rollup.per_agent[1].conversation_id, orchestrator_id);
            assert_eq!(rollup.per_agent[1].credits_spent, 3.0);
            assert_eq!(rollup.per_agent[1].avatar, AgentAvatar::Orchestrator);
            assert_eq!(rollup.per_agent[1].display_name, "Orchestrator");
        });
    });
}

#[test]
fn excludes_zero_credit_descendants_from_breakdown() {
    App::test((), |mut app| async move {
        let terminal_view_id = EntityId::new();
        let history = app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());

        let orchestrator_id = history.update(&mut app, |history, ctx| {
            history.start_new_conversation(terminal_view_id, false, false, false, ctx)
        });
        let alpha_id = spawn_child(
            &mut app,
            &history,
            "Alpha",
            orchestrator_id,
            terminal_view_id,
        );
        let beta_id = spawn_child(
            &mut app,
            &history,
            "Beta",
            orchestrator_id,
            terminal_view_id,
        );
        let _idle_id = spawn_child(
            &mut app,
            &history,
            "IdleChild",
            orchestrator_id,
            terminal_view_id,
        );

        set_credits(&mut app, &history, orchestrator_id, 2.0);
        set_credits(&mut app, &history, alpha_id, 12.0);
        set_credits(&mut app, &history, beta_id, 5.0);

        history.read(&app, |history, _| {
            let rollup = compute_orchestration_rollup(orchestrator_id, history)
                .expect("rollup should be Some");
            assert_eq!(rollup.total_credits, 19.0);
            assert_eq!(rollup.per_agent.len(), 3);
            let ordered_ids: Vec<_> = rollup
                .per_agent
                .iter()
                .map(|entry| entry.conversation_id)
                .collect();
            assert_eq!(ordered_ids, vec![alpha_id, beta_id, orchestrator_id]);
        });
    });
}

#[test]
fn rolls_up_grandchildren_transitively() {
    App::test((), |mut app| async move {
        let terminal_view_id = EntityId::new();
        let history = app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());

        let orchestrator_id = history.update(&mut app, |history, ctx| {
            history.start_new_conversation(terminal_view_id, false, false, false, ctx)
        });
        let child_id = spawn_child(
            &mut app,
            &history,
            "ChildA",
            orchestrator_id,
            terminal_view_id,
        );
        let grandchild_id = spawn_child(&mut app, &history, "GrandA1", child_id, terminal_view_id);

        set_credits(&mut app, &history, orchestrator_id, 1.0);
        set_credits(&mut app, &history, child_id, 4.0);
        set_credits(&mut app, &history, grandchild_id, 9.0);

        history.read(&app, |history, _| {
            let rollup = compute_orchestration_rollup(orchestrator_id, history)
                .expect("rollup should be Some");
            assert_eq!(rollup.total_credits, 14.0);
            let ordered_ids: Vec<_> = rollup
                .per_agent
                .iter()
                .map(|entry| entry.conversation_id)
                .collect();
            assert_eq!(ordered_ids, vec![grandchild_id, child_id, orchestrator_id]);
        });
    });
}

#[test]
fn returns_six_contributors_for_show_n_more_caller() {
    App::test((), |mut app| async move {
        let terminal_view_id = EntityId::new();
        let history = app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());

        let orchestrator_id = history.update(&mut app, |history, ctx| {
            history.start_new_conversation(terminal_view_id, false, false, false, ctx)
        });
        set_credits(&mut app, &history, orchestrator_id, 1.0);

        for i in 0..5 {
            let id = spawn_child(
                &mut app,
                &history,
                &format!("Agent{i}"),
                orchestrator_id,
                terminal_view_id,
            );
            // Distinct credit values so we don't rely on tie-break behavior.
            set_credits(&mut app, &history, id, (10 + i) as f32);
        }

        history.read(&app, |history, _| {
            let rollup = compute_orchestration_rollup(orchestrator_id, history)
                .expect("rollup should be Some");
            assert_eq!(rollup.per_agent.len(), 6);
        });
    });
}

#[test]
fn returns_none_when_only_orchestrator_has_zero_credits_with_loaded_children() {
    App::test((), |mut app| async move {
        let terminal_view_id = EntityId::new();
        let history = app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());

        let orchestrator_id = history.update(&mut app, |history, ctx| {
            history.start_new_conversation(terminal_view_id, false, false, false, ctx)
        });
        // One spawned child, but neither it nor the orchestrator has spent
        // any credits yet.
        let _child_id = spawn_child(
            &mut app,
            &history,
            "Idle",
            orchestrator_id,
            terminal_view_id,
        );

        history.read(&app, |history, _| {
            assert!(compute_orchestration_rollup(orchestrator_id, history).is_none());
        });
    });
}

#[test]
fn ties_break_by_spawn_order_earlier_first() {
    App::test((), |mut app| async move {
        let terminal_view_id = EntityId::new();
        let history = app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());

        let orchestrator_id = history.update(&mut app, |history, ctx| {
            history.start_new_conversation(terminal_view_id, false, false, false, ctx)
        });
        let first_id = spawn_child(
            &mut app,
            &history,
            "FirstSpawned",
            orchestrator_id,
            terminal_view_id,
        );
        let second_id = spawn_child(
            &mut app,
            &history,
            "SecondSpawned",
            orchestrator_id,
            terminal_view_id,
        );

        // Equal credit values force a tie-break.
        set_credits(&mut app, &history, first_id, 7.0);
        set_credits(&mut app, &history, second_id, 7.0);

        history.read(&app, |history, _| {
            let rollup = compute_orchestration_rollup(orchestrator_id, history)
                .expect("rollup should be Some");
            assert_eq!(rollup.per_agent.len(), 2);
            assert_eq!(rollup.per_agent[0].conversation_id, first_id);
            assert_eq!(rollup.per_agent[1].conversation_id, second_id);
        });
    });
}

#[test]
fn unloaded_descendant_id_is_silently_skipped() {
    App::test((), |mut app| async move {
        let terminal_view_id = EntityId::new();
        let history = app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());

        let orchestrator_id = history.update(&mut app, |history, ctx| {
            history.start_new_conversation(terminal_view_id, false, false, false, ctx)
        });
        let real_child_id = spawn_child(
            &mut app,
            &history,
            "RealChild",
            orchestrator_id,
            terminal_view_id,
        );
        set_credits(&mut app, &history, real_child_id, 4.0);

        // Manually insert a dangling parent → child mapping for an ID that
        // is not present in `conversations_by_id`. This emulates an
        // orchestration topology entry where the child's `AIConversation`
        // hasn't been hydrated locally (e.g. remote-only child agent).
        let unloaded_id = AIConversationId::new();
        history.update(&mut app, |history, _| {
            history.set_parent_for_conversation(unloaded_id, orchestrator_id);
        });

        history.read(&app, |history, _| {
            let rollup = compute_orchestration_rollup(orchestrator_id, history)
                .expect("rollup should be Some");
            assert_eq!(rollup.total_credits, 4.0);
            assert_eq!(rollup.per_agent.len(), 1);
            assert_eq!(rollup.per_agent[0].conversation_id, real_child_id);
        });
    });
}

use warpui::{App, EntityId};

use super::*;
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::blocklist::BlocklistAIHistoryModel;
use crate::persistence::model::ChargedUsageTotals;
use crate::test_util::settings::initialize_history_persistence_for_tests;

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

fn set_cost_in_cents(
    app: &mut App,
    history: &warpui::ModelHandle<BlocklistAIHistoryModel>,
    id: AIConversationId,
    cost_in_cents: Option<f32>,
) {
    history.update(app, |history, _| {
        history
            .conversation_mut(&id)
            .expect("conversation must be loaded")
            .set_cost_in_cents_for_test(cost_in_cents);
    });
}

fn set_charged_usage_tokens(
    app: &mut App,
    history: &warpui::ModelHandle<BlocklistAIHistoryModel>,
    id: AIConversationId,
    total_tokens: Option<u32>,
) {
    history.update(app, |history, _| {
        history
            .conversation_mut(&id)
            .expect("conversation must be loaded")
            .set_charged_usage_for_test(total_tokens.map(|input_tokens| ChargedUsageTotals {
                input_tokens,
                ..Default::default()
            }));
    });
}

fn set_charged_usage(
    app: &mut App,
    history: &warpui::ModelHandle<BlocklistAIHistoryModel>,
    id: AIConversationId,
    cost_in_cents: f32,
    total_tokens: u32,
) {
    history.update(app, |history, _| {
        history
            .conversation_mut(&id)
            .expect("conversation must be loaded")
            .set_charged_usage_for_test(Some(ChargedUsageTotals {
                input_cost_in_cents: cost_in_cents,
                input_tokens: total_tokens,
                ..Default::default()
            }));
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
            false,
            ctx,
        )
    })
}

#[test]
fn returns_none_when_orchestrator_has_no_descendants() {
    App::test((), |mut app| async move {
        initialize_history_persistence_for_tests(&mut app);
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
        initialize_history_persistence_for_tests(&mut app);
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
fn sums_cost_in_cents_when_all_contributors_have_a_baseline() {
    App::test((), |mut app| async move {
        initialize_history_persistence_for_tests(&mut app);
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
        set_cost_in_cents(&mut app, &history, orchestrator_id, Some(6.0));
        set_cost_in_cents(&mut app, &history, child_id, Some(60.0));

        history.read(&app, |history, _| {
            let rollup = compute_orchestration_rollup(orchestrator_id, history)
                .expect("rollup should be Some");
            assert_eq!(rollup.total_cost_in_cents, Some(66.0));
        });
    });
}

#[test]
fn sums_charged_usage_cost_over_divergent_provider_cost() {
    App::test((), |mut app| async move {
        initialize_history_persistence_for_tests(&mut app);
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
        // Deliberately diverge each contributor's provider-only baseline
        // from its charged-usage total.
        set_cost_in_cents(&mut app, &history, orchestrator_id, Some(6.0));
        set_cost_in_cents(&mut app, &history, child_id, Some(60.0));
        set_charged_usage(&mut app, &history, orchestrator_id, 5.0, 0);
        set_charged_usage(&mut app, &history, child_id, 50.0, 0);

        history.read(&app, |history, _| {
            let rollup = compute_orchestration_rollup(orchestrator_id, history)
                .expect("rollup should be Some");
            assert_eq!(
                rollup.total_cost_in_cents,
                Some(55.0),
                "the rollup total must come from charged usage, not the divergent provider baseline"
            );
        });
    });
}

#[test]
fn omits_cost_in_cents_when_any_contributor_lacks_a_baseline() {
    App::test((), |mut app| async move {
        initialize_history_persistence_for_tests(&mut app);
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
        set_cost_in_cents(&mut app, &history, orchestrator_id, Some(6.0));
        // Child has no known cost baseline (e.g. a legacy conversation).
        set_cost_in_cents(&mut app, &history, child_id, None);

        history.read(&app, |history, _| {
            let rollup = compute_orchestration_rollup(orchestrator_id, history)
                .expect("rollup should be Some");
            assert_eq!(rollup.total_cost_in_cents, None);
        });
    });
}

#[test]
fn sums_tokens_when_all_contributors_have_a_count() {
    App::test((), |mut app| async move {
        initialize_history_persistence_for_tests(&mut app);
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
        set_charged_usage_tokens(&mut app, &history, orchestrator_id, Some(100));
        set_charged_usage_tokens(&mut app, &history, child_id, Some(900));

        history.read(&app, |history, _| {
            let rollup = compute_orchestration_rollup(orchestrator_id, history)
                .expect("rollup should be Some");
            assert_eq!(rollup.total_tokens, Some(1000));
        });
    });
}

#[test]
fn omits_tokens_when_any_contributor_lacks_a_count() {
    App::test((), |mut app| async move {
        initialize_history_persistence_for_tests(&mut app);
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
        set_charged_usage_tokens(&mut app, &history, orchestrator_id, Some(100));
        // Child has no known token count (e.g. flag off, or a legacy conversation).
        set_charged_usage_tokens(&mut app, &history, child_id, None);

        history.read(&app, |history, _| {
            let rollup = compute_orchestration_rollup(orchestrator_id, history)
                .expect("rollup should be Some");
            assert_eq!(rollup.total_tokens, None);
        });
    });
}

#[test]
fn zero_credit_descendant_does_not_poison_cost_or_token_totals() {
    App::test((), |mut app| async move {
        initialize_history_persistence_for_tests(&mut app);
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
        // Freshly spawned, has not reported any usage: zero credits, and no
        // known cost/token baseline (the defaults).
        let _idle_id = spawn_child(
            &mut app,
            &history,
            "IdleChild",
            orchestrator_id,
            terminal_view_id,
        );

        set_credits(&mut app, &history, orchestrator_id, 3.0);
        set_credits(&mut app, &history, child_id, 30.0);
        // Charged usage carries cost and tokens together, mirroring the
        // real wire shape (unlike the provider-only baseline, which never
        // carries a token count).
        set_charged_usage(&mut app, &history, orchestrator_id, 6.0, 100);
        set_charged_usage(&mut app, &history, child_id, 60.0, 900);

        history.read(&app, |history, _| {
            let rollup = compute_orchestration_rollup(orchestrator_id, history)
                .expect("rollup should be Some");
            assert_eq!(
                rollup.total_cost_in_cents,
                Some(66.0),
                "the idle child's unset cost baseline must not force the total to None"
            );
            assert_eq!(
                rollup.total_tokens,
                Some(1000),
                "the idle child's unset token count must not force the total to None"
            );
        });
    });
}

#[test]
fn excludes_zero_credit_descendants_from_breakdown() {
    App::test((), |mut app| async move {
        initialize_history_persistence_for_tests(&mut app);
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
        initialize_history_persistence_for_tests(&mut app);
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
        initialize_history_persistence_for_tests(&mut app);
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
        initialize_history_persistence_for_tests(&mut app);
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
        initialize_history_persistence_for_tests(&mut app);
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
        initialize_history_persistence_for_tests(&mut app);
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

use ai::agent::action_result::RequestCommandOutputResult;
use ai::hooks::test_util::{ScriptedRunner, test_binding, test_source_id};
use ai::hooks::{DeclaredBinding, HookBoundary};
use warpui::{App, ModelHandle, SingletonEntity};

use super::*;
use crate::ai::agent::{AIAgentActionId, AIAgentActionResult};
use crate::ai::blocklist::hooks::testing::{
    action_model, hook_test_app, init_project_action, read_nothing_action, settle, shell_action,
};
use crate::ai::blocklist::{BlocklistAIActionModel, BlocklistAIHistoryModel};
use crate::features::FeatureFlag;
use crate::terminal::model::block::BlockId;

fn observing_bindings() -> Vec<DeclaredBinding> {
    let source = test_source_id("observer");
    vec![
        test_binding(source.clone(), HookBoundary::ToolResult, 0, None),
        test_binding(source, HookBoundary::ToolFailure, 0, None),
    ]
}

fn observed(app: &App) -> Vec<ObservedTerminalResult> {
    app.read(observed_terminal_results)
}

fn has_result(
    model: &ModelHandle<BlocklistAIActionModel>,
    action_id: &AIAgentActionId,
    app: &mut App,
) -> bool {
    model.read(app, |model, _| model.get_action_result(action_id).is_some())
}

#[test]
fn a_synchronous_completion_is_observed_once_as_a_result() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
        let harness = hook_test_app(&mut app);
        app.update(|ctx| harness.hooks.install_bindings(observing_bindings(), ctx));
        let model = action_model(&mut app, &harness);
        let action = init_project_action("init");

        model.update(&mut app, |model, ctx| {
            model.queue_actions(vec![action.clone()], harness.conversation_id, ctx)
        });
        settle(&mut app, |app| has_result(&model, &action.id, app)).await;

        assert_eq!(
            observed(&app),
            vec![ObservedTerminalResult {
                action_id: action.id.clone(),
                boundary: HookBoundary::ToolResult,
            }]
        );
        settle(&mut app, |_| harness.hooks.adapter.encoded_count() == 1).await;
    });
}

#[test]
fn an_asynchronous_completion_is_observed_once_as_a_result() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
        let harness = hook_test_app(&mut app);
        app.update(|ctx| harness.hooks.install_bindings(observing_bindings(), ctx));
        let model = action_model(&mut app, &harness);
        let action = read_nothing_action("read");

        model.update(&mut app, |model, ctx| {
            model.queue_actions(vec![action.clone()], harness.conversation_id, ctx)
        });
        settle(&mut app, |app| has_result(&model, &action.id, app)).await;

        assert_eq!(
            observed(&app),
            vec![ObservedTerminalResult {
                action_id: action.id.clone(),
                boundary: HookBoundary::ToolResult,
            }]
        );
    });
}

#[test]
fn a_hook_denial_is_observed_once_as_a_failure() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
        let harness = hook_test_app(&mut app);
        let deny = test_binding(
            test_source_id("policy"),
            HookBoundary::ToolPre,
            0,
            Some("Shell"),
        );
        harness
            .hooks
            .runner
            .script(deny.id.clone(), ScriptedRunner::deny_with("not today"));
        let mut bindings = observing_bindings();
        bindings.push(deny);
        app.update(|ctx| harness.hooks.install_bindings(bindings, ctx));
        let model = action_model(&mut app, &harness);
        let action = shell_action("shell", "ls");

        model.update(&mut app, |model, ctx| {
            model.queue_actions(vec![action.clone()], harness.conversation_id, ctx)
        });
        settle(&mut app, |app| has_result(&model, &action.id, app)).await;

        let result = model.read(&app, |model, _| {
            model.get_action_result(&action.id).cloned()
        });
        assert!(matches!(
            result.as_deref().map(|result| &result.result),
            Some(AIAgentActionResultType::RequestCommandOutput(
                RequestCommandOutputResult::HookDenied { .. }
            ))
        ));
        assert_eq!(
            observed(&app),
            vec![ObservedTerminalResult {
                action_id: action.id.clone(),
                boundary: HookBoundary::ToolFailure,
            }]
        );
    });
}

#[test]
fn cancelling_a_running_action_is_observed_once_as_a_failure() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
        let harness = hook_test_app(&mut app);
        app.update(|ctx| harness.hooks.install_bindings(observing_bindings(), ctx));
        let model = action_model(&mut app, &harness);
        let action = shell_action("shell", "sleep 100");

        model.update(&mut app, |model, ctx| {
            model.queue_actions(vec![action.clone()], harness.conversation_id, ctx)
        });
        settle(&mut app, |app| {
            model.read(app, |model, _| {
                model
                    .get_action_status(&action.id)
                    .is_some_and(|s| s.is_blocked())
            })
        })
        .await;
        model.update(&mut app, |model, ctx| {
            model.execute_action(&action.id, harness.conversation_id, ctx)
        });
        assert!(model.read(&app, |model, _| {
            model
                .get_action_status(&action.id)
                .is_some_and(|s| s.is_running())
        }));

        model.update(&mut app, |model, ctx| {
            model.cancel_action_with_id(
                harness.conversation_id,
                &action.id,
                CancellationReason::ManuallyCancelled,
                ctx,
            )
        });
        settle(&mut app, |app| has_result(&model, &action.id, app)).await;

        assert_eq!(
            observed(&app),
            vec![ObservedTerminalResult {
                action_id: action.id.clone(),
                boundary: HookBoundary::ToolFailure,
            }]
        );
    });
}

#[test]
fn cancelling_a_pending_action_is_observed_once_as_a_failure() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
        let harness = hook_test_app(&mut app);
        app.update(|ctx| harness.hooks.install_bindings(observing_bindings(), ctx));
        let model = action_model(&mut app, &harness);
        let action = shell_action("shell", "ls");

        model.update(&mut app, |model, ctx| {
            model.queue_actions(vec![action.clone()], harness.conversation_id, ctx)
        });
        settle(&mut app, |app| {
            model.read(app, |model, _| {
                model
                    .get_action_status(&action.id)
                    .is_some_and(|s| s.is_blocked())
            })
        })
        .await;

        model.update(&mut app, |model, ctx| {
            model.cancel_action_with_id(
                harness.conversation_id,
                &action.id,
                CancellationReason::ManuallyCancelled,
                ctx,
            )
        });
        assert!(has_result(&model, &action.id, &mut app));
        assert_eq!(
            observed(&app),
            vec![ObservedTerminalResult {
                action_id: action.id.clone(),
                boundary: HookBoundary::ToolFailure,
            }]
        );
    });
}

#[test]
fn a_shared_session_viewer_does_not_observe_mirrored_results() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
        let harness = hook_test_app(&mut app);
        app.update(|ctx| harness.hooks.install_bindings(observing_bindings(), ctx));
        let viewer_conversation_id = app.update(|ctx| {
            BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                history.start_new_conversation(harness.terminal_view_id, false, true, false, ctx)
            })
        });
        let model = action_model(&mut app, &harness);
        model.update(&mut app, |model, _| model.set_view_only(true));
        let first = shell_action("first", "sleep 100");
        let second = shell_action("second", "ls");

        model.update(&mut app, |model, ctx| {
            model.queue_actions(
                vec![first.clone(), second.clone()],
                viewer_conversation_id,
                ctx,
            )
        });
        settle(&mut app, |app| {
            model.read(app, |model, _| {
                model
                    .get_pending_actions_for_conversation(&viewer_conversation_id)
                    .count()
                    == 2
            })
        })
        .await;

        // The sharer's snapshot for the first command arrives, which drains the second one.
        model.update(&mut app, |model, ctx| {
            model.apply_finished_action_result(
                viewer_conversation_id,
                AIAgentActionResult {
                    id: first.id.clone(),
                    task_id: first.task_id.clone(),
                    result: AIAgentActionResultType::RequestCommandOutput(
                        RequestCommandOutputResult::LongRunningCommandSnapshot {
                            block_id: BlockId::new(),
                            command: "sleep 100".to_owned(),
                            grid_contents: String::new(),
                            cursor: String::new(),
                            is_alt_screen_active: false,
                            activity: None,
                        },
                    ),
                },
                ctx,
            )
        });
        assert!(has_result(&model, &first.id, &mut app));
        assert!(has_result(&model, &second.id, &mut app));
        assert!(observed(&app).is_empty());
    });
}

#[test]
fn nothing_is_observed_when_no_binding_is_active() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
        let harness = hook_test_app(&mut app);
        let model = action_model(&mut app, &harness);
        let action = init_project_action("init");
        model.update(&mut app, |model, ctx| {
            model.queue_actions(vec![action.clone()], harness.conversation_id, ctx)
        });
        settle(&mut app, |app| has_result(&model, &action.id, app)).await;
        assert!(observed(&app).is_empty());
    });
}

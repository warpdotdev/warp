use std::cell::{Cell, RefCell};
use std::rc::Rc;

use ai::agent::orchestration_config::{
    OrchestrationConfig, OrchestrationConfigStatus, OrchestrationExecutionMode,
};
use warp::tui_export::{
    AIActionStatus, AIAgentAction, AIAgentActionId, AIAgentActionType, Appearance,
    AuthSecretSelection, OptionRow, OptionSnapshot, OptionSourceStatus, OrchestrationConfigState,
    OrchestrationEditState, RunAgentsAgentRunConfig, RunAgentsExecutionMode, RunAgentsRequest,
    TaskId,
};
use warpui::platform::WindowStyle;
use warpui::{AddWindowOptions, App, ViewHandle};
use warpui_core::TypedActionView as _;

use super::{
    build_request, CardMode, ConfigPage, OrchestrationBlockController, TuiOrchestrationBlock,
    TuiOrchestrationBlockAction, TuiOrchestrationBlockEvent,
};
use crate::option_selector::{TuiOptionSelectorAction, TuiOptionSelectorEvent};
use crate::test_fixtures::TestHostView;

/// Builds a request with the given harness and execution mode.
fn request(harness: &str, execution_mode: RunAgentsExecutionMode) -> RunAgentsRequest {
    RunAgentsRequest {
        summary: "Parallelize the task.".to_string(),
        base_prompt: "base".to_string(),
        skills: Vec::new(),
        model_id: "auto".to_string(),
        harness_type: harness.to_string(),
        execution_mode,
        agent_run_configs: vec![RunAgentsAgentRunConfig {
            name: "researcher".to_string(),
            prompt: "research".to_string(),
            title: "Researcher".to_string(),
        }],
        plan_id: "plan-1".to_string(),
        harness_auth_secret_name: None,
    }
}

#[test]
fn only_the_model_page_is_searchable() {
    assert!(ConfigPage::Model.is_searchable());
    for page in [
        ConfigPage::Location,
        ConfigPage::Harness,
        ConfigPage::ApiKey,
        ConfigPage::Host,
        ConfigPage::Environment,
    ] {
        assert!(!page.is_searchable(), "{page:?}");
    }
}

/// A Cloud execution mode with the given env/host.
fn remote(environment_id: &str, worker_host: &str) -> RunAgentsExecutionMode {
    RunAgentsExecutionMode::Remote {
        environment_id: environment_id.to_string(),
        worker_host: worker_host.to_string(),
        computer_use_enabled: true,
    }
}

#[test]
fn local_collapses_the_page_sequence_to_two_pages() {
    let state = TuiOrchestrationBlock::config_state_from_request(
        &request("oz", RunAgentsExecutionMode::Local),
        None,
    );
    assert_eq!(
        TuiOrchestrationBlock::page_sequence(&state),
        vec![ConfigPage::Location, ConfigPage::Model],
    );
}

#[test]
fn cloud_oz_uses_five_pages_without_the_api_key_page() {
    let state = TuiOrchestrationBlock::config_state_from_request(
        &request("oz", remote("env-1", "warp")),
        None,
    );
    assert_eq!(
        TuiOrchestrationBlock::page_sequence(&state),
        vec![
            ConfigPage::Location,
            ConfigPage::Harness,
            ConfigPage::Host,
            ConfigPage::Environment,
            ConfigPage::Model,
        ],
    );
}

#[test]
fn cloud_managed_credential_harness_inserts_the_api_key_page() {
    let state = TuiOrchestrationBlock::config_state_from_request(
        &request("claude", remote("env-1", "warp")),
        None,
    );
    assert_eq!(
        TuiOrchestrationBlock::page_sequence(&state),
        vec![
            ConfigPage::Location,
            ConfigPage::Harness,
            ConfigPage::ApiKey,
            ConfigPage::Host,
            ConfigPage::Environment,
            ConfigPage::Model,
        ],
    );
}

#[test]
fn edit_state_carries_the_request_auth_secret() {
    let mut with_secret = request("claude", remote("env-1", "warp"));
    with_secret.harness_auth_secret_name = Some("work-key".to_string());
    let state = TuiOrchestrationBlock::config_state_from_request(&with_secret, None);
    assert_eq!(
        state.auth_secret_selection,
        AuthSecretSelection::Named("work-key".to_string()),
    );
    // Absence means "no choice yet", not Inherit.
    let state =
        TuiOrchestrationBlock::config_state_from_request(&request("claude", remote("", "")), None);
    assert_eq!(state.auth_secret_selection, AuthSecretSelection::Unset);
}

#[test]
fn edit_state_is_overridden_by_an_approved_config() {
    let incoming = request("oz", RunAgentsExecutionMode::Local);
    let config = OrchestrationConfig {
        model_id: "sonnet".to_string(),
        harness_type: "claude".to_string(),
        execution_mode: OrchestrationExecutionMode::Remote {
            environment_id: "env-2".to_string(),
            worker_host: "warp".to_string(),
        },
    };
    let state = TuiOrchestrationBlock::config_state_from_request(
        &incoming,
        Some(&(config.clone(), OrchestrationConfigStatus::Approved)),
    );
    assert_eq!(state.harness_type, "claude");
    assert_eq!(state.model_id, "sonnet");
    assert!(state.execution_mode.is_remote());

    // A disapproved config does not override the request.
    let state = TuiOrchestrationBlock::config_state_from_request(
        &incoming,
        Some(&(config, OrchestrationConfigStatus::Disapproved)),
    );
    assert_eq!(state.harness_type, "oz");
    assert_eq!(state.model_id, "auto");
    assert!(!state.execution_mode.is_remote());
}

#[test]
fn unapproved_local_request_forces_oz_harness() {
    let state = TuiOrchestrationBlock::config_state_from_request(
        &request("claude", RunAgentsExecutionMode::Local),
        None,
    );

    assert_eq!(state.harness_type, "oz");
    assert_eq!(state.model_id, "");
}

#[test]
fn build_request_carries_card_fields_and_edited_run_wide_state() {
    let original = request("oz", remote("env-1", "warp"));
    let mut state = TuiOrchestrationBlock::config_state_from_request(&original, None);
    state.model_id = "gpt-5".to_string();
    state.harness_type = "codex".to_string();
    state.set_environment_id("env-9".to_string());
    state.set_worker_host("self-hosted".to_string());
    state.auth_secret_selection = AuthSecretSelection::Named("codex-key".to_string());

    let built = build_request(&original, &state);
    // Card fields pass through unchanged.
    assert_eq!(built.summary, original.summary);
    assert_eq!(built.base_prompt, original.base_prompt);
    assert_eq!(built.agent_run_configs, original.agent_run_configs);
    assert_eq!(built.plan_id, original.plan_id);
    // Run-wide fields come from the edited state; the per-call
    // computer-use flag is preserved through the round trip.
    assert_eq!(built.model_id, "gpt-5");
    assert_eq!(built.harness_type, "codex");
    assert_eq!(
        built.execution_mode,
        RunAgentsExecutionMode::Remote {
            environment_id: "env-9".to_string(),
            worker_host: "self-hosted".to_string(),
            computer_use_enabled: true,
        },
    );
    assert_eq!(built.harness_auth_secret_name.as_deref(), Some("codex-key"));
}

#[test]
fn build_request_omits_the_auth_secret_when_the_picker_is_not_applicable() {
    // A stale Named(_) selection must not leak into a Local dispatch.
    let original = request("claude", RunAgentsExecutionMode::Local);
    let mut state = TuiOrchestrationBlock::config_state_from_request(&original, None);
    state.auth_secret_selection = AuthSecretSelection::Named("stale".to_string());
    assert_eq!(
        build_request(&original, &state).harness_auth_secret_name,
        None
    );
}

#[derive(Default)]
struct TestController {
    executed_requests: RefCell<Vec<RunAgentsRequest>>,
    accept_error: RefCell<Option<String>>,
}

impl OrchestrationBlockController for TestController {
    fn action_status(
        &self,
        _action_id: &AIAgentActionId,
        _ctx: &warpui::AppContext,
    ) -> Option<AIActionStatus> {
        Some(AIActionStatus::Blocked)
    }

    fn snapshot_for_page(
        &self,
        page: ConfigPage,
        state: &OrchestrationConfigState,
        _ctx: &warpui::AppContext,
    ) -> OptionSnapshot {
        let (rows, selected_id) = match page {
            ConfigPage::Location => (
                vec![row("cloud", "Cloud"), row("local", "Local")],
                if state.execution_mode.is_remote() {
                    "cloud"
                } else {
                    "local"
                },
            ),
            ConfigPage::Harness => (vec![row("oz", "Warp")], "oz"),
            ConfigPage::ApiKey => (vec![row("", "Skip")], ""),
            ConfigPage::Host => (vec![row("warp", "Warp")], "warp"),
            ConfigPage::Environment => (vec![row("", "Empty environment")], ""),
            ConfigPage::Model => (vec![row("auto", "Auto")], "auto"),
        };
        OptionSnapshot {
            rows,
            selected_id: Some(selected_id.to_string()),
            status: OptionSourceStatus::Ready,
            footer: None,
        }
    }

    fn apply_page_selection(
        &self,
        page: ConfigPage,
        id: &str,
        edit_state: &mut OrchestrationEditState,
        _fallback_base_model_id: Option<String>,
        _ctx: &mut warpui::AppContext,
    ) {
        let state = &mut edit_state.orchestration_config_state;
        match page {
            ConfigPage::Location => state.toggle_execution_mode_to_remote(id == "cloud"),
            ConfigPage::Harness => state.harness_type = id.to_string(),
            ConfigPage::ApiKey => {
                state.auth_secret_selection = if id.is_empty() {
                    AuthSecretSelection::Inherit
                } else {
                    AuthSecretSelection::Named(id.to_string())
                };
            }
            ConfigPage::Host => state.set_worker_host(id.to_string()),
            ConfigPage::Environment => state.set_environment_id(id.to_string()),
            ConfigPage::Model => state.model_id = id.to_string(),
        }
    }

    fn accept(
        &self,
        _action_id: &AIAgentActionId,
        request: RunAgentsRequest,
        _state: &OrchestrationConfigState,
        _ctx: &mut warpui::AppContext,
    ) -> Result<(), String> {
        if let Some(reason) = self.accept_error.borrow().clone() {
            return Err(reason);
        }
        self.executed_requests.borrow_mut().push(request);
        Ok(())
    }
}

/// Builds an enabled test option row.
fn row(id: &str, label: &str) -> OptionRow {
    OptionRow {
        id: id.to_string(),
        label: label.to_string(),
        harness: None,
        badge: None,
        disabled_reason: None,
    }
}

/// Constructs an interactive block with the local fake controller.
fn test_block(
    app: &mut App,
    request: &RunAgentsRequest,
) -> (ViewHandle<TuiOrchestrationBlock>, Rc<TestController>) {
    app.add_singleton_model(|_| Appearance::mock());
    let action = AIAgentAction {
        id: AIAgentActionId::from("run-agents-1".to_string()),
        task_id: TaskId::new("task-1".to_string()),
        action: AIAgentActionType::RunAgents(request.clone()),
        requires_result: true,
    };
    let controller = Rc::new(TestController::default());
    let controller_for_view: Rc<dyn OrchestrationBlockController> = controller.clone();
    let request = request.clone();
    let view = app.update(|ctx| {
        let (window_id, _) = ctx.add_tui_window(
            AddWindowOptions {
                window_style: WindowStyle::NotStealFocus,
                ..Default::default()
            },
            |_| TestHostView,
        );
        ctx.add_typed_action_tui_view(window_id, move |ctx| {
            TuiOrchestrationBlock::from_parts(
                action,
                &request,
                None,
                controller_for_view,
                Some("auto".to_string()),
                false,
                Vec::new(),
                ctx,
            )
        })
    });
    (view, controller)
}

/// Dispatches a typed action directly to the test block.
fn act(
    app: &mut App,
    block: &ViewHandle<TuiOrchestrationBlock>,
    action: TuiOrchestrationBlockAction,
) {
    block.update(app, |block, ctx| block.handle_action(&action, ctx));
}

#[test]
fn selector_layout_invalidations_are_forwarded() {
    App::test((), |mut app| async move {
        let (block, _) = test_block(&mut app, &request("oz", RunAgentsExecutionMode::Local));
        let events = Rc::new(RefCell::new(Vec::new()));
        let captured_events = events.clone();
        app.update(|ctx| {
            ctx.subscribe_to_view(&block, move |_, event, _| {
                captured_events.borrow_mut().push(event.clone());
            });
        });

        block.update(&mut app, |block, ctx| {
            block.handle_selector_event(&TuiOptionSelectorEvent::LayoutInvalidated, ctx);
        });

        assert!(events
            .borrow()
            .iter()
            .any(|event| matches!(event, TuiOrchestrationBlockEvent::LayoutInvalidated)));
    });
}

#[test]
fn selector_actions_commit_edits_and_follow_the_dynamic_page_sequence() {
    App::test((), |mut app| async move {
        let (block, _) = test_block(&mut app, &request("oz", remote("env-1", "warp")));
        act(&mut app, &block, TuiOrchestrationBlockAction::Configure);
        let selector = app.read(|ctx| block.as_ref(ctx).selector.clone());

        selector.update(&mut app, |selector, ctx| {
            selector.handle_action(&TuiOptionSelectorAction::MoveDown, ctx);
        });
        act(
            &mut app,
            &block,
            TuiOrchestrationBlockAction::CommitAndPreviousPage,
        );
        app.read(|ctx| {
            let block = block.as_ref(ctx);
            assert!(!block
                .orchestration_edit_state
                .orchestration_config_state
                .execution_mode
                .is_remote());
            assert_eq!(
                block.mode,
                CardMode::Configuring {
                    page: ConfigPage::Location
                }
            );
        });

        selector.update(&mut app, |selector, ctx| {
            selector.handle_action(&TuiOptionSelectorAction::MoveUp, ctx);
        });
        act(
            &mut app,
            &block,
            TuiOrchestrationBlockAction::CommitAndNextPage,
        );
        app.read(|ctx| {
            let block = block.as_ref(ctx);
            assert!(block
                .orchestration_edit_state
                .orchestration_config_state
                .execution_mode
                .is_remote());
            assert_eq!(
                block.mode,
                CardMode::Configuring {
                    page: ConfigPage::Harness
                }
            );
        });
    });
}

#[test]
fn blocked_accept_invalidates_card_layout() {
    App::test((), |mut app| async move {
        let (block, controller) =
            test_block(&mut app, &request("oz", RunAgentsExecutionMode::Local));
        *controller.accept_error.borrow_mut() = Some("Choose a model.".to_string());
        let invalidations = Rc::new(Cell::new(0));
        let invalidations_for_subscription = invalidations.clone();
        app.update(|ctx| {
            ctx.subscribe_to_view(&block, move |_, event, _| match event {
                TuiOrchestrationBlockEvent::BlockingStateChanged => {
                    invalidations_for_subscription.set(invalidations_for_subscription.get() + 1);
                }
                TuiOrchestrationBlockEvent::RejectRequested => {}
                TuiOrchestrationBlockEvent::LayoutInvalidated => {}
            });
        });

        act(&mut app, &block, TuiOrchestrationBlockAction::Accept);

        assert_eq!(invalidations.get(), 1);
        assert_eq!(
            block.read(&app, |block, _| block.accept_error.clone()),
            Some("Choose a model.".to_string())
        );
    });
}

#[test]
fn failed_arrow_confirmation_does_not_change_later_enter_navigation() {
    App::test((), |mut app| async move {
        let (block, _) = test_block(&mut app, &request("oz", RunAgentsExecutionMode::Local));
        act(&mut app, &block, TuiOrchestrationBlockAction::Configure);
        let selector = app.read(|ctx| block.as_ref(ctx).selector.clone());

        let mut disabled_local = row("local", "Local");
        disabled_local.disabled_reason = Some("Unavailable".to_string());
        selector.update(&mut app, |selector, ctx| {
            selector.refresh_snapshot(
                OptionSnapshot {
                    rows: vec![disabled_local],
                    selected_id: Some("local".to_string()),
                    status: OptionSourceStatus::Ready,
                    footer: None,
                },
                ctx,
            );
        });
        act(
            &mut app,
            &block,
            TuiOrchestrationBlockAction::CommitAndPreviousPage,
        );

        selector.update(&mut app, |selector, ctx| {
            selector.refresh_snapshot(
                OptionSnapshot {
                    rows: vec![row("local", "Local")],
                    selected_id: Some("local".to_string()),
                    status: OptionSourceStatus::Ready,
                    footer: None,
                },
                ctx,
            );
            selector.handle_action(&TuiOptionSelectorAction::ConfirmSelected, ctx);
        });

        assert_eq!(
            block.read(&app, |block, _| block.mode),
            CardMode::Configuring {
                page: ConfigPage::Model
            }
        );
    });
}
#[test]
fn confirming_a_search_result_returns_focus_to_the_acceptance_card() {
    App::test((), |mut app| async move {
        let (block, _) = test_block(&mut app, &request("oz", RunAgentsExecutionMode::Local));
        block.update(&mut app, |block, ctx| {
            block.open_page(ConfigPage::Model, ctx);
        });
        let window_id = app.read(|ctx| block.window_id(ctx));
        let selector = app.read(|ctx| block.as_ref(ctx).selector.clone());
        selector.update(&mut app, |selector, ctx| {
            selector.handle_action(&TuiOptionSelectorAction::FocusSearchAndInsert('a'), ctx);
        });
        assert_ne!(app.focused_view_id(window_id), Some(block.id()));

        selector.update(&mut app, |selector, ctx| {
            selector.handle_action(&TuiOptionSelectorAction::SelectItem(0), ctx);
        });

        assert_eq!(
            block.read(&app, |block, _| block.mode),
            CardMode::Acceptance
        );
        assert_eq!(app.focused_view_id(window_id), Some(block.id()));
    });
}
#[test]
fn accepting_dispatches_once_and_releases_focus() {
    App::test((), |mut app| async move {
        let request = request("oz", RunAgentsExecutionMode::Local);
        let (block, controller) = test_block(&mut app, &request);
        assert!(app.read(|ctx| block.as_ref(ctx).is_awaiting_confirmation(ctx)));

        act(&mut app, &block, TuiOrchestrationBlockAction::Accept);
        act(&mut app, &block, TuiOrchestrationBlockAction::Accept);
        act(&mut app, &block, TuiOrchestrationBlockAction::Reject);

        assert_eq!(controller.executed_requests.borrow().as_slice(), &[request]);
        assert!(app.read(|ctx| !block.as_ref(ctx).is_awaiting_confirmation(ctx)));
    });
}

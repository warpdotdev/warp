use std::path::PathBuf;

use ai::agent::action::{RunAgentsAgentRunConfig, RunAgentsExecutionMode, RunAgentsRequest};
use ai::agent::action_result::{
    RunAgentsAgentOutcome, RunAgentsAgentOutcomeKind, RunAgentsLaunchedExecutionMode,
    RunAgentsResult,
};
use ai::skills::SkillReference;
use warp_util::local_or_remote_path::LocalOrRemotePath;

use super::{RunAgentsCardFields, RunAgentsEditState};
use crate::ai::blocklist::inline_action::orchestration_controls::OrchestrationConfigState;

fn make_request(harness: &str, mode: RunAgentsExecutionMode) -> RunAgentsRequest {
    make_request_with_skills(harness, mode, Vec::new())
}

fn make_request_with_skills(
    harness: &str,
    mode: RunAgentsExecutionMode,
    skills: Vec<SkillReference>,
) -> RunAgentsRequest {
    RunAgentsRequest {
        summary: "summary".to_string(),
        base_prompt: "base".to_string(),
        skills,
        model_id: "auto".to_string(),
        harness_type: harness.to_string(),
        execution_mode: mode,
        agent_run_configs: vec![RunAgentsAgentRunConfig {
            name: "child".to_string(),
            prompt: "do work".to_string(),
            title: "Child agent".to_string(),
            agent_identity_uid: String::new(),
            model_id: String::new(),
        }],
        plan_id: String::new(),
        harness_auth_secret_name: None,
    }
}

fn make_config_state_with_orch_fields(
    harness: &str,
    mode: RunAgentsExecutionMode,
) -> RunAgentsEditState {
    let request = make_request(harness, mode);
    RunAgentsEditState {
        orchestration_config_state: OrchestrationConfigState::from_run_agents_fields(
            Some(&request.model_id),
            Some(&request.harness_type),
            &request.execution_mode,
        ),
        card: RunAgentsCardFields {
            agent_run_configs: request.agent_run_configs,
            base_prompt: request.base_prompt,
            summary: request.summary,
            skills: request.skills,
            plan_id: request.plan_id,
        },
    }
}

#[test]
fn local_to_cloud_initializes_remote_with_empty_environment() {
    let mut state =
        RunAgentsEditState::from_request(&make_request("oz", RunAgentsExecutionMode::Local));
    assert!(matches!(
        state.orchestration_config_state.execution_mode,
        RunAgentsExecutionMode::Local
    ));

    state
        .orchestration_config_state
        .toggle_execution_mode_to_remote(true);
    let RunAgentsExecutionMode::Remote {
        environment_id,
        worker_host,
        computer_use_enabled,
        ..
    } = state.orchestration_config_state.execution_mode
    else {
        panic!("expected Remote after toggle");
    };
    assert_eq!(environment_id, "");
    assert_eq!(worker_host, "warp");
    assert!(!computer_use_enabled);
}

#[test]
fn cloud_to_local_drops_environment() {
    let mut state = RunAgentsEditState::from_request(&make_request(
        "oz",
        RunAgentsExecutionMode::Remote {
            environment_id: "env-1".to_string(),
            worker_host: "warp".to_string(),
            computer_use_enabled: false,
            runner_id: String::new(),
        },
    ));
    state
        .orchestration_config_state
        .toggle_execution_mode_to_remote(false);
    assert!(matches!(
        state.orchestration_config_state.execution_mode,
        RunAgentsExecutionMode::Local
    ));
}

#[test]
fn local_to_cloud_resets_opencode_to_oz() {
    let mut state =
        RunAgentsEditState::from_request(&make_request("opencode", RunAgentsExecutionMode::Local));
    state
        .orchestration_config_state
        .toggle_execution_mode_to_remote(true);
    assert_eq!(state.orchestration_config_state.harness_type, "oz");
}

#[test]
fn cloud_without_env_no_longer_disables_accept() {
    let state = RunAgentsEditState::from_request(&make_request(
        "oz",
        RunAgentsExecutionMode::Remote {
            environment_id: String::new(),
            worker_host: "warp".to_string(),
            computer_use_enabled: false,
            runner_id: String::new(),
        },
    ));
    assert!(
        state
            .orchestration_config_state
            .accept_disabled_reason()
            .is_none(),
        "Cloud without env should NOT disable Accept (soft recommendation only)"
    );
}

#[test]
fn cloud_with_opencode_disables_accept() {
    // Bypass the toggle helper to test the validation gate directly.
    let state = RunAgentsEditState::from_request(&make_request(
        "opencode",
        RunAgentsExecutionMode::Remote {
            environment_id: "env-1".to_string(),
            worker_host: "warp".to_string(),
            computer_use_enabled: false,
            runner_id: String::new(),
        },
    ));
    let reason = state.orchestration_config_state.accept_disabled_reason();
    assert!(reason.is_some(), "Cloud + OpenCode should disable Accept");
    assert!(reason.unwrap().contains("OpenCode"));
}

#[test]
fn local_with_any_harness_does_not_disable_accept() {
    for harness in ["oz", "claude", "gemini", "opencode"] {
        let state =
            RunAgentsEditState::from_request(&make_request(harness, RunAgentsExecutionMode::Local));
        assert!(
            state
                .orchestration_config_state
                .accept_disabled_reason()
                .is_none(),
            "Local + {harness} should allow Accept"
        );
    }
}

#[test]
fn local_with_disabled_codex_disables_accept() {
    let state = make_config_state_with_orch_fields("codex", RunAgentsExecutionMode::Local);
    assert_eq!(
        state.orchestration_config_state.accept_disabled_reason(),
        Some("Local Codex child agents are temporarily disabled.")
    );
}

#[test]
fn from_request_sanitizes_disabled_local_harness_to_oz() {
    let state =
        RunAgentsEditState::from_request(&make_request("codex", RunAgentsExecutionMode::Local));

    assert_eq!(state.orchestration_config_state.harness_type, "oz");
    assert_eq!(state.orchestration_config_state.model_id, "");
    assert!(
        state
            .orchestration_config_state
            .accept_disabled_reason()
            .is_none()
    );
}

#[test]
fn cloud_with_env_and_non_opencode_harness_allows_accept() {
    for harness in ["oz", "claude", "gemini"] {
        let state = RunAgentsEditState::from_request(&make_request(
            harness,
            RunAgentsExecutionMode::Remote {
                environment_id: "env-1".to_string(),
                worker_host: "warp".to_string(),
                computer_use_enabled: false,
                runner_id: String::new(),
            },
        ));
        assert!(
            state
                .orchestration_config_state
                .accept_disabled_reason()
                .is_none(),
            "Cloud + env + {harness} should allow Accept"
        );
    }
}

#[test]
fn set_environment_id_no_op_in_local_mode() {
    let mut state =
        RunAgentsEditState::from_request(&make_request("oz", RunAgentsExecutionMode::Local));
    state
        .orchestration_config_state
        .set_environment_id("env-1".to_string());
    assert!(matches!(
        state.orchestration_config_state.execution_mode,
        RunAgentsExecutionMode::Local
    ));
}

#[test]
fn set_environment_id_updates_remote() {
    let mut state = RunAgentsEditState::from_request(&make_request(
        "oz",
        RunAgentsExecutionMode::Remote {
            environment_id: "old".to_string(),
            worker_host: "warp".to_string(),
            computer_use_enabled: false,
            runner_id: String::new(),
        },
    ));
    state
        .orchestration_config_state
        .set_environment_id("new-env".to_string());
    let RunAgentsExecutionMode::Remote { environment_id, .. } =
        state.orchestration_config_state.execution_mode
    else {
        panic!("expected Remote");
    };
    assert_eq!(environment_id, "new-env");
}

#[test]
fn set_runner_id_updates_remote_and_round_trips() {
    let mut state = RunAgentsEditState::from_request(&make_request(
        "oz",
        RunAgentsExecutionMode::Remote {
            environment_id: "env-1".to_string(),
            worker_host: "warp".to_string(),
            computer_use_enabled: false,
            runner_id: String::new(),
        },
    ));
    state
        .orchestration_config_state
        .set_runner_id("runner-9".to_string());
    let RunAgentsExecutionMode::Remote { runner_id, .. } =
        &state.orchestration_config_state.execution_mode
    else {
        panic!("expected Remote");
    };
    assert_eq!(runner_id, "runner-9");
    // The runner flows back out through to_request unchanged.
    assert_eq!(
        state.to_request().execution_mode,
        RunAgentsExecutionMode::Remote {
            environment_id: "env-1".to_string(),
            worker_host: "warp".to_string(),
            computer_use_enabled: false,
            runner_id: "runner-9".to_string(),
        }
    );
}

#[test]
fn set_runner_id_no_op_in_local_mode() {
    let mut state =
        RunAgentsEditState::from_request(&make_request("oz", RunAgentsExecutionMode::Local));
    state
        .orchestration_config_state
        .set_runner_id("runner-1".to_string());
    assert!(matches!(
        state.orchestration_config_state.execution_mode,
        RunAgentsExecutionMode::Local
    ));
}

#[test]
fn to_request_round_trips_request_fields() {
    let mut req = make_request_with_skills(
        "claude",
        RunAgentsExecutionMode::Remote {
            environment_id: "env-2".to_string(),
            worker_host: "warp".to_string(),
            computer_use_enabled: true,
            runner_id: String::new(),
        },
        vec![
            SkillReference::BundledSkillId("writing-pr-descriptions".to_string()),
            SkillReference::Path(LocalOrRemotePath::Local(PathBuf::from(
                "/tmp/skill/SKILL.md",
            ))),
        ],
    );
    req.plan_id = "plan-1".to_string();
    let state = RunAgentsEditState::from_request(&req);
    let round_tripped = state.to_request();
    assert_eq!(round_tripped.summary, req.summary);
    assert_eq!(round_tripped.base_prompt, req.base_prompt);
    assert_eq!(round_tripped.model_id, req.model_id);
    assert_eq!(round_tripped.harness_type, req.harness_type);
    assert_eq!(round_tripped.execution_mode, req.execution_mode);
    assert_eq!(round_tripped.agent_run_configs, req.agent_run_configs);
    assert_eq!(round_tripped.skills, req.skills);
    assert_eq!(round_tripped.plan_id, req.plan_id);
}

mod resolve_pending_card_state_tests {
    use super::super::{PendingCardState, resolve_pending_card_state};

    #[test]
    fn blocked_status_renders_confirmation_regardless_of_owner_cancellation() {
        assert_eq!(
            resolve_pending_card_state(true, false, false),
            PendingCardState::Confirmation
        );
        assert_eq!(
            resolve_pending_card_state(true, false, true),
            PendingCardState::Confirmation
        );
    }

    #[test]
    fn no_status_while_owner_not_cancelled_renders_configuring() {
        // The action hasn't reached the action model yet, and the owner
        // block has not been explicitly cancelled: show the transient
        // "Configuring agents..." placeholder, not a cancelled card.
        assert_eq!(
            resolve_pending_card_state(false, true, false),
            PendingCardState::Configuring
        );
    }

    #[test]
    fn no_status_and_owner_cancelled_renders_cancelled_mid_stream() {
        // Mid-tool-call cancellation: the action never made it into
        // BlocklistAIActionModel, so status stays None forever, and the
        // owner block reached the explicit `Cancelled` status. This must
        // resolve to the cancelled presentation instead of getting stuck
        // Configuring.
        assert_eq!(
            resolve_pending_card_state(false, true, true),
            PendingCardState::CancelledMidStream
        );
    }

    #[test]
    fn no_status_and_owner_merely_complete_renders_configuring_not_cancelled() {
        // Regression guard: on the happy path, `mark_response_stream_completed_successfully`
        // flips the owner block to `Complete` (not `Cancelled`) before the
        // action is queued and preprocessed. `get_action_status` is `None`
        // in that legitimate pre-queue/pre-preprocessing window, but the
        // owner was never cancelled, so this must NOT resolve to
        // `CancelledMidStream` (a real bug would flash a false-cancelled
        // card on every successful run_agents call).
        assert_eq!(
            resolve_pending_card_state(false, true, false),
            PendingCardState::Configuring
        );
    }

    #[test]
    fn non_blocked_status_while_owner_not_cancelled_renders_configuring() {
        // e.g. Preprocessing/Queued with the owner not cancelled.
        assert_eq!(
            resolve_pending_card_state(false, false, false),
            PendingCardState::Configuring
        );
    }

    #[test]
    fn non_blocked_status_takes_precedence_over_owner_cancellation() {
        // A concrete (non-`Blocked`) action status takes precedence over
        // the mid-stream-cancellation heuristic, which only applies when
        // there is no action status at all.
        assert_eq!(
            resolve_pending_card_state(false, false, true),
            PendingCardState::Configuring
        );
    }
}

/// End-to-end tests that construct a real `RunAgentsCardView` backed by a
/// real `BlocklistAIActionModel` and a mutable `FakeAIBlockModel`, so the
/// invalidation latch (`owner_was_cancelled`) and the live `render()` /
/// `update_request()` wiring are exercised, not just the pure
/// `resolve_pending_card_state` helper.
mod live_cancellation_tests {
    use std::rc::Rc;
    use std::sync::Arc;

    use ai::agent::action::{RunAgentsAgentRunConfig, RunAgentsExecutionMode, RunAgentsRequest};
    use async_channel::unbounded;
    use parking_lot::FairMutex;
    use warpui::platform::WindowStyle;
    use warpui::{App, EntityId, ModelHandle, ViewHandle};

    use super::super::{PendingCardState, RunAgentsCardView};
    use crate::ai::agent::AIAgentActionId;
    use crate::ai::blocklist::FakeAIBlockModel;
    use crate::ai::blocklist::action_model::BlocklistAIActionModel;
    use crate::ai::blocklist::block::AIBlock;
    use crate::ai::blocklist::block::model::AIBlockModel;
    use crate::ai::get_relevant_files::controller::GetRelevantFilesController;
    use crate::server::experiments::ServerExperiments;
    use crate::terminal::model::session::Sessions;
    use crate::terminal::model::session::active_session::ActiveSession;
    use crate::terminal::model::terminal_model::TerminalModel;
    use crate::terminal::model_events::ModelEventDispatcher;
    use crate::test_util::terminal::initialize_app_for_terminal_view;

    fn make_request() -> RunAgentsRequest {
        RunAgentsRequest {
            summary: "summary".to_string(),
            base_prompt: "base".to_string(),
            skills: Vec::new(),
            model_id: "auto".to_string(),
            harness_type: "oz".to_string(),
            execution_mode: RunAgentsExecutionMode::Local,
            agent_run_configs: vec![RunAgentsAgentRunConfig {
                name: "child".to_string(),
                prompt: "do work".to_string(),
                title: "Child agent".to_string(),
                agent_identity_uid: String::new(),
                model_id: String::new(),
            }],
            plan_id: String::new(),
            harness_auth_secret_name: None,
        }
    }

    /// Registers every singleton `initialize_app_for_terminal_view` sets up
    /// (covering the pickers, harness/model catalogs, and self-hosted
    /// workers that `RunAgentsCardView::new` touches) plus `ServerExperiments`
    /// (which that helper does not register), then builds a real
    /// `BlocklistAIActionModel` so `get_action_status` behaves exactly as it
    /// does live.
    fn initialize(app: &mut App) -> ModelHandle<BlocklistAIActionModel> {
        initialize_app_for_terminal_view(app);
        app.add_singleton_model(|ctx| ServerExperiments::new_from_cache(vec![], ctx));

        let terminal_view_id = EntityId::new();
        let terminal_model = Arc::new(FairMutex::new(TerminalModel::mock(None, None)));
        let sessions = app.add_model(|_| Sessions::new_for_test());
        let (_model_events_tx, model_events_rx) = unbounded();
        let model_event_dispatcher =
            app.add_model(|ctx| ModelEventDispatcher::new(model_events_rx, sessions.clone(), ctx));
        let active_session = app.add_model(|ctx| {
            ActiveSession::new(sessions.clone(), model_event_dispatcher.clone(), ctx)
        });
        let get_relevant_files_controller = app.add_model(GetRelevantFilesController::new);

        app.add_model(|ctx| {
            BlocklistAIActionModel::new(
                terminal_model,
                active_session,
                &model_event_dispatcher,
                get_relevant_files_controller,
                terminal_view_id,
                ctx,
            )
        })
    }

    /// Builds a `RunAgentsCardView` as a standalone window-rooted view, wired
    /// to `action_model` and `block_model`, with no action ever queued (so
    /// `get_action_status` is `None` for the lifetime of the test unless the
    /// test explicitly queues one).
    fn build_card(
        app: &mut App,
        action_model: ModelHandle<BlocklistAIActionModel>,
        block_model: Rc<dyn AIBlockModel<View = AIBlock>>,
    ) -> ViewHandle<RunAgentsCardView> {
        let action_id = AIAgentActionId::from("run-agents-test-action".to_string());
        let request = make_request();
        let run_agents_executor =
            app.read(|ctx| action_model.as_ref(ctx).run_agents_executor(ctx).clone());
        let (_window_id, view) = app.add_window(WindowStyle::NotStealFocus, move |ctx| {
            RunAgentsCardView::new(
                action_id,
                &request,
                None,
                action_model,
                run_agents_executor,
                block_model,
                ctx,
            )
        });
        view
    }

    #[test]
    fn cancelled_mid_stream_after_owner_cancels_without_request_change() {
        App::test((), |mut app| async move {
            let action_model = initialize(&mut app);
            let fake_model = Rc::new(FakeAIBlockModel::new_streaming(vec![]));
            let block_model: Rc<dyn AIBlockModel<View = AIBlock>> = fake_model.clone();
            let view = build_card(&mut app, action_model, block_model);

            // While streaming with no action status yet, the card must show
            // the transient placeholder, not a cancelled card.
            view.read(&app, |view, ctx| {
                assert_eq!(
                    view.pending_card_state_for_test(ctx),
                    PendingCardState::Configuring
                );
                assert!(!view.owner_was_cancelled_for_test());
            });

            // Cancel the owner block WITHOUT changing the streamed
            // RunAgentsRequest, then drive the exact update path
            // `AIBlock::ensure_run_agents_card_view` uses on every chunk
            // (including the last one delivered at cancellation).
            fake_model.cancel(None);
            let unchanged_request = make_request();
            view.update(&mut app, |view, ctx| {
                view.update_request(&unchanged_request, ctx);
            });

            view.read(&app, |view, ctx| {
                assert!(
                    view.owner_was_cancelled_for_test(),
                    "the invalidation latch must flip once the owner is cancelled, \
                     even though the request content did not change"
                );
                assert_eq!(
                    view.pending_card_state_for_test(ctx),
                    PendingCardState::CancelledMidStream,
                    "the card must reflect the cancellation live, without needing a \
                     changed request or a Finished/Blocked action status"
                );
            });
        });
    }

    #[test]
    fn successful_completion_pre_queue_gap_does_not_render_cancelled() {
        App::test((), |mut app| async move {
            let action_model = initialize(&mut app);
            let fake_model = Rc::new(FakeAIBlockModel::new_streaming(vec![]));
            let block_model: Rc<dyn AIBlockModel<View = AIBlock>> = fake_model.clone();
            let view = build_card(&mut app, action_model, block_model);

            // Simulate `mark_response_stream_completed_successfully`: the
            // owner flips to `Complete` (not `Cancelled`) before
            // `AfterStreamFinished` queues the action and preprocessing runs.
            fake_model.complete(crate::ai::agent::AIAgentOutput::default());
            let unchanged_request = make_request();
            view.update(&mut app, |view, ctx| {
                view.update_request(&unchanged_request, ctx);
            });

            view.read(&app, |view, ctx| {
                assert!(
                    !view.owner_was_cancelled_for_test(),
                    "a successful Complete transition must not trip the cancellation latch"
                );
                assert_eq!(
                    view.pending_card_state_for_test(ctx),
                    PendingCardState::Configuring,
                    "must not flash a false-cancelled card during the happy path's \
                     pre-queue/pre-preprocessing gap"
                );
            });
        });
    }

    #[test]
    fn constructed_after_owner_already_cancelled_is_not_stuck() {
        App::test((), |mut app| async move {
            let action_model = initialize(&mut app);
            let fake_model = Rc::new(FakeAIBlockModel::new_streaming(vec![]));
            // Cancel before the card is ever constructed.
            fake_model.cancel(None);
            let block_model: Rc<dyn AIBlockModel<View = AIBlock>> = fake_model.clone();
            let view = build_card(&mut app, action_model, block_model);

            view.read(&app, |view, ctx| {
                assert!(
                    view.owner_was_cancelled_for_test(),
                    "the latch must be seeded true at construction time, not stuck false, \
                     when the owner is already cancelled before the card exists"
                );
                assert_eq!(
                    view.pending_card_state_for_test(ctx),
                    PendingCardState::CancelledMidStream
                );
            });
        });
    }
}

mod format_terminal_state_tests {
    use super::super::{StatusKind, format_terminal_state};
    use super::*;

    fn launched(name: &str, agent_id: &str) -> RunAgentsAgentOutcome {
        RunAgentsAgentOutcome {
            name: name.to_string(),
            resolved_model_id: String::new(),
            kind: RunAgentsAgentOutcomeKind::Launched {
                agent_id: agent_id.to_string(),
            },
        }
    }

    fn failed(name: &str, error: &str) -> RunAgentsAgentOutcome {
        RunAgentsAgentOutcome {
            name: name.to_string(),
            resolved_model_id: String::new(),
            kind: RunAgentsAgentOutcomeKind::Failed {
                error: error.to_string(),
            },
        }
    }

    fn launched_result(agents: Vec<RunAgentsAgentOutcome>) -> RunAgentsResult {
        RunAgentsResult::Launched {
            model_id: "auto".to_string(),
            harness_type: "oz".to_string(),
            execution_mode: RunAgentsLaunchedExecutionMode::Local,
            agents,
        }
    }

    #[test]
    fn launched_singular_uses_singular_label() {
        let result = launched_result(vec![launched("child", "a-1")]);
        let (label, kind) = format_terminal_state(&result);
        assert_eq!(label, "Spawned 1 agent");
        assert!(matches!(kind, StatusKind::Success));
    }

    #[test]
    fn launched_plural_uses_plural_label() {
        let result = launched_result(vec![
            launched("a", "a-1"),
            launched("b", "a-2"),
            launched("c", "a-3"),
        ]);
        let (label, kind) = format_terminal_state(&result);
        assert_eq!(label, "Spawned 3 agents");
        assert!(matches!(kind, StatusKind::Success));
    }

    #[test]
    fn launched_partial_uses_x_of_y_label_and_mixed_status() {
        let result = launched_result(vec![
            launched("a", "a-1"),
            failed("b", "boom"),
            launched("c", "a-3"),
        ]);
        let (label, kind) = format_terminal_state(&result);
        assert_eq!(label, "Spawned 2 of 3 agents");
        assert!(matches!(kind, StatusKind::Mixed));
    }

    #[test]
    fn all_failed_uses_failure_status_not_mixed() {
        let result = launched_result(vec![
            failed("a", "boom"),
            failed("b", "boom"),
            failed("c", "boom"),
        ]);
        let (label, kind) = format_terminal_state(&result);
        assert_eq!(label, "Failed to spawn 3 agents");
        assert!(matches!(kind, StatusKind::Failure));
    }

    #[test]
    fn single_failed_uses_singular_failure_label() {
        let result = launched_result(vec![failed("a", "boom")]);
        let (label, kind) = format_terminal_state(&result);
        assert_eq!(label, "Failed to spawn agent");
        assert!(matches!(kind, StatusKind::Failure));
    }

    #[test]
    fn failure_with_error_includes_error_text() {
        let (label, kind) = format_terminal_state(&RunAgentsResult::Failure {
            error: "server rejected request".to_string(),
        });
        assert_eq!(
            label,
            "Failed to start orchestration: server rejected request"
        );
        assert!(matches!(kind, StatusKind::Failure));
    }

    #[test]
    fn failure_with_empty_error_uses_short_label() {
        let (label, kind) = format_terminal_state(&RunAgentsResult::Failure {
            error: String::new(),
        });
        assert_eq!(label, "Failed to start orchestration");
        assert!(matches!(kind, StatusKind::Failure));
    }

    #[test]
    fn denied_with_reason_appends_reason() {
        let (label, kind) = format_terminal_state(&RunAgentsResult::Denied {
            reason: "disapproved".to_string(),
        });
        assert!(label.contains("disapproved"));
        assert!(matches!(kind, StatusKind::Cancelled));
    }

    #[test]
    fn denied_without_reason_uses_short_label() {
        let (label, kind) = format_terminal_state(&RunAgentsResult::Denied {
            reason: String::new(),
        });
        assert!(!label.contains("()"));
        assert!(matches!(kind, StatusKind::Cancelled));
    }

    #[test]
    fn cancelled_uses_cancelled_status() {
        let (label, kind) = format_terminal_state(&RunAgentsResult::Cancelled);
        assert_eq!(label, "Spawn agents cancelled");
        assert!(matches!(kind, StatusKind::Cancelled));
    }
}

mod override_from_approved_config_tests {
    use ai::agent::orchestration_config::{OrchestrationConfig, OrchestrationExecutionMode};

    use super::super::RunAgentsEditState;
    use super::*;

    fn local_config(model: &str, harness: &str) -> OrchestrationConfig {
        OrchestrationConfig {
            model_id: model.to_string(),
            harness_type: harness.to_string(),
            execution_mode: OrchestrationExecutionMode::Local,
        }
    }

    fn remote_config(model: &str, harness: &str, env: &str) -> OrchestrationConfig {
        OrchestrationConfig {
            model_id: model.to_string(),
            harness_type: harness.to_string(),
            execution_mode: OrchestrationExecutionMode::Remote {
                environment_id: env.to_string(),
                worker_host: "warp".to_string(),
                runner_id: String::new(),
            },
        }
    }

    #[test]
    fn overrides_model_and_harness_unconditionally() {
        let mut state =
            RunAgentsEditState::from_request(&make_request("oz", RunAgentsExecutionMode::Local));
        assert_eq!(state.orchestration_config_state.model_id, "auto");
        assert_eq!(state.orchestration_config_state.harness_type, "oz");

        state
            .orchestration_config_state
            .override_from_approved_config(&local_config("claude-4-opus", "claude"));
        assert_eq!(state.orchestration_config_state.model_id, "claude-4-opus");
        assert_eq!(state.orchestration_config_state.harness_type, "claude");
    }

    #[test]
    fn overrides_even_when_request_has_values() {
        let mut state = RunAgentsEditState::from_request(&make_request(
            "claude",
            RunAgentsExecutionMode::Local,
        ));
        state
            .orchestration_config_state
            .override_from_approved_config(&local_config("gpt-5", "codex"));
        assert_eq!(state.orchestration_config_state.model_id, "gpt-5");
        assert_eq!(state.orchestration_config_state.harness_type, "codex");
    }

    #[test]
    fn overrides_local_to_remote() {
        let mut state =
            RunAgentsEditState::from_request(&make_request("oz", RunAgentsExecutionMode::Local));
        state
            .orchestration_config_state
            .override_from_approved_config(&remote_config("auto", "oz", "env-1"));
        let RunAgentsExecutionMode::Remote {
            environment_id,
            worker_host,
            ..
        } = &state.orchestration_config_state.execution_mode
        else {
            panic!("expected Remote after override");
        };
        assert_eq!(environment_id, "env-1");
        assert_eq!(worker_host, "warp");
    }

    #[test]
    fn overrides_remote_to_local() {
        let mut state = RunAgentsEditState::from_request(&make_request(
            "oz",
            RunAgentsExecutionMode::Remote {
                environment_id: "env-1".to_string(),
                worker_host: "warp".to_string(),
                computer_use_enabled: true,
                runner_id: String::new(),
            },
        ));
        state
            .orchestration_config_state
            .override_from_approved_config(&local_config("auto", "oz"));
        assert!(
            matches!(
                state.orchestration_config_state.execution_mode,
                RunAgentsExecutionMode::Local
            ),
            "should be Local after override"
        );
    }

    #[test]
    fn preserves_computer_use_when_both_remote() {
        let mut state = RunAgentsEditState::from_request(&make_request(
            "oz",
            RunAgentsExecutionMode::Remote {
                environment_id: "old-env".to_string(),
                worker_host: "warp".to_string(),
                computer_use_enabled: true,
                runner_id: String::new(),
            },
        ));
        state
            .orchestration_config_state
            .override_from_approved_config(&remote_config("auto", "oz", "new-env"));
        let RunAgentsExecutionMode::Remote {
            environment_id,
            computer_use_enabled,
            ..
        } = &state.orchestration_config_state.execution_mode
        else {
            panic!("expected Remote");
        };
        assert_eq!(environment_id, "new-env", "env should come from config");
        assert!(
            *computer_use_enabled,
            "computer_use_enabled should be preserved from original request"
        );
    }

    #[test]
    fn does_not_carry_computer_use_from_local_to_remote() {
        let mut state =
            RunAgentsEditState::from_request(&make_request("oz", RunAgentsExecutionMode::Local));
        state
            .orchestration_config_state
            .override_from_approved_config(&remote_config("auto", "oz", "env-1"));
        let RunAgentsExecutionMode::Remote {
            computer_use_enabled,
            ..
        } = &state.orchestration_config_state.execution_mode
        else {
            panic!("expected Remote");
        };
        assert!(
            !*computer_use_enabled,
            "computer_use_enabled should default to false when original was Local"
        );
    }

    #[test]
    fn approved_local_disabled_harness_reports_disabled_reason_after_override() {
        let mut state =
            RunAgentsEditState::from_request(&make_request("oz", RunAgentsExecutionMode::Local));
        state
            .orchestration_config_state
            .override_from_approved_config(&local_config("auto", "codex"));
        assert_eq!(
            state.orchestration_config_state.accept_disabled_reason(),
            Some("Local Codex child agents are temporarily disabled.")
        );
    }
}

#[test]
fn local_to_cloud_idempotent_when_already_remote() {
    let mut state = RunAgentsEditState::from_request(&make_request(
        "oz",
        RunAgentsExecutionMode::Remote {
            environment_id: "env-1".to_string(),
            worker_host: "warp".to_string(),
            computer_use_enabled: true,
            runner_id: String::new(),
        },
    ));
    state
        .orchestration_config_state
        .toggle_execution_mode_to_remote(true);
    let RunAgentsExecutionMode::Remote {
        environment_id,
        computer_use_enabled,
        ..
    } = state.orchestration_config_state.execution_mode
    else {
        panic!("expected Remote");
    };
    assert_eq!(
        environment_id, "env-1",
        "toggle to Remote when already Remote should not clobber env"
    );
    assert!(
        computer_use_enabled,
        "toggle to Remote when already Remote should not clobber computer_use"
    );
}

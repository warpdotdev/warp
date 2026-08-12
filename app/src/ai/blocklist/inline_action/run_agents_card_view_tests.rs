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

mod pending_confirmation_status_tests {
    use super::super::{StatusKind, pending_confirmation_status};

    // Regression test for the bug where cancelling the conversation while a
    // `run_agents` tool call is still streaming (i.e. before the action is
    // ever queued into the action model) left the card spinning on
    // "Configuring agents..." forever, with no cancelled state ever shown.
    #[test]
    fn block_cancelled_while_awaiting_confirmation_renders_cancelled() {
        let (label, kind) = pending_confirmation_status(true);
        assert_eq!(label, "Spawn agents cancelled");
        assert!(matches!(kind, StatusKind::Cancelled));
    }

    #[test]
    fn block_not_cancelled_while_awaiting_confirmation_renders_spawning_placeholder() {
        let (label, kind) = pending_confirmation_status(false);
        assert_eq!(label, "Configuring agents\u{2026}");
        assert!(matches!(kind, StatusKind::Spawning));
    }
}

/// End-to-end regression test for the same bug, driven through a real
/// `RunAgentsCardView` instead of the pure `pending_confirmation_status`
/// helper: it constructs the actual view against a real (unqueued)
/// `BlocklistAIActionModel`, flips a fake block model from streaming to
/// `Cancelled`, and asserts the card's own `render()` output changes
/// accordingly. This also exercises `RunAgentsCardView::render`'s
/// `self.block_model.status(app).is_cancelled()` fallback branch directly,
/// rather than just the extracted decision function.
mod cancelled_while_streaming_tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use ai::agent::action::RunAgentsExecutionMode;
    use warpui::platform::WindowStyle;
    use warpui::{App, AppContext, View, ViewContext};

    use super::super::RunAgentsCardView;
    use super::make_request;
    use crate::ai::agent::conversation::AIConversationId;
    use crate::ai::agent::{AIAgentActionId, AIAgentInput, CancellationReason, ServerOutputId};
    use crate::ai::blocklist::action_model::RunAgentsExecutor;
    use crate::ai::blocklist::block::AIBlock;
    use crate::ai::blocklist::block::model::{
        AIBlockModel, AIBlockOutputStatus, AIRequestType, OutputStatusUpdateCallback,
    };
    use crate::ai::llms::LLMId;
    use crate::server::experiments::ServerExperiments;
    use crate::test_util::terminal::{add_window_with_terminal, initialize_app_for_terminal_view};

    /// A block model whose cancelled status can be flipped after
    /// construction, to simulate the enclosing conversation being cancelled
    /// while the `run_agents` tool call is still streaming.
    struct FakeStreamingBlockModel {
        status: RefCell<AIBlockOutputStatus>,
        model_id: LLMId,
    }

    impl FakeStreamingBlockModel {
        fn new() -> Rc<Self> {
            Rc::new(Self {
                status: RefCell::new(AIBlockOutputStatus::Pending),
                model_id: "fake-llm".to_string().into(),
            })
        }

        fn cancel(&self) {
            *self.status.borrow_mut() = AIBlockOutputStatus::Cancelled {
                partial_output: None,
                reason: CancellationReason::ManuallyCancelled,
            };
        }
    }

    impl AIBlockModel for FakeStreamingBlockModel {
        type View = AIBlock;

        fn status(&self, _app: &AppContext) -> AIBlockOutputStatus {
            self.status.borrow().clone()
        }

        fn server_output_id(&self, _app: &AppContext) -> Option<ServerOutputId> {
            None
        }

        fn model_id(&self, _app: &AppContext) -> Option<LLMId> {
            None
        }

        fn base_model<'a>(&'a self, _app: &'a AppContext) -> Option<&'a LLMId> {
            Some(&self.model_id)
        }

        fn inputs_to_render<'a>(&'a self, _app: &'a AppContext) -> &'a [AIAgentInput] {
            &[]
        }

        fn conversation_id(&self, _app: &AppContext) -> Option<AIConversationId> {
            None
        }

        fn on_updated_output(
            &self,
            _callback: OutputStatusUpdateCallback<AIBlock>,
            _ctx: &mut ViewContext<AIBlock>,
        ) {
        }

        fn request_type(&self, _app: &AppContext) -> AIRequestType {
            AIRequestType::Active
        }
    }

    #[test]
    fn block_cancelled_with_no_action_status_renders_cancelled_fallback() {
        App::test((), |mut app| async move {
            initialize_app_for_terminal_view(&mut app);
            app.add_singleton_model(|ctx| ServerExperiments::new_from_cache(vec![], ctx));
            let terminal_view = add_window_with_terminal(&mut app, None);
            let (action_model, run_agents_executor) = app.read(|ctx| {
                let action_model = terminal_view.as_ref(ctx).ai_action_model().clone();
                let run_agents_executor: warpui::ModelHandle<RunAgentsExecutor> =
                    action_model.as_ref(ctx).run_agents_executor(ctx);
                (action_model, run_agents_executor)
            });

            let block_model = FakeStreamingBlockModel::new();
            let block_model_for_view: Rc<dyn AIBlockModel<View = AIBlock>> = block_model.clone();
            let action_id = AIAgentActionId::from("run-agents-1".to_string());
            let request = make_request("oz", RunAgentsExecutionMode::Local);
            let (_window_id, card) = app.add_window(WindowStyle::NotStealFocus, move |ctx| {
                RunAgentsCardView::new(
                    action_id,
                    &request,
                    None,
                    action_model,
                    run_agents_executor,
                    block_model_for_view,
                    ctx,
                )
            });

            // The action was never queued (still streaming, per the real bug
            // scenario), so the card should show the streaming placeholder.
            let before = card.read(&app, |card, ctx| {
                card.render(ctx).debug_text_content().unwrap_or_default()
            });
            assert!(
                before.contains("Configuring agents"),
                "expected the still-streaming placeholder before cancellation: {before}"
            );

            // Cancel the block: the action still never gets queued, but the
            // card must now render the cancelled fallback instead of
            // spinning on the placeholder forever.
            block_model.cancel();

            let after = card.read(&app, |card, ctx| {
                card.render(ctx).debug_text_content().unwrap_or_default()
            });
            assert!(
                after.contains("Spawn agents cancelled"),
                "expected the cancelled fallback once the block is cancelled: {after}"
            );
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

use std::collections::HashMap;

use ai::agent::action::{RunAgentsAgentRunConfig, RunAgentsExecutionMode, RunAgentsRequest};
use ai::agent::orchestration_config::{
    OrchestrationConfig, OrchestrationConfigStatus, OrchestrationExecutionMode,
};
use settings::Setting;
use warp_core::execution_mode::ExecutionMode;
use warp_core::features::FeatureFlag;
use warpui::{App, Entity, EntityId, ModelHandle};

use super::*;
use crate::ai::active_agent_views_model::ActiveAgentViewsModel;
use crate::ai::agent::task::TaskId;
use crate::ai::blocklist::{
    BlocklistAIHistoryModel, BlocklistAIPermissions, StartAgentExecutorEvent, StartAgentRequest,
};
use crate::ai::cloud_agent_settings::CloudAgentSettings;
use crate::ai::document::ai_document_model::{AIDocumentModel, AIDocumentSaveStatus};
use crate::ai::execution_profiles::profiles::AIExecutionProfilesModel;
use crate::ai::execution_profiles::RunAgentsPermission;
use crate::ai::llms::{AvailableLLMs, LLMId, LLMInfo, LLMPreferences, ModelsByFeature};
use crate::ai::mcp::templatable_manager::TemplatableMCPServerManager;
use crate::appearance::Appearance;
use crate::auth::auth_manager::AuthManager;
use crate::auth::AuthStateProvider;
use crate::cloud_object::model::persistence::CloudModel;
use crate::network::NetworkStatus;
use crate::server::cloud_objects::update_manager::UpdateManager;
use crate::server::ids::SyncId;
use crate::server::server_api::ServerApiProvider;
use crate::server::sync_queue::SyncQueue;
use crate::settings::PrivacySettings;
use crate::terminal::cli_agent_sessions::CLIAgentSessionsModel;
use crate::test_util::settings::initialize_settings_for_tests_with_mode;
use crate::workspaces::team_tester::TeamTesterStatus;
use crate::workspaces::user_workspaces::UserWorkspaces;
use crate::{
    AgentNotificationsModel, GlobalResourceHandles, GlobalResourceHandlesProvider, LaunchMode,
};

struct RunAgentsTestState {
    conversation_id: AIConversationId,
    executor: ModelHandle<RunAgentsExecutor>,
    start_agent_executor: ModelHandle<StartAgentExecutor>,
}

#[derive(Default)]
struct CapturedStartAgentRequests(Vec<StartAgentRequest>);

impl Entity for CapturedStartAgentRequests {
    type Event = ();
}
fn with_plan_id(mut action: AIAgentAction, plan_id: &str) -> AIAgentAction {
    let AIAgentActionType::RunAgents(request) = &mut action.action else {
        panic!("expected run_agents action");
    };
    request.plan_id = plan_id.to_string();
    action
}

fn set_orchestration_profile_model(app: &mut App, model_id: &str) {
    let model_id = LLMId::from(model_id);
    LLMPreferences::handle(app).update(app, |preferences, ctx| {
        preferences.update_feature_model_choices(
            Ok(ModelsByFeature {
                agent_mode: AvailableLLMs::new(
                    LLMId::from("auto"),
                    [
                        LLMInfo::new_for_test("auto"),
                        LLMInfo::new_for_test(model_id.as_str()),
                    ],
                    None,
                )
                .expect("test model choices should be valid"),
                ..Default::default()
            }),
            ctx,
        );
    });
    AIExecutionProfilesModel::handle(app).update(app, |profiles, ctx| {
        let profile_id = *profiles.active_profile(None, ctx).id();
        profiles.set_orchestration_model(profile_id, Some(model_id), ctx);
    });
}

#[test]
fn executor_defaults_omitted_oz_model_from_active_profile() {
    App::test((), |mut app| async move {
        let state = initialize_run_agents_test(&mut app, ExecutionMode::App);
        set_orchestration_profile_model(&mut app, "profile-router");
        let AIAgentActionType::RunAgents(mut request) = remote_run_agents_action("oz").action
        else {
            panic!("expected run_agents action");
        };

        state.executor.update(&mut app, |executor, ctx| {
            populate_default_orchestration_model_for_execution(
                &mut request,
                executor.terminal_view_id,
                ctx,
            );
        });

        assert_eq!(request.model_id, "profile-router");
        let mode = run_agents_to_start_agent_mode(
            &request.execution_mode,
            &request.harness_type,
            &request.model_id,
            &request.skills,
            None,
            &request.agent_run_configs[0],
        )
        .expect("normalized request should map to a child launch");
        assert!(matches!(
            mode,
            StartAgentExecutionMode::Remote { model_id, .. } if model_id == "profile-router"
        ));
    });
}

#[test]
fn execute_fans_out_profile_router_to_every_oz_child_with_distinct_prompts() {
    App::test((), |mut app| async move {
        let state = initialize_run_agents_test(&mut app, ExecutionMode::App);
        set_orchestration_profile_model(&mut app, "profile-router");
        let captured = subscribe_to_start_agent_requests(&mut app, &state.start_agent_executor);
        let mut action = remote_run_agents_action("oz");
        let AIAgentActionType::RunAgents(request) = &mut action.action else {
            panic!("expected run_agents action");
        };
        request.model_id.clear();
        request.execution_mode = RunAgentsExecutionMode::Local;
        request.base_prompt = "Shared orchestration context".to_string();
        request.agent_run_configs = vec![
            RunAgentsAgentRunConfig {
                name: "researcher".to_string(),
                prompt: "Investigate the parser".to_string(),
                title: String::new(),
            },
            RunAgentsAgentRunConfig {
                name: "tester".to_string(),
                prompt: "Design regression coverage".to_string(),
                title: String::new(),
            },
        ];

        let execution = state.executor.update(&mut app, |executor, ctx| {
            executor
                .execute(
                    ExecuteActionInput {
                        action: &action,
                        conversation_id: state.conversation_id,
                    },
                    ctx,
                )
                .into()
        });
        assert!(matches!(execution, AnyActionExecution::Async { .. }));

        for _ in 0..10 {
            if captured.read(&app, |captured, _| captured.0.len()) == 2 {
                break;
            }
            futures_lite::future::yield_now().await;
        }

        captured.read(&app, |captured, _| {
            assert_eq!(captured.0.len(), 2);
            assert_eq!(captured.0[0].name, "researcher");
            assert_eq!(
                captured.0[0].prompt,
                "Shared orchestration context\n\nInvestigate the parser"
            );
            assert_eq!(captured.0[1].name, "tester");
            assert_eq!(
                captured.0[1].prompt,
                "Shared orchestration context\n\nDesign regression coverage"
            );
            for request in &captured.0 {
                assert_eq!(
                    request.execution_mode,
                    StartAgentExecutionMode::Local {
                        harness_type: None,
                        model_id: Some("profile-router".to_string()),
                    }
                );
            }
        });
    });
}

#[test]
fn executor_preserves_explicit_oz_model_over_profile_default() {
    App::test((), |mut app| async move {
        let state = initialize_run_agents_test(&mut app, ExecutionMode::App);
        set_orchestration_profile_model(&mut app, "profile-router");
        let AIAgentActionType::RunAgents(mut request) = remote_run_agents_action("oz").action
        else {
            panic!("expected run_agents action");
        };
        request.model_id = "explicit-model".to_string();

        state.executor.update(&mut app, |executor, ctx| {
            populate_default_orchestration_model_for_execution(
                &mut request,
                executor.terminal_view_id,
                ctx,
            );
        });

        assert_eq!(request.model_id, "explicit-model");
    });
}

#[test]
fn executor_leaves_omitted_third_party_model_empty() {
    App::test((), |mut app| async move {
        let state = initialize_run_agents_test(&mut app, ExecutionMode::App);
        set_orchestration_profile_model(&mut app, "profile-router");
        let AIAgentActionType::RunAgents(mut request) = remote_run_agents_action("claude").action
        else {
            panic!("expected run_agents action");
        };

        state.executor.update(&mut app, |executor, ctx| {
            populate_default_orchestration_model_for_execution(
                &mut request,
                executor.terminal_view_id,
                ctx,
            );
        });

        assert!(request.model_id.is_empty());
    });
}

#[test]
fn executor_rejects_local_router_for_remote_oz() {
    App::test((), |mut app| async move {
        let state = initialize_run_agents_test(&mut app, ExecutionMode::App);
        let AIAgentActionType::RunAgents(mut request) = remote_run_agents_action("oz").action
        else {
            panic!("expected run_agents action");
        };
        request.model_id = "custom-router:local:private".to_string();

        let error = state
            .executor
            .update(&mut app, |_, ctx| validate_request(&request, ctx))
            .expect_err("local router should not be cloud-runnable");

        assert!(error.contains("only available locally"));
    });
}

#[test]
fn executor_allows_local_router_for_local_oz() {
    App::test((), |mut app| async move {
        let state = initialize_run_agents_test(&mut app, ExecutionMode::App);
        let AIAgentActionType::RunAgents(mut request) = remote_run_agents_action("oz").action
        else {
            panic!("expected run_agents action");
        };
        request.execution_mode = RunAgentsExecutionMode::Local;
        request.model_id = "custom-router:local:private".to_string();

        state.executor.update(&mut app, |_, ctx| {
            validate_request(&request, ctx).expect("local Oz should accept local routers");
        });
    });
}

#[test]
fn executor_does_not_apply_oz_validation_to_third_party_model_namespace() {
    App::test((), |mut app| async move {
        let state = initialize_run_agents_test(&mut app, ExecutionMode::App);
        let AIAgentActionType::RunAgents(mut request) = remote_run_agents_action("claude").action
        else {
            panic!("expected run_agents action");
        };
        request.model_id = "custom-router:local:third-party-model-name".to_string();

        state.executor.update(&mut app, |_, ctx| {
            validate_request(&request, ctx)
                .expect("third-party harness model namespaces should remain untouched");
        });
    });
}

#[test]
fn executor_keeps_approved_model_over_profile_and_explicit_models() {
    App::test((), |mut app| async move {
        let state = initialize_run_agents_test(&mut app, ExecutionMode::App);
        set_orchestration_profile_model(&mut app, "profile-router");
        let AIAgentActionType::RunAgents(mut request) = remote_run_agents_action("oz").action
        else {
            panic!("expected run_agents action");
        };
        request.model_id = "explicit-model".to_string();
        resolve_request_from_config(
            &mut request,
            &OrchestrationConfig {
                model_id: "approved-model".to_string(),
                harness_type: "oz".to_string(),
                execution_mode: OrchestrationExecutionMode::Remote {
                    environment_id: "env-1".to_string(),
                    worker_host: "warp".to_string(),
                },
            },
        );

        state.executor.update(&mut app, |executor, ctx| {
            populate_default_orchestration_model_for_execution(
                &mut request,
                executor.terminal_view_id,
                ctx,
            );
        });

        assert_eq!(request.model_id, "approved-model");
    });
}

#[test]
fn run_agents_result_reports_normalized_model_as_resolved_model_id() {
    let result = RunAgentsResult::Launched {
        model_id: "profile-router".to_string(),
        harness_type: "oz".to_string(),
        execution_mode: RunAgentsLaunchedExecutionMode::Local,
        agents: Vec::new(),
    };

    let wire_result =
        warp_multi_agent_api::request::input::tool_call_result::Result::try_from(result)
            .expect("launched result should convert");
    let warp_multi_agent_api::request::input::tool_call_result::Result::RunAgentsResult(result) =
        wire_result
    else {
        panic!("expected run_agents result");
    };
    let Some(warp_multi_agent_api::run_agents_result::Outcome::Launched(launched)) = result.outcome
    else {
        panic!("expected launched outcome");
    };

    assert_eq!(launched.resolved_model_id, "profile-router");
}

fn persist_plan_config(
    app: &mut App,
    conversation_id: AIConversationId,
    plan_id: &str,
    status: OrchestrationConfigStatus,
) {
    persist_plan_config_with_harness(app, conversation_id, plan_id, "oz", status);
}

fn persist_plan_config_with_harness(
    app: &mut App,
    conversation_id: AIConversationId,
    plan_id: &str,
    harness_type: &str,
    status: OrchestrationConfigStatus,
) {
    BlocklistAIHistoryModel::handle(app).update(app, |history, _ctx| {
        history
            .conversation_mut(&conversation_id)
            .expect("conversation should exist")
            .set_orchestration_config_for_plan(
                plan_id.to_string(),
                OrchestrationConfig {
                    model_id: "auto".to_string(),
                    harness_type: harness_type.to_string(),
                    execution_mode: OrchestrationExecutionMode::Remote {
                        environment_id: "env-1".to_string(),
                        worker_host: "warp".to_string(),
                    },
                },
                status,
            );
    });
}

#[test]
fn should_autoexecute_duplicate_launched_agent_denial() {
    App::test((), |mut app| async move {
        let state = initialize_run_agents_test(&mut app, ExecutionMode::App);
        state.executor.update(&mut app, |executor, _ctx| {
            executor.record_launched_agents(
                state.conversation_id,
                &[RunAgentsAgentOutcome {
                    name: "child".to_string(),
                    kind: RunAgentsAgentOutcomeKind::Launched {
                        agent_id: "agent-123".to_string(),
                    },
                }],
            );
        });
        let action = remote_run_agents_action("oz");

        let should_autoexecute = state.executor.update(&mut app, |executor, ctx| {
            executor.should_autoexecute(
                ExecuteActionInput {
                    action: &action,
                    conversation_id: state.conversation_id,
                },
                ctx,
            )
        });

        assert!(should_autoexecute);
    });
}

#[test]
fn execute_denies_duplicate_launched_agent() {
    App::test((), |mut app| async move {
        let state = initialize_run_agents_test(&mut app, ExecutionMode::App);
        state.executor.update(&mut app, |executor, _ctx| {
            executor.record_launched_agents(
                state.conversation_id,
                &[RunAgentsAgentOutcome {
                    name: "child".to_string(),
                    kind: RunAgentsAgentOutcomeKind::Launched {
                        agent_id: "agent-123".to_string(),
                    },
                }],
            );
        });
        let action = with_agent_name(remote_run_agents_action("oz"), "Child");

        let execution = state.executor.update(&mut app, |executor, ctx| {
            executor
                .execute(
                    ExecuteActionInput {
                        action: &action,
                        conversation_id: state.conversation_id,
                    },
                    ctx,
                )
                .into()
        });

        let AnyActionExecution::Sync(AIAgentActionResultType::RunAgents(RunAgentsResult::Denied {
            reason,
        })) = execution
        else {
            panic!("expected synchronous run_agents denial");
        };
        assert!(reason.contains("child (agent-123)"));
        assert!(reason.contains("send_message_to_agent"));
    });
}

fn initialize_run_agents_test(app: &mut App, mode: ExecutionMode) -> RunAgentsTestState {
    initialize_settings_for_tests_with_mode(app, mode, false);
    app.add_singleton_model(|_| ServerApiProvider::new_for_test());
    let global_resource_handles = GlobalResourceHandles::mock(app);
    app.add_singleton_model(|_| GlobalResourceHandlesProvider::new(global_resource_handles));
    let history = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], vec![], &[]));
    app.add_singleton_model(|_| CLIAgentSessionsModel::new());
    app.add_singleton_model(|_| ActiveAgentViewsModel::new());
    app.add_singleton_model(AgentNotificationsModel::new);
    app.add_singleton_model(BlocklistAIPermissions::new);
    let terminal_view_id = EntityId::new();
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());
    app.add_singleton_model(AuthManager::new_for_test);
    app.add_singleton_model(SyncQueue::mock);
    app.add_singleton_model(|_| NetworkStatus::new());
    app.add_singleton_model(TeamTesterStatus::mock);
    app.add_singleton_model(UpdateManager::mock);
    app.add_singleton_model(CloudModel::mock);
    app.add_singleton_model(|_| Appearance::mock());
    app.add_singleton_model(|_| AIDocumentModel::new_for_test());
    app.add_singleton_model(|_| TemplatableMCPServerManager::default());
    app.add_singleton_model(|ctx| {
        AIExecutionProfilesModel::new(&LaunchMode::new_for_unit_test(), ctx)
    });
    app.add_singleton_model(PrivacySettings::mock);
    app.add_singleton_model(UserWorkspaces::default_mock);
    app.add_singleton_model(LLMPreferences::new);
    let conversation_id = history.update(app, |history_model, ctx| {
        history_model.start_new_conversation(terminal_view_id, false, false, false, ctx)
    });
    let start_agent_executor = app.add_model(StartAgentExecutor::new);
    let executor =
        app.add_model(|_| RunAgentsExecutor::new(start_agent_executor.clone(), terminal_view_id));

    RunAgentsTestState {
        conversation_id,
        executor,
        start_agent_executor,
    }
}

fn subscribe_to_start_agent_requests(
    app: &mut App,
    start_agent_executor: &ModelHandle<StartAgentExecutor>,
) -> ModelHandle<CapturedStartAgentRequests> {
    let captured = app.add_model(|_| CapturedStartAgentRequests::default());
    captured.update(app, |_, ctx| {
        ctx.subscribe_to_model(start_agent_executor, |captured, _, event, _ctx| {
            if let StartAgentExecutorEvent::CreateAgent(request) = event {
                captured.0.push(request.as_ref().clone());
            }
        });
    });
    captured
}

fn remote_run_agents_action(harness_type: &str) -> AIAgentAction {
    AIAgentAction {
        id: AIAgentActionId::from("run-agents-action".to_string()),
        task_id: TaskId::new("run-agents-task".to_string()),
        requires_result: true,
        action: AIAgentActionType::RunAgents(RunAgentsRequest {
            summary: "Run child agent".to_string(),
            base_prompt: "Help".to_string(),
            skills: vec![],
            model_id: String::new(),
            harness_type: harness_type.to_string(),
            execution_mode: RunAgentsExecutionMode::Remote {
                environment_id: "env-1".to_string(),
                worker_host: "warp".to_string(),
                computer_use_enabled: false,
            },
            agent_run_configs: vec![RunAgentsAgentRunConfig {
                name: "child".to_string(),
                prompt: "Help".to_string(),
                title: String::new(),
            }],
            plan_id: String::new(),
            harness_auth_secret_name: None,
        }),
    }
}

fn with_agent_name(mut action: AIAgentAction, name: &str) -> AIAgentAction {
    let AIAgentActionType::RunAgents(request) = &mut action.action else {
        panic!("expected run_agents action");
    };
    request.agent_run_configs[0].name = name.to_string();
    action
}

#[test]
fn local_codex_run_agents_maps_to_local_harness_mode_when_flag_enabled() {
    let _local_codex = FeatureFlag::LocalClaudeCodexChildHarnesses.override_enabled(true);
    let cfg = RunAgentsAgentRunConfig {
        name: "child".to_string(),
        prompt: "Investigate the failure".to_string(),
        title: String::new(),
    };

    let mode = run_agents_to_start_agent_mode(
        &RunAgentsExecutionMode::Local,
        "codex",
        "",
        &[],
        None,
        &cfg,
    )
    .expect("local Codex should be accepted when the feature flag is enabled");

    assert_eq!(
        mode,
        StartAgentExecutionMode::Local {
            harness_type: Some("codex".to_string()),
            model_id: None,
        }
    );
}

fn persist_default_auth_secret(app: &mut App, harness_config_name: &str, secret_name: &str) {
    CloudAgentSettings::handle(app).update(app, |settings, ctx| {
        let mut secrets = settings.last_selected_auth_secret.value().clone();
        secrets.insert(harness_config_name.to_string(), secret_name.to_string());
        settings
            .last_selected_auth_secret
            .set_value(secrets, ctx)
            .unwrap();
        settings
            .inherit_auth_secret_harnesses
            .set_value(HashMap::new(), ctx)
            .unwrap();
    });
}

#[test]
fn should_autoexecute_when_plan_has_approved_orchestration_config() {
    App::test((), |mut app| async move {
        let state = initialize_run_agents_test(&mut app, ExecutionMode::App);
        persist_plan_config(
            &mut app,
            state.conversation_id,
            "plan-1",
            OrchestrationConfigStatus::Approved,
        );
        let action = with_plan_id(remote_run_agents_action("oz"), "plan-1");

        let should_autoexecute = state.executor.update(&mut app, |executor, ctx| {
            executor.should_autoexecute(
                ExecuteActionInput {
                    action: &action,
                    conversation_id: state.conversation_id,
                },
                ctx,
            )
        });

        assert!(should_autoexecute);
    });
}

#[test]
fn should_not_autoexecute_approved_remote_non_warp_plan_without_default_auth_secret() {
    App::test((), |mut app| async move {
        let state = initialize_run_agents_test(&mut app, ExecutionMode::App);
        persist_plan_config_with_harness(
            &mut app,
            state.conversation_id,
            "plan-1",
            "codex",
            OrchestrationConfigStatus::Approved,
        );
        let action = with_plan_id(remote_run_agents_action("oz"), "plan-1");

        let should_autoexecute = state.executor.update(&mut app, |executor, ctx| {
            executor.should_autoexecute(
                ExecuteActionInput {
                    action: &action,
                    conversation_id: state.conversation_id,
                },
                ctx,
            )
        });

        assert!(!should_autoexecute);
    });
}

#[test]
fn execute_denies_disapproved_plan_config() {
    App::test((), |mut app| async move {
        let state = initialize_run_agents_test(&mut app, ExecutionMode::App);
        persist_plan_config(
            &mut app,
            state.conversation_id,
            "plan-1",
            OrchestrationConfigStatus::Disapproved,
        );
        let action = with_plan_id(remote_run_agents_action("oz"), "plan-1");

        let execution = state.executor.update(&mut app, |executor, ctx| {
            executor
                .execute(
                    ExecuteActionInput {
                        action: &action,
                        conversation_id: state.conversation_id,
                    },
                    ctx,
                )
                .into()
        });

        let AnyActionExecution::Sync(AIAgentActionResultType::RunAgents(RunAgentsResult::Denied {
            reason,
        })) = execution
        else {
            panic!("expected synchronous run_agents denial");
        };
        assert_eq!(reason, "Orchestration config was disapproved");
    });
}

#[test]
fn execute_denies_never_allow_profile_setting() {
    App::test((), |mut app| async move {
        let state = initialize_run_agents_test(&mut app, ExecutionMode::App);
        set_run_agents_permission(&mut app, RunAgentsPermission::NeverAllow);
        let action = remote_run_agents_action("oz");

        let execution = state.executor.update(&mut app, |executor, ctx| {
            executor
                .execute(
                    ExecuteActionInput {
                        action: &action,
                        conversation_id: state.conversation_id,
                    },
                    ctx,
                )
                .into()
        });

        let AnyActionExecution::Sync(AIAgentActionResultType::RunAgents(RunAgentsResult::Denied {
            reason,
        })) = execution
        else {
            panic!("expected synchronous run_agents denial");
        };
        assert_eq!(
            reason,
            "Running child agents is disabled by the active execution profile."
        );
    });
}

#[test]
fn autonomous_mode_autoexecutes_and_does_not_deny_missing_api_key() {
    App::test((), |mut app| async move {
        let state = initialize_run_agents_test(&mut app, ExecutionMode::Sdk);
        set_run_agents_permission(&mut app, RunAgentsPermission::NeverAllow);
        let action = remote_run_agents_action("codex");

        let should_autoexecute = state.executor.update(&mut app, |executor, ctx| {
            executor.should_autoexecute(
                ExecuteActionInput {
                    action: &action,
                    conversation_id: state.conversation_id,
                },
                ctx,
            )
        });
        assert!(should_autoexecute);

        let execution = state.executor.update(&mut app, |executor, ctx| {
            executor
                .execute(
                    ExecuteActionInput {
                        action: &action,
                        conversation_id: state.conversation_id,
                    },
                    ctx,
                )
                .into()
        });
        assert!(matches!(execution, AnyActionExecution::Async { .. }));
    });
}

#[test]
fn execute_publishes_every_parent_owned_plan_before_dispatch() {
    App::test((), |mut app| async move {
        let state = initialize_run_agents_test(&mut app, ExecutionMode::Sdk);
        BlocklistAIHistoryModel::handle(&app).update(&mut app, |model, ctx| {
            model.assign_run_id_for_conversation(
                state.conversation_id,
                "00000000-0000-0000-0000-000000000001".to_string(),
                None,
                EntityId::new(),
                ctx,
            );
        });
        let unrelated_conversation_id = AIConversationId::new();
        let (first_plan_id, second_plan_id, unrelated_plan_id) = AIDocumentModel::handle(&app)
            .update(&mut app, |model, ctx| {
                (
                    model.create_document(
                        "First plan",
                        "# First",
                        state.conversation_id,
                        None,
                        ctx,
                    ),
                    model.create_document(
                        "Second plan",
                        "# Second",
                        state.conversation_id,
                        None,
                        ctx,
                    ),
                    model.create_document(
                        "Unrelated plan",
                        "# Unrelated",
                        unrelated_conversation_id,
                        None,
                        ctx,
                    ),
                )
            });
        let captured = subscribe_to_start_agent_requests(&mut app, &state.start_agent_executor);
        let action = remote_run_agents_action("oz");

        let execution = state.executor.update(&mut app, |executor, ctx| {
            executor
                .execute(
                    ExecuteActionInput {
                        action: &action,
                        conversation_id: state.conversation_id,
                    },
                    ctx,
                )
                .into()
        });

        assert!(matches!(execution, AnyActionExecution::Async { .. }));
        captured.read(&app, |captured, _ctx| {
            assert!(captured.0.is_empty());
        });
        AIDocumentModel::handle(&app).read(&app, |model, _ctx| {
            assert!(matches!(
                model.get_document_save_status(&first_plan_id),
                AIDocumentSaveStatus::Saving
            ));
            assert!(matches!(
                model.get_document_save_status(&second_plan_id),
                AIDocumentSaveStatus::Saving
            ));
            assert!(matches!(
                model.get_document_save_status(&unrelated_plan_id),
                AIDocumentSaveStatus::NotSaved
            ));
        });
    });
}

/// A run_agents call holds in the `Publishing` state while it waits for the parent's
/// plans to become server-backed, then dispatches children. This verifies that
/// cancelling mid-publication prevents fan-out: even when the plan finishes publishing
/// afterwards (resolving the wait), the post-wait dispatch is skipped because
/// `cancel_execution` cleared the pending marker that `is_pending` guards on.
#[test]
fn cancel_during_plan_publication_does_not_dispatch_children() {
    App::test((), |mut app| async move {
        let state = initialize_run_agents_test(&mut app, ExecutionMode::Sdk);
        BlocklistAIHistoryModel::handle(&app).update(&mut app, |model, ctx| {
            model.assign_run_id_for_conversation(
                state.conversation_id,
                "00000000-0000-0000-0000-000000000001".to_string(),
                None,
                EntityId::new(),
                ctx,
            );
        });
        let plan_id = AIDocumentModel::handle(&app).update(&mut app, |model, ctx| {
            model.create_document("Plan", "# Plan", state.conversation_id, None, ctx)
        });
        let captured = subscribe_to_start_agent_requests(&mut app, &state.start_agent_executor);
        let action = remote_run_agents_action("oz");
        let action_id = action.id.clone();

        let execution = state.executor.update(&mut app, |executor, ctx| {
            executor
                .execute(
                    ExecuteActionInput {
                        action: &action,
                        conversation_id: state.conversation_id,
                    },
                    ctx,
                )
                .into()
        });
        // The action is awaiting plan publication, so it's pending but no children dispatched yet.
        assert!(matches!(execution, AnyActionExecution::Async { .. }));
        state.executor.update(&mut app, |executor, ctx| {
            assert!(executor.is_pending(&action_id));
            executor.cancel_execution(&action_id, ctx);
            assert!(!executor.is_pending(&action_id));
        });

        // Finish publishing the plan, which resolves the wait the dispatch was blocked on.
        AIDocumentModel::handle(&app).update(&mut app, |model, ctx| {
            model.create_document_from_notebook(
                plan_id,
                SyncId::ServerId(123.into()),
                "Plan",
                "# Plan",
                state.conversation_id,
                None,
                ctx,
            );
        });
        for _ in 0..3 {
            futures_lite::future::yield_now().await;
        }

        // Cancellation won the race: the resolved wait does not fan out children.
        captured.read(&app, |captured, _ctx| {
            assert!(captured.0.is_empty());
        });
    });
}

fn set_run_agents_permission(app: &mut App, permission: RunAgentsPermission) {
    AIExecutionProfilesModel::handle(app).update(app, |profiles, ctx| {
        let profile_id = *profiles.active_profile(None, ctx).id();
        profiles.set_run_agents(profile_id, permission, ctx);
    });
}

#[test]
fn should_not_autoexecute_without_approved_plan_or_always_allow_profile() {
    App::test((), |mut app| async move {
        let state = initialize_run_agents_test(&mut app, ExecutionMode::App);
        let action = remote_run_agents_action("oz");

        let should_autoexecute = state.executor.update(&mut app, |executor, ctx| {
            executor.should_autoexecute(
                ExecuteActionInput {
                    action: &action,
                    conversation_id: state.conversation_id,
                },
                ctx,
            )
        });

        assert!(!should_autoexecute);
    });
}

#[test]
fn execute_denies_remote_non_warp_harness_without_default_auth_secret() {
    App::test((), |mut app| async move {
        let state = initialize_run_agents_test(&mut app, ExecutionMode::App);
        let action = remote_run_agents_action("codex");

        let execution = state.executor.update(&mut app, |executor, ctx| {
            executor
                .execute(
                    ExecuteActionInput {
                        action: &action,
                        conversation_id: state.conversation_id,
                    },
                    ctx,
                )
                .into()
        });

        let AnyActionExecution::Sync(AIAgentActionResultType::RunAgents(RunAgentsResult::Denied {
            reason,
        })) = execution
        else {
            panic!("expected synchronous run_agents denial");
        };
        assert_eq!(
            reason,
            "Cloud child agents using this harness require an API key before they can run."
        );
    });
}

#[test]
fn should_autoexecute_remote_non_warp_harness_with_always_allow_even_without_default_auth_secret() {
    App::test((), |mut app| async move {
        let state = initialize_run_agents_test(&mut app, ExecutionMode::App);
        set_run_agents_permission(&mut app, RunAgentsPermission::AlwaysAllow);
        let action = remote_run_agents_action("codex");

        let should_autoexecute = state.executor.update(&mut app, |executor, ctx| {
            executor.should_autoexecute(
                ExecuteActionInput {
                    action: &action,
                    conversation_id: state.conversation_id,
                },
                ctx,
            )
        });

        assert!(should_autoexecute);
    });
}

#[test]
fn should_autoexecute_remote_non_warp_harness_with_default_auth_secret() {
    App::test((), |mut app| async move {
        let state = initialize_run_agents_test(&mut app, ExecutionMode::App);
        set_run_agents_permission(&mut app, RunAgentsPermission::AlwaysAllow);
        persist_default_auth_secret(&mut app, "codex", "default-openai-key");
        let action = remote_run_agents_action("codex");

        let should_autoexecute = state.executor.update(&mut app, |executor, ctx| {
            executor.should_autoexecute(
                ExecuteActionInput {
                    action: &action,
                    conversation_id: state.conversation_id,
                },
                ctx,
            )
        });

        assert!(should_autoexecute);
    });
}

#[test]
fn should_autoexecute_remote_warp_harness_without_default_auth_secret() {
    App::test((), |mut app| async move {
        let state = initialize_run_agents_test(&mut app, ExecutionMode::App);
        set_run_agents_permission(&mut app, RunAgentsPermission::AlwaysAllow);
        let action = remote_run_agents_action("oz");

        let should_autoexecute = state.executor.update(&mut app, |executor, ctx| {
            executor.should_autoexecute(
                ExecuteActionInput {
                    action: &action,
                    conversation_id: state.conversation_id,
                },
                ctx,
            )
        });

        assert!(should_autoexecute);
    });
}

#[test]
fn populate_default_auth_secret_for_autoexecute_uses_persisted_secret() {
    App::test((), |mut app| async move {
        let state = initialize_run_agents_test(&mut app, ExecutionMode::App);
        persist_default_auth_secret(&mut app, "claude", "default-anthropic-key");
        let AIAgentActionType::RunAgents(mut request) = remote_run_agents_action("claude").action
        else {
            panic!("expected run_agents action");
        };

        state.executor.update(&mut app, |_, ctx| {
            populate_default_auth_secret_for_execution(&mut request, ctx);
        });

        assert_eq!(
            request.harness_auth_secret_name.as_deref(),
            Some("default-anthropic-key")
        );
    });
}

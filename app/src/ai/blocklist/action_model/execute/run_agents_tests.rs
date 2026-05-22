use std::collections::HashMap;

use ai::agent::action::{RunAgentsAgentRunConfig, RunAgentsExecutionMode, RunAgentsRequest};
use ai::agent::orchestration_config::{
    OrchestrationConfig, OrchestrationConfigStatus, OrchestrationExecutionMode,
};
use settings::Setting;
use warp_core::execution_mode::ExecutionMode;
use warpui::{App, EntityId, ModelHandle};

use super::*;
use crate::ai::active_agent_views_model::ActiveAgentViewsModel;
use crate::ai::agent::task::TaskId;
use crate::ai::blocklist::{BlocklistAIHistoryModel, BlocklistAIPermissions};
use crate::ai::cloud_agent_settings::CloudAgentSettings;
use crate::ai::execution_profiles::profiles::AIExecutionProfilesModel;
use crate::ai::execution_profiles::RunAgentsPermission;
use crate::ai::mcp::templatable_manager::TemplatableMCPServerManager;
use crate::auth::AuthStateProvider;
use crate::cloud_object::model::persistence::CloudModel;
use crate::network::NetworkStatus;
use crate::server::cloud_objects::update_manager::UpdateManager;
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
}
fn with_plan_id(mut action: AIAgentAction, plan_id: &str) -> AIAgentAction {
    let AIAgentActionType::RunAgents(request) = &mut action.action else {
        panic!("expected run_agents action");
    };
    request.plan_id = plan_id.to_string();
    action
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

fn initialize_run_agents_test(app: &mut App, mode: ExecutionMode) -> RunAgentsTestState {
    initialize_settings_for_tests_with_mode(app, mode, false);
    let global_resource_handles = GlobalResourceHandles::mock(app);
    app.add_singleton_model(|_| GlobalResourceHandlesProvider::new(global_resource_handles));
    let history = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));
    app.add_singleton_model(|_| CLIAgentSessionsModel::new());
    app.add_singleton_model(|_| ActiveAgentViewsModel::new());
    app.add_singleton_model(AgentNotificationsModel::new);
    app.add_singleton_model(BlocklistAIPermissions::new);
    let terminal_view_id = EntityId::new();
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());
    app.add_singleton_model(SyncQueue::mock);
    app.add_singleton_model(|_| NetworkStatus::new());
    app.add_singleton_model(TeamTesterStatus::mock);
    app.add_singleton_model(UpdateManager::mock);
    app.add_singleton_model(CloudModel::mock);
    app.add_singleton_model(|_| TemplatableMCPServerManager::default());
    app.add_singleton_model(|ctx| {
        AIExecutionProfilesModel::new(&LaunchMode::new_for_unit_test(), ctx)
    });
    app.add_singleton_model(PrivacySettings::mock);
    app.add_singleton_model(UserWorkspaces::default_mock);
    let conversation_id = history.update(app, |history_model, ctx| {
        history_model.start_new_conversation(terminal_view_id, false, false, false, ctx)
    });
    let start_agent_executor = app.add_model(StartAgentExecutor::new);
    let executor =
        app.add_model(|_| RunAgentsExecutor::new(start_agent_executor, terminal_view_id));

    RunAgentsTestState {
        conversation_id,
        executor,
    }
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

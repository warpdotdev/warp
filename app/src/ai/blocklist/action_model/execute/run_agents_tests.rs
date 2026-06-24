use ai::agent::action::{RunAgentsAgentRunConfig, RunAgentsExecutionMode, RunAgentsRequest};
use warp_core::execution_mode::ExecutionMode;
use warpui::{App, EntityId, SingletonEntity};

use super::*;
use crate::ai::active_agent_views_model::ActiveAgentViewsModel;
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::agent::task::TaskId;
use crate::ai::blocklist::{BlocklistAIHistoryModel, BlocklistAIPermissions};
use crate::ai::document::ai_document_model::{AIDocumentModel, AIDocumentSaveStatus};
use crate::ai::execution_profiles::profiles::AIExecutionProfilesModel;
use crate::ai::mcp::templatable_manager::TemplatableMCPServerManager;
use crate::appearance::Appearance;
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
    app.add_singleton_model(|_| Appearance::mock());
    app.add_singleton_model(|_| AIDocumentModel::new_for_test());
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
        app.add_model(|_| RunAgentsExecutor::new(start_agent_executor.clone(), terminal_view_id));

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

#[test]
fn local_codex_run_agents_maps_to_local_harness_mode() {
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
    .expect("local Codex should still parse for persisted request compatibility");

    assert_eq!(
        mode,
        StartAgentExecutionMode::Local {
            harness_type: Some("codex".to_string()),
            model_id: None,
        }
    );
}

#[test]
fn execute_denies_run_agents_without_dispatching_children() {
    App::test((), |mut app| async move {
        let state = initialize_run_agents_test(&mut app, ExecutionMode::App);
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
        assert_eq!(reason, RUN_AGENTS_DISABLED_REASON);
        state.executor.read(&app, |executor, _| {
            assert!(!executor.is_pending(&action.id));
        });
    });
}

#[test]
fn should_autoexecute_run_agents_so_disabled_result_is_recorded() {
    App::test((), |mut app| async move {
        let state = initialize_run_agents_test(&mut app, ExecutionMode::App);
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
fn execute_does_not_publish_parent_plans() {
    App::test((), |mut app| async move {
        let state = initialize_run_agents_test(&mut app, ExecutionMode::Sdk);
        let plan_id = AIDocumentModel::handle(&app).update(&mut app, |model, ctx| {
            model.create_document("Plan", "# Plan", state.conversation_id, None, ctx)
        });
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

        assert!(matches!(execution, AnyActionExecution::Sync(_)));
        AIDocumentModel::handle(&app).read(&app, |model, _ctx| {
            assert!(matches!(
                model.get_document_save_status(&plan_id),
                AIDocumentSaveStatus::NotSaved
            ));
        });
    });
}

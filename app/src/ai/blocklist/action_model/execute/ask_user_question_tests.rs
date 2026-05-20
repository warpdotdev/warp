use super::*;
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::agent::task::TaskId;
use crate::ai::agent::{AIAgentAction, AIAgentActionId, AIAgentActionResultType};
use crate::ai::blocklist::{BlocklistAIHistoryModel, BlocklistAIPermissions};
use crate::ai::execution_profiles::{
    profiles::AIExecutionProfilesModel, AskUserQuestionPermission,
};
use crate::ai::mcp::templatable_manager::TemplatableMCPServerManager;
use crate::auth::AuthStateProvider;
use crate::cloud_object::model::persistence::CloudModel;
use crate::network::NetworkStatus;
use crate::server::{cloud_objects::update_manager::UpdateManager, sync_queue::SyncQueue};
use crate::test_util::settings::initialize_settings_for_tests;
use crate::workspaces::{team_tester::TeamTesterStatus, user_workspaces::UserWorkspaces};
use crate::LaunchMode;
use ai::agent::action::AskUserQuestionItem;
use ai::agent::action_result::{AskUserQuestionAnswerItem, AskUserQuestionResult};
use warpui::{App, EntityId, ModelHandle};

fn build_action(action_id: &str) -> AIAgentAction {
    AIAgentAction {
        id: AIAgentActionId::from(action_id.to_string()),
        action: AIAgentActionType::AskUserQuestion {
            questions: vec![AskUserQuestionItem {
                question_id: "q1".to_string(),
                question: "What should we use?".to_string(),
                question_type: ai::agent::action::AskUserQuestionType::MultipleChoice {
                    is_multiselect: false,
                    options: vec![],
                    supports_other: true,
                },
            }],
        },
        task_id: TaskId::new(format!("task-{action_id}")),
        requires_result: false,
    }
}

#[test]
fn should_autoexecute_returns_false_when_autoapprove_is_enabled_and_profile_always_blocks() {
    App::test((), |mut app| async move {
        let terminal_view_id = EntityId::new();
        let (history, profiles) = initialize_ask_user_question_test(&mut app, terminal_view_id);
        let executor = app.add_model(|_| AskUserQuestionExecutor::new(terminal_view_id));
        let action = build_action("ask-user-question");
        let conversation_id = history.update(&mut app, |history, ctx| {
            history.start_new_conversation(terminal_view_id, true, false, false, ctx)
        });

        profiles.update(&mut app, |profiles, ctx| {
            let profile_id = *profiles.active_profile(Some(terminal_view_id), ctx).id();
            profiles.set_ask_user_question(profile_id, AskUserQuestionPermission::AlwaysAsk, ctx);
        });

        let result = executor.update(&mut app, |executor, ctx| {
            let input = ExecuteActionInput {
                action: &action,
                conversation_id,
            };
            executor.should_autoexecute(input, ctx)
        });

        assert!(!result);
    });
}

fn initialize_ask_user_question_test(
    app: &mut App,
    terminal_view_id: EntityId,
) -> (
    ModelHandle<BlocklistAIHistoryModel>,
    ModelHandle<AIExecutionProfilesModel>,
) {
    initialize_settings_for_tests(app);
    let history = app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());
    app.add_singleton_model(SyncQueue::mock);
    app.add_singleton_model(|_| NetworkStatus::new());
    app.add_singleton_model(TeamTesterStatus::mock);
    app.add_singleton_model(UpdateManager::mock);
    app.add_singleton_model(CloudModel::mock);
    app.add_singleton_model(|_| TemplatableMCPServerManager::default());
    app.add_singleton_model(UserWorkspaces::default_mock);
    let profiles = app.add_singleton_model(|ctx| {
        AIExecutionProfilesModel::new(&LaunchMode::new_for_unit_test(), ctx)
    });
    app.add_singleton_model(BlocklistAIPermissions::new);
    // Ensure asking questions is allowed by default regardless of compile-time profile
    // defaults (e.g. agent_mode_evals overrides ask_user_question to Never).
    profiles.update(app, |profiles, ctx| {
        if let Some(profile_id) = profiles.create_profile(ctx) {
            profiles.set_ask_user_question(
                profile_id,
                AskUserQuestionPermission::AskExceptInAutoApprove,
                ctx,
            );
            profiles.set_active_profile(terminal_view_id, profile_id, ctx);
        }
    });
    (history, profiles)
}

#[test]
fn should_autoexecute_returns_false_when_questions_are_allowed() {
    App::test((), |mut app| async move {
        let terminal_view_id = EntityId::new();
        initialize_ask_user_question_test(&mut app, terminal_view_id);
        let executor = app.add_model(|_| AskUserQuestionExecutor::new(terminal_view_id));
        let action = build_action("ask-user-question");
        let result = executor.update(&mut app, |executor, ctx| {
            let input = ExecuteActionInput {
                action: &action,
                conversation_id: AIConversationId::new(),
            };
            executor.should_autoexecute(input, ctx)
        });

        assert!(!result);
    });
}

#[test]
fn should_autoexecute_returns_true_when_autoapprove_is_enabled_and_profile_allows_override() {
    App::test((), |mut app| async move {
        let terminal_view_id = EntityId::new();
        let (history, _) = initialize_ask_user_question_test(&mut app, terminal_view_id);
        let executor = app.add_model(|_| AskUserQuestionExecutor::new(terminal_view_id));
        let action = build_action("ask-user-question");
        let conversation_id = history.update(&mut app, |history, ctx| {
            history.start_new_conversation(terminal_view_id, true, false, false, ctx)
        });
        let result = executor.update(&mut app, |executor, ctx| {
            let input = ExecuteActionInput {
                action: &action,
                conversation_id,
            };
            executor.should_autoexecute(input, ctx)
        });

        assert!(result);
    });
}

#[test]
fn execute_returns_sync_skipped_question_ids_when_autoapprove_is_enabled() {
    App::test((), |mut app| async move {
        let terminal_view_id = EntityId::new();
        let (history, _) = initialize_ask_user_question_test(&mut app, terminal_view_id);
        let executor = app.add_model(|_| AskUserQuestionExecutor::new(terminal_view_id));
        let action = build_action("ask-user-question");
        let conversation_id = history.update(&mut app, |history, ctx| {
            history.start_new_conversation(terminal_view_id, true, false, false, ctx)
        });

        let execution = executor.update(&mut app, |executor, ctx| {
            let input = ExecuteActionInput {
                action: &action,
                conversation_id,
            };
            let result: AnyActionExecution = executor.execute(input, ctx).into();
            result
        });

        assert!(matches!(
            execution,
            AnyActionExecution::Sync(AIAgentActionResultType::AskUserQuestion(
                AskUserQuestionResult::SkippedByAutoApprove { question_ids }
            )) if question_ids == vec!["q1".to_string()]
        ));
    });
}

#[test]
fn execute_returns_async_and_resolves_on_complete() {
    App::test((), |mut app| async move {
        let terminal_view_id = EntityId::new();
        initialize_ask_user_question_test(&mut app, terminal_view_id);
        let executor = app.add_model(|_| AskUserQuestionExecutor::new(terminal_view_id));
        let action = build_action("action-a");

        let execution = executor.update(&mut app, |executor, ctx| {
            let input = ExecuteActionInput {
                action: &action,
                conversation_id: AIConversationId::new(),
            };
            let result: AnyActionExecution = executor.execute(input, ctx).into();
            result
        });

        let AnyActionExecution::Async {
            execute_future,
            on_complete,
        } = execution
        else {
            panic!("expected async execution");
        };

        executor.update(&mut app, |executor, _| {
            executor.complete(vec![AskUserQuestionAnswerItem::Skipped {
                question_id: "q1".to_string(),
            }]);
        });

        let async_result = execute_future.await;
        let result = app.update(|ctx| on_complete(async_result, ctx));
        assert!(matches!(
            result,
            AIAgentActionResultType::AskUserQuestion(AskUserQuestionResult::Success { answers })
                if answers
                    == vec![AskUserQuestionAnswerItem::Skipped {
                        question_id: "q1".to_string(),
                    }]
        ));
    });
}

#[test]
fn cancel_resolves_as_cancelled() {
    App::test((), |mut app| async move {
        let terminal_view_id = EntityId::new();
        initialize_ask_user_question_test(&mut app, terminal_view_id);
        let executor = app.add_model(|_| AskUserQuestionExecutor::new(terminal_view_id));
        let action = build_action("ask-user-question");

        let execution = executor.update(&mut app, |executor, ctx| {
            let input = ExecuteActionInput {
                action: &action,
                conversation_id: AIConversationId::new(),
            };
            let result: AnyActionExecution = executor.execute(input, ctx).into();
            result
        });

        let AnyActionExecution::Async {
            execute_future,
            on_complete,
        } = execution
        else {
            panic!("expected async execution");
        };

        executor.update(&mut app, |executor, _| {
            executor.cancel();
        });

        let async_result = execute_future.await;
        let result = app.update(|ctx| on_complete(async_result, ctx));
        assert!(matches!(
            result,
            AIAgentActionResultType::AskUserQuestion(AskUserQuestionResult::Cancelled)
        ));
    });
}

#[test]
fn should_autoexecute_uses_active_terminal_profile_permission() {
    App::test((), |mut app| async move {
        let terminal_view_id = EntityId::new();
        let (history, profiles) = initialize_ask_user_question_test(&mut app, terminal_view_id);
        let executor = app.add_model(|_| AskUserQuestionExecutor::new(terminal_view_id));
        let action = build_action("ask-user-question");
        let conversation_id = history.update(&mut app, |history, ctx| {
            history.start_new_conversation(terminal_view_id, false, false, false, ctx)
        });

        profiles.update(&mut app, |profiles, ctx| {
            let profile_id = profiles
                .create_profile(ctx)
                .expect("test profile should be created");
            profiles.set_ask_user_question(profile_id, AskUserQuestionPermission::Never, ctx);
            profiles.set_active_profile(terminal_view_id, profile_id, ctx);
        });

        let result = executor.update(&mut app, |executor, ctx| {
            let input = ExecuteActionInput {
                action: &action,
                conversation_id,
            };
            executor.should_autoexecute(input, ctx)
        });

        assert!(result);
    });
}

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::anyhow;
use warp_multi_agent_api::client_action::Action;
use warp_multi_agent_api::response_event::stream_finished;
use warpui::{App, Entity, SingletonEntity};

use super::*;
use crate::ai::agent::conversation::ConversationStatus;
use crate::ai::agent::task::TaskId;
use crate::ai::agent::{AIAgentContext, AIAgentInput, UserQueryMode};
use crate::auth::AuthStateProvider;
use crate::server::server_api::AIApiError;
use crate::server::telemetry::context_provider::AppTelemetryContextProvider;
use crate::test_util::settings::initialize_history_persistence_for_tests;

struct TestDelegate;

impl Entity for TestDelegate {
    type Event = ();
}

impl AgentConversationEngineDelegate for TestDelegate {
    fn skill_path_origin(&self, _ctx: &AppContext) -> SkillPathOrigin {
        SkillPathOrigin::Local
    }
}

fn server_root_task(task_id: &TaskId) -> warp_multi_agent_api::Task {
    warp_multi_agent_api::Task {
        id: task_id.to_string(),
        messages: vec![],
        dependencies: None,
        description: "test root task".to_string(),
        summary: String::new(),
        server_data: String::new(),
    }
}

fn user_query_input(query: &str) -> AIAgentInput {
    AIAgentInput::UserQuery {
        query: query.to_owned(),
        context: Arc::<[AIAgentContext]>::from([]),
        static_query_type: None,
        referenced_attachments: HashMap::new(),
        user_query_mode: UserQueryMode::Normal,
        running_command: None,
        intended_agent: None,
    }
}

fn request_input(conversation_id: AIConversationId, task_id: TaskId, query: &str) -> RequestInput {
    RequestInput {
        conversation_id,
        input_messages: HashMap::from([(task_id, vec![user_query_input(query)])]),
        working_directory: None,
        model_id: "test-model".into(),
        coding_model_id: "test-coding-model".into(),
        cli_agent_model_id: "test-cli-model".into(),
        computer_use_model_id: "test-computer-use-model".into(),
        shared_session_response_initiator: None,
        request_start_ts: chrono::Local::now(),
        supported_tools_override: None,
    }
}

fn start_pending_request(
    app: &mut App,
    owner_id: AgentSessionOwnerId,
) -> (AIConversationId, ResponseStreamId, TaskId) {
    let history = app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
    let conversation_id = history.update(app, |history_model, ctx| {
        history_model.start_new_conversation(owner_id.entity_id(), false, false, false, ctx)
    });
    let task_id = history.read(app, |history_model, _| {
        history_model
            .conversation(&conversation_id)
            .expect("conversation should exist")
            .get_root_task_id()
            .clone()
    });
    let stream_id = ResponseStreamId::new_for_test();
    history.update(app, |history_model, ctx| {
        history_model
            .update_conversation_for_new_request_input(
                request_input(conversation_id, task_id.clone(), "hello"),
                stream_id.clone(),
                owner_id.entity_id(),
                ctx,
            )
            .expect("request input should update history");
    });
    (conversation_id, stream_id, task_id)
}

fn agent_output_message(task_id: &TaskId) -> warp_multi_agent_api::Message {
    warp_multi_agent_api::Message {
        id: "agent-message-1".to_owned(),
        task_id: task_id.to_string(),
        server_message_data: String::new(),
        citations: vec![],
        message: Some(warp_multi_agent_api::message::Message::AgentOutput(
            warp_multi_agent_api::message::AgentOutput {
                text: "hello from agent".to_owned(),
            },
        )),
        request_id: "request-1".to_owned(),
        timestamp: None,
    }
}

#[test]
fn folds_init_client_actions_and_finished_into_history() {
    App::test((), |mut app| async move {
        initialize_history_persistence_for_tests(&mut app);
        let owner_id = AgentSessionOwnerId::new(warpui::EntityId::new());
        let (conversation_id, stream_id, task_id) = start_pending_request(&mut app, owner_id);
        let delegate = app.add_model(|_| TestDelegate);

        delegate.update(&mut app, |delegate, ctx| {
            AgentConversationEngine::fold_received_event_for_test(
                delegate,
                owner_id,
                true,
                &stream_id,
                conversation_id,
                Ok(warp_multi_agent_api::ResponseEvent {
                    r#type: Some(response_event::Type::Init(response_event::StreamInit {
                        request_id: "request-1".to_owned(),
                        conversation_id: "server-conversation-1".to_owned(),
                        run_id: String::new(),
                    })),
                }),
                false,
                ctx,
            );
        });
        delegate.update(&mut app, |delegate, ctx| {
            AgentConversationEngine::fold_received_event_for_test(
                delegate,
                owner_id,
                true,
                &stream_id,
                conversation_id,
                Ok(warp_multi_agent_api::ResponseEvent {
                    r#type: Some(response_event::Type::ClientActions(
                        response_event::ClientActions {
                            actions: vec![
                                warp_multi_agent_api::ClientAction {
                                    action: Some(Action::CreateTask(
                                        warp_multi_agent_api::client_action::CreateTask {
                                            task: Some(server_root_task(&task_id)),
                                        },
                                    )),
                                },
                                warp_multi_agent_api::ClientAction {
                                    action: Some(Action::AddMessagesToTask(
                                        warp_multi_agent_api::client_action::AddMessagesToTask {
                                            task_id: task_id.to_string(),
                                            messages: vec![agent_output_message(&task_id)],
                                        },
                                    )),
                                },
                            ],
                        },
                    )),
                }),
                false,
                ctx,
            );
        });
        delegate.update(&mut app, |delegate, ctx| {
            AgentConversationEngine::fold_received_event_for_test(
                delegate,
                owner_id,
                true,
                &stream_id,
                conversation_id,
                Ok(warp_multi_agent_api::ResponseEvent {
                    r#type: Some(response_event::Type::Finished(
                        response_event::StreamFinished {
                            reason: Some(stream_finished::Reason::Done(stream_finished::Done {})),
                            conversation_usage_metadata: None,
                            token_usage: vec![],
                            should_refresh_model_config: false,
                            request_cost: None,
                        },
                    )),
                }),
                false,
                ctx,
            );
        });

        BlocklistAIHistoryModel::handle(&app).read(&app, |history_model, _| {
            let conversation = history_model
                .conversation(&conversation_id)
                .expect("conversation should exist");
            assert_eq!(conversation.status(), &ConversationStatus::Success);
            assert_eq!(
                conversation
                    .server_conversation_token()
                    .map(|token| token.as_str()),
                Some("server-conversation-1")
            );
            let exchange = conversation
                .root_task_exchanges()
                .last()
                .expect("exchange should exist");
            assert!(exchange.output_status.is_finished_and_successful());
            assert_eq!(
                exchange
                    .output_status
                    .output()
                    .expect("output should exist")
                    .get()
                    .messages
                    .len(),
                1
            );
        });
    });
}

#[test]
fn folds_stream_error_into_history() {
    App::test((), |mut app| async move {
        initialize_history_persistence_for_tests(&mut app);
        app.add_singleton_model(|_| AuthStateProvider::new_for_test());
        app.add_singleton_model(AppTelemetryContextProvider::new_context_provider);
        app.add_singleton_model(|_| NetworkStatus::new());
        let owner_id = AgentSessionOwnerId::new(warpui::EntityId::new());
        let (conversation_id, stream_id, _) = start_pending_request(&mut app, owner_id);
        let delegate = app.add_model(|_| TestDelegate);

        delegate.update(&mut app, |delegate, ctx| {
            AgentConversationEngine::fold_received_event_for_test(
                delegate,
                owner_id,
                true,
                &stream_id,
                conversation_id,
                Err(Arc::new(AIApiError::Other(anyhow!("boom")))),
                false,
                ctx,
            );
        });

        BlocklistAIHistoryModel::handle(&app).read(&app, |history_model, _| {
            let conversation = history_model
                .conversation(&conversation_id)
                .expect("conversation should exist");
            assert_eq!(conversation.status(), &ConversationStatus::Error);
        });
    });
}

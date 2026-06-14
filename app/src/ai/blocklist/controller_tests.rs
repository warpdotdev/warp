use std::collections::HashMap;

use chrono::Local;
use uuid::Uuid;
use warpui::{App, SingletonEntity};

use crate::ai::agent::task::TaskId;
use crate::ai::agent::{
    api, AIAgentAttachment, AIAgentContext, AIAgentInput, ImageContext, PassiveSuggestionTrigger,
    ReceivedMessageInput, UserQueryMode,
};
use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::ai::blocklist::{
    BlocklistAIHistoryModel, PendingAttachment, PendingFile, RequestInput, SessionContext,
};
use crate::ai::llms::LLMId;
use crate::persistence::model::PendingConversationHandoff;
use crate::test_util::terminal::{add_window_with_terminal, initialize_app_for_terminal_view};

fn new_ambient_agent_task_id() -> AmbientAgentTaskId {
    Uuid::new_v4().to_string().parse().unwrap()
}

fn request_input_for_test(input: AIAgentInput) -> RequestInput {
    let model = LLMId::from("test-model");
    RequestInput {
        conversation_id: crate::ai::agent::conversation::AIConversationId::new(),
        input_messages: HashMap::from([(TaskId::new("test-task".to_owned()), vec![input])]),
        working_directory: None,
        model_id: model.clone(),
        coding_model_id: model.clone(),
        cli_agent_model_id: model.clone(),
        computer_use_model_id: model,
        shared_session_response_initiator: None,
        request_start_ts: Local::now(),
        supported_tools_override: None,
    }
}

fn normal_user_query_input() -> AIAgentInput {
    AIAgentInput::UserQuery {
        query: "continue".to_owned(),
        context: vec![].into(),
        static_query_type: None,
        referenced_attachments: HashMap::new(),
        user_query_mode: UserQueryMode::Normal,
        running_command: None,
        intended_agent: None,
    }
}

#[test]
fn request_params_snapshots_pending_handoff_for_user_inputs_requests() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        app.update(|ctx| {
            let user_query_request = request_input_for_test(normal_user_query_input());
            let inter_agent_message_request =
                request_input_for_test(AIAgentInput::MessagesReceivedFromAgents {
                    messages: vec![ReceivedMessageInput {
                        message_id: "message-id".to_owned(),
                        sender_agent_id: "agent-id".to_owned(),
                        addresses: vec!["recipient-id".to_owned()],
                        subject: "subject".to_owned(),
                        message_body: "body".to_owned(),
                    }],
                });
            let top_level_request = request_input_for_test(AIAgentInput::SummarizeConversation {
                prompt: None,
                context: vec![].into(),
            });
            let conversation = api::ConversationData {
                id: user_query_request.conversation_id,
                tasks: vec![],
                server_conversation_token: None,
                forked_from_conversation_token: None,
                pending_conversation_handoff: Some(PendingConversationHandoff::CloudToLocal),
                ambient_agent_task_id: None,
                existing_suggestions: None,
            };

            let user_query_params = api::RequestParams::new(
                None,
                SessionContext::new_for_test(),
                &user_query_request,
                conversation.clone(),
                None,
                ctx,
            );
            assert_eq!(
                user_query_params.pending_conversation_handoff,
                Some(PendingConversationHandoff::CloudToLocal),
            );

            let inter_agent_message_params = api::RequestParams::new(
                None,
                SessionContext::new_for_test(),
                &inter_agent_message_request,
                conversation.clone(),
                None,
                ctx,
            );
            assert_eq!(
                inter_agent_message_params.pending_conversation_handoff,
                Some(PendingConversationHandoff::CloudToLocal),
            );
            let top_level_params = api::RequestParams::new(
                None,
                SessionContext::new_for_test(),
                &top_level_request,
                conversation,
                None,
                ctx,
            );
            assert_eq!(top_level_params.pending_conversation_handoff, None);
        });
    });
}

fn image_attachment(file_name: &str) -> PendingAttachment {
    PendingAttachment::Image(ImageContext {
        data: String::new(),
        mime_type: "image/png".to_owned(),
        file_name: file_name.to_owned(),
        is_figma: false,
    })
}

fn file_attachment(file_name: &str) -> PendingAttachment {
    PendingAttachment::File(PendingFile {
        file_name: file_name.to_owned(),
        file_path: file_name.into(),
        mime_type: "text/plain".to_owned(),
    })
}

#[test]
fn passive_suggestions_request_params_omit_ambient_agent_task_id() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |terminal, ctx| {
            let task_id = new_ambient_agent_task_id();
            let conversation_id =
                BlocklistAIHistoryModel::handle(ctx).update(ctx, |history_model, ctx| {
                    history_model.start_new_conversation(terminal.id(), false, false, false, ctx)
                });

            terminal.ai_controller().update(ctx, |controller, ctx| {
                controller.set_ambient_agent_task_id(Some(task_id), ctx);

                assert_eq!(controller.get_ambient_agent_task_id(), Some(task_id));
                assert_eq!(
                    controller
                        .build_passive_suggestions_request_params(
                            Some(conversation_id),
                            PassiveSuggestionTrigger::FilesChanged,
                            vec![],
                            ctx,
                        )
                        .expect("existing conversation should build passive suggestion params")
                        .1
                        .ambient_agent_task_id,
                    None
                );
                assert_eq!(
                    controller
                        .build_passive_suggestions_request_params(
                            None,
                            PassiveSuggestionTrigger::FilesChanged,
                            vec![],
                            ctx,
                        )
                        .expect("new conversation should build passive suggestion params")
                        .1
                        .ambient_agent_task_id,
                    None
                );
            });
        });
    });
}

#[test]
fn input_for_query_converts_prompt_attachments_and_ignores_live_staging() {
    // `input_for_query` builds its image/file context purely from the explicitly-provided
    // attachment set (resolved by `send_query` from either the queued row or live staging),
    // never from the context model's pending attachments.
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |terminal, ctx| {
            let conversation_id =
                BlocklistAIHistoryModel::handle(ctx).update(ctx, |history_model, ctx| {
                    history_model.start_new_conversation(terminal.id(), false, false, false, ctx)
                });

            let controller = terminal.ai_controller();
            let context_model = controller.as_ref(ctx).context_model.clone();
            let active_session = controller.as_ref(ctx).active_session.clone();

            // Stage *live* attachments that must NOT leak into a query built from a different,
            // explicitly-provided attachment set.
            context_model.update(ctx, |m, ctx| {
                m.append_pending_attachments(
                    vec![image_attachment("live.png"), file_attachment("live.txt")],
                    ctx,
                );
            });

            let task_id = TaskId::new("test-task".to_owned());
            // Two files sharing a basename to exercise duplicate-basename suffixing.
            let prompt_attachments = vec![
                image_attachment("queued.png"),
                file_attachment("notes.txt"),
                file_attachment("notes.txt"),
            ];

            let input = super::input_for_query(
                "build a query".to_owned(),
                &task_id,
                conversation_id,
                None,
                UserQueryMode::Normal,
                None,
                HashMap::new(),
                prompt_attachments,
                context_model.as_ref(ctx),
                active_session.as_ref(ctx),
                ctx,
            );

            let AIAgentInput::UserQuery {
                context,
                referenced_attachments,
                ..
            } = input
            else {
                panic!("expected UserQuery");
            };

            // The provided image is attached as image context; the live-staged image is not.
            let image_names: Vec<&str> = context
                .iter()
                .filter_map(|c| match c {
                    AIAgentContext::Image(img) => Some(img.file_name.as_str()),
                    _ => None,
                })
                .collect();
            assert_eq!(image_names, vec!["queued.png"]);

            // The provided files are attached as FilePathReference with duplicate-basename
            // suffixing; the live-staged file is not.
            let mut file_names: Vec<String> = referenced_attachments
                .values()
                .filter_map(|a| match a {
                    AIAgentAttachment::FilePathReference { file_name, .. } => {
                        Some(file_name.clone())
                    }
                    _ => None,
                })
                .collect();
            file_names.sort();
            assert_eq!(
                file_names,
                vec!["notes.txt".to_owned(), "notes.txt".to_owned()]
            );
            assert!(referenced_attachments.contains_key("notes.txt"));
            assert!(referenced_attachments.contains_key("notes.txt (1)"));
            assert!(!referenced_attachments.contains_key("live.txt"));
        });
    });
}

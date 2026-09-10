use std::collections::HashMap;

use session_sharing_protocol::common::{AgentAttachment, ParticipantId};
use uuid::Uuid;
use warp_core::features::FeatureFlag;
use warp_multi_agent_api::response_event::StreamInit;
use warpui::{App, ModelContext, SingletonEntity};

use super::*;
use crate::ai::blocklist::agent_view::AgentViewEntryOrigin;
use crate::ai::blocklist::{BlocklistAIControllerEvent, QueuedQueryOrigin};
use crate::test_util::terminal::{add_window_with_terminal, initialize_app_for_terminal_view};

fn finish_turn(
    controller: &mut BlocklistAIController,
    conversation_id: AIConversationId,
    ctx: &mut ModelContext<BlocklistAIController>,
) {
    let streams = controller
        .in_flight_response_streams
        .stream_ids_for_conversation(conversation_id, ctx);
    assert_eq!(streams.len(), 1);
    let stream_id = streams[0].clone();
    BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
        let server_token = history
            .conversation(&conversation_id)
            .unwrap()
            .server_conversation_token()
            .map(|token| token.as_str().to_owned())
            .unwrap_or_else(|| "550e8400-e29b-41d4-a716-446655440a00".into());
        history.initialize_output_for_response_stream(
            &stream_id,
            conversation_id,
            controller.terminal_surface_id,
            StreamInit {
                request_id: "test-request".into(),
                conversation_id: server_token,
                run_id: String::new(),
            },
            ctx,
        );
        history.mark_response_stream_completed_successfully(
            &stream_id,
            conversation_id,
            controller.terminal_surface_id,
            ctx,
        );
    });
    controller
        .in_flight_response_streams
        .cleanup_stream(&stream_id);
    ctx.emit(BlocklistAIControllerEvent::FinishedReceivingOutput {
        stream_id,
        conversation_id,
    });
}

#[test]
fn startup_injections_continue_after_each_turn_without_losing_the_middle_prompt() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let _queue_v2 = FeatureFlag::QueuedPromptsV2.override_enabled(false);
        let terminal = add_window_with_terminal(&mut app, None);
        let controller = terminal.read(&app, |terminal, _| terminal.ai_controller().clone());
        let participant = ParticipantId::new();
        let id = controller.update(&mut app, |controller, ctx| {
            let id = controller.prepare_native_prompt_queue(None, ctx);
            controller.execute_warp_agent_prompt_from_shared_session_injection(
                "prompt2".into(),
                None,
                vec![],
                participant.clone(),
                ctx,
            );
            controller.execute_warp_agent_prompt_from_shared_session_injection(
                "prompt3".into(),
                None,
                vec![],
                participant.clone(),
                ctx,
            );
            assert_eq!(
                BlocklistAIHistoryModel::as_ref(ctx)
                    .conversation(&id)
                    .unwrap()
                    .exchange_count(),
                0
            );
            assert!(QueuedQueryModel::as_ref(ctx).peek_autofire(id).is_none());
            id
        });
        terminal.update(&mut app, |terminal, ctx| {
            terminal.enter_agent_view(None, Some(id), AgentViewEntryOrigin::Cli, ctx);
        });
        controller.update(&mut app, |controller, ctx| {
            controller.send_user_query_in_conversation("prompt1".into(), id, None, ctx);
        });
        terminal.update(&mut app, |terminal, ctx| {
            terminal.input().update(ctx, |input, ctx| {
                input.replace_buffer_content("local draft", ctx)
            });
        });
        controller.update(&mut app, |controller, ctx| finish_turn(controller, id, ctx));
        QueuedQueryModel::handle(&app).read(&app, |queue, _| {
            assert_eq!(
                queue
                    .queue(id)
                    .iter()
                    .map(QueuedQuery::text)
                    .collect::<Vec<_>>(),
                vec!["prompt3"]
            );
        });
        controller.update(&mut app, |controller, ctx| finish_turn(controller, id, ctx));
        BlocklistAIHistoryModel::handle(&app).read(&app, |history, _| {
            let conversation = history.conversation(&id).unwrap();
            let prompts: Vec<_> = conversation
                .root_task_exchanges()
                .flat_map(|exchange| exchange.input.iter())
                .filter_map(|input| match input {
                    AIAgentInput::UserQuery { query, .. } => Some(query.as_str()),
                    _ => None,
                })
                .collect();
            assert_eq!(prompts, vec!["prompt1", "prompt2", "prompt3"]);
        });
        QueuedQueryModel::handle(&app).read(&app, |queue, _| assert!(!queue.has_queue(id)));
        terminal.read(&app, |terminal, ctx| {
            assert_eq!(terminal.input().as_ref(ctx).buffer_text(ctx), "local draft")
        });
    });
}

#[test]
fn injections_during_initial_turn_queue_without_interrupting_it() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let terminal = add_window_with_terminal(&mut app, None);
        let controller = terminal.read(&app, |terminal, _| terminal.ai_controller().clone());
        let id = controller.update(&mut app, |controller, ctx| {
            controller.prepare_native_prompt_queue(None, ctx)
        });
        terminal.update(&mut app, |terminal, ctx| {
            terminal.enter_agent_view(None, Some(id), AgentViewEntryOrigin::Cli, ctx);
        });
        let token = ServerConversationToken::from_uuid(Uuid::new_v4());
        let initial_streams = controller.update(&mut app, |controller, ctx| {
            controller.send_user_query_in_conversation("prompt1".into(), id, None, ctx);
            let streams = controller
                .in_flight_response_streams
                .stream_ids_for_conversation(id, ctx);
            BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                history.initialize_output_for_response_stream(
                    &streams[0],
                    id,
                    controller.terminal_surface_id,
                    StreamInit {
                        request_id: "initial-request".into(),
                        conversation_id: token.to_string(),
                        run_id: String::new(),
                    },
                    ctx,
                );
            });
            assert!(!QueuedQueryModel::as_ref(ctx).has_queue(id));
            streams
        });
        controller.update(&mut app, |controller, ctx| {
            let participant = ParticipantId::new();
            controller.execute_warp_agent_prompt_from_shared_session_injection(
                "prompt2".into(),
                Some(token),
                vec![],
                participant.clone(),
                ctx,
            );
            controller.execute_warp_agent_prompt_from_shared_session_injection(
                "prompt3".into(),
                Some(token),
                vec![],
                participant,
                ctx,
            );
            assert_eq!(
                controller
                    .in_flight_response_streams
                    .stream_ids_for_conversation(id, ctx),
                initial_streams,
                "startup injections must not cancel the initial stream",
            );
            assert_eq!(
                QueuedQueryModel::as_ref(ctx)
                    .queue(id)
                    .iter()
                    .map(QueuedQuery::text)
                    .collect::<Vec<_>>(),
                vec!["prompt2", "prompt3"]
            );
        });
        controller.update(&mut app, |controller, ctx| finish_turn(controller, id, ctx));
        QueuedQueryModel::handle(&app).read(&app, |queue, _| {
            assert_eq!(
                queue
                    .queue(id)
                    .iter()
                    .map(QueuedQuery::text)
                    .collect::<Vec<_>>(),
                vec!["prompt3"]
            );
        });
        controller.update(&mut app, |controller, ctx| finish_turn(controller, id, ctx));
        BlocklistAIHistoryModel::handle(&app).read(&app, |history, _| {
            let prompts: Vec<_> = history
                .conversation(&id)
                .unwrap()
                .root_task_exchanges()
                .flat_map(|exchange| exchange.input.iter())
                .filter_map(|input| match input {
                    AIAgentInput::UserQuery { query, .. } => Some(query.as_str()),
                    _ => None,
                })
                .collect();
            assert_eq!(prompts, vec!["prompt1", "prompt2", "prompt3"]);
        });
    });
}

#[test]
fn queued_injection_context_keeps_attribution_and_does_not_consume_live_staging() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        terminal.update(&mut app, |terminal, ctx| {
            terminal.ai_controller().update(ctx, |controller, ctx| {
                let id = controller.prepare_native_prompt_queue(None, ctx);
                controller.context_model.update(ctx, |context, ctx| {
                    context.set_pending_context_selected_text(
                        Some("local selection".into()),
                        false,
                        ctx,
                    );
                });
                let participant = ParticipantId::new();
                let row = QueuedQuery::new_shared_session_prompt(
                    "prompt2".into(),
                    participant.clone(),
                    vec![AgentAttachment::PlainText {
                        content: "remote context".into(),
                    }],
                );
                let request = controller
                    .queued_injection_request(id, &row, HashMap::new(), ctx)
                    .unwrap();
                assert_eq!(request.conversation_id, id);
                assert_eq!(request.shared_session_response_initiator, Some(participant));
                let AIAgentInput::UserQuery { context, .. } = request.all_inputs().next().unwrap()
                else {
                    panic!("expected prompt")
                };
                let selected: Vec<_> = context
                    .iter()
                    .filter_map(|context| match context {
                        AIAgentContext::SelectedText(text) => Some(text.as_str()),
                        _ => None,
                    })
                    .collect();
                assert_eq!(selected, vec!["remote context"]);
                assert_eq!(
                    controller
                        .context_model
                        .as_ref(ctx)
                        .pending_context_selected_text()
                        .map(String::as_str),
                    Some("local selection")
                );
            });
        });
    });
}

#[test]
fn startup_injections_reuse_the_restored_conversation() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        terminal.update(&mut app, |terminal, ctx| {
            let restored = BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                history.start_new_conversation(terminal.id(), false, false, false, ctx)
            });
            terminal.ai_controller().update(ctx, |controller, ctx| {
                assert_eq!(
                    controller.prepare_native_prompt_queue(Some(restored), ctx),
                    restored
                );
                assert_eq!(controller.prepare_native_prompt_queue(None, ctx), restored);
                controller.execute_warp_agent_prompt_from_shared_session_injection(
                    "followup".into(),
                    None,
                    vec![],
                    ParticipantId::new(),
                    ctx,
                );
                let row = &QueuedQueryModel::as_ref(ctx).queue(restored)[0];
                assert_eq!(row.text(), "followup");
                assert_eq!(row.origin(), QueuedQueryOrigin::SharedSessionInjection);
                assert_eq!(
                    BlocklistAIHistoryModel::as_ref(ctx)
                        .conversation(&restored)
                        .unwrap()
                        .exchange_count(),
                    0
                );
            });
        });
    });
}

#[test]
fn startup_injection_retains_a_token_until_the_initial_response_binds_it() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        terminal.update(&mut app, |terminal, ctx| {
            terminal.ai_controller().update(ctx, |controller, ctx| {
                let id = controller.prepare_native_prompt_queue(None, ctx);
                let token = ServerConversationToken::from_uuid(Uuid::new_v4());
                controller.execute_warp_agent_prompt_from_shared_session_injection(
                    "followup".into(),
                    Some(token),
                    vec![],
                    ParticipantId::new(),
                    ctx,
                );
                let row = QueuedQueryModel::as_ref(ctx).queue(id)[0].clone();
                assert_eq!(row.shared_session_target(), Some(&token));
                assert!(
                    controller
                        .queued_injection_request(id, &row, HashMap::new(), ctx)
                        .is_err()
                );
                BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, _| {
                    history.set_server_conversation_token_for_conversation(id, token.to_string());
                });
                assert_eq!(
                    controller
                        .queued_injection_request(id, &row, HashMap::new(), ctx)
                        .unwrap()
                        .conversation_id,
                    id
                );
            });
        });
    });
}

#[test]
fn native_initial_prompt_uses_its_bound_conversation_without_agent_view() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(false);
        let terminal = add_window_with_terminal(&mut app, None);
        terminal.update(&mut app, |terminal, ctx| {
            terminal.ai_controller().update(ctx, |controller, ctx| {
                let id = controller.prepare_native_prompt_queue(None, ctx);
                controller.execute_warp_agent_prompt_from_shared_session_injection(
                    "followup".into(),
                    None,
                    vec![],
                    ParticipantId::new(),
                    ctx,
                );
                controller.send_ai_input_with_context(
                    |context| AIAgentInput::StartFromAmbientRunPrompt {
                        ambient_run_id: "550e8400-e29b-41d4-a716-446655440a00".into(),
                        context,
                        runtime_skill: None,
                        attachments_dir: None,
                    },
                    ctx,
                );
                assert_eq!(
                    BlocklistAIHistoryModel::as_ref(ctx)
                        .conversation(&id)
                        .unwrap()
                        .exchange_count(),
                    1
                );
                assert!(QueuedQueryModel::as_ref(ctx).is_dispatch_blocked(id));
                assert_eq!(QueuedQueryModel::as_ref(ctx).queue(id).len(), 1);
            });
        });
    });
}

#[test]
fn stopped_worker_cannot_dispatch_a_downloaded_prompt() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        terminal.update(&mut app, |terminal, ctx| {
            terminal.ai_controller().update(ctx, |controller, ctx| {
                let id = controller.prepare_native_prompt_queue(None, ctx);
                let row = QueuedQuery::new_shared_session_prompt(
                    "pending".into(),
                    ParticipantId::new(),
                    vec![],
                );
                let query_id = row.id();
                QueuedQueryModel::handle(ctx).update(ctx, |queue, ctx| {
                    queue.append(id, row.clone(), ctx);
                    queue.finish_native_setup(id, ctx);
                    queue.finish_native_initial_turn(id, ctx);
                    assert!(queue.claim_injection(id, query_id, ctx).is_some());
                });
                controller.stop_native_prompt_queue(ctx);
                controller.finish_queued_injection(id, row, Ok(HashMap::new()), ctx);
                assert_eq!(
                    BlocklistAIHistoryModel::as_ref(ctx)
                        .conversation(&id)
                        .unwrap()
                        .exchange_count(),
                    0
                );
                assert_eq!(QueuedQueryModel::as_ref(ctx).queue(id).len(), 1);
            });
        });
    });
}

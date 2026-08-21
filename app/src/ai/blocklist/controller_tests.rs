use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use chrono::Local;
use uuid::Uuid;
use warp_multi_agent_api::response_event;
use warpui::{App, SingletonEntity};

use super::response_stream::{PendingResume, RecoveryBudget};
use crate::ai::agent::conversation::{AIConversationId, ConversationStatus};
use crate::ai::agent::task::TaskId;
use crate::ai::agent::{
    AIAgentAction, AIAgentActionId, AIAgentActionResult, AIAgentActionResultType,
    AIAgentActionType, AIAgentAttachment, AIAgentContext, AIAgentInput, CancellationReason,
    FetchConversationResult, ImageContext, PassiveSuggestionTrigger, UserQueryMode,
};
use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::ai::blocklist::{
    BlocklistAIHistoryEvent, BlocklistAIHistoryModel, PendingAttachment, PendingFile, RequestInput,
    ResponseStream, ResponseStreamId,
};
use crate::ai::llms::LLMId;
use crate::test_util::terminal::{add_window_with_terminal, initialize_app_for_terminal_view};

fn pending_fetch_conversation_action(task_id: TaskId) -> AIAgentAction {
    AIAgentAction {
        id: AIAgentActionId::from("fetch-conversation-1".to_owned()),
        task_id,
        action: AIAgentActionType::FetchConversation {
            conversation_id: "target-conversation".to_owned(),
        },
        requires_result: true,
    }
}

fn new_ambient_agent_task_id() -> AmbientAgentTaskId {
    Uuid::new_v4().to_string().parse().unwrap()
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

#[test]
fn cancelling_conversation_aborts_pending_auto_resume() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);

        // An ID with no backing conversation: if the scheduled wait ever
        // completes, the resume is a harmless no-op.
        let conversation_id = AIConversationId::new();

        terminal.update(&mut app, |terminal, ctx| {
            terminal.ai_controller().update(ctx, |controller, ctx| {
                let resume = PendingResume::new_for_test(
                    RecoveryBudget::fresh().next_attempt(),
                    std::time::Duration::from_millis(1),
                );
                controller.schedule_auto_resume_after_error(conversation_id, resume, ctx);
                assert!(
                    controller
                        .pending_auto_resume_handles
                        .contains_key(&conversation_id)
                );

                controller.cancel_conversation_progress(
                    conversation_id,
                    CancellationReason::ManuallyCancelled,
                    ctx,
                );
                assert!(
                    !controller
                        .pending_auto_resume_handles
                        .contains_key(&conversation_id)
                );
            });
        });
    });
}

#[test]
fn mock_response_stream_updates_history_through_controller() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        let captured_events = Arc::new(Mutex::new(Vec::new()));
        let events_for_subscription = Arc::clone(&captured_events);
        app.update(|ctx| {
            ctx.subscribe_to_model(&BlocklistAIHistoryModel::handle(ctx), move |_, event, _| {
                events_for_subscription.lock().unwrap().push(event.clone())
            });
        });

        let (conversation_id, stream) = terminal.update(&mut app, |view, ctx| {
            let terminal_surface_id = view.id();
            let stream_id = ResponseStreamId::new_for_test();
            let conversation_id =
                BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                    let conversation_id = history.start_new_conversation(
                        terminal_surface_id,
                        false,
                        false,
                        false,
                        ctx,
                    );
                    let task_id = history
                        .conversation(&conversation_id)
                        .unwrap()
                        .get_root_task_id()
                        .clone();
                    history
                        .update_conversation_for_new_request_input(
                            RequestInput {
                                conversation_id,
                                input_messages: HashMap::from([(task_id, vec![])]),
                                working_directory: None,
                                model_id: LLMId::from("test-model"),
                                coding_model_id: LLMId::from("test-coding-model"),
                                cli_agent_model_id: LLMId::from("test-cli-agent-model"),
                                computer_use_model_id: LLMId::from("test-computer-use-model"),
                                shared_session_response_initiator: None,
                                request_start_ts: Local::now(),
                                supported_tools_override: None,
                            },
                            stream_id.clone(),
                            terminal_surface_id,
                            ctx,
                        )
                        .unwrap();
                    conversation_id
                });
            let stream = ctx.add_model(|_| ResponseStream::new_for_test(stream_id.clone()));
            view.ai_controller().update(ctx, |controller, ctx| {
                controller.register_mock_stream_for_test(
                    stream_id,
                    conversation_id,
                    stream.clone(),
                    ctx,
                );
            });
            (conversation_id, stream)
        });

        stream.update(&mut app, |stream, ctx| {
            stream.emit_response_event_for_test(
                warp_multi_agent_api::ResponseEvent {
                    r#type: Some(response_event::Type::Init(response_event::StreamInit {
                        request_id: "test-request".to_string(),
                        conversation_id: "test-server-conversation".to_string(),
                        run_id: String::new(),
                    })),
                },
                ctx,
            );
            stream.emit_response_event_for_test(
                warp_multi_agent_api::ResponseEvent {
                    r#type: Some(response_event::Type::Finished(
                        response_event::StreamFinished {
                            reason: Some(response_event::stream_finished::Reason::Done(
                                response_event::stream_finished::Done {},
                            )),
                            conversation_usage_metadata: None,
                            token_usage: vec![],
                            should_refresh_model_config: false,
                            #[allow(deprecated)]
                            request_cost: None,
                            request_charges: None,
                        },
                    )),
                },
                ctx,
            );
        });

        BlocklistAIHistoryModel::handle(&app).read(&app, |history, _| {
            assert_eq!(
                history.conversation(&conversation_id).map(|c| c.status()),
                Some(&crate::ai::agent::conversation::ConversationStatus::Success)
            );
        });
        let events = captured_events.lock().unwrap();
        assert!(events.iter().any(|event| matches!(
            event,
            BlocklistAIHistoryEvent::ConversationServerTokenAssigned {
                conversation_id: id,
                ..
            } if *id == conversation_id
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            BlocklistAIHistoryEvent::UpdatedStreamingExchange {
                conversation_id: id,
                ..
            } if *id == conversation_id
        )));
    });
}

/// When an agent command exits the shell, the conversation must be finalized as
/// `Error` (not `Cancelled`), and a subsequent `ManuallyCancelled` (as fired by
/// the pane-close path) must not overwrite that failure.
#[test]
fn fail_conversation_due_to_shell_exit_reports_error_and_survives_manual_cancel() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);

        let conversation_id = terminal.update(&mut app, |view, ctx| {
            let terminal_surface_id = view.id();
            let stream_id = ResponseStreamId::new_for_test();
            let conversation_id =
                BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                    let conversation_id = history.start_new_conversation(
                        terminal_surface_id,
                        false,
                        false,
                        false,
                        ctx,
                    );
                    let task_id = history
                        .conversation(&conversation_id)
                        .unwrap()
                        .get_root_task_id()
                        .clone();
                    history
                        .update_conversation_for_new_request_input(
                            RequestInput {
                                conversation_id,
                                input_messages: HashMap::from([(task_id, vec![])]),
                                working_directory: None,
                                model_id: LLMId::from("test-model"),
                                coding_model_id: LLMId::from("test-coding-model"),
                                cli_agent_model_id: LLMId::from("test-cli-agent-model"),
                                computer_use_model_id: LLMId::from("test-computer-use-model"),
                                shared_session_response_initiator: None,
                                request_start_ts: Local::now(),
                                supported_tools_override: None,
                            },
                            stream_id.clone(),
                            terminal_surface_id,
                            ctx,
                        )
                        .unwrap();
                    conversation_id
                });
            let stream = ctx.add_model(|_| ResponseStream::new_for_test(stream_id.clone()));
            view.ai_controller().update(ctx, |controller, ctx| {
                controller.register_mock_stream_for_test(stream_id, conversation_id, stream, ctx);
                controller.fail_conversation_due_to_shell_exit(
                    conversation_id,
                    "exit 1".to_string(),
                    ctx,
                );
            });
            conversation_id
        });

        // The in-flight request is finalized as Error (with the shell-exit error
        // on its exchange), not Cancelled.
        BlocklistAIHistoryModel::handle(&app).read(&app, |history, _| {
            assert_eq!(
                history.conversation(&conversation_id).map(|c| c.status()),
                Some(&crate::ai::agent::conversation::ConversationStatus::Error)
            );
        });

        // The pane-close cancellation path must be a no-op now that the
        // conversation is terminal.
        terminal.update(&mut app, |view, ctx| {
            view.ai_controller().update(ctx, |controller, ctx| {
                controller.cancel_conversation_progress(
                    conversation_id,
                    CancellationReason::ManuallyCancelled,
                    ctx,
                );
            });
        });
        BlocklistAIHistoryModel::handle(&app).read(&app, |history, _| {
            assert_eq!(
                history.conversation(&conversation_id).map(|c| c.status()),
                Some(&crate::ai::agent::conversation::ConversationStatus::Error)
            );
        });
    });
}

/// An optimistic long-running-command completion that cancels an in-flight
/// stream must finalize the conversation as `Success`, not `Cancelled`. This is
/// a regression test for the reason -> status mapping living in a single place
/// (`CancellationReason::conversation_outcome`).
#[test]
fn optimistic_cli_subagent_completion_with_in_flight_stream_reports_success() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);

        let conversation_id = terminal.update(&mut app, |view, ctx| {
            let terminal_surface_id = view.id();
            let stream_id = ResponseStreamId::new_for_test();
            let conversation_id =
                BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                    let conversation_id = history.start_new_conversation(
                        terminal_surface_id,
                        false,
                        false,
                        false,
                        ctx,
                    );
                    let task_id = history
                        .conversation(&conversation_id)
                        .unwrap()
                        .get_root_task_id()
                        .clone();
                    history
                        .update_conversation_for_new_request_input(
                            RequestInput {
                                conversation_id,
                                input_messages: HashMap::from([(task_id, vec![])]),
                                working_directory: None,
                                model_id: LLMId::from("test-model"),
                                coding_model_id: LLMId::from("test-coding-model"),
                                cli_agent_model_id: LLMId::from("test-cli-agent-model"),
                                computer_use_model_id: LLMId::from("test-computer-use-model"),
                                shared_session_response_initiator: None,
                                request_start_ts: Local::now(),
                                supported_tools_override: None,
                            },
                            stream_id.clone(),
                            terminal_surface_id,
                            ctx,
                        )
                        .unwrap();
                    conversation_id
                });
            let stream = ctx.add_model(|_| ResponseStream::new_for_test(stream_id.clone()));
            view.ai_controller().update(ctx, |controller, ctx| {
                controller.register_mock_stream_for_test(stream_id, conversation_id, stream, ctx);
                // The long-running command finished while the agent was still
                // streaming, cancelling the in-flight stream optimistically.
                controller.cancel_conversation_progress(
                    conversation_id,
                    CancellationReason::CommandFinishedDuringInlineAgentView,
                    ctx,
                );
            });
            conversation_id
        });

        BlocklistAIHistoryModel::handle(&app).read(&app, |history, _| {
            assert_eq!(
                history.conversation(&conversation_id).map(|c| c.status()),
                Some(&crate::ai::agent::conversation::ConversationStatus::Success)
            );
        });
    });
}

/// `FetchConversation` has no user-facing cancel affordance, so its cancellation is always
/// driven by the generic action-cancellation machinery. A genuine terminal cancellation (e.g.
/// the Stop button, mapped to `ManuallyCancelled`) must not be converted into a new outbound
/// request -- doing so would revive a conversation the user explicitly stopped.
#[test]
fn manual_cancel_of_pending_fetch_conversation_does_not_revive_conversation() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);

        let sent_requests = Arc::new(Mutex::new(0usize));
        let sent_requests_for_subscription = Arc::clone(&sent_requests);

        let conversation_id = terminal.update(&mut app, |view, ctx| {
            let controller_handle = view.ai_controller().clone();
            ctx.subscribe_to_model(&controller_handle, move |_, _, event, _| {
                if matches!(event, super::BlocklistAIControllerEvent::SentRequest { .. }) {
                    *sent_requests_for_subscription.lock().unwrap() += 1;
                }
            });

            let terminal_surface_id = view.id();
            let conversation_id =
                BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                    history.start_new_conversation(terminal_surface_id, false, false, false, ctx)
                });
            let task_id = BlocklistAIHistoryModel::as_ref(ctx)
                .conversation(&conversation_id)
                .unwrap()
                .get_root_task_id()
                .clone();

            view.ai_controller().update(ctx, |controller, ctx| {
                controller.action_model.update(ctx, |action_model, _| {
                    action_model.enqueue_pending_action_for_test(
                        conversation_id,
                        pending_fetch_conversation_action(task_id),
                    );
                });

                controller.cancel_conversation_progress(
                    conversation_id,
                    CancellationReason::ManuallyCancelled,
                    ctx,
                );
            });

            conversation_id
        });

        assert_eq!(*sent_requests.lock().unwrap(), 0);
        BlocklistAIHistoryModel::handle(&app).read(&app, |history, _| {
            assert_eq!(
                history.conversation(&conversation_id).map(|c| c.status()),
                Some(&ConversationStatus::Cancelled)
            );
        });
    });
}

/// Collateral cancellation (a follow-up submitted for the same conversation while the fetch is
/// still resolving) must not mark the conversation `Cancelled` on its own -- unlike a genuine
/// terminal cancellation, its `CancellationOutcome` is `KeepInProgress`, so the conversation is
/// left alone rather than being spuriously finalized while a follow-up is already underway.
#[test]
fn collateral_cancel_of_pending_fetch_conversation_does_not_mark_conversation_cancelled() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);

        let (conversation_id, initial_status, drained) = terminal.update(&mut app, |view, ctx| {
            let terminal_surface_id = view.id();
            let conversation_id =
                BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                    history.start_new_conversation(terminal_surface_id, false, false, false, ctx)
                });
            let task_id = BlocklistAIHistoryModel::as_ref(ctx)
                .conversation(&conversation_id)
                .unwrap()
                .get_root_task_id()
                .clone();
            let initial_status = BlocklistAIHistoryModel::as_ref(ctx)
                .conversation(&conversation_id)
                .map(|c| c.status().clone());

            let drained = view.ai_controller().update(ctx, |controller, ctx| {
                controller.action_model.update(ctx, |action_model, _| {
                    action_model.enqueue_pending_action_for_test(
                        conversation_id,
                        pending_fetch_conversation_action(task_id),
                    );
                });

                controller.cancel_conversation_progress(
                    conversation_id,
                    CancellationReason::FollowUpSubmitted {
                        is_for_same_conversation: true,
                    },
                    ctx,
                );

                controller.action_model.update(ctx, |action_model, _| {
                    action_model.drain_finished_action_results(conversation_id)
                })
            });

            (conversation_id, initial_status, drained)
        });

        // Collateral cancellation must not finalize the conversation locally; that would
        // surface as a spurious "cancelled" state even though the user never cancelled.
        BlocklistAIHistoryModel::handle(&app).read(&app, |history, _| {
            assert_eq!(
                history
                    .conversation(&conversation_id)
                    .map(|c| c.status().clone()),
                initial_status
            );
        });

        // The cancelled result was drained (available to whatever sends the next request
        // for this conversation), rather than left stuck in the pending queue.
        assert_eq!(drained.len(), 1);
        assert!(matches!(
            drained[0].result,
            AIAgentActionResultType::FetchConversation(FetchConversationResult::Cancelled)
        ));
    });
}

/// A fast-resolving action (e.g. a `FetchConversation` fetch served from an already
/// in-memory conversation) can have its `FinishedAction` processed *before* the response
/// stream that dispatched it has finished processing its own `StreamFinished` event --
/// receiving that response event marks the exchange complete but does not yet clear the
/// stream's bookkeeping (see `ConversationStatus`'s `mark_request_completed` vs.
/// `cleanup_completed_response_stream`, only the latter of which runs on
/// `AfterStreamFinished`). `send_follow_up_for_conversation` correctly defers in that
/// window rather than dropping the result, but without a retry, the deferred result would
/// be stranded forever once the window closes. This proves
/// `flush_stranded_follow_up_for_conversation` (called from the `AfterStreamFinished`
/// normal-completion path) recovers it.
#[test]
fn stranded_action_result_is_flushed_once_triggering_stream_completes() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);

        let sent_requests = Arc::new(Mutex::new(0usize));
        let sent_requests_for_subscription = Arc::clone(&sent_requests);

        let (conversation_id, task_id, stream) = terminal.update(&mut app, |view, ctx| {
            let controller_handle = view.ai_controller().clone();
            ctx.subscribe_to_model(&controller_handle, move |_, _, event, _| {
                if matches!(event, super::BlocklistAIControllerEvent::SentRequest { .. }) {
                    *sent_requests_for_subscription.lock().unwrap() += 1;
                }
            });

            let terminal_surface_id = view.id();
            let stream_id = ResponseStreamId::new_for_test();
            let (conversation_id, task_id) =
                BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                    let conversation_id = history.start_new_conversation(
                        terminal_surface_id,
                        false,
                        false,
                        false,
                        ctx,
                    );
                    let task_id = history
                        .conversation(&conversation_id)
                        .unwrap()
                        .get_root_task_id()
                        .clone();
                    history
                        .update_conversation_for_new_request_input(
                            RequestInput {
                                conversation_id,
                                input_messages: HashMap::from([(task_id.clone(), vec![])]),
                                working_directory: None,
                                model_id: LLMId::from("test-model"),
                                coding_model_id: LLMId::from("test-coding-model"),
                                cli_agent_model_id: LLMId::from("test-cli-agent-model"),
                                computer_use_model_id: LLMId::from("test-computer-use-model"),
                                shared_session_response_initiator: None,
                                request_start_ts: Local::now(),
                                supported_tools_override: None,
                            },
                            stream_id.clone(),
                            terminal_surface_id,
                            ctx,
                        )
                        .unwrap();
                    (conversation_id, task_id)
                });
            let stream = ctx.add_model(|_| ResponseStream::new_for_test(stream_id.clone()));
            view.ai_controller().update(ctx, |controller, ctx| {
                controller.register_mock_stream_for_test(
                    stream_id,
                    conversation_id,
                    stream.clone(),
                    ctx,
                );
            });
            (conversation_id, task_id, stream)
        });

        // The response event carrying `Finished(Done)` is processed -- this marks the
        // exchange complete, but (deliberately, matching production) does not yet clear
        // this stream's `PendingResponseStreams`/`added_exchanges_by_response` bookkeeping.
        stream.update(&mut app, |stream, ctx| {
            stream.emit_response_event_for_test(
                warp_multi_agent_api::ResponseEvent {
                    r#type: Some(response_event::Type::Finished(
                        response_event::StreamFinished {
                            reason: Some(response_event::stream_finished::Reason::Done(
                                response_event::stream_finished::Done {},
                            )),
                            conversation_usage_metadata: None,
                            token_usage: vec![],
                            should_refresh_model_config: false,
                            #[allow(deprecated)]
                            request_cost: None,
                            request_charges: None,
                        },
                    )),
                },
                ctx,
            );
        });

        // Simulate the fast-resolving action's own completion landing in this window: its
        // `FinishedAction` is processed while the stream still looks active.
        terminal.update(&mut app, |view, ctx| {
            view.ai_controller().update(ctx, |controller, ctx| {
                controller.action_model.update(ctx, |action_model, ctx| {
                    action_model.apply_finished_action_result(
                        conversation_id,
                        AIAgentActionResult {
                            id: AIAgentActionId::from("fetch-conversation-1".to_owned()),
                            task_id: task_id.clone(),
                            result: AIAgentActionResultType::FetchConversation(
                                FetchConversationResult::Success {
                                    directory_path: "/tmp/conversation".to_owned(),
                                },
                            ),
                        },
                        ctx,
                    );
                });
            });
        });

        // Deferred, not delivered: no request has been sent yet, and the result is still
        // sitting undelivered.
        assert_eq!(*sent_requests.lock().unwrap(), 0);
        terminal.update(&mut app, |view, ctx| {
            view.ai_controller().update(ctx, |controller, ctx| {
                assert!(
                    controller
                        .action_model
                        .as_ref(ctx)
                        .get_finished_action_results(conversation_id)
                        .is_some_and(|results| !results.is_empty()),
                    "the result should still be pending delivery"
                );
            });
        });

        // The stream's own completion is now fully processed, clearing its bookkeeping.
        stream.update(&mut app, |stream, ctx| {
            stream.emit_stream_finished_for_test(ctx);
        });

        // The flush recovers it: exactly one follow-up request went out, and nothing is
        // left stranded.
        assert_eq!(*sent_requests.lock().unwrap(), 1);
        terminal.update(&mut app, |view, ctx| {
            view.ai_controller().update(ctx, |controller, ctx| {
                assert!(
                    controller
                        .action_model
                        .as_ref(ctx)
                        .get_finished_action_results(conversation_id)
                        .is_none_or(|results| results.is_empty())
                );
            });
        });
    });
}

/// A stream can fail with a recoverable error *after* dispatching client actions -- exactly
/// the situation that can strand one of those actions' results (see the test above) -- in
/// which case an automatic resume is scheduled for once the stream finishes. If a stranded
/// result is flushed for that same stream completion, that flush's follow-up request already
/// takes over resuming the conversation; the scheduled resume must not *also* fire, or the
/// conversation gets two outbound requests racing each other.
#[test]
fn flushed_stranded_result_suppresses_a_scheduled_resume_for_the_same_stream() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);

        let sent_requests = Arc::new(Mutex::new(0usize));
        let sent_requests_for_subscription = Arc::clone(&sent_requests);

        let (conversation_id, task_id, stream) = terminal.update(&mut app, |view, ctx| {
            let controller_handle = view.ai_controller().clone();
            ctx.subscribe_to_model(&controller_handle, move |_, _, event, _| {
                if matches!(event, super::BlocklistAIControllerEvent::SentRequest { .. }) {
                    *sent_requests_for_subscription.lock().unwrap() += 1;
                }
            });

            let terminal_surface_id = view.id();
            let stream_id = ResponseStreamId::new_for_test();
            let (conversation_id, task_id) =
                BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                    let conversation_id = history.start_new_conversation(
                        terminal_surface_id,
                        false,
                        false,
                        false,
                        ctx,
                    );
                    let task_id = history
                        .conversation(&conversation_id)
                        .unwrap()
                        .get_root_task_id()
                        .clone();
                    history
                        .update_conversation_for_new_request_input(
                            RequestInput {
                                conversation_id,
                                input_messages: HashMap::from([(task_id.clone(), vec![])]),
                                working_directory: None,
                                model_id: LLMId::from("test-model"),
                                coding_model_id: LLMId::from("test-coding-model"),
                                cli_agent_model_id: LLMId::from("test-cli-agent-model"),
                                computer_use_model_id: LLMId::from("test-computer-use-model"),
                                shared_session_response_initiator: None,
                                request_start_ts: Local::now(),
                                supported_tools_override: None,
                            },
                            stream_id.clone(),
                            terminal_surface_id,
                            ctx,
                        )
                        .unwrap();
                    (conversation_id, task_id)
                });
            let stream = ctx.add_model(|_| ResponseStream::new_for_test(stream_id.clone()));
            view.ai_controller().update(ctx, |controller, ctx| {
                controller.register_mock_stream_for_test(
                    stream_id,
                    conversation_id,
                    stream.clone(),
                    ctx,
                );
            });
            (conversation_id, task_id, stream)
        });

        // A fast action finishes and is deferred, exactly as in the stranding test above.
        terminal.update(&mut app, |view, ctx| {
            view.ai_controller().update(ctx, |controller, ctx| {
                controller.action_model.update(ctx, |action_model, ctx| {
                    action_model.apply_finished_action_result(
                        conversation_id,
                        AIAgentActionResult {
                            id: AIAgentActionId::from("fetch-conversation-1".to_owned()),
                            task_id: task_id.clone(),
                            result: AIAgentActionResultType::FetchConversation(
                                FetchConversationResult::Success {
                                    directory_path: "/tmp/conversation".to_owned(),
                                },
                            ),
                        },
                        ctx,
                    );
                });
            });
        });
        assert_eq!(*sent_requests.lock().unwrap(), 0);

        // But this time the stream failed (recoverably) after dispatching that action, so a
        // resume is scheduled for once it finishes.
        stream.update(&mut app, |stream, ctx| {
            stream.set_pending_resume_for_test(PendingResume::new_for_test(
                RecoveryBudget::fresh().next_attempt(),
                std::time::Duration::from_secs(3600),
            ));
            stream.emit_stream_finished_for_test(ctx);
        });

        // The flush already sent exactly one follow-up request that takes over resuming the
        // conversation, so the scheduled resume must not also be armed.
        assert_eq!(*sent_requests.lock().unwrap(), 1);
        terminal.update(&mut app, |view, ctx| {
            view.ai_controller().update(ctx, |controller, ctx| {
                assert!(
                    !controller
                        .pending_auto_resume_handles
                        .contains_key(&conversation_id),
                    "the scheduled resume must have been suppressed, not armed alongside the flush"
                );

                // The flushed request must not have reset the shared recovery counter: it
                // has to inherit the resume's already-charged budget (one attempt used),
                // not start over at zero.
                let new_stream_id = controller
                    .in_flight_response_streams
                    .stream_ids_for_conversation(conversation_id, ctx)
                    .into_iter()
                    .next()
                    .expect("the flush should have registered a new stream");
                let new_stream = controller
                    .in_flight_response_streams
                    .stream_for_test(&new_stream_id)
                    .expect("the new stream should be registered");
                assert_eq!(
                    new_stream.as_ref(ctx).recovery_for_test().attempts_used(),
                    1,
                    "the flushed request must inherit the suppressed resume's charged budget"
                );
            });
        });
    });
}

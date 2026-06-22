//! Headless tests for the TUI root view and core model bootstrap.

use settings::{PrivatePreferences, PublicPreferences};
use warp_core::execution_mode::{AppExecutionMode, ExecutionMode};
use warp_multi_agent_api::client_action::Action;
use warp_multi_agent_api::response_event::{self, stream_finished};
use warpui::{App, EntityId, SingletonEntity, ViewHandle, WindowId};
use warpui_core::elements::tui::{TuiBufferExt, TuiRect};
use warpui_core::platform::WindowStyle;
use warpui_core::presenter::tui::TuiPresenter;
use warpui_core::AddWindowOptions;
use warpui_extras::user_preferences::in_memory::InMemoryPreferences;

use super::input_view::InputAction;
use super::{CoreTuiModel, RootTuiView, INPUT_ROWS};
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::agent::task::TaskId;
use crate::ai::blocklist::{
    AgentConversationEngine, AgentSessionOwnerId, BlocklistAIHistoryModel, BlocklistAIPermissions,
    ResponseStreamId,
};
use crate::ai::execution_profiles::profiles::AIExecutionProfilesModel;
use crate::ai::llms::LLMPreferences;
use crate::ai::mcp::TemplatableMCPServerManager;
use crate::default_terminal::DefaultTerminal;
use crate::interval_timer::IntervalTimer;
use crate::network::NetworkStatus;
use crate::server::server_api::ServerApiProvider;
use crate::workspace::ActiveSession;
use crate::{initialize_app, LaunchMode};

fn window_options() -> AddWindowOptions {
    AddWindowOptions {
        window_style: WindowStyle::NotStealFocus,
        ..Default::default()
    }
}

/// Types `text` into the input one scalar at a time.
fn type_text(app: &App, window_id: WindowId, input_id: EntityId, text: &str) {
    for ch in text.chars() {
        app.dispatch_typed_action(window_id, &[input_id], &InputAction::Insert(ch.to_string()));
    }
}

/// Presses Enter on the input, as its `enter` key binding does.
fn submit(app: &App, window_id: WindowId, input_id: EntityId) {
    app.dispatch_typed_action(window_id, &[input_id], &InputAction::Submit);
}

/// The first row index whose painted line contains `needle`.
fn row_with(lines: &[String], needle: &str) -> Option<usize> {
    lines.iter().position(|line| line.contains(needle))
}

/// Registers the singletons normally installed before `initialize_app`.
fn register_pre_initialize_singletons(app: &mut App) {
    app.add_singleton_model(|ctx| AppExecutionMode::new(ExecutionMode::App, false, ctx));
    app.add_singleton_model(|_| PublicPreferences::new(Box::<InMemoryPreferences>::default()));
    app.add_singleton_model(|_| PrivatePreferences::new(Box::<InMemoryPreferences>::default()));
}

/// Runs the TUI launch-mode singleton bootstrap without creating a TUI window.
fn initialize_tui_app(app: &mut App) {
    register_pre_initialize_singletons(app);
    app.update(|ctx| {
        let _ = initialize_app(
            &LaunchMode::Tui { prompt: None },
            IntervalTimer::new(),
            None,
            ctx,
            Vec::<anyhow::Error>::new(),
        );
    });
}

/// Creates a root view backed by the initialized TUI agent singleton graph.
fn add_registered_root(app: &mut App) -> (WindowId, ViewHandle<RootTuiView>, AgentSessionOwnerId) {
    initialize_tui_app(app);
    let (window_id, root) =
        app.update(|ctx| ctx.add_tui_window(window_options(), RootTuiView::new));
    let owner = AgentSessionOwnerId::new(root.id());
    CoreTuiModel::handle(app).update(app, |model, ctx| {
        model.register_session(owner, ctx);
    });
    (window_id, root, owner)
}

/// Returns the active conversation and response stream IDs.
fn active_request(app: &App) -> (AIConversationId, ResponseStreamId) {
    let (conversation_id, stream) = CoreTuiModel::handle(app).read(app, |model, _| {
        (
            model
                .active_conversation_id()
                .expect("request should have an active conversation"),
            model
                .in_flight_response_stream_for_test()
                .expect("request should be in flight"),
        )
    });
    let stream_id = stream.read(app, |stream, _| stream.id().clone());
    (conversation_id, stream_id)
}
/// Returns a clone of the in-flight request params for assertions.
fn in_flight_request_params(app: &App) -> crate::ai::agent::api::RequestParams {
    let stream = CoreTuiModel::handle(app).read(app, |model, _| {
        model
            .in_flight_response_stream_for_test()
            .expect("request should be in flight")
    });
    stream.read(app, |stream, _| stream.params_for_test().clone())
}

/// Returns the root task id for a TUI conversation.
fn root_task_id(app: &App, conversation_id: AIConversationId) -> TaskId {
    BlocklistAIHistoryModel::handle(app).read(app, |history_model, _| {
        history_model
            .conversation(&conversation_id)
            .expect("conversation should exist")
            .get_root_task_id()
            .clone()
    })
}

/// Creates a server root task for fake stream events.
fn server_root_task(task_id: &TaskId) -> warp_multi_agent_api::Task {
    warp_multi_agent_api::Task {
        id: task_id.to_string(),
        messages: vec![],
        dependencies: None,
        description: "TUI test task".to_string(),
        summary: String::new(),
        server_data: String::new(),
    }
}

/// Creates a fake agent output message for a task.
fn agent_output_message(
    task_id: &TaskId,
    request_id: &str,
    message_id: &str,
    text: &str,
) -> warp_multi_agent_api::Message {
    warp_multi_agent_api::Message {
        id: message_id.to_string(),
        task_id: task_id.to_string(),
        server_message_data: String::new(),
        citations: vec![],
        message: Some(warp_multi_agent_api::message::Message::AgentOutput(
            warp_multi_agent_api::message::AgentOutput {
                text: text.to_string(),
            },
        )),
        request_id: request_id.to_string(),
        timestamp: None,
    }
}

/// Folds a fake server response event through the TUI model's shared engine delegate.
fn fold_response_event(
    app: &mut App,
    owner: AgentSessionOwnerId,
    stream_id: &ResponseStreamId,
    conversation_id: AIConversationId,
    event: warp_multi_agent_api::ResponseEvent,
) {
    CoreTuiModel::handle(app).update(app, |model, ctx| {
        AgentConversationEngine::fold_received_event_for_test(
            model,
            owner,
            true,
            stream_id,
            conversation_id,
            Ok(event),
            false,
            ctx,
        );
    });
}

/// Finalizes the fake stream through the same after-stream path as a real stream.
fn finish_fake_stream(app: &mut App, owner: AgentSessionOwnerId, stream_id: &ResponseStreamId) {
    let stream = CoreTuiModel::handle(app).read(app, |model, _| {
        model
            .in_flight_response_stream_for_test()
            .expect("request should be in flight")
    });
    CoreTuiModel::handle(app).update(app, |model, ctx| {
        AgentConversationEngine::fold_after_stream_finished_for_test(
            model, owner, stream_id, &stream, ctx,
        );
    });
}

/// Completes the first fake stream, including server root-task creation.
fn complete_initial_fake_stream(
    app: &mut App,
    owner: AgentSessionOwnerId,
    conversation_id: AIConversationId,
    stream_id: &ResponseStreamId,
) {
    let task_id = root_task_id(app, conversation_id);
    fold_response_event(
        app,
        owner,
        stream_id,
        conversation_id,
        warp_multi_agent_api::ResponseEvent {
            r#type: Some(response_event::Type::Init(response_event::StreamInit {
                request_id: "request-1".to_string(),
                conversation_id: "server-conversation-1".to_string(),
                run_id: String::new(),
            })),
        },
    );
    fold_response_event(
        app,
        owner,
        stream_id,
        conversation_id,
        warp_multi_agent_api::ResponseEvent {
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
                                    messages: vec![agent_output_message(
                                        &task_id,
                                        "request-1",
                                        "agent-message-1",
                                        "first response",
                                    )],
                                },
                            )),
                        },
                    ],
                },
            )),
        },
    );
    fold_finished_event(app, owner, stream_id, conversation_id);
}

/// Completes a fake follow-up stream on an existing server-backed task.
fn complete_follow_up_fake_stream(
    app: &mut App,
    owner: AgentSessionOwnerId,
    conversation_id: AIConversationId,
    stream_id: &ResponseStreamId,
) {
    let task_id = root_task_id(app, conversation_id);
    fold_response_event(
        app,
        owner,
        stream_id,
        conversation_id,
        warp_multi_agent_api::ResponseEvent {
            r#type: Some(response_event::Type::Init(response_event::StreamInit {
                request_id: "request-2".to_string(),
                conversation_id: "server-conversation-1".to_string(),
                run_id: String::new(),
            })),
        },
    );
    fold_response_event(
        app,
        owner,
        stream_id,
        conversation_id,
        warp_multi_agent_api::ResponseEvent {
            r#type: Some(response_event::Type::ClientActions(
                response_event::ClientActions {
                    actions: vec![warp_multi_agent_api::ClientAction {
                        action: Some(Action::AddMessagesToTask(
                            warp_multi_agent_api::client_action::AddMessagesToTask {
                                task_id: task_id.to_string(),
                                messages: vec![agent_output_message(
                                    &task_id,
                                    "request-2",
                                    "agent-message-2",
                                    "second response",
                                )],
                            },
                        )),
                    }],
                },
            )),
        },
    );
    fold_finished_event(app, owner, stream_id, conversation_id);
}

/// Folds a successful StreamFinished event and finalizes the fake stream.
fn fold_finished_event(
    app: &mut App,
    owner: AgentSessionOwnerId,
    stream_id: &ResponseStreamId,
    conversation_id: AIConversationId,
) {
    fold_response_event(
        app,
        owner,
        stream_id,
        conversation_id,
        warp_multi_agent_api::ResponseEvent {
            r#type: Some(response_event::Type::Finished(
                response_event::StreamFinished {
                    reason: Some(stream_finished::Reason::Done(stream_finished::Done {})),
                    conversation_usage_metadata: None,
                    token_usage: vec![],
                    should_refresh_model_config: false,
                    request_cost: None,
                },
            )),
        },
    );
    finish_fake_stream(app, owner, stream_id);
}

#[test]
fn submitting_sends_prompt_and_streamed_response_into_transcript() {
    App::test((), |mut app| async move {
        let (window_id, root, owner) = add_registered_root(&mut app);
        let input_id = app.read(|ctx| root.read(ctx, |view, _| view.input.id()));

        // The input is focused at construction, so the cursor is owned by it.
        assert_eq!(app.focused_view_id(window_id), Some(input_id));

        let mut presenter = TuiPresenter::new();
        // 8 rows tall: the input frame is the bottom `INPUT_ROWS` (3) rows, so
        // the inner text row sits at `height - 2` and the transcript fills the
        // rows above it.
        let width = 40;
        let height = 8;
        let area = TuiRect::new(0, 0, width, height);
        let input_text_row = height - 2;
        let first_input_row = height - INPUT_ROWS;

        type_text(&app, window_id, input_id, "hello world");

        // Before submission the draft renders inside the input frame and the
        // cursor trails the typed text (one cell past it, offset by the border).
        let frame = app.update(|ctx| presenter.present(ctx, &root, area));
        let lines = frame.buffer.to_lines();
        assert!(
            lines[input_text_row as usize].contains("hello world"),
            "the draft should render in the input frame's text row:\n{}",
            lines.join("\n")
        );
        // 11 chars typed, +1 for the left border cell.
        assert_eq!(frame.cursor, Some((12, input_text_row)));

        submit(&app, window_id, input_id);
        let (conversation_id, stream_id) = active_request(&app);
        complete_initial_fake_stream(&mut app, owner, conversation_id, &stream_id);

        let frame = app.update(|ctx| presenter.present(ctx, &root, area));
        let lines = frame.buffer.to_lines();

        // The shared-history transcript shows both the submitted prompt and the
        // response folded from the fake MAA stream.
        let transcript_row = row_with(&lines, "hello world")
            .expect("the transcript should show the submitted prompt");
        let response_row = row_with(&lines, "first response")
            .expect("the transcript should show the streamed response");
        assert!(
            transcript_row < first_input_row as usize,
            "the submitted prompt should render above the input frame (row {transcript_row}):\n{}",
            lines.join("\n")
        );
        assert!(
            response_row < first_input_row as usize,
            "the streamed response should render above the input frame (row {response_row}):\n{}",
            lines.join("\n")
        );

        // The input is cleared back to its placeholder empty-state, and the
        // submitted text is no longer in the input frame.
        let input_region = lines[first_input_row as usize..].join("\n");
        assert!(
            input_region.contains("Warp anything"),
            "the cleared input should show its placeholder:\n{input_region}"
        );
        assert!(
            !input_region.contains("hello world"),
            "the submitted text should no longer be in the input frame:\n{input_region}"
        );

        // Focus stays on the input and the cursor resets to the frame's start.
        assert_eq!(app.focused_view_id(window_id), Some(input_id));
        assert_eq!(frame.cursor, Some((1, input_text_row)));
    });
}

#[test]
fn tui_initialize_app_registers_agent_singletons_without_terminal_session() {
    App::test((), |mut app| async move {
        initialize_tui_app(&mut app);

        assert_eq!(app.models_of_type::<CoreTuiModel>().len(), 1);
        assert_eq!(app.models_of_type::<BlocklistAIHistoryModel>().len(), 1);
        assert_eq!(app.models_of_type::<LLMPreferences>().len(), 1);
        assert_eq!(app.models_of_type::<AIExecutionProfilesModel>().len(), 1);
        assert_eq!(app.models_of_type::<BlocklistAIPermissions>().len(), 1);
        assert_eq!(app.models_of_type::<ServerApiProvider>().len(), 1);
        assert_eq!(app.models_of_type::<NetworkStatus>().len(), 1);
        assert_eq!(app.models_of_type::<TemplatableMCPServerManager>().len(), 1);
        assert!(app.models_of_type::<ActiveSession>().is_empty());
        assert!(app.models_of_type::<DefaultTerminal>().is_empty());
    });
}

#[test]
fn transcript_follows_streamed_output_and_can_scroll_to_older_exchanges() {
    App::test((), |mut app| async move {
        let (window_id, root, owner) = add_registered_root(&mut app);
        let input_id = app.read(|ctx| root.read(ctx, |view, _| view.input.id()));
        type_text(&app, window_id, input_id, "first");
        submit(&app, window_id, input_id);
        let (conversation_id, first_stream_id) = active_request(&app);
        complete_initial_fake_stream(&mut app, owner, conversation_id, &first_stream_id);

        type_text(&app, window_id, input_id, "second");
        submit(&app, window_id, input_id);
        let (follow_up_conversation_id, second_stream_id) = active_request(&app);
        assert_eq!(follow_up_conversation_id, conversation_id);
        complete_follow_up_fake_stream(&mut app, owner, conversation_id, &second_stream_id);

        // 7 rows tall leaves a four-row transcript viewport, shorter than the
        // two complete exchanges and their spacer rows.
        let mut presenter = TuiPresenter::new();
        let area = TuiRect::new(0, 0, 40, 7);
        let frame = app.update(|ctx| presenter.present(ctx, &root, area));
        let lines = frame.buffer.to_lines();
        let rendered = lines.join("\n");
        assert!(
            rendered.contains("second response"),
            "new streamed output should be visible after following the bottom:\n{rendered}"
        );
        assert!(
            !rendered.contains("first response"),
            "older output should be above the bottom-following viewport:\n{rendered}"
        );
        assert!(
            root.read(&app, |root, _| root.scroll.offset()) > 0,
            "the transcript should be scrolled away from the top"
        );

        root.update(&mut app, |root, ctx| {
            root.scroll.set_offset(0);
            ctx.notify();
        });
        let frame = app.update(|ctx| presenter.present(ctx, &root, area));
        let rendered = frame.buffer.to_lines().join("\n");
        assert!(
            rendered.contains("first response"),
            "scrolling to the top should reveal older output:\n{rendered}"
        );
    });
}

#[test]
fn rejected_submission_is_restored_to_the_input() {
    App::test((), |mut app| async move {
        let (window_id, root, _owner) = add_registered_root(&mut app);
        let input_id = app.read(|ctx| root.read(ctx, |view, _| view.input.id()));

        type_text(&app, window_id, input_id, "first");
        submit(&app, window_id, input_id);
        type_text(&app, window_id, input_id, "retry this");
        submit(&app, window_id, input_id);

        let mut presenter = TuiPresenter::new();
        let area = TuiRect::new(0, 0, 40, 8);
        let frame = app.update(|ctx| presenter.present(ctx, &root, area));
        let lines = frame.buffer.to_lines();
        let input_region = lines[(8 - INPUT_ROWS) as usize..].join("\n");
        assert!(
            input_region.contains("retry this"),
            "a prompt rejected while another request is in flight should be restored:\n{input_region}"
        );

        let conversation_id = CoreTuiModel::handle(&app).read(&app, |model, _| {
            model
                .active_conversation_id()
                .expect("first prompt should start a conversation")
        });
        BlocklistAIHistoryModel::handle(&app).read(&app, |history_model, _| {
            let conversation = history_model
                .conversation(&conversation_id)
                .expect("active conversation should exist");
            assert_eq!(
                conversation.root_task_exchanges().count(),
                1,
                "the rejected prompt should not create another exchange"
            );
        });
    });
}

#[test]
fn core_tui_model_sends_initial_prompt_and_follow_up() {
    App::test((), |mut app| async move {
        initialize_tui_app(&mut app);
        let owner = AgentSessionOwnerId::new(EntityId::new());
        CoreTuiModel::handle(&app).update(&mut app, |model, ctx| {
            model.register_session(owner, ctx);
        });

        let (conversation_id, first_stream_id) = CoreTuiModel::handle(&app)
            .update(&mut app, |model, ctx| {
                model.send_prompt_for_test("first".to_string(), ctx)
            })
            .expect("first prompt should send");
        let first_params = in_flight_request_params(&app);
        assert_eq!(first_params.input.len(), 1);
        assert_eq!(first_params.conversation_token, None);
        assert_eq!(first_params.tasks.len(), 0);
        assert_eq!(first_params.supported_tools_override, None);

        complete_initial_fake_stream(&mut app, owner, conversation_id, &first_stream_id);

        BlocklistAIHistoryModel::handle(&app).read(&app, |history_model, _| {
            let conversation = history_model
                .conversation(&conversation_id)
                .expect("conversation should exist after first prompt");
            assert_eq!(conversation.root_task_exchanges().count(), 1);
            assert!(conversation.server_conversation_token().is_some());
            assert!(conversation
                .root_task_exchanges()
                .last()
                .expect("first exchange should exist")
                .output_status
                .is_finished_and_successful());
            assert!(conversation
                .new_exchange_ids_for_response(&first_stream_id)
                .next()
                .is_none());
        });

        let (follow_up_conversation_id, _second_stream_id) = CoreTuiModel::handle(&app)
            .update(&mut app, |model, ctx| {
                model.send_prompt_for_test("second".to_string(), ctx)
            })
            .expect("second prompt should send");
        assert_eq!(follow_up_conversation_id, conversation_id);
        let second_params = in_flight_request_params(&app);
        assert_eq!(second_params.input.len(), 1);
        assert!(
            second_params.conversation_token.is_some(),
            "follow-up should carry server conversation token",
        );
        assert!(
            !second_params.tasks.is_empty(),
            "follow-up should carry prior task context",
        );
        assert_eq!(second_params.supported_tools_override, None);

        complete_follow_up_fake_stream(&mut app, owner, conversation_id, &_second_stream_id);

        BlocklistAIHistoryModel::handle(&app).read(&app, |history_model, _| {
            let conversations = history_model
                .all_live_conversations_for_terminal_view(owner.entity_id())
                .collect::<Vec<_>>();
            assert_eq!(conversations.len(), 1);
            let conversation = conversations[0];
            assert_eq!(conversation.id(), conversation_id);
            assert_eq!(conversation.root_task_exchanges().count(), 2);
            assert!(conversation
                .root_task_exchanges()
                .last()
                .expect("second exchange should exist")
                .output_status
                .is_finished_and_successful());
        });
    });
}

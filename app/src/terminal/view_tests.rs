use std::any::Any;
use std::cell::RefCell;
use std::collections::HashSet;
use std::pin::pin;
use std::rc::Rc;
use std::str::FromStr;
use std::sync::Arc;

use chrono::Local;
use parking_lot::FairMutex;
use session_sharing_protocol::common::CLIAgentSessionState;
use warp_cli::agent::Harness;
use warp_terminal::model::escape_sequences::{BRACKETED_PASTE_END, BRACKETED_PASTE_START, C0};
use warpui::notification::UserNotification;
use warpui::platform::WindowStyle;
use warpui::{App, Presenter, ReadModel, WindowInvalidation};

use super::*;
use crate::ai::agent::conversation::{AIConversation, ConversationStatus};
use crate::ai::agent::task::TaskId;
use crate::ai::agent::{
    AIAgentActionId, AIAgentExchange, AIAgentExchangeId, AIAgentInput, AIAgentOutputStatus,
    UserQueryMode,
};
use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::ai::blocklist::agent_view::toolbar_item::AgentToolbarItemKind;
use crate::ai::blocklist::agent_view::{
    AgentViewEntryBlock, AgentViewEntryOrigin, AgentViewState, EnterAgentBlockAction,
    ExitAgentViewError,
};
use crate::ai::blocklist::block::cli_controller::UserTakeOverReason;
use crate::ai::blocklist::{
    BlocklistAIHistoryEvent, BlocklistAIHistoryModel, InputConfig, InputType, ResponseStream,
    ResponseStreamId,
};
use crate::ai::cloud_environments::{
    AmbientAgentEnvironment, CloudAmbientAgentEnvironment, CloudAmbientAgentEnvironmentModel,
};
use crate::ai::llms::LLMId;
use crate::cloud_object::model::persistence::CloudModel;
use crate::cloud_object::{CloudObjectMetadata, CloudObjectPermissions};
use crate::context_chips::prompt::Prompt;
use crate::editor::{AutosuggestionLocation, AutosuggestionType, CrdtOperation};
use crate::features::FeatureFlag;
use crate::pane_group::focus_state::PaneGroupFocusState;
use crate::pane_group::pane::PaneStack;
use crate::pane_group::{BackingView, TerminalPaneId};
use crate::server::ids::{ClientId, SyncId};
use crate::server::server_api::ai::SpawnAgentRequest;
use crate::settings::import::model::ImportedConfigModel;
use crate::settings::{AISettings, AppEditorSettings, WarpPromptSeparator};
use crate::terminal::alt_screen::should_intercept_mouse;
use crate::terminal::block_list_element::{SnackbarPoint, SnackbarTranslationMode};
use crate::terminal::block_list_viewport::{ClampingMode, ScrollLines};
use crate::terminal::cli_agent_sessions::event::{
    CLIAgentEvent, CLIAgentEventPayload, CLIAgentEventSource, CLIAgentEventType,
};
use crate::terminal::cli_agent_sessions::listener::CLIAgentSessionListener;
use crate::terminal::cli_agent_sessions::{
    CLIAgentInputEntrypoint, CLIAgentInputState, CLIAgentRichInputCloseReason, CLIAgentSession,
    CLIAgentSessionContext, CLIAgentSessionStatus, CLIAgentSessionsModel,
};
use crate::terminal::model::ansi::{self, BootstrappedValue, InitShellValue, PreexecValue};
use crate::terminal::model::block::AgentViewVisibility;
use crate::terminal::model::blocks::{insert_block, TotalIndex};
use crate::terminal::model::grid::Dimensions as _;
use crate::terminal::model::terminal_model::WithinBlock;
use crate::terminal::session_settings::AgentToolbarChipSelection;
use crate::terminal::shared_session::shared_handlers::{
    apply_cli_agent_state_update, RemoteUpdateGuard,
};
use crate::terminal::shared_session::{SharedSessionSource, SharedSessionStatus};
use crate::terminal::view::ambient_agent::AmbientAgentViewModelEvent;
use crate::terminal::view::load_ai_conversation::{
    RestoreConversationEntryBehavior, RestoredAIConversation,
};
use crate::terminal::view::shared_session::ConversationEndedTombstoneView;
use crate::terminal::{CLIAgent, MockTerminalManager, TerminalManager, TerminalModel};
use crate::test_util::terminal::{
    add_window_with_id_and_terminal, initialize_app_for_terminal_view,
};
use crate::test_util::{add_window_with_terminal, assert_eventually};
use crate::view_components::find::FindWithinBlockState;
use crate::workspace::ToastStack;
use crate::ActiveAgentViewsModel;

fn add_window_with_cloud_mode_terminal(app: &mut App) -> ViewHandle<TerminalView> {
    let tips_model = app.add_model(|_| Default::default());
    let (_, terminal) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
        TerminalView::new_for_test_with_cloud_mode(tips_model, None, true, ctx)
    });
    terminal.update(app, |view, _| {
        view.model.lock().set_is_dummy_cloud_mode_session(true);
    });
    terminal
}

fn has_pending_user_query_block(view: &TerminalView) -> bool {
    let Some(view_id) = view.pending_user_query_view_id else {
        return false;
    };
    view.rich_content_views.iter().any(|rich_content| {
        rich_content.view_id() == view_id && rich_content.is_pending_user_query()
    })
}

#[test]
fn focus_reporting_writes_focus_events_in_normal_screen() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        let pty_writes: Rc<RefCell<Vec<Vec<u8>>>> = Rc::new(RefCell::new(Vec::new()));
        let writes = pty_writes.clone();

        app.update(|ctx| {
            ctx.subscribe_to_view(&terminal, move |_, event, _| {
                if let Event::WriteBytesToPty { bytes } = event {
                    writes.borrow_mut().push(bytes.to_vec());
                }
            });
        });

        terminal.update(&mut app, |view, ctx| {
            let mut model = view.model.lock();
            model.simulate_long_running_block("python3 /tmp/warp_focus_test.py", "");
            assert!(!model.is_alt_screen_active());
            ansi::Handler::set_mode(&mut *model, ansi::Mode::ReportFocusInOut);
            assert!(model.is_term_mode_set(TermMode::FOCUS_IN_OUT));
            drop(model);
            assert!(view.should_report_focus(ctx));

            view.maybe_report_focus_out(ctx);
            view.maybe_report_focus_in(ctx);
        });

        assert_eq!(
            *pty_writes.borrow(),
            vec![
                escape_sequences::EscCodes::FOCUS_OUT.to_vec(),
                escape_sequences::EscCodes::FOCUS_IN.to_vec(),
            ]
        );
    })
}

fn input_operations_for_buffer_content(app: &mut App, content: &str) -> Vec<CrdtOperation> {
    let terminal = add_window_with_terminal(app, None);
    terminal.update(app, |view, ctx| {
        view.input().update(ctx, |input, ctx| {
            input.replace_buffer_content(content, ctx);
        });
    });
    terminal.read(app, |view, ctx| {
        view.input()
            .as_ref(ctx)
            .latest_buffer_operations()
            .cloned()
            .collect()
    })
}

fn exchange_with_inputs(inputs: Vec<AIAgentInput>) -> AIAgentExchange {
    AIAgentExchange {
        id: AIAgentExchangeId::new(),
        input: inputs,
        output_status: AIAgentOutputStatus::Streaming { output: None },
        added_message_ids: HashSet::new(),
        start_time: Local::now(),
        finish_time: None,
        time_to_first_token_ms: None,
        working_directory: None,
        model_id: LLMId::from("test-model"),
        request_cost: None,
        coding_model_id: LLMId::from("test-coding-model"),
        cli_agent_model_id: LLMId::from("test-cli-agent-model"),
        computer_use_model_id: LLMId::from("test-computer-use-model"),
        response_initiator: None,
    }
}

fn append_exchange_and_handle_event(
    view: &mut TerminalView,
    input: AIAgentInput,
    ctx: &mut ViewContext<TerminalView>,
) -> (
    AIConversationId,
    TaskId,
    AIAgentExchangeId,
    ResponseStreamId,
) {
    append_exchange_with_inputs_and_handle_event(view, vec![input], ctx)
}

fn append_exchange_with_inputs_and_handle_event(
    view: &mut TerminalView,
    inputs: Vec<AIAgentInput>,
    ctx: &mut ViewContext<TerminalView>,
) -> (
    AIConversationId,
    TaskId,
    AIAgentExchangeId,
    ResponseStreamId,
) {
    let history_model = BlocklistAIHistoryModel::handle(ctx);
    let (conversation_id, task_id, exchange_id, response_stream_id) =
        history_model.update(ctx, |history_model, ctx| {
            let conversation_id =
                history_model.start_new_conversation(view.view_id, false, false, false, ctx);
            let task_id = history_model
                .conversation(&conversation_id)
                .expect("conversation should exist")
                .get_root_task_id()
                .clone();
            let response_stream_id = ResponseStreamId::new_for_test();
            let exchange = exchange_with_inputs(inputs);
            let exchange_id = exchange.id;
            history_model
                .conversation_mut(&conversation_id)
                .expect("conversation should exist")
                .append_reassigned_exchange(&response_stream_id, exchange, view.view_id, ctx)
                .expect("exchange should append");
            (conversation_id, task_id, exchange_id, response_stream_id)
        });

    view.handle_ai_history_model_event(
        history_model,
        &BlocklistAIHistoryEvent::AppendedExchange {
            exchange_id,
            task_id: task_id.clone(),
            terminal_view_id: view.view_id,
            conversation_id,
            is_hidden: false,
            response_stream_id: Some(response_stream_id.clone()),
        },
        ctx,
    );
    (conversation_id, task_id, exchange_id, response_stream_id)
}

fn update_exchange_input_and_handle_event(
    view: &mut TerminalView,
    conversation_id: AIConversationId,
    exchange_id: AIAgentExchangeId,
    response_stream_id: ResponseStreamId,
    inputs: Vec<AIAgentInput>,
    ctx: &mut ViewContext<TerminalView>,
) {
    let history_model = BlocklistAIHistoryModel::handle(ctx);
    history_model.update(ctx, |history_model, ctx| {
        let conversation = history_model
            .conversation_mut(&conversation_id)
            .expect("conversation should exist");
        let mut exchange = conversation
            .remove_exchange(exchange_id)
            .expect("exchange should exist");
        exchange.input = inputs;
        conversation
            .append_reassigned_exchange(&response_stream_id, exchange, view.view_id, ctx)
            .expect("exchange should append");
    });

    view.handle_ai_history_model_event(
        history_model,
        &BlocklistAIHistoryEvent::UpdatedStreamingExchange {
            exchange_id,
            terminal_view_id: view.view_id,
            conversation_id,
            is_hidden: false,
        },
        ctx,
    );
}

fn ai_block_count(view: &TerminalView) -> usize {
    view.rich_content_views
        .iter()
        .filter(|rich_content| {
            matches!(
                rich_content.metadata(),
                Some(RichContentMetadata::AIBlock(_))
            )
        })
        .count()
}

fn agent_view_entry_count_for_conversation(
    view: &TerminalView,
    conversation_id: AIConversationId,
) -> usize {
    view.rich_content_views
        .iter()
        .filter(|rich_content| {
            matches!(
                rich_content.metadata(),
                Some(RichContentMetadata::AgentViewEntry(params))
                    if params.conversation_id == conversation_id
            )
        })
        .count()
}

fn command_block_count_for_conversation(
    view: &TerminalView,
    conversation_id: AIConversationId,
) -> usize {
    view.model
        .lock()
        .block_list()
        .blocks()
        .iter()
        .filter(|block| {
            matches!(
                block.agent_view_visibility(),
                AgentViewVisibility::Agent {
                    origin_conversation_id,
                    ..
                } if *origin_conversation_id == conversation_id
            )
        })
        .count()
}

/// Bootstraps the terminal model with one completed block and one active long-running block.
fn bootstrap_with_long_running_block(view: &mut TerminalView) {
    let mut model = view.model.lock();
    model.init_shell(InitShellValue {
        session_id: 0.into(),
        shell: "zsh".to_owned(),
        ..Default::default()
    });
    model.bootstrapped(BootstrappedValue {
        shell: "zsh".to_owned(),
        ..Default::default()
    });
    model.simulate_block("ls", "file.txt");
    model.simulate_long_running_block("long-command", "output");
}

/// Places the active block in agent-driving-but-not-monitoring state:
/// `requested_command_action_id` is set but `long_running_control_state` is None.
/// This simulates the window between when the agent writes the command to the
/// PTY and when `BlocklistAIHistoryEvent::CreatedSubtask` fires.
fn set_active_block_agent_driving(view: &mut TerminalView, conversation_id: AIConversationId) {
    let action_id = AIAgentActionId::from("test-action".to_owned());
    view.model
        .lock()
        .block_list_mut()
        .active_block_mut()
        .set_agent_interaction_mode_for_requested_command(action_id, None, conversation_id);
}

#[test]
fn updated_conversation_metadata_refreshes_selected_conversation_pane_title() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(false);
        let terminal = add_window_with_terminal(&mut app, None);
        let conversation_id = AIConversationId::new();

        terminal.update(&mut app, |view, ctx| {
            let conversation = AIConversation::new_restored(
                conversation_id,
                vec![warp_multi_agent_api::Task {
                    id: "root-task".to_string(),
                    messages: vec![],
                    dependencies: None,
                    description: "Original title".to_string(),
                    summary: String::new(),
                    server_data: String::new(),
                }],
                None,
            )
            .expect("conversation should restore");

            BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                history.restore_conversations(view.view_id, vec![conversation], ctx);
            });
            view.ai_context_model.update(ctx, |context_model, ctx| {
                context_model.set_pending_query_state_for_existing_conversation(
                    conversation_id,
                    AgentViewEntryOrigin::AgentViewBlock,
                    ctx,
                );
            });
            view.update_pane_configuration(ctx);
            assert_eq!(
                view.pane_configuration.as_ref(ctx).title(),
                "Original title"
            );

            BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                history.apply_conversation_title(conversation_id, "Renamed title".to_string(), ctx)
            });
            view.handle_ai_history_model_event(
                BlocklistAIHistoryModel::handle(ctx),
                &BlocklistAIHistoryEvent::UpdatedConversationTitle {
                    terminal_view_id: Some(view.view_id),
                    conversation_id,
                    title: "Renamed title".to_string(),
                },
                ctx,
            );

            assert_eq!(view.pane_configuration.as_ref(ctx).title(), "Renamed title");
        });
    })
}
struct TestTerminalManager {
    model: Arc<FairMutex<TerminalModel>>,
    view: ViewHandle<TerminalView>,
}

impl TerminalManager for TestTerminalManager {
    fn model(&self) -> Arc<FairMutex<TerminalModel>> {
        self.model.clone()
    }

    fn view(&self) -> ViewHandle<TerminalView> {
        self.view.clone()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// Test to verify that blocks created through normal execution
/// have the correct local status set
#[test]
fn test_create_new_block_with_local_status() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);

        // Set up a terminal with a local session
        terminal.update(&mut app, |view, _ctx| {
            let mut model = view.model.lock();

            // Initialize a local session
            model.init_shell(InitShellValue {
                session_id: 0.into(),
                shell: "bash".to_owned(),
                ..Default::default()
            });
            model.bootstrapped(BootstrappedValue {
                shell: "bash".to_owned(),
                ..Default::default()
            });
        });

        assert_eventually!(
            terminal.read(&app, |view, ctx| !view
                .active_block_is_considered_remote(ctx)),
            "Block should be local"
        );

        // No remote blocks should exist
        assert_eventually!(
            terminal.read(&app, |view, _ctx| !view.contains_restored_remote_blocks()),
            "No remote blocks should exist"
        );

        // Update the view's flags
        // view.update_focused_terminal_info(ctx);
        assert_eventually!(
            terminal.read(&app, |view, _ctx| !view.any_session_contains_remote_blocks),
            "No remote blocks should exist"
        );

        // Now test with a remote session
        terminal.update(&mut app, |view, _ctx| {
            let mut model = view.model.lock();

            // Create a new block with a remote session ID and remote_shell
            model.init_shell(InitShellValue {
                session_id: 1.into(),
                shell: "bash".to_owned(),
                user: "user".to_owned(),
                hostname: "remote".to_owned(),
                ..Default::default()
            });
            model.bootstrapped(BootstrappedValue {
                shell: "bash".to_owned(),
                ..Default::default()
            });

            // Create a block in the remote session
            model.simulate_block("echo remote", "remote output");
        });

        // Verify block is non-local (remote)
        assert_eventually!(
            terminal.read(&app, |view, ctx| view
                .active_block_is_considered_remote(ctx)),
            "Block should be non-local (remote)"
        );

        // Remote blocks should be detected
        assert_eventually!(
            terminal.read(&app, |view, _ctx| view.any_session_contains_remote_blocks),
            "Remote blocks should be detected"
        );
    })
}

#[test]
fn submit_cli_agent_rich_input_restores_unlocked_input_config() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let _cli_agent_rich_input = FeatureFlag::CLIAgentRichInput.override_enabled(true);
        AISettings::handle(&app).update(&mut app, |settings, ctx| {
            let _ = settings
                .auto_dismiss_rich_input_after_submit
                .set_value(true, ctx);
        });

        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |view, ctx| {
            view.input.update(ctx, |input, ctx| {
                input.ai_input_model().update(ctx, |ai_input, ctx| {
                    ai_input.set_input_config(
                        InputConfig {
                            input_type: InputType::Shell,
                            is_locked: false,
                        },
                        true,
                        None,
                        ctx,
                    );
                });
            });

            CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
                sessions.set_session(
                    view.view_id,
                    CLIAgentSession {
                        agent: CLIAgent::Droid,
                        status: CLIAgentSessionStatus::InProgress,
                        session_context: CLIAgentSessionContext::default(),
                        input_state: CLIAgentInputState::Closed,
                        should_auto_toggle_input: false,
                        listener: None,
                        remote_host: None,
                        plugin_version: None,
                        draft_text: None,
                        custom_command_prefix: None,
                        received_rich_notification: false,
                    },
                    ctx,
                );
            });

            view.open_cli_agent_rich_input(CLIAgentInputEntrypoint::FooterButton, ctx);
            assert!(view.has_active_cli_agent_input_session(ctx));

            view.submit_cli_agent_rich_input("hello!".to_owned(), ctx);
            assert!(!view.has_active_cli_agent_input_session(ctx));
        });

        terminal.read(&app, |view, ctx| {
            let input = view.input.as_ref(ctx);
            let ai_input_model = input.ai_input_model().as_ref(ctx);

            assert_eq!(
                ai_input_model.input_config(),
                InputConfig {
                    input_type: InputType::Shell,
                    is_locked: false,
                }
            );
            assert!(input.editor().as_ref(ctx).buffer_text(ctx).is_empty());
        });
    })
}

#[test]
fn unregister_cli_agent_session_restores_unlocked_input_config() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let _cli_agent_rich_input = FeatureFlag::CLIAgentRichInput.override_enabled(true);

        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |view, ctx| {
            view.input.update(ctx, |input, ctx| {
                input.ai_input_model().update(ctx, |ai_input, ctx| {
                    ai_input.set_input_config(
                        InputConfig {
                            input_type: InputType::Shell,
                            is_locked: false,
                        },
                        true,
                        None,
                        ctx,
                    );
                });
            });

            CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
                sessions.set_session(
                    view.view_id,
                    CLIAgentSession {
                        agent: CLIAgent::Claude,
                        status: CLIAgentSessionStatus::InProgress,
                        session_context: CLIAgentSessionContext::default(),
                        input_state: CLIAgentInputState::Closed,
                        should_auto_toggle_input: false,
                        listener: None,
                        remote_host: None,
                        plugin_version: None,
                        draft_text: None,
                        custom_command_prefix: None,
                        received_rich_notification: false,
                    },
                    ctx,
                );
            });

            view.open_cli_agent_rich_input(CLIAgentInputEntrypoint::FooterButton, ctx);
            assert!(view.has_active_cli_agent_input_session(ctx));

            CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
                sessions.remove_session(view.view_id, ctx);
            });
            assert!(!view.has_active_cli_agent_input_session(ctx));
            assert!(CLIAgentSessionsModel::as_ref(ctx)
                .session(view.view_id)
                .is_none());
        });

        terminal.read(&app, |view, ctx| {
            let input = view.input.as_ref(ctx);
            let ai_input_model = input.ai_input_model().as_ref(ctx);

            assert_eq!(
                ai_input_model.input_config(),
                InputConfig {
                    input_type: InputType::Shell,
                    is_locked: false,
                }
            );
            assert!(input.editor().as_ref(ctx).buffer_text(ctx).is_empty());
        });
    })
}

#[test]
fn clear_buffer_action_in_fullscreen_agent_view_starts_new_conversation() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        FeatureFlag::AgentView.set_enabled(true);

        let terminal = add_window_with_terminal(&mut app, None);

        let original_conversation_id = terminal.update(&mut app, |view, ctx| {
            view.agent_view_controller().update(ctx, |controller, ctx| {
                controller
                    .try_enter_agent_view(
                        None,
                        AgentViewEntryOrigin::Input {
                            was_prompt_autodetected: false,
                        },
                        ctx,
                    )
                    .expect("Should be able to enter agent view")
            })
        });

        terminal.update(&mut app, |view, ctx| {
            view.handle_action(&TerminalAction::ClearBuffer, ctx);
        });

        terminal.update(&mut app, |view, ctx| {
            let new_conversation_id = view
                .agent_view_controller()
                .as_ref(ctx)
                .agent_view_state()
                .active_conversation_id()
                .expect("agent view should still be active");
            assert_ne!(new_conversation_id, original_conversation_id);
        });
    })
}

fn agent_jump_user_query(query: &str) -> AIAgentInput {
    AIAgentInput::UserQuery {
        query: query.to_owned(),
        context: Default::default(),
        static_query_type: None,
        referenced_attachments: Default::default(),
        user_query_mode: UserQueryMode::Normal,
        running_command: None,
        intended_agent: None,
    }
}

#[test]
fn jump_to_latest_agent_message_no_ops_when_agent_view_disabled() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);

        // Create a conversation while the agent view feature is enabled...
        let agent_view = FeatureFlag::AgentView.override_enabled(true);
        terminal.update(&mut app, |view, ctx| {
            append_exchange_and_handle_event(view, agent_jump_user_query("hi"), ctx);
        });
        drop(agent_view);

        // ...then turn the feature off: the action must be inert even though a
        // conversation with a visible exchange exists.
        let _agent_view_off = FeatureFlag::AgentView.override_enabled(false);
        terminal.update(&mut app, |view, ctx| {
            view.jump_to_latest_agent_message(ctx);
        });

        terminal.read(&app, |view, ctx| {
            assert!(!view.agent_view_controller().as_ref(ctx).is_active());
            assert_eq!(view.pending_agent_scroll_target, None);
        });
    })
}

#[test]
fn jump_to_latest_agent_message_no_ops_without_conversations() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);

        let terminal = add_window_with_terminal(&mut app, None);

        // No conversations exist, so there is nothing to jump to.
        terminal.update(&mut app, |view, ctx| {
            view.jump_to_latest_agent_message(ctx);
        });

        terminal.read(&app, |view, ctx| {
            assert!(!view.agent_view_controller().as_ref(ctx).is_active());
            assert_eq!(view.pending_agent_scroll_target, None);
        });
    })
}

#[test]
fn jump_to_latest_agent_message_enters_agent_view_and_records_pending_scroll() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);

        let terminal = add_window_with_terminal(&mut app, None);

        let (conversation_id, _task_id, exchange_id, _stream_id) = terminal
            .update(&mut app, |view, ctx| {
                append_exchange_and_handle_event(view, agent_jump_user_query("hi"), ctx)
            });

        terminal.update(&mut app, |view, ctx| {
            view.jump_to_latest_agent_message(ctx);
        });

        terminal.read(&app, |view, ctx| {
            match view.agent_view_controller().as_ref(ctx).agent_view_state() {
                AgentViewState::Active {
                    conversation_id: active_conversation_id,
                    origin,
                    ..
                } => {
                    assert_eq!(*active_conversation_id, conversation_id);
                    assert_eq!(*origin, AgentViewEntryOrigin::JumpToLatestAgentMessage);
                }
                state => panic!("expected an active agent view, got {state:?}"),
            }
            // Entering from the terminal mounts the target block on a later frame,
            // so the scroll target is recorded for `after_terminal_view_layout`.
            assert_eq!(view.pending_agent_scroll_target, Some(exchange_id));
        });
    })
}

#[test]
fn jump_to_latest_agent_message_targets_latest_visible_exchange() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);

        let terminal = add_window_with_terminal(&mut app, None);

        let (conversation_id, _task_id, _first_exchange_id, _stream_id) = terminal
            .update(&mut app, |view, ctx| {
                append_exchange_and_handle_event(view, agent_jump_user_query("first"), ctx)
            });

        // Append a second, newer exchange to the same conversation.
        let second_exchange_id = terminal.update(&mut app, |view, ctx| {
            let history_model = BlocklistAIHistoryModel::handle(ctx);
            history_model.update(ctx, |history_model, ctx| {
                let response_stream_id = ResponseStreamId::new_for_test();
                let exchange = exchange_with_inputs(vec![agent_jump_user_query("second")]);
                let exchange_id = exchange.id;
                history_model
                    .conversation_mut(&conversation_id)
                    .expect("conversation should exist")
                    .append_reassigned_exchange(&response_stream_id, exchange, view.view_id, ctx)
                    .expect("exchange should append");
                exchange_id
            })
        });

        terminal.update(&mut app, |view, ctx| {
            view.jump_to_latest_agent_message(ctx);
        });

        terminal.read(&app, |view, _ctx| {
            // The jump targets the latest visible exchange, not the first one.
            assert_eq!(view.pending_agent_scroll_target, Some(second_exchange_id));
        });
    })
}

#[test]
fn jump_to_latest_agent_message_scrolls_without_re_entering_when_already_in_view() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);

        let terminal = add_window_with_terminal(&mut app, None);

        let (conversation_id, _task_id, _exchange_id, _stream_id) = terminal
            .update(&mut app, |view, ctx| {
                append_exchange_and_handle_event(view, agent_jump_user_query("hi"), ctx)
            });

        // First jump enters the agent view from the terminal and records a pending
        // scroll target; simulate the layout pass consuming it.
        terminal.update(&mut app, |view, ctx| {
            view.jump_to_latest_agent_message(ctx);
            view.pending_agent_scroll_target = None;
        });

        // Second jump: already in this conversation's agent view, so it scrolls
        // directly without re-entering or recording a new pending target.
        terminal.update(&mut app, |view, ctx| {
            view.jump_to_latest_agent_message(ctx);
        });

        terminal.read(&app, |view, ctx| {
            let active_conversation_id = view
                .agent_view_controller()
                .as_ref(ctx)
                .agent_view_state()
                .active_conversation_id()
                .expect("agent view should be active");
            assert_eq!(active_conversation_id, conversation_id);
            assert_eq!(view.pending_agent_scroll_target, None);
        });
    })
}

#[test]
fn restoring_conversation_to_new_pane_transfers_blocks_from_previous_owner() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);

        let original_owner = add_window_with_terminal(&mut app, None);
        let restored_view = add_window_with_terminal(&mut app, None);

        let original_owner_view_id = original_owner.read(&app, |view, _| view.view_id);
        let restored_view_id = restored_view.read(&app, |view, _| view.view_id);

        let conversation_id = original_owner.update(&mut app, |view, ctx| {
            let (conversation_id, _, _, _) = append_exchange_with_inputs_and_handle_event(
                view,
                vec![AIAgentInput::UserQuery {
                    query: "first query".to_owned(),
                    context: Default::default(),
                    static_query_type: None,
                    referenced_attachments: Default::default(),
                    user_query_mode: UserQueryMode::Normal,
                    running_command: None,
                    intended_agent: None,
                }],
                ctx,
            );
            view.insert_agent_view_entry_block(
                AgentViewEntryBlockParams {
                    conversation_id,
                    is_new: false,
                    is_restored: false,
                    origin: AgentViewEntryOrigin::AgentViewBlock,
                    agent_view_controller: view.agent_view_controller().clone(),
                },
                RichContentInsertionPosition::Append {
                    insert_below_long_running_block: false,
                },
                ctx,
            );
            {
                let mut model = view.model.lock();
                model.simulate_block("agent command", "agent output");
                let command_block_index = model.block_list().blocks().len() - 2;
                model.block_list_mut().blocks_mut()[command_block_index]
                    .set_conversation_id(conversation_id);
            }
            conversation_id
        });

        original_owner.read(&app, |view, _| {
            assert_eq!(ai_block_count(view), 1);
            assert_eq!(
                agent_view_entry_count_for_conversation(view, conversation_id),
                1
            );
            assert_eq!(
                command_block_count_for_conversation(view, conversation_id),
                1
            );
        });

        let restored_conversation =
            BlocklistAIHistoryModel::handle(&app).read(&app, |history, _| {
                history
                    .conversation(&conversation_id)
                    .cloned()
                    .expect("conversation should exist")
            });

        restored_view.update(&mut app, |view, ctx| {
            view.restore_conversation_after_view_creation(
                RestoredAIConversation::new(restored_conversation),
                true,
                RestoreConversationEntryBehavior::EnterRestoredConversation,
                ctx,
            );
        });

        BlocklistAIHistoryModel::handle(&app).read(&app, |history, _| {
            let original_owner_live_conversation_ids = history
                .all_live_conversations_for_terminal_view(original_owner_view_id)
                .map(|conversation| conversation.id())
                .collect::<Vec<_>>();
            let restored_view_live_conversation_ids = history
                .all_live_conversations_for_terminal_view(restored_view_id)
                .map(|conversation| conversation.id())
                .collect::<Vec<_>>();
            assert_eq!(
                history.terminal_view_id_for_conversation(&conversation_id),
                Some(restored_view_id)
            );
            assert!(original_owner_live_conversation_ids.is_empty());
            assert_eq!(restored_view_live_conversation_ids, vec![conversation_id]);
        });

        original_owner.read(&app, |view, _| {
            assert_eq!(ai_block_count(view), 0);
            assert_eq!(
                agent_view_entry_count_for_conversation(view, conversation_id),
                1
            );
            assert_eq!(
                command_block_count_for_conversation(view, conversation_id),
                0
            );
        });
        restored_view.read(&app, |view, _| {
            assert_eq!(ai_block_count(view), 1);
        });
    })
}

#[test]
fn clicking_old_banner_for_open_conversation_focuses_current_owner_without_transferring_blocks() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);

        let original_owner = add_window_with_terminal(&mut app, None);
        let restored_view = add_window_with_terminal(&mut app, None);

        let original_owner_window_id = app.read(|ctx| original_owner.window_id(ctx));
        let original_owner_view_id = original_owner.read(&app, |view, _| view.view_id);
        let restored_view_id = restored_view.read(&app, |view, _| view.view_id);

        let conversation_id = original_owner.update(&mut app, |view, ctx| {
            let (conversation_id, _, _, _) = append_exchange_with_inputs_and_handle_event(
                view,
                vec![AIAgentInput::UserQuery {
                    query: "first query".to_owned(),
                    context: Default::default(),
                    static_query_type: None,
                    referenced_attachments: Default::default(),
                    user_query_mode: UserQueryMode::Normal,
                    running_command: None,
                    intended_agent: None,
                }],
                ctx,
            );
            view.insert_agent_view_entry_block(
                AgentViewEntryBlockParams {
                    conversation_id,
                    is_new: false,
                    is_restored: false,
                    origin: AgentViewEntryOrigin::AgentViewBlock,
                    agent_view_controller: view.agent_view_controller().clone(),
                },
                RichContentInsertionPosition::Append {
                    insert_below_long_running_block: false,
                },
                ctx,
            );
            conversation_id
        });

        let restored_conversation =
            BlocklistAIHistoryModel::handle(&app).read(&app, |history, _| {
                history
                    .conversation(&conversation_id)
                    .cloned()
                    .expect("conversation should exist")
            });

        restored_view.update(&mut app, |view, ctx| {
            view.restore_conversation_after_view_creation(
                RestoredAIConversation::new(restored_conversation),
                true,
                RestoreConversationEntryBehavior::PreserveAgentViewState,
                ctx,
            );
            assert_eq!(
                view.agent_view_controller()
                    .as_ref(ctx)
                    .agent_view_state()
                    .active_conversation_id(),
                None
            );
            view.agent_view_controller().update(ctx, |controller, ctx| {
                controller
                    .try_enter_agent_view(
                        Some(conversation_id),
                        AgentViewEntryOrigin::AgentViewBlock,
                        ctx,
                    )
                    .expect("restored view should enter agent view");
            });
        });
        let restored_agent_view_controller =
            restored_view.read(&app, |view, _| view.agent_view_controller().clone());
        let restored_active_session =
            restored_view.read(&app, |view, _| view.active_session().clone());
        ActiveAgentViewsModel::handle(&app).update(&mut app, |active_views, ctx| {
            active_views.register_agent_view_controller(
                &restored_agent_view_controller,
                &restored_active_session,
                restored_view_id,
                ctx,
            );
        });

        ActiveAgentViewsModel::handle(&app).read(&app, |active_views, ctx| {
            assert_eq!(
                active_views.terminal_view_id_for_conversation(conversation_id, ctx),
                Some(restored_view_id)
            );
        });
        original_owner.read(&app, |view, _| {
            assert_eq!(ai_block_count(view), 0);
            assert_eq!(
                agent_view_entry_count_for_conversation(view, conversation_id),
                1
            );
        });
        restored_view.read(&app, |view, _| {
            assert_eq!(ai_block_count(view), 1);
        });

        let entry_blocks = app
            .views_of_type::<AgentViewEntryBlock>(original_owner_window_id)
            .expect("original window should contain agent entry block");
        assert_eq!(entry_blocks.len(), 1);
        entry_blocks[0].update(&mut app, |block, ctx| {
            block.handle_action(
                &EnterAgentBlockAction::EnterAgentMode { conversation_id },
                ctx,
            );
        });

        BlocklistAIHistoryModel::handle(&app).read(&app, |history, _| {
            assert_eq!(
                history.terminal_view_id_for_conversation(&conversation_id),
                Some(restored_view_id)
            );
            assert!(history
                .all_live_conversations_for_terminal_view(original_owner_view_id)
                .next()
                .is_none());
        });
        original_owner.read(&app, |view, _| {
            assert_eq!(ai_block_count(view), 0);
            assert_eq!(
                agent_view_entry_count_for_conversation(view, conversation_id),
                1
            );
        });
        restored_view.read(&app, |view, _| {
            assert_eq!(ai_block_count(view), 1);
        });
    })
}

#[test]
fn appended_exchange_renders_in_current_owner_after_conversation_transfer() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);

        let original_owner = add_window_with_terminal(&mut app, None);
        let transferred_owner = add_window_with_terminal(&mut app, None);

        let original_owner_view_id = original_owner.read(&app, |view, _| view.view_id);
        let transferred_owner_view_id = transferred_owner.read(&app, |view, _| view.view_id);

        let conversation_id = original_owner.update(&mut app, |view, ctx| {
            let (conversation_id, _, _, _) = append_exchange_with_inputs_and_handle_event(
                view,
                vec![AIAgentInput::UserQuery {
                    query: "first query".to_owned(),
                    context: Default::default(),
                    static_query_type: None,
                    referenced_attachments: Default::default(),
                    user_query_mode: UserQueryMode::Normal,
                    running_command: None,
                    intended_agent: None,
                }],
                ctx,
            );
            conversation_id
        });

        let restored_conversation =
            BlocklistAIHistoryModel::handle(&app).read(&app, |history, _| {
                history
                    .conversation(&conversation_id)
                    .cloned()
                    .expect("conversation should exist")
            });

        transferred_owner.update(&mut app, |view, ctx| {
            view.restore_conversation_after_view_creation(
                RestoredAIConversation::new(restored_conversation),
                true,
                RestoreConversationEntryBehavior::EnterRestoredConversation,
                ctx,
            );
        });

        BlocklistAIHistoryModel::handle(&app).read(&app, |history, _| {
            assert_eq!(
                history.terminal_view_id_for_conversation(&conversation_id),
                Some(transferred_owner_view_id)
            );
        });

        let original_owner_block_count_after_restore =
            original_owner.read(&app, |view, _| ai_block_count(view));
        let transferred_owner_block_count_after_restore =
            transferred_owner.read(&app, |view, _| ai_block_count(view));
        assert_eq!(transferred_owner_block_count_after_restore, 1);

        let (task_id, exchange_id, response_stream_id) = BlocklistAIHistoryModel::handle(&app)
            .update(&mut app, |history, ctx| {
                let conversation = history
                    .conversation_mut(&conversation_id)
                    .expect("conversation should exist");
                let task_id = conversation.get_root_task_id().clone();
                let response_stream_id = ResponseStreamId::new_for_test();
                let exchange = exchange_with_inputs(vec![AIAgentInput::UserQuery {
                    query: "follow up".to_owned(),
                    context: Default::default(),
                    static_query_type: None,
                    referenced_attachments: Default::default(),
                    user_query_mode: UserQueryMode::Normal,
                    running_command: None,
                    intended_agent: None,
                }]);
                let exchange_id = exchange.id;
                conversation
                    .append_reassigned_exchange(
                        &response_stream_id,
                        exchange,
                        original_owner_view_id,
                        ctx,
                    )
                    .expect("exchange should append");
                (task_id, exchange_id, response_stream_id)
            });

        original_owner.update(&mut app, |view, ctx| {
            view.handle_ai_history_model_event(
                BlocklistAIHistoryModel::handle(ctx),
                &BlocklistAIHistoryEvent::AppendedExchange {
                    exchange_id,
                    task_id: task_id.clone(),
                    terminal_view_id: original_owner_view_id,
                    conversation_id,
                    is_hidden: false,
                    response_stream_id: Some(response_stream_id.clone()),
                },
                ctx,
            );
        });

        transferred_owner.update(&mut app, |view, ctx| {
            view.handle_ai_history_model_event(
                BlocklistAIHistoryModel::handle(ctx),
                &BlocklistAIHistoryEvent::AppendedExchange {
                    exchange_id,
                    task_id,
                    terminal_view_id: original_owner_view_id,
                    conversation_id,
                    is_hidden: false,
                    response_stream_id: Some(response_stream_id),
                },
                ctx,
            );
        });

        original_owner.read(&app, |view, _| {
            assert_eq!(
                ai_block_count(view),
                original_owner_block_count_after_restore
            );
        });
        transferred_owner.read(&app, |view, _| {
            assert_eq!(
                ai_block_count(view),
                transferred_owner_block_count_after_restore + 1
            );
        });
    })
}

#[test]
fn command_first_word_and_suffix_preserves_leading_whitespace() {
    assert_eq!(
        command_first_word_and_suffix("  myssh arg"),
        Some(("myssh", " arg"))
    );
}

#[test]
fn command_first_word_and_suffix_handles_alias_without_args() {
    assert_eq!(
        command_first_word_and_suffix("  myssh"),
        Some(("myssh", ""))
    );
}

#[test]
fn escape_pops_nested_cloud_agent_view_with_long_running_command() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let _cloud_mode = FeatureFlag::CloudMode.override_enabled(true);

        let parent_terminal = add_window_with_terminal(&mut app, None);
        let cloud_terminal = add_window_with_cloud_mode_terminal(&mut app);

        let parent_view = parent_terminal.clone();
        let cloud_view = cloud_terminal.clone();
        let parent_model = parent_terminal.read(&app, |view, _| view.model.clone());
        let cloud_model = cloud_terminal.read(&app, |view, _| view.model.clone());
        let pane_stack = app.update(move |ctx| {
            let parent_manager = ctx.add_model(|_| {
                let manager: Box<dyn TerminalManager> = Box::new(TestTerminalManager {
                    model: parent_model,
                    view: parent_view.clone(),
                });
                manager
            });
            let cloud_manager = ctx.add_model(|_| {
                let manager: Box<dyn TerminalManager> = Box::new(TestTerminalManager {
                    model: cloud_model,
                    view: cloud_view.clone(),
                });
                manager
            });
            let pane_stack = ctx.add_model(|ctx| PaneStack::new(parent_manager, parent_view, ctx));
            pane_stack.update(ctx, |stack, ctx| {
                stack.push(cloud_manager, cloud_view, ctx);
            });
            pane_stack
        });

        cloud_terminal.update(&mut app, |view, ctx| {
            view.enter_agent_view_for_new_conversation(None, AgentViewEntryOrigin::CloudAgent, ctx);
            view.model
                .lock()
                .simulate_long_running_block("sleep 10", "running");

            assert!(view.is_ambient_agent_session(ctx));
            assert!(view.is_nested_cloud_mode(ctx));
            assert_eq!(
                view.agent_view_controller()
                    .as_ref(ctx)
                    .can_exit_agent_view(),
                Ok(())
            );
        });

        assert_eq!(
            app.read_model(&pane_stack, |stack, _| stack.active_view().id()),
            cloud_terminal.id()
        );

        cloud_terminal.update(&mut app, |view, ctx| {
            view.handle_input_event(&InputEvent::Escape, ctx);
        });

        assert_eq!(
            app.read_model(&pane_stack, |stack, _| stack.active_view().id()),
            parent_terminal.id()
        );
    })
}

#[test]
fn escape_does_not_exit_root_cloud_agent_view_with_long_running_command() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let _cloud_mode = FeatureFlag::CloudMode.override_enabled(true);

        let terminal = add_window_with_cloud_mode_terminal(&mut app);

        terminal.update(&mut app, |view, ctx| {
            view.enter_agent_view_for_new_conversation(None, AgentViewEntryOrigin::CloudAgent, ctx);
            view.model
                .lock()
                .simulate_long_running_block("claude", "running");

            view.handle_input_event(&InputEvent::Escape, ctx);

            // Root cloud-mode pane has no parent terminal to return to,
            // so Escape is a no-op and agent view stays active.
            assert!(view.agent_view_controller().as_ref(ctx).is_active());
        });
    })
}

#[test]
fn escape_does_not_exit_local_agent_view_with_long_running_command() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);

        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |view, ctx| {
            view.enter_agent_view_for_new_conversation(
                None,
                AgentViewEntryOrigin::Input {
                    was_prompt_autodetected: false,
                },
                ctx,
            );
            view.model
                .lock()
                .simulate_long_running_block("sleep 10", "running");

            assert!(matches!(
                view.agent_view_controller()
                    .as_ref(ctx)
                    .can_exit_agent_view(),
                Err(ExitAgentViewError::LongRunningCommand)
            ));

            view.handle_input_event(&InputEvent::Escape, ctx);

            assert!(view.agent_view_controller().as_ref(ctx).is_active());
        });
    })
}

#[test]
fn root_cloud_mode_pane_sets_root_cloud_mode_context_key() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        app.add_singleton_model(ImportedConfigModel::new);
        FeatureFlag::AgentView.set_enabled(true);
        FeatureFlag::CloudMode.set_enabled(true);

        let terminal = add_window_with_cloud_mode_terminal(&mut app);
        let nested_terminal = add_window_with_cloud_mode_terminal(&mut app);

        terminal.read(&app, |view, ctx| {
            assert!(view
                .keymap_context(ctx)
                .set
                .contains(init::ROOT_CLOUD_MODE_PANE_KEY));
        });

        let root_view = terminal.clone();
        let nested_view = nested_terminal.clone();
        let root_model = terminal.read(&app, |view, _| view.model.clone());
        let nested_model = nested_terminal.read(&app, |view, _| view.model.clone());
        let _pane_stack = app.update(move |ctx| {
            let root_manager = ctx.add_model(|_| {
                let manager: Box<dyn TerminalManager> = Box::new(TestTerminalManager {
                    model: root_model,
                    view: root_view.clone(),
                });
                manager
            });
            let nested_manager = ctx.add_model(|_| {
                let manager: Box<dyn TerminalManager> = Box::new(TestTerminalManager {
                    model: nested_model,
                    view: nested_view.clone(),
                });
                manager
            });
            let pane_stack = ctx.add_model(|ctx| PaneStack::new(root_manager, root_view, ctx));
            pane_stack.update(ctx, |stack, ctx| {
                stack.push(nested_manager, nested_view, ctx);
            });
            pane_stack
        });

        terminal.read(&app, |view, ctx| {
            assert!(view
                .keymap_context(ctx)
                .set
                .contains(init::ROOT_CLOUD_MODE_PANE_KEY));
        });

        nested_terminal.read(&app, |view, ctx| {
            assert!(!view
                .keymap_context(ctx)
                .set
                .contains(init::ROOT_CLOUD_MODE_PANE_KEY));
        });
    });
}

#[test]
fn set_input_mode_agent_does_not_enter_local_agent_from_root_cloud_mode_pane() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        FeatureFlag::AgentView.set_enabled(true);
        FeatureFlag::CloudMode.set_enabled(true);

        let terminal = add_window_with_cloud_mode_terminal(&mut app);

        terminal.update(&mut app, |view, ctx| {
            view.ambient_agent_view_model()
                .expect("cloud mode terminal should have ambient model")
                .update(ctx, |model, ctx| {
                    model.enter_setup(ctx);
                });
            view.model
                .lock()
                .set_shared_session_status(SharedSessionStatus::FinishedViewer);
        });

        terminal.update(&mut app, |view, ctx| {
            assert!(!view.agent_view_controller().as_ref(ctx).is_active());
            view.handle_action(&TerminalAction::SetInputModeAgent, ctx);
            assert!(!view.agent_view_controller().as_ref(ctx).is_active());
        });
    });
}

#[test]
fn cloud_mode_v1_agent_prefixed_query_spawns_cloud_agent() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_mode = FeatureFlag::AgentMode.override_enabled(true);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let _cloud_mode = FeatureFlag::CloudMode.override_enabled(true);
        let _cloud_mode_input_v2 = FeatureFlag::CloudModeInputV2.override_enabled(false);

        let terminal = add_window_with_cloud_mode_terminal(&mut app);
        let input = terminal.read(&app, |view, _| view.input.clone());

        input.update(&mut app, |input, ctx| {
            input.ai_input_model().update(ctx, |ai_input, ctx| {
                ai_input.set_input_config(
                    InputConfig {
                        input_type: InputType::AI,
                        is_locked: false,
                    },
                    true,
                    None,
                    ctx,
                );
            });
            assert!(!input.is_cloud_mode_input_v2_composing(ctx));
            input.replace_buffer_content("/agent fix the tests", ctx);
            input.input_enter(ctx);
        });

        terminal.read(&app, |view, ctx| {
            let ambient_model = view
                .ambient_agent_view_model()
                .expect("cloud mode terminal should have ambient model")
                .as_ref(ctx);
            let request = ambient_model
                .request()
                .expect("enter should submit through the cloud agent spawn path");
            assert_eq!(request.prompt.as_deref(), Some("/agent fix the tests"));
            assert_eq!(request.mode, UserQueryMode::Normal);
            assert!(input.as_ref(ctx).buffer_text(ctx).is_empty());
        });
    });
}

#[test]
fn cloud_mode_v2_agent_prefixed_query_spawns_cloud_agent() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_mode = FeatureFlag::AgentMode.override_enabled(true);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let _cloud_mode = FeatureFlag::CloudMode.override_enabled(true);
        let _cloud_mode_input_v2 = FeatureFlag::CloudModeInputV2.override_enabled(true);

        let terminal = add_window_with_cloud_mode_terminal(&mut app);
        let input = terminal.read(&app, |view, _| view.input.clone());

        // The cloud mode v2 submit path now opens a create-environment modal if
        // no environment is selected. Register a stub environment and select it
        // so the test exercises the spawn path instead of the modal-open path.
        let env_id = register_test_cloud_environment(&mut app);
        terminal.update(&mut app, |view, ctx| {
            view.ambient_agent_view_model()
                .expect("cloud mode terminal should have ambient model")
                .update(ctx, |model, ctx| {
                    model.set_environment_id(Some(env_id), ctx);
                });
        });

        input.update(&mut app, |input, ctx| {
            assert!(input.is_cloud_mode_input_v2_composing(ctx));
            input.replace_buffer_content("/agent fix the tests", ctx);
            input.input_enter(ctx);
        });

        terminal.read(&app, |view, ctx| {
            let ambient_model = view
                .ambient_agent_view_model()
                .expect("cloud mode terminal should have ambient model")
                .as_ref(ctx);
            let request = ambient_model
                .request()
                .expect("enter should submit through the cloud agent spawn path");
            assert_eq!(request.prompt.as_deref(), Some("/agent fix the tests"));
            assert_eq!(request.mode, UserQueryMode::Normal);
            assert!(input.as_ref(ctx).buffer_text(ctx).is_empty());
        });
    });
}

/// Registers a stub `CloudAmbientAgentEnvironment` in the test `CloudModel` and
/// returns its `SyncId` so the caller can attach it to an ambient view model.
fn register_test_cloud_environment(app: &mut App) -> SyncId {
    let sync_id = SyncId::ClientId(ClientId::new());
    app.update(|ctx| {
        let environment = AmbientAgentEnvironment::new(
            "Test Environment".to_string(),
            None,
            vec![],
            "ubuntu:latest".to_string(),
            vec![],
        );
        let object = CloudAmbientAgentEnvironment::new(
            sync_id,
            CloudAmbientAgentEnvironmentModel::new(environment),
            CloudObjectMetadata::mock(),
            CloudObjectPermissions::mock_personal(),
        );
        CloudModel::handle(ctx).update(ctx, |model, ctx| {
            model.create_object(sync_id, object, ctx);
        });
    });
    sync_id
}

#[test]
fn fresh_cloud_mode_setup_enters_agent_view_when_view_pending() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let _cloud_mode = FeatureFlag::CloudMode.override_enabled(true);

        let terminal = add_window_with_cloud_mode_terminal(&mut app);

        terminal.update(&mut app, |view, ctx| {
            view.model
                .lock()
                .set_shared_session_status(SharedSessionStatus::ViewPending);

            assert!(!view.agent_view_controller().as_ref(ctx).is_active());
            view.enter_ambient_agent_setup(Some("write the tests".to_string()), ctx);

            assert!(view.agent_view_controller().as_ref(ctx).is_active());
            assert_eq!(view.input().as_ref(ctx).buffer_text(ctx), "write the tests");
        });
    });
}

#[test]
fn shared_third_party_viewer_sync_enters_agent_view_and_retags_existing_block() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let _agent_harness = FeatureFlag::AgentHarness.override_enabled(true);

        let terminal = add_window_with_cloud_mode_terminal(&mut app);

        terminal.update(&mut app, |view, ctx| {
            let harness_block_id = {
                let mut model = view.model.lock();
                model.set_shared_session_source(SharedSessionSource::ambient_agent(None));
                model.set_shared_session_status(SharedSessionStatus::ActiveViewer {
                    role: Default::default(),
                });
                model.simulate_block("claude", "running");
                model
                    .block_list()
                    .blocks()
                    .iter()
                    .find(|block| block.command_to_string() == "claude")
                    .expect("harness block should exist")
                    .id()
                    .clone()
            };

            view.ambient_agent_view_model()
                .expect("cloud mode terminal should have ambient model")
                .update(ctx, |model, ctx| {
                    model.set_harness(Harness::Claude, ctx);
                });

            let conversation_id = view
                .sync_agent_view_for_shared_third_party_viewer(ctx)
                .expect("shared third-party viewer should sync");
            let idempotent_conversation_id = view
                .sync_agent_view_for_shared_third_party_viewer(ctx)
                .expect("sync should be idempotent");
            assert_eq!(conversation_id, idempotent_conversation_id);

            match view.agent_view_controller().as_ref(ctx).agent_view_state() {
                AgentViewState::Active {
                    conversation_id: active_conversation_id,
                    origin,
                    ..
                } => {
                    assert_eq!(*active_conversation_id, conversation_id);
                    assert_eq!(*origin, AgentViewEntryOrigin::ThirdPartyCloudAgent);
                }
                state => panic!("expected active agent view, got {state:?}"),
            }

            let model = view.model.lock();
            let block = model
                .block_list()
                .block_with_id(&harness_block_id)
                .expect("harness block should still exist");
            assert!(!block.should_hide_block(model.block_list().agent_view_state()));
            match block.agent_view_visibility() {
                AgentViewVisibility::Terminal {
                    conversation_ids,
                    pending_conversation_ids,
                } => {
                    assert!(pending_conversation_ids.is_empty());
                    assert!(conversation_ids.contains(&conversation_id));
                }
                visibility => panic!("expected terminal block visibility, got {visibility:?}"),
            }
        });
    });
}

#[test]
fn shared_third_party_viewer_syncs_from_viewer_harness_updated_when_harness_unchanged() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let _agent_harness = FeatureFlag::AgentHarness.override_enabled(true);

        let terminal = add_window_with_cloud_mode_terminal(&mut app);

        terminal.update(&mut app, |view, ctx| {
            let harness_block_id = {
                let mut model = view.model.lock();
                model.set_shared_session_source(SharedSessionSource::ambient_agent(None));
                model.set_shared_session_status(SharedSessionStatus::ActiveViewer {
                    role: Default::default(),
                });
                model.simulate_block("claude", "running");
                model
                    .block_list()
                    .blocks()
                    .iter()
                    .find(|block| block.command_to_string() == "claude")
                    .expect("harness block should exist")
                    .id()
                    .clone()
            };

            view.ambient_agent_view_model()
                .expect("cloud mode terminal should have ambient model")
                .update(ctx, |model, ctx| {
                    model.set_harness(Harness::Claude, ctx);
                    model.set_harness(Harness::Claude, ctx);
                });
            assert!(!view.agent_view_controller().as_ref(ctx).is_active());

            view.handle_ambient_agent_event(
                &AmbientAgentViewModelEvent::ViewerHarnessResolved,
                ctx,
            );

            let AgentViewState::Active {
                conversation_id,
                origin,
                ..
            } = view.agent_view_controller().as_ref(ctx).agent_view_state()
            else {
                panic!("expected active agent view");
            };
            assert_eq!(*origin, AgentViewEntryOrigin::ThirdPartyCloudAgent);

            let model = view.model.lock();
            let block = model
                .block_list()
                .block_with_id(&harness_block_id)
                .expect("harness block should still exist");
            assert!(!block.should_hide_block(model.block_list().agent_view_state()));
            match block.agent_view_visibility() {
                AgentViewVisibility::Terminal {
                    conversation_ids,
                    pending_conversation_ids,
                } => {
                    assert!(pending_conversation_ids.is_empty());
                    assert!(conversation_ids.contains(conversation_id));
                }
                visibility => panic!("expected terminal block visibility, got {visibility:?}"),
            }
        });
    });
}
#[test]
fn shared_third_party_viewer_syncs_from_cli_agent_state_without_ambient_model() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let _agent_harness = FeatureFlag::AgentHarness.override_enabled(true);

        let terminal = add_window_with_terminal(&mut app, None);

        let harness_block_id = terminal.update(&mut app, |view, _| {
            assert!(view.ambient_agent_view_model().is_none());
            let mut model = view.model.lock();
            model.set_shared_session_source(SharedSessionSource::ambient_agent(None));
            model.set_shared_session_status(SharedSessionStatus::ActiveViewer {
                role: Default::default(),
            });
            model.simulate_block("claude", "running");
            model
                .block_list()
                .blocks()
                .iter()
                .find(|block| block.command_to_string() == "claude")
                .expect("harness block should exist")
                .id()
                .clone()
        });

        app.update(|ctx| {
            let guard = RemoteUpdateGuard::new();
            let active_update = guard.start_remote_update();
            apply_cli_agent_state_update(
                &terminal.downgrade(),
                &CLIAgentSessionState::Active {
                    cli_agent: CLIAgent::Claude.to_serialized_name(),
                    is_rich_input_open: false,
                },
                &active_update,
                ctx,
            );
        });

        terminal.read(&app, |view, ctx| {
            let AgentViewState::Active {
                conversation_id,
                origin,
                ..
            } = view.agent_view_controller().as_ref(ctx).agent_view_state()
            else {
                panic!("expected active agent view");
            };
            assert_eq!(*origin, AgentViewEntryOrigin::ThirdPartyCloudAgent);

            let model = view.model.lock();
            let block = model
                .block_list()
                .block_with_id(&harness_block_id)
                .expect("harness block should still exist");
            assert!(!block.should_hide_block(model.block_list().agent_view_state()));
            match block.agent_view_visibility() {
                AgentViewVisibility::Terminal {
                    conversation_ids,
                    pending_conversation_ids,
                } => {
                    assert!(pending_conversation_ids.is_empty());
                    assert!(conversation_ids.contains(conversation_id));
                }
                visibility => panic!("expected terminal block visibility, got {visibility:?}"),
            }
        });
    });
}
#[test]
fn cloud_mode_followup_input_uses_explicit_submit_event_even_when_view_pending() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let _agent_mode = FeatureFlag::AgentMode.override_enabled(true);
        let _cloud_mode = FeatureFlag::CloudMode.override_enabled(true);
        let _handoff = FeatureFlag::HandoffCloudCloud.override_enabled(true);
        let _setup_v2 = FeatureFlag::CloudModeSetupV2.override_enabled(true);

        let terminal = add_window_with_cloud_mode_terminal(&mut app);
        let task_id = AmbientAgentTaskId::from_str("123e4567-e89b-12d3-a456-426614174000")
            .expect("valid task id");

        let ambient_agent_view_model = terminal.update(&mut app, |view, ctx| {
            view.model
                .lock()
                .set_shared_session_status(SharedSessionStatus::ViewPending);
            view.pending_cloud_followup_task_id = Some(task_id);
            let ambient_agent_view_model = view
                .ambient_agent_view_model()
                .expect("cloud mode terminal should have ambient model")
                .clone();
            ambient_agent_view_model.update(ctx, |model, ctx| {
                model.enter_viewing_existing_session(task_id, ctx);
            });

            view.input().update(ctx, |input, ctx| {
                input.set_input_mode_agent(true, ctx);
                input.replace_buffer_content("follow up", ctx);
                input.input_enter(ctx);
            });
            ambient_agent_view_model
        });

        terminal.read(&app, |_view, ctx| {
            assert_eq!(
                ambient_agent_view_model
                    .as_ref(ctx)
                    .pending_followup_prompt(),
                Some("follow up")
            );
        });
    });
}

#[test]
fn pending_cloud_followup_without_ambient_model_restores_prompt() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        app.add_singleton_model(|_| ToastStack);
        let _flag = FeatureFlag::HandoffCloudCloud.override_enabled(true);
        let terminal = add_window_with_terminal(&mut app, None);

        let task_id = AmbientAgentTaskId::from_str("123e4567-e89b-12d3-a456-426614174000")
            .expect("valid task id");

        terminal.update(&mut app, |view, ctx| {
            view.pending_cloud_followup_task_id = Some(task_id);

            assert!(view.try_submit_pending_cloud_followup("follow up".to_string(), ctx));
        });

        terminal.read(&app, |view, ctx| {
            assert_eq!(view.pending_cloud_followup_task_id, None);
            assert_eq!(view.input.as_ref(ctx).buffer_text(ctx), "follow up");
        });
    });
}

#[test]
fn cloud_mode_dispatched_agent_inserts_queued_user_query() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let _cloud_mode = FeatureFlag::CloudMode.override_enabled(true);
        let _handoff = FeatureFlag::HandoffCloudCloud.override_enabled(true);
        let _setup_v2 = FeatureFlag::CloudModeSetupV2.override_enabled(true);

        let terminal = add_window_with_cloud_mode_terminal(&mut app);

        terminal.update(&mut app, |view, ctx| {
            view.ambient_agent_view_model()
                .expect("cloud mode terminal should have ambient model")
                .update(ctx, |model, ctx| {
                    model.spawn_agent_with_request(
                        SpawnAgentRequest {
                            prompt: Some("write the tests".to_string()),
                            mode: UserQueryMode::Normal,
                            config: None,
                            title: None,
                            team: None,
                            agent_identity_uid: None,
                            skill: None,
                            attachments: vec![],
                            interactive: None,
                            parent_run_id: None,
                            runtime_skills: vec![],
                            referenced_attachments: vec![],
                            conversation_id: None,
                            initial_snapshot_token: None,
                            snapshot_disabled: None,
                            orchestration_handoff: None,
                        },
                        ctx,
                    );
                });
            view.handle_ambient_agent_event(&AmbientAgentViewModelEvent::DispatchedAgent, ctx);

            assert!(has_pending_user_query_block(view));
        });
    });
}

#[test]
fn cloud_mode_failed_keeps_queued_query_above_tombstone_and_hides_input() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let _cloud_mode = FeatureFlag::CloudMode.override_enabled(true);
        let _handoff = FeatureFlag::HandoffCloudCloud.override_enabled(true);
        let _setup_v2 = FeatureFlag::CloudModeSetupV2.override_enabled(true);

        let terminal = add_window_with_cloud_mode_terminal(&mut app);

        terminal.update(&mut app, |view, ctx| {
            view.model
                .lock()
                .set_shared_session_status(SharedSessionStatus::ViewPending);
            view.enter_ambient_agent_setup(None, ctx);
            view.insert_cloud_mode_queued_user_query_block("queued prompt".to_string(), ctx);
            assert!(has_pending_user_query_block(view));
            let pending_query_view_id = view
                .pending_user_query_view_id
                .expect("queued query should have a view id");

            view.handle_ambient_agent_event(
                &AmbientAgentViewModelEvent::Failed {
                    error_message: "setup failed".to_string(),
                },
                ctx,
            );

            assert!(has_pending_user_query_block(view));
            assert!(view.conversation_ended_tombstone_view_id.is_some());
            assert_eq!(view.rich_content_views.len(), 2);
            {
                let model = view.model.lock();
                assert!(!view.is_input_box_visible(&model, ctx));
                let tombstone_view_id = view
                    .conversation_ended_tombstone_view_id
                    .expect("failed cloud mode should insert a tombstone");
                let rich_content_view_ids = model
                    .block_list()
                    .block_heights()
                    .items()
                    .iter()
                    .filter_map(|item| {
                        match item {
                            crate::terminal::model::blocks::BlockHeightItem::RichContent(item) => {
                                Some(item.view_id)
                            }
                            crate::terminal::model::blocks::BlockHeightItem::Block(_)
                            | crate::terminal::model::blocks::BlockHeightItem::Gap(_)
                            | crate::terminal::model::blocks::BlockHeightItem::RestoredBlockSeparator {
                                ..
                            }
                            | crate::terminal::model::blocks::BlockHeightItem::InlineBanner { .. }
                            | crate::terminal::model::blocks::BlockHeightItem::SubshellSeparator {
                                ..
                            } => None,
                        }
                    })
                    .collect::<Vec<_>>();
                let pending_query_position = rich_content_view_ids
                    .iter()
                    .position(|view_id| *view_id == pending_query_view_id)
                    .expect("queued query should be in the block list");
                let tombstone_position = rich_content_view_ids
                    .iter()
                    .position(|view_id| *view_id == tombstone_view_id)
                    .expect("tombstone should be in the block list");
                assert!(pending_query_position < tombstone_position);
            }

            view.handle_ambient_agent_event(
                &AmbientAgentViewModelEvent::Failed {
                    error_message: "setup failed again".to_string(),
                },
                ctx,
            );
            assert_eq!(view.rich_content_views.len(), 2);
        });

        let window_id = app.read(|ctx| terminal.window_id(ctx));
        let tombstones = app
            .views_of_type::<ConversationEndedTombstoneView>(window_id)
            .expect("window should have tombstone views");
        let tombstone = tombstones
            .last()
            .expect("failed cloud mode should insert a tombstone");
        tombstone.read(&app, |tombstone, _| {
            assert_eq!(
                tombstone.title_for_test(),
                Some("Cloud agent failed to start")
            );
            assert_eq!(
                tombstone.error_message_for_test(),
                Some("setup failed again")
            );
            assert_eq!(tombstone.credits_for_test(), None);
            assert!(!tombstone.has_continue_in_cloud_button_for_test());
            assert!(!tombstone.has_continue_locally_button_for_test());
        });
    });
}

#[test]
fn cmd_enter_from_terminal_without_selected_block_enters_agent_view() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        app.add_singleton_model(ImportedConfigModel::new);
        app.update(|ctx| {
            crate::terminal::init(ctx);
            crate::editor::init(ctx);
        });
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);

        let (window_id, terminal) = add_window_with_id_and_terminal(&mut app, None);

        terminal.update(&mut app, |view, ctx| {
            assert!(view
                .ai_context_model
                .as_ref(ctx)
                .pending_context_block_ids()
                .is_empty());
            view.focus_terminal(ctx);
        });

        let keystroke = if cfg!(target_os = "macos") {
            "cmd-enter"
        } else {
            "ctrl-shift-enter"
        };
        let handled = app
            .dispatch_keystroke(
                window_id,
                &[terminal.id()],
                &warpui::keymap::Keystroke::parse(keystroke).expect("valid keystroke"),
                false,
            )
            .expect("dispatch should succeed");
        assert!(
            handled,
            "{keystroke} should be handled from terminal context"
        );

        terminal.read(&app, |view, ctx| {
            assert!(view
                .agent_view_controller()
                .as_ref(ctx)
                .agent_view_state()
                .is_fullscreen());
            assert!(view
                .ai_context_model
                .as_ref(ctx)
                .pending_context_block_ids()
                .is_empty());
        });
    });
}

#[test]
fn cmd_enter_from_terminal_with_selected_block_enters_agent_view_with_context() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        app.add_singleton_model(ImportedConfigModel::new);
        app.update(|ctx| {
            crate::terminal::init(ctx);
            crate::editor::init(ctx);
        });
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);

        let (window_id, terminal) = add_window_with_id_and_terminal(&mut app, None);

        let selected_block_id = terminal.update(&mut app, |view, ctx| {
            let (selected_block_index, selected_block_id) = {
                let mut model = view.model.lock();
                model.simulate_block("echo selected", "selected");
                let block = model
                    .block_list()
                    .blocks()
                    .iter()
                    .find(|block| block.command_to_string() == "echo selected")
                    .expect("simulated block should exist");
                (block.index(), block.id().clone())
            };

            view.integration_test_change_block_selection_to_single(selected_block_index, ctx);
            assert!(view
                .ai_context_model
                .as_ref(ctx)
                .pending_context_block_ids()
                .contains(&selected_block_id));
            view.focus_terminal(ctx);
            selected_block_id
        });

        let keystroke = if cfg!(target_os = "macos") {
            "cmd-enter"
        } else {
            "ctrl-shift-enter"
        };
        let handled = app
            .dispatch_keystroke(
                window_id,
                &[terminal.id()],
                &warpui::keymap::Keystroke::parse(keystroke).expect("valid keystroke"),
                false,
            )
            .expect("dispatch should succeed");
        assert!(
            handled,
            "{keystroke} should be handled from terminal context"
        );

        terminal.read(&app, |view, ctx| {
            let conversation_id = view
                .agent_view_controller()
                .as_ref(ctx)
                .agent_view_state()
                .active_conversation_id()
                .expect("agent view should be active");

            let model = view.model.lock();
            let block = model
                .block_list()
                .block_with_id(&selected_block_id)
                .expect("selected block should still exist");
            assert!(
                !block.should_hide_block(model.block_list().agent_view_state()),
                "selected block should remain visible in the new agent conversation"
            );
            match block.agent_view_visibility() {
                AgentViewVisibility::Terminal {
                    pending_conversation_ids,
                    ..
                } => {
                    assert!(
                        pending_conversation_ids.contains(&conversation_id),
                        "selected block should be attached as pending context for the new conversation"
                    );
                }
                visibility => panic!("expected terminal block visibility, got {visibility:?}"),
            }
        });
    });
}

#[test]
fn cmd_enter_from_active_non_empty_agent_view_requires_confirmation() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        app.add_singleton_model(ImportedConfigModel::new);
        app.update(|ctx| {
            crate::terminal::init(ctx);
            crate::editor::init(ctx);
        });
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);

        let (window_id, terminal) = add_window_with_id_and_terminal(&mut app, None);

        let original_conversation_id = terminal.update(&mut app, |view, ctx| {
            let (conversation_id, _, _, _) =
                append_exchange_and_handle_event(view, agent_jump_user_query("first"), ctx);
            view.enter_agent_view_for_conversation(
                None,
                AgentViewEntryOrigin::ConversationSelector,
                conversation_id,
                ctx,
            );
            view.focus_terminal(ctx);
            conversation_id
        });

        let keystroke = if cfg!(target_os = "macos") {
            "cmd-enter"
        } else {
            "ctrl-shift-enter"
        };
        let keystroke = warpui::keymap::Keystroke::parse(keystroke).expect("valid keystroke");

        let handled = app
            .dispatch_keystroke(window_id, &[terminal.id()], &keystroke, false)
            .expect("dispatch should succeed");
        assert!(
            handled,
            "new conversation keybinding should be handled from terminal context"
        );

        terminal.read(&app, |view, ctx| {
            assert_eq!(
                view.agent_view_controller()
                    .as_ref(ctx)
                    .agent_view_state()
                    .active_conversation_id(),
                Some(original_conversation_id),
                "first keybinding press should keep the current conversation active"
            );
        });

        let handled = app
            .dispatch_keystroke(window_id, &[terminal.id()], &keystroke, false)
            .expect("dispatch should succeed");
        assert!(
            handled,
            "new conversation keybinding should be handled from terminal context"
        );

        terminal.read(&app, |view, ctx| {
            let new_conversation_id = view
                .agent_view_controller()
                .as_ref(ctx)
                .agent_view_state()
                .active_conversation_id()
                .expect("agent view should be active");
            assert_ne!(
                new_conversation_id, original_conversation_id,
                "second keybinding press should start a new conversation"
            );
        });
    });
}

#[test]
fn cloud_mode_followup_dispatched_inserts_queued_user_query() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let _cloud_mode = FeatureFlag::CloudMode.override_enabled(true);
        let _handoff = FeatureFlag::HandoffCloudCloud.override_enabled(true);
        let _setup_v2 = FeatureFlag::CloudModeSetupV2.override_enabled(true);

        let terminal = add_window_with_cloud_mode_terminal(&mut app);
        let task_id = AmbientAgentTaskId::from_str("123e4567-e89b-12d3-a456-426614174000")
            .expect("valid task id");

        terminal.update(&mut app, |view, ctx| {
            view.ambient_agent_view_model()
                .expect("cloud mode terminal should have ambient model")
                .update(ctx, |model, ctx| {
                    model.enter_viewing_existing_session(task_id, ctx);
                    model.submit_cloud_followup("follow up".to_string(), ctx);
                });
            view.handle_ambient_agent_event(&AmbientAgentViewModelEvent::FollowupDispatched, ctx);

            assert!(has_pending_user_query_block(view));
        });
    });
}

#[test]
fn cloud_mode_setup_v2_suppresses_sharer_input_updates_while_followup_setup_commands_run() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let _cloud_mode = FeatureFlag::CloudMode.override_enabled(true);
        let _handoff = FeatureFlag::HandoffCloudCloud.override_enabled(true);
        let _setup_v2 = FeatureFlag::CloudModeSetupV2.override_enabled(true);
        let setup_command_ops = input_operations_for_buffer_content(&mut app, "setup command text");
        let normal_input_ops = input_operations_for_buffer_content(&mut app, "normal sync text");

        let terminal = add_window_with_cloud_mode_terminal(&mut app);
        let task_id = AmbientAgentTaskId::from_str("123e4567-e89b-12d3-a456-426614174000")
            .expect("valid task id");

        terminal.update(&mut app, |view, ctx| {
            let ambient_agent_view_model = view
                .ambient_agent_view_model()
                .expect("cloud mode terminal should have ambient model")
                .clone();
            ambient_agent_view_model.update(ctx, |model, ctx| {
                model.enter_viewing_existing_session(task_id, ctx);
            });
            view.handle_ambient_agent_event(&AmbientAgentViewModelEvent::FollowupDispatched, ctx);

            {
                let model = view.model.lock();
                assert!(view.is_input_box_visible(&model, ctx));
            }
            assert!(view.should_suppress_ambient_setup_input_sync(ctx));

            let active_block_id = view.model.lock().block_list().active_block_id().clone();
            view.input().update(ctx, |input, ctx| {
                input.refresh_deferred_remote_operations(ctx);
            });
            view.apply_viewer_shared_session_input_update(&active_block_id, setup_command_ops, ctx);
            assert_eq!(view.input().as_ref(ctx).buffer_text(ctx), "");
            ambient_agent_view_model.update(ctx, |model, _| {
                model
                    .setup_command_state_mut()
                    .set_did_execute_a_setup_command(true);
            });
            assert!(view.should_suppress_ambient_setup_input_sync(ctx));

            ambient_agent_view_model.update(ctx, |model, _| {
                let group_id = model.setup_command_state().current_group_id();
                model.setup_command_state_mut().finish_group(group_id);
            });
            assert!(!view.should_suppress_ambient_setup_input_sync(ctx));
            view.apply_viewer_shared_session_input_update(&active_block_id, normal_input_ops, ctx);
            assert_eq!(
                view.input().as_ref(ctx).buffer_text(ctx),
                "normal sync text"
            );

            let model = view.model.lock();
            assert!(view.is_input_box_visible(&model, ctx));
        });
    });
}

#[test]
fn pending_cloud_mode_query_waits_for_renderable_user_query_exchange() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);

        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |view, ctx| {
            view.insert_cloud_mode_queued_user_query_block("queued prompt".to_string(), ctx);
            assert!(has_pending_user_query_block(view));

            append_exchange_and_handle_event(
                view,
                AIAgentInput::ResumeConversation {
                    context: Default::default(),
                },
                ctx,
            );
            assert!(has_pending_user_query_block(view));

            append_exchange_and_handle_event(
                view,
                AIAgentInput::UserQuery {
                    query: "real prompt".to_string(),
                    context: Default::default(),
                    static_query_type: None,
                    referenced_attachments: Default::default(),
                    user_query_mode: UserQueryMode::default(),
                    running_command: None,
                    intended_agent: None,
                },
                ctx,
            );
            assert!(!has_pending_user_query_block(view));
        });
    });
}

#[test]
fn pending_cloud_mode_query_clears_when_streaming_exchange_becomes_renderable() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);

        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |view, ctx| {
            view.insert_cloud_mode_queued_user_query_block(
                "write a poem about rocks".to_string(),
                ctx,
            );
            assert!(has_pending_user_query_block(view));

            let (conversation_id, _, exchange_id, response_stream_id) =
                append_exchange_with_inputs_and_handle_event(view, vec![], ctx);
            assert!(has_pending_user_query_block(view));

            update_exchange_input_and_handle_event(
                view,
                conversation_id,
                exchange_id,
                response_stream_id,
                vec![AIAgentInput::UserQuery {
                    query: "write an ode about stones".to_string(),
                    context: Default::default(),
                    static_query_type: None,
                    referenced_attachments: Default::default(),
                    user_query_mode: UserQueryMode::Normal,
                    running_command: None,
                    intended_agent: None,
                }],
                ctx,
            );
            assert!(!has_pending_user_query_block(view));

            let conversation = BlocklistAIHistoryModel::as_ref(ctx)
                .conversation(&conversation_id)
                .expect("conversation should exist");
            let initial_user_query = conversation.initial_user_query();
            let exchange = conversation
                .exchange_with_id(exchange_id)
                .expect("exchange should exist");
            assert_eq!(
                exchange.input[0]
                    .display_user_query(initial_user_query.as_ref())
                    .as_deref(),
                Some("/agent write an ode about stones")
            );
        });
    });
}

/// Test clearing of session flag state when terminal is cleared
#[test]
fn test_clear_session_flag_state() {
    use warp_terminal::shell::ShellType;

    use crate::ai::blocklist::SerializedBlockListItem;
    use crate::terminal::model::block::SerializedBlock;
    use crate::terminal::ShellHost;

    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        // Create a remote restored block
        let mut remote_block =
            SerializedBlock::new_for_test("echo remote".into(), "remote output".into());
        remote_block.is_local = Some(false); // Mark it as a remote block
        remote_block.shell_host = Some(ShellHost {
            shell_type: ShellType::Bash,
            user: "user".to_string(),
            hostname: "remote".to_string(), // Remote hostname indicates a remote session
        });

        // Convert to SerializedBlockListItem
        let restored_blocks = [SerializedBlockListItem::Command {
            block: Box::new(remote_block),
        }];

        // Create terminal with the restored remote block
        let terminal = add_window_with_terminal(&mut app, Some(&restored_blocks));

        terminal.update(&mut app, |view, ctx| {
            // Verify initial state - block was created as remote and restored
            assert!(
                !view.any_session_contains_remote_blocks,
                "Terminal should not have remote blocks"
            );
            assert!(
                view.any_session_contains_restored_remote_blocks,
                "Terminal should have restored remote blocks"
            );

            {
                // Verify the block was properly created with correct properties
                let model = view.model.lock();
                let blocks = model.block_list().blocks();

                // The first block should be our restored remote block
                assert!(!blocks.is_empty(), "At least one block should exist");
                if let Some(first_block) = blocks.first() {
                    assert_eq!(
                        first_block.restored_block_was_local(),
                        Some(false),
                        "First block should be marked as a remote restored block"
                    );
                }
            }

            // Now clear the terminal
            view.clear_buffer_for_testing(ctx);

            // Flags should be reset
            assert!(
                !view.any_session_contains_remote_blocks,
                "Terminal should not have remote blocks after clearing"
            );
            assert!(
                !view.any_session_contains_restored_remote_blocks,
                "Terminal should not have restored remote blocks after clearing"
            );
        });
    })
}

fn assert_block_has_find_match(find_model: &TerminalFindModel, block_index: BlockIndex) {
    assert!(find_model
        .block_list_find_run()
        .is_some_and(|run| run.matches_for_block(block_index).next().is_some()));
}

impl TerminalView {
    fn is_top_of_active_block_in_viewport(
        &self,
        model: &TerminalModel,
        input_mode: InputMode,
        app: &AppContext,
    ) -> bool {
        let active_block_index = model.block_list().active_block_index();
        let viewport = self.viewport_state(model.block_list(), input_mode, app);
        viewport.is_block_in_view(active_block_index, BlockVisibilityMode::TopOfBlockVisible)
    }

    fn scroll_top_in_lines(
        &self,
        model: &TerminalModel,
        input_mode: InputMode,
        app: &AppContext,
    ) -> Lines {
        let viewport = self.viewport_state(model.block_list(), input_mode, app);
        viewport.scroll_top_in_lines()
    }

    fn is_vertically_scrollable(&self, app: &AppContext) -> bool {
        let total_block_heights = self
            .model
            .lock()
            .block_list()
            .block_heights()
            .summary()
            .height;
        let visible_rows = self.content_element_height_lines(app);
        heights_approx_gt(total_block_heights, visible_rows)
    }
}

fn read_from_clipboard(ctx: &mut ViewContext<TerminalView>) -> String {
    TerminalView::read_from_clipboard(Some(ShellFamily::Posix), ctx)
}

#[test]
fn test_insert() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);

        let select_text = |view: &mut TerminalView, ctx: &mut ViewContext<TerminalView>| {
            {
                let mut model = view.model.lock();
                model.start_command_execution();
                let blocks = model.block_list_mut();
                blocks.input('f');
                blocks.linefeed();
                blocks.preexec(PreexecValue::default());
                blocks.on_finish_byte_processing(&ansi::ProcessorInput::new(&[]));
            }
            view.begin_block_text_selection(
                BlockListPoint::new(1.0, 1),
                Side::Right,
                SelectionType::Semantic,
                Vector2F::zero(),
                ctx,
            );
            view.end_text_selection(ctx);
        };
        let assert_input_text_eq = |app: &mut App, expected_text: &str| {
            terminal.read(app, |view, _ctx| {
                view.input.read(app, |view, ctx| {
                    assert_eq!(view.buffer_text(ctx), String::from(expected_text));
                });
            });
        };
        let assert_selected_blocks_cardinality_eq =
            |app: &mut App, expected_cardinality: BlockSelectionCardinality| {
                terminal.read(app, |view, _ctx| {
                    assert_eq!(
                        view.selected_blocks.cardinality().as_keymap_context_value(),
                        expected_cardinality.as_keymap_context_value()
                    );
                });
            };
        let assert_selected_text_eq = |app: &mut App, expected_text: Option<String>| {
            terminal.update(app, |view, ctx| {
                let semantic_selection = SemanticSelection::as_ref(ctx);
                let model = view.model.lock();
                let context_selected_text =
                    model.selection_to_string(semantic_selection, false, ctx);
                assert_eq!(context_selected_text, expected_text);
            });
        };

        // Shell Mode: Nothing selected
        terminal.update(&mut app, |view, ctx| {
            view.focus_terminal(ctx);
            view.typed_characters_on_terminal("hello", ctx);
        });
        assert_input_text_eq(&mut app, "hello");
        assert_selected_blocks_cardinality_eq(&mut app, BlockSelectionCardinality::None);
        assert_selected_text_eq(&mut app, None);

        // Shell Mode: Block selected
        terminal.update(&mut app, |view, ctx| {
            view.selected_blocks.reset_to_single(BlockIndex::zero());
            view.focus_terminal(ctx);
            view.typed_characters_on_terminal("_this", ctx);
        });
        assert_input_text_eq(&mut app, "hello_this");
        assert_selected_blocks_cardinality_eq(&mut app, BlockSelectionCardinality::None);
        assert_selected_text_eq(&mut app, None);

        // Shell Mode: Text selected
        terminal.update(&mut app, |view, ctx| {
            select_text(view, ctx);
            view.focus_terminal(ctx);
            view.typed_characters_on_terminal("_is", ctx);
        });
        assert_input_text_eq(&mut app, "hello_this_is");
        assert_selected_blocks_cardinality_eq(&mut app, BlockSelectionCardinality::None);
        assert_selected_text_eq(&mut app, None);

        // Activate Agent Mode, which should no longer allow text insertion to clear the selected block(s) or text
        terminal.update(&mut app, |view, ctx| {
            view.set_ai_input_mode_with_query(None, ctx);
        });

        // Agent Mode: Nothing selected
        terminal.update(&mut app, |view, ctx| {
            view.focus_terminal(ctx);
            view.typed_characters_on_terminal("_your", ctx);
        });
        assert_input_text_eq(&mut app, "hello_this_is_your");
        assert_selected_blocks_cardinality_eq(&mut app, BlockSelectionCardinality::None);
        assert_selected_text_eq(&mut app, None);

        // Agent Mode: Block selected
        terminal.update(&mut app, |view, ctx| {
            view.selected_blocks.reset_to_single(BlockIndex::zero());
            view.focus_terminal(ctx);
            view.typed_characters_on_terminal("_captain", ctx);
        });
        assert_input_text_eq(&mut app, "hello_this_is_your_captain");
        assert_selected_blocks_cardinality_eq(&mut app, BlockSelectionCardinality::One);
        assert_selected_text_eq(&mut app, None);

        // Agent Mode: Text selected
        terminal.update(&mut app, |view, ctx| {
            select_text(view, ctx);
            view.focus_terminal(ctx);
            view.typed_characters_on_terminal("_speaking", ctx);
        });
        assert_input_text_eq(&mut app, "hello_this_is_your_captain_speaking");
        assert_selected_blocks_cardinality_eq(&mut app, BlockSelectionCardinality::None);
        assert_selected_text_eq(&mut app, Some("f".to_owned()));
    })
}

const BODY_PREFIX: &str = "Latest output: ";

/// Regression test for CORE-1654. Tests the "Insert into Input" functionality from the context menu.
#[test]
fn test_insert_into_input() {
    // Note that this is defined as a unit test rather than an integration test since it requires precise selections
    // (where we don't want UI updates making the test brittle, due to hardcoded mouse positions).
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);

        // TODO: Potentially explore if we can re-use helpers from `input_test.rs` (`select_first_command_line_of_block` and `insert_dummy_block`).
        terminal.update(&mut app, |terminal_view, ctx| {
            {
                let mut terminal_model = terminal_view.model.lock();
                let blocks = terminal_model.block_list_mut();
                // Add two lines to the command grid and output grid in a new block.
                let block_index = insert_block(blocks, "cmd_a\ncmd_b\n", "output_a\noutput_b\n");
                let block = blocks.block_at(block_index).expect("block should exist");
                // Selections are inclusive of endpoint, hence we need to identify the last column to select the first command.
                let block_command_columns =
                    block.prompt_and_command_grid().grid_handler().columns();
                let command_grid_offset = block.command_grid_offset();
                // Create a selection that just spans the first line of the command grid in the block.
                blocks.start_selection(
                    BlockListPoint::new(command_grid_offset, 0),
                    SelectionType::Simple,
                    Side::Left,
                );
                blocks.update_selection(
                    BlockListPoint::new(command_grid_offset, block_command_columns),
                    Side::Right,
                );
                let selection = blocks.selection();
                assert!(selection.is_some());
            }

            terminal_view.context_menu_insert_selected_text(ctx);
        });

        // Confirm that the blocklist selection is cleared upon inserting into the input box.
        terminal.read(&app, |terminal_view, _ctx| {
            let terminal_model = terminal_view.model.lock();
            let blocks = terminal_model.block_list();
            let selection = blocks.selection();
            assert!(
                selection.is_none(),
                "Expected no selections in the blocklist but got {selection:?}"
            );
        });
        let input = terminal.read(&app, |terminal, _ctx| terminal.input().clone());
        // Confirm that the input box has the correct text (the first line of the command grid was selected above).
        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "cmd_a");
        });
    });
}

#[test]
fn test_copy_on_select() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let terminal = add_window_with_terminal(&mut app, None);
        // Add some text and make sure we update the selection
        terminal.update(&mut app, |view, ctx| {
            {
                let mut model = view.model.lock();
                model.start_command_execution();
                let blocks = model.block_list_mut();

                blocks.input('f');
                blocks.input('o');
                blocks.input('o');

                blocks.linefeed();

                blocks.preexec(PreexecValue::default());

                blocks.on_finish_byte_processing(&ansi::ProcessorInput::new(&[]));
            }

            view.begin_block_text_selection(
                BlockListPoint::new(1.0, 1),
                Side::Right,
                SelectionType::Semantic,
                Vector2F::zero(),
                ctx,
            );

            let selection_settings = SelectionSettings::as_ref(ctx);
            assert!(selection_settings.copy_on_select_enabled());
            assert_eq!("", &read_from_clipboard(ctx));
            view.end_text_selection(ctx);
            assert_eq!("foo", &read_from_clipboard(ctx));
        });
    })
}

#[test]
fn test_alt_screen_copy_on_select() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let terminal = add_window_with_terminal(&mut app, None);
        terminal.update(&mut app, |view, ctx| {
            {
                // Enter alt screen and add text
                let mut model = view.model.lock();
                model.set_mode(ansi::Mode::SwapScreen {
                    save_cursor_and_clear_screen: true,
                });
                assert!(model.is_alt_screen_active());

                model.alt_screen_mut().input('h');
            }
            // Ensure copy on select is enabled
            let selection_settings = SelectionSettings::as_ref(ctx);
            assert!(selection_settings.copy_on_select_enabled());

            // Select input
            view.begin_alt_selection(Point::new(0, 0), Side::Left, SelectionType::Simple, ctx);
            assert_eq!("", &read_from_clipboard(ctx));
            view.update_alt_selection(Point::new(0, 2), Side::Left, &Lines::zero(), ctx);
            view.end_alt_selection(ctx);
            // Ensure selection is copied
            assert_eq!("h", &read_from_clipboard(ctx));
        });
    })
}

#[test]
fn test_alt_screen_select_with_sgr_mouse() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let (window_id, terminal) = add_window_with_id_and_terminal(&mut app, None);

        let mut updated = HashSet::new();
        updated.insert(app.root_view_id(window_id).unwrap());
        let invalidation = WindowInvalidation {
            updated,
            ..Default::default()
        };
        let presenter = Rc::new(RefCell::new(Presenter::new(window_id)));

        let semantic_selection = SemanticSelection::mock(true, "");

        let size_info = terminal.update(&mut app, |view, ctx| {
            {
                // Enter alt screen and enable SGR Mouse
                let mut model = view.model.lock();
                model.set_mode(ansi::Mode::SwapScreen {
                    save_cursor_and_clear_screen: true,
                });
                model.set_mode(ansi::Mode::SgrMouse);
                assert!(model.is_alt_screen_active());
                assert!(!should_intercept_mouse(&model, false, ctx));
                assert!(should_intercept_mouse(&model, true, ctx));

                // Write a bunch of characters into the alt screen.
                // ABCDEFG
                // HIJKLMN
                // OPQRSTU
                // VWXYZ[\
                // ]^_`abc
                // defghij
                // klmnopq
                // rstuvwx
                // yz{|}~
                // € ‚ƒ„…†
                // ‡ˆ‰Š‹Œ
                let mut ascii: u8 = 65;
                for _ in 0..view.size_info.rows {
                    for _ in 0..view.size_info.columns {
                        model.alt_screen_mut().input(ascii as char);
                        ascii += 1;
                    }
                }

                *view.size_info
            }
        });

        // We need to manually trigger re-renders to ensure the AltScreenElement is recreated, e.g.
        // so its `is_terminal_selecting` property will be up-to-date.
        macro_rules! rerender {
            ($app:ident, $presenter:expr, $invalidation:expr, $size_info:expr) => {
                app.update(enclose!((presenter, invalidation) move |ctx| {
                    presenter
                        .borrow_mut()
                        .invalidate(invalidation, ctx);
                    presenter.borrow_mut().build_scene(
                        vec2f(size_info.pane_width_px, size_info.pane_height_px),
                        1.,
                        None,
                        ctx,
                    );
                }));
            }
        }

        // The start and end positions corresponds to 'J'
        // and 'a' in the grid, respectively.
        //
        // We adjust the vertical coordinates to account for padding
        // in the alt-screen.
        let start_position = vec2f(
            2. * size_info.cell_width_px.as_f32(),
            2. * size_info.cell_height_px.as_f32() - 1.,
        );
        let end_position = vec2f(
            5. * size_info.cell_width_px.as_f32(),
            5. * size_info.cell_height_px.as_f32() - 1.,
        );

        // Simulate a mouse drag from the "J" to the "a" cell.
        rerender!(app, presenter, invalidation, size_info);
        app.update(enclose!((presenter) move |ctx| {
            ctx.simulate_window_event(
                warpui::Event::LeftMouseDown {
                    position: start_position,
                    modifiers: Default::default(),
                    click_count: 1,
                    is_first_mouse: false,
                },
                window_id,
                presenter.clone(),
            );
        }));
        rerender!(app, presenter, invalidation, size_info);
        app.update(enclose!((presenter) move |ctx| {
            ctx.simulate_window_event(
                warpui::Event::LeftMouseDragged {
                    position: end_position,
                    modifiers: Default::default(),
                },
                window_id,
                presenter.clone(),
            );
        }));
        rerender!(app, presenter, invalidation, size_info);
        app.update(enclose!((presenter) move |ctx| {
            ctx.simulate_window_event(
                warpui::Event::LeftMouseUp {
                    position: end_position,
                    modifiers: Default::default(),
                },
                window_id,
                presenter.clone(),
            );
        }));

        // No selection should've occurred as we aren't intercepting mouse events.
        terminal.read(&app, |view, ctx| {
            let selected_text =
                view.model
                    .lock()
                    .selection_to_string(&semantic_selection, false, ctx);
            assert_eq!(selected_text, None);
        });

        // This time, hold Shift key for all mouse events.
        rerender!(app, presenter, invalidation, size_info);
        app.update(enclose!((presenter) move |ctx| {
            ctx.simulate_window_event(
                warpui::Event::LeftMouseDown {
                    position: start_position,
                    modifiers: ModifiersState {
                        shift: true,
                        ..Default::default()
                    },
                    click_count: 1,
                    is_first_mouse: false,
                },
                window_id,
                presenter.clone(),
            );
        }));
        rerender!(app, presenter, invalidation, size_info);
        app.update(enclose!((presenter) move |ctx| {
            ctx.simulate_window_event(
                warpui::Event::LeftMouseDragged {
                    position: end_position,
                    modifiers: ModifiersState {
                        shift: true,
                        ..Default::default()
                    },
                },
                window_id,
                presenter.clone(),
            );
        }));
        rerender!(app, presenter, invalidation, size_info);
        app.update(enclose!((presenter) move |ctx| {
            ctx.simulate_window_event(
                warpui::Event::LeftMouseUp {
                    position: end_position,
                    modifiers: ModifiersState {
                        shift: true,
                        ..Default::default()
                    },
                },
                window_id,
                presenter.clone(),
            );
        }));

        // This time we expect a selection since the Shift key had been held for this mouse drag.
        terminal.read(&app, |view, ctx| {
            let selected_text =
                view.model
                    .lock()
                    .selection_to_string(&semantic_selection, false, ctx);
            assert_eq!(selected_text.as_ref().unwrap(), "JKLMNOPQRSTUVWXYZ[\\]^_`a");
        });
    })
}

// Regression test for WAR-3433 on find bar selection crash.
#[test]
fn test_find_bar_select() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let terminal = add_window_with_terminal(&mut app, None);
        // Add some text and make sure we update the selection
        terminal.update(&mut app, |view, ctx| {
            // Mock a block with content 'foo g'.
            {
                let mut model = view.model.lock();
                model.start_command_execution();
                let blocks = model.block_list_mut();

                blocks.input('f');
                blocks.input('o');
                blocks.input('o');

                blocks.input(' ');
                blocks.input('g');

                blocks.linefeed();

                blocks.preexec(PreexecValue::default());

                blocks.on_finish_byte_processing(&ansi::ProcessorInput::new(&[]));
            }

            // Select 'foo'.
            view.begin_block_text_selection(
                BlockListPoint::new(1.0, 1),
                Side::Right,
                SelectionType::Semantic,
                Vector2F::zero(),
                ctx,
            );

            let selection_settings = SelectionSettings::as_ref(ctx);
            assert!(selection_settings.copy_on_select_enabled());
            assert_eq!("", &read_from_clipboard(ctx));
            view.end_text_selection(ctx);
            assert_eq!("foo", &read_from_clipboard(ctx));

            // Show find bar. The find bar should have selected text 'foo' in its editor.
            view.show_find_bar(ctx);
            view.find_bar.read(ctx, |find, ctx| {
                find.editor().read(ctx, |editor, ctx| {
                    assert_eq!("foo".to_string(), editor.selected_text(ctx));
                })
            });

            // Now select 'foo g'.
            view.begin_block_text_selection(
                BlockListPoint::new(1.0, 1),
                Side::Right,
                SelectionType::Lines,
                Vector2F::zero(),
                ctx,
            );

            let selection_settings = SelectionSettings::as_ref(ctx);
            assert!(selection_settings.copy_on_select_enabled());
            assert_eq!("foo", &read_from_clipboard(ctx));
            view.end_text_selection(ctx);
            assert_eq!("foo g", &read_from_clipboard(ctx));

            // Show find bar. The find bar should have selected text 'foo g' in its editor.
            view.show_find_bar(ctx);
            view.find_bar.read(ctx, |find, ctx| {
                find.editor().read(ctx, |editor, ctx| {
                    assert_eq!("foo g".to_string(), editor.selected_text(ctx));
                })
            });
        });
    })
}

#[test]
fn test_viewport_iter_most_recent_at_bottom() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let terminal = add_window_with_terminal(&mut app, None);
        terminal.update(&mut app, |view, ctx| {
            let mut model = view.model.lock();
            model.simulate_block("ls", "foo");
            model.simulate_block("echo multiline", "bar\nhey");
            let viewport = view.viewport_state(model.block_list(), InputMode::PinnedToBottom, ctx);
            let mut iter = viewport.iter();
            let first_block = iter.next().expect("item 1");
            assert_eq!(
                Some(std::convert::Into::<BlockIndex>::into(1)),
                first_block.block_index
            );
            assert_eq!(
                std::convert::Into::<TotalIndex>::into(1),
                first_block.entry_index
            );
            assert!(first_block.block_height_item.height().into_lines() > Lines::zero());
            assert_eq!(
                Some(std::convert::Into::<BlockIndex>::into(1)),
                viewport.topmost_visible_block()
            );

            let second_block = iter.next().expect("item 2");
            assert_eq!(
                Some(std::convert::Into::<BlockIndex>::into(2)),
                second_block.block_index
            );
            assert_eq!(
                std::convert::Into::<TotalIndex>::into(2),
                second_block.entry_index
            );
            assert!(
                second_block.block_height_item.height() > first_block.block_height_item.height()
            );
            assert!(viewport.is_block_in_view(
                std::convert::Into::<BlockIndex>::into(2),
                BlockVisibilityMode::TopOfBlockVisible
            ));

            let third_block = iter.next().expect("item 3");
            assert_eq!(
                Some(std::convert::Into::<BlockIndex>::into(3)),
                third_block.block_index
            );
            assert_eq!(
                std::convert::Into::<TotalIndex>::into(3),
                third_block.entry_index
            );
            assert_eq!(0., third_block.block_height_item.height().as_f64());
        });
    })
}

#[test]
fn test_viewport_iter_most_recent_at_top() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let terminal = add_window_with_terminal(&mut app, None);
        terminal.update(&mut app, |view, ctx: &mut ViewContext<'_, TerminalView>| {
            let mut model = view.model.lock();
            model.simulate_block("ls", "foo");
            model.simulate_block("echo multiline", "bar\nhey");
            let viewport = view.viewport_state(model.block_list(), InputMode::PinnedToTop, ctx);
            let mut iter = viewport.iter();
            let echo_block = iter.next().expect("item 2");
            assert_eq!(
                Some(std::convert::Into::<BlockIndex>::into(2)),
                echo_block.block_index
            );
            assert_eq!(
                std::convert::Into::<TotalIndex>::into(2),
                echo_block.entry_index
            );
            assert!(echo_block.block_height_item.height().into_lines() > Lines::zero());
            assert_eq!(Pixels::zero(), viewport.offset_to_top_of_first_block(ctx));
            assert_eq!(
                Some(std::convert::Into::<BlockIndex>::into(2)),
                viewport.topmost_visible_block()
            );
            assert!(viewport.is_block_in_view(
                std::convert::Into::<BlockIndex>::into(2),
                BlockVisibilityMode::TopOfBlockVisible
            ));

            let ls_block = iter.next().expect("item 1");
            assert_eq!(
                Some(std::convert::Into::<BlockIndex>::into(1)),
                ls_block.block_index
            );
            assert_eq!(
                std::convert::Into::<TotalIndex>::into(1),
                ls_block.entry_index
            );
            assert!(
                echo_block.block_height_item.height().as_f64()
                    > ls_block.block_height_item.height().as_f64()
            );
        });
    })
}

#[test]
fn test_viewport_most_recent_at_top() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let terminal = add_window_with_terminal(&mut app, None);
        terminal.update(&mut app, |view, ctx| {
            let mut model = view.model.lock();
            model.simulate_block("ls", "foo");
            model.simulate_block("echo multiline", "bar\nhey");
            let viewport = view.viewport_state(model.block_list(), InputMode::PinnedToTop, ctx);
            // Most recent block should be visible.
            let topmost_visible_block = viewport.topmost_visible_block().unwrap();
            assert!(viewport.is_block_in_view(
                topmost_visible_block,
                BlockVisibilityMode::TopOfBlockVisible
            ));
            assert_eq!(Pixels::zero(), viewport.offset_to_top_of_first_block(ctx));
            assert_eq!(0., viewport.scroll_top_in_lines().as_f64());
            assert!(matches!(
                viewport.next_scroll_position(
                    ScrollPositionUpdate::AfterScrollEvent {
                        scroll_delta: 1.0.into_lines()
                    },
                    ctx
                ),
                ScrollPosition::FixedAtPosition { .. }
            ));
            assert_eq!(
                Lines::zero(),
                viewport.top_of_block_in_lines(topmost_visible_block)
            );
            assert!(matches!(
                viewport.scroll_position_at_bottom_of_block(topmost_visible_block),
                ScrollPosition::FollowsBottomOfMostRecentBlock
            ));
            let block_list_point = viewport
                .screen_coord_to_blocklist_point(
                    vec2f(0., 0.),
                    SnackbarPoint {
                        coord: vec2f(0., 0.),
                        translation_mode: SnackbarTranslationMode::WithinSnackbar,
                    },
                    ClampingMode::ClampToGrid,
                )
                .unwrap();
            assert_eq!(
                Some(2.into()),
                viewport.block_index_from_point(block_list_point)
            );
        });
    })
}

#[test]
fn test_scroll_fixed_to_bottom() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let terminal = add_window_with_terminal(&mut app, None);
        terminal.read(&app, |view, _| {
            assert_eq!(
                view.scroll_position(),
                ScrollPosition::FollowsBottomOfMostRecentBlock
            );
        });
        terminal.update(&mut app, |view, ctx| {
            {
                let mut model = view.model.lock();
                // Put in enough blocks so that the view should be scrollable
                for _ in 0..100 {
                    model.simulate_block("ls", "foo");
                }
            }
            assert!(view.is_vertically_scrollable(ctx));
            assert_eq!(
                view.scroll_position(),
                ScrollPosition::FollowsBottomOfMostRecentBlock
            );
            view.scroll(1.0.into_lines(), ctx);

            let expected_scroll_top = {
                let model = view.model.lock();
                model.block_list().block_heights().summary().height
                    - view.content_element_height_lines(ctx)
                    - 1.0.into_lines()
            };
            assert_eq!(
                view.scroll_position(),
                ScrollPosition::FixedAtPosition {
                    scroll_lines: ScrollLines::ScrollTop(expected_scroll_top)
                },
            );
            // Now add to the active block and make sure we don't scroll
            {
                let mut model = view.model.lock();
                model.simulate_cmd("test");
            }
            {
                let mut model = view.model.lock();
                for _ in 0..100 {
                    model.linefeed();
                }
            }
            assert_eq!(
                view.scroll_position(),
                ScrollPosition::FixedAtPosition {
                    scroll_lines: ScrollLines::ScrollTop(expected_scroll_top)
                },
            );
        });
    })
}

#[test]
fn test_scroll_to_row() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let terminal = add_window_with_terminal(&mut app, None);
        terminal.update(&mut app, |view, ctx| {
            {
                let mut model = view.model.lock();
                // Put in enough blocks so that the view should be scrollable
                for _ in 0..50 {
                    model.simulate_block("ls", "foo\nfie\nfay\nfoe\nfum");
                }
            }

            assert!(view.is_vertically_scrollable(ctx));
            assert_eq!(
                view.scroll_position(),
                ScrollPosition::FollowsBottomOfMostRecentBlock
            );

            // Scroll upwards (no snackbar)
            let a = BlockListPoint::new(30.0, 0);
            view.scroll_to_row_if_not_visible(a.row.into_lines(), ctx);
            assert_eq!(
                view.scroll_position(),
                ScrollPosition::FixedAtPosition {
                    scroll_lines: ScrollLines::ScrollTop(30.0.into_lines())
                }
            );

            // Don't scroll at all
            let b = BlockListPoint::new(38.0, 0);
            view.scroll_to_row_if_not_visible(b.row.into_lines(), ctx);
            assert_eq!(
                view.scroll_position(),
                ScrollPosition::FixedAtPosition {
                    scroll_lines: ScrollLines::ScrollTop(30.0.into_lines())
                }
            );

            // Scroll downwards
            let c = BlockListPoint::new(100.0, 0);
            view.scroll_to_row_if_not_visible(c.row.into_lines(), ctx);
            assert_eq!(
                view.scroll_position(),
                ScrollPosition::FixedAtPosition {
                    scroll_lines: ScrollLines::ScrollTop(90.5.into_lines())
                }
            );
        });
    })
}

#[test]
fn test_stable_scrolling_during_grid_truncation() {
    App::test((), |mut app| async move {
        const MAX_GRID_SIZE: usize = 50;
        const INPUT_MODE: InputMode = InputMode::PinnedToBottom;

        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);

        // Note: this test is done in a single `update` to prevent
        // any changes in the presenter's position cache throughout.
        terminal.update(&mut app, |view, ctx| {
            // Set up the block list by creating a long-running
            // block that spans the entire viewport.
            {
                let mut model = view.model.lock();
                model.update_max_grid_size(MAX_GRID_SIZE);

                // Create a dummy, finished block and a long-running block.
                model.simulate_block("ls", "foo");
                model.simulate_long_running_block("cat", "");
                assert!(model
                    .block_list()
                    .active_block()
                    .is_active_and_long_running());

                // Add enough newlines so that the long-running block spans at
                // least the viewport and surely exceeds the grid size.
                let mut i = 0;
                while view.is_top_of_active_block_in_viewport(&model, INPUT_MODE, ctx)
                    || i < MAX_GRID_SIZE * 2
                {
                    model.process_bytes("\n");
                    i += 1;
                }
            }

            // Scroll up one line.
            assert_eq!(
                view.scroll_position(),
                ScrollPosition::FollowsBottomOfMostRecentBlock
            );
            view.scroll(1.into_lines(), ctx);
            assert!(matches!(
                view.scroll_position(),
                ScrollPosition::FixedWithinLongRunningBlock { .. }
            ));

            // Introduce new lines and make sure the scroll-top is adjusted as expected.
            {
                let mut model = view.model.lock();
                let active_block_index = model.block_list().active_block_index();
                let scroll_top_before_scrolling = view.scroll_top_in_lines(&model, INPUT_MODE, ctx);

                // To get to the top of the block, we need 50 lines for output grid and
                // then one line for command grid.
                for i in 1..=(MAX_GRID_SIZE + 1) {
                    model.process_bytes("\n");

                    let actual_scroll_top = view.scroll_top_in_lines(&model, INPUT_MODE, ctx);
                    let expected_scroll_top = scroll_top_before_scrolling - i.into_lines();
                    assert_eq!(actual_scroll_top, expected_scroll_top);
                }

                // Flush one full line in case the top of the block doesn't perfectly
                // line up with full lines (e.g. due to padding).
                model.process_bytes("\n");

                // Any remaining newlines should not move the scroll-top;
                // it should be "locked" at the top of the block.
                for _ in 0..MAX_GRID_SIZE {
                    model.process_bytes("\n");

                    let viewport = view.viewport_state(model.block_list(), INPUT_MODE, ctx);
                    let actual_scroll_top = viewport.scroll_top_in_lines();
                    let expected_scroll_top = viewport.top_of_block_in_lines(active_block_index);
                    assert_eq!(actual_scroll_top, expected_scroll_top);
                }
            }

            // Scroll up one line, bringing the previous block into the viewport.
            view.scroll(1.into_lines(), ctx);
            assert!(matches!(
                view.scroll_position(),
                ScrollPosition::FixedAtPosition { .. }
            ));

            // Introduce newlines and make sure the scroll-top does _not_ change anymore.
            {
                let mut model = view.model.lock();
                let scroll_top_before_newlines = view.scroll_top_in_lines(&model, INPUT_MODE, ctx);

                for _ in 0..MAX_GRID_SIZE {
                    model.process_bytes("\n");

                    let new_scroll_top = view.scroll_top_in_lines(&model, INPUT_MODE, ctx);
                    assert_eq!(scroll_top_before_newlines, new_scroll_top);
                }
            }
        });
    })
}

#[test]
fn test_clear_buffer() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let terminal = add_window_with_terminal(&mut app, None);
        terminal.update(&mut app, |view, ctx| {
            {
                let mut model = view.model.lock();
                for _ in 0..10 {
                    model.simulate_block("ls", "foo");
                }

                assert!(!model.block_list().blocks().is_empty());
            }

            view.bookmark_block(&BlockIndex::zero(), ctx);
            view.clear_buffer(ctx);

            {
                let model = view.model.lock();

                // There should be only one precmd block.
                assert_eq!(model.block_list().blocks().len(), 1);
                assert_eq!(view.bookmarked_blocks.len(), 0);
            }
        });
    })
}

#[test]
fn test_context_menu_includes_clear_when_block_list_non_empty() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let terminal = add_window_with_terminal(&mut app, None);
        terminal.update(&mut app, |view, ctx| {
            {
                let mut model = view.model.lock();
                model.simulate_block("ls", "foo");
                assert!(!model.is_block_list_empty());
            }

            let menu_source = BlockListMenuSource::OutsideBlockRightClick {
                position_in_terminal_view: Vector2F::zero(),
            };
            let items = view.context_menu_items(&menu_source, ctx);
            let labels: Vec<&str> = items
                .iter()
                .filter_map(|item| item.fields().map(|fields| fields.label()))
                .collect();
            assert!(
                labels.contains(&"Clear Blocks"),
                "Expected `Clear Blocks` menu item, got {labels:?}"
            );
        });
    })
}

#[test]
fn test_context_menu_omits_clear_when_block_list_empty() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let terminal = add_window_with_terminal(&mut app, None);
        terminal.update(&mut app, |view, ctx| {
            {
                let model = view.model.lock();
                assert!(model.is_block_list_empty());
            }

            let menu_source = BlockListMenuSource::OutsideBlockRightClick {
                position_in_terminal_view: Vector2F::zero(),
            };
            let items = view.context_menu_items(&menu_source, ctx);
            let labels: Vec<&str> = items
                .iter()
                .filter_map(|item| item.fields().map(|fields| fields.label()))
                .collect();
            assert!(
                !labels.contains(&"Clear Blocks"),
                "Did not expect `Clear Blocks` menu item when block list is empty, got {labels:?}"
            );
        });
    })
}

#[test]
fn test_context_menu_omits_clear_for_text_right_click() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let terminal = add_window_with_terminal(&mut app, None);
        terminal.update(&mut app, |view, ctx| {
            {
                let mut model = view.model.lock();
                model.simulate_block("ls", "foo");
                assert!(!model.is_block_list_empty());
            }

            let menu_source = BlockListMenuSource::RegularTextRightClick {
                position_in_terminal_view: Vector2F::zero(),
            };
            let items = view.context_menu_items(&menu_source, ctx);
            let labels: Vec<&str> = items
                .iter()
                .filter_map(|item| item.fields().map(|fields| fields.label()))
                .collect();
            assert!(
                !labels.contains(&"Clear Blocks"),
                "Did not expect `Clear Blocks` in text-selection right-click menu, got {labels:?}"
            );
        });
    })
}

#[test]
fn test_clear_buffer_clears_autosuggestion() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let terminal = add_window_with_terminal(&mut app, None);
        terminal.update(&mut app, |view, ctx| {
            // Set a next command suggestion (empty input)
            view.input.update(ctx, |input, ctx| {
                input.editor().update(ctx, |editor, ctx| {
                    editor.set_autosuggestion(
                        "git status",
                        AutosuggestionLocation::EndOfBuffer,
                        AutosuggestionType::Command {
                            was_intelligent_autosuggestion: true,
                        },
                        ctx,
                    );
                });
            });

            // Verify autosuggestion is present
            view.input.read(ctx, |input, ctx| {
                input.editor().read(ctx, |editor, _ctx| {
                    assert!(
                        editor.active_autosuggestion(),
                        "Autosuggestion should be active before clear_buffer"
                    );
                });
            });

            // Clear the buffer
            view.clear_buffer(ctx);

            // Verify autosuggestion is cleared
            view.input.read(ctx, |input, ctx| {
                input.editor().read(ctx, |editor, _ctx| {
                    assert!(
                        !editor.active_autosuggestion(),
                        "Autosuggestion should be cleared after clear_buffer"
                    );
                });
            });
        });
    })
}

#[test]
fn test_bookmark_blocks_navigation() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let terminal = add_window_with_terminal(&mut app, None);
        terminal.update(&mut app, |view, ctx| {
            {
                let mut model = view.model.lock();
                for _ in 0..10 {
                    model.simulate_block("ls", "foo");
                }

                assert!(!model.block_list().blocks().is_empty());
            }

            view.bookmark_block(&BlockIndex::zero(), ctx);
            view.bookmark_block(&BlockIndex::from(1), ctx);
            view.bookmark_block(&BlockIndex::from(4), ctx);

            view.bookmark_up(ctx);
            assert_eq!(view.selected_blocks.tail(), Some(4.into()));
            view.bookmark_down(ctx);
            assert_eq!(view.selected_blocks.tail(), Some(0.into()));
            view.bookmark_up(ctx);
            assert_eq!(view.selected_blocks.tail(), Some(4.into()));
            view.bookmark_up(ctx);
            assert_eq!(view.selected_blocks.tail(), Some(1.into()));
            view.bookmark_up(ctx);
            assert_eq!(view.selected_blocks.tail(), Some(0.into()));
            view.bookmark_down(ctx);
            assert_eq!(view.selected_blocks.tail(), Some(1.into()));
            view.bookmark_down(ctx);
            assert_eq!(view.selected_blocks.tail(), Some(4.into()));
        });
    })
}

fn run_navigation_test(input_mode: InputMode) {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let terminal = add_window_with_terminal(&mut app, None);
        terminal.read(&app, |view, _ctx| {
            assert_eq!(
                view.scroll_position(),
                ScrollPosition::FollowsBottomOfMostRecentBlock
            );
        });
        terminal.update(&mut app, |view, ctx| {
            InputModeSettings::handle(ctx).update(ctx, |input_mode_settings, ctx| {
                let _ = input_mode_settings.input_mode.set_value(input_mode, ctx);
            });

            {
                let mut model = view.model.lock();
                // Put in enough blocks so that the view should be scrollable
                for _ in 0..100 {
                    model.simulate_block("ls", "foo");
                }

                // Put in one block that is larger than the viewport height.
                model.simulate_block("ls", "foo\n".repeat(100).as_str())
            }

            assert!(view.is_vertically_scrollable(ctx));
            assert_eq!(
                view.scroll_position(),
                ScrollPosition::FollowsBottomOfMostRecentBlock
            );

            view.select_most_recent_blocks(1, ctx);
            assert_eq!(view.selected_blocks.tail(), Some(101.into()));

            view.select_less_recent_block(false /* is_shift_down */, ctx);
            assert_eq!(view.selected_blocks.tail(), Some(100.into()));

            view.select_more_recent_block(
                true,  /* is_cmd_down */
                false, /* is_shift_down */
                ctx,
            );
            assert_ne!(
                view.scroll_position(),
                ScrollPosition::FollowsBottomOfMostRecentBlock
            );
            assert_eq!(view.selected_blocks.tail(), Some(101.into()));

            view.select_more_recent_block(
                true,  /* is_cmd_down */
                false, /* is_shift_down */
                ctx,
            );
            if input_mode.is_inverted_blocklist() {
                // In the inverted case, we intentionally align to the
                // top of the most recent block here, not to its bottom
                assert!(matches!(
                    view.scroll_position(),
                    ScrollPosition::FixedAtPosition { .. }
                ));
            } else {
                assert_eq!(
                    view.scroll_position(),
                    ScrollPosition::FollowsBottomOfMostRecentBlock
                );
            }
            assert_eq!(view.selected_blocks.tail(), Some(101.into()));

            view.select_more_recent_block(
                true,  /* is_cmd_down */
                false, /* is_shift_down */
                ctx,
            );
            assert_eq!(view.selected_blocks.tail(), None);
        });
    });
}

#[test]
fn test_navigate_blocks() {
    run_navigation_test(InputMode::PinnedToBottom);
}

// #[test]
// fn test_navigate_blocks_inverted_blocklist() {
//     run_navigation_test(InputMode::PinnedToTop);
// }

#[test]
fn test_alt_scroll_sequences() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let terminal = add_window_with_terminal(&mut app, None);
        // Test scrolling a distance of zero lines.
        terminal.update(&mut app, |view, _| {
            let content = view.alt_scroll_sequences(0);
            assert!(content.is_empty());
        });
        // Scroll down 3 lines
        terminal.update(&mut app, |view, _| {
            let content = view.alt_scroll_sequences(-3);
            assert_eq!(content.len(), 3 * 3);
            assert_eq!(
                content
                    .into_iter()
                    .filter(|b| *b == escape_sequences::EscCodes::ARROW_DOWN)
                    .count(),
                3
            );
        });
        // Scroll up 5 lines
        terminal.update(&mut app, |view, _| {
            let content = view.alt_scroll_sequences(5);
            assert_eq!(content.len(), 5 * 3);
            assert_eq!(
                content
                    .into_iter()
                    .filter(|b| *b == escape_sequences::EscCodes::ARROW_UP)
                    .count(),
                5
            );
        });
    })
}

#[test]
fn test_not_bootstrapped() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let terminal = add_window_with_terminal(&mut app, None);
        terminal.update(&mut app, |view, ctx| {
            let model = view.model.lock();
            assert!(view.is_input_box_visible(&model, ctx));
            drop(model);

            assert_eq!(view.active_session_path_if_local(ctx), None);
        });
    })
}

#[test]
fn test_block_select() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |view, ctx| {
            view.selected_blocks
                .toggle(10.into(), Some(11.into()), Some(9.into()));

            let single_mouse_down = BlockSelectAction::MouseDown(Some(1.into()));
            // On Mac, we use cmd-click to toggle block selections, but
            // we use ctrl-click on non-Mac platforms.
            let single_mouse_up = if cfg!(target_os = "macos") {
                BlockSelectAction::MouseUp {
                    block_index: 1.into(),
                    is_ctrl_down: false,
                    is_cmd_down: true,
                    is_shift_down: false,
                }
            } else {
                BlockSelectAction::MouseUp {
                    block_index: 1.into(),
                    is_ctrl_down: true,
                    is_cmd_down: false,
                    is_shift_down: false,
                }
            };
            view.block_select(&single_mouse_down, true, ctx);
            view.block_select(&single_mouse_up, true, ctx);
            assert!(view.selected_blocks.is_selected(1.into()));
            assert!(view.selected_blocks.is_selected(10.into()));

            let range_mouse_down = BlockSelectAction::MouseDown(Some(5.into()));
            let range_mouse_up = BlockSelectAction::MouseUp {
                block_index: 5.into(),
                is_ctrl_down: false,
                is_cmd_down: false,
                is_shift_down: true,
            };
            view.block_select(&range_mouse_down, true, ctx);
            view.block_select(&range_mouse_up, true, ctx);
            assert!(!view.selected_blocks.is_selected(10.into()));
            assert_eq!(view.selected_blocks_pivot_index(), Some(1.into()));
            assert_eq!(view.selected_blocks_tail_index(), Some(5.into()));
        });
    })
}

#[test]
fn test_select_all_blocks() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let terminal = add_window_with_terminal(&mut app, None);
        terminal.update(&mut app, |view, ctx| {
            {
                let mut model = view.model.lock();
                // Put in enough blocks so that the view should be scrollable
                for _ in 0..100 {
                    model.simulate_block("ls", "foo");
                }
            }
            assert!(view.is_vertically_scrollable(ctx));

            view.select_all_blocks(ctx);
            assert_eq!(view.selected_blocks_pivot_index().unwrap(), 1.into());
            assert_eq!(view.selected_blocks_tail_index().unwrap(), 100.into());
            for i in 1..100 {
                assert!(view.selected_blocks.is_selected(i.into()));
            }
        });
    })
}

#[test]
fn test_expand_selection_above_and_below() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let terminal = add_window_with_terminal(&mut app, None);
        terminal.update(&mut app, |view, ctx| {
            {
                let mut model = view.model.lock();
                // Put in enough blocks so that the view should be scrollable
                for _ in 0..100 {
                    model.simulate_block("ls", "foo");
                }
            }
            assert!(view.is_vertically_scrollable(ctx));

            // helper to ensure indices are all selected
            fn assert_all_selected(selected_blocks: &SelectedBlocks, indices: Vec<BlockIndex>) {
                for &idx in indices.iter() {
                    assert!(selected_blocks.is_selected(idx));
                }
            }

            view.selected_blocks
                .toggle(5.into(), Some(6.into()), Some(4.into()));
            assert_all_selected(&view.selected_blocks, vec![5.into()]);

            view.select_more_recent_block(
                false, /* is_cmd_down */
                true,  /* is_shift_down */
                ctx,
            );
            assert_all_selected(&view.selected_blocks, vec![5.into(), 6.into()]);

            view.select_more_recent_block(
                false, /* is_cmd_down */
                true,  /* is_shift_down */
                ctx,
            );
            assert_all_selected(&view.selected_blocks, vec![5.into(), 6.into(), 7.into()]);

            view.select_less_recent_block(true /* is_shift_down */, ctx);
            assert_all_selected(&view.selected_blocks, vec![5.into(), 6.into()]);

            view.select_less_recent_block(true /* is_shift_down */, ctx);
            assert_all_selected(&view.selected_blocks, vec![5.into()]);

            view.select_less_recent_block(true /* is_shift_down */, ctx);
            assert_all_selected(&view.selected_blocks, vec![5.into(), 4.into()]);

            view.select_more_recent_block(
                false, /* is_cmd_down */
                true,  /* is_shift_down */
                ctx,
            );
            assert_all_selected(&view.selected_blocks, vec![5.into()]);
        });
    })
}

#[test]
fn test_copy_blocks() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let terminal = add_window_with_terminal(&mut app, None);
        terminal.update(&mut app, |view, ctx| {
            let (first_command, first_output) = ("ls", "foo");
            let (second_command, second_output) = ("pwd", "bar");

            {
                let mut model = view.model.lock();
                model.simulate_block(first_command, first_output);
                model.simulate_block(second_command, second_output);
            }

            // select a single block
            view.selected_blocks.toggle(2.into(), None, Some(1.into()));

            // test copy for a single block
            view.copy_blocks(BlockEntity::Command, ctx);
            assert_eq!(read_from_clipboard(ctx), second_command.to_string());

            view.copy_blocks(BlockEntity::Output, ctx);
            assert_eq!(read_from_clipboard(ctx), second_output.to_string());

            view.copy_blocks(BlockEntity::CommandAndOutput, ctx);
            assert_eq!(
                read_from_clipboard(ctx),
                format!("{second_command}\n{second_output}")
            );

            // select another block (in reverse)
            view.selected_blocks.toggle(1.into(), Some(2.into()), None);

            // test copy semantics for multiple blocks
            view.copy_blocks(BlockEntity::Command, ctx);
            let expected_commands_str = format!("{first_command}\n{second_command}");
            assert_eq!(read_from_clipboard(ctx), expected_commands_str);

            view.copy_blocks(BlockEntity::Output, ctx);
            let expected_outputs_str = format!("{first_output}\n{second_output}");
            assert_eq!(read_from_clipboard(ctx), expected_outputs_str);

            view.copy_blocks(BlockEntity::CommandAndOutput, ctx);
            let expected_both_str =
                format!("{first_command}\n{first_output}\n{second_command}\n{second_output}");
            assert_eq!(read_from_clipboard(ctx), expected_both_str);
        });
    })
}

#[test]
fn test_reinput_blocks() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let terminal = add_window_with_terminal(&mut app, None);
        terminal.update(&mut app, |view, ctx| {
            let (first_command, first_output) = ("ls", "foo");
            let (second_command, second_output) = ("pwd", "bar");

            {
                let mut model = view.model.lock();
                model.simulate_block(first_command, first_output);
                model.simulate_block(second_command, second_output);
            }

            // test reinput command for single block
            view.selected_blocks.toggle(2.into(), None, Some(1.into()));
            view.reinput_commands(false /* as_root */, ctx);
            assert_eq!(view.input().as_ref(ctx).buffer_text(ctx), second_command);

            view.selected_blocks.toggle(2.into(), None, Some(1.into()));
            view.reinput_commands(true /* as_root */, ctx);
            assert_eq!(
                view.input().as_ref(ctx).buffer_text(ctx),
                format!("sudo {second_command}")
            );

            // test reinput commands for multiple blocks (selected in reverse)
            view.selected_blocks.toggle(2.into(), None, Some(1.into()));
            view.selected_blocks.toggle(1.into(), Some(2.into()), None);
            view.reinput_commands(false /* as_root */, ctx);
            assert_eq!(
                view.input().as_ref(ctx).buffer_text(ctx),
                format!("{first_command}\n{second_command}")
            );

            view.selected_blocks.toggle(2.into(), None, Some(1.into()));
            view.selected_blocks.toggle(1.into(), Some(2.into()), None);
            view.reinput_commands(true /* as_root */, ctx);
            assert_eq!(
                view.input().as_ref(ctx).buffer_text(ctx),
                format!("sudo {first_command}\nsudo {second_command}")
            );
        });
    })
}

fn run_find_test(input_mode: InputMode) {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let terminal = add_window_with_terminal(&mut app, None);
        terminal.update(&mut app, |view, ctx| {
            InputModeSettings::handle(ctx).update(ctx, |input_mode_settings, ctx| {
                let _ = input_mode_settings.input_mode.set_value(input_mode, ctx);
            });

            let (first_command, first_output) = ("ls", "foo");
            let (second_command, second_output) = ("pwd", "foobar foo beans");
            let (third_command, third_output) = ("fools", "baz");

            {
                let mut model = view.model.lock();
                model.simulate_block(first_command, first_output);
                model.simulate_block(second_command, second_output);
                model.simulate_block(third_command, third_output);
            }

            view.show_find_bar(ctx);

            // Test without find_in_block enabled (results should be selection-agnostic)
            view.find_bar.update(ctx, |view, _ctx| {
                view.display_find_within_block = FindWithinBlockState::Disabled;
            });

            // find when no block is selected
            view.handle_find_event(
                &FindEvent::Update {
                    query: Some("foo".to_string()),
                },
                ctx,
            );
            assert_eq!(
                view.find_model.as_ref(ctx).visible_block_list_match_count(),
                4
            );
            assert_eq!(
                view.find_model
                    .as_ref(ctx)
                    .block_list_find_run()
                    .expect("BlockListFindRun exists.")
                    .focused_match_block_index()
                    .expect("Focused match exists."),
                3.into()
            );
            view.handle_find_event(
                &FindEvent::NextMatch {
                    direction: FindDirection::Down,
                },
                ctx,
            );
            if input_mode.is_inverted_blocklist() {
                // should go "down" to middle block
                assert_eq!(
                    view.find_model
                        .as_ref(ctx)
                        .block_list_find_run()
                        .expect("BlockListFindRun exists.")
                        .focused_match_block_index()
                        .expect("Focused match exists."),
                    2.into()
                );
            } else {
                // should loop to earliest block
                assert_eq!(
                    view.find_model
                        .as_ref(ctx)
                        .block_list_find_run()
                        .expect("BlockListFindRun exists.")
                        .focused_match_block_index()
                        .expect("Focused match exists."),
                    1.into()
                );
            }

            // find when a single block is selected
            view.selected_blocks.reset_to_single(2.into());
            view.handle_find_event(
                &FindEvent::Update {
                    query: Some("ls".to_string()),
                },
                ctx,
            );
            assert_eq!(
                view.find_model.as_ref(ctx).visible_block_list_match_count(),
                2
            );
            assert_block_has_find_match(view.find_model.as_ref(ctx), 1.into());
            assert_block_has_find_match(view.find_model.as_ref(ctx), 3.into());

            // Test with find_in_block enabled
            view.find_bar.update(ctx, |view, _ctx| {
                view.display_find_within_block = FindWithinBlockState::Enabled;
            });

            // find when no block is selected
            view.selected_blocks.reset();
            view.handle_find_event(
                &FindEvent::Update {
                    query: Some("foo".to_string()),
                },
                ctx,
            );
            assert_eq!(
                view.find_model.as_ref(ctx).visible_block_list_match_count(),
                0
            );

            // find when a single block is selected
            view.selected_blocks.reset_to_single(2.into());
            view.handle_find_event(
                &FindEvent::Update {
                    query: Some("pwd".to_string()),
                },
                ctx,
            );
            assert_eq!(
                view.find_model.as_ref(ctx).visible_block_list_match_count(),
                1
            );
            assert_block_has_find_match(view.find_model.as_ref(ctx), 2.into());

            // find when multiple blocks are selected, and find in block is enabled
            view.selected_blocks.toggle(3.into(), Some(2.into()), None);
            view.handle_find_event(
                &FindEvent::Update {
                    query: Some("foo".to_string()),
                },
                ctx,
            );
            assert_eq!(
                view.find_model.as_ref(ctx).visible_block_list_match_count(),
                3
            );
            assert_block_has_find_match(view.find_model.as_ref(ctx), 2.into());
            assert_block_has_find_match(view.find_model.as_ref(ctx), 3.into());
        });
    })
}

#[test]
fn test_find_in_blocks() {
    run_find_test(InputMode::PinnedToBottom);
}

#[test]
fn test_find_in_blocks_inverted_blocklist() {
    run_find_test(InputMode::PinnedToTop);
}

#[test]
fn test_case_sensitive_find() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let terminal = add_window_with_terminal(&mut app, None);
        terminal.update(&mut app, |view, ctx| {
            let (first_command, first_output) = ("ls", "foo");
            let (second_command, second_output) = ("pwd", "fOObar");
            let (third_command, third_output) = ("FoOls", "baz");

            {
                let mut model = view.model.lock();
                model.simulate_block(first_command, first_output);
                model.simulate_block(second_command, second_output);
                model.simulate_block(third_command, third_output);
            }

            view.show_find_bar(ctx);

            // Test without case sensitivity enabled (no blocks enabled)
            view.handle_find_event(
                &FindEvent::Update {
                    query: Some("fOO".to_string()),
                },
                ctx,
            );
            assert_eq!(
                view.find_model.as_ref(ctx).visible_block_list_match_count(),
                3
            );

            // Test without case sensitivity enabled, but with find in block
            view.find_bar.update(ctx, |view, _ctx| {
                view.display_find_within_block = FindWithinBlockState::Enabled;
            });
            view.selected_blocks.reset_to_single(1.into());
            view.update_find_selection(ctx);
            assert_eq!(
                view.find_model.as_ref(ctx).visible_block_list_match_count(),
                1
            );
            assert_block_has_find_match(view.find_model.as_ref(ctx), 1.into());

            // Test with case sensitivity enabled (one block enabled)
            view.handle_find_event(
                &FindEvent::ToggleCaseSensitivity {
                    is_case_sensitive: true,
                },
                ctx,
            );
            view.selected_blocks.reset_to_single(1.into());
            view.update_find_selection(ctx);
            assert_eq!(
                view.find_model.as_ref(ctx).visible_block_list_match_count(),
                0
            );

            view.selected_blocks.reset_to_single(2.into());
            view.update_find_selection(ctx);
            assert_eq!(
                view.find_model.as_ref(ctx).visible_block_list_match_count(),
                1
            );
            assert_block_has_find_match(view.find_model.as_ref(ctx), 2.into());

            // Test with case sensitivity enabled (no blocks enabled)
            view.selected_blocks.reset();
            view.find_bar.update(ctx, |view, _ctx| {
                view.display_find_within_block = FindWithinBlockState::Disabled;
            });
            assert_eq!(
                view.find_model.as_ref(ctx).visible_block_list_match_count(),
                1
            );
            assert_block_has_find_match(view.find_model.as_ref(ctx), 2.into());

            // Change regex to mismatch case sensitivity across all blocks
            view.handle_find_event(
                &FindEvent::Update {
                    query: Some("FOO".to_string()),
                },
                ctx,
            );
            assert_eq!(
                view.find_model.as_ref(ctx).visible_block_list_match_count(),
                0
            );
        });
    })
}

#[test]
fn test_find_bar_prefix_search() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let terminal = add_window_with_terminal(&mut app, None);
        terminal.update(&mut app, |view, ctx| {
            let (command_1, output_1) = ("echo foo", "foo");
            let (command_2, output_2) = ("echo bar foo", "bar foo");

            {
                let mut model = view.model.lock();
                model.simulate_block(command_1, output_1);
                model.simulate_block(command_2, output_2);
            }

            view.show_find_bar(ctx);

            // Test without regex enabled
            view.handle_find_event(
                &FindEvent::Update {
                    query: Some("^foo".to_string()),
                },
                ctx,
            );

            assert_eq!(
                view.find_model.as_ref(ctx).visible_block_list_match_count(),
                0
            );

            view.handle_find_event(
                &FindEvent::ToggleRegexSearch {
                    is_regex_enabled: true,
                },
                ctx,
            );

            assert_eq!(
                view.find_model.as_ref(ctx).visible_block_list_match_count(),
                1
            );
        });
    });
}

#[test]
fn test_create_notification_shorter_than_max() {
    let command = "cargo run";
    let output = "error: failed to find directory";
    let command_succeeded = false;
    let block_duration = Duration::new(4, 2);

    let trigger = NotificationsTrigger::LongRunningCommand(command_succeeded, block_duration);

    let actual_content =
        trigger.create_notification_content(command.to_string(), output.to_string());

    let expected_title = format!("'{command}' failed after 4s");
    let expected_body = format!("{BODY_PREFIX}{output}");

    assert_eq!(actual_content.title, expected_title);
    assert_eq!(actual_content.body, expected_body);
}

#[test]
fn test_create_notification_as_long_as_max() {
    let expected_title_suffix = " finished after 4s";
    let max_command_len = UserNotification::MAX_TITLE_LENGTH - expected_title_suffix.len() - 2;
    let command = "a".repeat(max_command_len);

    let max_output_len = UserNotification::MAX_BODY_LENGTH - BODY_PREFIX.len();
    let output = "a".repeat(max_output_len);

    let command_succeeded = true;
    let block_duration = Duration::new(4, 2);

    let trigger = NotificationsTrigger::LongRunningCommand(command_succeeded, block_duration);

    let actual_content =
        trigger.create_notification_content(command.to_string(), output.to_string());

    let expected_title = format!("'{command}'{expected_title_suffix}");
    let expected_body = format!("{BODY_PREFIX}{output}");

    assert_eq!(actual_content.title, expected_title);
    assert_eq!(actual_content.body, expected_body);
}

#[test]
fn test_create_notification_longer_than_max() {
    let expected_title_suffix = " finished after 4s";
    let max_command_len = UserNotification::MAX_TITLE_LENGTH - expected_title_suffix.len() - 2;
    let command = "a".repeat(max_command_len + 1);

    let max_output_len = UserNotification::MAX_BODY_LENGTH - BODY_PREFIX.len();
    let output = "a".repeat(max_output_len + 1);

    let command_succeeded = true;
    let block_duration = Duration::new(4, 2);

    let trigger = NotificationsTrigger::LongRunningCommand(command_succeeded, block_duration);

    let actual_content =
        trigger.create_notification_content(command.to_string(), output.to_string());

    let expected_title = format!(
        "'{}...'{expected_title_suffix}",
        &command[..max_command_len - 3]
    );
    let expected_body = format!("{BODY_PREFIX}...{}", &output[..max_output_len - 3]);

    assert_eq!(actual_content.title, expected_title);
    assert_eq!(actual_content.body, expected_body);
}

#[test]
fn test_create_notification_char_boundaries_respected() {
    let expected_title_suffix = " finished after 4s";
    let max_command_len = UserNotification::MAX_TITLE_LENGTH - expected_title_suffix.len() - 2;
    let command = "😊".repeat(max_command_len + 1);

    let output = "error: failed to find directory";
    let command_succeeded = true;
    let block_duration = Duration::new(4, 2);

    let trigger = NotificationsTrigger::LongRunningCommand(command_succeeded, block_duration);

    let actual_content = trigger.create_notification_content(command, output.to_string());

    let expected_command_prefix = "😊".repeat(max_command_len - 3);
    let expected_title = format!("'{expected_command_prefix}...'{expected_title_suffix}",);
    assert_eq!(actual_content.title, expected_title);
}

#[test]
fn test_banner_for_incompatible_plugins() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal =
            MockTerminalManager::create_new_terminal_view_window_for_test(&mut app, None);

        SessionSettings::handle(&app).update(&mut app, |session_settings, ctx| {
            let _ = session_settings.honor_ps1.set_value(true, ctx);
        });

        terminal.update(&mut app, |view, _ctx| {
            let mut model = view.model.lock();
            model.init_shell(InitShellValue {
                session_id: 0.into(),
                shell: "zsh".to_owned(),
                ..Default::default()
            });
            model.bootstrapped(BootstrappedValue {
                shell: "zsh".to_owned(),
                shell_plugins: Some(HashSet::from(["p10k_unsupported".to_string()])),
                ..Default::default()
            });
        });

        // This is asynchronous because we're waiting for the bootstrap event
        // to be sent from the terminal model to the terminal view.
        assert_eventually!(
            200 => terminal.read(&app, |view, _ctx| view
                .is_incompatible_configuration_banner_open),
            "Banner did not open in time"
        );
    })
}

#[test]
fn test_bash_vim_banner_already_shown() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal =
            MockTerminalManager::create_new_terminal_view_window_for_test(&mut app, None);

        // Ensure the terminal is the active session.
        terminal.update(&mut app, |view, ctx| {
            let terminal_pane_id = TerminalPaneId::dummy_terminal_pane_id();
            let focus_state = ctx.add_model(|_| {
                PaneGroupFocusState::new(terminal_pane_id.into(), Some(terminal_pane_id), false)
            });
            let focus_handle = PaneFocusHandle::new(terminal_pane_id.into(), focus_state);
            view.set_focus_handle(focus_handle, ctx);
        });

        // The banner has already been shown and dismissed.
        VimBannerSettings::handle(&app).update(&mut app, |banner_settings, ctx| {
            let _ = banner_settings
                .vim_keybindings_banner_state
                .set_value(BannerState::Dismissed, ctx);
        });

        // Ensure Warp's vim keybindings are off.
        AppEditorSettings::handle(&app).update(&mut app, |editor_settings, ctx| {
            let _ = editor_settings.vim_mode.set_value(false, ctx);
        });

        // Bootstrap a bash session with vi mode enabled.
        terminal.update(&mut app, |view, _ctx| {
            let mut model = view.model.lock();
            model.init_shell(InitShellValue {
                session_id: 0.into(),
                shell: "bash".to_owned(),
                ..Default::default()
            });
            model.bootstrapped(BootstrappedValue {
                shell: "bash".to_owned(),
                shell_options: Some(HashSet::from(["vi_mode".to_string()])),
                ..Default::default()
            });
        });

        // This is asynchronous because we're waiting for the bootstrap event
        // to be sent from the terminal model to the terminal view.
        assert_eventually!(
            // Since the user already dismissed the banner, it should not
            // be shown again.
            terminal.read(&app, |terminal, _terminal_ctx| {
                terminal.inline_banners_state.vim_banner_state.is_none()
            }),
            "Banner should not have opened"
        );
    })
}

#[test]
fn test_bash_vim_banner_on() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal =
            MockTerminalManager::create_new_terminal_view_window_for_test(&mut app, None);

        // Ensure the terminal is the active session.
        terminal.update(&mut app, |view, ctx| {
            let terminal_pane_id = TerminalPaneId::dummy_terminal_pane_id();
            let focus_state = ctx.add_model(|_| {
                PaneGroupFocusState::new(terminal_pane_id.into(), Some(terminal_pane_id), false)
            });
            let focus_handle = PaneFocusHandle::new(terminal_pane_id.into(), focus_state);
            view.set_focus_handle(focus_handle, ctx);
        });

        // Ensure the banner has never been shown.
        VimBannerSettings::handle(&app).update(&mut app, |banner_settings, ctx| {
            let _ = banner_settings
                .vim_keybindings_banner_state
                .set_value(BannerState::NotDismissed, ctx);
        });

        // Ensure Warp's vim keybindings are off.
        AppEditorSettings::handle(&app).update(&mut app, |editor_settings, ctx| {
            let _ = editor_settings.vim_mode.set_value(false, ctx);
        });

        // Bootstrap a bash session with vi mode enabled.
        terminal.update(&mut app, |view, _ctx| {
            let mut model = view.model.lock();
            model.init_shell(InitShellValue {
                session_id: 0.into(),
                shell: "bash".to_owned(),
                ..Default::default()
            });
            model.bootstrapped(BootstrappedValue {
                shell: "bash".to_owned(),
                shell_options: Some(HashSet::from(["vi_mode".to_string()])),
                ..Default::default()
            });
        });

        // This is asynchronous because we're waiting for the bootstrap event
        // to be sent from the terminal model to the terminal view.
        assert_eventually!(
            // The vim keybinding banner should display.
            200 => terminal.read(&app, |terminal, _terminal_ctx| {
                terminal.inline_banners_state.vim_banner_state.is_some()
            }),
            "Banner did not open in time"
        );
    })
}

#[test]
fn test_bash_vim_banner_off() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal =
            MockTerminalManager::create_new_terminal_view_window_for_test(&mut app, None);

        // Ensure the terminal is the active session.
        terminal.update(&mut app, |view, ctx| {
            let terminal_pane_id = TerminalPaneId::dummy_terminal_pane_id();
            let focus_state = ctx.add_model(|_| {
                PaneGroupFocusState::new(terminal_pane_id.into(), Some(terminal_pane_id), false)
            });
            let focus_handle = PaneFocusHandle::new(terminal_pane_id.into(), focus_state);
            view.set_focus_handle(focus_handle, ctx);
        });

        // Ensure the banner has never been shown.
        VimBannerSettings::handle(&app).update(&mut app, |banner_settings, ctx| {
            let _ = banner_settings
                .vim_keybindings_banner_state
                .set_value(BannerState::NotDismissed, ctx);
        });

        // Ensure Warp's vim keybindings are on.
        AppEditorSettings::handle(&app).update(&mut app, |editor_settings, ctx| {
            let _ = editor_settings.vim_mode.set_value(true, ctx);
        });

        // Bootstrap a bash session with vi mode enabled.
        terminal.update(&mut app, |view, _ctx| {
            let mut model = view.model.lock();
            model.init_shell(InitShellValue {
                session_id: 0.into(),
                shell: "bash".to_owned(),
                ..Default::default()
            });
            model.bootstrapped(BootstrappedValue {
                shell: "bash".to_owned(),
                shell_options: Some(HashSet::from(["vi_mode".to_string()])),
                ..Default::default()
            });
        });

        // This is asynchronous because we're waiting for the bootstrap event
        // to be sent from the terminal model to the terminal view.
        assert_eventually!(
            // The vim keybinding banner should NOT display
            // because the user already has vim keybindings turned on.
            terminal.read(&app, |terminal, _terminal_ctx| {
                terminal.inline_banners_state.vim_banner_state.is_none()
            }),
            "Banner should not have opened"
        );
    })
}

#[test]
fn test_zsh_vim_banner_on() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal =
            MockTerminalManager::create_new_terminal_view_window_for_test(&mut app, None);

        // Ensure the terminal is the active session.
        terminal.update(&mut app, |view, ctx| {
            let terminal_pane_id = TerminalPaneId::dummy_terminal_pane_id();
            let focus_state = ctx.add_model(|_| {
                PaneGroupFocusState::new(terminal_pane_id.into(), Some(terminal_pane_id), false)
            });
            let focus_handle = PaneFocusHandle::new(terminal_pane_id.into(), focus_state);
            view.set_focus_handle(focus_handle, ctx);
        });

        // Ensure the banner has never been shown.
        VimBannerSettings::handle(&app).update(&mut app, |banner_settings, ctx| {
            let _ = banner_settings
                .vim_keybindings_banner_state
                .set_value(BannerState::NotDismissed, ctx);
        });

        // Ensure Warp's vim keybindings are off.
        AppEditorSettings::handle(&app).update(&mut app, |editor_settings, ctx| {
            let _ = editor_settings.vim_mode.set_value(false, ctx);
        });

        // Bootstrap a zsh session with vi mode enabled.
        terminal.update(&mut app, |view, _ctx| {
            let mut model = view.model.lock();
            model.init_shell(InitShellValue {
                session_id: 0.into(),
                shell: "zsh".to_owned(),
                ..Default::default()
            });
            model.bootstrapped(BootstrappedValue {
                shell: "zsh".to_owned(),
                shell_plugins: Some(HashSet::from(["vi".to_string()])),
                ..Default::default()
            });
        });

        // This is asynchronous because we're waiting for the bootstrap event
        // to be sent from the terminal model to the terminal view.
        assert_eventually!(
            // The vim keybinding banner should display.
            200 => terminal.read(&app, |terminal, _terminal_ctx| {
                terminal.inline_banners_state.vim_banner_state.is_some()
            }),
            "Banner did not open in time"
        );
    })
}

#[test]
fn test_zsh_vim_banner_off() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal =
            MockTerminalManager::create_new_terminal_view_window_for_test(&mut app, None);

        // Ensure the terminal is the active session.
        terminal.update(&mut app, |view, ctx| {
            let terminal_pane_id = TerminalPaneId::dummy_terminal_pane_id();
            let focus_state = ctx.add_model(|_| {
                PaneGroupFocusState::new(terminal_pane_id.into(), Some(terminal_pane_id), false)
            });
            let focus_handle = PaneFocusHandle::new(terminal_pane_id.into(), focus_state);
            view.set_focus_handle(focus_handle, ctx);
        });

        // Ensure the banner has never been shown.
        VimBannerSettings::handle(&app).update(&mut app, |banner_settings, ctx| {
            let _ = banner_settings
                .vim_keybindings_banner_state
                .set_value(BannerState::NotDismissed, ctx);
        });

        // Ensure Warp's vim keybindings are on.
        AppEditorSettings::handle(&app).update(&mut app, |editor_settings, ctx| {
            let _ = editor_settings.vim_mode.set_value(true, ctx);
        });

        // Bootstrap a zsh session with vi mode enabled.
        terminal.update(&mut app, |view, _ctx| {
            let mut model = view.model.lock();
            model.init_shell(InitShellValue {
                session_id: 0.into(),
                shell: "zsh".to_owned(),
                ..Default::default()
            });
            model.bootstrapped(BootstrappedValue {
                shell: "zsh".to_owned(),
                shell_plugins: Some(HashSet::from(["vi".to_string()])),
                ..Default::default()
            });
        });

        // This is asynchronous because we're waiting for the bootstrap event
        // to be sent from the terminal model to the terminal view.
        assert_eventually!(
            // The vim keybinding banner should NOT display
            // because the user already has vim keybindings turned on.
            terminal.read(&app, |terminal, _terminal_ctx| {
                terminal.inline_banners_state.vim_banner_state.is_none()
            }),
            "Banner should not have opened"
        );
    })
}

#[test]
fn test_fish_vim_banner_on() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal =
            MockTerminalManager::create_new_terminal_view_window_for_test(&mut app, None);

        // Ensure the terminal is the active session.
        terminal.update(&mut app, |view, ctx| {
            let terminal_pane_id = TerminalPaneId::dummy_terminal_pane_id();
            let focus_state = ctx.add_model(|_| {
                PaneGroupFocusState::new(terminal_pane_id.into(), Some(terminal_pane_id), false)
            });
            let focus_handle = PaneFocusHandle::new(terminal_pane_id.into(), focus_state);
            view.set_focus_handle(focus_handle, ctx);
        });

        // Ensure Warp's vim keybindings are off.
        AppEditorSettings::handle(&app).update(&mut app, |editor_settings, ctx| {
            let _ = editor_settings.vim_mode.set_value(false, ctx);
        });

        // Bootstrap a fish session with vi mode enabled.
        terminal.update(&mut app, |view, _ctx| {
            let mut model = view.model.lock();
            model.init_shell(InitShellValue {
                session_id: 0.into(),
                shell: "fish".to_owned(),
                ..Default::default()
            });
            model.bootstrapped(BootstrappedValue {
                shell: "fish".to_owned(),
                shell_options: Some(HashSet::from(["vi_mode".to_string()])),
                ..Default::default()
            });
        });

        // This is asynchronous because we're waiting for the bootstrap event
        // to be sent from the terminal model to the terminal view.
        assert_eventually!(
            // The vim keybinding banner should display.
            200 => terminal.read(&app, |terminal, _terminal_ctx| {
                terminal.inline_banners_state.vim_banner_state.is_some()
            }),
            "Banner did not open in time"
        );
    })
}

#[test]
fn test_fish_vim_banner_off() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal =
            MockTerminalManager::create_new_terminal_view_window_for_test(&mut app, None);

        // Ensure the terminal is the active session.
        terminal.update(&mut app, |view, ctx| {
            let terminal_pane_id = TerminalPaneId::dummy_terminal_pane_id();
            let focus_state = ctx.add_model(|_| {
                PaneGroupFocusState::new(terminal_pane_id.into(), Some(terminal_pane_id), false)
            });
            let focus_handle = PaneFocusHandle::new(terminal_pane_id.into(), focus_state);
            view.set_focus_handle(focus_handle, ctx);
        });

        // Ensure Warp's vim keybindings are on.
        AppEditorSettings::handle(&app).update(&mut app, |editor_settings, ctx| {
            let _ = editor_settings.vim_mode.set_value(true, ctx);
        });

        // Bootstrap a fish session with vi mode enabled.
        terminal.update(&mut app, |view, _ctx| {
            let mut model = view.model.lock();
            model.init_shell(InitShellValue {
                session_id: 0.into(),
                shell: "fish".to_owned(),
                ..Default::default()
            });
            model.bootstrapped(BootstrappedValue {
                shell: "fish".to_owned(),
                shell_options: Some(HashSet::from(["vi_mode".to_string()])),
                ..Default::default()
            });
        });

        // This is asynchronous because we're waiting for the bootstrap event
        // to be sent from the terminal model to the terminal view.
        assert_eventually!(
            // The vim keybinding banner should NOT display
            // because the user already has vim keybindings turned on.
            terminal.read(&app, |terminal, _terminal_ctx| {
                terminal.inline_banners_state.vim_banner_state.is_none()
            }),
            "Banner should not have opened"
        );
    })
}

#[test]
fn test_prompt_context_menu_items_for_ps1() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);

        SessionSettings::handle(&app).update(&mut app, |session_settings, ctx| {
            let _ = session_settings.honor_ps1.set_value(true, ctx);
        });

        terminal.read(&app, |view, ctx| {
            let items = view.prompt_context_menu_items(ctx);
            let len = items.len();
            assert_eq!(len, 3);
            assert_eq!(items[0].fields().unwrap().label(), "Copy prompt");
            assert!(items[1].is_separator());
            assert_eq!(items[2].fields().unwrap().label(), "Edit prompt");
            assert!(!items[2].fields().unwrap().is_disabled());
        });
    })
}

#[test]
fn test_prompt_context_menu_items_for_context_chips() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let terminal = add_window_with_terminal(&mut app, None);
        terminal.update(&mut app, |view, ctx| {
            let model = view.model.lock();
            view.current_prompt.update(ctx, |prompt, ctx| {
                let PromptType::Dynamic { prompt } = prompt else {
                    return;
                };
                prompt.update(ctx, |prompt, ctx| {
                    prompt.update_context(model.block_list().active_block(), ctx)
                });
            })
        });

        // Set the prompt to something we can actually read for.
        let prompt = Prompt::handle(&app);
        prompt.update(&mut app, |prompt, ctx| {
            prompt
                .update(
                    [ContextChipKind::Time12],
                    false,
                    WarpPromptSeparator::None,
                    ctx,
                )
                .expect("updating prompt to time chip failed");
        });

        let session_settings = SessionSettings::handle(&app);
        session_settings.update(&mut app, |settings, ctx| {
            // Force a toggle so the change event fires.
            let _ = settings.honor_ps1.set_value(true, ctx);
            let _ = settings.honor_ps1.set_value(false, ctx);
        });

        terminal.read(&app, |view, ctx| {
            let items: Vec<MenuItem<TerminalAction>> = view.prompt_context_menu_items(ctx);
            assert_eq!(items.len(), 5);

            // We expect the prompt menu items to be something like the following when context chips are used:
            // Copy prompt
            // ------------
            // <context chip specific actions>
            // ------------
            // Edit prompt
            assert_eq!(items[0].fields().unwrap().label(), "Copy prompt");
            assert!(items[1].is_separator());
            assert_eq!(
                items[2].fields().unwrap().label(),
                "Copy Time (12-hour format)"
            );
            assert!(items[3].is_separator());
            assert_eq!(items[4].fields().unwrap().label(), "Edit prompt");
            assert!(!items[4].fields().unwrap().is_disabled());
        });
    })
}

#[test]
fn test_prompt_context_menu_items_for_no_context_chips() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let terminal = add_window_with_terminal(&mut app, None);
        terminal.update(&mut app, |view, ctx| {
            let model = view.model.lock();
            view.current_prompt.update(ctx, |prompt, ctx| {
                let PromptType::Dynamic { prompt } = prompt else {
                    return;
                };
                prompt.update(ctx, |prompt, ctx| {
                    prompt.update_context(model.block_list().active_block(), ctx)
                });
            })
        });

        let session_settings = SessionSettings::handle(&app);
        session_settings.update(&mut app, |settings, ctx| {
            let _ = settings.honor_ps1.set_value(false, ctx);
        });

        terminal.read(&app, |view, ctx| {
            let items: Vec<MenuItem<TerminalAction>> = view.prompt_context_menu_items(ctx);
            assert_eq!(items.len(), 3);

            // We expect the prompt menu items to be something like the following when no context chips exist:
            // Copy prompt
            // ------------
            // Edit prompt
            assert_eq!(items[0].fields().unwrap().label(), "Copy prompt");
            assert!(items[1].is_separator());
            assert_eq!(items[2].fields().unwrap().label(), "Edit prompt");
            assert!(!items[2].fields().unwrap().is_disabled());
        });
    })
}

#[test]
fn test_prompt_context_menu_items_for_agent_toolbelt_flag() {
    let _agent_view_guard = FeatureFlag::AgentView.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let terminal = add_window_with_terminal(&mut app, None);
        terminal.update(&mut app, |view, ctx| {
            view.agent_view_controller().update(ctx, |controller, ctx| {
                controller
                    .try_enter_agent_view(
                        None,
                        AgentViewEntryOrigin::Input {
                            was_prompt_autodetected: false,
                        },
                        ctx,
                    )
                    .expect("Should be able to enter agent view");
            });
        });

        {
            let _agent_footer_guard = FeatureFlag::AgentToolbarEditor.override_enabled(false);
            terminal.read(&app, |view, ctx| {
                let items = view.prompt_context_menu_items(ctx);
                let labels = items
                    .iter()
                    .filter_map(|item| item.fields().map(|fields| fields.label()))
                    .collect::<Vec<_>>();

                assert!(!labels.contains(&"Edit prompt"));
                assert!(!labels.contains(&"Edit agent toolbelt"));
            });
        }

        {
            let _agent_footer_guard = FeatureFlag::AgentToolbarEditor.override_enabled(true);
            terminal.read(&app, |view, ctx| {
                let items = view.prompt_context_menu_items(ctx);
                let labels = items
                    .iter()
                    .filter_map(|item| item.fields().map(|fields| fields.label()))
                    .collect::<Vec<_>>();
                assert!(!labels.contains(&"Edit prompt"));
                assert!(labels.contains(&"Edit agent toolbelt"));
            });
        }
    })
}

#[test]
fn agent_footer_updates_chip_groups_when_side_assignment_changes() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        FeatureFlag::AgentView.set_enabled(true);

        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |view, ctx| {
            let model = view.model.lock();
            view.current_prompt.update(ctx, |prompt, ctx| {
                let PromptType::Dynamic { prompt } = prompt else {
                    return;
                };
                prompt.update(ctx, |prompt, ctx| {
                    prompt.update_context(model.block_list().active_block(), ctx);
                });
            });
        });

        SessionSettings::handle(&app).update(&mut app, |settings, ctx| {
            let _ = settings.agent_footer_chip_selection.set_value(
                AgentToolbarChipSelection::Custom {
                    left: vec![AgentToolbarItemKind::ContextChip(ContextChipKind::Time12)],
                    right: vec![AgentToolbarItemKind::ContextChip(ContextChipKind::Time24)],
                },
                ctx,
            );
        });

        assert_eventually!(
            terminal.read(&app, |view, ctx| {
                view.input().as_ref(ctx).agent_footer_chip_kinds(ctx)
                    == (vec![ContextChipKind::Time12], vec![ContextChipKind::Time24])
            }),
            "Agent footer should render separate left and right chip groups"
        );

        SessionSettings::handle(&app).update(&mut app, |settings, ctx| {
            let _ = settings.agent_footer_chip_selection.set_value(
                AgentToolbarChipSelection::Custom {
                    left: vec![
                        AgentToolbarItemKind::ContextChip(ContextChipKind::Time12),
                        AgentToolbarItemKind::ContextChip(ContextChipKind::Time24),
                    ],
                    right: vec![],
                },
                ctx,
            );
        });

        assert_eventually!(
            terminal.read(&app, |view, ctx| {
                view.input().as_ref(ctx).agent_footer_chip_kinds(ctx)
                    == (
                        vec![ContextChipKind::Time12, ContextChipKind::Time24],
                        vec![],
                    )
            }),
            "Agent footer should update when a chip moves between sides without changing overall chip order"
        );
    })
}

#[test]
fn test_link_at_range_trims_zero_width_spaces() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);

        // NOTE: this has two zero-width spaces, one after the '(', and one before the ')'
        let input_url = "(\u{200b}https://warp.dev\u{200b})";
        // NOTE: the final character in this string is a zero-width space
        let non_escaped_url = "https://warp.dev\u{200b}";
        let escaped_url = "https://warp.dev";

        terminal.update(&mut app, |view, _ctx| {
            view.model.lock().simulate_block(
                r"printf '(%bhttps://warp.dev%b)\n' '\U200b' '\U200b'",
                input_url,
            );
        });

        terminal.read(&app, |view, ctx| {
            let model = view.model.lock();

            let block = view
                .viewport_state(model.block_list(), InputMode::PinnedToBottom, ctx)
                .iter()
                .next()
                .expect("blocklist should have at least one item");

            let point = WithinModel::BlockList(WithinBlock::new(
                // I picked the point 0, 4 b/c it seemed to work. It's not clear to me
                // why 4 works when numbers like 9 do not. Either way, this is just to
                // get the actual url out (passing 9 fails on url_at_point), and does
                // not matter for testing link_at_range.
                Point::new(0, 4),
                block.block_index.expect("block index should exist"),
                crate::terminal::GridType::Output,
            ));

            let url = model
                .url_at_point(&point)
                .expect("url at the designated point should exist");

            // Assert that string_at_range preserves the ZW Space
            assert_eq!(
                model.string_at_range(&url, RespectObfuscatedSecrets::No),
                non_escaped_url
            );

            // Assert that link_at_range removes the ZW Space
            assert_eq!(
                model.link_at_range(&url, RespectObfuscatedSecrets::No),
                escaped_url
            );
        });
    })
}

#[test]
fn test_scroll_position_doesnt_change_when_block_finished() {
    use futures_lite::StreamExt;

    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);

        let (tx, rx) = async_channel::bounded(1);
        app.update(|ctx| {
            ctx.subscribe_to_view(&terminal, move |_, event, _| {
                if let Event::BlockCompleted { block, .. } = event {
                    let output = std::str::from_utf8(&block.stylized_output).unwrap();
                    if output.trim() == "lr" {
                        tx.try_send(()).expect("Can send over channel");
                    }
                }
            });
        });

        let scroll_position_before_finished = terminal.update(&mut app, |view, ctx| {
            // Finish a lengthy block.
            view.model.lock().simulate_block("ls", &"\n".repeat(1000));
            assert!(view.is_vertically_scrollable(ctx));
            assert_eq!(
                view.scroll_position(),
                ScrollPosition::FollowsBottomOfMostRecentBlock
            );

            // Start long-running block.
            view.model.lock().simulate_long_running_block("", "lr");

            // Before the block is finished, scroll up.
            view.scroll(1.0.into_lines(), ctx);
            let scroll_position_before_finished = view.scroll_position();
            assert!(matches!(
                scroll_position_before_finished,
                ScrollPosition::FixedAtPosition { .. }
            ));

            // Finish the block.
            view.model.lock().finish_block();

            scroll_position_before_finished
        });

        // Wait until the terminal view acknowledges the block as completed.
        assert!(pin!(rx).next().await.is_some());

        // Make sure the scroll position is unchanged when the block finishes.
        terminal.read(&app, |view, _| {
            let scroll_position_after_finished = view.scroll_position();
            assert_eq!(
                scroll_position_before_finished,
                scroll_position_after_finished
            );
        });
    })
}

#[test]
fn inline_agent_view_exits_when_tagged_in_long_running_command_is_tagged_out() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        FeatureFlag::AgentView.set_enabled(true);

        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |view, ctx| {
            {
                let mut model = view.model.lock();
                model.init_shell(InitShellValue {
                    session_id: 0.into(),
                    shell: "zsh".to_owned(),
                    ..Default::default()
                });
                model.bootstrapped(BootstrappedValue {
                    shell: "zsh".to_owned(),
                    ..Default::default()
                });
                model.simulate_long_running_block("sleep 10", "running");
            }

            view.agent_view_controller().update(ctx, |controller, ctx| {
                controller
                    .try_enter_inline_agent_view(
                        None,
                        AgentViewEntryOrigin::LongRunningCommand,
                        ctx,
                    )
                    .expect("should enter inline agent view for a tagged-in command");
            });
            view.model
                .lock()
                .block_list_mut()
                .active_block_mut()
                .set_is_agent_tagged_in(true);

            assert!(view.agent_view_controller().as_ref(ctx).is_inline());
            assert!(view
                .model
                .lock()
                .block_list()
                .active_block()
                .is_agent_tagged_in());

            let model = view.model.lock();
            assert!(view.is_input_box_visible(&model, ctx));
            drop(model);

            view.handle_action(&TerminalAction::SetInputModeTerminal, ctx);

            assert!(!view.agent_view_controller().as_ref(ctx).is_active());
            let model = view.model.lock();
            let active_block = model.block_list().active_block();
            assert!(!active_block.is_agent_tagged_in());
            assert!(!view.is_input_box_visible(&model, ctx));
        });
    })
}

#[test]
fn ctrl_c_after_stop_takeover_cancels_conversation() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        FeatureFlag::AgentView.set_enabled(true);

        let terminal = add_window_with_terminal(&mut app, None);
        let pty_writes: Rc<RefCell<Vec<Vec<u8>>>> = Rc::new(RefCell::new(Vec::new()));
        let writes = pty_writes.clone();
        app.update(|ctx| {
            ctx.subscribe_to_view(&terminal, move |_, event, _| {
                if let Event::WriteBytesToPty { bytes } = event {
                    writes.borrow_mut().push(bytes.to_vec());
                }
            });
        });

        let conversation_id = terminal.update(&mut app, |view, ctx| {
            let conversation_id =
                BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                    history.start_new_conversation(view.view_id, false, false, false, ctx)
                });

            view.model
                .lock()
                .simulate_long_running_block("sleep 20", "running");
            let task_id = TaskId::new("test-cli-subagent".to_owned());
            view.model
                .lock()
                .block_list_mut()
                .active_block_mut()
                .set_agent_interaction_mode_for_agent_monitored_command(&task_id, conversation_id)
                .expect("command should become agent monitored");

            view.cli_subagent_controller.update(ctx, |controller, ctx| {
                controller.switch_control_to_user(UserTakeOverReason::Stop, ctx);
            });

            conversation_id
        });

        terminal.update(&mut app, |view, ctx| {
            view.handle_action(&TerminalAction::CtrlC, ctx);
        });

        assert_eq!(*pty_writes.borrow(), vec![vec![C0::ETX]]);
        terminal.read(&app, |_, ctx| {
            let conversation = BlocklistAIHistoryModel::as_ref(ctx)
                .conversation(&conversation_id)
                .expect("conversation should exist");
            assert_eq!(conversation.status(), &ConversationStatus::Cancelled);
        });
    })
}

#[test]
fn ctrl_c_after_transfer_takeover_does_not_cancel_conversation() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        FeatureFlag::AgentView.set_enabled(true);

        let terminal = add_window_with_terminal(&mut app, None);
        let pty_writes: Rc<RefCell<Vec<Vec<u8>>>> = Rc::new(RefCell::new(Vec::new()));
        let writes = pty_writes.clone();
        app.update(|ctx| {
            ctx.subscribe_to_view(&terminal, move |_, event, _| {
                if let Event::WriteBytesToPty { bytes } = event {
                    writes.borrow_mut().push(bytes.to_vec());
                }
            });
        });

        let conversation_id = terminal.update(&mut app, |view, ctx| {
            let conversation_id =
                BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                    history.start_new_conversation(view.view_id, false, false, false, ctx)
                });

            view.model
                .lock()
                .simulate_long_running_block("ssh localhost", "Password:");
            let task_id = TaskId::new("test-cli-subagent".to_owned());
            view.model
                .lock()
                .block_list_mut()
                .active_block_mut()
                .set_agent_interaction_mode_for_agent_monitored_command(&task_id, conversation_id)
                .expect("command should become agent monitored");

            view.cli_subagent_controller.update(ctx, |controller, ctx| {
                controller.switch_control_to_user(
                    UserTakeOverReason::TransferFromAgent {
                        reason: "Enter your password".to_owned(),
                    },
                    ctx,
                );
            });

            conversation_id
        });

        terminal.update(&mut app, |view, ctx| {
            view.handle_action(&TerminalAction::CtrlC, ctx);
        });

        assert_eq!(*pty_writes.borrow(), vec![vec![C0::ETX]]);
        terminal.read(&app, |_, ctx| {
            let conversation = BlocklistAIHistoryModel::as_ref(ctx)
                .conversation(&conversation_id)
                .expect("conversation should exist");
            // Transfer takeover is still waiting to report control handback or command
            // completion to the agent, so Ctrl-C should interrupt the command without
            // cancelling the conversation.
            assert_eq!(conversation.status(), &ConversationStatus::InProgress);
        });
    })
}

#[test]
fn inline_agent_view_persists_across_transfer_takeover_for_monitored_long_running_command() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        FeatureFlag::AgentView.set_enabled(true);

        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |view, ctx| {
            {
                let mut model = view.model.lock();
                model.init_shell(InitShellValue {
                    session_id: 0.into(),
                    shell: "zsh".to_owned(),
                    ..Default::default()
                });
                model.bootstrapped(BootstrappedValue {
                    shell: "zsh".to_owned(),
                    ..Default::default()
                });
                model.simulate_long_running_block("ssh localhost", "Password:");
            }

            let conversation_id = view.agent_view_controller().update(ctx, |controller, ctx| {
                controller
                    .try_enter_inline_agent_view(
                        None,
                        AgentViewEntryOrigin::LongRunningCommand,
                        ctx,
                    )
                    .expect("inline agent view should create a conversation")
            });
            view.model
                .lock()
                .block_list_mut()
                .active_block_mut()
                .set_is_agent_tagged_in(true);

            let task_id = TaskId::new("test-task".to_owned());
            view.model
                .lock()
                .block_list_mut()
                .active_block_mut()
                .set_agent_interaction_mode_for_agent_monitored_command(&task_id, conversation_id)
                .expect("tagged-in command should transition to agent-monitored");

            assert!(view.agent_view_controller().as_ref(ctx).is_inline());

            let model = view.model.lock();
            assert!(model.block_list().active_block().is_agent_in_control());
            assert!(view.is_input_box_visible(&model, ctx));
            drop(model);

            view.cli_subagent_controller.update(ctx, |controller, ctx| {
                controller.switch_control_to_user(
                    UserTakeOverReason::TransferFromAgent {
                        reason: "Enter your password".to_owned(),
                    },
                    ctx,
                );
            });

            assert!(view.agent_view_controller().as_ref(ctx).is_inline());
            let model = view.model.lock();
            let active_block = model.block_list().active_block();
            assert!(active_block.is_eligible_for_agent_handoff());
            assert!(!view.is_input_box_visible(&model, ctx));
            drop(model);

            view.cli_subagent_controller.update(ctx, |controller, ctx| {
                controller.handoff_active_command_control_to_agent(ctx);
            });

            assert!(view.agent_view_controller().as_ref(ctx).is_inline());
            let model = view.model.lock();
            let active_block = model.block_list().active_block();
            assert!(active_block.is_agent_in_control());
            assert!(view.is_input_box_visible(&model, ctx));
        });
    })
}

#[test]
fn use_agent_footer_renders_for_transfer_handoff_even_when_user_command_footer_setting_disabled() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        FeatureFlag::AgentView.set_enabled(true);
        AISettings::handle(&app).update(&mut app, |settings, ctx| {
            let _ = settings
                .should_render_use_agent_footer_for_user_commands
                .set_value(false, ctx);
        });

        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |view, ctx| {
            {
                let mut model = view.model.lock();
                model.init_shell(InitShellValue {
                    session_id: 0.into(),
                    shell: "zsh".to_owned(),
                    ..Default::default()
                });
                model.bootstrapped(BootstrappedValue {
                    shell: "zsh".to_owned(),
                    ..Default::default()
                });
                model.simulate_long_running_block("ssh localhost", "Password:");
            }

            view.maybe_show_use_agent_footer_in_blocklist(ctx);
            {
                let model = view.model.lock();
                assert!(!view.should_render_use_agent_footer(&model, ctx));
                let active_block_index = model.block_list().active_block_index();
                assert!(model
                    .block_list()
                    .last_non_hidden_rich_content_block_after_block(Some(active_block_index))
                    .is_none());
            }

            let conversation_id = view.agent_view_controller().update(ctx, |controller, ctx| {
                controller
                    .try_enter_inline_agent_view(
                        None,
                        AgentViewEntryOrigin::LongRunningCommand,
                        ctx,
                    )
                    .expect("inline agent view should create a conversation")
            });
            view.model
                .lock()
                .block_list_mut()
                .active_block_mut()
                .set_is_agent_tagged_in(true);

            let task_id = TaskId::new("test-task".to_owned());
            view.model
                .lock()
                .block_list_mut()
                .active_block_mut()
                .set_agent_interaction_mode_for_agent_monitored_command(&task_id, conversation_id)
                .expect("tagged-in command should transition to agent-monitored");

            view.cli_subagent_controller.update(ctx, |controller, ctx| {
                controller.switch_control_to_user(
                    UserTakeOverReason::TransferFromAgent {
                        reason: "Enter your password".to_owned(),
                    },
                    ctx,
                );
            });

            view.maybe_show_use_agent_footer_in_blocklist(ctx);
            let model = view.model.lock();
            assert!(view.should_render_use_agent_footer(&model, ctx));
            let active_block_index = model.block_list().active_block_index();
            let rendered_footer_view_id = model
                .block_list()
                .last_non_hidden_rich_content_block_after_block(Some(active_block_index))
                .map(|(_, item)| item.view_id);
            assert_eq!(rendered_footer_view_id, Some(view.use_agent_footer.id()));
        });
    })
}

#[test]
fn test_first_onboarding_block_exists() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |terminal_view, ctx| {
            terminal_view.handle_action(
                &TerminalAction::OnboardingFlow(OnboardingVersion::Legacy),
                ctx,
            );
        });
        terminal.update(&mut app, |terminal_view, ctx| {
            assert!(terminal_view.block_onboarding_active);
            // Here we assert that Agentic Suggestions block is the first one. As we modify the sequence, this test will have to be updated.
            ctx.subscribe_to_model(&History::handle(ctx), move |me, _, event, _| match event {
                HistoryEvent::Initialized(_) => {
                    assert!(me.onboarding_agentic_suggestions_block.is_some());
                }
            });
        });
    })
}

#[test]
fn exiting_agent_view_removes_empty_conversations() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);

        // Enter agent view (creates new conversation)
        let conversation_id = terminal.update(&mut app, |view, ctx| {
            view.agent_view_controller().update(ctx, |controller, ctx| {
                controller
                    .try_enter_agent_view(
                        None,
                        AgentViewEntryOrigin::Input {
                            was_prompt_autodetected: false,
                        },
                        ctx,
                    )
                    .expect("Should be able to enter agent view")
            })
        });

        // Entering agent view without specifying a conversation creates a new conversation.
        let exists_before_exit = BlocklistAIHistoryModel::handle(&app).read(&app, |history, _| {
            history
                .conversation(&conversation_id)
                .is_some_and(|c| c.exchange_count() == 0)
        });
        assert!(exists_before_exit);

        // Sanity: conversation exists but has no exchanges.
        terminal.update(&mut app, |view, ctx| {
            view.agent_view_controller()
                .update(ctx, |controller, ctx| controller.exit_agent_view(ctx))
        });

        // Exiting agent view should remove the empty conversation.
        let exists_after_exit = BlocklistAIHistoryModel::handle(&app).read(&app, |history, _| {
            history.conversation(&conversation_id).is_some()
        });
        assert!(!exists_after_exit);
    })
}

#[test]
fn ctrl_c_exit_agent_view_requires_confirmation() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        FeatureFlag::AgentView.set_enabled(true);

        let terminal = add_window_with_terminal(&mut app, None);

        // Enter agent view (creates new conversation)
        terminal.update(&mut app, |view, ctx| {
            view.agent_view_controller().update(ctx, |controller, ctx| {
                controller
                    .try_enter_agent_view(
                        None,
                        AgentViewEntryOrigin::Input {
                            was_prompt_autodetected: false,
                        },
                        ctx,
                    )
                    .expect("Should be able to enter agent view")
            })
        });

        // First ctrl-c should arm confirmation but not exit.
        terminal.update(&mut app, |view, ctx| {
            assert!(view.agent_view_controller().as_ref(ctx).is_active());
            view.handle_input_event(
                &InputEvent::CtrlC {
                    cleared_buffer_len: 0,
                },
                ctx,
            );
            assert!(view.agent_view_controller().as_ref(ctx).is_active());
        });

        // Second ctrl-c should confirm and exit.
        terminal.update(&mut app, |view, ctx| {
            view.handle_input_event(
                &InputEvent::CtrlC {
                    cleared_buffer_len: 0,
                },
                ctx,
            );
            assert!(!view.agent_view_controller().as_ref(ctx).is_active());
        });
    })
}

#[test]
fn ctrl_c_buffer_clear_then_exit_requires_three_presses_in_agent_view() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        FeatureFlag::AgentView.set_enabled(true);

        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |view, ctx| {
            view.agent_view_controller().update(ctx, |controller, ctx| {
                controller
                    .try_enter_agent_view(
                        None,
                        AgentViewEntryOrigin::Input {
                            was_prompt_autodetected: false,
                        },
                        ctx,
                    )
                    .expect("Should be able to enter agent view")
            })
        });

        // 1st ctrl-c clears input buffer (simulated) and should not trigger cancel/exit.
        terminal.update(&mut app, |view, ctx| {
            assert!(view.agent_view_controller().as_ref(ctx).is_active());
            view.handle_input_event(
                &InputEvent::CtrlC {
                    cleared_buffer_len: 5,
                },
                ctx,
            );
            assert!(view.agent_view_controller().as_ref(ctx).is_active());
        });

        // 2nd ctrl-c arms exit confirmation.
        terminal.update(&mut app, |view, ctx| {
            view.handle_input_event(
                &InputEvent::CtrlC {
                    cleared_buffer_len: 0,
                },
                ctx,
            );
            assert!(view.agent_view_controller().as_ref(ctx).is_active());
        });

        // 3rd ctrl-c confirms and exits.
        terminal.update(&mut app, |view, ctx| {
            view.handle_input_event(
                &InputEvent::CtrlC {
                    cleared_buffer_len: 0,
                },
                ctx,
            );
            assert!(!view.agent_view_controller().as_ref(ctx).is_active());
        });
    })
}

#[test]
fn terminal_action_ctrl_c_exit_agent_view_requires_confirmation() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        FeatureFlag::AgentView.set_enabled(true);

        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |view, ctx| {
            view.agent_view_controller().update(ctx, |controller, ctx| {
                controller
                    .try_enter_agent_view(
                        None,
                        AgentViewEntryOrigin::Input {
                            was_prompt_autodetected: false,
                        },
                        ctx,
                    )
                    .expect("Should be able to enter agent view")
            })
        });

        terminal.update(&mut app, |view, ctx| {
            assert!(view.agent_view_controller().as_ref(ctx).is_active());
            view.handle_action(&TerminalAction::CtrlC, ctx);
            assert!(view.agent_view_controller().as_ref(ctx).is_active());
        });

        terminal.update(&mut app, |view, ctx| {
            view.handle_action(&TerminalAction::CtrlC, ctx);
            assert!(!view.agent_view_controller().as_ref(ctx).is_active());
        });
    })
}

/// Sets up a CLI agent session, opens rich input, submits `text`, and returns
/// the terminal handle and the collected PTY writes.
#[allow(clippy::type_complexity)]
fn submit_rich_input_and_collect_pty_writes(
    app: &mut App,
    agent: CLIAgent,
    text: &str,
) -> (ViewHandle<TerminalView>, Rc<RefCell<Vec<Vec<u8>>>>) {
    let terminal = add_window_with_terminal(app, None);
    let pty_writes: Rc<RefCell<Vec<Vec<u8>>>> = Rc::new(RefCell::new(Vec::new()));
    let writes = pty_writes.clone();
    app.update(|ctx| {
        ctx.subscribe_to_view(&terminal, move |_, event, _| {
            if let Event::WriteBytesToPty { bytes } = event {
                writes.borrow_mut().push(bytes.to_vec());
            }
        });
    });

    terminal.update(app, |view, ctx| {
        CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
            sessions.set_session(
                view.view_id,
                CLIAgentSession {
                    agent,
                    status: CLIAgentSessionStatus::InProgress,
                    session_context: CLIAgentSessionContext::default(),
                    input_state: CLIAgentInputState::Closed,
                    should_auto_toggle_input: false,
                    listener: None,
                    remote_host: None,
                    plugin_version: None,
                    draft_text: None,
                    custom_command_prefix: None,
                    received_rich_notification: false,
                },
                ctx,
            );
        });

        view.open_cli_agent_rich_input(CLIAgentInputEntrypoint::FooterButton, ctx);
        assert!(view.has_active_cli_agent_input_session(ctx));

        view.submit_cli_agent_rich_input(text.to_owned(), ctx);
    });

    (terminal, pty_writes)
}

fn open_cli_agent_rich_input_for_agent(app: &mut App, agent: CLIAgent) -> ViewHandle<TerminalView> {
    open_cli_agent_rich_input_for_agent_with_window_id(app, agent).1
}

fn open_cli_agent_rich_input_for_agent_with_window_id(
    app: &mut App,
    agent: CLIAgent,
) -> (WindowId, ViewHandle<TerminalView>) {
    let (window_id, terminal) = add_window_with_id_and_terminal(app, None);
    terminal.update(app, |view, ctx| {
        CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
            sessions.set_session(
                view.view_id,
                CLIAgentSession {
                    agent,
                    status: CLIAgentSessionStatus::InProgress,
                    session_context: CLIAgentSessionContext::default(),
                    input_state: CLIAgentInputState::Closed,
                    should_auto_toggle_input: false,
                    listener: None,
                    remote_host: None,
                    plugin_version: None,
                    draft_text: None,
                    custom_command_prefix: None,
                    received_rich_notification: false,
                },
                ctx,
            );
        });

        view.open_cli_agent_rich_input(CLIAgentInputEntrypoint::FooterButton, ctx);
        assert!(view.has_active_cli_agent_input_session(ctx));
    });
    (window_id, terminal)
}

/// Verifies that Ctrl-G closes CLI agent rich input when dispatched from the
/// focused editor context. This is a regression test for #9286 where the
/// keybinding only matched the terminal context, not the embedded editor.
#[test]
fn ctrl_g_closes_cli_agent_rich_input_when_editor_is_focused() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        app.add_singleton_model(ImportedConfigModel::new);
        // Register keybindings so keystroke dispatch can match the Ctrl-G binding.
        app.update(|ctx| {
            crate::terminal::init(ctx);
            crate::editor::init(ctx);
        });
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let _cli_rich = FeatureFlag::CLIAgentRichInput.override_enabled(true);

        let (window_id, terminal) =
            open_cli_agent_rich_input_for_agent_with_window_id(&mut app, CLIAgent::OpenCode);

        // Dispatch Ctrl-G through the focused editor's responder chain.
        let (input_id, editor_id) = terminal.read(&app, |view, ctx| {
            let input = view.input.clone();
            let editor = input.as_ref(ctx).editor().clone();
            (input.id(), editor.id())
        });
        let handled = app
            .dispatch_keystroke(
                window_id,
                &[terminal.id(), input_id, editor_id],
                &warpui::keymap::Keystroke::parse("ctrl-g").expect("valid keystroke"),
                false,
            )
            .expect("dispatch should succeed");

        assert!(handled, "ctrl-g should be handled from the focused editor");
        terminal.read(&app, |view, ctx| {
            assert!(
                !view.has_active_cli_agent_input_session(ctx),
                "rich input should be closed after Ctrl-G"
            );
        });
    })
}

/// Verifies that Ctrl-G closes CLI agent rich input when dispatched from the
/// terminal context alone (no editor in the responder chain). Regression test
/// for #9916 where the keybinding only opened rich input but did not close it
/// in scenarios where focus was outside the embedded editor and the active
/// block had transitioned out of `LongRunningCommand` — for example, when the
/// CLI agent has paused waiting for user input.
#[test]
fn ctrl_g_closes_cli_agent_rich_input_from_terminal_context() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        app.add_singleton_model(ImportedConfigModel::new);
        // Register keybindings so keystroke dispatch can match the Ctrl-G binding.
        app.update(|ctx| {
            crate::terminal::init(ctx);
            crate::editor::init(ctx);
        });
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let _cli_rich = FeatureFlag::CLIAgentRichInput.override_enabled(true);

        let (window_id, terminal) =
            open_cli_agent_rich_input_for_agent_with_window_id(&mut app, CLIAgent::OpenCode);

        // Dispatch Ctrl-G with only the terminal view in the responder chain.
        // This simulates the case where focus is not on the embedded editor
        // (e.g., on the block list) and the previous Case 1 / Case 2 predicates
        // would both fail to match.
        let handled = app
            .dispatch_keystroke(
                window_id,
                &[terminal.id()],
                &warpui::keymap::Keystroke::parse("ctrl-g").expect("valid keystroke"),
                false,
            )
            .expect("dispatch should succeed");

        assert!(
            handled,
            "ctrl-g should be handled from the terminal context when rich input is open"
        );
        terminal.read(&app, |view, ctx| {
            assert!(
                !view.has_active_cli_agent_input_session(ctx),
                "rich input should be closed after Ctrl-G from terminal context"
            );
        });
    })
}

/// Verifies that Ctrl-G is a true toggle: opens then closes rich input from
/// the terminal context. Regression test for #9916.
#[test]
fn ctrl_g_toggles_cli_agent_rich_input_from_terminal_context() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        app.add_singleton_model(ImportedConfigModel::new);
        app.update(|ctx| {
            crate::terminal::init(ctx);
            crate::editor::init(ctx);
        });
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let _cli_rich = FeatureFlag::CLIAgentRichInput.override_enabled(true);

        // Start with rich input open, then close via Ctrl-G, then re-open via
        // direct call (Ctrl-G open path requires LongRunningCommand which is
        // tricky to simulate in a unit test), then close via Ctrl-G again.
        let (window_id, terminal) =
            open_cli_agent_rich_input_for_agent_with_window_id(&mut app, CLIAgent::OpenCode);

        let keystroke = warpui::keymap::Keystroke::parse("ctrl-g").expect("valid keystroke");

        // First close: rich input is open → Ctrl-G should close.
        let handled = app
            .dispatch_keystroke(window_id, &[terminal.id()], &keystroke, false)
            .expect("dispatch should succeed");
        assert!(handled, "first ctrl-g should be handled (close)");
        terminal.read(&app, |view, ctx| {
            assert!(
                !view.has_active_cli_agent_input_session(ctx),
                "rich input should be closed after first Ctrl-G"
            );
        });

        // Re-open programmatically (mirrors the user re-triggering open via
        // Ctrl-G in a long-running context).
        terminal.update(&mut app, |view, ctx| {
            view.open_cli_agent_rich_input(CLIAgentInputEntrypoint::CtrlG, ctx);
            assert!(view.has_active_cli_agent_input_session(ctx));
        });

        // Second close: rich input is open again → Ctrl-G should close again.
        let handled = app
            .dispatch_keystroke(window_id, &[terminal.id()], &keystroke, false)
            .expect("dispatch should succeed");
        assert!(handled, "second ctrl-g should be handled (close again)");
        terminal.read(&app, |view, ctx| {
            assert!(
                !view.has_active_cli_agent_input_session(ctx),
                "rich input should be closed after second Ctrl-G"
            );
        });
    })
}

#[test]
fn cli_agent_rich_input_hint_text_mentions_active_cli_agent() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let _cli_rich = FeatureFlag::CLIAgentRichInput.override_enabled(true);

        for (agent, expected_hint_text) in [
            (CLIAgent::Claude, "Enter prompt for Claude Code..."),
            (CLIAgent::Gemini, "Enter prompt for Gemini..."),
            (CLIAgent::Codex, "Enter prompt for Codex..."),
            (CLIAgent::Unknown, "Tell the agent what to build..."),
        ] {
            let terminal = open_cli_agent_rich_input_for_agent(&mut app, agent);
            terminal.read(&app, |view, ctx| {
                let placeholder_text = view
                    .input
                    .as_ref(ctx)
                    .editor()
                    .as_ref(ctx)
                    .placeholder_text("");
                assert_eq!(placeholder_text, Some(expected_hint_text));
            });
        }
    })
}

#[test]
fn cli_agent_rich_input_shell_mode_uses_run_commands_hint_text() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let _cli_rich = FeatureFlag::CLIAgentRichInput.override_enabled(true);

        let terminal = open_cli_agent_rich_input_for_agent(&mut app, CLIAgent::Claude);
        terminal.update(&mut app, |view, ctx| {
            view.input.update(ctx, |input, ctx| {
                input.ai_input_model().update(ctx, |ai_input, ctx| {
                    ai_input.set_input_config(
                        InputConfig {
                            input_type: InputType::Shell,
                            is_locked: true,
                        },
                        true,
                        None,
                        ctx,
                    );
                });
                input.set_zero_state_hint_text(ctx);
            });
        });
        terminal.read(&app, |view, ctx| {
            let placeholder_text = view
                .input
                .as_ref(ctx)
                .editor()
                .as_ref(ctx)
                .placeholder_text("");
            assert_eq!(placeholder_text, Some("Run commands"));
        });
    })
}

#[test]
fn submit_cli_agent_rich_input_codex_uses_bracketed_paste() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let _cli_rich = FeatureFlag::CLIAgentRichInput.override_enabled(true);

        let (_terminal, pty_writes) =
            submit_rich_input_and_collect_pty_writes(&mut app, CLIAgent::Codex, "hello");

        let writes = pty_writes.borrow();
        // BracketedPaste: first write is ESC[200~ + text + ESC[201~, second is \r.
        assert_eq!(
            writes.len(),
            2,
            "expected 2 PTY writes, got {}",
            writes.len()
        );

        let mut expected_paste =
            Vec::with_capacity(BRACKETED_PASTE_START.len() + 5 + BRACKETED_PASTE_END.len());
        expected_paste.extend_from_slice(BRACKETED_PASTE_START);
        expected_paste.extend_from_slice(b"hello");
        expected_paste.extend_from_slice(BRACKETED_PASTE_END);
        assert_eq!(writes[0], expected_paste);
        assert_eq!(writes[1], b"\r");
    })
}

#[test]
fn submit_cli_agent_rich_input_opencode_defers_enter_and_close() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let _cli_rich = FeatureFlag::CLIAgentRichInput.override_enabled(true);

        let (_terminal, pty_writes) =
            submit_rich_input_and_collect_pty_writes(&mut app, CLIAgent::OpenCode, "hello");

        // Immediately after submit, only the text should have been written;
        // the \r is sent after a short delay.
        assert_eq!(pty_writes.borrow().len(), 1);
        assert_eq!(pty_writes.borrow()[0], b"hello");

        // Wait for the delayed \r to arrive.
        assert_eventually!(
            pty_writes.borrow().len() == 2,
            "carriage return should be written after delay"
        );
        assert_eq!(pty_writes.borrow()[1], b"\r");
    })
}

#[test]
fn attach_path_as_context_routes_to_open_cli_agent_rich_input() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let _cli_rich = FeatureFlag::CLIAgentRichInput.override_enabled(true);
        let _hoa_code_review = FeatureFlag::HoaCodeReview.override_enabled(true);

        let terminal = open_cli_agent_rich_input_for_agent(&mut app, CLIAgent::Claude);
        let pty_writes: Rc<RefCell<Vec<Vec<u8>>>> = Rc::new(RefCell::new(Vec::new()));
        let writes = pty_writes.clone();
        app.update(|ctx| {
            ctx.subscribe_to_view(&terminal, move |_, event, _| {
                if let Event::WriteBytesToPty { bytes } = event {
                    writes.borrow_mut().push(bytes.to_vec());
                }
            });
        });

        terminal.update(&mut app, |view, ctx| {
            view.attach_path_as_context(std::path::Path::new("src/main.rs"), ctx);
        });

        terminal.read(&app, |view, ctx| {
            assert_eq!(view.input.as_ref(ctx).buffer_text(ctx), "src/main.rs");
        });
        assert!(
            pty_writes.borrow().is_empty(),
            "context should be inserted into rich input instead of written to PTY"
        );
    })
}
#[test]
fn drag_drop_image_in_cli_agent_long_running_command_pastes_via_clipboard() {
    // Regression test: dropping an image file into a tab where a CLI agent
    // (e.g. Claude Code) is the foreground long-running process should
    // mirror the Cmd+V image-paste path — write the image to the system
    // clipboard and send the agent's paste keystroke to the PTY — instead
    // of shell-escaping the path and typing it into the agent's prompt.
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);

        // The new path actually reads the file off disk, so we need a real
        // file. Bytes don't have to be a valid PNG.
        let mut image_path = std::env::temp_dir();
        image_path.push(format!(
            "warp-test-cli-agent-drop-{}.png",
            std::process::id()
        ));
        std::fs::write(&image_path, b"fake-png-bytes").expect("write tmp image");
        let image_path_str = image_path.to_string_lossy().into_owned();

        let terminal = add_window_with_terminal(&mut app, None);

        let pty_writes: Rc<RefCell<Vec<Vec<u8>>>> = Rc::new(RefCell::new(Vec::new()));
        let writes = pty_writes.clone();
        app.update(|ctx| {
            ctx.subscribe_to_view(&terminal, move |_, event, _| {
                if let Event::WriteBytesToPty { bytes } = event {
                    writes.borrow_mut().push(bytes.to_vec());
                }
            });
        });

        terminal.update(&mut app, |view, ctx| {
            CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
                sessions.set_session(
                    view.view_id,
                    CLIAgentSession {
                        agent: CLIAgent::Claude,
                        status: CLIAgentSessionStatus::InProgress,
                        session_context: CLIAgentSessionContext::default(),
                        input_state: CLIAgentInputState::Closed,
                        should_auto_toggle_input: false,
                        listener: None,
                        remote_host: None,
                        plugin_version: None,
                        draft_text: None,
                        custom_command_prefix: None,
                        received_rich_notification: false,
                    },
                    ctx,
                );
            });

            // The CLI-agent paste branch is gated on the active block being
            // long-running (the agent's TUI). Without a long-running block
            // we'd fall through to the regular image-attach flow.
            {
                let mut model = view.model.lock();
                model.simulate_long_running_block("claude", "");
                assert!(model
                    .block_list()
                    .active_block()
                    .is_active_and_long_running());
            }

            view.drag_and_drop_files(&[image_path_str], ctx);
        });

        // The paste flow is async (off-thread file read, then hop back to
        // the view to write the clipboard + paste keystroke). Wait for the
        // single PTY write of the platform-appropriate paste byte: 0x16
        // (Ctrl+V) on macOS/Linux, or `ESC v` on Windows. Without the fix
        // a shell-escaped path string is written here instead.
        let expected_paste_bytes: Vec<u8> = if cfg!(windows) {
            vec![0x1b, b'v']
        } else {
            vec![0x16]
        };
        assert_eventually!(
            pty_writes.borrow().len() == 1 && pty_writes.borrow()[0] == expected_paste_bytes,
            "expected single paste-keystroke PTY write {:?}; got {:?}",
            expected_paste_bytes,
            pty_writes.borrow()
        );

        std::fs::remove_file(&image_path).ok();
    })
}

#[test]
fn paste_raw_image_clipboard_in_cli_agent_sends_correct_bytes() {
    fn run_for_agent(agent: CLIAgent) {
        App::test((), move |mut app| async move {
            initialize_app_for_terminal_view(&mut app);
            let _agent_view = FeatureFlag::AgentView.override_enabled(true);

            let terminal = add_window_with_terminal(&mut app, None);

            let pty_writes: Rc<RefCell<Vec<Vec<u8>>>> = Rc::new(RefCell::new(Vec::new()));
            let writes = pty_writes.clone();
            app.update(|ctx| {
                ctx.subscribe_to_view(&terminal, move |_, event, _| {
                    if let Event::WriteBytesToPty { bytes } = event {
                        writes.borrow_mut().push(bytes.to_vec());
                    }
                });
            });

            terminal.update(&mut app, |view, ctx| {
                CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
                    sessions.set_session(
                        view.view_id,
                        CLIAgentSession {
                            agent,
                            status: CLIAgentSessionStatus::InProgress,
                            session_context: CLIAgentSessionContext::default(),
                            input_state: CLIAgentInputState::Closed,
                            should_auto_toggle_input: false,
                            listener: None,
                            remote_host: None,
                            plugin_version: None,
                            draft_text: None,
                            custom_command_prefix: None,
                            received_rich_notification: false,
                        },
                        ctx,
                    );
                });

                {
                    let mut model = view.model.lock();
                    model.simulate_long_running_block(agent.command_prefix(), "");
                    model.set_mode(ansi::Mode::BracketedPaste);
                }

                // Write image-only data to the clipboard (no text, no paths).
                ctx.clipboard().write(ClipboardContent {
                    images: Some(vec![warpui::clipboard::ImageData {
                        data: vec![0x89, 0x50, 0x4E, 0x47], // PNG magic bytes
                        mime_type: "image/png".to_string(),
                        filename: None,
                    }]),
                    ..Default::default()
                });

                view.handle_action(&TerminalAction::Paste, ctx);
            });

            let writes = pty_writes.borrow();
            assert_eq!(
                writes.len(),
                1,
                "expected 1 PTY write, got {}",
                writes.len()
            );

            if cfg!(windows) {
                if agent == CLIAgent::Claude {
                    assert_eq!(writes[0], vec![C0::ESC, b'v']);
                } else {
                    let mut expected = Vec::new();
                    expected.extend_from_slice(BRACKETED_PASTE_START);
                    expected.extend_from_slice(BRACKETED_PASTE_END);
                    assert_eq!(writes[0], expected);
                }
            } else {
                assert_eq!(writes[0], vec![C0::SYN]);
            }
        })
    }

    run_for_agent(CLIAgent::Claude);
    run_for_agent(CLIAgent::OpenCode);
    run_for_agent(CLIAgent::Codex);
}

#[test]
fn submit_without_auto_dismiss_keeps_rich_input_open() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let _cli_rich = FeatureFlag::CLIAgentRichInput.override_enabled(true);
        // auto_dismiss defaults to false — leave it off.

        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |view, ctx| {
            CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
                sessions.set_session(
                    view.view_id,
                    CLIAgentSession {
                        agent: CLIAgent::Claude,
                        status: CLIAgentSessionStatus::InProgress,
                        session_context: CLIAgentSessionContext::default(),
                        input_state: CLIAgentInputState::Closed,
                        should_auto_toggle_input: false,
                        listener: None,
                        remote_host: None,
                        plugin_version: None,
                        draft_text: None,
                        custom_command_prefix: None,
                        received_rich_notification: false,
                    },
                    ctx,
                );
            });

            view.open_cli_agent_rich_input(CLIAgentInputEntrypoint::FooterButton, ctx);
            assert!(view.has_active_cli_agent_input_session(ctx));

            view.submit_cli_agent_rich_input("hello".to_owned(), ctx);

            // Rich input stays open because auto_dismiss is off.
            assert!(view.has_active_cli_agent_input_session(ctx));
        });

        // Buffer should still be cleared even though rich input is open.
        terminal.read(&app, |view, ctx| {
            let input = view.input.as_ref(ctx);
            assert!(input.editor().as_ref(ctx).buffer_text(ctx).is_empty());
        });
    })
}

#[test]
fn submit_with_plugin_and_auto_toggle_keeps_rich_input_open() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let _cli_rich = FeatureFlag::CLIAgentRichInput.override_enabled(true);
        // auto_toggle_rich_input defaults to true.
        // Turn on auto_dismiss too — it should be overridden by auto_toggle.
        AISettings::handle(&app).update(&mut app, |settings, ctx| {
            let _ = settings
                .auto_dismiss_rich_input_after_submit
                .set_value(true, ctx);
        });

        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |view, ctx| {
            // Create a session with a plugin listener and should_auto_toggle_input.
            let listener = ctx.add_model(|ctx| {
                CLIAgentSessionListener::new(
                    view.view_id,
                    CLIAgent::Claude,
                    &view.model_events_handle,
                    ctx,
                )
            });
            CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
                sessions.set_session(
                    view.view_id,
                    CLIAgentSession {
                        agent: CLIAgent::Claude,
                        status: CLIAgentSessionStatus::InProgress,
                        session_context: CLIAgentSessionContext::default(),
                        input_state: CLIAgentInputState::Closed,
                        should_auto_toggle_input: true,
                        listener: Some(listener),
                        remote_host: None,
                        plugin_version: Some("1.0.0".to_owned()),
                        draft_text: None,
                        custom_command_prefix: None,
                        received_rich_notification: true,
                    },
                    ctx,
                );
            });

            view.open_cli_agent_rich_input(CLIAgentInputEntrypoint::FooterButton, ctx);
            assert!(view.has_active_cli_agent_input_session(ctx));

            view.submit_cli_agent_rich_input("hello".to_owned(), ctx);

            // Rich input stays open because auto_toggle + plugin takes precedence.
            assert!(view.has_active_cli_agent_input_session(ctx));
        });
    })
}

#[test]
fn submit_with_plugin_but_auto_toggle_off_respects_auto_dismiss() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let _cli_rich = FeatureFlag::CLIAgentRichInput.override_enabled(true);
        AISettings::handle(&app).update(&mut app, |settings, ctx| {
            let _ = settings.auto_toggle_rich_input.set_value(false, ctx);
            let _ = settings
                .auto_dismiss_rich_input_after_submit
                .set_value(true, ctx);
        });

        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |view, ctx| {
            let listener = ctx.add_model(|ctx| {
                CLIAgentSessionListener::new(
                    view.view_id,
                    CLIAgent::Claude,
                    &view.model_events_handle,
                    ctx,
                )
            });
            CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
                sessions.set_session(
                    view.view_id,
                    CLIAgentSession {
                        agent: CLIAgent::Claude,
                        status: CLIAgentSessionStatus::InProgress,
                        session_context: CLIAgentSessionContext::default(),
                        input_state: CLIAgentInputState::Closed,
                        should_auto_toggle_input: true,
                        listener: Some(listener),
                        remote_host: None,
                        plugin_version: Some("1.0.0".to_owned()),
                        draft_text: None,
                        custom_command_prefix: None,
                        received_rich_notification: false,
                    },
                    ctx,
                );
            });

            view.open_cli_agent_rich_input(CLIAgentInputEntrypoint::FooterButton, ctx);
            assert!(view.has_active_cli_agent_input_session(ctx));

            view.submit_cli_agent_rich_input("hello".to_owned(), ctx);
        });

        // auto_toggle is off, so auto_dismiss closes rich input.
        // Claude uses DelayedEnter, so the close happens after a timer.
        assert_eventually!(
            terminal.read(&app, |view, ctx| !view
                .has_active_cli_agent_input_session(ctx)),
            "Rich input should be closed after submit with auto_dismiss"
        );
    })
}

#[test]
fn status_blocked_auto_closes_rich_input() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let _cli_rich = FeatureFlag::CLIAgentRichInput.override_enabled(true);
        // auto_toggle_rich_input defaults to true.

        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |view, ctx| {
            let listener = ctx.add_model(|ctx| {
                CLIAgentSessionListener::new(
                    view.view_id,
                    CLIAgent::Claude,
                    &view.model_events_handle,
                    ctx,
                )
            });
            CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
                sessions.set_session(
                    view.view_id,
                    CLIAgentSession {
                        agent: CLIAgent::Claude,
                        status: CLIAgentSessionStatus::InProgress,
                        session_context: CLIAgentSessionContext::default(),
                        input_state: CLIAgentInputState::Closed,
                        should_auto_toggle_input: true,
                        listener: Some(listener),
                        remote_host: None,
                        plugin_version: Some("1.0.0".to_owned()),
                        draft_text: None,
                        custom_command_prefix: None,
                        received_rich_notification: false,
                    },
                    ctx,
                );
            });

            view.open_cli_agent_rich_input(CLIAgentInputEntrypoint::FooterButton, ctx);
            assert!(view.has_active_cli_agent_input_session(ctx));

            // Simulate a PermissionRequest event → status transitions to Blocked.
            CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
                sessions.update_from_event(
                    view.view_id,
                    &CLIAgentEvent {
                        source: CLIAgentEventSource::RichPlugin,
                        v: 1,
                        agent: CLIAgent::Claude,
                        event: CLIAgentEventType::PermissionRequest,
                        session_id: None,
                        cwd: None,
                        project: None,
                        payload: CLIAgentEventPayload {
                            summary: Some("Approve?".to_owned()),
                            ..Default::default()
                        },
                    },
                    ctx,
                );
            });
        });

        // The StatusChanged event is delivered to the terminal view, which
        // auto-closes rich input because the agent is blocked.
        terminal.read(&app, |view, ctx| {
            assert!(!view.has_active_cli_agent_input_session(ctx));
        });

        // should_auto_toggle_input is preserved so auto-open can fire later.
        terminal.read(&app, |_view, ctx| {
            let session = CLIAgentSessionsModel::as_ref(ctx).session(_view.view_id);
            assert!(session.unwrap().should_auto_toggle_input);
        });
    })
}

#[test]
fn status_in_progress_auto_opens_rich_input_after_blocked() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let _cli_rich = FeatureFlag::CLIAgentRichInput.override_enabled(true);

        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |view, ctx| {
            let listener = ctx.add_model(|ctx| {
                CLIAgentSessionListener::new(
                    view.view_id,
                    CLIAgent::Claude,
                    &view.model_events_handle,
                    ctx,
                )
            });
            CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
                sessions.set_session(
                    view.view_id,
                    CLIAgentSession {
                        agent: CLIAgent::Claude,
                        status: CLIAgentSessionStatus::InProgress,
                        session_context: CLIAgentSessionContext::default(),
                        input_state: CLIAgentInputState::Closed,
                        should_auto_toggle_input: true,
                        listener: Some(listener),
                        remote_host: None,
                        plugin_version: Some("1.0.0".to_owned()),
                        draft_text: None,
                        custom_command_prefix: None,
                        received_rich_notification: false,
                    },
                    ctx,
                );
            });

            // Open rich input, then simulate blocked → closed automatically.
            view.open_cli_agent_rich_input(CLIAgentInputEntrypoint::FooterButton, ctx);
            CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
                sessions.update_from_event(
                    view.view_id,
                    &CLIAgentEvent {
                        source: CLIAgentEventSource::RichPlugin,
                        v: 1,
                        agent: CLIAgent::Claude,
                        event: CLIAgentEventType::PermissionRequest,
                        session_id: None,
                        cwd: None,
                        project: None,
                        payload: CLIAgentEventPayload {
                            summary: Some("Approve?".to_owned()),
                            ..Default::default()
                        },
                    },
                    ctx,
                );
            });
        });

        // Rich input should be auto-closed from the blocked status.
        terminal.read(&app, |view, ctx| {
            assert!(!view.has_active_cli_agent_input_session(ctx));
        });

        // Simulate permission replied → status transitions back to InProgress.
        terminal.update(&mut app, |view, ctx| {
            CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
                sessions.update_from_event(
                    view.view_id,
                    &CLIAgentEvent {
                        source: CLIAgentEventSource::RichPlugin,
                        v: 1,
                        agent: CLIAgent::Claude,
                        event: CLIAgentEventType::PermissionReplied,
                        session_id: None,
                        cwd: None,
                        project: None,
                        payload: CLIAgentEventPayload::default(),
                    },
                    ctx,
                );
            });
        });

        // Rich input should auto-open because should_auto_toggle_input was preserved.
        terminal.read(&app, |view, ctx| {
            assert!(view.has_active_cli_agent_input_session(ctx));
        });
    })
}

// Regression test for https://github.com/warpdotdev/warp/issues/9059.
// Codex's listener doesn't emit Blocked-state events (it only forwards opaque
// OSC 9 notifications as Stop), so auto-toggling rich input would trap arrow
// keys when Codex shows interactive option menus. Auto-toggle must not fire
// for agents whose handlers report `supports_rich_status() == false`.
#[test]
fn codex_status_change_does_not_auto_open_rich_input() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let _cli_rich = FeatureFlag::CLIAgentRichInput.override_enabled(true);
        // auto_toggle_rich_input defaults to true.

        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |view, ctx| {
            let listener = ctx.add_model(|ctx| {
                CLIAgentSessionListener::new(
                    view.view_id,
                    CLIAgent::Codex,
                    &view.model_events_handle,
                    ctx,
                )
            });
            CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
                sessions.set_session(
                    view.view_id,
                    CLIAgentSession {
                        agent: CLIAgent::Codex,
                        status: CLIAgentSessionStatus::InProgress,
                        session_context: CLIAgentSessionContext::default(),
                        input_state: CLIAgentInputState::Closed,
                        should_auto_toggle_input: true,
                        listener: Some(listener),
                        remote_host: None,
                        plugin_version: None,
                        draft_text: None,
                        custom_command_prefix: None,
                        received_rich_notification: false,
                    },
                    ctx,
                );
            });

            // Rich input starts closed. Simulating a Stop event (the only
            // status Codex's handler ever emits) must not re-open it,
            // because the user may be navigating Codex's option menus.
            assert!(!view.has_active_cli_agent_input_session(ctx));
            CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
                sessions.update_from_event(
                    view.view_id,
                    &CLIAgentEvent {
                        source: CLIAgentEventSource::CodexOsc9Fallback,
                        v: 1,
                        agent: CLIAgent::Codex,
                        event: CLIAgentEventType::Stop,
                        session_id: None,
                        cwd: None,
                        project: None,
                        payload: CLIAgentEventPayload {
                            query: Some("Agent turn complete".to_owned()),
                            ..Default::default()
                        },
                    },
                    ctx,
                );
            });
        });

        terminal.read(&app, |view, ctx| {
            assert!(!view.has_active_cli_agent_input_session(ctx));
        });
    })
}

#[test]
fn cli_session_status_updates_active_child_conversation() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);

        let terminal = add_window_with_terminal(&mut app, None);

        let child_conversation_id = terminal.update(&mut app, |view, ctx| {
            let parent_conversation_id =
                BlocklistAIHistoryModel::handle(ctx).update(ctx, |history_model, ctx| {
                    history_model.start_new_conversation(view.view_id, false, false, false, ctx)
                });
            let child_conversation_id =
                BlocklistAIHistoryModel::handle(ctx).update(ctx, |history_model, ctx| {
                    history_model.start_new_child_conversation(
                        view.view_id,
                        "Agent 2".to_string(),
                        parent_conversation_id,
                        None,
                        ctx,
                    )
                });

            view.enter_agent_view(
                None,
                Some(child_conversation_id),
                AgentViewEntryOrigin::ChildAgent,
                ctx,
            );

            CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
                sessions.set_session(
                    view.view_id,
                    CLIAgentSession {
                        agent: CLIAgent::Claude,
                        status: CLIAgentSessionStatus::InProgress,
                        session_context: CLIAgentSessionContext::default(),
                        input_state: CLIAgentInputState::Closed,
                        should_auto_toggle_input: false,
                        listener: None,
                        remote_host: None,
                        plugin_version: None,
                        draft_text: None,
                        custom_command_prefix: None,
                        received_rich_notification: false,
                    },
                    ctx,
                );
            });

            child_conversation_id
        });

        terminal.read(&app, |_view, ctx| {
            let conversation = BlocklistAIHistoryModel::as_ref(ctx)
                .conversation(&child_conversation_id)
                .expect("child conversation should exist");
            assert_eq!(conversation.status(), &ConversationStatus::InProgress);
        });

        terminal.update(&mut app, |view, ctx| {
            CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
                sessions.update_from_event(
                    view.view_id,
                    &CLIAgentEvent {
                        source: CLIAgentEventSource::RichPlugin,
                        v: 1,
                        agent: CLIAgent::Claude,
                        event: CLIAgentEventType::PermissionRequest,
                        session_id: None,
                        cwd: None,
                        project: None,
                        payload: CLIAgentEventPayload {
                            summary: Some("Approve?".to_owned()),
                            ..Default::default()
                        },
                    },
                    ctx,
                );
            });
        });

        terminal.read(&app, |_view, ctx| {
            let conversation = BlocklistAIHistoryModel::as_ref(ctx)
                .conversation(&child_conversation_id)
                .expect("child conversation should exist");
            assert_eq!(
                conversation.status(),
                &ConversationStatus::Blocked {
                    blocked_action: "Approve?".to_string(),
                }
            );
        });

        terminal.update(&mut app, |view, ctx| {
            CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
                sessions.update_from_event(
                    view.view_id,
                    &CLIAgentEvent {
                        source: CLIAgentEventSource::RichPlugin,
                        v: 1,
                        agent: CLIAgent::Claude,
                        event: CLIAgentEventType::PermissionReplied,
                        session_id: None,
                        cwd: None,
                        project: None,
                        payload: CLIAgentEventPayload::default(),
                    },
                    ctx,
                );
            });
        });

        terminal.read(&app, |_view, ctx| {
            let conversation = BlocklistAIHistoryModel::as_ref(ctx)
                .conversation(&child_conversation_id)
                .expect("child conversation should exist");
            assert_eq!(conversation.status(), &ConversationStatus::InProgress);
        });

        terminal.update(&mut app, |view, ctx| {
            CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
                sessions.update_from_event(
                    view.view_id,
                    &CLIAgentEvent {
                        source: CLIAgentEventSource::RichPlugin,
                        v: 1,
                        agent: CLIAgent::Claude,
                        event: CLIAgentEventType::Stop,
                        session_id: None,
                        cwd: None,
                        project: None,
                        payload: CLIAgentEventPayload {
                            response: Some("Done".to_owned()),
                            ..Default::default()
                        },
                    },
                    ctx,
                );
            });
        });

        terminal.read(&app, |_view, ctx| {
            let conversation = BlocklistAIHistoryModel::as_ref(ctx)
                .conversation(&child_conversation_id)
                .expect("child conversation should exist");
            assert_eq!(conversation.status(), &ConversationStatus::Success);
        });
    })
}

#[test]
fn cli_session_status_updates_single_child_conversation_without_agent_view() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);

        let terminal = add_window_with_terminal(&mut app, None);

        let child_conversation_id = terminal.update(&mut app, |view, ctx| {
            let parent_conversation_id =
                BlocklistAIHistoryModel::handle(ctx).update(ctx, |history_model, ctx| {
                    history_model.start_new_conversation(view.view_id, false, false, false, ctx)
                });
            let child_conversation_id =
                BlocklistAIHistoryModel::handle(ctx).update(ctx, |history_model, ctx| {
                    history_model.start_new_child_conversation(
                        view.view_id,
                        "Agent 2".to_string(),
                        parent_conversation_id,
                        None,
                        ctx,
                    )
                });

            CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
                sessions.set_session(
                    view.view_id,
                    CLIAgentSession {
                        agent: CLIAgent::Claude,
                        status: CLIAgentSessionStatus::InProgress,
                        session_context: CLIAgentSessionContext::default(),
                        input_state: CLIAgentInputState::Closed,
                        should_auto_toggle_input: false,
                        listener: None,
                        remote_host: None,
                        plugin_version: None,
                        draft_text: None,
                        custom_command_prefix: None,
                        received_rich_notification: false,
                    },
                    ctx,
                );
            });

            child_conversation_id
        });

        terminal.read(&app, |_view, ctx| {
            let conversation = BlocklistAIHistoryModel::as_ref(ctx)
                .conversation(&child_conversation_id)
                .expect("child conversation should exist");
            assert_eq!(conversation.status(), &ConversationStatus::InProgress);
        });

        terminal.update(&mut app, |view, ctx| {
            CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
                sessions.update_from_event(
                    view.view_id,
                    &CLIAgentEvent {
                        source: CLIAgentEventSource::RichPlugin,
                        v: 1,
                        agent: CLIAgent::Claude,
                        event: CLIAgentEventType::Stop,
                        session_id: None,
                        cwd: None,
                        project: None,
                        payload: CLIAgentEventPayload {
                            response: Some("Done".to_owned()),
                            ..Default::default()
                        },
                    },
                    ctx,
                );
            });
        });

        terminal.read(&app, |_view, ctx| {
            let conversation = BlocklistAIHistoryModel::as_ref(ctx)
                .conversation(&child_conversation_id)
                .expect("child conversation should exist");
            assert_eq!(conversation.status(), &ConversationStatus::Success);
        });
    })
}

#[test]
fn manual_dismiss_disables_auto_toggle_for_session() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let _cli_rich = FeatureFlag::CLIAgentRichInput.override_enabled(true);

        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |view, ctx| {
            let listener = ctx.add_model(|ctx| {
                CLIAgentSessionListener::new(
                    view.view_id,
                    CLIAgent::Claude,
                    &view.model_events_handle,
                    ctx,
                )
            });
            CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
                sessions.set_session(
                    view.view_id,
                    CLIAgentSession {
                        agent: CLIAgent::Claude,
                        status: CLIAgentSessionStatus::InProgress,
                        session_context: CLIAgentSessionContext::default(),
                        input_state: CLIAgentInputState::Closed,
                        should_auto_toggle_input: true,
                        listener: Some(listener),
                        remote_host: None,
                        plugin_version: Some("1.0.0".to_owned()),
                        draft_text: None,
                        custom_command_prefix: None,
                        received_rich_notification: false,
                    },
                    ctx,
                );
            });

            view.open_cli_agent_rich_input(CLIAgentInputEntrypoint::FooterButton, ctx);
            assert!(view.has_active_cli_agent_input_session(ctx));

            // Manual dismiss via the "disable auto-toggle" path (Escape / Ctrl-G / footer).
            view.close_cli_agent_rich_input_and_disable_auto_toggle(ctx);
            assert!(!view.has_active_cli_agent_input_session(ctx));
        });

        // should_auto_toggle_input should now be false.
        terminal.read(&app, |view, ctx| {
            let session = CLIAgentSessionsModel::as_ref(ctx).session(view.view_id);
            assert!(!session.unwrap().should_auto_toggle_input);
        });

        // A status change to InProgress should NOT auto-open rich input.
        terminal.update(&mut app, |view, ctx| {
            // First move to Blocked so we can transition back.
            CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
                sessions.update_from_event(
                    view.view_id,
                    &CLIAgentEvent {
                        source: CLIAgentEventSource::RichPlugin,
                        v: 1,
                        agent: CLIAgent::Claude,
                        event: CLIAgentEventType::PermissionRequest,
                        session_id: None,
                        cwd: None,
                        project: None,
                        payload: CLIAgentEventPayload {
                            summary: Some("Approve?".to_owned()),
                            ..Default::default()
                        },
                    },
                    ctx,
                );
            });
            CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
                sessions.update_from_event(
                    view.view_id,
                    &CLIAgentEvent {
                        source: CLIAgentEventSource::RichPlugin,
                        v: 1,
                        agent: CLIAgent::Claude,
                        event: CLIAgentEventType::PermissionReplied,
                        session_id: None,
                        cwd: None,
                        project: None,
                        payload: CLIAgentEventPayload::default(),
                    },
                    ctx,
                );
            });
        });

        // Rich input should remain closed.
        terminal.read(&app, |view, ctx| {
            assert!(!view.has_active_cli_agent_input_session(ctx));
        });
    })
}

#[test]
fn close_cli_agent_rich_input_saves_draft_and_reopen_restores_it() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let _cli_rich = FeatureFlag::CLIAgentRichInput.override_enabled(true);

        let terminal = open_cli_agent_rich_input_for_agent(&mut app, CLIAgent::Claude);

        // Type some text into the composer.
        terminal.update(&mut app, |view, ctx| {
            view.input.update(ctx, |input, ctx| {
                input.replace_buffer_content("work in progress", ctx);
            });
        });

        // Close the composer — the buffer text should be saved as a draft.
        terminal.update(&mut app, |view, ctx| {
            view.close_cli_agent_rich_input(CLIAgentRichInputCloseReason::Manual, ctx);
            assert!(!view.has_active_cli_agent_input_session(ctx));
        });

        terminal.read(&app, |view, ctx| {
            let session = CLIAgentSessionsModel::as_ref(ctx)
                .session(view.view_id)
                .expect("session should exist");
            assert_eq!(
                session.draft_text.as_deref(),
                Some("work in progress"),
                "draft should be saved on close"
            );
        });

        // Reopen — draft should be restored into the buffer and consumed.
        terminal.update(&mut app, |view, ctx| {
            view.open_cli_agent_rich_input(CLIAgentInputEntrypoint::FooterButton, ctx);
            assert!(view.has_active_cli_agent_input_session(ctx));
        });

        terminal.read(&app, |view, ctx| {
            assert_eq!(
                view.input.as_ref(ctx).buffer_text(ctx),
                "work in progress",
                "draft should be restored on reopen"
            );
            let session = CLIAgentSessionsModel::as_ref(ctx)
                .session(view.view_id)
                .expect("session should exist");
            assert_eq!(
                session.draft_text, None,
                "draft should be consumed after restore"
            );
        });
    })
}

#[test]
fn submit_cli_agent_rich_input_clears_draft() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let _cli_rich = FeatureFlag::CLIAgentRichInput.override_enabled(true);
        AISettings::handle(&app).update(&mut app, |settings, ctx| {
            // Keep the input open after submit so we can inspect the buffer.
            let _ = settings
                .auto_dismiss_rich_input_after_submit
                .set_value(false, ctx);
        });

        let terminal = open_cli_agent_rich_input_for_agent(&mut app, CLIAgent::Claude);

        terminal.update(&mut app, |view, ctx| {
            view.submit_cli_agent_rich_input("hello agent".to_owned(), ctx);
            // Input stays open because auto-dismiss is off.
            assert!(view.has_active_cli_agent_input_session(ctx));
        });

        terminal.read(&app, |view, ctx| {
            let session = CLIAgentSessionsModel::as_ref(ctx)
                .session(view.view_id)
                .expect("session should exist");
            assert_eq!(
                session.draft_text, None,
                "draft should be cleared after submit"
            );
            assert!(
                view.input.as_ref(ctx).buffer_text(ctx).is_empty(),
                "buffer should be empty after submit"
            );
        });
    })
}

#[test]
fn close_cli_agent_rich_input_with_empty_buffer_stores_no_draft() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let _cli_rich = FeatureFlag::CLIAgentRichInput.override_enabled(true);

        let terminal = open_cli_agent_rich_input_for_agent(&mut app, CLIAgent::Claude);

        // Close immediately without typing anything.
        terminal.update(&mut app, |view, ctx| {
            view.close_cli_agent_rich_input(CLIAgentRichInputCloseReason::Manual, ctx);
            assert!(!view.has_active_cli_agent_input_session(ctx));
        });

        terminal.read(&app, |view, ctx| {
            let session = CLIAgentSessionsModel::as_ref(ctx)
                .session(view.view_id)
                .expect("session should exist");
            assert_eq!(
                session.draft_text, None,
                "no draft should be stored for empty buffer"
            );
        });
    })
}

#[test]
fn ctrl_c_does_not_accept_prompt_suggestion_banner() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);

        let block_id = terminal.update(&mut app, |view, _ctx| {
            let mut model = view.model.lock();
            model.simulate_block("ls", "output");
            let last_completed_block_index = BlockIndex(model.block_list().blocks().len() - 2);
            model
                .block_list()
                .block_at(last_completed_block_index)
                .unwrap()
                .id()
                .clone()
        });

        terminal.update(&mut app, |view, ctx| {
            view.on_legacy_prompt_suggestion_generated(
                AgentModePromptSuggestion::Success(PromptSuggestion {
                    id: "suggestion".to_owned(),
                    label: Some("Do something".to_owned()),
                    prompt: "Do something".to_owned(),
                    coding_query_context: None,
                    static_prompt_suggestion_name: None,
                    should_start_new_conversation: false,
                }),
                block_id.clone(),
                "ls".to_owned(),
                0,
                ctx,
            );

            assert!(view
                .inline_banners_state
                .prompt_suggestions_banner
                .is_some());

            // Ctrl-C should not accept the prompt suggestion.
            view.handle_action(&TerminalAction::CtrlC, ctx);

            assert!(view
                .inline_banners_state
                .prompt_suggestions_banner
                .is_some());
        });
    })
}

/// Regression test for GH703: a Linear deeplink prompt must never be auto-submitted
/// to the LLM. Because `LinearDeepLink` returns `AutoTriggerBehavior::Never`, the
/// prompt must land in the input buffer as a draft and the "press enter again to
/// send" ephemeral message must be shown so the user can inspect and explicitly
/// send it.
#[test]
fn linear_deeplink_populates_input_as_draft_when_not_in_agent_view() {
    use super::agent_view::ENTER_AGAIN_TO_SEND_MESSAGE_ID;

    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        FeatureFlag::AgentView.set_enabled(true);

        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |view, ctx| {
            view.enter_agent_view_for_new_conversation(
                Some("attacker prompt".to_owned()),
                AgentViewEntryOrigin::LinearDeepLink,
                ctx,
            );
        });

        terminal.read(&app, |view, ctx| {
            assert!(
                view.agent_view_controller().as_ref(ctx).is_active(),
                "Linear deeplink should enter agent view"
            );
            assert_eq!(
                view.input.as_ref(ctx).buffer_text(ctx),
                "attacker prompt",
                "Linear deeplink prompt should be placed in the input buffer as a draft"
            );
            let ephemeral_message_id = view
                .ephemeral_message_model
                .as_ref(ctx)
                .current_message()
                .and_then(|msg| msg.id().map(|id| id.to_owned()));
            assert_eq!(
                ephemeral_message_id.as_deref(),
                Some(ENTER_AGAIN_TO_SEND_MESSAGE_ID),
                "the 'enter again to send' affordance should be shown"
            );
        });
    })
}

/// The critical regression guard for GH703: even when the user is already in
/// fullscreen agent view, a Linear deeplink prompt must not be auto-submitted to
/// the LLM. `LinearDeepLink` returns `AutoTriggerBehavior::Never`, so even the
/// `was_in_agent_view_already` shortcut cannot promote it to auto-submit.
#[test]
fn linear_deeplink_does_not_auto_submit_when_already_in_agent_view() {
    use super::agent_view::ENTER_AGAIN_TO_SEND_MESSAGE_ID;

    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        FeatureFlag::AgentView.set_enabled(true);

        let terminal = add_window_with_terminal(&mut app, None);

        // First enter fullscreen agent view with no initial prompt. This matches the
        // pre-condition in the issue: the focused terminal is already in fullscreen
        // agent view when the `warp://linear/work?prompt=...` URI is dispatched.
        let original_conversation_id = terminal.update(&mut app, |view, ctx| {
            view.agent_view_controller().update(ctx, |controller, ctx| {
                controller
                    .try_enter_agent_view(
                        None,
                        AgentViewEntryOrigin::Input {
                            was_prompt_autodetected: false,
                        },
                        ctx,
                    )
                    .expect("Should be able to enter agent view")
            })
        });

        terminal.read(&app, |view, ctx| {
            assert!(view
                .agent_view_controller()
                .as_ref(ctx)
                .agent_view_state()
                .is_fullscreen());
        });

        // Now dispatch the Linear deeplink while already in fullscreen agent view.
        terminal.update(&mut app, |view, ctx| {
            view.enter_agent_view_for_new_conversation(
                Some("attacker prompt".to_owned()),
                AgentViewEntryOrigin::LinearDeepLink,
                ctx,
            );
        });

        terminal.read(&app, |view, ctx| {
            // A new conversation should have been created for the Linear deeplink.
            let new_conversation_id = view
                .agent_view_controller()
                .as_ref(ctx)
                .agent_view_state()
                .active_conversation_id()
                .expect("agent view should still be active after Linear deeplink entry");
            assert_ne!(
                new_conversation_id, original_conversation_id,
                "Linear deeplink should open a new conversation"
            );

            // The prompt must be a draft, not auto-submitted.
            assert_eq!(
                view.input.as_ref(ctx).buffer_text(ctx),
                "attacker prompt",
                "Linear deeplink prompt must stay as a draft in the input buffer"
            );
            let ephemeral_message_id = view
                .ephemeral_message_model
                .as_ref(ctx)
                .current_message()
                .and_then(|msg| msg.id().map(|id| id.to_owned()));
            assert_eq!(
                ephemeral_message_id.as_deref(),
                Some(ENTER_AGAIN_TO_SEND_MESSAGE_ID),
                "the 'enter again to send' affordance must be shown so the user can \
                 consciously send the Linear-originated prompt"
            );
        });
    })
}

/// `LinearDeepLink` returns `AutoTriggerBehavior::Never`, so it must not
/// auto-submit regardless of prior agent-view state.
#[test]
fn linear_deeplink_via_default_entrypoint_does_not_auto_submit_in_fullscreen() {
    use super::agent_view::ENTER_AGAIN_TO_SEND_MESSAGE_ID;

    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        FeatureFlag::AgentView.set_enabled(true);

        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |view, ctx| {
            view.agent_view_controller().update(ctx, |controller, ctx| {
                controller
                    .try_enter_agent_view(
                        None,
                        AgentViewEntryOrigin::Input {
                            was_prompt_autodetected: false,
                        },
                        ctx,
                    )
                    .expect("Should be able to enter agent view")
            });
        });

        terminal.update(&mut app, |view, ctx| {
            view.enter_agent_view_for_new_conversation(
                Some("attacker prompt".to_owned()),
                AgentViewEntryOrigin::LinearDeepLink,
                ctx,
            );
        });

        terminal.read(&app, |view, ctx| {
            assert_eq!(
                view.input.as_ref(ctx).buffer_text(ctx),
                "attacker prompt",
                "Linear deeplink prompt must not be auto-submitted"
            );
            let ephemeral_message_id = view
                .ephemeral_message_model
                .as_ref(ctx)
                .current_message()
                .and_then(|msg| msg.id().map(|id| id.to_owned()));
            assert_eq!(
                ephemeral_message_id.as_deref(),
                Some(ENTER_AGAIN_TO_SEND_MESSAGE_ID),
            );
        });
    })
}

/// Regression test for https://github.com/warpdotdev/warp/issues/11212.
///
/// Closing the find bar must immediately clear find highlights on AI blocks.
/// AI blocks are separate child views, so unless `close_find_bar` clears find
/// state they keep stale highlights until the pane is refocused.
#[test]
fn close_find_bar_clears_ai_block_find_highlights() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let terminal = add_window_with_terminal(&mut app, None);

        // Create an AI block whose user query contains a searchable term.
        terminal.update(&mut app, |view, ctx| {
            append_exchange_and_handle_event(
                view,
                AIAgentInput::UserQuery {
                    query: "highlight the needle here".to_owned(),
                    context: Default::default(),
                    static_query_type: None,
                    referenced_attachments: Default::default(),
                    user_query_mode: UserQueryMode::Normal,
                    running_command: None,
                    intended_agent: None,
                },
                ctx,
            );
        });

        let ai_block = terminal.read(&app, |view, _| {
            view.rich_content_views
                .iter()
                .find_map(|rich_content| {
                    rich_content
                        .ai_block_metadata()
                        .map(|metadata| metadata.ai_block_handle.clone())
                })
                .expect("an AI block should have been inserted")
        });

        // Open the find bar and run find for a term that matches the AI block.
        // The blocklist find pipeline only visits rich-content views that have
        // been laid out, which does not happen in this headless test, so also
        // drive the AI block's `run_find` directly to put it in the
        // highlighted state.
        let find_options = || FindOptions {
            query: Some("needle".to_owned().into()),
            ..Default::default()
        };
        terminal.update(&mut app, |view, ctx| {
            view.show_find_bar(ctx);
            view.run_find(find_options(), ctx);
        });
        ai_block.update(&mut app, |block, ctx| {
            crate::terminal::find::FindableRichContentView::run_find(block, &find_options(), ctx);
        });

        assert_eq!(
            ai_block.read(&app, |block, _| block.find_match_count()),
            1,
            "running find should highlight the match inside the AI block"
        );

        // Dismissing the find bar must clear the AI block's find highlights.
        terminal.update(&mut app, |view, ctx| {
            view.close_find_bar(ctx);
        });

        assert_eq!(
            ai_block.read(&app, |block, _| block.find_match_count()),
            0,
            "closing the find bar should immediately clear AI block find highlights"
        );
    })
}

/// Regression test for the async-find branch of #11212.
///
/// Closing the find bar must clear stale AI block highlights without dropping
/// the saved query options on the async-find path. `open_find_bar` reads
/// `active_find_options` to restore the previous query; if `close_find_bar`
/// routes through `clear_matches → AsyncFindController::clear_results`, that
/// helper resets `current_find_options` and reopening the find bar starts
/// from a blank query instead of the previous one.
#[test]
fn close_find_bar_preserves_options_on_async_find_path() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _async_find = FeatureFlag::AsyncFind.override_enabled(true);
        let terminal = add_window_with_terminal(&mut app, None);

        let needle_options = || FindOptions {
            query: Some("needle".to_owned().into()),
            ..Default::default()
        };

        terminal.update(&mut app, |view, ctx| {
            view.show_find_bar(ctx);
            view.run_find(needle_options(), ctx);
        });

        // The async controller should have saved the active query.
        assert_eq!(
            terminal.read(&app, |view, ctx| view
                .find_model
                .as_ref(ctx)
                .active_find_options()
                .map(|o| o.query.clone())),
            Some(needle_options().query),
            "running find on the async path must save the active query"
        );

        // Closing the find bar must NOT drop the saved query — otherwise
        // the next `open_find_bar` would start blank instead of restoring
        // the previous search.
        terminal.update(&mut app, |view, ctx| {
            view.close_find_bar(ctx);
        });

        assert_eq!(
            terminal.read(&app, |view, ctx| view
                .find_model
                .as_ref(ctx)
                .active_find_options()
                .map(|o| o.query.clone())),
            Some(needle_options().query),
            "closing the find bar must preserve the saved query on the async path"
        );
    })
}

#[test]
fn cmd_k_does_not_clear_buffer_when_agent_is_driving_command() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |view, ctx| {
            bootstrap_with_long_running_block(view);

            let conversation_id =
                BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                    history.start_new_conversation(view.view_id, false, false, false, ctx)
                });
            set_active_block_agent_driving(view, conversation_id);

            assert!(view
                .model
                .lock()
                .block_list()
                .active_block()
                .is_agent_driving_command());
            assert!(!view
                .model
                .lock()
                .block_list()
                .active_block()
                .is_agent_monitoring());

            let block_count_before = view.model.lock().block_list().blocks().len();

            view.clear_buffer_for_testing(ctx);

            assert_eq!(
                view.model.lock().block_list().blocks().len(),
                block_count_before,
                "cmd-k must not wipe blocks while the agent is driving a command"
            );
        });
    })
}

#[test]
fn cmd_k_in_agent_view_clears_active_block_not_full_buffer_when_agent_driving_command() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        FeatureFlag::AgentView.set_enabled(true);

        let terminal = add_window_with_terminal(&mut app, None);

        let conversation_id = terminal.update(&mut app, |view, ctx| {
            let conversation_id = view.agent_view_controller().update(ctx, |controller, ctx| {
                controller
                    .try_enter_agent_view(
                        None,
                        AgentViewEntryOrigin::Input {
                            was_prompt_autodetected: false,
                        },
                        ctx,
                    )
                    .expect("should enter agent view")
            });

            bootstrap_with_long_running_block(view);
            set_active_block_agent_driving(view, conversation_id);

            assert!(view
                .agent_view_controller()
                .as_ref(ctx)
                .agent_view_state()
                .is_fullscreen());
            assert!(view
                .model
                .lock()
                .block_list()
                .active_block()
                .is_agent_driving_command());

            conversation_id
        });

        let block_count_before = terminal.read(&app, |view, _| {
            view.model.lock().block_list().blocks().len()
        });

        terminal.update(&mut app, |view, ctx| {
            view.clear_buffer_for_testing(ctx);
        });

        terminal.read(&app, |view, ctx| {
            // Same conversation still active: no new conversation was started.
            assert_eq!(
                view.agent_view_controller()
                    .as_ref(ctx)
                    .agent_view_state()
                    .active_conversation_id(),
                Some(conversation_id),
                "cmd-k must not start a new conversation while agent is driving a command"
            );
            // Block count unchanged: only the active block output was cleared.
            assert_eq!(
                view.model.lock().block_list().blocks().len(),
                block_count_before,
                "cmd-k must not remove blocks while agent is driving a command"
            );
        });
    })
}

#[test]
fn cmd_k_in_agent_view_cancels_in_progress_conversation_and_starts_new_one() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        FeatureFlag::AgentView.set_enabled(true);

        let terminal = add_window_with_terminal(&mut app, None);

        let old_conversation_id = terminal.update(&mut app, |view, ctx| {
            view.agent_view_controller().update(ctx, |controller, ctx| {
                controller
                    .try_enter_agent_view(
                        None,
                        AgentViewEntryOrigin::Input {
                            was_prompt_autodetected: false,
                        },
                        ctx,
                    )
                    .expect("should enter agent view")
            })
        });

        // New conversations always start InProgress.
        terminal.read(&app, |_, ctx| {
            assert_eq!(
                BlocklistAIHistoryModel::as_ref(ctx)
                    .conversation(&old_conversation_id)
                    .map(|c| c.status().clone()),
                Some(ConversationStatus::InProgress)
            );
        });

        // Attach a mock in-flight response stream so cancel_conversation_progress
        // can actually cancel it and flip the status to Cancelled.
        let stream_id = ResponseStreamId::new_for_test();
        terminal.update(&mut app, |view, ctx| {
            // Associate the stream_id with the conversation in the history model so
            // is_processing_response_stream returns true when the controller looks it up.
            BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                let exchange = exchange_with_inputs(vec![]);
                history
                    .conversation_mut(&old_conversation_id)
                    .expect("conversation should exist")
                    .append_reassigned_exchange(&stream_id, exchange, view.view_id, ctx)
                    .expect("exchange should append");
            });
            let stream = ctx.add_model(|_| ResponseStream::new_for_test(stream_id.clone()));
            view.ai_controller.update(ctx, |controller, ctx| {
                controller.register_mock_stream_for_test(
                    stream_id.clone(),
                    old_conversation_id,
                    stream,
                    ctx,
                );
            });
        });

        // Cmd+K with no long-running command: cancels the old in-progress conversation
        // (stream is cancelled → AfterStreamFinished → Cancelled status) and starts a new one.
        terminal.update(&mut app, |view, ctx| {
            view.clear_buffer_for_testing(ctx);
        });

        terminal.read(&app, |view, ctx| {
            // A new conversation must now be active.
            let new_conversation_id = view
                .agent_view_controller()
                .as_ref(ctx)
                .agent_view_state()
                .active_conversation_id()
                .expect("agent view should still be active after cmd-k");
            assert_ne!(
                new_conversation_id, old_conversation_id,
                "cmd-k must start a new conversation when an in-progress one is active"
            );

            // The old conversation must be Cancelled — the stream was actually cancelled.
            assert_eq!(
                BlocklistAIHistoryModel::as_ref(ctx)
                    .conversation(&old_conversation_id)
                    .map(|c| c.status().clone()),
                Some(ConversationStatus::Cancelled),
                "the old in-progress conversation must be Cancelled after cmd-k"
            );
        });
    })
}

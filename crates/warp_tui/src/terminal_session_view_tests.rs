use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use ai::LLMProvider;
use ai::api_keys::ApiKeyManager;
use chrono::NaiveDate;
use instant::Instant;
use string_offset::CharOffset;
use tempfile::TempDir;
use warp::appearance::Appearance;
#[cfg(feature = "voice_input")]
use warp::settings::TuiVoiceSettings;
use warp::settings::{
    AISettings, SettingsFileError, TuiStatuslineConfig, TuiStatuslineItem, TuiTheme,
    TuiThemeSettings, TuiUsageDisplayMode, TuiVoiceInputHoldKey, TuiZeroStateObject,
};
use warp::terminal::model::ansi::{Handler, InputBufferValue, Mode};
use warp::tui_export::{
    AIAgentAction, AIAgentActionId, AIAgentActionType, AIAgentExchangeId, AIAgentInput,
    AIAgentOutput, AIAgentOutputMessage, AIAgentOutputMessageType, AIAgentTodo, AIAgentTodoList,
    AIBlockModel, AIBlockOutputStatus, AIConversationAutoexecuteMode, AIConversationId,
    AIRequestType, AgentViewEntryOrigin, AskUserQuestionItem, AskUserQuestionOption,
    AskUserQuestionType, BlockPadding, BlocklistAIHistoryEvent, BlocklistAIHistoryModel,
    ConversationStatus, ConversationUsageTotals, Harness, InputTypeAutoDetectionSource, LLMId,
    LLMPreferences, LinkedWorkflowData, LongRunningCommandControlState, MessageId,
    OutputStatusUpdateCallback, ParsedSlashCommandInput, PtyIntent, PtyIntentEvent, ServerOutputId,
    Session, Shared, SizeInfo, SizeUpdate, SlashCommandDataSource as _, SlashCommandKind, TaskId,
    TranscriptScope, TuiMcpAction, TuiMcpServerId, TuiOnboardingMarker, TuiOnboardingMarkers,
    TuiUpArrowHistoryItemKind, UserTakeOverReason, WarpConfig, WarpConfigUpdateEvent,
    export_conversation_markdown, forkable_tui_conversation_for_test, queue_tui_permission_action,
    register_tui_session_view_test_singletons, set_tui_default_team_admin_for_test, slash_commands,
};
use warp_core::channel::{Channel, ChannelState};
use warp_core::features::FeatureFlag;
use warp_core::settings::Setting as _;
use warp_editor::model::CoreEditorModel;
use warpui::platform::WindowStyle;
use warpui::{
    AddWindowOptions, EntityIdMap, ModelHandle, ReadModel, SingletonEntity, UpdateModel, ViewHandle,
};
use warpui_core::r#async::Timer;
use warpui_core::elements::tui::{
    Color, TuiBuffer, TuiBufferExt, TuiConstrainedBox, TuiConstraint, TuiContainer, TuiElement,
    TuiEvent, TuiEventContext, TuiFlex, TuiLayoutContext, TuiPaintContext, TuiPaintSurface,
    TuiPoint, TuiRect, TuiScene, TuiScreenPosition, TuiSize, TuiStyle, TuiText,
    TuiViewportPosition,
};
#[cfg(feature = "voice_input")]
use warpui_core::event::KeyState;
use warpui_core::event::ModifiersState;
use warpui_core::keymap::{Context, DescriptionContext, Keystroke, Trigger};
use warpui_core::platform::keyboard::KeyCode;
use warpui_core::presenter::tui::{TuiFrame, TuiPresenter};
use warpui_core::telemetry::{EventPayload, flush_events};
use warpui_core::{App, AppContext, TuiView, TypedActionView, ViewContext, WindowInvalidation};

use super::statusline::{
    ContextWindowUsage, FooterSegment, FooterSegments, format_context_window_usage,
    format_statusline_date, format_statusline_time_12_hour, format_statusline_time_24_hour,
    format_todo_progress, render_git_branch_status, render_status_footer_row,
    render_statusline_datetime, should_render_plain_git_branch,
};
use super::{
    ACCEPT_BLOCKED_TERMINAL_USE_ACTION_BINDING_NAME, ATTACH_AGENT_TO_RUNNING_COMMAND_BINDING_NAME,
    AUTO_APPROVE_DISABLED_HINT, AUTO_APPROVE_ENABLED_HINT, AUTO_APPROVE_FEEDBACK_DURATION,
    AUTO_APPROVE_TOGGLE_BINDING_NAME, BlockingInputSource, COST_CONVERSATION_IN_PROGRESS_HINT,
    COST_EMPTY_CONVERSATION_HINT, COST_NO_ACTIVE_CONVERSATION_HINT, CTRL_C_EXIT_HINT,
    CTRL_C_KILL_CHILD_HINT, ConversationRestoreState,
    DETACH_AGENT_FROM_RUNNING_COMMAND_BINDING_NAME, INLINE_MENU_TOP_PADDING_ROWS,
    LOADING_CONVERSATION_HINT, LOG_BUNDLE_FAILED_HINT, RUNNING_COMMAND_DETACH_HINT,
    SESSION_CAN_ACCEPT_BLOCKED_TERMINAL_USE_ACTION_FLAG,
    SESSION_CAN_ATTACH_AGENT_TO_RUNNING_COMMAND_FLAG,
    SESSION_CAN_DETACH_AGENT_FROM_RUNNING_COMMAND_FLAG, SHELL_MODE_HINT, STATUSLINE_RESET_HINT,
    TuiConversationRestoreOrigin, TuiTerminalSessionAction, TuiTerminalSessionEvent,
    TuiTerminalSessionView, attachment_focus_available, cost_command_unavailable_hint,
    export_file_success_message, log_bundle_success_message, mcp_primary_action_hint,
    raw_prompt_if_not_blank, render_mcp_install_footer, render_mcp_menu_footer,
};
#[cfg(feature = "voice_input")]
use super::{
    SESSION_COMPOSER_SHORTCUTS_ACTIVE_FLAG, VOICE_INPUT_BINDING_NAME, VOICE_USAGE_HINT,
    voice_argument_is_empty, voice_command_argument,
};
use crate::agent_block::{TuiAIBlock, upgrade_url};
use crate::autoupdate::TuiAutoupdater;
use crate::editor_element::TuiEditorAction;
use crate::inline_menu::MAX_INLINE_MENU_ROWS;
use crate::input::view::TuiInputAction;
use crate::input_mode_policy::{AI_LOCKED_CONFIG, AI_UNLOCKED_CONFIG};
use crate::input_suggestions_mode::TuiInputSuggestionsMode;
use crate::keybindings::{
    CONTEXTUAL_PLAN_TOGGLE_BINDING_NAME, KEYBOARD_ENHANCEMENT_AVAILABLE_FLAG,
    PLAN_TOGGLE_AVAILABLE_FLAG, PLAN_TOGGLE_BINDING_NAME, TUI_BINDING_GROUP,
};
use crate::orchestrated_agent_identity_styling::AgentIdentity;
use crate::orchestration_model::TuiOrchestrationModel;
use crate::orchestration_tab_bar::{
    ORCHESTRATION_TAB_BAR_FOCUSED_FLAG, orchestration_tab_icon,
    render_orchestration_child_selected_tab_footer, render_orchestration_tab_footer,
};
use crate::read_only_menu::TuiReadOnlyMenuKind;
use crate::root_view::RootTuiView;
use crate::session_registry::{TuiSessionId, TuiSessions};
use crate::statusline_config_view::TuiStatuslineConfigEvent;
use crate::telemetry::TuiConversationRestoreTelemetryTarget;
use crate::terminal_block::{block_content_rows, should_render_terminal_block};
use crate::terminal_use::TuiInputTarget;
use crate::test_fixtures::{
    add_test_semantic_selection, add_test_terminal_session,
    add_test_terminal_session_with_first_run_onboarding,
    add_test_terminal_session_with_settings_file_error,
};
use crate::transcript_view::TRANSCRIPT_BLOCK_SPACING;
use crate::transient_hint::TransientHintTone;
use crate::tui_builder::TuiUiBuilder;
use crate::usage::UsageToggle;
#[cfg(feature = "voice_input")]
use crate::voice_input::{TuiVoiceInputState, requires_modifier_key_reporting};
use crate::zero_state_animation::{
    ZeroStateAnimationConfig, ZeroStateAnimationConfigEvent, ZeroStateAnimationLoadFailure,
};

struct FocusTestFixture {
    window_id: warpui_core::WindowId,
    sessions: ModelHandle<TuiSessions>,
}

#[test]
fn only_conversation_list_restores_emit_restore_telemetry() {
    assert!(!TuiConversationRestoreOrigin::Startup.records_telemetry());
    assert!(TuiConversationRestoreOrigin::ConversationList.records_telemetry());
    assert!(!TuiConversationRestoreOrigin::Fork.records_telemetry());
}

#[test]
fn mcp_install_footer_labels_final_value_as_install_and_enable() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            ctx.add_singleton_model(|_| Appearance::mock());
            let footer = render_mcp_install_footer(
                &TuiUiBuilder::from_app(ctx),
                Some("to install and enable"),
            )
            .finish();
            assert_eq!(
                render_element(footer, ctx, 120).to_lines(),
                vec!["Enter to install and enable  Esc to cancel".to_owned()],
            );
        });
    });
}

fn todo(id: &str, title: &str) -> AIAgentTodo {
    AIAgentTodo::new(id.to_owned().into(), title.to_owned(), String::new())
}

fn set_selected_todo_list(
    app: &mut App,
    view: &ViewHandle<TuiTerminalSessionView>,
    completed: Vec<AIAgentTodo>,
    pending: Vec<AIAgentTodo>,
    status: ConversationStatus,
) -> AIConversationId {
    view.update(app, |view, ctx| {
        let conversation_id = view.conversation_selection.update(ctx, |selection, ctx| {
            selection
                .try_start_new_conversation(AgentViewEntryOrigin::Tui, ctx)
                .expect("test conversation should start")
        });
        let terminal_surface_id = view.terminal_surface_id;
        BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
            let conversation = history
                .conversation_mut(&conversation_id)
                .expect("selected conversation should exist");
            conversation.set_todo_lists_for_test(vec![
                AIAgentTodoList::default()
                    .with_completed_items(completed)
                    .with_pending_items(pending),
            ]);
            conversation.update_status(status, terminal_surface_id, ctx);
        });
        conversation_id
    })
}
#[test]
fn mcp_menu_footer_replaces_status_with_controls() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            ctx.add_singleton_model(|_| Appearance::mock());
            let footer = render_mcp_menu_footer(
                &TuiUiBuilder::from_app(ctx),
                Some(TuiMcpAction::Stop(TuiMcpServerId::FileBased(1))),
                true,
            )
            .finish();
            assert_eq!(
                render_element(footer, ctx, 120).to_lines(),
                vec![
                    "Enter to stop  Ctrl+R to log out & remove credentials  Esc to close"
                        .to_owned()
                ],
            );
        });
    });
}

#[test]
fn out_of_credits_ctrl_o_binding_opens_upgrade() {
    App::test((), |mut app| async move {
        app.update(crate::keybindings::init);
        app.read(|ctx| {
            let ctrl_o = Trigger::Keystrokes(vec![Keystroke::parse("ctrl-o").unwrap()]);
            assert!(
                ctx.get_key_bindings().any(|binding| {
                    *binding.trigger == ctrl_o
                        && binding.name.is_empty()
                        && binding.group == Some(TUI_BINDING_GROUP)
                }),
                "out-of-credits ctrl-o binding should be registered"
            );
        });

        let fixture = focus_test_fixture(&mut app);
        let (view, _) = add_focus_test_session(&mut app, &fixture, true);
        let expected_upgrade_url = app.read(upgrade_url);
        app.read(|ctx| {
            let ctrl_o = Trigger::Keystrokes(vec![Keystroke::parse("ctrl-o").unwrap()]);
            let input_view_id = view.as_ref(ctx).input_view.id();
            assert!(
                !ctx.key_bindings_for_view(fixture.window_id, input_view_id)
                    .iter()
                    .any(|binding| *binding.trigger == ctrl_o),
                "ctrl-o should not be active without an out-of-credits failure"
            );
        });
        let opened_urls = Rc::new(RefCell::new(Vec::new()));
        let opened_urls_for_callback = opened_urls.clone();
        app.update(|ctx| {
            ctx.set_before_open_url(move |url, _| {
                opened_urls_for_callback.borrow_mut().push(url.to_owned());
                url.to_owned()
            });
        });
        view.update(&mut app, |view, ctx| {
            view.handle_action(&TuiTerminalSessionAction::OpenUpgradeUrl, ctx);
        });
        assert_eq!(opened_urls.borrow().as_slice(), &[expected_upgrade_url]);
    });
}

#[test]
fn usage_slash_command_opens_panel_and_enables_upgrade_binding() {
    App::test((), |mut app| async move {
        app.update(crate::keybindings::init);
        let fixture = focus_test_fixture(&mut app);
        let (view, _) = add_focus_test_session(&mut app, &fixture, true);

        app.read(|ctx| {
            let ctrl_o = Trigger::Keystrokes(vec![Keystroke::parse("ctrl-o").unwrap()]);
            let input_view_id = view.as_ref(ctx).input_view.id();
            assert!(
                !ctx.key_bindings_for_view(fixture.window_id, input_view_id)
                    .iter()
                    .any(|binding| *binding.trigger == ctrl_o),
                "ctrl-o should not be active before the /usage panel is opened"
            );
        });

        view.update(&mut app, |view, ctx| {
            view.execute_tui_slash_command(&slash_commands::USAGE, None, ctx);
        });
        view.read(&app, |view, ctx| {
            assert_eq!(
                view.suggestions_mode.as_ref(ctx).mode(),
                TuiInputSuggestionsMode::ReadOnlyMenu(TuiReadOnlyMenuKind::Usage)
            );
        });

        app.read(|ctx| {
            let ctrl_o = Trigger::Keystrokes(vec![Keystroke::parse("ctrl-o").unwrap()]);
            let input_view_id = view.as_ref(ctx).input_view.id();
            assert!(
                ctx.key_bindings_for_view(fixture.window_id, input_view_id)
                    .iter()
                    .any(|binding| *binding.trigger == ctrl_o),
                "ctrl-o should open the upgrade page while the /usage panel is open"
            );
        });
    });
}

#[test]
fn upgrade_slash_command_is_always_available_and_opens_the_upgrade_page() {
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        let (view, _) = add_focus_test_session(&mut app, &fixture, true);
        let expected_upgrade_url = app.read(upgrade_url);
        let opened_urls = Rc::new(RefCell::new(Vec::new()));
        let opened_urls_for_callback = opened_urls.clone();
        app.update(|ctx| {
            ctx.set_before_open_url(move |url, _| {
                opened_urls_for_callback.borrow_mut().push(url.to_owned());
                url.to_owned()
            });
        });

        view.update(&mut app, |view, ctx| {
            view.input_view
                .update(ctx, |input, ctx| input.set_text("/upgrade", ctx));
            assert!(matches!(
                view.slash_commands_source
                    .as_ref(ctx)
                    .parse_input("/upgrade", ctx),
                ParsedSlashCommandInput::SlashCommand(_)
            ));
            view.execute_tui_slash_command(&slash_commands::UPGRADE, None, ctx);
        });

        assert_eq!(opened_urls.borrow().as_slice(), &[expected_upgrade_url]);
        view.read(&app, |view, ctx| {
            assert!(view.input_view.as_ref(ctx).is_empty(ctx));
        });
    });
}
#[test]
fn manage_billing_slash_command_opens_the_default_team_billing_page_for_admins() {
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        let (view, _) = add_focus_test_session(&mut app, &fixture, true);
        view.read(&app, |view, ctx| {
            assert!(!matches!(
                view.slash_commands_source
                    .as_ref(ctx)
                    .parse_input("/manage-billing", ctx),
                ParsedSlashCommandInput::SlashCommand(_)
            ));
        });
        app.update(set_tui_default_team_admin_for_test);
        let opened_urls = Rc::new(RefCell::new(Vec::new()));
        let opened_urls_for_callback = opened_urls.clone();
        app.update(|ctx| {
            ctx.set_before_open_url(move |url, _| {
                opened_urls_for_callback.borrow_mut().push(url.to_owned());
                url.to_owned()
            });
        });

        view.update(&mut app, |view, ctx| {
            assert!(matches!(
                view.slash_commands_source
                    .as_ref(ctx)
                    .parse_input("/manage-billing", ctx),
                ParsedSlashCommandInput::SlashCommand(_)
            ));
            view.execute_tui_slash_command(&slash_commands::MANAGE_BILLING, None, ctx);
        });

        assert_eq!(
            opened_urls.borrow().as_slice(),
            &[format!(
                "{}/admin/test_uid00000000000123/billing",
                ChannelState::server_root_url().trim_end_matches('/')
            )]
        );
    });
}

#[test]
fn manage_billing_slash_command_rejects_users_without_an_admin_team() {
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        let (view, _) = add_focus_test_session(&mut app, &fixture, true);
        let opened_urls = Rc::new(RefCell::new(Vec::new()));
        let opened_urls_for_callback = opened_urls.clone();
        app.update(|ctx| {
            ctx.set_before_open_url(move |url, _| {
                opened_urls_for_callback.borrow_mut().push(url.to_owned());
                url.to_owned()
            });
        });

        view.update(&mut app, |view, ctx| {
            assert!(!matches!(
                view.slash_commands_source
                    .as_ref(ctx)
                    .parse_input("/manage-billing", ctx),
                ParsedSlashCommandInput::SlashCommand(_)
            ));
            view.execute_tui_slash_command(&slash_commands::MANAGE_BILLING, None, ctx);
        });

        assert!(opened_urls.borrow().is_empty());
        assert_eq!(
            view.read(&app, |view, _| {
                view.transient_hint
                    .current()
                    .map(|(text, _)| text.to_owned())
            }),
            Some("Billing management is only available to team admins".to_owned())
        );
    });
}

#[test]
fn mcp_menu_footer_hides_unavailable_primary_control() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            ctx.add_singleton_model(|_| Appearance::mock());
            let builder = TuiUiBuilder::from_app(ctx);
            let logout_only = render_mcp_menu_footer(&builder, None, true).finish();
            assert_eq!(
                render_element(logout_only, ctx, 120).to_lines(),
                vec!["Ctrl+R to log out & remove credentials  Esc to close".to_owned()],
            );
            let close_only = render_mcp_menu_footer(&builder, None, false).finish();
            assert_eq!(
                render_element(close_only, ctx, 120).to_lines(),
                vec!["Esc to close".to_owned()],
            );
        });
    });
}

#[test]
fn api_keys_slash_command_opens_inline_and_clears_the_input() {
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        let (view, _) = add_focus_test_session(&mut app, &fixture, true);
        view.update(&mut app, |view, ctx| {
            view.input_view
                .update(ctx, |input, ctx| input.set_text("/api-keys", ctx));
            view.execute_tui_slash_command(&slash_commands::API_KEYS, None, ctx);
        });

        view.read(&app, |view, ctx| {
            assert!(view.api_keys_menu.as_ref(ctx).is_open(ctx));
            assert_eq!(
                view.suggestions_mode.as_ref(ctx).mode(),
                TuiInputSuggestionsMode::ApiKeys
            );
            assert!(view.input_view.as_ref(ctx).is_empty(ctx));
        });
        let rendered = render_session(&mut app, &view, 100, 40).join("\n");
        assert!(rendered.contains("API keys"), "{rendered}");
        assert!(rendered.contains("Anthropic API key"), "{rendered}");
        assert!(rendered.contains("enter to set api key"), "{rendered}");
        assert!(!rendered.contains("ctrl + x"), "{rendered}");
        assert!(!rendered.contains("/api-keys"), "{rendered}");
    });
}

#[test]
fn connect_grok_slash_command_opens_the_api_keys_menu_in_grok_flow() {
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        let (view, _) = add_focus_test_session(&mut app, &fixture, true);
        view.update(&mut app, |view, ctx| {
            view.input_view
                .update(ctx, |input, ctx| input.set_text("/connect-grok", ctx));
            view.execute_tui_slash_command(&slash_commands::CONNECT_GROK, None, ctx);
        });

        view.read(&app, |view, ctx| {
            assert!(view.api_keys_menu.as_ref(ctx).is_open(ctx));
            assert_eq!(
                view.suggestions_mode.as_ref(ctx).mode(),
                TuiInputSuggestionsMode::ApiKeys
            );
            assert!(view.input_view.as_ref(ctx).is_empty(ctx));
        });
    });
}

#[test]
fn mcp_primary_action_hints_match_available_actions() {
    let id = TuiMcpServerId::FileBased(1);
    assert_eq!(
        mcp_primary_action_hint(TuiMcpAction::Enable(id)),
        Some("to install and enable")
    );
    assert_eq!(
        mcp_primary_action_hint(TuiMcpAction::Start(id)),
        Some("to start")
    );
    assert_eq!(
        mcp_primary_action_hint(TuiMcpAction::Stop(id)),
        Some("to stop")
    );
    assert_eq!(
        mcp_primary_action_hint(TuiMcpAction::Retry(id)),
        Some("to retry")
    );
    assert_eq!(
        mcp_primary_action_hint(TuiMcpAction::ReopenAuthorization(id)),
        Some("to authenticate")
    );
    assert_eq!(mcp_primary_action_hint(TuiMcpAction::LogOut(id)), None);
    assert_eq!(mcp_primary_action_hint(TuiMcpAction::ReloadConfig), None);
}
#[test]
fn mcp_menu_footer_hides_unavailable_logout_control() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            ctx.add_singleton_model(|_| Appearance::mock());
            let footer = render_mcp_menu_footer(
                &TuiUiBuilder::from_app(ctx),
                Some(TuiMcpAction::Start(TuiMcpServerId::FileBased(1))),
                false,
            )
            .finish();
            assert_eq!(
                render_element(footer, ctx, 120).to_lines(),
                vec!["Enter to start  Esc to close".to_owned()],
            );
        });
    });
}

#[test]
fn ctrl_x_clears_the_selected_api_key_through_the_real_keymap() {
    App::test((), |mut app| async move {
        app.update(crate::keybindings::init);
        let fixture = focus_test_fixture(&mut app);
        let (view, _) = add_focus_test_session(&mut app, &fixture, true);
        ApiKeyManager::handle(&app)
            .update(&mut app, |manager, ctx| {
                manager.persist_provider_key(
                    LLMProvider::Anthropic,
                    Some("test-secret".to_owned()),
                    ctx,
                )
            })
            .unwrap();
        view.update(&mut app, |view, ctx| {
            view.execute_tui_slash_command(&slash_commands::API_KEYS, None, ctx);
            ctx.focus(&view.input_view);
        });
        let before_clear = render_session(&mut app, &view, 100, 40).join("\n");
        assert!(before_clear.contains("ctrl + x"), "{before_clear}");

        let (window_id, responder_chain) = app.read(|ctx| {
            let window_id = view.window_id(ctx);
            let focused = ctx.focused_view_id(window_id).unwrap();
            assert_eq!(focused, view.as_ref(ctx).input_view.id());
            (window_id, ctx.view_ancestors(window_id, focused))
        });
        let handled = app
            .dispatch_keystroke(
                window_id,
                &responder_chain,
                &Keystroke::parse("ctrl-x").unwrap(),
                false,
            )
            .unwrap();

        assert!(handled);
        app.read(|ctx| {
            assert_eq!(ApiKeyManager::as_ref(ctx).keys().anthropic, None);
            assert!(view.as_ref(ctx).api_keys_menu.as_ref(ctx).is_open(ctx));
        });
        let after_clear = render_session(&mut app, &view, 100, 40).join("\n");
        assert!(!after_clear.contains("ctrl + x"), "{after_clear}");
    });
}

#[test]
fn figma_statusline_metadata_formats_are_stable() {
    let now = NaiveDate::from_ymd_opt(2026, 7, 20)
        .unwrap()
        .and_hms_opt(13, 8, 0)
        .unwrap();
    assert_eq!(format_statusline_date(now), "July 20, 2026");
    assert_eq!(format_statusline_time_12_hour(now), "1:08pm");
    assert_eq!(format_statusline_time_24_hour(now), "13:08");
    assert_eq!(format_todo_progress(1, 10, false), "❒ 1/10");
    assert_eq!(format_todo_progress(10, 10, true), "✓ 10/10");
    assert_eq!(
        format_context_window_usage(0.0),
        ContextWindowUsage {
            bar: "████".to_owned(),
            percentage_remaining: 100,
            warning: false,
        }
    );
    assert_eq!(
        format_context_window_usage(0.25),
        ContextWindowUsage {
            bar: "███░".to_owned(),
            percentage_remaining: 75,
            warning: false,
        }
    );
    assert_eq!(
        format_context_window_usage(0.5),
        ContextWindowUsage {
            bar: "██░░".to_owned(),
            percentage_remaining: 50,
            warning: false,
        }
    );
    assert_eq!(
        format_context_window_usage(0.75),
        ContextWindowUsage {
            bar: "█░░░".to_owned(),
            percentage_remaining: 25,
            warning: true,
        }
    );
}

#[test]
fn statusline_datetime_requests_a_periodic_repaint() {
    App::test((), |app| async move {
        app.read(|ctx| {
            let datetime =
                render_statusline_datetime(format_statusline_time_24_hour, TuiStyle::default());
            let mut presenter = TuiPresenter::new();
            let frame = presenter.present_element(datetime, TuiRect::new(0, 0, 5, 1), ctx);
            assert!(
                frame.repaint_at.is_some(),
                "visible date/time items must repaint so their value cannot freeze"
            );
        });
    });
}
#[test]
fn footer_supports_arbitrary_order_and_figma_group_dividers() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            ctx.add_singleton_model(|_| Appearance::mock());
            let builder = TuiUiBuilder::from_app(ctx);
            let row = render_status_footer_row(
                FooterSegments {
                    ordered: vec![
                        FooterSegment::ContextWindowUsage(format_context_window_usage(0.426)),
                        FooterSegment::GitBranch("feature/statusline".to_owned()),
                        FooterSegment::AutoApproveIndicator(TuiText::new("▶▶").finish()),
                        FooterSegment::WorkingDirectory("/tmp/warp".to_owned()),
                        FooterSegment::DateTime(TuiText::new("July 20, 2026").finish()),
                    ],
                },
                &builder,
            )
            .finish();
            assert_eq!(
                render_element(row, ctx, 120).to_lines(),
                vec![
                    "██░░ 57% context remaining | ⊢ feature/statusline | ▶▶ | /tmp/warp | July 20, 2026"
                        .to_owned()
                ],
            );

            let branch_only = render_status_footer_row(
                FooterSegments {
                    ordered: vec![FooterSegment::GitBranch("main".to_owned())],
                },
                &builder,
            )
            .finish();
            assert_eq!(
                render_element(branch_only, ctx, 80).to_lines(),
                vec!["⊢ main".to_owned()],
            );
        });
    });
}

#[test]
#[cfg(feature = "voice_input")]
fn footer_uses_pipes_between_figma_groups_and_preserves_within_group_separators() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            ctx.add_singleton_model(|_| Appearance::mock());
            let builder = TuiUiBuilder::from_app(ctx);
            let row = render_status_footer_row(
                FooterSegments {
                    ordered: vec![
                        FooterSegment::AutoApproveIndicator(TuiText::new("▶▶").finish()),
                        FooterSegment::Model(TuiText::new("model").finish()),
                        FooterSegment::WorkingDirectory("/tmp/warp".to_owned()),
                        FooterSegment::GitBranch("main".to_owned()),
                        FooterSegment::GitBranchStatus(render_git_branch_status(
                            "main",
                            false,
                            Some("1".to_owned()),
                            Some("2".to_owned()),
                            &builder,
                        )),
                        FooterSegment::GitDiff {
                            files_changed: 6,
                            additions: 31,
                            deletions: 12,
                        },
                        FooterSegment::CreditUsage(TuiText::new("40 credits").finish()),
                        FooterSegment::GitHubPullRequest(TuiText::new("PR #123").finish()),
                        FooterSegment::ContextWindowUsage(format_context_window_usage(0.426)),
                        FooterSegment::DateTime(TuiText::new("July 20, 2026").finish()),
                        FooterSegment::DateTime(TuiText::new("1:08pm").finish()),
                        FooterSegment::AgentTodoList(TuiText::new("❒ 1/10").finish()),
                        FooterSegment::VoiceInput(TuiText::new("◉ Voice").finish()),
                    ],
                },
                &builder,
            )
            .finish();
            assert_eq!(
                render_element(row, ctx, 160).to_lines(),
                vec![
                    "▶▶ | model | /tmp/warp ⊢ main | ⊢ main • ↑1 ↓2 | ☰ 6 • +31 -12 | 40 credits | PR #123 | ██░░ 57% context remaining | July 20, 2026 • 1:08pm | ❒ 1/10 | ◉ Voice"
                        .to_owned()
                ],
            );
        });
    });
}

#[test]
fn git_diff_status_matches_figma_file_count_content_and_styles() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            ctx.add_singleton_model(|_| Appearance::mock());
            let builder = TuiUiBuilder::from_app(ctx);
            let row = render_status_footer_row(
                FooterSegments {
                    ordered: vec![FooterSegment::GitDiff {
                        files_changed: 6,
                        additions: 31,
                        deletions: 12,
                    }],
                },
                &builder,
            )
            .finish();
            let buffer = render_element(row, ctx, 80);
            assert_eq!(buffer.to_lines(), vec!["☰ 6 • +31 -12".to_owned()],);
            assert_eq!(
                buffer[(0, 0)].fg,
                builder
                    .muted_text_style()
                    .fg
                    .expect("file glyph should use the muted foreground"),
            );
            assert_eq!(
                buffer[(
                    (0..buffer.area().width)
                        .find(|column| buffer[(*column, 0)].symbol() == "+")
                        .expect("addition glyph should render"),
                    0,
                )]
                    .fg,
                builder
                    .diff_added_style()
                    .fg
                    .expect("addition count should use the added foreground"),
            );
            assert_eq!(
                buffer[(
                    (0..buffer.area().width)
                        .find(|column| buffer[(*column, 0)].symbol() == "-")
                        .expect("deletion glyph should render"),
                    0,
                )]
                    .fg,
                builder
                    .diff_removed_style()
                    .fg
                    .expect("deletion count should use the removed foreground"),
            );

            let file_only = render_status_footer_row(
                FooterSegments {
                    ordered: vec![FooterSegment::GitDiff {
                        files_changed: 1,
                        additions: 0,
                        deletions: 0,
                    }],
                },
                &builder,
            )
            .finish();
            assert_eq!(
                render_element(file_only, ctx, 80).to_lines(),
                vec!["☰ 1".to_owned()],
                "binary or zero-line changes should remain visible through their file count",
            );
        });
    });
}

#[test]
fn git_branch_status_matches_figma_content_styles_and_tracking_variants() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            ctx.add_singleton_model(|_| Appearance::mock());
            let builder = TuiUiBuilder::from_app(ctx);
            let status = render_element(
                render_git_branch_status(
                    "main",
                    false,
                    Some("1".to_owned()),
                    Some("2".to_owned()),
                    &builder,
                ),
                ctx,
                80,
            );
            assert_eq!(status.to_lines(), vec!["⊢ main • ↑1 ↓2".to_owned()]);
            let muted = builder
                .muted_text_style()
                .fg
                .expect("muted status text has a foreground");
            let accent = builder
                .accent_text_style()
                .fg
                .expect("branch status arrows have a foreground");
            assert_eq!(status[(0, 0)].fg, muted);
            assert_eq!(status[(9, 0)].fg, accent);
            assert_eq!(status[(10, 0)].fg, muted);
            assert_eq!(status[(12, 0)].fg, accent);
            assert_eq!(status[(13, 0)].fg, muted);

            assert_eq!(
                render_element(
                    render_git_branch_status("main", false, Some("1".to_owned()), None, &builder,),
                    ctx,
                    80,
                )
                .to_lines(),
                vec!["⊢ main • ↑1".to_owned()]
            );
            assert_eq!(
                render_element(
                    render_git_branch_status("main", false, None, Some("2".to_owned()), &builder,),
                    ctx,
                    80,
                )
                .to_lines(),
                vec!["⊢ main • ↓2".to_owned()]
            );
            assert_eq!(
                render_element(
                    render_git_branch_status("main", true, None, None, &builder),
                    ctx,
                    80,
                )
                .to_lines(),
                vec!["⊢ main • ⇅".to_owned()]
            );
            assert_eq!(
                render_element(
                    render_git_branch_status("main", false, None, None, &builder),
                    ctx,
                    80,
                )
                .to_lines(),
                vec!["⊢ main".to_owned()]
            );
        });
    });
}

#[test]
fn composite_git_branch_status_suppresses_the_plain_branch_item() {
    let branch_only = TuiStatuslineConfig {
        order: TuiStatuslineItem::ALL.to_vec(),
        enabled: vec![TuiStatuslineItem::GitBranch],
    };
    assert!(should_render_plain_git_branch(&branch_only));

    let branch_and_status = TuiStatuslineConfig {
        order: TuiStatuslineItem::ALL.to_vec(),
        enabled: vec![
            TuiStatuslineItem::GitBranch,
            TuiStatuslineItem::GitBranchStatus,
        ],
    };
    assert!(!should_render_plain_git_branch(&branch_and_status));
}
#[test]
fn empty_configurable_footer_has_zero_height() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            ctx.add_singleton_model(|_| Appearance::mock());
            let builder = TuiUiBuilder::from_app(ctx);
            let row = render_status_footer_row(
                FooterSegments {
                    ordered: Vec::new(),
                },
                &builder,
            )
            .finish();
            assert!(render_element(row, ctx, 80).to_lines().is_empty());
        });
    });
}

#[test]
fn enabled_auto_approve_indicator_is_always_visible_with_state_aware_color() {
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        let (view, _) = add_focus_test_session(&mut app, &fixture, true);
        view.update(&mut app, |view, ctx| {
            view.conversation_selection.update(ctx, |selection, ctx| {
                selection
                    .try_start_new_conversation(AgentViewEntryOrigin::Tui, ctx)
                    .expect("test conversation should start")
            });
            AISettings::handle(ctx).update(ctx, |settings, ctx| {
                settings
                    .tui_statusline
                    .set_value(
                        TuiStatuslineConfig {
                            order: vec![TuiStatuslineItem::AutoApprove],
                            enabled: vec![TuiStatuslineItem::AutoApprove],
                        }
                        .normalized(),
                        ctx,
                    )
                    .expect("statusline setting should persist");
            });
        });

        let disabled = render_footer(&mut app, &view, 80);
        assert_eq!(disabled.to_lines(), vec!["▶▶".to_owned()]);
        assert_eq!(
            disabled[(0, 0)].fg,
            app.read(|ctx| {
                TuiUiBuilder::from_app(ctx)
                    .muted_text_style()
                    .fg
                    .expect("muted text style should have a foreground")
            })
        );

        view.update(&mut app, |view, ctx| {
            view.conversation_selection.update(ctx, |selection, ctx| {
                selection.toggle_pending_query_autoexecute(ctx);
            });
        });
        let enabled = render_footer(&mut app, &view, 80);
        assert_eq!(enabled.to_lines(), vec!["▶▶".to_owned()]);
        assert_eq!(
            enabled[(0, 0)].fg,
            app.read(|ctx| {
                TuiUiBuilder::from_app(ctx)
                    .success_glyph_style()
                    .fg
                    .expect("success glyph style should have a foreground")
            })
        );
    });
}

#[test]
fn auto_approve_controls_retain_independent_mouse_state() {
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        let (view, _) = add_focus_test_session(&mut app, &fixture, true);
        view.read(&app, |view, _| {
            assert!(
                !Arc::ptr_eq(
                    &view.footer_auto_approve_mouse,
                    &view.warping_auto_approve_mouse,
                ),
                "footer and warping controls must not share retained mouse state",
            );
        });

        let (mut element, scene, buffer) = view.read(&app, |view, ctx| {
            let builder = TuiUiBuilder::from_app(ctx);
            let controls = TuiFlex::column()
                .child(view.render_warping_indicator("Warping...", Duration::ZERO, ctx))
                .child(view.render_auto_approve_statusline(&builder, ctx))
                .finish();
            render_retained_element(controls, ctx, 80, 2)
        });
        let lines = buffer.to_lines();
        let footer_row = lines
            .iter()
            .position(|line| line.trim_end() == "▶▶")
            .expect("footer control should render") as u16;
        let footer_col = first_visible_column(&lines[usize::from(footer_row)]) as u16;
        let (warping_col, warping_row) = footer_label_position(&buffer, "▶▶ Auto approve off");

        assert!(dispatch_session_event(
            &app,
            &view,
            &mut element,
            scene.clone(),
            &left_mouse_down(footer_col, footer_row),
        ));
        view.read(&app, |view, _| {
            assert!(view.footer_auto_approve_mouse.lock().unwrap().is_clicked());
            assert!(!view.warping_auto_approve_mouse.lock().unwrap().is_clicked());
        });
        assert!(
            dispatch_session_event(
                &app,
                &view,
                &mut element,
                scene.clone(),
                &left_mouse_up(footer_col, footer_row),
            ),
            "warping control must not cancel the footer's armed click",
        );
        view.read(&app, |view, _| {
            assert!(!view.footer_auto_approve_mouse.lock().unwrap().is_clicked());
            assert!(!view.warping_auto_approve_mouse.lock().unwrap().is_clicked());
        });

        assert!(dispatch_session_event(
            &app,
            &view,
            &mut element,
            scene.clone(),
            &left_mouse_down(warping_col, warping_row),
        ));
        view.read(&app, |view, _| {
            assert!(!view.footer_auto_approve_mouse.lock().unwrap().is_clicked());
            assert!(view.warping_auto_approve_mouse.lock().unwrap().is_clicked());
        });
        assert!(
            dispatch_session_event(
                &app,
                &view,
                &mut element,
                scene,
                &left_mouse_up(warping_col, warping_row),
            ),
            "footer control must not cancel the warping control's armed click",
        );
    });
}

#[test]
fn shell_mode_reserves_tab_even_when_attachments_render() {
    assert!(attachment_focus_available(false, true));
    assert!(!attachment_focus_available(true, true));
    assert!(!attachment_focus_available(false, false));
}
#[test]
fn shell_completion_source_warmup_loads_path_executables() {
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        let (view, _) = add_focus_test_session(&mut app, &fixture, true);
        let session = Arc::new(Session::test());

        view.update(&mut app, |view, ctx| {
            view.warm_shell_completion_sources(session.clone(), ctx);
        });

        let deadline = Instant::now() + Duration::from_secs(5);
        while !session.has_loaded_external_commands() && Instant::now() < deadline {
            Timer::after(Duration::from_millis(10)).await;
        }

        assert!(session.has_attempted_to_load_external_commands());
        assert!(session.has_loaded_external_commands());
        assert!(session.executable_names().any(|command| command == "git"));
    });
}

#[test]
fn nld_reset_only_unlocks_after_agent_control_and_not_on_user_edit() {
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        let (view, _) = add_focus_test_session(&mut app, &fixture, true);

        view.update(&mut app, |view, ctx| {
            AISettings::handle(ctx).update(ctx, |settings, ctx| {
                settings
                    .ai_autodetection_enabled_internal
                    .set_value(true, ctx)
                    .expect("test setting should update");
            });
            view.input_view.update(ctx, |input, ctx| {
                input.exit_shell_mode(ctx);
                input.set_text("git status", ctx);
            });
            assert_eq!(
                view.ai_input_model.as_ref(ctx).input_config(),
                AI_LOCKED_CONFIG,
                "an explicit Agent lock should be retained while the user edits"
            );

            // User edits must not reinterpret an explicit Agent lock as stale
            // agent-control state.
            view.handle_input_content_changed(true, ctx);
            assert_eq!(
                view.ai_input_model.as_ref(ctx).input_config(),
                AI_LOCKED_CONFIG,
                "user edits must not unlock an explicit Agent lock"
            );

            // A lock installed for agent terminal control is reset when that
            // control completes, which restores the first post-agent prompt to
            // the setting-derived NLD state.
            view.input_view.update(ctx, |input, ctx| {
                input.lock_for_agent_control(ctx);
            });
            view.input_view.update(ctx, |input, ctx| {
                input.reset_after_agent_control(ctx);
            });
            assert_eq!(
                view.ai_input_model.as_ref(ctx).input_config(),
                AI_UNLOCKED_CONFIG,
                "agent-control completion should resume NLD"
            );
        });
    });
}

#[test]
#[cfg(feature = "voice_input")]
fn voice_accepts_exact_and_whitespace_only_arguments() {
    assert_eq!(voice_command_argument("/voice"), Some(""));
    assert_eq!(voice_command_argument("/voice   "), Some("   "));
    assert_eq!(voice_command_argument("/voice text"), Some(" text"));
    assert_eq!(voice_command_argument("/voice-command text"), None);
    assert!(voice_argument_is_empty(None));
    assert!(voice_argument_is_empty(Some(&String::new())));
    assert!(voice_argument_is_empty(Some(&"   ".to_owned())));
    assert!(!voice_argument_is_empty(Some(&"text".to_owned())));
}

#[test]
#[cfg(feature = "voice_input")]
fn voice_slash_command_rejects_arguments_before_prompt_fallback() {
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        let (view, _) = add_focus_test_session(&mut app, &fixture, true);

        view.update(&mut app, |view, ctx| {
            view.input_view.update(ctx, |input, ctx| {
                input.set_text("/voice transcribe this", ctx);
            });
            view.handle_submitted_input("/voice transcribe this", ctx);
        });

        assert_eq!(app.read(|ctx| input_text(&view, ctx)), "");
        assert_eq!(
            view.read(&app, |view, _| {
                view.transient_hint
                    .current()
                    .map(|(text, _)| text.to_owned())
            }),
            Some(VOICE_USAGE_HINT.to_owned())
        );
    });
}

#[test]
fn log_bundle_success_message_includes_the_absolute_path() {
    let path = std::path::Path::new("/tmp/warp-20260718-132640.zip");
    assert_eq!(
        log_bundle_success_message(path),
        "Log bundle saved to /tmp/warp-20260718-132640.zip"
    );
}

#[test]
fn tui_cli_shell_command_uses_channel_entry_points() {
    assert_eq!(
        super::tui_cli_shell_command(Channel::Local, "--version"),
        "./script/run-tui -- --version"
    );
    assert_eq!(
        super::tui_cli_shell_command(Channel::Stable, "--version"),
        "warp --version"
    );
    assert_eq!(
        super::tui_cli_shell_command(Channel::Dev, "--version"),
        "warp-dev --version"
    );
    assert_eq!(
        super::tui_cli_shell_command(Channel::Preview, "--version"),
        "warp-preview --version"
    );
    assert_eq!(
        super::tui_cli_shell_command(Channel::Oss, "--version"),
        "warp-oss --version"
    );
    assert_eq!(
        super::tui_cli_shell_command(Channel::Integration, "--version"),
        "warp-integration --version"
    );
}

#[test]
fn log_bundle_failure_hint_does_not_hardcode_a_frontend_path() {
    assert!(!LOG_BUNDLE_FAILED_HINT.contains("warp.log"));
    assert!(!LOG_BUNDLE_FAILED_HINT.contains("/oz/"));
    assert!(!LOG_BUNDLE_FAILED_HINT.contains("/tui/"));
    assert!(!LOG_BUNDLE_FAILED_HINT.contains("/warp-cli/"));
}
#[test]
fn inline_menu_padding_preserves_result_capacity() {
    App::test((), |app| async move {
        app.read(|ctx| {
            let menu_rows = (0..MAX_INLINE_MENU_ROWS)
                .map(|row| format!("menu {row}"))
                .collect::<Vec<_>>();
            let menu = TuiConstrainedBox::new(
                TuiContainer::new(TuiText::new(menu_rows.join("\n")).finish())
                    .with_padding_top(INLINE_MENU_TOP_PADDING_ROWS)
                    .finish(),
            )
            .with_max_rows(MAX_INLINE_MENU_ROWS + INLINE_MENU_TOP_PADDING_ROWS)
            .finish();
            let lines = render_element_with_size(
                menu,
                ctx,
                20,
                MAX_INLINE_MENU_ROWS + INLINE_MENU_TOP_PADDING_ROWS,
            )
            .to_lines();

            assert_eq!(lines.len(), usize::from(MAX_INLINE_MENU_ROWS + 1));
            assert!(lines[0].trim().is_empty());
            assert_eq!(&lines[1..], menu_rows);
        });
    });
}

#[test]
fn zero_state_reload_failure_renders_as_an_error_footer_hint() {
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        let (view, _) = add_focus_test_session(&mut app, &fixture, true);

        app.update(|ctx| {
            ZeroStateAnimationConfig::handle(ctx).update(ctx, |_, ctx| {
                ctx.emit(ZeroStateAnimationConfigEvent::LoadFailed(
                    ZeroStateAnimationLoadFailure::Reload,
                ));
            });
        });

        assert_eq!(
            view.read(&app, |view, _| {
                view.transient_hint
                    .current()
                    .map(|(text, tone)| (text.to_owned(), tone))
            }),
            Some((
                super::ZERO_STATE_ASCII_RELOAD_FAILED_HINT.to_owned(),
                TransientHintTone::Error
            ))
        );

        app.read(|ctx| {
            let footer = view.as_ref(ctx).render_footer(ctx).finish();
            let buffer = render_element(footer, ctx, 120);
            assert_eq!(
                buffer.to_lines(),
                vec![super::ZERO_STATE_ASCII_RELOAD_FAILED_HINT.to_owned()]
            );
            assert_eq!(
                buffer[(0, 0)].fg,
                TuiUiBuilder::from_app(ctx)
                    .error_text_style()
                    .fg
                    .expect("error text style should have a foreground")
            );
        });
    });
}

#[test]
fn settings_reload_failure_renders_as_an_error_footer_hint() {
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        let (view, _) = add_focus_test_session(&mut app, &fixture, true);

        app.update(|ctx| {
            WarpConfig::handle(ctx).update(ctx, |_, ctx| {
                ctx.emit(WarpConfigUpdateEvent::SettingsErrors(
                    SettingsFileError::InvalidSettings(vec!["Theme".to_owned()]),
                ));
            });
        });

        assert_eq!(
            view.read(&app, |view, _| {
                view.transient_hint
                    .current()
                    .map(|(text, tone)| (text.to_owned(), tone))
            }),
            Some((
                super::SETTINGS_INVALID_VALUES_HINT.to_owned(),
                TransientHintTone::Error
            ))
        );

        app.read(|ctx| {
            let footer = view.as_ref(ctx).render_footer(ctx).finish();
            assert_eq!(
                render_element(footer, ctx, 120).to_lines(),
                vec![super::SETTINGS_INVALID_VALUES_HINT.to_owned()]
            );
        });
    });
}

#[test]
fn theme_slash_command_accepts_direct_selection_and_rejects_invalid_values() {
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        let (view, _) = add_focus_test_session(&mut app, &fixture, true);
        let light = "light".to_owned();

        view.update(&mut app, |view, ctx| {
            view.execute_tui_slash_command(&slash_commands::THEME, Some(&light), ctx);
        });
        app.read(|ctx| {
            assert_eq!(
                TuiTheme::from(Appearance::as_ref(ctx).theme()),
                TuiTheme::Light
            );
            assert_eq!(
                TuiThemeSettings::as_ref(ctx).selected_theme(),
                TuiTheme::Light
            );
        });
        let dark = "dark".to_owned();
        view.update(&mut app, |view, ctx| {
            view.execute_tui_slash_command(&slash_commands::THEME, Some(&dark), ctx);
        });
        app.read(|ctx| {
            assert_eq!(
                TuiTheme::from(Appearance::as_ref(ctx).theme()),
                TuiTheme::Dark
            );
            assert_eq!(
                TuiThemeSettings::as_ref(ctx).selected_theme(),
                TuiTheme::Dark
            );
        });

        let auto = "auto".to_owned();
        view.update(&mut app, |view, ctx| {
            view.execute_tui_slash_command(&slash_commands::THEME, Some(&auto), ctx);
        });
        app.read(|ctx| {
            assert_eq!(
                TuiTheme::from(Appearance::as_ref(ctx).theme()),
                TuiTheme::Dark
            );
            assert_eq!(
                TuiThemeSettings::as_ref(ctx).selected_theme(),
                TuiTheme::Auto
            );
        });

        let invalid = "sepia".to_owned();
        view.update(&mut app, |view, ctx| {
            view.execute_tui_slash_command(&slash_commands::THEME, Some(&invalid), ctx);
        });
        app.read(|ctx| {
            assert_eq!(
                TuiThemeSettings::as_ref(ctx).selected_theme(),
                TuiTheme::Auto
            );
        });
        assert_eq!(
            view.read(&app, |view, _| {
                view.transient_hint
                    .current()
                    .map(|(text, _)| text.to_owned())
            }),
            Some(super::THEME_INVALID_ARGUMENT_HINT.to_owned())
        );
    });
}

#[test]
fn zero_state_initial_load_failure_shows_an_error_footer_hint() {
    App::test((), |mut app| async move {
        let temp_dir = TempDir::new().unwrap();
        let config = ZeroStateAnimationConfig::load(
            &TuiZeroStateObject::AsciiFile {
                path: "missing.txt".into(),
            },
            5.0,
            0.18,
            temp_dir.path(),
        );
        app.add_singleton_model(move |_| config);
        let fixture = focus_test_fixture(&mut app);
        let (view, _) = add_focus_test_session(&mut app, &fixture, true);

        assert_eq!(
            view.read(&app, |view, _| {
                view.transient_hint
                    .current()
                    .map(|(text, tone)| (text.to_owned(), tone))
            }),
            Some((
                super::ZERO_STATE_ASCII_INITIAL_LOAD_FAILED_HINT.to_owned(),
                TransientHintTone::Error
            ))
        );
    });
}

#[test]
fn startup_settings_parse_failure_renders_as_an_error_footer_hint() {
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        let view = add_focus_test_session_with_settings_file_error(
            &mut app,
            &fixture,
            SettingsFileError::FileParseFailed("expected a value".to_owned()),
        );

        assert_eq!(
            view.read(&app, |view, _| {
                view.transient_hint
                    .current()
                    .map(|(text, tone)| (text.to_owned(), tone))
            }),
            Some((
                super::SETTINGS_PARSE_FAILED_HINT.to_owned(),
                TransientHintTone::Error
            ))
        );

        app.read(|ctx| {
            let footer = view.as_ref(ctx).render_footer(ctx).finish();
            assert_eq!(
                render_element(footer, ctx, 120).to_lines(),
                vec![super::SETTINGS_PARSE_FAILED_HINT.to_owned()]
            );
        });
    });
}

#[test]
#[cfg(feature = "voice_input")]
fn listening_voice_input_animates_the_input_border() {
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        let (view, _) = add_focus_test_session(&mut app, &fixture, true);
        view.update(&mut app, |view, ctx| {
            view.terminal_model
                .lock()
                .simulate_block("echo ready", "ready\r\n");
            let voice_input = view.input_view.as_ref(ctx).voice_input_model().clone();
            voice_input.update(ctx, |voice, ctx| {
                voice.set_state_for_test(TuiVoiceInputState::Listening, ctx);
            });
        });

        let mut presenter = TuiPresenter::new();
        let listening_frame = app.update(|ctx| {
            let mut invalidation = WindowInvalidation::default();
            invalidation.updated.insert(view.id());
            invalidation
                .updated
                .extend(view.as_ref(ctx).child_view_ids(ctx));
            presenter.invalidate(&invalidation, ctx, fixture.window_id);
            presenter.present(ctx, &view, TuiRect::new(0, 0, 100, 40))
        });
        assert!(
            listening_frame.repaint_at.is_some(),
            "the listening border should schedule its next animation frame"
        );

        view.update(&mut app, |view, ctx| {
            let voice_input = view.input_view.as_ref(ctx).voice_input_model().clone();
            voice_input.update(ctx, |voice, ctx| {
                voice.set_state_for_test(TuiVoiceInputState::Transcribing, ctx);
            });
        });
        let transcribing_frame = app.update(|ctx| {
            let mut invalidation = WindowInvalidation::default();
            invalidation.updated.insert(view.id());
            invalidation
                .updated
                .extend(view.as_ref(ctx).child_view_ids(ctx));
            presenter.invalidate(&invalidation, ctx, fixture.window_id);
            presenter.present(ctx, &view, TuiRect::new(0, 0, 100, 40))
        });
        assert!(
            transcribing_frame.repaint_at.is_none(),
            "the border should stop animating after recording stops"
        );
    });
}
fn mouse_moved(x: u16, y: u16) -> TuiEvent {
    TuiEvent::MouseMoved {
        position: TuiPoint::new(x, y),
        modifiers: ModifiersState::default(),
        is_synthetic: false,
    }
}

fn left_mouse_down(x: u16, y: u16) -> TuiEvent {
    TuiEvent::LeftMouseDown {
        position: TuiPoint::new(x, y),
        modifiers: ModifiersState::default(),
        click_count: 1,
        is_first_mouse: false,
    }
}

fn left_mouse_up(x: u16, y: u16) -> TuiEvent {
    TuiEvent::LeftMouseUp {
        position: TuiPoint::new(x, y),
        modifiers: ModifiersState::default(),
    }
}

/// Renders the session view's element tree outside the presenter so the test
/// can dispatch mouse events against the retained element + scene. Child views
/// (transcript/input/attachment bar) are absent from `rendered_views`, so they
/// lay out zero-size; the footer — part of the session view's own tree —
/// renders with the clickable model label.
fn render_retained_element(
    mut element: Box<dyn TuiElement>,
    ctx: &AppContext,
    width: u16,
    height: u16,
) -> (Box<dyn TuiElement>, Rc<TuiScene>, TuiBuffer) {
    let mut rendered_views = EntityIdMap::default();
    let mut layout_ctx = TuiLayoutContext {
        rendered_views: &mut rendered_views,
    };
    let size = element.layout(
        TuiConstraint::loose(TuiSize::new(width, height)),
        &mut layout_ctx,
        ctx,
    );
    element.after_layout(&mut layout_ctx, ctx);
    let area = TuiRect::new(0, 0, size.width.min(width), size.height.min(height));
    let mut buffer = TuiBuffer::empty(area);
    let mut paint_ctx = TuiPaintContext::new(&mut rendered_views);
    {
        let mut surface = TuiPaintSurface::new(&mut buffer);
        element.render(TuiScreenPosition::new(0, 0), &mut surface, &mut paint_ctx);
    }
    let scene = Rc::new(paint_ctx.scene.clone());
    (element, scene, buffer)
}

fn render_retained_session(
    app: &App,
    view: &ViewHandle<super::TuiTerminalSessionView>,
    width: u16,
    height: u16,
) -> (Box<dyn TuiElement>, Rc<TuiScene>, TuiBuffer) {
    app.read(|ctx| {
        let element = ctx
            .render_tui_view(view.window_id(ctx), view.id())
            .expect("session view should render");
        render_retained_element(element, ctx, width, height)
    })
}
fn render_footer_lines(
    app: &mut App,
    view: &ViewHandle<super::TuiTerminalSessionView>,
    width: u16,
) -> Vec<String> {
    render_footer(app, view, width).to_lines()
}
#[test]
fn input_area_renders_all_six_editor_rows() {
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        let (view, _) = add_focus_test_session(&mut app, &fixture, true);
        view.update(&mut app, |view, ctx| {
            view.input_view.update(ctx, |input, ctx| {
                input.set_text("input-0\ninput-1\ninput-2\ninput-3\ninput-4\ninput-5", ctx);
            });
        });

        let rendered = render_session(&mut app, &view, 80, 24).join("\n");
        for row in 0..6 {
            assert!(
                rendered.contains(&format!("input-{row}")),
                "input row {row} should be visible:\n{rendered}"
            );
        }
    });
}

/// Dispatches `event` into the retained session element tree with the session
/// view as the action origin, returning whether the tree handled it.
fn dispatch_session_event(
    app: &App,
    view: &ViewHandle<super::TuiTerminalSessionView>,
    element: &mut Box<dyn TuiElement>,
    scene: Rc<TuiScene>,
    event: &TuiEvent,
) -> bool {
    app.read(|ctx| {
        let mut rendered_views = EntityIdMap::default();
        let mut event_ctx = TuiEventContext::new(scene, &mut rendered_views);
        event_ctx.set_origin_view(Some(view.id()));
        element.dispatch_event(event, &mut event_ctx, ctx)
    })
}

/// Locates a label in the rendered footer, returning the (column, row) of its
/// first cell. Counts chars (not bytes) so multi-byte glyphs earlier in the
/// footer row don't shift the column.
fn footer_label_position(buffer: &TuiBuffer, label: &str) -> (u16, u16) {
    let lines = buffer.to_lines();
    for (row, line) in lines.iter().enumerate() {
        if let Some(byte_offset) = line.find(label) {
            let col = line[..byte_offset].chars().count() as u16;
            return (col, row as u16);
        }
    }
    panic!(
        "label {:?} not found in rendered footer:\n{}",
        label,
        lines.join("\n")
    );
}

#[test]
fn toggle_model_menu_action_opens_and_closes_the_inline_model_menu() {
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        let (view, _) = add_focus_test_session(&mut app, &fixture, true);
        view.read(&app, |view, ctx| {
            assert!(
                !view.model_menu.as_ref(ctx).is_open(ctx),
                "model menu should start closed"
            );
        });
        view.update(&mut app, |view, ctx| {
            view.handle_action(&TuiTerminalSessionAction::ToggleModelMenu, ctx);
        });
        view.read(&app, |view, ctx| {
            assert!(
                view.model_menu.as_ref(ctx).is_open(ctx),
                "ToggleModelMenu action should open a closed inline model menu"
            );
        });
        view.update(&mut app, |view, ctx| {
            view.handle_action(&TuiTerminalSessionAction::ToggleModelMenu, ctx);
        });
        view.read(&app, |view, ctx| {
            assert!(
                !view.model_menu.as_ref(ctx).is_open(ctx),
                "ToggleModelMenu action should close an open inline model menu"
            );
        });
    });
}

#[test]
fn accepted_model_only_changes_the_current_session() {
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        LLMPreferences::handle(&app).update(&mut app, |preferences, ctx| {
            let mut alternate = preferences.get_default_base_model(ctx).clone();
            alternate.id = "tui-session-override".into();
            alternate.display_name = "TUI session override".to_owned();
            preferences.add_agent_mode_model_for_test(alternate);
        });
        let (first_view, _) = add_focus_test_session(&mut app, &fixture, true);
        let (second_view, _) = add_focus_test_session(&mut app, &fixture, false);
        let (profile_default_id, alternate_id) = app.read(|ctx| {
            let preferences = LLMPreferences::as_ref(ctx);
            let profile_default_id = preferences
                .get_active_profile_base_model(ctx, None)
                .id
                .clone();
            let alternate_id = preferences
                .get_base_llm_choices_for_agent_mode(ctx)
                .find(|model| model.id != profile_default_id && model.disable_reason.is_none())
                .expect("test model catalog should include an alternate model")
                .id
                .clone();
            (profile_default_id, alternate_id)
        });

        first_view.update(&mut app, |view, ctx| {
            view.handle_accepted_model(&alternate_id, ctx);
        });

        app.read(|ctx| {
            let preferences = LLMPreferences::as_ref(ctx);
            let first_surface_id = first_view.as_ref(ctx).terminal_surface_id;
            let second_surface_id = second_view.as_ref(ctx).terminal_surface_id;
            assert_eq!(
                preferences
                    .get_active_base_model(ctx, Some(first_surface_id))
                    .id,
                alternate_id
            );
            assert_eq!(
                preferences
                    .get_active_base_model(ctx, Some(second_surface_id))
                    .id,
                profile_default_id
            );
            assert_eq!(
                preferences
                    .get_active_profile_base_model(ctx, Some(first_surface_id))
                    .id,
                profile_default_id
            );
        });
    });
}

#[test]
fn model_menu_labels_the_profile_default_model() {
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        let (view, _) = add_focus_test_session(&mut app, &fixture, true);
        view.update(&mut app, |view, ctx| {
            view.model_menu.update(ctx, |menu, ctx| menu.open(ctx));
        });

        view.read(&app, |view, ctx| {
            let default_name = LLMPreferences::as_ref(ctx)
                .get_active_profile_base_model(ctx, Some(view.terminal_surface_id))
                .display_name
                .clone();
            let snapshot = view
                .model_menu
                .as_ref(ctx)
                .snapshot(ctx)
                .expect("model menu should be open");
            let default_row = snapshot
                .rows
                .iter()
                .find(|row| row.title == default_name)
                .expect("profile default model should be listed");
            assert!(
                default_row
                    .state_suffix
                    .as_deref()
                    .is_some_and(|suffix| suffix.contains("(default)"))
            );
            assert_eq!(
                snapshot
                    .rows
                    .iter()
                    .filter(|row| {
                        row.state_suffix
                            .as_deref()
                            .is_some_and(|suffix| suffix.contains("(default)"))
                    })
                    .count(),
                1
            );
        });
    });
}
#[test]
fn todo_menu_renders_active_list_and_toggles_through_shared_suggestions_mode() {
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        let (view, _) = add_focus_test_session(&mut app, &fixture, true);
        set_enabled_statusline_items(&mut app, vec![TuiStatuslineItem::AgentTodoList]);
        set_selected_todo_list(
            &mut app,
            &view,
            vec![todo("done", "Completed task")],
            vec![todo("current", "Current task"), todo("later", "Later task")],
            ConversationStatus::InProgress,
        );

        assert_eq!(render_footer_lines(&mut app, &view, 80), vec!["❒ 1/3"]);
        view.update(&mut app, |view, ctx| {
            view.handle_action(&TuiTerminalSessionAction::ToggleTodoMenu, ctx);
        });
        view.read(&app, |view, ctx| {
            assert_eq!(
                view.suggestions_mode.as_ref(ctx).mode(),
                TuiInputSuggestionsMode::ReadOnlyMenu(TuiReadOnlyMenuKind::Todos)
            );
            assert_eq!(
                view.read_only_menu_viewport.position(),
                TuiViewportPosition::RowsFromTop(2),
                "the title and completed row precede the current task"
            );
        });

        let rendered = render_session(&mut app, &view, 80, 24).join("\n");
        let completed = rendered.find("✓ Completed task").unwrap();
        let current = rendered.find("● Current task").unwrap();
        let later = rendered.find("◌ Later task").unwrap();
        assert!(rendered.contains("Tasks 1/3"));
        assert!(completed < current && current < later);

        view.update(&mut app, |view, ctx| {
            view.suggestions_mode.update(ctx, |mode, ctx| {
                mode.set_mode(
                    TuiInputSuggestionsMode::ReadOnlyMenu(TuiReadOnlyMenuKind::Status),
                    ctx,
                );
            });
            view.handle_action(&TuiTerminalSessionAction::ToggleTodoMenu, ctx);
            assert_eq!(
                view.suggestions_mode.as_ref(ctx).mode(),
                TuiInputSuggestionsMode::ReadOnlyMenu(TuiReadOnlyMenuKind::Todos)
            );
            view.handle_action(&TuiTerminalSessionAction::ToggleTodoMenu, ctx);
            assert_eq!(
                view.suggestions_mode.as_ref(ctx).mode(),
                TuiInputSuggestionsMode::Closed
            );
        });
    });
}

#[test]
fn finished_todo_list_remains_visible_and_openable() {
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        let (view, _) = add_focus_test_session(&mut app, &fixture, true);
        set_enabled_statusline_items(&mut app, vec![TuiStatuslineItem::AgentTodoList]);
        set_selected_todo_list(
            &mut app,
            &view,
            vec![todo("done", "Completed task")],
            Vec::new(),
            ConversationStatus::Success,
        );

        assert_eq!(render_footer_lines(&mut app, &view, 80), vec!["✓ 1/1"]);
        view.update(&mut app, |view, ctx| {
            view.handle_action(&TuiTerminalSessionAction::ToggleTodoMenu, ctx);
        });
        let rendered = render_session(&mut app, &view, 80, 24).join("\n");
        assert!(rendered.contains("Tasks 1/1"));
        assert!(rendered.contains("✓ Completed task"));
    });
}

#[test]
fn todo_updates_preserve_scroll_and_close_the_menu_when_the_list_disappears() {
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        let (view, _) = add_focus_test_session(&mut app, &fixture, true);
        let conversation_id = set_selected_todo_list(
            &mut app,
            &view,
            Vec::new(),
            vec![todo("current", "Current task")],
            ConversationStatus::InProgress,
        );
        view.update(&mut app, |view, ctx| {
            view.handle_action(&TuiTerminalSessionAction::ToggleTodoMenu, ctx);
            view.read_only_menu_viewport.scroll_to_rows_from_top(4);
            view.handle_history_event(
                &BlocklistAIHistoryEvent::UpdatedTodoList {
                    terminal_surface_id: view.terminal_surface_id,
                },
                ctx,
            );
            assert_eq!(
                view.read_only_menu_viewport.position(),
                TuiViewportPosition::RowsFromTop(4)
            );
            view.handle_action(
                &TuiTerminalSessionAction::ToggleAutoApprove {
                    show_feedback: false,
                },
                ctx,
            );
            assert_eq!(
                view.read_only_menu_viewport.position(),
                TuiViewportPosition::RowsFromTop(4)
            );

            BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, _| {
                history
                    .conversation_mut(&conversation_id)
                    .unwrap()
                    .set_todo_lists_for_test(vec![
                        AIAgentTodoList::default()
                            .with_pending_items(vec![todo("old", "Old task")]),
                        AIAgentTodoList::default()
                            .with_completed_items(vec![todo("done", "Completed task")])
                            .with_pending_items(vec![todo("new", "New current task")]),
                    ]);
            });
            view.handle_history_event(
                &BlocklistAIHistoryEvent::UpdatedTodoList {
                    terminal_surface_id: view.terminal_surface_id,
                },
                ctx,
            );
            assert_eq!(
                view.read_only_menu_viewport.position(),
                TuiViewportPosition::RowsFromTop(2)
            );

            BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, _| {
                history
                    .conversation_mut(&conversation_id)
                    .unwrap()
                    .set_todo_lists_for_test(Vec::new());
            });
            view.handle_history_event(
                &BlocklistAIHistoryEvent::UpdatedTodoList {
                    terminal_surface_id: view.terminal_surface_id,
                },
                ctx,
            );
            assert_eq!(
                view.suggestions_mode.as_ref(ctx).mode(),
                TuiInputSuggestionsMode::Closed
            );
        });
    });
}

#[test]
fn footer_todo_item_is_a_bounded_click_target() {
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        let (view, _) = add_focus_test_session(&mut app, &fixture, true);
        set_enabled_statusline_items(&mut app, vec![TuiStatuslineItem::AgentTodoList]);
        set_selected_todo_list(
            &mut app,
            &view,
            Vec::new(),
            vec![todo("current", "Current task")],
            ConversationStatus::InProgress,
        );
        let (mut element, scene, buffer) = render_retained_session(&app, &view, 40, 20);
        let (todo_col, todo_row) = footer_label_position(&buffer, "❒ 0/1");
        let inside = (todo_col + 1, todo_row);
        let outside = (todo_col + 6, todo_row);

        dispatch_session_event(
            &app,
            &view,
            &mut element,
            scene.clone(),
            &mouse_moved(inside.0, inside.1),
        );
        assert!(view.read(&app, |view, _| {
            view.todo_list_mouse.lock().unwrap().is_hovered()
        }));
        assert!(dispatch_session_event(
            &app,
            &view,
            &mut element,
            scene.clone(),
            &left_mouse_down(inside.0, inside.1),
        ));
        assert!(dispatch_session_event(
            &app,
            &view,
            &mut element,
            scene.clone(),
            &left_mouse_up(inside.0, inside.1),
        ));
        assert!(!view.read(&app, |view, _| {
            view.todo_list_mouse.lock().unwrap().is_clicked()
        }));

        dispatch_session_event(
            &app,
            &view,
            &mut element,
            scene.clone(),
            &mouse_moved(outside.0, outside.1),
        );
        assert!(!view.read(&app, |view, _| {
            view.todo_list_mouse.lock().unwrap().is_hovered()
        }));
        assert!(!dispatch_session_event(
            &app,
            &view,
            &mut element,
            scene,
            &left_mouse_down(outside.0, outside.1),
        ));
    });
}
#[test]
fn auto_approve_slash_command_toggles_selected_conversation_off_on_off() {
    App::test((), |mut app| async move {
        assert_eq!(
            AUTO_APPROVE_FEEDBACK_DURATION,
            std::time::Duration::from_secs(3)
        );
        let fixture = focus_test_fixture(&mut app);
        let (view, _) = add_focus_test_session(&mut app, &fixture, true);

        // New TUI conversations default to `RespectUserSettings` (off).
        view.read(&app, |view, ctx| {
            assert_eq!(
                view.conversation_selection
                    .as_ref(ctx)
                    .pending_query_autoexecute_override(ctx),
                AIConversationAutoexecuteMode::RespectUserSettings
            );
            assert!(view.auto_approve_feedback_conversation_id.is_none());
        });

        // Invoking `/auto-approve` executes the TUI `AutoApprove` arm and toggles
        // the selected conversation on.
        view.update(&mut app, |view, ctx| {
            view.execute_tui_slash_command(&slash_commands::AUTO_APPROVE, None, ctx);
        });
        view.read(&app, |view, ctx| {
            assert_eq!(
                view.conversation_selection
                    .as_ref(ctx)
                    .pending_query_autoexecute_override(ctx),
                AIConversationAutoexecuteMode::RunToCompletion
            );
            assert_eq!(
                view.auto_approve_feedback_conversation_id,
                view.conversation_selection
                    .as_ref(ctx)
                    .selected_conversation_id(ctx)
            );
            assert_eq!(
                view.transient_hint.current(),
                Some((
                    AUTO_APPROVE_ENABLED_HINT,
                    crate::transient_hint::TransientHintTone::Success
                ))
            );
        });

        // Invoking `/auto-approve` again toggles it back off.
        view.update(&mut app, |view, ctx| {
            view.execute_tui_slash_command(&slash_commands::AUTO_APPROVE, None, ctx);
        });
        view.read(&app, |view, ctx| {
            assert_eq!(
                view.conversation_selection
                    .as_ref(ctx)
                    .pending_query_autoexecute_override(ctx),
                AIConversationAutoexecuteMode::RespectUserSettings
            );
            assert_eq!(
                view.auto_approve_feedback_conversation_id,
                view.conversation_selection
                    .as_ref(ctx)
                    .selected_conversation_id(ctx)
            );
            assert_eq!(
                view.transient_hint.current(),
                Some((
                    AUTO_APPROVE_DISABLED_HINT,
                    crate::transient_hint::TransientHintTone::Success
                ))
            );
        });
    });
}

#[test]
fn theme_slash_command_rejects_a_missing_argument() {
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        let (view, _) = add_focus_test_session(&mut app, &fixture, true);

        app.read(|ctx| {
            assert_eq!(
                TuiTheme::from(Appearance::as_ref(ctx).theme()),
                TuiTheme::Dark
            );
            assert_eq!(
                TuiThemeSettings::as_ref(ctx).selected_theme(),
                TuiTheme::Auto
            );
        });

        view.update(&mut app, |view, ctx| {
            view.execute_tui_slash_command(&slash_commands::THEME, None, ctx);
        });
        app.read(|ctx| {
            assert_eq!(
                TuiTheme::from(Appearance::as_ref(ctx).theme()),
                TuiTheme::Dark
            );
            assert_eq!(
                TuiThemeSettings::as_ref(ctx).selected_theme(),
                TuiTheme::Auto
            );
        });
        assert_eq!(
            view.read(&app, |view, _| {
                view.transient_hint
                    .current()
                    .map(|(text, _)| text.to_owned())
            }),
            Some(super::THEME_INVALID_ARGUMENT_HINT.to_owned())
        );
    });
}

#[test]
fn statusline_slash_command_preserves_nested_focus_and_restores_the_input_cursor() {
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        let (view, _) = add_focus_test_session(&mut app, &fixture, true);
        view.update(&mut app, |view, ctx| {
            view.input_view.update(ctx, |input, ctx| {
                input.set_text("/statusline", ctx);
            });
            view.execute_tui_slash_command(&slash_commands::STATUSLINE, None, ctx);
        });

        let (picker_id, picker_focus_id) = view.read(&app, |view, ctx| {
            let picker = view
                .statusline_config_view
                .as_ref()
                .expect("statusline picker should be open");
            assert_eq!(
                view.input_view
                    .as_ref(ctx)
                    .model()
                    .as_ref(ctx)
                    .content()
                    .as_ref(ctx)
                    .text()
                    .into_string(),
                ""
            );
            assert!(ctx.check_view_or_child_focused(fixture.window_id, &picker.id()));
            (
                picker.id(),
                ctx.focused_view_id(fixture.window_id)
                    .expect("the statusline picker should delegate focus to a child"),
            )
        });

        assert!(view.update(&mut app, |view, ctx| { view.handle_terminal_wakeup(ctx) }));
        app.read(|ctx| {
            assert_eq!(
                ctx.focused_view_id(fixture.window_id),
                Some(picker_focus_id),
                "a redraw must preserve the interaction surface's delegated child focus"
            );
        });
        view.update(&mut app, |view, ctx| {
            view.execute_tui_slash_command(&slash_commands::STATUSLINE, None, ctx);
        });
        assert_eq!(
            view.read(&app, |view, _| {
                view.statusline_config_view.as_ref().map(ViewHandle::id)
            }),
            Some(picker_id),
        );

        view.update(&mut app, |view, ctx| {
            view.handle_statusline_config_event(&TuiStatuslineConfigEvent::Cancelled, ctx);
        });
        view.read(&app, |view, ctx| {
            assert!(view.statusline_config_view.is_none());
            assert!(ctx.check_view_or_child_focused(fixture.window_id, &view.input_view.id()));
        });
        assert!(
            render_session_frame(&mut app, &view, 80, 24)
                .cursor
                .is_some(),
            "dismissing the interaction surface should restore the input cursor"
        );
    });
}

#[test]
fn saving_statusline_configuration_persists_and_restores_input_focus() {
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        let (view, _) = add_focus_test_session(&mut app, &fixture, true);
        let config = TuiStatuslineConfig {
            order: vec![
                TuiStatuslineItem::ContextWindowUsage,
                TuiStatuslineItem::CreditUsage,
            ],
            enabled: Vec::new(),
        }
        .normalized();

        view.update(&mut app, |view, ctx| {
            view.execute_tui_slash_command(&slash_commands::STATUSLINE, None, ctx);
            view.handle_statusline_config_event(
                &TuiStatuslineConfigEvent::Saved(config.clone()),
                ctx,
            );
        });

        assert_eq!(
            app.read(|ctx| AISettings::as_ref(ctx).tui_statusline.normalized()),
            config,
        );
        view.read(&app, |view, ctx| {
            assert!(view.statusline_config_view.is_none());
            assert!(ctx.check_view_or_child_focused(fixture.window_id, &view.input_view.id()));
            assert_eq!(
                view.transient_hint.current().map(|(text, _)| text),
                Some(super::STATUSLINE_SAVED_HINT),
            );
        });
    });
}

#[test]
fn reset_statusline_command_restores_default_items_and_ordering() {
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        let (view, _) = add_focus_test_session(&mut app, &fixture, true);
        let custom = TuiStatuslineConfig {
            order: vec![TuiStatuslineItem::CreditUsage, TuiStatuslineItem::Model],
            enabled: vec![TuiStatuslineItem::CreditUsage],
        }
        .normalized();
        assert_ne!(custom, TuiStatuslineConfig::default());

        view.update(&mut app, |view, ctx| {
            AISettings::handle(ctx).update(ctx, |settings, ctx| {
                settings
                    .tui_statusline
                    .set_value(custom, ctx)
                    .expect("custom statusline should persist");
            });
            view.input_view.update(ctx, |input, ctx| {
                input.set_text("/reset-statusline", ctx);
            });
            view.execute_tui_slash_command(&slash_commands::RESET_STATUSLINE, None, ctx);
        });

        assert_eq!(
            app.read(|ctx| AISettings::as_ref(ctx).tui_statusline.clone()),
            TuiStatuslineConfig::default(),
        );
        view.read(&app, |view, ctx| {
            assert!(view.statusline_config_view.is_none());
            assert_eq!(
                view.input_view
                    .as_ref(ctx)
                    .model()
                    .as_ref(ctx)
                    .content()
                    .as_ref(ctx)
                    .text()
                    .into_string(),
                ""
            );
            assert_eq!(
                view.transient_hint.current().map(|(text, _)| text),
                Some(STATUSLINE_RESET_HINT),
            );
        });
    });
}

#[test]
fn cost_slash_command_rejects_an_empty_conversation_like_the_gui() {
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        let (view, _) = add_focus_test_session(&mut app, &fixture, true);

        view.update(&mut app, |view, ctx| {
            view.conversation_selection.update(ctx, |selection, ctx| {
                selection
                    .try_start_new_conversation(AgentViewEntryOrigin::Tui, ctx)
                    .expect("test conversation should start");
            });
        });

        view.update(&mut app, |view, ctx| {
            view.execute_tui_slash_command(&slash_commands::COST, None, ctx);
        });
        view.read(&app, |view, _| {
            assert!(view.hidden_response_summary_exchange_ids.is_empty());
            assert_eq!(
                view.transient_hint.current().map(|(text, _)| text),
                Some(COST_EMPTY_CONVERSATION_HINT),
            );
        });
    });
}

#[test]
fn fork_slash_command_is_available_on_both_surfaces() {
    assert_eq!(slash_commands::FORK.name, "/fork");
    assert!(slash_commands::FORK.supported_surfaces.supports_gui());
    assert!(slash_commands::FORK.supported_surfaces.supports_tui());
}

#[test]
fn fork_slash_command_rejects_an_empty_conversation() {
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        let (view, _) = add_focus_test_session(&mut app, &fixture, true);

        view.update(&mut app, |view, ctx| {
            view.execute_tui_slash_command(&slash_commands::FORK, None, ctx);
        });

        view.read(&app, |view, ctx| {
            assert_eq!(
                view.transient_hint.current().map(|(text, _)| text),
                Some(super::FORK_EMPTY_CONVERSATION_HINT),
            );
            assert!(view.input_view.as_ref(ctx).is_empty(ctx));
        });
    });
}

#[test]
fn fork_slash_command_keeps_a_conversation_without_a_resume_id_selected() {
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        let (view, _) = add_focus_test_session(&mut app, &fixture, true);
        let source_conversation = forkable_tui_conversation_for_test("local-only conversation");
        let source_conversation_id = source_conversation.id();

        view.update(&mut app, |view, ctx| {
            view.replace_conversation_surface(
                source_conversation,
                TuiConversationRestoreOrigin::ConversationList,
                TuiConversationRestoreTelemetryTarget::Local,
                ctx,
            );
            view.execute_tui_slash_command(&slash_commands::FORK, None, ctx);
        });

        view.read(&app, |view, ctx| {
            assert_eq!(
                view.conversation_selection
                    .as_ref(ctx)
                    .selected_conversation_id(ctx),
                Some(source_conversation_id),
            );
            assert_eq!(
                view.transient_hint.current().map(|(text, _)| text),
                Some(super::FORK_NO_RESUME_ID_HINT),
            );
        });
    });
}

#[test]
fn fork_slash_command_replaces_the_surface_and_renders_original_resume_guidance() {
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        let (view, _) = add_focus_test_session(&mut app, &fixture, true);
        let (model_event_sender, _model_event_receiver) = std::sync::mpsc::sync_channel(2);
        app.update(|ctx| {
            warp::tui_export::GlobalResourceHandlesProvider::handle(ctx).update(
                ctx,
                |provider, _| {
                    provider.set_model_event_sender_for_test(model_event_sender);
                },
            );
        });

        let source_token = "11111111-1111-1111-1111-111111111111";
        let source_conversation =
            forkable_tui_conversation_for_test("Original fork boundary prompt");
        let source_conversation_id = source_conversation.id();
        let source_root_task_id = source_conversation.get_root_task_id().clone();
        view.update(&mut app, |view, ctx| {
            let source_conversation =
                BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                    history.restore_conversations(
                        view.terminal_surface_id,
                        vec![source_conversation],
                        ctx,
                    );
                    history.set_server_conversation_token_for_conversation(
                        source_conversation_id,
                        source_token.to_owned(),
                    );
                    history
                        .conversation(&source_conversation_id)
                        .expect("source conversation should be restored")
                        .clone()
                });
            view.replace_conversation_surface(
                source_conversation,
                TuiConversationRestoreOrigin::ConversationList,
                TuiConversationRestoreTelemetryTarget::Local,
                ctx,
            );
            assert!(
                view.slash_commands_source
                    .as_ref(ctx)
                    .active_commands()
                    .any(|(_, command)| command.kind == SlashCommandKind::Fork)
            );
            view.execute_tui_slash_command(&slash_commands::FORK, None, ctx);
        });

        let forked_conversation_id = view.read(&app, |view, ctx| {
            view.conversation_selection
                .as_ref(ctx)
                .selected_conversation_id(ctx)
                .expect("fork should select a replacement conversation")
        });
        assert_ne!(forked_conversation_id, source_conversation_id);
        app.read(|ctx| {
            let history = BlocklistAIHistoryModel::as_ref(ctx);
            assert!(history.conversation(&source_conversation_id).is_some());
            let forked = history
                .conversation(&forked_conversation_id)
                .expect("forked conversation should be restored");
            assert_ne!(forked.get_root_task_id(), &source_root_task_id);
            assert_eq!(
                forked
                    .forked_from_server_conversation_token()
                    .map(|token| token.as_str()),
                Some(source_token),
            );
            assert_eq!(forked.exchange_count(), 1);
            assert_eq!(
                view.as_ref(ctx)
                    .transcript
                    .as_ref(ctx)
                    .agent_blocks_in_canonical_order()
                    .len(),
                1,
            );
        });

        let rendered = render_session(&mut app, &view, 100, 40).join("\n");
        let notice = rendered
            .find("Forked conversation. To resume the original in another session, run:")
            .unwrap_or_else(|| panic!("fork should render resume guidance:\n{rendered}"));
        let resume_id = rendered.find(source_token).unwrap_or_else(|| {
            panic!("resume guidance should contain the original server token:\n{rendered}")
        });
        assert!(notice < resume_id);
    });
}

#[test]
fn cost_command_uses_the_gui_eligibility_rules() {
    assert_eq!(
        cost_command_unavailable_hint(None),
        Some(COST_NO_ACTIVE_CONVERSATION_HINT),
    );
    assert_eq!(
        cost_command_unavailable_hint(Some((true, false))),
        Some(COST_EMPTY_CONVERSATION_HINT),
    );
    assert_eq!(
        cost_command_unavailable_hint(Some((false, false))),
        Some(COST_CONVERSATION_IN_PROGRESS_HINT),
    );
    assert_eq!(cost_command_unavailable_hint(Some((false, true))), None);
}

/// Renders the agent-mode footer row (`render_status_footer_row` + the real
/// `UsageToggle::render_entry`) to text lines with fixed totals.
fn render_usage_footer_row(app: &mut App, totals: ConversationUsageTotals) -> Vec<String> {
    app.update(|ctx| {
        let builder = TuiUiBuilder::from_app(ctx);
        let mode = AISettings::as_ref(ctx).usage_display_mode;
        let usage = UsageToggle::default().render_entry(mode, totals, ctx, |_, _| {});
        let row = render_status_footer_row(
            FooterSegments {
                ordered: vec![
                    FooterSegment::Model(
                        TuiText::new("TestModel")
                            .with_style(builder.primary_text_style())
                            .truncate()
                            .finish(),
                    ),
                    FooterSegment::CreditUsage(usage),
                ],
            },
            &builder,
        )
        .finish();
        render_element(row, ctx, 60).to_lines()
    })
}

#[test]
fn response_summary_visibility_is_independent_from_the_footer_usage_mode() {
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        let (view, _) = add_focus_test_session(&mut app, &fixture, true);
        let exchange_id = AIAgentExchangeId::new();

        let totals = ConversationUsageTotals {
            credits_spent: 2.5,
            cost_in_cents: Some(3.2),
            has_usage: true,
        };

        assert_eq!(
            app.read(|ctx| AISettings::as_ref(ctx).usage_display_mode),
            TuiUsageDisplayMode::Credits,
        );
        let footer_before = render_usage_footer_row(&mut app, totals);
        let summary_before = view.read(&app, |view, ctx| {
            view.render_response_summary_for_exchange(
                exchange_id,
                Duration::from_secs(2),
                Some(3.0),
                ctx,
            )
            .map(|summary| render_element(summary, ctx, 60).to_lines())
        });
        assert_eq!(summary_before, Some(vec!["∷ 2s • 3 credits".to_owned()]),);

        view.update(&mut app, |view, _| {
            view.toggle_response_summary_visibility_for_exchange(exchange_id);
        });
        let summary_hidden = view.read(&app, |view, ctx| {
            view.render_response_summary_for_exchange(
                exchange_id,
                Duration::from_secs(2),
                Some(3.0),
                ctx,
            )
        });
        assert!(summary_hidden.is_none());
        assert_eq!(
            app.read(|ctx| AISettings::as_ref(ctx).usage_display_mode),
            TuiUsageDisplayMode::Credits,
        );
        assert_eq!(
            render_usage_footer_row(&mut app, totals),
            footer_before,
            "hiding the response summary must not change the persistent footer",
        );

        view.update(&mut app, |view, _| {
            view.toggle_response_summary_visibility_for_exchange(exchange_id);
        });
        let summary_again = view.read(&app, |view, ctx| {
            view.render_response_summary_for_exchange(
                exchange_id,
                Duration::from_secs(2),
                Some(3.0),
                ctx,
            )
            .map(|summary| render_element(summary, ctx, 60).to_lines())
        });
        assert_eq!(summary_again, Some(vec!["∷ 2s • 3 credits".to_owned()]),);
    });
}

#[test]
fn auto_approve_actions_control_visible_feedback() {
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        let (view, _) = add_focus_test_session(&mut app, &fixture, true);

        view.update(&mut app, |view, ctx| {
            view.handle_action(
                &TuiTerminalSessionAction::ToggleAutoApprove {
                    show_feedback: true,
                },
                ctx,
            );
        });
        view.read(&app, |view, ctx| {
            assert_eq!(
                view.conversation_selection
                    .as_ref(ctx)
                    .pending_query_autoexecute_override(ctx),
                AIConversationAutoexecuteMode::RunToCompletion
            );
            assert_eq!(
                view.auto_approve_feedback_conversation_id,
                view.conversation_selection
                    .as_ref(ctx)
                    .selected_conversation_id(ctx)
            );
            assert_eq!(
                view.transient_hint.current(),
                Some((
                    AUTO_APPROVE_ENABLED_HINT,
                    crate::transient_hint::TransientHintTone::Success
                ))
            );
        });

        view.update(&mut app, |view, ctx| {
            view.handle_action(
                &TuiTerminalSessionAction::ToggleAutoApprove {
                    show_feedback: false,
                },
                ctx,
            );
        });
        view.read(&app, |view, ctx| {
            assert_eq!(
                view.conversation_selection
                    .as_ref(ctx)
                    .pending_query_autoexecute_override(ctx),
                AIConversationAutoexecuteMode::RespectUserSettings
            );
            assert!(view.auto_approve_feedback_conversation_id.is_none());
            assert!(view.auto_approve_feedback_timer.is_none());
        });
    });
}

#[test]
fn auto_queue_is_not_exposed_in_statusline_or_shortcuts() {
    App::test((), |mut app| async move {
        app.update(crate::keybindings::init);
        let fixture = focus_test_fixture(&mut app);
        let (view, _) = add_focus_test_session(&mut app, &fixture, true);
        view.update(&mut app, |view, ctx| {
            view.suggestions_mode.update(ctx, |mode, ctx| {
                mode.set_mode(
                    TuiInputSuggestionsMode::ReadOnlyMenu(TuiReadOnlyMenuKind::Shortcuts),
                    ctx,
                );
            });
        });

        app.read(|ctx| {
            assert!(
                ctx.editable_bindings().all(|binding| {
                    binding.description.in_context(DescriptionContext::Default)
                        != "Toggle Auto Queue"
                }),
                "auto-queue toggle binding must not be registered"
            );
        });
        let rendered = render_session(&mut app, &view, 80, 24).join("\n");
        assert!(!rendered.contains("toggle auto-queue"), "{rendered}");
        assert!(!rendered.contains('↳'), "{rendered}");
    });
}
#[test]
fn footer_model_label_is_a_bounded_click_target() {
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        let (view, _) = add_focus_test_session(&mut app, &fixture, true);
        // Force the bootstrap (Disabled) state so the footer — and its
        // clickable model label — render deterministically.
        view.update(&mut app, |view, _| {
            view.terminal_model.lock().block_list_mut().reinit_shell();
        });

        let model_name = view.read(&app, |view, ctx| {
            LLMPreferences::as_ref(ctx)
                .get_active_base_model(ctx, Some(view.terminal_surface_id))
                .display_name
                .clone()
        });
        let (mut element, scene, buffer) = render_retained_session(&app, &view, 80, 40);
        let (label_col, label_row) = footer_label_position(&buffer, &model_name);
        let inside = (label_col + 1, label_row);
        let outside = (0, label_row);

        assert!(!view.read(&app, |v, _| {
            v.model_label_hover.lock().unwrap().is_hovered()
        }));
        // Hovering onto the label marks the retained handle as hovered.
        dispatch_session_event(
            &app,
            &view,
            &mut element,
            scene.clone(),
            &mouse_moved(inside.0, inside.1),
        );
        assert!(view.read(&app, |v, _| {
            v.model_label_hover.lock().unwrap().is_hovered()
        }));
        // Hovering back off (into the left footer slot) clears it.
        dispatch_session_event(
            &app,
            &view,
            &mut element,
            scene.clone(),
            &mouse_moved(outside.0, outside.1),
        );
        assert!(!view.read(&app, |v, _| {
            v.model_label_hover.lock().unwrap().is_hovered()
        }));

        // A press inside the label arms the pending click and is consumed.
        assert!(dispatch_session_event(
            &app,
            &view,
            &mut element,
            scene.clone(),
            &left_mouse_down(inside.0, inside.1)
        ));
        assert!(view.read(&app, |v, _| {
            v.model_label_hover.lock().unwrap().is_clicked()
        }));
        // Releasing inside disarms (the click handler dispatches ToggleModelMenu).
        assert!(dispatch_session_event(
            &app,
            &view,
            &mut element,
            scene.clone(),
            &left_mouse_up(inside.0, inside.1)
        ));
        assert!(!view.read(&app, |v, _| {
            v.model_label_hover.lock().unwrap().is_clicked()
        }));

        // A press outside the label does not arm and is not consumed.
        assert!(!dispatch_session_event(
            &app,
            &view,
            &mut element,
            scene.clone(),
            &left_mouse_down(outside.0, outside.1)
        ));
        assert!(!view.read(&app, |v, _| {
            v.model_label_hover.lock().unwrap().is_clicked()
        }));
        // A following release outside does not fire a click.
        assert!(!dispatch_session_event(
            &app,
            &view,
            &mut element,
            scene.clone(),
            &left_mouse_up(outside.0, outside.1)
        ));
    });
}

fn focus_test_fixture(app: &mut App) -> FocusTestFixture {
    register_tui_session_view_test_singletons(app);
    add_test_semantic_selection(app);
    app.update(TuiAutoupdater::register);
    let (window_id, _) = app.update(|ctx| {
        ctx.add_tui_window(
            AddWindowOptions {
                window_style: WindowStyle::NotStealFocus,
                ..Default::default()
            },
            |_| RootTuiView::new(),
        )
    });
    let sessions = app.add_singleton_model(|_| TuiSessions::new_for_test());
    let orchestration = app.update(TuiOrchestrationModel::register);
    app.update(|ctx| TuiSessions::wire_orchestration(&sessions, &orchestration, ctx));
    FocusTestFixture {
        window_id,
        sessions,
    }
}

fn add_focus_test_session(
    app: &mut App,
    fixture: &FocusTestFixture,
    focus: bool,
) -> (ViewHandle<super::TuiTerminalSessionView>, TuiSessionId) {
    let (view, manager) = add_test_terminal_session(app, fixture.window_id);
    let session_id = app.update(|ctx| {
        TuiSessions::register_session(&fixture.sessions, view.clone(), manager, focus, ctx)
    });
    (view, session_id)
}

fn add_first_run_onboarding_test_session(
    app: &mut App,
    fixture: &FocusTestFixture,
    focus: bool,
) -> (ViewHandle<super::TuiTerminalSessionView>, TuiSessionId) {
    let (view, manager) =
        add_test_terminal_session_with_first_run_onboarding(app, fixture.window_id);
    let session_id = app.update(|ctx| {
        TuiSessions::register_session(&fixture.sessions, view.clone(), manager, focus, ctx)
    });
    (view, session_id)
}

fn add_focus_test_session_with_settings_file_error(
    app: &mut App,
    fixture: &FocusTestFixture,
    error: SettingsFileError,
) -> ViewHandle<super::TuiTerminalSessionView> {
    let (view, manager) =
        add_test_terminal_session_with_settings_file_error(app, fixture.window_id, Some(error));
    app.update(|ctx| {
        TuiSessions::register_session(&fixture.sessions, view.clone(), manager, true, ctx);
    });
    view
}

fn render_element(element: Box<dyn TuiElement>, ctx: &AppContext, width: u16) -> TuiBuffer {
    render_element_with_size(element, ctx, width, 1)
}

fn render_element_with_size(
    mut element: Box<dyn TuiElement>,
    ctx: &AppContext,
    width: u16,
    height: u16,
) -> TuiBuffer {
    let mut rendered_views = EntityIdMap::default();
    let mut layout_ctx = TuiLayoutContext {
        rendered_views: &mut rendered_views,
    };
    let size = element.layout(
        TuiConstraint::loose(TuiSize::new(width, height)),
        &mut layout_ctx,
        ctx,
    );
    let area = TuiRect::new(0, 0, size.width, size.height);
    let mut buffer = TuiBuffer::empty(area);
    let mut paint_ctx = TuiPaintContext::new(&mut rendered_views);
    {
        let mut surface = TuiPaintSurface::new(&mut buffer);
        element.render(
            TuiScreenPosition::new(i32::from(area.x), i32::from(area.y)),
            &mut surface,
            &mut paint_ctx,
        );
    }
    buffer
}
fn render_session(
    app: &mut App,
    view: &ViewHandle<super::TuiTerminalSessionView>,
    width: u16,
    height: u16,
) -> Vec<String> {
    render_session_buffer(app, view, width, height).to_lines()
}

fn render_session_buffer(
    app: &mut App,
    view: &ViewHandle<super::TuiTerminalSessionView>,
    width: u16,
    height: u16,
) -> TuiBuffer {
    render_session_frame(app, view, width, height).buffer
}

fn render_session_frame(
    app: &mut App,
    view: &ViewHandle<super::TuiTerminalSessionView>,
    width: u16,
    height: u16,
) -> TuiFrame {
    let mut presenter = TuiPresenter::new();
    app.update(|ctx| {
        let mut invalidation = WindowInvalidation::default();
        invalidation.updated.insert(view.id());
        invalidation
            .updated
            .extend(view.as_ref(ctx).child_view_ids(ctx));
        presenter.invalidate(&invalidation, ctx, view.window_id(ctx));
        presenter.present(ctx, view, TuiRect::new(0, 0, width, height))
    })
}

fn first_visible_column(line: &str) -> usize {
    line.chars()
        .position(|character| !character.is_whitespace())
        .unwrap_or_else(|| panic!("line must contain visible content: {line:?}"))
}
#[test]
fn input_adjacent_surfaces_follow_figma_outer_edge_alignment() {
    App::test((), |mut app| async move {
        app.update(crate::keybindings::init);
        let fixture = focus_test_fixture(&mut app);
        let (view, _) = add_focus_test_session(&mut app, &fixture, true);

        let lines = render_session(&mut app, &view, 80, 24);
        let input_border_column = lines
            .iter()
            .find(|line| line.contains('▏'))
            .map(|line| first_visible_column(line))
            .unwrap_or_else(|| panic!("input border must render:\n{}", lines.join("\n")));
        let statusline_column = lines
            .iter()
            .find(|line| line.contains("auto (cost-efficient)"))
            .map(|line| first_visible_column(line))
            .unwrap_or_else(|| panic!("statusline must render:\n{}", lines.join("\n")));
        assert_eq!(
            statusline_column,
            input_border_column,
            "statusline must begin at the input border's outer edge:\n{}",
            lines.join("\n")
        );

        view.update(&mut app, |view, ctx| {
            view.input_view.update(ctx, |input, ctx| {
                input.set_text("/", ctx);
            });
        });
        futures_lite::future::yield_now().await;
        let buffer = render_session_buffer(&mut app, &view, 80, 24);
        let lines = buffer.to_lines();
        let (slash_command_row, slash_command_column) = lines
            .iter()
            .enumerate()
            .find(|(_, line)| line.contains("/agent"))
            .map(|(row, line)| (row, first_visible_column(line)))
            .unwrap_or_else(|| panic!("slash-command menu must render:\n{}", lines.join("\n")));
        assert_eq!(
            slash_command_column,
            input_border_column,
            "inline-menu content must begin at the input border's outer edge:\n{}",
            lines.join("\n")
        );
        assert_eq!(
            buffer[(input_border_column as u16, slash_command_row as u16)].bg,
            app.read(|ctx| TuiUiBuilder::from_app(ctx).slash_command_selection_background()),
            "selected inline-menu background must begin at the input border's outer edge"
        );

        view.update(&mut app, |view, ctx| {
            view.suggestions_mode.update(ctx, |mode, ctx| {
                mode.set_mode(
                    TuiInputSuggestionsMode::ReadOnlyMenu(TuiReadOnlyMenuKind::Shortcuts),
                    ctx,
                );
            });
        });
        let buffer = render_session_buffer(&mut app, &view, 80, 24);
        let lines = buffer.to_lines();
        let (shortcuts_row, shortcuts_column) = lines
            .iter()
            .enumerate()
            .find(|(_, line)| line.contains("Shortcuts"))
            .map(|(row, line)| (row, first_visible_column(line)))
            .unwrap_or_else(|| panic!("shortcuts surface must render:\n{}", lines.join("\n")));
        assert_eq!(
            shortcuts_column,
            input_border_column + 1,
            "shortcuts text keeps its designed one-cell internal padding:\n{}",
            lines.join("\n")
        );
        assert_eq!(
            buffer[(input_border_column as u16, shortcuts_row as u16)].bg,
            app.read(|ctx| TuiUiBuilder::from_app(ctx).read_only_menu_background()),
            "shortcuts background must begin at the input border's outer edge"
        );
    });
}

#[test]
fn shortcuts_surface_renders_above_the_input() {
    App::test((), |mut app| async move {
        app.update(crate::keybindings::init);
        let fixture = focus_test_fixture(&mut app);
        let (view, _) = add_focus_test_session(&mut app, &fixture, true);
        view.update(&mut app, |view, ctx| {
            view.suggestions_mode.update(ctx, |mode, ctx| {
                mode.set_mode(
                    TuiInputSuggestionsMode::ReadOnlyMenu(TuiReadOnlyMenuKind::Shortcuts),
                    ctx,
                );
            });
        });

        let rendered = render_session(&mut app, &view, 80, 24).join("\n");
        assert!(rendered.contains("Shortcuts"), "{rendered}");
        assert!(rendered.contains("? shortcuts"), "{rendered}");
        assert!(rendered.contains("/ commands"), "{rendered}");
        assert!(rendered.contains("! shell mode"), "{rendered}");
        assert!(rendered.contains("← conversations"), "{rendered}");
        assert!(rendered.contains("↑ input history"), "{rendered}");
        assert!(!rendered.contains("toggle auto-queue"), "{rendered}");
        assert!(rendered.contains("toggle auto-approve"), "{rendered}");
        // The shortcuts panel must NOT include the status section (that
        // lives in the dedicated status menu opened by /status).
        assert!(
            !rendered.contains("Version"),
            "Shortcuts panel must not show Version:\n{rendered}"
        );
        assert!(
            !rendered.contains("Working directory"),
            "Shortcuts panel must not show Working directory:\n{rendered}"
        );

        view.update(&mut app, |view, ctx| {
            view.handle_action(
                &TuiTerminalSessionAction::ToggleAutoApprove {
                    show_feedback: true,
                },
                ctx,
            );
        });
        let rendered = render_session(&mut app, &view, 80, 24).join("\n");
        assert!(rendered.contains("toggle auto-approve"), "{rendered}");
        assert!(rendered.contains(AUTO_APPROVE_ENABLED_HINT), "{rendered}");

        let narrow = render_session(&mut app, &view, 40, 24).join("\n");
        assert!(narrow.contains("Shortcuts"), "{narrow}");
        assert!(narrow.contains("? shortcuts"), "{narrow}");
    });
}
fn input_text(view: &ViewHandle<super::TuiTerminalSessionView>, ctx: &AppContext) -> String {
    view.as_ref(ctx)
        .input_view
        .as_ref(ctx)
        .model()
        .as_ref(ctx)
        .content()
        .as_ref(ctx)
        .text()
        .into_string()
}

#[test]
fn typeahead_event_inserts_and_overwrites_the_tui_input() {
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        let (view, _) = add_focus_test_session(&mut app, &fixture, true);

        view.update(&mut app, |view, ctx| {
            {
                let mut model = view.terminal_model.lock();
                model.simulate_long_running_block("sleep 5", "");
                model.finish_block();
                model.input_buffer(InputBufferValue {
                    buffer: "ec".to_owned(),
                    session_id: None,
                });
            }
            view.handle_typeahead_event(ctx);
        });
        assert_eq!(app.read(|ctx| input_text(&view, ctx)), "ec");

        view.update(&mut app, |view, ctx| {
            view.terminal_model.lock().input_buffer(InputBufferValue {
                buffer: "echo hi".to_owned(),
                session_id: None,
            });
            view.handle_typeahead_event(ctx);
        });
        assert_eq!(app.read(|ctx| input_text(&view, ctx)), "echo hi");
    });
}

#[test]
fn empty_typeahead_event_leaves_the_tui_input_unchanged() {
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        let (view, _) = add_focus_test_session(&mut app, &fixture, true);
        view.update(&mut app, |view, ctx| {
            view.input_view.update(ctx, |input, ctx| {
                input.set_text("draft", ctx);
            });
            {
                let mut model = view.terminal_model.lock();
                model.simulate_long_running_block("sleep 5", "");
                model.finish_block();
            }
            view.handle_typeahead_event(ctx);
        });

        assert_eq!(app.read(|ctx| input_text(&view, ctx)), "draft");
    });
}

#[test]
fn nld_slash_command_toggles_and_reports_its_effects() {
    App::test((), |mut app| async move {
        let _agent_mode = warp_core::features::FeatureFlag::AgentMode.override_enabled(true);
        let fixture = focus_test_fixture(&mut app);
        let (view, _) = add_focus_test_session(&mut app, &fixture, true);
        flush_events();

        view.update(&mut app, |view, ctx| {
            view.input_view.update(ctx, |input, ctx| {
                input.set_text("/natural-language-detection", ctx);
            });
            view.execute_tui_slash_command(&slash_commands::NATURAL_LANGUAGE_DETECTION, None, ctx);
        });

        assert!(app.read(|ctx| {
            *AISettings::as_ref(ctx)
                .ai_autodetection_enabled_internal
                .value()
        }));
        assert_eq!(app.read(|ctx| input_text(&view, ctx)), "");
        assert_eq!(
            view.read(&app, |view, _| {
                view.transient_hint
                    .current()
                    .map(|(text, tone)| (text.to_owned(), tone))
            }),
            Some((
                "Natural language detection enabled.".to_owned(),
                TransientHintTone::Success
            ))
        );

        view.update(&mut app, |view, ctx| {
            view.input_view.update(ctx, |input, ctx| {
                input.set_text("/natural-language-detection", ctx);
            });
            view.execute_tui_slash_command(&slash_commands::NATURAL_LANGUAGE_DETECTION, None, ctx);
        });
        futures_lite::future::yield_now().await;

        assert!(!app.read(|ctx| {
            *AISettings::as_ref(ctx)
                .ai_autodetection_enabled_internal
                .value()
        }));
        assert_eq!(app.read(|ctx| input_text(&view, ctx)), "");
        assert_eq!(
            view.read(&app, |view, _| {
                view.transient_hint
                    .current()
                    .map(|(text, tone)| (text.to_owned(), tone))
            }),
            Some((
                "Natural language detection disabled.".to_owned(),
                TransientHintTone::Success
            ))
        );

        let deadline = Instant::now() + Duration::from_secs(5);
        let mut toggles = Vec::new();
        while toggles.len() < 2 {
            toggles.extend(
                flush_events()
                    .into_iter()
                    .filter_map(|event| match event.payload {
                        EventPayload::NamedEvent {
                            name,
                            value: Some(value),
                            ..
                        } if name == "AgentMode.ToggleAutoDetectionSetting" => Some(value),
                        _ => None,
                    }),
            );
            if toggles.len() >= 2 || Instant::now() >= deadline {
                break;
            }
            Timer::after(Duration::from_millis(10)).await;
        }
        assert_eq!(toggles.len(), 2);
        assert_eq!(
            toggles[0],
            serde_json::json!({
                "is_autodetection_enabled": true,
                "origin": "slash_command",
            })
        );
        assert_eq!(
            toggles[1],
            serde_json::json!({
                "is_autodetection_enabled": false,
                "origin": "slash_command",
            })
        );
    });
}

#[test]
fn status_slash_command_opens_dedicated_status_menu_via_shared_structure() {
    App::test((), |mut app| async move {
        app.update(crate::keybindings::init);
        let fixture = focus_test_fixture(&mut app);
        let (view, _) = add_focus_test_session(&mut app, &fixture, true);

        view.update(&mut app, |view, ctx| {
            view.input_view.update(ctx, |input, ctx| {
                input.set_text("/status", ctx);
            });
            view.execute_tui_slash_command(&slash_commands::STATUS, None, ctx);
        });

        // /status and ? select distinct projections of the shared read-only
        // menu component.
        assert!(
            app.read(|ctx| {
                matches!(
                    view.as_ref(ctx).suggestions_mode.as_ref(ctx).mode(),
                    TuiInputSuggestionsMode::ReadOnlyMenu(TuiReadOnlyMenuKind::Status)
                )
            }),
            "/status should open the dedicated status overlay (Status mode)"
        );
        assert!(
            app.read(|ctx| {
                !matches!(
                    view.as_ref(ctx).suggestions_mode.as_ref(ctx).mode(),
                    TuiInputSuggestionsMode::ReadOnlyMenu(TuiReadOnlyMenuKind::Shortcuts)
                )
            }),
            "/status must NOT open the shortcuts panel (Shortcuts mode)"
        );

        // The full session render must show the six status fields through the
        // shared read-only panel structure.
        let rendered = render_session(&mut app, &view, 80, 24).join("\n");
        assert!(
            rendered.contains("Status"),
            "Status section header:\n{rendered}"
        );
        assert!(rendered.contains("Version"), "Version row:\n{rendered}");
        assert!(rendered.contains("Session"), "Session row:\n{rendered}");
        assert!(
            rendered.contains("Conversation ID"),
            "Conversation ID row:\n{rendered}"
        );
        assert!(
            rendered.contains("Working directory"),
            "Working directory row:\n{rendered}"
        );
        assert!(rendered.contains("Org"), "Org row:\n{rendered}");
        assert!(rendered.contains("Email"), "Email row:\n{rendered}");

        // The fixture signs in as the test user, so the panel surfaces that
        // email. No workspace is loaded (Org degrades to the em-dash placeholder)
        // and there is no conversation yet (Session falls back to "Untitled").
        assert!(
            rendered.contains("test_user@warp.dev"),
            "Email value:\n{rendered}"
        );
        assert!(rendered.contains("Untitled"), "Session value:\n{rendered}");
        // Em dash (—) appears as the Org placeholder.
        assert!(
            rendered.contains("\u{2014}"),
            "Org placeholder (em dash):\n{rendered}"
        );

        // The dedicated status menu does NOT include keyboard shortcut rows.
        assert!(
            !rendered.contains("? shortcuts"),
            "Status menu must not include shortcuts rows:\n{rendered}"
        );

        // Dismissing the panel closes the Status overlay.
        view.update(&mut app, |view, ctx| {
            view.suggestions_mode.update(ctx, |mode, ctx| {
                mode.close_if_active(
                    TuiInputSuggestionsMode::ReadOnlyMenu(TuiReadOnlyMenuKind::Status),
                    ctx,
                );
            });
        });
        assert!(
            !app.read(|ctx| matches!(
                view.as_ref(ctx).suggestions_mode.as_ref(ctx).mode(),
                TuiInputSuggestionsMode::ReadOnlyMenu(TuiReadOnlyMenuKind::Status)
            )),
            "dismissing status mode should close the panel"
        );
    });
}

#[test]
fn status_conversation_id_uses_the_selected_id_or_none() {
    let conversation_id = AIConversationId::new();
    assert_eq!(
        super::format_status_conversation_id(Some(conversation_id)),
        conversation_id.to_string()
    );
    assert_eq!(super::format_status_conversation_id(None), "None");
}

#[test]
fn user_info_updates_only_require_an_open_status_menu_repaint() {
    assert!(!super::status_menu_is_open(TuiInputSuggestionsMode::Closed));
    assert!(!super::status_menu_is_open(
        TuiInputSuggestionsMode::ReadOnlyMenu(TuiReadOnlyMenuKind::Shortcuts)
    ));
    assert!(super::status_menu_is_open(
        TuiInputSuggestionsMode::ReadOnlyMenu(TuiReadOnlyMenuKind::Status)
    ));
}
#[test]
fn bootstrap_renders_starting_shell_above_input() {
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        let (view, _) = add_focus_test_session(&mut app, &fixture, true);
        view.update(&mut app, |view, _| {
            view.terminal_model.lock().block_list_mut().reinit_shell();
        });

        let lines = render_session(&mut app, &view, 80, 40);
        let status_index = lines
            .iter()
            .position(|line| line.trim() == "Starting shell...")
            .unwrap_or_else(|| panic!("bootstrap status should render:\n{}", lines.join("\n")));
        let input_index = lines
            .iter()
            .enumerate()
            .skip(status_index + 1)
            .find(|(_, line)| line.contains('▏') || line.contains('▁') || line.contains('─'))
            .map(|(index, _)| index)
            .expect("bootstrap input border should render below the status");
        assert!(status_index < input_index);
    });
}

#[test]
fn zero_state_position_stays_stable_across_shell_bootstrap() {
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        let (view, _) = add_focus_test_session(&mut app, &fixture, true);

        let ready_lines = render_session(&mut app, &view, 80, 40);
        assert!(
            ready_lines
                .iter()
                .all(|line| line.trim() != "Starting shell..."),
            "ready state must not render the bootstrap hint:\n{}",
            ready_lines.join("\n")
        );

        view.update(&mut app, |view, _| {
            view.terminal_model.lock().block_list_mut().reinit_shell();
        });
        let bootstrap_lines = render_session(&mut app, &view, 80, 40);
        assert!(
            bootstrap_lines
                .iter()
                .any(|line| line.trim() == "Starting shell..."),
            "bootstrap state must render the starting-shell hint:\n{}",
            bootstrap_lines.join("\n")
        );

        let title_row = |lines: &[String]| {
            lines
                .iter()
                .position(|line| line.contains("Warp Agent CLI"))
                .unwrap_or_else(|| panic!("zero-state title should render:\n{}", lines.join("\n")))
        };
        assert_eq!(
            title_row(&bootstrap_lines),
            title_row(&ready_lines),
            "zero state must not shift when the bootstrap hint disappears\nbootstrap:\n{}\nready:\n{}",
            bootstrap_lines.join("\n"),
            ready_lines.join("\n")
        );
    });
}

/// The input child's rendered element is cached by the presenter, and
/// transcript emptiness can flip without any input-owned event (a terminal
/// block landing via the PTY wakeup path only invalidates the session view).
/// The placeholder hint must still switch off the zero-state copy because the
/// provider re-resolves on every layout pass.
#[test]
fn agent_hint_tracks_transcript_emptiness_without_input_invalidation() {
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        let (view, _) = add_focus_test_session(&mut app, &fixture, true);
        let mut presenter = TuiPresenter::new();

        // Initial full present: every child renders once and is cached.
        let lines = app.update(|ctx| {
            let mut invalidation = WindowInvalidation::default();
            invalidation.updated.insert(view.id());
            invalidation
                .updated
                .extend(view.as_ref(ctx).child_view_ids(ctx));
            presenter.invalidate(&invalidation, ctx, view.window_id(ctx));
            presenter
                .present(ctx, &view, TuiRect::new(0, 0, 100, 40))
                .buffer
                .to_lines()
        });
        assert!(
            lines
                .iter()
                .any(|line| line.contains("← for conversations")),
            "zero state should show the zero-state hint:\n{}",
            lines.join("\n")
        );

        // A finished terminal block lands without any input-owned event; only
        // the session view is invalidated, mirroring the PTY wakeup path.
        view.update(&mut app, |view, _| {
            let mut model = view.terminal_model.lock();
            model
                .block_list_mut()
                .set_transcript_scope(TranscriptScope::Unfiltered);
            model.simulate_block("echo hi", "hi\r\n");
        });
        let lines = app.update(|ctx| {
            let mut invalidation = WindowInvalidation::default();
            invalidation.updated.insert(view.id());
            presenter.invalidate(&invalidation, ctx, view.window_id(ctx));
            presenter
                .present(ctx, &view, TuiRect::new(0, 0, 100, 40))
                .buffer
                .to_lines()
        });
        assert!(
            !lines
                .iter()
                .any(|line| line.contains("← for conversations")),
            "the cached input element must drop the zero-state hint:\n{}",
            lines.join("\n")
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("Ask the agent anything")),
            "the started-conversation hint should render:\n{}",
            lines.join("\n")
        );
    });
}

#[test]
fn submit_is_blocked_during_bootstrap_and_allowed_at_prompt() {
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        let (view, _) = add_focus_test_session(&mut app, &fixture, true);

        view.update(&mut app, |view, ctx| {
            view.input_view.update(ctx, |input, ctx| {
                input.set_text("draft", ctx);
            });
            view.terminal_model.lock().block_list_mut().reinit_shell();
            view.handle_submitted("draft".to_owned(), None, ctx);
        });

        assert_eq!(
            app.read(|ctx| input_text(&view, ctx)),
            "draft",
            "bootstrap submission must leave the draft untouched"
        );
        assert!(!view.read(&app, |view, _| {
            view.input_target().agent_editor_owns_input()
        }));
        assert!(TuiInputTarget::AgentEditor.agent_editor_owns_input());
    });
}

#[test]
fn accepted_command_history_executes_through_the_shell_submission_path() {
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        let (view, _) = add_focus_test_session(&mut app, &fixture, true);
        let executed = Rc::new(RefCell::new(Vec::new()));
        app.update(|ctx| {
            let executed = executed.clone();
            ctx.subscribe_to_view(&view, move |_, event, _| {
                if let TuiTerminalSessionEvent::ExecuteCommand(event) = event {
                    executed.borrow_mut().push(event.command.clone());
                }
            });
        });

        view.update(&mut app, |view, ctx| {
            view.handle_accepted_prompt_and_command_history(
                "echo from history".to_owned(),
                TuiUpArrowHistoryItemKind::Command {
                    linked_workflow_data: None,
                },
                ctx,
            );
        });

        assert_eq!(executed.borrow().as_slice(), &["echo from history"]);
        assert_eq!(app.read(|ctx| input_text(&view, ctx)), "");
    });
}

#[test]
fn accepted_command_history_preserves_workflow_metadata() {
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        let (view, _) = add_focus_test_session(&mut app, &fixture, true);
        let executed = Rc::new(RefCell::new(Vec::new()));
        app.update(|ctx| {
            let executed = executed.clone();
            ctx.subscribe_to_view(&view, move |_, event, _| {
                if let TuiTerminalSessionEvent::ExecuteCommand(event) = event {
                    executed.borrow_mut().push((**event).clone());
                }
            });
        });

        view.update(&mut app, |view, ctx| {
            view.handle_accepted_prompt_and_command_history(
                "deploy production".to_owned(),
                TuiUpArrowHistoryItemKind::Command {
                    linked_workflow_data: Some(LinkedWorkflowData::Command(
                        "deploy {{environment}}".to_owned(),
                    )),
                },
                ctx,
            );
        });

        let executed = executed.borrow();
        let event = executed.as_slice().first().expect("command was executed");
        assert_eq!(event.command, "deploy production");
        assert_eq!(event.workflow_id, None);
        assert_eq!(
            event.workflow_command.as_deref(),
            Some("deploy {{environment}}")
        );
    });
}

#[test]
fn accepted_prompt_history_submits_to_the_selected_ai_conversation() {
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        let (view, _) = add_focus_test_session(&mut app, &fixture, true);

        view.update(&mut app, |view, ctx| {
            view.handle_accepted_prompt_and_command_history(
                "explain the build".to_owned(),
                TuiUpArrowHistoryItemKind::Prompt,
                ctx,
            );
        });

        view.read(&app, |view, ctx| {
            let queries = view
                .conversation_selection
                .as_ref(ctx)
                .selected_conversation(ctx)
                .expect("selected conversation")
                .latest_exchange()
                .expect("accepted prompt should append an exchange")
                .input
                .iter()
                .filter_map(|input| input.user_query())
                .collect::<Vec<_>>();
            assert_eq!(queries, vec!["explain the build"]);
        });
        assert_eq!(app.read(|ctx| input_text(&view, ctx)), "");
    });
}

#[test]
fn long_running_command_keeps_input_hidden() {
    App::test((), |mut app| async move {
        app.update(crate::keybindings::init);
        let fixture = focus_test_fixture(&mut app);
        let (view, _) = add_focus_test_session(&mut app, &fixture, true);
        view.update(&mut app, |view, ctx| {
            let mut terminal_model = view.terminal_model.lock();
            terminal_model
                .block_list_mut()
                .set_transcript_scope(TranscriptScope::Unfiltered);
            terminal_model.simulate_block("echo ready", "ready\r\n");
            terminal_model.simulate_long_running_block("cat", "");
            drop(terminal_model);
            assert!(matches!(
                view.session_state(ctx)
                    .expect("session state resolves")
                    .blocking_input_source(),
                Some(&BlockingInputSource::LongRunningCommand)
            ));
            assert!(
                !view.transcript.as_ref(ctx).is_empty(),
                "command output should make the transcript visible"
            );
        });

        let lines = render_session(&mut app, &view, 80, 40);
        assert!(
            !lines
                .iter()
                .any(|line| line.trim_end() == "Starting shell..."),
            "LRC must not render bootstrap status:\n{}",
            lines.join("\n")
        );
        assert!(
            !lines
                .iter()
                .any(|line| line.chars().any(|glyph| "┌┐└┘─│▁▏▕▔".contains(glyph))),
            "LRC must keep the input editor hidden:\n{}",
            lines.join("\n")
        );
        // Manual attachment remains advertised while the running command is visible.
        let hint = view.read(&app, |view, ctx| {
            view.running_command_hint(ctx)
                .expect("visible running command should have an attachment hint")
        });
        assert!(
            lines.iter().any(|line| line.trim() == hint),
            "LRC must render the attach hint row:\n{}",
            lines.join("\n")
        );
        assert_eq!(hint, "Ctrl + Shift + ⏎  to use agent");
        assert!(
            lines
                .iter()
                .all(|line| !line.contains(RUNNING_COMMAND_DETACH_HINT)),
            "LRC must not show the detach hint before agent attachment:\n{}",
            lines.join("\n")
        );
    });
}

#[test]
fn zero_state_running_command_hint_shows_attachment() {
    App::test((), |mut app| async move {
        app.update(crate::keybindings::init);
        let fixture = focus_test_fixture(&mut app);
        let (view, _) = add_focus_test_session(&mut app, &fixture, true);
        view.update(&mut app, |view, ctx| {
            let mut terminal_model = view.terminal_model.lock();
            terminal_model.simulate_long_running_block("cat", "");
            terminal_model
                .block_list_mut()
                .active_block_mut()
                .set_should_hide_command_grid(true);
            drop(terminal_model);

            assert!(
                view.transcript.as_ref(ctx).is_empty(),
                "hidden command without output should preserve zero state"
            );
            assert!(
                view.session_state(ctx)
                    .expect("session state resolves")
                    .user_owns_running_command()
            );
        });

        let lines = render_session(&mut app, &view, 80, 40);
        assert!(
            lines.iter().any(|line| line.contains("Warp Agent CLI")),
            "zero state should remain visible:\n{}",
            lines.join("\n")
        );
        assert!(
            lines.iter().any(|line| line.contains("to use agent")),
            "zero state should preserve manual attachment:\n{}",
            lines.join("\n")
        );
    });
}

#[test]
fn manual_attach_and_detach_switch_running_command_input_ownership() {
    App::test((), |mut app| async move {
        app.update(crate::keybindings::init);
        let fixture = focus_test_fixture(&mut app);
        let (view, _) = add_focus_test_session(&mut app, &fixture, true);
        let interrupt_count = Rc::new(RefCell::new(0));
        let interrupt_count_for_events = interrupt_count.clone();
        app.update(|ctx| {
            ctx.subscribe_to_view(&view, move |_, event, _| {
                if matches!(event, TuiTerminalSessionEvent::InterruptPty) {
                    *interrupt_count_for_events.borrow_mut() += 1;
                }
            });
        });
        view.update(&mut app, |view, ctx| {
            view.input_view.update(ctx, |input, ctx| {
                input.set_text("stale draft", ctx);
            });
            view.terminal_model
                .lock()
                .simulate_long_running_block("cat", "");

            let state = view.session_state(ctx).expect("session state resolves");
            assert!(state.can_attach_agent_to_running_command());
            assert!(
                view.keymap_context(ctx)
                    .set
                    .contains(SESSION_CAN_ATTACH_AGENT_TO_RUNNING_COMMAND_FLAG)
            );

            assert!(view.try_attach_agent_to_running_command(ctx));

            assert!(view.input_target().agent_editor_owns_input());
            assert!(
                view.terminal_model
                    .lock()
                    .block_list()
                    .active_block()
                    .is_agent_tagged_in()
            );
            assert_eq!(
                view.ai_input_model
                    .as_ref(ctx)
                    .last_ai_autodetection_source(),
                Some(InputTypeAutoDetectionSource::AgentTerminalControl)
            );
            assert!(
                view.keymap_context(ctx)
                    .set
                    .contains(SESSION_CAN_DETACH_AGENT_FROM_RUNNING_COMMAND_FLAG)
            );
            view.suggestions_mode.update(ctx, |mode, ctx| {
                mode.set_mode(
                    TuiInputSuggestionsMode::ReadOnlyMenu(TuiReadOnlyMenuKind::Shortcuts),
                    ctx,
                );
            });
            assert!(
                !view
                    .keymap_context(ctx)
                    .set
                    .contains(SESSION_CAN_DETACH_AGENT_FROM_RUNNING_COMMAND_FLAG)
            );
            view.suggestions_mode.update(ctx, |mode, ctx| {
                mode.set_mode(TuiInputSuggestionsMode::Closed, ctx);
            });
            assert!(
                view.keymap_context(ctx)
                    .set
                    .contains(SESSION_CAN_DETACH_AGENT_FROM_RUNNING_COMMAND_FLAG)
            );
        });
        assert_eq!(app.read(|ctx| input_text(&view, ctx)), "");

        let lines = render_session(&mut app, &view, 80, 40);
        assert!(
            lines.iter().any(|line| line.contains('▏')),
            "tagging in should render the composer:\n{}",
            lines.join("\n")
        );
        assert!(
            lines
                .iter()
                .any(|line| line.trim() == RUNNING_COMMAND_DETACH_HINT),
            "tagging in should replace the footer with the detach hint:\n{}",
            lines.join("\n")
        );
        view.update(&mut app, |view, ctx| {
            view.input_view.update(ctx, |input, ctx| {
                input.set_text("unsent agent prompt", ctx);
            });
            view.handle_action(&TuiTerminalSessionAction::Interrupt, ctx);
            assert!(
                !view.exit_confirmation.is_armed(),
                "leaving a tagged LRC must not arm TUI exit"
            );

            assert!(view.input_target().pty_owns_input());
            assert!(
                !view
                    .terminal_model
                    .lock()
                    .block_list()
                    .active_block()
                    .is_agent_tagged_in()
            );
            assert_ne!(
                view.ai_input_model
                    .as_ref(ctx)
                    .last_ai_autodetection_source(),
                Some(InputTypeAutoDetectionSource::AgentTerminalControl)
            );
            assert!(
                !view.try_detach_agent_from_running_command(ctx),
                "detaching an already-detached command should report no transition"
            );
        });
        assert_eq!(
            app.read(|ctx| input_text(&view, ctx)),
            "",
            "detaching must discard an unsent agent prompt"
        );
        assert_eq!(
            *interrupt_count.borrow(),
            0,
            "leaving the tagged composer must not send ctrl-c to the running command"
        );
        let lines = render_session(&mut app, &view, 80, 40);
        assert!(
            lines
                .iter()
                .all(|line| !line.contains(RUNNING_COMMAND_DETACH_HINT)),
            "ctrl-c should remove the detach footer hint:\n{}",
            lines.join("\n")
        );
        assert!(
            lines.iter().any(|line| line.contains("to use agent")),
            "detaching should restore the attach hint:\n{}",
            lines.join("\n")
        );
        view.update(&mut app, |view, ctx| {
            let block_id = {
                let mut terminal_model = view.terminal_model.lock();
                let block_id = terminal_model.block_list().active_block().id().clone();
                terminal_model.finish_block();
                block_id
            };
            view.handle_block_completed(&block_id, ctx);
            assert!(view.input_target().agent_editor_owns_input());
        });
        assert_eq!(
            app.read(|ctx| input_text(&view, ctx)),
            "",
            "the discarded prompt must not reappear after command completion"
        );
    });
}

#[test]
fn running_command_completion_clears_transient_attachment_lock() {
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        let (view, _) = add_focus_test_session(&mut app, &fixture, true);
        view.update(&mut app, |view, ctx| {
            view.terminal_model
                .lock()
                .simulate_long_running_block("sleep 1", "");
            view.handle_action(&TuiTerminalSessionAction::AttachAgentToRunningCommand, ctx);
            assert_eq!(
                view.ai_input_model
                    .as_ref(ctx)
                    .last_ai_autodetection_source(),
                Some(InputTypeAutoDetectionSource::AgentTerminalControl)
            );

            let block_id = {
                let mut terminal_model = view.terminal_model.lock();
                let block_id = terminal_model.block_list().active_block().id().clone();
                terminal_model.finish_block();
                block_id
            };
            view.handle_block_completed(&block_id, ctx);

            assert_ne!(
                view.ai_input_model
                    .as_ref(ctx)
                    .last_ai_autodetection_source(),
                Some(InputTypeAutoDetectionSource::AgentTerminalControl)
            );
            assert!(view.input_target().agent_editor_owns_input());
        });
    });
}

#[test]
fn tagged_in_alt_screen_keeps_output_and_composer_visible() {
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        let (view, _) = add_focus_test_session(&mut app, &fixture, true);
        let interrupt_count = Rc::new(RefCell::new(0));
        let interrupt_count_for_events = interrupt_count.clone();
        app.update(|ctx| {
            ctx.subscribe_to_view(&view, move |_, event, _| {
                if matches!(event, TuiTerminalSessionEvent::InterruptPty) {
                    *interrupt_count_for_events.borrow_mut() += 1;
                }
            });
        });
        view.update(&mut app, |view, ctx| {
            let mut terminal_model = view.terminal_model.lock();
            terminal_model.simulate_long_running_block("vim", "");
            terminal_model.set_mode(Mode::SwapScreen {
                save_cursor_and_clear_screen: true,
            });
            for character in "TAGGED ALT SCREEN".chars() {
                terminal_model.alt_screen_mut().input(character);
            }
            drop(terminal_model);
            view.handle_action(&TuiTerminalSessionAction::AttachAgentToRunningCommand, ctx);
        });

        assert!(view.read(&app, |view, _| {
            view.input_target().agent_editor_owns_input()
        }));
        let lines = render_session(&mut app, &view, 80, 12);
        let compact_output = lines
            .iter()
            .flat_map(|line| line.chars())
            .filter(|character| !character.is_whitespace())
            .collect::<String>();
        assert!(
            compact_output.contains("TAGGEDALTSCREEN"),
            "tagged-in alternate-screen output should remain visible:\n{}",
            lines.join("\n")
        );
        assert!(
            lines.iter().any(|line| line.contains('▏')),
            "tagged-in alternate screen should render the composer:\n{}",
            lines.join("\n")
        );
        assert!(
            lines
                .iter()
                .any(|line| line.trim() == RUNNING_COMMAND_DETACH_HINT),
            "tagged-in alternate screen should show the detach footer hint:\n{}",
            lines.join("\n")
        );
        view.update(&mut app, |view, ctx| {
            view.handle_action(&TuiTerminalSessionAction::Interrupt, ctx);
            assert!(
                !view.exit_confirmation.is_armed(),
                "leaving a tagged alternate-screen LRC must not arm TUI exit"
            );
            assert!(view.input_target().pty_owns_input());
            assert!(
                !view
                    .terminal_model
                    .lock()
                    .block_list()
                    .active_block()
                    .is_agent_tagged_in()
            );
        });
        assert_eq!(
            *interrupt_count.borrow(),
            0,
            "leaving the tagged composer must not send ctrl-c to the alternate-screen command"
        );
    });
}
#[test]
fn agent_controlled_alt_screen_keeps_output_and_composer_visible() {
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        let (view, _) = add_focus_test_session(&mut app, &fixture, true);
        view.update(&mut app, |view, _| {
            let mut terminal_model = view.terminal_model.lock();
            terminal_model.simulate_long_running_block("vim", "");
            let conversation_id = AIConversationId::new();
            let task_id = TaskId::new("alt-screen-terminal-use".to_owned());
            let block = terminal_model.block_list_mut().active_block_mut();
            block.set_agent_interaction_mode_for_requested_command(
                AIAgentActionId::from("alt-screen-command".to_owned()),
                Some(task_id.clone()),
                conversation_id,
            );
            block
                .set_agent_interaction_mode_for_agent_monitored_command(&task_id, conversation_id)
                .expect("command should become agent monitored");
            terminal_model.set_mode(Mode::SwapScreen {
                save_cursor_and_clear_screen: true,
            });
            for character in "ALT SCREEN".chars() {
                terminal_model.alt_screen_mut().input(character);
            }
        });

        assert!(view.read(&app, |view, _| {
            view.input_target().agent_editor_owns_input()
        }));
        let lines = render_session(&mut app, &view, 80, 12);
        let compact_output = lines
            .iter()
            .flat_map(|line| line.chars())
            .filter(|character| !character.is_whitespace())
            .collect::<String>();
        assert!(
            compact_output.contains("ALTSCREEN"),
            "alternate-screen output should remain visible:\n{}",
            lines.join("\n")
        );
        let alt_screen_row = lines
            .iter()
            .position(|line| line.contains("ALT"))
            .expect("alternate-screen output should start in the output area");
        let input_row = lines
            .iter()
            .position(|line| line.contains('▏'))
            .expect("agent-controlled alternate screen should render the composer");
        assert!(
            alt_screen_row < input_row,
            "alternate-screen output should render above the composer:\n{}",
            lines.join("\n")
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("auto (cost-efficient)")),
            "the normal agent footer should remain visible:\n{}",
            lines.join("\n")
        );
    });
}

#[test]
fn user_controlled_alt_screen_keeps_full_session_input_on_the_pty() {
    App::test((), |mut app| async move {
        app.update(crate::keybindings::init);
        let fixture = focus_test_fixture(&mut app);
        let (view, _) = add_focus_test_session(&mut app, &fixture, true);
        view.update(&mut app, |view, _| {
            let mut terminal_model = view.terminal_model.lock();
            terminal_model.simulate_long_running_block("vim", "");
            terminal_model.set_mode(Mode::SwapScreen {
                save_cursor_and_clear_screen: true,
            });
            for character in "USER ALT SCREEN".chars() {
                terminal_model.alt_screen_mut().input(character);
            }
        });

        assert!(view.read(&app, |view, _| view.input_target().pty_owns_input()));
        let lines = render_session(&mut app, &view, 80, 12);
        let compact_output = lines
            .iter()
            .flat_map(|line| line.chars())
            .filter(|character| !character.is_whitespace())
            .collect::<String>();
        assert!(
            compact_output.contains("USERALTSCREEN"),
            "alternate-screen output should render:\n{}",
            lines.join("\n")
        );
        assert!(
            !lines.iter().any(|line| {
                line.chars().any(|glyph| "┌┐└┘─│▁▏▕▔".contains(glyph))
                    || line.contains("auto (cost-efficient)")
            }),
            "user-controlled alternate screen should not render the agent composer:\n{}",
            lines.join("\n")
        );
        let hint = view.read(&app, |view, ctx| {
            view.running_command_hint(ctx)
                .expect("alternate screen should have a running-command hint")
        });
        assert!(
            lines.iter().any(|line| line.trim() == hint),
            "user-controlled alternate screen should render the attach hint:\n{}",
            lines.join("\n")
        );
    });
}

#[test]
fn stale_user_pty_bytes_are_dropped_after_agent_takes_control_or_is_tagged_in() {
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        let (view, _) = add_focus_test_session(&mut app, &fixture, true);
        let writes = Rc::new(RefCell::new(Vec::new()));
        let writes_for_events = writes.clone();
        app.update(|ctx| {
            ctx.subscribe_to_view(&view, move |_, event, _| {
                if let TuiTerminalSessionEvent::WriteUserInput(bytes) = event {
                    writes_for_events.borrow_mut().push(bytes.to_vec());
                }
            });
        });

        view.update(&mut app, |view, ctx| {
            view.handle_action(
                &TuiTerminalSessionAction::ForwardUserPtyBytes(b"user".to_vec()),
                ctx,
            );
            let mut terminal_model = view.terminal_model.lock();
            terminal_model.simulate_long_running_block("vim", "");
            terminal_model
                .block_list_mut()
                .active_block_mut()
                .set_is_agent_tagged_in(true);
            drop(terminal_model);
            view.handle_action(
                &TuiTerminalSessionAction::ForwardUserPtyBytes(b"tagged".to_vec()),
                ctx,
            );
            let mut terminal_model = view.terminal_model.lock();
            let conversation_id = AIConversationId::new();
            let task_id = TaskId::new("stale-pty-write".to_owned());
            terminal_model
                .block_list_mut()
                .active_block_mut()
                .set_agent_interaction_mode_for_requested_command(
                    AIAgentActionId::from("stale-pty-command".to_owned()),
                    Some(task_id.clone()),
                    conversation_id,
                );
            terminal_model
                .block_list_mut()
                .active_block_mut()
                .set_agent_interaction_mode_for_agent_monitored_command(&task_id, conversation_id)
                .expect("command should become agent monitored");
            drop(terminal_model);
            view.handle_action(
                &TuiTerminalSessionAction::ForwardUserPtyBytes(b"agent".to_vec()),
                ctx,
            );
        });

        assert_eq!(*writes.borrow(), vec![b"user".to_vec()]);
    });
}
/// Visible startup-script execution also routes input to the PTY, but it is
/// not a user-controlled command: the running-command hint row must not appear.
#[test]
fn visible_startup_script_shows_no_running_command_hint() {
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        let (view, _) = add_focus_test_session(&mut app, &fixture, true);
        view.update(&mut app, |view, ctx| {
            {
                let mut terminal_model = view.terminal_model.lock();
                terminal_model.block_list_mut().reinit_shell();
                terminal_model
                    .update_blockheight_items(TRANSCRIPT_BLOCK_SPACING.block_padding, 0.0);
                // Advance past WarpInput, then leave an unfinished startup-script
                // block with visible output owning PTY input.
                terminal_model.simulate_block("bootstrap", "");
                terminal_model.simulate_long_running_block("shell init", "startup output\r\n");
            }
            assert!(
                !view
                    .session_state(ctx)
                    .expect("session state resolves")
                    .can_attach_agent_to_running_command()
            );
            assert!(
                !view
                    .keymap_context(ctx)
                    .set
                    .contains(SESSION_CAN_ATTACH_AGENT_TO_RUNNING_COMMAND_FLAG)
            );
            assert!(
                !view.try_attach_agent_to_running_command(ctx),
                "startup-script input is not an attachable user LRC"
            );
            assert!(view.input_target().pty_owns_input());
        });
        assert!(
            view.read(&app, |view, _| view.input_target().pty_owns_input()),
            "fixture should route input to the PTY during the visible startup script"
        );

        let lines = render_session(&mut app, &view, 80, 40);
        assert!(
            !lines.iter().any(|line| line.contains("to use agent")),
            "startup-script execution must not advertise agent attachment:\n{}",
            lines.join("\n")
        );
    });
}

#[test]
fn zero_state_renders_with_only_zero_height_bootstrap_blocks() {
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        let (view, _) = add_focus_test_session(&mut app, &fixture, true);
        view.update(&mut app, |view, _| {
            let mut terminal_model = view.terminal_model.lock();
            terminal_model.block_list_mut().reinit_shell();
            terminal_model.update_blockheight_items(TRANSCRIPT_BLOCK_SPACING.block_padding, 0.0);
            terminal_model.simulate_block("bootstrap", "");
            terminal_model.simulate_long_running_block("shell init", "");
            let bootstrap_block_id = terminal_model.block_list().active_block().id().clone();
            terminal_model.finish_block();
            let bootstrap_block = terminal_model
                .block_list_mut()
                .mut_block_from_id(&bootstrap_block_id)
                .expect("bootstrap block should remain in the block list");
            bootstrap_block.set_should_hide_command_grid(true);
            terminal_model.update_blockheight_items(
                BlockPadding {
                    bottom: 1.0,
                    ..TRANSCRIPT_BLOCK_SPACING.block_padding
                },
                0.0,
            );

            let block_list = terminal_model.block_list();
            let bootstrap_block = block_list
                .block_with_id(&bootstrap_block_id)
                .expect("bootstrap block should remain in the block list");
            assert!(
                should_render_terminal_block(bootstrap_block, block_list),
                "fixture should contain an eligible shell bootstrap block"
            );
            assert!(
                block_content_rows(bootstrap_block).is_empty(),
                "fixture bootstrap block should have zero displayed height"
            );
        });
        view.read(&app, |view, ctx| {
            assert!(
                view.transcript.as_ref(ctx).is_empty(),
                "zero-height terminal blocks should leave the transcript empty"
            );
        });

        let mut presenter = TuiPresenter::new();
        let frame = app.update(|ctx| {
            let mut invalidation = WindowInvalidation::default();
            invalidation.updated.insert(view.id());
            invalidation
                .updated
                .extend(view.as_ref(ctx).child_view_ids(ctx));
            presenter.invalidate(&invalidation, ctx, fixture.window_id);
            presenter.present(ctx, &view, TuiRect::new(0, 0, 200, 40))
        });
        let lines = frame.buffer.to_lines();
        let title_row = lines
            .iter()
            .position(|line| line.contains("Warp Agent CLI"))
            .expect("zero state should render the Warp Agent CLI title");
        assert!(
            title_row < 28,
            "zero-state title should render in the transcript area:\n{}",
            lines.join("\n")
        );
        // The 32-column animation panel is centered in the 152 columns left
        // after the 48-column copy region: 48 + (152 - 32) / 2 = 108.
        let animation_start = 108;
        let animation_end = 140;
        assert!(
            lines.iter().take(28).any(|line| line
                .chars()
                .skip(animation_start)
                .take(animation_end - animation_start)
                .any(|character| character != ' ')),
            "animation content should render in the centered remaining-space panel:\n{}",
            lines.join("\n")
        );
        assert!(
            lines.iter().take(28).any(|line| line
                .chars()
                .skip(animation_end)
                .any(|character| character != ' ')),
            "starfield content should extend beyond the centered logo panel:\n{}",
            lines.join("\n")
        );
    });
}

#[test]
fn first_zero_state_stays_hidden_while_markers_load_and_reconciles_without_replacing_session() {
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        app.update(|ctx| {
            TuiOnboardingMarkers::handle(ctx).update(ctx, |markers, ctx| {
                markers.reset_for_account_transition(ctx);
            });
        });
        let (view, session_id) = add_first_run_onboarding_test_session(&mut app, &fixture, true);

        app.read(|ctx| {
            assert!(
                !view
                    .as_ref(ctx)
                    .session_state
                    .as_ref(ctx)
                    .show_first_zero_state()
            );
        });
        let lines = render_session(&mut app, &view, 100, 24);
        assert!(lines.iter().any(|line| line.contains("Warp Agent CLI")));
        assert!(
            lines
                .iter()
                .all(|line| !line.contains("What’s different about Warp"))
        );
        assert!(lines.iter().all(|line| !line.contains("████")));

        app.update(|ctx| {
            TuiOnboardingMarkers::handle(ctx).update(ctx, |markers, ctx| {
                markers.set_ready_for_test(false, false, ctx);
            });
        });
        app.read(|ctx| {
            assert!(
                !view
                    .as_ref(ctx)
                    .session_state
                    .as_ref(ctx)
                    .show_first_zero_state()
            );
            assert_eq!(
                TuiSessions::as_ref(ctx).focused_session_id(),
                Some(session_id)
            );
            assert!(TuiSessions::as_ref(ctx).session(session_id).is_some());
        });
    });
}

#[test]
fn dismissed_pending_zero_state_stays_hidden_but_consumes_ready_marker() {
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        app.update(|ctx| {
            TuiOnboardingMarkers::handle(ctx).update(ctx, |markers, ctx| {
                markers.reset_for_account_transition(ctx);
            });
        });
        let (view, _) = add_first_run_onboarding_test_session(&mut app, &fixture, true);

        view.update(&mut app, |view, ctx| {
            view.session_state.update(ctx, |state, ctx| {
                state.dismiss_first_zero_state(ctx);
            });
        });
        app.update(|ctx| {
            TuiOnboardingMarkers::handle(ctx).update(ctx, |markers, ctx| {
                markers.set_ready_for_test(true, false, ctx);
            });
        });

        app.read(|ctx| {
            assert!(
                !view
                    .as_ref(ctx)
                    .session_state
                    .as_ref(ctx)
                    .show_first_zero_state()
            );
        });
        app.update(|ctx| {
            let consumed_again = TuiOnboardingMarkers::handle(ctx).update(ctx, |markers, ctx| {
                markers.consume(TuiOnboardingMarker::FirstZeroState, ctx)
            });
            assert!(!consumed_again);
        });
    });
}

#[test]
fn background_session_does_not_receive_first_run_onboarding() {
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        app.update(|ctx| {
            TuiOnboardingMarkers::handle(ctx).update(ctx, |markers, ctx| {
                markers.reset_for_account_transition(ctx);
            });
        });
        let (onboarding_view, _) = add_first_run_onboarding_test_session(&mut app, &fixture, true);
        let (background_view, _) = add_focus_test_session(&mut app, &fixture, false);

        app.read(|ctx| {
            assert!(
                !onboarding_view
                    .as_ref(ctx)
                    .session_state
                    .as_ref(ctx)
                    .show_first_zero_state()
            );
            assert!(
                !background_view
                    .as_ref(ctx)
                    .session_state
                    .as_ref(ctx)
                    .show_first_zero_state()
            );
        });
        app.update(|ctx| {
            TuiOnboardingMarkers::handle(ctx).update(ctx, |markers, ctx| {
                markers.set_ready_for_test(true, false, ctx);
            });
        });
        app.read(|ctx| {
            assert!(
                onboarding_view
                    .as_ref(ctx)
                    .session_state
                    .as_ref(ctx)
                    .show_first_zero_state()
            );
            assert!(
                !background_view
                    .as_ref(ctx)
                    .session_state
                    .as_ref(ctx)
                    .show_first_zero_state()
            );
        });
        let lines = render_session(&mut app, &onboarding_view, 100, 24);
        assert!(lines.iter().any(|line| line.contains("Welcome to Warp")));
        assert!(
            lines
                .iter()
                .any(|line| line.contains("What’s different about Warp"))
        );
    });
}

#[test]
fn account_transition_hides_first_zero_state_while_markers_reload() {
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        let (view, session_id) = add_first_run_onboarding_test_session(&mut app, &fixture, true);
        app.read(|ctx| {
            assert!(
                !view
                    .as_ref(ctx)
                    .session_state
                    .as_ref(ctx)
                    .show_first_zero_state()
            );
        });

        app.update(|ctx| {
            TuiOnboardingMarkers::handle(ctx).update(ctx, |markers, ctx| {
                markers.reset_for_account_transition(ctx);
            });
        });
        app.read(|ctx| {
            assert!(
                !view
                    .as_ref(ctx)
                    .session_state
                    .as_ref(ctx)
                    .show_first_zero_state()
            );
            assert_eq!(
                TuiSessions::as_ref(ctx).focused_session_id(),
                Some(session_id)
            );
            assert!(TuiSessions::as_ref(ctx).session(session_id).is_some());
        });
    });
}

#[test]
fn zero_state_transitions_through_bootstrap_lifecycle() {
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        let (view, _) = add_focus_test_session(&mut app, &fixture, true);

        // Phase 1: an unfinished ScriptExecution block with visible output suppresses the zero
        // state. The `|| !block.finished()` lifecycle guard covers this case: PTY input is still
        // routed to the block, so the zero state must stay hidden while the block runs.
        view.update(&mut app, |view, _| {
            let mut terminal_model = view.terminal_model.lock();
            terminal_model.block_list_mut().reinit_shell();
            terminal_model.update_blockheight_items(TRANSCRIPT_BLOCK_SPACING.block_padding, 0.0);
            // Advance past WarpInput to ScriptExecution.
            terminal_model.simulate_block("bootstrap", "");
            // Create an unfinished ScriptExecution block with visible output rows.
            terminal_model.simulate_long_running_block("shell init", "startup output\r\n");
        });
        view.read(&app, |view, ctx| {
            assert!(
                !view.transcript.as_ref(ctx).is_empty(),
                "unfinished startup block with visible content should suppress the zero state"
            );
        });

        // Phase 2: once the startup block finishes it no longer satisfies the lifecycle guard
        // (it is finished, not restored, and not PostBootstrapPrecmd), so the zero state returns.
        view.update(&mut app, |view, _| {
            let mut terminal_model = view.terminal_model.lock();
            // Advance bootstrap stage so finish_block() promotes the list to PostBootstrapPrecmd.
            terminal_model.block_list_mut().set_bootstrapped();
            terminal_model.finish_block();
        });
        view.read(&app, |view, ctx| {
            assert!(
                view.transcript.as_ref(ctx).is_empty(),
                "finished ScriptExecution block should no longer suppress the zero state"
            );
        });

        // Phase 3: the first normal post-bootstrap command dismisses the zero state.
        view.update(&mut app, |view, _| {
            view.terminal_model
                .lock()
                .simulate_block("echo hello", "hello\r\n");
        });
        view.read(&app, |view, ctx| {
            assert!(
                !view.transcript.as_ref(ctx).is_empty(),
                "post-bootstrap command with visible output should dismiss the zero state"
            );
        });
    });
}

fn render_footer(
    app: &mut App,
    view: &ViewHandle<super::TuiTerminalSessionView>,
    width: u16,
) -> TuiBuffer {
    app.update(|ctx| {
        let footer = view.as_ref(ctx).render_footer(ctx).finish();
        render_element(footer, ctx, width)
    })
}
fn set_enabled_statusline_items(app: &mut App, items: Vec<TuiStatuslineItem>) {
    app.update(|ctx| {
        AISettings::handle(ctx).update(ctx, |settings, ctx| {
            settings
                .tui_statusline
                .set_value(
                    TuiStatuslineConfig {
                        order: items.clone(),
                        enabled: items,
                    }
                    .normalized(),
                    ctx,
                )
                .expect("statusline setting should persist");
        });
    });
}

#[test]
#[cfg(feature = "voice_input")]
fn footer_falls_back_to_replacing_voice_hints_when_voice_item_is_disabled() {
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        let (view, _) = add_focus_test_session(&mut app, &fixture, true);
        let expected_color = app.read(|ctx| {
            TuiUiBuilder::from_app(ctx)
                .voice_input_status_style()
                .fg
                .expect("voice input status should have a foreground color")
        });

        view.update(&mut app, |view, ctx| {
            let voice_input = view.input_view.as_ref(ctx).voice_input_model().clone();
            voice_input.update(ctx, |voice, ctx| {
                voice.set_state_for_test(TuiVoiceInputState::Listening, ctx);
            });
        });
        let listening_footer = render_footer(&mut app, &view, 80);
        assert_eq!(
            listening_footer.to_lines(),
            vec!["listening to voice input... · esc or enter to stop"]
        );
        assert_eq!(listening_footer[(0, 0)].fg, expected_color);
        view.update(&mut app, |view, ctx| {
            let voice_input = view.input_view.as_ref(ctx).voice_input_model().clone();
            voice_input.update(ctx, |voice, _| {
                voice.set_hold_key_for_test(Some(KeyCode::ControlLeft));
            });
            ctx.notify();
        });
        let held_footer = render_footer(&mut app, &view, 80);
        assert_eq!(
            held_footer.to_lines(),
            vec!["listening to voice input... · release key to stop"]
        );

        view.update(&mut app, |view, ctx| {
            let voice_input = view.input_view.as_ref(ctx).voice_input_model().clone();
            voice_input.update(ctx, |voice, ctx| {
                voice.set_state_for_test(TuiVoiceInputState::Transcribing, ctx);
            });
        });
        let transcribing_footer = render_footer(&mut app, &view, 80);
        assert_eq!(
            transcribing_footer.to_lines(),
            vec!["Transcribing... · esc to cancel"]
        );
        assert_eq!(transcribing_footer[(0, 0)].fg, expected_color);
    });
}

#[test]
#[cfg(feature = "voice_input")]
fn configured_voice_item_renders_idle_listening_and_transcribing_states() {
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        let (view, _) = add_focus_test_session(&mut app, &fixture, true);
        set_enabled_statusline_items(&mut app, vec![TuiStatuslineItem::VoiceInput]);
        assert!(view.read(&app, |view, ctx| {
            view.voice_statusline_is_available(false, ctx)
                && !view.voice_statusline_is_available(true, ctx)
        }));

        let idle_footer = render_footer(&mut app, &view, 80);
        assert_eq!(idle_footer.to_lines(), vec!["◉ Voice"]);

        view.update(&mut app, |view, ctx| {
            let voice_input = view.input_view.as_ref(ctx).voice_input_model().clone();
            voice_input.update(ctx, |voice, ctx| {
                voice.set_state_for_test(TuiVoiceInputState::Listening, ctx);
            });
        });
        let listening_footer = render_footer(&mut app, &view, 80);
        assert_eq!(listening_footer.to_lines(), vec!["◉ Voice"]);
        assert_eq!(
            listening_footer[(0, 0)].fg,
            app.read(|ctx| {
                TuiUiBuilder::from_app(ctx)
                    .success_glyph_style()
                    .fg
                    .expect("success glyph style should have a foreground")
            })
        );

        view.update(&mut app, |view, ctx| {
            let voice_input = view.input_view.as_ref(ctx).voice_input_model().clone();
            voice_input.update(ctx, |voice, ctx| {
                voice.set_state_for_test(TuiVoiceInputState::Transcribing, ctx);
            });
        });
        let transcribing_footer = render_footer(&mut app, &view, 80);
        assert_eq!(transcribing_footer.to_lines(), vec!["… Transcribing"]);
        assert_eq!(
            transcribing_footer[(0, 0)].fg,
            app.read(|ctx| {
                TuiUiBuilder::from_app(ctx)
                    .voice_input_status_style()
                    .fg
                    .expect("voice input status should have a foreground")
            })
        );

        view.update(&mut app, |view, ctx| {
            let voice_input = view.input_view.as_ref(ctx).voice_input_model().clone();
            voice_input.update(ctx, |voice, ctx| {
                voice.set_state_for_test(TuiVoiceInputState::Idle, ctx);
            });
            AISettings::handle(ctx).update(ctx, |settings, ctx| {
                settings
                    .voice_input_enabled_internal
                    .set_value(false, ctx)
                    .expect("voice setting should persist");
            });
        });
        assert!(render_footer(&mut app, &view, 80).to_lines().is_empty());
    });
}

#[test]
#[cfg(feature = "voice_input")]
fn voice_click_is_interactive_only_within_the_segment_bounds() {
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        let (view, _) = add_focus_test_session(&mut app, &fixture, true);
        set_enabled_statusline_items(&mut app, vec![TuiStatuslineItem::VoiceInput]);
        let (mut element, scene, buffer) = render_retained_session(&app, &view, 20, 20);
        let (voice_col, voice_row) = footer_label_position(&buffer, "◉ Voice");
        let inside = (voice_col + 1, voice_row);
        let outside = (voice_col + 7, voice_row);

        dispatch_session_event(
            &app,
            &view,
            &mut element,
            scene.clone(),
            &mouse_moved(inside.0, inside.1),
        );
        assert!(view.read(&app, |view, _| {
            view.voice_input_mouse.lock().unwrap().is_hovered()
        }));
        assert!(dispatch_session_event(
            &app,
            &view,
            &mut element,
            scene.clone(),
            &left_mouse_down(inside.0, inside.1),
        ));
        assert!(view.read(&app, |view, _| {
            view.voice_input_mouse.lock().unwrap().is_clicked()
        }));
        assert!(dispatch_session_event(
            &app,
            &view,
            &mut element,
            scene.clone(),
            &left_mouse_up(inside.0, inside.1),
        ));
        assert!(!view.read(&app, |view, _| {
            view.voice_input_mouse.lock().unwrap().is_clicked()
        }));

        dispatch_session_event(
            &app,
            &view,
            &mut element,
            scene.clone(),
            &mouse_moved(outside.0, outside.1),
        );
        assert!(!view.read(&app, |view, _| {
            view.voice_input_mouse.lock().unwrap().is_hovered()
        }));
        assert!(!dispatch_session_event(
            &app,
            &view,
            &mut element,
            scene,
            &left_mouse_down(outside.0, outside.1),
        ));
    });
}

#[test]
#[cfg(feature = "voice_input")]
fn voice_toggle_stops_listening_and_ignores_transcribing() {
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        let (view, _) = add_focus_test_session(&mut app, &fixture, true);
        view.update(&mut app, |view, ctx| {
            let voice_input = view.input_view.as_ref(ctx).voice_input_model().clone();
            voice_input.update(ctx, |voice, ctx| {
                voice.set_state_for_test(TuiVoiceInputState::Listening, ctx);
            });
            view.handle_action(
                &TuiTerminalSessionAction::ToggleVoiceInputFromStatusline,
                ctx,
            );
        });
        view.read(&app, |view, ctx| {
            assert_eq!(
                view.input_view.as_ref(ctx).voice_state(ctx),
                TuiVoiceInputState::Transcribing
            );
        });

        view.update(&mut app, |view, ctx| {
            view.handle_action(
                &TuiTerminalSessionAction::ToggleVoiceInputFromStatusline,
                ctx,
            );
        });
        view.read(&app, |view, ctx| {
            assert_eq!(
                view.input_view.as_ref(ctx).voice_state(ctx),
                TuiVoiceInputState::Transcribing
            );
        });
    });
}
/// A replacing hint occupies the whole status row, so no section separators,
/// branch arrows, or usage text should appear alongside it.
fn assert_footer_segments_absent(lines: &[String]) {
    let row = lines.join("\n");
    assert!(
        !row.contains('│'),
        "a replacing hint should occupy the whole row with no sections: {row}"
    );
    assert!(
        !row.contains(" | "),
        "a replacing hint should contain no statusline group dividers: {row}"
    );
    assert!(
        !row.contains(" ⊢ "),
        "the cwd/branch section is absent: {row}"
    );
    assert!(
        !row.contains("credits"),
        "the usage section is absent: {row}"
    );
}

#[test]
fn new_slash_command_clears_shell_commands_from_transcript() {
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        let (view, _) = add_focus_test_session(&mut app, &fixture, true);
        view.update(&mut app, |view, _| {
            let mut terminal_model = view.terminal_model.lock();
            terminal_model.block_list_mut().set_bootstrapped();
            terminal_model.simulate_block("echo before-new", "before-new\r\n");
        });

        view.read(&app, |view, ctx| {
            assert!(!view.transcript.as_ref(ctx).is_empty());
            assert!(
                view.terminal_model
                    .lock()
                    .block_list()
                    .blocks()
                    .iter()
                    .any(|block| block.command_to_string() == "echo before-new")
            );
        });

        view.update(&mut app, |view, ctx| {
            view.execute_tui_slash_command(&slash_commands::NEW, None, ctx);
        });

        view.read(&app, |view, ctx| {
            assert!(
                view.transcript.as_ref(ctx).is_empty(),
                "/new should clear both agent and shell transcript blocks"
            );
            assert_eq!(
                view.terminal_model.lock().block_list().blocks().len(),
                1,
                "/new should leave only the active prompt block"
            );
        });
    });
}
#[test]
fn clear_slash_command_clears_shell_commands_from_transcript() {
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        let (view, _) = add_focus_test_session(&mut app, &fixture, true);
        view.update(&mut app, |view, _| {
            let mut terminal_model = view.terminal_model.lock();
            terminal_model.block_list_mut().set_bootstrapped();
            terminal_model.simulate_block("echo before-clear", "before-clear\r\n");
        });

        view.read(&app, |view, ctx| {
            assert!(!view.transcript.as_ref(ctx).is_empty());
            assert!(
                view.terminal_model
                    .lock()
                    .block_list()
                    .blocks()
                    .iter()
                    .any(|block| block.command_to_string() == "echo before-clear")
            );
        });

        view.update(&mut app, |view, ctx| {
            view.execute_tui_slash_command(&slash_commands::CLEAR, None, ctx);
        });

        view.read(&app, |view, ctx| {
            assert!(
                view.transcript.as_ref(ctx).is_empty(),
                "/clear should clear both agent and shell transcript blocks, identical to /new"
            );
            assert_eq!(
                view.terminal_model.lock().block_list().blocks().len(),
                1,
                "/clear should leave only the active prompt block"
            );
        });
    });
}

#[test]
fn orchestration_tab_icon_replaces_identity_only_while_active_or_blocked() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            ctx.add_singleton_model(|_| Appearance::mock());
            let builder = TuiUiBuilder::from_app(ctx);
            let identity = AgentIdentity {
                glyph: "✠",
                style: TuiStyle::default().fg(Color::Blue),
            };
            for (status, expected_glyph) in [
                (ConversationStatus::InProgress, "●"),
                (ConversationStatus::TransientError, "●"),
                (ConversationStatus::WaitingForEvents, "●"),
                (
                    ConversationStatus::Blocked {
                        blocked_action: "approval".to_owned(),
                    },
                    "■",
                ),
            ] {
                assert_eq!(
                    orchestration_tab_icon(&status, &identity, &builder).0,
                    expected_glyph,
                );
            }
            for status in [
                ConversationStatus::Success,
                ConversationStatus::Error,
                ConversationStatus::Cancelled,
            ] {
                assert_eq!(
                    orchestration_tab_icon(&status, &identity, &builder),
                    (identity.glyph, identity.style),
                );
            }
        });
    });
}

#[test]
fn footer_renders_agent_sections_left_aligned() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            ctx.add_singleton_model(|_| Appearance::mock());
            let builder = TuiUiBuilder::from_app(ctx);
            let usage = UsageToggle::default().render_entry(
                TuiUsageDisplayMode::default(),
                ConversationUsageTotals {
                    credits_spent: 2.5,
                    cost_in_cents: Some(0.0),
                    has_usage: true,
                },
                ctx,
                |_, _| {},
            );
            let row = render_status_footer_row(
                FooterSegments {
                    ordered: vec![
                        FooterSegment::Model(
                            TuiText::new("TestModel")
                                .with_style(builder.primary_text_style())
                                .truncate()
                                .finish(),
                        ),
                        FooterSegment::WorkingDirectory("/home/user/warp".to_owned()),
                        FooterSegment::GitBranch("main".to_owned()),
                        FooterSegment::CreditUsage(usage),
                        FooterSegment::GitDiff {
                            files_changed: 2,
                            additions: 3,
                            deletions: 1,
                        },
                    ],
                },
                &builder,
            )
            .finish();
            let lines = render_element(row, ctx, 120).to_lines();
            let line = lines.join("\n");

            assert_eq!(
                lines,
                vec!["TestModel | /home/user/warp ⊢ main | 2.5 credits | ☰ 2 • +3 -1"],
                "agent footer is left-aligned in order model → cwd/branch → usage → diff"
            );
            assert!(
                line.starts_with("TestModel"),
                "the first segment starts at the left edge (no flex-spacer padding)"
            );
            assert!(!line.contains('←'), "the conversations callout is absent");
        });
    });
}

/// The footer's usage entry is gated on `has_usage`: a freshly started
/// conversation that has not reported any usage yet must not surface totals.
#[test]
fn footer_usage_entry_is_hidden_until_the_conversation_reports_usage() {
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        let (view, _) = add_focus_test_session(&mut app, &fixture, true);

        view.update(&mut app, |view, ctx| {
            view.conversation_selection.update(ctx, |selection, ctx| {
                selection
                    .try_start_new_conversation(AgentViewEntryOrigin::Tui, ctx)
                    .expect("test conversation should start");
            });
        });

        view.read(&app, |view, ctx| {
            assert!(
                view.conversation_selection
                    .as_ref(ctx)
                    .selected_conversation(ctx)
                    .is_some(),
                "a conversation must be selected for the gate to be meaningful"
            );
            assert!(
                view.selected_conversation_usage_totals(ctx).is_none(),
                "a conversation without usage must not surface footer totals"
            );
        });
    });
}

/// A conversation with usage but an unknown historical cost (legacy restore)
/// still renders the usage entry — `Cost unavailable` in cost mode, never
/// `$0.00` — even when credits happen to be zero. Cost mode is only reachable
/// while the credits⇄dollars toggle is enabled, so this exercises the
/// `PricingTransparency`-on path.
#[test]
fn footer_usage_entry_shows_unknown_cost_even_with_zero_credits() {
    App::test((), |mut app| async move {
        let _cost_transparency = FeatureFlag::PricingTransparency.override_enabled(true);
        app.update(|ctx| {
            ctx.add_singleton_model(|_| Appearance::mock());
            let builder = TuiUiBuilder::from_app(ctx);
            let usage = UsageToggle::default().render_entry(
                TuiUsageDisplayMode::Cost,
                ConversationUsageTotals {
                    credits_spent: 0.0,
                    cost_in_cents: None,
                    has_usage: true,
                },
                ctx,
                |_, _| {},
            );
            let row = render_status_footer_row(
                FooterSegments {
                    ordered: vec![
                        FooterSegment::Model(
                            TuiText::new("TestModel")
                                .with_style(builder.primary_text_style())
                                .truncate()
                                .finish(),
                        ),
                        FooterSegment::CreditUsage(usage),
                    ],
                },
                &builder,
            )
            .finish();
            let line = render_element(row, ctx, 80).to_lines().join("\n");

            assert!(
                line.contains("Cost unavailable"),
                "unknown historical cost renders the stable fallback text, got: {line}"
            );
            assert!(
                !line.contains("$0.00"),
                "unknown cost must never render as $0.00, got: {line}"
            );
        });
    });
}

#[test]
fn footer_does_not_render_credit_actions() {
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        let (view, _) = add_focus_test_session(&mut app, &fixture, true);

        let lines = render_session(&mut app, &view, 80, 40);
        assert!(
            lines.iter().all(|line| {
                !line.contains("Out of credits")
                    && !line.contains("Compare plans")
                    && !line.contains("Use your own API keys")
            }),
            "credit actions belong to the failed transcript block:\n{}",
            lines.join("\n")
        );
    });
}

#[test]
fn footer_renders_shell_mode_sections_without_model_or_usage() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            ctx.add_singleton_model(|_| Appearance::mock());
            let builder = TuiUiBuilder::from_app(ctx);
            let row = render_status_footer_row(
                FooterSegments {
                    ordered: vec![
                        FooterSegment::ShellMode,
                        FooterSegment::WorkingDirectory("/home/user/warp".to_owned()),
                        FooterSegment::GitBranch("main".to_owned()),
                        FooterSegment::GitDiff {
                            files_changed: 2,
                            additions: 3,
                            deletions: 1,
                        },
                    ],
                },
                &builder,
            )
            .finish();
            let buffer = render_element(row, ctx, 120);
            assert_eq!(
                buffer[(0, 0)].fg,
                builder
                    .shell_command_accent_style()
                    .fg
                    .expect("shell command accent has a foreground")
            );
            let lines = buffer.to_lines();
            let line = lines.join("\n");

            assert_eq!(
                lines,
                vec![format!(
                    "{SHELL_MODE_HINT} /home/user/warp ⊢ main | ☰ 2 • +3 -1"
                )],
                "shell footer leads with the shell-mode indicator and hides model/usage"
            );
            assert!(
                line.starts_with(SHELL_MODE_HINT),
                "shell mode is the first segment"
            );
            assert!(
                !line.contains("TestModel"),
                "model segment is hidden in shell mode"
            );
            assert!(
                !line.contains("2.5 credits"),
                "usage segment is hidden in shell mode"
            );
        });
    });
}

#[test]
fn footer_transient_state_replaces_all_sections() {
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        let (view, _) = add_focus_test_session(&mut app, &fixture, true);

        // ctrl-c exit confirmation replaces the whole row.
        view.update(&mut app, |view, _| {
            view.exit_confirmation.arm(Instant::now());
        });
        let lines = render_footer_lines(&mut app, &view, 80);
        assert_eq!(lines, vec![CTRL_C_EXIT_HINT]);
        assert_footer_segments_absent(&lines);

        // Loading-conversation hint replaces the whole row.
        view.update(&mut app, |view, _| {
            view.exit_confirmation.disarm();
            view.conversation_restore_state = ConversationRestoreState::Loading {
                origin: TuiConversationRestoreOrigin::ConversationList,
                target: TuiConversationRestoreTelemetryTarget::Local,
                request_id: 0,
                future: None,
            };
        });
        let lines = render_footer_lines(&mut app, &view, 80);
        assert_eq!(lines, vec![LOADING_CONVERSATION_HINT]);
        assert_footer_segments_absent(&lines);

        // A transient notice replaces the whole row.
        view.update(&mut app, |view, ctx| {
            view.conversation_restore_state = ConversationRestoreState::Idle;
            view.show_transient_hint("transient notice".to_owned(), ctx);
        });
        let lines = render_footer_lines(&mut app, &view, 80);
        assert_eq!(lines, vec!["transient notice"]);
        assert_footer_segments_absent(&lines);

        // Priority: when ctrl-c, loading, and a transient notice all overlap,
        // ctrl-c wins (the existing ctrl-c → loading → transient order).
        view.update(&mut app, |view, ctx| {
            view.exit_confirmation.arm(Instant::now());
            view.conversation_restore_state = ConversationRestoreState::Loading {
                origin: TuiConversationRestoreOrigin::ConversationList,
                target: TuiConversationRestoreTelemetryTarget::Local,
                request_id: 1,
                future: None,
            };
            view.show_transient_hint("transient notice".to_owned(), ctx);
        });
        let lines = render_footer_lines(&mut app, &view, 80);
        assert_eq!(lines, vec![CTRL_C_EXIT_HINT]);
    });
}

#[test]
fn footer_conversations_callout_no_longer_renders() {
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        let (view, _) = add_focus_test_session(&mut app, &fixture, true);

        // With an empty input and no replacing hint, the footer renders the
        // left-aligned sectioned row — never the obsolete `← for conversations`
        // callout (render_left_footer_hint and the show_conversations_hint
        // branch are removed, not merely unreachable).
        let lines = render_footer_lines(&mut app, &view, 80);
        let row = lines.join("\n");
        assert!(
            !row.contains("← for conversations"),
            "the conversations callout must not render: {row}"
        );
        assert!(
            !row.contains('←'),
            "no conversations-callout glyph remains: {row}"
        );
        assert!(
            row.contains("auto (cost-efficient)"),
            "the configured status row renders in place of the callout: {row}"
        );
    });
}
#[test]
fn interrupt_event_projects_to_high_level_pty_intent() {
    let event = TuiTerminalSessionEvent::InterruptPty;
    assert!(matches!(event.pty_intent(), Some(PtyIntent::Interrupt)));
}

#[test]
fn terminal_use_interrupt_closes_shortcuts_before_taking_control() {
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        let (view, _) = add_focus_test_session(&mut app, &fixture, true);
        view.update(&mut app, |view, ctx| {
            let terminal_surface_id = ctx.view_id();
            let conversation_id =
                BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                    history.start_new_conversation(terminal_surface_id, false, false, false, ctx)
                });
            view.terminal_model
                .lock()
                .simulate_long_running_block("sleep 20", "running");
            view.terminal_model
                .lock()
                .block_list_mut()
                .active_block_mut()
                .set_agent_interaction_mode_for_agent_monitored_command(
                    &TaskId::new("test-cli-subagent".to_owned()),
                    conversation_id,
                )
                .expect("command should become agent monitored");
            view.suggestions_mode.update(ctx, |mode, ctx| {
                mode.set_mode(
                    TuiInputSuggestionsMode::ReadOnlyMenu(TuiReadOnlyMenuKind::Shortcuts),
                    ctx,
                );
            });

            view.handle_action(&TuiTerminalSessionAction::Interrupt, ctx);

            assert_eq!(
                view.suggestions_mode.as_ref(ctx).mode(),
                TuiInputSuggestionsMode::Closed
            );
            let target = view
                .cli_subagent_controller
                .as_ref(ctx)
                .active_target()
                .expect("terminal use target should remain active");
            assert!(matches!(
                target.control_state,
                LongRunningCommandControlState::User {
                    reason: UserTakeOverReason::Stop {
                        should_auto_resume: true
                    }
                }
            ));
        });
    });
}

#[test]
fn user_input_event_projects_to_raw_user_bytes() {
    let event = TuiTerminalSessionEvent::WriteUserInput(b"hello\r".to_vec().into());
    let Some(PtyIntent::WriteBytes(bytes)) = event.pty_intent() else {
        panic!("user input event should map to raw PTY bytes");
    };
    assert_eq!(&*bytes, b"hello\r");
}

#[test]
fn running_command_attachment_bindings_are_context_scoped() {
    App::test((), |mut app| async move {
        app.update(crate::keybindings::init);
        app.read(|ctx| {
            let attach = ctx
                .editable_bindings()
                .find(|binding| binding.name == ATTACH_AGENT_TO_RUNNING_COMMAND_BINDING_NAME)
                .expect("running-command attach binding");
            assert_eq!(
                *attach.trigger,
                Trigger::Keystrokes(vec![Keystroke::parse("ctrl-shift-enter").unwrap()])
            );
            let detach = ctx
                .editable_bindings()
                .find(|binding| binding.name == DETACH_AGENT_FROM_RUNNING_COMMAND_BINDING_NAME)
                .expect("running-command detach binding");
            assert_eq!(
                *detach.trigger,
                Trigger::Keystrokes(vec![Keystroke::parse("escape").unwrap()])
            );

            let mut input_context = Context::default();
            input_context.set.insert("TuiInputView");
            assert!(!attach.in_context(&input_context));
            assert!(!detach.in_context(&input_context));

            input_context
                .set
                .insert(SESSION_CAN_ATTACH_AGENT_TO_RUNNING_COMMAND_FLAG);
            assert!(attach.in_context(&input_context));
            assert!(!detach.in_context(&input_context));

            input_context
                .set
                .remove(SESSION_CAN_ATTACH_AGENT_TO_RUNNING_COMMAND_FLAG);
            input_context
                .set
                .insert(SESSION_CAN_DETACH_AGENT_FROM_RUNNING_COMMAND_FLAG);
            assert!(!attach.in_context(&input_context));
            assert!(detach.in_context(&input_context));
        });
    });
}
#[test]
fn plan_toggle_uses_contextual_ctrl_p_and_ctrl_shift_p() {
    App::test((), |mut app| async move {
        app.update(crate::keybindings::init);
        app.read(|ctx| {
            let toggle = ctx
                .get_binding_by_name(PLAN_TOGGLE_BINDING_NAME)
                .expect("primary plan toggle binding");
            assert_eq!(
                *toggle.trigger,
                Trigger::Keystrokes(vec![Keystroke::parse("ctrl-shift-P").unwrap()])
            );

            let fallback = ctx
                .editable_bindings()
                .find(|binding| binding.name == CONTEXTUAL_PLAN_TOGGLE_BINDING_NAME)
                .expect("contextual plan toggle binding");
            let ctrl_p = Trigger::Keystrokes(vec![Keystroke::parse("ctrl-p").unwrap()]);
            assert_eq!(*fallback.trigger, ctrl_p);

            let mut input_without_plan = Context::default();
            input_without_plan.set.insert("TuiInputView");
            let mut input_with_plan = input_without_plan.clone();
            input_with_plan.set.insert(PLAN_TOGGLE_AVAILABLE_FLAG);
            let mut enhanced_input_with_plan = input_with_plan.clone();
            enhanced_input_with_plan
                .set
                .insert(KEYBOARD_ENHANCEMENT_AVAILABLE_FLAG);
            assert!(!fallback.in_context(&input_without_plan));
            assert!(fallback.in_context(&input_with_plan));
            assert!(!fallback.in_context(&enhanced_input_with_plan));

            let ctrl_p_move_up = ctx
                .editable_bindings()
                .find(|binding| binding.name == "tui:input:move_up" && *binding.trigger == ctrl_p)
                .expect("Ctrl+P move-up fallback");
            assert!(ctrl_p_move_up.in_context(&input_without_plan));
            assert!(!ctrl_p_move_up.in_context(&input_with_plan));
            assert!(ctrl_p_move_up.in_context(&enhanced_input_with_plan));
        });
    });
}

#[test]
fn auto_approve_uses_ctrl_shift_i() {
    App::test((), |mut app| async move {
        app.update(crate::keybindings::init);
        app.read(|ctx| {
            let binding = ctx
                .editable_bindings()
                .find(|binding| binding.name == AUTO_APPROVE_TOGGLE_BINDING_NAME)
                .expect("auto-approve toggle binding");
            assert_eq!(
                *binding.trigger,
                Trigger::Keystrokes(vec![Keystroke::parse("ctrl-shift-I").unwrap()])
            );

            let mut session_context = Context::default();
            session_context
                .set
                .insert(TuiTerminalSessionView::ui_name());
            assert!(binding.in_context(&session_context));
        });
    });
}

#[test]
fn voice_hold_keys_preserve_left_and_right_modifiers() {
    let cases = [
        (TuiVoiceInputHoldKey::None, None),
        (TuiVoiceInputHoldKey::AltLeft, Some(KeyCode::AltLeft)),
        (TuiVoiceInputHoldKey::AltRight, Some(KeyCode::AltRight)),
        (
            TuiVoiceInputHoldKey::ControlLeft,
            Some(KeyCode::ControlLeft),
        ),
        (
            TuiVoiceInputHoldKey::ControlRight,
            Some(KeyCode::ControlRight),
        ),
        (TuiVoiceInputHoldKey::SuperLeft, Some(KeyCode::SuperLeft)),
        (TuiVoiceInputHoldKey::SuperRight, Some(KeyCode::SuperRight)),
        (TuiVoiceInputHoldKey::ShiftLeft, Some(KeyCode::ShiftLeft)),
        (TuiVoiceInputHoldKey::ShiftRight, Some(KeyCode::ShiftRight)),
    ];
    for (setting, modifier) in cases {
        let converted: Option<KeyCode> = setting.into();
        assert_eq!(converted, modifier);
    }
}

#[cfg(feature = "voice_input")]
fn voice_key_event(key: KeyCode, state: KeyState) -> TuiEvent {
    TuiEvent::ModifierKeyChanged {
        key_code: key,
        state,
    }
}
#[test]
#[cfg(feature = "voice_input")]
fn voice_hold_handler_matches_only_the_configured_side() {
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        let (view, _) = add_focus_test_session(&mut app, &fixture, true);
        app.read(|ctx| {
            assert!(
                !requires_modifier_key_reporting(ctx),
                "the default hold key must not request modifier reporting"
            );
        });
        app.update(|ctx| {
            TuiVoiceSettings::handle(ctx).update(ctx, |settings, ctx| {
                settings
                    .voice_input_hold_key
                    .set_value(TuiVoiceInputHoldKey::ControlLeft, ctx)
                    .expect("voice hold key should update");
            });
        });
        app.read(|ctx| {
            assert!(
                requires_modifier_key_reporting(ctx),
                "a configured hold key must request modifier reporting"
            );
        });
        view.update(&mut app, |view, _| {
            view.keyboard_enhancement_supported = true;
        });
        let (mut element, scene, _) = render_retained_session(&app, &view, 100, 40);

        assert!(dispatch_session_event(
            &app,
            &view,
            &mut element,
            scene.clone(),
            &voice_key_event(KeyCode::ControlLeft, KeyState::Pressed),
        ));
        assert!(!dispatch_session_event(
            &app,
            &view,
            &mut element,
            scene,
            &voice_key_event(KeyCode::ControlRight, KeyState::Released),
        ));
    });
}

#[test]
#[cfg(feature = "voice_input")]
fn voice_hold_handler_keeps_release_after_composer_loses_input() {
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        let (view, _) = add_focus_test_session(&mut app, &fixture, true);
        view.update(&mut app, |view, ctx| {
            view.keyboard_enhancement_supported = true;
            let voice_input = view.input_view.as_ref(ctx).voice_input_model().clone();
            voice_input.update(ctx, |voice, _| {
                voice.set_hold_key_for_test(Some(KeyCode::ControlLeft));
            });
        });
        let (mut element, scene) = app.read(|ctx| {
            let mut element =
                view.as_ref(ctx)
                    .with_voice_hold_handler(TuiText::new("").finish(), false, ctx);
            let mut rendered_views = EntityIdMap::default();
            let mut layout_ctx = TuiLayoutContext {
                rendered_views: &mut rendered_views,
            };
            element.layout(
                TuiConstraint::loose(TuiSize::new(1, 1)),
                &mut layout_ctx,
                ctx,
            );
            let mut buffer = TuiBuffer::empty(TuiRect::new(0, 0, 1, 1));
            let mut paint_ctx = TuiPaintContext::new(&mut rendered_views);
            {
                let mut surface = TuiPaintSurface::new(&mut buffer);
                element.render(TuiScreenPosition::new(0, 0), &mut surface, &mut paint_ctx);
            }
            (element, Rc::new(paint_ctx.scene))
        });

        assert!(dispatch_session_event(
            &app,
            &view,
            &mut element,
            scene.clone(),
            &voice_key_event(KeyCode::ControlLeft, KeyState::Released),
        ));
        assert!(!dispatch_session_event(
            &app,
            &view,
            &mut element,
            scene,
            &voice_key_event(KeyCode::ControlLeft, KeyState::Pressed),
        ));
    });
}

#[test]
#[cfg(not(feature = "voice_input"))]
fn voice_controls_stay_disabled_without_voice_input_support() {
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        let (view, _) = add_focus_test_session(&mut app, &fixture, true);
        app.update(crate::keybindings::init);
        app.read(|ctx| {
            assert!(
                ctx.editable_bindings()
                    .all(|binding| binding.name != "tui:session:start_voice_input")
            );
            assert!(!view.as_ref(ctx).voice_statusline_is_available(false, ctx));
        });
    });
}
#[test]
fn blocked_terminal_use_action_acceptance_uses_ctrl_enter_without_rebinding_submit() {
    App::test((), |mut app| async move {
        app.update(crate::keybindings::init);
        app.read(|ctx| {
            let accept = ctx
                .editable_bindings()
                .find(|binding| binding.name == ACCEPT_BLOCKED_TERMINAL_USE_ACTION_BINDING_NAME)
                .expect("blocked terminal-use action acceptance binding");
            assert_eq!(
                *accept.trigger,
                Trigger::Keystrokes(vec![Keystroke::parse("ctrl-enter").unwrap()])
            );

            let mut input_context = Context::default();
            input_context.set.insert("TuiInputView");
            assert!(!accept.in_context(&input_context));
            input_context
                .set
                .insert(SESSION_CAN_ACCEPT_BLOCKED_TERMINAL_USE_ACTION_FLAG);
            assert!(accept.in_context(&input_context));

            let submit = ctx
                .editable_bindings()
                .find(|binding| binding.name == "tui:input:submit")
                .expect("input submit binding");
            assert_eq!(
                *submit.trigger,
                Trigger::Keystrokes(vec![Keystroke::parse("enter").unwrap()])
            );
            assert!(submit.in_context(&input_context));
        });
    });
}
#[test]
#[cfg(feature = "voice_input")]
fn voice_input_uses_ctrl_s_only_when_composer_shortcuts_are_active() {
    App::test((), |mut app| async move {
        app.update(crate::keybindings::init);
        app.read(|ctx| {
            let binding = ctx
                .editable_bindings()
                .filter(|binding| binding.name == VOICE_INPUT_BINDING_NAME)
                .find(|binding| {
                    *binding.trigger
                        == Trigger::Keystrokes(vec![Keystroke::parse("ctrl-s").unwrap()])
                })
                .expect("hardcoded ctrl-s voice-input binding");

            let mut session_context = Context::default();
            session_context
                .set
                .insert(TuiTerminalSessionView::ui_name());
            assert!(!binding.in_context(&session_context));

            session_context
                .set
                .insert(SESSION_COMPOSER_SHORTCUTS_ACTIVE_FLAG);
            assert!(binding.in_context(&session_context));
        });
    });
}
#[test]
fn ctrl_d_is_owned_by_the_session_surface_not_input_delete_forward() {
    App::test((), |mut app| async move {
        app.update(crate::keybindings::init);
        app.read(|ctx| {
            let ctrl_d = Trigger::Keystrokes(vec![Keystroke::parse("ctrl-d").unwrap()]);

            // The prompt input no longer binds ctrl-d to delete-forward (the
            // session surface owns it); only the `delete` key deletes forward.
            let input_delete_forward_binds_ctrl_d = ctx
                .editable_bindings()
                .any(|b| b.name == "tui:input:delete_forward" && *b.trigger == ctrl_d);
            assert!(
                !input_delete_forward_binds_ctrl_d,
                "input delete-forward must not bind ctrl-d"
            );

            // The generic editor keeps ctrl-d as delete-forward.
            let editor_delete_forward_binds_ctrl_d = ctx
                .editable_bindings()
                .any(|b| b.name == "tui:editor:delete_forward" && *b.trigger == ctrl_d);
            assert!(
                editor_delete_forward_binds_ctrl_d,
                "editor delete-forward should still bind ctrl-d"
            );

            // The session handles ctrl-d only while the prompt is focused.
            // When a process owns focus, ctrl-d falls through to the terminal
            // element's standard PTY key encoding.
            let session_binds_ctrl_d = ctx.get_key_bindings().any(|b| {
                *b.trigger == ctrl_d && b.name.is_empty() && b.group == Some(TUI_BINDING_GROUP)
            });
            assert!(
                session_binds_ctrl_d,
                "the session should bind ctrl-d for prompt exit / deletion"
            );
        });
    });
}

#[test]
fn non_command_prompt_preserves_leading_whitespace() {
    assert_eq!(raw_prompt_if_not_blank("  /compact"), Some("  /compact"));
}

#[test]
fn whitespace_only_prompt_is_ignored() {
    assert_eq!(raw_prompt_if_not_blank(" \t\n"), None);
}

#[test]
fn file_export_success_message_includes_destination_path() {
    let directory = tempfile::tempdir().expect("temp directory");
    let export = export_conversation_markdown(
        Some(directory.path().to_str().expect("UTF-8 temp path")),
        Some("conversation.md"),
        None,
        "# Conversation",
    )
    .expect("conversation export");

    assert_eq!(
        export_file_success_message(&export),
        format!("Conversation exported to {}", export.path().display())
    );
}

#[test]
fn resize_event_maps_to_pty_resize_intent() {
    let last_size = SizeInfo::new_without_font_metrics(24, 120);
    let size_update = SizeUpdate::from_cell_dimensions(last_size, 8, 42);
    let event = TuiTerminalSessionEvent::Resize(size_update);

    let Some(PtyIntent::Resize(actual_update)) = event.pty_intent() else {
        panic!("resize event should map to a PTY resize intent");
    };
    assert_eq!(actual_update.new_size().rows(), 8);
    assert_eq!(actual_update.new_size().columns(), 42);
}

#[test]
fn alternate_screen_clears_orchestration_tab_focus_and_bindings() {
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        let (view, _) = add_focus_test_session(&mut app, &fixture, true);

        view.update(&mut app, |view, ctx| {
            view.orchestration_tabs_focused = true;
            view.terminal_model.lock().process_bytes("\u{1b}[?1049h");
            view.focus_current_owner(ctx);
        });
        view.read(&app, |view, ctx| {
            assert!(!view.orchestration_tabs_focused);
            assert!(
                !view
                    .keymap_context(ctx)
                    .set
                    .contains(ORCHESTRATION_TAB_BAR_FOCUSED_FLAG)
            );
        });
    });
}

#[test]
fn orchestration_updates_refresh_only_the_focused_session() {
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        let (foreground, foreground_id) = add_focus_test_session(&mut app, &fixture, true);
        let (background, background_id) = add_focus_test_session(&mut app, &fixture, false);

        background.update(&mut app, |view, _| {
            view.orchestration_tabs_focused = true;
        });
        app.update(|ctx| {
            TuiOrchestrationModel::handle(ctx).update(ctx, |_, ctx| {
                ctx.notify();
            });
        });

        assert_eq!(
            app.read_model(&fixture.sessions, |sessions, _| {
                sessions.focused_session_id()
            }),
            Some(foreground_id)
        );
        assert!(
            app.read(|ctx| {
                ctx.check_view_or_child_focused(fixture.window_id, &foreground.id())
            })
        );
        assert!(background.read(&app, |view, _| view.orchestration_tabs_focused));

        app.update_model(&fixture.sessions, |sessions, ctx| {
            assert!(sessions.focus_session(background_id, ctx));
        });
        assert!(!background.read(&app, |view, _| view.orchestration_tabs_focused));
    });
}

#[test]
fn terminal_wakeup_redraws_only_the_focused_session() {
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        let (foreground, _) = add_focus_test_session(&mut app, &fixture, true);
        let (background, _) = add_focus_test_session(&mut app, &fixture, false);

        assert!(foreground.update(&mut app, |view, ctx| { view.handle_terminal_wakeup(ctx) }));
        assert!(!background.update(&mut app, |view, ctx| { view.handle_terminal_wakeup(ctx) }));
    });
}

#[test]
fn terminal_wakeup_focuses_a_new_long_running_command() {
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        let (view, _) = add_focus_test_session(&mut app, &fixture, true);
        let input_id = view.read(&app, |view, _| view.input_view.id());

        view.update(&mut app, |view, ctx| {
            view.terminal_model
                .lock()
                .simulate_long_running_block("cat", "");
            assert!(view.input_target().pty_owns_input());
            assert_eq!(
                ctx.focused_view_id(fixture.window_id),
                Some(input_id),
                "the composer remains focused until the delayed terminal wakeup"
            );

            assert!(view.handle_terminal_wakeup(ctx));
        });

        app.read(|ctx| {
            assert_eq!(
                ctx.focused_view_id(fixture.window_id),
                Some(view.id()),
                "the PTY-owning session must receive input after the wakeup"
            );
        });
    });
}
#[test]
fn background_focus_reconciliation_does_not_steal_foreground_focus() {
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        let (foreground, _) = add_focus_test_session(&mut app, &fixture, true);
        let (background, _) = add_focus_test_session(&mut app, &fixture, false);
        let foreground_input_id = foreground.read(&app, |view, _| view.input_view.id());

        background.update(&mut app, |view, ctx| {
            view.reconcile_focus(ctx);
        });

        app.read(|ctx| {
            assert_eq!(
                ctx.focused_view_id(fixture.window_id),
                Some(foreground_input_id),
                "background ownership transitions must not change framework focus"
            );
        });
    });
}

#[test]
fn empty_background_attachment_update_does_not_steal_foreground_focus() {
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        let (foreground, _) = add_focus_test_session(&mut app, &fixture, true);
        let (background, _) = add_focus_test_session(&mut app, &fixture, false);
        let foreground_input_id = foreground.read(&app, |view, _| view.input_view.id());

        background.update(&mut app, |view, ctx| {
            assert!(!view.attachment_bar.as_ref(ctx).should_render(ctx));
            view.attachment_bar
                .update(ctx, |bar, ctx| bar.emit_model_updated_for_test(ctx));
        });

        app.read(|ctx| {
            assert_eq!(
                ctx.focused_view_id(fixture.window_id),
                Some(foreground_input_id),
                "an empty background context update must not focus its hidden input"
            );
        });
    });
}

#[test]
fn foreground_input_pointer_action_recovers_focus_from_background_session() {
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        let (foreground, _) = add_focus_test_session(&mut app, &fixture, true);
        let (background, _) = add_focus_test_session(&mut app, &fixture, false);
        let foreground_input = foreground.read(&app, |view, _| view.input_view.clone());
        let background_input = background.read(&app, |view, _| view.input_view.clone());

        background_input.update(&mut app, |_, ctx| ctx.focus_self());
        foreground_input.update(&mut app, |input, ctx| {
            input.handle_action(
                &TuiInputAction::Editor(TuiEditorAction::SelectionStartAt {
                    offset: CharOffset::from(1),
                }),
                ctx,
            );
        });

        assert!(app.read(|ctx| foreground_input.is_focused(ctx)));
    });
}

#[test]
fn stale_foreground_input_pointer_action_preserves_new_pty_owner() {
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        let (foreground, _) = add_focus_test_session(&mut app, &fixture, true);
        let (background, _) = add_focus_test_session(&mut app, &fixture, false);
        let foreground_input = foreground.read(&app, |view, _| view.input_view.clone());
        let background_input = background.read(&app, |view, _| view.input_view.clone());

        foreground.update(&mut app, |view, _| {
            view.terminal_model
                .lock()
                .simulate_long_running_block("cat", "");
            assert!(view.input_target().pty_owns_input());
        });
        background_input.update(&mut app, |_, ctx| ctx.focus_self());
        foreground_input.update(&mut app, |input, ctx| {
            input.handle_action(
                &TuiInputAction::Editor(TuiEditorAction::SelectionStartAt {
                    offset: CharOffset::from(1),
                }),
                ctx,
            );
        });

        assert!(app.read(|ctx| foreground.is_focused(ctx)));
    });
}
struct LateMaterializationBlockModel {
    conversation_id: AIConversationId,
    status: AIBlockOutputStatus,
}

impl AIBlockModel for LateMaterializationBlockModel {
    type View = TuiAIBlock;

    fn status(&self, _app: &AppContext) -> AIBlockOutputStatus {
        self.status.clone()
    }

    fn server_output_id(&self, _app: &AppContext) -> Option<ServerOutputId> {
        None
    }

    fn model_id(&self, _app: &AppContext) -> Option<LLMId> {
        None
    }

    fn base_model<'a>(&'a self, _app: &'a AppContext) -> Option<&'a LLMId> {
        None
    }

    fn inputs_to_render<'a>(&'a self, _app: &'a AppContext) -> &'a [AIAgentInput] {
        &[]
    }

    fn conversation_id(&self, _app: &AppContext) -> Option<AIConversationId> {
        Some(self.conversation_id)
    }

    fn on_updated_output(
        &self,
        _callback: OutputStatusUpdateCallback<Self::View>,
        _ctx: &mut ViewContext<Self::View>,
    ) {
    }

    fn request_type(&self, _app: &AppContext) -> AIRequestType {
        AIRequestType::Active
    }
}

fn materialize_already_blocked_action(
    app: &mut App,
    session: &ViewHandle<TuiTerminalSessionView>,
    action: AIAgentAction,
) -> ViewHandle<TuiAIBlock> {
    let (conversation_id, action_model, transcript) = session.update(app, |session, ctx| {
        let conversation_id = session
            .conversation_selection
            .update(ctx, |selection, ctx| {
                selection
                    .try_start_new_conversation(AgentViewEntryOrigin::Tui, ctx)
                    .expect("test conversation should start")
            });
        ctx.focus(&session.input_view);
        (
            conversation_id,
            session.ai_action_model.clone(),
            session.transcript.clone(),
        )
    });
    action_model.update(app, |model, ctx| {
        queue_tui_permission_action(model, action.clone(), conversation_id, ctx);
    });
    let status = AIBlockOutputStatus::Complete {
        output: Shared::new(AIAgentOutput {
            messages: vec![AIAgentOutputMessage {
                id: MessageId::new("late-action-message".to_owned()),
                message: AIAgentOutputMessageType::Action(action),
                citations: Vec::new(),
            }],
            ..Default::default()
        }),
    };
    transcript.update(app, |transcript, ctx| {
        transcript.append_agent_block_for_test(
            conversation_id,
            AIAgentExchangeId::new(),
            Rc::new(LateMaterializationBlockModel {
                conversation_id,
                status,
            }),
            ctx,
        )
    })
}

#[test]
fn foreground_session_focuses_an_already_blocked_question_when_materialized() {
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        let (session, _) = add_focus_test_session(&mut app, &fixture, true);
        let action = AIAgentAction {
            id: AIAgentActionId::from("late-question".to_owned()),
            task_id: TaskId::new("task".to_owned()),
            action: AIAgentActionType::AskUserQuestion {
                questions: vec![AskUserQuestionItem {
                    question_id: "question".to_owned(),
                    question: "Which option?".to_owned(),
                    question_type: AskUserQuestionType::MultipleChoice {
                        is_multiselect: false,
                        options: vec![AskUserQuestionOption {
                            label: "Alpha".to_owned(),
                            recommended: false,
                        }],
                        supports_other: false,
                    },
                }],
            },
            requires_result: true,
        };

        let block = materialize_already_blocked_action(&mut app, &session, action);
        let question = block.read(&app, |block, ctx| {
            let Some(BlockingInputSource::AskQuestion(view)) =
                block.active_blocking_input_source(ctx)
            else {
                panic!("materialized question should be the active blocker");
            };
            view.clone()
        });

        app.read(|ctx| {
            assert!(ctx.check_view_or_child_focused(fixture.window_id, &question.id()));
        });
    });
}

#[test]
fn foreground_session_focuses_an_already_blocked_permission_when_materialized() {
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        let (session, _) = add_focus_test_session(&mut app, &fixture, true);
        let action = AIAgentAction {
            id: AIAgentActionId::from("late-permission".to_owned()),
            task_id: TaskId::new("task".to_owned()),
            action: AIAgentActionType::InitProject,
            requires_result: true,
        };

        let block = materialize_already_blocked_action(&mut app, &session, action);
        let permission = block.read(&app, |block, ctx| {
            let Some(BlockingInputSource::Permission(view)) =
                block.active_blocking_input_source(ctx)
            else {
                panic!("materialized permission prompt should be the active blocker");
            };
            view.clone()
        });

        app.read(|ctx| {
            assert!(ctx.check_view_or_child_focused(fixture.window_id, &permission.id()));
        });
    });
}
fn tab_focused_context() -> Context {
    let mut context = Context::default();
    context.set.insert(super::TuiTerminalSessionView::ui_name());
    context.set.insert(ORCHESTRATION_TAB_BAR_FOCUSED_FLAG);
    context
}

fn input_only_context() -> Context {
    let mut context = Context::default();
    context.set.insert(crate::input::TuiInputView::ui_name());
    context
}

#[test]
fn focus_input_bindings_match_down_and_shift_down_in_tab_context_only() {
    App::test((), |mut app| async move {
        app.update(crate::keybindings::init);
        app.read(|ctx| {
            let down = Trigger::Keystrokes(vec![Keystroke::parse("down").unwrap()]);
            let shift_down = Trigger::Keystrokes(vec![Keystroke::parse("shift-down").unwrap()]);

            let focus_input_bindings: Vec<_> = ctx
                .editable_bindings()
                .filter(|b| b.name == "tui:orchestration_tabs:focus_input")
                .collect();
            assert_eq!(
                focus_input_bindings.len(),
                2,
                "down + shift-down bindings should be registered"
            );
            assert!(
                focus_input_bindings.iter().any(|b| *b.trigger == down),
                "plain down should focus the input"
            );
            assert!(
                focus_input_bindings
                    .iter()
                    .any(|b| *b.trigger == shift_down),
                "shift-down should remain an alias"
            );

            let tab_context = tab_focused_context();
            for binding in &focus_input_bindings {
                assert!(
                    binding.in_context(&tab_context),
                    "focus-input binding {:?} should match the tab-focused context",
                    binding.trigger
                );
            }

            let input_context = input_only_context();
            for binding in &focus_input_bindings {
                assert!(
                    !binding.in_context(&input_context),
                    "focus-input binding {:?} must not match a normal input context",
                    binding.trigger
                );
            }
        });
    });
}

#[test]
fn escape_binding_targets_main_agent_in_tab_context_only() {
    App::test((), |mut app| async move {
        app.update(crate::keybindings::init);
        app.read(|ctx| {
            let escape = Trigger::Keystrokes(vec![Keystroke::parse("escape").unwrap()]);
            let binding = ctx
                .editable_bindings()
                .find(|b| b.name == "tui:orchestration_tabs:focus_main")
                .expect("escape focus-main binding is registered");
            assert_eq!(*binding.trigger, escape);

            assert!(binding.in_context(&tab_focused_context()));
            assert!(!binding.in_context(&input_only_context()));
        });
    });
}

#[test]
fn orchestration_tab_navigation_bindings_remain_scoped_to_tab_context() {
    App::test((), |mut app| async move {
        app.update(crate::keybindings::init);
        app.read(|ctx| {
            let tab_context = tab_focused_context();
            let input_context = input_only_context();
            for (name, key) in [
                ("tui:orchestration_tabs:previous", "left"),
                ("tui:orchestration_tabs:next", "right"),
                ("tui:orchestration_tabs:tree_previous", "shift-tab"),
                ("tui:orchestration_tabs:tree_next", "tab"),
                ("tui:orchestration_tabs:first_child", "shift-left"),
                ("tui:orchestration_tabs:last_child", "shift-right"),
            ] {
                let trigger = Trigger::Keystrokes(vec![Keystroke::parse(key).unwrap()]);
                let binding = ctx
                    .editable_bindings()
                    .find(|b| b.name == name && *b.trigger == trigger)
                    .unwrap_or_else(|| panic!("missing {name} on {key}"));
                assert!(
                    binding.in_context(&tab_context),
                    "{name} {key} should match the tab-focused context"
                );
                assert!(
                    !binding.in_context(&input_context),
                    "{name} {key} must not match a normal input context"
                );
            }
        });
    });
}

#[test]
fn orchestration_tab_footer_advertises_down_without_shift_or_escape_hint() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            ctx.add_singleton_model(|_| Appearance::mock());
            let builder = TuiUiBuilder::from_app(ctx);
            let buffer = render_element(render_orchestration_tab_footer(&builder), ctx, 80);
            let footer = buffer.to_lines().join("\n");
            assert!(
                footer.contains("↓ to send a message"),
                "footer should advertise ↓: {footer}"
            );
            assert!(
                !footer.contains("Shift + ↓"),
                "footer must not advertise Shift + ↓: {footer}"
            );
            assert!(
                !footer.to_lowercase().contains("esc"),
                "footer must not advertise an Escape hint: {footer}"
            );
        });
    });
}

/// Registers a session with a live active conversation, returning its view and conversation id.
fn add_orchestration_session(
    app: &mut App,
    fixture: &FocusTestFixture,
    focus: bool,
) -> (
    ViewHandle<super::TuiTerminalSessionView>,
    TuiSessionId,
    AIConversationId,
) {
    let (view, manager) = add_test_terminal_session(app, fixture.window_id);
    let session_id = app.update(|ctx| {
        TuiSessions::register_session(&fixture.sessions, view.clone(), manager, focus, ctx)
    });
    let conversation_id = app.update(|ctx| {
        BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
            let conversation_id =
                history.start_new_conversation(session_id.surface_id(), false, false, false, ctx);
            history.set_active_conversation_id(conversation_id, session_id.surface_id(), ctx);
            conversation_id
        })
    });
    (view, session_id, conversation_id)
}

/// Registers a child session under a parent conversation.
fn add_orchestration_child(
    app: &mut App,
    fixture: &FocusTestFixture,
    parent_conversation_id: AIConversationId,
    name: &str,
) -> (
    ViewHandle<super::TuiTerminalSessionView>,
    TuiSessionId,
    AIConversationId,
) {
    let (view, manager) = add_test_terminal_session(app, fixture.window_id);
    let session_id = app.update(|ctx| {
        TuiSessions::register_session(&fixture.sessions, view.clone(), manager, false, ctx)
    });
    let conversation_id = app.update(|ctx| {
        BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
            let conversation_id = history.start_new_child_conversation(
                session_id.surface_id(),
                name.to_owned(),
                parent_conversation_id,
                Some(Harness::Oz),
                false,
                ctx,
            );
            history.set_active_conversation_id(conversation_id, session_id.surface_id(), ctx);
            conversation_id
        })
    });
    (view, session_id, conversation_id)
}

#[test]
fn new_slash_command_kills_descendant_agents() {
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        let (parent_view, _parent_session_id, parent_conversation_id) =
            add_orchestration_session(&mut app, &fixture, true);
        let (_child_view, _child_session_id, child_conversation_id) =
            add_orchestration_child(&mut app, &fixture, parent_conversation_id, "child");
        let (_grandchild_view, _grandchild_session_id, grandchild_conversation_id) =
            add_orchestration_child(&mut app, &fixture, child_conversation_id, "grandchild");

        parent_view.update(&mut app, |view, ctx| {
            view.conversation_selection.update(ctx, |selection, ctx| {
                selection.select_existing_conversation(
                    parent_conversation_id,
                    AgentViewEntryOrigin::Tui,
                    ctx,
                );
            });
            view.execute_tui_slash_command(&slash_commands::NEW, None, ctx);
        });

        app.read(|ctx| {
            let history = BlocklistAIHistoryModel::as_ref(ctx);
            assert!(
                history.conversation(&child_conversation_id).is_none(),
                "/new should delete direct child conversations"
            );
            assert!(
                history.conversation(&grandchild_conversation_id).is_none(),
                "/new should delete nested child conversations"
            );
            let new_conversation_id = parent_view
                .as_ref(ctx)
                .conversation_selection
                .as_ref(ctx)
                .selected_conversation_id(ctx)
                .expect("/new should select a replacement conversation");
            assert_ne!(new_conversation_id, parent_conversation_id);
            assert!(history.conversation(&new_conversation_id).is_some());
        });
    });
}
#[test]
fn escape_from_child_tab_switches_to_root_and_clears_tab_focus() {
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        let (parent_view, parent_session_id, parent_conversation_id) =
            add_orchestration_session(&mut app, &fixture, true);
        let (child_view, child_session_id, child_conversation_id) =
            add_orchestration_child(&mut app, &fixture, parent_conversation_id, "child");

        // Focus the child session and point its conversation selection at the child
        // conversation so the orchestration snapshot resolves the parent as root.
        app.update(|ctx| {
            TuiSessions::handle(ctx).update(ctx, |sessions, ctx| {
                sessions.focus_session(child_session_id, ctx);
            });
        });
        child_view.update(&mut app, |view, ctx| {
            view.conversation_selection.update(ctx, |selection, ctx| {
                selection.select_existing_conversation(
                    child_conversation_id,
                    AgentViewEntryOrigin::Tui,
                    ctx,
                );
            });
            view.refresh_orchestration_tab_state(ctx);
            view.orchestration_tabs_focused = true;
            view.refresh_orchestration_tab_bar(ctx);
        });
        app.read(|ctx| {
            assert_eq!(
                child_view
                    .as_ref(ctx)
                    .orchestration_tab_bar
                    .as_ref(ctx)
                    .tree_root_key(),
                Some(parent_conversation_id.to_string()),
                "tab bar should expose the parent as the tree root"
            );
        });

        child_view.update(&mut app, |view, ctx| {
            view.handle_action(&TuiTerminalSessionAction::FocusMainOrchestrationTab, ctx);
        });

        app.read(|ctx| {
            assert_eq!(
                TuiSessions::as_ref(ctx).focused_session_id(),
                Some(parent_session_id),
                "escape should switch focus to the root/main session"
            );
            assert!(
                !child_view.as_ref(ctx).orchestration_tabs_focused,
                "child tab focus should be cleared"
            );
            assert!(
                !parent_view.as_ref(ctx).orchestration_tabs_focused,
                "parent tab focus should remain cleared"
            );
            assert!(
                ctx.check_view_or_child_focused(fixture.window_id, &parent_view.id()),
                "root session input should own focus after escape"
            );
        });
    });
}

#[test]
fn escape_with_root_selected_clears_tab_focus_without_switching() {
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        let (parent_view, parent_session_id, parent_conversation_id) =
            add_orchestration_session(&mut app, &fixture, true);
        let (_child_view, _child_session_id, _child_conversation_id) =
            add_orchestration_child(&mut app, &fixture, parent_conversation_id, "child");

        // Point the parent session's conversation selection at the root conversation so
        // the orchestration snapshot resolves the root as both root and selected.
        parent_view.update(&mut app, |view, ctx| {
            view.conversation_selection.update(ctx, |selection, ctx| {
                selection.select_existing_conversation(
                    parent_conversation_id,
                    AgentViewEntryOrigin::Tui,
                    ctx,
                );
            });
            view.refresh_orchestration_tab_state(ctx);
            view.orchestration_tabs_focused = true;
            view.refresh_orchestration_tab_bar(ctx);
        });
        app.read(|ctx| {
            assert_eq!(
                parent_view
                    .as_ref(ctx)
                    .orchestration_tab_bar
                    .as_ref(ctx)
                    .tree_root_key(),
                Some(parent_conversation_id.to_string()),
                "root tab bar should expose the root as the tree root"
            );
        });

        parent_view.update(&mut app, |view, ctx| {
            view.handle_action(&TuiTerminalSessionAction::FocusMainOrchestrationTab, ctx);
        });

        app.read(|ctx| {
            assert_eq!(
                TuiSessions::as_ref(ctx).focused_session_id(),
                Some(parent_session_id),
                "escape with root selected should not switch sessions"
            );
            assert!(
                !parent_view.as_ref(ctx).orchestration_tabs_focused,
                "root tab focus should be cleared"
            );
            assert!(
                ctx.check_view_or_child_focused(fixture.window_id, &parent_view.id()),
                "root session input should own focus after escape"
            );
        });
    });
}

// ── Vim mode tests ───────────────────────────────────────────────────────────

/// The `/vim-mode` slash command static definition must be correctly
/// populated: a non-empty name, a non-empty description, and a TUI-only
/// supported surface (so the command appears only in the TUI's slash-command
/// menu, not in the GUI).
///
/// The global `COMMAND_REGISTRY` is initialized with `SettingsMode::Gui` in
/// unit-test processes (the mode defaults to Gui when not explicitly set at
/// startup), so TUI-only commands are correctly excluded from that registry.
/// This test validates the static definition directly without relying on the
/// filtered registry.
#[test]
fn vim_mode_slash_command_is_registered_in_command_registry() {
    use warp::tui_export::SlashCommandSurfaces;

    let cmd = &slash_commands::VIM_MODE;
    assert_eq!(cmd.name, "/vim-mode");
    assert!(
        !cmd.description.is_empty(),
        "/vim-mode must have a non-empty description"
    );
    // The command must be TUI-only so it appears in the TUI's slash-command
    // menu but is excluded from the GUI surface.
    assert_eq!(
        cmd.supported_surfaces,
        SlashCommandSurfaces::TuiOnly,
        "/vim-mode must be registered as TUI-only"
    );
}

/// Executing the `/vim-mode` slash command must toggle and persist the
/// `AppEditorSettings::vim_mode` setting on each invocation.
#[test]
fn vim_mode_slash_command_persists_toggle() {
    App::test((), |mut app| async move {
        use warp::settings::AppEditorSettings;
        use warpui::SingletonEntity as _;

        let fixture = focus_test_fixture(&mut app);
        let (view, _) = add_focus_test_session(&mut app, &fixture, true);
        // AppEditorSettings is not included in the standard TUI test fixture;
        // register it explicitly (mirrors the `enable_vim_mode` helper in
        // view_tests.rs).
        app.update(AppEditorSettings::register);

        assert!(
            !app.read(|ctx| AppEditorSettings::as_ref(ctx).vim_mode_enabled()),
            "vim mode should start disabled"
        );

        // First toggle: off → on.
        view.update(&mut app, |view, ctx| {
            view.execute_tui_slash_command(&slash_commands::VIM_MODE, None, ctx);
        });
        assert!(
            app.read(|ctx| AppEditorSettings::as_ref(ctx).vim_mode_enabled()),
            "/vim-mode should enable vim mode on the first toggle"
        );
        assert_eq!(
            view.read(&app, |view, _| {
                view.transient_hint
                    .current()
                    .map(|(text, _)| text.to_owned())
            }),
            Some(super::VIM_MODE_ENABLED_HINT.to_owned()),
            "should surface an enabled hint after enabling vim mode"
        );

        // Second toggle: on → off.
        view.update(&mut app, |view, ctx| {
            view.execute_tui_slash_command(&slash_commands::VIM_MODE, None, ctx);
        });
        assert!(
            !app.read(|ctx| AppEditorSettings::as_ref(ctx).vim_mode_enabled()),
            "/vim-mode should disable vim mode on the second toggle"
        );
        assert_eq!(
            view.read(&app, |view, _| {
                view.transient_hint
                    .current()
                    .map(|(text, _)| text.to_owned())
            }),
            Some(super::VIM_MODE_DISABLED_HINT.to_owned()),
            "should surface a disabled hint after disabling vim mode"
        );
    });
}

/// Verifies that `/copy-debugging-id` is available for the TUI's eagerly-created blank
/// conversation, matching the GUI's active-conversation semantics.
#[test]
fn copy_debugging_id_available_in_active_commands_at_zero_state() {
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        let (view, _) = add_focus_test_session(&mut app, &fixture, true);

        view.read(&app, |view, ctx| {
            let has_copy_debugging_id = view
                .slash_commands_source
                .as_ref(ctx)
                .active_commands()
                .any(|(_, cmd)| cmd.kind == SlashCommandKind::CopyDebuggingId);
            assert!(
                has_copy_debugging_id,
                "/copy-debugging-id must be available for the blank active conversation",
            );
        });
    });
}

/// Verifies that `/handoff` remains available for the TUI's blank active conversation.
#[test]
fn handoff_is_available_at_zero_state() {
    App::test((), |mut app| async move {
        let _oz_handoff = FeatureFlag::OzHandoff.override_enabled(true);
        let _local_cloud = FeatureFlag::HandoffLocalCloud.override_enabled(true);
        let fixture = focus_test_fixture(&mut app);
        let (view, _) = add_focus_test_session(&mut app, &fixture, true);
        view.read(&app, |view, ctx| {
            let active_names: Vec<&str> = view
                .slash_commands_source
                .as_ref(ctx)
                .active_commands()
                .map(|(_, cmd)| cmd.name)
                .collect();

            assert!(
                active_names.contains(&slash_commands::MOVE_TO_CLOUD.name),
                "/handoff must be active at zero state",
            );
        });
    });
}

/// Verifies that the full TUI session renders the no-token error hint in its footer.
#[test]
fn copy_debugging_id_footer_hint_renders_in_session() {
    App::test((), |mut app| async move {
        app.update(crate::keybindings::init);
        let fixture = focus_test_fixture(&mut app);
        let (view, _) = add_focus_test_session(&mut app, &fixture, true);

        view.update(&mut app, |view, ctx| {
            view.input_view.update(ctx, |input, ctx| {
                input.set_text(slash_commands::COPY_DEBUGGING_ID.name, ctx);
            });
            view.handle_submitted_input(slash_commands::COPY_DEBUGGING_ID.name, ctx);
        });

        // Render the full session and verify the error hint appears in the
        // rendered output (footer_hint() feeds transient_hint.current() into
        // the footer row at the bottom of the session canvas).
        let rendered = render_session(&mut app, &view, 80, 24).join("\n");
        assert!(
            rendered.contains(super::COPY_DEBUGGING_ID_NO_TOKEN_HINT),
            "rendered session must contain the no-token hint in the footer; got:\n{rendered}",
        );
    });
}

/// The Vim mode indicator (INS/NOR/VIS/V-L/REP) must appear in the footer only
/// while Vim mode is enabled.
///
/// This test validates the accessor and full render path. Notification
/// delivery is covered directly by the input-view mode-change event test.
#[test]
fn vim_mode_indicator_shown_only_when_vim_mode_is_enabled() {
    App::test((), |mut app| async move {
        use warp::settings::AppEditorSettings;
        use warpui::SingletonEntity as _;

        let fixture = focus_test_fixture(&mut app);
        let (view, _) = add_focus_test_session(&mut app, &fixture, true);
        app.update(AppEditorSettings::register);

        // Vim mode off: vim_mode_indicator returns None regardless of mode.
        app.read(|ctx| {
            let indicator = view.as_ref(ctx).vim_mode_indicator(ctx);
            assert!(
                indicator.is_none(),
                "indicator must be None when vim mode is disabled, got {indicator:?}"
            );
        });

        // Enable vim mode. The FSA starts in Insert mode, so the indicator
        // shows "INS", matching the GUI Vim status indicator.
        app.update(|ctx| {
            AppEditorSettings::handle(ctx).update(ctx, |settings, ctx| {
                settings
                    .vim_mode
                    .set_value(true, ctx)
                    .expect("failed to enable vim mode");
            });
        });
        app.read(|ctx| {
            let indicator = view.as_ref(ctx).vim_mode_indicator(ctx);
            assert_eq!(
                indicator,
                Some("INS"),
                "indicator must be INS in Insert mode when vim mode is enabled, got {indicator:?}"
            );
        });

        // Drive the input to Normal mode (Escape from Insert): indicator → Some("NOR").
        view.update(&mut app, |view, ctx| {
            view.input_view.update(ctx, |input, ctx| {
                // Process Escape to move Insert → Normal.
                input.handle_action(&crate::input::view::TuiInputAction::HandleEscape, ctx);
            });
        });
        // Verify via accessor that the mode state is correct.
        app.read(|ctx| {
            let indicator = view.as_ref(ctx).vim_mode_indicator(ctx);
            assert_eq!(
                indicator,
                Some("NOR"),
                "indicator must be NOR in Normal mode when vim mode is enabled"
            );
        });
        // Verify via the full render path: the footer must contain NOR.
        let rendered = render_session(&mut app, &view, 80, 24).join("\n");
        assert!(
            rendered.contains("NOR"),
            "rendered footer must contain 'NOR' after Insert\u{2192}Normal transition, got:\n{rendered}"
        );
        // Uppercase R enters continuous Replace mode and the footer reflects it.
        view.update(&mut app, |view, ctx| {
            view.input_view.update(ctx, |input, ctx| {
                input.handle_action(
                    &crate::input::view::TuiInputAction::Editor(
                        crate::editor_element::TuiEditorAction::InsertChar('R'),
                    ),
                    ctx,
                );
            });
        });
        app.read(|ctx| {
            assert_eq!(view.as_ref(ctx).vim_mode_indicator(ctx), Some("REP"));
        });
        let rendered = render_session(&mut app, &view, 80, 24).join("\n");
        assert!(
            rendered.contains("REP"),
            "rendered footer must contain 'REP' in continuous Replace mode, got:\n{rendered}"
        );

        // Disable vim mode: indicator → None again.
        app.update(|ctx| {
            AppEditorSettings::handle(ctx).update(ctx, |settings, ctx| {
                settings
                    .vim_mode
                    .set_value(false, ctx)
                    .expect("failed to disable vim mode");
            });
        });
        app.read(|ctx| {
            let indicator = view.as_ref(ctx).vim_mode_indicator(ctx);
            assert!(
                indicator.is_none(),
                "indicator must be None after vim mode is disabled, got {indicator:?}"
            );
        });
    });
}

/// Verifies that the footer hint slot shows an error-toned notice after
/// `/copy-debugging-id` is executed when the conversation has no server token.
/// `transient_hint.current()` is the canonical source read by `footer_hint()`
/// when rendering the footer row, so asserting it covers the rendered behavior.
#[test]
fn copy_debugging_id_shows_error_hint_when_no_server_token() {
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        let (view, _) = add_focus_test_session(&mut app, &fixture, true);

        view.update(&mut app, |view, ctx| {
            view.execute_tui_slash_command(&slash_commands::COPY_DEBUGGING_ID, None, ctx);
        });

        // The hint slot must carry the no-token error text with Error tone.
        // `footer_hint()` reads `transient_hint.current()` verbatim when
        // present, so this assertion covers what the footer renders.
        view.read(&app, |view, _| {
            let hint = view.transient_hint.current();
            assert_eq!(
                hint.map(|(text, _)| text),
                Some(super::COPY_DEBUGGING_ID_NO_TOKEN_HINT),
                "/copy-debugging-id with no server token must set the no-token error hint",
            );
            assert_eq!(
                hint.map(|(_, tone)| tone),
                Some(super::super::transient_hint::TransientHintTone::Error),
                "the no-token hint must use the error tone",
            );
        });
    });
}

#[test]
fn kill_child_hint_constant_matches_expected_text() {
    assert_eq!(CTRL_C_KILL_CHILD_HINT, "ctrl-c again to kill child agent");
}

#[test]
fn orchestration_child_selected_footer_shows_kill_hint() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            ctx.add_singleton_model(|_| Appearance::mock());
            let builder = TuiUiBuilder::from_app(ctx);
            let buffer = render_element(
                render_orchestration_child_selected_tab_footer(&builder, 0),
                ctx,
                120,
            );
            let footer = buffer.to_lines().join("\n");
            assert!(
                footer.contains("Ctrl+C"),
                "child-selected footer should show Ctrl+C: {footer}"
            );
            assert!(
                footer.contains("kill sub-agent"),
                "child-selected footer should describe the kill action: {footer}"
            );
            assert!(
                !footer.contains("nested"),
                "leaf footer must not name a nested blast radius: {footer}"
            );
            let group_buffer = render_element(
                render_orchestration_child_selected_tab_footer(&builder, 2),
                ctx,
                120,
            );
            let group_footer = group_buffer.to_lines().join("\n");
            assert!(
                group_footer.contains("kill sub-agent +2 nested"),
                "group footer should name the blast radius: {group_footer}"
            );
            assert!(
                footer.contains('\u{2193}'),
                "child-selected footer should still show the send-message \u{2193} hint: {footer}"
            );
        });
    });
}

#[test]
fn ctrl_c_on_child_tab_with_tabs_focused_kills_immediately() {
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        let (_parent_view, parent_session_id, parent_conversation_id) =
            add_orchestration_session(&mut app, &fixture, true);
        let (child_view, child_session_id, child_conversation_id) =
            add_orchestration_child(&mut app, &fixture, parent_conversation_id, "child");

        // Focus the child session and point its selection at the child conversation so
        // the orchestration snapshot resolves the parent as root and the child as selected.
        app.update(|ctx| {
            TuiSessions::handle(ctx).update(ctx, |sessions, ctx| {
                sessions.focus_session(child_session_id, ctx);
            });
        });
        child_view.update(&mut app, |view, ctx| {
            view.conversation_selection.update(ctx, |selection, ctx| {
                selection.select_existing_conversation(
                    child_conversation_id,
                    AgentViewEntryOrigin::Tui,
                    ctx,
                );
            });
            view.refresh_orchestration_tab_state(ctx);
            view.orchestration_tabs_focused = true;
            view.refresh_orchestration_tab_bar(ctx);
        });

        // Verify the snapshot sees the child tab as the selected non-root tab.
        app.read(|ctx| {
            assert!(
                child_view
                    .as_ref(ctx)
                    .is_child_conversation_selected(ctx)
                    .is_some(),
                "child conversation should be detected as selected"
            );
        });

        // Single ctrl-c should kill the child immediately (no double-press window).
        child_view.update(&mut app, |view, ctx| {
            view.handle_action(&TuiTerminalSessionAction::Interrupt, ctx);
        });

        app.read(|ctx| {
            assert!(
                !child_view.as_ref(ctx).exit_confirmation.is_armed(),
                "kill path must not arm the exit confirmation window"
            );
            assert_eq!(
                child_view.as_ref(ctx).child_kill_armed_conversation,
                None,
                "kill path must not set child_kill_armed_conversation"
            );
            // The child conversation should be deleted from history.
            assert!(
                BlocklistAIHistoryModel::as_ref(ctx)
                    .conversation(&child_conversation_id)
                    .is_none(),
                "child conversation should be deleted from history after kill"
            );
            // Focus should return to the parent session.
            assert_eq!(
                TuiSessions::as_ref(ctx).focused_session_id(),
                Some(parent_session_id),
                "focus should return to the root/main agent after kill"
            );
        });
    });
}

#[test]
fn ctrl_c_on_drilled_anchor_with_tabs_focused_kills_the_subtree() {
    let _flag = FeatureFlag::MultiLevelOrchestration.override_enabled(true);
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        let (_parent_view, parent_session_id, parent_conversation_id) =
            add_orchestration_session(&mut app, &fixture, true);
        let (child_view, child_session_id, child_conversation_id) =
            add_orchestration_child(&mut app, &fixture, parent_conversation_id, "child");
        let (_grandchild_view, _grandchild_session_id, grandchild_conversation_id) =
            add_orchestration_child(&mut app, &fixture, child_conversation_id, "grandchild");

        // Select the group child: it re-anchors the bar, occupying the
        // main-tab slot (selected == anchor != root).
        app.update(|ctx| {
            TuiSessions::handle(ctx).update(ctx, |sessions, ctx| {
                sessions.focus_session(child_session_id, ctx);
            });
        });
        child_view.update(&mut app, |view, ctx| {
            view.conversation_selection.update(ctx, |selection, ctx| {
                selection.select_existing_conversation(
                    child_conversation_id,
                    AgentViewEntryOrigin::Tui,
                    ctx,
                );
            });
            view.refresh_orchestration_tab_state(ctx);
            view.orchestration_tabs_focused = true;
            view.refresh_orchestration_tab_bar(ctx);
        });
        app.read(|ctx| {
            assert_eq!(
                child_view.as_ref(ctx).bar_focused_kill_target(ctx),
                Some((child_conversation_id, 1)),
                "the drilled-in anchor must be the kill target with its blast radius"
            );
        });

        // A single ctrl-c kills the anchor and its subtree — it must not
        // cancel the conversation and arm the app-exit window.
        child_view.update(&mut app, |view, ctx| {
            view.handle_action(&TuiTerminalSessionAction::Interrupt, ctx);
        });

        app.read(|ctx| {
            assert!(
                !child_view.as_ref(ctx).exit_confirmation.is_armed(),
                "the drilled-anchor kill must not arm the exit confirmation window"
            );
            let history = BlocklistAIHistoryModel::as_ref(ctx);
            assert!(
                history.conversation(&child_conversation_id).is_none(),
                "the anchor conversation must be deleted"
            );
            assert!(
                history.conversation(&grandchild_conversation_id).is_none(),
                "the anchor's subtree must be deleted with it"
            );
            assert!(
                history.conversation(&parent_conversation_id).is_some(),
                "the root must survive"
            );
            assert_eq!(
                TuiSessions::as_ref(ctx).focused_session_id(),
                Some(parent_session_id),
                "focus should return to the root agent after the subtree kill"
            );
        });
    });
}

#[test]
fn ctrl_c_on_child_conversation_without_tab_focus_arms_kill_window() {
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        let (_parent_view, _parent_session_id, parent_conversation_id) =
            add_orchestration_session(&mut app, &fixture, true);
        let (child_view, child_session_id, child_conversation_id) =
            add_orchestration_child(&mut app, &fixture, parent_conversation_id, "child");

        // Focus the child session but without tab-bar focus.
        app.update(|ctx| {
            TuiSessions::handle(ctx).update(ctx, |sessions, ctx| {
                sessions.focus_session(child_session_id, ctx);
            });
        });
        child_view.update(&mut app, |view, ctx| {
            view.conversation_selection.update(ctx, |selection, ctx| {
                selection.select_existing_conversation(
                    child_conversation_id,
                    AgentViewEntryOrigin::Tui,
                    ctx,
                );
            });
            view.refresh_orchestration_tab_state(ctx);
            // orchestration_tabs_focused stays false (default)
        });

        // First ctrl-c should arm the kill window, not delete the conversation.
        child_view.update(&mut app, |view, ctx| {
            view.handle_action(&TuiTerminalSessionAction::Interrupt, ctx);
        });

        app.read(|ctx| {
            assert!(
                child_view.as_ref(ctx).exit_confirmation.is_armed(),
                "first ctrl-c on a child conversation should arm the kill window"
            );
            assert_eq!(
                child_view.as_ref(ctx).child_kill_armed_conversation,
                Some(child_conversation_id),
                "child_kill_armed_conversation should target the viewed child"
            );
            // Conversation should NOT be deleted yet.
            assert!(
                BlocklistAIHistoryModel::as_ref(ctx)
                    .conversation(&child_conversation_id)
                    .is_some(),
                "child conversation must not be deleted after only one ctrl-c"
            );
        });
    });
}

#[test]
fn footer_shows_kill_hint_when_child_kill_window_is_armed() {
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        let (_parent_view, _parent_session_id, parent_conversation_id) =
            add_orchestration_session(&mut app, &fixture, true);
        let (child_view, child_session_id, child_conversation_id) =
            add_orchestration_child(&mut app, &fixture, parent_conversation_id, "child");

        // Focus the child session (no tab-bar focus).
        app.update(|ctx| {
            TuiSessions::handle(ctx).update(ctx, |sessions, ctx| {
                sessions.focus_session(child_session_id, ctx);
            });
        });
        child_view.update(&mut app, |view, ctx| {
            view.conversation_selection.update(ctx, |selection, ctx| {
                selection.select_existing_conversation(
                    child_conversation_id,
                    AgentViewEntryOrigin::Tui,
                    ctx,
                );
            });
            view.refresh_orchestration_tab_state(ctx);
        });

        // Footer before any ctrl-c: should NOT show the kill hint.
        let lines_before = render_footer_lines(&mut app, &child_view, 80);
        assert!(
            !lines_before.join("\n").contains(CTRL_C_KILL_CHILD_HINT),
            "footer should not show kill hint before arming: {lines_before:?}"
        );

        // First ctrl-c: arm the kill window.
        child_view.update(&mut app, |view, ctx| {
            view.handle_action(&TuiTerminalSessionAction::Interrupt, ctx);
        });

        // Footer after first ctrl-c: must show the child-kill hint, not the exit hint.
        let lines_after = render_footer_lines(&mut app, &child_view, 80);
        assert_eq!(
            lines_after,
            vec![CTRL_C_KILL_CHILD_HINT],
            "footer must show the kill-child hint when the kill window is armed"
        );
        let lines_str = lines_after.join("\n");
        assert!(
            !lines_str.contains(CTRL_C_EXIT_HINT),
            "kill-armed footer must not show the exit hint: {lines_str:?}"
        );
    });
}

#[test]
fn second_ctrl_c_within_window_kills_the_child_agent() {
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        let (_parent_view, parent_session_id, parent_conversation_id) =
            add_orchestration_session(&mut app, &fixture, true);
        let (child_view, child_session_id, child_conversation_id) =
            add_orchestration_child(&mut app, &fixture, parent_conversation_id, "child");

        // Focus the child session (no tab-bar focus).
        app.update(|ctx| {
            TuiSessions::handle(ctx).update(ctx, |sessions, ctx| {
                sessions.focus_session(child_session_id, ctx);
            });
        });
        child_view.update(&mut app, |view, ctx| {
            view.conversation_selection.update(ctx, |selection, ctx| {
                selection.select_existing_conversation(
                    child_conversation_id,
                    AgentViewEntryOrigin::Tui,
                    ctx,
                );
            });
            view.refresh_orchestration_tab_state(ctx);
        });

        // First ctrl-c: arms the kill window.
        child_view.update(&mut app, |view, ctx| {
            view.handle_action(&TuiTerminalSessionAction::Interrupt, ctx);
        });

        // Confirm the kill window is armed.
        assert!(
            child_view.read(&app, |view, _| view.exit_confirmation.is_armed()),
            "kill window must be armed after first ctrl-c"
        );

        // Second ctrl-c within the window: kills the child.
        child_view.update(&mut app, |view, ctx| {
            view.handle_action(&TuiTerminalSessionAction::Interrupt, ctx);
        });

        app.read(|ctx| {
            assert_eq!(
                child_view.as_ref(ctx).child_kill_armed_conversation,
                None,
                "kill window should be cleared after the kill"
            );
            assert!(
                !child_view.as_ref(ctx).exit_confirmation.is_armed(),
                "exit window should be cleared after the kill"
            );
            // Child conversation should be gone from history.
            assert!(
                BlocklistAIHistoryModel::as_ref(ctx)
                    .conversation(&child_conversation_id)
                    .is_none(),
                "child conversation should be deleted after double ctrl-c kill"
            );
            // Focus should return to the root/main session.
            assert_eq!(
                TuiSessions::as_ref(ctx).focused_session_id(),
                Some(parent_session_id),
                "focus should return to the root/main agent after kill"
            );
        });
    });
}

#[test]
fn ctrl_c_on_root_conversation_does_not_trigger_kill_path() {
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        let (parent_view, _parent_session_id, parent_conversation_id) =
            add_orchestration_session(&mut app, &fixture, true);
        let (_child_view, _child_session_id, _child_conversation_id) =
            add_orchestration_child(&mut app, &fixture, parent_conversation_id, "child");

        // The parent session is focused and pointing at the root conversation.
        // ctrl-c should follow the normal exit path, not the kill path.
        parent_view.update(&mut app, |view, ctx| {
            view.handle_action(&TuiTerminalSessionAction::Interrupt, ctx);
        });

        app.read(|ctx| {
            assert!(
                parent_view.as_ref(ctx).exit_confirmation.is_armed(),
                "ctrl-c on root should arm the normal exit window"
            );
            assert_eq!(
                parent_view.as_ref(ctx).child_kill_armed_conversation,
                None,
                "ctrl-c on root must not set child_kill_armed_conversation"
            );
        });
    });
}

#[test]
fn lapsed_kill_window_does_not_kill_child_on_next_ctrl_c() {
    // If the 1-second kill window lapses without a second press, the next
    // ctrl-c is a fresh first press and must arm a new window, NOT kill the child.
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        let (_parent_view, _parent_session_id, parent_conversation_id) =
            add_orchestration_session(&mut app, &fixture, true);
        let (child_view, child_session_id, child_conversation_id) =
            add_orchestration_child(&mut app, &fixture, parent_conversation_id, "child");

        // Focus the child session (no tab-bar focus).
        app.update(|ctx| {
            TuiSessions::handle(ctx).update(ctx, |sessions, ctx| {
                sessions.focus_session(child_session_id, ctx);
            });
        });
        child_view.update(&mut app, |view, ctx| {
            view.conversation_selection.update(ctx, |selection, ctx| {
                selection.select_existing_conversation(
                    child_conversation_id,
                    AgentViewEntryOrigin::Tui,
                    ctx,
                );
            });
            view.refresh_orchestration_tab_state(ctx);
        });

        // First ctrl-c: arms the kill window.
        child_view.update(&mut app, |view, ctx| {
            view.handle_action(&TuiTerminalSessionAction::Interrupt, ctx);
        });
        assert!(
            child_view.read(&app, |view, _| view.exit_confirmation.is_armed()),
            "kill window must be armed after first ctrl-c"
        );

        // Simulate window lapse: disarm + clear the armed conversation (as the
        // timer callback does), without triggering the kill.
        child_view.update(&mut app, |view, _| {
            view.exit_confirmation.disarm();
            view.child_kill_armed_conversation = None;
        });

        // Next ctrl-c: armed = None, should arm a new kill window rather than kill.
        child_view.update(&mut app, |view, ctx| {
            view.handle_action(&TuiTerminalSessionAction::Interrupt, ctx);
        });

        app.read(|ctx| {
            // Child conversation must still exist — lapse prevented the kill.
            assert!(
                BlocklistAIHistoryModel::as_ref(ctx)
                    .conversation(&child_conversation_id)
                    .is_some(),
                "child conversation must survive a ctrl-c after the window lapsed"
            );
            // A new kill window should now be armed, not executed.
            assert!(
                child_view.as_ref(ctx).exit_confirmation.is_armed(),
                "a new kill window should be armed by the post-lapse ctrl-c"
            );
            assert_eq!(
                child_view.as_ref(ctx).child_kill_armed_conversation,
                Some(child_conversation_id),
                "post-lapse ctrl-c should re-arm for the same child"
            );
        });
    });
}

#[test]
fn killing_child_does_not_exit_tui_parent_session_remains_alive() {
    // Killing a child agent must never cause the whole TUI to exit.
    // The parent session must remain focused and its conversation intact.
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        let (parent_view, parent_session_id, parent_conversation_id) =
            add_orchestration_session(&mut app, &fixture, true);
        let (child_view, child_session_id, child_conversation_id) =
            add_orchestration_child(&mut app, &fixture, parent_conversation_id, "child");

        // Kill via the tab-bar-focused single-press path.
        app.update(|ctx| {
            TuiSessions::handle(ctx).update(ctx, |sessions, ctx| {
                sessions.focus_session(child_session_id, ctx);
            });
        });
        child_view.update(&mut app, |view, ctx| {
            view.conversation_selection.update(ctx, |selection, ctx| {
                selection.select_existing_conversation(
                    child_conversation_id,
                    AgentViewEntryOrigin::Tui,
                    ctx,
                );
            });
            view.refresh_orchestration_tab_state(ctx);
            view.orchestration_tabs_focused = true;
            view.refresh_orchestration_tab_bar(ctx);
        });
        // Single ctrl-c kills the child.
        child_view.update(&mut app, |view, ctx| {
            view.handle_action(&TuiTerminalSessionAction::Interrupt, ctx);
        });

        // TUI must still be alive: the singleton models exist, the parent
        // session is focused, and the parent's conversation is untouched.
        app.read(|ctx| {
            assert!(
                ctx.has_singleton_model::<TuiSessions>(),
                "TuiSessions singleton must survive the child kill"
            );
            assert!(
                ctx.has_singleton_model::<TuiOrchestrationModel>(),
                "TuiOrchestrationModel singleton must survive the child kill"
            );
            assert_eq!(
                TuiSessions::as_ref(ctx).focused_session_id(),
                Some(parent_session_id),
                "parent session must be focused after child kill"
            );
            assert!(
                BlocklistAIHistoryModel::as_ref(ctx)
                    .conversation(&parent_conversation_id)
                    .is_some(),
                "parent conversation must survive child kill"
            );
            assert!(
                parent_view
                    .as_ref(ctx)
                    .child_kill_armed_conversation
                    .is_none(),
                "parent view must not have a stale kill window after kill"
            );
        });
    });
}

#[test]
fn status_email_fallback_chain_requires_a_validated_identity() {
    // Arm 1: non-empty email wins regardless of username.
    assert_eq!(
        super::resolve_status_email(
            Some("user@example.com".to_owned()),
            Some("display_name".to_owned()),
            Some("user-123".to_owned()),
            true,
        ),
        "user@example.com"
    );
    // Arm 2a: empty email falls back to a non-empty username.
    assert_eq!(
        super::resolve_status_email(
            Some(String::new()),
            Some("display_name".to_owned()),
            Some("user-123".to_owned()),
            true,
        ),
        "display_name"
    );
    // Arm 2b: None email falls back to a non-empty username.
    assert_eq!(
        super::resolve_status_email(
            None,
            Some("display_name".to_owned()),
            Some("user-123".to_owned()),
            true,
        ),
        "display_name"
    );
    // Arm 3: user ID is the final validated identity fallback.
    assert_eq!(
        super::resolve_status_email(None, None, Some("user-123".to_owned()), true),
        "user-123"
    );
    // Arm 4: a missing identity never degrades to bare "Signed in".
    assert_eq!(
        super::resolve_status_email(Some(String::new()), Some(String::new()), None, true),
        super::STATUS_NOT_SIGNED_IN
    );
    // Arm 5: an identity is ignored unless auth has been validated.
    assert_eq!(
        super::resolve_status_email(
            Some("user@example.com".to_owned()),
            Some("display_name".to_owned()),
            Some("user-123".to_owned()),
            false,
        ),
        super::STATUS_NOT_SIGNED_IN
    );
}

#[test]
fn resume_shell_commands_use_shared_tui_launcher() {
    assert_eq!(
        super::tui_resume_shell_command(Channel::Local, "conversation-token"),
        "./script/run-tui -- --resume conversation-token"
    );
    assert_eq!(
        super::tui_resume_shell_command(Channel::Preview, "conversation-token"),
        "warp-preview --resume conversation-token"
    );
}

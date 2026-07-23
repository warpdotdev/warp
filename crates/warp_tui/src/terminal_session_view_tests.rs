use std::rc::Rc;
use std::time::Duration;

use instant::Instant;
use warp::appearance::Appearance;
use warp::settings::{AISettings, TuiUsageDisplayMode};
use warp::terminal::model::ansi::{Handler, InputBufferValue};
use warp::tui_export::{
    AIAgentExchangeId, AIConversationAutoexecuteMode, AIConversationId, AgentViewEntryOrigin,
    BlockPadding, BlocklistAIHistoryModel, ConversationStatus, ConversationUsageTotals, Harness,
    LLMPreferences, PtyIntent, PtyIntentEvent, SizeInfo, SizeUpdate, TranscriptScope,
    export_conversation_markdown, register_tui_session_view_test_singletons, slash_commands,
};
use warp_core::settings::Setting as _;
use warp_editor::model::CoreEditorModel;
use warpui::platform::WindowStyle;
use warpui::{
    AddWindowOptions, EntityIdMap, ModelHandle, ReadModel, SingletonEntity, UpdateModel, ViewHandle,
};
use warpui_core::r#async::Timer;
use warpui_core::elements::tui::{
    Color, TuiBuffer, TuiBufferExt, TuiConstrainedBox, TuiConstraint, TuiContainer, TuiElement,
    TuiEvent, TuiEventContext, TuiLayoutContext, TuiPaintContext, TuiPaintSurface, TuiPoint,
    TuiRect, TuiScene, TuiScreenPosition, TuiSize, TuiStyle, TuiText,
};
use warpui_core::event::ModifiersState;
use warpui_core::keymap::{Context, Keystroke, Trigger};
use warpui_core::presenter::tui::TuiPresenter;
use warpui_core::telemetry::{EventPayload, flush_events};
use warpui_core::{App, AppContext, TuiView, TypedActionView as _, WindowInvalidation};

use super::{
    AUTO_APPROVE_FEEDBACK_DURATION, AUTO_APPROVE_TOGGLE_BINDING_NAME,
    COST_CONVERSATION_IN_PROGRESS_HINT, COST_EMPTY_CONVERSATION_HINT,
    COST_NO_ACTIVE_CONVERSATION_HINT, CTRL_C_EXIT_HINT, ConversationRestoreState, FooterSegments,
    INLINE_MENU_TOP_PADDING_ROWS, LOADING_CONVERSATION_HINT, LOG_BUNDLE_FAILED_HINT,
    SHELL_MODE_HINT, TuiConversationRestoreOrigin, TuiTerminalSessionAction,
    TuiTerminalSessionEvent, TuiTerminalSessionView, cost_command_unavailable_hint,
    export_file_success_message, log_bundle_success_message, raw_prompt_if_not_blank,
    render_status_footer_row,
};
use crate::autoupdate::TuiAutoupdater;
use crate::inline_menu::MAX_INLINE_MENU_ROWS;
use crate::keybindings::{
    CONTEXTUAL_PLAN_TOGGLE_BINDING_NAME, KEYBOARD_ENHANCEMENT_AVAILABLE_FLAG,
    PLAN_TOGGLE_AVAILABLE_FLAG, PLAN_TOGGLE_BINDING_NAME, TUI_BINDING_GROUP,
};
use crate::orchestrated_agent_identity_styling::AgentIdentity;
use crate::orchestration_model::TuiOrchestrationModel;
use crate::orchestration_tab_bar::{
    ORCHESTRATION_TAB_BAR_FOCUSED_FLAG, orchestration_tab_icon, render_orchestration_tab_footer,
};
use crate::root_view::RootTuiView;
use crate::session_registry::{TuiSessionId, TuiSessions};
use crate::terminal_block::{block_content_rows, should_render_terminal_block};
use crate::terminal_use::TuiInputTarget;
use crate::test_fixtures::{add_test_semantic_selection, add_test_terminal_session};
use crate::transcript_view::TRANSCRIPT_BLOCK_SPACING;
use crate::tui_builder::TuiUiBuilder;
use crate::usage::UsageToggle;

struct FocusTestFixture {
    window_id: warpui_core::WindowId,
    sessions: ModelHandle<TuiSessions>,
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
fn render_retained_session(
    app: &App,
    view: &ViewHandle<super::TuiTerminalSessionView>,
    width: u16,
    height: u16,
) -> (Box<dyn TuiElement>, Rc<TuiScene>, TuiBuffer) {
    app.read(|ctx| {
        let mut element = ctx
            .render_tui_view(view.window_id(ctx), view.id())
            .expect("session view should render");
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
    })
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

/// Locates the footer's active-model label in the rendered buffer, returning
/// the (column, row) of its first cell. Counts chars (not bytes) so multi-byte
/// glyphs earlier in the footer row don't shift the column.
fn model_label_position(buffer: &TuiBuffer, model_name: &str) -> (u16, u16) {
    let lines = buffer.to_lines();
    for (row, line) in lines.iter().enumerate() {
        if let Some(byte_offset) = line.find(model_name) {
            let col = line[..byte_offset].chars().count() as u16;
            return (col, row as u16);
        }
    }
    panic!(
        "model label {:?} not found in rendered footer:\n{}",
        model_name,
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
                shell_mode: false,
                model_label: Some(
                    TuiText::new("TestModel")
                        .with_style(builder.primary_text_style())
                        .truncate()
                        .finish(),
                ),
                cwd: None,
                branch: None,
                usage: Some(usage),
                diff_additions: 0,
                diff_deletions: 0,
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
            cost_in_cents: 3.2,
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
fn auto_approve_actions_control_transient_color_feedback() {
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
        let (label_col, label_row) = model_label_position(&buffer, &model_name);
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
    let mut presenter = TuiPresenter::new();
    app.update(|ctx| {
        let mut invalidation = WindowInvalidation::default();
        invalidation.updated.insert(view.id());
        invalidation
            .updated
            .extend(view.as_ref(ctx).child_view_ids(ctx));
        presenter.invalidate(&invalidation, ctx, view.window_id(ctx));
        presenter
            .present(ctx, view, TuiRect::new(0, 0, width, height))
            .buffer
            .to_lines()
    })
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
                super::TransientHintTone::Success
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
                super::TransientHintTone::Success
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
            .find(|(_, line)| line.contains('┌') || line.contains('─'))
            .map(|(index, _)| index)
            .expect("bootstrap input border should render below the status");
        assert!(status_index < input_index);
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
            view.handle_submitted("draft".to_owned(), ctx);
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
fn long_running_command_keeps_input_hidden() {
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        let (view, _) = add_focus_test_session(&mut app, &fixture, true);
        view.update(&mut app, |view, _| {
            view.terminal_model
                .lock()
                .simulate_long_running_block("cat", "");
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
                .any(|line| line.contains('┌') || line.contains('─')),
            "LRC must keep the input editor hidden:\n{}",
            lines.join("\n")
        );
        // The interrupt affordance renders as a ghosted row in the input's
        // slot while the command owns input.
        assert!(
            lines
                .iter()
                .any(|line| line.trim() == crate::input_hints::LONG_RUNNING_COMMAND_HINT),
            "LRC must render the interrupt hint row:\n{}",
            lines.join("\n")
        );
    });
}

/// Visible startup-script execution also routes input to the PTY, but it is
/// not a user-controlled command: the interrupt hint row must not appear.
#[test]
fn visible_startup_script_shows_no_interrupt_hint() {
    App::test((), |mut app| async move {
        let fixture = focus_test_fixture(&mut app);
        let (view, _) = add_focus_test_session(&mut app, &fixture, true);
        view.update(&mut app, |view, _| {
            let mut terminal_model = view.terminal_model.lock();
            terminal_model.block_list_mut().reinit_shell();
            terminal_model.update_blockheight_items(TRANSCRIPT_BLOCK_SPACING.block_padding, 0.0);
            // Advance past WarpInput, then leave an unfinished startup-script
            // block with visible output owning PTY input.
            terminal_model.simulate_block("bootstrap", "");
            terminal_model.simulate_long_running_block("shell init", "startup output\r\n");
        });
        assert!(
            view.read(&app, |view, _| view.input_target().pty_owns_input()),
            "fixture should route input to the PTY during the visible startup script"
        );

        let lines = render_session(&mut app, &view, 80, 40);
        assert!(
            !lines
                .iter()
                .any(|line| line.trim() == crate::input_hints::LONG_RUNNING_COMMAND_HINT),
            "startup-script execution must not advertise the interrupt hint:\n{}",
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
            presenter.present(ctx, &view, TuiRect::new(0, 0, 120, 40))
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

fn render_footer_lines(
    app: &mut App,
    view: &ViewHandle<super::TuiTerminalSessionView>,
    width: u16,
) -> Vec<String> {
    app.update(|ctx| {
        let footer = view.as_ref(ctx).render_footer(ctx).finish();
        render_element(footer, ctx, width).to_lines()
    })
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
        !row.contains(" ↬ "),
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
                    cost_in_cents: 0.0,
                },
                ctx,
                |_, _| {},
            );
            let row = render_status_footer_row(
                FooterSegments {
                    shell_mode: false,
                    model_label: Some(
                        TuiText::new("TestModel")
                            .with_style(builder.primary_text_style())
                            .truncate()
                            .finish(),
                    ),
                    cwd: Some("/home/user/warp".to_owned()),
                    branch: Some("main".to_owned()),
                    usage: Some(usage),
                    diff_additions: 3,
                    diff_deletions: 1,
                },
                &builder,
            )
            .finish();
            let lines = render_element(row, ctx, 120).to_lines();
            let line = lines.join("\n");

            assert_eq!(
                lines,
                vec!["TestModel /home/user/warp ↬ main • 2.5 credits • +3 -1"],
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
            let usage = UsageToggle::default().render_entry(
                TuiUsageDisplayMode::default(),
                ConversationUsageTotals {
                    credits_spent: 2.5,
                    cost_in_cents: 0.0,
                },
                ctx,
                |_, _| {},
            );
            let row = render_status_footer_row(
                FooterSegments {
                    shell_mode: true,
                    model_label: Some(
                        TuiText::new("TestModel")
                            .with_style(builder.primary_text_style())
                            .truncate()
                            .finish(),
                    ),
                    cwd: Some("/home/user/warp".to_owned()),
                    branch: Some("main".to_owned()),
                    usage: Some(usage),
                    diff_additions: 3,
                    diff_deletions: 1,
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
                vec![format!("{SHELL_MODE_HINT} /home/user/warp ↬ main • +3 -1")],
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
            row.starts_with("auto (cost-efficient) "),
            "the model-led status row renders in place of the callout: {row}"
        );
    });
}
#[test]
fn interrupt_event_projects_to_high_level_pty_intent() {
    let event = TuiTerminalSessionEvent::InterruptPty;
    assert!(matches!(event.pty_intent(), Some(PtyIntent::Interrupt)));
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
                ("tui:orchestration_tabs:previous", "shift-tab"),
                ("tui:orchestration_tabs:next", "right"),
                ("tui:orchestration_tabs:next", "tab"),
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
                ctx,
            );
            history.set_active_conversation_id(conversation_id, session_id.surface_id(), ctx);
            conversation_id
        })
    });
    (view, session_id, conversation_id)
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
                    .main_tab_key(),
                Some(parent_conversation_id.to_string()),
                "tab bar should expose the parent as the main tab"
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
                    .main_tab_key(),
                Some(parent_conversation_id.to_string()),
                "root tab bar should expose the root as the main tab"
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

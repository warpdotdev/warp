//! Regression tests for [`TuiInputView`] cursor/coordinate + kill logic.
//!
//! These drive a real [`CodeEditorModel`] (TUI char-cell mode) behind a real
//! [`TuiInputView`] so they exercise the exact render/layout/cursor path the
//! presenter uses, not a reimplementation of it.
use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;

use string_offset::CharOffset;
use warp::appearance::Appearance;
use warp::editor::CodeEditorModel;
use warp::settings::AISettingsChangedEvent;
use warp::tui_export::{
    AcceptSlashCommandOrSavedPrompt, BlocklistAIInputModel, ConversationSelectionEvent,
    InputConfig, InputModePolicy, InputType, LLMId, PolicyConfigUpdate, SlashCommandId,
    SlashCommandMixer,
};
use warp_editor::model::CoreEditorModel;
use warpui::EntityIdMap;
use warpui_core::elements::tui::{
    TuiBuffer, TuiBufferExt, TuiConstraint, TuiElement, TuiEvent, TuiEventContext,
    TuiLayoutContext, TuiPaintContext, TuiPaintSurface, TuiPoint, TuiRect, TuiScene,
    TuiScreenPosition, TuiSize,
};
use warpui_core::event::{KeyEventDetails, ModifiersState};
use warpui_core::keymap::Keystroke;
use warpui_core::platform::WindowStyle;
use warpui_core::{
    AddWindowOptions, App, AppContext, Entity, ModelHandle, TuiView, TypedActionView, ViewHandle,
};

use super::{
    input_keymap_context, TuiInputAction, TuiInputView, TuiInputViewEvent,
    INPUT_HANDLES_ESCAPE_FLAG,
};
use crate::editor_element::{TuiEditorAction, TuiEditorElement};
use crate::editor_interaction::TuiEditorCommand;
use crate::inline_menu::{
    TuiInlineMenu, TuiInlineMenuAccepted, TuiInlineMenuHandle, TuiInlineMenuHeader,
    TuiInlineMenuSnapshot, TuiInlineMenuStatus,
};
use crate::input_mode_policy::AI_LOCKED_CONFIG;
use crate::input_suggestions_mode::{TuiInputSuggestionsMode, TuiInputSuggestionsModeModel};
use crate::model_menu::TuiModelMenuModel;
use crate::slash_commands::{TuiSlashCommandModel, TuiSlashCommandRow};
use crate::test_fixtures::add_test_semantic_selection;
use crate::tui_builder::TuiUiBuilder;

const W: u16 = 80;

struct TestInputModePolicy;

impl InputModePolicy for TestInputModePolicy {
    fn initial_config(&self, _app: &AppContext) -> InputConfig {
        AI_LOCKED_CONFIG
    }

    fn allows_locked_ai_input(&self, _app: &AppContext) -> bool {
        true
    }

    fn is_autodetection_enabled(&self, _app: &AppContext) -> bool {
        false
    }

    fn config_on_conversation_selection_changed(
        &self,
        _event: &ConversationSelectionEvent,
        _current: InputConfig,
        _app: &AppContext,
    ) -> Option<PolicyConfigUpdate> {
        None
    }

    fn config_on_ai_settings_changed(
        &self,
        _event: &AISettingsChangedEvent,
        _current: InputConfig,
        _is_autodetection_enabled_for_current_context: bool,
        _app: &AppContext,
    ) -> Option<PolicyConfigUpdate> {
        None
    }
}

#[test]
fn input_escape_context_is_present_only_while_escape_is_handled() {
    let closed = input_keymap_context(false, false, false);
    assert!(closed.set.contains("TuiInputView"));
    assert!(!closed.set.contains(INPUT_HANDLES_ESCAPE_FLAG));
    assert!(!closed
        .set
        .contains(crate::keybindings::PLAN_TOGGLE_AVAILABLE_FLAG));
    assert!(!closed
        .set
        .contains(crate::keybindings::KEYBOARD_ENHANCEMENT_AVAILABLE_FLAG));

    let open = input_keymap_context(true, true, true);
    assert!(open.set.contains("TuiInputView"));
    assert!(open.set.contains(INPUT_HANDLES_ESCAPE_FLAG));
    assert!(open
        .set
        .contains(crate::keybindings::PLAN_TOGGLE_AVAILABLE_FLAG));
    assert!(open
        .set
        .contains(crate::keybindings::KEYBOARD_ENHANCEMENT_AVAILABLE_FLAG));
}

fn add_suggestions_mode(
    ctx: &mut AppContext,
    initial_mode: TuiInputSuggestionsMode,
) -> ModelHandle<TuiInputSuggestionsModeModel> {
    let mode = ctx.add_model(|_| TuiInputSuggestionsModeModel::new());
    mode.update(ctx, |mode, ctx| mode.set_mode(initial_mode, ctx));
    mode
}

#[test]
fn slash_command_argument_hint_renders_after_menu_closes() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let (view, menu_model, _) = build_view_with_inline_menu(ctx);
            let input = "/export-to-file ";
            view.update(ctx, |view, ctx| view.set_text(input, ctx));
            menu_model.update(ctx, |model, _| {
                model.set_argument_hint_text_for_test(Some("<optional filename>"));
            });
            menu_model.update(ctx, |model, ctx| {
                model.accept_selected(ctx);
                assert!(!model.is_open(ctx));
            });

            let buffer = render_input_buffer(&view, ctx);
            let line = &buffer.to_lines()[0];
            assert!(line.starts_with("/export-to-file <optional filename>"));

            let hint_column = input.chars().count() as u16;
            let expected = TuiUiBuilder::from_app(ctx)
                .dim_text_style()
                .fg
                .expect("ghost text has a foreground");
            assert_eq!(buffer[(hint_column, 0)].fg, expected);
        });
    });
}

fn render_input_buffer(view: &ViewHandle<TuiInputView>, ctx: &AppContext) -> TuiBuffer {
    let mut element = view.as_ref(ctx).render_element(ctx);
    let mut rendered_views = EntityIdMap::default();
    let mut layout_ctx = TuiLayoutContext {
        rendered_views: &mut rendered_views,
    };
    let size = element.layout(
        TuiConstraint::loose(TuiSize::new(W, 20)),
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

fn render_element_lines(
    mut element: Box<dyn TuiElement>,
    ctx: &AppContext,
    width: u16,
    height: u16,
) -> Vec<String> {
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
    buffer.to_lines()
}

struct TestConversationMenu {
    is_open: bool,
    suggestions_mode: ModelHandle<TuiInputSuggestionsModeModel>,
}

impl Entity for TestConversationMenu {
    type Event = ();
}

#[derive(Clone)]
struct TestConversationMenuHandle(ModelHandle<TestConversationMenu>);

impl TuiInlineMenuHandle for TestConversationMenuHandle {
    fn mode(&self) -> TuiInputSuggestionsMode {
        TuiInputSuggestionsMode::ConversationMenu
    }

    fn is_open(&self, ctx: &AppContext) -> bool {
        self.0.as_ref(ctx).is_open
            && self.0.as_ref(ctx).suggestions_mode.as_ref(ctx).mode()
                == TuiInputSuggestionsMode::ConversationMenu
    }

    fn open(&self, ctx: &mut AppContext) {
        self.0.update(ctx, |menu, ctx| {
            if menu.suggestions_mode.update(ctx, |mode, ctx| {
                mode.try_open(TuiInputSuggestionsMode::ConversationMenu, ctx)
            }) {
                menu.is_open = true;
            }
        });
    }

    fn input_highlight_range(&self, _ctx: &AppContext) -> Option<Range<CharOffset>> {
        None
    }

    fn input_argument_hint_text(&self, _ctx: &AppContext) -> Option<&'static str> {
        None
    }

    fn select_previous(&self, _ctx: &mut AppContext) {}

    fn select_next(&self, _ctx: &mut AppContext) {}

    fn accept(&self, _ctx: &mut AppContext) -> Option<TuiInlineMenuAccepted> {
        None
    }

    fn dismiss(&self, ctx: &mut AppContext) {
        self.0.update(ctx, |menu, ctx| {
            menu.is_open = false;
            menu.suggestions_mode.update(ctx, |mode, ctx| {
                mode.close_if_active(TuiInputSuggestionsMode::ConversationMenu, ctx);
            });
        });
    }

    fn snapshot(&self, ctx: &AppContext) -> Option<TuiInlineMenuSnapshot> {
        self.is_open(ctx).then(|| TuiInlineMenuSnapshot {
            header: Some(TuiInlineMenuHeader {
                title: Some("Conversations".to_owned()),
                tabs: Vec::new(),
            }),
            rows: Vec::new(),
            selected_index: None,
            scroll_offset: 0,
            max_visible_rows: 8,
            status: Some(TuiInlineMenuStatus::Empty(
                "No conversations found".to_owned(),
            )),
        })
    }
}

#[test]
fn recognized_slash_command_prefix_matches_menu_color_after_menu_closes() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let (view, menu_model, _) = build_view_with_inline_menu(ctx);
            view.update(ctx, |view, ctx| view.set_text("/plan argument", ctx));
            menu_model.update(ctx, |model, _| {
                model.set_highlighted_prefix_len_for_test(Some(5));
            });
            menu_model.update(ctx, |model, ctx| {
                model.accept_selected(ctx);
                assert!(!model.is_open(ctx));
            });

            let buffer = render_input_buffer(&view, ctx);
            let expected = TuiUiBuilder::from_app(ctx)
                .slash_command_text_style()
                .fg
                .expect("slash-command text has a foreground");
            assert_eq!(buffer[(0, 0)].fg, expected);
            assert_eq!(buffer[(4, 0)].fg, expected);
            assert_ne!(buffer[(5, 0)].fg, expected);
        });
    });
}

fn build_view(ctx: &mut AppContext) -> ViewHandle<TuiInputView> {
    // `CodeEditorModel::new_tui` reads syntax colors from the `Appearance`
    // singleton, so register a mock one before constructing the editor.
    ctx.add_singleton_model(|_| Appearance::mock());
    add_test_semantic_selection(ctx);
    let input_mode = BlocklistAIInputModel::mock(Rc::new(TestInputModePolicy), ctx);
    let suggestions_mode = add_suggestions_mode(ctx, TuiInputSuggestionsMode::Closed);
    let (_window_id, view) = ctx.add_tui_window(
        AddWindowOptions {
            window_style: WindowStyle::NotStealFocus,
            ..Default::default()
        },
        |ctx| {
            let model = ctx.add_model(|ctx| CodeEditorModel::new_tui(W, ctx));
            TuiInputView::new_for_test(model, input_mode, suggestions_mode, Vec::new(), ctx)
        },
    );
    view
}

fn build_view_with_conversation_menu(
    ctx: &mut AppContext,
) -> (
    ViewHandle<TuiInputView>,
    ModelHandle<TestConversationMenu>,
    TuiInlineMenu,
) {
    ctx.add_singleton_model(|_| Appearance::mock());
    add_test_semantic_selection(ctx);
    let input_model = ctx.add_model(|ctx| CodeEditorModel::new_tui(W, ctx));
    let input_mode = BlocklistAIInputModel::mock(Rc::new(TestInputModePolicy), ctx);
    let suggestions_mode = add_suggestions_mode(ctx, TuiInputSuggestionsMode::Closed);
    let menu_model = ctx.add_model(|_| TestConversationMenu {
        is_open: false,
        suggestions_mode: suggestions_mode.clone(),
    });
    let inline_menu = TuiInlineMenu::new(TestConversationMenuHandle(menu_model.clone()));
    let inline_menu_for_view = inline_menu.clone();
    let (_window_id, view) = ctx.add_tui_window(
        AddWindowOptions {
            window_style: WindowStyle::NotStealFocus,
            ..Default::default()
        },
        move |ctx| {
            TuiInputView::new_for_test(
                input_model,
                input_mode,
                suggestions_mode,
                vec![inline_menu_for_view],
                ctx,
            )
        },
    );
    (view, menu_model, inline_menu)
}

fn build_view_with_inline_menu(
    ctx: &mut AppContext,
) -> (
    ViewHandle<TuiInputView>,
    ModelHandle<TuiSlashCommandModel>,
    [SlashCommandId; 2],
) {
    ctx.add_singleton_model(|_| Appearance::mock());
    add_test_semantic_selection(ctx);
    let input_model = ctx.add_model(|ctx| CodeEditorModel::new_tui(W, ctx));
    let input_mode = BlocklistAIInputModel::mock(Rc::new(TestInputModePolicy), ctx);
    let suggestions_mode = add_suggestions_mode(ctx, TuiInputSuggestionsMode::SlashCommands);
    let mixer = ctx.add_model(|_| SlashCommandMixer::new());
    let ids = [SlashCommandId::new(), SlashCommandId::new()];
    let rows = ids
        .iter()
        .enumerate()
        .map(|(index, id)| TuiSlashCommandRow {
            title: format!("Command {index}"),
            description: None,
            action: AcceptSlashCommandOrSavedPrompt::SlashCommand { id: *id },
        })
        .collect();
    let menu_model = ctx.add_model(|_| {
        TuiSlashCommandModel::new_for_test(
            input_model.clone(),
            suggestions_mode.clone(),
            mixer,
            rows,
            0,
        )
    });
    let inline_menu = TuiInlineMenu::new(menu_model.clone());
    let (_window_id, view) = ctx.add_tui_window(
        AddWindowOptions {
            window_style: WindowStyle::NotStealFocus,
            ..Default::default()
        },
        move |ctx| {
            TuiInputView::new_for_test(
                input_model,
                input_mode,
                suggestions_mode,
                vec![inline_menu],
                ctx,
            )
        },
    );
    (view, menu_model, ids)
}

fn build_view_with_model_menu(
    ctx: &mut AppContext,
) -> (
    ViewHandle<TuiInputView>,
    ModelHandle<TuiModelMenuModel>,
    LLMId,
) {
    ctx.add_singleton_model(|_| Appearance::mock());
    add_test_semantic_selection(ctx);
    let input_model = ctx.add_model(|ctx| CodeEditorModel::new_tui(W, ctx));
    let input_mode = BlocklistAIInputModel::mock(Rc::new(TestInputModePolicy), ctx);
    let suggestions_mode = add_suggestions_mode(ctx, TuiInputSuggestionsMode::ModelSelector);
    let id = LLMId::from("gpt-5");
    let id_for_model = id.clone();
    let menu_model = ctx.add_model(|_| {
        TuiModelMenuModel::new_for_test(
            input_model.clone(),
            suggestions_mode.clone(),
            vec![(id_for_model, true)],
            0,
        )
    });
    let inline_menu = TuiInlineMenu::new(menu_model.clone());
    let (_window_id, view) = ctx.add_tui_window(
        AddWindowOptions {
            window_style: WindowStyle::NotStealFocus,
            ..Default::default()
        },
        move |ctx| {
            TuiInputView::new_for_test(
                input_model,
                input_mode,
                suggestions_mode,
                vec![inline_menu],
                ctx,
            )
        },
    );
    (view, menu_model, id)
}

fn selected_slash_command_id(
    menu_model: &ModelHandle<TuiSlashCommandModel>,
    ctx: &AppContext,
) -> Option<SlashCommandId> {
    match menu_model.as_ref(ctx).selected_action()? {
        AcceptSlashCommandOrSavedPrompt::SlashCommand { id } => Some(id),
        AcceptSlashCommandOrSavedPrompt::SavedPrompt { .. }
        | AcceptSlashCommandOrSavedPrompt::Skill { .. } => None,
    }
}

#[test]
fn inline_menu_navigation_routes_before_editor_navigation() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let (view, menu_model, ids) = build_view_with_inline_menu(ctx);
            assert_eq!(selected_slash_command_id(&menu_model, ctx), Some(ids[0]));

            dispatch(
                &view,
                ctx,
                &[TuiInputAction::EditorCommand(TuiEditorCommand::MoveDown)],
            );
            assert_eq!(selected_slash_command_id(&menu_model, ctx), Some(ids[1]));

            dispatch(
                &view,
                ctx,
                &[TuiInputAction::EditorCommand(TuiEditorCommand::MoveUp)],
            );
            assert_eq!(selected_slash_command_id(&menu_model, ctx), Some(ids[0]));
        });
    });
}

#[test]
fn inline_menu_accept_dismisses_before_emitting_unchanged_payload() {
    App::test((), |mut app| async move {
        let (view, menu_model, ids, accepted) = app.update(|ctx| {
            let (view, menu_model, ids) = build_view_with_inline_menu(ctx);
            let accepted = Rc::new(RefCell::new(Vec::new()));
            let accepted_for_subscription = accepted.clone();
            let menu_for_subscription = menu_model.clone();
            ctx.subscribe_to_view(&view, move |_, event, ctx| {
                if let TuiInputViewEvent::AcceptedSlashCommand(
                    AcceptSlashCommandOrSavedPrompt::SlashCommand { id },
                ) = event
                {
                    accepted_for_subscription
                        .borrow_mut()
                        .push((*id, !menu_for_subscription.as_ref(ctx).is_open(ctx)));
                }
            });
            (view, menu_model, ids, accepted)
        });
        app.update(|ctx| {
            dispatch(&view, ctx, &[TuiInputAction::Submit]);
        });
        app.read(|ctx| {
            assert_eq!(accepted.borrow().as_slice(), &[(ids[0], true)]);
            assert!(!menu_model.as_ref(ctx).is_open(ctx));
        });
    });
}

#[test]
fn model_menu_accept_emits_selected_id_and_stays_open_for_persistence() {
    App::test((), |mut app| async move {
        let (view, menu_model, id, accepted) = app.update(|ctx| {
            let (view, menu_model, id) = build_view_with_model_menu(ctx);
            let accepted = Rc::new(RefCell::new(Vec::new()));
            let accepted_for_subscription = accepted.clone();
            ctx.subscribe_to_view(&view, move |_, event, _| {
                if let TuiInputViewEvent::AcceptedModel(id) = event {
                    accepted_for_subscription.borrow_mut().push(id.clone());
                }
            });
            (view, menu_model, id, accepted)
        });

        app.update(|ctx| dispatch(&view, ctx, &[TuiInputAction::Submit]));
        app.read(|ctx| {
            assert_eq!(accepted.borrow().as_slice(), &[id]);
            assert!(menu_model.as_ref(ctx).is_open(ctx));
        });
    });
}

#[test]
fn escape_dismisses_menu_and_closed_menu_submit_falls_through() {
    App::test((), |mut app| async move {
        let (view, menu_model, submitted) = app.update(|ctx| {
            let (view, menu_model, _) = build_view_with_inline_menu(ctx);
            type_str(&view, ctx, "!");
            assert!(view.as_ref(ctx).is_shell_mode(ctx));
            dispatch(&view, ctx, &[TuiInputAction::HandleEscape]);
            assert!(
                view.as_ref(ctx).is_shell_mode(ctx),
                "the first escape must dismiss the menu before exiting shell mode"
            );

            let submitted = Rc::new(RefCell::new(Vec::new()));
            let submitted_for_subscription = submitted.clone();
            ctx.subscribe_to_view(&view, move |_, event, _| {
                if let TuiInputViewEvent::Submitted(text) = event {
                    submitted_for_subscription.borrow_mut().push(text.clone());
                }
            });
            (view, menu_model, submitted)
        });

        app.update(|ctx| {
            assert!(!menu_model.as_ref(ctx).is_open(ctx));
            assert!(view
                .as_ref(ctx)
                .keymap_context(ctx)
                .set
                .contains(INPUT_HANDLES_ESCAPE_FLAG));
            type_str(&view, ctx, "prompt");
            dispatch(&view, ctx, &[TuiInputAction::Submit]);
        });
        app.read(|ctx| {
            assert_eq!(submitted.borrow().as_slice(), &["prompt"]);
            assert_eq!(text(&view, ctx), "prompt");
        });
    });
}

#[test]
fn multiline_paste_inserts_without_submitting_until_enter() {
    App::test((), |mut app| async move {
        let (view, submitted) = app.update(|ctx| {
            let view = build_view(ctx);
            let submitted = Rc::new(RefCell::new(Vec::new()));
            let submitted_for_subscription = submitted.clone();
            ctx.subscribe_to_view(&view, move |_, event, _| {
                if let TuiInputViewEvent::Submitted(text) = event {
                    submitted_for_subscription.borrow_mut().push(text.clone());
                }
            });
            (view, submitted)
        });
        let payload = "USER:\nhello\n\nAGENT:\nHi!\n";

        app.update(|ctx| {
            dispatch(
                &view,
                ctx,
                &[TuiInputAction::Editor(TuiEditorAction::InsertText(
                    payload.to_owned(),
                ))],
            );
        });
        app.read(|ctx| {
            assert_eq!(text(&view, ctx), payload);
            assert!(
                submitted.borrow().is_empty(),
                "paste must not emit a submission"
            );
        });

        app.update(|ctx| {
            dispatch(&view, ctx, &[TuiInputAction::Submit]);
        });
        assert_eq!(submitted.borrow().as_slice(), &[payload]);
    });
}

fn dispatch(view: &ViewHandle<TuiInputView>, ctx: &mut AppContext, actions: &[TuiInputAction]) {
    view.update(ctx, |v, vctx| {
        for action in actions {
            v.handle_action(action, vctx);
        }
    });
}

#[test]
fn clear_selection_collapses_to_head_without_changing_text() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let view = build_view(ctx);
            type_str(&view, ctx, "hello world");
            mouse(&view, ctx, &left_down(0, 0, 1, false));
            mouse(&view, ctx, &left_drag(5, 0));
            assert_eq!(selected_text(&view, ctx).as_deref(), Some("hello"));

            view.update(ctx, |view, ctx| view.clear_selection(ctx));

            assert_eq!(selected_text(&view, ctx), None);
            assert_eq!(text(&view, ctx), "hello world");
            assert!(!is_drag_selecting(&view, ctx));
        });
    });
}

fn type_str(view: &ViewHandle<TuiInputView>, ctx: &mut AppContext, s: &str) {
    let actions: Vec<TuiInputAction> = s
        .chars()
        .map(|c| TuiInputAction::Editor(TuiEditorAction::InsertChar(c)))
        .collect();
    dispatch(view, ctx, &actions);
}

/// Render the view, lay it out at width `W`, and return `(cursor, height)`.
fn cursor_and_height(
    view: &ViewHandle<TuiInputView>,
    ctx: &AppContext,
) -> (Option<(u16, u16)>, u16) {
    let mut element = view.as_ref(ctx).render(ctx);
    let mut rendered_views = EntityIdMap::default();
    let mut lctx = TuiLayoutContext {
        rendered_views: &mut rendered_views,
    };
    let size = element.layout(TuiConstraint::loose(TuiSize::new(W, 20)), &mut lctx, ctx);
    let area = TuiRect::new(0, 0, size.width, size.height);
    let mut buffer = TuiBuffer::empty(area);
    let mut paint_ctx = TuiPaintContext::new(&mut rendered_views);
    {
        let mut surface = TuiPaintSurface::new(&mut buffer);
        element.render(TuiScreenPosition::new(0, 0), &mut surface, &mut paint_ctx);
    }
    let cursor = paint_ctx
        .terminal_cursor()
        .and_then(|point| Some((u16::try_from(point.x).ok()?, u16::try_from(point.y).ok()?)));
    (cursor, size.height)
}

fn text(view: &ViewHandle<TuiInputView>, ctx: &AppContext) -> String {
    let v = view.as_ref(ctx);
    let inner = v.model().as_ref(ctx);
    let buffer = inner.content().as_ref(ctx);
    if buffer.is_empty() {
        String::new()
    } else {
        buffer.text().into_string()
    }
}

/// The currently selected substring, or `None` when there is no selection.
fn selected_text(view: &ViewHandle<TuiInputView>, ctx: &AppContext) -> Option<String> {
    let range = view.as_ref(ctx).selection_range(ctx)?;
    // `selection_range` is a 1-based gap range; convert to 0-based plain-text indices.
    let start = range.start.as_usize().saturating_sub(1);
    let end = range.end.as_usize().saturating_sub(1);
    let full = text(view, ctx);
    Some(full.chars().skip(start).take(end - start).collect())
}

/// The char-cell viewport's first visible display row (model-owned scroll).
fn scroll_offset(view: &ViewHandle<TuiInputView>, ctx: &AppContext) -> u32 {
    view.as_ref(ctx)
        .model()
        .as_ref(ctx)
        .render_state()
        .as_ref(ctx)
        .char_cell()
        .expect("TUI editor model is char-cell")
        .scroll_offset()
}

/// Whether a mouse drag-selection is pending on the selection model.
fn is_drag_selecting(view: &ViewHandle<TuiInputView>, ctx: &AppContext) -> bool {
    view.as_ref(ctx)
        .model()
        .as_ref(ctx)
        .selection_model()
        .as_ref(ctx)
        .has_pending_selection()
}

#[test]
fn move_left_on_empty_buffer_opens_conversation_menu() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let (view, menu_model, inline_menu) = build_view_with_conversation_menu(ctx);
            assert!(!menu_model.as_ref(ctx).is_open);

            dispatch(
                &view,
                ctx,
                &[TuiInputAction::EditorCommand(TuiEditorCommand::MoveLeft)],
            );

            assert!(menu_model.as_ref(ctx).is_open);
            let lines = render_element_lines(
                inline_menu
                    .render(ctx)
                    .expect("open conversation menu should render"),
                ctx,
                40,
                4,
            );
            assert_eq!(lines[0].trim(), "Conversations");
            assert!(lines
                .iter()
                .any(|line| line.trim() == "No conversations found"));
            assert_eq!(cursor_and_height(&view, ctx).0, Some((0, 0)));
        });
    });
}

#[test]
fn move_left_on_non_empty_buffer_only_moves_cursor() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let (view, menu_model, _) = build_view_with_conversation_menu(ctx);
            type_str(&view, ctx, "ab");
            assert_eq!(cursor_and_height(&view, ctx).0, Some((2, 0)));

            dispatch(
                &view,
                ctx,
                &[TuiInputAction::EditorCommand(TuiEditorCommand::MoveLeft)],
            );

            assert!(!menu_model.as_ref(ctx).is_open);
            assert_eq!(cursor_and_height(&view, ctx).0, Some((1, 0)));
            assert!(render_input_buffer(&view, ctx).to_lines()[0].starts_with("ab"));
        });
    });
}

/// The `!` shell prefix is not part of `plain_text`, so an empty shell command
/// must not trip the empty-buffer Left branch: Left in shell mode stays a plain
/// cursor move and never opens the conversation picker.
#[test]
fn move_left_in_shell_mode_does_not_open_conversation_menu() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let (view, menu_model, _) = build_view_with_conversation_menu(ctx);
            type_str(&view, ctx, "!");
            assert!(view.as_ref(ctx).is_shell_mode(ctx));
            assert!(
                view.as_ref(ctx).is_empty(ctx),
                "the `!` prefix is not buffered"
            );

            dispatch(
                &view,
                ctx,
                &[TuiInputAction::EditorCommand(TuiEditorCommand::MoveLeft)],
            );

            assert!(
                !menu_model.as_ref(ctx).is_open,
                "Left in shell mode must not open the conversation picker"
            );
            assert!(
                view.as_ref(ctx).is_shell_mode(ctx),
                "input stays in shell mode"
            );
        });
    });
}

#[test]
fn cursor_at_origin_when_empty() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let view = build_view(ctx);
            let (cursor, height) = cursor_and_height(&view, ctx);
            assert_eq!(cursor, Some((0, 0)));
            assert_eq!(height, 1);
        });
    });
}

/// Regression: navigating a freshly-built (empty, never-edited) view must not
/// panic. The char-cell `line_starts` is seeded with `[0]` at construction, so
/// the soft-wrap helpers reached via `move_to_line_start` etc. index it safely
/// before the first edit ever runs `CharCellState::update_text`.
#[test]
fn navigation_on_empty_buffer_does_not_panic() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let view = build_view(ctx);
            dispatch(
                &view,
                ctx,
                &[
                    TuiInputAction::EditorCommand(TuiEditorCommand::MoveToLineStart),
                    TuiInputAction::EditorCommand(TuiEditorCommand::MoveToLineEnd),
                    TuiInputAction::EditorCommand(TuiEditorCommand::MoveLeft),
                    TuiInputAction::EditorCommand(TuiEditorCommand::MoveRight),
                    TuiInputAction::EditorCommand(TuiEditorCommand::MoveUp),
                    TuiInputAction::EditorCommand(TuiEditorCommand::MoveDown),
                ],
            );
            let (cursor, height) = cursor_and_height(&view, ctx);
            assert_eq!(cursor, Some((0, 0)));
            assert_eq!(height, 1);
        });
    });
}

#[test]
fn cursor_tracks_end_of_single_line() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let view = build_view(ctx);
            type_str(&view, ctx, "ab");
            let (cursor, height) = cursor_and_height(&view, ctx);
            assert_eq!(cursor, Some((2, 0)));
            assert_eq!(height, 1);
        });
    });
}

/// Bug 1: after a hard newline the cursor must render at the start of the new
/// (empty) row. Previously the empty trailing row laid out as 0 height, so the
/// column was only 1 row tall and the cursor (row 1) was clipped away.
#[test]
fn cursor_renders_at_start_of_new_line() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let view = build_view(ctx);
            type_str(&view, ctx, "ab");
            dispatch(
                &view,
                ctx,
                &[TuiInputAction::EditorCommand(
                    TuiEditorCommand::InsertNewline,
                )],
            );
            let (cursor, height) = cursor_and_height(&view, ctx);
            assert_eq!(cursor, Some((0, 1)), "cursor should be at row 1, col 0");
            assert!(height >= 2, "two visual rows expected, got height {height}");
        });
    });
}

/// Bug 2: an empty interior line must occupy its own row so following lines —
/// and the cursor — land on the correct visual row.
#[test]
fn interior_empty_line_does_not_collapse() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let view = build_view(ctx);
            // "a\n\nb"
            type_str(&view, ctx, "a");
            dispatch(
                &view,
                ctx,
                &[
                    TuiInputAction::EditorCommand(TuiEditorCommand::InsertNewline),
                    TuiInputAction::EditorCommand(TuiEditorCommand::InsertNewline),
                ],
            );
            type_str(&view, ctx, "b");
            let (cursor, height) = cursor_and_height(&view, ctx);
            assert_eq!(height, 3, "three visual rows expected");
            assert_eq!(cursor, Some((1, 2)), "cursor should be on the 3rd row");
        });
    });
}

/// Bug 2 (navigation): moving up from the last line lands the cursor on the
/// correct (rendered) row, not a collapsed one.
#[test]
fn move_up_through_empty_line_positions_cursor() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let view = build_view(ctx);
            type_str(&view, ctx, "a");
            dispatch(
                &view,
                ctx,
                &[
                    TuiInputAction::EditorCommand(TuiEditorCommand::InsertNewline),
                    TuiInputAction::EditorCommand(TuiEditorCommand::InsertNewline),
                ],
            );
            type_str(&view, ctx, "b");
            // Cursor on row 2 ("b"); move up to the empty row 1.
            dispatch(
                &view,
                ctx,
                &[TuiInputAction::EditorCommand(TuiEditorCommand::MoveUp)],
            );
            let (cursor, height) = cursor_and_height(&view, ctx);
            assert_eq!(height, 3);
            assert_eq!(
                cursor,
                Some((0, 1)),
                "cursor should be on the empty 2nd row"
            );
        });
    });
}

/// Kill bug: `Ctrl+K` from mid-line must delete from the cursor to the end of
/// the visual line (and nothing before it).
#[test]
fn kill_to_line_end_from_midline() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let view = build_view(ctx);
            type_str(&view, ctx, "abcd");
            // Move cursor to just after 'b'.
            dispatch(
                &view,
                ctx,
                &[
                    TuiInputAction::EditorCommand(TuiEditorCommand::MoveLeft),
                    TuiInputAction::EditorCommand(TuiEditorCommand::MoveLeft),
                ],
            );
            dispatch(
                &view,
                ctx,
                &[TuiInputAction::EditorCommand(
                    TuiEditorCommand::KillToLineEnd,
                )],
            );
            assert_eq!(text(&view, ctx), "ab");
        });
    });
}

/// Kill bug: `Ctrl+K` at the end of a line is a no-op (nothing after cursor).
#[test]
fn kill_to_line_end_at_eol_is_noop() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let view = build_view(ctx);
            type_str(&view, ctx, "abcd");
            dispatch(
                &view,
                ctx,
                &[TuiInputAction::EditorCommand(
                    TuiEditorCommand::KillToLineEnd,
                )],
            );
            assert_eq!(text(&view, ctx), "abcd");
        });
    });
}

/// Kill bug: `Ctrl+U` from mid-line must delete from the start of the visual
/// line up to the cursor (and nothing after it).
#[test]
fn kill_to_line_start_from_midline() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let view = build_view(ctx);
            type_str(&view, ctx, "abcd");
            dispatch(
                &view,
                ctx,
                &[
                    TuiInputAction::EditorCommand(TuiEditorCommand::MoveLeft),
                    TuiInputAction::EditorCommand(TuiEditorCommand::MoveLeft),
                ],
            );
            dispatch(
                &view,
                ctx,
                &[TuiInputAction::EditorCommand(
                    TuiEditorCommand::KillToLineStart,
                )],
            );
            assert_eq!(text(&view, ctx), "cd");
        });
    });
}

/// Kill + yank round-trips the killed text at the cursor.
#[test]
fn kill_then_yank_round_trips() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let view = build_view(ctx);
            type_str(&view, ctx, "abcd");
            dispatch(
                &view,
                ctx,
                &[
                    TuiInputAction::EditorCommand(TuiEditorCommand::MoveLeft),
                    TuiInputAction::EditorCommand(TuiEditorCommand::MoveLeft),
                ],
            );
            dispatch(
                &view,
                ctx,
                &[TuiInputAction::EditorCommand(
                    TuiEditorCommand::KillToLineEnd,
                )],
            ); // kills "cd" -> "ab"
            dispatch(
                &view,
                ctx,
                &[TuiInputAction::EditorCommand(TuiEditorCommand::Yank)],
            ); // yanks "cd" -> "abcd"
            assert_eq!(text(&view, ctx), "abcd");
        });
    });
}

/// Ctrl-c clear: emptying the buffer resets the text and the viewport scroll.
#[test]
fn clear_empties_buffer_and_resets_scroll() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let view = build_view(ctx);
            type_lines(&view, ctx, 10); // 10 rows > 6-row viewport
            assert_eq!(scroll_offset(&view, ctx), 4);
            assert!(!view.as_ref(ctx).is_empty(ctx));

            view.update(ctx, |v, ctx| v.clear(ctx));

            assert!(view.as_ref(ctx).is_empty(ctx));
            assert_eq!(text(&view, ctx), "");
            assert_eq!(scroll_offset(&view, ctx), 0);
            assert_eq!(cursor_and_height(&view, ctx).0, Some((0, 0)));
        });
    });
}

/// Bug 3: word-wise selection (Ctrl+Shift+←) extends the selection one word back.
#[test]
fn select_word_left_selects_previous_word() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let view = build_view(ctx);
            type_str(&view, ctx, "hello world");
            dispatch(
                &view,
                ctx,
                &[TuiInputAction::EditorCommand(
                    TuiEditorCommand::SelectWordLeft,
                )],
            );
            assert_eq!(selected_text(&view, ctx).as_deref(), Some("world"));
        });
    });
}

/// Bug 3: word-wise selection (Ctrl+Shift+→) extends the selection one word forward.
#[test]
fn select_word_right_selects_next_word() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let view = build_view(ctx);
            type_str(&view, ctx, "hello world");
            dispatch(
                &view,
                ctx,
                &[TuiInputAction::EditorCommand(
                    TuiEditorCommand::MoveToLineStart,
                )],
            );
            dispatch(
                &view,
                ctx,
                &[TuiInputAction::EditorCommand(
                    TuiEditorCommand::SelectWordRight,
                )],
            );
            assert_eq!(selected_text(&view, ctx).as_deref(), Some("hello"));
        });
    });
}

/// Line-boundary navigation (Home/End) lands on the right column of a multi-line buffer.
#[test]
fn move_to_line_start_and_end_multiline() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let view = build_view(ctx);
            type_str(&view, ctx, "abc");
            dispatch(
                &view,
                ctx,
                &[TuiInputAction::EditorCommand(
                    TuiEditorCommand::InsertNewline,
                )],
            );
            type_str(&view, ctx, "def");
            // Cursor is at end of "def" (row 1, col 3).
            dispatch(
                &view,
                ctx,
                &[TuiInputAction::EditorCommand(
                    TuiEditorCommand::MoveToLineStart,
                )],
            );
            assert_eq!(cursor_and_height(&view, ctx).0, Some((0, 1)));
            dispatch(
                &view,
                ctx,
                &[TuiInputAction::EditorCommand(
                    TuiEditorCommand::MoveToLineEnd,
                )],
            );
            assert_eq!(cursor_and_height(&view, ctx).0, Some((3, 1)));
        });
    });
}

/// Wide (double-width) CJK characters advance the cursor by two display columns
/// each, so the rendered cursor column reflects display width, not char count.
#[test]
fn cursor_accounts_for_wide_chars() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let view = build_view(ctx);
            type_str(&view, ctx, "你好");
            let (cursor, height) = cursor_and_height(&view, ctx);
            assert_eq!(
                cursor,
                Some((4, 0)),
                "two double-width chars → cursor col 4"
            );
            assert_eq!(height, 1);
            assert_eq!(text(&view, ctx), "你好");
        });
    });
}

/// A combining mark is zero-width: it shares its base character's cell, so
/// "a\u{0301}b" occupies two display columns and the cursor ends at column 2.
#[test]
fn cursor_accounts_for_zero_width_chars() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let view = build_view(ctx);
            type_str(&view, ctx, "a\u{0301}b");
            let (cursor, _height) = cursor_and_height(&view, ctx);
            assert_eq!(cursor, Some((2, 0)), "a + combining + b → 2 display cols");
        });
    });
}
#[test]
fn cursor_accounts_for_multi_char_graphemes() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let view = build_view(ctx);

            type_str(&view, ctx, "\u{2328}\u{fe0f}");
            assert_eq!(
                cursor_and_height(&view, ctx).0,
                Some((2, 0)),
                "VS16 emoji occupies two columns"
            );

            type_str(&view, ctx, "👨‍👩‍👧‍👦");
            assert_eq!(
                cursor_and_height(&view, ctx).0,
                Some((4, 0)),
                "ZWJ family adds two columns"
            );

            type_str(&view, ctx, "🇺🇸");
            assert_eq!(
                cursor_and_height(&view, ctx).0,
                Some((6, 0)),
                "regional-indicator flag adds two columns"
            );
        });
    });
}

#[test]
fn multi_char_grapheme_wraps_as_one_unit() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let view = build_view(ctx);
            type_str(&view, ctx, &"x".repeat(usize::from(W) - 1));
            type_str(&view, ctx, "\u{2328}\u{fe0f}");

            assert_eq!(cursor_and_height(&view, ctx), (Some((2, 1)), 2));
        });
    });
}

// ─────────────────────────────────────────────────────────────────────────────
// Soft-wrap growth
// ─────────────────────────────────────────────────────────────────────────────

/// Typing until the first line exactly fills the terminal width wraps the
/// cursor to the next visual row (deferred-wrap terminal behavior), so the
/// input must grow to show that row — and must not scroll the first row away.
#[test]
fn input_grows_when_line_exactly_fills_width() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let view = build_view(ctx);
            type_str(&view, ctx, &"a".repeat(W as usize));
            assert_eq!(scroll_offset(&view, ctx), 0, "first row must stay visible");
            let (cursor, height) = cursor_and_height(&view, ctx);
            assert_eq!(height, 2, "wrapped cursor row must be shown");
            assert_eq!(cursor, Some((0, 1)), "cursor wraps to start of next row");
        });
    });
}

/// Once the first line soft-wraps past the terminal width, the input must be
/// two rows tall with both rows visible.
#[test]
fn input_grows_when_first_line_softwraps() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let view = build_view(ctx);
            type_str(&view, ctx, &"a".repeat(W as usize + 5));
            assert_eq!(scroll_offset(&view, ctx), 0, "first row must stay visible");
            let (cursor, height) = cursor_and_height(&view, ctx);
            assert_eq!(height, 2, "two visual rows expected");
            assert_eq!(cursor, Some((5, 1)), "cursor on second row after wrap");
        });
    });
}

// ─────────────────────────────────────────────────────────────────────────────
// Mouse selection
// ─────────────────────────────────────────────────────────────────────────────

fn left_down(x: u16, y: u16, click_count: u32, shift: bool) -> TuiEvent {
    TuiEvent::LeftMouseDown {
        position: TuiPoint::new(x, y),
        modifiers: ModifiersState {
            shift,
            ..Default::default()
        },
        click_count,
        is_first_mouse: false,
    }
}

fn left_drag(x: u16, y: u16) -> TuiEvent {
    TuiEvent::LeftMouseDragged {
        position: TuiPoint::new(x, y),
        modifiers: ModifiersState::default(),
    }
}

fn left_up(x: u16, y: u16) -> TuiEvent {
    TuiEvent::LeftMouseUp {
        position: TuiPoint::new(x, y),
        modifiers: ModifiersState::default(),
    }
}

/// A mouse-wheel event at `(x, y)`. `delta_rows` follows crossterm's convention
/// (+1 = wheel up / toward the top, -1 = wheel down).
fn scroll_wheel(x: u16, y: u16, delta_rows: isize) -> TuiEvent {
    TuiEvent::ScrollWheel {
        position: TuiPoint::new(x, y),
        delta: (0, delta_rows),
        precise: false,
        modifiers: ModifiersState::default(),
    }
}

/// Types `n` short logical lines ("0".."n-1") into the input.
fn type_lines(view: &ViewHandle<TuiInputView>, ctx: &mut AppContext, n: usize) {
    for i in 0..n {
        if i > 0 {
            dispatch(
                view,
                ctx,
                &[TuiInputAction::EditorCommand(
                    TuiEditorCommand::InsertNewline,
                )],
            );
        }
        type_str(view, ctx, &i.to_string());
    }
}

/// Builds + lays out the view's concrete element at width `W` (height capped
/// by the view), returning the element and the area it occupies. Built via
/// `render_element` (the same element `render_input` boxes) so tests can
/// drive the element's `mouse_action` mapping.
fn laid_out_element(
    view: &ViewHandle<TuiInputView>,
    ctx: &AppContext,
) -> (TuiEditorElement, TuiRect) {
    let mut element = view.as_ref(ctx).render_element(ctx);
    let mut rendered_views = EntityIdMap::default();
    let mut lctx = TuiLayoutContext {
        rendered_views: &mut rendered_views,
    };
    let size = element.layout(TuiConstraint::loose(TuiSize::new(W, 20)), &mut lctx, ctx);
    (element, TuiRect::new(0, 0, size.width, size.height))
}

/// Paints `element` and returns its retained scene.
fn paint_event_scene(element: &mut dyn TuiElement, area: TuiRect) -> Rc<TuiScene> {
    let mut rendered_views = EntityIdMap::default();
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
    Rc::new(paint_ctx.scene.clone())
}

/// Drives the full mouse path for `event`: lay out the element, map the event to
/// its editor action, and apply the corresponding [`TuiInputAction`] to the view.
/// Returns whether an action fired (i.e. the event was not ignored).
fn mouse(view: &ViewHandle<TuiInputView>, ctx: &mut AppContext, event: &TuiEvent) -> bool {
    let action = {
        let (mut element, area) = laid_out_element(view, ctx);
        let scene = paint_event_scene(&mut element, area);
        let mut rendered_views = EntityIdMap::default();
        let event_ctx = TuiEventContext::new(scene, &mut rendered_views);
        element.mouse_action(event, &event_ctx, ctx)
    };
    match action {
        Some(action) => {
            dispatch(view, ctx, &[TuiInputAction::Editor(action)]);
            true
        }
        None => false,
    }
}

#[test]
fn single_click_places_cursor() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let view = build_view(ctx);
            type_str(&view, ctx, "hello world");
            assert!(mouse(&view, ctx, &left_down(3, 0, 1, false)));
            assert!(mouse(&view, ctx, &left_up(3, 0)));
            assert_eq!(cursor_and_height(&view, ctx).0, Some((3, 0)));
            assert_eq!(selected_text(&view, ctx), None);
        });
    });
}

#[test]
fn clicks_map_around_wide_grapheme() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let view = build_view(ctx);
            type_str(&view, ctx, "a\u{2328}\u{fe0f}b");

            mouse(&view, ctx, &left_down(2, 0, 1, false));
            mouse(&view, ctx, &left_up(2, 0));
            assert_eq!(
                cursor_and_height(&view, ctx).0,
                Some((1, 0)),
                "clicking inside the wide grapheme places the cursor before it"
            );

            mouse(&view, ctx, &left_down(3, 0, 1, false));
            mouse(&view, ctx, &left_up(3, 0));
            assert_eq!(
                cursor_and_height(&view, ctx).0,
                Some((3, 0)),
                "clicking after the wide grapheme places the cursor after it"
            );
        });
    });
}
/// Clicking the phantom deferred-wrap row (rendered when a logical line
/// exactly fills the width) must resolve to the end-of-buffer gap — where the
/// cursor visibly sits — not clamp into the preceding full row and teleport
/// the cursor to its start.
#[test]
fn click_on_phantom_wrap_row_keeps_cursor_at_end() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let view = build_view(ctx);
            type_str(&view, ctx, &"a".repeat(W as usize));
            // The exactly-full line renders two rows: the text row and the
            // phantom cursor row below it.
            assert_eq!(cursor_and_height(&view, ctx), (Some((0, 1)), 2));

            // Click the phantom row at its left edge.
            assert!(mouse(&view, ctx, &left_down(0, 1, 1, false)));
            assert!(mouse(&view, ctx, &left_up(0, 1)));

            // The cursor stays at the buffer end (and the row stays rendered).
            assert_eq!(cursor_and_height(&view, ctx), (Some((0, 1)), 2));
            assert_eq!(selected_text(&view, ctx), None);
        });
    });
}

#[test]
fn click_outside_area_is_ignored() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let view = build_view(ctx);
            type_str(&view, ctx, "hi");
            // The single-line input is one row tall; row 5 is outside it.
            assert!(!mouse(&view, ctx, &left_down(0, 5, 1, false)));
            assert_eq!(selected_text(&view, ctx), None);
        });
    });
}

#[test]
fn drag_selects_range() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let view = build_view(ctx);
            type_str(&view, ctx, "hello world");
            mouse(&view, ctx, &left_down(0, 0, 1, false));
            mouse(&view, ctx, &left_drag(5, 0));
            assert_eq!(selected_text(&view, ctx).as_deref(), Some("hello"));
            mouse(&view, ctx, &left_up(5, 0));
            assert_eq!(selected_text(&view, ctx).as_deref(), Some("hello"));
        });
    });
}

#[test]
fn shift_click_extends_selection() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let view = build_view(ctx);
            type_str(&view, ctx, "hello world");
            // Place the cursor at the start, then shift-click after "hello".
            mouse(&view, ctx, &left_down(0, 0, 1, false));
            mouse(&view, ctx, &left_up(0, 0));
            mouse(&view, ctx, &left_down(5, 0, 1, true));
            assert_eq!(selected_text(&view, ctx).as_deref(), Some("hello"));
        });
    });
}

#[test]
fn double_click_selects_word() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let view = build_view(ctx);
            type_str(&view, ctx, "hello world");
            assert!(mouse(&view, ctx, &left_down(2, 0, 2, false)));
            assert_eq!(selected_text(&view, ctx).as_deref(), Some("hello"));
        });
    });
}

#[test]
fn triple_click_selects_line() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let view = build_view(ctx);
            type_str(&view, ctx, "hello world");
            assert!(mouse(&view, ctx, &left_down(2, 0, 3, false)));
            assert_eq!(selected_text(&view, ctx).as_deref(), Some("hello world"));
        });
    });
}

#[test]
fn drag_past_last_visible_row_autoscrolls() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let view = build_view(ctx);
            // 10 logical lines, exceeding the 6-row viewport.
            for i in 0..10 {
                if i > 0 {
                    dispatch(
                        &view,
                        ctx,
                        &[TuiInputAction::EditorCommand(
                            TuiEditorCommand::InsertNewline,
                        )],
                    );
                }
                type_str(&view, ctx, &i.to_string());
            }
            // Scroll back to the top.
            for _ in 0..9 {
                dispatch(
                    &view,
                    ctx,
                    &[TuiInputAction::EditorCommand(TuiEditorCommand::MoveUp)],
                );
            }
            assert_eq!(scroll_offset(&view, ctx), 0);

            // Begin a selection at the top, then drag well below the viewport.
            mouse(&view, ctx, &left_down(0, 0, 1, false));
            mouse(&view, ctx, &left_drag(0, 50));

            // The head followed the drag to the last row, scrolling the viewport.
            assert!(
                scroll_offset(&view, ctx) > 0,
                "drag past the last visible row should auto-scroll"
            );
            assert!(selected_text(&view, ctx).is_some());
        });
    });
}

#[test]
fn wheel_scrolls_viewport_without_moving_cursor() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let view = build_view(ctx);
            type_lines(&view, ctx, 10); // 10 rows > 6-row viewport
                                        // Typing leaves the cursor at the end, scrolled to the bottom.
            assert_eq!(scroll_offset(&view, ctx), 4);
            let cursor_before = view.as_ref(ctx).cursor_offset(ctx);

            // Wheel up (delta +1) scrolls toward the top by WHEEL_STEP (2) rows.
            assert!(mouse(&view, ctx, &scroll_wheel(0, 0, 1)));
            assert_eq!(scroll_offset(&view, ctx), 2);
            // Further wheel-ups clamp at the top.
            mouse(&view, ctx, &scroll_wheel(0, 0, 1));
            assert_eq!(scroll_offset(&view, ctx), 0);
            mouse(&view, ctx, &scroll_wheel(0, 0, 1));
            assert_eq!(scroll_offset(&view, ctx), 0);

            // Scrolling never moved the cursor.
            assert_eq!(view.as_ref(ctx).cursor_offset(ctx), cursor_before);
        });
    });
}

#[test]
fn wheel_scroll_down_clamps_at_bottom() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let view = build_view(ctx);
            type_lines(&view, ctx, 10);
            // Scroll to the top first.
            mouse(&view, ctx, &scroll_wheel(0, 0, 1));
            mouse(&view, ctx, &scroll_wheel(0, 0, 1));
            assert_eq!(scroll_offset(&view, ctx), 0);

            // Wheel down (delta -1) scrolls toward the bottom, clamped at
            // max_scroll = 10 rows - 6 visible = 4.
            mouse(&view, ctx, &scroll_wheel(0, 0, -1));
            assert_eq!(scroll_offset(&view, ctx), 2);
            mouse(&view, ctx, &scroll_wheel(0, 0, -1));
            assert_eq!(scroll_offset(&view, ctx), 4);
            mouse(&view, ctx, &scroll_wheel(0, 0, -1));
            assert_eq!(scroll_offset(&view, ctx), 4);
        });
    });
}

#[test]
fn wheel_outside_area_is_ignored() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let view = build_view(ctx);
            type_lines(&view, ctx, 10);
            let before = scroll_offset(&view, ctx);
            // Row 50 is well outside the 6-row viewport.
            assert!(!mouse(&view, ctx, &scroll_wheel(0, 50, 1)));
            assert_eq!(scroll_offset(&view, ctx), before);
        });
    });
}

// ─────────────────────────────────────────────────────────────────────────────
// Shell mode
// ─────────────────────────────────────────────────────────────────────────────
//
// Mode *transitions* live on the shared `BlocklistAIInputModel` (exercised by
// the app crate's `input_model` tests; the view tests drive it through
// [`BlocklistAIInputModel::mock`]); these tests cover the view's `!` trigger,
// the submit/clear split, and the shell-mode gutter geometry of the composed
// `!`-affordance row (built directly via `TuiInputView::shell_element`).

/// A `!` typed at the start of the buffer enters shell mode without inserting;
/// subsequent text lands in the buffer.
#[test]
fn bang_at_start_enters_shell_mode() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let view = build_view(ctx);
            type_str(&view, ctx, "!ls");
            assert!(view.as_ref(ctx).is_shell_mode(ctx));
            assert_eq!(text(&view, ctx), "ls", "the `!` must not be inserted");
        });
    });
}

#[test]
fn explicit_shell_mode_survives_deleting_the_buffer() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let view = build_view(ctx);
            type_str(&view, ctx, "!cargo");
            for _ in 0.."cargo".chars().count() {
                dispatch(
                    &view,
                    ctx,
                    &[TuiInputAction::EditorCommand(TuiEditorCommand::Backspace)],
                );
            }

            assert_eq!(text(&view, ctx), "");
            assert!(view.as_ref(ctx).is_shell_mode(ctx));
        });
    });
}

#[test]
fn autodetected_unlocked_shell_uses_shell_mode_ui() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let view = build_view(ctx);
            let input_mode = view.as_ref(ctx).input_mode.clone();
            input_mode.update(ctx, |input_mode, ctx| {
                input_mode.set_input_config(
                    InputConfig {
                        input_type: InputType::Shell,
                        is_locked: false,
                    },
                    false,
                    None,
                    ctx,
                );
            });

            assert!(view.as_ref(ctx).is_shell_mode(ctx));
            let mut element = view.as_ref(ctx).render(ctx);
            let mut rendered_views = EntityIdMap::default();
            let mut layout_ctx = TuiLayoutContext {
                rendered_views: &mut rendered_views,
            };
            let size = element.layout(
                TuiConstraint::loose(TuiSize::new(W, 20)),
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
            assert!(buffer.to_lines()[0].starts_with("! "));
        });
    });
}

/// A `!` typed anywhere but the buffer start inserts literally.
#[test]
fn bang_mid_text_inserts_literally() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let view = build_view(ctx);
            type_str(&view, ctx, "a!b");
            assert!(!view.as_ref(ctx).is_shell_mode(ctx));
            assert_eq!(text(&view, ctx), "a!b");
        });
    });
}

// ─────────────────────────────────────────────────────────────────────────────
// Up-to-recall (↑ to edit)
// ─────────────────────────────────────────────────────────────────────────────

/// Pressing ↑ in an empty buffer after a submission recalls the last submitted
/// text so the user can re-edit and re-submit it.
#[test]
fn move_up_in_empty_buffer_recalls_last_submitted_text() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let view = build_view(ctx);
            // Type and submit a message.
            type_str(&view, ctx, "hello world");
            dispatch(&view, ctx, &[TuiInputAction::Submit]);
            // Simulate the session view clearing the input after submission.
            view.update(ctx, |v, vctx| v.clear(vctx));
            assert!(view.as_ref(ctx).is_empty(ctx));

            // ↑ in an empty buffer should restore the last submitted text.
            dispatch(
                &view,
                ctx,
                &[TuiInputAction::EditorCommand(TuiEditorCommand::MoveUp)],
            );
            assert_eq!(text(&view, ctx), "hello world");
        });
    });
}

/// Pressing ↑ with text already in the buffer moves the cursor up (normal
/// editor behavior) rather than recalling the last submitted text.
#[test]
fn move_up_in_nonempty_buffer_moves_cursor_not_recalls() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let view = build_view(ctx);
            // Submit a message so there is something to potentially recall.
            type_str(&view, ctx, "previous message");
            dispatch(&view, ctx, &[TuiInputAction::Submit]);
            view.update(ctx, |v, vctx| v.clear(vctx));

            // Type a new multi-line message.
            type_str(&view, ctx, "line one");
            dispatch(
                &view,
                ctx,
                &[TuiInputAction::EditorCommand(TuiEditorCommand::InsertNewline)],
            );
            type_str(&view, ctx, "line two");

            // Cursor is on line 2; ↑ should move it to line 1, not recall.
            dispatch(
                &view,
                ctx,
                &[TuiInputAction::EditorCommand(TuiEditorCommand::MoveUp)],
            );
            assert_eq!(text(&view, ctx), "line one\nline two");
            // Cursor should now be on line 0.
            assert_eq!(cursor_and_height(&view, ctx).0, Some((8, 0)));
        });
    });
}

/// Pressing ↑ in an empty buffer with no prior submission is a no-op (does
/// not panic and leaves the buffer empty).
#[test]
fn move_up_in_empty_buffer_with_no_prior_submission_is_noop() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let view = build_view(ctx);
            assert!(view.as_ref(ctx).is_empty(ctx));
            dispatch(
                &view,
                ctx,
                &[TuiInputAction::EditorCommand(TuiEditorCommand::MoveUp)],
            );
            assert!(view.as_ref(ctx).is_empty(ctx));
            assert_eq!(cursor_and_height(&view, ctx).0, Some((0, 0)));
        });
    });
}

/// Pressing ↑ in an empty buffer after a whitespace-only submission does not
/// recall the blank entry; only non-whitespace-only submissions are stored.
#[test]
fn whitespace_only_submission_is_not_recalled() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let view = build_view(ctx);
            // Submit a real message first.
            type_str(&view, ctx, "hello world");
            dispatch(&view, ctx, &[TuiInputAction::Submit]);
            view.update(ctx, |v, vctx| v.clear(vctx));
            // Submit a whitespace-only message.
            type_str(&view, ctx, "   ");
            dispatch(&view, ctx, &[TuiInputAction::Submit]);
            view.update(ctx, |v, vctx| v.clear(vctx));

            // ↑ should recall "hello world", not the blank submission.
            dispatch(
                &view,
                ctx,
                &[TuiInputAction::EditorCommand(TuiEditorCommand::MoveUp)],
            );
            assert_eq!(text(&view, ctx), "hello world");
        });
    });
}

/// Submit emits without clearing; the owner clears via [`TuiInputView::clear`]
/// once a submission is accepted.
#[test]
fn submit_keeps_buffer_until_cleared() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let view = build_view(ctx);
            type_str(&view, ctx, "ab");
            dispatch(&view, ctx, &[TuiInputAction::Submit]);
            assert_eq!(text(&view, ctx), "ab", "submit must not clear the buffer");
            view.update(ctx, |v, vctx| v.clear(vctx));
            assert_eq!(text(&view, ctx), "");
            assert_eq!(cursor_and_height(&view, ctx).0, Some((0, 0)));
        });
    });
}

/// Esc is never consumed by the element; the contextual Escape keymap binding
/// dispatches `HandleEscape`, which is a no-op outside an inline menu or shell mode.
#[test]
fn escape_is_not_consumed_by_the_element() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let view = build_view(ctx);
            type_str(&view, ctx, "ab");
            let (mut element, area) = laid_out_element(&view, ctx);
            let scene = paint_event_scene(&mut element, area);
            let mut rendered_views = EntityIdMap::default();
            let mut event_ctx = TuiEventContext::new(scene, &mut rendered_views);
            event_ctx.set_origin_view(Some(view.id()));
            let escape = TuiEvent::KeyDown {
                keystroke: Keystroke {
                    key: "escape".to_owned(),
                    ..Default::default()
                },
                chars: String::new(),
                details: KeyEventDetails::default(),
                is_composing: false,
            };
            assert!(
                !element.dispatch_event(&escape, &mut event_ctx, ctx),
                "escape must not be consumed by the element"
            );

            dispatch(&view, ctx, &[TuiInputAction::HandleEscape]);
            assert_eq!(text(&view, ctx), "ab", "no-op outside shell mode");
        });
    });
}

/// The keymap context enables the unified Escape binding exactly while shell
/// mode or an inline menu needs it.
#[test]
fn keymap_context_flags_shell_mode() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let view = build_view(ctx);
            assert_eq!(
                view.as_ref(ctx).keymap_context(ctx),
                input_keymap_context(false, false, false)
            );

            type_str(&view, ctx, "!");
            assert_eq!(
                view.as_ref(ctx).keymap_context(ctx),
                input_keymap_context(true, false, false)
            );
        });
    });
}

/// Lays out the shell-mode composition (the `!` gutter row wrapping the
/// editor) at width `W`, returning the boxed row element and its area.
fn laid_out_shell_row(
    view: &ViewHandle<TuiInputView>,
    ctx: &AppContext,
) -> (Box<dyn TuiElement>, TuiRect) {
    let mut element = view.as_ref(ctx).shell_element(ctx);
    let mut rendered_views = EntityIdMap::default();
    let mut lctx = TuiLayoutContext {
        rendered_views: &mut rendered_views,
    };
    let size = element.layout(TuiConstraint::loose(TuiSize::new(W, 20)), &mut lctx, ctx);
    (element, TuiRect::new(0, 0, size.width, size.height))
}

/// Lays out the editor content element in the slot the shell row's flex hands
/// it: two columns narrower, offset right of the gutter.
fn laid_out_shell_content_slot(
    view: &ViewHandle<TuiInputView>,
    ctx: &AppContext,
) -> (TuiEditorElement, TuiRect) {
    let mut element = view.as_ref(ctx).render_element(ctx);
    let mut rendered_views = EntityIdMap::default();
    let mut lctx = TuiLayoutContext {
        rendered_views: &mut rendered_views,
    };
    let size = element.layout(
        TuiConstraint::loose(TuiSize::new(W - 2, 20)),
        &mut lctx,
        ctx,
    );
    (element, TuiRect::new(2, 0, size.width, size.height))
}

/// In shell mode the rendered cursor is shifted right by the `!` gutter.
#[test]
fn shell_mode_offsets_cursor_by_gutter() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let view = build_view(ctx);
            type_str(&view, ctx, "ab");
            let (mut element, area) = laid_out_shell_row(&view, ctx);
            let mut rendered_views = EntityIdMap::default();
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
            let cursor = paint_ctx.terminal_cursor().and_then(|point| {
                Some((u16::try_from(point.x).ok()?, u16::try_from(point.y).ok()?))
            });
            assert_eq!(cursor, Some((4, 0)));
        });
    });
}

/// In shell mode mouse columns are measured from the editable area (the
/// editor's slot starts after the gutter), and a click on the gutter itself
/// is consumed, placing the cursor at the start of the buffer.
#[test]
fn shell_mode_offsets_mouse_mapping_by_gutter() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let view = build_view(ctx);
            type_str(&view, ctx, "hello world");
            let action = {
                let (mut element, area) = laid_out_shell_content_slot(&view, ctx);
                let scene = paint_event_scene(&mut element, area);
                let mut rendered_views = EntityIdMap::default();
                let event_ctx = TuiEventContext::new(scene, &mut rendered_views);
                element
                    .mouse_action(&left_down(2 + 3, 0, 1, false), &event_ctx, ctx)
                    .map(TuiInputAction::Editor)
            };
            let Some(TuiInputAction::Editor(TuiEditorAction::SelectionStartAt { offset })) = action
            else {
                panic!("expected SelectionStartAt, got {action:?}");
            };
            // Screen column 5 = content column 3 = gap offset 4 (1-based).
            assert_eq!(offset.as_usize(), 4);

            // A press on the gutter arms the `!` affordance's click, and the
            // release inside it fires the handler (which moves the cursor to
            // the buffer start); both halves are consumed.
            let (mut row, area) = laid_out_shell_row(&view, ctx);
            let scene = paint_event_scene(row.as_mut(), area);
            let mut rendered_views = EntityIdMap::default();
            let mut event_ctx = TuiEventContext::new(scene, &mut rendered_views);
            event_ctx.set_origin_view(Some(view.id()));
            assert!(
                row.dispatch_event(&left_down(0, 0, 1, false), &mut event_ctx, ctx),
                "gutter presses must be consumed"
            );
            assert!(
                row.dispatch_event(&left_up(0, 0), &mut event_ctx, ctx),
                "the release completing a gutter click must be consumed"
            );
        });
    });
}

/// The gutter click places the cursor without starting a drag selection
/// (`SetCursor`), so a later drag cannot extend a stale selection anchored at
/// the buffer start.
#[test]
fn gutter_click_places_cursor_without_selecting() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let view = build_view(ctx);
            type_str(&view, ctx, "hello");
            // The gutter's click handler dispatches `SetCursor` at the start.
            dispatch(
                &view,
                ctx,
                &[TuiInputAction::SetCursor {
                    offset: CharOffset::from(1),
                }],
            );
            assert_eq!(cursor_and_height(&view, ctx).0, Some((0, 0)));
            assert!(!is_drag_selecting(&view, ctx));
            assert_eq!(selected_text(&view, ctx), None);

            // With no press on the editor itself, a drag maps to no action and
            // selects nothing.
            assert!(!mouse(&view, ctx, &left_drag(3, 0)));
            assert_eq!(selected_text(&view, ctx), None);
        });
    });
}

/// The gutter narrows the editable width, so wrapping happens two columns
/// earlier in shell mode.
#[test]
fn shell_mode_wraps_at_gutter_narrowed_width() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let view = build_view(ctx);
            // W - 1 chars: fits one row at width W, wraps at width W - 2.
            type_str(&view, ctx, &"x".repeat(usize::from(W) - 1));
            let (_, area) = laid_out_element(&view, ctx);
            assert_eq!(area.height, 1);
            let (_, area) = laid_out_shell_row(&view, ctx);
            assert_eq!(area.height, 2, "shell mode should wrap two columns earlier");
        });
    });
}

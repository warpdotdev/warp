use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use parking_lot::FairMutex;
use string_offset::CharOffset;
use warp::tui_export::{
    AIAgentAction, AIAgentActionId, AIAgentActionType, AIConversationId, Appearance, TaskId,
    TerminalModel, queue_tui_permission_action,
};
use warpui::AddWindowOptions;
use warpui::platform::WindowStyle;
use warpui_core::elements::tui::{
    Color, TuiBufferExt, TuiConstraint, TuiLayoutContext, TuiRect, TuiSize,
};
use warpui_core::keymap::Keystroke;
use warpui_core::presenter::tui::TuiPresenter;
use warpui_core::{
    App, AppContext, EntityIdMap, TuiView, TypedActionView, ViewHandle, WindowInvalidation,
};

use super::{
    ShellCommandViewState, TuiShellCommandView, TuiShellCommandViewAction, TuiShellCommandViewEvent,
};
use crate::editor_element::TuiEditorAction;
use crate::editor_view::TuiEditorViewAction;
use crate::test_fixtures::{TestHostView, add_test_action_model};
use crate::tui_builder::TuiUiBuilder;
use crate::tui_permission_prompt::TuiPermissionPromptAction;

#[test]
fn command_without_terminal_block_uses_fallback_row() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let action = command_action("action-1", "echo hi");
        let view = add_shell_view(
            &mut app,
            action,
            Arc::new(FairMutex::new(TerminalModel::mock(None, None))),
        );

        app.read(|app| {
            assert_eq!(
                render_non_empty_lines(&view, 80, app),
                vec!["○ Run `echo hi`"]
            );
        });
    });
}

#[test]
fn blocked_command_card_matches_permission_layout() {
    App::test((), |mut app| async move {
        let action = command_action("action-1", "echo 1");
        let view = add_shell_view(
            &mut app,
            action.clone(),
            Arc::new(FairMutex::new(TerminalModel::mock(None, None))),
        );
        let (action_model, conversation_id) = app.read(|ctx| {
            let view = view.as_ref(ctx);
            (view.action_model.clone(), view.conversation_id)
        });
        action_model.update(&mut app, |model, ctx| {
            queue_tui_permission_action(model, action, conversation_id, ctx);
        });

        let mut presenter = TuiPresenter::new();
        let frame = app.update(|ctx| {
            let view_ref = view.as_ref(ctx);
            let prompt = &view_ref.permission_prompt;
            let mut invalidation = WindowInvalidation::default();
            invalidation.updated.insert(view.id());
            invalidation.updated.insert(view_ref.command_editor.id());
            invalidation.updated.insert(prompt.id());
            invalidation
                .updated
                .extend(prompt.as_ref(ctx).child_view_ids(ctx));
            presenter.invalidate(&invalidation, ctx, view.window_id(ctx));
            presenter.present(ctx, &view, TuiRect::new(0, 0, 80, 16))
        });
        let lines = frame.buffer.to_lines();
        let row_containing = |text: &str| {
            u16::try_from(
                lines
                    .iter()
                    .position(|line| line.contains(text))
                    .unwrap_or_else(|| panic!("missing {text:?} in {lines:?}")),
            )
            .expect("row fits in the TUI")
        };
        let header_row = row_containing("Is it OK if I run this command");
        let command_row = row_containing("echo 1");
        let first_option_row = row_containing("(1) yes");
        row_containing("(3) Other");
        let footer_row = row_containing("Esc to cancel");
        assert!(
            lines[usize::from(header_row)]
                .trim_end()
                .ends_with("e to edit command")
        );
        assert!(first_option_row >= command_row + 2);

        let (header_background, surface_background) = app.read(|ctx| {
            let builder = TuiUiBuilder::from_app(ctx);
            (
                builder.permission_header_background(),
                builder.permission_surface_background(),
            )
        });
        assert_ne!(header_background, surface_background);
        assert_eq!(frame.buffer[(79, header_row)].bg, header_background);
        assert_eq!(frame.buffer[(79, command_row)].bg, surface_background);
        assert_eq!(frame.buffer[(79, footer_row)].bg, Color::Reset);
    });
}

#[test]
fn finishing_command_editing_selects_yes_without_executing() {
    App::test((), |mut app| async move {
        app.update(super::init);
        let action = command_action("action-1", "echo original");
        let view = add_shell_view(
            &mut app,
            action.clone(),
            Arc::new(FairMutex::new(TerminalModel::mock(None, None))),
        );
        let (action_model, conversation_id, prompt) = app.read(|ctx| {
            let view = view.as_ref(ctx);
            (
                view.action_model.clone(),
                view.conversation_id,
                view.permission_prompt.clone(),
            )
        });
        action_model.update(&mut app, |model, ctx| {
            queue_tui_permission_action(model, action, conversation_id, ctx);
        });

        prompt.update(&mut app, |prompt, ctx| {
            prompt.handle_action(&TuiPermissionPromptAction::EditBody, ctx);
        });
        app.read(|ctx| {
            let view = view.as_ref(ctx);
            assert!(view.command_editor.as_ref(ctx).is_focused());
        });
        let command_editor = app.read(|ctx| view.as_ref(ctx).command_editor.clone());
        command_editor.update(&mut app, |editor, ctx| {
            editor.set_text("echo edited\necho second", ctx)
        });
        present_shell_view(&mut app, &view);
        assert!(dispatch_focused_key(&mut app, &view, "enter"));

        app.read(|ctx| {
            let view = view.as_ref(ctx);
            assert!(!view.command_editor.as_ref(ctx).is_focused());
            assert_eq!(
                view.permission_prompt.as_ref(ctx).highlighted_index(ctx),
                Some(0)
            );
            assert_eq!(
                view.command_editor.as_ref(ctx).text(ctx),
                "echo edited\necho second"
            );
            assert!(
                view.action_model
                    .as_ref(ctx)
                    .get_action_result(&view.action.id)
                    .is_none()
            );
        });
    });
}

#[test]
fn command_editor_arrows_move_within_multiline_text_then_cycle_at_boundaries() {
    App::test((), |mut app| async move {
        app.update(super::init);
        app.update(crate::option_selector::init);
        let action = command_action("action-1", "first\nsecond\nthird");
        let view = add_shell_view(
            &mut app,
            action.clone(),
            Arc::new(FairMutex::new(TerminalModel::mock(None, None))),
        );
        let (action_model, conversation_id, prompt, command_editor) = app.read(|ctx| {
            let view = view.as_ref(ctx);
            (
                view.action_model.clone(),
                view.conversation_id,
                view.permission_prompt.clone(),
                view.command_editor.clone(),
            )
        });
        action_model.update(&mut app, |model, ctx| {
            queue_tui_permission_action(model, action, conversation_id, ctx);
        });
        prompt.update(&mut app, |prompt, ctx| {
            prompt.handle_action(&TuiPermissionPromptAction::EditBody, ctx);
        });
        command_editor.update(&mut app, |editor, ctx| {
            editor.handle_action(
                &TuiEditorViewAction::Editor(TuiEditorAction::SelectionStartAt {
                    offset: CharOffset::from(1),
                }),
                ctx,
            );
        });

        present_shell_view(&mut app, &view);
        app.read(|ctx| {
            let window_id = view.window_id(ctx);
            let focused = ctx
                .focused_view_id(window_id)
                .expect("command editor is focused");
            let responder_chain = ctx.view_ancestors(window_id, focused);
            let selector_id = prompt.as_ref(ctx).child_view_ids(ctx)[0];
            assert!(responder_chain.contains(&selector_id));
        });

        assert!(dispatch_focused_key(&mut app, &view, "down"));
        assert!(app.read(|ctx| command_editor.as_ref(ctx).is_focused()));
        assert!(dispatch_focused_key(&mut app, &view, "down"));
        assert!(app.read(|ctx| command_editor.as_ref(ctx).is_focused()));
        assert!(dispatch_focused_key(&mut app, &view, "down"));
        app.read(|ctx| {
            assert!(!command_editor.as_ref(ctx).is_focused());
            assert_eq!(prompt.as_ref(ctx).highlighted_index(ctx), Some(0));
        });

        prompt.update(&mut app, |prompt, ctx| {
            prompt.handle_action(&TuiPermissionPromptAction::EditBody, ctx);
        });
        command_editor.update(&mut app, |editor, ctx| {
            editor.handle_action(
                &TuiEditorViewAction::Editor(TuiEditorAction::SelectionStartAt {
                    offset: CharOffset::from(1),
                }),
                ctx,
            );
        });
        assert!(dispatch_focused_key(&mut app, &view, "up"));
        app.read(|ctx| {
            assert!(!command_editor.as_ref(ctx).is_focused());
            assert_eq!(prompt.as_ref(ctx).highlighted_index(ctx), Some(2));
        });
    });
}

#[test]
fn streamed_action_refresh_invalidates_layout() {
    App::test((), |mut app| async move {
        let action = command_action("action-1", "echo original");
        let view = add_shell_view(
            &mut app,
            action.clone(),
            Arc::new(FairMutex::new(TerminalModel::mock(None, None))),
        );
        let layout_invalidations = Rc::new(Cell::new(0));
        let invalidations_for_subscription = layout_invalidations.clone();
        app.update(|ctx| {
            ctx.subscribe_to_view(&view, move |_, event, _| match event {
                TuiShellCommandViewEvent::LayoutChanged => {
                    invalidations_for_subscription.set(invalidations_for_subscription.get() + 1);
                }
                TuiShellCommandViewEvent::BlockingStateChanged
                | TuiShellCommandViewEvent::ReplacementGuidanceSubmitted(_) => {}
            });
        });

        view.update(&mut app, |view, ctx| {
            view.command_was_edited = true;
            view.update_action(action, true, ctx);
        });

        assert_eq!(layout_invalidations.get(), 1);
    });
}

#[test]
fn terminal_block_is_collapsed_by_default_and_expands_inline() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let action = command_action("action-1", "printf result");
        let terminal_model =
            terminal_model_with_command(&action, "printf result", "command output");
        let view = add_shell_view(&mut app, action, terminal_model);
        let layout_invalidations = Rc::new(Cell::new(0));
        let invalidations_for_subscription = layout_invalidations.clone();
        app.update(|ctx| {
            ctx.subscribe_to_view(&view, move |_, event, _| match event {
                TuiShellCommandViewEvent::LayoutChanged => {
                    invalidations_for_subscription.set(invalidations_for_subscription.get() + 1);
                }
                TuiShellCommandViewEvent::BlockingStateChanged
                | TuiShellCommandViewEvent::ReplacementGuidanceSubmitted(_) => {}
            });
        });
        let collapsed_height = app.read(|app| {
            let collapsed_lines = render_non_empty_lines(&view, 80, app);
            assert_eq!(collapsed_lines, vec!["✓ Ran `printf result`  ▸"]);
            rendered_height(&view, 80, app)
        });
        view.update(&mut app, |view, ctx| {
            view.handle_action(&TuiShellCommandViewAction::ToggleExpanded, ctx);
        });
        app.read(|app| {
            let expanded_lines = render_non_empty_lines(&view, 80, app);
            assert_eq!(expanded_lines[0], "✓ Ran `printf result`  ▾");
            let terminal_content = expanded_lines[1..].join("");
            assert!(
                terminal_content.contains("command output"),
                "{expanded_lines:?}"
            );
            assert!(rendered_height(&view, 80, app) > collapsed_height);
        });

        view.update(&mut app, |view, ctx| {
            view.handle_action(&TuiShellCommandViewAction::ToggleExpanded, ctx);
        });
        app.read(|app| {
            assert_eq!(rendered_height(&view, 80, app), collapsed_height);
            assert_eq!(layout_invalidations.get(), 2);
        });
    });
}

#[test]
fn shell_command_views_keep_independent_collapse_state() {
    let mut first = ShellCommandViewState::new_collapsed();
    let second = ShellCommandViewState::new_collapsed();

    first.toggle();

    assert!(!first.is_collapsed());
    assert!(second.is_collapsed());
}

#[test]
fn manual_collapse_override_wins_over_auto_expansion() {
    let mut state = ShellCommandViewState::new_collapsed();

    state.toggle();

    assert!(!state.is_collapsed());
    assert!(state.manual_override);
    assert!(!state.auto_expanded);
}

fn present_shell_view(app: &mut App, view: &ViewHandle<TuiShellCommandView>) {
    let mut presenter = TuiPresenter::new();
    app.update(|ctx| {
        let view_ref = view.as_ref(ctx);
        let prompt = &view_ref.permission_prompt;
        let mut invalidation = WindowInvalidation::default();
        invalidation.updated.insert(view.id());
        invalidation.updated.insert(view_ref.command_editor.id());
        invalidation.updated.insert(prompt.id());
        invalidation
            .updated
            .extend(prompt.as_ref(ctx).child_view_ids(ctx));
        presenter.invalidate(&invalidation, ctx, view.window_id(ctx));
        presenter.present(ctx, view, TuiRect::new(0, 0, 80, 16));
    });
}

fn dispatch_focused_key(app: &mut App, view: &ViewHandle<TuiShellCommandView>, key: &str) -> bool {
    let (window_id, responder_chain) = app.read(|ctx| {
        let window_id = view.window_id(ctx);
        let focused = ctx
            .focused_view_id(window_id)
            .expect("shell permission interaction has a focused view");
        (window_id, ctx.view_ancestors(window_id, focused))
    });
    app.dispatch_keystroke(
        window_id,
        &responder_chain,
        &Keystroke::parse(key).expect("valid keystroke"),
        false,
    )
    .expect("keystroke dispatch succeeds")
}

fn add_shell_view(
    app: &mut App,
    action: AIAgentAction,
    terminal_model: Arc<FairMutex<TerminalModel>>,
) -> ViewHandle<TuiShellCommandView> {
    let action_model = add_test_action_model(app);
    app.update(|ctx| {
        let (window_id, _) = ctx.add_tui_window(
            AddWindowOptions {
                window_style: WindowStyle::NotStealFocus,
                ..Default::default()
            },
            |_| TestHostView,
        );
        ctx.add_typed_action_tui_view(window_id, move |ctx| {
            TuiShellCommandView::new(
                action,
                false,
                action_model,
                AIConversationId::new(),
                terminal_model,
                ctx,
            )
        })
    })
}

fn terminal_model_with_command(
    action: &AIAgentAction,
    command: &str,
    output: &str,
) -> Arc<FairMutex<TerminalModel>> {
    let mut model = TerminalModel::mock(None, None);
    model.simulate_block(command, &format!("{output}\r\n"));
    let block = model
        .block_list_mut()
        .blocks_mut()
        .iter_mut()
        .find(|block| block.command_to_string().contains(command))
        .expect("simulated command block");
    block.set_agent_interaction_mode_for_requested_command(
        action.id.clone(),
        None,
        AIConversationId::new(),
    );
    Arc::new(FairMutex::new(model))
}

fn command_action(id: &str, command: &str) -> AIAgentAction {
    AIAgentAction {
        id: AIAgentActionId::from(id.to_owned()),
        task_id: TaskId::new("task-1".to_owned()),
        action: AIAgentActionType::RequestCommandOutput {
            command: command.to_owned(),
            is_read_only: Some(true),
            is_risky: Some(false),
            wait_until_completion: true,
            uses_pager: Some(false),
            rationale: None,
            citations: Vec::new(),
        },
        requires_result: true,
    }
}

fn rendered_height(view: &ViewHandle<TuiShellCommandView>, width: u16, app: &AppContext) -> u16 {
    let mut rendered_views = EntityIdMap::default();
    let mut layout_ctx = TuiLayoutContext {
        rendered_views: &mut rendered_views,
    };
    let mut element = view.as_ref(app).render(app);
    element
        .layout(
            TuiConstraint::loose(TuiSize::new(width, u16::MAX)),
            &mut layout_ctx,
            app,
        )
        .height
}

fn render_non_empty_lines(
    view: &ViewHandle<TuiShellCommandView>,
    width: u16,
    app: &AppContext,
) -> Vec<String> {
    let height = rendered_height(view, width, app).max(1);
    let mut presenter = TuiPresenter::new();
    presenter
        .present_element(
            view.as_ref(app).render(app),
            TuiRect::new(0, 0, width, height),
            app,
        )
        .buffer
        .to_lines()
        .into_iter()
        .map(|line| line.trim_end().to_owned())
        .filter(|line| !line.is_empty())
        .collect()
}

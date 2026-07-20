use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use parking_lot::FairMutex;
use warp::tui_export::{
    AIAgentAction, AIAgentActionId, AIAgentActionType, AIConversationId, Appearance, TaskId,
    TerminalModel,
};
use warpui::AddWindowOptions;
use warpui::platform::WindowStyle;
use warpui_core::elements::tui::{TuiBufferExt, TuiConstraint, TuiLayoutContext, TuiRect, TuiSize};
use warpui_core::presenter::tui::TuiPresenter;
use warpui_core::{App, AppContext, EntityIdMap, TuiView, TypedActionView, ViewHandle};

use super::{
    ShellCommandViewState, TuiShellCommandView, TuiShellCommandViewAction, TuiShellCommandViewEvent,
};
use crate::test_fixtures::{TestHostView, add_test_action_model};

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
        ctx.add_typed_action_tui_view(window_id, move |_| {
            TuiShellCommandView::new(action, false, action_model, terminal_model)
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

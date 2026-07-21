use std::cell::RefCell;
use std::rc::Rc;

use warp::tui_export::{
    AIAgentAction, AIAgentActionId, AIAgentActionType, AIConversationId, Appearance, TaskId,
    queue_tui_permission_action,
};
use warpui::platform::WindowStyle;
use warpui::{AddWindowOptions, App, WindowInvalidation};
use warpui_core::elements::tui::{TuiBufferExt, TuiElement, TuiRect, TuiText};
use warpui_core::presenter::tui::TuiPresenter;
use warpui_core::{TuiView as _, TypedActionView as _, ViewHandle};

use super::{
    PERMISSION_PROMPT_ACTIVE, TuiPermissionPrompt, TuiPermissionPromptAction,
    TuiPermissionPromptEvent, render_permission_card,
};
use crate::editor_view::TuiEditorView;
use crate::option_selector::{TuiOptionSelectorAction, TuiOptionSelectorEvent};
use crate::test_fixtures::{TestHostView, add_test_action_model};
fn add_prompt(app: &mut App, body_editable: bool) -> ViewHandle<TuiPermissionPrompt> {
    app.add_singleton_model(|_| Appearance::mock());
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
            let body_editor =
                body_editable.then(|| ctx.add_typed_action_tui_view(TuiEditorView::single_line));
            TuiPermissionPrompt::new(
                action_model,
                AIAgentActionId::from("permission-action".to_owned()),
                body_editor,
                ctx,
            )
        })
    })
}

fn render_lines(app: &mut App, prompt: &ViewHandle<TuiPermissionPrompt>) -> Vec<String> {
    let mut presenter = TuiPresenter::new();
    app.update(|ctx| {
        let mut invalidation = WindowInvalidation::default();
        invalidation.updated.insert(prompt.id());
        invalidation
            .updated
            .insert(prompt.as_ref(ctx).selector.id());
        presenter.invalidate(&invalidation, ctx, prompt.window_id(ctx));
        presenter
            .present_element(
                render_permission_card(
                    prompt,
                    "Permission",
                    Some(TuiText::new("details").finish()),
                    ctx,
                ),
                TuiRect::new(0, 0, 80, 12),
                ctx,
            )
            .buffer
            .to_lines()
            .into_iter()
            .map(|line| line.trim().to_owned())
            .filter(|line| !line.is_empty())
            .collect()
    })
}

#[test]
fn permission_prompt_defaults_to_yes_and_renders_other() {
    App::test((), |mut app| async move {
        let prompt = add_prompt(&mut app, false);

        assert_eq!(
            render_lines(&mut app, &prompt),
            [
                "■ Permission",
                "details",
                "(1) yes",
                "(2) no",
                "(3) Other",
                "Esc to cancel  Enter to run",
            ]
        );
        app.read(|ctx| {
            assert_eq!(
                prompt.as_ref(ctx).selector.as_ref(ctx).highlighted_index(),
                Some(0)
            );
        });
    });
}

#[test]
fn leading_editor_participates_in_selector_focus_cycle() {
    App::test((), |mut app| async move {
        let prompt = add_prompt(&mut app, true);
        let (action_model, action) = app.read(|ctx| {
            let prompt = prompt.as_ref(ctx);
            (prompt.action_model.clone(), pending_action(prompt))
        });
        action_model.update(&mut app, |model, ctx| {
            queue_tui_permission_action(model, action, AIConversationId::new(), ctx);
        });

        prompt.update(&mut app, |prompt, ctx| {
            prompt.handle_action(&TuiPermissionPromptAction::MoveUp, ctx);
        });

        app.read(|ctx| {
            let prompt = prompt.as_ref(ctx);
            assert!(
                prompt
                    .body_editor
                    .as_ref()
                    .expect("editable prompt has a body editor")
                    .as_ref(ctx)
                    .is_focused()
            );
            assert_eq!(prompt.selector.as_ref(ctx).highlighted_index(), None);
            assert!(
                !prompt
                    .keymap_context(ctx)
                    .set
                    .contains(PERMISSION_PROMPT_ACTIVE)
            );
        });

        prompt.update(&mut app, |prompt, ctx| {
            prompt.handle_action(&TuiPermissionPromptAction::MoveUp, ctx);
        });
        app.read(|ctx| {
            assert_eq!(
                prompt.as_ref(ctx).selector.as_ref(ctx).highlighted_index(),
                Some(2)
            );
        });
    });
}

#[test]
fn editable_prompt_uses_edit_option_for_shortcut_and_click() {
    App::test((), |mut app| async move {
        let prompt = add_prompt(&mut app, true);
        let lines = render_lines(&mut app, &prompt);
        assert!(lines.iter().any(|line| line == "(e) edit command"));
        assert!(lines.iter().all(|line| !line.contains("Other")));

        let (action_model, action) = app.read(|ctx| {
            let prompt = prompt.as_ref(ctx);
            (prompt.action_model.clone(), pending_action(prompt))
        });
        action_model.update(&mut app, |model, ctx| {
            queue_tui_permission_action(model, action, AIConversationId::new(), ctx);
        });

        let selector = prompt.read(&app, |prompt, _| prompt.selector.clone());
        selector.update(&mut app, |selector, ctx| {
            selector.handle_action(&TuiOptionSelectorAction::SelectShortcut('e'), ctx);
        });
        assert!(app.read(|ctx| {
            prompt
                .as_ref(ctx)
                .body_editor
                .as_ref()
                .expect("editable prompt has a body editor")
                .as_ref(ctx)
                .is_focused()
        }));

        prompt.update(&mut app, |prompt, ctx| prompt.restore_options_focus(ctx));
        selector.update(&mut app, |selector, ctx| {
            selector.handle_action(&TuiOptionSelectorAction::SelectItem(2), ctx);
        });
        assert!(app.read(|ctx| {
            prompt
                .as_ref(ctx)
                .body_editor
                .as_ref()
                .expect("editable prompt has a body editor")
                .as_ref(ctx)
                .is_focused()
        }));
    });
}

fn pending_action(prompt: &TuiPermissionPrompt) -> AIAgentAction {
    AIAgentAction {
        id: prompt.action_id.clone(),
        task_id: TaskId::new("task".to_owned()),
        action: AIAgentActionType::InitProject,
        requires_result: true,
    }
}

#[test]
fn no_requests_rejection_without_resolving_in_the_selector() {
    App::test((), |mut app| async move {
        let prompt = add_prompt(&mut app, false);
        let (action_model, conversation_id, action) = app.read(|ctx| {
            let prompt = prompt.as_ref(ctx);
            (
                prompt.action_model.clone(),
                AIConversationId::new(),
                pending_action(prompt),
            )
        });
        action_model.update(&mut app, |model, ctx| {
            queue_tui_permission_action(model, action, conversation_id, ctx);
        });
        let rejected = Rc::new(RefCell::new(false));
        let rejected_for_event = rejected.clone();
        app.update(|ctx| {
            ctx.subscribe_to_view(&prompt, move |_, event, _| {
                if matches!(event, TuiPermissionPromptEvent::RejectRequested) {
                    *rejected_for_event.borrow_mut() = true;
                }
            });
        });

        prompt.update(&mut app, |prompt, ctx| {
            prompt.handle_selector_event(
                &TuiOptionSelectorEvent::Confirmed {
                    id: "no".to_owned(),
                },
                ctx,
            );
        });

        assert!(*rejected.borrow());
        app.read(|ctx| {
            assert!(
                action_model
                    .as_ref(ctx)
                    .get_pending_action_by_id(&prompt.as_ref(ctx).action_id)
                    .is_some()
            );
        });
    });
}

#[test]
fn other_emits_guidance_without_requesting_rejection() {
    App::test((), |mut app| async move {
        let prompt = add_prompt(&mut app, false);
        let (action_model, conversation_id, action) = app.read(|ctx| {
            let prompt = prompt.as_ref(ctx);
            (
                prompt.action_model.clone(),
                AIConversationId::new(),
                pending_action(prompt),
            )
        });
        action_model.update(&mut app, |model, ctx| {
            queue_tui_permission_action(model, action, conversation_id, ctx);
        });
        let submitted = Rc::new(RefCell::new(Vec::new()));
        let submitted_for_event = submitted.clone();
        let rejected = Rc::new(RefCell::new(false));
        let rejected_for_event = rejected.clone();
        app.update(|ctx| {
            ctx.subscribe_to_view(&prompt, move |_, event, _| match event {
                TuiPermissionPromptEvent::ReplacementGuidanceSubmitted(text) => {
                    submitted_for_event.borrow_mut().push(text.clone());
                }
                TuiPermissionPromptEvent::RejectRequested => {
                    *rejected_for_event.borrow_mut() = true;
                }
                TuiPermissionPromptEvent::AcceptRequested
                | TuiPermissionPromptEvent::BlockingStateChanged
                | TuiPermissionPromptEvent::LayoutChanged => {}
            });
        });

        prompt.update(&mut app, |prompt, ctx| {
            prompt.handle_selector_event(
                &TuiOptionSelectorEvent::CustomTextSubmitted {
                    value: "use a safer command".to_owned(),
                },
                ctx,
            );
        });

        assert_eq!(
            submitted.borrow().clone(),
            vec!["use a safer command".to_owned()]
        );
        assert!(!*rejected.borrow());
        app.read(|ctx| {
            assert!(
                action_model
                    .as_ref(ctx)
                    .get_pending_action_by_id(&prompt.as_ref(ctx).action_id)
                    .is_some()
            );
        });
    });
}

#[test]
fn editable_prompt_includes_the_edit_hint() {
    App::test((), |mut app| async move {
        let prompt = add_prompt(&mut app, true);

        assert!(
            render_lines(&mut app, &prompt)
                .iter()
                .any(|line| line == "Esc to cancel  Ctrl+E to edit/save  Enter to run")
        );
    });
}

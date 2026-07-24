use warp::tui_export::{
    AIAgentAction, AIAgentActionId, AIAgentActionType, AIConversationId, Appearance,
    AskUserQuestionAction, AskUserQuestionAnswerItem, AskUserQuestionItem, AskUserQuestionOption,
    AskUserQuestionType, TaskId, queue_tui_permission_action,
};
use warpui::platform::WindowStyle;
use warpui::{AddWindowOptions, WindowInvalidation};
use warpui_core::elements::tui::{TuiBufferExt, TuiRect};
use warpui_core::keymap::Keystroke;
use warpui_core::presenter::tui::TuiPresenter;
use warpui_core::{App, TuiView as _, TypedActionView as _, ViewHandle};

use super::{TuiAskQuestionView, TuiAskQuestionViewAction};
use crate::option_selector::TuiOptionSelectorAction;
use crate::test_fixtures::add_test_action_model;

fn question(
    id: &str,
    text: &str,
    is_multiselect: bool,
    supports_other: bool,
    options: &[&str],
) -> AskUserQuestionItem {
    AskUserQuestionItem {
        question_id: id.to_owned(),
        question: text.to_owned(),
        question_type: AskUserQuestionType::MultipleChoice {
            is_multiselect,
            options: options
                .iter()
                .map(|label| AskUserQuestionOption {
                    label: (*label).to_owned(),
                    recommended: false,
                })
                .collect(),
            supports_other,
        },
    }
}

fn add_view(
    app: &mut App,
    questions: Vec<AskUserQuestionItem>,
) -> (warpui::WindowId, ViewHandle<TuiAskQuestionView>) {
    app.add_singleton_model(|_| Appearance::mock());
    let action_model = add_test_action_model(app);
    app.update(|ctx| {
        ctx.add_tui_window(
            AddWindowOptions {
                window_style: WindowStyle::NotStealFocus,
                ..Default::default()
            },
            |ctx| {
                TuiAskQuestionView::new(
                    action_model,
                    AIConversationId::new(),
                    AIAgentActionId::from("ask-question".to_owned()),
                    questions,
                    ctx,
                )
            },
        )
    })
}

fn render_active_lines(app: &mut App, view: &ViewHandle<TuiAskQuestionView>) -> Vec<String> {
    let mut presenter = TuiPresenter::new();
    app.update(|ctx| {
        let mut invalidation = WindowInvalidation::default();
        invalidation.updated.insert(view.as_ref(ctx).selector.id());
        presenter.invalidate(&invalidation, ctx, view.window_id(ctx));
        let element = view.as_ref(ctx).render_active(ctx);
        presenter
            .present_element(element, TuiRect::new(0, 0, 80, 20), ctx)
            .buffer
            .to_lines()
            .into_iter()
            .map(|line| line.trim_end().to_owned())
            .filter(|line| !line.is_empty())
            .collect()
    })
}

fn queue_question_action(app: &mut App, view: &ViewHandle<TuiAskQuestionView>) {
    let (action_model, conversation_id, action) = app.read(|ctx| {
        let view = view.as_ref(ctx);
        (
            view.action_model.clone(),
            view.conversation_id,
            AIAgentAction {
                id: view.action_id.clone(),
                task_id: TaskId::new("task".to_owned()),
                action: AIAgentActionType::AskUserQuestion {
                    questions: view.source_questions.clone(),
                },
                requires_result: true,
            },
        )
    });
    action_model.update(app, |model, ctx| {
        queue_tui_permission_action(model, action, conversation_id, ctx);
    });
}

fn present_active_view(app: &mut App, view: &ViewHandle<TuiAskQuestionView>) {
    let mut presenter = TuiPresenter::new();
    app.update(|ctx| {
        let selector = view.as_ref(ctx).selector.clone();
        let mut invalidation = WindowInvalidation::default();
        invalidation.updated.insert(view.id());
        invalidation.updated.insert(selector.id());
        invalidation
            .updated
            .extend(selector.as_ref(ctx).child_view_ids(ctx));
        presenter.invalidate(&invalidation, ctx, view.window_id(ctx));
        presenter.present(ctx, view, TuiRect::new(0, 0, 80, 20));
    });
}

fn dispatch_focused_key(app: &mut App, view: &ViewHandle<TuiAskQuestionView>, key: &str) -> bool {
    let (window_id, responder_chain) = app.read(|ctx| {
        let window_id = view.window_id(ctx);
        let focused = ctx
            .focused_view_id(window_id)
            .expect("question interaction has a focused view");
        let responder_chain = ctx.view_ancestors(window_id, focused);
        assert!(responder_chain.contains(&view.id()));
        assert!(responder_chain.contains(&view.as_ref(ctx).selector.id()));
        (window_id, responder_chain)
    });
    app.dispatch_keystroke(
        window_id,
        &responder_chain,
        &Keystroke::parse(key).expect("valid keystroke"),
        false,
    )
    .expect("keystroke dispatch succeeds")
}

#[test]
fn active_card_matches_question_panel_structure() {
    App::test((), |mut app| async move {
        let (_, view) = add_view(
            &mut app,
            vec![
                question(
                    "q1",
                    "Which targets should be tested?",
                    true,
                    true,
                    &["Stable", "Nightly"],
                ),
                question("q2", "Which shell?", false, false, &["zsh"]),
            ],
        );

        let lines = render_active_lines(&mut app, &view);
        assert_eq!(
            lines,
            [
                "┌──────────────────────────────────────────────────────────────────────────────┐",
                "│                                                                              │",
                "│ ■ Agent questions                                                 ← 1 of 2 → │",
                "│                                                                              │",
                "│ Which targets should be tested? (select all that apply)                      │",
                "│ (1) [ ] Stable                                                               │",
                "│ (2) [ ] Nightly                                                              │",
                "│ (3) [ ] Other…                                                               │",
                "│                                                                              │",
                "│ Shift + Enter to advance Enter or number to select Ctrl + C to cancel questi │",
                "│                                                                              │",
                "└──────────────────────────────────────────────────────────────────────────────┘",
            ]
        );
    });
}

#[test]
fn enter_selects_options_and_other_before_shift_enter_advances_multiselect() {
    App::test((), |mut app| async move {
        app.update(crate::keybindings::init);
        let (_, view) = add_view(
            &mut app,
            vec![
                question(
                    "multi",
                    "Which targets?",
                    true,
                    true,
                    &["Stable", "Nightly"],
                ),
                question("single", "Which shell?", false, false, &["zsh"]),
            ],
        );
        queue_question_action(&mut app, &view);
        present_active_view(&mut app, &view);

        assert!(dispatch_focused_key(&mut app, &view, "enter"));
        app.read(|ctx| {
            let view = view.as_ref(ctx);
            assert_eq!(view.session.current_question_index(), 0);
            assert!(view.auto_advance.is_none());
            assert!(
                view.session
                    .draft_for_question(0)
                    .is_some_and(|draft| draft.selected_option_indices.contains(&0))
            );
        });
        assert!(
            render_active_lines(&mut app, &view)
                .iter()
                .any(|line| line.contains("(1) [✓] Stable"))
        );

        let selector = app.read(|ctx| view.as_ref(ctx).selector.clone());
        selector.update(&mut app, |selector, ctx| {
            selector.handle_action(&TuiOptionSelectorAction::SelectItem(2), ctx);
            selector.set_active_custom_text_for_test("Canary", ctx);
        });
        assert!(dispatch_focused_key(&mut app, &view, "enter"));
        app.read(|ctx| {
            let view = view.as_ref(ctx);
            assert_eq!(view.session.current_question_index(), 0);
            assert!(view.auto_advance.is_none());
            let draft = view
                .session
                .draft_for_question(0)
                .expect("multi-select draft exists");
            assert!(draft.selected_option_indices.contains(&0));
            assert_eq!(draft.other_text.as_deref(), Some("Canary"));
        });
        assert!(
            render_active_lines(&mut app, &view)
                .iter()
                .any(|line| line.contains("(3) [✓] Canary"))
        );
        assert!(dispatch_focused_key(&mut app, &view, "enter"));
        app.read(|ctx| {
            let view = view.as_ref(ctx);
            let draft = view
                .session
                .draft_for_question(0)
                .expect("regular multi-select option remains selected");
            assert!(draft.selected_option_indices.contains(&0));
            assert!(draft.other_text.is_none());
        });
        assert!(
            render_active_lines(&mut app, &view)
                .iter()
                .any(|line| line.contains("(3) [ ] Other…"))
        );

        assert!(dispatch_focused_key(&mut app, &view, "shift-enter"));
        assert_eq!(
            app.read(|ctx| view.as_ref(ctx).session.current_question_index()),
            1
        );
    });
}

#[test]
fn enter_keeps_single_select_auto_advance_behavior() {
    App::test((), |mut app| async move {
        app.update(super::init);
        let (_, view) = add_view(
            &mut app,
            vec![question(
                "single",
                "Which shell?",
                false,
                false,
                &["zsh", "fish"],
            )],
        );
        queue_question_action(&mut app, &view);
        present_active_view(&mut app, &view);

        assert!(dispatch_focused_key(&mut app, &view, "enter"));
        app.read(|ctx| {
            let view = view.as_ref(ctx);
            assert!(view.auto_advance.is_some());
            assert!(
                view.session
                    .draft_for_question(0)
                    .is_some_and(|draft| draft.selected_option_indices.contains(&0))
            );
        });
    });
}

#[test]
fn enter_does_not_submit_a_final_multiselect_question() {
    App::test((), |mut app| async move {
        app.update(super::init);
        let (_, view) = add_view(
            &mut app,
            vec![question(
                "multi",
                "Which targets?",
                true,
                true,
                &["Stable", "Nightly"],
            )],
        );
        queue_question_action(&mut app, &view);
        present_active_view(&mut app, &view);

        assert!(dispatch_focused_key(&mut app, &view, "enter"));
        app.read(|ctx| {
            let view = view.as_ref(ctx);
            assert!(view.session.is_editing());
            assert_eq!(view.session.current_question_index(), 0);
            assert!(view.auto_advance.is_none());
            assert!(
                view.session
                    .draft_for_question(0)
                    .is_some_and(|draft| draft.selected_option_indices.contains(&0))
            );
        });
    });
}

#[test]
fn completed_answers_match_normalized_questions_by_id() {
    App::test((), |mut app| async move {
        let (_, view) = add_view(
            &mut app,
            vec![
                question("single", "Which shell?", false, false, &["zsh"]),
                question(
                    "multi",
                    "Which targets?",
                    true,
                    false,
                    &["Stable", "Nightly"],
                ),
            ],
        );
        let answers = vec![
            AskUserQuestionAnswerItem::Answered {
                question_id: "single".to_owned(),
                selected_options: vec!["zsh".to_owned()],
                other_text: String::new(),
            },
            AskUserQuestionAnswerItem::Answered {
                question_id: "multi".to_owned(),
                selected_options: vec!["Stable".to_owned(), "Nightly".to_owned()],
                other_text: String::new(),
            },
        ];
        let lines = app.read(|ctx| {
            let mut presenter = TuiPresenter::new();
            presenter
                .present_element(
                    view.as_ref(ctx).render_answers(&answers, ctx),
                    TuiRect::new(0, 0, 80, 10),
                    ctx,
                )
                .buffer
                .to_lines()
                .into_iter()
                .map(|line| line.trim_end().to_owned())
                .filter(|line| !line.is_empty())
                .collect::<Vec<_>>()
        });

        assert_eq!(
            lines,
            [
                "✓ Answered all 2 questions",
                "  Q: Which targets?",
                "  A: Stable, Nightly",
                "  Q: Which shell?",
                "  A: zsh",
            ]
        );
    });
}

#[test]
fn mixed_question_order_matches_the_original_action_payload() {
    App::test((), |mut app| async move {
        let questions = vec![
            question("single", "Which shell?", false, false, &["zsh"]),
            question(
                "multi",
                "Which targets?",
                true,
                false,
                &["Stable", "Nightly"],
            ),
        ];
        let (_, view) = add_view(&mut app, questions.clone());

        app.read(|ctx| {
            let view = view.as_ref(ctx);
            assert_eq!(view.session.questions()[0].question_id, "multi");
            assert!(view.matches_action(
                &AIAgentActionId::from("ask-question".to_owned()),
                &questions,
            ));
        });
    });
}

#[test]
fn navigation_restores_multi_selection_and_other_text() {
    App::test((), |mut app| async move {
        let (_, view) = add_view(
            &mut app,
            vec![
                question("q1", "Targets?", true, true, &["Stable", "Nightly"]),
                question("q2", "Shell?", false, false, &["zsh"]),
            ],
        );

        view.update(&mut app, |view, ctx| {
            let effect = view
                .session
                .apply(AskUserQuestionAction::ToggleOption { option_index: 1 });
            view.handle_effect(effect, ctx);
            let effect = view.session.apply(AskUserQuestionAction::SaveOtherText {
                text: Some("Canary".to_owned()),
            });
            view.handle_effect(effect, ctx);
            let effect = view.session.apply(AskUserQuestionAction::NavigateNext);
            view.handle_effect(effect, ctx);
            let effect = view.session.apply(AskUserQuestionAction::NavigatePrev);
            view.handle_effect(effect, ctx);
        });

        let lines = render_active_lines(&mut app, &view);
        assert!(lines.iter().any(|line| line.contains("[✓] Nightly")));
        assert!(lines.iter().any(|line| line.contains("[✓] Canary")));
    });
}

#[test]
fn opening_and_leaving_other_keeps_selector_and_shared_session_in_sync() {
    App::test((), |mut app| async move {
        let (_, view) = add_view(
            &mut app,
            vec![question("q1", "Target?", false, true, &["Stable"])],
        );
        let selector = app.read(|ctx| view.as_ref(ctx).selector.clone());
        selector.update(&mut app, |selector, ctx| {
            selector.handle_action(&TuiOptionSelectorAction::SelectItem(1), ctx);
        });

        app.read(|ctx| {
            let view = view.as_ref(ctx);
            assert!(
                view.session
                    .current()
                    .and_then(|current| current.draft)
                    .is_some_and(|draft| draft.is_other_input_active)
            );
            assert_eq!(view.selector.as_ref(ctx).highlighted_question_index(), None);
        });

        selector.update(&mut app, |selector, ctx| {
            selector.handle_action(&TuiOptionSelectorAction::MoveUp, ctx);
        });
        app.read(|ctx| {
            let view = view.as_ref(ctx);
            assert!(
                view.session
                    .current()
                    .and_then(|current| current.draft)
                    .is_none_or(|draft| !draft.is_other_input_active)
            );
            assert_eq!(
                view.selector.as_ref(ctx).highlighted_question_index(),
                Some(0)
            );
        });
    });
}

#[test]
fn submitting_other_restores_question_navigation_focus() {
    App::test((), |mut app| async move {
        app.update(super::init);
        let (_, view) = add_view(
            &mut app,
            vec![
                question("q1", "Target?", true, true, &["Alpha"]),
                question("q2", "Shell?", false, true, &["Gamma"]),
            ],
        );
        queue_question_action(&mut app, &view);
        present_active_view(&mut app, &view);

        let selector = app.read(|ctx| view.as_ref(ctx).selector.clone());
        selector.update(&mut app, |selector, ctx| {
            selector.handle_action(&TuiOptionSelectorAction::SelectItem(1), ctx);
            selector.set_active_custom_text_for_test("custom answer", ctx);
        });
        view.update(&mut app, |view, ctx| {
            view.handle_action(&TuiAskQuestionViewAction::Enter, ctx);
            view.abort_auto_advance();
            let effect = view.session.apply(AskUserQuestionAction::Confirm);
            view.handle_effect(effect, ctx);
        });
        present_active_view(&mut app, &view);

        app.read(|ctx| {
            let view = view.as_ref(ctx);
            assert_eq!(view.session.current_question_index(), 1);
            assert!(view.selector.as_ref(ctx).list_is_focused(ctx));
        });
        assert!(dispatch_focused_key(&mut app, &view, "left"));
        assert_eq!(
            app.read(|ctx| view.as_ref(ctx).session.current_question_index()),
            0
        );
    });
}

#[test]
fn navigating_away_from_a_cleared_other_editor_removes_the_previous_answer() {
    App::test((), |mut app| async move {
        let (_, view) = add_view(
            &mut app,
            vec![
                question("q1", "Target?", false, true, &["Stable"]),
                question("q2", "Shell?", false, false, &["zsh"]),
            ],
        );

        view.update(&mut app, |view, ctx| {
            let effect = view.session.apply(AskUserQuestionAction::SaveOtherText {
                text: Some("Canary".to_owned()),
            });
            view.handle_effect(effect, ctx);
            view.show_current_question(ctx);

            let effect = view
                .session
                .apply(AskUserQuestionAction::EnterCustomAnswerEditing);
            view.handle_effect(effect, ctx);
            view.selector.update(ctx, |selector, ctx| {
                selector.set_active_custom_text_for_test("", ctx);
            });

            view.commit_active_other_text(ctx);
            let effect = view.session.apply(AskUserQuestionAction::NavigateNext);
            view.handle_effect(effect, ctx);
        });

        app.read(|ctx| {
            let view = view.as_ref(ctx);
            assert_eq!(view.session.current_question_index(), 1);
            assert!(view.session.draft_for_question(0).is_none());
        });
    });
}

#[test]
fn completed_answers_render_deterministic_question_and_answer_rows() {
    App::test((), |mut app| async move {
        let (_, view) = add_view(
            &mut app,
            vec![
                question("q1", "Target?", false, false, &["Stable"]),
                question("q2", "Shell?", false, false, &["zsh"]),
            ],
        );
        let answers = vec![
            AskUserQuestionAnswerItem::Answered {
                question_id: "q1".to_owned(),
                selected_options: vec!["Stable".to_owned()],
                other_text: String::new(),
            },
            AskUserQuestionAnswerItem::Skipped {
                question_id: "q2".to_owned(),
            },
        ];
        let lines = app.read(|ctx| {
            let mut presenter = TuiPresenter::new();
            presenter
                .present_element(
                    view.as_ref(ctx).render_answers(&answers, ctx),
                    TuiRect::new(0, 0, 80, 10),
                    ctx,
                )
                .buffer
                .to_lines()
                .into_iter()
                .map(|line| line.trim_end().to_owned())
                .filter(|line| !line.is_empty())
                .collect::<Vec<_>>()
        });

        assert_eq!(
            lines,
            [
                "✓ Answered 1 of 2 questions",
                "  Q: Target?",
                "  A: Stable",
                "  Q: Shell?",
                "  A: Skipped",
            ]
        );
    });
}

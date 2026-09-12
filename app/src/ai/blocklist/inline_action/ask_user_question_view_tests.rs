use ai::agent::action::{AskUserQuestionItem, AskUserQuestionOption, AskUserQuestionType};
use ai::agent::{AskUserQuestionAction, AskUserQuestionSession};
use pathfinder_geometry::vector::vec2f;
use warpui::platform::WindowStyle;
use warpui::{App, Presenter, WindowInvalidation};

use super::number_shortcut_buttons::NumberShortcutButtonsAction;
use super::*;

fn build_question(question_id: &str, supports_other: bool) -> AskUserQuestionItem {
    AskUserQuestionItem {
        question_id: question_id.to_string(),
        question: "Question".to_string(),
        question_type: AskUserQuestionType::MultipleChoice {
            is_multiselect: false,
            options: vec![AskUserQuestionOption {
                label: "Stable".to_string(),
                recommended: false,
            }],
            supports_other,
        },
    }
}

#[test]
fn view_state_shows_other_input_only_for_the_current_question() {
    let mut session = AskUserQuestionSession::new(vec![
        build_question("q1", true),
        build_question("q2", false),
    ]);

    assert_eq!(
        ask_user_question_view_state(session.current()),
        AskUserQuestionViewState {
            show_other_input: false,
        }
    );

    session.apply(AskUserQuestionAction::EnterCustomAnswerEditing);
    assert_eq!(
        ask_user_question_view_state(session.current()),
        AskUserQuestionViewState {
            show_other_input: true,
        }
    );

    session.apply(AskUserQuestionAction::NavigateNext);
    assert_eq!(
        ask_user_question_view_state(session.current()),
        AskUserQuestionViewState {
            show_other_input: false,
        }
    );
}

/// Test-only harness mounting the *real* `wrap_scrollable_body` + `NumberShortcutButtons`
/// construction used by `AskUserQuestionView::render_active` for a single-question card, so the
/// option-count-vs-height-cap interaction can be exercised without standing up the full
/// `BlocklistAIActionModel` action-executor state machine `AskUserQuestionView` itself depends on.
struct OverflowTestView {
    buttons: ViewHandle<NumberShortcutButtons>,
    scroll_state: ClippedScrollStateHandle,
}

impl OverflowTestView {
    fn new(option_count: usize, ctx: &mut ViewContext<Self>) -> Self {
        let scroll_state = ClippedScrollStateHandle::new();
        let builders: Vec<NumberShortcutButtonBuilder> = (1..=option_count)
            .map(|i| {
                number_shortcut_buttons::numbered_shortcut_button(
                    i,
                    format!("Option {i}"),
                    false,
                    false,
                    true,
                    MouseStateHandle::default(),
                    AskUserQuestionViewAction::OptionToggled { option_index: i },
                )
            })
            .collect();
        let scroll_state_for_buttons = scroll_state.clone();
        let buttons = ctx.add_typed_action_view(move |ctx| {
            NumberShortcutButtons::new_with_config(
                builders,
                None,
                NumberShortcutButtonsConfig::new()
                    .with_keyboard_navigation()
                    .with_scroll_state(scroll_state_for_buttons),
                ctx,
            )
        });
        Self {
            buttons,
            scroll_state,
        }
    }
}

impl Entity for OverflowTestView {
    type Event = ();
}

impl View for OverflowTestView {
    fn ui_name() -> &'static str {
        "AskUserQuestionOverflowTestView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let question_text =
            AskUserQuestionView::render_question_text("Question", appearance, theme);
        let options = Container::new(ChildView::new(&self.buttons).finish())
            .with_padding_bottom(ASK_USER_QUESTION_OPTIONS_BOTTOM_PADDING)
            .finish();
        let body = AskUserQuestionView::wrap_scrollable_body(
            question_text,
            options,
            self.scroll_state.clone(),
            theme,
        );
        // Matches the outer cap `render_active` applies to a single-question card's body.
        ConstrainedBox::new(body)
            .with_max_height(ASK_USER_QUESTION_SINGLE_MAX_CONTAINER_HEIGHT)
            .finish()
    }
}

impl TypedActionView for OverflowTestView {
    type Action = AskUserQuestionViewAction;
}

/// Regression test for APP-5386: a long option list (matching the ~20-option Factory-picker
/// report) must stay fully keyboard-reachable within a single-question card's height cap. Before
/// the fix, `ASK_USER_QUESTION_SINGLE_MAX_CONTAINER_HEIGHT` (800px) was tall enough that a
/// 20-option list's content fit underneath it without the internal `NewScrollable` ever detecting
/// overflow, so arrowing down selected options that never scrolled into view and no scrollbar
/// appeared. The window here is deliberately taller than the cap, so the cap -- not a short
/// viewport -- is what's being exercised.
#[test]
fn long_option_list_stays_keyboard_reachable_within_the_single_question_cap() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let option_count = 20;
        let (window_id, view) = app.add_window(WindowStyle::NotStealFocus, move |ctx| {
            OverflowTestView::new(option_count, ctx)
        });
        let buttons = view.read(&app, |view, _| view.buttons.clone());
        let scroll_state = view.read(&app, |view, _| view.scroll_state.clone());
        let root_view_id = app
            .root_view_id(window_id)
            .expect("window should have a root view");

        let mut presenter = Presenter::new(window_id);
        let invalidation = WindowInvalidation {
            updated: [root_view_id, buttons.id()].into_iter().collect(),
            ..Default::default()
        };

        app.update(|ctx| {
            presenter.invalidate(invalidation.clone(), ctx);
            presenter.build_scene(vec2f(600., 1000.), 1., None, ctx);

            buttons.update(ctx, |buttons, ctx| {
                // The first ArrowDown selects index 0, so `option_count` presses land on the
                // last option's (0-indexed) `option_count - 1`.
                for _ in 0..option_count {
                    buttons.handle_action(&NumberShortcutButtonsAction::ArrowDown, ctx);
                }
            });

            presenter.invalidate(invalidation, ctx);
            presenter.build_scene(vec2f(600., 1000.), 1., None, ctx);

            assert_eq!(
                buttons.read(ctx, |buttons, _| buttons.selected_button_index()),
                Some(option_count - 1),
                "expected keyboard navigation to reach the last option"
            );
            assert!(
                scroll_state.scroll_start().as_f32() > 0.,
                "a {option_count}-option list capped at {ASK_USER_QUESTION_SINGLE_MAX_CONTAINER_HEIGHT}px \
                 should scroll to bring the last option into view instead of silently fitting \
                 under the cap",
            );
        });
    });
}

use ai::agent::action::{AskUserQuestionItem, AskUserQuestionOption, AskUserQuestionType};
use ai::agent::action_result::AskUserQuestionAnswerItem;

use super::{
    ask_user_question_view_state, AskUserQuestionAction, AskUserQuestionEffect,
    AskUserQuestionPhase, AskUserQuestionSession, AskUserQuestionViewState,
};

fn build_question(
    question_id: &str,
    question: &str,
    is_multiselect: bool,
    supports_other: bool,
    options: &[&str],
) -> AskUserQuestionItem {
    AskUserQuestionItem {
        question_id: question_id.to_string(),
        question: question.to_string(),
        question_type: AskUserQuestionType::MultipleChoice {
            is_multiselect,
            options: options
                .iter()
                .map(|label| AskUserQuestionOption {
                    label: (*label).to_string(),
                    recommended: false,
                })
                .collect(),
            supports_other,
        },
    }
}

fn build_session(questions: Vec<AskUserQuestionItem>) -> AskUserQuestionSession {
    AskUserQuestionSession::new(questions)
}

fn view_state_for(session: &AskUserQuestionSession) -> AskUserQuestionViewState {
    ask_user_question_view_state(session.current())
}

fn current_draft(session: &AskUserQuestionSession) -> Option<&super::QuestionDraft> {
    session.current().and_then(|current| current.draft)
}

#[test]
fn enter_on_other_row_focuses_the_other_input() {
    let mut session = build_session(vec![build_question("q1", "Only", false, true, &["Stable"])]);

    assert_eq!(
        session.apply(AskUserQuestionAction::PressEnter {
            highlighted_index: Some(1),
            active_other_text: None,
        }),
        AskUserQuestionEffect::FocusOtherInput
    );
}

#[test]
fn enter_without_an_answer_submits_the_last_question_immediately() {
    let mut session = build_session(vec![build_question(
        "q1",
        "Only",
        false,
        true,
        &["Stable", "Nightly"],
    )]);

    assert_eq!(
        session.apply(AskUserQuestionAction::PressEnter {
            highlighted_index: None,
            active_other_text: None,
        }),
        AskUserQuestionEffect::Submit(vec![AskUserQuestionAnswerItem::Skipped {
            question_id: "q1".to_string(),
        }])
    );
}

#[test]
fn enter_after_a_selected_answer_schedules_auto_advance() {
    let mut session = build_session(vec![build_question(
        "q1",
        "Only",
        false,
        false,
        &["Stable", "Nightly"],
    )]);

    assert_eq!(
        session.apply(AskUserQuestionAction::ToggleOption { option_index: 1 }),
        AskUserQuestionEffect::ScheduleAutoAdvance
    );

    assert_eq!(
        session.apply(AskUserQuestionAction::PressEnter {
            highlighted_index: None,
            active_other_text: None,
        }),
        AskUserQuestionEffect::ScheduleAutoAdvance
    );
}

#[test]
fn enter_with_blank_other_input_submits_the_last_question_immediately() {
    let mut session = build_session(vec![build_question("q1", "Only", false, true, &["Stable"])]);

    assert_eq!(
        session.apply(AskUserQuestionAction::OpenOtherInput),
        AskUserQuestionEffect::FocusOtherInput
    );

    assert_eq!(
        session.apply(AskUserQuestionAction::PressEnter {
            highlighted_index: None,
            active_other_text: None,
        }),
        AskUserQuestionEffect::Submit(vec![AskUserQuestionAnswerItem::Skipped {
            question_id: "q1".to_string(),
        }])
    );
}

#[test]
fn enter_with_active_other_text_schedules_auto_advance() {
    let mut session = build_session(vec![build_question("q1", "Only", false, true, &["Stable"])]);

    assert_eq!(
        session.apply(AskUserQuestionAction::OpenOtherInput),
        AskUserQuestionEffect::FocusOtherInput
    );
    assert_eq!(
        session.apply(AskUserQuestionAction::PressEnter {
            highlighted_index: None,
            active_other_text: Some("nightly".to_string()),
        }),
        AskUserQuestionEffect::ScheduleAutoAdvance
    );
    assert_eq!(
        current_draft(&session).and_then(|draft| draft.other_text.as_deref()),
        Some("nightly")
    );
}

#[test]
fn enter_on_single_select_option_toggles_it_and_schedules_auto_advance() {
    let mut session = build_session(vec![build_question(
        "q1",
        "Only",
        false,
        true,
        &["Stable", "Nightly"],
    )]);

    assert_eq!(
        session.apply(AskUserQuestionAction::PressEnter {
            highlighted_index: Some(1),
            active_other_text: None,
        }),
        AskUserQuestionEffect::ScheduleAutoAdvance
    );
    assert!(current_draft(&session).is_some_and(|draft| draft.selected_option_indices.contains(&1)));
}

#[test]
fn enter_on_non_last_multi_select_option_toggles_it_and_schedules_auto_advance() {
    let mut session = build_session(vec![
        build_question("q1", "First", true, true, &["Stable", "Nightly"]),
        build_question("q2", "Second", false, false, &["CLI"]),
    ]);

    assert_eq!(
        session.apply(AskUserQuestionAction::PressEnter {
            highlighted_index: Some(1),
            active_other_text: None,
        }),
        AskUserQuestionEffect::ScheduleAutoAdvance
    );
    assert!(current_draft(&session).is_some_and(|draft| draft.selected_option_indices.contains(&1)));
}

#[test]
fn enter_on_answered_non_last_multi_select_option_keeps_existing_selection_and_advances() {
    let mut session = build_session(vec![
        build_question("q1", "First", true, true, &["Stable", "Nightly"]),
        build_question("q2", "Second", false, false, &["CLI"]),
    ]);

    assert_eq!(
        session.apply(AskUserQuestionAction::ToggleOption { option_index: 0 }),
        AskUserQuestionEffect::RefreshCurrent
    );
    assert_eq!(
        session.apply(AskUserQuestionAction::PressEnter {
            highlighted_index: Some(1),
            active_other_text: None,
        }),
        AskUserQuestionEffect::ScheduleAutoAdvance
    );
    assert!(current_draft(&session).is_some_and(|draft| {
        draft.selected_option_indices.contains(&0) && draft.selected_option_indices.contains(&1)
    }));
}

#[test]
fn single_select_non_last_toggle_schedules_auto_advance() {
    let mut session = build_session(vec![
        build_question("q1", "First", false, false, &["Rust", "Go"]),
        build_question("q2", "Second", false, false, &["CLI", "GUI"]),
    ]);

    let effect = session.apply(AskUserQuestionAction::ToggleOption { option_index: 1 });

    assert_eq!(effect, AskUserQuestionEffect::ScheduleAutoAdvance);
    assert_eq!(session.current_question_index(), 0);
    assert!(current_draft(&session).is_some_and(|draft| draft.selected_option_indices.contains(&1)));
    assert!(matches!(session.phase(), AskUserQuestionPhase::Editing));
}

#[test]
fn multi_select_non_last_toggle_does_not_auto_advance() {
    let mut session = build_session(vec![
        build_question("q1", "First", true, false, &["Rust", "Go"]),
        build_question("q2", "Second", false, false, &["CLI", "GUI"]),
    ]);

    let effect = session.apply(AskUserQuestionAction::ToggleOption { option_index: 1 });

    assert_eq!(effect, AskUserQuestionEffect::RefreshCurrent);
    assert_eq!(session.current_question_index(), 0);
    assert!(current_draft(&session).is_some_and(|draft| draft.selected_option_indices.contains(&1)));
    assert!(matches!(session.phase(), AskUserQuestionPhase::Editing));
}

#[test]
fn last_multi_select_toggle_schedules_auto_advance() {
    let mut session = build_session(vec![build_question(
        "q1",
        "Only",
        true,
        false,
        &["Rust", "Go"],
    )]);

    let effect = session.apply(AskUserQuestionAction::ToggleOption { option_index: 0 });

    assert_eq!(effect, AskUserQuestionEffect::ScheduleAutoAdvance);
    assert_eq!(session.current_question_index(), 0);
    assert!(current_draft(&session).is_some_and(|draft| draft.selected_option_indices.contains(&0)));
}

#[test]
fn single_select_clicking_selected_option_clears_the_draft() {
    let mut session = build_session(vec![build_question(
        "q1",
        "Only",
        false,
        false,
        &["Rust", "Go"],
    )]);

    assert_eq!(
        session.apply(AskUserQuestionAction::ToggleOption { option_index: 0 }),
        AskUserQuestionEffect::ScheduleAutoAdvance
    );
    assert_eq!(
        session.apply(AskUserQuestionAction::ToggleOption { option_index: 0 }),
        AskUserQuestionEffect::RefreshCurrent
    );
    assert!(current_draft(&session).is_none());
    assert_eq!(session.current_question_index(), 0);
}

#[test]
fn drafts_survive_navigation_and_submit_skips_only_unanswered_questions() {
    let mut session = build_session(vec![
        build_question("q1", "First", true, false, &["Rust", "Go"]),
        build_question("q2", "Second", true, false, &["CLI", "GUI"]),
        build_question("q3", "Third", false, true, &["Stable"]),
    ]);

    assert_eq!(
        session.apply(AskUserQuestionAction::ToggleOption { option_index: 0 }),
        AskUserQuestionEffect::RefreshCurrent
    );
    assert_eq!(
        session.apply(AskUserQuestionAction::NavigateNext),
        AskUserQuestionEffect::ShowQuestion
    );
    assert_eq!(
        session.apply(AskUserQuestionAction::NavigatePrev),
        AskUserQuestionEffect::ShowQuestion
    );
    assert_eq!(
        current_draft(&session).map(|draft| draft.selected_option_indices.len()),
        Some(1)
    );
    assert_eq!(
        session.apply(AskUserQuestionAction::NavigateNext),
        AskUserQuestionEffect::ShowQuestion
    );
    assert_eq!(
        session.apply(AskUserQuestionAction::NavigateNext),
        AskUserQuestionEffect::ShowQuestion
    );
    assert_eq!(
        session.apply(AskUserQuestionAction::OpenOtherInput),
        AskUserQuestionEffect::FocusOtherInput
    );
    assert_eq!(
        session.apply(AskUserQuestionAction::SaveOtherText {
            text: Some("nightly toolchain".to_string()),
        }),
        AskUserQuestionEffect::ScheduleAutoAdvance
    );

    let effect = session.apply(AskUserQuestionAction::Confirm);

    assert_eq!(
        effect,
        AskUserQuestionEffect::Submit(vec![
            AskUserQuestionAnswerItem::Answered {
                question_id: "q1".to_string(),
                selected_options: vec!["Rust".to_string()],
                other_text: String::new(),
            },
            AskUserQuestionAnswerItem::Skipped {
                question_id: "q2".to_string(),
            },
            AskUserQuestionAnswerItem::Answered {
                question_id: "q3".to_string(),
                selected_options: vec![],
                other_text: "nightly toolchain".to_string(),
            },
        ])
    );
    assert!(matches!(
        session.phase(),
        AskUserQuestionPhase::Completed { .. }
    ));
}

#[test]
fn multi_select_other_text_does_not_auto_advance_before_last_question() {
    let mut session = build_session(vec![
        build_question("q1", "First", true, true, &["Rust"]),
        build_question("q2", "Second", false, false, &["CLI"]),
    ]);

    assert_eq!(
        session.apply(AskUserQuestionAction::OpenOtherInput),
        AskUserQuestionEffect::FocusOtherInput
    );
    assert_eq!(
        session.apply(AskUserQuestionAction::SaveOtherText {
            text: Some("nightly".to_string()),
        }),
        AskUserQuestionEffect::RefreshCurrent
    );
    assert_eq!(session.current_question_index(), 0);
    assert_eq!(
        current_draft(&session).and_then(|draft| draft.other_text.as_deref()),
        Some("nightly")
    );
}

#[test]
fn skip_all_moves_session_to_completed_with_skipped_answers() {
    let mut session = build_session(vec![
        build_question("q1", "First", true, false, &["Rust"]),
        build_question("q2", "Second", false, true, &["Stable"]),
    ]);

    session.apply(AskUserQuestionAction::ToggleOption { option_index: 0 });
    session.apply(AskUserQuestionAction::NavigateNext);
    session.apply(AskUserQuestionAction::OpenOtherInput);
    session.apply(AskUserQuestionAction::SaveOtherText {
        text: Some("nightly".to_string()),
    });

    let effect = session.apply(AskUserQuestionAction::SkipAll);

    assert_eq!(
        effect,
        AskUserQuestionEffect::Submit(vec![
            AskUserQuestionAnswerItem::Skipped {
                question_id: "q1".to_string(),
            },
            AskUserQuestionAnswerItem::Skipped {
                question_id: "q2".to_string(),
            },
        ])
    );
    assert!(matches!(
        session.phase(),
        AskUserQuestionPhase::Completed { .. }
    ));
}

#[test]
fn other_text_submission_exits_input_and_submits_last_question() {
    let mut session = build_session(vec![build_question("q1", "Only", false, true, &["Stable"])]);

    assert_eq!(
        session.apply(AskUserQuestionAction::OpenOtherInput),
        AskUserQuestionEffect::FocusOtherInput
    );
    assert!(view_state_for(&session).show_other_input);

    assert_eq!(
        session.apply(AskUserQuestionAction::SaveOtherText {
            text: Some("nightly".to_string()),
        }),
        AskUserQuestionEffect::ScheduleAutoAdvance
    );

    let draft = current_draft(&session).expect("draft should exist");
    assert_eq!(draft.other_text.as_deref(), Some("nightly"));
    assert!(!draft.is_other_input_active);
    assert!(!view_state_for(&session).show_other_input);

    let effect = session.apply(AskUserQuestionAction::Confirm);

    assert_eq!(
        effect,
        AskUserQuestionEffect::Submit(vec![AskUserQuestionAnswerItem::Answered {
            question_id: "q1".to_string(),
            selected_options: vec![],
            other_text: "nightly".to_string(),
        }])
    );
}

#[test]
fn navigating_next_on_last_question_is_a_noop() {
    let mut session = build_session(vec![build_question(
        "q1",
        "Only",
        false,
        false,
        &["Rust", "Go"],
    )]);

    assert_eq!(
        session.apply(AskUserQuestionAction::ToggleOption { option_index: 0 }),
        AskUserQuestionEffect::ScheduleAutoAdvance
    );
    assert_eq!(
        session.apply(AskUserQuestionAction::NavigateNext),
        AskUserQuestionEffect::Noop
    );
    assert!(matches!(session.phase(), AskUserQuestionPhase::Editing));
    assert!(current_draft(&session).is_some_and(|draft| draft.selected_option_indices.contains(&0)));
}

#[test]
fn view_state_shows_other_input() {
    let mut session = build_session(vec![
        build_question("q1", "First", false, true, &["Stable"]),
        build_question("q2", "Second", false, false, &["CLI"]),
    ]);

    assert_eq!(
        view_state_for(&session),
        AskUserQuestionViewState {
            show_other_input: false,
        }
    );

    assert_eq!(
        session.apply(AskUserQuestionAction::OpenOtherInput),
        AskUserQuestionEffect::FocusOtherInput
    );
    assert_eq!(
        view_state_for(&session),
        AskUserQuestionViewState {
            show_other_input: true,
        }
    );

    assert_eq!(
        session.apply(AskUserQuestionAction::SaveOtherText {
            text: Some("nightly".to_string()),
        }),
        AskUserQuestionEffect::ScheduleAutoAdvance
    );
    assert_eq!(
        session.apply(AskUserQuestionAction::NavigateNext),
        AskUserQuestionEffect::ShowQuestion
    );

    assert_eq!(
        view_state_for(&session),
        AskUserQuestionViewState {
            show_other_input: false,
        }
    );
}

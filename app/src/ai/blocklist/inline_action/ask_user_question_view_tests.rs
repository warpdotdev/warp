use ai::agent::action::{AskUserQuestionItem, AskUserQuestionOption, AskUserQuestionType};
use ai::agent::{AskUserQuestionAction, AskUserQuestionSession};

use super::{AskUserQuestionViewState, ask_user_question_view_state};

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

    session.apply(AskUserQuestionAction::OpenOtherInput);
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

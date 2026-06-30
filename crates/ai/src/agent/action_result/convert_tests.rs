use super::*;

#[test]
fn cancelled_request_command_converts_to_explicit_tool_result() {
    let result = api::request::input::tool_call_result::Result::try_from(
        RequestCommandOutputResult::CancelledBeforeExecution {
            command: "sleep 10".to_string(),
        },
    )
    .expect("cancelled run command should still produce a tool result");

    let api::request::input::tool_call_result::Result::RunShellCommand(result) = result else {
        panic!("expected run_shell_command result");
    };

    assert_eq!(result.command, "sleep 10");
    assert!(
        result.result.is_none(),
        "cancelled run command should resolve the tool call with an empty result"
    );
}

#[test]
fn ask_user_question_skipped_by_auto_approve_converts_to_skipped_answers() {
    let result = api::request::input::tool_call_result::Result::from(
        AskUserQuestionResult::SkippedByAutoApprove {
            question_ids: vec!["q1".to_string(), "q2".to_string()],
        },
    );

    let api::request::input::tool_call_result::Result::AskUserQuestion(result) = result else {
        panic!("expected ask_user_question result");
    };

    let Some(api::ask_user_question_result::Result::Success(success)) = result.result else {
        panic!("expected success result");
    };

    assert_eq!(success.answers.len(), 2);
    assert_eq!(success.answers[0].question_id, "q1");
    assert_eq!(success.answers[1].question_id, "q2");
    assert!(matches!(
        success.answers[0].answer,
        Some(AskUserQuestionAnswer::Skipped(()))
    ));
    assert!(matches!(
        success.answers[1].answer,
        Some(AskUserQuestionAnswer::Skipped(()))
    ));
}

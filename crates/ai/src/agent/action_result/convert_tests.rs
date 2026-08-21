use super::*;

#[test]
fn read_files_partial_success_converts_failed_files() {
    let result =
        api::request::input::tool_call_result::Result::try_from(ReadFilesResult::Success {
            files: vec![FileContext::new(
                "/tmp/success.txt".to_string(),
                AnyFileContent::StringContent("hello".to_string()),
                None,
                None,
            )],
            failed_files: vec![ReadFilesFailedFile {
                path: "/tmp/missing.txt".to_string(),
                message: "File not found or could not be read".to_string(),
            }],
        })
        .expect("read_files success should convert");

    let api::request::input::tool_call_result::Result::ReadFiles(result) = result else {
        panic!("expected read_files result");
    };

    let Some(api::read_files_result::Result::AnyFilesSuccess(success)) = result.result else {
        panic!("expected any files success result");
    };

    assert_eq!(success.files.len(), 1);
    assert_eq!(success.failed_reads.len(), 1);
    assert_eq!(success.failed_reads[0].path, "/tmp/missing.txt");
    assert_eq!(
        success.failed_reads[0].message,
        "File not found or could not be read"
    );
}

#[test]
fn fetch_conversation_cancelled_converts_to_error_instead_of_being_dropped() {
    // FetchConversation is a nested, server-driven tool call inside a
    // ConversationSearch subagent flow with no user checkpoint of its own.
    // Dropping this via `ConvertToAPITypeError::Ignore` leaves the server
    // waiting on a result that never arrives, so it must always round-trip.
    let result =
        api::request::input::tool_call_result::Result::try_from(FetchConversationResult::Cancelled)
            .expect("cancelled fetch should still convert to a wire result");

    let api::request::input::tool_call_result::Result::FetchConversation(result) = result else {
        panic!("expected fetch_conversation result");
    };

    let Some(api::fetch_conversation_result::Result::Error(error)) = result.result else {
        panic!("expected an error result, not a dropped/ignored conversion");
    };
    assert_eq!(error.message, "Fetching the conversation was cancelled");
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

use warp_terminal::event::ObservedExitStatus;

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

#[test]
fn recovered_shell_status_only_populates_observed_exit_codes() {
    let cases = [
        (ObservedExitStatus::Code(42), 42),
        (ObservedExitStatus::Code(0), 0),
        (ObservedExitStatus::Signal(9), 0),
        (ObservedExitStatus::Unavailable, 0),
    ];

    for (status, expected_exit_code) in cases {
        let converted = api::request::input::tool_call_result::Result::try_from(
            RequestCommandOutputResult::ShellRecovered {
                block_id: BlockId::new(),
                command: "exit".to_owned(),
                output: "recovered".to_owned(),
                status,
                restored_working_directory: "/home/agent".to_owned(),
                used_fallback_directory: false,
                start_ts: None,
                completed_ts: None,
            },
        )
        .expect("recovered result should convert");
        let api::request::input::tool_call_result::Result::RunShellCommand(result) = converted
        else {
            panic!("expected run shell command result");
        };
        let Some(api::run_shell_command_result::Result::CommandFinished(result)) = result.result
        else {
            panic!("expected completed shell command");
        };

        assert_eq!(result.exit_code, expected_exit_code);
    }
}

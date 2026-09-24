use prost::Message as _;
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

fn converted_recovered_command(
    status: ObservedExitStatus,
    output: &str,
) -> api::ShellCommandFinished {
    let converted = api::request::input::tool_call_result::Result::try_from(
        RequestCommandOutputResult::ShellRecovered {
            block_id: BlockId::new(),
            command: "exit".to_owned(),
            output: output.to_owned(),
            status,
            restored_working_directory: "/home/agent".to_owned(),
            used_fallback_directory: false,
            start_ts: None,
            completed_ts: None,
        },
    )
    .expect("recovered result should convert");
    let api::request::input::tool_call_result::Result::RunShellCommand(result) = converted else {
        panic!("expected run shell command result");
    };
    let Some(api::run_shell_command_result::Result::CommandFinished(result)) = result.result else {
        panic!("expected completed shell command");
    };
    result
}
fn converted_read_recovered_command(
    status: ObservedExitStatus,
    output: &str,
) -> api::ShellCommandFinished {
    let converted = api::request::input::tool_call_result::Result::try_from(
        ReadShellCommandOutputResult::ShellRecovered {
            block_id: BlockId::new(),
            command: "exit".to_owned(),
            output: output.to_owned(),
            status,
            start_ts: None,
            completed_ts: None,
        },
    )
    .expect("recovered read result should convert");
    let api::request::input::tool_call_result::Result::ReadShellCommandOutput(result) = converted
    else {
        panic!("expected read shell command result");
    };
    let Some(api::read_shell_command_output_result::Result::CommandFinished(result)) =
        result.result
    else {
        panic!("expected completed shell command");
    };
    result
}

#[test]
fn recovered_signal_and_unavailable_omit_wire_exit_code() {
    for status in [
        ObservedExitStatus::Signal(9),
        ObservedExitStatus::Unavailable,
    ] {
        for result in [
            converted_recovered_command(status, ""),
            converted_read_recovered_command(status, ""),
        ] {
            assert!(!result.encode_to_vec().contains(&0x10));
        }
    }
}

#[test]
fn recovered_observed_zero_is_explicit_in_stable_output() {
    let result =
        converted_recovered_command(ObservedExitStatus::Code(0), "Observed status: exit code 0");

    assert_eq!(result.exit_code, 0);
    assert_eq!(result.output, "Observed status: exit code 0");
    assert!(!result.encode_to_vec().contains(&0x10));
    let read_result = converted_read_recovered_command(
        ObservedExitStatus::Code(0),
        "Observed status: exit code 0",
    );
    assert_eq!(read_result.output, "Observed status: exit code 0");
    assert!(!read_result.encode_to_vec().contains(&0x10));
}

#[test]
fn recovered_nonzero_exit_code_is_serialized() {
    let result = converted_recovered_command(ObservedExitStatus::Code(42), "");

    assert_eq!(result.exit_code, 42);
    assert!(
        result
            .encode_to_vec()
            .windows(2)
            .any(|bytes| bytes == [0x10, 42])
    );
    let read_result = converted_read_recovered_command(ObservedExitStatus::Code(42), "");
    assert_eq!(read_result.exit_code, 42);
    assert!(
        read_result
            .encode_to_vec()
            .windows(2)
            .any(|bytes| bytes == [0x10, 42])
    );
}

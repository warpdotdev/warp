use crate::ai::agent::task::TaskId;
use crate::ai::agent::{
    AIAgentActionResult, AIAgentActionResultType, TransferShellCommandControlToUserResult,
};
use crate::terminal::model::block::BlockId;
use chrono::DateTime;
use chrono::Utc;
use warp_core::command::ExitCode;
use warp_multi_agent_api as api;

#[test]
fn transfer_control_snapshot_result_converts_to_tool_call_result_input() {
    let block_id = BlockId::default();
    let input =
        api::request::input::user_inputs::user_input::Input::try_from(AIAgentActionResult {
            id: "tool_call".to_string().into(),
            task_id: TaskId::new("task".to_string()),
            result: AIAgentActionResultType::TransferShellCommandControlToUser(
                TransferShellCommandControlToUserResult::Snapshot {
                    block_id: block_id.clone(),
                    grid_contents: "snapshot".to_string(),
                    cursor: "<|cursor|>".to_string(),
                    is_alt_screen_active: false,
                    is_preempted: false,
                },
            ),
        })
        .unwrap();

    match input {
        api::request::input::user_inputs::user_input::Input::ToolCallResult(result) => {
            assert_eq!(result.tool_call_id, "tool_call");
            match result.result {
                Some(api::request::input::tool_call_result::Result::TransferShellCommandControlToUser(
                    api_result,
                )) => match api_result.result {
                    Some(
                        api::transfer_shell_command_control_to_user_result::Result::LongRunningCommandSnapshot(snapshot),
                    ) => {
                        assert_eq!(snapshot.command_id, block_id.to_string());
                        assert_eq!(snapshot.output, "snapshot");
                        assert_eq!(snapshot.cursor, "<|cursor|>");
                    }
                    other => panic!("Expected snapshot result, got {other:?}"),
                },
                other => panic!("Expected transfer-control tool call result, got {other:?}"),
            }
        }
        other => panic!("Expected tool-call-result input, got {other:?}"),
    }
}

#[test]
fn transfer_control_finished_result_converts_to_tool_call_result_input() {
    let block_id = BlockId::default();
    let start_ts = DateTime::from(Utc::now());
    let completed_ts = DateTime::from(Utc::now());
    let input =
        api::request::input::user_inputs::user_input::Input::try_from(AIAgentActionResult {
            id: "tool_call".to_string().into(),
            task_id: TaskId::new("task".to_string()),
            result: AIAgentActionResultType::TransferShellCommandControlToUser(
                TransferShellCommandControlToUserResult::CommandFinished {
                    block_id: block_id.clone(),
                    output: "done".to_string(),
                    exit_code: ExitCode::from(17),
                    start_ts: Some(start_ts),
                    completed_ts: Some(completed_ts),
                },
            ),
        })
        .unwrap();

    match input {
        api::request::input::user_inputs::user_input::Input::ToolCallResult(result) => {
            assert_eq!(result.tool_call_id, "tool_call");
            match result.result {
                Some(api::request::input::tool_call_result::Result::TransferShellCommandControlToUser(
                    api_result,
                )) => match api_result.result {
                    Some(
                        api::transfer_shell_command_control_to_user_result::Result::CommandFinished(finished),
                    ) => {
                        assert_eq!(finished.command_id, block_id.to_string());
                        assert_eq!(finished.output, "done");
                        assert_eq!(finished.exit_code, 17);
                        assert_eq!(finished.start_ts, Some(super::local_datetime_to_timestamp(start_ts)));
                        assert_eq!(finished.finish_ts, Some(super::local_datetime_to_timestamp(completed_ts)));
                    }
                    other => panic!("Expected command-finished result, got {other:?}"),
                },
                other => panic!("Expected transfer-control tool call result, got {other:?}"),
            }
        }
        other => panic!("Expected tool-call-result input, got {other:?}"),
    }
}

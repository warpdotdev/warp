use anyhow::anyhow;
use serde_json::{Value, json};

use super::*;

const UNANSWERED_SERVER_TOOL: &str = "Internal error: Claude ended the turn without returning \
                                      results for tool calls: srvtoolu_018jms7VSGbY5G5N2SagnC2M";

fn rpc_error(code: i64, message: &str, data: Option<Value>) -> RpcError {
    RpcError {
        code,
        message: message.to_owned(),
        data,
    }
}

fn incomplete_tool_call(message: &str) -> RpcError {
    rpc_error(
        -32603,
        message,
        Some(json!({ "errorKind": "incomplete_tool_call" })),
    )
}

const FAILED: PromptErrorReading = PromptErrorReading::Failed(TurnFailure::TurnFailed);
const LOST: PromptErrorReading = PromptErrorReading::Failed(TurnFailure::SessionLost);

#[test]
fn an_agent_error_fails_the_turn_for_every_harness_unless_the_session_is_gone() {
    let overloaded = rpc_error(-32603, "model overloaded", None);
    let session_not_found = rpc_error(
        -32002,
        "Session sess_1 not found",
        Some(json!({ "sessionId": "sess_1" })),
    );
    for harness in [
        Harness::Claude,
        Harness::Codex,
        Harness::Gemini,
        Harness::Pi,
        Harness::Oz,
        Harness::OpenCode,
        Harness::Unknown,
    ] {
        assert_eq!(
            read_prompt_error(harness, &overloaded, false),
            FAILED,
            "{harness}"
        );
        assert_eq!(
            read_prompt_error(harness, &overloaded, true),
            LOST,
            "{harness}: the agent has disconnected"
        );
        assert_eq!(
            read_prompt_error(harness, &session_not_found, false),
            LOST,
            "{harness}: the agent no longer has the session"
        );
    }
}

#[test]
fn claude_rejecting_a_prompt_for_unanswered_server_tool_calls_is_a_completed_turn() {
    let one = incomplete_tool_call(UNANSWERED_SERVER_TOOL);
    assert_eq!(
        read_prompt_error(Harness::Claude, &one, false),
        PromptErrorReading::Completed
    );
    let several = incomplete_tool_call(
        "Internal error: Claude ended the turn without returning results for tool calls: \
         srvtoolu_01A, srvtoolu_01B",
    );
    assert_eq!(
        read_prompt_error(Harness::Claude, &several, false),
        PromptErrorReading::Completed
    );
}

#[test]
fn the_claude_reading_needs_every_named_call_to_be_a_server_tool_call() {
    for message in [
        // A client tool call left without a result is a real failure.
        "Internal error: Claude ended the turn without returning results for tool calls: \
         toolu_01A",
        "Internal error: Claude ended the turn without returning results for tool calls: \
         srvtoolu_01A, toolu_01B",
        // Nothing to tell which calls were left unanswered.
        "Internal error",
        "Internal error: Claude ended the turn without returning results for tool calls: ",
        "Internal error: Claude ended the turn without returning results for tool calls: \
         srvtoolu_",
        "Internal error: some other wording about srvtoolu_01A happened",
    ] {
        assert_eq!(
            read_prompt_error(Harness::Claude, &incomplete_tool_call(message), false),
            FAILED,
            "{message}"
        );
    }
}

#[test]
fn the_claude_reading_applies_to_no_other_harness_kind_or_connection_state() {
    let error = incomplete_tool_call(UNANSWERED_SERVER_TOOL);
    assert_eq!(read_prompt_error(Harness::Codex, &error, false), FAILED);
    assert_eq!(read_prompt_error(Harness::Unknown, &error, false), FAILED);
    assert_eq!(
        read_prompt_error(Harness::Claude, &error, true),
        LOST,
        "a disconnected agent is not read as having completed its turn"
    );

    for data in [
        None,
        Some(json!("incomplete_tool_call")),
        Some(json!({ "errorKind": "no_result" })),
        Some(json!({ "errorKind": "rate_limit" })),
        Some(json!({ "errorKind": 7 })),
    ] {
        let error = rpc_error(-32603, UNANSWERED_SERVER_TOOL, data.clone());
        assert_eq!(
            read_prompt_error(Harness::Claude, &error, false),
            FAILED,
            "{data:?}"
        );
    }
}

#[test]
fn codex_losing_its_app_server_loses_the_session_only_when_it_cannot_restart() {
    let exited = |data: Value| rpc_error(1001, "Codex app-server exited", Some(data));
    assert_eq!(
        read_prompt_error(
            Harness::Codex,
            &exited(json!({ "exitCode": 1, "signal": null, "restartable": false })),
            false
        ),
        LOST
    );
    assert_eq!(
        read_prompt_error(
            Harness::Codex,
            &exited(json!({ "exitCode": 1, "signal": null, "restartable": true })),
            false
        ),
        FAILED,
        "the next prompt respawns the app-server"
    );
    assert_eq!(
        read_prompt_error(Harness::Codex, &exited(json!({ "exitCode": 1 })), false),
        FAILED
    );
    assert_eq!(
        read_prompt_error(
            Harness::Codex,
            &rpc_error(1001, "Codex app-server exited", None),
            false
        ),
        FAILED
    );
    assert_eq!(
        read_prompt_error(
            Harness::Codex,
            &rpc_error(
                -32603,
                "Internal error",
                Some(json!({ "restartable": false }))
            ),
            false
        ),
        FAILED,
        "only the app-server's own code carries the flag"
    );
    assert_eq!(
        read_prompt_error(
            Harness::Claude,
            &exited(json!({ "restartable": false })),
            false
        ),
        FAILED
    );
}

#[test]
fn errors_the_agent_did_not_send_are_classified_by_the_connection_alone() {
    let driver_side = anyhow!("the agent did not answer session/cancel within 15s");
    assert_eq!(
        classify_turn_failure(Harness::Claude, &driver_side, false),
        TurnFailure::TurnFailed
    );
    assert_eq!(
        classify_turn_failure(Harness::Claude, &driver_side, true),
        TurnFailure::SessionLost
    );

    let agent_sent = anyhow::Error::new(rpc_error(-32002, "Session not found", None));
    assert_eq!(
        classify_turn_failure(Harness::Pi, &agent_sent, false),
        TurnFailure::SessionLost
    );
    let wrapped = anyhow::Error::new(rpc_error(
        1001,
        "Codex app-server exited",
        Some(json!({ "restartable": false })),
    ))
    .context("session/prompt failed");
    assert_eq!(
        classify_turn_failure(Harness::Codex, &wrapped, false),
        TurnFailure::SessionLost,
        "an agent's error is still read through added context"
    );
}

#[test]
fn each_kind_of_failure_has_its_own_metric_label() {
    assert_eq!(TurnFailure::TurnFailed.metric_label(), "error_turn_failed");
    assert_eq!(
        TurnFailure::SessionLost.metric_label(),
        "error_session_lost"
    );
}

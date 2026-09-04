use std::time::Duration;

use super::{AgentDriverError, McpServerWaitOutcome, startup_result_from_outcomes};

#[test]
fn startup_result_ignores_ready_outcomes() {
    let result = startup_result_from_outcomes(
        [McpServerWaitOutcome::Ready, McpServerWaitOutcome::Ready],
        Duration::from_secs(20),
    );
    assert!(result.is_ok());
}

#[test]
fn startup_result_collects_sorted_failure_details() {
    let result = startup_result_from_outcomes(
        [
            McpServerWaitOutcome::TimedOut {
                detail: "beta (uuid): Starting".to_string(),
            },
            McpServerWaitOutcome::FailedToStart {
                detail: "'alpha' failed to start".to_string(),
            },
        ],
        Duration::from_secs(20),
    );
    match result {
        Err(AgentDriverError::MCPStartupFailed { details }) => {
            assert_eq!(
                details,
                vec![
                    "'alpha' failed to start".to_string(),
                    "beta (uuid): Starting did not start within 20s".to_string(),
                ]
            );
        }
        other => panic!("expected MCPStartupFailed, got {other:?}"),
    }
}

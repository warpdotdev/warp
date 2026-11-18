use warp_core::command::ExitCode;

use super::AgentDriverError;
use super::harness::prepare_harness_failure_output;
use super::harness_output_monitor::DetectedHarnessError;

pub(super) fn final_save_error(harness: &str, error: anyhow::Error) -> AgentDriverError {
    AgentDriverError::HarnessConversationSaveFailed {
        harness: harness.to_owned(),
        detail: prepare_harness_failure_output(&format!("{error:#}")),
    }
}

pub(super) fn finish_harness_run(
    harness: String,
    command_result: Result<ExitCode, AgentDriverError>,
    failure_output: Option<String>,
    runtime_failure: Option<DetectedHarnessError>,
    final_save_result: Result<(), AgentDriverError>,
) -> Result<(), AgentDriverError> {
    // A runtime failure detected mid-run takes precedence over the
    // harness's own exit code: surface the actionable detail rather
    // than a generic "exit code N".
    if let Some(error) = runtime_failure {
        return Err(AgentDriverError::HarnessRuntimeFailureDetected {
            harness,
            pattern: error.pattern,
            excerpt: error.excerpt,
        });
    }

    let exit_code = command_result?;
    log::debug!("Agent harness exited with status {exit_code}");
    if !exit_code.was_successful() {
        return Err(AgentDriverError::HarnessCommandFailed {
            exit_code: exit_code.value(),
            output: failure_output,
        });
    }

    final_save_result
}

#[cfg(test)]
#[path = "harness_outcome_tests.rs"]
mod tests;

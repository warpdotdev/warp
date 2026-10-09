use chrono::{DateTime, Local};
use instant::Instant;
use warp_terminal::event::ObservedExitStatus;

use crate::ai::agent::AIAgentActionId;
use crate::terminal::model::block::BlockId;
use crate::terminal::model::session::SessionId;

#[cfg_attr(target_family = "wasm", allow(dead_code))]
const CLOUD_SHELL_RECOVERY_GUIDANCE: &str = "This command terminated the persistent cloud shell. Warp started a replacement shell and did not replay the command. Some shell state might be lost. Do not use `exit`, `logout`, `exec`, `kill $$`, or source a script that exits. Run risky exit logic in a subshell, and use the tool result to inspect its exit code. Check the reported restored state and partial side effects before retrying.";

#[derive(Debug, Clone)]
pub struct CloudShellRecoveryRequest {
    pub action_id: AIAgentActionId,
    pub block_id: BlockId,
    pub partial_output: String,
    pub status: ObservedExitStatus,
    pub requested_working_directory: Option<String>,
    pub session_id: Option<SessionId>,
    pub start_ts: Option<DateTime<Local>>,
    pub recovery_started_at: Instant,
}

#[cfg_attr(target_family = "wasm", allow(dead_code))]
pub(crate) fn recovered_command_output(
    partial_output: &str,
    status: ObservedExitStatus,
    restored_working_directory: &str,
    used_fallback_directory: bool,
) -> String {
    let fallback = if used_fallback_directory {
        " (fallback directory)"
    } else {
        ""
    };
    let mut output = format!(
        "{CLOUD_SHELL_RECOVERY_GUIDANCE}\n\nRecovery details:\n- Observed status: {status}\n- Restored working directory: {restored_working_directory}{fallback}\n- The interrupted command was not replayed.\n- Partial side effects may remain; aliases, functions, jobs, traps, shell options, shell-local variables, process groups, open file descriptors, and unflushed history may be lost."
    );
    if !partial_output.is_empty() {
        output.push_str("\n\nPartial command output:\n");
        output.push_str(partial_output);
    }
    output
}

#[cfg(test)]
#[path = "shell_recovery_tests.rs"]
mod tests;

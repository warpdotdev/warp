use std::collections::HashMap;
use std::ffi::{OsStr, OsString};

use chrono::{DateTime, Local};
use instant::Instant;
use warp_terminal::event::ObservedExitStatus;

use crate::ai::agent::AIAgentActionId;
use crate::ai::agent::conversation::AIConversationId;
use crate::terminal::model::block::BlockId;
use crate::terminal::model::session::SessionId;

pub(crate) const MAX_CLOUD_SHELL_RECOVERIES: u8 = 3;
pub(crate) const CLOUD_SHELL_RECOVERY_GUIDANCE: &str = "This command terminated the persistent cloud shell. Warp started a replacement shell and did not replay the command. Some shell state might be lost. Do not use `exit`, `logout`, `exec`, `kill $$`, or source a script that exits. Run risky exit logic in a subshell, and use the tool result to inspect its exit code. Check the reported restored state and partial side effects before retrying.";
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CloudShellRecoveryDecision {
    Attempt(u8),
    Capped,
    Ineligible,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct CloudShellRecoveryEligibility {
    pub feature_enabled: bool,
    pub manual_shutdown_requested: bool,
    pub recovery_in_progress: bool,
    pub terminal_failure: bool,
    pub recovery_count: u8,
    pub login_shell_bootstrapped: bool,
    pub third_party_harness: bool,
    pub shared_ambient_session: bool,
    pub active_sharer: bool,
    pub running_environment_setup: bool,
}

impl CloudShellRecoveryEligibility {
    pub fn decision(self) -> CloudShellRecoveryDecision {
        if !self.feature_enabled
            || self.manual_shutdown_requested
            || self.recovery_in_progress
            || self.terminal_failure
            || !self.login_shell_bootstrapped
            || self.third_party_harness
            || !self.shared_ambient_session
            || !self.active_sharer
            || self.running_environment_setup
        {
            return CloudShellRecoveryDecision::Ineligible;
        }
        if self.recovery_count >= MAX_CLOUD_SHELL_RECOVERIES {
            return CloudShellRecoveryDecision::Capped;
        }
        CloudShellRecoveryDecision::Attempt(self.recovery_count + 1)
    }
}

#[derive(Debug, Clone)]
pub struct CloudShellRecoveryRequest {
    pub action_id: AIAgentActionId,
    pub conversation_id: AIConversationId,
    pub block_id: BlockId,
    pub partial_output: String,
    pub status: ObservedExitStatus,
    pub requested_working_directory: Option<String>,
    pub session_id: Option<SessionId>,
    pub start_ts: Option<DateTime<Local>>,
    pub attempt: u8,
    pub recovery_started_at: Instant,
    pub dynamic_session_environment_available: bool,
}

pub(crate) fn sanitized_recovery_environment(
    original: &HashMap<OsString, OsString>,
    dynamic: Option<HashMap<String, String>>,
) -> HashMap<OsString, OsString> {
    let mut restored = original
        .iter()
        .filter(|(key, _)| is_recoverable_environment_key(key))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect::<HashMap<_, _>>();
    for (key, value) in dynamic.into_iter().flatten() {
        let key = OsString::from(key);
        if is_recoverable_environment_key(&key) {
            restored.insert(key, OsString::from(value));
        }
    }
    restored
}

fn is_recoverable_environment_key(key: &OsStr) -> bool {
    let key = key.to_string_lossy();
    !key.starts_with("WARP_")
        && !key.starts_with("BASH_FUNC_")
        && !matches!(
            key.as_ref(),
            "PWD"
                | "OLDPWD"
                | "SHLVL"
                | "_"
                | "HOME"
                | "SHELL"
                | "TERM"
                | "TERM_PROGRAM"
                | "TERM_PROGRAM_VERSION"
                | "COLORTERM"
                | "HISTFILESIZE"
                | "HISTSIZE"
        )
}

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

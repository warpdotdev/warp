//! Signal-based process control for forcibly terminating a third-party
//! harness's CLI process when it doesn't exit gracefully within the exit
//! escalation ladder in [`super::super::run_harness`].
//!
//! This mirrors two existing, independent implementations rather than
//! reusing either directly, to avoid widening either module's visibility for
//! this one new caller:
//! - The process-group `SIGKILL` already used for local generator commands
//!   (`crate::terminal::model::session::command_executor::local_command_executor`).
//! - The foreground-process-group lookup already used for long-running
//!   command activity sampling
//!   (`crate::ai::blocklist::action_model::execute::lrc_activity::sampler`).
//!
//! Consolidating all three into one shared utility is a reasonable follow-up
//! once this new caller is established.

use crate::terminal::model::terminal_model::ShellProcessInfo;

/// Returns the pty's current foreground process group, if any.
///
/// The shell's own pid is deliberately not what callers want here: an
/// interactively-launched CLI harness (Claude Code, Codex) runs as its own
/// foreground process group under the shell, and that's what needs to be
/// signaled to actually stop the harness rather than just the shell that
/// launched it.
#[cfg(unix)]
pub(super) fn foreground_pgid(shell: &ShellProcessInfo) -> Option<u32> {
    let fd = shell.pty_leader_fd?;
    // SAFETY: `tcgetpgrp` only reads terminal state for `fd`. A stale or
    // reused descriptor makes it fail or answer about an unrelated
    // terminal; both are handled by returning `None`.
    let pgid = unsafe { libc::tcgetpgrp(fd) };
    (pgid > 0).then_some(pgid as u32)
}

#[cfg(not(unix))]
pub(super) fn foreground_pgid(_shell: &ShellProcessInfo) -> Option<u32> {
    None
}

/// Sends `SIGKILL` to every process in the given process group.
///
/// Fire-and-forget: `SIGKILL` cannot be caught, blocked, or ignored, so the
/// kernel guarantees the targeted processes will be torn down once this
/// call succeeds. Callers do not need to wait for confirmation that the
/// process actually exited before proceeding.
#[cfg(unix)]
pub(super) fn kill_process_group(pgid: u32) {
    use nix::sys::signal::{Signal, kill};
    use nix::unistd::Pid;

    // A pgid of 0 targets the caller's own process group, and 1 negates to
    // -1, which SIGKILLs every process this user is allowed to signal.
    // Neither is ever a legitimate target, so refuse them rather than let a
    // bad pgid reach `kill`.
    if pgid < 2 {
        log::warn!("Refusing to force-kill process group {pgid}: pid is below 2");
        return;
    }

    // Killing a negative pid kills every process in that process group.
    match kill(Pid::from_raw(-(pgid as i32)), Signal::SIGKILL) {
        Ok(()) => log::info!("Force-killed harness process group {pgid}"),
        Err(nix::errno::Errno::ESRCH) => {
            log::info!("Harness process group {pgid} had already exited");
        }
        Err(error) => {
            log::warn!("Failed to force-kill harness process group {pgid}: {error}");
        }
    }
}

#[cfg(not(unix))]
pub(super) fn kill_process_group(_pgid: u32) {}

//! Shared arguments for spawning and dispatching subprocesses of the terminal.

pub const PARENT_PID_ARG: &str = "parent-pid";
#[cfg(unix)]
pub const TERMINAL_SERVER: &str = "terminal-server";
pub const RIPGREP_SEARCH: &str = "ripgrep-search";

pub fn parent_pid_flag() -> String {
    format!("--{PARENT_PID_ARG}={}", std::process::id())
}

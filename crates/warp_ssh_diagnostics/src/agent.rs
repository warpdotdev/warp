//! Probe `ssh-agent` health and surface a structured status.
//!
//! Three outcomes:
//!
//! - [`SshAgentStatus::Available`] — `$SSH_AUTH_SOCK` is set and
//!   the socket answers `ssh-add -l`. `keys_loaded` reports the
//!   count (0 is legal — agent's running but no keys added yet).
//! - [`SshAgentStatus::NotConfigured`] — `$SSH_AUTH_SOCK` is unset.
//!   Most commonly the user launched Warp from Finder/Spotlight
//!   rather than from a shell that had the agent set up.
//! - [`SshAgentStatus::Stale`] — the env var points at a socket
//!   that doesn't answer. The agent was probably killed (or
//!   restarted with a fresh socket path) since the desktop
//!   launched.
//!
//! The probe runs `ssh-add -l` through the [`SshAddRunner`] trait
//! so tests substitute canned outputs instead of spawning real
//! subprocesses.

#[cfg(not(target_family = "wasm"))]
use std::process::Stdio;
#[cfg(not(target_family = "wasm"))]
use std::time::Duration;

#[cfg(not(target_family = "wasm"))]
use command::blocking::Command;
#[cfg(not(target_family = "wasm"))]
use instant::Instant;
use serde::{Deserialize, Serialize};

/// Health snapshot for the ambient `ssh-agent`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "state")]
pub enum SshAgentStatus {
    /// Agent is running and reachable. `keys_loaded` reports the
    /// count from `ssh-add -l`.
    Available {
        socket_path: String,
        keys_loaded: u32,
    },
    /// `$SSH_AUTH_SOCK` isn't set in the desktop's environment.
    NotConfigured,
    /// `$SSH_AUTH_SOCK` is set but the socket didn't answer the
    /// probe. `reason` is the best-effort human-readable cause
    /// (timeout, spawn failure, non-zero exit).
    Stale { socket_path: String, reason: String },
}

/// Output of an `ssh-add -l` invocation. Mirrors `process::Output`
/// minus the raw bytes the parser doesn't need.
pub struct SshAddOutcome {
    /// Exit code. `Some(0)` means keys are listed; `Some(1)` means
    /// "the agent has no identities" (still a healthy agent); other
    /// codes / spawn failures fold to `None` so the caller can flag
    /// as stale.
    pub status_code: Option<i32>,
    /// Combined stdout. Parser counts non-empty lines for the
    /// "keys loaded" stat.
    pub stdout: String,
}

/// Plumbed so tests drive a shim impl with scripted output instead
/// of depending on the host's real `ssh-add` binary.
pub trait SshAddRunner: Send + Sync {
    /// Run `ssh-add -l` with the desktop's environment plus
    /// `SSH_AUTH_SOCK=<socket>`. Production callers swallow IO
    /// errors and surface them as the `None` exit-code variant.
    fn list_keys(&self, socket: &str) -> SshAddOutcome;
}

/// Production [`SshAddRunner`] that spawns the real `ssh-add`.
/// Not available on wasm targets — subprocess spawning doesn't
/// exist there, and the production probe is only meaningful on a
/// host that has openssh installed.
#[cfg(not(target_family = "wasm"))]
pub struct ProcessSshAddRunner;

#[cfg(not(target_family = "wasm"))]
impl SshAddRunner for ProcessSshAddRunner {
    fn list_keys(&self, socket: &str) -> SshAddOutcome {
        // 750ms is way more than enough for `ssh-add -l` against a
        // healthy agent (a couple ms locally). The point is to fail
        // fast on a wedged socket so the diagnostics chip refresh
        // doesn't stall behind a hung Unix-socket connect.
        let mut cmd = Command::new("ssh-add");
        cmd.arg("-l").env("SSH_AUTH_SOCK", socket);
        run_with_timeout(cmd, Duration::from_millis(750)).unwrap_or(SshAddOutcome {
            status_code: None,
            stdout: String::new(),
        })
    }
}

/// Production entry point — uses the process environment + the
/// real `ssh-add` binary. Not available on wasm; wasm callers stick
/// with [`ssh_agent_status_with`] + a stubbed runner if they need
/// the type-level surface.
#[cfg(not(target_family = "wasm"))]
pub fn ssh_agent_status() -> SshAgentStatus {
    ssh_agent_status_with(
        &ProcessSshAddRunner,
        std::env::var("SSH_AUTH_SOCK").ok().as_deref(),
    )
}

/// Test seam: explicit `runner` + explicit `socket` (or `None`).
/// Pure function — no env access — so tests can drive every state.
pub fn ssh_agent_status_with(runner: &dyn SshAddRunner, socket: Option<&str>) -> SshAgentStatus {
    let Some(socket) = socket.filter(|s| !s.is_empty()) else {
        return SshAgentStatus::NotConfigured;
    };

    let outcome = runner.list_keys(socket);
    match outcome.status_code {
        Some(0) => SshAgentStatus::Available {
            socket_path: socket.to_string(),
            keys_loaded: count_listed_keys(&outcome.stdout),
        },
        Some(1) => {
            // openssh's `ssh-add -l` exits 1 with the body "The
            // agent has no identities." That's still a healthy
            // agent — the user just hasn't loaded keys.
            SshAgentStatus::Available {
                socket_path: socket.to_string(),
                keys_loaded: 0,
            }
        }
        Some(code) => SshAgentStatus::Stale {
            socket_path: socket.to_string(),
            reason: format!("ssh-add exited {code}"),
        },
        None => SshAgentStatus::Stale {
            socket_path: socket.to_string(),
            reason: "ssh-add could not be spawned (or hung past the timeout)".to_string(),
        },
    }
}

/// Count non-empty lines in `ssh-add -l` stdout. Each loaded key
/// produces one line of the form
/// `<bits> SHA256:<hash> <comment> (<type>)`.
fn count_listed_keys(stdout: &str) -> u32 {
    stdout
        .lines()
        .filter(|line| !line.trim().is_empty())
        .count() as u32
}

#[cfg(not(target_family = "wasm"))]
fn run_with_timeout(mut cmd: Command, timeout: Duration) -> Option<SshAddOutcome> {
    cmd.stdout(Stdio::piped())
        .stderr(Stdio::null())
        .stdin(Stdio::null());
    let mut child = cmd.spawn().ok()?;
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                use std::io::Read;
                let mut stdout = String::new();
                if let Some(mut handle) = child.stdout.take() {
                    let _ = handle.read_to_string(&mut stdout);
                }
                return Some(SshAddOutcome {
                    status_code: status.code(),
                    stdout,
                });
            }
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(25)),
            Err(_) => return None,
        }
    }
}

#[cfg(test)]
#[path = "agent_tests.rs"]
mod tests;

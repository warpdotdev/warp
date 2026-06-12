//! SSH connection diagnostics — failure-mode classification, key /
//! identity enumeration, and `ssh-agent` health probing.
//!
//! Surfaces actionable explanations for common SSH errors and lets
//! the UI render a diagnostics chip without re-implementing the
//! state machine in every consumer. R3.3 of the SSH connection
//! management roadmap.
//!
//! Three independent pieces:
//!
//! 1. [`ConnectionFailureMode`] + [`classify_ssh_stderr`] — turn an
//!    ssh client stderr blob into a structured cause + suggested
//!    fix. Pure pattern matching, no IO.
//! 2. [`SshIdentity`] + [`list_ssh_identities`] — enumerate visible
//!    public keys in `~/.ssh` with a private-key-presence flag.
//!    Best-effort filesystem read.
//! 3. [`SshAgentStatus`] + [`ssh_agent_status`] — classify the
//!    current ssh-agent state (Available / NotConfigured / Stale).
//!    Driven via the [`SshAddRunner`] trait so tests substitute
//!    canned outputs instead of spawning real subprocesses.

mod agent;
mod failure_modes;
mod identities;

#[cfg(not(target_family = "wasm"))]
pub use agent::{ssh_agent_status, ProcessSshAddRunner};
pub use agent::{ssh_agent_status_with, SshAddOutcome, SshAddRunner, SshAgentStatus};
pub use failure_modes::{classify_ssh_stderr, ConnectionFailureMode};
pub use identities::{list_ssh_identities, list_ssh_identities_in, SshIdentity};

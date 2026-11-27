use std::ffi::OsString;

use serde::{Deserialize, Serialize};

use crate::event::ObservedExitStatus;
use crate::local_tty::{PtyOptions, PtySpawnResult};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) enum Result<T> {
    Ok(T),
    Err(String),
}

impl<T> From<anyhow::Result<T>> for Result<T> {
    fn from(value: anyhow::Result<T>) -> Self {
        match value {
            Ok(val) => self::Result::Ok(val),
            Err(err) => self::Result::Err(err.to_string()),
        }
    }
}

/// The API for communication between the terminal client and server.  This is
/// organized into request/response pairs for the API "methods".
///
/// ### Future work
/// We may want to structure this differently to group messages sent by the client or server,
/// simplifying each side's parsing logic. We currently check, for example, that the server does
/// not receive a message that should only be sent from the server to the client. It would be
/// preferable if that check were unnecessary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) enum Message {
    /// A message sent from client -> server requesting that the server spawns
    /// a new pty using the provided options.
    SpawnShellRequest { options: Box<PtyOptions> },
    /// The response for a `SpawnShellRequest`, with the result of the spawn
    /// operation.  Should only be sent from server -> client.
    SpawnShellResponse {
        spawn_result: Result<PtySpawnResult>,
    },
    /// A message sent from client -> server requesting that the server kill the
    /// child process with the provided process ID.
    KillChildRequest { pid: u32 },
    /// The response for a `KillChildRequest`, returning the string message from
    /// an error that occurred during the operation, if any.  Should only be
    /// sent from server -> client.
    KillChildResponse { error_msg: Option<String> },
    /// A message sent from server -> client requesting that a log message be
    /// written to the host application's log.  This has no matching response
    /// message - these requests are fire-and-forget from the server to the
    /// host application.
    WriteLogRequest {
        level: log::Level,
        target: String,
        message: String,
    },
    /// A message sent from server -> client notifying the client that one or
    /// more child processes have terminated.  This has no matching response
    /// message - these requests are fire-and-forget from the server to the
    /// host application.
    ChildrenTerminatedRequest {
        children: Vec<(u32, ObservedExitStatus)>,
    },
}

pub const DAEMON_PROTOCOL_VERSION: u32 = 2;
pub const RING_CAPACITY: usize = 10 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PersistentSessionId(pub [u8; 16]);

impl From<uuid::Uuid> for PersistentSessionId {
    fn from(value: uuid::Uuid) -> Self {
        Self(value.into_bytes())
    }
}

impl From<PersistentSessionId> for uuid::Uuid {
    fn from(value: PersistentSessionId) -> Self {
        uuid::Uuid::from_bytes(value.0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMetadata {
    pub hook_session_id: u64,
    pub shell_type: String,
}

/// The environment of the client at spawn time. The daemon passes these
/// variables as the shell's base environment (`PtyOptions::base_env`), so the
/// shell starts as if the client had spawned it instead of inheriting the
/// environment the daemon started with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpawnEnvironment {
    pub vars: Vec<(OsString, OsString)>,
}

impl SpawnEnvironment {
    /// Captures the current process environment.
    pub fn capture() -> Self {
        Self {
            vars: std::env::vars_os().collect(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ClientMessage {
    Hello {
        protocol_version: u32,
        client_pid: u32,
        app_version: Option<String>,
    },
    Request {
        request_id: u64,
        request: DaemonRequest,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DaemonRequest {
    List,
    Spawn {
        session_id: PersistentSessionId,
        metadata: SessionMetadata,
        options: Box<PtyOptions>,
        /// `None` keeps the daemon's own environment.
        environment: Option<SpawnEnvironment>,
    },
    Attach {
        session_id: PersistentSessionId,
    },
    Kill {
        session_id: PersistentSessionId,
    },
    Prune {
        keep: Vec<PersistentSessionId>,
    },
    DetachAll,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DaemonMessage {
    HelloAck {
        protocol_version: u32,
        daemon_pid: u32,
    },
    HelloReject {
        protocol_version: u32,
        reason: String,
    },
    Response {
        request_id: u64,
        response: DaemonResponse,
    },
    Event(DaemonEvent),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DaemonResponse {
    Sessions(Vec<SessionSummary>),
    Attached(AttachInfo),
    Done,
    Error(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSummary {
    pub session_id: PersistentSessionId,
    pub pid: u32,
    pub metadata: SessionMetadata,
    pub exit_status: Option<ObservedExitStatus>,
    pub attached: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttachInfo {
    pub session_id: PersistentSessionId,
    pub pid: u32,
    pub metadata: SessionMetadata,
    pub replay_len: u64,
    pub has_bootstrap_preamble: bool,
    pub exit_status: Option<ObservedExitStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DaemonEvent {
    SessionExited {
        session_id: PersistentSessionId,
        pid: u32,
        status: ObservedExitStatus,
    },
}

#[cfg(test)]
#[path = "api_tests.rs"]
mod tests;

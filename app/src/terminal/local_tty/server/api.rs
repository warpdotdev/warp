use serde::{Deserialize, Serialize};

use crate::terminal::local_tty::{PtyOptions, PtySpawnResult};

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
/// * We may want to structure this slightly differently to group
/// messages sent by the client or sent by the server, simplifying logic that
/// exists on each side for message parsing.  (We currently have error-checking
/// logic to ensure that, for example, the server doesn't receive a message that
/// should only be sent server->client; it would be preferable if we didn't need
/// to ever perform that check.)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) enum Message {
    /// A message sent from client -> server requesting that the server spawns
    /// a new pty using the provided options.
    SpawnShellRequest { options: PtyOptions },
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
    /// A message sent from client -> server requesting that the server clean
    /// up all hosted shell processes before shutting itself down.
    ShutdownRequest,
    /// The response for a `ShutdownRequest`, sent once hosted shell cleanup
    /// has completed. Should only be sent from server -> client.
    ShutdownResponse,
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
    ChildrenTerminatedRequest { pids: Vec<u32> },
}

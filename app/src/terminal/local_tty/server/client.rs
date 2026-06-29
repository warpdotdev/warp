use std::{
    collections::HashSet,
    os::unix::prelude::*,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

use anyhow::{anyhow, bail, Result};
use parking_lot::Mutex;

use crate::terminal::local_tty::{PtyOptions, PtySpawnResult};

use super::{api, protocol};

/// A client for communicating with the terminal server.
///
/// This owns the client side file descriptor for the Unix domain socket which
/// we use to communicate with the terminal server.  This structure should be
/// passed around inside an [`Arc`](std::sync::Arc) so that it can be accessed
/// from various locations within the codebase.
pub struct TerminalServerClient {
    /// The file descriptor for the Unix domain socket that is connected to the
    /// terminal server.
    ///
    /// This is stored inside a Mutex so that the client can guarantee ownership
    /// of the socket across a send/receive pair (to avoid interference from
    /// other threads).
    socket_fd: Mutex<OwnedFd>,
    /// The set of process IDs of terminated children which have not yet been
    /// processed by the pty event loops.
    terminated_children: Arc<Mutex<HashSet<u32>>>,
    next_request_id: AtomicU64,
}

impl TerminalServerClient {
    /// Constructs a new terminal server client which communicates with the
    /// server via the provided Unix domain socket file descriptor and holds
    /// onto a list of terminated child process IDs.
    pub fn new(client_fd: OwnedFd, terminated_children: Arc<Mutex<HashSet<u32>>>) -> Self {
        Self {
            socket_fd: Mutex::new(client_fd),
            terminated_children,
            next_request_id: AtomicU64::new(1),
        }
    }

    /// Asks the server to spawn a pty, returning the pty leader file descriptor
    /// and other metadata upon success.
    pub fn spawn_pty(&self, options: PtyOptions) -> Result<PtySpawnResult> {
        let request_id = self.next_request_id();

        // Lock access to the socket to ensure that nothing interferes with our
        // request/response handshake.
        let fd = self.socket_fd.lock();

        protocol::send_message(
            fd.as_fd(),
            api::Message::SpawnShellRequest {
                request_id,
                options,
            },
            Option::<RawFd>::None,
        )?;

        loop {
            let result = protocol::receive_message(fd.as_fd())?;
            match result {
                Some(api::Message::SpawnShellResponse {
                    request_id: response_request_id,
                    spawn_result: api::Result::Ok(spawn_result),
                }) => {
                    if response_request_id == request_id {
                        return Ok(spawn_result);
                    }
                    log::warn!(
                        "Ignoring stale SpawnShellResponse while waiting for request {request_id}: got {response_request_id}"
                    );
                    close_stale_spawn_result(spawn_result);
                }
                Some(api::Message::SpawnShellResponse {
                    request_id: response_request_id,
                    spawn_result: api::Result::Err(message),
                }) => {
                    if response_request_id == request_id {
                        bail!("Terminal server failed to spawn a shell: {message}");
                    }
                    log::warn!(
                        "Ignoring stale failed SpawnShellResponse while waiting for request {request_id}: got {response_request_id}: {message}"
                    );
                }
                Some(api::Message::KillChildResponse {
                    request_id: response_request_id,
                    error_msg,
                }) => {
                    log::warn!(
                        "Ignoring stale KillChildResponse while waiting for SpawnShellResponse request {request_id}: got {response_request_id}: {error_msg:?}"
                    );
                }
                Some(api::Message::ChildrenTerminatedRequest { pids }) => {
                    self.terminated_children.lock().extend(pids);
                }
                Some(api::Message::WriteLogRequest {
                    level,
                    target,
                    message,
                }) => {
                    super::logging::handle_write_log_request(level, target, message);
                }
                Some(_) => {
                    bail!("Got response message other than SpawnShellResponse after sending a SpawnShellRequest message!");
                }
                None => {
                    bail!("Received error reading message back from terminal server");
                }
            }
        }
    }

    /// Asks the server to terminate and clean up its child process with the
    /// given process ID.
    pub fn kill_child(&self, pid: u32) -> Result<()> {
        if self.has_child_terminated(pid) {
            return Ok(());
        }
        let request_id = self.next_request_id();

        // Lock access to the socket to ensure that nothing interferes with our
        // request/response handshake.
        let fd = self.socket_fd.lock();

        if let Err(error) = protocol::send_message(
            fd.as_fd(),
            api::Message::KillChildRequest { request_id, pid },
            Option::<RawFd>::None,
        ) {
            if error.downcast_ref::<nix::Error>() == Some(&nix::Error::EPIPE) {
                log::warn!("Received EPIPE when trying to kill child shell process; assuming the terminal server has terminated.");
                return Ok(());
            } else {
                return Err(error);
            }
        }

        loop {
            let result = protocol::receive_message(fd.as_fd())?;
            match result {
                Some(api::Message::KillChildResponse {
                    request_id: response_request_id,
                    error_msg,
                }) => {
                    if response_request_id == request_id {
                        return match error_msg {
                            Some(error_msg) => Err(anyhow!(error_msg)),
                            None => Ok(()),
                        };
                    }
                    log::warn!(
                        "Ignoring stale KillChildResponse while waiting for request {request_id}: got {response_request_id}: {error_msg:?}"
                    );
                }
                Some(api::Message::SpawnShellResponse {
                    request_id: response_request_id,
                    spawn_result,
                }) => {
                    log::warn!(
                        "Ignoring stale SpawnShellResponse while waiting for KillChildResponse request {request_id}: got {response_request_id}"
                    );
                    if let api::Result::Ok(spawn_result) = spawn_result {
                        close_stale_spawn_result(spawn_result);
                    }
                }
                Some(api::Message::ChildrenTerminatedRequest { pids }) => {
                    self.terminated_children.lock().extend(pids);
                }
                Some(api::Message::WriteLogRequest {
                    level,
                    target,
                    message,
                }) => {
                    super::logging::handle_write_log_request(level, target, message);
                }
                Some(_) => {
                    bail!("Got response message other than KillChildResponse after sending a KillChildRequest message!");
                }
                None => {
                    bail!("Received error reading message back from terminal server");
                }
            }
        }
    }

    /// Returns whether or not the child process with the given process ID has
    /// terminated.  This will only return true once for each process ID.
    pub fn has_child_terminated(&self, pid: u32) -> bool {
        self.terminated_children.lock().remove(&pid)
    }

    fn next_request_id(&self) -> api::RequestId {
        self.next_request_id.fetch_add(1, Ordering::Relaxed)
    }
}

fn close_stale_spawn_result(spawn_result: PtySpawnResult) {
    if spawn_result.leader_fd >= 0 {
        if let Err(err) = nix::unistd::close(spawn_result.leader_fd) {
            log::warn!(
                "Failed to close fd for stale SpawnShellResponse pid {}: {err:?}",
                spawn_result.pid
            );
        }
    }
}

#[cfg(test)]
#[path = "client_tests.rs"]
mod tests;

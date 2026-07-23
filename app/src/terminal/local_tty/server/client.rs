use std::collections::HashSet;
use std::os::unix::prelude::*;
use std::sync::Arc;

use anyhow::{Result, bail};
use parking_lot::Mutex;

use super::{api, protocol};
use crate::terminal::local_tty::{PtyOptions, PtySpawnResult};

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
}

impl TerminalServerClient {
    /// Constructs a new terminal server client which communicates with the
    /// server via the provided Unix domain socket file descriptor and holds
    /// onto a list of terminated child process IDs.
    pub fn new(client_fd: OwnedFd, terminated_children: Arc<Mutex<HashSet<u32>>>) -> Self {
        Self {
            socket_fd: Mutex::new(client_fd),
            terminated_children,
        }
    }

    /// Asks the server to spawn a pty, returning the pty leader file descriptor
    /// and other metadata upon success.
    pub fn spawn_pty(&self, options: PtyOptions) -> Result<PtySpawnResult> {
        // Lock access to the socket to ensure that nothing interferes with our
        // request/response handshake.
        let fd = self.socket_fd.lock();

        protocol::send_message(
            fd.as_fd(),
            api::Message::SpawnShellRequest { options },
            Option::<RawFd>::None,
        )?;

        let result = protocol::receive_message(fd.as_fd())?;
        match result {
            Some(api::Message::SpawnShellResponse {
                spawn_result: api::Result::Ok(spawn_result),
            }) => Ok(spawn_result),
            Some(api::Message::SpawnShellResponse {
                spawn_result: api::Result::Err(message),
            }) => {
                bail!("Terminal server failed to spawn a shell: {message}");
            }
            Some(_) => {
                bail!(
                    "Got response message other than SpawnShellResponse after sending a SpawnShellRequest message!"
                );
            }
            None => {
                bail!("Received error reading message back from terminal server");
            }
        }
    }

    /// Asks the server to terminate and clean up its child process with the
    /// given process ID.
    pub fn kill_child(&self, pid: u32) -> Result<()> {
        if self.has_child_terminated(pid) {
            return Ok(());
        }

        // Lock access to the socket to ensure that nothing interferes with our
        // request/response handshake.
        let fd = self.socket_fd.lock();

        if let Err(error) = protocol::send_message(
            fd.as_fd(),
            api::Message::KillChildRequest { pid },
            Option::<RawFd>::None,
        ) {
            if error.downcast_ref::<nix::Error>() == Some(&nix::Error::EPIPE) {
                log::warn!(
                    "Received EPIPE when trying to kill child shell process; assuming the terminal server has terminated."
                );
                return Ok(());
            } else {
                return Err(error);
            }
        }

        let result = protocol::receive_message(fd.as_fd())?;
        match result {
            Some(api::Message::KillChildResponse { error_msg }) => match error_msg {
                Some(error_msg) => Err(anyhow::anyhow!(error_msg)),
                None => Ok(()),
            },
            Some(_) => {
                bail!(
                    "Got response message other than KillChildResponse after sending a KillChildRequest message!"
                );
            }
            None => {
                bail!("Received error reading message back from terminal server");
            }
        }
    }

    /// Returns whether or not the child process with the given process ID has
    /// terminated.  This will only return true once for each process ID.
    pub fn has_child_terminated(&self, pid: u32) -> bool {
        self.terminated_children.lock().remove(&pid)
    }
}

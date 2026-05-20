//! Logic relating to a secondary process which Warp uses to spawn new shell
//! sessions.
//!
//! The purpose of this "server" process is to ensure that shell processes are
//! created in as clean of a state as possible.  In general, the act of creating
//! a new process in Unix "leaks" some state from the parent into the child
//! (e.g.: open file descriptors).  By spawning this secondary process very
//! early during Warp's initialization, we can be more confident that there are
//! no resources that will leak into shell processes (and, subsequently, into
//! the programs the user runs within the shell).
//!
//! ## Important Notes
//!
//! All reads and writes _must_ be paired, otherwise the app can end up
//! deadlocked waiting for a response that never arrives.  The APIs here can be
//! better designed to make it impossible for this to occur (e.g.: encapsulate
//! the receive/send pair in a single function which delegates to helpers that
//! transform a request into a response), but until then, newly added code will
//! need to enforce this invariant.

mod api;
mod client;
mod event_loop;
mod logging;
mod protocol;

use command::blocking::Command;
use std::{collections::HashSet, os::unix::prelude::*, sync::Arc};

use anyhow::{Context, Result};
use cvt::cvt;
use nix::sys::socket;
use parking_lot::Mutex;

use crate::init_feature_flags;

pub use self::client::TerminalServerClient;

use super::spawner::PtyHandle;

/// The file descriptor of the Unix domain socket where the terminal server will
/// receive requests from the host application.
///
/// ### Future work
/// * We could pass the file descriptor index to the child process as a command
/// line argument - this might enable the use of `posix_spawn` as the underlying
/// process creation behavior (vs. fork/exec).  (`posix_spawn` is typically a
/// bit more performant, though not sure how much that matters given this is a
/// one-time cost.)
const RECV_SOCKET_FILENO: RawFd = libc::STDERR_FILENO + 1;
/// The file descriptor of the Unix domain socket where the terminal server will
/// send requests to the host application.
const SEND_SOCKET_FILENO: RawFd = RECV_SOCKET_FILENO + 1;

/// Runs the terminal server event loop.
///
/// This should be executed very shortly after process start; it is important
/// to minimize the number of resources acquired that could be leaked to a child
/// process through a fork/exec pair.
pub fn run_terminal_server(args: &warp_cli::TerminalServerArgs) {
    // We initialize context-independent feature flags early, as the terminal
    // server process may need to reference them. User-controlled flags are overridden
    // soon after.
    init_feature_flags();
    let event_loop = event_loop::EventLoop::new(args);
    event_loop.run()
}

/// Spawns a thread to handle fire-and-forget messages sent back from the server.
fn spawn_message_receiver_thread(socket_fd: RawFd, terminated_children: Arc<Mutex<HashSet<u32>>>) {
    std::thread::spawn(move || {
        loop {
            let message = match protocol::receive_message(socket_fd) {
                Ok(msg) => msg,
                Err(e) => {
                    log::error!("Failed to receive message from terminal server: {e:?}");
                    break;
                }
            };
            match message {
                Some(api::Message::WriteLogRequest {
                    level,
                    target,
                    message,
                }) => {
                    logging::handle_write_log_request(level, target, message);
                }
                Some(api::Message::ChildrenTerminatedRequest { pids }) => {
                    terminated_children.lock().extend(pids);
                    // Send ourselves a SIGCHLD signal to notify the event loop threads that
                    // they should check to see if their associated shell process has
                    // terminated.
                    if let Err(err) =
                        nix::sys::signal::kill(nix::unistd::getpid(), nix::sys::signal::SIGCHLD)
                    {
                        log::error!("Failed to send SIGCHLD to self: {err:?}");
                    }
                }
                Some(_) => {
                    log::error!(
                        "host application received unexpected message from terminal server"
                    );
                }
                None => {
                    // The server process exited, so we can just let this thread terminate.
                    break;
                }
            }
        }
    });
}

/// A "terminal server" subprocess which spawns and manages ptys.
///
/// This provides a limited API for creation and lifecycle management for ptys,
/// implemented via a simple communication protocol built atop Unix domain
/// sockets.
///
/// Unlike a standard pipe, Unix domain sockets support sending file descriptors
/// between processes, enabling the Warp application process to communicate
/// directly with the pty (grandchild) process - this is much more performant
/// than communicating with the grandchild via the terminal server as an
/// intermediary.
pub(super) struct TerminalServer {
    /// The terminal server child process.
    server: std::process::Child,

    /// A client for communicating with the terminal server process.
    client: Arc<TerminalServerClient>,
}

impl TerminalServer {
    /// Spawns a new terminal server subprocess and returns a structure that
    /// provides access to a client API and, when dropped, shuts down the
    /// server.
    pub fn new() -> Result<Self> {
        log::info!("Spawning terminal server process...");

        // Create a pair of Unix domain sockets which we can use to send
        // messages and file descriptors between the client and server.
        let (server_recv_fd, client_send_fd) = socket::socketpair(
            socket::AddressFamily::Unix,
            socket::SockType::Stream,
            None,
            socket::SockFlag::empty(),
        )
        .context("Failed to create Unix domain socket pair")?;

        let (server_send_fd, client_recv_fd) = socket::socketpair(
            socket::AddressFamily::Unix,
            socket::SockType::Stream,
            None,
            socket::SockFlag::empty(),
        )
        .context("Failed to create Unix domain socket pair")?;

        // Create a concurrency-safe set to track the list of terminated
        // children that the terminal server has notified us about but
        // the pty event loops haven't yet processed.
        let terminated_children = Arc::new(Mutex::new(HashSet::new()));

        // Spawn the message receiver background thread.
        spawn_message_receiver_thread(client_recv_fd, terminated_children.clone());

        unsafe {
            // Make sure the client file descriptor is closed when the server process
            // is executed (as it only needs the server side of the socket pair).
            cvt(libc::fcntl(client_send_fd, libc::F_SETFD, libc::FD_CLOEXEC))
                .context("Failed to set CLOEXEC flag on client fd before fork")?;
            cvt(libc::fcntl(client_recv_fd, libc::F_SETFD, libc::FD_CLOEXEC))
                .context("Failed to set CLOEXEC flag on client fd before fork")?;

            let program = std::env::current_exe()
                .context("Failed to determine path to current executable")?;
            let server = Command::new(program)
                .pre_exec(move || {
                    // Make sure the server file descriptor is at the index
                    // we expect.
                    if server_recv_fd != RECV_SOCKET_FILENO {
                        cvt(libc::dup2(server_recv_fd, RECV_SOCKET_FILENO))?;
                        cvt(libc::close(server_recv_fd))?;
                    }
                    if server_send_fd != SEND_SOCKET_FILENO {
                        cvt(libc::dup2(server_send_fd, SEND_SOCKET_FILENO))?;
                        cvt(libc::close(server_send_fd))?;
                    }
                    Ok(())
                })
                .arg(warp_cli::terminal_server_subcommand())
                // Tell the terminal server process what process ID it should
                // expect its parent to have.  This allows it to terminate
                // itself when it detects its parent process to have changed.
                .arg(warp_cli::parent_flag())
                .spawn()
                .context("Failed to spawn terminal server process")?;

            // Close the server file descriptor, as we won't use it here (on the client
            // side).
            cvt(libc::close(server_recv_fd))
                .context("Failed to close server file descriptor in client process")?;
            cvt(libc::close(server_send_fd))
                .context("Failed to close server file descriptor in client process")?;

            let client = Arc::new(TerminalServerClient::new(
                OwnedFd::from_raw_fd(client_send_fd),
                terminated_children,
            ));

            Ok(Self { server, client })
        }
    }

    /// Returns an API client for interacting with the terminal server.
    pub fn client(&self) -> &Arc<TerminalServerClient> {
        &self.client
    }
}

impl Drop for TerminalServer {
    fn drop(&mut self) {
        // Kill the server child process and wait for it to terminate.
        let _ = self.server.kill();
        let _ = self.server.wait();
    }
}

/// A handle for a pty that is owned by a terminal server forked from the
/// current process.
pub struct ServerOwnedPtyHandle {
    pub pid: u32,
    pub client: Arc<TerminalServerClient>,
}

impl PtyHandle for ServerOwnedPtyHandle {
    fn pid(&self) -> u32 {
        self.pid
    }

    fn has_process_terminated(&mut self) -> Result<bool> {
        Ok(self.client.has_child_terminated(self.pid))
    }

    fn kill(&mut self) -> Result<()> {
        self.client.kill_child(self.pid())
    }
}

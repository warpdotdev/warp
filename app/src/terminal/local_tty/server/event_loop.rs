use std::collections::HashMap;
use std::os::unix::prelude::*;
use std::process::Child;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::Result;
use itertools::Itertools;
use mio::Interest;
use parking_lot::Mutex;
use signal_hook_mio::v1_0::Signals;
use warp_cli::TerminalServerArgs;

use super::{api, logging, protocol, RECV_SOCKET_FILENO, SEND_SOCKET_FILENO};
use crate::terminal::local_tty::server::protocol::NonblockingSocketFd;
use crate::terminal::local_tty::{self};
use crate::terminal::platform;

const RECV_SOCKET_TOKEN: mio::Token = mio::Token(0);
const SIGNALS_TOKEN: mio::Token = mio::Token(1);
const CHILD_TERMINATION_GRACE_PERIOD: Duration = Duration::from_millis(500);
const CHILD_TERMINATION_POLL_INTERVAL: Duration = Duration::from_millis(10);

/// A helper structure for holding onto child processes and ensuring that
/// all children are killed when the structure is dropped.
struct Children(HashMap<u32, Child>);

impl Children {
    fn new() -> Self {
        Self(Default::default())
    }

    /// Inserts a child process into the collection.
    fn insert(&mut self, child: Child) {
        self.0.insert(child.id(), child);
    }

    /// Removes the child process with the given process ID from the collection,
    /// if it exists.
    fn remove(&mut self, pid: &u32) -> Option<Child> {
        self.0.remove(pid)
    }

    /// Checks all known children to see which have already terminated, returning
    /// process IDs for children that are no longer running.  Any child returned
    /// this way is removed from the list of children.
    fn terminated_children(&mut self) -> Vec<u32> {
        let mut terminated_children = vec![];
        let keys = self.0.keys().cloned().collect_vec();
        for k in keys {
            let Some(child) = self.0.get_mut(&k) else {
                continue;
            };
            if matches!(child.try_wait(), Ok(Some(_))) {
                self.0.remove(&k);
                terminated_children.push(k);
            }
        }
        terminated_children
    }

    /// Sends a signal to all processes in one hosted PTY process group.
    ///
    /// The group may already have exited before cleanup observes it, which is
    /// equivalent to successful termination.
    fn signal_process_group(pgid: u32, signal: nix::sys::signal::Signal) -> Result<()> {
        let process_group = nix::unistd::Pid::from_raw(pgid as i32);
        match nix::sys::signal::killpg(process_group, signal) {
            Ok(()) | Err(nix::errno::Errno::ESRCH) => Ok(()),
            Err(err) => Err(err.into()),
        }
    }

    /// Returns whether any process still exists in one hosted PTY process group.
    fn process_group_exists(pgid: u32) -> bool {
        let process_group = nix::unistd::Pid::from_raw(pgid as i32);
        match nix::sys::signal::killpg(process_group, None) {
            Ok(()) | Err(nix::errno::Errno::EPERM) => true,
            Err(nix::errno::Errno::ESRCH) => false,
            Err(err) => {
                log::warn!("Failed to inspect PTY process group {pgid}: {err:#}");
                true
            }
        }
    }

    /// Terminates one hosted PTY process group within a bounded period.
    ///
    /// PTY shell processes are process-group leaders, so their process IDs are
    /// also their process-group IDs. The direct child may exit from `SIGHUP`
    /// while descendants remain in that group, so this keeps checking group
    /// liveness throughout a grace period and sends `SIGKILL` to survivors.
    fn terminate_child(&mut self, pid: u32) -> Result<()> {
        if !self.0.contains_key(&pid) {
            log::info!(
                "Did not find child shell process with pid {pid}; assuming it has already terminated."
            );
            return Ok(());
        }
        let pgid = pid;
        Self::signal_process_group(pgid, nix::sys::signal::SIGHUP)?;
        let deadline = Instant::now() + CHILD_TERMINATION_GRACE_PERIOD;
        while Instant::now() < deadline {
            if let Some(child) = self.0.get_mut(&pid) {
                let _ = child.try_wait()?;
            }
            if !Self::process_group_exists(pgid) {
                if let Some(mut child) = self.remove(&pid) {
                    child.wait()?;
                }
                return Ok(());
            }
            thread::sleep(CHILD_TERMINATION_POLL_INTERVAL);
        }
        if Self::process_group_exists(pgid) {
            Self::signal_process_group(pgid, nix::sys::signal::SIGKILL)?;
        }
        if let Some(mut child) = self.remove(&pid) {
            child.wait()?;
        }
        Ok(())
    }

    /// Terminates all hosted PTY process groups during server shutdown.
    fn terminate_all(&mut self) {
        let pgids = self.0.keys().cloned().collect_vec();
        for pgid in &pgids {
            if let Err(err) = Self::signal_process_group(*pgid, nix::sys::signal::SIGHUP) {
                log::warn!("Failed to send SIGHUP to PTY process group {pgid}: {err:#}");
            }
        }

        let deadline = Instant::now() + CHILD_TERMINATION_GRACE_PERIOD;
        while Instant::now() < deadline {
            for child in self.0.values_mut() {
                let _ = child.try_wait();
            }
            if pgids.iter().all(|pgid| !Self::process_group_exists(*pgid)) {
                break;
            }
            thread::sleep(CHILD_TERMINATION_POLL_INTERVAL);
        }
        for pgid in pgids {
            if Self::process_group_exists(pgid) {
                if let Err(err) = Self::signal_process_group(pgid, nix::sys::signal::SIGKILL) {
                    log::warn!("Failed to send SIGKILL to PTY process group {pgid}: {err:#}");
                }
            }
        }
        for child in self.0.values_mut() {
            let _ = child.wait();
        }
        self.0.clear();
    }
}

impl std::ops::Drop for Children {
    fn drop(&mut self) {
        self.terminate_all();
    }
}

/// A structure to hold state for and manage the terminal server event loop.
pub struct EventLoop {
    /// Information about the terminal server's child processes, including
    /// handles that can be used to kill and reap them.
    children: Children,
    /// The process ID for our original parent process.  If we notice that our
    /// parent is a different process, the original one must have died, so we
    /// should exit.
    original_parent_pid: nix::unistd::Pid,
    /// A non-blocking Unix socket over which we will receive requests from
    /// the host process.
    recv_socket_fd: NonblockingSocketFd,
}

impl EventLoop {
    /// Constructs a new event loop.
    pub fn new(args: &TerminalServerArgs) -> Self {
        use nix::fcntl;

        let original_parent_pid = args
            .parent
            .pid
            .expect("terminal server process should be spawned with a --parent-pid argument");
        let original_parent_pid = nix::unistd::Pid::from_raw(original_parent_pid as _);

        // Make sure a file descriptor exists where we expect the socket file
        // descriptor to exist, and set it to close on exec (so it doesn't end up
        // in shells that we spawn).
        fcntl::fcntl(RECV_SOCKET_FILENO, fcntl::F_GETFD)
            .expect("should have valid file descriptor");
        fcntl::fcntl(
            RECV_SOCKET_FILENO,
            fcntl::F_SETFD(fcntl::FdFlag::FD_CLOEXEC),
        )
        .expect("should be able to set FD_CLOEXEC on unix socket");

        fcntl::fcntl(SEND_SOCKET_FILENO, fcntl::F_GETFD)
            .expect("should have valid file descriptor");
        fcntl::fcntl(
            SEND_SOCKET_FILENO,
            fcntl::F_SETFD(fcntl::FdFlag::FD_CLOEXEC),
        )
        .expect("should be able to set FD_CLOEXEC on unix socket");

        let recv_socket_fd = NonblockingSocketFd::new(RECV_SOCKET_FILENO)
            .expect("should be able to make unix socket non-blocking");

        Self {
            children: Children::new(),
            original_parent_pid,
            recv_socket_fd,
        }
    }

    /// Runs the terminal server event loop.
    ///
    /// This should be executed very shortly after process start; it is important
    /// to minimize the number of resources acquired that could be leaked to a child
    /// process through a fork/exec pair.
    pub fn run(mut self) {
        let send_socket_fd = Arc::new(Mutex::new(SEND_SOCKET_FILENO));

        // Set up our custom logger - we send log entries across the send
        // socket to the host process.
        log::set_boxed_logger(Box::new(logging::RemoteLogger::new(send_socket_fd.clone())))
            .expect("should not fail to set logger");
        log::set_max_level(log::LevelFilter::Info);

        log::info!("Running terminal server...");

        // Make sure any platform-specific initialization is performed to prepare
        // the terminal server process for spawning shell processes.
        platform::init()
            .expect("should not fail to perform platform-level terminal initialization");

        let mut poll = mio::Poll::new().expect("should not fail to create mio::Poll");

        poll.registry()
            .register(
                &mut mio::unix::SourceFd(&self.recv_socket_fd.as_raw_fd()),
                RECV_SOCKET_TOKEN,
                Interest::READABLE,
            )
            .expect("should not fail to register for socket events");

        let mut signals =
            Signals::new([signal_hook::consts::SIGCHLD]).expect("error preparing signal handling");

        poll.registry()
            .register(&mut signals, SIGNALS_TOKEN, Interest::READABLE)
            .expect("should not fail to register for signal events");

        let mut events = mio::Events::with_capacity(10);

        'event_loop: loop {
            if let Err(err) = poll.poll(&mut events, None) {
                match err.kind() {
                    std::io::ErrorKind::Interrupted => continue,
                    _ => panic!("EventLoop polling error: {err:?}"),
                }
            }

            for event in &events {
                match event.token() {
                    RECV_SOCKET_TOKEN => {
                        // If the other end of the socket is closed, break out of the
                        // loop.
                        if event.is_read_closed() || event.is_write_closed() {
                            break 'event_loop;
                        }

                        // Read messages from the socket until there's nothing left
                        // to read.  If the other end of the socket is closed, break
                        // out of the loop.
                        if self.read_messages().is_none() {
                            break 'event_loop;
                        }
                    }
                    SIGNALS_TOKEN => {
                        for signal in signals.pending() {
                            if signal == signal_hook::consts::SIGCHLD {
                                let terminated_children_pids = self.children.terminated_children();
                                if let Err(err) = protocol::send_message(
                                    *send_socket_fd.lock(),
                                    api::Message::ChildrenTerminatedRequest {
                                        pids: terminated_children_pids,
                                    },
                                    Option::<RawFd>::None,
                                ) {
                                    log::error!("Failed to notify host process about terminated children: {err:#}");
                                }
                            }
                        }
                    }
                    _ => log::error!("Received event with unexpected token!"),
                }
            }

            // If we've been reparented to a different process, stop running -
            // the original host Warp process died and we're now an orphan.
            if nix::unistd::Pid::parent() != self.original_parent_pid {
                log::info!("Detected a change in parent process; shutting down terminal server.");
                break 'event_loop;
            }
        }
    }

    /// Read all available messages off of the socket and process them.
    /// Returns None if we will not be able to communicate over the socket
    /// anymore and should shut down the server.
    fn read_messages(&mut self) -> Option<()> {
        loop {
            let result = match protocol::try_receive_message(self.recv_socket_fd) {
                Ok(result) => result,
                Err(err) => {
                    log::error!("Encountered unexpected error receiving message from host process: {err:#}.");
                    log::info!("Shutting down terminal server...");
                    return None;
                }
            };
            let message = match result {
                protocol::TryReceiveMessageResult::Success(message) => message,
                protocol::TryReceiveMessageResult::WouldBlock => return Some(()),
                protocol::TryReceiveMessageResult::SocketClosed => {
                    // The socket was closed on the other end, so we no longer need
                    // to listen for messages.
                    log::info!("Socket closed; shutting down terminal server...");
                    return None;
                }
            };
            match message {
                api::Message::SpawnShellRequest { mut options } => {
                    // No need to close all open file descriptors when spawning a pty from
                    // the terminal server, as it spawns ptys cleanly.
                    options.close_fds = false;
                    let result = match local_tty::spawn(options) {
                        Ok(pty_spawn_info) => {
                            let child = pty_spawn_info.child;
                            log::info!("Successfully spawned tty with pid: {}", child.id());
                            self.children.insert(child);
                            Ok(pty_spawn_info.result)
                        }
                        Err(err) => {
                            log::error!("Failed to spawn tty from server: {err:?}");
                            Err(err)
                        }
                    };

                    let leader_fd = result.as_ref().ok().map(|result| result.leader_fd);

                    // Send back a message indicating whether we succeeded or failed
                    // to spawn a shell.
                    if let Err(err) = protocol::send_message(
                        RECV_SOCKET_FILENO,
                        api::Message::SpawnShellResponse {
                            spawn_result: result.into(),
                        },
                        leader_fd,
                    ) {
                        log::error!("Encountered unexpected error sending message to host process: {err:#}.");
                        log::info!("Shutting down terminal server...");
                        return None;
                    };

                    // Close the leader file descriptor now that the host
                    // process is holding a copy of it.
                    if let Some(leader_fd) = leader_fd {
                        if let Err(err) = nix::unistd::close(leader_fd) {
                            log::error!("Failed to close leader fd after sending it back to host process: {err:?}");
                        }
                    }
                }
                api::Message::KillChildRequest { pid } => {
                    let error_msg = self.children.terminate_child(pid).err().map(|err| {
                        format!("Failed to terminate child shell process {pid}: {err:#}")
                    });
                    if let Err(err) = protocol::send_message(
                        RECV_SOCKET_FILENO,
                        api::Message::KillChildResponse { error_msg },
                        Option::<RawFd>::None,
                    ) {
                        log::error!("Encountered unexpected error sending message to host process: {err:#}.");
                        log::info!("Shutting down terminal server...");
                        return None;
                    };
                }
                api::Message::ShutdownRequest => {
                    self.children.terminate_all();
                    if let Err(err) = protocol::send_message(
                        RECV_SOCKET_FILENO,
                        api::Message::ShutdownResponse,
                        Option::<RawFd>::None,
                    ) {
                        log::error!("Encountered unexpected error sending terminal server shutdown response: {err:#}.");
                    }
                    return None;
                }
                api::Message::SpawnShellResponse { .. } => {
                    log::error!("Terminal server received unexpected SpawnShellResponse message!");
                }
                api::Message::KillChildResponse { .. } => {
                    log::error!("Terminal server received unexpected KillChildResponse message!");
                }
                api::Message::ShutdownResponse => {
                    log::error!("Terminal server received unexpected ShutdownResponse message!");
                }
                api::Message::WriteLogRequest { .. } => {
                    log::error!("Terminal server received unexpected WriteLogRequest message!");
                }
                api::Message::ChildrenTerminatedRequest { .. } => {
                    log::error!(
                        "Terminal server received unexpected ChildrenTerminatedRequest message!"
                    );
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "event_loop_tests.rs"]
mod tests;

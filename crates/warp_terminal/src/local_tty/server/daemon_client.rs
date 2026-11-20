use std::collections::HashMap;
use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Weak, mpsc};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use instant::Instant;
use parking_lot::Mutex;
use warp_errors::report_error;

use super::api::{
    AttachInfo, ClientMessage, DAEMON_PROTOCOL_VERSION, DaemonEvent, DaemonMessage, DaemonRequest,
    DaemonResponse, PersistentSessionId, SessionMetadata, SessionSummary, SpawnEnvironment,
};
use super::daemon::{DaemonPaths, check_peer_uid};
use super::protocol;
use crate::event::ObservedExitStatus;
use crate::local_tty::PtyOptions;
use crate::local_tty::spawner::PtyHandle;
use crate::local_tty::unix::close_fds_upper_bound;

const SPAWN_RETRY_INTERVAL: Duration = Duration::from_millis(50);
const SPAWN_TIMEOUT: Duration = Duration::from_secs(3);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(3);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const PRUNE_TIMEOUT: Duration = Duration::from_secs(60);
const DETACH_TIMEOUT: Duration = Duration::from_secs(2);

type PendingResponse = (DaemonResponse, Vec<OwnedFd>);

/// State shared between the client and its reader thread.
struct Shared {
    socket: OwnedFd,
    write_lock: Mutex<()>,
    next_request_id: AtomicU64,
    /// Senders for in-flight requests. `None` once the daemon connection is
    /// gone, so no new request can wait forever.
    pending: Mutex<Option<HashMap<u64, mpsc::Sender<PendingResponse>>>>,
    detached: AtomicBool,
    /// Sessions attached through this client, by pid.
    attached: Mutex<HashMap<PersistentSessionId, u32>>,
    /// Exit statuses not yet consumed by the pty event loops.
    terminated: Mutex<HashMap<u32, ObservedExitStatus>>,
}

impl Shared {
    fn is_alive(&self) -> bool {
        self.pending.lock().is_some()
    }

    fn request(&self, request: DaemonRequest, timeout: Duration) -> Result<PendingResponse> {
        let request_id = self.next_request_id.fetch_add(1, Ordering::Relaxed);
        let message = ClientMessage::Request {
            request_id,
            request,
        };
        let size = bincode::serialized_size(&message).context("Failed to size request")?;
        if size > protocol::MAX_CLIENT_FRAME_SIZE as u64 {
            bail!("terminal daemon request is too large ({size} bytes)");
        }
        let (sender, receiver) = mpsc::channel();
        match self.pending.lock().as_mut() {
            Some(pending) => {
                pending.insert(request_id, sender);
            }
            None => bail!("terminal daemon connection is closed"),
        }
        let sent = {
            let _guard = self.write_lock.lock();
            protocol::send_frame(self.socket.as_raw_fd(), &message, &[])
        };
        if let Err(err) = sent {
            if let Some(pending) = self.pending.lock().as_mut() {
                pending.remove(&request_id);
            }
            return Err(err.context("Failed to send request to terminal daemon"));
        }
        match receiver.recv_timeout(timeout) {
            Ok(response) => Ok(response),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if let Some(pending) = self.pending.lock().as_mut() {
                    pending.remove(&request_id);
                }
                bail!("terminal daemon did not respond in time")
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                bail!("terminal daemon connection is closed")
            }
        }
    }

    fn expect_done(&self, request: DaemonRequest, timeout: Duration) -> Result<()> {
        match self.request(request, timeout)?.0 {
            DaemonResponse::Done => Ok(()),
            DaemonResponse::Error(err) => bail!("terminal daemon error: {err}"),
            other => bail!("unexpected terminal daemon response: {other:?}"),
        }
    }

    fn kill_session(&self, id: PersistentSessionId) -> Result<()> {
        if self.detached.load(Ordering::SeqCst) || !self.is_alive() {
            return Ok(());
        }
        if !self.attached.lock().contains_key(&id) {
            return Ok(());
        }
        let result = self.expect_done(DaemonRequest::Kill { session_id: id }, REQUEST_TIMEOUT);
        self.attached.lock().remove(&id);
        result
    }

    fn record_exit(&self, pid: u32, status: ObservedExitStatus) {
        self.terminated.lock().insert(pid, status);
        raise_sigchld();
    }

    /// Updates bookkeeping for an `Attached` response. Runs on the reader
    /// thread so it is ordered with respect to `SessionExited` events.
    fn on_attached(&self, info: &AttachInfo) {
        match info.exit_status {
            Some(status) => self.record_exit(info.pid, status),
            None => {
                self.terminated.lock().remove(&info.pid);
                self.attached.lock().insert(info.session_id, info.pid);
            }
        }
    }

    fn on_disconnected(&self) {
        let pending = self.pending.lock().take();
        drop(pending);
        let pids: Vec<u32> = self.attached.lock().drain().map(|(_, pid)| pid).collect();
        {
            let mut terminated = self.terminated.lock();
            for pid in pids {
                terminated
                    .entry(pid)
                    .or_insert(ObservedExitStatus::Unavailable);
            }
        }
        raise_sigchld();
    }
}

/// Returns whether reading the peer credentials failed because the daemon
/// already closed the connection (macOS reports `EINVAL` then).
fn peer_is_gone(err: &anyhow::Error) -> bool {
    protocol::is_disconnect(err)
        || err.chain().any(|cause| {
            cause
                .downcast_ref::<std::io::Error>()
                .is_some_and(|io| io.raw_os_error() == Some(libc::EINVAL))
        })
}

/// Sends SIGCHLD to the current process so pty event loops check whether
/// their child exited.
fn raise_sigchld() {
    if let Err(err) = nix::sys::signal::kill(nix::unistd::getpid(), nix::sys::signal::SIGCHLD) {
        report_error!(anyhow::Error::new(err).context("Failed to send SIGCHLD to self"));
    }
}

fn run_reader(shared: Arc<Shared>) {
    loop {
        match protocol::receive_frame::<DaemonMessage>(shared.socket.as_raw_fd()) {
            Ok(Some((
                DaemonMessage::Response {
                    request_id,
                    response,
                },
                fds,
            ))) => {
                if let DaemonResponse::Attached(info) = &response {
                    shared.on_attached(info);
                }
                let sender = shared
                    .pending
                    .lock()
                    .as_mut()
                    .and_then(|pending| pending.remove(&request_id));
                if let Some(sender) = sender {
                    let _ = sender.send((response, fds));
                }
            }
            Ok(Some((
                DaemonMessage::Event(DaemonEvent::SessionExited {
                    session_id,
                    pid,
                    status,
                }),
                _,
            ))) => {
                shared.attached.lock().remove(&session_id);
                shared.record_exit(pid, status);
            }
            Ok(Some((message, _))) => {
                log::warn!("Unexpected message from terminal daemon: {message:?}");
            }
            Ok(None) => break,
            Err(err) => {
                log::warn!("Failed to read from terminal daemon: {err:#}");
                break;
            }
        }
    }
    log::info!("Terminal daemon connection closed");
    shared.on_disconnected();
}

/// A connection to the persistent terminal daemon.
pub struct DaemonClient {
    shared: Arc<Shared>,
    weak_self: Weak<DaemonClient>,
}

impl DaemonClient {
    /// Connects to the daemon, starting it if it isn't running.
    pub fn connect_or_spawn() -> anyhow::Result<Arc<Self>> {
        let paths = DaemonPaths::default_for_user();
        paths.ensure_private_dir()?;
        Self::connect_or_spawn_with(&paths.socket, spawn_daemon_process)
    }

    /// Connects to the daemon only if it is already running.
    pub fn connect_existing() -> anyhow::Result<Option<Arc<Self>>> {
        let paths = DaemonPaths::default_for_user();
        if !paths.validate_private_dir()? {
            return Ok(None);
        }
        Self::connect_at(&paths.socket)
    }

    pub(crate) fn connect_or_spawn_with(
        socket_path: &Path,
        spawn: impl FnOnce() -> Result<()>,
    ) -> Result<Arc<Self>> {
        if let Some(client) = Self::connect_at(socket_path)? {
            return Ok(client);
        }
        spawn()?;
        let deadline = Instant::now() + SPAWN_TIMEOUT;
        loop {
            if let Some(client) = Self::connect_at(socket_path)? {
                return Ok(client);
            }
            if Instant::now() >= deadline {
                bail!("terminal daemon did not start in time");
            }
            std::thread::sleep(SPAWN_RETRY_INTERVAL);
        }
    }

    /// Connects and performs the handshake. Returns `None` if no daemon is
    /// listening at `socket_path` or it went away during the handshake.
    pub(crate) fn connect_at(socket_path: &Path) -> Result<Option<Arc<Self>>> {
        let stream = match UnixStream::connect(socket_path) {
            Ok(stream) => stream,
            Err(err)
                if matches!(
                    err.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
                ) =>
            {
                return Ok(None);
            }
            Err(err) => return Err(err).context("Failed to connect to terminal daemon"),
        };
        match check_peer_uid(stream.as_raw_fd(), nix::unistd::geteuid().as_raw()) {
            Ok(()) => {}
            Err(err) if peer_is_gone(&err) => return Ok(None),
            Err(err) => {
                return Err(
                    err.context("Refusing to talk to a terminal daemon owned by another user")
                );
            }
        }
        if let Err(err) = stream.set_read_timeout(Some(HANDSHAKE_TIMEOUT)) {
            let err = anyhow::Error::new(err);
            if peer_is_gone(&err) {
                return Ok(None);
            }
            return Err(err.context("Failed to configure the terminal daemon socket"));
        }
        let hello = protocol::send_frame(
            stream.as_raw_fd(),
            &ClientMessage::Hello {
                protocol_version: DAEMON_PROTOCOL_VERSION,
                client_pid: std::process::id(),
                app_version: warp_core::channel::ChannelState::app_version().map(str::to_owned),
            },
            &[],
        );
        match hello {
            Ok(()) => {}
            Err(err) if protocol::is_disconnect(&err) => return Ok(None),
            Err(err) => return Err(err.context("Failed to send hello to terminal daemon")),
        }
        match protocol::receive_frame::<DaemonMessage>(stream.as_raw_fd()) {
            Ok(Some((DaemonMessage::HelloAck { .. }, _))) => {}
            Ok(Some((
                DaemonMessage::HelloReject {
                    protocol_version,
                    reason,
                },
                _,
            ))) => bail!("terminal daemon (protocol {protocol_version}) rejected client: {reason}"),
            Ok(Some((other, _))) => bail!("unexpected handshake reply: {other:?}"),
            Ok(None) => return Ok(None),
            Err(err) if protocol::is_disconnect(&err) => return Ok(None),
            Err(err) => return Err(err.context("Terminal daemon handshake failed")),
        };
        if let Err(err) = stream.set_read_timeout(None) {
            let err = anyhow::Error::new(err);
            if peer_is_gone(&err) {
                return Ok(None);
            }
            return Err(err.context("Failed to configure the terminal daemon socket"));
        }

        let shared = Arc::new(Shared {
            socket: OwnedFd::from(stream),
            write_lock: Mutex::new(()),
            next_request_id: AtomicU64::new(1),
            pending: Mutex::new(Some(HashMap::new())),
            detached: AtomicBool::new(false),
            attached: Mutex::new(HashMap::new()),
            terminated: Mutex::new(HashMap::new()),
        });
        let reader_shared = shared.clone();
        std::thread::Builder::new()
            .name("terminal-daemon-client".to_owned())
            .spawn(move || run_reader(reader_shared))
            .context("Failed to spawn terminal daemon reader thread")?;
        Ok(Some(Arc::new_cyclic(|weak_self| Self {
            shared,
            weak_self: weak_self.clone(),
        })))
    }

    pub fn list(&self) -> anyhow::Result<Vec<SessionSummary>> {
        match self.shared.request(DaemonRequest::List, REQUEST_TIMEOUT)?.0 {
            DaemonResponse::Sessions(sessions) => Ok(sessions),
            DaemonResponse::Error(err) => bail!("terminal daemon error: {err}"),
            other => bail!("unexpected terminal daemon response: {other:?}"),
        }
    }

    pub fn spawn(
        &self,
        id: PersistentSessionId,
        metadata: SessionMetadata,
        options: PtyOptions,
    ) -> anyhow::Result<AttachedSession> {
        self.spawn_with_environment(id, metadata, options, SpawnEnvironment::capture())
    }

    /// Spawns a session whose shell starts from `environment` instead of the
    /// current process environment.
    pub(crate) fn spawn_with_environment(
        &self,
        id: PersistentSessionId,
        metadata: SessionMetadata,
        options: PtyOptions,
        environment: SpawnEnvironment,
    ) -> anyhow::Result<AttachedSession> {
        let response = self.shared.request(
            DaemonRequest::Spawn {
                session_id: id,
                metadata,
                options: Box::new(options),
                environment: Some(environment),
            },
            REQUEST_TIMEOUT,
        )?;
        self.attached_session(response)
    }

    pub fn attach(&self, id: PersistentSessionId) -> anyhow::Result<AttachedSession> {
        let response = self
            .shared
            .request(DaemonRequest::Attach { session_id: id }, REQUEST_TIMEOUT)?;
        self.attached_session(response)
    }

    fn attached_session(&self, (response, fds): PendingResponse) -> Result<AttachedSession> {
        let info = match response {
            DaemonResponse::Attached(info) => info,
            DaemonResponse::Error(err) => bail!("terminal daemon error: {err}"),
            other => bail!("unexpected terminal daemon response: {other:?}"),
        };
        let mut fds = fds.into_iter();
        let data = fds
            .next()
            .ok_or_else(|| anyhow!("terminal daemon did not send a data socket"))?;
        let leader = fds.next();
        if info.exit_status.is_none() && leader.is_none() {
            bail!("terminal daemon did not send the pty leader");
        }

        let client = self
            .weak_self
            .upgrade()
            .ok_or_else(|| anyhow!("terminal daemon client was dropped"))?;
        let handle = Box::new(DaemonPtyHandle {
            session_id: info.session_id,
            pid: info.pid,
            client,
        });
        Ok(AttachedSession {
            info,
            data,
            leader,
            handle,
        })
    }

    pub fn prune(&self, keep: Vec<PersistentSessionId>) -> anyhow::Result<()> {
        self.shared
            .expect_done(DaemonRequest::Prune { keep }, PRUNE_TIMEOUT)
    }

    /// Detaches every session; afterwards `kill()` on their handles is a
    /// no-op.
    pub fn detach_all(&self) {
        self.shared.detached.store(true, Ordering::SeqCst);
        self.shared.attached.lock().clear();
        if self.shared.is_alive()
            && let Err(err) = self
                .shared
                .expect_done(DaemonRequest::DetachAll, DETACH_TIMEOUT)
        {
            log::warn!("Failed to detach terminal daemon sessions: {err:#}");
        }
    }

    /// Kills every session attached through this client.
    pub fn kill_all_attached(&self) {
        let ids: Vec<_> = self.shared.attached.lock().keys().copied().collect();
        for id in ids {
            if let Err(err) = self.shared.kill_session(id) {
                log::warn!("Failed to kill terminal daemon session: {err:#}");
            }
        }
    }

    pub fn is_alive(&self) -> bool {
        self.shared.is_alive()
    }
}

impl Drop for DaemonClient {
    fn drop(&mut self) {
        let _ = nix::sys::socket::shutdown(
            self.shared.socket.as_raw_fd(),
            nix::sys::socket::Shutdown::Both,
        );
    }
}

/// Starts `<current exe> terminal-server --daemon` and waits for the
/// short-lived launcher process, which exits once the daemon has forked.
fn spawn_daemon_process() -> Result<()> {
    let program = std::env::current_exe().context("Failed to locate the current executable")?;
    spawn_daemon_command(&program)
}

fn spawn_daemon_command(program: &PathBuf) -> Result<()> {
    #[cfg(target_os = "macos")]
    if let Some(set_disclaim) = spawn_macos::set_disclaim_fn() {
        let subcommand = warp_cli::terminal_server_subcommand();
        let status = spawn_macos::spawn_disclaimed_and_wait(
            program,
            &[
                std::ffi::OsStr::new(&subcommand),
                std::ffi::OsStr::new("--daemon"),
            ],
            set_disclaim,
        )
        .context("Failed to spawn terminal daemon")?;
        if !status.success() {
            bail!("terminal daemon launcher exited with {status}");
        }
        return Ok(());
    }
    let mut command = command::blocking::Command::new(program);
    command
        .arg(warp_cli::terminal_server_subcommand())
        .arg("--daemon")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let fd_limit = close_fds_upper_bound();
    // SAFETY: the closure only calls async-signal-safe functions.
    unsafe {
        command.pre_exec(move || {
            for fd in 3..fd_limit {
                libc::close(fd);
            }
            Ok(())
        });
    }
    let mut child = command.spawn().context("Failed to spawn terminal daemon")?;
    let status = child.wait().context("Failed to wait for terminal daemon")?;
    if !status.success() {
        bail!("terminal daemon launcher exited with {status}");
    }
    Ok(())
}

pub struct AttachedSession {
    pub info: AttachInfo,
    pub data: OwnedFd,
    pub leader: Option<OwnedFd>,
    pub handle: Box<dyn PtyHandle>,
}

/// A handle for a pty owned by the terminal daemon.
pub struct DaemonPtyHandle {
    session_id: PersistentSessionId,
    pid: u32,
    client: Arc<DaemonClient>,
}

impl PtyHandle for DaemonPtyHandle {
    fn pid(&self) -> u32 {
        self.pid
    }

    fn process_termination_status(&mut self) -> Result<Option<ObservedExitStatus>> {
        Ok(self.client.shared.terminated.lock().remove(&self.pid))
    }

    fn kill(&mut self) -> Result<()> {
        self.client.shared.kill_session(self.session_id)
    }
}

#[cfg(target_os = "macos")]
#[path = "daemon_spawn_macos.rs"]
mod spawn_macos;

#[cfg(test)]
#[path = "daemon_client_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "daemon_e2e_tests.rs"]
mod e2e_tests;

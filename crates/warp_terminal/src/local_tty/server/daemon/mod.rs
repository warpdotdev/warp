//! The persistent terminal daemon.
//!
//! A per-user, per-protocol-version process that owns PTYs so shells outlive
//! the app. It always reads every PTY into a ring buffer, and forwards bytes to
//! the single attached client over a per-session data socketpair.

mod listener;
mod process;
mod ring;
mod scanner;
mod session;

use std::collections::{HashMap, HashSet};
use std::fs::{File, OpenOptions};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use instant::Instant;
use mio::Interest;
use mio::net::{UnixListener, UnixStream};
use nix::sys::socket;
use signal_hook_mio::v1_0::Signals;

pub(crate) use self::listener::{DaemonPaths, check_peer_uid};
use self::listener::{InstanceLock, bind_socket, peer_uid};
use self::process::harden_daemon_process;
use self::session::{Attachment, PumpOutcome, Session, observed_exit_status};
use super::api::{
    AttachInfo, ClientMessage, DAEMON_PROTOCOL_VERSION, DaemonEvent, DaemonMessage, DaemonRequest,
    DaemonResponse, PersistentSessionId, RING_CAPACITY, SessionMetadata, SpawnEnvironment,
};
use super::protocol::{
    self, FillResult, FlushResult, FrameReader, FrameWriter, MAX_CLIENT_FRAME_SIZE,
};
use crate::local_tty::{self, ChildResourceLimits, PtyOptions};

/// Overrides the idle timeout, in seconds.
pub(crate) const IDLE_TIMEOUT_ENV: &str = "WARP_TERMINAL_DAEMON_IDLE_SECS";
const DEFAULT_IDLE_TIMEOUT: Duration = Duration::from_secs(60);
const EXITED_SESSION_TTL: Duration = Duration::from_secs(24 * 60 * 60);
/// Connections that do not say `Hello` within this time are closed.
const HELLO_TIMEOUT: Duration = Duration::from_secs(5);
/// The maximum number of open control connections.
const MAX_CONNECTIONS: usize = 16;
/// The maximum number of bytes queued for one control connection.
const MAX_CONNECTION_QUEUE: usize = 8 * 1024 * 1024;
/// How long to wait before accepting again after running out of fds.
const ACCEPT_BACKOFF: Duration = Duration::from_millis(100);
/// How long the daemon keeps its copies of fds sent to a client. Closing a
/// socket while it is still in flight over `SCM_RIGHTS` can get it flushed by
/// the macOS unix socket garbage collector, which the receiver then sees as
/// EOF.
const IN_FLIGHT_FD_GRACE: Duration = Duration::from_secs(5);
const TICK: Duration = Duration::from_secs(1);
const MAX_LOG_SIZE: u64 = 5 * 1024 * 1024;

const LISTENER_TOKEN: mio::Token = mio::Token(0);
const SIGNALS_TOKEN: mio::Token = mio::Token(1);
const WAKER_TOKEN: mio::Token = mio::Token(2);
const FIRST_DYNAMIC_TOKEN: usize = 3;

#[derive(Debug, Clone)]
pub(crate) struct DaemonConfig {
    pub paths: DaemonPaths,
    pub idle_timeout: Duration,
    pub exited_session_ttl: Duration,
    pub ring_capacity: usize,
    pub hello_timeout: Duration,
    pub max_connections: usize,
    /// The limits restored for spawned shells; `None` leaves them unchanged.
    pub inherited_limits: Option<ChildResourceLimits>,
    /// The log file that stdout and stderr point to, rotated by size.
    pub log_file: Option<PathBuf>,
}

impl DaemonConfig {
    pub fn new(paths: DaemonPaths) -> Self {
        let idle_timeout = std::env::var(IDLE_TIMEOUT_ENV)
            .ok()
            .and_then(|secs| secs.parse::<f64>().ok())
            .filter(|secs| secs.is_finite() && *secs >= 0.0)
            .map(Duration::from_secs_f64)
            .unwrap_or(DEFAULT_IDLE_TIMEOUT);
        Self {
            paths,
            idle_timeout,
            exited_session_ttl: EXITED_SESSION_TTL,
            ring_capacity: RING_CAPACITY,
            hello_timeout: HELLO_TIMEOUT,
            max_connections: MAX_CONNECTIONS,
            inherited_limits: None,
            log_file: None,
        }
    }
}

/// Entry point for `terminal-server --daemon`: detaches from the caller, takes
/// the instance lock, and serves until idle.
pub(crate) fn run_daemon_process(
    init_feature_flags: impl FnOnce(),
    init_platform: impl FnOnce() -> anyhow::Result<()>,
) {
    let paths = DaemonPaths::default_for_user();
    if let Err(err) = paths.ensure_private_dir() {
        eprintln!("Failed to prepare terminal daemon directory: {err:#}");
        std::process::exit(1);
    }
    if let Err(err) = daemonize(&paths.log) {
        eprintln!("Failed to daemonize terminal daemon: {err:#}");
        std::process::exit(1);
    }

    let inherited_limits = harden_daemon_process();
    let mut config = DaemonConfig::new(paths.clone());
    config.inherited_limits = inherited_limits;
    config.log_file = Some(paths.log.clone());
    let daemon = match Daemon::bind(config) {
        Ok(Some(daemon)) => daemon,
        Ok(None) => std::process::exit(0),
        Err(err) => {
            eprintln!("Failed to start terminal daemon: {err:#}");
            std::process::exit(1);
        }
    };

    if log::set_boxed_logger(Box::new(StderrLogger)).is_ok() {
        log::set_max_level(log::LevelFilter::Info);
    }
    init_feature_flags();
    if let Err(err) = init_platform() {
        log::error!("Failed to initialize platform: {err:#}");
    }
    log::info!("Terminal daemon running with pid {}", std::process::id());
    daemon.run();
    log::info!("Terminal daemon exiting");
}

/// Double-forks into a new session, ignores SIGHUP, and redirects stdio.
fn daemonize(log_path: &Path) -> Result<()> {
    // SAFETY: called before any thread is spawned; the children only continue
    // executing this single-threaded program.
    unsafe {
        match libc::fork() {
            -1 => return Err(std::io::Error::last_os_error()).context("fork failed"),
            0 => {}
            _ => libc::_exit(0),
        }
        if libc::setsid() < 0 {
            return Err(std::io::Error::last_os_error()).context("setsid failed");
        }
        libc::signal(libc::SIGHUP, libc::SIG_IGN);
        match libc::fork() {
            -1 => return Err(std::io::Error::last_os_error()).context("fork failed"),
            0 => {}
            _ => libc::_exit(0),
        }
        libc::umask(0o077);
    }
    std::env::set_current_dir("/")?;

    let dev_null = File::open("/dev/null")?;
    let truncate = std::fs::metadata(log_path).is_ok_and(|m| m.len() > MAX_LOG_SIZE);
    let mut options = OpenOptions::new();
    options.create(true).mode(0o600);
    if truncate {
        options.write(true).truncate(true);
    } else {
        options.append(true);
    }
    let log = options.open(log_path)?;
    for (fd, target) in [
        (dev_null.as_raw_fd(), libc::STDIN_FILENO),
        (log.as_raw_fd(), libc::STDOUT_FILENO),
        (log.as_raw_fd(), libc::STDERR_FILENO),
    ] {
        // SAFETY: both fds are valid.
        if unsafe { libc::dup2(fd, target) } < 0 {
            return Err(std::io::Error::last_os_error()).context("dup2 failed");
        }
    }
    Ok(())
}

/// Moves the log aside once it exceeds `max_size` and points `targets` at a
/// fresh file. Returns whether it rotated.
fn rotate_log(path: &Path, max_size: u64, targets: &[RawFd]) -> Result<bool> {
    match std::fs::metadata(path) {
        Ok(metadata) if metadata.len() <= max_size => return Ok(false),
        Ok(_) => {
            let mut old = path.as_os_str().to_owned();
            old.push(".old");
            std::fs::rename(path, &old).context("Failed to move the log aside")?;
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => return Err(err).context("Failed to inspect the log"),
    }
    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(path)
        .context("Failed to open a new log")?;
    for target in targets {
        // SAFETY: both fds are valid.
        if unsafe { libc::dup2(log.as_raw_fd(), *target) } < 0 {
            return Err(std::io::Error::last_os_error()).context("dup2 failed");
        }
    }
    Ok(true)
}

struct StderrLogger;

impl log::Log for StderrLogger {
    fn enabled(&self, _metadata: &log::Metadata) -> bool {
        true
    }

    fn log(&self, record: &log::Record) {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or_default();
        eprintln!(
            "{timestamp} {} {}: {}",
            record.level(),
            record.target(),
            record.args()
        );
    }

    fn flush(&self) {}
}

enum Source {
    Leader(PersistentSessionId),
    Data(PersistentSessionId),
}

struct Connection {
    stream: UnixStream,
    reader: FrameReader,
    writer: FrameWriter,
    said_hello: bool,
    accepted_at: Instant,
    /// Close once the outbound queue is flushed.
    closing: bool,
}

/// Serializes fd-creating and forking operations of daemons that run on test
/// threads, so a shell spawned for one test never inherits another test's
/// fds in the window before they are marked close-on-exec.
#[cfg(test)]
pub(crate) static TEST_FORK_LOCK: parking_lot::Mutex<()> = parking_lot::Mutex::new(());

#[cfg(test)]
fn fork_guard() -> Option<parking_lot::MutexGuard<'static, ()>> {
    Some(TEST_FORK_LOCK.lock())
}

#[cfg(not(test))]
fn fork_guard() -> Option<()> {
    None
}

/// Observations of the event loop that tests wait on instead of sleeping.
#[cfg(test)]
#[derive(Default)]
pub(crate) struct TestStats {
    /// How many times a session paused reading its PTY for backpressure.
    pub pauses: std::sync::atomic::AtomicU64,
    /// Since when the daemon had sessions but no client, and when it last
    /// checked that.
    pub unattended: parking_lot::Mutex<Option<(Instant, Instant)>>,
}

/// Stops a running daemon from another thread.
#[cfg(test)]
#[derive(Clone)]
pub(crate) struct StopHandle {
    stop: Arc<AtomicBool>,
    waker: Arc<mio::Waker>,
    pub stats: Arc<TestStats>,
}

#[cfg(test)]
impl StopHandle {
    pub fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = self.waker.wake();
    }
}

pub(crate) struct Daemon {
    config: DaemonConfig,
    _lock: InstanceLock,
    listener: UnixListener,
    poll: mio::Poll,
    signals: Signals,
    stop: Arc<AtomicBool>,
    #[cfg_attr(not(test), allow(dead_code))]
    waker: Arc<mio::Waker>,
    next_token: usize,
    sources: HashMap<mio::Token, Source>,
    connections: HashMap<mio::Token, Connection>,
    active: Option<mio::Token>,
    sessions: HashMap<PersistentSessionId, Session>,
    orphans: Vec<std::process::Child>,
    in_flight: Vec<InFlightFds>,
    ready: HashSet<PersistentSessionId>,
    ready_connections: HashSet<mio::Token>,
    accept_backoff_until: Option<Instant>,
    idle_since: Option<Instant>,
    #[cfg(test)]
    stats: Arc<TestStats>,
}

/// Copies of fds sent to a client, kept open for [`IN_FLIGHT_FD_GRACE`].
struct InFlightFds {
    sent_at: Instant,
    _fds: Vec<OwnedFd>,
}

impl Daemon {
    /// Takes the instance lock and binds the socket. Returns `None` if another
    /// daemon holds the lock.
    pub fn bind(config: DaemonConfig) -> Result<Option<Self>> {
        config.paths.ensure_private_dir()?;
        let Some(lock) = InstanceLock::try_acquire(&config.paths.lock)? else {
            return Ok(None);
        };
        let listener = bind_socket(&config.paths.socket)?;
        let mut listener = UnixListener::from_std(listener);
        let poll = mio::Poll::new()?;
        poll.registry()
            .register(&mut listener, LISTENER_TOKEN, Interest::READABLE)?;
        let mut signals = Signals::new([signal_hook::consts::SIGCHLD])?;
        poll.registry()
            .register(&mut signals, SIGNALS_TOKEN, Interest::READABLE)?;
        let waker = Arc::new(mio::Waker::new(poll.registry(), WAKER_TOKEN)?);
        Ok(Some(Self {
            config,
            _lock: lock,
            listener,
            poll,
            signals,
            stop: Arc::new(AtomicBool::new(false)),
            waker,
            next_token: FIRST_DYNAMIC_TOKEN,
            sources: HashMap::new(),
            connections: HashMap::new(),
            active: None,
            sessions: HashMap::new(),
            orphans: Vec::new(),
            in_flight: Vec::new(),
            ready: HashSet::new(),
            ready_connections: HashSet::new(),
            accept_backoff_until: None,
            idle_since: None,
            #[cfg(test)]
            stats: Arc::new(TestStats::default()),
        }))
    }

    #[cfg(test)]
    pub fn stop_handle(&self) -> StopHandle {
        StopHandle {
            stop: self.stop.clone(),
            waker: self.waker.clone(),
            stats: self.stats.clone(),
        }
    }

    /// Serves until idle for the configured timeout or stopped.
    pub fn run(mut self) {
        let mut events = mio::Events::with_capacity(128);
        let mut last_tick = Instant::now();
        self.idle_since = Some(Instant::now());
        loop {
            let timeout = if self.ready.is_empty() && self.ready_connections.is_empty() {
                let idle_left = self
                    .idle_since
                    .map(|since| self.config.idle_timeout.saturating_sub(since.elapsed()))
                    .unwrap_or(TICK);
                let backoff_left = self
                    .accept_backoff_until
                    .map(|until| until.saturating_duration_since(Instant::now()))
                    .unwrap_or(TICK);
                TICK.min(idle_left).min(backoff_left)
            } else {
                Duration::ZERO
            };
            if let Err(err) = self.poll.poll(&mut events, Some(timeout))
                && err.kind() != std::io::ErrorKind::Interrupted
            {
                log::error!("Daemon poll failed: {err}");
                break;
            }

            for event in events.iter() {
                match event.token() {
                    LISTENER_TOKEN => self.accept_connections(),
                    SIGNALS_TOKEN => {
                        if self.signals.pending().next().is_some() {
                            self.reap_children();
                        }
                    }
                    WAKER_TOKEN => {}
                    token if self.connections.contains_key(&token) => {
                        self.on_connection_event(token)
                    }
                    token => match self.sources.get(&token) {
                        Some(Source::Leader(id) | Source::Data(id)) => {
                            let id = *id;
                            self.pump(id);
                        }
                        None => {}
                    },
                }
            }

            for id in std::mem::take(&mut self.ready) {
                self.pump(id);
            }
            for token in std::mem::take(&mut self.ready_connections) {
                self.on_connection_event(token);
            }
            if self
                .accept_backoff_until
                .is_some_and(|until| Instant::now() >= until)
            {
                self.accept_backoff_until = None;
                self.accept_connections();
            }

            if self.stop.load(Ordering::SeqCst) {
                break;
            }

            if last_tick.elapsed() >= TICK {
                last_tick = Instant::now();
                self.reap_children();
                self.expire_exited_sessions();
                self.in_flight
                    .retain(|entry| entry.sent_at.elapsed() < IN_FLIGHT_FD_GRACE);
                self.close_silent_connections();
                if let Some(log) = &self.config.log_file
                    && let Err(err) = rotate_log(
                        log,
                        MAX_LOG_SIZE,
                        &[libc::STDOUT_FILENO, libc::STDERR_FILENO],
                    )
                {
                    log::warn!("Failed to rotate the log: {err:#}");
                }
            }

            #[cfg(test)]
            {
                let mut unattended = self.stats.unattended.lock();
                if self.connections.is_empty() && !self.sessions.is_empty() {
                    let now = Instant::now();
                    let since = unattended.map_or(now, |(since, _)| since);
                    *unattended = Some((since, now));
                } else {
                    *unattended = None;
                }
            }

            if self.sessions.is_empty() && self.connections.is_empty() {
                let since = *self.idle_since.get_or_insert_with(Instant::now);
                if since.elapsed() >= self.config.idle_timeout {
                    log::info!("Daemon idle; shutting down");
                    let _ = std::fs::remove_file(&self.config.paths.socket);
                    break;
                }
            } else {
                self.idle_since = None;
            }
        }
    }

    fn allocate_token(&mut self) -> mio::Token {
        let token = mio::Token(self.next_token);
        self.next_token += 1;
        token
    }

    /// Accepts every pending connection. The listener is edge-triggered, so
    /// on a transient error (such as running out of fds) accepting is retried
    /// after [`ACCEPT_BACKOFF`] instead of waiting for the next connection.
    fn accept_connections(&mut self) {
        loop {
            match self.listener.accept() {
                Ok((mut stream, _)) => {
                    let our_uid = nix::unistd::geteuid().as_raw();
                    match peer_uid(stream.as_raw_fd()) {
                        Ok(uid) if uid == our_uid => {}
                        _ => {
                            log::warn!("Rejected connection from a different user");
                            continue;
                        }
                    }
                    if self.connections.len() >= self.config.max_connections {
                        log::warn!("Rejected connection: too many open connections");
                        continue;
                    }
                    let token = self.allocate_token();
                    if let Err(err) = self.poll.registry().register(
                        &mut stream,
                        token,
                        Interest::READABLE | Interest::WRITABLE,
                    ) {
                        log::warn!("Failed to register connection: {err}");
                        continue;
                    }
                    self.connections.insert(
                        token,
                        Connection {
                            stream,
                            reader: FrameReader::with_max_frame_size(MAX_CLIENT_FRAME_SIZE),
                            writer: FrameWriter::new(MAX_CONNECTION_QUEUE),
                            said_hello: false,
                            accepted_at: Instant::now(),
                            closing: false,
                        },
                    );
                    // Data may have arrived before registration.
                    self.on_connection_event(token);
                }
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => return,
                Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(err) if err.kind() == std::io::ErrorKind::ConnectionAborted => continue,
                Err(err) => {
                    log::warn!("Failed to accept connection: {err}; retrying shortly");
                    self.accept_backoff_until = Some(Instant::now() + ACCEPT_BACKOFF);
                    return;
                }
            }
        }
    }

    /// Closes connections that did not say `Hello` in time.
    fn close_silent_connections(&mut self) {
        let timeout = self.config.hello_timeout;
        let silent: Vec<_> = self
            .connections
            .iter()
            .filter(|(_, connection)| {
                !connection.said_hello && connection.accepted_at.elapsed() >= timeout
            })
            .map(|(token, _)| *token)
            .collect();
        for token in silent {
            log::warn!("Closing a connection that did not say hello in time");
            self.close_connection(token);
        }
    }

    /// Writes queued frames and reads new ones. Returns whether the connection
    /// is still open.
    fn flush_connection(&mut self, token: mio::Token) -> bool {
        let Some(connection) = self.connections.get_mut(&token) else {
            return false;
        };
        let mut sent_fds = Vec::new();
        let result = connection
            .writer
            .flush(connection.stream.as_raw_fd(), &mut sent_fds);
        let closing = connection.closing && connection.writer.is_empty();
        if !sent_fds.is_empty() {
            self.in_flight.push(InFlightFds {
                sent_at: Instant::now(),
                _fds: sent_fds,
            });
        }
        match result {
            Ok(_) if closing => {
                self.close_connection(token);
                false
            }
            Ok(FlushResult::Done | FlushResult::WouldBlock) => true,
            Err(err) => {
                log::warn!("Failed to send message to client: {err:#}");
                self.close_connection(token);
                false
            }
        }
    }

    fn on_connection_event(&mut self, token: mio::Token) {
        if !self.flush_connection(token) {
            return;
        }
        let Some(connection) = self.connections.get_mut(&token) else {
            return;
        };
        if connection.closing {
            return;
        }
        let fill = connection.reader.fill(connection.stream.as_raw_fd());
        let mut messages = Vec::new();
        let mut broken = false;
        loop {
            match connection.reader.next_frame::<ClientMessage>() {
                Ok(Some((message, _fds))) => messages.push(message),
                Ok(None) => break,
                Err(err) => {
                    log::warn!("Received malformed frame: {err:#}");
                    broken = true;
                    break;
                }
            }
        }
        for message in messages {
            if self
                .connections
                .get(&token)
                .is_none_or(|connection| connection.closing)
            {
                return;
            }
            self.handle_message(token, message);
        }
        match fill {
            _ if broken => self.close_connection(token),
            Ok(FillResult::WouldBlock) => {}
            Ok(FillResult::Budget) => {
                self.ready_connections.insert(token);
            }
            Ok(FillResult::Closed) => self.close_connection(token),
            Err(err) => {
                log::warn!("Failed to read from client: {err:#}");
                self.close_connection(token);
            }
        }
    }

    fn close_connection(&mut self, token: mio::Token) {
        let Some(mut connection) = self.connections.remove(&token) else {
            return;
        };
        let _ = self.poll.registry().deregister(&mut connection.stream);
        self.ready_connections.remove(&token);
        if self.active == Some(token) {
            self.active = None;
            self.detach_all();
        }
    }

    /// Queues `message` for the client and writes as much as the socket
    /// accepts without blocking; the rest is written when it becomes
    /// writable. A client whose queue overflows is dropped.
    fn send(&mut self, token: mio::Token, message: &DaemonMessage, fds: Vec<OwnedFd>) {
        let Some(connection) = self.connections.get_mut(&token) else {
            return;
        };
        if let Err(err) = connection.writer.push(message, fds) {
            log::warn!("Failed to queue message for client: {err:#}");
            self.close_connection(token);
            return;
        }
        self.flush_connection(token);
    }

    /// Sends `message` and closes the connection once it is written.
    fn send_and_close(&mut self, token: mio::Token, message: &DaemonMessage) {
        if let Some(connection) = self.connections.get_mut(&token) {
            connection.closing = true;
        }
        self.send(token, message, Vec::new());
    }

    fn handle_message(&mut self, token: mio::Token, message: ClientMessage) {
        let said_hello = self
            .connections
            .get(&token)
            .is_some_and(|connection| connection.said_hello);
        match message {
            ClientMessage::Hello {
                protocol_version,
                client_pid,
                ..
            } => {
                if said_hello {
                    self.close_connection(token);
                    return;
                }
                if protocol_version != DAEMON_PROTOCOL_VERSION {
                    log::warn!(
                        "Rejecting client {client_pid} with protocol version {protocol_version}"
                    );
                    self.send_and_close(
                        token,
                        &DaemonMessage::HelloReject {
                            protocol_version: DAEMON_PROTOCOL_VERSION,
                            reason: format!(
                                "unsupported protocol version {protocol_version}; expected {DAEMON_PROTOCOL_VERSION}"
                            ),
                        },
                    );
                    return;
                }
                if let Some(previous) = self.active.filter(|previous| *previous != token) {
                    log::info!("New client {client_pid} replaces the attached client");
                    self.close_connection(previous);
                }
                if let Some(connection) = self.connections.get_mut(&token) {
                    connection.said_hello = true;
                }
                self.active = Some(token);
                self.send(
                    token,
                    &DaemonMessage::HelloAck {
                        protocol_version: DAEMON_PROTOCOL_VERSION,
                        daemon_pid: std::process::id(),
                    },
                    Vec::new(),
                );
            }
            ClientMessage::Request { .. } if !said_hello => self.close_connection(token),
            ClientMessage::Request {
                request_id,
                request,
            } => {
                let (response, fds) = self.handle_request(request);
                self.send(
                    token,
                    &DaemonMessage::Response {
                        request_id,
                        response,
                    },
                    fds,
                );
            }
        }
    }

    fn handle_request(&mut self, request: DaemonRequest) -> (DaemonResponse, Vec<OwnedFd>) {
        match request {
            DaemonRequest::List => {
                let sessions = self.sessions.values().map(Session::summary).collect();
                (DaemonResponse::Sessions(sessions), Vec::new())
            }
            DaemonRequest::Spawn {
                session_id,
                metadata,
                options,
                environment,
            } => match self.spawn_session(session_id, metadata, *options, environment) {
                Ok(()) => self.attach_session(session_id, false),
                Err(err) => (DaemonResponse::Error(format!("{err:#}")), Vec::new()),
            },
            DaemonRequest::Attach { session_id } => self.attach_session(session_id, true),
            DaemonRequest::Kill { session_id } => {
                self.kill_session(session_id);
                (DaemonResponse::Done, Vec::new())
            }
            DaemonRequest::Prune { keep } => {
                let keep: HashSet<_> = keep.into_iter().collect();
                let doomed: Vec<_> = self
                    .sessions
                    .values()
                    .filter(|session| session.attachment.is_none() && !keep.contains(&session.id))
                    .map(|session| session.id)
                    .collect();
                for id in doomed {
                    self.kill_session(id);
                }
                (DaemonResponse::Done, Vec::new())
            }
            DaemonRequest::DetachAll => {
                self.detach_all();
                (DaemonResponse::Done, Vec::new())
            }
        }
    }

    fn spawn_session(
        &mut self,
        id: PersistentSessionId,
        metadata: SessionMetadata,
        mut options: PtyOptions,
        environment: Option<SpawnEnvironment>,
    ) -> Result<()> {
        if self.sessions.contains_key(&id) {
            log::info!("Replacing an existing session on spawn");
            self.kill_session(id);
        }
        options.close_fds = true;
        options.child_rlimits = self.config.inherited_limits;
        if let Some(environment) = environment {
            options.base_env = Some(environment.vars);
        }
        let info = {
            let _guard = fork_guard();
            local_tty::spawn(options)?
        };
        // SAFETY: the spawn result transfers ownership of the leader fd.
        let leader = unsafe { OwnedFd::from_raw_fd(info.result.leader_fd) };
        protocol::set_nonblocking(leader.as_raw_fd())?;
        let token = self.allocate_token();
        let session = match Session::new(
            id,
            metadata,
            info.child,
            leader,
            token,
            self.config.ring_capacity,
            self.poll.registry(),
        ) {
            Ok(session) => session,
            Err(err) => bail!("Failed to watch pty: {err}"),
        };
        log::info!("Spawned session with pid {}", session.pid);
        self.sources.insert(token, Source::Leader(id));
        self.sessions.insert(id, session);
        Ok(())
    }

    fn attach_session(
        &mut self,
        id: PersistentSessionId,
        with_replay: bool,
    ) -> (DaemonResponse, Vec<OwnedFd>) {
        match self.try_attach_session(id, with_replay) {
            Ok(result) => result,
            Err(err) => (DaemonResponse::Error(format!("{err:#}")), Vec::new()),
        }
    }

    fn try_attach_session(
        &mut self,
        id: PersistentSessionId,
        with_replay: bool,
    ) -> Result<(DaemonResponse, Vec<OwnedFd>)> {
        if !self.sessions.contains_key(&id) {
            bail!("unknown session");
        }
        self.detach_session(id);

        let (daemon_end, app_end) = {
            let _guard = fork_guard();
            let (daemon_end, app_end) = socket::socketpair(
                socket::AddressFamily::Unix,
                socket::SockType::Stream,
                None,
                socketpair_flags(),
            )?;
            // SAFETY: socketpair returned two fresh fds that we now own.
            let (daemon_end, app_end) = unsafe {
                (
                    OwnedFd::from_raw_fd(daemon_end),
                    OwnedFd::from_raw_fd(app_end),
                )
            };
            protocol::set_cloexec(daemon_end.as_raw_fd())?;
            protocol::set_cloexec(app_end.as_raw_fd())?;
            (daemon_end, app_end)
        };
        protocol::set_nonblocking(daemon_end.as_raw_fd())?;

        let token = self.allocate_token();
        self.poll.registry().register(
            &mut mio::unix::SourceFd(&daemon_end.as_raw_fd()),
            token,
            Interest::READABLE | Interest::WRITABLE,
        )?;
        self.sources.insert(token, Source::Data(id));

        let session = self.sessions.get_mut(&id).expect("checked above");
        let (replay, has_bootstrap_preamble) = if with_replay {
            let replay = session.history.replay();
            (replay.bytes, replay.has_bootstrap_preamble)
        } else {
            (Vec::new(), session.history.has_bootstrap_preamble())
        };
        let replay_len = replay.len() as u64;
        let mut attachment = Attachment::new(daemon_end, token, replay);
        if session.exit_status.is_some() {
            attachment.close_after_flush = Some(false);
        }
        session.attachment = Some(attachment);

        let mut fds = vec![app_end];
        if let Some(leader) = &session.leader {
            let _guard = fork_guard();
            let leader = leader.try_clone()?;
            protocol::set_cloexec(leader.as_raw_fd())?;
            fds.push(leader);
        }
        let info = AttachInfo {
            session_id: id,
            pid: session.pid,
            metadata: session.metadata.clone(),
            replay_len,
            has_bootstrap_preamble,
            exit_status: session.exit_status,
        };
        self.ready.insert(id);
        Ok((DaemonResponse::Attached(info), fds))
    }

    fn unregister_attachment(&mut self, attachment: Attachment) {
        let _ = self
            .poll
            .registry()
            .deregister(&mut mio::unix::SourceFd(&attachment.data.as_raw_fd()));
        self.sources.remove(&attachment.token);
    }

    fn detach_session(&mut self, id: PersistentSessionId) {
        let Some(session) = self.sessions.get_mut(&id) else {
            return;
        };
        if session.attachment.is_none() {
            return;
        }
        let attachment = session.detach();
        if let Some(attachment) = attachment {
            self.unregister_attachment(attachment);
        }

        self.ready.insert(id);
    }

    fn detach_all(&mut self) {
        let ids: Vec<_> = self.sessions.keys().copied().collect();
        for id in ids {
            self.detach_session(id);
        }
    }

    /// Removes a session and sends SIGKILL to its process without waiting;
    /// the process is reaped on SIGCHLD.
    fn kill_session(&mut self, id: PersistentSessionId) {
        let Some(mut session) = self.remove_session(id) else {
            return;
        };
        let Some(mut child) = session.child.take() else {
            return;
        };
        let _ = child.kill();
        if !matches!(child.try_wait(), Ok(Some(_)) | Err(_)) {
            self.orphans.push(child);
        }
    }

    /// Removes a session, closing its leader and data socket.
    fn remove_session(&mut self, id: PersistentSessionId) -> Option<Session> {
        let mut session = self.sessions.remove(&id)?;
        self.ready.remove(&id);

        if let Some(attachment) = session.attachment.take() {
            self.unregister_attachment(attachment);
        }
        session.close_leader(self.poll.registry());
        self.sources.remove(&session.leader_token);
        Some(session)
    }

    fn pump(&mut self, id: PersistentSessionId) {
        let Some(session) = self.sessions.get_mut(&id) else {
            return;
        };
        let outcome = session.pump(self.poll.registry());
        #[cfg(test)]
        if session.is_paused() {
            self.stats
                .pauses
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        match outcome {
            PumpOutcome::Idle => {}
            PumpOutcome::MoreWork => {
                self.ready.insert(id);
            }
            PumpOutcome::DataClosed => self.detach_session(id),
            PumpOutcome::Finished { emit_event } => {
                let Some(session) = self.remove_session(id) else {
                    return;
                };
                if emit_event
                    && let (Some(token), Some(status)) = (self.active, session.exit_status)
                {
                    self.send(
                        token,
                        &DaemonMessage::Event(DaemonEvent::SessionExited {
                            session_id: id,
                            pid: session.pid,
                            status,
                        }),
                        Vec::new(),
                    );
                }
            }
        }
    }

    fn reap_children(&mut self) {
        self.orphans
            .retain_mut(|child| !matches!(child.try_wait(), Ok(Some(_)) | Err(_)));

        let mut exited = Vec::new();
        for session in self.sessions.values_mut() {
            if let Some(child) = &mut session.child
                && let Ok(Some(status)) = child.try_wait()
            {
                exited.push((session.id, observed_exit_status(status)));
            }
        }
        for (id, status) in exited {
            self.on_child_exit(id, status);
        }
    }

    fn on_child_exit(&mut self, id: PersistentSessionId, status: crate::event::ObservedExitStatus) {
        let registry = self.poll.registry();
        let Some(session) = self.sessions.get_mut(&id) else {
            return;
        };
        log::info!("Session with pid {} exited: {status:?}", session.pid);
        session.child = None;
        session.exit_status = Some(status);
        session.drain_leader(registry);
        session.close_leader(registry);
        self.sources.remove(&session.leader_token);
        match &mut session.attachment {
            Some(attachment) => {
                attachment.close_after_flush = Some(true);
                self.ready.insert(id);
            }
            None => session.exited_at = Some(Instant::now()),
        }
    }

    fn expire_exited_sessions(&mut self) {
        let ttl = self.config.exited_session_ttl;
        let expired: Vec<_> = self
            .sessions
            .values()
            .filter(|session| {
                session.attachment.is_none()
                    && session.exited_at.is_some_and(|at| at.elapsed() >= ttl)
            })
            .map(|session| session.id)
            .collect();
        for id in expired {
            self.remove_session(id);
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn socketpair_flags() -> socket::SockFlag {
    socket::SockFlag::SOCK_CLOEXEC
}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
fn socketpair_flags() -> socket::SockFlag {
    socket::SockFlag::empty()
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let ids: Vec<_> = self.sessions.keys().copied().collect();
        for id in ids {
            self.kill_session(id);
        }
        for mut child in self.orphans.drain(..) {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[cfg(test)]
#[path = "daemon_tests.rs"]
mod tests;

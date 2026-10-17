//! Pane-scoped endpoint that publishes Warp-to-agent control events (currently rich input
//! visibility) to local CLI agent processes as UTF-8 JSON Lines.

use std::ffi::{OsStr, OsString};
use std::future::Future;
use std::io;
use std::sync::Arc;
use std::time::Duration;

use async_compat::Compat;
use parking_lot::Mutex;
use tokio::io::{AsyncWrite, AsyncWriteExt as _};
use uuid::Uuid;
use warp_core::cli_agent_protocol::CLIAgentControlEvent;
use warpui::r#async::executor::{Background, BackgroundTask};

use super::{CLIAgentInputState, CLIAgentSessionsModelEvent};

/// Simultaneously connected clients per pane; further connections are closed on accept.
const MAX_CLIENTS: usize = 16;
/// Lines queued per client before it is treated as stalled and dropped, so a client that stopped
/// reading never back-pressures the publisher.
const CLIENT_QUEUE_CAPACITY: usize = 8;
/// Upper bound on a single client write before the client is dropped.
const CLIENT_WRITE_TIMEOUT: Duration = Duration::from_secs(5);
/// Hex chars kept from the pane's random id, short enough for macOS's 104-byte socket path limit.
const SHORT_ID_LEN: usize = 16;

struct EndpointState {
    rich_input_active: bool,
    clients: Vec<async_channel::Sender<Arc<str>>>,
    closed: bool,
}

/// Cloneable handle that broadcasts state changes to the endpoint's connected clients.
#[derive(Clone)]
pub(crate) struct CLIAgentControlPublisher {
    state: Arc<Mutex<EndpointState>>,
}

impl CLIAgentControlPublisher {
    /// Mirrors a session model event for this pane onto the control channel.
    pub(crate) fn observe(&self, event: &CLIAgentSessionsModelEvent) {
        match event {
            CLIAgentSessionsModelEvent::InputSessionChanged {
                new_input_state, ..
            } => self.set_rich_input_active(matches!(
                new_input_state,
                CLIAgentInputState::Open { .. }
            )),
            CLIAgentSessionsModelEvent::Ended { .. } => self.set_rich_input_active(false),
            CLIAgentSessionsModelEvent::Started { .. }
            | CLIAgentSessionsModelEvent::StatusChanged { .. }
            | CLIAgentSessionsModelEvent::SessionUpdated { .. } => {}
        }
    }

    /// Publishes a `rich_input` event if `active` differs from the last published state.
    pub(crate) fn set_rich_input_active(&self, active: bool) {
        let mut state = self.state.lock();
        if state.rich_input_active == active {
            return;
        }
        state.rich_input_active = active;
        let line = rich_input_line(active);
        state
            .clients
            .retain(|client| client.try_send(line.clone()).is_ok());
    }
}

/// Owns one pane's listener and client connections; dropping it closes all of them.
pub(crate) struct CLIAgentControlEndpoint {
    address: OsString,
    publisher: CLIAgentControlPublisher,
    accept_task: BackgroundTask,
}

impl CLIAgentControlEndpoint {
    /// Binds a new endpoint with rich input reported as inactive and starts accepting clients
    /// on `executor`.
    pub(crate) fn bind(executor: Arc<Background>) -> io::Result<Self> {
        let id = Uuid::new_v4().simple().to_string();
        let address = platform::address(&id[..SHORT_ID_LEN])?;
        let listener = platform::bind(&address)?;
        let state = Arc::new(Mutex::new(EndpointState {
            rich_input_active: false,
            clients: Vec::new(),
            closed: false,
        }));
        let accept_task = executor.spawn(accept_loop(
            listener,
            address.clone(),
            state.clone(),
            executor.clone(),
        ));
        Ok(Self {
            address,
            publisher: CLIAgentControlPublisher { state },
            accept_task,
        })
    }

    /// Address CLI agents connect to: a socket path on Unix, a named pipe path on Windows.
    pub(crate) fn address(&self) -> &OsStr {
        &self.address
    }

    pub(crate) fn publisher(&self) -> CLIAgentControlPublisher {
        self.publisher.clone()
    }
}

impl Drop for CLIAgentControlEndpoint {
    fn drop(&mut self) {
        self.accept_task.abort();
        self.publisher.set_rich_input_active(false);
        {
            let mut state = self.publisher.state.lock();
            state.closed = true;
            // Dropping the senders lets each writer flush what is queued and then close its
            // stream.
            state.clients.clear();
        }
        #[cfg(unix)]
        let _ = std::fs::remove_file(&self.address);
    }
}

fn rich_input_line(active: bool) -> Arc<str> {
    let mut line = serde_json::to_string(&CLIAgentControlEvent::rich_input(active))
        .expect("control event serializes to JSON");
    line.push('\n');
    line.into()
}

async fn accept_loop(
    mut listener: platform::Listener,
    address: OsString,
    state: Arc<Mutex<EndpointState>>,
    executor: Arc<Background>,
) {
    loop {
        let stream = match listener.accept().await {
            Ok(stream) => stream,
            Err(err) => {
                log::warn!(
                    "CLI agent control endpoint {} stopped accepting clients: {err}",
                    address.display()
                );
                return;
            }
        };
        let (tx, rx) = async_channel::bounded(CLIENT_QUEUE_CAPACITY);
        {
            let mut state = state.lock();
            state.clients.retain(|client| !client.is_closed());
            if state.closed || state.clients.len() >= MAX_CLIENTS {
                continue;
            }
            // Queued before the client is visible to the publisher, so it always observes the
            // current state first.
            let _ = tx.try_send(rich_input_line(state.rich_input_active));
            state.clients.push(tx);
        }
        executor.spawn(write_loop(stream, rx)).detach();
    }
}

async fn write_loop(
    mut stream: impl AsyncWrite + Unpin,
    lines: async_channel::Receiver<Arc<str>>,
) {
    while let Ok(line) = lines.recv().await {
        match tokio::time::timeout(CLIENT_WRITE_TIMEOUT, stream.write_all(line.as_bytes())).await {
            Ok(Ok(())) => {}
            Ok(Err(_)) | Err(_) => return,
        }
    }
}

/// Prefixes an I/O error with the path it concerns.
fn address_error(address: &OsStr, err: io::Error) -> io::Error {
    io::Error::new(err.kind(), format!("{}: {err}", address.display()))
}

/// Tokio I/O resources register with the reactor on creation, which requires a tokio context.
fn in_tokio_context<T>(future: impl Future<Output = T>) -> T {
    warpui::r#async::block_on(Compat::new(future))
}

#[cfg(unix)]
mod platform {
    use std::ffi::{OsStr, OsString};
    use std::fs::Permissions;
    use std::io;
    use std::os::unix::fs::PermissionsExt as _;
    use std::os::unix::net::UnixListener as StdUnixListener;
    use std::path::{Path, PathBuf};

    use tokio::net::{UnixListener, UnixStream};

    use super::{address_error, in_tokio_context};

    pub(super) type Stream = UnixStream;

    pub(super) struct Listener(UnixListener);

    fn socket_dir() -> io::Result<PathBuf> {
        match std::env::var_os("XDG_RUNTIME_DIR") {
            Some(runtime_dir) if !runtime_dir.is_empty() => {
                let dir = PathBuf::from(runtime_dir).join("warp");
                std::fs::create_dir_all(&dir)
                    .and_then(|()| std::fs::set_permissions(&dir, Permissions::from_mode(0o700)))
                    .map_err(|err| address_error(dir.as_os_str(), err))?;
                Ok(dir)
            }
            Some(_) | None => Ok(std::env::temp_dir()),
        }
    }

    pub(super) fn address(id: &str) -> io::Result<OsString> {
        Ok(socket_dir()?.join(format!("ca-{id}.sock")).into_os_string())
    }

    pub(super) fn bind(address: &OsStr) -> io::Result<Listener> {
        let path = Path::new(address);
        let listener = StdUnixListener::bind(path).map_err(|err| address_error(address, err))?;
        let listener = register(path, listener).map_err(|err| {
            let _ = std::fs::remove_file(path);
            address_error(address, err)
        })?;
        Ok(Listener(listener))
    }

    fn register(path: &Path, listener: StdUnixListener) -> io::Result<UnixListener> {
        std::fs::set_permissions(path, Permissions::from_mode(0o600))?;
        listener.set_nonblocking(true)?;
        in_tokio_context(async { UnixListener::from_std(listener) })
    }

    impl Listener {
        pub(super) async fn accept(&mut self) -> io::Result<Stream> {
            self.0.accept().await.map(|(stream, _)| stream)
        }
    }
}

#[cfg(windows)]
mod platform {
    use std::ffi::{OsStr, OsString};
    use std::io;

    use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};
    use warp_core::channel::ChannelState;

    use super::{address_error, in_tokio_context};

    pub(super) type Stream = NamedPipeServer;

    /// The pipe instance waiting for the next client; a fresh instance replaces it on accept.
    pub(super) struct Listener {
        address: OsString,
        server: NamedPipeServer,
    }

    pub(super) fn address(id: &str) -> io::Result<OsString> {
        Ok(format!(
            r"\\.\pipe\Warp{:?}_cli_agent_control_{id}",
            ChannelState::channel()
        )
        .into())
    }

    pub(super) fn bind(address: &OsStr) -> io::Result<Listener> {
        let server = in_tokio_context(async {
            ServerOptions::new()
                .first_pipe_instance(true)
                .create(address)
        })
        .map_err(|err| address_error(address, err))?;
        Ok(Listener {
            address: address.to_owned(),
            server,
        })
    }

    impl Listener {
        pub(super) async fn accept(&mut self) -> io::Result<Stream> {
            self.server.connect().await?;
            let next = ServerOptions::new().create(&self.address)?;
            Ok(std::mem::replace(&mut self.server, next))
        }
    }
}

#[cfg(test)]
#[path = "control_endpoint_tests.rs"]
mod tests;

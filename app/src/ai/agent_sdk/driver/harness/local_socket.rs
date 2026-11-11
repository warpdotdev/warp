//! A socket the agent driver listens on for helper processes it starts on the same host.
//!
//! Where the platform has Unix domain sockets the driver listens on a socket file in a private
//! directory, so filesystem permissions already decide who can connect; elsewhere it listens on
//! an ephemeral loopback port. Either way a helper presents a token before anything else, which
//! tells it from any other local client that found the endpoint.
use std::fmt;
use std::io::{self, Read, Write};
use std::net::{Ipv4Addr, Shutdown, SocketAddr, TcpListener, TcpStream};
#[cfg(unix)]
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::task::{Context, Poll};

use anyhow::{Context as _, Result, anyhow};
use async_io::Async;
use futures::io::{AsyncRead, AsyncReadExt as _, AsyncWrite};
use rand::RngCore as _;
use serde::{Deserialize, Serialize};

#[cfg(unix)]
const SOCKET_FILE_NAME: &str = "agent.sock";
/// Size of `sockaddr_un::sun_path` on the most constrained supported platform (macOS), which
/// bounds the socket path including its terminator.
#[cfg(unix)]
const MAX_UNIX_SOCKET_PATH: usize = 104;
/// Random bytes in a token, which travels hex-encoded.
pub(super) const TOKEN_BYTES: usize = 32;
/// Upper bound on the token line a helper sends first; anything longer is not our peer.
pub(super) const MAX_TOKEN_LINE: usize = 2 * TOKEN_BYTES;
/// The driver's reply to a token it accepts.
pub(super) const VERDICT_ACCEPTED: &str = "ok";
/// The driver's reply to a token it does not, after which it closes the connection.
pub(super) const VERDICT_REJECTED: &str = "rejected";
/// Upper bound on the verdict line; a driver that says anything longer is not ours.
const MAX_VERDICT_LINE: usize = 16;

/// Where the driver accepts connections.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Endpoint {
    /// A socket file in the driver's private directory.
    Unix(PathBuf),
    /// An ephemeral loopback port.
    Tcp(SocketAddr),
}

impl fmt::Display for Endpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unix(path) => write!(f, "{}", path.display()),
            Self::Tcp(address) => write!(f, "{address}"),
        }
    }
}

/// A helper's end of the connection, over whichever transport the driver chose.
pub(super) enum Stream {
    #[cfg(unix)]
    Unix(UnixStream),
    Tcp(TcpStream),
}

impl Stream {
    pub(super) fn connect(endpoint: &Endpoint) -> Result<Self> {
        match endpoint {
            #[cfg(unix)]
            Endpoint::Unix(path) => UnixStream::connect(path)
                .map(Self::Unix)
                .with_context(|| format!("Failed to connect to the agent driver at {endpoint}")),
            #[cfg(not(unix))]
            Endpoint::Unix(_) => Err(anyhow!(
                "The agent driver listens on a Unix socket, which this platform cannot connect to"
            )),
            Endpoint::Tcp(address) => {
                let stream = TcpStream::connect(address).with_context(|| {
                    format!("Failed to connect to the agent driver at {endpoint}")
                })?;
                stream.set_nodelay(true)?;
                Ok(Self::Tcp(stream))
            }
        }
    }

    pub(super) fn try_clone(&self) -> io::Result<Self> {
        match self {
            #[cfg(unix)]
            Self::Unix(stream) => stream.try_clone().map(Self::Unix),
            Self::Tcp(stream) => stream.try_clone().map(Self::Tcp),
        }
    }

    pub(super) fn shutdown(&self, how: Shutdown) -> io::Result<()> {
        match self {
            #[cfg(unix)]
            Self::Unix(stream) => stream.shutdown(how),
            Self::Tcp(stream) => stream.shutdown(how),
        }
    }
}

impl Read for Stream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            #[cfg(unix)]
            Self::Unix(stream) => stream.read(buf),
            Self::Tcp(stream) => stream.read(buf),
        }
    }
}

impl Write for Stream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            #[cfg(unix)]
            Self::Unix(stream) => stream.write(buf),
            Self::Tcp(stream) => stream.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            #[cfg(unix)]
            Self::Unix(stream) => stream.flush(),
            Self::Tcp(stream) => stream.flush(),
        }
    }
}

#[cfg(unix)]
impl From<Stream> for std::os::fd::OwnedFd {
    fn from(stream: Stream) -> Self {
        match stream {
            Stream::Unix(stream) => stream.into(),
            Stream::Tcp(stream) => stream.into(),
        }
    }
}

/// Connects to the driver, presents the token, and returns once the driver has accepted it.
pub(super) fn connect_with_token(endpoint: &Endpoint, token: &str) -> Result<Stream> {
    let mut stream = Stream::connect(endpoint)?;
    stream
        .write_all(format!("{token}\n").as_bytes())
        .context("Failed to send the token to the agent driver")?;
    let verdict = read_line(&mut stream, MAX_VERDICT_LINE)
        .context("The agent driver closed the connection before answering the token")?;
    if verdict == VERDICT_ACCEPTED {
        Ok(stream)
    } else if verdict == VERDICT_REJECTED {
        Err(anyhow!("The agent driver rejected the token"))
    } else {
        Err(anyhow!(
            "Unexpected reply from the agent driver: {verdict:?}"
        ))
    }
}

/// Reads one newline-terminated line, one byte at a time so nothing of the stream that follows
/// is consumed.
pub(super) fn read_line(stream: &mut impl Read, max_len: usize) -> Result<String> {
    let mut line = Vec::with_capacity(max_len);
    let mut byte = [0u8; 1];
    loop {
        stream.read_exact(&mut byte)?;
        if byte[0] == b'\n' {
            return Ok(String::from_utf8(line)?.trim_end().to_owned());
        }
        line.push(byte[0]);
        if line.len() > max_len {
            anyhow::bail!("line too long");
        }
    }
}

/// A fresh hex-encoded token from the OS entropy source.
pub(super) fn new_token() -> String {
    let mut bytes = [0u8; TOKEN_BYTES];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}

/// Reads the newline-terminated token a helper sends first.
pub(super) async fn read_token_line(connection: &Connection) -> Result<String> {
    read_connection_line(connection, MAX_TOKEN_LINE).await
}

/// Reads one newline-terminated line of at most `max_len` bytes from a helper, one byte at a
/// time so nothing of the stream that follows is consumed.
pub(super) async fn read_connection_line(
    connection: &Connection,
    max_len: usize,
) -> Result<String> {
    let mut connection = connection;
    let mut line = Vec::with_capacity(max_len.min(MAX_TOKEN_LINE));
    let mut byte = [0u8; 1];
    loop {
        connection.read_exact(&mut byte).await?;
        if byte[0] == b'\n' {
            return Ok(String::from_utf8(line)?.trim_end().to_owned());
        }
        line.push(byte[0]);
        if line.len() > max_len {
            anyhow::bail!("line too long");
        }
    }
}

/// The driver's listening socket.
pub(super) enum Listener {
    #[cfg(unix)]
    Unix(Async<UnixListener>),
    Tcp(Async<TcpListener>),
}

impl Listener {
    /// Listens on a socket file in `dir` where the platform supports it and the path fits;
    /// otherwise on an ephemeral loopback port.
    pub(super) fn bind_in(dir: &Path) -> Result<(Self, Endpoint)> {
        if let Some(bound) = Self::bind_unix_in(dir)? {
            return Ok(bound);
        }
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .context("Failed to bind a loopback port for the agent driver")?;
        let address = listener.local_addr()?;
        let listener =
            Async::new(listener).context("Failed to register the agent driver's listener")?;
        Ok((Self::Tcp(listener), Endpoint::Tcp(address)))
    }

    #[cfg(unix)]
    fn bind_unix_in(dir: &Path) -> Result<Option<(Self, Endpoint)>> {
        let path = dir.join(SOCKET_FILE_NAME);
        if path.as_os_str().len() >= MAX_UNIX_SOCKET_PATH {
            log::warn!(
                "Socket path {} is too long for a Unix socket; using loopback TCP",
                path.display()
            );
            return Ok(None);
        }
        let listener = UnixListener::bind(&path)
            .with_context(|| format!("Failed to bind socket {}", path.display()))?;
        let listener =
            Async::new(listener).context("Failed to register the agent driver's listener")?;
        Ok(Some((Self::Unix(listener), Endpoint::Unix(path))))
    }

    #[cfg(not(unix))]
    fn bind_unix_in(_: &Path) -> Result<Option<(Self, Endpoint)>> {
        Ok(None)
    }

    pub(super) async fn accept(&self) -> io::Result<Connection> {
        match self {
            #[cfg(unix)]
            Self::Unix(listener) => listener
                .accept()
                .await
                .map(|(stream, _)| Connection::Unix(stream)),
            Self::Tcp(listener) => listener
                .accept()
                .await
                .map(|(stream, _)| Connection::Tcp(stream)),
        }
    }
}

/// The driver's end of an accepted connection.
pub(super) enum Connection {
    #[cfg(unix)]
    Unix(Async<UnixStream>),
    Tcp(Async<TcpStream>),
}

impl Connection {
    pub(super) fn shutdown(&self, how: Shutdown) -> io::Result<()> {
        match self {
            #[cfg(unix)]
            Self::Unix(stream) => stream.get_ref().shutdown(how),
            Self::Tcp(stream) => stream.get_ref().shutdown(how),
        }
    }
}

impl AsyncRead for &Connection {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        match &**self {
            #[cfg(unix)]
            Connection::Unix(stream) => Pin::new(&mut &*stream).poll_read(cx, buf),
            Connection::Tcp(stream) => Pin::new(&mut &*stream).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for &Connection {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        match &**self {
            #[cfg(unix)]
            Connection::Unix(stream) => Pin::new(&mut &*stream).poll_write(cx, buf),
            Connection::Tcp(stream) => Pin::new(&mut &*stream).poll_write(cx, buf),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match &**self {
            #[cfg(unix)]
            Connection::Unix(stream) => Pin::new(&mut &*stream).poll_flush(cx),
            Connection::Tcp(stream) => Pin::new(&mut &*stream).poll_flush(cx),
        }
    }

    fn poll_close(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(self.shutdown(Shutdown::Write))
    }
}

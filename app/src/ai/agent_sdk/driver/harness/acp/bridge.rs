//! Runs an ACP agent inside the driver's PTY shell session while the driver talks to it over a
//! local socket.
//!
//! The driver listens on a socket, writes its endpoint, a one-time token, and the agent command
//! line to a launch file in a private directory, and submits `<this executable> acp-bridge
//! --launch-file <path>` as a command in the terminal session. Nothing else crosses the shell,
//! so the agent's argv is never subject to the shell's quoting or native-argument rules. The
//! bridge subcommand connects, proves it is the expected peer by sending the token, waits for
//! the driver's verdict, and then hands the connection to the agent as its stdin and stdout: on
//! Unix by replacing itself with the agent, elsewhere by relaying between the socket and the
//! agent's pipes. Either way the agent runs as the shell's foreground command (it inherits the
//! shell state established by environment setup commands, its launch is a block in the shared
//! session, and the terminal's interrupt and force-kill paths reach it) while its protocol
//! stream never touches the PTY.
//!
//! Where the platform has Unix domain sockets the driver listens on a socket file in the same
//! private directory as the launch file, so filesystem permissions already decide who can
//! connect and the token is a second layer; elsewhere it listens on an ephemeral loopback port
//! and the token is the only one.
//!
//! This requires the driver, the shell session, and the agent to share one OS instance (the
//! socket and the launch file are both host-local). That holds for every cloud-runner shell; it
//! would not for a sandboxed, WSL, or SSH session launched from a host Warp.
use std::fmt;
use std::io::{self, Read, Write};
use std::net::{Ipv4Addr, Shutdown, SocketAddr, TcpListener, TcpStream};
#[cfg(unix)]
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use anyhow::{Context as _, Result, anyhow};
use async_io::Async;
use futures::FutureExt as _;
use futures::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};
use rand::RngCore as _;
use serde::{Deserialize, Serialize};
use warp_util::path::ShellFamily;

const LAUNCH_FILE_NAME: &str = "launch.json";
#[cfg(unix)]
const SOCKET_FILE_NAME: &str = "agent.sock";
/// Size of `sockaddr_un::sun_path` on the most constrained supported platform (macOS), which
/// bounds the socket path including its terminator.
#[cfg(unix)]
const MAX_UNIX_SOCKET_PATH: usize = 104;
/// Random bytes in the one-time token, which travels hex-encoded.
const TOKEN_BYTES: usize = 32;
/// Upper bound on the token line the bridge sends first; anything longer is not our peer.
const MAX_TOKEN_LINE: usize = 2 * TOKEN_BYTES;
/// The driver's reply to a token it accepts.
const VERDICT_ACCEPTED: &str = "ok";
/// The driver's reply to a token it does not, after which it closes the connection.
const VERDICT_REJECTED: &str = "rejected";
/// Upper bound on the verdict line; a driver that says anything longer is not ours.
const MAX_VERDICT_LINE: usize = 16;
/// How long a relayed agent may keep running after the driver has closed the connection before
/// the bridge terminates it.
#[cfg_attr(unix, allow(dead_code, reason = "Unix execs the agent instead"))]
const AGENT_EXIT_GRACE: Duration = Duration::from_secs(5);

/// Where the driver accepts the bridge connection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum BridgeEndpoint {
    /// A socket file in the launch file's private directory.
    Unix(PathBuf),
    /// An ephemeral loopback port.
    Tcp(SocketAddr),
}

impl fmt::Display for BridgeEndpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unix(path) => write!(f, "{}", path.display()),
            Self::Tcp(address) => write!(f, "{address}"),
        }
    }
}

/// What the driver hands the bridge through the launch file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct BridgeLaunch {
    pub endpoint: BridgeEndpoint,
    /// Presented before any protocol traffic so the driver can tell the bridge from any other
    /// local client that found the endpoint.
    pub token: String,
    /// The agent command line.
    pub agent: Vec<String>,
}

/// Builds the shell command that launches the agent described by `launch_file` through the
/// bridge. The bridge is this same executable, so the command names it by path.
pub(crate) fn bridge_command(launch_file: &Path, shell_family: ShellFamily) -> Result<String> {
    let executable = std::env::current_exe()
        .context("Failed to locate the running executable for the ACP bridge")?;
    Ok(render_bridge_command(
        &executable,
        launch_file,
        shell_family,
    ))
}

fn render_bridge_command(
    executable: &Path,
    launch_file: &Path,
    shell_family: ShellFamily,
) -> String {
    let executable = executable.display().to_string();
    let executable = shell_family.shell_escape(&executable);
    let invocation = match shell_family {
        ShellFamily::Posix => executable,
        ShellFamily::PowerShell => format!("& {executable}").into(),
    };
    format!(
        "{invocation} acp-bridge --launch-file {}",
        shell_family.shell_escape(&launch_file.display().to_string())
    )
}

/// Worker-side entry point. Only returns on failure; on success the agent's exit status becomes
/// this process's.
pub(crate) fn run_bridge(launch_file: &Path) -> Result<()> {
    let launch = read_launch(launch_file)?;
    let (program, args) = launch
        .agent
        .split_first()
        .ok_or_else(|| anyhow!("The ACP launch file names no agent command"))?;
    let stream = connect_with_token(&launch.endpoint, &launch.token)?;
    #[cfg(unix)]
    {
        use command::unix::CommandExt as _;
        let mut command = exec_agent_command(stream, program, args)?;
        let error = command.exec();
        Err(anyhow!(error)).with_context(|| format!("Failed to exec ACP agent `{program}`"))
    }
    #[cfg(not(unix))]
    {
        let status = relay_agent(stream, program, args, AGENT_EXIT_GRACE)?;
        std::process::exit(status.code().unwrap_or(1))
    }
}

fn read_launch(launch_file: &Path) -> Result<BridgeLaunch> {
    let contents = std::fs::read_to_string(launch_file)
        .with_context(|| format!("Failed to read ACP launch file {}", launch_file.display()))?;
    serde_json::from_str(&contents)
        .with_context(|| format!("Malformed ACP launch file {}", launch_file.display()))
}

/// The bridge's end of the connection, over whichever transport the driver chose.
enum BridgeStream {
    #[cfg(unix)]
    Unix(UnixStream),
    Tcp(TcpStream),
}

impl BridgeStream {
    fn connect(endpoint: &BridgeEndpoint) -> Result<Self> {
        match endpoint {
            #[cfg(unix)]
            BridgeEndpoint::Unix(path) => UnixStream::connect(path)
                .map(Self::Unix)
                .with_context(|| format!("Failed to connect to the ACP driver at {endpoint}")),
            #[cfg(not(unix))]
            BridgeEndpoint::Unix(_) => Err(anyhow!(
                "The ACP driver listens on a Unix socket, which this platform cannot connect to"
            )),
            BridgeEndpoint::Tcp(address) => {
                let stream = TcpStream::connect(address).with_context(|| {
                    format!("Failed to connect to the ACP driver at {endpoint}")
                })?;
                stream.set_nodelay(true)?;
                Ok(Self::Tcp(stream))
            }
        }
    }

    fn try_clone(&self) -> io::Result<Self> {
        match self {
            #[cfg(unix)]
            Self::Unix(stream) => stream.try_clone().map(Self::Unix),
            Self::Tcp(stream) => stream.try_clone().map(Self::Tcp),
        }
    }

    fn shutdown(&self, how: Shutdown) -> io::Result<()> {
        match self {
            #[cfg(unix)]
            Self::Unix(stream) => stream.shutdown(how),
            Self::Tcp(stream) => stream.shutdown(how),
        }
    }
}

impl Read for BridgeStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            #[cfg(unix)]
            Self::Unix(stream) => stream.read(buf),
            Self::Tcp(stream) => stream.read(buf),
        }
    }
}

impl Write for BridgeStream {
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
impl From<BridgeStream> for std::os::fd::OwnedFd {
    fn from(stream: BridgeStream) -> Self {
        match stream {
            BridgeStream::Unix(stream) => stream.into(),
            BridgeStream::Tcp(stream) => stream.into(),
        }
    }
}

/// Connects to the driver, presents the token, and returns once the driver has accepted it.
fn connect_with_token(endpoint: &BridgeEndpoint, token: &str) -> Result<BridgeStream> {
    let mut stream = BridgeStream::connect(endpoint)?;
    stream
        .write_all(format!("{token}\n").as_bytes())
        .context("Failed to send the ACP token")?;
    let verdict = read_line(&mut stream, MAX_VERDICT_LINE)
        .context("The ACP driver closed the connection before answering the token")?;
    if verdict == VERDICT_ACCEPTED {
        Ok(stream)
    } else if verdict == VERDICT_REJECTED {
        Err(anyhow!("The ACP driver rejected the bridge token"))
    } else {
        Err(anyhow!("Unexpected reply from the ACP driver: {verdict:?}"))
    }
}

/// Reads one newline-terminated line, one byte at a time so nothing of the stream that follows
/// is consumed.
fn read_line(stream: &mut impl Read, max_len: usize) -> Result<String> {
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

/// The agent command with the socket as its stdin and stdout. Stderr is inherited, so agent
/// diagnostics land in the terminal block.
#[cfg(unix)]
fn exec_agent_command(
    stream: BridgeStream,
    program: &str,
    args: &[String],
) -> Result<command::blocking::Command> {
    use std::os::fd::OwnedFd;
    use std::process::Stdio;

    let stdin = stream
        .try_clone()
        .context("Failed to clone the ACP socket for stdin")?;
    let mut command = command::blocking::Command::new(program);
    command
        .args(args)
        .stdin(Stdio::from(OwnedFd::from(stdin)))
        .stdout(Stdio::from(OwnedFd::from(stream)));
    Ok(command)
}

/// Runs the agent with piped stdin/stdout and copies bytes between those pipes and the socket
/// until the agent exits. Used where the agent cannot simply inherit the socket.
///
/// Once the driver closes its side the agent sees EOF on stdin; if it is still running after
/// `exit_grace` it is terminated so the terminal block cannot hang on an agent that ignores EOF.
#[cfg_attr(
    unix,
    allow(dead_code, reason = "Unix execs the agent instead; kept testable")
)]
fn relay_agent(
    stream: BridgeStream,
    program: &str,
    args: &[String],
    exit_grace: Duration,
) -> Result<std::process::ExitStatus> {
    use std::process::Stdio;
    use std::sync::mpsc;

    // Agents installed through npm are `.cmd` shims on Windows. `CreateProcess` does not consult
    // `PATHEXT`, so look the program up the way the shell would; std then runs a `.cmd`/`.bat`
    // path through `cmd.exe` with the appropriate quoting.
    let program_path = warp_util::path::resolve_executable(program)
        .ok_or_else(|| anyhow!("ACP agent `{program}` was not found on PATH"))?;
    let mut child = command::blocking::Command::new(program_path.as_ref())
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .with_context(|| format!("Failed to start ACP agent `{program}`"))?;
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle as _;
        // Ties the agent and anything it spawns (the shim's `node`, for instance) to this
        // process: when the bridge exits for any reason, including being killed, the job closes
        // and the whole agent tree goes with it.
        if let Err(error) = command::windows::JobObject::new()
            .assign_process(child.as_raw_handle() as isize)
            .kill_children_on_close()
            .create()
        {
            eprintln!("acp-bridge: the agent will outlive the bridge if killed: {error:#}");
        }
    }
    let mut agent_stdin = child.stdin.take().context("agent has no stdin")?;
    let mut agent_stdout = child.stdout.take().context("agent has no stdout")?;

    let mut socket_reader = stream.try_clone()?;
    let (driver_closed_tx, driver_closed_rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("acp-bridge-to-agent".into())
        .spawn(move || {
            // Not `io::copy`: on Linux it becomes `splice`, which holds the pipe lock while
            // waiting for socket data, so the agent cannot even close its stdin to exit.
            let mut buffer = [0u8; 8192];
            loop {
                match socket_reader.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if agent_stdin.write_all(&buffer[..n]).is_err() {
                            break;
                        }
                    }
                }
            }
            // EOF from the driver (or a failure) closes the agent's stdin by dropping it.
            drop(agent_stdin);
            let _ = driver_closed_tx.send(());
        })?;
    let mut socket_writer = stream.try_clone()?;
    let (agent_drained_tx, agent_drained_rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("acp-bridge-from-agent".into())
        .spawn(move || {
            let mut buffer = [0u8; 8192];
            loop {
                match agent_stdout.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if socket_writer.write_all(&buffer[..n]).is_err() {
                            break;
                        }
                    }
                }
            }
            let _ = socket_writer.shutdown(Shutdown::Write);
            let _ = agent_drained_tx.send(());
        })?;

    let status = wait_with_exit_grace(&mut child, &driver_closed_rx, exit_grace)?;
    // Deliver whatever the agent wrote before exiting, but do not wait on a stdout pipe that a
    // surviving grandchild may still hold open. The threads are not joined: the socket pump
    // blocks until the driver closes its side, which may be long after the agent is gone.
    let _ = agent_drained_rx.recv_timeout(exit_grace);
    let _ = stream.shutdown(Shutdown::Both);
    Ok(status)
}

/// Waits for `child` to exit, killing it if it is still running `exit_grace` after
/// `driver_closed` fires.
#[cfg_attr(
    unix,
    allow(dead_code, reason = "Unix execs the agent instead; kept testable")
)]
fn wait_with_exit_grace(
    child: &mut std::process::Child,
    driver_closed: &std::sync::mpsc::Receiver<()>,
    exit_grace: Duration,
) -> io::Result<std::process::ExitStatus> {
    use std::sync::mpsc::RecvTimeoutError;

    use instant::Instant;

    const POLL_INTERVAL: Duration = Duration::from_millis(50);

    let mut kill_at = None;
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        match kill_at {
            None => match driver_closed.recv_timeout(POLL_INTERVAL) {
                Ok(()) | Err(RecvTimeoutError::Disconnected) => {
                    kill_at = Some(Instant::now() + exit_grace);
                }
                Err(RecvTimeoutError::Timeout) => {}
            },
            Some(deadline) if Instant::now() >= deadline => {
                eprintln!("acp-bridge: agent did not exit within {exit_grace:?} of EOF; killing");
                child.kill()?;
                return child.wait();
            }
            Some(_) => std::thread::sleep(POLL_INTERVAL),
        }
    }
}

/// Driver-side listener for one bridged agent.
pub(crate) struct BridgeListener {
    /// Holds the private directory the launch file (and socket file, where used) lives in;
    /// removed with the listener.
    _dir: tempfile::TempDir,
    token: String,
    launch_file: PathBuf,
    endpoint: BridgeEndpoint,
    listener: Listener,
}

impl BridgeListener {
    /// Starts listening and writes the launch file, with a fresh token, for the agent command
    /// `program` `args`.
    pub(crate) fn bind(program: &str, args: &[String]) -> Result<Self> {
        // The token is only as private as this directory, and on Unix so is the socket.
        // `tempfile` creates it mode 0700 there; on Windows it inherits the ACL of `%TEMP%`,
        // which is per-user by default.
        let dir = tempfile::Builder::new()
            .prefix("oz-acp-")
            .tempdir()
            .context("Failed to create a directory for the ACP launch file")?;
        let (listener, endpoint) = Listener::bind_in(dir.path())?;
        let token = new_token();
        let launch = BridgeLaunch {
            endpoint: endpoint.clone(),
            token: token.clone(),
            agent: std::iter::once(program.to_owned())
                .chain(args.iter().cloned())
                .collect(),
        };
        let launch_file = dir.path().join(LAUNCH_FILE_NAME);
        std::fs::write(&launch_file, serde_json::to_vec(&launch)?).with_context(|| {
            format!("Failed to write ACP launch file {}", launch_file.display())
        })?;
        Ok(Self {
            _dir: dir,
            token,
            launch_file,
            endpoint,
            listener,
        })
    }

    pub(crate) fn launch_file(&self) -> &Path {
        &self.launch_file
    }

    /// Waits for the bridge to connect and present the token, telling other local clients they
    /// are rejected and moving on, and returns the agent's stdout (our reader) and stdin (our
    /// writer) ends of the socket.
    pub(crate) async fn accept(&self, timeout: Duration) -> Result<(BridgeReader, BridgeWriter)> {
        let deadline = warpui::r#async::Timer::after(timeout).fuse();
        futures::pin_mut!(deadline);
        loop {
            let accept = self.listener.accept().fuse();
            futures::pin_mut!(accept);
            let connection = futures::select! {
                accepted = accept => {
                    accepted.context("Failed to accept the ACP bridge connection")?
                }
                _ = deadline => return Err(anyhow!(
                    "The ACP agent did not connect to {} within {timeout:?}",
                    self.endpoint
                )),
            };
            let presented = {
                let token_line = read_token_line(&connection).fuse();
                futures::pin_mut!(token_line);
                futures::select! {
                    presented = token_line => presented,
                    _ = deadline => return Err(anyhow!(
                        "The ACP agent did not present its token within {timeout:?}"
                    )),
                }
            };
            match presented {
                Ok(presented) if presented == self.token => {
                    (&mut &connection)
                        .write_all(format!("{VERDICT_ACCEPTED}\n").as_bytes())
                        .await
                        .context("Failed to acknowledge the ACP bridge")?;
                    let connection = Arc::new(connection);
                    return Ok((BridgeReader(connection.clone()), BridgeWriter(connection)));
                }
                Ok(_) => log::warn!("Rejected an ACP bridge connection with the wrong token"),
                Err(error) => log::warn!("Rejected an ACP bridge connection: {error}"),
            }
            // Best effort: the impostor may already be gone, and either way it is dropped next.
            let _ = (&mut &connection)
                .write_all(format!("{VERDICT_REJECTED}\n").as_bytes())
                .await;
            let _ = connection.shutdown(Shutdown::Both);
        }
    }
}

/// A fresh hex-encoded token from the OS entropy source.
fn new_token() -> String {
    let mut bytes = [0u8; TOKEN_BYTES];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}

/// Reads the newline-terminated token the bridge sends first, one byte at a time so nothing of
/// the protocol stream that follows is consumed.
async fn read_token_line(connection: &Connection) -> Result<String> {
    let mut connection = connection;
    let mut line = Vec::with_capacity(MAX_TOKEN_LINE);
    let mut byte = [0u8; 1];
    loop {
        connection.read_exact(&mut byte).await?;
        if byte[0] == b'\n' {
            return Ok(String::from_utf8(line)?.trim_end().to_owned());
        }
        line.push(byte[0]);
        if line.len() > MAX_TOKEN_LINE {
            anyhow::bail!("token line too long");
        }
    }
}

/// The driver's listening socket.
enum Listener {
    #[cfg(unix)]
    Unix(Async<UnixListener>),
    Tcp(Async<TcpListener>),
}

impl Listener {
    /// Listens on a socket file in `dir` where the platform supports it and the path fits;
    /// otherwise on an ephemeral loopback port.
    fn bind_in(dir: &Path) -> Result<(Self, BridgeEndpoint)> {
        if let Some(bound) = Self::bind_unix_in(dir)? {
            return Ok(bound);
        }
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .context("Failed to bind a loopback port for the ACP bridge")?;
        let address = listener.local_addr()?;
        let listener = Async::new(listener).context("Failed to register the ACP listener")?;
        Ok((Self::Tcp(listener), BridgeEndpoint::Tcp(address)))
    }

    #[cfg(unix)]
    fn bind_unix_in(dir: &Path) -> Result<Option<(Self, BridgeEndpoint)>> {
        let path = dir.join(SOCKET_FILE_NAME);
        if path.as_os_str().len() >= MAX_UNIX_SOCKET_PATH {
            log::warn!(
                "ACP socket path {} is too long for a Unix socket; using loopback TCP",
                path.display()
            );
            return Ok(None);
        }
        let listener = UnixListener::bind(&path)
            .with_context(|| format!("Failed to bind ACP socket {}", path.display()))?;
        let listener = Async::new(listener).context("Failed to register the ACP listener")?;
        Ok(Some((Self::Unix(listener), BridgeEndpoint::Unix(path))))
    }

    #[cfg(not(unix))]
    fn bind_unix_in(_: &Path) -> Result<Option<(Self, BridgeEndpoint)>> {
        Ok(None)
    }

    async fn accept(&self) -> io::Result<Connection> {
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
enum Connection {
    #[cfg(unix)]
    Unix(Async<UnixStream>),
    Tcp(Async<TcpStream>),
}

impl Connection {
    fn shutdown(&self, how: Shutdown) -> io::Result<()> {
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

/// The agent's stdout.
pub(crate) struct BridgeReader(Arc<Connection>);

impl AsyncRead for BridgeReader {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut &*self.0).poll_read(cx, buf)
    }
}

/// The agent's stdin. Closing it shuts down the socket's write side, which the agent sees as
/// EOF; dropping it without closing leaves the agent waiting for more input.
pub(crate) struct BridgeWriter(Arc<Connection>);

impl AsyncWrite for BridgeWriter {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut &*self.0).poll_write(cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut &*self.0).poll_flush(cx)
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut &*self.0).poll_close(cx)
    }
}

#[cfg(test)]
#[path = "bridge_tests.rs"]
mod tests;

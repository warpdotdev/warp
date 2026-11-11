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
use std::io::{self, Read, Write};
use std::net::Shutdown;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use anyhow::{Context as _, Result, anyhow};
use futures::FutureExt as _;
use futures::io::{AsyncRead, AsyncWrite, AsyncWriteExt as _};
use serde::{Deserialize, Serialize};
use warp_util::path::ShellFamily;

pub(crate) use super::super::local_socket::Endpoint as BridgeEndpoint;
use super::super::local_socket::{
    Connection, Listener, Stream as BridgeStream, VERDICT_ACCEPTED, VERDICT_REJECTED,
    connect_with_token, new_token, read_token_line,
};

const LAUNCH_FILE_NAME: &str = "launch.json";
/// How long a relayed agent may keep running after the driver has closed the connection before
/// the bridge terminates it.
#[cfg_attr(unix, allow(dead_code, reason = "Unix execs the agent instead"))]
const AGENT_EXIT_GRACE: Duration = Duration::from_secs(5);

/// What the driver hands the bridge through the launch file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct BridgeLaunch {
    pub endpoint: BridgeEndpoint,
    /// Presented before any protocol traffic so the driver can tell the bridge from any other
    /// local client that found the endpoint.
    pub token: String,
    /// The agent command line, naming the preferred program.
    pub agent: Vec<String>,
    /// Other names the program may be installed under, tried in order when the preferred one
    /// is not on the shell's `PATH`.
    #[serde(default)]
    pub alternates: Vec<String>,
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
    let (preferred, args) = launch
        .agent
        .split_first()
        .ok_or_else(|| anyhow!("The ACP launch file names no agent command"))?;
    let stream = connect_with_token(&launch.endpoint, &launch.token)?;
    // Resolved here, in the shell's environment, rather than by the driver: the driver's `PATH`
    // need not match the shell's once environment setup commands have run.
    let path_env = std::env::var_os("PATH").unwrap_or_default();
    let program = select_agent_program(preferred, &launch.alternates, &path_env)?;
    if program != *preferred {
        eprintln!("oz: `{preferred}` is not on PATH, launching the ACP agent as `{program}`");
    }
    let program = program.as_str();
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

/// The first of `preferred` and `alternates` installed on `path_env`.
fn select_agent_program(
    preferred: &str,
    alternates: &[String],
    path_env: &std::ffi::OsStr,
) -> Result<String> {
    let candidates = std::iter::once(preferred).chain(alternates.iter().map(String::as_str));
    candidates
        .clone()
        .find(|candidate| {
            warp_util::path::resolve_executable_in_path(candidate, path_env).is_some()
        })
        .map(str::to_owned)
        .ok_or_else(|| {
            let names = candidates
                .map(|candidate| format!("`{candidate}`"))
                .collect::<Vec<_>>()
                .join(", ");
            anyhow!("No ACP agent found on PATH; tried {names}")
        })
}

fn read_launch(launch_file: &Path) -> Result<BridgeLaunch> {
    let contents = std::fs::read_to_string(launch_file)
        .with_context(|| format!("Failed to read ACP launch file {}", launch_file.display()))?;
    serde_json::from_str(&contents)
        .with_context(|| format!("Malformed ACP launch file {}", launch_file.display()))
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
    /// `programs[0]` `args`, where the rest of `programs` are other names the bridge may find
    /// the same agent under.
    pub(crate) fn bind(programs: &[&str], args: &[String]) -> Result<Self> {
        let (program, alternates) = programs
            .split_first()
            .ok_or_else(|| anyhow!("No ACP agent program to launch"))?;
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
            agent: std::iter::once((*program).to_owned())
                .chain(args.iter().cloned())
                .collect(),
            alternates: alternates.iter().map(|name| (*name).to_owned()).collect(),
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

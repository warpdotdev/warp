//! Runs one hook command as a child process: the JSON payload goes in on stdin, the decision
//! comes back on stdout. This is the minimal runner: it enforces the invocation timeout with a
//! single kill. Output limits, a termination grace period, Windows Job Objects, bounded
//! concurrency, and the full environment policy come later.

use std::ffi::OsString;
use std::future::Future;
use std::io;
use std::process::{ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_process::Child;
use command::r#async::Command;
use futures::FutureExt;
use futures::future::{BoxFuture, Fuse, FusedFuture};
use futures::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use instant::Instant;
use warpui_core::r#async::Timer;

use super::{HookInvocation, HookProcessOutcome, HookRunner, PathCapture, ProcessExit};
use crate::hooks::model::HOOK_PROTOCOL_VERSION;

/// Longest stderr excerpt kept from one run.
const STDERR_TAIL_LIMIT: usize = 4096;

/// How often the cancel flag is checked while the process runs.
const CANCEL_POLL_INTERVAL: Duration = Duration::from_millis(50);

/// How long to keep reading the pipes after the hook itself has exited. A descendant the hook
/// left behind can hold them open; its output is not the hook's decision.
const PIPE_GRACE: Duration = Duration::from_secs(1);

/// How long to wait for a killed process to be collected before moving on without it.
const REAP_GRACE: Duration = Duration::from_secs(2);

/// Runs hook commands through the host shell: `sh -c` on Unix, `cmd.exe /C` on Windows. The
/// command string is passed to the shell exactly as written in the configuration; payload bytes
/// only ever travel over stdin.
pub struct CommandHookRunner {
    path_capture: PathCapture,
    shell: HostShell,
}

#[derive(Clone)]
struct HostShell {
    program: OsString,
    command_flag: &'static str,
}

impl HostShell {
    fn for_this_host() -> Self {
        #[cfg(windows)]
        {
            Self {
                program: OsString::from("cmd.exe"),
                command_flag: "/C",
            }
        }
        #[cfg(not(windows))]
        {
            Self {
                program: OsString::from("/bin/sh"),
                command_flag: "-c",
            }
        }
    }

    fn command(&self) -> Command {
        #[cfg(unix)]
        {
            Command::new_with_process_group(&self.program)
        }
        #[cfg(not(unix))]
        {
            Command::new(&self.program)
        }
    }
}

impl CommandHookRunner {
    pub fn new(path_capture: PathCapture) -> Self {
        Self {
            path_capture,
            shell: HostShell::for_this_host(),
        }
    }

    /// Replaces the shell program, so tests can make spawning fail.
    #[cfg(any(test, feature = "test-util"))]
    pub fn with_shell_program(mut self, program: impl Into<OsString>) -> Self {
        self.shell.program = program.into();
        self
    }
}

impl HookRunner for CommandHookRunner {
    fn run(&self, invocation: HookInvocation) -> BoxFuture<'static, HookProcessOutcome> {
        let login_path = (self.path_capture)();
        let shell = self.shell.clone();
        async move { run_once(shell, login_path, invocation).await }.boxed()
    }
}

async fn run_once(
    shell: HostShell,
    login_path: Option<OsString>,
    invocation: HookInvocation,
) -> HookProcessOutcome {
    let started = Instant::now();
    let finish = |exit: ProcessExit, (stdout, stderr): (Vec<u8>, Vec<u8>)| HookProcessOutcome {
        exit,
        stdout,
        stdout_truncated: false,
        stderr_tail: stderr_tail(&stderr),
        duration: started.elapsed(),
    };

    if invocation.cancel.load(Ordering::Acquire) {
        return finish(ProcessExit::Cancelled, (Vec::new(), Vec::new()));
    }
    let mut child = match build_command(&shell, login_path, &invocation).spawn() {
        Ok(child) => child,
        Err(error) => {
            return finish(
                ProcessExit::SpawnFailed(error.to_string()),
                (Vec::new(), Vec::new()),
            );
        }
    };
    let pid = child.id();
    let mut pipes = HookPipes::attach(&mut child, invocation.stdin);
    let exited = child.status();

    let exit = match wait_for_exit(exited, &mut pipes, invocation.timeout, invocation.cancel).await
    {
        Ok(status) => {
            pipes.drain_after_exit(pid).await;
            exit_from_status(status)
        }
        Err(interrupted) => kill_and_reap(&mut child, pid, interrupted).await,
    };
    finish(exit, pipes.take_output())
}

/// The shell invocation for one hook, with stdin, stdout, and stderr piped.
fn build_command(
    shell: &HostShell,
    login_path: Option<OsString>,
    invocation: &HookInvocation,
) -> Command {
    let mut command = shell.command();
    command
        .arg(shell.command_flag)
        .arg(&invocation.command)
        .current_dir(&invocation.cwd)
        .envs(invocation.env.iter().map(|(key, value)| (key, value)));
    if let Some(path) = login_path {
        command.env("PATH", path);
    }
    command
        .env(
            "WARP_HOOK_PROTOCOL_VERSION",
            HOOK_PROTOCOL_VERSION.to_string(),
        )
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    command
}

/// The hook's three pipes as futures, plus the output read so far. The three are driven together
/// with the exit wait so a hook that fills one pipe before touching another cannot deadlock
/// against this process.
struct HookPipes {
    stdin_written: Fuse<BoxFuture<'static, ()>>,
    stdout_drained: Fuse<BoxFuture<'static, ()>>,
    stderr_drained: Fuse<BoxFuture<'static, ()>>,
    /// Shared with the drain futures so whatever arrived before reading stopped is kept.
    stdout_bytes: Arc<Mutex<Vec<u8>>>,
    stderr_bytes: Arc<Mutex<Vec<u8>>>,
}

impl HookPipes {
    fn attach(child: &mut Child, payload: Vec<u8>) -> Self {
        let stdin = child.stdin.take();
        let stdout_bytes = Arc::new(Mutex::new(Vec::new()));
        let stderr_bytes = Arc::new(Mutex::new(Vec::new()));
        Self {
            stdin_written: async move {
                // A hook that exits without reading its input is judged by its exit code, so a
                // failed write is not an error here. Dropping the handle closes the pipe.
                if let Some(mut stdin) = stdin {
                    let _ = stdin.write_all(&payload).await;
                    let _ = stdin.close().await;
                }
            }
            .boxed()
            .fuse(),
            stdout_drained: drain(child.stdout.take(), stdout_bytes.clone())
                .boxed()
                .fuse(),
            stderr_drained: drain(child.stderr.take(), stderr_bytes.clone())
                .boxed()
                .fuse(),
            stdout_bytes,
            stderr_bytes,
        }
    }

    /// Keeps reading for up to [`PIPE_GRACE`] once the hook has exited. A background process the
    /// hook started may still hold the pipes; after the grace, reading stops rather than letting a
    /// stray descendant turn a decision into a timeout.
    async fn drain_after_exit(&mut self, pid: u32) {
        let mut grace = Timer::after(PIPE_GRACE).fuse();
        while !(self.stdout_drained.is_terminated() && self.stderr_drained.is_terminated()) {
            let mut stdin_written = &mut self.stdin_written;
            let mut stdout_drained = &mut self.stdout_drained;
            let mut stderr_drained = &mut self.stderr_drained;
            futures::select_biased! {
                () = stdout_drained => {}
                () = stderr_drained => {}
                () = stdin_written => {}
                _ = grace => {
                    log::debug!(
                        "hook process {pid} exited but something it started still holds its \
                         output pipes; reading stopped"
                    );
                    return;
                }
            }
        }
    }

    fn take_output(&self) -> (Vec<u8>, Vec<u8>) {
        (
            take_bytes(&self.stdout_bytes),
            take_bytes(&self.stderr_bytes),
        )
    }
}

/// Drives the pipes until the hook exits, the deadline passes, or the cancel flag is set. The
/// exit wins over a deadline that expired in the same instant, so a finished hook is never
/// reported as timed out.
async fn wait_for_exit(
    exited: impl Future<Output = io::Result<ExitStatus>>,
    pipes: &mut HookPipes,
    timeout: Duration,
    cancel: Arc<AtomicBool>,
) -> Result<io::Result<ExitStatus>, ProcessExit> {
    let exited = exited.fuse();
    futures::pin_mut!(exited);
    let mut deadline = Timer::after(timeout).fuse();
    let mut cancelled = Box::pin(
        async move {
            loop {
                Timer::after(CANCEL_POLL_INTERVAL).await;
                if cancel.load(Ordering::Acquire) {
                    return;
                }
            }
        }
        .fuse(),
    );
    let mut stdin_written = &mut pipes.stdin_written;
    let mut stdout_drained = &mut pipes.stdout_drained;
    let mut stderr_drained = &mut pipes.stderr_drained;
    loop {
        futures::select_biased! {
            status = exited => return Ok(status),
            () = stdin_written => {}
            () = stdout_drained => {}
            () = stderr_drained => {}
            _ = deadline => return Err(ProcessExit::TimedOut),
            _ = cancelled => return Err(ProcessExit::Cancelled),
        }
    }
}

/// Ends a hook that outlived its deadline or was cancelled, and waits for it to be collected so
/// a pid check afterwards sees it gone. A hook that turns out to have exited already is reported
/// by its exit status, as it would have been one poll earlier.
async fn kill_and_reap(child: &mut Child, pid: u32, interrupted: ProcessExit) -> ProcessExit {
    match kill_unless_exited(child, pid) {
        Some(status) => exit_from_status(Ok(status)),
        None => {
            futures::select_biased! {
                _ = child.status().fuse() => {}
                _ = Timer::after(REAP_GRACE).fuse() => {
                    log::warn!(
                        "hook process {pid} was killed but did not exit within {REAP_GRACE:?}"
                    );
                }
            }
            interrupted
        }
    }
}

/// Kills the hook and, on Unix, everything else in its process group. The group is signalled
/// only while the hook itself is known to be alive: once it has been collected its pid, and so
/// its group id, may already belong to someone else. Returns the exit status when the hook turns
/// out to have exited already.
fn kill_unless_exited(child: &mut Child, pid: u32) -> Option<ExitStatus> {
    match child.try_status() {
        Ok(None) => {
            #[cfg(unix)]
            {
                use nix::sys::signal::{Signal, killpg};
                use nix::unistd::Pid;
                if let Err(error) = killpg(Pid::from_raw(pid as i32), Signal::SIGKILL) {
                    log::debug!("could not signal hook process group {pid}: {error}");
                }
            }
            if let Err(error) = child.kill() {
                log::debug!("could not kill hook process {pid}: {error}");
            }
            None
        }
        Ok(Some(status)) => {
            log::debug!(
                "hook process {pid} had already exited; anything it left running was not signalled"
            );
            Some(status)
        }
        Err(error) => {
            log::debug!("could not check hook process {pid} before killing it: {error}");
            None
        }
    }
}

fn exit_from_status(status: io::Result<ExitStatus>) -> ProcessExit {
    let status = match status {
        Ok(status) => status,
        Err(error) => return ProcessExit::SpawnFailed(format!("could not wait for exit: {error}")),
    };
    if let Some(code) = status.code() {
        return ProcessExit::Code(code);
    }
    #[cfg(unix)]
    if let Some(signal) = std::os::unix::process::ExitStatusExt::signal(&status) {
        return ProcessExit::Signal(signal);
    }
    // Every supported platform reports either a code or a signal; treat anything else as a
    // failed exit so the failure policy applies.
    ProcessExit::Code(-1)
}

/// Copies everything `reader` produces into `sink` until it closes or fails.
async fn drain(reader: Option<impl AsyncRead + Unpin>, sink: Arc<Mutex<Vec<u8>>>) {
    let Some(mut reader) = reader else {
        return;
    };
    let mut chunk = [0u8; 8192];
    loop {
        match reader.read(&mut chunk).await {
            Ok(0) | Err(_) => return,
            Ok(read) => sink
                .lock()
                .expect("hook output lock")
                .extend_from_slice(&chunk[..read]),
        }
    }
}

fn take_bytes(sink: &Mutex<Vec<u8>>) -> Vec<u8> {
    std::mem::take(&mut *sink.lock().expect("hook output lock"))
}

fn stderr_tail(stderr: &[u8]) -> String {
    let start = stderr.len().saturating_sub(STDERR_TAIL_LIMIT);
    String::from_utf8_lossy(&stderr[start..]).into_owned()
}

#[cfg(test)]
#[path = "command_tests.rs"]
mod tests;

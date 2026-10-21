//! Runs one hook command as a child process: the JSON payload goes in on stdin, the decision
//! comes back on stdout. The runner enforces the invocation's deadline and cancel flag, ends the
//! hook's whole process tree when the hook is done with a stop request first and a kill after a
//! grace period, keeps a bounded amount of output, runs at most [`HOOK_MAX_CONCURRENT`] hooks at
//! once, and writes one log line per invocation.

use std::ffi::OsString;
use std::future::Future;
use std::io;
use std::path::PathBuf;
use std::process::{ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_process::Child;
use command::r#async::{Command, JobGuard};
use futures::FutureExt;
use futures::future::{BoxFuture, Fuse, FusedFuture};
use futures::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use instant::Instant;
use warpui_core::r#async::Timer;

use super::env::{RunnerEnvironment, host_environment};
use super::{HookInvocation, HookProcessOutcome, HookRunner, PathCapture, ProcessExit};
use crate::hooks::model::{HOOK_PROTOCOL_VERSION, HookBindingId};

/// How long a hook's process tree gets to stop on its own after being asked to, before it is
/// killed.
pub const HOOK_KILL_GRACE: Duration = Duration::from_secs(2);

/// Most stdout bytes kept from one hook. The rest is read and discarded so the hook can exit,
/// and the outcome says the response was cut off.
pub const HOOK_STDOUT_LIMIT: usize = 1024 * 1024;

/// Most stderr bytes kept from one hook, counted from the end.
pub const HOOK_STDERR_TAIL: usize = 4096;

/// How many hook processes may run at the same time. Invocations beyond this wait their turn,
/// and the wait counts against their deadline.
pub const HOOK_MAX_CONCURRENT: usize = 4;

/// Longest stderr excerpt written to the invocation log line.
const STDERR_TAIL_LOG_LIMIT: usize = 500;

/// How often the cancel flag and the process tree are checked while waiting on them.
const POLL_INTERVAL: Duration = Duration::from_millis(50);

/// How long to keep reading the pipes once the hook's tree has ended. A writer that left the
/// tree (with `setsid`, for example) can still hold them; its output is not the hook's decision.
const PIPE_GRACE: Duration = Duration::from_secs(1);

/// How long to wait for a killed hook to be collected before moving on without it.
const REAP_GRACE: Duration = Duration::from_secs(2);

/// Runs hook commands through the host shell: `sh -c` on Unix, `cmd.exe /C` on Windows. The
/// command string is passed to the shell exactly as written in the configuration; payload bytes
/// only ever travel over stdin.
pub struct CommandHookRunner {
    path_capture: PathCapture,
    home: Option<PathBuf>,
    shell: HostShell,
    permits: Permits,
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

    /// The shell invocation for `script`, with the command text appended the way the shell
    /// expects it: as one argument on Unix, and verbatim on Windows, where `cmd.exe` does not
    /// follow the quoting rules an argument would be encoded with.
    fn invoke(&self, script: &str) -> Command {
        let mut command = Command::new_in_job(&self.program);
        command.arg(self.command_flag);
        #[cfg(windows)]
        {
            use command::windows::CommandExt;
            command.raw_arg(script);
        }
        #[cfg(not(windows))]
        command.arg(script);
        command
    }
}

impl CommandHookRunner {
    pub fn new(path_capture: PathCapture) -> Self {
        Self {
            path_capture,
            home: dirs::home_dir(),
            shell: HostShell::for_this_host(),
            permits: Permits::new(HOOK_MAX_CONCURRENT),
        }
    }

    /// Replaces the shell program, so tests can make spawning fail.
    #[cfg(any(test, feature = "test-util"))]
    pub fn with_shell_program(mut self, program: impl Into<OsString>) -> Self {
        self.shell.program = program.into();
        self
    }

    /// Replaces the home directory given to hooks.
    #[cfg(any(test, feature = "test-util"))]
    pub fn with_home(mut self, home: Option<PathBuf>) -> Self {
        self.home = home;
        self
    }
}

impl HookRunner for CommandHookRunner {
    fn run(&self, invocation: HookInvocation) -> BoxFuture<'static, HookProcessOutcome> {
        let runner_env = RunnerEnvironment {
            variables: std::env::vars_os().collect(),
            login_path: (self.path_capture)(),
            home: self.home.clone(),
            protocol_version: HOOK_PROTOCOL_VERSION,
        };
        let shell = self.shell.clone();
        let permits = self.permits.clone();
        async move {
            let binding = invocation.binding.clone();
            let outcome = run_once(&shell, runner_env, &permits, invocation).await;
            log_invocation(&binding, &outcome);
            outcome
        }
        .boxed()
    }
}

/// The one record of an invocation that is always written: how the hook ended and the end of
/// what it said on stderr, never its input or its stdout.
fn log_invocation(binding: &HookBindingId, outcome: &HookProcessOutcome) {
    log::info!(
        "hook invocation: event={} source={} binding={} exit={:?} duration_ms={} stderr_tail={:?}",
        binding.event,
        binding.source,
        binding.index,
        outcome.exit,
        outcome.duration.as_millis(),
        truncate_to_char_boundary(&outcome.stderr_tail, STDERR_TAIL_LOG_LIMIT)
    );
}

async fn run_once(
    shell: &HostShell,
    runner_env: RunnerEnvironment,
    permits: &Permits,
    invocation: HookInvocation,
) -> HookProcessOutcome {
    let started = Instant::now();
    let deadline = started + invocation.timeout;
    let finish = |exit: ProcessExit, captured: Captured| HookProcessOutcome {
        exit,
        stdout: captured.stdout,
        stdout_truncated: captured.stdout_truncated,
        stderr_tail: escape_control(&captured.stderr_tail),
        duration: started.elapsed(),
    };

    if invocation.cancel.load(Ordering::Acquire) {
        return finish(ProcessExit::Cancelled, Captured::default());
    }
    let _permit = match permits.acquire(deadline, &invocation.cancel).await {
        Ok(permit) => permit,
        Err(exit) => {
            let why = match exit {
                ProcessExit::TimedOut => "its deadline passed",
                ProcessExit::Cancelled => "it was cancelled",
                ProcessExit::Code(_) | ProcessExit::Signal(_) | ProcessExit::SpawnFailed(_) => {
                    "it ended"
                }
            };
            log::warn!(
                "hook {} did not start: {why} while waiting for one of the \
                 {HOOK_MAX_CONCURRENT} hook slots",
                invocation.binding
            );
            return finish(exit, Captured::default());
        }
    };

    let mut command = shell.invoke(&invocation.command);
    command
        .current_dir(&invocation.cwd)
        .env_clear()
        .envs(host_environment(&runner_env))
        .envs(invocation.env.iter().map(|(key, value)| (key, value)))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let (mut child, tree) = match command.spawn_in_job() {
        Ok(spawned) => spawned,
        Err(error) => {
            return finish(
                ProcessExit::SpawnFailed(error.to_string()),
                Captured::default(),
            );
        }
    };
    let pid = child.id();
    let mut pipes = HookPipes::attach(&mut child, invocation.stdin);
    let exited = child.status();

    let exit = match wait_for_exit(exited, &mut pipes, deadline, invocation.cancel).await {
        Ok(status) => {
            end_tree(&tree, &mut child, &mut pipes).await;
            exit_from_status(status)
        }
        Err(interrupted) => {
            // A hook that exited in the moment between its exit and the deadline is reported by
            // its exit status, as it would have been one poll earlier.
            let already_exited = match child.try_status() {
                Ok(status) => status,
                Err(error) => {
                    log::debug!("could not check hook process {pid} before stopping it: {error}");
                    None
                }
            };
            end_tree(&tree, &mut child, &mut pipes).await;
            reap(&mut child, pid).await;
            match already_exited {
                Some(status) => exit_from_status(Ok(status)),
                None => interrupted,
            }
        }
    };
    pipes.finish_reading(pid).await;
    finish(exit, pipes.take_captured())
}

/// Drives the pipes until the hook exits, the deadline passes, or the cancel flag is set. The
/// exit wins over a deadline that expired in the same instant, so a finished hook is never
/// reported as timed out.
async fn wait_for_exit(
    exited: impl Future<Output = io::Result<ExitStatus>>,
    pipes: &mut HookPipes,
    deadline: Instant,
    cancel: Arc<AtomicBool>,
) -> Result<io::Result<ExitStatus>, ProcessExit> {
    let exited = exited.fuse();
    futures::pin_mut!(exited);
    let mut deadline = Timer::at(deadline).fuse();
    let mut cancelled = Box::pin(cancelled(cancel).fuse());
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

/// Resolves once `cancel` is set.
async fn cancelled(cancel: Arc<AtomicBool>) {
    loop {
        Timer::after(POLL_INTERVAL).await;
        if cancel.load(Ordering::Acquire) {
            return;
        }
    }
}

/// Ends whatever is left of the hook's process tree: asks it to stop, gives it
/// [`HOOK_KILL_GRACE`] to do so, then kills it. Nothing a hook starts outlives its invocation,
/// on any path, so a timed-out hook cannot keep working and a finished one cannot leave a
/// descendant holding its pipes. The pipes keep being read and the hook keeps being collected
/// while this waits, since the tree only counts as ended once its members are gone.
async fn end_tree(tree: &JobGuard, child: &mut Child, pipes: &mut HookPipes) {
    if !tree.is_alive() {
        return;
    }
    tree.request_stop();
    let mut grace = Timer::after(HOOK_KILL_GRACE).fuse();
    let exited = child.status().fuse();
    futures::pin_mut!(exited);
    let mut poll = Timer::after(POLL_INTERVAL).fuse();
    loop {
        let mut stdin_written = &mut pipes.stdin_written;
        let mut stdout_drained = &mut pipes.stdout_drained;
        let mut stderr_drained = &mut pipes.stderr_drained;
        futures::select_biased! {
            _ = exited => {}
            () = stdout_drained => {}
            () = stderr_drained => {}
            () = stdin_written => {}
            _ = poll => {
                if !tree.is_alive() {
                    return;
                }
                poll = Timer::after(POLL_INTERVAL).fuse();
            }
            _ = grace => break,
        }
    }
    log::debug!("hook process tree did not stop within {HOOK_KILL_GRACE:?}; killing it");
    tree.terminate();
}

/// Waits for a stopped hook to be collected, so a pid check afterwards sees it gone.
async fn reap(child: &mut Child, pid: u32) {
    futures::select_biased! {
        _ = child.status().fuse() => {}
        _ = Timer::after(REAP_GRACE).fuse() => {
            log::warn!("hook process {pid} was killed but was not collected within {REAP_GRACE:?}");
        }
    }
}

/// Lets at most a fixed number of hooks run at once. A permit is a token taken from a channel
/// and returned when dropped; a waiter that gives up holds nothing.
#[derive(Clone)]
struct Permits {
    available: async_channel::Receiver<()>,
    returns: async_channel::Sender<()>,
}

struct Permit {
    returns: async_channel::Sender<()>,
}

impl Permits {
    fn new(count: usize) -> Self {
        let (returns, available) = async_channel::bounded(count);
        for _ in 0..count {
            returns
                .try_send(())
                .expect("the channel holds exactly `count` permits");
        }
        Self { available, returns }
    }

    /// Waits for a permit until `deadline` passes or `cancel` is set.
    async fn acquire(
        &self,
        deadline: Instant,
        cancel: &Arc<AtomicBool>,
    ) -> Result<Permit, ProcessExit> {
        let mut taken = Box::pin(self.available.recv().fuse());
        let mut deadline = Timer::at(deadline).fuse();
        let mut cancelled = Box::pin(cancelled(cancel.clone()).fuse());
        futures::select_biased! {
            _ = taken => Ok(Permit { returns: self.returns.clone() }),
            _ = cancelled => Err(ProcessExit::Cancelled),
            _ = deadline => Err(ProcessExit::TimedOut),
        }
    }
}

impl Drop for Permit {
    fn drop(&mut self) {
        // Only as many permits exist as the channel has room for, so this cannot fail while the
        // runner lives; a runner that is gone has no one left to wait.
        let _ = self.returns.try_send(());
    }
}

/// What has been read from the hook so far, within the limits.
#[derive(Default)]
struct Captured {
    stdout: Vec<u8>,
    stdout_truncated: bool,
    stderr_tail: Vec<u8>,
}

/// The hook's three pipes as futures, plus the output read so far. The three are driven together
/// with the exit wait so a hook that fills one pipe before touching another cannot deadlock
/// against this process.
struct HookPipes {
    stdin_written: Fuse<BoxFuture<'static, ()>>,
    stdout_drained: Fuse<BoxFuture<'static, ()>>,
    stderr_drained: Fuse<BoxFuture<'static, ()>>,
    /// Shared with the drain futures so whatever arrived before reading stopped is kept.
    captured: Arc<Mutex<Captured>>,
}

impl HookPipes {
    fn attach(child: &mut Child, payload: Vec<u8>) -> Self {
        let stdin = child.stdin.take();
        let captured = Arc::new(Mutex::new(Captured::default()));
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
            stdout_drained: drain_stdout(child.stdout.take(), captured.clone())
                .boxed()
                .fuse(),
            stderr_drained: drain_stderr(child.stderr.take(), captured.clone())
                .boxed()
                .fuse(),
            captured,
        }
    }

    /// Reads until both output pipes close, for up to [`PIPE_GRACE`]. Once the hook's tree has
    /// ended only a writer that escaped it can still hold them, and it does not get to delay
    /// the outcome.
    async fn finish_reading(&mut self, pid: u32) {
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
                        "hook process {pid} is gone but something outside its process tree still \
                         holds its output pipes; reading stopped"
                    );
                    return;
                }
            }
        }
    }

    fn take_captured(&self) -> Captured {
        std::mem::take(&mut *self.captured.lock().expect("hook output lock"))
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

/// Keeps the first [`HOOK_STDOUT_LIMIT`] bytes of stdout and reads the rest away, so the hook
/// is never blocked on a full pipe, noting that the response was cut off.
async fn drain_stdout(reader: Option<impl AsyncRead + Unpin>, captured: Arc<Mutex<Captured>>) {
    let Some(mut reader) = reader else {
        return;
    };
    let mut chunk = [0u8; 8192];
    loop {
        match reader.read(&mut chunk).await {
            Ok(0) | Err(_) => return,
            Ok(read) => {
                let mut captured = captured.lock().expect("hook output lock");
                let room = HOOK_STDOUT_LIMIT.saturating_sub(captured.stdout.len());
                let kept = read.min(room);
                captured.stdout.extend_from_slice(&chunk[..kept]);
                if kept < read {
                    captured.stdout_truncated = true;
                }
            }
        }
    }
}

/// Keeps only the last [`HOOK_STDERR_TAIL`] bytes of stderr.
async fn drain_stderr(reader: Option<impl AsyncRead + Unpin>, captured: Arc<Mutex<Captured>>) {
    let Some(mut reader) = reader else {
        return;
    };
    let mut chunk = [0u8; 8192];
    loop {
        match reader.read(&mut chunk).await {
            Ok(0) | Err(_) => return,
            Ok(read) => {
                let mut captured = captured.lock().expect("hook output lock");
                let tail = &mut captured.stderr_tail;
                tail.extend_from_slice(&chunk[..read]);
                let excess = tail.len().saturating_sub(HOOK_STDERR_TAIL);
                if excess > 0 {
                    tail.drain(..excess);
                }
            }
        }
    }
}

/// Makes hook stderr safe to log or show: lossy UTF-8 with every control character other than
/// newline and tab written as `\xNN`, so a script cannot put terminal escape sequences into a
/// log or a notice. The result is cut from the front to [`HOOK_STDERR_TAIL`] bytes, since
/// escaping can lengthen it.
pub fn escape_control(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        if character.is_control() && character != '\n' && character != '\t' {
            escaped.push_str(&format!("\\x{:02x}", character as u32));
        } else {
            escaped.push(character);
        }
    }
    if escaped.len() > HOOK_STDERR_TAIL {
        let mut start = escaped.len() - HOOK_STDERR_TAIL;
        while !escaped.is_char_boundary(start) {
            start += 1;
        }
        escaped.drain(..start);
    }
    escaped
}

/// Cuts `text` to at most `limit` bytes without splitting a character.
fn truncate_to_char_boundary(text: &str, limit: usize) -> &str {
    if text.len() <= limit {
        return text;
    }
    let mut end = limit;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

#[cfg(test)]
#[path = "command_tests.rs"]
mod tests;

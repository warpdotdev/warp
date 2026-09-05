use std::future::Future;
use std::io;
use std::path::Path;
use std::pin::Pin;
use std::process::{Output, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll, Waker};

use command::r#async::Command;
use futures_util::future::{try_join, try_join3};
use futures_util::io::AsyncReadExt;
use parking_lot::Mutex;

pub(crate) use super::kill::ProcessGroupKillOnDrop;
#[cfg(test)]
pub(crate) use super::kill::take_process_group_terminations;
use super::newline::NewlineNormalizer;
use super::operation::DevContainerBuildCancel;

pub(crate) const STDOUT_LIMIT: usize = 1024 * 1024;
const STDERR_TAIL_LIMIT: usize = 8 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PtySize {
    pub columns: u16,
    pub rows: u16,
}

impl Default for PtySize {
    fn default() -> Self {
        Self {
            columns: 80,
            rows: 24,
        }
    }
}

impl PtySize {
    pub(crate) fn from_grid(columns: usize, rows: usize) -> Self {
        Self {
            columns: columns.max(1) as u16,
            rows: rows.max(1) as u16,
        }
    }
}

#[cfg(unix)]
#[derive(Clone)]
pub(crate) struct PtyResizeHandle {
    stdout: Arc<std::fs::File>,
    stderr: Arc<std::fs::File>,
}

#[cfg(not(unix))]
#[derive(Clone)]
pub(crate) struct PtyResizeHandle;

#[cfg(unix)]
impl PtyResizeHandle {
    pub(crate) fn resize(&self, size: PtySize) -> io::Result<()> {
        set_pty_winsize(&*self.stdout, size)?;
        set_pty_winsize(&*self.stderr, size)?;
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn reported_sizes(&self) -> io::Result<(PtySize, PtySize)> {
        Ok((pty_winsize(&*self.stdout)?, pty_winsize(&*self.stderr)?))
    }
}

pub(crate) struct DevContainerUpStdout {
    pub bytes: Vec<u8>,
    pub oversized: bool,
}

pub(crate) struct DevContainerDrain {
    pub stdout: DevContainerUpStdout,
    pub stderr_tail: Vec<u8>,
}

pub(crate) fn dev_container_up_command(
    cli: &Path,
    workspace_folder: &Path,
    config_file: &Path,
    pty_size: PtySize,
) -> Command {
    let mut command = Command::new_with_process_group(cli);
    command
        .arg("up")
        .arg("--workspace-folder")
        .arg(workspace_folder)
        .arg("--config")
        .arg(config_file)
        .arg("--log-format")
        .arg("text")
        .arg("--terminal-columns")
        .arg(pty_size.columns.to_string())
        .arg("--terminal-rows")
        .arg(pty_size.rows.to_string())
        .kill_on_drop(true);
    #[cfg(not(unix))]
    {
        let _ = pty_size;
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
    }
    command
}

#[cfg(test)]
pub(crate) async fn drain_dev_container_child<F>(
    command: Command,
    cancel: Option<&DevContainerBuildCancel>,
    on_output: F,
) -> io::Result<(DevContainerDrain, bool)>
where
    F: FnMut(&[u8]) + Send,
{
    drain_dev_container_child_with_size_and_resize(
        command,
        cancel,
        on_output,
        PtySize::default(),
        None,
    )
    .await
}

pub(crate) async fn drain_dev_container_child_with_size_and_resize<F>(
    mut command: Command,
    cancel: Option<&DevContainerBuildCancel>,
    on_output: F,
    pty_size: PtySize,
    resize_slot: Option<Arc<Mutex<Option<PtyResizeHandle>>>>,
) -> io::Result<(DevContainerDrain, bool)>
where
    F: FnMut(&[u8]) + Send,
{
    #[cfg(unix)]
    {
        let (stdout_master, stderr_master, resize) = attach_stdio_ptys(&mut command, pty_size)?;
        if let Some(slot) = resize_slot {
            *slot.lock() = Some(resize);
        }
        let mut child = command.spawn()?;
        drop(command);
        let process_group_id = child.id();
        let kill_group = ProcessGroupKillOnDrop::new(process_group_id);
        if let Some(cancel) = cancel
            && !cancel.register_kill_group(kill_group.clone())
        {
            kill_group.terminate_now();
            let _ = child.status().await;
            return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"));
        }
        let child_exit = Arc::new(ChildExit::new());
        let drain_fut = drain_dev_container_pipes(
            ExitAwareReader::new(PtyMasterReader(stdout_master), child_exit.clone(), 0),
            ExitAwareReader::new(PtyMasterReader(stderr_master), child_exit.clone(), 1),
            on_output,
        );
        let status_fut = {
            let kill_group = kill_group.clone();
            let child_exit = child_exit.clone();
            async move {
                let status = child.status().await?;
                child_exit.mark();
                kill_group.terminate_now();
                io::Result::Ok(status)
            }
        };
        join_drain_and_status(kill_group, drain_fut, status_fut).await
    }
    #[cfg(not(unix))]
    {
        let _ = pty_size;
        let _ = resize_slot;
        let mut child = command.spawn()?;
        let process_group_id = child.id();
        let kill_group = ProcessGroupKillOnDrop::new(process_group_id);
        if let Some(cancel) = cancel
            && !cancel.register_kill_group(kill_group.clone())
        {
            kill_group.terminate_now();
            let _ = child.status().await;
            return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"));
        }
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("devcontainer up stdout was not piped"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| io::Error::other("devcontainer up stderr was not piped"))?;
        let child_exit = Arc::new(ChildExit::new());
        let drain_fut = drain_dev_container_pipes(
            ExitAwareReader::new(stdout, child_exit.clone(), 0),
            ExitAwareReader::new(stderr, child_exit.clone(), 1),
            on_output,
        );
        let status_fut = {
            let kill_group = kill_group.clone();
            let child_exit = child_exit.clone();
            async move {
                let status = child.status().await?;
                child_exit.mark();
                kill_group.terminate_now();
                io::Result::Ok(status)
            }
        };
        join_drain_and_status(kill_group, drain_fut, status_fut).await
    }
}

async fn join_drain_and_status(
    _kill_group: ProcessGroupKillOnDrop,
    drain_fut: impl Future<Output = io::Result<DevContainerDrain>>,
    status_fut: impl Future<Output = io::Result<std::process::ExitStatus>>,
) -> io::Result<(DevContainerDrain, bool)> {
    let (drain, status) = try_join(drain_fut, status_fut).await?;
    Ok((drain, status.success()))
}

pub(crate) async fn run_cancellable_process_group_command(
    mut command: Command,
    cancel: &DevContainerBuildCancel,
) -> io::Result<Output> {
    command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = command.spawn()?;
    let process_group_id = child.id();
    let kill_group = ProcessGroupKillOnDrop::new(process_group_id);
    if !cancel.register_kill_group(kill_group.clone()) {
        kill_group.terminate_now();
        let _ = child.status().await;
        return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"));
    }
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("stdout was not piped"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| io::Error::other("stderr was not piped"))?;
    let stdout_fut = read_to_end(stdout);
    let stderr_fut = read_to_end(stderr);
    let status_fut = {
        let kill_group = kill_group.clone();
        async move {
            let status = child.status().await?;
            kill_group.terminate_now();
            io::Result::Ok(status)
        }
    };
    let (stdout, stderr, status) = try_join3(stdout_fut, stderr_fut, status_fut).await?;
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

async fn read_to_end<R>(mut reader: R) -> io::Result<Vec<u8>>
where
    R: futures_util::AsyncRead + Unpin,
{
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes).await?;
    Ok(bytes)
}

pub(crate) async fn drain_dev_container_pipes<R1, R2, F>(
    stdout: R1,
    stderr: R2,
    on_output: F,
) -> io::Result<DevContainerDrain>
where
    R1: futures_util::AsyncRead + Unpin,
    R2: futures_util::AsyncRead + Unpin,
    F: FnMut(&[u8]) + Send,
{
    let on_output = Arc::new(Mutex::new(on_output));
    let stdout_task = drain_stdout(stdout, on_output.clone());
    let stderr_task = drain_stderr(stderr, on_output);
    let (stdout, stderr_tail) = try_join(stdout_task, stderr_task).await?;
    Ok(DevContainerDrain {
        stdout,
        stderr_tail,
    })
}

async fn drain_stdout<R, F>(
    mut stdout: R,
    on_output: Arc<Mutex<F>>,
) -> io::Result<DevContainerUpStdout>
where
    R: futures_util::AsyncRead + Unpin,
    F: FnMut(&[u8]),
{
    let mut buf = [0_u8; 8192];
    let mut complete = Vec::new();
    let mut pending = Vec::new();
    let mut oversized = false;
    // Final status JSON is parsed for attach and must not appear in the build pane.
    let mut held_outcome: Option<Vec<u8>> = None;
    let mut normalizer = NewlineNormalizer::new();
    loop {
        let n = stdout.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        if let Some(previous) = held_outcome.take() {
            emit_output(&on_output, &mut normalizer, &previous);
        }
        pending.extend_from_slice(&buf[..n]);
        while let Some(newline_at) = pending.iter().position(|&b| b == b'\n') {
            let record: Vec<u8> = pending.drain(..=newline_at).collect();
            append_complete_stdout_record(&mut complete, &record);
            if let Some(previous) = held_outcome.take() {
                emit_output(&on_output, &mut normalizer, &previous);
            }
            if is_outcome_json_record(&record) {
                held_outcome = Some(record);
            } else {
                emit_output(&on_output, &mut normalizer, &record);
            }
        }
        if !pending.is_empty()
            && let Some(previous) = held_outcome.take()
        {
            emit_output(&on_output, &mut normalizer, &previous);
        }
        if pending.len() > STDOUT_LIMIT {
            oversized = true;
            pending.clear();
        } else if !pending.is_empty() && !could_be_outcome_json_prefix(&pending) {
            append_complete_stdout_record(&mut complete, &pending);
            emit_output(&on_output, &mut normalizer, &pending);
            pending.clear();
        }
    }
    if !pending.is_empty() {
        append_complete_stdout_record(&mut complete, &pending);
        if !is_outcome_json_record(&pending) {
            emit_output(&on_output, &mut normalizer, &pending);
        }
    }
    let trailing = normalizer.finish();
    if !trailing.is_empty() {
        (on_output.lock())(&trailing);
    }
    Ok(DevContainerUpStdout {
        bytes: complete,
        oversized,
    })
}

fn emit_output<F>(on_output: &Arc<Mutex<F>>, normalizer: &mut NewlineNormalizer, bytes: &[u8])
where
    F: FnMut(&[u8]),
{
    if bytes.is_empty() {
        return;
    }
    let normalized = normalizer.push(bytes);
    if !normalized.is_empty() {
        (on_output.lock())(&normalized);
    }
}

fn is_outcome_json_record(record: &[u8]) -> bool {
    let line = String::from_utf8_lossy(record);
    let line = line.trim();
    if line.is_empty() {
        return false;
    }
    serde_json::from_str::<serde_json::Value>(line)
        .ok()
        .and_then(|value| {
            value
                .get("outcome")
                .and_then(|outcome| outcome.as_str())
                .map(|outcome| !outcome.is_empty())
        })
        .unwrap_or(false)
}

fn could_be_outcome_json_prefix(pending: &[u8]) -> bool {
    match pending.iter().find(|&&byte| !matches!(byte, b' ' | b'\t')) {
        Some(&b'{') => true,
        Some(_) => false,
        None => !pending.is_empty(),
    }
}

fn append_complete_stdout_record(complete: &mut Vec<u8>, record: &[u8]) {
    complete.extend_from_slice(record);
    if complete.len() <= STDOUT_LIMIT {
        return;
    }
    let overflow = complete.len() - STDOUT_LIMIT;
    let skip = complete[overflow..]
        .iter()
        .position(|&b| b == b'\n')
        .map(|i| overflow + i + 1)
        .unwrap_or(overflow);
    complete.drain(..skip);
}

async fn drain_stderr<R, F>(mut stderr: R, on_output: Arc<Mutex<F>>) -> io::Result<Vec<u8>>
where
    R: futures_util::AsyncRead + Unpin,
    F: FnMut(&[u8]),
{
    let mut buf = [0_u8; 8192];
    let mut normalizer = NewlineNormalizer::new();
    let mut stderr_tail = Vec::new();
    loop {
        let n = stderr.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        let normalized = normalizer.push(&buf[..n]);
        append_bounded_tail(&mut stderr_tail, &normalized);
        (on_output.lock())(&normalized);
    }
    let trailing = normalizer.finish();
    if !trailing.is_empty() {
        append_bounded_tail(&mut stderr_tail, &trailing);
        (on_output.lock())(&trailing);
    }
    Ok(stderr_tail)
}

#[cfg(test)]
pub(crate) fn transform_dev_container_stderr(chunks: &[&[u8]]) -> Vec<u8> {
    let mut normalizer = NewlineNormalizer::new();
    let mut out = Vec::new();
    for chunk in chunks {
        out.extend(normalizer.push(chunk));
    }
    out.extend(normalizer.finish());
    out
}

fn append_bounded_tail(tail: &mut Vec<u8>, chunk: &[u8]) {
    tail.extend_from_slice(chunk);
    if tail.len() > STDERR_TAIL_LIMIT {
        let overflow = tail.len() - STDERR_TAIL_LIMIT;
        tail.drain(..overflow);
    }
}

struct ChildExit {
    done: AtomicBool,
    wakers: [Mutex<Option<Waker>>; 2],
}

impl ChildExit {
    fn new() -> Self {
        Self {
            done: AtomicBool::new(false),
            wakers: [Mutex::new(None), Mutex::new(None)],
        }
    }

    fn mark(&self) {
        self.done.store(true, Ordering::Release);
        for slot in &self.wakers {
            if let Some(waker) = slot.lock().take() {
                waker.wake();
            }
        }
    }

    fn is_done(&self) -> bool {
        self.done.load(Ordering::Acquire)
    }

    fn register(&self, slot: usize, waker: &Waker) {
        if self.is_done() {
            waker.wake_by_ref();
            return;
        }
        *self.wakers[slot].lock() = Some(waker.clone());
        if self.is_done() {
            waker.wake_by_ref();
        }
    }

    #[cfg(test)]
    fn registered_wakers(&self) -> usize {
        self.wakers
            .iter()
            .filter(|slot| slot.lock().is_some())
            .count()
    }
}

/// Docker can keep a PTY/pipe slave open after `devcontainer` exits; waiting for
/// EIO would hang attach. Once the direct child has exited, a blocked read is EOF.
struct ExitAwareReader<R> {
    inner: R,
    child_exit: Arc<ChildExit>,
    slot: usize,
}

impl<R> ExitAwareReader<R> {
    fn new(inner: R, child_exit: Arc<ChildExit>, slot: usize) -> Self {
        Self {
            inner,
            child_exit,
            slot,
        }
    }
}

impl<R> futures_util::AsyncRead for ExitAwareReader<R>
where
    R: futures_util::AsyncRead + Unpin,
{
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        match Pin::new(&mut self.inner).poll_read(cx, buf) {
            Poll::Ready(Ok(0)) => Poll::Ready(Ok(0)),
            Poll::Ready(result) => Poll::Ready(result),
            Poll::Pending => {
                if self.child_exit.is_done() {
                    Poll::Ready(Ok(0))
                } else {
                    self.child_exit.register(self.slot, cx.waker());
                    Poll::Pending
                }
            }
        }
    }
}

#[cfg(unix)]
pub(crate) fn attach_stdio_ptys(
    command: &mut Command,
    pty_size: PtySize,
) -> io::Result<(
    async_io::Async<std::fs::File>,
    async_io::Async<std::fs::File>,
    PtyResizeHandle,
)> {
    let (stdout_master, stdout_slave, stdout_resize) = open_stdio_pty(pty_size)?;
    let (stderr_master, stderr_slave, stderr_resize) = open_stdio_pty(pty_size)?;
    command
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout_slave))
        .stderr(Stdio::from(stderr_slave))
        .env("TERM", "xterm-256color");
    Ok((
        stdout_master,
        stderr_master,
        PtyResizeHandle {
            stdout: Arc::new(stdout_resize),
            stderr: Arc::new(stderr_resize),
        },
    ))
}

#[cfg(unix)]
fn open_stdio_pty(
    pty_size: PtySize,
) -> io::Result<(async_io::Async<std::fs::File>, std::fs::File, std::fs::File)> {
    use std::os::unix::io::FromRawFd;

    let size = libc::winsize {
        ws_row: pty_size.rows.max(1),
        ws_col: pty_size.columns.max(1),
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    let ends = nix::pty::openpty(Some(&size), None).map_err(io::Error::other)?;
    unsafe {
        libc::fcntl(ends.master, libc::F_SETFD, libc::FD_CLOEXEC);
        libc::fcntl(ends.slave, libc::F_SETFD, libc::FD_CLOEXEC);
        let master = std::fs::File::from_raw_fd(ends.master);
        let slave = std::fs::File::from_raw_fd(ends.slave);
        let resize = dup_cloexec(&master)?;
        Ok((async_io::Async::new(master)?, slave, resize))
    }
}

#[cfg(unix)]
fn dup_cloexec(file: &impl std::os::unix::io::AsRawFd) -> io::Result<std::fs::File> {
    use std::os::unix::io::{FromRawFd, RawFd};

    let fd: RawFd = unsafe { libc::dup(file.as_raw_fd()) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    unsafe {
        libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC);
        Ok(std::fs::File::from_raw_fd(fd))
    }
}

#[cfg(unix)]
fn set_pty_winsize(file: &impl std::os::unix::io::AsRawFd, size: PtySize) -> io::Result<()> {
    let ws = libc::winsize {
        ws_row: size.rows.max(1),
        ws_col: size.columns.max(1),
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    let rc = unsafe { libc::ioctl(file.as_raw_fd(), libc::TIOCSWINSZ, &ws) };
    if rc < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(unix)]
#[cfg(test)]
fn pty_winsize(file: &impl std::os::unix::io::AsRawFd) -> io::Result<PtySize> {
    let mut ws = libc::winsize {
        ws_row: 0,
        ws_col: 0,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    let rc = unsafe { libc::ioctl(file.as_raw_fd(), libc::TIOCGWINSZ, &mut ws) };
    if rc < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(PtySize {
            columns: ws.ws_col,
            rows: ws.ws_row,
        })
    }
}

#[cfg(unix)]
struct PtyMasterReader(async_io::Async<std::fs::File>);

#[cfg(unix)]
impl futures_util::AsyncRead for PtyMasterReader {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut [u8],
    ) -> std::task::Poll<io::Result<usize>> {
        match std::pin::Pin::new(&mut self.get_mut().0).poll_read(cx, buf) {
            std::task::Poll::Ready(Err(error)) if error.raw_os_error() == Some(libc::EIO) => {
                std::task::Poll::Ready(Ok(0))
            }
            other => other,
        }
    }
}

#[cfg(test)]
#[path = "stream_tests.rs"]
mod tests;

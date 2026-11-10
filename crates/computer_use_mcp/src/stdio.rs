//! Serving the MCP server over the process's stdin and stdout.

use anyhow::Context as _;
use rmcp::ServiceExt as _;
use tokio::io::AsyncWrite;

use crate::{ComputerUseMcpConfig, ComputerUseMcpServer};

/// Serves computer-use tools over MCP on stdin and stdout until the client disconnects or the
/// process is asked to terminate.
///
/// Must be called on the main thread: the macOS actor builds its keyboard layout cache on the main
/// thread, which only works while the main thread is running this server's event loop.
pub fn run_stdio(config: ComputerUseMcpConfig) -> anyhow::Result<()> {
    let protocol_output = protocol_output()?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("Failed to start the computer use MCP runtime")?;
    let result = runtime.block_on(serve(config, protocol_output));
    // Reading stdin blocks a runtime thread that can't be cancelled, so don't wait for it.
    runtime.shutdown_background();
    result
}

async fn serve(
    config: ComputerUseMcpConfig,
    output: impl AsyncWrite + Send + Unpin + 'static,
) -> anyhow::Result<()> {
    let running = ComputerUseMcpServer::new(config)
        .serve((tokio::io::stdin(), output))
        .await
        .context("Failed to initialize the computer use MCP session")?;
    tokio::select! {
        quit_reason = running.waiting() => {
            let quit_reason = quit_reason.context("The computer use MCP session failed")?;
            log::info!("Computer use MCP session ended: {quit_reason:?}");
        }
        () = shutdown_signal() => log::info!("Computer use MCP server received a shutdown signal"),
    }
    Ok(())
}

/// Returns the stream to write protocol messages to, after redirecting stdout to stderr so that
/// stray output from this process or its children can't corrupt the protocol stream.
#[cfg(unix)]
fn protocol_output() -> anyhow::Result<tokio::fs::File> {
    use std::os::fd::FromRawFd as _;

    // SAFETY: duplicating and redirecting the standard descriptors has no memory-safety
    // preconditions. `F_DUPFD_CLOEXEC` keeps child processes from inheriting the protocol stream.
    let protocol_fd = unsafe { libc::fcntl(libc::STDOUT_FILENO, libc::F_DUPFD_CLOEXEC, 3) };
    if protocol_fd < 0 {
        return Err(std::io::Error::last_os_error()).context("Failed to duplicate stdout");
    }
    // SAFETY: as above.
    if unsafe { libc::dup2(libc::STDERR_FILENO, libc::STDOUT_FILENO) } < 0 {
        return Err(std::io::Error::last_os_error()).context("Failed to redirect stdout");
    }
    // SAFETY: `protocol_fd` is a newly duplicated descriptor that nothing else owns.
    let file = unsafe { std::fs::File::from_raw_fd(protocol_fd) };
    Ok(tokio::fs::File::from_std(file))
}

#[cfg(not(unix))]
fn protocol_output() -> anyhow::Result<tokio::io::Stdout> {
    Ok(tokio::io::stdout())
}

#[cfg(unix)]
async fn shutdown_signal() {
    use tokio::signal::unix::{SignalKind, signal};

    match signal(SignalKind::terminate()) {
        Ok(mut terminate) => {
            tokio::select! {
                _ = terminate.recv() => {}
                _ = tokio::signal::ctrl_c() => {}
            }
        }
        Err(error) => {
            log::warn!("Failed to listen for SIGTERM: {error}");
            let _ = tokio::signal::ctrl_c().await;
        }
    }
}

#[cfg(not(unix))]
async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}

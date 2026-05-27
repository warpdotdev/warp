//! Unix-specific implementation of the remote server daemon and proxy.
//!
//! - `run_proxy()`: entry point for the `remote-server-proxy` subcommand.
//!   Uses a ControlMaster-like pattern (flock + fork + exec) to daemonize
//!   the server and bridge the SSH stdio channel to its Unix socket.
//!
//! - `run_daemon()`: entry point for the `remote-server-daemon` subcommand.
//!   Binds a Unix domain socket, accepts multiple concurrent proxy connections,
//!   and exits after a grace period with no connections.
//!
//! All platform-specific code is contained here so that the parent `mod.rs`
//! is a thin dispatcher with no Unix assumptions.

pub(super) mod proxy;

use std::fs::Permissions;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use warpui::r#async::executor;
use warpui::SingletonEntity;

use super::server_model::{ConnectionId, ServerModel};
use crate::{send_telemetry_from_app_ctx, TelemetryEvent};

/// Run the `remote-server-daemon` subcommand.
///
/// Delegates to `run_internal` with `LaunchMode::RemoteServerDaemon`.
/// All initialization (feature flags, profiling, logging, resource limits,
/// TLS, `initialize_app`, crash reporting) is handled by `run_internal`.
/// The daemon-specific socket binding and `ServerModel` registration
/// happen in [`launch_daemon`], called from `launch()`.
pub fn run_daemon(identity_key: String) -> anyhow::Result<()> {
    let result = crate::run_internal(crate::LaunchMode::RemoteServerDaemon {
        identity_key: identity_key.clone(),
    });

    // Clean up socket and PID files after the event loop exits.
    let socket_path = proxy::socket_path(&identity_key);
    let pid_path = proxy::pid_path(&identity_key);
    let _ = std::fs::remove_file(&socket_path);
    let _ = std::fs::remove_file(&pid_path);
    log::info!("Daemon exiting");
    result
}

fn daemon_host_id_path(identity_key: &str) -> PathBuf {
    let path = remote_server::setup::remote_server_daemon_host_id_path(identity_key);
    PathBuf::from(shellexpand::tilde(&path).into_owned())
}

fn load_or_create_daemon_host_id(identity_key: &str) -> String {
    load_or_create_daemon_host_id_at_path(&daemon_host_id_path(identity_key))
}

fn load_or_create_daemon_host_id_at_path(host_id_path: &Path) -> String {
    match std::fs::read_to_string(host_id_path) {
        Ok(existing_host_id) => {
            let existing_host_id = existing_host_id.trim();
            if !existing_host_id.is_empty() {
                return existing_host_id.to_string();
            }
            log::warn!(
                "Daemon host ID file is empty, generating a replacement: {}",
                host_id_path.display()
            );
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => {
            log::warn!(
                "Failed to read daemon host ID file, generating a replacement: path={} error={err}",
                host_id_path.display()
            );
        }
    }

    let host_id = uuid::Uuid::new_v4().to_string();
    if let Err(err) = write_daemon_host_id(host_id_path, &host_id) {
        log::warn!(
            "Failed to persist daemon host ID: path={} error={err}",
            host_id_path.display()
        );
    }
    host_id
}

fn write_daemon_host_id(host_id_path: &Path, host_id: &str) -> anyhow::Result<()> {
    if let Some(parent) = host_id_path.parent() {
        proxy::ensure_private_daemon_dir(parent)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        // Set the file mode at creation time so the host ID is never world-readable.
        .mode(0o600)
        .open(host_id_path)?;
    file.set_permissions(Permissions::from_mode(0o600))?;
    writeln!(file, "{host_id}")?;
    Ok(())
}

/// Called from `launch()` inside the headless AppBuilder callback.
/// Binds the Unix domain socket, writes the PID file, spawns the
/// accept loop, and registers the `ServerModel` singleton.
pub(crate) fn launch_daemon(identity_key: &str, ctx: &mut warpui::AppContext) {
    let socket_path = proxy::socket_path(identity_key);
    let pid_path = proxy::pid_path(identity_key);

    if let Some(parent) = socket_path.parent() {
        if let Err(e) = proxy::ensure_private_daemon_dir(parent) {
            log::error!("Failed to create daemon directory: {e}");
            return;
        }
    }
    if socket_path.exists() {
        let _ = std::fs::remove_file(&socket_path);
    }

    let listener = match std::os::unix::net::UnixListener::bind(&socket_path) {
        Ok(l) => l,
        Err(e) => {
            log::error!("Daemon: failed to bind socket: {e}");
            return;
        }
    };
    let _ = std::fs::set_permissions(&socket_path, Permissions::from_mode(0o600));
    listener.set_nonblocking(true).ok();
    log::info!("Daemon bound to {}", socket_path.display());

    // Flush the accumulated IntervalTimer data as telemetry now that the
    // daemon is ready to accept connections. The timer was created in
    // `run_internal` and carries intervals from the full startup path
    // (logging, SQLite, singleton models, etc.).
    //
    // All telemetry dependencies are ready at this point:
    // `AppTelemetryContextProvider` and `AuthStateProvider` are
    // registered during `initialize_app` (before `launch` calls us),
    // and `TelemetryCollector` is already running its periodic flush.
    // The flush sends directly to Rudderstack using a baked-in write
    // key — no user auth token is required.
    let timing_data =
        warp_core::interval_timer::IntervalTimer::handle(ctx).update(ctx, |timer, _| {
            timer.mark_interval_end("DAEMON_SOCKET_BOUND");
            timer.compute_stats()
        });
    send_telemetry_from_app_ctx!(
        TelemetryEvent::RemoteServerDaemonStartup { timing_data },
        ctx
    );

    let _ = std::fs::write(&pid_path, std::process::id().to_string());
    let stable_host_id = load_or_create_daemon_host_id(identity_key);

    ctx.add_singleton_model(move |ctx| {
        let spawner = ctx.spawner();
        let exec = ctx.background_executor();
        let spawner_loop = spawner.clone();
        let background_executor = exec.clone();

        exec.spawn(async move {
            let listener = match async_io::Async::new(listener) {
                Ok(l) => l,
                Err(e) => {
                    log::error!("Daemon: async listener error: {e}");
                    return;
                }
            };
            loop {
                match listener.accept().await {
                    Ok((stream, _)) => {
                        let conn_id = uuid::Uuid::new_v4();
                        log::info!("Daemon: accepted connection {conn_id}");
                        let spawner = spawner_loop.clone();
                        background_executor
                            .spawn(handle_daemon_connection(
                                conn_id,
                                stream,
                                spawner,
                                background_executor.clone(),
                            ))
                            .detach();
                    }
                    Err(e) => log::error!("Daemon: accept error: {e}"),
                }
            }
        })
        .detach();
        ServerModel::new_with_host_id(stable_host_id.clone(), ctx)
    });
}

/// Handles a single Unix socket connection from a proxy process.
///
/// Spawns a dedicated **reader task** that owns the read half of the socket
/// and runs a tight `read_client_message` loop, forwarding each decoded
/// message to `ServerModel` via the spawner.  The reader is never cancelled
/// mid-read, which avoids the framing desynchronisation that would occur if
/// `read_client_message` were polled inside a `select!` branch.
///
/// The calling task becomes the **writer loop**: it drains the per-connection
/// outbound channel (`conn_rx`) and writes each `ServerMessage` to the socket.
/// When the reader exits (EOF / error) it calls `deregister_connection`, which
/// drops `conn_tx` from `ServerModel` and causes `conn_rx` to close, naturally
/// terminating the writer loop.
pub(super) async fn handle_daemon_connection(
    conn_id: ConnectionId,
    stream: async_io::Async<std::os::unix::net::UnixStream>,
    spawner: warpui::ModelSpawner<ServerModel>,
    exec: std::sync::Arc<executor::Background>,
) {
    use futures::io::{AsyncWriteExt, BufReader, BufWriter};
    use futures::AsyncReadExt as _;

    let (conn_tx, conn_rx) = async_channel::unbounded::<remote_server::proto::ServerMessage>();

    // Register with ServerModel (cancels grace timer if running).
    let _ = spawner
        .spawn({
            let conn_tx_reg = conn_tx.clone();
            move |me, ctx| {
                me.register_connection(conn_id, conn_tx_reg, ctx);
            }
        })
        .await;

    let (read_half, write_half) = stream.split();
    let mut writer = BufWriter::new(write_half);

    // ---- Reader task -------------------------------------------------------
    // Owns the read half; dispatches decoded messages to ServerModel.
    // On exit it calls deregister_connection, which drops conn_tx from
    // ServerModel and closes conn_rx, terminating the writer loop below.
    let spawner_reader = spawner.clone();
    exec.spawn(async move {
        let mut reader = BufReader::new(read_half);
        loop {
            match remote_server::protocol::read_client_message(&mut reader).await {
                Ok(msg) => {
                    let result = spawner_reader
                        .spawn(move |me, ctx| {
                            me.handle_message(conn_id, msg, ctx);
                        })
                        .await;
                    if result.is_err() {
                        log::warn!("Daemon: ServerModel dropped, closing conn {conn_id}");
                        break;
                    }
                }
                Err(remote_server::protocol::ProtocolError::UnexpectedEof) => {
                    log::info!("Daemon: proxy {conn_id} disconnected (EOF)");
                    break;
                }
                Err(e) if e.is_read_recoverable() => {
                    log::warn!("Daemon: skipping malformed message from conn {conn_id}: {e}");
                }
                Err(e) => {
                    if is_disconnect_error(&e) {
                        log::warn!(
                            "Daemon: read error from conn {conn_id} (client disconnected): {e}"
                        );
                    } else {
                        log::error!("Daemon: fatal read error from conn {conn_id}: {e}");
                    }
                    break;
                }
            }
        }
        // Deregistering drops conn_tx from ServerModel, closing conn_rx and
        // causing the writer loop to exit naturally.
        let _ = spawner_reader
            .spawn(move |me, ctx| {
                me.deregister_connection(conn_id, ctx);
            })
            .await;
    })
    .detach();

    // ---- Writer loop -------------------------------------------------------
    // Drains outbound messages until conn_rx closes (reader called
    // deregister_connection) or a fatal write error occurs.
    while let Ok(msg) = conn_rx.recv().await {
        if let Err(e) = remote_server::protocol::write_server_message(&mut writer, &msg).await {
            if !e.is_write_recoverable() {
                if is_disconnect_protocol_error(&e) {
                    log::warn!("Daemon: write error on conn {conn_id} (client disconnected): {e}");
                } else {
                    log::error!("Daemon: write error on conn {conn_id}: {e}");
                }
                break;
            }
            // Recoverable write error (e.g. MessageTooLarge): nothing was
            // written to the stream, so it remains aligned. Log and skip
            // rather than tearing down the entire connection.
            log::warn!("Daemon: skipping undeliverable message on conn {conn_id}: {e}");

            // Send an ErrorResponse so the client doesn't hang waiting
            // for a response that will never arrive.
            if msg.request_id.is_empty() {
                continue;
            }
            let error_msg = remote_server::proto::ServerMessage {
                request_id: msg.request_id.clone(),
                message: Some(remote_server::proto::server_message::Message::Error(
                    remote_server::proto::ErrorResponse {
                        code: remote_server::proto::ErrorCode::Internal.into(),
                        message: format!("Response could not be delivered: {e}"),
                    },
                )),
            };
            if let Err(e2) =
                remote_server::protocol::write_server_message(&mut writer, &error_msg).await
            {
                if !e2.is_write_recoverable() {
                    log::error!("Daemon: failed to send error response on conn {conn_id}: {e2}");
                    break;
                }
                log::warn!("Daemon: failed to send error response on conn {conn_id}: {e2}");
                continue;
            }
            // Fall through to flush the error response.
        }
        // Flush after every message so responses reach the proxy without
        // waiting for the BufWriter's internal buffer to fill up.
        if let Err(e) = writer.flush().await {
            if is_disconnect_io_error(&e) {
                log::warn!("Daemon: flush error on conn {conn_id} (client disconnected): {e}");
            } else {
                log::error!("Daemon: flush error on conn {conn_id}: {e}");
            }
            break;
        }
    }

    let _ = writer.flush().await;

    // Deregister in case the writer exited due to a write error before the
    // reader task called deregister. This is a no-op if already deregistered.
    let _ = spawner
        .spawn(move |me, ctx| {
            me.deregister_connection(conn_id, ctx);
        })
        .await;
}

/// Returns `true` if the IO error represents a normal client disconnect.
fn is_disconnect_io_error(e: &std::io::Error) -> bool {
    matches!(
        e.kind(),
        std::io::ErrorKind::BrokenPipe
            | std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::ConnectionAborted
    )
}

/// Returns `true` if the `ProtocolError` wraps a disconnect IO error.
fn is_disconnect_error(e: &remote_server::protocol::ProtocolError) -> bool {
    match e {
        remote_server::protocol::ProtocolError::Io(io_err) => is_disconnect_io_error(io_err),
        _ => false,
    }
}

/// Alias for [`is_disconnect_error`] — used in the write path for clarity.
fn is_disconnect_protocol_error(e: &remote_server::protocol::ProtocolError) -> bool {
    is_disconnect_error(e)
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    fn temp_host_id_path() -> PathBuf {
        std::env::temp_dir()
            .join(format!(
                "warp-remote-server-host-id-{}",
                uuid::Uuid::new_v4()
            ))
            .join("host_id")
    }

    #[test]
    fn load_or_create_daemon_host_id_reuses_existing_id() {
        let path = temp_host_id_path();
        let parent = path.parent().unwrap();
        std::fs::create_dir_all(parent).unwrap();
        std::fs::write(&path, "existing-host-id\n").unwrap();

        assert_eq!(
            load_or_create_daemon_host_id_at_path(&path),
            "existing-host-id"
        );

        std::fs::remove_dir_all(parent).unwrap();
    }

    #[test]
    fn load_or_create_daemon_host_id_persists_generated_id() {
        let path = temp_host_id_path();
        let parent = path.parent().unwrap();

        let host_id = load_or_create_daemon_host_id_at_path(&path);
        assert!(!host_id.is_empty());
        assert_eq!(std::fs::read_to_string(&path).unwrap().trim(), host_id);
        assert_eq!(load_or_create_daemon_host_id_at_path(&path), host_id);

        let parent_permissions = std::fs::metadata(parent).unwrap().permissions().mode() & 0o777;
        let file_permissions = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(parent_permissions, 0o700);
        assert_eq!(file_permissions, 0o600);

        std::fs::remove_dir_all(parent).unwrap();
    }
}

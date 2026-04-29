#[cfg(not(target_family = "wasm"))]
use crate::server::server_api::{ServerApiEvent, ServerApiProvider};
#[cfg(not(target_family = "wasm"))]
use remote_server::manager::RemoteServerManager;
#[cfg(not(target_family = "wasm"))]
use warpui::SingletonEntity;
// Re-export everything from the `remote_server` crate so existing
// `crate::remote_server::*` imports in `app` continue to work.
pub use remote_server::*;

#[cfg(not(target_family = "wasm"))]
pub mod auth_context;
#[cfg(not(target_family = "wasm"))]
pub mod server_model;
#[cfg(not(target_family = "wasm"))]
pub mod ssh_transport;
#[cfg(unix)]
pub mod unix;

/// Run the `remote-server-proxy` subcommand.
#[cfg(unix)]
pub fn run_proxy(identity_key: String) -> anyhow::Result<()> {
    unix::run_proxy(identity_key)
}

#[cfg(not(unix))]
pub fn run_proxy(_identity_key: String) -> anyhow::Result<()> {
    anyhow::bail!("remote-server-proxy is not supported on this platform")
}

/// Run the `remote-server-daemon` subcommand.
#[cfg(unix)]
pub fn run_daemon(identity_key: String) -> anyhow::Result<()> {
    unix::run_daemon(identity_key)
}

#[cfg(not(unix))]
pub fn run_daemon(_identity_key: String) -> anyhow::Result<()> {
    anyhow::bail!("remote-server-daemon is not supported on this platform")
}

/// Start the WarpUI headless app with all daemon singleton models.
///
/// This is the platform-agnostic core of every `run_daemon` implementation.
/// Platform-specific code (Unix sockets, Windows named pipes, …) binds a
/// listener and calls this function with the appropriate `ServerModel`
/// constructor — everything else (DirectoryWatcher, DetectedRepositories,
/// RepoMetadataModel, FileModel) is shared.
///
/// # Example
/// ```ignore
/// // In unix/mod.rs:
/// super::run_daemon_app(move |ctx| ServerModel::new(unix_listener, ctx))
/// ```
#[cfg(not(target_family = "wasm"))]
pub(super) fn run_daemon_app(
    server_model_init: impl FnOnce(&mut warpui::ModelContext<server_model::ServerModel>) -> server_model::ServerModel
        + 'static,
) -> anyhow::Result<()> {
    use warpui::platform::app::AppCallbacks;
    use warpui::platform::AppBuilder;

    AppBuilder::new_headless(AppCallbacks::default(), Box::new(()), None).run(|ctx| {
        // Rotate log files from the previous daemon invocation in the background.
        ctx.background_executor()
            .spawn(warp_logging::rotate_log_files())
            .detach();

        use crate::server::telemetry::context_provider::NoopTelemetryContextProvider;
        use repo_metadata::repositories::DetectedRepositories;
        use repo_metadata::watcher::DirectoryWatcher;
        use repo_metadata::RepoMetadataModel;

        // Register a no-op telemetry context so that `send_telemetry_from_ctx!`
        // calls (e.g. from RepoMetadataModel on ExceededMaxFileLimit) don't
        // panic due to a missing TelemetryContextModel singleton.
        ctx.add_singleton_model(NoopTelemetryContextProvider::new_context_provider);

        // Order matters: DetectedRepositories must be registered before
        // RepoMetadataModel because LocalRepoMetadataModel::new()
        // subscribes to DetectedRepositories::handle(ctx).
        ctx.add_singleton_model(DirectoryWatcher::new);
        ctx.add_singleton_model(|_ctx| DetectedRepositories::default());
        ctx.add_singleton_model(RepoMetadataModel::new_with_incremental_updates);
        ctx.add_singleton_model(warp_files::FileModel::new);
        ctx.add_singleton_model(server_model_init);
    })?;
    Ok(())
}

/// Forwards app auth-token rotation events to the remote-server manager.
#[cfg(not(target_family = "wasm"))]
pub fn wire_auth_token_rotation(ctx: &mut warpui::AppContext) {
    let server_api = ServerApiProvider::handle(ctx);
    let manager = RemoteServerManager::handle(ctx);
    ctx.subscribe_to_model(&server_api, move |_, event, ctx| {
        if let ServerApiEvent::AccessTokenRefreshed { token } = event {
            manager.update(ctx, |manager, _| {
                manager.rotate_auth_token(token.clone());
            });
        }
    });
}

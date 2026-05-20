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
pub mod codebase_index_model;
#[cfg(not(target_family = "wasm"))]
mod codebase_index_status;
pub mod diff_state_proto;
#[cfg(not(target_family = "wasm"))]
pub mod diff_state_tracker;
#[cfg(not(target_family = "wasm"))]
pub mod server_buffer_tracker;
#[cfg(not(target_family = "wasm"))]
pub mod server_model;
#[cfg(not(target_family = "wasm"))]
pub mod ssh_transport;
#[cfg(unix)]
pub mod unix;

/// Run the `remote-server-proxy` subcommand.
#[cfg(unix)]
pub fn run_proxy(identity_key: String) -> anyhow::Result<()> {
    unix::proxy::run(&identity_key)
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

/// Forwards app auth-token rotation and privacy preference change events
/// to the remote-server manager.
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

    // Forward crash reporting preference changes to all connected daemons.
    use crate::settings::{PrivacySettings, PrivacySettingsChangedEvent};
    let privacy_settings = PrivacySettings::handle(ctx);
    let manager = RemoteServerManager::handle(ctx);
    ctx.subscribe_to_model(&privacy_settings, move |_, event, ctx| {
        if let &PrivacySettingsChangedEvent::UpdateIsCrashReportingEnabled { new_value, .. } = event
        {
            for client in manager.as_ref(ctx).all_connected_clients() {
                client.update_preferences(new_value);
            }
        }
    });
}

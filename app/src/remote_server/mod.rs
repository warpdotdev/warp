#[cfg(not(target_family = "wasm"))]
use remote_server::manager::RemoteServerManager;
// Re-export everything from the `remote_server` crate so existing
// `crate::remote_server::*` imports in `app` continue to work.
pub use remote_server::*;
#[cfg(not(target_family = "wasm"))]
use warp_server_client::auth::AuthEvent;
#[cfg(not(target_family = "wasm"))]
use warpui::SingletonEntity as _;

#[cfg(not(target_family = "wasm"))]
use crate::ai::{AIRequestUsageModel, AIRequestUsageModelEvent};
#[cfg(not(target_family = "wasm"))]
use crate::server::server_api::ServerApiProvider;

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
pub mod git_status_proto;
#[cfg(not(target_family = "wasm"))]
pub(crate) mod handoff_snapshot;
#[cfg(not(target_family = "wasm"))]
pub mod server_buffer_tracker;
#[cfg(not(target_family = "wasm"))]
pub mod server_model;
#[cfg(not(target_family = "wasm"))]
pub mod ssh_transport;
#[cfg(unix)]
pub mod unix;

#[cfg(not(target_family = "wasm"))]
fn current_codebase_index_limits(
    ctx: &warpui::AppContext,
) -> remote_server::proto::CodebaseIndexLimits {
    let limits = AIRequestUsageModel::as_ref(ctx).codebase_context_limits();
    remote_server::proto::CodebaseIndexLimits {
        max_indices_allowed: limits.max_indices_allowed.map(|limit| limit as u64),
        max_files_per_repo: limits.max_files_per_repo as u64,
        embedding_generation_batch_size: limits.embedding_generation_batch_size as u64,
    }
}

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
    let codebase_index_limits = current_codebase_index_limits(ctx);
    RemoteServerManager::handle(ctx).update(ctx, |manager, _| {
        manager.update_codebase_index_limits(Some(codebase_index_limits));
    });
    let server_api = ServerApiProvider::handle(ctx);
    let manager = RemoteServerManager::handle(ctx);
    ctx.subscribe_to_model(&server_api, move |_, event, ctx| {
        if let AuthEvent::AccessTokenRefreshed { token } = event {
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
            let codebase_index_limits = current_codebase_index_limits(ctx);
            manager.update(ctx, |manager, _| {
                manager.update_codebase_index_limits(Some(codebase_index_limits));
            });
            for client in manager.as_ref(ctx).all_connected_clients() {
                client.update_preferences(new_value, Some(codebase_index_limits));
            }
        }
    });

    let request_usage = AIRequestUsageModel::handle(ctx);
    let manager = RemoteServerManager::handle(ctx);
    ctx.subscribe_to_model(&request_usage, move |_, event, ctx| {
        if matches!(event, AIRequestUsageModelEvent::RequestUsageUpdated) {
            let codebase_index_limits = current_codebase_index_limits(ctx);
            let crash_reporting_enabled = PrivacySettings::as_ref(ctx).is_crash_reporting_enabled;
            manager.update(ctx, |manager, _| {
                manager.update_codebase_index_limits(Some(codebase_index_limits));
                for client in manager.all_connected_clients() {
                    client.update_preferences(crash_reporting_enabled, Some(codebase_index_limits));
                }
            });
        }
    });
}

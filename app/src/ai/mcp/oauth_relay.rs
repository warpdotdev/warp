//! Desktop callback relay for cross-process MCP OAuth coordination.
//!
//! When a `warp://mcp/oauth2callback` is delivered to a process that is not the
//! current OAuth leader (it has no matching local CSRF mapping), that process
//! forwards the URL to the leader over the existing `crates/ipc` transport. The
//! leader started a short-lived relay service ([`McpCallbackRelayServiceImpl]])
//! at an instance-derived address and published that address as owner metadata
//! (see `crates/mcp::oauth::coordinator`). The relay reuses the manager's
//! existing `handle_oauth_callback` CSRF validation and delivers exactly one
//! result to the leader's local OAuth channel.
//!
//! This is desktop-only: the TUI loopback path receives callbacks on the
//! leader's own `127.0.0.1` socket, so no relay is needed.

use std::sync::Arc;

use anyhow::Context as _;
use ipc::{Client, ConnectionAddress, ServerBuilder, Service, ServiceImpl, service_caller};
use mcp::oauth::{
    CredentialNamespace, OAuthCoordinator, OwnerRelay, forwarded_callback_looks_valid,
};
use parking_lot::Mutex;
use url::Url;
use uuid::Uuid;
use warp_core::channel::ChannelState;
use warpui::r#async::executor::Background;
use warpui::{AppContext, ModelSpawner};

use crate::ai::mcp::TemplatableMCPServerManager;

/// Marker type for the IPC `Service` trait that routes forwarded MCP OAuth
/// callbacks to the leader's relay. It carries no data — `service_caller::<McpCallbackRelay>`
/// uses it only as the compile-time service key, and [`McpCallbackRelayServiceImpl`]
/// provides the actual request handling. Kept as a unit struct so the app crate
/// compiles clean under `-D warnings` (a single-variant enum would be dead code).
#[derive(Debug, Clone, Copy)]
pub struct McpCallbackRelay;

impl Service for McpCallbackRelay {
    type Request = String;
    type Response = bool;
}

/// Server-side implementation of [`McpCallbackRelay`]. It forwards each
/// incoming URL to the leader's own `handle_oauth_callback`, which validates the
/// CSRF state against `pending_oauth_csrf` and delivers the result to the
/// leader's local OAuth channel.
#[derive(Clone)]
struct McpCallbackRelayServiceImpl {
    spawner: ModelSpawner<TemplatableMCPServerManager>,
}

#[async_trait::async_trait]
impl ServiceImpl for McpCallbackRelayServiceImpl {
    type Service = McpCallbackRelay;

    async fn handle_request(&self, url: String) -> bool {
        let Ok(parsed) = Url::parse(&url) else {
            return false;
        };
        match self
            .spawner
            .spawn(move |manager, _ctx| manager.handle_oauth_callback(&parsed))
            .await
        {
            Ok(Ok(())) => true,
            Ok(Err(err)) => {
                log::debug!("MCP OAuth relay rejected forwarded callback: {err:#}");
                false
            }
            Err(err) => {
                log::debug!("MCP OAuth relay spawner dropped: {err:?}");
                false
            }
        }
    }
}

/// Desktop [`OwnerRelay`] implementation: starts a short-lived `crates/ipc`
/// server at a channel-scoped address and publishes it as owner metadata so
/// other processes can forward callbacks to this leader.
pub struct DesktopOwnerRelay {
    spawner: ModelSpawner<TemplatableMCPServerManager>,
    background_executor: Arc<Background>,
    server: Mutex<Option<ipc::Server>>,
    address: Mutex<Option<String>>,
}

impl DesktopOwnerRelay {
    pub fn new(
        spawner: ModelSpawner<TemplatableMCPServerManager>,
        background_executor: Arc<Background>,
    ) -> Self {
        Self {
            spawner,
            background_executor,
            server: Mutex::new(None),
            address: Mutex::new(None),
        }
    }
}

impl OwnerRelay for DesktopOwnerRelay {
    fn start(&self, _csrf_state: String) -> anyhow::Result<String> {
        // The CSRF state is validated by the manager's `handle_oauth_callback`
        // when a forwarded callback arrives, so the relay itself only needs the
        // spawner. The address is channel-scoped and unique per leader flow so a
        // promoted successor never collides with a stale predecessor.
        let address = format!(
            "WarpMcpOauthRelay-{:?}-{}",
            ChannelState::channel(),
            Uuid::new_v4().simple()
        );
        let service = McpCallbackRelayServiceImpl {
            spawner: self.spawner.clone(),
        };
        let (server, _) = ServerBuilder::default()
            .with_fixed_address(address.clone())
            .with_service(service)
            .build_and_run(self.background_executor.clone())
            .map_err(|err| anyhow::anyhow!("{err:?}"))
            .context("failed to start MCP OAuth callback relay")?;
        *self.server.lock() = Some(server);
        *self.address.lock() = Some(address.clone());
        Ok(address)
    }

    fn stop(&self) {
        // Dropping the server cancels its background tasks; the OS reclaims the
        // socket/pipe. Owner metadata is removed by the coordinator's
        // `LeaderGuard` on the leader's terminal path.
        *self.server.lock() = None;
        *self.address.lock() = None;
    }
}

/// Forwards a callback URL to the current OAuth leader over the relay. Tries
/// each published owner (across both credential namespaces) until exactly one —
/// the leader whose CSRF matches — accepts. Returns `Ok(())` if a leader
/// accepted, or an error if no leader could be reached.
pub async fn forward_oauth_callback_to_leader(
    url: &Url,
    background_executor: Arc<Background>,
) -> anyhow::Result<()> {
    let channel = format!("{:?}", ChannelState::channel());
    let url_string = url.to_string();
    for namespace in [
        CredentialNamespace::Templatable,
        CredentialNamespace::FileBased,
    ] {
        for metadata in OAuthCoordinator::read_all_owner_metadata(&channel, namespace) {
            if !forwarded_callback_looks_valid(&url_string, metadata.installation_uuid) {
                continue;
            }
            match call_relay(
                &metadata.endpoint_address,
                &url_string,
                background_executor.clone(),
            )
            .await
            {
                Ok(true) => return Ok(()),
                Ok(false) => continue,
                Err(err) => {
                    log::debug!(
                        "MCP OAuth callback relay {} unreachable or errored: {err:#}",
                        metadata.endpoint_address
                    );
                    continue;
                }
            }
        }
    }
    anyhow::bail!("no MCP OAuth leader accepted the forwarded callback")
}

/// Connects to the relay at `address` and asks the leader to handle `url`.
async fn call_relay(
    address: &str,
    url: &str,
    background_executor: Arc<Background>,
) -> anyhow::Result<bool> {
    let client = Arc::new(
        Client::connect(
            ConnectionAddress::from(address.to_string()),
            background_executor,
        )
        .await
        .map_err(|err| anyhow::anyhow!("{err:?}"))
        .context("failed to connect to MCP OAuth relay")?,
    );
    let caller = service_caller::<McpCallbackRelay>(client);
    caller
        .call(url.to_string())
        .await
        .map_err(|err| anyhow::anyhow!("{err:?}"))
        .context("MCP OAuth relay call failed")
}

/// Spawns a background task that forwards a callback URL to the current leader.
/// Used by the desktop URI handler when this process has no matching local
/// OAuth flow (it is a follower). Fire-and-forget: the leader completes the
/// flow and publishes credentials, which this process then reloads.
pub fn spawn_forward_oauth_callback(url: Url, ctx: &AppContext) {
    let executor = ctx.background_executor();
    let task_executor = executor.clone();
    executor
        .spawn(async move {
            if let Err(err) = forward_oauth_callback_to_leader(&url, task_executor).await {
                log::warn!("Failed to forward MCP OAuth callback to leader: {err:#}");
            }
        })
        .detach();
}

/// Returns `true` if this process has a pending local OAuth flow for the
/// callback's CSRF state (i.e. it is the leader for this flow). The desktop URI
/// handler uses this to decide whether to handle the callback locally or
/// forward it to the leader.
pub fn process_has_local_oauth_flow(manager: &TemplatableMCPServerManager, url: &Url) -> bool {
    let Some(state) = url
        .query_pairs()
        .find(|(k, _)| k == "state")
        .map(|(_, v)| v.into_owned())
    else {
        return false;
    };
    manager.has_pending_oauth_csrf_state(&state)
}

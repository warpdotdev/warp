//! A peer wrapper that transparently handles reconnection when the underlying transport is closed.

use std::future::Future;

use uuid::Uuid;
use warpui::ModelSpawner;

use super::TemplatableMCPServerManager;

/// A wrapper around an MCP server connection that transparently handles reconnection.
///
/// When making requests (e.g., `call_tool` or `read_resource`), this type checks if the
/// underlying transport is closed and automatically triggers reconnection before retrying
/// the request.
#[derive(Clone)]
pub struct ReconnectingPeer {
    installation_uuid: Uuid,
    spawner: ModelSpawner<TemplatableMCPServerManager>,
}

/// Error type for reconnecting peer operations.
#[derive(Debug, thiserror::Error)]
pub enum ReconnectingPeerError {
    #[error("Service error: {0}")]
    Service(#[from] rmcp::ServiceError),
    #[error("Reconnection failed: {0}")]
    ReconnectionFailed(String),
    #[error("Model dropped")]
    ModelDropped,
}

impl From<ReconnectingPeerError> for rmcp::ServiceError {
    fn from(e: ReconnectingPeerError) -> Self {
        rmcp::ServiceError::McpError(rmcp::model::ErrorData {
            code: rmcp::model::ErrorCode::INTERNAL_ERROR,
            message: e.to_string().into(),
            data: None,
        })
    }
}

impl ReconnectingPeer {
    /// Creates a new `ReconnectingPeer` with the given installation UUID and spawner.
    pub fn new(
        installation_uuid: Uuid,
        spawner: ModelSpawner<TemplatableMCPServerManager>,
    ) -> Self {
        Self {
            installation_uuid,
            spawner,
        }
    }

    /// Gets the current peer if connected, or triggers reconnection and waits for it.
    async fn get_connected_peer(
        &self,
    ) -> Result<rmcp::Peer<rmcp::RoleClient>, ReconnectingPeerError> {
        let installation_uuid = self.installation_uuid;

        // First, check if we have a connected peer.
        let peer_result = self
            .spawner
            .spawn(move |manager, _ctx| manager.get_peer_if_connected(installation_uuid))
            .await
            .map_err(|_| ReconnectingPeerError::ModelDropped)?;

        if let Some(peer) = peer_result {
            return Ok(peer);
        }

        // Peer is not connected, trigger reconnection.
        log::debug!("Triggering reconnection for MCP server {installation_uuid}");
        let (tx, rx) = tokio::sync::oneshot::channel();

        self.spawner
            .spawn(move |manager, ctx| {
                manager.reconnect_server(installation_uuid, tx, ctx);
            })
            .await
            .map_err(|_| ReconnectingPeerError::ModelDropped)?;

        // Wait for reconnection to complete.
        let peer = rx
            .await
            .map_err(|_| ReconnectingPeerError::ReconnectionFailed("Channel closed".to_string()))?
            .map_err(|e| ReconnectingPeerError::ReconnectionFailed(e.to_string()))?;

        log::debug!("Reconnection completed for MCP server {installation_uuid}");
        Ok(peer)
    }

    /// Executes a request with automatic retry on `TransportClosed` errors.
    ///
    /// If the initial request fails with `TransportClosed`, the reconnecting peer will
    /// detect the closed transport and attempt to reconnect before the retry.
    ///
    /// Note: We intentionally retry only once to avoid infinite reconnection loops if the
    /// server is persistently failing. If the retry also fails, the error propagates to the
    /// caller.
    async fn with_reconnect_retry<T, R, F, Fut>(
        &self,
        params: T,
        f: F,
    ) -> Result<R, rmcp::ServiceError>
    where
        T: Clone,
        F: Fn(rmcp::Peer<rmcp::RoleClient>, T) -> Fut,
        Fut: Future<Output = Result<R, rmcp::ServiceError>>,
    {
        let peer = self.get_connected_peer().await?;
        match f(peer, params.clone()).await {
            Err(rmcp::ServiceError::TransportClosed) => {
                let peer = self.get_connected_peer().await?;
                f(peer, params).await
            }
            result => result,
        }
    }

    /// Calls a tool on the MCP server.
    pub async fn call_tool(
        &self,
        params: rmcp::model::CallToolRequestParams,
    ) -> Result<rmcp::model::CallToolResult, rmcp::ServiceError> {
        self.with_reconnect_retry(params, |peer, p| async move { peer.call_tool(p).await })
            .await
    }

    /// Reads a resource from the MCP server.
    pub async fn read_resource(
        &self,
        params: rmcp::model::ReadResourceRequestParams,
    ) -> Result<rmcp::model::ReadResourceResult, rmcp::ServiceError> {
        self.with_reconnect_retry(params, |peer, p| async move { peer.read_resource(p).await })
            .await
    }
}

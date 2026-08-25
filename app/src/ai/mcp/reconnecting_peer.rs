//! A peer wrapper that transparently handles reconnection when the underlying transport is closed.

use std::future::Future;

use mcp::error_classification::{McpErrorClass, classify_service_error, is_safe_to_resend};
use uuid::Uuid;
use warp_core::features::FeatureFlag;
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

    /// Executes a request with automatic retry on recoverable transport errors.
    ///
    /// If the initial request fails in a way a reconnect can fix (closed
    /// transport, or — with `McpSelfHeal` — a recoverable send failure), the
    /// reconnecting peer triggers reconnection before the retry.
    ///
    /// Note: We intentionally retry only once to avoid infinite reconnection loops if the
    /// server is persistently failing. If the retry also fails, the error propagates to the
    /// caller. The manager additionally applies a per-server backoff to repeated
    /// reconnect failures.
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
            Err(error) if should_retry_after_reconnect(&error) => {
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

#[cfg(test)]
mod tests {
    use mcp::error_classification::ProxyAuthReason;
    use rmcp::transport::DynamicTransportError;
    use rmcp::transport::streamable_http_client::{AuthRequiredError, StreamableHttpError};

    use super::*;

    fn send_error(error: StreamableHttpError<reqwest::Error>) -> rmcp::ServiceError {
        rmcp::ServiceError::TransportSend(DynamicTransportError::from_parts(
            "test-transport",
            std::any::TypeId::of::<()>(),
            Box::new(error),
        ))
    }

    fn expired_proxy_token_error() -> rmcp::ServiceError {
        send_error(StreamableHttpError::AuthRequired(AuthRequiredError::new(
            r#"Bearer error="invalid_token", error_description="proxy_token_expired""#.to_string(),
        )))
    }

    #[test]
    fn transport_closed_always_retries() {
        let _flag = FeatureFlag::McpSelfHeal.override_enabled(false);
        assert!(should_retry_after_reconnect(
            &rmcp::ServiceError::TransportClosed
        ));
    }

    #[test]
    fn recoverable_send_failures_retry_only_with_the_flag() {
        {
            let _flag = FeatureFlag::McpSelfHeal.override_enabled(true);
            assert!(should_retry_after_reconnect(&expired_proxy_token_error()));
            // Sanity-check the classification the decision builds on.
            assert!(matches!(
                classify_service_error(&expired_proxy_token_error()),
                McpErrorClass::AuthExpiredRecoverable(ProxyAuthReason::ProxyTokenExpired)
            ));
        }
        let _flag = FeatureFlag::McpSelfHeal.override_enabled(false);
        assert!(!should_retry_after_reconnect(&expired_proxy_token_error()));
    }

    #[test]
    fn possibly_executed_or_user_fixable_failures_never_retry() {
        let _flag = FeatureFlag::McpSelfHeal.override_enabled(true);
        // A 5xx after the server accepted the request may have run the tool.
        assert!(!should_retry_after_reconnect(&send_error(
            StreamableHttpError::UnexpectedServerResponse(
                "HTTP 500 Internal Server Error: boom".to_string().into()
            )
        )));
        // A downstream auth rejection needs the user, not a retry.
        assert!(!should_retry_after_reconnect(&send_error(
            StreamableHttpError::UnexpectedServerResponse(
                "HTTP 401 Unauthorized: nope".to_string().into()
            )
        )));
        // Timeouts mean the request was delivered; don't double-execute.
        assert!(!should_retry_after_reconnect(
            &rmcp::ServiceError::Timeout {
                timeout: std::time::Duration::from_secs(1)
            }
        ));
    }
}

/// Whether a failed request should be retried after reconnecting.
///
/// `TransportClosed` always retries (pre-existing behavior). With
/// `McpSelfHeal` enabled, send-stage failures also retry when the request
/// provably never executed (no double-running non-idempotent tools) and the
/// failure is one a reconnect can fix — a transient transport error or an
/// expired Warp proxy session that reconnection re-mints.
fn should_retry_after_reconnect(error: &rmcp::ServiceError) -> bool {
    if matches!(error, rmcp::ServiceError::TransportClosed) {
        return true;
    }
    if !FeatureFlag::McpSelfHeal.is_enabled() {
        return false;
    }
    if !is_safe_to_resend(error) {
        return false;
    }
    matches!(
        classify_service_error(error),
        McpErrorClass::Transient | McpErrorClass::AuthExpiredRecoverable(_)
    )
}

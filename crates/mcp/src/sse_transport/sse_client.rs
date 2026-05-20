// This file contains code copied from the rmcp crate (https://github.com/modelcontextprotocol/rust-sdk),
// originally located at `crates/rmcp/src/transport/sse_client.rs`.
// Used under the terms of the Apache License, Version 2.0.
// See https://github.com/modelcontextprotocol/rust-sdk/blob/main/LICENSE for the full license text.
//
// Reference: <https://html.spec.whatwg.org/multipage/server-sent-events.html>

use std::{
    pin::Pin,
    sync::{Arc, RwLock},
};

use futures::{future::BoxFuture, StreamExt};
use http::Uri;
use rmcp::{
    model::{ClientJsonRpcMessage, ServerJsonRpcMessage},
    transport::Transport,
    RoleClient,
};
use sse_stream::{Error as SseError, Sse};
use thiserror::Error;

use super::client_side_sse::{
    BoxedSseResponse, SseAutoReconnectStream, SseRetryPolicy, SseStreamReconnect,
};

#[derive(Error, Debug)]
pub enum SseTransportError<E: std::error::Error + Send + Sync + 'static> {
    #[error("SSE error: {0}")]
    Sse(#[from] SseError),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Client error: {0}")]
    Client(E),
    #[error("unexpected end of stream")]
    UnexpectedEndOfStream,
    #[error("Unexpected content type: {0:?}")]
    UnexpectedContentType(Option<String>),
    #[error("Auth error: {0}")]
    Auth(#[from] rmcp::transport::AuthError),
    #[error("Invalid uri: {0}")]
    InvalidUri(#[from] http::uri::InvalidUri),
    #[error("Invalid uri parts: {0}")]
    InvalidUriParts(#[from] http::uri::InvalidUriParts),
}

pub trait SseClient: Clone + Send + Sync + 'static {
    type Error: std::error::Error + Send + Sync + 'static;
    fn post_message(
        &self,
        uri: Uri,
        message: ClientJsonRpcMessage,
        auth_token: Option<String>,
    ) -> impl std::future::Future<Output = Result<(), SseTransportError<Self::Error>>> + Send + '_;
    fn get_stream(
        &self,
        uri: Uri,
        last_event_id: Option<String>,
        auth_token: Option<String>,
    ) -> impl std::future::Future<Output = Result<BoxedSseResponse, SseTransportError<Self::Error>>>
           + Send
           + '_;
}

/// Helper that refreshes the POST endpoint whenever the server emits
/// control frames during SSE reconnect; used together with
/// [`SseAutoReconnectStream`].
struct SseClientReconnect<C> {
    pub client: C,
    pub uri: Uri,
    pub message_endpoint: Arc<RwLock<Uri>>,
}

impl<C: SseClient> SseStreamReconnect for SseClientReconnect<C> {
    type Error = SseTransportError<C::Error>;
    type Future = BoxFuture<'static, Result<BoxedSseResponse, Self::Error>>;
    fn retry_connection(&mut self, last_event_id: Option<&str>) -> Self::Future {
        let client = self.client.clone();
        let uri = self.uri.clone();
        let last_event_id = last_event_id.map(|s| s.to_owned());
        Box::pin(async move { client.get_stream(uri, last_event_id, None).await })
    }

    fn handle_control_event(&mut self, event: &Sse) -> Result<(), Self::Error> {
        if event.event.as_deref() != Some("endpoint") {
            return Ok(());
        }
        let Some(data) = event.data.as_ref() else {
            return Ok(());
        };
        // Servers typically resend the message POST endpoint (often with a new
        // sessionId) when a stream reconnects. Reuse `message_endpoint` helper
        // to resolve it and update the shared URI.
        let new_endpoint = message_endpoint(self.uri.clone(), data.clone())
            .map_err(SseTransportError::InvalidUri)?;
        *self
            .message_endpoint
            .write()
            .expect("message endpoint lock poisoned") = new_endpoint;
        Ok(())
    }

    fn handle_stream_error(
        &mut self,
        error: &(dyn std::error::Error + 'static),
        last_event_id: Option<&str>,
    ) {
        tracing::warn!(
            uri = %self.uri,
            last_event_id = last_event_id.unwrap_or(""),
            "sse stream error: {error}"
        );
    }
}
type ServerMessageStream<C> = Pin<Box<SseAutoReconnectStream<SseClientReconnect<C>>>>;

/// A client-agnostic SSE transport for MCP that supports Server-Sent Events.
///
/// This transport allows you to choose your preferred HTTP client implementation
/// by implementing the [`SseClient`] trait. The transport handles SSE streaming
/// and automatic reconnection.
pub struct SseClientTransport<C: SseClient> {
    client: C,
    config: SseClientConfig,
    /// Current POST endpoint; refreshed when the server sends new endpoint
    /// control frames.
    message_endpoint: Arc<RwLock<Uri>>,
    stream: Option<ServerMessageStream<C>>,
}

impl<C: SseClient> Transport<RoleClient> for SseClientTransport<C> {
    type Error = SseTransportError<C::Error>;
    async fn receive(&mut self) -> Option<ServerJsonRpcMessage> {
        self.stream.as_mut()?.next().await?.ok()
    }
    fn send(
        &mut self,
        item: rmcp::service::TxJsonRpcMessage<RoleClient>,
    ) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send + 'static {
        let client = self.client.clone();
        let message_endpoint = self.message_endpoint.clone();
        async move {
            let uri = {
                let guard = message_endpoint
                    .read()
                    .expect("message endpoint lock poisoned");
                guard.clone()
            };
            client.post_message(uri, item, None).await
        }
    }
    async fn close(&mut self) -> Result<(), Self::Error> {
        self.stream.take();
        Ok(())
    }
}

impl<C: SseClient + std::fmt::Debug> std::fmt::Debug for SseClientTransport<C> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SseClientTransport")
            .field("client", &self.client)
            .field("config", &self.config)
            .finish()
    }
}

impl<C: SseClient> SseClientTransport<C> {
    pub async fn start_with_client(
        client: C,
        config: SseClientConfig,
    ) -> Result<Self, SseTransportError<C::Error>> {
        let sse_endpoint = config.sse_endpoint.as_ref().parse::<http::Uri>()?;

        let mut sse_stream = client.get_stream(sse_endpoint.clone(), None, None).await?;
        let initial_message_endpoint = if let Some(endpoint) = config.use_message_endpoint.clone() {
            let ep = endpoint.parse::<http::Uri>()?;
            let mut sse_endpoint_parts = sse_endpoint.clone().into_parts();
            sse_endpoint_parts.path_and_query = ep.into_parts().path_and_query;
            Uri::from_parts(sse_endpoint_parts)?
        } else {
            // Wait for the endpoint event.
            loop {
                let sse = sse_stream
                    .next()
                    .await
                    .ok_or(SseTransportError::UnexpectedEndOfStream)??;
                let Some("endpoint") = sse.event.as_deref() else {
                    continue;
                };
                let ep = sse.data.unwrap_or_default();

                break message_endpoint(sse_endpoint.clone(), ep)?;
            }
        };
        let message_endpoint = Arc::new(RwLock::new(initial_message_endpoint));

        let stream = Box::pin(SseAutoReconnectStream::new(
            sse_stream,
            SseClientReconnect {
                client: client.clone(),
                uri: sse_endpoint.clone(),
                message_endpoint: message_endpoint.clone(),
            },
            config.retry_policy.clone(),
        ));
        Ok(Self {
            client,
            config,
            message_endpoint,
            stream: Some(stream),
        })
    }
}

fn message_endpoint(base: http::Uri, endpoint: String) -> Result<http::Uri, http::uri::InvalidUri> {
    // If endpoint is a full URL, parse and return it directly.
    if endpoint.starts_with("http://") || endpoint.starts_with("https://") {
        return endpoint.parse::<http::Uri>();
    }

    let mut base_parts = base.into_parts();
    let endpoint_clone = endpoint.clone();

    if endpoint.starts_with("?") {
        // Query only - keep base path and append query.
        if let Some(base_path_and_query) = &base_parts.path_and_query {
            let base_path = base_path_and_query.path();
            base_parts.path_and_query = Some(format!("{base_path}{endpoint}").parse()?);
        } else {
            base_parts.path_and_query = Some(format!("/{endpoint}").parse()?);
        }
    } else {
        // Path (with optional query) - replace entire path_and_query.
        let path_to_use = if endpoint.starts_with("/") {
            endpoint // Use absolute path as-is.
        } else {
            format!("/{endpoint}") // Make relative path absolute.
        };
        base_parts.path_and_query = Some(path_to_use.parse()?);
    }

    http::Uri::from_parts(base_parts).map_err(|_| endpoint_clone.parse::<http::Uri>().unwrap_err())
}

#[derive(Debug, Clone)]
pub struct SseClientConfig {
    /// The client SSE endpoint URL.
    pub sse_endpoint: Arc<str>,
    pub retry_policy: Arc<dyn SseRetryPolicy>,
    /// If this is set, the client will use this endpoint to send messages and
    /// skip waiting for the endpoint event.
    pub use_message_endpoint: Option<String>,
}

impl Default for SseClientConfig {
    fn default() -> Self {
        Self {
            sse_endpoint: "".into(),
            retry_policy: Arc::new(super::client_side_sse::FixedInterval::default()),
            use_message_endpoint: None,
        }
    }
}

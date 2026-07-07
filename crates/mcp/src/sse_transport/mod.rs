/// Legacy SSE client transport for MCP, preserved from the rmcp fork after upstream
/// removed SSE transport support in v0.11.0. This allows Warp to continue connecting
/// to MCP servers that only support the older SSE protocol.
mod auth_impl;
mod client_side_sse;
mod reqwest_impl;
mod sse_client;

pub use client_side_sse::{ExponentialBackoff, FixedInterval, NeverRetry, SseRetryPolicy};
pub use sse_client::{SseClient, SseClientConfig, SseClientTransport, SseTransportError};

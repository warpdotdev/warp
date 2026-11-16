use anyhow::Result;
use async_trait::async_trait;

/// This trait is used to abstract over the underlying transport mechanisms.
/// The official jsonrpc specification does not discuss the transport layer.
#[async_trait]
pub trait Transport: Send + Sync {
    /// Reads one complete JSON-RPC message from the transport.
    /// Returns an empty string on EOF.
    async fn read(&self) -> Result<String>;

    /// Writes one complete JSON-RPC message to the transport with appropriate framing.
    async fn write(&self, message: &str) -> Result<()>;

    //// Closes/shutsdown the transport
    async fn shutdown(&self, timeout: std::time::Duration) -> Result<()>;
}

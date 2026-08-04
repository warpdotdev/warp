// Mock MCP backends are only resolved from agent run flows, which don't run on WASM.
#![cfg_attr(target_family = "wasm", expect(dead_code))]

use anyhow::Result;
use async_trait::async_trait;
use cynic::MutationBuilder;
#[cfg(test)]
use mockall::automock;
use warp_graphql::mutations::create_mock_mcp_client_config::{
    CreateMockMcpClientConfig, CreateMockMcpClientConfigInput, CreateMockMcpClientConfigVariables,
    MockMcpClientConfig,
};

use super::ServerApi;

#[cfg_attr(test, automock)]
#[cfg_attr(not(target_family = "wasm"), async_trait)]
#[cfg_attr(target_family = "wasm", async_trait(?Send))]
pub trait MockMcpClient: 'static + Send + Sync {
    /// Resolves a mock MCP backend to a URL-backed connection. `template` is a
    /// managed MCP warp_id or well-known integration id (e.g. "linear") whose
    /// stored tool catalog the mock endpoint mirrors; `model_id` is an optional
    /// user-facing model alias for the LLM that generates tool responses.
    async fn create_mock_mcp_client_config(
        &self,
        template: String,
        model_id: Option<String>,
    ) -> Result<MockMcpClientConfig>;
}

#[cfg_attr(not(target_family = "wasm"), async_trait)]
#[cfg_attr(target_family = "wasm", async_trait(?Send))]
impl MockMcpClient for ServerApi {
    async fn create_mock_mcp_client_config(
        &self,
        template: String,
        model_id: Option<String>,
    ) -> Result<MockMcpClientConfig> {
        let variables = CreateMockMcpClientConfigVariables {
            input: CreateMockMcpClientConfigInput { template, model_id },
        };
        let operation = CreateMockMcpClientConfig::build(variables);
        let response = self.send_graphql_request(operation, None).await?;
        Ok(response.create_mock_mcp_client_config)
    }
}

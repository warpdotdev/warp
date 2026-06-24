// We don't resolve managed MCPs from agent run CLI flows on WASM, so this code is unused there.
#![cfg_attr(target_family = "wasm", expect(dead_code))]

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use cynic::MutationBuilder;
#[cfg(test)]
use mockall::automock;
use uuid::Uuid;
use warp_graphql::mutations::create_managed_mcp_client_config::{
    CreateManagedMcpClientConfig, CreateManagedMcpClientConfigInput,
    CreateManagedMcpClientConfigOutput, CreateManagedMcpClientConfigResult,
    CreateManagedMcpClientConfigVariables,
};

use super::ServerApi;
use crate::server::graphql::{get_request_context, get_user_facing_error_message};

#[cfg_attr(test, automock)]
#[cfg_attr(not(target_family = "wasm"), async_trait)]
#[cfg_attr(target_family = "wasm", async_trait(?Send))]
pub trait ManagedMcpClient: 'static + Send + Sync {
    async fn create_managed_mcp_client_config(
        &self,
        uid: Uuid,
    ) -> Result<CreateManagedMcpClientConfigOutput>;
}

#[cfg_attr(not(target_family = "wasm"), async_trait)]
#[cfg_attr(target_family = "wasm", async_trait(?Send))]
impl ManagedMcpClient for ServerApi {
    async fn create_managed_mcp_client_config(
        &self,
        uid: Uuid,
    ) -> Result<CreateManagedMcpClientConfigOutput> {
        let variables = CreateManagedMcpClientConfigVariables {
            input: CreateManagedMcpClientConfigInput {
                uid: cynic::Id::new(uid.to_string()),
            },
            request_context: get_request_context(),
        };
        let operation = CreateManagedMcpClientConfig::build(variables);
        let response = self.send_graphql_request(operation, None).await?;

        match response.create_managed_mcp_client_config {
            CreateManagedMcpClientConfigResult::CreateManagedMcpClientConfigOutput(output) => {
                Ok(output)
            }
            CreateManagedMcpClientConfigResult::UserFacingError(error) => {
                Err(anyhow!(get_user_facing_error_message(error)))
            }
            CreateManagedMcpClientConfigResult::Unknown => Err(anyhow!(
                "Unknown error while creating managed MCP client config"
            )),
        }
    }
}

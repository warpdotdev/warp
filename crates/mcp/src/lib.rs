#[cfg(not(target_family = "wasm"))]
pub mod oauth;
#[cfg(not(target_family = "wasm"))]
pub mod runtime;
#[cfg(not(target_family = "wasm"))]
pub mod sse_transport;

use uuid::Uuid;

/// Information about a single connected MCP server.
pub struct TemplatableMCPServerInfo {
    name: String,
    service: rmcp::service::RunningService<
        rmcp::RoleClient,
        Box<dyn rmcp::service::DynService<rmcp::RoleClient>>,
    >,
    resources: Vec<rmcp::model::Resource>,
    tools: Vec<rmcp::model::Tool>,
    installation_id: Uuid,
    description: Option<String>,
    /// Whether the underlying transport uses authentication.
    ///
    /// TODO(vorporeal): Use this to display a toast when server authentication and connection is complete, and
    /// to provide a "log out" button.
    #[allow(dead_code)]
    is_authenticated_transport: bool,
}

impl TemplatableMCPServerInfo {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn resources(&self) -> &Vec<rmcp::model::Resource> {
        &self.resources
    }

    pub fn tools(&self) -> &Vec<rmcp::model::Tool> {
        &self.tools
    }

    pub fn installation_id(&self) -> Uuid {
        self.installation_id
    }

    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    pub fn peer(&self) -> rmcp::Peer<rmcp::RoleClient> {
        self.service.clone()
    }

    pub fn peer_if_connected(&self) -> Option<rmcp::Peer<rmcp::RoleClient>> {
        if self.service.is_transport_closed() {
            None
        } else {
            Some(self.service.clone())
        }
    }

    pub fn has_tool(&self, tool_name: &str) -> bool {
        self.tools.iter().any(|tool| tool.name == tool_name)
    }

    pub fn has_resource(&self, resource: &rmcp::model::Resource) -> bool {
        self.resources
            .iter()
            .any(|other_resource| resource.uri == other_resource.uri)
    }

    pub fn has_resource_name_or_uri(&self, name: &str, uri: Option<&str>) -> bool {
        self.resources.iter().any(|resource| {
            if let Some(uri) = uri {
                resource.uri == uri
            } else {
                resource.name == name
            }
        })
    }

    pub fn tool_input_schema(
        &self,
        tool_name: &str,
    ) -> Option<std::sync::Arc<rmcp::model::JsonObject>> {
        self.tools
            .iter()
            .find(|tool| tool.name == tool_name)
            .map(|tool| tool.input_schema.clone())
    }

    pub async fn shutdown(self) -> Result<rmcp::service::QuitReason, tokio::task::JoinError> {
        self.service.cancel().await
    }
}

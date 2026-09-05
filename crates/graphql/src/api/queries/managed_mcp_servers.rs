use crate::error::UserFacingError;
use crate::request_context::RequestContext;
use crate::schema;

/// A GraphQL query to list managed MCP servers visible to the current user.
#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "RootQuery", variables = "ManagedMcpServersVariables")]
pub struct ManagedMcpServersQuery {
    #[arguments(input: $input, requestContext: $request_context)]
    pub managed_mcp_servers: ManagedMcpServersResult,
}

crate::client::define_operation! {
    managed_mcp_servers(ManagedMcpServersVariables) -> ManagedMcpServersQuery;
}

#[derive(cynic::QueryVariables, Debug)]
pub struct ManagedMcpServersVariables {
    pub input: ManagedMcpServersInput,
    pub request_context: RequestContext,
}

#[derive(cynic::InputObject, Debug, Default)]
pub struct ManagedMcpServersInput {
    pub owner_scope: Option<ManagedMcpOwnerScope>,
    pub team_uid: Option<cynic::Id>,
}

/// Who owns a managed MCP server: an individual user or a team.
#[derive(cynic::Enum, Clone, Copy, Debug, Eq, PartialEq)]
pub enum ManagedMcpOwnerScope {
    User,
    Team,
}

/// Lifecycle status of a managed MCP server.
#[derive(cynic::Enum, Clone, Copy, Debug, Eq, PartialEq)]
pub enum ManagedMcpStatus {
    Draft,
    Active,
    Error,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct ManagedMcpServer {
    pub uid: cynic::Id,
    pub display_name: String,
    pub owner_scope: ManagedMcpOwnerScope,
    pub team_uid: Option<cynic::Id>,
    pub status: ManagedMcpStatus,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct ManagedMcpServersOutput {
    pub servers: Vec<ManagedMcpServer>,
}

#[derive(cynic::InlineFragments, Debug)]
pub enum ManagedMcpServersResult {
    ManagedMcpServersOutput(ManagedMcpServersOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

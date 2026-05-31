use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use instant::Duration;
use warp_graphql::client::RequestOptions;

use crate::auth::AgentIdentity;

/// Application-provided transport and platform capabilities used by extracted server API clients.
///
/// The base client keeps UI/model reactions, ambient-agent integration, and transport
/// construction outside extracted endpoint implementations.
#[cfg_attr(not(target_family = "wasm"), async_trait)]
#[cfg_attr(target_family = "wasm", async_trait(?Send))]
pub trait BaseClient: Send + Sync {
    /// Returns the HTTP transport used to send server API requests.
    ///
    /// Extracted client implementations should use this client rather than
    /// constructing their own transport so application-level HTTP setup remains shared.
    fn http_client(&self) -> Arc<http_client::Client>;

    /// Returns the anonymous installation identifier used to correlate unauthenticated requests.
    ///
    /// Endpoint implementations should add this identifier to requests whose
    /// protocol includes anonymous experiment or pre-login identity handling.
    fn anonymous_id(&self) -> String;

    /// Returns GraphQL request options for a request that does not use the logged-in credentials.
    ///
    /// Clients may extend these options with request-specific headers or tokens, such
    /// as the explicit token supplied while fetching a newly authenticated user.
    fn unauthenticated_graphql_request_options(&self) -> RequestOptions;

    /// Returns GraphQL request options for an authenticated operation.
    ///
    /// Extracted GraphQL clients should use this method through the shared request
    /// helper so timeouts and application-owned headers remain centralized.
    async fn graphql_request_options(&self, timeout: Option<Duration>) -> Result<RequestOptions>;

    /// Lists public agent identities available to API-key creation flows.
    ///
    /// This is a base capability until its public REST endpoint is extracted alongside
    /// the GraphQL-backed API client methods that consume its result.
    async fn list_agent_identities(&self) -> Result<Vec<AgentIdentity>>;

    /// Returns an ambient workload token when the current runtime supports issuing one.
    ///
    /// Extracted clients surface this for ambient-agent authentication while leaving
    /// workload-token caching and platform integration in the application.
    async fn get_or_create_ambient_workload_token(&self) -> Result<Option<String>>;

    /// Returns whether authentication failures may be handled as refreshable user-session failures.
    ///
    /// The shared GraphQL request helper uses this distinction to avoid emitting
    /// user-session events for externally managed credentials.
    fn is_auth_refresh_allowed(&self) -> bool;

    /// Notifies the application that a GraphQL request was blocked by staging access controls.
    ///
    /// The shared request helper calls this hook instead of depending on application event types.
    fn on_graphql_staging_access_blocked(&self);

    /// Notifies the application that a GraphQL request received an IAP challenge.
    ///
    /// The shared request helper calls this hook so the application can refresh
    /// its IAP state without exposing that state to extracted clients.
    fn on_graphql_iap_challenge_received(&self);

    /// Notifies the application that a GraphQL response indicates a disabled user account.
    ///
    /// The shared request helper only invokes this for refreshable user sessions;
    /// callers using externally managed credentials receive an authentication error instead.
    fn on_graphql_user_account_disabled(&self);
}

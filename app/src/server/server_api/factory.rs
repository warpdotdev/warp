use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use cynic::{MutationBuilder, QueryBuilder};
#[cfg(test)]
use mockall::automock;
use serde::Deserialize;
use warp_graphql::mutations::delete_runner::{
    DeleteRunner, DeleteRunnerInput, DeleteRunnerResult, DeleteRunnerVariables,
};
use warp_graphql::mutations::upsert_runner::{
    UpsertRunner, UpsertRunnerInput, UpsertRunnerResult, UpsertRunnerVariables,
};
use warp_graphql::queries::get_runners::{
    GetRunners, GetRunnersResult, GetRunnersVariables, Runner, RunnerSortBy,
};

use super::ServerApi;
use crate::ChannelState;
use crate::server::graphql::{get_request_context, get_user_facing_error_message};

/// The result of upserting a runner: the resulting [`Runner`] plus whether the
/// operation updated an existing runner (vs. creating a new one).
// `upsert_runner`/`delete_runner` back CLI commands that aren't built for wasm, so
// this type is unused there while `get_runners` still powers the runner picker.
#[cfg_attr(target_family = "wasm", allow(dead_code))]
pub struct UpsertedRunner {
    pub runner: Runner,
    pub is_update: bool,
}

/// Response from `GET /api/v1/factory/access`, matching the same contract the Platform web
/// client relies on to gate Factory access (see APP-5583).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub struct FactoryAccessResponse {
    pub allowed: bool,
}

/// Request timeout for the Factory access probe, matching the Platform web client's own
/// request timeout for the same endpoint.
const FACTORY_ACCESS_REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

/// Client for the Factory GraphQL surface (runner CRUD) plus the REST Factory access probe.
#[cfg_attr(test, automock)]
#[cfg_attr(not(target_family = "wasm"), async_trait)]
#[cfg_attr(target_family = "wasm", async_trait(?Send))]
pub trait FactoryClient: 'static + Send + Sync {
    /// Fetch all runners visible to the caller, optionally sorted.
    async fn get_runners(&self, sort_by: Option<RunnerSortBy>) -> Result<Vec<Runner>>;

    /// Create or update a runner. `input.uid` is `None` for a create and
    /// `Some(_)` for an update; this single method backs both CLI commands.
    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    async fn upsert_runner(&self, input: UpsertRunnerInput) -> Result<UpsertedRunner>;

    /// Delete a runner by UID, returning the deleted UID on success.
    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    async fn delete_runner(&self, uid: String) -> Result<String>;

    /// Checks whether the signed-in viewer has Factory access, via `GET
    /// /api/v1/factory/access`. Used to decide whether cloud-run links route to Platform or
    /// stay on Oz for the rest of the authenticated session (APP-5583); this is the
    /// authoritative policy boundary, so callers must not reconstruct it from experiments.
    async fn get_factory_access(&self) -> Result<FactoryAccessResponse>;
}

#[cfg_attr(not(target_family = "wasm"), async_trait)]
#[cfg_attr(target_family = "wasm", async_trait(?Send))]
impl FactoryClient for ServerApi {
    async fn get_runners(&self, sort_by: Option<RunnerSortBy>) -> Result<Vec<Runner>> {
        let operation = GetRunners::build(GetRunnersVariables {
            request_context: get_request_context(),
            sort_by,
        });
        let response = self.send_graphql_request(operation, None).await?;
        match response.get_runners {
            GetRunnersResult::GetRunnersOutput(output) => Ok(output.runners),
            GetRunnersResult::UserFacingError(e) => Err(anyhow!(get_user_facing_error_message(e))),
            GetRunnersResult::Unknown => Err(anyhow!("failed to list runners")),
        }
    }

    async fn upsert_runner(&self, input: UpsertRunnerInput) -> Result<UpsertedRunner> {
        let operation = UpsertRunner::build(UpsertRunnerVariables {
            input,
            request_context: get_request_context(),
        });
        let response = self.send_graphql_request(operation, None).await?;
        match response.upsert_runner {
            UpsertRunnerResult::UpsertRunnerOutput(output) => Ok(UpsertedRunner {
                runner: output.runner,
                is_update: output.is_update,
            }),
            UpsertRunnerResult::UserFacingError(e) => {
                Err(anyhow!(get_user_facing_error_message(e)))
            }
            UpsertRunnerResult::Unknown => Err(anyhow!("failed to upsert runner")),
        }
    }

    async fn delete_runner(&self, uid: String) -> Result<String> {
        let operation = DeleteRunner::build(DeleteRunnerVariables {
            input: DeleteRunnerInput {
                uid: cynic::Id::new(uid),
            },
            request_context: get_request_context(),
        });
        let response = self.send_graphql_request(operation, None).await?;
        match response.delete_runner {
            DeleteRunnerResult::DeleteRunnerOutput(output) => {
                Ok(output.deleted_uid.inner().to_string())
            }
            DeleteRunnerResult::UserFacingError(e) => {
                Err(anyhow!(get_user_facing_error_message(e)))
            }
            DeleteRunnerResult::Unknown => Err(anyhow!("failed to delete runner")),
        }
    }

    async fn get_factory_access(&self) -> Result<FactoryAccessResponse> {
        let auth_token = self
            .get_or_refresh_access_token()
            .await
            .context("Failed to get access token for Factory access request")?;

        let url = format!("{}/api/v1/factory/access", ChannelState::server_root_url());
        let mut request = self
            .base_client
            .http_client()
            .get(&url)
            .timeout(FACTORY_ACCESS_REQUEST_TIMEOUT);
        if let Some(token) = auth_token.as_bearer_token() {
            request = request.bearer_auth(token);
        }

        let response = request
            .send()
            .await
            .with_context(|| format!("Failed to send Factory access request to {url}"))?;

        if !response.status().is_success() {
            return Err(Self::error_from_response(response).await);
        }

        response
            .json::<FactoryAccessResponse>()
            .await
            .with_context(|| format!("Failed to deserialize Factory access response from {url}"))
    }
}

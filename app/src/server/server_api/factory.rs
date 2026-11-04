use anyhow::{Result, anyhow};
use async_trait::async_trait;
use cynic::{MutationBuilder, QueryBuilder};
#[cfg(test)]
use mockall::automock;
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
use crate::server::graphql::{get_request_context, get_user_facing_error_message};
use crate::server::team_scope::RequestTeamScope;

/// The result of upserting a runner: the resulting [`Runner`] plus whether the
/// operation updated an existing runner (vs. creating a new one).
// `upsert_runner`/`delete_runner` back CLI commands that aren't built for wasm, so
// this type is unused there while `get_runners` still powers the runner picker.
#[cfg_attr(target_family = "wasm", allow(dead_code))]
pub struct UpsertedRunner {
    pub runner: Runner,
    pub is_update: bool,
}

#[cfg_attr(not(feature = "tui"), allow(dead_code))]
#[derive(serde::Serialize, serde::Deserialize)]
pub struct FactoryGitTokenResponse {
    pub token: String,
    expires_at: chrono::DateTime<chrono::Utc>,
}

impl FactoryGitTokenResponse {
    #[cfg_attr(not(feature = "tui"), allow(dead_code))]
    pub fn expires_in(&self, delta: chrono::TimeDelta) -> bool {
        self.expires_at < chrono::Utc::now() + delta
    }
}

/// Client for the Factory CRUD API surface:
/// - GraphQL for runners
/// - REST for git tokens
#[cfg_attr(test, automock)]
#[cfg_attr(not(target_family = "wasm"), async_trait)]
#[cfg_attr(target_family = "wasm", async_trait(?Send))]
pub trait FactoryClient: 'static + Send + Sync {
    /// Fetch all runners visible to the caller, optionally sorted.
    async fn get_runners(
        &self,
        sort_by: Option<RunnerSortBy>,
        team_scope: Option<RequestTeamScope>,
    ) -> Result<Vec<Runner>>;

    /// Create or update a runner. `input.uid` is `None` for a create and
    /// `Some(_)` for an update; this single method backs both CLI commands.
    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    async fn upsert_runner(
        &self,
        input: UpsertRunnerInput,
        team_scope: Option<RequestTeamScope>,
    ) -> Result<UpsertedRunner>;

    /// Delete a runner by UID, returning the deleted UID on success.
    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    async fn delete_runner(&self, uid: String) -> Result<String>;

    /// Mint a git access jwt token for the definition of a factory
    #[cfg_attr(not(feature = "tui"), allow(dead_code))]
    async fn get_factory_git_token(&self, uid: String) -> Result<FactoryGitTokenResponse>;
}

#[cfg_attr(not(target_family = "wasm"), async_trait)]
#[cfg_attr(target_family = "wasm", async_trait(?Send))]
impl FactoryClient for ServerApi {
    async fn get_runners(
        &self,
        sort_by: Option<RunnerSortBy>,
        team_scope: Option<RequestTeamScope>,
    ) -> Result<Vec<Runner>> {
        let operation = GetRunners::build(GetRunnersVariables {
            request_context: get_request_context(),
            sort_by,
        });
        let response = match team_scope {
            Some(team_scope) => {
                self.send_graphql_request_for_team(operation, team_scope)
                    .await?
            }
            None => self.send_graphql_request(operation, None).await?,
        };
        match response.get_runners {
            GetRunnersResult::GetRunnersOutput(output) => Ok(output.runners),
            GetRunnersResult::UserFacingError(e) => Err(anyhow!(get_user_facing_error_message(e))),
            GetRunnersResult::Unknown => Err(anyhow!("failed to list runners")),
        }
    }

    async fn upsert_runner(
        &self,
        input: UpsertRunnerInput,
        team_scope: Option<RequestTeamScope>,
    ) -> Result<UpsertedRunner> {
        let operation = UpsertRunner::build(UpsertRunnerVariables {
            input,
            request_context: get_request_context(),
        });
        let response = match team_scope {
            Some(team_scope) => {
                self.send_graphql_request_for_team(operation, team_scope)
                    .await?
            }
            None => self.send_graphql_request(operation, None).await?,
        };
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

    async fn get_factory_git_token(&self, uid: String) -> Result<FactoryGitTokenResponse> {
        let response: FactoryGitTokenResponse = self
            .post_public_api(
                &format!("factory/{}/git-token", uid),
                &serde_json::json!({}),
            )
            .await?;
        Ok(response)
    }
}

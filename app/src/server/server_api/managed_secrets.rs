use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use cynic::{MutationBuilder, QueryBuilder};
use warp_graphql::managed_secrets::{ManagedSecret, ManagedSecretType};
use warp_graphql::mutations::create_managed_secret::{
    CreateManagedSecret, CreateManagedSecretInput, CreateManagedSecretResult,
    CreateManagedSecretVariables,
};
use warp_graphql::mutations::delete_managed_secret::{
    DeleteManagedSecret, DeleteManagedSecretInput, DeleteManagedSecretResult,
    DeleteManagedSecretVariables,
};
use warp_graphql::mutations::issue_task_identity_token::{
    IssueTaskIdentityToken, IssueTaskIdentityTokenInput, IssueTaskIdentityTokenResult,
    IssueTaskIdentityTokenVariables,
};
use warp_graphql::mutations::update_managed_secret::{
    UpdateManagedSecret, UpdateManagedSecretInput, UpdateManagedSecretResult,
    UpdateManagedSecretVariables,
};
use warp_graphql::object_permissions::{Owner, OwnerType};
use warp_graphql::queries::list_harness_auth_secrets::{
    ListHarnessAuthSecrets, ListHarnessAuthSecretsInput, ListHarnessAuthSecretsVariables,
};
use warp_graphql::queries::list_managed_secrets::{
    ListManagedSecrets, ListManagedSecretsVariables, ManagedSecretsInput, ManagedSecretsResult,
};
use warp_graphql::queries::managed_secret_config::{
    GetManagedSecretConfig, GetManagedSecretConfigVariables, UserResult,
};
use warp_graphql::queries::task_secrets::{
    ManagedSecretValue, TaskSecrets, TaskSecretsInput, TaskSecretsResult, TaskSecretsVariables,
};
pub use warp_managed_secrets::client::{ManagedSecretConfigs, ManagedSecretsClient};
use warp_managed_secrets::client::{SecretOwner, TaskIdentityToken};

use super::ServerApi;
use crate::server::graphql::{get_request_context, get_user_facing_error_message};
use crate::server::team_scope::RequestTeamScope;

struct ManagedSecretsTransport<'a> {
    api: &'a ServerApi,
    team_scope: Option<RequestTeamScope>,
}

impl<'a> ManagedSecretsTransport<'a> {
    fn new(api: &'a ServerApi, team_scope: Option<RequestTeamScope>) -> Self {
        Self { api, team_scope }
    }

    async fn send_graphql_request<QF, O>(&self, operation: O) -> Result<QF>
    where
        O: warp_graphql::client::Operation<QF> + Send,
    {
        match self.team_scope {
            Some(team_scope) => {
                self.api
                    .send_graphql_request_for_team(operation, team_scope, None)
                    .await
            }
            None => self.api.send_graphql_request(operation, None).await,
        }
    }
    async fn get_managed_secret_configs(&self) -> Result<ManagedSecretConfigs> {
        let variables = GetManagedSecretConfigVariables {
            request_context: get_request_context(),
        };
        let operation = GetManagedSecretConfig::build(variables);
        let response = self.send_graphql_request(operation).await?;

        match response.user {
            UserResult::UserOutput(output) => {
                let mut team_configs = HashMap::new();
                for workspace in output.user.workspaces {
                    for team in workspace.teams {
                        if let Some(config) = team.managed_secrets {
                            // DO NOT inline the `insert` call into the `debug_assert!` macro. It will get compiled out in release builds.
                            let prior_config = team_configs.insert(team.uid.into_inner(), config);
                            debug_assert!(
                                prior_config.is_none(),
                                "Duplicate team UID returned from server"
                            );
                        }
                    }
                }
                Ok(ManagedSecretConfigs {
                    user_secrets: output.user.managed_secrets,
                    team_secrets: team_configs,
                })
            }
            UserResult::UserFacingError(error) => {
                Err(anyhow!(get_user_facing_error_message(error)))
            }
            UserResult::Unknown => Err(anyhow!(
                "Unknown error while getting managed secret configs"
            )),
        }
    }

    async fn create_managed_secret(
        &self,
        owner: SecretOwner,
        name: String,
        secret_type: ManagedSecretType,
        encrypted_value: String,
        description: Option<String>,
    ) -> Result<ManagedSecret> {
        let graphql_owner = match owner {
            SecretOwner::CurrentUser => Owner {
                type_: OwnerType::User,
                uid: None,
            },
            SecretOwner::Team { team_uid } => Owner {
                type_: OwnerType::Team,
                uid: Some(cynic::Id::new(team_uid)),
            },
        };

        let variables = CreateManagedSecretVariables {
            input: CreateManagedSecretInput {
                description,
                encrypted_value,
                name,
                owner: graphql_owner,
                type_: secret_type,
            },
            request_context: get_request_context(),
        };
        let operation = CreateManagedSecret::build(variables);
        let response = self.send_graphql_request(operation).await?;

        match response.create_managed_secret {
            CreateManagedSecretResult::CreateManagedSecretOutput(output) => {
                Ok(output.managed_secret)
            }
            CreateManagedSecretResult::UserFacingError(error) => {
                Err(anyhow!(get_user_facing_error_message(error)))
            }
            CreateManagedSecretResult::Unknown => {
                Err(anyhow!("Unknown error while creating managed secret"))
            }
        }
    }

    async fn delete_managed_secret(&self, owner: SecretOwner, name: String) -> Result<()> {
        let graphql_owner = match owner {
            SecretOwner::CurrentUser => Owner {
                type_: OwnerType::User,
                uid: None,
            },
            SecretOwner::Team { team_uid } => Owner {
                type_: OwnerType::Team,
                uid: Some(cynic::Id::new(team_uid)),
            },
        };

        let variables = DeleteManagedSecretVariables {
            input: DeleteManagedSecretInput {
                name,
                owner: graphql_owner,
            },
            request_context: get_request_context(),
        };
        let operation = DeleteManagedSecret::build(variables);
        let response = self.send_graphql_request(operation).await?;

        match response.delete_managed_secret {
            DeleteManagedSecretResult::DeleteManagedSecretOutput(_) => Ok(()),
            DeleteManagedSecretResult::UserFacingError(error) => {
                Err(anyhow!(get_user_facing_error_message(error)))
            }
            DeleteManagedSecretResult::Unknown => {
                Err(anyhow!("Unknown error while deleting managed secret"))
            }
        }
    }

    async fn update_managed_secret(
        &self,
        owner: SecretOwner,
        name: String,
        encrypted_value: Option<String>,
        description: Option<String>,
    ) -> Result<ManagedSecret> {
        let graphql_owner = match owner {
            SecretOwner::CurrentUser => Owner {
                type_: OwnerType::User,
                uid: None,
            },
            SecretOwner::Team { team_uid } => Owner {
                type_: OwnerType::Team,
                uid: Some(cynic::Id::new(team_uid)),
            },
        };

        let variables = UpdateManagedSecretVariables {
            input: UpdateManagedSecretInput {
                name,
                owner: graphql_owner,
                encrypted_value,
                description,
            },
            request_context: get_request_context(),
        };
        let operation = UpdateManagedSecret::build(variables);
        let response = self.send_graphql_request(operation).await?;

        match response.update_managed_secret {
            UpdateManagedSecretResult::UpdateManagedSecretOutput(output) => {
                Ok(output.managed_secret)
            }
            UpdateManagedSecretResult::UserFacingError(error) => {
                Err(anyhow!(get_user_facing_error_message(error)))
            }
            UpdateManagedSecretResult::Unknown => {
                Err(anyhow!("Unknown error while updating managed secret"))
            }
        }
    }

    async fn list_harness_auth_secrets(
        &self,
        harness: warp_graphql::ai::AgentHarness,
    ) -> Result<Vec<ManagedSecret>> {
        let Some(harness_input) = Option::<
            warp_graphql::queries::list_harness_auth_secrets::AgentHarnessInput,
        >::from(harness) else {
            return Ok(vec![]);
        };
        let variables = ListHarnessAuthSecretsVariables {
            input: ListHarnessAuthSecretsInput {
                harness: harness_input,
            },
            request_context: get_request_context(),
        };
        let operation = ListHarnessAuthSecrets::build(variables);
        let response = self.send_graphql_request(operation).await?;

        match response.harness_auth_secrets {
            warp_graphql::queries::list_harness_auth_secrets::HarnessAuthSecretsResult::HarnessAuthSecretsOutput(output) => {
                Ok(output.managed_secrets)
            }
            warp_graphql::queries::list_harness_auth_secrets::HarnessAuthSecretsResult::UserFacingError(error) => {
                Err(anyhow!(get_user_facing_error_message(error)))
            }
            warp_graphql::queries::list_harness_auth_secrets::HarnessAuthSecretsResult::Unknown => {
                Err(anyhow!("Unknown error while listing harness auth secrets"))
            }
        }
    }

    async fn list_secrets(&self) -> Result<Vec<ManagedSecret>> {
        let variables = ListManagedSecretsVariables {
            // Pagination over managed secrets is not yet supported.
            input: ManagedSecretsInput { cursor: None },
            request_context: get_request_context(),
        };
        let operation = ListManagedSecrets::build(variables);
        let response = self.send_graphql_request(operation).await?;

        match response.managed_secrets {
            ManagedSecretsResult::ManagedSecretsOutput(output) => Ok(output.managed_secrets),
            ManagedSecretsResult::UserFacingError(error) => {
                Err(anyhow!(get_user_facing_error_message(error)))
            }
            ManagedSecretsResult::Unknown => {
                Err(anyhow!("Unknown error while listing managed secrets"))
            }
        }
    }

    async fn get_task_secrets(
        &self,
        task_id: String,
        workload_token: String,
    ) -> Result<HashMap<String, ManagedSecretValue>> {
        let variables = TaskSecretsVariables {
            input: TaskSecretsInput {
                task_id: cynic::Id::new(task_id),
                workload_token,
            },
            request_context: get_request_context(),
        };
        let operation = TaskSecrets::build(variables);
        let response = self.send_graphql_request(operation).await?;

        match response.task_secrets {
            TaskSecretsResult::TaskSecretsOutput(output) => {
                let mut secrets = HashMap::new();
                for entry in output.secrets {
                    secrets.insert(entry.name, entry.value);
                }
                Ok(secrets)
            }
            TaskSecretsResult::UserFacingError(error) => {
                Err(anyhow!(get_user_facing_error_message(error)))
            }
            TaskSecretsResult::Unknown => Err(anyhow!("Unknown error while getting task secrets")),
        }
    }

    async fn issue_task_identity_token(
        &self,
        options: warp_managed_secrets::client::IdentityTokenOptions,
    ) -> Result<TaskIdentityToken> {
        let requested_duration_seconds = options
            .requested_duration
            .as_secs()
            .try_into()
            .context("Requested duration out of bounds")?;
        let variables = IssueTaskIdentityTokenVariables {
            input: IssueTaskIdentityTokenInput {
                audience: options.audience,
                requested_duration_seconds,
                subject_template: Some(options.subject_template.into_vec()),
            },
            request_context: get_request_context(),
        };
        let operation = IssueTaskIdentityToken::build(variables);
        let response = self.send_graphql_request(operation).await?;

        match response.issue_task_identity_token {
            IssueTaskIdentityTokenResult::IssueTaskIdentityTokenOutput(output) => {
                Ok(TaskIdentityToken {
                    token: output.token,
                    expires_at: output.expires_at.utc(),
                    issuer: output.issuer,
                })
            }
            IssueTaskIdentityTokenResult::UserFacingError(error) => {
                Err(anyhow!(get_user_facing_error_message(error)))
            }
            IssueTaskIdentityTokenResult::Unknown => {
                Err(anyhow!("Unknown error while issuing task identity token"))
            }
        }
    }
}

trait ManagedSecretsTransportProvider: Send + Sync {
    fn managed_secrets_transport(&self) -> ManagedSecretsTransport<'_>;
}

impl ManagedSecretsTransportProvider for ServerApi {
    fn managed_secrets_transport(&self) -> ManagedSecretsTransport<'_> {
        ManagedSecretsTransport::new(self, None)
    }
}

pub(crate) struct ScopedManagedSecretsClient {
    api: Arc<ServerApi>,
    team_scope: RequestTeamScope,
}

impl ScopedManagedSecretsClient {
    pub(crate) fn new(api: Arc<ServerApi>, team_scope: RequestTeamScope) -> Self {
        Self { api, team_scope }
    }
}

impl ManagedSecretsTransportProvider for ScopedManagedSecretsClient {
    fn managed_secrets_transport(&self) -> ManagedSecretsTransport<'_> {
        ManagedSecretsTransport::new(&self.api, Some(self.team_scope))
    }
}

macro_rules! impl_managed_secrets_client {
    ($client:ty) => {
        #[cfg_attr(not(target_family = "wasm"), async_trait)]
        #[cfg_attr(target_family = "wasm", async_trait(?Send))]
        impl ManagedSecretsClient for $client {
            async fn get_managed_secret_configs(&self) -> Result<ManagedSecretConfigs> {
                self.managed_secrets_transport()
                    .get_managed_secret_configs()
                    .await
            }

            async fn create_managed_secret(
                &self,
                owner: SecretOwner,
                name: String,
                secret_type: ManagedSecretType,
                encrypted_value: String,
                description: Option<String>,
            ) -> Result<ManagedSecret> {
                self.managed_secrets_transport()
                    .create_managed_secret(owner, name, secret_type, encrypted_value, description)
                    .await
            }

            async fn delete_managed_secret(&self, owner: SecretOwner, name: String) -> Result<()> {
                self.managed_secrets_transport()
                    .delete_managed_secret(owner, name)
                    .await
            }

            async fn update_managed_secret(
                &self,
                owner: SecretOwner,
                name: String,
                encrypted_value: Option<String>,
                description: Option<String>,
            ) -> Result<ManagedSecret> {
                self.managed_secrets_transport()
                    .update_managed_secret(owner, name, encrypted_value, description)
                    .await
            }

            async fn list_secrets(&self) -> Result<Vec<ManagedSecret>> {
                self.managed_secrets_transport().list_secrets().await
            }

            async fn list_harness_auth_secrets(
                &self,
                harness: warp_graphql::ai::AgentHarness,
            ) -> Result<Vec<ManagedSecret>> {
                self.managed_secrets_transport()
                    .list_harness_auth_secrets(harness)
                    .await
            }

            async fn get_task_secrets(
                &self,
                task_id: String,
                workload_token: String,
            ) -> Result<HashMap<String, ManagedSecretValue>> {
                self.managed_secrets_transport()
                    .get_task_secrets(task_id, workload_token)
                    .await
            }

            async fn issue_task_identity_token(
                &self,
                options: warp_managed_secrets::client::IdentityTokenOptions,
            ) -> Result<TaskIdentityToken> {
                self.managed_secrets_transport()
                    .issue_task_identity_token(options)
                    .await
            }
        }
    };
}

impl_managed_secrets_client!(ServerApi);
impl_managed_secrets_client!(ScopedManagedSecretsClient);

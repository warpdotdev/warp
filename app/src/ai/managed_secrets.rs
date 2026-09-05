use std::future::Future;
use std::sync::Arc;

use warp_graphql::managed_secrets::ManagedSecret;
use warp_managed_secrets::client::{ManagedSecretsClient, SecretOwner};
use warp_managed_secrets::{ActorProvider, ManagedSecretManager, ManagedSecretValue};
use warpui::{Entity, SingletonEntity};

use crate::server::server_api::ServerApi;
use crate::server::server_api::managed_secrets::ScopedManagedSecretsClient;
use crate::server::team_scope::RequestTeamScope;

#[derive(Clone)]
pub struct ManagedSecretsFacade {
    api: Arc<ServerApi>,
    actor_provider: Arc<dyn ActorProvider>,
}

impl ManagedSecretsFacade {
    pub fn new(api: Arc<ServerApi>, actor_provider: Arc<dyn ActorProvider>) -> Self {
        Self {
            api,
            actor_provider,
        }
    }

    fn manager_for_scope(&self, team_scope: RequestTeamScope) -> ManagedSecretManager {
        ManagedSecretManager::new(
            self.client_for_scope(team_scope),
            self.actor_provider.clone(),
        )
    }

    pub(crate) fn client_for_scope(
        &self,
        team_scope: RequestTeamScope,
    ) -> Arc<dyn ManagedSecretsClient> {
        Arc::new(ScopedManagedSecretsClient::new(
            self.api.clone(),
            team_scope,
        ))
    }

    pub fn create_secret(
        &self,
        team_scope: RequestTeamScope,
        owner: SecretOwner,
        name: String,
        value: ManagedSecretValue,
        description: Option<String>,
    ) -> impl Future<Output = anyhow::Result<ManagedSecret>> + use<> {
        self.manager_for_scope(team_scope)
            .create_secret(owner, name, value, description)
    }

    pub fn delete_secret(
        &self,
        team_scope: RequestTeamScope,
        owner: SecretOwner,
        name: String,
    ) -> impl Future<Output = anyhow::Result<()>> + use<> {
        self.manager_for_scope(team_scope)
            .delete_secret(owner, name)
    }

    pub fn update_secret(
        &self,
        team_scope: RequestTeamScope,
        owner: SecretOwner,
        name: String,
        value: Option<ManagedSecretValue>,
        description: Option<String>,
    ) -> impl Future<Output = anyhow::Result<ManagedSecret>> + use<> {
        self.manager_for_scope(team_scope)
            .update_secret(owner, name, value, description)
    }

    pub fn list_secrets(
        &self,
        team_scope: RequestTeamScope,
    ) -> impl Future<Output = anyhow::Result<Vec<ManagedSecret>>> + use<> {
        self.manager_for_scope(team_scope).list_secrets()
    }

    pub fn list_harness_auth_secrets(
        &self,
        team_scope: RequestTeamScope,
        harness: warp_graphql::ai::AgentHarness,
    ) -> impl Future<Output = anyhow::Result<Vec<ManagedSecret>>> + use<> {
        let client = self.client_for_scope(team_scope);
        async move { client.list_harness_auth_secrets(harness).await }
    }
}

impl Entity for ManagedSecretsFacade {
    type Event = ();
}

impl SingletonEntity for ManagedSecretsFacade {}

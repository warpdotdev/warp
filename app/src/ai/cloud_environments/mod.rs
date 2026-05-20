use serde::{Deserialize, Serialize};
use std::fmt;
use warp_server_client::cloud_object::Owner;

use crate::{
    auth::AuthStateProvider,
    cloud_object::{
        model::{
            generic_string_model::{GenericStringModel, GenericStringObjectId, StringModel},
            json_model::{JsonModel, JsonSerializer},
            persistence::CloudModel,
        },
        GenericCloudObject, GenericStringObjectFormat, GenericStringObjectUniqueKey,
        JsonObjectType, Revision,
    },
    server::{ids::SyncId, sync_queue::QueueItem},
    workspaces::user_workspaces::UserWorkspaces,
};
use warpui::{AppContext, SingletonEntity as _};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct GithubRepo {
    /// Repository owner (e.g. "warpdotdev")
    pub owner: String,
    /// Repository name (e.g. "warp-internal")
    pub repo: String,
}

impl GithubRepo {
    pub fn new(owner: String, repo: String) -> Self {
        Self { owner, repo }
    }
}

impl fmt::Display for GithubRepo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.owner, self.repo)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum BaseImage {
    DockerImage(String),
}

impl fmt::Display for BaseImage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BaseImage::DockerImage(s) => s.fmt(f),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct GcpProviderConfig {
    pub project_number: String,
    pub workload_identity_federation_pool_id: String,
    pub workload_identity_federation_provider_id: String,
    /// Service account email for impersonation. When set, the federated token
    /// is exchanged for a service account access token.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_account_email: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct AwsProviderConfig {
    pub role_arn: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub struct ProvidersConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gcp: Option<GcpProviderConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aws: Option<AwsProviderConfig>,
}

impl ProvidersConfig {
    pub fn is_empty(&self) -> bool {
        self.gcp.is_none() && self.aws.is_none()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
/// An AmbientAgentEnvironment represents an environment that we would run a Warp agent in.
pub struct AmbientAgentEnvironment {
    /// Environment name
    #[serde(default)]
    pub name: String,
    /// Optional description of the environment (max 240 characters)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// List of GitHub repositories
    #[serde(default)]
    pub github_repos: Vec<GithubRepo>,
    /// Base image specification
    #[serde(flatten)]
    pub base_image: BaseImage,
    /// List of setup commands to run after cloning
    #[serde(default)]
    pub setup_commands: Vec<String>,
    /// Optional cloud provider configurations for automatic auth.
    #[serde(default, skip_serializing_if = "ProvidersConfig::is_empty")]
    pub providers: ProvidersConfig,
}

pub type CloudAmbientAgentEnvironment =
    GenericCloudObject<GenericStringObjectId, CloudAmbientAgentEnvironmentModel>;
pub type CloudAmbientAgentEnvironmentModel =
    GenericStringModel<AmbientAgentEnvironment, JsonSerializer>;

impl CloudAmbientAgentEnvironment {
    pub fn get_all(app: &AppContext) -> Vec<CloudAmbientAgentEnvironment> {
        CloudModel::as_ref(app)
            .get_all_objects_of_type::<GenericStringObjectId, CloudAmbientAgentEnvironmentModel>()
            .cloned()
            .collect()
    }

    pub fn get_by_id<'a>(
        sync_id: &'a SyncId,
        app: &'a AppContext,
    ) -> Option<&'a CloudAmbientAgentEnvironment> {
        CloudModel::as_ref(app)
            .get_object_of_type::<GenericStringObjectId, CloudAmbientAgentEnvironmentModel>(sync_id)
    }
}

impl AmbientAgentEnvironment {
    pub fn new(
        name: String,
        description: Option<String>,
        github_repos: Vec<GithubRepo>,
        docker_image: String,
        setup_commands: Vec<String>,
    ) -> Self {
        Self {
            name,
            description,
            github_repos,
            base_image: BaseImage::DockerImage(docker_image),
            setup_commands,
            providers: ProvidersConfig::default(),
        }
    }
}

impl StringModel for AmbientAgentEnvironment {
    type CloudObjectType = CloudAmbientAgentEnvironment;

    fn model_type_name(&self) -> &'static str {
        "Cloud environment"
    }

    fn should_enforce_revisions() -> bool {
        true
    }

    fn model_format() -> GenericStringObjectFormat {
        GenericStringObjectFormat::Json(JsonObjectType::CloudEnvironment)
    }

    fn display_name(&self) -> String {
        self.name.clone()
    }

    fn update_object_queue_item(
        &self,
        revision_ts: Option<Revision>,
        object: &CloudAmbientAgentEnvironment,
    ) -> QueueItem {
        QueueItem::UpdateCloudEnvironment {
            model: object.model().clone().into(),
            id: object.id,
            revision: revision_ts.or_else(|| object.metadata.revision.clone()),
        }
    }

    fn uniqueness_key(&self) -> Option<GenericStringObjectUniqueKey> {
        None
    }

    fn should_show_activity_toasts() -> bool {
        false
    }

    fn warn_if_unsaved_at_quit() -> bool {
        true
    }
}

impl JsonModel for AmbientAgentEnvironment {
    fn json_object_type() -> JsonObjectType {
        JsonObjectType::CloudEnvironment
    }
}

/// Resolves the current owner for creating new environments.
///
/// If the user is on a team, returns `Owner::Team`. Otherwise, returns
/// `Owner::User` with the current user's ID. Returns `None` if the user
/// is not logged in.
pub fn owner_for_new_environment(ctx: &AppContext) -> Option<Owner> {
    if let Some(team_uid) = UserWorkspaces::as_ref(ctx).current_team_uid() {
        Some(Owner::Team { team_uid })
    } else {
        let user_id = AuthStateProvider::as_ref(ctx).get().user_id()?;
        Some(Owner::User { user_uid: user_id })
    }
}

/// Resolves the current owner for creating new personal environments.
///
/// Returns `Owner::User` with the current user's ID. Returns `None` if the user
/// is not logged in.
pub fn owner_for_new_personal_environment(ctx: &AppContext) -> Option<Owner> {
    let user_id = AuthStateProvider::as_ref(ctx).get().user_id()?;
    Some(Owner::User { user_uid: user_id })
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;

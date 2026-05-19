pub use warp_server_client::cloud_object::models::{
    AmbientAgentEnvironment, AwsProviderConfig, BaseImage, GcpProviderConfig, GithubRepo,
    ProvidersConfig,
};
use warp_server_client::cloud_object::Owner;

use crate::{
    auth::AuthStateProvider,
    cloud_object::{
        model::{
            generic_string_model::{GenericStringModel, GenericStringObjectId, StringModel},
            json_model::{JsonModel, JsonSerializer},
        },
        GenericCloudObject, GenericStringObjectFormat, GenericStringObjectUniqueKey,
        JsonObjectType, Revision,
    },
    server::sync_queue::QueueItem,
    workspaces::user_workspaces::UserWorkspaces,
};
use warpui::{AppContext, SingletonEntity as _};

pub type CloudAmbientAgentEnvironment =
    GenericCloudObject<GenericStringObjectId, CloudAmbientAgentEnvironmentModel>;
pub type CloudAmbientAgentEnvironmentModel =
    GenericStringModel<AmbientAgentEnvironment, JsonSerializer>;

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

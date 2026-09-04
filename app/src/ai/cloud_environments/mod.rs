mod catalog;
pub use catalog::CloudEnvironmentCatalog;
#[cfg(all(feature = "local_fs", not(target_family = "wasm")))]
pub(crate) use catalog::sort_environments_by_recency;
#[cfg(feature = "tui")]
pub use catalog::{CloudEnvironment, CloudEnvironmentCatalogEvent};
#[cfg_attr(target_family = "wasm", expect(unused_imports))]
pub use cloud_object_models::{
    AmbientAgentEnvironment, AwsProviderConfig, BaseImage, CloudAmbientAgentEnvironment,
    CloudAmbientAgentEnvironmentModel, GcpProviderConfig, GithubRepo, ProvidersConfig, SourceRepo,
};
use cloud_objects::cloud_object::Owner;
use warpui::{AppContext, Entity, SingletonEntity as _, ViewContext};

use crate::auth::AuthStateProvider;
use crate::cloud_object::model::generic_string_model::StringModel;
use crate::cloud_object::model::json_model::JsonModel;
use crate::cloud_object::{
    GenericStringObjectFormat, GenericStringObjectUniqueKey, JsonObjectType, Revision,
};
use crate::server::sync_queue::QueueItem;
use crate::workspaces::user_workspaces::UserWorkspaces;

/// Oz web app page for viewing and creating cloud environments.
#[cfg(feature = "tui")]
pub const OZ_ENVIRONMENTS_URL: &str = "https://oz.warp.dev/environments";

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
            revision: revision_ts.or(object.metadata.revision),
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

pub fn owner_for_new_environment<T: Entity>(ctx: &ViewContext<T>) -> Option<Owner> {
    if let Some(team_uid) = UserWorkspaces::as_ref(ctx)
        .team_for_view(ctx)
        .map(|team| team.uid)
    {
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

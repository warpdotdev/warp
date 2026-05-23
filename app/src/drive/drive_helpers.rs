use warpui::{SingletonEntity, ViewContext};

use crate::auth::auth_manager::AuthManager;
use crate::auth::AuthStateProvider;
use crate::cloud_object::model::persistence::CloudModel;
use crate::cloud_object::{GenericStringObjectFormat, JsonObjectType, ObjectType, Space};

pub fn has_feature_gated_anonymous_user_reached_notebook_limit<V: warpui::View>(
    ctx: &mut ViewContext<V>,
) -> bool {
    let count = CloudModel::handle(ctx).read(ctx, |model, ctx| {
        model
            .active_non_welcome_notebooks_in_space(Space::Personal, ctx)
            .count()
    });
    if AuthStateProvider::handle(ctx).read(ctx, |auth_state_provider, _ctx| {
        auth_state_provider
            .get()
            .is_anonymous_user_past_object_limit(ObjectType::Notebook, count + 1)
            .unwrap_or_default()
    }) {
        AuthManager::handle(ctx).update(ctx, |auth_manager: &mut AuthManager, ctx| {
            auth_manager.anonymous_user_hit_drive_object_limit(ctx);
        });
        return true;
    };

    false
}

pub fn has_feature_gated_anonymous_user_reached_workflow_limit<V: warpui::View>(
    ctx: &mut ViewContext<V>,
) -> bool {
    let count = CloudModel::handle(ctx).read(ctx, |model, ctx| {
        model
            .active_non_welcome_workflows_in_space(Space::Personal, ctx)
            .count()
    });
    if AuthStateProvider::handle(ctx).read(ctx, |auth_state_provider, _ctx| {
        auth_state_provider
            .get()
            .is_anonymous_user_past_object_limit(ObjectType::Workflow, count + 1)
            .unwrap_or_default()
    }) {
        AuthManager::handle(ctx).update(ctx, |auth_manager: &mut AuthManager, ctx| {
            auth_manager.anonymous_user_hit_drive_object_limit(ctx);
        });
        return true;
    };

    false
}

pub fn has_feature_gated_anonymous_user_reached_env_var_limit<V: warpui::View>(
    ctx: &mut ViewContext<V>,
) -> bool {
    let count = CloudModel::handle(ctx).read(ctx, |model, ctx| {
        model
            .active_non_welcome_env_var_collections_in_space(Space::Personal, ctx)
            .count()
    });
    if AuthStateProvider::handle(ctx).read(ctx, |auth_state_provider, _ctx| {
        auth_state_provider
            .get()
            .is_anonymous_user_past_object_limit(
                ObjectType::GenericStringObject(GenericStringObjectFormat::Json(
                    JsonObjectType::EnvVarCollection,
                )),
                count + 1,
            )
            .unwrap_or_default()
    }) {
        AuthManager::handle(ctx).update(ctx, |auth_manager: &mut AuthManager, ctx| {
            auth_manager.anonymous_user_hit_drive_object_limit(ctx);
        });
        return true;
    };

    false
}

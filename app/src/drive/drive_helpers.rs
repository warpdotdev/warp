use warp_server_auth::AuthStateProvider;
use warp_server_auth::auth_state::AuthState;
use warp_server_auth::user::PersonalObjectLimits;
use warpui::{AppContext, SingletonEntity};

use crate::auth::auth_manager::AuthManager;
use crate::cloud_object::Space;
use crate::cloud_object::model::persistence::CloudModel;

#[derive(Clone, Copy)]
enum AnonymousUserObjectLimit {
    Notebook,
    Workflow,
    EnvVarCollection,
}

impl AnonymousUserObjectLimit {
    fn is_exceeded(self, limits: PersonalObjectLimits, num_objects: usize) -> bool {
        match self {
            Self::Notebook => num_objects > limits.notebook_limit,
            Self::Workflow => num_objects > limits.workflow_limit,
            Self::EnvVarCollection => num_objects > limits.env_var_limit,
        }
    }
}

fn is_feature_gated_anonymous_user_past_limit(
    auth_state: &AuthState,
    num_objects: usize,
    object_limit: AnonymousUserObjectLimit,
) -> bool {
    auth_state
        .is_anonymous_user_feature_gated()
        .unwrap_or_default()
        && auth_state
            .personal_object_limits()
            .is_some_and(|limits| object_limit.is_exceeded(limits, num_objects))
}

pub(crate) fn is_feature_gated_anonymous_user_past_notebook_limit(
    auth_state: &AuthState,
    num_objects: usize,
) -> bool {
    is_feature_gated_anonymous_user_past_limit(
        auth_state,
        num_objects,
        AnonymousUserObjectLimit::Notebook,
    )
}

pub(crate) fn is_feature_gated_anonymous_user_past_workflow_limit(
    auth_state: &AuthState,
    num_objects: usize,
) -> bool {
    is_feature_gated_anonymous_user_past_limit(
        auth_state,
        num_objects,
        AnonymousUserObjectLimit::Workflow,
    )
}

pub(crate) fn is_feature_gated_anonymous_user_past_env_var_limit(
    auth_state: &AuthState,
    num_objects: usize,
) -> bool {
    is_feature_gated_anonymous_user_past_limit(
        auth_state,
        num_objects,
        AnonymousUserObjectLimit::EnvVarCollection,
    )
}

fn has_feature_gated_anonymous_user_reached_limit(
    ctx: &mut AppContext,
    num_objects: usize,
    object_limit: AnonymousUserObjectLimit,
) -> bool {
    if AuthStateProvider::handle(ctx).read(ctx, |auth_state_provider, _ctx| {
        is_feature_gated_anonymous_user_past_limit(
            auth_state_provider.get(),
            num_objects,
            object_limit,
        )
    }) {
        AuthManager::handle(ctx).update(ctx, |auth_manager: &mut AuthManager, ctx| {
            auth_manager.anonymous_user_hit_drive_object_limit(ctx);
        });
        return true;
    };

    false
}

pub fn has_feature_gated_anonymous_user_reached_notebook_limit(ctx: &mut AppContext) -> bool {
    let count = CloudModel::handle(ctx).read(ctx, |model, ctx| {
        model
            .active_non_welcome_notebooks_in_space(Space::Personal, ctx)
            .count()
    });
    has_feature_gated_anonymous_user_reached_limit(
        ctx,
        count + 1,
        AnonymousUserObjectLimit::Notebook,
    )
}

pub fn has_feature_gated_anonymous_user_reached_workflow_limit(ctx: &mut AppContext) -> bool {
    let count = CloudModel::handle(ctx).read(ctx, |model, ctx| {
        model
            .active_non_welcome_workflows_in_space(Space::Personal, ctx)
            .count()
    });
    has_feature_gated_anonymous_user_reached_limit(
        ctx,
        count + 1,
        AnonymousUserObjectLimit::Workflow,
    )
}

pub fn has_feature_gated_anonymous_user_reached_env_var_limit(ctx: &mut AppContext) -> bool {
    let count = CloudModel::handle(ctx).read(ctx, |model, ctx| {
        model
            .active_non_welcome_env_var_collections_in_space(Space::Personal, ctx)
            .count()
    });
    has_feature_gated_anonymous_user_reached_limit(
        ctx,
        count + 1,
        AnonymousUserObjectLimit::EnvVarCollection,
    )
}

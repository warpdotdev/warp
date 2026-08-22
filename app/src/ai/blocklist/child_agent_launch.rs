//! Frontend-neutral preparation and settings propagation for local Oz children.
#[cfg(not(target_family = "wasm"))]
use std::future::Future;

use warpui::{AppContext, EntityId, SingletonEntity as _};
#[cfg(not(target_family = "wasm"))]
use {
    crate::ai::ambient_agents::task::normalize_orchestrator_agent_name,
    crate::ai::ambient_agents::{AgentConfigSnapshot, AmbientAgentTaskId},
    crate::server::server_api::ServerApiProvider,
};

use crate::AIExecutionProfilesModel;
use crate::ai::execution_profiles::ExecutionProfileId;
#[cfg(not(target_family = "wasm"))]
use crate::ai::llms::AgentModeLLMOverrideUpdate;
use crate::ai::llms::{LLMId, LLMPreferences};
use crate::workspaces::user_workspaces::TeamContext;
#[cfg(feature = "tui")]
use crate::workspaces::user_workspaces::TeamContextForOperation;

/// Server-side state prepared before a frontend creates the child's surface.
#[cfg(not(target_family = "wasm"))]
pub struct PreparedLocalOzChildLaunch {
    pub task_id: AmbientAgentTaskId,
    pub conversation_name: String,
}
pub(crate) struct InheritedChildAgentSettings {
    profile_id: ExecutionProfileId,
    base_model_id: LLMId,
    profile_default_model_id: LLMId,
}

/// Creates the server task row shared by the GUI hidden-pane and TUI
/// background-session launch paths.
#[cfg(not(target_family = "wasm"))]
pub fn prepare_local_oz_child_launch(
    name: &str,
    prompt: &str,
    parent_run_id: Option<&str>,
    ctx: &AppContext,
) -> impl Future<Output = anyhow::Result<PreparedLocalOzChildLaunch>> + 'static + use<> {
    let ai_client = ServerApiProvider::as_ref(ctx).get_ai_client();
    let agent_name = normalize_orchestrator_agent_name(name);
    let conversation_name = agent_name.clone().unwrap_or_default();
    let prompt = prompt.to_owned();
    let parent_run_id = parent_run_id.map(str::to_owned);
    async move {
        let task_id = ai_client
            .create_agent_task(
                prompt,
                None,
                parent_run_id,
                Some(AgentConfigSnapshot {
                    name: agent_name,
                    ..Default::default()
                }),
            )
            .await?;
        Ok(PreparedLocalOzChildLaunch {
            task_id,
            conversation_name,
        })
    }
}

/// Copies the parent's execution profile and effective base model to a child
/// surface before its first request is sent.
#[cfg(feature = "tui")]
pub fn inherit_child_agent_settings(
    parent_surface_id: EntityId,
    child_surface_id: EntityId,
    team_context: &TeamContextForOperation,
    ctx: &mut AppContext,
) {
    let profile_id = AIExecutionProfilesModel::as_ref(ctx)
        .active_profile(Some(parent_surface_id), ctx)
        .id()
        .clone();
    let preferences = LLMPreferences::as_ref(ctx);
    let base_model_id = preferences
        .get_active_base_model(Some(parent_surface_id), team_context, ctx)
        .id
        .clone();
    let profile_default_model_id = preferences
        .get_active_profile_base_model(Some(parent_surface_id), team_context, ctx)
        .id
        .clone();
    let settings = InheritedChildAgentSettings {
        profile_id,
        base_model_id,
        profile_default_model_id,
    };
    apply_inherited_child_agent_settings(child_surface_id, settings, ctx);
}
pub(crate) fn inherited_child_agent_settings_for_team_context(
    parent_surface_id: EntityId,
    team_context: Option<&TeamContext<'_>>,
    ctx: &AppContext,
) -> InheritedChildAgentSettings {
    let profile_id = AIExecutionProfilesModel::as_ref(ctx)
        .active_profile(Some(parent_surface_id), ctx)
        .id()
        .clone();
    let preferences = LLMPreferences::as_ref(ctx);
    let base_model_id = preferences
        .get_active_base_model_for_team_context(Some(parent_surface_id), team_context, ctx)
        .id
        .clone();
    let profile_default_model_id = preferences
        .get_active_profile_base_model_for_team_context(Some(parent_surface_id), team_context, ctx)
        .id
        .clone();
    InheritedChildAgentSettings {
        profile_id,
        base_model_id,
        profile_default_model_id,
    }
}

pub(crate) fn apply_inherited_child_agent_settings(
    child_surface_id: EntityId,
    settings: InheritedChildAgentSettings,
    ctx: &mut AppContext,
) {
    AIExecutionProfilesModel::handle(ctx).update(ctx, |profiles, ctx| {
        profiles.set_active_profile(child_surface_id, settings.profile_id, ctx);
    });
    LLMPreferences::handle(ctx).update(ctx, |preferences, ctx| {
        preferences.update_preferred_agent_mode_llm_with_profile_default(
            &settings.base_model_id,
            child_surface_id,
            &settings.profile_default_model_id,
            ctx,
        );
    });
}

/// Applies a non-empty run-wide model override after parent settings have
/// been inherited.
#[cfg(not(target_family = "wasm"))]
#[cfg(feature = "tui")]
pub fn apply_child_agent_model_override(
    child_surface_id: EntityId,
    model_id: Option<&str>,
    team_context: &TeamContextForOperation,
    ctx: &mut AppContext,
) {
    let Some(model_id) = model_id.map(str::trim).filter(|id| !id.is_empty()) else {
        return;
    };
    let model_id = LLMId::from(model_id);
    LLMPreferences::handle(ctx).update(ctx, |preferences, ctx| {
        preferences.set_agent_mode_llm_override(child_surface_id, model_id, team_context, ctx);
    });
}

#[cfg(not(target_family = "wasm"))]
pub(crate) fn prepare_child_agent_model_override_for_team_context(
    child_surface_id: EntityId,
    model_id: Option<&str>,
    team_context: Option<&TeamContext<'_>>,
    ctx: &AppContext,
) -> Option<AgentModeLLMOverrideUpdate> {
    let model_id = model_id.map(str::trim).filter(|id| !id.is_empty())?;
    Some(
        LLMPreferences::as_ref(ctx).prepare_agent_mode_llm_override_for_team_context(
            child_surface_id,
            LLMId::from(model_id),
            team_context,
            ctx,
        ),
    )
}

#[cfg(not(target_family = "wasm"))]
pub(crate) fn apply_prepared_child_agent_model_override(
    update: Option<AgentModeLLMOverrideUpdate>,
    ctx: &mut AppContext,
) {
    let Some(update) = update else {
        return;
    };
    LLMPreferences::handle(ctx).update(ctx, |preferences, ctx| {
        preferences.apply_agent_mode_llm_override(update, ctx);
    });
}

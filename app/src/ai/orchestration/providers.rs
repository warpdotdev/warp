//! `AppContext`-backed catalog lookups, default resolution, and
//! persistence helpers for orchestration edit flows. No GUI types.

use ai::agent::action::RunAgentsRequest;
use settings::Setting;
use warp_cli::agent::Harness;
use warp_errors::report_if_error;
use warpui::{AppContext, SingletonEntity};

use crate::ai::auth_secret_types::auth_secret_types_for_harness;
use crate::ai::cloud_agent_settings::CloudAgentSettings;
use crate::ai::cloud_environments::CloudAmbientAgentEnvironment;
use crate::ai::connected_self_hosted_workers::WARP_WORKER_HOST;
use crate::ai::harness_availability::{AuthSecretFetchState, HarnessAvailabilityModel};
use crate::ai::llms::LLMInfo;
use crate::ai::orchestration::config_state::AuthSecretSelection;
use crate::cloud_object::CloudObjectLookup as _;
use crate::workspaces::user_workspaces::UserWorkspaces;
use crate::LLMPreferences;

/// Env var override for the workspace default host (developer testing).
/// Mirrors the single-agent ambient flow.
const DEFAULT_HOST_ENV_VAR: &str = "WARP_CLOUD_MODE_DEFAULT_HOST";

pub const ORCHESTRATION_WARP_WORKER_HOST: &str = WARP_WORKER_HOST;
pub const ORCHESTRATION_ENV_NONE_LABEL: &str = "Empty environment";

/// Returns Warp base-model choices for orchestration.
pub(crate) fn get_base_model_choices<'a>(
    llm_prefs: &'a LLMPreferences,
    app: &'a AppContext,
    is_local: bool,
) -> impl Iterator<Item = &'a LLMInfo> {
    llm_prefs
        .get_base_llm_choices_for_agent_mode(app)
        .filter(move |llm| is_local || llm_prefs.custom_llm_info_for_id(&llm.id).is_none())
}

/// Returns whether the given model_id is present in the harness-filtered
/// model choices. Used to detect when a harness change invalidates the
/// current model selection.
pub fn is_model_in_filtered_choices(
    model_id: &str,
    harness_type: &str,
    is_local: bool,
    ctx: &AppContext,
) -> bool {
    let harness = Harness::parse_orchestration_harness(harness_type);
    match harness {
        Some(Harness::Oz) | None => {
            let llm_prefs = LLMPreferences::as_ref(ctx);
            get_base_model_choices(llm_prefs, ctx, is_local)
                .any(|llm| llm.id.to_string() == model_id)
        }
        Some(Harness::Codex) if is_local => model_id.is_empty(),
        Some(harness) => {
            // Empty string is always valid (the "Default model" entry).
            if model_id.is_empty() {
                return true;
            }
            let availability = HarnessAvailabilityModel::as_ref(ctx);
            availability
                .models_for(harness)
                .is_some_and(|models| models.iter().any(|m| m.id == model_id))
        }
    }
}

/// Returns the default model_id for the given harness.
///
/// For Oz this is the first Warp LLM; for non-Oz harnesses it is an empty
/// string (the "Default model" entry).
pub fn first_filtered_model_id(harness_type: &str, ctx: &AppContext) -> Option<String> {
    let harness = Harness::parse_orchestration_harness(harness_type);
    match harness {
        Some(Harness::Oz) | None => {
            let llm_prefs = LLMPreferences::as_ref(ctx);
            llm_prefs
                .get_base_llm_choices_for_agent_mode(ctx)
                .next()
                .map(|llm| llm.id.to_string())
        }
        Some(_) => Some(String::new()),
    }
}

/// Resolves the workspace-configured default host slug, honoring the
/// `WARP_CLOUD_MODE_DEFAULT_HOST` env var override for developer
/// testing. Mirrors the single-agent ambient flow.
pub fn resolve_default_host_slug(ctx: &AppContext) -> Option<String> {
    if let Ok(slug) = std::env::var(DEFAULT_HOST_ENV_VAR) {
        let trimmed = slug.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }
    UserWorkspaces::as_ref(ctx)
        .default_host_slug()
        .map(str::to_string)
        .filter(|s| !s.trim().is_empty())
}

/// Returns the user's last-selected custom host slug from
/// `CloudAgentSettings.last_selected_host`, excluding `"warp"` and the
/// workspace default (those are surfaced as separate menu rows).
pub fn resolve_recent_host_slug(ctx: &AppContext) -> Option<String> {
    let last = CloudAgentSettings::as_ref(ctx)
        .last_selected_host
        .value()
        .clone()
        .filter(|s| !s.trim().is_empty())?;
    if last.eq_ignore_ascii_case(ORCHESTRATION_WARP_WORKER_HOST) {
        return None;
    }
    if resolve_default_host_slug(ctx).as_deref() == Some(last.as_str()) {
        return None;
    }
    Some(last)
}

/// Persists the user's most-recent host selection to
/// `CloudAgentSettings.last_selected_host`. Skipped for `"warp"` and
/// empty values (those don't represent a custom slug worth remembering).
pub fn persist_host_selection(worker_host: &str, ctx: &mut AppContext) {
    let trimmed = worker_host.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case(ORCHESTRATION_WARP_WORKER_HOST) {
        return;
    }
    let value = trimmed.to_string();
    CloudAgentSettings::handle(ctx).update(ctx, |settings, ctx| {
        report_if_error!(settings.last_selected_host.set_value(Some(value), ctx));
    });
}

/// Normalizes a harness_type string for use as a HashMap key in
/// per-harness model memory. Empty string (the wire representation
/// of Oz) is mapped to "oz" so saves and lookups are consistent.
pub fn harness_save_key(harness_type: &str) -> &str {
    if harness_type.is_empty() {
        "oz"
    } else {
        harness_type
    }
}

/// Resolves a default environment ID using the same logic as the
/// `/cloud-agent` environment selector: first tries the user's
/// last-selected environment from settings, then falls back to the
/// most recently used environment.
pub fn resolve_default_environment_id(ctx: &AppContext) -> Option<String> {
    if let Some(env_id) = *CloudAgentSettings::as_ref(ctx)
        .last_selected_environment_id
        .value()
    {
        if CloudAmbientAgentEnvironment::get_by_id(&env_id, ctx).is_some() {
            return Some(env_id.uid());
        }
    }
    let mut envs = CloudAmbientAgentEnvironment::get_all(ctx);
    envs.sort_by(|a, b| {
        b.metadata
            .last_task_run_ts
            .cmp(&a.metadata.last_task_run_ts)
            .then_with(|| {
                a.model()
                    .string_model
                    .name
                    .cmp(&b.model().string_model.name)
            })
    });
    envs.first().map(|e| e.id.uid())
}

/// Persists the user's environment selection to settings so it can
/// be restored as the default next time. Shared by both the plan
/// card and confirmation card `EnvironmentChanged` handlers.
pub fn persist_environment_selection(environment_id: &str, ctx: &mut AppContext) {
    if environment_id.is_empty() {
        return;
    }
    let all_envs = CloudAmbientAgentEnvironment::get_all(ctx);
    if let Some(env) = all_envs.iter().find(|e| e.id.uid() == environment_id) {
        let sync_id = env.id;
        CloudAgentSettings::handle(ctx).update(ctx, |settings, ctx| {
            if let Err(e) = settings
                .last_selected_environment_id
                .set_value(Some(sync_id), ctx)
            {
                log::warn!("Failed to persist environment selection: {e:?}");
            }
        });
    }
}

/// Returns the persisted last-selected secret name for this harness, or
/// `None`. Only promotes a persisted name; never auto-picks the first
/// loaded secret. Validates against the loaded secrets list when present,
/// returning `None` if the persisted name has been deleted server-side.
pub fn resolve_default_auth_secret_for_harness(
    harness_type: &str,
    ctx: &AppContext,
) -> Option<String> {
    let harness = Harness::parse_orchestration_harness(harness_type)?;
    if harness == Harness::Oz {
        return None;
    }
    let persisted = CloudAgentSettings::as_ref(ctx)
        .last_selected_auth_secret
        .value()
        .get(harness.config_name())
        .cloned()
        .filter(|name| !name.trim().is_empty());

    let availability = HarnessAvailabilityModel::as_ref(ctx);
    match availability.auth_secrets_for(harness) {
        AuthSecretFetchState::Loaded(secrets) => {
            // Drop the persisted name if the secret was deleted server-side.
            persisted.filter(|name| secrets.iter().any(|s| s.name == *name))
        }
        // Pre-fetch: optimistically show the persisted name; the
        // `AuthSecretsLoaded` subscription will re-resolve.
        AuthSecretFetchState::NotFetched
        | AuthSecretFetchState::Loading
        | AuthSecretFetchState::Failed(_) => persisted,
    }
}

/// Returns the full persisted selection (Named / Inherit / Unset) for
/// this harness. Prefers an explicit `Inherit` choice over a `Named`
/// fallback so the plan card's "Inherit" survives across the RunAgents
/// handoff (the `OrchestrationConfig` proto doesn't carry auth state).
pub fn resolve_auth_secret_selection_for_harness(
    harness_type: &str,
    ctx: &AppContext,
) -> AuthSecretSelection {
    let Some(harness) = Harness::parse_orchestration_harness(harness_type) else {
        return AuthSecretSelection::Unset;
    };
    if harness == Harness::Oz {
        return AuthSecretSelection::Unset;
    }
    // Explicit Inherit wins over a stale Named fallback.
    let inherit_chosen = CloudAgentSettings::as_ref(ctx)
        .inherit_auth_secret_harnesses
        .value()
        .get(harness.config_name())
        .copied()
        .unwrap_or(false);
    if inherit_chosen {
        return AuthSecretSelection::Inherit;
    }
    match resolve_default_auth_secret_for_harness(harness_type, ctx) {
        Some(name) => AuthSecretSelection::Named(name),
        None => AuthSecretSelection::Unset,
    }
}

/// Persists the user's auth-secret choice for the active harness.
/// `Named` writes to `last_selected_auth_secret` and clears any prior
/// `Inherit` flag. `Inherit` clears the named entry and sets the inherit
/// flag. `Unset`/`CreatingNew` clear both (no recorded choice). No-op for
/// Oz / unknown.
pub(crate) fn persist_auth_secret_selection(
    harness_type: &str,
    selection: &AuthSecretSelection,
    ctx: &mut AppContext,
) {
    let Some(harness) = Harness::parse_orchestration_harness(harness_type) else {
        return;
    };
    if harness == Harness::Oz {
        return;
    }
    let key = harness.config_name().to_string();
    let selection = selection.clone();
    CloudAgentSettings::handle(ctx).update(ctx, |settings, ctx| {
        let mut named_map = settings.last_selected_auth_secret.value().clone();
        let mut inherit_map = settings.inherit_auth_secret_harnesses.value().clone();
        match selection {
            AuthSecretSelection::Named(name) => {
                named_map.insert(key.clone(), name.clone());
                inherit_map.remove(&key);
            }
            AuthSecretSelection::Inherit => {
                named_map.remove(&key);
                inherit_map.insert(key, true);
            }
            AuthSecretSelection::Unset | AuthSecretSelection::CreatingNew => {
                named_map.remove(&key);
                inherit_map.remove(&key);
            }
        }
        report_if_error!(settings.last_selected_auth_secret.set_value(named_map, ctx));
        report_if_error!(settings
            .inherit_auth_secret_harnesses
            .set_value(inherit_map, ctx));
    });
}

/// Whether Remote execution of `request` requires a managed auth secret
/// (non-Oz cloud harness with at least one supported secret type).
fn requires_default_auth_secret_for_execution(request: &RunAgentsRequest) -> bool {
    if !request.execution_mode.is_remote() {
        return false;
    }
    let Some(harness) = Harness::parse_orchestration_harness(&request.harness_type) else {
        return false;
    };
    harness != Harness::Oz && !auth_secret_types_for_harness(harness).is_empty()
}

/// Whether the request can execute as-is: either it doesn't need a
/// managed auth secret, already carries one, or a persisted default
/// exists for the harness.
pub(crate) fn can_execute_with_auth_secret(request: &RunAgentsRequest, ctx: &AppContext) -> bool {
    if !requires_default_auth_secret_for_execution(request) {
        return true;
    }
    if request
        .harness_auth_secret_name
        .as_deref()
        .is_some_and(|name| !name.trim().is_empty())
    {
        return true;
    }
    default_auth_secret_name_for_harness(&request.harness_type, ctx).is_some()
}

/// Returns the persisted default managed-secret name for a harness, if any.
pub(crate) fn default_auth_secret_name_for_harness(
    harness_type: &str,
    ctx: &AppContext,
) -> Option<String> {
    let harness = Harness::parse_orchestration_harness(harness_type)?;
    if harness == Harness::Oz {
        return None;
    }
    CloudAgentSettings::as_ref(ctx)
        .last_selected_auth_secret
        .value()
        .get(harness.config_name())
        .cloned()
        .filter(|name| !name.trim().is_empty())
}

/// Fills `harness_auth_secret_name` from the persisted per-harness default
/// when the request needs one and doesn't already carry a name.
pub(crate) fn populate_default_auth_secret_for_execution(
    request: &mut RunAgentsRequest,
    ctx: &AppContext,
) {
    if !requires_default_auth_secret_for_execution(request)
        || request
            .harness_auth_secret_name
            .as_deref()
            .is_some_and(|name| !name.trim().is_empty())
    {
        return;
    }
    request.harness_auth_secret_name =
        default_auth_secret_name_for_harness(&request.harness_type, ctx);
}

//! [`OrchestrationEditState`] and the logic for applying user edits to
//! the orchestration config, shared by the GUI pickers and the TUI
//! configuration pages. Changing one field (harness, execution mode,
//! auth secret) cascades into dependent fields — model fallback,
//! environment pre-fill, auth re-resolution — so those updates live here
//! as `apply_*` methods rather than in each frontend. Each public method
//! takes an `AppContext` for catalog/persistence access; the cores are
//! parameterized over catalog callbacks so they can be unit-tested
//! without app singletons.

use std::collections::HashMap;

use ai::agent::action::RunAgentsExecutionMode;
use warp_cli::agent::Harness;
use warpui::{AppContext, SingletonEntity};

use super::config_state::{AuthSecretSelection, OrchestrationConfigState};
use super::providers::{
    first_filtered_model_id, harness_save_key, is_model_in_filtered_choices,
    persist_auth_secret_selection, resolve_auth_secret_selection_for_harness,
    resolve_default_environment_id,
};
use crate::ai::harness_availability::{AuthSecretFetchState, HarnessAvailabilityModel};

impl OrchestrationConfigState {
    /// Toggles Local ↔ Cloud, pre-fills the default environment when
    /// switching to Cloud with no environment selected, and revalidates
    /// the model against the new mode's catalog.
    pub fn apply_execution_mode_change(
        &mut self,
        is_remote: bool,
        fallback_base_model_id: Option<String>,
        ctx: &AppContext,
    ) {
        let default_environment_id = if is_remote {
            resolve_default_environment_id(ctx)
        } else {
            None
        };
        self.apply_execution_mode_change_core(
            is_remote,
            fallback_base_model_id,
            default_environment_id,
            &|id, harness, is_local| is_model_in_filtered_choices(id, harness, is_local, ctx),
            &|harness| first_filtered_model_id(harness, ctx),
        );
    }

    /// Records the auth-secret picker choice (`None` means Inherit) and
    /// persists it to `CloudAgentSettings`.
    pub fn apply_auth_secret_change(&mut self, new_name: Option<String>, ctx: &mut AppContext) {
        let normalized = new_name.filter(|s| !s.trim().is_empty());
        self.auth_secret_selection = match normalized {
            Some(name) => AuthSecretSelection::Named(name),
            None => AuthSecretSelection::Inherit,
        };
        persist_auth_secret_selection(&self.harness_type, &self.auth_secret_selection, ctx);
    }

    /// Revalidates the state after a live catalog change: resets a
    /// vanished model to the harness default, drops a deleted `Named(_)`
    /// secret, and re-seeds an `Unset` selection from persisted settings.
    /// This is the frontend-neutral core of the GUI's
    /// `repopulate_all_pickers`.
    pub fn revalidate_after_catalog_change(&mut self, ctx: &AppContext) {
        let loaded_secret_names = Harness::parse_orchestration_harness(&self.harness_type)
            .filter(|harness| *harness != Harness::Oz)
            .and_then(|harness| {
                match HarnessAvailabilityModel::as_ref(ctx).auth_secrets_for(harness) {
                    AuthSecretFetchState::Loaded(secrets) => {
                        Some(secrets.iter().map(|s| s.name.clone()).collect::<Vec<_>>())
                    }
                    AuthSecretFetchState::NotFetched
                    | AuthSecretFetchState::Loading
                    | AuthSecretFetchState::Failed(_) => None,
                }
            });
        let reseeded_selection = resolve_auth_secret_selection_for_harness(&self.harness_type, ctx);
        self.revalidate_after_catalog_change_core(
            loaded_secret_names.as_deref(),
            reseeded_selection,
            &|id, harness, is_local| is_model_in_filtered_choices(id, harness, is_local, ctx),
            &|harness| first_filtered_model_id(harness, ctx),
        );
    }

    /// Core of [`Self::apply_execution_mode_change`]; catalog access is
    /// injected so tests can drive it without app singletons.
    fn apply_execution_mode_change_core(
        &mut self,
        is_remote: bool,
        fallback_base_model_id: Option<String>,
        default_environment_id: Option<String>,
        model_is_valid: &dyn Fn(&str, &str, bool) -> bool,
        default_model_id: &dyn Fn(&str) -> Option<String>,
    ) {
        self.toggle_execution_mode_to_remote(is_remote);
        let is_local = !self.execution_mode.is_remote();
        // Pre-fill environment with the last-selected one when switching
        // to Cloud.
        if is_remote {
            if let RunAgentsExecutionMode::Remote { environment_id, .. } = &self.execution_mode {
                if environment_id.is_empty() {
                    if let Some(default_env) = default_environment_id {
                        self.set_environment_id(default_env);
                    }
                }
            }
        }
        self.reset_model_if_invalid(
            fallback_base_model_id,
            is_local,
            model_is_valid,
            default_model_id,
        );
    }

    /// Core of [`Self::revalidate_after_catalog_change`].
    /// `loaded_secret_names` is `Some` only when secrets for the active
    /// non-Oz harness are loaded; `reseeded_selection` is the persisted
    /// selection used to replace `Unset`.
    fn revalidate_after_catalog_change_core(
        &mut self,
        loaded_secret_names: Option<&[String]>,
        reseeded_selection: AuthSecretSelection,
        model_is_valid: &dyn Fn(&str, &str, bool) -> bool,
        default_model_id: &dyn Fn(&str) -> Option<String>,
    ) {
        let is_local = !self.execution_mode.is_remote();
        if is_local {
            self.sanitize_for_local_execution();
        }
        // Reset model if it disappeared from the harness's catalog.
        if !model_is_valid(&self.model_id, &self.harness_type, is_local) {
            if let Some(first_id) = default_model_id(&self.harness_type) {
                self.model_id = first_id;
            }
        }
        // Drop any `Named(_)` selection whose secret no longer exists.
        if let (Some(names), AuthSecretSelection::Named(name)) =
            (loaded_secret_names, &self.auth_secret_selection)
        {
            if !names.iter().any(|n| n == name) {
                self.auth_secret_selection = AuthSecretSelection::Unset;
            }
        }
        // Re-seed `Unset` from persisted settings. Leaves `Inherit` alone.
        // Uses the full selection resolver so a prior explicit Inherit is
        // restored (rather than being downgraded to Unset).
        if matches!(self.auth_secret_selection, AuthSecretSelection::Unset)
            && !matches!(reseeded_selection, AuthSecretSelection::Unset)
        {
            self.auth_secret_selection = reseeded_selection;
        }
    }

    /// Resets `model_id` when it is invalid for the current harness/mode:
    /// prefers the (validated) fallback, then the harness default.
    fn reset_model_if_invalid(
        &mut self,
        fallback_base_model_id: Option<String>,
        is_local: bool,
        model_is_valid: &dyn Fn(&str, &str, bool) -> bool,
        default_model_id: &dyn Fn(&str) -> Option<String>,
    ) {
        if !model_is_valid(&self.model_id, &self.harness_type, is_local) {
            let reset_id = fallback_base_model_id
                .filter(|id| model_is_valid(id, &self.harness_type, is_local))
                .or_else(|| default_model_id(&self.harness_type))
                .unwrap_or_default();
            self.model_id = reset_id;
        }
    }
}

/// The edit state for one orchestration card: the run-wide config being
/// edited plus the per-harness model memory, which is UI state rather
/// than request state. Card views own one of these; the executor keeps
/// constructing a bare [`OrchestrationConfigState`] and never carries the
/// memory map.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrchestrationEditState {
    pub orchestration_config_state: OrchestrationConfigState,
    /// Per-harness model memory so switching harnesses preserves the
    /// user's previous model selection for each harness. Keyed by
    /// [`harness_save_key`].
    pub saved_model_per_harness: HashMap<String, String>,
}

impl OrchestrationEditState {
    /// Wraps `orchestration_config_state` with empty per-harness memory.
    pub fn new(orchestration_config_state: OrchestrationConfigState) -> Self {
        Self {
            orchestration_config_state,
            saved_model_per_harness: HashMap::new(),
        }
    }

    /// Handles a harness change: saves the current model for the old
    /// harness, restores a previously saved (still valid) model for the
    /// new harness or falls back to a default, and re-resolves the auth
    /// secret selection for the new harness.
    pub fn apply_harness_change(
        &mut self,
        new_harness_type: &str,
        fallback_base_model_id: Option<String>,
        ctx: &mut AppContext,
    ) {
        let resolved_auth = resolve_auth_secret_selection_for_harness(new_harness_type, ctx);
        let ctx: &AppContext = ctx;
        self.apply_harness_change_core(
            new_harness_type,
            fallback_base_model_id,
            resolved_auth,
            &|id, harness, is_local| is_model_in_filtered_choices(id, harness, is_local, ctx),
            &|harness| first_filtered_model_id(harness, ctx),
        );
    }

    /// Core of [`Self::apply_harness_change`]; catalog access and the
    /// resolved auth selection are injected for unit testing.
    fn apply_harness_change_core(
        &mut self,
        new_harness_type: &str,
        fallback_base_model_id: Option<String>,
        resolved_auth: AuthSecretSelection,
        model_is_valid: &dyn Fn(&str, &str, bool) -> bool,
        default_model_id: &dyn Fn(&str) -> Option<String>,
    ) {
        // Save current model for the old harness.
        let old_key = harness_save_key(&self.orchestration_config_state.harness_type).to_string();
        self.saved_model_per_harness
            .insert(old_key, self.orchestration_config_state.model_id.clone());
        self.orchestration_config_state.harness_type = new_harness_type.to_string();

        let is_local = !self.orchestration_config_state.execution_mode.is_remote();
        if is_local {
            self.orchestration_config_state
                .sanitize_for_local_execution();
        }
        // Try to restore a previously saved model for this harness.
        let new_key = harness_save_key(&self.orchestration_config_state.harness_type);
        let restored = self
            .saved_model_per_harness
            .get(new_key)
            .filter(|id| {
                model_is_valid(id, &self.orchestration_config_state.harness_type, is_local)
            })
            .cloned();
        if let Some(saved_id) = restored {
            self.orchestration_config_state.model_id = saved_id;
        } else {
            // No saved model — fall back to conversation base model
            // for Oz, or default for non-Oz.
            self.orchestration_config_state.reset_model_if_invalid(
                fallback_base_model_id,
                is_local,
                model_is_valid,
                default_model_id,
            );
        }

        // Re-resolve auth selection from per-harness persisted state.
        // Honors an explicit `Inherit` choice for the new harness.
        self.orchestration_config_state.auth_secret_selection = resolved_auth;
    }
}

#[cfg(test)]
#[path = "edit_state_tests.rs"]
mod tests;

//! Run-wide orchestration edit state shared by the GUI confirmation
//! card / plan-card config block and the TUI orchestration card.

use ai::agent::action::RunAgentsExecutionMode;
use ai::agent::orchestration_config::{OrchestrationConfig, OrchestrationExecutionMode};
use warp_cli::agent::Harness;

use super::providers::ORCHESTRATION_WARP_WORKER_HOST;
use super::validation::should_show_auth_secret_picker;
use crate::ai::local_harness_setup::local_harness_product_disabled_message;

/// The user's current selection in the auth secret picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthSecretSelection {
    /// No choice yet; re-seeded from persisted settings. Blocks Accept.
    Unset,
    /// User explicitly chose to inherit credentials from the worker env.
    Inherit,
    /// User picked a managed secret by name.
    Named(String),
    /// Creating a key (modal open). Blocks Accept and, unlike `Unset`, is
    /// not re-seeded from persisted settings.
    CreatingNew,
}

impl AuthSecretSelection {
    /// `Some(name)` → `Named`, `None` → `Unset`. Wire payloads and persisted
    /// settings carry only the name, so absence always means "no choice yet".
    pub fn from_optional_name(name: Option<String>) -> Self {
        match name {
            Some(name) if !name.trim().is_empty() => Self::Named(name),
            _ => Self::Unset,
        }
    }
}

/// Run-wide configuration fields shared between the confirmation card
/// editor and the plan-card config block. Card-specific fields
/// (agent_run_configs, base_prompt, summary, skills)
/// remain on the per-view state structs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrchestrationConfigState {
    pub model_id: String,
    pub harness_type: String,
    pub execution_mode: RunAgentsExecutionMode,
    /// Per-call value hidden from the orchestration editors. Kept outside
    /// `execution_mode` so a temporary switch to Local does not discard it.
    remote_computer_use_enabled: bool,
    /// Drives the picker display and Accept gate. Persisted as
    /// `Named(_)` only via `CloudAgentSettings.last_selected_auth_secret`.
    pub auth_secret_selection: AuthSecretSelection,
}

impl OrchestrationConfigState {
    /// Returns the on-wire secret name; `None` for `Inherit`, `Unset`, or
    /// when the current mode/harness doesn't support managed auth secrets
    /// (Local, Oz, or harnesses without managed-secret types). Gating on
    /// visibility here prevents a stale `Named(_)` left over from a prior
    /// Cloud/non-Oz config from leaking into the on-wire payload after the
    /// user toggles to Local or switches to a harness without auth.
    pub fn auth_secret_name(&self) -> Option<&str> {
        if !should_show_auth_secret_picker(self) {
            return None;
        }
        match &self.auth_secret_selection {
            AuthSecretSelection::Named(name) => Some(name.as_str()),
            AuthSecretSelection::Inherit
            | AuthSecretSelection::Unset
            | AuthSecretSelection::CreatingNew => None,
        }
    }

    /// User picked "New API key…"; mark `CreatingNew` to block Accept until a
    /// key is created or another option is chosen.
    pub fn select_create_new_auth_secret(&mut self) {
        self.auth_secret_selection = AuthSecretSelection::CreatingNew;
    }
}

impl OrchestrationConfigState {
    pub(crate) fn sanitize_for_local_execution(&mut self) {
        let Some(harness) = Harness::parse_local_child_harness(&self.harness_type) else {
            return;
        };
        if local_harness_product_disabled_message(harness).is_some() {
            self.harness_type = "oz".to_string();
            self.model_id.clear();
        }
    }

    /// `None` (or an empty string wrapped in `Some`) leaves the field
    /// unset, matching the wire encoding where absence is emptiness.
    pub fn from_run_agents_fields(
        model_id: Option<&str>,
        harness_type: Option<&str>,
        execution_mode: &RunAgentsExecutionMode,
    ) -> Self {
        let remote_computer_use_enabled = match execution_mode {
            RunAgentsExecutionMode::Local => false,
            RunAgentsExecutionMode::Remote {
                computer_use_enabled,
                ..
            } => *computer_use_enabled,
        };

        Self {
            model_id: model_id.unwrap_or_default().to_string(),
            harness_type: harness_type.unwrap_or_default().to_string(),
            execution_mode: execution_mode.clone(),
            remote_computer_use_enabled,
            auth_secret_selection: AuthSecretSelection::Unset,
        }
    }

    pub fn from_orchestration_config(config: &OrchestrationConfig) -> Self {
        let execution_mode = match &config.execution_mode {
            OrchestrationExecutionMode::Local => RunAgentsExecutionMode::Local,
            OrchestrationExecutionMode::Remote {
                environment_id,
                worker_host,
            } => RunAgentsExecutionMode::Remote {
                environment_id: environment_id.clone(),
                worker_host: worker_host.clone(),
                computer_use_enabled: false,
            },
        };
        let mut state = Self {
            model_id: config.model_id.clone(),
            harness_type: config.harness_type.clone(),
            execution_mode,
            remote_computer_use_enabled: false,
            auth_secret_selection: AuthSecretSelection::Unset,
        };
        if matches!(state.execution_mode, RunAgentsExecutionMode::Local) {
            state.sanitize_for_local_execution();
        }
        state
    }

    /// Toggle Local ↔ Cloud. Resets OpenCode to Oz when switching
    /// to Cloud (unsupported combination).
    pub fn toggle_execution_mode_to_remote(&mut self, is_remote: bool) {
        if is_remote {
            if self.harness_type.eq_ignore_ascii_case("opencode") {
                self.harness_type = "oz".to_string();
            }
            if !self.execution_mode.is_remote() {
                self.execution_mode = RunAgentsExecutionMode::Remote {
                    environment_id: String::new(),
                    worker_host: ORCHESTRATION_WARP_WORKER_HOST.to_string(),
                    computer_use_enabled: self.remote_computer_use_enabled,
                };
            }
        } else {
            if let RunAgentsExecutionMode::Remote {
                computer_use_enabled,
                ..
            } = &self.execution_mode
            {
                self.remote_computer_use_enabled = *computer_use_enabled;
            }
            self.execution_mode = RunAgentsExecutionMode::Local;
            self.sanitize_for_local_execution();
        }
    }

    pub fn set_environment_id(&mut self, environment_id: String) {
        if let RunAgentsExecutionMode::Remote {
            environment_id: id, ..
        } = &mut self.execution_mode
        {
            *id = environment_id;
        }
    }

    pub fn set_worker_host(&mut self, worker_host: String) {
        if let RunAgentsExecutionMode::Remote {
            worker_host: wh, ..
        } = &mut self.execution_mode
        {
            *wh = worker_host;
        }
    }

    /// Returns `Some(reason)` if Accept / Apply must be disabled.
    /// Hard blocks: OpenCode + Cloud, and product-disabled local harnesses.
    pub fn accept_disabled_reason(&self) -> Option<&'static str> {
        match &self.execution_mode {
            RunAgentsExecutionMode::Local => Harness::parse_local_child_harness(&self.harness_type)
                .and_then(local_harness_product_disabled_message),
            RunAgentsExecutionMode::Remote { .. }
                if self.harness_type.eq_ignore_ascii_case("opencode") =>
            {
                Some(
                    "OpenCode is not supported on Cloud yet. Switch to Local or pick a different harness.",
                )
            }
            RunAgentsExecutionMode::Remote { .. } => None,
        }
    }

    /// Fills in empty fields from the approved orchestration config.
    /// When the LLM omits harness/model/execution_mode to inherit from
    /// the active config, the raw request arrives with defaults (empty
    /// harness, empty model, Local mode). This resolves those to the
    /// config values so the UI shows the intended settings.
    pub fn resolve_from_config(&mut self, config: &OrchestrationConfig) {
        if self.harness_type.is_empty() && !config.harness_type.is_empty() {
            self.harness_type = config.harness_type.clone();
        }
        if self.model_id.is_empty() && !config.model_id.is_empty() {
            self.model_id = config.model_id.clone();
        }
        if !self.execution_mode.is_remote() && config.execution_mode.is_remote() {
            self.execution_mode = Self::from_orchestration_config(config).execution_mode;
        }
        if matches!(self.execution_mode, RunAgentsExecutionMode::Local) {
            self.sanitize_for_local_execution();
        }
    }

    /// Unconditionally overrides model, harness, and execution mode
    /// from the approved orchestration config. The plan config is the
    /// user-approved source of truth — the LLM's run_agents call may
    /// omit or set these differently, but the config always wins.
    ///
    /// `computer_use_enabled` is preserved when the config changes execution
    /// mode, since it is a per-call flag rather than part of the approved plan
    /// configuration.
    pub fn override_from_approved_config(&mut self, config: &OrchestrationConfig) {
        self.model_id = config.model_id.clone();
        self.harness_type = config.harness_type.clone();
        if let RunAgentsExecutionMode::Remote {
            computer_use_enabled,
            ..
        } = &self.execution_mode
        {
            self.remote_computer_use_enabled = *computer_use_enabled;
        }

        self.execution_mode = Self::from_orchestration_config(config).execution_mode;

        if let RunAgentsExecutionMode::Remote {
            computer_use_enabled,
            ..
        } = &mut self.execution_mode
        {
            *computer_use_enabled = self.remote_computer_use_enabled;
        }
    }

    /// Converts to a native `OrchestrationConfig` for storage / match.
    pub fn to_orchestration_config(&self) -> OrchestrationConfig {
        let execution_mode = match &self.execution_mode {
            RunAgentsExecutionMode::Local => OrchestrationExecutionMode::Local,
            RunAgentsExecutionMode::Remote {
                environment_id,
                worker_host,
                ..
            } => OrchestrationExecutionMode::Remote {
                environment_id: environment_id.clone(),
                worker_host: worker_host.clone(),
            },
        };
        OrchestrationConfig {
            model_id: self.model_id.clone(),
            harness_type: self.harness_type.clone(),
            execution_mode,
        }
    }
}

#[cfg(test)]
#[path = "config_state_tests.rs"]
mod tests;

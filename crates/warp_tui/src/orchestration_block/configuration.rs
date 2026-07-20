//! Configuration pages and shared-model adapters for the orchestration card.

use warp::tui_export::{
    AIActionStatus, AIAgentActionId, BlocklistAIActionModel, OptionSnapshot,
    OrchestrationConfigState, OrchestrationEditState, RunAgentsExecutionMode, RunAgentsRequest,
    accept_disabled_reason_with_auth, api_key_snapshot, environment_snapshot, harness_snapshot,
    host_snapshot, location_snapshot, model_snapshot, persist_environment_selection,
    persist_host_selection,
};
use warpui_core::{AppContext, ModelHandle};

/// Row id emitted by `location_snapshot` for remote execution.
const LOCATION_CLOUD_ID: &str = "cloud";

/// Applies the TUI policy that Local configuration has no harness page and
/// therefore always uses Oz.
pub(super) fn normalize_tui_local_harness(state: &mut OrchestrationConfigState) {
    if matches!(state.execution_mode, RunAgentsExecutionMode::Local)
        && !state.harness_type.trim().is_empty()
        && !state.harness_type.eq_ignore_ascii_case("oz")
    {
        state.harness_type = "oz".to_string();
        state.model_id.clear();
    }
}

/// Builds a dispatched request from immutable card fields and edited run-wide state.
pub(super) fn build_request(
    fields: &RunAgentsRequest,
    state: &OrchestrationConfigState,
) -> RunAgentsRequest {
    RunAgentsRequest {
        summary: fields.summary.clone(),
        base_prompt: fields.base_prompt.clone(),
        skills: fields.skills.clone(),
        model_id: state.model_id.clone(),
        harness_type: state.harness_type.clone(),
        execution_mode: state.execution_mode.clone(),
        agent_run_configs: fields.agent_run_configs.clone(),
        plan_id: fields.plan_id.clone(),
        harness_auth_secret_name: state.auth_secret_name().map(str::to_string),
    }
}

/// One single-field configuration page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ConfigPage {
    Location,
    Harness,
    ApiKey,
    Host,
    Environment,
    Model,
}

impl ConfigPage {
    /// Returns the page question with the request's agent count pluralized.
    pub(super) fn question(self, agent_count: usize) -> String {
        let agent = if agent_count == 1 { "agent" } else { "agents" };
        match self {
            Self::Location => format!("Where should the {agent} run?"),
            Self::Harness => format!("Which harness should the {agent} use?"),
            Self::ApiKey => format!("Which API key should the {agent} use?"),
            Self::Host => format!("Which host should run the {agent}?"),
            Self::Environment => format!("Which environment should the {agent} use?"),
            Self::Model => format!("Which model should the {agent} use?"),
        }
    }

    /// Whether this page opts into the selector's pinned search editor.
    pub(super) fn is_searchable(self) -> bool {
        matches!(self, Self::Model)
    }
}

/// External orchestration behavior used by the block.
pub(super) trait OrchestrationBlockController {
    /// Returns the current lifecycle status for the action.
    fn action_status(
        &self,
        action_id: &AIAgentActionId,
        ctx: &AppContext,
    ) -> Option<AIActionStatus>;

    /// Builds the current option snapshot for a configuration page.
    fn snapshot_for_page(
        &self,
        page: ConfigPage,
        state: &OrchestrationConfigState,
        ctx: &AppContext,
    ) -> OptionSnapshot;

    /// Commits one page selection into the edit state.
    fn apply_page_selection(
        &self,
        page: ConfigPage,
        id: &str,
        edit_state: &mut OrchestrationEditState,
        fallback_base_model_id: Option<String>,
        ctx: &mut AppContext,
    );

    /// Validates and dispatches an accepted request, returning the blocking
    /// reason when the edited configuration cannot launch.
    fn accept(
        &self,
        action_id: &AIAgentActionId,
        request: RunAgentsRequest,
        state: &OrchestrationConfigState,
        ctx: &mut AppContext,
    ) -> Result<(), String>;
}

/// Production controller backed by the shared orchestration models.
pub(super) struct ModelOrchestrationBlockController {
    pub(super) action_model: ModelHandle<BlocklistAIActionModel>,
}

impl OrchestrationBlockController for ModelOrchestrationBlockController {
    fn action_status(
        &self,
        action_id: &AIAgentActionId,
        ctx: &AppContext,
    ) -> Option<AIActionStatus> {
        self.action_model.as_ref(ctx).get_action_status(action_id)
    }

    fn snapshot_for_page(
        &self,
        page: ConfigPage,
        state: &OrchestrationConfigState,
        ctx: &AppContext,
    ) -> OptionSnapshot {
        match page {
            ConfigPage::Location => location_snapshot(state, ctx),
            ConfigPage::Harness => harness_snapshot(state, ctx),
            ConfigPage::ApiKey => api_key_snapshot(state, ctx),
            ConfigPage::Host => host_snapshot(state, ctx),
            ConfigPage::Environment => environment_snapshot(state, ctx),
            ConfigPage::Model => model_snapshot(state, ctx),
        }
    }

    fn apply_page_selection(
        &self,
        page: ConfigPage,
        id: &str,
        edit_state: &mut OrchestrationEditState,
        fallback_base_model_id: Option<String>,
        ctx: &mut AppContext,
    ) {
        match page {
            ConfigPage::Location => {
                let is_remote = id == LOCATION_CLOUD_ID;
                if !is_remote {
                    // For now, we only allow local runs to use the oz harness
                    edit_state.apply_harness_change("oz", fallback_base_model_id.clone(), ctx);
                }

                edit_state
                    .orchestration_config_state
                    .apply_execution_mode_change(is_remote, fallback_base_model_id, ctx);
                normalize_tui_local_harness(&mut edit_state.orchestration_config_state);
            }
            ConfigPage::Harness => {
                edit_state.apply_harness_change(id, fallback_base_model_id, ctx);
            }
            ConfigPage::ApiKey => {
                let name = (!id.is_empty()).then(|| id.to_string());
                edit_state
                    .orchestration_config_state
                    .apply_auth_secret_change(name, ctx);
            }
            ConfigPage::Host => {
                edit_state
                    .orchestration_config_state
                    .set_worker_host(id.to_string());
                persist_host_selection(id, ctx);
            }
            ConfigPage::Environment => {
                edit_state
                    .orchestration_config_state
                    .set_environment_id(id.to_string());
                persist_environment_selection(id, ctx);
            }
            ConfigPage::Model => {
                edit_state.orchestration_config_state.model_id = id.to_string();
            }
        }
    }

    fn accept(
        &self,
        action_id: &AIAgentActionId,
        request: RunAgentsRequest,
        state: &OrchestrationConfigState,
        ctx: &mut AppContext,
    ) -> Result<(), String> {
        if let Some(reason) = accept_disabled_reason_with_auth(state, ctx) {
            return Err(reason);
        }
        self.action_model.update(ctx, |action_model, ctx| {
            action_model.execute_run_agents(action_id, request, ctx);
        });
        Ok(())
    }
}

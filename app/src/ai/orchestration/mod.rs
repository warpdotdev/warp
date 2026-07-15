//! Frontend-neutral orchestration domain: edit state, transitions,
//! validation, and catalog providers shared by the GUI orchestration
//! controls and the TUI orchestration card.
//!
//! Nothing in this module may depend on `warpui::elements` or any other
//! GUI rendering types; it only reads/writes app singletons through
//! `AppContext`.

mod config_state;
mod edit_state;
mod providers;
mod validation;

pub use config_state::{AuthSecretSelection, OrchestrationConfigState};
pub use edit_state::OrchestrationEditState;
pub(crate) use providers::{
    can_execute_with_auth_secret, get_base_model_choices, persist_auth_secret_selection,
    populate_default_auth_secret_for_execution,
};
pub use providers::{
    persist_environment_selection, persist_host_selection,
    resolve_auth_secret_selection_for_harness, resolve_default_environment_id,
    resolve_default_host_slug, resolve_recent_host_slug, ORCHESTRATION_ENV_NONE_LABEL,
    ORCHESTRATION_WARP_WORKER_HOST,
};
// Consumed by the TUI via `tui_export`; the GUI gates Accept through
// `accept_disabled_reason_with_auth` instead.
#[cfg_attr(not(feature = "tui"), allow(unused_imports))]
pub use validation::auth_secret_selection_required;
pub use validation::{
    accept_disabled_reason_with_auth, empty_env_recommendation_message, harness_is_selectable,
    should_show_auth_secret_picker,
};

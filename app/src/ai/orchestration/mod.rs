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
mod snapshots;
mod validation;

pub use config_state::{AuthSecretSelection, OrchestrationConfigState};
pub use edit_state::OrchestrationEditState;
// `ORCHESTRATION_ENV_NONE_LABEL` is only consumed by the TUI via
// `tui_export`; the GUI reads it from the environment snapshot rows.
#[cfg_attr(not(feature = "tui"), allow(unused_imports))]
pub use providers::ORCHESTRATION_ENV_NONE_LABEL;
pub(crate) use providers::{
    can_execute_with_auth_secret, persist_auth_secret_selection,
    populate_default_auth_secret_for_execution,
};
pub use providers::{
    persist_environment_selection, persist_host_selection,
    resolve_auth_secret_selection_for_harness, resolve_default_environment_id,
    resolve_default_host_slug, ORCHESTRATION_WARP_WORKER_HOST,
};
pub(crate) use snapshots::AUTH_SECRET_INHERIT_LABEL;
pub use snapshots::{
    api_key_snapshot, environment_snapshot, harness_snapshot, host_snapshot, model_snapshot,
    OptionBadge, OptionFooter, OptionSnapshot, OptionSourceStatus,
};
// `location_snapshot` and `OptionRow` are only named by the TUI (via
// `tui_export`); the GUI renders its own Cloud/Local mode toggle and
// destructures snapshot rows without naming the type.
#[cfg_attr(not(feature = "tui"), allow(unused_imports))]
pub use snapshots::{location_snapshot, OptionRow};
pub use validation::{
    accept_disabled_reason_with_auth, empty_env_recommendation_message,
    should_show_auth_secret_picker,
};
// These validation predicates back the shared snapshot builders and are
// re-exported for the TUI via `tui_export`.
#[cfg_attr(not(feature = "tui"), allow(unused_imports))]
pub use validation::{auth_secret_selection_required, harness_is_selectable};

pub mod app_id;
pub mod assertions;
pub mod r#async;
pub mod channel;
pub mod command;
pub mod context_flag;
pub mod errors;
pub mod execution_mode;
pub mod features;
pub mod interval_timer;
#[cfg(target_os = "macos")]
pub mod macos;
pub mod operating_system_info;
pub mod paths;
pub mod platform;
pub mod safe_log;
pub mod semantic_selection;
pub use settings;
// Re-export settings macros for backward compatibility
pub use settings::{
    define_setting, define_settings_group, implement_setting_for_enum, maybe_define_setting,
};
pub mod session_id;
pub mod sync_queue;
pub mod telemetry;
pub mod ui;
pub mod user_preferences;

// Re-export anyhow so the `safe_assert!` macros' string-literal form can build
// an `anyhow::Error` without callers needing `anyhow` in scope.
#[doc(hidden)]
pub use anyhow as __anyhow;
pub use app_id::AppId;
pub use session_id::SessionId;
// The error-reporting macros now live in the `warp_errors` leaf crate; re-export
// them at the crate root so `report_error!` etc. keep working.
pub use warp_errors::{register_error, report_error, report_if_error};
pub use warp_util::host_id::HostId;
// Re-export warpui_core so that it can be referenced safely from the
// telemetry macros.
pub use warpui_core;

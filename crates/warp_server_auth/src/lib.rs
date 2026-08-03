pub mod anonymous_id;
pub mod auth_state;
pub mod credentials;
pub mod user;
pub mod user_uid;

pub use auth_state::AuthStateProvider;
pub use user_uid::UserUid;

/// Prefix for API keys used in authentication.
#[cfg_attr(target_family = "wasm", allow(dead_code))]
pub const API_KEY_PREFIX: &str = "wk-";

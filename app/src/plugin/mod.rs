pub(crate) mod app;
pub(crate) mod service;

#[path = "host/native/mod.rs"]
mod host;

pub(crate) use app::PluginHost;
pub use host::run as run_plugin_host;

/// Flag to be passed to the warp executable when executing the warp binary as the plugin host
/// process rather than the main app.
pub const PLUGIN_HOST_FLAG: &str = "--plugin_host";

/// The name of the environment variable used to pass connection address for the app server to the
/// plugin host process.
const PLUGIN_HOST_ADDRESS_ENV_VAR: &str = "WARP_PLUGIN_HOST_ADDRESS";

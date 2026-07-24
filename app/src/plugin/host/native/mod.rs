mod js_api;
mod logging;
mod plugin;
mod plugin_caller;
mod plugin_ref;
mod runner;
mod runners;
mod service_impl;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use logging::initialize_logging;
use warpui::r#async::executor::Background;

use self::plugin_caller::PluginCaller;
use self::plugin_ref::PluginRef;
use self::runner::PLUGIN_ENTRYPOINT_JS_FILE_NAME;
use self::service_impl::CallJsFunctionServiceImpl;
use super::PLUGIN_HOST_ADDRESS_ENV_VAR;
use super::service::{
    PluginHostBootstrapRequest, PluginHostBootstrapResponse, PluginHostBootstrapService,
};
use crate::plugin::host::runners::PluginRunners;

pub fn run() -> Result<()> {
    warpui::r#async::block_on(async move {
        let executor = Arc::new(Background::default());

        // Initialize a client connection to the warp app process.
        let connection_address: ipc::ConnectionAddress = std::env::var(PLUGIN_HOST_ADDRESS_ENV_VAR)
            .context("Failed to retrieve connection key from env var.")?
            .into();
        let client = Arc::new(
            ipc::Client::connect(connection_address.clone(), executor.clone())
                .await
                .context("Failed to instantiate LocalSocketClient.")?,
        );

        // Initialize logging, which internally transmits logs from this process to the warp app
        // process via `LogService`.
        initialize_logging(&client, &executor);

        // Spawn plugin runners for each plugin.
        let (plugin_request_tx, plugin_request_rx) = async_channel::unbounded();
        let mut plugin_runners =
            PluginRunners::new(plugin_request_rx, client.clone(), executor.clone());

        #[cfg(feature = "completions_v2")]
        plugin_runners.run_plugin(PluginRef::BuiltIn(
            plugin_ref::BuiltInPluginType::Completions,
        ));

        for plugin_path in plugin_paths() {
            plugin_runners.run_plugin(PluginRef::Path(plugin_path));
        }

        // Initialize the plugin host server, which serves services implemented by plugins.
        let (_server, plugin_host_connection_key) = ipc::ServerBuilder::default()
            .with_service(CallJsFunctionServiceImpl::new(PluginCaller::new(
                plugin_request_tx,
            )))
            .build_and_run(executor.clone())
            .expect("Failed to instantiate Plugin Host server ");

        // Send the connection address for the plugin host server back to the warp app.
        let bootstrap_service = ipc::service_caller::<PluginHostBootstrapService>(client.clone());
        if !matches!(
            bootstrap_service
                .call(PluginHostBootstrapRequest {
                    connection_address: plugin_host_connection_key,
                })
                .await,
            Ok(PluginHostBootstrapResponse { success: true })
        ) {
            return Err(anyhow!(
                "Handshake for plugin host server connection address failed."
            ));
        }

        // Wait for the connection to be dropped to exit.
        client.wait_for_disconnect().await;
        Ok(())
    })
}

/// Returns a vector of validated plugin directory paths in the plugins directory.
///
/// This assumes that all plugins are located in ~/.warp/plugins.
fn plugin_paths() -> Vec<PathBuf> {
    const PLUGIN_PATH_SUFFIX: &str = ".warp/plugins";

    dirs::home_dir()
        .map(|home_dir| home_dir.join(PLUGIN_PATH_SUFFIX))
        .and_then(|plugins_dir| fs::read_dir(plugins_dir).ok())
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            if is_plugin_dir(path.as_path()) {
                log::info!("Plugin detected at {path:?}");
                Some(path)
            } else {
                None
            }
        })
        .collect()
}

/// Returns `true` if the directory at the given path is directory containing JS source for a Warp plugin.
fn is_plugin_dir(path: &Path) -> bool {
    if !path.is_dir() {
        return false;
    }

    let expected_plugin_entrypoint = path.join(PLUGIN_ENTRYPOINT_JS_FILE_NAME);
    expected_plugin_entrypoint.is_file()
}

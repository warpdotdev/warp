mod service_impl;

use std::sync::Arc;

use anyhow::{Context, Result};
use command::blocking::Command;
use service_impl::{LogServiceImpl, PluginHostBootstrapServiceImpl};
use warp_errors::report_error;
use warpui::{Entity, ModelContext, SingletonEntity};

use super::{PLUGIN_HOST_ADDRESS_ENV_VAR, PLUGIN_HOST_FLAG};

/// Singleton model responsible for spawning the plugin host child process and initializing IPC
/// server and clients for communication between the app and plugin host processes.
pub struct PluginHost {
    /// A handle on the actual plugin host process.
    ///
    /// This is `None` if we fail to spawn the plugin host process.
    host_process: Option<std::process::Child>,

    /// The IPC server that serves app services ([`ipc::Service`] implementations) to the plugin
    /// host process.
    ///
    /// This is `None` if server initialization fails.
    _server: Option<ipc::Server>,

    /// An IPC client for sending requests to the plugin host process.
    ///
    /// This is `None` if the IPC handshake for relaying the plugin host process's connection
    /// address fails.
    host_client: Option<Arc<ipc::Client>>,
}

impl PluginHost {
    #[cfg_attr(not(feature = "plugin_host"), allow(dead_code))]
    pub fn new(ctx: &mut ModelContext<Self>) -> Result<Self> {
        let plugin_host_bootstrap_service = PluginHostBootstrapServiceImpl::new();
        let connection_address_rx = plugin_host_bootstrap_service.connection_address_rx();

        // Schedule a task that awaits a request containing the connection address for the plugin
        // host process and uses it to instantiate a Client when it's received.
        let background_executor = ctx.background_executor();
        ctx.spawn(
            async move {
                match connection_address_rx.recv().await {
                    Ok(connection_address) => {
                        match ipc::Client::connect(connection_address, background_executor).await {
                            Ok(client) => Some(client),
                            Err(e) => {
                                report_error!(
                                    anyhow::Error::new(e)
                                        .context("Failed to instantiate LocalSocketClient")
                                );
                                None
                            }
                        }
                    }
                    Err(e) => {
                        report_error!(anyhow::Error::new(e).context(
                            "Failed to receive connection address for PluginHost services"
                        ));
                        None
                    }
                }
            },
            |me, client, _| {
                me.host_client = client.map(Arc::new);
            },
        );

        let server_builder = ipc::ServerBuilder::default()
            .with_service(plugin_host_bootstrap_service)
            .with_service(LogServiceImpl::new());

        #[cfg(feature = "completions_v2")]
        let server_builder =
            server_builder.with_service(service_impl::RegisterCommandSignatureServiceImpl::new(
                warp_completer::signatures::CommandRegistry::global_instance(),
            ));

        let (server, plugin_host_process) =
            match server_builder.build_and_run(ctx.background_executor()) {
                Ok((server, connection_address)) => {
                    log::info!("Successfully initialized plugin app server.");

                    // Spawn the plugin host process if the app server was successfully initialized.
                    let program = std::env::current_exe()
                        .context("Failed to determine path to current executable.")?;
                    let plugin_host_process = Command::new(program)
                        .args(std::env::args().skip(1))
                        .arg(PLUGIN_HOST_FLAG)
                        .env(PLUGIN_HOST_ADDRESS_ENV_VAR, connection_address.to_string())
                        .spawn()
                        .context("Failed to spawn plugin host process.")?;
                    log::info!("Successfully spawned plugin host process.");

                    (Some(server), Some(plugin_host_process))
                }
                Err(e) => {
                    report_error!(anyhow::Error::new(e).context("Could not initialize server"));
                    (None, None)
                }
            };

        Ok(Self {
            host_process: plugin_host_process,
            _server: server,
            host_client: None,
        })
    }

    /// Returns an `ipc::ServiceCaller` for the service specified as `S`.
    ///
    /// `S` is assumed to be served by the plugin host process; the returned service caller directs
    /// requests over the IPC connection to the plugin host process.
    pub fn plugin_service_caller<S: ipc::Service>(&self) -> Option<Box<dyn ipc::ServiceCaller<S>>> {
        self.host_client.clone().map(ipc::service_caller::<S>)
    }
}

impl Drop for PluginHost {
    fn drop(&mut self) {
        if let Some(mut host_process) = self.host_process.take() {
            if let Ok(Some(exit_status)) = host_process.try_wait() {
                report_error!(
                    "Plugin host process had exited early",
                    extra: { "status" => ?exit_status }
                );
            } else {
                // Calling `wait()` is necessary for the OS to release process resources on some
                // systems; processes that have exited but not been `wait`-ed upon are "zombie"
                // processes that can exhaust OS resources.
                //
                // See https://doc.rust-lang.org/std/process/struct.Child.html#warning for more
                // context.
                let _ = host_process.kill();
                let _ = host_process.wait();
            }
        }
    }
}

impl Entity for PluginHost {
    type Event = ();
}

impl SingletonEntity for PluginHost {}

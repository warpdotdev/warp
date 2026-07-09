use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use async_channel::Receiver;
use futures::channel::oneshot;
use parking_lot::Mutex;
use warp_errors::report_error;
use warp_js::JsFunctionId;
use warpui::r#async::executor::Background;

use super::plugin::{AppServiceCallers, PluginRequest, PluginResponse};
use super::plugin_ref::PluginRef;
use super::runner::PluginRunner;

/// Message type for messages that may be sent to each `PluginRunner`.
///
/// This message type only exists for `PluginRunners` -> `PluginRunner` communication.
pub(super) enum PluginRunnerMessage {
    /// A request to execute some plugin logic and send back its output.
    Request(PluginRequest, oneshot::Sender<PluginResponse>),

    /// The plugin runner that receives this should exit.
    Exit,
}

#[derive(Clone)]
struct PluginRunnerSender {
    /// A `Sender` for emitting `PlugginRunnerMessage`s to the plugin runner.
    runner_tx: async_channel::Sender<PluginRunnerMessage>,

    /// The set of IDs for JsFunctions registered by the plugin runner that owns the receiving end
    /// of `runner_tx`.
    registered_js_function_ids: HashSet<JsFunctionId>,
}

/// Responsible for spawning threads for [`PluginRunner`]s, broadcasting incoming `PluginRequest`s
/// to them, and aggregating their responses into one to be dispatched back to the `PluginRequest`
/// caller.
pub(super) struct PluginRunners {
    plugin_runner_senders: Arc<Mutex<HashMap<PluginRef, PluginRunnerSender>>>,
    app_client: Arc<ipc::Client>,
}

impl PluginRunners {
    pub(super) fn new(
        plugin_request_rx: Receiver<(PluginRequest, oneshot::Sender<PluginResponse>)>,
        app_client: Arc<ipc::Client>,
        executor: Arc<Background>,
    ) -> Self {
        let plugin_runner_senders = Arc::new(Mutex::new(HashMap::new()));
        executor
            .spawn(proxy_requests_to_plugin_runners(
                executor.clone(),
                plugin_request_rx,
                plugin_runner_senders.clone(),
            ))
            .detach();
        Self {
            plugin_runner_senders,
            app_client,
        }
    }

    /// Spawns a new thread to execute the plugin at the given `path`.
    ///
    /// If there is an existing runner for a plugin at the given path, kills it and starts a new
    /// runner. This effectively "reloads" the plugin.
    pub(super) fn run_plugin(&mut self, plugin_ref: PluginRef) {
        // If there's a live plugin runner for a plugin at this path, attempt to kill it.
        if let Some(PluginRunnerSender { runner_tx, .. }) =
            self.plugin_runner_senders.lock().remove(&plugin_ref)
        {
            let _ = runner_tx.try_send(PluginRunnerMessage::Exit);
        }

        let (message_tx, message_rx) = async_channel::unbounded::<PluginRunnerMessage>();
        self.plugin_runner_senders.lock().insert(
            plugin_ref.clone(),
            PluginRunnerSender {
                runner_tx: message_tx,
                registered_js_function_ids: HashSet::new(),
            },
        );

        let app_client = self.app_client.clone();
        let plugin_ref_clone = plugin_ref.clone();
        let plugin_runner_senders = self.plugin_runner_senders.clone();
        std::thread::spawn(move || {
            let registered_js_function_id = move |id: JsFunctionId| {
                if let Some(registered_js_function_ids) = plugin_runner_senders
                    .lock()
                    .get_mut(&plugin_ref_clone)
                    .map(|sender| &mut sender.registered_js_function_ids)
                {
                    registered_js_function_ids.insert(id);
                }
            };
            let Ok(mut runner) = PluginRunner::new(
                message_rx,
                AppServiceCallers::new(app_client),
                registered_js_function_id,
            ) else {
                report_error!(
                    "Failed to instantiate PluginRunner for plugin",
                    extra: { "plugin_ref" => ?plugin_ref }
                );
                return;
            };
            if let Err(e) =
                anyhow::Context::context(runner.run(&plugin_ref), "Failed to run plugin")
            {
                report_error!(e);
            }
        });
    }
}

/// Proxies [`PluginRequest`]s received via the given `request_rx` to each plugin runner.
///
/// For each received request, a corresponding `PluginRunnerMessage` is sent to the individual
/// plugin runner that registered the requested JS function.
async fn proxy_requests_to_plugin_runners(
    executor: Arc<Background>,
    request_rx: Receiver<(PluginRequest, oneshot::Sender<PluginResponse>)>,
    plugin_runner_senders: Arc<Mutex<HashMap<PluginRef, PluginRunnerSender>>>,
) {
    while let Ok((request, response_tx)) = request_rx.recv().await {
        // Find the plugin runner which hosts the JS function being called.
        let Some(matching_plugin_runner_tx) = plugin_runner_senders
            .lock()
            .values()
            .find(|sender| match request {
                PluginRequest::CallJsFunction { id, .. } => {
                    sender.registered_js_function_ids.contains(&id)
                }
            })
            .map(|sender| sender.runner_tx.clone())
        else {
            log::warn!("No plugin runner found for request {request:?}");
            continue;
        };

        executor
            .spawn(async move {
                let (runner_response_tx, runner_response_rx) = oneshot::channel();

                if let Err(e) = matching_plugin_runner_tx
                    .send(PluginRunnerMessage::Request(
                        request.clone(),
                        runner_response_tx,
                    ))
                    .await
                {
                    log::warn!("Failed to dispatch request to plugin runner: {e:?}");
                }

                match runner_response_rx.await {
                    Ok(response) => {
                        let _ = response_tx.send(response);
                    }
                    Err(e) => {
                        log::warn!("Received error when awaiting plugin runner response: {e:?}");
                    }
                }
            })
            .detach();
    }
}

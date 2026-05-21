use anyhow::{Context as AnyhowContext, Result};
use async_channel::Receiver;
use rquickjs::{Context, Function, Runtime};
use warp_js::JsFunctionId;

use super::js_api;
use super::plugin::{AppServiceCallers, PluginHandle};
use super::plugin_ref::PluginRef;
use super::runners::PluginRunnerMessage;

/// The name of the entrypoint JS file for a plugin.
pub(super) const PLUGIN_ENTRYPOINT_JS_FILE_NAME: &str = "main.js";

/// A "runner" for a single JS plugin.
///
/// This struct wraps a single QuickJS runtime. It is primarily responsible for loading, compiling,
/// and running the plugin.
pub(super) struct PluginRunner {
    /// The execution context of the QuickJS runtime.
    ctx: Context,

    /// A handle on the actual JS "plugin" logic (e.g. exports of the plugin JS represented in Rust).
    plugin: PluginHandle,

    /// A receiver for [`PluginRunnerMessage`]s received from the plugin host main thread.
    message_rx: Receiver<PluginRunnerMessage>,
}

impl PluginRunner {
    /// Instantiates a new [`PluginRunner`].
    pub(super) fn new(
        message_rx: Receiver<PluginRunnerMessage>,
        app_service_callers: AppServiceCallers,
        on_registered_js_function_callback: impl Fn(JsFunctionId) + 'static,
    ) -> Result<Self> {
        let rt = Runtime::new().context("Could not instantiate runtime")?;
        let ctx = Context::full(&rt).context("Could not instantiate context.")?;

        Ok(Self {
            ctx,
            message_rx,
            plugin: PluginHandle::new(app_service_callers, on_registered_js_function_callback),
        })
    }

    /// Loads and evaluates the plugin source JS, and then runs a blocking "event loop" to serve
    /// incoming [`PluginRequest`]s.
    ///
    /// After compiling the plugin module, its exported 'activate()' function is called with an
    /// instance of the warp API object.
    ///
    /// After `activate()`, listens for incoming [`PluginRequest`]s from the host main thread and
    /// serves corresponding responses.
    pub(super) fn run(&mut self, plugin_ref: &PluginRef) -> Result<()> {
        let plugin_source_bytes = plugin_ref.plugin_bytes()?;
        let plugin = self.plugin.clone();
        self.ctx.with(|ctx| -> Result<()> {
            let plugin_module = ctx
                .compile("plugin", plugin_source_bytes)
                .context("Could not compile plugin source")?;

            ctx.globals().set("console", js_api::console(ctx))?;

            let warp_api = js_api::warp(plugin, ctx);

            let activate_fn: Function = plugin_module
                .get("activate")
                .context("Could not resolve activate() function")?;
            activate_fn
                .call::<_, ()>((warp_api,))
                .context("Failed to call activate() function")?;

            Ok(())
        })?;

        while let Ok(message) = self.message_rx.recv_blocking() {
            match message {
                PluginRunnerMessage::Request(request, response_tx) => {
                    if let Ok(output) = self.plugin.get_mut().handle_request(request, &self.ctx) {
                        // If the consuming end of the output response is dropped, we don't really
                        // care (just means that the caller no longer cares about the response).
                        let _ = response_tx.send(output);
                    }
                }
                PluginRunnerMessage::Exit => break,
            }
        }
        log::info!("Plugin thread exiting...");
        Ok(())
    }
}

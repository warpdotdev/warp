use std::cell::{RefCell, RefMut};
use std::rc::Rc;
use std::sync::Arc;

use anyhow::anyhow;
use rquickjs::Context;
use warp_js::{JsFunctionId, JsFunctionRegistry, SerializedJsValue};

cfg_if::cfg_if! {
    if #[cfg(feature = "completions_v2")] {
        use warp_completer::signatures::CommandSignature;

        use crate::plugin::service::{
            RegisterCommandSignatureRequest, RegisterCommandSignatureService,
        };
    }
}

/// A handle on a single JS plugin.
///
/// We use a `Rc<RefCell<_>>` to make it possible for the plugin "API" (JS functions implemented in
/// Rust) _and_ native Rust logic to share ownership over the plugin "state".  We know that, since
/// JS is singlethreaded, the JS-side logic will never actually ask for a mutable borrow at the same
/// time as host-side Rust.
#[derive(Clone)]
pub struct PluginHandle(Rc<RefCell<Plugin>>);

impl PluginHandle {
    pub(super) fn new(
        app_service_callers: AppServiceCallers,
        on_registered_js_function_callback: impl Fn(JsFunctionId) + 'static,
    ) -> Self {
        Self(Rc::new(RefCell::new(Plugin::new(
            app_service_callers,
            on_registered_js_function_callback,
        ))))
    }

    pub(super) fn get_mut(&self) -> RefMut<'_, Plugin> {
        (*self.0).borrow_mut()
    }
}

/// Container struct for holding ipc `ServiceCaller` dependencies of `Plugin`.
pub(super) struct AppServiceCallers {
    #[cfg(feature = "completions_v2")]
    register_command_signatures_caller:
        Box<dyn ipc::ServiceCaller<RegisterCommandSignatureService>>,
}

impl AppServiceCallers {
    pub fn new(#[allow(unused_variables)] app_client: Arc<ipc::Client>) -> Self {
        Self {
            #[cfg(feature = "completions_v2")]
            register_command_signatures_caller: ipc::service_caller::<
                RegisterCommandSignatureService,
            >(app_client),
        }
    }
}

/// Represents logic and functionality implemented by a Plugin in JS.
///
/// Conceptually, this an intermediate abstraction between plugin JS and "host-side" Rust.
///
/// Concretely, this can generally be viewed as a wrapper around functions implemented by the
/// plugin in JavaScript, exposing APIs to call such JS functions from Rust.
///
/// This ultimately the main backing data structure behind the JS plugin API exposed to plugins
/// (see [`super::js_api`]).
pub(super) struct Plugin {
    app_services: AppServiceCallers,
    js_function_registry: JsFunctionRegistry,
}

impl Plugin {
    /// Handles the given `request` and returns its corresponding response.
    ///
    /// Generally, should return a `PluginResponse` enum variant that matches the request variant.
    pub(super) fn handle_request(
        &mut self,
        request: PluginRequest,
        ctx: &Context,
    ) -> anyhow::Result<PluginResponse> {
        match request {
            PluginRequest::CallJsFunction { id, input } => self
                .call_js_function(input, &id, ctx)
                .map(|serialized_value| PluginResponse::CallJsFunctionResult {
                    output: serialized_value,
                }),
        }
    }

    /// Registers the given command signatures.
    #[cfg(feature = "completions_v2")]
    pub(super) fn register_command_signatures(&mut self, signatures: Vec<CommandSignature>) {
        if let Err(e) = warpui::r#async::block_on(
            self.app_services
                .register_command_signatures_caller
                .call(RegisterCommandSignatureRequest { signatures }),
        ) {
            log::warn!("Failed to register command signature: {e:?}");
        }
    }

    pub(super) fn js_function_registry_mut(&mut self) -> &mut JsFunctionRegistry {
        &mut self.js_function_registry
    }

    /// Calls the js function with the given function_id and input, if registered.
    ///
    /// The function with the given `function_id` is expected to be registered in this `Plugin`'s
    /// `JsFunctionRegistry`. If the function_id has no registered function, returns an error.
    fn call_js_function(
        &mut self,
        input: SerializedJsValue,
        function_id: &JsFunctionId,
        ctx: &Context,
    ) -> anyhow::Result<SerializedJsValue> {
        let Some(function) = self.js_function_registry.get_function(function_id) else {
            return Err(anyhow!(
                "Attempted to call unregistered JS Function with ID {:?}.",
                function_id
            ));
        };
        Ok(ctx.with(|ctx| function.call(input, &mut self.js_function_registry, ctx))?)
    }

    fn new(
        app_services: AppServiceCallers,
        on_registered_js_function_callback: impl Fn(JsFunctionId) + 'static,
    ) -> Self {
        Self {
            js_function_registry: JsFunctionRegistry::new()
                .on_registered_js_function(on_registered_js_function_callback),
            app_services,
        }
    }
}

/// A request to be served by a plugin.
#[derive(Debug, Clone)]
pub(super) enum PluginRequest {
    /// Request for execution of a JS function with the contained `id` and `input` to be passed as
    /// a parameter. `input` should be the serialized bytes representation of the
    /// `IntoPluginJs`-implementing Rust struct to be passed to the JS function.
    CallJsFunction {
        id: JsFunctionId,
        input: SerializedJsValue,
    },
}

/// The response to a PluginRequest.
#[derive(Debug)]
pub(super) enum PluginResponse {
    /// Result of JS function execution requested via `PluginRequest::CallJsFunction`.
    CallJsFunctionResult {
        /// The serialized bytes representation of the `FromPluginJs`-implementing Rust value that
        /// is the return value of the JS function.
        ///
        /// If this response is coming from a thread that does "host" the JS function (e.g. it was
        /// registered by a different plugin), this is `None`.
        output: SerializedJsValue,
    },
}

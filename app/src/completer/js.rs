//! Implementation of `JsExecutionContext` for V2 completions in terminal sessions.
//!
//! This calls out to the `CallJsFunctionService` IPC service, which is served by the plugin host
//! process where JS plugins are loaded and executed.
use std::sync::Arc;

use async_trait::async_trait;
use ipc::ServiceCaller;
use warp_completer::completer::{JsExecutionContext, JsExecutionError};
use warp_js::{JsFunctionId, SerializedJsValue};

use crate::plugin::service::{
    CallJsFunctionRequest, CallJsFunctionResponse, CallJsFunctionService,
};

#[derive(Clone)]
pub struct SessionJsExecutionContext {
    js_function_caller: Arc<dyn ServiceCaller<CallJsFunctionService>>,
}

impl SessionJsExecutionContext {
    pub fn new(js_function_caller: Box<dyn ServiceCaller<CallJsFunctionService>>) -> Self {
        Self {
            js_function_caller: Arc::from(js_function_caller),
        }
    }
}

#[async_trait]
impl JsExecutionContext for SessionJsExecutionContext {
    async fn call_js_function(
        &self,
        input: SerializedJsValue,
        function_id: JsFunctionId,
    ) -> Result<SerializedJsValue, JsExecutionError> {
        let response = self
            .js_function_caller
            .call(CallJsFunctionRequest {
                id: function_id,
                serialized_input: input,
            })
            .await;

        match response {
            Ok(CallJsFunctionResponse::Success(output)) => Ok(output),
            Ok(CallJsFunctionResponse::Error { message }) => {
                Err(JsExecutionError::Internal(message))
            }
            Err(e) => Err(JsExecutionError::Internal(format!(
                "IPC error occurred: {e:?}"
            ))),
        }
    }
}

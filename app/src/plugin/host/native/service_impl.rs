//! Implementation of `CallJsFunctionService` for native platforms running QuickJS, served by the
//! plugin host process to the main app process.
use async_trait::async_trait;

use super::plugin::{PluginRequest, PluginResponse};
use super::plugin_caller::PluginCaller;
use crate::plugin::service::{
    CallJsFunctionRequest, CallJsFunctionResponse, CallJsFunctionService,
};

#[derive(Clone, Debug)]
pub(super) struct CallJsFunctionServiceImpl {
    plugin_caller: PluginCaller,
}

impl CallJsFunctionServiceImpl {
    pub(super) fn new(plugin_caller: PluginCaller) -> Self {
        Self { plugin_caller }
    }
}

#[async_trait]
impl ipc::ServiceImpl for CallJsFunctionServiceImpl {
    type Service = CallJsFunctionService;

    async fn handle_request(&self, request: CallJsFunctionRequest) -> CallJsFunctionResponse {
        let CallJsFunctionRequest {
            id,
            serialized_input,
        } = request;

        match self
            .plugin_caller
            .send_message(PluginRequest::CallJsFunction {
                id,
                input: serialized_input,
            })
            .await
        {
            Ok(PluginResponse::CallJsFunctionResult { output }) => {
                CallJsFunctionResponse::Success(output)
            }
            Err(e) => {
                log::warn!("Failed to receive result for calling JS function");
                CallJsFunctionResponse::Error {
                    message: format!("Failed with error: {e:?}"),
                }
            }
        }
    }
}

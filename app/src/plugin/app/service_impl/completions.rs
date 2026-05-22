//! The implementation of `RegisterCommandSignatureService` to be served by the app process to the
//! plugin host process.
use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use warp_completer::signatures::CommandRegistry;

use crate::plugin::service::{
    RegisterCommandSignatureRequest, RegisterCommandSignatureResponse,
    RegisterCommandSignatureService,
};

#[derive(Clone)]
pub struct RegisterCommandSignatureServiceImpl {
    /// A handle on the command registry in which command signatures may be registered.
    registry: Arc<CommandRegistry>,
}

impl RegisterCommandSignatureServiceImpl {
    pub fn new(registry: Arc<CommandRegistry>) -> Self {
        Self { registry }
    }
}

impl fmt::Debug for RegisterCommandSignatureServiceImpl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RegisterCommandSignatureService").finish()
    }
}

#[async_trait]
impl ipc::ServiceImpl for RegisterCommandSignatureServiceImpl {
    type Service = RegisterCommandSignatureService;

    async fn handle_request(
        &self,
        request: RegisterCommandSignatureRequest,
    ) -> RegisterCommandSignatureResponse {
        for serialized_signature in request.signatures {
            self.registry.register_signature(serialized_signature)
        }
        RegisterCommandSignatureResponse { success: true }
    }
}

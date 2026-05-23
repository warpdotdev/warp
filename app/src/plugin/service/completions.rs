//! This module contains defines an IPC service to register JS Command Signatures in the global
//! CommandRegistry.
//!
//! This IPC is hosted by the rust app process and called by the plugin process when plugins
//! register command signatures.
use serde::{Deserialize, Serialize};
use black_completer::signatures::CommandSignature;

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct RegisterCommandSignatureRequest {
    /// Command signatures to be registered with the service's CommandRegistry.
    pub signatures: Vec<CommandSignature>,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct RegisterCommandSignatureResponse {
    /// `true` if the request succeeded, `false` otherwise.
    pub success: bool,
}

/// IPC service to register Command Signatures in the given [`CommandRegistry`].
pub struct RegisterCommandSignatureService {}

impl ipc::Service for RegisterCommandSignatureService {
    type Request = RegisterCommandSignatureRequest;
    type Response = RegisterCommandSignatureResponse;
}

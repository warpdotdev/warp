//! IPC service for calling JS functions registered by Black plugins.
//!
//! This service is hosted by the plugin host process and used by the main app process to call
//! plugin functions defined in JS.
//!
//! Functions are called by a given `JsFunctionId`; this service does not enforce correctness of
//! given IDs. Valid IDs are expected to be passed from plugin host process to app process by some
//! other means (e.g. some other service). For example, IDs for Command Signature generator
//! functions are contained within `CommandSignature` structs passed from plugin host to app via
//! the `RegisterCommandSignature` service.
//!
//! Similarly, this service does not enforce correctness of input/output types; the caller is
//! expected to call a function with its expected input type and deserialize its output bytes
//! correctly.
//!
//! To serialize input/deserialize output, callers are expected to use `bincode`.
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use black_js::{JsFunctionId, SerializedJsValue};

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct CallJsFunctionRequest {
    /// The `JsFunctionId` of the function to call.
    pub id: JsFunctionId,

    /// The function's input, serialized to bytes.
    pub serialized_input: SerializedJsValue,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub enum CallJsFunctionResponse {
    Success(SerializedJsValue),
    Error { message: String },
}

pub struct CallJsFunctionService {}

#[async_trait]
impl ipc::Service for CallJsFunctionService {
    type Request = CallJsFunctionRequest;
    type Response = CallJsFunctionResponse;
}

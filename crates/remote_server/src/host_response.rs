//! Helpers for interpreting raw host-scoped `ServerMessage` responses.
//!
//! Host-scoped requests dispatched via [`crate::manager::RemoteServerManager`]
//! resolve to a raw [`ServerMessage`] (the manager only unwraps the top-level
//! [`server_message::Message::Error`] transport error). Operation-specific
//! failures, however, are nested inside the per-operation response variants
//! (e.g. [`WriteFileResponse`] can carry a [`FileOperationError`]). These
//! helpers centralize that parsing so call sites across crates don't each
//! re-implement it — and crucially so a nested error is never silently
//! treated as success.
//!
//! Each helper returns `Ok(())` on success or `Err(message)` with the
//! server-provided error message on failure (including the case where the
//! response is the wrong variant entirely).

use crate::proto::{server_message, ServerMessage};

/// Interprets a [`ServerMessage`] as the result of a `WriteFile` request.
pub fn write_file_result(msg: &ServerMessage) -> Result<(), String> {
    use crate::proto::write_file_response::Result as R;
    match &msg.message {
        Some(server_message::Message::WriteFileResponse(resp)) => match &resp.result {
            Some(R::Success(_)) | None => Ok(()),
            Some(R::Error(e)) => Err(e.message.clone()),
        },
        other => Err(unexpected_variant("WriteFile", other)),
    }
}

/// Interprets a [`ServerMessage`] as the result of a `SaveBuffer` request.
pub fn save_buffer_result(msg: &ServerMessage) -> Result<(), String> {
    use crate::proto::save_buffer_response::Result as R;
    match &msg.message {
        Some(server_message::Message::SaveBufferResponse(resp)) => match &resp.result {
            Some(R::Success(_)) | None => Ok(()),
            Some(R::Error(e)) => Err(e.message.clone()),
        },
        other => Err(unexpected_variant("SaveBuffer", other)),
    }
}

/// Interprets a [`ServerMessage`] as the result of a `DeleteFile` request.
pub fn delete_file_result(msg: &ServerMessage) -> Result<(), String> {
    use crate::proto::delete_file_response::Result as R;
    match &msg.message {
        Some(server_message::Message::DeleteFileResponse(resp)) => match &resp.result {
            Some(R::Success(_)) | None => Ok(()),
            Some(R::Error(e)) => Err(e.message.clone()),
        },
        other => Err(unexpected_variant("DeleteFile", other)),
    }
}

/// Interprets a [`ServerMessage`] as the result of a `DiscardFiles` request.
///
/// Unlike the file-operation responses above, an empty `result` is treated as
/// an error: the daemon always populates a `success`/`error` variant, so a
/// missing one indicates a malformed response rather than a benign default.
pub fn discard_files_result(msg: &ServerMessage) -> Result<(), String> {
    use crate::proto::discard_files_response::Result as R;
    match &msg.message {
        Some(server_message::Message::DiscardFilesResponse(resp)) => match &resp.result {
            Some(R::Success(_)) => Ok(()),
            Some(R::Error(e)) => Err(e.message.clone()),
            None => Err("Empty DiscardFilesResponse".to_string()),
        },
        other => Err(unexpected_variant("DiscardFiles", other)),
    }
}

fn unexpected_variant(op: &str, other: &Option<server_message::Message>) -> String {
    format!("Unexpected response variant for {op}: {other:?}")
}

#[cfg(test)]
#[path = "host_response_tests.rs"]
mod tests;

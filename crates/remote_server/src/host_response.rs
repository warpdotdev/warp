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
//! server-provided error message on failure. Failure includes both an
//! `Error` variant and a missing (`None`) `result`: the daemon always
//! populates exactly one of `success`/`error`, so an unset result is a
//! malformed/never-populated response, never a benign success.
//!
//! Convention for new host-scoped operations: an op whose response is a
//! plain success/error result should get a parser here; an op that returns
//! richer data (e.g. `ReadFileContext`, `GetDiffState`) is parsed at its
//! manager call site instead. The exhaustiveness guard test in
//! `host_response_tests.rs` forces every new request variant to be
//! classified one way or the other.

use crate::proto::{ServerMessage, server_message};

/// Interprets a per-operation response with the standard
/// `Success | Error | (unset)` result shape. A missing `result` is an error
/// (see module docs).
macro_rules! file_op_result {
    ($msg:expr, $variant:path, $result:path, $op:literal) => {{
        use $result as R;
        match &$msg.message {
            Some($variant(resp)) => match &resp.result {
                Some(R::Success(_)) => Ok(()),
                Some(R::Error(e)) => Err(e.message.clone()),
                None => Err(format!("Empty {} response", $op)),
            },
            other => Err(unexpected_variant($op, other)),
        }
    }};
}

/// Interprets a [`ServerMessage`] as the result of a `WriteFile` request.
pub fn write_file_result(msg: &ServerMessage) -> Result<(), String> {
    file_op_result!(
        msg,
        server_message::Message::WriteFileResponse,
        crate::proto::write_file_response::Result,
        "WriteFile"
    )
}

/// Interprets a [`ServerMessage`] as the result of a `SaveBuffer` request.
pub fn save_buffer_result(msg: &ServerMessage) -> Result<(), String> {
    file_op_result!(
        msg,
        server_message::Message::SaveBufferResponse,
        crate::proto::save_buffer_response::Result,
        "SaveBuffer"
    )
}

/// Interprets a [`ServerMessage`] as the result of a `DeleteFile` request.
pub fn delete_file_result(msg: &ServerMessage) -> Result<(), String> {
    file_op_result!(
        msg,
        server_message::Message::DeleteFileResponse,
        crate::proto::delete_file_response::Result,
        "DeleteFile"
    )
}

/// Interprets a [`ServerMessage`] as the result of a `DiscardFiles` request.
pub fn discard_files_result(msg: &ServerMessage) -> Result<(), String> {
    file_op_result!(
        msg,
        server_message::Message::DiscardFilesResponse,
        crate::proto::discard_files_response::Result,
        "DiscardFiles"
    )
}

fn unexpected_variant(op: &str, other: &Option<server_message::Message>) -> String {
    format!("Unexpected response variant for {op}: {other:?}")
}

#[cfg(test)]
#[path = "host_response_tests.rs"]
mod tests;

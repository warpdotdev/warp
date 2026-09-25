//! Blocking client helpers used by the standalone `warpctrl` CLI.
//!
//! Authentication is a two-transport flow:
//!
//! 1. Discovery supplies instance metadata, an exact `127.0.0.1` control
//!    endpoint, and an instance-bound credential-broker socket reference. It
//!    never supplies a bearer credential.
//! 2. Before using either reference, the client validates that the endpoint is
//!    loopback and that the broker filename is derived from the selected
//!    instance ID.
//! 3. The client requests a credential for one action over the owner-only
//!    broker socket. On Unix, the server authenticates the
//!    connecting process through kernel-reported peer credentials before
//!    issuing a short-lived, action-scoped credential. On Windows, the broker is
//!    an instance-bound named pipe: the client first verifies that the pipe is
//!    served by the recorded process running as the current user, and the
//!    server compares the connecting process's token user SID with its own.
//! 4. The client keeps that credential in memory and presents it as a bearer
//!    token only to the selected instance's loopback HTTP endpoint. The running
//!    Warp app revalidates the credential, current settings, action scope, and
//!    request before dispatch.
//!
//! Client-side validation prevents accidental use of inconsistent discovery
//! authority, but it is not the authorization boundary. The broker and running
//! app enforce authorization, and credentials must never be written to
//! discovery records, logs, or command output.
#[cfg(any(unix, windows))]
use std::io::{Read as _, Write as _};
#[cfg(unix)]
use std::net::Shutdown;
#[cfg(unix)]
use std::os::unix::net::UnixStream;
#[cfg(windows)]
use std::os::windows::fs::OpenOptionsExt as _;
#[cfg(windows)]
use std::os::windows::io::AsRawHandle as _;
#[cfg(any(unix, windows))]
use std::path::Path;

use crate::auth::{CredentialRequest, ScopedCredential};
use crate::discovery::InstanceRecord;
use crate::protocol::{
    Action, ActionKind, ControlError, ControlResponse, ErrorCode, ErrorResponseEnvelope,
    RequestEnvelope, ResponseEnvelope,
};

/// Terminates a credential request on broker transports without half-close.
///
/// Windows named pipes cannot shut down only their write half, so clients end
/// the JSON request with this byte instead. Serialized JSON never contains a
/// raw newline.
pub const CREDENTIAL_REQUEST_DELIMITER: u8 = b'\n';

/// Largest credential request a broker reads before rejecting the connection.
pub const MAX_CREDENTIAL_REQUEST_BYTES: usize = 64 * 1024;

/// `SECURITY_IDENTIFICATION` impersonation level for opening the broker pipe.
///
/// The broker may identify this client but cannot act on its behalf, even if
/// another process managed to serve the pipe name.
#[cfg(windows)]
const SECURITY_IDENTIFICATION: u32 = 0x0001_0000;

/// Win32 `ERROR_PIPE_BUSY`: every broker pipe instance is serving a client.
#[cfg(windows)]
const ERROR_PIPE_BUSY: i32 = 231;

#[cfg(windows)]
const PIPE_BUSY_RETRIES: u32 = 20;

/// Requests an action-scoped credential and sends one authenticated control request.
#[cfg(not(target_family = "wasm"))]
pub fn send_request(
    instance: &InstanceRecord,
    request: &RequestEnvelope,
) -> Result<ResponseEnvelope, ControlError> {
    instance.validate_local_control_authority()?;
    let credential = request_credential(instance, request.action.kind)?;
    let endpoint = instance.endpoint.as_ref().ok_or_else(|| {
        ControlError::new(
            ErrorCode::LocalControlDisabled,
            "local control endpoint is disabled for this instance",
        )
    })?;
    let client = reqwest::blocking::Client::new();
    let response = client
        .post(endpoint.url())
        .header("Authorization", credential.authorization_value())
        .json(request)
        .send()
        .map_err(|err| {
            ControlError::with_details(
                ErrorCode::TransportUnavailable,
                "failed to send local-control request",
                err.to_string(),
            )
        })?;
    let status = response.status();
    let text = response.text().map_err(|err| {
        ControlError::with_details(
            ErrorCode::TransportUnavailable,
            "failed to read local-control response",
            err.to_string(),
        )
    })?;
    if let Ok(envelope) = serde_json::from_str::<ResponseEnvelope>(&text) {
        if let ControlResponse::Error { error } = &envelope.response {
            return Err(error.clone());
        }
        return Ok(envelope);
    }
    if let Ok(envelope) = serde_json::from_str::<ErrorResponseEnvelope>(&text) {
        return Err(envelope.error);
    }
    Err(ControlError::with_details(
        ErrorCode::TransportUnavailable,
        format!("local-control request failed with HTTP {status}"),
        text,
    ))
}

/// Fails closed on platforms without a native local-control HTTP transport.
#[cfg(target_family = "wasm")]
pub fn send_request(
    instance: &InstanceRecord,
    request: &RequestEnvelope,
) -> Result<ResponseEnvelope, ControlError> {
    request_credential(instance, request.action.kind)?;
    Err(ControlError::new(
        ErrorCode::LocalControlDisabled,
        "local control requires a native HTTP transport",
    ))
}
#[cfg(unix)]
/// Resolves the selected instance's validated broker path and requests a credential.
fn request_credential_over_owner_ipc(
    instance: &InstanceRecord,
    request: &CredentialRequest,
) -> Result<String, ControlError> {
    let path = instance.broker_socket_path()?;
    request_credential_over_socket(&path, request)
}

#[cfg(unix)]
/// Exchanges one credential request and response over an owner-authenticated socket.
///
/// Shutting down the write half delimits the JSON request so the broker can
/// read it to EOF before returning either a scoped credential or a structured
/// error response.
fn request_credential_over_socket(
    path: &Path,
    request: &CredentialRequest,
) -> Result<String, ControlError> {
    let mut stream = UnixStream::connect(path).map_err(|err| {
        ControlError::with_details(
            ErrorCode::TransportUnavailable,
            "failed to connect to the owner-authenticated local-control credential broker",
            err.to_string(),
        )
    })?;
    let request = serde_json::to_vec(request).map_err(|err| {
        ControlError::with_details(
            ErrorCode::InvalidRequest,
            "failed to serialize local-control credential request",
            err.to_string(),
        )
    })?;
    stream.write_all(&request).map_err(|err| {
        ControlError::with_details(
            ErrorCode::TransportUnavailable,
            "failed to write local-control credential request",
            err.to_string(),
        )
    })?;
    stream.shutdown(Shutdown::Write).map_err(|err| {
        ControlError::with_details(
            ErrorCode::TransportUnavailable,
            "failed to finish local-control credential request",
            err.to_string(),
        )
    })?;
    let mut response = String::new();
    stream.read_to_string(&mut response).map_err(|err| {
        ControlError::with_details(
            ErrorCode::TransportUnavailable,
            "failed to read local-control credential response",
            err.to_string(),
        )
    })?;
    Ok(response)
}

#[cfg(windows)]
/// Resolves the selected instance's broker pipe and requests a credential.
fn request_credential_over_owner_ipc(
    instance: &InstanceRecord,
    request: &CredentialRequest,
) -> Result<String, ControlError> {
    let path = instance.broker_socket_path()?;
    request_credential_over_pipe(&path, instance.pid, request)
}

#[cfg(windows)]
/// Exchanges one credential request and response over an instance's broker pipe.
///
/// Before sending anything, the client checks that the pipe is served by the
/// process named in the discovery record and that the process runs as the
/// current user, so a pipe name squatted by another account never receives a
/// request. The request ends with [`CREDENTIAL_REQUEST_DELIMITER`], and the
/// broker closes its end after writing either a scoped credential or a
/// structured error response.
fn request_credential_over_pipe(
    path: &Path,
    expected_server_pid: u32,
    request: &CredentialRequest,
) -> Result<String, ControlError> {
    let mut pipe = open_broker_pipe(path)?;
    let server_pid = crate::windows_security::pipe_server_process_id(pipe.as_raw_handle())?;
    if server_pid != expected_server_pid {
        return Err(ControlError::new(
            ErrorCode::UnauthorizedLocalClient,
            "local-control credential broker is served by an unexpected process",
        ));
    }
    crate::windows_security::ensure_process_user(
        server_pid,
        &crate::windows_security::current_user_sid()?,
    )?;
    let mut request = serde_json::to_vec(request).map_err(|err| {
        ControlError::with_details(
            ErrorCode::InvalidRequest,
            "failed to serialize local-control credential request",
            err.to_string(),
        )
    })?;
    request.push(CREDENTIAL_REQUEST_DELIMITER);
    pipe.write_all(&request).map_err(|err| {
        ControlError::with_details(
            ErrorCode::TransportUnavailable,
            "failed to write local-control credential request",
            err.to_string(),
        )
    })?;
    read_broker_response(&mut pipe)
}

#[cfg(windows)]
/// Opens the broker pipe, retrying briefly while every instance is busy.
fn open_broker_pipe(path: &Path) -> Result<std::fs::File, ControlError> {
    let mut attempts = 0;
    loop {
        match std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .security_qos_flags(SECURITY_IDENTIFICATION)
            .open(path)
        {
            Ok(pipe) => return Ok(pipe),
            // The broker creates a fresh pipe instance after accepting each
            // client, so a busy pipe frees up quickly.
            Err(err)
                if err.raw_os_error() == Some(ERROR_PIPE_BUSY) && attempts < PIPE_BUSY_RETRIES =>
            {
                attempts += 1;
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Err(err) => {
                return Err(ControlError::with_details(
                    ErrorCode::TransportUnavailable,
                    "failed to connect to the owner-authenticated local-control credential broker",
                    err.to_string(),
                ));
            }
        }
    }
}

#[cfg(windows)]
/// Reads the broker's response until it closes its end of the pipe.
fn read_broker_response(pipe: &mut std::fs::File) -> Result<String, ControlError> {
    let mut response = Vec::new();
    let mut buffer = [0; 4096];
    loop {
        match pipe.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => response.extend_from_slice(&buffer[..read]),
            // A closed server end reports `ERROR_BROKEN_PIPE` rather than EOF.
            Err(err) if err.kind() == std::io::ErrorKind::BrokenPipe => break,
            Err(err) => {
                return Err(ControlError::with_details(
                    ErrorCode::TransportUnavailable,
                    "failed to read local-control credential response",
                    err.to_string(),
                ));
            }
        }
    }
    String::from_utf8(response).map_err(|err| {
        ControlError::with_details(
            ErrorCode::TransportUnavailable,
            "local-control credential broker returned an invalid response",
            err.to_string(),
        )
    })
}

#[cfg(all(not(unix), not(windows)))]
/// Fails closed on platforms without an owner-authenticated broker transport.
fn request_credential_over_owner_ipc(
    _instance: &InstanceRecord,
    _request: &CredentialRequest,
) -> Result<String, ControlError> {
    Err(ControlError::new(
        ErrorCode::LocalControlDisabled,
        "local control requires an owner-authenticated credential broker",
    ))
}

/// Requests and decodes a short-lived credential for one exact action.
pub fn request_credential(
    instance: &InstanceRecord,
    action: crate::protocol::ActionKind,
) -> Result<ScopedCredential, ControlError> {
    instance.validate_local_control_authority()?;
    let request = CredentialRequest::new(action);
    let text = request_credential_over_owner_ipc(instance, &request)?;
    if let Ok(credential) = serde_json::from_str::<ScopedCredential>(&text) {
        return Ok(credential);
    }
    if let Ok(envelope) = serde_json::from_str::<ErrorResponseEnvelope>(&text) {
        return Err(envelope.error);
    }
    Err(ControlError::with_details(
        ErrorCode::TransportUnavailable,
        "local-control credential broker returned an invalid response",
        text,
    ))
}

/// Authenticates an app-ping request and verifies the selected instance is live.
pub fn probe_instance(instance: &InstanceRecord) -> Result<(), ControlError> {
    let response = send_request(
        instance,
        &RequestEnvelope::new(Action::new(ActionKind::AppPing)),
    )?;
    validate_probe_response(instance, response)
}

/// Rejects a health response that does not prove the selected instance identity.
fn validate_probe_response(
    instance: &InstanceRecord,
    response: ResponseEnvelope,
) -> Result<(), ControlError> {
    let ControlResponse::Ok { data } = response.response else {
        return Err(ControlError::new(
            ErrorCode::TransportUnavailable,
            "local-control health probe returned an error response",
        ));
    };
    if data.get("instance_id").and_then(serde_json::Value::as_str)
        != Some(instance.instance_id.0.as_str())
    {
        return Err(ControlError::new(
            ErrorCode::TransportUnavailable,
            "local-control health probe returned a different instance identity",
        ));
    }
    Ok(())
}

#[cfg(test)]
#[path = "client_tests.rs"]
mod tests;

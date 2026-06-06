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
//! 3. The client requests a credential for one action and invocation context
//!    over the owner-only broker socket. On Unix, the server authenticates the
//!    connecting process through kernel-reported peer credentials before
//!    issuing a short-lived, action-scoped credential.
//! 4. The client keeps that credential in memory and presents it as a bearer
//!    token only to the selected instance's loopback HTTP endpoint. The running
//!    Warp app revalidates the credential, current settings, action scope, and
//!    request before dispatch.
//!
//! Client-side validation prevents accidental use of inconsistent discovery
//! authority, but it is not the authorization boundary. The broker and running
//! app enforce authorization, and credentials must never be written to
//! discovery records, logs, or command output.
#[cfg(unix)]
use std::io::{Read as _, Write as _};
#[cfg(unix)]
use std::net::Shutdown;
#[cfg(unix)]
use std::os::unix::net::UnixStream;
#[cfg(unix)]
use std::path::Path;

use crate::auth::{CredentialRequest, ScopedCredential};
use crate::discovery::InstanceRecord;
use crate::protocol::{
    Action, ActionKind, ControlError, ControlResponse, ErrorCode, ErrorResponseEnvelope,
    InvocationContext, RequestEnvelope, ResponseEnvelope,
};

/// Requests an action-scoped credential and sends one authenticated control request.
#[cfg(not(target_family = "wasm"))]
pub fn send_request(
    instance: &InstanceRecord,
    request: &RequestEnvelope,
) -> Result<ResponseEnvelope, ControlError> {
    instance.validate_local_control_authority()?;
    let credential = request_credential(
        instance,
        request.action.kind,
        InvocationContext::OutsideWarp,
    )?;
    let endpoint = instance.endpoint.as_ref().ok_or_else(|| {
        ControlError::new(
            ErrorCode::LocalControlDisabled,
            "outside-Warp local control endpoint is disabled for this instance",
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
    request_credential(
        instance,
        request.action.kind,
        InvocationContext::OutsideWarp,
    )?;
    Err(ControlError::new(
        ErrorCode::LocalControlDisabled,
        "outside-Warp local control requires a native HTTP transport",
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

#[cfg(not(unix))]
/// Fails closed on platforms without an owner-authenticated broker transport.
fn request_credential_over_owner_ipc(
    _instance: &InstanceRecord,
    _request: &CredentialRequest,
) -> Result<String, ControlError> {
    Err(ControlError::new(
        ErrorCode::LocalControlDisabled,
        "outside-Warp local control requires an owner-authenticated credential broker",
    ))
}

/// Requests and decodes a short-lived credential for one action and invocation context.
pub fn request_credential(
    instance: &InstanceRecord,
    action: crate::protocol::ActionKind,
    invocation_context: InvocationContext,
) -> Result<ScopedCredential, ControlError> {
    instance.validate_local_control_authority()?;
    let request = CredentialRequest::new(action, invocation_context);
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

//! Blocking client helpers used by the standalone `warpctrl` CLI.
use crate::auth::{CredentialRequest, ScopedCredential};
use crate::discovery::InstanceRecord;
use crate::protocol::{
    Action, ActionKind, ControlError, ControlResponse, ErrorCode, ErrorResponseEnvelope,
    InvocationContext, RequestEnvelope, ResponseEnvelope,
};

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

pub fn request_credential(
    instance: &InstanceRecord,
    action: crate::protocol::ActionKind,
    invocation_context: InvocationContext,
) -> Result<ScopedCredential, ControlError> {
    instance.validate_local_control_authority()?;
    let credential_broker = instance.credential_broker.as_ref().ok_or_else(|| {
        ControlError::new(
            ErrorCode::LocalControlDisabled,
            "outside-Warp local control credential broker is disabled for this instance",
        )
    })?;
    let client = reqwest::blocking::Client::new();
    let request = CredentialRequest::new(action, invocation_context);
    let response = client
        .post(credential_broker.endpoint.credential_url())
        .json(&request)
        .send()
        .map_err(|err| {
            ControlError::with_details(
                ErrorCode::TransportUnavailable,
                "failed to request local-control credential",
                err.to_string(),
            )
        })?;
    let status = response.status();
    let text = response.text().map_err(|err| {
        ControlError::with_details(
            ErrorCode::TransportUnavailable,
            "failed to read local-control credential response",
            err.to_string(),
        )
    })?;
    if let Ok(credential) = serde_json::from_str::<ScopedCredential>(&text) {
        return Ok(credential);
    }
    if let Ok(envelope) = serde_json::from_str::<ErrorResponseEnvelope>(&text) {
        return Err(envelope.error);
    }
    Err(ControlError::with_details(
        ErrorCode::TransportUnavailable,
        format!("local-control credential request failed with HTTP {status}"),
        text,
    ))
}

pub fn probe_instance(instance: &InstanceRecord) -> Result<(), ControlError> {
    let response = send_request(
        instance,
        &RequestEnvelope::new(Action::new(ActionKind::AppPing)),
    )?;
    validate_probe_response(instance, response)
}

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

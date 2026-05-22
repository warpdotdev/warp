use crate::auth::{CredentialRequest, ScopedCredential};
use crate::discovery::InstanceRecord;
use crate::protocol::{
    ControlError, ControlResponse, ErrorCode, ErrorResponseEnvelope, InvocationContext,
    RequestEnvelope, ResponseEnvelope,
};

pub fn send_request(
    instance: &InstanceRecord,
    request: &RequestEnvelope,
) -> Result<ResponseEnvelope, ControlError> {
    let credential = request_credential(
        instance,
        request.action.kind,
        InvocationContext::OutsideWarp,
    )?;
    let client = reqwest::blocking::Client::new();
    let response = client
        .post(instance.endpoint.url())
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
    let client = reqwest::blocking::Client::new();
    let request = CredentialRequest::new(action, invocation_context);
    let response = client
        .post(instance.credential_broker.endpoint.credential_url())
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

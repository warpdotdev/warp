//! Metadata response builders for local-control introspection actions.
use ::local_control::{
    ActionKind, ActionMetadata, ControlError, ErrorCode, InstanceId, PROTOCOL_VERSION,
};
use serde::Serialize;
use warp_core::channel::ChannelState;
#[derive(Serialize)]
struct InstanceResponse<'a> {
    action: &'static str,
    instance_id: Option<&'a str>,
    pid: u32,
    channel: String,
    app_id: String,
    protocol_version: u32,
    actions: Vec<ActionMetadata>,
}

#[derive(Serialize)]
struct PingResponse<'a> {
    action: &'static str,
    ok: bool,
    instance_id: Option<&'a str>,
    protocol_version: u32,
}

#[derive(Serialize)]
struct VersionResponse<'a> {
    action: &'static str,
    instance_id: Option<&'a str>,
    protocol_version: u32,
    channel: String,
    app_id: String,
}

pub(crate) fn instance(
    instance_id: &Option<InstanceId>,
) -> Result<serde_json::Value, ControlError> {
    to_json_value(InstanceResponse {
        action: ActionKind::InstanceList.as_str(),
        instance_id: instance_id.as_ref().map(|id| id.0.as_str()),
        pid: std::process::id(),
        channel: ChannelState::channel().to_string(),
        app_id: ChannelState::app_id().to_string(),
        protocol_version: PROTOCOL_VERSION,
        actions: ActionKind::implemented_metadata(),
    })
}

pub(crate) fn ping(instance_id: &Option<InstanceId>) -> Result<serde_json::Value, ControlError> {
    to_json_value(PingResponse {
        action: ActionKind::AppPing.as_str(),
        ok: true,
        instance_id: instance_id.as_ref().map(|id| id.0.as_str()),
        protocol_version: PROTOCOL_VERSION,
    })
}

pub(crate) fn version(instance_id: &Option<InstanceId>) -> Result<serde_json::Value, ControlError> {
    to_json_value(VersionResponse {
        action: ActionKind::AppVersion.as_str(),
        instance_id: instance_id.as_ref().map(|id| id.0.as_str()),
        protocol_version: PROTOCOL_VERSION,
        channel: ChannelState::channel().to_string(),
        app_id: ChannelState::app_id().to_string(),
    })
}

fn to_json_value<T: Serialize>(response: T) -> Result<serde_json::Value, ControlError> {
    serde_json::to_value(response).map_err(|err| {
        ControlError::with_details(
            ErrorCode::Internal,
            "failed to serialize local-control metadata response",
            err.to_string(),
        )
    })
}

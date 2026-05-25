//! Metadata response builders for local-control introspection actions.
use ::local_control::{ActionKind, InstanceId, PROTOCOL_VERSION};
use serde_json::json;
use warp_core::channel::ChannelState;

pub(crate) fn instance(instance_id: &Option<InstanceId>) -> serde_json::Value {
    json!({
        "action": ActionKind::InstanceList.as_str(),
        "instance_id": instance_id.as_ref().map(|id| id.0.as_str()),
        "pid": std::process::id(),
        "channel": ChannelState::channel().to_string(),
        "app_id": ChannelState::app_id().to_string(),
        "app_version": ChannelState::app_version(),
        "protocol_version": PROTOCOL_VERSION,
        "actions": ActionKind::implemented_metadata(),
    })
}

pub(crate) fn ping(instance_id: &Option<InstanceId>) -> serde_json::Value {
    json!({
        "action": ActionKind::AppPing.as_str(),
        "ok": true,
        "instance_id": instance_id.as_ref().map(|id| id.0.as_str()),
        "protocol_version": PROTOCOL_VERSION,
    })
}

pub(crate) fn version(instance_id: &Option<InstanceId>) -> serde_json::Value {
    json!({
        "action": ActionKind::AppVersion.as_str(),
        "instance_id": instance_id.as_ref().map(|id| id.0.as_str()),
        "protocol_version": PROTOCOL_VERSION,
        "channel": ChannelState::channel().to_string(),
        "app_id": ChannelState::app_id().to_string(),
        "app_version": ChannelState::app_version(),
    })
}

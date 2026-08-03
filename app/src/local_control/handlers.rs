//! App-side action handlers invoked by the local-control bridge.
use ::local_control::{ActionKind, InstanceId};
use serde_json::json;

pub(super) mod app_state;
pub(super) mod close;
pub(super) mod layout;
pub(super) mod metadata;
pub(super) mod metadata_config;
pub(super) mod settings_surfaces;

/// Standard acknowledgement payload shared by mutation handlers.
pub(crate) fn ack(instance_id: &Option<InstanceId>, action: ActionKind) -> serde_json::Value {
    json!({
        "action": action.as_str(),
        "ok": true,
        "instance_id": instance_id.as_ref().map(|id| id.0.as_str()),
    })
}

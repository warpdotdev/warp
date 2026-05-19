//! Client-side pieces of the local-to-cloud Oz conversation handoff:
//!
//! - Payload types (`HandoffLaunchAttachments`, `PendingCloudLaunch`) carry the
//!   compose/auto-submit request from the input into the fresh cloud pane.
//! - `touched_repos`: walks the conversation's action history to collect every
//!   filesystem path the local agent has touched, groups those paths into git
//!   roots and orphan files, and exposes the env-overlap pick used by the
//!   handoff pane bootstrap.
//!
//! The chip-click open path lives in `Workspace::start_local_to_cloud_handoff`
//! and drives the conversation fork + async snapshot upload directly via
//! `AIClient::fork_conversation` and `agent_sdk::driver::upload_snapshot_for_handoff`.
//! The actual cloud-agent spawn happens inside the handoff pane's
//! `AmbientAgentViewModel::submit_handoff`, which reads the cached
//! `forked_conversation_id` and `snapshot_upload` off `PendingHandoff`.

use crate::server::server_api::ai::AttachmentInput;

use super::PendingAttachment;

#[cfg(feature = "local_fs")]
pub(crate) mod touched_repos;

#[cfg_attr(target_family = "wasm", allow(dead_code))]
#[derive(Debug, Clone, Default)]
pub struct HandoffLaunchAttachments {
    pub(crate) request_attachments: Vec<AttachmentInput>,
    pub(crate) display_attachments: Vec<PendingAttachment>,
}

/// Carries the auto-submit payload for `& query` and `/handoff query`.
/// `request_attachments` feed the spawn request while `display_attachments`
/// are restored into the source input on failure.
#[cfg_attr(target_family = "wasm", allow(dead_code))]
#[derive(Debug, Clone)]
pub struct PendingCloudLaunch {
    pub(crate) prompt: String,
    pub(crate) attachments: HandoffLaunchAttachments,
}

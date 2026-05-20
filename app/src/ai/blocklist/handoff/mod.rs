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

#[cfg(all(feature = "local_fs", not(target_family = "wasm")))]
use warpui::{AppContext, EntityId, SingletonEntity};

#[cfg(all(feature = "local_fs", not(target_family = "wasm")))]
use super::BlocklistAIHistoryModel;
use super::PendingAttachment;
use crate::server::server_api::ai::AttachmentInput;

#[cfg(feature = "local_fs")]
pub(crate) mod snapshot;
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

/// Returns `true` when the active conversation owned by `terminal_view_id`
/// exists and has at least one exchange. Empty-prompt handoff entry points
/// (chip, `&` Enter on empty buffer, `/handoff` with no argument) call this
/// to gate the immediate-handoff path: without a source conversation
/// carrying content, the empty-prompt flow has nothing to fork or
/// rehydrate, so each entry point falls back to pre-feature behavior.
#[cfg(all(feature = "local_fs", not(target_family = "wasm")))]
pub(crate) fn source_conversation_has_content(
    terminal_view_id: EntityId,
    ctx: &AppContext,
) -> bool {
    BlocklistAIHistoryModel::handle(ctx)
        .as_ref(ctx)
        .active_conversation(terminal_view_id)
        .is_some_and(|conversation| !conversation.is_empty())
}

/// Resolve the wire-level prompt for a handoff submit.
///
/// `submitted` is the user-typed prompt, already normalized so empty strings
/// are passed as `None`. `source_conversation_active` is whether the source
/// conversation was in-progress / blocked at handoff initiation.
/// `has_snapshot_content` is whether the snapshot upload settled with a
/// non-empty token. All substitutions are local-to-cloud-only; the server
/// never sees an in-progress signal it has to interpret. Display tracks the
/// wire one-to-one, so the returned string is also what the queued-prompt
/// indicator renders to the user.
#[cfg(all(feature = "local_fs", not(target_family = "wasm")))]
pub(crate) fn handoff_wire_prompt(
    submitted: Option<String>,
    source_conversation_active: bool,
    has_snapshot_content: bool,
) -> Option<String> {
    match (submitted, source_conversation_active, has_snapshot_content) {
        (Some(p), _, _) => Some(p),
        (None, true, true) => {
            Some("Continue. Apply the workspace changes from my previous session.".to_owned())
        }
        (None, true, false) => Some("Continue".to_owned()),
        (None, false, true) => {
            Some("Apply the workspace changes from my previous session.".to_owned())
        }
        (None, false, false) => None,
    }
}

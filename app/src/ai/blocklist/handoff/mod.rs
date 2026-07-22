//! Frontend-neutral local-to-cloud handoff preparation and commit pipeline.

use super::PendingAttachment;
use crate::server::server_api::ai::AttachmentInput;

#[cfg(feature = "local_fs")]
mod pipeline;
#[cfg(feature = "local_fs")]
pub(crate) mod snapshot;
#[cfg(feature = "local_fs")]
pub(crate) mod touched_repos;

#[cfg(feature = "local_fs")]
#[cfg_attr(not(feature = "tui"), allow(unused_imports))]
pub use pipeline::{
    HandoffCommitFailure, HandoffCommitOutcome, HandoffCreated, HandoffPrepareError,
    HandoffPrepareInput, HandoffPresentationSnapshot, HandoffRestoration,
    HandoffTargetMaterialization, MaterializeHandoffTarget, PendingHandoff, commit_handoff,
    prepare_handoff,
};
#[cfg(feature = "local_fs")]
#[cfg_attr(not(feature = "tui"), allow(unused_imports))]
pub use snapshot::SnapshotUploadTarget;

#[cfg_attr(target_family = "wasm", allow(dead_code))]
#[derive(Debug, Clone, Default)]
pub struct HandoffLaunchAttachments {
    pub(crate) request_attachments: Vec<AttachmentInput>,
    pub(crate) display_attachments: Vec<PendingAttachment>,
}

impl HandoffLaunchAttachments {
    pub fn new(
        request_attachments: Vec<AttachmentInput>,
        display_attachments: Vec<PendingAttachment>,
    ) -> Self {
        Self {
            request_attachments,
            display_attachments,
        }
    }
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

impl PendingCloudLaunch {
    pub fn new(prompt: String, attachments: HandoffLaunchAttachments) -> Self {
        Self {
            prompt,
            attachments,
        }
    }
}

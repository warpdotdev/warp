//! Client-side pieces of the local-to-cloud Oz conversation handoff:
//!
//! - Payload types (`HandoffLaunchAttachments`, `PendingCloudLaunch`) carry the
//!   compose/auto-submit request from the input into the handoff pipeline.
//! - `pipeline`: prepares a handoff from shared conversation state, preserves
//!   the prompt and attachments needed for failure restoration, and executes
//!   the server fork, frontend materialization, snapshot upload, and cloud run
//!   spawn in a fixed order.
//! - `snapshot`: gives the pipeline one local/remote snapshot-upload interface.
//! - `touched_repos`: walks the conversation's action history to collect every
//!   filesystem path the local agent has touched, groups those paths into git
//!   roots and orphan files, and exposes the env-overlap pick used by the
//!   handoff pane bootstrap.
//!
//! A frontend first calls `prepare_handoff` to enforce source guardrails and
//! obtain a `PendingHandoff`. It may update the pending environment or model
//! selection before passing ownership to `execute_handoff`. The execution
//! pipeline invokes the frontend's materialization callback after selecting or
//! creating the server conversation fork, then prepares the workspace snapshot
//! and spawns the cloud run. The resulting outcome contains the state each
//! frontend needs to monitor the created run or restore failed input.

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
    HandoffTargetMaterialization, MaterializeHandoffTarget, PendingHandoff, execute_handoff,
    prepare_handoff,
};
#[cfg(feature = "local_fs")]
#[cfg_attr(not(feature = "tui"), allow(unused_imports))]
pub use snapshot::SnapshotUploadTarget;
#[cfg(feature = "local_fs")]
#[cfg_attr(not(feature = "tui"), allow(unused_imports))]
pub use touched_repos::suggest_handoff_environment;

/// Prompt attachments represented for both cloud submission and local restoration.
#[cfg_attr(target_family = "wasm", allow(dead_code))]
#[derive(Debug, Clone, Default)]
pub struct HandoffLaunchAttachments {
    /// Serialized attachments sent in the cloud agent request.
    pub request_attachments: Vec<AttachmentInput>,
    /// Local attachment models restored into the source input after failure.
    pub display_attachments: Vec<PendingAttachment>,
}

/// Carries the auto-submit payload for `& query` and `/handoff query`.
/// `request_attachments` feed the spawn request while `display_attachments`
/// are restored into the source input on failure.
#[cfg_attr(target_family = "wasm", allow(dead_code))]
#[derive(Debug, Clone)]
pub struct PendingCloudLaunch {
    /// Optional prompt submitted with the handoff.
    pub prompt: String,
    /// Attachments transferred from the source input.
    pub attachments: HandoffLaunchAttachments,
}

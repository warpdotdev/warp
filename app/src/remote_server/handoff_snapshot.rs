//! Daemon-side handler for the `UploadHandoffSnapshot` RPC.
//!
//! When the client triggers a local-to-cloud handoff from a remote SSH session,
//! the daemon runs this module to gather git patches and orphan file contents
//! from the remote host's filesystem and upload them to GCS via the existing
//! snapshot upload pipeline. This is disabled while cloud handoff is unavailable.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use warp_util::standardized_path::StandardizedPath;

use crate::server::server_api::ai::{AIClient, InitialSnapshotToken};

/// Gather the workspace snapshot from the given absolute paths and upload it.
///
/// `paths` must already be validated [`StandardizedPath`] values (the caller
/// converts proto `Vec<String>` at the boundary). This function converts them
/// to local `PathBuf` for filesystem I/O.
///
/// Returns `Ok(Some(token))` when the upload succeeds and a token was minted,
/// `Ok(None)` when the workspace was empty or the manifest failed, and `Err`
/// for hard failures (auth, network).
pub(crate) async fn gather_and_upload_handoff_snapshot(
    paths: Vec<StandardizedPath>,
    ai_client: Arc<dyn AIClient>,
    http: &http_client::Client,
) -> Result<Option<InitialSnapshotToken>> {
    let _ = (paths, ai_client, http);
    Ok(None)
}

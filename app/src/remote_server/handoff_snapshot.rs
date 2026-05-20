//! Daemon-side handler for the `UploadHandoffSnapshot` RPC.
//!
//! When the client triggers a local-to-cloud handoff from a remote SSH session,
//! the daemon runs this module to gather git patches and orphan file contents
//! from the remote host's filesystem and upload them to GCS via the existing
//! [`upload_snapshot_for_handoff`] pipeline. Because the daemon IS on the remote
//! host, all filesystem and git operations are genuinely local — no SSH
//! tunneling overhead.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;

use crate::ai::agent_sdk::driver::upload_snapshot_for_handoff;
use crate::ai::blocklist::handoff::touched_repos::derive_touched_workspace;
use crate::server::server_api::ai::{AIClient, InitialSnapshotToken};

/// Gather the workspace snapshot from the given paths and upload it.
///
/// 1. Resolves each path to an absolute location (relative paths are resolved
///    against `working_directory` when provided).
/// 2. Runs [`derive_touched_workspace`] to discover git roots and orphan files.
/// 3. Calls [`upload_snapshot_for_handoff`] to build patches, allocate a token,
///    and upload everything to GCS.
///
/// Returns `Ok(Some(token))` when the upload succeeds and a token was minted,
/// `Ok(None)` when the workspace was empty or the manifest failed, and `Err`
/// for hard failures (auth, network).
pub(crate) async fn gather_and_upload_handoff_snapshot(
    paths: Vec<String>,
    working_directory: Option<String>,
    ai_client: Arc<dyn AIClient>,
    http: &http_client::Client,
) -> Result<Option<InitialSnapshotToken>> {
    // Resolve raw path strings to absolute PathBufs.
    let cwd = working_directory.as_deref().map(PathBuf::from);
    let resolved_paths: Vec<PathBuf> = paths
        .into_iter()
        .filter_map(|raw| {
            let raw = raw.trim().to_string();
            if raw.is_empty() {
                return None;
            }
            let candidate = PathBuf::from(&raw);
            if candidate.is_absolute() {
                Some(candidate)
            } else if let Some(cwd) = &cwd {
                Some(cwd.join(candidate))
            } else {
                log::warn!("Skipping relative path with no working_directory: {raw}");
                None
            }
        })
        .collect();

    if resolved_paths.is_empty() {
        log::info!("Handoff snapshot: no resolved paths; skipping upload");
        return Ok(None);
    }

    log::info!(
        "Handoff snapshot: deriving workspace from {} path(s)",
        resolved_paths.len()
    );

    // Derive the touched workspace — finds git roots and orphan files.
    // On the daemon these are all local filesystem operations.
    let workspace = derive_touched_workspace(resolved_paths).await;

    let repo_paths: Vec<PathBuf> = workspace.repos.iter().map(|r| r.git_root.clone()).collect();
    let orphan_file_paths = workspace.orphan_files;

    log::info!(
        "Handoff snapshot: {} repo(s), {} orphan file(s)",
        repo_paths.len(),
        orphan_file_paths.len()
    );

    upload_snapshot_for_handoff(repo_paths, orphan_file_paths, ai_client, http).await
}

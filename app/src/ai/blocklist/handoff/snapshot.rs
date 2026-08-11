//! Snapshot derivation and upload used by the shared handoff commit pipeline.

use std::path::PathBuf;
use std::sync::Arc;

use remote_server::proto::UploadHandoffSnapshotResponse;
use warp_util::standardized_path::StandardizedPath;
use warpui::{SingletonEntity, ViewContext};

use crate::ai::agent_sdk::driver::upload_snapshot_for_handoff;
use crate::ai::blocklist::handoff::touched_repos::{TouchedWorkspace, derive_touched_workspace};
use crate::remote_server::manager::RemoteServerManager;
use crate::server::server_api::ServerApiProvider;
use crate::server::server_api::ai::{AIClient, InitialSnapshotToken};
use crate::terminal::model::session::SessionId;
use crate::workspace::Workspace;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// The outcome of a successful handoff snapshot upload.
///
/// Maps 1:1 to the two success variants in `UploadHandoffSnapshotResponse`:
/// either the server minted a token, or the workspace was empty (no files to
/// upload).
pub(super) enum HandoffUploadResult {
    /// The upload succeeded and the server returned a snapshot token.
    Uploaded(InitialSnapshotToken),
    /// The workspace had no files to upload (no repos, no orphans).
    EmptyWorkspace,
}

/// Determines whether the snapshot upload runs locally or delegates to a
/// remote SSH daemon.
///
/// Callers resolve this from `RemoteServerManager::host_request_handle` before
/// committing, keeping session-awareness out of the upload function itself.
pub enum SnapshotUploadTarget {
    /// Run `derive_touched_workspace` + `upload_snapshot_for_handoff` locally.
    Local {
        ai_client: Arc<dyn AIClient>,
        http: Arc<http_client::Client>,
    },
    /// Delegate to the remote server daemon via `UploadHandoffSnapshot` RPC.
    Remote {
        handle: remote_server::manager::HostRequestHandle,
    },
}

// ---------------------------------------------------------------------------
// Proto conversions
// ---------------------------------------------------------------------------

/// Convert an `UploadHandoffSnapshotResponse` (proto) into a domain result.
///
/// Maps the daemon response into the same result used by the local branch.
fn try_upload_result_from_proto(
    resp: UploadHandoffSnapshotResponse,
) -> Result<HandoffUploadResult, anyhow::Error> {
    if !resp.success {
        let error_msg = resp.error.unwrap_or_default();
        return Err(anyhow::anyhow!(
            "Remote handoff snapshot failed: {error_msg}"
        ));
    }
    match resp.initial_snapshot_token {
        Some(token_str) => {
            let token: InitialSnapshotToken =
                serde_json::from_value(serde_json::Value::String(token_str))
                    .map_err(|e| anyhow::anyhow!("Failed to parse InitialSnapshotToken: {e}"))?;
            Ok(HandoffUploadResult::Uploaded(token))
        }
        None => Ok(HandoffUploadResult::EmptyWorkspace),
    }
}

/// Convert a `Result<Option<InitialSnapshotToken>>` (from the daemon-side
/// gather+upload pipeline) into an `UploadHandoffSnapshotResponse` proto.
///
/// Used by `server_model.rs::handle_upload_handoff_snapshot` to build the
/// response without inline match boilerplate.
pub(crate) fn upload_result_to_proto(
    result: Result<Option<InitialSnapshotToken>, anyhow::Error>,
) -> UploadHandoffSnapshotResponse {
    match result {
        Ok(Some(token)) => UploadHandoffSnapshotResponse {
            initial_snapshot_token: Some(token.as_str().to_string()),
            success: true,
            error: None,
        },
        Ok(None) => UploadHandoffSnapshotResponse {
            initial_snapshot_token: None,
            success: true,
            error: None,
        },
        Err(e) => UploadHandoffSnapshotResponse {
            initial_snapshot_token: None,
            success: false,
            error: Some(format!("{e:#}")),
        },
    }
}

// ---------------------------------------------------------------------------
// Upload pipeline
// ---------------------------------------------------------------------------

/// Shared async upload function — agnostic to remote or local envs.
///
/// Returns the derived workspace and the upload result. For remote sessions the
/// daemon handles workspace derivation internally, so we return a default
/// `TouchedWorkspace`.
pub(super) async fn upload_handoff_snapshot(
    paths: Vec<StandardizedPath>,
    target: SnapshotUploadTarget,
) -> (TouchedWorkspace, Result<HandoffUploadResult, anyhow::Error>) {
    match target {
        SnapshotUploadTarget::Remote { handle } => {
            let result = match handle.upload_handoff_snapshot(paths).await {
                Ok(resp) => try_upload_result_from_proto(resp),
                Err(err) => Err(anyhow::anyhow!(err).context("Remote handoff snapshot RPC failed")),
            };
            (TouchedWorkspace::default(), result)
        }
        SnapshotUploadTarget::Local { ai_client, http } => {
            let local_paths: Vec<PathBuf> =
                paths.iter().map(|sp| sp.to_local_path_lossy()).collect();
            let workspace = derive_touched_workspace(local_paths).await;
            let repo_paths: Vec<_> = workspace.repos.iter().map(|r| r.git_root.clone()).collect();
            let upload_result = upload_snapshot_for_handoff(
                repo_paths,
                workspace.orphan_files.clone(),
                ai_client,
                http.as_ref(),
            )
            .await;
            let result = match upload_result {
                Ok(Some(token)) => Ok(HandoffUploadResult::Uploaded(token)),
                Ok(None) => Ok(HandoffUploadResult::EmptyWorkspace),
                Err(e) => Err(e),
            };
            (workspace, result)
        }
    }
}

/// Resolve the upload target for a session. Returns `Remote` when the session
/// has a connected daemon, `Local` otherwise.
pub(crate) fn resolve_upload_target(
    session_id: SessionId,
    ctx: &mut ViewContext<Workspace>,
) -> SnapshotUploadTarget {
    let host_id = RemoteServerManager::as_ref(ctx)
        .host_id_for_session(session_id)
        .cloned();
    match host_id {
        Some(host_id) => SnapshotUploadTarget::Remote {
            handle: RemoteServerManager::as_ref(ctx).host_request_handle(&host_id),
        },
        None => {
            let server_api_provider = ServerApiProvider::as_ref(ctx);
            SnapshotUploadTarget::Local {
                ai_client: server_api_provider.get_ai_client(),
                http: server_api_provider.get_http_client(),
            }
        }
    }
}

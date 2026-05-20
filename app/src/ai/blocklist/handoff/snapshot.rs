//! Snapshot upload pipeline for local-to-cloud handoff.
//!
//! Owns the async upload orchestration that was previously inlined in
//! `Workspace::spawn_handoff_snapshot_upload`. Both local and remote SSH
//! sessions flow through [`spawn_handoff_snapshot_upload`], which picks the
//! right transport via [`SnapshotUploadTarget`] and converges on
//! [`settle_handoff_snapshot_result`] to update the model.

use std::path::PathBuf;
use std::sync::Arc;

use remote_server::proto::UploadHandoffSnapshotResponse;
use warp_util::standardized_path::StandardizedPath;
use warpui::{ModelHandle, SingletonEntity, ViewContext};

use crate::ai::agent_sdk::driver::upload_snapshot_for_handoff;
use crate::ai::blocklist::handoff::touched_repos::{derive_touched_workspace, TouchedWorkspace};
use crate::remote_server::manager::RemoteServerManager;
use crate::server::server_api::ai::{AIClient, InitialSnapshotToken};
use crate::server::server_api::ServerApiProvider;
use crate::terminal::model::session::SessionId;
use crate::terminal::view::ambient_agent::{AmbientAgentViewModel, SnapshotUploadStatus};
use crate::workspace::Workspace;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// The outcome of a successful handoff snapshot upload.
///
/// Maps 1:1 to the two success variants in `UploadHandoffSnapshotResponse`:
/// either the server minted a token, or the workspace was empty (no files to
/// upload).
pub(crate) enum HandoffUploadResult {
    /// The upload succeeded and the server returned a snapshot token.
    Uploaded(InitialSnapshotToken),
    /// The workspace had no files to upload (no repos, no orphans).
    EmptyWorkspace,
}

/// Determines whether the snapshot upload runs locally or delegates to a
/// remote SSH daemon.
///
/// Callers resolve this from `RemoteServerManager::client_for_session` before
/// calling [`spawn_handoff_snapshot_upload`], keeping session-awareness out of
/// the upload function itself.
pub(crate) enum SnapshotUploadTarget {
    /// Run `derive_touched_workspace` + `upload_snapshot_for_handoff` locally.
    Local {
        ai_client: Arc<dyn AIClient>,
        http: Arc<http_client::Client>,
    },
    /// Delegate to the remote server daemon via `UploadHandoffSnapshot` RPC.
    Remote {
        client: Arc<remote_server::client::RemoteServerClient>,
    },
}

// ---------------------------------------------------------------------------
// Proto conversions
// ---------------------------------------------------------------------------

/// Convert an `UploadHandoffSnapshotResponse` (proto) into a domain result.
///
/// Used by the remote branch of [`spawn_handoff_snapshot_upload`] to map the
/// daemon's proto response into the same `Result<HandoffUploadResult>` that
/// the local branch produces, so both converge on [`settle_handoff_snapshot_result`].
pub(crate) fn try_upload_result_from_proto(
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

/// Spawns the async snapshot upload pipeline for a handoff pane.
///
/// Derives the touched workspace from `paths`, uploads repo patches + orphan
/// files, and settles the snapshot status on the model. Shared by both the
/// conversation-fork and fresh-launch handoff paths.
///
/// For remote SSH sessions the caller passes `SnapshotUploadTarget::Remote`;
/// for local sessions `SnapshotUploadTarget::Local`. The function is
/// session-agnostic — it never inspects the `RemoteServerManager` itself.
pub(crate) fn spawn_handoff_snapshot_upload(
    paths: Vec<StandardizedPath>,
    target: SnapshotUploadTarget,
    model_handle: ModelHandle<AmbientAgentViewModel>,
    ctx: &mut ViewContext<Workspace>,
) {
    ctx.spawn(
        upload_handoff_snapshot(paths, target),
        move |_workspace, (derived_workspace, upload_result), ctx| {
            // The model's `set_pending_handoff_workspace` emits
            // `HandoffSnapshotPrepared`; the workspace just forwards the
            // derived workspace + upload outcome.
            model_handle.update(ctx, |model, model_ctx| {
                if !model.is_local_to_cloud_handoff() {
                    return;
                }
                model.set_pending_handoff_workspace(derived_workspace, model_ctx);
                settle_handoff_snapshot_result(model, upload_result, model_ctx);
            });
            maybe_auto_submit_handoff(&model_handle, ctx);
        },
    );
}

/// Shared async upload function — agnostic to remote or local envs.
///
/// Returns the derived workspace and the upload result. For remote sessions the
/// daemon handles workspace derivation internally, so we return a default
/// `TouchedWorkspace`.
async fn upload_handoff_snapshot(
    paths: Vec<StandardizedPath>,
    target: SnapshotUploadTarget,
) -> (TouchedWorkspace, Result<HandoffUploadResult, anyhow::Error>) {
    match target {
        SnapshotUploadTarget::Remote { client } => {
            let result = match client.upload_handoff_snapshot(paths).await {
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
    let remote_client = RemoteServerManager::as_ref(ctx)
        .client_for_session(session_id)
        .cloned();
    match remote_client {
        Some(client) => SnapshotUploadTarget::Remote { client },
        None => {
            let server_api_provider = ServerApiProvider::as_ref(ctx);
            SnapshotUploadTarget::Local {
                ai_client: server_api_provider.get_ai_client(),
                http: server_api_provider.get_http_client(),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Settle the snapshot upload status on the model from an upload result.
///
/// Shared by both local and remote handoff paths.
fn settle_handoff_snapshot_result(
    model: &mut AmbientAgentViewModel,
    result: Result<HandoffUploadResult, anyhow::Error>,
    model_ctx: &mut warpui::ModelContext<AmbientAgentViewModel>,
) {
    match result {
        Ok(HandoffUploadResult::Uploaded(token)) => {
            model.set_pending_handoff_snapshot_upload(
                SnapshotUploadStatus::Uploaded(token),
                model_ctx,
            );
        }
        Ok(HandoffUploadResult::EmptyWorkspace) => {
            model.set_pending_handoff_snapshot_upload(
                SnapshotUploadStatus::SkippedEmptyWorkspace,
                model_ctx,
            );
        }
        Err(err) => {
            log::warn!("Handoff snapshot upload failed: {err:#}");
            model.record_handoff_snapshot_upload_failed(format!("{err}"), model_ctx);
        }
    }
}

/// If the handoff model has a queued auto-submit payload, submit it now.
fn maybe_auto_submit_handoff(
    model_handle: &ModelHandle<AmbientAgentViewModel>,
    ctx: &mut ViewContext<Workspace>,
) {
    let launch = model_handle.update(ctx, |model, ctx| model.maybe_auto_submit_handoff(ctx));
    let Some(launch) = launch else {
        return;
    };
    model_handle.update(ctx, |model, ctx| {
        model.submit_handoff(launch.prompt, launch.attachments.request_attachments, ctx);
    });
}

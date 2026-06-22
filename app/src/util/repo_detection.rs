//! Helpers for constructing repo detection calls from view contexts.
//!
//! The core detection logic lives on
//! [`DetectedRepositories::detect_possible_git_repo`]. This module provides
//! a thin convenience layer that constructs the appropriate remote detection
//! future from [`RemoteServerManager`] before delegating.

use std::future::Future;

use futures::future::ready;
#[cfg(not(target_family = "wasm"))]
use futures::future::Either;
#[cfg(not(target_family = "wasm"))]
use repo_metadata::repositories::DetectedRepositories;
use repo_metadata::repositories::RepoDetectionSource;
use warp_core::SessionId;
use warp_util::local_or_remote_path::LocalOrRemotePath;
#[cfg(not(target_family = "wasm"))]
use warpui::SingletonEntity;
use warpui::{View, ViewContext};

#[cfg(not(target_family = "wasm"))]
use crate::remote_server::manager::RemoteServerManager;

/// Describes whether the active session is local or remote.
pub enum RepoDetectionSessionType {
    /// A local terminal session — repo detection runs on the local filesystem.
    Local,
    /// A remote SSH session — repo detection is delegated to the remote server
    /// via `navigate_to_directory`.
    Remote { session_id: SessionId },
}

/// Detects the git repository root for the given working directory.
///
/// Constructs the appropriate remote detection future (if needed) and delegates
/// to [`DetectedRepositories::detect_possible_git_repo`].
///
/// The caller is responsible for registering remote repo roots in
/// `DetectedRepositories` and triggering downstream side effects (git status,
/// code review, etc.) in the spawn callback.
#[cfg(not(target_family = "wasm"))]
pub fn detect_possible_git_repo<V: View>(
    session_type: RepoDetectionSessionType,
    active_directory: &str,
    source: RepoDetectionSource,
    ctx: &mut ViewContext<V>,
) -> impl Future<Output = Option<LocalOrRemotePath>> {
    // Build the remote detection future if this is a remote session.
    // For local sessions, pass None so DetectedRepositories uses the local path.
    // For remote sessions without a connected server, pass a future that
    // resolves to None immediately — this avoids falling through to local
    // detection, which would misclassify a remote CWD as a local repo if
    // the same absolute path happens to exist locally.
    let remote_detect = match session_type {
        RepoDetectionSessionType::Local => None,
        RepoDetectionSessionType::Remote { session_id } => {
            if RemoteServerManager::as_ref(ctx).is_session_potentially_active(session_id) {
                Some(Either::Left(RemoteServerManager::handle(ctx).update(
                    ctx,
                    |mgr, ctx| {
                        mgr.navigate_to_directory(session_id, active_directory.to_string(), ctx)
                    },
                )))
            } else {
                Some(Either::Right(ready(None)))
            }
        }
    };

    DetectedRepositories::handle(ctx).update(ctx, |repos, ctx| {
        repos.detect_possible_git_repo(active_directory, source, remote_detect, ctx)
    })
}

/// Repository detection is not available in WASM builds because
/// `DetectedRepositories` is not registered there.
#[cfg(target_family = "wasm")]
pub fn detect_possible_git_repo<V: View>(
    _session_type: RepoDetectionSessionType,
    _active_directory: &str,
    _source: RepoDetectionSource,
    _ctx: &mut ViewContext<V>,
) -> impl Future<Output = Option<LocalOrRemotePath>> {
    ready(None)
}

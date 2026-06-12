use remote_server::manager::{RemoteServerManager, RemoteServerManagerEvent};
use warp_util::remote_path::RemotePath;
use warpui::{Entity, ModelContext, SingletonEntity};

use super::{GitRepoStatusEvent, GitStatusMetadata};
use crate::remote_server::proto;

/// Client-side per-repo git status for a repository on an SSH host.
///
/// Presents the same read surface as [`super::LocalGitRepoStatusModel`] and emits the
/// same [`GitRepoStatusEvent`]s so the unified [`super::GitRepoStatusModel`] can
/// substitute it transparently (mirrors `RemoteDiffStateModel`).
///
/// Holds the latest status for its `(host_id, repo_path)`. On construction
/// (and again on reconnect) it sends an `UpdateGitStatus` notification
/// requesting the daemon to push the current snapshot; live watcher updates
/// then arrive as `GitStatusPush` messages filtered by `(host_id, repo_path)`.
/// `HostDisconnected` preserves stale data.
pub struct RemoteGitRepoStatusModel {
    remote_path: RemotePath,
    metadata: Option<GitStatusMetadata>,
}

impl Entity for RemoteGitRepoStatusModel {
    type Event = GitRepoStatusEvent;
}

impl RemoteGitRepoStatusModel {
    pub fn new(remote_path: RemotePath, ctx: &mut ModelContext<Self>) -> Self {
        let mgr = RemoteServerManager::handle(ctx);
        ctx.subscribe_to_model(&mgr, Self::handle_manager_event);
        let model = Self {
            remote_path,
            metadata: None,
        };
        model.request_snapshot(ctx);
        model
    }

    fn handle_manager_event(
        &mut self,
        event: &RemoteServerManagerEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        match event {
            RemoteServerManagerEvent::GitStatusPushReceived {
                host_id,
                repo_path,
                metadata,
            } if host_id == &self.remote_path.host_id && repo_path == &self.remote_path.path => {
                self.apply_push(metadata, ctx);
            }
            RemoteServerManagerEvent::HostConnected { host_id }
                if host_id == &self.remote_path.host_id =>
            {
                self.request_snapshot(ctx);
            }
            _ => {}
        }
    }

    pub(super) fn request_snapshot(&self, ctx: &mut ModelContext<Self>) {
        RemoteServerManager::handle(ctx).update(ctx, |mgr, _| {
            mgr.update_git_status(self.remote_path.host_id.clone(), &self.remote_path.path);
        });
    }

    /// Decode a pushed `GitStatusMetadata` (branch + stats) and replace the
    /// stored value, emitting `MetadataChanged`.
    fn apply_push(&mut self, metadata: &proto::GitStatusMetadata, ctx: &mut ModelContext<Self>) {
        match GitStatusMetadata::try_from(metadata) {
            Ok(status) => {
                self.metadata = Some(status);
                ctx.emit(GitRepoStatusEvent::MetadataChanged);
            }
            Err(error) => {
                warp_core::safe_error!(
                    safe: ("RemoteGitRepoStatusModel: failed to decode git status push"),
                    full: ("RemoteGitRepoStatusModel: failed to decode git status push: {error}")
                );
            }
        }
    }

    pub fn metadata(&self) -> Option<&GitStatusMetadata> {
        self.metadata.as_ref()
    }
}

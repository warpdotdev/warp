use std::path::{Path, PathBuf};
use std::time::Duration;

use async_channel::Sender;
use repo_metadata::repository::{RepositorySubscriber, SubscriberId};
use repo_metadata::{Repository, RepositoryUpdate};
use warpui::r#async::SpawnedFutureHandle;
use warpui::{Entity, ModelContext, ModelHandle};

use super::{GitRepoStatusEvent, GitStatusMetadata};
use crate::code_review::diff_state::diff_metadata_against_head;
use crate::context_chips::display_chip::GitBranchTrackingStatus;
use crate::throttle::throttle;
use crate::util::git::{detect_current_branch_display, detect_main_branch};

/// Per-repository model that owns the filesystem watcher and exposes git status
/// metadata. Consumers hold a `ModelHandle<GitRepoStatusModel>` and subscribe
/// to its events directly — no path-filtering required.
///
/// When all strong handles are dropped the model (and its watcher) is
/// automatically torn down.
pub struct LocalGitRepoStatusModel {
    repo_path: PathBuf,
    repository: ModelHandle<Repository>,
    subscriber_id: Option<SubscriberId>,
    metadata: Option<GitStatusMetadata>,
    computing_metadata_abort_handle: Option<SpawnedFutureHandle>,
}

impl Entity for LocalGitRepoStatusModel {
    type Event = GitRepoStatusEvent;
}

impl LocalGitRepoStatusModel {
    /// Create a new per-repo status model, set up the filesystem watcher, and
    /// kick off the initial metadata computation.
    pub(super) fn new(
        repo_path: PathBuf,
        repository_model: ModelHandle<Repository>,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        let mut model = Self {
            repo_path: repo_path.clone(),
            repository: repository_model.clone(),
            subscriber_id: None,
            metadata: None,
            computing_metadata_abort_handle: None,
        };

        // Kick off initial metadata computation.
        model.refresh_metadata(ctx);

        // Start watching for filesystem changes.
        let (repository_update_tx, repository_update_rx) = async_channel::unbounded();
        let (throttled_tx, throttled_rx) = async_channel::unbounded();
        let start = repository_model.update(ctx, |repo, ctx| {
            repo.start_watching(
                Box::new(GitStatusRepositorySubscriber {
                    repository_update_tx,
                }),
                ctx,
            )
        });
        model.subscriber_id = Some(start.subscriber_id);

        // Handle watcher registration.
        ctx.spawn(start.registration_future, |me, result, ctx| {
            if let Err(err) = result {
                log::warn!("GitRepoStatusModel: watcher registration failed: {err}");
                if let Some(subscriber_id) = me.subscriber_id.take() {
                    me.repository.update(ctx, |repo, ctx| {
                        repo.stop_watching(subscriber_id, ctx);
                    });
                }
            }
        });

        // Stream raw updates; determine whether a throttled metadata refresh is warranted.
        {
            let throttled_tx_clone = throttled_tx;
            ctx.spawn_stream_local(
                repository_update_rx,
                move |_me, update: RepositoryUpdate, _ctx| {
                    if Self::should_refresh_metadata(&update) {
                        let _ = throttled_tx_clone.try_send(());
                    }
                },
                |_, _| {},
            );
        }

        // Throttled metadata refresh (at most once every 5 seconds).
        ctx.spawn_stream_local(
            throttle(Duration::from_secs(5), throttled_rx),
            |me, _, ctx| {
                me.refresh_metadata(ctx);
            },
            |_, _| {},
        );

        model
    }

    /// Read the current metadata.  Returns `None` if metadata hasn't been
    /// computed yet.
    pub fn metadata(&self) -> Option<&GitStatusMetadata> {
        self.metadata.as_ref()
    }

    /// The path to the repository root.
    pub fn repo_path(&self) -> &Path {
        &self.repo_path
    }

    /// Manually trigger a metadata refresh.  Called by the terminal view after
    /// events that may have changed git state (block completed, agent file
    /// edits, etc.).
    pub fn refresh_metadata(&mut self, ctx: &mut ModelContext<Self>) {
        if let Some(handle) = self.computing_metadata_abort_handle.take() {
            handle.abort();
        }
        let repo_path_buf = self.repo_path.clone();
        self.computing_metadata_abort_handle = Some(ctx.spawn(
            async move { Self::load_metadata(repo_path_buf).await },
            |me, result, ctx| {
                me.handle_metadata_result(result, ctx);
            },
        ));
    }

    // ── internal helpers ─────────────────────────────────────────────

    fn handle_metadata_result(
        &mut self,
        result: anyhow::Result<GitStatusMetadata>,
        ctx: &mut ModelContext<Self>,
    ) {
        match result {
            Ok(metadata) => {
                self.metadata = Some(metadata);
            }
            Err(e) => {
                log::warn!("GitRepoStatusModel: metadata load failed: {e}");
                self.metadata = None;
            }
        }
        ctx.emit(GitRepoStatusEvent::MetadataChanged);
    }

    /// Decide whether a `RepositoryUpdate` warrants a metadata refresh.
    fn should_refresh_metadata(update: &RepositoryUpdate) -> bool {
        if update.is_empty() {
            return false;
        }
        if update.commit_updated || update.index_lock_detected || update.remote_ref_updated {
            return true;
        }
        // Check if any non-ignored file was touched.
        let changed_count = update
            .added
            .iter()
            .chain(&update.modified)
            .chain(&update.deleted)
            .chain(update.moved.keys())
            .chain(update.moved.values())
            .filter(|f| !f.is_ignored)
            .count();
        changed_count > 0
    }

    fn parse_branch_tracking_counts(output: &str) -> Option<(u32, u32, u32)> {
        let mut parts = output.split_whitespace();
        let ahead = parts.next()?.parse().ok()?;
        let behind = parts.next()?.parse().ok()?;
        let equivalent = parts.next().map(str::parse).transpose().ok()?.unwrap_or(0);
        Some((ahead, behind, equivalent))
    }

    async fn branch_tracking_status(
        repo_path: &Path,
        current_branch_name: &str,
    ) -> GitBranchTrackingStatus {
        let upstream = warp_util::git::run_git_command(
            repo_path,
            &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"],
        )
        .await
        .ok()
        .and_then(|output| {
            output
                .lines()
                .next()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(str::to_string)
        });

        let Some(upstream) = upstream else {
            return GitBranchTrackingStatus::new(current_branch_name.to_string(), None, 0, 0);
        };

        let counts = warp_util::git::run_git_command(
            repo_path,
            &[
                "rev-list",
                "--left-right",
                "--cherry-mark",
                "--count",
                "HEAD...@{u}",
            ],
        )
        .await
        .ok()
        .and_then(|output| Self::parse_branch_tracking_counts(&output));

        let Some((ahead, behind, equivalent)) = counts else {
            return GitBranchTrackingStatus::without_counts(
                current_branch_name.to_string(),
                Some(upstream),
            );
        };

        if ahead == 0 && behind == 0 && equivalent > 0 {
            return GitBranchTrackingStatus::rebased(current_branch_name.to_string(), upstream);
        }

        GitBranchTrackingStatus::new(
            current_branch_name.to_string(),
            Some(upstream),
            ahead,
            behind,
        )
    }

    /// Compute metadata for a repo — branch names and diff stats against HEAD.
    ///
    /// This reuses logic extracted from `DiffStateModel::load_metadata_for_repo`
    /// but only computes the HEAD (uncommitted) stats since that's all the git
    /// chip needs.
    async fn load_metadata(repo_path: PathBuf) -> anyhow::Result<GitStatusMetadata> {
        // Detect main branch.
        let main_branch_name = detect_main_branch(&repo_path).await?;
        // Detect current branch (using the display variant so detached HEAD
        // shows the short SHA instead of the literal "HEAD").
        let current_branch_name = detect_current_branch_display(&repo_path).await?;
        // Diff stats against HEAD.
        let stats_against_head = diff_metadata_against_head(&repo_path).await?;
        let branch_tracking_status =
            Self::branch_tracking_status(&repo_path, &current_branch_name).await;

        Ok(GitStatusMetadata {
            current_branch_name,
            main_branch_name,
            stats_against_head: stats_against_head.aggregate_stats,
            branch_tracking_status,
        })
    }
}

#[cfg(test)]
impl LocalGitRepoStatusModel {
    pub(crate) fn new_for_test(
        repository: ModelHandle<Repository>,
        metadata: Option<GitStatusMetadata>,
    ) -> Self {
        Self {
            repo_path: PathBuf::from("/test"),
            repository,
            subscriber_id: None,
            metadata,
            computing_metadata_abort_handle: None,
        }
    }

    pub(crate) fn set_metadata_for_test(
        &mut self,
        metadata: Option<GitStatusMetadata>,
        ctx: &mut ModelContext<Self>,
    ) {
        self.metadata = metadata;
        ctx.emit(GitRepoStatusEvent::MetadataChanged);
    }
}

#[cfg(test)]
#[path = "local_tests.rs"]
mod tests;

impl Drop for LocalGitRepoStatusModel {
    fn drop(&mut self) {
        // Note: we cannot call `repository.update()` here because `Drop` does
        // not have access to `ModelContext`.  The `Repository` model will clean
        // up the subscriber when it notices the channel has been dropped.
        if let Some(handle) = self.computing_metadata_abort_handle.take() {
            handle.abort();
        }
    }
}

// ── Repository subscriber adapter ───────────────────────────────────────────

struct GitStatusRepositorySubscriber {
    repository_update_tx: Sender<RepositoryUpdate>,
}

impl RepositorySubscriber for GitStatusRepositorySubscriber {
    fn on_scan(
        &mut self,
        _repository: &Repository,
        _ctx: &mut ModelContext<Repository>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'static>> {
        Box::pin(async {})
    }

    fn on_files_updated(
        &mut self,
        repository: &Repository,
        update: &RepositoryUpdate,
        _ctx: &mut ModelContext<Repository>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'static>> {
        let tx = self.repository_update_tx.clone();
        let update = update.clone();
        let index_lock_path = repository.git_dir().join("index.lock");
        Box::pin(async move {
            // Suppress commit_updated events while the git index is locked to
            // avoid reacting to stale intermediate state during git operations.
            if update.commit_updated && async_fs::metadata(&index_lock_path).await.is_ok() {
                return;
            }
            let _ = tx.send(update).await;
        })
    }
}

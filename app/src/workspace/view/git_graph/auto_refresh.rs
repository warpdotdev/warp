//! Auto-refresh: keep the graph in sync with git operations made *outside* the
//! panel — terminal commands, an IDE, an external `git` — by subscribing to the
//! selected repository's `.git` changes through `repo_metadata`'s `Repository`
//! watcher. That shares the same filesystem watch the terminal's git status
//! already uses (and its battle-tested `.git` event routing for worktrees /
//! packed-refs / shared refs), rather than starting a second watcher on `.git`.
//!
//! Three concerns live here, all `local_fs`-gated like the rest of the
//! `repo_metadata` integration (a non-`local_fs` build has no watcher, so the
//! graph only refreshes on explicit user actions):
//! - **Position restore** ([`relocate_view`]): after a refresh re-reads the
//!   graph, map the pre-refresh selection / scroll anchor onto the new commit
//!   list *by hash* so an auto-refresh doesn't yank the user away from what they
//!   were looking at.
//! - **Throttle** ([`throttle_signal`]): rate-limit reload signals to one
//!   refresh per [`AUTO_REFRESH_MIN_INTERVAL`], trailing-edge — so an event
//!   storm (builds, bulk file writes) doesn't fan out into a git-process storm,
//!   while the graph still converges on the final state.
//! - **Subscription** ([`should_reload`] + [`GitGraphRepositorySubscriber`]):
//!   turn a [`repo_metadata::RepositoryUpdate`] into a "reload" signal.

#[cfg(feature = "local_fs")]
use super::data::CommitNode;

/// Where the commit list should sit after an auto-refresh re-read, expressed as
/// indices into the *new* commit list.
#[cfg(feature = "local_fs")]
pub(crate) struct ViewAnchor {
    /// Row to re-select — the pre-refresh selected commit, if it still exists.
    pub(crate) selected: Option<usize>,
    /// Row to pin to the top of the viewport, keeping the user's place.
    pub(crate) scroll_to: usize,
}

/// Snapshot the user's place in the outgoing commit list as hashes — the
/// capture half of [`relocate_view`]. Taken when a reload *lands* (not when it
/// starts), so a selection made while the reload was in flight wins over any
/// pre-reload state.
///
/// - `selected`: selected row as a commit index.
/// - `visible_start`: top visible row as a UniformList index, which counts the
///   synthetic uncommitted row when present (`has_uncommitted_row`).
///
/// Returns `(selected_hash, anchor_hash)`. Out-of-range indices (the list
/// shrank under a stale index) yield `None`.
#[cfg(feature = "local_fs")]
pub(crate) fn capture_anchor(
    commits: &[CommitNode],
    selected: Option<usize>,
    visible_start: usize,
    has_uncommitted_row: bool,
) -> (Option<String>, Option<String>) {
    let offset = usize::from(has_uncommitted_row);
    let selected_hash = selected
        .and_then(|i| commits.get(i))
        .map(|c| c.hash.clone());
    let anchor_hash = commits
        .get(visible_start.saturating_sub(offset))
        .map(|c| c.hash.clone());
    (selected_hash, anchor_hash)
}

/// Map a pre-refresh selection + scroll anchor onto a freshly-loaded commit
/// list, *by commit hash*, so an auto-refresh doesn't jump the view.
///
/// - `selected_hash`: hash of the row that was selected (if any).
/// - `anchor_hash`: hash of the row that was at the top of the viewport.
///
/// A hash that no longer exists in `new_commits` (e.g. dropped by a rebase /
/// amend) degrades gracefully: the scroll anchor falls back to the selected
/// row, then to the top (index 0 = newest commit).
#[cfg(feature = "local_fs")]
pub(crate) fn relocate_view(
    new_commits: &[CommitNode],
    selected_hash: Option<&str>,
    anchor_hash: Option<&str>,
) -> ViewAnchor {
    let selected = selected_hash.and_then(|h| index_of_hash(new_commits, h));
    let scroll_to = anchor_hash
        .and_then(|h| index_of_hash(new_commits, h))
        .or(selected)
        .unwrap_or(0);
    ViewAnchor {
        selected,
        scroll_to,
    }
}

#[cfg(feature = "local_fs")]
fn index_of_hash(commits: &[CommitNode], hash: &str) -> Option<usize> {
    commits.iter().position(|c| c.hash == hash)
}

/// Minimum spacing between two auto-refresh reloads. The first signal after a
/// quiet period refreshes immediately; within this window further signals
/// collapse into a single trailing catch-up refresh.
#[cfg(feature = "local_fs")]
pub(crate) const AUTO_REFRESH_MIN_INTERVAL: std::time::Duration = std::time::Duration::from_secs(3);

/// What to do with one watcher signal fed into the refresh throttle.
#[cfg(feature = "local_fs")]
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ThrottleDecision {
    /// Outside the cooldown window (or never refreshed): refresh immediately.
    RefreshNow,
    /// First signal inside the window: schedule one catch-up refresh after the
    /// carried delay (the remainder of the window).
    Defer(std::time::Duration),
    /// A catch-up is already scheduled and covers this change too: drop it.
    AlreadyDeferred,
}

/// Throttle reload signals to one refresh per `min_interval`, trailing-edge:
/// the leading signal refreshes immediately (a `git commit` in the terminal
/// shows up right away), and signals inside the window collapse into a single
/// catch-up at the window's end — never dropped outright, so the graph always
/// converges on the final state.
#[cfg(feature = "local_fs")]
pub(crate) fn throttle_signal(
    elapsed_since_last: Option<std::time::Duration>,
    catch_up_pending: bool,
    min_interval: std::time::Duration,
) -> ThrottleDecision {
    match elapsed_since_last {
        Some(elapsed) if elapsed < min_interval => {
            if catch_up_pending {
                ThrottleDecision::AlreadyDeferred
            } else {
                ThrottleDecision::Defer(min_interval - elapsed)
            }
        }
        // Never refreshed, or the window has passed (a stale pending mark is
        // absorbed by the caller into this refresh).
        _ => ThrottleDecision::RefreshNow,
    }
}

/// What the open detail pane should do after an auto-refresh reload lands.
#[cfg(feature = "local_fs")]
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum DetailRefresh {
    /// The detail's target survived the reload: leave the pane as is.
    Keep,
    /// The uncommitted row is selected and the working tree still has changes:
    /// re-read the uncommitted detail in place (the very change that triggered
    /// the reload may have altered it).
    RefreshUncommitted,
    /// The detail's target is gone — the selected commit vanished (e.g.
    /// amended away), or the tree became clean while the uncommitted row was
    /// selected: drop the now-stale detail.
    Clear,
}

/// Decide the detail pane's fate after an auto-refresh reload. The uncommitted
/// row is not hash-addressed, so [`relocate_view`] always reports it as
/// `selected: None`; it must be told apart from a vanished commit selection by
/// `uncommitted_selected` — otherwise an open uncommitted detail gets blanked
/// by the reload its own file change triggered.
#[cfg(feature = "local_fs")]
pub(crate) fn detail_refresh_after_reload(
    uncommitted_selected: bool,
    uncommitted_count: usize,
    relocated_selected: Option<usize>,
) -> DetailRefresh {
    if uncommitted_selected {
        if uncommitted_count > 0 {
            DetailRefresh::RefreshUncommitted
        } else {
            DetailRefresh::Clear
        }
    } else if relocated_selected.is_none() {
        DetailRefresh::Clear
    } else {
        DetailRefresh::Keep
    }
}

#[cfg(feature = "local_fs")]
pub(crate) use subscription::GitGraphRepositorySubscriber;

/// Subscription side: turns `repo_metadata` repository events into reload
/// signals.
#[cfg(feature = "local_fs")]
mod subscription {
    use std::future::Future;
    use std::pin::Pin;

    use async_channel::Sender;
    use repo_metadata::repository::RepositorySubscriber;
    use repo_metadata::{Repository, RepositoryUpdate};
    use warpui::ModelContext;

    /// Whether a repository change warrants re-reading the graph: a local HEAD /
    /// branch move (`commit_updated`), a tracked remote-ref update
    /// (`remote_ref_updated`), or any non-ignored working-tree file change (so
    /// the uncommitted-changes row's count stays current).
    pub(crate) fn should_reload(update: &RepositoryUpdate) -> bool {
        update.commit_updated || update.remote_ref_updated || has_non_ignored_file_change(update)
    }

    /// Whether the update touches any non-ignored working-tree file.
    fn has_non_ignored_file_change(update: &RepositoryUpdate) -> bool {
        update
            .added
            .iter()
            .chain(&update.modified)
            .chain(&update.deleted)
            .chain(update.moved.keys())
            .chain(update.moved.values())
            .any(|f| !f.is_ignored)
    }

    /// Bridges `repo_metadata`'s per-repository watcher to the Git Graph view:
    /// on a graph-affecting change it pushes `()` onto the channel the view
    /// drains to trigger a position-preserving reload.
    pub(crate) struct GitGraphRepositorySubscriber {
        pub(crate) signal_tx: Sender<()>,
    }

    impl RepositorySubscriber for GitGraphRepositorySubscriber {
        fn on_scan(
            &mut self,
            _repository: &Repository,
            _ctx: &mut ModelContext<Repository>,
        ) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>> {
            Box::pin(async {})
        }

        fn on_files_updated(
            &mut self,
            repository: &Repository,
            update: &RepositoryUpdate,
            _ctx: &mut ModelContext<Repository>,
        ) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>> {
            let signal_tx = self.signal_tx.clone();
            let reload = should_reload(update);
            let commit_updated = update.commit_updated;
            // While a git operation is mid-flight the index lock is held; skip
            // the reload then, since the operation's final ref write fires
            // another event once the lock is gone — so we still refresh, just on
            // a settled state rather than a half-applied one.
            let index_lock = repository.git_dir().join("index.lock");
            Box::pin(async move {
                if !reload {
                    return;
                }
                if commit_updated && async_fs::metadata(&index_lock).await.is_ok() {
                    return;
                }
                let _ = signal_tx.send(()).await;
            })
        }
    }
}

#[cfg(all(test, feature = "local_fs"))]
#[path = "auto_refresh_tests.rs"]
mod tests;

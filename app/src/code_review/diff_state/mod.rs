//! Unified diff state module.
//!
//! [`DiffStateModel`] is an enum that provides a unified API over local and remote models.
//! It holds one of [`LocalDiffStateModel`] or [`RemoteDiffStateModel`] and dispatches
//! operations to whichever is active.
//! All consumers should use `DiffStateModel` rather than accessing sub-models directly.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use warp_core::SessionId;
use warp_util::remote_path::RemotePath;
use warp_util::standardized_path::StandardizedPath;
use warpui::{AppContext, ModelContext, ModelHandle};

use crate::code_review::diff_size_limits::DiffSize;
use crate::util::git::{BranchEntry, Commit, FileChangeEntry, PrInfo};
#[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
mod local;
#[cfg(feature = "local_fs")]
pub(crate) use local::diff_metadata_against_head;
pub use local::LocalDiffStateModel;

mod remote;
pub use remote::RemoteDiffStateModel;

#[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
mod error;
pub(crate) use error::DiffStateError;

/// What to chain after a commit: commit only, commit + push, or commit + push
/// + create-PR. The single shared commit-chain vocabulary, used end to end: the
/// commit dialog stores the user's selection as this, both the local
/// (`git_actions::run_commit_chain`) and remote (`DiffStateModel::git_commit_chain`)
/// backends accept it, and it's converted to the wire enum
/// (`proto::GitCommitChainMode`) at the manager boundary via the `From` impl in
/// the `diff_state_proto` module.
#[allow(clippy::enum_variant_names)] // `Commit` prefix is intentional: every chain starts with a commit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommitChainMode {
    CommitOnly,
    CommitAndPush,
    CommitAndCreatePr,
}

/// Identifies the host of a [`DiffStateModel`] so failure telemetry can be
/// attributed to where the model actually ran. This is more specific than the
/// local/remote split already encoded by `is_local`: a [`LocalDiffStateModel`]
/// can be instantiated on the user's client (`ClientLocal`) or on a remote
/// daemon (`RemoteDaemon`) serving subscribers, and only the host knows which.
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
pub enum BackendOrigin {
    /// `LocalDiffStateModel` running on the user's client against local files.
    #[serde(rename = "client_local")]
    ClientLocal,
    /// `RemoteDiffStateModel` running on the user's client; talks to a daemon.
    #[serde(rename = "client_remote")]
    ClientRemote,
    /// `LocalDiffStateModel` running on a remote daemon, serving subscribers.
    #[serde(rename = "remote_daemon")]
    RemoteDaemon,
}

/// Identifies the diff-state operation that produced a [`DiffStateError`]
/// on the `LoadDiffFailed` telemetry path. Carried alongside the error so
/// failures can be sliced by originating operation — every operation shares
/// the same failure pool, so the error variant alone doesn't reveal where
/// it came from.
///
/// Metadata-load failures are reported through a dedicated
/// `LoadMetadataFailed` event and therefore don't need a variant here.
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
pub enum DiffOperation {
    /// Per-file diff refresh triggered by the file-invalidation queue.
    #[serde(rename = "file_invalidation")]
    FileInvalidation,
    /// Full repo-wide diff snapshot load.
    #[serde(rename = "diff_load")]
    DiffLoad,
    /// Client-side reaction to a remote daemon's diff-state response.
    #[serde(rename = "remote_diff")]
    RemoteDiff,
}

// -- Shared types ──────────────────────────────────────────────────────

/// Represents the status of a file in the git working directory
/// This matches Git Desktop's AppFileStatusKind enum
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum GitFileStatus {
    New,
    Modified,
    Deleted,
    Renamed { old_path: String },
    Copied { old_path: String },
    Untracked,
    Conflicted,
}

impl GitFileStatus {
    pub fn is_renamed(&self) -> bool {
        matches!(self, Self::Renamed { .. })
    }

    pub fn is_new_file(&self) -> bool {
        matches!(self, Self::New | Self::Untracked)
    }
}

#[derive(Clone, Debug)]
pub struct FileStatusInfo {
    pub path: StandardizedPath,
    pub status: GitFileStatus,
}

impl TryFrom<&str> for GitFileStatus {
    type Error = anyhow::Error;

    fn try_from(status_code: &str) -> Result<Self> {
        match status_code {
            ".M" | "M." | "MM" => Ok(GitFileStatus::Modified),
            ".A" | "A." | "AM" => Ok(GitFileStatus::New),
            ".D" | "D." | "AD" => Ok(GitFileStatus::Deleted),
            _ => Ok(GitFileStatus::Modified), // Default fallback
        }
    }
}

/// Represents a single line in a diff hunk, as rendered by `git diff`.
/// This matches Git Desktop's DiffLine structure.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DiffLine {
    pub line_type: DiffLineType,
    pub old_line_number: Option<usize>,
    pub new_line_number: Option<usize>,
    pub text: String,
    pub no_trailing_newline: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum DiffLineType {
    Context,
    Add,
    Delete,
    HunkHeader,
}

/// Represents a hunk of changes in a file diff, as rendered by `git diff`,
/// including the header and context lines before/after an insertion or
/// deletion.
/// This matches Git Desktop's DiffHunk structure.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DiffHunk {
    pub old_start_line: usize,
    pub old_line_count: usize,
    pub new_start_line: usize,
    pub new_line_count: usize,
    pub lines: Vec<DiffLine>,
    pub unified_diff_start: usize,
    pub unified_diff_end: usize,
}

/// Represents the diff for a single file, as rendered by `git diff`.
/// This matches Git Desktop's FileDiff structure.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FileDiff {
    /// Repo-relative path for this diff file. Absolute file identities should use
    /// `StandardizedPath` or `LocalOrRemotePath` at API boundaries.
    pub file_path: String,
    pub status: GitFileStatus,
    pub hunks: Arc<Vec<DiffHunk>>,
    pub is_binary: bool,
    pub is_autogenerated: bool,
    pub max_line_number: usize,
    pub has_hidden_bidi_chars: bool,
    pub size: DiffSize,
}

impl FileDiff {
    pub fn is_empty(&self) -> bool {
        self.additions() == 0 && self.deletions() == 0
    }

    /// Returns the number of added lines in this file diff
    pub fn additions(&self) -> usize {
        self.hunks
            .iter()
            .flat_map(|hunk| &hunk.lines)
            .filter(|line| line.line_type == DiffLineType::Add)
            .count()
    }

    /// Returns the number of deleted lines in this file diff
    pub fn deletions(&self) -> usize {
        self.hunks
            .iter()
            .flat_map(|hunk| &hunk.lines)
            .filter(|line| line.line_type == DiffLineType::Delete)
            .count()
    }
}

/// IMPORTANT: This struct contains expensive data like the full content of diff files
/// at base. This should not be cloned at any time.
#[derive(Debug)]
pub struct FileDiffAndContent {
    pub file_diff: FileDiff,
    pub content_at_head: Option<String>,
}

/// IMPORTANT: This struct contains expensive data like the full content of diff files
/// at base. This should not be cloned at any time.
#[derive(Debug)]
pub struct GitDiffWithBaseContent {
    pub files: Vec<FileDiffAndContent>,
    pub total_additions: usize,
    pub total_deletions: usize,
    pub files_changed: usize,
}

impl From<GitDiffWithBaseContent> for GitDiffData {
    fn from(value: GitDiffWithBaseContent) -> Self {
        Self {
            files: value.files.into_iter().map(|file| file.file_diff).collect(),
            total_additions: value.total_additions,
            total_deletions: value.total_deletions,
            files_changed: value.files_changed,
        }
    }
}

impl From<&GitDiffWithBaseContent> for GitDiffData {
    fn from(value: &GitDiffWithBaseContent) -> Self {
        Self {
            files: value
                .files
                .iter()
                .map(|file| file.file_diff.clone())
                .collect(),
            total_additions: value.total_additions,
            total_deletions: value.total_deletions,
            files_changed: value.files_changed,
        }
    }
}

impl From<&GitDiffData> for GitDiffWithBaseContent {
    fn from(value: &GitDiffData) -> Self {
        Self {
            files: value
                .files
                .iter()
                .map(|file_diff| FileDiffAndContent {
                    file_diff: file_diff.clone(),
                    content_at_head: None,
                })
                .collect(),
            total_additions: value.total_additions,
            total_deletions: value.total_deletions,
            files_changed: value.files_changed,
        }
    }
}

/// Represents the complete git diff information for a repository
#[derive(Clone)]
pub struct GitDiffData {
    pub files: Vec<FileDiff>,
    pub total_additions: usize,
    pub total_deletions: usize,
    pub files_changed: usize,
}

impl GitDiffData {
    pub fn is_dirty(&self) -> bool {
        self.total_additions + self.total_deletions + self.files_changed > 0
    }
}

/// Some actions should only apply when a [`GitDiffData`] is dirty, i.e. not empty. This enum allows
/// callers to express this preference.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum GitDeltaPreference {
    Always,
    OnlyDirty,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Default, Serialize)]
pub enum DiffMode {
    /// Show changes in working directory against latest commit (git diff)
    #[default]
    Head,
    /// Show changes in working directory against main branch (git diff $(git merge-base HEAD origin/master))
    MainBranch,
    /// Show changes in working directory against an arbitrary branch (git diff $(git merge-base HEAD <branch>))
    OtherBranch(#[serde(skip_serializing)] String),
}

impl DiffMode {
    /// Creates a DiffMode from a branch name.
    /// If the branch matches the repository's main branch, returns `MainBranch`;
    /// otherwise returns `OtherBranch(branch)`.
    pub fn from_branch(branch: &str, main_branch_name: Option<&str>) -> Self {
        if main_branch_name == Some(branch) {
            DiffMode::MainBranch
        } else {
            DiffMode::OtherBranch(branch.to_string())
        }
    }
}

/// User-visible representation of the diffs we've loaded,
/// which only includes changes against the specific base the user has selected.
#[derive(Debug)]
pub enum DiffState {
    NotInRepository,
    Loading,
    Error(String),
    Loaded,
    /// The remote connection was lost. The model will re-subscribe
    /// automatically when a session becomes available.
    Disconnected,
}

#[derive(Clone, Debug, Default)]
pub struct DiffMetadata {
    pub main_branch_name: String,
    pub current_branch_name: String,
    pub against_head: DiffMetadataAgainstBase,
    pub against_base_branch: Option<DiffMetadataAgainstBase>,
    pub has_head_commit: bool,
    pub unpushed_commits: Vec<Commit>,
    pub upstream_ref: Option<String>,
}

#[derive(Clone, Default, Debug)]
pub struct DiffMetadataAgainstBase {
    pub aggregate_stats: DiffStats,
    /// Per-file change entries (path + additions/deletions) for this base.
    /// Populated from the same numstat that produces `aggregate_stats`, so the
    /// git dialog's Changes box can render without a working-tree read — this
    /// is what lets the box populate for remote repos, where the list rides
    /// along in synced metadata.
    pub files: Vec<FileChangeEntry>,
}

impl DiffMetadataAgainstBase {
    pub fn is_dirty(&self) -> bool {
        !self.aggregate_stats.has_no_changes()
    }
}

#[derive(Debug, Copy, Clone, Default)]
pub struct DiffStats {
    pub files_changed: usize,
    pub total_additions: usize,
    pub total_deletions: usize,
}

impl DiffStats {
    pub(crate) fn has_no_changes(&self) -> bool {
        self.files_changed == 0
    }
}

#[derive(Debug)]
pub enum DiffStateModelEvent {
    /// Event dispatched when the current branch changes.
    CurrentBranchChanged,
    /// Event dispatched when new diffs are computed (full reload).
    NewDiffsComputed {
        diffs: Option<Arc<GitDiffWithBaseContent>>,
        load_duration: Option<Duration>,
    },
    /// Event dispatched when a single file's diff is updated incrementally.
    SingleFileUpdated {
        /// Repo-relative path for the updated file.
        path: String,
        diff: Option<Arc<FileDiffAndContent>>,
    },
    /// Event dispatched when diff metadata (stats, branch info) is refreshed.
    MetadataRefreshed(Box<DiffMetadata>),
    /// The remote connection was lost. Stale diffs should be preserved while
    /// the model waits for a new subscription.
    ConnectionLost,
    /// Branch list received from the backend (local git or remote server).
    BranchesReceived(Vec<BranchEntry>),
    /// A remote git operation completed. The model has already applied any
    /// successful metadata delta to the cached metadata.
    GitOpCompleted(GitOpResult),
    /// An AI-generated commit message arrived from the remote daemon (issued
    /// at commit-dialog open). `Ok` carries the message, `Err` the error
    /// string. The `GitDialog` populates its message editor from this; the
    /// local path fills the editor directly without going through an event.
    CommitMessageGenerated(Result<String, String>),
    /// Committed branch files (`merge_base(HEAD, main)..HEAD`) arrived for the
    /// Create PR dialog's Changes box. Fetched on dialog open and delivered the
    /// same way for both backends: the local model computes them off-thread and
    /// emits this; the remote model emits it on the daemon's RPC response.
    BranchCommittedFilesReceived(Vec<FileChangeEntry>),
}

/// Result of a remote git operation, emitted via
/// `DiffStateModelEvent::GitOpCompleted`. The model applies the post-op
/// delta before emitting, so the dialog only handles UI concerns.
#[derive(Debug, Clone)]
pub enum GitOpResult {
    /// Commit chain completed. `Ok(Some(pr))` when create-PR was part of
    /// the chain; `Ok(None)` for commit-only or commit-and-push.
    CommitChainCompleted(Result<Option<PrInfo>, String>),
    /// Standalone push completed.
    PushCompleted(Result<(), String>),
    /// Standalone create-PR completed.
    PrCreated(Result<PrInfo, String>),
}

// ── Unified model ────────────────────────────────────────────────────────

/// Unified diff state model that dispatches to a local or remote backend.
///
/// Only one variant is populated at a time, since a diff state belongs to
/// exactly one repository (either local or remote). All consumers should
/// interact with this enum rather than accessing sub-models directly.
pub enum DiffStateModel {
    Local(ModelHandle<LocalDiffStateModel>),
    Remote(ModelHandle<RemoteDiffStateModel>),
}

impl warpui::Entity for DiffStateModel {
    type Event = DiffStateModelEvent;
}

impl DiffStateModel {
    // ── Construction ─────────────────────────────────────────────────

    /// Creates a new local-backed `DiffStateModel`. The wrapper subscribes
    /// to the inner model so it can forward events.
    pub fn new_local(path: PathBuf, ctx: &mut ModelContext<Self>) -> Self {
        let repo_path = Some(path.display().to_string());
        let local = ctx
            .add_model(|ctx| LocalDiffStateModel::new(repo_path, BackendOrigin::ClientLocal, ctx));
        ctx.subscribe_to_model(&local, Self::forward_event);
        Self::Local(local)
    }

    /// Creates a new remote-backed `DiffStateModel`. The model is keyed by
    /// `(host_id, repo, mode)` and shared across sessions viewing the same
    /// repo. `preferred_session` is the session that opened this review (when
    /// known): `GetDiffState` is session-scoped, so the manager dispatches it
    /// over that session when it's connected and falls back to any connected
    /// session for the host otherwise. Callers must ensure a session for the
    /// host is connected before constructing.
    pub fn new_remote(
        remote_path: RemotePath,
        preferred_session: Option<SessionId>,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        let remote = ctx.add_model(|ctx| {
            RemoteDiffStateModel::new(remote_path, DiffMode::default(), preferred_session, ctx)
        });
        ctx.subscribe_to_model(&remote, Self::forward_event);
        Self::Remote(remote)
    }

    // ── Event forwarding ─────────────────────────────────────────────

    fn forward_event(&mut self, event: &DiffStateModelEvent, ctx: &mut ModelContext<Self>) {
        match event {
            DiffStateModelEvent::CurrentBranchChanged => {
                ctx.emit(DiffStateModelEvent::CurrentBranchChanged);
            }
            DiffStateModelEvent::NewDiffsComputed {
                diffs,
                load_duration,
            } => {
                ctx.emit(DiffStateModelEvent::NewDiffsComputed {
                    diffs: diffs.clone(),
                    load_duration: *load_duration,
                });
            }
            DiffStateModelEvent::SingleFileUpdated { path, diff } => {
                ctx.emit(DiffStateModelEvent::SingleFileUpdated {
                    path: path.clone(),
                    diff: diff.clone(),
                });
            }
            DiffStateModelEvent::MetadataRefreshed(metadata) => {
                ctx.emit(DiffStateModelEvent::MetadataRefreshed(metadata.clone()));
            }
            DiffStateModelEvent::ConnectionLost => {
                ctx.emit(DiffStateModelEvent::ConnectionLost);
            }
            DiffStateModelEvent::BranchesReceived(branches) => {
                ctx.emit(DiffStateModelEvent::BranchesReceived(branches.clone()));
            }
            DiffStateModelEvent::GitOpCompleted(result) => {
                ctx.emit(DiffStateModelEvent::GitOpCompleted(result.clone()));
            }
            DiffStateModelEvent::CommitMessageGenerated(result) => {
                ctx.emit(DiffStateModelEvent::CommitMessageGenerated(result.clone()));
            }
            DiffStateModelEvent::BranchCommittedFilesReceived(files) => {
                ctx.emit(DiffStateModelEvent::BranchCommittedFilesReceived(
                    files.clone(),
                ));
            }
        }
    }

    // ── Unified read API ─────────────────────────────────────────────

    pub(crate) fn get(&self, ctx: &AppContext) -> DiffState {
        match self {
            Self::Local(m) => m.as_ref(ctx).get(),
            Self::Remote(m) => m.as_ref(ctx).get(),
        }
    }

    pub(crate) fn diff_mode(&self, ctx: &AppContext) -> DiffMode {
        match self {
            Self::Local(m) => m.as_ref(ctx).diff_mode(),
            Self::Remote(m) => m.as_ref(ctx).diff_mode(),
        }
    }

    pub(crate) fn get_uncommitted_stats(&self, ctx: &AppContext) -> Option<DiffStats> {
        match self {
            Self::Local(m) => m.as_ref(ctx).get_uncommitted_stats(),
            Self::Remote(m) => m.as_ref(ctx).get_uncommitted_stats(),
        }
    }

    /// Per-file entries for the uncommitted-vs-HEAD changes, sourced from
    /// synced metadata (`against_head.files`). The per-file counterpart to
    /// `get_uncommitted_stats`. Empty until metadata loads. Available for both
    /// backends, so the commit dialog's Changes box works for remote repos
    /// without reading the working tree.
    pub(crate) fn uncommitted_file_entries<'a>(
        &self,
        ctx: &'a AppContext,
    ) -> &'a [FileChangeEntry] {
        match self {
            Self::Local(m) => m.as_ref(ctx).uncommitted_file_entries(),
            Self::Remote(m) => m.as_ref(ctx).uncommitted_file_entries(),
        }
    }

    pub(crate) fn get_main_branch_name(&self, ctx: &AppContext) -> Option<String> {
        match self {
            Self::Local(m) => m.as_ref(ctx).get_main_branch_name(),
            Self::Remote(m) => m.as_ref(ctx).get_main_branch_name(),
        }
    }

    pub fn get_current_branch_name(&self, ctx: &AppContext) -> Option<String> {
        match self {
            Self::Local(m) => m.as_ref(ctx).get_current_branch_name(),
            Self::Remote(m) => m.as_ref(ctx).get_current_branch_name(),
        }
    }

    pub(crate) fn is_on_main_branch(&self, ctx: &AppContext) -> bool {
        match self {
            Self::Local(m) => m.as_ref(ctx).is_on_main_branch(),
            Self::Remote(m) => m.as_ref(ctx).is_on_main_branch(),
        }
    }

    pub(crate) fn unpushed_commits<'a>(&self, ctx: &'a AppContext) -> &'a [Commit] {
        match self {
            Self::Local(m) => m.as_ref(ctx).unpushed_commits(),
            Self::Remote(m) => m.as_ref(ctx).unpushed_commits(),
        }
    }

    pub(crate) fn upstream_ref<'a>(&self, ctx: &'a AppContext) -> Option<&'a str> {
        match self {
            Self::Local(m) => m.as_ref(ctx).upstream_ref(),
            Self::Remote(m) => m.as_ref(ctx).upstream_ref(),
        }
    }

    pub(crate) fn upstream_differs_from_main(&self, ctx: &AppContext) -> bool {
        match self {
            Self::Local(m) => m.as_ref(ctx).upstream_differs_from_main(),
            Self::Remote(m) => m.as_ref(ctx).upstream_differs_from_main(),
        }
    }

    pub(crate) fn is_git_operation_blocked(&self, ctx: &AppContext) -> bool {
        match self {
            Self::Local(m) => m.as_ref(ctx).is_git_operation_blocked(ctx),
            // Remote git ops rely on the daemon-side `.git` sentinel as the
            // authoritative guard, so the client doesn't pre-emptively block.
            Self::Remote(_) => false,
        }
    }

    pub(crate) fn has_head(&self, ctx: &AppContext) -> bool {
        match self {
            Self::Local(m) => m.as_ref(ctx).has_head(),
            Self::Remote(m) => m.as_ref(ctx).has_head(),
        }
    }

    // ── Unified write API ─────────────────────────────────────────────

    /// `preferred_session` is the session that triggered this call (the
    /// session showing the review). It's forwarded per-call to the remote
    /// model so the `GetDiffState` RPC rides that session; the local backend
    /// ignores it. The remote model never caches it.
    pub(crate) fn set_diff_mode(
        &self,
        mode: DiffMode,
        should_fetch_base: bool,
        track_load_duration: bool,
        preferred_session: Option<SessionId>,
        ctx: &mut ModelContext<Self>,
    ) {
        match self {
            Self::Local(local) => {
                local.update(ctx, |local, ctx| {
                    local.set_diff_mode(mode, should_fetch_base, track_load_duration, ctx);
                });
            }
            Self::Remote(model) => {
                model.update(ctx, |model, ctx| {
                    model.set_diff_mode(mode, track_load_duration, preferred_session, ctx);
                });
            }
        }
    }

    pub(crate) fn set_diff_mode_and_fetch_base(
        &self,
        mode: DiffMode,
        preferred_session: Option<SessionId>,
        ctx: &mut ModelContext<Self>,
    ) {
        match self {
            Self::Local(local) => {
                local.update(ctx, |local, ctx| {
                    local.set_diff_mode_and_fetch_base(mode, ctx);
                });
            }
            Self::Remote(model) => {
                model.update(ctx, |model, ctx| {
                    model.set_diff_mode(mode, true, preferred_session, ctx);
                });
            }
        }
    }

    pub(crate) fn load_diffs_for_current_repo(
        &self,
        should_fetch_base: bool,
        track_load_duration: bool,
        preferred_session: Option<SessionId>,
        ctx: &mut ModelContext<Self>,
    ) {
        match self {
            Self::Local(local) => {
                local.update(ctx, |local, ctx| {
                    local.load_diffs_for_current_repo(should_fetch_base, track_load_duration, ctx);
                });
            }
            Self::Remote(remote) => {
                remote.update(ctx, |remote, ctx| {
                    remote.fetch_fresh_snapshot(track_load_duration, preferred_session, ctx);
                });
            }
        }
    }

    pub(crate) fn set_code_review_metadata_refresh_enabled(
        &self,
        enabled: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        match self {
            Self::Local(local) => {
                local.update(ctx, |local, ctx| {
                    local.set_code_review_metadata_refresh_enabled(enabled, ctx);
                });
            }
            Self::Remote(_) => {}
        }
    }

    pub(crate) fn fetch_branches(&self, ctx: &mut ModelContext<Self>) {
        match self {
            Self::Local(local) => {
                local.update(ctx, |local, ctx| {
                    local.fetch_branches(ctx);
                });
            }
            Self::Remote(model) => {
                model.update(ctx, |model, ctx| {
                    model.fetch_branches(ctx);
                });
            }
        }
    }

    pub(crate) fn refresh_metadata_after_git_operation(&self, ctx: &mut ModelContext<Self>) {
        match self {
            Self::Local(local) => {
                local.update(ctx, |local, ctx| {
                    local.refresh_metadata_after_git_operation(ctx);
                });
            }
            Self::Remote(_) => {}
        }
    }

    pub(crate) fn discard_files(
        &self,
        file_infos: Vec<FileStatusInfo>,
        should_stash: bool,
        branch_name: Option<String>,
        ctx: &mut ModelContext<Self>,
    ) {
        match self {
            Self::Local(local) => {
                local.update(ctx, |local, ctx| {
                    local.discard_files(file_infos, should_stash, branch_name, ctx);
                });
            }
            Self::Remote(model) => {
                model.update(ctx, |model, ctx| {
                    model.discard_files(file_infos, should_stash, branch_name, ctx);
                });
            }
        }
    }

    /// Runs a commit chain (commit, then optionally push/create-PR).
    pub(crate) fn git_commit_chain(
        &self,
        mode: CommitChainMode,
        message: String,
        include_unstaged: bool,
        branch: String,
        autogenerate_pr_content: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        match self {
            Self::Local(local) => local.update(ctx, |local, ctx| {
                local.git_commit_chain(
                    mode,
                    message,
                    include_unstaged,
                    branch,
                    autogenerate_pr_content,
                    ctx,
                );
            }),
            Self::Remote(remote) => remote.update(ctx, |remote, ctx| {
                remote.git_commit_chain(
                    mode,
                    message,
                    include_unstaged,
                    branch,
                    autogenerate_pr_content,
                    ctx,
                );
            }),
        }
    }

    /// Issues an AI commit-message generation request.
    pub(crate) fn generate_commit_message(
        &self,
        include_unstaged: bool,
        branch_name: String,
        ctx: &mut ModelContext<Self>,
    ) {
        match self {
            Self::Local(local) => local.update(ctx, |local, ctx| {
                local.generate_commit_message(include_unstaged, branch_name, ctx);
            }),
            Self::Remote(remote) => remote.update(ctx, |remote, ctx| {
                remote.generate_commit_message(include_unstaged, branch_name, ctx);
            }),
        }
    }

    /// Pushes the given git branch to the remote origin.
    pub(crate) fn git_push(&self, branch: String, ctx: &mut ModelContext<Self>) {
        match self {
            Self::Local(local) => local.update(ctx, |local, ctx| {
                local.git_push(branch, ctx);
            }),
            Self::Remote(remote) => remote.update(ctx, |remote, ctx| {
                remote.git_push(branch, ctx);
            }),
        }
    }

    /// Creates a PR for the current branch.
    ///
    /// When `autogenerate_content` is set, the PR title/body are AI-generated,
    /// otherwise fallback to `gh pr create --fill`.
    pub(crate) fn create_pr(
        &self,
        branch: String,
        autogenerate_content: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        match self {
            Self::Local(local) => local.update(ctx, |local, ctx| {
                local.create_pr(branch, autogenerate_content, ctx);
            }),
            Self::Remote(remote) => remote.update(ctx, |remote, ctx| {
                remote.create_pr(branch, autogenerate_content, ctx);
            }),
        }
    }

    /// Fetches the committed branch files (`merge_base(HEAD, main)..HEAD`) for
    /// the Create PR dialog's Changes box. Both backends deliver the result via
    /// `DiffStateModelEvent::BranchCommittedFilesReceived`: the local model
    /// computes them from committed history off-thread; the remote model issues
    /// the `GitGetCommittedBranchFiles` RPC. Committed-only, so uncommitted and
    /// untracked changes are excluded — matching what the PR will contain.
    pub(crate) fn fetch_committed_branch_files(&self, ctx: &mut ModelContext<Self>) {
        match self {
            Self::Local(local) => local.update(ctx, |local, ctx| {
                local.fetch_committed_branch_files(ctx);
            }),
            Self::Remote(model) => model.update(ctx, |model, ctx| {
                model.fetch_committed_branch_files(ctx);
            }),
        }
    }

    #[cfg(feature = "local_fs")]
    pub(crate) fn stop_active_watcher(&self, ctx: &mut ModelContext<Self>) {
        match self {
            Self::Local(local) => {
                local.update(ctx, |local, ctx| {
                    local.stop_active_watcher(ctx);
                });
            }
            Self::Remote(remote) => {
                remote.update(ctx, |remote, ctx| {
                    remote.unsubscribe(ctx);
                });
            }
        }
    }
}

#[cfg(test)]
impl DiffStateModel {
    /// Test-only constructor that creates a local-backend model without a
    /// repository. All existing tests exercise local behavior; add a
    /// `new_for_test_remote` variant when remote-backend tests are needed.
    pub fn new_for_test(ctx: &mut ModelContext<Self>) -> Self {
        let local = ctx.add_model(LocalDiffStateModel::new_for_test);
        ctx.subscribe_to_model(&local, Self::forward_event);
        Self::Local(local)
    }
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod wrapper_tests;

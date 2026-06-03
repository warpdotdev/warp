//! Typed errors for diff state operations.
//!
//! [`DiffStateError`] is the single error type used across every diff-state operation:
//! - per-file invalidation
//! - full diff load
//! - metadata load
//! - remote-daemon snapshot/error responses
//! The same pool of git / filesystem failures can surface in any of these operations,
//! so a single classifier keeps the code DRY and ensures every site reports failures the same way.
//!
//! A [`DiffStateError`] pairs a sanitized [`DiffStateErrorKind`] with the raw
//! underlying error, but only the sanitized half is ever emitted off-device:
//! - [`std::fmt::Display`] renders only the sanitized `kind`, so passing this
//!   through [`warp_core::report_error!`] or code-review telemetry keeps logs,
//!   Sentry, and analytics free of repo paths, refs, command output, or
//!   secrets. The raw cause is never exposed via `Display` or `source`.
//!
//! For [`DiffStateErrorKind::Unknown`] the raw cause is additionally consulted
//! via [`AnyhowErrorExt::is_actionable`] so registered non-actionable causes
//! (transient I/O, network, etc.) auto-demote it to a warning instead of a
//! Sentry capture.
//!
//! Use the operation tag [`super::DiffOperation`] alongside this error in telemetry to distinguish where a given failure originated.

use warp_core::errors::{AnyhowErrorExt, ErrorExt};
use warp_core::sync_queue::IsTransientError;

/// Sanitized classification of a [`DiffStateError`]. Every variant has a
/// fixed, PII-free [`std::fmt::Display`] string that is safe to send to logs
/// and Sentry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub(crate) enum DiffStateErrorKind {
    // ── Git environment / repository state ──────────────────────────────
    #[error("git rejected repository ownership")]
    GitRejectedRepositoryOwnership,
    #[error("git is unavailable")]
    GitUnavailable,
    #[error("git lfs is unavailable")]
    GitLfsUnavailable,
    #[error("xcode license is not accepted")]
    XcodeLicenseNotAccepted,
    #[error("invalid empty pathspec")]
    InvalidEmptyPathspec,
    #[error("path is outside repository")]
    PathOutsideRepository,
    #[error("path is not a git repository")]
    NotGitRepository,
    #[error("repository is not a work tree")]
    NotWorkTree,
    #[error("repository resource is not accessible")]
    RepositoryPathNotAccessible,
    #[error("path is not valid UTF-8")]
    NonUtf8Path,
    #[error("git revision is unavailable")]
    GitRevisionUnavailable,
    #[error("git head tree is invalid")]
    GitHeadTreeInvalid,
    #[error("git status output is invalid")]
    InvalidGitStatusOutput,
    #[error("repository path is invalid")]
    RepositoryPathInvalid,

    // ── Remote daemon application-level outcomes ────────────────────────
    /// The remote daemon reported `DiffState::Loaded` but no `GitDiffData`
    /// accompanied it. Only constructed by `RemoteDiffStateModel`.
    #[error("server returned empty diff data")]
    EmptyDiffData,

    // ── Unclassified ────────────────────────────────────────────────────
    /// Unrecognized error. Add a dedicated variant once a new pattern is
    /// identified from the raw text recorded in telemetry.
    #[error("unknown diff state error")]
    Unknown,
}

impl DiffStateErrorKind {
    fn classify(message: &str) -> Option<Self> {
        if message.contains("detected dubious ownership in repository") {
            Some(Self::GitRejectedRepositoryOwnership)
        } else if message.contains("No such file or directory")
            || message.contains("program not found")
            || message.contains("No developer tools were found")
        {
            Some(Self::GitUnavailable)
        } else if message.contains("git-lfs: command not found") {
            Some(Self::GitLfsUnavailable)
        } else if message.contains("Xcode license agreements") {
            Some(Self::XcodeLicenseNotAccepted)
        } else if message.contains("empty string is not a valid pathspec") {
            Some(Self::InvalidEmptyPathspec)
        } else if message.contains("outside repository") {
            Some(Self::PathOutsideRepository)
        } else if message.contains("not a git repository") {
            Some(Self::NotGitRepository)
        } else if message.contains("this operation must be run in a work tree") {
            Some(Self::NotWorkTree)
        } else if message.contains("Operation not permitted")
            || message.contains("Permission denied")
        {
            Some(Self::RepositoryPathNotAccessible)
        } else if message.contains("non-UTF-8 path") {
            Some(Self::NonUtf8Path)
        } else if message.contains("bad revision") || message.contains("unknown revision") {
            Some(Self::GitRevisionUnavailable)
        } else if message.contains("bad tree object HEAD") {
            Some(Self::GitHeadTreeInvalid)
        } else if message.contains("os error 267") {
            Some(Self::RepositoryPathInvalid)
        } else if message.contains("Invalid status code") {
            Some(Self::InvalidGitStatusOutput)
        } else {
            None
        }
    }
}

/// A diff-state failure: a sanitized [`DiffStateErrorKind`] paired with the
/// raw underlying error. See the module docs for how the two halves are
/// routed to telemetry vs. logs / Sentry.
#[derive(Debug, thiserror::Error)]
#[error("{kind}")]
pub(crate) struct DiffStateError {
    kind: DiffStateErrorKind,
    /// Raw underlying error. Consulted only for [`DiffStateErrorKind::Unknown`]
    /// actionability and never exposed via `Display`, `source`, or telemetry,
    /// so logs, Sentry, and analytics only ever see the sanitized `kind`.
    cause: anyhow::Error,
}

impl DiffStateError {
    /// Build a `DiffStateError` from a plain error message string. Used when
    /// the source error has already been flattened to a `String` (e.g. by
    /// `DiffsWithBaseContent::changes`, or by the remote daemon over the
    /// wire).
    pub(crate) fn from_message(message: &str) -> Self {
        Self {
            kind: DiffStateErrorKind::classify(message).unwrap_or(DiffStateErrorKind::Unknown),
            cause: anyhow::anyhow!("{message}"),
        }
    }

    /// Build the [`DiffStateErrorKind::EmptyDiffData`] error, reported when the
    /// remote daemon claims `DiffState::Loaded` but sends no diff data.
    pub(crate) fn empty_diff_data() -> Self {
        let kind = DiffStateErrorKind::EmptyDiffData;
        let cause = anyhow::anyhow!("{kind}");
        Self { kind, cause }
    }
}

impl From<anyhow::Error> for DiffStateError {
    fn from(cause: anyhow::Error) -> Self {
        let kind = DiffStateErrorKind::classify(&format!("{cause:#}"))
            .unwrap_or(DiffStateErrorKind::Unknown);
        Self { kind, cause }
    }
}

impl ErrorExt for DiffStateError {
    fn is_actionable(&self) -> bool {
        match self.kind {
            // Caller / engineering bugs — surface to Sentry at error level.
            DiffStateErrorKind::InvalidEmptyPathspec
            | DiffStateErrorKind::InvalidGitStatusOutput
            | DiffStateErrorKind::EmptyDiffData => true,
            // Unknown errors defer to the anyhow chain so registered
            // transient/non-actionable causes (network, transient I/O, etc.)
            // log at warn level instead of paging us via Sentry.
            DiffStateErrorKind::Unknown => self.cause.is_actionable(),
            // User environment failures — not our bug; log as warning.
            DiffStateErrorKind::GitRejectedRepositoryOwnership
            | DiffStateErrorKind::GitUnavailable
            | DiffStateErrorKind::GitLfsUnavailable
            | DiffStateErrorKind::XcodeLicenseNotAccepted
            | DiffStateErrorKind::PathOutsideRepository
            | DiffStateErrorKind::NotGitRepository
            | DiffStateErrorKind::NotWorkTree
            | DiffStateErrorKind::RepositoryPathNotAccessible
            | DiffStateErrorKind::NonUtf8Path
            | DiffStateErrorKind::GitRevisionUnavailable
            | DiffStateErrorKind::GitHeadTreeInvalid
            | DiffStateErrorKind::RepositoryPathInvalid => false,
        }
    }
}
warp_core::errors::register_error!(DiffStateError);

impl IsTransientError for DiffStateError {
    fn is_transient(&self) -> bool {
        match self.kind {
            // Repo / filesystem state can briefly churn while the queue is
            // processing invalidations, so these are worth the sync queue's
            // short retry budget.
            DiffStateErrorKind::RepositoryPathNotAccessible
            | DiffStateErrorKind::GitRevisionUnavailable
            | DiffStateErrorKind::GitHeadTreeInvalid
            | DiffStateErrorKind::EmptyDiffData
            | DiffStateErrorKind::Unknown => true,
            // Caller bugs, invalid inputs, missing tools, and user-actionable
            // environment setup issues won't resolve by retrying the same
            // operation a few seconds later.
            DiffStateErrorKind::GitRejectedRepositoryOwnership
            | DiffStateErrorKind::GitUnavailable
            | DiffStateErrorKind::GitLfsUnavailable
            | DiffStateErrorKind::XcodeLicenseNotAccepted
            | DiffStateErrorKind::InvalidEmptyPathspec
            | DiffStateErrorKind::PathOutsideRepository
            | DiffStateErrorKind::NotGitRepository
            | DiffStateErrorKind::NotWorkTree
            | DiffStateErrorKind::NonUtf8Path
            | DiffStateErrorKind::RepositoryPathInvalid
            | DiffStateErrorKind::InvalidGitStatusOutput => false,
        }
    }
}

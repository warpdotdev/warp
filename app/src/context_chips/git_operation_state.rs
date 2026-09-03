//! Detection of an in-progress Git operation (rebase, merge, cherry-pick,
//! revert, `git am`, bisect) and the state-transition actions Warp offers for
//! it, mirroring the states surfaced by tools like Starship's `git_state`
//! module.
//!
//! [`GitOperationKind::detect`] inspects an already-resolved `.git` directory
//! rather than assuming `<worktree>/.git` is that directory: for a linked
//! worktree, `.git` in the working tree is a *file* pointing at the real git
//! dir under the main repository's `.git/worktrees/<name>`, and the sentinel
//! files this module looks for (e.g. `MERGE_HEAD`) live there instead.
//! Callers must resolve the git dir first, e.g. via
//! `repo_metadata::Repository::git_dir`, which already returns the correct
//! per-worktree directory. This is the detection backing the
//! `GitRepoStatusModel`'s `git_operation_state` metadata field
//! (`code_review/git_repo_model`), and by extension the `GitOperationState`
//! prompt chip; it never shells out.

use std::path::Path;

/// The specific Git operation currently in progress in a repository.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitOperationKind {
    /// An interactive rebase (`rebase-merge` directory present).
    RebaseInteractive,
    /// A non-interactive, patch-based rebase (`rebase-apply` directory
    /// present, without the `applying` marker file).
    RebaseApply,
    /// A `git am` patch application (`rebase-apply` directory present, with
    /// the `applying` marker file).
    Am,
    Merge,
    CherryPick,
    Revert,
    Bisect,
}

impl GitOperationKind {
    /// Detects the in-progress Git operation, if any, from an already-resolved
    /// `.git` directory. See the module docs for why this must be resolved
    /// (e.g. via `git rev-parse --git-dir`) rather than assumed.
    pub fn detect(git_dir: &Path) -> Option<Self> {
        if git_dir.join("rebase-merge").is_dir() {
            Some(Self::RebaseInteractive)
        } else if git_dir.join("rebase-apply").is_dir() {
            if git_dir.join("rebase-apply").join("applying").is_file() {
                Some(Self::Am)
            } else {
                Some(Self::RebaseApply)
            }
        } else if git_dir.join("MERGE_HEAD").is_file() {
            Some(Self::Merge)
        } else if git_dir.join("CHERRY_PICK_HEAD").is_file() {
            Some(Self::CherryPick)
        } else if git_dir.join("REVERT_HEAD").is_file() {
            Some(Self::Revert)
        } else if git_dir.join("BISECT_LOG").is_file() {
            Some(Self::Bisect)
        } else {
            None
        }
    }

    /// Parses the stable wire/chip-value token produced by [`Self::token`].
    /// Used to decode both the `GitStatusMetadata` proto field and the
    /// `GitOperationState` chip's `ChipValue::Text`.
    pub fn from_token(token: &str) -> Option<Self> {
        match token.trim() {
            "rebase-interactive" => Some(Self::RebaseInteractive),
            "rebase-apply" => Some(Self::RebaseApply),
            "am" => Some(Self::Am),
            "merge" => Some(Self::Merge),
            "cherry-pick" => Some(Self::CherryPick),
            "revert" => Some(Self::Revert),
            "bisect" => Some(Self::Bisect),
            _ => None,
        }
    }

    /// The stable token identifying this state, inverse of [`Self::from_token`].
    pub fn token(self) -> &'static str {
        match self {
            Self::RebaseInteractive => "rebase-interactive",
            Self::RebaseApply => "rebase-apply",
            Self::Am => "am",
            Self::Merge => "merge",
            Self::CherryPick => "cherry-pick",
            Self::Revert => "revert",
            Self::Bisect => "bisect",
        }
    }

    /// The label shown on the prompt chip.
    pub fn display_label(self) -> &'static str {
        match self {
            Self::RebaseInteractive | Self::RebaseApply => "REBASING",
            Self::Am => "AM",
            Self::Merge => "MERGING",
            Self::CherryPick => "CHERRY-PICKING",
            Self::Revert => "REVERTING",
            Self::Bisect => "BISECTING",
        }
    }

    /// The actions Warp offers for this state, in menu display order.
    pub fn available_actions(self) -> &'static [GitOperationAction] {
        use GitOperationAction::*;
        match self {
            Self::RebaseInteractive | Self::RebaseApply => {
                &[RebaseContinue, RebaseSkip, RebaseAbort]
            }
            Self::Am => &[AmContinue, AmSkip, AmAbort],
            Self::Merge => &[MergeContinue, MergeAbort],
            Self::CherryPick => &[CherryPickContinue, CherryPickSkip, CherryPickAbort],
            Self::Revert => &[RevertContinue, RevertSkip, RevertAbort],
            Self::Bisect => &[BisectGood, BisectBad, BisectSkip, BisectReset],
        }
    }
}

/// A state-transition action offered for a [`GitOperationKind`]. Each variant
/// maps to a fixed, static `git` argv (see [`Self::git_args`]) that is never
/// built from repository- or user-derived text, so it can be run without any
/// shell-quoting or injection concerns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitOperationAction {
    RebaseContinue,
    RebaseSkip,
    RebaseAbort,
    AmContinue,
    AmSkip,
    AmAbort,
    MergeContinue,
    MergeAbort,
    CherryPickContinue,
    CherryPickSkip,
    CherryPickAbort,
    RevertContinue,
    RevertSkip,
    RevertAbort,
    BisectGood,
    BisectBad,
    BisectSkip,
    BisectReset,
}

impl GitOperationAction {
    /// The literal `git` argv for this action.
    pub fn git_args(self) -> &'static [&'static str] {
        match self {
            Self::RebaseContinue => &["rebase", "--continue"],
            Self::RebaseSkip => &["rebase", "--skip"],
            Self::RebaseAbort => &["rebase", "--abort"],
            Self::AmContinue => &["am", "--continue"],
            Self::AmSkip => &["am", "--skip"],
            Self::AmAbort => &["am", "--abort"],
            Self::MergeContinue => &["merge", "--continue"],
            Self::MergeAbort => &["merge", "--abort"],
            Self::CherryPickContinue => &["cherry-pick", "--continue"],
            Self::CherryPickSkip => &["cherry-pick", "--skip"],
            Self::CherryPickAbort => &["cherry-pick", "--abort"],
            Self::RevertContinue => &["revert", "--continue"],
            Self::RevertSkip => &["revert", "--skip"],
            Self::RevertAbort => &["revert", "--abort"],
            Self::BisectGood => &["bisect", "good"],
            Self::BisectBad => &["bisect", "bad"],
            Self::BisectSkip => &["bisect", "skip"],
            Self::BisectReset => &["bisect", "reset"],
        }
    }

    /// The label shown for this action in the chip's menu.
    pub fn label(self) -> &'static str {
        match self {
            Self::RebaseContinue
            | Self::AmContinue
            | Self::MergeContinue
            | Self::CherryPickContinue
            | Self::RevertContinue => "Continue",
            Self::RebaseSkip
            | Self::AmSkip
            | Self::CherryPickSkip
            | Self::RevertSkip
            | Self::BisectSkip => "Skip",
            Self::RebaseAbort
            | Self::AmAbort
            | Self::MergeAbort
            | Self::CherryPickAbort
            | Self::RevertAbort => "Abort",
            Self::BisectGood => "Good",
            Self::BisectBad => "Bad",
            Self::BisectReset => "Reset",
        }
    }
}

#[cfg(test)]
#[path = "git_operation_state_tests.rs"]
mod tests;

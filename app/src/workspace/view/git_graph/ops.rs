//! Write-operation layer for the Git Graph: the set of mutating git actions
//! reachable from the right-click context menus (checkout, branch/tag
//! create/delete, merge, rebase, reset, cherry-pick, revert, push/pull,
//! archive).
//!
//! Mirrors [`super::data`]'s split of pure logic from IO: [`GitWriteOp::args`]
//! and [`GitWriteOp::confirm_message`] are pure (unit-tested in `ops_tests.rs`),
//! while [`run_write_op`] is a thin async wrapper over
//! [`warp_util::git::run_git_command`]. Every mutating action is gated at the UI
//! layer by [`warp_features::FeatureFlag::GitGraphWrite`].

#[cfg(not(target_family = "wasm"))]
use std::path::Path;
use std::path::PathBuf;

#[cfg(not(target_family = "wasm"))]
use anyhow::Result;

/// `git reset` mode (chosen from the "Reset current branch to this Commit"
/// submenu). Each moves the current branch ref to the target commit; they
/// differ in what happens to the index and working tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResetMode {
    /// Keep index and working tree (only move the branch ref).
    Soft,
    /// Reset the index, keep the working tree (git's default).
    Mixed,
    /// Reset index and working tree — discards uncommitted changes.
    Hard,
}

impl ResetMode {
    fn flag(self) -> &'static str {
        match self {
            ResetMode::Soft => "--soft",
            ResetMode::Mixed => "--mixed",
            ResetMode::Hard => "--hard",
        }
    }
}

/// Output format for "Create Archive", derived from the file extension the user
/// picks in the save dialog. `git archive` accepts both `zip` and `tar.gz`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ArchiveFormat {
    Zip,
    TarGz,
}

impl ArchiveFormat {
    fn name(self) -> &'static str {
        match self {
            ArchiveFormat::Zip => "zip",
            ArchiveFormat::TarGz => "tar.gz",
        }
    }
}

/// Infers the archive format from the chosen output path's extension; defaults
/// to [`ArchiveFormat::TarGz`] when the extension is neither `.zip` nor a
/// gzip-tar variant (`.tar.gz` / `.tgz`).
pub(crate) fn archive_format_from_path(path: &std::path::Path) -> ArchiveFormat {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    if name.ends_with(".zip") {
        ArchiveFormat::Zip
    } else {
        ArchiveFormat::TarGz
    }
}

/// Splits a remote-branch display name (`origin/feature`, `origin/feat/x`) into
/// its remote and branch parts on the first `/`. A name without a `/` is
/// treated as having no remote prefix (the whole string is the branch).
pub(crate) fn split_remote_ref(display_name: &str) -> (String, String) {
    match display_name.split_once('/') {
        Some((remote, branch)) => (remote.to_string(), branch.to_string()),
        None => (String::new(), display_name.to_string()),
    }
}

/// A fully-specified, ready-to-run mutating git operation. The UI builds one of
/// these once it has all the inputs it needs (text from a prompt dialog, a mode
/// from the reset submenu, a path from the save dialog), then runs it through
/// [`run_write_op`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum GitWriteOp {
    /// `git tag <name> <hash>` (lightweight) or `git tag -a <name> -m <msg>
    /// <hash>` (annotated, when a message is given).
    AddTag {
        hash: String,
        name: String,
        message: Option<String>,
    },
    /// `git branch <name> <hash>`.
    CreateBranch { hash: String, name: String },
    /// `git checkout <hash>` (detached HEAD).
    CheckoutCommit { hash: String },
    /// `git cherry-pick <hash>` onto the current branch.
    CherryPick { hash: String },
    /// `git revert --no-edit <hash>` on the current branch.
    Revert { hash: String },
    /// Drop a commit from the current branch's history via
    /// `git rebase --onto <hash>^ <hash>` (rewrites history).
    DropCommit { hash: String },
    /// `git merge <rev>` into the current branch (`rev` is a commit hash or a
    /// remote-branch ref like `origin/feature`).
    Merge { rev: String },
    /// `git rebase <hash>` — rebase the current branch onto the commit.
    Rebase { hash: String },
    /// `git rebase <branch>` — rebase the current branch onto another branch.
    RebaseOntoBranch { branch: String },
    /// `git reset --soft|--mixed|--hard <hash>` — move the current branch ref.
    Reset { hash: String, mode: ResetMode },
    /// `git checkout <branch>` — check out a (remote) branch by its short name,
    /// letting git set up tracking.
    CheckoutBranch { branch: String },
    /// `git push <remote> --delete <branch>`.
    DeleteRemoteBranch { remote: String, branch: String },
    /// `git pull <remote> <branch>` into the current branch.
    Pull { remote: String, branch: String },
    /// `git branch -m <old> <new>`.
    RenameBranch { old: String, new: String },
    /// `git branch -d <name>` — delete a (merged) local branch.
    DeleteLocalBranch { name: String },
    /// `git push <remote> <branch>`.
    PushBranch { remote: String, branch: String },
    /// `git tag -d <name>`.
    DeleteTag { name: String },
    /// `git push <remote> <tag>`.
    PushTag { remote: String, name: String },
    /// `git archive --format <fmt> -o <output> <rev>`.
    Archive {
        rev: String,
        output: PathBuf,
        format: ArchiveFormat,
    },
}

impl GitWriteOp {
    /// The exact `git` argument vector for this operation. Pure (no IO) so it can
    /// be unit-tested; `run_write_op` borrows these as `&str`s.
    pub(crate) fn args(&self) -> Vec<String> {
        match self {
            GitWriteOp::AddTag {
                hash,
                name,
                message: None,
            } => vec!["tag".into(), name.clone(), hash.clone()],
            GitWriteOp::AddTag {
                hash,
                name,
                message: Some(msg),
            } => vec![
                "tag".into(),
                "-a".into(),
                name.clone(),
                "-m".into(),
                msg.clone(),
                hash.clone(),
            ],
            GitWriteOp::CreateBranch { hash, name } => {
                vec!["branch".into(), name.clone(), hash.clone()]
            }
            GitWriteOp::CheckoutCommit { hash } => vec!["checkout".into(), hash.clone()],
            GitWriteOp::CherryPick { hash } => vec!["cherry-pick".into(), hash.clone()],
            GitWriteOp::Revert { hash } => {
                vec!["revert".into(), "--no-edit".into(), hash.clone()]
            }
            GitWriteOp::DropCommit { hash } => vec![
                "rebase".into(),
                "--onto".into(),
                format!("{hash}^"),
                hash.clone(),
            ],
            GitWriteOp::Merge { rev } => vec!["merge".into(), rev.clone()],
            GitWriteOp::Rebase { hash } => vec!["rebase".into(), hash.clone()],
            GitWriteOp::RebaseOntoBranch { branch } => vec!["rebase".into(), branch.clone()],
            GitWriteOp::Reset { hash, mode } => {
                vec!["reset".into(), mode.flag().into(), hash.clone()]
            }
            GitWriteOp::CheckoutBranch { branch } => vec!["checkout".into(), branch.clone()],
            GitWriteOp::DeleteRemoteBranch { remote, branch } => vec![
                "push".into(),
                remote.clone(),
                "--delete".into(),
                branch.clone(),
            ],
            GitWriteOp::Pull { remote, branch } => {
                vec!["pull".into(), remote.clone(), branch.clone()]
            }
            GitWriteOp::RenameBranch { old, new } => {
                vec!["branch".into(), "-m".into(), old.clone(), new.clone()]
            }
            GitWriteOp::DeleteLocalBranch { name } => {
                vec!["branch".into(), "-d".into(), name.clone()]
            }
            GitWriteOp::PushBranch { remote, branch } => {
                vec!["push".into(), remote.clone(), branch.clone()]
            }
            GitWriteOp::DeleteTag { name } => vec!["tag".into(), "-d".into(), name.clone()],
            GitWriteOp::PushTag { remote, name } => {
                vec!["push".into(), remote.clone(), name.clone()]
            }
            GitWriteOp::Archive {
                rev,
                output,
                format,
            } => vec![
                "archive".into(),
                "--format".into(),
                format.name().into(),
                "-o".into(),
                output.to_string_lossy().into_owned(),
                rev.clone(),
            ],
        }
    }

    /// The confirmation prompt shown before running this operation. Operations
    /// reached through a text-input dialog (add tag / create branch / rename)
    /// return `None` — their dialog already gates them — as does the OS save
    /// dialog for archives; everything else returns a tailored yes/no message so
    /// the user always sees what is about to happen (especially history-rewriting
    /// or remote-mutating actions).
    pub(crate) fn confirm_message(&self) -> Option<String> {
        let short = |h: &str| h.chars().take(7).collect::<String>();
        match self {
            GitWriteOp::AddTag { .. }
            | GitWriteOp::CreateBranch { .. }
            | GitWriteOp::RenameBranch { .. }
            | GitWriteOp::Archive { .. } => None,
            GitWriteOp::CheckoutCommit { hash } => Some(format!(
                "Check out commit {} as a detached HEAD?",
                short(hash)
            )),
            GitWriteOp::CherryPick { hash } => Some(format!(
                "Cherry-pick commit {} onto the current branch?",
                short(hash)
            )),
            GitWriteOp::Revert { hash } => Some(format!(
                "Revert commit {} on the current branch?",
                short(hash)
            )),
            GitWriteOp::DropCommit { hash } => Some(format!(
                "Drop commit {} from the current branch? This rewrites history.",
                short(hash)
            )),
            GitWriteOp::Merge { rev } => Some(format!("Merge {} into the current branch?", rev)),
            GitWriteOp::Rebase { hash } => Some(format!(
                "Rebase the current branch onto {}? This rewrites history.",
                short(hash)
            )),
            GitWriteOp::RebaseOntoBranch { branch } => Some(format!(
                "Rebase the current branch onto \"{branch}\"? This rewrites history."
            )),
            GitWriteOp::DeleteLocalBranch { name } => {
                Some(format!("Delete branch \"{name}\"? This cannot be undone."))
            }
            GitWriteOp::Reset { hash, mode } => Some(match mode {
                ResetMode::Hard => format!(
                    "Hard-reset the current branch to {}? Uncommitted changes will be lost.",
                    short(hash)
                ),
                ResetMode::Soft => {
                    format!("Soft-reset the current branch to {}?", short(hash))
                }
                ResetMode::Mixed => {
                    format!("Reset the current branch to {}?", short(hash))
                }
            }),
            GitWriteOp::CheckoutBranch { branch } => {
                Some(format!("Check out branch \"{branch}\"?"))
            }
            GitWriteOp::DeleteRemoteBranch { remote, branch } => Some(format!(
                "Delete branch \"{branch}\" from remote \"{remote}\"? This cannot be undone."
            )),
            GitWriteOp::Pull { remote, branch } => {
                Some(format!("Pull {remote}/{branch} into the current branch?"))
            }
            GitWriteOp::PushBranch { remote, branch } => {
                Some(format!("Push branch \"{branch}\" to \"{remote}\"?"))
            }
            GitWriteOp::DeleteTag { name } => {
                Some(format!("Delete tag \"{name}\"? This cannot be undone."))
            }
            GitWriteOp::PushTag { remote, name } => {
                Some(format!("Push tag \"{name}\" to \"{remote}\"?"))
            }
        }
    }
}

/// Runs a mutating git operation in `repo_root`. Thin async wrapper over
/// [`warp_util::git::run_git_command`]; the success/failure is surfaced by the
/// caller (a reload on success, an error banner on failure).
#[cfg(not(target_family = "wasm"))]
pub(crate) async fn run_write_op(repo_root: &Path, op: &GitWriteOp) -> Result<()> {
    let args = op.args();
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    warp_util::git::run_git_command(repo_root, &arg_refs).await?;
    Ok(())
}

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;

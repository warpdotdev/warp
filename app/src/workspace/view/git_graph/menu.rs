//! Builds the right-click context menus for the Git Graph. Pure: each builder
//! turns a [`MenuKind`] (plus the target commit, the current branch, and whether
//! write operations are enabled) into a list of [`MenuItem`]s. The view owns the
//! [`Menu`] view and the dialog/async plumbing the items dispatch into.
//!
//! [`Menu`]: crate::menu::Menu

use super::data::CommitNode;
use super::ops::{split_remote_ref, GitWriteOp};
use super::view::GitGraphAction;
use crate::menu::{MenuItem, MenuItemFields};

/// What was right-clicked, and the row it lives on. `index` is the commit row
/// the menu was opened from (the row itself for a commit, or the row a ref badge
/// sits on); it lets ref menus resolve the underlying commit (e.g. tag "View
/// Details" selects that row).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MenuKind {
    Commit {
        index: usize,
    },
    /// The short-hash text to the left of a commit's subject. Opens a focused
    /// menu with just "Copy Short Hash to Clipboard" (the 8-char hash), so a
    /// right-click landing on the hash copies exactly what's shown rather than
    /// the full 40-char hash offered by the commit menu.
    ShortHash {
        index: usize,
    },
    Tag {
        index: usize,
        name: String,
    },
    /// `name` is the remote-branch display name, e.g. `origin/feature`.
    RemoteBranch {
        index: usize,
        name: String,
    },
    /// A local branch. `is_current` is true for the HEAD badge (the checked-out
    /// branch), which omits operations that don't apply to the current branch
    /// (checkout-self / delete / merge-into-current / rebase-onto-self).
    LocalBranch {
        index: usize,
        name: String,
        is_current: bool,
    },
    /// A stash entry badge (`stash@{n}`). `index` is the stash's row; `name` is the
    /// selector (`stash@{0}`) used by stash operations and the "copy name" item.
    Stash {
        index: usize,
        name: String,
    },
    /// The synthetic "uncommitted changes" row. Its menu offers working-tree
    /// operations (clean untracked files) and has no anchor commit.
    Uncommitted,
}

impl MenuKind {
    pub(crate) fn index(&self) -> usize {
        match self {
            MenuKind::Commit { index }
            | MenuKind::ShortHash { index }
            | MenuKind::Tag { index, .. }
            | MenuKind::RemoteBranch { index, .. }
            | MenuKind::LocalBranch { index, .. }
            | MenuKind::Stash { index, .. } => *index,
            // The uncommitted row sits above the newest commit; its menu ignores
            // the anchor commit, so index 0 (newest) is a harmless placeholder.
            MenuKind::Uncommitted => 0,
        }
    }
}

/// A text-input dialog request: the user must type a name before the operation
/// can be built. Carries the context needed to construct the final
/// [`GitWriteOp`] once the text is known.
///
/// The Add-tag flow is intentionally NOT here: its dialog also carries a
/// "Push to remote" checkbox, so it lives on its own
/// [`super::view::DialogState::AddTag`] variant rather than the shared single-line
/// input prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PromptKind {
    CreateBranch {
        hash: String,
    },
    RenameBranch {
        old: String,
    },
    /// Create a branch from a stash; `selector` is the `stash@{n}` to apply.
    StashBranch {
        selector: String,
    },
}

impl PromptKind {
    /// Title shown at the top of the input dialog.
    pub(crate) fn title(&self) -> &'static str {
        match self {
            PromptKind::CreateBranch { .. } => "Create branch",
            PromptKind::RenameBranch { .. } => "Rename branch",
            PromptKind::StashBranch { .. } => "Create branch from stash",
        }
    }

    /// The pre-filled text (e.g. the existing branch name, for rename).
    pub(crate) fn initial_text(&self) -> String {
        match self {
            PromptKind::CreateBranch { .. } | PromptKind::StashBranch { .. } => String::new(),
            PromptKind::RenameBranch { old } => old.clone(),
        }
    }

    /// Builds the operation once the user has entered `text` (already trimmed and
    /// known non-empty by the caller).
    pub(crate) fn into_op(self, text: String) -> GitWriteOp {
        match self {
            PromptKind::CreateBranch { hash } => GitWriteOp::CreateBranch { hash, name: text },
            PromptKind::RenameBranch { old } => GitWriteOp::RenameBranch { old, new: text },
            PromptKind::StashBranch { selector } => GitWriteOp::StashBranch {
                selector,
                name: text,
            },
        }
    }
}

/// Default remote used by "Push Branch" / "Push Tag" (the branch's configured
/// upstream remote is not resolved; `origin` is the overwhelmingly common case,
/// and a wrong remote surfaces as a normal error in the banner).
pub(crate) const DEFAULT_PUSH_REMOTE: &str = "origin";

fn item(label: &str, action: GitGraphAction) -> MenuItem<GitGraphAction> {
    MenuItemFields::new(label)
        .with_on_select_action(action)
        .into_item()
}

/// Suggests a save-dialog filename for an archive of `rev`, sanitizing path
/// separators that are invalid in a file name (`origin/x` → `origin-x.zip`).
fn archive_suggested_name(rev: &str) -> String {
    format!("{}.zip", rev.replace('/', "-"))
}

/// Joins non-empty groups with a [`MenuItem::Separator`] between them, so a
/// section that is entirely gated away (write disabled / detached HEAD) doesn't
/// leave a dangling separator.
fn join_groups(groups: Vec<Vec<MenuItem<GitGraphAction>>>) -> Vec<MenuItem<GitGraphAction>> {
    let mut out: Vec<MenuItem<GitGraphAction>> = Vec::new();
    for group in groups.into_iter().filter(|g| !g.is_empty()) {
        if !out.is_empty() {
            out.push(MenuItem::Separator);
        }
        out.extend(group);
    }
    out
}

/// Builds the menu items for a right-click target.
///
/// - `commit` is the commit at `kind.index()` (the menu's anchor commit).
/// - `write_enabled` reflects [`FeatureFlag::GitGraphWrite`]; when false only the
///   read-only items (copy / view details / unselect) are shown.
///
/// Operations phrased "…current branch" are always offered when writing is
/// enabled; on a detached HEAD git applies them to the detached HEAD or fails
/// (surfaced in the error banner), rather than the menu silently hiding them.
///
/// [`FeatureFlag::GitGraphWrite`]: warp_features::FeatureFlag::GitGraphWrite
pub(crate) fn build_menu(
    kind: &MenuKind,
    commit: &CommitNode,
    write_enabled: bool,
) -> Vec<MenuItem<GitGraphAction>> {
    match kind {
        MenuKind::Commit { .. } => build_commit_menu(commit, write_enabled),
        MenuKind::ShortHash { .. } => build_short_hash_menu(commit),
        MenuKind::Tag { index, name } => build_tag_menu(*index, name, write_enabled),
        MenuKind::RemoteBranch { name, .. } => build_remote_branch_menu(name, write_enabled),
        MenuKind::LocalBranch {
            name, is_current, ..
        } => build_local_branch_menu(name, *is_current, write_enabled),
        MenuKind::Stash { name, .. } => build_stash_menu(name, commit, write_enabled),
        MenuKind::Uncommitted => build_uncommitted_menu(write_enabled),
    }
}

/// The menu for a stash badge (`name` is the `stash@{n}` selector). When writing
/// is enabled it offers apply / create-branch / pop / drop; the read-only copy
/// items (selector + full commit hash) are always present.
fn build_stash_menu(
    name: &str,
    commit: &CommitNode,
    write_enabled: bool,
) -> Vec<MenuItem<GitGraphAction>> {
    let mut ops = vec![];
    if write_enabled {
        ops.push(item(
            "Apply Stash…",
            GitGraphAction::BeginWriteOp(GitWriteOp::StashApply {
                selector: name.to_string(),
            }),
        ));
        ops.push(item(
            "Create Branch from Stash…",
            GitGraphAction::PromptInput(PromptKind::StashBranch {
                selector: name.to_string(),
            }),
        ));
        ops.push(item(
            "Pop Stash…",
            GitGraphAction::BeginWriteOp(GitWriteOp::StashPop {
                selector: name.to_string(),
            }),
        ));
        ops.push(item(
            "Drop Stash…",
            GitGraphAction::BeginWriteOp(GitWriteOp::StashDrop {
                selector: name.to_string(),
            }),
        ));
    }

    let copy = vec![
        item(
            "Copy Stash Name to Clipboard",
            GitGraphAction::CopyToClipboard(name.to_string()),
        ),
        item(
            "Copy Stash Hash to Clipboard",
            GitGraphAction::CopyToClipboard(commit.hash.clone()),
        ),
    ];

    join_groups(vec![ops, copy])
}

/// The menu for the synthetic "uncommitted changes" row: stash, working-tree
/// reset, and cleanup. Empty when writing is disabled (the view then opens no
/// menu), since it offers no read-only items.
///
/// - "Stash uncommitted changes…" opens the stash dialog (a message input plus an
///   Include-untracked checkbox, checked by default).
/// - "Reset uncommitted changes…" opens a Mixed/Hard picker (a soft reset to HEAD
///   is a no-op, so it's omitted).
/// - "Clean untracked files…" seeds [`GitWriteOp::CleanUntracked`] with
///   `directories: true`, so its confirm dialog's checkbox starts checked.
fn build_uncommitted_menu(write_enabled: bool) -> Vec<MenuItem<GitGraphAction>> {
    if !write_enabled {
        return vec![];
    }
    let stash = vec![item(
        "Stash uncommitted changes…",
        GitGraphAction::PromptStash,
    )];
    let reset_clean = vec![
        item(
            "Reset uncommitted changes…",
            GitGraphAction::PromptResetUncommitted,
        ),
        item(
            "Clean untracked files…",
            GitGraphAction::BeginWriteOp(GitWriteOp::CleanUntracked { directories: true }),
        ),
    ];
    join_groups(vec![stash, reset_clean])
}

fn build_commit_menu(commit: &CommitNode, write_enabled: bool) -> Vec<MenuItem<GitGraphAction>> {
    let hash = commit.hash.clone();

    let create = if write_enabled {
        vec![
            item(
                "Add Tag…",
                GitGraphAction::OpenAddTag { hash: hash.clone() },
            ),
            item(
                "Create Branch…",
                GitGraphAction::PromptInput(PromptKind::CreateBranch { hash: hash.clone() }),
            ),
        ]
    } else {
        vec![]
    };

    let mut apply = vec![];
    if write_enabled {
        apply.push(item(
            "Checkout…",
            GitGraphAction::BeginWriteOp(GitWriteOp::CheckoutCommit {
                hash: hash.clone(),
                force: false,
            }),
        ));
        apply.push(item(
            "Cherry Pick…",
            GitGraphAction::BeginWriteOp(GitWriteOp::CherryPick { hash: hash.clone() }),
        ));
        apply.push(item(
            "Revert…",
            GitGraphAction::BeginWriteOp(GitWriteOp::Revert { hash: hash.clone() }),
        ));
        apply.push(item(
            "Drop…",
            GitGraphAction::BeginWriteOp(GitWriteOp::DropCommit { hash: hash.clone() }),
        ));
    }

    let mut current_branch = vec![];
    if write_enabled {
        current_branch.push(item(
            "Merge into current branch…",
            GitGraphAction::BeginWriteOp(GitWriteOp::Merge { rev: hash.clone() }),
        ));
        current_branch.push(item(
            "Rebase current branch on this Commit…",
            GitGraphAction::BeginWriteOp(GitWriteOp::Rebase { hash: hash.clone() }),
        ));
        current_branch.push(item(
            "Reset current branch to this Commit…",
            GitGraphAction::PromptResetMode { hash: hash.clone() },
        ));
    }

    let copy = vec![
        item(
            "Copy Commit Hash to Clipboard",
            GitGraphAction::CopyToClipboard(hash.clone()),
        ),
        item(
            "Copy Commit Subject to Clipboard",
            GitGraphAction::CopyToClipboard(commit.subject.clone()),
        ),
    ];

    join_groups(vec![create, apply, current_branch, copy])
}

/// The focused menu for a right-click on the short-hash text: a single
/// "Copy Short Hash to Clipboard" that copies the displayed 8-char hash. Always
/// available (copying is read-only), independent of `write_enabled`. Distinct
/// from the commit menu's "Copy Commit Hash", which copies the full hash.
fn build_short_hash_menu(commit: &CommitNode) -> Vec<MenuItem<GitGraphAction>> {
    vec![item(
        "Copy Short Hash to Clipboard",
        GitGraphAction::CopyToClipboard(commit.short_hash.clone()),
    )]
}

fn build_tag_menu(index: usize, name: &str, write_enabled: bool) -> Vec<MenuItem<GitGraphAction>> {
    let mut head = vec![item("View Details", GitGraphAction::SelectCommit(index))];
    if write_enabled {
        head.push(item(
            "Delete Tag…",
            GitGraphAction::BeginWriteOp(GitWriteOp::DeleteTag {
                name: name.to_string(),
            }),
        ));
        head.push(item(
            "Push Tag…",
            GitGraphAction::BeginWriteOp(GitWriteOp::PushTag {
                remote: DEFAULT_PUSH_REMOTE.to_string(),
                name: name.to_string(),
                force: false,
            }),
        ));
    }

    let mut tail = vec![];
    if write_enabled {
        tail.push(item(
            "Create Archive",
            GitGraphAction::BeginArchive {
                rev: name.to_string(),
                suggested_name: archive_suggested_name(name),
            },
        ));
    }
    tail.push(item(
        "Copy Tag Name to Clipboard",
        GitGraphAction::CopyToClipboard(name.to_string()),
    ));

    join_groups(vec![head, tail])
}

fn build_remote_branch_menu(name: &str, write_enabled: bool) -> Vec<MenuItem<GitGraphAction>> {
    let (remote, branch) = split_remote_ref(name);
    let full_ref = format!("refs/remotes/{name}");

    let mut ops = vec![];
    if write_enabled {
        ops.push(item(
            "Checkout Branch…",
            GitGraphAction::BeginWriteOp(GitWriteOp::CheckoutBranch {
                branch: branch.clone(),
                force: false,
            }),
        ));
        ops.push(item(
            "Delete Remote Branch…",
            GitGraphAction::BeginWriteOp(GitWriteOp::DeleteRemoteBranch {
                remote: remote.clone(),
                branch: branch.clone(),
            }),
        ));
        ops.push(item(
            "Merge into current branch…",
            GitGraphAction::BeginWriteOp(GitWriteOp::Merge {
                rev: name.to_string(),
            }),
        ));
        ops.push(item(
            "Pull into current branch…",
            GitGraphAction::BeginWriteOp(GitWriteOp::Pull {
                remote: remote.clone(),
                branch: branch.clone(),
            }),
        ));
    }

    let mut middle = vec![];
    if write_enabled {
        middle.push(item(
            "Create Archive",
            GitGraphAction::BeginArchive {
                rev: name.to_string(),
                suggested_name: archive_suggested_name(name),
            },
        ));
    }
    middle.push(item(
        "Unselect in Branches Dropdown",
        GitGraphAction::ToggleBranch(full_ref),
    ));

    let copy = vec![item(
        "Copy Branch Name to Clipboard",
        GitGraphAction::CopyToClipboard(name.to_string()),
    )];

    join_groups(vec![ops, middle, copy])
}

fn build_local_branch_menu(
    name: &str,
    is_current: bool,
    write_enabled: bool,
) -> Vec<MenuItem<GitGraphAction>> {
    let full_ref = format!("refs/heads/{name}");

    let mut ops = vec![];
    if write_enabled {
        // Checkout / delete / merge-into-current / rebase-onto only make sense
        // for a branch other than the one already checked out.
        if !is_current {
            ops.push(item(
                "Checkout Branch…",
                GitGraphAction::BeginWriteOp(GitWriteOp::CheckoutBranch {
                    branch: name.to_string(),
                    force: false,
                }),
            ));
        }
        ops.push(item(
            "Rename Branch…",
            GitGraphAction::PromptInput(PromptKind::RenameBranch {
                old: name.to_string(),
            }),
        ));
        if !is_current {
            ops.push(item(
                "Delete Branch…",
                GitGraphAction::BeginWriteOp(GitWriteOp::DeleteLocalBranch {
                    name: name.to_string(),
                    force: false,
                }),
            ));
            ops.push(item(
                "Merge into current branch…",
                GitGraphAction::BeginWriteOp(GitWriteOp::Merge {
                    rev: name.to_string(),
                }),
            ));
            ops.push(item(
                "Rebase current branch on Branch…",
                GitGraphAction::BeginWriteOp(GitWriteOp::RebaseOntoBranch {
                    branch: name.to_string(),
                }),
            ));
        }
        ops.push(item(
            "Push Branch…",
            GitGraphAction::BeginWriteOp(GitWriteOp::PushBranch {
                remote: DEFAULT_PUSH_REMOTE.to_string(),
                branch: name.to_string(),
                force: false,
            }),
        ));
    }

    let mut middle = vec![];
    if write_enabled {
        middle.push(item(
            "Create Archive",
            GitGraphAction::BeginArchive {
                rev: name.to_string(),
                suggested_name: archive_suggested_name(name),
            },
        ));
    }
    middle.push(item(
        "Unselect in Branches Dropdown",
        GitGraphAction::ToggleBranch(full_ref),
    ));

    let copy = vec![item(
        "Copy Branch Name to Clipboard",
        GitGraphAction::CopyToClipboard(name.to_string()),
    )];

    join_groups(vec![ops, middle, copy])
}

#[cfg(test)]
#[path = "menu_tests.rs"]
mod tests;

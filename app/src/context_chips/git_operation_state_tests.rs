use command::Stdio;
use command::r#async::Command;
use warp_util::git::run_git_command;

use super::{GitOperationAction, GitOperationKind};

/// Run a git command inside `repo`, ignoring its output. Used only to put a
/// repository into a specific mid-operation state.
async fn git(repo: &std::path::Path, args: &[&str]) {
    Command::new("git")
        .args(args)
        .current_dir(repo)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .expect("failed to run git");
}

fn make_git_dir() -> tempfile::TempDir {
    tempfile::tempdir().expect("failed to create temp dir")
}

#[test]
fn detect_returns_none_for_a_repo_with_no_operation_in_progress() {
    let git_dir = make_git_dir();
    assert_eq!(GitOperationKind::detect(git_dir.path()), None);
}

#[test]
fn detect_returns_rebase_interactive_when_rebase_merge_dir_present() {
    let git_dir = make_git_dir();
    std::fs::create_dir(git_dir.path().join("rebase-merge")).unwrap();

    assert_eq!(
        GitOperationKind::detect(git_dir.path()),
        Some(GitOperationKind::RebaseInteractive)
    );
}

#[test]
fn detect_returns_rebase_apply_when_rebase_apply_dir_present_without_applying_marker() {
    let git_dir = make_git_dir();
    std::fs::create_dir(git_dir.path().join("rebase-apply")).unwrap();

    assert_eq!(
        GitOperationKind::detect(git_dir.path()),
        Some(GitOperationKind::RebaseApply)
    );
}

#[test]
fn detect_returns_am_when_rebase_apply_dir_has_applying_marker() {
    let git_dir = make_git_dir();
    let rebase_apply = git_dir.path().join("rebase-apply");
    std::fs::create_dir(&rebase_apply).unwrap();
    std::fs::write(rebase_apply.join("applying"), "").unwrap();

    assert_eq!(
        GitOperationKind::detect(git_dir.path()),
        Some(GitOperationKind::Am)
    );
}

#[test]
fn detect_returns_merge_when_merge_head_present() {
    let git_dir = make_git_dir();
    std::fs::write(git_dir.path().join("MERGE_HEAD"), "").unwrap();

    assert_eq!(
        GitOperationKind::detect(git_dir.path()),
        Some(GitOperationKind::Merge)
    );
}

#[test]
fn detect_returns_cherry_pick_when_cherry_pick_head_present() {
    let git_dir = make_git_dir();
    std::fs::write(git_dir.path().join("CHERRY_PICK_HEAD"), "").unwrap();

    assert_eq!(
        GitOperationKind::detect(git_dir.path()),
        Some(GitOperationKind::CherryPick)
    );
}

#[test]
fn detect_returns_revert_when_revert_head_present() {
    let git_dir = make_git_dir();
    std::fs::write(git_dir.path().join("REVERT_HEAD"), "").unwrap();

    assert_eq!(
        GitOperationKind::detect(git_dir.path()),
        Some(GitOperationKind::Revert)
    );
}

#[test]
fn detect_returns_bisect_when_bisect_log_present() {
    let git_dir = make_git_dir();
    std::fs::write(git_dir.path().join("BISECT_LOG"), "").unwrap();

    assert_eq!(
        GitOperationKind::detect(git_dir.path()),
        Some(GitOperationKind::Bisect)
    );
}

#[test]
fn detect_prefers_rebase_merge_over_other_concurrently_present_sentinels() {
    // Precedence matches Git's own behavior: a rebase-merge sentinel takes
    // priority even if stale sentinels from another state are also present.
    let git_dir = make_git_dir();
    std::fs::create_dir(git_dir.path().join("rebase-merge")).unwrap();
    std::fs::write(git_dir.path().join("MERGE_HEAD"), "").unwrap();
    std::fs::write(git_dir.path().join("BISECT_LOG"), "").unwrap();

    assert_eq!(
        GitOperationKind::detect(git_dir.path()),
        Some(GitOperationKind::RebaseInteractive)
    );
}

#[test]
fn token_round_trips_through_from_token_for_every_variant() {
    for kind in [
        GitOperationKind::RebaseInteractive,
        GitOperationKind::RebaseApply,
        GitOperationKind::Am,
        GitOperationKind::Merge,
        GitOperationKind::CherryPick,
        GitOperationKind::Revert,
        GitOperationKind::Bisect,
    ] {
        assert_eq!(GitOperationKind::from_token(kind.token()), Some(kind));
    }
}

#[test]
fn from_token_parses_every_known_token() {
    assert_eq!(
        GitOperationKind::from_token("rebase-interactive"),
        Some(GitOperationKind::RebaseInteractive)
    );
    assert_eq!(
        GitOperationKind::from_token("rebase-apply"),
        Some(GitOperationKind::RebaseApply)
    );
    assert_eq!(
        GitOperationKind::from_token("am"),
        Some(GitOperationKind::Am)
    );
    assert_eq!(
        GitOperationKind::from_token("merge"),
        Some(GitOperationKind::Merge)
    );
    assert_eq!(
        GitOperationKind::from_token("cherry-pick"),
        Some(GitOperationKind::CherryPick)
    );
    assert_eq!(
        GitOperationKind::from_token("revert"),
        Some(GitOperationKind::Revert)
    );
    assert_eq!(
        GitOperationKind::from_token("bisect"),
        Some(GitOperationKind::Bisect)
    );
}

#[test]
fn from_token_trims_surrounding_whitespace() {
    assert_eq!(
        GitOperationKind::from_token("  merge\n"),
        Some(GitOperationKind::Merge)
    );
}

#[test]
fn from_token_rejects_unrecognized_or_empty_tokens() {
    assert_eq!(GitOperationKind::from_token(""), None);
    assert_eq!(GitOperationKind::from_token("not-a-state"), None);
}

#[test]
fn available_actions_offers_continue_skip_abort_for_rebase_states() {
    assert_eq!(
        GitOperationKind::RebaseInteractive.available_actions(),
        &[
            GitOperationAction::RebaseContinue,
            GitOperationAction::RebaseSkip,
            GitOperationAction::RebaseAbort,
        ]
    );
    assert_eq!(
        GitOperationKind::RebaseApply.available_actions(),
        GitOperationKind::RebaseInteractive.available_actions()
    );
}

#[test]
fn available_actions_offers_only_continue_and_abort_for_merge() {
    assert_eq!(
        GitOperationKind::Merge.available_actions(),
        &[
            GitOperationAction::MergeContinue,
            GitOperationAction::MergeAbort
        ]
    );
}

#[test]
fn available_actions_offers_good_bad_skip_reset_for_bisect() {
    assert_eq!(
        GitOperationKind::Bisect.available_actions(),
        &[
            GitOperationAction::BisectGood,
            GitOperationAction::BisectBad,
            GitOperationAction::BisectSkip,
            GitOperationAction::BisectReset,
        ]
    );
}

#[test]
fn git_args_maps_each_action_to_its_exact_static_argv() {
    assert_eq!(
        GitOperationAction::RebaseContinue.git_args(),
        &["rebase", "--continue"]
    );
    assert_eq!(
        GitOperationAction::RebaseSkip.git_args(),
        &["rebase", "--skip"]
    );
    assert_eq!(
        GitOperationAction::RebaseAbort.git_args(),
        &["rebase", "--abort"]
    );
    assert_eq!(
        GitOperationAction::AmContinue.git_args(),
        &["am", "--continue"]
    );
    assert_eq!(GitOperationAction::AmSkip.git_args(), &["am", "--skip"]);
    assert_eq!(GitOperationAction::AmAbort.git_args(), &["am", "--abort"]);
    assert_eq!(
        GitOperationAction::MergeContinue.git_args(),
        &["merge", "--continue"]
    );
    assert_eq!(
        GitOperationAction::MergeAbort.git_args(),
        &["merge", "--abort"]
    );
    assert_eq!(
        GitOperationAction::CherryPickContinue.git_args(),
        &["cherry-pick", "--continue"]
    );
    assert_eq!(
        GitOperationAction::CherryPickSkip.git_args(),
        &["cherry-pick", "--skip"]
    );
    assert_eq!(
        GitOperationAction::CherryPickAbort.git_args(),
        &["cherry-pick", "--abort"]
    );
    assert_eq!(
        GitOperationAction::RevertContinue.git_args(),
        &["revert", "--continue"]
    );
    assert_eq!(
        GitOperationAction::RevertSkip.git_args(),
        &["revert", "--skip"]
    );
    assert_eq!(
        GitOperationAction::RevertAbort.git_args(),
        &["revert", "--abort"]
    );
    assert_eq!(
        GitOperationAction::BisectGood.git_args(),
        &["bisect", "good"]
    );
    assert_eq!(GitOperationAction::BisectBad.git_args(), &["bisect", "bad"]);
    assert_eq!(
        GitOperationAction::BisectSkip.git_args(),
        &["bisect", "skip"]
    );
    assert_eq!(
        GitOperationAction::BisectReset.git_args(),
        &["bisect", "reset"]
    );
}

#[test]
fn label_groups_actions_by_verb_regardless_of_operation() {
    assert_eq!(GitOperationAction::RebaseContinue.label(), "Continue");
    assert_eq!(GitOperationAction::CherryPickContinue.label(), "Continue");
    assert_eq!(GitOperationAction::RebaseSkip.label(), "Skip");
    assert_eq!(GitOperationAction::BisectSkip.label(), "Skip");
    assert_eq!(GitOperationAction::RebaseAbort.label(), "Abort");
    assert_eq!(GitOperationAction::MergeAbort.label(), "Abort");
    assert_eq!(GitOperationAction::BisectGood.label(), "Good");
    assert_eq!(GitOperationAction::BisectBad.label(), "Bad");
    assert_eq!(GitOperationAction::BisectReset.label(), "Reset");
}

/// Creates a temp git repo with one commit and returns `(dir_handle, repo_path)`.
async fn init_repo() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().expect("failed to create temp dir");
    let path = dir.path().to_path_buf();

    git(&path, &["init", "-b", "main"]).await;
    git(&path, &["config", "user.email", "test@test.com"]).await;
    git(&path, &["config", "user.name", "Test"]).await;
    std::fs::write(path.join("file.txt"), "one\n").unwrap();
    git(&path, &["add", "file.txt"]).await;
    git(&path, &["commit", "-m", "initial"]).await;

    (dir, path)
}

/// A linked worktree's `.git` is a *file*, and the sentinel files this module
/// looks for live under the main repo's `.git/worktrees/<name>`, not under
/// `<worktree>/.git`. This confirms detection works correctly when driven by
/// the git-resolved directory (`git rev-parse --git-dir`) from inside the
/// worktree, rather than by assuming `<worktree>/.git` is itself the git dir.
#[tokio::test]
async fn detect_finds_rebase_in_progress_from_a_linked_worktree() {
    let (_main_dir, main_repo) = init_repo().await;
    git(&main_repo, &["branch", "feature"]).await;

    let worktree_dir = tempfile::tempdir().expect("failed to create worktree temp dir");
    let worktree_path = worktree_dir.path().join("wt");
    git(
        &main_repo,
        &[
            "worktree",
            "add",
            worktree_path.to_str().unwrap(),
            "feature",
        ],
    )
    .await;

    // `.git` inside a linked worktree is a file, not a directory.
    assert!(worktree_path.join(".git").is_file());

    // Create a conflicting rebase target so `rebase` stops mid-way.
    std::fs::write(main_repo.join("file.txt"), "two\n").unwrap();
    git(&main_repo, &["add", "file.txt"]).await;
    git(&main_repo, &["commit", "-m", "conflicting change"]).await;

    std::fs::write(worktree_path.join("file.txt"), "three\n").unwrap();
    git(&worktree_path, &["add", "file.txt"]).await;
    git(&worktree_path, &["commit", "-m", "worktree change"]).await;
    git(&worktree_path, &["rebase", "main"]).await;

    let resolved_git_dir = run_git_command(&worktree_path, &["rev-parse", "--git-dir"])
        .await
        .expect("failed to resolve git dir")
        .trim()
        .to_string();
    // `git rev-parse --git-dir` returns a path relative to the cwd it was run
    // in when the resolved dir is beneath it; resolve it the same way.
    let resolved_git_dir = if std::path::Path::new(&resolved_git_dir).is_absolute() {
        std::path::PathBuf::from(resolved_git_dir)
    } else {
        worktree_path.join(resolved_git_dir)
    };

    // A plain `git rebase` may use either the "apply" or "merge" backend
    // depending on the installed git version's default (`rebase.backend`);
    // either is a correct detection of "a rebase is in progress" here. The
    // property under test is that detection finds it at all from the
    // worktree's resolved git dir, not which backend produced it.
    assert!(
        matches!(
            GitOperationKind::detect(&resolved_git_dir),
            Some(GitOperationKind::RebaseInteractive | GitOperationKind::RebaseApply)
        ),
        "expected an in-progress rebase to be detected"
    );
}

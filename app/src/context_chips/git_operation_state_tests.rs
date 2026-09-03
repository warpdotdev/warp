use command::Stdio;
use command::r#async::Command;
use warp_util::git::run_git_command;

use super::{GitOperationAction, GitOperationKind};
use crate::context_chips::builtins::shell_git_operation_state;
use crate::terminal::shell::ShellType;

/// Runs the actual bash variant of `shell_git_operation_state`'s detection
/// command (the same text sent to a user's shell to refresh the chip) inside
/// `repo`, and parses its output. Exercises the real shell script end to end,
/// rather than only the Rust-side token parsing.
///
/// Not run on Windows: CI's `windows-latest-large` runners resolve `bash` to
/// Git for Windows' bundled MSYS bash, and the nested `bash -c "sh -c '...'"`
/// invocation this helper performs reliably fails to see real mid-operation
/// state there (`BISECT_LOG`/`MERGE_HEAD`) even though the same state is
/// correctly detected by the PowerShell variant below and by direct
/// filesystem checks (`GitOperationKind::detect`) — narrowing this to an
/// environment quirk in that nested-bash invocation rather than the
/// generated script itself. This mirrors this repo's own CI, which likewise
/// only runs the bash/zsh/fish/powershell `shell_integration_tests` on
/// non-Windows (see `.github/workflows/ci.yml`).
#[cfg(not(windows))]
async fn detect_via_generated_shell_command(repo: &std::path::Path) -> Option<GitOperationKind> {
    let generator = shell_git_operation_state();
    let command = generator
        .command()
        .for_shell(ShellType::Bash)
        .expect("a bash command should be defined")
        .to_string();

    let output = Command::new("bash")
        .arg("-c")
        .arg(&command)
        .current_dir(repo)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .expect("failed to run the generated detection command");

    if !output.status.success() {
        return None;
    }
    GitOperationKind::from_token(&String::from_utf8_lossy(&output.stdout))
}

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
fn from_token_parses_every_token_emitted_by_the_detection_shell_command() {
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

#[cfg(not(windows))]
#[tokio::test]
async fn generated_shell_command_reports_no_state_for_a_clean_repo() {
    let (_dir, repo) = init_repo().await;
    assert_eq!(detect_via_generated_shell_command(&repo).await, None);
}

#[cfg(not(windows))]
#[tokio::test]
async fn generated_shell_command_reports_bisect_during_a_real_bisect() {
    let (_dir, repo) = init_repo().await;
    for i in 1..=3 {
        git(
            &repo,
            &["commit", "--allow-empty", "-m", &format!("commit {i}")],
        )
        .await;
    }
    git(&repo, &["bisect", "start"]).await;
    git(&repo, &["bisect", "bad"]).await;
    let root_commit = run_git_command(&repo, &["rev-list", "--max-parents=0", "HEAD"])
        .await
        .expect("failed to resolve the root commit");
    git(&repo, &["bisect", "good", root_commit.trim()]).await;

    assert_eq!(
        detect_via_generated_shell_command(&repo).await,
        Some(GitOperationKind::Bisect)
    );
}

#[cfg(not(windows))]
#[tokio::test]
async fn generated_shell_command_reports_merge_during_a_real_conflicting_merge() {
    let (_dir, repo) = init_repo().await;
    git(&repo, &["checkout", "-b", "feature"]).await;
    std::fs::write(repo.join("file.txt"), "feature\n").unwrap();
    git(&repo, &["commit", "-am", "feature change"]).await;
    git(&repo, &["checkout", "main"]).await;
    std::fs::write(repo.join("file.txt"), "main\n").unwrap();
    git(&repo, &["commit", "-am", "main change"]).await;
    git(&repo, &["merge", "feature"]).await;

    assert_eq!(
        detect_via_generated_shell_command(&repo).await,
        Some(GitOperationKind::Merge)
    );
}

/// Runs the actual PowerShell variant of `shell_git_operation_state`'s
/// detection command inside `repo`, and parses its output. Windows CI ships
/// `pwsh` on its runners (see `.github/workflows/ci.yml`), so this exercises
/// the real PowerShell script end to end, mirroring the Bash coverage above.
#[cfg(windows)]
async fn detect_via_generated_shell_command_powershell(
    repo: &std::path::Path,
) -> Option<GitOperationKind> {
    let generator = shell_git_operation_state();
    let command = generator
        .command()
        .for_shell(ShellType::PowerShell)
        .expect("a powershell command should be defined")
        .to_string();

    let output = Command::new("pwsh")
        .arg("-NoProfile")
        .arg("-Command")
        .arg(&command)
        .current_dir(repo)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .expect("failed to run the generated detection command");

    if !output.status.success() {
        return None;
    }
    GitOperationKind::from_token(&String::from_utf8_lossy(&output.stdout))
}

#[cfg(windows)]
#[tokio::test]
async fn generated_shell_command_reports_no_state_for_a_clean_repo_powershell() {
    let (_dir, repo) = init_repo().await;
    assert_eq!(
        detect_via_generated_shell_command_powershell(&repo).await,
        None
    );
}

#[cfg(windows)]
#[tokio::test]
async fn generated_shell_command_reports_bisect_during_a_real_bisect_powershell() {
    let (_dir, repo) = init_repo().await;
    for i in 1..=3 {
        git(
            &repo,
            &["commit", "--allow-empty", "-m", &format!("commit {i}")],
        )
        .await;
    }
    git(&repo, &["bisect", "start"]).await;
    git(&repo, &["bisect", "bad"]).await;
    let root_commit = run_git_command(&repo, &["rev-list", "--max-parents=0", "HEAD"])
        .await
        .expect("failed to resolve the root commit");
    git(&repo, &["bisect", "good", root_commit.trim()]).await;

    assert_eq!(
        detect_via_generated_shell_command_powershell(&repo).await,
        Some(GitOperationKind::Bisect)
    );
}

#[cfg(windows)]
#[tokio::test]
async fn generated_shell_command_reports_merge_during_a_real_conflicting_merge_powershell() {
    let (_dir, repo) = init_repo().await;
    git(&repo, &["checkout", "-b", "feature"]).await;
    std::fs::write(repo.join("file.txt"), "feature\n").unwrap();
    git(&repo, &["commit", "-am", "feature change"]).await;
    git(&repo, &["checkout", "main"]).await;
    std::fs::write(repo.join("file.txt"), "main\n").unwrap();
    git(&repo, &["commit", "-am", "main change"]).await;
    git(&repo, &["merge", "feature"]).await;

    assert_eq!(
        detect_via_generated_shell_command_powershell(&repo).await,
        Some(GitOperationKind::Merge)
    );
}

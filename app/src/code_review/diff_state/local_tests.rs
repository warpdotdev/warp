use tempfile::TempDir;

use super::*;
use crate::util::git::{
    BranchEntry, parse_range, parse_unified_diff_header, sort_branches_main_first,
};

#[test]
fn test_parse_range_with_comma() {
    let (start, count) =
        parse_range("10,5").expect("parse_range should succeed for range with count");
    assert_eq!(start, 10);
    assert_eq!(count, 5);
}

#[test]
fn test_parse_range_without_comma() {
    let (start, count) =
        parse_range("10").expect("parse_range should succeed for range without count");
    assert_eq!(start, 10);
    assert_eq!(count, 1);
}

#[test]
fn test_parse_unified_diff_header_basic() {
    let header = "@@ -10,5 +12,7 @@";
    let parsed = parse_unified_diff_header(header)
        .expect("parse_unified_diff_header should succeed for basic header");
    assert_eq!(parsed.old_start_line, 10);
    assert_eq!(parsed.old_line_count, 5);
    assert_eq!(parsed.new_start_line, 12);
    assert_eq!(parsed.new_line_count, 7);
}

#[test]
fn test_parse_unified_diff_header_with_context() {
    let header = "@@ -4978,33 +4978,43 @@ impl TerminalView {";
    let parsed = parse_unified_diff_header(header)
        .expect("parse_unified_diff_header should succeed for header with context");
    assert_eq!(parsed.old_start_line, 4978);
    assert_eq!(parsed.old_line_count, 33);
    assert_eq!(parsed.new_start_line, 4978);
    assert_eq!(parsed.new_line_count, 43);
}

#[test]
fn test_parse_unified_diff_header_single_line() {
    let header = "@@ -10 +12,3 @@";
    let parsed = parse_unified_diff_header(header)
        .expect("parse_unified_diff_header should succeed for single line header");
    assert_eq!(parsed.old_start_line, 10);
    assert_eq!(parsed.old_line_count, 1);
    assert_eq!(parsed.new_start_line, 12);
    assert_eq!(parsed.new_line_count, 3);
}

#[test]
fn test_sort_branches_main_first_empty() {
    let branches: Vec<BranchEntry> = vec![];
    let result: Vec<_> = sort_branches_main_first(&branches).collect();
    assert!(result.is_empty());
}

#[test]
fn test_sort_branches_main_first_no_main() {
    let branches = vec![
        BranchEntry {
            name: "feature-a".to_string(),
            is_main: false,
        },
        BranchEntry {
            name: "feature-b".to_string(),
            is_main: false,
        },
        BranchEntry {
            name: "feature-c".to_string(),
            is_main: false,
        },
    ];
    let result: Vec<_> = sort_branches_main_first(&branches).collect();
    // No main branches — order should be unchanged.
    assert_eq!(result, branches.iter().collect::<Vec<_>>());
}

#[test]
fn test_sort_branches_main_first_promotes_main() {
    let branches = vec![
        BranchEntry {
            name: "feature-a".to_string(),
            is_main: false,
        },
        BranchEntry {
            name: "main".to_string(),
            is_main: true,
        },
        BranchEntry {
            name: "feature-b".to_string(),
            is_main: false,
        },
    ];
    let result: Vec<_> = sort_branches_main_first(&branches)
        .map(|entry| entry.name.as_str())
        .collect();
    assert_eq!(result, vec!["main", "feature-a", "feature-b"]);
}

#[test]
fn test_sort_branches_main_first_main_already_first() {
    let branches = vec![
        BranchEntry {
            name: "main".to_string(),
            is_main: true,
        },
        BranchEntry {
            name: "feature-a".to_string(),
            is_main: false,
        },
        BranchEntry {
            name: "feature-b".to_string(),
            is_main: false,
        },
    ];
    let result: Vec<_> = sort_branches_main_first(&branches)
        .map(|entry| entry.name.as_str())
        .collect();
    assert_eq!(result, vec!["main", "feature-a", "feature-b"]);
}

#[test]
fn test_sort_branches_main_first_preserves_recency_order_for_non_main() {
    // Non-main branches should remain in their original (recency) order.
    let branches = vec![
        BranchEntry {
            name: "recent-feature".to_string(),
            is_main: false,
        },
        BranchEntry {
            name: "main".to_string(),
            is_main: true,
        },
        BranchEntry {
            name: "older-feature".to_string(),
            is_main: false,
        },
        BranchEntry {
            name: "oldest-feature".to_string(),
            is_main: false,
        },
    ];
    let result: Vec<_> = sort_branches_main_first(&branches)
        .map(|entry| entry.name.as_str())
        .collect();
    assert_eq!(
        result,
        vec!["main", "recent-feature", "older-feature", "oldest-feature"]
    );
}

#[test]
fn test_sort_branches_main_first_multiple_main_flags() {
    // Defensive: both flagged as main (shouldn't happen in practice, but
    // sort_branches_main_first should handle it gracefully).
    let branches = vec![
        BranchEntry {
            name: "feature".to_string(),
            is_main: false,
        },
        BranchEntry {
            name: "main".to_string(),
            is_main: true,
        },
        BranchEntry {
            name: "master".to_string(),
            is_main: true,
        },
    ];
    let result: Vec<_> = sort_branches_main_first(&branches)
        .map(|entry| entry.name.as_str())
        .collect();
    // Both main-flagged entries appear first, non-main last.
    assert_eq!(result, vec!["main", "master", "feature"]);
}

#[test]
fn test_parse_unified_diff_header_malformed() {
    let header = "not a diff header";
    let result = parse_unified_diff_header(header);
    assert!(result.is_err());

    let header2 = "@@ incomplete";
    let result2 = parse_unified_diff_header(header2);
    assert!(result2.is_err());
}

#[test]
fn test_parse_git_status_modified_file_with_spaces() {
    // Porcelain v2 output for a modified file with spaces in the name.
    // Format: 1 <XY> <sub> <mH> <mI> <mW> <hH> <hI> <path>
    let status_output = "1 .M N... 100644 100644 100644 abc1234 def5678 test file.txt";
    let result = LocalDiffStateModel::parse_git_status(status_output).unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].0, "test file.txt");
    assert_eq!(result[0].1, GitFileStatus::Modified);
}

#[test]
fn test_parse_git_status_modified_file_with_multiple_spaces() {
    // Filename with multiple spaces.
    let status_output = "1 .M N... 100644 100644 100644 abc1234 def5678 path to/my test file.txt";
    let result = LocalDiffStateModel::parse_git_status(status_output).unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].0, "path to/my test file.txt");
    assert_eq!(result[0].1, GitFileStatus::Modified);
}

#[test]
fn test_parse_git_status_new_file_with_spaces() {
    let status_output = "1 A. N... 000000 100644 100644 0000000 abc1234 new file name.rs";
    let result = LocalDiffStateModel::parse_git_status(status_output).unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].0, "new file name.rs");
    assert_eq!(result[0].1, GitFileStatus::New);
}

#[test]
fn test_parse_git_status_renamed_file_with_spaces() {
    // Porcelain v2 renamed entry (type 2) with spaces in the new path.
    // Format: 2 <XY> <sub> <mH> <mI> <mW> <hH> <hI> <X><score> <path>\0<origPath>
    let status_output =
        "2 R. N... 100644 100644 100644 abc1234 def5678 R100 new name.txt\0old name.txt";
    let result = LocalDiffStateModel::parse_git_status(status_output).unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].0, "new name.txt");
    assert!(matches!(
        &result[0].1,
        GitFileStatus::Renamed { old_path } if old_path == "old name.txt"
    ));
}

#[test]
fn test_parse_git_status_untracked_file_with_spaces() {
    let status_output = "? my untracked file.txt";
    let result = LocalDiffStateModel::parse_git_status(status_output).unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].0, "my untracked file.txt");
    assert_eq!(result[0].1, GitFileStatus::Untracked);
}

#[test]
fn test_parse_git_status_unmerged_file_with_spaces() {
    // Porcelain v2 unmerged entry (type u) with spaces in the path.
    // Format: u <xy> <sub> <m1> <m2> <m3> <mW> <h1> <h2> <h3> <path>
    let status_output =
        "u UU N... 100644 100644 100644 100644 abc1234 def5678 ghi9012 conflict file.txt";
    let result = LocalDiffStateModel::parse_git_status(status_output).unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].0, "conflict file.txt");
    assert_eq!(result[0].1, GitFileStatus::Conflicted);
}

#[test]
fn test_parse_git_status_mixed_entries_with_spaces() {
    // Multiple entries separated by NUL, mixing files with and without spaces.
    let status_output = "1 .M N... 100644 100644 100644 abc1234 def5678 test file.txt\0\
         1 .M N... 100644 100644 100644 abc1234 def5678 normal.txt\0\
         ? another file with spaces.rs";
    let result = LocalDiffStateModel::parse_git_status(status_output).unwrap();
    assert_eq!(result.len(), 3);
    assert_eq!(result[0].0, "test file.txt");
    assert_eq!(result[1].0, "normal.txt");
    assert_eq!(result[2].0, "another file with spaces.rs");
}

#[test]
fn test_parse_git_status_file_without_spaces_still_works() {
    // Ensure the splitn change doesn't break files without spaces.
    let status_output = "1 .M N... 100644 100644 100644 abc1234 def5678 simple.txt";
    let result = LocalDiffStateModel::parse_git_status(status_output).unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].0, "simple.txt");
    assert_eq!(result[0].1, GitFileStatus::Modified);
}

#[tokio::test]
async fn untracked_directory_diff_is_empty_and_non_binary() {
    let repo_dir = tempfile::tempdir().expect("create temp repo dir");
    std::fs::create_dir(repo_dir.path().join("nested-repo")).expect("create nested dir");

    // `git status` reports a nested repo/worktree as a single untracked
    // directory entry (with a trailing slash). It must short-circuit to an
    // empty non-binary diff — the error fallback would otherwise mislabel it
    // as binary and the view would render "Binary file - no diff available"
    // instead of "New empty file".
    let diff = LocalDiffStateModel::get_file_diff(
        repo_dir.path(),
        "nested-repo/",
        &GitFileStatus::Untracked,
        false,
        None,
    )
    .await
    .expect("get_file_diff should succeed for an untracked directory");

    assert!(!diff.is_binary);
    assert_eq!(diff.hunks.len(), 0);
    assert_eq!(diff.status, GitFileStatus::Untracked);
}

#[tokio::test]
async fn untracked_directory_has_no_baseline_content() {
    let repo_dir = tempfile::tempdir().expect("create temp repo dir");
    std::fs::create_dir(repo_dir.path().join("nested-repo")).expect("create nested dir");
    std::fs::write(repo_dir.path().join("new-file.txt"), "hello\n").expect("write file");

    // No baseline for a directory entry, so no editor is constructed for it.
    let dir_content = LocalDiffStateModel::get_file_content_at_head(
        repo_dir.path(),
        "nested-repo/",
        &GitFileStatus::Untracked,
    )
    .await;
    assert_eq!(dir_content, None);

    // Regular untracked files keep their empty baseline.
    let file_content = LocalDiffStateModel::get_file_content_at_head(
        repo_dir.path(),
        "new-file.txt",
        &GitFileStatus::Untracked,
    )
    .await;
    assert_eq!(file_content, Some(String::new()));
}

#[tokio::test]
async fn renamed_file_content_at_head_reads_old_path() {
    let repo_dir = tempfile::tempdir().expect("create temp repo dir");
    let repo_path = repo_dir.path();

    // Set up a real git repo with one committed file, then rename it in the working tree
    // (without committing the rename) so HEAD only knows about the old path.
    run_git_command(repo_path, &["init", "-b", "main"])
        .await
        .expect("git init");
    run_git_command(repo_path, &["config", "user.email", "test@test.com"])
        .await
        .expect("git config email");
    run_git_command(repo_path, &["config", "user.name", "Test"])
        .await
        .expect("git config name");
    std::fs::write(repo_path.join("old.txt"), "hello world\n").expect("write old.txt");
    run_git_command(repo_path, &["add", "old.txt"])
        .await
        .expect("git add");
    run_git_command(repo_path, &["commit", "-m", "initial"])
        .await
        .expect("git commit");

    // Rename in the working tree only — `old.txt` no longer exists at this path, so `git
    // show HEAD:new.txt` would fail (the bug in APP-5111).
    std::fs::rename(repo_path.join("old.txt"), repo_path.join("new.txt"))
        .expect("rename old.txt to new.txt");

    let content = LocalDiffStateModel::get_file_content_at_head(
        repo_path,
        "new.txt",
        &GitFileStatus::Renamed {
            old_path: "old.txt".to_string(),
        },
    )
    .await;

    // The baseline content at HEAD must come from the old path, not the new one, so the code
    // review pane can render a diff instead of "Unable to load file content".
    assert_eq!(content, Some("hello world\n".to_string()));
}

#[tokio::test]
async fn staged_rename_and_modify_produces_non_empty_diff() {
    let repo_dir = tempfile::tempdir().expect("create temp repo dir");
    let repo_path = repo_dir.path();

    run_git_command(repo_path, &["init", "-b", "main"])
        .await
        .expect("git init");
    run_git_command(repo_path, &["config", "user.email", "test@test.com"])
        .await
        .expect("git config email");
    run_git_command(repo_path, &["config", "user.name", "Test"])
        .await
        .expect("git config name");
    std::fs::write(
        repo_path.join("old.txt"),
        "line one\nline two\nline three\n",
    )
    .expect("write old.txt");
    run_git_command(repo_path, &["add", "old.txt"])
        .await
        .expect("git add");
    run_git_command(repo_path, &["commit", "-m", "initial"])
        .await
        .expect("git commit");

    // Stage both the rename and a content edit, so nothing is left unstaged (git status
    // reports this as a plain "R " entry with no unstaged component).
    run_git_command(repo_path, &["mv", "old.txt", "new.txt"])
        .await
        .expect("git mv");
    std::fs::write(
        repo_path.join("new.txt"),
        "line one\nline two changed\nline three\n",
    )
    .expect("write new.txt");
    run_git_command(repo_path, &["add", "new.txt"])
        .await
        .expect("git add new.txt");

    let diff = LocalDiffStateModel::get_file_diff(
        repo_path,
        "new.txt",
        &GitFileStatus::Renamed {
            old_path: "old.txt".to_string(),
        },
        false,
        None,
    )
    .await
    .expect("get_file_diff should succeed for a fully staged rename+modify");

    // A fully staged rename with a staged content edit must still render an inline diff
    // instead of falling through to "File renamed without changes": comparing only the
    // index against the working tree (as before the fix) produced an empty diff here,
    // since both changes were already staged.
    assert!(
        !diff.is_empty(),
        "expected a non-empty diff for a fully staged rename+modify"
    );
}

#[tokio::test]
async fn num_lines_in_file_if_non_binary_counts_lines_in_text_file() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let file_path = dir.path().join("file.txt");
    std::fs::write(&file_path, "one\ntwo\nthree\n").expect("write file");

    let num_lines = LocalDiffStateModel::num_lines_in_file_if_non_binary(&file_path)
        .await
        .expect("counting a regular file should succeed");
    assert_eq!(num_lines, Some(3));
}

#[tokio::test]
async fn num_lines_in_file_if_non_binary_errors_for_directory() {
    let dir = tempfile::tempdir().expect("create temp dir");

    // Directories aren't countable. The metadata callers degrade this error
    // to a 0-line contribution per entry instead of failing the whole
    // metadata computation.
    let result = LocalDiffStateModel::num_lines_in_file_if_non_binary(dir.path()).await;
    assert!(result.is_err());
}

// ── Pull request layer diff mode ────────────────────────────────────────

#[test]
fn is_full_git_object_id_accepts_only_full_hex_oids() {
    assert!(LocalDiffStateModel::is_full_git_object_id(&"a".repeat(40)));
    assert!(LocalDiffStateModel::is_full_git_object_id(
        "0123456789abcdef0123456789abcdef01234567"
    ));
    // Too short / too long.
    assert!(!LocalDiffStateModel::is_full_git_object_id(&"a".repeat(39)));
    assert!(!LocalDiffStateModel::is_full_git_object_id(&"a".repeat(41)));
    // Non-hex characters.
    assert!(!LocalDiffStateModel::is_full_git_object_id(&format!(
        "{}g",
        "a".repeat(39)
    )));
    // Shell/ref-like input must never pass.
    assert!(!LocalDiffStateModel::is_full_git_object_id("HEAD"));
    assert!(!LocalDiffStateModel::is_full_git_object_id(""));
}

/// Runs a git command in `repo`, panicking on failure, and returns trimmed stdout.
async fn git(repo: &Path, args: &[&str]) -> String {
    run_git_command(repo, args)
        .await
        .unwrap_or_else(|e| panic!("git {args:?} in {repo:?} failed: {e}"))
        .trim()
        .to_string()
}

/// Sets up a bare `origin` repo and a `work` clone of it, both rooted under one
/// temp dir. `origin` has one commit on `main` (returned as `base_oid`);
/// `work` is cloned from `origin` right after that push, so it has `base_oid`
/// but not yet any pull request commit.
async fn init_pr_layer_fixture() -> (TempDir, PathBuf, PathBuf, String) {
    let root = tempfile::tempdir().expect("create root temp dir");
    let origin_path = root.path().join("origin.git");
    let seed_path = root.path().join("seed");
    let work_path = root.path().join("work");
    std::fs::create_dir(&origin_path).expect("create origin dir");
    std::fs::create_dir(&seed_path).expect("create seed dir");

    git(&origin_path, &["init", "--bare", "-b", "main"]).await;

    git(&seed_path, &["init", "-b", "main"]).await;
    git(&seed_path, &["config", "user.email", "test@test.com"]).await;
    git(&seed_path, &["config", "user.name", "Test"]).await;
    std::fs::write(seed_path.join("a.txt"), "base content\n").expect("write base file");
    git(&seed_path, &["add", "a.txt"]).await;
    git(&seed_path, &["commit", "-m", "initial"]).await;
    let base_oid = git(&seed_path, &["rev-parse", "HEAD"]).await;
    git(
        &seed_path,
        &["remote", "add", "origin", origin_path.to_str().unwrap()],
    )
    .await;
    git(&seed_path, &["push", "origin", "main"]).await;

    git(
        root.path(),
        &[
            "clone",
            origin_path.to_str().unwrap(),
            work_path.to_str().unwrap(),
        ],
    )
    .await;

    (root, origin_path, seed_path, base_oid)
}

#[tokio::test]
async fn pull_request_layer_rejects_invalid_object_ids() {
    let (root, _origin_path, _seed_path, base_oid) = init_pr_layer_fixture().await;
    let work_path = root.path().join("work");

    let result =
        LocalDiffStateModel::diff_state_against_pr_layer(&work_path, 7, "not-an-oid", &base_oid)
            .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn pull_request_layer_loads_diff_and_fetches_missing_head_without_mutating_repo() {
    let (root, origin_path, seed_path, base_oid) = init_pr_layer_fixture().await;
    let work_path = root.path().join("work");

    // Create the pull request's head commit and publish it under GitHub's
    // `refs/pull/{n}/head` convention. `work_path` was cloned before this, so
    // it doesn't have the head commit locally.
    git(&seed_path, &["checkout", "-b", "feature"]).await;
    std::fs::write(seed_path.join("b.txt"), "pr content\n").expect("write pr file");
    git(&seed_path, &["add", "b.txt"]).await;
    git(&seed_path, &["commit", "-m", "add b.txt"]).await;
    let head_oid = git(&seed_path, &["rev-parse", "HEAD"]).await;
    git(&seed_path, &["push", "origin", "feature:refs/pull/7/head"]).await;

    // Snapshot mutable repo state before loading the layer.
    let head_before = git(&work_path, &["rev-parse", "HEAD"]).await;
    let status_before = git(&work_path, &["status", "--porcelain=v2", "--branch"]).await;
    let branches_before = git(&work_path, &["branch", "--list"]).await;
    let remotes_before = git(&work_path, &["for-each-ref", "refs/remotes"]).await;

    let diffs =
        LocalDiffStateModel::diff_state_against_pr_layer(&work_path, 7, &base_oid, &head_oid)
            .await
            .expect("pull request layer diff should load");

    assert_eq!(diffs.files.len(), 1);
    assert_eq!(diffs.files[0].file_diff.file_path, "b.txt");
    assert_eq!(diffs.files[0].content_at_head.as_deref(), Some(""));

    // The head commit had to be fetched; confirm it landed under the
    // Warp-owned ref and resolves to the expected OID.
    let fetched_head = git(
        &work_path,
        &["rev-parse", "refs/warp/code-review/pr/7/head"],
    )
    .await;
    assert_eq!(fetched_head, head_oid);

    // HEAD, the working tree, local branches, and remote-tracking refs must
    // be byte-for-byte unchanged.
    let head_after = git(&work_path, &["rev-parse", "HEAD"]).await;
    let status_after = git(&work_path, &["status", "--porcelain=v2", "--branch"]).await;
    let branches_after = git(&work_path, &["branch", "--list"]).await;
    let remotes_after = git(&work_path, &["for-each-ref", "refs/remotes"]).await;
    assert_eq!(head_before, head_after);
    assert_eq!(status_before, status_after);
    assert_eq!(branches_before, branches_after);
    assert_eq!(remotes_before, remotes_after);

    let _ = origin_path;
}

#[tokio::test]
async fn set_diff_mode_does_not_reload_when_pull_request_layer_is_unchanged() {
    warpui::App::test((), |mut app| async move {
        let handle = app.add_model(LocalDiffStateModel::new_for_test);
        let mode = DiffMode::PullRequestLayer {
            pr_number: 7,
            base_oid: "a".repeat(40),
            head_oid: "b".repeat(40),
        };

        handle.update(&mut app, |model, ctx| {
            model.set_diff_mode(mode.clone(), false, false, ctx);
        });
        let mode_after_first = handle.read(&app, |model, _| model.diff_mode());
        assert_eq!(mode_after_first, mode);

        // Re-applying the identical mode must be a no-op per the `self.mode
        // != mode` guard in `set_diff_mode` — there is no repository attached
        // to this test model, so a real reload would panic/return early
        // either way; this asserts the mode itself is unaffected and equal.
        handle.update(&mut app, |model, ctx| {
            model.set_diff_mode(mode.clone(), false, false, ctx);
        });
        let mode_after_second = handle.read(&app, |model, _| model.diff_mode());
        assert_eq!(mode_after_second, mode);
    });
}

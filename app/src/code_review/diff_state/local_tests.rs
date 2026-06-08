use super::*;
use crate::util::git::{
    parse_range, parse_unified_diff_header, sort_branches_main_first, BranchEntry,
};

// === Helpers for the integration-style tests below ====================================

#[cfg(feature = "local_fs")]
async fn run_git(repo: &std::path::Path, args: &[&str]) {
    use command::r#async::Command;
    let output = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .await
        .expect("failed to run git");
    if !output.status.success() {
        panic!(
            "git {args:?} failed in {repo:?}: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[cfg(feature = "local_fs")]
async fn init_repo_with_initial_commit(file: &str, contents: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("failed to create temp dir");
    let repo = dir.path();
    run_git(repo, &["init", "-b", "main", "--quiet"]).await;
    run_git(repo, &["config", "user.email", "test@test.com"]).await;
    run_git(repo, &["config", "user.name", "Test"]).await;
    run_git(repo, &["config", "commit.gpgsign", "false"]).await;
    // Disable Windows CRLF normalization so that `"v1\n"` round-trips
    // through `git add` without creating phantom diffs.
    run_git(repo, &["config", "core.autocrlf", "false"]).await;
    std::fs::write(repo.join(file), contents).expect("write initial file");
    run_git(repo, &["add", file]).await;
    run_git(repo, &["commit", "-m", "initial", "--quiet"]).await;
    dir
}

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

// ===== Staged-then-reverted regression coverage (#10512) ==============================

#[test]
fn should_retry_with_staged_only_for_modified_or_deleted() {
    // Eligible: Modified, Deleted — these are the cases where the index
    // can still differ from HEAD after a working-tree revert.
    assert!(LocalDiffStateModel::should_retry_with_staged(
        &GitFileStatus::Modified
    ));
    assert!(LocalDiffStateModel::should_retry_with_staged(
        &GitFileStatus::Deleted
    ));

    // Not eligible: Untracked / New (not in the index in a comparable
    // way), Renamed / Copied (paired old/new path semantics that the
    // simple `--cached -- new_path` form doesn't reproduce), Conflicted
    // (`--cached` against an unmerged entry would render stage-2-vs-HEAD,
    // which is not what the panel wants).
    for ineligible in [
        GitFileStatus::Untracked,
        GitFileStatus::New,
        GitFileStatus::Renamed {
            old_path: "x".into(),
        },
        GitFileStatus::Copied {
            old_path: "x".into(),
        },
        GitFileStatus::Conflicted,
    ] {
        assert!(
            !LocalDiffStateModel::should_retry_with_staged(&ineligible),
            "expected staged fallback to skip {ineligible:?}"
        );
    }
}

#[test]
fn staged_diff_args_targets_index_against_head_for_given_file() {
    let args = LocalDiffStateModel::staged_diff_args("path/to/foo.txt");
    // `--cached` (synonym for `--staged`) compares the index to HEAD, which
    // is exactly the patch the staged-then-reverted fallback wants.
    assert!(args.contains(&"--cached"));
    // The file path must be the last positional after `--`, and we must
    // disable external diff drivers / colors / context-collapse the same
    // way the primary command does to keep the parser happy.
    assert_eq!(args.last(), Some(&"path/to/foo.txt"));
    assert!(args.contains(&"--no-ext-diff"));
    assert!(args.contains(&"--no-color"));
    let dash_dash_pos = args.iter().position(|a| *a == "--").unwrap();
    assert_eq!(args[dash_dash_pos + 1], "path/to/foo.txt");
}

#[cfg(feature = "local_fs")]
#[tokio::test]
async fn diff_state_against_head_surfaces_staged_then_reverted_file() {
    // Regression for #10512: after `edit; git add; revert`, `git status`
    // still lists the file (XY = MM) but `git diff HEAD -- file` is empty
    // because the worktree matches HEAD. Without the staged fallback the
    // Code Review panel listed the file with no hunks.
    let dir = init_repo_with_initial_commit("foo.txt", "v1\n").await;
    let repo = dir.path();

    // Stage v2, then revert the worktree back to v1.
    std::fs::write(repo.join("foo.txt"), "v2\n").expect("write v2");
    run_git(repo, &["add", "foo.txt"]).await;
    std::fs::write(repo.join("foo.txt"), "v1\n").expect("revert worktree");

    // Sanity: status reports the file, worktree-vs-HEAD is empty.
    let worktree_diff = command::r#async::Command::new("git")
        .args(["diff", "HEAD", "--", "foo.txt"])
        .current_dir(repo)
        .output()
        .await
        .expect("git diff");
    assert!(
        String::from_utf8_lossy(&worktree_diff.stdout).is_empty(),
        "test scaffolding precondition: worktree-vs-HEAD must be empty"
    );

    let diffs = LocalDiffStateModel::diff_state_against_head(repo)
        .await
        .expect("diff_state_against_head");

    assert_eq!(
        diffs.files_changed, 1,
        "expected the staged-then-reverted file to be reported"
    );
    let file = &diffs.files[0].file_diff;
    assert_eq!(file.file_path, std::path::PathBuf::from("foo.txt"));
    assert!(
        !file.hunks.is_empty(),
        "expected staged fallback to surface hunks; got status={:?} hunks={:?}",
        file.status,
        file.hunks,
    );
}

#[cfg(feature = "local_fs")]
#[tokio::test]
async fn retrieve_diff_state_keeps_binary_staged_then_reverted_file() {
    // Regression for the Oz review on #10512: the per-file invalidation
    // path (`retrieve_diff_state` → `file_diff_for_path`) used the caller's
    // upstream `is_binary` probe to decide whether to drop empty-hunk
    // files. When the staged-then-reverted fallback inside `get_file_diff`
    // detects that the *staged* content is binary, it returns
    // `is_binary: true` with empty hunks — but the caller-side filter saw
    // `!is_binary` and dropped the file entirely. The filter must trust
    // `file_diff.is_binary`, the post-call truth.
    let dir = init_repo_with_initial_commit("foo.txt", "v1\n").await;
    let repo = dir.path();

    // Stage a binary blob, then revert the worktree back to text v1.
    let binary_payload: Vec<u8> = (0..256u16).map(|b| (b & 0xff) as u8).collect();
    std::fs::write(repo.join("foo.txt"), &binary_payload).expect("write binary");
    run_git(repo, &["add", "foo.txt"]).await;
    std::fs::write(repo.join("foo.txt"), "v1\n").expect("revert worktree");

    let abs_path = repo.join("foo.txt");
    let (relative, diff) =
        LocalDiffStateModel::retrieve_diff_state(repo, &abs_path, &DiffMode::Head, None)
            .await
            .expect("retrieve_diff_state");

    assert_eq!(relative, std::path::PathBuf::from("foo.txt"));
    let diff = diff.expect(
        "binary-staged-then-reverted file must not be filtered out by the per-file invalidation path",
    );
    assert!(
        diff.file_diff.is_binary,
        "expected staged-binary fallback to surface as is_binary=true"
    );
}

#[cfg(feature = "local_fs")]
#[tokio::test]
async fn diff_metadata_against_head_counts_staged_then_reverted_lines() {
    // Regression: the panel header / git chip / agent footer pull from
    // `diff_metadata_against_head`, which runs `git diff --numstat HEAD`
    // (worktree vs HEAD) and so misses staged-then-reverted files for the
    // same reason `get_file_diff` did before the #10512 `--cached` fallback.
    //
    // The per-file path now surfaces `foo.txt +1 -1`; without mirroring the
    // fallback here the aggregate would still report `1 file changed, +0 -0`
    // and disagree with the file list rendered in the same panel.
    let dir = init_repo_with_initial_commit("foo.txt", "v1\n").await;
    let repo = dir.path();

    std::fs::write(repo.join("foo.txt"), "v2\n").expect("write v2");
    run_git(repo, &["add", "foo.txt"]).await;
    std::fs::write(repo.join("foo.txt"), "v1\n").expect("revert worktree");

    let metadata = diff_metadata_against_head(repo)
        .await
        .expect("diff_metadata_against_head");

    assert_eq!(metadata.aggregate_stats.files_changed, 1);
    assert_eq!(
        metadata.aggregate_stats.total_additions, 1,
        "expected staged numstat fallback to contribute +1; got {metadata:?}"
    );
    assert_eq!(
        metadata.aggregate_stats.total_deletions, 1,
        "expected staged numstat fallback to contribute -1; got {metadata:?}"
    );
}

#[cfg(feature = "local_fs")]
#[tokio::test]
async fn diff_metadata_against_head_prefers_worktree_numstat_over_staged() {
    // Control: when the worktree diverges from both the index and HEAD,
    // numstat HEAD reports the worktree delta and the staged fallback must
    // not double-count. Stage v2 then edit the worktree to a 3-line v3 (no
    // re-add): worktree-vs-HEAD numstat is +1 -1 (one line replaced) plus
    // two new lines, totalling +3 -1. The staged fallback would have
    // returned +1 -1 against v2 — observably different.
    let dir = init_repo_with_initial_commit("foo.txt", "v1\n").await;
    let repo = dir.path();

    std::fs::write(repo.join("foo.txt"), "v2\n").expect("write v2");
    run_git(repo, &["add", "foo.txt"]).await;
    std::fs::write(repo.join("foo.txt"), "a\nb\nc\n").expect("write worktree v3");

    let metadata = diff_metadata_against_head(repo)
        .await
        .expect("diff_metadata_against_head");

    assert_eq!(metadata.aggregate_stats.files_changed, 1);
    assert_eq!(
        metadata.aggregate_stats.total_additions, 3,
        "expected worktree numstat (+3) not staged (+1); got {metadata:?}"
    );
    assert_eq!(
        metadata.aggregate_stats.total_deletions, 1,
        "expected worktree numstat (-1); got {metadata:?}"
    );
}

#[cfg(feature = "local_fs")]
#[tokio::test]
async fn diff_state_against_head_uses_worktree_when_worktree_diverges_from_index() {
    // Control case: distinguishes the primary path from the fallback by
    // staging v2, then editing the worktree to v3 *without* re-staging.
    // Now `git diff HEAD -- foo.txt` (worktree vs HEAD) shows v1→v3 while
    // `git diff --cached -- foo.txt` would show v1→v2. If the primary
    // path is taken, the resulting hunks must mention v3 — that's
    // observably different from the fallback path.
    let dir = init_repo_with_initial_commit("foo.txt", "v1\n").await;
    let repo = dir.path();

    std::fs::write(repo.join("foo.txt"), "v2\n").expect("write v2");
    run_git(repo, &["add", "foo.txt"]).await;
    std::fs::write(repo.join("foo.txt"), "v3\n").expect("write v3");

    let diffs = LocalDiffStateModel::diff_state_against_head(repo)
        .await
        .expect("diff_state_against_head");

    assert_eq!(diffs.files_changed, 1);
    let hunks = &diffs.files[0].file_diff.hunks;
    assert!(!hunks.is_empty(), "expected hunks from primary path");
    let hunk_text: String = hunks
        .iter()
        .flat_map(|h| h.lines.iter().map(|l| l.text.clone()))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        hunk_text.contains("v3"),
        "expected primary (worktree) path; hunks did not mention v3: {hunk_text:?}"
    );
    assert!(
        !hunk_text.contains("v2"),
        "expected fallback (staged) path NOT to be used; hunks mention v2: {hunk_text:?}"
    );
}

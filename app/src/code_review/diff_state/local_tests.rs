use super::*;
use crate::util::git::{
    BranchEntry, parse_range, parse_unified_diff_header, sort_branches_main_first,
};
async fn file_content_at_head(
    repo_path: &Path,
    file_path: &str,
    status: &GitFileStatus,
) -> Option<String> {
    let source =
        LocalDiffStateModel::resolve_file_content_at_head(repo_path, file_path, status).await?;
    LocalDiffStateModel::get_file_content(repo_path, &source).await
}

async fn file_content_size_at_head(
    repo_path: &Path,
    file_path: &str,
    status: &GitFileStatus,
) -> Option<usize> {
    let source =
        LocalDiffStateModel::resolve_file_content_at_head(repo_path, file_path, status).await?;
    LocalDiffStateModel::get_file_content_size(repo_path, &source).await
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
    let dir_content =
        file_content_at_head(repo_dir.path(), "nested-repo/", &GitFileStatus::Untracked).await;
    assert_eq!(dir_content, None);

    // Regular untracked files keep their empty baseline.
    let file_content =
        file_content_at_head(repo_dir.path(), "new-file.txt", &GitFileStatus::Untracked).await;
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

    let content = file_content_at_head(
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
async fn head_diff_respects_aggregate_retained_allocation_budget_boundary() {
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
    std::fs::write(repo_path.join("a.txt"), "old a\n").expect("write a.txt");
    std::fs::write(repo_path.join("b.txt"), "old b\n").expect("write b.txt");
    run_git_command(repo_path, &["add", "a.txt", "b.txt"])
        .await
        .expect("git add");
    run_git_command(repo_path, &["commit", "-m", "initial"])
        .await
        .expect("git commit");
    std::fs::write(repo_path.join("a.txt"), "new a\n").expect("modify a.txt");
    std::fs::write(repo_path.join("b.txt"), "new b\n").expect("modify b.txt");

    let first_diff = LocalDiffStateModel::get_file_diff(
        repo_path,
        "a.txt",
        &GitFileStatus::Modified,
        false,
        None,
    )
    .await
    .expect("load first file diff");
    let first_content = file_content_at_head(repo_path, "a.txt", &GitFileStatus::Modified).await;
    let first_file_bytes = approx_file_diff_bytes(&first_diff.hunks, first_content.as_ref());
    assert!(first_file_bytes < MAX_DIFF_SIZE);

    let diffs = LocalDiffStateModel::diff_state_against_head_with_limits(
        repo_path,
        first_file_bytes,
        usize::MAX,
    )
    .await
    .expect("load aggregate diff");

    assert_eq!(diffs.files.len(), 2);
    assert_eq!(diffs.files[0].file_diff.file_path, "a.txt");
    assert_eq!(diffs.files[0].content_at_head.as_deref(), Some("old a\n"));
    assert_eq!(diffs.files[1].file_diff.file_path, "b.txt");
    assert_eq!(diffs.files[1].content_at_head, None);
    assert!(diffs.files[1].file_diff.hunks.is_empty());
    assert_eq!(
        diffs.files[1].file_diff.size,
        DiffSize::Unrenderable(UnrenderableReason::FileTooLarge)
    );

    let below_boundary = LocalDiffStateModel::diff_state_against_head_with_limits(
        repo_path,
        first_file_bytes - 1,
        usize::MAX,
    )
    .await
    .expect("load aggregate diff below boundary");

    assert_eq!(
        below_boundary.files[0].file_diff.size,
        DiffSize::Unrenderable(UnrenderableReason::FileTooLarge)
    );
    assert!(below_boundary.files[0].file_diff.hunks.is_empty());
    assert_eq!(below_boundary.files[0].content_at_head, None);
}
#[tokio::test]
async fn head_diff_rejects_over_budget_base_content_with_small_patch() {
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
    let baseline = (0..2_000)
        .map(|line| format!("baseline line {line}\n"))
        .collect::<String>();
    std::fs::write(repo_path.join("large-base.txt"), &baseline).expect("write baseline");
    run_git_command(repo_path, &["add", "large-base.txt"])
        .await
        .expect("git add");
    run_git_command(repo_path, &["commit", "-m", "initial"])
        .await
        .expect("git commit");
    let modified = baseline.replacen("baseline line 1000", "changed line 1000", 1);
    std::fs::write(repo_path.join("large-base.txt"), modified).expect("modify baseline");

    let file_diff = LocalDiffStateModel::get_file_diff(
        repo_path,
        "large-base.txt",
        &GitFileStatus::Modified,
        false,
        None,
    )
    .await
    .expect("load file diff");
    let diff_bytes = approx_file_diff_bytes(&file_diff.hunks, None);
    let base_bytes =
        file_content_size_at_head(repo_path, "large-base.txt", &GitFileStatus::Modified)
            .await
            .expect("load base object size");
    assert_eq!(base_bytes, baseline.len());
    assert!(base_bytes > diff_bytes);

    let diffs = LocalDiffStateModel::diff_state_against_head_with_limits(
        repo_path,
        diff_bytes.saturating_add(base_bytes).saturating_sub(1),
        usize::MAX,
    )
    .await
    .expect("load aggregate diff");

    assert_eq!(diffs.files.len(), 1);
    assert_eq!(
        diffs.files[0].file_diff.size,
        DiffSize::Unrenderable(UnrenderableReason::FileTooLarge)
    );
    assert!(diffs.files[0].file_diff.hunks.is_empty());
    assert_eq!(diffs.files[0].content_at_head, None);
}

#[tokio::test]
async fn head_invalidation_rejects_over_budget_base_content_with_small_patch() {
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
    let baseline = (0..2_000)
        .map(|line| format!("baseline line {line}\n"))
        .collect::<String>();
    let file_path = repo_path.join("large-base.txt");
    std::fs::write(&file_path, &baseline).expect("write baseline");
    run_git_command(repo_path, &["add", "large-base.txt"])
        .await
        .expect("git add");
    run_git_command(repo_path, &["commit", "-m", "initial"])
        .await
        .expect("git commit");
    let modified = baseline.replacen("baseline line 1000", "changed line 1000", 1);
    std::fs::write(&file_path, modified).expect("modify baseline");

    let file_diff = LocalDiffStateModel::get_file_diff(
        repo_path,
        "large-base.txt",
        &GitFileStatus::Modified,
        false,
        None,
    )
    .await
    .expect("load file diff");
    let diff_bytes = approx_file_diff_bytes(&file_diff.hunks, None);
    assert!(baseline.len() > diff_bytes);
    let allowance = HeadMaterializationAllowance {
        remaining_bytes: diff_bytes.saturating_add(baseline.len()).saturating_sub(1),
        has_file_slot: true,
    };

    let (relative, diff) = LocalDiffStateModel::retrieve_diff_state(
        repo_path,
        &file_path,
        &DiffMode::Head,
        None,
        Some(allowance),
    )
    .await
    .expect("load invalidated file");
    assert_eq!(relative, "large-base.txt");
    let diff = diff.expect("changed file should remain in the diff");

    assert_eq!(
        diff.file_diff.size,
        DiffSize::Unrenderable(UnrenderableReason::FileTooLarge)
    );
    assert!(diff.file_diff.hunks.is_empty());
    assert_eq!(diff.content_at_head, None);
}

#[tokio::test]
async fn head_content_size_uses_status_specific_base_path() {
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
    let baseline = "content at the old path\n";
    std::fs::write(repo_path.join("old.txt"), baseline).expect("write baseline");
    run_git_command(repo_path, &["add", "old.txt"])
        .await
        .expect("git add");
    run_git_command(repo_path, &["commit", "-m", "initial"])
        .await
        .expect("git commit");
    std::fs::write(repo_path.join("new.txt"), "new content\n").expect("write new file");

    assert_eq!(
        file_content_size_at_head(repo_path, "old.txt", &GitFileStatus::Deleted,).await,
        Some(baseline.len())
    );
    assert_eq!(
        file_content_size_at_head(
            repo_path,
            "new.txt",
            &GitFileStatus::Renamed {
                old_path: "old.txt".to_string(),
            },
        )
        .await,
        Some(baseline.len())
    );
    assert_eq!(
        file_content_size_at_head(repo_path, "new.txt", &GitFileStatus::New,).await,
        Some(0)
    );
    assert_eq!(
        file_content_size_at_head(repo_path, "new.txt", &GitFileStatus::Untracked,).await,
        Some(0)
    );
}

#[tokio::test]
async fn resolved_head_content_uses_same_object_after_head_moves() {
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
    let original = "original baseline\n";
    std::fs::write(repo_path.join("file.txt"), original).expect("write original");
    run_git_command(repo_path, &["add", "file.txt"])
        .await
        .expect("git add original");
    run_git_command(repo_path, &["commit", "-m", "original"])
        .await
        .expect("commit original");

    let source = LocalDiffStateModel::resolve_file_content_at_head(
        repo_path,
        "file.txt",
        &GitFileStatus::Modified,
    )
    .await
    .expect("resolve original base object");

    let replacement = "a much larger replacement baseline\n";
    std::fs::write(repo_path.join("file.txt"), replacement).expect("write replacement");
    run_git_command(repo_path, &["add", "file.txt"])
        .await
        .expect("git add replacement");
    run_git_command(repo_path, &["commit", "-m", "replacement"])
        .await
        .expect("commit replacement");

    assert_eq!(
        LocalDiffStateModel::get_file_content_size(repo_path, &source).await,
        Some(original.len())
    );
    assert_eq!(
        LocalDiffStateModel::get_file_content(repo_path, &source).await,
        Some(original.to_string())
    );
}
#[test]
fn head_materialization_budget_bounds_incremental_updates() {
    let mut budget = HeadMaterializationBudget::default();

    assert!(budget.try_apply_update(
        "initial.txt",
        HeadMaterializationUpdate::Materialized(60),
        100,
        2,
    ));
    assert!(budget.try_apply_update(
        "added.txt",
        HeadMaterializationUpdate::Materialized(40),
        100,
        2,
    ));
    let replacement_allowance = budget.allowance_for_update("initial.txt", 100, 2);
    assert_eq!(replacement_allowance.remaining_bytes, 60);
    assert!(replacement_allowance.has_file_slot);
    let addition_allowance = budget.allowance_for_update("overflow.txt", 100, 2);
    assert_eq!(addition_allowance.remaining_bytes, 0);
    assert!(!addition_allowance.has_file_slot);
    assert!(!budget.try_apply_update(
        "overflow.txt",
        HeadMaterializationUpdate::Materialized(1),
        100,
        2,
    ));

    assert!(budget.try_apply_update(
        "initial.txt",
        HeadMaterializationUpdate::Materialized(20),
        100,
        2,
    ));
    assert_eq!(budget.bytes_by_path["initial.txt"], 60);
    assert!(!budget.try_apply_update(
        "added.txt",
        HeadMaterializationUpdate::Materialized(41),
        100,
        2,
    ));

    assert!(budget.try_apply_update("initial.txt", HeadMaterializationUpdate::Removed, 100, 2,));
    assert!(budget.try_apply_update(
        "overflow.txt",
        HeadMaterializationUpdate::Materialized(60),
        100,
        2,
    ));
    assert_eq!(budget.total_bytes, 100);
    assert_eq!(budget.bytes_by_path.len(), 2);
}

#[test]
fn aggregate_budget_rejects_files_after_file_limit() {
    let mut budget = DiffMaterializationBudget::new(usize::MAX, 1);

    assert!(budget.try_reserve(1));
    assert!(!budget.try_reserve(0));
}

#[test]
fn unrenderable_head_diff_drops_materialized_hunks() {
    let file_diff = FileDiff {
        file_path: "large.txt".to_string(),
        status: GitFileStatus::Modified,
        hunks: Arc::new(vec![DiffHunk {
            old_start_line: 1,
            old_line_count: 1,
            new_start_line: 1,
            new_line_count: 1,
            lines: vec![DiffLine {
                line_type: DiffLineType::Add,
                old_line_number: None,
                new_line_number: Some(1),
                text: "retained allocation".to_string(),
                no_trailing_newline: false,
            }],
            unified_diff_start: 0,
            unified_diff_end: 1,
        }]),
        is_binary: false,
        is_autogenerated: false,
        max_line_number: 1,
        has_hidden_bidi_chars: true,
        size: DiffSize::Unrenderable(UnrenderableReason::DiffTooLarge),
    };

    let file = LocalDiffStateModel::file_diff_without_materialized_content(file_diff);

    assert_eq!(file.file_diff.file_path, "large.txt");
    assert_eq!(file.file_diff.status, GitFileStatus::Modified);
    assert_eq!(
        file.file_diff.size,
        DiffSize::Unrenderable(UnrenderableReason::DiffTooLarge)
    );
    assert!(file.file_diff.hunks.is_empty());
    assert_eq!(file.file_diff.max_line_number, 0);
    assert!(!file.file_diff.has_hidden_bidi_chars);
    assert_eq!(file.content_at_head, None);
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

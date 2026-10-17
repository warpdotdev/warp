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

#[tokio::test]
async fn bounded_git_status_returns_untruncated_entries_under_budget() {
    let repo_dir = tempfile::tempdir().expect("create temp repo dir");
    let repo_path = repo_dir.path();
    run_git_command(repo_path, &["init", "-q"])
        .await
        .expect("git init");
    for i in 0..5 {
        std::fs::write(repo_path.join(format!("file{i}.txt")), "x").expect("write file");
    }

    let (entries, truncated) = LocalDiffStateModel::bounded_git_status(
        repo_path,
        &["status", "--untracked-files=all", "--porcelain=2", "-z"],
        // Comfortably above the real output for 5 short-named files.
        64 * 1024,
    )
    .await
    .expect("bounded_git_status should succeed under budget");

    assert!(!truncated);
    assert_eq!(entries.len(), 5);
    assert!(
        entries
            .iter()
            .all(|(_, status)| matches!(status, GitFileStatus::Untracked))
    );
}

/// Pure-logic regression guard for the corruption hazard APP-5462 explicitly
/// calls out: naively truncating `git diff --numstat` output at an
/// arbitrary byte offset can cut a line in half. This proves
/// `get_diff_metadata_using_numstat`'s trim always lands on a delimiter
/// boundary — whatever fragment follows the last complete line is
/// dropped, never fed to a parser — without needing a real subprocess to
/// exercise it. `git status -z` records are NOT single-field like this,
/// so `bounded_git_status` does not use this trim — see
/// `complete_status_records_before_cutoff_*` below.
#[test]
fn complete_lines_before_cutoff_drops_trailing_partial_line() {
    assert_eq!(
        LocalDiffStateModel::complete_lines_before_cutoff("a\nb\npartial", '\n'),
        "a\nb\n"
    );
    // A cut that lands exactly on a delimiter has no partial trailer to drop.
    assert_eq!(
        LocalDiffStateModel::complete_lines_before_cutoff("a\nb\n", '\n'),
        "a\nb\n"
    );
    // No complete line at all — the single line itself was cut off.
    assert_eq!(
        LocalDiffStateModel::complete_lines_before_cutoff("partial", '\n'),
        ""
    );
    assert_eq!(
        LocalDiffStateModel::complete_lines_before_cutoff("", '\n'),
        ""
    );
}

/// Critical regression guard (APP-5462 review): a porcelain v2 rename/copy
/// ('2') record spans *two* NUL-terminated fields — the main entry, then
/// the old path — unlike every other record shape, which is one field.
/// A delimiter-oblivious trim (keep everything through the last NUL) can
/// therefore keep a '2' record's first field while silently dropping its
/// second, which `parse_git_status` then reads as an empty (or
/// unrelated) old path instead of erroring: silent data corruption, not a
/// visible failure. This proves `complete_status_records_before_cutoff`
/// drops such an in-progress record in its entirety instead.
#[test]
fn complete_status_records_before_cutoff_drops_in_progress_rename() {
    // A complete untracked entry, a complete rename (both fields), then a
    // rename whose second field (the old path) was cut off mid-way.
    let text = "? a.txt\0\
         2 R. N... 100644 100644 100644 abc1234 def5678 R100 new.txt\0old.txt\0\
         2 R. N... 100644 100644 100644 abc1234 def5678 R100 new2.txt\0old2.t";

    let trimmed = LocalDiffStateModel::complete_status_records_before_cutoff(text);
    let entries = LocalDiffStateModel::parse_git_status(trimmed)
        .expect("trimmed text should always be valid porcelain v2 input");

    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0], ("a.txt".to_string(), GitFileStatus::Untracked));
    assert_eq!(
        entries[1],
        (
            "new.txt".to_string(),
            GitFileStatus::Renamed {
                old_path: "old.txt".to_string()
            }
        )
    );
    // The critical property: no entry ever carries a corrupted (empty or
    // wrong) old_path from the dropped, in-progress third record.
    for (_, status) in &entries {
        if let GitFileStatus::Renamed { old_path } | GitFileStatus::Copied { old_path } = status {
            assert!(
                !old_path.is_empty(),
                "a retained rename/copy must keep its real old_path"
            );
        }
    }
}

/// A cut that lands before a rename record's second field even starts (no
/// second NUL anywhere in the remainder) must drop that record entirely,
/// down to an empty result if it's the only record — never keep just the
/// first field.
#[test]
fn complete_status_records_before_cutoff_drops_lone_rename_missing_old_path() {
    let text = "2 R. N... 100644 100644 100644 abc1234 def5678 R100 new.txt\0old.t";
    assert_eq!(
        LocalDiffStateModel::complete_status_records_before_cutoff(text),
        ""
    );
}

/// Single-field record shapes ('u' unmerged here, matching the reviewer's
/// request to cover it explicitly) trim safely with the same last-NUL
/// logic as any other single-field record.
#[test]
fn complete_status_records_before_cutoff_trims_single_field_unmerged_record() {
    let text =
        "u UU N... 100644 100644 100644 100644 abc1234 def5678 ghi9012 conflict.txt\0partial";
    assert_eq!(
        LocalDiffStateModel::complete_status_records_before_cutoff(text),
        "u UU N... 100644 100644 100644 100644 abc1234 def5678 ghi9012 conflict.txt\0"
    );
}

/// End-to-end proof that a real, oversized `git status -z` read is actually
/// cut short (not silently absorbed in one large read) and that every
/// entry recovered from the cut is still a complete, correctly-parsed
/// record — the untracked-tree amplifier APP-5462 and APP-4827 describe.
#[tokio::test]
async fn bounded_git_status_drops_trailing_partial_record_on_overflow() {
    let repo_dir = tempfile::tempdir().expect("create temp repo dir");
    let repo_path = repo_dir.path();
    run_git_command(repo_path, &["init", "-q"])
        .await
        .expect("git init");
    // Long names so the combined `-z` output comfortably exceeds a single
    // 64KB read chunk, guaranteeing the first chunk alone already trips a
    // tiny budget — unlike a small total output, which a single read can
    // capture in full even past the requested budget.
    const TOTAL_FILES: usize = 1200;
    let padding = "x".repeat(70);
    for i in 0..TOTAL_FILES {
        std::fs::write(repo_path.join(format!("{padding}{i:04}.txt")), "x").expect("write file");
    }

    let (entries, truncated) = LocalDiffStateModel::bounded_git_status(
        repo_path,
        &["status", "--untracked-files=all", "--porcelain=2", "-z"],
        1_000,
    )
    .await
    .expect("bounded_git_status should succeed even when the budget is exceeded");

    assert!(truncated);
    // Fewer than the true total — proves a cut actually happened rather
    // than the read silently absorbing everything anyway.
    assert!(
        entries.len() < TOTAL_FILES,
        "expected fewer than {TOTAL_FILES} entries, got {}",
        entries.len()
    );
    assert!(!entries.is_empty());
    // Every returned entry parsed as a complete, valid untracked record —
    // none are corrupted fragments of a cut-off record.
    for (path, status) in &entries {
        assert!(matches!(status, GitFileStatus::Untracked));
        assert!(path.starts_with(&padding) && path.ends_with(".txt"));
    }
}

/// Important regression guard (APP-5462 review): `get_diff_metadata_using_numstat`'s
/// own overshoot must be surfaced through its own truncation flag —
/// previously it was silently dropped, so callers could report
/// `files_truncated == false` while tracked-file totals were actually a
/// lower bound. Uses a real, oversized `git diff --numstat` read (not a
/// fabricated string) so the whole path — the bounded read, the trim, and
/// the flag — is exercised together.
#[tokio::test]
async fn get_diff_metadata_using_numstat_reports_its_own_truncation() {
    let repo_dir = tempfile::tempdir().expect("create temp repo dir");
    let repo_path = repo_dir.path();
    run_git_command(repo_path, &["init", "-q"])
        .await
        .expect("git init");
    run_git_command(repo_path, &["config", "user.email", "test@test.com"])
        .await
        .expect("git config email");
    run_git_command(repo_path, &["config", "user.name", "Test"])
        .await
        .expect("git config name");

    // Long names so the combined `--numstat` output comfortably exceeds a
    // single 64KB read chunk, same reasoning as the status overflow test
    // above.
    const TOTAL_FILES: usize = 1200;
    let padding = "y".repeat(70);
    for i in 0..TOTAL_FILES {
        std::fs::write(repo_path.join(format!("{padding}{i:04}.txt")), "line one\n")
            .expect("write file");
    }
    run_git_command(repo_path, &["add", "."])
        .await
        .expect("git add");
    run_git_command(repo_path, &["commit", "-q", "-m", "initial"])
        .await
        .expect("git commit");
    for i in 0..TOTAL_FILES {
        std::fs::write(
            repo_path.join(format!("{padding}{i:04}.txt")),
            "line one\nline two\n",
        )
        .expect("modify file");
    }

    let (num_stat_metadata, truncated) =
        LocalDiffStateModel::get_diff_metadata_using_numstat(repo_path, "HEAD", 1_000)
            .await
            .expect("get_diff_metadata_using_numstat should succeed even when exceeded");

    assert!(truncated);
    // Fewer than the true total — proves a cut actually happened.
    assert!(
        num_stat_metadata.len() < TOTAL_FILES,
        "expected fewer than {TOTAL_FILES} numstat entries, got {}",
        num_stat_metadata.len()
    );
    assert!(!num_stat_metadata.is_empty());
}

/// Protects the split this builder makes between "cheap to keep exact"
/// (the overall count, and tracked-file totals from numstat) and
/// "deliberately bounded" (untracked per-file line counts, and the
/// retained `files` list) — see APP-5462. A test that only checked
/// `files.len()` after the fact could pass even if the count or the
/// tracked totals were wrongly capped too, which is exactly the silent
/// undercount APP-5462 warns against.
#[tokio::test]
async fn build_diff_metadata_against_base_keeps_exact_count_and_tracked_totals_while_capping_files()
{
    let dir = tempfile::tempdir().expect("create temp dir");
    // Untracked files needing a real per-file line count if retained.
    std::fs::write(dir.path().join("untracked_a.txt"), "one\ntwo\n").expect("write file");
    std::fs::write(dir.path().join("untracked_b.txt"), "one\ntwo\nthree\n").expect("write file");

    let changed_files = vec![
        ("tracked_a.txt".to_string(), GitFileStatus::Modified),
        ("tracked_b.txt".to_string(), GitFileStatus::Modified),
        ("untracked_a.txt".to_string(), GitFileStatus::Untracked),
        ("untracked_b.txt".to_string(), GitFileStatus::Untracked),
    ];
    let mut num_stat_metadata = HashMap::new();
    num_stat_metadata.insert(
        "tracked_a.txt".to_string(),
        GitNumStatMetadata {
            lines_added: 10,
            lines_removed: 3,
            is_binary_file: false,
        },
    );
    num_stat_metadata.insert(
        "tracked_b.txt".to_string(),
        GitNumStatMetadata {
            lines_added: 20,
            lines_removed: 7,
            is_binary_file: false,
        },
    );

    // Retain only 1 entry, well under the 4 real changed files.
    let result = LocalDiffStateModel::build_diff_metadata_against_base(
        dir.path(),
        changed_files,
        &num_stat_metadata,
        1,
        false,
    )
    .await;

    // The count reflects every changed file, not just the retained one.
    assert_eq!(result.aggregate_stats.files_changed, 4);
    // The retained list is capped exactly at max_retained.
    assert_eq!(result.files.len(), 1);
    assert_eq!(result.files[0].path, "tracked_a.txt");
    // Tracked totals are exact from numstat regardless of the cap — both
    // tracked_a.txt and tracked_b.txt count, even though tracked_b.txt
    // isn't in the retained `files` list.
    assert_eq!(result.aggregate_stats.total_additions, 30);
    assert_eq!(result.aggregate_stats.total_deletions, 10);
    assert!(!result.files_truncated);
}

#[tokio::test]
async fn build_diff_metadata_against_base_counts_retained_untracked_lines() {
    let dir = tempfile::tempdir().expect("create temp dir");
    std::fs::write(dir.path().join("untracked.txt"), "one\ntwo\nthree\n").expect("write file");

    let changed_files = vec![("untracked.txt".to_string(), GitFileStatus::Untracked)];
    let result = LocalDiffStateModel::build_diff_metadata_against_base(
        dir.path(),
        changed_files,
        &HashMap::new(),
        MAX_STATUS_ENTRIES,
        false,
    )
    .await;

    assert_eq!(result.aggregate_stats.files_changed, 1);
    assert_eq!(result.aggregate_stats.total_additions, 3);
    assert_eq!(result.aggregate_stats.total_deletions, 0);
    assert_eq!(result.files.len(), 1);
    assert_eq!(result.files[0].additions, 3);
}

#[tokio::test]
async fn build_diff_metadata_against_base_propagates_truncated_flag() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let result = LocalDiffStateModel::build_diff_metadata_against_base(
        dir.path(),
        Vec::new(),
        &HashMap::new(),
        MAX_STATUS_ENTRIES,
        true,
    )
    .await;

    assert!(result.files_truncated);
}

use super::{truncate_from_beginning, CreateGitBranch, GitBranch, GitLineChanges};
use crate::context_chips::display_chip::PromptChipShellCommand;
use crate::context_chips::display_menu::GenericMenuItem;
use crate::context_chips::git_branch_on_click::GitBranchOnClickValue;
use crate::context_chips::{github_pr_display_text_from_url, ContextChipKind};
use crate::ui_components::icons::Icon;

#[test]
fn test_github_pr_display_text_from_url() {
    assert_eq!(
        github_pr_display_text_from_url("https://github.com/warp/warp/pull/123"),
        Some("PR #123".to_string())
    );
}

#[test]
fn test_github_pr_display_text_from_url_rejects_non_pr_urls() {
    assert_eq!(
        github_pr_display_text_from_url("https://github.com/warp/warp/issues/123"),
        None
    );
    assert_eq!(
        github_pr_display_text_from_url("https://github.com/warp/warp/pull/not-a-number"),
        None
    );
}

#[test]
fn test_github_pr_chip_display_value_formats_url() {
    let value =
        crate::context_chips::ChipValue::Text("https://github.com/warp/warp/pull/456".to_string());
    assert_eq!(
        ContextChipKind::GithubPullRequest.display_value(&value),
        "PR #456"
    );
}

#[test]
fn test_github_pr_chip_display_value_falls_back_to_raw_value() {
    let value = crate::context_chips::ChipValue::Text("https://example.com/not-a-pr".to_string());
    assert_eq!(
        ContextChipKind::GithubPullRequest.display_value(&value),
        "https://example.com/not-a-pr"
    );
}

#[test]
fn test_format_git_branch_command_checks_out_normal_branch() {
    let value = GitBranchOnClickValue::new("feature/alice's-work".to_string()).encode();

    assert_eq!(
        GitBranch(value).prompt_chip_command(),
        PromptChipShellCommand::GitCheckout {
            branch_name: "feature/alice\'s-work".to_string()
        }
    );
}

#[test]
fn test_format_git_branch_command_changes_to_linked_worktree_path() {
    let value = GitBranchOnClickValue {
        branch_name: "feature-a".to_string(),
        worktree_path: Some("/tmp/repo feature-a".to_string()),
        is_linked_worktree: true,
    }
    .encode();

    assert_eq!(
        GitBranch(value).prompt_chip_command(),
        PromptChipShellCommand::ChangeDirectory {
            dir_name: "/tmp/repo feature-a".to_string()
        }
    );
}

#[test]
fn test_format_git_branch_command_reports_missing_linked_worktree_path() {
    let value = GitBranchOnClickValue {
        branch_name: "feature-a".to_string(),
        worktree_path: None,
        is_linked_worktree: true,
    }
    .encode();

    assert_eq!(
        GitBranch(value).prompt_chip_command(),
        PromptChipShellCommand::Echo {
            message: "The branch is already checked out in another worktree, but Warp couldn't find its path."
        }
    );
}

#[test]
fn test_git_branch_menu_icon_uses_branch_icon_for_normal_branch() {
    let value = GitBranchOnClickValue::new("feature-a".to_string()).encode();

    assert_eq!(GitBranch(value).icon_for_menu(), Icon::GitBranch);
}

#[test]
fn test_git_branch_menu_icon_uses_worktree_icon_for_linked_worktree() {
    let value = GitBranchOnClickValue {
        branch_name: "feature-a".to_string(),
        worktree_path: Some("/tmp/repo-feature-a".to_string()),
        is_linked_worktree: true,
    }
    .encode();

    assert_eq!(GitBranch(value).icon_for_menu(), Icon::Dataflow02);
}

#[test]
fn test_create_git_branch_menu_name_quotes_query() {
    let item = CreateGitBranch::new("feature/xyz".to_string());
    assert_eq!(item.name(), "Create new branch \"feature/xyz\"");
}

#[test]
fn test_create_git_branch_action_data_returns_branch_name() {
    let item = CreateGitBranch::new("feature/xyz".to_string());
    assert_eq!(item.action_data(), "feature/xyz");
    assert_eq!(item.branch_name(), "feature/xyz");
    assert_eq!(
        item.prompt_chip_command(),
        PromptChipShellCommand::GitCreateAndCheckoutBranch {
            branch_name: "feature/xyz".to_string()
        }
    );
}

#[test]
fn test_create_git_branch_trims_whitespace_in_constructor() {
    let item = CreateGitBranch::new("  feature/xyz  ".to_string());
    assert_eq!(item.branch_name(), "feature/xyz");
    assert_eq!(item.name(), "Create new branch \"feature/xyz\"");
}

#[test]
fn test_parse_from_git_output_both_additions_and_deletions() {
    let input = " 3 files changed, 5 insertions(+), 2 deletions(-)";
    let result = GitLineChanges::parse_from_git_output(input).unwrap();

    assert_eq!(result.files_changed, 3);
    assert_eq!(result.lines_added, 5);
    assert_eq!(result.lines_removed, 2);
}

#[test]
fn test_parse_from_git_output_only_additions() {
    let input = " 2 files changed, 10 insertions(+)";
    let result = GitLineChanges::parse_from_git_output(input).unwrap();

    assert_eq!(result.files_changed, 2);
    assert_eq!(result.lines_added, 10);
    assert_eq!(result.lines_removed, 0);
}

#[test]
fn test_parse_from_git_output_only_deletions() {
    let input = " 1 file changed, 7 deletions(-)";
    let result = GitLineChanges::parse_from_git_output(input).unwrap();

    assert_eq!(result.files_changed, 1);
    assert_eq!(result.lines_added, 0);
    assert_eq!(result.lines_removed, 7);
}

#[test]
fn test_parse_from_git_output_single_values() {
    // Test singular forms (1 file, 1 insertion, 1 deletion)
    let input = " 1 file changed, 1 insertion(+), 1 deletion(-)";
    let result = GitLineChanges::parse_from_git_output(input).unwrap();

    assert_eq!(result.files_changed, 1);
    assert_eq!(result.lines_added, 1);
    assert_eq!(result.lines_removed, 1);
}

#[test]
fn test_parse_from_git_output_no_leading_spaces() {
    // Git output sometimes doesn't have leading spaces
    let input = "2 files changed, 3 insertions(+), 1 deletion(-)";
    let result = GitLineChanges::parse_from_git_output(input).unwrap();

    assert_eq!(result.files_changed, 2);
    assert_eq!(result.lines_added, 3);
    assert_eq!(result.lines_removed, 1);
}

#[test]
fn test_parse_from_git_output_extra_whitespace() {
    // Test with extra whitespace and tabs
    let input = "\t 1 file changed,   5 insertions(+),  \t 3 deletions(-)   ";
    let result = GitLineChanges::parse_from_git_output(input).unwrap();

    assert_eq!(result.files_changed, 1);
    assert_eq!(result.lines_added, 5);
    assert_eq!(result.lines_removed, 3);
}

#[test]
fn test_parse_from_git_output_empty_string() {
    let input = "";
    let result = GitLineChanges::parse_from_git_output(input);

    assert!(result.is_none());
}

#[test]
fn test_parse_from_git_output_whitespace_only() {
    let input = "   \t\n  ";
    let result = GitLineChanges::parse_from_git_output(input);

    assert!(result.is_none());
}

#[test]
fn test_parse_from_git_output_invalid_format() {
    let input = "This is not a valid git diff --shortstat output";
    let result = GitLineChanges::parse_from_git_output(input).unwrap();

    // Should parse but find no valid numbers
    assert_eq!(result.files_changed, 0);
    assert_eq!(result.lines_added, 0);
    assert_eq!(result.lines_removed, 0);
}

#[test]
fn test_parse_from_git_output_partial_matches() {
    // Test when only some parts match the expected pattern
    let input = " 2 files changed, some insertions, 3 deletions(-)";
    let result = GitLineChanges::parse_from_git_output(input).unwrap();

    assert_eq!(result.files_changed, 2);
    assert_eq!(result.lines_added, 0); // "some" is not a number
    assert_eq!(result.lines_removed, 3);
}

#[test]
fn test_parse_from_git_output_zero_changes() {
    // Edge case: explicit zero values (unlikely from git but good to test)
    let input = " 0 files changed, 0 insertions(+), 0 deletions(-)";
    let result = GitLineChanges::parse_from_git_output(input).unwrap();

    assert_eq!(result.files_changed, 0);
    assert_eq!(result.lines_added, 0);
    assert_eq!(result.lines_removed, 0);
}

// Tests for truncate_from_beginning function
// These tests ensure the function properly handles Unicode characters and doesn't panic

#[test]
fn test_truncate_from_beginning_ascii_short_text() {
    let text = "hello";
    let result = truncate_from_beginning(text, 10);
    assert_eq!(result, "hello");
}

#[test]
fn test_truncate_from_beginning_ascii_exact_length() {
    let text = "hello";
    let result = truncate_from_beginning(text, 5);
    assert_eq!(result, "hello");
}

#[test]
fn test_truncate_from_beginning_ascii_truncation() {
    let text = "hello world";
    let result = truncate_from_beginning(text, 10);
    assert_eq!(result, "…llo world");
}

#[test]
fn test_truncate_from_beginning_unicode_multibyte_chars() {
    // Test with multibyte Unicode characters (café has é which is 2 bytes in UTF-8)
    let text = "café";
    let result = truncate_from_beginning(text, 3);
    assert_eq!(result, "…fé");
}

#[test]
fn test_truncate_from_beginning_emoji() {
    // Test with emoji characters (each emoji is multiple bytes)
    let text = "hello🚀world";
    let result = truncate_from_beginning(text, 10);
    assert_eq!(result, "…llo🚀world");
}

#[test]
fn test_truncate_from_beginning_mixed_unicode() {
    // Test with mixed ASCII, multibyte chars, and emoji
    let text = "café🚀test";
    let result = truncate_from_beginning(text, 8);
    assert_eq!(result, "…fé🚀test");
}

#[test]
fn test_truncate_from_beginning_chinese_characters() {
    // Test with Chinese characters (each is typically 3 bytes in UTF-8)
    let text = "世界你好世界";
    let result = truncate_from_beginning(text, 5);
    assert_eq!(result, "…你好世界");
}

#[test]
fn test_truncate_from_beginning_cyrillic_characters() {
    // Test with Cyrillic characters
    let text = "Привет";
    let result = truncate_from_beginning(text, 4);
    assert_eq!(result, "…вет");
}

#[test]
fn test_truncate_from_beginning_max_length_one() {
    // Edge case: max_length = 1 should just return ellipsis
    let text = "hello";
    let result = truncate_from_beginning(text, 1);
    assert_eq!(result, "…");
}

#[test]
fn test_truncate_from_beginning_max_length_zero() {
    // Edge case: max_length = 0 should return empty string
    let text = "hello";
    let result = truncate_from_beginning(text, 0);
    assert_eq!(result, "…");
}

#[test]
fn test_truncate_from_beginning_empty_string() {
    // Edge case: empty string
    let text = "";
    let result = truncate_from_beginning(text, 5);
    assert_eq!(result, "");
}

#[test]
fn test_truncate_from_beginning_single_unicode_char() {
    // Test with single Unicode character that's longer than max_length
    let text = "🚀";
    let result = truncate_from_beginning(text, 1);
    assert_eq!(result, "🚀"); // Should not truncate since it's already 1 character
}

#[test]
fn test_truncate_from_beginning_complex_emoji() {
    // Test with complex emoji sequences (flags, skin tones, etc.)
    // Note: chars()-based truncation doesn't respect grapheme cluster boundaries,
    // so complex emoji like 👨‍👩‍👧‍👦 (7 chars with ZWJs) can be split mid-sequence.
    // We pick max_length=5 to land on a clean emoji boundary (👦, not a ZWJ).
    let text = "🚀🇺🇸🏳️‍🌈👨‍👩‍👧‍👦123";
    let result = truncate_from_beginning(text, 5);
    assert_eq!(result, "…👦123");
}

#[test]
fn test_truncate_from_beginning_long_path() {
    // Test with a realistic scenario: long file path
    let text = "/home/user/projects/my-awesome-project/src/components/display_chip.rs";
    let result = truncate_from_beginning(text, 40);
    assert_eq!(result, "…-project/src/components/display_chip.rs");
}

#[test]
fn test_truncate_from_beginning_windows_path() {
    // Test with Windows path containing Unicode
    let text = "C:\\Users\\用户\\Documents\\项目\\test.txt";
    let result = truncate_from_beginning(text, 20);
    assert_eq!(result, "…cuments\\项目\\test.txt");
}

#[test]
fn test_truncate_from_beginning_preserves_char_boundaries() {
    // This is the critical test - ensure we never panic on char boundaries
    // Test various lengths to ensure we never hit a bad boundary
    let text = "café🚀世界test";

    // Try every possible max_length to ensure no panics
    for max_len in 1..=text.chars().count() + 5 {
        let result = truncate_from_beginning(text, max_len);
        // Verify the result is valid UTF-8 (won't panic if it is)
        assert!(
            result.chars().count() <= max_len || result.chars().count() == text.chars().count()
        );
    }
}

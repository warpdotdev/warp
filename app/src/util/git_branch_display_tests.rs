use settings_value::SettingsValue;

use super::{GitBranchNameTruncation, format_git_branch_name, git_branch_name_pattern_is_valid};
use crate::util::truncation::truncate_from_beginning;

const LONG_BRANCH: &str = "ABC-123-some-long-feature-that-needs-to-be-implemented";

#[test]
fn end_truncation_matches_existing_beginning_truncation() {
    let displayed = format_git_branch_name(LONG_BRANCH, GitBranchNameTruncation::End, "", 40);
    assert_eq!(displayed, truncate_from_beginning(LONG_BRANCH, 40));
    assert!(!displayed.contains("ABC-123"));
    assert!(displayed.starts_with('…'));
}

#[test]
fn start_truncation_keeps_the_ticket_prefix() {
    let displayed = format_git_branch_name(LONG_BRANCH, GitBranchNameTruncation::Start, "", 40);
    assert!(displayed.starts_with("ABC-123"));
    assert!(displayed.ends_with('…'));
    assert_eq!(displayed.chars().count(), 40);
}

#[test]
fn short_names_are_not_truncated() {
    let branch = "main";
    assert_eq!(
        format_git_branch_name(branch, GitBranchNameTruncation::End, "", 40),
        branch
    );
    assert_eq!(
        format_git_branch_name(branch, GitBranchNameTruncation::Start, "", 40),
        branch
    );
}

#[test]
fn capture_group_shows_only_the_ticket_id() {
    let displayed = format_git_branch_name(
        LONG_BRANCH,
        GitBranchNameTruncation::End,
        r"^([A-Z]+-\d+)",
        40,
    );
    assert_eq!(displayed, "ABC-123");
}

#[test]
fn pattern_without_groups_shows_the_whole_match() {
    let displayed =
        format_git_branch_name(LONG_BRANCH, GitBranchNameTruncation::Start, r"ABC-\d+", 40);
    assert_eq!(displayed, "ABC-123");
}

#[test]
fn unmatched_pattern_falls_back_to_the_full_name() {
    let displayed = format_git_branch_name(
        LONG_BRANCH,
        GitBranchNameTruncation::Start,
        r"^TICKET-(\d+)",
        40,
    );
    assert_eq!(
        displayed,
        format_git_branch_name(LONG_BRANCH, GitBranchNameTruncation::Start, "", 40)
    );
}

#[test]
fn invalid_pattern_falls_back_to_the_full_name() {
    let displayed =
        format_git_branch_name(LONG_BRANCH, GitBranchNameTruncation::End, r"(unclosed", 40);
    assert_eq!(
        displayed,
        format_git_branch_name(LONG_BRANCH, GitBranchNameTruncation::End, "", 40)
    );
}

#[test]
fn whitespace_around_a_pattern_is_ignored() {
    let displayed = format_git_branch_name(
        LONG_BRANCH,
        GitBranchNameTruncation::End,
        "  ^([A-Z]+-\\d+)  ",
        40,
    );
    assert_eq!(displayed, "ABC-123");
}

#[test]
fn unmatched_capture_group_keeps_the_full_name() {
    let displayed = format_git_branch_name(
        "feature-without-ticket",
        GitBranchNameTruncation::End,
        r"^([A-Z]+-\d+)?",
        40,
    );
    assert_eq!(displayed, "feature-without-ticket");
}

#[test]
fn unicode_start_truncation_keeps_char_boundaries() {
    let branch = "票-1-很长的分支名字需要被截断才能放进提示";
    let displayed = format_git_branch_name(branch, GitBranchNameTruncation::Start, "", 8);
    assert!(displayed.starts_with("票-1"));
    assert!(displayed.ends_with('…'));
    assert_eq!(displayed.chars().count(), 8);
    assert!(branch.starts_with(displayed.trim_end_matches('…')));
}

#[test]
fn pattern_validity_accepts_empty_and_rejects_broken_regex() {
    assert!(git_branch_name_pattern_is_valid(""));
    assert!(git_branch_name_pattern_is_valid("   "));
    assert!(git_branch_name_pattern_is_valid(r"^([A-Z]+-\d+)"));
    assert!(!git_branch_name_pattern_is_valid(r"(unclosed"));
    assert!(!git_branch_name_pattern_is_valid(&"a".repeat(257)));
}

#[test]
fn truncation_serializes_as_snake_case() {
    assert_eq!(
        GitBranchNameTruncation::default(),
        GitBranchNameTruncation::End
    );
    assert_eq!(
        GitBranchNameTruncation::End.to_file_value(),
        serde_json::json!("end")
    );
    assert_eq!(
        GitBranchNameTruncation::Start.to_file_value(),
        serde_json::json!("start")
    );
    assert_eq!(
        GitBranchNameTruncation::from_file_value(&serde_json::json!("start")).unwrap(),
        GitBranchNameTruncation::Start
    );
    assert!(GitBranchNameTruncation::from_file_value(&serde_json::json!("middle")).is_none());
}

#[test]
fn toggled_switches_between_ends() {
    assert_eq!(
        GitBranchNameTruncation::End.toggled(),
        GitBranchNameTruncation::Start
    );
    assert_eq!(
        GitBranchNameTruncation::Start.toggled(),
        GitBranchNameTruncation::End
    );
}

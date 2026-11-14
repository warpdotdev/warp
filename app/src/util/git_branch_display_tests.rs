use settings_value::SettingsValue;

use super::{
    DEFAULT_GIT_BRANCH_NAME_PREFIX_CHARS, DEFAULT_GIT_BRANCH_NAME_SUFFIX_CHARS,
    GitBranchNameTruncation, format_git_branch_name,
};
use crate::util::truncation::{truncate_from_beginning, truncate_from_end};

const LONG_BRANCH: &str = "ABC-123-some-long-feature-that-needs-to-be-implemented";

fn format(
    branch: &str,
    truncation: GitBranchNameTruncation,
    prefix_chars: usize,
    suffix_chars: usize,
    max_chars: usize,
) -> String {
    format_git_branch_name(branch, truncation, prefix_chars, suffix_chars, max_chars)
}

#[test]
fn omit_start_matches_existing_beginning_truncation() {
    let displayed = format(LONG_BRANCH, GitBranchNameTruncation::OmitStart, 12, 12, 40);
    assert_eq!(displayed, truncate_from_beginning(LONG_BRANCH, 40));
    assert!(displayed.starts_with('…'));
    assert!(!displayed.contains("ABC-123"));
}

#[test]
fn omit_end_keeps_the_ticket_prefix_and_the_ellipsis() {
    let displayed = format(LONG_BRANCH, GitBranchNameTruncation::OmitEnd, 3, 3, 40);
    assert_eq!(displayed, truncate_from_end(LONG_BRANCH, 40));
    assert!(displayed.starts_with("ABC-123"));
    assert!(displayed.ends_with('…'));
    assert_eq!(displayed.chars().count(), 40);
}

#[test]
fn edge_counts_do_not_change_start_or_end_omission() {
    let with_defaults = format(LONG_BRANCH, GitBranchNameTruncation::OmitStart, 12, 12, 40);
    let with_other_counts = format(LONG_BRANCH, GitBranchNameTruncation::OmitStart, 1, 30, 40);
    assert_eq!(with_defaults, with_other_counts);
}

#[test]
fn short_names_keep_every_character() {
    let branch = "main";
    for truncation in [
        GitBranchNameTruncation::OmitStart,
        GitBranchNameTruncation::OmitEnd,
        GitBranchNameTruncation::OmitMiddle,
    ] {
        assert_eq!(format(branch, truncation, 12, 12, 40), branch);
    }
}

#[test]
fn omit_middle_keeps_both_edges_and_the_ellipsis() {
    let displayed = format(
        LONG_BRANCH,
        GitBranchNameTruncation::OmitMiddle,
        DEFAULT_GIT_BRANCH_NAME_PREFIX_CHARS,
        DEFAULT_GIT_BRANCH_NAME_SUFFIX_CHARS,
        40,
    );
    assert_eq!(displayed, "ABC-123-some…-implemented");
    assert!(displayed.contains('…'));
    assert_ne!(displayed, LONG_BRANCH);
}

#[test]
fn omit_middle_shows_the_name_when_the_edges_cover_it() {
    let branch = "ABC-123-short";
    assert_eq!(
        format(branch, GitBranchNameTruncation::OmitMiddle, 8, 8, 40),
        branch
    );
}

#[test]
fn omit_middle_shrinks_the_suffix_before_the_prefix() {
    let displayed = format(LONG_BRANCH, GitBranchNameTruncation::OmitMiddle, 30, 30, 40);
    assert_eq!(displayed.chars().count(), 40);
    let without_ellipsis: String = displayed.chars().filter(|c| *c != '…').collect();
    assert_eq!(without_ellipsis.chars().count(), 39);
    assert!(displayed.starts_with(&LONG_BRANCH.chars().take(30).collect::<String>()));
    assert!(LONG_BRANCH.ends_with(&displayed.chars().skip(31).collect::<String>()));
    assert_eq!(displayed.chars().nth(30), Some('…'));
}

#[test]
fn omit_middle_with_no_edge_characters_is_an_ellipsis() {
    assert_eq!(
        format(LONG_BRANCH, GitBranchNameTruncation::OmitMiddle, 0, 0, 40),
        "…"
    );
}

#[test]
fn unicode_omit_end_keeps_char_boundaries() {
    let branch = "票-1-很长的分支名字需要被截断才能放进提示";
    let displayed = format(branch, GitBranchNameTruncation::OmitEnd, 0, 0, 8);
    assert!(displayed.starts_with("票-1"));
    assert!(displayed.ends_with('…'));
    assert_eq!(displayed.chars().count(), 8);
    assert!(branch.starts_with(displayed.trim_end_matches('…')));
}

#[test]
fn unicode_omit_middle_keeps_char_boundaries() {
    let branch = "票-1-很长的分支名字需要被截断才能放进提示";
    let displayed = format(branch, GitBranchNameTruncation::OmitMiddle, 3, 2, 40);
    assert_eq!(displayed, "票-1…提示");
}

#[test]
fn truncation_serializes_as_snake_case() {
    assert_eq!(
        GitBranchNameTruncation::default(),
        GitBranchNameTruncation::OmitStart
    );
    assert_eq!(
        GitBranchNameTruncation::OmitStart.to_file_value(),
        serde_json::json!("omit_start")
    );
    assert_eq!(
        GitBranchNameTruncation::OmitEnd.to_file_value(),
        serde_json::json!("omit_end")
    );
    assert_eq!(
        GitBranchNameTruncation::OmitMiddle.to_file_value(),
        serde_json::json!("omit_middle")
    );
    assert_eq!(
        GitBranchNameTruncation::from_file_value(&serde_json::json!("omit_middle")).unwrap(),
        GitBranchNameTruncation::OmitMiddle
    );
    assert!(GitBranchNameTruncation::from_file_value(&serde_json::json!("end")).is_none());
}

#[test]
fn toggled_switches_between_hiding_the_start_and_the_end() {
    assert_eq!(
        GitBranchNameTruncation::OmitStart.toggled(),
        GitBranchNameTruncation::OmitEnd
    );
    assert_eq!(
        GitBranchNameTruncation::OmitEnd.toggled(),
        GitBranchNameTruncation::OmitStart
    );
    assert_eq!(
        GitBranchNameTruncation::OmitMiddle.toggled(),
        GitBranchNameTruncation::OmitEnd
    );
}

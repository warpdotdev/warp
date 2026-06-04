use super::*;

#[test]
fn test_git_branch_on_click_value_round_trips_through_encode_decode() {
    let values = [
        GitBranchOnClickValue::new("feature-a".to_string()),
        GitBranchOnClickValue::linked_worktree(
            "feature-b".to_string(),
            Some("/repo/feature-b".to_string()),
        ),
        GitBranchOnClickValue::linked_worktree("feature-c".to_string(), None),
    ];

    for value in values {
        assert_eq!(GitBranchOnClickValue::decode(&value.encode()), value);
    }
}

#[test]
fn test_git_branch_on_click_value_decode_uses_branch_name_for_unknown_payload() {
    let value =
        format!("feature-a{ENCODED_VALUE_SEPARATOR}unknown{ENCODED_VALUE_SEPARATOR}metadata");

    assert_eq!(
        GitBranchOnClickValue::decode(&value),
        GitBranchOnClickValue::new("feature-a".to_string())
    );
}

#[test]
fn test_git_branch_on_click_values_resolve_linked_worktree_paths() {
    let values = Some(vec![
        "  feature-a".to_string(),
        "+ linked-worktree".to_string(),
        "* main".to_string(),
        "".to_string(),
        "  +literal-plus".to_string(),
        WORKTREE_LIST_SEPARATOR.to_string(),
        "worktree /repo".to_string(),
        "branch refs/heads/main".to_string(),
        "".to_string(),
        "worktree /repo-linked".to_string(),
        "branch refs/heads/linked-worktree".to_string(),
    ]);

    let values = filter_git_branch_on_click_values(values).unwrap();
    let values: Vec<_> = values
        .iter()
        .map(|value| GitBranchOnClickValue::decode(value))
        .collect();

    assert_eq!(
        values,
        vec![
            GitBranchOnClickValue::new("main".to_string()),
            GitBranchOnClickValue::new("feature-a".to_string()),
            GitBranchOnClickValue::linked_worktree(
                "linked-worktree".to_string(),
                Some("/repo-linked".to_string()),
            ),
            GitBranchOnClickValue::new("+literal-plus".to_string()),
        ]
    );
}

#[test]
fn test_git_branch_on_click_values_keep_linked_marker_without_path() {
    let values = filter_git_branch_on_click_values(Some(vec!["+ feature".to_string()]))
        .expect("expected parsed branch values");
    let value = GitBranchOnClickValue::decode(&values[0]);

    assert_eq!(value.branch_name, "feature");
    assert_eq!(value.worktree_path, None);
    assert!(value.is_linked_worktree);
}

#[test]
fn test_is_plausible_new_branch_name_accepts_typical_names() {
    for name in [
        "feature/xyz",
        "fix-123",
        "release/v1.2.3",
        "user/alice/work",
        "main",
    ] {
        assert!(
            is_plausible_new_branch_name(name),
            "expected {name:?} to be accepted",
        );
    }
}

#[test]
fn test_is_plausible_new_branch_name_rejects_empty_or_whitespace() {
    for name in ["", "   ", "\t\n"] {
        assert!(
            !is_plausible_new_branch_name(name),
            "expected {name:?} to be rejected",
        );
    }
}

#[test]
fn test_is_plausible_new_branch_name_rejects_leading_dash() {
    assert!(!is_plausible_new_branch_name("-foo"));
    assert!(!is_plausible_new_branch_name("--all"));
}

#[test]
fn test_is_plausible_new_branch_name_rejects_internal_whitespace() {
    assert!(!is_plausible_new_branch_name("my branch"));
    assert!(!is_plausible_new_branch_name("foo\tbar"));
}

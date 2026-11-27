use super::is_valid_worktree_branch_name;

#[test]
fn accepts_common_branch_names() {
    for name in [
        "my-feature",
        "feature/login",
        "fix/JIRA-123_crash",
        "release/v1.2",
        "  padded  ",
    ] {
        assert!(
            is_valid_worktree_branch_name(name),
            "{name} should be valid"
        );
    }
}

#[test]
fn rejects_names_git_or_the_shell_would_choke_on() {
    for name in [
        "",
        "   ",
        "-starts-with-dash",
        "/leading-slash",
        "trailing-slash/",
        "double//slash",
        "has..dots",
        "ends-with-dot.",
        "feature/.hidden",
        "branch.lock",
        "feature/x.lock/y",
        "has space",
        "quote\"name",
        "semi;colon",
        "dollar$sign",
    ] {
        assert!(
            !is_valid_worktree_branch_name(name),
            "{name} should be invalid"
        );
    }
}

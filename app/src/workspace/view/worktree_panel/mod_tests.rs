use anyhow::anyhow;

use super::git_error_message;

#[test]
fn git_error_message_extracts_fatal_reason() {
    let error = anyhow!(
        "Git command failed: fatal: '/tmp/wt' contains modified or untracked files, use --force to delete it\n, "
    );
    assert_eq!(
        git_error_message(&error),
        "'/tmp/wt' contains modified or untracked files, use --force to delete it"
    );
}

#[test]
fn git_error_message_falls_back_to_full_message() {
    let error = anyhow!("Failed to execute git command: not found");
    assert_eq!(
        git_error_message(&error),
        "Failed to execute git command: not found"
    );
}

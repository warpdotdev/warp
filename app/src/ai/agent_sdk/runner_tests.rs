use super::confirm_delete;

#[test]
fn confirm_delete_refuses_non_interactive_without_force() {
    // In non-interactive mode, refusal must surface as an error so the caller
    // exits non-zero instead of treating a skipped delete as a success.
    let err = confirm_delete("runner-123", false).expect_err("non-interactive refusal is an error");
    let msg = err.to_string();
    assert!(msg.contains("non-interactive"), "got: {msg}");
    assert!(msg.contains("runner-123"), "got: {msg}");
}

use super::{build_ssh_failure_banner_error, MAX_RAW_TAIL_CHARS};

// ── classified path ─────────────────────────────────────────────────

#[test]
fn auth_publickey_rejected_uses_classified_label_as_body() {
    let banner = build_ssh_failure_banner_error("", Some("Permission denied (publickey)."));
    assert_eq!(banner.body, "Public-key auth rejected");
    let detail = banner.detail.expect("detail set on classified mode");
    assert!(
        detail.starts_with("Make sure your key is loaded"),
        "detail should lead with the suggested fix, got: {detail}",
    );
    assert!(detail.contains("SSH output:"));
    assert!(detail.contains("Permission denied (publickey)."));
}

#[test]
fn classified_detail_includes_error_field_when_present() {
    let banner =
        build_ssh_failure_banner_error("Failed to start daemon", Some("Connection refused"));
    let detail = banner.detail.expect("detail");
    assert_eq!(banner.body, "SSH port closed");
    assert!(detail.contains("Error: Failed to start daemon"));
    assert!(detail.contains("SSH output:\nConnection refused"));
}

#[test]
fn classifier_sees_combined_text_when_only_error_field_carries_signal() {
    // If `proxy_stderr` is empty but the manager-built `error` text
    // happens to embed an ssh-pattern phrase, classification should
    // still fire. (Edge case: some intermediate layers concatenate
    // stderr into the error string before it hits the banner.)
    let banner = build_ssh_failure_banner_error(
        "client_loop: send disconnect: Broken pipe — daemon teardown",
        None,
    );
    assert_eq!(banner.body, "Connection dropped");
    let detail = banner.detail.expect("detail");
    assert!(detail.contains("TCP stream was torn down"));
}

#[test]
fn host_key_changed_surfaces_strong_warning_label() {
    let stderr = "@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@\n\
                  @    WARNING: REMOTE HOST IDENTIFICATION HAS CHANGED!     @\n\
                  @@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@\n\
                  Host key verification failed.";
    let banner = build_ssh_failure_banner_error("", Some(stderr));
    // The "CHANGED" variant beats the generic mismatch — this is
    // the MitM-warning path, not the cleared-known_hosts path.
    assert_eq!(banner.body, "Host key CHANGED — investigate");
}

#[test]
fn identity_file_bad_permissions_wins_over_downstream_auth_failure() {
    // openssh prints both the UNPROTECTED PRIVATE KEY FILE banner
    // AND a Permission denied later in the same blob. The
    // user-actionable cause is the file permissions, so the body
    // should reflect that rather than the auth failure.
    let stderr = "@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@\n\
                  @         WARNING: UNPROTECTED PRIVATE KEY FILE!          @\n\
                  @@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@\n\
                  Permissions 0644 for '/Users/me/.ssh/id_ed25519' are too open.\n\
                  Permission denied (publickey).";
    let banner = build_ssh_failure_banner_error("", Some(stderr));
    assert_eq!(banner.body, "Identity file permissions too open");
}

#[test]
fn classified_detail_truncates_very_long_stderr() {
    let long = "a".repeat(MAX_RAW_TAIL_CHARS + 200);
    let stderr = format!("Permission denied (publickey).\n{long}");
    let banner = build_ssh_failure_banner_error("", Some(&stderr));
    let detail = banner.detail.expect("detail");

    // The SSH-output block should be capped — verify the trailing
    // ellipsis is present and the body is bounded.
    assert!(detail.contains("SSH output:"));
    assert!(detail.contains('…'), "expected truncation marker");

    let after_marker = detail
        .split("SSH output:\n")
        .nth(1)
        .expect("SSH output section");
    // +1 for the trailing `…`, allow a small slack.
    assert!(
        after_marker.chars().count() <= MAX_RAW_TAIL_CHARS + 1,
        "truncated body length {} too long",
        after_marker.chars().count()
    );
}

// ── fallback path ───────────────────────────────────────────────────

#[test]
fn unrecognized_stderr_falls_back_to_generic_body() {
    let banner =
        build_ssh_failure_banner_error("Some unmatched error blob", Some("daemon ate my homework"));
    assert_eq!(banner.body, "Failed to start SSH extension");
    // Fallback uses the bare `error` field as detail, matching the
    // pre-R3.6 banner shape exactly so this change is purely
    // additive for unclassified errors.
    assert_eq!(banner.detail.as_deref(), Some("Some unmatched error blob"));
}

#[test]
fn empty_error_and_no_stderr_yields_no_detail() {
    let banner = build_ssh_failure_banner_error("", None);
    assert_eq!(banner.body, "Failed to start SSH extension");
    assert!(banner.detail.is_none());
}

#[test]
fn empty_error_with_unclassifiable_stderr_falls_back_with_none_detail() {
    // The fallback intentionally drops `proxy_stderr` when the
    // error field is empty — preserves the existing banner copy.
    // Adjusting that policy is a follow-up; the test pins current
    // behavior so a future refactor surfaces it.
    let banner = build_ssh_failure_banner_error("", Some("daemon ate my homework"));
    assert_eq!(banner.body, "Failed to start SSH extension");
    assert_eq!(banner.detail, None);
}

#[test]
fn empty_stderr_treated_as_none_for_classification() {
    // Some call sites pass `Some("")` rather than `None` when the
    // stderr drain was zero-length. Classification should treat
    // both the same way.
    let banner_some_empty =
        build_ssh_failure_banner_error("Permission denied (publickey).", Some(""));
    assert_eq!(banner_some_empty.body, "Public-key auth rejected");

    let banner_none = build_ssh_failure_banner_error("Permission denied (publickey).", None);
    assert_eq!(banner_none.body, "Public-key auth rejected");
}

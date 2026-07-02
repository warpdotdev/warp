use super::{classify_ssh_stderr, ConnectionFailureMode};

// ── classify_ssh_stderr ─────────────────────────────────────────────

#[test]
fn empty_stderr_classifies_as_none() {
    assert!(classify_ssh_stderr("").is_none());
}

#[test]
fn unrecognized_stderr_classifies_as_none() {
    assert!(classify_ssh_stderr("the daemon ate my homework").is_none());
}

#[test]
fn hostname_unresolved() {
    let s = "ssh: Could not resolve hostname prod-was-typo: nodename nor servname provided, or not known";
    assert_eq!(
        classify_ssh_stderr(s),
        Some(ConnectionFailureMode::HostnameUnresolved)
    );
}

#[test]
fn network_unreachable_via_no_route() {
    let s = "ssh: connect to host 10.0.0.5 port 22: No route to host";
    assert_eq!(
        classify_ssh_stderr(s),
        Some(ConnectionFailureMode::NetworkUnreachable)
    );
}

#[test]
fn network_unreachable_via_unreachable_text() {
    let s = "ssh: connect to host 10.0.0.5 port 22: Network is unreachable";
    assert_eq!(
        classify_ssh_stderr(s),
        Some(ConnectionFailureMode::NetworkUnreachable)
    );
}

#[test]
fn port_closed() {
    let s = "ssh: connect to host prod-web port 22: Connection refused";
    assert_eq!(
        classify_ssh_stderr(s),
        Some(ConnectionFailureMode::PortClosed)
    );
}

#[test]
fn connect_timeout_plain_form() {
    let s = "ssh: connect to host prod-web port 22: Operation timed out";
    assert_eq!(
        classify_ssh_stderr(s),
        Some(ConnectionFailureMode::ConnectTimeout)
    );
}

#[test]
fn connect_timeout_connection_timed_out_form() {
    let s = "ssh: connect to host prod-web port 22: Connection timed out";
    assert_eq!(
        classify_ssh_stderr(s),
        Some(ConnectionFailureMode::ConnectTimeout)
    );
}

#[test]
fn session_timeout_from_read_form() {
    let s = "Read from remote host prod-web: Operation timed out";
    assert_eq!(
        classify_ssh_stderr(s),
        Some(ConnectionFailureMode::SessionTimeout)
    );
}

#[test]
fn broken_pipe() {
    let s = "client_loop: send disconnect: Broken pipe";
    assert_eq!(
        classify_ssh_stderr(s),
        Some(ConnectionFailureMode::BrokenPipe)
    );
}

#[test]
fn auth_publickey_rejected_is_most_specific() {
    let s = "Permission denied (publickey).";
    assert_eq!(
        classify_ssh_stderr(s),
        Some(ConnectionFailureMode::AuthPublicKeyRejected)
    );
}

#[test]
fn auth_publickey_rejected_when_combined_with_password() {
    // openssh emits "publickey,password,keyboard-interactive" when all
    // three were offered. We want the publickey signal to win because
    // that's the most actionable (the user's key is missing or
    // rejected) — the password fallback rarely matters in our context.
    let s = "Permission denied (publickey,password,keyboard-interactive).";
    assert_eq!(
        classify_ssh_stderr(s),
        Some(ConnectionFailureMode::AuthPublicKeyRejected)
    );
}

#[test]
fn auth_password_rejected_when_no_publickey() {
    let s = "Permission denied (password).";
    assert_eq!(
        classify_ssh_stderr(s),
        Some(ConnectionFailureMode::AuthPasswordRejected)
    );
}

#[test]
fn auth_rejected_generic_fallback() {
    let s = "Permission denied.";
    assert_eq!(
        classify_ssh_stderr(s),
        Some(ConnectionFailureMode::AuthRejected)
    );
}

#[test]
fn host_key_mismatch() {
    let s = "Host key verification failed.";
    assert_eq!(
        classify_ssh_stderr(s),
        Some(ConnectionFailureMode::HostKeyMismatch)
    );
}

#[test]
fn host_key_changed_takes_precedence_over_mismatch() {
    // The "CHANGED" banner is a much stronger signal — the user
    // needs to investigate, not just clear known_hosts.
    let s = "@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@\n\
             @    WARNING: REMOTE HOST IDENTIFICATION HAS CHANGED!     @\n\
             @@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@\n\
             IT IS POSSIBLE THAT SOMEONE IS DOING SOMETHING NASTY!\n\
             Host key verification failed.";
    assert_eq!(
        classify_ssh_stderr(s),
        Some(ConnectionFailureMode::HostKeyChanged)
    );
}

#[test]
fn kex_closed_by_remote() {
    let s = "kex_exchange_identification: Connection closed by remote host";
    assert_eq!(
        classify_ssh_stderr(s),
        Some(ConnectionFailureMode::KexClosedByRemote)
    );
}

#[test]
fn proxyjump_failed() {
    let s = "ProxyJump request failed";
    assert_eq!(
        classify_ssh_stderr(s),
        Some(ConnectionFailureMode::ProxyJumpFailed)
    );
}

#[test]
fn identity_file_bad_permissions_wins_over_downstream_auth_failure() {
    // openssh prints both the UNPROTECTED PRIVATE KEY FILE banner
    // AND a Permission denied later in the same blob. The
    // user-actionable cause is the file permissions.
    let s = "@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@\n\
             @         WARNING: UNPROTECTED PRIVATE KEY FILE!          @\n\
             @@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@\n\
             Permissions 0644 for '/Users/david/.ssh/id_ed25519' are too open.\n\
             Permission denied (publickey).";
    assert_eq!(
        classify_ssh_stderr(s),
        Some(ConnectionFailureMode::IdentityFileBadPermissions)
    );
}

#[test]
fn identity_file_missing_wins_over_downstream_auth_failure() {
    let s = "Warning: Identity file /Users/david/.ssh/missing_key not accessible: No such file or directory.\n\
             Permission denied (publickey).";
    assert_eq!(
        classify_ssh_stderr(s),
        Some(ConnectionFailureMode::IdentityFileMissing)
    );
}

#[test]
fn ssh_config_syntax_error() {
    let s = "/Users/david/.ssh/config: line 12: Bad configuration option: HostNmae";
    assert_eq!(
        classify_ssh_stderr(s),
        Some(ConnectionFailureMode::SshConfigSyntaxError)
    );
}

#[test]
fn too_many_auth_failures() {
    let s = "Received disconnect from 10.0.0.5 port 22:2: Too many authentication failures";
    assert_eq!(
        classify_ssh_stderr(s),
        Some(ConnectionFailureMode::TooManyAuthFailures)
    );
}

#[test]
fn connection_reset_by_peer() {
    let s = "ssh: connect to host prod-web port 22: Connection reset by peer";
    assert_eq!(
        classify_ssh_stderr(s),
        Some(ConnectionFailureMode::ConnectionResetByPeer)
    );
}

#[test]
fn classifier_is_case_insensitive() {
    let s = "PERMISSION DENIED (PUBLICKEY).";
    assert_eq!(
        classify_ssh_stderr(s),
        Some(ConnectionFailureMode::AuthPublicKeyRejected)
    );
}

// ── label / suggested_fix / is_transient ────────────────────────────

#[test]
fn every_mode_has_nonempty_label_and_fix() {
    use ConnectionFailureMode::*;
    let all = [
        HostnameUnresolved,
        NetworkUnreachable,
        PortClosed,
        ConnectTimeout,
        SessionTimeout,
        BrokenPipe,
        AuthPublicKeyRejected,
        AuthPasswordRejected,
        AuthRejected,
        HostKeyMismatch,
        HostKeyChanged,
        KexClosedByRemote,
        ProxyJumpFailed,
        IdentityFileBadPermissions,
        IdentityFileMissing,
        SshConfigSyntaxError,
        TooManyAuthFailures,
        ConnectionResetByPeer,
    ];
    for mode in all {
        assert!(!mode.label().is_empty(), "{mode:?} has empty label");
        assert!(
            !mode.suggested_fix().is_empty(),
            "{mode:?} has empty suggested_fix",
        );
    }
}

#[test]
fn transient_modes_marked_for_retry() {
    use ConnectionFailureMode::*;
    assert!(NetworkUnreachable.is_transient());
    assert!(ConnectTimeout.is_transient());
    assert!(SessionTimeout.is_transient());
    assert!(BrokenPipe.is_transient());
    assert!(KexClosedByRemote.is_transient());
    assert!(ConnectionResetByPeer.is_transient());
}

#[test]
fn non_transient_modes_should_not_retry_automatically() {
    use ConnectionFailureMode::*;
    // Retrying these without changing input would just fail the
    // same way. Auto-reconnect policy uses `is_transient` to skip
    // the backoff retry and surface to the user immediately.
    assert!(!HostnameUnresolved.is_transient());
    assert!(!PortClosed.is_transient());
    assert!(!AuthPublicKeyRejected.is_transient());
    assert!(!AuthPasswordRejected.is_transient());
    assert!(!AuthRejected.is_transient());
    assert!(!HostKeyMismatch.is_transient());
    assert!(!HostKeyChanged.is_transient());
    assert!(!ProxyJumpFailed.is_transient());
    assert!(!IdentityFileBadPermissions.is_transient());
    assert!(!IdentityFileMissing.is_transient());
    assert!(!SshConfigSyntaxError.is_transient());
    assert!(!TooManyAuthFailures.is_transient());
}

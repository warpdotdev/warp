use super::{ssh_agent_status_with, SshAddOutcome, SshAddRunner, SshAgentStatus};

/// Test runner that returns a fixed canned outcome regardless of
/// the socket arg. The body it would return on a real call.
struct CannedRunner {
    outcome_fn: Box<dyn Fn(&str) -> SshAddOutcome + Send + Sync>,
}

impl CannedRunner {
    fn always_exit(code: i32, stdout: &'static str) -> Self {
        Self {
            outcome_fn: Box::new(move |_socket| SshAddOutcome {
                status_code: Some(code),
                stdout: stdout.to_string(),
            }),
        }
    }

    fn spawn_failure() -> Self {
        Self {
            outcome_fn: Box::new(|_socket| SshAddOutcome {
                status_code: None,
                stdout: String::new(),
            }),
        }
    }
}

impl SshAddRunner for CannedRunner {
    fn list_keys(&self, socket: &str) -> SshAddOutcome {
        (self.outcome_fn)(socket)
    }
}

// ── NotConfigured ──────────────────────────────────────────────────

#[test]
fn none_socket_classifies_as_not_configured() {
    let runner = CannedRunner::spawn_failure();
    let status = ssh_agent_status_with(&runner, None);
    assert_eq!(status, SshAgentStatus::NotConfigured);
}

#[test]
fn empty_socket_classifies_as_not_configured() {
    // openssh treats empty $SSH_AUTH_SOCK the same as unset.
    let runner = CannedRunner::spawn_failure();
    let status = ssh_agent_status_with(&runner, Some(""));
    assert_eq!(status, SshAgentStatus::NotConfigured);
}

// ── Available ──────────────────────────────────────────────────────

#[test]
fn exit_zero_with_listed_keys_classifies_as_available_with_count() {
    let listing = "256 SHA256:abc id_ed25519 (ED25519)\n\
                   2048 SHA256:def work_rsa (RSA)\n";
    let runner = CannedRunner::always_exit(0, listing);
    let status = ssh_agent_status_with(&runner, Some("/tmp/agent.sock"));

    assert_eq!(
        status,
        SshAgentStatus::Available {
            socket_path: "/tmp/agent.sock".to_string(),
            keys_loaded: 2,
        }
    );
}

#[test]
fn exit_one_with_no_identities_text_still_classifies_as_available_with_zero_keys() {
    // openssh emits exit code 1 + "The agent has no identities."
    // for a healthy agent with no keys loaded. That's not stale —
    // it's the same agent the user could `ssh-add` a key into.
    let runner = CannedRunner::always_exit(1, "The agent has no identities.\n");
    let status = ssh_agent_status_with(&runner, Some("/tmp/agent.sock"));

    assert_eq!(
        status,
        SshAgentStatus::Available {
            socket_path: "/tmp/agent.sock".to_string(),
            keys_loaded: 0,
        }
    );
}

#[test]
fn exit_zero_with_one_key() {
    let runner = CannedRunner::always_exit(0, "256 SHA256:abc id_ed25519 (ED25519)\n");
    let status = ssh_agent_status_with(&runner, Some("/tmp/agent.sock"));
    assert_eq!(
        status,
        SshAgentStatus::Available {
            socket_path: "/tmp/agent.sock".to_string(),
            keys_loaded: 1,
        }
    );
}

#[test]
fn empty_lines_in_stdout_are_not_counted_as_keys() {
    // Defensive: a trailing blank line shouldn't inflate the count.
    let runner = CannedRunner::always_exit(0, "256 SHA256:abc id_ed25519 (ED25519)\n\n\n");
    let status = ssh_agent_status_with(&runner, Some("/tmp/agent.sock"));
    assert_eq!(
        status,
        SshAgentStatus::Available {
            socket_path: "/tmp/agent.sock".to_string(),
            keys_loaded: 1,
        }
    );
}

// ── Stale ──────────────────────────────────────────────────────────

#[test]
fn other_exit_code_classifies_as_stale() {
    let runner = CannedRunner::always_exit(2, "");
    let status = ssh_agent_status_with(&runner, Some("/tmp/agent.sock"));
    match status {
        SshAgentStatus::Stale {
            socket_path,
            reason,
        } => {
            assert_eq!(socket_path, "/tmp/agent.sock");
            assert!(reason.contains("exited 2"));
        }
        other => panic!("expected Stale, got {other:?}"),
    }
}

#[test]
fn spawn_failure_classifies_as_stale() {
    let runner = CannedRunner::spawn_failure();
    let status = ssh_agent_status_with(&runner, Some("/tmp/agent.sock"));
    match status {
        SshAgentStatus::Stale {
            socket_path,
            reason,
        } => {
            assert_eq!(socket_path, "/tmp/agent.sock");
            assert!(reason.contains("could not be spawned"));
        }
        other => panic!("expected Stale, got {other:?}"),
    }
}

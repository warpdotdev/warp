//! Classify common ssh client stderr blobs into structured failure
//! modes so the UI can render an explanation + suggested fix
//! instead of dumping the raw text at the user.
//!
//! Pure pattern matching against substrings of the stderr output —
//! no IO, no subprocess spawning, no network. Tests drive
//! representative real-world ssh stderr captured from openssh
//! 9.x and Apple LibreSSH builds.
//!
//! The classifier is *deliberately conservative*: when an stderr
//! could match multiple modes, the highest-signal one wins (e.g.
//! "Permission denied (publickey)" classifies as
//! [`ConnectionFailureMode::AuthPublicKeyRejected`] rather than the
//! broader [`ConnectionFailureMode::AuthRejected`]). When no
//! pattern matches, returns `None` so the UI falls back to showing
//! the raw stderr verbatim — better to leak nothing than to
//! misclassify into a "suggested fix" that misleads.

use serde::{Deserialize, Serialize};

/// Structured reason an ssh connection attempt failed (or, in some
/// cases, an established connection dropped mid-session). Used by
/// the diagnostics chip + by the auto-reconnect policy to decide
/// whether a retry has a chance of succeeding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "mode")]
pub enum ConnectionFailureMode {
    /// `Could not resolve hostname X`. Typo in the host alias, or
    /// the host isn't in `~/.ssh/config` and isn't resolvable via
    /// DNS.
    HostnameUnresolved,
    /// `Network is unreachable`, `No route to host`. Local network
    /// problem or firewall blocking the destination.
    NetworkUnreachable,
    /// `Connection refused`. Reached the host but the ssh port
    /// (default 22) isn't listening. Possibly wrong port in
    /// `~/.ssh/config`, or sshd isn't running on the remote.
    PortClosed,
    /// `Connection timed out` (during initial connect). Firewall is
    /// silently dropping packets or the host is offline.
    ConnectTimeout,
    /// `Operation timed out` mid-session, or `Read from remote host
    /// X: Operation timed out`. Connection was established then the
    /// link silently dropped (typical for a laptop suspending or a
    /// flaky VPN).
    SessionTimeout,
    /// `client_loop: send disconnect: Broken pipe`. Connection was
    /// established then the TCP stream was forcibly torn down by an
    /// intermediate (NAT timeout, ISP reset).
    BrokenPipe,
    /// `Permission denied (publickey)`. Server only accepts publickey
    /// auth and ssh couldn't present a usable key. Suggests:
    /// `ssh-add` a key, fix `IdentityFile`, or check the remote
    /// `authorized_keys`.
    AuthPublicKeyRejected,
    /// `Permission denied (password)` or `Permission denied
    /// (password,keyboard-interactive)`. Password auth failed or
    /// publickey auth wasn't even attempted; suggests checking
    /// `IdentityFile` config.
    AuthPasswordRejected,
    /// `Permission denied` without a more specific qualifier — fall-
    /// back when we couldn't pin down the rejection reason.
    AuthRejected,
    /// `Host key verification failed`. The remote host's key
    /// doesn't match the entry in `known_hosts`. Either the host
    /// was reinstalled (benign — `ssh-keygen -R` clears the stale
    /// entry) or it's a MitM (not benign — investigate).
    HostKeyMismatch,
    /// `WARNING: REMOTE HOST IDENTIFICATION HAS CHANGED!` — same
    /// underlying cause as [`Self::HostKeyMismatch`] but with the
    /// stronger formatting openssh uses for the truly-unsafe path.
    /// Classified separately so the UI can use a stronger warning.
    HostKeyChanged,
    /// `kex_exchange_identification: Connection closed by remote
    /// host`. Server actively closed the connection during key
    /// exchange. Common causes: rate-limiter triggered, version
    /// mismatch, or sshd refusing the source IP.
    KexClosedByRemote,
    /// `ProxyJump request failed`, `Connection closed by UNKNOWN
    /// port 65535` after a ProxyJump line. The jump host couldn't
    /// reach the final host.
    ProxyJumpFailed,
    /// `Bad permissions` on key file (`UNPROTECTED PRIVATE KEY FILE!`
    /// banner). openssh refuses to use the key until permissions
    /// are tightened.
    IdentityFileBadPermissions,
    /// `Warning: Identity file X not accessible: No such file or
    /// directory`. `IdentityFile` config points at a path that
    /// doesn't exist.
    IdentityFileMissing,
    /// `Bad configuration option: X`. Syntax error in `~/.ssh/config`.
    SshConfigSyntaxError,
    /// `Too many authentication failures`. Server rate-limited
    /// after too many key attempts (usually agent has too many
    /// keys to offer).
    TooManyAuthFailures,
    /// `ssh: connect to host X port Y: Connection reset by peer`.
    /// Intermediate device or firewall actively reset the TCP
    /// connection.
    ConnectionResetByPeer,
}

impl ConnectionFailureMode {
    /// Short user-facing label.
    pub fn label(&self) -> &'static str {
        match self {
            Self::HostnameUnresolved => "Host name not resolved",
            Self::NetworkUnreachable => "Network unreachable",
            Self::PortClosed => "SSH port closed",
            Self::ConnectTimeout => "Connection timed out",
            Self::SessionTimeout => "Session timed out",
            Self::BrokenPipe => "Connection dropped",
            Self::AuthPublicKeyRejected => "Public-key auth rejected",
            Self::AuthPasswordRejected => "Password auth rejected",
            Self::AuthRejected => "Authentication rejected",
            Self::HostKeyMismatch => "Host key mismatch",
            Self::HostKeyChanged => "Host key CHANGED — investigate",
            Self::KexClosedByRemote => "Server closed connection during handshake",
            Self::ProxyJumpFailed => "ProxyJump host unreachable",
            Self::IdentityFileBadPermissions => "Identity file permissions too open",
            Self::IdentityFileMissing => "Identity file not found",
            Self::SshConfigSyntaxError => "Bad ~/.ssh/config syntax",
            Self::TooManyAuthFailures => "Too many auth attempts",
            Self::ConnectionResetByPeer => "Connection reset by peer",
        }
    }

    /// Actionable hint surfaced in the diagnostics chip's tooltip.
    pub fn suggested_fix(&self) -> &'static str {
        match self {
            Self::HostnameUnresolved => {
                "Check the alias in ~/.ssh/config or that the hostname resolves via DNS."
            }
            Self::NetworkUnreachable => {
                "Check VPN / network connectivity from this machine."
            }
            Self::PortClosed => {
                "Check the Port directive in ~/.ssh/config and that sshd is running on the remote."
            }
            Self::ConnectTimeout => {
                "Verify the host is online and reachable; a firewall may be silently dropping packets."
            }
            Self::SessionTimeout => {
                "Network link dropped after connecting. Often a laptop suspending or VPN flapping."
            }
            Self::BrokenPipe => {
                "TCP stream was torn down (NAT timeout, ISP reset). Reconnect and consider raising ServerAliveInterval."
            }
            Self::AuthPublicKeyRejected => {
                "Make sure your key is loaded (`ssh-add`), IdentityFile points at the right key, and the remote authorized_keys is current."
            }
            Self::AuthPasswordRejected => {
                "Password rejected or no IdentityFile was offered before falling back to password auth."
            }
            Self::AuthRejected => "Authentication rejected. Check key + password auth methods.",
            Self::HostKeyMismatch => {
                "Run `ssh-keygen -R <host>` to drop the stale known_hosts entry IF you trust the new key."
            }
            Self::HostKeyChanged => {
                "STRONG WARNING — host key changed unexpectedly. Verify out-of-band before clearing known_hosts."
            }
            Self::KexClosedByRemote => {
                "Server closed the handshake. Possible rate-limit or sshd refusing the source IP."
            }
            Self::ProxyJumpFailed => {
                "The ProxyJump host couldn't reach the final destination. Check the intermediate host first."
            }
            Self::IdentityFileBadPermissions => {
                "Run `chmod 600 <key>` (and `chmod 700 ~/.ssh`) so openssh stops refusing the key."
            }
            Self::IdentityFileMissing => {
                "IdentityFile in ~/.ssh/config points at a path that doesn't exist."
            }
            Self::SshConfigSyntaxError => {
                "There's a syntax error in ~/.ssh/config. Run `ssh -G <host>` to see openssh's parsing."
            }
            Self::TooManyAuthFailures => {
                "Server rate-limited too many key attempts. Trim the agent's keyring or add `IdentitiesOnly yes`."
            }
            Self::ConnectionResetByPeer => {
                "An intermediate firewall or device reset the connection. Try again or check the network path."
            }
        }
    }

    /// `true` when retrying on the same input has a meaningful
    /// chance of succeeding (transient cause). Auto-reconnect
    /// policies use this to decide whether to back off and retry vs.
    /// surface to the user immediately.
    pub fn is_transient(&self) -> bool {
        matches!(
            self,
            Self::NetworkUnreachable
                | Self::ConnectTimeout
                | Self::SessionTimeout
                | Self::BrokenPipe
                | Self::KexClosedByRemote
                | Self::ConnectionResetByPeer
        )
    }
}

/// Classify an ssh client stderr blob. Returns `None` when no
/// pattern matched — the UI should surface the raw text instead of
/// inventing a category.
///
/// Order matters: more-specific patterns are checked before broader
/// ones (e.g. `Permission denied (publickey)` matches
/// [`ConnectionFailureMode::AuthPublicKeyRejected`] before falling
/// through to the bare [`ConnectionFailureMode::AuthRejected`]).
pub fn classify_ssh_stderr(stderr: &str) -> Option<ConnectionFailureMode> {
    // Compare lowercased so e.g. "Could not resolve" / "could not
    // resolve" both match. Patterns themselves stay lowercase below.
    let s = stderr.to_lowercase();

    // ── Identity-flagged warnings come before the bare auth checks
    // ── because they emit "Permission denied" later in the same
    // ── stderr blob. The user-actionable cause is the key issue,
    // ── not the downstream auth failure.
    if s.contains("unprotected private key file") {
        return Some(ConnectionFailureMode::IdentityFileBadPermissions);
    }
    if s.contains("identity file") && s.contains("not accessible: no such file or directory") {
        return Some(ConnectionFailureMode::IdentityFileMissing);
    }

    // ── Config-time errors (no network call attempted)
    if s.contains("bad configuration option:") {
        return Some(ConnectionFailureMode::SshConfigSyntaxError);
    }

    // ── Host key check
    if s.contains("warning: remote host identification has changed") {
        return Some(ConnectionFailureMode::HostKeyChanged);
    }
    if s.contains("host key verification failed") {
        return Some(ConnectionFailureMode::HostKeyMismatch);
    }

    // ── Proxy jump
    if s.contains("proxyjump") && s.contains("request failed") {
        return Some(ConnectionFailureMode::ProxyJumpFailed);
    }

    // ── KEX / protocol-layer
    if s.contains("kex_exchange_identification") && s.contains("connection closed by remote") {
        return Some(ConnectionFailureMode::KexClosedByRemote);
    }

    // ── Auth (specific qualifier wins over the bare form)
    if s.contains("too many authentication failures") {
        return Some(ConnectionFailureMode::TooManyAuthFailures);
    }
    if s.contains("permission denied") && s.contains("publickey") {
        return Some(ConnectionFailureMode::AuthPublicKeyRejected);
    }
    if s.contains("permission denied") && s.contains("password") {
        return Some(ConnectionFailureMode::AuthPasswordRejected);
    }
    if s.contains("permission denied") {
        return Some(ConnectionFailureMode::AuthRejected);
    }

    // ── Network-layer (network unreachable beats connect-timeout
    // ── because "no route to host" is non-transient until routing
    // ── is fixed, vs a timeout that can succeed on retry)
    if s.contains("could not resolve hostname") {
        return Some(ConnectionFailureMode::HostnameUnresolved);
    }
    if s.contains("network is unreachable") || s.contains("no route to host") {
        return Some(ConnectionFailureMode::NetworkUnreachable);
    }
    if s.contains("connection refused") {
        return Some(ConnectionFailureMode::PortClosed);
    }
    if s.contains("connection reset by peer") {
        return Some(ConnectionFailureMode::ConnectionResetByPeer);
    }

    // ── Mid-session breakage
    if s.contains("client_loop: send disconnect: broken pipe") {
        return Some(ConnectionFailureMode::BrokenPipe);
    }
    if s.contains("read from remote host") && s.contains("operation timed out") {
        return Some(ConnectionFailureMode::SessionTimeout);
    }
    if s.contains("operation timed out") {
        // Plain "operation timed out" (no "Read from remote host"
        // prefix) is more often a connect-timeout than a session
        // timeout in practice.
        return Some(ConnectionFailureMode::ConnectTimeout);
    }
    if s.contains("connection timed out") {
        return Some(ConnectionFailureMode::ConnectTimeout);
    }

    None
}

#[cfg(test)]
#[path = "failure_modes_tests.rs"]
mod tests;

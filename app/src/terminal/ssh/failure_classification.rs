//! Map a `SessionConnectionFailed` event's raw `error` + `proxy_stderr`
//! strings to a structured user-facing banner.
//!
//! This is the consumer-side integration for `warp_ssh_diagnostics::classify_ssh_stderr`
//! (R3.6). When the remote-server manager fires a connection-failed
//! event, the terminal view used to show a generic
//! `"Failed to start SSH extension"` banner with the raw error text
//! as the detail. With this helper, the banner gets a classified
//! label (e.g. "Public-key auth rejected") + a suggested-fix line
//! (e.g. "Make sure your key is loaded (`ssh-add`)…") + the raw
//! stderr below the fold — so the user sees both the actionable
//! summary and the underlying ssh output.
//!
//! Pure function — no IO, no async — so it's unit-testable without
//! a transport setup. The integration call sits in
//! [`crate::terminal::view`] where the `RemoteServerManagerEvent::SessionConnectionFailed`
//! arm builds the banner's `UserFacingError`.

use remote_server::transport::UserFacingError;
use warp_ssh_diagnostics::{classify_ssh_stderr, ConnectionFailureMode};

/// Cap the raw-stderr tail so the banner stays readable when sshd /
/// the daemon dumps a wall of output.
const MAX_RAW_TAIL_CHARS: usize = 512;

/// Build the banner's `UserFacingError` from the `RemoteServerManagerEvent::SessionConnectionFailed`
/// payload. Classifies via [`classify_ssh_stderr`] when the input
/// matches a known failure mode; falls back to the original generic
/// body + raw error text when nothing matches.
pub fn build_ssh_failure_banner_error(error: &str, proxy_stderr: Option<&str>) -> UserFacingError {
    // Pattern match across both fields. The `proxy_stderr` typically
    // carries the actual ssh client output (the high-signal text);
    // `error` is sometimes a friendly wrapper from the manager
    // ("Failed to start daemon: …") that the classifier rarely
    // recognizes on its own. Concatenating with a newline lets the
    // matcher see both without us having to pick a preference order.
    let combined = match proxy_stderr {
        Some(stderr) if !stderr.is_empty() => {
            if error.is_empty() {
                stderr.to_string()
            } else {
                format!("{error}\n{stderr}")
            }
        }
        _ => error.to_string(),
    };

    match classify_ssh_stderr(&combined) {
        Some(mode) => UserFacingError {
            body: mode.label().to_string(),
            detail: Some(format_classified_detail(mode, error, proxy_stderr)),
        },
        None => UserFacingError {
            body: "Failed to start SSH extension".into(),
            detail: if error.is_empty() {
                None
            } else {
                Some(error.to_string())
            },
        },
    }
}

/// Compose the banner's detail text from the classified mode +
/// original strings:
///
/// 1. The mode's `suggested_fix()` line.
/// 2. (When non-empty) the original `error` line, prefixed so the
///    user knows where it came from.
/// 3. (When the stderr tail is non-empty) up to
///    [`MAX_RAW_TAIL_CHARS`] characters of the raw stderr, prefixed
///    and truncated with `…` so a verbose sshd dump doesn't blow out
///    the banner height.
fn format_classified_detail(
    mode: ConnectionFailureMode,
    error: &str,
    proxy_stderr: Option<&str>,
) -> String {
    let mut detail = String::new();
    detail.push_str(mode.suggested_fix());

    if !error.is_empty() {
        detail.push_str("\n\nError: ");
        detail.push_str(error);
    }

    if let Some(tail) = proxy_stderr.and_then(non_empty) {
        detail.push_str("\n\nSSH output:\n");
        detail.push_str(&truncate_chars(tail, MAX_RAW_TAIL_CHARS));
    }

    detail
}

fn non_empty(s: &str) -> Option<&str> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(s)
    }
}

/// Truncate `s` to at most `max_chars` characters, appending `…`
/// when truncated. Operates on character boundaries so a UTF-8
/// multi-byte sequence isn't cut in the middle.
fn truncate_chars(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let end = s
        .char_indices()
        .nth(max_chars)
        .map(|(i, _)| i)
        .unwrap_or(s.len());
    format!("{}…", &s[..end])
}

#[cfg(test)]
#[path = "failure_classification_tests.rs"]
mod tests;

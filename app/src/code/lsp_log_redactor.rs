//! App-side [`lsp::LogRedactor`] backed by Warp's secret redaction.

use std::borrow::Cow;

use lsp::LogRedactor;

use crate::server::telemetry::secret_redaction::redact_secrets_in_string;

/// Masks secrets in substituted custom-LSP `command`/`args` before they are
/// logged, using Warp's shared secret-redaction patterns.
pub struct AppSecretRedactor;

impl LogRedactor for AppSecretRedactor {
    fn redact_for_log<'a>(&self, value: &'a str) -> Cow<'a, str> {
        let mut redacted = value.to_string();
        redact_secrets_in_string(&mut redacted);
        if redacted.as_str() == value {
            Cow::Borrowed(value)
        } else {
            Cow::Owned(redacted)
        }
    }
}

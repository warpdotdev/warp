//! Log-time secret redaction injected across the `lsp` ↔ `app` boundary.

use std::borrow::Cow;

/// Masks secret-shaped substrings from a value before it is written to a log.
pub trait LogRedactor: Send + Sync {
    /// Returns `value` with any secret-shaped substrings masked, borrowing the
    /// input when nothing needed redaction.
    fn redact_for_log<'a>(&self, value: &'a str) -> Cow<'a, str>;
}

/// No-op redactor used when no app-level redactor is injected (tests and
/// headless paths). Returns the input unchanged.
pub struct NoopLogRedactor;

impl LogRedactor for NoopLogRedactor {
    fn redact_for_log<'a>(&self, value: &'a str) -> Cow<'a, str> {
        Cow::Borrowed(value)
    }
}

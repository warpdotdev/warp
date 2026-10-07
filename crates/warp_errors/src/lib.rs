//! Explicit error reporting for Warp.
//!
//! Provides the [`report_error!`] / [`report_if_error!`] macros and the machinery they depend on
//! (`ErrorExt`, `AnyhowErrorExt`, `register_error!`, `ReportErrorLogMode`). This is a leaf crate
//! that depends only on third-party crates, so any crate in the workspace can report errors to
//! Sentry without creating a dependency cycle with `warp_core`.
//!
//! Errors reported via these macros are captured to Sentry directly (structured, via
//! `capture_anyhow`/`capture_error`); plain `log::error!` is log-only.

mod anyhow;
mod registration;

// Built-in `ErrorExt` classifications for common third-party error types. These pull heavier
// dependencies (reqwest/tokio/websocket), so they are feature-gated and enabled by `warp_core`;
// leaf crates that only need `report_error!` don't pull them in.
#[cfg(feature = "reqwest-errors")]
mod reqwest;
#[cfg(feature = "tokio-errors")]
mod tokio;
#[cfg(feature = "websocket-errors")]
mod websocket;

// Re-export for macro use. The `register_error!` macro itself is available at the crate root via
// `#[macro_export]`; here we only re-export the supporting types it references. Re-export anyhow
// so the `report_error!` macro's string-literal form can build an `anyhow::Error` without callers
// needing `anyhow` in scope.
#[doc(hidden)]
pub use ::anyhow as __anyhow;
#[doc(hidden)]
pub use inventory::submit;
pub use registration::{ErrorRegistration, RegisteredError};

pub use self::anyhow::AnyhowErrorExt;

/// The `target` that is set by log entries from this crate.
pub const LOG_TARGET: &str = "errors::report_error";

/// Controls how often a [`report_error!`] invocation logs errors.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ReportErrorLogMode {
    /// Log every time the error is reported.
    #[default]
    EveryTime,
    /// Log only the first time this macro invocation is reached during the current app run.
    OncePerRun,
}

/// Reports an error encountered during execution.
///
/// If the error is actionable, it is captured to Sentry (via `capture_anyhow`/`capture_error`) and
/// logged at the Error level; otherwise it is logged at the Warn level and not reported. Plain
/// `log::error!` no longer creates Sentry events (see `sentry_log_filter` in `warp_logging`), so
/// `report_error!` is the explicit way to send an error to Sentry.
#[macro_export]
macro_rules! report_error {
    (@log $err:expr) => {{
        #[allow(unused_imports)]
        use $crate::{AnyhowErrorExt as _, ErrorExt as _, LOG_TARGET};
        let err = $err;
        let log_level = if err.is_actionable() {
            err.report_error();
            log::Level::Error
        } else {
            log::Level::Warn
        };
        log::log!(target: LOG_TARGET, log_level, "{:#}", err);
    }};
    (@once_per_run $err:expr) => {{
        static HAS_LOGGED_REPORT_ERROR: ::std::sync::atomic::AtomicBool =
            ::std::sync::atomic::AtomicBool::new(false);
        if !HAS_LOGGED_REPORT_ERROR.swap(true, ::std::sync::atomic::Ordering::Relaxed) {
            $crate::report_error!(@log $err);
        }
    }};
    (@once_per_run_extra $err:expr, { $($fields:tt)* }) => {{
        static HAS_LOGGED_REPORT_ERROR: ::std::sync::atomic::AtomicBool =
            ::std::sync::atomic::AtomicBool::new(false);
        if !HAS_LOGGED_REPORT_ERROR.swap(true, ::std::sync::atomic::Ordering::Relaxed) {
            $crate::report_error!(@log_extra $err, { $($fields)* });
        }
    }};
    // Reports `err` (capturing to Sentry when actionable) and attaches the given fields as a
    // structured Sentry context block ("details"), keeping per-instance specifics out of the
    // message so grouping stays stable. The fields are also appended to the local log line so
    // local/breadcrumb detail is retained.
    (@log_extra $err:expr, { $($fields:tt)* }) => {{
        #[allow(unused_imports)]
        use $crate::{AnyhowErrorExt as _, ErrorExt as _, LOG_TARGET};
        let err = $err;
        let mut __fields: ::std::vec::Vec<(&'static str, ::std::string::String)> =
            ::std::vec::Vec::new();
        $crate::report_error!(@fields __fields $($fields)*);
        let __suffix = $crate::format_context_suffix(&__fields);
        if err.is_actionable() {
            $crate::with_error_context(&__fields, || err.report_error());
            log::log!(target: LOG_TARGET, log::Level::Error, "{:#}{}", err, __suffix);
        } else {
            log::log!(target: LOG_TARGET, log::Level::Warn, "{:#}{}", err, __suffix);
        }
    }};
    // Field muncher for `extra: { .. }`. `%expr` forces Display, `?expr` forces Debug, a bare expr
    // defaults to Display.
    (@fields $vec:ident $key:literal => ? $value:expr $(, $($rest:tt)*)?) => {{
        $vec.push(($key, format!("{:?}", $value)));
        $crate::report_error!(@fields $vec $($($rest)*)?);
    }};
    (@fields $vec:ident $key:literal => % $value:expr $(, $($rest:tt)*)?) => {{
        $vec.push(($key, format!("{}", $value)));
        $crate::report_error!(@fields $vec $($($rest)*)?);
    }};
    (@fields $vec:ident $key:literal => $value:expr $(, $($rest:tt)*)?) => {{
        $vec.push(($key, format!("{}", $value)));
        $crate::report_error!(@fields $vec $($($rest)*)?);
    }};
    (@fields $vec:ident $(,)?) => {};
    // Static-message form: a bare string literal, wrapped in an `anyhow::Error`. It deliberately
    // does NOT accept trailing format arguments, to discourage interpolating variable data into
    // the (grouped) error message. Put variable data in `extra: { .. }`, or use
    // `report_error!(anyhow!(..))` explicitly.
    ($fmt:literal, extra: { $($fields:tt)* }) => {{
        $crate::report_error!(@log_extra $crate::__anyhow::anyhow!($fmt), { $($fields)* });
    }};
    // Static-message form with a structured `extra:` block AND an explicit log mode (e.g.
    // `ReportErrorLogMode::OncePerRun`), so throttled reports can still carry per-instance data
    // out of the grouped message.
    ($fmt:literal, extra: { $($fields:tt)* }, $log_mode:expr) => {{
        match $log_mode {
            $crate::ReportErrorLogMode::EveryTime => {
                $crate::report_error!(@log_extra $crate::__anyhow::anyhow!($fmt), { $($fields)* });
            }
            $crate::ReportErrorLogMode::OncePerRun => {
                $crate::report_error!(
                    @once_per_run_extra $crate::__anyhow::anyhow!($fmt), { $($fields)* }
                );
            }
        }
    }};
    ($fmt:literal) => {{
        $crate::report_error!(@log $crate::__anyhow::anyhow!($fmt));
    }};
    // Error-value forms.
    ($err:expr, extra: { $($fields:tt)* }) => {{
        $crate::report_error!(@log_extra $err, { $($fields)* });
    }};
    // Error-value form with a structured `extra:` block AND an explicit log mode.
    ($err:expr, extra: { $($fields:tt)* }, $log_mode:expr) => {{
        match $log_mode {
            $crate::ReportErrorLogMode::EveryTime => {
                $crate::report_error!(@log_extra $err, { $($fields)* });
            }
            $crate::ReportErrorLogMode::OncePerRun => {
                $crate::report_error!(@once_per_run_extra $err, { $($fields)* });
            }
        }
    }};
    ($err:expr) => {{
        $crate::report_error!(@log $err);
    }};
    ($err:expr, $crate::ReportErrorLogMode::EveryTime) => {{
        $crate::report_error!(@log $err);
    }};
    ($err:expr, ReportErrorLogMode::EveryTime) => {{
        $crate::report_error!(@log $err);
    }};
    ($err:expr, $crate::ReportErrorLogMode::OncePerRun) => {{
        $crate::report_error!(@once_per_run $err);
    }};
    ($err:expr, ReportErrorLogMode::OncePerRun) => {{
        $crate::report_error!(@once_per_run $err);
    }};
    ($err:expr, $log_mode:expr) => {{
        match $log_mode {
            $crate::ReportErrorLogMode::EveryTime => {
                $crate::report_error!(@log $err);
            }
            $crate::ReportErrorLogMode::OncePerRun => {
                $crate::report_error!(@once_per_run $err);
            }
        }
    }};
}

/// Reports an error if the provided [`Result`] is [`Err`].
///
/// This checks whether or not the error is actionable, and logs an error or warning accordingly.
#[macro_export]
macro_rules! report_if_error {
    ($result:expr) => {{
        if let Err(error) = &$result {
            $crate::report_error!(error);
        }
    }};
    ($result:expr, extra: { $($fields:tt)* }) => {{
        if let Err(error) = &$result {
            $crate::report_error!(error, extra: { $($fields)* });
        }
    }};
    ($result:expr, $log_mode:expr) => {{
        if let Err(error) = &$result {
            $crate::report_error!(error, $log_mode);
        }
    }};
}

/// Runs `report` while the current Sentry scope carries `fields` as a structured "details" context
/// block. Used by `report_error!(.., extra: { .. })` so per-instance specifics stay out of the
/// error message (and out of grouping).
#[doc(hidden)]
#[cfg(feature = "crash_reporting")]
pub fn with_error_context(fields: &[(&'static str, String)], report: impl FnOnce()) {
    if fields.is_empty() {
        report();
        return;
    }
    sentry::with_scope(
        |scope| {
            let mut context = std::collections::BTreeMap::new();
            for (key, value) in fields {
                context.insert((*key).to_string(), value.clone().into());
            }
            scope.set_context("details", sentry::protocol::Context::Other(context));
        },
        report,
    );
}

/// Non-`crash_reporting` builds have no Sentry scope, so just run `report`.
#[doc(hidden)]
#[cfg(not(feature = "crash_reporting"))]
pub fn with_error_context(_fields: &[(&'static str, String)], report: impl FnOnce()) {
    report();
}

/// Formats `report_error!` context fields for the local log line. This log line is not what Sentry
/// groups on, so it can carry the full per-instance detail.
#[doc(hidden)]
pub fn format_context_suffix(fields: &[(&'static str, String)]) -> String {
    if fields.is_empty() {
        return String::new();
    }
    let mut suffix = String::from(" [");
    for (index, (key, value)) in fields.iter().enumerate() {
        if index > 0 {
            suffix.push_str(", ");
        }
        suffix.push_str(key);
        suffix.push('=');
        suffix.push_str(value);
    }
    suffix.push(']');
    suffix
}

/// Returns whether or not a log entry with the given metadata should be ignored by Sentry.
#[cfg(feature = "crash_reporting")]
pub fn should_ignore_log_for_sentry(md: &log::Metadata) -> bool {
    // Filter out any Error-level log entries generated by report_error!().
    // report_error!() utilizes capture_anyhow() to report structured errors
    // instead of simple string error messages, and we don't want to _also_
    // report the Error-level log line to Sentry.
    md.target() == LOG_TARGET && md.level() == log::Level::Error
}

pub trait ErrorExt: RegisteredError + std::error::Error {
    /// Returns whether or not an error is something that is actionable by our engineering team.
    fn is_actionable(&self) -> bool;

    fn report_error(&self) {
        #[cfg(feature = "crash_reporting")]
        sentry::capture_error(self);
    }
}

#[cfg(test)]
#[path = "errors_tests.rs"]
mod tests;

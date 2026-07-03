mod anyhow;
mod registration;
mod reqwest;
mod tokio;
// Re-export for macro use.
#[doc(hidden)]
pub use inventory::submit;

pub use self::anyhow::AnyhowErrorExt;
pub use registration::{ErrorRegistration, RegisteredError};

pub use registration::register_error;

/// The `target` that is set by log entries from this module.
pub const LOG_TARGET: &str = "errors::report_error";

/// Reports an error encountered during execution.
///
/// This checks whether or not the error is actionable, and logs an error or
/// warning accordingly.  (Logs at the Error level get reported back to us, so
/// we don't want to log anything at Error level that we aren't able to act
/// upon.)
#[macro_export]
macro_rules! report_error {
    ($err:expr) => {{
        #[allow(unused_imports)]
        use $crate::errors::{AnyhowErrorExt as _, ErrorExt as _, LOG_TARGET};
        let err = $err;
        let log_level = if err.is_actionable() {
            err.report_error();
            log::Level::Error
        } else {
            log::Level::Warn
        };
        log::log!(target: LOG_TARGET, log_level, "{:#}", err);
    }};
}
pub use report_error;

/// Reports an error if the provided [`Result`] is [`Err`].
///
/// This checks whether or not the error is actionable, and logs an error or
/// warning accordingly.  (Logs at the Error level get reported back to us, so
/// we don't want to log anything at Error level that we aren't able to act
/// upon.)
#[macro_export]
macro_rules! report_if_error {
    ($result:expr) => {{
        if let Err(error) = &$result {
            $crate::report_error!(error);
        }
    }};
}
pub use report_if_error;

pub trait ErrorExt: RegisteredError + std::error::Error {
    /// Returns whether or not an error is something that is actionable by our
    /// engineering team.
    fn is_actionable(&self) -> bool;

    fn report_error(&self) {}
}

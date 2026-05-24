use super::registration::AnyErrorRegistration;

/// A version of [`ErrorExt`] that works for [`anyhow::Error`] (which does not
/// implement [`std::error::Error`]).
pub trait AnyhowErrorExt {
    /// Returns whether or not an error is something that is actionable by our
    /// engineering team.
    fn is_actionable(&self) -> bool;

    /// Reports the error.
    fn report_error(&self);
}

impl AnyhowErrorExt for anyhow::Error {
    fn is_actionable(&self) -> bool {
        for cause in self.chain() {
            for imp in inventory::iter::<&'static dyn AnyErrorRegistration>() {
                if imp.downcast_and_is_actionable(cause) == Some(false) {
                    return false;
                }
            }
        }

        true
    }

    fn report_error(&self) {
        #[cfg(feature = "crash_reporting")]
        sentry::integrations::anyhow::capture_anyhow(self);
    }
}

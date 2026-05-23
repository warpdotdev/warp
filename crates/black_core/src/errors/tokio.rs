use super::{register_error, ErrorExt};

impl ErrorExt for tokio::task::JoinError {
    fn is_actionable(&self) -> bool {
        // If the task was cancelled (aborted), this is expected behavior and not actionable.
        if self.is_cancelled() {
            return false;
        }

        // If the task panicked, this is actionable - we need to know about panics.
        if self.is_panic() {
            return true;
        }

        // Other join errors are actionable.
        true
    }
}
register_error!(tokio::task::JoinError);

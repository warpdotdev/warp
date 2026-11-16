use std::time::Duration;

use crate::{IsolationPlatformError, WorkloadToken};

/// Issue a Docker sandbox workload token.
/// Docker sandbox tokens do not have an expiration time.
pub async fn issue_workload_token(
    _duration: Option<Duration>,
) -> Result<WorkloadToken, IsolationPlatformError> {
    crate::read_generic_workload_token()
}

//! The implementation of `LogService` to be served by the app process to the plugin host process.
use async_trait::async_trait;

use crate::plugin::service::{LogService, LogServiceRequest, LogServiceResponse};

#[derive(Debug, Clone)]
pub struct LogServiceImpl {}

impl LogServiceImpl {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl ipc::ServiceImpl for LogServiceImpl {
    type Service = LogService;

    async fn handle_request(
        &self,
        request: LogServiceRequest,
    ) -> Result<LogServiceResponse, String> {
        let LogServiceRequest {
            level,
            target,
            message,
        } = request;
        let log_fn = || {
            log::log!(target: target.as_str(), level, "{message}");
        };
        cfg_if::cfg_if! {
            if #[cfg(feature = "crash_reporting")] {
                // Explicitly write the log line in the context of the main
                // Sentry hub; this log receiver thread is spawned before
                // Sentry is configured, so the thread-local hub doesn't
                // have the appropriate client and scope configuration.
                sentry::Hub::run(sentry::Hub::main(), log_fn);
            } else {
                log_fn();
            }
        }
        Ok(LogServiceResponse { success: true })
    }
}

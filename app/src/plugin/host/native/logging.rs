use std::sync::Arc;

use warpui::r#async::block_on;
use warpui::r#async::executor::Background;

use crate::plugin::service::{LogService, LogServiceRequest};

/// Initializes logging for the plugin host process. Internally, the logger relays log messages to
/// the main app process via the IPC LogService.
pub(super) fn initialize_logging(client: &Arc<ipc::Client>, executor: &Arc<Background>) {
    let log_service = ipc::service_caller::<LogService>(client.clone());
    log::set_boxed_logger(Box::new(PluginHostLogger::new(
        log_service,
        executor.clone(),
    )))
    .expect("Logger should only be set once.");
    log::set_max_level(log::LevelFilter::Info);
}

/// A logger that relays plugin host log messages to the main app process via IPC `LogService`.
pub(super) struct PluginHostLogger {
    request_tx: async_channel::Sender<LogServiceRequest>,
}

impl PluginHostLogger {
    pub(super) fn new(
        log_service: Box<dyn ipc::ServiceCaller<LogService>>,
        background_executor: Arc<Background>,
    ) -> Self {
        let (request_tx, message_rx) = async_channel::unbounded();
        background_executor
            .spawn(async move {
                while let Ok(message) = message_rx.recv().await {
                    if let Err(err) = log_service.call(message).await {
                        // In failing tests, the app shuts down abruptly and this message pollutes the test
                        // output.
                        if !cfg!(feature = "integration_tests") {
                            eprintln!("Failed to send log record to host process: {err:#}");
                        }
                    }
                }
            })
            .detach();

        Self { request_tx }
    }
}

impl log::Log for PluginHostLogger {
    fn enabled(&self, _metadata: &log::Metadata) -> bool {
        true
    }

    fn log(&self, record: &log::Record) {
        let _ = block_on(self.request_tx.send(LogServiceRequest {
            level: record.level(),
            target: record.target().to_string(),
            message: record.args().to_string(),
        }));
    }

    fn flush(&self) {}
}

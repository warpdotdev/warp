#[cfg(not(target_family = "wasm"))]
use std::time::Duration;

use tracing::subscriber;

#[cfg(not(target_family = "wasm"))]
mod native;

#[cfg(not(target_family = "wasm"))]
const DEFAULT_EXPORT_TIMEOUT: Duration = Duration::from_secs(10);

pub fn init() -> anyhow::Result<Initialization> {
    #[cfg(target_family = "wasm")]
    {
        install_no_subscriber()?;
        Ok(Initialization::default())
    }

    #[cfg(not(target_family = "wasm"))]
    native::init()
}

fn install_no_subscriber() -> anyhow::Result<()> {
    // Configure the global tracing subscriber to not care about any spans or
    // events.
    //
    // This is done so that we prevent the `tracing` crate from writing out log
    // lines for spans and trace events.
    subscriber::set_global_default(subscriber::NoSubscriber::new())?;
    Ok(())
}

#[cfg_attr(target_family = "wasm", derive(Default))]
pub struct Initialization {
    initialization_warning: Option<anyhow::Error>,
    #[cfg(not(target_family = "wasm"))]
    active_spans: Option<native::ActiveSpanRegistry>,
    #[cfg(not(target_family = "wasm"))]
    provider: Option<opentelemetry_sdk::trace::SdkTracerProvider>,
    #[cfg(not(target_family = "wasm"))]
    shutdown_timeout: std::time::Duration,
}

#[cfg(not(target_family = "wasm"))]
impl Default for Initialization {
    fn default() -> Self {
        Self {
            initialization_warning: None,
            active_spans: None,
            provider: None,
            shutdown_timeout: DEFAULT_EXPORT_TIMEOUT,
        }
    }
}

impl Initialization {
    pub fn log_initialization_warning(&mut self) {
        if let Some(err) = self.initialization_warning.take() {
            log::warn!("Failed to initialize cloud-agent OpenTelemetry exporting: {err:#}");
        }
    }

    pub(crate) fn shutdown(&mut self) {
        #[cfg(not(target_family = "wasm"))]
        {
            match (self.active_spans.take(), self.provider.take()) {
                (Some(active_spans), Some(provider)) => {
                    if let Err(err) = active_spans.shutdown(&provider, self.shutdown_timeout) {
                        log::warn!(
                            "Failed to shut down cloud-agent OpenTelemetry exporting: {err}"
                        );
                    }
                }
                (None, Some(provider)) => {
                    if let Err(err) = provider.shutdown_with_timeout(self.shutdown_timeout) {
                        log::warn!(
                            "Failed to shut down cloud-agent OpenTelemetry exporting: {err}"
                        );
                    }
                }
                (Some(_), None) | (None, None) => {}
            }
        }
    }
}

impl Drop for Initialization {
    fn drop(&mut self) {
        self.shutdown();
    }
}

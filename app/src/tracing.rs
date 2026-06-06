#[cfg(not(target_family = "wasm"))]
mod native;

pub fn init() -> anyhow::Result<Initialization> {
    #[cfg(target_family = "wasm")]
    {
        // Configure the global tracing subscriber to not care about any spans or
        // events.
        //
        // This is done so that we prevent the `tracing` crate from writing out log
        // lines for spans and trace events.
        tracing::subscriber::set_global_default(subscriber::NoSubscriber::new())?;
        return Ok(Initialization {
            initialization_warning: None,
        });
    }

    #[cfg(not(target_family = "wasm"))]
    native::init()
}

pub struct Initialization {
    initialization_warning: Option<anyhow::Error>,
    #[cfg(not(target_family = "wasm"))]
    provider: Option<opentelemetry_sdk::trace::SdkTracerProvider>,
    #[cfg(not(target_family = "wasm"))]
    shutdown_timeout: std::time::Duration,
}

impl Initialization {
    pub fn log_initialization_warning(&mut self) {
        if let Some(err) = self.initialization_warning.take() {
            log::warn!("Failed to initialize cloud-agent OpenTelemetry exporting: {err:#}");
        }
    }
}

impl Drop for Initialization {
    fn drop(&mut self) {
        if let Some(provider) = self.provider.take() {
            if let Err(err) = provider.shutdown_with_timeout(self.shutdown_timeout) {
                log::warn!("Failed to shut down cloud-agent OpenTelemetry exporting: {err}");
            }
        }
    }
}

use tracing::subscriber;

#[cfg(not(target_family = "wasm"))]
mod cloud_agent;

/// Initializes tracing and any purpose-scoped integrations selected by their configuration.

pub fn init() -> anyhow::Result<Initialization> {
    #[cfg(target_family = "wasm")]
    {
        install_no_subscriber()?;
        return Ok(Initialization::default());
    }

    #[cfg(not(target_family = "wasm"))]
    {
        Ok(Initialization {
            cloud_agent: cloud_agent::init()?,
        })
    }
}

#[cfg(not(target_family = "wasm"))]
/// Starts cloud-agent trace credential refresh after authenticated application services exist.
///
/// The exporter and dispatch credential are initialized earlier by [`init`]. This later lifecycle
/// hook supplies the authenticated managed-secrets client needed to mint replacements without
/// broadening tracing initialization to ordinary application processes.
pub fn start_auth_refresh(
    client: std::sync::Arc<dyn warp_managed_secrets::client::ManagedSecretsClient>,
    ctx: &mut warpui::AppContext,
) {
    cloud_agent::start_auth_refresh(client, ctx);
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

/// Retains lifecycle state for the purpose-scoped tracing integrations that opted in at startup.
#[derive(Default)]
pub struct Initialization {
    #[cfg(not(target_family = "wasm"))]
    cloud_agent: Option<cloud_agent::Initialization>,
}

impl Initialization {
    /// Logs delayed initialization warnings after application logging is available.
    pub fn log_initialization_warning(&mut self) {
        #[cfg(not(target_family = "wasm"))]
        if let Some(cloud_agent) = self.cloud_agent.as_mut() {
            cloud_agent.log_initialization_warning();
        }
    }

    /// Shuts down each tracing integration that opted in during initialization.
    pub(crate) fn shutdown(&mut self) {
        #[cfg(not(target_family = "wasm"))]
        if let Some(cloud_agent) = self.cloud_agent.as_mut() {
            cloud_agent.shutdown();
        }
    }
}

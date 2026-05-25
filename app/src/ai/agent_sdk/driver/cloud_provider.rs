use std::collections::HashMap;
use std::ffi::OsString;
use std::future::Future;
use std::pin::Pin;

use anyhow::Error;
use warpui::ModelSpawner;

use super::terminal::TerminalDriver;
use crate::ai::cloud_environments::ProvidersConfig;

mod aws;
mod gcp;

pub(crate) type Result<T> = std::result::Result<T, CloudProviderSetupError>;

#[derive(Debug, thiserror::Error)]
#[error("{provider_name} setup failed")]
pub(crate) struct CloudProviderSetupError {
    provider_name: &'static str,
    #[source]
    source: Error,
}

impl CloudProviderSetupError {
    pub(crate) fn new(provider_name: &'static str, source: impl Into<Error>) -> Self {
        Self {
            provider_name,
            source: source.into(),
        }
    }
}

/// A cloud provider that we configure automatic Oz access to.
pub(crate) trait CloudProvider: Send {
    /// Return environment variables that should be injected into the terminal
    /// session.
    fn env_vars(&self) -> Result<HashMap<OsString, OsString>>;

    /// Perform any async setup that requires the terminal session to be running.
    fn setup(
        &mut self,
        _spawner: ModelSpawner<TerminalDriver>,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>> {
        Box::pin(async { Ok(()) })
    }

    /// Best-effort cleanup of any resources created during setup.
    ///
    /// The default implementation is a no-op.
    fn cleanup(self: Box<Self>) -> Pin<Box<dyn Future<Output = Result<()>> + Send>> {
        Box::pin(async { Ok(()) })
    }
}

/// Build the set of cloud providers from an environment's provider configuration.
pub(crate) fn load_providers(
    providers: &ProvidersConfig,
    run_id: &str,
) -> Result<Vec<Box<dyn CloudProvider>>> {
    let mut result: Vec<Box<dyn CloudProvider>> = Vec::new();

    if let Some(aws) = &providers.aws {
        result.push(Box::new(aws::AwsCloudProvider::new(aws, run_id)?));
    }

    if let Some(gcp) = &providers.gcp {
        result.push(Box::new(gcp::GcpCloudProvider::new(gcp, run_id)?));
    }

    Ok(result)
}

/// Collect all environment variables from a list of providers.
pub(crate) fn collect_env_vars(
    providers: &[Box<dyn CloudProvider>],
    vars: &mut HashMap<OsString, OsString>,
) -> Result<()> {
    for provider in providers {
        vars.extend(provider.env_vars()?);
    }
    Ok(())
}

#[cfg(test)]
#[path = "cloud_provider_tests.rs"]
mod tests;

use std::{
    collections::HashMap, error::Error as StdError, ffi::OsString, fmt, future::Future, pin::Pin,
};

use anyhow::Error;
use warp_localization::{LocaleId, replace_placeholders};
use warpui::ModelSpawner;

use super::terminal::TerminalDriver;
use crate::ai::cloud_environments::ProvidersConfig;
use crate::localization;

mod aws;
mod gcp;

pub(crate) type Result<T> = std::result::Result<T, CloudProviderSetupError>;

fn text(key: &str) -> String {
    localization::text_for_locale(LocaleId::EnUs, key)
}

fn text_with_args(key: &str, args: &[(&str, &str)]) -> String {
    replace_placeholders(&text(key), args)
        .expect("localized text template arguments must match the catalog")
}

#[derive(Debug)]
pub(crate) struct CloudProviderSetupError {
    provider_name: &'static str,
    source: Error,
}

impl fmt::Display for CloudProviderSetupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&text_with_args(
            "agent_sdk.driver.cloud_provider.error.setup_failed",
            &[("provider_name", self.provider_name)],
        ))
    }
}

impl StdError for CloudProviderSetupError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        Some(self.source.as_ref())
    }
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

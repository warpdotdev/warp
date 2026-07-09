use itertools::Itertools as _;
use warp_errors::report_error;
use warpui::{Entity, ModelContext, SingletonEntity};
use warpui_extras::user_preferences::registry_backed::KEY_NOT_FOUND_ERR;
use windows_registry::CURRENT_USER;
use windows_result::Error as WindowsError;

use crate::send_telemetry_from_ctx;
use crate::server::telemetry::TelemetryEvent;

const DOCKER_DESKTOP_WSL_DISTRO_PREFIX: &str = "docker-desktop";
const RANCHER_DESKTOP_WSL_DISTRO_PREFIX: &str = "rancher-desktop";

/// Contains information about WSL distributions available on the user's Windows machine.
pub(crate) struct WslInfo {
    distributions: Vec<Distribution>,
}

impl Entity for WslInfo {
    type Event = ();
}

impl SingletonEntity for WslInfo {}

#[derive(Debug, PartialEq)]
pub(crate) struct Distribution {
    uuid: String,
    pub name: String,
    pub is_default: bool,
}

impl WslInfo {
    pub(crate) fn new(ctx: &mut ModelContext<Self>) -> Self {
        let distributions = match Self::find_available_distributions() {
            Ok(distributions) => distributions,
            // This error merely occurs when the user doesn't have WSL installed/enabled.
            Err(Error::MainKey(err)) => {
                log::info!("{err:#}");
                send_telemetry_from_ctx!(TelemetryEvent::WSLRegistryError, ctx);
                Vec::new()
            }
            Err(err @ Error::DistributionIterator(_)) => {
                report_error!(anyhow::Error::new(err).context("WSL registry error"));
                Vec::new()
            }
        };
        Self { distributions }
    }

    pub(crate) fn distributions(&self) -> impl Iterator<Item = &Distribution> {
        self.distributions.iter()
    }

    /// Finds available WSL distributions available on the local machine.
    fn find_available_distributions() -> Result<Vec<Distribution>, Error> {
        // The storage format is the following:
        // Lxss/
        //      DefaultDistribution: 1787c461-d291-401d-b579-1ceff55f97e3
        //      {1787c461-d291-401d-b579-1ceff55f97e3}
        //          DistributionName: Ubuntu
        //      {63375eca-1f0a-4d5d-9682-16b0d42fded8}
        //          DistributionName: Ubuntu-18.04
        let key = CURRENT_USER
            .open("Software\\Microsoft\\Windows\\CurrentVersion\\Lxss")
            .map_err(Error::MainKey)?;

        let default_distribution_uuid = key
            .get_string("DefaultDistribution")
            .inspect_err(|err| log::warn!("Could not obtain the default distribution: {err:#}"));
        let distribution_keys = key.keys().map_err(Error::DistributionIterator)?;
        let distributions = distribution_keys
            .into_iter()
            .flat_map(|uuid| {
                let distribution_key = key
                    .open(&uuid)
                    .map_err(|err| {
                        report_error!(anyhow::Error::new(err)
                            .context("Could not open distribution registry key"))
                    })
                    .ok()?;
                let name = distribution_key
                    .get_string("DistributionName")
                    .map_err(|err| {
                        // Some entries don't have names, that's not an error state we need to monitor.
                        if err.code() != KEY_NOT_FOUND_ERR {
                            report_error!(
                                anyhow::Error::new(err).context("Unable to read distribution name")
                            );
                        }
                    })
                    .ok()?;

                // Docker uses WSL on windows but we don't expect users to start a new session for
                // that WSL instance. Same with Rancher.
                if name.starts_with(DOCKER_DESKTOP_WSL_DISTRO_PREFIX)
                    || name.starts_with(RANCHER_DESKTOP_WSL_DISTRO_PREFIX)
                {
                    return None;
                }

                Some(Distribution {
                    name,
                    is_default: default_distribution_uuid
                        .as_ref()
                        .is_ok_and(|default_distribution_uuid| *default_distribution_uuid == uuid),
                    uuid,
                })
            })
            .collect_vec();

        if distributions.iter().all(|distro| !distro.is_default) {
            log::warn!("No distribution matched the default guid");
        }

        Ok(distributions)
    }
}

#[derive(thiserror::Error, Debug)]
enum Error {
    #[error("Error opening the main key: {0:#}")]
    MainKey(#[source] WindowsError),
    #[error("Could not iterate through distributions: {0:#}")]
    DistributionIterator(#[source] WindowsError),
}

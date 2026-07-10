use std::env;
use std::fs::read_to_string;
use std::sync::Arc;

use anyhow::{Context as _, Result};
use channel_versions::ChannelVersions;
use warp_errors::report_error;

use crate::channel::{Channel, ChannelState};
use crate::server::server_api::{ServerApi, FETCH_CHANNEL_VERSIONS_TIMEOUT};

// Fetches channel versions asynchronously from the Warp server. If the Warp server request fails,
// then fetches from GCP JSON storage as a fallback.
pub async fn fetch_channel_versions(
    nonce: &str,
    server_api: Arc<ServerApi>,
    include_changelogs: bool,
    is_daily: bool,
) -> Result<ChannelVersions> {
    if let Ok(path) = env::var("WARP_CHANNEL_VERSIONS_PATH") {
        // Load channel versions from local filesystem. Used for testing both
        // autoupdate and changelog behavior.
        let path = shellexpand::tilde(&path);
        let channel_versions_string = read_to_string::<&str>(&path)?;
        return serde_json::from_str(channel_versions_string.as_str())
            .context("Failed to parse channel versions JSON");
    }

    let channel_versions = server_api
        .fetch_channel_versions(include_changelogs, is_daily)
        .await
        .context("Failed to retrieve channel versions from Warp server");
    match channel_versions {
        channel_versions @ Ok(_) => channel_versions,
        Err(err) => {
            match ChannelState::channel() {
                // Only log an error on Dev and Preview -- if this is failing, its likely to be
                // failing for all users, and Stable has too many users (this error would flood
                // our Sentry logs).
                Channel::Dev | Channel::Preview => report_error!(err),
                _ => log::warn!(
                    "Failed to retrieve channel versions from Warp server, falling \
                back to GCP JSON storage."
                ),
            }
            fetch_channel_versions_from_json_storage(server_api.http_client(), nonce).await
        }
    }
}

// Synchronously fetches updated Warp [`ChannelVersions`] from GCP JSON storage. This will soon
// be deprecated in favor of retrieving updated channel versions from the Warp Server.
// Note, in order to run against a test file you can use the "channel_versions_test.json" file
// and update the file using gsutil cp channel_versions_test.json gs://warp-releases/channel_versions_test.json
async fn fetch_channel_versions_from_json_storage(
    client: &http_client::Client,
    nonce: &str,
) -> Result<ChannelVersions> {
    log::info!("Fetching channel versions from GCP JSON storage");
    let res = client
        .get(
            format!(
                "{}/channel_versions.json?r={}",
                ChannelState::releases_base_url(),
                nonce
            )
            .as_str(),
        )
        .timeout(FETCH_CHANNEL_VERSIONS_TIMEOUT)
        .send()
        .await?;
    let versions: ChannelVersions = res.json().await?;
    log::info!("Received channel versions from GCP JSON storage: {versions}");
    Ok(versions)
}

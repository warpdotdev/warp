//! Loading of computer-use screenshots whose bytes live in Warp-managed object
//! storage rather than inline on the client (e.g. restored/shared conversations
//! whose screenshot bytes were offloaded by the server).

use std::sync::Arc;

use warp_multi_agent_api::StoredScreenshotRef;
use warpui::assets::asset_cache::{AssetSource, AsyncAssetId, AsyncAssetType};

use crate::server::server_api::ai::AIClient;

/// Namespace for computer-use screenshot bytes fetched on demand from
/// Warp-managed object storage.
struct StoredScreenshotAsset;
impl AsyncAssetType for StoredScreenshotAsset {}

/// Builds an async asset source that resolves the stored ref to a signed URL and downloads the
/// screenshot bytes on demand. Keyed by the screenshot UID (not the signed URL, which changes on
/// every resolution) so repeat loads reuse the cached bytes.
pub fn stored_screenshot_asset_source(
    stored_ref: StoredScreenshotRef,
    ai_client: Arc<dyn AIClient>,
    http_client: Arc<http_client::Client>,
) -> AssetSource {
    AssetSource::Async {
        id: AsyncAssetId::new::<StoredScreenshotAsset>(stored_ref.screenshot_uid.clone()),
        fetch: Arc::new(move || {
            let ai_client = ai_client.clone();
            let http_client = http_client.clone();
            let stored_ref = stored_ref.clone();
            Box::pin(async move {
                let download = ai_client
                    .get_screenshot_download(
                        &stored_ref.conversation_id,
                        &stored_ref.screenshot_uid,
                    )
                    .await?;
                // Strip the signed URL from transport errors so it cannot leak
                // into logs or Sentry breadcrumbs.
                let response = http_client
                    .get(&download.download_url)
                    .send()
                    .await
                    .map_err(|error| anyhow::Error::new(error.without_url()))?;
                if !response.status().is_success() {
                    anyhow::bail!(
                        "Failed to download screenshot {}: HTTP {}",
                        stored_ref.screenshot_uid,
                        response.status()
                    );
                }
                response
                    .bytes()
                    .await
                    .map_err(|error| anyhow::Error::new(error.without_url()))
            })
        }),
    }
}

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

/// Builds an async asset source that downloads the screenshot bytes on demand. Keyed by the
/// screenshot UID so repeat loads reuse the cached bytes.
pub fn stored_screenshot_asset_source(
    stored_ref: StoredScreenshotRef,
    ai_client: Arc<dyn AIClient>,
) -> AssetSource {
    AssetSource::Async {
        id: AsyncAssetId::new::<StoredScreenshotAsset>(stored_ref.screenshot_uid.clone()),
        fetch: Arc::new(move || {
            let ai_client = ai_client.clone();
            let stored_ref = stored_ref.clone();
            Box::pin(async move {
                ai_client
                    .download_stored_screenshot(
                        &stored_ref.conversation_id,
                        &stored_ref.screenshot_uid,
                    )
                    .await
            })
        }),
    }
}

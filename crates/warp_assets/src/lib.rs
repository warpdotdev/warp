use std::borrow::Cow;

use anyhow::{Result, anyhow};
use rust_embed::RustEmbed;
use warpui_core::AssetProvider;

#[derive(Clone, Copy, RustEmbed)]
#[folder = "../../app/assets"]
#[include = "bundled/**"] // Should be kept in sync with BUNDLED_ASSETS_DIR.
#[include = "async/**"] // Should be kept in sync with ASYNC_ASSETS_DIR.
#[cfg_attr(target_family = "wasm", exclude = "async/**")]
// Excludes take precedence.
// Standalone CLI builds (the `oz` tarball) are headless and never render the
// onboarding/theme imagery in `async/`, so we exclude those bytes from the
// embedded asset set to keep the CLI binary small — mirroring the carve-out
// already applied for the WASM target above.
#[cfg_attr(feature = "standalone", exclude = "async/**")]
pub struct Assets;

impl AssetProvider for Assets {
    fn get(&self, path: &str) -> Result<Cow<'_, [u8]>> {
        <Assets as RustEmbed>::get(path)
            .map(|f| f.data)
            .ok_or_else(|| anyhow!("no asset exists at path {}", path))
    }
}

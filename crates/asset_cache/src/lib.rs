use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use base64::Engine as _;
use base64::prelude::BASE64_STANDARD;
use bytes::Bytes;
use reqwest::Url;
use warpui_core::assets::asset_cache::{
    Asset, AssetCache, AssetSource, AssetState, AsyncAssetId, AsyncAssetType,
};

/// Namespace marker for URL-based async asset sources without persistence.
pub struct UrlAssetWithoutPersistence;
impl AsyncAssetType for UrlAssetWithoutPersistence {}

/// Namespace marker for inline base64 `data:` URI async asset sources.
pub struct DataUriAsset;
impl AsyncAssetType for DataUriAsset {}

pub const MAX_DATA_URI_PAYLOAD_BYTES: usize = 16 * 1024 * 1024;

/// Namespace marker for URL-based async asset sources with persistence.
///
/// This is intentionally separate from `UrlAssetWithoutPersistence` to allow
/// ensure we persist the asset even if we fetched it once already without
/// persistence.
pub struct UrlAssetWithPersistence;
impl AsyncAssetType for UrlAssetWithPersistence {}

/// Creates an [`AssetSource::Async`] that fetches bytes from the given URL
/// without persisting them to the local filesystem.
pub fn url_source(url: impl Into<String>) -> AssetSource {
    let url = url.into();
    let url_for_fetch = url.clone();
    AssetSource::Async {
        id: AsyncAssetId::new::<UrlAssetWithoutPersistence>(url),
        fetch: Arc::new(move || {
            let url = url_for_fetch.clone();
            Box::pin(async move {
                let parsed = Url::parse(&url)?;
                fetch_file_to_memory(parsed).await
            })
        }),
    }
}

/// Returns `true` if `source` is a base64 `data:` URI whose encoded payload
/// exceeds `MAX_DATA_URI_PAYLOAD_BYTES`. Non-`data:` URIs and `data:` URIs
/// without a `;base64` marker return `false`.
pub fn data_uri_exceeds_limit(source: &str) -> bool {
    let Some((header, payload)) = source
        .strip_prefix("data:")
        .and_then(|rest| rest.split_once(','))
    else {
        return false;
    };
    header
        .split(';')
        .any(|segment| segment.eq_ignore_ascii_case("base64"))
        && payload.len() > MAX_DATA_URI_PAYLOAD_BYTES
}

/// Creates an [`AssetSource::Async`] that decodes an inline base64 `data:` URI
/// (e.g. `data:image/png;base64,<payload>`) into its raw bytes.
pub fn data_uri_source(source: &str) -> Option<AssetSource> {
    // data:[<mediatype>][;base64],<payload>
    let (header, payload) = source.strip_prefix("data:")?.split_once(',')?;
    if !header
        .split(';')
        .any(|segment| segment.eq_ignore_ascii_case("base64"))
    {
        return None;
    }

    // `source` is untrusted; reject oversized payloads before cloning/decoding
    if data_uri_exceeds_limit(source) {
        return None;
    }

    // Derive a compact, stable cache key from the full URI so identical payloads
    // dedupe and we don't retain the (potentially large) data URI as the key.
    let mut hasher = DefaultHasher::new();
    source.hash(&mut hasher);
    let id = format!("{:x}", hasher.finish());

    // base64 payloads may contain embedded whitespace/newlines; strip it before
    // decoding.
    let payload: String = payload.chars().filter(|c| !c.is_whitespace()).collect();

    Some(AssetSource::Async {
        id: AsyncAssetId::new::<DataUriAsset>(id),
        fetch: Arc::new(move || {
            let payload = payload.clone();
            Box::pin(async move {
                BASE64_STANDARD
                    .decode(payload.as_bytes())
                    .map(Bytes::from)
                    .map_err(Into::into)
            })
        }),
    })
}

/// Creates an [`AssetSource::Async`] that fetches bytes from the given URL,
/// persisting them to a file under `cache_dir` for future reads.
pub fn url_source_with_persistence(url: impl Into<String>, cache_dir: &Path) -> AssetSource {
    let url = url.into();
    let url_for_fetch = url.clone();
    let cache_dir_owned = cache_dir.to_path_buf();
    AssetSource::Async {
        id: AsyncAssetId::new::<UrlAssetWithPersistence>(url),
        fetch: Arc::new(move || {
            let url = url_for_fetch.clone();
            let cache_dir = cache_dir_owned.clone();
            Box::pin(async move {
                let parsed = Url::parse(&url)?;
                let file = get_file_path_for_asset(&parsed, &cache_dir);
                fetch_asset_from_url(parsed, Some(file)).await
            })
        }),
    }
}

/// Extension trait that adds URL-based asset loading to [`AssetCache`].
pub trait AssetCacheExt {
    /// Loads an asset from a URL, optionally persisting the fetched bytes to
    /// a file under `cache_dir` for future cache hits.
    fn load_asset_from_url<T: Asset>(&self, url: &str, cache_dir: Option<&Path>) -> AssetState<T>;
}

impl AssetCacheExt for AssetCache {
    fn load_asset_from_url<T: Asset>(&self, url: &str, cache_dir: Option<&Path>) -> AssetState<T> {
        let source = match cache_dir {
            Some(dir) => url_source_with_persistence(url, dir),
            None => url_source(url),
        };
        self.load_asset(source)
    }
}

/// Fetches a file from the given `url` to memory.
async fn fetch_file_to_memory(url: Url) -> Result<Bytes, anyhow::Error> {
    cfg_if::cfg_if! {
        if #[cfg(target_family = "wasm")] {
            let response = reqwest::get(url).await?;
        } else {
            // On non-web platforms, reqwest expects that it is operating within
            // a Tokio-compatible runtime, so use async-compat to wrap the call
            // so reqwest's expectations are met.
            let response = async_compat::Compat::new(async move { reqwest::get(url).await }).await?;
        }
    }
    let content = response.error_for_status()?.bytes().await?;
    Ok(content)
}

/// Given a url and a directory where cached artifacts are stored, returns a unique
/// file path for an asset.
fn get_file_path_for_asset(url: &Url, cache_dir: &Path) -> PathBuf {
    // Hash the URL so that we can derive a "safe" file name for it. We need something
    // unique and not too long (most filesystems have a maximum length limit for file
    // names. On MacOS it's 255).
    let mut hasher = DefaultHasher::new();
    url.hash(&mut hasher);
    let digest = hasher.finish();
    // Stringify the bytes in hexadecimal. Be careful not to use base64-digests in file
    // names b/c base64 uses a mix of upper and lowercase chars, which is problematic on
    // case-insensitive filesystems such as MacOS
    let filename = format!("{digest:x}");
    cache_dir.join(filename)
}

#[cfg(not(target_family = "wasm"))]
async fn persist_bytes(bytes: &Bytes, file: &Path) {
    use async_fs::{OpenOptions, create_dir_all};
    use futures::AsyncWriteExt;

    let Some(parent_folder) = file.parent() else {
        log::error!("attempted to write cache file in filesystem root");
        return;
    };

    if let Err(e) = create_dir_all(parent_folder).await {
        log::error!("Error creating directory for cache files: {e:#}");
    }

    let mut file = match OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(file)
        .await
    {
        Ok(file) => file,
        Err(e) => {
            log::error!("Error opening file: {e:#}");
            return;
        }
    };

    if let Err(e) = file.write_all(bytes).await {
        log::error!("Error writing to file: {e:#}");
    }

    if let Err(e) = file.flush().await {
        log::error!("Error flushing file: {e:#}");
    };
}

#[cfg(target_family = "wasm")]
async fn persist_bytes(_bytes: &Bytes, file: &Path) {
    log::debug!("Cannot persist asset to {} on the web", file.display());
}

async fn fetch_file_and_persist_bytes(url: Url, file: Option<PathBuf>) -> Result<Bytes> {
    let result = fetch_file_to_memory(url).await;

    // If the bytes should be written to a file, do so now.
    if let Ok(bytes) = result.as_ref()
        && let Some(filename) = file
    {
        persist_bytes(bytes, &filename).await;
    }

    result
}

async fn fetch_asset_from_url(url: Url, file: Option<PathBuf>) -> Result<Bytes> {
    match file {
        // If a file path is specified and that file path currently exists in the
        // user's filesystem, read the bytes out of the file.
        Some(filename) if filename.exists() => {
            log::debug!("Reading bytes from cached file: {filename:?}");
            let buffer = async_fs::read(filename.clone()).await?;

            // If buffer is empty, try to fetch from url instead
            if buffer.is_empty() {
                return fetch_file_and_persist_bytes(url, Some(filename)).await;
            }

            Ok(buffer.into())
        }
        // Otherwise, fetch the bytes from the url.
        _ => fetch_file_and_persist_bytes(url, file).await,
    }
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;

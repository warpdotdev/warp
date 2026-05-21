use anyhow::anyhow;
use anyhow::{Error, Result};
use async_channel::{self, Receiver, Sender};
use bytes::Bytes;
use derivative::Derivative;
use futures::FutureExt as _;
use futures::{future::BoxFuture, Future};
use std::any::{Any, TypeId};
use std::pin::Pin;
use std::{cell::RefCell, collections::HashMap, hash::Hash, rc::Rc, sync::Arc};

use crate::image_cache::ImageCache;
use crate::{r#async::executor, Entity, ModelContext, SingletonEntity};

use super::AssetProvider;

pub trait FetchAsset: crate::r#async::Spawnable + Future<Output = Result<Bytes>> {}
impl<T: crate::r#async::Spawnable + Future<Output = Result<Bytes>> + ?Sized> FetchAsset for T {}

/// Marker trait for async asset ID namespaces.
///
/// Each distinct kind of async asset source defines its own zero-sized marker
/// type that implements this trait. The marker's [`TypeId`] is stored inside
/// [`AsyncAssetId`] so that IDs from different sources can never collide, even
/// if they happen to share the same key string.
pub trait AsyncAssetType: 'static {}

/// A namespaced identifier for an [`AssetSource::Async`] entry.
///
/// The namespace is stored as a [`TypeId`] derived from a marker type that
/// implements [`AsyncAssetType`]. This guarantees that two different async
/// sources cannot accidentally produce colliding cache keys.
#[derive(Clone, Hash, PartialEq, Eq)]
pub struct AsyncAssetId {
    namespace: TypeId,
    key: String,
}

impl AsyncAssetId {
    /// Creates a new ID in the namespace defined by `N`.
    pub fn new<N: AsyncAssetType>(key: impl Into<String>) -> Self {
        Self {
            namespace: TypeId::of::<N>(),
            key: key.into(),
        }
    }

    /// Returns the key portion of this ID.
    pub fn key(&self) -> &str {
        &self.key
    }
}

impl std::fmt::Debug for AsyncAssetId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // TypeId's Debug output is opaque, so just print the key.
        f.debug_struct("AsyncAssetId")
            .field("key", &self.key)
            .finish()
    }
}

/// A "URI" for some data file. In other words, the location of an asset.
#[derive(Derivative)]
#[derivative(Clone, Hash, PartialEq, Eq, Debug)]
pub enum AssetSource {
    /// Loaded from an arbitrary asynchronous source (e.g. a URL fetch).
    Async {
        /// A namespaced identifier used as the cache key.
        id: AsyncAssetId,
        /// A factory that produces the future to fetch the asset bytes.
        /// Called at most once per unique `id` — only when the asset is
        /// not already loaded or loading.
        #[derivative(Hash = "ignore", PartialEq = "ignore", Debug = "ignore")]
        fetch: Arc<dyn Fn() -> Pin<Box<dyn FetchAsset>> + Send + Sync>,
    },
    /// Included in the app bundle.
    Bundled {
        // Assets that are statically included in the bundle can be statically
        // referenced, hence using a `&'static str` here and not a `String`.
        path: &'static str,
    },
    /// Accessible in the user's local filesystem at the provided path.
    LocalFile { path: String },
    /// Image loaded directly with bytes
    Raw { id: String },
}

/// The public representation of an asset's current state (i.e., in-memory availability).
pub enum AssetState<T> {
    Loading { handle: AssetHandle },
    Loaded { data: Rc<T> },
    Evicted,
    FailedToLoad(Rc<Error>),
}

/// An external type so views can refer to the asset they requested.
/// Transforms into a future that resolves once the asset is finished loading, allowing
/// work to be scheduled at the time of load completion.
#[derive(Clone, Hash, PartialEq, Eq, Debug)]
pub struct AssetHandle {
    source: AssetSource,
    asset_type: TypeId,
}

impl AssetHandle {
    /// Creates a future that resolves whenever the asset is finished loading.
    pub fn when_loaded(&self, asset_cache: &AssetCache) -> Option<BoxFuture<'static, ()>> {
        asset_cache.create_future_for_loading_asset(self)
    }
}

/// An internal representation of an asset's state, as it's tracked and updated by the
/// AssetCache. An implementation.
enum AssetStateInternal {
    Loading {
        channel: (Sender<()>, Receiver<()>),
    },
    Loaded {
        data: Rc<dyn Any>,
        timestamp: u64,
        size_in_bytes: usize,
    },
    Evicted,
    Error(Rc<Error>),
}

impl AssetStateInternal {
    fn loading() -> Self {
        // Whenever we add an asset in a loading state, we create a channel that
        // can be alerted once the asset load completes (i.e., becomes available
        // or encounters an error). The channel must support the ability to clone one
        // side of the channel.
        let channel = async_channel::bounded(1);
        AssetStateInternal::Loading { channel }
    }

    fn to_external_type<T: Asset>(&self, source: AssetSource) -> AssetState<T> {
        match self {
            AssetStateInternal::Loading { .. } => AssetState::Loading {
                handle: AssetHandle {
                    source,
                    asset_type: TypeId::of::<T>(),
                },
            },
            AssetStateInternal::Loaded { data, .. } => AssetState::Loaded {
                data: data
                    .clone()
                    .downcast::<T>()
                    .expect("should not fail to downcast"),
            },
            AssetStateInternal::Evicted => AssetState::Evicted,
            AssetStateInternal::Error(err) => AssetState::FailedToLoad(err.clone()),
        }
    }
}

/// A general-purpose data cache for managing assets. Generalized to any file type.
/// Internally handles networking and persistence caching.
pub struct AssetCache {
    // Note: interior mutability allows us to update the state of an asset
    // without requiring a mutable reference to the AssetCache.
    inner: Rc<RefCell<HashMap<AssetHandle, AssetStateInternal>>>,

    bundled_asset_provider: Box<dyn AssetProvider>,
    foreground_executor: Rc<executor::Foreground>,
    background_executor: Arc<executor::Background>,
}

pub trait Asset: Any {
    fn try_from_bytes(data: &[u8]) -> anyhow::Result<Self>
    where
        Self: Sized;

    fn size_in_bytes(&self) -> usize;
}

impl Asset for String {
    fn try_from_bytes(data: &[u8]) -> anyhow::Result<Self>
    where
        Self: Sized,
    {
        std::str::from_utf8(data)
            .map(|s| s.to_string())
            .map_err(|e| e.into())
    }

    fn size_in_bytes(&self) -> usize {
        self.len()
    }
}

impl AssetCache {
    const MAX_RAW_ASSET_SIZE: usize = 320 * 1000 * 1000; // 320MB

    pub fn new(
        bundled_asset_provider: Box<dyn AssetProvider>,
        foreground_executor: Rc<executor::Foreground>,
        background_executor: Arc<executor::Background>,
    ) -> Self {
        Self {
            inner: Rc::new(RefCell::new(HashMap::new())),
            bundled_asset_provider,
            foreground_executor,
            background_executor,
        }
    }

    /// Tracks the current total size of raw assets in memory.
    pub fn get_total_raw_asset_size(&self) -> usize {
        self.inner
            .borrow()
            .iter()
            .filter_map(|(handle, state)| {
                if let AssetStateInternal::Loaded { size_in_bytes, .. } = state {
                    if matches!(handle.source, AssetSource::Raw { .. }) {
                        return Some(*size_in_bytes);
                    }
                }
                None
            })
            .sum()
    }

    /// Removes the least recently added raw assets until the total size is within the limit.
    fn evict_raw_assets_if_needed(&self, ctx: &ModelContext<Self>) -> Vec<u32> {
        let mut total_size = self.get_total_raw_asset_size();
        let mut assets = self.inner.borrow_mut();

        if total_size <= Self::MAX_RAW_ASSET_SIZE {
            return vec![];
        }

        // Collect all raw assets with their timestamps
        let mut raw_assets: Vec<_> = assets
            .iter()
            .filter_map(|(handle, state)| {
                if matches!(handle.source, AssetSource::Raw { .. }) {
                    if let AssetStateInternal::Loaded {
                        timestamp,
                        size_in_bytes,
                        ..
                    } = state
                    {
                        return Some((handle.clone(), *timestamp, *size_in_bytes));
                    }
                }
                None
            })
            .collect();

        // Sort by timestamp (oldest first)
        raw_assets.sort_by_key(|&(_, timestamp, _)| timestamp);

        let mut evicted_image_ids = vec![];

        // Evict until within the limit
        for (handle, _, size_in_bytes) in raw_assets {
            if total_size <= Self::MAX_RAW_ASSET_SIZE {
                break;
            }
            if let AssetSource::Raw { id } = &handle.source {
                if assets.remove(&handle).is_some() {
                    assets.insert(handle.clone(), AssetStateInternal::Evicted);
                    ImageCache::as_ref(ctx).evict_image(&handle.source);
                    total_size -= size_in_bytes;

                    if let Ok(id) = id.parse::<u32>() {
                        evicted_image_ids.push(id);
                    }
                }
            }
        }

        evicted_image_ids
    }

    /// The main API of the asset cache. Given the location of an asset, returns an indicator of the
    /// in-memory availability of the asset. If the asset is not already loaded or loading, a background
    /// task is spawned to perform the retrieval.
    ///
    /// Note: this is an idempotent operation. It can be called as many times as needed on a given
    /// asset and won't duplicate work.
    pub fn load_asset<T: Asset>(&self, source: AssetSource) -> AssetState<T> {
        let mut assets = self.inner.borrow_mut();

        // If we've already seen this asset source, we can simply return the current state of it. Otherwise,
        // begin the load.
        let key = AssetHandle {
            source: source.clone(),
            asset_type: TypeId::of::<T>(),
        };
        if !assets.contains_key(&key) {
            match source.clone() {
                AssetSource::Async { fetch, .. } => {
                    assets.insert(key.clone(), AssetStateInternal::loading());
                    let future = (fetch)();
                    self.load_asynchronously::<T>(source.clone(), future);
                }
                AssetSource::Bundled { path } => {
                    let asset_state = match self
                        .bundled_asset_provider
                        .get(path)
                        .and_then(|bytes| T::try_from_bytes(&bytes))
                    {
                        Ok(asset) => {
                            let timestamp = instant::now() as u64;
                            let size_in_bytes = asset.size_in_bytes();

                            AssetStateInternal::Loaded {
                                data: Rc::new(asset) as Rc<dyn Any>,
                                timestamp,
                                size_in_bytes,
                            }
                        }
                        Err(err) => AssetStateInternal::Error(Rc::new(err)),
                    };
                    assets.insert(key.clone(), asset_state);
                }
                AssetSource::LocalFile { path } => {
                    assets.insert(key.clone(), AssetStateInternal::loading());
                    self.load_asynchronously::<T>(
                        source.clone(),
                        Box::pin(load_local_file_bounded(path)),
                    );
                }
                AssetSource::Raw { id } => {
                    assets.insert(
                        key.clone(),
                        AssetStateInternal::Error(Rc::new(anyhow!(
                            "Raw image with ID {:?} did not exist",
                            id
                        ))),
                    );
                }
            };
        }

        assets[&key].to_external_type(source)
    }

    pub fn insert_raw_asset_bytes<T: Asset>(
        &self,
        id: String,
        bytes: &[u8],
        ctx: &mut ModelContext<Self>,
    ) {
        let mut assets = self.inner.borrow_mut();
        let source = AssetSource::Raw { id: id.clone() };
        let key = AssetHandle {
            source: source.clone(),
            asset_type: TypeId::of::<T>(),
        };
        match T::try_from_bytes(bytes) {
            Ok(asset) => {
                let timestamp = instant::now() as u64;
                let size_in_bytes = asset.size_in_bytes();

                assets.insert(
                    key.clone(),
                    AssetStateInternal::Loaded {
                        data: Rc::new(asset) as Rc<dyn Any>,
                        timestamp,
                        size_in_bytes,
                    },
                );
            }
            Err(err) => {
                log::warn!("Raw asset conversion failed (ID: {id}): {err:#}");
                assets.insert(key.clone(), AssetStateInternal::Error(Rc::new(err)));
            }
        };

        ImageCache::as_ref(ctx).evict_image(&source);

        drop(assets);
        let image_ids = self.evict_raw_assets_if_needed(ctx);

        if !image_ids.is_empty() {
            ctx.emit(AssetCacheEvent::ImagesEvicted { image_ids });
        }
    }

    // Creates a future that resolves when an asset is loaded into moemory.
    fn create_future_for_loading_asset(
        &self,
        asset_handle: &AssetHandle,
    ) -> Option<BoxFuture<'static, ()>> {
        let assets = self.inner.borrow_mut();

        assets.get(asset_handle).map(|asset_state| {
            match asset_state {
                AssetStateInternal::Loading { channel } => {
                    // Internally, the future works by cloning a new receiver on the channel that's assigned
                    // to this asset. Inside the future, we simply wait on the receiving end of the channel.
                    // Note that the channel is held by the AssetStateInternal::Loading variant, so when the asset
                    // is promoted to the Loaded or FailedToLoad variants, the channel is dropped. This returns a
                    // RecvError to any receivers, serving as our notification that the asset is no longer loading.
                    let rx = channel.1.clone();
                    async move {
                        let _ = rx.recv().await;
                    }
                    .boxed()
                }
                // If the asset isn't currently loading, it is either already loaded or it's in an error state. Either
                // way, we should return a future that resolves immediately since there's no more pending updates
                // for this asset.
                _ => futures::future::ready(()).boxed(),
            }
        })
    }

    // Helper method to spawn the futures that perform an asset load and place the results into the asset cache.
    fn load_asynchronously<T: Asset>(
        &self,
        asset_source: AssetSource,
        future: Pin<Box<dyn FetchAsset>>,
    ) {
        let (tx, rx) = futures::channel::oneshot::channel();

        // Spawn the work on the background executor.
        self.background_executor
            .spawn(async move {
                let result = future.await;
                // When the fetch finished, send the results to the future running on the foreground executor.
                if tx.send(result).is_err() {
                    log::error!("Error sending background task result to main thread");
                }
            })
            .detach();

        // Spawn a receiver on the foreground executor.
        let assets = Rc::downgrade(&self.inner);
        self.foreground_executor
            .spawn_boxed(Box::pin(async move {
                let result = match rx.await {
                    Ok(result) => result,
                    Err(_) => {
                        let msg = "sender unexpectedly dropped before receiver";
                        log::error!("{msg}");
                        Err(anyhow!(msg))
                    }
                };

                let Some(assets) = assets.upgrade() else {
                    return;
                };

                let mut assets = assets.borrow_mut();

                // Populate the asset cache with the result.
                let handle = AssetHandle {
                    source: asset_source.clone(),
                    asset_type: TypeId::of::<T>(),
                };
                match result {
                    Ok(bytes) => match T::try_from_bytes(&bytes) {
                        Ok(asset) => {
                            log::debug!("Asset fetch succeeded: {asset_source:?}");

                            let timestamp = instant::now() as u64;
                            let size_in_bytes = asset.size_in_bytes();

                            assets.insert(
                                handle,
                                AssetStateInternal::Loaded {
                                    data: Rc::new(asset) as Rc<dyn Any>,
                                    timestamp,
                                    size_in_bytes,
                                },
                            );
                        }
                        Err(err) => {
                            log::warn!("Asset conversion failed ({asset_source:?}): {err:#}");
                            assets.insert(handle, AssetStateInternal::Error(Rc::new(err)));
                        }
                    },
                    Err(err) => {
                        log::warn!("Asset fetch failed ({asset_source:?}): {err:#}");
                        assets.insert(handle, AssetStateInternal::Error(Rc::new(err)));
                    }
                }
            }))
            .detach();
    }
}

#[derive(Debug, Clone)]
pub enum AssetCacheEvent {
    ImagesEvicted { image_ids: Vec<u32> },
}

impl Entity for AssetCache {
    type Event = AssetCacheEvent;
}

impl SingletonEntity for AssetCache {}

/// Maximum bytes accepted from a `LocalFile` read for raster (non-SVG) inputs.
/// Matches `MAX_PREVIEW_FILE_BYTES` in the workspace image-preview arm; the
/// pre-read metadata stat there should make this cap unreachable in the
/// happy path, but the asset-cache layer keeps the bound as a defense
/// against TOCTOU growth and against any future `LocalFile` consumer that
/// does not pre-stat. See specs/GH9729/tech.md §400.
const MAX_ASSET_LOCAL_FILE_BYTES: u64 = 64 * 1024 * 1024;

/// Tighter byte cap applied to inputs that look like XML/SVG on a 1 KB
/// content peek. Keyed on content (not extension) so XML hidden under a
/// non-`.svg` extension is also tightened. See specs/GH9729/tech.md §400.
const MAX_SVG_BYTES: u64 = 4 * 1024 * 1024;

/// Inner bounded-read entry point that takes the raster and SVG caps as
/// parameters. Lets tests exercise the cap-selection and over-cap rejection
/// paths against modest fixtures with small caps. Production callers go
/// through `load_local_file_bounded` which threads the GH9729 production
/// constants.
async fn load_local_file_bounded_inner(
    path: String,
    raster_cap: u64,
    svg_cap: u64,
) -> Result<Bytes> {
    use futures_lite::AsyncReadExt as _;

    #[cfg(unix)]
    use async_fs::unix::OpenOptionsExt as _;

    let mut file = {
        let mut opts = async_fs::OpenOptions::new();
        opts.read(true);
        #[cfg(unix)]
        opts.custom_flags(libc::O_NONBLOCK);
        opts.open(&path).await?
    };

    let meta = file.metadata().await?;
    if !meta.file_type().is_file() {
        anyhow::bail!("local asset is not a regular file");
    }

    let mut peek = [0u8; 1024];
    let n = file.read(&mut peek).await?;
    let cap = if crate::image_cache::looks_like_svg_xml(&peek[..n]) {
        svg_cap
    } else {
        raster_cap
    };

    let mut buf: Vec<u8> = Vec::with_capacity(n);
    buf.extend_from_slice(&peek[..n]);
    let remaining = (cap + 1).saturating_sub(buf.len() as u64);
    let mut taken = file.take(remaining);
    taken.read_to_end(&mut buf).await?;
    if buf.len() as u64 > cap {
        anyhow::bail!("local asset exceeds size cap");
    }
    Ok(Bytes::from(buf))
}

/// Bounded read of a local file under the GH9729 asset-cache cap.
///
/// Closes four surfaces enumerated in tech.md §400:
///   1. `O_NONBLOCK` on Unix so `open()` of a FIFO returns immediately
///      rather than blocking indefinitely waiting for a writer. (Read on a
///      regular file is unaffected by `O_NONBLOCK` on POSIX.)
///   2. Post-open `is_file()` against the OPENED descriptor (`fstat`, not
///      path-based stat). Closes the TOCTOU window where the path was
///      swapped to a FIFO / character device / directory between the
///      workspace pre-read stat and this open syscall.
///   3. Content-keyed cap selection: a 1 KB peek runs the same
///      `looks_like_svg_xml` predicate that gates `usvg::Tree::from_data`,
///      so an XML payload hidden under any extension is capped at 4 MB.
///   4. `MAX + 1` bounded read: deterministically rejects a file that
///      grew past the cap between the workspace stat and this read.
async fn load_local_file_bounded(path: String) -> Result<Bytes> {
    load_local_file_bounded_inner(path, MAX_ASSET_LOCAL_FILE_BYTES, MAX_SVG_BYTES).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_lite::future::block_on;

    /// SVG XML body sized to `target_bytes` total. Begins with the standard
    /// `<svg>` opening tag (so `looks_like_svg_xml` matches on the peek)
    /// and pads to size with `<g/>` elements.
    fn svg_payload(target_bytes: usize) -> Vec<u8> {
        let prelude = b"<svg xmlns=\"http://www.w3.org/2000/svg\">";
        let suffix = b"</svg>";
        let pad = b"<g/>";
        let mut out = Vec::with_capacity(target_bytes);
        out.extend_from_slice(prelude);
        while out.len() + suffix.len() + pad.len() <= target_bytes {
            out.extend_from_slice(pad);
        }
        out.extend_from_slice(suffix);
        // Top up with single bytes if we're still under target.
        while out.len() < target_bytes {
            out.push(b' ');
        }
        out
    }

    /// Encode a small valid PNG (200x100 blank RGBA) and return its bytes.
    fn small_png_bytes() -> Vec<u8> {
        let img = image::RgbaImage::new(200, 100);
        let mut out: Vec<u8> = Vec::new();
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
            .expect("PNG encode for test");
        out
    }

    #[test]
    fn local_file_read_passes_under_cap() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("ok.bin");
        std::fs::write(&path, b"hello").unwrap();
        let bytes = block_on(load_local_file_bounded_inner(
            path.to_string_lossy().into_owned(),
            /* raster_cap */ 1024,
            /* svg_cap */ 1024,
        ))
        .expect("under-cap read should succeed");
        assert_eq!(bytes.as_ref(), b"hello");
    }

    #[test]
    fn local_file_read_caps_at_max_bytes() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("over.bin");
        // 200 bytes against a 100-byte raster cap.
        std::fs::write(&path, vec![0u8; 200]).unwrap();
        let result = block_on(load_local_file_bounded_inner(
            path.to_string_lossy().into_owned(),
            /* raster_cap */ 100,
            /* svg_cap */ 100,
        ));
        assert!(result.is_err(), "over-cap raster read must fail");
    }

    #[test]
    fn local_file_read_rejects_post_open_non_regular_file() {
        // Pass a directory path directly: `open()` succeeds on some platforms
        // (and `async_fs::open` on a directory reports an error elsewhere).
        // Either way the load future must NOT yield bytes. The post-open
        // is_file() rejection is the test target; on platforms where open()
        // itself fails for directories, the broader contract still holds
        // ("this never returns Ok").
        let dir = tempfile::TempDir::new().unwrap();
        let result = block_on(load_local_file_bounded_inner(
            dir.path().to_string_lossy().into_owned(),
            1024,
            1024,
        ));
        assert!(result.is_err(), "directory path must not yield Ok bytes");
    }

    #[cfg(unix)]
    #[test]
    fn local_file_read_does_not_block_on_fifo() {
        // Create a FIFO with no writer attached. Without O_NONBLOCK the
        // open() syscall would block this test indefinitely; with
        // O_NONBLOCK, open() returns immediately and the post-open
        // is_file() check rejects it.
        let dir = tempfile::TempDir::new().unwrap();
        let fifo = dir.path().join("test.fifo");
        let fifo_c = std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
        let mode: libc::mode_t = 0o644;
        let rc = unsafe { libc::mkfifo(fifo_c.as_ptr(), mode) };
        assert_eq!(rc, 0, "mkfifo failed");

        let result = block_on(load_local_file_bounded_inner(
            fifo.to_string_lossy().into_owned(),
            1024,
            1024,
        ));
        assert!(result.is_err(), "FIFO must not yield Ok bytes");
    }

    #[test]
    fn local_file_read_caps_svg_at_smaller_limit() {
        // SVG XML payload over the SVG cap, under the raster cap. The peek
        // matches `looks_like_svg_xml` so the SVG cap (small) applies and
        // the read must fail.
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("huge.svg");
        // 200 bytes of SVG XML against a 50-byte SVG cap.
        std::fs::write(&path, svg_payload(200)).unwrap();
        let result = block_on(load_local_file_bounded_inner(
            path.to_string_lossy().into_owned(),
            /* raster_cap */ 1024,
            /* svg_cap */ 50,
        ));
        assert!(result.is_err(), "over-SVG-cap read must fail");

        // Same payload renamed `huge.bin`: content-keying still selects
        // the SVG cap, so this also fails.
        let path2 = dir.path().join("huge.bin");
        std::fs::write(&path2, svg_payload(200)).unwrap();
        let result2 = block_on(load_local_file_bounded_inner(
            path2.to_string_lossy().into_owned(),
            /* raster_cap */ 1024,
            /* svg_cap */ 50,
        ));
        assert!(
            result2.is_err(),
            "content-keying must apply SVG cap regardless of extension",
        );
    }

    #[test]
    fn local_file_read_caps_svg_content_under_png_extension() {
        // SVG XML hidden under a `.png` extension. Without content-keying
        // the raster cap (large) would apply and the bytes would flow to
        // the parser. With content-keying the SVG cap (small) applies.
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("evil.png");
        std::fs::write(&path, svg_payload(200)).unwrap();
        let result = block_on(load_local_file_bounded_inner(
            path.to_string_lossy().into_owned(),
            /* raster_cap */ 1024,
            /* svg_cap */ 50,
        ));
        assert!(
            result.is_err(),
            "content-keyed SVG cap must fire despite .png extension",
        );
    }

    #[test]
    fn local_file_read_uses_raster_cap_for_non_svg_content() {
        // Real PNG bytes hidden under a `.svg` extension. The peek does
        // NOT match `looks_like_svg_xml` (PNG starts with 0x89 P N G),
        // so the raster cap applies and the file passes despite the
        // misleading extension. This is the symmetric assertion that
        // content-keying does not over-tighten.
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("large.svg");
        let png = small_png_bytes();
        let png_len = png.len();
        std::fs::write(&path, &png).unwrap();
        let bytes = block_on(load_local_file_bounded_inner(
            path.to_string_lossy().into_owned(),
            /* raster_cap */ (png_len as u64) + 1024,
            /* svg_cap */ 50,
        ))
        .expect("PNG bytes must pass under raster cap regardless of extension");
        assert_eq!(bytes.len(), png_len);
    }
}

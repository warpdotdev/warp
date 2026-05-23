use std::any::{Any, TypeId};
use std::cell::RefCell;
use std::collections::HashMap;
use std::hash::Hash;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::Arc;

use anyhow::{anyhow, Error, Result};
use async_channel::{self, Receiver, Sender};
use bytes::Bytes;
use derivative::Derivative;
use futures::future::BoxFuture;
use futures::{Future, FutureExt as _};

use super::AssetProvider;
use crate::image_cache::ImageCache;
use crate::r#async::executor;
use crate::{Entity, ModelContext, SingletonEntity};

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
                        Box::pin(async move {
                            let buffer = async_fs::read(path).await?;
                            Ok(buffer.into())
                        }),
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

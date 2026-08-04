use std::any::{Any, TypeId};
use std::borrow::Cow;

use anyhow::{Result, anyhow};
pub mod asset_cache;

impl AssetProvider for () {
    fn get(&self, path: &str) -> Result<Cow<'_, [u8]>> {
        Err(anyhow!(
            "get called on empty asset provider with \"{}\"",
            path
        ))
    }
}

/// Identifies the asset set a provider serves, for use as a key in caches that
/// memoize derived data across providers (e.g. the bootstrap script cache).
///
/// Two providers that compare equal here must serve identical bytes for every
/// path, since a cache may return one provider's derived data for the other.
#[derive(Clone, Copy, Debug, Hash, Eq, PartialEq)]
pub struct AssetCacheKey {
    provider_type: TypeId,
    /// Distinguishes instances of a provider type whose contents vary per
    /// instance. Stateless providers leave this at `0`.
    instance: u64,
}

impl AssetCacheKey {
    /// Key for a provider whose contents are fixed by its type, so all
    /// instances serve identical bytes.
    pub fn for_type<T: 'static + ?Sized>() -> Self {
        Self {
            provider_type: TypeId::of::<T>(),
            instance: 0,
        }
    }

    /// Key for a provider whose contents vary per instance. `instance` must
    /// differ whenever the served bytes differ.
    pub fn for_instance<T: 'static + ?Sized>(instance: u64) -> Self {
        Self {
            provider_type: TypeId::of::<T>(),
            instance,
        }
    }
}

/// `Any` is a supertrait so [`AssetProvider::cache_key`]'s default
/// implementation can read the concrete type's [`TypeId`] through a trait
/// object. It's automatically satisfied by any `'static` type, so it adds no
/// burden on implementors.
pub trait AssetProvider: Any + 'static {
    fn get(&self, path: &str) -> Result<Cow<'_, [u8]>>;

    /// Identifies this provider's asset set for cross-provider caches.
    ///
    /// The default is correct for providers whose contents are fixed by their
    /// type (unit structs, `RustEmbed` types). A provider whose contents vary
    /// per instance **must** override this with
    /// [`AssetCacheKey::for_instance`], or callers may serve it another
    /// instance's cached data.
    fn cache_key(&self) -> AssetCacheKey {
        AssetCacheKey {
            provider_type: self.type_id(),
            instance: 0,
        }
    }
}

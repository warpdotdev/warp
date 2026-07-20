use std::any::Any;
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

/// `Any` is a supertrait so callers can key caches on the concrete provider's
/// [`std::any::TypeId`] (e.g. to distinguish the real bundled-asset provider
/// from a test fixture). It's automatically satisfied by any `'static` type,
/// so it adds no burden on implementors.
pub trait AssetProvider: Any + 'static {
    fn get(&self, path: &str) -> Result<Cow<'_, [u8]>>;
}

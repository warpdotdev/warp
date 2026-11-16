use gloo_storage::errors::StorageError;
use gloo_storage::{LocalStorage, Storage};

use super::UserPreferences;

/// An implementation of the [`UserPreferences`] trait using the local storage
/// property of the Web Storage API for persistence.
///
/// See: https://developer.mozilla.org/en-US/docs/Web/API/Storage
#[derive(Default)]
pub struct LocalStoragePreferences;

impl UserPreferences for LocalStoragePreferences {
    fn write_value(&self, key: &str, value: String) -> Result<(), super::Error> {
        LocalStorage::set(key, value)
            .map_err(anyhow::Error::from)
            .map_err(super::Error::from)
    }

    fn read_value(&self, key: &str) -> Result<Option<String>, super::Error> {
        match LocalStorage::get(key) {
            Ok(val) => Ok(Some(val)),
            Err(StorageError::KeyNotFound(_)) => Ok(None),
            Err(e) => Err(super::Error::from(anyhow::Error::from(e))),
        }
    }

    fn remove_value(&self, key: &str) -> Result<(), super::Error> {
        LocalStorage::delete(key);
        Ok(())
    }
}
